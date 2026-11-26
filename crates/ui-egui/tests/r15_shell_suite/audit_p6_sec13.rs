//! Regression suite for P6 UI-core audit findings (`FA-SEC13-02`..`14` and `FA-SEC04-04`).
#![forbid(unsafe_code)]

use std::io::{BufReader, Cursor};
use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, CellValue, ContingencySpec, ContingencyTable, DataSheet, ErrorCode, ErrorPropagationMode, GlmFamily,
    GlmSpec, InfoSheet, MultivariateSpec, NonlinearFitSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTabKind, SheetId,
    SourceCellId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XyTable,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::control_io::serve_control_stream_with_quit_and_token;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::modal_predicates::{evaluate_dialog_button_for_app, evaluate_dialog_widget};
use curvebook_ui_egui::os_integration::{ClipboardBundle, OsAdapterStatus, PlatformClipboardAdapter, StubPlatformClipboardAdapter};
use curvebook_ui_egui::workspace_nav::WorkspaceNavState;
use curvebook_ui_egui::{KeyModifiers, UiApp, inspect_ui_tree};
use serde_json::{Value, json};

use crate::common::{TestRes, call_control, cell_number, decode_base64, ensure, key_down, seed_xy_data_and_graph_app, text_input};
use crate::helpers::{exec, gui, make_result_tab};

fn build_xy_app() -> Result<(UiApp, SheetId, SheetId), String> {
    seed_xy_data_and_graph_app("ds-xy", "gs-xy")
}

fn build_result_app(status: ResultStatus) -> Result<(UiApp, SheetId, SheetId), String> {
    let (mut app, ds_id, _) = build_xy_app()?;
    let mut spec = AnalysisSpec::new("res-1", ds_id.as_str(), AnalysisKind::OneWayAnova(Default::default())).map_err(|e| e.to_string())?;
    spec.custom_title = Some(String::from("+CustomTitle"));
    let mut rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    rs.status = status;
    rs.tabs.push(make_result_tab(
        "tab-1",
        "Summary",
        ResultTabKind::AnovaTable,
        &["=Header", "Value"],
        &["Row1"],
        vec![vec![ResultCell::Text(String::from("=1+1")), ResultCell::Number { value: 42.5, prefix: None, units: None }]],
    ));
    let mut doc = app.document().clone();
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::from("res-1"));
    app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    Ok((app, ds_id, SheetId::from("res-1")))
}

fn build_contingency_app(rows: usize, cols: usize, id: &str) -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("Contingency").map_err(|e| e.to_string())?;
    let ds = DataSheet::new(id, id, TableArchetype::Contingency(ContingencyTable::new(rows, cols).map_err(|e| e.to_string())?))
        .map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    let cont = ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() };
    let rs_id = format!("res-{id}");
    let spec = AnalysisSpec::new(rs_id.as_str(), id, AnalysisKind::Contingency(cont)).map_err(|e| e.to_string())?;
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.open_edit_parameters(&app.document().clone(), &SheetId::from(rs_id.as_str()));
    Ok(app)
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
    let (mut ok_app, _, rs_id) = build_result_app(ResultStatus::UpToDate)?;
    for (id, params, needles) in [
        ("tidy-ok", json!({ "format": "csv", "sheet_id": rs_id.as_str(), "tidy": true }), &["'=1+1", "'+CustomTitle"][..]),
        ("tab-ok", json!({ "format": "csv", "sheet_id": rs_id.as_str(), "tab_id": "tab-1" }), &["'=Header", "'=1+1"][..]),
    ] {
        let resp = call_control(&mut ok_app, &json!({ "id": id, "method": "doc.export", "params": params }))?;
        ensure(resp["ok"] == json!(true), format!("{id}: expected ok, got {resp}"))?;
        let csv =
            String::from_utf8(decode_base64(resp["result"]["data_base64"].as_str().ok_or("missing data_base64")?)?).map_err(|e| e.to_string())?;
        for needle in needles {
            ensure(csv.contains(needle), format!("{id}: missing `{needle}` in:\n{csv}"))?;
        }
    }

    let (mut fail_app, _, rs_fail) =
        build_result_app(ResultStatus::Failed { code: ErrorCode::InsufficientData, message: String::from("need >= 2 groups") })?;
    for (id, params) in [
        ("tidy-fail", json!({ "format": "csv", "sheet_id": rs_fail.as_str(), "tidy": true })),
        ("tab-fail", json!({ "format": "csv", "sheet_id": rs_fail.as_str(), "tab_id": "tab-1" })),
    ] {
        let resp = call_control(&mut fail_app, &json!({ "id": id, "method": "doc.export", "params": params }))?;
        let err = resp["error"].as_str().unwrap_or_default();
        ensure(resp["ok"] == json!(false) && err.contains("CSV export failed") && err.contains("need >= 2 groups"), format!("{id}: {resp}"))?;
    }
    Ok(())
}

