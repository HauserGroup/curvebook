//! Curve calculus: Area Under the Curve (AUC) with peak detection and SE propagation,
//! Savitzky-Golay / moving-average / LOWESS smoothing, differentiation, and integration.
//!
//! References (D1 citation pass 2026-11-18; Savitzky–Golay sources are cited inline):
//! - Yeh, K. C., & Kwan, K. C. (1978). A comparison of numerical integrating algorithms by
//!   trapezoidal, Lagrange, and spline approximation. *Journal of Pharmacokinetics and
//!   Biopharmaceutics*, 6(1), 79-98. `doi:10.1007/bf01066064`.
//! - Bailer, A. J. (1988). Testing for the equality of area under the curves when using destructive
//!   measurement techniques. *Journal of Pharmacokinetics and Biopharmaceutics*, 16(3), 303-309.
//!   `doi:10.1007/bf01062139`.
//! - Cleveland, W. S. (1979). Robust locally weighted regression and smoothing scatterplots.
//!   *Journal of the American Statistical Association*, 74(368), 829-836.
//!   `doi:10.1080/01621459.1979.10481038`.

use curvebook_doc::{
    AnalysisSpec, AreaUnderCurveSpec, AucBaseline, Cell, ChecklistItem, ChecklistStatus, DataSheet, MultipleVariablesTable, ResultCell,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SplineLowessSpec, SubcolumnMode, SummarizedPoint, TableArchetype,
    VariableColumn, VariableRole, XColumnMode, XyTable,
};
use curvebook_math::{DenseMatrix, solve_linear_least_squares, sum};

use crate::error::TransformError;
use crate::spline::fit_lowess_sorted_values;

/// Options controlling Area Under the Curve and peak detection.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AucOptions {
    /// Horizontal baseline threshold (default `0.0`).
    pub baseline: f64,
    /// Minimum peak height above baseline required to report a peak (default `0.0`).
    pub min_peak_height: f64,
    /// Minimum fraction of total positive area required to retain a peak (default `0.0`).
    pub min_area_fraction: f64,
}

impl Default for AucOptions {
    fn default() -> Self {
        Self { baseline: 0.0, min_peak_height: 0.0, min_area_fraction: 0.0 }
    }
}

/// Extended Area Under the Curve options supporting [`AucBaseline`], partial-window integration,
/// and design-gated Bailer (1988) standard-error propagation (`R17.7` / `R19.2`).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AucWindowOptions {
    /// Horizontal reference baseline policy (`Fixed { value }` or `FirstPoint`).
    pub baseline: AucBaseline,
    /// Optional inclusive lower *X* integration bound (linearly interpolated when falling between knots).
    pub x_min: Option<f64>,
    /// Optional inclusive upper *X* integration bound (linearly interpolated when falling between knots).
    pub x_max: Option<f64>,
    /// Whether time-point means come from independent cohorts/subjects (Bailer 1988).
    /// When `false`, `standard_error` is suppressed (`None`).
    pub compute_bailer_se: bool,
    /// Minimum peak height above the resolved baseline required to retain a peak (`>= 0.0`).
    pub min_peak_height: f64,
    /// Minimum fraction of total positive area in `[0.0, 1.0]` required to retain a peak.
    pub min_area_fraction: f64,
}

impl Default for AucWindowOptions {
    fn default() -> Self {
        Self { baseline: AucBaseline::default(), x_min: None, x_max: None, compute_bailer_se: false, min_peak_height: 0.0, min_area_fraction: 0.0 }
    }
}

impl From<&AucOptions> for AucWindowOptions {
    fn from(opts: &AucOptions) -> Self {
        Self {
            baseline: AucBaseline::Fixed { value: opts.baseline },
            x_min: None,
            x_max: None,
            compute_bailer_se: true,
            min_peak_height: opts.min_peak_height,
            min_area_fraction: opts.min_area_fraction,
        }
    }
}

impl From<&AreaUnderCurveSpec> for AucWindowOptions {
    fn from(spec: &AreaUnderCurveSpec) -> Self {
        Self {
            baseline: spec.baseline,
            x_min: spec.x_min,
            x_max: spec.x_max,
            compute_bailer_se: spec.independent_time_points,
            min_peak_height: spec.min_peak_height,
            min_area_fraction: spec.min_area_fraction,
        }
    }
}

/// Partial-window Area Under the Curve result with resolved bounds, baseline, linear-functional
/// knot weights, and missing-SE diagnostics (`R17.7` / `R19.2`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowedAucResult {
    /// Core trapezoidal area decomposition, clipped peak list, and design-gated Bailer SE (`standard_error`).
    pub auc: AucResult,
    /// Effective lower *X* coordinate of the integrated interval.
    pub start_x: f64,
    /// Effective upper *X* coordinate of the integrated interval.
    pub end_x: f64,
    /// Effective integration width `end_x - start_x > 0.0`.
    pub span: f64,
    /// Number of distinct sampled knots contributing to the clipped integration window.
    pub n_points: usize,
    /// Resolved numeric baseline *Y* used for area partitioning and peak detection.
    pub resolved_baseline: f64,
    /// *X* coordinates of contributing knots that lacked a finite point SE (for example `n = 1` replicate).
    pub missing_se_x: Vec<f64>,
    /// Linear-functional weights `(x_k, weight_k)` on contributing knots for `net_area`
    /// (sum equals `span` under `Fixed` baseline and `0.0` under `FirstPoint`).
    pub knot_weights: Vec<(f64, f64)>,
}

/// Summary of a single detected peak excursion above the baseline.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PeakSummary {
    /// Independent *X* coordinate at the maximum height of the excursion.
    pub peak_x: f64,
    /// Raw *Y* coordinate at the peak apex (`peak_x`).
    pub peak_y: f64,
    /// Vertical amplitude above baseline (`peak_y - baseline >= 0.0`).
    pub height_above_baseline: f64,
    /// Interpolated *X* coordinate where the curve crosses upward above `baseline` (or first sample *X* if already above `baseline`).
    pub start_x: f64,
    /// Interpolated *X* coordinate where the curve crosses back down to `baseline` (or last sample *X* if still above `baseline`).
    pub end_x: f64,
    /// Positive trapezoidal area (`>= 0.0`, in `X * Y` units) enclosed between `[start_x, end_x]` and `baseline`.
    pub area: f64,
    /// Share `area / positive_area` in `[0.0, 1.0]` (`0.0` when `positive_area == 0.0`).
    pub area_fraction: f64,
}

/// Complete Area Under the Curve result with baseline partitioning, peak list, and optional SE.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AucResult {
    /// Total unsigned excursion area `positive_area + negative_area >= 0.0` relative to `baseline` (`X * Y` units).
    pub total_area: f64,
    /// Cumulative trapezoidal area (`>= 0.0`) of all curve segments lying above `baseline` (`Y > baseline`).
    pub positive_area: f64,
    /// Cumulative unsigned trapezoidal area (`>= 0.0`) of all curve segments lying below `baseline` (`Y < baseline`).
    pub negative_area: f64,
    /// Signed net trapezoidal integral `positive_area - negative_area = integral (Y(x) - baseline) dx`.
    pub net_area: f64,
    /// Sum of `area` across all retained [`Self::peaks`] passing `min_peak_height` and `min_area_fraction`.
    pub total_peak_area: f64,
    /// Analytical Bailer (1988) standard error `sqrt(sum_i w_i^2 SE_i^2)` of `net_area` assuming independent point estimates; `None` when point SEs were not supplied (`compute_auc` or single-replicate series).
    pub standard_error: Option<f64>,
    /// Chronologically ordered (`start_x` ascending) positive excursions above `baseline` meeting height and area-fraction filters.
    pub peaks: Vec<PeakSummary>,
}

/// Smoothing algorithm selection.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SmoothMethod {
    /// 5-point quadratic/cubic Savitzky–Golay local polynomial filter (Savitzky & Golay 1964).
    SavitzkyGolay5,
    /// 7-point quadratic/cubic Savitzky–Golay local polynomial filter (Savitzky & Golay 1964).
    SavitzkyGolay7,
    /// 9-point quadratic/cubic Savitzky–Golay local polynomial filter (Savitzky & Golay 1964).
    SavitzkyGolay9,
    /// Symmetric unweighted sliding-window arithmetic mean.
    MovingAverage {
        /// Odd/positive window width (`>= 1` points).
        window: usize,
    },
    /// Locally weighted linear regression smoother with tricube kernel and optional bisquare robustness iterations (Cleveland 1979).
    Lowess {
        /// Fraction of sample points in `(0.0, 1.0]` included in each local neighborhood fit.
        span: f64,
        /// Number of Tukey bisquare outlier-downweighting iterations (`0` for standard unweighted LOESS, `2`–`3` for robust LOWESS).
        robustness_iters: usize,
    },
}

/// Numerical differentiation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DerivativeOrder {
    /// First derivative `dY / dX` via non-uniform three-point Lagrange finite differences (`Y / X` units).
    First,
    /// Second derivative `d^2Y / dX^2` (`Y / X^2` units).
    Second,
}

#[derive(Debug, Clone, Copy)]
struct CurvePoint {
    x: f64,
    y: f64,
    se: Option<f64>,
}

/// Compute linear trapezoidal Area Under the Curve (Yeh & Kwan 1978) and peak summaries from
/// `(x, y)` pairs.
pub fn compute_auc(points: &[(f64, f64)], opts: &AucOptions) -> Result<AucResult, TransformError> {
    let pts: Vec<(f64, f64, Option<f64>)> = points.iter().map(|&(x, y)| (x, y, None)).collect();
    let window_opts = AucWindowOptions::from(opts);
    compute_windowed_auc(&pts, &window_opts).map(|res| res.auc)
}

