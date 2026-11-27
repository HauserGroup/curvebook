//! Per-row replicate summary statistics (`AnalysisKind::RowStatistics`, `R23.3`).
//!
//! Computes row-wise totals, arithmetic means (with SD, SEM, `%CV`, Student-*t* CI, or Min/Max),
//! medians (with Type-6 quartiles/IQR, Min/Max, or Type-7 custom percentiles), and geometric means
//! (with geometric SD factor or back-transformed *t*-CI over strictly positive replicates) across
//! `Xy`, `Grouped`, `Column`, and `MultipleVariables` tables, emitting both structured result tabs
//! and a validated downstream-analyzable [`DataSheet`].
//!
//! References:
//! - Hyndman, R. J., & Fan, Y. (1996). Sample quantiles in statistical packages.
//!   *The American Statistician*, 50(4), 361–365. `doi:10.1080/00031305.1996.10473566`.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Prentice Hall.
//! - Motulsky, H. (2018). *Intuitive Biostatistics: A Nonmathematical Guide to Statistical Thinking*
//!   (4th ed.). Oxford University Press.

use curvebook_doc::error::MAX_SHEET_ID_BYTES;
use curvebook_doc::{
    AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSetColumn, DataSheet, GroupedTable, MAX_TITLE_BYTES, MultipleVariablesTable,
    ReplicateStacking, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, RowStatisticsSpec, RowSummaryKind,
    SubcolumnMode, SummarizedPoint, TableArchetype, VariableColumn, VariableRole, XyTable,
};
use curvebook_math::{dist, quantile_type7_sorted, sum};

use crate::error::TransformError;
use crate::ops::{TransformContext, bounded_suffixed_str};

/// Default upper percentile (`90%`, paired with symmetric `10%`) for [`RowSummaryKind::MedianPercentiles`].
const DEFAULT_UPPER_PERCENTILE: f64 = 90.0;

/// Compute a Hyndman & Fan (1996) Type-6 sample quantile (`h = p * (n + 1)` clamped to `[1, n]`)
/// from a non-empty ascending slice of finite `f64` values.
#[must_use]
pub fn quantile_type6_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    let n = sorted.len();
    if n == 0 || !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return None;
    }
    if n == 1 {
        return sorted[0].is_finite().then_some(sorted[0]);
    }
    #[allow(clippy::cast_precision_loss)]
    let n_f = n as f64;
    #[allow(clippy::cast_precision_loss)]
    let h = (p * ((n + 1) as f64)).clamp(1.0, n_f);
    let h_floor = h.floor();
    let h_ceil = h.ceil();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = (h_floor as usize).saturating_sub(1).min(n - 1);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let hi = (h_ceil as usize).saturating_sub(1).min(n - 1);
    let v_lo = sorted[lo];
    let v_hi = sorted[hi];
    if lo == hi {
        return v_lo.is_finite().then_some(v_lo);
    }
    let frac = h - h_floor;
    let diff = v_hi - v_lo;
    let out = if diff.is_finite() { v_lo + frac * diff } else { (1.0 - frac) * v_lo + frac * v_hi };
    out.is_finite().then_some(out)
}

#[derive(Debug, Clone, Copy, Default)]
struct RowMetrics {
    n: usize,
    total: Option<f64>,
    mean: Option<f64>,
    sd: Option<f64>,
    sem: Option<f64>,
    cv_percent: Option<f64>,
    mean_ci: Option<(f64, f64)>,
    min: Option<f64>,
    max: Option<f64>,
    range: Option<f64>,
    q1: Option<f64>,
    median: Option<f64>,
    q3: Option<f64>,
    iqr: Option<f64>,
    p_low: Option<f64>,
    p_high: Option<f64>,
    geo_mean: Option<f64>,
    geo_sd_factor: Option<f64>,
    geo_ci: Option<(f64, f64)>,
    has_non_positive: bool,
}

