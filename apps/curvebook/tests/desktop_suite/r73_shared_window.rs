//! Integration tests for `R7.3` Checkbox 3: opt-in `--window` shared live desktop window
//! for `curvebook --control <port>`.
#![forbid(unsafe_code)]

use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook_doc::{CellCoord, SheetId};
use curvebook_engine::Session;
use curvebook_format::{JOURNAL_FILE_NAME, LOCKFILE_NAME, open_project_file};
use curvebook_ui_egui::{ActiveDialog, CONTROL_HTTP_FORBIDDEN_RESPONSE, ControlAuth, MAX_CONTROL_LINE_BYTES, ViewportMetrics};
use eframe::egui;
use serde_json::{Value, json};

use crate::common::{
    TempDirGuard, bind_loopback_probe_listener, drive_live_window_until_quit, new_egui_context, send_json_line, start_live_control_server_on_listener,
};

fn connect_rw(addr: SocketAddr) -> Result<(BufReader<TcpStream>, BufWriter<TcpStream>), String> {
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    Ok((r, BufWriter::new(s)))
}

fn step_window_frame(app: &mut CurveBookNativeApp, ctx: &egui::Context) {
    let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    app.drain_synthetic_events_into(&mut raw);
    let mut out = ctx.run_ui(raw, |ui| app.step_ui(ui));
    out.textures_delta.clear();
}

