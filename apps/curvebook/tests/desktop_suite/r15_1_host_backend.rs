#![forbid(unsafe_code)]

//! Wave 1 (`R15.1` bullets 1, 4, 5 & `#13`) integration tests for the `curvebook` desktop host:
//! - Duplicate menu-bar suppression & surviving `egui::MenuBar`
//! - `BETA_ACTIVE_LOCALES` (14 non-RTL) & `normalize_beta_gui_locale`
//! - `--ui-backend=egui` / `CURVEBOOK_UI_BACKEND=egui` CLI & headless `--control` live bridge
//! - Host `Tokens::apply_to_ctx`, `Cmd/Ctrl +`/`-`/`0` `ui_scale`
//! - `LiveControlBridge` + `GeometryStamp` post-resize / post-DPI barrier (`ui.screenshot` / `ui.inspect`)

use std::fs;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, NativeFileDialogAdapter, NativeFileDialogKind, NativeFileDialogRequest,
};
use curvebook::gui_window::{CurveBookNativeApp, LiveControlChannel, LiveControlRequest};
use curvebook::live_control::{LiveControlBridge, serve_live_control_io};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, LinearRegressionSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{BETA_ACTIVE_LOCALES, ControlAuth, LOCALES, UiApp, inspect_ui_tree, normalize_beta_gui_locale};
use eframe::egui;
use serde_json::{Value, json};

use crate::common::{NativeHarness, TempDirGuard, decode_base64, new_egui_context, png_dimensions, read_jsonl_values, reserve_ephemeral_port};

#[derive(Debug)]
struct RecordingFileDialogAdapter(std::sync::mpsc::Sender<NativeFileDialogRequest>);

impl RecordingFileDialogAdapter {
    fn new() -> (Self, std::sync::mpsc::Receiver<NativeFileDialogRequest>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self(tx), rx)
    }
}

impl NativeFileDialogAdapter for RecordingFileDialogAdapter {
    fn pick(&self, request: &NativeFileDialogRequest) -> Result<Option<PathBuf>, String> {
        self.0.send(request.clone()).map_err(|e| e.to_string())?;
        Ok(None)
    }
}

fn run_scripted_live_control(
    native: &mut CurveBookNativeApp,
    ctx: &egui::Context,
    bridge: &LiveControlBridge,
    script: String,
    mut make_raw: impl FnMut(&egui::Context) -> egui::RawInput,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let bridge_bg = bridge.clone();
    let client = std::thread::spawn(move || -> Result<Vec<u8>, String> {
        let mut reader = BufReader::new(Cursor::new(script.into_bytes()));
        let mut out_bytes = Vec::new();
        serve_live_control_io(&mut reader, &mut out_bytes, &bridge_bg, |_| {}).map_err(|e| e.to_string())?;
        Ok(out_bytes)
    });
    let start = Instant::now();
    while !client.is_finished() && start.elapsed() < Duration::from_secs(5) {
        let raw = make_raw(ctx);
        let mut out = ctx.run_ui(raw, |ui| native.step_ui(ui));
        out.textures_delta.clear();
        std::thread::sleep(Duration::from_millis(5));
    }
    bridge.signal_shutdown();
    read_jsonl_values(&client.join().map_err(|_| "client thread panicked")??)
}

