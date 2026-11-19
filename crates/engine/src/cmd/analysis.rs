//! Scientific transform, statistical test, curve fit, and result reporting/export commands.

use std::collections::HashSet;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, BlandAltmanSpec, CellValue, ClusterDistance, ClusterLinkage, ContingencyMethod, ContingencySpec,
    CoxTieMethod, DataSheet, DerivedColumnFormula, DescriptiveSpec, ErrorCode, ErrorPropagationMode, FactorialAnovaSpec, FitMethod, GlmFamily,
    GlmSpec, HookSource, KappaWeighting, LinearRegressionSpec, MathTransformPreset, MixedModelDfMethod, ModelComparisonSpec, MultipleRowTestsSpec,
    MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor,
    OneWayAnovaSpec, OneWayMethod, OutlierSpec, PValueStackMethod, PValueStackSpec, ParameterRule, ParameterScope, PassingBablokSpec,
    PowerAnalysisSpec, PowerEffectSpec, PowerOneSidedDirection, PowerSolveTarget, PowerTailMode, PowerTestFamily, ProjectDocument,
    RepeatedMeasuresDesign, ResidualCovariance, ResultSheet, ResultStatus, RocCiMethod, RocSpec, SheetId, SimulationNoise, SumOfSquaresType,
    SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableColumn, VariableRole,
    WeightingScheme, XyTable,
};
use curvebook_fit::{get_builtin_model, resolve_model_from_spec};
use curvebook_format::{CsvOptions, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv};
use curvebook_math::PValueFormat;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::export::{de_opt_usize, write_atomic_output_file};
use super::helpers::{next_unique_sheet_id, parse_sheet_id, require_analyzable_data_sheet, require_result_sheet};
use super::params::{
    any_json_schema, array_schema, bool_schema, deserialize_strict, int_schema, non_empty_str_schema, nullable_schema, num_schema,
    obj_schema_with_aliases, parse_params, str_schema,
};
use super::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::dag;
use crate::methods_report::{ReportFormat, generate_methods_report};
use crate::{CommandOutcome, EngineError, Session};

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

static SPECS: &[CommandSpec] = &[
    CommandSpec {
        id: "analyze.run",
        label: "Run Analyses",
        menu_path: "Analyze/Run Analyses",
        shortcut: "Cmd+Enter",
        summary: "Recalculate all dirty or upstream-dependent analysis result sheets across the project DAG.",
        beginner: "Refreshes every analysis sheet so all tables, stats, and curve fits match the latest data.",
        group: CommandGroup::Analysis,
        effects: MUTATE_AND_IDEMPOTENT,
        input_schema: schema_analyze_run,
        enabled: always_enabled,
        run: run_analyze_run,
    },
    CommandSpec {
        id: "transform.run",
        label: "Run Transformation",
        menu_path: "Analyze/Transform...",
        shortcut: "",
        summary: "Create and execute a mathematical, normalization, baseline, transpose, prune, select, or simulation transformation sheet.",
        beginner: "Runs a reproducible data transformation pipeline on a data sheet and writes the output to a linked result sheet.",
        group: CommandGroup::Analysis,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_transform_run,
        enabled: has_data_sheet,
        run: run_transform_run,
    },
    CommandSpec {
        id: "stats.run",
        label: "Run Statistical Test",
        menu_path: "Analyze/Statistical Test...",
        shortcut: "",
        summary: "Create and execute a hypothesis test, ANOVA, contingency, survival, ROC, GLM, or multivariate analysis.",
        beginner: "Runs a statistical test on a compatible data sheet and stores tables, P values, and assumption checks.",
        group: CommandGroup::Analysis,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_stats_run,
        enabled: has_data_sheet,
        run: run_stats_run,
    },
    CommandSpec {
        id: "fit.run",
        label: "Run Curve Fit / Regression",
        menu_path: "Analyze/Fit Curve...",
        shortcut: "",
        summary: "Create and execute linear, Deming, built-in nonlinear, global shared-parameter, or custom DSL regression.",
        beginner: "Fits a linear or nonlinear dose-response/kinetic curve with confidence bands and model comparisons.",
        group: CommandGroup::Analysis,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_fit_run,
        enabled: has_data_sheet,
        run: run_fit_run,
    },
    CommandSpec {
        id: "analysis.add",
        label: "Add Analysis",
        menu_path: "Analyze/Add Analysis...",
        shortcut: "",
        summary: "Attach and compute a new analysis of any domain (transform, stats, or regression) for a data sheet.",
        beginner: "Adds a new live analysis sheet linked to an input table and computes its initial results.",
        group: CommandGroup::Analysis,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_analysis_run,
        enabled: has_data_sheet,
        run: run_analysis_add,
    },
    CommandSpec {
        id: "analysis.run",
        label: "New / Run Analysis",
        menu_path: "Analyze/Analyze Data...",
        shortcut: "",
        summary: "Create and run a new analysis sheet, recalculate an existing result sheet by ID, or recalculate the project when empty.",
        beginner: "Runs a new analysis when source and kind are provided, or refreshes an existing analysis sheet.",
        group: CommandGroup::Analysis,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_analysis_run,
        enabled: always_enabled,
        run: run_analysis_run,
    },
    CommandSpec {
        id: "analysis.updateParams",
        label: "Change Analysis Parameters",
        menu_path: "Change/Analysis Parameters...",
        shortcut: "",
        summary: "Update analysis kind, confidence level, dataset selection, P-value formatting, or parameter rules and recalculate.",
        beginner: "Modifies settings on an existing result sheet without recreating it and refreshes downstream sheets.",
        group: CommandGroup::Analysis,
        effects: MUTATE_AND_IDEMPOTENT,
        input_schema: schema_analysis_update_params,
        enabled: has_result_sheet,
        run: run_analysis_update_params,
    },
    CommandSpec {
        id: "analysis.retargetData",
        label: "Change Data Analyzed",
        menu_path: "Change/Data Analyzed...",
        shortcut: "",
        summary: "Rebind an existing analysis result sheet to a different compatible data sheet or subset of datasets.",
        beginner: "Points an existing analysis at a different data table and recalculates it immediately.",
        group: CommandGroup::Analysis,
        effects: MUTATE_AND_IDEMPOTENT,
        input_schema: schema_analysis_retarget_data,
        enabled: has_result_sheet,
        run: run_analysis_retarget_data,
    },
    CommandSpec {
        id: "analysis.exportJson",
        label: "Export Result Sheet JSON",
        menu_path: "File/Export/Results JSON...",
        shortcut: "",
        summary: "Serialize a result sheet (tables, scalars, parameters, checklist) to JSON and optionally save it atomically.",
        beginner: "Exports an analysis result sheet as structured JSON text or saves it to a `.json` file.",
        group: CommandGroup::Analysis,
        effects: EXPORT_EFFECTS,
        input_schema: schema_analysis_export_json,
        enabled: has_result_sheet,
        run: run_analysis_export_json,
    },
    CommandSpec {
        id: "analysis.exportCsv",
        label: "Export Result Sheet CSV",
        menu_path: "File/Export/Results CSV...",
        shortcut: "",
        summary: "Export a specific tab or tidy combined CSV representation of a result sheet and optionally save it atomically.",
        beginner: "Exports result tables to CSV/TSV text with configurable delimiter and decimal separator.",
        group: CommandGroup::Analysis,
        effects: EXPORT_EFFECTS,
        input_schema: schema_analysis_export_csv,
        enabled: has_result_sheet,
        run: run_analysis_export_csv,
    },
    CommandSpec {
        id: "result.generateReport",
        label: "Generate Methods Report",
        menu_path: "File/Export/Methods Report...",
        shortcut: "",
        summary: "Generate a publication-ready statistical methods prose report in `plain`, `markdown`, or `latex`, optionally saving to disk.",
        beginner: "Creates a narrative methods paragraph describing the statistical test, sample sizes, and options used.",
        group: CommandGroup::Analysis,
        effects: EXPORT_EFFECTS,
        input_schema: schema_result_generate_report,
        enabled: has_result_sheet,
        run: run_result_generate_report,
    },
    CommandSpec {
        id: "stats.pvalueStack",
        label: "Analyze a Stack of P Values",
        menu_path: "Analyze/Column Analyses/Analyze a Stack of P Values...",
        shortcut: "",
        summary: "Adjust a column stack of raw P values across 7 FWER/FDR multiplicity procedures and report discoveries.",
        beginner: "Adjusts a column of P values using Bonferroni, Holm, Šídák, Holm-Šídák, Benjamini-Hochberg, BKY two-stage, or Benjamini-Yekutieli.",
        group: CommandGroup::Analysis,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_stats_pvalue_stack,
        enabled: has_data_sheet,
        run: run_stats_pvalue_stack,
    },
];

fn always_enabled(_session: &Session) -> bool {
    true
}

fn has_data_sheet(session: &Session) -> bool {
    !session.document().data_sheets.is_empty() || session.document().result_sheets.iter().any(|r| r.analyzable_data.is_some())
}

fn has_result_sheet(session: &Session) -> bool {
    !session.document().result_sheets.is_empty()
}

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

/// Parse an array of non-negative integers, tolerating exact whole floats such as `1.0`.
pub(super) fn parse_usize_list(val: Option<&Value>) -> Result<Option<Vec<usize>>, EngineError> {
    let Some(v) = val else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let arr = v.as_array().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "expected array of non-negative integers"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let idx = parse_usize_json_value(item, "array item")?;
        out.push(idx);
    }
    Ok(Some(out))
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

fn parse_opt_u64_field(params: &Value, key: &str, default_val: u64) -> Result<u64, EngineError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default_val),
        Some(v) => parse_u64_json_value(v, key),
    }
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
    #[serde(default, alias = "id")]
    sheet_id: Option<String>,
    #[serde(default, alias = "source_sheet", alias = "data_sheet_id")]
    source_sheet_id: Option<String>,
    #[serde(default)]
    spec: Option<Value>,
    #[serde(default)]
    kind: Option<Value>,
    #[serde(default)]
    overwrite: Option<bool>,
    #[serde(default, alias = "custom_title", deserialize_with = "de_present_opt")]
    title: Option<Option<String>>,
    #[serde(default, alias = "confidence_level")]
    confidence_percent: Option<f64>,
    #[serde(default, alias = "selected_columns", alias = "columns")]
    selected_data_sets: Option<Vec<Value>>,
    #[serde(default)]
    p_value_format: Option<Value>,
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
        props.push(("equation_id", nullable_schema(non_empty_str_schema("Built-in or custom equation identifier"))));
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
        props.push(("y_range_filter", nullable_schema(any_json_schema("Optional [min, max] Y inclusion filter"))));
        props.push(("identify_unstable_parameters", nullable_schema(bool_schema("Assess nonlinear parameter identifiability"))));
        props.push((
            "flag_r_squared_below",
            nullable_schema(num_schema("Warn when nonlinear R squared is below this threshold", Some(0.0), Some(1.0))),
        ));
    }
    obj_schema_with_aliases(
        props,
        &[],
        vec![
            ("sheet_id", &["id"]),
            ("source_sheet_id", &["source_sheet", "data_sheet_id"]),
            ("title", &["custom_title"]),
            ("confidence_percent", &["confidence_level"]),
            ("selected_data_sets", &["selected_columns", "columns"]),
            ("comparison", &["model_comparison"]),
        ],
        true,
    )
}

fn schema_transform_run() -> Value {
    schema_analysis_base(
        "Transform kind (`math`, `normalize`, `remove_baseline`, `transpose`, `prune_rows`, `select_and_transform`, `simulate`, `random_sample`) or typed object",
        false,
        false,
    )
}

fn schema_stats_run() -> Value {
    schema_analysis_base("Statistical test kind shorthand or typed AnalysisKind object", false, true)
}

fn schema_stats_pvalue_stack() -> Value {
    obj_schema_with_aliases(
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
        vec![
            ("sheet_id", &["id"]),
            ("source_sheet_id", &["source_sheet", "source", "data_sheet_id"]),
            ("selected_data_sets", &["selected_columns", "columns"]),
            ("threshold", &["alpha", "q", "fdr_q"]),
        ],
        true,
    )
}

fn schema_fit_run() -> Value {
    schema_analysis_base("Regression kind or equation shorthand (e.g. `four_parameter_logistic`, `linear`, `deming`) or typed object", true, true)
}

fn schema_analysis_run() -> Value {
    schema_analysis_base("Any analysis kind shorthand or typed AnalysisKind object", true, true)
}

