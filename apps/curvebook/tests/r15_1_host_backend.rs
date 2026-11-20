#![forbid(unsafe_code)]

//! Wave 1 (`R15.1` bullets 1, 4, 5 & `#13`) integration tests for the `curvebook` desktop host:
//! - Phase 0 duplicate menu-bar suppression (`host_draws_menu_bar`) & surviving `egui::MenuBar`
//! - `BETA_ACTIVE_LOCALES` (14 non-RTL) & `normalize_beta_gui_locale`
//! - `--ui-backend=egui` / `CURVEBOOK_UI_BACKEND=egui` CLI & headless `--control` + `GuiOnlyPolicy::Strict`
//! - Host `Tokens::apply_to_ctx`, `Cmd/Ctrl +`/`-`/`0` `ui_scale` & `preferences.json` round-trip
//! - `LiveControlBridge` + `GeometryStamp` post-resize / post-DPI barrier (`ui.screenshot` / `ui.inspect`)

use std::io::{BufRead, BufReader, Cursor};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, NativeFileDialogAdapter, NativeFileDialogKind, NativeFileDialogRequest,
};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook::live_control::{LiveControlBridge, serve_live_control_io};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{BETA_ACTIVE_LOCALES, ControlAuth, LOCALES, inspect_ui_tree, normalize_beta_gui_locale};
use eframe::egui;
use serde_json::{Value, json};

#[derive(Debug)]
struct RecordingFileDialogAdapter {
    tx: std::sync::mpsc::Sender<NativeFileDialogRequest>,
}

impl RecordingFileDialogAdapter {
    fn new() -> (Self, std::sync::mpsc::Receiver<NativeFileDialogRequest>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { tx }, rx)
    }
}

