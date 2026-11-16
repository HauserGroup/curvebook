//! Scientific transform, statistical test, curve fit, and result reporting/export commands.

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, BlandAltmanSpec, ClusterDistance, ClusterLinkage, ContingencyMethod, ContingencySpec,
    CoxTieMethod, DescriptiveSpec, ErrorCode, ErrorPropagationMode, FactorialAnovaSpec, FitMethod, GlmFamily, GlmSpec, HookSource, KappaWeighting,
    LinearRegressionSpec, MathTransformPreset, ModelComparisonSpec, MultipleRowTestsSpec, MultiplicityMethod, MultiplicitySpec, MultivariateSpec,
    NestedAnalysisSpec, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, OneWayMethod, OutlierSpec, PValueStackMethod, PValueStackSpec,
    ParameterRule, ParameterScope, PassingBablokSpec, ProjectDocument, RepeatedMeasuresDesign, ResultSheet, ResultStatus, RocCiMethod, RocSpec,
    SheetId, SimulationNoise, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec,
    VariableRole, WeightingScheme,
};
use lumencraft_fit::get_builtin_model;
use lumencraft_format::{CsvOptions, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv};
use lumencraft_math::PValueFormat;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::export::{de_opt_usize, write_atomic_output_file};
use super::helpers::{next_unique_sheet_id, parse_sheet_id, require_analyzable_data_sheet, require_result_sheet};
use super::params::{
    any_json_schema, array_schema, bool_schema, int_schema, non_empty_str_schema, nullable_schema, num_schema, obj_schema_with_aliases, parse_params,
    str_schema,
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

fn parse_usize_json_value(val: &Value, label: &str) -> Result<usize, EngineError> {
    if let Some(raw) = val.as_u64() {
        return usize::try_from(raw).map_err(|_| EngineError::with_code(ErrorCode::BadInput, format!("`{label}` exceeds platform index bounds")));
    }
    const MAX_SAFE_INT_F64: f64 = 9_007_199_254_740_991.0;
    if let Some(f) = val.as_f64()
        && f.is_finite()
        && f >= 0.0
        && f.fract() == 0.0
        && f <= MAX_SAFE_INT_F64
    {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let u = f as u64;
        return usize::try_from(u).map_err(|_| EngineError::with_code(ErrorCode::BadInput, format!("`{label}` exceeds platform index bounds")));
    }
    Err(EngineError::with_code(ErrorCode::BadInput, format!("`{label}` must be a non-negative integer")))
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
        props.push(("x_range_filter", nullable_schema(any_json_schema("Optional [min, max] X inclusion filter"))));
        props.push(("y_range_filter", nullable_schema(any_json_schema("Optional [min, max] Y inclusion filter"))));
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
    apply_metadata_updates(
        &mut updated,
        parsed.title.as_ref().map(Option::as_deref),
        parsed.confidence_percent,
        parsed.selected_data_sets.as_deref(),
        parsed.p_value_format.as_ref(),
    )?;

    if let Some(ref kind_val) = parsed.kind {
        let mut new_kind = if kind_val.is_object() {
            serde_json::from_value(kind_val.clone())
                .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid typed `kind` object: {err}")))?
        } else {
            let name = kind_val
                .as_str()
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`kind` must be string or object"))?
                .trim()
                .to_ascii_lowercase();
            resolve_kind_shorthand(&name, &combined)?
        };
        patch_analysis_kind_fields(&mut new_kind, &combined)?;
        apply_shorthand_kind_overrides(&mut new_kind, &combined)?;
        updated.update_kind(new_kind);
    } else if let Some(ref patch_obj) = parsed.params {
        let mut kind_copy = updated.kind.clone();
        patch_analysis_kind_fields(&mut kind_copy, patch_obj)?;
        apply_shorthand_kind_overrides(&mut kind_copy, patch_obj)?;
        updated.update_kind(kind_copy);
    } else {
        let mut kind_copy = updated.kind.clone();
        patch_analysis_kind_fields(&mut kind_copy, &combined)?;
        apply_shorthand_kind_overrides(&mut kind_copy, &combined)?;
        updated.update_kind(kind_copy);
    }

    if !updated.kind.is_compatible_with(source_kind) {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("updated analysis kind is incompatible with table type `{}`", source_kind.label()),
        )
        .with_remediation("Choose an analysis kind compatible with the source table archetype."));
    }

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
        ResultStatus::Failed { code, message } => Err(EngineError::with_code(*code, format!("analysis `{}` failed ({code}): {message}", res.id))
            .with_details(json!({ "sheet_id": res.id.as_str(), "error_code": code }))),
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
    combined: &Value,
    domain: AnalysisDomain,
) -> Result<Option<AnalysisSpec>, EngineError> {
    if let Some(ref spec_val) = parsed.spec {
        let spec: AnalysisSpec = serde_json::from_value(spec_val.clone()).map_err(|err| {
            EngineError::with_code(ErrorCode::BadInput, format!("invalid `spec` object: {err}")).with_details(json!({ "pointer": "/spec" }))
        })?;
        let table_kind = require_analyzable_data_sheet(doc, &spec.source_sheet_id)?.kind();
        validate_domain_and_compat(&spec.kind, domain, table_kind)?;
        return Ok(Some(spec));
    }

    if let (Some(id_str), Some(src_str), Some(kind_val)) = (parsed.sheet_id.as_deref(), parsed.source_sheet_id.as_deref(), parsed.kind.as_ref())
        && kind_val.is_object()
    {
        let mut obj = combined.as_object().cloned().unwrap_or_default();
        obj.insert(String::from("id"), Value::String(id_str.to_owned()));
        obj.insert(String::from("source_sheet_id"), Value::String(src_str.to_owned()));
        if let Ok(spec) = serde_json::from_value::<AnalysisSpec>(Value::Object(obj)) {
            let table_kind = require_analyzable_data_sheet(doc, &spec.source_sheet_id)?.kind();
            validate_domain_and_compat(&spec.kind, domain, table_kind)?;
            return Ok(Some(spec));
        }
    }
    Ok(None)
}

