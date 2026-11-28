//! Shared numerical, ranking, quantile, validation, and `ResultSheet` construction helpers.
//!
//! References:
//! - Hyndman, R. J., & Fan, Y. (1996). Sample quantiles in statistical packages. *The American
//!   Statistician*, 50(4), 361-365. `doi:10.1080/00031305.1996.10473566`.
//! - Johnson, D. B., & Mizoguchi, T. (1978). Selecting the *K*th element in *X* + *Y* and
//!   *X*1 + *X*2 + ... + *X*m. *SIAM Journal on Computing*, 7(2), 147-153. `doi:10.1137/0207013`.
//! - Kruskal, W. H., & Wallis, W. A. (1952). Use of ranks in one-criterion variance analysis.
//!   *Journal of the American Statistical Association*, 47(260), 583-621.
//!   `doi:10.1080/01621459.1952.10483441`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistItem, ChecklistStatus, CiBound, DataSheet, ObservationAccounting, ObservationAccountingKind,
    PairedObservationAccounting, RemediationKind, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SubcolumnMode,
    SummaryModeRefusalReason, SummaryModeSupport, TableArchetype, VariableRole, summary_mode_row_for_spec,
};
pub use curvebook_doc::{
    KvTabBuilder, TabBuilder, bool_cell, ci_cell, finite_ci_cell, num_cell, opt_ci_cell, opt_num_cell, p_cell, p_cell_at_alpha, text_cell,
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
///
/// This is Definition 7 of Hyndman & Fan (1996): linear interpolation between the points
/// `(p_k, x_(k))` with `p_k = (k - 1) / (n - 1)`, that is, the 0-based position `p * (n - 1)`.
#[must_use]
pub fn quantile_sorted(sorted: &[f64], p: f64) -> Option<f64> {
    curvebook_math::quantile_type7_sorted(sorted, p)
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
    /// One-based average mid-ranks in `[1.0, n]` aligned 1-to-1 with the input slice.
    pub ranks: Vec<f64>,
    /// Multiplicity count `t_k >= 2` for each distinct tied value group (empty when all values are unique).
    pub tie_counts: Vec<usize>,
    /// Tie correction numerator `sum_k (t_k^3 - t_k)` used in Spearman, Mann–Whitney, Kruskal–Wallis, and Friedman adjustments.
    pub tie_sum_t3_minus_t: f64,
}

/// Assign 1-based average ranks in `O(n log n)` with exact tie group accounting.
///
/// Tied values share the mean of the ranks they span (mid-ranks). The returned
/// `sum_k (t_k^3 - t_k)` over tie groups of size `t_k` is the quantity in the tie corrections of
/// rank tests such as Kruskal & Wallis (1952), `1 - sum (t^3 - t) / (N^3 - N)`.
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

/// Resolve selected dataset column indices (`empty` in `selected` means all `0..total`),
/// rejecting out-of-range indices with [`StatsError::MissingColumn`] and duplicate indices with
/// [`StatsError::InvalidParameter`].
pub fn try_resolve_selected_indices(selected: &[usize], total: usize) -> Result<Vec<usize>, StatsError> {
    if selected.is_empty() {
        return Ok((0..total).collect());
    }
    let mut resolved = Vec::with_capacity(selected.len());
    for &idx in selected {
        if idx >= total {
            return Err(StatsError::MissingColumn { index: idx });
        }
        if resolved.contains(&idx) {
            return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: idx as f64 });
        }
        resolved.push(idx);
    }
    Ok(resolved)
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
    /// Zero-based originating dataset or variable column index in the source table.
    pub index: usize,
    /// Column title string from the source table.
    pub label: String,
    /// Active included finite numeric observations extracted from the column.
    pub values: Vec<f64>,
    /// Number of excluded cells skipped during extraction.
    pub excluded_count: usize,
    /// Number of non-numeric text cells ignored during extraction.
    pub non_numeric_count: usize,
}