/// Compute trapezoidal AUC with analytical standard-error propagation from `(x, y, se)` tuples.
///
/// `Var(AUC) = sum_i w_i^2 SE_i^2` with trapezoid weights `w_i` (Bailer 1988). This assumes the
/// point estimates are independent (separate cohorts/subjects per time point); omitting
/// within-subject or within-well covariance on repeated measurements can bias the SE in either
/// direction.
pub fn compute_auc_with_se(points: &[(f64, f64, f64)], opts: &AucOptions) -> Result<AucResult, TransformError> {
    let pts: Vec<(f64, f64, Option<f64>)> = points.iter().map(|&(x, y, se)| (x, y, Some(se))).collect();
    let window_opts = AucWindowOptions::from(opts);
    compute_windowed_auc(&pts, &window_opts).map(|res| res.auc)
}

/// Compute partial-window trapezoidal AUC (Yeh & Kwan 1978), clipped peak summaries, and
/// design-gated Bailer (1988) standard error from `(x, y, optional_se)` tuples (`R17.7` / `R19.2`).
pub fn compute_windowed_auc(points: &[(f64, f64, Option<f64>)], opts: &AucWindowOptions) -> Result<WindowedAucResult, TransformError> {
    validate_window_options(opts)?;
    let sorted = prepare_sorted_points(points)?;
    let knots = collapse_tied_auc_points(&sorted)?;
    compute_windowed_auc_on_knots(knots, opts).map(|internal| internal.result).map_err(|message| TransformError::InvalidInput { message })
}

/// Compute AUC for a single dataset in an `XyTable` [`DataSheet`].
pub fn compute_sheet_auc(sheet: &DataSheet, data_set: usize, opts: &AucOptions) -> Result<AucResult, TransformError> {
    let TableArchetype::Xy(xy) = &sheet.archetype else {
        return Err(TransformError::IncompatibleTable { operation: "AUC", kind: sheet.kind() });
    };
    if data_set >= xy.data_sets.len() {
        return Err(TransformError::InvalidInput { message: format!("dataset index {data_set} out of bounds") });
    }
    let window_opts = AucWindowOptions::from(opts);
    validate_window_options(&window_opts)?;
    let extracted = collect_sheet_auc_series(xy, data_set)?;
    compute_windowed_auc_on_knots(extracted.knots, &window_opts)
        .map(|internal| internal.result.auc)
        .map_err(|message| TransformError::InvalidInput { message })
}

/// Execute a first-class [`AreaUnderCurveSpec`] analysis against an `XY` [`DataSheet`] and return
/// a populated [`ResultSheet`] with `auc-summary`, optional `auc-peaks`, hookable scalars,
/// `analyzable_data`, and `QOL-7` checklist diagnostics (`R17.7` / `R19.2`).
pub fn execute_area_under_curve(source: &DataSheet, analysis: &AnalysisSpec, spec: &AreaUnderCurveSpec) -> Result<ResultSheet, TransformError> {
    let TableArchetype::Xy(xy) = &source.archetype else {
        return Err(TransformError::IncompatibleTable { operation: "Area Under the Curve", kind: source.kind() });
    };
    let window_opts = AucWindowOptions::from(spec);
    validate_window_options(&window_opts)?;

    let active_cols = resolve_auc_columns(xy.data_sets.len(), &analysis.selected_data_sets)?;
    if active_cols.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("XY table has no datasets for Area Under the Curve") });
    }

    let non_numeric_x_rows = count_non_numeric_x_rows(xy);
    let mut outcomes = Vec::with_capacity(active_cols.len());
    let mut succeeded = 0_usize;

    for &ds_idx in &active_cols {
        let title = xy.data_sets[ds_idx].title.clone();
        let extracted = collect_sheet_auc_series(xy, ds_idx)?;
        let orig_knot_xs: Vec<f64> = extracted.knots.iter().map(|p| p.x).collect();
        let outcome = if extracted.knots.len() < 2 {
            let reason = if non_numeric_x_rows > 0 {
                String::from("Fewer than 2 distinct finite numeric X points (non-numeric X cells were ignored)")
            } else {
                String::from("Fewer than 2 distinct finite X points")
            };
            Err(reason)
        } else {
            match compute_windowed_auc_on_knots(extracted.knots, &window_opts) {
                Ok(internal) => {
                    succeeded += 1;
                    let bridged_interior_rows =
                        count_bridged_interior_rows(xy, &extracted.active_rows, &orig_knot_xs, internal.first_contrib_idx, internal.last_contrib_idx);
                    Ok(EvaluatedSeriesAuc { result: internal.result, bridged_interior_rows })
                }
                Err(reason) => Err(reason),
            }
        };
        outcomes.push(SeriesAucOutcome { ds_idx, title, outcome });
    }

    if succeeded == 0 {
        let first_reason = outcomes
            .iter()
            .find_map(|o| o.outcome.as_ref().err().cloned())
            .unwrap_or_else(|| String::from("no selected series produced a valid AUC"));
        let hint = if non_numeric_x_rows > 0 {
            " Area Under the Curve requires numeric X coordinates (for example seconds or minutes rather than text timestamps)."
        } else {
            ""
        };
        return Err(TransformError::InvalidInput {
            message: format!("Area Under the Curve failed for all {} selected series: {first_reason}.{hint}", outcomes.len()),
        });
    }

    let mut sheet = ResultSheet::new(analysis.clone())?;
    sheet.status = ResultStatus::UpToDate;
    sheet.tabs.push(build_auc_summary_tab(&outcomes, spec));
    if let Some(peaks_tab) = build_auc_peaks_tab(&outcomes, spec) {
        sheet.tabs.push(peaks_tab);
    }
    push_auc_scalars(&mut sheet.scalars, &outcomes);
    sheet.analyzable_data = Some(build_auc_analyzable_sheet(analysis, &outcomes)?);
    sheet.checklist = build_auc_checklist(&outcomes, spec, non_numeric_x_rows);
    Ok(sheet)
}

struct ExtractedSheetAucSeries {
    knots: Vec<CurvePoint>,
    active_rows: Vec<bool>,
}

fn collect_sheet_auc_series(xy: &XyTable, data_set: usize) -> Result<ExtractedSheetAucSeries, TransformError> {
    let Some(ds) = xy.data_sets.get(data_set) else {
        return Err(TransformError::InvalidInput { message: format!("dataset index {data_set} out of bounds") });
    };
    let mut active_rows = vec![false; xy.row_count()];
    let raw = if xy.y_subcolumn_mode.supports_parametric_moments() {
        collect_parametric_auc_points(xy, data_set, &mut active_rows)?
    } else if xy.y_subcolumn_mode.is_pre_summarized() {
        let mut central = Vec::new();
        for row in 0..xy.row_count() {
            let Some(x_raw) = xy.x_summary_at(row)?.filter(|v| v.is_finite()) else {
                continue;
            };
            let Some(subcells) = ds.cells.get(row) else {
                continue;
            };
            if let Some(center) = subcells.first().and_then(Cell::active_number) {
                active_rows[row] = true;
                central.push((normalize_zero(x_raw), center, None));
            }
        }
        central
    } else {
        collect_replicate_auc_points(xy, ds, &mut active_rows)?
    };
    let sorted = prepare_sorted_points(&raw)?;
    let knots = collapse_tied_auc_points(&sorted)?;
    Ok(ExtractedSheetAucSeries { knots, active_rows })
}