impl NativeFileDialogAdapter for RecordingFileDialogAdapter {
    fn pick(&self, request: &NativeFileDialogRequest) -> Result<Option<PathBuf>, String> {
        self.tx.send(request.clone()).map_err(|e| e.to_string())?;
        Ok(None)
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let val = |b: u8| -> Option<u32> {
        match b {
            b'A'..=b'Z' => Some(u32::from(b - b'A')),
            b'a'..=b'z' => Some(u32::from(b - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(b - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        return Err(String::from("invalid base64 length"));
    }
    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    for &[c0, c1, c2, c3] in clean.as_chunks::<4>().0 {
        let pad = usize::from(c2 == b'=') + usize::from(c3 == b'=');
        let a = val(c0).ok_or_else(|| String::from("invalid base64 byte"))?;
        let b = val(c1).ok_or_else(|| String::from("invalid base64 byte"))?;
        let c = if c2 == b'=' { 0 } else { val(c2).ok_or_else(|| String::from("invalid base64 byte"))? };
        let d = if c3 == b'=' { 0 } else { val(c3).ok_or_else(|| String::from("invalid base64 byte"))? };
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        out.push(((triple >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((triple >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((triple & 0xff) as u8);
        }
    }
    Ok(out)
}

fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || !png.starts_with(b"\x89PNG\r\n\x1a\n") || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    Some((w, h))
}

fn temp_test_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn allocate_loopback_port() -> Result<u16, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn read_jsonl_outputs(bytes: &[u8]) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for line in BufReader::new(Cursor::new(bytes)).lines() {
        let line = line?;
        if !line.trim().is_empty() {
            out.push(serde_json::from_str(&line)?);
        }
    }
    Ok(out)
}

#[test]
fn desktop_menu_bar_sets_host_draws_flag_suppresses_canvas_duplicates_and_drives_actions() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1200.0, 760.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: false,
        ui_backend: UiBackend::Egui,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: metrics,
    });
    let (adapter, dialog_rx) = RecordingFileDialogAdapter::new();
    let adapter = Arc::new(adapter);
    rt.set_file_dialog_adapter(adapter);
    let win_id = rt.open_window(None, metrics)?;

    let mut native = CurveBookNativeApp::new(rt, win_id, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));

    // Render one live host frame -> sets `host_draws_menu_bar = true`.
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))), ..Default::default() },
        |ui| native.step_ui(ui),
    );
    out.textures_delta.clear();

    {
        let win = native.runtime().window(win_id)?;
        assert!(!win.app().host_draws_menu_bar, "in-window egui menu bar renders in top_chrome without host suppression");
        let menus_snap = inspect_ui_tree(win.app(), Some("menus"), true);
        assert!(menus_snap.menus.as_ref().is_some_and(|m| !m.is_empty()), "expected top menus in egui inspect snapshot");
    }

    // Verify theme, locale (14 Beta locales vs 16 registered), RTL fallback, and native File dialog triggers.
    assert_eq!(LOCALES.len(), 16);
    assert_eq!(BETA_ACTIVE_LOCALES.len(), 14);
    assert_eq!(normalize_beta_gui_locale("ar"), "en");
    assert_eq!(normalize_beta_gui_locale("he"), "en");

    let _ = native.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.theme.set:dark") })?;
    let _ = native.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.locale.set:de") })?;
    assert_eq!(native.runtime().window(win_id)?.app().locale, "de");

    let _ = native.runtime_mut().window_mut(win_id)?.app_mut().execute_command("sheet.createData", json!({ "sheet_id": "ds-menu", "kind": "xy" }))?;
    let _ = native
        .runtime_mut()
        .window_mut(win_id)?
        .app_mut()
        .execute_command("graph.create", json!({ "sheet_id": "gs-menu", "data_sheet_id": "ds-menu", "kind": "xy" }))?;
    let _ = native.runtime_mut().dispatch_native_file_command(win_id, "project.open")?;
    let _ = native.runtime_mut().dispatch_native_file_command(win_id, "file.save_as")?;
    let _ = native.runtime_mut().dispatch_native_file_command(win_id, "export.pdf")?;
    let kinds: Vec<NativeFileDialogKind> = dialog_rx.try_iter().map(|r| r.kind).collect();
    assert_eq!(kinds, vec![NativeFileDialogKind::OpenFile, NativeFileDialogKind::SaveFile, NativeFileDialogKind::SaveFile]);

    // Open `File` menu button in top `egui::MenuBar` via AccessKit bounds and verify keyboard
    // navigation stays in popup without leaking ArrowDown into the canvas while popup is open.
    ctx.enable_accesskit();
    let mut out_ak = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))), ..Default::default() },
        |ui| native.step_ui(ui),
    );
    out_ak.textures_delta.clear();
    let ak_update = out_ak.platform_output.accesskit_update.as_ref().ok_or("missing AccessKit TreeUpdate on menu bar")?;
    let localized_file = curvebook_ui_egui::tr("de", "menu.file");
    let file_bounds = ak_update
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.label().is_some_and(|l| l.contains(localized_file)) && node.bounds().is_some_and(|b| b.y1 <= 40.0)).then(|| node.bounds()).flatten()
        })
        .ok_or_else(|| format!("missing `{localized_file}` menu bar button in AccessKit tree"))?;
    let click_pt = egui::pos2(((file_bounds.x0 + file_bounds.x1) * 0.5) as f32, ((file_bounds.y0 + file_bounds.y1) * 0.5) as f32);
    let mut out_click = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))),
            events: vec![
                egui::Event::PointerMoved(click_pt),
                egui::Event::PointerButton { pos: click_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos: click_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
            ..Default::default()
        },
        |ui| native.step_ui(ui),
    );
    out_click.textures_delta.clear();
    assert!(egui::Popup::is_any_open(&ctx), "clicking File menu bar button must open egui::MenuBar popup");

    let active_cell_before = inspect_ui_tree(native.runtime().window(win_id)?.app(), None, false).grid_selection.active_cell;
    let mut out_nav = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))),
            events: vec![
                egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
                egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
            ],
            ..Default::default()
        },
        |ui| native.step_ui(ui),
    );
    out_nav.textures_delta.clear();
    assert_eq!(
        inspect_ui_tree(native.runtime().window(win_id)?.app(), None, false).grid_selection.active_cell,
        active_cell_before,
        "menu navigation keys while egui::MenuBar popup is open must not mutate canvas/grid state behind it"
    );
    Ok(())
}

