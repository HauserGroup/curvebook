//! Scientific transform, statistical test, curve fit, and result reporting/export commands.

use std::collections::HashSet;

use curvebook_doc::{
    AnalysisDomainKind, AnalysisKind, AnalysisSpec, BaselineOperation, BlandAltmanSpec, CellValue, ClusterDistance, ClusterLinkage,
    ContingencyMethod, ContingencySpec, CoxTieMethod, DataSheet, DerivedColumnFormula, DescriptiveSpec, DocError, ErrorCode, ErrorPropagationMode,
    FactorialAnovaSpec, FitMethod, GlmFamily, GlmSpec, GraphSheet, HookSource, KappaWeighting, LeveneCenter, LeveneTestSpec, LinearRegressionSpec,
    MAX_SHEETS_PER_PROJECT, MathTransformPreset, ModelComparisonSpec, MultipleRowTestsSpec, MultipleVariablesTable, MultiplicityMethod,
    MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, OneWayMethod, OutlierSpec,
    PValueStackSpec, ParameterRule, ParameterScope, PassingBablokSpec, PowerAnalysisSpec, PowerEffectSpec, PowerOneSidedDirection, PowerTailMode,
    ProjectDocument, RepeatedMeasuresDesign, ResultSheet, ResultStatus, RocCiMethod, RocSpec, SheetId, SimpleLogisticSpec, SimulationNoise,
    SplineLowessMethod, SplineLowessSpec, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod,
    TwoGroupSpec, VariableColumn, VariableRole, XyTable,
};
use curvebook_fit::{get_builtin_model, list_builtin_equations, resolve_model_from_spec};
use curvebook_format::{CsvOptions, export_result_sheet_json, try_export_result_sheet_tidy_csv, try_export_result_tab_csv};
use curvebook_math::PValueFormat;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::export::de_opt_usize;
use super::helpers::{
    always_enabled, bad_input, has_analyzable_data_sheet, has_result_sheet, next_unique_sheet_id, parse_serde_enum, parse_sheet_id,
    require_analyzable_data_sheet, require_result_sheet, sheet_id_exists, write_output_file,
};
use super::params::{
    any_json_schema, array_schema, bool_schema, deserialize_strict, int_schema, non_empty_str_schema, nullable_schema, num_schema, obj_schema,
    parse_params, str_schema,
};
use super::{CommandGroup, CommandSpec, Ctx, Effects, command_specs};
use crate::dag;
use crate::methods_report::{ReportFormat, generate_methods_report};
use crate::{CommandOutcome, EngineError, ExternalPathPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalysisDomain {
    Any,
    TransformOnly,
    StatsOnly,
    FitOnly,
}

type AxisRangeFilter = (Option<f64>, Option<f64>);

const MUTATE_AND_IDEMPOTENT: Effects = Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT);
const EXPORT_EFFECTS: Effects = Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT);

/// Return all registered analysis and result reporting command specifications.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

static SPECS: &[CommandSpec] = &command_specs!(
    CommandGroup::Analysis;
    {
        id: "analyze.run",
        label: "Run Analyses",
        menu_path: "Analyze/Run Analyses",
        shortcut: "Cmd+Enter",
        summary: "Recalculate all dirty or upstream-dependent analysis result sheets across the project DAG.",
        beginner: "Refreshes every analysis sheet so all tables, stats, and curve fits match the latest data.",
        effects: MUTATE_AND_IDEMPOTENT,
        input_schema: schema_analyze_run,
        enabled: always_enabled,
        run: run_analyze_run,
    },
    {
        id: "transform.run",
        label: "Run Transformation",
        menu_path: "Analyze/Transform...",
        summary: "Create and execute a mathematical, normalization, baseline, transpose, prune, select, or simulation transformation sheet.",
        beginner: "Runs a reproducible data transformation pipeline on a data sheet and writes the output to a linked result sheet.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_transform_run,
        enabled: has_analyzable_data_sheet,
        run: run_transform_run,
    },
    {
        id: "stats.run",
        label: "Run Statistical Test",
        menu_path: "Analyze/Statistical Test...",
        summary: "Create and execute a hypothesis test, ANOVA, contingency, survival, ROC, GLM, or multivariate analysis.",
        beginner: "Runs a statistical test on a compatible data sheet and stores tables, P values, and assumption checks.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_stats_run,
        enabled: has_analyzable_data_sheet,
        run: run_stats_run,
    },
    {
        id: "fit.run",
        label: "Run Curve Fit / Regression",
        menu_path: "Analyze/Fit Curve...",
        summary: "Create and execute linear, Deming, built-in nonlinear, global shared-parameter, or custom DSL regression.",
        beginner: "Fits a linear or nonlinear dose-response/kinetic curve with confidence bands and model comparisons.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_fit_run,
        enabled: has_analyzable_data_sheet,
        run: run_fit_run,
    },
    {
        id: "analysis.add",
        label: "Add Analysis",
        menu_path: "Analyze/Add Analysis...",
        summary: "Attach and compute a new analysis of any domain (transform, stats, or regression) for a data sheet.",
        beginner: "Adds a new live analysis sheet linked to an input table and computes its initial results.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_analysis_run,
        enabled: has_analyzable_data_sheet,
        run: run_analysis_add,
    },
    {
        id: "analysis.run",
        label: "New / Run Analysis",
        menu_path: "Analyze/Analyze Data...",
        summary: "Create and run a new analysis sheet, recalculate an existing result sheet by ID, or recalculate the project when empty.",
        beginner: "Runs a new analysis when source and kind are provided, or refreshes an existing analysis sheet.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_analysis_run,
        enabled: always_enabled,
        run: run_analysis_run,
    },
    {
        id: "analysis.updateParams",
        label: "Change Analysis Parameters",
        menu_path: "Change/Analysis Parameters...",
        summary: "Update analysis kind, confidence level, dataset selection, P-value formatting, or parameter rules and recalculate.",
        beginner: "Modifies settings on an existing result sheet without recreating it and refreshes downstream sheets.",
        effects: MUTATE_AND_IDEMPOTENT,
        input_schema: schema_analysis_update_params,
        enabled: has_result_sheet,
        run: run_analysis_update_params,
    },
    {
        id: "analysis.retargetData",
        label: "Change Data Analyzed",
        menu_path: "Change/Data Analyzed...",
        summary: "Rebind an existing analysis result sheet to a different compatible data sheet or subset of datasets.",
        beginner: "Points an existing analysis at a different data table and recalculates it immediately.",
        effects: MUTATE_AND_IDEMPOTENT,
        input_schema: schema_analysis_retarget_data,
        enabled: has_result_sheet,
        run: run_analysis_retarget_data,
    },
    {
        id: "analysis.exportJson",
        label: "Export Result Sheet JSON",
        menu_path: "File/Export/Results JSON...",
        summary: "Serialize a result sheet (tables, scalars, parameters, checklist) to JSON and optionally save it atomically.",
        beginner: "Exports an analysis result sheet as structured JSON text or saves it to a `.json` file.",
        effects: EXPORT_EFFECTS,
        input_schema: schema_analysis_export_json,
        enabled: has_result_sheet,
        run: run_analysis_export_json,
    },
    {
        id: "analysis.exportCsv",
        label: "Export Result Sheet CSV",
        menu_path: "File/Export/Results CSV...",
        summary: "Export a specific tab or tidy combined CSV representation of a result sheet and optionally save it atomically.",
        beginner: "Exports result tables to CSV/TSV text with configurable delimiter and decimal separator.",
        effects: EXPORT_EFFECTS,
        input_schema: schema_analysis_export_csv,
        enabled: has_result_sheet,
        run: run_analysis_export_csv,
    },
    {
        id: "result.generateReport",
        label: "Generate Methods Report",
        menu_path: "File/Export/Methods Report...",
        summary: "Generate a publication-ready statistical methods prose report in `plain`, `markdown`, or `latex`, optionally saving to disk.",
        beginner: "Creates a narrative methods paragraph describing the statistical test, sample sizes, and options used.",
        effects: EXPORT_EFFECTS,
        input_schema: schema_result_generate_report,
        enabled: has_result_sheet,
        run: run_result_generate_report,
    },
    {
        id: "stats.pvalueStack",
        label: "Analyze a Stack of P Values",
        menu_path: "Analyze/Column Analyses/Analyze a Stack of P Values...",
        summary: "Adjust a column stack of raw P values across 7 FWER/FDR multiplicity procedures and report discoveries.",
        beginner: "Adjusts a column of P values using Bonferroni, Holm, Šídák, Holm-Šídák, Benjamini-Hochberg, BKY two-stage, or Benjamini-Yekutieli.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_stats_pvalue_stack,
        enabled: has_analyzable_data_sheet,
        run: run_stats_pvalue_stack,
    },
);

// ---------------------------------------------------------------------------
// Shared JSON Utilities (reused by graph.rs and layout.rs)
// ---------------------------------------------------------------------------

/// Merge `overlay` onto serialized JSON form of `base`.
pub(super) fn merge_json_struct<T: Serialize>(base: &T, overlay: &Value) -> Result<Value, EngineError> {
    let mut base_val =
        serde_json::to_value(base).map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("serialization error: {err}")))?;
    merge_json_values(&mut base_val, overlay);
    Ok(base_val)
}

/// Deep-merge JSON objects (`overlay` overrides keys in `base`).
pub(super) fn merge_json_values(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(over_map)) => {
            for (k, v) in over_map {
                match base_map.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge_json_values(existing, v),
                    _ => {
                        base_map.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (target, source) => {
            *target = source.clone();
        }
    }
}

/// Return a clone of object `val` omitting any key listed in `ignored`.
pub(super) fn strip_keys(val: &Value, ignored: &[&str]) -> Value {
    let Some(map) = val.as_object() else {
        return val.clone();
    };
    let mut out = Map::new();
    for (k, v) in map {
        if !ignored.contains(&k.as_str()) {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

fn parse_u64_json_value(val: &Value, label: &str) -> Result<u64, EngineError> {
    if let Some(raw) = val.as_u64() {
        return Ok(raw);
    }
    const MAX_SAFE_INT_F64: f64 = 9_007_199_254_740_991.0;
    if let Some(f) = val.as_f64()
        && f.is_finite()
        && f >= 0.0
        && f.fract() == 0.0
        && f <= MAX_SAFE_INT_F64
    {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Ok(f as u64);
    }
    Err(EngineError::with_code(ErrorCode::BadInput, format!("`{label}` must be a non-negative integer")))
}

fn parse_usize_json_value(val: &Value, label: &str) -> Result<usize, EngineError> {
    let raw = parse_u64_json_value(val, label)?;
    usize::try_from(raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, format!("`{label}` exceeds platform index bounds")))
}

// ---------------------------------------------------------------------------
// Schemas & Typed Parameter Structs
// ---------------------------------------------------------------------------

/// Deserialize an optional nullable field so omitted (`None`) is distinct from explicit JSON `null` (`Some(None)`).
pub(super) fn de_present_opt<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
struct AnalyzeRunParams {}

fn schema_analyze_run() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true
    })
}

#[derive(Debug, Deserialize)]
struct RunAnalysisParams {
    #[serde(default)]
    sheet_id: Option<String>,
    #[serde(default)]
    source_sheet_id: Option<String>,
    #[serde(default)]
    spec: Option<Value>,
    #[serde(default)]
    kind: Option<Value>,
    #[serde(default)]
    overwrite: Option<bool>,
    #[serde(default, deserialize_with = "de_present_opt")]
    title: Option<Option<String>>,
    #[serde(default)]
    confidence_percent: Option<f64>,
    #[serde(default)]
    selected_data_sets: Option<Vec<Value>>,
    #[serde(default)]
    p_value_format: Option<Value>,
    #[serde(default)]
    create_graph: Option<bool>,
    #[serde(default)]
    graph_sheet_id: Option<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn kind_schema(description: &str) -> Value {
    json!({
        "description": description,
        "anyOf": [
            { "type": "string", "minLength": 1 },
            { "type": "object" },
            { "type": "null" }
        ]
    })
}

fn schema_analysis_base(summary: &str, include_fit_extras: bool, include_stats_extras: bool) -> Value {
    let mut props = vec![
        ("sheet_id", nullable_schema(non_empty_str_schema("Optional target result sheet ID"))),
        ("source_sheet_id", nullable_schema(non_empty_str_schema("Source data sheet ID"))),
        ("spec", nullable_schema(json!({ "type": "object", "description": "Full serialized AnalysisSpec payload" }))),
        ("kind", kind_schema(summary)),
        ("overwrite", nullable_schema(bool_schema("Overwrite an existing result sheet with the same ID"))),
        ("title", nullable_schema(str_schema("Optional custom title override"))),
        ("confidence_percent", nullable_schema(num_schema("Confidence percentage in (0, 100)", Some(0.0), Some(100.0)))),
        (
            "selected_data_sets",
            nullable_schema(array_schema(
                "Subset of zero-based dataset column indices to analyze",
                int_schema("Dataset column index", Some(0), None),
                None,
                None,
            )),
        ),
        ("p_value_format", nullable_schema(any_json_schema("P-value formatting rule (`four_decimal`, `apa`, `nejm`, `scientific`, or object)"))),
    ];
    if include_stats_extras {
        props.push(("multiplicity", nullable_schema(str_schema("Post-hoc multiplicity adjustment method"))));
        props.push((
            "repeated_measures",
            nullable_schema(str_schema("Repeated-measures matching for factorial ANOVA (`none`, `row`, `column`, `both`)")),
        ));
        props.push((
            "residual_covariance",
            nullable_schema(str_schema("REML residual covariance structure for repeated-measures ANOVA (`compound_symmetry`, `unstructured`)")),
        ));
        props.push(("df_method", nullable_schema(str_schema("REML denominator degrees-of-freedom method (`kenward_roger`, `satterthwaite`)"))));
    }
    if include_fit_extras {
        props.push(("equation_id", nullable_schema(non_empty_str_schema(
            "Built-in equation ID (e.g. `log_inhibitor_vs_response_4pl`, `log_agonist_vs_response_4pl`, `log_agonist_vs_response_3pl`, `michaelis_menten`, `one_phase_decay`, `two_phase_decay`, `exponential_growth`, `one_site_specific_binding`, `gaussian`, `bell_shaped_dose_response`) or custom label with `custom_dsl`",
        ))));
        props.push((
            "method",
            nullable_schema(any_json_schema(
                "Fitting/outlier method: `least_squares` (default), `robust` (Cauchy), `rout` (ROUT FDR 1% outlier elimination + OLS refit), or `{\"rout_outlier_elimination\": {\"fdr_q_percent\": 1.0}}`",
            )),
        ));
        props.push(("weighting", nullable_schema(str_schema("Regression weighting scheme (`none`, `1/y`, `1/y2`, `1/x`, `1/x2`)"))));
        props.push(("custom_dsl", nullable_schema(str_schema("Custom mathematical equation DSL string"))));
        props.push((
            "parameter_rules",
            nullable_schema(array_schema("Parameter constraints and shared-global rules", json!({ "type": "object" }), None, None)),
        ));
        props.push(("comparison", nullable_schema(any_json_schema("Nested/non-nested model comparison specification"))));
        props.push(("x_range_filter", nullable_schema(any_json_schema("Inclusive [min, max] X bounds for linear and nonlinear fits"))));
        props.push(("plot_x_min", nullable_schema(num_schema("Override the fitted curve's starting X", None, None))));
        props.push(("plot_x_max", nullable_schema(num_schema("Override the fitted curve's ending X", None, None))));
        props.push((
            "plot_segments",
            nullable_schema(int_schema(
                "Number of line segments in a linear fit overlay",
                Some(1),
                Some(LinearRegressionSpec::MAX_PLOT_SEGMENTS as i64),
            )),
        ));
        props.push(("fixed_point", nullable_schema(any_json_schema("Optional fixed-point constraint [x0, y0] forcing Y - y0 = slope * (X - x0)"))));
        props.push((
            "standard_addition",
            nullable_schema(bool_schema("Report standard-addition unknown concentration (-X-intercept) and negated Fieller CI")),
        ));
        props.push((
            "extend_line_to_x_intercept",
            nullable_schema(bool_schema("Extend fitted regression line plot range to include the X-intercept")),
        ));
        props.push((
            "deming_error",
            nullable_schema(any_json_schema("Model II Deming error model (`EqualError`, `StandardDeviations`, or `VarianceRatio`)")),
        ));
        props.push(("y_range_filter", nullable_schema(any_json_schema("Optional [min, max] Y inclusion filter"))));
        props.push(("identify_unstable_parameters", nullable_schema(bool_schema("Assess nonlinear parameter identifiability"))));
        props.push((
            "identifiability_mode",
            nullable_schema(any_json_schema(
                "Three-way nonlinear identifiability mode (`IdentifyUnstableParameters`, `IdentifyAmbiguousFits`, or `ReportAll`)",
            )),
        ));
        props.push((
            "show_standard_errors",
            nullable_schema(bool_schema(
                "Explicitly show or omit nonlinear parameter Std. Error column (default: omitted under profile-likelihood CIs)",
            )),
        ));
        props.push((
            "flag_r_squared_below",
            nullable_schema(num_schema("Warn when nonlinear R squared is below this threshold", Some(0.0), Some(1.0))),
        ));
        props.push((
            "comparison_question",
            nullable_schema(any_json_schema("4-question nonlinear model comparison hypothesis (`AlternativeEquation`, `ParameterInvarianceAcrossDatasets`, `SingleGlobalCurveVsSeparate`, or `ParameterEqualsTheoretical`)")),
        ));
        props.push((
            "comparison_method",
            nullable_schema(any_json_schema(
                "Model comparison test (`ExtraSumOfSquaresF`, `Aicc`, or `LikelihoodRatioTest`) for `comparison_question`",
            )),
        ));
        props.push((
            "prefer_well_defined_fit",
            nullable_schema(bool_schema(
                "Select the clean candidate immediately without F/LRT/AICc when the competing fit is Ambiguous, Unstable, or Flagged",
            )),
        ));
        props.push((
            "summary_parameter",
            nullable_schema(non_empty_str_schema("Parameter or derived scalar name to tabulate into an analyzable parameter summary tab")),
        ));
        props.push((
            "create_summary_graph",
            nullable_schema(bool_schema("When true with `summary_parameter`, create a linked secondary bar or XY GraphSheet")),
        ));
        props.push((
            "output_significant_digits",
            nullable_schema(int_schema("Displayed significant-digit precision (1..=15) on parameter and Embedded Result tables", Some(1), Some(15))),
        ));
        props.push((
            "fit_flag_rules",
            nullable_schema(any_json_schema("Automated 6-rule fit-quality flag criteria (`min_r_squared`, `min_observations`, `max_dependency`, `min_normality_p`, `min_runs_or_lof_p`, `max_rout_outliers`)")),
        ));
    }
    obj_schema(props, &[], true)
}

fn schema_transform_run() -> Value {
    let mut schema = schema_analysis_base(
        "Transform kind (`math`, `normalize`, `remove_baseline`, `transpose`, `prune_rows`, `select_and_transform`, `simulate`, `random_sample`) or typed object",
        false,
        false,
    );
    if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        props.insert(
            String::from("create_graph"),
            nullable_schema(bool_schema("When true, create a reactive GraphSheet bound to the transform output")),
        );
        props.insert(
            String::from("graph_sheet_id"),
            nullable_schema(non_empty_str_schema("Optional explicit GraphSheet ID created when `create_graph` is true")),
        );
    }
    schema
}

fn schema_stats_run() -> Value {
    schema_analysis_base(
        "Statistical test shorthand (e.g. `unpaired_t`, `welch_t`, `paired_t`, `mann_whitney`, `wilcoxon`, `one_sample_t`, `one_way_anova`, `welch_anova`, `rm_one_way_anova`, `kruskal_wallis`, `friedman`, `two_way_anova`, `three_way_anova`, `nested_t`, `nested_anova`, `descriptive`, `rout`, `grubbs`, `multiple_row_t_tests`, `contingency`, `fisher_exact`, `kaplan_meier`, `cox_ph`, `roc`, `bland_altman`, `passing_bablok`, `pvalue_stack`, `power`, `pca`, `correlation`, `clustering`) or typed AnalysisKind object",
        false,
        true,
    )
}

fn schema_stats_pvalue_stack() -> Value {
    obj_schema(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional target result sheet ID"))),
            ("source_sheet_id", non_empty_str_schema("Source data sheet ID containing raw P-value column(s)")),
            (
                "method",
                nullable_schema(str_schema(
                    "Multiplicity procedure (`bonferroni`, `holm`, `sidak`, `holm_sidak`, `benjamini_hochberg`, `two_stage_bky`, `benjamini_yekutieli`)",
                )),
            ),
            ("threshold", nullable_schema(num_schema("Significance alpha (FWER) or desired FDR Q strictly in (0, 1)", Some(0.0), Some(1.0)))),
            ("column", nullable_schema(int_schema("Optional zero-based column index override", Some(0), None))),
            (
                "selected_data_sets",
                nullable_schema(array_schema(
                    "Subset of zero-based column indices to analyze",
                    int_schema("Dataset column index", Some(0), None),
                    None,
                    None,
                )),
            ),
            ("overwrite", nullable_schema(bool_schema("Overwrite an existing result sheet with the same ID"))),
            ("title", nullable_schema(str_schema("Optional custom title override"))),
            ("p_value_format", nullable_schema(any_json_schema("P-value formatting rule (`four_decimal`, `apa`, `nejm`, `scientific`, or object)"))),
        ],
        &["source_sheet_id"],
        true,
    )
}

