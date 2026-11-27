//! Integration tests wiring Waves 20-21 (`doc`, `transform`, `stats`, `fit`, `chart`)
//! through `curvebook-format` and `curvebook-engine`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ClusterDistance, ClusterLinkage, ColumnDesignHint, ConstantValue, ContingencyMethod, ContingencySpec,
    DataSheet, EquationDisplayMode, FigureCanvasPreset, GraphAnnotation, InfoSheet, KappaWeighting, MultiplicityMethod, MultiplicitySpec,
    MultivariateSpec, PartsOfWholeTable, ProjectDocument, RectPt, ResultStatus, ResultTabKind, SheetId, TableArchetype, TableKind,
    TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole,
};
use curvebook_engine::dag::{recalculate_project, recalculate_project_with_report};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::{ErrorCode, Session};
use curvebook_format::{FormatError, detect_dependency_cycles};
use serde_json::json;

use super::common::{
    assert_err_code, col_sheet_hint, contingency_sheet, ds, grouped_rep_sheet, gs, info, mv_sheet, ok_cmd as exec_ok, rs, run_cmds, session_with,
    xy_sheet,
};

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
    rs(session, res_id).lookup_scalar(name, None).unwrap_or_else(|| panic!("missing scalar `{name}` on `{res_id}`"))
}

fn assert_reports_contain(doc: &ProjectDocument, checks: &[(&str, &[&str])]) {
    for (res_id, expected_substrs) in checks {
        let md = generate_methods_report(doc, &SheetId::new(*res_id), ReportFormat::Markdown).expect("report");
        for needle in *expected_substrs {
            assert!(md.contains(needle), "expected `{needle}` in `{res_id}` report:\n{md}");
        }
        let lower = md.to_ascii_lowercase();
        for forbidden in
            ["recommend switching", "switch to", "consider switching", "use welch", "use mann-whitney", "apply log transform", "transform the data"]
        {
            assert!(!lower.contains(forbidden), "QOL-7 violation `{forbidden}` in report:\n{md}");
        }
    }
}

#[test]
fn table_randomize_groups_arrives_constants_and_undo_redo() {
    let mut session = Session::new();
    let params = json!({ "sheet_id": "rand-cb", "title": "Preclinical Cohort", "groups": [{ "label": "HighDose", "weight": 2 }, { "label": "LowDose", "weight": 1 }, "Vehicle"], "subject_count": 12, "scheme": "complete_balanced", "seed": 4242 });
    let out1 = exec_ok(&mut session, "table.randomizeGroups", params.clone());
    assert!(out1.mutated && out1.payload["subject_count"] == 12);
    let info1_id = out1.payload["info_sheet_id"].as_str().expect("info_sheet_id").to_owned();
    assert_eq!(out1.payload["arrive_constants"]["randomization.method"], "complete_balanced");
    assert_eq!(out1.payload["arrive_constants"]["randomization.allocation_ratio"], "2:1:1");
    assert_eq!(out1.payload["arrive_constants"]["randomization.seed"], "4242");

    let d1 = ds(&session, "rand-cb");
    assert_eq!(
        (d1.kind(), d1.archetype.row_count(), d1.info_sheet_id.as_ref().map(SheetId::as_str)),
        (TableKind::MultipleVariables, 12, Some(info1_id.as_str()))
    );
    assert_eq!(
        info(&session, &info1_id).get_constant("randomization.allocation_ratio").map(|c| &c.value),
        Some(&ConstantValue::Text(String::from("2:1:1")))
    );

    // Determinism with same seed
    let mut session_b = Session::new();
    exec_ok(&mut session_b, "table.randomizeGroups", params);
    assert_eq!(ds(&session, "rand-cb").archetype, ds(&session_b, "rand-cb").archetype);

    assert!(session.undo().unwrap());
    assert!(session.document().data_sheet(&SheetId::new("rand-cb")).is_none() && session.document().info_sheet(&SheetId::new(&info1_id)).is_none());
    assert!(session.redo().unwrap());
    assert_eq!(info(&session, &info1_id).get_constant("randomization.seed").map(|c| &c.value), Some(&ConstantValue::Text(String::from("4242"))));

    // Permuted-block updating existing InfoSheet
    let out2 = exec_ok(
        &mut session,
        "table.randomizeGroups",
        json!({ "sheet_id": "rand-pb", "groups": ["Vehicle", "Drug"], "subject_ids": ["M-01", "M-02", "M-03", "M-04", "M-05", "M-06"], "scheme": "permuted_block", "block_sizes": [2, 4], "seed": 99, "info_sheet_id": &info1_id }),
    );
    assert_eq!(
        (&out2.payload["arrive_constants"]["randomization.method"], &out2.payload["arrive_constants"]["randomization.block_sizes"]),
        (&json!("permuted_block"), &json!("2, 4"))
    );
    assert_eq!(
        info(&session, &info1_id).get_constant("randomization.total_subjects").map(|c| &c.value),
        Some(&ConstantValue::Text(String::from("6")))
    );
    assert!(session.undo().unwrap());
    assert_eq!(
        info(&session, &info1_id).get_constant("randomization.total_subjects").map(|c| &c.value),
        Some(&ConstantValue::Text(String::from("12")))
    );
}

