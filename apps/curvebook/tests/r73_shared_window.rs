//! Integration tests for `R7.3` Checkbox 3: opt-in `--window` shared live desktop window
//! for `curvebook --control <port>`.
#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime};
use curvebook::gui_window::{CurveBookNativeApp, LiveControlChannel, build_egui_font_definitions};
use curvebook::live_control::{LiveControlBridge, LiveControlServerGuard, serve_live_control_io, spawn_live_control_accept_loop};
use curvebook_doc::{CellCoord, SheetId};
use curvebook_engine::Session;
use curvebook_format::{JOURNAL_FILE_NAME, LOCKFILE_NAME, open_project_file};
use curvebook_ui_egui::control_io::CONTROL_CLIENT_IDLE_TIMEOUT;
use curvebook_ui_egui::{
    ActiveDialog, CONTROL_HTTP_FORBIDDEN_RESPONSE, ControlAuth, MAX_CONTROL_LINE_BYTES, ViewportMetrics, bind_control_tcp_listener,
    validate_loopback_addr,
};
use serde_json::{Value, json};

static R73_TEST_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = R73_TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-r73-win-{label}-{}-{seq}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bind_loopback_probe_listener() -> Result<Option<(TcpListener, SocketAddr)>, Box<dyn std::error::Error>> {
    let listener = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(None),
        Err(err) => return Err(Box::new(err)),
    };
    let addr = listener.local_addr()?;
    match TcpStream::connect_timeout(&addr, Duration::from_millis(250)) {
        Ok(probe) => {
            drop(probe);
            let _ = listener.accept();
            Ok(Some((listener, addr)))
        }
        Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => Ok(None),
        Err(err) => Err(Box::new(err)),
    }
}

fn reserve_ephemeral_port() -> Result<Option<u16>, Box<dyn std::error::Error>> {
    let Some((listener, addr)) = bind_loopback_probe_listener()? else {
        return Ok(None);
    };
    let port = addr.port();
    drop(listener);
    Ok(Some(port))
}

fn wait_for_token_file(child: &mut Child, token_path: &Path, timeout: Duration) -> Result<String, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    while !token_path.exists() {
        if let Some(status) = child.try_wait()? {
            return Err(format!("curvebook --control exited early with {status}").into());
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for control token file `{}`", token_path.display()).into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(fs::read_to_string(token_path)?)
}

fn send_json_line(
    writer: &mut BufWriter<TcpStream>,
    reader: &mut BufReader<TcpStream>,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    writer.write_all(serde_json::to_string(payload)?.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

fn start_live_control_server_on_listener(
    listener: TcpListener,
    auth: ControlAuth,
) -> Result<(LiveControlServerGuard, LiveControlChannel), Box<dyn std::error::Error>> {
    listener.set_nonblocking(true)?;
    let (bridge, channel) = LiveControlBridge::new(auth);
    let guard = spawn_live_control_accept_loop(
        bridge,
        move || listener.accept().map(|(stream, _)| stream),
        |mut stream: TcpStream, bridge: &LiveControlBridge| {
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_millis(100)))?;
            stream.set_write_timeout(Some(CONTROL_CLIENT_IDLE_TIMEOUT))?;
            let mut reader = BufReader::new(stream.try_clone()?);
            serve_live_control_io(&mut reader, &mut stream, bridge, |tcp| {
                let _ = tcp.shutdown(Shutdown::Write);
                let _ = tcp.set_read_timeout(Some(Duration::from_millis(20)));
                let mut discard = [0u8; 8192];
                let mut drained = 0usize;
                let deadline = Instant::now() + Duration::from_millis(50);
                while drained < 2 * 1024 * 1024 && Instant::now() < deadline {
                    match tcp.read(&mut discard) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => drained += n,
                    }
                }
            })
        },
    )?;
    Ok((guard, channel))
}

fn drive_live_window_until_quit(app: &mut CurveBookNativeApp, ctx: &egui::Context, timeout: Duration) -> bool {
    let mut saw_close_cmd = false;
    let deadline = Instant::now() + timeout;
    while !app.close_confirmed() && Instant::now() < deadline {
        let raw_input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(raw_input, |ui| app.step_ui(ui));
        output.textures_delta.clear();
        if output.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::Close)) {
            saw_close_cmd = true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    saw_close_cmd
}

