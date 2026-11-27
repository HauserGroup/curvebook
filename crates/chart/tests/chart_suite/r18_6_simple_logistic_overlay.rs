#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Chart overlay & equation box tests for R18.6 Standalone XY Simple Logistic Regression.

use curvebook_chart::equation_box::{ModelEquationParams, render_model_equation_annotation, resolve_model_equation_tex};
use curvebook_chart::scales::resolve_template_hooks;
use curvebook_chart::{ChartScene, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, DataSheet, DiagnosticPlotKind, EquationDisplayMode, GlmFamily, GlmSpec, GraphAnnotation, GraphKind, GraphSheet,
    LinearRegressionSpec, MultipleVariablesTable, ProjectDocument, RectPt, RegressionOverlaySpec, ResultScalarEntry, ResultSheet, ResultStatus,
    SheetId, SimpleLogisticSpec, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_fit::{HookContext, execute_analysis};
use curvebook_stats::run_analysis;

use super::common::scene_texts;

#[test]
fn sigmoid_curve_overlay_and_equation_box_compile_cleanly() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 12).expect("xy");
    let pts = [
        (0.5, 0.0),
        (1.0, 0.0),
        (1.5, 0.0),
        (2.0, 1.0),
        (2.5, 0.0),
        (3.0, 1.0),
        (3.5, 0.0),
        (4.0, 1.0),
        (4.5, 1.0),
        (5.0, 1.0),
        (5.5, 1.0),
        (6.0, 1.0),
    ];
    for (r, (x, y)) in pts.into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sheet = DataSheet::new("xy-1", "Binary Dose", TableArchetype::Xy(xy)).expect("sheet");
    let sl_spec = SimpleLogisticSpec {
        emit_row_classification: true,
        confidence_bands: true,
        classification_cutoff: 0.5,
        plot_x_min: Some(0.0),
        plot_x_max: Some(7.0),
        plot_segments: Some(80),
    };
    let spec = AnalysisSpec::new("res-sl", "xy-1", AnalysisKind::SimpleLogisticRegression(sl_spec)).expect("spec");
    let result = run_analysis(&sheet, &spec).expect("run_analysis");

    let mut graph = GraphSheet::new_uninitialized_for_data("graph-1", &sheet).expect("graph");
    graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::try_new("res-sl").expect("sheet id"),
        sample_segments: 256,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    graph.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::try_new("res-sl").expect("sheet id"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: RectPt::new(80.0, 40.0, 220.0, 40.0),
        font_size_pt: 10.0,
        bordered: true,
    });

    let mut doc = ProjectDocument::new("Logistic Chart").expect("doc");
    doc.data_sheets.push(sheet);
    doc.result_sheets.push(result);
    doc.graph_sheets.push(graph.clone());

    let scene = compile_graph(&graph, &doc).expect("compile_graph");
    assert!(scene.warnings.is_empty(), "expected zero chart warnings, got {:?}", scene.warnings);
    assert!(!scene.primitives.is_empty());
}

