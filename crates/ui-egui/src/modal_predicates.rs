//! Declarative modal conditional enablement & graying predicates (`R1`–`R10`, `docs/gui-workflows.md` §3.2).
//!
//! Single source of truth consumed by live egui dialog widgets (`.add_enabled` + disabled tooltips),
//! `ui.inspect` (`ControlNode { enabled, disabled_reason }`), `ui.switch_tab`, and `ui.answer_dialog`.

use curvebook_doc::{
    AnalysisKind, Cell, CellValue, ColumnDesignHint, ContingencySpec, MultiplicityMethod, OneWayMethod, PValueFormat, ProjectDocument,
    RepeatedMeasuresDesign, ResidualCovariance, SheetId, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod, VariableRole,
};
use serde::{Deserialize, Serialize};

use crate::dialogs_canvas::{ActiveDialog, AnalyzeDialogState, CanvasDialogState, EditAnalysisParametersState, PairwiseComparisonsState};
use crate::power_wizard::PowerWizardStage;

/// Conditional enablement result attached to modal controls, tabs, and action buttons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlEnablement {
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

impl ControlEnablement {
    /// Construct an enabled predicate state.
    #[must_use]
    pub const fn enabled() -> Self {
        Self { enabled: true, disabled_reason: None }
    }

    /// Construct a disabled/grayed predicate state with a non-empty user-facing rationale.
    #[must_use]
    pub fn disabled(reason: impl Into<String>) -> Self {
        Self { enabled: false, disabled_reason: Some(reason.into()) }
    }
}

// ============================================================================
// Rules R1–R10 Predicates (`docs/gui-workflows.md` §3.2)
// ============================================================================

/// R1: ANOVA pairing -> `Repeated Measures` tab enablement.
#[must_use]
pub fn r1_repeated_measures_tab(paired_or_matched: bool) -> ControlEnablement {
    if paired_or_matched {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Repeated Measures tab requires a matched or repeated-measures design.")
    }
}

/// R1b: Sphericity (`Geisser-Greenhouse`) and `Mixed-effects` REML controls require matched/repeated ANOVA.
#[must_use]
pub fn r1_sphericity_or_reml_control(paired_or_matched: bool) -> ControlEnablement {
    if paired_or_matched {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Geisser-Greenhouse sphericity and Mixed-effects REML require matched/repeated measures.")
    }
}

/// R1b: Sphericity corrections (`Geisser-Greenhouse` / `Assume sphericity`) require matched ANOVA and
/// are disabled whenever Mixed-Effects REML (`Kenward-Roger` / `Satterthwaite`) is active.
#[must_use]
pub fn r1b_sphericity_correction(paired_or_matched: bool, reml_active: bool) -> ControlEnablement {
    if !paired_or_matched {
        return r1_sphericity_or_reml_control(false);
    }
    if reml_active {
        ControlEnablement::disabled("Sphericity corrections are disabled when Mixed-Effects REML (Kenward-Roger / Satterthwaite) is active.")
    } else {
        ControlEnablement::enabled()
    }
}

/// Whether a multiplicity adjustment is supported under Mixed-Effects REML (`KenwardRoger` / `Satterthwaite`):
/// Bonferroni, Sidak, Holm-Sidak, FDR (BH/BKY/BY), Fisher's LSD, or None.
#[must_use]
pub fn r1b_is_reml_allowed_post_hoc(method: MultiplicityMethod) -> bool {
    matches!(
        method,
        MultiplicityMethod::None
            | MultiplicityMethod::Bonferroni
            | MultiplicityMethod::Sidak
            | MultiplicityMethod::HolmSidak
            | MultiplicityMethod::BenjaminiHochbergFdr
            | MultiplicityMethod::TwoStageBkyFdr
            | MultiplicityMethod::BenjaminiYekutieliFdr
            | MultiplicityMethod::FisherLsd
    )
}

/// R1b + R2 combined post-hoc option enablement gate.
#[must_use]
pub fn r1b_post_hoc_option_enabled(assume_gaussian: bool, reml_active: bool, method: MultiplicityMethod) -> ControlEnablement {
    if matches!(method, MultiplicityMethod::DunnRankSum) {
        if reml_active {
            return ControlEnablement::disabled("Nonparametric Dunn's comparisons are disabled in Mixed-Effects REML mode.");
        }
        return r2_post_hoc_option_enabled(assume_gaussian, false);
    }
    if matches!(
        method,
        MultiplicityMethod::None
            | MultiplicityMethod::BenjaminiHochbergFdr
            | MultiplicityMethod::TwoStageBkyFdr
            | MultiplicityMethod::BenjaminiYekutieliFdr
    ) {
        return ControlEnablement::enabled();
    }
    let gaussian_gate = r2_post_hoc_option_enabled(assume_gaussian, true);
    if !gaussian_gate.enabled {
        return gaussian_gate;
    }
    if reml_active && !r1b_is_reml_allowed_post_hoc(method) {
        return ControlEnablement::disabled(
            "Mixed-Effects REML (Kenward-Roger / Satterthwaite) restricts post-hoc comparisons to Bonferroni, Sidak, Holm-Sidak, FDR, and Fisher's LSD.",
        );
    }
    ControlEnablement::enabled()
}

/// Inspect the upstream sheet for missing observations in repeated-measures blocks (`Column` or `Grouped`).
#[must_use]
pub fn table_has_incomplete_rm_blocks(doc: &ProjectDocument, source_sheet_id: Option<&SheetId>, selected: &[usize]) -> bool {
    let Some(ds) = resolve_data_sheet(doc, source_sheet_id) else {
        return false;
    };
    match &ds.archetype {
        TableArchetype::Column(col) => {
            let indices: Vec<usize> = if selected.is_empty() {
                (0..col.data_sets.len()).collect()
            } else {
                selected.iter().copied().filter(|&i| i < col.data_sets.len()).collect()
            };
            col.complete_paired_rows_with_report(&indices).skipped_incomplete_rows > 0
        }
        TableArchetype::Grouped(grouped) => {
            let SubcolumnMode::RawReplicates(n_subs) = grouped.subcolumn_mode else {
                return false;
            };
            let indices: Vec<usize> = if selected.is_empty() {
                (0..grouped.data_sets.len()).collect()
            } else {
                selected.iter().copied().filter(|&i| i < grouped.data_sets.len()).collect()
            };
            (0..grouped.row_count())
                .any(|r| indices.iter().any(|&c| (0..n_subs).any(|s| grouped.observation(r, c, s).and_then(Cell::active_number).is_none())))
        }
        _ => false,
    }
}