#[test]
fn fa_sec13_02_atomic_batch_rolls_back_beyond_64_undo_entries() -> TestRes {
    let mut app = UiApp::new();
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "ds-seed", "title": "Seed", "kind": "xy", "cols": 1, "rows": 2, "auto_graph": false }))?;
    let (baseline_doc, baseline_undo, baseline_redo) = (app.document().clone(), app.session().undo_depth(), app.session().redo_depth());

    let mut commands: Vec<Value> = (0..70usize)
        .map(|i| json!({ "command": "sheet.createData", "params": { "sheet_id": format!("tmp-{i}"), "title": format!("Tmp {i}"), "kind": "column", "cols": 1, "rows": 2, "auto_graph": false } }))
        .collect();
    commands.push(json!({ "command": "nonexistent.command", "params": {} }));

    let mix_commands = vec![
        json!({ "command": "sheet.createData", "params": { "sheet_id": "tmp-a", "title": "TmpA", "auto_graph": false } }),
        json!({ "command": "edit.undo", "params": {} }),
        json!({ "command": "sheet.createData", "params": { "sheet_id": "tmp-b", "title": "TmpB", "auto_graph": false } }),
        json!({ "command": "nonexistent.command", "params": {} }),
    ];

    for (id, cmds) in [("batch-70", commands), ("batch-undo-mix", mix_commands)] {
        let resp = call_control(&mut app, &json!({ "id": id, "method": "command.batch", "params": { "atomic": true, "commands": cmds } }))?;
        ensure(resp["ok"] == json!(false), format!("{id}: expected failure, got {resp}"))?;
        ensure(app.document() == &baseline_doc, format!("{id}: document not rolled back"))?;
        ensure(app.session().undo_depth() == baseline_undo && app.session().redo_depth() == baseline_redo, format!("{id}: undo/redo drifted"))?;
    }
    Ok(())
}