fn build_analysis_spec(
    doc: &ProjectDocument,
    parsed: &RunAnalysisParams,
    combined: &Value,
    domain: AnalysisDomain,
) -> Result<AnalysisSpec, EngineError> {
    if let Some(spec) = try_raw_analysis_spec(doc, parsed, combined, domain)? {
        return Ok(spec);
    }

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

    let kind = parse_analysis_kind(parsed.kind.as_ref(), combined, domain, table_kind)?;
    validate_domain_and_compat(&kind, domain, table_kind)?;

    let mut spec = AnalysisSpec::new(id, source_sheet_id, kind)?;
    apply_metadata_updates(
        &mut spec,
        parsed.title.as_ref().map(Option::as_deref),
        parsed.confidence_percent,
        parsed.selected_data_sets.as_deref(),
        parsed.p_value_format.as_ref(),
    )?;
    if matches!(spec.kind, AnalysisKind::Multivariate(_)) && spec.selected_data_sets.is_empty() {
        spec.selected_data_sets = default_multivariate_numeric_columns(doc, &spec.source_sheet_id);
    }
    Ok(spec)
}

fn default_multivariate_numeric_columns(doc: &ProjectDocument, sheet_id: &SheetId) -> Vec<usize> {
    let Ok(sheet) = require_analyzable_data_sheet(doc, sheet_id) else {
        return Vec::new();
    };
    let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
        return Vec::new();
    };
    let continuous: Vec<usize> = mv
        .variables
        .iter()
        .enumerate()
        .filter(|(_, v)| v.role == VariableRole::Continuous && v.cells.iter().any(|c| c.active_number().is_some_and(f64::is_finite)))
        .map(|(i, _)| i)
        .collect();
    if continuous.len() >= 2 && continuous.len() < mv.variables.len() {
        return continuous;
    }
    let numeric: Vec<usize> = mv
        .variables
        .iter()
        .enumerate()
        .filter(|(_, v)| v.cells.iter().any(|c| c.active_number().is_some_and(f64::is_finite)))
        .map(|(i, _)| i)
        .collect();
    if numeric.len() >= 2 && numeric.len() < mv.variables.len() {
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

fn parse_analysis_kind(kind_val: Option<&Value>, params: &Value, domain: AnalysisDomain, table_kind: TableKind) -> Result<AnalysisKind, EngineError> {
    if let Some(kv) = kind_val
        && kv.is_object()
    {
        let mut kind: AnalysisKind =
            serde_json::from_value(kv.clone()).map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `kind` object: {err}")))?;
        apply_shorthand_kind_overrides(&mut kind, params)?;
        return Ok(kind);
    }

    let raw_kind = kind_val.and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase());
    let mut kind = match raw_kind.as_deref() {
        None => infer_default_kind(params, domain, table_kind)?,
        Some(name) => resolve_kind_shorthand(name, params)?,
    };
    patch_analysis_kind_fields(&mut kind, params)?;
    apply_shorthand_kind_overrides(&mut kind, params)?;
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
            TableKind::Nested => {
                Ok(AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: false, multiplicity: MultiplicitySpec::default() }))
            }
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
    let y_transform = Some(parse_math_preset(opt_str(params, &["y_transform", "preset", "operation"]))?);
    let x_transform = opt_str(params, &["x_transform"]).map(|s| parse_math_preset(Some(s))).transpose()?;
    let error_propagation = parse_error_prop(opt_str(params, &["error_propagation", "propagate_error"]))?;
    Ok(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation }))
}

