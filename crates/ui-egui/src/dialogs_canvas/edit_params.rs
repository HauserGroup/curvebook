//! `Edit Analysis Parameters...` (`Cmd+T`) dialog state machine and live preview diffing.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, DiagnosticPlotKind, GraphKind, MixedModelDfMethod, MultiplicityMethod, MultivariateSpec, OneWayMethod, OutlierSpec,
    PValueFormat, ProjectDocument, RepeatedMeasuresDesign, ResidualCovariance, ResultSheet, SheetId, SurvivalAnalysisSpec, TwoGroupMethod,
};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::analyze_dialog::{VariableRoleFilter, default_one_way_param_unpaired, default_outlier_alpha, default_outlier_fdr_q};

pub(super) const fn default_tg_param_unpaired() -> TwoGroupMethod {
    TwoGroupMethod::UnpairedT
}

pub(super) const fn default_tg_param_paired() -> TwoGroupMethod {
    TwoGroupMethod::PairedT
}

pub(super) const fn default_tg_nonparam_unpaired() -> TwoGroupMethod {
    TwoGroupMethod::MannWhitneyU
}

pub(super) const fn default_tg_nonparam_paired() -> TwoGroupMethod {
    TwoGroupMethod::WilcoxonSignedRank
}

/// Preserves each quadrant's last chosen [`TwoGroupMethod`] across Gaussian / paired toggles (`RV-76`, `RV-78`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TwoGroupSubmethodMemory {
    #[serde(default = "default_tg_param_unpaired")]
    pub saved_two_group_param_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_param_paired")]
    pub saved_two_group_param_paired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_unpaired")]
    pub saved_two_group_nonparam_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_paired")]
    pub saved_two_group_nonparam_paired: TwoGroupMethod,
}

impl Default for TwoGroupSubmethodMemory {
    fn default() -> Self {
        Self {
            saved_two_group_param_unpaired: default_tg_param_unpaired(),
            saved_two_group_param_paired: default_tg_param_paired(),
            saved_two_group_nonparam_unpaired: default_tg_nonparam_unpaired(),
            saved_two_group_nonparam_paired: default_tg_nonparam_paired(),
        }
    }
}

impl TwoGroupSubmethodMemory {
    pub fn remember(&mut self, method: TwoGroupMethod) {
        match method {
            TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::LognormalUnpairedT => {
                self.saved_two_group_param_unpaired = method;
            }
            TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT => {
                self.saved_two_group_param_paired = method;
            }
            TwoGroupMethod::MannWhitneyU | TwoGroupMethod::KolmogorovSmirnov => {
                self.saved_two_group_nonparam_unpaired = method;
            }
            TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest => {
                self.saved_two_group_nonparam_paired = method;
            }
            TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. } => {}
        }
    }

    #[must_use]
    pub const fn resolve(&self, assume_gaussian: bool, paired: bool, current: Option<TwoGroupMethod>) -> TwoGroupMethod {
        if let Some(TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical }) = current {
            return if assume_gaussian { TwoGroupMethod::OneSampleT { hypothetical } } else { TwoGroupMethod::OneSampleWilcoxon { hypothetical } };
        }
        match (assume_gaussian, paired) {
            (true, false) => self.saved_two_group_param_unpaired,
            (true, true) => self.saved_two_group_param_paired,
            (false, false) => self.saved_two_group_nonparam_unpaired,
            (false, true) => self.saved_two_group_nonparam_paired,
        }
    }
}

pub(super) const fn two_group_is_paired(method: TwoGroupMethod) -> bool {
    matches!(method, TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest)
}

pub(super) const fn two_group_is_gaussian(method: TwoGroupMethod) -> bool {
    !matches!(
        method,
        TwoGroupMethod::MannWhitneyU
            | TwoGroupMethod::WilcoxonSignedRank
            | TwoGroupMethod::KolmogorovSmirnov
            | TwoGroupMethod::OneSampleWilcoxon { .. }
            | TwoGroupMethod::PairedSignTest
    )
}

pub(super) fn check_pcr_target_leakage(col: usize, blocked: bool) -> Result<(), String> {
    if blocked {
        return Err(format!("Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."));
    }
    Ok(())
}

/// Toggle `col` inside a sorted `Vec<usize>`, returning `true` if inserted or `false` if removed.
pub(super) fn toggle_sorted_col(cols: &mut Vec<usize>, col: usize) -> bool {
    if let Some(pos) = cols.iter().position(|&c| c == col) {
        cols.remove(pos);
        false
    } else {
        cols.push(col);
        cols.sort_unstable();
        true
    }
}

pub(super) fn draft_forces_reml_mode(spec: &AnalysisSpec) -> bool {
    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => {
            matches!(a.method, OneWayMethod::RepeatedMeasures { .. })
                && (a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured)
        }
        AnalysisKind::FactorialAnova(f) => {
            f.repeated_measures != RepeatedMeasuresDesign::None
                && (f.mixed_effects_reml || f.residual_covariance == ResidualCovariance::Unstructured || f.three_way)
        }
        _ => false,
    }
}

pub(super) fn default_true_flag() -> bool {
    true
}

fn default_edit_tab() -> String {
    String::from("experimental_design")
}

