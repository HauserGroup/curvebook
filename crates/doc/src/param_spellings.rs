//! Canonical command parameter spellings and retired-alias resolution rules.
//!
//! This module is the L1 source of truth shared across persisted format migration
//! (`curvebook-format`) and command execution (`curvebook-engine`).
//!
//! Each spelling table stores the single canonical `snake_case` key at index `0` followed by any
//! retired spellings still accepted during the R16.5 one-release deprecation window.

/// Math-transform preset applied to Y.
pub const TRANSFORM_Y: &[&str] = &["y_transform", "preset", "operation"];

/// Error-propagation mode for math transforms.
pub const TRANSFORM_ERROR_PROPAGATION: &[&str] = &["error_propagation", "propagate_error"];

/// Lower anchor (0%) for normalization transforms.
pub const TRANSFORM_ZERO_ANCHOR: &[&str] = &["zero_anchor", "zero"];

/// Upper anchor (100%) for normalization transforms.
pub const TRANSFORM_HUNDRED_ANCHOR: &[&str] = &["hundred_anchor", "hundred"];

/// Sample count for random-row sampling transforms.
pub const TRANSFORM_SAMPLE_SIZE: &[&str] = &["sample_size", "n_samples"];

/// Row decimation stride for `prune_rows` transforms.
pub const TRANSFORM_EVERY_K_ROWS: &[&str] = &["every_k_rows", "step"];

/// Window-averaging toggle for `prune_rows` transforms.
pub const TRANSFORM_AVERAGE_WINDOW: &[&str] = &["average_window", "average_dropped"];

/// Output row count for `simulate` transforms.
pub const TRANSFORM_ROW_COUNT: &[&str] = &["row_count", "n_points"];

/// Derived-column list for `select_and_transform` transforms.
pub const TRANSFORM_DERIVED_COLUMNS: &[&str] = &["derived_columns", "derived", "column_formulas", "formulas"];

/// Output column title on a derived-column spec.
pub const DERIVED_COLUMN_TITLE: &[&str] = &["title", "name"];

/// Formula expression on a derived-column spec.
pub const DERIVED_COLUMN_FORMULA: &[&str] = &["formula", "expr"];

/// Test selection for contingency analysis (`stats.run`).
pub const CONTINGENCY_METHOD: &[&str] = &["method", "contingency_method"];

/// Yates continuity correction flag for contingency chi-square tests.
pub const CONTINGENCY_YATES: &[&str] = &["yates_continuity", "yates"];

/// Cluster count for k-means multivariate clustering.
pub const MULTIVARIATE_K_CLUSTERS: &[&str] = &["k_clusters", "k_means_k"];

/// Multiplicity correction method for P-value stacking.
pub const PVALUE_STACK_METHOD: &[&str] = &["method", "pvalue_stack_method", "multiplicity_method", "multiplicity"];

/// Alpha or FDR threshold Q for P-value stacking.
pub const PVALUE_STACK_THRESHOLD: &[&str] = &["threshold", "pvalue_stack_threshold", "alpha", "q", "fdr_q"];

/// Source P-value column index for P-value stacking.
pub const PVALUE_STACK_COLUMN: &[&str] = &["column", "pvalue_stack_column"];

/// Post-hoc multiplicity method for ANOVA and multiple-comparison tests.
pub const MULTIPLICITY: &[&str] = &["multiplicity", "post_hoc", "multiplicity_method"];

/// Family-wise alpha threshold for multiplicity adjustment.
pub const MULTIPLICITY_ALPHA: &[&str] = &["alpha", "threshold"];

/// Predictor column list for Cox proportional hazards regression.
pub const COX_PREDICTOR_COLUMNS: &[&str] = &["predictor_columns", "cox_covariate_columns"];

/// Enable log-rank (Mantel-Cox) test for survival analysis.
pub const SURVIVAL_LOG_RANK: &[&str] = &["log_rank_mantel_cox", "log_rank"];

/// Enable Gehan-Breslow-Wilcoxon test for survival analysis.
pub const SURVIVAL_GEHAN: &[&str] = &["gehan_breslow_wilcoxon", "gehan"];

/// Survival time column index on `MultipleVariables` tables.
pub const MV_TIME_COLUMN: &[&str] = &["mv_time_column", "time_column"];

/// Event/censoring status column index on `MultipleVariables` tables.
pub const MV_STATUS_COLUMN: &[&str] = &["mv_status_column", "status_column"];

/// Grouping variable column index on `MultipleVariables` tables.
pub const MV_GROUP_COLUMN: &[&str] = &["mv_group_column", "group_column"];

/// Outcome/response column index on `MultipleVariables` tables.
pub const MV_RESPONSE_COLUMN: &[&str] = &["mv_response_column", "response_column"];

/// Generalized linear model link/family.
pub const GLM_FAMILY: &[&str] = &["family", "glm_family"];

/// Categorical column index inside a GLM reference-level entry.
pub const REFERENCE_LEVEL_COLUMN: &[&str] = &["column", "col", "column_index"];

/// Reference level label inside a GLM reference-level entry.
pub const REFERENCE_LEVEL_LEVEL: &[&str] = &["level", "reference_level"];

/// Predictor column index list for multivariate regression/classification models.
pub const MULTIVARIATE_PREDICTOR_COLUMNS: &[&str] = &["predictor_columns", "predictors"];

/// Use Spearman rank correlation instead of Pearson correlation.
pub const MULTIVARIATE_SPEARMAN: &[&str] = &["spearman", "spearman_rank"];

/// Number of multiple-imputation datasets `m`.
pub const MULTIVARIATE_IMPUTATIONS: &[&str] = &["imputations_m", "m", "num_imputations"];

/// Maximum optimizer iteration count for multivariate iterative algorithms.
pub const MULTIVARIATE_MAX_ITERS: &[&str] = &["max_iters", "iterations"];

/// Dependent/response variable column index for multivariate regression.
pub const MULTIVARIATE_RESPONSE_COLUMN: &[&str] = &["response_column", "target_column"];

/// Weighting scheme (`unweighted`, `linear`, `quadratic`) for Cohen's kappa.
pub const KAPPA_WEIGHTING: &[&str] = &["weighting", "kappa_weighting"];

/// Dissimilarity/distance metric for hierarchical clustering.
pub const CLUSTER_DISTANCE: &[&str] = &["distance", "distance_metric"];

/// Agglomerative linkage criterion for hierarchical clustering.
pub const CLUSTER_LINKAGE: &[&str] = &["linkage", "linkage_method"];

/// Hypothetical mean/median value for one-sample tests.
pub const ONE_SAMPLE_HYPOTHETICAL: &[&str] = &["hypothetical", "hypothetical_mean"];

/// Enable Deming (Model II) linear regression.
pub const LINEAR_DEMING: &[&str] = &["deming", "deming_model_ii"];

/// False discovery rate Q% threshold for ROUT outlier identification.
pub const OUTLIER_Q_PERCENT: &[&str] = &["q_percent", "fdr_q_percent"];

/// Sample size parameter for power/sample-size calculation.
pub const POWER_SAMPLE_SIZE: &[&str] = &["sample_size", "n"];

/// Group-2 sample size for two-group unbalanced power calculation.
pub const POWER_SAMPLE_SIZE_GROUP2: &[&str] = &["sample_size_group2", "n2"];

/// Degrees of freedom for chi-square power calculation.
pub const POWER_CHI_SQUARE_DF: &[&str] = &["chi_square_df", "df"];

