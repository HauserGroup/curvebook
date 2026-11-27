//! Fraction-of-total normalization and exact binomial confidence intervals
//! (`AnalysisKind::FractionOfTotal`, `R23.3`).
//!
//! Computes proportions or percentages relative to column totals, row totals, the table grand
//! total, or all three simultaneously, with optional exact Clopper–Pearson (1934) beta-quantile
//! or Wilson (1927) score confidence intervals for non-negative integer count tables.
//!
//! References:
//! - Clopper, C. J., & Pearson, E. S. (1934). The use of confidence or fiducial limits illustrated
//!   in the case of the binomial. *Biometrika*, 26(4), 404–413. `doi:10.1093/biomet/26.4.404`.
//! - Wilson, E. B. (1927). Probable inference, the law of succession, and statistical inference.
//!   *Journal of the American Statistical Association*, 22(158), 209–212.
//!   `doi:10.1080/01621459.1927.10502953`.
//! - Newcombe, R. G. (1998). Two-sided confidence intervals for the single proportion: comparison of
//!   seven methods. *Statistics in Medicine*, 17(8), 857–872.
//!   `doi:10.1002/(SICI)1097-0258(19980430)17:8<857::AID-SIM777>3.0.CO;2-E`.

use curvebook_doc::error::MAX_SHEET_ID_BYTES;
use curvebook_doc::{
    AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSetColumn, DataSheet, FractionDenominator, FractionOfTotalSpec, GroupedTable,
    MAX_TITLE_BYTES, MultipleVariablesTable, PartsOfWholeTable, ProportionCiMethod, ReplicateStacking, ResultCell, ResultScalarEntry, ResultSheet,
    ResultStatus, ResultTab, ResultTabKind, SubcolumnMode, TableArchetype, VariableColumn, VariableRole, XyTable,
};
use curvebook_math::{dist, special, sum};

use crate::error::TransformError;
use crate::ops::{TransformContext, bounded_suffixed_str};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Basis {
    Column,
    Row,
    Grand,
}