fn schema_fit_run() -> Value {
    schema_analysis_base(
        "Regression kind or equation shorthand (e.g. `4pl` / `log_agonist_vs_response_4pl`, `4pl_inhibitor` / `log_inhibitor_vs_response_4pl`, `3pl`, `linear`, `deming`, `michaelis_menten`, `one_phase_decay`, `one_site_specific_binding`) or typed object",
        true,
        true,
    )
}

fn schema_analysis_run() -> Value {
    schema_analysis_base("Any analysis kind shorthand or typed AnalysisKind object", true, true)
}

#[derive(Debug, Deserialize)]
struct AnalysisUpdateParams {
    sheet_id: String,
    #[serde(default)]
    kind: Option<Value>,
    #[serde(default, deserialize_with = "de_present_opt")]
    title: Option<Option<String>>,
    #[serde(default)]
    confidence_percent: Option<f64>,
    #[serde(default)]
    selected_data_sets: Option<Vec<Value>>,
    #[serde(default)]
    p_value_format: Option<Value>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn schema_analysis_update_params() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Existing result sheet ID to update")),
            ("kind", kind_schema("Optional replacement analysis kind")),
            ("title", nullable_schema(str_schema("Optional custom title override"))),
            ("confidence_percent", nullable_schema(num_schema("Confidence percentage", Some(0.0), Some(100.0)))),
            (
                "selected_data_sets",
                nullable_schema(array_schema("Subset of dataset indices", int_schema("Dataset index", Some(0), None), None, None)),
            ),
            ("p_value_format", nullable_schema(any_json_schema("P-value format override"))),
            ("params", nullable_schema(json!({ "type": "object", "description": "Optional field patch object merged into current analysis kind" }))),
        ],
        &["sheet_id"],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct RetargetDataParams {
    sheet_id: String,
    source_sheet_id: String,
    #[serde(default)]
    selected_data_sets: Option<Vec<Value>>,
    #[serde(default)]
    clear_selected_data_sets: Option<bool>,
}

fn schema_analysis_retarget_data() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Existing result sheet ID to retarget")),
            ("source_sheet_id", non_empty_str_schema("Replacement compatible data sheet ID")),
            (
                "selected_data_sets",
                nullable_schema(array_schema("Optional subset of dataset indices", int_schema("Dataset index", Some(0), None), None, None)),
            ),
            ("clear_selected_data_sets", nullable_schema(bool_schema("Clear dataset subset filter so all datasets are analyzed"))),
        ],
        &["sheet_id", "source_sheet_id"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ExportResultJsonParams {
    sheet_id: String,
    pretty: Option<bool>,
    path: Option<String>,
}

fn schema_analysis_export_json() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Result sheet ID to export")),
            ("pretty", nullable_schema(bool_schema("Pretty-print JSON output (default true)"))),
            ("path", nullable_schema(non_empty_str_schema("Optional output `.json` file path"))),
        ],
        &["sheet_id"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ExportResultCsvParams {
    sheet_id: String,
    tab_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_usize")]
    tab_index: Option<usize>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
    path: Option<String>,
}

fn schema_analysis_export_csv() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Result sheet ID to export")),
            ("tab_id", nullable_schema(non_empty_str_schema("Optional specific result tab identifier"))),
            ("tab_index", nullable_schema(int_schema("Optional zero-based result tab index", Some(0), None))),
            ("delimiter", nullable_schema(non_empty_str_schema("Single-character CSV delimiter (default `,`)"))),
            ("decimal_separator", nullable_schema(non_empty_str_schema("Single-character decimal separator (default `.`)"))),
            ("path", nullable_schema(non_empty_str_schema("Optional output `.csv` file path"))),
        ],
        &["sheet_id"],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GenerateReportParams {
    result_sheet_id: String,
    format: Option<String>,
    path: Option<String>,
}

fn schema_result_generate_report() -> Value {
    obj_schema(
        vec![
            ("result_sheet_id", non_empty_str_schema("Result sheet ID to document")),
            ("format", nullable_schema(non_empty_str_schema("Report output format (`plain`, `markdown`, or `latex`)"))),
            ("path", nullable_schema(non_empty_str_schema("Optional output file path"))),
        ],
        &["result_sheet_id"],
        false,
    )
}

// ---------------------------------------------------------------------------
// Command Runners
// ---------------------------------------------------------------------------

fn run_analyze_run(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_analyze_run();
    let _parsed: AnalyzeRunParams = parse_params(&schema, raw)?;
    let recalculated = dag::recalculate_project(&mut ctx.session.doc)?;
    Ok(CommandOutcome::mutated("analyze.run", "ran analysis pipeline", json!({ "recalculated_sheets": recalculated })))
}

fn run_transform_run(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_transform_run();
    run_analysis_with_domain(ctx, "transform.run", raw, &schema, AnalysisDomain::TransformOnly)
}

fn run_stats_run(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_stats_run();
    run_analysis_with_domain(ctx, "stats.run", raw, &schema, AnalysisDomain::StatsOnly)
}

fn run_stats_pvalue_stack(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_stats_pvalue_stack();
    let mut validated: Value = parse_params(&schema, raw)?;
    if let Value::Object(ref mut map) = validated {
        map.entry(String::from("kind")).or_insert_with(|| json!("pvalue_stack"));
    }
    let exec_schema = schema_stats_run();
    run_analysis_with_domain(ctx, "stats.pvalueStack", validated, &exec_schema, AnalysisDomain::StatsOnly)
}

fn run_fit_run(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_fit_run();
    run_analysis_with_domain(ctx, "fit.run", raw, &schema, AnalysisDomain::FitOnly)
}

fn run_analysis_add(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_analysis_run();
    run_analysis_with_domain(ctx, "analysis.add", raw, &schema, AnalysisDomain::Any)
}

fn run_analysis_run(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_analysis_run();
    run_analysis_with_domain(ctx, "analysis.run", raw, &schema, AnalysisDomain::Any)
}

fn params_to_combined_value(parsed: &RunAnalysisParams) -> Value {
    let mut map = parsed.extra.clone();
    if let Some(ref s) = parsed.sheet_id {
        map.insert(String::from("sheet_id"), Value::String(s.clone()));
    }
    if let Some(ref s) = parsed.source_sheet_id {
        map.insert(String::from("source_sheet_id"), Value::String(s.clone()));
    }
    if let Some(ref spec) = parsed.spec {
        map.insert(String::from("spec"), spec.clone());
    }
    if let Some(ref kind) = parsed.kind {
        map.insert(String::from("kind"), kind.clone());
    }
    if let Some(b) = parsed.overwrite {
        map.insert(String::from("overwrite"), Value::Bool(b));
    }
    if let Some(ref t_opt) = parsed.title {
        map.insert(String::from("title"), t_opt.as_ref().map_or(Value::Null, |t| Value::String(t.clone())));
    }
    if let Some(c) = parsed.confidence_percent {
        map.insert(String::from("confidence_percent"), json!(c));
    }
    if let Some(ref sel) = parsed.selected_data_sets {
        map.insert(String::from("selected_data_sets"), Value::Array(sel.clone()));
    }
    if let Some(ref pv) = parsed.p_value_format {
        map.insert(String::from("p_value_format"), pv.clone());
    }
    Value::Object(map)
}

fn has_creation_fields(parsed: &RunAnalysisParams) -> bool {
    parsed.spec.is_some()
        || parsed.source_sheet_id.is_some()
        || parsed.kind.is_some()
        || parsed.extra.contains_key("equation_id")
        || parsed.extra.contains_key("custom_dsl")
}

fn run_analysis_with_domain(
    ctx: &mut Ctx<'_>,
    command_id: &str,
    raw: Value,
    schema: &Value,
    domain: AnalysisDomain,
) -> Result<CommandOutcome, EngineError> {
    let parsed: RunAnalysisParams = parse_params(schema, raw)?;
    let doc = &mut ctx.session.doc;

    if command_id == "analysis.run" && !has_creation_fields(&parsed) {
        if let Some(pct) = parsed.confidence_percent {
            validate_confidence_percent(pct)?;
        }
        reject_extra_keys_without_creation(&parsed)?;
        return run_existing_or_project_analysis(doc, command_id, parsed.sheet_id.as_deref());
    }

    let combined = params_to_combined_value(&parsed);
    let spec = build_analysis_spec(doc, &parsed, &combined, domain)?;
    let sheet_id = spec.id.clone();
    let overwrite = parsed.overwrite.unwrap_or(false);
    let wants_create_graph = command_id == "transform.run" && parsed.create_graph.unwrap_or(false);
    let explicit_graph_id = if wants_create_graph {
        let candidate_graph_id = parsed.graph_sheet_id.as_deref().map(parse_sheet_id).transpose()?;
        if let Some(ref gid) = candidate_graph_id
            && (gid == &sheet_id || sheet_id_exists(doc, gid))
        {
            return Err(DocError::DuplicateSheetId { sheet_id: gid.clone() }.into());
        }
        let result_added = usize::from(doc.sheet_section(&sheet_id).is_none());
        let needed_total = doc.total_sheet_count().saturating_add(result_added).saturating_add(1);
        if needed_total > MAX_SHEETS_PER_PROJECT {
            return Err(DocError::TooManySheets { count: needed_total, max: MAX_SHEETS_PER_PROJECT }.into());
        }
        candidate_graph_id
    } else {
        None
    };

    if doc.sheet_section(&sheet_id).is_some() {
        if overwrite && let Some(existing) = doc.result_sheet_mut(&sheet_id) {
            existing.analysis = spec;
        } else {
            return Err(EngineError::with_code(
                ErrorCode::AlreadyExists,
                format!("sheet `{sheet_id}` already exists; pass `overwrite: true` to replace an existing result sheet"),
            )
            .with_remediation("Choose a unique `sheet_id` or pass `overwrite: true`.")
            .with_details(json!({ "sheet_id": sheet_id.as_str() })));
        }
    } else {
        doc.add_result_sheet(ResultSheet::new(spec)?)?;
    }

    let recalculated = dag::recalculate_dirty_with_report(doc, std::slice::from_ref(&sheet_id))?.total_updated();
    let res = require_result_sheet(doc, &sheet_id)?;
    ensure_result_succeeded(res)?;
    let mut payload = result_summary_payload(res, recalculated);
    let eff_title = res.analysis.effective_title().to_owned();

    let summary_graph_param = match &res.analysis.kind {
        AnalysisKind::NonlinearRegression(nl) if nl.create_summary_graph && res.analyzable_data.is_some() => nl.summary_parameter.clone(),
        _ => None,
    };

    if wants_create_graph {
        let graph_id = match explicit_graph_id {
            Some(gid) => gid,
            None => next_unique_sheet_id(doc, "graph")?,
        };
        let (ds, _) = super::graph::resolve_graph_create_source_sheet(doc, &sheet_id)?;
        let mut graph = GraphSheet::new_uninitialized_for_data(graph_id.clone(), &ds)?;
        graph.title.clone_from(&eff_title);
        graph.spec.title.clone_from(&eff_title);
        graph.initialized = true;
        graph.spec.primary_source_sheet_id = sheet_id.clone();
        for s in &mut graph.spec.series {
            s.source_sheet_id = sheet_id.clone();
        }
        super::graph::validate_graph_spec_in_doc(doc, &graph_id, &graph.spec)?;
        doc.add_graph_sheet(graph)?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(String::from("graph_sheet_id"), Value::String(graph_id.as_str().to_owned()));
        }
    }

    if let Some(summary_param) = summary_graph_param {
        let graph_id = next_unique_sheet_id(doc, "graph")?;
        let (ds, _) = super::graph::resolve_graph_create_source_sheet(doc, &sheet_id)?;
        let mut graph = GraphSheet::new_uninitialized_for_data(graph_id.clone(), &ds)?;
        let summary_title = format!("{eff_title} — {summary_param} summary");
        graph.title.clone_from(&summary_title);
        graph.spec.title = summary_title;
        graph.initialized = true;
        graph.spec.primary_source_sheet_id = sheet_id.clone();
        for s in &mut graph.spec.series {
            s.source_sheet_id = sheet_id.clone();
        }
        super::graph::validate_graph_spec_in_doc(doc, &graph_id, &graph.spec)?;
        doc.add_graph_sheet(graph)?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(String::from("summary_graph_sheet_id"), Value::String(graph_id.as_str().to_owned()));
        }
    }

    Ok(CommandOutcome::mutated(command_id, format!("ran {eff_title}"), payload))
}

fn run_existing_or_project_analysis(doc: &mut ProjectDocument, command_id: &str, raw_id: Option<&str>) -> Result<CommandOutcome, EngineError> {
    if let Some(id_str) = raw_id {
        let id = parse_sheet_id(id_str)?;
        if doc.result_sheet(&id).is_none() {
            return Err(EngineError::with_code(
                ErrorCode::NotFound,
                format!("result sheet `{id}` not found; supply `source_sheet_id` and `kind` to create a new analysis"),
            )
            .with_remediation("Supply `source_sheet_id` and `kind` to create a new result sheet, or verify the existing `sheet_id`.")
            .with_details(json!({ "sheet_id": id.as_str() })));
        }
        let recalculated = dag::recalculate_dirty_with_report(doc, std::slice::from_ref(&id))?.total_updated();
        let res = require_result_sheet(doc, &id)?;
        ensure_result_succeeded(res)?;
        return Ok(CommandOutcome::mutated(command_id, "recalculated result sheet", result_summary_payload(res, recalculated)));
    }
    let recalculated = dag::recalculate_project(doc)?;
    Ok(CommandOutcome::mutated(command_id, "recalculated analyses", json!({ "recalculated_sheets": recalculated })))
}

fn run_analysis_update_params(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_analysis_update_params();
    let parsed: AnalysisUpdateParams = parse_params(&schema, raw)?;
    let doc = &mut ctx.session.doc;
    let id = parse_sheet_id(&parsed.sheet_id)?;
    let mut updated = require_result_sheet(doc, &id)?.analysis.clone();
    let source_kind = require_analyzable_data_sheet(doc, &updated.source_sheet_id)?.kind();

    let combined = update_params_to_value(&parsed);
    let explicit_empty_selection = parsed.selected_data_sets.as_ref().is_some_and(Vec::is_empty);
    apply_metadata_updates(
        &mut updated,
        parsed.title.as_ref().map(Option::as_deref),
        parsed.confidence_percent,
        parsed.selected_data_sets.as_deref(),
        parsed.p_value_format.as_ref(),
    )?;

    let cleaned_patch_map = extract_clean_patch_map(&mut updated, parsed.params.as_ref())?;
    let cleaned_patch_val = cleaned_patch_map.as_ref().map(|m| Value::Object(m.clone()));

    let kind_replaced = parsed.kind.is_some();
    if let Some(ref kind_val) = parsed.kind {
        let mut new_kind = if kind_val.is_object() {
            deserialize_strict(kind_val, "/kind")?
        } else {
            let name = kind_val
                .as_str()
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`kind` must be string or object"))?
                .trim()
                .to_ascii_lowercase();
            resolve_kind_shorthand(&name, &combined)?
        };
        apply_kind_patch_and_overrides(&mut new_kind, &combined)?;
        if let Some(ref patch_val) = cleaned_patch_val {
            apply_kind_patch_and_overrides(&mut new_kind, patch_val)?;
        }
        updated.update_kind(new_kind);
    } else {
        let mut kind_copy = updated.kind.clone();
        apply_kind_patch_and_overrides(&mut kind_copy, &combined)?;
        if let Some(ref patch_val) = cleaned_patch_val {
            apply_kind_patch_and_overrides(&mut kind_copy, patch_val)?;
        }
        updated.update_kind(kind_copy);
    }

    validate_map_keys_for_kind(&updated.kind, &parsed.extra, KeyValidationMode::UpdateParamsTopLevel, "parameter")?;
    if let Some(ref patch_map) = cleaned_patch_map {
        validate_map_keys_for_kind(&updated.kind, patch_map, KeyValidationMode::UpdateParamsNested, "parameter in `params`")?;
    }
    if matches!(updated.kind, AnalysisKind::LinearRegression(_))
        && let Some(rules_val) = parsed.extra.get("parameter_rules")
    {
        let _ = parse_parameter_rules(rules_val)?;
    }
    if matches!(updated.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. })) {
        let _ = opt_bool_checked(&combined, "correlation_all_pairs")?;
        let _ = opt_bool_checked(&combined, "correlation_heatmap")?;
    }

    if !updated.kind.is_compatible_with(source_kind) {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("updated analysis kind is incompatible with table type `{}`", source_kind.label()),
        )
        .with_remediation("Choose an analysis kind compatible with the source table archetype."));
    }
    let source_ds = require_analyzable_data_sheet(doc, &updated.source_sheet_id)?;
    validate_selected_data_sets_for_sheet(source_ds, &updated)?;
    if kind_replaced {
        persist_default_mv_columns(source_ds, &mut updated, explicit_empty_selection);
    }
    resolve_xy_bland_altman_mode(source_ds, &mut updated);
    validate_analysis_spec_semantics(&updated)?;
    validate_xy_analysis_spec(source_ds, &updated)?;
    validate_multiple_row_tests_spec(source_ds, &updated)?;

    if let Some(res) = doc.result_sheet_mut(&id) {
        res.analysis = updated;
    }
    let recalculated = dag::recalculate_dirty_with_report(doc, std::slice::from_ref(&id))?.total_updated();
    let res = require_result_sheet(doc, &id)?;
    ensure_result_succeeded(res)?;

    Ok(CommandOutcome::mutated("analysis.updateParams", format!("updated analysis parameters on `{id}`"), result_summary_payload(res, recalculated)))
}

fn update_params_to_value(parsed: &AnalysisUpdateParams) -> Value {
    let mut map = parsed.extra.clone();
    if let Some(ref t_opt) = parsed.title {
        map.insert(String::from("title"), t_opt.as_ref().map_or(Value::Null, |t| Value::String(t.clone())));
    }
    if let Some(c) = parsed.confidence_percent {
        map.insert(String::from("confidence_percent"), json!(c));
    }
    if let Some(ref sel) = parsed.selected_data_sets {
        map.insert(String::from("selected_data_sets"), Value::Array(sel.clone()));
    }
    if let Some(ref pv) = parsed.p_value_format {
        map.insert(String::from("p_value_format"), pv.clone());
    }
    Value::Object(map)
}

fn run_analysis_retarget_data(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_analysis_retarget_data();
    let parsed: RetargetDataParams = parse_params(&schema, raw)?;
    let doc = &mut ctx.session.doc;
    let id = parse_sheet_id(&parsed.sheet_id)?;
    let new_source = parse_sheet_id(&parsed.source_sheet_id)?;
    let new_table_kind = require_analyzable_data_sheet(doc, &new_source)?.kind();
    let mut updated = require_result_sheet(doc, &id)?.analysis.clone();
    let source_changed = updated.source_sheet_id != new_source;

    let next_selected = if parsed.clear_selected_data_sets == Some(true) {
        Vec::new()
    } else if let Some(ref vals) = parsed.selected_data_sets {
        let mut out = Vec::with_capacity(vals.len());
        for v in vals {
            out.push(parse_usize_json_value(v, "selected_data_sets")?);
        }
        out
    } else {
        updated.selected_data_sets.clone()
    };

    updated.retarget_source(new_source.clone(), new_table_kind, next_selected)?;
    if source_changed && let AnalysisKind::BlandAltman(ref mut ba) = updated.kind {
        ba.xy_compare_x_vs_y = None;
    }
    let new_source_ds = require_analyzable_data_sheet(doc, &new_source)?;
    validate_selected_data_sets_for_sheet(new_source_ds, &updated)?;
    resolve_xy_bland_altman_mode(new_source_ds, &mut updated);
    validate_analysis_spec_semantics(&updated)?;
    validate_xy_analysis_spec(new_source_ds, &updated)?;
    validate_multiple_row_tests_spec(new_source_ds, &updated)?;
    if let Some(res) = doc.result_sheet_mut(&id) {
        res.analysis = updated;
    }
    let recalculated = dag::recalculate_dirty_with_report(doc, std::slice::from_ref(&id))?.total_updated();
    let res = require_result_sheet(doc, &id)?;
    ensure_result_succeeded(res)?;

    Ok(CommandOutcome::mutated(
        "analysis.retargetData",
        format!("retargeted analysis `{id}` to `{new_source}`"),
        result_summary_payload(res, recalculated),
    ))
}