fn resolve_transform_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let tr = match name {
        "math" | "transform.math" | "math_transform" => return resolve_transform_math(params).map(Some),
        "normalize" | "transform.normalize" => TransformAnalysisSpec::Normalize {
            zero_anchor: parse_normalize_anchor(opt_str(params, &["zero_anchor", "zero"]).unwrap_or("smallest")),
            hundred_anchor: parse_normalize_anchor(opt_str(params, &["hundred_anchor", "hundred"]).unwrap_or("largest")),
            output_percent: !params.get("as_fraction").and_then(Value::as_bool).unwrap_or(false),
            per_data_set: params.get("per_data_set").and_then(Value::as_bool).unwrap_or(true),
        },
        "remove_baseline" | "baseline" | "transform.remove_baseline" => TransformAnalysisSpec::RemoveBaseline {
            baseline_columns: parse_usize_list(params.get("baseline_columns"))?.unwrap_or_else(|| vec![0]),
            operation: parse_baseline_op(opt_str(params, &["operation"]).unwrap_or("difference")),
        },
        "transpose" | "transform.transpose" => TransformAnalysisSpec::Transpose {
            swap_row_and_column_titles: params.get("swap_row_and_column_titles").and_then(Value::as_bool).unwrap_or(true),
        },
        "prune_rows" | "prune" | "transform.prune_rows" => TransformAnalysisSpec::PruneRows {
            every_k_rows: usize_opt(params, "every_k_rows").or_else(|| usize_opt(params, "step")).unwrap_or(2).max(1),
            average_window: params.get("average_window").or_else(|| params.get("average_dropped")).and_then(Value::as_bool).unwrap_or(false),
        },
        "select_and_transform" | "select" | "transform.select_and_transform" => TransformAnalysisSpec::SelectAndTransform {
            filter_formula: opt_str(params, &["filter_formula"]).map(str::to_owned),
            target_archetype: TableKind::Xy,
        },
        "simulate" | "simulate_xy" | "transform.simulate" => TransformAnalysisSpec::Simulate {
            seed: params.get("seed").and_then(Value::as_u64).unwrap_or(42),
            row_count: usize_opt(params, "row_count").or_else(|| usize_opt(params, "n_points")).unwrap_or(13).max(2),
            replicates: usize_opt(params, "replicates").unwrap_or(3).max(1),
            x_min: params.get("x_min").and_then(Value::as_f64).unwrap_or(-9.0),
            x_max: params.get("x_max").and_then(Value::as_f64).unwrap_or(-3.0),
            equation_id: opt_str(params, &["equation_id"]).unwrap_or("log_agonist_vs_response_4pl").to_owned(),
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
                seed: params.get("seed").and_then(Value::as_u64).unwrap_or(42),
                sample_size,
                stratify_column,
                proportional_allocation: params.get("proportional_allocation").and_then(Value::as_bool).unwrap_or(true),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(AnalysisKind::Transform(tr)))
}