fn collect_parametric_auc_points(xy: &XyTable, data_set: usize, active_rows: &mut [bool]) -> Result<Vec<(f64, f64, Option<f64>)>, TransformError> {
    let mut rows: Vec<(f64, SummarizedPoint)> = Vec::new();
    for row in 0..xy.row_count() {
        let Some(x_raw) = xy.x_summary_at(row)?.filter(|v| v.is_finite()) else {
            continue;
        };
        if let Some(pt) = xy.summarized_y_point(row, data_set)?
            && pt.mean.is_finite()
            && pt.n.is_finite()
            && pt.n > 0.0
        {
            if let Some(flag) = active_rows.get_mut(row) {
                *flag = true;
            }
            rows.push((normalize_zero(x_raw), pt));
        }
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < rows.len() {
        let x_k = rows[i].0;
        let mut j = i + 1;
        while j < rows.len() && rows[j].0 == x_k {
            j += 1;
        }
        if j - i == 1 {
            let pt = &rows[i].1;
            let se = (pt.n >= 2.0).then(|| pt.sd / pt.n.sqrt());
            out.push((x_k, pt.mean, se));
        } else {
            let mut n_acc = sum::NeumaierSum::new();
            let mut wmean_acc = sum::NeumaierSum::new();
            let mut ssw_acc = sum::NeumaierSum::new();
            for &(_, pt) in &rows[i..j] {
                n_acc.add(pt.n);
                wmean_acc.add(pt.n * pt.mean);
                ssw_acc.add(pt.ss_within);
            }
            let n_tot = n_acc.total();
            let mean_p = wmean_acc.total() / n_tot;
            let mut ssb_acc = sum::NeumaierSum::new();
            for &(_, pt) in &rows[i..j] {
                let d = pt.mean - mean_p;
                ssb_acc.add(pt.n * d * d);
            }
            let se = if n_tot >= 2.0 {
                let sd_p = ((ssw_acc.total() + ssb_acc.total()) / (n_tot - 1.0)).max(0.0).sqrt();
                Some(sd_p / n_tot.sqrt())
            } else {
                None
            };
            out.push((x_k, mean_p, se));
        }
        i = j;
    }
    Ok(out)
}

fn collect_replicate_auc_points(
    xy: &XyTable,
    ds: &curvebook_doc::DataSetColumn,
    active_rows: &mut [bool],
) -> Result<Vec<(f64, f64, Option<f64>)>, TransformError> {
    let mut obs: Vec<(f64, f64)> = Vec::new();
    for row in 0..xy.row_count() {
        let Some(x_raw) = xy.x_summary_at(row)?.filter(|v| v.is_finite()) else {
            continue;
        };
        let Some(subcells) = ds.cells.get(row) else {
            continue;
        };
        let x = normalize_zero(x_raw);
        let mut saw_y = false;
        for y in subcells.iter().filter_map(Cell::active_number) {
            obs.push((x, y));
            saw_y = true;
        }
        if saw_y && let Some(flag) = active_rows.get_mut(row) {
            *flag = true;
        }
    }
    obs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < obs.len() {
        let x_k = obs[i].0;
        let mut j = i + 1;
        while j < obs.len() && obs[j].0 == x_k {
            j += 1;
        }
        let ys: Vec<f64> = obs[i..j].iter().map(|&(_, y)| y).collect();
        let mean = sum::mean(&ys)?;
        let se = if ys.len() >= 2 { Some(sum::SummaryMoments::from_slice(&ys)?.sem()?) } else { None };
        out.push((x_k, mean, se));
        i = j;
    }
    Ok(out)
}

fn boundary_tol(bound: f64) -> f64 {
    1e-12 * bound.abs().max(1.0)
}

fn normalize_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

fn validate_window_options(opts: &AucWindowOptions) -> Result<(), TransformError> {
    if let AucBaseline::Fixed { value } = opts.baseline
        && !value.is_finite()
    {
        return Err(TransformError::InvalidSpec { message: String::from("AUC baseline must be finite") });
    }
    if let Some(lo) = opts.x_min
        && !lo.is_finite()
    {
        return Err(TransformError::InvalidSpec { message: String::from("AUC x_min must be finite") });
    }
    if let Some(hi) = opts.x_max
        && !hi.is_finite()
    {
        return Err(TransformError::InvalidSpec { message: String::from("AUC x_max must be finite") });
    }
    if let (Some(lo), Some(hi)) = (opts.x_min, opts.x_max)
        && lo >= hi
    {
        return Err(TransformError::InvalidSpec { message: format!("AUC x_min ({lo}) must be strictly less than x_max ({hi})") });
    }
    if !opts.min_peak_height.is_finite() || opts.min_peak_height < 0.0 {
        return Err(TransformError::InvalidSpec { message: format!("AUC min_peak_height must be finite and >= 0, got {}", opts.min_peak_height) });
    }
    if !opts.min_area_fraction.is_finite() || !(0.0..=1.0).contains(&opts.min_area_fraction) {
        return Err(TransformError::InvalidSpec {
            message: format!("AUC min_area_fraction must be finite and in [0, 1], got {}", opts.min_area_fraction),
        });
    }
    Ok(())
}

struct WindowedAucInternal {
    result: WindowedAucResult,
    first_contrib_idx: usize,
    last_contrib_idx: usize,
}

fn resolve_window_bounds(pts: &mut [CurvePoint], x_min: Option<f64>, x_max: Option<f64>) -> Result<(f64, f64), String> {
    let last_idx = pts.len() - 1;
    let data_min = pts[0].x;
    let data_max = pts[last_idx].x;

    let start_x = match x_min {
        Some(req_min) => {
            let tol = boundary_tol(req_min);
            if data_min > req_min + tol {
                return Err(format!(
                    "lower X coverage failed: data starts at {}, requires <= {}",
                    format_compact_f64(data_min),
                    format_compact_f64(req_min)
                ));
            }
            if data_min > req_min {
                pts[0].x = normalize_zero(req_min);
            }
            normalize_zero(req_min)
        }
        None => data_min,
    };

    let end_x = match x_max {
        Some(req_max) => {
            let tol = boundary_tol(req_max);
            if data_max < req_max - tol {
                return Err(format!(
                    "upper X coverage failed: data ends at {}, requires >= {}",
                    format_compact_f64(data_max),
                    format_compact_f64(req_max)
                ));
            }
            if data_max < req_max {
                pts[last_idx].x = normalize_zero(req_max);
            }
            normalize_zero(req_max)
        }
        None => data_max,
    };

    if !(end_x > start_x && (end_x - start_x).is_finite()) {
        return Err(format!(
            "empty integration window: effective range [{}, {}] has non-positive width",
            format_compact_f64(start_x),
            format_compact_f64(end_x)
        ));
    }
    Ok((start_x, end_x))
}

fn compute_windowed_auc_on_knots(mut pts: Vec<CurvePoint>, opts: &AucWindowOptions) -> Result<WindowedAucInternal, String> {
    if pts.len() < 2 {
        return Err(String::from("AUC requires at least 2 distinct finite X points"));
    }
    let (start_x, end_x) = resolve_window_bounds(&mut pts, opts.x_min, opts.x_max)?;
    let span = normalize_zero(end_x - start_x);

    let n = pts.len();
    let mut weight_acc = vec![sum::NeumaierSum::new(); n];
    let mut contributing = vec![false; n];
    let mut clipped: Vec<CurvePoint> = Vec::with_capacity(n + 2);
    let mut first_active: Option<(usize, f64, f64)> = None;

    for i in 0..(n - 1) {
        let p0 = pts[i];
        let p1 = pts[i + 1];
        let dx = p1.x - p0.x;
        if dx <= 0.0 {
            continue;
        }
        let a = p0.x.max(start_x);
        let b = p1.x.min(end_x);
        if b <= a {
            continue;
        }
        let seg_len = b - a;
        let t_a = if a == p0.x { 0.0 } else { ((a - p0.x) / dx).clamp(0.0, 1.0) };
        let t_b = if b == p1.x { 1.0 } else { ((b - p0.x) / dx).clamp(0.0, 1.0) };
        let y_a = if a == p0.x { p0.y } else { (1.0 - t_a) * p0.y + t_a * p1.y };
        let y_b = if b == p1.x { p1.y } else { (1.0 - t_b) * p0.y + t_b * p1.y };
        let mid = 0.5 * (t_a + t_b);

        weight_acc[i].add(seg_len * (1.0 - mid));
        weight_acc[i + 1].add(seg_len * mid);
        contributing[i] = true;
        contributing[i + 1] = true;

        if first_active.is_none() {
            first_active = Some((i, t_a, y_a));
            clipped.push(CurvePoint { x: a, y: y_a, se: None });
        }
        clipped.push(CurvePoint { x: b, y: y_b, se: None });
    }

    let Some((k0, t0, y_start)) = first_active else {
        return Err(String::from("no curve segments overlap the requested X window"));
    };

    let resolved_baseline = match opts.baseline {
        AucBaseline::Fixed { value } => normalize_zero(value),
        AucBaseline::FirstPoint => {
            weight_acc[k0].add(-span * (1.0 - t0));
            weight_acc[k0 + 1].add(-span * t0);
            normalize_zero(y_start)
        }
    };

    let (pos_area, neg_area, raw_peaks) = integrate_segments_and_peaks(&clipped, resolved_baseline);
    let net_area = normalize_zero(pos_area - neg_area);
    let total_area = normalize_zero(pos_area + neg_area);

    let mut missing_se_x = Vec::new();
    let mut knot_weights = Vec::new();
    let mut first_contrib_idx = n;
    let mut last_contrib_idx = 0_usize;
    let mut var_acc = sum::NeumaierSum::new();

    for (idx, pt) in pts.iter().enumerate() {
        if !contributing[idx] {
            continue;
        }
        first_contrib_idx = first_contrib_idx.min(idx);
        last_contrib_idx = last_contrib_idx.max(idx);
        let w = normalize_zero(weight_acc[idx].total());
        knot_weights.push((pt.x, w));
        match pt.se {
            Some(se) => var_acc.add((w * se) * (w * se)),
            None => missing_se_x.push(pt.x),
        }
    }

    let standard_error = if opts.compute_bailer_se && missing_se_x.is_empty() { Some(normalize_zero(var_acc.total().max(0.0).sqrt())) } else { None };

    let qualifying: Vec<PeakSummary> = raw_peaks
        .into_iter()
        .filter(|p| p.height_above_baseline >= opts.min_peak_height)
        .filter(|p| if pos_area > 0.0 { (p.area / pos_area) >= opts.min_area_fraction } else { opts.min_area_fraction == 0.0 })
        .map(|mut p| {
            p.area_fraction = if pos_area > 0.0 { p.area / pos_area } else { 0.0 };
            p
        })
        .collect();
    let total_peak_area = normalize_zero(sum::sum(&qualifying.iter().map(|p| p.area).collect::<Vec<_>>()));
    let n_points = knot_weights.len();

    Ok(WindowedAucInternal {
        result: WindowedAucResult {
            auc: AucResult {
                total_area,
                positive_area: normalize_zero(pos_area),
                negative_area: normalize_zero(neg_area),
                net_area,
                total_peak_area,
                standard_error,
                peaks: qualifying,
            },
            start_x,
            end_x,
            span,
            n_points,
            resolved_baseline,
            missing_se_x,
            knot_weights,
        },
        first_contrib_idx,
        last_contrib_idx,
    })
}

fn collapse_tied_auc_points(pts: &[CurvePoint]) -> Result<Vec<CurvePoint>, TransformError> {
    let mut knots = Vec::with_capacity(pts.len());
    let mut i = 0_usize;
    while i < pts.len() {
        let x_k = pts[i].x;
        let mut j = i + 1;
        while j < pts.len() && pts[j].x == x_k {
            j += 1;
        }
        if j - i == 1 {
            knots.push(pts[i]);
        } else {
            let slice = &pts[i..j];
            let ys: Vec<f64> = slice.iter().map(|p| p.y).collect();
            let mean_y = sum::mean(&ys)?;
            let se = if slice.iter().all(|p| p.se.is_some()) {
                let mut var_acc = sum::NeumaierSum::new();
                for p in slice {
                    let s = p.se.unwrap_or(0.0);
                    var_acc.add(s * s);
                }
                #[allow(clippy::cast_precision_loss)]
                let m = slice.len() as f64;
                Some(var_acc.total().max(0.0).sqrt() / m)
            } else {
                None
            };
            knots.push(CurvePoint { x: x_k, y: mean_y, se });
        }
        i = j;
    }
    Ok(knots)
}

fn prepare_sorted_points(raw: &[(f64, f64, Option<f64>)]) -> Result<Vec<CurvePoint>, TransformError> {
    let mut pts = Vec::with_capacity(raw.len());
    for &(x_raw, y, se) in raw {
        if !x_raw.is_finite() || !y.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("curve points must have finite x and y") });
        }
        if let Some(s) = se
            && (!s.is_finite() || s < 0.0)
        {
            return Err(TransformError::InvalidInput { message: format!("curve standard error must be finite and >= 0, got {s}") });
        }
        pts.push(CurvePoint { x: normalize_zero(x_raw), y, se });
    }
    pts.sort_by(|a, b| a.x.total_cmp(&b.x));
    Ok(pts)
}

