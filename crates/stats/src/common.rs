//! Shared numerical, ranking, quantile, validation, and `ResultSheet` construction helpers.

use lumencraft_doc::{
    AnalysisSpec, Cell, ChecklistItem, ChecklistStatus, CiBound, DataSheet, PValueFormat, RemediationKind, ResultCell, ResultScalarEntry,
    ResultSheet, ResultStatus, ResultTab, ResultTabKind, SubcolumnMode, TableArchetype,
};
use lumencraft_math::ConfidenceLevel;

use crate::error::StatsError;

/// Validate that `values` is non-empty and every entry is finite.
pub fn validate_finite_slice(values: &[f64]) -> Result<(), StatsError> {
    if values.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    for &v in values {
        if !v.is_finite() {
            return Err(StatsError::Math(lumencraft_math::MathError::DomainError { name: "x", value: v }));
        }
    }
    Ok(())
}

/// Filter `values` into finite-only entries and return a sorted copy.
#[must_use]
pub fn sorted_finite(values: &[f64]) -> Vec<f64> {
    let mut out: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    out.sort_by(f64::total_cmp);
    out
}

/// Linearly interpolated quantile (`R` type 7 / standard continuous quantile) on a sorted slice
/// for `p` in `[0.0, 1.0]`.
#[must_use]
pub fn quantile_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    let n = sorted.len();
    if n == 0 || !p.is_finite() {
        return None;
    }
    let pc = p.clamp(0.0, 1.0);
    if n == 1 {
        return Some(sorted[0]);
    }
    let idx = pc * ((n - 1) as f64);
    let lo = (idx.floor() as usize).min(n - 1);
    let hi = (lo + 1).min(n - 1);
    let frac = idx - (lo as f64);
    Some(sorted[lo] + frac * (sorted[hi] - sorted[lo]))
}

/// Biostatistics hinge quartiles `(Q1, median, Q3)` matching standard Type-6/hinge convention
/// (`R` type 6: `m = p * (n + 1)` clamped to `[1, n]`), which gives exact medians of lower/upper
/// halves on small samples.
#[must_use]
pub fn quartiles_hinge(sorted: &[f64]) -> Option<(f64, f64, f64)> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let q1 = quantile_type6_sorted(sorted, 0.25)?;
    let med = quantile_type6_sorted(sorted, 0.50)?;
    let q3 = quantile_type6_sorted(sorted, 0.75)?;
    Some((q1, med, q3))
}

/// `R` type-6 quantile (`(n + 1) * p` 1-based index with linear interpolation).
#[must_use]
pub fn quantile_type6_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    let n = sorted.len();
    if n == 0 || !p.is_finite() {
        return None;
    }
    if n == 1 {
        return Some(sorted[0]);
    }
    let pos = p.clamp(0.0, 1.0) * ((n + 1) as f64);
    if pos <= 1.0 {
        return Some(sorted[0]);
    }
    if pos >= (n as f64) {
        return Some(sorted[n - 1]);
    }
    let k = pos.floor() as usize;
    let frac = pos - (k as f64);
    let lo = k.saturating_sub(1).min(n - 1);
    let hi = (lo + 1).min(n - 1);
    Some(sorted[lo] + frac * (sorted[hi] - sorted[lo]))
}

/// Output of `rank_with_ties`: 1-based average ranks and counts of tied groups of size >= 2.
#[derive(Debug, Clone, PartialEq)]
pub struct TieRankResult {
    pub ranks: Vec<f64>,
    pub tie_counts: Vec<usize>,
    /// Tie correction numerator `sum_k (t_k^3 - t_k)`.
    pub tie_sum_t3_minus_t: f64,
}

/// Assign 1-based average ranks in `O(n log n)` with exact tie group accounting.
pub fn rank_with_ties(values: &[f64]) -> Result<TieRankResult, StatsError> {
    validate_finite_slice(values)?;
    let n = values.len();
    let mut indexed: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut ranks = vec![0.0_f64; n];
    let mut tie_counts = Vec::new();
    let mut tie_sum = 0.0_f64;
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && indexed[j].1 == indexed[i].1 {
            j += 1;
        }
        let count = j - i;
        let avg_rank = 0.5 * ((i + 1 + j) as f64);
        for item in indexed.iter().take(j).skip(i) {
            ranks[item.0] = avg_rank;
        }
        if count > 1 {
            tie_counts.push(count);
            let cf = count as f64;
            tie_sum += cf * cf * cf - cf;
        }
        i = j;
    }
    Ok(TieRankResult { ranks, tie_counts, tie_sum_t3_minus_t: tie_sum })
}