/// Standardized effect size for power calculation.
pub const POWER_EFFECT_SIZE: &[&str] = &["effect_size", "standardized_effect"];

/// Number of ANOVA groups for power calculation.
pub const POWER_NUM_GROUPS: &[&str] = &["num_groups", "anova_groups"];

/// Nested model-comparison configuration object for nonlinear regression.
pub const NONLINEAR_COMPARISON: &[&str] = &["comparison", "model_comparison"];

/// Parameter estimation method (`least_squares`, `robust`, `poisson`) for nonlinear regression.
pub const NONLINEAR_FIT_METHOD: &[&str] = &["method", "fit_method"];

/// Normality-of-residuals test toggle for nonlinear regression.
pub const NONLINEAR_NORMALITY_TESTS: &[&str] = &["normality_of_residuals", "normality_tests"];

/// Alternative equation ID within model comparison.
pub const MODEL_COMPARISON_ALTERNATIVE: &[&str] = &["alternative_equation_id", "alternative_model"];

/// Comparison method (`extra_sum_of_squares_f`, `aicc`) within a comparison object.
pub const MODEL_COMPARISON_METHOD: &[&str] = &["method", "kind"];

/// Alternative equation ID lookup within a comparison object (also accepting fallback `equation_id`).
pub const MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION: &[&str] = &["alternative_equation_id", "alternative_model", "equation_id"];

/// Parameter identifier within a `parameter_rules` entry.
pub const PARAMETER_RULE_NAME: &[&str] = &["name", "parameter"];

/// Fixed parameter value array for fixed/per-dataset scope rules.
pub const PARAMETER_SCOPE_VALUES: &[&str] = &["values", "constants", "per_dataset_constants"];

/// Info-sheet hook specification for `hooked_constant` parameter scope rules.
pub const PARAMETER_SCOPE_HOOK: &[&str] = &["hook", "source"];

/// Replicate subcolumn index of a point reference or override.
pub const POINT_SUBCOLUMN: &[&str] = &["subcol", "subcolumn", "replicate"];

/// Full `GraphicalDataObject` payload inside a series patch.
pub const SERIES_GRAPHICAL_OBJECT: &[&str] = &["gdo", "graphical_object"];

/// Accent/stroke color on a series patch.
pub const SERIES_COLOR: &[&str] = &["color", "fill_color"];

/// Marker styling patch on a series patch.
pub const SERIES_MARKER: &[&str] = &["marker", "markers"];

/// Connecting line styling patch on a series patch.
pub const SERIES_CONNECTING_LINE: &[&str] = &["connecting_line", "line"];

/// Vertical (Y) error-bar specification on a series patch.
pub const SERIES_Y_ERROR_BARS: &[&str] = &["y_error_bars", "error_bars"];

/// Stroke dash pattern on connecting line objects (`GraphCurveStyle` / `ConnectingLineStyle`).
pub const LINE_DASH_STYLE: &[&str] = &["dash_style", "dash"];

/// Stroke width in points on connecting line objects.
pub const LINE_STROKE_WIDTH: &[&str] = &["stroke_width_pt", "width_pt"];

/// Fill color inside a `MarkerStyle` patch.
pub const MARKER_FILL_COLOR: &[&str] = &["fill_color", "color"];

/// Border toggle for text-box and model-equation graph annotations.
pub const ANNOTATION_BORDERED: &[&str] = &["bordered", "boxed"];

/// Dash pattern on line/arrow and shape graph annotations.
pub const ANNOTATION_DASH: &[&str] = &["dash", "dash_style"];

/// Bounding rectangle in points for shape annotations.
pub const ANNOTATION_BOUNDS: &[&str] = &["bounds_pt", "rect_pt"];

/// Outline stroke color for shape annotations.
pub const ANNOTATION_STROKE_COLOR: &[&str] = &["stroke_color", "color"];

/// Variable/column name on `table.simulateDistributions` column specs.
pub const SIMULATED_COLUMN_NAME: &[&str] = &["name", "title"];

/// Distribution family identifier on `table.simulateDistributions` column specs.
pub const SIMULATED_COLUMN_KIND: &[&str] = &["kind", "type", "distribution"];

/// Preview-only flag for `table.convertArchetype`.
pub const CONVERT_PREVIEW: &[&str] = &["preview_only", "previewOnly", "preview", "dry_run", "dryRun"];

/// Every spelling table in canonical-first precedence order, paired with its constant name.
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

// ---------------------------------------------------------------------------
// Command- and context-aware deprecated-alias mappings for migrations
// ---------------------------------------------------------------------------

type AliasPair = (&'static str, &'static str);

const COORD_ALIASES: &[AliasPair] =
    &[("coordinate", "coord"), ("data_set", "col"), ("dataSet", "col"), ("column", "col"), ("subcolumn", "subcol"), ("replicate", "subcol")];

const FLEX_COORD_ALIASES: &[AliasPair] = &[
    ("coordinate", "coord"),
    ("target_row", "row"),
    ("targetRow", "row"),
    ("target_col", "col"),
    ("targetCol", "col"),
    ("data_set", "col"),
    ("dataSet", "col"),
    ("column", "col"),
    ("target_subcol", "subcol"),
    ("targetSubcol", "subcol"),
    ("subcolumn", "subcol"),
    ("replicate", "subcol"),
];

const CELL_RANGE_ALIASES: &[AliasPair] = &[
    ("from", "start"),
    ("to", "end"),
    ("target_row", "row"),
    ("targetRow", "row"),
    ("target_col", "col"),
    ("targetCol", "col"),
    ("data_set", "col"),
    ("dataSet", "col"),
    ("column", "col"),
    ("target_subcol", "subcol"),
    ("targetSubcol", "subcol"),
    ("subcolumn", "subcol"),
    ("replicate", "subcol"),
];

const CELL_EDIT_ALIASES: &[AliasPair] = &[
    ("coordinate", "coord"),
    ("data_set", "col"),
    ("dataSet", "col"),
    ("column", "col"),
    ("subcolumn", "subcol"),
    ("replicate", "subcol"),
    ("decimalSeparator", "decimal_separator"),
    ("decimal", "decimal_separator"),
    ("parseAsteriskExclusion", "parse_asterisk_exclusion"),
];

const X_CELL_EDIT_ALIASES: &[AliasPair] = &[
    ("subcolumn", "subcol"),
    ("replicate", "subcol"),
    ("decimalSeparator", "decimal_separator"),
    ("decimal", "decimal_separator"),
    ("parseAsteriskExclusion", "parse_asterisk_exclusion"),
];

const PLACEMENT_ALIASES: &[AliasPair] = &[
    ("start", "anchor"),
    ("startRow", "start_row"),
    ("target_row", "start_row"),
    ("targetRow", "start_row"),
    ("row", "start_row"),
    ("startCol", "start_col"),
    ("target_col", "start_col"),
    ("targetCol", "start_col"),
    ("col", "start_col"),
    ("data_set", "start_col"),
    ("dataSet", "start_col"),
    ("startSubcol", "start_subcol"),
    ("target_subcol", "start_subcol"),
    ("targetSubcol", "start_subcol"),
    ("subcol", "start_subcol"),
    ("subcolumn", "start_subcol"),
    ("placement_mode", "mode"),
    ("placementMode", "mode"),
    ("propagateTitleHeader", "propagate_title_header"),
    ("skipContiguousBlanks", "skip_contiguous_blanks"),
];