fn resolve_two_group_or_anova_shorthand(name: &str, params: &Value) -> Result<Option<AnalysisKind>, EngineError> {
    let hyp = params.get("hypothetical").or_else(|| params.get("hypothetical_mean")).and_then(Value::as_f64).unwrap_or(0.0);
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
            let gg = params.get("geisser_greenhouse").and_then(Value::as_bool).unwrap_or(true);
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
        "outliers" | "outlier" | "rout" => {
            AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: params.get("q_percent").and_then(Value::as_f64).unwrap_or(1.0) })
        }
        "grubbs" => AnalysisKind::OutlierDetection(OutlierSpec::Grubbs {
            alpha: params.get("alpha").and_then(Value::as_f64).unwrap_or(0.05),
            iterative: params.get("iterative").and_then(Value::as_bool).unwrap_or(false),
        }),
        "nested_t" | "nested_anova" | "nested" => {
            AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: name == "nested_t", multiplicity: MultiplicitySpec::default() })
        }
        "multiple_row_t_tests" | "stats.multiple_row_t_tests" | "multiple_row_tests" | "multiple_t_tests" => {
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default())
        }
        "bland_altman" | "stats.bland_altman" => AnalysisKind::BlandAltman(BlandAltmanSpec::default()),
        "passing_bablok" | "stats.passing_bablok" => AnalysisKind::PassingBablok(PassingBablokSpec::default()),
        "pvalue_stack" | "stats.pvalue_stack" | "p_value_stack" | "pvalues" | "pvaluestack" | "stats.pvaluestack" => {
            AnalysisKind::PValueStack(PValueStackSpec::default())
        }
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
                _ => match opt_str(params, &["method", "contingency_method"]) {
                    Some(raw) => parse_contingency_method(raw)?,
                    None => ContingencyMethod::PearsonChiSquare { yates_continuity: false },
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
            spearman_rank: params.get("spearman").and_then(Value::as_bool).unwrap_or(false),
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
        "multiple_imputation" | "stats.multiple_imputation" | "mice" => AnalysisKind::Multivariate(default_multiple_imputation()),
        _ => return Ok(None),
    };
    Ok(Some(kind))
}

fn resolve_fit_shorthand(name: &str, params: &Value) -> Result<AnalysisKind, EngineError> {
    if matches!(name, "linear" | "simple_linear" | "linear_regression" | "deming" | "deming_regression") {
        let deming_model_ii = matches!(name, "deming" | "deming_regression") || params.get("deming").and_then(Value::as_bool).unwrap_or(false);
        return Ok(AnalysisKind::LinearRegression(LinearRegressionSpec { deming_model_ii, ..LinearRegressionSpec::default() }));
    }

    let eq_id = match name {
        "four_parameter_logistic" | "4pl" | "4pl_agonist" => "log_agonist_vs_response_4pl",
        "4pl_inhibitor" | "four_parameter_logistic_inhibitor" | "ic50" => "log_inhibitor_vs_response_4pl",
        "agonist_4pl" => "agonist_vs_response_4pl",
        "inhibitor_4pl" => "inhibitor_vs_response_4pl",
        "3pl" | "three_parameter_logistic" => "log_agonist_vs_response_3pl",
        "3pl_inhibitor" => "log_inhibitor_vs_response_3pl",
        "one_site_binding" | "saturation_binding" => "one_site_specific_binding",
        "nonlinear" | "nonlinear_regression" => opt_str(params, &["equation_id"]).unwrap_or("log_agonist_vs_response_4pl"),
        other => {
            if get_builtin_model(other).is_some() {
                other
            } else if params.get("custom_dsl").and_then(Value::as_str).is_some() {
                "custom_dsl"
            } else {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown analysis kind or equation `{other}`"))
                    .with_remediation("Supply a supported analysis kind or built-in curve fit equation identifier.")
                    .with_details(json!({ "pointer": "/kind", "kind": other })));
            }
        }
    };
    let explicit_eq = opt_str(params, &["equation_id"]).unwrap_or(eq_id);
    let mut fit = NonlinearFitSpec::new(explicit_eq);
    if let Some(dsl) = opt_str(params, &["custom_dsl"]) {
        fit.custom_dsl = Some(dsl.to_owned());
    }
    Ok(AnalysisKind::NonlinearRegression(fit))
}