impl Basis {
    const fn tab_id(self) -> &'static str {
        match self {
            Self::Column => "fraction-column",
            Self::Row => "fraction-row",
            Self::Grand => "fraction-grand",
        }
    }

    fn tab_title(self, output_percent: bool) -> String {
        let noun = if output_percent { "Percent" } else { "Fraction" };
        match self {
            Self::Column => format!("{noun} of column total"),
            Self::Row => format!("{noun} of row total"),
            Self::Grand => format!("{noun} of grand total"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FractionCell {
    value: Option<f64>,
    ci: Option<(f64, f64)>,
}

/// Compute exact two-sided Clopper–Pearson (1934) binomial proportion confidence interval at
/// significance level `alpha = 1 - confidence` for integer counts `0 <= k <= n` (`n >= 1`).
#[must_use]
pub fn clopper_pearson_ci(k: f64, n: f64, alpha: f64) -> Option<(f64, f64)> {
    if !is_valid_binomial_counts(k, n) || !alpha.is_finite() || !(0.0 < alpha && alpha < 1.0) {
        return None;
    }
    let half_alpha = 0.5 * alpha;
    let lo = if k == 0.0 { 0.0 } else { special::reg_beta_inv(k, n - k + 1.0, half_alpha).ok()? };
    let hi = if k == n { 1.0 } else { special::reg_beta_c_inv(k + 1.0, n - k, half_alpha).ok()? };
    if lo.is_finite() && hi.is_finite() { Some((lo.clamp(0.0, 1.0), hi.clamp(0.0, 1.0))) } else { None }
}

/// Compute two-sided Wilson (1927) score binomial proportion confidence interval (without continuity
/// correction) at significance level `alpha = 1 - confidence` for counts `0 <= k <= n` (`n >= 1`).
#[must_use]
pub fn wilson_score_ci(k: f64, n: f64, alpha: f64) -> Option<(f64, f64)> {
    if !is_valid_binomial_counts(k, n) || !alpha.is_finite() || !(0.0 < alpha && alpha < 1.0) {
        return None;
    }
    let z = dist::norm_two_sided_isf(alpha).ok()?;
    let p = k / n;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denom;
    let half = (z / denom) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).max(0.0).sqrt();
    let lo = if k == 0.0 { 0.0 } else { (center - half).clamp(0.0, 1.0) };
    let hi = if k == n { 1.0 } else { (center + half).clamp(0.0, 1.0) };
    if lo.is_finite() && hi.is_finite() { Some((lo, hi)) } else { None }
}

fn is_exact_nonneg_integer(v: f64) -> bool {
    v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= 9_007_199_254_740_992.0
}

fn is_valid_binomial_counts(k: f64, n: f64) -> bool {
    is_exact_nonneg_integer(k) && is_exact_nonneg_integer(n) && n >= 1.0 && k <= n
}

/// Execute [`AnalysisKind::FractionOfTotal`](curvebook_doc::AnalysisKind::FractionOfTotal) against
/// a compatible [`DataSheet`] (`Column`, `Xy`, `Grouped`, `Contingency`, `PartsOfWhole`, or
/// `MultipleVariables`) and return a populated [`ResultSheet`] with tabs, scalars, and
/// `analyzable_data`.
pub fn execute_fraction_of_total(
    source: &DataSheet,
    analysis: &AnalysisSpec,
    spec: &FractionOfTotalSpec,
    ctx: &TransformContext<'_>,
) -> Result<ResultSheet, TransformError> {
    source.archetype.validate()?;
    let conf = analysis.confidence_level.percent() / 100.0;
    let alpha = analysis.confidence_level.alpha();
    if !alpha.is_finite() || !(0.0 < alpha && alpha < 1.0) {
        return Err(TransformError::InvalidSpec { message: format!("confidence alpha must be in (0, 1), got {alpha}") });
    }

    let col_count = source.archetype.column_count();
    let selected = resolve_selected_columns(col_count, &ctx.selected_data_sets, &analysis.selected_data_sets)?;
    if selected.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("table has no data columns for Fraction of Total") });
    }

    let (col_titles, row_labels, matrix) = extract_numeric_matrix(&source.archetype, &selected)?;
    let n_rows = matrix.len();
    let n_cols = col_titles.len();

    let row_totals: Vec<f64> = matrix
        .iter()
        .map(|row| {
            let active: Vec<f64> = row.iter().copied().flatten().collect();
            sum::sum(&active)
        })
        .collect();
    let col_totals: Vec<f64> = (0..n_cols)
        .map(|c| {
            let active: Vec<f64> = matrix.iter().filter_map(|row| row.get(c).copied().flatten()).collect();
            sum::sum(&active)
        })
        .collect();
    let all_active: Vec<f64> = matrix.iter().flat_map(|row| row.iter().copied().flatten()).collect();
    let grand_total = sum::sum(&all_active);

    let bases: &[Basis] = match spec.denominator {
        FractionDenominator::ColumnTotal => &[Basis::Column],
        FractionDenominator::RowTotal => &[Basis::Row],
        FractionDenominator::GrandTotal => &[Basis::Grand],
        FractionDenominator::AllThree => &[Basis::Column, Basis::Row, Basis::Grand],
    };

    let mut sheet = ResultSheet::new(analysis.clone())?;
    sheet.status = ResultStatus::UpToDate;

    let mut primary_grid: Option<Vec<Vec<FractionCell>>> = None;
    for (tab_idx, &basis) in bases.iter().enumerate() {
        let (grid, omitted_ci_cells) = compute_basis_grid(&matrix, &row_totals, &col_totals, grand_total, basis, spec, alpha);
        let tab = build_fraction_tab(&col_titles, &row_labels, &grid, basis, spec, conf, omitted_ci_cells, tab_idx == 0);
        sheet.tabs.push(tab);
        if primary_grid.is_none() {
            primary_grid = Some(grid);
        }
    }

    if grand_total.is_finite() {
        sheet.scalars.push(ResultScalarEntry { key: String::from("GrandTotal"), data_set_index: None, value: grand_total, units: None });
    }
    for (local_idx, &orig_col) in selected.iter().enumerate() {
        if let Some(&tot) = col_totals.get(local_idx)
            && tot.is_finite()
        {
            sheet.scalars.push(ResultScalarEntry { key: String::from("ColumnTotal"), data_set_index: Some(orig_col), value: tot, units: None });
        }
    }
    #[allow(clippy::cast_precision_loss)]
    sheet.scalars.push(ResultScalarEntry { key: String::from("RowCount"), data_set_index: None, value: n_rows as f64, units: None });

    if let Some(grid) = primary_grid {
        let analyzable = build_analyzable_sheet(source, analysis, spec, &col_titles, &row_labels, &grid, ctx)?;
        sheet.analyzable_data = Some(analyzable);
    }

    Ok(sheet)
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

