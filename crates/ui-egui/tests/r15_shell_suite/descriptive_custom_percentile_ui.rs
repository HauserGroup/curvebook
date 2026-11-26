#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! UI tests for `DescriptiveSpec::custom_upper_percentile` in the `egui_shell` Descriptive
//! Statistics modal and `AnalyzeDialogState` / `dialog_patch` wiring.

use super::common;

use common::{ShellTestState, make_shell_driver_with_state, seed_column_doc, set_textbox};
use curvebook_doc::{AnalysisKind, ColumnDesignHint, DescriptiveSpec, SheetId};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{AnalyzeDialogState, dialog_patch};
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::i18n::LocalePreset;
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::{Map, json};

#[test]
fn descriptive_modal_custom_upper_percentile_round_trip() {
    let (doc, _) = seed_column_doc(
        "Descriptive Custom Percentile UI Test",
        "col-desc",
        "Descriptive Data",
        ColumnDesignHint::UnpairedReplicates,
        &[("Assay", &[2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0])],
    )
    .unwrap();
    let mut session = Session::from_document(doc);
    let kind_val = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "col-desc", "kind": kind_val })).unwrap();
    let res_id = SheetId::try_new("res-desc").unwrap();

    let mut app = UiApp::from_session(session);
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &res_id));
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = make_shell_driver_with_state(ShellTestState::new(app).with_commit_all().with_merge_passes(), ViewportMetrics::default(), tokens);

    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct_toggle").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_none());

    driver.click_widget_id("edit.desc.custom_pct_toggle").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_some());

    for invalid in ["40", "100"] {
        set_textbox(&mut driver, "edit.desc.custom_pct", invalid).unwrap();
    }
    assert!(matches!(
        &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind,
        AnalysisKind::DescriptiveStats(d) if d.custom_upper_percentile == Some(90.0)
    ));

    set_textbox(&mut driver, "edit.desc.custom_pct", "95").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    {
        let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
        assert!(matches!(&rs.analysis.kind, AnalysisKind::DescriptiveStats(d) if d.custom_upper_percentile == Some(95.0)));
        let tab = rs.tabs.iter().find(|t| t.id == "desc-summary").expect("desc-summary tab");
        for lbl in [
            "5% Percentile (Type 7)",
            "95% Percentile (Type 7)",
            "95% CI of Geometric mean",
            "95% CI of Harmonic mean",
            "95% CI of Quadratic mean (RMS)",
        ] {
            assert!(tab.row_labels.iter().any(|l| l == lbl), "missing `{lbl}`");
        }
    }

    let doc2 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc2, &res_id));
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_some());
    driver.click_widget_id("edit.desc.custom_pct_toggle").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_none());
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
    assert!(matches!(&rs.analysis.kind, AnalysisKind::DescriptiveStats(d) if d.custom_upper_percentile.is_none()));
    assert!(!rs.tabs.iter().find(|t| t.id == "desc-summary").unwrap().row_labels.iter().any(|l| l.contains("Type 7")));
}

#[test]
fn analyze_dialog_wires_custom_upper_percentile_into_analysis_run() {
    let (doc, sid) =
        seed_column_doc("Run", "col-run", "Run Data", ColumnDesignHint::UnpairedReplicates, &[("A", &[1.0, 2.0, 3.0, 4.0, 5.0])]).unwrap();
    let mut dlg = AnalyzeDialogState::default();
    dlg.open_for_data_sheet(doc.data_sheet(&sid).unwrap());
    dlg.select_catalog_entry("stats.descriptive");
    let preset = LocalePreset::default();

    let mut bad = Map::new();
    bad.insert(String::from("custom_upper_percentile"), json!(50.0));
    assert!(dialog_patch::patch_analyze(&mut dlg, &bad, &preset).is_err());

    let mut patch = Map::new();
    patch.insert(String::from("custom_upper_percentile"), json!(97.5));
    dialog_patch::patch_analyze(&mut dlg, &patch, &preset).unwrap();

    let intent = dlg.build_create_new_intent().expect("analysis.run intent");
    assert_eq!(intent.command_id, "analysis.run");
    assert_eq!(intent.params.pointer("/kind/DescriptiveStats/custom_upper_percentile").and_then(serde_json::Value::as_f64), Some(97.5));
}
