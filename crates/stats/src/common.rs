//! Shared numerical, ranking, quantile, validation, and `ResultSheet` construction helpers.

use curvebook_doc::{
    AnalysisSpec, Cell, ChecklistItem, ChecklistStatus, CiBound, DataSheet, PValueFormat, RemediationKind, ResultCell, ResultScalarEntry,
    ResultSheet, ResultStatus, ResultTab, ResultTabKind, SubcolumnMode, TableArchetype,
};
use curvebook_math::ConfidenceLevel;

use crate::error::StatsError;

/// Validate that `values` is non-empty and every entry is finite.
pub fn validate_finite_slice(values: &[f64]) -> Result<(), StatsError> {
    if values.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    for &v in values {
        if !v.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "x", value: v }));
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

/// Quartiles `(Q1, median, Q3)` by the type-6 sample-quantile definition of Hyndman & Fan (1996)
/// (`R` type 6: `m = p * (n + 1)` clamped to `[1, n]`). These are not Tukey's hinges: for example
/// `[1, 2, 3, 4, 5]` gives `Q1 = 1.5`, whereas the lower hinge is `2`.
///
/// Hyndman, R. J., & Fan, Y. (1996). Sample quantiles in statistical packages. *The American
/// Statistician*, 50(4), 361-365. `doi:10.1080/00031305.1996.10473566`.
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

/// `R` type-6 quantile (`(n + 1) * p` 1-based index with linear interpolation; Hyndman & Fan 1996).
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

/// Return `true` when an unexcluded, non-empty cell in a numeric column contains non-numeric
/// (e.g. `CellValue::Text`) or non-finite content and is ignored by numeric extraction.
#[must_use]
pub fn is_ignored_non_numeric_cell(cell: &Cell) -> bool {
    !cell.excluded && !cell.is_empty() && cell.active_number().is_none()
}

/// Extracted labeled numeric sample column from a `DataSheet`.
#[derive(Debug, Clone)]
pub struct ExtractedColumn {
    pub index: usize,
    pub label: String,
    pub values: Vec<f64>,
    pub excluded_count: usize,
    pub non_numeric_count: usize,
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
                let mut non_numeric_count = 0_usize;
                for subs in &ds.cells {
                    if let Some(cell) = subs.first() {
                        if cell.excluded {
                            excluded_count += 1;
                        } else if let Some(v) = cell.active_number() {
                            values.push(v);
                        } else if is_ignored_non_numeric_cell(cell) {
                            non_numeric_count += 1;
                        }
                    }
                }
                out.push(ExtractedColumn { index: idx, label: ds.title.clone(), values, excluded_count, non_numeric_count });
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
                let mut non_numeric_count = 0_usize;
                for cell in &var.cells {
                    if cell.excluded {
                        excluded_count += 1;
                    } else if let Some(v) = cell.active_number() {
                        values.push(v);
                    } else if is_ignored_non_numeric_cell(cell) {
                        non_numeric_count += 1;
                    }
                }
                out.push(ExtractedColumn { index: idx, label: var.title.clone(), values, excluded_count, non_numeric_count });
            }
            Ok(out)
        }
        other => Err(StatsError::IncompatibleTable { analysis: "column extraction", actual: other.kind() }),
    }
}

/// Extracted paired numeric series together with counts of omitted rows and ignored non-numeric cells.
#[derive(Debug, Clone, PartialEq)]
pub struct PairedValuesReport {
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub skipped_incomplete_rows: usize,
    pub non_numeric_cells: usize,
}

