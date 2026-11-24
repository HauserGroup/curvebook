//! `Edit Analysis Parameters...` (`Cmd+T`) dialog state machine and live preview diffing.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ContingencyMethod, DiagnosticPlotKind, FitMethod, GraphKind, MultiplicityMethod, MultivariateSpec, OneWayMethod,
    OutlierSpec, PValueFormat, ProjectDocument, RepeatedMeasuresDesign, ResidualCovariance, ResultSheet, SheetId, SurvivalAnalysisSpec,
    TransformAnalysisSpec, TwoGroupMethod,
};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::LocalePreset;

use super::analyze_dialog::{VariableRoleFilter, default_one_way_param_unpaired, default_outlier_alpha, default_outlier_fdr_q};
use super::parse_helpers::{
    TwoGroupSubmethodMemory, apply_transform_formula_str, check_pcr_target_leakage, draft_forces_reml_mode, parse_baseline_operation_str,
    parse_bool_field, parse_ci_method_choice, parse_contingency_method_name, parse_df_method_str, parse_error_prop_value, parse_math_preset_opt,
    parse_multiplicity_method_name, parse_one_way_method_str, parse_open_unit_f64, parse_open_unit_f64_with_val, parse_opt_usize_min, parse_pos_f64,
    parse_power_family_str, parse_power_solve_target_str, parse_str_field, parse_subcolumn_aggregation_val, parse_table_kind_str,
    post_hoc_method_for_widget_key, toggle_sorted_col, two_group_is_gaussian, two_group_is_paired,
};
use crate::dialogs_r9::{
    parse_cluster_distance_str, parse_cluster_linkage_str, parse_cox_tie_method_str, parse_f64_list_with_locale, parse_finite_f64_with_locale,
    parse_fit_method, parse_glm_family_str, parse_kappa_weighting, parse_opt_finite_f64_with_locale, parse_outlier_alpha,
    parse_outlier_fdr_q_percent, parse_outlier_method_str, parse_parameter_rules, parse_pvalue_stack_method_val, parse_reference_levels,
    parse_residual_covariance_str, parse_rm_design_str, parse_ss_type_str, parse_two_group_method_str, parse_u64_val, parse_usize_list,
    parse_usize_min, parse_weighting_scheme,
};

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
        }
    }
}