/// Build a numeric [`ResultCell`].
#[must_use]
pub fn num_cell(value: f64) -> ResultCell {
    if value.is_finite() { ResultCell::Number { value, prefix: None, units: None } } else { ResultCell::Empty }
}

/// Build an optional numeric [`ResultCell`].
#[must_use]
pub fn opt_num_cell(value: Option<f64>) -> ResultCell {
    match value {
        Some(v) if v.is_finite() => ResultCell::Number { value: v, prefix: None, units: None },
        _ => ResultCell::Empty,
    }
}

/// Build a [`ResultCell::ConfidenceInterval`] from finite bounds.
#[must_use]
pub fn ci_cell(lower: f64, upper: f64) -> ResultCell {
    let lo = if lower.is_finite() { CiBound::Finite(lower) } else { CiBound::Open };
    let hi = if upper.is_finite() { CiBound::Finite(upper) } else { CiBound::Open };
    ResultCell::ConfidenceInterval { lower: lo, upper: hi }
}

/// Build a [`ResultCell::ConfidenceInterval`] from `Option<(f64, f64)>`.
#[must_use]
pub fn opt_ci_cell(bounds: Option<(f64, f64)>) -> ResultCell {
    match bounds {
        Some((lo, hi)) => ci_cell(lo, hi),
        None => ResultCell::Empty,
    }
}

/// Build a [`ResultCell::PValue`] storing unrounded `raw` probability and formatted strings.
#[must_use]
pub fn p_cell(raw_p: f64, format: PValueFormat) -> ResultCell {
    if !raw_p.is_finite() {
        return ResultCell::Empty;
    }
    let clamped = raw_p.clamp(0.0, 1.0);
    ResultCell::PValue { raw: clamped, formatted: format.format_p(clamped), stars: format.stars(clamped).to_owned() }
}

/// Construct a [`ChecklistItem`] with an optional structured [`RemediationKind`].
#[must_use]
pub fn checklist_item_with_remediation(
    assumption: impl Into<String>,
    status: ChecklistStatus,
    metric: Option<String>,
    explanation: impl Into<String>,
    remediation: Option<RemediationKind>,
) -> ChecklistItem {
    ChecklistItem { assumption: assumption.into(), status, diagnostic_metric: metric, explanation: explanation.into(), remediation }
}

/// Construct a diagnostic [`ChecklistItem`] with `remediation: None`.
#[must_use]
pub fn checklist_item(
    assumption: impl Into<String>,
    status: ChecklistStatus,
    metric: Option<String>,
    explanation: impl Into<String>,
) -> ChecklistItem {
    checklist_item_with_remediation(assumption, status, metric, explanation, None)
}

/// Initialize an up-to-date [`ResultSheet`] from an [`AnalysisSpec`].
pub fn init_result_sheet(spec: &AnalysisSpec) -> Result<ResultSheet, StatsError> {
    let mut sheet = ResultSheet::new(spec.clone())?;
    sheet.status = ResultStatus::UpToDate;
    Ok(sheet)
}

/// Push a named scalar into `ResultSheet`.
pub fn push_scalar(sheet: &mut ResultSheet, key: impl Into<String>, data_set_index: Option<usize>, value: f64) {
    if value.is_finite() {
        sheet.scalars.push(ResultScalarEntry { key: key.into(), data_set_index, value, units: None });
    }
}

/// Resolve selected dataset column indices (`empty` in `AnalysisSpec::selected_data_sets` means all).
#[must_use]
pub fn resolve_selected_indices(selected: &[usize], total: usize) -> Vec<usize> {
    if selected.is_empty() { (0..total).collect() } else { selected.iter().copied().filter(|&i| i < total).collect() }
}

