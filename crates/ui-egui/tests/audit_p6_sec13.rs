//! Regression suite for P6 UI-core audit findings (`FA-SEC13-02`..`14` and `FA-SEC04-04`).
#![forbid(unsafe_code)]

use std::io::{BufReader, Cursor};
use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, CellValue, ContingencySpec, ContingencyTable, DataSheet, ErrorCode, ErrorPropagationMode, GlmFamily,
    GlmSpec, InfoSheet, MultivariateSpec, NonlinearFitSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
    SheetId, SourceCellId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XyTable,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::control_io::{dispatch_gui_action, handle_control_line, serve_control_stream_with_quit_and_token};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::modal_predicates::{evaluate_dialog_button_for_app, evaluate_dialog_widget};
use curvebook_ui_egui::os_integration::{ClipboardBundle, OsAdapterStatus, PlatformClipboardAdapter, StubPlatformClipboardAdapter};
use curvebook_ui_egui::workspace_nav::WorkspaceNavState;
use curvebook_ui_egui::{KeyModifiers, UiApp, UiInputEvent, ViewportMetrics, dispatch_ui_event, inspect_ui_tree};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn call_control(app: &mut UiApp, req: &Value) -> Result<Value, String> {
    let line = serde_json::to_string(req).map_err(|e| format!("serialize request: {e}"))?;
    let resp_line = handle_control_line(app, &line);
    serde_json::from_str(&resp_line).map_err(|e| format!("parse control response `{resp_line}`: {e}"))
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>, String> {
    let clean: Vec<u8> = encoded.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        return Err(format!("invalid base64 length {}", clean.len()));
    }
    let val_of = |b: u8| -> Result<u8, String> {
        match b {
            b'A'..=b'Z' => Ok(b - b'A'),
            b'a'..=b'z' => Ok(b - b'a' + 26),
            b'0'..=b'9' => Ok(b - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 byte {b}")),
        }
    };
    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    for chunk in clean.as_chunks::<4>().0 {
        let a = val_of(chunk[0])?;
        let b = val_of(chunk[1])?;
        let pad1 = chunk[2] == b'=';
        let pad2 = chunk[3] == b'=';
        let c = if pad1 { 0 } else { val_of(chunk[2])? };
        let d = if pad2 { 0 } else { val_of(chunk[3])? };
        out.push((a << 2) | (b >> 4));
        if !pad1 {
            out.push((b << 4) | (c >> 2));
        }
        if !pad2 {
            out.push((c << 6) | d);
        }
    }
    Ok(out)
}

fn build_xy_app() -> Result<(UiApp, SheetId, SheetId), String> {
    let mut app = UiApp::new();
    let created = app
        .execute_command(
            "sheet.createData",
            json!({ "sheet_id": "ds-xy", "title": "Assay XY", "kind": "xy", "cols": 2, "rows": 4, "auto_graph": true, "graph_sheet_id": "gs-xy" }),
        )
        .map_err(|e| format!("create xy sheet: {e}"))?;
    let ds_id =
        SheetId::try_new(created.payload["sheet_id"].as_str().ok_or_else(|| String::from("missing sheet_id"))?).map_err(|e| format!("{e}"))?;
    let gs_id = SheetId::try_new(
        created.payload["auto_graph_sheet_id"]
            .as_str()
            .or_else(|| app.document().graph_sheets.first().map(|g| g.id.as_str()))
            .ok_or_else(|| String::from("missing auto graph sheet"))?,
    )
    .map_err(|e| format!("{e}"))?;
    app.execute_command("table.setXCells", json!({ "sheet_id": ds_id.as_str(), "start_row": 0, "values": [1.0, 2.0, 3.0, 4.0] }))
        .map_err(|e| format!("seed x: {e}"))?;
    app.execute_command(
        "table.paste",
        json!({ "sheet_id": ds_id.as_str(), "start_row": 0, "start_col": 0, "text": "10\t15\n20\t25\n30\t35\n40\t45" }),
    )
    .map_err(|e| format!("seed y: {e}"))?;
    app.sync_views();
    Ok((app, ds_id, gs_id))
}

fn build_result_app(status: ResultStatus) -> Result<(UiApp, SheetId, SheetId), String> {
    let (mut app, ds_id, _gs_id) = build_xy_app()?;
    let mut spec = AnalysisSpec::new("res-1", ds_id.as_str(), AnalysisKind::OneWayAnova(Default::default())).map_err(|e| format!("{e}"))?;
    spec.custom_title = Some(String::from("+CustomTitle"));
    let mut rs = ResultSheet::new(spec).map_err(|e| format!("{e}"))?;
    rs.status = status;
    rs.tabs.push(ResultTab {
        id: String::from("tab-1"),
        title: String::from("Summary"),
        kind: ResultTabKind::AnovaTable,
        column_headers: vec![String::from("=Header"), String::from("Value")],
        row_labels: vec![String::from("Row1")],
        rows: vec![vec![ResultCell::Text(String::from("=1+1")), ResultCell::Number { value: 42.5, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    let mut doc = app.document().clone();
    doc.add_result_sheet(rs).map_err(|e| format!("{e}"))?;
    doc.active_sheet_id = Some(SheetId::from("res-1"));
    app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    Ok((app, ds_id, SheetId::from("res-1")))
}

fn find_widget_in_tree(tree: &curvebook_ui_egui::inspect::UiTreeSnapshot, wid: &str) -> Option<curvebook_ui_egui::inspect::UiWidgetNode> {
    tree.find_widget(wid)
}

fn key_down(app: &mut UiApp, key: &str, modifiers: KeyModifiers) -> Result<curvebook_ui_egui::interaction::UiEventOutcome, String> {
    dispatch_ui_event(app, UiInputEvent::KeyDown { key: key.to_owned(), modifiers }, ViewportMetrics::default())
}

fn text_input(app: &mut UiApp, text: &str) -> Result<curvebook_ui_egui::interaction::UiEventOutcome, String> {
    dispatch_ui_event(app, UiInputEvent::TextInput { text: text.to_owned() }, ViewportMetrics::default())
}

#[derive(Debug)]
struct FailingWriteClipboardAdapter;

impl PlatformClipboardAdapter for FailingWriteClipboardAdapter {
    fn write_text(&self, _text: &str) -> Result<OsAdapterStatus, String> {
        Err(String::from("simulated clipboard backend write failure"))
    }

    fn write_bundle(&self, _bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        Err(String::from("simulated clipboard backend write failure"))
    }
}

#[test]
fn fa_sec04_04_doc_export_csv_propagates_failed_result_and_neutralizes_formulas() -> TestRes {
    let (mut ok_app, _ds_id, rs_id) = build_result_app(ResultStatus::UpToDate)?;

    let tidy_resp = call_control(
        &mut ok_app,
        &json!({ "id": "tidy-ok", "method": "doc.export", "params": { "format": "csv", "sheet_id": rs_id.as_str(), "tidy": true } }),
    )?;
    if tidy_resp["ok"] != json!(true) {
        return Err(format!("expected tidy CSV export success, got {tidy_resp}"));
    }
    let tidy_b64 = tidy_resp["result"]["data_base64"].as_str().ok_or_else(|| format!("missing tidy data_base64: {tidy_resp}"))?;
    let tidy_csv = String::from_utf8(decode_base64(tidy_b64)?).map_err(|e| format!("utf8 tidy csv: {e}"))?;
    if !tidy_csv.contains("'=1+1") || !tidy_csv.contains("'+CustomTitle") {
        return Err(format!("expected formula neutralization in tidy CSV, got:\n{tidy_csv}"));
    }

    let tab_resp = call_control(
        &mut ok_app,
        &json!({ "id": "tab-ok", "method": "doc.export", "params": { "format": "csv", "sheet_id": rs_id.as_str(), "tab_id": "tab-1" } }),
    )?;
    if tab_resp["ok"] != json!(true) {
        return Err(format!("expected tab CSV export success, got {tab_resp}"));
    }
    let tab_b64 = tab_resp["result"]["data_base64"].as_str().ok_or_else(|| format!("missing tab data_base64: {tab_resp}"))?;
    let tab_csv = String::from_utf8(decode_base64(tab_b64)?).map_err(|e| format!("utf8 tab csv: {e}"))?;
    if !tab_csv.contains("'=Header") || !tab_csv.contains("'=1+1") {
        return Err(format!("expected formula neutralization in tab CSV, got:\n{tab_csv}"));
    }

    let (mut failed_app, _ds_id2, rs_failed_id) =
        build_result_app(ResultStatus::Failed { code: ErrorCode::InsufficientData, message: String::from("need >= 2 groups") })?;

    let tidy_fail = call_control(
        &mut failed_app,
        &json!({ "id": "tidy-fail", "method": "doc.export", "params": { "format": "csv", "sheet_id": rs_failed_id.as_str(), "tidy": true } }),
    )?;
    if tidy_fail["ok"] != json!(false) {
        return Err(format!("failed result tidy CSV export must error, got {tidy_fail}"));
    }
    let tidy_err = tidy_fail["error"].as_str().unwrap_or_default();
    if !tidy_err.contains("CSV export failed") || !tidy_err.contains("need >= 2 groups") {
        return Err(format!("unexpected tidy failure error message: {tidy_fail}"));
    }

    let tab_fail = call_control(
        &mut failed_app,
        &json!({ "id": "tab-fail", "method": "doc.export", "params": { "format": "csv", "sheet_id": rs_failed_id.as_str(), "tab_id": "tab-1" } }),
    )?;
    if tab_fail["ok"] != json!(false) {
        return Err(format!("failed result tab CSV export must error, got {tab_fail}"));
    }
    let tab_err = tab_fail["error"].as_str().unwrap_or_default();
    if !tab_err.contains("CSV export failed") || !tab_err.contains("need >= 2 groups") {
        return Err(format!("unexpected tab failure error message: {tab_fail}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_02_atomic_batch_rolls_back_beyond_64_undo_entries() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-seed", "title": "Seed", "kind": "xy", "cols": 1, "rows": 2, "auto_graph": false }),
    )
    .map_err(|e| format!("create seed sheet: {e}"))?;
    let baseline_doc = app.document().clone();
    let baseline_undo = app.session().undo_depth();
    let baseline_redo = app.session().redo_depth();

    let mut commands = Vec::with_capacity(72);
    for idx in 0..70usize {
        commands.push(json!({
            "command": "sheet.createData",
            "params": { "sheet_id": format!("tmp-{idx}"), "title": format!("Tmp {idx}"), "kind": "column", "cols": 1, "rows": 2, "auto_graph": false }
        }));
    }
    commands.push(json!({ "command": "nonexistent.command", "params": {} }));

    let resp = call_control(&mut app, &json!({ "id": "batch-70", "method": "command.batch", "params": { "atomic": true, "commands": commands } }))?;
    if resp["ok"] != json!(false) {
        return Err(String::from("expected failing >64-step atomic batch to return error"));
    }
    if app.document() != &baseline_doc {
        return Err(format!("document was not fully rolled back across >64 steps: sheet count {}", app.document().data_sheets.len()));
    }
    if app.session().undo_depth() != baseline_undo || app.session().redo_depth() != baseline_redo {
        return Err(format!(
            "undo/redo drifted from ({baseline_undo}, {baseline_redo}) to ({}, {})",
            app.session().undo_depth(),
            app.session().redo_depth()
        ));
    }

    // Also verify a batch containing edit.undo captures pre-batch fallback baseline so partial edits still roll back.
    let mix_resp = call_control(
        &mut app,
        &json!({
            "id": "batch-undo-mix",
            "method": "command.batch",
            "params": {
                "atomic": true,
                "commands": [
                    { "command": "sheet.createData", "params": { "sheet_id": "tmp-a", "title": "TmpA", "auto_graph": false } },
                    { "command": "edit.undo", "params": {} },
                    { "command": "sheet.createData", "params": { "sheet_id": "tmp-b", "title": "TmpB", "auto_graph": false } },
                    { "command": "nonexistent.command", "params": {} }
                ]
            }
        }),
    )?;
    if mix_resp["ok"] != json!(false) || app.document() != &baseline_doc {
        return Err(format!("mixed edit.undo batch did not roll back cleanly: {mix_resp}"));
    }
    if app.session().undo_depth() != baseline_undo || app.session().redo_depth() != baseline_redo {
        return Err(format!("mixed edit.undo batch leaked undo/redo state: ({}, {})", app.session().undo_depth(), app.session().redo_depth()));
    }
    Ok(())
}

#[test]
fn fa_sec13_03_unifies_egui_dispatch_under_control_io_auth_panic_and_batch() -> TestRes {
    let mut app = UiApp::new();
    app.ui_backend = UiBackend::Egui;

    // Compatibility shim must return None so all control requests pass through control_io.
    if app.try_dispatch_egui_backend("ui.inspect", &json!({})).is_some() {
        return Err(String::from("try_dispatch_egui_backend compatibility shim must return None"));
    }

    // Install panicking egui hook and verify catch_unwind + auth enforcement via serve_control_stream_with_quit_and_token.
    app.set_egui_dispatch_hook(Arc::new(|_app, method, _params| {
        if method == "ui.inspect" {
            panic!("simulated egui hook panic");
        }
        None
    }));

    let stream_payload = concat!(
        "{\"id\":\"bad-tok\",\"method\":\"ui.inspect\",\"token\":\"wrong-token\",\"params\":{\"metrics\":{\"width_pt\":800,\"height_pt\":600}}}\n",
        "{\"id\":\"panic-ok-tok\",\"method\":\"ui.inspect\",\"token\":\"good-tok\",\"params\":{\"metrics\":{\"width_pt\":800,\"height_pt\":600}}}\n",
        "{\"id\":\"after-panic\",\"method\":\"ping\",\"token\":\"good-tok\"}\n"
    );
    let mut reader = BufReader::new(Cursor::new(stream_payload.as_bytes()));
    let mut out = Vec::new();
    let _quit = serve_control_stream_with_quit_and_token(&mut app, &mut reader, &mut out, "good-tok").map_err(|e| format!("stream error: {e}"))?;
    let out_text = String::from_utf8(out).map_err(|e| format!("utf8 output: {e}"))?;
    let lines: Vec<&str> = out_text.lines().collect();
    if lines.len() != 3 {
        return Err(format!("expected 3 control response lines, got {lines:?}"));
    }

    let bad_tok: Value = serde_json::from_str(lines[0]).map_err(|e| format!("{e}"))?;
    if bad_tok["ok"] != json!(false) || bad_tok.pointer("/error_detail/code").and_then(Value::as_str) != Some("unauthorized") {
        return Err(format!("expected unauthorized error for bad token on ui.inspect, got {bad_tok}"));
    }

    let panic_resp: Value = serde_json::from_str(lines[1]).map_err(|e| format!("{e}"))?;
    if panic_resp["ok"] != json!(false) || panic_resp.pointer("/error_detail/code").and_then(Value::as_str) != Some("internal_panic") {
        return Err(format!("expected caught internal_panic error from egui hook, got {panic_resp}"));
    }

    let ping_resp: Value = serde_json::from_str(lines[2]).map_err(|e| format!("{e}"))?;
    if ping_resp["ok"] != json!(true) {
        return Err(format!("server should remain responsive after caught hook panic: {ping_resp}"));
    }

    // Without hook, verify ui.inspect validates metrics bounds and runs inside command.batch under UiBackend::Egui.
    let mut clean_app = UiApp::new();
    clean_app.ui_backend = UiBackend::Egui;
    let invalid_metrics = call_control(
        &mut clean_app,
        &json!({ "id": "bad-dim", "method": "ui.inspect", "params": { "metrics": { "width_pt": 64, "height_pt": 64 } } }),
    )?;
    if invalid_metrics["ok"] != json!(false) {
        return Err(format!("expected ui.inspect to reject out-of-bounds width_pt=64, got {invalid_metrics}"));
    }

    let batch_resp = call_control(
        &mut clean_app,
        &json!({
            "id": "batch-inspect",
            "method": "command.batch",
            "params": {
                "atomic": true,
                "commands": [
                    { "command": "ui.inspect", "params": { "metrics": { "width_pt": 1024, "height_pt": 640 } } }
                ]
            }
        }),
    )?;
    if batch_resp["ok"] != json!(true) {
        return Err(format!("expected ui.inspect inside command.batch to succeed, got {batch_resp}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_04_copy_methods_report_respects_human_path_stub_and_errors() -> TestRes {
    let (mut app, _ds_id, rs_id) = build_result_app(ResultStatus::UpToDate)?;
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(stub.clone());
    app.set_human_path_only(true);

    let skip_resp = call_control(
        &mut app,
        &json!({ "id": "m1", "method": "ui.result.copyMethodsReport", "params": { "result_sheet_id": rs_id.as_str(), "skip_os_clipboard": true } }),
    )?;
    if skip_resp["ok"] != json!(true) || !stub.captured_texts().is_empty() {
        return Err(format!("skip_os_clipboard=true should not write clipboard stub: {skip_resp}"));
    }

    let copy_resp =
        call_control(&mut app, &json!({ "id": "m2", "method": "ui.result.copyMethodsReport", "params": { "result_sheet_id": rs_id.as_str() } }))?;
    if copy_resp["ok"] != json!(true) {
        return Err(format!("expected human_path_only + custom stub copyMethodsReport success, got {copy_resp}"));
    }
    if stub.captured_texts().len() != 1 {
        return Err(format!("expected stub clipboard adapter to receive methods report under human_path_only, got {:?}", stub.captured_texts()));
    }

    app.set_clipboard_adapter(Arc::new(FailingWriteClipboardAdapter));
    let err_resp =
        call_control(&mut app, &json!({ "id": "m3", "method": "ui.result.copyMethodsReport", "params": { "result_sheet_id": rs_id.as_str() } }))?;
    if err_resp["ok"] != json!(false) {
        return Err(format!("failing clipboard write must propagate error, got {err_resp}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_05_workspace_nav_click_is_immutable_and_emits_sheet_select() -> TestRes {
    let mut doc = ProjectDocument::new("Nav Test").map_err(|e| format!("{e}"))?;
    let ds1 = DataSheet::new("ds-1", "First", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).map_err(|e| format!("{e}"))?))
        .map_err(|e| format!("{e}"))?;
    let ds2 = DataSheet::new("ds-2", "Second", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).map_err(|e| format!("{e}"))?))
        .map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds1).map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds2).map_err(|e| format!("{e}"))?;
    doc.active_sheet_id = Some(SheetId::from("ds-1"));
    let doc_ref: &ProjectDocument = &doc;

    let mut nav = WorkspaceNavState::default();
    nav.click_navigator_entry(doc_ref, &SheetId::from("ds-2")).map_err(|e| format!("click_navigator_entry failed: {e}"))?;
    nav.click_family_entry(doc_ref, &SheetId::from("ds-1"), false).map_err(|e| format!("click_family_entry failed: {e}"))?;
    if nav.click_navigator_entry(doc_ref, &SheetId::from("missing-sheet")).is_ok() {
        return Err(String::from("click_navigator_entry must reject nonexistent sheet"));
    }
    if doc.active_sheet_id.as_ref().map(SheetId::as_str) != Some("ds-1") {
        return Err(String::from("immutable click helpers must not mutate doc.active_sheet_id directly"));
    }
    let intents = std::mem::take(&mut nav.pending_intents);
    if intents.len() != 2 || intents[0].command_id != "sheet.select" || intents[1].command_id != "sheet.select" {
        return Err(format!("expected 2 queued sheet.select intents, got {intents:?}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_06_multiline_modal_enter_inserts_newline_and_cmd_enter_submits() -> TestRes {
    let (mut app, ds_id, _gs_id) = build_xy_app()?;

    // 1. Checkpoints multiline note: plain Enter inserts newline; Cmd+Enter creates checkpoint with multiline note.
    app.canvas_dialogs.open_checkpoints(&app.document().clone());
    app.focus_widget_id = Some(String::from("checkpoints.note"));

    let _ = text_input(&mut app, "Line 1")?;
    let enter_out = key_down(&mut app, "Enter", KeyModifiers::default())?;
    if !enter_out.handled || app.canvas_dialogs.active_dialog != ActiveDialog::Checkpoints {
        return Err(format!("plain Enter in multiline checkpoints.note must insert newline without closing modal: out={enter_out:?}"));
    }
    let _ = text_input(&mut app, "Line 2")?;
    if app.canvas_dialogs.checkpoints.note_draft != "Line 1\nLine 2" {
        return Err(format!("expected multiline note_draft, got {:?}", app.canvas_dialogs.checkpoints.note_draft));
    }

    let submit_out = key_down(&mut app, "Enter", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    if !submit_out.handled {
        return Err(format!("Cmd+Enter must submit checkpoints modal: out={submit_out:?}"));
    }
    let saved_note = app.document().checkpoints.last().map(|cp| cp.note.as_str());
    if saved_note != Some("Line 1\nLine 2") {
        return Err(format!("committed checkpoint note lost newline: {saved_note:?}"));
    }
    app.canvas_dialogs.close_dialog();

    // 2. ExcludeWithReason multiline note: plain Enter inserts newline keeping modal open; Cmd+Enter submits & closes modal.
    app.canvas_dialogs.open_exclude_with_reason(ds_id.clone(), vec![GridCellCoord::new(0, 0, 0)]);
    app.focus_widget_id = Some(String::from("exclude.note"));
    let _ = text_input(&mut app, "Reason A")?;
    let excl_enter = key_down(&mut app, "Enter", KeyModifiers::default())?;
    if !excl_enter.handled || !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }) {
        return Err(format!("plain Enter in exclude.note must not close modal: {excl_enter:?}"));
    }
    let _ = text_input(&mut app, "Reason B")?;
    if app.canvas_dialogs.exclude_with_reason.note != "Reason A\nReason B" {
        return Err(format!("expected multiline exclude note, got {:?}", app.canvas_dialogs.exclude_with_reason.note));
    }
    let excl_cmd_enter = key_down(&mut app, "Enter", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    if !excl_cmd_enter.handled || app.canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(format!(
            "Cmd+Enter must submit & close ExcludeWithReason: out={excl_cmd_enter:?}, active={:?}",
            app.canvas_dialogs.active_dialog
        ));
    }
    let ds = app.document().data_sheet(&ds_id).ok_or_else(|| String::from("missing ds"))?;
    if ds.cell(CellCoord::new(0, 0, 0)).is_none_or(|c| !c.excluded) {
        return Err(String::from("expected cell (0, 0, 0) to be excluded after Cmd+Enter"));
    }

    // 3. Single-line modal textbox still submits on plain Enter.
    app.canvas_dialogs.open_command_palette();
    app.focus_widget_id = Some(String::from("palette.query"));
    let _ = text_input(&mut app, "ui.split.toggle")?;
    let single_enter = key_down(&mut app, "Enter", KeyModifiers::default())?;
    if !single_enter.handled || app.canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(format!("single-line palette.query plain Enter must confirm dialog: {single_enter:?}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_07_textbox_copy_cut_update_stub_clipboard_over_stale_text() -> TestRes {
    let mut app = UiApp::new();
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("stale_os_clipboard");
    app.set_clipboard_adapter(stub.clone());

    app.canvas_dialogs.open_command_palette();
    app.focus_widget_id = Some(String::from("palette.query"));
    let _ = text_input(&mut app, "fresh_copy")?;
    let cmd = KeyModifiers { meta: true, ..KeyModifiers::default() };
    let _ = key_down(&mut app, "a", cmd)?;
    let copy_out = key_down(&mut app, "c", cmd)?;
    if !copy_out.handled || stub.last_text().as_deref() != Some("fresh_copy") {
        return Err(format!("textbox Cmd+C must update custom clipboard adapter: out={copy_out:?}, stub={:?}", stub.last_text()));
    }

    app.canvas_dialogs.close_dialog();
    app.canvas_dialogs.open_checkpoints(&app.document().clone());
    app.focus_widget_id = Some(String::from("checkpoints.name"));
    let _ = key_down(&mut app, "a", cmd)?;
    let paste_out = key_down(&mut app, "v", cmd)?;
    if !paste_out.handled || app.canvas_dialogs.checkpoints.name_draft != "fresh_copy" {
        return Err(format!("textbox Cmd+V must paste copied text rather than stale text, got {:?}", app.canvas_dialogs.checkpoints.name_draft));
    }

    // Verify Cmd+X cuts selection, updates clipboard adapter, and syncs widget state.
    let _ = key_down(&mut app, "a", cmd)?;
    let cut_out = key_down(&mut app, "x", cmd)?;
    if !cut_out.handled || !app.canvas_dialogs.checkpoints.name_draft.is_empty() {
        return Err(format!("textbox Cmd+X must cut selection and sync dialog state, got {:?}", app.canvas_dialogs.checkpoints.name_draft));
    }
    app.focus_widget_id = Some(String::from("checkpoints.note"));
    let _ = key_down(&mut app, "v", cmd)?;
    if app.canvas_dialogs.checkpoints.note_draft != "fresh_copy" {
        return Err(format!("expected pasted cut text in checkpoints.note, got {:?}", app.canvas_dialogs.checkpoints.note_draft));
    }
    Ok(())
}

#[test]
fn fa_sec13_08_cmd_v_on_active_x_cell_pastes_x_and_spills_y() -> TestRes {
    let (mut app, ds_id, _gs_id) = build_xy_app()?;
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("11.5\t110.0\n22.5\t220.0");
    app.set_clipboard_adapter(stub);
    app.focus_widget_id = Some(String::from("grid.cell"));
    app.grid.select_x_cell_sync(0, 0);

    let out = key_down(&mut app, "v", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    if !out.handled {
        return Err(format!("Cmd+V on active X cell must be handled, got {out:?}"));
    }
    let ds = app.document().data_sheet(&ds_id).ok_or_else(|| String::from("missing datasheet"))?;
    let x0 = ds.x_or_time_cell(0, 0).map(|c| c.value.clone());
    let x1 = ds.x_or_time_cell(1, 0).map(|c| c.value.clone());
    let y0 = ds.cell(CellCoord { row: 0, col: 0, subcol: 0 }).map(|c| c.value.clone());
    let y1 = ds.cell(CellCoord { row: 1, col: 0, subcol: 0 }).map(|c| c.value.clone());
    if !matches!(x0, Some(CellValue::Number(v)) if (v - 11.5).abs() < 1e-9)
        || !matches!(x1, Some(CellValue::Number(v)) if (v - 22.5).abs() < 1e-9)
        || !matches!(y0, Some(CellValue::Number(v)) if (v - 110.0).abs() < 1e-9)
        || !matches!(y1, Some(CellValue::Number(v)) if (v - 220.0).abs() < 1e-9)
    {
        return Err(format!("X+Y paste mismatch: x0={x0:?}, x1={x1:?}, y0={y0:?}, y1={y1:?}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_09_all_textbox_nodes_accept_typing_and_sync_including_info_notes() -> TestRes {
    let (mut app, ds_id, gs_id) = build_xy_app()?;
    let mut doc = app.document().clone();
    let mut info = InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| format!("{e}"))?;
    info.notes_markdown = String::from("Initial");
    doc.add_info_sheet(info).map_err(|e| format!("{e}"))?;

    let spec_fit = AnalysisSpec::new("res-fit", ds_id.as_str(), AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("hyperbolic")))
        .map_err(|e| format!("{e}"))?;
    doc.add_result_sheet(ResultSheet::new(spec_fit).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;

    let spec_tr = AnalysisSpec::new(
        "res-tr",
        ds_id.as_str(),
        AnalysisKind::Transform(TransformAnalysisSpec::Math {
            x_transform: None,
            y_transform: None,
            error_propagation: ErrorPropagationMode::default(),
        }),
    )
    .map_err(|e| format!("{e}"))?;
    doc.add_result_sheet(ResultSheet::new(spec_tr).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;

    let spec_glm = AnalysisSpec::new(
        "res-glm",
        ds_id.as_str(),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )
    .map_err(|e| format!("{e}"))?;
    doc.add_result_sheet(ResultSheet::new(spec_glm).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;

    let spec_mi = AnalysisSpec::new(
        "res-mi",
        ds_id.as_str(),
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            response_column: Some(0),
            predictor_columns: vec![1],
            imputations_m: 5,
            max_iters: 10,
            seed: 42,
        }),
    )
    .map_err(|e| format!("{e}"))?;
    doc.add_result_sheet(ResultSheet::new(spec_mi).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;

    app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();

    type DialogOpener<'a> = (&'a str, Box<dyn Fn(&mut UiApp)>);
    // Open each inspectable dialog (and variant configurations) and verify emitted editable `textbox`
    // controls accept TextInput (skipping hover-only stepper `.val` labels).
    let openers: Vec<DialogOpener<'_>> = vec![
        ("PowerAndSampleSize", Box::new(|a| a.canvas_dialogs.open_power_wizard())),
        (
            "FormatSelectedPoints",
            Box::new({
                let gs = gs_id.clone();
                move |a| {
                    let d = a.document().clone();
                    a.canvas_dialogs.open_format_selected_points(&d, &gs, vec![(0, 0)]);
                }
            }),
        ),
        (
            "AnalyzeSheet:fit",
            Box::new({
                let ds = ds_id.clone();
                move |a| {
                    let d = a.document().clone();
                    a.canvas_dialogs.open_analyze_dialog(&d, &ds);
                    a.canvas_dialogs.analyze.selected_entry_id = Some(String::from("nonlinear_regression"));
                }
            }),
        ),
        (
            "AnalyzeSheet:transform",
            Box::new({
                let ds = ds_id.clone();
                move |a| {
                    let d = a.document().clone();
                    a.canvas_dialogs.open_analyze_dialog(&d, &ds);
                    a.canvas_dialogs.analyze.selected_entry_id = Some(String::from("transform_data"));
                }
            }),
        ),
        (
            "EditAnalysisParameters:fit",
            Box::new(|a| {
                let d = a.document().clone();
                a.canvas_dialogs.open_edit_parameters(&d, &SheetId::from("res-fit"));
            }),
        ),
        (
            "EditAnalysisParameters:transform",
            Box::new(|a| {
                let d = a.document().clone();
                a.canvas_dialogs.open_edit_parameters(&d, &SheetId::from("res-tr"));
            }),
        ),
        (
            "EditAnalysisParameters:glm",
            Box::new(|a| {
                let d = a.document().clone();
                a.canvas_dialogs.open_edit_parameters(&d, &SheetId::from("res-glm"));
            }),
        ),
        ("ImportReshape", Box::new(|a| a.canvas_dialogs.open_import_reshape())),
        (
            "FormatAxes",
            Box::new({
                let gs = gs_id.clone();
                move |a| {
                    a.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gs.clone() };
                }
            }),
        ),
        (
            "CreateGraph",
            Box::new({
                let ds = ds_id.clone();
                move |a| {
                    let d = a.document().clone();
                    a.canvas_dialogs.open_create_graph(&d, Some(&ds));
                }
            }),
        ),
        (
            "RandomizeGroups",
            Box::new(|a| {
                let d = a.document().clone();
                a.canvas_dialogs.open_randomize_groups(&d);
            }),
        ),
        (
            "RandomSampleRows",
            Box::new({
                let ds = ds_id.clone();
                move |a| {
                    let d = a.document().clone();
                    a.canvas_dialogs.open_random_sample_rows(&d, Some(&ds));
                }
            }),
        ),
        ("SimulateDistributions", Box::new(|a| a.canvas_dialogs.open_simulate_distributions())),
        (
            "ChangeDataAnalyzed",
            Box::new(|a| {
                let d = a.document().clone();
                a.canvas_dialogs.open_change_data_analyzed(&d, &SheetId::from("res-fit"));
            }),
        ),
        (
            "ConstantHookPicker",
            Box::new({
                let ds = ds_id.clone();
                move |a| {
                    let d = a.document().clone();
                    a.canvas_dialogs.open_hook_picker(&d, &ds, None);
                }
            }),
        ),
        (
            "ExcludeWithReason",
            Box::new({
                let ds = ds_id.clone();
                move |a| {
                    a.canvas_dialogs.open_exclude_with_reason(ds.clone(), vec![GridCellCoord::new(0, 0, 0)]);
                }
            }),
        ),
        ("CommandPalette", Box::new(|a| a.canvas_dialogs.open_command_palette())),
        (
            "Checkpoints",
            Box::new(|a| {
                let d = a.document().clone();
                a.canvas_dialogs.open_checkpoints(&d);
            }),
        ),
    ];

    for (name, open_fn) in openers {
        open_fn(&mut app);
        let tree = inspect_ui_tree(&app, Some("dialog"), false);
        let text_ids: Vec<String> = tree
            .dialog
            .as_ref()
            .map(|d| d.controls.iter().filter(|n| n.role == "textbox" && !n.widget_id.ends_with(".val")).map(|n| n.widget_id.clone()).collect())
            .unwrap_or_default();
        if text_ids.is_empty() {
            return Err(format!("expected at least 1 editable textbox in dialog {name}"));
        }
        for wid in &text_ids {
            app.focus_widget_id = Some(wid.clone());
            let out = text_input(&mut app, "1")?;
            if !out.handled || out.action == "text_input_ignored" {
                return Err(format!("textbox `{wid}` in dialog {name} ignored TextInput: {out:?}"));
            }
        }
        app.canvas_dialogs.close_dialog();
        curvebook_ui_egui::interaction::clear_modal_text_buffers(&mut app);
    }

    // Verify ALL canonical inspect/interaction textboxes also accept input and sync live state.
    app.canvas_dialogs.open_power_wizard();
    app.focus_widget_id = Some(String::from("power.alpha"));
    let _ = key_down(&mut app, "a", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    let _ = text_input(&mut app, "0.01")?;
    if (app.canvas_dialogs.power_wizard.alpha - 0.01).abs() > 1e-9 {
        return Err(format!("power.alpha did not sync: {}", app.canvas_dialogs.power_wizard.alpha));
    }
    app.canvas_dialogs.close_dialog();
    curvebook_ui_egui::interaction::clear_modal_text_buffers(&mut app);

    app.canvas_dialogs.open_edit_parameters(&app.document().clone(), &SheetId::from("res-fit"));
    app.focus_widget_id = Some(String::from("edit.custom_title"));
    let _ = text_input(&mut app, "RevisedTitle")?;
    if app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.custom_title.as_deref()) != Some("RevisedTitle") {
        return Err(format!(
            "edit.custom_title did not sync: {:?}",
            app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.custom_title.as_ref())
        ));
    }
    app.canvas_dialogs.close_dialog();
    curvebook_ui_egui::interaction::clear_modal_text_buffers(&mut app);

    app.canvas_dialogs.open_analyze_dialog(&app.document().clone(), &ds_id);
    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("nonlinear_regression"));
    app.focus_widget_id = Some(String::from("analyze.fit.equation_id"));
    let _ = key_down(&mut app, "a", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    let _ = text_input(&mut app, "michaelis_menten")?;
    if app.canvas_dialogs.analyze.equation_id != "michaelis_menten" {
        return Err(format!("analyze.fit.equation_id did not sync: {:?}", app.canvas_dialogs.analyze.equation_id));
    }
    app.canvas_dialogs.close_dialog();
    curvebook_ui_egui::interaction::clear_modal_text_buffers(&mut app);

    app.canvas_dialogs.open_format_selected_points(&app.document().clone(), &gs_id, vec![(0, 0)]);
    app.focus_widget_id = Some(String::from("format_points.color"));
    let _ = key_down(&mut app, "a", KeyModifiers { meta: true, ..KeyModifiers::default() })?;
    let _ = text_input(&mut app, "#224466")?;
    if app.canvas_dialogs.format_points.color.as_deref() != Some("#224466") {
        return Err(format!("format_points.color did not sync: {:?}", app.canvas_dialogs.format_points.color));
    }
    app.canvas_dialogs.close_dialog();
    curvebook_ui_egui::interaction::clear_modal_text_buffers(&mut app);

    // Verify info.notes syncs directly to InfoSheet::notes_markdown preserving seeded text.
    app.execute_command("sheet.select", json!({ "sheet_id": "info-1" })).map_err(|e| format!("{e}"))?;
    app.focus_widget_id = Some(String::from("info.notes"));
    let _ = text_input(&mut app, " note")?;
    let info_notes = &app.document().info_sheet(&SheetId::from("info-1")).ok_or_else(|| String::from("missing info-1"))?.notes_markdown;
    if info_notes != "Initial note" {
        return Err(format!("expected info.notes to preserve initial text and sync live, got {info_notes:?}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_10_tab_not_swallowed_when_formula_bar_active_but_unfocused() -> TestRes {
    let (mut app, _ds_id, _gs_id) = build_xy_app()?;
    let doc = app.document().clone();
    app.grid.formula_bar.active = true;
    app.grid.formula_bar.set_input("=ME", None, &doc);
    app.focus_widget_id = Some(String::from("inspector.title"));

    let unfocused_tab = key_down(&mut app, "Tab", KeyModifiers::default())?;
    if unfocused_tab.action == "formula_bar.tab_complete" || app.grid.formula_bar.input != "=ME" {
        return Err(format!(
            "Tab must not autocomplete formula bar when focus is elsewhere: out={unfocused_tab:?}, input={}",
            app.grid.formula_bar.input
        ));
    }

    app.focus_widget_id = Some(String::from("formula_bar.input"));
    let focused_tab = key_down(&mut app, "Tab", KeyModifiers::default())?;
    if focused_tab.action != "formula_bar.tab_complete" || app.grid.formula_bar.input == "=ME" {
        return Err(format!("Tab should autocomplete formula bar when focused: out={focused_tab:?}, input={}", app.grid.formula_bar.input));
    }
    Ok(())
}

#[test]
fn fa_sec13_11_newcombe_wilson_enabled_for_reopened_2x2_contingency_with_odds_ratio() -> TestRes {
    let mut doc = ProjectDocument::new("Contingency Test").map_err(|e| format!("{e}"))?;
    let ds = DataSheet::new("cont-1", "2x2", TableArchetype::Contingency(ContingencyTable::new(2, 2).map_err(|e| format!("{e}"))?))
        .map_err(|e| format!("{e}"))?;
    doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
    let cont = ContingencySpec { compute_odds_and_risk_ratios: true, ..ContingencySpec::default() };
    let spec = AnalysisSpec::new("res-cont", "cont-1", AnalysisKind::Contingency(cont)).map_err(|e| format!("{e}"))?;
    let rs = ResultSheet::new(spec).map_err(|e| format!("{e}"))?;
    doc.add_result_sheet(rs).map_err(|e| format!("{e}"))?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.open_edit_parameters(&app.document().clone(), &SheetId::from("res-cont"));
    // Reopening a 2x2 Contingency analysis with `compute_odds_and_risk_ratios=true` must initialize
    // `contingency_attributable_risk_checked=true` and keep Newcombe-Wilson enabled immediately (`FA-SEC13-11`).
    if !app.canvas_dialogs.edit_params.contingency_attributable_risk_checked {
        return Err(String::from("expected attributable_risk initialized true on reopen when compute_odds_and_risk_ratios=true"));
    }
    let pred_reopen = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.contingency.ci.newcombe_wilson");
    if !pred_reopen.enabled {
        return Err(format!("expected Newcombe-Wilson enabled on reopen when compute_odds_and_risk_ratios=true: pred={pred_reopen:?}"));
    }

    // Explicitly unchecking `attributable_risk` must disable Newcombe-Wilson (even while `odds_ratio` stays true).
    let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "edit.contingency.attributable_risk": false } }))
        .map_err(|e| format!("patch attributable_risk off: {e}"))?;
    let pred_uncheck = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.contingency.ci.newcombe_wilson");
    if pred_uncheck.enabled {
        return Err(format!("expected Newcombe-Wilson disabled after unchecking attributable_risk: pred={pred_uncheck:?}"));
    }

    // Disabling compute_odds_and_risk_ratios with attributable_risk=false disables Newcombe-Wilson.
    let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "edit.contingency.odds_ratio": false } }))
        .map_err(|e| format!("patch odds_ratio off: {e}"))?;
    let pred_off = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.contingency.ci.newcombe_wilson");
    if pred_off.enabled {
        return Err(format!("expected Newcombe-Wilson disabled when both flags are false, got {pred_off:?}"));
    }

    // Enabling attributable_risk enables Newcombe-Wilson even with odds_ratio=false.
    let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "edit.contingency.attributable_risk": true } }))
        .map_err(|e| format!("patch attributable_risk on: {e}"))?;
    let pred_ar = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.contingency.ci.newcombe_wilson");
    if !pred_ar.enabled {
        return Err(format!("expected Newcombe-Wilson enabled when attributable_risk=true, got {pred_ar:?}"));
    }

    // Verify non-2x2 (3x2) contingency disables Newcombe-Wilson even when compute_odds_and_risk_ratios=true.
    let mut doc_3x2 = ProjectDocument::new("Contingency 3x2").map_err(|e| format!("{e}"))?;
    doc_3x2
        .add_data_sheet(
            DataSheet::new("cont-3x2", "3x2", TableArchetype::Contingency(ContingencyTable::new(3, 2).map_err(|e| format!("{e}"))?))
                .map_err(|e| format!("{e}"))?,
        )
        .map_err(|e| format!("{e}"))?;
    let cont2 = ContingencySpec { compute_odds_and_risk_ratios: true, ..ContingencySpec::default() };
    let spec2 = AnalysisSpec::new("res-cont-3x2", "cont-3x2", AnalysisKind::Contingency(cont2)).map_err(|e| format!("{e}"))?;
    doc_3x2.add_result_sheet(ResultSheet::new(spec2).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;
    let mut app_3x2 = UiApp::from_session(Session::from_document(doc_3x2));
    app_3x2.canvas_dialogs.open_edit_parameters(&app_3x2.document().clone(), &SheetId::from("res-cont-3x2"));
    let pred_3x2 = evaluate_dialog_widget(&app_3x2.canvas_dialogs, app_3x2.document(), "edit.contingency.ci.newcombe_wilson");
    if pred_3x2.enabled {
        return Err(format!("expected Newcombe-Wilson disabled on 3x2 table, got {pred_3x2:?}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_12_format_points_brushed_fallback_and_factorial_residual_covariance() -> TestRes {
    let (mut app, ds_id, gs_id) = build_xy_app()?;
    app.canvas_dialogs.open_format_selected_points(&app.document().clone(), &gs_id, Vec::new());
    app.brushed_points = vec![SourceCellId { sheet_id: ds_id.clone(), row: 0, col: 0, subcol: 0 }];
    let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "format_points.color": "#112233" } }))
        .map_err(|e| format!("patch format_points: {e}"))?;

    let ok_enabled = evaluate_dialog_button_for_app(&app, "ok").ok_or_else(|| String::from("missing ok predicate"))?;
    let tree = inspect_ui_tree(&app, Some("dialog"), false);
    let ok_node = find_widget_in_tree(&tree, "dialog.button.ok").ok_or_else(|| String::from("missing dialog.button.ok node"))?;
    if !ok_enabled.enabled || !ok_node.enabled {
        return Err(format!("expected FormatSelectedPoints ok enabled via brushed_points fallback: ok={ok_enabled:?}, node={ok_node:?}"));
    }

    app.brushed_points.clear();
    app.brushed_cell = None;
    let ok_disabled = evaluate_dialog_button_for_app(&app, "ok").ok_or_else(|| String::from("missing ok disabled predicate"))?;
    if ok_disabled.enabled {
        return Err(format!("expected FormatSelectedPoints ok disabled without selected or brushed points: {ok_disabled:?}"));
    }
    app.canvas_dialogs.close_dialog();

    // Factorial residual_covariance disabled when three_way == true.
    app.canvas_dialogs.open_analyze_dialog(&app.document().clone(), &ds_id);
    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("two_way_anova"));
    app.canvas_dialogs.analyze.factorial_three_way = true;
    let tree_fac = inspect_ui_tree(&app, Some("dialog"), false);
    let res_node = find_widget_in_tree(&tree_fac, "analyze.factorial.residual_covariance")
        .ok_or_else(|| String::from("missing analyze.factorial.residual_covariance node"))?;
    let res_pred = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "analyze.factorial.residual_covariance");
    if res_node.enabled || res_pred.enabled {
        return Err(format!("expected residual_covariance disabled under three_way=true: node={res_node:?}, pred={res_pred:?}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_13_duplicate_choice_reflects_live_view_shard_state() -> TestRes {
    let (mut app, ds_id, _gs_id) = build_xy_app()?;
    let spec = AnalysisSpec::new("res-existing", ds_id.as_str(), AnalysisKind::OneWayAnova(Default::default())).map_err(|e| format!("{e}"))?;
    let mut doc = app.document().clone();
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| format!("{e}"))?).map_err(|e| format!("{e}"))?;
    app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
        source_sheet_id: ds_id,
        existing_result_ids: vec![SheetId::from("res-existing")],
        pending_entry_id: String::from("one_way_anova"),
    };

    let tree_initial = inspect_ui_tree(&app, Some("dialog"), false);
    let initial_node = find_widget_in_tree(&tree_initial, "duplicate.choice").ok_or_else(|| String::from("missing initial duplicate.choice node"))?;
    if initial_node.value != Some(json!("create_new")) {
        return Err(format!("expected initial duplicate.choice == create_new, got {:?}", initial_node.value));
    }

    let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "duplicate.choice": "modify_existing" } }))
        .map_err(|e| format!("patch duplicate.choice: {e}"))?;
    let tree_updated = inspect_ui_tree(&app, Some("dialog"), false);
    let updated_node = find_widget_in_tree(&tree_updated, "duplicate.choice").ok_or_else(|| String::from("missing updated duplicate.choice node"))?;
    if updated_node.value != Some(json!("modify_existing")) {
        return Err(format!("expected duplicate.choice to reflect live view_shards value `modify_existing`, got {:?}", updated_node.value));
    }
    Ok(())
}