struct ActivePeak {
    start_x: f64,
    peak_x: f64,
    peak_y: f64,
    area_acc: sum::NeumaierSum,
}

fn integrate_segments_and_peaks(pts: &[CurvePoint], baseline: f64) -> (f64, f64, Vec<PeakSummary>) {
    let mut pos_acc = sum::NeumaierSum::new();
    let mut neg_acc = sum::NeumaierSum::new();
    let mut peaks = Vec::new();
    let mut active: Option<ActivePeak> = if pts[0].y > baseline {
        Some(ActivePeak { start_x: pts[0].x, peak_x: pts[0].x, peak_y: pts[0].y, area_acc: sum::NeumaierSum::new() })
    } else {
        None
    };

    for pair in pts.windows(2) {
        let p0 = pair[0];
        let p1 = pair[1];
        let dx = p1.x - p0.x;
        if dx <= 0.0 {
            continue;
        }
        let d0 = p0.y - baseline;
        let d1 = p1.y - baseline;

        if d0 >= 0.0 && d1 >= 0.0 {
            let area = 0.5 * (d0 + d1) * dx;
            pos_acc.add(area);
            if d0 > 0.0 || d1 > 0.0 {
                let pk = active.get_or_insert_with(|| ActivePeak { start_x: p0.x, peak_x: p0.x, peak_y: p0.y, area_acc: sum::NeumaierSum::new() });
                pk.area_acc.add(area);
                if p1.y > pk.peak_y {
                    pk.peak_x = p1.x;
                    pk.peak_y = p1.y;
                }
            }
            if d1 == 0.0
                && let Some(finished) = active.take()
            {
                peaks.push(finish_peak(finished, p1.x, baseline));
            }
        } else if d0 <= 0.0 && d1 <= 0.0 {
            neg_acc.add(-0.5 * (d0 + d1) * dx);
            if let Some(finished) = active.take() {
                peaks.push(finish_peak(finished, p0.x, baseline));
            }
        } else {
            let scale = d0.abs().max(d1.abs());
            let frac = if scale > 0.0 && scale.is_finite() {
                let s0 = d0 / scale;
                let s1 = d1 / scale;
                (s0.abs() / (s0 - s1).abs()).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let x_cross = (p0.x + frac * dx).clamp(p0.x, p1.x);
            if d0 > 0.0 {
                let area = 0.5 * d0 * (x_cross - p0.x);
                pos_acc.add(area);
                neg_acc.add(-0.5 * d1 * (p1.x - x_cross));
                if let Some(mut pk) = active.take() {
                    pk.area_acc.add(area);
                    peaks.push(finish_peak(pk, x_cross, baseline));
                }
            } else {
                neg_acc.add(-0.5 * d0 * (x_cross - p0.x));
                let area = 0.5 * d1 * (p1.x - x_cross);
                pos_acc.add(area);
                let mut pk = ActivePeak { start_x: x_cross, peak_x: p1.x, peak_y: p1.y, area_acc: sum::NeumaierSum::new() };
                pk.area_acc.add(area);
                active = Some(pk);
            }
        }
    }

    if let Some(finished) = active.take() {
        let end_x = pts.last().map_or(finished.start_x, |p| p.x);
        peaks.push(finish_peak(finished, end_x, baseline));
    }

    (pos_acc.total(), neg_acc.total(), peaks)
}

fn finish_peak(pk: ActivePeak, end_x: f64, baseline: f64) -> PeakSummary {
    PeakSummary {
        peak_x: pk.peak_x,
        peak_y: pk.peak_y,
        height_above_baseline: pk.peak_y - baseline,
        start_x: pk.start_x,
        end_x,
        area: pk.area_acc.total(),
        area_fraction: 0.0,
    }
}

struct EvaluatedSeriesAuc {
    result: WindowedAucResult,
    bridged_interior_rows: usize,
}

struct SeriesAucOutcome {
    ds_idx: usize,
    title: String,
    outcome: Result<EvaluatedSeriesAuc, String>,
}

fn resolve_auc_columns(total_cols: usize, selected: &[usize]) -> Result<Vec<usize>, TransformError> {
    if selected.is_empty() {
        return Ok((0..total_cols).collect());
    }
    let mut out = Vec::with_capacity(selected.len().min(total_cols));
    for &idx in selected {
        if idx >= total_cols {
            return Err(TransformError::InvalidSpec { message: format!("selected dataset index {idx} exceeds column count {total_cols}") });
        }
        if !out.contains(&idx) {
            out.push(idx);
        }
    }
    Ok(out)
}

fn count_non_numeric_x_rows(xy: &XyTable) -> usize {
    if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) {
        return 0;
    }
    let mut count = 0_usize;
    for row in 0..xy.row_count() {
        let Some(subcells) = xy.x_cells.get(row) else {
            continue;
        };
        let has_non_numeric_text = subcells.iter().any(|cell| !cell.excluded && cell.active_text().is_some());
        if has_non_numeric_text && xy.x_summary_at(row).ok().flatten().is_none() {
            count += 1;
        }
    }
    count
}

fn row_raw_finite_x(xy: &XyTable, row: usize) -> Option<f64> {
    if let Ok(Some(x)) = xy.x_summary_at(row)
        && x.is_finite()
    {
        return Some(normalize_zero(x));
    }
    if let Some(x) = xy.raw_x_value_at(row)
        && x.is_finite()
    {
        return Some(normalize_zero(x));
    }
    if let XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(_) } = xy.x_mode
        && let Some(subcells) = xy.x_cells.get(row)
    {
        let vals: Vec<f64> = subcells.iter().filter_map(|c| c.value.finite_f64()).collect();
        if !vals.is_empty()
            && let Ok(mean) = sum::mean(&vals)
            && mean.is_finite()
        {
            return Some(normalize_zero(mean));
        }
    }
    None
}

fn count_bridged_interior_rows(xy: &XyTable, active_rows: &[bool], orig_knot_xs: &[f64], first_contrib_idx: usize, last_contrib_idx: usize) -> usize {
    let (Some(&min_x), Some(&max_x)) = (orig_knot_xs.get(first_contrib_idx), orig_knot_xs.get(last_contrib_idx)) else {
        return 0;
    };
    let mut bridged = 0_usize;
    for row in 0..xy.row_count() {
        if active_rows.get(row).copied().unwrap_or(false) {
            continue;
        }
        let Some(x) = row_raw_finite_x(xy, row) else {
            continue;
        };
        if x > min_x && x < max_x && orig_knot_xs.binary_search_by(|k| k.total_cmp(&x)).is_err() {
            bridged += 1;
        }
    }
    bridged
}

fn auc_num_cell(value: f64) -> ResultCell {
    ResultCell::Number { value, prefix: None, units: None }
}

fn dominant_peak(peaks: &[PeakSummary]) -> Option<&PeakSummary> {
    peaks.iter().max_by(|a, b| {
        a.height_above_baseline
            .total_cmp(&b.height_above_baseline)
            .then_with(|| a.area.total_cmp(&b.area))
            .then_with(|| b.peak_x.total_cmp(&a.peak_x))
    })
}

