#![forbid(unsafe_code)]
//! `R15.3` Cross-Host Menu Bar Parity (`muda` vs `egui::MenuBar`), Non-Main-Thread `muda` Descriptor
//! Construction Safety, and Wave 2 QA Finding #3 (`in_window_menus` / `ui.inspect` Coordinate Parity).

use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook::live_control::{LiveControlBridge, serve_live_control_io};
use curvebook::mac_menu::{NATIVE_MENU_BAR_TITLES, collect_egui_menubar_parity, collect_menu_parity_entries, verify_menu_parity};
use curvebook_ui_egui::i18n::BETA_ACTIVE_LOCALES;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{ControlAuth, UiApp};
use eframe::egui;
use serde_json::{Value, json};

use crate::common::{InteractiveControlClient, find_bounds_in_snapshot};

fn bounds_close(a: [f64; 4], b: [f64; 4], tol: f64) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= tol)
}

#[test]
fn muda_vs_egui_menubar_1_to_1_parity_across_locales_and_sheet_states() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(NATIVE_MENU_BAR_TITLES.len(), 8);
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);

    for &loc in BETA_ACTIVE_LOCALES {
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
            in_window_menus: true,
            new_instance: true,
            locale_override: Some(loc.to_owned()),
            initial_viewport: metrics,
            ..DesktopHostConfig::default()
        });
        let win_id = rt.open_window(None, metrics)?;

        // State 1: Empty document parity
        let chrome_empty = rt.window(win_id)?.chrome_state();
        let egui_empty = collect_egui_menubar_parity(rt.window(win_id)?.app());
        verify_menu_parity(&chrome_empty, &egui_empty)?;

        #[cfg(target_os = "macos")]
        {
            let mut mac_host = curvebook::mac_menu::MacNativeMenuHost::new();
            mac_host.sync_from_chrome(&chrome_empty)?;
            assert!(!mac_host.is_installed_for_nsapp(), "headless sync must never call init_for_nsapp()");
            verify_menu_parity(&chrome_empty, mac_host.items_parity_entries())?;
            assert_eq!(mac_host.items_parity_entries(), egui_empty.as_slice());
            let total_accels: usize = mac_host.muda_section_specs().iter().flat_map(|s| s.items.iter()).filter(|i| i.accelerator.is_some()).count();
            assert!(total_accels >= 10, "expected parsed muda accelerators across menu commands, got {total_accels}");
        }

        // State 2: DataSheet + ResultSheet + GraphSheet contextual enablement symmetry.
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

    assert_eq!(collect_egui_menubar_parity(&UiApp::new()).len(), 8);
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
    let (logical_w, logical_h) = (1120.0_f32, 680.0_f32);
    let initial = ViewportMetrics::new(f64::from(logical_w), f64::from(logical_h), scale);

    let mut rt =
        DesktopHostRuntime::new(DesktopHostConfig { in_window_menus, new_instance: true, initial_viewport: initial, ..DesktopHostConfig::default() });
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

    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());
    let (mut client, mut srv_reader, mut srv_writer) = InteractiveControlClient::pair(None);

    let bridge_srv = bridge.clone();
    let srv_handle = std::thread::spawn(move || -> io::Result<()> { serve_live_control_io(&mut srv_reader, &mut srv_writer, &bridge_srv, |_| {}) });

    let token = auth.as_str().to_owned();
    let client_handle = std::thread::spawn(move || -> Result<Value, String> {
        assert_eq!(client.send(&json!({ "id": "0", "method": "auth", "params": { "token": token } }))?.get("ok"), Some(&json!(true)));
        let insp = client.send(&json!({
            "id": "1", "method": "ui.inspect",
            "params": { "include_bounds": true, "metrics": { "logical_width": f64::from(logical_w), "logical_height": f64::from(logical_h), "scale_factor": scale } }
        }))?;
        assert_eq!(insp.get("ok"), Some(&json!(true)));
        let analyze_b = find_bounds_in_snapshot(&insp, "toolbar.analyze").ok_or("missing toolbar.analyze bounds in ui.inspect")?;
        let gesture = client.send(&json!({
            "id": "2", "method": "ui.pointer_gesture",
            "params": { "action": "click", "x": (analyze_b[0] + analyze_b[2] * 0.5) / scale, "y": (analyze_b[1] + analyze_b[3] * 0.5) / scale }
        }))?;
        assert_eq!(gesture.get("ok"), Some(&json!(true)));
        let _ = client.send(&json!({ "id": "3", "method": "app.quit", "params": {} }));
        Ok(insp)
    });

    let mut native_app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);
    let ctx = egui::Context::default();
    if use_bundled_fonts {
        ctx.set_fonts(build_egui_font_definitions(None));
    }
    let step_once = |native: &mut CurveBookNativeApp| {
        let mut raw =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(logical_w, logical_h))), ..Default::default() };
        if let Some(vp) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
            vp.native_pixels_per_point = Some(scale as f32);
        }
        native.drain_synthetic_events_into(&mut raw);
        let mut out = ctx.run_ui(raw, |ui| native.step_ui(ui));
        out.textures_delta.clear();
    };
    for _ in 0..4 {
        step_once(&mut native_app);
    }

    let mut offscreen_app = native_app.runtime().window(win_id)?.app().clone();
    let offscreen_val = json!({ "result": serde_json::to_value(offscreen_app.inspect_egui_shell_snapshot(initial, None, true))? });

    let mut live_bounds_before = HashMap::new();
    for wid in ["toolbar.home", "toolbar.command_palette", "toolbar.analyze", "toolbar.split"] {
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
        live_bounds_before.insert(wid, live_px);
        let off_px = find_bounds_in_snapshot(&offscreen_val, wid).ok_or_else(|| format!("missing offscreen `{wid}`"))?;
        assert!((live_px[1] - off_px[1]).abs() <= 0.5 && (live_px[3] - off_px[3]).abs() <= 0.5, "Y mismatch `{wid}`: {live_px:?} vs {off_px:?}");
        if use_bundled_fonts {
            assert!(bounds_close(live_px, off_px, 0.5), "full-bounds mismatch `{wid}`: {live_px:?} vs {off_px:?}");
        }
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    while !client_handle.is_finished() && Instant::now() < deadline {
        step_once(&mut native_app);
        std::thread::sleep(Duration::from_millis(2));
    }
    bridge.signal_shutdown();
    let live_insp = client_handle.join().map_err(|_| "client panic")??;
    let _ = srv_handle.join();

    for wid in ["toolbar.home", "toolbar.analyze", "toolbar.command_palette"] {
        let live_ctrl_px = find_bounds_in_snapshot(&live_insp, wid).ok_or("missing ctrl bounds")?;
        let expected_live_px = live_bounds_before[wid];
        assert!(bounds_close(live_ctrl_px, expected_live_px, 0.5), "live ui.inspect vs inspect_meta diverged for `{wid}`");
        let off_px = find_bounds_in_snapshot(&offscreen_val, wid).ok_or("missing offscreen bounds")?;
        assert!((live_ctrl_px[1] - off_px[1]).abs() <= 0.5 && (live_ctrl_px[3] - off_px[3]).abs() <= 0.5, "live vs offscreen Y diverged for `{wid}`");
        if use_bundled_fonts {
            assert!(bounds_close(live_ctrl_px, off_px, 0.5), "live vs offscreen bounds diverged for `{wid}`");
        }
    }

    assert!(
        matches!(
            native_app.runtime().window(win_id)?.app().canvas_dialogs.active_dialog,
            curvebook_ui_egui::dialogs_canvas::ActiveDialog::AnalyzeSheet { .. }
        ),
        "expected ui.inspect analyze coordinate click to open AnalyzeSheet (bundled={use_bundled_fonts}, in_window_menus={in_window_menus}, scale={scale})"
    );
    Ok(())
}