/// Whether `edit` currently executes a Mixed-Effects REML repeated-measures model (`Kenward-Roger` / `Satterthwaite`).
#[must_use]
pub fn is_reml_mixed_effects_active(edit: &EditAnalysisParametersState, doc: &ProjectDocument) -> bool {
    let Some(spec) = edit.draft.as_ref() else {
        return false;
    };
    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => {
            matches!(a.method, OneWayMethod::RepeatedMeasures { .. })
                && (a.mixed_effects_reml
                    || a.residual_covariance == ResidualCovariance::Unstructured
                    || table_has_incomplete_rm_blocks(doc, Some(&spec.source_sheet_id), &spec.selected_data_sets))
        }
        AnalysisKind::FactorialAnova(f) => {
            f.repeated_measures != RepeatedMeasuresDesign::None
                && (f.mixed_effects_reml
                    || f.residual_covariance == ResidualCovariance::Unstructured
                    || f.three_way
                    || (f.fit_reml_on_missing && table_has_incomplete_rm_blocks(doc, Some(&spec.source_sheet_id), &spec.selected_data_sets)))
        }
        _ => false,
    }
}

/// R2: Gaussian toggle -> Parametric post-hoc methods vs nonparametric `Dunn's` rank comparisons.
#[must_use]
pub fn r2_parametric_post_hoc_method(assume_gaussian: bool, method: MultiplicityMethod) -> ControlEnablement {
    let is_parametric_posthoc = !matches!(method, MultiplicityMethod::None | MultiplicityMethod::DunnRankSum);
    if !assume_gaussian && is_parametric_posthoc {
        ControlEnablement::disabled("Parametric post-hoc tests are disabled when Assume Gaussian is No; use Dunn's rank comparisons.")
    } else {
        ControlEnablement::enabled()
    }
}

/// R2b: Individual parametric post-hoc option enablement given `assume_gaussian`.
#[must_use]
pub fn r2_post_hoc_option_enabled(assume_gaussian: bool, option_is_parametric: bool) -> ControlEnablement {
    if !assume_gaussian && option_is_parametric {
        ControlEnablement::disabled("Requires Assume Gaussian = Yes; nonparametric mode uses Dunn's comparisons.")
    } else if assume_gaussian && !option_is_parametric {
        ControlEnablement::disabled("Dunn's rank comparisons are only active when Assume Gaussian = No.")
    } else {
        ControlEnablement::enabled()
    }
}

/// Apply Rule R2 automatically when toggling `assume_gaussian` on a draft [`AnalysisKind`],
/// preserving compatible sub-methods (`WelchT`, `RatioPairedT`, `LognormalUnpairedT`, `KolmogorovSmirnov`,
/// `PairedSignTest`, `BrownForsytheAndWelch`, `RepeatedMeasures { geisser_greenhouse }`) and REML settings (`RV-78`).
pub fn apply_r2_gaussian_toggle(kind: &mut AnalysisKind, assume_gaussian: bool, matched: bool, control_col: usize) {
    match kind {
        AnalysisKind::OneWayAnova(spec) => {
            if assume_gaussian {
                if matched {
                    if !matches!(spec.method, curvebook_doc::OneWayMethod::RepeatedMeasures { .. }) {
                        spec.method = curvebook_doc::OneWayMethod::RepeatedMeasures { geisser_greenhouse: true };
                    }
                } else if !matches!(spec.method, curvebook_doc::OneWayMethod::OrdinaryAnova | curvebook_doc::OneWayMethod::BrownForsytheAndWelch) {
                    spec.method = curvebook_doc::OneWayMethod::OrdinaryAnova;
                }
                if spec.multiplicity.method == MultiplicityMethod::DunnRankSum {
                    spec.multiplicity.method = MultiplicityMethod::TukeyHsd;
                }
            } else {
                spec.method = if matched { curvebook_doc::OneWayMethod::Friedman } else { curvebook_doc::OneWayMethod::KruskalWallis };
                spec.multiplicity.method = MultiplicityMethod::DunnRankSum;
            }
        }
        AnalysisKind::TwoGroupComparison(spec) => {
            spec.method = match spec.method {
                TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
                    if assume_gaussian {
                        TwoGroupMethod::OneSampleT { hypothetical }
                    } else {
                        TwoGroupMethod::OneSampleWilcoxon { hypothetical }
                    }
                }
                TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::LognormalUnpairedT if assume_gaussian && !matched => spec.method,
                TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT if assume_gaussian && matched => spec.method,
                TwoGroupMethod::MannWhitneyU | TwoGroupMethod::KolmogorovSmirnov if !assume_gaussian && !matched => spec.method,
                TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest if !assume_gaussian && matched => spec.method,
                _ => match (assume_gaussian, matched) {
                    (true, false) => TwoGroupMethod::UnpairedT,
                    (true, true) => TwoGroupMethod::PairedT,
                    (false, false) => TwoGroupMethod::MannWhitneyU,
                    (false, true) => TwoGroupMethod::WilcoxonSignedRank,
                },
            };
        }
        _ => {
            let _ = control_col;
        }
    }
}

/// R3: Control-group contrast -> control-column picker enablement (`Dunnett` recommended).
#[must_use]
pub fn r3_control_column_picker(compare_against_control: bool, method: MultiplicityMethod) -> ControlEnablement {
    if compare_against_control || matches!(method, MultiplicityMethod::Dunnett { .. } | MultiplicityMethod::DunnettT3) {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Control-column picker is only active when comparing against a control group (Dunnett).")
    }
}

/// R4a: Two-group test mode -> `Graph differences` (paired designs only).
#[must_use]
pub fn r4_two_group_graph_differences(method: TwoGroupMethod) -> ControlEnablement {
    let is_paired = matches!(
        method,
        TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest
    );
    if is_paired {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Graph differences requires a paired or matched-pairs two-group test.")
    }
}

/// R4b: Two-group test mode -> `Graph ranks` & `Hodges-Lehmann CI` (nonparametric tests only).
#[must_use]
pub fn r4_two_group_nonparametric_options(method: TwoGroupMethod) -> ControlEnablement {
    let is_nonparametric = matches!(
        method,
        TwoGroupMethod::MannWhitneyU
            | TwoGroupMethod::WilcoxonSignedRank
            | TwoGroupMethod::KolmogorovSmirnov
            | TwoGroupMethod::OneSampleWilcoxon { .. }
            | TwoGroupMethod::PairedSignTest
    );
    if is_nonparametric {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Graph ranks and Hodges-Lehmann CI are only available for nonparametric rank tests.")
    }
}

