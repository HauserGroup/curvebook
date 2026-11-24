#![forbid(unsafe_code)]

use std::io::{self, Cursor, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curvebook_platform::sync::Mutex;

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, match_routed_desktop_shortcut, open_document_from_files};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook::live_control::{LiveControlBridge, serve_live_control_io, spawn_live_control_accept_loop};
use curvebook::mac_menu::localized_app_submenu_labels;
use curvebook_chart::{PointPt, compile_graph};
use curvebook_doc::{BracketLabelMode, Cell, CellCoord, SheetId};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_ui_egui::ControlAuth;
use curvebook_ui_egui::StubPlatformClipboardAdapter;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use eframe::egui;
use serde_json::{Value, json};

fn unique_temp_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("cb-audit-p8-{}-{nanos}-{name}", std::process::id()))
}

fn step_native_frame(native: &mut CurveBookNativeApp, ctx: &egui::Context, raw_events: Vec<egui::Event>) -> Result<(), String> {
    let mut raw_input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        events: raw_events,
        ..Default::default()
    };
    if let Some(vp) = raw_input.viewports.get_mut(&egui::ViewportId::ROOT) {
        vp.native_pixels_per_point = Some(1.0);
    }
    native.drain_synthetic_events_into(&mut raw_input);
    let mut out = ctx.run_ui(raw_input, |ui| native.step_ui(ui));
    out.textures_delta.clear();
    Ok(())
}

fn build_anova_bracket_session() -> Result<Session, Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute(
        "sheet.createData",
        json!({
            "sheet_id": "col-1",
            "title": "Groups",
            "kind": "column",
            "cols": 3,
            "rows": 4,
            "auto_graph": false
        }),
    )?;
    for col in 0..3 {
        let base = f64::from((col + 1) * 10);
        for row in 0..4 {
            session.execute("table.setCell", json!({ "sheet_id": "col-1", "row": row, "col": col, "value": base + f64::from(row) }))?;
        }
    }
    session.execute(
        "stats.run",
        json!({
            "kind": "one_way_anova",
            "source_sheet_id": "col-1",
            "sheet_id": "res-anova",
            "confidence_percent": 95.0,
            "multiplicity": "tukey",
            "alpha": 0.05,
            "report_confidence_intervals": false
        }),
    )?;
    session.execute("graph.create", json!({ "sheet_id": "gr-col", "data_sheet_id": "col-1", "kind": "column" }))?;
    Ok(session)
}

#[test]
fn fa_sec19_01_bracket_intents_are_flat_and_execute_cleanly() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = build_anova_bracket_session()?;
    let g_id = SheetId::new("gr-col");
    let mut ui_app = UiApp::from_session(session.clone());
    let canvas = &mut ui_app.canvas_dialogs.canvas;
    canvas.set_mode(serde_json::from_value(json!("Bracket"))?);

    let anchor_g0_a = serde_json::from_value(json!({
        "pt": { "x": 100.0, "y": 90.0 },
        "group_index": 0,
        "data_value": null,
        "result_sheet_id": "res-anova"
    }))?;
    assert!(canvas.click_bracket(&g_id, anchor_g0_a, BracketLabelMode::SummaryStars).is_none());

    // Repeated same-group click must preserve pending_bracket_start.
    let anchor_g0_b = serde_json::from_value(json!({
        "pt": { "x": 102.0, "y": 91.0 },
        "group_index": 0,
        "data_value": null,
        "result_sheet_id": "res-anova"
    }))?;
    assert!(canvas.click_bracket(&g_id, anchor_g0_b, BracketLabelMode::SummaryStars).is_none());
    assert_eq!(canvas.pending_bracket_start.as_ref().and_then(|a| a.group_index), Some(0));

    let anchor_g2 = serde_json::from_value(json!({
        "pt": { "x": 220.0, "y": 98.0 },
        "group_index": 2,
        "data_value": null,
        "result_sheet_id": null
    }))?;
    let intent = canvas.click_bracket(&g_id, anchor_g2, BracketLabelMode::NumericPOrQ).ok_or("expected flat comparison bracket intent")?;
    assert_eq!(intent.command_id, "graph.addComparisonBrackets");
    assert!(intent.params.get("brackets").is_none(), "must emit flat engine schema params: {}", intent.params);

    session.execute(&intent.command_id, intent.params.clone())?;
    let gs = session.document().graph_sheet(&g_id).ok_or("missing gr-col")?.clone();
    assert_eq!(gs.spec.comparisons.len(), 1);
    assert_eq!(gs.spec.comparisons[0].group_a, 0);
    assert_eq!(gs.spec.comparisons[0].group_b, 2);

    // Drag with ResultSheet emits a valid bracket intent, while drag without any backing ResultSheet returns None.
    let scene = compile_graph(&gs, session.document())?;
    let mid_y = scene.plot_rect_pt.y + scene.plot_rect_pt.height * 0.5;
    let p_left = PointPt::new(scene.plot_rect_pt.x + scene.plot_rect_pt.width * (1.0 / 6.0), mid_y);
    let p_right = PointPt::new(scene.plot_rect_pt.x + scene.plot_rect_pt.width * (5.0 / 6.0), mid_y);
    canvas.begin_drag(p_left);
    let drag_intent =
        canvas.end_drag_on_graph(&scene, &gs, session.document(), p_right).ok_or("expected drag bracket intent when ResultSheet exists")?;
    assert_eq!(drag_intent.params["result_sheet_id"], json!("res-anova"));

    let mut no_res_doc = session.document().clone();
    no_res_doc.result_sheets.clear();
    let no_res_scene = compile_graph(&gs, &no_res_doc)?;
    canvas.begin_drag(p_left);
    assert!(
        canvas.end_drag_on_graph(&no_res_scene, &gs, &no_res_doc, p_right).is_none(),
        "must not substitute graph.id when no ResultSheet backs the graph"
    );
    Ok(())
}

