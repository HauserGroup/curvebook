#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Desktop UI tests for one-way MANOVA, competing-risks CIF, Cochran–Mantel–Haenszel,
//! Scheffé post-hoc comparisons, and Gardner–Altman dual-axis estimation plot wiring.

use super::common;

use curvebook_doc::{
    AnalysisKind, Cell, CmhSpec, ColumnDesignHint, CompetingRisksSpec, ContingencyTable, DataSheet, DiagnosticPlotKind, GraphKind, ManovaSpec,
    MultiplicityMethod, ProjectDocument, SheetId, TableArchetype, TableKind,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveDialog, GRAPH_TYPE_OPTIONS, default_analysis_kind_for_id};
use curvebook_ui_egui::interaction::ViewportMetrics;

use common::{make_shell_driver, seed_column_doc};

#[test]
fn advanced_stats_catalog_and_estimation_graph_type_registered() {
    for (id, expected_kind, compat_tables) in [
        ("stats.manova", AnalysisKind::Manova(ManovaSpec::default()), &[TableKind::MultipleVariables][..]),
        (
            "stats.competing_risks",
            AnalysisKind::CompetingRisks(CompetingRisksSpec::default()),
            &[TableKind::Survival, TableKind::MultipleVariables][..],
        ),
        ("stats.cmh", AnalysisKind::CochranMantelHaenszel(CmhSpec::default()), &[TableKind::Contingency, TableKind::MultipleVariables][..]),
    ] {
        assert!(ANALYSIS_CATALOG.iter().any(|item| item.id == id), "missing catalog entry `{id}`");
        let kind = default_analysis_kind_for_id(id).expect("default kind");
        assert_eq!(kind, expected_kind);
        for &tk in compat_tables {
            assert!(kind.is_compatible_with(tk), "`{id}` must be compatible with {tk:?}");
        }
    }

    let est_opt = GRAPH_TYPE_OPTIONS.iter().find(|opt| opt.id == "diag.estimation_plot").expect("diag.estimation_plot graph type");
    assert_eq!(est_opt.graph_kind, GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot));
    assert!(est_opt.compatible_tables.contains(&TableKind::Column));
    assert!(est_opt.compatible_tables.contains(&TableKind::MultipleVariables));
}

#[test]
fn cmh_analyze_picker_and_scheffe_anova_post_hoc_ui_flow() {
    // 1. CMH through Analyze picker + parameter modal continuity-correction toggle.
    let mut ct = ContingencyTable::new(4, 3).unwrap();
    ct.row_titles = vec!["Center 1".into(), "Center 2".into(), "Center 3".into()];
    for (r, vals) in [[10., 15., 20., 40.], [18., 10., 12., 25.], [8., 12., 14., 30.]].iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            ct.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let mut doc = ProjectDocument::new("CMH UI").unwrap();
    let ds_id = SheetId::new("ct-1");
    doc.add_data_sheet(DataSheet::new(ds_id.clone(), "Strata", TableArchetype::Contingency(ct)).unwrap()).unwrap();
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &ds_id));
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1024.0, 700.0, 1.0));
    driver.type_text("mantel").unwrap();
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.cmh").is_some());
    driver.click_widget_id("analyze.entry.stats.cmh").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let res_id = driver.state().app.document().active_sheet_id.clone().expect("active CMH result sheet");
    let doc_now = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_now, &res_id));
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.cmh.cc.false").is_some());
    driver.click_widget_id("edit.cmh.cc.false").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    let rs = driver.state().app.document().result_sheet(&res_id).expect("CMH result sheet");
    assert!(matches!(rs.analysis.kind, AnalysisKind::CochranMantelHaenszel(ref c) if !c.continuity_correction));

    // 2. Scheffé post-hoc selection in One-Way ANOVA modal.
    let groups: [(&str, &[f64]); 3] = [("Ctrl", &[10.0, 11.0, 12.0, 10.5]), ("Low", &[13.0, 14.0, 13.5, 14.5]), ("High", &[18.0, 19.0, 18.5, 19.5])];
    let (col_doc, col_id) = seed_column_doc("Scheffe UI", "col-1", "Groups", ColumnDesignHint::UnpairedReplicates, &groups).unwrap();
    let mut app2 = UiApp::from_session(Session::from_document(col_doc));
    app2.sync_views();
    let snap2 = app2.document().clone();
    assert!(app2.canvas_dialogs.open_analyze_dialog(&snap2, &col_id));
    let mut driver2 = make_shell_driver(app2, ViewportMetrics::new(1024.0, 700.0, 1.0));
    driver2.type_text("one-way anova").unwrap();
    driver2.click_widget_id("analyze.entry.stats.one_way_anova").unwrap();
    driver2.click_widget_id("modal.ok").unwrap();
    driver2.run_settled();

    let anova_id = driver2.state().app.document().active_sheet_id.clone().expect("active ANOVA result sheet");
    let doc_a = driver2.state().app.document().clone();
    assert!(driver2.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_a, &anova_id));
    driver2.run_settled();
    driver2.click_widget_id("edit.tab.multiple_comparisons").unwrap();
    driver2.run_settled();
    assert!(driver2.frame_meta().find_by_widget_id("edit.post_hoc.scheffe").is_some(), "Scheffe selector must render in ANOVA modal");
    driver2.click_widget_id("edit.post_hoc.scheffe").unwrap();
    driver2.click_widget_id("modal.ok").unwrap();
    driver2.run_settled();

    let rs_a = driver2.state().app.document().result_sheet(&anova_id).expect("ANOVA result sheet");
    assert!(matches!(
        rs_a.analysis.kind,
        AnalysisKind::OneWayAnova(ref a) if a.multiplicity.method == MultiplicityMethod::Scheffe
    ));
}