type ExtractedMatrix = (Vec<String>, Vec<String>, Vec<Vec<Option<f64>>>);

fn extract_dataset_matrix(
    data_sets: &[DataSetColumn],
    mode: SubcolumnMode,
    row_titles: &[String],
    row_count: usize,
    selected: &[usize],
) -> ExtractedMatrix {
    let col_titles: Vec<String> = selected.iter().map(|&c| data_sets[c].title.clone()).collect();
    let labels = fallback_row_labels(row_titles, row_count);
    let mut matrix = Vec::with_capacity(row_count);
    for r in 0..row_count {
        let mut row = Vec::with_capacity(selected.len());
        for &c in selected {
            let val = data_sets[c].cells.get(r).and_then(|subs| {
                if mode.is_pre_summarized() || mode == SubcolumnMode::Single {
                    subs.first().and_then(Cell::active_number)
                } else {
                    let active: Vec<f64> = subs.iter().filter_map(Cell::active_number).collect();
                    if active.is_empty() { None } else { sum::mean(&active).ok() }
                }
            });
            row.push(val);
        }
        matrix.push(row);
    }
    (col_titles, labels, matrix)
}

fn extract_numeric_matrix(arch: &TableArchetype, selected: &[usize]) -> Result<ExtractedMatrix, TransformError> {
    match arch {
        TableArchetype::Column(t) => Ok(extract_dataset_matrix(&t.data_sets, t.subcolumn_mode, &t.row_titles, t.row_count(), selected)),
        TableArchetype::Xy(t) => {
            let (col_titles, mut labels, matrix) = extract_dataset_matrix(&t.data_sets, t.y_subcolumn_mode, &t.row_titles, t.row_count(), selected);
            for (r, label) in labels.iter_mut().enumerate() {
                if t.row_titles.get(r).is_none_or(|s| s.trim().is_empty())
                    && let Ok(Some(x)) = t.x_summary_at(r)
                    && x.is_finite()
                {
                    let norm = if x == 0.0 { 0.0 } else { x };
                    *label = format!("{norm}");
                }
            }
            Ok((col_titles, labels, matrix))
        }
        TableArchetype::Grouped(t) => Ok(extract_dataset_matrix(&t.data_sets, t.subcolumn_mode, &t.row_titles, t.row_count(), selected)),
        TableArchetype::Contingency(t) => Ok(extract_dataset_matrix(&t.data_sets, SubcolumnMode::Single, &t.row_titles, t.row_count(), selected)),
        TableArchetype::PartsOfWhole(t) => Ok(extract_dataset_matrix(&t.data_sets, SubcolumnMode::Single, &t.slice_labels, t.row_count(), selected)),
        TableArchetype::MultipleVariables(mv) => {
            let row_count = mv.row_count();
            let col_titles: Vec<String> = selected.iter().map(|&c| mv.variables[c].title.clone()).collect();
            let labels = fallback_row_labels(&mv.row_titles, row_count);
            let mut matrix = Vec::with_capacity(row_count);
            for r in 0..row_count {
                let mut row = Vec::with_capacity(selected.len());
                for &c in selected {
                    row.push(mv.variables[c].cells.get(r).and_then(Cell::active_number));
                }
                matrix.push(row);
            }
            Ok((col_titles, labels, matrix))
        }
        TableArchetype::Survival(_) | TableArchetype::Nested(_) => {
            Err(TransformError::IncompatibleTable { operation: "Fraction of Total", kind: arch.kind() })
        }
    }
}