#[test]
fn table_random_sample_rows_stratified_provenance_and_undo_redo() {
    let mut doc = ProjectDocument::new("Sampling").unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-main", "Study Info").unwrap()).unwrap();
    let strata: Vec<Cell> = (0..12).map(|r| Cell::text(if r < 8 { "A" } else { "B" })).collect();
    let resps: Vec<Cell> = (1..=12).map(|r| Cell::number(r as f64)).collect();
    let mut src = mv_sheet(
        "mv-src",
        "Source Cohort",
        &[],
        &[("Stratum", Some(VariableRole::Categorical), &strata), ("Response", Some(VariableRole::Continuous), &resps)],
    );
    src.info_sheet_id = Some(SheetId::new("info-main"));
    doc.add_data_sheet(src).unwrap();

    let mut session = Session::from_document(doc);
    let out = exec_ok(
        &mut session,
        "table.randomSampleRows",
        json!({ "source_sheet_id": "mv-src", "sample_size": 6, "seed": 777, "stratify_column": 0, "proportional_allocation": true, "sheet_id": "mv-sample" }),
    );
    assert_eq!((&out.payload["row_count"], &out.payload["info_sheet_id"]), (&json!(6), &json!("info-main")));

    let TableArchetype::MultipleVariables(ref smv) = ds(&session, "mv-sample").archetype else { panic!("expected MV") };
    assert_eq!(smv.variables[0].cells.iter().filter(|c| c.active_text() == Some("A")).count(), 4);
    assert_eq!(smv.variables[0].cells.iter().filter(|c| c.active_text() == Some("B")).count(), 2);
    assert_eq!(info(&session, "info-main").get_constant("sample.seed").map(|c| &c.value), Some(&ConstantValue::Text(String::from("777"))));
    assert_eq!(info(&session, "info-main").get_constant("sample.sample_size").map(|c| &c.value), Some(&ConstantValue::Text(String::from("6"))));

    assert!(session.undo().unwrap());
    assert!(session.document().data_sheet(&SheetId::new("mv-sample")).is_none() && info(&session, "info-main").get_constant("sample.seed").is_none());
    assert!(session.redo().unwrap() && session.document().data_sheet(&SheetId::new("mv-sample")).is_some());
}

