//! Golden ("recorded outcomes") regression corpus pinning how engine command handlers treat
//! alternative parameter spellings: which spellings are accepted, which spelling wins when several
//! are present (including an explicit `null`), and whether wrongly typed values are ignored or
//! rejected.
//!
//! Every case runs on a freshly built fixture session. A successful command records
//! `ok:<FNV-1a 64 hash of the serialized ProjectDocument>`; a failed command records
//! `err:<ErrorCode Debug>:<EngineError Display>`. Each case runs twice to flag nondeterminism.
//!
//! Regenerate `param_spellings/golden.rs` after an intentional behaviour change with
//! `CURVEBOOK_UPDATE_GOLDEN=1 cargo test -p curvebook-engine --test param_spellings -- --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "param_spellings/golden.rs"]
mod golden;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use curvebook_doc::SheetId;
use curvebook_engine::{Caller, GuiOnlyPolicy, Session};
use serde_json::{Map, Value, json};

const UPDATE_ENV: &str = "CURVEBOOK_UPDATE_GOLDEN";

/// Spelling groups in precedence order (first present spelling wins).
const GROUPS: &[(&str, &[&str])] = &[
    // crates/engine/src/cmd/analysis.rs
    ("TRANSFORM_Y", &["y_transform", "preset", "operation"]),
    ("TRANSFORM_ERROR_PROPAGATION", &["error_propagation", "propagate_error"]),
    ("TRANSFORM_ZERO_ANCHOR", &["zero_anchor", "zero"]),
    ("TRANSFORM_HUNDRED_ANCHOR", &["hundred_anchor", "hundred"]),
    ("TRANSFORM_SAMPLE_SIZE", &["sample_size", "n_samples"]),
    ("TRANSFORM_EVERY_K_ROWS", &["every_k_rows", "step"]),
    ("TRANSFORM_AVERAGE_WINDOW", &["average_window", "average_dropped"]),
    ("TRANSFORM_ROW_COUNT", &["row_count", "n_points"]),
    ("TRANSFORM_DERIVED_COLUMNS", &["derived_columns", "derived", "column_formulas", "formulas"]),
    ("DERIVED_COLUMN_TITLE", &["title", "name"]),
    ("DERIVED_COLUMN_FORMULA", &["formula", "expr"]),
    ("CONTINGENCY_METHOD", &["method", "contingency_method"]),
    ("CONTINGENCY_YATES", &["yates_continuity", "yates"]),
    ("MULTIVARIATE_K_CLUSTERS", &["k_clusters", "k_means_k"]),
    ("PVALUE_STACK_METHOD", &["method", "pvalue_stack_method", "multiplicity_method", "multiplicity"]),
    ("PVALUE_STACK_THRESHOLD", &["threshold", "pvalue_stack_threshold", "alpha", "q", "fdr_q"]),
    ("PVALUE_STACK_COLUMN", &["column", "pvalue_stack_column"]),
    ("MULTIPLICITY", &["multiplicity", "post_hoc"]),
    ("MULTIPLICITY_ALPHA", &["alpha", "threshold"]),
    ("COX_PREDICTOR_COLUMNS", &["predictor_columns", "cox_covariate_columns"]),
    ("SURVIVAL_LOG_RANK", &["log_rank_mantel_cox", "log_rank"]),
    ("SURVIVAL_GEHAN", &["gehan_breslow_wilcoxon", "gehan"]),
    ("MV_TIME_COLUMN", &["mv_time_column", "time_column"]),
    ("MV_STATUS_COLUMN", &["mv_status_column", "status_column"]),
    ("MV_GROUP_COLUMN", &["mv_group_column", "group_column"]),
    ("MV_RESPONSE_COLUMN", &["mv_response_column", "response_column"]),
    ("GLM_FAMILY", &["family", "glm_family"]),
    ("REFERENCE_LEVEL_COLUMN", &["column", "col", "column_index"]),
    ("REFERENCE_LEVEL_LEVEL", &["level", "reference_level"]),
    ("MULTIVARIATE_PREDICTOR_COLUMNS", &["predictor_columns", "predictors"]),
    ("MULTIVARIATE_SPEARMAN", &["spearman", "spearman_rank"]),
    ("MULTIVARIATE_IMPUTATIONS", &["imputations_m", "m", "num_imputations"]),
    ("MULTIVARIATE_MAX_ITERS", &["max_iters", "iterations"]),
    ("MULTIVARIATE_RESPONSE_COLUMN", &["response_column", "target_column"]),
    ("KAPPA_WEIGHTING", &["weighting", "kappa_weighting"]),
    ("CLUSTER_DISTANCE", &["distance", "distance_metric"]),
    ("CLUSTER_LINKAGE", &["linkage", "linkage_method"]),
    ("ONE_SAMPLE_HYPOTHETICAL", &["hypothetical", "hypothetical_mean"]),
    ("LINEAR_DEMING", &["deming", "deming_model_ii"]),
    ("OUTLIER_Q_PERCENT", &["q_percent", "fdr_q_percent"]),
    ("POWER_SAMPLE_SIZE", &["sample_size", "n"]),
    ("POWER_SAMPLE_SIZE_GROUP2", &["sample_size_group2", "n2"]),
    ("POWER_CHI_SQUARE_DF", &["chi_square_df", "df"]),
    ("POWER_EFFECT_SIZE", &["effect_size", "standardized_effect"]),
    ("POWER_NUM_GROUPS", &["num_groups", "anova_groups"]),
    ("NONLINEAR_COMPARISON", &["comparison", "model_comparison"]),
    ("NONLINEAR_FIT_METHOD", &["method", "fit_method"]),
    ("NONLINEAR_NORMALITY_TESTS", &["normality_of_residuals", "normality_tests"]),
    ("MODEL_COMPARISON_ALTERNATIVE", &["alternative_equation_id", "alternative_model"]),
    ("MODEL_COMPARISON_METHOD", &["method", "kind"]),
    ("MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION", &["alternative_equation_id", "alternative_model", "equation_id"]),
    ("PARAMETER_RULE_NAME", &["name", "parameter"]),
    ("PARAMETER_SCOPE_VALUES", &["values", "constants", "per_dataset_constants"]),
    ("PARAMETER_SCOPE_HOOK", &["hook", "source"]),
    // crates/engine/src/cmd/graph.rs
    ("POINT_SUBCOLUMN", &["subcol", "subcolumn", "replicate"]),
    ("SERIES_GRAPHICAL_OBJECT", &["gdo", "graphical_object"]),
    ("SERIES_COLOR", &["color", "fill_color"]),
    ("SERIES_MARKER", &["marker", "markers"]),
    ("SERIES_CONNECTING_LINE", &["connecting_line", "line"]),
    ("SERIES_Y_ERROR_BARS", &["y_error_bars", "error_bars"]),
    ("LINE_DASH_STYLE", &["dash_style", "dash"]),
    ("LINE_STROKE_WIDTH", &["stroke_width_pt", "width_pt"]),
    ("MARKER_FILL_COLOR", &["fill_color", "color"]),
    ("ANNOTATION_BORDERED", &["bordered", "boxed"]),
    ("ANNOTATION_DASH", &["dash", "dash_style"]),
    ("ANNOTATION_BOUNDS", &["bounds_pt", "rect_pt"]),
    ("ANNOTATION_STROKE_COLOR", &["stroke_color", "color"]),
    // crates/engine/src/cmd/table.rs
    ("SIMULATED_COLUMN_NAME", &["name", "title"]),
    ("SIMULATED_COLUMN_KIND", &["kind", "type", "distribution"]),
    // crates/engine/src/lib.rs
    ("CONVERT_PREVIEW", &["preview_only", "previewOnly", "preview", "dry_run", "dryRun"]),
];