#[test]
fn desktop_menu_bar_sets_host_draws_flag_suppresses_canvas_duplicates_and_drives_actions() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1200.0, 760.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, initial_viewport: metrics, ..DesktopHostConfig::default() });
    let (adapter, dialog_rx) = RecordingFileDialogAdapter::new();
    rt.set_file_dialog_adapter(Arc::new(adapter));
    let win_id = rt.open_window(None, metrics)?;

    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, win_id, None), egui::vec2(1200.0, 760.0));
    let _ = h.step_events(Vec::new());
    {
        let win = h.app.runtime().window(win_id)?;
        assert!(!win.app().host_draws_menu_bar);
        assert!(inspect_ui_tree(win.app(), Some("menus"), true).menus.as_ref().is_some_and(|m| !m.is_empty()));
    }

    assert_eq!((LOCALES.len(), BETA_ACTIVE_LOCALES.len()), (16, 14));
    assert_eq!((normalize_beta_gui_locale("ar"), normalize_beta_gui_locale("he")), ("en", "en"));

    let _ = h.app.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.theme.set:dark") })?;
    let _ = h.app.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.locale.set:de") })?;
    assert_eq!(h.app.runtime().window(win_id)?.app().locale, "de");

    let _ = h.app.runtime_mut().window_mut(win_id)?.app_mut().execute_command("sheet.createData", json!({ "sheet_id": "ds-menu", "kind": "xy" }))?;
    let _ = h
        .app
        .runtime_mut()
        .window_mut(win_id)?
        .app_mut()
        .execute_command("graph.create", json!({ "sheet_id": "gs-menu", "data_sheet_id": "ds-menu", "kind": "xy" }))?;
    for cmd in ["project.open", "file.save_as", "export.pdf"] {
        let _ = h.app.runtime_mut().dispatch_native_file_command(win_id, cmd)?;
    }
    let kinds: Vec<NativeFileDialogKind> = dialog_rx.try_iter().map(|r| r.kind).collect();
    assert_eq!(kinds, vec![NativeFileDialogKind::OpenFile, NativeFileDialogKind::SaveFile, NativeFileDialogKind::SaveFile]);

    // Open `File` menu button via AccessKit bounds and verify keyboard nav stays in popup.
    h.ctx.enable_accesskit();
    let out_ak = h.step_events(Vec::new());
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
    let _ = h.step_events(vec![
        egui::Event::PointerMoved(click_pt),
        egui::Event::PointerButton { pos: click_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos: click_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    assert!(egui::Popup::is_any_open(&h.ctx), "clicking File menu bar button must open egui::MenuBar popup");

    let active_cell_before = inspect_ui_tree(h.app.runtime().window(win_id)?.app(), None, false).grid_selection.active_cell;
    let _ = h.step_events(vec![
        egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
    ]);
    assert_eq!(inspect_ui_tree(h.app.runtime().window(win_id)?.app(), None, false).grid_selection.active_cell, active_cell_before);
    Ok(())
}

#[test]
fn cmd_ctrl_ui_scale_hotkeys_apply_to_ctx_and_survive_theme_switch() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = DesktopHostConfig { in_window_menus: true, initial_viewport: ViewportMetrics::new(1000.0, 680.0, 1.0), ..DesktopHostConfig::default() };
    let (mut h, win_id) = NativeHarness::from_config(cfg, None)?;
    let cmd_mods = egui::Modifiers { command: true, ctrl: true, ..Default::default() };

    let _ = h.step_events(vec![
        egui::Event::Key { key: egui::Key::Plus, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods },
        egui::Event::Key { key: egui::Key::Equals, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods },
    ]);
    assert!((h.app.runtime().window(win_id)?.app().ui_scale() - 1.2).abs() < 1e-4);
    let _ = h.step_events(Vec::new());
    assert!((h.ctx.zoom_factor() - 1.2).abs() < 1e-3);

    let _ = h.step_events(vec![egui::Event::Key { key: egui::Key::Minus, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods }]);
    assert!((h.app.runtime().window(win_id)?.app().ui_scale() - 1.1).abs() < 1e-4);

    h.app.runtime_mut().window_mut(win_id)?.app_mut().set_theme_mode(curvebook_ui_egui::UiThemeMode::Dark);
    assert!((h.app.runtime().window(win_id)?.app().ui_scale() - 1.1).abs() < 1e-4);

    let _ = h.step_events(vec![egui::Event::Key { key: egui::Key::Num0, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods }]);
    assert!((h.app.runtime().window(win_id)?.app().ui_scale() - 1.0).abs() < 1e-4);
    Ok(())
}

#[test]
fn live_control_ui_screenshot_waits_for_post_resize_dpi_geometry_stamp_settlement() -> Result<(), Box<dyn std::error::Error>> {
    let initial = ViewportMetrics::new(800.0, 500.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, initial_viewport: initial, ..DesktopHostConfig::default() });
    let win_id = rt.open_window(None, initial)?;
    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());
    let ctx = new_egui_context();
    let mut native = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let make_raw = |size: egui::Vec2, scale: f32| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(scale), ..Default::default() })]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let mut out1 = ctx.run_ui(make_raw(egui::vec2(800.0, 500.0), 1.0), |ui| native.step_ui(ui));
    out1.textures_delta.clear();
    assert!(native.geometry_stamp().ok_or("missing frame 1 GeometryStamp")?.matches_target(800, 500, 1.0));

    let script = format!(
        "{{\"id\":\"a\",\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n\
         {{\"id\":\"s\",\"method\":\"ui.screenshot\",\"params\":{{\"target\":\"full_window\"}}}}\n\
         {{\"id\":\"i\",\"method\":\"ui.inspect\",\"params\":{{}}}}\n",
        auth.as_str()
    );
    let replies = run_scripted_live_control(&mut native, &ctx, &bridge, script, |_| make_raw(egui::vec2(1120.0, 700.0), 1.5))?;
    assert_eq!(replies.len(), 3);
    assert_eq!((&replies[0]["ok"], &replies[1]["ok"], &replies[2]["ok"]), (&json!(true), &json!(true), &json!(true)));
    assert_eq!(
        (&replies[1]["result"]["width_px"], &replies[1]["result"]["height_px"], &replies[1]["result"]["dpi"]),
        (&json!(1680), &json!(1050), &json!(144))
    );
    let decoded = decode_base64(replies[1]["result"]["data_base64"].as_str().ok_or("missing screenshot data_base64")?)?;
    assert_eq!(png_dimensions(&decoded), Some((1680, 1050)));
    Ok(())
}