/// `Cmd+T` parameter re-entry draft editing an existing [`AnalysisSpec`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditAnalysisParametersState {
    pub result_sheet_id: Option<SheetId>,
    pub draft: Option<AnalysisSpec>,
    pub title_was_custom: bool,
    #[serde(default = "default_edit_tab")]
    pub active_tab: String,
    #[serde(default = "default_true_flag")]
    pub assume_gaussian: bool,
    #[serde(default)]
    pub paired_or_matched: bool,
    #[serde(default)]
    pub compare_against_control: bool,
    #[serde(default)]
    pub control_column: usize,
    #[serde(default = "default_true_flag")]
    pub correlation_all_pairs: bool,
    #[serde(default)]
    pub correlation_heatmap: bool,
    #[serde(default)]
    pub contingency_odds_ratio_checked: bool,
    #[serde(default)]
    pub contingency_relative_risk_checked: bool,
    #[serde(default)]
    pub contingency_attributable_risk_checked: bool,
    #[serde(default)]
    pub contingency_or_ci_method: String,
    #[serde(default)]
    pub contingency_rr_ci_method: String,
    #[serde(default)]
    pub contingency_arr_ci_method: String,
    #[serde(default)]
    pub contingency_diag_ci_method: String,
    #[serde(default)]
    pub graph_differences: bool,
    #[serde(default)]
    pub graph_ranks: bool,
    #[serde(default)]
    pub hodges_lehmann_ci: bool,
    #[serde(default)]
    pub welch_correction: bool,
    #[serde(default)]
    pub pca_predictors: Vec<usize>,
    #[serde(default)]
    pub pcr_outcome_column: Option<usize>,
    #[serde(default)]
    pub role_filter: VariableRoleFilter,
    #[serde(flatten)]
    pub two_group_memory: TwoGroupSubmethodMemory,
    #[serde(default = "default_one_way_param_unpaired")]
    pub saved_one_way_param_unpaired: OneWayMethod,
    #[serde(default = "default_true_flag")]
    pub saved_rm_geisser_greenhouse: bool,
    #[serde(default = "default_outlier_fdr_q")]
    pub saved_outlier_rout_q: f64,
    #[serde(default = "default_outlier_alpha")]
    pub saved_outlier_grubbs_alpha: f64,
    #[serde(default)]
    pub saved_outlier_grubbs_iterative: bool,
    #[serde(default = "default_schematic_mode")]
    pub schematic_mode: String,
    #[serde(default = "default_true_flag")]
    pub diag_qq_plot: bool,
    #[serde(default = "default_true_flag")]
    pub diag_residuals_plot: bool,
    #[serde(default = "default_true_flag")]
    pub diag_homoscedasticity: bool,
    #[serde(default)]
    pub diag_influence_leverage: bool,
    #[serde(default = "default_true_flag")]
    pub diag_assumption_checklist: bool,
    #[serde(default)]
    pub synced_diag_qq: Option<bool>,
    #[serde(default)]
    pub synced_diag_residuals: Option<bool>,
    #[serde(default)]
    pub synced_diag_homoscedasticity: Option<bool>,
    #[serde(default)]
    pub synced_diag_influence: Option<bool>,
    /// `true` when `EditAnalysisParameters` stages an uncommitted first-run analysis (`analysis.run`).
    #[serde(default)]
    pub pending_create: bool,
    /// Optional deferred `(source_result_sheet_id, tab_id)` extraction (`sheet.extractResultTab`)
    /// executed atomically before `analysis.run` on commit.
    #[serde(default)]
    pub pending_extract: Option<(SheetId, String)>,
}

fn default_schematic_mode() -> String {
    String::from("auto")
}

impl Default for EditAnalysisParametersState {
    fn default() -> Self {
        Self {
            result_sheet_id: None,
            draft: None,
            title_was_custom: false,
            active_tab: default_edit_tab(),
            assume_gaussian: true,
            paired_or_matched: false,
            compare_against_control: false,
            control_column: 0,
            correlation_all_pairs: true,
            correlation_heatmap: false,
            contingency_odds_ratio_checked: false,
            contingency_relative_risk_checked: false,
            contingency_attributable_risk_checked: false,
            contingency_or_ci_method: String::from("Baptista-Pike"),
            contingency_rr_ci_method: String::from("Koopman"),
            contingency_arr_ci_method: String::from("Newcombe-Wilson"),
            contingency_diag_ci_method: String::from("Wilson-Brown"),
            graph_differences: false,
            graph_ranks: false,
            hodges_lehmann_ci: false,
            welch_correction: false,
            pca_predictors: Vec::new(),
            pcr_outcome_column: None,
            role_filter: VariableRoleFilter::All,
            two_group_memory: TwoGroupSubmethodMemory::default(),
            saved_one_way_param_unpaired: default_one_way_param_unpaired(),
            saved_rm_geisser_greenhouse: true,
            saved_outlier_rout_q: default_outlier_fdr_q(),
            saved_outlier_grubbs_alpha: default_outlier_alpha(),
            saved_outlier_grubbs_iterative: false,
            schematic_mode: default_schematic_mode(),
            diag_qq_plot: true,
            diag_residuals_plot: true,
            diag_homoscedasticity: true,
            diag_influence_leverage: false,
            diag_assumption_checklist: true,
            synced_diag_qq: None,
            synced_diag_residuals: None,
            synced_diag_homoscedasticity: None,
            synced_diag_influence: None,
            pending_create: false,
            pending_extract: None,
        }
    }
}

impl EditAnalysisParametersState {
    pub(super) fn try_update_draft_kind(&mut self, mutate: impl FnOnce(&mut AnalysisKind) -> Result<bool, String>) -> Result<bool, String> {
        let Some(spec) = &mut self.draft else {
            return Ok(false);
        };
        let mut updated = spec.kind.clone();
        let changed = mutate(&mut updated)?;
        if changed {
            spec.update_kind(updated);
        }
        Ok(changed)
    }

    pub(super) fn update_draft_kind(&mut self, mutate: impl FnOnce(&mut AnalysisKind) -> bool) {
        if let Some(spec) = &mut self.draft {
            let mut updated = spec.kind.clone();
            if mutate(&mut updated) {
                spec.update_kind(updated);
            }
        }
    }

