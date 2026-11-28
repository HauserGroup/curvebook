#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Desktop shell UI tests for general ANCOVA, Holm-Bonferroni & Duncan multiplicity,
//! Lineweaver-Burk transform wiring, CurveCalculus / Savitzky-Golay modal wiring,
//! Kendall's tau-b correlation UI, standalone Dendrogram, and PCA Biplot UI.

use super::common;

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, CorrelationMethod, CurveCalculusOp, DataSheet, DiagnosticPlotKind, GraphKind, MultipleVariablesTable,
    MultiplicityMethod, MultivariateSpec, PharmacologyTransformPreset, ProjectDocument, SheetId, SubcolumnMode, SumOfSquaresType, TableArchetype,
    TableKind, TransformAnalysisSpec, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveDialog, GRAPH_TYPE_OPTIONS, default_analysis_kind_for_id};
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::json;

use common::{make_shell_driver, seed_column_doc};

fn seed_mv_study_doc() -> (ProjectDocument, SheetId) {
    let mut mv = MultipleVariablesTable::new(4, 9).unwrap();
    mv.variables[0].title = String::from("Response");
    mv.variables[1].title = String::from("Treatment");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Baseline");
    mv.variables[3].title = String::from("Biomarker");
    let rows = [
        (10.0, "Ctrl", 1.0, 2.0),
        (12.2, "Ctrl", 2.0, 2.5),
        (14.1, "Ctrl", 3.0, 3.1),
        (14.5, "Low", 1.0, 4.0),
        (16.3, "Low", 2.0, 4.6),
        (18.2, "Low", 3.0, 5.2),
        (20.1, "High", 1.0, 6.1),
        (22.4, "High", 2.0, 6.8),
        (24.0, "High", 3.0, 7.5),
    ];
    for (r, (y, g, x1, x2)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(y);
        mv.variables[1].cells[r] = Cell::text(g);
        mv.variables[2].cells[r] = Cell::number(x1);
        mv.variables[3].cells[r] = Cell::number(x2);
    }
    let mut doc = ProjectDocument::new("Tier 2-3 UI").unwrap();
    let id = SheetId::new("mv-study");
    doc.add_data_sheet(DataSheet::new(id.clone(), "MV Study", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();
    (doc, id)
}

fn seed_xy_kinetics_doc() -> (ProjectDocument, SheetId) {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 7).unwrap();
    xy.x_title = String::from("[S]");
    xy.data_sets[0].title = String::from("Velocity");
    for (r, &s) in [0.0_f64, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0].iter().enumerate() {
        xy.set_x_cell(r, Cell::number(s)).unwrap();
        let v = if s == 0.0 { 0.0 } else { 100.0 * s / (2.0 + s) };
        xy.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let mut doc = ProjectDocument::new("XY Kinetics UI").unwrap();
    let id = SheetId::new("xy-kin");
    doc.add_data_sheet(DataSheet::new(id.clone(), "Kinetics", TableArchetype::Xy(xy)).unwrap()).unwrap();
    (doc, id)
}

#[test]
fn catalog_and_graph_type_options_expose_ancova_curve_calculus_dendrogram_and_biplot() {
    let anc = ANALYSIS_CATALOG.iter().find(|i| i.id == "stats.ancova").expect("stats.ancova entry");
    assert!(anc.label.contains("ANCOVA"));
    let anc_kind = default_analysis_kind_for_id("stats.ancova").expect("ancova default");
    assert!(anc_kind.is_compatible_with(TableKind::MultipleVariables) && anc_kind.is_compatible_with(TableKind::Xy));

    let cc = ANALYSIS_CATALOG.iter().find(|i| i.id == "transform.curve_calculus").expect("transform.curve_calculus entry");
    assert!(cc.keywords.contains("savitzky"));
    let cc_kind = default_analysis_kind_for_id("transform.curve_calculus").expect("curve_calculus default");
    assert!(cc_kind.is_compatible_with(TableKind::Xy));

    assert!(GRAPH_TYPE_OPTIONS.iter().any(|o| o.id == "diag.dendrogram" && o.graph_kind == GraphKind::Diagnostic(DiagnosticPlotKind::Dendrogram)));
    assert!(GRAPH_TYPE_OPTIONS.iter().any(|o| o.id == "diag.pca_biplot" && o.graph_kind == GraphKind::Diagnostic(DiagnosticPlotKind::PcaBiplot)));
}

#[test]
fn ancova_kendall_biplot_and_dendrogram_modals_and_result_actions_work_end_to_end() {
    let (doc, mv_id) = seed_mv_study_doc();
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1120.0, 760.0, 1.0));

    // 1. ANCOVA via Analyze dialog -> Edit Parameters (switch SS type & Duncan multiplicity)
    let snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_analyze_dialog(&snap, &mv_id));
    driver.run_settled();
    driver.type_text("ancova").unwrap();
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.ancova").is_some());
    driver.click_widget_id("analyze.entry.stats.ancova").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let anc_id = driver.state().app.document().active_sheet_id.clone().expect("active ancova result");

    let snap2 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap2, &anc_id));
    driver.run_settled();
    driver.click_widget_id("edit.ancova.ss.type1").unwrap();
    driver.click_widget_id("edit.tab.multiple_comparisons").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.post_hoc.holm_bonferroni").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.post_hoc.duncan").is_some());
    driver.click_widget_id("edit.post_hoc.duncan").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    let anc_rs = driver.state().app.document().result_sheet(&anc_id).expect("ancova rs");
    assert!(matches!(
        &anc_rs.analysis.kind,
        AnalysisKind::Ancova(a) if a.ss_type == SumOfSquaresType::TypeI && a.multiplicity.method == MultiplicityMethod::Duncan
    ));

    // 2. Correlation matrix -> Kendall's tau-b in Edit Parameters modal
    let corr_out = driver
        .state_mut()
        .app
        .execute_command("stats.run", json!({ "source_sheet_id": mv_id.as_str(), "kind": "correlation_matrix", "selected_data_sets": [0, 2, 3] }))
        .unwrap();
    let corr_id = SheetId::new(corr_out.payload["sheet_id"].as_str().unwrap());
    let snap3 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap3, &corr_id));
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.corr.kendall").is_some());
    driver.click_widget_id("edit.corr.kendall").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    let corr_rs = driver.state().app.document().result_sheet(&corr_id).expect("corr rs");
    assert!(matches!(
        &corr_rs.analysis.kind,
        AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { method: Some(CorrelationMethod::KendallTauB), .. })
    ));

    // 3. PCA -> one-click PCA Biplot button in ResultsView header
    let pca_out = driver
        .state_mut()
        .app
        .execute_command("stats.run", json!({ "source_sheet_id": mv_id.as_str(), "kind": "pca", "selected_data_sets": [0, 2, 3] }))
        .unwrap();
    let pca_id = SheetId::new(pca_out.payload["sheet_id"].as_str().unwrap());
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": pca_id.as_str() })).unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("result.create_pca_biplot").is_some());
    driver.click_widget_id("result.create_pca_biplot").unwrap();
    driver.run_settled();
    assert!(
        driver
            .state()
            .app
            .document()
            .graph_sheets
            .iter()
            .any(|g| { g.spec.primary_source_sheet_id == pca_id && g.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::PcaBiplot) })
    );

    // 4. Clustering -> one-click Dendrogram button in ResultsView header
    let cl_out = driver
        .state_mut()
        .app
        .execute_command("stats.run", json!({ "source_sheet_id": mv_id.as_str(), "kind": "clustering", "selected_data_sets": [0, 2, 3] }))
        .unwrap();
    let cl_id = SheetId::new(cl_out.payload["sheet_id"].as_str().unwrap());
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": cl_id.as_str() })).unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("result.create_dendrogram").is_some());
    driver.click_widget_id("result.create_dendrogram").unwrap();
    driver.run_settled();
    assert!(
        driver
            .state()
            .app
            .document()
            .graph_sheets
            .iter()
            .any(|g| { g.spec.primary_source_sheet_id == cl_id && g.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Dendrogram) })
    );
}

