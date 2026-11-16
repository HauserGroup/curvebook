//! Standalone multiplicity analysis for a stack (column) of raw *P* values (`R10.10`).
//!
//! Given one or more columns of valid probability values `p in [0.0, 1.0]` and a threshold
//! `alpha` (for family-wise error rate procedures) or desired false discovery rate `Q` (for
//! false-discovery-rate procedures) strictly in `(0.0, 1.0)`, computes multiplicity-adjusted
//! *P* values (or *q*-values), rank mappings, and boolean discovery decisions across all seven
//! classical procedures by delegating to [`crate::multiplicity`]:
//!
//! - **Bonferroni** (single-step FWER): Dunn, O. J. (1961). *Multiple comparisons among means*.
//!   Journal of the American Statistical Association, 56(293), 52–64. (Following Bonferroni, C. E.,
//!   1936, *Teoria statistica delle classi e calcolo delle probabilità*, Pubblicazioni del R.
//!   Istituto Superiore di Scienze Economiche e Commerciali di Firenze, 8, 3–62.)
//! - **Holm (1979)** (step-down FWER): Holm, S. (1979). *A simple sequentially rejective multiple
//!   test procedure*. Scandinavian Journal of Statistics, 6(2), 65–70.
//! - **Šídák (1967)** (single-step FWER under independence/positive orthant dependence): Šidák, Z.
//!   (1967). *Rectangular confidence regions for the means of multivariate normal distributions*.
//!   Journal of the American Statistical Association, 62(318), 626–633.
//! - **Holm–Šídák** (step-down FWER): Holm, S. (1979), ibid.; Šidák, Z. (1967), ibid.; and
//!   Holland, B. S., & Copenhaver, M. D. (1987). *An improved sequentially rejective Bonferroni test
//!   procedure*. Biometrics, 43(2), 417–423.
//! - **Benjamini–Hochberg (1995)** (linear step-up FDR): Benjamini, Y., & Hochberg, Y. (1995).
//!   *Controlling the false discovery rate: a practical and powerful approach to multiple testing*.
//!   Journal of the Royal Statistical Society, Series B (Methodological), 57(1), 289–300.
//! - **Benjamini–Krieger–Yekutieli two-stage adaptive (2006)** (two-stage adaptive step-up FDR):
//!   Benjamini, Y., Krieger, A. M., & Yekutieli, D. (2006). *Adaptive linear step-up procedures
//!   that control the false discovery rate*. Biometrika, 93(3), 491–507.
//! - **Benjamini–Yekutieli (2001)** (step-up FDR under arbitrary dependence): Benjamini, Y., &
//!   Yekutieli, D. (2001). *The control of the false discovery rate in multiple testing under
//!   dependency*. Annals of Statistics, 29(4), 1165–1188.

use std::collections::HashSet;

use lumencraft_doc::{
    AnalysisSpec, Cell, CellValue, ChecklistStatus, DataSheet, PValueStackMethod, PValueStackSpec, ResultCell, ResultSheet, ResultTabKind,
    TableArchetype, VariableRole,
};
use lumencraft_math::MathError;
use serde::{Deserialize, Serialize};

use crate::common::{
    append_ignored_text_checklist_and_footnote, build_tab, checklist_item, init_result_sheet, num_cell, p_cell, push_scalar,
    sanitize_result_sheet_floats,
};
use crate::error::StatsError;
use crate::multiplicity::{
    adjust_benjamini_hochberg, adjust_benjamini_yekutieli, adjust_bonferroni, adjust_holm_bonferroni, adjust_holm_sidak, adjust_sidak,
    adjust_two_stage_bky,
};

/// Result for a single row in an analyzed stack of raw *P* values, preserving input row order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PValueStackRowResult {
    /// Zero-based position among the active observations passed into the procedure (`0..m`).
    pub input_index: usize,
    /// Zero-based row index in the source table (`== input_index` in the slice API).
    pub table_row_index: usize,
    /// Row label from the source table (`Row <N>` when blank).
    pub row_label: String,
    /// One-based stable ascending sorted order (`1..=m`, smallest *P* = `1`).
    pub sorted_order: usize,
    /// One-based competition rank (`1..=m`), assigning identical rank to tied raw *P* values.
    pub rank: usize,
    /// Validated raw *P* value in `[0.0, 1.0]`.
    pub raw_p: f64,
    /// Multiplicity-adjusted *P* value (FWER methods) or *q*-value (FDR methods) in `[0.0, 1.0]`.
    pub adjusted_p_or_q: f64,
    /// Discovery / significance decision (`adjusted_p_or_q <= threshold`).
    pub discovery: bool,
}