#[derive(Debug, Deserialize)]
struct AnalysisUpdateParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(default)]
    kind: Option<Value>,
    #[serde(default, alias = "custom_title", deserialize_with = "de_present_opt")]
    title: Option<Option<String>>,
    #[serde(default, alias = "confidence_level")]
    confidence_percent: Option<f64>,
    #[serde(default, alias = "selected_columns", alias = "columns")]
    selected_data_sets: Option<Vec<Value>>,
    #[serde(default)]
    p_value_format: Option<Value>,
    #[serde(default, alias = "patch")]
    params: Option<Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn schema_analysis_update_params() -> Value {
    obj_schema_with_aliases(
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
        vec![
            ("sheet_id", &["id"]),
            ("title", &["custom_title"]),
            ("confidence_percent", &["confidence_level"]),
            ("selected_data_sets", &["selected_columns", "columns"]),
            ("params", &["patch"]),
        ],
        true,
    )
}

#[derive(Debug, Deserialize)]
struct RetargetDataParams {
    #[serde(alias = "id")]
    sheet_id: String,
    #[serde(alias = "source_sheet", alias = "data_sheet_id")]
    source_sheet_id: String,
    #[serde(default)]
    selected_data_sets: Option<Vec<Value>>,
    #[serde(default)]
    clear_selected_data_sets: Option<bool>,
}

fn schema_analysis_retarget_data() -> Value {
    obj_schema_with_aliases(
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
        vec![("sheet_id", &["id"]), ("source_sheet_id", &["source_sheet", "data_sheet_id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ExportResultJsonParams {
    #[serde(alias = "id")]
    sheet_id: String,
    pretty: Option<bool>,
    path: Option<String>,
}

fn schema_analysis_export_json() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Result sheet ID to export")),
            ("pretty", nullable_schema(bool_schema("Pretty-print JSON output (default true)"))),
            ("path", nullable_schema(non_empty_str_schema("Optional output `.json` file path"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ExportResultCsvParams {
    #[serde(alias = "id")]
    sheet_id: String,
    tab_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_usize")]
    tab_index: Option<usize>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
    path: Option<String>,
}

fn schema_analysis_export_csv() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Result sheet ID to export")),
            ("tab_id", nullable_schema(non_empty_str_schema("Optional specific result tab identifier"))),
            ("tab_index", nullable_schema(int_schema("Optional zero-based result tab index", Some(0), None))),
            ("delimiter", nullable_schema(non_empty_str_schema("Single-character CSV delimiter (default `,`)"))),
            ("decimal_separator", nullable_schema(non_empty_str_schema("Single-character decimal separator (default `.`)"))),
            ("path", nullable_schema(non_empty_str_schema("Optional output `.csv` file path"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct GenerateReportParams {
    #[serde(alias = "sheet_id", alias = "id")]
    result_sheet_id: String,
    format: Option<String>,
    path: Option<String>,
}

fn schema_result_generate_report() -> Value {
    obj_schema_with_aliases(
        vec![
            ("result_sheet_id", non_empty_str_schema("Result sheet ID to document")),
            ("format", nullable_schema(non_empty_str_schema("Report output format (`plain`, `markdown`, or `latex`)"))),
            ("path", nullable_schema(non_empty_str_schema("Optional output file path"))),
        ],
        &["result_sheet_id"],
        vec![("result_sheet_id", &["sheet_id", "id"])],
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

    Ok(CommandOutcome::mutated(command_id, format!("ran {}", res.analysis.effective_title()), result_summary_payload(res, recalculated)))
}

fn run_existing_or_project_analysis(doc: &mut ProjectDocument, command_id: &str, raw_id: Option<&str>) -> Result<CommandOutcome, EngineError> {
    if let Some(id_str) = raw_id {
        let id = parse_sheet_id(id_str)?;
        if doc.result_sheet(&id).is_none() {
            return Err(EngineError::with_code(
                ErrorCode::NotFound,
                format!("result sheet `{id}` not found; supply `source_sheet` and `kind` to create a new analysis"),
            )
            .with_remediation("Supply `source_sheet` and `kind` to create a new result sheet, or verify the existing `sheet_id`.")
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

fn run_analysis_export_json(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_analysis_export_json();
    let params: ExportResultJsonParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let res = require_result_sheet(ctx.session.document(), &id)?;
    let pretty = params.pretty.unwrap_or(true);
    let json_text = export_result_sheet_json(res, pretty)?;
    if let Some(ref p) = params.path {
        write_atomic_output_file(p, json_text.as_bytes())?;
    }
    Ok(CommandOutcome::read_only(
        "analysis.exportJson",
        format!("exported analysis `{id}` to JSON"),
        json!({ "sheet_id": id.as_str(), "byte_len": json_text.len(), "path": params.path, "json": json_text }),
    ))
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
        export_result_tab_csv(tab, &opts)
    } else if let Some(tab_index) = params.tab_index {
        let tab = res
            .tabs
            .get(tab_index)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result tab index {tab_index} out of bounds")))?;
        export_result_tab_csv(tab, &opts)
    } else {
        export_result_sheet_tidy_csv(res, &opts)
    };

    if let Some(ref p) = params.path {
        write_atomic_output_file(p, csv.as_bytes())?;
    }
    Ok(CommandOutcome::read_only(
        "analysis.exportCsv",
        format!("exported analysis `{id}` to CSV"),
        json!({ "sheet_id": id.as_str(), "byte_len": csv.len(), "path": params.path, "csv": csv }),
    ))
}

fn run_result_generate_report(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_result_generate_report();
    let params: GenerateReportParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.result_sheet_id)?;
    let format = match params.format.as_deref() {
        None => ReportFormat::Markdown,
        Some(raw_fmt) => ReportFormat::from_name(raw_fmt).ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, format!("unknown report format `{raw_fmt}`"))
                .with_remediation("Use `plain`, `markdown`, or `latex`.")
                .with_details(json!({ "pointer": "/format", "format": raw_fmt }))
        })?,
    };
    let report = generate_methods_report(ctx.session.document(), &id, format)?;
    if let Some(ref p) = params.path {
        write_atomic_output_file(p, report.as_bytes())?;
    }
    Ok(CommandOutcome::read_only(
        "result.generateReport",
        format!("generated {} methods report for `{id}`", format.as_str()),
        json!({
            "sheet_id": id.as_str(),
            "format": format.as_str(),
            "path": params.path,
            "report": report,
        }),
    ))
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
                .with_remediation("Supply `source_sheet` / `source_sheet_id` referencing an existing data sheet.")
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
        AnalysisDomain::TransformOnly => "tr",
        AnalysisDomain::StatsOnly => "stat",
        AnalysisDomain::FitOnly => "fit",
        AnalysisDomain::Any => "res",
    }
}

fn validate_domain_and_compat(kind: &AnalysisKind, domain: AnalysisDomain, table_kind: TableKind) -> Result<(), EngineError> {
    let domain_ok = match domain {
        AnalysisDomain::Any => true,
        AnalysisDomain::TransformOnly => matches!(kind, AnalysisKind::Transform(_)),
        AnalysisDomain::FitOnly => matches!(kind, AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_)),
        AnalysisDomain::StatsOnly => {
            !matches!(kind, AnalysisKind::Transform(_) | AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_))
        }
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
    if let Some(conf_val) = cleaned.remove("confidence_percent").or_else(|| cleaned.remove("confidence_level")) {
        let _ = cleaned.remove("confidence_level");
        let pct = conf_val
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`confidence_percent` must be a finite percentage in (1.0, 99.99]"))?;
        apply_metadata_updates(spec, None, Some(pct), None, None)?;
    }
    if let Some(title_val) = cleaned.remove("title").or_else(|| cleaned.remove("custom_title")) {
        let _ = cleaned.remove("custom_title");
        match title_val {
            Value::Null => apply_metadata_updates(spec, Some(None), None, None, None)?,
            Value::String(ref s) => apply_metadata_updates(spec, Some(Some(s.as_str())), None, None, None)?,
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, "`title` must be a string or null")),
        }
    }
    if let Some(sel_val) = cleaned.remove("selected_data_sets").or_else(|| cleaned.remove("selected_columns")).or_else(|| cleaned.remove("columns")) {
        let _ = cleaned.remove("selected_columns");
        let _ = cleaned.remove("columns");
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
        && nl.mv_y_column.is_some()
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
    if let Some(yc) = nl.mv_y_column
        && yc >= n_vars
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`mv_y_column` index {yc} out of range")));
    }
    let has_cat_fallback = mv
        .variables
        .iter()
        .enumerate()
        .any(|(idx, v)| Some(idx) != nl.mv_y_column && !nl.mv_x_columns.contains(&idx) && v.role == VariableRole::Categorical);
    if nl.mv_group_columns.is_empty() && !has_cat_fallback {
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
    let mv_spec =
        curvebook_fit::MultivarExtractionSpec { group_columns: &nl.mv_group_columns, x_columns: &nl.mv_x_columns, y_column: nl.mv_y_column };
    match curvebook_fit::extract_datasets_from_sheet_with_options(source_ds, &[], &mv_spec) {
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
        _ => {}
    }
    Ok(())
}

fn validate_nonlinear_spec_semantics(nl: &NonlinearFitSpec) -> Result<(), EngineError> {
    if let FitMethod::RoutOutlierElimination { fdr_q_percent } = nl.method
        && (!fdr_q_percent.is_finite() || fdr_q_percent <= 0.0 || fdr_q_percent > 50.0)
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("nonlinear ROUT `fdr_q_percent` ({fdr_q_percent}) must lie in (0, 50]")));
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
                let defaults = default_multivariate_numeric_columns(mv);
                let skip_all_continuous_explicit_pca =
                    explicit_empty_selection && matches!(mv_spec, MultivariateSpec::Pca { .. }) && defaults.len() == mv.variables.len();
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
            let num_cols: Vec<usize> = pool.into_iter().filter(|&i| is_mv_numeric_var(&mv.variables[i])).collect();
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
            let cat_cands: Vec<usize> =
                pool.iter().copied().filter(|&i| Some(i) != tg.mv_response_column && mv.variables[i].role == VariableRole::Categorical).collect();
            if let [only] = cat_cands.as_slice() {
                tg.mv_group_column = Some(*only);
            }
        }
        if tg.mv_response_column.is_none()
            && let Some(g_col) = tg.mv_group_column
        {
            let num_cands: Vec<usize> = pool.into_iter().filter(|&i| i != g_col && is_mv_numeric_var(&mv.variables[i])).collect();
            tg.mv_response_column = select_default_mv_response_col(mv, &num_cands);
        }
        return;
    }
    let cat_cols: Vec<usize> = pool.iter().copied().filter(|&i| mv.variables[i].role == VariableRole::Categorical).collect();
    let num_cols: Vec<usize> = pool.iter().copied().filter(|&i| is_mv_numeric_var(&mv.variables[i])).collect();
    if cat_cols.len() == 1
        && let Some(r_col) = select_default_mv_response_col(mv, &num_cols)
    {
        tg.mv_group_column = Some(cat_cols[0]);
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
        selected.iter().copied().filter(|&i| i < n_vars && !group_set.contains(&i) && is_mv_numeric_var(&mv.variables[i])).collect();

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
        .filter(|&i| i < n_vars && i != y_col && !x_used.contains(&i) && mv.variables[i].role == VariableRole::Categorical)
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
    if !is_mv_numeric_var(&mv.variables[y_col]) || x_cols.is_empty() || x_cols.iter().any(|&c| !is_mv_numeric_var(&mv.variables[c])) {
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
                let lower = mv.variables[i].title.to_ascii_lowercase();
                ["status", "event", "censor", "death"].iter().any(|h| lower.contains(h))
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
                let lower = mv.variables[i].title.to_ascii_lowercase();
                ["time", "surv", "follow", "dur", "day", "month", "year"].iter().any(|h| lower.contains(h))
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
        *mv_group_column = pool.iter().copied().find(|&i| i != time_col && i != status_col && mv.variables[i].role == VariableRole::Categorical);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyValidationMode {
    Create,
    UpdateParamsTopLevel,
    UpdateParamsNested,
}

impl KeyValidationMode {
    const fn allow_simulate_params(self) -> bool {
        matches!(self, Self::Create)
    }

    const fn allow_linear_hook_compat(self) -> bool {
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
        AnalysisKind::Transform(tr) => is_key_allowed_for_transform(tr, key, mode.allow_simulate_params()),
        AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) => {
            is_key_allowed_for_fit(kind, key, mode.allow_linear_hook_compat())
        }
        _ => is_key_allowed_for_stats(kind, key),
    }
}

fn is_key_allowed_for_transform(tr: &TransformAnalysisSpec, key: &str, allow_simulate_params: bool) -> bool {
    match tr {
        TransformAnalysisSpec::Math { .. } => {
            matches!(key, "x_transform" | "y_transform" | "preset" | "operation" | "error_propagation" | "propagate_error")
        }
        TransformAnalysisSpec::Normalize { .. } => {
            matches!(key, "zero_anchor" | "zero" | "hundred_anchor" | "hundred" | "output_percent" | "as_fraction" | "per_data_set")
        }
        TransformAnalysisSpec::RemoveBaseline { .. } => matches!(key, "baseline_columns" | "operation"),
        TransformAnalysisSpec::Transpose { .. } => key == "swap_row_and_column_titles",
        TransformAnalysisSpec::PruneRows { .. } => matches!(key, "every_k_rows" | "step" | "average_window" | "average_dropped"),
        TransformAnalysisSpec::SelectAndTransform { .. } => {
            matches!(key, "filter_formula" | "target_archetype" | "derived_columns" | "derived" | "column_formulas" | "formulas")
        }
        TransformAnalysisSpec::Simulate { .. } => {
            matches!(key, "seed" | "row_count" | "n_points" | "replicates" | "x_min" | "x_max" | "equation_id" | "parameters" | "noise")
                || (allow_simulate_params && key == "params")
        }
        TransformAnalysisSpec::RandomSample { .. } => {
            matches!(key, "seed" | "sample_size" | "n_samples" | "stratify_column" | "proportional_allocation")
        }
    }
}

fn is_key_allowed_for_multiplicity(mult: &MultiplicitySpec, key: &str) -> bool {
    matches!(key, "multiplicity" | "post_hoc" | "multiplicity_method" | "report_confidence_intervals" | "alpha" | "threshold")
        || (matches!(mult.method, MultiplicityMethod::Dunnett { .. }) && key == "control_column")
}

fn is_key_allowed_for_fit(kind: &AnalysisKind, key: &str, allow_linear_hook_compat: bool) -> bool {
    match kind {
        AnalysisKind::LinearRegression(_) => {
            (allow_linear_hook_compat && key == "parameter_rules")
                || matches!(
                    key,
                    "deming"
                        | "deming_model_ii"
                        | "force_through_origin"
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
                    | "fit_method"
                    | "weighting"
                    | "parameter_rules"
                    | "profile_likelihood_ci"
                    | "identify_unstable_parameters"
                    | "flag_r_squared_below"
                    | "comparison"
                    | "model_comparison"
                    | "confidence_bands"
                    | "prediction_bands"
                    | "interpolate_unknowns"
                    | "replicates_lack_of_fit_test"
                    | "normality_of_residuals"
                    | "normality_tests"
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
            ) || (matches!(nl.method, FitMethod::RoutOutlierElimination { .. }) && matches!(key, "q_percent" | "fdr_q_percent"))
                || (nl.comparison.is_some() && matches!(key, "alternative_equation_id" | "alternative_model"))
        }
        _ => false,
    }
}

fn is_key_allowed_for_stats(kind: &AnalysisKind, key: &str) -> bool {
    match kind {
        AnalysisKind::DescriptiveStats(_) => {
            matches!(key, "aggregation" | "custom_upper_percentile" | "run_normality_battery" | "hypothetical_value")
        }
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { .. }) => matches!(key, "q_percent" | "fdr_q_percent"),
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { .. }) => matches!(key, "alpha" | "iterative"),
        AnalysisKind::TwoGroupComparison(tg) => {
            matches!(
                key,
                "two_tailed" | "one_tailed" | "compute_effect_sizes" | "mv_group_column" | "group_column" | "mv_response_column" | "response_column"
            ) || (matches!(tg.method, TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. })
                && matches!(key, "hypothetical" | "hypothetical_mean"))
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
                    | "contingency_method"
                    | "compute_odds_and_risk_ratios"
                    | "compute_sensitivity_specificity"
                    | "compute_phi_and_cramers_v"
                    | "expected_proportions"
                    | "null_proportion"
            ) || (matches!(c.method, ContingencyMethod::PearsonChiSquare { .. }) && matches!(key, "yates" | "yates_continuity"))
                || (matches!(c.method, ContingencyMethod::CohenKappa { .. }) && matches!(key, "weighting" | "kappa_weighting"))
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => {
            matches!(
                key,
                "log_rank_mantel_cox"
                    | "log_rank"
                    | "gehan_breslow_wilcoxon"
                    | "gehan"
                    | "trend_test"
                    | "asymmetrical_greenwood_ci"
                    | "mv_time_column"
                    | "time_column"
                    | "mv_status_column"
                    | "status_column"
                    | "mv_group_column"
                    | "group_column"
            ) || is_key_allowed_for_multiplicity(multiplicity, key)
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. }) => {
            matches!(key, "time_column" | "status_column" | "predictor_columns" | "cox_covariate_columns" | "reference_levels" | "tie_method")
        }
        AnalysisKind::RocCurve(_) => key == "ci_method",
        AnalysisKind::GeneralizedLinearModel(_) => {
            matches!(key, "family" | "glm_family" | "response_column" | "predictor_columns" | "reference_levels" | "compute_vif")
        }
        AnalysisKind::Multivariate(mv) => is_key_allowed_for_multivariate(mv, key),
        AnalysisKind::PValueStack(_) => matches!(
            key,
            "method"
                | "pvalue_stack_method"
                | "multiplicity_method"
                | "multiplicity"
                | "threshold"
                | "pvalue_stack_threshold"
                | "alpha"
                | "q"
                | "fdr_q"
                | "column"
                | "pvalue_stack_column"
        ),
        AnalysisKind::PowerAnalysis(_) => matches!(
            key,
            "family"
                | "solve_target"
                | "tail_mode"
                | "two_tailed"
                | "alpha"
                | "target_power"
                | "sample_size"
                | "n"
                | "sample_size_group2"
                | "n2"
                | "allocation_ratio"
                | "effect"
                | "effect_size"
                | "standardized_effect"
                | "chi_square_df"
                | "df"
                | "two_proportions_mode"
                | "continuity_correction"
                | "correlation_method"
                | "info_sheet_id"
                | "num_groups"
                | "anova_groups"
        ),
        AnalysisKind::Transform(_) | AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) => false,
    }
}