    /// Toggle a MV predictor column for PCA/PCR while maintaining disjoint PCR outcome selection.
    pub fn toggle_pca_predictor(&mut self, col: usize) {
        if toggle_sorted_col(&mut self.pca_predictors, col) && self.pcr_outcome_column == Some(col) {
            self.pcr_outcome_column = None;
        }
        if let Some(spec) = &mut self.draft {
            spec.selected_data_sets.clone_from(&self.pca_predictors);
            if let AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column, .. }) = &mut spec.kind {
                *pcr_outcome_column = self.pcr_outcome_column;
            }
        }
    }

    /// Select (or toggle off) the disjoint PCR outcome column with R10 predictor leakage guard.
    pub fn select_pcr_outcome(&mut self, col: usize) -> Result<(), String> {
        check_pcr_target_leakage(col, self.pca_predictors.contains(&col))?;
        let next = if self.pcr_outcome_column == Some(col) { None } else { Some(col) };
        self.set_draft_pcr_outcome(next);
        Ok(())
    }

    /// Reset dialog state and open Edit Analysis Parameters for an existing `ResultSheet` (`RV-79`).
    pub fn open_for_result(&mut self, result: &ResultSheet) {
        self.open_from_spec(result.id.clone(), result.analysis.clone(), false, None);
    }

    /// Reset dialog state and stage an uncommitted first-run `AnalysisSpec` (`R20.20`).
    ///
    /// On `OK`, `commit_intent` emits `analysis.run` rather than `analysis.updateParams`; on `Cancel`,
    /// closing discards this draft without mutating the project document.
    pub fn open_for_pending_create(&mut self, spec: AnalysisSpec, pending_extract: Option<(SheetId, String)>) {
        let id = spec.id.clone();
        self.open_from_spec(id, spec, true, pending_extract);
    }

    /// Clear transient draft and pending-create state when dismissing the modal.
    pub fn clear_session(&mut self) {
        *self = Self::default();
    }

    fn open_from_spec(&mut self, sheet_id: SheetId, spec: AnalysisSpec, pending_create: bool, pending_extract: Option<(SheetId, String)>) {
        *self = Self::default();
        self.result_sheet_id = Some(sheet_id);
        self.title_was_custom = spec.custom_title.is_some();
        self.pending_create = pending_create;
        self.pending_extract = pending_extract;
        if let AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column, .. }) = &spec.kind {
            self.pca_predictors.clone_from(&spec.selected_data_sets);
            self.pcr_outcome_column = *pcr_outcome_column;
        }
        self.draft = Some(spec);
        self.sync_rule_flags_from_kind();
        if let Some(spec_ref) = &self.draft {
            match &spec_ref.kind {
                AnalysisKind::Contingency(cg) => {
                    self.contingency_odds_ratio_checked = cg.compute_odds_and_risk_ratios;
                    self.contingency_relative_risk_checked = cg.compute_odds_and_risk_ratios;
                    self.contingency_attributable_risk_checked = cg.compute_odds_and_risk_ratios;
                }
                AnalysisKind::TwoGroupComparison(two) => {
                    self.hodges_lehmann_ci = matches!(two.method, TwoGroupMethod::MannWhitneyU | TwoGroupMethod::WilcoxonSignedRank);
                }
                AnalysisKind::NonlinearRegression(nl) => {
                    self.diag_qq_plot = nl.normality_of_residuals;
                    self.diag_residuals_plot = nl.emit_residuals_tab;
                    self.diag_homoscedasticity = nl.homoscedasticity_test;
                    self.synced_diag_qq = Some(nl.normality_of_residuals);
                    self.synced_diag_residuals = Some(nl.emit_residuals_tab);
                    self.synced_diag_homoscedasticity = Some(nl.homoscedasticity_test);
                }
                AnalysisKind::GeneralizedLinearModel(glm) => {
                    self.diag_influence_leverage = glm.compute_vif;
                    self.synced_diag_influence = Some(glm.compute_vif);
                }
                _ => {}
            }
        }
    }

    /// Capture compatible sub-method selections before a design/Gaussian toggle (`RV-76`, `RV-78`).
    pub fn remember_submethods_from_kind(&mut self, kind: &AnalysisKind) {
        match kind {
            AnalysisKind::OneWayAnova(spec) => match spec.method {
                OneWayMethod::OrdinaryAnova | OneWayMethod::BrownForsytheAndWelch => {
                    self.saved_one_way_param_unpaired = spec.method;
                }
                OneWayMethod::RepeatedMeasures { geisser_greenhouse } => {
                    self.saved_rm_geisser_greenhouse = geisser_greenhouse;
                }
                OneWayMethod::KruskalWallis | OneWayMethod::Friedman => {}
            },
            AnalysisKind::TwoGroupComparison(spec) => self.two_group_memory.remember(spec.method),
            AnalysisKind::OutlierDetection(spec) => match spec {
                OutlierSpec::Rout { fdr_q_percent } => self.saved_outlier_rout_q = *fdr_q_percent,
                OutlierSpec::Grubbs { alpha, iterative } => {
                    self.saved_outlier_grubbs_alpha = *alpha;
                    self.saved_outlier_grubbs_iterative = *iterative;
                }
            },
            _ => {}
        }
    }

    /// Synchronize predicate flags (`assume_gaussian`, `paired_or_matched`, etc.) from `draft.kind`.
    pub fn sync_rule_flags_from_kind(&mut self) {
        let Some(kind) = self.draft.as_ref().map(|s| s.kind.clone()) else {
            return;
        };
        self.remember_submethods_from_kind(&kind);
        match &kind {
            AnalysisKind::OneWayAnova(spec) => {
                self.assume_gaussian = !matches!(spec.method, OneWayMethod::KruskalWallis | OneWayMethod::Friedman);
                self.paired_or_matched = matches!(spec.method, OneWayMethod::RepeatedMeasures { .. } | OneWayMethod::Friedman);
                self.sync_dunnett_control(spec.multiplicity.method);
            }
            AnalysisKind::FactorialAnova(spec) => {
                self.assume_gaussian = true;
                self.paired_or_matched = spec.repeated_measures != RepeatedMeasuresDesign::None;
                self.sync_dunnett_control(spec.multiplicity.method);
            }
            AnalysisKind::TwoGroupComparison(spec) => {
                self.assume_gaussian = two_group_is_gaussian(spec.method);
                self.paired_or_matched = two_group_is_paired(spec.method);
                self.welch_correction = spec.method == TwoGroupMethod::WelchT;
                if !crate::modal_predicates::r4_two_group_graph_differences(spec.method).enabled {
                    self.graph_differences = false;
                }
                if !crate::modal_predicates::r4_two_group_nonparametric_options(spec.method).enabled {
                    self.graph_ranks = false;
                    self.hodges_lehmann_ci = false;
                }
            }
            AnalysisKind::Contingency(cg) => {
                self.contingency_odds_ratio_checked = cg.compute_odds_and_risk_ratios;
                self.contingency_relative_risk_checked = cg.compute_odds_and_risk_ratios;
                if !cg.compute_odds_and_risk_ratios {
                    self.contingency_attributable_risk_checked = false;
                }
            }
            AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column, .. }) => {
                self.pca_predictors = self.draft.as_ref().map(|d| d.selected_data_sets.clone()).unwrap_or_default();
                self.pcr_outcome_column = *pcr_outcome_column;
            }
            _ => {}
        }
    }

    fn sync_dunnett_control(&mut self, method: MultiplicityMethod) {
        if let MultiplicityMethod::Dunnett { control_column } = method {
            self.compare_against_control = true;
            self.control_column = control_column;
        } else {
            self.compare_against_control = false;
        }
    }

    /// Mutate [`AnalysisKind`] in-place, updating `auto_title` while preserving any user-pinned
    /// `custom_title`.
    pub fn set_kind(&mut self, new_kind: AnalysisKind) -> bool {
        let Some(spec) = &mut self.draft else {
            return false;
        };
        spec.update_kind(new_kind);
        self.sync_rule_flags_from_kind();
        true
    }

    /// Update confidence percentage (`0.01..=99.99`).
    pub fn set_confidence_percent(&mut self, percent: f64) -> Result<(), String> {
        let Some(spec) = &mut self.draft else {
            return Err(String::from("no active analysis draft"));
        };
        spec.set_confidence_percent(percent).map_err(|e| e.to_string())
    }

    /// Set display P-value reporting format.
    pub fn set_p_value_format(&mut self, format: PValueFormat) {
        if let Some(spec) = &mut self.draft {
            spec.p_value_format = format;
        }
    }

    /// Update custom decimal precision when `PValueFormat::Custom` is active.
    pub fn set_p_value_decimals(&mut self, decimals: u8) -> Result<(), String> {
        if let Some(spec) = &mut self.draft {
            let PValueFormat::Custom { scientific_below, .. } = spec.p_value_format else {
                return Err(String::from("Custom decimal precision spinner is disabled unless Custom P-value format is selected."));
            };
            spec.p_value_format = PValueFormat::new_custom(decimals, scientific_below);
        }
        Ok(())
    }

    /// Toggle Gaussian / parametric assumption with sub-method quadrant memory.
    pub fn set_assume_gaussian(&mut self, assume_gaussian: bool) {
        self.assume_gaussian = assume_gaussian;
        let matched = self.paired_or_matched;
        let ctrl = self.control_column;
        let Some(mut spec) = self.draft.take() else {
            return;
        };
        self.remember_submethods_from_kind(&spec.kind);
        let mut kind = spec.kind.clone();
        crate::modal_predicates::apply_r2_gaussian_toggle(&mut kind, assume_gaussian, matched, ctrl);
        match &mut kind {
            AnalysisKind::OneWayAnova(a) if assume_gaussian => {
                a.method = if matched {
                    OneWayMethod::RepeatedMeasures { geisser_greenhouse: self.saved_rm_geisser_greenhouse }
                } else {
                    self.saved_one_way_param_unpaired
                };
            }
            AnalysisKind::TwoGroupComparison(t) => {
                t.method = self.two_group_memory.resolve(assume_gaussian, matched, Some(t.method));
            }
            _ => {}
        }
        spec.update_kind(kind);
        self.draft = Some(spec);
        self.sync_rule_flags_from_kind();
    }

    /// Toggle paired / repeated-measures design with sub-method quadrant memory.
    pub fn set_paired_or_matched(&mut self, matched: bool) {
        self.paired_or_matched = matched;
        let Some(mut spec) = self.draft.take() else {
            return;
        };
        self.remember_submethods_from_kind(&spec.kind);
        let mut updated = spec.kind.clone();
        match &mut updated {
            AnalysisKind::OneWayAnova(a) => {
                a.method = match (self.assume_gaussian, matched) {
                    (true, true) => OneWayMethod::RepeatedMeasures { geisser_greenhouse: self.saved_rm_geisser_greenhouse },
                    (true, false) => self.saved_one_way_param_unpaired,
                    (false, true) => OneWayMethod::Friedman,
                    (false, false) => OneWayMethod::KruskalWallis,
                };
            }
            AnalysisKind::TwoGroupComparison(t) => {
                t.method = self.two_group_memory.resolve(self.assume_gaussian, matched, Some(t.method));
            }
            AnalysisKind::FactorialAnova(f) => {
                f.repeated_measures = if matched { RepeatedMeasuresDesign::RowFactorMatched } else { RepeatedMeasuresDesign::None };
            }
            _ => {}
        }
        spec.update_kind(updated);
        self.draft = Some(spec);
        self.sync_rule_flags_from_kind();
    }

    /// Enable or disable compare-against-control mode (switches post-hoc to Dunnett when enabled).
    pub fn set_compare_against_control(&mut self, enabled: bool) {
        self.compare_against_control = enabled;
        if enabled {
            self.set_anova_multiplicity(MultiplicityMethod::Dunnett { control_column: self.control_column });
        }
    }

    /// Set the control column index for Dunnett comparisons.
    pub fn set_control_column(&mut self, col: usize) {
        self.control_column = col;
        if self.compare_against_control {
            self.set_anova_multiplicity(MultiplicityMethod::Dunnett { control_column: col });
        }
    }

    /// Toggle correlation all-pairs mode (turning it off clears heatmap generation).
    pub fn set_correlation_all_pairs(&mut self, all_pairs: bool) {
        self.correlation_all_pairs = all_pairs;
        if !all_pairs {
            self.correlation_heatmap = false;
        }
    }

    /// Toggle correlation heatmap generation (requires all-pairs mode).
    pub fn set_correlation_heatmap(&mut self, enabled: bool) -> Result<(), String> {
        if enabled && !self.correlation_all_pairs {
            return Err(String::from("Create correlation heatmap is disabled in X vs every Y mode."));
        }
        self.correlation_heatmap = enabled;
        Ok(())
    }

    /// Set multiplicity method on ANOVA / Nested / Kaplan-Meier drafts with R1b/R2 gating.
    pub fn set_multiplicity_method(&mut self, method: MultiplicityMethod) -> Result<(), String> {
        let draft_reml = self.draft.as_ref().is_some_and(draft_forces_reml_mode);
        let gate = crate::modal_predicates::r1b_post_hoc_option_enabled(self.assume_gaussian, draft_reml, method);
        if !gate.enabled {
            return Err(gate.disabled_reason.unwrap_or_else(|| String::from("Post-hoc method is disabled.")));
        }
        self.set_anova_multiplicity(method);
        Ok(())
    }

    fn set_anova_multiplicity(&mut self, method: MultiplicityMethod) {
        self.update_draft_kind(|updated| {
            match updated {
                AnalysisKind::OneWayAnova(a) => a.multiplicity.method = method,
                AnalysisKind::FactorialAnova(f) => f.multiplicity.method = method,
                AnalysisKind::NestedComparison(n) => n.multiplicity.method = method,
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => multiplicity.method = method,
                _ => return false,
            }
            true
        });
        if self.draft.is_some() {
            self.sync_rule_flags_from_kind();
        }
    }

    /// Update one-way ANOVA method while preserving sub-method memory and syncing rule flags.
    pub fn set_one_way_method(&mut self, method: OneWayMethod) {
        if let Some(mut spec) = self.draft.take() {
            self.remember_submethods_from_kind(&spec.kind);
            if let AnalysisKind::OneWayAnova(mut a) = spec.kind.clone() {
                a.method = method;
                spec.update_kind(AnalysisKind::OneWayAnova(a));
            }
            self.draft = Some(spec);
            self.sync_rule_flags_from_kind();
        }
    }

    /// Set Geisser-Greenhouse sphericity correction on repeated-measures one-way ANOVA.
    pub fn set_geisser_greenhouse(&mut self, gg: bool) -> Result<(), String> {
        if !self.paired_or_matched {
            return Err(String::from("Geisser-Greenhouse sphericity control is disabled when No matching is selected."));
        }
        self.try_update_draft_kind(|k| match k {
            AnalysisKind::OneWayAnova(a) => {
                if a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured {
                    return Err(String::from("Sphericity corrections are disabled when Mixed-Effects REML is active."));
                }
                a.method = OneWayMethod::RepeatedMeasures { geisser_greenhouse: gg };
                Ok(true)
            }
            _ => Ok(false),
        })?;
        self.sync_rule_flags_from_kind();
        Ok(())
    }

    /// Enable or disable mixed-effects REML on One-Way or Factorial ANOVA drafts.
    pub fn set_mixed_effects_reml(&mut self, enabled: bool) {
        self.update_draft_kind(|k| match k {
            AnalysisKind::OneWayAnova(a) => {
                a.mixed_effects_reml = enabled;
                if enabled && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(a.multiplicity.method) {
                    a.multiplicity.method = MultiplicityMethod::Bonferroni;
                }
                true
            }
            AnalysisKind::FactorialAnova(f) => {
                f.mixed_effects_reml = enabled;
                if enabled && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(f.multiplicity.method) {
                    f.multiplicity.method = MultiplicityMethod::Bonferroni;
                }
                true
            }
            _ => false,
        });
        self.sync_rule_flags_from_kind();
    }

    /// Set denominator degrees-of-freedom approximation for REML / nested models.
    pub fn set_df_method(&mut self, method: MixedModelDfMethod) {
        self.update_draft_kind(|k| match k {
            AnalysisKind::OneWayAnova(a) => {
                a.df_method = method;
                true
            }
            AnalysisKind::FactorialAnova(f) => {
                f.df_method = method;
                true
            }
            AnalysisKind::NestedComparison(n) => {
                n.df_method = method;
                true
            }
            _ => false,
        });
        self.sync_rule_flags_from_kind();
    }

    /// Set residual covariance structure on One-Way or Factorial ANOVA drafts.
    pub fn set_residual_covariance(&mut self, cov: ResidualCovariance) {
        self.update_draft_kind(|k| match k {
            AnalysisKind::OneWayAnova(a) => {
                a.residual_covariance = cov;
                true
            }
            AnalysisKind::FactorialAnova(f) => {
                f.residual_covariance = cov;
                true
            }
            _ => false,
        });
        self.sync_rule_flags_from_kind();
    }

    /// Toggle Welch's unequal-variance correction on two-group parametric drafts.
    pub fn set_welch_correction(&mut self, enabled: bool) -> Result<(), String> {
        let two_method = self.draft.as_ref().and_then(|s| match &s.kind {
            AnalysisKind::TwoGroupComparison(two) => Some(two.method),
            _ => None,
        });
        if let Some(method) = two_method {
            if !crate::modal_predicates::r4_two_group_welch_correction(method).enabled {
                if enabled {
                    return Err(String::from("Welch's correction is disabled for paired or nonparametric tests."));
                }
                self.welch_correction = false;
                return Ok(());
            }
            self.welch_correction = enabled;
            self.update_draft_kind(|k| match k {
                AnalysisKind::TwoGroupComparison(two) => {
                    if enabled {
                        two.method = TwoGroupMethod::WelchT;
                    } else if matches!(two.method, TwoGroupMethod::WelchT | TwoGroupMethod::UnpairedT) {
                        two.method = TwoGroupMethod::UnpairedT;
                    }
                    true
                }
                _ => false,
            });
            self.sync_rule_flags_from_kind();
        }
        Ok(())
    }

    /// Toggle attributable risk option on Contingency drafts.
    pub fn set_contingency_attributable_risk(&mut self, enabled: bool) {
        self.contingency_attributable_risk_checked = enabled;
        let keep_ratios = enabled || self.contingency_odds_ratio_checked || self.contingency_relative_risk_checked;
        self.update_draft_kind(|k| match k {
            AnalysisKind::Contingency(c) => {
                c.compute_odds_and_risk_ratios = keep_ratios;
                true
            }
            _ => false,
        });
    }

    /// Toggle automatic k-means cluster selection (`None` <-> `Some(3)`).
    pub fn toggle_k_means_auto(&mut self) {
        self.update_draft_kind(|k| match k {
            AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, .. }) => {
                *k_means_k = if k_means_k.is_none() { Some(3) } else { None };
                true
            }
            _ => false,
        });
        self.sync_rule_flags_from_kind();
    }

    /// Update GLM VIF computation flag on the active draft.
    pub fn set_glm_compute_vif(&mut self, enabled: bool) {
        self.update_draft_kind(|k| match k {
            AnalysisKind::GeneralizedLinearModel(glm) => {
                glm.compute_vif = enabled;
                true
            }
            _ => false,
        });
    }

    /// Pin or clear custom title on the draft [`AnalysisSpec`].
    pub fn set_custom_title(&mut self, custom: Option<String>) -> Result<(), String> {
        let Some(spec) = &mut self.draft else {
            return Err(String::from("no active analysis draft"));
        };
        spec.set_custom_title(custom).map_err(|e| e.to_string())?;
        self.title_was_custom = spec.custom_title.is_some();
        Ok(())
    }

    pub(super) fn set_draft_pcr_outcome(&mut self, outcome: Option<usize>) {
        self.pcr_outcome_column = outcome;
        if let Some(spec) = &mut self.draft {
            if let Some(col) = outcome {
                spec.selected_data_sets.retain(|&c| c != col);
            }
            if let AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column, .. }) = &mut spec.kind {
                *pcr_outcome_column = outcome;
            }
        }
    }

    /// Effective displayed title reflecting auto-title vs custom title rules.
    #[must_use]
    pub fn preview_title(&self) -> Option<&str> {
        self.draft.as_ref().map(AnalysisSpec::effective_title)
    }

    /// Returns an explicit error when a `Contingency` draft selects a non-default CI method
    /// (`Woolf`, `Katz`, `Wald`, `Clopper-Pearson`) that `ContingencySpec` / `curvebook-stats`
    /// does not yet compute (`AUD-09-12`), preventing silent wrong-method commits.
    #[must_use]
    pub fn unsupported_contingency_ci_error(&self) -> Option<String> {
        let spec = self.draft.as_ref()?;
        if !matches!(spec.kind, AnalysisKind::Contingency(_)) {
            return None;
        }
        for (label, current, supported) in [
            ("OR", self.contingency_or_ci_method.as_str(), "Baptista-Pike"),
            ("RR", self.contingency_rr_ci_method.as_str(), "Koopman"),
            ("ARR", self.contingency_arr_ci_method.as_str(), "Newcombe-Wilson"),
            ("diagnostic", self.contingency_diag_ci_method.as_str(), "Wilson-Brown"),
        ] {
            if current != supported {
                return Some(format!("unsupported {label} CI method `{current}`; engine currently computes `{supported}` only"));
            }
        }
        None
    }

    /// Emit declarative `analysis.updateParams` (existing sheet) or `analysis.run` (pending first-run create).
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        if self.unsupported_contingency_ci_error().is_some() {
            return None;
        }
        let spec = self.draft.as_ref()?;
        let mut kind = spec.kind.clone();
        match &mut kind {
            AnalysisKind::NonlinearRegression(nl) => {
                if self.synced_diag_qq.is_some_and(|s| s != self.diag_qq_plot) {
                    nl.normality_of_residuals = self.diag_qq_plot;
                }
                if self.synced_diag_residuals.is_some_and(|s| s != self.diag_residuals_plot) {
                    nl.emit_residuals_tab = self.diag_residuals_plot;
                }
                if self.synced_diag_homoscedasticity.is_some_and(|s| s != self.diag_homoscedasticity) {
                    nl.homoscedasticity_test = self.diag_homoscedasticity;
                }
            }
            AnalysisKind::GeneralizedLinearModel(glm) if self.synced_diag_influence.is_some_and(|s| s != self.diag_influence_leverage) => {
                glm.compute_vif = self.diag_influence_leverage;
            }
            _ => {}
        }
        if self.pending_create {
            let source_sheet_id = spec.source_sheet_id.as_ref()?;
            let mut payload = json!({
                "source_sheet_id": source_sheet_id.as_str(),
                "kind": kind,
                "confidence_percent": spec.confidence_level.percent(),
                "selected_data_sets": spec.selected_data_sets,
                "p_value_format": spec.p_value_format,
            });
            if let Some(title) = &spec.custom_title
                && let Some(map) = payload.as_object_mut()
            {
                map.insert(String::from("title"), json!(title));
            }
            return Some(ToolCommandIntent::new("analysis.run", payload));
        }
        let id = self.result_sheet_id.as_ref()?;
        let mut payload = json!({
            "sheet_id": id.as_str(),
            "kind": kind,
            "confidence_percent": spec.confidence_level.percent(),
            "selected_data_sets": spec.selected_data_sets,
            "p_value_format": spec.p_value_format,
            "title": spec.custom_title,
        });
        if let Some(map) = payload.as_object_mut() {
            if matches!(spec.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. })) {
                map.insert(String::from("correlation_all_pairs"), json!(self.correlation_all_pairs));
                map.insert(String::from("correlation_heatmap"), json!(self.correlation_heatmap));
            }
            if matches!(spec.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })) {
                map.insert(String::from("pcr_outcome_column"), json!(self.pcr_outcome_column));
            }
        }
        Some(ToolCommandIntent::new("analysis.updateParams", payload))
    }

    /// Emit `analysis.updateParams` (or `analysis.run` in pending create mode) and, when
    /// `correlation_heatmap` is toggled on for a correlation matrix, spawn a linked
    /// `DiagnosticPlotKind::Heatmap` `GraphSheet` if none already points to `result_sheet_id`.
    #[must_use]
    pub fn commit_intents(&self, doc: Option<&ProjectDocument>) -> Vec<ToolCommandIntent> {
        let Some(primary) = self.commit_intent() else {
            return Vec::new();
        };
        let mut intents = vec![primary];
        if !self.pending_create
            && let (Some(id), Some(spec)) = (&self.result_sheet_id, &self.draft)
            && self.correlation_heatmap
            && self.correlation_all_pairs
            && matches!(spec.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
        {
            let already_has_heatmap = doc.is_some_and(|d| {
                d.graph_sheets
                    .iter()
                    .any(|g| g.spec.primary_source_sheet_id == *id && g.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap))
            });
            if !already_has_heatmap {
                intents.push(ToolCommandIntent::new(
                    "graph.create",
                    json!({
                        "data_sheet_id": id.as_str(),
                        "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
                        "initialized": true,
                    }),
                ));
            }
        }
        if let (Some(spec), Some(d)) = (&self.draft, doc)
            && self.graph_differences
            && let AnalysisKind::TwoGroupComparison(two) = &spec.kind
            && crate::modal_predicates::r4_two_group_graph_differences(two.method).enabled
            && let Some(src_id) = spec.source_sheet_id.as_ref()
            && let Some(source_ds) = d.data_sheet(src_id)
            && matches!(source_ds.archetype, curvebook_doc::TableArchetype::Column(_))
        {
            let already_has_before_after = d.graph_sheets.iter().any(|g| {
                g.spec.primary_source_sheet_id == *src_id
                    && g.spec.series.iter().any(|s| matches!(s.gdo, curvebook_doc::GraphicalDataObject::BeforeAfterLines { .. }))
            });
            if !already_has_before_after {
                let base_id = format!("{}_diff", src_id.as_str());
                let mut cand = base_id.clone();
                let mut suffix = 2_usize;
                while let Ok(sid) = SheetId::try_new(cand.as_str())
                    && d.sheet_section(&sid).is_some()
                {
                    cand = format!("{base_id}_{suffix}");
                    suffix += 1;
                }
                if SheetId::try_new(cand.as_str()).is_ok() {
                    intents.push(ToolCommandIntent::new(
                        "graph.create",
                        json!({
                            "sheet_id": cand,
                            "data_sheet_id": src_id.as_str(),
                            "kind": GraphKind::Column,
                            "title": format!("{} (Before-After)", source_ds.title),
                            "initialized": true,
                        }),
                    ));
                    intents.push(ToolCommandIntent::new(
                        "graph.setKind",
                        json!({
                            "sheet_id": cand,
                            "option_id": "col.before_after",
                            "kind": GraphKind::Column,
                            "reset_series_style": true,
                        }),
                    ));
                }
            }
        }
        if let (Some(spec), Some(d)) = (&self.draft, doc)
            && self.graph_ranks
            && let AnalysisKind::TwoGroupComparison(two) = &spec.kind
            && crate::modal_predicates::r4_two_group_nonparametric_options(two.method).enabled
            && let Some(src_id) = spec.source_sheet_id.as_ref()
            && let Some(source_ds) = d.data_sheet(src_id)
            && matches!(source_ds.archetype, curvebook_doc::TableArchetype::Column(_))
        {
            let rank_title = format!("{} (Ranks)", source_ds.title);
            let base_id = format!("{}_ranks", src_id.as_str());
            let already_has_rank_graph = d.graph_sheets.iter().any(|g| {
                g.spec.primary_source_sheet_id == *src_id && (g.id.as_str() == base_id || g.title == rank_title || g.spec.title == rank_title)
            });
            if !already_has_rank_graph {
                let mut cand = base_id.clone();
                let mut suffix = 2_usize;
                while let Ok(sid) = SheetId::try_new(cand.as_str())
                    && d.sheet_section(&sid).is_some()
                {
                    cand = format!("{base_id}_{suffix}");
                    suffix += 1;
                }
                if SheetId::try_new(cand.as_str()).is_ok() {
                    intents.push(ToolCommandIntent::new(
                        "graph.create",
                        json!({
                            "sheet_id": cand,
                            "data_sheet_id": src_id.as_str(),
                            "kind": GraphKind::Column,
                            "title": rank_title,
                            "initialized": true,
                        }),
                    ));
                    intents.push(ToolCommandIntent::new(
                        "graph.setKind",
                        json!({
                            "sheet_id": cand,
                            "option_id": "col.aligned_dot",
                            "kind": GraphKind::Column,
                            "reset_series_style": true,
                        }),
                    ));
                }
            }
        }
        intents
    }
}