/// Complete multiplicity summary for one column (stack) of raw *P* values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PValueStackColumnResult {
    pub data_set_index: usize,
    pub column_label: String,
    pub method: PValueStackMethod,
    pub is_fdr: bool,
    pub threshold: f64,
    pub count: usize,
    pub discovery_count: usize,
    pub min_raw_p: f64,
    pub min_adjusted: f64,
    pub skipped_excluded_or_empty: usize,
    pub ignored_non_numeric: usize,
    /// Per-observation results strictly in original input row order.
    pub rows: Vec<PValueStackRowResult>,
}

/// Validate threshold (`alpha` or `Q`) strictly within `(0.0, 1.0)`.
pub fn validate_pvalue_threshold(threshold: f64) -> Result<(), StatsError> {
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    Ok(())
}

/// Validate every raw *P* value is finite and lies in `[0.0, 1.0]`.
///
/// Rejects empty slices with [`StatsError::EmptyInput`] and `NaN`, `±Inf`, `< 0.0`, or `> 1.0` with
/// [`StatsError::Math`] wrapping [`MathError::DomainError`]. Never coerces invalid entries.
pub fn validate_raw_pvalues(raw_p: &[f64]) -> Result<(), StatsError> {
    if raw_p.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    for &p in raw_p {
        if !p.is_finite() || !(0.0..=1.0).contains(&p) {
            return Err(StatsError::Math(MathError::DomainError { name: "p_value", value: p }));
        }
    }
    Ok(())
}

/// Adjust a slice of raw *P* values using `method` at threshold `threshold` (`alpha` or `Q`),
/// returning adjusted *P* / *q* values and discovery flags strictly in input order.
pub fn analyze_pvalue_stack(raw_p: &[f64], method: PValueStackMethod, threshold: f64) -> Result<PValueStackColumnResult, StatsError> {
    validate_pvalue_threshold(threshold)?;
    validate_raw_pvalues(raw_p)?;
    let normalized: Vec<f64> = raw_p.iter().map(|&p| if p == 0.0 { 0.0 } else { p }).collect();
    let labels: Vec<String> = (1..=normalized.len()).map(|n| format!("Row {n}")).collect();
    let table_rows: Vec<usize> = (0..normalized.len()).collect();
    compute_column_result(0, String::from("P values"), &normalized, &table_rows, &labels, method, threshold, 0, 0)
}

fn dispatch_multiplicity(raw_p: &[f64], method: PValueStackMethod, threshold: f64) -> Vec<f64> {
    match method {
        PValueStackMethod::Bonferroni => adjust_bonferroni(raw_p),
        PValueStackMethod::HolmBonferroni => adjust_holm_bonferroni(raw_p),
        PValueStackMethod::Sidak => adjust_sidak(raw_p),
        PValueStackMethod::HolmSidak => adjust_holm_sidak(raw_p),
        PValueStackMethod::BenjaminiHochberg => adjust_benjamini_hochberg(raw_p),
        PValueStackMethod::TwoStageBky => adjust_two_stage_bky(raw_p, threshold),
        PValueStackMethod::BenjaminiYekutieli => adjust_benjamini_yekutieli(raw_p),
    }
}

fn compute_ranks(raw_p: &[f64]) -> (Vec<usize>, Vec<usize>) {
    let mut order: Vec<usize> = (0..raw_p.len()).collect();
    order.sort_by(|&a, &b| {
        let pa = raw_p.get(a).copied().unwrap_or(1.0);
        let pb = raw_p.get(b).copied().unwrap_or(1.0);
        pa.total_cmp(&pb).then_with(|| a.cmp(&b))
    });

    let mut sorted_order = vec![0_usize; raw_p.len()];
    let mut competition_rank = vec![0_usize; raw_p.len()];
    let mut current_rank = 1_usize;
    let mut prev_p: Option<f64> = None;

    for (pos, &orig_idx) in order.iter().enumerate() {
        let p = raw_p.get(orig_idx).copied().unwrap_or(1.0);
        if let Some(pp) = prev_p
            && p.total_cmp(&pp) != std::cmp::Ordering::Equal
        {
            current_rank = pos + 1;
        }
        if let Some(slot) = sorted_order.get_mut(orig_idx) {
            *slot = pos + 1;
        }
        if let Some(slot) = competition_rank.get_mut(orig_idx) {
            *slot = current_rank;
        }
        prev_p = Some(p);
    }

    (sorted_order, competition_rank)
}