fn is_key_allowed_for_multivariate(mv: &MultivariateSpec, key: &str) -> bool {
    match mv {
        MultivariateSpec::Pca { .. } => matches!(key, "standardize" | "parallel_analysis_seed" | "pcr_outcome_column"),
        MultivariateSpec::CorrelationMatrix { .. } => {
            matches!(key, "spearman" | "spearman_rank" | "correlation_all_pairs" | "correlation_heatmap")
        }
        MultivariateSpec::Clustering { .. } => {
            matches!(key, "k_means_k" | "k_clusters" | "distance" | "distance_metric" | "linkage" | "linkage_method" | "k_sweep_max")
        }
        MultivariateSpec::MultipleImputation { .. } => matches!(
            key,
            "imputations_m"
                | "m"
                | "num_imputations"
                | "max_iters"
                | "iterations"
                | "seed"
                | "response_column"
                | "target_column"
                | "predictor_columns"
                | "predictors"
        ),
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
    if selected_data_sets.len() == 1 && glm.response_column != 0 && glm.response_column != selected_data_sets[0] {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("GLM `response_column` {} does not match selected XY dataset {}", glm.response_column, selected_data_sets[0]),
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
    if glm.predictor_columns.len() != 1 || glm.predictor_columns[0] >= 2 {
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
        let mut kind: AnalysisKind = deserialize_strict(kv, "/kind")?;
        apply_shorthand_kind_overrides(&mut kind, params)?;
        kind
    } else {
        let raw_kind = kind_val.and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase());
        let mut kind = match raw_kind.as_deref() {
            None => infer_default_kind(params, domain, table_kind)?,
            Some(name) => resolve_kind_shorthand(name, params)?,
        };
        apply_kind_patch_and_overrides(&mut kind, params)?;
        kind
    };
    validate_map_keys_for_kind(&kind, extra, KeyValidationMode::Create, "parameter")?;
    let _ = &mut kind;
    Ok(kind)
}

fn infer_default_kind(params: &Value, domain: AnalysisDomain, table_kind: TableKind) -> Result<AnalysisKind, EngineError> {
    if params.get("equation_id").is_some() || params.get("custom_dsl").is_some() {
        return resolve_fit_shorthand("nonlinear", params);
    }
    match domain {
        AnalysisDomain::TransformOnly => resolve_transform_math(params),
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
    if let Some(kind) = resolve_transform_shorthand(name, params)? {
        return Ok(kind);
    }
    if let Some(kind) = resolve_two_group_or_anova_shorthand(name, params)? {
        return Ok(kind);
    }
    if let Some(kind) = resolve_other_stats_shorthand(name, params)? {
        return Ok(kind);
    }
    resolve_fit_shorthand(name, params)
}

fn resolve_transform_math(params: &Value) -> Result<AnalysisKind, EngineError> {
    let y_val = params.get("y_transform").or_else(|| params.get("preset")).or_else(|| params.get("operation"));
    let y_transform = match y_val {
        None | Some(Value::Null) => Some(parse_math_preset(None)?),
        Some(v) => Some(parse_serde_or_str(v, |s| parse_math_preset(Some(s)), "`y_transform` must be a preset string or object")?),
    };
    let x_transform = match params.get("x_transform") {
        None | Some(Value::Null) => None,
        Some(v) => Some(parse_serde_or_str(v, |s| parse_math_preset(Some(s)), "`x_transform` must be a preset string, object, or null")?),
    };
    let ep_val = params.get("error_propagation").or_else(|| params.get("propagate_error"));
    let error_propagation = match ep_val {
        None | Some(Value::Null) => parse_error_prop(None)?,
        Some(v) => parse_serde_or_str(v, |s| parse_error_prop(Some(s)), "`error_propagation` must be an error propagation mode")?,
    };
    Ok(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation }))
}

