#![forbid(unsafe_code)]
//! `R15.3` Cross-Host Menu Bar Parity (`muda` vs `egui::MenuBar`), Non-Main-Thread `muda` Descriptor
//! Construction Safety, and Wave 2 QA Finding #3 (`in_window_menus` / `ui.inspect` Coordinate Parity).

use std::io::{self, BufRead, BufReader, Write};
use std::sync::mpsc;

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook::live_control::{LiveControlBridge, serve_live_control_io};
use curvebook::mac_menu::{NATIVE_MENU_BAR_TITLES, collect_egui_menubar_parity, collect_menu_parity_entries, verify_menu_parity};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::i18n::BETA_ACTIVE_LOCALES;
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::{Value, json};

struct ChannelPipeReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl ChannelPipeReader {
    const fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self { rx, buf: Vec::new(), pos: 0 }
    }
}

impl io::Read for ChannelPipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = (self.buf.len() - self.pos).min(out.len());
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct ChannelPipeWriter {
    tx: mpsc::Sender<Vec<u8>>,
}

impl io::Write for ChannelPipeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.tx.send(data.to_vec()).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "closed"))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct InteractiveClient {
    reader: BufReader<ChannelPipeReader>,
    writer: ChannelPipeWriter,
}

impl InteractiveClient {
    fn send(&mut self, req: &Value) -> Result<Value, String> {
        let mut raw = serde_json::to_vec(req).map_err(|e| e.to_string())?;
        raw.push(b'\n');
        self.writer.write_all(&raw).map_err(|e| e.to_string())?;
        self.writer.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        self.reader.read_line(&mut line).map_err(|e| e.to_string())?;
        serde_json::from_str(&line).map_err(|e| format!("invalid json `{line}`: {e}"))
    }
}

fn find_bounds_in_snapshot(snap: &Value, target_wid: &str) -> Option<[f64; 4]> {
    for group in ["toolbar", "tabs"] {
        if let Some(arr) = snap.pointer(&format!("/result/{group}")).and_then(Value::as_array) {
            for item in arr {
                if item.get("widget_id").and_then(Value::as_str) == Some(target_wid)
                    && let Some(bounds) = item.get("bounds").and_then(Value::as_array)
                    && bounds.len() == 4
                {
                    return Some([bounds[0].as_f64()?, bounds[1].as_f64()?, bounds[2].as_f64()?, bounds[3].as_f64()?]);
                }
            }
        }
    }
    None
}

fn bounds_close(a: [f64; 4], b: [f64; 4], tol: f64) -> bool {
    (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol && (a[2] - b[2]).abs() <= tol && (a[3] - b[3]).abs() <= tol
}

#[test]
fn muda_vs_egui_menubar_1_to_1_parity_across_locales_and_sheet_states() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(NATIVE_MENU_BAR_TITLES.len(), 10);

    for &loc in BETA_ACTIVE_LOCALES {
        let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
            in_window_menus: true,
            new_instance: true,
            gui_only: false,
            ui_backend: UiBackend::Egui,
            locale_override: Some(loc.to_owned()),
            theme_override: None,
            recovery_root: None,
            initial_viewport: metrics,
        });
        let win_id = rt.open_window(None, metrics)?;

        // State 1: Empty document parity
        let chrome_empty = rt.window(win_id)?.chrome_state();
        let egui_empty = collect_egui_menubar_parity(rt.window(win_id)?.app());
        verify_menu_parity(&chrome_empty, &egui_empty)?;

        #[cfg(target_os = "macos")]
        {
            // Run `MacNativeMenuHost::sync_from_chrome` strictly in headless mode on test thread (must NOT touch NSMenu or panic).
            let mut mac_host = curvebook::mac_menu::MacNativeMenuHost::new();
            mac_host.sync_from_chrome(&chrome_empty)?;
            assert!(!mac_host.is_installed_for_nsapp(), "headless sync must never call init_for_nsapp()");
            verify_menu_parity(&chrome_empty, mac_host.items_parity_entries())?;
            assert_eq!(mac_host.items_parity_entries(), egui_empty.as_slice());

            let total_accels: usize = mac_host.muda_section_specs().iter().flat_map(|s| s.items.iter()).filter(|i| i.accelerator.is_some()).count();
            assert!(total_accels >= 10, "expected parsed muda accelerators across menu commands, got {total_accels}");
        }

        // State 2: DataSheet + ResultSheet + GraphSheet transitions update contextual enablement symmetrically.
        {
            let win = rt.window_mut(win_id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-p", "kind": "column", "cols": 2, "rows": 4 }))?;
            for c in 0..2 {
                for r in 0..4 {
                    win.execute_and_journal("table.setCell", json!({ "sheet_id": "ds-p", "row": r, "col": c, "value": 5.0 + (r + c) as f64 }))?;
                }
            }
            win.execute_and_journal("analysis.run", json!({ "source_sheet_id": "ds-p", "kind": "unpaired_t_test", "sheet_id": "rs-p" }))?;
            win.execute_and_journal("graph.create", json!({ "sheet_id": "gs-p", "data_sheet_id": "ds-p" }))?;
        }
        let chrome_rich = rt.window(win_id)?.chrome_state();
        let egui_rich = collect_egui_menubar_parity(rt.window(win_id)?.app());
        verify_menu_parity(&chrome_rich, &egui_rich)?;

        #[cfg(target_os = "macos")]
        {
            let mut mac_host = curvebook::mac_menu::MacNativeMenuHost::new();
            mac_host.sync_from_chrome(&chrome_rich)?;
            verify_menu_parity(&chrome_rich, mac_host.items_parity_entries())?;
            assert_eq!(mac_host.items_parity_entries(), egui_rich.as_slice());

            // Enablement check: `ui.editParams.open` enabled now that ResultSheet exists.
            let edit_mac = mac_host
                .items_parity_entries()
                .iter()
                .flat_map(|s| s.items.iter())
                .find(|i| i.command_id == "ui.editParams.open")
                .ok_or("missing ui.editParams.open")?;
            let edit_egui =
                egui_rich.iter().flat_map(|s| s.items.iter()).find(|i| i.command_id == "ui.editParams.open").ok_or("missing ui.editParams.open")?;
            assert!(edit_mac.enabled && edit_egui.enabled);
        }
    }

    // Verify helper mirror on UiApp itself.
    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);
    assert_eq!(collect_egui_menubar_parity(&app).len(), 7);
    let mut helper_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let helper_win = helper_rt.open_window(None, ViewportMetrics::new(800.0, 600.0, 1.0))?;
    let _ = collect_menu_parity_entries(&helper_rt.window(helper_win)?.chrome_state());
    Ok(())
}

