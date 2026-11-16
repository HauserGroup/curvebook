//! Integration tests wiring Waves 20-21 (`doc`, `transform`, `stats`, `fit`, `chart`)
//! through `curvebook-format` and `curvebook-engine`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyMethod,
    ContingencySpec, ContingencyTable, DataSheet, EquationDisplayMode, FigureCanvasPreset, GraphAnnotation, GroupedTable, InfoSheet, KappaWeighting,
    MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, PartsOfWholeTable, ProjectDocument, RectPt, ResultStatus,
    ResultTabKind, SheetId, SubcolumnMode, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole, XyTable,
};
use curvebook_engine::dag::{recalculate_project, recalculate_project_with_report};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::{ErrorCode, Session};
use curvebook_format::{FormatError, detect_dependency_cycles};
use serde_json::json;

fn exec_ok(session: &mut Session, id: &str, params: serde_json::Value) -> curvebook_engine::CommandOutcome {
    session.execute(id, params).unwrap_or_else(|e| panic!("command `{id}` failed unexpectedly: {e}"))
}

fn exec_err_code(session: &mut Session, id: &str, params: serde_json::Value) -> ErrorCode {
    session.execute(id, params).expect_err("expected command failure").code
}

fn scene_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn scalar_val(session: &Session, res_id: &str, name: &str) -> f64 {
    session
        .document()
        .result_sheet(&SheetId::new(res_id))
        .expect("result sheet")
        .lookup_scalar(name, None)
        .unwrap_or_else(|| panic!("missing scalar `{name}` on `{res_id}`"))
}

fn assert_no_qol7_remediation_advice(report: &str) {
    let lower = report.to_ascii_lowercase();
    for forbidden in
        ["recommend switching", "switch to", "consider switching", "use welch", "use mann-whitney", "apply log transform", "transform the data"]
    {
        assert!(!lower.contains(forbidden), "QOL-7 violation `{forbidden}` in report:\n{report}");
    }
}

fn assert_reports_contain(doc: &ProjectDocument, checks: &[(&str, &[&str])]) {
    for (res_id, expected_substrs) in checks {
        let md = generate_methods_report(doc, &SheetId::new(*res_id), ReportFormat::Markdown).expect("report");
        for needle in *expected_substrs {
            assert!(md.contains(needle), "expected `{needle}` in `{res_id}` report:\n{md}");
        }
        assert_no_qol7_remediation_advice(&md);
    }
}