fn resolve_transform_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let tr = match name {
        "math" | "transform.math" | "math_transform" => return resolve_transform_math(params).map(Some),
        "normalize" | "transform.normalize" => TransformAnalysisSpec::Normalize {
            zero_anchor: match params.get("zero_anchor").or_else(|| params.get("zero")) {
                None | Some(Value::Null) => NormalizeAnchor::ColumnMin,
                Some(v) => parse_serde_or_str(v, parse_normalize_anchor, "`zero_anchor` must be a normalize anchor string or object")?,
            },
            hundred_anchor: match params.get("hundred_anchor").or_else(|| params.get("hundred")) {
                None | Some(Value::Null) => NormalizeAnchor::ColumnMax,
                Some(v) => parse_serde_or_str(v, parse_normalize_anchor, "`hundred_anchor` must be a normalize anchor string or object")?,
            },
            output_percent: !opt_bool_checked(params, &["as_fraction"])?.unwrap_or(false),
            per_data_set: opt_bool_checked(params, &["per_data_set"])?.unwrap_or(true),
        },
        "remove_baseline" | "baseline" | "transform.remove_baseline" => TransformAnalysisSpec::RemoveBaseline {
            baseline_columns: parse_usize_list(params.get("baseline_columns"))?.unwrap_or_else(|| vec![0]),
            operation: match params.get("operation") {
                None | Some(Value::Null) => BaselineOperation::Difference,
                Some(v) => parse_serde_or_str(v, parse_baseline_op, "`operation` must be a baseline operation string or object")?,
            },
        },
        "transpose" | "transform.transpose" => TransformAnalysisSpec::Transpose {
            swap_row_and_column_titles: opt_bool_checked(params, &["swap_row_and_column_titles"])?.unwrap_or(true),
        },
        "prune_rows" | "prune" | "transform.prune_rows" => TransformAnalysisSpec::PruneRows {
            every_k_rows: match opt_usize_checked(params, &["every_k_rows", "step"])? {
                Some((v, false)) => {
                    if v == 0 {
                        return Err(EngineError::with_code(ErrorCode::BadInput, "`every_k_rows` must be >= 1"));
                    }
                    v
                }
                Some((_, true)) | None => 2,
            },
            average_window: opt_bool_checked(params, &["average_window", "average_dropped"])?.unwrap_or(false),
        },
        "select_and_transform" | "select" | "transform.select_and_transform" => TransformAnalysisSpec::SelectAndTransform {
            filter_formula: opt_str_checked(params, &["filter_formula"])?.map(str::to_owned),
            target_archetype: TableKind::Xy,
            derived_columns: Vec::new(),
        },
        "simulate" | "simulate_xy" | "transform.simulate" => TransformAnalysisSpec::Simulate {
            seed: parse_opt_u64_field(params, "seed", 42)?,
            row_count: match opt_usize_checked(params, &["row_count", "n_points"])? {
                Some((v, false)) => v.max(2),
                Some((_, true)) | None => 13,
            },
            replicates: match opt_usize_checked(params, &["replicates"])? {
                Some((v, false)) => v.max(1),
                Some((_, true)) | None => 3,
            },
            x_min: opt_f64_checked(params, &["x_min"])?.unwrap_or(-9.0),
            x_max: opt_f64_checked(params, &["x_max"])?.unwrap_or(-3.0),
            equation_id: opt_str_checked(params, &["equation_id"])?.unwrap_or("log_agonist_vs_response_4pl").to_owned(),
            parameters: parse_named_params_vec(params.get("params"))?,
            noise: SimulationNoise::None,
        },
        "random_sample" | "sample" | "transform.random_sample" => {
            let sample_size = match params.get("sample_size").or_else(|| params.get("n_samples")) {
                Some(v) => {
                    let n = parse_usize_json_value(v, "sample_size")?;
                    if n == 0 {
                        return Err(EngineError::with_code(ErrorCode::BadInput, "`sample_size` must be >= 1"));
                    }
                    n
                }
                None => 10,
            };
            let stratify_column = opt_usize_checked(params, &["stratify_column"])?.and_then(|(col, is_null)| if is_null { None } else { Some(col) });
            TransformAnalysisSpec::RandomSample {
                seed: parse_opt_u64_field(params, "seed", 42)?,
                sample_size,
                stratify_column,
                proportional_allocation: opt_bool_checked(params, &["proportional_allocation"])?.unwrap_or(true),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(AnalysisKind::Transform(tr)))
}

fn resolve_two_group_or_anova_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let hyp = opt_f64_checked(params, &["hypothetical", "hypothetical_mean"])?.unwrap_or(0.0);
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
            let gg = opt_bool_checked(params, &["geisser_greenhouse"])?.unwrap_or(true);
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

fn resolve_other_stats_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let kind = match name {
        "descriptive" | "descriptive_stats" | "column_stats" => AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
        "outliers" | "outlier" | "rout" => AnalysisKind::OutlierDetection(OutlierSpec::Rout {
            fdr_q_percent: opt_f64_checked(params, &["q_percent", "fdr_q_percent"])?.unwrap_or(1.0),
        }),
        "grubbs" => AnalysisKind::OutlierDetection(OutlierSpec::Grubbs {
            alpha: opt_f64_checked(params, &["alpha"])?.unwrap_or(0.05),
            iterative: opt_bool_checked(params, &["iterative"])?.unwrap_or(false),
        }),
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
                "cohen_kappa" | "kappa" => ContingencyMethod::CohenKappa { weighting: parse_kappa_weighting_opt(params)?.unwrap_or_default() },
                "fleiss_kappa" => ContingencyMethod::FleissKappa,
                "exact_binomial" | "binomial" => ContingencyMethod::ExactBinomial,
                _ => match params.get("method").or_else(|| params.get("contingency_method")) {
                    None | Some(Value::Null) => ContingencyMethod::PearsonChiSquare { yates_continuity: false },
                    Some(m_val) => parse_serde_or_str(m_val, parse_contingency_method, "`method` must be a contingency method string or object")?,
                },
            };
            AnalysisKind::Contingency(ContingencySpec { method, ..ContingencySpec::default() })
        }
        "kaplan_meier" | "survival" | "log_rank" => AnalysisKind::Survival(default_km_survival()),
        "cox_ph" | "cox" => AnalysisKind::Survival(default_cox_survival(params)?),
        "roc" => AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong }),
        "glm" | "linear_glm" | "multiple_linear" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Linear)),
        "logistic_glm" | "multiple_logistic" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Logistic)),
        "poisson_glm" | "multiple_poisson" => AnalysisKind::GeneralizedLinearModel(default_glm(GlmFamily::Poisson)),
        "pca" | "principal_components" => AnalysisKind::Multivariate(default_pca()),
        "correlation" | "correlation_matrix" => AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix {
            spearman_rank: opt_bool_checked(params, &["spearman"])?.unwrap_or(false),
        }),
        "clustering" | "kmeans" => {
            let k_means_k = if let Some(v) = params.get("k_clusters").or_else(|| params.get("k_means_k")) {
                if v.is_null() {
                    None
                } else {
                    let k = parse_usize_json_value(v, "k_means_k")?;
                    if k < 2 {
                        return Err(EngineError::with_code(ErrorCode::BadInput, "`k_means_k` must be >= 2"));
                    }
                    Some(k)
                }
            } else if name == "kmeans" {
                Some(3)
            } else {
                None
            };
            let k_sweep_max = opt_usize_checked(params, &["k_sweep_max"])?.and_then(|(v, is_null)| if is_null { None } else { Some(v) });
            if let Some(k_max) = k_sweep_max
                && k_max < 2
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`k_sweep_max` must be >= 2"));
            }
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k,
                distance: parse_cluster_distance_opt(params)?.unwrap_or_default(),
                linkage: parse_cluster_linkage_opt(params)?.unwrap_or_default(),
                k_sweep_max,
            })
        }
        "multiple_imputation" | "stats.multiple_imputation" | "mice" => {
            AnalysisKind::Multivariate(default_multiple_imputation(parse_opt_u64_field(params, "seed", 42)?))
        }
        _ => return Ok(None),
    };
    Ok(Some(kind))
}

fn resolve_fit_shorthand(name: &str, params: &Value) -> Result<AnalysisKind, EngineError> {
    if matches!(name, "linear" | "simple_linear" | "linear_regression" | "deming" | "deming_regression") {
        let deming_model_ii = matches!(name, "deming" | "deming_regression") || opt_bool_checked(params, &["deming"])?.unwrap_or(false);
        return Ok(AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii, ..LinearRegressionSpec::default() }));
    }

    let explicit_eq = opt_str_checked(params, &["equation_id"])?;
    let explicit_dsl = opt_str_checked(params, &["custom_dsl"])?;
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
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown analysis kind or equation `{other}`"))
                    .with_remediation("Supply a supported analysis kind or built-in curve fit equation identifier.")
                    .with_details(json!({ "pointer": "/kind", "kind": other })));
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

fn apply_shorthand_kind_overrides(kind: &mut AnalysisKind, params: &Value) -> Result<(), EngineError> {
    match kind {
        AnalysisKind::Transform(spec) => apply_transform_overrides(spec, params)?,
        AnalysisKind::OutlierDetection(spec) => apply_outlier_overrides(spec, params)?,
        AnalysisKind::OneWayAnova(spec) => apply_one_way_overrides(spec, params)?,
        AnalysisKind::FactorialAnova(spec) => apply_factorial_overrides(spec, params)?,
        AnalysisKind::NestedComparison(spec) => apply_nested_overrides(spec, params)?,
        AnalysisKind::MultipleRowTTests(spec) => apply_multiple_row_overrides(spec, params)?,
        AnalysisKind::BlandAltman(spec) => apply_bland_altman_overrides(spec, params)?,
        AnalysisKind::PassingBablok(spec) => apply_passing_bablok_overrides(spec, params)?,
        AnalysisKind::PValueStack(spec) => apply_pvalue_stack_overrides(spec, params)?,
        AnalysisKind::PowerAnalysis(spec) => apply_power_overrides(spec, params)?,
        AnalysisKind::TwoGroupComparison(spec) => apply_two_group_overrides(spec, params)?,
        AnalysisKind::Contingency(spec) => apply_contingency_overrides(spec, params)?,
        AnalysisKind::Survival(spec) => apply_survival_overrides(spec, params)?,
        AnalysisKind::Multivariate(spec) => apply_multivariate_overrides(spec, params)?,
        AnalysisKind::NonlinearRegression(spec) => apply_nonlinear_overrides(spec, params)?,
        AnalysisKind::LinearRegression(spec) => apply_linear_overrides(spec, params)?,
        AnalysisKind::GeneralizedLinearModel(spec) => apply_glm_overrides(spec, params)?,
        _ => {}
    }
    Ok(())
}

fn apply_linear_overrides(spec: &mut LinearRegressionSpec, params: &Value) -> Result<(), EngineError> {
    apply_bool_opt_checked(params, &["deming", "deming_model_ii"], &mut spec.deming_model_ii)?;
    apply_bool_opt_checked(params, &["force_through_origin"], &mut spec.force_through_origin)?;
    apply_bool_opt_checked(params, &["confidence_bands"], &mut spec.confidence_bands)?;
    apply_bool_opt_checked(params, &["prediction_bands"], &mut spec.prediction_bands)?;
    apply_bool_opt_checked(params, &["interpolate_unknowns"], &mut spec.interpolate_unknowns)?;
    if let Some(value) = params.get("x_range_filter") {
        spec.x_range_filter = parse_opt_range_filter(value, "x_range_filter")?;
    }
    apply_opt_f64_field(params, &["plot_x_min"], &mut spec.plot_x_min)?;
    apply_opt_f64_field(params, &["plot_x_max"], &mut spec.plot_x_max)?;
    apply_opt_usize_field(params, &["plot_segments"], &mut spec.plot_segments)
}

fn apply_outlier_overrides(spec: &mut OutlierSpec, params: &Value) -> Result<(), EngineError> {
    match spec {
        OutlierSpec::Rout { fdr_q_percent } => {
            if let Some(q) = opt_f64_checked(params, &["q_percent", "fdr_q_percent"])? {
                *fdr_q_percent = q;
            }
        }
        OutlierSpec::Grubbs { alpha, iterative } => {
            if let Some(a) = opt_f64_checked(params, &["alpha"])? {
                *alpha = a;
            }
            apply_bool_opt_checked(params, &["iterative"], iterative)?;
        }
    }
    Ok(())
}

fn apply_kind_patch_and_overrides(kind: &mut AnalysisKind, params: &Value) -> Result<(), EngineError> {
    apply_shorthand_kind_overrides(kind, params)?;
    patch_analysis_kind_fields(kind, params)?;
    apply_shorthand_kind_overrides(kind, params)?;
    Ok(())
}

fn apply_two_group_overrides(spec: &mut TwoGroupSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(two_tailed) = opt_bool_checked(params, &["two_tailed"])? {
        spec.two_tailed = two_tailed;
    } else if let Some(one_tailed) = opt_bool_checked(params, &["one_tailed"])? {
        spec.two_tailed = !one_tailed;
    }
    apply_bool_opt_checked(params, &["compute_effect_sizes"], &mut spec.compute_effect_sizes)?;
    apply_opt_usize_field(params, &["mv_group_column", "group_column"], &mut spec.mv_group_column)?;
    apply_opt_usize_field(params, &["mv_response_column", "response_column"], &mut spec.mv_response_column)?;
    Ok(())
}

fn apply_multiple_row_overrides(spec: &mut MultipleRowTestsSpec, params: &Value) -> Result<(), EngineError> {
    apply_bool_opt_checked(params, &["paired"], &mut spec.paired)?;
    apply_bool_opt_checked(params, &["nonparametric"], &mut spec.nonparametric)?;
    apply_bool_opt_checked(params, &["assume_equal_sd"], &mut spec.assume_equal_sd)?;
    apply_multiplicity_override(&mut spec.multiplicity, params)
}

fn apply_bland_altman_overrides(spec: &mut BlandAltmanSpec, params: &Value) -> Result<(), EngineError> {
    apply_usize_field(params, &["column_a"], &mut spec.column_a)?;
    apply_usize_field(params, &["column_b"], &mut spec.column_b)?;
    apply_bool_opt_checked(params, &["percent_difference"], &mut spec.percent_difference)?;
    if let Some(xy_val) = params.get("xy_compare_x_vs_y") {
        spec.xy_compare_x_vs_y = if xy_val.is_null() {
            None
        } else {
            Some(xy_val.as_bool().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`xy_compare_x_vs_y` must be boolean or null"))?)
        };
    }
    Ok(())
}

fn apply_passing_bablok_overrides(spec: &mut PassingBablokSpec, params: &Value) -> Result<(), EngineError> {
    apply_usize_field(params, &["x_column"], &mut spec.x_column)?;
    apply_usize_field(params, &["y_column"], &mut spec.y_column)?;
    Ok(())
}

fn parse_rm_design_override(raw: &str) -> Result<RepeatedMeasuresDesign, EngineError> {
    match raw.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "none" | "no_matching" => Ok(RepeatedMeasuresDesign::None),
        "row" | "row_factor" | "row_factor_matched" => Ok(RepeatedMeasuresDesign::RowFactorMatched),
        "column" | "col" | "column_factor" | "column_factor_matched" => Ok(RepeatedMeasuresDesign::ColumnFactorMatched),
        "both" | "both_factors" | "both_factors_matched" => Ok(RepeatedMeasuresDesign::BothFactorsMatched),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown repeated-measures design `{raw}`"))),
    }
}

fn parse_residual_covariance_override(raw: &str) -> Result<ResidualCovariance, EngineError> {
    match raw.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "compound_symmetry" | "compoundsymmetry" | "cs" | "random_intercept" => Ok(ResidualCovariance::CompoundSymmetry),
        "unstructured" | "un" | "general" => Ok(ResidualCovariance::Unstructured),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown residual covariance structure `{raw}`"))),
    }
}

