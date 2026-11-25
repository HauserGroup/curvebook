#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Focused `EguiHumanPathDriver` tests for the R19.2 standalone Levene / Brown–Forsythe variance test.

#[path = "common/mod.rs"]
mod common;

use curvebook_doc::{AnalysisKind, ColumnDesignHint, LeveneCenter, LeveneTestSpec, SheetId, TableKind};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveDialog, default_analysis_kind_for_id};
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::json;

use common::{make_shell_driver, result_scalar, seed_column_doc};

const HET: [(&str, &[f64]); 4] = [
    ("Group 1", &[10.2, 10.8, 9.9, 11.1, 10.5, 10.3]),
    ("Group 2", &[12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1]),
    ("Group 3", &[18.5, 22.0, 16.4, 24.1, 19.8]),
    ("Group 4", &[25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9]),
];

fn headless_scalars(center: &str) -> (f64, f64) {
    let (doc, _) = seed_column_doc("R19.2 Levene UI", "col-het", "Variance groups", ColumnDesignHint::UnpairedReplicates, &HET).unwrap();
    let mut session = Session::from_document(doc);
    session.execute("stats.run", json!({ "sheet_id": "oracle", "source_sheet_id": "col-het", "kind": "levene", "center": center })).unwrap();
    let rs = session.document().result_sheet(&SheetId::try_new("oracle").unwrap()).unwrap();
    (result_scalar(rs, "F").unwrap(), result_scalar(rs, "P value").unwrap())
}

#[test]
fn r19_2_levene_catalog_entry_is_column_and_mv_only() {
    let item = ANALYSIS_CATALOG.iter().find(|i| i.id == "stats.levene").expect("stats.levene catalog entry");
    assert!(item.label.contains("Levene"));
    let kind = default_analysis_kind_for_id("stats.levene").expect("default kind");
    assert_eq!(kind, AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Median }));
    assert!(kind.is_compatible_with(TableKind::Column) && kind.is_compatible_with(TableKind::MultipleVariables));
    for incompatible in [TableKind::Xy, TableKind::Grouped, TableKind::Contingency, TableKind::Survival, TableKind::Nested, TableKind::PartsOfWhole] {
        assert!(!kind.is_compatible_with(incompatible), "Levene must not be offered for {incompatible:?}");
    }
}

#[test]
fn r19_2_levene_analyze_picker_and_parameter_modal_human_path() {
    let (doc, col_id) = seed_column_doc("R19.2 Levene UI", "col-het", "Variance groups", ColumnDesignHint::UnpairedReplicates, &HET).unwrap();
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &col_id));
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1024.0, 700.0, 1.0));

    driver.type_text("levene").unwrap();
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.levene").is_some());
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.one_way_anova").is_none());
    driver.click_widget_id("analyze.entry.stats.levene").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let res_id = driver.state().app.document().active_sheet_id.clone().expect("created result sheet is active");
    for (idx, (center, slug, expected_f)) in
        [(LeveneCenter::Median, "median", 6.458_547_396_160_991), (LeveneCenter::Mean, "mean", 7.895_495_746_240_47)].into_iter().enumerate()
    {
        if idx > 0 {
            let doc_now = driver.state().app.document().clone();
            assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_now, &res_id));
            driver.run_settled();
            assert!(driver.frame_meta().find_by_widget_id("edit.levene.center.median").is_some());
            driver.click_widget_id("edit.levene.center.mean").unwrap();
            driver.click_widget_id("modal.ok").unwrap();
            driver.run_settled();
            assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
        }
        let rs = driver.state().app.document().result_sheet(&res_id).expect("result sheet");
        assert_eq!(rs.analysis.kind, AnalysisKind::LeveneTest(LeveneTestSpec { center }));
        let (f_oracle, p_oracle) = headless_scalars(slug);
        assert!((result_scalar(rs, "F").unwrap() - f_oracle).abs() <= 1e-12);
        assert!((result_scalar(rs, "P value").unwrap() - p_oracle).abs() <= 1e-12);
        assert!((f_oracle - expected_f).abs() <= 1e-10);
    }
}