fn apply_shorthand_kind_overrides(kind: &mut AnalysisKind, params: &Value) -> Result<(), EngineError> {
    match kind {
        AnalysisKind::Transform(spec) => apply_transform_overrides(spec, params)?,
        AnalysisKind::OneWayAnova(spec) => apply_multiplicity_override(&mut spec.multiplicity, params)?,
        AnalysisKind::FactorialAnova(spec) => apply_multiplicity_override(&mut spec.multiplicity, params)?,
        AnalysisKind::NestedComparison(spec) => apply_multiplicity_override(&mut spec.multiplicity, params)?,
        AnalysisKind::MultipleRowTTests(spec) => apply_multiple_row_overrides(spec, params)?,
        AnalysisKind::BlandAltman(spec) => apply_bland_altman_overrides(spec, params)?,
        AnalysisKind::PassingBablok(spec) => apply_passing_bablok_overrides(spec, params)?,
        AnalysisKind::PValueStack(spec) => apply_pvalue_stack_overrides(spec, params)?,
        AnalysisKind::TwoGroupComparison(spec) => apply_two_group_overrides(spec, params)?,
        AnalysisKind::Contingency(spec) => apply_contingency_overrides(spec, params)?,
        AnalysisKind::Survival(spec) => apply_survival_overrides(spec, params)?,
        AnalysisKind::Multivariate(spec) => apply_multivariate_overrides(spec, params)?,
        AnalysisKind::NonlinearRegression(spec) => apply_nonlinear_overrides(spec, params)?,
        AnalysisKind::LinearRegression(spec) => {
            apply_bool_opt(params, &["deming", "deming_model_ii"], &mut spec.deming_model_ii);
            apply_bool_opt(params, &["force_through_origin"], &mut spec.force_through_origin);
        }
        AnalysisKind::GeneralizedLinearModel(spec) => {
            if let Some(resp) = usize_opt(params, "response_column") {
                spec.response_column = resp;
            }
            if let Some(preds) = parse_usize_list(params.get("predictor_columns"))? {
                spec.predictor_columns = preds;
            }
        }
        _ => {}
    }
    Ok(())
}

fn apply_two_group_overrides(spec: &mut TwoGroupSpec, params: &Value) -> Result<(), EngineError> {
    if let Some(two_tailed) = params.get("two_tailed").and_then(Value::as_bool) {
        spec.two_tailed = two_tailed;
    } else if let Some(one_tailed) = params.get("one_tailed").and_then(Value::as_bool) {
        spec.two_tailed = !one_tailed;
    }
    apply_bool_opt(params, &["compute_effect_sizes"], &mut spec.compute_effect_sizes);
    apply_opt_usize_field(params, &["mv_group_column", "group_column"], &mut spec.mv_group_column)?;
    apply_opt_usize_field(params, &["mv_response_column", "response_column"], &mut spec.mv_response_column)?;
    Ok(())
}

fn apply_multiple_row_overrides(spec: &mut MultipleRowTestsSpec, params: &Value) -> Result<(), EngineError> {
    apply_bool_opt(params, &["paired"], &mut spec.paired);
    apply_bool_opt(params, &["nonparametric"], &mut spec.nonparametric);
    apply_bool_opt(params, &["assume_equal_sd"], &mut spec.assume_equal_sd);
    apply_multiplicity_override(&mut spec.multiplicity, params)
}

fn apply_bland_altman_overrides(spec: &mut BlandAltmanSpec, params: &Value) -> Result<(), EngineError> {
    apply_usize_field(params, &["column_a"], &mut spec.column_a)?;
    apply_usize_field(params, &["column_b"], &mut spec.column_b)?;
    apply_bool_opt(params, &["percent_difference"], &mut spec.percent_difference);
    Ok(())
}

fn apply_passing_bablok_overrides(spec: &mut PassingBablokSpec, params: &Value) -> Result<(), EngineError> {
    apply_usize_field(params, &["x_column"], &mut spec.x_column)?;
    apply_usize_field(params, &["y_column"], &mut spec.y_column)?;
    Ok(())
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

fn apply_transform_overrides(spec: &mut TransformAnalysisSpec, params: &Value) -> Result<(), EngineError> {
    if let TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation } = spec {
        if let Some(seed_val) = params.get("seed")
            && !seed_val.is_null()
        {
            *seed = seed_val.as_u64().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`seed` must be an unsigned integer"))?;
        }
        apply_usize_field(params, &["sample_size", "n_samples"], sample_size)?;
        if *sample_size == 0 {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`sample_size` must be >= 1"));
        }
        apply_opt_usize_field(params, &["stratify_column"], stratify_column)?;
        apply_bool_opt(params, &["proportional_allocation"], proportional_allocation);
    }
    Ok(())
}