#[test]
fn fa_sec13_03_unifies_egui_dispatch_under_control_io_auth_panic_and_batch() -> TestRes {
    let mut app = UiApp::new();
    let payload = concat!(
        "{\"id\":\"bad-tok\",\"method\":\"ui.inspect\",\"token\":\"wrong-token\",\"params\":{\"metrics\":{\"width_pt\":800,\"height_pt\":600}}}\n",
        "{\"id\":\"inspect-ok-tok\",\"method\":\"ui.inspect\",\"token\":\"good-tok\",\"params\":{\"metrics\":{\"width_pt\":800,\"height_pt\":600}}}\n",
        "{\"id\":\"after-inspect\",\"method\":\"ping\",\"token\":\"good-tok\"}\n"
    );
    let mut out = Vec::new();
    let _ = serve_control_stream_with_quit_and_token(&mut app, &mut BufReader::new(Cursor::new(payload.as_bytes())), &mut out, "good-tok")
        .map_err(|e| e.to_string())?;
    let replies: Vec<Value> = String::from_utf8(out)
        .map_err(|e| e.to_string())?
        .lines()
        .map(|l| serde_json::from_str(l).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    ensure(replies.len() == 3, format!("expected 3 replies, got {}", replies.len()))?;
    ensure(
        replies[0]["ok"] == json!(false) && replies[0].pointer("/error_detail/code").and_then(Value::as_str) == Some("unauthorized"),
        format!("bad_tok: {}", replies[0]),
    )?;
    ensure(
        replies[1]["ok"] == json!(true) && replies[1].pointer("/result/menus").and_then(Value::as_array).is_some_and(|a| !a.is_empty()),
        format!("inspect: {}", replies[1]),
    )?;
    ensure(replies[2]["ok"] == json!(true), format!("ping: {}", replies[2]))?;

    let mut clean_app = UiApp::new();
    let bad = call_control(
        &mut clean_app,
        &json!({ "id": "bad-dim", "method": "ui.inspect", "params": { "metrics": { "width_pt": 64, "height_pt": 64 } } }),
    )?;
    ensure(bad["ok"] == json!(false), format!("expected out-of-bounds reject: {bad}"))?;
    let batch = call_control(
        &mut clean_app,
        &json!({ "id": "batch-inspect", "method": "command.batch", "params": { "atomic": true, "commands": [{ "command": "ui.inspect", "params": { "metrics": { "width_pt": 1024, "height_pt": 640 } } }] } }),
    )?;
    ensure(batch["ok"] == json!(true), format!("batch ui.inspect failed: {batch}"))
}

#[test]
fn fa_sec13_04_copy_methods_report_respects_human_path_stub_and_errors() -> TestRes {
    let (mut app, _, rs_id) = build_result_app(ResultStatus::UpToDate)?;
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(stub.clone());
    app.set_human_path_only(true);

    let skip = call_control(
        &mut app,
        &json!({ "id": "m1", "method": "ui.result.copyMethodsReport", "params": { "result_sheet_id": rs_id.as_str(), "skip_os_clipboard": true } }),
    )?;
    ensure(skip["ok"] == json!(true) && stub.captured_texts().is_empty(), format!("skip_os_clipboard wrote stub: {skip}"))?;

    let copy =
        call_control(&mut app, &json!({ "id": "m2", "method": "ui.result.copyMethodsReport", "params": { "result_sheet_id": rs_id.as_str() } }))?;
    ensure(copy["ok"] == json!(true) && stub.captured_texts().len() == 1, format!("human_path copy failed: {copy}"))?;

    app.set_clipboard_adapter(Arc::new(FailingWriteClipboardAdapter));
    let err =
        call_control(&mut app, &json!({ "id": "m3", "method": "ui.result.copyMethodsReport", "params": { "result_sheet_id": rs_id.as_str() } }))?;
    ensure(err["ok"] == json!(false), format!("failing clipboard must error: {err}"))
}

#[test]
fn fa_sec13_05_workspace_nav_click_is_immutable_and_emits_sheet_select() -> TestRes {
    let mut doc = ProjectDocument::new("Nav Test").map_err(|e| e.to_string())?;
    for (id, title) in [("ds-1", "First"), ("ds-2", "Second")] {
        let xy = XyTable::new(SubcolumnMode::Single, 1, 2).map_err(|e| e.to_string())?;
        doc.add_data_sheet(DataSheet::new(id, title, TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    doc.active_sheet_id = Some(SheetId::from("ds-1"));
    let doc_ref: &ProjectDocument = &doc;

    let mut nav = WorkspaceNavState::default();
    nav.click_navigator_entry(doc_ref, &SheetId::from("ds-2")).map_err(|e| e.to_string())?;
    nav.click_family_entry(doc_ref, &SheetId::from("ds-1"), false).map_err(|e| e.to_string())?;
    ensure(nav.click_navigator_entry(doc_ref, &SheetId::from("missing-sheet")).is_err(), "nonexistent sheet must fail")?;
    ensure(doc.active_sheet_id.as_ref().map(SheetId::as_str) == Some("ds-1"), "doc mutated directly")?;
    let intents = std::mem::take(&mut nav.pending_intents);
    ensure(intents.len() == 2 && intents.iter().all(|i| i.command_id == "sheet.select"), format!("bad intents: {intents:?}"))
}

#[test]
fn fa_sec13_06_multiline_modal_enter_inserts_newline_and_cmd_enter_submits() -> TestRes {
    let (mut app, ds_id, _) = build_xy_app()?;
    let cmd = KeyModifiers { meta: true, ..Default::default() };

    app.canvas_dialogs.open_checkpoints(&app.document().clone());
    app.focus_widget_id = Some(String::from("checkpoints.note"));
    text_input(&mut app, "Line 1")?;
    let enter = key_down(&mut app, "Enter", KeyModifiers::default())?;
    ensure(enter.handled && app.canvas_dialogs.active_dialog == ActiveDialog::Checkpoints, format!("checkpoints Enter: {enter:?}"))?;
    text_input(&mut app, "Line 2")?;
    ensure(app.canvas_dialogs.checkpoints.note_draft == "Line 1\nLine 2", "multiline note_draft mismatch")?;
    ensure(key_down(&mut app, "Enter", cmd)?.handled, "Cmd+Enter must submit checkpoints")?;
    ensure(app.document().checkpoints.last().map(|cp| cp.note.as_str()) == Some("Line 1\nLine 2"), "saved note lost newline")?;
    app.canvas_dialogs.close_dialog();

    app.canvas_dialogs.open_exclude_with_reason(ds_id.clone(), vec![GridCellCoord::new(0, 0, 0)]);
    app.focus_widget_id = Some(String::from("exclude.note"));
    text_input(&mut app, "Reason A")?;
    let ee = key_down(&mut app, "Enter", KeyModifiers::default())?;
    ensure(ee.handled && matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExcludeWithReason { .. }), "exclude Enter closed modal")?;
    text_input(&mut app, "Reason B")?;
    ensure(app.canvas_dialogs.exclude_with_reason.note == "Reason A\nReason B", "exclude multiline note mismatch")?;
    ensure(key_down(&mut app, "Enter", cmd)?.handled && app.canvas_dialogs.active_dialog == ActiveDialog::None, "Cmd+Enter exclude failed")?;
    ensure(app.document().data_sheet(&ds_id).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).is_some_and(|c| c.excluded), "cell not excluded")?;

    app.canvas_dialogs.open_command_palette();
    app.focus_widget_id = Some(String::from("palette.query"));
    text_input(&mut app, "ui.split.toggle")?;
    let se = key_down(&mut app, "Enter", KeyModifiers::default())?;
    ensure(se.handled && app.canvas_dialogs.active_dialog == ActiveDialog::None, "single-line plain Enter must confirm")
}

#[test]
fn fa_sec13_07_textbox_copy_cut_update_stub_clipboard_over_stale_text() -> TestRes {
    let mut app = UiApp::new();
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("stale_os_clipboard");
    app.set_clipboard_adapter(stub.clone());
    let cmd = KeyModifiers { meta: true, ..Default::default() };

    app.canvas_dialogs.open_command_palette();
    app.focus_widget_id = Some(String::from("palette.query"));
    text_input(&mut app, "fresh_copy")?;
    key_down(&mut app, "a", cmd)?;
    ensure(key_down(&mut app, "c", cmd)?.handled && stub.last_text().as_deref() == Some("fresh_copy"), "Cmd+C failed")?;

    app.canvas_dialogs.close_dialog();
    app.canvas_dialogs.open_checkpoints(&app.document().clone());
    app.focus_widget_id = Some(String::from("checkpoints.name"));
    key_down(&mut app, "a", cmd)?;
    ensure(key_down(&mut app, "v", cmd)?.handled && app.canvas_dialogs.checkpoints.name_draft == "fresh_copy", "Cmd+V failed")?;

    key_down(&mut app, "a", cmd)?;
    ensure(key_down(&mut app, "x", cmd)?.handled && app.canvas_dialogs.checkpoints.name_draft.is_empty(), "Cmd+X failed")?;
    app.focus_widget_id = Some(String::from("checkpoints.note"));
    key_down(&mut app, "v", cmd)?;
    ensure(app.canvas_dialogs.checkpoints.note_draft == "fresh_copy", "pasted cut text mismatch")
}

#[test]
fn fa_sec13_08_cmd_v_on_active_x_cell_pastes_x_and_spills_y() -> TestRes {
    let (mut app, ds_id, _) = build_xy_app()?;
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("11.5\t110.0\n22.5\t220.0");
    app.set_clipboard_adapter(stub);
    app.focus_widget_id = Some(String::from("grid.cell"));
    app.grid.select_x_cell_sync(0, 0);

    ensure(key_down(&mut app, "v", KeyModifiers { meta: true, ..Default::default() })?.handled, "Cmd+V unhandled")?;
    let ds = app.document().data_sheet(&ds_id).ok_or("missing ds")?;
    for (row, exp_x, exp_y) in [(0, 11.5, 110.0), (1, 22.5, 220.0)] {
        ensure(ds.x_or_time_cell(row, 0).map(|c| &c.value) == Some(&CellValue::Number(exp_x)), format!("x[{row}] mismatch"))?;
        ensure(cell_number(ds, row, 0) == Some(exp_y), format!("y[{row}] mismatch"))?;
    }
    Ok(())
}

fn open_sec13_09_dialog(app: &mut UiApp, name: &str, ds: &SheetId, gs: &SheetId) -> TestRes {
    let d = app.document().clone();
    match name {
        "FormatSelectedPoints" => {
            app.canvas_dialogs.open_format_selected_points(&d, gs, vec![(0, 0)]);
        }
        "AnalyzeSheet:fit" => {
            app.canvas_dialogs.open_analyze_dialog(&d, ds);
            app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("nonlinear_regression"));
        }
        "AnalyzeSheet:transform" => {
            app.canvas_dialogs.open_analyze_dialog(&d, ds);
            app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("transform_data"));
        }
        "EditAnalysisParameters:fit" => {
            app.canvas_dialogs.open_edit_parameters(&d, &SheetId::from("res-fit"));
        }
        "EditAnalysisParameters:transform" => {
            app.canvas_dialogs.open_edit_parameters(&d, &SheetId::from("res-tr"));
        }
        "EditAnalysisParameters:glm" => {
            app.canvas_dialogs.open_edit_parameters(&d, &SheetId::from("res-glm"));
        }
        "ImportReshape" => app.canvas_dialogs.open_import_reshape(),
        "FormatAxes" => app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gs.clone() },
        "CreateGraph" => {
            app.canvas_dialogs.open_create_graph(&d, Some(ds));
        }
        "RandomizeGroups" => {
            app.canvas_dialogs.open_randomize_groups(&d);
        }
        "RandomSampleRows" => {
            app.canvas_dialogs.open_random_sample_rows(&d, Some(ds));
        }
        "SimulateDistributions" => app.canvas_dialogs.open_simulate_distributions(),
        "ChangeDataAnalyzed" => {
            app.canvas_dialogs.open_change_data_analyzed(&d, &SheetId::from("res-fit"));
        }
        "ConstantHookPicker" => app.canvas_dialogs.open_hook_picker(&d, ds, None),
        "ExcludeWithReason" => app.canvas_dialogs.open_exclude_with_reason(ds.clone(), vec![GridCellCoord::new(0, 0, 0)]),
        "CommandPalette" => app.canvas_dialogs.open_command_palette(),
        "Checkpoints" => app.canvas_dialogs.open_checkpoints(&d),
        _ => return Err(format!("unknown dialog {name}")),
    }
    Ok(())
}