fn parse_df_method_override(raw: &str) -> Result<MixedModelDfMethod, EngineError> {
    match raw.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "kenward_roger" | "kenwardroger" | "kr" => Ok(MixedModelDfMethod::KenwardRoger),
        "satterthwaite" | "sat" | "fai_cornelius" => Ok(MixedModelDfMethod::Satterthwaite),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown mixed-model df method `{raw}`"))),
    }
}

fn apply_residual_covariance_override(cov: &mut ResidualCovariance, params: &Value) -> Result<(), EngineError> {
    if let Some(cov_val) = params.get("residual_covariance").filter(|v| !v.is_null()) {
        if let Ok(parsed) = serde_json::from_value::<ResidualCovariance>(cov_val.clone()) {
            *cov = parsed;
        } else if let Some(raw) = cov_val.as_str() {
            *cov = parse_residual_covariance_override(raw)?;
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`residual_covariance` must be `compound_symmetry` or `unstructured`"));
        }
    }
    Ok(())
}

fn apply_df_method_override(df_method: &mut MixedModelDfMethod, params: &Value) -> Result<(), EngineError> {
    if let Some(df_val) = params.get("df_method").filter(|v| !v.is_null()) {
        if let Ok(parsed) = serde_json::from_value::<MixedModelDfMethod>(df_val.clone()) {
            *df_method = parsed;
        } else if let Some(raw) = df_val.as_str() {
            *df_method = parse_df_method_override(raw)?;
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`df_method` must be `kenward_roger` or `satterthwaite`"));
        }
    }
    Ok(())
}

fn apply_one_way_overrides(spec: &mut OneWayAnovaSpec, params: &Value) -> Result<(), EngineError> {
    apply_bool_opt_checked(params, &["mixed_effects_reml"], &mut spec.mixed_effects_reml)?;
    apply_df_method_override(&mut spec.df_method, params)?;
    apply_residual_covariance_override(&mut spec.residual_covariance, params)?;
    apply_bool_opt_checked(params, &["compute_effect_sizes"], &mut spec.compute_effect_sizes)?;
    apply_multiplicity_override(&mut spec.multiplicity, params)
}

fn apply_nested_overrides(spec: &mut NestedAnalysisSpec, params: &Value) -> Result<(), EngineError> {
    apply_bool_opt_checked(params, &["two_group_t_test"], &mut spec.two_group_t_test)?;
    apply_df_method_override(&mut spec.df_method, params)?;
    apply_multiplicity_override(&mut spec.multiplicity, params)
}

fn apply_factorial_overrides(spec: &mut FactorialAnovaSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(rm_val) = params.get("repeated_measures").filter(|v| !v.is_null()) {
        if let Ok(parsed) = serde_json::from_value::<RepeatedMeasuresDesign>(rm_val.clone()) {
            spec.repeated_measures = parsed;
        } else if let Some(raw) = rm_val.as_str() {
            spec.repeated_measures = parse_rm_design_override(raw)?;
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`repeated_measures` must be a valid design string"));
        }
    }
    apply_bool_opt_checked(params, &["mixed_effects_reml"], &mut spec.mixed_effects_reml)?;
    apply_df_method_override(&mut spec.df_method, params)?;
    apply_residual_covariance_override(&mut spec.residual_covariance, params)?;
    apply_bool_opt_checked(params, &["fit_reml_on_missing"], &mut spec.fit_reml_on_missing)?;
    apply_bool_opt_checked(params, &["compute_effect_sizes"], &mut spec.compute_effect_sizes)?;
    apply_usize_field(params, &["max_interaction_order"], &mut spec.max_interaction_order)?;
    apply_multiplicity_override(&mut spec.multiplicity, params)
}

fn parse_pvalue_stack_method(raw: &str) -> Result<PValueStackMethod, EngineError> {
    let norm = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match norm.as_str() {
        "bonferroni" => Ok(PValueStackMethod::Bonferroni),
        "holm" | "holm_bonferroni" | "holm_1979" => Ok(PValueStackMethod::HolmBonferroni),
        "sidak" | "šidák" | "šídák" | "sidak_1967" => Ok(PValueStackMethod::Sidak),
        "holm_sidak" | "holm_šidák" | "holm_šídák" => Ok(PValueStackMethod::HolmSidak),
        "bh" | "benjamini_hochberg" | "benjamini_hochberg_fdr" | "fdr_bh" => Ok(PValueStackMethod::BenjaminiHochberg),
        "bky" | "two_stage_bky" | "two_stage_bky_fdr" | "benjamini_krieger_yekutieli" | "bky_2006" => Ok(PValueStackMethod::TwoStageBky),
        "by" | "benjamini_yekutieli" | "benjamini_yekutieli_fdr" | "fdr_by" => Ok(PValueStackMethod::BenjaminiYekutieli),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown P-value stack multiplicity method `{raw}`"))),
    }
}

fn apply_pvalue_stack_overrides(spec: &mut PValueStackSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(m_val) = params
        .get("method")
        .or_else(|| params.get("pvalue_stack_method"))
        .or_else(|| params.get("multiplicity_method"))
        .or_else(|| params.get("multiplicity"))
        && !m_val.is_null()
    {
        if let Ok(parsed) = serde_json::from_value::<PValueStackMethod>(m_val.clone()) {
            spec.method = parsed;
        } else if let Some(raw) = m_val.as_str() {
            spec.method = parse_pvalue_stack_method(raw)?;
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`method` must be a valid P-value stack multiplicity method string"));
        }
    }
    if let Some(t_val) = params
        .get("threshold")
        .or_else(|| params.get("pvalue_stack_threshold"))
        .or_else(|| params.get("alpha"))
        .or_else(|| params.get("q"))
        .or_else(|| params.get("fdr_q"))
        && !t_val.is_null()
    {
        let t = t_val
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0 && *v < 1.0)
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`threshold` (`alpha` / `Q`) must be a finite number strictly in (0, 1)"))?;
        spec.threshold = t;
    }
    apply_opt_usize_field(params, &["column", "pvalue_stack_column"], &mut spec.column)?;
    Ok(())
}

fn apply_power_overrides(spec: &mut PowerAnalysisSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(fam_val) = opt_str_checked(params, &["family"])? {
        let norm: String = fam_val.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        spec.family = match norm.as_str() {
            "unpairedt" | "twosampleunpairedt" | "independentt" => PowerTestFamily::TwoSampleUnpairedT,
            "pairedt" | "twosamplepairedt" | "dependentt" => PowerTestFamily::TwoSamplePairedT,
            "onesamplet" | "singlesamplet" => PowerTestFamily::OneSampleT,
            "onewayanova" | "anova" => PowerTestFamily::OneWayAnova,
            "twoproportions" | "proportions" => PowerTestFamily::TwoProportions,
            "correlation" | "pearsoncorrelation" => PowerTestFamily::Correlation,
            "chisquare" | "chi2" => PowerTestFamily::ChiSquare,
            "simplelinearregression" | "linearregression" | "regression" => PowerTestFamily::SimpleLinearRegression,
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown power test family `{fam_val}`"))),
        };
    }
    if let Some(target_val) = opt_str_checked(params, &["solve_target"])? {
        let norm: String = target_val.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        spec.solve_target = match norm.as_str() {
            "samplesize" | "n" => PowerSolveTarget::SampleSize,
            "power" => PowerSolveTarget::Power,
            "detectableeffect" | "effect" | "mde" => PowerSolveTarget::DetectableEffect,
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown power solve target `{target_val}`"))),
        };
    }
    if let Some(two_tailed) = opt_bool_checked(params, &["two_tailed"])? {
        spec.tail_mode = if two_tailed { PowerTailMode::TwoSided } else { PowerTailMode::OneSided(PowerOneSidedDirection::Greater) };
    }
    if let Some(a) = opt_f64_checked(params, &["alpha"])? {
        spec.alpha = a;
    }
    apply_opt_f64_field(params, &["target_power"], &mut spec.target_power)?;
    apply_opt_f64_field(params, &["sample_size", "n"], &mut spec.sample_size)?;
    apply_opt_f64_field(params, &["sample_size_group2", "n2"], &mut spec.sample_size_group2)?;
    if let Some(r) = opt_f64_checked(params, &["allocation_ratio"])? {
        spec.allocation_ratio = r;
    }
    if let Some(df) = opt_f64_checked(params, &["chi_square_df", "df"])? {
        spec.chi_square_df = df;
    }
    if let Some(eff) = opt_f64_checked(params, &["effect_size", "standardized_effect"])? {
        spec.effect = PowerEffectSpec::Standardized(eff);
    }
    if let Some(info_raw) = params.get("info_sheet_id") {
        spec.info_sheet_id = if info_raw.is_null() {
            None
        } else {
            let s = info_raw.as_str().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`info_sheet_id` must be a string or null"))?;
            Some(SheetId::try_new(s)?)
        };
    }
    apply_bool_opt_checked(params, &["continuity_correction"], &mut spec.continuity_correction)?;
    apply_opt_usize_field(params, &["num_groups", "anova_groups"], &mut spec.num_groups)?;
    Ok(())
}

fn parse_serde_or_str<T: serde::de::DeserializeOwned>(
    val: &Value,
    parse_str: impl FnOnce(&str) -> Result<T, EngineError>,
    err_msg: &str,
) -> Result<T, EngineError> {
    if val.is_object() {
        return deserialize_strict(val, "");
    }
    if let Ok(parsed) = serde_json::from_value::<T>(val.clone()) {
        return Ok(parsed);
    }
    if let Some(raw) = val.as_str() {
        return parse_str(raw);
    }
    Err(EngineError::with_code(ErrorCode::BadInput, err_msg))
}

fn apply_transform_overrides(spec: &mut TransformAnalysisSpec, params: &Value) -> Result<(), EngineError> {
    match spec {
        TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation } => {
            if let Some(xv) = params.get("x_transform") {
                *x_transform = if xv.is_null() {
                    None
                } else {
                    Some(parse_serde_or_str(xv, |s| parse_math_preset(Some(s)), "`x_transform` must be a preset string, object, or null")?)
                };
            }
            if let Some(yv) = params.get("y_transform").or_else(|| params.get("preset")).or_else(|| params.get("operation")) {
                *y_transform = if yv.is_null() {
                    None
                } else {
                    Some(parse_serde_or_str(yv, |s| parse_math_preset(Some(s)), "`y_transform` must be a preset string, object, or null")?)
                };
            }
            if let Some(ev) = params.get("error_propagation").or_else(|| params.get("propagate_error"))
                && !ev.is_null()
            {
                *error_propagation = parse_serde_or_str(ev, |s| parse_error_prop(Some(s)), "`error_propagation` must be an error propagation mode")?;
            }
        }
        TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set } => {
            if let Some(zv) = params.get("zero_anchor").or_else(|| params.get("zero"))
                && !zv.is_null()
            {
                *zero_anchor = parse_serde_or_str(zv, parse_normalize_anchor, "`zero_anchor` must be a normalize anchor")?;
            }
            if let Some(hv) = params.get("hundred_anchor").or_else(|| params.get("hundred"))
                && !hv.is_null()
            {
                *hundred_anchor = parse_serde_or_str(hv, parse_normalize_anchor, "`hundred_anchor` must be a normalize anchor")?;
            }
            apply_bool_opt_checked(params, &["output_percent"], output_percent)?;
            if let Some(as_frac) = opt_bool_checked(params, &["as_fraction"])? {
                *output_percent = !as_frac;
            }
            apply_bool_opt_checked(params, &["per_data_set"], per_data_set)?;
        }
        TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation } => {
            if let Some(cols) = parse_usize_list(params.get("baseline_columns"))? {
                *baseline_columns = cols;
            }
            if let Some(ov) = params.get("operation")
                && !ov.is_null()
            {
                *operation = parse_serde_or_str(ov, parse_baseline_op, "`operation` must be a baseline operation")?;
            }
        }
        TransformAnalysisSpec::Transpose { swap_row_and_column_titles } => {
            apply_bool_opt_checked(params, &["swap_row_and_column_titles"], swap_row_and_column_titles)?;
        }
        TransformAnalysisSpec::PruneRows { every_k_rows, average_window } => {
            apply_usize_field(params, &["every_k_rows", "step"], every_k_rows)?;
            if *every_k_rows == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`every_k_rows` must be >= 1"));
            }
            apply_bool_opt_checked(params, &["average_window", "average_dropped"], average_window)?;
        }
        TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, derived_columns } => {
            if let Some(fv) = params.get("filter_formula") {
                *filter_formula = if fv.is_null() {
                    None
                } else {
                    Some(
                        fv.as_str().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`filter_formula` must be string or null"))?.to_owned(),
                    )
                };
            }
            if let Some(tv) = params.get("target_archetype")
                && !tv.is_null()
            {
                *target_archetype = serde_json::from_value(tv.clone())
                    .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `target_archetype`: {err}")))?;
            }
            if let Some(dv) = params
                .get("derived_columns")
                .or_else(|| params.get("derived"))
                .or_else(|| params.get("column_formulas"))
                .or_else(|| params.get("formulas"))
            {
                *derived_columns = parse_derived_columns_value(dv)?;
            }
        }
        TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation } => {
            if let Some(seed_val) = params.get("seed")
                && !seed_val.is_null()
            {
                *seed = parse_u64_json_value(seed_val, "seed")?;
            }
            apply_usize_field(params, &["sample_size", "n_samples"], sample_size)?;
            if *sample_size == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`sample_size` must be >= 1"));
            }
            apply_opt_usize_field(params, &["stratify_column"], stratify_column)?;
            apply_bool_opt_checked(params, &["proportional_allocation"], proportional_allocation)?;
        }
        TransformAnalysisSpec::Simulate { seed, row_count, replicates, x_min, x_max, equation_id, parameters, .. } => {
            if let Some(seed_val) = params.get("seed")
                && !seed_val.is_null()
            {
                *seed = parse_u64_json_value(seed_val, "seed")?;
            }
            apply_usize_field(params, &["row_count", "n_points"], row_count)?;
            if *row_count < 2 {
                *row_count = 2;
            }
            apply_usize_field(params, &["replicates"], replicates)?;
            if *replicates == 0 {
                *replicates = 1;
            }
            if let Some(xmin) = opt_f64_checked(params, &["x_min"])? {
                *x_min = xmin;
            }
            if let Some(xmax) = opt_f64_checked(params, &["x_max"])? {
                *x_max = xmax;
            }
            if let Some(eq) = opt_str_checked(params, &["equation_id"])? {
                *equation_id = eq.to_owned();
            }
            if let Some(p_val) = params.get("parameters").or_else(|| params.get("params"))
                && !p_val.is_null()
                && p_val.is_object()
            {
                *parameters = parse_named_params_vec(Some(p_val))?;
            }
        }
    }
    Ok(())
}