#[test]
fn cmd_ctrl_ui_scale_hotkeys_apply_to_ctx_and_survive_theme_switch() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1000.0, 680.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        ui_backend: UiBackend::Egui,
        initial_viewport: metrics,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window(None, metrics)?;
    let mut native = CurveBookNativeApp::new(rt, win_id, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));

    // Cmd/Ctrl + zooms `ui_scale` up (`1.0 -> 1.1 -> 1.2`) and updates `ctx.zoom_factor()`.
    let cmd_mods = egui::Modifiers { command: true, ctrl: true, ..Default::default() };
    let mut out1 = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 680.0))),
            events: vec![
                egui::Event::Key { key: egui::Key::Plus, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods },
                egui::Event::Key { key: egui::Key::Equals, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods },
            ],
            ..Default::default()
        },
        |ui| native.step_ui(ui),
    );
    out1.textures_delta.clear();
    assert!((native.runtime().window(win_id)?.app().ui_scale() - 1.2).abs() < 1e-4);

    // Run one more pass so `Tokens::apply_to_ctx` reflects `ui_scale == 1.2` on `ctx.zoom_factor()`.
    let mut out2 = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 680.0))), ..Default::default() },
        |ui| native.step_ui(ui),
    );
    out2.textures_delta.clear();
    assert!((ctx.zoom_factor() - 1.2).abs() < 1e-3);

    // Cmd/Ctrl - steps down (`1.2 -> 1.1`), theme change preserves `ui_scale`, and Cmd/Ctrl 0 resets (`1.0`).
    let mut out3 = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 680.0))),
            events: vec![egui::Event::Key { key: egui::Key::Minus, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods }],
            ..Default::default()
        },
        |ui| native.step_ui(ui),
    );
    out3.textures_delta.clear();
    assert!((native.runtime().window(win_id)?.app().ui_scale() - 1.1).abs() < 1e-4);

    native.runtime_mut().window_mut(win_id)?.app_mut().set_theme_mode(curvebook_ui_egui::UiThemeMode::Dark);
    assert!((native.runtime().window(win_id)?.app().ui_scale() - 1.1).abs() < 1e-4);

    let mut out4 = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 680.0))),
            events: vec![egui::Event::Key { key: egui::Key::Num0, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods }],
            ..Default::default()
        },
        |ui| native.step_ui(ui),
    );
    out4.textures_delta.clear();
    assert!((native.runtime().window(win_id)?.app().ui_scale() - 1.0).abs() < 1e-4);
    Ok(())
}

#[test]
fn live_control_ui_screenshot_waits_for_post_resize_dpi_geometry_stamp_settlement() -> Result<(), Box<dyn std::error::Error>> {
    let initial = ViewportMetrics::new(800.0, 500.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        ui_backend: UiBackend::Egui,
        initial_viewport: initial,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window(None, initial)?;

    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());

    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut native = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    // Frame 1 settles at 800x500 @ 1.0x.
    let mut out1 = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(1.0), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        },
        |ui| native.step_ui(ui),
    );
    out1.textures_delta.clear();
    let stamp1 = native.geometry_stamp().ok_or("missing frame 1 GeometryStamp")?;
    assert!(stamp1.matches_target(800, 500, 1.0));

    let token = auth.as_str().to_owned();
    let bridge_bg = bridge.clone();
    let client = std::thread::spawn(move || -> Result<Vec<u8>, String> {
        let script = format!(
            "{{\"id\":\"a\",\"method\":\"auth\",\"params\":{{\"token\":\"{token}\"}}}}\n\
             {{\"id\":\"s\",\"method\":\"ui.screenshot\",\"params\":{{\"target\":\"full_window\"}}}}\n\
             {{\"id\":\"i\",\"method\":\"ui.inspect\",\"params\":{{}}}}\n"
        );
        let mut reader = BufReader::new(Cursor::new(script.into_bytes()));
        let mut out_bytes = Vec::new();
        serve_live_control_io(&mut reader, &mut out_bytes, &bridge_bg, |_| {}).map_err(|e| e.to_string())?;
        Ok(out_bytes)
    });

    // Resize to 1120x700 @ 1.5x (1680x1050 physical px) while control requests wait on settled barrier.
    let resized_raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1120.0, 700.0))),
        viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(1.5), ..Default::default() })].into_iter().collect(),
        ..Default::default()
    };
    let start = Instant::now();
    while !client.is_finished() && start.elapsed() < Duration::from_secs(5) {
        let mut out_n = ctx.run_ui(resized_raw.clone(), |ui| native.step_ui(ui));
        out_n.textures_delta.clear();
        std::thread::sleep(Duration::from_millis(5));
    }

    bridge.signal_shutdown();
    let raw_out = client.join().map_err(|_| "client thread panicked")??;
    let replies = read_jsonl_outputs(&raw_out)?;
    assert_eq!(replies.len(), 3, "expected auth + screenshot + inspect replies, got {replies:?}");
    assert_eq!(replies[0]["ok"], json!(true));

    let shot = &replies[1];
    assert_eq!(shot["ok"], json!(true), "screenshot failed: {shot}");
    assert_eq!(shot["result"]["width_px"], json!(1680), "screenshot must capture post-resize 1680px width, got {shot}");
    assert_eq!(shot["result"]["height_px"], json!(1050), "screenshot must capture post-resize 1050px height, got {shot}");
    assert_eq!(shot["result"]["dpi"], json!(144));
    let decoded_shot = decode_base64(shot["result"]["data_base64"].as_str().ok_or("missing screenshot data_base64")?)?;
    assert_eq!(png_dimensions(&decoded_shot), Some((1680, 1050)), "PNG IHDR must reflect post-resize 1680x1050");

    let inspect = &replies[2];
    assert_eq!(inspect["ok"], json!(true), "inspect failed: {inspect}");
    Ok(())
}