#[test]
fn fa_sec13_09_all_textbox_nodes_accept_typing_and_sync_including_info_notes() -> TestRes {
    let (mut app, ds_id, gs_id) = build_xy_app()?;
    let mut doc = app.document().clone();
    let mut info = InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| e.to_string())?;
    info.notes_markdown = String::from("Initial");
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    for (id, kind) in [
        ("res-fit", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("hyperbolic"))),
        (
            "res-tr",
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: None,
                error_propagation: ErrorPropagationMode::default(),
            }),
        ),
        (
            "res-glm",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![1],
                reference_levels: Vec::new(),
                compute_vif: false,
            }),
        ),
        (
            "res-mi",
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                response_column: Some(0),
                predictor_columns: vec![1],
                imputations_m: 5,
                max_iters: 10,
                seed: 42,
            }),
        ),
    ] {
        let spec = AnalysisSpec::new(id, ds_id.as_str(), kind).map_err(|e| e.to_string())?;
        doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();

    let dialogs = [
        "FormatSelectedPoints",
        "AnalyzeSheet:fit",
        "AnalyzeSheet:transform",
        "EditAnalysisParameters:fit",
        "EditAnalysisParameters:transform",
        "EditAnalysisParameters:glm",
        "ImportReshape",
        "FormatAxes",
        "CreateGraph",
        "RandomizeGroups",
        "RandomSampleRows",
        "SimulateDistributions",
        "ChangeDataAnalyzed",
        "ConstantHookPicker",
        "ExcludeWithReason",
        "CommandPalette",
        "Checkpoints",
    ];
    for name in dialogs {
        open_sec13_09_dialog(&mut app, name, &ds_id, &gs_id)?;
        if name == "FormatAxes" {
            gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "edit.tab.axes.titles" }))?;
        }
        let tree = inspect_ui_tree(&app, Some("dialog"), false);
        let text_ids: Vec<String> = tree
            .dialog
            .as_ref()
            .map(|d| d.controls.iter().filter(|n| n.role == "textbox" && !n.widget_id.ends_with(".val")).map(|n| n.widget_id.clone()).collect())
            .unwrap_or_default();
        if text_ids.is_empty() {
            ensure(
                matches!(name, "FormatSelectedPoints" | "RandomizeGroups" | "RandomSampleRows" | "SimulateDistributions" | "ChangeDataAnalyzed"),
                format!("expected >= 1 textbox in {name}"),
            )?;
        }
        for wid in &text_ids {
            app.focus_widget_id = Some(wid.clone());
            let out = text_input(&mut app, "1")?;
            ensure(out.handled && out.action != "text_input_ignored", format!("textbox `{wid}` in {name} ignored input: {out:?}"))?;
        }
        app.canvas_dialogs.close_dialog();
        curvebook_ui_egui::interaction::clear_modal_text_buffers(&mut app);
    }

    // Verify canonical inspect/interaction textboxes sync live state.
    let cmd = KeyModifiers { meta: true, ..Default::default() };
    let mut check_sync = |open: &dyn Fn(&mut UiApp), wid: &str, select_all: bool, val: &str, ok: &dyn Fn(&UiApp) -> bool| -> TestRes {
        open(&mut app);
        app.focus_widget_id = Some(wid.to_owned());
        if select_all {
            key_down(&mut app, "a", cmd)?;
        }
        text_input(&mut app, val)?;
        ensure(ok(&app), format!("{wid} did not sync `{val}`"))?;
        app.canvas_dialogs.close_dialog();
        curvebook_ui_egui::interaction::clear_modal_text_buffers(&mut app);
        Ok(())
    };
    check_sync(&|a| a.canvas_dialogs.open_power_wizard(), "power.alpha", true, "0.01", &|a| {
        (a.canvas_dialogs.power_wizard.alpha - 0.01).abs() < 1e-9
    })?;
    check_sync(
        &|a| {
            let d = a.document().clone();
            a.canvas_dialogs.open_edit_parameters(&d, &SheetId::from("res-fit"));
        },
        "edit.custom_title",
        false,
        "RevisedTitle",
        &|a| a.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.custom_title.as_deref()) == Some("RevisedTitle"),
    )?;
    check_sync(
        &|a| {
            let d = a.document().clone();
            a.canvas_dialogs.open_analyze_dialog(&d, &ds_id);
            a.canvas_dialogs.analyze.selected_entry_id = Some(String::from("nonlinear_regression"));
        },
        "analyze.fit.equation_id",
        true,
        "michaelis_menten",
        &|a| a.canvas_dialogs.analyze.equation_id == "michaelis_menten",
    )?;
    check_sync(
        &|a| {
            let d = a.document().clone();
            a.canvas_dialogs.open_format_selected_points(&d, &gs_id, vec![(0, 0)]);
        },
        "format_points.color",
        true,
        "#224466",
        &|a| a.canvas_dialogs.format_points.color.as_deref() == Some("#224466"),
    )?;

    exec(&mut app, "sheet.select", json!({ "sheet_id": "info-1" }))?;
    app.focus_widget_id = Some(String::from("info.notes"));
    text_input(&mut app, " note")?;
    ensure(app.document().info_sheet(&SheetId::from("info-1")).ok_or("missing info-1")?.notes_markdown == "Initial note", "info.notes sync failed")
}