/// R4c: Two-group test mode -> `Welch's correction` (unpaired parametric tests only).
#[must_use]
pub fn r4_two_group_welch_correction(method: TwoGroupMethod) -> ControlEnablement {
    let allows_welch = matches!(method, TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::LognormalUnpairedT);
    if allows_welch {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Welch's unequal-variance correction applies only to unpaired parametric t-tests.")
    }
}

/// R4d: Two-group test mode -> `Diagnostics / Residuals` disabled under `TwoGroupMethod::WelchT`.
#[must_use]
pub fn r4_two_group_diagnostics(method: TwoGroupMethod) -> ControlEnablement {
    if method == TwoGroupMethod::WelchT {
        ControlEnablement::disabled("Diagnostics / Residuals are disabled under Welch's unequal-variance t-test.")
    } else {
        ControlEnablement::enabled()
    }
}

/// R5a: Contingency table dimensions -> 2x2-specific clinical effect sizes
/// (`Odds ratio`, `Relative risk`, `Attributable risk`, `Sensitivity/Specificity`).
#[must_use]
pub fn r5_contingency_2x2_effect_sizes(rows: usize, cols: usize) -> ControlEnablement {
    if rows == 2 && cols == 2 {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled(format!(
            "2x2-specific effect sizes (OR, RR, Attributable Risk, Sensitivity/Specificity) are disabled for {rows}x{cols} tables."
        ))
    }
}

/// R5b: Contingency table dimensions -> `Cochran-Armitage trend test` (`R >= 3 x 2` or `2 x C >= 3`).
#[must_use]
pub fn r5_contingency_cochran_armitage_trend(rows: usize, cols: usize) -> ControlEnablement {
    if (rows >= 3 && cols == 2) || (rows == 2 && cols >= 3) {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled(format!(
            "Cochran-Armitage trend test requires an ordered Rx2 (R >= 3) or 2xC (C >= 3) table (got {rows}x{cols})."
        ))
    }
}

/// R6: Contingency effect-size checkbox -> CI method selectors
/// (`Baptista-Pike`/`Woolf`, `Koopman`, `Newcombe-Wilson`, `Wilson-Brown`).
#[must_use]
pub fn r6_contingency_ci_selector(rows: usize, cols: usize, effect_checkbox_checked: bool, ci_family_label: &str) -> ControlEnablement {
    let dim_gate = r5_contingency_2x2_effect_sizes(rows, cols);
    if !dim_gate.enabled {
        return dim_gate;
    }
    if effect_checkbox_checked {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled(format!("Check the corresponding 2x2 effect-size box before selecting `{ci_family_label}` CI."))
    }
}

/// R7: Correlation mode -> `Create correlation heatmap` toggle (`Correlation matrix` only).
#[must_use]
pub fn r7_correlation_heatmap_toggle(all_pairs_matrix: bool) -> ControlEnablement {
    if all_pairs_matrix {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Correlation heatmap requires all-pairs Correlation matrix mode (disabled for X vs. every Y).")
    }
}

/// R8: Single-subcolumn table -> Replicate-subcolumn aggregation radios (`Normality` / `Descriptive`).
#[must_use]
pub fn r8_subcolumn_aggregation(max_subcolumns: usize) -> ControlEnablement {
    if max_subcolumns > 1 {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Replicate-subcolumn aggregation is disabled because the upstream table has no replicate subcolumns.")
    }
}

/// R9: P-value formatting style -> custom precision `[N]` digits spinner.
#[must_use]
pub fn r9_pvalue_precision_spinner(format: &PValueFormat) -> ControlEnablement {
    if matches!(format, PValueFormat::Custom { .. }) {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Custom decimal-digit precision spinner is disabled unless Custom P-value format is selected.")
    }
}

/// Count survival cohorts on `Survival` (cohort count) or distinct active levels in the MV grouping column.
#[must_use]
pub fn survival_group_count(doc: &ProjectDocument, source_sheet_id: Option<&SheetId>, mv_group_col: Option<usize>) -> usize {
    let Some(ds) = resolve_data_sheet(doc, source_sheet_id) else {
        return 2;
    };
    match &ds.archetype {
        TableArchetype::Survival(s) => s.cohorts.len(),
        TableArchetype::MultipleVariables(mv) => {
            let col = mv_group_col.or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Categorical));
            let Some(var) = col.and_then(|c| mv.variables.get(c)) else {
                return mv.variables.len();
            };
            let mut levels = std::collections::BTreeSet::new();
            for cell in &var.cells {
                if cell.excluded {
                    continue;
                }
                match &cell.value {
                    CellValue::Text(s) if !s.trim().is_empty() => {
                        levels.insert(s.trim().to_owned());
                    }
                    CellValue::Number(v) if v.is_finite() => {
                        levels.insert(format!("{v}"));
                    }
                    _ => {}
                }
            }
            levels.len()
        }
        other => other.column_count(),
    }
}

/// R11: Kaplan-Meier Log-rank test for trend requires >= 3 ordered cohorts/groups.
#[must_use]
pub fn r_km_trend_test(survival_groups: usize) -> ControlEnablement {
    if survival_groups >= 3 {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled(format!("Log-rank test for trend requires 3 or more ordered cohorts (got {survival_groups})."))
    }
}

/// R10: PCA input checklist -> Leakage-safe `PCR` outcome picker (forbids any checked PCA predictor).
#[must_use]
pub fn r10_pcr_outcome_column(col_index: usize, pca_checked_predictors: &[usize]) -> ControlEnablement {
    if pca_checked_predictors.contains(&col_index) {
        ControlEnablement::disabled(format!(
            "Column {col_index} is checked as a PCA input predictor and cannot be selected as the PCR dependent variable (target leakage guard)."
        ))
    } else {
        ControlEnablement::enabled()
    }
}

// ============================================================================
// Aggregated Dialog Control, Tab & Button Predicates
// ============================================================================

/// Evaluate enablement for any named modal tab inside `EditAnalysisParameters`.
#[must_use]
pub fn evaluate_edit_params_tab(state: &EditAnalysisParametersState, tab_id: &str) -> ControlEnablement {
    let norm = normalize_key(tab_id);
    if norm == "repeatedmeasures" {
        return r1_repeated_measures_tab(state.paired_or_matched);
    }
    if (norm == "diagnostics" || norm == "residuals" || norm == "diagnosticsresiduals")
        && let Some(AnalysisKind::TwoGroupComparison(two)) = state.draft.as_ref().map(|d| &d.kind)
    {
        return r4_two_group_diagnostics(two.method);
    }
    ControlEnablement::enabled()
}

