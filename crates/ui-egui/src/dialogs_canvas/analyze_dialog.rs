//! `Analyze...` dialog state machine, variable role filters, duplicate prompts, and candidate collection.

use curvebook_doc::{
    AnalysisKind, BaselineOperation, ClusterDistance, ClusterLinkage, ContingencyMethod, CoxTieMethod, DataSheet, ErrorPropagationMode, FitMethod,
    GlmFamily, MathTransformPreset, MultiplicityMethod, MultivariateSpec, NonlinearFitSpec, OneWayMethod, OutlierSpec, PValueStackMethod,
    ParameterRule, ProjectDocument, RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultSheet, ResultTab, SheetId, SubcolumnAggregation,
    SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod, VariableRole, WeightingScheme,
};
use curvebook_palette::Rgba8;

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::LocalePreset;
use crate::theme::UiThemeSpec;

use super::analysis_catalog::{ANALYSIS_CATALOG, AnalysisCatalogItem, AnalysisCategory, default_analysis_kind_for_id};
use super::edit_params::default_true_flag;
use super::parse_helpers::{
    apply_transform_formula_str, parse_baseline_operation_str, parse_contingency_method_name, parse_error_prop_value, parse_math_preset_opt,
    parse_multiplicity_method_name, parse_subcolumn_aggregation_val, parse_table_kind_str,
};
use crate::dialogs_r9::{
    parse_cluster_distance_str, parse_cluster_linkage_str, parse_cox_tie_method_str, parse_f64_list_with_locale, parse_finite_f64_with_locale,
    parse_fit_method, parse_glm_family_str, parse_kappa_weighting, parse_opt_finite_f64_with_locale, parse_outlier_alpha,
    parse_outlier_fdr_q_percent, parse_outlier_method_str, parse_parameter_rules, parse_pvalue_stack_method_val, parse_reference_levels,
    parse_residual_covariance_str, parse_rm_design_str, parse_ss_type_str, parse_two_group_method_str, parse_u64_val, parse_usize_list,
    parse_usize_min, parse_weighting_scheme,
};

/// Filter rule applied to `MultipleVariables` column pickers in analysis and bubble dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum VariableRoleFilter {
    #[default]
    All,
    ContinuousOnly,
    CategoricalOnly,
    LabelOnly,
    CalculatedOnly,
    NumericForModeling,
}

impl VariableRoleFilter {
    #[must_use]
    pub const fn matches(self, role: VariableRole) -> bool {
        match self {
            Self::All => true,
            Self::ContinuousOnly => matches!(role, VariableRole::Continuous),
            Self::CategoricalOnly => matches!(role, VariableRole::Categorical),
            Self::LabelOnly => matches!(role, VariableRole::Label),
            Self::CalculatedOnly => matches!(role, VariableRole::Calculated),
            Self::NumericForModeling => matches!(role, VariableRole::Continuous | VariableRole::Calculated),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All Variables",
            Self::ContinuousOnly => "Continuous",
            Self::CategoricalOnly => "Categorical",
            Self::LabelOnly => "Label",
            Self::CalculatedOnly => "Calculated",
            Self::NumericForModeling => "Numeric (Continuous + Calculated)",
        }
    }

    /// Parse a canonical role-filter slug (`all`, `continuous`, `categorical`, `label`, `calculated`, `numeric`).
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug.trim().to_ascii_lowercase().as_str() {
            "all" => Some(Self::All),
            "continuous" => Some(Self::ContinuousOnly),
            "categorical" => Some(Self::CategoricalOnly),
            "label" => Some(Self::LabelOnly),
            "calculated" => Some(Self::CalculatedOnly),
            "numeric" | "numeric_for_modeling" => Some(Self::NumericForModeling),
            _ => None,
        }
    }
}

/// Contrasting badge foreground ink (`BLACK` or `WHITE`) for a [`variable_role_color`] fill.
#[must_use]
pub const fn variable_role_badge_fg(bg: Rgba8) -> Rgba8 {
    let luma = (bg.r as u32) * 299 + (bg.g as u32) * 587 + (bg.b as u32) * 114;
    if luma >= 145_000 { Rgba8::BLACK } else { Rgba8::WHITE }
}

/// Short badge text for a typed variable role (`MultipleVariablesTable`).
#[must_use]
pub const fn variable_role_badge(role: VariableRole) -> &'static str {
    match role {
        VariableRole::Continuous => "NUM",
        VariableRole::Categorical => "CAT",
        VariableRole::Label => "LBL",
        VariableRole::Calculated => "FX=",
    }
}

/// Theme-aware badge fill colour for a typed variable role.
#[must_use]
pub const fn variable_role_color(role: VariableRole, theme: &UiThemeSpec) -> Rgba8 {
    match role {
        VariableRole::Continuous => theme.link_blue,
        VariableRole::Categorical => Rgba8::rgb(0xD5, 0x5E, 0x00),
        VariableRole::Label => Rgba8::rgb(0x88, 0x44, 0x99),
        VariableRole::Calculated => theme.link_green,
    }
}

/// Metadata row representing one variable/column selectable in role-filtered pickers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableCandidate {
    pub col_index: usize,
    pub title: String,
    pub role: VariableRole,
    pub role_inferred: bool,
    pub selected: bool,
    pub is_response: bool,
}

/// Resolution chosen by the user when an analysis of the same family already exists on a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnalyzeDuplicateChoice {
    ModifyExisting(SheetId),
    CreateNew,
}