// ---------------------------------------------------------------------------
// Fixtures (built only through public engine commands)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Fx {
    Empty,
    Xy,
    XyMath,
    XyNorm,
    XyPrune,
    XySample,
    XyGraph,
    XyGraphFit,
    Col,
    ColStrict,
    Pv,
    Ct,
    CtRes,
    Mv,
    MvMissing,
    MvCox,
    MvCluster,
}

const ALL_FIXTURES: &[Fx] = &[
    Fx::Empty,
    Fx::Xy,
    Fx::XyMath,
    Fx::XyNorm,
    Fx::XyPrune,
    Fx::XySample,
    Fx::XyGraph,
    Fx::XyGraphFit,
    Fx::Col,
    Fx::ColStrict,
    Fx::Pv,
    Fx::Ct,
    Fx::CtRes,
    Fx::Mv,
    Fx::MvMissing,
    Fx::MvCox,
    Fx::MvCluster,
];

fn setup(session: &mut Session, command_id: &str, params: Value) {
    if let Err(err) = session.execute(command_id, params.clone()) {
        panic!("fixture command `{command_id}` failed: {err} (params: {params})");
    }
}

fn hill(log_x: f64) -> f64 {
    5.0 + 95.0 / (1.0 + 10_f64.powf((-6.0 - log_x) * 1.15))
}

fn xy_session() -> Session {
    let mut s = Session::new();
    setup(
        &mut s,
        "sheet.createData",
        json!({ "sheet_id": "xy", "title": "Dose", "archetype": "xy", "rows": 8, "cols": 1, "replicates": 2, "auto_info": true, "auto_graph": false }),
    );
    let log_doses = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mut x_cells = Vec::new();
    let mut y_cells = Vec::new();
    for (row, &lx) in log_doses.iter().enumerate() {
        x_cells.push(json!({ "row": row, "value": lx }));
        let ideal = hill(lx);
        y_cells.push(json!({ "row": row, "col": 0, "subcol": 0, "value": ideal - 0.8 }));
        y_cells.push(json!({ "row": row, "col": 0, "subcol": 1, "value": ideal + 0.8 }));
    }
    setup(&mut s, "table.setXCells", json!({ "sheet_id": "xy", "cells": x_cells }));
    setup(&mut s, "table.setCells", json!({ "sheet_id": "xy", "cells": y_cells }));
    setup(&mut s, "info.setConstant", json!({ "sheet_id": "xy", "name": "Floor", "value": 0.0 }));
    s
}

fn xy_with_transform(extra: Value) -> Session {
    let mut s = xy_session();
    let mut params = json!({ "source_sheet": "xy", "sheet_id": "res" });
    merge(&mut params, &extra);
    setup(&mut s, "transform.run", params);
    s
}

fn xy_graph_session() -> Session {
    let mut s = xy_session();
    setup(&mut s, "graph.create", json!({ "sheet_id": "g", "source_sheet_id": "xy", "kind": "xy", "title": "Dose graph" }));
    s
}

fn column_session() -> Session {
    let mut s = Session::new();
    setup(
        &mut s,
        "sheet.createData",
        json!({ "sheet_id": "col", "title": "Cohorts", "archetype": "column", "rows": 6, "cols": 3, "auto_graph": false }),
    );
    setup(&mut s, "table.setGroupTitles", json!({ "sheet_id": "col", "titles": ["Vehicle", "Low", "High"] }));
    setup(
        &mut s,
        "table.paste",
        json!({ "sheet_id": "col", "text": "10.2\t18.1\t32.4\n11.4\t19.0\t34.1\n9.8\t17.4\t30.9\n10.6\t18.8\t33.5\n11.0\t18.5\t31.8\n12.9\t17.9\t29.7" }),
    );
    s
}

fn pvalue_session() -> Session {
    let mut s = Session::new();
    setup(
        &mut s,
        "sheet.createData",
        json!({ "sheet_id": "pv", "title": "P values", "archetype": "column", "rows": 6, "cols": 2, "auto_graph": false }),
    );
    setup(
        &mut s,
        "table.paste",
        json!({ "sheet_id": "pv", "text": "0.001\t0.20\n0.012\t0.03\n0.030\t0.004\n0.045\t0.50\n0.080\t0.011\n0.400\t0.026" }),
    );
    s
}

fn contingency_session() -> Session {
    let mut s = Session::new();
    setup(
        &mut s,
        "sheet.createData",
        json!({ "sheet_id": "ct", "title": "Response", "archetype": "contingency", "rows": 2, "cols": 2, "auto_graph": false }),
    );
    setup(&mut s, "table.paste", json!({ "sheet_id": "ct", "text": "9\t2\n3\t10" }));
    s
}

const MV_ROWS: &[(f64, f64, f64, &str, f64)] = &[
    (12.0, 5.0, 1.0, "Ctrl", 35.0),
    (14.5, 8.0, 0.0, "Ctrl", 42.0),
    (13.2, 3.0, 1.0, "Ctrl", 38.0),
    (15.1, 9.0, 1.0, "Ctrl", 45.0),
    (11.8, 4.0, 1.0, "Ctrl", 33.0),
    (16.0, 7.0, 0.0, "Ctrl", 47.0),
    (22.4, 12.0, 1.0, "Treat", 52.0),
    (25.0, 15.0, 0.0, "Treat", 58.0),
    (24.1, 11.0, 1.0, "Treat", 55.0),
    (26.8, 18.0, 1.0, "Treat", 61.0),
    (23.5, 14.0, 0.0, "Treat", 50.0),
    (21.9, 10.0, 1.0, "Treat", 49.0),
];

fn mv_session(with_missing: bool) -> Session {
    let mut s = Session::new();
    setup(
        &mut s,
        "sheet.createData",
        json!({ "sheet_id": "mv", "title": "Clinical", "archetype": "multiple_variables", "rows": MV_ROWS.len(), "cols": 5, "auto_graph": false }),
    );
    setup(&mut s, "table.setGroupTitles", json!({ "sheet_id": "mv", "titles": ["Response", "Time", "Status", "Cohort", "Age"] }));
    let mut text = String::new();
    for (row, &(resp, time, status, cohort, age)) in MV_ROWS.iter().enumerate() {
        let resp_txt = if with_missing && row == 2 { String::new() } else { resp.to_string() };
        let age_txt = if with_missing && row == 8 { String::new() } else { age.to_string() };
        writeln!(text, "{resp_txt}\t{time}\t{status}\t{cohort}\t{age_txt}").unwrap();
    }
    setup(&mut s, "table.paste", json!({ "sheet_id": "mv", "text": text.trim_end() }));
    s
}