/// Evaluate contingency dimensions `(rows, cols)` and max subcolumn count for the upstream table
/// of a `ResultSheet` or active [`AnalyzeDialogState`].
#[must_use]
pub fn source_table_shape(doc: &ProjectDocument, source_sheet_id: Option<&SheetId>) -> (usize, usize, usize) {
    let Some(id) = source_sheet_id else {
        return (2, 2, 1);
    };
    let ds = doc
        .data_sheet(id)
        .or_else(|| doc.result_sheet(id).and_then(|rs| rs.analyzable_data.as_ref()))
        .or_else(|| doc.result_sheet(id).and_then(|rs| doc.data_sheet(&rs.analysis.source_sheet_id)));
    let Some(sheet) = ds else {
        return (2, 2, 1);
    };
    let max_sub = max_subcolumn_count(&sheet.archetype);
    match &sheet.archetype {
        TableArchetype::Contingency(cont) => (cont.row_count(), cont.data_sets.len(), max_sub),
        other => (other.row_count(), other.column_count(), max_sub),
    }
}

fn max_subcolumn_count(arch: &TableArchetype) -> usize {
    match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Contingency(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) | TableArchetype::Survival(_) => 1,
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Nested(t) => t.groups.first().map_or(1, |g| g.subcolumn_titles.len().max(1)),
    }
}

fn resolve_data_sheet<'a>(doc: &'a ProjectDocument, id: Option<&SheetId>) -> Option<&'a curvebook_doc::DataSheet> {
    let id = id?;
    doc.data_sheet(id)
        .or_else(|| doc.result_sheet(id).and_then(|rs| rs.analyzable_data.as_ref()))
        .or_else(|| doc.result_sheet(id).and_then(|rs| doc.data_sheet(&rs.analysis.source_sheet_id)))
}

/// Whether `source_sheet_id` (data sheet or result sheet source) is a pre-summarized `Column` table (`MeanSdN`, `MeanSemN`, `MeanCvN`).
#[must_use]
pub fn is_pre_summarized_column_table(doc: &ProjectDocument, source_sheet_id: Option<&SheetId>) -> bool {
    matches!(resolve_data_sheet(doc, source_sheet_id).map(|ds| &ds.archetype), Some(TableArchetype::Column(col)) if col.subcolumn_mode.is_pre_summarized())
}

/// Pre-summarized Column table gate for nonparametric mode (`assume_gaussian = false`).
#[must_use]
pub fn r2_assume_gaussian_allowed(doc: &ProjectDocument, source_sheet_id: Option<&SheetId>) -> ControlEnablement {
    if is_pre_summarized_column_table(doc, source_sheet_id) {
        ControlEnablement::disabled(
            "Nonparametric rank tests are disabled for pre-summarized (Mean +/- SD/SEM/%CV + n) Column tables; use raw replicates.",
        )
    } else {
        ControlEnablement::enabled()
    }
}

/// R1 predicate for `edit.paired_or_matched`: on `Column` tables, requires raw replicates and `>= 2`
/// data columns; for `ColumnDesignHint::PairedRepeatedMeasures`, incomplete-block RM tables remain
/// selectable for mixed-effects REML (`>= 2` active columns, `>= 2` subject rows, total obs > columns),
/// while `UnpairedReplicates` still requires equal replicate counts (`>= 2` complete blocks).
#[must_use]
pub fn r1_paired_or_matched_allowed(doc: &ProjectDocument, source_sheet_id: Option<&SheetId>) -> ControlEnablement {
    let Some(ds) = resolve_data_sheet(doc, source_sheet_id) else {
        return ControlEnablement::enabled();
    };
    let TableArchetype::Column(col) = &ds.archetype else {
        return ControlEnablement::enabled();
    };
    if col.subcolumn_mode.is_pre_summarized() {
        return ControlEnablement::disabled(
            "Paired and repeated-measures designs are disabled for pre-summarized (Mean +/- SD/SEM/%CV + n) Column tables.",
        );
    }
    if col.data_sets.len() < 2 {
        return ControlEnablement::disabled("Paired or repeated-measures analysis requires at least 2 data columns.");
    }
    let counts: Vec<usize> = (0..col.data_sets.len()).map(|c| col.active_column_values(c).len()).collect();
    if col.design_hint == ColumnDesignHint::PairedRepeatedMeasures {
        let active_cols = counts.iter().filter(|&&n| n >= 1).count();
        let total_obs: usize = counts.iter().sum();
        let rows_with_any = (0..col.row_count())
            .filter(|&r| (0..col.data_sets.len()).any(|c| col.data_sets[c].cell(r, 0).and_then(Cell::active_number).is_some()))
            .count();
        if active_cols < 2 || rows_with_any < 2 || total_obs <= active_cols {
            return ControlEnablement::disabled(
                "Incomplete-block repeated-measures analysis requires at least 2 active columns, 2 subject rows, and more total observations than conditions.",
            );
        }
        return ControlEnablement::enabled();
    }
    let first_n = counts.first().copied().unwrap_or(0);
    let all_cols: Vec<usize> = (0..col.data_sets.len()).collect();
    let complete_rows = col.complete_paired_rows(&all_cols).len();
    if first_n < 2 || !counts.iter().all(|&n| n == first_n) || complete_rows < 2 {
        return ControlEnablement::disabled(
            "Paired or repeated-measures analysis requires equal replicate counts (>= 2 rows) across all Column data sets.",
        );
    }
    ControlEnablement::enabled()
}

/// Evaluate a control widget by `widget_id` inside the currently active dialog.
#[must_use]
pub fn evaluate_dialog_widget(dialogs: &CanvasDialogState, doc: &ProjectDocument, widget_id: &str) -> ControlEnablement {
    let id = widget_id.trim();
    if let Some(button_res) = evaluate_dialog_button(dialogs, doc, id) {
        return button_res;
    }

    match &dialogs.active_dialog {
        ActiveDialog::EditAnalysisParameters { result_sheet_id } => {
            let (rows, cols, max_sub) = source_table_shape(doc, Some(result_sheet_id));
            evaluate_edit_params_widget(&dialogs.edit_params, doc, rows, cols, max_sub, id)
        }
        ActiveDialog::AnalyzeSheet { source_sheet_id } => evaluate_analyze_widget(&dialogs.analyze, doc, Some(source_sheet_id), id),
        ActiveDialog::PairwiseComparisons { .. } => evaluate_pairwise_widget(&dialogs.pairwise, id),
        ActiveDialog::PowerAndSampleSize => evaluate_power_wizard_widget(&dialogs.power_wizard, id),
        _ => ControlEnablement::enabled(),
    }
}