/// Interactive state for the searchable `Analyze...` dialog (`docs/evidence/user-interface.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyzeDialogState {
    pub source_sheet_id: Option<SheetId>,
    pub source_table_kind: Option<TableKind>,
    #[serde(default)]
    pub extracted_result_tab_id: Option<String>,
    pub search_query: String,
    pub category_filter: Option<AnalysisCategory>,
    pub selected_entry_id: Option<String>,
    pub selected_data_sets: Vec<usize>,
    pub variable_candidates: Vec<VariableCandidate>,
    pub role_filter: VariableRoleFilter,
    pub equation_id: String,
    #[serde(default)]
    pub pca_predictors: Vec<usize>,
    #[serde(default)]
    pub pcr_outcome_column: Option<usize>,
    #[serde(default = "default_true_flag")]
    pub correlation_all_pairs: bool,
    #[serde(default)]
    pub correlation_heatmap: bool,
    #[serde(default)]
    pub transform_x: Option<MathTransformPreset>,
    #[serde(default = "default_transform_y")]
    pub transform_y: Option<MathTransformPreset>,
    #[serde(default)]
    pub transform_error_propagation: ErrorPropagationMode,
    #[serde(default)]
    pub transform_formula: Option<String>,
    #[serde(default = "default_transform_target_archetype")]
    pub transform_target_archetype: TableKind,
    #[serde(default = "default_baseline_columns")]
    pub baseline_columns: Vec<usize>,
    #[serde(default = "default_baseline_operation")]
    pub baseline_operation: BaselineOperation,
    #[serde(default)]
    pub two_group_method: Option<TwoGroupMethod>,
    #[serde(default)]
    pub two_group_hypothetical: f64,
    #[serde(default)]
    pub mv_group_column: Option<usize>,
    #[serde(default)]
    pub mv_response_column: Option<usize>,
    #[serde(default)]
    pub contingency_method: Option<ContingencyMethod>,
    #[serde(default)]
    pub contingency_null_proportion: Option<f64>,
    #[serde(default)]
    pub contingency_expected_proportions: Option<Vec<f64>>,
    #[serde(default)]
    pub factorial_three_way: bool,
    #[serde(default)]
    pub factorial_repeated_measures: RepeatedMeasuresDesign,
    #[serde(default)]
    pub factorial_residual_covariance: ResidualCovariance,
    #[serde(default)]
    pub factorial_ss_type: SumOfSquaresType,
    #[serde(default)]
    pub factorial_multiplicity: MultiplicityMethod,
    #[serde(default)]
    pub cluster_distance: ClusterDistance,
    #[serde(default)]
    pub cluster_linkage: ClusterLinkage,
    #[serde(default)]
    pub cluster_k_means_k: Option<usize>,
    #[serde(default)]
    pub cluster_k_sweep_max: Option<usize>,
    #[serde(default = "default_true_flag")]
    pub pca_standardize: bool,
    #[serde(default)]
    pub corr_spearman_rank: bool,
    #[serde(default = "default_imputation_m")]
    pub imputation_m: usize,
    #[serde(default)]
    pub glm_family: GlmFamily,
    #[serde(default)]
    pub glm_reference_levels: Vec<(usize, String)>,
    #[serde(default = "default_true_flag")]
    pub glm_compute_vif: bool,
    #[serde(default)]
    pub cox_time_column: Option<usize>,
    #[serde(default)]
    pub cox_status_column: Option<usize>,
    #[serde(default)]
    pub cox_predictor_columns: Vec<usize>,
    #[serde(default)]
    pub cox_tie_method: CoxTieMethod,
    #[serde(default)]
    pub cox_reference_levels: Vec<(usize, String)>,
    #[serde(default)]
    pub km_mv_time_column: Option<usize>,
    #[serde(default)]
    pub km_mv_status_column: Option<usize>,
    #[serde(default = "default_true_flag")]
    pub km_log_rank_mantel_cox: bool,
    #[serde(default)]
    pub km_gehan_breslow_wilcoxon: bool,
    #[serde(default)]
    pub km_trend_test: bool,
    #[serde(default)]
    pub km_multiplicity_method: MultiplicityMethod,
    #[serde(default)]
    pub custom_dsl: Option<String>,
    #[serde(default)]
    pub fit_method: FitMethod,
    #[serde(default)]
    pub fit_weighting: WeightingScheme,
    #[serde(default)]
    pub parameter_rules: Vec<ParameterRule>,
    #[serde(default)]
    pub mv_x_columns: Vec<usize>,
    #[serde(default)]
    pub mv_y_column: Option<usize>,
    #[serde(default)]
    pub mv_group_columns: Vec<usize>,
    #[serde(default = "default_sample_seed")]
    pub sample_seed: u64,
    #[serde(default = "default_sample_size")]
    pub sample_size: usize,
    #[serde(default)]
    pub sample_stratify_column: Option<usize>,
    #[serde(default = "default_true_flag")]
    pub sample_proportional: bool,
    #[serde(default)]
    pub descriptive_aggregation: SubcolumnAggregation,
    #[serde(default)]
    pub pvalue_stack_method: PValueStackMethod,
    #[serde(default = "default_pvalue_stack_threshold")]
    pub pvalue_stack_threshold: f64,
    #[serde(default)]
    pub pvalue_stack_column: Option<usize>,
    #[serde(default)]
    pub compute_effect_sizes: Option<bool>,
    #[serde(default = "default_outlier_method")]
    pub outlier_method: String,
    #[serde(default = "default_outlier_alpha")]
    pub outlier_alpha: f64,
    #[serde(default = "default_outlier_fdr_q")]
    pub outlier_fdr_q_percent: f64,
    #[serde(default)]
    pub outlier_iterative: bool,
    #[serde(default = "default_tg_param_unpaired")]
    pub saved_two_group_param_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_param_paired")]
    pub saved_two_group_param_paired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_unpaired")]
    pub saved_two_group_nonparam_unpaired: TwoGroupMethod,
    #[serde(default = "default_tg_nonparam_paired")]
    pub saved_two_group_nonparam_paired: TwoGroupMethod,
    #[serde(skip)]
    pub explicit_response_column: Option<usize>,
}

fn default_outlier_method() -> String {
    String::from("rout")
}

pub(super) const fn default_outlier_alpha() -> f64 {
    0.05
}

pub(super) const fn default_outlier_fdr_q() -> f64 {
    1.0
}

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

pub(super) const fn default_one_way_param_unpaired() -> OneWayMethod {
    OneWayMethod::OrdinaryAnova
}

const fn default_pvalue_stack_threshold() -> f64 {
    0.05
}

const fn default_transform_y() -> Option<MathTransformPreset> {
    Some(MathTransformPreset::Log10)
}

const fn default_transform_target_archetype() -> TableKind {
    TableKind::Column
}

fn default_baseline_columns() -> Vec<usize> {
    vec![0]
}

const fn default_baseline_operation() -> BaselineOperation {
    BaselineOperation::Difference
}

const fn default_imputation_m() -> usize {
    5
}

const fn default_sample_seed() -> u64 {
    1
}

const fn default_sample_size() -> usize {
    10
}

impl Default for AnalyzeDialogState {
    fn default() -> Self {
        Self {
            source_sheet_id: None,
            source_table_kind: None,
            extracted_result_tab_id: None,
            search_query: String::new(),
            category_filter: None,
            selected_entry_id: None,
            selected_data_sets: Vec::new(),
            variable_candidates: Vec::new(),
            role_filter: VariableRoleFilter::All,
            equation_id: String::new(),
            pca_predictors: Vec::new(),
            pcr_outcome_column: None,
            correlation_all_pairs: true,
            correlation_heatmap: false,
            transform_x: None,
            transform_y: Some(MathTransformPreset::Log10),
            transform_error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            transform_formula: None,
            transform_target_archetype: TableKind::Column,
            baseline_columns: vec![0],
            baseline_operation: BaselineOperation::Difference,
            two_group_method: None,
            two_group_hypothetical: 0.0,
            mv_group_column: None,
            mv_response_column: None,
            contingency_method: None,
            contingency_null_proportion: None,
            contingency_expected_proportions: None,
            factorial_three_way: false,
            factorial_repeated_measures: RepeatedMeasuresDesign::None,
            factorial_residual_covariance: ResidualCovariance::CompoundSymmetry,
            factorial_ss_type: SumOfSquaresType::TypeIII,
            factorial_multiplicity: MultiplicityMethod::None,
            cluster_distance: ClusterDistance::Euclidean,
            cluster_linkage: ClusterLinkage::Ward,
            cluster_k_means_k: Some(3),
            cluster_k_sweep_max: None,
            pca_standardize: true,
            corr_spearman_rank: false,
            imputation_m: 5,
            glm_family: GlmFamily::Linear,
            glm_reference_levels: Vec::new(),
            glm_compute_vif: true,
            cox_time_column: None,
            cox_status_column: None,
            cox_predictor_columns: Vec::new(),
            cox_tie_method: CoxTieMethod::Efron,
            cox_reference_levels: Vec::new(),
            km_mv_time_column: None,
            km_mv_status_column: None,
            km_log_rank_mantel_cox: true,
            km_gehan_breslow_wilcoxon: false,
            km_trend_test: false,
            km_multiplicity_method: MultiplicityMethod::None,
            custom_dsl: None,
            fit_method: FitMethod::LeastSquares,
            fit_weighting: WeightingScheme::None,
            parameter_rules: Vec::new(),
            mv_x_columns: Vec::new(),
            mv_y_column: None,
            mv_group_columns: Vec::new(),
            sample_seed: 1,
            sample_size: 10,
            sample_stratify_column: None,
            sample_proportional: true,
            descriptive_aggregation: SubcolumnAggregation::RowMeans,
            pvalue_stack_method: PValueStackMethod::default(),
            pvalue_stack_threshold: default_pvalue_stack_threshold(),
            pvalue_stack_column: None,
            compute_effect_sizes: None,
            outlier_method: default_outlier_method(),
            outlier_alpha: default_outlier_alpha(),
            outlier_fdr_q_percent: default_outlier_fdr_q(),
            outlier_iterative: false,
            saved_two_group_param_unpaired: default_tg_param_unpaired(),
            saved_two_group_param_paired: default_tg_param_paired(),
            saved_two_group_nonparam_unpaired: default_tg_nonparam_unpaired(),
            saved_two_group_nonparam_paired: default_tg_nonparam_paired(),
            explicit_response_column: None,
        }
    }
}