fn apply_contingency_overrides(spec: &mut ContingencySpec, params: &Value) -> Result<(), EngineError> {
    if let Some(m_val) = params.get("method").or_else(|| params.get("contingency_method")) {
        if let Ok(parsed) = serde_json::from_value::<ContingencyMethod>(m_val.clone()) {
            spec.method = parsed;
        } else if let Some(raw) = m_val.as_str() {
            spec.method = parse_contingency_method(raw)?;
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "contingency `method` must be a string or method object"));
        }
    }
    if let ContingencyMethod::PearsonChiSquare { yates_continuity } = &mut spec.method {
        apply_bool_opt(params, &["yates_continuity", "yates"], yates_continuity);
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
    if let Some(name) = opt_str(params, &["multiplicity", "post_hoc"]) {
        *mult = parse_multiplicity_method(name)?;
    }
    if let Some(t) = params.get("alpha").or_else(|| params.get("threshold")).and_then(Value::as_f64) {
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
            apply_bool_opt(params, &["log_rank_mantel_cox", "log_rank"], log_rank_mantel_cox);
            apply_bool_opt(params, &["gehan_breslow_wilcoxon", "gehan"], gehan_breslow_wilcoxon);
            apply_bool_opt(params, &["trend_test"], trend_test);
            apply_bool_opt(params, &["asymmetrical_greenwood_ci"], asymmetrical_greenwood_ci);
            apply_opt_usize_field(params, &["mv_time_column", "time_column"], mv_time_column)?;
            apply_opt_usize_field(params, &["mv_status_column", "status_column"], mv_status_column)?;
            apply_opt_usize_field(params, &["mv_group_column", "group_column"], mv_group_column)?;
            Ok(())
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, .. } => {
            apply_usize_field(params, &["time_column"], time_column)?;
            apply_usize_field(params, &["status_column"], status_column)?;
            if let Some(preds) = parse_usize_list(params.get("predictor_columns").or_else(|| params.get("cox_covariate_columns")))? {
                *predictor_columns = preds;
            }
            Ok(())
        }
    }
}

