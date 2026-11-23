//! Regression tests for post-audit delta findings `DA-06-01`, `DA-06-03`,
//! `DA-06-05`, `DA-06-06`, and `DA-06-07`.

use curvebook_doc::SheetId;
use curvebook_ui_egui::control_io::handle_control_line;
use curvebook_ui_egui::control_io::host_routing::{
    contextual_command_params, has_non_null_path_param, is_command_read_only, is_read_only_control_method,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn control_json(app: &mut UiApp, id: u64, method: &str, params: Value) -> Result<Value, String> {
    let line = json!({ "id": id, "method": method, "params": params }).to_string();
    let raw = handle_control_line(app, &line);
    serde_json::from_str(&raw).map_err(|err| format!("parse control response: {err}"))
}

fn assert_gui_only_violation(resp: &Value, context: &str) {
    assert_eq!(resp.get("ok").and_then(Value::as_bool), Some(false), "{context}: expected error, got {resp}");
    let code = resp.pointer("/error_detail/code").or_else(|| resp.pointer("/result/error/code")).and_then(Value::as_str);
    assert_eq!(code, Some("gui_only_violation"), "{context}: expected gui_only_violation, got {resp}");
}

fn setup_column_and_result(app: &mut UiApp) -> Result<String, String> {
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "column", "cols": 2, "rows": 4, "auto_graph": true }))
        .map_err(|err| format!("createData: {err}"))?;
    app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "ds-1",
            "edits": [
                { "row": 0, "col": 0, "value": 10.0 },
                { "row": 1, "col": 0, "value": 12.0 },
                { "row": 2, "col": 0, "value": 14.0 },
                { "row": 0, "col": 1, "value": 20.0 },
                { "row": 1, "col": 1, "value": 22.0 },
                { "row": 2, "col": 1, "value": 25.0 }
            ]
        }),
    )
    .map_err(|err| format!("setCells: {err}"))?;
    let out = app
        .execute_command("analysis.run", json!({ "source_sheet_id": "ds-1", "kind": "descriptive_stats" }))
        .map_err(|err| format!("analysis.run: {err}"))?;
    out.payload
        .get("sheet_id")
        .or_else(|| out.payload.get("result_sheet_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("analysis.run payload missing result sheet id: {}", out.payload))
}

#[test]
fn da_06_01_gui_only_blocks_analysis_update_params_with_empty_or_null_params() -> TestRes {
    let mut app = UiApp::new();
    let rs_id = setup_column_and_result(&mut app)?;
    app.execute_command("sheet.select", json!({ "sheet_id": rs_id })).map_err(|err| format!("select result: {err}"))?;
    app.set_gui_only(true);

    // Direct calls with empty object or null params must fail with GuiOnlyViolation without opening dialog.
    let direct_empty = control_json(&mut app, 1, "analysis.updateParams", json!({}))?;
    assert_gui_only_violation(&direct_empty, "direct analysis.updateParams {}");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let direct_null = control_json(&mut app, 2, "analysis.updateParams", Value::Null)?;
    assert_gui_only_violation(&direct_null, "direct analysis.updateParams null");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // command.run wrapper with empty/null params must also be blocked.
    let run_empty = control_json(&mut app, 3, "command.run", json!({ "command": "analysis.updateParams", "params": {} }))?;
    assert_gui_only_violation(&run_empty, "command.run analysis.updateParams {}");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let run_null = control_json(&mut app, 4, "command.run", json!({ "command_id": "analysis.updateParams", "params": Value::Null }))?;
    assert_gui_only_violation(&run_null, "command.run analysis.updateParams null");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // Atomic and non-atomic command.batch must also be blocked.
    let batch_atomic =
        control_json(&mut app, 5, "command.batch", json!({ "atomic": true, "commands": [{ "command": "analysis.updateParams", "params": {} }] }))?;
    assert_gui_only_violation(&batch_atomic, "atomic command.batch analysis.updateParams {}");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let batch_non_atomic = control_json(
        &mut app,
        6,
        "command.batch",
        json!({ "atomic": false, "commands": [{ "command_id": "analysis.updateParams", "params": Value::Null }] }),
    )?;
    assert_gui_only_violation(&batch_non_atomic, "non-atomic command.batch analysis.updateParams null");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // Legitimate GUI action `ui.invoke_menu` with `analysis.updateParams` still opens the dialog.
    let menu_ok = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "analysis.updateParams" }))
        .map_err(|err| format!("ui.invoke_menu analysis.updateParams should still work under --gui-only: {err}"))?;
    assert_eq!(menu_ok.get("ui").and_then(Value::as_bool), Some(true));
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    Ok(())
}