#[allow(clippy::too_many_arguments)]
fn compute_column_result(
    data_set_index: usize,
    column_label: String,
    raw_p: &[f64],
    table_row_indices: &[usize],
    row_labels: &[String],
    method: PValueStackMethod,
    threshold: f64,
    skipped_excluded_or_empty: usize,
    ignored_non_numeric: usize,
) -> Result<PValueStackColumnResult, StatsError> {
    if raw_p.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let adjusted = dispatch_multiplicity(raw_p, method, threshold);
    if adjusted.len() != raw_p.len() {
        return Err(StatsError::DimensionMismatch { expected: raw_p.len(), actual: adjusted.len() });
    }
    let (sorted_orders, ranks) = compute_ranks(raw_p);

    let mut rows = Vec::with_capacity(raw_p.len());
    let mut discovery_count = 0_usize;
    let mut min_raw_p = f64::INFINITY;
    let mut min_adjusted = f64::INFINITY;

    for idx in 0..raw_p.len() {
        let rp = raw_p.get(idx).copied().unwrap_or(1.0);
        let adj_raw = adjusted.get(idx).copied().unwrap_or(1.0);
        if !adj_raw.is_finite() {
            return Err(StatsError::Math(MathError::DomainError { name: "adjusted_p_value", value: adj_raw }));
        }
        let adj = if adj_raw == 0.0 { 0.0 } else { adj_raw.clamp(0.0, 1.0) };
        let discovery = adj <= threshold;
        if discovery {
            discovery_count += 1;
        }
        min_raw_p = min_raw_p.min(rp);
        min_adjusted = min_adjusted.min(adj);

        let table_row_index = table_row_indices.get(idx).copied().unwrap_or(idx);
        let row_label = row_labels.get(idx).cloned().unwrap_or_else(|| format!("Row {}", table_row_index + 1));
        rows.push(PValueStackRowResult {
            input_index: idx,
            table_row_index,
            row_label,
            sorted_order: sorted_orders.get(idx).copied().unwrap_or(idx + 1),
            rank: ranks.get(idx).copied().unwrap_or(idx + 1),
            raw_p: rp,
            adjusted_p_or_q: adj,
            discovery,
        });
    }

    Ok(PValueStackColumnResult {
        data_set_index,
        column_label,
        method,
        is_fdr: method.is_fdr(),
        threshold,
        count: rows.len(),
        discovery_count,
        min_raw_p,
        min_adjusted,
        skipped_excluded_or_empty,
        ignored_non_numeric,
        rows,
    })
}

fn parse_nonfinite_token(trimmed: &str) -> Option<f64> {
    if trimmed.eq_ignore_ascii_case("nan") {
        return Some(f64::NAN);
    }
    if trimmed.eq_ignore_ascii_case("inf")
        || trimmed.eq_ignore_ascii_case("+inf")
        || trimmed.eq_ignore_ascii_case("infinity")
        || trimmed.eq_ignore_ascii_case("+infinity")
    {
        return Some(f64::INFINITY);
    }
    if trimmed.eq_ignore_ascii_case("-inf") || trimmed.eq_ignore_ascii_case("-infinity") {
        return Some(f64::NEG_INFINITY);
    }
    None
}

struct ExtractedPValueColumn {
    data_set_index: usize,
    label: String,
    raw_p: Vec<f64>,
    table_row_indices: Vec<usize>,
    row_labels: Vec<String>,
    skipped_excluded_or_empty: usize,
    ignored_non_numeric: usize,
}

enum ColumnSource<'a> {
    Grid { label: String, row_titles: &'a [String], cells: &'a [Vec<Cell>], is_categorical: bool },
    SingleCol { label: String, row_titles: &'a [String], cells: &'a [Cell], is_categorical: bool },
}

impl<'a> ColumnSource<'a> {
    fn label(&self) -> &str {
        match self {
            Self::Grid { label, .. } | Self::SingleCol { label, .. } => label,
        }
    }