#[test]
fn cli_ui_backend_egui_env_override_gui_only_and_preferences_ui_scale() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_curvebook");
    let help = Command::new(bin).arg("--help").output()?;
    assert!(help.status.success());
    let help_txt = String::from_utf8_lossy(&help.stdout);
    assert!(help_txt.contains("--ui-backend"), "expected --ui-backend in --help: {help_txt}");

    let bad = Command::new(bin).args(["--ui-backend=unknown_backend"]).output()?;
    assert!(!bad.status.success(), "unknown --ui-backend value must fail CLI validation");

    let rejected_legacy_cli = Command::new(bin).args(["--ui-backend=legacy"]).output()?;
    assert!(!rejected_legacy_cli.status.success(), "--ui-backend=legacy must be rejected cleanly after R15.5 cutover");

    let rejected_legacy_env = Command::new(bin).env("CURVEBOOK_UI_BACKEND", "legacy").arg("--headless-once").output()?;
    assert!(!rejected_legacy_env.status.success(), "CURVEBOOK_UI_BACKEND=legacy must be rejected cleanly after R15.5 cutover");

    // Verify CURVEBOOK_UI_BACKEND=egui + `--headless-once` works on the compiled binary.
    let dir = temp_test_dir("cb-r15-bin-egui")?;
    let headless_ok = Command::new(bin)
        .env("CURVEBOOK_UI_BACKEND", "egui")
        .args(["--headless-once", "--portable-dir"])
        .arg(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    assert!(headless_ok.status.success(), "CURVEBOOK_UI_BACKEND=egui --headless-once failed: {}", String::from_utf8_lossy(&headless_ok.stderr));

    // Verify GuiOnlyPolicy::Strict + UiBackend::Egui over live control channel (using TCP if sandbox permits,
    // plus deterministic in-process LiveControlBridge verification).
    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        gui_only: true,
        ui_backend: UiBackend::Egui,
        initial_viewport: ViewportMetrics::new(800.0, 500.0, 1.0),
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window(None, ViewportMetrics::new(800.0, 500.0, 1.0))?;
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut native = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let token = auth.as_str().to_owned();
    let bridge_bg = bridge.clone();
    let client = std::thread::spawn(move || -> Result<Vec<u8>, String> {
        let script = format!(
            "{{\"id\":\"0\",\"method\":\"auth\",\"params\":{{\"token\":\"{token}\"}}}}\n\
             {{\"id\":\"1\",\"method\":\"doc.inspect\"}}\n\
             {{\"id\":\"2\",\"method\":\"ui.screenshot\",\"params\":{{\"path\":\"blocked.png\"}}}}\n\
             {{\"id\":\"3\",\"method\":\"ui.inspect\",\"params\":{{}}}}\n\
             {{\"id\":\"4\",\"method\":\"ui.screenshot\",\"params\":{{\"width_px\":400,\"height_px\":260,\"dpi\":96}}}}\n\
             {{\"id\":\"5\",\"method\":\"ui.invoke_menu\",\"params\":{{\"command_id\":\"ui.theme.set\",\"params\":{{\"mode\":\"dark\"}}}}}}\n"
        );
        let mut reader = BufReader::new(Cursor::new(script.into_bytes()));
        let mut out_bytes = Vec::new();
        serve_live_control_io(&mut reader, &mut out_bytes, &bridge_bg, |_| {}).map_err(|e| e.to_string())?;
        Ok(out_bytes)
    });

    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))), ..Default::default() };
    let start = Instant::now();
    while !client.is_finished() && start.elapsed() < Duration::from_secs(5) {
        let mut out = ctx.run_ui(raw.clone(), |ui| native.step_ui(ui));
        out.textures_delta.clear();
        std::thread::sleep(Duration::from_millis(5));
    }

    bridge.signal_shutdown();
    let replies = read_jsonl_outputs(&client.join().map_err(|_| "client thread panicked")??)?;
    assert_eq!(replies.len(), 6, "expected 6 control replies, got {replies:?}");
    assert_eq!(replies[0]["ok"], json!(true));
    assert_eq!(replies[1]["ok"], json!(false));
    assert_eq!(replies[1]["error_detail"]["code"], json!("gui_only_violation"));
    assert_eq!(replies[2]["ok"], json!(false));
    assert_eq!(replies[2]["error_detail"]["code"], json!("gui_only_violation"));
    assert_eq!(replies[3]["ok"], json!(true));
    assert!(replies[3]["result"]["toolbar"].is_array());
    assert_eq!(replies[4]["ok"], json!(true));
    assert_eq!(replies[4]["result"]["width_px"], json!(400));
    assert_eq!(replies[4]["result"]["height_px"], json!(260));
    let bin_shot = decode_base64(replies[4]["result"]["data_base64"].as_str().ok_or("missing data_base64")?)?;
    assert_eq!(png_dimensions(&bin_shot), Some((400, 260)));

    // Unused `allocate_loopback_port` check when loopback binding is allowed outside sandbox.
    if let Ok(port) = allocate_loopback_port() {
        assert!(port > 0);
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn step_ui_clears_discarded_pass_intents_without_double_or_stale_execution() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(960.0, 640.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        ui_backend: UiBackend::Egui,
        initial_viewport: metrics,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window(None, metrics)?;
    let mut native = CurveBookNativeApp::new(rt, win_id, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));

    let raw_empty = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))), ..Default::default() };
    let mut out_init = ctx.run_ui(raw_empty.clone(), |ui| native.step_ui(ui));
    out_init.textures_delta.clear();

    let about_center = native
        .inspect_meta()
        .find_by_widget_id("toolbar.help.about")
        .map(|n| n.clipped_rect.center())
        .ok_or("missing toolbar.help.about clipped_rect in egui_shell")?;

    // Emulate an `egui` multi-pass frame where Pass 0 emits an intent (`toolbar.help.about` click)
    // AND calls `request_discard`. Pass 0's discarded intent must NOT execute and must be cleared
    // before Pass 1 (settled) and subsequent frames run.
    let raw_click = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))),
        events: vec![
            egui::Event::PointerMoved(about_center),
            egui::Event::PointerButton { pos: about_center, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: about_center, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
        ..Default::default()
    };
    let mut pass_records: Vec<(bool, ActiveDialog)> = Vec::new();
    let mut out_discard = ctx.run_ui(raw_click.clone(), |ui| {
        if pass_records.is_empty() {
            ui.ctx().request_discard("simulated multi-pass resize discard");
        }
        native.step_ui(ui);
        let settled = native.geometry_stamp().is_some_and(|s| s.settled);
        let dialog = native.runtime().window(win_id).map_or(ActiveDialog::None, |w| w.app().canvas_dialogs.active_dialog.clone());
        pass_records.push((settled, dialog));
    });
    out_discard.textures_delta.clear();
    assert_eq!(pass_records.len(), 2, "expected Pass 0 discarded + Pass 1 settled, got {pass_records:?}");
    assert_eq!(pass_records[0], (false, ActiveDialog::None), "discarded Pass 0 must not commit intent");
    assert_eq!(pass_records[1], (true, ActiveDialog::None), "settled Pass 1 must not inherit discarded Pass 0 intent");

    // Next separate frame with NO click must also not observe any carried-over discarded intent.
    let mut out_after_discard = ctx.run_ui(raw_empty, |ui| native.step_ui(ui));
    out_after_discard.textures_delta.clear();
    assert_eq!(
        native.runtime().window(win_id)?.app().canvas_dialogs.active_dialog,
        ActiveDialog::None,
        "discarded pass intent must be cleared and never execute on a later settled frame"
    );
    assert!(native.geometry_stamp().ok_or("missing GeometryStamp")?.settled);

    // A genuinely settled click (no `request_discard`) must execute the intent cleanly once.
    let mut out_settled = ctx.run_ui(raw_click, |ui| native.step_ui(ui));
    out_settled.textures_delta.clear();
    assert_eq!(native.runtime().window(win_id)?.app().canvas_dialogs.active_dialog, ActiveDialog::About);
    Ok(())
}