const FILTER_OBJ_ALIASES: &[AliasPair] = &[
    ("rowStart", "row_start"),
    ("rowEnd", "row_end"),
    ("colStart", "col_start"),
    ("colEnd", "col_end"),
    ("decimateEveryK", "decimate_every_k"),
    ("every_nth_row", "decimate_every_k"),
    ("everyNthRow", "decimate_every_k"),
    ("unstackGroupColumn", "unstack_group_column"),
    ("parseAsteriskExclusions", "parse_asterisk_exclusions"),
    ("parse_asterisk_exclusion", "parse_asterisk_exclusions"),
    ("parseAsteriskExclusion", "parse_asterisk_exclusions"),
];

const BASELINE_OBJ_ALIASES: &[AliasPair] =
    &[("column", "col"), ("reference_col", "col"), ("referenceCol", "col"), ("operation", "op"), ("mode", "op")];

const TIDY_MAPPING_ALIASES: &[AliasPair] =
    &[("xCol", "x_col"), ("groupCol", "group_col"), ("replicateCol", "replicate_col"), ("valueCol", "value_col"), ("excludedCol", "excluded_col")];

const PLATE_ROLE_OBJ_ALIASES: &[AliasPair] = &[
    ("type", "role"),
    ("kind", "role"),
    ("groupName", "group_name"),
    ("group", "group_name"),
    ("dilutionX", "dilution_x"),
    ("x", "dilution_x"),
    ("concentration", "dilution_x"),
    ("rep", "replicate"),
];

const SHEET_ID_BASIC: &[AliasPair] = &[("id", "sheet_id")];
const SHEET_ID_BOTH: &[AliasPair] = &[("sheetId", "sheet_id"), ("id", "sheet_id")];

const ANALYSIS_BASE_TOP: &[AliasPair] = &[
    ("id", "sheet_id"),
    ("source_sheet", "source_sheet_id"),
    ("data_sheet_id", "source_sheet_id"),
    ("custom_title", "title"),
    ("confidence_level", "confidence_percent"),
    ("selected_columns", "selected_data_sets"),
    ("columns", "selected_data_sets"),
    ("model_comparison", "comparison"),
];

const ANALYSIS_UPDATE_TOP: &[AliasPair] = &[
    ("id", "sheet_id"),
    ("custom_title", "title"),
    ("confidence_level", "confidence_percent"),
    ("selected_columns", "selected_data_sets"),
    ("columns", "selected_data_sets"),
    ("patch", "params"),
    ("model_comparison", "comparison"),
];

const SERIES_PATCH_ALIASES: &[AliasPair] = &[
    ("source_sheet", "source_sheet_id"),
    ("data_sheet_id", "source_sheet_id"),
    ("graphical_object", "gdo"),
    ("fill_color", "color"),
    ("markers", "marker"),
    ("line", "connecting_line"),
    ("error_bars", "y_error_bars"),
    ("label", "legend_label"),
];

const FORMAT_POINT_OVERRIDE_ALIASES: &[AliasPair] =
    &[("subcolumn", "subcol"), ("replicate", "subcol"), ("shape", "symbol"), ("stroke_width_pt", "border_width_pt")];

fn lookup_pairs(pairs: &[AliasPair], key: &str) -> Option<&'static str> {
    pairs.iter().find_map(|&(alias, canonical)| (alias == key).then_some(canonical))
}