fn apply_contingency_overrides(spec: &mut ContingencySpec, params: &Value) -> Result<(), EngineError> {
    if let Some(m_val) = params.get("method").or_else(|| params.get("contingency_method")) {
        spec.method = parse_serde_or_str(m_val, parse_contingency_method, "contingency `method` must be a string or method object")?;
    }
    if let ContingencyMethod::PearsonChiSquare { yates_continuity } = &mut spec.method {
        apply_bool_opt_checked(params, &["yates_continuity", "yates"], yates_continuity)?;
    }
    if let ContingencyMethod::CohenKappa { weighting } = &mut spec.method
        && let Some(w) = parse_kappa_weighting_opt(params)?
    {
        *weighting = w;
    }
    apply_opt_f64_field(params, &["null_proportion"], &mut spec.null_proportion)?;
    if let Some(p0) = spec.null_proportion
        && (p0 <= 0.0 || p0 >= 1.0)
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`null_proportion` must lie strictly in (0, 1)"));
    }
    if let Some(ep_val) = params.get("expected_proportions") {
        spec.expected_proportions = parse_opt_expected_proportions(ep_val)?;
    }
    Ok(())
}

fn apply_multiplicity_override(mult: &mut MultiplicitySpec, params: &Value) -> Result<(), EngineError> {
    if let Some(m_val) = params.get("multiplicity").or_else(|| params.get("post_hoc")).or_else(|| params.get("multiplicity_method"))
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
        } else if let Ok(parsed_method) = serde_json::from_value::<MultiplicityMethod>(m_val.clone()) {
            mult.method = parsed_method;
        } else if let Some(name) = m_val.as_str() {
            mult.method = parse_multiplicity_method(name)?.method;
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`multiplicity` must be a method name or MultiplicitySpec object"));
        }
    }
    if let MultiplicityMethod::Dunnett { ref mut control_column } = mult.method {
        apply_usize_field(params, &["control_column"], control_column)?;
    }
    apply_bool_opt_checked(params, &["report_confidence_intervals"], &mut mult.report_confidence_intervals)?;
    if let Some(t) = opt_f64_checked(params, &["alpha", "threshold"])? {
        mult.threshold = t;
    }
    Ok(())
}

fn apply_survival_overrides(spec: &mut SurvivalAnalysisSpec, params: &Value) -> Result<(), EngineError> {
    match spec {
        SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox,
            gehan_breslow_wilcoxon,
            trend_test,
            asymmetrical_greenwood_ci,
            multiplicity,
            mv_time_column,
            mv_status_column,
            mv_group_column,
        } => {
            apply_multiplicity_override(multiplicity, params)?;
            apply_bool_opt_checked(params, &["log_rank_mantel_cox", "log_rank"], log_rank_mantel_cox)?;
            apply_bool_opt_checked(params, &["gehan_breslow_wilcoxon", "gehan"], gehan_breslow_wilcoxon)?;
            apply_bool_opt_checked(params, &["trend_test"], trend_test)?;
            apply_bool_opt_checked(params, &["asymmetrical_greenwood_ci"], asymmetrical_greenwood_ci)?;
            apply_opt_usize_field(params, &["mv_time_column", "time_column"], mv_time_column)?;
            apply_opt_usize_field(params, &["mv_status_column", "status_column"], mv_status_column)?;
            apply_opt_usize_field(params, &["mv_group_column", "group_column"], mv_group_column)?;
            Ok(())
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, tie_method } => {
            apply_usize_field(params, &["time_column"], time_column)?;
            apply_usize_field(params, &["status_column"], status_column)?;
            if let Some(preds) = parse_usize_list(params.get("predictor_columns").or_else(|| params.get("cox_covariate_columns")))? {
                *predictor_columns = preds;
            }
            if let Some(rl_val) = params.get("reference_levels") {
                *reference_levels = if rl_val.is_null() { Vec::new() } else { parse_reference_levels(rl_val)? };
            }
            if let Some(tm_val) = params.get("tie_method")
                && !tm_val.is_null()
            {
                *tie_method = parse_cox_tie_method(tm_val)?;
            }
            Ok(())
        }
    }
}

fn apply_glm_overrides(spec: &mut GlmSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(fam_val) = params.get("family").or_else(|| params.get("glm_family"))
        && !fam_val.is_null()
    {
        if let Ok(parsed) = serde_json::from_value::<GlmFamily>(fam_val.clone()) {
            spec.family = parsed;
        } else if let Some(raw) = fam_val.as_str() {
            spec.family = match raw.trim().to_ascii_lowercase().as_str() {
                "linear" => GlmFamily::Linear,
                "logistic" => GlmFamily::Logistic,
                "poisson" => GlmFamily::Poisson,
                other => return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown GLM family `{other}`"))),
            };
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`family` must be Linear, Logistic, or Poisson"));
        }
    }
    apply_usize_field(params, &["response_column"], &mut spec.response_column)?;
    if let Some(preds) = parse_usize_list(params.get("predictor_columns"))? {
        spec.predictor_columns = preds;
    }
    if let Some(rl_val) = params.get("reference_levels") {
        spec.reference_levels = if rl_val.is_null() { Vec::new() } else { parse_reference_levels(rl_val)? };
    }
    apply_bool_opt_checked(params, &["compute_vif"], &mut spec.compute_vif)?;
    Ok(())
}

fn apply_multivariate_overrides(spec: &mut MultivariateSpec, params: &Value) -> Result<(), EngineError> {
    match spec {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed } => {
            apply_bool_opt_checked(params, &["standardize"], standardize)?;
            if let Some(seed_val) = params.get("parallel_analysis_seed") {
                *parallel_analysis_seed = if seed_val.is_null() { None } else { Some(parse_u64_json_value(seed_val, "parallel_analysis_seed")?) };
            }
        }
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            apply_bool_opt_checked(params, &["spearman", "spearman_rank"], spearman_rank)?;
        }
        MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max } => {
            if let Some(v) = params.get("k_clusters").or_else(|| params.get("k_means_k")) {
                if v.is_null() {
                    *k_means_k = None;
                } else {
                    let k = parse_usize_json_value(v, "k_means_k")?;
                    if k < 2 {
                        return Err(EngineError::with_code(ErrorCode::BadInput, "`k_means_k` must be >= 2"));
                    }
                    *k_means_k = Some(k);
                }
            }
            if let Some(d) = parse_cluster_distance_opt(params)? {
                *distance = d;
            }
            if let Some(l) = parse_cluster_linkage_opt(params)? {
                *linkage = l;
            }
            apply_opt_usize_field(params, &["k_sweep_max"], k_sweep_max)?;
            if let Some(k_max) = *k_sweep_max
                && k_max < 2
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`k_sweep_max` must be >= 2"));
            }
        }
        MultivariateSpec::MultipleImputation { imputations_m, max_iters, seed, response_column, predictor_columns } => {
            apply_usize_field(params, &["imputations_m", "m", "num_imputations"], imputations_m)?;
            if !(2..=50).contains(imputations_m) {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`imputations_m` must be in 2..=50"));
            }
            apply_usize_field(params, &["max_iters", "iterations"], max_iters)?;
            if !(1..=100).contains(max_iters) {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`max_iters` must be in 1..=100"));
            }
            if let Some(seed_val) = params.get("seed")
                && !seed_val.is_null()
            {
                *seed = parse_u64_json_value(seed_val, "seed")?;
            }
            apply_opt_usize_field(params, &["response_column", "target_column"], response_column)?;
            if let Some(preds) = parse_usize_list(params.get("predictor_columns").or_else(|| params.get("predictors")))? {
                *predictor_columns = preds;
            }
        }
    }
    Ok(())
}

fn apply_nonlinear_overrides(spec: &mut NonlinearFitSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(eq) = opt_str_checked(params, &["equation_id"])? {
        spec.equation_id = eq.to_owned();
    }
    if let Some(dsl_val) = params.get("custom_dsl") {
        spec.custom_dsl = if dsl_val.is_null() {
            None
        } else {
            Some(dsl_val.as_str().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`custom_dsl` must be string or null"))?.to_owned())
        };
    }
    if let Some(w) = opt_str_checked(params, &["weighting"])? {
        spec.weighting = parse_weighting_scheme(w)?;
    }
    if let Some(m_val) = params.get("method").or_else(|| params.get("fit_method"))
        && !m_val.is_null()
    {
        spec.method = parse_serde_or_str(m_val, parse_fit_method, "`method` must be a valid fit method string or object")?;
    }
    if let Some(q) = opt_f64_checked(params, &["q_percent", "fdr_q_percent"])?
        && let FitMethod::RoutOutlierElimination { ref mut fdr_q_percent } = spec.method
    {
        *fdr_q_percent = q;
    }
    apply_nonlinear_flag_and_mv_overrides(spec, params)?;
    if let Some(rules_val) = params.get("parameter_rules") {
        spec.parameter_rules = parse_parameter_rules(rules_val)?;
    }
    if let Some(comp_val) = params.get("comparison").or_else(|| params.get("model_comparison")) {
        spec.comparison = parse_model_comparison(comp_val, params)?;
    }
    if let Some(xr_val) = params.get("x_range_filter") {
        spec.x_range_filter = parse_opt_range_filter(xr_val, "x_range_filter")?;
    }
    if let Some(yr_val) = params.get("y_range_filter") {
        spec.y_range_filter = parse_opt_range_filter(yr_val, "y_range_filter")?;
    }
    apply_opt_f64_field(params, &["plot_x_min"], &mut spec.plot_x_min)?;
    apply_opt_f64_field(params, &["plot_x_max"], &mut spec.plot_x_max)?;
    Ok(())
}

fn apply_nonlinear_flag_and_mv_overrides(spec: &mut NonlinearFitSpec, params: &Value) -> Result<(), EngineError> {
    apply_bool_opt_checked(params, &["confidence_bands"], &mut spec.confidence_bands)?;
    apply_bool_opt_checked(params, &["prediction_bands"], &mut spec.prediction_bands)?;
    apply_bool_opt_checked(params, &["interpolate_unknowns"], &mut spec.interpolate_unknowns)?;
    apply_bool_opt_checked(params, &["profile_likelihood_ci"], &mut spec.profile_likelihood_ci)?;
    apply_bool_opt_checked(params, &["identify_unstable_parameters"], &mut spec.identify_unstable_parameters)?;
    apply_opt_f64_field(params, &["flag_r_squared_below"], &mut spec.flag_r_squared_below)?;
    apply_bool_opt_checked(params, &["normality_of_residuals", "normality_tests"], &mut spec.normality_of_residuals)?;
    apply_bool_opt_checked(params, &["replicates_lack_of_fit_test"], &mut spec.replicates_lack_of_fit_test)?;
    apply_bool_opt_checked(params, &["plot_function_only"], &mut spec.plot_function_only)?;
    apply_bool_opt_checked(params, &["homoscedasticity_test"], &mut spec.homoscedasticity_test)?;
    apply_bool_opt_checked(params, &["emit_residuals_tab"], &mut spec.emit_residuals_tab)?;
    apply_bool_opt_checked(params, &["emit_covariance_tab"], &mut spec.emit_covariance_tab)?;
    let _ = opt_bool_checked(params, &["band_log_spacing"])?;
    apply_usize_field(params, &["max_iters"], &mut spec.max_iters)?;
    if spec.max_iters == 0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`max_iters` must be >= 1"));
    }
    if let Some(groups) = parse_usize_list(params.get("mv_group_columns"))? {
        spec.mv_group_columns = groups;
    }
    if let Some(xs) = parse_usize_list(params.get("mv_x_columns"))? {
        spec.mv_x_columns = xs;
    }
    apply_opt_usize_field(params, &["mv_y_column"], &mut spec.mv_y_column)?;
    Ok(())
}

