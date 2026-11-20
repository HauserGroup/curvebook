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

use super::analyze_dialog::{
    VariableRoleFilter, default_one_way_param_unpaired, default_outlier_alpha, default_outlier_fdr_q, default_tg_nonparam_paired,
    default_tg_nonparam_unpaired, default_tg_param_paired, default_tg_param_unpaired,
};
use super::parse_helpers::{
    apply_transform_formula_str, draft_forces_reml_mode, parse_baseline_operation_str, parse_contingency_method_name, parse_df_method_str,
    parse_error_prop_value, parse_math_preset_opt, parse_multiplicity_method_name, parse_subcolumn_aggregation_val, parse_table_kind_str,
    post_hoc_method_for_widget_key,
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
    #[serde(default = "default_tg_param_unpaired")]
    pub saved_two_group_param_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_param_paired")]
    pub saved_two_group_param_paired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_unpaired")]
    pub saved_two_group_nonparam_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_paired")]
    pub saved_two_group_nonparam_paired: TwoGroupMethod,
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
            saved_two_group_param_unpaired: default_tg_param_unpaired(),
            saved_two_group_param_paired: default_tg_param_paired(),
            saved_two_group_nonparam_unpaired: default_tg_nonparam_unpaired(),
            saved_two_group_nonparam_paired: default_tg_nonparam_paired(),
            saved_one_way_param_unpaired: default_one_way_param_unpaired(),
            saved_rm_geisser_greenhouse: true,
            saved_outlier_rout_q: default_outlier_fdr_q(),
            saved_outlier_grubbs_alpha: default_outlier_alpha(),
            saved_outlier_grubbs_iterative: false,
        }
    }
}

impl EditAnalysisParametersState {
    /// Toggle a MV predictor column for PCA/PCR while maintaining disjoint PCR outcome selection.
    pub fn toggle_pca_predictor(&mut self, col: usize) {
        if let Some(pos) = self.pca_predictors.iter().position(|&c| c == col) {
            self.pca_predictors.remove(pos);
        } else {
            self.pca_predictors.push(col);
            self.pca_predictors.sort_unstable();
            if self.pcr_outcome_column == Some(col) {
                self.pcr_outcome_column = None;
            }
        }
        if let Some(spec) = &mut self.draft {
            spec.selected_data_sets.clone_from(&self.pca_predictors);
        }
    }