fn evaluate_pairwise_widget(pw: &PairwiseComparisonsState, widget_id: &str) -> ControlEnablement {
    match widget_id {
        "pairwise.cld_ordering"
        | "pairwise.cld_uppercase"
        | "pairwise.cld_alpha_override"
        | "pairwise.cld_font_size_pt"
        | "pairwise.cld_offset_pt" => {
            if pw.cld_enabled {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled("Enable Compact Letter Display to configure letter ordering and typography.")
            }
        }
        _ => ControlEnablement::enabled(),
    }
}

/// Evaluate dialog footer buttons (`ok`, `apply`, `cancel`, `back`, `next`, `create`, `preview`)
/// for `ui.inspect` and `ui.answer_dialog` validation.
#[must_use]
pub fn evaluate_dialog_button(dialogs: &CanvasDialogState, doc: &ProjectDocument, action: &str) -> Option<ControlEnablement> {
    let norm = normalize_key(action);
    match norm.as_str() {
        "cancel" => return Some(ControlEnablement::enabled()),
        "ok" | "apply" | "create" | "next" | "back" | "preview" => {}
        _ => return None,
    }

    Some(match &dialogs.active_dialog {
        ActiveDialog::None => ControlEnablement::disabled("No modal dialog is currently open."),
        ActiveDialog::PowerAndSampleSize => evaluate_power_button(dialogs, &norm),
        ActiveDialog::FormatSelectedPoints { graph_sheet_id } => evaluate_format_points_button(dialogs, doc, graph_sheet_id, &norm),
        ActiveDialog::AnalyzeSheet { .. } => evaluate_analyze_button(&dialogs.analyze, &norm),
        ActiveDialog::EditAnalysisParameters { result_sheet_id } => evaluate_edit_params_button(&dialogs.edit_params, doc, result_sheet_id, &norm),
        ActiveDialog::ChangeDataAnalyzed { .. } => {
            if dialogs.change_data.selected_target_id.is_some() {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled("Select a compatible upstream data table before confirming.")
            }
        }
        ActiveDialog::DuplicateAnalysisPrompt { .. } => ControlEnablement::enabled(),
        ActiveDialog::InsertModelEquation { .. } => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.insert_model_equation.result_sheet_id.is_none() {
                ControlEnablement::disabled("Select an upstream linear, nonlinear, or GLM result sheet first.")
            } else {
                ControlEnablement::enabled()
            }
        }
        ActiveDialog::RandomizeGroups => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.randomize_groups.commit_intent().is_none() {
                ControlEnablement::disabled("Specify at least one group arm and subject count >= 1.")
            } else {
                ControlEnablement::enabled()
            }
        }
        ActiveDialog::RandomSampleRows { .. } => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.random_sample_rows.commit_intent().is_none() {
                ControlEnablement::disabled("Select a valid source table and positive sample size.")
            } else {
                ControlEnablement::enabled()
            }
        }
        ActiveDialog::SimulateDistributions => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.simulate_distributions.commit_intent().is_none() {
                ControlEnablement::disabled("Specify at least one simulated column and n_rows >= 1.")
            } else {
                ControlEnablement::enabled()
            }
        }
        ActiveDialog::PairwiseComparisons { .. } => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.pairwise.cld_enabled && dialogs.pairwise.selected_result_sheet_id.is_none() {
                ControlEnablement::disabled("Select a source ResultSheet with pairwise comparisons before enabling Compact Letter Display.")
            } else {
                ControlEnablement::enabled()
            }
        }
        ActiveDialog::FormatAxes { .. }
        | ActiveDialog::FormatGraph { .. }
        | ActiveDialog::ChangeGraphType { .. }
        | ActiveDialog::BubblePlotInspector { .. }
        | ActiveDialog::ApplyStyleFrom { .. }
        | ActiveDialog::PageLayoutComposer { .. }
        | ActiveDialog::ExportFigure { .. }
        | ActiveDialog::ConvertArchetype { .. }
        | ActiveDialog::ImportReshape
        | ActiveDialog::ExcludeWithReason { .. }
        | ActiveDialog::ConstantHookPicker { .. }
        | ActiveDialog::ResultViewer { .. }
        | ActiveDialog::Print { .. }
        | ActiveDialog::CommandPalette
        | ActiveDialog::About => ControlEnablement::enabled(),
        ActiveDialog::CreateGraph { .. } => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.create_graph.commit_intent().is_none() {
                ControlEnablement::disabled("Select a source data table first.")
            } else {
                ControlEnablement::enabled()
            }
        }
        ActiveDialog::EqualizeScaling { .. } => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.equalize_scaling.commit_intent().is_none() {
                ControlEnablement::disabled("This layout has no live graph panels to equalize.")
            } else {
                ControlEnablement::enabled()
            }
        }
        ActiveDialog::FormatEmbeddedTable { .. } => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.format_embedded_table.commit_intent(doc).is_none() {
                ControlEnablement::disabled("Target embedded table annotation or panel no longer exists.")
            } else {
                ControlEnablement::enabled()
            }
        }
        ActiveDialog::FormatImage { .. } => {
            if matches!(norm.as_str(), "ok" | "apply") && dialogs.format_image.commit_intent().is_none() {
                ControlEnablement::disabled("Target embedded image no longer exists.")
            } else {
                ControlEnablement::enabled()
            }
        }
    })
}

fn evaluate_power_button(dialogs: &CanvasDialogState, button: &str) -> ControlEnablement {
    let pw = &dialogs.power_wizard;
    match button {
        "back" => {
            if pw.stage == PowerWizardStage::ExperimentDesign {
                ControlEnablement::disabled("Already on Stage 1 (Experiment Design).")
            } else {
                ControlEnablement::enabled()
            }
        }
        "next" => {
            if pw.stage == PowerWizardStage::Results {
                ControlEnablement::disabled("Already on the final Stage 4 (Results).")
            } else if let Err(reason) = pw.can_advance() {
                ControlEnablement::disabled(reason)
            } else {
                ControlEnablement::enabled()
            }
        }
        "create" | "ok" => {
            if pw.stage != PowerWizardStage::Results {
                ControlEnablement::disabled("Complete stages 1-3 and advance to Stage 4 Results before creating output sheets.")
            } else if let Some(ref err) = pw.validation_error {
                ControlEnablement::disabled(err.clone())
            } else if pw.result.is_none() {
                ControlEnablement::disabled("Power calculation has no valid solved result.")
            } else {
                ControlEnablement::enabled()
            }
        }
        _ => ControlEnablement::enabled(),
    }
}