fn write_export_outcome(
    policy: &ExternalPathPolicy,
    command_id: &str,
    message: String,
    path: Option<&str>,
    text: &str,
    payload: Value,
) -> Result<CommandOutcome, EngineError> {
    if let Some(p) = path {
        write_output_file(policy, p, text.as_bytes())?;
    }
    Ok(CommandOutcome::read_only(command_id, message, payload))
}

fn run_analysis_export_json(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_analysis_export_json();
    let params: ExportResultJsonParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let res = require_result_sheet(ctx.session.document(), &id)?;
    let pretty = params.pretty.unwrap_or(true);
    let json_text = export_result_sheet_json(res, pretty)?;
    let payload = json!({ "sheet_id": id.as_str(), "byte_len": json_text.len(), "path": params.path, "json": json_text });
    write_export_outcome(
        &ctx.session.path_policy,
        "analysis.exportJson",
        format!("exported analysis `{id}` to JSON"),
        params.path.as_deref(),
        &json_text,
        payload,
    )
}

fn run_analysis_export_csv(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_analysis_export_csv();
    let params: ExportResultCsvParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let res = require_result_sheet(ctx.session.document(), &id)?;

    let mut opts = CsvOptions::csv();
    if let Some(ref delim) = params.delimiter {
        opts.delimiter = single_char(delim, "delimiter")?;
    }
    if let Some(ref dec) = params.decimal_separator {
        opts.decimal_separator = single_char(dec, "decimal_separator")?;
    }

    let csv = if let Some(ref tab_id) = params.tab_id {
        let tab = res
            .tabs
            .iter()
            .find(|t| t.id == *tab_id)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result tab `{tab_id}` not found")))?;
        try_export_result_tab_csv(res, tab, &opts)?
    } else if let Some(tab_index) = params.tab_index {
        let tab = res
            .tabs
            .get(tab_index)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result tab index {tab_index} out of bounds")))?;
        try_export_result_tab_csv(res, tab, &opts)?
    } else {
        try_export_result_sheet_tidy_csv(res, &opts)?
    };

    let payload = json!({ "sheet_id": id.as_str(), "byte_len": csv.len(), "path": params.path, "csv": csv });
    write_export_outcome(
        &ctx.session.path_policy,
        "analysis.exportCsv",
        format!("exported analysis `{id}` to CSV"),
        params.path.as_deref(),
        &csv,
        payload,
    )
}

fn run_result_generate_report(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_result_generate_report();
    let params: GenerateReportParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.result_sheet_id)?;
    let format = match params.format.as_deref() {
        None => ReportFormat::Markdown,
        Some(raw_fmt) => ReportFormat::from_name(raw_fmt).ok_or_else(|| {
            bad_input(format!("unknown report format `{raw_fmt}`"))
                .with_remediation("Use `plain`, `markdown`, or `latex`.")
                .with_details(json!({ "pointer": "/format", "format": raw_fmt }))
        })?,
    };
    let report = generate_methods_report(ctx.session.document(), &id, format)?;
    let payload = json!({
        "sheet_id": id.as_str(),
        "format": format.as_str(),
        "path": params.path,
        "report": report,
    });
    write_export_outcome(
        &ctx.session.path_policy,
        "result.generateReport",
        format!("generated {} methods report for `{id}`", format.as_str()),
        params.path.as_deref(),
        &report,
        payload,
    )
}

fn ensure_result_succeeded(res: &ResultSheet) -> Result<(), EngineError> {
    match &res.status {
        ResultStatus::Failed { code, message } => {
            let mapped = if *code == ErrorCode::SolverFailed && message.contains("paired_selected_columns") { ErrorCode::BadInput } else { *code };
            Err(EngineError::with_code(mapped, format!("analysis `{}` failed ({mapped}): {message}", res.id))
                .with_details(json!({ "sheet_id": res.id.as_str(), "error_code": mapped })))
        }
        ResultStatus::Dirty | ResultStatus::UpToDate => Ok(()),
    }
}

fn result_summary_payload(res: &ResultSheet, recalculated: usize) -> Value {
    let scalars: Map<String, Value> =
        res.scalars.iter().map(|s| (s.key.clone(), json!({ "data_set_index": s.data_set_index, "value": s.value, "units": s.units }))).collect();
    let mut payload = json!({
        "sheet_id": res.id.as_str(),
        "source_sheet_id": res.analysis.source_sheet_id.as_str(),
        "title": res.analysis.effective_title(),
        "status": res.status,
        "recalculated_sheets": recalculated,
        "tabs_count": res.tabs.len(),
        "scalars_count": res.scalars.len(),
        "scalars": scalars,
        "pairwise_comparisons_count": res.pairwise_comparisons.len(),
        "fitted_curves_count": res.fitted_curves.len(),
    });
    if matches!(&res.analysis.kind, AnalysisKind::NonlinearRegression(nl) if nl.method == FitMethod::RobustCauchy)
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert(String::from("inference_availability"), Value::String(String::from("NotApplicableInRobustMode")));
    }
    payload
}

// ---------------------------------------------------------------------------
// AnalysisSpec Builder & Shorthand Resolution
// ---------------------------------------------------------------------------

fn try_raw_analysis_spec(
    doc: &ProjectDocument,
    parsed: &RunAnalysisParams,
    _combined: &Value,
    domain: AnalysisDomain,
) -> Result<Option<AnalysisSpec>, EngineError> {
    if let Some(ref spec_val) = parsed.spec {
        if parsed.kind.is_some() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "cannot specify both `spec` and `kind`"));
        }
        if !parsed.extra.is_empty() {
            let extra_keys: Vec<&str> = parsed.extra.keys().map(String::as_str).collect();
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("unrecognized parameter(s) {} alongside `spec`", extra_keys.join(", "))));
        }
        let mut spec: AnalysisSpec = deserialize_strict(spec_val, "/spec")?;
        validate_confidence_percent(spec.confidence_level.percent())?;
        apply_metadata_updates(
            &mut spec,
            parsed.title.as_ref().map(Option::as_deref),
            parsed.confidence_percent,
            parsed.selected_data_sets.as_deref(),
            parsed.p_value_format.as_ref(),
        )?;
        let table_kind = require_analyzable_data_sheet(doc, &spec.source_sheet_id)?.kind();
        validate_domain_and_compat(&spec.kind, domain, table_kind)?;
        return Ok(Some(spec));
    }

    if let (Some(id_str), Some(src_str), Some(kind_val)) = (parsed.sheet_id.as_deref(), parsed.source_sheet_id.as_deref(), parsed.kind.as_ref())
        && kind_val.is_object()
        && parsed.extra.contains_key("auto_title")
    {
        let mut obj = parsed.extra.clone();
        obj.insert(String::from("id"), Value::String(id_str.to_owned()));
        obj.insert(String::from("source_sheet_id"), Value::String(src_str.to_owned()));
        obj.insert(String::from("kind"), kind_val.clone());
        if let Some(ref t_opt) = parsed.title {
            obj.insert(String::from("custom_title"), t_opt.as_ref().map_or(Value::Null, |t| Value::String(t.clone())));
        }
        if let Some(c) = parsed.confidence_percent {
            obj.insert(String::from("confidence_level"), json!(c));
        }
        if let Some(ref sel) = parsed.selected_data_sets {
            obj.insert(String::from("selected_data_sets"), Value::Array(sel.clone()));
        }
        if let Some(ref pv) = parsed.p_value_format {
            obj.insert(String::from("p_value_format"), pv.clone());
        }
        let spec: AnalysisSpec = deserialize_strict(&Value::Object(obj), "")?;
        validate_confidence_percent(spec.confidence_level.percent())?;
        let table_kind = require_analyzable_data_sheet(doc, &spec.source_sheet_id)?.kind();
        validate_domain_and_compat(&spec.kind, domain, table_kind)?;
        return Ok(Some(spec));
    }
    Ok(None)
}

fn build_analysis_spec(
    doc: &ProjectDocument,
    parsed: &RunAnalysisParams,
    combined: &Value,
    domain: AnalysisDomain,
) -> Result<AnalysisSpec, EngineError> {
    let mut spec = if let Some(spec) = try_raw_analysis_spec(doc, parsed, combined, domain)? {
        spec
    } else {
        let raw_source = parsed.source_sheet_id.as_deref().ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "missing required parameter `source_sheet_id`")
                .with_remediation("Supply `source_sheet_id` referencing an existing data sheet.")
                .with_details(json!({ "pointer": "/source_sheet_id" }))
        })?;
        let source_sheet_id = parse_sheet_id(raw_source)?;
        let table_kind = require_analyzable_data_sheet(doc, &source_sheet_id)?.kind();
        let id = match parsed.sheet_id.as_deref() {
            Some(raw) => parse_sheet_id(raw)?,
            None => next_unique_sheet_id(doc, domain_prefix(domain))?,
        };

        let kind = parse_analysis_kind(parsed.kind.as_ref(), combined, &parsed.extra, domain, table_kind)?;
        validate_domain_and_compat(&kind, domain, table_kind)?;

        let mut spec = AnalysisSpec::new(id, source_sheet_id, kind)?;
        apply_metadata_updates(
            &mut spec,
            parsed.title.as_ref().map(Option::as_deref),
            parsed.confidence_percent,
            parsed.selected_data_sets.as_deref(),
            parsed.p_value_format.as_ref(),
        )?;
        spec
    };

    let explicit_empty_selection = parsed.selected_data_sets.as_ref().is_some_and(Vec::is_empty);
    finalize_built_analysis_spec(doc, &mut spec, explicit_empty_selection)?;
    Ok(spec)
}

fn finalize_built_analysis_spec(doc: &ProjectDocument, spec: &mut AnalysisSpec, explicit_empty_selection: bool) -> Result<(), EngineError> {
    validate_confidence_percent(spec.confidence_level.percent())?;
    let source_ds = require_analyzable_data_sheet(doc, &spec.source_sheet_id)?;
    validate_selected_data_sets_for_sheet(source_ds, spec)?;
    persist_default_mv_columns(source_ds, spec, explicit_empty_selection);
    resolve_xy_bland_altman_mode(source_ds, spec);
    validate_analysis_spec_semantics(spec)?;
    validate_xy_analysis_spec(source_ds, spec)?;
    validate_multiple_row_tests_spec(source_ds, spec)?;
    Ok(())
}

fn default_multivariate_numeric_columns(mv: &curvebook_doc::MultipleVariablesTable) -> Vec<usize> {
    let continuous: Vec<usize> = mv
        .variables
        .iter()
        .enumerate()
        .filter(|(_, v)| v.role == VariableRole::Continuous && v.cells.iter().any(|c| c.active_number().is_some_and(f64::is_finite)))
        .map(|(i, _)| i)
        .collect();
    if continuous.len() >= 2 {
        return continuous;
    }
    let numeric: Vec<usize> = mv
        .variables
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            !matches!(v.role, VariableRole::Categorical | VariableRole::Label)
                && v.cells.iter().any(|c| c.active_number().is_some_and(f64::is_finite))
        })
        .map(|(i, _)| i)
        .collect();
    if numeric.len() >= 2 {
        return numeric;
    }
    Vec::new()
}

fn domain_prefix(domain: AnalysisDomain) -> &'static str {
    match domain {
        AnalysisDomain::TransformOnly => AnalysisDomainKind::Transform.default_sheet_prefix(),
        AnalysisDomain::StatsOnly => AnalysisDomainKind::Stats.default_sheet_prefix(),
        AnalysisDomain::FitOnly => AnalysisDomainKind::Fit.default_sheet_prefix(),
        AnalysisDomain::Any => "res",
    }
}

fn validate_domain_and_compat(kind: &AnalysisKind, domain: AnalysisDomain, table_kind: TableKind) -> Result<(), EngineError> {
    let actual_domain = kind.descriptor().domain;
    let domain_ok = match domain {
        AnalysisDomain::Any => true,
        AnalysisDomain::TransformOnly => actual_domain == AnalysisDomainKind::Transform,
        AnalysisDomain::FitOnly => actual_domain == AnalysisDomainKind::Fit,
        AnalysisDomain::StatsOnly => actual_domain == AnalysisDomainKind::Stats,
    };
    if !domain_ok {
        return Err(EngineError::with_code(ErrorCode::BadInput, "analysis kind does not match the requested command family"));
    }
    if !kind.is_compatible_with(table_kind) {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("analysis `{}` is incompatible with `{}` tables", kind.derive_auto_title(), table_kind.label()),
        )
        .with_remediation("Select a compatible data table or analysis kind."));
    }
    Ok(())
}

fn validate_xy_analysis_spec(ds: &DataSheet, spec: &AnalysisSpec) -> Result<(), EngineError> {
    let TableArchetype::Xy(xy) = &ds.archetype else {
        return Ok(());
    };
    if xy.data_sets.is_empty() {
        return Ok(());
    }
    match &spec.kind {
        AnalysisKind::BlandAltman(ba) => validate_xy_bland_altman_params(xy, ba),
        AnalysisKind::GeneralizedLinearModel(glm) => validate_xy_glm_params(xy, &spec.selected_data_sets, glm),
        _ => Ok(()),
    }
}

fn validate_xy_bland_altman_params(xy: &XyTable, ba: &BlandAltmanSpec) -> Result<(), EngineError> {
    if ba.column_a == ba.column_b {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("Bland-Altman requires two distinct columns (`column_a` = {}, `column_b` = {})", ba.column_a, ba.column_b),
        ));
    }
    let compare_x_vs_y = ba.xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1);
    if compare_x_vs_y {
        if ba.column_a != 0 || ba.column_b != 1 {
            let missing = if ba.column_a != 0 { ba.column_a } else { ba.column_b };
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("X-vs-Y XY Bland-Altman compares X (0) vs Y0 (1); invalid column index {missing}"),
            ));
        }
        return Ok(());
    }
    if ba.column_a >= xy.data_sets.len() || ba.column_b >= xy.data_sets.len() {
        let missing = if ba.column_a >= xy.data_sets.len() { ba.column_a } else { ba.column_b };
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("Bland-Altman column index {missing} out of range for XY table with {} Y data sets", xy.data_sets.len()),
        ));
    }
    Ok(())
}

fn validate_confidence_percent(pct: f64) -> Result<(), EngineError> {
    if !pct.is_finite() || pct <= 1.0 || pct > 99.99 {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!(
                "`confidence_percent` ({pct}) must be a percentage strictly greater than 1.0 in (1.0, 99.99], such as 95.0 for 95% confidence (not a decimal fraction like 0.95)"
            ),
        )
        .with_remediation("Supply `confidence_percent` on a percentage scale (for example 90.0, 95.0, or 99.0), not as a fraction in [0, 1].")
        .with_details(json!({ "pointer": "/confidence_percent", "confidence_percent": pct })));
    }
    Ok(())
}

fn reject_extra_keys_without_creation(parsed: &RunAnalysisParams) -> Result<(), EngineError> {
    if !parsed.extra.is_empty() {
        let keys: Vec<&str> = parsed.extra.keys().map(String::as_str).collect();
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unrecognized parameter(s) {} for `analysis.run` without `source_sheet_id` and `kind`", keys.join(", ")),
        )
        .with_remediation("Supply `source_sheet_id` and `kind` to create an analysis, or pass only `sheet_id` to recalculate."));
    }
    if parsed.overwrite.is_some()
        || parsed.title.is_some()
        || parsed.confidence_percent.is_some()
        || parsed.selected_data_sets.is_some()
        || parsed.p_value_format.is_some()
    {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "analysis configuration options require `source_sheet_id` and `kind` (use `analysis.updateParams` to modify an existing sheet)",
        ));
    }
    Ok(())
}

fn extract_clean_patch_map(spec: &mut AnalysisSpec, params_val: Option<&Value>) -> Result<Option<Map<String, Value>>, EngineError> {
    let Some(val) = params_val else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let Value::Object(map) = val else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`params` must be a JSON object or null"));
    };
    let mut cleaned = map.clone();
    if let Some(conf_val) = cleaned.remove("confidence_percent") {
        let pct = conf_val
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`confidence_percent` must be a finite percentage in (1.0, 99.99]"))?;
        apply_metadata_updates(spec, None, Some(pct), None, None)?;
    }
    if let Some(title_val) = cleaned.remove("title") {
        match title_val {
            Value::Null => apply_metadata_updates(spec, Some(None), None, None, None)?,
            Value::String(ref s) => apply_metadata_updates(spec, Some(Some(s.as_str())), None, None, None)?,
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, "`title` must be a string or null")),
        }
    }
    if let Some(sel_val) = cleaned.remove("selected_data_sets") {
        let Value::Array(arr) = sel_val else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`selected_data_sets` must be an array of dataset indices"));
        };
        apply_metadata_updates(spec, None, None, Some(&arr), None)?;
    }
    if let Some(pv_val) = cleaned.remove("p_value_format") {
        apply_metadata_updates(spec, None, None, None, Some(&pv_val))?;
    }
    Ok(Some(cleaned))
}

fn resolve_xy_bland_altman_mode(source_ds: &DataSheet, spec: &mut AnalysisSpec) {
    if let AnalysisKind::BlandAltman(ref mut ba) = spec.kind {
        match &source_ds.archetype {
            TableArchetype::Xy(xy) => {
                if ba.xy_compare_x_vs_y.is_none() {
                    ba.xy_compare_x_vs_y = Some(xy.data_sets.len() == 1);
                }
            }
            _ => {
                ba.xy_compare_x_vs_y = None;
            }
        }
    }
}

fn validate_selected_data_sets_for_sheet(source_ds: &DataSheet, spec: &AnalysisSpec) -> Result<(), EngineError> {
    let mut seen = HashSet::with_capacity(spec.selected_data_sets.len());
    for &idx in &spec.selected_data_sets {
        if !seen.insert(idx) {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("duplicate index {idx} in `selected_data_sets`")));
        }
    }
    if spec.selected_data_sets.is_empty() {
        return Ok(());
    }
    if let AnalysisKind::NonlinearRegression(nl) = &spec.kind
        && let TableArchetype::MultipleVariables(mv) = &source_ds.archetype
        && !nl.mv_x_columns.is_empty()
        && !nl.effective_mv_y_columns().is_empty()
    {
        return validate_nl_mv_group_selection(source_ds, mv, nl, &spec.selected_data_sets);
    }
    let col_count = source_ds.archetype.column_count();
    for &idx in &spec.selected_data_sets {
        if idx >= col_count {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("selected dataset index {idx} out of range for `{}` table with {col_count} columns", source_ds.archetype.kind().label()),
            ));
        }
    }
    Ok(())
}

fn validate_nl_mv_group_selection(
    source_ds: &DataSheet,
    mv: &MultipleVariablesTable,
    nl: &NonlinearFitSpec,
    selected: &[usize],
) -> Result<(), EngineError> {
    let n_vars = mv.variables.len();
    for &gc in &nl.mv_group_columns {
        if gc >= n_vars {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`mv_group_columns` index {gc} out of range")));
        }
    }
    for &xc in &nl.mv_x_columns {
        if xc >= n_vars {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`mv_x_columns` index {xc} out of range")));
        }
    }
    let effective_y_cols = nl.effective_mv_y_columns();
    for &yc in &effective_y_cols {
        if yc >= n_vars {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`mv_y_columns` index {yc} out of range")));
        }
    }
    let has_cat_fallback = mv
        .variables
        .iter()
        .enumerate()
        .any(|(idx, v)| !effective_y_cols.contains(&idx) && !nl.mv_x_columns.contains(&idx) && v.role == VariableRole::Categorical);
    if nl.mv_group_columns.is_empty() && !has_cat_fallback && effective_y_cols.len() <= 1 {
        for &idx in selected {
            if idx >= n_vars {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("selected dataset index {idx} out of range for Multivariate table with {n_vars} columns"),
                ));
            }
        }
        return Ok(());
    }
    match curvebook_fit::extract_datasets_from_sheet_multi(source_ds, &[], &nl.mv_group_columns, &nl.mv_x_columns, &effective_y_cols) {
        Ok(groups) => {
            for &idx in selected {
                if idx >= groups.len() {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("selected group index {idx} out of range for Multivariate fit with {} groups", groups.len()),
                    ));
                }
            }
            Ok(())
        }
        Err(curvebook_fit::FitError::InvalidParameter { name, reason }) => {
            Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `{name}`: {reason}")))
        }
        Err(_) => {
            for &idx in selected {
                if idx >= n_vars {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("selected dataset index {idx} out of range for Multivariate table with {n_vars} columns"),
                    ));
                }
            }
            Ok(())
        }
    }
}