#[test]
fn egui_backend_suppresses_phantom_legacy_pointer_wheel_and_non_global_keys() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(960.0, 640.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        ui_backend: UiBackend::Egui,
        initial_viewport: metrics,
        ..DesktopHostConfig::default()
    });
    let (adapter, dialog_rx) = RecordingFileDialogAdapter::new();
    rt.set_file_dialog_adapter(Arc::new(adapter));
    let win_id = rt.open_window(None, metrics)?;

    let mut native = CurveBookNativeApp::new(rt, win_id, None);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));

    let raw_init = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))), ..Default::default() };
    let mut out_init = ctx.run_ui(raw_init, |ui| native.step_ui(ui));
    out_init.textures_delta.clear();

    let (active_cell_before, zoom_before, inline_before) = {
        let app = native.runtime().window(win_id)?.app();
        let snap = inspect_ui_tree(app, None, false);
        (snap.grid_selection.active_cell, snap.zoom, (app.inline_editor.active, app.inline_editor.buffer.clone()))
    };

    // Click inside central workspace, scroll wheel, and type arrows/text: must NOT mutate legacy grid/canvas.
    let canvas_pt = egui::pos2(500.0, 340.0);
    let raw_events = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))),
        events: vec![
            egui::Event::PointerMoved(canvas_pt),
            egui::Event::PointerButton { pos: canvas_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: canvas_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(18.0, -72.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
            egui::Event::Key { key: egui::Key::Num4, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
            egui::Event::Text(String::from("4")),
            egui::Event::Ime(egui::ImeEvent::Commit(String::from("z"))),
        ],
        ..Default::default()
    };
    let mut out_ev = ctx.run_ui(raw_events, |ui| native.step_ui(ui));
    out_ev.textures_delta.clear();

    {
        let app = native.runtime().window(win_id)?.app();
        let snap = inspect_ui_tree(app, None, false);
        assert_eq!(snap.grid_selection.active_cell, active_cell_before, "legacy grid selection must not change under UiBackend::Egui");
        assert!((snap.zoom - zoom_before).abs() < 1e-9, "legacy canvas zoom must not change under UiBackend::Egui");
        assert_eq!((app.inline_editor.active, app.inline_editor.buffer.clone()), inline_before, "legacy inline editor must not activate");
    }

    // Global `Cmd/Ctrl` shortcuts (`ui_scale` + `Cmd+O` native open) must remain functional.
    let cmd_mods = egui::Modifiers { command: true, ctrl: true, ..Default::default() };
    let mut out_cmd = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))),
            events: vec![
                egui::Event::Key { key: egui::Key::Plus, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods },
                egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods },
            ],
            ..Default::default()
        },
        |ui| native.step_ui(ui),
    );
    out_cmd.textures_delta.clear();
    assert!((native.runtime().window(win_id)?.app().ui_scale() - 1.1).abs() < 1e-4);
    let opened_kind = dialog_rx.try_iter().next().map(|r| r.kind);
    assert_eq!(opened_kind, Some(NativeFileDialogKind::OpenFile));
    Ok(())
}