/// Extracted labeled numeric sample column from a `DataSheet`.
#[derive(Debug, Clone)]
pub struct ExtractedColumn {
    pub index: usize,
    pub label: String,
    pub values: Vec<f64>,
    pub excluded_count: usize,
}

/// Extract active numeric columns from `Column` or `MultipleVariables` sheets.
pub fn extract_numeric_columns(sheet: &DataSheet, selected: &[usize]) -> Result<Vec<ExtractedColumn>, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col_tab) => {
            if col_tab.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = resolve_selected_indices(selected, col_tab.data_sets.len());
            let mut out = Vec::with_capacity(indices.len());
            for idx in indices {
                let ds = &col_tab.data_sets[idx];
                let mut values = Vec::new();
                let mut excluded_count = 0_usize;
                for subs in &ds.cells {
                    if let Some(cell) = subs.first() {
                        if cell.excluded {
                            excluded_count += 1;
                        } else if let Some(v) = cell.active_number() {
                            values.push(v);
                        }
                    }
                }
                out.push(ExtractedColumn { index: idx, label: ds.title.clone(), values, excluded_count });
            }
            Ok(out)
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices = resolve_selected_indices(selected, mv.variables.len());
            let mut out = Vec::with_capacity(indices.len());
            for idx in indices {
                let var = &mv.variables[idx];
                let mut values = Vec::new();
                let mut excluded_count = 0_usize;
                for cell in &var.cells {
                    if cell.excluded {
                        excluded_count += 1;
                    } else if let Some(v) = cell.active_number() {
                        values.push(v);
                    }
                }
                out.push(ExtractedColumn { index: idx, label: var.title.clone(), values, excluded_count });
            }
            Ok(out)
        }
        other => Err(StatsError::IncompatibleTable { analysis: "column extraction", actual: other.kind() }),
    }
}

/// Extract paired numeric values `(row_label, x, y)` across two columns of `Column` or `MultipleVariables`.
pub fn extract_paired_values(sheet: &DataSheet, col_a: usize, col_b: usize) -> Result<(Vec<f64>, Vec<f64>), StatsError> {
    let get_cell = |row: usize, col: usize| -> Option<&Cell> {
        match &sheet.archetype {
            TableArchetype::Column(t) => t.data_sets.get(col)?.cell(row, 0),
            TableArchetype::MultipleVariables(t) => t.variables.get(col)?.cells.get(row),
            _ => None,
        }
    };
    let rows = sheet.archetype.row_count();
    let cols = sheet.archetype.column_count();
    if col_a >= cols {
        return Err(StatsError::MissingColumn { index: col_a });
    }
    if col_b >= cols {
        return Err(StatsError::MissingColumn { index: col_b });
    }
    if let TableArchetype::Column(t) = &sheet.archetype
        && t.subcolumn_mode.is_pre_summarized()
    {
        return Err(StatsError::RequiresRawReplicates);
    }
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for r in 0..rows {
        if let (Some(a), Some(b)) = (get_cell(r, col_a).and_then(Cell::active_number), get_cell(r, col_b).and_then(Cell::active_number)) {
            xs.push(a);
            ys.push(b);
        }
    }
    if xs.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    Ok((xs, ys))
}

/// Build a simple summary `ResultTab`.
#[must_use]
pub fn build_tab(
    id: impl Into<String>,
    title: impl Into<String>,
    kind: ResultTabKind,
    column_headers: Vec<String>,
    row_labels: Vec<String>,
    rows: Vec<Vec<ResultCell>>,
) -> ResultTab {
    ResultTab { id: id.into(), title: title.into(), kind, column_headers, row_labels, rows, footnotes: Vec::new() }
}

/// Two-sided Student's `t` critical value `t_{1 - alpha/2, df}`.
pub fn t_crit_two_sided(confidence: ConfidenceLevel, df: f64) -> Result<f64, StatsError> {
    Ok(lumencraft_math::dist::student_t_two_sided_isf(confidence.alpha(), df)?)
}

/// Check if a subcolumn mode is pre-summarized with parametric moments (`MeanSdN`, `MeanSemN`, `MeanCvN`).
#[must_use]
pub const fn is_parametric_summary_mode(mode: SubcolumnMode) -> bool {
    mode.supports_parametric_moments()
}