fn validate_multiple_row_tests_spec(source_ds: &DataSheet, spec: &AnalysisSpec) -> Result<(), EngineError> {
    let AnalysisKind::MultipleRowTTests(mr) = &spec.kind else {
        return Ok(());
    };
    match &source_ds.archetype {
        TableArchetype::Grouped(g) => validate_grouped_or_xy_row_tests("Grouped", g.data_sets.len(), &spec.selected_data_sets, mr.paired),
        TableArchetype::Xy(xy) => validate_grouped_or_xy_row_tests("XY", xy.data_sets.len(), &spec.selected_data_sets, mr.paired),
        TableArchetype::Column(col) => {
            let count = if spec.selected_data_sets.is_empty() { col.data_sets.len() } else { spec.selected_data_sets.len() };
            if mr.paired && !count.is_multiple_of(2) {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("paired row-wise tests on Column tables require an even number of selected columns (got {count})"),
                ));
            }
            Ok(())
        }
        TableArchetype::MultipleVariables(mv) => {
            let count = if spec.selected_data_sets.is_empty() {
                mv.variables.iter().filter(|v| !matches!(v.role, VariableRole::Categorical | VariableRole::Label)).count()
            } else {
                spec.selected_data_sets.iter().filter(|&&i| mv.variables.get(i).is_some_and(|v| v.role != VariableRole::Categorical)).count()
            };
            if mr.paired && !count.is_multiple_of(2) {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("paired row-wise tests on Multiple Variables tables require an even number of numeric columns (got {count})"),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_grouped_or_xy_row_tests(label: &str, total: usize, selected: &[usize], paired: bool) -> Result<(), EngineError> {
    let count = if selected.is_empty() { total } else { selected.len() };
    if paired && count != 2 {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("paired row-wise tests on {label} tables require exactly 2 data sets (`selected_data_sets`), got {count}"),
        ));
    }
    if !selected.is_empty() && count != 2 {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("explicit `selected_data_sets` for multiple row tests on {label} tables must select 2 data sets, got {count}"),
        ));
    }
    Ok(())
}

fn validate_analysis_spec_semantics(spec: &AnalysisSpec) -> Result<(), EngineError> {
    match &spec.kind {
        AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column: Some(outcome_col), .. }) => {
            if spec.selected_data_sets.contains(outcome_col) {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("PCR `pcr_outcome_column` ({outcome_col}) cannot overlap `selected_data_sets` predictors (target leakage)"),
                ));
            }
        }
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent }) => {
            if !fdr_q_percent.is_finite() || *fdr_q_percent <= 0.0 || *fdr_q_percent > 100.0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("ROUT `fdr_q_percent` ({fdr_q_percent}) must lie in (0, 100]")));
            }
        }
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, .. }) => {
            if !alpha.is_finite() || *alpha <= 0.0 || *alpha >= 1.0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("Grubbs `alpha` ({alpha}) must lie strictly in (0, 1)")));
            }
        }
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m, max_iters, .. }) => {
            if !(2..=50).contains(imputations_m) {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("multiple imputation `imputations_m` ({imputations_m}) must be in 2..=50"),
                ));
            }
            if !(1..=100).contains(max_iters) {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("multiple imputation `max_iters` ({max_iters}) must be in 1..=100")));
            }
        }
        AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, k_sweep_max, .. }) => {
            if let Some(k) = k_means_k
                && *k < 2
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`k_means_k` must be >= 2"));
            }
            if let Some(k_max) = k_sweep_max
                && *k_max < 2
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`k_sweep_max` must be >= 2"));
            }
        }
        AnalysisKind::NonlinearRegression(nl) => validate_nonlinear_spec_semantics(nl)?,
        AnalysisKind::SimpleLogisticRegression(sl) => {
            if !sl.classification_cutoff.is_finite() || sl.classification_cutoff <= 0.0 || sl.classification_cutoff >= 1.0 {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("simple logistic `classification_cutoff` ({}) must lie strictly in (0, 1)", sl.classification_cutoff),
                ));
            }
            if let (Some(lo), Some(hi)) = (sl.plot_x_min, sl.plot_x_max)
                && hi <= lo
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`plot_x_max` must be strictly greater than `plot_x_min`"));
            }
            if let Some(seg) = sl.plot_segments
                && !(1..=SimpleLogisticSpec::MAX_PLOT_SEGMENTS).contains(&seg)
            {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("`plot_segments` ({seg}) must be in 1..={}", SimpleLogisticSpec::MAX_PLOT_SEGMENTS),
                ));
            }
        }
        AnalysisKind::FitSplineLowess(sp) => validate_spline_lowess_spec_semantics(sp)?,
        _ => {}
    }
    Ok(())
}

fn validate_spline_lowess_spec_semantics(sp: &SplineLowessSpec) -> Result<(), EngineError> {
    if !(SplineLowessSpec::MIN_SEGMENTS..=SplineLowessSpec::MAX_SEGMENTS).contains(&sp.segment_count) {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("`segment_count` ({}) must lie in {}..={}", sp.segment_count, SplineLowessSpec::MIN_SEGMENTS, SplineLowessSpec::MAX_SEGMENTS),
        ));
    }
    if let Some(knots) = sp.smoothing_knots
        && knots < 2
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`smoothing_knots` ({knots}) must be >= 2")));
    }
    if let Some(df) = sp.smoothing_df
        && (!df.is_finite() || df < 2.0)
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`smoothing_df` ({df}) must be finite and >= 2")));
    }
    if let Some(frac) = sp.lowess_fraction
        && (!frac.is_finite() || frac <= 0.0 || frac > 1.0)
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`lowess_fraction` ({frac}) must be finite and in (0, 1]")));
    }
    if sp.lowess_robustness_iters > SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("`lowess_robustness_iters` ({}) must be <= {}", sp.lowess_robustness_iters, SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS),
        ));
    }
    if let (Some(lo), Some(hi)) = (sp.plot_x_min, sp.plot_x_max)
        && hi <= lo
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`plot_x_min` ({lo}) must be strictly less than `plot_x_max` ({hi})")));
    }
    Ok(())
}

fn validate_nonlinear_spec_semantics(nl: &NonlinearFitSpec) -> Result<(), EngineError> {
    if let FitMethod::RoutOutlierElimination { fdr_q_percent } = nl.method
        && (!fdr_q_percent.is_finite() || fdr_q_percent <= 0.0 || fdr_q_percent > 50.0)
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("nonlinear ROUT `fdr_q_percent` ({fdr_q_percent}) must lie in (0, 50]")));
    }
    if let Some(d) = nl.output_significant_digits
        && !(1..=15).contains(&d)
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`output_significant_digits` ({d}) must lie in 1..=15")));
    }
    if let Some(rules) = &nl.fit_flag_rules {
        if rules.min_r_squared.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`fit_flag_rules.min_r_squared` must lie in [0, 1]"));
        }
        if rules.max_dependency.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`fit_flag_rules.max_dependency` must lie in [0, 1]"));
        }
        if rules.min_normality_p.is_some_and(|v| !v.is_finite() || v <= 0.0 || v >= 1.0) {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`fit_flag_rules.min_normality_p` must lie strictly in (0, 1)"));
        }
        if rules.min_runs_or_lof_p.is_some_and(|v| !v.is_finite() || v <= 0.0 || v >= 1.0) {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`fit_flag_rules.min_runs_or_lof_p` must lie strictly in (0, 1)"));
        }
    }
    if nl.parameter_rules.is_empty() {
        return Ok(());
    }
    let model = resolve_model_from_spec(nl)
        .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid nonlinear model `{}`: {err}", nl.equation_id)))?;
    let param_names = model.param_names();
    let mut seen: Vec<&str> = Vec::with_capacity(nl.parameter_rules.len());
    for rule in &nl.parameter_rules {
        if !param_names.iter().any(|p| p.eq_ignore_ascii_case(&rule.name)) {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("unknown parameter rule `{}` for `{}` (expected one of: {})", rule.name, nl.equation_id, param_names.join(", ")),
            ));
        }
        if seen.iter().any(|prev| prev.eq_ignore_ascii_case(&rule.name)) {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("duplicate `parameter_rules` entry for `{}`", rule.name)));
        }
        seen.push(&rule.name);
    }
    Ok(())
}

fn persist_default_mv_columns(source_ds: &DataSheet, spec: &mut AnalysisSpec, explicit_empty_selection: bool) {
    let TableArchetype::MultipleVariables(mv) = &source_ds.archetype else {
        return;
    };
    match &mut spec.kind {
        AnalysisKind::Multivariate(mv_spec) => {
            if spec.selected_data_sets.is_empty() {
                let pcr_outcome = match mv_spec {
                    MultivariateSpec::Pca { pcr_outcome_column, .. } => *pcr_outcome_column,
                    _ => None,
                };
                let defaults: Vec<usize> = default_multivariate_numeric_columns(mv).into_iter().filter(|c| Some(*c) != pcr_outcome).collect();
                let skip_all_continuous_explicit_pca = explicit_empty_selection
                    && pcr_outcome.is_none()
                    && matches!(mv_spec, MultivariateSpec::Pca { .. })
                    && defaults.len() == mv.variables.len();
                if !skip_all_continuous_explicit_pca {
                    spec.selected_data_sets = defaults;
                }
            }
        }
        AnalysisKind::TwoGroupComparison(tg) => persist_mv_two_group_defaults(mv, &mut spec.selected_data_sets, tg),
        AnalysisKind::NonlinearRegression(nl) => persist_mv_nonlinear_defaults(mv, &spec.selected_data_sets, nl),
        AnalysisKind::LinearRegression(_) => persist_mv_linear_defaults(mv, &mut spec.selected_data_sets),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, mv_status_column, mv_group_column, .. }) => {
            persist_mv_km_defaults(mv, &spec.selected_data_sets, mv_time_column, mv_status_column, mv_group_column);
        }
        AnalysisKind::DescriptiveStats(_)
        | AnalysisKind::OneWayAnova(_)
        | AnalysisKind::MultipleRowTTests(_)
        | AnalysisKind::OutlierDetection(_)
        | AnalysisKind::RocCurve(_)
        | AnalysisKind::PValueStack(_)
        | AnalysisKind::LeveneTest(_)
            if spec.selected_data_sets.is_empty() =>
        {
            let numeric_cols: Vec<usize> = mv.variables.iter().enumerate().filter(|(_, v)| is_mv_numeric_var(v)).map(|(i, _)| i).collect();
            if !numeric_cols.is_empty() {
                spec.selected_data_sets = numeric_cols;
            }
        }
        _ => {}
    }
}

fn is_mv_numeric_var(var: &VariableColumn) -> bool {
    !matches!(var.role, VariableRole::Categorical | VariableRole::Label)
}

fn select_default_mv_response_col(mv: &MultipleVariablesTable, candidates: &[usize]) -> Option<usize> {
    if candidates.len() <= 1 {
        return candidates.first().copied();
    }
    let active: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&idx| mv.variables.get(idx).is_some_and(|v| v.cells.iter().any(|c| c.active_number().is_some())))
        .collect();
    active
        .iter()
        .copied()
        .rfind(|&idx| mv.variables.get(idx).is_some_and(|v| v.role == VariableRole::Calculated))
        .or_else(|| active.last().copied())
        .or_else(|| candidates.last().copied())
}

fn persist_mv_two_group_defaults(mv: &MultipleVariablesTable, selected: &mut Vec<usize>, tg: &mut TwoGroupSpec) {
    let n_vars = mv.variables.len();
    let pool: Vec<usize> = if selected.is_empty() { (0..n_vars).collect() } else { selected.iter().copied().filter(|&i| i < n_vars).collect() };
    if matches!(tg.method, TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. }) {
        if tg.mv_response_column.is_none() && selected.is_empty() {
            let num_cols: Vec<usize> = pool.into_iter().filter(|&i| mv.variables.get(i).is_some_and(is_mv_numeric_var)).collect();
            if !num_cols.is_empty() {
                *selected = num_cols;
            }
        }
        return;
    }
    if tg.mv_group_column.is_some() || tg.mv_response_column.is_some() {
        if tg.mv_group_column.is_some_and(|g| g >= n_vars) || tg.mv_response_column.is_some_and(|r| r >= n_vars) {
            return;
        }
        if tg.mv_group_column.is_none() {
            let cat_cands: Vec<usize> = pool
                .iter()
                .copied()
                .filter(|&i| Some(i) != tg.mv_response_column && mv.variables.get(i).is_some_and(|v| v.role == VariableRole::Categorical))
                .collect();
            if let [only] = cat_cands.as_slice() {
                tg.mv_group_column = Some(*only);
            }
        }
        if tg.mv_response_column.is_none()
            && let Some(g_col) = tg.mv_group_column
        {
            let num_cands: Vec<usize> = pool.into_iter().filter(|&i| i != g_col && mv.variables.get(i).is_some_and(is_mv_numeric_var)).collect();
            tg.mv_response_column = select_default_mv_response_col(mv, &num_cands);
        }
        return;
    }
    let cat_cols: Vec<usize> = pool.iter().copied().filter(|&i| mv.variables.get(i).is_some_and(|v| v.role == VariableRole::Categorical)).collect();
    let num_cols: Vec<usize> = pool.iter().copied().filter(|&i| mv.variables.get(i).is_some_and(is_mv_numeric_var)).collect();
    if let [only_cat] = cat_cols.as_slice()
        && let Some(r_col) = select_default_mv_response_col(mv, &num_cols)
    {
        tg.mv_group_column = Some(*only_cat);
        tg.mv_response_column = Some(r_col);
    } else if selected.is_empty() && !num_cols.is_empty() {
        *selected = num_cols;
    }
}

fn pick_default_mv_fit_col(mv: &MultipleVariablesTable, candidates: impl Iterator<Item = usize>) -> Option<usize> {
    let mut role_match = None;
    for idx in candidates {
        let Some(var) = mv.variables.get(idx) else { continue };
        if !is_mv_numeric_var(var) {
            continue;
        }
        if var.cells.iter().any(|c| c.active_number().is_some()) {
            return Some(idx);
        }
        if role_match.is_none() {
            role_match = Some(idx);
        }
    }
    role_match
}