#[test]
fn live_control_ui_scale_1_5_does_not_double_scale_inspect_or_screenshot() -> Result<(), Box<dyn std::error::Error>> {
    let initial = ViewportMetrics::new(800.0, 500.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        ui_backend: UiBackend::Egui,
        initial_viewport: initial,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window(None, initial)?;
    {
        let app = rt.window_mut(win_id)?.app_mut();
        app.set_ui_scale(1.5);
        let _ = app.dispatch_control_line(r#"{"id":"open-about","method":"app.about"}"#);
    }

    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut native = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    // Run with fixed 1200x750 physical px and native 1.0x DPI; match `egui-winit` by dividing
    // physical pixel dimensions by `ctx.pixels_per_point()` once `zoom_factor = 1.5` activates,
    // yielding `ctx.content_rect() = 800x500` pt and `ctx.pixels_per_point() = 1.5`.
    let raw_for_ctx = |c: &egui::Context| -> egui::RawInput {
        let cur_ppp = c.pixels_per_point().max(1.0);
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0 / cur_ppp, 750.0 / cur_ppp))),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(1.0), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        }
    };
    for _ in 0..4 {
        let raw = raw_for_ctx(&ctx);
        let mut out = ctx.run_ui(raw, |ui| native.step_ui(ui));
        out.textures_delta.clear();
        if native.geometry_stamp().is_some_and(|s| s.matches_target(1200, 750, 1.5)) {
            break;
        }
    }
    let stamp = native.geometry_stamp().ok_or("missing settled GeometryStamp")?;
    assert!(stamp.matches_target(1200, 750, 1.5), "expected settled 1200x750 @ 1.5x stamp, got {stamp:?}");

    let token = auth.as_str().to_owned();
    let bridge_bg = bridge.clone();
    let client = std::thread::spawn(move || -> Result<Vec<u8>, String> {
        let script = format!(
            "{{\"id\":\"a\",\"method\":\"auth\",\"params\":{{\"token\":\"{token}\"}}}}\n\
             {{\"id\":\"s\",\"method\":\"ui.screenshot\",\"params\":{{\"target\":\"full_window\"}}}}\n\
             {{\"id\":\"i\",\"method\":\"ui.inspect\",\"params\":{{\"scope\":\"dialog\",\"include_bounds\":true}}}}\n"
        );
        let mut reader = BufReader::new(Cursor::new(script.into_bytes()));
        let mut out_bytes = Vec::new();
        serve_live_control_io(&mut reader, &mut out_bytes, &bridge_bg, |_| {}).map_err(|e| e.to_string())?;
        Ok(out_bytes)
    });

    let start = Instant::now();
    while !client.is_finished() && start.elapsed() < Duration::from_secs(5) {
        let raw = raw_for_ctx(&ctx);
        let mut out = ctx.run_ui(raw, |ui| native.step_ui(ui));
        out.textures_delta.clear();
        std::thread::sleep(Duration::from_millis(5));
    }

    bridge.signal_shutdown();
    let replies = read_jsonl_outputs(&client.join().map_err(|_| "client thread panicked")??)?;
    assert_eq!(replies.len(), 3, "expected 3 control replies, got {replies:?}");

    let shot = &replies[1];
    assert_eq!(shot["ok"], json!(true), "screenshot failed: {shot}");
    assert_eq!(shot["result"]["width_px"], json!(1200));
    assert_eq!(shot["result"]["height_px"], json!(750));
    let png = decode_base64(shot["result"]["data_base64"].as_str().ok_or("missing data_base64")?)?;
    assert_eq!(png_dimensions(&png), Some((1200, 750)));

    let inspect = &replies[2];
    assert_eq!(inspect["ok"], json!(true), "inspect failed: {inspect}");
    let bounds = inspect["result"]["dialog"]["bounds"].as_array().ok_or("missing dialog.bounds")?;
    let bx = bounds.first().and_then(Value::as_f64).ok_or("bad x")?;
    let by = bounds.get(1).and_then(Value::as_f64).ok_or("bad y")?;
    let bw = bounds.get(2).and_then(Value::as_f64).ok_or("bad w")?;
    let bh = bounds.get(3).and_then(Value::as_f64).ok_or("bad h")?;
    assert!(
        bx >= 0.0 && by >= 0.0 && bx + bw <= 1202.0 && by + bh <= 752.0 && bw < 760.0,
        "dialog bounds at ui_scale=1.5 must reflect 1.5x (not double-scaled 2.25x): {bounds:?}"
    );
    Ok(())
}
