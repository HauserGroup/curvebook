//! Post-merge Phase 6 integration tests for single-step atomic undo grouping (`FA-SEC13-14`,
//! `FA-SEC14-11`, `FA-SEC15-11`) and live `egui_shell` `Newcombe-Wilson` parity (`FA-SEC13-11`).
#![forbid(unsafe_code)]

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ContingencySpec, ContingencyTable, DataSheet, ProjectDocument, ResultSheet, SheetId, SubcolumnMode,
    TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::control_io::dispatch_gui_action;
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::views::{ViewShardStates, dispatch_custom_view_action};
use curvebook_ui_egui::{KeyModifiers, UiApp, UiInputEvent, ViewportMetrics, dispatch_ui_event, inspect_ui_tree};
use serde_json::json;

type TestRes = Result<(), String>;

fn find_widget(snap: &curvebook_ui_egui::inspect::UiTreeSnapshot, id: &str) -> Option<curvebook_ui_egui::inspect::UiWidgetNode> {
    snap.find_widget(id)
}

#[test]
fn fa_sec13_14_xy_spill_paste_collapses_into_single_undo_entry() -> TestRes {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4).map_err(|e| format!("{e}"))?;
    xy.set_x_cell(0, Cell::number(1.0)).map_err(|e| format!("{e}"))?;
    xy.data_sets[0].cells[0][0] = Cell::number(10.0);
    let ds = DataSheet::new("xy-1", "Assay", TableArchetype::Xy(xy)).map_err(|e| format!("{e}"))?;
    let mut doc = ProjectDocument::new("Atomic X+Y Paste").map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
    doc.active_sheet_id = Some(SheetId::from("xy-1"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    let before_doc = app.document().clone();
    let before_undo = app.session().undo_depth();

    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("2.0\t20.0\n3.0\t30.0");
    app.set_clipboard_adapter(stub);
    app.focus_widget_id = Some(String::from("grid.cell"));
    app.grid.select_x_cell_sync(1, 0);

    let paste_ev = UiInputEvent::KeyDown { key: String::from("v"), modifiers: KeyModifiers { meta: true, ..KeyModifiers::default() } };
    dispatch_ui_event(&mut app, paste_ev, ViewportMetrics::default()).map_err(|e| format!("paste: {e}"))?;

    assert_eq!(app.session().undo_depth(), before_undo + 1, "X+Y spill paste must create exactly 1 undo step");
    let ds_after = app.document().data_sheet(&SheetId::from("xy-1")).ok_or_else(|| String::from("missing xy-1"))?;
    assert_eq!(ds_after.x_or_time_cell(2, 0).and_then(|c| c.value.finite_f64()), Some(3.0));
    assert_eq!(ds_after.cell(CellCoord::new(2, 0, 0)).and_then(|c| c.value.finite_f64()), Some(30.0));

    app.execute_command("edit.undo", json!({})).map_err(|e| format!("undo: {e}"))?;
    assert_eq!(app.document(), &before_doc, "a single edit.undo must revert BOTH X and Y pasted values");
    assert_eq!(app.session().undo_depth(), before_undo);
    Ok(())
}

#[test]
fn fa_sec14_11_power_wizard_create_collapses_into_single_undo_entry() -> TestRes {
    let mut app = UiApp::new();
    let before_doc = app.document().clone();
    let before_undo = app.session().undo_depth();

    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.powerWizard.open" })).map_err(|e| format!("open power wizard: {e}"))?;
    for step in 0..3 {
        let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "next" }))
            .map_err(|e| format!("advance power wizard stage {step}: {e}"))?;
    }
    app.canvas_dialogs.power_wizard.recalculate().map_err(|e| format!("recalc: {e}"))?;
    let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "create" })).map_err(|e| format!("power create: {e}"))?;

    assert_eq!(app.session().undo_depth(), before_undo + 1, "Power Wizard create must push exactly 1 undo step");
    assert!(!app.document().data_sheets.is_empty());
    assert!(!app.document().result_sheets.is_empty());

    app.execute_command("edit.undo", json!({})).map_err(|e| format!("undo: {e}"))?;
    assert_eq!(app.document(), &before_doc, "one edit.undo must revert all Power Wizard Data/Info/Graph/Result sheets");
    assert_eq!(app.session().undo_depth(), before_undo);

    // Direct PowerWizardState::execute_create on Session must also push a single undo entry.
    let mut session = Session::new();
    let base_depth = session.undo_depth();
    let mut wizard = curvebook_ui_egui::power_wizard::PowerWizardState::default();
    wizard.recalculate().map_err(|e| format!("direct recalc: {e}"))?;
    wizard.execute_create(&mut session).map_err(|e| format!("direct create: {e}"))?;
    assert_eq!(session.undo_depth(), base_depth + 1);
    session.undo().map_err(|e| format!("direct undo: {e}"))?;
    assert_eq!(session.document().total_sheet_count(), 0);
    Ok(())
}