#[test]
fn table_simulate_distributions_all_six_and_undo_redo() {
    let mut session = Session::new();
    let out_col = exec_ok(
        &mut session,
        "table.simulateDistributions",
        json!({
            "sheet_id": "sim-col", "title": "Six Parametric Variates", "n_rows": 25, "seed": 123, "use_multiple_variables": false,
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
    assert_eq!((&out_col.payload["n_rows"], &out_col.payload["n_cols"]), (&json!(25), &json!(6)));
    let TableArchetype::Column(ref col_tbl) = ds(&session, "sim-col").archetype else { panic!("expected Column") };
    assert_eq!(col_tbl.data_sets.len(), 6);
    assert!(col_tbl.active_column_values(0).iter().all(|&v| (-2.0..=4.0).contains(&v)));
    assert!(col_tbl.active_column_values(2).iter().all(|&v| v > 0.0));
    assert!(col_tbl.active_column_values(3).iter().all(|&v| (0.0..=10.0).contains(&v) && v.fract() == 0.0));
    assert!(col_tbl.active_column_values(4).iter().all(|&v| v >= 0.0 && v.fract() == 0.0));
    assert!(col_tbl.active_column_values(5).iter().all(|&v| v >= 0.0));

    let out_mv = exec_ok(
        &mut session,
        "table.simulateDistributions",
        json!({ "sheet_id": "sim-mv", "n_rows": 10, "seed": 456, "use_multiple_variables": true, "columns": [{ "name": "X", "distribution": { "kind": "gaussian", "mean": 0.0, "sd": 1.0 } }, { "name": "Count", "distribution": { "Poisson": { "lambda": 2.0 } } }] }),
    );
    assert_eq!((&out_mv.payload["use_multiple_variables"], ds(&session, "sim-mv").kind()), (&json!(true), TableKind::MultipleVariables));
    assert!(session.undo().unwrap() && session.document().data_sheet(&SheetId::new("sim-mv")).is_none());
    assert!(session.redo().unwrap() && session.document().data_sheet(&SheetId::new("sim-mv")).is_some());
}

#[test]
fn graph_add_annotation_model_equation_resize_clone_and_live_recalc() {
    let mut session = session_with(
        "Model Equation Workflow",
        vec![xy_sheet("xy-1", "Calibration", "X", &[0.0, 1.0, 2.0, 3.0, 4.0], &[("Linear Series", &[1.0, 3.0, 5.0, 7.0, 9.0])])],
    );
    run_cmds(
        &mut session,
        &[
            ("fit.run", json!({ "sheet_id": "res-lin", "source_sheet_id": "xy-1", "kind": "linear" })),
            ("graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-1", "kind": "xy" })),
        ],
    );
    let _ = assert_err_code(
        &mut session,
        "graph.addAnnotation",
        json!({ "sheet_id": "g-xy", "kind": "equation", "result_sheet_id": "missing-res" }),
        ErrorCode::NotFound,
    );

    run_cmds(
        &mut session,
        &[
            (
                "graph.addAnnotation",
                json!({ "sheet_id": "g-xy", "kind": "equation", "result_sheet_id": "res-lin", "data_set_index": 0, "mode": "substituted", "bounds_pt": [64.0, 42.0, 200.0, 40.0], "font_size_pt": 11.0, "bordered": true }),
            ),
            (
                "graph.addAnnotation",
                json!({ "sheet_id": "g-xy", "kind": "model_equation", "result_sheet_id": "res-lin", "mode": "symbolic", "bounds_pt": [64.0, 90.0, 180.0, 32.0] }),
            ),
        ],
    );
    detect_dependency_cycles(session.document()).unwrap();
    let g_v1 = gs(&session, "g-xy");
    assert_eq!(g_v1.spec.annotations.len(), 2);
    assert!(matches!(&g_v1.spec.annotations[1], GraphAnnotation::ModelEquation { mode: EquationDisplayMode::Symbolic, .. }));

    let texts_v1 = scene_texts(&compile_graph(g_v1, session.document()).unwrap()).join("\n");
    assert!(texts_v1.contains("Y = "));

    exec_ok(
        &mut session,
        "table.setCells",
        json!({ "sheet_id": "xy-1", "edits": [{ "row": 0, "col": 0, "value": 3.0 }, { "row": 1, "col": 0, "value": 8.0 }, { "row": 2, "col": 0, "value": 13.0 }, { "row": 3, "col": 0, "value": 18.0 }, { "row": 4, "col": 0, "value": 23.0 }] }),
    );
    let texts_v2 = scene_texts(&compile_graph(gs(&session, "g-xy"), session.document()).unwrap()).join("\n");
    assert!(texts_v2 != texts_v1 && texts_v2.contains('5') && texts_v2.contains('3'));

    let eq_box = |s: &Session| match gs(s, "g-xy").spec.annotations[0] {
        GraphAnnotation::ModelEquation { bounds_pt, font_size_pt, .. } => (bounds_pt, font_size_pt),
        _ => panic!("expected ModelEquation"),
    };
    let size_before = eq_box(&session);
    exec_ok(&mut session, "graph.applyCanvasPreset", json!({ "sheet_id": "g-xy", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }));
    let size_after = eq_box(&session);
    assert!(
        size_after.0.width < size_before.0.width && FigureCanvasPreset::SingleColumn88mm.dimensions_pt().width > 0.0 && size_after.1 < size_before.1
    );

    let cloned = exec_ok(&mut session, "sheet.clonePipeline", json!({ "sheet_id": "xy-1", "prefix": "clone", "clear_data": false }));
    let (cg, cr) = (cloned.payload["graph_sheet_ids"][0].as_str().unwrap(), cloned.payload["result_sheet_ids"][0].as_str().unwrap());
    assert!(
        matches!(&gs(&session, cg).spec.annotations[0], GraphAnnotation::ModelEquation { result_sheet_id, .. } if result_sheet_id.as_str() == cr)
    );

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
    assert!(matches!(detect_dependency_cycles(&cyclic), Err(FormatError::DependencyCycle { .. })) && recalculate_project(&mut cyclic).is_err());
}

#[test]
fn analysis_wave20_21_categorical_agreement_binomial_and_reports() {
    let make_pow = |id: &str, title: &str, labels: &[&str], vals: &[f64]| {
        let mut pow = PartsOfWholeTable::new(1, vals.len()).unwrap();
        pow.slice_labels = labels.iter().map(|s| (*s).to_owned()).collect();
        for (r, &v) in vals.iter().enumerate() {
            pow.data_sets[0].cells[r][0] = Cell::number(v);
        }
        DataSheet::new(id, title, TableArchetype::PartsOfWhole(pow)).unwrap()
    };
    let mut session = session_with(
        "Wave 20-21 Categorical & Agreement",
        vec![
            make_pow("pow-bin", "Binomial Counts", &["Responder", "Non-responder"], &[18.0, 2.0]),
            make_pow("pow-gof", "Mendelian 1:2:1", &["AA", "AB", "BB"], &[25.0, 50.0, 25.0]),
            contingency_sheet(
                "ct-ck",
                "Raters Pair",
                &["Mild", "Mod", "Sev"],
                &["A", "B", "C"],
                &[&[10.0, 2.0, 0.0], &[1.0, 12.0, 2.0], &[0.0, 1.0, 9.0]],
            ),
            contingency_sheet(
                "ct-fk",
                "Multi-Rater",
                &[],
                &["A", "B", "C"],
                &[&[3.0, 0.0, 0.0], &[0.0, 3.0, 0.0], &[2.0, 1.0, 0.0], &[0.0, 0.0, 3.0], &[1.0, 1.0, 1.0]],
            ),
        ],
    );

    exec_ok(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "res-binom", "source_sheet_id": "pow-bin", "kind": "exact_binomial", "null_proportion": 0.5 }),
    );
    assert!(
        (scalar_val(&session, "res-binom", "proportion") - 0.9).abs() < 1e-12
            && (scalar_val(&session, "res-binom", "null_proportion") - 0.5).abs() < 1e-12
    );
    exec_ok(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-binom", "null_proportion": 0.8 }));
    assert!((scalar_val(&session, "res-binom", "null_proportion") - 0.8).abs() < 1e-12);

    exec_ok(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "res-gof", "source_sheet_id": "pow-gof", "kind": "contingency", "method": "pearson", "expected_proportions": [1.0, 2.0, 1.0] }),
    );
    assert!(scalar_val(&session, "res-gof", "Goodness-of-fit chi-square") < 1e-9);

    exec_ok(&mut session, "analysis.run", json!({ "sheet_id": "res-ck", "source_sheet_id": "ct-ck", "kind": "cohen_kappa", "weighting": "linear" }));
    let k_lin = scalar_val(&session, "res-ck", "kappa");
    exec_ok(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-ck", "weighting": "quadratic" }));
    let k_quad = scalar_val(&session, "res-ck", "kappa");
    assert!(
        k_quad > k_lin
            && k_quad < 1.0
            && matches!(
                rs(&session, "res-ck").analysis.kind,
                AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic }, .. })
            )
    );

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
    let col_pair = col_sheet_hint(
        "col-pair",
        "Paired Outcome",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[("Post", &[12.0, 15.0, 14.0, 18.0, 11.0, 20.0]), ("Pre", &[10.0, 11.0, 11.0, 12.0, 11.0, 14.0])],
    );
    let pts = [(0.0, 0.2), (0.2, 0.1), (0.1, 0.3), (0.3, 0.0), (8.0, 8.2), (8.2, 8.1), (8.1, 8.4), (7.9, 8.0)];
    let mv_clust = mv_sheet(
        "mv-clust",
        "Cluster MV",
        &[],
        &[
            ("Group", Some(VariableRole::Categorical), &(0..8).map(|r| Cell::text(if r < 4 { "Lo" } else { "Hi" })).collect::<Vec<_>>()),
            ("F1", Some(VariableRole::Continuous), &pts.iter().map(|p| Cell::number(p.0)).collect::<Vec<_>>()),
            ("F2", Some(VariableRole::Continuous), &pts.iter().map(|p| Cell::number(p.1)).collect::<Vec<_>>()),
        ],
    );
    let grp = grouped_rep_sheet(
        "grp-2x2",
        "Factorial 2x2",
        &["Control", "Stim"],
        &[("Vehicle", &[&[10.0, 10.5, 9.8], &[20.0, 20.4, 19.6]]), ("Drug", &[&[12.0, 12.2, 11.8], &[30.0, 30.5, 29.7]])],
    );
    let mut session = session_with("Wave 20-21 Paired/Cluster/Factorial", vec![col_pair, mv_clust, grp]);

    exec_ok(&mut session, "analysis.run", json!({ "sheet_id": "res-sign", "source_sheet_id": "col-pair", "kind": "paired_sign_test" }));
    assert!(
        (scalar_val(&session, "res-sign", "median_difference") - 3.5).abs() < 1e-12
            && (scalar_val(&session, "res-sign", "n_positive") - 5.0).abs() < 1e-12
            && (scalar_val(&session, "res-sign", "n_ties") - 1.0).abs() < 1e-12
    );

    exec_ok(
        &mut session,
        "transform.run",
        json!({ "sheet_id": "res-rs", "source_sheet_id": "mv-clust", "kind": "random_sample", "seed": 11, "sample_size": 4, "stratify_column": 0, "proportional_allocation": true }),
    );
    assert_eq!(rs(&session, "res-rs").analyzable_data.as_ref().unwrap().archetype.row_count(), 4);
    exec_ok(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-rs", "sample_size": 6 }));
    assert_eq!(rs(&session, "res-rs").analyzable_data.as_ref().unwrap().archetype.row_count(), 6);
    assert!(matches!(rs(&session, "res-rs").analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { sample_size: 6, .. })));

    exec_ok(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "res-cl", "source_sheet_id": "mv-clust", "kind": "clustering", "selected_data_sets": [1, 2], "distance": "manhattan", "linkage": "upgma", "k_means_k": null, "k_sweep_max": 4 }),
    );
    assert!(
        rs(&session, "res-cl").status == ResultStatus::UpToDate
            && matches!(
                rs(&session, "res-cl").analysis.kind,
                AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                    k_means_k: None,
                    distance: ClusterDistance::Manhattan,
                    linkage: ClusterLinkage::Upgma,
                    k_sweep_max: Some(4),
                    ..
                })
            )
    );
    exec_ok(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "res-cl", "k_means_k": 2, "distance": "cosine", "linkage": "ward", "k_sweep_max": 3 }),
    );
    assert!(matches!(
        rs(&session, "res-cl").analysis.kind,
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(2),
            distance: ClusterDistance::Cosine,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: Some(3),
            ..
        })
    ));

    exec_ok(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "res-2way", "source_sheet_id": "grp-2x2", "kind": "two_way_anova", "multiplicity": "tukey" }),
    );
    let mut doc_recalc = session.document().clone();
    assert!(recalculate_project_with_report(&mut doc_recalc).unwrap().evaluated_order.contains(&SheetId::new("res-2way")));
    assert!(doc_recalc.result_sheet(&SheetId::new("res-2way")).unwrap().tabs.iter().any(|t| t.kind == ResultTabKind::MultipleComparisons));

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
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedSignTest, ..Default::default() }),
    )
    .unwrap();
    let _ = MultiplicitySpec {
        method: MultiplicityMethod::TukeyHsd,
        threshold: 0.05,
        report_confidence_intervals: true,
        control_column: None,
        selected_pairs: None,
    };
}