fn evaluate_format_points_button(dialogs: &CanvasDialogState, doc: &ProjectDocument, graph_id: &SheetId, button: &str) -> ControlEnablement {
    if !matches!(button, "ok" | "apply") {
        return ControlEnablement::enabled();
    }
    if doc.graph_sheet(graph_id).is_none() {
        return ControlEnablement::disabled("Target graph sheet does not exist.");
    }
    if dialogs.format_points.selected_points.is_empty() {
        return ControlEnablement::disabled("Select at least one data point before applying point formatting.");
    }
    if !dialogs.format_points.clear_overrides && !dialogs.format_points.show_row_labels && dialogs.format_points.build_commit_intent().is_none() {
        return ControlEnablement::disabled("Specify at least one visual override (color, shape, size, border, or row labels) or Clear.");
    }
    ControlEnablement::enabled()
}

fn evaluate_analyze_button(analyze: &AnalyzeDialogState, button: &str) -> ControlEnablement {
    if !matches!(button, "ok" | "apply") {
        return ControlEnablement::enabled();
    }
    if analyze.selected_entry_id.is_none() {
        return ControlEnablement::disabled("Choose an analysis from the catalog before running.");
    }
    if analyze.selected_entry_id.as_deref() == Some("stats.pcr")
        && let Some(outcome_col) = analyze.pcr_outcome_column
    {
        let gate = r10_pcr_outcome_column(outcome_col, &analyze.selected_data_sets);
        if !gate.enabled {
            return gate;
        }
    }
    ControlEnablement::enabled()
}

fn evaluate_edit_params_button(
    edit: &EditAnalysisParametersState,
    doc: &ProjectDocument,
    result_sheet_id: &SheetId,
    button: &str,
) -> ControlEnablement {
    if !matches!(button, "ok" | "apply") {
        return ControlEnablement::enabled();
    }
    let Some(spec) = edit.draft.as_ref() else {
        return ControlEnablement::disabled("No active analysis specification loaded.");
    };
    let (rows, cols, _max_sub) = source_table_shape(doc, Some(result_sheet_id));
    if let AnalysisKind::Contingency(cont) = &spec.kind {
        if cont.method == curvebook_doc::ContingencyMethod::CochranArmitageTrend {
            let trend = r5_contingency_cochran_armitage_trend(rows, cols);
            if !trend.enabled {
                return trend;
            }
        }
        if (cont.compute_odds_and_risk_ratios || cont.compute_sensitivity_specificity) && !(rows == 2 && cols == 2) {
            return r5_contingency_2x2_effect_sizes(rows, cols);
        }
    }
    ControlEnablement::enabled()
}

fn evaluate_edit_params_widget(
    edit: &EditAnalysisParametersState,
    doc: &ProjectDocument,
    rows: usize,
    cols: usize,
    max_sub: usize,
    widget_id: &str,
) -> ControlEnablement {
    let norm = normalize_key(widget_id);
    if norm == "editpairedormatched" || norm == "pairedormatched" || norm == "matched" {
        return r1_paired_or_matched_allowed(doc, edit.draft.as_ref().map(|d| &d.source_sheet_id));
    }
    if norm == "editassumegaussian" || norm == "assumegaussian" {
        return r2_assume_gaussian_allowed(doc, edit.draft.as_ref().map(|d| &d.source_sheet_id));
    }
    let reml_active = is_reml_mixed_effects_active(edit, doc);
    if norm.contains("repeatedmeasures") && norm.contains("tab") {
        return r1_repeated_measures_tab(edit.paired_or_matched);
    }
    if norm.contains("geissergreenhouse") || norm.contains("assumesphericity") || norm == "editsphericity" || norm == "sphericity" {
        return r1b_sphericity_correction(edit.paired_or_matched, reml_active);
    }
    if norm.contains("reml") || norm.contains("dfkenwardroger") || norm.contains("dfsatterthwaite") || norm.contains("residualcov") {
        return r1_sphericity_or_reml_control(edit.paired_or_matched);
    }
    if norm.contains("dunn") && !norm.contains("dunnett") {
        return r1b_post_hoc_option_enabled(edit.assume_gaussian, reml_active, MultiplicityMethod::DunnRankSum);
    }
    if norm.contains("tukey") {
        return r1b_post_hoc_option_enabled(edit.assume_gaussian, reml_active, MultiplicityMethod::TukeyHsd);
    }
    if norm.contains("dunnett") {
        return r1b_post_hoc_option_enabled(edit.assume_gaussian, reml_active, MultiplicityMethod::Dunnett { control_column: 0 });
    }
    if norm.contains("gameshowell") {
        return r1b_post_hoc_option_enabled(edit.assume_gaussian, reml_active, MultiplicityMethod::GamesHowell);
    }
    if norm.contains("bonferroni") {
        return r1b_post_hoc_option_enabled(edit.assume_gaussian, reml_active, MultiplicityMethod::Bonferroni);
    }
    if norm.contains("holmsidak") {
        return r1b_post_hoc_option_enabled(edit.assume_gaussian, reml_active, MultiplicityMethod::HolmSidak);
    }
    if norm.contains("sidak") {
        return r1b_post_hoc_option_enabled(edit.assume_gaussian, reml_active, MultiplicityMethod::Sidak);
    }
    if norm.contains("fisherlsd") || norm.contains("lsd") {
        return r1b_post_hoc_option_enabled(edit.assume_gaussian, reml_active, MultiplicityMethod::FisherLsd);
    }
    if norm.contains("posthoc") {
        return r2_post_hoc_option_enabled(edit.assume_gaussian, true);
    }
    if norm.contains("controlcolumn") {
        let method = match edit.draft.as_ref().map(|s| &s.kind) {
            Some(AnalysisKind::OneWayAnova(a)) => a.multiplicity.method,
            _ => MultiplicityMethod::None,
        };
        return r3_control_column_picker(edit.compare_against_control, method);
    }
    if let Some(AnalysisKind::TwoGroupComparison(two)) = edit.draft.as_ref().map(|s| &s.kind) {
        if norm.contains("tabdiagnostics") || norm.starts_with("editdiag") {
            return r4_two_group_diagnostics(two.method);
        }
        if norm.contains("graphdifferences") {
            return r4_two_group_graph_differences(two.method);
        }
        if norm.contains("graphranks") || norm.contains("hodgeslehmann") {
            return r4_two_group_nonparametric_options(two.method);
        }
        if norm.contains("welch") {
            return r4_two_group_welch_correction(two.method);
        }
    }
    if norm.contains("contingency")
        || norm.contains("cochranarmitage")
        || norm.starts_with("editci")
        || norm.contains("baptistapike")
        || norm.contains("koopman")
        || norm.contains("newcombewilson")
        || norm.contains("wilsonbrown")
    {
        let cont = match edit.draft.as_ref().map(|s| &s.kind) {
            Some(AnalysisKind::Contingency(c)) => c.clone(),
            _ => ContingencySpec::default(),
        };
        if norm.contains("cochranarmitage") || norm.contains("trend") {
            return r5_contingency_cochran_armitage_trend(rows, cols);
        }
        if norm.contains("baptistapike") || norm.contains("woolf") {
            return r6_contingency_ci_selector(
                rows,
                cols,
                edit.contingency_odds_ratio_checked || cont.compute_odds_and_risk_ratios,
                "Baptista-Pike / Woolf",
            );
        }
        if norm.contains("koopman") {
            return r6_contingency_ci_selector(rows, cols, edit.contingency_relative_risk_checked || cont.compute_odds_and_risk_ratios, "Koopman");
        }
        if norm.contains("newcombewilson") {
            return r6_contingency_ci_selector(rows, cols, edit.contingency_attributable_risk_checked, "Newcombe-Wilson");
        }
        if norm.contains("wilsonbrown") {
            return r6_contingency_ci_selector(rows, cols, cont.compute_sensitivity_specificity, "Wilson-Brown");
        }
        if norm.contains("oddsratio")
            || norm.contains("relativerisk")
            || norm.contains("attributablerisk")
            || norm.contains("sensitivity")
            || norm.contains("fisher")
            || norm.contains("mcnemar")
        {
            return r5_contingency_2x2_effect_sizes(rows, cols);
        }
    }
    if norm.contains("heatmap") {
        return r7_correlation_heatmap_toggle(edit.correlation_all_pairs);
    }
    if norm.contains("subcolumnagg") || norm.contains("replicatesubcolumn") {
        return r8_subcolumn_aggregation(max_sub);
    }
    if (norm == "editkmtrendtest" || norm == "editkmlogrankfortrend" || norm == "trendtest")
        && let Some(spec) = &edit.draft
        && let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_group_column, .. }) = spec.kind
    {
        return r_km_trend_test(survival_group_count(doc, Some(&spec.source_sheet_id), mv_group_column));
    }
    if (norm.contains("pvaluedigits") || norm.contains("pvaluespinner"))
        && let Some(spec) = &edit.draft
    {
        return r9_pvalue_precision_spinner(&spec.p_value_format);
    }
    ControlEnablement::enabled()
}