#[test]
fn table_randomize_groups_arrives_constants_and_undo_redo() {
    let mut session = Session::new();

    // 1) Complete-balanced with auto-generated InfoSheet and subject_count
    let out1 = exec_ok(
        &mut session,
        "table.randomizeGroups",
        json!({
            "sheet_id": "rand-cb",
            "title": "Preclinical Cohort",
            "groups": [{ "label": "HighDose", "weight": 2 }, { "label": "LowDose", "weight": 1 }, "Vehicle"],
            "subject_count": 12,
            "scheme": "complete_balanced",
            "seed": 4242
        }),
    );
    assert!(out1.mutated);
    let info1_id = SheetId::new(out1.payload["info_sheet_id"].as_str().expect("info_sheet_id"));
    assert_eq!(out1.payload["subject_count"], json!(12));
    assert_eq!(out1.payload["arrive_constants"]["randomization.method"], json!("complete_balanced"));
    assert_eq!(out1.payload["arrive_constants"]["randomization.allocation_ratio"], json!("2:1:1"));
    assert_eq!(out1.payload["arrive_constants"]["randomization.seed"], json!("4242"));

    let ds1 = session.document().data_sheet(&SheetId::new("rand-cb")).expect("rand-cb");
    assert_eq!(ds1.kind(), TableKind::MultipleVariables);
    assert_eq!(ds1.archetype.row_count(), 12);
    assert_eq!(ds1.info_sheet_id.as_ref(), Some(&info1_id));
    let info1 = session.document().info_sheet(&info1_id).expect("info1");
    assert_eq!(info1.get_constant("randomization.allocation_ratio").map(|c| &c.value), Some(&ConstantValue::Text(String::from("2:1:1"))));

    // Determinism check with a second session using the same seed
    let mut session_b = Session::new();
    exec_ok(
        &mut session_b,
        "table.randomizeGroups",
        json!({
            "sheet_id": "rand-cb",
            "title": "Preclinical Cohort",
            "groups": [{ "label": "HighDose", "weight": 2 }, { "label": "LowDose", "weight": 1 }, "Vehicle"],
            "subject_count": 12,
            "scheme": "complete_balanced",
            "seed": 4242
        }),
    );
    assert_eq!(
        session.document().data_sheet(&SheetId::new("rand-cb")).unwrap().archetype,
        session_b.document().data_sheet(&SheetId::new("rand-cb")).unwrap().archetype
    );

    // Undo / Redo removes both randomized DataSheet and newly created InfoSheet
    assert!(session.undo().expect("undo randomizeGroups"));
    assert!(session.document().data_sheet(&SheetId::new("rand-cb")).is_none());
    assert!(session.document().info_sheet(&info1_id).is_none());
    assert!(session.redo().expect("redo randomizeGroups"));
    assert!(session.document().data_sheet(&SheetId::new("rand-cb")).is_some());
    assert_eq!(
        session.document().info_sheet(&info1_id).unwrap().get_constant("randomization.seed").map(|c| &c.value),
        Some(&ConstantValue::Text(String::from("4242")))
    );

    // 2) Permuted-block with explicit subject_ids + targeted existing InfoSheet update
    let out2 = exec_ok(
        &mut session,
        "table.randomizeGroups",
        json!({
            "sheet_id": "rand-pb",
            "groups": ["Vehicle", "Drug"],
            "subject_ids": ["M-01", "M-02", "M-03", "M-04", "M-05", "M-06"],
            "scheme": "permuted_block",
            "block_sizes": [2, 4],
            "seed": 99,
            "info_sheet_id": info1_id.as_str()
        }),
    );
    assert_eq!(out2.payload["arrive_constants"]["randomization.method"], json!("permuted_block"));
    assert_eq!(out2.payload["arrive_constants"]["randomization.block_sizes"], json!("2, 4"));
    assert_eq!(
        session.document().info_sheet(&info1_id).unwrap().get_constant("randomization.total_subjects").map(|c| &c.value),
        Some(&ConstantValue::Text(String::from("6")))
    );
    assert!(session.undo().expect("undo second randomizeGroups"));
    assert_eq!(
        session.document().info_sheet(&info1_id).unwrap().get_constant("randomization.total_subjects").map(|c| &c.value),
        Some(&ConstantValue::Text(String::from("12")))
    );
}

#[test]
fn table_random_sample_rows_stratified_provenance_and_undo_redo() {
    let mut doc = ProjectDocument::new("Sampling").unwrap();
    let info = InfoSheet::new_preseeded("info-main", "Study Info").unwrap();
    doc.add_info_sheet(info).unwrap();

    let mut mv = MultipleVariablesTable::new(2, 12).unwrap();
    mv.variables[0].title = String::from("Stratum");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Response");
    mv.variables[1].lock_role(VariableRole::Continuous);
    for r in 0..12_usize {
        let lbl = if r < 8 { "A" } else { "B" };
        mv.variables[0].cells[r] = Cell::text(lbl);
        mv.variables[1].cells[r] = Cell::number((r + 1) as f64);
    }
    let mut src = DataSheet::new("mv-src", "Source Cohort", TableArchetype::MultipleVariables(mv)).unwrap();
    src.info_sheet_id = Some(SheetId::new("info-main"));
    doc.add_data_sheet(src).unwrap();

    let mut session = Session::from_document(doc);
    let out = exec_ok(
        &mut session,
        "table.randomSampleRows",
        json!({
            "source_sheet_id": "mv-src",
            "sample_size": 6,
            "seed": 777,
            "stratify_column": 0,
            "proportional_allocation": true,
            "sheet_id": "mv-sample"
        }),
    );
    assert_eq!(out.payload["row_count"], json!(6));
    assert_eq!(out.payload["info_sheet_id"], json!("info-main"));

    let sampled = session.document().data_sheet(&SheetId::new("mv-sample")).expect("mv-sample");
    let TableArchetype::MultipleVariables(ref smv) = sampled.archetype else {
        panic!("expected MultipleVariables sample");
    };
    let count_a = smv.variables[0].cells.iter().filter(|c| c.active_text() == Some("A")).count();
    let count_b = smv.variables[0].cells.iter().filter(|c| c.active_text() == Some("B")).count();
    assert_eq!(count_a, 4);
    assert_eq!(count_b, 2);

    let info_after = session.document().info_sheet(&SheetId::new("info-main")).expect("info-main");
    assert_eq!(info_after.get_constant("sample.seed").map(|c| &c.value), Some(&ConstantValue::Text(String::from("777"))));
    assert_eq!(info_after.get_constant("sample.sample_size").map(|c| &c.value), Some(&ConstantValue::Text(String::from("6"))));

    assert!(session.undo().expect("undo randomSampleRows"));
    assert!(session.document().data_sheet(&SheetId::new("mv-sample")).is_none());
    assert!(session.document().info_sheet(&SheetId::new("info-main")).unwrap().get_constant("sample.seed").is_none());
    assert!(session.redo().expect("redo randomSampleRows"));
    assert!(session.document().data_sheet(&SheetId::new("mv-sample")).is_some());
}