fn lookup_tables(tables: &[&[&'static str]], key: &str) -> Option<&'static str> {
    for &table in tables {
        if let Some(&canonical) = table.first()
            && table[1..].contains(&key)
        {
            return Some(canonical);
        }
    }
    None
}

fn analysis_shorthand_alias(kind: Option<&str>, key: &str) -> Option<&'static str> {
    let norm_kind = kind.map(|k| k.trim().to_ascii_lowercase());
    let desc_id = norm_kind.as_deref().and_then(crate::analysis::resolve_analysis_descriptor).map(|d| d.id);
    let is_pvalue_stack = desc_id == Some(crate::analysis::AnalysisKindId::PValueStack)
        || matches!(
            norm_kind.as_deref(),
            Some("pvalue_stack" | "p_value_stack" | "pvaluestack" | "pvalues" | "stats.pvalue_stack" | "stats.pvaluestack")
        );
    let is_cox = matches!(norm_kind.as_deref(), Some("cox" | "cox_ph" | "cox_proportional_hazards" | "coxph" | "cox_regression" | "survival.cox"));
    let is_simulate = matches!(norm_kind.as_deref(), Some("simulate" | "simulate_xy" | "transform.simulate" | "simulate_data"));
    let is_glm_or_mv_model =
        matches!(desc_id, Some(crate::analysis::AnalysisKindId::GeneralizedLinearModel | crate::analysis::AnalysisKindId::Multivariate))
            || matches!(
                norm_kind.as_deref(),
                Some(
                    "glm"
                        | "generalized_linear_model"
                        | "linear_glm"
                        | "multiple_linear"
                        | "logistic_glm"
                        | "multiple_logistic"
                        | "poisson_glm"
                        | "multiple_poisson"
                        | "multiple_regression"
                        | "multiple_linear_regression"
                        | "logistic_regression"
                        | "poisson_regression"
                        | "multivariate"
                        | "pls"
                        | "pls_regression"
                        | "pca"
                        | "principal_components"
                        | "principal_component_analysis"
                        | "correlation"
                        | "correlation_matrix"
                        | "clustering"
                        | "kmeans"
                        | "k_means"
                        | "hierarchical_clustering"
                        | "multiple_imputation"
                        | "mice"
                )
            );

    if is_pvalue_stack {
        if let Some(c) = lookup_tables(&[PVALUE_STACK_METHOD, PVALUE_STACK_THRESHOLD, PVALUE_STACK_COLUMN], key) {
            return Some(c);
        }
    } else if norm_kind.is_some() {
        if let Some(c) = lookup_tables(&[MULTIPLICITY, MULTIPLICITY_ALPHA], key) {
            return Some(c);
        }
    } else {
        // When `kind` is omitted (e.g. `analysis.updateParams`), avoid flipping tokens whose
        // canonical/alias polarity reverses across analysis families (`alpha`/`threshold`,
        // `multiplicity`/`method`, `time_column`/`mv_time_column`, `response_column`/`mv_response_column`).
        if let Some(c) = lookup_pairs(
            &[
                ("post_hoc", "multiplicity"),
                ("pvalue_stack_method", "method"),
                ("pvalue_stack_threshold", "threshold"),
                ("pvalue_stack_column", "column"),
            ],
            key,
        ) {
            return Some(c);
        }
    }

    if is_cox && matches!(key, "time_column" | "status_column") {
        return None;
    }

    if is_glm_or_mv_model && let Some(c) = lookup_tables(&[MULTIVARIATE_RESPONSE_COLUMN, MULTIVARIATE_PREDICTOR_COLUMNS], key) {
        return Some(c);
    }

    if is_simulate && key == "params" {
        return Some("parameters");
    }

    if (desc_id == Some(crate::analysis::AnalysisKindId::OutlierDetection)
        || matches!(norm_kind.as_deref(), Some("outliers" | "outlier" | "outlier_detection" | "identify_outliers" | "rout" | "grubbs")))
        && let Some(c) = lookup_tables(&[OUTLIER_Q_PERCENT], key)
    {
        return Some(c);
    }

    let is_km_survival = (desc_id == Some(crate::analysis::AnalysisKindId::Survival) && !is_cox)
        || matches!(norm_kind.as_deref(), Some("kaplan_meier" | "survival" | "log_rank" | "logrank" | "km"));
    if is_km_survival && let Some(c) = lookup_tables(&[MV_TIME_COLUMN, MV_STATUS_COLUMN, MV_GROUP_COLUMN], key) {
        return Some(c);
    }
    if norm_kind.is_some()
        && !is_glm_or_mv_model
        && !is_cox
        && let Some(c) = lookup_tables(&[MV_GROUP_COLUMN, MV_RESPONSE_COLUMN], key)
    {
        return Some(c);
    }

    lookup_tables(
        &[
            TRANSFORM_Y,
            TRANSFORM_ERROR_PROPAGATION,
            TRANSFORM_ZERO_ANCHOR,
            TRANSFORM_HUNDRED_ANCHOR,
            TRANSFORM_SAMPLE_SIZE,
            TRANSFORM_EVERY_K_ROWS,
            TRANSFORM_AVERAGE_WINDOW,
            TRANSFORM_ROW_COUNT,
            TRANSFORM_DERIVED_COLUMNS,
            CONTINGENCY_METHOD,
            CONTINGENCY_YATES,
            MULTIVARIATE_K_CLUSTERS,
            COX_PREDICTOR_COLUMNS,
            SURVIVAL_LOG_RANK,
            SURVIVAL_GEHAN,
            GLM_FAMILY,
            MULTIVARIATE_PREDICTOR_COLUMNS,
            MULTIVARIATE_SPEARMAN,
            MULTIVARIATE_IMPUTATIONS,
            MULTIVARIATE_MAX_ITERS,
            MULTIVARIATE_RESPONSE_COLUMN,
            KAPPA_WEIGHTING,
            CLUSTER_DISTANCE,
            CLUSTER_LINKAGE,
            ONE_SAMPLE_HYPOTHETICAL,
            LINEAR_DEMING,
            OUTLIER_Q_PERCENT,
            POWER_SAMPLE_SIZE,
            POWER_SAMPLE_SIZE_GROUP2,
            POWER_CHI_SQUARE_DF,
            POWER_EFFECT_SIZE,
            POWER_NUM_GROUPS,
            NONLINEAR_COMPARISON,
            NONLINEAR_FIT_METHOD,
            NONLINEAR_NORMALITY_TESTS,
            MODEL_COMPARISON_ALTERNATIVE,
        ],
        key,
    )
}

fn graph_annotation_alias(kind: Option<&str>, key: &str) -> Option<&'static str> {
    if let Some(c) = lookup_pairs(&[("id", "sheet_id"), ("type", "kind"), ("markdown", "text"), ("result_sheet", "result_sheet_id")], key) {
        return Some(c);
    }
    if let Some(c) = lookup_tables(&[ANNOTATION_BORDERED, ANNOTATION_DASH, ANNOTATION_BOUNDS], key) {
        return Some(c);
    }
    let norm = kind.map(|k| k.trim().to_ascii_lowercase());
    if matches!(norm.as_deref(), Some("shape" | "rect" | "ellipse")) && key == "color" {
        return Some("stroke_color");
    }
    None
}

fn is_analysis_family_command(command_id: &str) -> bool {
    matches!(command_id, "analyze.run" | "transform.run" | "stats.run" | "fit.run" | "analysis.add" | "analysis.run" | "analysis.updateParams")
}

fn analysis_nested_alias(path: &[&str], kind: Option<&str>, key: &str) -> Option<&'static str> {
    match path {
        ["derived_columns"] | ["params", "derived_columns"] => lookup_tables(&[DERIVED_COLUMN_TITLE, DERIVED_COLUMN_FORMULA], key),
        ["comparison"] | ["params", "comparison"] => lookup_tables(&[MODEL_COMPARISON_METHOD, MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION], key),
        ["parameter_rules"] | ["params", "parameter_rules"] => {
            lookup_tables(&[PARAMETER_RULE_NAME, PARAMETER_SCOPE_VALUES, PARAMETER_SCOPE_HOOK], key)
        }
        ["reference_levels"] | ["params", "reference_levels"] => lookup_tables(&[REFERENCE_LEVEL_COLUMN, REFERENCE_LEVEL_LEVEL], key),
        ["params"] => analysis_shorthand_alias(kind, key),
        _ => None,
    }
}

fn project_command_alias(command_id: &str, key: &str) -> Option<&'static str> {
    match command_id {
        "file.new" | "project.new" => lookup_pairs(&[("name", "title")], key),
        "project.open" => lookup_pairs(
            &[
                ("file_path", "path"),
                ("filePath", "path"),
                ("bytesBase64", "bytes_base64"),
                ("archive_base64", "bytes_base64"),
                ("unpacked_dir", "unpacked"),
                ("unpackedDir", "unpacked"),
            ],
            key,
        ),
        "project.save" => lookup_pairs(
            &[
                ("file_path", "path"),
                ("filePath", "path"),
                ("returnBytes", "return_bytes"),
                ("include_bytes", "return_bytes"),
                ("includeBytes", "return_bytes"),
                ("unpacked_dir", "unpacked"),
                ("unpackedDir", "unpacked"),
            ],
            key,
        ),
        "project.saveTemplate" => lookup_pairs(
            &[
                ("file_path", "path"),
                ("filePath", "path"),
                ("returnBytes", "return_bytes"),
                ("include_bytes", "return_bytes"),
                ("includeBytes", "return_bytes"),
            ],
            key,
        ),
        "project.instantiateTemplate" => lookup_pairs(
            &[
                ("file_path", "path"),
                ("filePath", "path"),
                ("template_path", "path"),
                ("templatePath", "path"),
                ("bytesBase64", "bytes_base64"),
                ("archive_base64", "bytes_base64"),
                ("project_title", "title"),
                ("projectTitle", "title"),
            ],
            key,
        ),
        "project.inspect" => lookup_pairs(SHEET_ID_BOTH, key),
        "project.recalculate" => lookup_pairs(&[("includeExternalLinks", "include_external_links")], key),
        "project.createCheckpoint" => lookup_pairs(
            &[
                ("label", "name"),
                ("title", "name"),
                ("checkpoint_id", "id"),
                ("checkpointId", "id"),
                ("description", "note"),
                ("timestampUtc", "timestamp_utc"),
                ("created_utc", "timestamp_utc"),
                ("createdUtc", "timestamp_utc"),
            ],
            key,
        ),
        "project.restoreCheckpoint" | "project.deleteCheckpoint" => lookup_pairs(
            &[("checkpoint_id", "id"), ("checkpointId", "id"), ("checkpoint_name", "name"), ("checkpointName", "name"), ("title", "name")],
            key,
        ),
        _ => None,
    }
}