impl RowMetrics {
    fn from_raw_replicates(values: &[f64], alpha: f64, upper_p: f64) -> Self {
        let n = values.len();
        if n == 0 {
            return Self::default();
        }
        let total = Some(sum::sum(values)).filter(|v| v.is_finite());
        let moments = sum::SummaryMoments::from_slice(values).ok();
        let mean = moments.as_ref().map(|m| m.mean).filter(|v| v.is_finite());
        let sd = moments.as_ref().and_then(|m| m.sample_sd().ok()).filter(|v| v.is_finite());
        let sem = moments.as_ref().and_then(|m| m.sem().ok()).filter(|v| v.is_finite());
        let cv_percent = match (mean, sd) {
            (Some(m), Some(s)) if m != 0.0 => {
                let cv = (100.0 * s) / m.abs();
                cv.is_finite().then_some(cv)
            }
            _ => None,
        };
        #[allow(clippy::cast_precision_loss)]
        let t_crit = if n >= 2 { dist::student_t_two_sided_isf(alpha, (n - 1) as f64).ok().filter(|v| v.is_finite()) } else { None };
        let mean_ci = match (mean, sem, t_crit) {
            (Some(m), Some(se), Some(t)) => {
                let lo = m - t * se;
                let hi = m + t * se;
                (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
            }
            _ => None,
        };

        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let min = sorted.first().copied().filter(|v| v.is_finite());
        let max = sorted.last().copied().filter(|v| v.is_finite());
        let range = match (min, max) {
            (Some(lo), Some(hi)) => {
                let r = hi - lo;
                r.is_finite().then_some(r)
            }
            _ => None,
        };
        let q1 = quantile_type6_sorted(&sorted, 0.25);
        let median = quantile_type6_sorted(&sorted, 0.50);
        let q3 = quantile_type6_sorted(&sorted, 0.75);
        let iqr = match (q1, q3) {
            (Some(a), Some(b)) => {
                let d = b - a;
                d.is_finite().then_some(d)
            }
            _ => None,
        };
        let lower_p = 100.0 - upper_p;
        let p_low = quantile_type7_sorted(&sorted, lower_p / 100.0).filter(|v| v.is_finite());
        let p_high = quantile_type7_sorted(&sorted, upper_p / 100.0).filter(|v| v.is_finite());

        let has_non_positive = values.iter().any(|&v| v <= 0.0);
        let (geo_mean, geo_sd_factor, geo_ci) = if has_non_positive {
            (None, None, None)
        } else {
            let logs: Vec<f64> = values.iter().map(|v| v.ln()).collect();
            let log_moments = sum::SummaryMoments::from_slice(&logs).ok();
            let ml = log_moments.as_ref().map(|m| m.mean).filter(|v| v.is_finite());
            let sl = log_moments.as_ref().and_then(|m| m.sample_sd().ok()).filter(|v| v.is_finite());
            let seml = log_moments.as_ref().and_then(|m| m.sem().ok()).filter(|v| v.is_finite());
            let gm = ml.map(f64::exp).filter(|v| v.is_finite());
            let gsd = sl.map(f64::exp).filter(|v| v.is_finite());
            let gci = match (ml, seml, t_crit) {
                (Some(m), Some(se), Some(t)) => {
                    let lo = (m - t * se).exp();
                    let hi = (m + t * se).exp();
                    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
                }
                _ => None,
            };
            (gm, gsd, gci)
        };

        Self {
            n,
            total,
            mean,
            sd,
            sem,
            cv_percent,
            mean_ci,
            min,
            max,
            range,
            q1,
            median,
            q3,
            iqr,
            p_low,
            p_high,
            geo_mean,
            geo_sd_factor,
            geo_ci,
            has_non_positive,
        }
    }

    fn from_summarized_point(pt: SummarizedPoint, alpha: f64) -> Self {
        if !pt.mean.is_finite() || !pt.n.is_finite() || pt.n <= 0.0 {
            return Self::default();
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n_usize = pt.n.round().max(1.0) as usize;
        let mean = Some(pt.mean);
        let total = (pt.mean * pt.n).is_finite().then_some(pt.mean * pt.n);
        let sd = (pt.n >= 2.0 && pt.sd.is_finite()).then_some(pt.sd);
        let sem = sd.map(|s| s / pt.n.sqrt()).filter(|v| v.is_finite());
        let cv_percent = match (mean, sd) {
            (Some(m), Some(s)) if m != 0.0 => {
                let cv = (100.0 * s) / m.abs();
                cv.is_finite().then_some(cv)
            }
            _ => None,
        };
        let mean_ci = match (mean, sem) {
            (Some(m), Some(se)) if pt.n >= 2.0 => dist::student_t_two_sided_isf(alpha, pt.n - 1.0).ok().and_then(|t| {
                let lo = m - t * se;
                let hi = m + t * se;
                (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
            }),
            _ => None,
        };
        Self { n: n_usize, total, mean, sd, sem, cv_percent, mean_ci, ..Self::default() }
    }
}

struct SeriesRowStats {
    title: String,
    rows: Vec<RowMetrics>,
}

/// Execute [`AnalysisKind::RowStatistics`](curvebook_doc::AnalysisKind::RowStatistics) against a
/// compatible [`DataSheet`] (`Column`, `Xy`, `Grouped`, or `MultipleVariables`) and return a
/// populated [`ResultSheet`] with structured summary tables and `analyzable_data`.
pub fn execute_row_statistics(
    source: &DataSheet,
    analysis: &AnalysisSpec,
    spec: &RowStatisticsSpec,
    ctx: &TransformContext<'_>,
) -> Result<ResultSheet, TransformError> {
    source.archetype.validate()?;
    let upper_p = validate_upper_percentile(spec.upper_percentile)?;
    let conf = analysis.confidence_level.percent() / 100.0;
    let alpha = analysis.confidence_level.alpha();
    if !alpha.is_finite() || !(0.0 < alpha && alpha < 1.0) {
        return Err(TransformError::InvalidSpec { message: format!("confidence alpha must be in (0, 1), got {alpha}") });
    }

    let col_count = source.archetype.column_count();
    let selected = resolve_selected_columns(col_count, &ctx.selected_data_sets, &analysis.selected_data_sets)?;
    if selected.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("table has no data columns for Row Statistics") });
    }