fn resolve_mv_fit_columns_default(
    mv: &MultipleVariablesTable,
    selected: &[usize],
    group_cols_in: &[usize],
    x_cols_in: &[usize],
    y_col_in: Option<usize>,
) -> Option<(Vec<usize>, usize, Vec<usize>)> {
    let n_vars = mv.variables.len();
    if n_vars < 2 || group_cols_in.iter().any(|&c| c >= n_vars) || x_cols_in.iter().any(|&c| c >= n_vars) || y_col_in.is_some_and(|c| c >= n_vars) {
        return None;
    }
    let group_set: HashSet<usize> = group_cols_in.iter().copied().collect();
    let x_set: HashSet<usize> = x_cols_in.iter().copied().collect();
    let selected_numeric: Vec<usize> =
        selected.iter().copied().filter(|&i| !group_set.contains(&i) && mv.variables.get(i).is_some_and(is_mv_numeric_var)).collect();

    let y_col = if let Some(yc) = y_col_in {
        yc
    } else if let Some(&sel) = selected_numeric.iter().rev().find(|i| !x_set.contains(i)) {
        sel
    } else {
        pick_default_mv_fit_col(mv, (0..n_vars).rev().filter(|i| !x_set.contains(i) && !group_set.contains(i)))
            .or_else(|| (0..n_vars).rev().find(|i| !x_set.contains(i) && !group_set.contains(i)))
            .unwrap_or_else(|| selected.first().copied().unwrap_or(1))
    };

    let selected_x: Vec<usize> = selected_numeric.iter().copied().filter(|&i| i != y_col).collect();
    let x_cols = if !x_cols_in.is_empty() {
        x_cols_in.to_vec()
    } else if !selected_x.is_empty() {
        selected_x
    } else {
        let fallback = pick_default_mv_fit_col(mv, (0..n_vars).filter(|&i| i != y_col && !group_set.contains(&i)))
            .or_else(|| (0..n_vars).find(|&i| i != y_col && !group_set.contains(&i)))
            .unwrap_or(usize::from(y_col == 0));
        vec![fallback]
    };

    let x_used: HashSet<usize> = x_cols.iter().copied().collect();
    let selected_categorical: Vec<usize> = selected
        .iter()
        .copied()
        .filter(|&i| i != y_col && !x_used.contains(&i) && mv.variables.get(i).is_some_and(|v| v.role == VariableRole::Categorical))
        .collect();
    let group_cols = if !group_cols_in.is_empty() {
        group_cols_in.to_vec()
    } else if !selected_categorical.is_empty() {
        selected_categorical
    } else if selected.len() < 2 {
        mv.variables
            .iter()
            .enumerate()
            .find(|(i, v)| *i != y_col && !x_used.contains(i) && v.role == VariableRole::Categorical)
            .map(|(i, _)| vec![i])
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Some((x_cols, y_col, group_cols))
}

fn persist_mv_nonlinear_defaults(mv: &MultipleVariablesTable, selected: &[usize], nl: &mut NonlinearFitSpec) {
    if !nl.mv_y_columns.is_empty() {
        return;
    }
    let Some((x_cols, y_col, group_cols)) = resolve_mv_fit_columns_default(mv, selected, &nl.mv_group_columns, &nl.mv_x_columns, nl.mv_y_column)
    else {
        return;
    };
    if selected.is_empty() {
        if nl.mv_x_columns.is_empty() {
            nl.mv_x_columns = x_cols;
        }
        if nl.mv_y_column.is_none() {
            nl.mv_y_column = Some(y_col);
        }
        if nl.mv_group_columns.is_empty() {
            nl.mv_group_columns = group_cols;
        }
        return;
    }
    if nl.mv_x_columns.is_empty() && nl.mv_y_column.is_none() {
        if group_cols.is_empty() {
            nl.mv_x_columns = x_cols;
            nl.mv_y_column = Some(y_col);
        } else if selected.len() < 2 {
            nl.mv_x_columns = x_cols;
            if nl.mv_group_columns.is_empty() {
                nl.mv_group_columns = group_cols;
            }
        }
    }
}

fn persist_mv_linear_defaults(mv: &MultipleVariablesTable, selected: &mut Vec<usize>) {
    if !selected.is_empty() {
        return;
    }
    let Some((x_cols, y_col, group_cols)) = resolve_mv_fit_columns_default(mv, &[], &[], &[], None) else {
        return;
    };
    if !mv.variables.get(y_col).is_some_and(is_mv_numeric_var)
        || x_cols.is_empty()
        || x_cols.iter().any(|&c| !mv.variables.get(c).is_some_and(is_mv_numeric_var))
    {
        return;
    }
    let mut pinned = Vec::new();
    for c in group_cols {
        if c != y_col && !pinned.contains(&c) {
            pinned.push(c);
        }
    }
    for c in x_cols {
        if c != y_col && !pinned.contains(&c) {
            pinned.push(c);
        }
    }
    if !pinned.is_empty() && !pinned.contains(&y_col) {
        pinned.push(y_col);
    }
    if pinned.len() >= 2 {
        *selected = pinned;
    }
}

fn is_binary_status_col(mv: &MultipleVariablesTable, col_idx: usize) -> bool {
    let Some(var) = mv.variables.get(col_idx) else { return false };
    let mut count = 0_usize;
    for cell in &var.cells {
        if cell.excluded || cell.is_empty() {
            continue;
        }
        match cell.value {
            CellValue::Number(v) if v.is_finite() && (v == 0.0 || v == 1.0) => count += 1,
            _ => return false,
        }
    }
    count > 0
}

fn persist_mv_km_defaults(
    mv: &MultipleVariablesTable,
    selected: &[usize],
    mv_time_column: &mut Option<usize>,
    mv_status_column: &mut Option<usize>,
    mv_group_column: &mut Option<usize>,
) {
    let n_vars = mv.variables.len();
    if mv_time_column.is_some_and(|t| t >= n_vars) || mv_status_column.is_some_and(|s| s >= n_vars) || mv_group_column.is_some_and(|g| g >= n_vars) {
        return;
    }
    let was_time_none = mv_time_column.is_none();
    let was_status_none = mv_status_column.is_none();
    let pool: Vec<usize> = if selected.is_empty() { (0..n_vars).collect() } else { selected.iter().copied().filter(|&i| i < n_vars).collect() };
    let status_col = match *mv_status_column {
        Some(s) => s,
        None => {
            let candidates: Vec<usize> =
                pool.iter().copied().filter(|&i| Some(i) != *mv_time_column && Some(i) != *mv_group_column && is_binary_status_col(mv, i)).collect();
            let preferred = candidates.iter().copied().find(|&i| {
                mv.variables.get(i).is_some_and(|v| {
                    let lower = v.title.to_ascii_lowercase();
                    ["status", "event", "censor", "death"].iter().any(|h| lower.contains(h))
                })
            });
            let Some(chosen) = preferred.or_else(|| candidates.first().copied()) else { return };
            *mv_status_column = Some(chosen);
            chosen
        }
    };
    let time_col = match *mv_time_column {
        Some(t) => t,
        None => {
            let candidates: Vec<usize> = pool
                .iter()
                .copied()
                .filter(|&i| {
                    i != status_col
                        && Some(i) != *mv_group_column
                        && mv.variables.get(i).is_some_and(|v| {
                            v.role != VariableRole::Categorical && v.cells.iter().filter_map(|c| c.active_number()).any(|x| x >= 0.0)
                        })
                })
                .collect();
            let preferred = candidates.iter().copied().find(|&i| {
                mv.variables.get(i).is_some_and(|v| {
                    let lower = v.title.to_ascii_lowercase();
                    ["time", "surv", "follow", "dur", "day", "month", "year"].iter().any(|h| lower.contains(h))
                })
            });
            let Some(chosen) = preferred.or_else(|| candidates.first().copied()) else { return };
            *mv_time_column = Some(chosen);
            chosen
        }
    };
    if time_col == status_col {
        return;
    }
    if mv_group_column.is_none() && (was_time_none || was_status_none || !selected.is_empty()) {
        *mv_group_column = pool
            .iter()
            .copied()
            .find(|&i| i != time_col && i != status_col && mv.variables.get(i).is_some_and(|v| v.role == VariableRole::Categorical));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyValidationMode {
    Create,
    UpdateParamsTopLevel,
    UpdateParamsNested,
}

impl KeyValidationMode {
    const fn allow_ui_update_compat(self) -> bool {
        matches!(self, Self::UpdateParamsTopLevel)
    }
}

fn validate_map_keys_for_kind(
    kind: &AnalysisKind,
    map: &Map<String, Value>,
    mode: KeyValidationMode,
    context_label: &str,
) -> Result<(), EngineError> {
    let invalid: Vec<&str> = map.keys().map(String::as_str).filter(|k| !is_key_allowed_for_kind(kind, k, mode)).collect();
    if invalid.is_empty() {
        return Ok(());
    }
    Err(EngineError::with_code(
        ErrorCode::BadInput,
        format!("unrecognized or inapplicable {context_label}(s) {} for `{}`", invalid.join(", "), kind.derive_auto_title()),
    )
    .with_remediation("Remove parameters that do not apply to the resolved analysis kind or check field spelling.")
    .with_details(json!({ "invalid_keys": invalid, "analysis": kind.derive_auto_title() })))
}

fn is_key_allowed_for_kind(kind: &AnalysisKind, key: &str, mode: KeyValidationMode) -> bool {
    match kind {
        AnalysisKind::Transform(tr) => is_key_allowed_for_transform(tr, key),
        AnalysisKind::FitSplineLowess(_) => matches!(
            key,
            "method"
                | "segment_count"
                | "smoothing_knots"
                | "smoothing_df"
                | "lowess_preset"
                | "lowess_fraction"
                | "lowess_robustness_iters"
                | "plot_x_min"
                | "plot_x_max"
                | "emit_curve_coordinates_tab"
                | "interpolate_unknowns"
        ),
        AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) => is_key_allowed_for_fit(kind, key, mode.allow_ui_update_compat()),
        _ => is_key_allowed_for_stats(kind, key, mode.allow_ui_update_compat()),
    }
}

fn is_key_allowed_for_transform(tr: &TransformAnalysisSpec, key: &str) -> bool {
    match tr {
        TransformAnalysisSpec::Math { .. } => matches!(key, "x_transform" | "y_transform" | "error_propagation"),
        TransformAnalysisSpec::Normalize { .. } => {
            matches!(key, "zero_anchor" | "hundred_anchor" | "output_percent" | "as_fraction" | "per_data_set")
        }
        TransformAnalysisSpec::RemoveBaseline { .. } => matches!(key, "baseline_columns" | "operation"),
        TransformAnalysisSpec::Transpose { .. } => key == "swap_row_and_column_titles",
        TransformAnalysisSpec::PruneRows { .. } => matches!(key, "every_k_rows" | "average_window"),
        TransformAnalysisSpec::SelectAndTransform { .. } => matches!(key, "filter_formula" | "target_archetype" | "derived_columns"),
        TransformAnalysisSpec::Simulate { .. } => {
            matches!(key, "seed" | "row_count" | "replicates" | "x_min" | "x_max" | "equation_id" | "parameters" | "noise")
        }
        TransformAnalysisSpec::RandomSample { .. } => {
            matches!(key, "seed" | "sample_size" | "stratify_column" | "proportional_allocation")
        }
    }
}

fn is_key_allowed_for_multiplicity(mult: &MultiplicitySpec, key: &str) -> bool {
    matches!(key, "multiplicity" | "report_confidence_intervals" | "alpha")
        || (matches!(mult.method, MultiplicityMethod::Dunnett { .. }) && key == "control_column")
}

fn is_key_allowed_for_fit(kind: &AnalysisKind, key: &str, allow_ui_update_compat: bool) -> bool {
    match kind {
        AnalysisKind::LinearRegression(_) => {
            (allow_ui_update_compat && key == "parameter_rules")
                || matches!(
                    key,
                    "deming"
                        | "deming_model_ii"
                        | "force_through_origin"
                        | "fixed_point"
                        | "standard_addition"
                        | "extend_line_to_x_intercept"
                        | "deming_error"
                        | "confidence_bands"
                        | "prediction_bands"
                        | "interpolate_unknowns"
                        | "x_range_filter"
                        | "plot_x_min"
                        | "plot_x_max"
                        | "plot_segments"
                )
        }
        AnalysisKind::NonlinearRegression(nl) => {
            matches!(
                key,
                "equation_id"
                    | "custom_dsl"
                    | "method"
                    | "weighting"
                    | "parameter_rules"
                    | "profile_likelihood_ci"
                    | "identify_unstable_parameters"
                    | "flag_r_squared_below"
                    | "comparison"
                    | "confidence_bands"
                    | "prediction_bands"
                    | "interpolate_unknowns"
                    | "replicates_lack_of_fit_test"
                    | "normality_of_residuals"
                    | "max_iters"
                    | "x_range_filter"
                    | "y_range_filter"
                    | "plot_function_only"
                    | "plot_x_min"
                    | "plot_x_max"
                    | "homoscedasticity_test"
                    | "emit_residuals_tab"
                    | "emit_covariance_tab"
                    | "band_log_spacing"
                    | "mv_group_columns"
                    | "mv_x_columns"
                    | "mv_y_column"
                    | "mv_y_columns"
                    | "weighting_source"
                    | "outlier_handling"
                    | "emit_cleaned_data_tab"
                    | "replicate_fit_mode"
                    | "convergence_strictness"
                    | "identifiability_mode"
                    | "show_standard_errors"
                    | "comparison_question"
                    | "comparison_method"
                    | "prefer_well_defined_fit"
                    | "summary_parameter"
                    | "create_summary_graph"
                    | "output_significant_digits"
                    | "fit_flag_rules"
            ) || (matches!(nl.method, FitMethod::RoutOutlierElimination { .. }) && matches!(key, "q_percent" | "fdr_q_percent"))
                || (nl.comparison.is_some() && key == "alternative_equation_id")
        }
        _ => false,
    }
}

fn is_key_allowed_for_stats(kind: &AnalysisKind, key: &str, allow_ui_update_compat: bool) -> bool {
    match kind {
        AnalysisKind::DescriptiveStats(_) => {
            matches!(key, "aggregation" | "custom_upper_percentile" | "run_normality_battery" | "hypothetical_value")
        }
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { .. }) => matches!(key, "q_percent" | "fdr_q_percent"),
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { .. }) => matches!(key, "alpha" | "iterative"),
        AnalysisKind::TwoGroupComparison(tg) => {
            matches!(key, "two_tailed" | "one_tailed" | "compute_effect_sizes" | "mv_group_column" | "mv_response_column")
                || (matches!(tg.method, TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. }) && key == "hypothetical")
        }
        AnalysisKind::BlandAltman(_) => matches!(key, "column_a" | "column_b" | "percent_difference" | "xy_compare_x_vs_y"),
        AnalysisKind::PassingBablok(_) => matches!(key, "x_column" | "y_column"),
        AnalysisKind::MultipleRowTTests(mr) => {
            matches!(key, "paired" | "nonparametric" | "assume_equal_sd") || is_key_allowed_for_multiplicity(&mr.multiplicity, key)
        }
        AnalysisKind::OneWayAnova(spec) => {
            matches!(key, "mixed_effects_reml" | "df_method" | "residual_covariance" | "compute_effect_sizes")
                || (matches!(spec.method, OneWayMethod::RepeatedMeasures { .. }) && key == "geisser_greenhouse")
                || is_key_allowed_for_multiplicity(&spec.multiplicity, key)
        }
        AnalysisKind::FactorialAnova(spec) => {
            matches!(
                key,
                "three_way"
                    | "multifactor_columns"
                    | "response_column"
                    | "max_interaction_order"
                    | "ss_type"
                    | "repeated_measures"
                    | "mixed_effects_reml"
                    | "df_method"
                    | "residual_covariance"
                    | "fit_reml_on_missing"
                    | "compute_effect_sizes"
            ) || is_key_allowed_for_multiplicity(&spec.multiplicity, key)
        }
        AnalysisKind::NestedComparison(spec) => {
            matches!(key, "two_group_t_test" | "df_method") || is_key_allowed_for_multiplicity(&spec.multiplicity, key)
        }
        AnalysisKind::Contingency(c) => {
            matches!(
                key,
                "method"
                    | "compute_odds_and_risk_ratios"
                    | "compute_sensitivity_specificity"
                    | "compute_phi_and_cramers_v"
                    | "expected_proportions"
                    | "null_proportion"
            ) || (matches!(c.method, ContingencyMethod::PearsonChiSquare { .. }) && key == "yates_continuity")
                || (matches!(c.method, ContingencyMethod::CohenKappa { .. }) && key == "weighting")
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => {
            matches!(
                key,
                "log_rank_mantel_cox"
                    | "gehan_breslow_wilcoxon"
                    | "trend_test"
                    | "asymmetrical_greenwood_ci"
                    | "mv_time_column"
                    | "mv_status_column"
                    | "mv_group_column"
            ) || is_key_allowed_for_multiplicity(multiplicity, key)
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. }) => {
            matches!(key, "time_column" | "status_column" | "predictor_columns" | "reference_levels" | "tie_method")
        }
        AnalysisKind::RocCurve(_) => key == "ci_method",
        AnalysisKind::GeneralizedLinearModel(_) => {
            matches!(key, "family" | "response_column" | "predictor_columns" | "reference_levels" | "compute_vif")
        }
        AnalysisKind::Multivariate(mv) => is_key_allowed_for_multivariate(mv, key, allow_ui_update_compat),
        AnalysisKind::PValueStack(_) => matches!(key, "method" | "threshold" | "column"),
        AnalysisKind::PowerAnalysis(_) => matches!(
            key,
            "family"
                | "solve_target"
                | "tail_mode"
                | "two_tailed"
                | "alpha"
                | "target_power"
                | "sample_size"
                | "sample_size_group2"
                | "allocation_ratio"
                | "effect"
                | "effect_size"
                | "chi_square_df"
                | "two_proportions_mode"
                | "continuity_correction"
                | "correlation_method"
                | "info_sheet_id"
                | "num_groups"
        ),
        AnalysisKind::SimpleLogisticRegression(_) => {
            matches!(key, "emit_row_classification" | "confidence_bands" | "classification_cutoff" | "plot_x_min" | "plot_x_max" | "plot_segments")
        }
        AnalysisKind::LeveneTest(_) => key == "center",
        AnalysisKind::Transform(_) | AnalysisKind::FitSplineLowess(_) | AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) => {
            false
        }
    }
}

fn is_key_allowed_for_multivariate(mv: &MultivariateSpec, key: &str, allow_ui_update_compat: bool) -> bool {
    match mv {
        MultivariateSpec::Pca { .. } => matches!(key, "standardize" | "parallel_analysis_seed" | "pcr_outcome_column"),
        MultivariateSpec::CorrelationMatrix { .. } => {
            matches!(key, "spearman" | "spearman_rank") || (allow_ui_update_compat && matches!(key, "correlation_all_pairs" | "correlation_heatmap"))
        }
        MultivariateSpec::Clustering { .. } => matches!(key, "k_means_k" | "k_clusters" | "distance" | "linkage" | "k_sweep_max"),
        MultivariateSpec::MultipleImputation { .. } => {
            matches!(key, "imputations_m" | "max_iters" | "seed" | "response_column" | "predictor_columns")
        }
    }
}

fn validate_xy_glm_params(xy: &XyTable, selected_data_sets: &[usize], glm: &GlmSpec) -> Result<(), EngineError> {
    if xy.y_subcolumn_mode.is_pre_summarized() {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "XY GLM requires raw replicate or single-Y values rather than pre-summarized statistics",
        ));
    }
    for &idx in selected_data_sets {
        if idx >= xy.data_sets.len() {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("selected dataset index {idx} out of range for XY table with {} Y data sets", xy.data_sets.len()),
            ));
        }
    }
    if glm.response_column >= xy.data_sets.len() {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("GLM `response_column` {} out of range for XY table with {} Y data sets", glm.response_column, xy.data_sets.len()),
        ));
    }
    if let [only_ds] = selected_data_sets
        && glm.response_column != 0
        && glm.response_column != *only_ds
    {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("GLM `response_column` {} does not match selected XY dataset {}", glm.response_column, only_ds),
        ));
    }
    if selected_data_sets.len() > 1 && !selected_data_sets.contains(&glm.response_column) {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("GLM `response_column` {} must be included in `selected_data_sets`", glm.response_column),
        ));
    }
    if glm.predictor_columns.is_empty() {
        return Err(EngineError::with_code(ErrorCode::InsufficientData, "XY GLM requires one X predictor column in `predictor_columns`"));
    }
    let max_pred_space = xy.data_sets.len().max(2);
    for &pc in &glm.predictor_columns {
        if pc >= max_pred_space {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("GLM predictor column {pc} out of range for XY table")));
        }
    }
    if !matches!(glm.predictor_columns.as_slice(), &[pc] if pc < 2) {
        return Err(EngineError::with_code(ErrorCode::BadInput, "XY GLM supports a single X predictor (`predictor_columns: [0]` or `[1]`)"));
    }
    Ok(())
}

fn apply_metadata_updates(
    spec: &mut AnalysisSpec,
    title: Option<Option<&str>>,
    confidence_percent: Option<f64>,
    selected_data_sets: Option<&[Value]>,
    p_value_format: Option<&Value>,
) -> Result<(), EngineError> {
    if let Some(title_opt) = title {
        match title_opt {
            None => spec.set_custom_title(None)?,
            Some(s) => {
                let custom = if s.trim().is_empty() { None } else { Some(s.to_owned()) };
                spec.set_custom_title(custom)?;
            }
        }
    }
    if let Some(pct) = confidence_percent {
        validate_confidence_percent(pct)?;
        spec.set_confidence_percent(pct)?;
    }
    if let Some(vals) = selected_data_sets {
        let mut indices = Vec::with_capacity(vals.len());
        for v in vals {
            indices.push(parse_usize_json_value(v, "selected_data_sets")?);
        }
        spec.selected_data_sets = indices;
    }
    if let Some(pv_val) = p_value_format {
        spec.p_value_format = parse_p_value_format(pv_val)?;
    }
    Ok(())
}

fn parse_analysis_kind(
    kind_val: Option<&Value>,
    params: &Value,
    extra: &Map<String, Value>,
    domain: AnalysisDomain,
    table_kind: TableKind,
) -> Result<AnalysisKind, EngineError> {
    let mut kind = if let Some(kv) = kind_val
        && kv.is_object()
    {
        deserialize_strict(kv, "/kind")?
    } else {
        let raw_kind = kind_val.and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase());
        match raw_kind.as_deref() {
            None => infer_default_kind(params, domain, table_kind)?,
            Some(name) => resolve_kind_shorthand(name, params)?,
        }
    };
    apply_kind_patch_and_overrides(&mut kind, params)?;
    validate_map_keys_for_kind(&kind, extra, KeyValidationMode::Create, "parameter")?;
    Ok(kind)
}

fn infer_default_kind(params: &Value, domain: AnalysisDomain, table_kind: TableKind) -> Result<AnalysisKind, EngineError> {
    if params.get("equation_id").is_some() || params.get("custom_dsl").is_some() {
        return resolve_fit_shorthand("nonlinear", params);
    }
    match domain {
        AnalysisDomain::TransformOnly => Ok(default_transform_math()),
        AnalysisDomain::FitOnly => resolve_fit_shorthand("four_parameter_logistic", params),
        AnalysisDomain::StatsOnly | AnalysisDomain::Any => match table_kind {
            TableKind::Xy => resolve_fit_shorthand("four_parameter_logistic", params),
            TableKind::Column => Ok(AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::OrdinaryAnova))),
            TableKind::Grouped => Ok(AnalysisKind::FactorialAnova(default_factorial_anova(false))),
            TableKind::Contingency => Ok(AnalysisKind::Contingency(ContingencySpec::default())),
            TableKind::Survival => Ok(AnalysisKind::Survival(default_km_survival())),
            TableKind::MultipleVariables => Ok(AnalysisKind::Multivariate(default_pca())),
            TableKind::Nested => Ok(AnalysisKind::NestedComparison(NestedAnalysisSpec::default())),
            TableKind::PartsOfWhole => Ok(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())),
        },
    }
}

fn resolve_kind_shorthand(name: &str, params: &Value) -> Result<AnalysisKind, EngineError> {
    if let Some(kind) = resolve_transform_shorthand(name) {
        return Ok(kind);
    }
    if let Some(kind) = resolve_two_group_or_anova_shorthand(name, params)? {
        return Ok(kind);
    }
    if let Some(kind) = resolve_other_stats_shorthand(name) {
        return Ok(kind);
    }
    resolve_fit_shorthand(name, params)
}

fn default_transform_math() -> AnalysisKind {
    AnalysisKind::Transform(TransformAnalysisSpec::Math {
        x_transform: None,
        y_transform: Some(MathTransformPreset::Log10),
        error_propagation: ErrorPropagationMode::ReplicateEndpoints,
    })
}

fn resolve_transform_shorthand(name: &str) -> Option<AnalysisKind> {
    let tr = match name {
        "math" | "transform.math" | "math_transform" => return Some(default_transform_math()),
        "normalize" | "transform.normalize" => TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: true,
        },
        "remove_baseline" | "baseline" | "transform.remove_baseline" => {
            TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Difference }
        }
        "transpose" | "transform.transpose" => TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true },
        "prune_rows" | "prune" | "transform.prune_rows" => TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: false },
        "select_and_transform" | "select" | "transform.select_and_transform" => {
            TransformAnalysisSpec::SelectAndTransform { filter_formula: None, target_archetype: TableKind::Xy, derived_columns: Vec::new() }
        }
        "simulate" | "simulate_xy" | "transform.simulate" => TransformAnalysisSpec::Simulate {
            seed: 42,
            row_count: 13,
            replicates: 3,
            x_min: -9.0,
            x_max: -3.0,
            equation_id: String::from("log_agonist_vs_response_4pl"),
            parameters: Vec::new(),
            noise: SimulationNoise::None,
        },
        "random_sample" | "sample" | "transform.random_sample" => {
            TransformAnalysisSpec::RandomSample { seed: 42, sample_size: 10, stratify_column: None, proportional_allocation: true }
        }
        "fit_spline_lowess" | "spline_lowess" | "spline" | "cubic_spline" | "natural_cubic_spline" | "fit.spline_lowess" => {
            return Some(AnalysisKind::FitSplineLowess(SplineLowessSpec {
                method: SplineLowessMethod::NaturalCubicSpline,
                ..SplineLowessSpec::default()
            }));
        }
        "akima" | "akima_subspline" => {
            return Some(AnalysisKind::FitSplineLowess(SplineLowessSpec {
                method: SplineLowessMethod::AkimaSubspline,
                ..SplineLowessSpec::default()
            }));
        }
        "smoothing_spline" => {
            return Some(AnalysisKind::FitSplineLowess(SplineLowessSpec {
                method: SplineLowessMethod::SmoothingSpline,
                ..SplineLowessSpec::default()
            }));
        }
        "lowess" => {
            return Some(AnalysisKind::FitSplineLowess(SplineLowessSpec { method: SplineLowessMethod::Lowess, ..SplineLowessSpec::default() }));
        }
        "point_to_point" => {
            return Some(AnalysisKind::FitSplineLowess(SplineLowessSpec { method: SplineLowessMethod::PointToPoint, ..SplineLowessSpec::default() }));
        }
        _ => return None,
    };
    Some(AnalysisKind::Transform(tr))
}