impl AnalyzeDialogState {
    fn reset_r9_options(&mut self) {
        self.two_group_method = None;
        self.two_group_hypothetical = 0.0;
        self.mv_group_column = None;
        self.mv_response_column = None;
        self.contingency_method = None;
        self.contingency_null_proportion = None;
        self.contingency_expected_proportions = None;
        self.factorial_three_way = false;
        self.factorial_repeated_measures = RepeatedMeasuresDesign::None;
        self.factorial_residual_covariance = ResidualCovariance::CompoundSymmetry;
        self.factorial_ss_type = SumOfSquaresType::TypeIII;
        self.factorial_multiplicity = MultiplicityMethod::None;
        self.cluster_distance = ClusterDistance::Euclidean;
        self.cluster_linkage = ClusterLinkage::Ward;
        self.cluster_k_means_k = Some(3);
        self.cluster_k_sweep_max = None;
        self.pca_standardize = true;
        self.corr_spearman_rank = false;
        self.imputation_m = 5;
        self.glm_family = GlmFamily::Linear;
        self.glm_reference_levels.clear();
        self.glm_compute_vif = true;
        self.cox_time_column = None;
        self.cox_status_column = None;
        self.cox_predictor_columns.clear();
        self.cox_tie_method = CoxTieMethod::Efron;
        self.cox_reference_levels.clear();
        self.km_mv_time_column = None;
        self.km_mv_status_column = None;
        self.km_log_rank_mantel_cox = true;
        self.km_gehan_breslow_wilcoxon = false;
        self.km_trend_test = false;
        self.km_multiplicity_method = MultiplicityMethod::None;
        self.transform_target_archetype = TableKind::Column;
        self.baseline_columns = vec![0];
        self.baseline_operation = BaselineOperation::Difference;
        self.custom_dsl = None;
        self.fit_method = FitMethod::LeastSquares;
        self.fit_weighting = WeightingScheme::None;
        self.parameter_rules.clear();
        self.mv_x_columns.clear();
        self.mv_y_column = None;
        self.mv_group_columns.clear();
        self.sample_seed = 1;
        self.sample_size = 10;
        self.sample_stratify_column = None;
        self.sample_proportional = true;
        self.descriptive_aggregation = SubcolumnAggregation::RowMeans;
        self.pvalue_stack_method = PValueStackMethod::default();
        self.pvalue_stack_threshold = default_pvalue_stack_threshold();
        self.outlier_method = default_outlier_method();
        self.outlier_alpha = default_outlier_alpha();
        self.outlier_fdr_q_percent = default_outlier_fdr_q();
        self.outlier_iterative = false;
        self.saved_two_group_param_unpaired = default_tg_param_unpaired();
        self.saved_two_group_param_paired = default_tg_param_paired();
        self.saved_two_group_nonparam_unpaired = default_tg_nonparam_unpaired();
        self.saved_two_group_nonparam_paired = default_tg_nonparam_paired();
        self.pvalue_stack_column = None;
        self.compute_effect_sizes = None;
        self.explicit_response_column = None;
    }

    /// Populate defaults and variable candidates for `sheet`.
    pub fn open_for_data_sheet(&mut self, sheet: &DataSheet) {
        self.source_sheet_id = Some(sheet.id.clone());
        self.source_table_kind = Some(sheet.kind());
        self.extracted_result_tab_id = None;
        self.search_query.clear();
        self.category_filter = None;
        self.selected_data_sets.clear();
        self.role_filter = VariableRoleFilter::All;
        self.pca_predictors.clear();
        self.pcr_outcome_column = None;
        self.correlation_all_pairs = true;
        self.correlation_heatmap = false;
        self.transform_x = None;
        self.transform_y = Some(MathTransformPreset::Log10);
        self.transform_error_propagation = ErrorPropagationMode::ReplicateEndpoints;
        self.transform_formula = None;
        self.reset_r9_options();
        if self.equation_id.is_empty() {
            self.equation_id = String::from("log_agonist_vs_response_4pl");
        }
        self.variable_candidates = collect_variable_candidates(sheet);
        let first_id = self.compatible_entries().first().map(|item| item.id.to_owned());
        self.selected_entry_id = first_id;
    }