    let (series_list, row_labels, non_pos_rows, pre_summarized_note) = match &source.archetype {
        TableArchetype::Xy(xy) => extract_xy_row_stats(xy, &selected, alpha, upper_p)?,
        TableArchetype::Grouped(g) => extract_grouped_row_stats(g, &selected, alpha, upper_p)?,
        TableArchetype::Column(c) => extract_column_row_stats(c, &selected, alpha, upper_p)?,
        TableArchetype::MultipleVariables(mv) => extract_mv_row_stats(mv, &selected, alpha, upper_p),
        _ => {
            return Err(TransformError::IncompatibleTable { operation: "Row Statistics", kind: source.kind() });
        }
    };

    let tab = build_row_stats_tab(&series_list, &row_labels, spec.summary, conf, upper_p, non_pos_rows, pre_summarized_note);
    let analyzable = build_analyzable_sheet(source, analysis, spec.summary, &selected, &series_list, &row_labels, upper_p, ctx)?;

    let mut sheet = ResultSheet::new(analysis.clone())?;
    sheet.status = ResultStatus::UpToDate;
    sheet.tabs.push(tab);
    #[allow(clippy::cast_precision_loss)]
    sheet.scalars.push(ResultScalarEntry { key: String::from("RowCount"), data_set_index: None, value: row_labels.len() as f64, units: None });
    #[allow(clippy::cast_precision_loss)]
    sheet.scalars.push(ResultScalarEntry { key: String::from("SeriesCount"), data_set_index: None, value: series_list.len() as f64, units: None });
    sheet.analyzable_data = Some(analyzable);
    Ok(sheet)
}

fn validate_upper_percentile(upper: Option<f64>) -> Result<f64, TransformError> {
    let p = upper.unwrap_or(DEFAULT_UPPER_PERCENTILE);
    if !p.is_finite() || !(p > 50.0 && p < 100.0) {
        return Err(TransformError::InvalidSpec { message: format!("RowStatistics upper_percentile must be finite and in (50, 100), got {p}") });
    }
    Ok(p)
}

fn resolve_selected_columns(total_cols: usize, ctx_selected: &[usize], analysis_selected: &[usize]) -> Result<Vec<usize>, TransformError> {
    let chosen = if !ctx_selected.is_empty() { ctx_selected } else { analysis_selected };
    if chosen.is_empty() {
        return Ok((0..total_cols).collect());
    }
    let mut out = Vec::with_capacity(chosen.len().min(total_cols));
    for &idx in chosen {
        if idx >= total_cols {
            return Err(TransformError::InvalidSpec { message: format!("selected dataset index {idx} exceeds column count {total_cols}") });
        }
        if !out.contains(&idx) {
            out.push(idx);
        }
    }
    Ok(out)
}