#[test]
fn simple_logistic_and_glm_roc_overlay_parity_and_no_raw_xy_fabrication() {
    let rows: [(f64, f64, f64); 12] = [
        (-5.0, 0.0, 0.0),
        (-4.0, 0.0, 1.0),
        (-3.0, 0.0, 0.0),
        (-2.0, 0.0, 0.0),
        (-1.0, 1.0, 1.0),
        (0.0, 0.0, 0.0),
        (1.0, 1.0, 1.0),
        (2.0, 1.0, 0.0),
        (3.0, 1.0, 1.0),
        (4.0, 1.0, 1.0),
        (5.0, 1.0, 1.0),
        (5.5, 1.0, 0.0),
    ];
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 12).expect("xy");
    xy.data_sets[0].title = String::from("DrugA");
    xy.data_sets[1].title = String::from("DrugB");
    for (r, (x, y0, y1)) in rows.into_iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x)).expect("x");
        xy.data_sets[0].cells[r][0] = Cell::number(y0);
        xy.data_sets[1].cells[r][0] = Cell::number(y1);
    }
    let xy_ds = DataSheet::new("xy-roc-sl", "Binary Dose", TableArchetype::Xy(xy)).expect("ds");
    let sl_res = run_analysis(
        &xy_ds,
        &AnalysisSpec::new("res-sl", "xy-roc-sl", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("spec"),
    )
    .expect("run simple logistic");
    let (auc_ds0, auc_ds1) = (sl_res.lookup_scalar("ROC_AUC", Some(0)).expect("DS0 AUC"), sl_res.lookup_scalar("ROC_AUC", Some(1)).expect("DS1 AUC"));
    assert!((auc_ds0 - auc_ds1).abs() > 0.05);
    let lin_res = execute_analysis(
        &AnalysisSpec::new("res-lin", "xy-roc-sl", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).expect("lin spec"),
        &xy_ds,
        &HookContext::default(),
    )
    .expect("linear fit");

    let mut doc = ProjectDocument::new("LogisticRocParity").expect("doc");
    doc.add_data_sheet(xy_ds.clone()).expect("add xy");
    doc.add_result_sheet(lin_res).expect("add lin first");
    doc.add_result_sheet(sl_res.clone()).expect("add sl");

    // 1. SimpleLogistic ROC overlay parity across res-sl (DS0/DS1) and xy-roc-sl (skipping res-lin).
    for (target_id, ds_idx, expected_auc) in [("res-sl", 0usize, auc_ds0), ("res-sl", 1, auc_ds1), ("xy-roc-sl", 0, auc_ds0)] {
        let mut g = GraphSheet::new_uninitialized_for_data(format!("g-roc-{target_id}-{ds_idx}"), &xy_ds).expect("g");
        g.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
        g.spec.primary_source_sheet_id = SheetId::new(target_id);
        g.spec.series.truncate(1);
        g.spec.series[0].data_set_index = ds_idx;
        let scene = compile_graph(&g, &doc).expect("compile roc");
        assert!(scene.warnings.is_empty(), "warnings={:?}", scene.warnings);
        let expected_lbl = format!("AUC = {expected_auc:.3}");
        assert!(scene_texts(&scene).iter().any(|t| t.contains(&expected_lbl)), "missing `{expected_lbl}`");
    }

    // 2. Logistic GLM ROC curve matches DeLong AUC parity.
    let mut mv = MultipleVariablesTable::new(3, 12).expect("mv");
    for (i, title) in ["Outcome", "Dose", "Age"].into_iter().enumerate() {
        mv.variables[i].title = String::from(title);
    }
    for (r, (x, y0, _)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(y0);
        mv.variables[1].cells[r] = Cell::number(x);
        mv.variables[2].cells[r] = Cell::number(30.0 + (r as f64) * 2.5);
    }
    let mv_ds = DataSheet::new("mv-glm", "GLM Cohort", TableArchetype::MultipleVariables(mv)).expect("mv ds");
    let glm_res = run_analysis(
        &mv_ds,
        &AnalysisSpec::new(
            "res-glm-logit",
            "mv-glm",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Logistic,
                response_column: 0,
                predictor_columns: vec![1, 2],
                reference_levels: Vec::new(),
                compute_vif: false,
                ..GlmSpec::default()
            }),
        )
        .expect("glm spec"),
    )
    .expect("run glm logistic");
    let glm_auc = glm_res.lookup_scalar("ROC AUC", None).expect("GLM ROC AUC");
    doc.add_data_sheet(mv_ds.clone()).expect("add mv");
    doc.add_result_sheet(glm_res).expect("add glm");
    let mut g_glm = GraphSheet::new_uninitialized_for_data("g-roc-glm", &mv_ds).expect("g glm");
    g_glm.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
    g_glm.spec.primary_source_sheet_id = SheetId::new("res-glm-logit");
    let glm_scene = compile_graph(&g_glm, &doc).expect("compile glm roc");
    assert!(glm_scene.warnings.is_empty(), "warnings={:?}", glm_scene.warnings);
    assert!(scene_texts(&glm_scene).iter().any(|t| t.contains(&format!("AUC = {glm_auc:.3}"))));

    // 3. Missing ROC overlay must never fabricate bogus AUC from raw XY (Dose, {0,1}).
    let mut doc_no_roc = ProjectDocument::new("NoRoc").expect("doc no roc");
    doc_no_roc.add_data_sheet(xy_ds.clone()).expect("add xy");
    let mut stripped = sl_res;
    stripped.fitted_curves.retain(|fc| !fc.equation_id.eq_ignore_ascii_case("ROC"));
    doc_no_roc.add_result_sheet(stripped).expect("add stripped");
    for target_id in ["res-sl", "xy-roc-sl"] {
        let mut g = GraphSheet::new_uninitialized_for_data(format!("g-noroc-{target_id}"), &xy_ds).expect("g");
        g.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve);
        g.spec.primary_source_sheet_id = SheetId::new(target_id);
        let scene = compile_graph(&g, &doc_no_roc).expect("compile stripped roc");
        assert!(scene_texts(&scene).iter().all(|t| !t.contains("AUC =")), "fabricated AUC for `{target_id}`");
        assert!(scene.warnings.iter().any(|w| w.code == "missing_roc_overlay") && scene.warnings.iter().any(|w| w.code == "empty_roc"));
    }

    // 4. XY graph with SimpleLogistic overlay renders sigmoid (not ROC) cleanly.
    let mut g_xy = GraphSheet::new_uninitialized_for_data("g-xy-sl", &xy_ds).expect("g xy");
    g_xy.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-sl"),
        sample_segments: 256,
        show_confidence_band: true,
        show_prediction_band: false,
        shaded_ribbon: true,
    });
    assert!(compile_graph(&g_xy, &doc).expect("compile xy sigmoid").warnings.is_empty());
}