fn info_or_export_or_layout_alias(command_id: &str, key: &str) -> Option<&'static str> {
    match command_id {
        "info.setNote" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("note", "text"),
                ("notes", "text"),
                ("notes_markdown", "text"),
                ("notesMarkdown", "text"),
                ("markdown", "text"),
            ],
            key,
        ),
        "info.setConstant" => lookup_pairs(&[("sheetId", "sheet_id"), ("id", "sheet_id"), ("key", "name")], key),
        "info.removeConstant" => {
            lookup_pairs(&[("sheetId", "sheet_id"), ("id", "sheet_id"), ("key", "name"), ("constant", "name"), ("ifExists", "if_exists")], key)
        }
        "export.pdf" | "graph.exportPdf" => lookup_pairs(&[("id", "sheet_id"), ("vector_text", "outline_text")], key),
        "export.svg" | "graph.exportSvg" | "export.png" | "graph.exportPng" | "export.tiff" | "graph.exportEmf" | "layout.exportEmf"
        | "layout.create" | "layout.setSpec" => lookup_pairs(SHEET_ID_BASIC, key),
        "layout.addPanel" => lookup_pairs(
            &[
                ("id", "sheet_id"),
                ("type", "kind"),
                ("graph_sheet", "graph_sheet_id"),
                ("source_sheet", "source_sheet_id"),
                ("text", "markdown"),
                ("badge", "badge_override"),
            ],
            key,
        ),
        "layout.alignOrigins" => lookup_pairs(&[("id", "sheet_id"), ("align_plot_origins", "align")], key),
        "layout.applyCanvasPreset" | "graph.applyCanvasPreset" => lookup_pairs(&[("id", "sheet_id"), ("mode", "resize_mode")], key),
        _ => None,
    }
}

fn sheet_command_alias(command_id: &str, key: &str) -> Option<&'static str> {
    match command_id {
        "sheet.createData" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("name", "title"),
                ("titles", "variables"),
                ("variable_titles", "variables"),
                ("variableTitles", "variables"),
                ("archetype", "kind"),
                ("table_kind", "kind"),
                ("tableKind", "kind"),
                ("type", "kind"),
                ("row_count", "rows"),
                ("rowCount", "rows"),
                ("slices", "rows"),
                ("columns", "cols"),
                ("data_sets", "cols"),
                ("dataSets", "cols"),
                ("column_count", "cols"),
                ("columnCount", "cols"),
                ("groups", "cols"),
                ("cohorts", "cols"),
                ("replicate_k", "replicates"),
                ("k", "replicates"),
                ("subcolumns", "replicates"),
                ("biological_units", "replicates"),
                ("biologicalUnits", "replicates"),
                ("subcolumnMode", "subcolumn_mode"),
                ("xMode", "x_mode"),
                ("xTitle", "x_title"),
                ("designHint", "design_hint"),
                ("rowFactorTitle", "row_factor_title"),
                ("columnFactorTitle", "column_factor_title"),
                ("timeTitle", "time_title"),
                ("autoFamily", "auto_family"),
                ("autoInfo", "auto_info"),
                ("autoGraph", "auto_graph"),
                ("infoSheetId", "info_sheet_id"),
                ("graphSheetId", "graph_sheet_id"),
            ],
            key,
        ),
        "sheet.rename" => {
            lookup_pairs(&[("sheetId", "sheet_id"), ("id", "sheet_id"), ("new_title", "title"), ("newTitle", "title"), ("name", "title")], key)
        }
        "sheet.delete" | "sheet.freeze" | "sheet.unfreeze" => lookup_pairs(SHEET_ID_BOTH, key),
        "sheet.duplicate" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("newSheetId", "new_sheet_id"),
                ("target_sheet_id", "new_sheet_id"),
                ("targetSheetId", "new_sheet_id"),
                ("new_id", "new_sheet_id"),
                ("new_title", "title"),
                ("newTitle", "title"),
            ],
            key,
        ),
        "sheet.duplicateFamily" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("clearData", "clear_data"),
                ("newSheetId", "new_sheet_id"),
                ("target_sheet_id", "new_sheet_id"),
                ("targetSheetId", "new_sheet_id"),
                ("new_title", "title"),
                ("newTitle", "title"),
            ],
            key,
        ),
        "sheet.clonePipeline" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("newSheetId", "new_sheet_id"),
                ("target_sheet_id", "new_sheet_id"),
                ("targetSheetId", "new_sheet_id"),
                ("new_title", "title"),
                ("newTitle", "title"),
                ("clearData", "clear_data"),
                ("keepXValues", "keep_x_values"),
                ("clearYValues", "clear_y_values"),
                ("clearColumnTitles", "clear_column_titles"),
                ("clearRowTitles", "clear_row_titles"),
                ("overrideReplicates", "override_replicates"),
                ("replicates", "override_replicates"),
                ("cloneAnalyses", "clone_analyses"),
                ("cloneGraphs", "clone_graphs"),
                ("cloneLayouts", "clone_layouts"),
                ("clearResults", "clear_results"),
            ],
            key,
        ),
        "sheet.reorder" => lookup_pairs(
            &[("fromIndex", "from_index"), ("toIndex", "to_index"), ("sheetId", "sheet_id"), ("id", "sheet_id"), ("position", "index")],
            key,
        ),
        "sheet.select" => lookup_pairs(&[("sheetId", "sheet_id"), ("id", "sheet_id"), ("pane", "mode")], key),
        "sheet.extractResultTab" => lookup_pairs(
            &[
                ("source_sheet_id", "result_sheet_id"),
                ("source_sheet", "result_sheet_id"),
                ("result_id", "result_sheet_id"),
                ("id", "sheet_id"),
                ("target_sheet_id", "sheet_id"),
                ("kind", "archetype"),
            ],
            key,
        ),
        _ => None,
    }
}

fn graph_command_alias(command_id: &str, path: &[&str], kind: Option<&str>, key: &str) -> Option<&'static str> {
    if !path.is_empty() {
        return match command_id {
            "graph.setSeries" => match path {
                ["series"] | ["all_series"] => lookup_pairs(SERIES_PATCH_ALIASES, key),
                ["marker"] | ["series", "marker"] | ["all_series", "marker"] => lookup_tables(&[MARKER_FILL_COLOR], key),
                ["connecting_line"] | ["series", "connecting_line"] | ["all_series", "connecting_line"] => {
                    lookup_tables(&[LINE_DASH_STYLE, LINE_STROKE_WIDTH], key)
                }
                _ => None,
            },
            "graph.formatSelectedPoints" => match path {
                ["points"] => lookup_tables(&[POINT_SUBCOLUMN], key),
                ["overrides"] => lookup_pairs(FORMAT_POINT_OVERRIDE_ALIASES, key),
                _ => None,
            },
            _ => None,
        };
    }
    match command_id {
        "graph.create" => lookup_pairs(&[("source_sheet_id", "data_sheet_id"), ("source_sheet", "data_sheet_id"), ("id", "sheet_id")], key),
        "graph.initialize" | "graph.setKind" | "graph.setSpec" | "graph.preflightCheck" => lookup_pairs(SHEET_ID_BASIC, key),
        "graph.setAxes" => lookup_pairs(&[("id", "sheet_id"), ("which", "axis")], key),
        "graph.setAdditionalTicks" => {
            lookup_pairs(&[("id", "sheet_id"), ("graph_sheet_id", "sheet_id"), ("which", "axis"), ("additional_ticks", "ticks")], key)
        }
        "graph.setSeries" => {
            lookup_pairs(&[("id", "sheet_id"), ("allSeries", "all_series")], key).or_else(|| lookup_pairs(SERIES_PATCH_ALIASES, key))
        }
        "graph.setRegressionOverlay" | "graph.addComparisonBrackets" => lookup_pairs(&[("id", "sheet_id"), ("result_sheet", "result_sheet_id")], key),
        "graph.addAnnotation" => graph_annotation_alias(kind, key),
        "graph.applyStyleFrom" => lookup_pairs(
            &[("target_sheet_id", "sheet_id"), ("id", "sheet_id"), ("from_sheet_id", "donor_sheet_id"), ("source_graph_id", "donor_sheet_id")],
            key,
        ),
        "graph.formatSelectedPoints" => {
            lookup_pairs(&[("id", "sheet_id"), ("graph_sheet_id", "sheet_id"), ("shape", "symbol"), ("stroke_width_pt", "border_width_pt")], key)
        }
        "graph.setCompactLetterDisplay" => {
            lookup_pairs(&[("id", "sheet_id"), ("result_sheet", "result_sheet_id"), ("display_mode", "pairwise_display_mode")], key)
        }
        _ => None,
    }
}