#[test]
fn window_flag_help_and_conflicting_headless_flags() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_curvebook");

    let help = Command::new(bin).arg("--help").output()?;
    assert!(help.status.success(), "--help failed: {}", String::from_utf8_lossy(&help.stderr));
    let help_text = String::from_utf8(help.stdout)?;
    assert!(help_text.contains("--window"), "missing --window in --help output:\n{help_text}");
    assert!(help_text.contains("--control <port>"), "missing --control <port> in --help output:\n{help_text}");
    assert!(help_text.contains("CURVEBOOK_CONTROL_PORT"), "missing CURVEBOOK_CONTROL_PORT in --help output:\n{help_text}");

    let conflict = Command::new(bin).args(["--control", "9125", "--window", "--headless-once"]).output()?;
    assert!(!conflict.status.success(), "--control + --window + --headless-once must fail");
    let conflict_err = String::from_utf8_lossy(&conflict.stderr);
    assert!(conflict_err.contains("--window cannot be combined with --headless-once"), "unexpected error: {conflict_err}");

    let window_headless = Command::new(bin).args(["--window", "--headless-once"]).output()?;
    assert!(!window_headless.status.success(), "--window + --headless-once must fail");

    let env_conflict = Command::new(bin).env("CURVEBOOK_CONTROL_PORT", "9125").args(["--window", "--headless-once"]).output()?;
    assert!(!env_conflict.status.success(), "CURVEBOOK_CONTROL_PORT + --window + --headless-once must fail");

    let zero_port = Command::new(bin).args(["--control", "0", "--window"]).output()?;
    assert!(!zero_port.status.success(), "--control 0 --window must fail");

    Ok(())
}

#[test]
fn omitting_window_preserves_headless_control_behavior() -> Result<(), Box<dyn std::error::Error>> {
    let Some(port) = reserve_ephemeral_port()? else {
        return Ok(());
    };
    let tmp = TempDirGuard::new("headless-preserved")?;
    let portable_dir = tmp.path.join("portable");
    fs::create_dir_all(&portable_dir)?;
    let token_path = portable_dir.join("curvebook-data").join(format!("control-{port}.token"));
    let save_path = tmp.path.join("headless-saved.curvebook");

    let bin = env!("CARGO_BIN_EXE_curvebook");
    let child =
        Command::new(bin).env("HOME", &tmp.path).args(["--portable-dir"]).arg(&portable_dir).args(["--control", &port.to_string()]).spawn()?;
    let mut guard = ChildGuard(child);

    let token = wait_for_token_file(&mut guard.0, &token_path, Duration::from_secs(10))?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(s) => s,
        Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => return Ok(()),
        Err(err) => return Err(Box::new(err)),
    };
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = BufWriter::new(stream);

    assert_eq!(send_json_line(&mut writer, &mut reader, &json!({"id": "h0", "method": "auth", "params": {"token": token}}))?["ok"], json!(true));
    assert_eq!(send_json_line(&mut writer, &mut reader, &json!({"id": "h1", "method": "ping"}))?["ok"], json!(true));
    assert_eq!(
        send_json_line(
            &mut writer,
            &mut reader,
            &json!({"id": "h2", "method": "sheet.createData", "params": {"sheet_id": "hl-1", "title": "Headless Table", "kind": "column", "cols": 1, "rows": 2}})
        )?["ok"],
        json!(true)
    );
    assert_eq!(
        send_json_line(
            &mut writer,
            &mut reader,
            &json!({"id": "h3", "method": "table.setCell", "params": {"sheet_id": "hl-1", "row": 0, "col": 0, "value": 42.5}})
        )?["ok"],
        json!(true)
    );
    assert_eq!(
        send_json_line(
            &mut writer,
            &mut reader,
            &json!({"id": "h4", "method": "project.save", "params": {"path": save_path.display().to_string()}})
        )?["ok"],
        json!(true)
    );
    let quit = send_json_line(&mut writer, &mut reader, &json!({"id": "h5", "method": "app.quit"}))?;
    assert_eq!(quit["result"]["quit"], json!(true));

    let status = guard.0.wait()?;
    assert!(status.success(), "headless curvebook --control exited with {status}");
    assert!(!token_path.exists(), "token file must be removed on exit");

    let doc = open_project_file(&save_path)?;
    let ds = doc.data_sheet(&SheetId::new("hl-1")).ok_or("missing saved headless sheet")?;
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(42.5));

    // Also verify CURVEBOOK_CONTROL_PORT without --window runs headlessly.
    let Some(env_port) = reserve_ephemeral_port()? else {
        return Ok(());
    };
    let env_token_path = portable_dir.join("curvebook-data").join(format!("control-{env_port}.token"));
    let env_child = Command::new(bin)
        .env("HOME", &tmp.path)
        .env("CURVEBOOK_CONTROL_PORT", env_port.to_string())
        .args(["--portable-dir"])
        .arg(&portable_dir)
        .spawn()?;
    let mut env_guard = ChildGuard(env_child);
    let env_token = wait_for_token_file(&mut env_guard.0, &env_token_path, Duration::from_secs(10))?;
    let env_stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], env_port)), Duration::from_secs(2))?;
    env_stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut env_reader = BufReader::new(env_stream.try_clone()?);
    let mut env_writer = BufWriter::new(env_stream);
    let env_quit = send_json_line(&mut env_writer, &mut env_reader, &json!({"id": "eq", "method": "app.quit", "token": env_token}))?;
    assert_eq!(env_quit["result"]["quit"], json!(true));
    assert!(env_guard.0.wait()?.success());

    Ok(())
}