fn fallback_row_labels(row_titles: &[String], row_count: usize) -> Vec<String> {
    (0..row_count)
        .map(|r| {
            let t = row_titles.get(r).map(String::as_str).unwrap_or("").trim();
            if t.is_empty() { format!("Row {}", r + 1) } else { t.to_owned() }
        })
        .collect()
}

fn compute_basis_grid(
    matrix: &[Vec<Option<f64>>],
    row_totals: &[f64],
    col_totals: &[f64],
    grand_total: f64,
    basis: Basis,
    spec: &FractionOfTotalSpec,
    alpha: f64,
) -> (Vec<Vec<FractionCell>>, usize) {
    let scale = if spec.output_percent { 100.0 } else { 1.0 };
    let mut omitted_ci = 0_usize;
    let mut grid = Vec::with_capacity(matrix.len());

    for (r, row) in matrix.iter().enumerate() {
        let mut out_row = Vec::with_capacity(row.len());
        for (c, &cell_opt) in row.iter().enumerate() {
            let Some(val) = cell_opt else {
                out_row.push(FractionCell { value: None, ci: None });
                continue;
            };
            let denom = match basis {
                Basis::Column => col_totals.get(c).copied().unwrap_or(0.0),
                Basis::Row => row_totals.get(r).copied().unwrap_or(0.0),
                Basis::Grand => grand_total,
            };
            let frac = if denom != 0.0 && denom.is_finite() {
                let scaled = (val / denom) * scale;
                scaled.is_finite().then_some(scaled)
            } else {
                None
            };
            let ci = match spec.ci_method {
                ProportionCiMethod::None => None,
                ProportionCiMethod::Wilson => {
                    let raw = wilson_score_ci(val, denom, alpha);
                    if raw.is_none() {
                        omitted_ci += 1;
                    }
                    raw.map(|(lo, hi)| (lo * scale, hi * scale))
                }
                ProportionCiMethod::ClopperPearson => {
                    let raw = clopper_pearson_ci(val, denom, alpha);
                    if raw.is_none() {
                        omitted_ci += 1;
                    }
                    raw.map(|(lo, hi)| (lo * scale, hi * scale))
                }
            };
            out_row.push(FractionCell { value: frac, ci });
        }
        grid.push(out_row);
    }
    (grid, omitted_ci)
}