impl Fx {
    fn build(self) -> Session {
        match self {
            Self::Empty => Session::new(),
            Self::Xy => xy_session(),
            Self::XyMath => xy_with_transform(json!({ "kind": "math", "y_transform": "log10" })),
            Self::XyNorm => xy_with_transform(json!({ "kind": "normalize" })),
            Self::XyPrune => xy_with_transform(json!({ "kind": "prune_rows" })),
            Self::XySample => xy_with_transform(json!({ "kind": "random_sample", "sample_size": 6 })),
            Self::XyGraph => xy_graph_session(),
            Self::XyGraphFit => {
                let mut s = xy_graph_session();
                setup(&mut s, "fit.run", json!({ "source_sheet": "xy", "sheet_id": "fit", "kind": "four_parameter_logistic" }));
                s
            }
            Self::Col => column_session(),
            Self::ColStrict => {
                let mut s = column_session();
                s.set_gui_only(GuiOnlyPolicy::Strict);
                s
            }
            Self::Pv => pvalue_session(),
            Self::Ct => contingency_session(),
            Self::CtRes => {
                let mut s = contingency_session();
                setup(&mut s, "stats.run", json!({ "source_sheet": "ct", "sheet_id": "res", "kind": "contingency" }));
                s
            }
            Self::Mv => mv_session(false),
            Self::MvMissing => mv_session(true),
            Self::MvCox => {
                let mut s = mv_session(false);
                setup(
                    &mut s,
                    "stats.run",
                    json!({ "source_sheet": "mv", "sheet_id": "res", "kind": "cox_ph", "time_column": 1, "status_column": 2, "predictor_columns": [0] }),
                );
                s
            }
            Self::MvCluster => {
                let mut s = mv_session(false);
                setup(&mut s, "stats.run", json!({ "source_sheet": "mv", "sheet_id": "res", "kind": "clustering" }));
                s
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Corpus model
// ---------------------------------------------------------------------------

struct Case {
    group: &'static str,
    label: String,
    fixture: Fx,
    command: &'static str,
    caller: Caller,
    params: Value,
}

impl Case {
    fn key(&self) -> String {
        format!("{}/{}", self.group, self.label)
    }
}

type Build = Box<dyn Fn(Map<String, Value>) -> Value>;

/// One spelling group exercised at one reader site (`variant`).
struct GroupSpec {
    group: &'static str,
    variant: &'static str,
    fixture: Fx,
    command: &'static str,
    caller: Caller,
    build: Build,
    value: Value,
    alts: Vec<(&'static str, Value)>,
    wrong: Option<Value>,
}

impl GroupSpec {
    fn alt(mut self, label: &'static str, val: Value) -> Self {
        self.alts.push((label, val));
        self
    }

    fn wrong(mut self, val: Value) -> Self {
        self.wrong = Some(val);
        self
    }

    fn caller(mut self, caller: Caller) -> Self {
        self.caller = caller;
        self
    }
}

struct SiteSpec {
    group: &'static str,
    variant: &'static str,
    fixture: Fx,
    command: &'static str,
}

/// Group spec with a `null` precedence alternative by default.
fn spec(site: SiteSpec, build: Build, value: Value) -> GroupSpec {
    GroupSpec {
        group: site.group,
        variant: site.variant,
        fixture: site.fixture,
        command: site.command,
        caller: Caller::Ui,
        build,
        value,
        alts: vec![("null", Value::Null)],
        wrong: None,
    }
}

fn site(group: &'static str, variant: &'static str, fixture: Fx, command: &'static str) -> SiteSpec {
    SiteSpec { group, variant, fixture, command }
}

fn merge(base: &mut Value, overlay: &Value) {
    if let (Some(b), Some(o)) = (base.as_object_mut(), overlay.as_object()) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
}

/// Spelling keys live at the top level of the command parameters.
fn top(base: Value) -> Build {
    Box::new(move |fields| {
        let mut out = base.clone();
        merge(&mut out, &Value::Object(fields));
        out
    })
}

/// Spelling keys live inside a nested object (`inner` holds the other nested fields), which `wrap` embeds.
fn nested(inner: Value, wrap: impl Fn(Value) -> Value + 'static) -> Build {
    Box::new(move |fields| {
        let mut obj = inner.clone();
        merge(&mut obj, &Value::Object(fields));
        wrap(obj)
    })
}

fn with_params(base: Value, key: &'static str, make: impl Fn(Value) -> Value + 'static) -> impl Fn(Value) -> Value + 'static {
    move |obj| {
        let mut out = base.clone();
        if let Some(map) = out.as_object_mut() {
            map.insert(String::from(key), make(obj));
        }
        out
    }
}

fn spellings_of(group: &str) -> &'static [&'static str] {
    GROUPS.iter().find(|(name, _)| *name == group).map_or_else(|| panic!("unknown group `{group}`"), |(_, s)| *s)
}

fn fields(pairs: &[(&str, &Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).clone())).collect()
}

fn expand(spec: &GroupSpec, out: &mut Vec<Case>) {
    let spellings = spellings_of(spec.group);
    let push = |out: &mut Vec<Case>, label: String, map: Map<String, Value>| {
        out.push(Case {
            group: spec.group,
            label: format!("{}{label}", spec.variant),
            fixture: spec.fixture,
            command: spec.command,
            caller: spec.caller,
            params: (spec.build)(map),
        });
    };
    push(out, String::from("baseline"), Map::new());
    for s in spellings {
        push(out, format!("spelling:{s}"), fields(&[(s, &spec.value)]));
    }
    let first = spellings[0];
    let last = spellings[spellings.len() - 1];
    for (alt_label, alt) in &spec.alts {
        push(out, format!("precedence:{first}={alt_label},{last}=value"), fields(&[(first, alt), (last, &spec.value)]));
        if spellings.len() >= 3 {
            let mid = spellings[1];
            push(out, format!("precedence:{mid}={alt_label},{last}=value"), fields(&[(mid, alt), (last, &spec.value)]));
        }
    }
    if let Some(wrong) = &spec.wrong {
        for s in spellings {
            push(out, format!("wrong_type:{s}"), fields(&[(s, wrong)]));
        }
        push(out, format!("wrong_type:{first}=wrong,{last}=value"), fields(&[(first, wrong), (last, &spec.value)]));
    }
}

fn custom(out: &mut Vec<Case>, group: &'static str, label: &str, fixture: Fx, command: &'static str, params: Value) {
    out.push(Case { group, label: label.to_owned(), fixture, command, caller: Caller::Ui, params });
}

// ---------------------------------------------------------------------------
// Corpus: analysis.rs
// ---------------------------------------------------------------------------

fn transform_run(kind: &str) -> Value {
    json!({ "source_sheet": "xy", "sheet_id": "res", "kind": kind })
}

fn update_res() -> Value {
    json!({ "sheet_id": "res" })
}

fn transform_specs() -> Vec<GroupSpec> {
    let run = |group, kind: &str, value: Value| spec(site(group, "run/", Fx::Xy, "transform.run"), top(transform_run(kind)), value);
    let upd = |group, fixture, value: Value| spec(site(group, "update/", fixture, "analysis.updateParams"), top(update_res()), value);
    vec![
        run("TRANSFORM_Y", "math", json!("ln")).alt("sqrt", json!("sqrt")).wrong(json!(5)),
        upd("TRANSFORM_Y", Fx::XyMath, json!("ln")).alt("sqrt", json!("sqrt")).wrong(json!(5)),
        run("TRANSFORM_ERROR_PROPAGATION", "math", json!("delta")).alt("asymmetric", json!("asymmetric")).wrong(json!(5)),
        upd("TRANSFORM_ERROR_PROPAGATION", Fx::XyMath, json!("delta")).alt("asymmetric", json!("asymmetric")).wrong(json!(5)),
        run("TRANSFORM_ZERO_ANCHOR", "normalize", json!("first")).alt("last", json!("last")).wrong(json!(5)),
        upd("TRANSFORM_ZERO_ANCHOR", Fx::XyNorm, json!("first")).alt("last", json!("last")).wrong(json!(5)),
        run("TRANSFORM_HUNDRED_ANCHOR", "normalize", json!("sum")).alt("last", json!("last")).wrong(json!(5)),
        upd("TRANSFORM_HUNDRED_ANCHOR", Fx::XyNorm, json!("sum")).alt("last", json!("last")).wrong(json!(5)),
        run("TRANSFORM_SAMPLE_SIZE", "random_sample", json!(3)).alt("5", json!(5)).wrong(json!("3")),
        upd("TRANSFORM_SAMPLE_SIZE", Fx::XySample, json!(3)).alt("5", json!(5)).wrong(json!("3")),
        run("TRANSFORM_EVERY_K_ROWS", "prune_rows", json!(3)).alt("4", json!(4)).wrong(json!("3")),
        upd("TRANSFORM_EVERY_K_ROWS", Fx::XyPrune, json!(3)).alt("4", json!(4)).wrong(json!("3")),
        run("TRANSFORM_AVERAGE_WINDOW", "prune_rows", json!(true)).alt("false", json!(false)).wrong(json!("yes")),
        upd("TRANSFORM_AVERAGE_WINDOW", Fx::XyPrune, json!(true)).alt("false", json!(false)).wrong(json!("yes")),
        // The simulate shorthand's default equation id is unknown to the simulator, so pin a built-in one.
        spec(
            site("TRANSFORM_ROW_COUNT", "run/", Fx::Xy, "transform.run"),
            top(json!({ "source_sheet": "xy", "sheet_id": "res", "kind": "simulate", "equation_id": "hill_4pl" })),
            json!(5),
        )
        .alt("7", json!(7))
        .wrong(json!("5")),
    ]
}

fn select_base() -> Value {
    json!({ "source_sheet": "mv", "sheet_id": "res", "kind": "select_and_transform", "target_archetype": "MultipleVariables" })
}

fn derived_specs() -> Vec<GroupSpec> {
    let wrap_derived = || with_params(select_base(), "derived_columns", |obj| Value::Array(vec![obj]));
    vec![
        spec(site("TRANSFORM_DERIVED_COLUMNS", "", Fx::Mv, "transform.run"), top(select_base()), json!(["Double = Response * 2"]))
            .alt("other", json!(["Half = Response / 2"]))
            .wrong(json!(5)),
        spec(
            site("DERIVED_COLUMN_TITLE", "", Fx::Mv, "transform.run"),
            nested(json!({ "formula": "Response * 2" }), wrap_derived()),
            json!("Double"),
        )
        .alt("other", json!("Other"))
        .wrong(json!(5)),
        spec(
            site("DERIVED_COLUMN_FORMULA", "", Fx::Mv, "transform.run"),
            nested(json!({ "title": "Double" }), wrap_derived()),
            json!("Response * 2"),
        )
        .alt("other", json!("Response / 2"))
        .wrong(json!(5)),
    ]
}

fn stats_on(sheet: &str, kind: &str, extra: Value) -> Value {
    let mut base = json!({ "source_sheet": sheet, "sheet_id": "res", "kind": kind });
    merge(&mut base, &extra);
    base
}

fn contingency_and_cluster_specs() -> Vec<GroupSpec> {
    let ct = |group, kind: &str, value: Value| spec(site(group, "run/", Fx::Ct, "stats.run"), top(stats_on("ct", kind, json!({}))), value);
    let mv = |group, variant, kind: &str, value: Value| spec(site(group, variant, Fx::Mv, "stats.run"), top(stats_on("mv", kind, json!({}))), value);
    let upd = |group, fixture, value: Value| spec(site(group, "update/", fixture, "analysis.updateParams"), top(update_res()), value);
    vec![
        ct("CONTINGENCY_METHOD", "contingency", json!("fisher")).alt("g2", json!("g2")).wrong(json!(5)),
        upd("CONTINGENCY_METHOD", Fx::CtRes, json!("fisher")).alt("g2", json!("g2")).wrong(json!(5)),
        ct("CONTINGENCY_YATES", "contingency", json!(true)).alt("false", json!(false)).wrong(json!("yes")),
        ct("KAPPA_WEIGHTING", "cohen_kappa", json!("linear")).alt("quadratic", json!("quadratic")).wrong(json!(5)),
        mv("MULTIVARIATE_K_CLUSTERS", "run/", "clustering", json!(2)).alt("3", json!(3)).wrong(json!("2")),
        upd("MULTIVARIATE_K_CLUSTERS", Fx::MvCluster, json!(2)).alt("3", json!(3)).wrong(json!("2")),
        mv("CLUSTER_DISTANCE", "", "clustering", json!("manhattan")).alt("cosine", json!("cosine")).wrong(json!(5)),
        mv("CLUSTER_LINKAGE", "", "clustering", json!("complete")).alt("single", json!("single")).wrong(json!(5)),
        mv("MULTIVARIATE_SPEARMAN", "", "correlation", json!(true)).alt("false", json!(false)).wrong(json!("yes")),
    ]
}

fn imputation_specs() -> Vec<GroupSpec> {
    let mi = |group, value: Value| {
        spec(site(group, "", Fx::MvMissing, "stats.run"), top(stats_on("mv", "multiple_imputation", json!({ "seed": 7 }))), value)
    };
    vec![
        mi("MULTIVARIATE_IMPUTATIONS", json!(3)).alt("4", json!(4)).wrong(json!("3")),
        mi("MULTIVARIATE_MAX_ITERS", json!(4)).alt("6", json!(6)).wrong(json!("4")),
        mi("MULTIVARIATE_RESPONSE_COLUMN", json!(0)).alt("1", json!(1)).wrong(json!("0")),
        mi("MULTIVARIATE_PREDICTOR_COLUMNS", json!([1, 4])).alt("other", json!([4])).wrong(json!(5)),
    ]
}

fn column_stats_specs() -> Vec<GroupSpec> {
    let col = |group, kind: &str, value: Value| spec(site(group, "", Fx::Col, "stats.run"), top(stats_on("col", kind, json!({}))), value);
    let pv = |group, value: Value| spec(site(group, "", Fx::Pv, "stats.run"), top(stats_on("pv", "pvalue_stack", json!({}))), value);
    let power = |group, extra: Value, value: Value| spec(site(group, "", Fx::Col, "stats.run"), top(stats_on("col", "power", extra)), value);
    vec![
        pv("PVALUE_STACK_METHOD", json!("bonferroni")).alt("holm", json!("holm")).wrong(json!(5)),
        pv("PVALUE_STACK_THRESHOLD", json!(0.01)).alt("0.2", json!(0.2)).wrong(json!("0.01")),
        pv("PVALUE_STACK_COLUMN", json!(1)).alt("0", json!(0)).wrong(json!("1")),
        col("MULTIPLICITY", "one_way_anova", json!("bonferroni")).alt("sidak", json!("sidak")).wrong(json!(5)),
        col("MULTIPLICITY_ALPHA", "one_way_anova", json!(0.01)).alt("0.1", json!(0.1)).wrong(json!("0.01")),
        col("ONE_SAMPLE_HYPOTHETICAL", "one_sample_t", json!(5.0)).alt("10", json!(10.0)).wrong(json!("5")),
        col("OUTLIER_Q_PERCENT", "outliers", json!(5.0)).alt("10", json!(10.0)).wrong(json!("5")),
        power("POWER_SAMPLE_SIZE", json!({ "solve_target": "Power", "effect_size": 0.5 }), json!(20.0)).alt("30", json!(30.0)).wrong(json!("20")),
        power("POWER_SAMPLE_SIZE_GROUP2", json!({ "solve_target": "Power", "effect_size": 0.5, "sample_size": 20.0 }), json!(25.0))
            .alt("30", json!(30.0))
            .wrong(json!("25")),
        power("POWER_CHI_SQUARE_DF", json!({ "family": "chi_square", "effect_size": 0.3, "target_power": 0.8 }), json!(3.0))
            .alt("4", json!(4.0))
            .wrong(json!("3")),
        power("POWER_EFFECT_SIZE", json!({ "target_power": 0.8 }), json!(0.8)).alt("0.3", json!(0.3)).wrong(json!("0.8")),
        power("POWER_NUM_GROUPS", json!({ "family": "one_way_anova", "effect_size": 0.25, "target_power": 0.8 }), json!(4))
            .alt("5", json!(5))
            .wrong(json!("4")),
    ]
}

fn survival_specs() -> Vec<GroupSpec> {
    let km = |group, extra: Value, value: Value| spec(site(group, "km/", Fx::Mv, "stats.run"), top(stats_on("mv", "kaplan_meier", extra)), value);
    let two =
        |group, extra: Value, value: Value| spec(site(group, "two_group/", Fx::Mv, "stats.run"), top(stats_on("mv", "unpaired_t", extra)), value);
    let km_full = json!({ "mv_time_column": 1, "mv_status_column": 2, "mv_group_column": 3 });
    vec![
        km("MV_TIME_COLUMN", json!({ "mv_status_column": 2, "mv_group_column": 3 }), json!(1)).alt("4", json!(4)).wrong(json!("1")),
        km("MV_STATUS_COLUMN", json!({ "mv_time_column": 1, "mv_group_column": 3 }), json!(2)).alt("0", json!(0)).wrong(json!("2")),
        km("MV_GROUP_COLUMN", json!({ "mv_time_column": 1, "mv_status_column": 2 }), json!(3)).alt("2", json!(2)).wrong(json!("3")),
        two("MV_GROUP_COLUMN", json!({ "mv_response_column": 0 }), json!(3)).alt("2", json!(2)).wrong(json!("3")),
        two("MV_RESPONSE_COLUMN", json!({ "mv_group_column": 3 }), json!(0)).alt("4", json!(4)).wrong(json!("0")),
        km("SURVIVAL_LOG_RANK", km_full.clone(), json!(false)).alt("true", json!(true)).wrong(json!("yes")),
        km("SURVIVAL_GEHAN", km_full, json!(true)).alt("false", json!(false)).wrong(json!("yes")),
        spec(
            site("COX_PREDICTOR_COLUMNS", "run/", Fx::Mv, "stats.run"),
            top(stats_on("mv", "cox_ph", json!({ "time_column": 1, "status_column": 2 }))),
            json!([0, 4]),
        )
        .alt("other", json!([4]))
        .wrong(json!(5)),
        spec(site("COX_PREDICTOR_COLUMNS", "update/", Fx::MvCox, "analysis.updateParams"), top(update_res()), json!([0, 4]))
            .alt("other", json!([4]))
            .wrong(json!(5)),
    ]
}

fn glm_specs() -> Vec<GroupSpec> {
    let glm_base = || stats_on("mv", "glm", json!({ "response_column": 0, "predictor_columns": [3, 4] }));
    let wrap_levels = || with_params(glm_base(), "reference_levels", |obj| Value::Array(vec![obj]));
    vec![
        spec(
            site("GLM_FAMILY", "", Fx::Mv, "stats.run"),
            top(stats_on("mv", "glm", json!({ "response_column": 2, "predictor_columns": [0] }))),
            json!("logistic"),
        )
        .alt("poisson", json!("poisson"))
        .wrong(json!(5)),
        spec(site("REFERENCE_LEVEL_COLUMN", "", Fx::Mv, "stats.run"), nested(json!({ "level": "Treat" }), wrap_levels()), json!(3))
            .alt("0", json!(0))
            .wrong(json!("3")),
        spec(site("REFERENCE_LEVEL_LEVEL", "", Fx::Mv, "stats.run"), nested(json!({ "column": 3 }), wrap_levels()), json!("Treat"))
            .alt("Ctrl", json!("Ctrl"))
            .wrong(json!(5)),
    ]
}

fn fit_base(extra: Value) -> Value {
    let mut base = json!({ "source_sheet": "xy", "sheet_id": "res", "kind": "four_parameter_logistic" });
    merge(&mut base, &extra);
    base
}

const ALT_3PL: &str = "log_agonist_vs_response_3pl";
const ALT_OTHER: &str = "agonist_vs_response_4pl";

fn fit_specs() -> Vec<GroupSpec> {
    let fit = |group, extra: Value, value: Value| spec(site(group, "", Fx::Xy, "fit.run"), top(fit_base(extra)), value);
    let in_comparison = |inner: Value| nested(inner, with_params(fit_base(json!({})), "comparison", |obj| obj));
    let in_rule = |inner: Value| nested(inner, with_params(fit_base(json!({})), "parameter_rules", |obj| Value::Array(vec![obj])));
    vec![
        spec(site("LINEAR_DEMING", "", Fx::Xy, "fit.run"), top(json!({ "source_sheet": "xy", "sheet_id": "res", "kind": "linear" })), json!(true))
            .alt("false", json!(false))
            .wrong(json!("yes")),
        fit("NONLINEAR_FIT_METHOD", json!({}), json!("robust")).alt("poisson", json!("poisson")).wrong(json!(5)),
        fit("NONLINEAR_COMPARISON", json!({ "alternative_equation_id": ALT_3PL }), json!("aicc")).alt("f_test", json!("f_test")).wrong(json!(5)),
        fit("NONLINEAR_NORMALITY_TESTS", json!({}), json!(true)).alt("false", json!(false)).wrong(json!("yes")),
        fit("MODEL_COMPARISON_ALTERNATIVE", json!({ "comparison": "aicc" }), json!(ALT_3PL)).alt("other", json!(ALT_OTHER)).wrong(json!(5)),
        spec(site("MODEL_COMPARISON_METHOD", "", Fx::Xy, "fit.run"), in_comparison(json!({ "alternative_equation_id": ALT_3PL })), json!("aicc"))
            .alt("f_test", json!("f_test"))
            .wrong(json!(5)),
        spec(site("MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION", "", Fx::Xy, "fit.run"), in_comparison(json!({ "method": "aicc" })), json!(ALT_3PL))
            .alt("other", json!(ALT_OTHER))
            .wrong(json!(5)),
        spec(site("PARAMETER_RULE_NAME", "", Fx::Xy, "fit.run"), in_rule(json!({ "scope": "constant", "value": 0.0 })), json!("Bottom"))
            .alt("Top", json!("Top"))
            .wrong(json!(5)),
        spec(
            site("PARAMETER_SCOPE_VALUES", "", Fx::Xy, "fit.run"),
            in_rule(json!({ "name": "Bottom", "scope": "per_dataset_constant" })),
            json!([0.0]),
        )
        .alt("other", json!([5.0]))
        .wrong(json!("x")),
        spec(
            site("PARAMETER_SCOPE_HOOK", "", Fx::Xy, "fit.run"),
            in_rule(json!({ "name": "Bottom", "scope": "hooked_constant" })),
            json!({ "InfoConstant": { "info_sheet_id": null, "constant_name": "Floor" } }),
        )
        .alt("missing_constant", json!({ "InfoConstant": { "info_sheet_id": null, "constant_name": "Missing" } }))
        .wrong(json!(5)),
    ]
}

fn analysis_custom_cases(out: &mut Vec<Case>) {
    // Object-form `comparison` falls back to the top-level `alternative_equation_id` only.
    custom(
        out,
        "MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION",
        "fallback:top_level_alternative_equation_id",
        Fx::Xy,
        "fit.run",
        fit_base(json!({ "comparison": { "method": "aicc" }, "alternative_equation_id": ALT_3PL })),
    );
    custom(
        out,
        "MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION",
        "fallback:top_level_alternative_model",
        Fx::Xy,
        "fit.run",
        fit_base(json!({ "comparison": { "method": "aicc" }, "alternative_model": ALT_3PL })),
    );
    custom(
        out,
        "MODEL_COMPARISON_ALTERNATIVE_OR_EQUATION",
        "fallback:nested_null_then_top_level",
        Fx::Xy,
        "fit.run",
        fit_base(json!({ "comparison": { "method": "aicc", "alternative_equation_id": null }, "alternative_equation_id": ALT_3PL })),
    );
    // `analysis.updateParams` with an explicit `params` patch object routes through the same readers.
    custom(
        out,
        "TRANSFORM_Y",
        "update/patch_object:preset",
        Fx::XyMath,
        "analysis.updateParams",
        json!({ "sheet_id": "res", "params": { "preset": "ln" } }),
    );
    custom(
        out,
        "CONTINGENCY_METHOD",
        "update/patch_object:contingency_method",
        Fx::CtRes,
        "analysis.updateParams",
        json!({ "sheet_id": "res", "params": { "contingency_method": "fisher" } }),
    );
}

// ---------------------------------------------------------------------------
// Corpus: graph.rs, table.rs, lib.rs
// ---------------------------------------------------------------------------

fn series_base() -> Value {
    json!({ "sheet_id": "g", "series_index": 0 })
}

/// Serialized graphical data object of the fixture's first series with every `size_pt` and
/// `stroke_width_pt` number replaced by `size`.
fn modified_gdo(size: f64) -> Value {
    let session = Fx::XyGraph.build();
    let graph = session.document().graph_sheet(&SheetId::new("g")).expect("fixture graph `g`");
    let series = graph.spec.series.first().expect("fixture graph series");
    let mut gdo = serde_json::to_value(&series.gdo).expect("serialize gdo");
    rewrite_numbers(&mut gdo, size);
    gdo
}

fn rewrite_numbers(val: &mut Value, size: f64) {
    match val {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if (k == "size_pt" || k == "stroke_width_pt") && v.is_number() {
                    *v = json!(size);
                } else {
                    rewrite_numbers(v, size);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| rewrite_numbers(v, size)),
        _ => {}
    }
}

fn graph_specs() -> Vec<GroupSpec> {
    let series = |group, value: Value| spec(site(group, "", Fx::XyGraph, "graph.setSeries"), top(series_base()), value);
    let in_line = |inner: Value| nested(inner, with_params(series_base(), "connecting_line", |obj| obj));
    let in_marker = |inner: Value| nested(inner, with_params(series_base(), "marker", |obj| obj));
    let annotation =
        |group, variant, fixture, base: Value, value: Value| spec(site(group, variant, fixture, "graph.addAnnotation"), top(base), value);
    let points = nested(
        json!({ "series_index": 0, "point_index": 0, "color": "#ff0000" }),
        with_params(json!({ "sheet_id": "g" }), "overrides", |obj| Value::Array(vec![obj])),
    );
    vec![
        spec(site("POINT_SUBCOLUMN", "", Fx::XyGraph, "graph.formatSelectedPoints"), points, json!(1)).alt("0", json!(0)).wrong(json!("1")),
        series("SERIES_GRAPHICAL_OBJECT", modified_gdo(11.0)).alt("other", modified_gdo(2.0)).wrong(json!(5)),
        series("SERIES_COLOR", json!("#ff0000")).alt("green", json!("#00ff00")).wrong(json!(5)),
        series("SERIES_MARKER", json!({ "shape": "square" })).alt("diamond", json!({ "shape": "diamond" })).wrong(json!(5)),
        series("SERIES_CONNECTING_LINE", json!({ "dash_style": "dash" })).alt("dot", json!({ "dash_style": "dot" })).wrong(json!(5)),
        series("SERIES_Y_ERROR_BARS", json!({ "metric": "sem" })).alt("range", json!({ "metric": "range" })).wrong(json!(5)),
        spec(site("LINE_DASH_STYLE", "", Fx::XyGraph, "graph.setSeries"), in_line(json!({})), json!("dash")).alt("dot", json!("dot")).wrong(json!(5)),
        spec(site("LINE_STROKE_WIDTH", "", Fx::XyGraph, "graph.setSeries"), in_line(json!({})), json!(3.0)).alt("0.5", json!(0.5)).wrong(json!("3")),
        spec(site("MARKER_FILL_COLOR", "", Fx::XyGraph, "graph.setSeries"), in_marker(json!({})), json!("#00ff00"))
            .alt("blue", json!("#0000ff"))
            .wrong(json!(5)),
        annotation("ANNOTATION_BORDERED", "text/", Fx::XyGraph, json!({ "sheet_id": "g", "kind": "text", "text": "note" }), json!(true))
            .alt("false", json!(false))
            .wrong(json!("yes")),
        annotation(
            "ANNOTATION_BORDERED",
            "equation/",
            Fx::XyGraphFit,
            json!({ "sheet_id": "g", "kind": "equation", "result_sheet_id": "fit" }),
            json!(false),
        )
        .alt("true", json!(true))
        .wrong(json!("no")),
        annotation("ANNOTATION_DASH", "", Fx::XyGraph, json!({ "sheet_id": "g", "kind": "line" }), json!("dash"))
            .alt("dot", json!("dot"))
            .wrong(json!(5)),
        annotation("ANNOTATION_BOUNDS", "", Fx::XyGraph, json!({ "sheet_id": "g", "kind": "shape" }), json!([10.0, 12.0, 50.0, 20.0]))
            .alt("other", json!([1.0, 2.0, 3.0, 4.0]))
            .wrong(json!("x")),
        annotation("ANNOTATION_STROKE_COLOR", "", Fx::XyGraph, json!({ "sheet_id": "g", "kind": "shape" }), json!("#ff0000"))
            .alt("green", json!("#00ff00"))
            .wrong(json!(5)),
    ]
}

fn simulate_base() -> Value {
    json!({ "n_rows": 5, "seed": 3, "sheet_id": "sim" })
}

fn table_specs() -> Vec<GroupSpec> {
    let wrap_column = || with_params(simulate_base(), "columns", |obj| Value::Array(vec![obj]));
    let wrap_distribution = || {
        with_params(simulate_base(), "columns", |obj| {
            let mut col = json!({ "name": "A" });
            if let Some(map) = col.as_object_mut() {
                map.insert(String::from("distribution"), obj);
            }
            Value::Array(vec![col])
        })
    };
    vec![
        spec(
            site("SIMULATED_COLUMN_NAME", "", Fx::Empty, "table.simulateDistributions"),
            nested(json!({ "distribution": { "kind": "gaussian", "mean": 2.0, "sd": 1.0 } }), wrap_column()),
            json!("A"),
        )
        .alt("B", json!("B"))
        .wrong(json!(5)),
        spec(
            site("SIMULATED_COLUMN_KIND", "", Fx::Empty, "table.simulateDistributions"),
            nested(json!({ "mean": 2.0, "sd": 1.0 }), wrap_distribution()),
            json!("gaussian"),
        )
        .alt("poisson", json!("poisson"))
        .wrong(json!(5)),
    ]
}

fn convert_base() -> Value {
    json!({ "sheet_id": "col", "target_kind": "grouped" })
}

fn convert_specs() -> Vec<GroupSpec> {
    vec![
        spec(site("CONVERT_PREVIEW", "ui/", Fx::Col, "table.convertArchetype"), top(convert_base()), json!(true))
            .alt("false", json!(false))
            .wrong(json!("yes")),
        spec(site("CONVERT_PREVIEW", "strict_mcp/", Fx::ColStrict, "table.convertArchetype"), top(convert_base()), json!(true))
            .alt("false", json!(false))
            .wrong(json!("yes"))
            .caller(Caller::Mcp),
    ]
}

fn convert_custom_cases(out: &mut Vec<Case>) {
    let all = |val: &Value| {
        let mut p = convert_base();
        for s in spellings_of("CONVERT_PREVIEW") {
            merge(&mut p, &json!({ *s: val }));
        }
        p
    };
    let single_false = |s: &str| {
        let mut p = convert_base();
        merge(&mut p, &json!({ s: false }));
        p
    };
    for (variant, fixture, caller) in [("ui/", Fx::Col, Caller::Ui), ("strict_mcp/", Fx::ColStrict, Caller::Mcp)] {
        let mut push = |label: String, params: Value| {
            out.push(Case {
                group: "CONVERT_PREVIEW",
                label: format!("{variant}{label}"),
                fixture,
                command: "table.convertArchetype",
                caller,
                params,
            });
        };
        push(String::from("agree:all_true"), all(&json!(true)));
        push(String::from("agree:all_false"), all(&json!(false)));
        for s in spellings_of("CONVERT_PREVIEW") {
            push(format!("false_only:{s}"), single_false(s));
        }
        let mut conflict_late = convert_base();
        merge(&mut conflict_late, &json!({ "preview_only": true, "previewOnly": true, "dryRun": false }));
        push(String::from("conflict:preview_only=true,previewOnly=true,dryRun=false"), conflict_late);
        let mut null_late = convert_base();
        merge(&mut null_late, &json!({ "preview_only": true, "dryRun": null }));
        push(String::from("conflict:preview_only=true,dryRun=null"), null_late);
    }
}

// ---------------------------------------------------------------------------
// Corpus assembly & execution
// ---------------------------------------------------------------------------

fn all_specs() -> Vec<GroupSpec> {
    let mut specs = Vec::new();
    specs.extend(transform_specs());
    specs.extend(derived_specs());
    specs.extend(contingency_and_cluster_specs());
    specs.extend(imputation_specs());
    specs.extend(column_stats_specs());
    specs.extend(survival_specs());
    specs.extend(glm_specs());
    specs.extend(fit_specs());
    specs.extend(graph_specs());
    specs.extend(table_specs());
    specs.extend(convert_specs());
    specs
}

fn corpus() -> Vec<Case> {
    let mut cases = Vec::new();
    for spec in all_specs() {
        expand(&spec, &mut cases);
    }
    analysis_custom_cases(&mut cases);
    convert_custom_cases(&mut cases);
    let mut seen = BTreeSet::new();
    for case in &cases {
        assert!(seen.insert(case.key()), "duplicate corpus key `{}`", case.key());
    }
    cases
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Result-sheet fields computed by analyses. They are left out of the hash so that platform-level
/// floating-point differences (libm, CPU) in fitted or iterated results cannot fail this test; the
/// analysis spec that produced them is still hashed.
const COMPUTED_RESULT_FIELDS: [&str; 7] =
    ["analyzable_data", "tabs", "scalars", "pairwise_comparisons", "fitted_curves", "eliminated_outliers", "checklist"];

/// Replace every non-integer number with a 12-significant-digit string so last-bit differences in
/// derived values (for example automatic axis ranges) do not change the hash.
fn round_floats(val: &mut Value) {
    match val {
        Value::Number(n) if !(n.is_i64() || n.is_u64()) => {
            if let Some(f) = n.as_f64() {
                *val = Value::String(format!("{f:.11e}"));
            }
        }
        Value::Array(items) => items.iter_mut().for_each(round_floats),
        Value::Object(map) => map.values_mut().for_each(round_floats),
        _ => {}
    }
}

fn document_hash(session: &Session) -> String {
    let mut doc = serde_json::to_value(session.document()).expect("serialize document");
    if let Some(results) = doc.get_mut("result_sheets").and_then(Value::as_array_mut) {
        for sheet in results.iter_mut().filter_map(Value::as_object_mut) {
            for field in COMPUTED_RESULT_FIELDS {
                sheet.remove(field);
            }
        }
    }
    round_floats(&mut doc);
    let text = serde_json::to_string(&doc).expect("serialize normalized document");
    format!("{:016x}", fnv1a64(text.as_bytes()))
}

fn run_case(case: &Case, fixtures: &HashMap<Fx, Session>) -> String {
    let mut session = fixtures.get(&case.fixture).cloned().unwrap_or_else(|| panic!("missing fixture {:?}", case.fixture));
    let cancelled = AtomicBool::new(false);
    match session.execute_with_caller(case.command, case.params.clone(), case.caller, &cancelled) {
        Ok(_) => format!("ok:{}", document_hash(&session)),
        Err(err) => format!("err:{:?}:{err}", err.code),
    }
}

fn build_fixtures() -> HashMap<Fx, Session> {
    ALL_FIXTURES.iter().map(|&fx| (fx, fx.build())).collect()
}

/// Run every case twice (against independently built fixture sets) and return outcomes by key.
fn compute_outcomes(cases: &[Case]) -> BTreeMap<String, String> {
    let first_fixtures = build_fixtures();
    let second_fixtures = build_fixtures();
    for fx in ALL_FIXTURES {
        assert_eq!(document_hash(&first_fixtures[fx]), document_hash(&second_fixtures[fx]), "fixture {fx:?} is not deterministic");
    }
    let mut outcomes = BTreeMap::new();
    for case in cases {
        let first = run_case(case, &first_fixtures);
        let second = run_case(case, &second_fixtures);
        assert_eq!(first, second, "nondeterministic outcome for `{}` (params: {})", case.key(), case.params);
        outcomes.insert(case.key(), first);
    }
    outcomes
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("param_spellings").join("golden.rs")
}

fn render_golden(outcomes: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    out.push_str("//! Recorded outcomes for `param_spellings.rs` (generated; do not edit by hand).\n");
    out.push_str("//!\n");
    out.push_str("//! Regenerate with `CURVEBOOK_UPDATE_GOLDEN=1 cargo test -p curvebook-engine --test param_spellings`.\n\n");
    out.push_str("pub const GOLDEN: &[(&str, &str)] = &[\n");
    for (key, outcome) in outcomes {
        let one_line = format!("    ({key:?}, {outcome:?}),");
        if one_line.len() <= 150 {
            out.push_str(&one_line);
            out.push('\n');
        } else {
            writeln!(out, "    (\n        {key:?},\n        {outcome:?},\n    ),").unwrap();
        }
    }
    out.push_str("];\n");
    out
}

/// Generation-time report: vacuous groups (first spelling == baseline) and spellings that disagree.
fn report(cases: &[Case], outcomes: &BTreeMap<String, String>) {
    let mut sites: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
    for case in cases {
        let (variant, rest) = case.label.rsplit_once('/').map_or((String::new(), case.label.clone()), |(v, r)| (format!("{v}/"), r.to_owned()));
        sites.entry((case.group.to_owned(), variant)).or_default().push((rest, outcomes[&case.key()].clone()));
    }
    eprintln!("==== param_spellings generation report ({} cases) ====", cases.len());
    for ((group, variant), entries) in &sites {
        let get = |label: &str| entries.iter().find(|(l, _)| l == label).map(|(_, o)| o.as_str());
        let Some(baseline) = get("baseline") else {
            continue;
        };
        let spellings = spellings_of(group);
        let first = get(&format!("spelling:{}", spellings[0]));
        if first == Some(baseline) {
            eprintln!("VACUOUS {group}/{variant}: first spelling outcome equals baseline ({baseline})");
        }
        let distinct: BTreeSet<&str> = spellings.iter().filter_map(|s| get(&format!("spelling:{s}"))).collect();
        if distinct.len() > 1 {
            eprintln!("DIVERGENT {group}/{variant}: spellings disagree for the same value:");
            for s in spellings {
                eprintln!("    {s}: {}", get(&format!("spelling:{s}")).unwrap_or("-"));
            }
        }
        for (label, outcome) in entries {
            eprintln!("    {group}/{variant}{label} => {outcome}");
        }
    }
}

#[test]
fn param_spellings_match_golden() {
    let cases = corpus();
    let outcomes = compute_outcomes(&cases);

    if std::env::var(UPDATE_ENV).is_ok_and(|v| v == "1") {
        report(&cases, &outcomes);
        let text = render_golden(&outcomes);
        curvebook_platform::fs::write_atomic(&golden_path(), text.as_bytes()).expect("write golden.rs");
        eprintln!("regenerated {} with {} entries", golden_path().display(), outcomes.len());
        return;
    }

    let golden: BTreeMap<&str, &str> = golden::GOLDEN.iter().copied().collect();
    assert_eq!(golden.len(), golden::GOLDEN.len(), "golden.rs contains duplicate keys");
    let mut problems = Vec::new();
    for (key, outcome) in &outcomes {
        match golden.get(key.as_str()) {
            None => problems.push(format!("missing golden entry: {key} => {outcome}")),
            Some(expected) if *expected != outcome => problems.push(format!("changed: {key}\n    golden: {expected}\n    actual: {outcome}")),
            Some(_) => {}
        }
    }
    for key in golden.keys() {
        if !outcomes.contains_key(*key) {
            problems.push(format!("extra golden entry with no corpus case: {key}"));
        }
    }
    assert!(problems.is_empty(), "{} golden mismatch(es) (set {UPDATE_ENV}=1 to regenerate):\n{}", problems.len(), problems.join("\n"));
}

fn collect_keys(val: &Value, out: &mut BTreeSet<String>) {
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                out.insert(k.clone());
                collect_keys(v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_keys(v, out)),
        _ => {}
    }
}

#[test]
fn corpus_covers_every_spelling() {
    let cases = corpus();
    let mut group_names = BTreeSet::new();
    for (group, spellings) in GROUPS {
        assert!(group_names.insert(*group), "duplicate group `{group}` in GROUPS");
        let mut keys = BTreeSet::new();
        for case in cases.iter().filter(|c| c.group == *group && !c.label.ends_with("baseline")) {
            collect_keys(&case.params, &mut keys);
        }
        for s in *spellings {
            assert!(keys.contains(*s), "group `{group}` has no non-baseline case using spelling `{s}`");
        }
    }
    for case in &cases {
        assert!(group_names.contains(case.group), "case `{}` uses a group missing from GROUPS", case.key());
    }
}

#[test]
fn groups_match_engine_spelling_tables() {
    let engine: Vec<(&str, Vec<&str>)> = curvebook_engine::cmd::spellings::ALL.iter().map(|(name, s)| (*name, s.to_vec())).collect();
    let corpus: Vec<(&str, Vec<&str>)> = GROUPS.iter().map(|(name, s)| (*name, s.to_vec())).collect();
    assert_eq!(corpus, engine, "GROUPS must list exactly the tables in `curvebook_engine::cmd::spellings::ALL`, in the same order");
}