const SHORTHAND_OVERRIDDEN_PATCH_KEYS: &[&str] = &[
    "method",
    "contingency_method",
    "fit_method",
    "multiplicity",
    "post_hoc",
    "multiplicity_method",
    "tie_method",
    "reference_levels",
    "seed",
    "parallel_analysis_seed",
    "weighting",
    "repeated_measures",
    "residual_covariance",
    "df_method",
    "distance",
    "distance_metric",
    "linkage",
    "linkage_method",
    "family",
    "glm_family",
    "comparison",
    "model_comparison",
    "parameter_rules",
    "x_range_filter",
    "y_range_filter",
    "plot_x_min",
    "plot_x_max",
    "plot_segments",
    "flag_r_squared_below",
    "expected_proportions",
    "null_proportion",
    "x_transform",
    "y_transform",
    "error_propagation",
    "zero_anchor",
    "hundred_anchor",
    "operation",
    "filter_formula",
    "target_archetype",
    "derived_columns",
    "derived",
    "column_formulas",
    "formulas",
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
        .or_else(|| map.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "derived column entry requires non-empty string `title` (or `name`)"))?;
    let formula = map
        .get("formula")
        .or_else(|| map.get("expr"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "derived column entry requires non-empty string `formula` (or `expr`)"))?;
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
            if (map.contains_key("title") || map.contains_key("name")) && (map.contains_key("formula") || map.contains_key("expr")) {
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
            if SHORTHAND_OVERRIDDEN_PATCH_KEYS.contains(&k.as_str()) {
                continue;
            }
            if target_map.contains_key(k) {
                target_map.insert(k.clone(), v.clone());
                patched_any = true;
            }
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

fn parse_cox_tie_method(val: &Value) -> Result<CoxTieMethod, EngineError> {
    if let Ok(parsed) = serde_json::from_value::<CoxTieMethod>(val.clone()) {
        return Ok(parsed);
    }
    let raw = val.as_str().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`tie_method` must be `Efron`, `Breslow`, or `Exact`"))?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "efron" => Ok(CoxTieMethod::Efron),
        "breslow" => Ok(CoxTieMethod::Breslow),
        "exact" => Ok(CoxTieMethod::Exact),
        other => {
            Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown Cox tie method `{other}`; expected `Efron`, `Breslow`, or `Exact`")))
        }
    }
}

fn parse_reference_levels(val: &Value) -> Result<Vec<(usize, String)>, EngineError> {
    let arr = val
        .as_array()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`reference_levels` must be an array of `[column_index, level]` pairs"))?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        if let Some(pair) = item.as_array() {
            if pair.len() != 2 {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("`reference_levels[{idx}]` must be a `[column_index, level]` pair")));
            }
            let col = parse_usize_json_value(&pair[0], "reference_levels column")?;
            let lvl = pair[1]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`reference_levels[{idx}][1]` must be a non-empty string")))?;
            out.push((col, lvl.to_owned()));
        } else if let Some(obj) = item.as_object() {
            let col_val = obj
                .get("column")
                .or_else(|| obj.get("col"))
                .or_else(|| obj.get("column_index"))
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`reference_levels[{idx}]` missing `column` index")))?;
            let col = parse_usize_json_value(col_val, "reference_levels column")?;
            let lvl = obj
                .get("level")
                .or_else(|| obj.get("reference_level"))
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

fn default_cox_survival(params: &Value) -> Result<SurvivalAnalysisSpec, EngineError> {
    let time_column = opt_usize_checked(params, &["time_column"])?.and_then(|(v, is_null)| (!is_null).then_some(v)).unwrap_or(0);
    let status_column = opt_usize_checked(params, &["status_column"])?.and_then(|(v, is_null)| (!is_null).then_some(v)).unwrap_or(1);
    let predictor_columns =
        parse_usize_list(params.get("predictor_columns").or_else(|| params.get("cox_covariate_columns")))?.unwrap_or_else(|| vec![2]);
    let reference_levels = match params.get("reference_levels") {
        Some(v) if !v.is_null() => parse_reference_levels(v)?,
        _ => Vec::new(),
    };
    let tie_method = match params.get("tie_method") {
        Some(v) if !v.is_null() => parse_cox_tie_method(v)?,
        _ => CoxTieMethod::Efron,
    };
    Ok(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, tie_method })
}

fn default_glm(family: GlmFamily) -> GlmSpec {
    GlmSpec { family, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: true }
}

fn default_pca() -> MultivariateSpec {
    MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(42) }
}

fn parse_multiplicity_method(raw: &str) -> Result<MultiplicitySpec, EngineError> {
    let method = match raw.trim().to_ascii_lowercase().as_str() {
        "none" => MultiplicityMethod::None,
        "tukey" | "tukey_hsd" => MultiplicityMethod::TukeyHsd,
        "newman_keuls" | "student_newman_keuls" | "snk" => MultiplicityMethod::NewmanKeuls,
        "fisher_lsd" | "lsd" | "uncorrected_fisher_lsd" => MultiplicityMethod::FisherLsd,
        "dunnett" => MultiplicityMethod::Dunnett { control_column: 0 },
        "sidak" => MultiplicityMethod::Sidak,
        "bonferroni" => MultiplicityMethod::Bonferroni,
        "holm_sidak" => MultiplicityMethod::HolmSidak,
        "dunnett_t3" => MultiplicityMethod::DunnettT3,
        "games_howell" => MultiplicityMethod::GamesHowell,
        "tamhane_t2" => MultiplicityMethod::TamhaneT2,
        "dunn" | "dunn_rank_sum" => MultiplicityMethod::DunnRankSum,
        "linear_trend" => MultiplicityMethod::LinearTrend,
        "benjamini_hochberg" | "bh" | "fdr" => MultiplicityMethod::BenjaminiHochbergFdr,
        "benjamini_yekutieli" | "by" => MultiplicityMethod::BenjaminiYekutieliFdr,
        "benjamini_krieger_yekutieli" | "bky" => MultiplicityMethod::TwoStageBkyFdr,
        other => return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown multiplicity method `{other}`"))),
    };
    Ok(MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true })
}

fn parse_contingency_method(raw: &str) -> Result<ContingencyMethod, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pearson" | "chi_square" | "pearson_chi_square" => Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: false }),
        "yates" | "yates_chi_square" => Ok(ContingencyMethod::PearsonChiSquare { yates_continuity: true }),
        "g2" | "likelihood_ratio_g2" => Ok(ContingencyMethod::LikelihoodRatioG2),
        "fisher" | "fisher_exact" => Ok(ContingencyMethod::FisherExact),
        "barnard" | "barnard_exact" => Ok(ContingencyMethod::BarnardExact),
        "trend" | "chi_square_for_trend" | "cochran_armitage" | "cochran_armitage_trend" => Ok(ContingencyMethod::CochranArmitageTrend),
        "mcnemar" | "mcnemar_paired" => Ok(ContingencyMethod::McNemarPaired),
        "cohen_kappa" | "kappa" | "cohen_kappa_unweighted" => Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted }),
        "cohen_kappa_linear" => Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Linear }),
        "cohen_kappa_quadratic" => Ok(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic }),
        "fleiss_kappa" => Ok(ContingencyMethod::FleissKappa),
        "exact_binomial" | "binomial" => Ok(ContingencyMethod::ExactBinomial),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown contingency method `{other}`"))),
    }
}

fn parse_kappa_weighting_opt(params: &Value) -> Result<Option<KappaWeighting>, EngineError> {
    let Some(val) = params.get("weighting").or_else(|| params.get("kappa_weighting")) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let raw = val
        .as_str()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`weighting` must be a string (`unweighted`, `linear`, or `quadratic`)"))?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "unweighted" | "none" => Ok(Some(KappaWeighting::Unweighted)),
        "linear" => Ok(Some(KappaWeighting::Linear)),
        "quadratic" => Ok(Some(KappaWeighting::Quadratic)),
        other => {
            Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown kappa weighting `{other}`; use `unweighted`, `linear`, or `quadratic`")))
        }
    }
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

fn parse_cluster_distance_opt(params: &Value) -> Result<Option<ClusterDistance>, EngineError> {
    let Some(val) = params.get("distance").or_else(|| params.get("distance_metric")) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    if let Ok(parsed) = serde_json::from_value::<ClusterDistance>(val.clone()) {
        return Ok(Some(parsed));
    }
    let raw = val
        .as_str()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`distance` must be a string (`euclidean`, `manhattan`, `pearson`, `cosine`)"))?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "euclidean" | "l2" => Ok(Some(ClusterDistance::Euclidean)),
        "manhattan" | "l1" => Ok(Some(ClusterDistance::Manhattan)),
        "pearson" | "correlation" => Ok(Some(ClusterDistance::Pearson)),
        "cosine" => Ok(Some(ClusterDistance::Cosine)),
        other => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unknown cluster distance `{other}`; use `euclidean`, `manhattan`, `pearson`, or `cosine`"),
        )),
    }
}

fn parse_cluster_linkage_opt(params: &Value) -> Result<Option<ClusterLinkage>, EngineError> {
    let Some(val) = params.get("linkage").or_else(|| params.get("linkage_method")) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    if let Ok(parsed) = serde_json::from_value::<ClusterLinkage>(val.clone()) {
        return Ok(Some(parsed));
    }
    let raw = val.as_str().ok_or_else(|| {
        EngineError::with_code(
            ErrorCode::BadInput,
            "`linkage` must be a string (`ward`, `complete`, `single`, `upgma`, `wpgma`, `centroid`, `median`)",
        )
    })?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "ward" | "ward_d2" | "ward.d2" => Ok(Some(ClusterLinkage::Ward)),
        "complete" => Ok(Some(ClusterLinkage::Complete)),
        "single" => Ok(Some(ClusterLinkage::Single)),
        "upgma" | "average" => Ok(Some(ClusterLinkage::Upgma)),
        "wpgma" | "weighted_average" => Ok(Some(ClusterLinkage::Wpgma)),
        "centroid" | "upgmc" => Ok(Some(ClusterLinkage::Centroid)),
        "median" | "wpgmc" => Ok(Some(ClusterLinkage::Median)),
        other => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unknown cluster linkage `{other}`; use `ward`, `complete`, `single`, `upgma`, `wpgma`, `centroid`, or `median`"),
        )),
    }
}

fn parse_weighting_scheme(raw: &str) -> Result<WeightingScheme, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "unweighted" => Ok(WeightingScheme::None),
        "relative" | "inverse_y" | "1/y" => Ok(WeightingScheme::Relative),
        "relative_square" | "inverse_y2" | "1/y2" | "relative_1_y2" => Ok(WeightingScheme::RelativeSquare),
        "inverse_x" | "1/x" => Ok(WeightingScheme::InverseX),
        "inverse_x2" | "1/x2" => Ok(WeightingScheme::InverseX2),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown weighting scheme `{other}`"))),
    }
}

fn parse_fit_method(raw: &str) -> Result<FitMethod, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ordinary" | "ols" | "least_squares" => Ok(FitMethod::LeastSquares),
        "poisson" | "poisson_deviance" => Ok(FitMethod::PoissonDeviance),
        "robust" | "cauchy" => Ok(FitMethod::RobustCauchy),
        "rout" => Ok(FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 }),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown fit method `{other}`"))),
    }
}