#[test]
fn cli_ui_backend_egui_env_override_live_control_and_preferences_ui_scale() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_curvebook");
    let help = Command::new(bin).arg("--help").output()?;
    assert!(help.status.success() && String::from_utf8_lossy(&help.stdout).contains("--ui-backend"));

    for bad_arg in ["--ui-backend=unknown_backend", "--ui-backend=legacy"] {
        assert!(!Command::new(bin).arg(bad_arg).output()?.status.success(), "{bad_arg} must fail");
    }
    assert!(!Command::new(bin).env("CURVEBOOK_UI_BACKEND", "legacy").arg("--headless-once").output()?.status.success());

    let dir = TempDirGuard::new("cb-r15-bin-egui")?;
    let headless_ok = Command::new(bin)
        .env("CURVEBOOK_UI_BACKEND", "egui")
        .args(["--headless-once", "--portable-dir"])
        .arg(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    assert!(headless_ok.status.success(), "CURVEBOOK_UI_BACKEND=egui failed: {}", String::from_utf8_lossy(&headless_ok.stderr));

    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());
    let initial = ViewportMetrics::new(800.0, 500.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, initial_viewport: initial, ..DesktopHostConfig::default() });
    let win_id = rt.open_window(None, initial)?;
    let ctx = new_egui_context();
    let mut native = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let script = format!(
        "{{\"id\":\"0\",\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n\
         {{\"id\":\"1\",\"method\":\"doc.inspect\"}}\n\
         {{\"id\":\"2\",\"method\":\"ui.inspect\",\"params\":{{}}}}\n\
         {{\"id\":\"3\",\"method\":\"ui.screenshot\",\"params\":{{\"width_px\":400,\"height_px\":260,\"dpi\":96}}}}\n\
         {{\"id\":\"4\",\"method\":\"ui.invoke_menu\",\"params\":{{\"command_id\":\"ui.theme.set\",\"params\":{{\"mode\":\"dark\"}}}}}}\n",
        auth.as_str()
    );
    let replies = run_scripted_live_control(&mut native, &ctx, &bridge, script, |_| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))),
        ..Default::default()
    })?;
    assert_eq!(replies.len(), 5);
    assert!(replies.iter().all(|r| r["ok"] == json!(true)));
    assert!(replies[2]["result"]["toolbar"].is_array());
    assert_eq!((&replies[3]["result"]["width_px"], &replies[3]["result"]["height_px"]), (&json!(400), &json!(260)));
    let bin_shot = decode_base64(replies[3]["result"]["data_base64"].as_str().ok_or("missing data_base64")?)?;
    assert_eq!(png_dimensions(&bin_shot), Some((400, 260)));

    if let Ok(Some(port)) = reserve_ephemeral_port() {
        assert!(port > 0);
    }
    Ok(())
}