fn evaluate_analyze_widget(
    analyze: &AnalyzeDialogState,
    doc: &ProjectDocument,
    source_sheet_id: Option<&SheetId>,
    widget_id: &str,
) -> ControlEnablement {
    let norm = normalize_key(widget_id);
    let (_rows, _cols, max_sub) = source_table_shape(doc, source_sheet_id);
    if norm == "analyzepairedormatched" || norm == "pairedormatched" || norm == "matched" {
        return r1_paired_or_matched_allowed(doc, source_sheet_id);
    }
    if norm == "analyzeassumegaussian" || norm == "assumegaussian" {
        return r2_assume_gaussian_allowed(doc, source_sheet_id);
    }
    if norm == "analyzekmtrendtest" || norm == "analyzekmlogrankfortrend" {
        return r_km_trend_test(survival_group_count(doc, source_sheet_id, analyze.mv_group_column));
    }
    if norm.contains("heatmap") {
        return r7_correlation_heatmap_toggle(analyze.correlation_all_pairs);
    }
    if norm.contains("subcolumnagg") || norm.contains("replicatesubcolumn") {
        return r8_subcolumn_aggregation(max_sub);
    }
    if let Some(rest) = widget_id.strip_prefix("analyze.pcr.outcome.")
        && let Ok(col_idx) = rest.parse::<usize>()
    {
        return r10_pcr_outcome_column(col_idx, &analyze.selected_data_sets);
    }
    ControlEnablement::enabled()
}

fn evaluate_power_wizard_widget(pw: &crate::power_wizard::PowerWizardState, widget_id: &str) -> ControlEnablement {
    let norm = normalize_key(widget_id);
    if (norm.contains("onetailed") || norm.contains("onesided")) && !pw.family.supports_one_sided() {
        return ControlEnablement::disabled(format!("{} supports two-tailed significance testing only.", pw.family.label()));
    }
    ControlEnablement::enabled()
}