fn table_nested_alias(command_id: &str, path: &[&str], key: &str) -> Option<&'static str> {
    if matches!(path.last().copied(), Some("coord" | "cells" | "coords")) {
        return lookup_pairs(COORD_ALIASES, key);
    }
    if matches!(path.last().copied(), Some("start" | "end" | "anchor")) {
        return lookup_pairs(FLEX_COORD_ALIASES, key);
    }
    if matches!(path.last().copied(), Some("range" | "source_range")) {
        return lookup_pairs(CELL_RANGE_ALIASES, key);
    }
    if matches!(path.last().copied(), Some("filter")) {
        return lookup_pairs(FILTER_OBJ_ALIASES, key);
    }
    if matches!(path.last().copied(), Some("baseline")) {
        return lookup_pairs(BASELINE_OBJ_ALIASES, key);
    }

    match (command_id, path) {
        ("table.setCells", ["edits"]) => lookup_pairs(CELL_EDIT_ALIASES, key),
        ("table.setXCells", ["edits"]) => lookup_pairs(X_CELL_EDIT_ALIASES, key),
        ("table.recodeLevels", ["mappings"]) => lookup_pairs(&[("old", "from"), ("source", "from"), ("new", "to"), ("target", "to")], key),
        ("table.setCategoricalOrder", ["order_spec"]) => lookup_pairs(&[("kind", "order"), ("mode", "order")], key),
        ("table.simulateXy", ["parameters"]) => lookup_pairs(&[("key", "name")], key),
        ("table.simulateDistributions", ["columns"]) => lookup_tables(&[SIMULATED_COLUMN_NAME, SIMULATED_COLUMN_KIND], key),
        ("table.randomizeGroups", ["groups"]) => lookup_pairs(&[("name", "label")], key),
        ("table.importTidyLong", ["mapping"]) => lookup_pairs(TIDY_MAPPING_ALIASES, key),
        ("table.importPlateMap", ["template"]) => lookup_pairs(
            &[("plate_geometry", "geometry"), ("plateGeometry", "geometry"), ("title", "name"), ("xTitle", "x_title"), ("roles", "wells")],
            key,
        ),
        ("table.importPlateMap", ["roles"]) | ("table.importPlateMap", ["template", "wells"]) => lookup_pairs(PLATE_ROLE_OBJ_ALIASES, key),
        _ => None,
    }
}