/// Extract paired numeric values `(x, y)` across two columns of `Column` or `MultipleVariables`,
/// reporting any incomplete/excluded rows and ignored non-numeric cells in the active range.
pub fn extract_paired_values_with_report(sheet: &DataSheet, col_a: usize, col_b: usize) -> Result<PairedValuesReport, StatsError> {
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
    let active_end = (0..rows).rfind(|&r| {
        get_cell(r, col_a).is_some_and(|c| !c.is_empty() || c.excluded) || get_cell(r, col_b).is_some_and(|c| !c.is_empty() || c.excluded)
    });
    let Some(end_row) = active_end else {
        return Err(StatsError::EmptyInput);
    };

    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut skipped_incomplete_rows = 0_usize;
    let mut non_numeric_cells = 0_usize;
    for r in 0..=end_row {
        let ca = get_cell(r, col_a);
        let cb = get_cell(r, col_b);
        if ca.is_some_and(is_ignored_non_numeric_cell) {
            non_numeric_cells += 1;
        }
        if cb.is_some_and(is_ignored_non_numeric_cell) {
            non_numeric_cells += 1;
        }
        let has_entry = ca.is_some_and(|c| !c.is_empty() || c.excluded) || cb.is_some_and(|c| !c.is_empty() || c.excluded);
        if let (Some(a), Some(b)) = (ca.and_then(Cell::active_number), cb.and_then(Cell::active_number)) {
            xs.push(a);
            ys.push(b);
        } else if has_entry {
            skipped_incomplete_rows += 1;
        }
    }
    if xs.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    Ok(PairedValuesReport { xs, ys, skipped_incomplete_rows, non_numeric_cells })
}

/// Extract paired numeric values `(row_label, x, y)` across two columns of `Column` or `MultipleVariables`.
pub fn extract_paired_values(sheet: &DataSheet, col_a: usize, col_b: usize) -> Result<(Vec<f64>, Vec<f64>), StatsError> {
    let report = extract_paired_values_with_report(sheet, col_a, col_b)?;
    Ok((report.xs, report.ys))
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
    Ok(curvebook_math::dist::student_t_two_sided_isf(confidence.alpha(), df)?)
}

/// Check if a subcolumn mode is pre-summarized with parametric moments (`MeanSdN`, `MeanSemN`, `MeanCvN`).
#[must_use]
pub const fn is_parametric_summary_mode(mode: SubcolumnMode) -> bool {
    mode.supports_parametric_moments()
}

/// Append a warning checklist entry and tab footnote when unexcluded non-numeric or non-finite
/// cells were ignored while reading numeric data columns (`R1-X2-08`).
pub fn append_ignored_text_checklist_and_footnote(sheet: &mut ResultSheet, context: &str, details: &[(String, usize)]) {
    let total: usize = details.iter().map(|(_, count)| *count).sum();
    if total == 0 {
        return;
    }
    let summary: String =
        details.iter().filter(|(_, count)| *count > 0).take(6).map(|(label, count)| format!("{label}: {count}")).collect::<Vec<_>>().join(", ");
    let suffix = if details.iter().filter(|(_, c)| *c > 0).count() > 6 { ", ..." } else { "" };
    let msg = format!("Non-numeric values ignored: {total} in {context} ({summary}{suffix}).");
    sheet.checklist.push(checklist_item(
        "Numeric input validation",
        ChecklistStatus::Warn,
        Some(format!("{total} non-numeric cell(s) ignored")),
        msg.clone(),
    ));
    if let Some(tab) = sheet.tabs.first_mut() {
        tab.footnotes.push(msg);
    }
}

/// Append an informational checklist entry and tab footnote when complete-case listwise row
/// alignment drops incomplete or excluded rows (`R1-S02-13`).
pub fn append_listwise_filtered_checklist_and_footnote(sheet: &mut ResultSheet, analysis: &str, skipped_rows: usize, kept_rows: usize) {
    if skipped_rows == 0 {
        return;
    }
    let msg = format!(
        "{analysis} uses complete-case listwise alignment and omitted {skipped_rows} incomplete or excluded row(s) ({kept_rows} complete row(s) retained)."
    );
    sheet.checklist.push(checklist_item(
        "Complete-case row alignment",
        ChecklistStatus::Info,
        Some(format!("{skipped_rows} row(s) omitted; {kept_rows} retained")),
        msg.clone(),
    ));
    if let Some(tab) = sheet.tabs.first_mut() {
        tab.footnotes.push(msg);
    }
}