/// Extract active numeric columns from `Column` or `MultipleVariables` sheets.
///
/// When `selected` is empty on a `MultipleVariables` sheet, columns with [`VariableRole::Categorical`]
/// or [`VariableRole::Label`] are excluded from the default numeric selection.
pub fn extract_numeric_columns(sheet: &DataSheet, selected: &[usize]) -> Result<Vec<ExtractedColumn>, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col_tab) => {
            if col_tab.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = try_resolve_selected_indices(selected, col_tab.data_sets.len())?;
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
            let mut indices = try_resolve_selected_indices(selected, mv.variables.len())?;
            if selected.is_empty() {
                indices.retain(|&idx| !matches!(mv.variables[idx].role, VariableRole::Categorical | VariableRole::Label));
            }
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
    /// Active complete finite numeric values for column/series `X` (`a`).
    pub xs: Vec<f64>,
    /// Matching active complete finite numeric values for column/series `Y` (`b`), aligned 1-to-1 with `xs`.
    pub ys: Vec<f64>,
    /// Number of rows omitted because either member of the pair was empty, excluded, or non-finite.
    pub skipped_incomplete_rows: usize,
    /// Number of non-numeric text cells encountered inside examined pair slots.
    pub non_numeric_cells: usize,
    /// Number of excluded cells encountered inside examined pair slots.
    pub excluded_cells: usize,
    /// Pairs analyzed and omitted rows broken down by reason; the omitted counts sum to `skipped_incomplete_rows`.
    pub accounting: PairedObservationAccounting,
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
    let mut excluded_cells = 0_usize;
    let mut accounting = PairedObservationAccounting::default();
    for r in 0..=end_row {
        let ca = get_cell(r, col_a);
        let cb = get_cell(r, col_b);
        if ca.is_some_and(|c| c.excluded) {
            excluded_cells += 1;
        }
        if cb.is_some_and(|c| c.excluded) {
            excluded_cells += 1;
        }
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
            record_omitted_pair(&mut accounting, ca, cb);
        }
    }
    if xs.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    accounting.pairs_analyzed = xs.len();
    Ok(PairedValuesReport { xs, ys, skipped_incomplete_rows, non_numeric_cells, excluded_cells, accounting })
}

/// Classify one omitted row or observation under its canonical mutually exclusive reason:
/// excluded cell first, then non-numeric cell, then domain-invalid value, then missing value.
pub fn record_omitted_row_reason(accounting: &mut ObservationAccounting, has_excluded: bool, has_non_numeric: bool, has_invalid: bool) {
    if has_excluded {
        accounting.omitted_excluded = accounting.omitted_excluded.saturating_add(1);
    } else if has_non_numeric {
        accounting.omitted_non_numeric = accounting.omitted_non_numeric.saturating_add(1);
    } else if has_invalid {
        accounting.omitted_invalid = accounting.omitted_invalid.saturating_add(1);
    } else {
        accounting.omitted_missing = accounting.omitted_missing.saturating_add(1);
    }
}

/// Build an unpaired observation accounting record tracking active observations, ignored
/// non-numeric cells, and excluded cells across the analyzed column scope.
#[must_use]
pub const fn unpaired_observation_accounting(analyzed: usize, ignored_non_numeric_cells: usize, excluded_cells: usize) -> ObservationAccounting {
    ObservationAccounting { ignored_non_numeric_cells, excluded_cells, ..ObservationAccounting::new(ObservationAccountingKind::Unpaired, analyzed) }
}