impl EditAnalysisParametersState {
    fn try_update_draft_kind(&mut self, mutate: impl FnOnce(&mut AnalysisKind) -> Result<bool, String>) -> Result<bool, String> {
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

    fn update_draft_kind(&mut self, mutate: impl FnOnce(&mut AnalysisKind) -> bool) {
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
        *self = Self::default();
        self.result_sheet_id = Some(result.id.clone());
        self.title_was_custom = result.analysis.custom_title.is_some();
        self.draft = Some(result.analysis.clone());
        if let AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column, .. }) = &result.analysis.kind {
            self.pca_predictors.clone_from(&result.analysis.selected_data_sets);
            self.pcr_outcome_column = *pcr_outcome_column;
        }
        self.sync_rule_flags_from_kind();
        match &result.analysis.kind {
            AnalysisKind::Contingency(cg) => {
                self.contingency_odds_ratio_checked = cg.compute_odds_and_risk_ratios;
                self.contingency_relative_risk_checked = cg.compute_odds_and_risk_ratios;
                self.contingency_attributable_risk_checked = cg.compute_odds_and_risk_ratios;
            }
            AnalysisKind::TwoGroupComparison(two) => {
                self.hodges_lehmann_ci = matches!(two.method, TwoGroupMethod::MannWhitneyU | TwoGroupMethod::WilcoxonSignedRank);
            }
            _ => {}
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
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }) => {
                self.correlation_all_pairs = true;
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

    /// Pin or clear custom title on the draft [`AnalysisSpec`].
    pub fn set_custom_title(&mut self, custom: Option<String>) -> Result<(), String> {
        let Some(spec) = &mut self.draft else {
            return Err(String::from("no active analysis draft"));
        };
        spec.set_custom_title(custom).map_err(|e| e.to_string())?;
        self.title_was_custom = spec.custom_title.is_some();
        Ok(())
    }

    /// Apply modal field updates from `ui.answer_dialog` / parameter widgets while enforcing R1–R9 predicates.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply modal field updates using regional numeric parsing (`preset`) while enforcing R1–R9 predicates.
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        const PRIORITY_KEYS: [&str; 10] = [
            "outlier_method",
            "edit.outlier.method",
            "fit_method",
            "edit.fit.method",
            "method",
            "assume_gaussian",
            "edit.assume_gaussian",
            "paired_or_matched",
            "matched",
            "edit.paired_or_matched",
        ];
        for key in PRIORITY_KEYS {
            if let Some(val) = fields.get(key) {
                self.apply_single_param_patch(key, val, preset)?;
            }
        }
        for (key, val) in fields {
            if !PRIORITY_KEYS.contains(&key.as_str()) {
                self.apply_single_param_patch(key, val, preset)?;
            }
        }
        Ok(())
    }

    fn apply_two_group_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        let two_method = self.draft.as_ref().and_then(|s| match &s.kind {
            AnalysisKind::TwoGroupComparison(two) => Some(two.method),
            _ => None,
        });
        match key {
            "welch_correction" | "edit.welch_correction" | "edit.two_group.welch" => {
                let w = parse_bool_field(val, "welch_correction")?;
                if let Some(method) = two_method {
                    if w && !crate::modal_predicates::r4_two_group_welch_correction(method).enabled {
                        return Err(String::from("Welch's correction is disabled for paired or nonparametric tests."));
                    }
                    self.welch_correction = w;
                    self.update_draft_kind(|k| match k {
                        AnalysisKind::TwoGroupComparison(two) => {
                            two.method = if w { TwoGroupMethod::WelchT } else { TwoGroupMethod::UnpairedT };
                            true
                        }
                        _ => false,
                    });
                    self.sync_rule_flags_from_kind();
                }
                Ok(true)
            }
            "graph_differences" | "edit.two_group.graph_differences" => {
                let b = parse_bool_field(val, "graph_differences")?;
                if b && two_method.is_some_and(|m| !crate::modal_predicates::r4_two_group_graph_differences(m).enabled) {
                    return Err(String::from("Estimation plot of differences is disabled in nonparametric/rank mode."));
                }
                self.graph_differences = b;
                Ok(true)
            }
            "graph_ranks" | "edit.two_group.graph_ranks" => {
                let b = parse_bool_field(val, "graph_ranks")?;
                if b && two_method.is_some_and(|m| !crate::modal_predicates::r4_two_group_nonparametric_options(m).enabled) {
                    return Err(String::from("Rank-based option is disabled in Gaussian parametric mode."));
                }
                self.graph_ranks = b;
                Ok(true)
            }
            "hodges_lehmann_ci" | "hodges_lehmann" | "edit.two_group.hodges_lehmann" => {
                let b = parse_bool_field(val, "hodges_lehmann_ci")?;
                if b && two_method.is_some_and(|m| !crate::modal_predicates::r4_two_group_nonparametric_options(m).enabled) {
                    return Err(String::from("Hodges-Lehmann CI is disabled in Gaussian parametric mode."));
                }
                self.hodges_lehmann_ci = b;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn set_draft_pcr_outcome(&mut self, outcome: Option<usize>) {
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

    fn apply_pca_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        match key {
            "selected_data_sets" | "edit.selected_data_sets" | "edit.transform.selected_data_sets" | "pca_predictors" | "edit.pca.predictors" => {
                let cols = parse_usize_list(val, key)?;
                let is_pca_key = matches!(key, "pca_predictors" | "edit.pca.predictors");
                let is_pca_draft = self.draft.as_ref().is_some_and(|s| matches!(s.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })));
                if is_pca_key || is_pca_draft {
                    self.pca_predictors.clone_from(&cols);
                    if self.pcr_outcome_column.is_some_and(|y| cols.contains(&y)) {
                        self.set_draft_pcr_outcome(None);
                    }
                }
                if let Some(spec) = &mut self.draft {
                    spec.selected_data_sets = cols;
                }
                Ok(true)
            }
            "pcr_outcome_column" | "edit.pcr.outcome_column" => {
                if let Some(col) = parse_opt_usize_min(val, key, 0)? {
                    check_pcr_target_leakage(col, self.pca_predictors.contains(&col))?;
                    self.set_draft_pcr_outcome(Some(col));
                } else {
                    self.set_draft_pcr_outcome(None);
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn apply_single_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<(), String> {
        if self.apply_contingency_param_patch(key, val, preset)?
            || self.apply_transform_param_patch(key, val)?
            || self.apply_pvalue_stack_param_patch(key, val, preset)?
            || self.apply_spline_lowess_param_patch(key, val)?
            || self.apply_power_analysis_param_patch(key, val, preset)?
            || self.apply_r9_param_patch(key, val)?
            || self.apply_two_group_param_patch(key, val)?
            || self.apply_pca_param_patch(key, val)?
        {
            return Ok(());
        }
        match key {
            "active_tab" | "tab" => {
                if let Some(t) = val.as_str() {
                    if t.eq_ignore_ascii_case("repeated_measures") && !self.paired_or_matched {
                        return Err(String::from("Repeated Measures tab is disabled when No matching is selected."));
                    }
                    self.active_tab = t.to_owned();
                }
            }
            "assume_gaussian" | "edit.assume_gaussian" => self.apply_gaussian_toggle_patch(parse_bool_field(val, "assume_gaussian")?),
            "paired_or_matched" | "matched" | "edit.paired_or_matched" => {
                let matched = parse_bool_field(val, "paired_or_matched")?;
                self.paired_or_matched = matched;
                self.apply_matched_toggle(matched);
            }
            "compare_against_control" | "edit.compare_against_control" => {
                let cmp = parse_bool_field(val, "compare_against_control")?;
                self.compare_against_control = cmp;
                if cmp {
                    self.set_anova_multiplicity(MultiplicityMethod::Dunnett { control_column: self.control_column });
                }
            }
            "control_column" | "edit.control_column" => {
                let col = val.as_u64().ok_or_else(|| String::from("`control_column` must be a non-negative integer"))? as usize;
                self.control_column = col;
                if self.compare_against_control {
                    self.set_anova_multiplicity(MultiplicityMethod::Dunnett { control_column: col });
                }
            }
            "correlation_all_pairs" | "edit.correlation_all_pairs" => {
                if let Some(b) = val.as_bool() {
                    self.correlation_all_pairs = b;
                    if !b {
                        self.correlation_heatmap = false;
                    }
                }
            }
            "correlation_heatmap" | "edit.correlation_heatmap" | "edit.correlation.heatmap" => {
                if let Some(b) = val.as_bool() {
                    if b && !self.correlation_all_pairs {
                        return Err(String::from("Correlation heatmap is disabled unless all-pairs Correlation matrix mode is active."));
                    }
                    self.correlation_heatmap = b;
                }
            }
            "p_value_format" | "edit.p_value_format" => {
                if let Some(s) = val.as_str() {
                    let next = match s.trim().to_ascii_lowercase().as_str() {
                        "apa" => PValueFormat::Apa,
                        "nejm" => PValueFormat::Nejm,
                        "custom" => PValueFormat::new_custom(4, None),
                        _ => PValueFormat::FourDecimal,
                    };
                    self.set_p_value_format(next);
                }
            }
            "p_value_decimals" | "edit.p_value_digits" | "edit.pvalue.digits" => {
                let d = u8::try_from(parse_usize_min(val, key, 0)?).map_err(|_| String::from("`p_value_decimals` out of range"))?;
                if let Some(spec) = &mut self.draft {
                    let PValueFormat::Custom { scientific_below, .. } = spec.p_value_format else {
                        return Err(String::from("Custom decimal precision spinner is disabled unless Custom P-value format is selected."));
                    };
                    spec.p_value_format = PValueFormat::new_custom(d, scientific_below);
                }
            }
            "confidence_percent" | "edit.confidence_percent" => {
                if !val.is_null() {
                    let c = parse_finite_f64_with_locale(val, key, preset)?;
                    self.set_confidence_percent(c)?;
                }
            }
            "custom_title" | "edit.custom_title" => {
                let parsed = match val {
                    Value::Null => None,
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    _ => return Err(String::from("`custom_title` must be a string or null")),
                };
                self.set_custom_title(parsed)?;
            }
            "geisser_greenhouse" | "edit.geisser_greenhouse" | "edit.sphericity.geisser_greenhouse" => {
                if !self.paired_or_matched {
                    return Err(String::from("Geisser-Greenhouse sphericity control is disabled when No matching is selected."));
                }
                self.try_update_draft_kind(|k| match k {
                    AnalysisKind::OneWayAnova(a) => {
                        if a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured {
                            return Err(String::from("Sphericity corrections are disabled when Mixed-Effects REML is active."));
                        }
                        if let Some(gg) = val.as_bool() {
                            a.method = OneWayMethod::RepeatedMeasures { geisser_greenhouse: gg };
                            Ok(true)
                        } else {
                            Ok(false)
                        }
                    }
                    _ => Ok(false),
                })?;
            }
            "multiplicity_method" => {
                if let Some(raw) = val.as_str() {
                    let method = parse_multiplicity_method_name(raw, self.control_column)?;
                    self.apply_checked_post_hoc_patch(method, "Post-hoc method disabled.")?;
                }
            }
            key if key.starts_with("edit.post_hoc.") || key.starts_with("edit.posthoc.") => {
                let method = post_hoc_method_for_widget_key(key, self.control_column)
                    .ok_or_else(|| format!("unknown parameter field `{key}` in EditAnalysisParameters"))?;
                if val.as_bool().unwrap_or(true) {
                    self.apply_checked_post_hoc_patch(method, "Post-hoc method is disabled.")?;
                }
            }
            "anova_method" | "one_way_method" | "edit.anova_method" | "edit.one_way_method" => {
                let next_method = parse_one_way_method_str(parse_str_field(val, "anova_method")?, self.saved_rm_geisser_greenhouse)?;
                if let Some(mut spec) = self.draft.take() {
                    self.remember_submethods_from_kind(&spec.kind);
                    if let AnalysisKind::OneWayAnova(mut a) = spec.kind.clone() {
                        a.method = next_method;
                        spec.update_kind(AnalysisKind::OneWayAnova(a));
                    }
                    self.draft = Some(spec);
                    self.sync_rule_flags_from_kind();
                }
            }
            other => return Err(format!("unknown Edit Analysis Parameters field `{other}`")),
        }
        Ok(())
    }

    /// Applies Contingency-specific field patches on `EditAnalysisParametersState`.
    fn apply_contingency_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
        let method_flag = match key {
            "fisher" | "edit.contingency.fisher" => Some(ContingencyMethod::FisherExact),
            "barnard" | "edit.contingency.barnard" | "edit.contingency.barnard_exact" => Some(ContingencyMethod::BarnardExact),
            "likelihood_ratio_g2" | "g2" | "edit.contingency.g2" | "edit.contingency.likelihood_g2" => Some(ContingencyMethod::LikelihoodRatioG2),
            "mcnemar" | "edit.contingency.mcnemar" => Some(ContingencyMethod::McNemarPaired),
            "cochran_armitage" | "edit.contingency.cochran_armitage" => Some(ContingencyMethod::CochranArmitageTrend),
            "cohen_kappa" | "edit.contingency.cohen_kappa" => {
                Some(ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Unweighted })
            }
            "fleiss_kappa" | "edit.contingency.fleiss_kappa" => Some(ContingencyMethod::FleissKappa),
            "exact_binomial" | "edit.contingency.exact_binomial" => Some(ContingencyMethod::ExactBinomial),
            _ => None,
        };
        if let Some(target_method) = method_flag {
            if let Some(b) = val.as_bool() {
                let next = if b { target_method } else { ContingencyMethod::PearsonChiSquare { yates_continuity: false } };
                self.update_contingency_spec(|c| c.method = next);
            }
            return Ok(true);
        }
        match key {
            "kappa_weighting" | "edit.contingency.kappa_weighting" => {
                let w = parse_kappa_weighting(val)?;
                self.update_contingency_spec(|c| c.method = ContingencyMethod::CohenKappa { weighting: w });
            }
            "weighting" if self.draft.as_ref().is_some_and(|s| matches!(s.kind, AnalysisKind::Contingency(_))) => {
                let w = parse_kappa_weighting(val)?;
                self.update_contingency_spec(|c| c.method = ContingencyMethod::CohenKappa { weighting: w });
            }
            "null_proportion" | "edit.contingency.null_proportion" => {
                let p = parse_opt_finite_f64_with_locale(val, key, preset)?;
                if let Some(v) = p
                    && !(0.0 < v && v < 1.0)
                {
                    return Err(format!("`null_proportion` ({v}) must be strictly between 0 and 1"));
                }
                self.update_contingency_spec(|c| c.null_proportion = p);
            }
            "expected_proportions" | "edit.contingency.expected_proportions" => {
                let props = if val.is_null() { None } else { Some(parse_f64_list_with_locale(val, key, preset)?) };
                self.update_contingency_spec(|c| c.expected_proportions = props);
            }
            "edit.contingency.ci.baptista_pike" => {
                self.contingency_or_ci_method = parse_ci_method_choice(val, key, "OR", ["Baptista-Pike", "Woolf"])?;
            }
            "edit.contingency.ci.koopman" => {
                self.contingency_rr_ci_method = parse_ci_method_choice(val, key, "RR", ["Koopman", "Katz"])?;
            }
            "edit.contingency.ci.newcombe_wilson" => {
                self.contingency_arr_ci_method = parse_ci_method_choice(val, key, "ARR", ["Newcombe-Wilson", "Wald"])?;
            }
            "edit.contingency.ci.wilson_brown" => {
                self.contingency_diag_ci_method = parse_ci_method_choice(val, key, "diagnostic", ["Wilson-Brown", "Clopper-Pearson"])?;
            }
            "compute_odds_and_risk_ratios" | "contingency_odds_ratio_checked" | "edit.contingency.odds_ratio" => {
                let b = parse_bool_field(val, key)?;
                self.contingency_odds_ratio_checked = b;
                self.contingency_relative_risk_checked = b;
                if !b {
                    self.contingency_attributable_risk_checked = false;
                }
                self.update_contingency_spec(|c| c.compute_odds_and_risk_ratios = b);
            }
            "contingency_attributable_risk_checked" | "attributable_risk" | "compute_attributable_risk" | "edit.contingency.attributable_risk" => {
                let b = parse_bool_field(val, key)?;
                self.contingency_attributable_risk_checked = b;
                if b {
                    self.update_contingency_spec(|c| c.compute_odds_and_risk_ratios = true);
                } else if !self.contingency_odds_ratio_checked && !self.contingency_relative_risk_checked {
                    self.update_contingency_spec(|c| c.compute_odds_and_risk_ratios = false);
                }
            }
            "compute_sensitivity_specificity" | "sensitivity_specificity" | "edit.contingency.sensitivity_specificity" => {
                let b = parse_bool_field(val, key)?;
                self.update_contingency_spec(|c| c.compute_sensitivity_specificity = b);
            }
            "contingency_method" => {
                if let Some(raw) = val.as_str() {
                    let method = parse_contingency_method_name(raw)?;
                    self.update_contingency_spec(|c| c.method = method);
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn update_contingency_spec(&mut self, mutate: impl FnOnce(&mut curvebook_doc::ContingencySpec)) {
        self.update_draft_kind(|k| match k {
            AnalysisKind::Contingency(c) => {
                mutate(c);
                true
            }
            _ => false,
        });
    }

    /// Applies `PValueStack`-specific field patches on `EditAnalysisParametersState`.
    fn apply_pvalue_stack_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
        self.try_update_draft_kind(|k| {
            let AnalysisKind::PValueStack(ps) = k else {
                return Ok(false);
            };
            match key {
                "method"
                | "pvalue_stack_method"
                | "multiplicity_method"
                | "multiplicity"
                | "edit.pvalue_stack.method"
                | "edit.pvalue_stack.multiplicity_method" => ps.method = parse_pvalue_stack_method_val(val)?,
                "threshold" | "pvalue_stack_threshold" | "alpha" | "q" | "fdr_q" | "edit.pvalue_stack.threshold" => {
                    ps.threshold = parse_open_unit_f64_with_val(val, key, preset)?;
                }
                "column" | "pvalue_stack_column" | "edit.pvalue_stack.column" => ps.column = parse_opt_usize_min(val, key, 0)?,
                _ => return Ok(false),
            }
            Ok(true)
        })
    }

    /// Applies `FitSplineLowess`-specific field patches on `EditAnalysisParametersState`.
    fn apply_spline_lowess_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        self.try_update_draft_kind(|k| {
            let AnalysisKind::FitSplineLowess(sp) = k else {
                return Ok(false);
            };
            match key {
                "method" | "spline_method" | "edit.spline.method" => {
                    sp.method = serde_json::from_value(val.clone()).map_err(|e| format!("invalid spline/LOWESS method: {e}"))?;
                }
                "segment_count" | "segments" | "edit.spline.segments" => {
                    if let Some(n) = parse_opt_usize_min(val, key, curvebook_doc::SplineLowessSpec::MIN_SEGMENTS)? {
                        sp.segment_count = n.min(curvebook_doc::SplineLowessSpec::MAX_SEGMENTS);
                    }
                }
                "smoothing_knots" | "knots" | "edit.spline.knots" => {
                    sp.smoothing_knots = parse_opt_usize_min(val, key, 2)?;
                }
                "smoothing_df" | "df" => {
                    sp.smoothing_df = if val.is_null() {
                        None
                    } else {
                        let v = val.as_f64().ok_or_else(|| format!("`{key}` must be a finite number >= 2"))?;
                        if !v.is_finite() || v < 2.0 {
                            return Err(format!("`{key}` must be finite and >= 2"));
                        }
                        Some(v)
                    };
                }
                "lowess_preset" | "edit.spline.lowess_preset" => {
                    sp.lowess_preset = serde_json::from_value(val.clone()).map_err(|e| format!("invalid LOWESS preset: {e}"))?;
                }
                "lowess_fraction" | "span" => {
                    sp.lowess_fraction = if val.is_null() {
                        None
                    } else {
                        let v = val.as_f64().ok_or_else(|| format!("`{key}` must be a finite number in (0, 1]"))?;
                        if !v.is_finite() || v <= 0.0 || v > 1.0 {
                            return Err(format!("`{key}` must be finite and in (0, 1]"));
                        }
                        Some(v)
                    };
                }
                "lowess_robustness_iters" | "edit.spline.lowess_iters" => {
                    if let Some(n) = parse_opt_usize_min(val, key, 0)? {
                        sp.lowess_robustness_iters = n.min(curvebook_doc::SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS);
                    }
                }
                "emit_curve_coordinates_tab" | "edit.spline.emit_coords" => {
                    if let Some(b) = val.as_bool() {
                        sp.emit_curve_coordinates_tab = b;
                    }
                }
                "interpolate_unknowns" | "edit.spline.interpolate_unknowns" => {
                    if let Some(b) = val.as_bool() {
                        sp.interpolate_unknowns = b;
                    }
                }
                _ => return Ok(false),
            }
            Ok(true)
        })
    }

    /// Applies `PowerAnalysis`-specific field patches on `EditAnalysisParametersState`.
    fn apply_power_analysis_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
        self.try_update_draft_kind(|k| {
            let AnalysisKind::PowerAnalysis(pa) = k else {
                return Ok(false);
            };
            match key {
                "edit.power.family" | "power.family" => pa.family = parse_power_family_str(parse_str_field(val, key)?)?,
                "edit.power.solve_target" | "power.solve_target" => pa.solve_target = parse_power_solve_target_str(parse_str_field(val, key)?)?,
                "edit.power.alpha" => pa.alpha = parse_open_unit_f64(val, key, preset)?,
                "edit.power.target_power" => {
                    pa.target_power = if val.is_null() { None } else { Some(parse_open_unit_f64(val, key, preset)?) };
                }
                "edit.power.sample_size" => {
                    pa.sample_size = if val.is_null() { None } else { Some(parse_finite_f64_with_locale(val, key, preset)?) };
                }
                "edit.power.sample_size_group2" => {
                    pa.sample_size_group2 = if val.is_null() { None } else { Some(parse_finite_f64_with_locale(val, key, preset)?) };
                }
                "edit.power.allocation_ratio" => pa.allocation_ratio = parse_pos_f64(val, key, preset)?,
                "edit.power.two_tailed" => {
                    pa.tail_mode = if parse_bool_field(val, key)? {
                        curvebook_doc::PowerTailMode::TwoSided
                    } else {
                        curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Greater)
                    };
                }
                "edit.power.continuity_correction" => pa.continuity_correction = parse_bool_field(val, key)?,
                "edit.power.chi_square_df" => pa.chi_square_df = parse_pos_f64(val, key, preset)?,
                _ => return Ok(false),
            }
            Ok(true)
        })
    }

    /// Applies Transform-specific field patches on `EditAnalysisParametersState`.
    fn apply_transform_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        match key {
            "x_transform" | "edit.transform.x_transform" => {
                let parsed = parse_math_preset_opt(val)?;
                self.update_draft_kind(|k| match k {
                    AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, .. }) => {
                        *x_transform = parsed;
                        true
                    }
                    _ => false,
                });
            }
            "y_transform" | "edit.transform.y_transform" => {
                let parsed = parse_math_preset_opt(val)?;
                self.update_draft_kind(|k| match k {
                    AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, .. }) => {
                        *y_transform = parsed;
                        true
                    }
                    _ => false,
                });
            }
            "error_propagation" | "edit.transform.error_propagation" => {
                let ep_mode = parse_error_prop_value(val)?;
                self.update_draft_kind(|k| match k {
                    AnalysisKind::Transform(TransformAnalysisSpec::Math { error_propagation, .. }) => {
                        *error_propagation = ep_mode;
                        true
                    }
                    _ => false,
                });
            }
            "formula" | "transform_formula" | "edit.transform.formula" | "filter_formula" | "edit.transform.filter_formula" => {
                self.try_update_draft_kind(|k| match k {
                    AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, .. }) => {
                        if let Some(raw) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                            apply_transform_formula_str(raw, x_transform, y_transform)?;
                            Ok(true)
                        } else {
                            Ok(false)
                        }
                    }
                    AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, .. }) => {
                        *filter_formula = if val.is_null() {
                            None
                        } else {
                            let s = val.as_str().ok_or_else(|| String::from("`filter_formula` must be a string or null"))?.trim();
                            (!s.is_empty()).then(|| s.to_owned())
                        };
                        Ok(true)
                    }
                    _ => Ok(false),
                })?;
            }
            "target_archetype" | "edit.transform.target_archetype" => {
                let parsed = parse_table_kind_str(parse_str_field(val, "target_archetype")?)?;
                self.update_draft_kind(|k| match k {
                    AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, .. }) => {
                        *target_archetype = parsed;
                        true
                    }
                    _ => false,
                });
            }
            "baseline_columns" | "edit.transform.baseline_columns" => {
                let cols = parse_usize_list(val, key)?;
                self.update_draft_kind(|k| match k {
                    AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, .. }) => {
                        *baseline_columns = cols;
                        true
                    }
                    _ => false,
                });
            }
            "baseline_operation" | "operation" | "edit.transform.baseline_operation" => {
                let op = parse_baseline_operation_str(parse_str_field(val, "baseline_operation")?)?;
                self.update_draft_kind(|k| match k {
                    AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { operation, .. }) => {
                        *operation = op;
                        true
                    }
                    _ => false,
                });
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn apply_gaussian_toggle_patch(&mut self, assume_gaussian: bool) {
        self.assume_gaussian = assume_gaussian;
        let matched = self.paired_or_matched;
        let ctrl = self.control_column;
        let Some(mut spec) = self.draft.take() else {
            return;
        };
        self.remember_submethods_from_kind(&spec.kind);
        let mut k = spec.kind.clone();
        crate::modal_predicates::apply_r2_gaussian_toggle(&mut k, assume_gaussian, matched, ctrl);
        match &mut k {
            AnalysisKind::OneWayAnova(a) => {
                if assume_gaussian {
                    a.method = if matched {
                        OneWayMethod::RepeatedMeasures { geisser_greenhouse: self.saved_rm_geisser_greenhouse }
                    } else {
                        self.saved_one_way_param_unpaired
                    };
                }
            }
            AnalysisKind::TwoGroupComparison(t) => {
                t.method = self.two_group_memory.resolve(assume_gaussian, matched, Some(t.method));
            }
            _ => {}
        }
        spec.update_kind(k);
        self.draft = Some(spec);
        self.sync_rule_flags_from_kind();
    }

    fn apply_matched_toggle(&mut self, matched: bool) {
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

    fn apply_checked_post_hoc_patch(&mut self, method: MultiplicityMethod, fallback_msg: &str) -> Result<(), String> {
        let draft_reml = self.draft.as_ref().is_some_and(draft_forces_reml_mode);
        let gate = crate::modal_predicates::r1b_post_hoc_option_enabled(self.assume_gaussian, draft_reml, method);
        if !gate.enabled {
            return Err(gate.disabled_reason.unwrap_or_else(|| String::from(fallback_msg)));
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
                _ => {}
            }
            true
        });
        if self.draft.is_some() {
            self.sync_rule_flags_from_kind();
        }
    }

    fn patch_outlier_kind(&mut self, key: &str, val: &Value, out: &mut OutlierSpec) -> Result<bool, String> {
        match key {
            "outlier_method" | "edit.outlier.method" | "method" => {
                match *out {
                    OutlierSpec::Rout { fdr_q_percent } => self.saved_outlier_rout_q = fdr_q_percent,
                    OutlierSpec::Grubbs { alpha, iterative } => {
                        self.saved_outlier_grubbs_alpha = alpha;
                        self.saved_outlier_grubbs_iterative = iterative;
                    }
                }
                let (mode, iter_opt) = parse_outlier_method_str(parse_str_field(val, "outlier_method")?)?;
                *out = if mode == "grubbs" {
                    OutlierSpec::Grubbs { alpha: self.saved_outlier_grubbs_alpha, iterative: iter_opt.unwrap_or(self.saved_outlier_grubbs_iterative) }
                } else {
                    OutlierSpec::Rout { fdr_q_percent: self.saved_outlier_rout_q }
                };
            }
            "fdr_q_percent" | "q_percent" | "rout_q" | "outlier_fdr_q_percent" | "edit.outlier.fdr_q_percent" | "edit.fit.rout_q" => {
                let q = parse_outlier_fdr_q_percent(val)?;
                self.saved_outlier_rout_q = q;
                let OutlierSpec::Rout { fdr_q_percent } = out else {
                    return Err(String::from("`fdr_q_percent` requires ROUT outlier detection mode"));
                };
                *fdr_q_percent = q;
            }
            "outlier_alpha" | "edit.outlier.alpha" | "alpha" => {
                let a = parse_outlier_alpha(val)?;
                self.saved_outlier_grubbs_alpha = a;
                let OutlierSpec::Grubbs { alpha, .. } = out else {
                    return Err(String::from("`alpha` for Grubbs requires Grubbs outlier detection mode"));
                };
                *alpha = a;
            }
            "iterative" | "outlier_iterative" | "edit.outlier.iterative" => {
                let it = parse_bool_field(val, "iterative")?;
                self.saved_outlier_grubbs_iterative = it;
                let OutlierSpec::Grubbs { iterative, .. } = out else {
                    return Err(String::from("`iterative` requires Grubbs outlier detection mode"));
                };
                *iterative = it;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn apply_r9_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        let Some(mut spec) = self.draft.take() else {
            return Ok(false);
        };
        let mut updated = spec.kind.clone();
        let ctrl = self.control_column;
        let res = match &mut updated {
            AnalysisKind::OutlierDetection(out) => self.patch_outlier_kind(key, val, out),
            _ => patch_r9_non_outlier_kind(key, val, ctrl, &mut updated),
        };
        let handled = match res {
            Ok(h) => h,
            Err(e) => {
                self.draft = Some(spec);
                return Err(e);
            }
        };
        if handled {
            spec.update_kind(updated);
        }
        self.draft = Some(spec);
        if handled {
            self.sync_rule_flags_from_kind();
        }
        Ok(handled)
    }

    /// Effective displayed title reflecting auto-title vs custom title rules.
    #[must_use]
    pub fn preview_title(&self) -> Option<&str> {
        self.draft.as_ref().map(AnalysisSpec::effective_title)
    }

    /// Emit declarative `analysis.updateParams` intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.result_sheet_id.as_ref()?;
        let spec = self.draft.as_ref()?;
        let mut payload = json!({
            "sheet_id": id.as_str(),
            "kind": spec.kind,
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

    /// Emit `analysis.updateParams` and, when `correlation_heatmap` is toggled on for a correlation matrix,
    /// spawn a linked `DiagnosticPlotKind::Heatmap` `GraphSheet` if none already points to `result_sheet_id`.
    #[must_use]
    pub fn commit_intents(&self, doc: Option<&ProjectDocument>) -> Vec<ToolCommandIntent> {
        let Some(primary) = self.commit_intent() else {
            return Vec::new();
        };
        let mut intents = vec![primary];
        if let (Some(id), Some(spec)) = (&self.result_sheet_id, &self.draft)
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
            && let Some(source_ds) = d.data_sheet(&spec.source_sheet_id)
            && matches!(source_ds.archetype, curvebook_doc::TableArchetype::Column(_))
        {
            let already_has_before_after = d.graph_sheets.iter().any(|g| {
                g.spec.primary_source_sheet_id == spec.source_sheet_id
                    && g.spec.series.iter().any(|s| matches!(s.gdo, curvebook_doc::GraphicalDataObject::BeforeAfterLines { .. }))
            });
            if !already_has_before_after {
                let base_id = format!("{}_diff", spec.source_sheet_id.as_str());
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
                            "data_sheet_id": spec.source_sheet_id.as_str(),
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
        intents
    }
}

fn patch_r9_non_outlier_kind(key: &str, val: &Value, control_col: usize, updated: &mut AnalysisKind) -> Result<bool, String> {
    match updated {
        AnalysisKind::DescriptiveStats(desc) => match key {
            "descriptive_subcolumn_agg" | "subcolumn_agg" | "edit.descriptive.subcolumn_agg" => {
                desc.aggregation = parse_subcolumn_aggregation_val(val)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        AnalysisKind::TwoGroupComparison(t) => patch_two_group_r9(key, val, t),
        AnalysisKind::OneWayAnova(a) => patch_one_way_r9(key, val, a),
        AnalysisKind::FactorialAnova(f) => patch_factorial_r9(key, val, control_col, f),
        AnalysisKind::NestedComparison(n) => match key {
            "df_method" | "edit.rm.df_method" => {
                n.df_method = parse_df_method_str(parse_str_field(val, "df_method")?)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        AnalysisKind::Multivariate(mv) => patch_multivariate_r9(key, val, mv),
        AnalysisKind::GeneralizedLinearModel(glm) => patch_glm_r9(key, val, glm),
        AnalysisKind::Survival(surv) => patch_survival_r9(key, val, control_col, surv),
        AnalysisKind::NonlinearRegression(fit) => patch_nonlinear_r9(key, val, fit),
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { sample_size, seed, stratify_column, proportional_allocation }) => match key {
            "sample_size" | "edit.sample.sample_size" => {
                *sample_size = parse_usize_min(val, key, 1)?;
                Ok(true)
            }
            "seed" | "edit.sample.seed" => {
                *seed = parse_u64_val(val, key)?;
                Ok(true)
            }
            "stratify_column" | "edit.sample.stratify_column" => {
                *stratify_column = parse_opt_usize_min(val, key, 0)?;
                Ok(true)
            }
            "proportional_allocation" | "edit.sample.proportional_allocation" => {
                *proportional_allocation = parse_bool_field(val, "proportional_allocation")?;
                Ok(true)
            }
            _ => Ok(false),
        },
        _ => Ok(false),
    }
}

fn patch_two_group_r9(key: &str, val: &Value, t: &mut curvebook_doc::TwoGroupSpec) -> Result<bool, String> {
    match key {
        "two_group_method" | "edit.two_group.method" => {
            let hyp = match t.method {
                TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => hypothetical,
                _ => 0.0,
            };
            t.method = parse_two_group_method_str(parse_str_field(val, "two_group_method")?, hyp)?;
        }
        "paired_sign_test" | "edit.two_group.paired_sign_test" => {
            t.method = if parse_bool_field(val, "paired_sign_test")? { TwoGroupMethod::PairedSignTest } else { TwoGroupMethod::WilcoxonSignedRank };
        }
        "two_tailed" | "edit.two_group.two_tailed" => t.two_tailed = parse_bool_field(val, "two_tailed")?,
        "compute_effect_sizes" | "edit.compute_effect_sizes" => t.compute_effect_sizes = parse_bool_field(val, "compute_effect_sizes")?,
        "mv_group_column" | "edit.two_group.mv_group_column" => t.mv_group_column = parse_opt_usize_min(val, key, 0)?,
        "mv_response_column" | "edit.two_group.mv_response_column" => t.mv_response_column = parse_opt_usize_min(val, key, 0)?,
        _ => {
            if let Some(col) = key.strip_prefix("edit.two_group.group.").and_then(|s| s.parse::<usize>().ok()) {
                t.mv_group_column = Some(col);
            } else if let Some(col) = key.strip_prefix("edit.two_group.resp.").and_then(|s| s.parse::<usize>().ok()) {
                t.mv_response_column = Some(col);
            } else {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn patch_one_way_r9(key: &str, val: &Value, a: &mut curvebook_doc::OneWayAnovaSpec) -> Result<bool, String> {
    match key {
        "mixed_effects_reml" | "edit.rm.mixed_effects_reml" => {
            let b = parse_bool_field(val, "mixed_effects_reml")?;
            a.mixed_effects_reml = b;
            if b && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(a.multiplicity.method) {
                a.multiplicity.method = MultiplicityMethod::Bonferroni;
            }
        }
        "df_method" | "edit.rm.df_method" => a.df_method = parse_df_method_str(parse_str_field(val, "df_method")?)?,
        "residual_covariance" | "edit.rm.residual_covariance" | "edit.reml.residual_covariance" => {
            a.residual_covariance = parse_residual_covariance_str(parse_str_field(val, "residual_covariance")?)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_factorial_r9(key: &str, val: &Value, control_col: usize, f: &mut curvebook_doc::FactorialAnovaSpec) -> Result<bool, String> {
    match key {
        "compute_effect_sizes" | "edit.compute_effect_sizes" => f.compute_effect_sizes = parse_bool_field(val, "compute_effect_sizes")?,
        "three_way" | "edit.factorial.three_way" => {
            f.three_way = parse_bool_field(val, "three_way")?;
            f.max_interaction_order = if f.three_way { f.max_interaction_order.max(3) } else { f.max_interaction_order.min(2) };
        }
        "repeated_measures" | "edit.factorial.repeated_measures" => {
            f.repeated_measures = parse_rm_design_str(parse_str_field(val, "repeated_measures")?)?;
        }
        "mixed_effects_reml" | "edit.rm.mixed_effects_reml" => {
            let b = parse_bool_field(val, "mixed_effects_reml")?;
            f.mixed_effects_reml = b;
            if b && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(f.multiplicity.method) {
                f.multiplicity.method = MultiplicityMethod::Bonferroni;
            }
        }
        "df_method" | "edit.rm.df_method" => f.df_method = parse_df_method_str(parse_str_field(val, "df_method")?)?,
        "residual_covariance" | "factorial_residual_covariance" | "edit.factorial.residual_covariance" | "edit.reml.residual_covariance" => {
            f.residual_covariance = parse_residual_covariance_str(parse_str_field(val, "residual_covariance")?)?;
        }
        "ss_type" | "sum_of_squares" | "edit.factorial.ss_type" => f.ss_type = parse_ss_type_str(parse_str_field(val, "ss_type")?)?,
        "max_interaction_order" | "edit.factorial.max_interaction_order" => f.max_interaction_order = parse_usize_min(val, key, 1)?,
        "fit_reml_on_missing" | "edit.factorial.fit_reml_on_missing" => f.fit_reml_on_missing = parse_bool_field(val, "fit_reml_on_missing")?,
        "multifactor_columns" | "edit.factorial.multifactor_columns" => f.multifactor_columns = parse_usize_list(val, key)?,
        "factorial_multiplicity" | "edit.factorial.multiplicity" => {
            f.multiplicity.method = parse_multiplicity_method_name(parse_str_field(val, "factorial.multiplicity")?, control_col)?;
        }
        "response_column" | "factorial_response_column" | "edit.factorial.response_column" => f.response_column = parse_opt_usize_min(val, key, 0)?,
        _ => {
            if let Some(col) = key.strip_prefix("edit.fac.resp.").and_then(|s| s.parse::<usize>().ok()) {
                f.response_column = Some(col);
            } else if let Some(col) = key.strip_prefix("edit.fac.col.").and_then(|s| s.parse::<usize>().ok()) {
                let want_on = val.as_bool().unwrap_or_else(|| !f.multifactor_columns.contains(&col));
                if want_on {
                    if !f.multifactor_columns.contains(&col) {
                        f.multifactor_columns.push(col);
                        f.multifactor_columns.sort_unstable();
                    }
                } else {
                    f.multifactor_columns.retain(|&c| c != col);
                }
            } else {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn patch_multivariate_r9(key: &str, val: &Value, mv: &mut MultivariateSpec) -> Result<bool, String> {
    match (key, mv) {
        ("distance" | "cluster_distance" | "edit.cluster.distance", MultivariateSpec::Clustering { distance, .. }) => {
            *distance = parse_cluster_distance_str(parse_str_field(val, "distance")?)?;
        }
        ("linkage" | "cluster_linkage" | "edit.cluster.linkage", MultivariateSpec::Clustering { linkage, .. }) => {
            *linkage = parse_cluster_linkage_str(parse_str_field(val, "linkage")?)?;
        }
        ("k_means_k" | "k" | "edit.cluster.k_means_k", MultivariateSpec::Clustering { k_means_k, .. }) => {
            *k_means_k = parse_opt_usize_min(val, key, 1)?;
        }
        ("k_means_auto" | "edit.cluster.k_means_auto", MultivariateSpec::Clustering { k_means_k, .. }) => {
            *k_means_k = if parse_bool_field(val, "k_means_auto")? { None } else { Some(k_means_k.unwrap_or(3)) };
        }
        ("k_sweep_max" | "edit.cluster.k_sweep_max", MultivariateSpec::Clustering { k_sweep_max, .. }) => {
            *k_sweep_max = parse_opt_usize_min(val, key, 2)?;
        }
        ("standardize" | "edit.pca.standardize", MultivariateSpec::Pca { standardize, .. }) => {
            *standardize = parse_bool_field(val, "standardize")?;
        }
        ("parallel_analysis_seed" | "edit.pca.parallel_analysis_seed", MultivariateSpec::Pca { parallel_analysis_seed, .. }) => {
            *parallel_analysis_seed = if val.is_null() { None } else { Some(parse_u64_val(val, key)?) };
        }
        ("spearman_rank" | "edit.corr.spearman_rank" | "edit.correlation.spearman_rank", MultivariateSpec::CorrelationMatrix { spearman_rank }) => {
            *spearman_rank = parse_bool_field(val, "spearman_rank")?;
        }
        ("imputations_m" | "m" | "edit.mi.imputations_m" | "edit.imputation.m", MultivariateSpec::MultipleImputation { imputations_m, .. }) => {
            *imputations_m = parse_usize_min(val, key, 1)?;
        }
        ("max_iters" | "edit.mi.max_iters", MultivariateSpec::MultipleImputation { max_iters, .. }) => {
            *max_iters = parse_usize_min(val, key, 1)?;
        }
        ("seed" | "edit.mi.seed", MultivariateSpec::MultipleImputation { seed, .. }) => *seed = parse_u64_val(val, key)?,
        ("response_column" | "edit.mi.response_column", MultivariateSpec::MultipleImputation { response_column, .. }) => {
            *response_column = parse_opt_usize_min(val, key, 0)?;
        }
        ("predictor_columns" | "edit.mi.predictor_columns", MultivariateSpec::MultipleImputation { predictor_columns, .. }) => {
            *predictor_columns = parse_usize_list(val, key)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_glm_r9(key: &str, val: &Value, glm: &mut curvebook_doc::GlmSpec) -> Result<bool, String> {
    match key {
        "family" | "glm_family" | "edit.glm.family" => glm.family = parse_glm_family_str(parse_str_field(val, "family")?)?,
        "response_column" | "edit.glm.response_column" => glm.response_column = parse_usize_min(val, key, 0)?,
        "predictor_columns" | "edit.glm.predictor_columns" => glm.predictor_columns = parse_usize_list(val, key)?,
        "reference_levels" | "edit.glm.reference_levels" => glm.reference_levels = parse_reference_levels(val)?,
        "compute_vif" | "edit.glm.compute_vif" => glm.compute_vif = parse_bool_field(val, "compute_vif")?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_survival_r9(key: &str, val: &Value, control_col: usize, surv: &mut SurvivalAnalysisSpec) -> Result<bool, String> {
    match (key, surv) {
        ("time_column" | "edit.cox.time_column", SurvivalAnalysisSpec::CoxProportionalHazards { time_column, .. }) => {
            *time_column = parse_usize_min(val, key, 0)?;
        }
        ("status_column" | "edit.cox.status_column", SurvivalAnalysisSpec::CoxProportionalHazards { status_column, .. }) => {
            *status_column = parse_usize_min(val, key, 0)?;
        }
        ("predictor_columns" | "edit.cox.predictor_columns", SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns, .. }) => {
            *predictor_columns = parse_usize_list(val, key)?;
        }
        ("reference_levels" | "edit.cox.reference_levels", SurvivalAnalysisSpec::CoxProportionalHazards { reference_levels, .. }) => {
            *reference_levels = parse_reference_levels(val)?;
        }
        ("tie_method" | "edit.cox.tie_method", SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. }) => {
            *tie_method = parse_cox_tie_method_str(parse_str_field(val, "tie_method")?)?;
        }
        ("log_rank_mantel_cox" | "edit.km.log_rank_mantel_cox", SurvivalAnalysisSpec::KaplanMeier { log_rank_mantel_cox, .. }) => {
            *log_rank_mantel_cox = parse_bool_field(val, "log_rank_mantel_cox")?;
        }
        ("gehan_breslow_wilcoxon" | "edit.km.gehan_breslow_wilcoxon", SurvivalAnalysisSpec::KaplanMeier { gehan_breslow_wilcoxon, .. }) => {
            *gehan_breslow_wilcoxon = parse_bool_field(val, "gehan_breslow_wilcoxon")?;
        }
        (
            "trend_test" | "edit.km.trend_test" | "log_rank_for_trend" | "edit.km.log_rank_for_trend",
            SurvivalAnalysisSpec::KaplanMeier { trend_test, .. },
        ) => {
            *trend_test = parse_bool_field(val, "trend_test")?;
        }
        ("multiplicity_method" | "edit.km.multiplicity_method", SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => {
            multiplicity.method = parse_multiplicity_method_name(parse_str_field(val, "multiplicity_method")?, control_col)?;
        }
        ("asymmetrical_greenwood_ci" | "edit.km.asymmetrical_greenwood_ci", SurvivalAnalysisSpec::KaplanMeier { asymmetrical_greenwood_ci, .. }) => {
            *asymmetrical_greenwood_ci = parse_bool_field(val, "asymmetrical_greenwood_ci")?;
        }
        ("mv_time_column" | "edit.km.mv_time_column", SurvivalAnalysisSpec::KaplanMeier { mv_time_column, .. }) => {
            *mv_time_column = parse_opt_usize_min(val, key, 0)?;
        }
        ("mv_status_column" | "edit.km.mv_status_column", SurvivalAnalysisSpec::KaplanMeier { mv_status_column, .. }) => {
            *mv_status_column = parse_opt_usize_min(val, key, 0)?;
        }
        ("mv_group_column" | "edit.km.mv_group_column", SurvivalAnalysisSpec::KaplanMeier { mv_group_column, .. }) => {
            *mv_group_column = parse_opt_usize_min(val, key, 0)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_nonlinear_r9(key: &str, val: &Value, fit: &mut curvebook_doc::NonlinearFitSpec) -> Result<bool, String> {
    match key {
        "equation_id" | "edit.fit.equation_id" | "edit.fit.model_id" => {
            let raw = parse_str_field(val, "equation_id")?.trim();
            if !raw.is_empty() {
                fit.equation_id = raw.to_owned();
            }
        }
        "custom_dsl" | "edit.fit.custom_dsl" => fit.custom_dsl = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(ToOwned::to_owned),
        "weighting" | "fit_weighting" | "edit.fit.weighting" => fit.weighting = parse_weighting_scheme(val)?,
        "fit_method" | "edit.fit.method" | "method" => fit.method = parse_fit_method(val)?,
        "rout_q" | "fdr_q_percent" | "q_percent" | "edit.fit.rout_q" => {
            let q = parse_outlier_fdr_q_percent(val)?;
            let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut fit.method else {
                return Err(String::from("`rout_q` requires ROUT outlier elimination fit method"));
            };
            *fdr_q_percent = q;
        }
        "parameter_rules" | "edit.fit.parameter_rules" => fit.parameter_rules = parse_parameter_rules(val)?,
        "profile_likelihood_ci" | "edit.fit.profile_likelihood_ci" => fit.profile_likelihood_ci = parse_bool_field(val, "profile_likelihood_ci")?,
        "confidence_bands" | "edit.fit.confidence_bands" => fit.confidence_bands = parse_bool_field(val, "confidence_bands")?,
        "prediction_bands" | "edit.fit.prediction_bands" => fit.prediction_bands = parse_bool_field(val, "prediction_bands")?,
        "homoscedasticity_test" | "edit.fit.homoscedasticity_test" => fit.homoscedasticity_test = parse_bool_field(val, "homoscedasticity_test")?,
        "emit_residuals_tab" | "edit.fit.emit_residuals_tab" => fit.emit_residuals_tab = parse_bool_field(val, "emit_residuals_tab")?,
        "emit_covariance_tab" | "edit.fit.emit_covariance_tab" => fit.emit_covariance_tab = parse_bool_field(val, "emit_covariance_tab")?,
        "mv_x_columns" | "edit.fit.mv_x_columns" => fit.mv_x_columns = parse_usize_list(val, key)?,
        "mv_y_column" | "edit.fit.mv_y_column" => fit.mv_y_column = parse_opt_usize_min(val, key, 0)?,
        "mv_y_columns" | "edit.fit.mv_y_columns" => fit.mv_y_columns = parse_usize_list(val, key)?,
        "mv_group_columns" | "edit.fit.mv_group_columns" => fit.mv_group_columns = parse_usize_list(val, key)?,
        "weighting_source" | "edit.fit.weighting_source" => {
            fit.weighting_source = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `weighting_source`: {e}"))?;
        }
        "outlier_handling" | "edit.fit.outlier_handling" => {
            fit.outlier_handling = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `outlier_handling`: {e}"))?;
        }
        "emit_cleaned_data_tab" | "edit.fit.emit_cleaned_data_tab" => {
            fit.emit_cleaned_data_tab = parse_bool_field(val, "emit_cleaned_data_tab")?;
        }
        "replicate_fit_mode" | "edit.fit.replicate_fit_mode" => {
            fit.replicate_fit_mode = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `replicate_fit_mode`: {e}"))?;
        }
        "convergence_strictness" | "edit.fit.convergence_strictness" => {
            fit.convergence_strictness = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `convergence_strictness`: {e}"))?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