#[test]
fn curve_calculus_savitzky_golay_lineweaver_burk_and_anova_holm_duncan_ui_work() {
    // 1. CurveCalculus (Savitzky-Golay) & Lineweaver-Burk on XY kinetics table
    let (xy_doc, xy_id) = seed_xy_kinetics_doc();
    let mut app = UiApp::from_session(Session::from_document(xy_doc));
    app.sync_views();
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1120.0, 760.0, 1.0));

    let snap = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_analyze_dialog(&snap, &xy_id));
    driver.run_settled();
    driver.type_text("savitzky").unwrap();
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.transform.curve_calculus").is_some());
    driver.click_widget_id("analyze.entry.transform.curve_calculus").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    let cc_id = driver.state().app.document().active_sheet_id.clone().expect("active curve calculus result");

    let snap2 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap2, &cc_id));
    driver.run_settled();
    driver.click_widget_id("edit.curve_calc.op.savitzky_golay").unwrap();
    driver.run_settled();
    driver.click_widget_id("edit.curve_calc.create_graph").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    let cc_rs = driver.state().app.document().result_sheet(&cc_id).expect("cc rs");
    assert!(matches!(
        &cc_rs.analysis.kind,
        AnalysisKind::CurveCalculus(c) if matches!(c.operation, CurveCalculusOp::SmoothSavitzkyGolay { window: 5, order: 2 })
    ));
    assert!(driver.state().app.document().graph_sheets.iter().any(|g| g.spec.primary_source_sheet_id == cc_id && g.spec.kind == GraphKind::Xy));

    // 2. Transform Values modal -> Pharmacology & Kinetics -> Lineweaver-Burk preset
    let lb_out = driver.state_mut().app.execute_command("transform.run", json!({ "source_sheet_id": xy_id.as_str(), "kind": "math" })).unwrap();
    let lb_id = SheetId::new(lb_out.payload["sheet_id"].as_str().unwrap());
    let snap3 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&snap3, &lb_id));
    driver.run_settled();
    driver.click_widget_id("edit.transform.mode.pharmacology").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.pharmacology.lineweaver_burk").is_some());
    driver.click_widget_id("edit.transform.pharmacology.lineweaver_burk").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    let lb_rs = driver.state().app.document().result_sheet(&lb_id).expect("lb rs");
    assert!(matches!(
        &lb_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::Math { pharmacology_preset: Some(PharmacologyTransformPreset::LineweaverBurk), .. })
    ));

    // 3. One-Way ANOVA modal exposes Holm-Bonferroni and Duncan multiplicity controls
    let (col_doc, col_id) = seed_column_doc(
        "ANOVA Mult UI",
        "col-m",
        "Groups",
        ColumnDesignHint::UnpairedReplicates,
        &[("A", &[10.0, 10.2, 9.8]), ("B", &[11.0, 11.2, 10.9]), ("C", &[13.5, 13.8, 13.4])],
    )
    .unwrap();
    let mut app2 = UiApp::from_session(Session::from_document(col_doc));
    app2.sync_views();
    let anova_out = app2.execute_command("stats.run", json!({ "source_sheet_id": col_id.as_str(), "kind": "one_way_anova" })).unwrap();
    let anova_id = SheetId::new(anova_out.payload["sheet_id"].as_str().unwrap());
    let mut driver2 = make_shell_driver(app2, ViewportMetrics::new(1120.0, 760.0, 1.0));
    let snap4 = driver2.state().app.document().clone();
    assert!(driver2.state_mut().app.canvas_dialogs.open_edit_parameters(&snap4, &anova_id));
    driver2.run_settled();
    driver2.click_widget_id("edit.tab.multiple_comparisons").unwrap();
    driver2.run_settled();
    assert!(driver2.frame_meta().find_by_widget_id("edit.post_hoc.holm_bonferroni").is_some());
    assert!(driver2.frame_meta().find_by_widget_id("edit.post_hoc.duncan").is_some());
    driver2.click_widget_id("edit.post_hoc.holm_bonferroni").unwrap();
    driver2.click_widget_id("modal.ok").unwrap();
    driver2.run_settled();
    let anova_rs = driver2.state().app.document().result_sheet(&anova_id).expect("anova rs");
    assert!(matches!(
        &anova_rs.analysis.kind,
        AnalysisKind::OneWayAnova(a) if a.multiplicity.method == MultiplicityMethod::HolmBonferroni
    ));
}