fn build_auc_summary_tab(outcomes: &[SeriesAucOutcome], spec: &AreaUnderCurveSpec) -> ResultTab {
    let column_headers = vec![
        String::from("Start X"),
        String::from("End X"),
        String::from("Span"),
        String::from("N Points"),
        String::from("Baseline"),
        String::from("Positive Area"),
        String::from("Negative Area"),
        String::from("Net Area"),
        String::from("Total Area"),
        String::from("Net Area SE (Bailer, indep.)"),
        String::from("Peak Count"),
        String::from("Peak X"),
        String::from("Peak Y"),
        String::from("Status"),
    ];
    let mut row_labels = Vec::with_capacity(outcomes.len());
    let mut rows = Vec::with_capacity(outcomes.len());

    for entry in outcomes {
        row_labels.push(entry.title.clone());
        match &entry.outcome {
            Ok(eval) => {
                let r = &eval.result;
                let primary = dominant_peak(&r.auc.peaks);
                #[allow(clippy::cast_precision_loss)]
                let n_pts = r.n_points as f64;
                #[allow(clippy::cast_precision_loss)]
                let pk_cnt = r.auc.peaks.len() as f64;
                rows.push(vec![
                    auc_num_cell(r.start_x),
                    auc_num_cell(r.end_x),
                    auc_num_cell(r.span),
                    auc_num_cell(n_pts),
                    auc_num_cell(r.resolved_baseline),
                    auc_num_cell(r.auc.positive_area),
                    auc_num_cell(r.auc.negative_area),
                    auc_num_cell(r.auc.net_area),
                    auc_num_cell(r.auc.total_area),
                    r.auc.standard_error.map_or(ResultCell::Empty, auc_num_cell),
                    auc_num_cell(pk_cnt),
                    primary.map_or(ResultCell::Empty, |p| auc_num_cell(p.peak_x)),
                    primary.map_or(ResultCell::Empty, |p| auc_num_cell(p.peak_y)),
                    ResultCell::Text(String::from("OK")),
                ]);
            }
            Err(reason) => {
                rows.push(vec![
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Text(format!("Unavailable: {reason}")),
                ]);
            }
        }
    }

    let baseline_note = match spec.baseline {
        AucBaseline::Fixed { value } => format!("Fixed baseline Y = {}.", format_compact_f64(value)),
        AucBaseline::FirstPoint => String::from(
            "First-point baseline Y(Start X) resolved on each series's clipped curve (linearly interpolated when Start X lies between knots).",
        ),
    };
    let se_note = if spec.independent_time_points {
        "Net Area SE uses Bailer (1988) analytical propagation assuming independent time-point estimates; blank if any contributing knot lacks a finite point SE (for example n = 1)."
    } else {
        "Net Area SE is blank because independent_time_points is false (Bailer 1988 requires independent time-point cohorts)."
    };

    ResultTab {
        id: String::from("auc-summary"),
        title: String::from("Area under the curve"),
        kind: ResultTabKind::MainSummary,
        column_headers,
        row_labels,
        rows,
        footnotes: vec![format!("Linear trapezoidal integration (Yeh & Kwan 1978). {baseline_note}"), String::from(se_note)],
    }
}

fn build_auc_peaks_tab(outcomes: &[SeriesAucOutcome], spec: &AreaUnderCurveSpec) -> Option<ResultTab> {
    let mut row_labels = Vec::new();
    let mut rows = Vec::new();

    for entry in outcomes {
        let Ok(eval) = &entry.outcome else {
            continue;
        };
        for (idx, pk) in eval.result.auc.peaks.iter().enumerate() {
            let peak_num = idx + 1;
            row_labels.push(format!("{} #{peak_num}", entry.title));
            #[allow(clippy::cast_precision_loss)]
            let peak_num_f = peak_num as f64;
            rows.push(vec![
                ResultCell::Text(entry.title.clone()),
                auc_num_cell(peak_num_f),
                auc_num_cell(pk.peak_x),
                auc_num_cell(pk.peak_y),
                auc_num_cell(pk.height_above_baseline),
                auc_num_cell(pk.start_x),
                auc_num_cell(pk.end_x),
                auc_num_cell(pk.area),
                auc_num_cell(pk.area_fraction),
            ]);
        }
    }

    if rows.is_empty() {
        return None;
    }

    Some(ResultTab {
        id: String::from("auc-peaks"),
        title: String::from("Peaks"),
        kind: ResultTabKind::DescriptiveSummary,
        column_headers: vec![
            String::from("Series"),
            String::from("Peak #"),
            String::from("Peak X"),
            String::from("Peak Y"),
            String::from("Height Above Baseline"),
            String::from("Start X"),
            String::from("End X"),
            String::from("Area"),
            String::from("Area Fraction"),
        ],
        row_labels,
        rows,
        footnotes: vec![format!(
            "Positive excursions above the resolved baseline clipped to each series's [Start X, End X] window (min_peak_height = {}, min_area_fraction = {}).",
            format_compact_f64(spec.min_peak_height),
            format_compact_f64(spec.min_area_fraction)
        )],
    })
}

fn push_auc_scalars(scalars: &mut Vec<ResultScalarEntry>, outcomes: &[SeriesAucOutcome]) {
    for entry in outcomes {
        let Ok(eval) = &entry.outcome else {
            continue;
        };
        let auc = &eval.result.auc;
        for (key, value) in
            [("PositiveArea", auc.positive_area), ("NegativeArea", auc.negative_area), ("NetArea", auc.net_area), ("TotalArea", auc.total_area)]
        {
            if value.is_finite() {
                scalars.push(ResultScalarEntry { key: String::from(key), data_set_index: Some(entry.ds_idx), value, units: None });
            }
        }
        if let Some(se) = auc.standard_error
            && se.is_finite()
        {
            scalars.push(ResultScalarEntry { key: String::from("NetAreaSe"), data_set_index: Some(entry.ds_idx), value: se, units: None });
        }
    }
}

fn build_auc_analyzable_sheet(analysis: &AnalysisSpec, outcomes: &[SeriesAucOutcome]) -> Result<DataSheet, TransformError> {
    let row_count = outcomes.len();
    let specs: [(&str, VariableRole); 11] = [
        ("Series", VariableRole::Label),
        ("Start X", VariableRole::Continuous),
        ("End X", VariableRole::Continuous),
        ("Span", VariableRole::Continuous),
        ("N Points", VariableRole::Continuous),
        ("Baseline", VariableRole::Continuous),
        ("Positive Area", VariableRole::Continuous),
        ("Negative Area", VariableRole::Continuous),
        ("Net Area", VariableRole::Continuous),
        ("Total Area", VariableRole::Continuous),
        ("Net Area SE", VariableRole::Continuous),
    ];

    let mut variables = Vec::with_capacity(specs.len());
    for (title, role) in specs {
        let mut col = VariableColumn::new(title, row_count)?;
        col.lock_role(role);
        variables.push(col);
    }

    let mut row_titles = Vec::with_capacity(row_count);
    for (row, entry) in outcomes.iter().enumerate() {
        row_titles.push(entry.title.clone());
        variables[0].cells[row] = Cell::text(entry.title.clone());
        if let Ok(eval) = &entry.outcome {
            let r = &eval.result;
            #[allow(clippy::cast_precision_loss)]
            let n_pts = r.n_points as f64;
            variables[1].cells[row] = Cell::number(r.start_x);
            variables[2].cells[row] = Cell::number(r.end_x);
            variables[3].cells[row] = Cell::number(r.span);
            variables[4].cells[row] = Cell::number(n_pts);
            variables[5].cells[row] = Cell::number(r.resolved_baseline);
            variables[6].cells[row] = Cell::number(r.auc.positive_area);
            variables[7].cells[row] = Cell::number(r.auc.negative_area);
            variables[8].cells[row] = Cell::number(r.auc.net_area);
            variables[9].cells[row] = Cell::number(r.auc.total_area);
            variables[10].cells[row] = r.auc.standard_error.map_or_else(Cell::empty, Cell::number);
        }
    }

    let mv = MultipleVariablesTable { row_titles, row_titles_expanded: false, variables };
    let data_id = format!("{}-data", analysis.id.as_str());
    let data_title = format!("{} (data)", analysis.effective_title());
    let sheet = match DataSheet::new(data_id, data_title.clone(), TableArchetype::MultipleVariables(mv.clone())) {
        Ok(s) => s,
        Err(_) => DataSheet::new(analysis.id.as_str(), analysis.effective_title(), TableArchetype::MultipleVariables(mv))?,
    };
    Ok(sheet)
}