#[allow(clippy::too_many_arguments)]
fn build_fraction_tab(
    col_titles: &[String],
    row_labels: &[String],
    grid: &[Vec<FractionCell>],
    basis: Basis,
    spec: &FractionOfTotalSpec,
    conf: f64,
    omitted_ci: usize,
    is_main: bool,
) -> ResultTab {
    let with_ci = spec.ci_method != ProportionCiMethod::None;
    let mut column_headers = Vec::with_capacity(col_titles.len() * if with_ci { 3 } else { 1 });
    for title in col_titles {
        column_headers.push(title.clone());
        if with_ci {
            column_headers.push(format!("{title} (Lower CI)"));
            column_headers.push(format!("{title} (Upper CI)"));
        }
    }

    let mut rows = Vec::with_capacity(grid.len());
    for row in grid {
        let mut rcells = Vec::with_capacity(column_headers.len());
        for cell in row {
            rcells.push(cell.value.map_or(ResultCell::Empty, |v| ResultCell::Number { value: v, prefix: None, units: None }));
            if with_ci {
                rcells.push(cell.ci.map_or(ResultCell::Empty, |(lo, _)| ResultCell::Number { value: lo, prefix: None, units: None }));
                rcells.push(cell.ci.map_or(ResultCell::Empty, |(_, hi)| ResultCell::Number { value: hi, prefix: None, units: None }));
            }
        }
        rows.push(rcells);
    }

    let mut footnotes = Vec::new();
    match spec.ci_method {
        ProportionCiMethod::None => {}
        ProportionCiMethod::Wilson => {
            footnotes
                .push(format!("Binomial {:.1}% confidence intervals use Wilson (1927) score limits without continuity correction.", conf * 100.0));
        }
        ProportionCiMethod::ClopperPearson => {
            footnotes.push(format!("Binomial {:.1}% confidence intervals use exact Clopper-Pearson (1934) beta-quantile limits.", conf * 100.0));
        }
    }
    if omitted_ci > 0 {
        footnotes.push(format!(
            "Omitted binomial confidence intervals on {omitted_ci} cell(s) because binomial CIs require non-negative integer counts with 0 <= k <= n and n >= 1."
        ));
    }

    ResultTab {
        id: String::from(basis.tab_id()),
        title: basis.tab_title(spec.output_percent),
        kind: if is_main { ResultTabKind::MainSummary } else { ResultTabKind::DescriptiveSummary },
        column_headers,
        row_labels: row_labels.to_vec(),
        rows,
        footnotes,
    }
}

fn build_analyzable_sheet(
    source: &DataSheet,
    analysis: &AnalysisSpec,
    spec: &FractionOfTotalSpec,
    col_titles: &[String],
    row_labels: &[String],
    grid: &[Vec<FractionCell>],
    ctx: &TransformContext<'_>,
) -> Result<DataSheet, TransformError> {
    let data_id = ctx.output_id.clone().unwrap_or_else(|| bounded_suffixed_str(analysis.id.as_str(), "-fraction", MAX_SHEET_ID_BYTES));
    let data_title =
        ctx.output_title.clone().unwrap_or_else(|| bounded_suffixed_str(analysis.effective_title(), " (Fraction of Total)", MAX_TITLE_BYTES));
    let with_ci = spec.ci_method != ProportionCiMethod::None;
    let n_rows = grid.len();
    let n_cols = col_titles.len();

    let archetype = match &source.archetype {
        TableArchetype::Xy(xy) => {
            let sub_mode = if with_ci { SubcolumnMode::UpperLowerLimits } else { SubcolumnMode::Single };
            let mut out_xy = XyTable::new_with_x_mode(xy.x_mode.clone(), sub_mode, n_cols, n_rows)?;
            out_xy.x_title = xy.x_title.clone();
            out_xy.x_cells = xy.x_cells.clone();
            out_xy.row_titles = xy.row_titles.clone();
            out_xy.row_titles_expanded = xy.row_titles_expanded;
            populate_fraction_datasets(&mut out_xy.data_sets, col_titles, grid, with_ci);
            TableArchetype::Xy(out_xy)
        }
        TableArchetype::Grouped(g) => {
            let sub_mode = if with_ci { SubcolumnMode::UpperLowerLimits } else { SubcolumnMode::RawReplicates(1) };
            let mut out_g = GroupedTable::new(sub_mode, n_cols, n_rows)?;
            out_g.row_factor_title = g.row_factor_title.clone();
            out_g.column_factor_title = g.column_factor_title.clone();
            out_g.row_titles = g.row_titles.clone();
            out_g.row_numeric_spacing = g.row_numeric_spacing.clone();
            out_g.replicate_stacking = ReplicateStacking::SideBySide;
            populate_fraction_datasets(&mut out_g.data_sets, col_titles, grid, with_ci);
            TableArchetype::Grouped(out_g)
        }
        TableArchetype::Column(c) if !with_ci => {
            let design_hint = if c.design_hint == ColumnDesignHint::PrecomputedError { ColumnDesignHint::UnpairedReplicates } else { c.design_hint };
            let mut out_c = ColumnTable::new_with_subcolumns(design_hint, SubcolumnMode::Single, n_cols, n_rows)?;
            out_c.row_titles = c.row_titles.clone();
            out_c.row_titles_expanded = c.row_titles_expanded;
            populate_fraction_datasets(&mut out_c.data_sets, col_titles, grid, false);
            TableArchetype::Column(out_c)
        }
        TableArchetype::PartsOfWhole(p) if !with_ci && grid.iter().flatten().all(|fc| fc.value.is_none_or(|v| v >= 0.0)) => {
            let mut out_p = PartsOfWholeTable::new(n_cols, n_rows)?;
            out_p.slice_labels = p.slice_labels.clone();
            populate_fraction_datasets(&mut out_p.data_sets, col_titles, grid, false);
            TableArchetype::PartsOfWhole(out_p)
        }
        _ => build_mv_fraction_archetype(col_titles, row_labels, grid, with_ci)?,
    };

    let sheet = DataSheet::new(data_id, data_title, archetype)?;
    sheet.validate()?;
    Ok(sheet)
}