#[test]
fn step_ui_clears_discarded_pass_intents_without_double_or_stale_execution() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = DesktopHostConfig { in_window_menus: true, initial_viewport: ViewportMetrics::new(960.0, 640.0, 1.0), ..DesktopHostConfig::default() };
    let (mut h, win_id) = NativeHarness::from_config(cfg, None)?;
    let _ = h.step_events(Vec::new());

    let home_center = h.app.inspect_meta().find_by_widget_id("toolbar.home").map(|n| n.clipped_rect.center()).ok_or("missing toolbar.home")?;
    let raw_click = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))),
        events: vec![
            egui::Event::PointerMoved(home_center),
            egui::Event::PointerButton { pos: home_center, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: home_center, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
        ..Default::default()
    };
    let mut pass_records = Vec::new();
    let mut out_discard = h.ctx.run_ui(raw_click.clone(), |ui| {
        if pass_records.is_empty() {
            ui.ctx().request_discard("simulated multi-pass resize discard");
        }
        h.app.step_ui(ui);
        pass_records.push((
            h.app.geometry_stamp().is_some_and(|s| s.settled),
            h.app.runtime().window(win_id).is_ok_and(|w| w.app().egui_views.launcher_open),
        ));
    });
    out_discard.textures_delta.clear();
    assert_eq!(pass_records, vec![(false, false), (true, true)]);

    h.app.runtime_mut().window_mut(win_id)?.app_mut().egui_views.launcher_open = false;
    h.app.runtime_mut().window_mut(win_id)?.app_mut().nav.welcome.open = false;
    let _ = h.step_events(Vec::new());
    assert!(!h.app.runtime().window(win_id)?.app().egui_views.launcher_open);
    assert!(h.app.geometry_stamp().ok_or("missing GeometryStamp")?.settled);

    let _ = h.step_raw(raw_click);
    assert!(h.app.runtime().window(win_id)?.app().egui_views.launcher_open);
    Ok(())
}

#[test]
fn egui_backend_suppresses_phantom_legacy_pointer_wheel_and_non_global_keys() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(960.0, 640.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, initial_viewport: metrics, ..DesktopHostConfig::default() });
    let (adapter, dialog_rx) = RecordingFileDialogAdapter::new();
    rt.set_file_dialog_adapter(Arc::new(adapter));
    let win_id = rt.open_window(None, metrics)?;

    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, win_id, None), egui::vec2(960.0, 640.0));
    let _ = h.step_events(Vec::new());

    let (active_before, zoom_before, inline_before) = {
        let app = h.app.runtime().window(win_id)?.app();
        let snap = inspect_ui_tree(app, None, false);
        (snap.grid_selection.active_cell, snap.zoom, (app.inline_editor.active, app.inline_editor.buffer.clone()))
    };

    let pt = egui::pos2(500.0, 340.0);
    let _ = h.step_events(vec![
        egui::Event::PointerMoved(pt),
        egui::Event::PointerButton { pos: pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos: pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
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
    ]);

    {
        let app = h.app.runtime().window(win_id)?.app();
        let snap = inspect_ui_tree(app, None, false);
        assert_eq!(snap.grid_selection.active_cell, active_before);
        assert!((snap.zoom - zoom_before).abs() < 1e-9);
        assert_eq!((app.inline_editor.active, app.inline_editor.buffer.clone()), inline_before);
    }

    let cmd_mods = egui::Modifiers { command: true, ctrl: true, ..Default::default() };
    let _ = h.step_events(vec![
        egui::Event::Key { key: egui::Key::Plus, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods },
        egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: true, repeat: false, modifiers: cmd_mods },
    ]);
    assert!((h.app.runtime().window(win_id)?.app().ui_scale() - 1.1).abs() < 1e-4);
    assert_eq!(dialog_rx.try_iter().next().map(|r| r.kind), Some(NativeFileDialogKind::OpenFile));
    Ok(())
}