fn extract_dataset_series(
    data_sets: &[DataSetColumn],
    mode: SubcolumnMode,
    row_count: usize,
    selected: &[usize],
    alpha: f64,
    upper_p: f64,
) -> Result<(Vec<SeriesRowStats>, usize, bool), TransformError> {
    let mut series = Vec::with_capacity(selected.len());
    let mut non_pos_rows = 0_usize;
    let is_pre_sum = mode.is_pre_summarized();

    for &col in selected {
        let ds = &data_sets[col];
        let mut rows = Vec::with_capacity(row_count);
        for r in 0..row_count {
            let subcells = ds.cells.get(r).map(Vec::as_slice).unwrap_or(&[]);
            let metrics = if mode.supports_parametric_moments() {
                match SummarizedPoint::reconstruct(mode, subcells)? {
                    Some(pt) => RowMetrics::from_summarized_point(pt, alpha),
                    None => RowMetrics::default(),
                }
            } else if is_pre_sum {
                let active: Vec<f64> = subcells.first().and_then(Cell::active_number).into_iter().collect();
                RowMetrics::from_raw_replicates(&active, alpha, upper_p)
            } else {
                let active: Vec<f64> = subcells.iter().filter_map(Cell::active_number).collect();
                RowMetrics::from_raw_replicates(&active, alpha, upper_p)
            };
            if metrics.has_non_positive {
                non_pos_rows += 1;
            }
            rows.push(metrics);
        }
        series.push(SeriesRowStats { title: ds.title.clone(), rows });
    }
    Ok((series, non_pos_rows, is_pre_sum))
}

fn extract_xy_row_stats(
    xy: &XyTable,
    selected: &[usize],
    alpha: f64,
    upper_p: f64,
) -> Result<(Vec<SeriesRowStats>, Vec<String>, usize, bool), TransformError> {
    let row_count = xy.row_count();
    let (series, non_pos, pre_sum) = extract_dataset_series(&xy.data_sets, xy.y_subcolumn_mode, row_count, selected, alpha, upper_p)?;
    let mut labels = Vec::with_capacity(row_count);
    for r in 0..row_count {
        let rt = xy.row_titles.get(r).map(String::as_str).unwrap_or("").trim();
        if !rt.is_empty() {
            labels.push(rt.to_owned());
        } else if let Ok(Some(x)) = xy.x_summary_at(r)
            && x.is_finite()
        {
            let norm = if x == 0.0 { 0.0 } else { x };
            labels.push(format!("{norm}"));
        } else {
            labels.push(format!("Row {}", r + 1));
        }
    }
    Ok((series, labels, non_pos, pre_sum))
}

fn extract_grouped_row_stats(
    g: &GroupedTable,
    selected: &[usize],
    alpha: f64,
    upper_p: f64,
) -> Result<(Vec<SeriesRowStats>, Vec<String>, usize, bool), TransformError> {
    let row_count = g.row_count();
    let (series, non_pos, pre_sum) = extract_dataset_series(&g.data_sets, g.subcolumn_mode, row_count, selected, alpha, upper_p)?;
    let labels = fallback_row_labels(&g.row_titles, row_count);
    Ok((series, labels, non_pos, pre_sum))
}

fn extract_column_row_stats(
    col: &ColumnTable,
    selected: &[usize],
    alpha: f64,
    upper_p: f64,
) -> Result<(Vec<SeriesRowStats>, Vec<String>, usize, bool), TransformError> {
    let row_count = col.row_count();
    let mut rows = Vec::with_capacity(row_count);
    let mut non_pos = 0_usize;
    let is_pre_sum = col.subcolumn_mode.is_pre_summarized();

    for r in 0..row_count {
        let mut active = Vec::with_capacity(selected.len());
        for &c in selected {
            if let Some(subcells) = col.data_sets.get(c).and_then(|ds| ds.cells.get(r)) {
                if is_pre_sum {
                    if let Some(v) = subcells.first().and_then(Cell::active_number) {
                        active.push(v);
                    }
                } else {
                    for cell in subcells {
                        if let Some(v) = cell.active_number() {
                            active.push(v);
                        }
                    }
                }
            }
        }
        let m = RowMetrics::from_raw_replicates(&active, alpha, upper_p);
        if m.has_non_positive {
            non_pos += 1;
        }
        rows.push(m);
    }
    let labels = fallback_row_labels(&col.row_titles, row_count);
    Ok((vec![SeriesRowStats { title: String::from("Row Summary"), rows }], labels, non_pos, false))
}