    /// Populate defaults and column candidates from a non-analyzable [`ResultTab`] for chained GUI `Analyze` (`docs/gui-workflows.md` §3.3).
    pub fn open_for_result_tab(&mut self, result: &ResultSheet, tab: &ResultTab) {
        self.source_sheet_id = Some(result.id.clone());
        self.extracted_result_tab_id = Some(tab.id.clone());
        let has_text = tab.rows.iter().flatten().any(|cell| matches!(cell, ResultCell::Text(_) | ResultCell::Boolean(_)));
        self.source_table_kind = Some(if has_text { TableKind::MultipleVariables } else { TableKind::Column });
        self.search_query.clear();
        self.category_filter = None;
        self.selected_data_sets.clear();
        self.role_filter = VariableRoleFilter::All;
        self.pca_predictors = if matches!(result.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })) {
            result.analysis.selected_data_sets.clone()
        } else {
            Vec::new()
        };
        self.pcr_outcome_column = None;
        self.correlation_all_pairs = true;
        self.correlation_heatmap = false;
        self.transform_x = None;
        self.transform_y = Some(MathTransformPreset::Log10);
        self.transform_error_propagation = ErrorPropagationMode::ReplicateEndpoints;
        self.transform_formula = None;
        self.reset_r9_options();
        if self.equation_id.is_empty() {
            self.equation_id = String::from("log_agonist_vs_response_4pl");
        }
        self.variable_candidates = collect_result_tab_candidates(tab);
        let first_id = self.compatible_entries().first().map(|item| item.id.to_owned());
        self.selected_entry_id = first_id;
    }

    /// Switch the selected catalog entry (`analyze.entry.<id>`), resetting PCR selection cleanly when entering `stats.pcr`.
    pub fn select_catalog_entry(&mut self, entry_id: &str) {
        let entering_pcr = entry_id == "stats.pcr" && self.selected_entry_id.as_deref() != Some("stats.pcr");
        self.selected_entry_id = Some(entry_id.to_owned());
        if entering_pcr {
            self.selected_data_sets.clear();
            self.pca_predictors.clear();
            self.pcr_outcome_column = None;
            for cand in &mut self.variable_candidates {
                cand.selected = false;
            }
        }
    }

    /// Toggle a MV predictor column for PCA/PCR while preserving disjoint PCR outcome selection.
    pub fn toggle_pca_predictor(&mut self, col: usize) {
        if let Some(pos) = self.pca_predictors.iter().position(|&c| c == col) {
            self.pca_predictors.remove(pos);
        } else {
            self.pca_predictors.push(col);
            self.pca_predictors.sort_unstable();
        }
        if let Some(pos) = self.selected_data_sets.iter().position(|&c| c == col) {
            self.selected_data_sets.remove(pos);
        } else {
            self.selected_data_sets.push(col);
            self.selected_data_sets.sort_unstable();
            if self.pcr_outcome_column == Some(col) {
                self.pcr_outcome_column = None;
            }
        }
        for cand in &mut self.variable_candidates {
            cand.selected = self.selected_data_sets.contains(&cand.col_index);
        }
    }

    /// Select (or toggle off) the disjoint PCR outcome column with R10 predictor leakage guard.
    pub fn select_pcr_outcome(&mut self, col: usize) -> Result<(), String> {
        if self.pca_predictors.contains(&col) || self.selected_data_sets.contains(&col) {
            return Err(format!("Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."));
        }
        self.pcr_outcome_column = if self.pcr_outcome_column == Some(col) { None } else { Some(col) };
        if let Some(outcome) = self.pcr_outcome_column {
            self.set_response_column(outcome);
        }
        Ok(())
    }

    /// Apply modal field updates from `ui.answer_dialog` / interactive controls.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply modal field updates using regional numeric parsing (`preset`).
    fn remember_two_group_submethod(&mut self, method: TwoGroupMethod) {
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

    fn toggle_two_group_design(&mut self, gaussian: bool, paired: bool) {
        if let Some(cur) = self.two_group_method {
            self.remember_two_group_submethod(cur);
            if let TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } = cur {
                self.two_group_method =
                    Some(if gaussian { TwoGroupMethod::OneSampleT { hypothetical } } else { TwoGroupMethod::OneSampleWilcoxon { hypothetical } });
                return;
            }
        }
        self.two_group_method = Some(match (gaussian, paired) {
            (true, false) => self.saved_two_group_param_unpaired,
            (true, true) => self.saved_two_group_param_paired,
            (false, false) => self.saved_two_group_nonparam_unpaired,
            (false, true) => self.saved_two_group_nonparam_paired,
        });
    }

    /// Apply modal field updates using regional numeric parsing (`preset`).
    ///
    /// # Errors
    /// Returns an error if an explicit field fails validation or points to an unknown key.
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        let has_explicit_predictors =
            ["selected_data_sets", "analyze.selected_data_sets", "pca_predictors", "analyze.pca_predictors"].iter().any(|k| fields.contains_key(*k));
        for key in ["entry_id", "selected_entry_id", "analysis", "analyze.entry", "analyze.entries_list"] {
            if let Some(s) = fields.get(key).and_then(Value::as_str) {
                if s == "stats.pcr" && !has_explicit_predictors {
                    self.select_catalog_entry(s);
                } else {
                    self.selected_entry_id = Some(s.to_owned());
                }
            }
        }
        // Apply method selectors before sub-parameters (`alpha`, `fdr_q_percent`, `rout_q`) so JSON
        // object key ordering never resets a caller-provided Q% or alpha (`RV-76`).
        for key in ["outlier_method", "analyze.outlier.method", "fit_method", "analyze.fit.method", "method"] {
            if let Some(val) = fields.get(key) {
                if matches!(key, "outlier_method" | "analyze.outlier.method")
                    || (key == "method" && self.selected_entry_id.as_deref() == Some("stats.outliers"))
                {
                    let raw = val.as_str().ok_or_else(|| String::from("`outlier_method` must be a string"))?;
                    let (m, iter_opt) = parse_outlier_method_str(raw)?;
                    self.outlier_method = m.to_owned();
                    if let Some(it) = iter_opt {
                        self.outlier_iterative = it;
                    }
                } else if matches!(key, "fit_method" | "analyze.fit.method")
                    || (key == "method" && self.selected_entry_id.as_deref() == Some("fit.nonlinear"))
                {
                    self.fit_method = parse_fit_method(val)?;
                }
            }
        }
        for (key, val) in fields {
            match key.as_str() {
                "entry_id"
                | "selected_entry_id"
                | "analysis"
                | "analyze.entry"
                | "analyze.entries_list"
                | "outlier_method"
                | "analyze.outlier.method"
                | "fit_method"
                | "analyze.fit.method" => {}
                "search" | "search_query" | "analyze.search" => {
                    if let Some(s) = val.as_str() {
                        self.search_query = s.to_owned();
                    }
                }
                "equation_id" | "analyze.equation_id" | "analyze.fit.model_id" => {
                    if let Some(s) = val.as_str() {
                        self.equation_id = s.to_owned();
                    }
                }
                "correlation_all_pairs" | "analyze.correlation_all_pairs" => {
                    if let Some(b) = val.as_bool() {
                        self.correlation_all_pairs = b;
                        if !b {
                            self.correlation_heatmap = false;
                        }
                    }
                }
                "correlation_heatmap" | "analyze.correlation_heatmap" | "analyze.correlation.heatmap" => {
                    if let Some(b) = val.as_bool() {
                        if b && !self.correlation_all_pairs {
                            return Err(String::from("Create correlation heatmap is disabled in X vs every Y mode."));
                        }
                        self.correlation_heatmap = b;
                    }
                }
                "selected_data_sets"
                | "analyze.selected_data_sets"
                | "analyze.transform.selected_data_sets"
                | "pca_predictors"
                | "analyze.pca_predictors" => {
                    let cols = parse_usize_list(val, key)?;
                    self.selected_data_sets = cols.clone();
                    if matches!(key.as_str(), "pca_predictors" | "analyze.pca_predictors") {
                        self.pca_predictors = cols;
                    }
                    for cand in &mut self.variable_candidates {
                        cand.selected = self.selected_data_sets.contains(&cand.col_index);
                    }
                }
                "pcr_outcome_column" | "response_column" | "analyze.pcr_outcome_column" => {
                    if let Some(col) = val.as_u64().map(|n| n as usize) {
                        if self.pca_predictors.contains(&col) || self.selected_data_sets.contains(&col) {
                            return Err(format!(
                                "Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."
                            ));
                        }
                        self.pcr_outcome_column = Some(col);
                        self.set_response_column(col);
                    }
                }
                "formula" | "transform_formula" | "analyze.transform.formula" | "filter_formula" | "analyze.transform.filter_formula" => {
                    if val.is_null() {
                        self.transform_formula = None;
                    } else {
                        let raw = val.as_str().ok_or_else(|| String::from("`formula` must be a string or null"))?.trim();
                        if raw.is_empty() {
                            self.transform_formula = None;
                        } else {
                            self.transform_formula = Some(raw.to_owned());
                            if self.selected_entry_id.as_deref() != Some("transform.select") {
                                let _ = apply_transform_formula_str(raw, &mut self.transform_x, &mut self.transform_y);
                            }
                        }
                    }
                }
                "target_archetype" | "analyze.transform.target_archetype" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`target_archetype` must be a string"))?;
                    self.transform_target_archetype = parse_table_kind_str(raw)?;
                }
                "baseline_columns" | "analyze.transform.baseline_columns" => {
                    self.baseline_columns = parse_usize_list(val, key)?;
                }
                "baseline_operation" | "operation" | "analyze.transform.baseline_operation" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`baseline_operation` must be a string"))?;
                    self.baseline_operation = parse_baseline_operation_str(raw)?;
                }
                "x_transform" | "analyze.transform.x_transform" | "transform_x" => {
                    self.transform_x = parse_math_preset_opt(val)?;
                }
                "y_transform" | "analyze.transform.y_transform" | "transform_y" => {
                    self.transform_y = parse_math_preset_opt(val)?;
                }
                "error_propagation" | "analyze.transform.error_propagation" => {
                    self.transform_error_propagation = parse_error_prop_value(val)?;
                }
                _ => self.apply_r9_analyze_field(key, val, preset)?,
            }
        }
        Ok(())
    }

    fn apply_r9_analyze_field(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<(), String> {
        if let Some(col_raw) = key.strip_prefix("analyze.pca.pred.")
            && let Ok(col) = col_raw.parse::<usize>()
        {
            let want_on = val.as_bool().unwrap_or_else(|| !self.selected_data_sets.contains(&col));
            if want_on != self.selected_data_sets.contains(&col) {
                self.toggle_pca_predictor(col);
            }
            return Ok(());
        }
        if let Some(col_raw) = key.strip_prefix("analyze.pcr.outcome.")
            && let Ok(col) = col_raw.parse::<usize>()
        {
            let selected = val.as_bool().unwrap_or(true);
            if selected {
                if self.pca_predictors.contains(&col) || self.selected_data_sets.contains(&col) {
                    return Err(format!(
                        "Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."
                    ));
                }
                self.pcr_outcome_column = Some(col);
                self.set_response_column(col);
            } else if self.pcr_outcome_column == Some(col) {
                self.pcr_outcome_column = None;
            }
            return Ok(());
        }
        match key {
            "assume_gaussian" | "analyze.assume_gaussian" => {
                let g = val.as_bool().ok_or_else(|| String::from("`assume_gaussian` must be boolean"))?;
                let paired = matches!(
                    self.two_group_method,
                    Some(
                        TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest
                    )
                );
                self.toggle_two_group_design(g, paired);
            }
            "paired_or_matched" | "analyze.paired_or_matched" => {
                let paired = val.as_bool().ok_or_else(|| String::from("`paired_or_matched` must be boolean"))?;
                let gaussian = !matches!(
                    self.two_group_method,
                    Some(
                        TwoGroupMethod::MannWhitneyU
                            | TwoGroupMethod::WilcoxonSignedRank
                            | TwoGroupMethod::PairedSignTest
                            | TwoGroupMethod::KolmogorovSmirnov
                            | TwoGroupMethod::OneSampleWilcoxon { .. }
                    )
                );
                self.toggle_two_group_design(gaussian, paired);
            }
            "descriptive_subcolumn_agg" | "subcolumn_agg" | "analyze.descriptive.subcolumn_agg" => {
                self.descriptive_aggregation = parse_subcolumn_aggregation_val(val)?;
            }
            "two_group_method" | "analyze.two_group.method" | "analyze.two_group_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`two_group_method` must be a string"))?;
                let parsed = parse_two_group_method_str(raw, self.two_group_hypothetical)?;
                self.remember_two_group_submethod(parsed);
                self.two_group_method = Some(parsed);
            }
            "compute_effect_sizes" | "analyze.compute_effect_sizes" | "analyze.two_group.compute_effect_sizes" => {
                let b = val.as_bool().ok_or_else(|| String::from("`compute_effect_sizes` must be boolean"))?;
                self.compute_effect_sizes = Some(b);
            }
            "hypothetical" | "two_group_hypothetical" | "analyze.two_group.hypothetical" => {
                self.two_group_hypothetical = parse_finite_f64_with_locale(val, key, preset)?;
            }
            "mv_group_column" | "analyze.mv_group_column" | "analyze.km.mv_group_column" => {
                self.mv_group_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "mv_response_column" | "analyze.mv_response_column" => {
                self.mv_response_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "contingency_method" | "analyze.contingency.method" | "analyze.contingency_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`contingency_method` must be a string"))?;
                self.contingency_method = Some(parse_contingency_method_name(raw)?);
            }
            "kappa_weighting" | "analyze.contingency.kappa_weighting" => {
                let w = parse_kappa_weighting(val)?;
                self.contingency_method = Some(ContingencyMethod::CohenKappa { weighting: w });
            }
            "weighting" => {
                if self.selected_entry_id.as_deref() == Some("stats.contingency") {
                    let w = parse_kappa_weighting(val)?;
                    self.contingency_method = Some(ContingencyMethod::CohenKappa { weighting: w });
                } else {
                    self.fit_weighting = parse_weighting_scheme(val)?;
                }
            }
            "fit_weighting" | "analyze.fit.weighting" => {
                self.fit_weighting = parse_weighting_scheme(val)?;
            }
            "null_proportion" | "analyze.contingency.null_proportion" | "analyze.null_proportion" => {
                self.contingency_null_proportion = parse_opt_finite_f64_with_locale(val, key, preset)?;
            }
            "expected_proportions" | "analyze.contingency.expected_proportions" | "analyze.expected_proportions" => {
                self.contingency_expected_proportions = if val.is_null() { None } else { Some(parse_f64_list_with_locale(val, key, preset)?) };
            }
            "three_way" | "analyze.factorial.three_way" => {
                self.factorial_three_way = val.as_bool().ok_or_else(|| String::from("`three_way` must be boolean"))?;
            }
            "repeated_measures" | "analyze.factorial.repeated_measures" => {
                let raw = val.as_str().ok_or_else(|| String::from("`repeated_measures` must be a string"))?;
                self.factorial_repeated_measures = parse_rm_design_str(raw)?;
            }
            "residual_covariance" | "factorial_residual_covariance" | "analyze.factorial.residual_covariance" => {
                let raw = val.as_str().ok_or_else(|| String::from("`residual_covariance` must be a string"))?;
                self.factorial_residual_covariance = parse_residual_covariance_str(raw)?;
            }
            "ss_type" | "sum_of_squares" | "analyze.factorial.ss_type" => {
                let raw = val.as_str().ok_or_else(|| String::from("`ss_type` must be a string"))?;
                self.factorial_ss_type = parse_ss_type_str(raw)?;
            }
            "multiplicity_method" | "analyze.multiplicity_method" | "analyze.factorial.multiplicity" => {
                if self.selected_entry_id.as_deref() == Some("stats.pvalue_stack") {
                    self.pvalue_stack_method = parse_pvalue_stack_method_val(val)?;
                } else {
                    let raw = val.as_str().ok_or_else(|| String::from("`multiplicity_method` must be a string"))?;
                    let parsed = parse_multiplicity_method_name(raw, 0)?;
                    self.factorial_multiplicity = parsed;
                    self.km_multiplicity_method = parsed;
                }
            }
            "distance" | "cluster_distance" | "analyze.cluster.distance" => {
                let raw = val.as_str().ok_or_else(|| String::from("`distance` must be a string"))?;
                self.cluster_distance = parse_cluster_distance_str(raw)?;
            }
            "linkage" | "cluster_linkage" | "analyze.cluster.linkage" => {
                let raw = val.as_str().ok_or_else(|| String::from("`linkage` must be a string"))?;
                self.cluster_linkage = parse_cluster_linkage_str(raw)?;
            }
            "k_means_k" | "k" | "analyze.cluster.k_means_k" => {
                self.cluster_k_means_k = if val.is_null() { None } else { Some(parse_usize_min(val, key, 1)?) };
            }
            "k_sweep_max" | "analyze.cluster.k_sweep_max" => {
                self.cluster_k_sweep_max = if val.is_null() { None } else { Some(parse_usize_min(val, key, 2)?) };
            }
            "standardize" | "pca_standardize" | "analyze.pca.standardize" => {
                self.pca_standardize = val.as_bool().ok_or_else(|| String::from("`standardize` must be boolean"))?;
            }
            "spearman_rank" | "analyze.corr.spearman_rank" | "analyze.correlation.spearman_rank" => {
                self.corr_spearman_rank = val.as_bool().ok_or_else(|| String::from("`spearman_rank` must be boolean"))?;
            }
            "imputations_m" | "m" | "analyze.mi.imputations_m" | "analyze.imputation.m" => {
                self.imputation_m = parse_usize_min(val, key, 1)?;
            }
            "family" | "glm_family" | "analyze.glm.family" => {
                let raw = val.as_str().ok_or_else(|| String::from("`glm_family` must be a string"))?;
                self.glm_family = parse_glm_family_str(raw)?;
            }
            "reference_levels" | "analyze.reference_levels" => {
                let refs = parse_reference_levels(val)?;
                self.glm_reference_levels.clone_from(&refs);
                self.cox_reference_levels = refs;
            }
            "compute_vif" | "analyze.glm.compute_vif" => {
                self.glm_compute_vif = val.as_bool().ok_or_else(|| String::from("`compute_vif` must be boolean"))?;
            }
            "time_column" | "cox_time_column" | "analyze.cox.time_column" => {
                self.cox_time_column = Some(parse_usize_min(val, key, 0)?);
            }
            "status_column" | "cox_status_column" | "analyze.cox.status_column" => {
                self.cox_status_column = Some(parse_usize_min(val, key, 0)?);
            }
            "predictor_columns" | "cox_predictor_columns" | "analyze.cox.predictor_columns" => {
                self.cox_predictor_columns = parse_usize_list(val, key)?;
            }
            "tie_method" | "cox_tie_method" | "analyze.cox.tie_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`tie_method` must be a string"))?;
                self.cox_tie_method = parse_cox_tie_method_str(raw)?;
            }
            "trend_test" | "km_trend_test" | "analyze.km.trend_test" | "log_rank_for_trend" | "analyze.km.log_rank_for_trend" => {
                self.km_trend_test = val.as_bool().ok_or_else(|| String::from("`trend_test` must be boolean"))?;
            }
            "mv_time_column" | "km_mv_time_column" | "analyze.km.mv_time_column" => {
                self.km_mv_time_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "mv_status_column" | "km_mv_status_column" | "analyze.km.mv_status_column" => {
                self.km_mv_status_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "log_rank_mantel_cox" | "km_log_rank_mantel_cox" | "analyze.km.log_rank_mantel_cox" => {
                self.km_log_rank_mantel_cox = val.as_bool().ok_or_else(|| String::from("`log_rank_mantel_cox` must be boolean"))?;
            }
            "gehan_breslow_wilcoxon" | "km_gehan_breslow_wilcoxon" | "analyze.km.gehan_breslow_wilcoxon" => {
                self.km_gehan_breslow_wilcoxon = val.as_bool().ok_or_else(|| String::from("`gehan_breslow_wilcoxon` must be boolean"))?;
            }
            "km_multiplicity_method" | "analyze.km.multiplicity_method" => {
                let raw = val.as_str().ok_or_else(|| String::from("`multiplicity_method` must be a string"))?;
                self.km_multiplicity_method = parse_multiplicity_method_name(raw, 0)?;
            }
            "custom_dsl" | "analyze.fit.custom_dsl" => {
                self.custom_dsl = val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(ToOwned::to_owned);
            }
            "parameter_rules" | "analyze.fit.parameter_rules" => {
                self.parameter_rules = parse_parameter_rules(val)?;
            }
            "mv_x_columns" | "analyze.fit.mv_x_columns" => {
                self.mv_x_columns = parse_usize_list(val, key)?;
            }
            "mv_y_column" | "analyze.fit.mv_y_column" => {
                self.mv_y_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "mv_group_columns" | "analyze.fit.mv_group_columns" => {
                self.mv_group_columns = parse_usize_list(val, key)?;
            }
            "sample_size" | "analyze.sample.sample_size" => {
                self.sample_size = parse_usize_min(val, key, 1)?;
            }
            "seed" | "sample_seed" | "analyze.sample.seed" => {
                self.sample_seed = parse_u64_val(val, key)?;
            }
            "stratify_column" | "analyze.sample.stratify_column" => {
                self.sample_stratify_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "proportional_allocation" | "analyze.sample.proportional_allocation" => {
                self.sample_proportional = val.as_bool().ok_or_else(|| String::from("`proportional_allocation` must be boolean"))?;
            }
            _ => self.apply_pvalue_stack_analyze_field(key, val, preset)?,
        }
        Ok(())
    }

    fn apply_pvalue_stack_analyze_field(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<(), String> {
        let is_pstack = self.selected_entry_id.as_deref() == Some("stats.pvalue_stack");
        let is_outliers = self.selected_entry_id.as_deref() == Some("stats.outliers");
        let is_nonlinear = self.selected_entry_id.as_deref() == Some("fit.nonlinear");
        match key {
            "outlier_fdr_q_percent" | "analyze.outlier.fdr_q_percent" | "analyze.fit.rout_q" => {
                let q = parse_outlier_fdr_q_percent(val)?;
                self.outlier_fdr_q_percent = q;
                if let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut self.fit_method {
                    *fdr_q_percent = q;
                }
                return Ok(());
            }
            "outlier_alpha" | "analyze.outlier.alpha" => {
                self.outlier_alpha = parse_outlier_alpha(val)?;
                return Ok(());
            }
            "outlier_iterative" | "analyze.outlier.iterative" => {
                self.outlier_iterative = val.as_bool().ok_or_else(|| String::from("`outlier_iterative` must be boolean"))?;
                return Ok(());
            }
            "fdr_q_percent" | "q_percent" | "rout_q" if is_outliers || is_nonlinear => {
                let q = parse_outlier_fdr_q_percent(val)?;
                if is_outliers {
                    self.outlier_fdr_q_percent = q;
                }
                if is_nonlinear {
                    let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut self.fit_method else {
                        return Err(String::from("`fdr_q_percent` requires ROUT outlier elimination fit method"));
                    };
                    *fdr_q_percent = q;
                }
                return Ok(());
            }
            "alpha" if is_outliers => {
                self.outlier_alpha = parse_outlier_alpha(val)?;
                return Ok(());
            }
            "iterative" if is_outliers => {
                self.outlier_iterative = val.as_bool().ok_or_else(|| String::from("`iterative` must be boolean"))?;
                return Ok(());
            }
            "method" if is_outliers || is_nonlinear => {
                return Ok(());
            }
            "pvalue_stack_method" | "analyze.pvalue_stack.method" | "analyze.pvalue_stack.multiplicity_method" => {
                self.pvalue_stack_method = parse_pvalue_stack_method_val(val)?;
            }
            "pvalue_stack_threshold" | "analyze.pvalue_stack.threshold" => {
                let t = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0 < t && t < 1.0) {
                    return Err(format!("`{key}` ({t}) must lie strictly in (0, 1)"));
                }
                self.pvalue_stack_threshold = t;
            }
            "pvalue_stack_column" | "analyze.pvalue_stack.column" => {
                self.pvalue_stack_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "method" if is_pstack => {
                self.pvalue_stack_method = parse_pvalue_stack_method_val(val)?;
            }
            "threshold" | "alpha" | "q" | "fdr_q" if is_pstack => {
                let t = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0 < t && t < 1.0) {
                    return Err(format!("`{key}` ({t}) must lie strictly in (0, 1)"));
                }
                self.pvalue_stack_threshold = t;
            }
            "column" if is_pstack => {
                self.pvalue_stack_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            other => return Err(format!("unknown Analyze field `{other}`")),
        }
        Ok(())
    }

    /// Filter [`ANALYSIS_CATALOG`] by source table compatibility, category chip, and search query.
    #[must_use]
    pub fn compatible_entries(&self) -> Vec<&'static AnalysisCatalogItem> {
        let query = self.search_query.trim().to_ascii_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        ANALYSIS_CATALOG.iter().filter(|item| self.item_matches(item, &terms)).collect()
    }

    fn item_matches(&self, item: &AnalysisCatalogItem, query_terms: &[&str]) -> bool {
        if let Some(cat) = self.category_filter
            && item.category != cat
        {
            return false;
        }
        if let Some(table_kind) = self.source_table_kind {
            let Some(kind) = default_analysis_kind_for_id(item.id) else {
                return false;
            };
            if !kind.is_compatible_with(table_kind) {
                return false;
            }
        }
        if query_terms.is_empty() {
            return true;
        }
        let haystack = format!("{} {} {} {}", item.id, item.label, item.category.label(), item.keywords).to_ascii_lowercase();
        query_terms.iter().all(|term| haystack.contains(term))
    }

    /// Explicit human-readable diagnostic banner when no catalog items match the active filters.
    #[must_use]
    pub fn empty_feedback(&self) -> Option<String> {
        if !self.compatible_entries().is_empty() {
            return None;
        }
        let table_label = self.source_table_kind.map_or("active sheet", TableKind::label);
        let q = self.search_query.trim();
        if q.is_empty() {
            Some(format!("No analyses in the selected category are compatible with {table_label} tables."))
        } else {
            Some(format!("No analyses compatible with {table_label} match \"{q}\"."))
        }
    }

    /// Role-filtered variables for `MultipleVariablesTable` column pickers.
    #[must_use]
    pub fn filtered_variables(&self) -> Vec<&VariableCandidate> {
        self.variable_candidates.iter().filter(|v| self.role_filter.matches(v.role)).collect()
    }

    /// Toggle inclusion of column `col_index` in `variable_candidates` and `selected_data_sets`.
    pub fn toggle_variable(&mut self, col_index: usize) {
        if let Some(var) = self.variable_candidates.iter_mut().find(|v| v.col_index == col_index) {
            var.selected = !var.selected;
        }
        self.selected_data_sets = self.variable_candidates.iter().filter(|v| v.selected).map(|v| v.col_index).collect();
    }

    /// Assign a primary response column (for GLM / Cox / Factorial modeling on MV tables).
    pub fn set_response_column(&mut self, col_index: usize) {
        self.explicit_response_column = Some(col_index);
        for var in &mut self.variable_candidates {
            var.is_response = var.col_index == col_index;
        }
    }

    /// Synthesize the configured [`AnalysisKind`] from the currently selected catalog entry and
    /// variable/equation choices.
    #[must_use]
    pub fn build_analysis_kind(&self) -> Option<AnalysisKind> {
        let id = self.selected_entry_id.as_deref()?;
        let mut kind = default_analysis_kind_for_id(id)?;
        let response_col = self.variable_candidates.iter().find(|v| v.is_response).map(|v| v.col_index);
        let predictor_cols: Vec<usize> = self
            .variable_candidates
            .iter()
            .filter(|v| v.selected && !v.is_response && !matches!(v.role, VariableRole::Label))
            .map(|v| v.col_index)
            .collect();

        match &mut kind {
            AnalysisKind::OutlierDetection(out) => {
                *out = if self.outlier_method == "grubbs" {
                    OutlierSpec::Grubbs { alpha: self.outlier_alpha, iterative: self.outlier_iterative }
                } else {
                    OutlierSpec::Rout { fdr_q_percent: self.outlier_fdr_q_percent }
                };
            }
            AnalysisKind::DescriptiveStats(desc) => {
                desc.aggregation = self.descriptive_aggregation;
            }
            AnalysisKind::TwoGroupComparison(tg) => {
                if let Some(m) = self.two_group_method {
                    tg.method = m;
                }
                if let Some(ces) = self.compute_effect_sizes {
                    tg.compute_effect_sizes = ces;
                }
                if self.mv_group_column.is_some() {
                    tg.mv_group_column = self.mv_group_column;
                } else if self.source_table_kind == Some(TableKind::MultipleVariables) {
                    tg.mv_group_column = self.variable_candidates.iter().find(|v| v.role == VariableRole::Categorical).map(|v| v.col_index);
                }
                if self.mv_response_column.is_some() {
                    tg.mv_response_column = self.mv_response_column;
                } else if self.source_table_kind == Some(TableKind::MultipleVariables) {
                    tg.mv_response_column = response_col.or_else(|| {
                        self.variable_candidates
                            .iter()
                            .rev()
                            .find(|v| !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
                            .map(|v| v.col_index)
                    });
                }
            }
            AnalysisKind::OneWayAnova(anova) => {
                if let Some(ces) = self.compute_effect_sizes {
                    anova.compute_effect_sizes = ces;
                }
            }
            AnalysisKind::Contingency(cont) => {
                if let Some(m) = self.contingency_method {
                    cont.method = m;
                }
                if self.contingency_null_proportion.is_some() {
                    cont.null_proportion = self.contingency_null_proportion;
                }
                if self.contingency_expected_proportions.is_some() {
                    cont.expected_proportions.clone_from(&self.contingency_expected_proportions);
                }
            }
            AnalysisKind::NonlinearRegression(fit) => self.populate_nonlinear_kind(fit),
            AnalysisKind::GeneralizedLinearModel(glm) => self.populate_glm_kind(glm, response_col, &predictor_cols),
            AnalysisKind::Survival(surv) => self.populate_survival_kind(surv, response_col, &predictor_cols),
            AnalysisKind::FactorialAnova(fac) => {
                fac.three_way = self.factorial_three_way;
                if fac.three_way {
                    fac.max_interaction_order = fac.max_interaction_order.max(3);
                }
                fac.repeated_measures = self.factorial_repeated_measures;
                fac.residual_covariance = self.factorial_residual_covariance;
                fac.ss_type = self.factorial_ss_type;
                fac.multiplicity.method = self.factorial_multiplicity;
                if let Some(ces) = self.compute_effect_sizes {
                    fac.compute_effect_sizes = ces;
                }
                if self.source_table_kind == Some(TableKind::MultipleVariables) {
                    let cat_cols: Vec<usize> =
                        self.variable_candidates.iter().filter(|v| v.role == VariableRole::Categorical).map(|v| v.col_index).collect();
                    let factor_limit = if fac.three_way { 3 } else { 2 };
                    if !cat_cols.is_empty() {
                        fac.multifactor_columns = cat_cols.into_iter().take(factor_limit).collect();
                    } else if !predictor_cols.is_empty() {
                        fac.multifactor_columns = predictor_cols;
                    }
                    fac.response_column = response_col;
                } else if !predictor_cols.is_empty() {
                    fac.multifactor_columns = predictor_cols;
                    fac.response_column = response_col;
                }
            }
            AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max }) => {
                *k_means_k = self.cluster_k_means_k;
                *distance = self.cluster_distance;
                *linkage = self.cluster_linkage;
                *k_sweep_max = self.cluster_k_sweep_max;
            }
            AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize, .. }) => {
                *standardize = self.pca_standardize;
            }
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank }) => {
                *spearman_rank = self.corr_spearman_rank;
            }
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m, response_column, predictor_columns, .. }) => {
                *imputations_m = self.imputation_m.max(1);
                *response_column = response_col;
                if !predictor_cols.is_empty() {
                    *predictor_columns = predictor_cols;
                }
            }
            AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation }) => {
                *x_transform = self.transform_x;
                *y_transform = self.transform_y;
                *error_propagation = self.transform_error_propagation;
            }
            AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation }) => {
                *baseline_columns = self.baseline_columns.clone();
                *operation = self.baseline_operation;
            }
            AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, .. }) => {
                *filter_formula = self.transform_formula.clone();
                *target_archetype = self.transform_target_archetype;
            }
            AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation }) => {
                *seed = self.sample_seed;
                *sample_size = self.sample_size.max(1);
                *stratify_column = self.sample_stratify_column;
                *proportional_allocation = self.sample_proportional;
            }
            AnalysisKind::PValueStack(ps) => {
                ps.method = self.pvalue_stack_method;
                ps.threshold = self.pvalue_stack_threshold;
                ps.column = self.pvalue_stack_column;
            }
            _ => {}
        }
        Some(kind)
    }

    fn populate_nonlinear_kind(&self, fit: &mut NonlinearFitSpec) {
        if !self.equation_id.trim().is_empty() {
            fit.equation_id = self.equation_id.trim().to_owned();
        }
        if self.custom_dsl.is_some() {
            fit.custom_dsl.clone_from(&self.custom_dsl);
        }
        fit.method = self.fit_method;
        fit.weighting = self.fit_weighting;
        if !self.parameter_rules.is_empty() {
            fit.parameter_rules.clone_from(&self.parameter_rules);
        }
        if !self.mv_x_columns.is_empty() {
            fit.mv_x_columns.clone_from(&self.mv_x_columns);
        }
        if self.mv_y_column.is_some() {
            fit.mv_y_column = self.mv_y_column;
        }
        if !self.mv_group_columns.is_empty() {
            fit.mv_group_columns.clone_from(&self.mv_group_columns);
        }
        if self.source_table_kind == Some(TableKind::MultipleVariables) && fit.mv_x_columns.is_empty() && fit.mv_y_column.is_none() {
            let numeric_cols: Vec<usize> = self
                .variable_candidates
                .iter()
                .filter(|v| !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
                .map(|v| v.col_index)
                .collect();
            if numeric_cols.len() >= 2 {
                fit.mv_x_columns = vec![numeric_cols[0]];
                fit.mv_y_column = numeric_cols.last().copied();
            }
        }
    }

    fn populate_glm_kind(&self, glm: &mut curvebook_doc::GlmSpec, response_col: Option<usize>, predictor_cols: &[usize]) {
        glm.family = self.glm_family;
        glm.compute_vif = self.glm_compute_vif;
        if !self.glm_reference_levels.is_empty() {
            glm.reference_levels.clone_from(&self.glm_reference_levels);
        }
        let first_numeric = self
            .variable_candidates
            .iter()
            .find(|v| v.selected && !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
            .map(|v| v.col_index);
        if let Some(resp) = self.explicit_response_column.or(first_numeric).or(response_col) {
            glm.response_column = resp;
            let preds: Vec<usize> = self
                .variable_candidates
                .iter()
                .filter(|v| v.selected && v.col_index != resp && !matches!(v.role, VariableRole::Label))
                .map(|v| v.col_index)
                .collect();
            if !preds.is_empty() {
                glm.predictor_columns = preds;
            }
        } else if !predictor_cols.is_empty() {
            glm.predictor_columns = predictor_cols.to_vec();
        }
    }

    fn populate_survival_kind(&self, surv: &mut SurvivalAnalysisSpec, response_col: Option<usize>, predictor_cols: &[usize]) {
        match surv {
            SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, tie_method } => {
                let numeric_cols: Vec<usize> = self
                    .variable_candidates
                    .iter()
                    .filter(|v| v.selected && !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
                    .map(|v| v.col_index)
                    .collect();
                if let Some(tc) = self.cox_time_column.or_else(|| numeric_cols.first().copied()).or(response_col) {
                    *time_column = tc;
                }
                if let Some(sc) = self
                    .cox_status_column
                    .or_else(|| numeric_cols.iter().copied().find(|&c| c != *time_column))
                    .or_else(|| predictor_cols.first().copied())
                {
                    *status_column = sc;
                }
                if !self.cox_predictor_columns.is_empty() {
                    predictor_columns.clone_from(&self.cox_predictor_columns);
                } else {
                    let covs: Vec<usize> = self
                        .variable_candidates
                        .iter()
                        .filter(|v| {
                            v.selected && !matches!(v.role, VariableRole::Label) && v.col_index != *time_column && v.col_index != *status_column
                        })
                        .map(|v| v.col_index)
                        .collect();
                    if !covs.is_empty() {
                        *predictor_columns = covs;
                    }
                }
                if !self.cox_reference_levels.is_empty() {
                    reference_levels.clone_from(&self.cox_reference_levels);
                }
                *tie_method = self.cox_tie_method;
            }
            SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox,
                gehan_breslow_wilcoxon,
                trend_test,
                multiplicity,
                mv_time_column,
                mv_status_column,
                mv_group_column,
                ..
            } => {
                *log_rank_mantel_cox = self.km_log_rank_mantel_cox;
                *gehan_breslow_wilcoxon = self.km_gehan_breslow_wilcoxon;
                *trend_test = self.km_trend_test;
                multiplicity.method = self.km_multiplicity_method;
                *mv_time_column = self.km_mv_time_column;
                *mv_status_column = self.km_mv_status_column;
                if self.mv_group_column.is_some() {
                    *mv_group_column = self.mv_group_column;
                }
            }
        }
    }

    /// Find existing result sheets on `doc` that already analyze the same source table using the
    /// same high-level analysis family (for the duplicate-analysis confirmation prompt).
    #[must_use]
    pub fn matching_existing_analyses(&self, doc: &ProjectDocument) -> Vec<SheetId> {
        let Some(src) = &self.source_sheet_id else {
            return Vec::new();
        };
        let Some(pending) = self.build_analysis_kind() else {
            return Vec::new();
        };
        let pending_tag = std::mem::discriminant(&pending);
        doc.result_sheets
            .iter()
            .filter(|rs| &rs.analysis.source_sheet_id == src && std::mem::discriminant(&rs.analysis.kind) == pending_tag)
            .map(|rs| rs.id.clone())
            .collect()
    }

    /// Either return `Err(existing_ids)` to prompt `Modify existing` vs `Create new`, or build an
    /// `analysis.run` [`ToolCommandIntent`].
    pub fn try_build_run_intent(&self, doc: &ProjectDocument) -> Result<ToolCommandIntent, Vec<SheetId>> {
        let matches = self.matching_existing_analyses(doc);
        if !matches.is_empty() {
            return Err(matches);
        }
        Ok(self.build_create_new_intent().unwrap_or_else(|| ToolCommandIntent::new("analyze.run", json!({}))))
    }

    /// Resolve a duplicate-analysis confirmation prompt into `analysis.updateParams` or `analysis.run`.
    #[must_use]
    pub fn resolve_duplicate_choice(&self, choice: &AnalyzeDuplicateChoice) -> Option<ToolCommandIntent> {
        let kind = self.build_analysis_kind()?;
        match choice {
            AnalyzeDuplicateChoice::ModifyExisting(result_sheet_id) => Some(ToolCommandIntent::new(
                "analysis.updateParams",
                json!({
                    "sheet_id": result_sheet_id.as_str(),
                    "kind": kind,
                    "selected_data_sets": self.selected_data_sets,
                }),
            )),
            AnalyzeDuplicateChoice::CreateNew => self.build_create_new_intent(),
        }
    }

    /// Build an unconditional `analysis.run` intent for creating a new result sheet.
    #[must_use]
    pub fn build_create_new_intent(&self) -> Option<ToolCommandIntent> {
        let src = self.source_sheet_id.as_ref()?;
        let kind = self.build_analysis_kind()?;
        Some(ToolCommandIntent::new(
            "analysis.run",
            json!({
                "source_sheet_id": src.as_str(),
                "selected_data_sets": self.selected_data_sets,
                "kind": kind,
            }),
        ))
    }
}