#[test]
fn loopback_only_enforcement_rejects_non_loopback_addresses() -> Result<(), Box<dyn std::error::Error>> {
    for bad in ["0.0.0.0:9125", "192.168.1.42:9125", "[::]:9125"] {
        let addr: SocketAddr = bad.parse()?;
        assert!(validate_loopback_addr(addr).is_err(), "expected validate_loopback_addr to reject {bad}");
        assert!(bind_control_tcp_listener(addr).is_err(), "expected bind_control_tcp_listener to reject {bad}");
    }
    for good in ["127.0.0.1:0", "[::1]:0"] {
        let addr: SocketAddr = good.parse()?;
        assert!(validate_loopback_addr(addr).is_ok(), "expected validate_loopback_addr to accept {good}");
    }
    Ok(())
}

#[test]
fn shared_live_window_human_and_control_edits_render_repaint_and_persist() -> Result<(), Box<dyn std::error::Error>> {
    let Some((listener, addr)) = bind_loopback_probe_listener()? else {
        return Ok(());
    };
    let tmp = TempDirGuard::new("shared-live-session")?;
    let recovery_dir = tmp.path.join("recovery");
    let save_path = tmp.path.join("shared-study.curvebook");

    let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "r73-live-owner", 300_000, 1_000)?;
    let viewport = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: false,
        ui_backend: curvebook_ui_egui::egui_shell::UiBackend::Egui,
        locale_override: None,
        theme_override: None,
        recovery_root: Some(recovery_dir.clone()),
        initial_viewport: viewport,
    });
    let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;

    // Human creates an initial sheet and toggles rulers inside the live window session.
    rt.dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.rulers.toggle") })?;
    rt.window_mut(win_id)?
        .execute_and_journal("sheet.createData", json!({ "sheet_id": "human-ds", "title": "Human Table", "kind": "column", "cols": 1, "rows": 2 }))?;
    rt.window_mut(win_id)?.execute_and_journal("table.setCell", json!({ "sheet_id": "human-ds", "row": 0, "col": 0, "value": 11.25 }))?;

    let auth = ControlAuth::generate()?;
    let (mut server_guard, channel) = start_live_control_server_on_listener(listener, auth.clone())?;

    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let repaint_count = Arc::new(AtomicUsize::new(0));
    let repaint_count_cb = Arc::clone(&repaint_count);
    ctx.set_request_repaint_callback(move |_info| {
        repaint_count_cb.fetch_add(1, Ordering::Relaxed);
    });

    let mut app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);
    // Run Frame 1 before control client connects so initial RGBA frame and window title are populated.
    let mut out0 = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0))), ..Default::default() },
        |ui| app.step_ui(ui),
    );
    out0.textures_delta.clear();
    let _ = app.runtime_mut().dispatch(DesktopHostEvent::RedrawRequested { window_id: win_id })?;
    let initial_frame_rgba = app.runtime().window(win_id)?.last_frame().ok_or("missing initial frame")?.rgba_bytes.clone();
    let initial_title = app.runtime().window(win_id)?.window_title();
    assert!(initial_title.contains("Untitled *"));

    let repaints_before_client = repaint_count.load(Ordering::Relaxed);
    let token_owned = auth.as_str().to_owned();
    let save_path_str = save_path.display().to_string();
    let agent_stage_done = Arc::new(AtomicBool::new(false));
    let agent_stage_done_c = Arc::clone(&agent_stage_done);
    let human_mid_done = Arc::new(AtomicBool::new(false));
    let human_mid_done_c = Arc::clone(&human_mid_done);

    let client = std::thread::spawn(move || -> Result<(), String> {
        // 1. Unauthenticated ping rejected.
        {
            let s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
            let mut w = BufWriter::new(s);
            let resp = send_json_line(&mut w, &mut r, &json!({"id": "unauth", "method": "ping"})).map_err(|e| e.to_string())?;
            assert_eq!(resp["ok"], json!(false));
            assert_eq!(resp["error_detail"]["code"], json!("unauthorized"));
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
            let s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
            let mut w = BufWriter::new(s);

            let auth_ok = send_json_line(&mut w, &mut r, &json!({"id": "a0", "method": "auth", "params": {"token": token_owned}}))
                .map_err(|e| e.to_string())?;
            assert_eq!(auth_ok["ok"], json!(true));

            // Subsequent commands without `"token"` reuse session auth across UI frames.
            let state = send_json_line(&mut w, &mut r, &json!({"id": "a1", "method": "ui.state"})).map_err(|e| e.to_string())?;
            assert_eq!(state["result"]["rulers_visible"], json!(true));

            let inspect = send_json_line(&mut w, &mut r, &json!({"id": "a2", "method": "doc.inspect"})).map_err(|e| e.to_string())?;
            let data_sheets = inspect["result"]["data_sheets"].as_array().ok_or("missing data_sheets")?;
            assert!(data_sheets.iter().any(|ds| ds["id"] == "human-ds"), "control client must see human-created sheet: {inspect}");

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

        // Wait for human to perform a mid-session edit inside the live window before saving.
        agent_stage_done_c.store(true, Ordering::Release);
        let wait_human = Instant::now() + Duration::from_secs(5);
        while !human_mid_done_c.load(Ordering::Acquire) {
            if Instant::now() >= wait_human {
                return Err(String::from("timed out waiting for mid-session human edit"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        // 4. Separate per-request token connections (like `curvebook-cli mcp --control-port`)
        //    observe the mid-session human edit, preview/export, save, and quit cleanly.
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
            let s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
            let mut w = BufWriter::new(s);
            let resp = send_json_line(&mut w, &mut r, &req).map_err(|e| e.to_string())?;
            assert_eq!(resp["ok"], json!(true), "mcp-style request[{idx}] failed: {resp}");
            if idx == 0 {
                let sheets = resp["result"]["data_sheets"].as_array().ok_or("missing data_sheets")?;
                assert!(sheets.iter().any(|ds| ds["id"] == "human-mid"), "control client must observe mid-session human sheet: {resp}");
            } else if idx == 1 || idx == 3 {
                assert!(resp["result"]["data_base64"].as_str().is_some_and(|b| b.starts_with("iVBORw0K")));
            } else if idx == 2 {
                assert!(resp["result"]["data_base64"].as_str().is_some_and(|b| !b.is_empty()));
            } else if idx == 5 {
                assert_eq!(resp["result"]["dirty"], json!(false), "engine.execute project.save must clear dirty state: {resp}");
            } else if idx == 6 {
                assert_eq!(resp["result"]["quit"], json!(true));
            }
        }

        Ok(())
    });

    // Step UI frames until agent finishes initial stage, then inject mid-session human edits, then drive until quit.
    let mid_deadline = Instant::now() + Duration::from_secs(5);
    while !agent_stage_done.load(Ordering::Acquire) && Instant::now() < mid_deadline {
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0))), ..Default::default() },
            |ui| app.step_ui(ui),
        );
        out.textures_delta.clear();
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
    assert!(app.close_confirmed(), "app.quit over --control must confirm close");
    assert!(saw_close_cmd, "app.quit must emit egui::ViewportCommand::Close");
    assert!(
        repaint_count.load(Ordering::Relaxed) > repaints_before_client,
        "incoming control requests must trigger egui::Context::request_repaint()"
    );

    let _ = app.runtime_mut().dispatch(DesktopHostEvent::RedrawRequested { window_id: win_id })?;
    let updated_frame_rgba = app.runtime().window(win_id)?.last_frame().ok_or("missing post-control frame")?.rgba_bytes.clone();
    assert_ne!(initial_frame_rgba, updated_frame_rgba, "live window framebuffer must repaint with control edits");
    let final_title = app.runtime().window(win_id)?.window_title();
    assert_eq!(final_title, "shared-study.curvebook — CurveBook", "saved window title must reflect saved file cleanly");
    assert!(!app.runtime().window(win_id)?.has_unsaved_changes(), "saved window must not report unsaved changes");

    app.finish_exit();
    server_guard.stop();
    assert!(!recovery_dir.join(LOCKFILE_NAME).exists(), "clean exit must release recovery lock");

    // Reopen saved file on disk and verify BOTH human (initial + mid-session) and --control edits survived persistence.
    let reopened = open_project_file(&save_path)?;
    let human_ds = reopened.data_sheet(&SheetId::new("human-ds")).ok_or("missing human-ds in saved file")?;
    assert_eq!(human_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(11.25));
    let human_mid_ds = reopened.data_sheet(&SheetId::new("human-mid")).ok_or("missing human-mid in saved file")?;
    assert_eq!(human_mid_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(77.5));
    let agent_ds = reopened.data_sheet(&SheetId::new("agent-xy")).ok_or("missing agent-xy in saved file")?;
    assert_eq!(agent_ds.cell(CellCoord::new(3, 0, 0)).and_then(|c| c.active_number()), Some(80.0));
    assert!(reopened.result_sheet(&SheetId::new("agent-fit")).is_some(), "missing fit result sheet in saved file");
    assert!(reopened.graph_sheet(&SheetId::new("agent-graph")).is_some(), "missing auto-generated graph sheet in saved file");

    Ok(())
}