    /// Select (or toggle off) the disjoint PCR outcome column with R10 predictor leakage guard.
    pub fn select_pcr_outcome(&mut self, col: usize) -> Result<(), String> {
        if self.pca_predictors.contains(&col) {
            return Err(format!("Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."));
        }
        self.pcr_outcome_column = if self.pcr_outcome_column == Some(col) { None } else { Some(col) };
        if let Some(spec) = &mut self.draft {
            spec.selected_data_sets.retain(|&c| Some(c) != self.pcr_outcome_column);
        }
        Ok(())
    }
    /// Load an existing [`ResultSheet`] for parameter re-entry (`Cmd+T`).
    /// Reset dialog state and open Edit Analysis Parameters for an existing `ResultSheet` (`RV-79`).
    pub fn open_for_result(&mut self, result: &ResultSheet) {
        *self = Self::default();
        self.result_sheet_id = Some(result.id.clone());
        self.title_was_custom = result.analysis.custom_title.is_some();
        self.active_tab = default_edit_tab();
        self.draft = Some(result.analysis.clone());
        self.pca_predictors = if matches!(result.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })) {
            result.analysis.selected_data_sets.clone()
        } else {
            Vec::new()
        };
        self.pcr_outcome_column = None;
        self.sync_rule_flags_from_kind();
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
            AnalysisKind::TwoGroupComparison(spec) => match spec.method {
                TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT | TwoGroupMethod::LognormalUnpairedT => {
                    self.saved_two_group_param_unpaired = spec.method;
                }
                TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT => {
                    self.saved_two_group_param_paired = spec.method;
                }
                TwoGroupMethod::MannWhitneyU | TwoGroupMethod::KolmogorovSmirnov => {
                    self.saved_two_group_nonparam_unpaired = spec.method;
                }
                TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest => {
                    self.saved_two_group_nonparam_paired = spec.method;
                }
                TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. } => {}
            },
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

    fn resolved_two_group_method(&self, assume_gaussian: bool, matched: bool, current: TwoGroupMethod) -> TwoGroupMethod {
        if let TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } = current {
            return if assume_gaussian { TwoGroupMethod::OneSampleT { hypothetical } } else { TwoGroupMethod::OneSampleWilcoxon { hypothetical } };
        }
        match (assume_gaussian, matched) {
            (true, false) => self.saved_two_group_param_unpaired,
            (true, true) => self.saved_two_group_param_paired,
            (false, false) => self.saved_two_group_nonparam_unpaired,
            (false, true) => self.saved_two_group_nonparam_paired,
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
                if let MultiplicityMethod::Dunnett { control_column } = spec.multiplicity.method {
                    self.compare_against_control = true;
                    self.control_column = control_column;
                } else {
                    self.compare_against_control = false;
                }
            }
            AnalysisKind::FactorialAnova(spec) => {
                self.assume_gaussian = true;
                self.paired_or_matched = spec.repeated_measures != RepeatedMeasuresDesign::None;
                if let MultiplicityMethod::Dunnett { control_column } = spec.multiplicity.method {
                    self.compare_against_control = true;
                    self.control_column = control_column;
                } else {
                    self.compare_against_control = false;
                }
            }
            AnalysisKind::TwoGroupComparison(spec) => {
                self.assume_gaussian = !matches!(
                    spec.method,
                    TwoGroupMethod::MannWhitneyU
                        | TwoGroupMethod::WilcoxonSignedRank
                        | TwoGroupMethod::KolmogorovSmirnov
                        | TwoGroupMethod::OneSampleWilcoxon { .. }
                        | TwoGroupMethod::PairedSignTest
                );
                self.paired_or_matched = matches!(
                    spec.method,
                    TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest
                );
                self.welch_correction = spec.method == TwoGroupMethod::WelchT;
            }
            AnalysisKind::Contingency(spec) => {
                self.contingency_odds_ratio_checked = spec.compute_odds_and_risk_ratios;
                self.contingency_relative_risk_checked = spec.compute_odds_and_risk_ratios;
            }
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }) => {
                self.correlation_all_pairs = true;
            }
            _ => {}
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
        let priority_keys = [
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
        for key in priority_keys {
            if let Some(val) = fields.get(key) {
                self.apply_single_param_patch(key, val, preset)?;
            }
        }
        for (key, val) in fields {
            if priority_keys.contains(&key.as_str()) {
                continue;
            }
            self.apply_single_param_patch(key, val, preset)?;
        }
        Ok(())
    }

    fn apply_single_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<(), String> {
        if self.apply_contingency_param_patch(key, val, preset)?
            || self.apply_transform_param_patch(key, val)?
            || self.apply_pvalue_stack_param_patch(key, val, preset)?
            || self.apply_power_analysis_param_patch(key, val, preset)?
            || self.apply_r9_param_patch(key, val)?
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
            "assume_gaussian" | "edit.assume_gaussian" => {
                let g = val.as_bool().ok_or_else(|| String::from("`assume_gaussian` must be boolean"))?;
                self.apply_gaussian_toggle_patch(g);
            }
            "paired_or_matched" | "matched" | "edit.paired_or_matched" => {
                let matched = val.as_bool().ok_or_else(|| String::from("`paired_or_matched` must be boolean"))?;
                self.paired_or_matched = matched;
                self.apply_matched_toggle(matched);
            }
            "compare_against_control" | "edit.compare_against_control" => {
                let cmp = val.as_bool().ok_or_else(|| String::from("`compare_against_control` must be boolean"))?;
                self.compare_against_control = cmp;
                if cmp {
                    let ctrl = self.control_column;
                    self.set_anova_multiplicity(MultiplicityMethod::Dunnett { control_column: ctrl });
                }
            }
            "control_column" | "edit.control_column" => {
                let col = val.as_u64().ok_or_else(|| String::from("`control_column` must be a non-negative integer"))? as usize;
                self.control_column = col;
                if self.compare_against_control {
                    self.set_anova_multiplicity(MultiplicityMethod::Dunnett { control_column: col });
                }
            }
            "welch_correction" | "edit.welch_correction" | "edit.two_group.welch" => {
                let w = val.as_bool().ok_or_else(|| String::from("`welch_correction` must be boolean"))?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::TwoGroupComparison(mut two) = spec.kind.clone()
                {
                    if w && !crate::modal_predicates::r4_two_group_welch_correction(two.method).enabled {
                        return Err(String::from("Welch's correction is disabled for paired or nonparametric tests."));
                    }
                    self.welch_correction = w;
                    two.method = if w { TwoGroupMethod::WelchT } else { TwoGroupMethod::UnpairedT };
                    spec.update_kind(AnalysisKind::TwoGroupComparison(two));
                    self.sync_rule_flags_from_kind();
                }
            }
            "graph_differences" | "edit.two_group.graph_differences" => {
                let b = val.as_bool().ok_or_else(|| String::from("`graph_differences` must be boolean"))?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::TwoGroupComparison(two) = &spec.kind
                    && b
                    && !crate::modal_predicates::r4_two_group_graph_differences(two.method).enabled
                {
                    return Err(String::from("Estimation plot of differences is disabled in nonparametric/rank mode."));
                }
                self.graph_differences = b;
            }
            "graph_ranks" | "edit.two_group.graph_ranks" => {
                let b = val.as_bool().ok_or_else(|| String::from("`graph_ranks` must be boolean"))?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::TwoGroupComparison(two) = &spec.kind
                    && b
                    && !crate::modal_predicates::r4_two_group_nonparametric_options(two.method).enabled
                {
                    return Err(String::from("Rank-based option is disabled in Gaussian parametric mode."));
                }
                self.graph_ranks = b;
            }
            "hodges_lehmann_ci" | "hodges_lehmann" | "edit.two_group.hodges_lehmann" => {
                let b = val.as_bool().ok_or_else(|| String::from("`hodges_lehmann_ci` must be boolean"))?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::TwoGroupComparison(two) = &spec.kind
                    && b
                    && !crate::modal_predicates::r4_two_group_nonparametric_options(two.method).enabled
                {
                    return Err(String::from("Hodges-Lehmann CI is disabled in Gaussian parametric mode."));
                }
                self.hodges_lehmann_ci = b;
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
                    if !matches!(spec.p_value_format, PValueFormat::Custom { .. }) {
                        return Err(String::from("Custom decimal precision spinner is disabled unless Custom P-value format is selected."));
                    }
                    let sci = match spec.p_value_format {
                        PValueFormat::Custom { scientific_below, .. } => scientific_below,
                        _ => None,
                    };
                    spec.p_value_format = PValueFormat::new_custom(d, sci);
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
            "selected_data_sets" | "edit.selected_data_sets" | "edit.transform.selected_data_sets" => {
                let cols = parse_usize_list(val, key)?;
                if let Some(spec) = &mut self.draft {
                    if matches!(spec.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })) {
                        self.pca_predictors.clone_from(&cols);
                    }
                    spec.selected_data_sets = cols;
                }
            }
            "pca_predictors" | "edit.pca.predictors" => {
                let cols = parse_usize_list(val, key)?;
                self.pca_predictors.clone_from(&cols);
                if let Some(spec) = &mut self.draft {
                    spec.selected_data_sets = cols;
                }
            }
            "pcr_outcome_column" | "edit.pcr.outcome_column" => {
                if val.is_null() {
                    self.pcr_outcome_column = None;
                } else {
                    let col = parse_usize_min(val, key, 0)?;
                    if self.pca_predictors.contains(&col) {
                        return Err(format!(
                            "Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."
                        ));
                    }
                    self.pcr_outcome_column = Some(col);
                    if let Some(spec) = &mut self.draft {
                        spec.selected_data_sets.retain(|&c| c != col);
                    }
                }
            }
            "geisser_greenhouse" | "edit.geisser_greenhouse" | "edit.sphericity.geisser_greenhouse" => {
                if !self.paired_or_matched {
                    return Err(String::from("Geisser-Greenhouse sphericity control is disabled when No matching is selected."));
                }
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::OneWayAnova(mut a) = spec.kind.clone()
                {
                    if a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured {
                        return Err(String::from("Sphericity corrections are disabled when Mixed-Effects REML is active."));
                    }
                    if let Some(gg) = val.as_bool() {
                        a.method = OneWayMethod::RepeatedMeasures { geisser_greenhouse: gg };
                        spec.update_kind(AnalysisKind::OneWayAnova(a));
                    }
                }
            }
            "multiplicity_method" => {
                if let Some(raw) = val.as_str() {
                    let method = parse_multiplicity_method_name(raw, self.control_column)?;
                    let draft_reml = self.draft.as_ref().is_some_and(draft_forces_reml_mode);
                    let gate = crate::modal_predicates::r1b_post_hoc_option_enabled(self.assume_gaussian, draft_reml, method);
                    if !gate.enabled {
                        return Err(gate.disabled_reason.unwrap_or_else(|| String::from("Post-hoc method disabled.")));
                    }
                    self.set_anova_multiplicity(method);
                }
            }
            key if key.starts_with("edit.post_hoc.") || key.starts_with("edit.posthoc.") => {
                let method = post_hoc_method_for_widget_key(key, self.control_column)
                    .ok_or_else(|| format!("unknown parameter field `{key}` in EditAnalysisParameters"))?;
                if val.as_bool().unwrap_or(true) {
                    self.apply_checked_post_hoc_patch(method)?;
                }
            }
            "anova_method" | "one_way_method" | "edit.anova_method" | "edit.one_way_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`anova_method` must be a string"))?.trim().to_ascii_lowercase();
                let saved_gg = self.saved_rm_geisser_greenhouse;
                let next_method = match raw.as_str() {
                    "ordinary" | "ordinary_anova" | "anova" => OneWayMethod::OrdinaryAnova,
                    "welch" | "brown_forsythe" | "brown_forsythe_welch" | "brown_forsythe_and_welch" => OneWayMethod::BrownForsytheAndWelch,
                    "kruskal" | "kruskal_wallis" => OneWayMethod::KruskalWallis,
                    "friedman" => OneWayMethod::Friedman,
                    "rm" | "repeated_measures" => OneWayMethod::RepeatedMeasures { geisser_greenhouse: saved_gg },
                    _ => return Err(format!("unknown one-way ANOVA method `{raw}`")),
                };
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
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                self.contingency_or_ci_method = s.to_owned();
            }
            "edit.contingency.ci.koopman" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                self.contingency_rr_ci_method = s.to_owned();
            }
            "edit.contingency.ci.newcombe_wilson" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                self.contingency_arr_ci_method = s.to_owned();
            }
            "edit.contingency.ci.wilson_brown" => {
                let s = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                self.contingency_diag_ci_method = s.to_owned();
            }
            "compute_odds_and_risk_ratios" | "contingency_odds_ratio_checked" | "edit.contingency.odds_ratio" => {
                if let Some(b) = val.as_bool() {
                    self.contingency_odds_ratio_checked = b;
                    self.contingency_relative_risk_checked = b;
                    self.update_contingency_spec(|c| c.compute_odds_and_risk_ratios = b);
                }
            }
            "contingency_attributable_risk_checked" | "attributable_risk" | "compute_attributable_risk" | "edit.contingency.attributable_risk" => {
                if let Some(b) = val.as_bool() {
                    self.contingency_attributable_risk_checked = b;
                }
            }
            "compute_sensitivity_specificity" | "sensitivity_specificity" | "edit.contingency.sensitivity_specificity" => {
                if let Some(b) = val.as_bool() {
                    self.update_contingency_spec(|c| c.compute_sensitivity_specificity = b);
                }
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
        if let Some(spec) = &mut self.draft
            && let AnalysisKind::Contingency(mut c) = spec.kind.clone()
        {
            mutate(&mut c);
            spec.update_kind(AnalysisKind::Contingency(c));
        }
    }

    /// Applies `PValueStack`-specific field patches on `EditAnalysisParametersState`.
    fn apply_pvalue_stack_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
        let Some(spec) = &mut self.draft else {
            return Ok(false);
        };
        let AnalysisKind::PValueStack(mut ps) = spec.kind.clone() else {
            return Ok(false);
        };
        let handled = match key {
            "method"
            | "pvalue_stack_method"
            | "multiplicity_method"
            | "multiplicity"
            | "edit.pvalue_stack.method"
            | "edit.pvalue_stack.multiplicity_method" => {
                ps.method = parse_pvalue_stack_method_val(val)?;
                true
            }
            "threshold" | "pvalue_stack_threshold" | "alpha" | "q" | "fdr_q" | "edit.pvalue_stack.threshold" => {
                let t = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0 < t && t < 1.0) {
                    return Err(format!("`{key}` ({t}) must lie strictly in (0, 1)"));
                }
                ps.threshold = t;
                true
            }
            "column" | "pvalue_stack_column" | "edit.pvalue_stack.column" => {
                ps.column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            _ => false,
        };
        if handled {
            spec.update_kind(AnalysisKind::PValueStack(ps));
        }
        Ok(handled)
    }

    /// Applies `PowerAnalysis`-specific field patches on `EditAnalysisParametersState`.
    fn apply_power_analysis_param_patch(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
        let Some(spec) = &mut self.draft else {
            return Ok(false);
        };
        let AnalysisKind::PowerAnalysis(mut pa) = spec.kind.clone() else {
            return Ok(false);
        };
        let handled = match key {
            "edit.power.family" | "power.family" => {
                let raw = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
                pa.family = match norm.as_str() {
                    "unpairedt" | "twosampleunpairedt" | "independentt" => curvebook_doc::PowerTestFamily::TwoSampleUnpairedT,
                    "pairedt" | "twosamplepairedt" | "dependentt" => curvebook_doc::PowerTestFamily::TwoSamplePairedT,
                    "onesamplet" | "singlesamplet" => curvebook_doc::PowerTestFamily::OneSampleT,
                    "onewayanova" | "anova" => curvebook_doc::PowerTestFamily::OneWayAnova,
                    "twoproportions" | "proportions" => curvebook_doc::PowerTestFamily::TwoProportions,
                    "correlation" | "pearsoncorrelation" => curvebook_doc::PowerTestFamily::Correlation,
                    "chisquare" | "chi2" => curvebook_doc::PowerTestFamily::ChiSquare,
                    "simplelinearregression" | "linearregression" | "regression" => curvebook_doc::PowerTestFamily::SimpleLinearRegression,
                    _ => return Err(format!("unknown power test family `{raw}`")),
                };
                true
            }
            "edit.power.solve_target" | "power.solve_target" => {
                let raw = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
                pa.solve_target = match norm.as_str() {
                    "samplesize" | "n" => curvebook_doc::PowerSolveTarget::SampleSize,
                    "power" => curvebook_doc::PowerSolveTarget::Power,
                    "detectableeffect" | "effect" | "mde" => curvebook_doc::PowerSolveTarget::DetectableEffect,
                    _ => return Err(format!("unknown power solve target `{raw}`")),
                };
                true
            }
            "edit.power.alpha" => {
                let a = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0..1.0).contains(&a) || a <= 0.0 {
                    return Err(format!("`{key}` must lie strictly in (0, 1)"));
                }
                pa.alpha = a;
                true
            }
            "edit.power.target_power" => {
                if val.is_null() {
                    pa.target_power = None;
                } else {
                    let pw = parse_finite_f64_with_locale(val, key, preset)?;
                    if !(0.0..1.0).contains(&pw) || pw <= 0.0 {
                        return Err(format!("`{key}` must lie strictly in (0, 1)"));
                    }
                    pa.target_power = Some(pw);
                }
                true
            }
            "edit.power.sample_size" => {
                pa.sample_size = if val.is_null() { None } else { Some(parse_finite_f64_with_locale(val, key, preset)?) };
                true
            }
            "edit.power.sample_size_group2" => {
                pa.sample_size_group2 = if val.is_null() { None } else { Some(parse_finite_f64_with_locale(val, key, preset)?) };
                true
            }
            "edit.power.allocation_ratio" => {
                let r = parse_finite_f64_with_locale(val, key, preset)?;
                if r <= 0.0 {
                    return Err(format!("`{key}` must be > 0"));
                }
                pa.allocation_ratio = r;
                true
            }
            "edit.power.two_tailed" => {
                let b = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
                pa.tail_mode = if b {
                    curvebook_doc::PowerTailMode::TwoSided
                } else {
                    curvebook_doc::PowerTailMode::OneSided(curvebook_doc::PowerOneSidedDirection::Greater)
                };
                true
            }
            "edit.power.continuity_correction" => {
                pa.continuity_correction = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
                true
            }
            "edit.power.chi_square_df" => {
                let df = parse_finite_f64_with_locale(val, key, preset)?;
                if df <= 0.0 {
                    return Err(format!("`{key}` must be > 0"));
                }
                pa.chi_square_df = df;
                true
            }
            _ => false,
        };
        if handled {
            spec.update_kind(AnalysisKind::PowerAnalysis(pa));
        }
        Ok(handled)
    }

    /// Applies Transform-specific field patches on `EditAnalysisParametersState`.
    fn apply_transform_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        match key {
            "x_transform" | "edit.transform.x_transform" => {
                let parsed = parse_math_preset_opt(val)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, error_propagation, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform: parsed, y_transform, error_propagation }));
                }
            }
            "y_transform" | "edit.transform.y_transform" => {
                let parsed = parse_math_preset_opt(val)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, error_propagation, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform: parsed, error_propagation }));
                }
            }
            "error_propagation" | "edit.transform.error_propagation" => {
                let ep_mode = parse_error_prop_value(val)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation: ep_mode }));
                }
            }
            "formula" | "transform_formula" | "edit.transform.formula" | "filter_formula" | "edit.transform.filter_formula" => {
                if let Some(spec) = &mut self.draft {
                    match spec.kind.clone() {
                        AnalysisKind::Transform(TransformAnalysisSpec::Math { mut x_transform, mut y_transform, error_propagation }) => {
                            if let Some(raw) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                apply_transform_formula_str(raw, &mut x_transform, &mut y_transform)?;
                                spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::Math {
                                    x_transform,
                                    y_transform,
                                    error_propagation,
                                }));
                            }
                        }
                        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, derived_columns, .. }) => {
                            let filter_formula = if val.is_null() {
                                None
                            } else {
                                let s = val.as_str().ok_or_else(|| String::from("`filter_formula` must be a string or null"))?.trim();
                                (!s.is_empty()).then(|| s.to_owned())
                            };
                            spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                                filter_formula,
                                target_archetype,
                                derived_columns,
                            }));
                        }
                        _ => {}
                    }
                }
            }
            "target_archetype" | "edit.transform.target_archetype" => {
                let raw = val.as_str().ok_or_else(|| String::from("`target_archetype` must be a string"))?;
                let parsed = parse_table_kind_str(raw)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, derived_columns, .. }) =
                        spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                        filter_formula,
                        target_archetype: parsed,
                        derived_columns,
                    }));
                }
            }
            "baseline_columns" | "edit.transform.baseline_columns" => {
                let cols = parse_usize_list(val, key)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { operation, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: cols, operation }));
                }
            }
            "baseline_operation" | "operation" | "edit.transform.baseline_operation" => {
                let raw = val.as_str().ok_or_else(|| String::from("`baseline_operation` must be a string"))?;
                let op = parse_baseline_operation_str(raw)?;
                if let Some(spec) = &mut self.draft
                    && let AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, .. }) = spec.kind.clone()
                {
                    spec.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation: op }));
                }
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
                t.method = self.resolved_two_group_method(assume_gaussian, matched, t.method);
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
                t.method = self.resolved_two_group_method(self.assume_gaussian, matched, t.method);
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

    fn apply_checked_post_hoc_patch(&mut self, method: MultiplicityMethod) -> Result<(), String> {
        let draft_reml = self.draft.as_ref().is_some_and(draft_forces_reml_mode);
        let gate = crate::modal_predicates::r1b_post_hoc_option_enabled(self.assume_gaussian, draft_reml, method);
        if !gate.enabled {
            return Err(gate.disabled_reason.unwrap_or_else(|| String::from("Post-hoc method is disabled.")));
        }
        self.set_anova_multiplicity(method);
        Ok(())
    }

    fn set_anova_multiplicity(&mut self, method: MultiplicityMethod) {
        let Some(spec) = &mut self.draft else {
            return;
        };
        let mut updated = spec.kind.clone();
        match &mut updated {
            AnalysisKind::OneWayAnova(a) => a.multiplicity.method = method,
            AnalysisKind::FactorialAnova(f) => f.multiplicity.method = method,
            AnalysisKind::NestedComparison(n) => n.multiplicity.method = method,
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => multiplicity.method = method,
            _ => {}
        }
        spec.update_kind(updated);
    }

    fn apply_r9_param_patch(&mut self, key: &str, val: &Value) -> Result<bool, String> {
        let Some(spec) = &mut self.draft else {
            return Ok(false);
        };
        let mut updated = spec.kind.clone();
        let handled = match (key, &mut updated) {
            ("outlier_method" | "edit.outlier.method" | "method", AnalysisKind::OutlierDetection(out)) => {
                match *out {
                    OutlierSpec::Rout { fdr_q_percent } => self.saved_outlier_rout_q = fdr_q_percent,
                    OutlierSpec::Grubbs { alpha, iterative } => {
                        self.saved_outlier_grubbs_alpha = alpha;
                        self.saved_outlier_grubbs_iterative = iterative;
                    }
                }
                let raw = val.as_str().ok_or_else(|| String::from("`outlier_method` must be a string"))?;
                let (mode, iter_opt) = parse_outlier_method_str(raw)?;
                *out = if mode == "grubbs" {
                    OutlierSpec::Grubbs { alpha: self.saved_outlier_grubbs_alpha, iterative: iter_opt.unwrap_or(self.saved_outlier_grubbs_iterative) }
                } else {
                    OutlierSpec::Rout { fdr_q_percent: self.saved_outlier_rout_q }
                };
                true
            }
            (
                "fdr_q_percent" | "q_percent" | "rout_q" | "outlier_fdr_q_percent" | "edit.outlier.fdr_q_percent" | "edit.fit.rout_q",
                AnalysisKind::OutlierDetection(out),
            ) => {
                let q = parse_outlier_fdr_q_percent(val)?;
                self.saved_outlier_rout_q = q;
                match out {
                    OutlierSpec::Rout { fdr_q_percent } => *fdr_q_percent = q,
                    OutlierSpec::Grubbs { .. } => {
                        return Err(String::from("`fdr_q_percent` requires ROUT outlier detection mode"));
                    }
                }
                true
            }
            ("outlier_alpha" | "edit.outlier.alpha" | "alpha", AnalysisKind::OutlierDetection(out)) => {
                let a = parse_outlier_alpha(val)?;
                self.saved_outlier_grubbs_alpha = a;
                match out {
                    OutlierSpec::Grubbs { alpha, .. } => *alpha = a,
                    OutlierSpec::Rout { .. } => {
                        return Err(String::from("`alpha` for Grubbs requires Grubbs outlier detection mode"));
                    }
                }
                true
            }
            ("iterative" | "outlier_iterative" | "edit.outlier.iterative", AnalysisKind::OutlierDetection(out)) => {
                let it = val.as_bool().ok_or_else(|| String::from("`iterative` must be boolean"))?;
                self.saved_outlier_grubbs_iterative = it;
                match out {
                    OutlierSpec::Grubbs { iterative, .. } => *iterative = it,
                    OutlierSpec::Rout { .. } => {
                        return Err(String::from("`iterative` requires Grubbs outlier detection mode"));
                    }
                }
                true
            }
            ("descriptive_subcolumn_agg" | "subcolumn_agg" | "edit.descriptive.subcolumn_agg", AnalysisKind::DescriptiveStats(desc)) => {
                desc.aggregation = parse_subcolumn_aggregation_val(val)?;
                true
            }
            ("two_group_method" | "edit.two_group.method", AnalysisKind::TwoGroupComparison(t)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`two_group_method` must be a string"))?;
                let hyp = match t.method {
                    TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => hypothetical,
                    _ => 0.0,
                };
                t.method = parse_two_group_method_str(raw, hyp)?;
                true
            }
            ("paired_sign_test" | "edit.two_group.paired_sign_test", AnalysisKind::TwoGroupComparison(t)) => {
                let b = val.as_bool().ok_or_else(|| String::from("`paired_sign_test` must be boolean"))?;
                t.method = if b { TwoGroupMethod::PairedSignTest } else { TwoGroupMethod::WilcoxonSignedRank };
                true
            }
            ("two_tailed" | "edit.two_group.two_tailed", AnalysisKind::TwoGroupComparison(t)) => {
                t.two_tailed = val.as_bool().ok_or_else(|| String::from("`two_tailed` must be boolean"))?;
                true
            }
            ("compute_effect_sizes" | "edit.compute_effect_sizes", AnalysisKind::TwoGroupComparison(t)) => {
                t.compute_effect_sizes = val.as_bool().ok_or_else(|| String::from("`compute_effect_sizes` must be boolean"))?;
                true
            }
            ("compute_effect_sizes" | "edit.compute_effect_sizes", AnalysisKind::FactorialAnova(f)) => {
                f.compute_effect_sizes = val.as_bool().ok_or_else(|| String::from("`compute_effect_sizes` must be boolean"))?;
                true
            }
            ("mv_group_column" | "edit.two_group.mv_group_column", AnalysisKind::TwoGroupComparison(t)) => {
                t.mv_group_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("mv_response_column" | "edit.two_group.mv_response_column", AnalysisKind::TwoGroupComparison(t)) => {
                t.mv_response_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("three_way" | "edit.factorial.three_way", AnalysisKind::FactorialAnova(f)) => {
                f.three_way = val.as_bool().ok_or_else(|| String::from("`three_way` must be boolean"))?;
                if f.three_way {
                    f.max_interaction_order = f.max_interaction_order.max(3);
                } else {
                    f.max_interaction_order = f.max_interaction_order.min(2);
                }
                true
            }
            ("repeated_measures" | "edit.factorial.repeated_measures", AnalysisKind::FactorialAnova(f)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`repeated_measures` must be a string"))?;
                f.repeated_measures = parse_rm_design_str(raw)?;
                true
            }
            ("mixed_effects_reml" | "edit.rm.mixed_effects_reml", AnalysisKind::OneWayAnova(a)) => {
                let b = val.as_bool().ok_or_else(|| String::from("`mixed_effects_reml` must be boolean"))?;
                a.mixed_effects_reml = b;
                if b && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(a.multiplicity.method) {
                    a.multiplicity.method = MultiplicityMethod::Bonferroni;
                }
                true
            }
            ("mixed_effects_reml" | "edit.rm.mixed_effects_reml", AnalysisKind::FactorialAnova(f)) => {
                let b = val.as_bool().ok_or_else(|| String::from("`mixed_effects_reml` must be boolean"))?;
                f.mixed_effects_reml = b;
                if b && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(f.multiplicity.method) {
                    f.multiplicity.method = MultiplicityMethod::Bonferroni;
                }
                true
            }
            ("df_method" | "edit.rm.df_method", AnalysisKind::OneWayAnova(a)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`df_method` must be a string"))?;
                a.df_method = parse_df_method_str(raw)?;
                true
            }
            ("df_method" | "edit.rm.df_method", AnalysisKind::FactorialAnova(f)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`df_method` must be a string"))?;
                f.df_method = parse_df_method_str(raw)?;
                true
            }
            ("df_method" | "edit.rm.df_method", AnalysisKind::NestedComparison(n)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`df_method` must be a string"))?;
                n.df_method = parse_df_method_str(raw)?;
                true
            }
            ("residual_covariance" | "edit.rm.residual_covariance" | "edit.reml.residual_covariance", AnalysisKind::OneWayAnova(a)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`residual_covariance` must be a string"))?;
                a.residual_covariance = parse_residual_covariance_str(raw)?;
                true
            }
            (
                "residual_covariance" | "factorial_residual_covariance" | "edit.factorial.residual_covariance" | "edit.reml.residual_covariance",
                AnalysisKind::FactorialAnova(f),
            ) => {
                let raw = val.as_str().ok_or_else(|| String::from("`residual_covariance` must be a string"))?;
                f.residual_covariance = parse_residual_covariance_str(raw)?;
                true
            }
            ("ss_type" | "sum_of_squares" | "edit.factorial.ss_type", AnalysisKind::FactorialAnova(f)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`ss_type` must be a string"))?;
                f.ss_type = parse_ss_type_str(raw)?;
                true
            }
            ("max_interaction_order" | "edit.factorial.max_interaction_order", AnalysisKind::FactorialAnova(f)) => {
                f.max_interaction_order = parse_usize_min(val, key, 1)?;
                true
            }
            ("fit_reml_on_missing" | "edit.factorial.fit_reml_on_missing", AnalysisKind::FactorialAnova(f)) => {
                f.fit_reml_on_missing = val.as_bool().ok_or_else(|| String::from("`fit_reml_on_missing` must be boolean"))?;
                true
            }
            ("multifactor_columns" | "edit.factorial.multifactor_columns", AnalysisKind::FactorialAnova(f)) => {
                f.multifactor_columns = parse_usize_list(val, key)?;
                true
            }
            ("factorial_multiplicity" | "edit.factorial.multiplicity", AnalysisKind::FactorialAnova(f)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`factorial.multiplicity` must be a string"))?;
                f.multiplicity.method = parse_multiplicity_method_name(raw, self.control_column)?;
                true
            }
            ("response_column" | "factorial_response_column" | "edit.factorial.response_column", AnalysisKind::FactorialAnova(f)) => {
                f.response_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            (
                "distance" | "cluster_distance" | "edit.cluster.distance",
                AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance, .. }),
            ) => {
                let raw = val.as_str().ok_or_else(|| String::from("`distance` must be a string"))?;
                *distance = parse_cluster_distance_str(raw)?;
                true
            }
            ("linkage" | "cluster_linkage" | "edit.cluster.linkage", AnalysisKind::Multivariate(MultivariateSpec::Clustering { linkage, .. })) => {
                let raw = val.as_str().ok_or_else(|| String::from("`linkage` must be a string"))?;
                *linkage = parse_cluster_linkage_str(raw)?;
                true
            }
            ("k_means_k" | "k" | "edit.cluster.k_means_k", AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, .. })) => {
                *k_means_k = if val.is_null() { None } else { Some(parse_usize_min(val, key, 1)?) };
                true
            }
            ("k_means_auto" | "edit.cluster.k_means_auto", AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, .. })) => {
                let auto = val.as_bool().ok_or_else(|| String::from("`k_means_auto` must be boolean"))?;
                *k_means_k = if auto { None } else { Some(k_means_k.unwrap_or(3)) };
                true
            }
            ("k_sweep_max" | "edit.cluster.k_sweep_max", AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_sweep_max, .. })) => {
                *k_sweep_max = if val.is_null() { None } else { Some(parse_usize_min(val, key, 2)?) };
                true
            }
            ("standardize" | "edit.pca.standardize", AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize, .. })) => {
                *standardize = val.as_bool().ok_or_else(|| String::from("`standardize` must be boolean"))?;
                true
            }
            (
                "parallel_analysis_seed" | "edit.pca.parallel_analysis_seed",
                AnalysisKind::Multivariate(MultivariateSpec::Pca { parallel_analysis_seed, .. }),
            ) => {
                *parallel_analysis_seed = if val.is_null() { None } else { Some(parse_u64_val(val, key)?) };
                true
            }
            (
                "spearman_rank" | "edit.corr.spearman_rank" | "edit.correlation.spearman_rank",
                AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank }),
            ) => {
                *spearman_rank = val.as_bool().ok_or_else(|| String::from("`spearman_rank` must be boolean"))?;
                true
            }
            (
                "imputations_m" | "m" | "edit.mi.imputations_m" | "edit.imputation.m",
                AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m, .. }),
            ) => {
                *imputations_m = parse_usize_min(val, key, 1)?;
                true
            }
            ("max_iters" | "edit.mi.max_iters", AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { max_iters, .. })) => {
                *max_iters = parse_usize_min(val, key, 1)?;
                true
            }
            ("seed" | "edit.mi.seed", AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { seed, .. })) => {
                *seed = parse_u64_val(val, key)?;
                true
            }
            (
                "response_column" | "edit.mi.response_column",
                AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { response_column, .. }),
            ) => {
                *response_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            (
                "predictor_columns" | "edit.mi.predictor_columns",
                AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { predictor_columns, .. }),
            ) => {
                *predictor_columns = parse_usize_list(val, key)?;
                true
            }
            ("family" | "glm_family" | "edit.glm.family", AnalysisKind::GeneralizedLinearModel(glm)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`family` must be a string"))?;
                glm.family = parse_glm_family_str(raw)?;
                true
            }
            ("response_column" | "edit.glm.response_column", AnalysisKind::GeneralizedLinearModel(glm)) => {
                glm.response_column = parse_usize_min(val, key, 0)?;
                true
            }
            ("predictor_columns" | "edit.glm.predictor_columns", AnalysisKind::GeneralizedLinearModel(glm)) => {
                glm.predictor_columns = parse_usize_list(val, key)?;
                true
            }
            ("reference_levels" | "edit.glm.reference_levels", AnalysisKind::GeneralizedLinearModel(glm)) => {
                glm.reference_levels = parse_reference_levels(val)?;
                true
            }
            ("compute_vif" | "edit.glm.compute_vif", AnalysisKind::GeneralizedLinearModel(glm)) => {
                glm.compute_vif = val.as_bool().ok_or_else(|| String::from("`compute_vif` must be boolean"))?;
                true
            }
            ("time_column" | "edit.cox.time_column", AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, .. })) => {
                *time_column = parse_usize_min(val, key, 0)?;
                true
            }
            (
                "status_column" | "edit.cox.status_column",
                AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { status_column, .. }),
            ) => {
                *status_column = parse_usize_min(val, key, 0)?;
                true
            }
            (
                "predictor_columns" | "edit.cox.predictor_columns",
                AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns, .. }),
            ) => {
                *predictor_columns = parse_usize_list(val, key)?;
                true
            }
            (
                "reference_levels" | "edit.cox.reference_levels",
                AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { reference_levels, .. }),
            ) => {
                *reference_levels = parse_reference_levels(val)?;
                true
            }
            ("tie_method" | "edit.cox.tie_method", AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. })) => {
                let raw = val.as_str().ok_or_else(|| String::from("`tie_method` must be a string"))?;
                *tie_method = parse_cox_tie_method_str(raw)?;
                true
            }
            (
                "log_rank_mantel_cox" | "edit.km.log_rank_mantel_cox",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { log_rank_mantel_cox, .. }),
            ) => {
                *log_rank_mantel_cox = val.as_bool().ok_or_else(|| String::from("`log_rank_mantel_cox` must be boolean"))?;
                true
            }
            (
                "gehan_breslow_wilcoxon" | "edit.km.gehan_breslow_wilcoxon",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { gehan_breslow_wilcoxon, .. }),
            ) => {
                *gehan_breslow_wilcoxon = val.as_bool().ok_or_else(|| String::from("`gehan_breslow_wilcoxon` must be boolean"))?;
                true
            }
            (
                "trend_test" | "edit.km.trend_test" | "log_rank_for_trend" | "edit.km.log_rank_for_trend",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { trend_test, .. }),
            ) => {
                *trend_test = val.as_bool().ok_or_else(|| String::from("`trend_test` must be boolean"))?;
                true
            }
            (
                "multiplicity_method" | "edit.km.multiplicity_method",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }),
            ) => {
                let raw = val.as_str().ok_or_else(|| String::from("`multiplicity_method` must be a string"))?;
                multiplicity.method = parse_multiplicity_method_name(raw, self.control_column)?;
                true
            }
            (
                "asymmetrical_greenwood_ci" | "edit.km.asymmetrical_greenwood_ci",
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { asymmetrical_greenwood_ci, .. }),
            ) => {
                *asymmetrical_greenwood_ci = val.as_bool().ok_or_else(|| String::from("`asymmetrical_greenwood_ci` must be boolean"))?;
                true
            }
            ("mv_time_column" | "edit.km.mv_time_column", AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, .. })) => {
                *mv_time_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("mv_status_column" | "edit.km.mv_status_column", AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_status_column, .. })) => {
                *mv_status_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("mv_group_column" | "edit.km.mv_group_column", AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_group_column, .. })) => {
                *mv_group_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("equation_id" | "edit.fit.equation_id" | "edit.fit.model_id", AnalysisKind::NonlinearRegression(fit)) => {
                let raw = val.as_str().ok_or_else(|| String::from("`equation_id` must be a string"))?.trim();
                if !raw.is_empty() {
                    fit.equation_id = raw.to_owned();
                }
                true
            }
            ("custom_dsl" | "edit.fit.custom_dsl", AnalysisKind::NonlinearRegression(fit)) => {
                fit.custom_dsl = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(ToOwned::to_owned);
                true
            }
            ("weighting" | "fit_weighting" | "edit.fit.weighting", AnalysisKind::NonlinearRegression(fit)) => {
                fit.weighting = parse_weighting_scheme(val)?;
                true
            }
            ("fit_method" | "edit.fit.method" | "method", AnalysisKind::NonlinearRegression(fit)) => {
                fit.method = parse_fit_method(val)?;
                true
            }
            ("rout_q" | "fdr_q_percent" | "q_percent" | "edit.fit.rout_q", AnalysisKind::NonlinearRegression(fit)) => {
                let q = parse_outlier_fdr_q_percent(val)?;
                let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut fit.method else {
                    return Err(String::from("`rout_q` requires ROUT outlier elimination fit method"));
                };
                *fdr_q_percent = q;
                true
            }
            ("parameter_rules" | "edit.fit.parameter_rules", AnalysisKind::NonlinearRegression(fit)) => {
                fit.parameter_rules = parse_parameter_rules(val)?;
                true
            }
            ("profile_likelihood_ci" | "edit.fit.profile_likelihood_ci", AnalysisKind::NonlinearRegression(fit)) => {
                fit.profile_likelihood_ci = val.as_bool().ok_or_else(|| String::from("`profile_likelihood_ci` must be boolean"))?;
                true
            }
            ("confidence_bands" | "edit.fit.confidence_bands", AnalysisKind::NonlinearRegression(fit)) => {
                fit.confidence_bands = val.as_bool().ok_or_else(|| String::from("`confidence_bands` must be boolean"))?;
                true
            }
            ("prediction_bands" | "edit.fit.prediction_bands", AnalysisKind::NonlinearRegression(fit)) => {
                fit.prediction_bands = val.as_bool().ok_or_else(|| String::from("`prediction_bands` must be boolean"))?;
                true
            }
            ("homoscedasticity_test" | "edit.fit.homoscedasticity_test", AnalysisKind::NonlinearRegression(fit)) => {
                fit.homoscedasticity_test = val.as_bool().ok_or_else(|| String::from("`homoscedasticity_test` must be boolean"))?;
                true
            }
            ("emit_residuals_tab" | "edit.fit.emit_residuals_tab", AnalysisKind::NonlinearRegression(fit)) => {
                fit.emit_residuals_tab = val.as_bool().ok_or_else(|| String::from("`emit_residuals_tab` must be boolean"))?;
                true
            }
            ("emit_covariance_tab" | "edit.fit.emit_covariance_tab", AnalysisKind::NonlinearRegression(fit)) => {
                fit.emit_covariance_tab = val.as_bool().ok_or_else(|| String::from("`emit_covariance_tab` must be boolean"))?;
                true
            }
            ("mv_x_columns" | "edit.fit.mv_x_columns", AnalysisKind::NonlinearRegression(fit)) => {
                fit.mv_x_columns = parse_usize_list(val, key)?;
                true
            }
            ("mv_y_column" | "edit.fit.mv_y_column", AnalysisKind::NonlinearRegression(fit)) => {
                fit.mv_y_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            ("mv_group_columns" | "edit.fit.mv_group_columns", AnalysisKind::NonlinearRegression(fit)) => {
                fit.mv_group_columns = parse_usize_list(val, key)?;
                true
            }
            ("sample_size" | "edit.sample.sample_size", AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { sample_size, .. })) => {
                *sample_size = parse_usize_min(val, key, 1)?;
                true
            }
            ("seed" | "edit.sample.seed", AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed, .. })) => {
                *seed = parse_u64_val(val, key)?;
                true
            }
            (
                "stratify_column" | "edit.sample.stratify_column",
                AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { stratify_column, .. }),
            ) => {
                *stratify_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                true
            }
            (
                "proportional_allocation" | "edit.sample.proportional_allocation",
                AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { proportional_allocation, .. }),
            ) => {
                *proportional_allocation = val.as_bool().ok_or_else(|| String::from("`proportional_allocation` must be boolean"))?;
                true
            }
            _ => {
                if let Some(col_raw) = key.strip_prefix("edit.two_group.group.")
                    && let Ok(col) = col_raw.parse::<usize>()
                    && let AnalysisKind::TwoGroupComparison(t) = &mut updated
                {
                    t.mv_group_column = Some(col);
                    true
                } else if let Some(col_raw) = key.strip_prefix("edit.two_group.resp.")
                    && let Ok(col) = col_raw.parse::<usize>()
                    && let AnalysisKind::TwoGroupComparison(t) = &mut updated
                {
                    t.mv_response_column = Some(col);
                    true
                } else if let Some(col_raw) = key.strip_prefix("edit.fac.resp.")
                    && let Ok(col) = col_raw.parse::<usize>()
                    && let AnalysisKind::FactorialAnova(f) = &mut updated
                {
                    f.response_column = Some(col);
                    true
                } else if let Some(col_raw) = key.strip_prefix("edit.fac.col.")
                    && let Ok(col) = col_raw.parse::<usize>()
                    && let AnalysisKind::FactorialAnova(f) = &mut updated
                {
                    let want_on = val.as_bool().unwrap_or_else(|| !f.multifactor_columns.contains(&col));
                    if want_on {
                        if !f.multifactor_columns.contains(&col) {
                            f.multifactor_columns.push(col);
                            f.multifactor_columns.sort_unstable();
                        }
                    } else {
                        f.multifactor_columns.retain(|&c| c != col);
                    }
                    true
                } else {
                    false
                }
            }
        };
        if handled {
            spec.update_kind(updated);
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
            "custom_title": spec.custom_title,
        });
        if let Some(map) = payload.as_object_mut() {
            if matches!(spec.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. })) {
                map.insert(String::from("correlation_all_pairs"), json!(self.correlation_all_pairs));
                map.insert(String::from("correlation_heatmap"), json!(self.correlation_heatmap));
            }
            if let Some(outcome_col) = self.pcr_outcome_column {
                map.insert(String::from("pcr_outcome_column"), json!(outcome_col));
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
        intents
    }
}