/// Defense-in-depth sanitizer ensuring a produced `ResultSheet` never emits non-finite floats
/// into scalars, tabs, pairwise records, or curve overlays (`R1-X2-09`).
pub fn sanitize_result_sheet_floats(sheet: &mut ResultSheet) -> Result<(), StatsError> {
    sheet.scalars.retain(|s| s.value.is_finite());
    for tab in &mut sheet.tabs {
        for row in &mut tab.rows {
            for cell in row {
                match cell {
                    ResultCell::Number { value, .. } if !value.is_finite() => *cell = ResultCell::Empty,
                    ResultCell::PValue { raw, .. } if !raw.is_finite() => *cell = ResultCell::Empty,
                    ResultCell::ConfidenceInterval { lower, upper } => {
                        if let CiBound::Finite(v) = lower
                            && !v.is_finite()
                        {
                            *lower = CiBound::Open;
                        }
                        if let CiBound::Finite(v) = upper
                            && !v.is_finite()
                        {
                            *upper = CiBound::Open;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for rec in &mut sheet.pairwise_comparisons {
        if rec.estimate_diff.is_some_and(|v| !v.is_finite()) {
            rec.estimate_diff = None;
        }
        if !rec.raw_p_or_q.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "non_finite_pairwise_p", value: rec.raw_p_or_q }));
        }
    }
    for curve in &mut sheet.fitted_curves {
        for pt in &mut curve.points {
            if !pt.x.is_finite() || !pt.y.is_finite() {
                let bad = if !pt.x.is_finite() { pt.x } else { pt.y };
                return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "non_finite_curve_point", value: bad }));
            }
            if pt.ci_lower.is_some_and(|v| !v.is_finite()) {
                pt.ci_lower = None;
            }
            if pt.ci_upper.is_some_and(|v| !v.is_finite()) {
                pt.ci_upper = None;
            }
            if pt.pred_lower.is_some_and(|v| !v.is_finite()) {
                pt.pred_lower = None;
            }
            if pt.pred_upper.is_some_and(|v| !v.is_finite()) {
                pt.pred_upper = None;
            }
        }
    }
    Ok(())
}