#[test]
fn fa_sec13_14_x_plus_y_spill_paste_rolls_back_atomically_on_invalid_x() -> TestRes {
    let (mut app, ds_id, _gs_id) = build_xy_app()?;
    let before_doc = app.document().clone();
    let before_undo = app.session().undo_depth();
    let before_seq = app.mutation_seq();

    // Start at the last row (row 3) so Y spill (`table.paste` on row 3 only) succeeds and mutates
    // Y[3,0] -> 999.0 while leaving row_count == 4, causing `table.setXCells` at row 4 to fail
    // with CellOutOfBounds; atomic rollback must restore both X and Y cleanly.
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("11.0\t999.0\n22.0");
    app.set_clipboard_adapter(stub);
    app.focus_widget_id = Some(String::from("grid.cell"));
    app.grid.select_x_cell_sync(3, 0);

    let res = key_down(&mut app, "v", KeyModifiers { meta: true, ..KeyModifiers::default() });
    if res.is_ok() {
        return Err(format!("expected out-of-bounds X row error on X+Y spill paste, got {res:?}"));
    }
    if app.document() != &before_doc {
        let ds = app.document().data_sheet(&ds_id).ok_or_else(|| String::from("missing ds"))?;
        return Err(format!("Y column was partially mutated after X paste failure: y[3]={:?}", ds.cell(CellCoord { row: 3, col: 0, subcol: 0 })));
    }
    if app.session().undo_depth() != before_undo || app.mutation_seq() != before_seq {
        return Err(format!(
            "session state drifted after failed X+Y paste: undo {}->{}, seq {}->{}",
            before_undo,
            app.session().undo_depth(),
            before_seq,
            app.mutation_seq()
        ));
    }
    Ok(())
}