#[test]
fn table_simulate_distributions_all_six_and_undo_redo() {
    let mut session = Session::new();

    let out_col = exec_ok(
        &mut session,
        "table.simulateDistributions",
        json!({
            "sheet_id": "sim-col",
            "title": "Six Parametric Variates",
            "n_rows": 25,
            "seed": 123,
            "use_multiple_variables": false,
            "columns": [
                { "name": "Unif", "distribution": "uniform", "min": -2.0, "max": 4.0 },
                { "name": "Gauss", "distribution": "gaussian", "mean": 10.0, "sd": 2.0 },
                { "name": "LogN", "distribution": "log_normal", "mean_log": 0.0, "sd_log": 0.5 },
                { "name": "Binom", "distribution": "binomial", "trials": 10, "p": 0.4 },
                { "name": "Pois", "distribution": "poisson", "lambda": 3.5 },
                { "name": "Exp", "distribution": "exponential", "rate": 0.5 }
            ]
        }),
    );
    assert_eq!(out_col.payload["n_rows"], json!(25));
    assert_eq!(out_col.payload["n_cols"], json!(6));
    let ds = session.document().data_sheet(&SheetId::new("sim-col")).expect("sim-col");
    let TableArchetype::Column(ref col_tbl) = ds.archetype else {
        panic!("expected Column table");
    };
    assert_eq!(col_tbl.data_sets.len(), 6);
    assert!(col_tbl.active_column_values(0).iter().all(|&v| (-2.0..=4.0).contains(&v)));
    assert!(col_tbl.active_column_values(2).iter().all(|&v| v > 0.0));
    assert!(col_tbl.active_column_values(3).iter().all(|&v| (0.0..=10.0).contains(&v) && v.fract() == 0.0));
    assert!(col_tbl.active_column_values(4).iter().all(|&v| v >= 0.0 && v.fract() == 0.0));
    assert!(col_tbl.active_column_values(5).iter().all(|&v| v >= 0.0));

    let out_mv = exec_ok(
        &mut session,
        "table.simulateDistributions",
        json!({
            "sheet_id": "sim-mv",
            "n_rows": 10,
            "seed": 456,
            "use_multiple_variables": true,
            "columns": [
                { "name": "X", "distribution": { "kind": "gaussian", "mean": 0.0, "sd": 1.0 } },
                { "name": "Count", "distribution": { "Poisson": { "lambda": 2.0 } } }
            ]
        }),
    );
    assert_eq!(out_mv.payload["use_multiple_variables"], json!(true));
    assert_eq!(session.document().data_sheet(&SheetId::new("sim-mv")).unwrap().kind(), TableKind::MultipleVariables);

    assert!(session.undo().expect("undo sim-mv"));
    assert!(session.document().data_sheet(&SheetId::new("sim-mv")).is_none());
    assert!(session.redo().expect("redo sim-mv"));
    assert!(session.document().data_sheet(&SheetId::new("sim-mv")).is_some());
}

