//! `Analyze...` dialog state machine, variable role filters, duplicate prompts, and candidate collection.

use curvebook_doc::{
    AnalysisKind, BaselineOperation, ClusterDistance, ClusterLinkage, ContingencyMethod, CorrelationMethod, CoxTieMethod, DataSheet,
    ErrorPropagationMode, FitMethod, GlmFamily, MathTransformPreset, MultiplicityMethod, MultivariateSpec, NonlinearFitSpec, OneWayMethod,
    OutlierSpec, PValueStackMethod, ParameterRule, ProjectDocument, RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultSheet, ResultTab,
    SheetId, SubcolumnAggregation, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod,
    VariableRole, WeightingScheme,
};
use curvebook_palette::Rgba8;

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::theme::UiThemeSpec;

use super::analysis_catalog::{ANALYSIS_CATALOG, AnalysisCatalogItem, AnalysisCategory, default_analysis_kind_for_id};
use super::edit_params::{TwoGroupSubmethodMemory, check_pcr_target_leakage, default_true_flag, toggle_sorted_col};

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
        VariableRole::Continuous => theme.text_primary,
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

/// Radio choice in the re-analysis disambiguation prompt (`DuplicateAnalysisPrompt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DuplicateChoice {
    ModifyExisting,
    #[default]
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
    pub pca_biplot: bool,
    #[serde(default)]
    pub clustering_dendrogram: bool,
    #[serde(default)]
    pub curve_calculus_create_graph: bool,
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
    #[serde(default)]
    pub corr_method: Option<CorrelationMethod>,
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
    pub descriptive_custom_upper_percentile: Option<f64>,
    #[serde(default = "default_true_flag")]
    pub descriptive_run_normality_battery: bool,
    #[serde(default)]
    pub descriptive_visible_metrics: curvebook_doc::DescriptiveMetricVisibility,
    #[serde(default)]
    pub descriptive_significant_digits: Option<u8>,
    #[serde(default)]
    pub descriptive_save_as_default: bool,
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
    #[serde(flatten)]
    pub two_group_memory: TwoGroupSubmethodMemory,
    #[serde(skip)]
    pub explicit_response_column: Option<usize>,
    #[serde(skip)]
    pub force_create_new_once: bool,
    #[serde(default)]
    pub collapsed_categories: Vec<String>,
    #[serde(default)]
    pub duplicate_choice: DuplicateChoice,
    #[serde(default)]
    pub duplicate_target_id: Option<String>,
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
            pca_biplot: false,
            clustering_dendrogram: false,
            curve_calculus_create_graph: false,
            transform_x: None,
            transform_y: default_transform_y(),
            transform_error_propagation: ErrorPropagationMode::ReplicateEndpoints,
            transform_formula: None,
            transform_target_archetype: default_transform_target_archetype(),
            baseline_columns: default_baseline_columns(),
            baseline_operation: default_baseline_operation(),
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
            corr_method: None,
            imputation_m: default_imputation_m(),
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
            sample_seed: default_sample_seed(),
            sample_size: default_sample_size(),
            sample_stratify_column: None,
            sample_proportional: true,
            descriptive_aggregation: SubcolumnAggregation::RowMeans,
            descriptive_custom_upper_percentile: None,
            descriptive_run_normality_battery: true,
            descriptive_visible_metrics: curvebook_doc::DescriptiveMetricVisibility::default(),
            descriptive_significant_digits: None,
            descriptive_save_as_default: false,
            pvalue_stack_method: PValueStackMethod::default(),
            pvalue_stack_threshold: default_pvalue_stack_threshold(),
            pvalue_stack_column: None,
            compute_effect_sizes: None,
            outlier_method: default_outlier_method(),
            outlier_alpha: default_outlier_alpha(),
            outlier_fdr_q_percent: default_outlier_fdr_q(),
            outlier_iterative: false,
            two_group_memory: TwoGroupSubmethodMemory::default(),
            explicit_response_column: None,
            force_create_new_once: false,
            collapsed_categories: Vec::new(),
            duplicate_choice: DuplicateChoice::CreateNew,
            duplicate_target_id: None,
        }
    }
}