#[test]
fn da_06_03_read_only_control_gates_disk_writing_exports_and_output_path() {
    // doc.export and render.pdf are conditionally read-only (only when no path param is given).
    assert!(!is_read_only_control_method("doc.export"));
    assert!(!is_read_only_control_method("render.pdf"));
    assert!(is_read_only_control_method("ping"));
    assert!(is_read_only_control_method("doc.inspect"));
    assert!(is_read_only_control_method("render.svg"));
    assert!(is_read_only_control_method("render.preview"));

    // Path detection includes output_path and outputPath and ignores null shadowing.
    assert!(!has_non_null_path_param(&json!({ "format": "csv" })));
    assert!(!has_non_null_path_param(&json!({ "path": Value::Null, "output_path": Value::Null })));
    assert!(has_non_null_path_param(&json!({ "path": "out.csv" })));
    assert!(has_non_null_path_param(&json!({ "file_path": "out.csv" })));
    assert!(has_non_null_path_param(&json!({ "filePath": "out.csv" })));
    assert!(has_non_null_path_param(&json!({ "output_path": "out.csv" })));
    assert!(has_non_null_path_param(&json!({ "outputPath": "out.csv" })));
    assert!(has_non_null_path_param(&json!({ "path": Value::Null, "output_path": "out.csv" })));

    // Pathless doc.export and render.pdf remain read-only; path-bearing variants are rejected.
    assert!(is_command_read_only("doc.export", Some(&json!({ "format": "csv" }))));
    assert!(!is_command_read_only("doc.export", Some(&json!({ "format": "csv", "path": "a.csv" }))));
    assert!(!is_command_read_only("doc.export", Some(&json!({ "format": "csv", "output_path": "a.csv" }))));
    assert!(!is_command_read_only("doc.export", Some(&json!({ "format": "csv", "outputPath": "a.csv" }))));
    assert!(!is_command_read_only("doc.export", Some(&json!({ "format": "csv", "path": Value::Null, "output_path": "a.csv" }))));

    assert!(is_command_read_only("render.pdf", Some(&json!({ "all": false }))));
    assert!(!is_command_read_only("render.pdf", Some(&json!({ "path": "out.pdf" }))));
    assert!(!is_command_read_only("render.pdf", Some(&json!({ "output_path": "out.pdf" }))));

    // Nested command.run and command.batch honor the same path gate.
    assert!(is_command_read_only("command.run", Some(&json!({ "command": "doc.export", "params": { "format": "tsv" } }))));
    assert!(!is_command_read_only("command.run", Some(&json!({ "command": "doc.export", "params": { "format": "tsv", "output_path": "out.tsv" } }))));
    assert!(!is_command_read_only("command.batch", Some(&json!({ "commands": [{ "command": "render.pdf", "params": { "path": "out.pdf" } }] }))));
}