#[test]
fn fa_sec13_10_tab_not_swallowed_when_formula_bar_active_but_unfocused() -> TestRes {
    let (mut app, _, _) = build_xy_app()?;
    let doc = app.document().clone();
    app.grid.formula_bar.active = true;
    app.grid.formula_bar.set_input("=ME", None, &doc);

    app.focus_widget_id = Some(String::from("inspector.title"));
    let unfocused = key_down(&mut app, "Tab", KeyModifiers::default())?;
    ensure(unfocused.action != "formula_bar.tab_complete" && app.grid.formula_bar.input == "=ME", format!("unfocused Tab: {unfocused:?}"))?;

    app.focus_widget_id = Some(String::from("formula_bar.input"));
    let focused = key_down(&mut app, "Tab", KeyModifiers::default())?;
    ensure(focused.action == "formula_bar.tab_complete" && app.grid.formula_bar.input != "=ME", format!("focused Tab: {focused:?}"))
}

#[test]
fn fa_sec13_11_newcombe_wilson_enabled_for_reopened_2x2_contingency_with_odds_ratio() -> TestRes {
    let mut app = build_contingency_app(2, 2, "cont-1")?;
    ensure(app.canvas_dialogs.edit_params.contingency_attributable_risk_checked, "attributable_risk must init true on reopen")?;
    ensure(evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.contingency.ci.newcombe_wilson").enabled, "NW enabled on reopen")?;

    for (step, expect_enabled) in [(0u8, false), (1, false), (2, true)] {
        match step {
            0 => app.canvas_dialogs.edit_params.set_contingency_attributable_risk(false),
            1 => {
                app.canvas_dialogs.edit_params.contingency_odds_ratio_checked = false;
                app.canvas_dialogs.edit_params.contingency_relative_risk_checked = false;
                app.canvas_dialogs.edit_params.contingency_attributable_risk_checked = false;
                if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft
                    && let AnalysisKind::Contingency(cg) = &mut spec.kind
                {
                    cg.compute_odds_and_risk_ratios = false;
                }
            }
            _ => app.canvas_dialogs.edit_params.set_contingency_attributable_risk(true),
        }
        let pred = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.contingency.ci.newcombe_wilson");
        ensure(pred.enabled == expect_enabled, format!("after step {step}: {pred:?}"))?;
    }

    let app_3x2 = build_contingency_app(3, 2, "cont-3x2")?;
    ensure(!evaluate_dialog_widget(&app_3x2.canvas_dialogs, app_3x2.document(), "edit.contingency.ci.newcombe_wilson").enabled, "3x2 must disable NW")
}

#[test]
fn fa_sec13_12_format_points_brushed_fallback_and_factorial_residual_covariance() -> TestRes {
    let (mut app, ds_id, gs_id) = build_xy_app()?;
    app.canvas_dialogs.open_format_selected_points(&app.document().clone(), &gs_id, Vec::new());
    app.brushed_points = vec![SourceCellId { sheet_id: ds_id.clone(), row: 0, col: 0, subcol: 0 }];
    app.canvas_dialogs.format_points.color = Some(String::from("#112233"));

    let ok_pred = evaluate_dialog_button_for_app(&app, "ok").ok_or("missing ok predicate")?;
    let ok_node = inspect_ui_tree(&app, Some("dialog"), false).find_widget("dialog.button.ok").ok_or("missing dialog.button.ok")?;
    ensure(ok_pred.enabled && ok_node.enabled, "brushed_points fallback must enable ok")?;

    app.brushed_points.clear();
    app.brushed_cell = None;
    ensure(!evaluate_dialog_button_for_app(&app, "ok").ok_or("missing ok")?.enabled, "ok must disable without points")?;
    app.canvas_dialogs.close_dialog();

    app.canvas_dialogs.open_analyze_dialog(&app.document().clone(), &ds_id);
    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("two_way_anova"));
    app.canvas_dialogs.analyze.factorial_three_way = true;
    let res_node = inspect_ui_tree(&app, Some("dialog"), false).find_widget("analyze.factorial.residual_covariance").ok_or("missing res_cov")?;
    let res_pred = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "analyze.factorial.residual_covariance");
    ensure(!res_node.enabled && !res_pred.enabled, "residual_covariance must disable under three_way=true")
}

