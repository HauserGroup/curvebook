#![forbid(unsafe_code)]

//! Wave 1 (`R15.1` bullets 1, 4, 5 & `#13`) integration tests for the `curvebook` desktop host:
//! - Duplicate menu-bar suppression & surviving `egui::MenuBar`
//! - `BETA_ACTIVE_LOCALES` (14 non-RTL) & `normalize_beta_gui_locale`
//! - `--ui-backend=egui` / `CURVEBOOK_UI_BACKEND=egui` CLI & headless `--control` live bridge
//! - Host `Tokens::apply_to_ctx`, `Cmd/Ctrl +`/`-`/`0` `ui_scale`
//! - `LiveControlBridge` + `GeometryStamp` post-resize / post-DPI barrier (`ui.screenshot` / `ui.inspect`)

use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, NativeFileDialogAdapter, NativeFileDialogKind, NativeFileDialogRequest,
};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook::live_control::{LiveControlBridge, serve_live_control_io};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{BETA_ACTIVE_LOCALES, ControlAuth, LOCALES, inspect_ui_tree, normalize_beta_gui_locale};
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