fn resolve_two_group_or_anova_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let hyp = opt_f64_checked(params, "hypothetical")?.unwrap_or(0.0);
    let kind = match name {
        "unpaired_t" | "t_test" | "unpaired_t_test" => two_group_kind(TwoGroupMethod::UnpairedT),
        "welch_t" | "welch_t_test" => two_group_kind(TwoGroupMethod::WelchT),
        "paired_t" | "paired_t_test" => two_group_kind(TwoGroupMethod::PairedT),
        "ratio_paired_t" => two_group_kind(TwoGroupMethod::RatioPairedT),
        "lognormal_t" | "lognormal_unpaired_t" => two_group_kind(TwoGroupMethod::LognormalUnpairedT),
        "mann_whitney" | "mann_whitney_u" => two_group_kind(TwoGroupMethod::MannWhitneyU),
        "wilcoxon" | "wilcoxon_matched_pairs" | "wilcoxon_signed_rank" => two_group_kind(TwoGroupMethod::WilcoxonSignedRank),
        "paired_sign_test" | "paired_sign" | "sign_test" => two_group_kind(TwoGroupMethod::PairedSignTest),
        "kolmogorov_smirnov" | "ks" | "ks_two_sample" => two_group_kind(TwoGroupMethod::KolmogorovSmirnov),
        "one_sample_t" => two_group_kind(TwoGroupMethod::OneSampleT { hypothetical: hyp }),
        "one_sample_wilcoxon" => two_group_kind(TwoGroupMethod::OneSampleWilcoxon { hypothetical: hyp }),
        "one_way_anova" | "anova" => AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::OrdinaryAnova)),
        "welch_anova" | "brown_forsythe" => AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::BrownForsytheAndWelch)),
        "rm_one_way_anova" | "repeated_measures_anova" => {
            let gg = opt_bool_checked(params, "geisser_greenhouse")?.unwrap_or(true);
            AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::RepeatedMeasures { geisser_greenhouse: gg }))
        }
        "kruskal_wallis" => AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::KruskalWallis)),
        "friedman" => AnalysisKind::OneWayAnova(default_one_way_anova(OneWayMethod::Friedman)),
        "two_way_anova" | "factorial_anova" => AnalysisKind::FactorialAnova(default_factorial_anova(false)),
        "three_way_anova" => AnalysisKind::FactorialAnova(default_factorial_anova(true)),
        _ => return Ok(None),
    };
    Ok(Some(kind))
}

fn resolve_other_stats_shorthand(name: &str) -> Option<AnalysisKind> {
    let kind = match name {
        "descriptive" | "descriptive_stats" | "column_stats" => AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
        "outliers" | "outlier" | "rout" => AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }),
        "grubbs" => AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false }),
        "nested_t" | "nested_anova" | "nested" => {
            AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: name == "nested_t", ..NestedAnalysisSpec::default() })
        }
        "multiple_row_t_tests" | "stats.multiple_row_t_tests" | "multiple_row_tests" | "multiple_t_tests" => {
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default())
        }
        "bland_altman" | "stats.bland_altman" => AnalysisKind::BlandAltman(BlandAltmanSpec::default()),
        "passing_bablok" | "stats.passing_bablok" => AnalysisKind::PassingBablok(PassingBablokSpec::default()),
        "pvalue_stack" | "stats.pvalue_stack" | "p_value_stack" | "pvalues" | "pvaluestack" | "stats.pvaluestack" => {
            AnalysisKind::PValueStack(PValueStackSpec::default())
        }
        "power" | "power_analysis" | "stats.power" | "power_sample_size" => AnalysisKind::PowerAnalysis(PowerAnalysisSpec::default()),
        "contingency"
        | "chi_square"
        | "fisher_exact"
        | "barnard_exact"
        | "stats.barnard_exact"
        | "cohen_kappa"
        | "kappa"
        | "fleiss_kappa"
        | "exact_binomial"
        | "binomial" => {
            let method = match name {
                "fisher_exact" => ContingencyMethod::FisherExact,
                "barnard_exact" | "stats.barnard_exact" => ContingencyMethod::BarnardExact,
                "cohen_kappa" | "kappa" => ContingencyMethod::CohenKappa { weighting: KappaWeighting::default() },
                "fleiss_kappa" => ContingencyMethod::FleissKappa,
                "exact_binomial" | "binomial" => ContingencyMethod::ExactBinomial,
                _ => ContingencyMethod::PearsonChiSquare { yates_continuity: false },
            };
            AnalysisKind::Contingency(ContingencySpec { method, ..ContingencySpec::default() })
        }
        "kaplan_meier" | "survival" | "log_rank" => AnalysisKind::Survival(default_km_survival()),
        "cox_ph" | "cox" => AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
        "roc" => AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong }),
        "glm" | "linear_glm" | "multiple_linear" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Linear)),
        "logistic_glm" | "multiple_logistic" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Logistic)),
        "poisson_glm" | "multiple_poisson" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Poisson)),
        "simple_logistic" | "simple_logistic_regression" | "stats.simple_logistic" | "xy_logistic" => {
            AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())
        }
        "levene_mean" => AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Mean }),
        "levene"
        | "levene_test"
        | "stats.levene"
        | "levene_median"
        | "brown_forsythe_variance"
        | "brown_forsythe_levene"
        | "variance_homogeneity"
        | "equal_variance_test" => AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Median }),
        "pca" | "stats.pca" | "principal_components" | "pcr" | "stats.pcr" | "principal_component_regression" => {
            AnalysisKind::Multivariate(default_pca())
        }
        "correlation" | "correlation_matrix" => AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }),
        "clustering" | "kmeans" => AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: (name == "kmeans").then_some(3),
            distance: ClusterDistance::default(),
            linkage: ClusterLinkage::default(),
            k_sweep_max: None,
        }),
        "multiple_imputation" | "stats.multiple_imputation" | "mice" => AnalysisKind::Multivariate(default_multiple_imputation(42)),
        _ => return None,
    };
    Some(kind)
}

fn resolve_fit_shorthand(name: &str, params: &Value) -> Result<AnalysisKind, EngineError> {
    if matches!(name, "linear" | "simple_linear" | "linear_regression" | "deming" | "deming_regression") {
        let deming_model_ii = matches!(name, "deming" | "deming_regression") || opt_bool_checked(params, "deming")?.unwrap_or(false);
        return Ok(AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii, ..LinearRegressionSpec::default() }));
    }

    let explicit_eq = opt_str_checked(params, "equation_id")?;
    let explicit_dsl = opt_str_checked(params, "custom_dsl")?;
    let eq_id = match name {
        "four_parameter_logistic" | "4pl" | "4pl_agonist" => "log_agonist_vs_response_4pl",
        "4pl_inhibitor" | "four_parameter_logistic_inhibitor" | "ic50" => "log_inhibitor_vs_response_4pl",
        "agonist_4pl" => "agonist_vs_response_4pl",
        "inhibitor_4pl" => "inhibitor_vs_response_4pl",
        "3pl" | "three_parameter_logistic" => "log_agonist_vs_response_3pl",
        "3pl_inhibitor" => "log_inhibitor_vs_response_3pl",
        "one_site_binding" | "saturation_binding" => "one_site_specific_binding",
        "nonlinear" | "nonlinear_regression" => explicit_eq.unwrap_or("log_agonist_vs_response_4pl"),
        other => {
            if get_builtin_model(other).is_some() {
                other
            } else if explicit_dsl.is_some() {
                "custom_dsl"
            } else {
                let suggestions = suggest_analysis_kinds(other);
                let mut remediation = String::from(
                    "Supply a supported analysis kind (`unpaired_t`, `welch_t`, `mann_whitney`, `one_way_anova`, `kruskal_wallis`, `two_way_anova`, `descriptive`, `rout`, `linear`, `log_inhibitor_vs_response_4pl`, `log_agonist_vs_response_4pl`, ...) or pass `custom_dsl`.",
                );
                if !suggestions.is_empty() {
                    remediation = format!("Did you mean: {}? {remediation}", suggestions.join(", "));
                }
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown analysis kind or equation `{other}`"))
                    .with_remediation(remediation)
                    .with_details(json!({ "pointer": "/kind", "kind": other, "suggestions": suggestions })));
            }
        }
    };
    let chosen_eq = explicit_eq.unwrap_or(eq_id);
    let mut fit = NonlinearFitSpec::new(chosen_eq);
    if let Some(dsl) = explicit_dsl {
        fit.custom_dsl = Some(dsl.to_owned());
    }
    Ok(AnalysisKind::NonlinearRegression(fit))
}

fn suggest_analysis_kinds(raw: &str) -> Vec<String> {
    const KNOWN_SHORTHANDS: &[&str] = &[
        "unpaired_t",
        "welch_t",
        "paired_t",
        "ratio_paired_t",
        "lognormal_t",
        "mann_whitney",
        "wilcoxon",
        "paired_sign_test",
        "kolmogorov_smirnov",
        "one_sample_t",
        "one_sample_wilcoxon",
        "one_way_anova",
        "welch_anova",
        "rm_one_way_anova",
        "kruskal_wallis",
        "friedman",
        "two_way_anova",
        "three_way_anova",
        "descriptive",
        "rout",
        "grubbs",
        "nested_t",
        "nested_anova",
        "multiple_row_t_tests",
        "bland_altman",
        "passing_bablok",
        "pvalue_stack",
        "power",
        "levene",
        "levene_mean",
        "contingency",
        "chi_square",
        "fisher_exact",
        "cohen_kappa",
        "fleiss_kappa",
        "kaplan_meier",
        "cox_ph",
        "roc",
        "linear_glm",
        "logistic_glm",
        "poisson_glm",
        "pca",
        "correlation",
        "clustering",
        "multiple_imputation",
        "linear",
        "deming",
        "4pl",
        "4pl_inhibitor",
        "3pl",
        "3pl_inhibitor",
        "one_site_binding",
        "math",
        "normalize",
        "remove_baseline",
        "transpose",
        "prune_rows",
        "select_and_transform",
        "simulate",
        "random_sample",
        "fit_spline_lowess",
        "spline",
        "akima",
        "smoothing_spline",
        "lowess",
        "point_to_point",
    ];
    let eqs = list_builtin_equations();
    let all_candidates = KNOWN_SHORTHANDS.iter().copied().chain(eqs.iter().map(|eq| eq.id.as_str()));
    super::fuzzy_suggest(raw, '_', all_candidates, 3)
}

fn map_canonical_kind_fields(params: &Value, kind: &AnalysisKind) -> Value {
    let mapping: Option<(&str, &str)> = match kind {
        AnalysisKind::LinearRegression(_) => Some(("deming", "deming_model_ii")),
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { .. }) => Some(("q_percent", "fdr_q_percent")),
        AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }) => Some(("spearman", "spearman_rank")),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. }) => Some(("k_clusters", "k_means_k")),
        _ => None,
    };
    let Some((canonical_key, struct_field)) = mapping else {
        return params.clone();
    };
    let Some(orig) = params.as_object() else {
        return params.clone();
    };
    let Some(val) = orig.get(canonical_key) else {
        return params.clone();
    };
    let mut out = orig.clone();
    out.remove(canonical_key);
    out.insert(struct_field.to_owned(), val.clone());
    Value::Object(out)
}

fn is_omitted_optional_field(kind: &AnalysisKind, key: &str) -> bool {
    match kind {
        AnalysisKind::BlandAltman(_) => key == "xy_compare_x_vs_y",
        AnalysisKind::Contingency(_) => matches!(key, "expected_proportions" | "null_proportion"),
        AnalysisKind::Multivariate(MultivariateSpec::Pca { .. }) => key == "pcr_outcome_column",
        AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. }) => key == "k_sweep_max",
        AnalysisKind::LinearRegression(_) => matches!(key, "fixed_point" | "plot_x_min" | "plot_x_max" | "plot_segments"),
        AnalysisKind::FitSplineLowess(_) => matches!(key, "smoothing_knots" | "smoothing_df" | "lowess_fraction" | "plot_x_min" | "plot_x_max"),
        AnalysisKind::NonlinearRegression(_) => matches!(
            key,
            "flag_r_squared_below"
                | "identifiability_mode"
                | "show_standard_errors"
                | "comparison_question"
                | "comparison_method"
                | "summary_parameter"
                | "output_significant_digits"
                | "fit_flag_rules"
        ),
        AnalysisKind::PowerAnalysis(_) => key == "info_sheet_id",
        AnalysisKind::SimpleLogisticRegression(_) => matches!(key, "plot_x_min" | "plot_x_max" | "plot_segments"),
        _ => false,
    }
}

const NULLABLE_KIND_FIELDS: &[&str] = &[
    "fixed_point",
    "plot_x_min",
    "plot_x_max",
    "plot_segments",
    "custom_upper_percentile",
    "hypothetical_value",
    "mv_group_column",
    "mv_response_column",
    "mv_time_column",
    "mv_status_column",
    "xy_compare_x_vs_y",
    "column",
    "target_power",
    "sample_size",
    "sample_size_group2",
    "info_sheet_id",
    "num_groups",
    "filter_formula",
    "stratify_column",
    "null_proportion",
    "parallel_analysis_seed",
    "pcr_outcome_column",
    "k_means_k",
    "k_sweep_max",
    "response_column",
    "custom_dsl",
    "flag_r_squared_below",
    "mv_y_column",
    "identifiability_mode",
    "show_standard_errors",
    "comparison_question",
    "comparison_method",
    "summary_parameter",
    "output_significant_digits",
    "fit_flag_rules",
    "smoothing_knots",
    "smoothing_df",
    "lowess_fraction",
];

const STRICT_NON_NULL_USIZE_FIELDS: &[&str] = &[
    "column_a",
    "column_b",
    "x_column",
    "y_column",
    "max_interaction_order",
    "every_k_rows",
    "sample_size",
    "row_count",
    "replicates",
    "time_column",
    "status_column",
    "response_column",
    "imputations_m",
    "max_iters",
    "segment_count",
    "lowess_robustness_iters",
];

const INTEGER_OR_INT_VEC_FIELDS: &[&str] = &[
    "plot_segments",
    "segment_count",
    "smoothing_knots",
    "lowess_robustness_iters",
    "mv_group_column",
    "mv_response_column",
    "mv_time_column",
    "mv_status_column",
    "column_a",
    "column_b",
    "x_column",
    "y_column",
    "max_interaction_order",
    "column",
    "num_groups",
    "every_k_rows",
    "sample_size",
    "stratify_column",
    "seed",
    "row_count",
    "replicates",
    "time_column",
    "status_column",
    "response_column",
    "parallel_analysis_seed",
    "pcr_outcome_column",
    "k_means_k",
    "k_sweep_max",
    "imputations_m",
    "max_iters",
    "mv_y_column",
    "baseline_columns",
    "predictor_columns",
    "mv_group_columns",
    "mv_x_columns",
    "mv_y_columns",
    "output_significant_digits",
];

fn coerce_whole_int_value(val: &Value, key: &str) -> Result<Value, EngineError> {
    if let Value::Array(items) = val {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(Value::Number(parse_u64_json_value(item, key)?.into()));
        }
        return Ok(Value::Array(out));
    }
    Ok(Value::Number(parse_u64_json_value(val, key)?.into()))
}

fn apply_enum_field<T: serde::de::DeserializeOwned>(params: &Value, key: &str, target: &mut T, field_desc: &str) -> Result<(), EngineError> {
    if let Some(val) = params.get(key)
        && !val.is_null()
    {
        *target = if val.is_object() { deserialize_strict(val, "")? } else { parse_serde_enum(val, field_desc)? };
    }
    Ok(())
}

fn apply_opt_enum_field<T: serde::de::DeserializeOwned>(
    params: &Value,
    key: &str,
    target: &mut Option<T>,
    field_desc: &str,
) -> Result<(), EngineError> {
    if let Some(val) = params.get(key) {
        *target = if val.is_null() {
            None
        } else if val.is_object() {
            Some(deserialize_strict(val, "")?)
        } else {
            Some(parse_serde_enum(val, field_desc)?)
        };
    }
    Ok(())
}

fn apply_kind_patch_and_overrides(kind: &mut AnalysisKind, params: &Value) -> Result<(), EngineError> {
    let normalized = map_canonical_kind_fields(params, kind);
    patch_analysis_kind_fields(kind, &normalized)?;
    apply_custom_kind_overrides(kind, &normalized)?;
    Ok(())
}

fn apply_custom_kind_overrides(kind: &mut AnalysisKind, norm: &Value) -> Result<(), EngineError> {
    match kind {
        AnalysisKind::LinearRegression(spec) => {
            if let Some(v) = norm.get("x_range_filter") {
                spec.x_range_filter = parse_opt_range_filter(v, "x_range_filter")?;
            }
            if let Some((x0, y0)) = spec.fixed_point
                && (!x0.is_finite() || !y0.is_finite())
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`fixed_point` coordinates (x0, y0) must be finite numbers"));
            }
            if spec.deming_error.resolved_lambda().is_none() {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    "`deming_error` standard deviations and variance ratio lambda must be finite and strictly positive",
                ));
            }
        }
        AnalysisKind::TwoGroupComparison(spec) => {
            if opt_bool_checked(norm, "two_tailed")?.is_none()
                && let Some(one_tailed) = opt_bool_checked(norm, "one_tailed")?
            {
                spec.two_tailed = !one_tailed;
            }
            if let Some(h) = opt_f64_checked(norm, "hypothetical")? {
                match &mut spec.method {
                    TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
                        *hypothetical = h;
                    }
                    _ => {}
                }
            }
        }
        AnalysisKind::MultipleRowTTests(spec) => apply_multiplicity_override(&mut spec.multiplicity, norm)?,
        AnalysisKind::OneWayAnova(spec) => {
            if let OneWayMethod::RepeatedMeasures { ref mut geisser_greenhouse } = spec.method {
                apply_bool_opt_checked(norm, "geisser_greenhouse", geisser_greenhouse)?;
            }
            apply_enum_field(norm, "df_method", &mut spec.df_method, "`df_method`")?;
            apply_enum_field(norm, "residual_covariance", &mut spec.residual_covariance, "`residual_covariance`")?;
            apply_multiplicity_override(&mut spec.multiplicity, norm)?;
        }
        AnalysisKind::NestedComparison(spec) => {
            apply_enum_field(norm, "df_method", &mut spec.df_method, "`df_method`")?;
            apply_multiplicity_override(&mut spec.multiplicity, norm)?;
        }
        AnalysisKind::FactorialAnova(spec) => {
            apply_enum_field(norm, "repeated_measures", &mut spec.repeated_measures, "`repeated_measures`")?;
            apply_enum_field(norm, "df_method", &mut spec.df_method, "`df_method`")?;
            apply_enum_field(norm, "residual_covariance", &mut spec.residual_covariance, "`residual_covariance`")?;
            apply_multiplicity_override(&mut spec.multiplicity, norm)?;
        }
        AnalysisKind::PValueStack(spec) => {
            apply_enum_field(norm, "method", &mut spec.method, "P-value stack `method`")?;
            if !spec.threshold.is_finite() || spec.threshold <= 0.0 || spec.threshold >= 1.0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`threshold` (`alpha` / `Q`) must be a finite number strictly in (0, 1)"));
            }
        }
        AnalysisKind::FitSplineLowess(spec) => {
            apply_enum_field(norm, "method", &mut spec.method, "spline/LOWESS `method`")?;
            apply_enum_field(norm, "lowess_preset", &mut spec.lowess_preset, "LOWESS `lowess_preset`")?;
        }
        AnalysisKind::PowerAnalysis(spec) => {
            apply_enum_field(norm, "family", &mut spec.family, "power test `family`")?;
            apply_enum_field(norm, "solve_target", &mut spec.solve_target, "power `solve_target`")?;
            if let Some(two_tailed) = opt_bool_checked(norm, "two_tailed")? {
                spec.tail_mode = if two_tailed { PowerTailMode::TwoSided } else { PowerTailMode::OneSided(PowerOneSidedDirection::Greater) };
            }
            if let Some(eff) = opt_f64_checked(norm, "effect_size")? {
                spec.effect = PowerEffectSpec::Standardized(eff);
            }
            if let Some(ref info_id) = spec.info_sheet_id {
                let _ = SheetId::try_new(info_id.as_str())?;
            }
        }
        AnalysisKind::Transform(spec) => apply_transform_custom_overrides(spec, norm)?,
        AnalysisKind::Contingency(spec) => apply_contingency_custom_overrides(spec, norm)?,
        AnalysisKind::Survival(spec) => match spec {
            SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. } => apply_multiplicity_override(multiplicity, norm)?,
            SurvivalAnalysisSpec::CoxProportionalHazards { reference_levels, tie_method, .. } => {
                if let Some(rl) = norm.get("reference_levels") {
                    *reference_levels = if rl.is_null() { Vec::new() } else { parse_reference_levels(rl)? };
                }
                apply_enum_field(norm, "tie_method", tie_method, "Cox `tie_method`")?;
            }
        },
        AnalysisKind::GeneralizedLinearModel(spec) => {
            apply_enum_field(norm, "family", &mut spec.family, "GLM `family`")?;
            if let Some(rl) = norm.get("reference_levels") {
                spec.reference_levels = if rl.is_null() { Vec::new() } else { parse_reference_levels(rl)? };
            }
        }
        AnalysisKind::Multivariate(spec) => apply_multivariate_custom_overrides(spec, norm)?,
        AnalysisKind::NonlinearRegression(spec) => apply_nonlinear_custom_overrides(spec, norm)?,
        _ => {}
    }
    Ok(())
}