#[test]
fn da_06_05_doc_export_csv_tsv_on_non_tabular_active_sheet_prefers_first_result_sheet() -> TestRes {
    let mut app = UiApp::new();
    let rs_id = setup_column_and_result(&mut app)?;
    let graph_id = app.document().graph_sheets.first().map(|g| g.id.as_str().to_owned()).ok_or_else(|| String::from("expected auto graph sheet"))?;

    // Active sheet is a non-tabular GraphSheet: omitting sheet_id/tab_id must fall back to first ResultSheet.
    app.execute_command("sheet.select", json!({ "sheet_id": graph_id })).map_err(|err| format!("select graph: {err}"))?;
    assert_eq!(app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some(graph_id.as_str()));

    for fmt in ["csv", "tsv"] {
        let resp = control_json(&mut app, 10, "doc.export", json!({ "format": fmt }))?;
        assert_eq!(resp.get("ok").and_then(Value::as_bool), Some(true), "doc.export({fmt}) failed: {resp}");
        assert_eq!(
            resp.pointer("/result/sheet_id").and_then(Value::as_str),
            Some(rs_id.as_str()),
            "doc.export({fmt}) on active GraphSheet must prefer first ResultSheet like headless MCP: {resp}"
        );
    }

    // Active sheet is a DataSheet: omitting sheet_id/tab_id must still export that active DataSheet.
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-1" })).map_err(|err| format!("select ds-1: {err}"))?;
    let ds_resp = control_json(&mut app, 11, "doc.export", json!({ "format": "csv" }))?;
    assert_eq!(ds_resp.get("ok").and_then(Value::as_bool), Some(true), "doc.export(csv) on DataSheet failed: {ds_resp}");
    assert_eq!(ds_resp.pointer("/result/sheet_id").and_then(Value::as_str), Some("ds-1"));

    // Active sheet is a GraphSheet with NO ResultSheets: must still fall back to DataSheet cleanly.
    let mut app_no_res = UiApp::new();
    app_no_res
        .execute_command("sheet.createData", json!({ "sheet_id": "ds-only", "kind": "column", "cols": 1, "rows": 2, "auto_graph": true }))
        .map_err(|err| format!("createData: {err}"))?;
    let g_only = app_no_res.document().graph_sheets.first().map(|g| g.id.as_str().to_owned()).ok_or_else(|| String::from("expected auto graph"))?;
    app_no_res.execute_command("sheet.select", json!({ "sheet_id": g_only })).map_err(|err| format!("select graph: {err}"))?;
    let fallback_resp = control_json(&mut app_no_res, 12, "doc.export", json!({ "format": "csv" }))?;
    assert_eq!(fallback_resp.get("ok").and_then(Value::as_bool), Some(true), "fallback export failed: {fallback_resp}");
    assert_eq!(fallback_resp.pointer("/result/sheet_id").and_then(Value::as_str), Some("ds-only"));
    Ok(())
}

#[test]
fn da_06_06_contextual_params_for_create_checkpoint_uses_name_and_succeeds() -> TestRes {
    let mut app = UiApp::new();
    let params = contextual_command_params(&app, "project.createCheckpoint");
    assert_eq!(params.get("name").and_then(Value::as_str), Some("Checkpoint"));
    assert!(params.get("label").is_none(), "retired `label` key must not be emitted: {params}");

    let out = app.execute_command("project.createCheckpoint", params).map_err(|err| format!("project.createCheckpoint failed: {err}"))?;
    assert!(out.mutated);
    assert_eq!(app.document().checkpoints.len(), 1);
    assert_eq!(app.document().checkpoints.first().map(|c| c.name.as_str()), Some("Checkpoint"));
    Ok(())
}

#[test]
fn da_06_07_manual_fallback_atomic_batch_collapses_into_single_undo_entry() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-base", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false }))
        .map_err(|err| format!("seed data sheet: {err}"))?;

    let initial_undo = app.session().undo_depth();
    let initial_sheets = app.document().data_sheets.len();
    assert_eq!(initial_sheets, 1);

    // Mix a UI command (`ui.rulers.toggle`) with multiple engine mutations so `command.batch(atomic: true)`
    // takes the manual fallback path rather than `try_execute_pure_engine_atomic_batch`.
    let resp = control_json(
        &mut app,
        20,
        "command.batch",
        json!({
            "atomic": true,
            "commands": [
                { "command": "ui.rulers.toggle", "params": {} },
                { "command": "sheet.createData", "params": { "sheet_id": "ds-m1", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false } },
                { "command": "sheet.createData", "params": { "sheet_id": "ds-m2", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false } },
                { "command": "sheet.rename", "params": { "sheet_id": "ds-m2", "title": "Renamed M2" } }
            ]
        }),
    )?;
    assert_eq!(resp.get("ok").and_then(Value::as_bool), Some(true), "mixed atomic batch failed: {resp}");
    assert_eq!(app.document().data_sheets.len(), 3);
    assert_eq!(
        app.session().undo_depth(),
        initial_undo + 1,
        "manual-fallback atomic batch must collapse all 3 document mutations into exactly 1 undo entry"
    );

    // One single edit.undo must revert ALL steps from the batch.
    let undo_resp = control_json(&mut app, 21, "edit.undo", json!({}))?;
    assert_eq!(undo_resp.get("ok").and_then(Value::as_bool), Some(true), "edit.undo failed: {undo_resp}");
    assert_eq!(app.document().data_sheets.len(), initial_sheets, "single undo must revert all sheets created in atomic batch");
    assert!(app.document().data_sheet(&SheetId::new("ds-m1")).is_none());
    assert!(app.document().data_sheet(&SheetId::new("ds-m2")).is_none());
    assert_eq!(app.session().undo_depth(), initial_undo);
    Ok(())
}