fn build_auc_checklist(outcomes: &[SeriesAucOutcome], spec: &AreaUnderCurveSpec, non_numeric_x_rows: usize) -> Vec<ChecklistItem> {
    let mut items = Vec::new();

    if !spec.independent_time_points {
        items.push(ChecklistItem {
            assumption: String::from("Time-point independence (Bailer SE)"),
            status: ChecklistStatus::Info,
            diagnostic_metric: Some(String::from("independent_time_points = false")),
            explanation: String::from(
                "Net Area SE is left blank because Bailer's (1988) analytical SE assumes independent time-point estimates (separate cohorts or subjects per time point). On longitudinal or repeated-measures curves, omitting within-well/within-subject covariance can bias SE in either direction (especially under FirstPoint baseline, where adjusted knot weights have opposite signs).",
            ),
            remediation: None,
        });
    } else {
        push_missing_se_checklist_item(&mut items, outcomes);
    }

    if spec.x_min.is_none() || spec.x_max.is_none() {
        push_window_comparability_checklist_item(&mut items, outcomes);
    }

    push_unavailable_series_checklist_item(&mut items, outcomes);
    push_bridged_rows_checklist_item(&mut items, outcomes);

    if non_numeric_x_rows > 0 {
        items.push(ChecklistItem {
            assumption: String::from("Numeric X coordinates"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: Some(format!("{non_numeric_x_rows} non-numeric X row(s) ignored")),
            explanation: format!(
                "Ignored {non_numeric_x_rows} row(s) with non-numeric text in the X column. Area Under the Curve requires numeric X coordinates in the desired integration units (for example seconds or minutes rather than text timestamps)."
            ),
            remediation: None,
        });
    }

    items
}

fn push_missing_se_checklist_item(items: &mut Vec<ChecklistItem>, outcomes: &[SeriesAucOutcome]) {
    let affected: Vec<(&str, &[f64])> = outcomes
        .iter()
        .filter_map(|entry| {
            let eval = entry.outcome.as_ref().ok()?;
            (!eval.result.missing_se_x.is_empty()).then_some((entry.title.as_str(), eval.result.missing_se_x.as_slice()))
        })
        .collect();
    if affected.is_empty() {
        return;
    }

    const MAX_LISTED_SERIES: usize = 12;
    const MAX_LISTED_KNOTS: usize = 8;
    let mut parts = Vec::with_capacity(affected.len().min(MAX_LISTED_SERIES));
    for &(title, xs) in affected.iter().take(MAX_LISTED_SERIES) {
        let mut x_strs: Vec<String> = xs.iter().take(MAX_LISTED_KNOTS).map(|&x| format_compact_f64(x)).collect();
        if xs.len() > MAX_LISTED_KNOTS {
            x_strs.push(format!("... ({} knots total)", xs.len()));
        }
        parts.push(format!("{title} (X = {})", x_strs.join(", ")));
    }
    let overflow =
        if affected.len() > MAX_LISTED_SERIES { format!("; ... and {} more series", affected.len() - MAX_LISTED_SERIES) } else { String::new() };
    let summary = format!("{}{overflow}", parts.join("; "));
    items.push(ChecklistItem {
        assumption: String::from("Point SE availability (n = 1 knots)"),
        status: ChecklistStatus::Info,
        diagnostic_metric: Some(summary.clone()),
        explanation: format!(
            "Net Area SE was left blank for {} series because at least one contributing knot lacked a finite point SE (for example n = 1 replicate or non-parametric summary): {summary}.",
            affected.len()
        ),
        remediation: None,
    });
}

fn push_window_comparability_checklist_item(items: &mut Vec<ChecklistItem>, outcomes: &[SeriesAucOutcome]) {
    let available: Vec<(&str, f64, f64, f64)> = outcomes
        .iter()
        .filter_map(|entry| {
            let r = &entry.outcome.as_ref().ok()?.result;
            Some((entry.title.as_str(), r.start_x, r.end_x, r.span))
        })
        .collect();
    let Some(&(_, ref_start, ref_end, _)) = available.first() else {
        return;
    };
    let differs = available.iter().skip(1).any(|&(_, start_x, end_x, _)| {
        let tol_start = boundary_tol(ref_start.abs().max(start_x.abs()));
        let tol_end = boundary_tol(ref_end.abs().max(end_x.abs()));
        (start_x - ref_start).abs() > tol_start || (end_x - ref_end).abs() > tol_end
    });
    if !differs {
        return;
    }

    const MAX_LISTED: usize = 12;
    let mut parts = Vec::with_capacity(available.len().min(MAX_LISTED));
    for &(title, start_x, end_x, span) in available.iter().take(MAX_LISTED) {
        parts.push(format!("{title}: [{}, {}] (span {})", format_compact_f64(start_x), format_compact_f64(end_x), format_compact_f64(span)));
    }
    let overflow = if available.len() > MAX_LISTED { format!("; ... and {} more series", available.len() - MAX_LISTED) } else { String::new() };
    let detail = format!("{}{overflow}", parts.join("; "));
    items.push(ChecklistItem {
        assumption: String::from("Comparable integration windows"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: Some(detail.clone()),
        explanation: format!(
            "Evaluated series integrate over different effective [Start X, End X] windows because at least one integration bound was omitted: {detail}. Raw AUC values over unequal spans are not directly comparable; specify both x_min and x_max to enforce a common integration window."
        ),
        remediation: None,
    });
}

fn push_unavailable_series_checklist_item(items: &mut Vec<ChecklistItem>, outcomes: &[SeriesAucOutcome]) {
    let unavailable: Vec<(&str, &str)> =
        outcomes.iter().filter_map(|entry| Some((entry.title.as_str(), entry.outcome.as_ref().err()?.as_str()))).collect();
    if unavailable.is_empty() {
        return;
    }

    const MAX_LISTED: usize = 12;
    let mut parts = Vec::with_capacity(unavailable.len().min(MAX_LISTED));
    for &(title, reason) in unavailable.iter().take(MAX_LISTED) {
        parts.push(format!("{title} ({reason})"));
    }
    let overflow = if unavailable.len() > MAX_LISTED { format!("; ... and {} more series", unavailable.len() - MAX_LISTED) } else { String::new() };
    items.push(ChecklistItem {
        assumption: String::from("Series coverage and point count"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: Some(format!("{} of {} selected series unavailable", unavailable.len(), outcomes.len())),
        explanation: format!(
            "Marked {} series unavailable (retained with empty numeric cells in auc-summary and analyzable_data): {}{overflow}.",
            unavailable.len(),
            parts.join("; ")
        ),
        remediation: None,
    });
}

fn push_bridged_rows_checklist_item(items: &mut Vec<ChecklistItem>, outcomes: &[SeriesAucOutcome]) {
    let bridged: Vec<(&str, usize)> = outcomes
        .iter()
        .filter_map(|entry| {
            let count = entry.outcome.as_ref().ok()?.bridged_interior_rows;
            (count > 0).then_some((entry.title.as_str(), count))
        })
        .collect();
    if bridged.is_empty() {
        return;
    }

    const MAX_LISTED: usize = 12;
    let mut parts = Vec::with_capacity(bridged.len().min(MAX_LISTED));
    for &(title, count) in bridged.iter().take(MAX_LISTED) {
        let noun = if count == 1 { "row" } else { "rows" };
        parts.push(format!("{title}: {count} bridged {noun}"));
    }
    let overflow = if bridged.len() > MAX_LISTED { format!("; ... and {} more series", bridged.len() - MAX_LISTED) } else { String::new() };
    let detail = format!("{}{overflow}", parts.join("; "));
    items.push(ChecklistItem {
        assumption: String::from("Interior missing or excluded points"),
        status: ChecklistStatus::Info,
        diagnostic_metric: Some(detail.clone()),
        explanation: format!(
            "Linearly bridged interior missing or excluded rows between consecutive valid active observations under the trapezoidal rule (Yeh & Kwan 1978): {detail}."
        ),
        remediation: None,
    });
}

fn format_compact_f64(value: f64) -> String {
    let v = normalize_zero(value);
    if !v.is_finite() {
        return format!("{v}");
    }
    if v == 0.0 {
        return String::from("0");
    }
    let abs = v.abs();
    if abs < 1e-4 || abs >= 1e9 {
        return format!("{v:.4e}");
    }
    let s = format!("{v:.6}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-0" { String::from("0") } else { trimmed.to_owned() }
}

fn validate_smooth_method(method: SmoothMethod) -> Result<(), TransformError> {
    match method {
        SmoothMethod::SavitzkyGolay5 | SmoothMethod::SavitzkyGolay7 | SmoothMethod::SavitzkyGolay9 => Ok(()),
        SmoothMethod::MovingAverage { window } => {
            if window == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("moving average window must be >= 1") });
            }
            Ok(())
        }
        SmoothMethod::Lowess { span, robustness_iters } => {
            if !span.is_finite() || span <= 0.0 || span > 1.0 {
                return Err(TransformError::InvalidSpec { message: format!("LOWESS span must be finite and in (0, 1], got {span}") });
            }
            if robustness_iters > SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS {
                return Err(TransformError::InvalidSpec {
                    message: format!("LOWESS robustness_iters must be <= {}, got {robustness_iters}", SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS),
                });
            }
            Ok(())
        }
    }
}

/// Smooth an `(x, y)` series using Savitzky-Golay, moving average, or LOWESS.
pub fn smooth_xy(points: &[(f64, f64)], method: SmoothMethod) -> Result<Vec<(f64, f64)>, TransformError> {
    validate_smooth_method(method)?;
    if points.is_empty() {
        return Ok(Vec::new());
    }
    let sorted = prepare_sorted_points(&points.iter().map(|&(x, y)| (x, y, None)).collect::<Vec<_>>())?;
    let xs: Vec<f64> = sorted.iter().map(|p| p.x).collect();
    let ys: Vec<f64> = sorted.iter().map(|p| p.y).collect();

    let smoothed_y = match method {
        SmoothMethod::SavitzkyGolay5 => savitzky_golay(&xs, &ys, 5)?,
        SmoothMethod::SavitzkyGolay7 => savitzky_golay(&xs, &ys, 7)?,
        SmoothMethod::SavitzkyGolay9 => savitzky_golay(&xs, &ys, 9)?,
        SmoothMethod::MovingAverage { window } => moving_average(&ys, window)?,
        SmoothMethod::Lowess { span, robustness_iters } => lowess(&xs, &ys, span, robustness_iters)?,
    };

    if smoothed_y.iter().any(|v| !v.is_finite()) {
        return Err(TransformError::InvalidInput { message: String::from("curve smoothing produced non-finite value") });
    }
    Ok(xs.into_iter().zip(smoothed_y).collect())
}

fn is_uniform_grid(xs: &[f64]) -> bool {
    if xs.len() < 3 {
        return true;
    }
    let h0 = xs[1] - xs[0];
    if !h0.is_finite() || h0 <= 0.0 {
        return false;
    }
    xs.windows(2).all(|w| {
        let h = w[1] - w[0];
        h.is_finite() && (h - h0).abs() <= 1e-7 * h0.abs()
    })
}

/// Polynomial order (`d = 3`, quadratic/cubic family) for Savitzky–Golay smoothing.
///
/// At the midpoint (`t = 0`) of a symmetric uniform stencil of width `2m + 1` (`5`, `7`, `9`),
/// odd monomials (`t`, `t^3`) are orthogonal to even monomials (`1`, `t^2`) because
/// `sum_{k=-m}^{m} k^{2p+1} = 0`, so quadratic (`d = 2`) and cubic (`d = 3`) least-squares fits
/// share identical central convolution weights (Savitzky & Golay, *Anal. Chem.*, 1964, 36(8),
/// 1627–1639, Table I). Evaluating local degree-3 (`SAVITZKY_GOLAY_DEGREE`) least-squares
/// polynomials at boundary offsets, short series (`n < width`), and non-uniform abscissas
/// matches the interior filter's exact cubic reproduction order across the entire domain
/// (Gorry, *Anal. Chem.*, 1990, 62(6), 570–573).
const SAVITZKY_GOLAY_DEGREE: usize = 3;

fn savitzky_golay(xs: &[f64], ys: &[f64], width: usize) -> Result<Vec<f64>, TransformError> {
    let n = ys.len();
    if n < width {
        let deg = n.saturating_sub(1).min(SAVITZKY_GOLAY_DEGREE);
        return Ok((0..n).map(|i| local_poly_eval(xs, ys, 0..n, xs[i], deg).unwrap_or(ys[i])).collect());
    }

    let half = width / 2;
    let uniform = is_uniform_grid(xs);
    let weights: &[f64] = match width {
        5 => &[-3.0 / 35.0, 12.0 / 35.0, 17.0 / 35.0, 12.0 / 35.0, -3.0 / 35.0],
        7 => &[-2.0 / 21.0, 3.0 / 21.0, 6.0 / 21.0, 7.0 / 21.0, 6.0 / 21.0, 3.0 / 21.0, -2.0 / 21.0],
        9 => &[-21.0 / 231.0, 14.0 / 231.0, 39.0 / 231.0, 54.0 / 231.0, 59.0 / 231.0, 54.0 / 231.0, 39.0 / 231.0, 14.0 / 231.0, -21.0 / 231.0],
        _ => return Err(TransformError::InvalidSpec { message: format!("unsupported Savitzky-Golay width {width}") }),
    };

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if uniform && i >= half && i + half < n {
            let window = &ys[i - half..=i + half];
            let mut acc = sum::NeumaierSum::new();
            for (&w, &yv) in weights.iter().zip(window.iter()) {
                acc.add(w * yv);
            }
            let val = acc.total();
            if val.is_finite() {
                out.push(val);
            } else {
                let scale = window.iter().fold(0.0_f64, |m, &v| m.max(v.abs()));
                if scale > 0.0 && scale.is_finite() {
                    let mut scaled = sum::NeumaierSum::new();
                    for (&w, &yv) in weights.iter().zip(window.iter()) {
                        scaled.add(w * (yv / scale));
                    }
                    out.push(scaled.total() * scale);
                } else {
                    out.push(ys[i]);
                }
            }
        } else {
            let start = if i < half {
                0
            } else if i + half >= n {
                n - width
            } else {
                i - half
            };
            let end = start + width;
            let val = local_poly_eval(xs, ys, start..end, xs[i], SAVITZKY_GOLAY_DEGREE).unwrap_or(ys[i]);
            out.push(val);
        }
    }
    Ok(out)
}