fn apply_transform_custom_overrides(spec: &mut TransformAnalysisSpec, norm: &Value) -> Result<(), EngineError> {
    match spec {
        TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation } => {
            apply_opt_enum_field(norm, "x_transform", x_transform, "`x_transform`")?;
            apply_opt_enum_field(norm, "y_transform", y_transform, "`y_transform`")?;
            apply_enum_field(norm, "error_propagation", error_propagation, "`error_propagation`")?;
        }
        TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, .. } => {
            apply_enum_field(norm, "zero_anchor", zero_anchor, "`zero_anchor`")?;
            apply_enum_field(norm, "hundred_anchor", hundred_anchor, "`hundred_anchor`")?;
            if let Some(as_frac) = opt_bool_checked(norm, "as_fraction")? {
                *output_percent = !as_frac;
            }
        }
        TransformAnalysisSpec::RemoveBaseline { operation, .. } => {
            apply_enum_field(norm, "operation", operation, "`operation`")?;
        }
        TransformAnalysisSpec::PruneRows { every_k_rows, .. } => {
            if *every_k_rows == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`every_k_rows` must be >= 1"));
            }
        }
        TransformAnalysisSpec::SelectAndTransform { target_archetype, derived_columns, .. } => {
            if let Some(tv) = norm.get("target_archetype").filter(|v| !v.is_null()) {
                *target_archetype = parse_target_table_kind(tv)?;
            }
            if let Some(dv) = norm.get("derived_columns") {
                *derived_columns = parse_derived_columns_value(dv)?;
            }
        }
        TransformAnalysisSpec::RandomSample { sample_size, .. } => {
            if *sample_size == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`sample_size` must be >= 1"));
            }
        }
        TransformAnalysisSpec::Simulate { row_count, replicates, parameters, .. } => {
            *row_count = (*row_count).max(2);
            *replicates = (*replicates).max(1);
            if let Some(p_val) = norm.get("parameters")
                && !p_val.is_null()
            {
                *parameters = parse_simulate_parameters_override(p_val)?;
            }
        }
        TransformAnalysisSpec::Transpose { .. } => {}
    }
    Ok(())
}

fn apply_contingency_custom_overrides(spec: &mut ContingencySpec, norm: &Value) -> Result<(), EngineError> {
    if let Some(m_val) = norm.get("method") {
        spec.method = parse_contingency_method_value(m_val)?;
    }
    if let ContingencyMethod::PearsonChiSquare { yates_continuity } = &mut spec.method {
        apply_bool_opt_checked(norm, "yates_continuity", yates_continuity)?;
    }
    if let ContingencyMethod::CohenKappa { weighting } = &mut spec.method {
        apply_enum_field(norm, "weighting", weighting, "kappa `weighting`")?;
    }
    if let Some(p0) = spec.null_proportion
        && (p0 <= 0.0 || p0 >= 1.0)
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`null_proportion` must lie strictly in (0, 1)"));
    }
    if let Some(ep_val) = norm.get("expected_proportions") {
        spec.expected_proportions = parse_opt_expected_proportions(ep_val)?;
    }
    Ok(())
}

fn apply_multiplicity_override(mult: &mut MultiplicitySpec, params: &Value) -> Result<(), EngineError> {
    if let Some(m_val) = params.get("multiplicity")
        && !m_val.is_null()
    {
        if m_val.is_object() {
            if m_val.as_object().is_some_and(|o| o.contains_key("method")) {
                *mult = deserialize_strict(m_val, "/multiplicity")?;
            } else {
                mult.method = deserialize_strict(m_val, "/multiplicity")?;
            }
        } else if let Ok(parsed) = serde_json::from_value::<MultiplicitySpec>(m_val.clone()) {
            *mult = parsed;
        } else if m_val.as_str().is_some_and(|s| s.trim().eq_ignore_ascii_case("dunnett")) {
            mult.method = MultiplicityMethod::Dunnett { control_column: 0 };
        } else {
            mult.method = parse_serde_enum(m_val, "`multiplicity`")?;
        }
    }
    if let MultiplicityMethod::Dunnett { ref mut control_column } = mult.method {
        apply_usize_field(params, "control_column", control_column)?;
    }
    apply_bool_opt_checked(params, "report_confidence_intervals", &mut mult.report_confidence_intervals)?;
    if let Some(t) = opt_f64_checked(params, "alpha")? {
        mult.threshold = t;
    }
    Ok(())
}

fn apply_multivariate_custom_overrides(spec: &mut MultivariateSpec, norm: &Value) -> Result<(), EngineError> {
    match spec {
        MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max } => {
            if let Some(k) = *k_means_k
                && k < 2
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`k_means_k` must be >= 2"));
            }
            apply_enum_field(norm, "distance", distance, "cluster `distance`")?;
            apply_enum_field(norm, "linkage", linkage, "cluster `linkage`")?;
            if let Some(k_max) = *k_sweep_max
                && k_max < 2
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`k_sweep_max` must be >= 2"));
            }
        }
        MultivariateSpec::MultipleImputation { imputations_m, max_iters, .. } => {
            if !(2..=50).contains(imputations_m) {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`imputations_m` must be in 2..=50"));
            }
            if !(1..=100).contains(max_iters) {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`max_iters` must be in 1..=100"));
            }
        }
        MultivariateSpec::Pca { .. } | MultivariateSpec::CorrelationMatrix { .. } => {}
    }
    Ok(())
}

fn apply_nonlinear_custom_overrides(spec: &mut NonlinearFitSpec, norm: &Value) -> Result<(), EngineError> {
    apply_enum_field(norm, "weighting", &mut spec.weighting, "`weighting`")?;
    apply_enum_field(norm, "weighting_source", &mut spec.weighting_source, "`weighting_source`")?;
    apply_enum_field(norm, "outlier_handling", &mut spec.outlier_handling, "`outlier_handling`")?;
    apply_enum_field(norm, "replicate_fit_mode", &mut spec.replicate_fit_mode, "`replicate_fit_mode`")?;
    apply_enum_field(norm, "convergence_strictness", &mut spec.convergence_strictness, "`convergence_strictness`")?;
    if let Some(m_val) = norm.get("method")
        && !m_val.is_null()
    {
        spec.method = parse_fit_method_value(m_val)?;
    }
    let q_override = match (opt_f64_checked(norm, "q_percent")?, opt_f64_checked(norm, "fdr_q_percent")?) {
        (Some(q), Some(fdr_q)) if (q - fdr_q).abs() > 1e-12 => {
            return Err(EngineError::with_code(ErrorCode::BadInput, "conflicting `q_percent` and `fdr_q_percent` values supplied"));
        }
        (Some(q), _) | (None, Some(q)) => Some(q),
        (None, None) => None,
    };
    if let Some(q) = q_override
        && let FitMethod::RoutOutlierElimination { ref mut fdr_q_percent } = spec.method
    {
        *fdr_q_percent = q;
    }
    let _ = opt_bool_checked(norm, "band_log_spacing")?;
    if spec.max_iters == 0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`max_iters` must be >= 1"));
    }
    if let Some(rules_val) = norm.get("parameter_rules") {
        spec.parameter_rules = parse_parameter_rules(rules_val)?;
    }
    if let Some(comp_val) = norm.get("comparison") {
        spec.comparison = parse_model_comparison(comp_val, norm)?;
    }
    if let Some(xr_val) = norm.get("x_range_filter") {
        spec.x_range_filter = parse_opt_range_filter(xr_val, "x_range_filter")?;
    }
    if let Some(yr_val) = norm.get("y_range_filter") {
        spec.y_range_filter = parse_opt_range_filter(yr_val, "y_range_filter")?;
    }
    apply_opt_enum_field(norm, "identifiability_mode", &mut spec.identifiability_mode, "`identifiability_mode`")?;
    apply_opt_enum_field(norm, "comparison_method", &mut spec.comparison_method, "`comparison_method`")?;
    if let Some(q_val) = norm.get("comparison_question") {
        spec.comparison_question = if q_val.is_null() { None } else { Some(deserialize_strict(q_val, "/comparison_question")?) };
    }
    if let Some(f_val) = norm.get("fit_flag_rules") {
        spec.fit_flag_rules = if f_val.is_null() { None } else { Some(deserialize_strict(f_val, "/fit_flag_rules")?) };
    }
    Ok(())
}

const CUSTOM_HANDLED_PATCH_KEYS: &[&str] = &[
    "method",
    "multiplicity",
    "tie_method",
    "reference_levels",
    "weighting",
    "weighting_source",
    "outlier_handling",
    "replicate_fit_mode",
    "convergence_strictness",
    "repeated_measures",
    "residual_covariance",
    "df_method",
    "distance",
    "linkage",
    "family",
    "solve_target",
    "comparison",
    "comparison_question",
    "comparison_method",
    "identifiability_mode",
    "fit_flag_rules",
    "parameter_rules",
    "x_range_filter",
    "y_range_filter",
    "expected_proportions",
    "x_transform",
    "y_transform",
    "error_propagation",
    "zero_anchor",
    "hundred_anchor",
    "operation",
    "parameters",
    "target_archetype",
    "derived_columns",
];

fn parse_derived_column_assignment(raw: &str) -> Result<DerivedColumnFormula, EngineError> {
    let Some((lhs, rhs)) = raw.split_once('=') else {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("derived column string `{raw}` must have `Title = formula` format")));
    };
    let title = lhs.trim();
    let formula = rhs.trim();
    if title.is_empty() || formula.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("derived column `{raw}` must specify non-empty title and formula")));
    }
    Ok(DerivedColumnFormula::new(title, formula))
}

fn parse_derived_column_object(map: &Map<String, Value>) -> Result<DerivedColumnFormula, EngineError> {
    let title = map
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "derived column entry requires non-empty string `title`"))?;
    let formula = map
        .get("formula")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "derived column entry requires non-empty string `formula`"))?;
    Ok(DerivedColumnFormula::new(title, formula))
}

fn parse_derived_columns_value(val: &Value) -> Result<Vec<DerivedColumnFormula>, EngineError> {
    match val {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => {
            if s.trim().is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![parse_derived_column_assignment(s)?])
            }
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(s) => out.push(parse_derived_column_assignment(s)?),
                    Value::Object(map) => out.push(parse_derived_column_object(map)?),
                    _ => {
                        return Err(EngineError::with_code(
                            ErrorCode::BadInput,
                            "`derived_columns` array entries must be `Title = formula` strings or `{title, formula}` objects",
                        ));
                    }
                }
            }
            Ok(out)
        }
        Value::Object(map) => {
            if map.contains_key("title") && map.contains_key("formula") {
                return Ok(vec![parse_derived_column_object(map)?]);
            }
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map {
                let title = k.trim();
                let formula = v
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "derived column map values must be non-empty formula strings"))?;
                if title.is_empty() {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "derived column title cannot be empty"));
                }
                out.push(DerivedColumnFormula::new(title, formula));
            }
            Ok(out)
        }
        _ => Err(EngineError::with_code(ErrorCode::BadInput, "`derived_columns` must be an array, object, string, or null")),
    }
}

fn patch_analysis_kind_fields(kind: &mut AnalysisKind, patch: &Value) -> Result<(), EngineError> {
    let Value::Object(patch_map) = patch else {
        return Ok(());
    };
    let mut current_json = serde_json::to_value(&*kind)
        .map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed serializing analysis kind: {err}")))?;
    let mut patched_any = false;
    if let Value::Object(ref mut variant_map) = current_json
        && variant_map.len() == 1
        && let Some((outer_key, inner)) = variant_map.iter_mut().next()
        && let Value::Object(inner_map) = inner
    {
        let target_map = if matches!(outer_key.as_str(), "Transform" | "OutlierDetection" | "Survival" | "Multivariate")
            && inner_map.len() == 1
            && let Some((_, Value::Object(nested_map))) = inner_map.iter_mut().next()
        {
            nested_map
        } else {
            inner_map
        };
        for (k, v) in patch_map {
            let ks = k.as_str();
            if CUSTOM_HANDLED_PATCH_KEYS.contains(&ks) {
                continue;
            }
            if !target_map.contains_key(ks) && !is_omitted_optional_field(kind, ks) {
                continue;
            }
            let is_nullable = NULLABLE_KIND_FIELDS.contains(&ks)
                && !matches!(kind, AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { .. }) if ks == "sample_size")
                && !matches!(kind, AnalysisKind::GeneralizedLinearModel(_) if ks == "response_column");
            if v.is_null() {
                if !is_nullable {
                    if STRICT_NON_NULL_USIZE_FIELDS.contains(&ks) {
                        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{ks}` must be a non-negative integer")));
                    }
                    continue;
                }
                target_map.insert(k.clone(), Value::Null);
                patched_any = true;
                continue;
            }
            let coerced = if INTEGER_OR_INT_VEC_FIELDS.contains(&ks) && !matches!(kind, AnalysisKind::PowerAnalysis(_) if ks == "sample_size") {
                coerce_whole_int_value(v, ks)?
            } else {
                v.clone()
            };
            target_map.insert(k.clone(), coerced);
            patched_any = true;
        }
    }
    if patched_any {
        let updated: AnalysisKind = serde_json::from_value(current_json)
            .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid analysis parameter patch: {err}")))?;
        *kind = updated;
    }
    Ok(())
}

fn two_group_kind(method: TwoGroupMethod) -> AnalysisKind {
    AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..TwoGroupSpec::default() })
}

fn default_one_way_anova(method: OneWayMethod) -> OneWayAnovaSpec {
    OneWayAnovaSpec { method, ..OneWayAnovaSpec::default() }
}

fn default_factorial_anova(three_way: bool) -> FactorialAnovaSpec {
    FactorialAnovaSpec {
        three_way,
        max_interaction_order: if three_way { 3 } else { 2 },
        ss_type: SumOfSquaresType::TypeIII,
        repeated_measures: RepeatedMeasuresDesign::None,
        ..FactorialAnovaSpec::default()
    }
}

fn default_km_survival() -> SurvivalAnalysisSpec {
    SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox: true,
        gehan_breslow_wilcoxon: false,
        trend_test: false,
        asymmetrical_greenwood_ci: true,
        multiplicity: MultiplicitySpec::default(),
        mv_time_column: None,
        mv_status_column: None,
        mv_group_column: None,
    }
}

fn default_multiple_imputation(seed: u64) -> MultivariateSpec {
    MultivariateSpec::MultipleImputation { imputations_m: 5, max_iters: 10, seed, response_column: None, predictor_columns: Vec::new() }
}

fn parse_target_table_kind(val: &Value) -> Result<TableKind, EngineError> {
    if let Ok(parsed) = serde_json::from_value::<TableKind>(val.clone()) {
        return Ok(parsed);
    }
    let raw = val.as_str().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`target_archetype` must be a valid TableKind"))?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "xy" => Ok(TableKind::Xy),
        "column" => Ok(TableKind::Column),
        "grouped" => Ok(TableKind::Grouped),
        "contingency" => Ok(TableKind::Contingency),
        "survival" => Ok(TableKind::Survival),
        "parts_of_whole" | "partsofwhole" => Ok(TableKind::PartsOfWhole),
        "multiple_variables" | "multiplevariables" | "mv" => Ok(TableKind::MultipleVariables),
        "nested" => Ok(TableKind::Nested),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid `target_archetype`: `{other}`"))),
    }
}

fn parse_reference_levels(val: &Value) -> Result<Vec<(usize, String)>, EngineError> {
    let arr = val
        .as_array()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`reference_levels` must be an array of `[column_index, level]` pairs"))?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        if let Some(pair) = item.as_array() {
            let [col_val, lvl_val] = pair.as_slice() else {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("`reference_levels[{idx}]` must be a `[column_index, level]` pair")));
            };
            let col = parse_usize_json_value(col_val, "reference_levels column")?;
            let lvl = lvl_val
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`reference_levels[{idx}][1]` must be a non-empty string")))?;
            out.push((col, lvl.to_owned()));
        } else if let Some(obj) = item.as_object() {
            let col_val = obj
                .get("column")
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`reference_levels[{idx}]` missing `column` index")))?;
            let col = parse_usize_json_value(col_val, "reference_levels column")?;
            let lvl = obj
                .get("level")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`reference_levels[{idx}]` missing non-empty `level`")))?;
            out.push((col, lvl.to_owned()));
        } else {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("`reference_levels[{idx}]` must be `[column_index, level]` or `{{ column, level }}`"),
            ));
        }
    }
    Ok(out)
}

fn default_glm(family: GlmFamily) -> GlmSpec {
    GlmSpec { family, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: true }
}

fn default_pca() -> MultivariateSpec {
    MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(42), pcr_outcome_column: None }
}

fn parse_contingency_method_value(val: &Value) -> Result<ContingencyMethod, EngineError> {
    if val.is_object() {
        return deserialize_strict(val, "");
    }
    if let Some(raw) = val.as_str() {
        match raw.trim().to_ascii_lowercase().as_str() {
            "pearson" | "chi_square" | "pearson_chi_square" => return Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: false }),
            "yates" | "yates_chi_square" => return Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: true }),
            "cohen_kappa" | "kappa" | "cohen_kappa_unweighted" => {
                return Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted });
            }
            "cohen_kappa_linear" => return Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Linear }),
            "cohen_kappa_quadratic" => return Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic }),
            _ => {}
        }
    }
    parse_serde_enum(val, "contingency `method`")
}

fn parse_fit_method_value(val: &Value) -> Result<FitMethod, EngineError> {
    if val.is_object() {
        return deserialize_strict(val, "");
    }
    if val.as_str().is_some_and(|s| s.trim().eq_ignore_ascii_case("rout")) {
        return Ok(FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 });
    }
    parse_serde_enum(val, "fit `method`")
}

fn parse_opt_expected_proportions(val: &Value) -> Result<Option<Vec<f64>>, EngineError> {
    if val.is_null() {
        return Ok(None);
    }
    let arr = val
        .as_array()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`expected_proportions` must be an array of non-negative numbers or null"))?;
    if arr.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`expected_proportions` must not be empty"));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let num = item
            .as_f64()
            .filter(|x| x.is_finite() && *x >= 0.0)
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`expected_proportions[{idx}]` must be finite and >= 0")))?;
        out.push(num);
    }
    let sum: f64 = out.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`expected_proportions` must have positive sum"));
    }
    Ok(Some(out))
}

fn parse_p_value_format(val: &Value) -> Result<PValueFormat, EngineError> {
    if let Ok(pv) = serde_json::from_value::<PValueFormat>(val.clone()) {
        return Ok(pv);
    }
    match val.as_str().map(str::to_ascii_lowercase).as_deref() {
        Some("four_decimal" | "numeric" | "digits" | "decimal") => Ok(PValueFormat::FourDecimal),
        Some("apa") => Ok(PValueFormat::Apa),
        Some("nejm") => Ok(PValueFormat::Nejm),
        Some("scientific") => Ok(PValueFormat::new_custom(4, Some(1.0))),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, "invalid `p_value_format`")),
    }
}

fn single_char(raw: &str, name: &str) -> Result<char, EngineError> {
    let mut chars = raw.chars();
    let ch = chars.next().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`{name}` cannot be empty")))?;
    if chars.next().is_some() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{name}` must be a single character")));
    }
    Ok(ch)
}

fn opt_field_checked<'a, T>(
    params: &'a Value,
    key: &str,
    expected: &str,
    extract: impl FnOnce(&'a Value) -> Option<T>,
) -> Result<Option<T>, EngineError> {
    let Some(val) = params.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    extract(val).map(Some).ok_or_else(|| bad_input(format!("`{key}` must be {expected} or null")))
}

fn opt_str_checked<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, EngineError> {
    opt_field_checked(params, key, "a string", Value::as_str)
}

fn opt_bool_checked(params: &Value, key: &str) -> Result<Option<bool>, EngineError> {
    opt_field_checked(params, key, "a boolean", Value::as_bool)
}

fn opt_f64_checked(params: &Value, key: &str) -> Result<Option<f64>, EngineError> {
    opt_field_checked(params, key, "finite number", |v| v.as_f64().filter(|x| x.is_finite()))
}

fn parse_simulate_parameters_override(val: &Value) -> Result<Vec<(String, f64)>, EngineError> {
    if let Some(obj) = val.as_object() {
        let mut vec = Vec::with_capacity(obj.len());
        for (k, v) in obj {
            let num = v
                .as_f64()
                .filter(|x| x.is_finite())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("parameter `{k}` must be numeric")))?;
            vec.push((k.clone(), num));
        }
        return Ok(vec);
    }
    if val.is_array() {
        let pairs: Vec<(String, f64)> = serde_json::from_value(val.clone()).map_err(|_| {
            EngineError::with_code(
                ErrorCode::BadInput,
                "`parameters` must be an object of numeric parameter overrides or an array of `[name, value]` pairs",
            )
        })?;
        for (idx, (name, num)) in pairs.iter().enumerate() {
            if name.trim().is_empty() {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("`parameters[{idx}]` name must be non-empty")));
            }
            if !num.is_finite() {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("parameter `{name}` must be finite")));
            }
        }
        return Ok(pairs);
    }
    Err(EngineError::with_code(
        ErrorCode::BadInput,
        "`parameters` must be an object of numeric parameter overrides or an array of `[name, value]` pairs",
    ))
}

fn apply_bool_opt_checked(params: &Value, key: &str, target: &mut bool) -> Result<(), EngineError> {
    if let Some(b) = opt_bool_checked(params, key)? {
        *target = b;
    }
    Ok(())
}