impl AnalyzeDialogState {
    /// Resolve the active `duplicate_choice` / `duplicate_target_id` into an engine [`ToolCommandIntent`].
    #[must_use]
    pub fn resolve_selected_duplicate_intent(&self, existing_result_ids: &[SheetId]) -> Option<ToolCommandIntent> {
        let choice = match self.duplicate_choice {
            DuplicateChoice::ModifyExisting => {
                let target = self
                    .duplicate_target_id
                    .as_deref()
                    .and_then(|tid| existing_result_ids.iter().find(|id| id.as_str() == tid))
                    .or_else(|| existing_result_ids.first())?;
                AnalyzeDuplicateChoice::ModifyExisting(target.clone())
            }
            DuplicateChoice::CreateNew => AnalyzeDuplicateChoice::CreateNew,
        };
        self.resolve_duplicate_choice(&choice)
    }

    fn reset_for_open(&mut self) {
        let equation_id =
            if self.equation_id.is_empty() { String::from("log_agonist_vs_response_4pl") } else { std::mem::take(&mut self.equation_id) };
        *self = Self { equation_id, ..Self::default() };
    }

    /// Seed Descriptive Statistics fields from a saved user default [`curvebook_doc::DescriptiveSpec`].
    pub fn apply_saved_descriptive_default(&mut self, default_spec: &curvebook_doc::DescriptiveSpec) {
        self.descriptive_aggregation = default_spec.aggregation;
        self.descriptive_custom_upper_percentile = default_spec.custom_upper_percentile;
        self.descriptive_run_normality_battery = default_spec.run_normality_battery;
        self.descriptive_visible_metrics = default_spec.visible_metrics;
        self.descriptive_significant_digits = default_spec.significant_digits;
        self.descriptive_save_as_default = false;
    }

    /// Populate defaults and variable candidates for `sheet`.
    pub fn open_for_data_sheet(&mut self, sheet: &DataSheet) {
        self.reset_for_open();
        self.source_sheet_id = Some(sheet.id.clone());
        self.source_table_kind = Some(sheet.kind());
        self.variable_candidates = collect_variable_candidates(sheet);
        match &sheet.archetype {
            TableArchetype::Xy(xy) => {
                let xs: Vec<f64> = (0..xy.row_count()).filter_map(|r| xy.x_summary_at(r).ok().flatten().filter(|x| x.is_finite())).collect();
                self.equation_id = String::from(curvebook_fit::suggested_default_dose_response_equation(&xs));
            }
            TableArchetype::MultipleVariables(mv) => {
                let xs: Vec<f64> = mv
                    .variables
                    .iter()
                    .find(|v| !matches!(v.role, VariableRole::Label | VariableRole::Categorical))
                    .map(|v| v.cells.iter().filter_map(|c| c.active_number()).collect())
                    .unwrap_or_default();
                self.equation_id = String::from(curvebook_fit::suggested_default_dose_response_equation(&xs));
            }
            _ => {
                self.equation_id = String::from("log_agonist_vs_response_4pl");
            }
        }
        self.selected_entry_id = self.compatible_entries().first().map(|item| item.id.to_owned());
    }