#[test]
fn live_control_ui_scale_1_5_does_not_double_scale_inspect_or_screenshot() -> Result<(), Box<dyn std::error::Error>> {
    let initial = ViewportMetrics::new(800.0, 500.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, initial_viewport: initial, ..DesktopHostConfig::default() });
    let win_id = rt.open_window(None, initial)?;
    {
        let app = rt.window_mut(win_id)?.app_mut();
        app.set_ui_scale(1.5);
        let _ = app.dispatch_control_line(r#"{"id":"open-about","method":"app.about"}"#);
    }

    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());
    let ctx = new_egui_context();
    let mut native = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let raw_for_ctx = |c: &egui::Context| {
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
        let mut out = ctx.run_ui(raw_for_ctx(&ctx), |ui| native.step_ui(ui));
        out.textures_delta.clear();
        if native.geometry_stamp().is_some_and(|s| s.matches_target(1200, 750, 1.5)) {
            break;
        }
    }
    assert!(native.geometry_stamp().ok_or("missing settled GeometryStamp")?.matches_target(1200, 750, 1.5));

    let script = format!(
        "{{\"id\":\"a\",\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n\
         {{\"id\":\"s\",\"method\":\"ui.screenshot\",\"params\":{{\"target\":\"full_window\"}}}}\n\
         {{\"id\":\"i\",\"method\":\"ui.inspect\",\"params\":{{\"scope\":\"dialog\",\"include_bounds\":true}}}}\n",
        auth.as_str()
    );
    let replies = run_scripted_live_control(&mut native, &ctx, &bridge, script, raw_for_ctx)?;
    assert_eq!(replies.len(), 3);
    assert_eq!((&replies[1]["ok"], &replies[1]["result"]["width_px"], &replies[1]["result"]["height_px"]), (&json!(true), &json!(1200), &json!(750)));
    assert_eq!(png_dimensions(&decode_base64(replies[1]["result"]["data_base64"].as_str().ok_or("missing data_base64")?)?), Some((1200, 750)));

    assert_eq!(replies[2]["ok"], json!(true));
    let bounds = replies[2]["result"]["dialog"]["bounds"].as_array().ok_or("missing dialog.bounds")?;
    let [bx, by, bw, bh] = [0, 1, 2, 3].map(|i| bounds.get(i).and_then(Value::as_f64).unwrap_or(-1.0));
    assert!(bx >= 0.0 && by >= 0.0 && bx + bw <= 1202.0 && by + bh <= 752.0 && bw < 760.0, "double-scaled dialog bounds: {bounds:?}");
    Ok(())
}