#[test]
fn graph_add_annotation_model_equation_resize_clone_and_live_recalc() {
    let mut doc = ProjectDocument::new("Model Equation Workflow").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    xy.x_title = String::from("X");
    xy.data_sets[0].title = String::from("Linear Series");
    for (r, x) in [0.0, 1.0, 2.0, 3.0, 4.0].iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(*x);
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * x + 1.0);
    }
    doc.add_data_sheet(DataSheet::new("xy-1", "Calibration", TableArchetype::Xy(xy)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    exec_ok(&mut session, "fit.run", json!({ "sheet_id": "res-lin", "source_sheet_id": "xy-1", "kind": "linear" }));
    exec_ok(&mut session, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy" }));

    assert_eq!(
        exec_err_code(&mut session, "graph.addAnnotation", json!({ "sheet_id": "g-xy", "kind": "equation", "result_sheet_id": "missing-res" })),
        ErrorCode::NotFound
    );

    exec_ok(
        &mut session,
        "graph.addAnnotation",
        json!({
            "sheet_id": "g-xy",
            "kind": "equation",
            "result_sheet_id": "res-lin",
            "data_set_index": 0,
            "mode": "substituted",
            "bounds_pt": [64.0, 42.0, 200.0, 40.0],
            "font_size_pt": 11.0,
            "bordered": true
        }),
    );
    exec_ok(
        &mut session,
        "graph.addAnnotation",
        json!({
            "sheet_id": "g-xy",
            "kind": "model_equation",
            "result_sheet_id": "res-lin",
            "mode": "symbolic",
            "bounds_pt": [64.0, 90.0, 180.0, 32.0]
        }),
    );

    detect_dependency_cycles(session.document()).expect("graph with ModelEquation must be acyclic");
    let g_v1 = session.document().graph_sheet(&SheetId::new("g-xy")).unwrap();
    assert_eq!(g_v1.spec.annotations.len(), 2);
    assert!(matches!(&g_v1.spec.annotations[1], GraphAnnotation::ModelEquation { mode: EquationDisplayMode::Symbolic, .. }));

    let scene_v1 = compile_graph(g_v1, session.document()).expect("compile graph v1");
    let texts_v1 = scene_texts(&scene_v1).join("\n");
    assert!(texts_v1.contains("Y = "), "expected substituted linear equation text, got:\n{texts_v1}");

    // Mutate upstream data from slope=2, intercept=1 to slope=5, intercept=3 -> live DAG recalc updates scene equation text
    exec_ok(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "xy-1",
            "edits": [
                { "row": 0, "col": 0, "value": 3.0 },
                { "row": 1, "col": 0, "value": 8.0 },
                { "row": 2, "col": 0, "value": 13.0 },
                { "row": 3, "col": 0, "value": 18.0 },
                { "row": 4, "col": 0, "value": 23.0 }
            ]
        }),
    );
    let g_v2 = session.document().graph_sheet(&SheetId::new("g-xy")).unwrap();
    let scene_v2 = compile_graph(g_v2, session.document()).expect("compile graph v2");
    let texts_v2 = scene_texts(&scene_v2).join("\n");
    assert_ne!(texts_v1, texts_v2, "ModelEquation rendered text must update when upstream XY data changes");
    assert!(texts_v2.contains('5') && texts_v2.contains('3'), "expected updated slope=5/intercept=3 text, got:\n{texts_v2}");

    // Resize canvas with ProportionalScale -> bounds_pt and font_size_pt scale proportionally
    let size_before = match session.document().graph_sheet(&SheetId::new("g-xy")).unwrap().spec.annotations[0] {
        GraphAnnotation::ModelEquation { bounds_pt, font_size_pt, .. } => (bounds_pt, font_size_pt),
        _ => panic!("expected ModelEquation"),
    };
    exec_ok(&mut session, "graph.applyCanvasPreset", json!({ "sheet_id": "g-xy", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }));
    let size_after = match session.document().graph_sheet(&SheetId::new("g-xy")).unwrap().spec.annotations[0] {
        GraphAnnotation::ModelEquation { bounds_pt, font_size_pt, .. } => (bounds_pt, font_size_pt),
        _ => panic!("expected ModelEquation"),
    };
    let expected_w = FigureCanvasPreset::SingleColumn88mm.dimensions_pt().width;
    assert!(size_after.0.width < size_before.0.width && expected_w > 0.0);
    assert!(size_after.1 < size_before.1);

    // Clone pipeline remaps ModelEquation result_sheet_id
    let cloned = exec_ok(&mut session, "sheet.clonePipeline", json!({ "sheet_id": "xy-1", "prefix": "clone", "clear_data": false }));
    let cloned_graph_id = cloned.payload["graph_sheet_ids"][0].as_str().expect("cloned graph id");
    let cloned_res_id = cloned.payload["result_sheet_ids"][0].as_str().expect("cloned res id");
    let cloned_graph = session.document().graph_sheet(&SheetId::new(cloned_graph_id)).unwrap();
    assert!(matches!(
        &cloned_graph.spec.annotations[0],
        GraphAnnotation::ModelEquation { result_sheet_id, .. } if result_sheet_id.as_str() == cloned_res_id
    ));

    // Format cycle detection catches synthetic cycle involving ModelEquation
    let mut cyclic = session.document().clone();
    let g_id = SheetId::new("g-xy");
    cyclic.graph_sheet_mut(&g_id).unwrap().spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: g_id.clone(),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: RectPt::new(10.0, 10.0, 80.0, 20.0),
        font_size_pt: 9.0,
        bordered: false,
    });
    assert!(matches!(detect_dependency_cycles(&cyclic), Err(FormatError::DependencyCycle { .. })));
    assert!(recalculate_project(&mut cyclic).is_err());
}