fn table_command_alias(command_id: &str, path: &[&str], key: &str) -> Option<&'static str> {
    if !path.is_empty() {
        return table_nested_alias(command_id, path, key);
    }
    match command_id {
        "table.setCell" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("coordinate", "coord"),
                ("data_set", "col"),
                ("dataSet", "col"),
                ("column", "col"),
                ("subcolumn", "subcol"),
                ("replicate", "subcol"),
                ("decimalSeparator", "decimal_separator"),
                ("decimal", "decimal_separator"),
                ("parseAsteriskExclusion", "parse_asterisk_exclusion"),
            ],
            key,
        ),
        "table.setCells" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("cells", "edits"),
                ("grid", "rows"),
                ("startRow", "start_row"),
                ("startCol", "start_col"),
                ("startSubcol", "start_subcol"),
                ("data_set", "col"),
                ("dataSet", "col"),
                ("column", "col"),
                ("subcolumn", "subcol"),
                ("replicate", "subcol"),
                ("decimalSeparator", "decimal_separator"),
                ("decimal", "decimal_separator"),
                ("parseAsteriskExclusion", "parse_asterisk_exclusion"),
            ],
            key,
        ),
        "table.setXCells" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("cells", "edits"),
                ("x", "values"),
                ("startRow", "start_row"),
                ("subcolumn", "subcol"),
                ("replicate", "subcol"),
                ("decimalSeparator", "decimal_separator"),
                ("decimal", "decimal_separator"),
                ("parseAsteriskExclusion", "parse_asterisk_exclusion"),
            ],
            key,
        ),
        "table.setRowTitles" => {
            lookup_pairs(&[("sheetId", "sheet_id"), ("row_titles", "titles"), ("rowTitles", "titles"), ("startRow", "start_row")], key)
        }
        "table.setGroupTitles" | "table.setTitles" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("xTitle", "x_title"),
                ("time_title", "x_title"),
                ("timeTitle", "x_title"),
                ("rowFactorTitle", "row_factor_title"),
                ("columnFactorTitle", "column_factor_title"),
                ("group_titles", "titles"),
                ("groupTitles", "titles"),
                ("column_titles", "titles"),
                ("columnTitles", "titles"),
                ("data_set_titles", "titles"),
                ("dataSetTitles", "titles"),
                ("y_titles", "titles"),
                ("yTitles", "titles"),
                ("startCol", "start_col"),
                ("group", "col"),
                ("data_set", "col"),
                ("dataSet", "col"),
                ("column", "col"),
                ("index", "col"),
                ("rowTitles", "row_titles"),
            ],
            key,
        ),
        "table.setSubcolumnTitles" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("group", "col"),
                ("data_set", "col"),
                ("dataSet", "col"),
                ("column", "col"),
                ("subcolumn_titles", "titles"),
                ("subcolumnTitles", "titles"),
                ("startSubcol", "start_subcol"),
                ("subcolumn", "subcol"),
                ("replicate", "subcol"),
                ("index", "subcol"),
            ],
            key,
        ),
        "table.excludeCells" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("selection", "range"),
                ("coords", "cells"),
                ("coordinate", "coord"),
                ("data_set", "col"),
                ("dataSet", "col"),
                ("column", "col"),
                ("subcolumn", "subcol"),
                ("replicate", "subcol"),
                ("rowStart", "row_start"),
                ("rowEnd", "row_end"),
                ("colStart", "col_start"),
                ("colEnd", "col_end"),
                ("subcolStart", "subcol_start"),
                ("subcolEnd", "subcol_end"),
            ],
            key,
        ),
        "table.excludeCellsWithReason" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("reasonTag", "reason_tag"),
                ("reason", "reason_tag"),
                ("comment", "note"),
                ("timestampUtc", "timestamp_utc"),
                ("created_utc", "timestamp_utc"),
                ("createdUtc", "timestamp_utc"),
                ("cells", "coords"),
                ("coordinate", "coord"),
                ("data_set", "col"),
                ("dataSet", "col"),
                ("column", "col"),
                ("subcolumn", "subcol"),
                ("replicate", "subcol"),
            ],
            key,
        ),
        "table.exclusionAuditReport" => lookup_pairs(SHEET_ID_BOTH, key),
        "table.clearCells" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("clearX", "clear_x"),
                ("include_x", "clear_x"),
                ("includeX", "clear_x"),
                ("break_links", "unlink"),
                ("breakLinks", "unlink"),
                ("selection", "range"),
                ("coords", "cells"),
                ("coordinate", "coord"),
                ("data_set", "col"),
                ("dataSet", "col"),
                ("column", "col"),
                ("subcolumn", "subcol"),
                ("replicate", "subcol"),
                ("rowStart", "row_start"),
                ("rowEnd", "row_end"),
                ("colStart", "col_start"),
                ("colEnd", "col_end"),
                ("subcolStart", "subcol_start"),
                ("subcolEnd", "subcol_end"),
            ],
            key,
        ),
        "table.paste" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("target_sheet_id", "sheet_id"),
                ("targetSheetId", "sheet_id"),
                ("tsv", "text"),
                ("csv", "text"),
                ("clipboard", "text"),
                ("grid", "rows"),
                ("decimalSeparator", "decimal_separator"),
                ("decimal", "decimal_separator"),
                ("normalizeDecimal", "normalize_decimal"),
                ("slice", "filter"),
            ],
            key,
        )
        .or_else(|| lookup_pairs(PLACEMENT_ALIASES, key)),
        "table.pasteLink" => lookup_pairs(
            &[
                ("targetSheetId", "target_sheet_id"),
                ("sheet_id", "target_sheet_id"),
                ("sheetId", "target_sheet_id"),
                ("linkSpec", "link_spec"),
                ("sourceSheetId", "source_sheet_id"),
                ("sourceKind", "source_kind"),
                ("tabId", "tab_id"),
                ("source_tab_id", "tab_id"),
                ("sourceTabId", "tab_id"),
                ("sourceRange", "source_range"),
                ("range", "source_range"),
                ("sourcePath", "source_path"),
                ("path", "source_path"),
                ("file_path", "source_path"),
                ("filePath", "source_path"),
                ("decimalSeparator", "decimal_separator"),
                ("decimal", "decimal_separator"),
                ("normalizeDecimal", "normalize_decimal"),
                ("slice", "filter"),
                ("excludeRowPredicate", "exclude_row_predicate"),
                ("linkId", "link_id"),
                ("provenance_summary", "provenance"),
            ],
            key,
        )
        .or_else(|| lookup_pairs(PLACEMENT_ALIASES, key)),
        "table.importCsv" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("tsv", "csv"),
                ("text", "csv"),
                ("content", "csv"),
                ("file_path", "path"),
                ("filePath", "path"),
                ("decimalSeparator", "decimal_separator"),
                ("decimal", "decimal_separator"),
                ("normalizeDecimal", "normalize_decimal"),
                ("hasHeader", "has_header"),
                ("first_row_titles", "has_header"),
                ("firstRowTitles", "has_header"),
                ("hasRowTitles", "has_row_titles"),
                ("k", "replicates"),
                ("subcolumnMode", "subcolumn_mode"),
                ("kind", "archetype"),
                ("table_kind", "archetype"),
                ("tableKind", "archetype"),
                ("slice", "filter"),
                ("live_link", "link"),
                ("liveLink", "link"),
            ],
            key,
        )
        .or_else(|| lookup_pairs(PLACEMENT_ALIASES, key)),
        "table.exportCsv" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("file_path", "path"),
                ("filePath", "path"),
                ("decimalSeparator", "decimal_separator"),
                ("decimal", "decimal_separator"),
            ],
            key,
        ),
        "table.importPrism" => {
            lookup_pairs(&[("file_path", "path"), ("filePath", "path"), ("bytesBase64", "bytes_base64"), ("archive_base64", "bytes_base64")], key)
        }
        "table.importPzfx" => {
            lookup_pairs(&[("text", "xml"), ("content", "xml"), ("file_path", "path"), ("filePath", "path"), ("bytesBase64", "bytes_base64")], key)
        }
        "table.setVariableRole" => lookup_pairs(
            &[("sheetId", "sheet_id"), ("column", "col"), ("column_index", "col"), ("columnIndex", "col"), ("index", "col"), ("variable", "col")],
            key,
        ),
        "table.recodeLevels" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("column", "col"),
                ("column_index", "col"),
                ("columnIndex", "col"),
                ("index", "col"),
                ("variable", "col"),
                ("mapping", "mappings"),
                ("map", "mappings"),
                ("recode", "mappings"),
            ],
            key,
        ),
        "table.setCategoricalOrder" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("column", "col"),
                ("column_index", "col"),
                ("columnIndex", "col"),
                ("index", "col"),
                ("variable", "col"),
                ("orderSpec", "order_spec"),
                ("explicit", "levels"),
                ("kind", "order"),
                ("mode", "order"),
            ],
            key,
        ),
        "table.setCalculatedVariable" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("column", "col"),
                ("column_index", "col"),
                ("columnIndex", "col"),
                ("index", "col"),
                ("variable", "col"),
                ("name", "title"),
                ("expression", "formula"),
            ],
            key,
        ),
        "table.simulateXy" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("name", "title"),
                ("rows", "row_count"),
                ("rowCount", "row_count"),
                ("k", "replicates"),
                ("subcolumns", "replicates"),
                ("xMin", "x_min"),
                ("xMax", "x_max"),
                ("equationId", "equation_id"),
                ("equation", "equation_id"),
                ("params", "parameters"),
                ("noiseKind", "noise_kind"),
                ("noise", "noise_kind"),
                ("noiseSd", "noise_sd"),
                ("sd", "noise_sd"),
                ("cvPercent", "cv_percent"),
                ("autoInfo", "auto_info"),
                ("graphSheetId", "graph_sheet_id"),
            ],
            key,
        ),
        "table.convertArchetype" => lookup_pairs(
            &[("sheetId", "sheet_id"), ("id", "sheet_id"), ("targetKind", "target_kind"), ("kind", "target_kind"), ("allowLossy", "allow_lossy")],
            key,
        )
        .or_else(|| lookup_tables(&[CONVERT_PREVIEW], key)),
        "table.importPlateMap" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("name", "title"),
                ("plate_geometry", "geometry"),
                ("plateGeometry", "geometry"),
                ("readings", "values"),
                ("well_values", "values"),
                ("plate_template", "template"),
                ("plateTemplate", "template"),
                ("wells", "roles"),
                ("templateName", "template_name"),
                ("xTitle", "x_title"),
                ("targetKind", "target_kind"),
                ("kind", "target_kind"),
                ("subtractBlankMean", "subtract_blank_mean"),
            ],
            key,
        ),
        "table.importTidyLong" => lookup_pairs(
            &[
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("name", "title"),
                ("targetKind", "target_kind"),
                ("kind", "target_kind"),
                ("text", "csv"),
                ("tsv", "csv"),
                ("source", "csv"),
                ("file_path", "path"),
                ("filePath", "path"),
                ("decimalSep", "decimal_sep"),
                ("decimal_separator", "decimal_sep"),
                ("decimalSeparator", "decimal_sep"),
                ("normalizeDecimal", "normalize_decimal"),
                ("column_mapping", "mapping"),
                ("columnMapping", "mapping"),
            ],
            key,
        )
        .or_else(|| lookup_pairs(TIDY_MAPPING_ALIASES, key)),
        "table.randomizeGroups" => lookup_pairs(
            &[
                ("n_subjects", "subject_count"),
                ("count", "subject_count"),
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("name", "title"),
                ("info_sheet", "info_sheet_id"),
            ],
            key,
        ),
        "table.randomSampleRows" => lookup_pairs(
            &[
                ("source_sheet", "source_sheet_id"),
                ("data_sheet_id", "source_sheet_id"),
                ("n_samples", "sample_size"),
                ("count", "sample_size"),
                ("stratify_col", "stratify_column"),
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("name", "title"),
                ("info_sheet", "info_sheet_id"),
            ],
            key,
        ),
        "table.simulateDistributions" => lookup_pairs(
            &[
                ("row_count", "n_rows"),
                ("rows", "n_rows"),
                ("multiple_variables", "use_multiple_variables"),
                ("sheetId", "sheet_id"),
                ("id", "sheet_id"),
                ("name", "title"),
            ],
            key,
        ),
        _ => None,
    }
}