    fn is_categorical(&self) -> bool {
        match self {
            Self::Grid { is_categorical, .. } | Self::SingleCol { is_categorical, .. } => *is_categorical,
        }
    }
}

fn extract_one_column(col_idx: usize, src: &ColumnSource<'_>) -> Result<ExtractedPValueColumn, StatsError> {
    let mut raw_p = Vec::new();
    let mut table_row_indices = Vec::new();
    let mut row_labels = Vec::new();
    let mut skipped_excluded_or_empty = 0_usize;
    let mut ignored_non_numeric = 0_usize;

    match src {
        ColumnSource::Grid { row_titles, cells, .. } => {
            for (r, row_cells) in cells.iter().enumerate() {
                let base_label =
                    row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| format!("Row {}", r + 1));
                let multiple_subs = row_cells.len() > 1;
                for (sub_idx, cell) in row_cells.iter().enumerate() {
                    let row_lbl = if multiple_subs { format!("{base_label} (Rep {})", sub_idx + 1) } else { base_label.clone() };
                    collect_cell(
                        r,
                        row_lbl,
                        cell,
                        &mut raw_p,
                        &mut table_row_indices,
                        &mut row_labels,
                        &mut skipped_excluded_or_empty,
                        &mut ignored_non_numeric,
                    )?;
                }
            }
        }
        ColumnSource::SingleCol { row_titles, cells, .. } => {
            for (r, cell) in cells.iter().enumerate() {
                let row_lbl =
                    row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| format!("Row {}", r + 1));
                collect_cell(
                    r,
                    row_lbl,
                    cell,
                    &mut raw_p,
                    &mut table_row_indices,
                    &mut row_labels,
                    &mut skipped_excluded_or_empty,
                    &mut ignored_non_numeric,
                )?;
            }
        }
    }

    Ok(ExtractedPValueColumn {
        data_set_index: col_idx,
        label: if src.label().trim().is_empty() { format!("Column {}", col_idx + 1) } else { src.label().to_owned() },
        raw_p,
        table_row_indices,
        row_labels,
        skipped_excluded_or_empty,
        ignored_non_numeric,
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_cell(
    row_idx: usize,
    row_label: String,
    cell: &Cell,
    raw_p: &mut Vec<f64>,
    table_row_indices: &mut Vec<usize>,
    row_labels: &mut Vec<String>,
    skipped_excluded_or_empty: &mut usize,
    ignored_non_numeric: &mut usize,
) -> Result<(), StatsError> {
    if cell.excluded {
        *skipped_excluded_or_empty += 1;
        return Ok(());
    }
    match &cell.value {
        CellValue::Empty => {
            *skipped_excluded_or_empty += 1;
        }
        CellValue::Number(v) => {
            if !v.is_finite() || !(0.0..=1.0).contains(v) {
                return Err(StatsError::Math(MathError::DomainError { name: "p_value", value: *v }));
            }
            raw_p.push(if *v == 0.0 { 0.0 } else { *v });
            table_row_indices.push(row_idx);
            row_labels.push(row_label);
        }
        CellValue::Text(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                *skipped_excluded_or_empty += 1;
            } else if let Some(non_finite) = parse_nonfinite_token(trimmed) {
                return Err(StatsError::Math(MathError::DomainError { name: "p_value", value: non_finite }));
            } else {
                *ignored_non_numeric += 1;
            }
        }
    }
    Ok(())
}

fn collect_column_sources<'a>(sheet: &'a DataSheet) -> Result<Vec<ColumnSource<'a>>, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col_t) => {
            if col_t.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            Ok(col_t
                .data_sets
                .iter()
                .map(|ds| ColumnSource::Grid { label: ds.title.clone(), row_titles: &col_t.row_titles, cells: &ds.cells, is_categorical: false })
                .collect())
        }
        TableArchetype::Grouped(grp_t) => {
            if grp_t.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            Ok(grp_t
                .data_sets
                .iter()
                .map(|ds| ColumnSource::Grid { label: ds.title.clone(), row_titles: &grp_t.row_titles, cells: &ds.cells, is_categorical: false })
                .collect())
        }
        TableArchetype::Xy(xy_t) => {
            if xy_t.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            Ok(xy_t
                .data_sets
                .iter()
                .map(|ds| ColumnSource::Grid { label: ds.title.clone(), row_titles: &xy_t.row_titles, cells: &ds.cells, is_categorical: false })
                .collect())
        }
        TableArchetype::MultipleVariables(mv_t) => Ok(mv_t
            .variables
            .iter()
            .map(|v| ColumnSource::SingleCol {
                label: v.title.clone(),
                row_titles: &mv_t.row_titles,
                cells: &v.cells,
                is_categorical: matches!(v.role, VariableRole::Categorical | VariableRole::Label),
            })
            .collect()),
        other => Err(StatsError::IncompatibleTable { analysis: "analyze a stack of P values", actual: other.kind() }),
    }
}