#[test]
fn shared_live_window_human_and_control_edits_render_repaint_and_persist() -> Result<(), Box<dyn std::error::Error>> {
    let Some((listener, addr)) = bind_loopback_probe_listener()? else {
        return Ok(());
    };
    let tmp = TempDirGuard::new("shared-live-session")?;
    let recovery_dir = tmp.child("recovery");
    let save_path = tmp.child("shared-study.curvebook");

    let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "r73-live-owner", 300_000, 1_000)?;
    let viewport = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        recovery_root: Some(recovery_dir.clone()),
        initial_viewport: viewport,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;

    rt.dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.rulers.toggle") })?;
    rt.window_mut(win_id)?
        .execute_and_journal("sheet.createData", json!({ "sheet_id": "human-ds", "title": "Human Table", "kind": "column", "cols": 1, "rows": 2 }))?;
    rt.window_mut(win_id)?.execute_and_journal("table.setCell", json!({ "sheet_id": "human-ds", "row": 0, "col": 0, "value": 11.25 }))?;

    let auth = ControlAuth::generate()?;
    let (mut server_guard, channel) = start_live_control_server_on_listener(listener, auth.clone())?;
    let ctx = new_egui_context();
    let repaint_count = Arc::new(AtomicUsize::new(0));
    let repaint_count_cb = Arc::clone(&repaint_count);
    ctx.set_request_repaint_callback(move |_| {
        repaint_count_cb.fetch_add(1, Ordering::Relaxed);
    });

    let mut app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);
    step_window_frame(&mut app, &ctx);
    let _ = app.runtime_mut().dispatch(DesktopHostEvent::RedrawRequested { window_id: win_id })?;
    let initial_frame_rgba = app.runtime().window(win_id)?.last_frame().ok_or("missing initial frame")?.rgba_bytes.clone();
    assert!(app.runtime().window(win_id)?.window_title().contains("Untitled *"));

    let repaints_before_client = repaint_count.load(Ordering::Relaxed);
    let token_owned = auth.as_str().to_owned();
    let save_path_str = save_path.display().to_string();
    let agent_stage_done = Arc::new(AtomicBool::new(false));
    let human_mid_done = Arc::new(AtomicBool::new(false));
    let (agent_stage_done_c, human_mid_done_c) = (Arc::clone(&agent_stage_done), Arc::clone(&human_mid_done));

    let client = std::thread::spawn(move || -> Result<(), String> {
        // 1. Unauthenticated ping rejected.
        {
            let (mut r, mut w) = connect_rw(addr)?;
            let resp = send_json_line(&mut w, &mut r, &json!({"id": "unauth", "method": "ping"})).map_err(|e| e.to_string())?;
            assert_eq!((&resp["ok"], &resp["error_detail"]["code"]), (&json!(false), &json!("unauthorized")));
        }

        // 2. HTTP request rejected immediately with 403 Forbidden and EOF.
        {
            let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            s.write_all(b"POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r\n{}").map_err(|e| e.to_string())?;
            s.flush().map_err(|e| e.to_string())?;
            let mut reply = String::new();
            s.read_to_string(&mut reply).map_err(|e| e.to_string())?;
            assert_eq!(reply, CONTROL_HTTP_FORBIDDEN_RESPONSE);
        }

        // 3. Authenticated session observes human edits and performs agent edits & fit.
        {
            let (mut r, mut w) = connect_rw(addr)?;
            let auth_ok = send_json_line(&mut w, &mut r, &json!({"id": "a0", "method": "auth", "params": {"token": token_owned}}))
                .map_err(|e| e.to_string())?;
            assert_eq!(auth_ok["ok"], json!(true));
            let state = send_json_line(&mut w, &mut r, &json!({"id": "a1", "method": "ui.state"})).map_err(|e| e.to_string())?;
            assert_eq!(state["result"]["rulers_visible"], json!(true));
            let inspect = send_json_line(&mut w, &mut r, &json!({"id": "a2", "method": "doc.inspect"})).map_err(|e| e.to_string())?;
            assert!(inspect["result"]["data_sheets"].as_array().is_some_and(|a| a.iter().any(|ds| ds["id"] == "human-ds")));

            for req in [
                json!({"id": "a3", "method": "sheet.createData", "params": {"sheet_id": "agent-xy", "graph_sheet_id": "agent-graph", "title": "Agent Curve", "kind": "xy", "cols": 1, "rows": 4, "auto_graph": true}}),
                json!({"id": "a4", "method": "table.setXCells", "params": {"sheet_id": "agent-xy", "values": [1.0, 2.0, 3.0, 4.0]}}),
                json!({"id": "a5", "method": "table.setCells", "params": {"sheet_id": "agent-xy", "rows": [[10.0], [20.0], [40.0], [80.0]]}}),
                json!({"id": "a6", "method": "fit.run", "params": {"source_sheet_id": "agent-xy", "sheet_id": "agent-fit", "kind": "linear"}}),
                json!({"id": "a7", "method": "ui.theme.set", "params": {"mode": "dark"}}),
            ] {
                let resp = send_json_line(&mut w, &mut r, &req).map_err(|e| e.to_string())?;
                assert_eq!(resp["ok"], json!(true), "request {req} failed: {resp}");
            }
        }

        agent_stage_done_c.store(true, Ordering::Release);
        let wait_human = Instant::now() + Duration::from_secs(5);
        while !human_mid_done_c.load(Ordering::Acquire) {
            if Instant::now() >= wait_human {
                return Err(String::from("timed out waiting for mid-session human edit"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        // 4. Separate per-request token connections observe mid-session human edit, preview/export, save, and quit.
        for (idx, req) in [
            json!({"id": "m1", "method": "doc.inspect", "token": token_owned}),
            json!({"id": "m2", "method": "render.preview", "token": token_owned, "params": {"sheet_id": "agent-graph", "width_px": 320, "height_px": 200}}),
            json!({"id": "m3", "method": "doc.export", "token": token_owned, "params": {"format": "svg", "sheet_id": "agent-graph"}}),
            json!({"id": "m4", "method": "ui.screenshot", "token": token_owned, "params": {"width_px": 320, "height_px": 200}}),
            json!({"id": "m5", "method": "engine.execute", "token": token_owned, "params": {"command_id": "project.save", "params": {"path": save_path_str}}}),
            json!({"id": "m6", "method": "ui.state", "token": token_owned}),
            json!({"id": "m7", "method": "app.quit", "token": token_owned}),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut r, mut w) = connect_rw(addr)?;
            let resp = send_json_line(&mut w, &mut r, &req).map_err(|e| e.to_string())?;
            assert_eq!(resp["ok"], json!(true), "mcp-style request[{idx}] failed: {resp}");
            match idx {
                0 => assert!(resp["result"]["data_sheets"].as_array().is_some_and(|s| s.iter().any(|ds| ds["id"] == "human-mid"))),
                1 | 3 => assert!(resp["result"]["data_base64"].as_str().is_some_and(|b| b.starts_with("iVBORw0K"))),
                2 => assert!(resp["result"]["data_base64"].as_str().is_some_and(|b| !b.is_empty())),
                5 => assert_eq!(resp["result"]["dirty"], json!(false)),
                6 => assert_eq!(resp["result"]["quit"], json!(true)),
                _ => {}
            }
        }
        Ok(())
    });

    let mid_deadline = Instant::now() + Duration::from_secs(5);
    while !agent_stage_done.load(Ordering::Acquire) && Instant::now() < mid_deadline {
        step_window_frame(&mut app, &ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(agent_stage_done.load(Ordering::Acquire), "agent stage did not complete in time");
    app.runtime_mut().window_mut(win_id)?.execute_and_journal(
        "sheet.createData",
        json!({ "sheet_id": "human-mid", "title": "Human Mid-Session", "kind": "column", "cols": 1, "rows": 2 }),
    )?;
    app.runtime_mut()
        .window_mut(win_id)?
        .execute_and_journal("table.setCell", json!({ "sheet_id": "human-mid", "row": 0, "col": 0, "value": 77.5 }))?;
    human_mid_done.store(true, Ordering::Release);

    let saw_close_cmd = drive_live_window_until_quit(&mut app, &ctx, Duration::from_secs(10));
    client.join().map_err(|_| "control client panicked")??;
    assert!(app.close_confirmed() && saw_close_cmd);
    assert!(repaint_count.load(Ordering::Relaxed) > repaints_before_client);

    let _ = app.runtime_mut().dispatch(DesktopHostEvent::RedrawRequested { window_id: win_id })?;
    let updated_frame_rgba = app.runtime().window(win_id)?.last_frame().ok_or("missing post-control frame")?.rgba_bytes.clone();
    assert_ne!(initial_frame_rgba, updated_frame_rgba);
    assert_eq!(app.runtime().window(win_id)?.window_title(), "shared-study.curvebook — CurveBook");
    assert!(!app.runtime().window(win_id)?.has_unsaved_changes());

    app.finish_exit();
    server_guard.stop();
    assert!(!recovery_dir.join(LOCKFILE_NAME).exists());

    let reopened = open_project_file(&save_path)?;
    for (sid, r, expected) in [("human-ds", 0, 11.25), ("human-mid", 0, 77.5), ("agent-xy", 3, 80.0)] {
        let ds = reopened.data_sheet(&SheetId::new(sid)).ok_or("missing saved sheet")?;
        assert_eq!(ds.cell(CellCoord::new(r, 0, 0)).and_then(|c| c.active_number()), Some(expected));
    }
    assert!(reopened.result_sheet(&SheetId::new("agent-fit")).is_some());
    assert!(reopened.graph_sheet(&SheetId::new("agent-graph")).is_some());
    Ok(())
}

#[test]
fn shared_live_window_shares_human_and_agent_state() -> Result<(), Box<dyn std::error::Error>> {
    let Some((listener, addr)) = bind_loopback_probe_listener()? else {
        return Ok(());
    };
    let viewport = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, initial_viewport: viewport, ..DesktopHostConfig::default() });
    let win_id = rt.open_window(None, viewport)?;
    rt.dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.rulers.toggle") })?;

    let auth = ControlAuth::generate()?;
    let (mut server_guard, channel) = start_live_control_server_on_listener(listener, auth.clone())?;
    let ctx = new_egui_context();
    let mut app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let token_owned = auth.as_str().to_owned();
    let client_done = Arc::new(AtomicBool::new(false));
    let client_done_bg = Arc::clone(&client_done);
    let client = std::thread::spawn(move || -> Result<(), String> {
        let (mut r, mut w) = connect_rw(addr)?;
        assert_eq!(
            send_json_line(&mut w, &mut r, &json!({"id": "g0", "method": "auth", "params": {"token": token_owned}})).map_err(|e| e.to_string())?["ok"],
            json!(true)
        );
        assert_eq!(send_json_line(&mut w, &mut r, &json!({"id": "g1", "method": "ping"})).map_err(|e| e.to_string())?["ok"], json!(true));
        let state = send_json_line(&mut w, &mut r, &json!({"id": "g2", "method": "ui.state"})).map_err(|e| e.to_string())?;
        assert_eq!(state["result"]["rulers_visible"], json!(true));

        let opened =
            send_json_line(&mut w, &mut r, &json!({"id": "g3", "method": "ui.invoke_menu", "params": {"path": "Analyze/Power and Sample Size"}}))
                .map_err(|e| e.to_string())?;
        assert_eq!(opened["ok"], json!(true), "ui.invoke_menu failed: {opened}");
        for step in 0..3 {
            let next = send_json_line(
                &mut w,
                &mut r,
                &json!({"id": format!("n{step}"), "method": "ui.pointer_gesture", "params": {"action": "click", "widget_id": "dialog.button.next"}}),
            )
            .map_err(|e| e.to_string())?;
            assert_eq!(next["ok"], json!(true));
        }
        assert_eq!(
            send_json_line(
                &mut w,
                &mut r,
                &json!({"id": "g4", "method": "ui.pointer_gesture", "params": {"action": "click", "widget_id": "dialog.button.create"}}),
            )
            .map_err(|e| e.to_string())?["ok"],
            json!(true)
        );
        let shot = send_json_line(&mut w, &mut r, &json!({"id": "g5", "method": "ui.screenshot", "params": {"target": "full_window"}}))
            .map_err(|e| e.to_string())?;
        assert_eq!(shot["ok"], json!(true));
        assert!(shot["result"]["data_base64"].as_str().is_some_and(|b| b.starts_with("iVBORw0K")));
        assert_eq!(send_json_line(&mut w, &mut r, &json!({"id": "g6", "method": "app.quit"})).map_err(|e| e.to_string())?["ok"], json!(true));
        client_done_bg.store(true, Ordering::Release);
        Ok(())
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while !client_done.load(Ordering::Acquire) && Instant::now() < deadline {
        step_window_frame(&mut app, &ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    client.join().map_err(|_| "live control client panicked")??;

    let live_ui = app.runtime().window(win_id)?.app();
    assert_eq!(live_ui.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(live_ui.document().total_sheet_count() >= 3);
    app.finish_exit();
    server_guard.stop();
    Ok(())
}

#[test]
fn shared_live_window_wal_recovery_and_human_close_unblocks_server() -> Result<(), Box<dyn std::error::Error>> {
    let Some((listener, addr)) = bind_loopback_probe_listener()? else {
        return Ok(());
    };
    let tmp = TempDirGuard::new("live-wal-and-human-close")?;
    let recovery_dir = tmp.child("recovery");
    let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "r73-wal-owner", 300_000, 1_000)?;
    let viewport = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        recovery_root: Some(recovery_dir.clone()),
        initial_viewport: viewport,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;

    let auth = ControlAuth::generate()?;
    let (mut server_guard, channel) = start_live_control_server_on_listener(listener, auth.clone())?;
    let ctx = new_egui_context();
    let mut app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let done_tx = Arc::new(AtomicBool::new(false));
    let done_rx = Arc::clone(&done_tx);
    let token_owned = auth.as_str().to_owned();
    let client = std::thread::spawn(move || -> Result<(), String> {
        let (mut r, mut w) = connect_rw(addr)?;
        for req in [
            json!({"id": "w1", "method": "sheet.createData", "token": token_owned, "params": {"sheet_id": "wal-win-1", "title": "WAL Live", "kind": "column", "cols": 1, "rows": 2}}),
            json!({"id": "w2", "method": "table.setCell", "token": token_owned, "params": {"sheet_id": "wal-win-1", "row": 0, "col": 0, "value": 99.5}}),
        ] {
            assert_eq!(send_json_line(&mut w, &mut r, &req).map_err(|e| e.to_string())?["ok"], json!(true));
        }
        done_rx.store(true, Ordering::Release);
        Ok(())
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while !done_tx.load(Ordering::Acquire) && Instant::now() < deadline {
        step_window_frame(&mut app, &ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    client.join().map_err(|_| "wal client panicked")??;
    assert!(recovery_dir.join(JOURNAL_FILE_NAME).is_file(), "live-window --control edits must flush immediately to ops.journal");

    app.finish_exit();
    assert!(server_guard.shutdown_flag().load(Ordering::Acquire), "human window close must signal control server shutdown");
    server_guard.stop();
    assert!(!recovery_dir.join(LOCKFILE_NAME).exists(), "human window close must release recovery lock");

    let (mut rec_session, rec_journal, _) = Session::open_dir_with_recovery(&recovery_dir, "verify-live-wal", 5_000, 2_000)?;
    let ds = rec_session.document().data_sheet(&SheetId::new("wal-win-1")).ok_or("missing WAL sheet after close")?;
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(99.5));
    let _ = rec_session.close_journal_cleanly(rec_journal)?;
    Ok(())
}

#[test]
fn live_control_server_rejects_oversized_frames_and_serves_concurrent_clients() -> Result<(), Box<dyn std::error::Error>> {
    let Some((listener, addr)) = bind_loopback_probe_listener()? else {
        return Ok(());
    };
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let win_id = rt.open_window(None, ViewportMetrics::new(960.0, 640.0, 1.0))?;
    let auth = ControlAuth::generate()?;
    let (mut server_guard, channel) = start_live_control_server_on_listener(listener, auth.clone())?;
    let ctx = new_egui_context();
    let mut app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let token_owned = auth.as_str().to_owned();
    let client = std::thread::spawn(move || -> Result<(), String> {
        // 1. Oversized frame (> MAX_CONTROL_LINE_BYTES) gets structured JSON error without crashing server.
        {
            let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut reader = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
            let prefix = format!("{{\"id\":\"big\",\"token\":\"{token_owned}\",\"method\":\"ping\",\"pad\":\"");
            let _ = s.write_all(prefix.as_bytes());
            let chunk = vec![b'a'; 64 * 1024];
            let mut sent = prefix.len();
            while sent <= MAX_CONTROL_LINE_BYTES {
                if let Err(err) = s.write_all(&chunk) {
                    if matches!(err.kind(), ErrorKind::BrokenPipe | ErrorKind::ConnectionReset) {
                        break;
                    }
                    return Err(err.to_string());
                }
                sent += chunk.len();
            }
            let _ = s.write_all(b"\"}\n");
            let _ = s.flush();
            let mut line = String::new();
            reader.read_line(&mut line).map_err(|e| e.to_string())?;
            let resp: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            assert_eq!(resp["ok"], json!(false));
            assert!(resp["error"].as_str().is_some_and(|e| e.contains("exceeds")));
        }

        // 2. Concurrent client stays responsive even while an earlier client holds an open idle connection.
        {
            let idle = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let (mut r, mut w) = connect_rw(addr)?;
            assert_eq!(
                send_json_line(&mut w, &mut r, &json!({"id": "c1", "method": "ping", "token": token_owned})).map_err(|e| e.to_string())?["ok"],
                json!(true)
            );
            let quit = send_json_line(&mut w, &mut r, &json!({"id": "c2", "method": "app.quit", "token": token_owned})).map_err(|e| e.to_string())?;
            assert_eq!(quit["result"]["quit"], json!(true));
            drop(idle);
        }
        Ok(())
    });

    let saw_close = drive_live_window_until_quit(&mut app, &ctx, Duration::from_secs(10));
    client.join().map_err(|_| "concurrent client panicked")??;
    assert!(saw_close);
    app.finish_exit();
    server_guard.stop();
    Ok(())
}

#[test]
fn live_control_frame_handles_invalid_utf8_auth_downgrade_and_http() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let win_id = rt.open_window(None, ViewportMetrics::new(800.0, 600.0, 1.0))?;
    let win = rt.window_mut(win_id)?;
    let auth = ControlAuth::from_token("r73-edge-case-token")?;

    let empty_out = win.dispatch_live_control_frame_with_hook(b"   \n", &auth, false, |_, _| Ok(()))?;
    assert!(empty_out.response_bytes.is_empty() && !empty_out.close_connection);

    let bad_utf8 = win.dispatch_live_control_frame_with_hook(b"\xff\xfe{\"id\":1}\n", &auth, true, |_, _| Ok(()))?;
    let utf8_val: Value = serde_json::from_slice(&bad_utf8.response_bytes)?;
    assert_eq!(utf8_val["ok"], json!(false));
    assert!(utf8_val["error"].as_str().is_some_and(|e| e.contains("UTF-8")));

    let auth_ok = win.dispatch_live_control_frame_with_hook(
        format!("{{\"id\":1,\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n", auth.as_str()).as_bytes(),
        &auth,
        false,
        |_, _| Ok(()),
    )?;
    assert_eq!(auth_ok.auth_succeeded, Some(true));

    let bad_json: Value =
        serde_json::from_slice(&win.dispatch_live_control_frame_with_hook(b"{broken-json\n", &auth, true, |_, _| Ok(()))?.response_bytes)?;
    assert!(bad_json["error"].as_str().is_some_and(|e| e.contains("invalid control JSON")));

    let auth_bad = win.dispatch_live_control_frame_with_hook(
        b"{\"id\":2,\"method\":\"auth\",\"params\":{\"token\":\"wrong-secret\"}}\n",
        &auth,
        true,
        |_, _| Ok(()),
    )?;
    assert_eq!(auth_bad.auth_succeeded, Some(false));

    let rejected: Value = serde_json::from_slice(
        &win.dispatch_live_control_frame_with_hook(b"{\"id\":3,\"method\":\"ping\"}\n", &auth, false, |_, _| Ok(()))?.response_bytes,
    )?;
    assert_eq!(rejected["error_detail"]["code"], json!("unauthorized"));

    let http_out = win.dispatch_live_control_frame_with_hook(b"GET / HTTP/1.1\r\n", &auth, true, |_, _| Ok(()))?;
    assert_eq!(String::from_utf8(http_out.response_bytes)?, CONTROL_HTTP_FORBIDDEN_RESPONSE);
    assert!(http_out.close_connection && !http_out.quit);
    Ok(())
}