#[test]
fn analysis_wave20_21_categorical_agreement_binomial_and_reports() {
    let mut doc = ProjectDocument::new("Wave 20-21 Categorical & Agreement").unwrap();

    let mut pow_bin = PartsOfWholeTable::new(1, 2).unwrap();
    pow_bin.slice_labels = vec![String::from("Responder"), String::from("Non-responder")];
    pow_bin.data_sets[0].cells[0][0] = Cell::number(18.0);
    pow_bin.data_sets[0].cells[1][0] = Cell::number(2.0);
    doc.add_data_sheet(DataSheet::new("pow-bin", "Binomial Counts", TableArchetype::PartsOfWhole(pow_bin)).unwrap()).unwrap();

    let mut pow_gof = PartsOfWholeTable::new(1, 3).unwrap();
    pow_gof.slice_labels = vec![String::from("AA"), String::from("AB"), String::from("BB")];
    pow_gof.data_sets[0].cells[0][0] = Cell::number(25.0);
    pow_gof.data_sets[0].cells[1][0] = Cell::number(50.0);
    pow_gof.data_sets[0].cells[2][0] = Cell::number(25.0);
    doc.add_data_sheet(DataSheet::new("pow-gof", "Mendelian 1:2:1", TableArchetype::PartsOfWhole(pow_gof)).unwrap()).unwrap();

    let mut ck = ContingencyTable::new(3, 3).unwrap();
    ck.row_titles = vec!["Mild".into(), "Mod".into(), "Sev".into()];
    for (r, row) in [[10.0, 2.0, 0.0], [1.0, 12.0, 2.0], [0.0, 1.0, 9.0]].iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            ck.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    doc.add_data_sheet(DataSheet::new("ct-ck", "Raters Pair", TableArchetype::Contingency(ck)).unwrap()).unwrap();

    let mut fk = ContingencyTable::new(3, 5).unwrap();
    for (r, row) in [[3.0, 0.0, 0.0], [0.0, 3.0, 0.0], [2.0, 1.0, 0.0], [0.0, 0.0, 3.0], [1.0, 1.0, 1.0]].iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            fk.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    doc.add_data_sheet(DataSheet::new("ct-fk", "Multi-Rater", TableArchetype::Contingency(fk)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    exec_ok(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "res-binom", "source_sheet_id": "pow-bin", "kind": "exact_binomial", "null_proportion": 0.5 }),
    );
    assert!((scalar_val(&session, "res-binom", "proportion") - 0.9).abs() < 1e-12);
    assert!((scalar_val(&session, "res-binom", "null_proportion") - 0.5).abs() < 1e-12);
    exec_ok(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-binom", "null_proportion": 0.8 }));
    assert!((scalar_val(&session, "res-binom", "null_proportion") - 0.8).abs() < 1e-12);

    exec_ok(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "res-gof",
            "source_sheet_id": "pow-gof",
            "kind": "contingency",
            "method": "pearson",
            "expected_proportions": [1.0, 2.0, 1.0]
        }),
    );
    assert!(scalar_val(&session, "res-gof", "Goodness-of-fit chi-square") < 1e-9);

    exec_ok(&mut session, "analysis.run", json!({ "sheet_id": "res-ck", "source_sheet_id": "ct-ck", "kind": "cohen_kappa", "weighting": "linear" }));
    let k_lin = scalar_val(&session, "res-ck", "kappa");
    exec_ok(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-ck", "weighting": "quadratic" }));
    let k_quad = scalar_val(&session, "res-ck", "kappa");
    assert!(k_quad > k_lin && k_quad < 1.0);
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("res-ck")).unwrap().analysis.kind,
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic }, .. })
    ));

    exec_ok(&mut session, "analysis.run", json!({ "sheet_id": "res-fk", "source_sheet_id": "ct-fk", "kind": "fleiss_kappa" }));
    assert!(scalar_val(&session, "res-fk", "kappa").is_finite());

    assert_reports_contain(
        session.document(),
        &[
            ("res-binom", &["null proportion p0 = 0.8", "Clopper-Pearson", "Wilson score", "Agresti-Coull"]),
            ("res-gof", &["custom ratios (1, 2, 1"]),
            ("res-ck", &["Cohen's kappa", "quadratic Fleiss-Cohen"]),
            ("res-fk", &["Fleiss' chance-corrected kappa"]),
        ],
    );
}

