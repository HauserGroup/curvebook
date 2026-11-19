//! Accepted spellings for command parameters that handlers read directly from JSON.
//!
//! Each table lists every accepted spelling of one parameter in precedence order: the first
//! spelling present in the request wins, even when its value is `null`, and later spellings are
//! then ignored. [`first_present`] implements exactly that rule. Parameters declared in a
//! command's input schema use the schema's `x-aliases` instead (see [`super::params`]).
//!
//! These tables are the single place to add or retire a spelling. [`ALL`] lists every table by
//! name for tests and tooling.

use serde_json::{Map, Value};

/// A JSON object that parameters can be looked up in.
pub trait ParamSource {
    /// Look up one parameter by its exact key.
    fn param(&self, key: &str) -> Option<&Value>;
}

impl ParamSource for Value {
    fn param(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl ParamSource for Map<String, Value> {
    fn param(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

/// Return the value of the first spelling present in `source`, even when that value is `null`.
#[must_use]
pub fn first_present<'a, S: ParamSource + ?Sized>(source: &'a S, spellings: &[&str]) -> Option<&'a Value> {
    spellings.iter().find_map(|key| source.param(key))
}

/// Math-transform preset applied to Y.
pub const TRANSFORM_Y: &[&str] = &["y_transform", "preset", "operation"];

/// Error-propagation mode for math transforms.
pub const TRANSFORM_ERROR_PROPAGATION: &[&str] = &["error_propagation", "propagate_error"];

/// Normalization anchor that maps to 0%.
pub const TRANSFORM_ZERO_ANCHOR: &[&str] = &["zero_anchor", "zero"];

/// Normalization anchor that maps to 100%.
pub const TRANSFORM_HUNDRED_ANCHOR: &[&str] = &["hundred_anchor", "hundred"];

/// Row count drawn by the random-sample transform.
pub const TRANSFORM_SAMPLE_SIZE: &[&str] = &["sample_size", "n_samples"];

/// Keep-every-k-rows stride for row pruning.
pub const TRANSFORM_EVERY_K_ROWS: &[&str] = &["every_k_rows", "step"];

/// Whether pruning averages the dropped rows into each kept row.
pub const TRANSFORM_AVERAGE_WINDOW: &[&str] = &["average_window", "average_dropped"];

/// Target row count for row pruning.
pub const TRANSFORM_ROW_COUNT: &[&str] = &["row_count", "n_points"];

/// Derived-column formulas for select-and-transform.
pub const TRANSFORM_DERIVED_COLUMNS: &[&str] = &["derived_columns", "derived", "column_formulas", "formulas"];

/// Title of one derived column.
pub const DERIVED_COLUMN_TITLE: &[&str] = &["title", "name"];

/// Formula of one derived column.
pub const DERIVED_COLUMN_FORMULA: &[&str] = &["formula", "expr"];

/// Contingency-table test method.
pub const CONTINGENCY_METHOD: &[&str] = &["method", "contingency_method"];

/// Yates continuity correction toggle.
pub const CONTINGENCY_YATES: &[&str] = &["yates_continuity", "yates"];

/// Number of k-means clusters.
pub const MULTIVARIATE_K_CLUSTERS: &[&str] = &["k_clusters", "k_means_k"];

/// Multiplicity method for a stacked P-value table.
pub const PVALUE_STACK_METHOD: &[&str] = &["method", "pvalue_stack_method", "multiplicity_method", "multiplicity"];

/// Significance threshold (alpha or Q) for a stacked P-value table.
pub const PVALUE_STACK_THRESHOLD: &[&str] = &["threshold", "pvalue_stack_threshold", "alpha", "q", "fdr_q"];

/// Column holding the stacked P values.
pub const PVALUE_STACK_COLUMN: &[&str] = &["column", "pvalue_stack_column"];

/// Multiple-comparison specification for an analysis.
pub const MULTIPLICITY: &[&str] = &["multiplicity", "post_hoc", "multiplicity_method"];

/// Family-wise alpha for multiple comparisons.
pub const MULTIPLICITY_ALPHA: &[&str] = &["alpha", "threshold"];

/// Cox regression covariate columns.
pub const COX_PREDICTOR_COLUMNS: &[&str] = &["predictor_columns", "cox_covariate_columns"];

/// Log-rank (Mantel–Cox) test toggle.
pub const SURVIVAL_LOG_RANK: &[&str] = &["log_rank_mantel_cox", "log_rank"];

/// Gehan–Breslow–Wilcoxon test toggle.
pub const SURVIVAL_GEHAN: &[&str] = &["gehan_breslow_wilcoxon", "gehan"];

/// Time column of a multiple-variables table.
pub const MV_TIME_COLUMN: &[&str] = &["mv_time_column", "time_column"];

/// Event-status column of a multiple-variables table.
pub const MV_STATUS_COLUMN: &[&str] = &["mv_status_column", "status_column"];

/// Grouping column of a multiple-variables table.
pub const MV_GROUP_COLUMN: &[&str] = &["mv_group_column", "group_column"];

/// Response column of a multiple-variables table (two-group comparisons).
pub const MV_RESPONSE_COLUMN: &[&str] = &["mv_response_column", "response_column"];

/// Generalized linear model family.
pub const GLM_FAMILY: &[&str] = &["family", "glm_family"];

/// Column index inside one reference-level entry.
pub const REFERENCE_LEVEL_COLUMN: &[&str] = &["column", "col", "column_index"];

/// Level name inside one reference-level entry.
pub const REFERENCE_LEVEL_LEVEL: &[&str] = &["level", "reference_level"];

/// Predictor columns for multivariate analyses.
pub const MULTIVARIATE_PREDICTOR_COLUMNS: &[&str] = &["predictor_columns", "predictors"];

/// Spearman rank correlation toggle.
pub const MULTIVARIATE_SPEARMAN: &[&str] = &["spearman", "spearman_rank"];

/// Number of multiple imputations.
pub const MULTIVARIATE_IMPUTATIONS: &[&str] = &["imputations_m", "m", "num_imputations"];

/// Iteration cap for iterative multivariate methods.
pub const MULTIVARIATE_MAX_ITERS: &[&str] = &["max_iters", "iterations"];

/// Response column for multivariate regression methods.
pub const MULTIVARIATE_RESPONSE_COLUMN: &[&str] = &["response_column", "target_column"];

/// Weighting scheme for Cohen's kappa.
pub const KAPPA_WEIGHTING: &[&str] = &["weighting", "kappa_weighting"];

/// Distance metric for clustering.
pub const CLUSTER_DISTANCE: &[&str] = &["distance", "distance_metric"];

/// Linkage method for hierarchical clustering.
pub const CLUSTER_LINKAGE: &[&str] = &["linkage", "linkage_method"];

/// Hypothetical mean for one-sample tests.
pub const ONE_SAMPLE_HYPOTHETICAL: &[&str] = &["hypothetical", "hypothetical_mean"];

/// Deming (model II) regression toggle.
pub const LINEAR_DEMING: &[&str] = &["deming", "deming_model_ii"];

/// ROUT false-discovery Q, in percent.
pub const OUTLIER_Q_PERCENT: &[&str] = &["q_percent", "fdr_q_percent"];

/// Sample size (group 1) for power analysis.
pub const POWER_SAMPLE_SIZE: &[&str] = &["sample_size", "n"];

/// Sample size of group 2 for power analysis.
pub const POWER_SAMPLE_SIZE_GROUP2: &[&str] = &["sample_size_group2", "n2"];

/// Chi-square degrees of freedom for power analysis.
pub const POWER_CHI_SQUARE_DF: &[&str] = &["chi_square_df", "df"];

/// Standardized effect size for power analysis.
pub const POWER_EFFECT_SIZE: &[&str] = &["effect_size", "standardized_effect"];

/// Number of ANOVA groups for power analysis.
pub const POWER_NUM_GROUPS: &[&str] = &["num_groups", "anova_groups"];

/// Model-comparison specification for nonlinear fits.
pub const NONLINEAR_COMPARISON: &[&str] = &["comparison", "model_comparison"];

/// Nonlinear fitting method.
pub const NONLINEAR_FIT_METHOD: &[&str] = &["method", "fit_method"];

/// Residual normality tests toggle for nonlinear fits.
pub const NONLINEAR_NORMALITY_TESTS: &[&str] = &["normality_of_residuals", "normality_tests"];

/// Alternative model in a model comparison.
pub const MODEL_COMPARISON_ALTERNATIVE: &[&str] = &["alternative_equation_id", "alternative_model"];

/// Model-comparison method.
pub const MODEL_COMPARISON_METHOD: &[&str] = &["method", "kind"];

/// Alternative model in a model comparison, also accepting a bare `equation_id`.
pub const MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION: &[&str] = &["alternative_equation_id", "alternative_model", "equation_id"];

/// Parameter name inside one parameter rule.
pub const PARAMETER_RULE_NAME: &[&str] = &["name", "parameter"];

/// Per-dataset constant values of a parameter scope.
pub const PARAMETER_SCOPE_VALUES: &[&str] = &["values", "constants", "per_dataset_constants"];

/// Hook source of a parameter scope.
pub const PARAMETER_SCOPE_HOOK: &[&str] = &["hook", "source"];

/// Replicate subcolumn of a point reference.
pub const POINT_SUBCOLUMN: &[&str] = &["subcol", "subcolumn", "replicate"];

/// Graphical data object patch for one series.
pub const SERIES_GRAPHICAL_OBJECT: &[&str] = &["gdo", "graphical_object"];

/// Series colour. Precedence is the reverse of `MARKER_FILL_COLOR`.
pub const SERIES_COLOR: &[&str] = &["color", "fill_color"];

/// Marker patch for one series.
pub const SERIES_MARKER: &[&str] = &["marker", "markers"];

/// Connecting-line patch for one series.
pub const SERIES_CONNECTING_LINE: &[&str] = &["connecting_line", "line"];

/// Y error-bar patch for one series.
pub const SERIES_Y_ERROR_BARS: &[&str] = &["y_error_bars", "error_bars"];

/// Dash style of a line patch. Precedence is the reverse of `ANNOTATION_DASH`.
pub const LINE_DASH_STYLE: &[&str] = &["dash_style", "dash"];

/// Stroke width of a line patch, in points.
pub const LINE_STROKE_WIDTH: &[&str] = &["stroke_width_pt", "width_pt"];

/// Marker fill colour. Precedence is the reverse of `SERIES_COLOR`.
pub const MARKER_FILL_COLOR: &[&str] = &["fill_color", "color"];

/// Whether a text annotation draws a border.
pub const ANNOTATION_BORDERED: &[&str] = &["bordered", "boxed"];

/// Dash style of an annotation. Precedence is the reverse of `LINE_DASH_STYLE`.
pub const ANNOTATION_DASH: &[&str] = &["dash", "dash_style"];

/// Annotation bounds rectangle, in points.
pub const ANNOTATION_BOUNDS: &[&str] = &["bounds_pt", "rect_pt"];

/// Annotation stroke colour.
pub const ANNOTATION_STROKE_COLOR: &[&str] = &["stroke_color", "color"];

/// Name of one simulated column.
pub const SIMULATED_COLUMN_NAME: &[&str] = &["name", "title"];

/// Distribution kind of one simulated column.
pub const SIMULATED_COLUMN_KIND: &[&str] = &["kind", "type", "distribution"];

/// Preview-only flag of `table.convertArchetype`. Unlike the other tables, conflicting values across spellings are rejected rather than resolved by precedence.
pub const CONVERT_PREVIEW: &[&str] = &["preview_only", "previewOnly", "preview", "dry_run", "dryRun"];

/// Every spelling table, by name.
pub const ALL: &[(&str, &[&str])] = &[
    ("TRANSFORM_Y", TRANSFORM_Y),
    ("TRANSFORM_ERROR_PROPAGATION", TRANSFORM_ERROR_PROPAGATION),
    ("TRANSFORM_ZERO_ANCHOR", TRANSFORM_ZERO_ANCHOR),
    ("TRANSFORM_HUNDRED_ANCHOR", TRANSFORM_HUNDRED_ANCHOR),
    ("TRANSFORM_SAMPLE_SIZE", TRANSFORM_SAMPLE_SIZE),
    ("TRANSFORM_EVERY_K_ROWS", TRANSFORM_EVERY_K_ROWS),
    ("TRANSFORM_AVERAGE_WINDOW", TRANSFORM_AVERAGE_WINDOW),
    ("TRANSFORM_ROW_COUNT", TRANSFORM_ROW_COUNT),
    ("TRANSFORM_DERIVED_COLUMNS", TRANSFORM_DERIVED_COLUMNS),
    ("DERIVED_COLUMN_TITLE", DERIVED_COLUMN_TITLE),
    ("DERIVED_COLUMN_FORMULA", DERIVED_COLUMN_FORMULA),
    ("CONTINGENCY_METHOD", CONTINGENCY_METHOD),
    ("CONTINGENCY_YATES", CONTINGENCY_YATES),
    ("MULTIVARIATE_K_CLUSTERS", MULTIVARIATE_K_CLUSTERS),
    ("PVALUE_STACK_METHOD", PVALUE_STACK_METHOD),
    ("PVALUE_STACK_THRESHOLD", PVALUE_STACK_THRESHOLD),
    ("PVALUE_STACK_COLUMN", PVALUE_STACK_COLUMN),
    ("MULTIPLICITY", MULTIPLICITY),
    ("MULTIPLICITY_ALPHA", MULTIPLICITY_ALPHA),
    ("COX_PREDICTOR_COLUMNS", COX_PREDICTOR_COLUMNS),
    ("SURVIVAL_LOG_RANK", SURVIVAL_LOG_RANK),
    ("SURVIVAL_GEHAN", SURVIVAL_GEHAN),
    ("MV_TIME_COLUMN", MV_TIME_COLUMN),
    ("MV_STATUS_COLUMN", MV_STATUS_COLUMN),
    ("MV_GROUP_COLUMN", MV_GROUP_COLUMN),
    ("MV_RESPONSE_COLUMN", MV_RESPONSE_COLUMN),
    ("GLM_FAMILY", GLM_FAMILY),
    ("REFERENCE_LEVEL_COLUMN", REFERENCE_LEVEL_COLUMN),
    ("REFERENCE_LEVEL_LEVEL", REFERENCE_LEVEL_LEVEL),
    ("MULTIVARIATE_PREDICTOR_COLUMNS", MULTIVARIATE_PREDICTOR_COLUMNS),
    ("MULTIVARIATE_SPEARMAN", MULTIVARIATE_SPEARMAN),
    ("MULTIVARIATE_IMPUTATIONS", MULTIVARIATE_IMPUTATIONS),
    ("MULTIVARIATE_MAX_ITERS", MULTIVARIATE_MAX_ITERS),
    ("MULTIVARIATE_RESPONSE_COLUMN", MULTIVARIATE_RESPONSE_COLUMN),
    ("KAPPA_WEIGHTING", KAPPA_WEIGHTING),
    ("CLUSTER_DISTANCE", CLUSTER_DISTANCE),
    ("CLUSTER_LINKAGE", CLUSTER_LINKAGE),
    ("ONE_SAMPLE_HYPOTHETICAL", ONE_SAMPLE_HYPOTHETICAL),
    ("LINEAR_DEMING", LINEAR_DEMING),
    ("OUTLIER_Q_PERCENT", OUTLIER_Q_PERCENT),
    ("POWER_SAMPLE_SIZE", POWER_SAMPLE_SIZE),
    ("POWER_SAMPLE_SIZE_GROUP2", POWER_SAMPLE_SIZE_GROUP2),
    ("POWER_CHI_SQUARE_DF", POWER_CHI_SQUARE_DF),
    ("POWER_EFFECT_SIZE", POWER_EFFECT_SIZE),
    ("POWER_NUM_GROUPS", POWER_NUM_GROUPS),
    ("NONLINEAR_COMPARISON", NONLINEAR_COMPARISON),
    ("NONLINEAR_FIT_METHOD", NONLINEAR_FIT_METHOD),
    ("NONLINEAR_NORMALITY_TESTS", NONLINEAR_NORMALITY_TESTS),
    ("MODEL_COMPARISON_ALTERNATIVE", MODEL_COMPARISON_ALTERNATIVE),
    ("MODEL_COMPARISON_METHOD", MODEL_COMPARISON_METHOD),
    ("MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION", MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION),
    ("PARAMETER_RULE_NAME", PARAMETER_RULE_NAME),
    ("PARAMETER_SCOPE_VALUES", PARAMETER_SCOPE_VALUES),
    ("PARAMETER_SCOPE_HOOK", PARAMETER_SCOPE_HOOK),
    ("POINT_SUBCOLUMN", POINT_SUBCOLUMN),
    ("SERIES_GRAPHICAL_OBJECT", SERIES_GRAPHICAL_OBJECT),
    ("SERIES_COLOR", SERIES_COLOR),
    ("SERIES_MARKER", SERIES_MARKER),
    ("SERIES_CONNECTING_LINE", SERIES_CONNECTING_LINE),
    ("SERIES_Y_ERROR_BARS", SERIES_Y_ERROR_BARS),
    ("LINE_DASH_STYLE", LINE_DASH_STYLE),
    ("LINE_STROKE_WIDTH", LINE_STROKE_WIDTH),
    ("MARKER_FILL_COLOR", MARKER_FILL_COLOR),
    ("ANNOTATION_BORDERED", ANNOTATION_BORDERED),
    ("ANNOTATION_DASH", ANNOTATION_DASH),
    ("ANNOTATION_BOUNDS", ANNOTATION_BOUNDS),
    ("ANNOTATION_STROKE_COLOR", ANNOTATION_STROKE_COLOR),
    ("SIMULATED_COLUMN_NAME", SIMULATED_COLUMN_NAME),
    ("SIMULATED_COLUMN_KIND", SIMULATED_COLUMN_KIND),
    ("CONVERT_PREVIEW", CONVERT_PREVIEW),
];
