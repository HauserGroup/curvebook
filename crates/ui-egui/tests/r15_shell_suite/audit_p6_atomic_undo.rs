//! Post-merge Phase 6 integration tests for single-step atomic undo grouping (`FA-SEC13-14`,
//! `FA-SEC14-11`, `FA-SEC15-11`) and live `egui_shell` `Newcombe-Wilson` parity (`FA-SEC13-11`).
#![forbid(unsafe_code)]

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ContingencySpec, ContingencyTable, DataSheet, ProjectDocument, ResultSheet, SheetId, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_ui_egui::egui_shell::welcome_hub::build_sample_creation_intent;
use curvebook_ui_egui::egui_shell::{ContingencyCiTarget, PendingUiIntents, StatParamsIntent, UiIntent};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::{KeyModifiers, UiApp, inspect_ui_tree};
use serde_json::json;

use crate::common::{TestRes, cell_number, key_down, seed_xy_doc};
use crate::helpers::{exec, gui};

#[test]
fn fa_sec13_14_xy_spill_paste_collapses_into_single_undo_entry() -> TestRes {
    let (doc, sid) = seed_xy_doc("Atomic X+Y Paste", "xy-1", "Assay", "X", &[1.0, 0.0, 0.0, 0.0], &[("Y", &[10.0])])?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    let (before_doc, before_undo) = (app.document().clone(), app.session().undo_depth());

    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("2.0\t20.0\n3.0\t30.0");
    app.set_clipboard_adapter(stub);
    app.focus_widget_id = Some(String::from("grid.cell"));
    app.grid.select_x_cell_sync(1, 0);

    key_down(&mut app, "v", KeyModifiers { meta: true, ..Default::default() }).map_err(|e| format!("paste: {e}"))?;
    assert_eq!(app.session().undo_depth(), before_undo + 1, "X+Y spill paste must create 1 undo step");
    let ds = app.document().data_sheet(&sid).ok_or("missing xy-1")?;
    assert_eq!(ds.x_or_time_cell(2, 0).and_then(|c| c.value.finite_f64()), Some(3.0));
    assert_eq!(cell_number(ds, 2, 0), Some(30.0));

    exec(&mut app, "edit.undo", json!({}))?;
    assert_eq!(app.document(), &before_doc, "single undo must revert both X and Y");
    assert_eq!(app.session().undo_depth(), before_undo);
    Ok(())
}

#[test]
fn fa_sec14_11_power_wizard_create_collapses_into_single_undo_entry() -> TestRes {
    let mut app = UiApp::new();
    let (before_doc, before_undo) = (app.document().clone(), app.session().undo_depth());

    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.powerWizard.open" }))?;
    for _ in 0..3 {
        gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.next" }))?;
    }
    app.canvas_dialogs.power_wizard.recalculate().map_err(|e| e.to_string())?;
    gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.create" }))?;
    assert_eq!(app.session().undo_depth(), before_undo + 1);
    assert!(!app.document().data_sheets.is_empty() && !app.document().result_sheets.is_empty());

    exec(&mut app, "edit.undo", json!({}))?;
    assert_eq!(app.document(), &before_doc);
    assert_eq!(app.session().undo_depth(), before_undo);

    let mut session = Session::new();
    let base_depth = session.undo_depth();
    let mut wizard = curvebook_ui_egui::power_wizard::PowerWizardState::default();
    wizard.recalculate().map_err(|e| e.to_string())?;
    wizard.execute_create(&mut session).map_err(|e| e.to_string())?;
    assert_eq!(session.undo_depth(), base_depth + 1);
    session.undo().map_err(|e| e.to_string())?;
    assert_eq!(session.document().total_sheet_count(), 0);
    Ok(())
}

#[test]
fn fa_sec15_11_welcome_sample_create_collapses_into_single_undo_entry() -> TestRes {
    let mut app = UiApp::new();
    let (before_doc, before_undo) = (app.document().clone(), app.session().undo_depth());

    let mut intents = PendingUiIntents::new();
    intents.push(build_sample_creation_intent(&app, "dose-response-4pl", None, false, false)?);
    let batch = intents.commit_all_if_settled(&mut app, false);
    assert_eq!(batch.results.len(), 1);
    assert!(batch.results[0].is_ok(), "sample creation failed: {:?}", batch.results);
    assert_eq!(app.session().undo_depth(), before_undo + 1);
    assert_eq!(app.document().data_sheets.len(), 1);

    exec(&mut app, "edit.undo", json!({}))?;
    assert_eq!(app.document(), &before_doc);
    assert_eq!(app.session().undo_depth(), before_undo);
    Ok(())
}

#[test]
fn fa_sec13_11_live_egui_shell_enables_newcombe_wilson_when_odds_ratio_active() -> TestRes {
    let mut doc = ProjectDocument::new("Live Contingency Parity").map_err(|e| e.to_string())?;
    let ds = DataSheet::new("cont-1", "2x2", TableArchetype::Contingency(ContingencyTable::new(2, 2).map_err(|e| e.to_string())?))
        .map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    let cont = ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() };
    let spec = AnalysisSpec::new("res-cont", "cont-1", AnalysisKind::Contingency(cont)).map_err(|e| e.to_string())?;
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.open_edit_parameters(&app.document().clone(), &SheetId::from("res-cont"));

    for (disable_ar, expect_enabled) in [(false, true), (true, false)] {
        if disable_ar {
            app.canvas_dialogs.edit_params.set_contingency_attributable_risk(false);
        }
        let tree = inspect_ui_tree(&app, Some("dialog"), false);
        let node = tree.find_widget("edit.contingency.ci.newcombe_wilson").ok_or("missing edit.contingency.ci.newcombe_wilson")?;
        assert_eq!(node.enabled, expect_enabled, "disable_ar={disable_ar}: {node:?}");
        let mut intents = PendingUiIntents::new();
        intents.push(UiIntent::StatParams(StatParamsIntent::CycleContingencyCi { which: ContingencyCiTarget::AttributableRisk }));
        let batch = intents.commit_all_if_settled(&mut app, false);
        assert_eq!(batch.results.first().is_some_and(Result::is_ok), expect_enabled, "StatParamsIntent disable_ar={disable_ar}");
    }
    Ok(())
}
