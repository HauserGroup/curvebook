//! Regression tests for post-audit delta findings `DA-06-03`, `DA-06-05`, `DA-06-06`, and `DA-06-07`.

use curvebook_doc::SheetId;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::control_io::host_routing::{
    contextual_command_params, has_non_null_path_param, is_command_read_only, is_read_only_control_method,
};
use serde_json::{Value, json};

use crate::common::{TestRes, call_control};
use crate::helpers::exec;

fn control_json(app: &mut UiApp, id: u64, method: &str, params: Value) -> Result<Value, String> {
    call_control(app, &json!({ "id": id, "method": method, "params": params }))
}

#[test]
fn da_06_03_read_only_control_gates_disk_writing_exports_and_output_path() {
    for (method, expected) in
        [("doc.export", false), ("render.pdf", false), ("ping", true), ("doc.inspect", true), ("render.svg", true), ("render.preview", true)]
    {
        assert_eq!(is_read_only_control_method(method), expected, "method={method}");
    }

    for (params, expected) in [
        (json!({ "format": "csv" }), false),
        (json!({ "path": Value::Null, "output_path": Value::Null }), false),
        (json!({ "path": "out.csv" }), true),
        (json!({ "file_path": "out.csv" }), true),
        (json!({ "filePath": "out.csv" }), true),
        (json!({ "output_path": "out.csv" }), true),
        (json!({ "outputPath": "out.csv" }), true),
        (json!({ "path": Value::Null, "output_path": "out.csv" }), true),
    ] {
        assert_eq!(has_non_null_path_param(&params), expected, "params={params}");
    }

    for (cmd, params, expected) in [
        ("doc.export", json!({ "format": "csv" }), true),
        ("doc.export", json!({ "format": "csv", "path": "a.csv" }), false),
        ("doc.export", json!({ "format": "csv", "output_path": "a.csv" }), false),
        ("doc.export", json!({ "format": "csv", "outputPath": "a.csv" }), false),
        ("doc.export", json!({ "format": "csv", "path": Value::Null, "output_path": "a.csv" }), false),
        ("render.pdf", json!({ "all": false }), true),
        ("render.pdf", json!({ "path": "out.pdf" }), false),
        ("render.pdf", json!({ "output_path": "out.pdf" }), false),
        ("command.run", json!({ "command": "doc.export", "params": { "format": "tsv" } }), true),
        ("command.run", json!({ "command": "doc.export", "params": { "format": "tsv", "output_path": "out.tsv" } }), false),
        ("command.batch", json!({ "commands": [{ "command": "render.pdf", "params": { "path": "out.pdf" } }] }), false),
    ] {
        assert_eq!(is_command_read_only(cmd, Some(&params)), expected, "cmd={cmd} params={params}");
    }
}

#[test]
fn da_06_05_doc_export_csv_tsv_on_non_tabular_active_sheet_prefers_first_result_sheet() -> TestRes {
    let mut app = UiApp::new();
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "ds-1", "kind": "column", "cols": 2, "rows": 4, "auto_graph": true }))?;
    exec(&mut app, "table.paste", json!({ "sheet_id": "ds-1", "start_row": 0, "start_col": 0, "text": "10\t20\n12\t22\n14\t25" }))?;
    let out = exec(&mut app, "analysis.run", json!({ "source_sheet_id": "ds-1", "kind": "descriptive_stats" }))?;
    let rs_id = out
        .payload
        .get("sheet_id")
        .or_else(|| out.payload.get("result_sheet_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing result sheet id: {}", out.payload))?;
    let graph_id = app.document().graph_sheets.first().map(|g| g.id.as_str().to_owned()).ok_or("expected auto graph")?;

    // Active GraphSheet with ResultSheet present prefers first ResultSheet; active DataSheet exports itself.
    exec(&mut app, "sheet.select", json!({ "sheet_id": graph_id }))?;
    assert_eq!(app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some(graph_id.as_str()));
    for (id, fmt, target) in [(10, "csv", rs_id.as_str()), (11, "tsv", rs_id.as_str())] {
        let resp = control_json(&mut app, id, "doc.export", json!({ "format": fmt }))?;
        assert_eq!(resp["ok"], json!(true), "doc.export({fmt}): {resp}");
        assert_eq!(resp.pointer("/result/sheet_id").and_then(Value::as_str), Some(target));
    }

    exec(&mut app, "sheet.select", json!({ "sheet_id": "ds-1" }))?;
    let ds_resp = control_json(&mut app, 12, "doc.export", json!({ "format": "csv" }))?;
    assert_eq!(ds_resp["ok"], json!(true));
    assert_eq!(ds_resp.pointer("/result/sheet_id").and_then(Value::as_str), Some("ds-1"));

    // Active GraphSheet with no ResultSheets falls back to DataSheet cleanly.
    let mut app_no_res = UiApp::new();
    exec(&mut app_no_res, "sheet.createData", json!({ "sheet_id": "ds-only", "kind": "column", "cols": 1, "rows": 2, "auto_graph": true }))?;
    let g_only = app_no_res.document().graph_sheets.first().map(|g| g.id.as_str().to_owned()).ok_or("expected auto graph")?;
    exec(&mut app_no_res, "sheet.select", json!({ "sheet_id": g_only }))?;
    let fb = control_json(&mut app_no_res, 13, "doc.export", json!({ "format": "csv" }))?;
    assert_eq!(fb["ok"], json!(true));
    assert_eq!(fb.pointer("/result/sheet_id").and_then(Value::as_str), Some("ds-only"));
    Ok(())
}

#[test]
fn da_06_06_contextual_params_for_create_checkpoint_uses_name_and_succeeds() -> TestRes {
    let mut app = UiApp::new();
    let params = contextual_command_params(&app, "project.createCheckpoint");
    assert_eq!(params.get("name").and_then(Value::as_str), Some("Checkpoint"));
    assert!(params.get("label").is_none(), "retired `label` key must not be emitted: {params}");
    let out = exec(&mut app, "project.createCheckpoint", params)?;
    assert!(out.mutated);
    assert_eq!(app.document().checkpoints.len(), 1);
    assert_eq!(app.document().checkpoints[0].name, "Checkpoint");
    Ok(())
}

#[test]
fn da_06_07_manual_fallback_atomic_batch_collapses_into_single_undo_entry() -> TestRes {
    let mut app = UiApp::new();
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "ds-base", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false }))?;
    let (initial_undo, initial_sheets) = (app.session().undo_depth(), app.document().data_sheets.len());
    assert_eq!(initial_sheets, 1);

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
    assert_eq!(resp["ok"], json!(true), "mixed atomic batch failed: {resp}");
    assert_eq!(app.document().data_sheets.len(), 3);
    assert_eq!(app.session().undo_depth(), initial_undo + 1);

    let undo_resp = control_json(&mut app, 21, "edit.undo", json!({}))?;
    assert_eq!(undo_resp["ok"], json!(true));
    assert_eq!(app.document().data_sheets.len(), initial_sheets);
    assert!(app.document().data_sheet(&SheetId::new("ds-m1")).is_none());
    assert!(app.document().data_sheet(&SheetId::new("ds-m2")).is_none());
    assert_eq!(app.session().undo_depth(), initial_undo);
    Ok(())
}