#[test]
fn secondary_window_prefs_sync_per_request_token_64k_lift_and_live_screenshot_tab_idle() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Secondary windows hydrate & sync preferences; closing Window 2 persists synced prefs.
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, ..DesktopHostConfig::default() });
    let persisted = Arc::new(curvebook_platform::sync::Mutex::named("test.prefs", Vec::<(String, UiThemeMode)>::new()));
    let p_hook = Arc::clone(&persisted);
    rt.set_preferences_init_hook(|app: &mut UiApp| {
        let _ = app.set_locale("de");
        app.set_theme_mode(UiThemeMode::Dark);
    });
    rt.set_preferences_persist_hook(move |app: &UiApp| {
        if let Ok(mut g) = p_hook.lock() {
            g.push((app.locale.clone(), app.theme.mode));
        }
    });
    let w1 = rt.open_window(None, ViewportMetrics::default())?;
    assert_eq!((rt.window(w1)?.app().locale.as_str(), rt.window(w1)?.app().theme.mode), ("de", UiThemeMode::Dark));
    let w2 = rt.dispatch(DesktopHostEvent::MenuAction { window_id: w1, command_id: String::from("window.new") })?.opened_window_id.ok_or("win2")?;
    assert_eq!((rt.window(w2)?.app().locale.as_str(), rt.window(w2)?.app().theme.mode), ("de", UiThemeMode::Dark));
    let _ = rt.dispatch(DesktopHostEvent::MenuAction { window_id: w1, command_id: String::from("ui.locale.set:fr") })?;
    let _ = rt.dispatch(DesktopHostEvent::MenuAction { window_id: w1, command_id: String::from("ui.theme.set") })?;
    let mode = rt.window(w1)?.app().theme.mode;
    assert!(mode != UiThemeMode::Dark && rt.window(w2)?.app().locale == "fr" && rt.window(w2)?.app().theme.mode == mode);
    let _ = rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: w2, force: false })?;
    assert_eq!(persisted.lock().map_err(|e| e.to_string())?.last(), Some(&(String::from("fr"), mode)));

    // 2. Per-request `"token"` promotes auth to lift 64 KiB cap & `project.save` preserves canonical `file_path()`.
    let td = TempDirGuard::new("token-64k-canon")?;
    let wd = td.child("wd");
    let sub = wd.join("sub");
    fs::create_dir_all(&sub)?;
    let mut rt_io = DesktopHostRuntime::new(DesktopHostConfig {
        recovery_root: None,
        path_policy: ExternalPathPolicy::workdir(&wd)?,
        ..DesktopHostConfig::default()
    });
    let wio = rt_io.open_window(None, ViewportMetrics::default())?;
    let auth = ControlAuth::from_token("rem-hosts-token-1321")?;
    let (bridge, channel) = LiveControlBridge::new(auth);
    let frames = format!(
        "{}\n{}\n",
        json!({ "id": "f1", "token": "rem-hosts-token-1321", "method": "engine.execute", "params": { "command_id": "project.save", "params": { "path": "sub/saved.curvebook" } } }),
        json!({ "id": "f2", "token": "rem-hosts-token-1321", "method": "engine.execute", "params": { "command_id": "sheet.createData", "params": { "sheet_id": "d2", "title": "x".repeat(70 * 1024), "kind": "Column" } } }),
    );
    let io_h = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let (mut r, mut w) = (BufReader::new(Cursor::new(frames.into_bytes())), Vec::new());
        serve_live_control_io(&mut r, &mut w, &bridge, |_| {})?;
        Ok(w)
    });
    while let Ok(req) = channel.request_rx.recv() {
        let out = rt_io.window_mut(wio)?.dispatch_live_control_frame_with_hook(&req.raw_bytes, &req.auth, req.conn_authenticated, |_, _| Ok(()));
        let _ = req.reply_tx.send(out);
    }
    let replies = read_jsonl_values(&io_h.join().map_err(|_| "io panic")??)?;
    assert_eq!((replies.len(), &replies[0]["ok"]), (2, &json!(true)));
    assert!(!replies[1]["error"].as_str().unwrap_or("").contains("control line exceeds"));
    assert_eq!(rt_io.window(wio)?.app().session().file_path(), Some(fs::canonicalize(sub.join("saved.curvebook"))?.as_path()));

    // 3. Live GUI window: per-request token deferred screenshot, workspace/scale_factor, dialog Conflict/Open, symlink reject, int tab switch, 0 idle repaints.
    let mut doc = ProjectDocument::new("GUI Window Study")?;
    let mut rs = ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?)?;
    rs.status = ResultStatus::UpToDate;
    let num = |v: f64| ResultCell::Number { value: v, prefix: None, units: None };
    for (id, title, kind, v) in [("tab_a", "Tab A", ResultTabKind::MainSummary, 1.0), ("tab_b", "Tab B", ResultTabKind::Residuals, 2.0)] {
        rs.tabs.push(ResultTab {
            id: id.into(),
            title: title.into(),
            kind,
            column_headers: vec!["V".into()],
            row_labels: vec!["1".into()],
            rows: vec![vec![num(v)]],
            footnotes: Vec::new(),
        });
    }
    doc.add_result_sheet(rs)?;
    let _ = doc.select_in_navigator(&curvebook_doc::SheetId::new("r1"));
    let mut rt_g = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, in_window_menus: true, ..DesktopHostConfig::default() });
    let wg = rt_g.open_window_with_session(Session::from_document(doc), ViewportMetrics::default(), false)?;
    let (req_tx, req_rx) = std::sync::mpsc::channel::<LiveControlRequest>();
    let ch = LiveControlChannel {
        request_rx: req_rx,
        egui_ctx_cell: Arc::new(std::sync::OnceLock::new()),
        shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        post_hook: None,
    };
    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt_g, wg, None).with_live_control(ch), egui::vec2(1024.0, 680.0));
    let repaints = Arc::new(AtomicUsize::new(0));
    let repaints_cb = Arc::clone(&repaints);
    h.ctx.set_request_repaint_callback(move |_| {
        repaints_cb.fetch_add(1, Ordering::Relaxed);
    });
    h.settle(3);

    let send =
        |payload: Value,
         authed: bool|
         -> Result<std::sync::mpsc::Receiver<Result<curvebook::desktop_host::LiveControlFrameOutput, String>>, Box<dyn std::error::Error>> {
            let (tx, rx) = std::sync::mpsc::channel();
            req_tx.send(LiveControlRequest {
                raw_bytes: format!("{payload}\n").into_bytes(),
                auth: ControlAuth::from_token("rem-hosts-token-gui")?,
                conn_authenticated: authed,
                reply_tx: tx,
            })?;
            Ok(rx)
        };
    let rx_shot =
        send(json!({ "id": "s1", "token": "rem-hosts-token-gui", "method": "ui.screenshot", "params": { "target": "full_window" } }), false)?;
    let rx_tab = send(json!({ "id": "t1", "token": "rem-hosts-token-gui", "method": "ui.switch_tab", "params": { "tab": 1 } }), false)?;
    h.settle(12);
    let shot_out = rx_shot.recv_timeout(Duration::from_secs(2))??;
    assert_eq!(shot_out.auth_succeeded, Some(true));
    let sj: Value = serde_json::from_slice(&shot_out.response_bytes)?;
    assert_eq!(
        (&sj["ok"], &sj["result"]["region"], &sj["result"]["format"], &sj["result"]["mime_type"]),
        (&json!(true), &json!("full_window"), &json!("png"), &json!("image/png"))
    );
    assert!(
        sj["result"]["bytes_len"].as_u64().is_some_and(|n| n > 0)
            && sj["result"]["path"].is_null()
            && sj["result"]["data_base64"].as_str().is_some_and(|s| !s.is_empty())
            && sj["result"].get("png_base64").is_none()
    );
    let tj: Value = serde_json::from_slice(&rx_tab.recv_timeout(Duration::from_secs(2))??.response_bytes)?;
    assert_eq!(
        (&tj["ok"], &tj["result"]["tab_id"], h.app.runtime().window(wg)?.app().canvas_dialogs.result_view.active_tab_index),
        (&json!(true), &json!("tab_b"), 1)
    );

    let rx_ws = send(json!({ "id": "ws", "method": "ui.screenshot", "params": { "target": "workspace", "scale_factor": 2.0 } }), true)?;
    let rx_nodlg = send(json!({ "id": "nd", "method": "ui.screenshot", "params": { "region": "dialog" } }), true)?;
    h.settle(12);
    let wsj: Value = serde_json::from_slice(&rx_ws.recv_timeout(Duration::from_secs(2))??.response_bytes)?;
    assert_eq!((&wsj["ok"], &wsj["result"]["region"], &wsj["result"]["dpi"]), (&json!(true), &json!("workspace"), &json!(192)));
    let ndj: Value = serde_json::from_slice(&rx_nodlg.recv_timeout(Duration::from_secs(2))??.response_bytes)?;
    assert_eq!((&ndj["ok"], &ndj["error_detail"]["code"]), (&json!(false), &json!("conflict")));

    h.app.runtime_mut().window_mut(wg)?.app_mut().canvas_dialogs.active_dialog = curvebook_ui_egui::dialogs_canvas::ActiveDialog::About;
    h.settle(3);
    let rx_dlg = send(json!({ "id": "dg", "method": "ui.screenshot", "params": { "target": "dialog" } }), true)?;
    h.settle(8);
    let dgj: Value = serde_json::from_slice(&rx_dlg.recv_timeout(Duration::from_secs(2))??.response_bytes)?;
    assert!(dgj["ok"] == json!(true) && dgj["result"]["region"] == json!("dialog") && dgj["result"]["bytes_len"].as_u64().is_some_and(|n| n > 0));
    h.app.runtime_mut().window_mut(wg)?.app_mut().canvas_dialogs.active_dialog = curvebook_ui_egui::dialogs_canvas::ActiveDialog::None;
    h.settle(3);

    #[cfg(unix)]
    {
        let (real_file, link_path) = (td.child("real.png"), td.child("link.png"));
        fs::write(&real_file, b"untouched")?;
        std::os::unix::fs::symlink(&real_file, &link_path)?;
        let rx_sym = send(
            json!({ "id": "sy", "method": "ui.screenshot", "params": { "target": "full_window", "path": link_path.display().to_string() } }),
            true,
        )?;
        h.settle(8);
        let syj: Value = serde_json::from_slice(&rx_sym.recv_timeout(Duration::from_secs(2))??.response_bytes)?;
        assert!(syj["ok"] == json!(false) && fs::read(&real_file)? == b"untouched");
    }

    repaints.store(0, Ordering::Relaxed);
    let _ = h.step_events(Vec::new());
    assert_eq!(repaints.load(Ordering::Relaxed), 0, "idle desktop frame must request 0 repaints");
    Ok(())
}