fn apply_usize_field(params: &Value, key: &str, target: &mut usize) -> Result<(), EngineError> {
    let Some(v) = params.get(key) else {
        return Ok(());
    };
    if v.is_null() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{key}` must be a non-negative integer")));
    }
    *target = parse_usize_json_value(v, key)?;
    Ok(())
}

fn parse_nullable_finite_f64(val: &Value, key: &str) -> Result<Option<f64>, EngineError> {
    if val.is_null() {
        return Ok(None);
    }
    let num = val
        .as_f64()
        .filter(|x| x.is_finite())
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`{key}` must be finite number or null")))?;
    Ok(Some(num))
}

fn parse_opt_range_filter(val: &Value, key: &str) -> Result<Option<AxisRangeFilter>, EngineError> {
    if val.is_null() {
        return Ok(None);
    }
    if let Some(arr) = val.as_array() {
        let [min_raw, max_raw] = arr.as_slice() else {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{key}` array must have [min, max] entries")));
        };
        let min_v = parse_nullable_finite_f64(min_raw, key)?;
        let max_v = parse_nullable_finite_f64(max_raw, key)?;
        return Ok(Some((min_v, max_v)));
    }
    if let Some(obj) = val.as_object() {
        for k in obj.keys() {
            if !matches!(k.as_str(), "min" | "max") {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown field `{k}` in `{key}`")));
            }
        }
        let min_v = val.get("min").map(|v| parse_nullable_finite_f64(v, key)).transpose()?.flatten();
        let max_v = val.get("max").map(|v| parse_nullable_finite_f64(v, key)).transpose()?.flatten();
        return Ok(Some((min_v, max_v)));
    }
    Err(EngineError::with_code(ErrorCode::BadInput, format!("`{key}` must be null, [min, max], or {{ min?, max? }}")))
}

fn parse_model_comparison(val: &Value, params: &Value) -> Result<Option<ModelComparisonSpec>, EngineError> {
    if val.is_null() {
        return Ok(None);
    }
    if let Some(obj) = val.as_object()
        && !obj.contains_key("method")
    {
        return deserialize_strict(val, "/comparison").map(Some);
    }
    let (method_raw, alt_id) = if let Some(s) = val.as_str() {
        let alt = opt_str_checked(params, "alternative_equation_id")?
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`comparison` shorthand string requires `alternative_equation_id`"))?;
        (s, alt.to_owned())
    } else if let Some(obj) = val.as_object() {
        for k in obj.keys() {
            if !matches!(k.as_str(), "method" | "alternative_equation_id") {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown field `{k}` in `comparison`")));
            }
        }
        let m = opt_str_checked(val, "method")?.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`comparison` object missing `method`"))?;
        let alt = match opt_str_checked(val, "alternative_equation_id")? {
            Some(s) => Some(s),
            None => opt_str_checked(params, "alternative_equation_id")?,
        }
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`comparison` object missing `alternative_equation_id`"))?;
        (m, alt.to_owned())
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`comparison` must be null, string, or comparison object"));
    };

    let spec = match method_raw.trim().to_ascii_lowercase().as_str() {
        "extra_sum_of_squares_f" | "f_test" | "f" => ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: alt_id },
        "aicc" | "aic" => ModelComparisonSpec::Aicc { alternative_equation_id: alt_id },
        "likelihood_ratio_test" | "lrt" | "likelihood_ratio" => ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: alt_id },
        other => return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown model comparison method `{other}`"))),
    };
    Ok(Some(spec))
}

fn parse_parameter_rules(val: &Value) -> Result<Vec<ParameterRule>, EngineError> {
    let arr = val.as_array().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`parameter_rules` must be an array"))?;
    let mut rules = Vec::with_capacity(arr.len());
    for item in arr {
        rules.push(parse_parameter_rule(item)?);
    }
    Ok(rules)
}

fn is_shorthand_parameter_rule_obj(map: &Map<String, Value>) -> bool {
    map.contains_key("value")
        || map.contains_key("values")
        || map.contains_key("hook")
        || map.get("scope").and_then(Value::as_str).is_some_and(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "local"
                    | "shared"
                    | "shared_global"
                    | "column_title_constant"
                    | "column_title"
                    | "constant"
                    | "per_dataset_constant"
                    | "per_dataset"
                    | "hooked_constant"
                    | "hooked"
            )
        })
}

fn parse_parameter_rule(item: &Value) -> Result<ParameterRule, EngineError> {
    let Value::Object(map) = item else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "each `parameter_rules` item must be an object"));
    };
    for k in map.keys() {
        if !matches!(k.as_str(), "name" | "scope" | "initial_override" | "lower_bound" | "upper_bound" | "value" | "values" | "hook") {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown field `{k}` in `parameter_rules` entry")));
        }
    }
    if !is_shorthand_parameter_rule_obj(map) {
        return deserialize_strict(item, "/parameter_rules");
    }
    let name =
        opt_str_checked(item, "name")?.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "parameter rule missing string `name`"))?.to_owned();
    let scope = parse_parameter_scope(item)?;
    let initial_override = item.get("initial_override").map(|v| parse_nullable_finite_f64(v, "initial_override")).transpose()?.flatten();
    let lower_bound = item.get("lower_bound").map(|v| parse_nullable_finite_f64(v, "lower_bound")).transpose()?.flatten();
    let upper_bound = item.get("upper_bound").map(|v| parse_nullable_finite_f64(v, "upper_bound")).transpose()?.flatten();
    Ok(ParameterRule { name, scope, initial_override, lower_bound, upper_bound })
}

fn parse_parameter_scope(item: &Value) -> Result<ParameterScope, EngineError> {
    let Some(scope_val) = item.get("scope") else {
        return Ok(ParameterScope::Local);
    };
    if scope_val.is_object() {
        return deserialize_strict(scope_val, "/parameter_rules/scope");
    }
    if let Ok(parsed) = serde_json::from_value::<ParameterScope>(scope_val.clone()) {
        return Ok(parsed);
    }
    let Some(raw) = scope_val.as_str() else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "invalid parameter rule `scope`"));
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "local" => Ok(ParameterScope::Local),
        "shared" | "shared_global" => Ok(ParameterScope::SharedGlobal),
        "column_title_constant" | "column_title" => Ok(ParameterScope::ColumnTitleConstant),
        "constant" => {
            let val = item
                .get("value")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`constant` scope requires finite numeric `value`"))?;
            Ok(ParameterScope::Constant(val))
        }
        "per_dataset_constant" | "per_dataset" => {
            let src =
                item.get("values").ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`per_dataset_constant` scope requires `values`"))?;
            Ok(ParameterScope::PerDatasetConstant(parse_per_dataset_constants(src)?))
        }
        "hooked_constant" | "hooked" => {
            let hook_val = item.get("hook").ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`hooked_constant` scope requires `hook`"))?;
            let hook: HookSource = deserialize_strict(hook_val, "/parameter_rules/hook")?;
            Ok(ParameterScope::HookedConstant(hook))
        }
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown parameter scope `{other}`"))),
    }
}

fn parse_per_dataset_constants(val: &Value) -> Result<Vec<(usize, f64)>, EngineError> {
    if let Some(pairs) = val.as_array() {
        let mut out = Vec::with_capacity(pairs.len());
        for (idx, entry) in pairs.iter().enumerate() {
            if let Some(pair) = entry.as_array() {
                let [ds_raw, num_raw] = pair.as_slice() else {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "per-dataset constant pair must have [dataset_index, value]"));
                };
                let ds = parse_usize_json_value(ds_raw, "dataset_index")?;
                let num = num_raw
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "per-dataset constant value must be finite number"))?;
                out.push((ds, num));
            } else {
                let num = entry
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "per-dataset constant value must be finite number"))?;
                out.push((idx, num));
            }
        }
        return Ok(out);
    }
    if let Some(obj) = val.as_object() {
        let mut out = Vec::with_capacity(obj.len());
        for (k, v) in obj {
            let ds = k.parse::<usize>().map_err(|_| EngineError::with_code(ErrorCode::BadInput, format!("invalid dataset index `{k}`")))?;
            let num = v
                .as_f64()
                .filter(|x| x.is_finite())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "per-dataset constant value must be finite number"))?;
            out.push((ds, num));
        }
        return Ok(out);
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "`per_dataset_constant` values must be array or map"))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, ProjectDocument, TableArchetype};
    use serde_json::json;

    use super::*;
    use crate::Session;

    fn session_with_col_data() -> Session {
        let mut doc = ProjectDocument::new("Analysis Unit Test").unwrap();
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
        table.data_sets[0].title = String::from("Ctrl");
        table.data_sets[1].title = String::from("Drug");
        for (r, &v) in [10.0, 11.0, 12.0, 10.5].iter().enumerate() {
            table.data_sets[0].cells[r][0] = Cell::number(v);
        }
        for (r, &v) in [20.0, 21.0, 22.0, 20.5].iter().enumerate() {
            table.data_sets[1].cells[r][0] = Cell::number(v);
        }
        let ds = DataSheet::new("col1", "Groups", TableArchetype::Column(table)).unwrap();
        doc.add_data_sheet(ds).unwrap();
        Session::from_document(doc)
    }

    #[test]
    fn registry_contains_all_twelve_analysis_commands() {
        assert_eq!(specs().len(), 12);
        assert!(specs().iter().any(|s| s.id == "stats.pvalueStack"));
        for spec in specs() {
            assert_eq!(spec.group, CommandGroup::Analysis);
            assert!(!spec.summary.is_empty());
            assert!(!spec.beginner.is_empty());
        }
    }

    #[test]
    fn runs_stats_generates_atomic_report_and_enforces_errors() {
        let mut session = session_with_col_data();
        let res = session
            .execute("stats.run", json!({ "source_sheet_id": "col1", "sheet_id": "t1", "kind": "unpaired_t" }))
            .expect("unpaired t-test must succeed");
        assert!(res.mutated);
        assert_eq!(res.payload["sheet_id"], json!("t1"));

        let out_dir = std::env::temp_dir().join(format!("cb_ana_test_{}", std::process::id()));
        let report_path = out_dir.join("methods.md");
        let rep = session
            .execute("result.generateReport", json!({ "result_sheet_id": "t1", "format": "markdown", "path": report_path.to_string_lossy() }))
            .expect("report write must succeed");
        assert!(report_path.exists());
        assert_eq!(rep.payload["format"], json!("markdown"));
        let _ = std::fs::remove_dir_all(&out_dir);

        let bad_fmt = session
            .execute("result.generateReport", json!({ "result_sheet_id": "t1", "format": "docx" }))
            .expect_err("unknown report format must fail");
        assert_eq!(bad_fmt.code, ErrorCode::BadInput);

        let empty_analyze = Session::new().execute("analyze.run", json!({})).expect("analyze.run works on empty session");
        assert_eq!(empty_analyze.payload["recalculated_sheets"], json!(0));
    }

    #[test]
    fn update_params_multiplicity_preserves_threshold_and_aligns_ci_with_confidence() {
        let mut doc = ProjectDocument::new("RV-65 Engine Post-Hoc").unwrap();
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).unwrap();
        for (idx, (title, vals)) in
            [("Ctrl", [10.0, 11.0, 10.5, 9.5]), ("Low", [13.0, 14.0, 13.5, 12.5]), ("High", [18.0, 19.0, 18.5, 17.5])].into_iter().enumerate()
        {
            table.data_sets[idx].title = String::from(title);
            for (r, v) in vals.into_iter().enumerate() {
                table.data_sets[idx].cells[r][0] = Cell::number(v);
            }
        }
        doc.add_data_sheet(DataSheet::new("col3", "Three Groups", TableArchetype::Column(table)).unwrap()).unwrap();
        let mut session = Session::from_document(doc);

        session
            .execute(
                "stats.run",
                json!({
                    "source_sheet_id": "col3",
                    "sheet_id": "anova1",
                    "kind": "one_way_anova",
                    "confidence_percent": 95.0,
                    "multiplicity": "tukey",
                    "alpha": 0.01,
                    "report_confidence_intervals": false
                }),
            )
            .expect("initial one-way ANOVA must succeed");

        // Updating `multiplicity` by string name must preserve existing `threshold = 0.01` and `report_confidence_intervals = false`.
        session
            .execute("analysis.updateParams", json!({ "sheet_id": "anova1", "params": { "multiplicity": "bonferroni" } }))
            .expect("multiplicity method update must succeed");

        let anova_id = SheetId::from("anova1");
        let spec = session.document().result_sheet(&anova_id).expect("result sheet exists").analysis.clone();
        assert!(matches!(spec.kind, AnalysisKind::OneWayAnova(_)));
        if let AnalysisKind::OneWayAnova(ref ow) = spec.kind {
            assert_eq!(ow.multiplicity.method, MultiplicityMethod::Bonferroni);
            assert!((ow.multiplicity.threshold - 0.01).abs() < 1e-12);
            assert!(!ow.multiplicity.report_confidence_intervals);
        }

        // Re-enable CIs and verify 95% vs 99% confidence updates CI width while keeping threshold = 0.01 intact.
        session
            .execute(
                "analysis.updateParams",
                json!({ "sheet_id": "anova1", "confidence_percent": 99.0, "params": { "report_confidence_intervals": true } }),
            )
            .expect("confidence update must succeed");

        let res99 = session.document().result_sheet(&anova_id).expect("result sheet exists");
        let mc_tab = res99.tabs.iter().find(|t| t.id == "multiple-comparisons").expect("post-hoc tab present");
        assert!(mc_tab.column_headers.iter().any(|h| h.contains("99% CI")), "headers must advertise 99% CI: {:?}", mc_tab.column_headers);
    }

    #[test]
    fn xy_bland_altman_and_glm_parameter_validation_matches_stats_contract() {
        use curvebook_doc::SubcolumnMode;

        let mut doc = ProjectDocument::new("XY BA and GLM").unwrap();

        let mut xy1 = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
        let mut xy2 = XyTable::new(SubcolumnMode::Single, 2, 5).unwrap();
        for r in 0..5 {
            let x = (r + 1) as f64;
            xy1.x_cells[r][0] = Cell::number(x);
            xy1.data_sets[0].cells[r][0] = Cell::number(x + 0.5 * ((r % 2) as f64));
            xy2.x_cells[r][0] = Cell::number(x);
            xy2.data_sets[0].cells[r][0] = Cell::number(1.0 + 2.0 * x + ((r % 2) as f64) * 0.1);
            xy2.data_sets[1].cells[r][0] = Cell::number(10.0 + 3.0 * x - ((r % 2) as f64) * 0.1);
        }
        let xy_sum = XyTable::new(SubcolumnMode::MeanSdN, 1, 5).unwrap();

        doc.add_data_sheet(DataSheet::new("xy1", "Single XY", TableArchetype::Xy(xy1)).unwrap()).unwrap();
        doc.add_data_sheet(DataSheet::new("xy2", "Multi XY", TableArchetype::Xy(xy2)).unwrap()).unwrap();
        doc.add_data_sheet(DataSheet::new("xysum", "Summary XY", TableArchetype::Xy(xy_sum)).unwrap()).unwrap();
        let mut session = Session::from_document(doc);

        // Audit #4: Bland-Altman on 1-dataset XY keeps (0, 1) valid and rejects invalid pairs.
        session
            .execute("analysis.run", json!({ "source_sheet_id": "xy1", "sheet_id": "ba1", "kind": "bland_altman" }))
            .expect("single-dataset XY Bland-Altman (0, 1) must succeed");
        for bad in [json!({"column_a": 99, "column_b": 99}), json!({"column_a": 0, "column_b": 0}), json!({"column_a": 0, "column_b": 2})] {
            let mut req = json!({ "source_sheet_id": "xy1", "kind": "bland_altman" });
            req.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
            let err = session.execute("analysis.run", req).expect_err("invalid single-XY Bland-Altman must fail");
            assert_eq!(err.code, ErrorCode::BadInput);
        }

        // Audit #4: Bland-Altman on 2-dataset XY accepts (0, 1) and rejects equal or out-of-range indices.
        session
            .execute("analysis.run", json!({ "source_sheet_id": "xy2", "sheet_id": "ba2", "kind": "bland_altman", "column_a": 0, "column_b": 1 }))
            .expect("2-dataset XY Bland-Altman (0, 1) must succeed");
        for bad in [json!({"column_a": 1, "column_b": 1}), json!({"column_a": 0, "column_b": 2})] {
            let mut req = json!({ "source_sheet_id": "xy2", "kind": "bland_altman" });
            req.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
            let err = session.execute("analysis.run", req).expect_err("invalid multi-XY Bland-Altman must fail");
            assert_eq!(err.code, ErrorCode::BadInput);
        }

        // Audit #13: XY GLM rejects pre-summarized XY tables and invalid column selections.
        let sum_err = session
            .execute("analysis.run", json!({ "source_sheet_id": "xysum", "kind": "linear_glm" }))
            .expect_err("pre-summarized XY GLM must fail");
        assert_eq!(sum_err.code, ErrorCode::BadInput);

        session
            .execute(
                "analysis.run",
                json!({ "source_sheet_id": "xy2", "sheet_id": "glm-ok", "kind": "linear_glm", "selected_data_sets": [1], "response_column": 1, "predictor_columns": [0] }),
            )
            .expect("valid XY GLM with dataset 1 and X predictor 0 must succeed");
        session
            .execute("analysis.run", json!({ "source_sheet_id": "xy2", "sheet_id": "glm-default", "kind": "linear_glm", "selected_data_sets": [1] }))
            .expect("default_glm placeholder [1] + selected_data_sets [1] must remain valid");

        let bad_pred = session
            .execute(
                "analysis.run",
                json!({ "source_sheet_id": "xy2", "kind": "linear_glm", "selected_data_sets": [1], "response_column": 1, "predictor_columns": [99] }),
            )
            .expect_err("impossible predictor [99] on XY GLM must fail");
        assert_eq!(bad_pred.code, ErrorCode::BadInput);

        let empty_pred = session
            .execute("analysis.run", json!({ "source_sheet_id": "xy2", "kind": "linear_glm", "predictor_columns": [] }))
            .expect_err("empty predictor_columns must fail with InsufficientData");
        assert_eq!(empty_pred.code, ErrorCode::InsufficientData);

        let mismatch_resp = session
            .execute("analysis.run", json!({ "source_sheet_id": "xy2", "kind": "linear_glm", "selected_data_sets": [0], "response_column": 1 }))
            .expect_err("mismatched response_column must fail");
        assert_eq!(mismatch_resp.code, ErrorCode::BadInput);

        let update_err = session
            .execute("analysis.updateParams", json!({ "sheet_id": "glm-ok", "params": { "predictor_columns": [99] } }))
            .expect_err("updateParams with invalid predictor_columns must fail cleanly");
        assert_eq!(update_err.code, ErrorCode::BadInput);
    }

    #[test]
    fn all_descriptor_shorthands_resolve_and_validate_domain_and_table_compat() {
        use curvebook_doc::{ALL_ANALYSIS_DESCRIPTORS, AnalysisDomainKind, AnalysisKindId, descriptor_by_shorthand};

        let empty = json!({});
        for desc in &ALL_ANALYSIS_DESCRIPTORS {
            let expected_domain = match desc.domain {
                AnalysisDomainKind::Transform => AnalysisDomain::TransformOnly,
                AnalysisDomainKind::Stats => AnalysisDomain::StatsOnly,
                AnalysisDomainKind::Fit => AnalysisDomain::FitOnly,
            };
            assert_eq!(domain_prefix(expected_domain), desc.domain.default_sheet_prefix());

            for &shorthand in desc.shorthands {
                assert_eq!(descriptor_by_shorthand(shorthand).map(|d| d.id), Some(desc.id), "descriptor lookup mismatch for `{shorthand}`");
                let resolved =
                    resolve_kind_shorthand(shorthand, &empty).unwrap_or_else(|err| panic!("shorthand `{shorthand}` failed to resolve: {err}"));
                assert_eq!(resolved.id(), desc.id, "AnalysisKindId mismatch for `{shorthand}`");
                assert_eq!(resolved.descriptor().domain, desc.domain, "domain mismatch for `{shorthand}`");

                for tk in TableKind::ALL {
                    let compat_res = validate_domain_and_compat(&resolved, expected_domain, tk);
                    let any_res = validate_domain_and_compat(&resolved, AnalysisDomain::Any, tk);
                    assert_eq!(compat_res.is_ok(), resolved.is_compatible_with(tk));
                    assert_eq!(any_res.is_ok(), resolved.is_compatible_with(tk));
                    if let Err(err) = compat_res {
                        assert_eq!(err.code, ErrorCode::Conflict);
                    }
                    for wrong_domain in [AnalysisDomain::TransformOnly, AnalysisDomain::StatsOnly, AnalysisDomain::FitOnly] {
                        if wrong_domain != expected_domain {
                            let err = validate_domain_and_compat(&resolved, wrong_domain, tk).expect_err("wrong domain must fail");
                            assert_eq!(err.code, ErrorCode::BadInput);
                        }
                    }
                }
            }
        }

        let eq_kind = resolve_kind_shorthand("one_phase_decay", &empty).expect("built-in equation ID must resolve");
        assert_eq!(eq_kind.id(), AnalysisKindId::NonlinearRegression);
        assert!(eq_kind.descriptor().accepts_equation_id_shorthand);
    }
}