const LINEAR_RANGE_INPUT_PREFIX: &str = "fit.lr.range.input.";
const R_SQUARED_THRESHOLD_INPUT: &str = "fit.diagnostics.r2.input";
const LINEAR_RANGE_FIELDS: [(&str, &str); 5] = [
    ("x_min", "Ignore points where X <"),
    ("x_max", "Ignore points where X >"),
    ("plot_min", "Curve start X"),
    ("plot_max", "Curve end X"),
    ("segments", "Curve segments"),
];

fn linear_range_field_text<'a>(app: &'a crate::UiApp, field: &str) -> Option<&'a str> {
    app.interaction.text_field_buffers.get(&format!("{LINEAR_RANGE_INPUT_PREFIX}{field}")).map(|buffer| buffer.text.trim())
}

fn read_linear_range_inputs(app: &crate::UiApp, mut fit: curvebook_doc::LinearRegressionSpec) -> Result<curvebook_doc::LinearRegressionSpec, String> {
    let bound = |(field, label), current| match linear_range_field_text(app, field) {
        None => Ok(current),
        Some("") => Ok(None),
        Some(text) => crate::grid_view::parse_localized_number_input(text, &app.locale_preset)
            .map(Some)
            .ok_or_else(|| format!("Enter a finite number for “{label}”.")),
    };
    let (lo, hi) = fit.x_range_filter.unwrap_or_default();
    let [min_field, max_field, plot_min_field, plot_max_field, (segments_field, _)] = LINEAR_RANGE_FIELDS;
    let lo = bound(min_field, lo)?;
    let hi = bound(max_field, hi)?;
    if matches!((lo, hi), (Some(lo), Some(hi)) if lo > hi) {
        return Err("The lower X filter bound must be at or below the upper bound.".to_owned());
    }
    fit.x_range_filter = (lo.is_some() || hi.is_some()).then_some((lo, hi));
    fit.plot_x_min = bound(plot_min_field, fit.plot_x_min)?;
    fit.plot_x_max = bound(plot_max_field, fit.plot_x_max)?;
    if matches!((fit.plot_x_min.or(lo), fit.plot_x_max.or(hi)), (Some(start), Some(end)) if start >= end) {
        return Err("The curve start X must be strictly less than the curve end X.".to_owned());
    }
    if let Some(text) = linear_range_field_text(app, segments_field) {
        fit.plot_segments = if text.is_empty() {
            None
        } else {
            let max = curvebook_doc::LinearRegressionSpec::MAX_PLOT_SEGMENTS;
            let segments = text.parse::<usize>().ok().filter(|value| (1..=max).contains(value));
            Some(segments.ok_or_else(|| format!("Curve segments needs a whole number from 1 to {max}."))?)
        };
    }
    Ok(fit)
}