/// Count one omitted row of a pair under its reason, in the order documented on
/// [`PairedObservationAccounting`]: an excluded cell, then a non-numeric cell, then an empty partner.
pub fn record_omitted_pair(accounting: &mut PairedObservationAccounting, a: Option<&Cell>, b: Option<&Cell>) {
    let pair = [a, b];
    if pair.iter().any(|c| c.is_some_and(|c| c.excluded)) {
        accounting.omitted_excluded = accounting.omitted_excluded.saturating_add(1);
    } else if pair.iter().any(|c| c.is_some_and(is_ignored_non_numeric_cell)) {
        accounting.omitted_non_numeric = accounting.omitted_non_numeric.saturating_add(1);
    } else {
        accounting.omitted_missing_partner = accounting.omitted_missing_partner.saturating_add(1);
    }
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

/// Enforce the canonical `curvebook-doc` summary-mode support matrix ([`curvebook_doc::SUMMARY_MODE_MATRIX`])
/// before running any statistical procedure on a `Column`, `Xy`, or `Grouped` [`DataSheet`].
///
/// Leaves archetype mismatches (`!kind.is_compatible_with(table_kind)`) to return
/// [`StatsError::IncompatibleTable`] downstream, and returns the matrix-prescribed refusal
/// ([`StatsError::RequiresRawReplicates`] or [`StatsError::UnsupportedSubcolumnMode`]) for every
/// refused pre-summarized cell.
pub fn enforce_summary_mode_matrix(sheet: &DataSheet, kind: &AnalysisKind) -> Result<(), StatsError> {
    let table_kind = sheet.archetype.kind();
    if !kind.is_compatible_with(table_kind) {
        return Ok(());
    }
    let Some(mode) = sheet.archetype.primary_subcolumn_mode() else {
        return Ok(());
    };
    if !mode.is_pre_summarized() {
        return Ok(());
    }
    let Some(row) = summary_mode_row_for_spec(kind, table_kind) else {
        return Ok(());
    };
    let Some(verdict) = row.support_for(mode) else {
        return Ok(());
    };
    match verdict {
        SummaryModeSupport::Supported(_) => Ok(()),
        SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates) => Err(StatsError::RequiresRawReplicates),
        SummaryModeSupport::Refused(reason) => {
            Err(StatsError::UnsupportedSubcolumnMode { analysis: row.label, table_kind, mode, reason: reason.message() })
        }
    }
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
        let prev_rem = state.rem;
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
        if state.rem >= prev_rem {
            return Err(StatsError::NotConverged { algorithm: "select_kth_monotone_matrix" });
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

fn validate_sorted_ascending(name: &'static str, values: &[f64]) -> Result<(), StatsError> {
    if let Some(bad) = values.windows(2).find(|w| w[0].total_cmp(&w[1]).is_gt()) {
        return Err(StatsError::InvalidParameter { name, value: bad[1] });
    }
    Ok(())
}

/// Exact `k`-th smallest (`0`-based) pairwise sum `sorted_xs[i] + sorted_ys[j]` across all pairs
/// in `O((n + m) log(n + m))` without materializing `O(n * m)` pairs. Both inputs must be sorted ascending and finite.
///
/// Algorithm KTHPAIR of Johnson & Mizoguchi (1978): treat `x_i + y_j` as a matrix whose rows and
/// columns are sorted, take the middle element of each row's remaining range weighted by that
/// range's length, partition every row at the weighted median of those elements, and discard the
/// part that cannot contain the `k`-th element. This differs from the paper in two details:
/// equal values are split into `<`, `=` and `>` counts so the pivot is returned as soon as it is
/// the answer, and the remaining candidates are selected directly once at most 256 remain.
pub fn select_kth_pairwise_sum(sorted_xs: &[f64], sorted_ys: &[f64], k: u64) -> Result<f64, StatsError> {
    validate_finite_slice(sorted_xs)?;
    validate_finite_slice(sorted_ys)?;
    validate_sorted_ascending("sorted_xs", sorted_xs)?;
    validate_sorted_ascending("sorted_ys", sorted_ys)?;
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
///
/// This is the `X + X` case of Johnson & Mizoguchi (1978, Corollary to Theorem 1), with pairs
/// restricted to `i <= j` (each row starts at its diagonal). The selection itself is the same as
/// in [`select_kth_pairwise_sum`].
pub fn select_kth_upper_triangle_half_sum(sorted_x: &[f64], k: u64) -> Result<f64, StatsError> {
    validate_finite_slice(sorted_x)?;
    validate_sorted_ascending("sorted_x", sorted_x)?;
    let n = sorted_x.len();
    let init_left: Vec<usize> = (0..n).collect();
    select_kth_monotone_matrix(&init_left, n, k, |i, j| normalize_zero(0.5 * sorted_x[i] + 0.5 * sorted_x[j]))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{CiBound, ResultCell};

    use super::{ci_cell, finite_ci_cell};

    #[test]
    fn finite_ci_cell_distinguishes_missing_nan_from_open_infinity() {
        assert_eq!(finite_ci_cell(1.0, 2.0), ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.0), upper: CiBound::Finite(2.0) });
        assert_eq!(finite_ci_cell(f64::NEG_INFINITY, 2.0), ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(2.0) });
        assert_eq!(finite_ci_cell(1.0, f64::INFINITY), ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.0), upper: CiBound::Open });
        assert_eq!(finite_ci_cell(f64::NAN, 2.0), ResultCell::Empty);
        assert_eq!(finite_ci_cell(1.0, f64::NAN), ResultCell::Empty);
        // Existing `ci_cell` keeps mapping non-finite (including NaN) to `CiBound::Open`.
        assert_eq!(ci_cell(f64::NAN, 2.0), ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(2.0) });
    }
}