#[test]
fn fa_sec13_13_duplicate_choice_reflects_live_view_shard_state() -> TestRes {
    let (mut app, ds_id, _) = build_xy_app()?;
    let spec = AnalysisSpec::new("res-existing", ds_id.as_str(), AnalysisKind::OneWayAnova(Default::default())).map_err(|e| e.to_string())?;
    let mut doc = app.document().clone();
    doc.add_result_sheet(ResultSheet::new(spec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
        source_sheet_id: ds_id,
        existing_result_ids: vec![SheetId::from("res-existing")],
        pending_entry_id: String::from("one_way_anova"),
    };

    ensure(
        inspect_ui_tree(&app, Some("dialog"), false).find_widget("duplicate.choice").and_then(|n| n.value) == Some(json!("create_new")),
        "initial choice",
    )?;
    app.canvas_dialogs.analyze.duplicate_choice = curvebook_ui_egui::dialogs_canvas::DuplicateChoice::ModifyExisting;
    ensure(
        inspect_ui_tree(&app, Some("dialog"), false).find_widget("duplicate.choice").and_then(|n| n.value) == Some(json!("modify_existing")),
        "updated choice",
    )
}

#[test]
fn fa_sec13_14_x_plus_y_spill_paste_rolls_back_atomically_on_invalid_x() -> TestRes {
    let (mut app, ds_id, _) = build_xy_app()?;
    let (before_doc, before_undo, before_seq) = (app.document().clone(), app.session().undo_depth(), app.mutation_seq());
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("11.0\t999.0\n22.0");
    app.set_clipboard_adapter(stub);
    app.focus_widget_id = Some(String::from("grid.cell"));
    app.grid.select_x_cell_sync(3, 0);

    ensure(key_down(&mut app, "v", KeyModifiers { meta: true, ..Default::default() }).is_err(), "out-of-bounds X row paste must fail")?;
    ensure(app.document() == &before_doc, format!("Y partially mutated: {:?}", app.document().data_sheet(&ds_id)))?;
    ensure(app.session().undo_depth() == before_undo && app.mutation_seq() == before_seq, "session state drifted after rollback")
}