fn extract_mv_row_stats(
    mv: &MultipleVariablesTable,
    selected: &[usize],
    alpha: f64,
    upper_p: f64,
) -> (Vec<SeriesRowStats>, Vec<String>, usize, bool) {
    let row_count = mv.row_count();
    let mut rows = Vec::with_capacity(row_count);
    let mut non_pos = 0_usize;
    for r in 0..row_count {
        let mut active = Vec::with_capacity(selected.len());
        for &c in selected {
            if let Some(v) = mv.variables.get(c).and_then(|var| var.cells.get(r)).and_then(Cell::active_number) {
                active.push(v);
            }
        }
        let m = RowMetrics::from_raw_replicates(&active, alpha, upper_p);
        if m.has_non_positive {
            non_pos += 1;
        }
        rows.push(m);
    }
    let labels = fallback_row_labels(&mv.row_titles, row_count);
    (vec![SeriesRowStats { title: String::from("Row Summary"), rows }], labels, non_pos, false)
}

fn fallback_row_labels(row_titles: &[String], row_count: usize) -> Vec<String> {
    (0..row_count)
        .map(|r| {
            let t = row_titles.get(r).map(String::as_str).unwrap_or("").trim();
            if t.is_empty() { format!("Row {}", r + 1) } else { t.to_owned() }
        })
        .collect()
}

fn num_or_empty(val: Option<f64>) -> ResultCell {
    match val {
        Some(v) if v.is_finite() => ResultCell::Number { value: v, prefix: None, units: None },
        _ => ResultCell::Empty,
    }
}

fn cell_or_empty(val: Option<f64>) -> Cell {
    match val {
        Some(v) if v.is_finite() => Cell::number(v),
        _ => Cell::empty(),
    }
}

fn summary_field_headers_clean(summary: RowSummaryKind, upper_p: f64) -> Vec<String> {
    let lower_p = 100.0 - upper_p;
    match summary {
        RowSummaryKind::Total => vec![String::from("Total"), String::from("N")],
        RowSummaryKind::MeanSd => vec![String::from("Mean"), String::from("SD"), String::from("N")],
        RowSummaryKind::MeanSem => vec![String::from("Mean"), String::from("SEM"), String::from("N")],
        RowSummaryKind::MeanCv => vec![String::from("Mean"), String::from("%CV"), String::from("N")],
        RowSummaryKind::MeanCi => vec![String::from("Mean"), String::from("Lower CI"), String::from("Upper CI"), String::from("N")],
        RowSummaryKind::MeanMinMax => vec![String::from("Mean"), String::from("Min"), String::from("Max"), String::from("Range"), String::from("N")],
        RowSummaryKind::MedianQuartiles => {
            vec![String::from("Median"), String::from("Q1"), String::from("Q3"), String::from("IQR"), String::from("N")]
        }
        RowSummaryKind::MedianMinMax => {
            vec![String::from("Median"), String::from("Min"), String::from("Max"), String::from("Range"), String::from("N")]
        }
        RowSummaryKind::MedianPercentiles => {
            vec![String::from("Median"), format_percentile_label(lower_p), format_percentile_label(upper_p), String::from("N")]
        }
        RowSummaryKind::GeometricMeanSd => vec![String::from("Geometric Mean"), String::from("Geometric SD Factor"), String::from("N")],
        RowSummaryKind::GeometricMeanCi => {
            vec![String::from("Geometric Mean"), String::from("Lower CI"), String::from("Upper CI"), String::from("N")]
        }
    }
}

fn format_percentile_label(p: f64) -> String {
    if (p - p.round()).abs() < 1e-9 {
        #[allow(clippy::cast_possible_truncation)]
        let int_p = p.round() as i64;
        format!("P{int_p}")
    } else {
        format!("P{p:.2}")
    }
}