fn local_poly_eval(xs: &[f64], ys: &[f64], range: std::ops::Range<usize>, x_target: f64, degree: usize) -> Option<f64> {
    let m = range.len();
    if m == 0 {
        return None;
    }
    let max_dt = range.clone().map(|idx| (xs[idx] - x_target).abs()).fold(0.0_f64, f64::max);
    if !max_dt.is_finite() {
        return None;
    }
    if max_dt == 0.0 {
        return sum::mean(ys.get(range)?).ok().filter(|v| v.is_finite());
    }
    let p = degree.min(m - 1) + 1;
    let mut data = Vec::with_capacity(m * p);
    let mut b = Vec::with_capacity(m);
    for idx in range {
        // Normalize offsets to [-1, 1] so the intercept c_0 = P(x_target) is invariant while the
        // local Vandermonde columns stay O(1)-conditioned across tiny or huge X scales.
        let dt = (xs[idx] - x_target) / max_dt;
        if !dt.is_finite() {
            return None;
        }
        let mut pow = 1.0_f64;
        for _ in 0..p {
            data.push(pow);
            pow *= dt;
        }
        b.push(ys[idx]);
    }
    let a = DenseMatrix::from_row_major(m, p, data).ok()?;
    let (sol, _) = solve_linear_least_squares(&a, &b).ok()?;
    if sol.rank == 0 {
        return None;
    }
    sol.coefficients.first().copied().filter(|v| v.is_finite())
}

fn moving_average(ys: &[f64], window: usize) -> Result<Vec<f64>, TransformError> {
    if window == 0 {
        return Err(TransformError::InvalidSpec { message: String::from("moving average window must be >= 1") });
    }
    let n = ys.len();
    let half_left = (window - 1) / 2;
    let half_right = window / 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = i.saturating_sub(half_left);
        let end = (i + half_right + 1).min(n);
        out.push(sum::mean(&ys[start..end])?);
    }
    Ok(out)
}

/// LOWESS of Cleveland (1979): tricube-weighted local linear fits over the nearest
/// `round(span * n)` points, with bisquare robustness weights from residuals scaled by `6 * MAD`.
fn lowess(xs: &[f64], ys: &[f64], span: f64, robustness_iters: usize) -> Result<Vec<f64>, TransformError> {
    if !span.is_finite() || span <= 0.0 || span > 1.0 {
        return Err(TransformError::InvalidSpec { message: format!("LOWESS span must be in (0, 1], got {span}") });
    }
    if robustness_iters > SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS {
        return Err(TransformError::InvalidSpec {
            message: format!("LOWESS robustness_iters must be <= {}, got {robustness_iters}", SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS),
        });
    }
    let n = xs.len();
    if n <= 2 {
        return Ok(ys.to_vec());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let q = ((span * (n as f64)).round() as usize).clamp(2, n);
    let (fitted, _) = fit_lowess_sorted_values(xs, ys, q, robustness_iters);
    Ok(fitted)
}

/// Compute numerical first (`dy/dx`) or second (`d2y/dx2`) derivative of an `(x, y)` curve.
///
/// Uses exact shifted-step 3-point Newton/Lagrange quadratic divided differences on both uniform
/// and non-uniform grids, eliminating coordinate-offset cancellation and `h^2` underflow/overflow.
pub fn differentiate_xy(points: &[(f64, f64)], order: DerivativeOrder) -> Result<Vec<(f64, f64)>, TransformError> {
    let pts = prepare_sorted_points(&points.iter().map(|&(x, y)| (x, y, None)).collect::<Vec<_>>())?;
    let n = pts.len();
    if n < 2 {
        return Err(TransformError::InvalidInput { message: String::from("differentiation requires at least 2 points") });
    }
    if n == 2 {
        let dx = pts[1].x - pts[0].x;
        if dx == 0.0 {
            return Err(TransformError::InvalidInput { message: String::from("duplicate x coordinates in differentiation") });
        }
        let slope = match order {
            DerivativeOrder::First => (pts[1].y - pts[0].y) / dx,
            DerivativeOrder::Second => 0.0,
        };
        if !slope.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("derivative overflowed to non-finite value") });
        }
        return Ok(vec![(pts[0].x, slope), (pts[1].x, slope)]);
    }

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (i0, i1, i2) = if i == 0 {
            (0, 1, 2)
        } else if i + 1 == n {
            (n - 3, n - 2, n - 1)
        } else {
            (i - 1, i, i + 1)
        };
        let (x0, y0) = (pts[i0].x, pts[i0].y);
        let (x1, y1) = (pts[i1].x, pts[i1].y);
        let (x2, y2) = (pts[i2].x, pts[i2].y);
        let h1 = x1 - x0;
        let h2 = x2 - x1;
        if h1 <= 0.0 || h2 <= 0.0 {
            return Err(TransformError::InvalidInput { message: String::from("duplicate x coordinates in differentiation") });
        }
        let s1 = (y1 - y0) / h1;
        let s2 = (y2 - y1) / h2;
        let h_max = h1.max(h2);
        let u1 = h1 / h_max;
        let u2 = h2 / h_max;
        let u_sum = u1 + u2;
        let r1 = u1 / u_sum;
        let r2 = u2 / u_sum;
        let val = match order {
            DerivativeOrder::First => {
                if i == 0 {
                    s1 - r1 * (s2 - s1)
                } else if i + 1 == n {
                    s2 + r2 * (s2 - s1)
                } else {
                    r2 * s1 + r1 * s2
                }
            }
            DerivativeOrder::Second => {
                let h_sum = h1 + h2;
                if h_sum.is_finite() { 2.0 * ((s2 - s1) / h_sum) } else { (s2 - s1) / (0.5 * h1 + 0.5 * h2) }
            }
        };
        if !val.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("derivative overflowed to non-finite value") });
        }
        out.push((pts[i].x, val));
    }
    Ok(out)
}

/// Compute the cumulative trapezoidal integral curve relative to `baseline`, starting at `(x_0, 0.0)`.
pub fn integrate_xy(points: &[(f64, f64)], baseline: f64) -> Result<Vec<(f64, f64)>, TransformError> {
    if !baseline.is_finite() {
        return Err(TransformError::InvalidSpec { message: String::from("integration baseline must be finite") });
    }
    if points.is_empty() {
        return Ok(Vec::new());
    }
    let pts = prepare_sorted_points(&points.iter().map(|&(x, y)| (x, y, None)).collect::<Vec<_>>())?;
    let mut out = Vec::with_capacity(pts.len());
    let mut acc = sum::NeumaierSum::new();
    out.push((pts[0].x, 0.0));
    for pair in pts.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let avg_y = 0.5 * ((pair[0].y - baseline) + (pair[1].y - baseline));
        acc.add(dx * avg_y);
        out.push((pair[1].x, acc.total()));
    }
    Ok(out)
}