fn populate_fraction_datasets(data_sets: &mut [DataSetColumn], col_titles: &[String], grid: &[Vec<FractionCell>], with_ci: bool) {
    for (c, ds) in data_sets.iter_mut().enumerate() {
        if let Some(title) = col_titles.get(c) {
            ds.title = title.clone();
        }
        for (r, row) in grid.iter().enumerate() {
            if let Some(fc) = row.get(c)
                && let Some(slot) = ds.cells.get_mut(r)
            {
                if with_ci {
                    *slot = vec![
                        fc.value.map_or_else(Cell::empty, Cell::number),
                        fc.ci.map_or_else(Cell::empty, |(lo, _)| Cell::number(lo)),
                        fc.ci.map_or_else(Cell::empty, |(_, hi)| Cell::number(hi)),
                    ];
                } else {
                    *slot = vec![fc.value.map_or_else(Cell::empty, Cell::number)];
                }
            }
        }
    }
}

fn build_mv_fraction_archetype(
    col_titles: &[String],
    row_labels: &[String],
    grid: &[Vec<FractionCell>],
    with_ci: bool,
) -> Result<TableArchetype, TransformError> {
    let n_rows = grid.len();
    let mut variables = Vec::with_capacity(col_titles.len() * if with_ci { 3 } else { 1 });
    for (c, title) in col_titles.iter().enumerate() {
        let mut main_col = VariableColumn::new(title.clone(), n_rows)?;
        main_col.lock_role(VariableRole::Continuous);
        for (r, row) in grid.iter().enumerate() {
            if let Some(v) = row.get(c).and_then(|fc| fc.value) {
                main_col.cells[r] = Cell::number(v);
            }
        }
        variables.push(main_col);

        if with_ci {
            let mut lo_col = VariableColumn::new(format!("{title} (Lower CI)"), n_rows)?;
            lo_col.lock_role(VariableRole::Continuous);
            let mut hi_col = VariableColumn::new(format!("{title} (Upper CI)"), n_rows)?;
            hi_col.lock_role(VariableRole::Continuous);
            for (r, row) in grid.iter().enumerate() {
                if let Some((lo, hi)) = row.get(c).and_then(|fc| fc.ci) {
                    lo_col.cells[r] = Cell::number(lo);
                    hi_col.cells[r] = Cell::number(hi);
                }
            }
            variables.push(lo_col);
            variables.push(hi_col);
        }
    }
    let mv = MultipleVariablesTable { row_titles: row_labels.to_vec(), row_titles_expanded: !row_labels.is_empty(), variables };
    Ok(TableArchetype::MultipleVariables(mv))
}