pub(super) fn collect_variable_candidates(sheet: &DataSheet) -> Vec<VariableCandidate> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let is_numeric_var = |v: &curvebook_doc::VariableColumn| !matches!(v.role, VariableRole::Label | VariableRole::Categorical);
            let default_resp_col = mv
                .variables
                .iter()
                .enumerate()
                .rev()
                .find(|(_, v)| is_numeric_var(v) && v.cells.iter().any(|c| c.active_number().is_some()))
                .map(|(i, _)| i)
                .or_else(|| mv.variables.iter().enumerate().rev().find(|(_, v)| is_numeric_var(v)).map(|(i, _)| i))
                .unwrap_or(0);
            mv.variables
                .iter()
                .enumerate()
                .map(|(col_index, var)| VariableCandidate {
                    col_index,
                    title: var.title.clone(),
                    role: var.role,
                    role_inferred: var.role_inferred,
                    selected: true,
                    is_response: col_index == default_resp_col,
                })
                .collect()
        }
        _ => (0..sheet.archetype.column_count())
            .map(|col_index| VariableCandidate {
                col_index,
                title: format!("Col {}", curvebook_doc::default_column_label(col_index)),
                role: VariableRole::Continuous,
                role_inferred: true,
                selected: true,
                is_response: false,
            })
            .collect(),
    }
}

fn collect_result_tab_candidates(tab: &ResultTab) -> Vec<VariableCandidate> {
    tab.column_headers
        .iter()
        .enumerate()
        .map(|(col_index, header)| {
            let has_text = tab.rows.iter().filter_map(|r| r.get(col_index)).any(|c| matches!(c, ResultCell::Text(_) | ResultCell::Boolean(_)));
            VariableCandidate {
                col_index,
                title: header.clone(),
                role: if has_text { VariableRole::Categorical } else { VariableRole::Continuous },
                role_inferred: true,
                selected: true,
                is_response: col_index == 0,
            }
        })
        .collect()
}