fn push_metrics_cells(m: &RowMetrics, summary: RowSummaryKind, out: &mut Vec<ResultCell>) {
    #[allow(clippy::cast_precision_loss)]
    let n_cell = ResultCell::Number { value: m.n as f64, prefix: None, units: None };
    match summary {
        RowSummaryKind::Total => {
            out.push(num_or_empty(m.total));
            out.push(n_cell);
        }
        RowSummaryKind::MeanSd => {
            out.push(num_or_empty(m.mean));
            out.push(num_or_empty(m.sd));
            out.push(n_cell);
        }
        RowSummaryKind::MeanSem => {
            out.push(num_or_empty(m.mean));
            out.push(num_or_empty(m.sem));
            out.push(n_cell);
        }
        RowSummaryKind::MeanCv => {
            out.push(num_or_empty(m.mean));
            out.push(num_or_empty(m.cv_percent));
            out.push(n_cell);
        }
        RowSummaryKind::MeanCi => {
            out.push(num_or_empty(m.mean));
            out.push(num_or_empty(m.mean_ci.map(|(lo, _)| lo)));
            out.push(num_or_empty(m.mean_ci.map(|(_, hi)| hi)));
            out.push(n_cell);
        }
        RowSummaryKind::MeanMinMax => {
            out.push(num_or_empty(m.mean));
            out.push(num_or_empty(m.min));
            out.push(num_or_empty(m.max));
            out.push(num_or_empty(m.range));
            out.push(n_cell);
        }
        RowSummaryKind::MedianQuartiles => {
            out.push(num_or_empty(m.median));
            out.push(num_or_empty(m.q1));
            out.push(num_or_empty(m.q3));
            out.push(num_or_empty(m.iqr));
            out.push(n_cell);
        }
        RowSummaryKind::MedianMinMax => {
            out.push(num_or_empty(m.median));
            out.push(num_or_empty(m.min));
            out.push(num_or_empty(m.max));
            out.push(num_or_empty(m.range));
            out.push(n_cell);
        }
        RowSummaryKind::MedianPercentiles => {
            out.push(num_or_empty(m.median));
            out.push(num_or_empty(m.p_low));
            out.push(num_or_empty(m.p_high));
            out.push(n_cell);
        }
        RowSummaryKind::GeometricMeanSd => {
            out.push(num_or_empty(m.geo_mean));
            out.push(num_or_empty(m.geo_sd_factor));
            out.push(n_cell);
        }
        RowSummaryKind::GeometricMeanCi => {
            out.push(num_or_empty(m.geo_mean));
            out.push(num_or_empty(m.geo_ci.map(|(lo, _)| lo)));
            out.push(num_or_empty(m.geo_ci.map(|(_, hi)| hi)));
            out.push(n_cell);
        }
    }
}

fn build_row_stats_tab(
    series_list: &[SeriesRowStats],
    row_labels: &[String],
    summary: RowSummaryKind,
    conf: f64,
    upper_p: f64,
    non_pos_rows: usize,
    pre_summarized_note: bool,
) -> ResultTab {
    let fields = summary_field_headers_clean(summary, upper_p);
    let multi = series_list.len() > 1;
    let mut column_headers = Vec::with_capacity(series_list.len() * fields.len());
    for s in series_list {
        for f in &fields {
            if multi {
                column_headers.push(format!("{}: {f}", s.title));
            } else {
                column_headers.push(f.clone());
            }
        }
    }

    let row_count = row_labels.len();
    let mut rows = Vec::with_capacity(row_count);
    for r in 0..row_count {
        let mut row_cells = Vec::with_capacity(column_headers.len());
        for s in series_list {
            let m = s.rows.get(r).copied().unwrap_or_default();
            push_metrics_cells(&m, summary, &mut row_cells);
        }
        rows.push(row_cells);
    }

    let mut footnotes = Vec::new();
    match summary {
        RowSummaryKind::MedianQuartiles => {
            footnotes.push(String::from("Quartiles Q1 and Q3 use Hyndman & Fan (1996) Type-6 interpolation; IQR = Q3 - Q1."));
        }
        RowSummaryKind::MedianPercentiles => {
            let lower_p = 100.0 - upper_p;
            footnotes.push(format!(
                "Custom symmetric percentiles ({} and {}) use Hyndman & Fan (1996) Type-7 linear interpolation.",
                format_percentile_label(lower_p),
                format_percentile_label(upper_p)
            ));
        }
        RowSummaryKind::MeanCi | RowSummaryKind::GeometricMeanCi => {
            footnotes.push(format!("Two-sided Student-t confidence intervals computed at {:.1}% confidence (df = N - 1).", conf * 100.0));
        }
        _ => {}
    }
    if matches!(summary, RowSummaryKind::GeometricMeanSd | RowSummaryKind::GeometricMeanCi) && non_pos_rows > 0 {
        footnotes.push(format!(
            "Geometric statistics left blank for {non_pos_rows} row(s) containing zero or negative observations (strict positivity required)."
        ));
    }
    if pre_summarized_note {
        footnotes.push(String::from("Input table is pre-summarized; order-statistic and geometric summaries require raw replicate subcolumns."));
    }

    ResultTab {
        id: String::from("row-statistics"),
        title: String::from("Row statistics"),
        kind: ResultTabKind::MainSummary,
        column_headers,
        row_labels: row_labels.to_vec(),
        rows,
        footnotes,
    }
}