#[test]
fn fa_sec15_11_welcome_sample_create_collapses_into_single_undo_entry() -> TestRes {
    let mut app = UiApp::new();
    let before_doc = app.document().clone();
    let before_undo = app.session().undo_depth();

    let mut shards = ViewShardStates::default();
    let _ = dispatch_custom_view_action(&mut app, &mut shards, "welcome", "fixture:dose-response-4pl").map_err(|e| format!("select fixture: {e}"))?;
    let out = dispatch_custom_view_action(&mut app, &mut shards, "welcome", "action:create").map_err(|e| format!("create sample: {e}"))?;
    assert!(out.handled);

    assert_eq!(app.session().undo_depth(), before_undo + 1, "welcome sample instantiation must push 1 undo step");
    assert_eq!(app.document().data_sheets.len(), 1);

    app.execute_command("edit.undo", json!({})).map_err(|e| format!("undo: {e}"))?;
    assert_eq!(app.document(), &before_doc, "one edit.undo must remove the sample sheet and its populated cells/links");
    assert_eq!(app.session().undo_depth(), before_undo);
    Ok(())
}

#[test]
fn fa_sec13_11_live_egui_shell_enables_newcombe_wilson_when_odds_ratio_active() -> TestRes {
    let mut doc = ProjectDocument::new("Live Contingency Parity").map_err(|e| format!("{e}"))?;
    let ds = DataSheet::new("cont-1", "2x2", TableArchetype::Contingency(ContingencyTable::new(2, 2).map_err(|e| format!("{e}"))?))
        .map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
    let cont = ContingencySpec { compute_odds_and_risk_ratios: true, ..ContingencySpec::default() };
    let spec = AnalysisSpec::new("res-cont", "cont-1", AnalysisKind::Contingency(cont)).map_err(|e| format!("{e}"))?;
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.open_edit_parameters(&app.document().clone(), &SheetId::from("res-cont"));

    // Immediately on reopen (`compute_odds_and_risk_ratios=true`), live `egui_shell` and
    // `views/stat_params` must enable Newcombe-Wilson (`FA-SEC13-11` + `edit_params.rs:206`).
    let tree = inspect_ui_tree(&app, Some("dialog"), false);
    let node = find_widget(&tree, "edit.contingency.ci.newcombe_wilson")
        .ok_or_else(|| String::from("missing edit.contingency.ci.newcombe_wilson in live egui dialog tree"))?;
    assert!(node.enabled, "live egui_shell must enable Newcombe-Wilson on reopen when compute_odds_and_risk_ratios=true: {node:?}");

    let mut shards = ViewShardStates::default();
    let cycle = dispatch_custom_view_action(&mut app, &mut shards, "stat_params", "cycle_contingency_ci:arr")
        .map_err(|e| format!("stat_params cycle_contingency_ci:arr: {e}"))?;
    assert!(cycle.handled);

    // Explicitly unchecking `attributable_risk` disables Newcombe-Wilson consistently across both surfaces (`R6`).
    let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "edit.contingency.attributable_risk": false } }))
        .map_err(|e| format!("patch attributable_risk off: {e}"))?;
    let tree_off = inspect_ui_tree(&app, Some("dialog"), false);
    let node_off = find_widget(&tree_off, "edit.contingency.ci.newcombe_wilson")
        .ok_or_else(|| String::from("missing edit.contingency.ci.newcombe_wilson after uncheck"))?;
    assert!(!node_off.enabled, "live egui_shell must disable Newcombe-Wilson when attributable_risk=false: {node_off:?}");
    assert!(
        dispatch_custom_view_action(&mut app, &mut shards, "stat_params", "cycle_contingency_ci:arr").is_err(),
        "stat_params must disable Newcombe-Wilson when attributable_risk=false"
    );
    Ok(())
}