fn read_r_squared_threshold(app: &crate::UiApp, current: Option<f64>) -> Result<Option<f64>, String> {
    let Some(text) = app.interaction.text_field_buffers.get(R_SQUARED_THRESHOLD_INPUT).map(|buf| buf.text.trim()) else {
        return Ok(current);
    };
    if text.is_empty() {
        return Ok(None);
    }
    crate::grid_view::parse_localized_number_input(text, &app.locale_preset)
        .filter(|value| (0.0..=1.0).contains(value))
        .map(Some)
        .ok_or_else(|| "Enter an R² threshold from 0 to 1 or leave it blank.".to_owned())
}

/// Synchronize pending linear-regression range and nonlinear R² threshold text buffers into the active `edit_params` draft.
///
/// # Errors
/// Returns a user-facing validation message if any buffered numeric input is malformed or inverted.
pub(crate) fn sync_fit_parameter_inputs(app: &mut crate::UiApp) -> Result<(), String> {
    let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() else {
        return Ok(());
    };
    let kind = match &spec.kind {
        AnalysisKind::LinearRegression(fit) => {
            let fit = read_linear_range_inputs(app, *fit)?;
            AnalysisKind::LinearRegression(fit)
        }
        AnalysisKind::NonlinearRegression(fit) if app.interaction.text_field_buffers.contains_key(R_SQUARED_THRESHOLD_INPUT) => {
            let mut fit = fit.clone();
            fit.flag_r_squared_below = read_r_squared_threshold(app, fit.flag_r_squared_below)?;
            AnalysisKind::NonlinearRegression(fit)
        }
        _ => return Ok(()),
    };
    if let Some(draft) = app.canvas_dialogs.edit_params.draft.as_mut() {
        draft.update_kind(kind);
    }
    Ok(())
}

/// Clear transient linear-range and R² threshold text buffers when closing `EditAnalysisParameters`.
pub(crate) fn clear_fit_parameter_inputs(app: &mut crate::UiApp) {
    app.interaction.text_field_buffers.retain(|id, _| id != R_SQUARED_THRESHOLD_INPUT && !id.starts_with(LINEAR_RANGE_INPUT_PREFIX));
}