#[test]
fn wave2_qa_finding3_in_window_menus_and_native_mode_inspect_coordinate_parity() -> Result<(), Box<dyn std::error::Error>> {
    for &use_bundled_fonts in &[false, true] {
        for &in_window_menus in &[true, false] {
            for &scale in &[1.0_f64, 1.5] {
                run_qa3_parity_case(use_bundled_fonts, in_window_menus, scale)?;
            }
        }
    }
    Ok(())
}

fn run_qa3_parity_case(use_bundled_fonts: bool, in_window_menus: bool, scale: f64) -> Result<(), Box<dyn std::error::Error>> {
    let logical_w = 1120.0_f32;
    let logical_h = 680.0_f32;
    let initial = ViewportMetrics::new(f64::from(logical_w), f64::from(logical_h), scale);

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus,
        new_instance: true,
        gui_only: false,
        ui_backend: UiBackend::Egui,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: initial,
    });
    let win_id = rt.open_window(None, initial)?;
    {
        let win = rt.window_mut(win_id)?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-qa3", "kind": "column", "cols": 3, "rows": 8 }))?;
        for col in 0..3 {
            for row in 0..5 {
                win.execute_and_journal(
                    "table.setCell",
                    json!({ "sheet_id": "ds-qa3", "row": row, "col": col, "value": 12.0 + col as f64 + row as f64 }),
                )?;
            }
        }
    }

    let auth = curvebook_ui_egui::ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());
    let (c2s_tx, c2s_rx) = mpsc::channel::<Vec<u8>>();
    let (s2c_tx, s2c_rx) = mpsc::channel::<Vec<u8>>();

    let bridge_srv = bridge.clone();
    let srv_handle = std::thread::spawn(move || -> io::Result<()> {
        let mut reader = BufReader::new(ChannelPipeReader::new(c2s_rx));
        let mut writer = ChannelPipeWriter { tx: s2c_tx };
        serve_live_control_io(&mut reader, &mut writer, &bridge_srv, |_| {})
    });

    let token = auth.as_str().to_owned();
    let client_handle = std::thread::spawn(move || -> Result<Value, String> {
        let mut client = InteractiveClient { reader: BufReader::new(ChannelPipeReader::new(s2c_rx)), writer: ChannelPipeWriter { tx: c2s_tx } };
        let r_auth = client.send(&json!({ "id": "0", "method": "auth", "params": { "token": token } }))?;
        assert_eq!(r_auth["ok"], json!(true));

        let insp = client.send(&json!({
            "id": "1",
            "method": "ui.inspect",
            "params": { "include_bounds": true, "metrics": { "logical_width": f64::from(logical_w), "logical_height": f64::from(logical_h), "scale_factor": scale } }
        }))?;
        assert_eq!(insp["ok"], json!(true));
        let analyze_b = find_bounds_in_snapshot(&insp, "toolbar.analyze").ok_or("missing toolbar.analyze bounds in ui.inspect")?;
        let click_x = (analyze_b[0] + analyze_b[2] * 0.5) / scale;
        let click_y = (analyze_b[1] + analyze_b[3] * 0.5) / scale;
        let gesture = client.send(&json!({
            "id": "2",
            "method": "ui.pointer_gesture",
            "params": { "action": "click", "x": click_x, "y": click_y }
        }))?;
        assert_eq!(gesture["ok"], json!(true));
        let _ = client.send(&json!({ "id": "3", "method": "app.quit", "params": {} }));
        Ok(insp)
    });

    let mut native_app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);
    let ctx = egui::Context::default();
    if use_bundled_fonts {
        ctx.set_fonts(build_egui_font_definitions(None));
    }

    let step_once = |native: &mut CurveBookNativeApp, raw_events: Vec<egui::Event>| {
        let mut raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(logical_w, logical_h))),
            events: raw_events,
            ..Default::default()
        };
        if let Some(vp) = raw_input.viewports.get_mut(&egui::ViewportId::ROOT) {
            vp.native_pixels_per_point = Some(scale as f32);
        }
        native.drain_synthetic_events_into(&mut raw_input);
        let mut out = ctx.run_ui(raw_input, |ui| native.step_ui(ui));
        out.textures_delta.clear();
    };

    // Settle initial frames.
    for _ in 0..4 {
        step_once(&mut native_app, Vec::new());
    }

    // Capture live host `inspect_meta` bounds in physical px BEFORE pointer click.
    let mut offscreen_app = native_app.runtime().window(win_id)?.app().clone();
    let offscreen_snap = offscreen_app.inspect_egui_shell_snapshot(initial, None, true);
    let offscreen_val = json!({ "result": serde_json::to_value(&offscreen_snap)? });

    for wid in ["toolbar.home", "toolbar.command_palette", "toolbar.analyze", "sheet.tab.ds-qa3"] {
        let live_entry = native_app
            .inspect_meta()
            .find_by_widget_id(wid)
            .ok_or_else(|| format!("missing live inspect_meta `{wid}` (in_window_menus={in_window_menus})"))?;
        let live_px = [
            f64::from(live_entry.clipped_rect.min.x) * scale,
            f64::from(live_entry.clipped_rect.min.y) * scale,
            f64::from(live_entry.clipped_rect.width()) * scale,
            f64::from(live_entry.clipped_rect.height()) * scale,
        ];
        let off_px =
            find_bounds_in_snapshot(&offscreen_val, wid).ok_or_else(|| format!("missing offscreen `{wid}` (in_window_menus={in_window_menus})"))?;
        assert!(
            (live_px[1] - off_px[1]).abs() <= 0.5 && (live_px[3] - off_px[3]).abs() <= 0.5,
            "Wave 2 QA #3 Y-offset/height mismatch for `{wid}` (bundled={use_bundled_fonts}, in_window_menus={in_window_menus}, scale={scale}): live={live_px:?} vs offscreen={off_px:?}"
        );
        if use_bundled_fonts {
            assert!(
                bounds_close(live_px, off_px, 0.5),
                "Wave 2 QA #3 full-bounds mismatch for `{wid}` (in_window_menus={in_window_menus}, scale={scale}): live={live_px:?} vs offscreen={off_px:?}"
            );
        }
    }

    // Drive frames until control client finishes inspect + coordinate click + quit.
    for _ in 0..24 {
        step_once(&mut native_app, Vec::new());
        if client_handle.is_finished() {
            break;
        }
    }
    let live_insp = client_handle.join().map_err(|_| "client panic")??;
    drop(bridge);
    let _ = srv_handle.join();

    for wid in ["toolbar.home", "toolbar.analyze", "toolbar.command_palette"] {
        let live_ctrl_px = find_bounds_in_snapshot(&live_insp, wid).ok_or("missing ctrl bounds")?;
        let off_px = find_bounds_in_snapshot(&offscreen_val, wid).ok_or("missing offscreen bounds")?;
        assert!(
            bounds_close(live_ctrl_px, off_px, 0.5),
            "live ui.inspect vs offscreen snapshot diverged for `{wid}` (in_window_menus={in_window_menus}, scale={scale}): {live_ctrl_px:?} vs {off_px:?}"
        );
    }

    // Coordinate click returned by `ui.inspect` must open AnalyzeSheet even when `in_window_menus == true`!
    assert!(
        matches!(
            native_app.runtime().window(win_id)?.app().canvas_dialogs.active_dialog,
            curvebook_ui_egui::dialogs_canvas::ActiveDialog::AnalyzeSheet { .. }
        ),
        "expected ui.inspect analyze coordinate click to open AnalyzeSheet (bundled={use_bundled_fonts}, in_window_menus={in_window_menus}, scale={scale})"
    );
    Ok(())
}