#[test]
fn analysis_wave20_21_paired_sample_cluster_factorial_and_reports() {
    let mut doc = ProjectDocument::new("Wave 20-21 Paired/Cluster/Factorial").unwrap();

    let mut col_pair = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 6).unwrap();
    col_pair.data_sets[0].title = String::from("Post");
    col_pair.data_sets[1].title = String::from("Pre");
    for (r, (a, b)) in [(12.0, 10.0), (15.0, 11.0), (14.0, 11.0), (18.0, 12.0), (11.0, 11.0), (20.0, 14.0)].iter().enumerate() {
        col_pair.data_sets[0].cells[r][0] = Cell::number(*a);
        col_pair.data_sets[1].cells[r][0] = Cell::number(*b);
    }
    doc.add_data_sheet(DataSheet::new("col-pair", "Paired Outcome", TableArchetype::Column(col_pair)).unwrap()).unwrap();

    let mut mv = MultipleVariablesTable::new(3, 8).unwrap();
    mv.variables[0].title = String::from("Group");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("F1");
    mv.variables[1].lock_role(VariableRole::Continuous);
    mv.variables[2].title = String::from("F2");
    mv.variables[2].lock_role(VariableRole::Continuous);
    let pts = [(0.0, 0.2), (0.2, 0.1), (0.1, 0.3), (0.3, 0.0), (8.0, 8.2), (8.2, 8.1), (8.1, 8.4), (7.9, 8.0)];
    for (r, &(x, y)) in pts.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(if r < 4 { "Lo" } else { "Hi" });
        mv.variables[1].cells[r] = Cell::number(x);
        mv.variables[2].cells[r] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("mv-clust", "Cluster MV", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap();
    grp.row_titles = vec![String::from("Control"), String::from("Stim")];
    grp.data_sets[0].title = String::from("Vehicle");
    grp.data_sets[1].title = String::from("Drug");
    let cell_vals = [[[10.0, 10.5, 9.8], [20.0, 20.4, 19.6]], [[12.0, 12.2, 11.8], [30.0, 30.5, 29.7]]];
    for (c, col_rows) in cell_vals.iter().enumerate() {
        for (r, reps) in col_rows.iter().enumerate() {
            for (s_idx, &v) in reps.iter().enumerate() {
                grp.data_sets[c].cells[r][s_idx] = Cell::number(v);
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("grp-2x2", "Factorial 2x2", TableArchetype::Grouped(grp)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    exec_ok(&mut session, "analysis.run", json!({ "sheet_id": "res-sign", "source_sheet_id": "col-pair", "kind": "paired_sign_test" }));
    assert!((scalar_val(&session, "res-sign", "median_difference") - 3.5).abs() < 1e-12);
    assert!((scalar_val(&session, "res-sign", "n_positive") - 5.0).abs() < 1e-12);
    assert!((scalar_val(&session, "res-sign", "n_ties") - 1.0).abs() < 1e-12);

    exec_ok(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "res-rs",
            "source_sheet_id": "mv-clust",
            "kind": "random_sample",
            "seed": 11,
            "sample_size": 4,
            "stratify_column": 0,
            "proportional_allocation": true
        }),
    );
    assert_eq!(session.document().result_sheet(&SheetId::new("res-rs")).unwrap().analyzable_data.as_ref().unwrap().archetype.row_count(), 4);
    exec_ok(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-rs", "sample_size": 6 }));
    assert_eq!(session.document().result_sheet(&SheetId::new("res-rs")).unwrap().analyzable_data.as_ref().unwrap().archetype.row_count(), 6);
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("res-rs")).unwrap().analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { sample_size: 6, .. })
    ));

    exec_ok(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "res-cl",
            "source_sheet_id": "mv-clust",
            "kind": "clustering",
            "selected_data_sets": [1, 2],
            "distance": "manhattan",
            "linkage": "upgma",
            "k_means_k": null,
            "k_sweep_max": 4
        }),
    );
    let r_cl = session.document().result_sheet(&SheetId::new("res-cl")).unwrap();
    assert_eq!(r_cl.status, ResultStatus::UpToDate);
    assert!(matches!(
        r_cl.analysis.kind,
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: None,
            distance: ClusterDistance::Manhattan,
            linkage: ClusterLinkage::Upgma,
            k_sweep_max: Some(4)
        })
    ));
    exec_ok(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "res-cl", "k_means_k": 2, "distance": "cosine", "linkage": "ward", "k_sweep_max": 3 }),
    );
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("res-cl")).unwrap().analysis.kind,
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(2),
            distance: ClusterDistance::Cosine,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: Some(3)
        })
    ));

    exec_ok(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "res-2way", "source_sheet_id": "grp-2x2", "kind": "two_way_anova", "multiplicity": "tukey" }),
    );
    let mut doc_recalc = session.document().clone();
    let rep = recalculate_project_with_report(&mut doc_recalc).expect("dag recalc");
    assert!(rep.evaluated_order.contains(&SheetId::new("res-2way")));
    let two_way_tabs = &doc_recalc.result_sheet(&SheetId::new("res-2way")).unwrap().tabs;
    assert!(two_way_tabs.iter().any(|t| t.kind == ResultTabKind::MultipleComparisons), "expected FactorialAnova MultipleComparisons tab");

    assert_reports_contain(
        session.document(),
        &[
            ("res-sign", &["paired sign test", "exact order-statistic confidence interval"]),
            ("res-rs", &["Fisher-Yates random sampling without replacement", "n = 6 rows"]),
            ("res-cl", &["cosine dissimilarity", "Ward.D2 minimum-variance", "cophenetic correlation", "k = 2 clusters"]),
        ],
    );

    let _ = AnalysisSpec::new(
        "typed-sign",
        "col-pair",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..TwoGroupSpec::default() }),
    )
    .unwrap();
    let _ = MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true };
}