/// Resolve a deprecated parameter spelling `key` for `command_id` at nested object `path`.
///
/// `path` holds canonical (or post-parent-normalization) property names from the root of
/// `params` down to the current JSON object (for example, `&[]` at root, `&["mapping"]`
/// inside `table.importTidyLong.mapping`, or `&["template"]` inside
/// `table.importPlateMap.template`).
///
/// `kind` supplies the optional context discriminant of the enclosing object (for example, the
/// analysis `kind` on `analysis.run`/`stats.run`/`fit.run` or annotation `kind` on
/// `graph.addAnnotation`).
///
/// Returns `Some(canonical_key)` when `key` is a retired spelling for `canonical_key`, or
/// `None` if `key` is already canonical or unrecognized for that command/context.
#[must_use]
pub fn canonical_command_param_alias(command_id: &str, path: &[&str], kind: Option<&str>, key: &str) -> Option<&'static str> {
    if is_analysis_family_command(command_id) {
        if !path.is_empty() {
            return analysis_nested_alias(path, kind, key);
        }
        if command_id == "analysis.updateParams" {
            return lookup_pairs(ANALYSIS_UPDATE_TOP, key).or_else(|| analysis_shorthand_alias(kind, key));
        }
        return lookup_pairs(ANALYSIS_BASE_TOP, key).or_else(|| analysis_shorthand_alias(kind, key));
    }

    if !path.is_empty() {
        return match command_id {
            "graph.setSeries" | "graph.formatSelectedPoints" => graph_command_alias(command_id, path, kind, key),
            _ if command_id.starts_with("table.") => table_command_alias(command_id, path, key),
            _ => None,
        };
    }

    match command_id {
        "analysis.retargetData" => {
            lookup_pairs(&[("id", "sheet_id"), ("source_sheet", "source_sheet_id"), ("data_sheet_id", "source_sheet_id")], key)
        }
        "analysis.exportJson" | "analysis.exportCsv" => lookup_pairs(SHEET_ID_BASIC, key),
        "result.generateReport" => lookup_pairs(&[("sheet_id", "result_sheet_id"), ("id", "result_sheet_id")], key),
        "stats.pvalueStack" => lookup_pairs(
            &[
                ("id", "sheet_id"),
                ("source_sheet", "source_sheet_id"),
                ("source", "source_sheet_id"),
                ("data_sheet_id", "source_sheet_id"),
                ("selected_columns", "selected_data_sets"),
                ("columns", "selected_data_sets"),
            ],
            key,
        )
        .or_else(|| lookup_tables(&[PVALUE_STACK_METHOD, PVALUE_STACK_THRESHOLD, PVALUE_STACK_COLUMN], key)),
        _ if command_id.starts_with("file.") || command_id.starts_with("project.") => project_command_alias(command_id, key),
        _ if command_id.starts_with("info.") || command_id.starts_with("export.") || command_id.starts_with("layout.") => {
            info_or_export_or_layout_alias(command_id, key)
        }
        _ if command_id.starts_with("sheet.") => sheet_command_alias(command_id, key),
        _ if command_id.starts_with("graph.") => {
            info_or_export_or_layout_alias(command_id, key).or_else(|| graph_command_alias(command_id, path, kind, key))
        }
        _ if command_id.starts_with("table.") => table_command_alias(command_id, path, key),
        _ => None,
    }
}

/// Resolve a retired key inside a `curvebook.batch.v1` `inputs[]` entry to its canonical key.
#[must_use]
pub fn canonical_batch_input_key(key: &str) -> Option<&'static str> {
    lookup_pairs(
        &[
            ("sheetId", "sheet"),
            ("sheet_id", "sheet"),
            ("tableKind", "kind"),
            ("table_kind", "kind"),
            ("subcolumnMode", "subcolumn_mode"),
            ("decimalSeparator", "decimal_separator"),
        ],
        key,
    )
}

/// Resolve a retired key inside a `curvebook.batch.v1` `exports[]` entry to its canonical key.
#[must_use]
pub fn canonical_batch_export_key(key: &str) -> Option<&'static str> {
    lookup_pairs(
        &[("sheetId", "sheet"), ("sheet_id", "sheet"), ("tabIndex", "tab_index"), ("pdfVersion", "pdf_version"), ("ttfFont", "ttf_font")],
        key,
    )
}

/// Resolve a retired command-selector key inside a `curvebook.batch.v1` `commands[]` item to `"cmd"`.
#[must_use]
pub fn canonical_batch_command_key(key: &str) -> Option<&'static str> {
    lookup_pairs(&[("command", "cmd"), ("method", "cmd")], key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_spelling_tables_have_unique_canonical_snake_case_primary() {
        assert_eq!(ALL.len(), 70);
        for (name, list) in ALL {
            assert!(list.len() >= 2, "{name} should have at least one alias");
            let primary = list[0];
            assert!(!primary.is_empty(), "{name} primary must be non-empty");
            assert!(!primary.chars().any(char::is_uppercase), "{name} primary `{primary}` must be lowercase snake_case");
            for (idx, candidate) in list.iter().enumerate() {
                assert!(list.iter().position(|x| x == candidate) == Some(idx), "{name} contains duplicate spelling `{candidate}`");
            }
        }
    }

    #[test]
    fn context_sensitive_disambiguation_matches_schemas() {
        assert_eq!(canonical_command_param_alias("sheet.rename", &[], None, "new_title"), Some("title"));
        assert_eq!(canonical_command_param_alias("table.importPlateMap", &[], None, "wells"), Some("roles"));
        assert_eq!(canonical_command_param_alias("table.importPlateMap", &["template"], None, "roles"), Some("wells"));
        assert_eq!(canonical_command_param_alias("graph.addAnnotation", &[], Some("shape"), "color"), Some("stroke_color"));
        assert_eq!(canonical_command_param_alias("graph.addAnnotation", &[], Some("line"), "color"), None);
        assert_eq!(canonical_command_param_alias("stats.run", &[], Some("pvalue_stack"), "alpha"), Some("threshold"));
        assert_eq!(canonical_command_param_alias("stats.run", &[], Some("one_way_anova"), "threshold"), Some("alpha"));
        assert_eq!(canonical_command_param_alias("stats.run", &[], Some("cox_ph"), "time_column"), None);
        assert_eq!(canonical_command_param_alias("stats.run", &[], Some("kaplan_meier"), "time_column"), Some("mv_time_column"));
        assert_eq!(canonical_batch_input_key("sheetId"), Some("sheet"));
        assert_eq!(canonical_batch_export_key("pdfVersion"), Some("pdf_version"));
        assert_eq!(canonical_batch_command_key("method"), Some("cmd"));
    }
}