fn parse_math_preset(raw: Option<&str>) -> Result<MathTransformPreset, EngineError> {
    match raw.unwrap_or("log10").trim().to_ascii_lowercase().as_str() {
        "log10_y" | "log10" => Ok(MathTransformPreset::Log10),
        "ln_y" | "ln" => Ok(MathTransformPreset::Ln),
        "log2_y" | "log2" => Ok(MathTransformPreset::Log2),
        "exp10_y" | "exp10" | "10_pow_y" => Ok(MathTransformPreset::Exp10),
        "exp_y" | "exp" | "e_pow_y" => Ok(MathTransformPreset::Exp),
        "reciprocal_y" | "reciprocal" | "1/y" => Ok(MathTransformPreset::Reciprocal),
        "sqrt" => Ok(MathTransformPreset::Sqrt),
        "square" => Ok(MathTransformPreset::Square),
        "logit" => Ok(MathTransformPreset::Logit),
        "probit" => Ok(MathTransformPreset::Probit),
        "arcsin_sqrt" => Ok(MathTransformPreset::ArcsinSqrt),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown math transform preset `{other}`"))),
    }
}

fn parse_error_prop(raw: Option<&str>) -> Result<ErrorPropagationMode, EngineError> {
    match raw.unwrap_or("asymmetric").trim().to_ascii_lowercase().as_str() {
        "asymmetric" | "replicate_endpoints" => Ok(ErrorPropagationMode::ReplicateEndpoints),
        "delta" | "delta_method" | "first_order_delta" => Ok(ErrorPropagationMode::FirstOrderDeltaMethod),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown error propagation mode `{other}`"))),
    }
}

fn parse_normalize_anchor(raw: &str) -> Result<NormalizeAnchor, EngineError> {
    match raw.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "smallest" | "min" | "column_min" | "columnmin" => Ok(NormalizeAnchor::ColumnMin),
        "largest" | "max" | "column_max" | "columnmax" => Ok(NormalizeAnchor::ColumnMax),
        "first" | "first_row" | "firstrow" => Ok(NormalizeAnchor::FirstRow),
        "last" | "last_row" | "lastrow" => Ok(NormalizeAnchor::LastRow),
        "sum" | "column_sum" | "columnsum" => Ok(NormalizeAnchor::ColumnSum),
        _ => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unknown normalize anchor `{raw}` (expected smallest, largest, first, last, or sum)"),
        )),
    }
}

fn parse_baseline_op(raw: &str) -> Result<BaselineOperation, EngineError> {
    match raw.trim().to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "difference" | "subtract" => Ok(BaselineOperation::Difference),
        "ratio" | "divide" => Ok(BaselineOperation::Ratio),
        "percent" | "percent_change" | "pct_change" | "pctchange" | "percentchange" => Ok(BaselineOperation::PercentChange),
        "fraction" | "fractional_change" | "fractionalchange" => Ok(BaselineOperation::FractionalChange),
        _ => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unknown baseline operation `{raw}` (expected difference, ratio, percent_change, or fractional_change)"),
        )),
    }
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

fn opt_str_checked<'a>(params: &'a Value, keys: &[&str]) -> Result<Option<&'a str>, EngineError> {
    for key in keys {
        if let Some(val) = params.get(*key) {
            if val.is_null() {
                return Ok(None);
            }
            let s = val.as_str().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`{key}` must be a string or null")))?;
            return Ok(Some(s));
        }
    }
    Ok(None)
}

fn opt_bool_checked(params: &Value, keys: &[&str]) -> Result<Option<bool>, EngineError> {
    for key in keys {
        if let Some(val) = params.get(*key) {
            if val.is_null() {
                return Ok(None);
            }
            let b = val.as_bool().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`{key}` must be a boolean or null")))?;
            return Ok(Some(b));
        }
    }
    Ok(None)
}

fn opt_f64_checked(params: &Value, keys: &[&str]) -> Result<Option<f64>, EngineError> {
    for key in keys {
        if let Some(val) = params.get(*key) {
            return parse_nullable_finite_f64(val, key);
        }
    }
    Ok(None)
}

fn parse_named_params_vec(val: Option<&Value>) -> Result<Vec<(String, f64)>, EngineError> {
    let mut vec = Vec::new();
    let Some(val) = val else {
        return Ok(vec);
    };
    if val.is_null() {
        return Ok(vec);
    }
    let obj =
        val.as_object().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`params` must be an object of numeric parameter overrides"))?;
    for (k, v) in obj {
        let num = v
            .as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("parameter `{k}` must be numeric")))?;
        vec.push((k.clone(), num));
    }
    Ok(vec)
}

fn apply_bool_opt_checked(params: &Value, keys: &[&str], target: &mut bool) -> Result<(), EngineError> {
    if let Some(b) = opt_bool_checked(params, keys)? {
        *target = b;
    }
    Ok(())
}

fn opt_usize_checked(params: &Value, keys: &[&str]) -> Result<Option<(usize, bool)>, EngineError> {
    for key in keys {
        if let Some(v) = params.get(*key) {
            if v.is_null() {
                return Ok(Some((0, true)));
            }
            let idx = parse_usize_json_value(v, key)?;
            return Ok(Some((idx, false)));
        }
    }
    Ok(None)
}

fn apply_usize_field(params: &Value, keys: &[&str], target: &mut usize) -> Result<(), EngineError> {
    if let Some((idx, is_null)) = opt_usize_checked(params, keys)? {
        if is_null {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{}` must be a non-negative integer", keys[0])));
        }
        *target = idx;
    }
    Ok(())
}

fn apply_opt_usize_field(params: &Value, keys: &[&str], target: &mut Option<usize>) -> Result<(), EngineError> {
    if let Some((idx, is_null)) = opt_usize_checked(params, keys)? {
        *target = if is_null { None } else { Some(idx) };
    }
    Ok(())
}

fn apply_opt_f64_field(params: &Value, keys: &[&str], target: &mut Option<f64>) -> Result<(), EngineError> {
    for key in keys {
        if let Some(v) = params.get(*key) {
            *target = parse_nullable_finite_f64(v, key)?;
            return Ok(());
        }
    }
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
        if arr.len() != 2 {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{key}` array must have [min, max] entries")));
        }
        let min_v = parse_nullable_finite_f64(&arr[0], key)?;
        let max_v = parse_nullable_finite_f64(&arr[1], key)?;
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
        && !obj.contains_key("kind")
    {
        return deserialize_strict(val, "/comparison").map(Some);
    }
    let (method_raw, alt_id) = if let Some(s) = val.as_str() {
        let alt = opt_str_checked(params, &["alternative_equation_id", "alternative_model"])?
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`comparison` shorthand string requires `alternative_equation_id`"))?;
        (s, alt.to_owned())
    } else if let Some(obj) = val.as_object() {
        for k in obj.keys() {
            if !matches!(k.as_str(), "method" | "kind" | "alternative_equation_id" | "alternative_model" | "equation_id") {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown field `{k}` in `comparison`")));
            }
        }
        let m = opt_str_checked(val, &["method", "kind"])?
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`comparison` object missing `method`"))?;
        let alt = match opt_str_checked(val, &["alternative_equation_id", "alternative_model", "equation_id"])? {
            Some(s) => Some(s),
            None => opt_str_checked(params, &["alternative_equation_id"])?,
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
    map.contains_key("parameter")
        || map.contains_key("value")
        || map.contains_key("values")
        || map.contains_key("constants")
        || map.contains_key("per_dataset_constants")
        || map.contains_key("hook")
        || map.contains_key("source")
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
        if !matches!(
            k.as_str(),
            "name"
                | "parameter"
                | "scope"
                | "initial_override"
                | "lower_bound"
                | "upper_bound"
                | "value"
                | "values"
                | "constants"
                | "per_dataset_constants"
                | "hook"
                | "source"
        ) {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown field `{k}` in `parameter_rules` entry")));
        }
    }
    if !is_shorthand_parameter_rule_obj(map) {
        return deserialize_strict(item, "/parameter_rules");
    }
    let name = opt_str_checked(item, &["name", "parameter"])?
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "parameter rule missing string `name`"))?
        .to_owned();
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
            let src = item
                .get("values")
                .or_else(|| item.get("constants"))
                .or_else(|| item.get("per_dataset_constants"))
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`per_dataset_constant` scope requires `values` or `constants`"))?;
            Ok(ParameterScope::PerDatasetConstant(parse_per_dataset_constants(src)?))
        }
        "hooked_constant" | "hooked" => {
            let hook_val = item
                .get("hook")
                .or_else(|| item.get("source"))
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`hooked_constant` scope requires `hook` or `source`"))?;
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
                if pair.len() != 2 {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "per-dataset constant pair must have [dataset_index, value]"));
                }
                let ds = parse_usize_json_value(&pair[0], "dataset_index")?;
                let num = pair[1]
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
            .execute("stats.run", json!({ "source_sheet": "col1", "sheet_id": "t1", "kind": "unpaired_t" }))
            .expect("unpaired t-test must succeed");
        assert!(res.mutated);
        assert_eq!(res.payload["sheet_id"], json!("t1"));

        let out_dir = std::env::temp_dir().join(format!("cb_ana_test_{}", std::process::id()));
        let report_path = out_dir.join("methods.md");
        let rep = session
            .execute("result.generateReport", json!({ "sheet_id": "t1", "format": "markdown", "path": report_path.to_string_lossy() }))
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
                    "source_sheet": "col3",
                    "sheet_id": "anova1",
                    "kind": "one_way_anova",
                    "confidence_level": 95.0,
                    "multiplicity": "tukey",
                    "threshold": 0.01,
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
            .execute("analysis.run", json!({ "source_sheet": "xy1", "sheet_id": "ba1", "kind": "bland_altman" }))
            .expect("single-dataset XY Bland-Altman (0, 1) must succeed");
        for bad in [json!({"column_a": 99, "column_b": 99}), json!({"column_a": 0, "column_b": 0}), json!({"column_a": 0, "column_b": 2})] {
            let mut req = json!({ "source_sheet": "xy1", "kind": "bland_altman" });
            req.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
            let err = session.execute("analysis.run", req).expect_err("invalid single-XY Bland-Altman must fail");
            assert_eq!(err.code, ErrorCode::BadInput);
        }

        // Audit #4: Bland-Altman on 2-dataset XY accepts (0, 1) and rejects equal or out-of-range indices.
        session
            .execute("analysis.run", json!({ "source_sheet": "xy2", "sheet_id": "ba2", "kind": "bland_altman", "column_a": 0, "column_b": 1 }))
            .expect("2-dataset XY Bland-Altman (0, 1) must succeed");
        for bad in [json!({"column_a": 1, "column_b": 1}), json!({"column_a": 0, "column_b": 2})] {
            let mut req = json!({ "source_sheet": "xy2", "kind": "bland_altman" });
            req.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
            let err = session.execute("analysis.run", req).expect_err("invalid multi-XY Bland-Altman must fail");
            assert_eq!(err.code, ErrorCode::BadInput);
        }

        // Audit #13: XY GLM rejects pre-summarized XY tables and invalid column selections.
        let sum_err =
            session.execute("analysis.run", json!({ "source_sheet": "xysum", "kind": "linear_glm" })).expect_err("pre-summarized XY GLM must fail");
        assert_eq!(sum_err.code, ErrorCode::BadInput);

        session
            .execute(
                "analysis.run",
                json!({ "source_sheet": "xy2", "sheet_id": "glm-ok", "kind": "linear_glm", "selected_data_sets": [1], "response_column": 1, "predictor_columns": [0] }),
            )
            .expect("valid XY GLM with dataset 1 and X predictor 0 must succeed");
        session
            .execute("analysis.run", json!({ "source_sheet": "xy2", "sheet_id": "glm-default", "kind": "linear_glm", "selected_data_sets": [1] }))
            .expect("default_glm placeholder [1] + selected_data_sets [1] must remain valid");

        let bad_pred = session
            .execute(
                "analysis.run",
                json!({ "source_sheet": "xy2", "kind": "linear_glm", "selected_data_sets": [1], "response_column": 1, "predictor_columns": [99] }),
            )
            .expect_err("impossible predictor [99] on XY GLM must fail");
        assert_eq!(bad_pred.code, ErrorCode::BadInput);

        let empty_pred = session
            .execute("analysis.run", json!({ "source_sheet": "xy2", "kind": "linear_glm", "predictor_columns": [] }))
            .expect_err("empty predictor_columns must fail with InsufficientData");
        assert_eq!(empty_pred.code, ErrorCode::InsufficientData);

        let mismatch_resp = session
            .execute("analysis.run", json!({ "source_sheet": "xy2", "kind": "linear_glm", "selected_data_sets": [0], "response_column": 1 }))
            .expect_err("mismatched response_column must fail");
        assert_eq!(mismatch_resp.code, ErrorCode::BadInput);

        let update_err = session
            .execute("analysis.updateParams", json!({ "sheet_id": "glm-ok", "params": { "predictor_columns": [99] } }))
            .expect_err("updateParams with invalid predictor_columns must fail cleanly");
        assert_eq!(update_err.code, ErrorCode::BadInput);
    }
}