fn target_subcolumn_mode(summary: RowSummaryKind, for_grouped: bool) -> SubcolumnMode {
    match summary {
        RowSummaryKind::MeanSd | RowSummaryKind::GeometricMeanSd => SubcolumnMode::MeanSdN,
        RowSummaryKind::MeanSem => SubcolumnMode::MeanSemN,
        RowSummaryKind::MeanCv => SubcolumnMode::MeanCvN,
        RowSummaryKind::MeanCi
        | RowSummaryKind::MeanMinMax
        | RowSummaryKind::MedianMinMax
        | RowSummaryKind::MedianPercentiles
        | RowSummaryKind::GeometricMeanCi => SubcolumnMode::UpperLowerLimits,
        RowSummaryKind::MedianQuartiles => SubcolumnMode::MedianIqr,
        RowSummaryKind::Total => {
            if for_grouped {
                SubcolumnMode::RawReplicates(1)
            } else {
                SubcolumnMode::Single
            }
        }
    }
}

fn metrics_to_subcells(m: &RowMetrics, summary: RowSummaryKind) -> Vec<Cell> {
    #[allow(clippy::cast_precision_loss)]
    let n_cell = if m.n > 0 { Cell::number(m.n as f64) } else { Cell::empty() };
    match summary {
        RowSummaryKind::Total => vec![cell_or_empty(m.total)],
        RowSummaryKind::MeanSd => vec![cell_or_empty(m.mean), cell_or_empty(m.sd), n_cell],
        RowSummaryKind::MeanSem => vec![cell_or_empty(m.mean), cell_or_empty(m.sem), n_cell],
        RowSummaryKind::MeanCv => vec![cell_or_empty(m.mean), cell_or_empty(m.cv_percent), n_cell],
        RowSummaryKind::MeanCi => vec![cell_or_empty(m.mean), cell_or_empty(m.mean_ci.map(|(lo, _)| lo)), cell_or_empty(m.mean_ci.map(|(_, hi)| hi))],
        RowSummaryKind::MeanMinMax => vec![cell_or_empty(m.mean), cell_or_empty(m.min), cell_or_empty(m.max)],
        RowSummaryKind::MedianQuartiles => vec![cell_or_empty(m.median), cell_or_empty(m.q1), cell_or_empty(m.q3)],
        RowSummaryKind::MedianMinMax => vec![cell_or_empty(m.median), cell_or_empty(m.min), cell_or_empty(m.max)],
        RowSummaryKind::MedianPercentiles => vec![cell_or_empty(m.median), cell_or_empty(m.p_low), cell_or_empty(m.p_high)],
        RowSummaryKind::GeometricMeanSd => {
            let n_geo = if m.geo_mean.is_some() { n_cell } else { Cell::empty() };
            vec![cell_or_empty(m.geo_mean), cell_or_empty(m.geo_sd_factor), n_geo]
        }
        RowSummaryKind::GeometricMeanCi => {
            vec![cell_or_empty(m.geo_mean), cell_or_empty(m.geo_ci.map(|(lo, _)| lo)), cell_or_empty(m.geo_ci.map(|(_, hi)| hi))]
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_analyzable_sheet(
    source: &DataSheet,
    analysis: &AnalysisSpec,
    summary: RowSummaryKind,
    selected: &[usize],
    series_list: &[SeriesRowStats],
    row_labels: &[String],
    upper_p: f64,
    ctx: &TransformContext<'_>,
) -> Result<DataSheet, TransformError> {
    let data_id = ctx.output_id.clone().unwrap_or_else(|| bounded_suffixed_str(analysis.id.as_str(), "-row-stats", MAX_SHEET_ID_BYTES));
    let data_title =
        ctx.output_title.clone().unwrap_or_else(|| bounded_suffixed_str(analysis.effective_title(), " (Row Statistics)", MAX_TITLE_BYTES));

    let archetype = match &source.archetype {
        TableArchetype::Xy(xy) => {
            let row_count = xy.row_count();
            let sub_mode = target_subcolumn_mode(summary, false);
            let mut out_xy = XyTable::new_with_x_mode(xy.x_mode.clone(), sub_mode, selected.len(), row_count)?;
            out_xy.x_title = xy.x_title.clone();
            out_xy.x_cells = xy.x_cells.clone();
            out_xy.row_titles = xy.row_titles.clone();
            out_xy.row_titles_expanded = xy.row_titles_expanded;
            for (dst_idx, series) in series_list.iter().enumerate() {
                if let Some(ds) = out_xy.data_sets.get_mut(dst_idx) {
                    ds.title = series.title.clone();
                    for (r, m) in series.rows.iter().enumerate() {
                        if let Some(slot) = ds.cells.get_mut(r) {
                            *slot = metrics_to_subcells(m, summary);
                        }
                    }
                }
            }
            TableArchetype::Xy(out_xy)
        }
        TableArchetype::Grouped(g) => {
            let row_count = g.row_count();
            let sub_mode = target_subcolumn_mode(summary, true);
            let mut out_g = GroupedTable::new(sub_mode, selected.len(), row_count)?;
            out_g.row_factor_title = g.row_factor_title.clone();
            out_g.column_factor_title = g.column_factor_title.clone();
            out_g.row_titles = g.row_titles.clone();
            out_g.row_numeric_spacing = g.row_numeric_spacing.clone();
            out_g.replicate_stacking = ReplicateStacking::SideBySide;
            for (dst_idx, series) in series_list.iter().enumerate() {
                if let Some(ds) = out_g.data_sets.get_mut(dst_idx) {
                    ds.title = series.title.clone();
                    for (r, m) in series.rows.iter().enumerate() {
                        if let Some(slot) = ds.cells.get_mut(r) {
                            *slot = metrics_to_subcells(m, summary);
                        }
                    }
                }
            }
            TableArchetype::Grouped(out_g)
        }
        TableArchetype::Column(c)
            if matches!(summary, RowSummaryKind::Total | RowSummaryKind::MeanSd | RowSummaryKind::MeanSem | RowSummaryKind::MeanCv) =>
        {
            let row_count = c.row_count();
            let sub_mode = target_subcolumn_mode(summary, false);
            let design_hint =
                if sub_mode == SubcolumnMode::Single { ColumnDesignHint::UnpairedReplicates } else { ColumnDesignHint::PrecomputedError };
            let mut ds = DataSetColumn::new(String::from("Row Summary"), row_count, sub_mode.subcolumn_count())?;
            if let Some(series) = series_list.first() {
                for (r, m) in series.rows.iter().enumerate() {
                    if let Some(slot) = ds.cells.get_mut(r) {
                        *slot = metrics_to_subcells(m, summary);
                    }
                }
            }
            let out_col = ColumnTable {
                design_hint,
                subcolumn_mode: sub_mode,
                row_titles: c.row_titles.clone(),
                row_titles_expanded: c.row_titles_expanded,
                data_sets: vec![ds],
            };
            TableArchetype::Column(out_col)
        }
        TableArchetype::Column(_) | TableArchetype::MultipleVariables(_) => build_mv_row_stats_archetype(series_list, row_labels, summary, upper_p)?,
        _ => {
            return Err(TransformError::IncompatibleTable { operation: "Row Statistics", kind: source.kind() });
        }
    };

    let sheet = DataSheet::new(data_id, data_title, archetype)?;
    sheet.validate()?;
    Ok(sheet)
}

fn build_mv_row_stats_archetype(
    series_list: &[SeriesRowStats],
    row_labels: &[String],
    summary: RowSummaryKind,
    upper_p: f64,
) -> Result<TableArchetype, TransformError> {
    let row_count = row_labels.len();
    let fields = summary_field_headers_clean(summary, upper_p);
    let series = series_list.first();
    let mut variables = Vec::with_capacity(fields.len());
    for title in &fields {
        let mut col = VariableColumn::new(title.clone(), row_count)?;
        col.lock_role(VariableRole::Continuous);
        variables.push(col);
    }
    if let Some(s) = series {
        for (r, m) in s.rows.iter().enumerate() {
            let mut cells = Vec::with_capacity(fields.len());
            push_metrics_cells(m, summary, &mut cells);
            for (c_idx, rcell) in cells.into_iter().enumerate() {
                if let Some(var) = variables.get_mut(c_idx)
                    && let Some(slot) = var.cells.get_mut(r)
                {
                    *slot = match rcell {
                        ResultCell::Number { value, .. } => Cell::number(value),
                        _ => Cell::empty(),
                    };
                }
            }
        }
    }
    let mv = MultipleVariablesTable { row_titles: row_labels.to_vec(), row_titles_expanded: !row_labels.is_empty(), variables };
    Ok(TableArchetype::MultipleVariables(mv))
}