    /// Populate defaults and column candidates from a non-analyzable [`ResultTab`] for chained GUI `Analyze` (`docs/gui-workflows.md` §3.3).
    pub fn open_for_result_tab(&mut self, result: &ResultSheet, tab: &ResultTab) {
        self.reset_for_open();
        self.source_sheet_id = Some(result.id.clone());
        self.extracted_result_tab_id = Some(tab.id.clone());
        let has_text = tab.rows.iter().flatten().any(|cell| matches!(cell, ResultCell::Text(_) | ResultCell::Boolean(_)));
        self.source_table_kind = Some(if has_text { TableKind::MultipleVariables } else { TableKind::Column });
        if let AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column, .. }) = &result.analysis.kind {
            self.pca_predictors.clone_from(&result.analysis.selected_data_sets);
            self.pcr_outcome_column = *pcr_outcome_column;
        }
        self.variable_candidates = collect_result_tab_candidates(tab);
        self.selected_entry_id = self.compatible_entries().first().map(|item| item.id.to_owned());
    }

    /// Switch the selected catalog entry (`analyze.entry.<id>`), resetting PCR selection cleanly when entering `stats.pcr`.
    pub fn select_catalog_entry(&mut self, entry_id: &str) {
        let entering_pcr = entry_id == "stats.pcr" && self.selected_entry_id.as_deref() != Some("stats.pcr");
        let leaving_pcr = self.selected_entry_id.as_deref() == Some("stats.pcr") && entry_id != "stats.pcr";
        self.selected_entry_id = Some(entry_id.to_owned());
        if entering_pcr {
            self.selected_data_sets.clear();
            self.pca_predictors.clear();
            self.pcr_outcome_column = None;
            for cand in &mut self.variable_candidates {
                cand.selected = false;
            }
        } else if leaving_pcr && self.selected_data_sets.is_empty() {
            for cand in &mut self.variable_candidates {
                cand.selected = true;
            }
        }
    }

    pub(super) fn is_pca_or_pcr_entry(&self) -> bool {
        matches!(self.selected_entry_id.as_deref(), Some("stats.pca" | "stats.pcr" | "pca" | "pcr"))
    }

    pub(super) fn clear_overlapping_pcr_outcome(&mut self) {
        if let Some(y) = self.pcr_outcome_column
            && (self.pca_predictors.contains(&y) || self.selected_data_sets.contains(&y))
        {
            self.pcr_outcome_column = None;
            if self.is_pca_or_pcr_entry() {
                for cand in &mut self.variable_candidates {
                    if cand.col_index == y {
                        cand.is_response = false;
                    }
                }
            }
        }
    }

    pub(super) fn is_pca_predictor(&self, col: usize) -> bool {
        self.pca_predictors.contains(&col) || self.selected_data_sets.contains(&col)
    }

    /// Toggle a MV predictor column for PCA/PCR while preserving disjoint PCR outcome selection.
    pub fn toggle_pca_predictor(&mut self, col: usize) {
        toggle_sorted_col(&mut self.pca_predictors, col);
        toggle_sorted_col(&mut self.selected_data_sets, col);
        self.clear_overlapping_pcr_outcome();
        for cand in &mut self.variable_candidates {
            cand.selected = self.selected_data_sets.contains(&cand.col_index);
        }
    }

    /// Replace the selected PCA/PCR predictor columns while preserving disjoint PCR outcome selection.
    pub fn set_pca_predictors(&mut self, cols: Vec<usize>) {
        self.selected_data_sets.clone_from(&cols);
        self.pca_predictors = cols;
        self.clear_overlapping_pcr_outcome();
        for cand in &mut self.variable_candidates {
            cand.selected = self.selected_data_sets.contains(&cand.col_index);
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

    /// Select (or toggle off) the disjoint PCR outcome column with R10 predictor leakage guard.
    pub fn select_pcr_outcome(&mut self, col: usize) -> Result<(), String> {
        check_pcr_target_leakage(col, self.is_pca_predictor(col))?;
        self.pcr_outcome_column = if self.pcr_outcome_column == Some(col) { None } else { Some(col) };
        if let Some(outcome) = self.pcr_outcome_column {
            self.set_response_column(outcome);
        } else {
            for cand in &mut self.variable_candidates {
                if cand.col_index == col {
                    cand.is_response = false;
                }
            }
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
    ///
    /// For non-PCA/PCR analyses, enforces a minimum of one selected variable so unchecking the
    /// last remaining candidate cannot collapse `selected_data_sets` to `[]` (which the engine
    /// interprets as "All data sets").
    pub fn toggle_variable(&mut self, col_index: usize) {
        let is_pca_or_pcr = self.is_pca_or_pcr_entry();
        let selected_count = self.variable_candidates.iter().filter(|v| v.selected).count();
        if let Some(var) = self.variable_candidates.iter_mut().find(|v| v.col_index == col_index)
            && !(var.selected && !is_pca_or_pcr && selected_count <= 1)
        {
            var.selected = !var.selected;
        }
        self.selected_data_sets = self.variable_candidates.iter().filter(|v| v.selected).map(|v| v.col_index).collect();
        if is_pca_or_pcr {
            self.pca_predictors = self.selected_data_sets.clone();
            self.clear_overlapping_pcr_outcome();
        }
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
                desc.custom_upper_percentile = self.descriptive_custom_upper_percentile;
                desc.run_normality_battery = self.descriptive_run_normality_battery;
                desc.visible_metrics = self.descriptive_visible_metrics;
                desc.significant_digits = self.descriptive_significant_digits;
                desc.save_as_default = self.descriptive_save_as_default;
            }
            AnalysisKind::TwoGroupComparison(tg) => {
                if let Some(m) = self.two_group_method {
                    tg.method = m;
                }
                if let TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } = &mut tg.method {
                    *hypothetical = self.two_group_hypothetical;
                } else if let TwoGroupMethod::OneSampleRatioT { hypothetical } = &mut tg.method {
                    *hypothetical =
                        if self.two_group_hypothetical.is_finite() && self.two_group_hypothetical > 0.0 { self.two_group_hypothetical } else { 1.0 };
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
            AnalysisKind::Ancova(anc) => {
                if let Some(ces) = self.compute_effect_sizes {
                    anc.compute_effect_sizes = ces;
                }
                if self.source_table_kind == Some(TableKind::MultipleVariables) {
                    let cat_col = self.variable_candidates.iter().find(|v| v.role == VariableRole::Categorical).map(|v| v.col_index);
                    let num_cols: Vec<usize> = self
                        .variable_candidates
                        .iter()
                        .filter(|v| !matches!(v.role, VariableRole::Categorical | VariableRole::Label))
                        .map(|v| v.col_index)
                        .collect();
                    let chosen_group = cat_col.or(self.mv_group_column).unwrap_or(anc.group_column);
                    anc.group_column = chosen_group;
                    let chosen_resp = response_col
                        .filter(|&c| c != chosen_group)
                        .or_else(|| num_cols.iter().copied().find(|&c| c != chosen_group))
                        .unwrap_or(anc.response_column);
                    anc.response_column = chosen_resp;
                    let covs: Vec<usize> = num_cols.into_iter().filter(|&c| c != chosen_group && c != chosen_resp).collect();
                    if !covs.is_empty() {
                        anc.covariate_columns = covs;
                    }
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
            AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max, .. }) => {
                *k_means_k = self.cluster_k_means_k;
                *distance = self.cluster_distance;
                *linkage = self.cluster_linkage;
                *k_sweep_max = self.cluster_k_sweep_max;
            }
            AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize, pcr_outcome_column, .. }) => {
                *standardize = self.pca_standardize;
                *pcr_outcome_column = self.pcr_outcome_column;
            }
            AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank, method }) => {
                *method = self.corr_method;
                *spearman_rank = self.corr_spearman_rank || self.corr_method == Some(CorrelationMethod::Spearman);
            }
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m, response_column, predictor_columns, .. }) => {
                *imputations_m = self.imputation_m.max(1);
                *response_column = response_col;
                if !predictor_cols.is_empty() {
                    *predictor_columns = predictor_cols;
                }
            }
            AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation, .. }) => {
                *x_transform = self.transform_x;
                *y_transform = self.transform_y;
                *error_propagation = self.transform_error_propagation;
            }
            AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation, .. }) => {
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
        if let Some(resp) = self.explicit_response_column.or(response_col).or(first_numeric) {
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
            SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, tie_method, .. } => {
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
        doc.result_sheets
            .iter()
            .filter(|rs| rs.analysis.source_sheet_id.as_ref() == Some(src) && same_analysis_family(&rs.analysis.kind, &pending))
            .map(|rs| rs.id.clone())
            .collect()
    }

    /// Either return `Err(existing_ids)` to prompt `Modify existing` vs `Create new`, return
    /// `Err(vec![])` when the current selection is incomplete/invalid, or build an `analysis.run`
    /// [`ToolCommandIntent`].
    pub fn try_build_run_intent(&self, doc: &ProjectDocument) -> Result<ToolCommandIntent, Vec<SheetId>> {
        let matches = self.matching_existing_analyses(doc);
        if !matches.is_empty() {
            return Err(matches);
        }
        self.build_create_new_intent().ok_or_else(Vec::new)
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

fn same_analysis_family(a: &AnalysisKind, b: &AnalysisKind) -> bool {
    match (a, b) {
        (AnalysisKind::DescriptiveStats(x), AnalysisKind::DescriptiveStats(y)) => x.normality_only == y.normality_only,
        (AnalysisKind::Transform(x), AnalysisKind::Transform(y)) => std::mem::discriminant(x) == std::mem::discriminant(y),
        (AnalysisKind::Multivariate(x), AnalysisKind::Multivariate(y)) => std::mem::discriminant(x) == std::mem::discriminant(y),
        (AnalysisKind::Survival(x), AnalysisKind::Survival(y)) => std::mem::discriminant(x) == std::mem::discriminant(y),
        (AnalysisKind::OutlierDetection(x), AnalysisKind::OutlierDetection(y)) => std::mem::discriminant(x) == std::mem::discriminant(y),
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

pub(super) fn collect_variable_candidates(sheet: &DataSheet) -> Vec<VariableCandidate> {
    let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
        return Vec::new();
    };
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