fn resolve_target_columns(
    sources: &[ColumnSource<'_>],
    ps: &PValueStackSpec,
    selected_data_sets: &[usize],
) -> Result<(Vec<usize>, bool), StatsError> {
    let total = sources.len();
    if let Some(col_idx) = ps.column {
        if col_idx >= total {
            return Err(StatsError::MissingColumn { index: col_idx });
        }
        return Ok((vec![col_idx], true));
    }
    if !selected_data_sets.is_empty() {
        let mut seen = HashSet::new();
        let mut indices = Vec::with_capacity(selected_data_sets.len());
        for &idx in selected_data_sets {
            if idx >= total {
                return Err(StatsError::MissingColumn { index: idx });
            }
            if seen.insert(idx) {
                indices.push(idx);
            }
        }
        return Ok((indices, true));
    }
    let indices: Vec<usize> = sources.iter().enumerate().filter_map(|(idx, s)| (!s.is_categorical()).then_some(idx)).collect();
    Ok((indices, false))
}

/// Execute [`PValueStackSpec`] on a [`DataSheet`] and emit a complete [`ResultSheet`] with
/// summary metrics, input-order adjusted *P* / *q* values, discovery flags, scalars, and
/// assumption checklist entries.
pub fn run_pvalue_stack_analysis(sheet: &DataSheet, spec: &AnalysisSpec, ps: &PValueStackSpec) -> Result<ResultSheet, StatsError> {
    validate_pvalue_threshold(ps.threshold)?;
    let mut res = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;
    let sources = collect_column_sources(sheet)?;
    let (target_indices, explicit_selection) = resolve_target_columns(&sources, ps, &spec.selected_data_sets)?;

    let mut extracted_cols = Vec::new();
    for idx in target_indices {
        let Some(src) = sources.get(idx) else {
            return Err(StatsError::MissingColumn { index: idx });
        };
        let col = extract_one_column(idx, src)?;
        if col.raw_p.is_empty() {
            if explicit_selection {
                return Err(StatsError::EmptyInput);
            }
            continue;
        }
        extracted_cols.push(col);
    }

    if extracted_cols.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let mut col_results = Vec::with_capacity(extracted_cols.len());
    for col in extracted_cols {
        col_results.push(compute_column_result(
            col.data_set_index,
            col.label,
            &col.raw_p,
            &col.table_row_indices,
            &col.row_labels,
            ps.method,
            ps.threshold,
            col.skipped_excluded_or_empty,
            col.ignored_non_numeric,
        )?);
    }

    let sum_headers: Vec<String> = col_results.iter().map(|c| c.column_label.clone()).collect();
    let threshold_label = if ps.method.is_fdr() { String::from("Desired FDR (Q)") } else { String::from("Significance level (alpha)") };
    let adj_header = if ps.method.is_fdr() { String::from("q-value") } else { String::from("Adjusted P") };
    let err_rate_label = if ps.method.is_fdr() { "False Discovery Rate (FDR)" } else { "Family-Wise Error Rate (FWER)" };
    let sum_row_labels = vec![
        String::from("Multiplicity procedure"),
        String::from("Error rate controlled"),
        threshold_label,
        String::from("Input P values (m)"),
        String::from("Discoveries (Significant?)"),
        String::from("Non-discoveries"),
        String::from("Discovery fraction"),
        String::from("Skipped (empty / excluded)"),
        String::from("Minimum raw P"),
        String::from("Minimum adjusted P / q"),
    ];
    let mut sum_rows = vec![Vec::with_capacity(col_results.len()); sum_row_labels.len()];
    let mut total_m = 0_usize;
    let mut total_disc = 0_usize;
    let mut ignored_text_cols = Vec::new();

    for col in &col_results {
        total_m += col.count;
        total_disc += col.discovery_count;
        let non_disc = col.count.saturating_sub(col.discovery_count);
        let frac = (col.discovery_count as f64) / (col.count as f64);

        if let Some(r) = sum_rows.get_mut(0) {
            r.push(ResultCell::Text(String::from(ps.method.label())));
        }
        if let Some(r) = sum_rows.get_mut(1) {
            r.push(ResultCell::Text(String::from(err_rate_label)));
        }
        if let Some(r) = sum_rows.get_mut(2) {
            r.push(num_cell(col.threshold));
        }
        if let Some(r) = sum_rows.get_mut(3) {
            r.push(num_cell(col.count as f64));
        }
        if let Some(r) = sum_rows.get_mut(4) {
            r.push(num_cell(col.discovery_count as f64));
        }
        if let Some(r) = sum_rows.get_mut(5) {
            r.push(num_cell(non_disc as f64));
        }
        if let Some(r) = sum_rows.get_mut(6) {
            r.push(num_cell(frac));
        }
        if let Some(r) = sum_rows.get_mut(7) {
            r.push(num_cell(col.skipped_excluded_or_empty as f64));
        }
        if let Some(r) = sum_rows.get_mut(8) {
            r.push(p_cell(col.min_raw_p, p_fmt));
        }
        if let Some(r) = sum_rows.get_mut(9) {
            r.push(p_cell(col.min_adjusted, p_fmt));
        }

        let ds_idx = Some(col.data_set_index);
        push_scalar(&mut res, "InputCount", ds_idx, col.count as f64);
        push_scalar(&mut res, "Discoveries", ds_idx, col.discovery_count as f64);
        push_scalar(&mut res, "MinRawP", ds_idx, col.min_raw_p);
        push_scalar(&mut res, "MinAdjustedP", ds_idx, col.min_adjusted);
        if col.ignored_non_numeric > 0 {
            ignored_text_cols.push((col.column_label.clone(), col.ignored_non_numeric));
        }
    }
    push_scalar(&mut res, "Threshold", None, ps.threshold);
    push_scalar(&mut res, "TotalInputCount", None, total_m as f64);
    push_scalar(&mut res, "TotalDiscoveries", None, total_disc as f64);

    res.tabs.push(build_tab(
        "pvalue-stack-summary",
        "Analyze a Stack of P Values — Summary",
        ResultTabKind::MainSummary,
        sum_headers,
        sum_row_labels,
        sum_rows,
    ));

    let detail_headers = vec![
        String::from("Column"),
        String::from("Row Index (1-based)"),
        String::from("Rank (ascending P)"),
        String::from("Raw P"),
        adj_header,
        String::from("Discovery?"),
    ];
    let mut detail_row_labels = Vec::with_capacity(total_m);
    let mut detail_rows = Vec::with_capacity(total_m);
    for col in &col_results {
        for r in &col.rows {
            let label = if col_results.len() > 1 { format!("{}: {}", col.column_label, r.row_label) } else { r.row_label.clone() };
            detail_row_labels.push(label);
            detail_rows.push(vec![
                ResultCell::Text(col.column_label.clone()),
                num_cell((r.table_row_index + 1) as f64),
                num_cell(r.rank as f64),
                p_cell(r.raw_p, p_fmt),
                p_cell(r.adjusted_p_or_q, p_fmt),
                ResultCell::Boolean(r.discovery),
            ]);
        }
    }
    res.tabs.push(build_tab(
        "pvalue-stack-adjusted",
        "Adjusted P Values & Discoveries",
        ResultTabKind::MultipleComparisons,
        detail_headers,
        detail_row_labels,
        detail_rows,
    ));

    let threshold_symbol = if ps.method.is_fdr() { "Q" } else { "alpha" };
    res.checklist.push(checklist_item(
        "P-value stack domain & multiplicity control",
        ChecklistStatus::Pass,
        Some(format!("{total_disc} / {total_m} discoveries at {threshold_symbol} = {}", ps.threshold)),
        format!("All active input P values lie in [0, 1] and were adjusted using {}.", ps.method.label()),
    ));
    append_ignored_text_checklist_and_footnote(&mut res, "P-value stack analysis", &ignored_text_cols);
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}