#[test]
fn simple_logistic_multidataset_equation_box_sig_figs_and_hooks() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 12).expect("xy");
    xy.data_sets[0].title = String::from("DrugA");
    xy.data_sets[1].title = String::from("DrugB");
    let xs = [0.0, 1e5, 2e5, 3e5, 4e5, 5e5, 6e5, 7e5, 8e5, 9e5, 1e6, 1.1e6];
    let y0s = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let y1s = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    for r in 0..12 {
        xy.set_x_cell(r, Cell::number(xs[r])).expect("x");
        xy.data_sets[0].cells[r][0] = Cell::number(y0s[r]);
        xy.data_sets[1].cells[r][0] = Cell::number(y1s[r]);
    }
    let ds = DataSheet::new("xy-sl-multi", "Large-Scale Binary", TableArchetype::Xy(xy)).expect("ds");
    let res = run_analysis(
        &ds,
        &AnalysisSpec::new("res-sl-multi", "xy-sl-multi", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("spec"),
    )
    .expect("run simple logistic");
    let b1_ds0 = res.lookup_scalar("Beta1", Some(0)).expect("DS0 Beta1");
    let _b1_ds1 = res.lookup_scalar("Beta1", Some(1)).expect("DS1 Beta1");
    let (x50_ds0, x50_ds1) = (res.lookup_scalar("X50", Some(0)).expect("DS0 X50"), res.lookup_scalar("X50", Some(1)).expect("DS1 X50"));
    assert!(b1_ds0.abs() > 0.0 && b1_ds0.abs() < 1e-4);
    assert!((x50_ds0 - x50_ds1).abs() > 1e5);

    let mut doc = ProjectDocument::new("LogisticSigFigs").expect("doc");
    doc.add_data_sheet(ds.clone()).expect("add ds");
    doc.add_result_sheet(res).expect("add res");

    let tex0 = resolve_model_equation_tex(&doc, &SheetId::new("res-sl-multi"), 0, EquationDisplayMode::Substituted).expect("DS0 tex");
    let tex1 = resolve_model_equation_tex(&doc, &SheetId::new("res-sl-multi"), 1, EquationDisplayMode::Substituted).expect("DS1 tex");
    assert_ne!(tex0, tex1);
    for tex in [&tex0, &tex1] {
        assert!(!tex.contains("0.0000") && tex.contains("\\times 10^{"), "tex={tex}");
    }

    let mut scene = ChartScene::new("sl-eq", "Test", 480.0, 320.0, None);
    render_model_equation_annotation(
        &mut scene,
        &doc,
        &ModelEquationParams {
            result_sheet_id: &SheetId::new("res-sl-multi"),
            data_set_index: 1,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(20.0, 20.0, 0.0, 0.0),
            font_size_pt: 10.0,
            bordered: true,
        },
    );
    let rendered = scene_texts(&scene).join(" ");
    assert!(!rendered.contains("Fit parameters unavailable") && !rendered.contains("0.0000") && !rendered.contains("\\times"));

    let hook0 = resolve_template_hooks("{{result:res-sl-multi:0:Beta1}} | {{result:res-sl-multi:0:X50}}", &doc, Some(&ds.id));
    let hook1 = resolve_template_hooks("{{result:res-sl-multi:1:Beta1}} | {{result:res-sl-multi:1:X50}}", &doc, Some(&ds.id));
    assert!(!hook0.contains("{{") && !hook0.contains("0.0000") && !hook1.contains("{{") && !hook1.contains("0.0000") && hook0 != hook1);

    // Legacy pre-Wave-1 fallback (`data_set_index: None`, `"DrugB.Beta0"` / `"DrugB.Beta1"` / `"DrugB.X50"`).
    let mut legacy = ResultSheet::new(
        AnalysisSpec::new("res-sl-legacy", "xy-sl-multi", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("spec"),
    )
    .expect("legacy");
    legacy.status = ResultStatus::UpToDate;
    legacy.scalars = [("DrugA.Beta0", -2.0), ("DrugA.Beta1", 4.0e-6), ("DrugB.Beta0", -3.5), ("DrugB.Beta1", 5.0e-6), ("DrugB.X50", 7.0e5)]
        .into_iter()
        .map(|(k, value)| ResultScalarEntry { key: String::from(k), data_set_index: None, value, units: None })
        .collect();
    doc.add_result_sheet(legacy).expect("add legacy");
    let legacy_tex = resolve_model_equation_tex(&doc, &SheetId::new("res-sl-legacy"), 1, EquationDisplayMode::Substituted).expect("legacy tex");
    assert!(legacy_tex.contains("-3.5") && legacy_tex.contains("5 \\times 10^{-6}"), "legacy_tex={legacy_tex}");
    assert_eq!(resolve_template_hooks("{{result:res-sl-legacy:1:X50}}", &doc, Some(&ds.id)), "7.000e5");
}