fn apply_multivariate_overrides(spec: &mut MultivariateSpec, params: &Value) -> Result<(), EngineError> {
    match spec {
        MultivariateSpec::Pca { standardize, .. } => {
            apply_bool_opt(params, &["standardize"], standardize);
        }
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            apply_bool_opt(params, &["spearman", "spearman_rank"], spearman_rank);
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
            if *imputations_m == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`imputations_m` must be >= 1"));
            }
            apply_usize_field(params, &["max_iters", "iterations"], max_iters)?;
            if *max_iters == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`max_iters` must be >= 1"));
            }
            if let Some(s) = params.get("seed").and_then(Value::as_u64) {
                *seed = s;
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
    if let Some(eq) = opt_str(params, &["equation_id"]) {
        spec.equation_id = eq.to_owned();
    }
    if let Some(dsl_val) = params.get("custom_dsl") {
        spec.custom_dsl = if dsl_val.is_null() {
            None
        } else {
            Some(dsl_val.as_str().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`custom_dsl` must be string or null"))?.to_owned())
        };
    }
    if let Some(w) = opt_str(params, &["weighting"]) {
        spec.weighting = parse_weighting_scheme(w)?;
    }
    if let Some(m) = opt_str(params, &["method", "fit_method"]) {
        spec.method = parse_fit_method(m)?;
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
    apply_bool_opt(params, &["confidence_bands"], &mut spec.confidence_bands);
    apply_bool_opt(params, &["prediction_bands"], &mut spec.prediction_bands);
    apply_bool_opt(params, &["interpolate_unknowns"], &mut spec.interpolate_unknowns);
    apply_bool_opt(params, &["profile_likelihood_ci"], &mut spec.profile_likelihood_ci);
    apply_bool_opt(params, &["normality_of_residuals", "normality_tests"], &mut spec.normality_of_residuals);
    apply_bool_opt(params, &["replicates_lack_of_fit_test"], &mut spec.replicates_lack_of_fit_test);
    apply_bool_opt(params, &["plot_function_only"], &mut spec.plot_function_only);
    apply_bool_opt(params, &["homoscedasticity_test"], &mut spec.homoscedasticity_test);
    apply_bool_opt(params, &["emit_residuals_tab"], &mut spec.emit_residuals_tab);
    apply_bool_opt(params, &["emit_covariance_tab"], &mut spec.emit_covariance_tab);
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

fn patch_analysis_kind_fields(kind: &mut AnalysisKind, patch: &Value) -> Result<(), EngineError> {
    let Value::Object(patch_map) = patch else {
        return Ok(());
    };
    let mut current_json = serde_json::to_value(&*kind)
        .map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed serializing analysis kind: {err}")))?;
    if let Value::Object(ref mut variant_map) = current_json
        && variant_map.len() == 1
        && let Some((_key, inner)) = variant_map.iter_mut().next()
        && let Value::Object(inner_map) = inner
    {
        for (k, v) in patch_map {
            if inner_map.contains_key(k) {
                inner_map.insert(k.clone(), v.clone());
            }
        }
    }
    if let Ok(updated) = serde_json::from_value::<AnalysisKind>(current_json) {
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

fn default_multiple_imputation() -> MultivariateSpec {
    MultivariateSpec::MultipleImputation { imputations_m: 5, max_iters: 10, seed: 42, response_column: None, predictor_columns: Vec::new() }
}

fn default_cox_survival(params: &Value) -> Result<SurvivalAnalysisSpec, EngineError> {
    let time_column = usize_opt(params, "time_column").unwrap_or(0);
    let status_column = usize_opt(params, "status_column").unwrap_or(1);
    let predictor_columns =
        parse_usize_list(params.get("predictor_columns").or_else(|| params.get("cox_covariate_columns")))?.unwrap_or_else(|| vec![2]);
    Ok(SurvivalAnalysisSpec::CoxProportionalHazards {
        time_column,
        status_column,
        predictor_columns,
        reference_levels: Vec::new(),
        tie_method: CoxTieMethod::Efron,
    })
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

fn parse_normalize_anchor(raw: &str) -> NormalizeAnchor {
    match raw.trim().to_ascii_lowercase().as_str() {
        "first" | "first_row" => NormalizeAnchor::FirstRow,
        "last" | "last_row" => NormalizeAnchor::LastRow,
        "sum" | "column_sum" => NormalizeAnchor::ColumnSum,
        "largest" | "max" | "column_max" => NormalizeAnchor::ColumnMax,
        _ => NormalizeAnchor::ColumnMin,
    }
}

fn parse_baseline_op(raw: &str) -> BaselineOperation {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ratio" | "divide" => BaselineOperation::Ratio,
        "percent" | "percent_change" => BaselineOperation::PercentChange,
        "fraction" | "fractional_change" => BaselineOperation::FractionalChange,
        _ => BaselineOperation::Difference,
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

fn opt_str<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| params.get(*k).and_then(Value::as_str))
}

fn usize_opt(params: &Value, key: &str) -> Option<usize> {
    params.get(key).and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok())
}

fn parse_named_params_vec(val: Option<&Value>) -> Result<Vec<(String, f64)>, EngineError> {
    let mut vec = Vec::new();
    let Some(obj) = val.and_then(Value::as_object) else {
        return Ok(vec);
    };
    for (k, v) in obj {
        let num = v
            .as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("parameter `{k}` must be numeric")))?;
        vec.push((k.clone(), num));
    }
    Ok(vec)
}

fn apply_bool_opt(params: &Value, keys: &[&str], target: &mut bool) {
    for key in keys {
        if let Some(b) = params.get(*key).and_then(Value::as_bool) {
            *target = b;
            return;
        }
    }
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
    if val.is_object() {
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
    if let Ok(parsed) = serde_json::from_value::<ModelComparisonSpec>(val.clone()) {
        return Ok(Some(parsed));
    }
    let (method_raw, alt_id) = if let Some(s) = val.as_str() {
        let alt = opt_str(params, &["alternative_equation_id", "alternative_model"])
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`comparison` shorthand string requires `alternative_equation_id`"))?;
        (s, alt.to_owned())
    } else if val.is_object() {
        let m =
            opt_str(val, &["method", "kind"]).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`comparison` object missing `method`"))?;
        let alt = opt_str(val, &["alternative_equation_id", "alternative_model", "equation_id"])
            .or_else(|| opt_str(params, &["alternative_equation_id"]))
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

fn parse_parameter_rule(item: &Value) -> Result<ParameterRule, EngineError> {
    if let Ok(parsed) = serde_json::from_value::<ParameterRule>(item.clone()) {
        return Ok(parsed);
    }
    let name = opt_str(item, &["name", "parameter"])
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
            let hook: HookSource = serde_json::from_value(hook_val.clone())
                .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `hook` source: {err}")))?;
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
    use lumencraft_doc::{Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, ProjectDocument, TableArchetype};
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

        let out_dir = std::env::temp_dir().join(format!("lc_ana_test_{}", std::process::id()));
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
}