fn weighted_median_pivot(candidates: &mut [(f64, usize)], total_weight: u64) -> Option<f64> {
    if candidates.is_empty() || total_weight == 0 {
        return None;
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let target = total_weight.div_ceil(2);
    let mut acc = 0_u64;
    for &(val, w) in candidates.iter() {
        acc += w as u64;
        if acc >= target {
            return Some(val);
        }
    }
    candidates.last().map(|&(val, _)| val)
}

struct MonotoneMatrixState {
    n_rows: usize,
    n_cols: usize,
    left: Vec<usize>,
    right: Vec<usize>,
    cut_lt: Vec<usize>,
    cut_le: Vec<usize>,
    rem: u64,
}

impl MonotoneMatrixState {
    fn new(init_left: &[usize], n_cols: usize) -> Self {
        let n_rows = init_left.len();
        let right = vec![n_cols; n_rows];
        let rem = (0..n_rows).map(|i| (n_cols.saturating_sub(init_left[i])) as u64).sum();
        Self { n_rows, n_cols, left: init_left.to_vec(), right, cut_lt: vec![0; n_rows], cut_le: vec![0; n_rows], rem }
    }

    fn collect_pivots<F: Fn(usize, usize) -> f64>(&self, eval: &F, buf: &mut Vec<(f64, usize)>) {
        buf.clear();
        for i in 0..self.n_rows {
            let l = self.left[i];
            let r = self.right[i];
            if l < r {
                let mid = l + (r - l) / 2;
                buf.push((eval(i, mid), r - l));
            }
        }
    }

    fn partition_counts<F: Fn(usize, usize) -> f64>(&mut self, pivot: f64, eval: &F) -> (u64, u64) {
        let mut j_lt = self.n_cols;
        let mut j_le = self.n_cols;
        let mut count_lt = 0_u64;
        let mut count_le = 0_u64;
        for i in 0..self.n_rows {
            let l = self.left[i];
            let r = self.right[i];
            while j_lt > 0 && eval(i, j_lt - 1) >= pivot {
                j_lt -= 1;
            }
            let clt = j_lt.clamp(l, r);
            self.cut_lt[i] = clt;
            count_lt += (clt - l) as u64;

            while j_le > 0 && eval(i, j_le - 1) > pivot {
                j_le -= 1;
            }
            let cle = j_le.clamp(l, r);
            self.cut_le[i] = cle;
            count_le += (cle - l) as u64;
        }
        (count_lt, count_le)
    }

    fn collect_tail<F: Fn(usize, usize) -> f64>(&self, eval: &F) -> Vec<f64> {
        let mut tail = Vec::with_capacity(self.rem as usize);
        for i in 0..self.n_rows {
            for j in self.left[i]..self.right[i] {
                tail.push(eval(i, j));
            }
        }
        tail
    }
}

fn select_kth_monotone_matrix<F: Fn(usize, usize) -> f64>(init_left: &[usize], n_cols: usize, k_target: u64, eval: F) -> Result<f64, StatsError> {
    let mut state = MonotoneMatrixState::new(init_left, n_cols);
    if k_target >= state.rem {
        return Err(StatsError::InvalidParameter { name: "k_target", value: k_target as f64 });
    }
    let mut k_rem = k_target;
    let mut pivots = Vec::with_capacity(state.n_rows);
    while state.rem > 256 {
        state.collect_pivots(&eval, &mut pivots);
        let pivot = weighted_median_pivot(&mut pivots, state.rem).ok_or(StatsError::EmptyInput)?;
        let (count_lt, count_le) = state.partition_counts(pivot, &eval);
        if k_rem < count_lt {
            state.right.copy_from_slice(&state.cut_lt);
            state.rem = count_lt;
        } else if k_rem < count_le {
            return Ok(pivot);
        } else {
            k_rem -= count_le;
            state.left.copy_from_slice(&state.cut_le);
            state.rem -= count_le;
        }
    }
    let mut tail = state.collect_tail(&eval);
    let idx = usize::try_from(k_rem).map_err(|_| StatsError::InvalidParameter { name: "k_target", value: k_rem as f64 })?;
    let (_, nth, _) = tail.select_nth_unstable_by(idx, f64::total_cmp);
    Ok(*nth)
}

#[inline]
fn normalize_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

/// Exact `k`-th smallest (`0`-based) pairwise sum `sorted_xs[i] + sorted_ys[j]` across all pairs
/// in `O((n + m) log(n + m))` without materializing `O(n * m)` pairs. Both inputs must be sorted ascending and finite.
pub fn select_kth_pairwise_sum(sorted_xs: &[f64], sorted_ys: &[f64], k: u64) -> Result<f64, StatsError> {
    validate_finite_slice(sorted_xs)?;
    validate_finite_slice(sorted_ys)?;
    let max_x = sorted_xs.first().map_or(0.0, |v| v.abs()).max(sorted_xs.last().map_or(0.0, |v| v.abs()));
    let max_y = sorted_ys.first().map_or(0.0, |v| v.abs()).max(sorted_ys.last().map_or(0.0, |v| v.abs()));
    let scale_half = max_x >= 1e300 || max_y >= 1e300;
    let init_left = vec![0_usize; sorted_xs.len()];
    let raw = select_kth_monotone_matrix(&init_left, sorted_ys.len(), k, |i, j| {
        let sum = if scale_half { 0.5 * sorted_xs[i] + 0.5 * sorted_ys[j] } else { sorted_xs[i] + sorted_ys[j] };
        normalize_zero(sum)
    })?;
    let out = if scale_half { 2.0 * raw } else { raw };
    if !out.is_finite() {
        return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "pairwise_sum_overflow", value: out }));
    }
    Ok(out)
}

/// Exact `k`-th smallest (`0`-based) upper-triangular Walsh average `0.5 * (sorted_x[i] + sorted_x[j])`
/// for `0 <= i <= j < n` in `O(n log n)` without materializing `O(n^2)` pairs. Input must be sorted ascending and finite.
pub fn select_kth_upper_triangle_half_sum(sorted_x: &[f64], k: u64) -> Result<f64, StatsError> {
    validate_finite_slice(sorted_x)?;
    let n = sorted_x.len();
    let init_left: Vec<usize> = (0..n).collect();
    select_kth_monotone_matrix(&init_left, n, k, |i, j| normalize_zero(0.5 * sorted_x[i] + 0.5 * sorted_x[j]))
}