#[test]
fn fa_sec19_04_shortcuts_routing_fkeys_alt_cmd_x_and_cmd_zero_collision() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    for (cmd_id, combo) in [("ui.analyze.open", "F8"), ("ui.editParams.open", "F4"), ("ui.print.open", "Alt+P"), ("ui.changeGraphType.open", "Alt+G")]
    {
        app.shortcuts.set_shortcut(cmd_id, combo)?;
    }

    let alt_mods = KeyModifiers { alt: true, ..Default::default() };
    let no_mods = KeyModifiers::default();
    assert_eq!(match_routed_desktop_shortcut(&app, "F8", no_mods).as_deref(), Some("ui.analyze.open"));
    assert_eq!(match_routed_desktop_shortcut(&app, "P", alt_mods).as_deref(), Some("ui.print.open"));
    assert_eq!(match_routed_desktop_shortcut(&app, "G", alt_mods).as_deref(), Some("ui.changeGraphType.open"));

    // Result-sheet scoped `ui.editParams.open` routes cleanly when active sheet is a ResultSheet.
    let mut rs_app = UiApp::from_session(build_anova_bracket_session()?);
    rs_app.session_mut().execute("sheet.select", json!({ "sheet_id": "res-anova" }))?;
    rs_app.shortcuts.set_shortcut("ui.editParams.open", "F4")?;
    assert_eq!(match_routed_desktop_shortcut(&rs_app, "F4", no_mods).as_deref(), Some("ui.editParams.open"));

    // Cmd+0 matches `ui.zoom.fitSheet` in desktop routing.
    assert_eq!(match_routed_desktop_shortcut(&app, "0", KeyModifiers { meta: true, ..Default::default() }).as_deref(), Some("ui.zoom.fitSheet"));

    // 1. Live CurveBookNativeApp `Cmd+X` cuts selection without hitting OS pasteboard.
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        clipboard_adapter: Some(Arc::clone(&stub) as _),
        ..DesktopHostConfig::default()
    });
    let mut sess = build_anova_bracket_session()?;
    sess.execute("sheet.select", json!({ "sheet_id": "col-1" }))?;
    let wid = rt.open_window_with_session(sess, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    rt.window_mut(wid)?.app_mut().grid.select_single_sync(CellCoord::new(0, 0, 0).into());
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut native = CurveBookNativeApp::new(rt, wid, None);
    step_native_frame(&mut native, &ctx, Vec::new())?;
    let cmd_x_mods = egui::Modifiers { command: true, mac_cmd: cfg!(target_os = "macos"), ctrl: !cfg!(target_os = "macos"), ..Default::default() };
    step_native_frame(
        &mut native,
        &ctx,
        vec![egui::Event::Key { key: egui::Key::X, physical_key: None, pressed: true, repeat: false, modifiers: cmd_x_mods }],
    )?;
    assert!(
        native
            .runtime()
            .window(wid)?
            .app()
            .document()
            .data_sheet(&SheetId::new("col-1"))
            .and_then(|d| d.cell(CellCoord::new(0, 0, 0)))
            .is_none_or(|c| c.is_empty()),
        "Cmd+X should clear selected cell"
    );

    // 2. Live CurveBookNativeApp: F-key & Alt bindings fire cleanly, custom shortcut overrides hardcoded fallback, and Cmd+0 disambiguates.
    native.runtime_mut().window_mut(wid)?.app_mut().shortcuts.set_shortcut("ui.print.open", "F8")?;
    native.runtime_mut().window_mut(wid)?.app_mut().shortcuts.set_shortcut("ui.analyze.open", "Alt+P")?;
    native.runtime_mut().window_mut(wid)?.app_mut().shortcuts.set_shortcut("project.open", "")?;
    native.runtime_mut().window_mut(wid)?.app_mut().shortcuts.set_shortcut("ui.analyze.open", "Cmd+O")?;
    step_native_frame(&mut native, &ctx, Vec::new())?;

    // Press F8 -> opens Print cleanly without Cmd modifier.
    step_native_frame(
        &mut native,
        &ctx,
        vec![egui::Event::Key { key: egui::Key::F8, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
    )?;
    assert!(matches!(native.runtime().window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::Print { .. }));
    native.runtime_mut().window_mut(wid)?.app_mut().canvas_dialogs.close_dialog();

    // Press Cmd+O -> customized `ui.analyze.open` takes priority over hardcoded `project.open`.
    let cmd_mods = egui::Modifiers { command: true, ctrl: true, ..Default::default() };
    step_native_frame(
        &mut native,
        &ctx,
        vec![egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods }],
    )?;
    assert!(matches!(native.runtime().window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    native.runtime_mut().window_mut(wid)?.app_mut().canvas_dialogs.close_dialog();

    // Rebind `ui.analyze.open` back to `Alt+P` and test Alt binding directly without primary_cmd.
    native.runtime_mut().window_mut(wid)?.app_mut().shortcuts.set_shortcut("ui.analyze.open", "Alt+P")?;
    step_native_frame(
        &mut native,
        &ctx,
        vec![egui::Event::Key { key: egui::Key::P, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT }],
    )?;
    assert!(matches!(native.runtime().window(wid)?.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
    native.runtime_mut().window_mut(wid)?.app_mut().canvas_dialogs.close_dialog();

    // Cmd+0 on zoomed canvas with ui_scale == 1.0 executes `ui.zoom.fitSheet`.
    native.runtime_mut().window_mut(wid)?.app_mut().canvas_dialogs.canvas.set_zoom(2.5);
    step_native_frame(
        &mut native,
        &ctx,
        vec![egui::Event::Key { key: egui::Key::Num0, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods }],
    )?;
    assert!((native.runtime().window(wid)?.app().canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);

    // Cmd+Plus bumps ui_scale to 1.1; then Cmd+0 with unzoomed canvas resets ui_scale back to 1.0.
    step_native_frame(
        &mut native,
        &ctx,
        vec![egui::Event::Key { key: egui::Key::Plus, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods }],
    )?;
    assert!((native.runtime().window(wid)?.app().ui_scale() - 1.1).abs() < 1e-4);
    step_native_frame(
        &mut native,
        &ctx,
        vec![egui::Event::Key { key: egui::Key::Num0, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods }],
    )?;
    assert!((native.runtime().window(wid)?.app().ui_scale() - 1.0).abs() < 1e-4);
    Ok(())
}

fn dispatch_via_bridge(bridge: &LiveControlBridge, req: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let mut raw = serde_json::to_vec(req)?;
    raw.push(b'\n');
    let out = bridge.dispatch_frame(raw, true).ok_or("bridge channel closed")?;
    let parsed: Value = serde_json::from_slice(&out.response_bytes)?;
    Ok(parsed)
}

#[test]
fn fa_sec19_05_pointer_gesture_validation_fifo_and_deferred_quit() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, ..DesktopHostConfig::default() });
    let sess = build_anova_bracket_session()?;
    let wid = rt.open_window_with_session(sess, ViewportMetrics::new(1280.0, 800.0, 1.0), false)?;
    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth);

    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut native = CurveBookNativeApp::new(rt, wid, None).with_live_control(channel);
    step_native_frame(&mut native, &ctx, Vec::new())?;

    // Run client worker sending validation checks + FIFO gesture + deferred quit.
    let worker = std::thread::spawn(move || -> Result<(Vec<Value>, Value, Value), String> {
        let mut errors = Vec::new();
        for bad in [
            json!({ "id": "missing-target", "method": "ui.pointer_gesture", "params": { "action": "click" } }),
            json!({ "id": "partial-xy", "method": "ui.pointer_gesture", "params": { "action": "click", "x": 120.0 } }),
            json!({ "id": "ambiguous", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": "toolbar.analyze", "x": 10.0, "y": 10.0 } }),
            json!({ "id": "unknown-wid", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": "no.such.widget" } }),
            json!({ "id": "bad-action", "method": "ui.pointer_gesture", "params": { "action": "teleport", "x": 50.0, "y": 50.0 } }),
        ] {
            let resp = dispatch_via_bridge(&bridge, &bad).map_err(|e| e.to_string())?;
            errors.push(resp);
        }

        // Send a valid widget click followed immediately by a deferred command & quit.
        let b2 = bridge.clone();
        let h_click = std::thread::spawn(move || {
            dispatch_via_bridge(
                &b2,
                &json!({ "id": "fifo-1", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": "toolbar.analyze" } }),
            )
            .map_err(|e| e.to_string())
        });
        std::thread::sleep(Duration::from_millis(15));
        let b3 = bridge.clone();
        let h_quit = std::thread::spawn(move || {
            dispatch_via_bridge(&b3, &json!({ "id": "fifo-2-quit", "method": "app.quit", "params": {} })).map_err(|e| e.to_string())
        });

        let r_click = h_click.join().map_err(|_| String::from("click thread panicked"))??;
        let r_quit = h_quit.join().map_err(|_| String::from("quit thread panicked"))??;
        Ok((errors, r_click, r_quit))
    });

    for _ in 0..24 {
        step_native_frame(&mut native, &ctx, Vec::new())?;
        if native.close_confirmed() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let (errs, click_resp, quit_resp) = worker.join().map_err(|_| "worker panicked")??;
    for err in &errs {
        assert_eq!(err["ok"], json!(false), "expected gesture error reply, got {err}");
    }
    assert_eq!(errs[3]["error_detail"]["code"], json!("not_found"));
    assert_eq!(click_resp["ok"], json!(true), "valid gesture failed: {click_resp}");
    assert_eq!(quit_resp["ok"], json!(true), "deferred quit failed: {quit_resp}");
    assert!(native.close_confirmed(), "deferred app.quit must propagate close_confirmed");
    Ok(())
}

#[test]
fn fa_sec19_06_paste_intent_executes_cleanly_via_session() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute(
        "sheet.createData",
        json!({ "sheet_id": "ds-paste", "title": "Paste", "kind": "column", "cols": 2, "rows": 4, "auto_graph": false }),
    )?;
    let mut app = UiApp::from_session(session.clone());
    app.grid.tool.decimal_separator = ',';
    app.grid.tool.select_single(CellCoord::new(0, 0, 0).into());
    let intent = app.grid.tool.emit_paste_intent(&SheetId::new("ds-paste"), "1,5\t2,5*\n3,5\t4,5", false, true)?;
    assert_eq!(intent.command_id, "table.paste");
    assert!(intent.params.get("cells").is_none());
    assert!(intent.params.get("parse_asterisk_exclusions").is_none());
    assert_eq!(intent.params["filter"]["parse_asterisk_exclusions"], json!(true));

    session.execute(&intent.command_id, intent.params)?;
    let ds = session.document().data_sheet(&SheetId::new("ds-paste")).ok_or("missing ds-paste")?;
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)), Some(&Cell::number(1.5)));
    assert_eq!(ds.cell(CellCoord::new(0, 1, 0)), Some(&Cell::excluded_number(2.5)));
    Ok(())
}

struct DelayedSharedWriter {
    sink: Arc<Mutex<Vec<u8>>>,
    flushed: Arc<AtomicBool>,
    delay_on_second_write: bool,
    writes: usize,
}

impl Write for DelayedSharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.delay_on_second_write && self.writes >= 2 {
            std::thread::sleep(Duration::from_millis(60));
        }
        let mut guard = self.sink.lock().map_err(|_| io::Error::other("poisoned sink"))?;
        guard.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.delay_on_second_write && self.writes >= 2 {
            self.flushed.store(true, Ordering::Release);
        }
        Ok(())
    }
}

#[test]
fn fa_sec19_07_pathless_or_post_mutated_save_and_live_quit_drain() -> Result<(), Box<dyn std::error::Error>> {
    // 1. LiveControlServerGuard::stop drains active client write/flush before returning.
    let auth = ControlAuth::generate()?;
    let tok = auth.as_str().to_owned();
    let (bridge, channel) = LiveControlBridge::new(auth);
    let (accept_tx, accept_rx) = mpsc::channel::<Vec<u8>>();
    let accept_rx = Mutex::named("audit_p8_accept_rx", accept_rx);
    let sink = Arc::new(Mutex::named("audit_p8_sink", Vec::<u8>::new()));
    let flushed = Arc::new(AtomicBool::new(false));
    let sink_client = Arc::clone(&sink);
    let flushed_client = Arc::clone(&flushed);

    let mut guard = spawn_live_control_accept_loop(
        bridge,
        move || {
            let rx = accept_rx.lock().map_err(|_| io::Error::other("poisoned accept_rx"))?;
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(bytes) => Ok(bytes),
                Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(io::ErrorKind::WouldBlock, "idle")),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed")),
            }
        },
        move |wire_bytes, bridge_ref| {
            let mut reader = io::BufReader::new(Cursor::new(wire_bytes));
            let mut writer =
                DelayedSharedWriter { sink: Arc::clone(&sink_client), flushed: Arc::clone(&flushed_client), delay_on_second_write: true, writes: 0 };
            serve_live_control_io(&mut reader, &mut writer, bridge_ref, |_| {})
        },
    )?;

    let wire = format!("{{\"id\":1,\"method\":\"auth\",\"params\":{{\"token\":\"{tok}\"}}}}\n{{\"id\":2,\"method\":\"app.quit\",\"params\":{{}}}}\n");
    accept_tx.send(wire.into_bytes())?;

    while let Ok(req) = channel.request_rx.recv_timeout(Duration::from_secs(3)) {
        let txt = String::from_utf8_lossy(&req.raw_bytes);
        if txt.contains("\"app.quit\"") {
            let mut bytes = br#"{"id":2,"ok":true,"result":{"quit":true}}"#.to_vec();
            bytes.push(b'\n');
            let _ = req.reply_tx.send(Ok(curvebook::desktop_host::LiveControlFrameOutput {
                response_bytes: bytes,
                quit: true,
                close_connection: true,
                auth_succeeded: None,
            }));
            break;
        }
        let mut bytes = br#"{"id":1,"ok":true,"result":{"authenticated":true}}"#.to_vec();
        bytes.push(b'\n');
        let _ = req.reply_tx.send(Ok(curvebook::desktop_host::LiveControlFrameOutput {
            response_bytes: bytes,
            quit: false,
            close_connection: false,
            auth_succeeded: Some(true),
        }));
    }
    drop(channel);
    guard.stop();

    assert!(flushed.load(Ordering::Acquire), "LiveControlServerGuard::stop must drain delayed quit reply flush");
    let written_str = String::from_utf8(sink.lock().map_err(|_| "poisoned sink")?.clone())?;
    let reply_lines: Vec<Value> = written_str.lines().map(serde_json::from_str).collect::<Result<_, _>>()?;
    assert_eq!(reply_lines.len(), 2);
    assert_eq!(reply_lines[1]["id"], json!(2));
    assert_eq!(reply_lines[1]["ok"], json!(true));

    // 2. control batch where `project.save` leaves session dirty does NOT clear dirty_commands.
    let save_path = unique_temp_path("journal-save.curvebook");
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let wid = rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
    let batch_req = json!({
        "id": 99,
        "method": "command.batch",
        "params": {
            "commands": [
                { "command": "sheet.createData", "params": { "sheet_id": "d1", "title": "First", "kind": "column" } },
                { "command": "project.save", "params": { "path": save_path.to_string_lossy(), "unpacked": false } },
                { "command": "sheet.createData", "params": { "sheet_id": "d2", "title": "Second", "kind": "column" } }
            ]
        }
    });
    let mut raw = serde_json::to_vec(&batch_req)?;
    raw.push(b'\n');
    let auth2 = ControlAuth::generate()?;
    let out = rt.window_mut(wid)?.dispatch_live_control_frame_with_hook(&raw, &auth2, true, |_, _| Ok(()))?;
    let resp: Value = serde_json::from_slice(&out.response_bytes)?;
    assert_eq!(resp["ok"], json!(true), "batch failed: {resp}");
    assert!(rt.window(wid)?.has_unsaved_changes());
    assert!(rt.window(wid)?.dirty_commands().contains(&String::from("sheet.createData")));
    let _ = std::fs::remove_file(&save_path);
    Ok(())
}

#[test]
fn fa_sec19_08_09_10_x1_06_unified_fixes() -> Result<(), Box<dyn std::error::Error>> {
    // FA-SEC19-08: Shared MAX_ZOOM (32.0) across UiApp canvas & PanZoomInertiaState.
    let mut zoom_app = UiApp::new();
    zoom_app.canvas_dialogs.canvas.set_zoom(100.0);
    assert!((zoom_app.canvas_dialogs.canvas.zoom - 32.0).abs() < f64::EPSILON);
    zoom_app.canvas_dialogs.pan_zoom.set_zoom_percent(5000.0);
    assert!((zoom_app.canvas_dialogs.pan_zoom.zoom - 32.0).abs() < f64::EPSILON);

    // FA-SEC19-09: `open_document_from_files` marks CSV/TSV & `.cbtemplate` dirty, and FileDropped resets coasting.
    let csv_path = unique_temp_path("startup.csv");
    std::fs::write(&csv_path, "X,Y\n1,2\n3,4\n")?;
    let csv_sess = open_document_from_files(std::slice::from_ref(&csv_path))?;
    assert!(csv_sess.is_dirty(), "opening CSV must start with is_dirty() == true");
    let _ = std::fs::remove_file(&csv_path);

    let archive_path = unique_temp_path("drop-target.curvebook");
    let mut save_sess = Session::new();
    save_sess.execute("sheet.createData", json!({ "sheet_id": "ds-drop", "title": "Dropped", "kind": "xy", "auto_graph": false }))?;
    save_sess.execute("project.save", json!({ "path": archive_path.to_string_lossy(), "unpacked": false }))?;

    let tpl_path = unique_temp_path("startup.cbtemplate");
    save_sess.execute("project.saveTemplate", json!({ "path": tpl_path.to_string_lossy() }))?;
    let tpl_sess = open_document_from_files(std::slice::from_ref(&tpl_path))?;
    assert!(tpl_sess.is_dirty(), "opening .cbtemplate must start with is_dirty() == true");
    let _ = std::fs::remove_file(&tpl_path);

    // FA-X1-06 + FA-SEC19-09: policy, human_path_only, clipboard propagation across open + FileDropped, plus coasting reset.
    let policy = ExternalPathPolicy::reject_external();
    let stub_clip = Arc::new(StubPlatformClipboardAdapter::new());
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: true,
        path_policy: policy.clone(),
        human_path_only: true,
        clipboard_adapter: Some(stub_clip),
        ..DesktopHostConfig::default()
    });
    let wid = rt.open_window_with_session(Session::new(), ViewportMetrics::new(1024.0, 640.0, 1.0), false)?;
    assert_eq!(rt.window(wid)?.app().session().path_policy(), &policy);
    assert!(rt.window(wid)?.app().is_human_path_only());
    assert!(rt.window(wid)?.app().has_custom_clipboard_adapter());
    assert!(rt.window(wid)?.app().is_gui_only());

    rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: archive_path.clone() })?;
    assert_eq!(rt.window(wid)?.app().session().path_policy(), &policy);
    assert!(rt.window(wid)?.app().is_human_path_only());
    assert!(rt.window(wid)?.app().has_custom_clipboard_adapter());
    assert!(rt.window(wid)?.app().is_gui_only());
    let _ = std::fs::remove_file(&archive_path);

    // FA-SEC19-10: Localized app submenu labels & macOS sync gating under --in-window-menus.
    let en = localized_app_submenu_labels("en");
    let de = localized_app_submenu_labels("de");
    let fr = localized_app_submenu_labels("fr");
    assert_ne!(en.0, de.0);
    assert_ne!(en.1, de.1);
    assert_ne!(en.2, de.2);
    assert_ne!(en.2, fr.2);

    #[cfg(target_os = "macos")]
    {
        let mut host = curvebook::mac_menu::MacNativeMenuHost::new();
        let mut chrome_en = rt.window(wid)?.chrome_state();
        chrome_en.locale = String::from("en");
        host.sync_from_chrome(&chrome_en)?;
        assert_eq!(host.app_submenu_labels(), en);

        let mut chrome_de = chrome_en;
        chrome_de.locale = String::from("de");
        host.sync_from_chrome(&chrome_de)?;
        assert_eq!(host.active_locale(), "de");
        assert_eq!(host.app_submenu_labels(), de);
        assert!(!host.is_installed_for_nsapp());
    }
    Ok(())
}