#[test]
fn shared_live_window_enforces_gui_only_and_shares_human_and_agent_state() -> Result<(), Box<dyn std::error::Error>> {
    let Some((listener, addr)) = bind_loopback_probe_listener()? else {
        return Ok(());
    };
    let viewport = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: true,
        ui_backend: curvebook_ui_egui::egui_shell::UiBackend::Egui,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: viewport,
    });
    let win_id = rt.open_window(None, viewport)?;
    // Human toggles rulers in the live window while --gui-only is active.
    rt.dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.rulers.toggle") })?;

    let auth = ControlAuth::generate()?;
    let (mut server_guard, channel) = start_live_control_server_on_listener(listener, auth.clone())?;
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let token_owned = auth.as_str().to_owned();
    let client_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let client_done_bg = std::sync::Arc::clone(&client_done);
    let client = std::thread::spawn(move || -> Result<(), String> {
        let s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
        let mut w = BufWriter::new(s);

        let auth_resp =
            send_json_line(&mut w, &mut r, &json!({"id": "g0", "method": "auth", "params": {"token": token_owned}})).map_err(|e| e.to_string())?;
        assert_eq!(auth_resp["ok"], json!(true));

        let ping = send_json_line(&mut w, &mut r, &json!({"id": "g1", "method": "ping"})).map_err(|e| e.to_string())?;
        assert_eq!(ping["result"]["gui_only"], json!(true));

        let state = send_json_line(&mut w, &mut r, &json!({"id": "g2", "method": "ui.state"})).map_err(|e| e.to_string())?;
        assert_eq!(state["result"]["gui_only"], json!(true));
        assert_eq!(state["result"]["rulers_visible"], json!(true));

        // Direct non-GUI mutations and screenshot disk writes must be blocked with `gui_only_violation`.
        for (idx, blocked_req) in [
            json!({"id": "b1", "method": "engine.execute", "params": {"command_id": "sheet.createData", "params": {"kind": "xy"}}}),
            json!({"id": "b2", "method": "command.run", "params": {"command": "sheet.createData", "params": {"kind": "xy"}}}),
            json!({"id": "b3", "method": "command.batch", "params": {"commands": [{"command": "sheet.createData", "params": {"kind": "xy"}}]}}),
            json!({"id": "b4", "method": "sheet.createData", "params": {"kind": "xy", "title": "Bypass"}}),
            json!({"id": "b5", "method": "ui.theme.set", "params": {"mode": "dark"}}),
            json!({"id": "b6", "method": "ui.screenshot", "params": {"path": "forbidden-gui-only.png"}}),
            json!({"id": "b7", "method": "doc.inspect", "params": {}}),
            json!({"id": "b8", "method": "render.svg", "params": {}}),
            json!({"id": "b9", "method": "render.pdf", "params": {}}),
        ]
        .into_iter()
        .enumerate()
        {
            let resp = send_json_line(&mut w, &mut r, &blocked_req).map_err(|e| e.to_string())?;
            assert_eq!(resp["ok"], json!(false), "blocked[{idx}] unexpectedly succeeded: {resp}");
            assert_eq!(resp["error_detail"]["code"], json!("gui_only_violation"), "blocked[{idx}] wrong code: {resp}");
        }

        // Interactive GUI actions work on the shared live window under --gui-only.
        let opened =
            send_json_line(&mut w, &mut r, &json!({"id": "g3", "method": "ui.invoke_menu", "params": {"path": "Analyze/Power and Sample Size..."}}))
                .map_err(|e| e.to_string())?;
        assert_eq!(opened["ok"], json!(true), "ui.invoke_menu failed: {opened}");

        for step in 0..3 {
            let next = send_json_line(&mut w, &mut r, &json!({"id": format!("n{step}"), "method": "ui.answer_dialog", "params": {"action": "next"}}))
                .map_err(|e| e.to_string())?;
            assert_eq!(next["ok"], json!(true));
        }
        let created = send_json_line(&mut w, &mut r, &json!({"id": "g4", "method": "ui.answer_dialog", "params": {"action": "create"}}))
            .map_err(|e| e.to_string())?;
        assert_eq!(created["ok"], json!(true));

        let shot = send_json_line(&mut w, &mut r, &json!({"id": "g5", "method": "ui.screenshot", "params": {"target": "full_window"}}))
            .map_err(|e| e.to_string())?;
        assert_eq!(shot["ok"], json!(true));
        assert!(shot["result"]["data_base64"].as_str().is_some_and(|b| b.starts_with("iVBORw0K")));

        // Under --gui-only (`R2-S10-04`), `app.quit` is also rejected via control channel.
        let quit = send_json_line(&mut w, &mut r, &json!({"id": "g6", "method": "app.quit"})).map_err(|e| e.to_string())?;
        assert_eq!(quit["ok"], json!(false), "app.quit must be blocked under --gui-only: {quit}");
        assert_eq!(quit["error_detail"]["code"], json!("gui_only_violation"));
        client_done_bg.store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while !client_done.load(std::sync::atomic::Ordering::Acquire) && Instant::now() < deadline {
        let raw_input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(raw_input, |ui| app.step_ui(ui));
        output.textures_delta.clear();
        std::thread::sleep(Duration::from_millis(5));
    }
    client.join().map_err(|_| "gui-only client panicked")??;

    let live_ui = app.runtime().window(win_id)?.app();
    assert_eq!(live_ui.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(live_ui.document().total_sheet_count() >= 3, "expected power wizard sheets in live window session");

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
    let recovery_dir = tmp.path.join("recovery");
    let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "r73-wal-owner", 300_000, 1_000)?;
    let viewport = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: false,
        ui_backend: curvebook_ui_egui::egui_shell::UiBackend::Egui,
        locale_override: None,
        theme_override: None,
        recovery_root: Some(recovery_dir.clone()),
        initial_viewport: viewport,
    });
    let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;

    let auth = ControlAuth::generate()?;
    let (mut server_guard, channel) = start_live_control_server_on_listener(listener, auth.clone())?;
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let done_tx = Arc::new(AtomicBool::new(false));
    let done_rx = Arc::clone(&done_tx);
    let token_owned = auth.as_str().to_owned();
    let client = std::thread::spawn(move || -> Result<(), String> {
        let s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
        let mut w = BufWriter::new(s);
        let resp = send_json_line(
            &mut w,
            &mut r,
            &json!({"id": "w1", "method": "sheet.createData", "token": token_owned, "params": {"sheet_id": "wal-win-1", "title": "WAL Live", "kind": "column", "cols": 1, "rows": 2}}),
        )
        .map_err(|e| e.to_string())?;
        assert_eq!(resp["ok"], json!(true));
        let resp2 = send_json_line(
            &mut w,
            &mut r,
            &json!({"id": "w2", "method": "table.setCell", "token": token_owned, "params": {"sheet_id": "wal-win-1", "row": 0, "col": 0, "value": 99.5}}),
        )
        .map_err(|e| e.to_string())?;
        assert_eq!(resp2["ok"], json!(true));
        done_rx.store(true, Ordering::Release);
        Ok(())
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while !done_tx.load(Ordering::Acquire) && Instant::now() < deadline {
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0))), ..Default::default() },
            |ui| app.step_ui(ui),
        );
        out.textures_delta.clear();
        std::thread::sleep(Duration::from_millis(5));
    }
    client.join().map_err(|_| "wal client panicked")??;
    assert!(recovery_dir.join(JOURNAL_FILE_NAME).is_file(), "live-window --control edits must flush immediately to ops.journal");

    // Simulate human closing the live window without sending `app.quit`.
    app.finish_exit();
    assert!(server_guard.shutdown_flag().load(Ordering::Acquire), "human window close must signal control server shutdown");
    server_guard.stop();
    assert!(!recovery_dir.join(LOCKFILE_NAME).exists(), "human window close must release recovery lock");

    // Reopen recovery directory to confirm WAL checkpoint survived clean close.
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
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
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
            let active = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = active.set_read_timeout(Some(Duration::from_secs(5)));
            let mut r = BufReader::new(active.try_clone().map_err(|e| e.to_string())?);
            let mut w = BufWriter::new(active);
            let ping = send_json_line(&mut w, &mut r, &json!({"id": "c1", "method": "ping", "token": token_owned})).map_err(|e| e.to_string())?;
            assert_eq!(ping["ok"], json!(true));
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

    // Empty whitespace line produces no output and does not close connection.
    let empty_out = win.dispatch_live_control_frame_with_hook(b"   \n", &auth, false, |_, _| Ok(()))?;
    assert!(empty_out.response_bytes.is_empty());
    assert!(!empty_out.close_connection);

    // Invalid UTF-8 produces structured JSON error without crashing.
    let bad_utf8 = win.dispatch_live_control_frame_with_hook(b"\xff\xfe{\"id\":1}\n", &auth, true, |_, _| Ok(()))?;
    let utf8_val: Value = serde_json::from_slice(&bad_utf8.response_bytes)?;
    assert_eq!(utf8_val["ok"], json!(false));
    assert!(utf8_val["error"].as_str().is_some_and(|e| e.contains("UTF-8")));

    // Auth success -> invalid JSON gets JSON parse error; auth failure downgrades session auth.
    let auth_ok = win.dispatch_live_control_frame_with_hook(
        format!("{{\"id\":1,\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n", auth.as_str()).as_bytes(),
        &auth,
        false,
        |_, _| Ok(()),
    )?;
    assert_eq!(auth_ok.auth_succeeded, Some(true));

    let bad_json_authed = win.dispatch_live_control_frame_with_hook(b"{broken-json\n", &auth, true, |_, _| Ok(()))?;
    let bad_json_val: Value = serde_json::from_slice(&bad_json_authed.response_bytes)?;
    assert!(bad_json_val["error"].as_str().is_some_and(|e| e.contains("invalid control JSON")));

    let auth_bad = win.dispatch_live_control_frame_with_hook(
        b"{\"id\":2,\"method\":\"auth\",\"params\":{\"token\":\"wrong-secret\"}}\n",
        &auth,
        true,
        |_, _| Ok(()),
    )?;
    assert_eq!(auth_bad.auth_succeeded, Some(false));

    let rejected_after_downgrade = win.dispatch_live_control_frame_with_hook(b"{\"id\":3,\"method\":\"ping\"}\n", &auth, false, |_, _| Ok(()))?;
    let rejected_val: Value = serde_json::from_slice(&rejected_after_downgrade.response_bytes)?;
    assert_eq!(rejected_val["error_detail"]["code"], json!("unauthorized"));

    // HTTP request line emits 403 Forbidden and signals connection close.
    let http_out = win.dispatch_live_control_frame_with_hook(b"GET / HTTP/1.1\r\n", &auth, true, |_, _| Ok(()))?;
    assert_eq!(String::from_utf8(http_out.response_bytes)?, CONTROL_HTTP_FORBIDDEN_RESPONSE);
    assert!(http_out.close_connection);
    assert!(!http_out.quit);

    Ok(())
}