fn normalize_key(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use curvebook_doc::{OneWayAnovaSpec, OneWayMethod, TwoGroupSpec};

    #[test]
    fn rules_r1_to_r5_cover_both_enabled_and_disabled_states() {
        // R1: ANOVA pairing -> Repeated Measures tab + sphericity/REML
        assert!(!r1_repeated_measures_tab(false).enabled);
        assert!(r1_repeated_measures_tab(false).disabled_reason.is_some());
        assert!(r1_repeated_measures_tab(true).enabled);
        assert!(!r1_sphericity_or_reml_control(false).enabled);
        assert!(r1_sphericity_or_reml_control(true).enabled);

        // R2: Gaussian toggle -> Post-hoc family + automatic switch to Dunn's
        assert!(!r2_parametric_post_hoc_method(false, MultiplicityMethod::TukeyHsd).enabled);
        assert!(r2_parametric_post_hoc_method(false, MultiplicityMethod::DunnRankSum).enabled);
        let mut anova = AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: true,
            multiplicity: curvebook_doc::MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: true,
            },
            ..OneWayAnovaSpec::default()
        });
        apply_r2_gaussian_toggle(&mut anova, false, false, 0);
        if let AnalysisKind::OneWayAnova(spec) = anova {
            assert_eq!(spec.method, OneWayMethod::KruskalWallis);
            assert_eq!(spec.multiplicity.method, MultiplicityMethod::DunnRankSum);
        } else {
            panic!("expected OneWayAnova");
        }

        // R3: Control-group contrast -> Picker & Dunnett
        assert!(!r3_control_column_picker(false, MultiplicityMethod::TukeyHsd).enabled);
        assert!(r3_control_column_picker(true, MultiplicityMethod::Dunnett { control_column: 0 }).enabled);

        // R4: Two-group test mode -> Output & variance options
        assert!(!r4_two_group_graph_differences(TwoGroupMethod::UnpairedT).enabled);
        assert!(r4_two_group_graph_differences(TwoGroupMethod::PairedT).enabled);
        assert!(!r4_two_group_nonparametric_options(TwoGroupMethod::UnpairedT).enabled);
        assert!(r4_two_group_nonparametric_options(TwoGroupMethod::MannWhitneyU).enabled);
        assert!(!r4_two_group_welch_correction(TwoGroupMethod::PairedT).enabled);
        assert!(!r4_two_group_welch_correction(TwoGroupMethod::MannWhitneyU).enabled);
        assert!(r4_two_group_welch_correction(TwoGroupMethod::UnpairedT).enabled);

        // R5: Contingency dimensions -> 2x2 effects vs Cochran-Armitage trend test
        assert!(r5_contingency_2x2_effect_sizes(2, 2).enabled);
        assert!(!r5_contingency_2x2_effect_sizes(3, 2).enabled);
        assert!(!r5_contingency_cochran_armitage_trend(2, 2).enabled);
        assert!(r5_contingency_cochran_armitage_trend(3, 2).enabled);
        assert!(r5_contingency_cochran_armitage_trend(2, 4).enabled);
    }

    #[test]
    fn rules_r6_to_r10_cover_both_enabled_and_disabled_states() {
        // R6: Contingency effect-size checkbox -> CI method selectors
        assert!(!r6_contingency_ci_selector(2, 2, false, "Baptista-Pike").enabled);
        assert!(r6_contingency_ci_selector(2, 2, true, "Baptista-Pike").enabled);
        assert!(!r6_contingency_ci_selector(3, 2, true, "Koopman").enabled);

        // R7: Correlation mode -> Heatmap toggle
        assert!(!r7_correlation_heatmap_toggle(false).enabled);
        assert!(r7_correlation_heatmap_toggle(true).enabled);

        // R8: Single-subcolumn table -> Subcolumn aggregation
        assert!(!r8_subcolumn_aggregation(1).enabled);
        assert!(r8_subcolumn_aggregation(3).enabled);

        // R9: P-value formatting style -> Precision spinner
        assert!(!r9_pvalue_precision_spinner(&PValueFormat::Apa).enabled);
        assert!(!r9_pvalue_precision_spinner(&PValueFormat::FourDecimal).enabled);
        assert!(r9_pvalue_precision_spinner(&PValueFormat::new_custom(5, None)).enabled);

        // R10: PCA input checklist -> Leakage-safe PCR outcome picker
        assert!(!r10_pcr_outcome_column(1, &[0, 1, 2]).enabled);
        assert!(r10_pcr_outcome_column(1, &[0, 1, 2]).disabled_reason.is_some());
        assert!(r10_pcr_outcome_column(3, &[0, 1, 2]).enabled);

        // KM Log-rank trend requires >= 3 cohorts
        assert!(!r_km_trend_test(2).enabled);
        assert!(r_km_trend_test(2).disabled_reason.is_some());
        assert!(r_km_trend_test(3).enabled);

        // Verify TwoGroupSpec constructor compiles cleanly in test scope
        let _tg = TwoGroupSpec::default();
    }

    #[test]
    fn wave7_r1_incomplete_rm_r1b_reml_and_r4_welch_predicates() {
        use curvebook_doc::{ColumnTable, DataSheet, FactorialAnovaSpec};

        // R1: Incomplete-block PairedRepeatedMeasures Column table remains selectable,
        // while unequal UnpairedReplicates remains disabled.
        let mut rm_tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 4).unwrap();
        rm_tab.data_sets[0].cells[0][0] = Cell::number(10.0);
        rm_tab.data_sets[1].cells[0][0] = Cell::number(12.0);
        rm_tab.data_sets[0].cells[1][0] = Cell::number(11.0);
        rm_tab.data_sets[2].cells[1][0] = Cell::number(14.0);
        rm_tab.data_sets[1].cells[2][0] = Cell::number(13.0);
        rm_tab.data_sets[2].cells[2][0] = Cell::number(15.0);
        let rm_ds = DataSheet::new("rm-inc", "RM Inc", TableArchetype::Column(rm_tab)).unwrap();

        let mut unp_tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).unwrap();
        for c in 0..3 {
            unp_tab.data_sets[c].cells[0][0] = Cell::number(1.0);
        }
        unp_tab.data_sets[0].cells[1][0] = Cell::number(2.0);
        unp_tab.data_sets[1].cells[1][0] = Cell::number(3.0);
        let unp_ds = DataSheet::new("unp-inc", "Unp Inc", TableArchetype::Column(unp_tab)).unwrap();

        let mut doc = ProjectDocument::new("Pred Test").unwrap();
        doc.add_data_sheet(rm_ds).unwrap();
        doc.add_data_sheet(unp_ds).unwrap();
        assert!(r1_paired_or_matched_allowed(&doc, Some(&SheetId::new("rm-inc"))).enabled);
        assert!(!r1_paired_or_matched_allowed(&doc, Some(&SheetId::new("unp-inc"))).enabled);

        // R1b: Sphericity disabled under KR/Satterthwaite REML, and post-hoc restricted to Bonferroni/Sidak/Holm-Sidak/FDR/Fisher's LSD
        assert!(r1b_sphericity_correction(true, false).enabled);
        assert!(!r1b_sphericity_correction(true, true).enabled);
        assert!(!r1b_post_hoc_option_enabled(true, true, MultiplicityMethod::TukeyHsd).enabled);
        assert!(!r1b_post_hoc_option_enabled(true, true, MultiplicityMethod::Dunnett { control_column: 0 }).enabled);
        assert!(!r1b_post_hoc_option_enabled(true, true, MultiplicityMethod::GamesHowell).enabled);
        for allowed in [
            MultiplicityMethod::Bonferroni,
            MultiplicityMethod::Sidak,
            MultiplicityMethod::HolmSidak,
            MultiplicityMethod::BenjaminiHochbergFdr,
            MultiplicityMethod::TwoStageBkyFdr,
            MultiplicityMethod::BenjaminiYekutieliFdr,
            MultiplicityMethod::FisherLsd,
            MultiplicityMethod::None,
        ] {
            assert!(r1b_post_hoc_option_enabled(true, true, allowed).enabled, "REML post-hoc should allow {allowed:?}");
        }

        // R4: TwoGroupMethod::WelchT disables Diagnostics / Residuals tab & widgets
        assert!(!r4_two_group_diagnostics(TwoGroupMethod::WelchT).enabled);
        assert!(r4_two_group_diagnostics(TwoGroupMethod::UnpairedT).enabled);
        let ep = EditAnalysisParametersState {
            draft: Some(
                curvebook_doc::AnalysisSpec::new(
                    "res-w",
                    "rm-inc",
                    AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }),
                )
                .unwrap(),
            ),
            ..Default::default()
        };
        assert!(!evaluate_edit_params_tab(&ep, "diagnostics").enabled);
        assert!(!evaluate_edit_params_widget(&ep, &doc, 4, 2, 1, "edit.diag.residuals_table").enabled);
        let _fac = FactorialAnovaSpec { df_method: curvebook_doc::MixedModelDfMethod::Satterthwaite, ..FactorialAnovaSpec::default() };
    }
}