/// Curve-calculus transform wrapper producing a derived `XyTable` [`DataSheet`].
pub fn apply_curve_calculus_to_sheet(sheet: &DataSheet, op: CurveSheetOp) -> Result<DataSheet, TransformError> {
    let TableArchetype::Xy(xy) = &sheet.archetype else {
        return Err(TransformError::IncompatibleTable { operation: "curve calculus", kind: sheet.kind() });
    };
    if xy.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("XY table has no datasets") });
    }

    let mut series_curves = Vec::with_capacity(xy.data_sets.len());
    for ds_idx in 0..xy.data_sets.len() {
        series_curves.push(transform_dataset_curve(xy, ds_idx, op)?);
    }

    let (x_grid, row_map) = build_aligned_x_grid(&series_curves);
    let mut out_xy = XyTable::new_with_x_mode(
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
        SubcolumnMode::Single,
        xy.data_sets.len(),
        x_grid.len(),
    )?;
    out_xy.x_title = xy.x_title.clone();
    for (out_row, &(_, x)) in x_grid.iter().enumerate() {
        out_xy.set_x_cell(out_row, Cell::number(x))?;
    }
    for (ds_idx, curve) in series_curves.iter().enumerate() {
        if let (Some(src_ds), Some(dst_ds)) = (xy.data_sets.get(ds_idx), out_xy.data_sets.get_mut(ds_idx)) {
            dst_ds.title = src_ds.title.clone();
            for &(src_row, _, y) in curve {
                if let Some(&out_row) = row_map.get(&src_row)
                    && let Some(cell) = dst_ds.cells.get_mut(out_row).and_then(|sub| sub.first_mut())
                {
                    *cell = Cell::number(y);
                }
            }
        }
    }

    Ok(DataSheet::new(format!("{}-curve", sheet.id.as_str()), format!("{} (Curve)", sheet.title), TableArchetype::Xy(out_xy))?)
}

/// Curve-calculus operation applicable to an entire `XyTable`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveSheetOp {
    /// Smooth each series using [`SmoothMethod`].
    Smooth(SmoothMethod),
    /// Compute numerical [`DerivativeOrder::First`] (`dY/dX`) or [`DerivativeOrder::Second`] (`d^2Y/dX^2`).
    Differentiate(DerivativeOrder),
    /// Compute running cumulative trapezoidal integral `integral_{x_0}^x (Y(u) - baseline) du`.
    Integrate {
        /// Horizontal baseline subtracted from *Y* prior to cumulative integration.
        baseline: f64,
    },
}

fn transform_dataset_curve(xy: &XyTable, ds_idx: usize, op: CurveSheetOp) -> Result<Vec<(usize, f64, f64)>, TransformError> {
    let entries = extract_xy_mean_entries(xy, ds_idx)?;
    let pairs: Vec<(f64, f64)> = entries.iter().map(|&(_, x, y)| (x, y)).collect();
    let transformed = match op {
        CurveSheetOp::Smooth(m) => smooth_xy(&pairs, m)?,
        CurveSheetOp::Differentiate(order) => differentiate_xy(&pairs, order)?,
        CurveSheetOp::Integrate { baseline } => integrate_xy(&pairs, baseline)?,
    };
    if transformed.len() != entries.len() {
        return Err(TransformError::InvalidInput {
            message: format!("curve calculus point count mismatch: expected {}, got {}", entries.len(), transformed.len()),
        });
    }
    Ok(entries.into_iter().zip(transformed).map(|((row, _, _), (tx, ty))| (row, tx, ty)).collect())
}

fn build_aligned_x_grid(curves: &[Vec<(usize, f64, f64)>]) -> (Vec<(usize, f64)>, std::collections::HashMap<usize, usize>) {
    let mut all_rows: Vec<(usize, f64)> = curves.iter().flat_map(|c| c.iter().map(|&(row, x, _)| (row, x))).collect();
    all_rows.sort_by_key(|&(row, _)| row);
    all_rows.dedup_by_key(|&mut (row, _)| row);
    all_rows.sort_by(|&(r_a, x_a), &(r_b, x_b)| x_a.total_cmp(&x_b).then_with(|| r_a.cmp(&r_b)));
    let mut row_map = std::collections::HashMap::with_capacity(all_rows.len());
    for (out_row, &(src_row, _)) in all_rows.iter().enumerate() {
        row_map.insert(src_row, out_row);
    }
    (all_rows, row_map)
}

fn extract_xy_mean_entries(xy: &XyTable, ds_idx: usize) -> Result<Vec<(usize, f64, f64)>, TransformError> {
    let Some(ds) = xy.data_sets.get(ds_idx) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (row, subcells) in ds.cells.iter().enumerate() {
        let Some(x) = xy.x_summary_at(row)? else {
            continue;
        };
        if xy.y_subcolumn_mode.is_pre_summarized() {
            if let Some(m) = subcells.first().and_then(Cell::active_number) {
                entries.push((row, x, m));
            }
        } else {
            let active: Vec<f64> = subcells.iter().filter_map(Cell::active_number).collect();
            if !active.is_empty()
                && let Ok(m) = sum::mean(&active)
            {
                entries.push((row, x, m));
            }
        }
    }
    // Match `prepare_sorted_points` ordering so `transformed` aligns 1:1 with `entries`.
    entries.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    Ok(entries)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use curvebook_doc::SplineLowessSpec;

    use super::{AucOptions, DerivativeOrder, SmoothMethod, compute_auc, differentiate_xy, smooth_xy};
    use crate::error::TransformError;

    #[test]
    fn aud_04_04_and_19_curve_lowess_no_collapse_and_caps_robustness_iters() {
        // 1. Cap robustness_iters cleanly even on tiny or empty input.
        assert!(matches!(
            smooth_xy(&[], SmoothMethod::Lowess { span: 0.5, robustness_iters: SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS + 1 }),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(
            smooth_xy(&[(0.0, 1.0), (1.0, 2.0)], SmoothMethod::Lowess { span: 0.5, robustness_iters: usize::MAX }),
            Err(TransformError::InvalidSpec { .. })
        ));
        assert!(matches!(smooth_xy(&[], SmoothMethod::MovingAverage { window: 0 }), Err(TransformError::InvalidSpec { .. })));

        // 2. Bisquare peak neighborhood never collapses to 0.0.
        let mut peak_pts: Vec<(f64, f64)> = (0..21).map(|i| (i as f64, 50.0)).collect();
        peak_pts[10].1 = 200.0;
        peak_pts[11].1 = 200.0;
        let smoothed = smooth_xy(&peak_pts, SmoothMethod::Lowess { span: 0.15, robustness_iters: 3 }).expect("LOWESS peak");
        for &(x, y) in &smoothed {
            assert!(y.is_finite() && y >= 49.0, "curve::lowess collapsed at x={x}: got {y}");
        }
    }

    #[test]
    fn aud_04_41_shifted_lagrange_derivative_scaled_auc_crossing_and_zero_area_filter() {
        // 1. Offset cancellation on non-power-of-two h = 0.001 at x ~ 1e9 (y = (x - x0)^2),
        //    and large Y baseline (y = 1e12 + (x - x0)^2 at h = 0.3).
        let x0 = 1e9_f64;
        let x1 = x0 + 0.001;
        let x2 = x0 + 0.002;
        let x_offset_pts = [(x0, 0.0), (x1, (x1 - x0).powi(2)), (x2, (x2 - x0).powi(2))];
        let d1 = differentiate_xy(&x_offset_pts, DerivativeOrder::First).expect("shifted d1");
        let d2 = differentiate_xy(&x_offset_pts, DerivativeOrder::Second).expect("shifted d2");
        assert!((d1[1].1 - 2.0 * (x1 - x0)).abs() < 1e-12, "d1 x-offset cancellation: got {}", d1[1].1);
        assert!((d2[1].1 - 2.0).abs() < 1e-10, "d2 x-offset cancellation: got {}", d2[1].1);

        let xb1 = x0 + 0.3;
        let xb2 = x0 + 0.6;
        let y_base_pts = [(x0, 1e12), (xb1, 1e12 + (xb1 - x0).powi(2)), (xb2, 1e12 + (xb2 - x0).powi(2))];
        let d2_base = differentiate_xy(&y_base_pts, DerivativeOrder::Second).expect("shifted d2 baseline");
        assert!((d2_base[1].1 - 2.0).abs() < 1e-3, "d2 y-baseline cancellation: got {}", d2_base[1].1);

        // 2. Tiny dx = 1e-162 must not falsely error as duplicate X (old d0 underflowed to 0.0).
        let dx = 1e-162_f64;
        let tiny = [(0.0, 0.0), (dx, dx), (2.0 * dx, 2.0 * dx)];
        let d1_tiny = differentiate_xy(&tiny, DerivativeOrder::First).expect("tiny dx d1");
        for &(_, v) in &d1_tiny {
            assert!((v - 1.0).abs() < 1e-12, "tiny dx first derivative got {v}");
        }

        // 3. Extreme Y sign crossing: d0 = 1e308, d1 = -1e308 must cross at midpoint x = 1.0 (not 0.0).
        let extreme_pts = [(0.0, 1e308), (2.0, -1e308)];
        let auc_ext = compute_auc(&extreme_pts, &AucOptions::default()).expect("extreme AUC");
        assert_eq!(auc_ext.peaks.len(), 1);
        assert!((auc_ext.peaks[0].end_x - 1.0).abs() < 1e-12, "expected crossing at x=1.0, got {}", auc_ext.peaks[0].end_x);

        // 4. Zero-area underflow peak must be rejected when min_area_fraction > 0.0.
        let tiny_peak = [(0.0, 0.0), (1e-200, 1e-200), (2e-200, 0.0)];
        let auc_filtered = compute_auc(&tiny_peak, &AucOptions { min_area_fraction: 0.1, ..AucOptions::default() }).expect("filtered AUC");
        assert!(auc_filtered.peaks.is_empty(), "zero-area peak must be rejected when min_area_fraction > 0");
        let auc_unfiltered = compute_auc(&tiny_peak, &AucOptions::default()).expect("unfiltered AUC");
        assert_eq!(auc_unfiltered.peaks.len(), 1);

        // 5. Moving average overflow-safe mean on [1e308, 1e308].
        let ma = smooth_xy(&[(0.0, 1e308), (1.0, 1e308)], SmoothMethod::MovingAverage { window: 2 }).expect("MA large");
        assert!((ma[0].1 - 1e308).abs() <= 1e295);
    }
}
