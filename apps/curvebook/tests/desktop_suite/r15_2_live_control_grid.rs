#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave 2 (`R15.2`) / R15.5 live `--control` Column grid acceptance tests:
//! drives `CurveBookNativeApp` + `LiveControlBridge` (`serve_live_control_io`) interactively
//! to verify off-screen culling, scroll reveal, coordinate click selection, pixel diff, and
//! `GeometryStamp` post-resize/DPI settlement.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use curvebook::gui_window::CurveBookNativeApp;
use curvebook::live_control::{LiveControlBridge, serve_live_control_io};
use curvebook_ui_egui::ControlAuth;
use eframe::egui;
use serde_json::{Value, json};

use crate::common::{InteractiveControlClient, bounds_rect, decode_base64, new_egui_context, open_host_window, png_dimensions};

fn find_grid_cell<'a>(inspect_resp: &'a Value, widget_id: &str) -> Option<&'a Value> {
    inspect_resp.pointer("/result/grid_cells")?.as_array()?.iter().find(|n| n.get("widget_id").and_then(Value::as_str) == Some(widget_id))
}

fn scroll_to_target_cell(client: &mut InteractiveControlClient, top_bounds: [f64; 4]) -> Result<[f64; 4], String> {
    let scroll_x = top_bounds[0] + top_bounds[2] * 0.5;
    let scroll_y = top_bounds[1] + top_bounds[3] * 0.5;
    let metrics = json!({ "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 });
    let mut last_summary = String::new();
    for step in 0..16 {
        let scr = client.send(&json!({
            "id": format!("scroll-{step}"), "method": "ui.pointer_gesture",
            "params": { "action": "scroll", "x": scroll_x, "y": scroll_y, "delta_x": 0.0, "delta_y": -180.0, "metrics": metrics }
        }))?;
        assert_eq!(scr.get("ok"), Some(&json!(true)), "scroll step {step} failed: {scr}");

        let mid = client.send(&json!({
            "id": format!("mid-inspect-{step}"), "method": "ui.inspect",
            "params": { "scope": "grid", "include_bounds": true, "metrics": metrics }
        }))?;
        if step == 0 || step == 15 {
            let sample: Vec<&str> = mid
                .pointer("/result/grid_cells")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| c.get("widget_id").and_then(Value::as_str))
                .filter(|id| id.starts_with("grid.cell."))
                .take(6)
                .collect();
            last_summary = format!("step={step} top={top_bounds:?} sample={sample:?} c45={:?}", find_grid_cell(&mid, "grid.cell.45.1.0"));
        }
        if let Some(cell) = find_grid_cell(&mid, "grid.cell.45.1.0")
            && let Some(b) = bounds_rect(cell)
            && b[2] > 0.0
            && b[3] > 0.0
            && b[1] + b[3] <= 660.0
        {
            assert_eq!(cell.pointer("/value/text"), Some(&json!("987.65")), "unexpected text for revealed cell: {cell}");
            return Ok(b);
        }
    }
    Err(format!("scrolling did not reveal grid.cell.45.1.0 within 16 steps ({last_summary})"))
}

fn verify_resize_geometry_stamp(
    client: &mut InteractiveControlClient,
    resize_trigger: &AtomicBool,
    resize_settled: &AtomicBool,
    pre_resize_bounds: [f64; 4],
) -> Result<(), String> {
    resize_trigger.store(true, Ordering::Release);
    let wait_start = Instant::now();
    while !resize_settled.load(Ordering::Acquire) {
        if wait_start.elapsed() > Duration::from_secs(5) {
            return Err(String::from("timed out waiting for GeometryStamp resize settle"));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let shot = client.send(&json!({ "id": "7", "method": "ui.screenshot", "params": { "target": "full_window" } }))?;
    assert_eq!(shot.get("ok"), Some(&json!(true)), "resized screenshot failed: {shot}");
    assert_eq!(shot.pointer("/result/width_px"), Some(&json!(1680)));
    assert_eq!(shot.pointer("/result/height_px"), Some(&json!(1050)));
    assert_eq!(shot.pointer("/result/dpi"), Some(&json!(144)));
    let png = decode_base64(shot.pointer("/result/data_base64").and_then(Value::as_str).ok_or("missing resized data_base64")?)?;
    assert_eq!(png_dimensions(&png), Some((1680, 1050)));

    let insp = client.send(&json!({ "id": "7b", "method": "ui.inspect", "params": { "scope": "grid", "include_bounds": true } }))?;
    assert_eq!(insp.get("ok"), Some(&json!(true)), "resized inspect failed: {insp}");
    let post_cell = find_grid_cell(&insp, "grid.cell.45.1.0").ok_or("expected grid.cell.45.1.0 to remain inspectable after 1.5x DPI resize")?;
    let resized_bounds = bounds_rect(post_cell).ok_or("missing resized cell bounds")?;
    assert!(resized_bounds[2] > 0.0 && resized_bounds[3] > 0.0);
    assert!(resized_bounds[0] + resized_bounds[2] <= 1680.0 && resized_bounds[1] + resized_bounds[3] <= 1050.0);
    assert_ne!(resized_bounds, pre_resize_bounds, "post-resize GeometryStamp bounds must not stay stale pre-resize 1.0x bounds");
    Ok(())
}

#[test]
fn egui_live_control_column_scroll_inspect_click_screenshot() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, win_id) = open_host_window(false, false, None, 1280.0, 800.0)?;
    let auth = ControlAuth::generate()?;
    let (bridge, channel) = LiveControlBridge::new(auth.clone());
    let (mut client, mut srv_reader, mut srv_writer) = InteractiveControlClient::pair(Some(Duration::from_millis(20)));

    let bridge_srv = bridge.clone();
    let srv_handle = std::thread::spawn(move || -> io::Result<()> { serve_live_control_io(&mut srv_reader, &mut srv_writer, &bridge_srv, |_| {}) });

    let token = auth.as_str().to_owned();
    let resize_trigger = Arc::new(AtomicBool::new(false));
    let resize_settled = Arc::new(AtomicBool::new(false));
    let (resize_trigger_c, resize_settled_c) = (Arc::clone(&resize_trigger), Arc::clone(&resize_settled));

    let client_handle = std::thread::spawn(move || -> Result<(), String> {
        let m = json!({ "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 });
        for req in [
            json!({ "id": "0", "method": "auth", "params": { "token": token } }),
            json!({ "id": "1", "method": "sheet.createData", "params": { "sheet_id": "col-ctrl", "title": "Control Scroll Column", "kind": "column", "cols": 4, "rows": 80, "auto_graph": false } }),
            json!({ "id": "cell-0-0", "method": "table.setCell", "params": { "sheet_id": "col-ctrl", "row": 0, "col": 0, "value": 10.5 } }),
            json!({ "id": "cell-45-1", "method": "table.setCell", "params": { "sheet_id": "col-ctrl", "row": 45, "col": 1, "value": 987.65 } }),
        ] {
            let r = client.send(&req)?;
            assert_eq!(r.get("ok"), Some(&json!(true)), "setup {req} failed: {r}");
        }

        // 1. Pre-scroll inspect: cell (0,0,0) visible, row 45 off-screen.
        let pre = client.send(&json!({ "id": "2", "method": "ui.inspect", "params": { "scope": "grid", "include_bounds": true, "metrics": m } }))?;
        assert_eq!(pre.get("ok"), Some(&json!(true)));
        let top_bounds = bounds_rect(find_grid_cell(&pre, "grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?).ok_or("missing top bounds")?;
        assert!(top_bounds[2] > 0.0 && top_bounds[3] > 0.0);
        assert!(find_grid_cell(&pre, "grid.cell.45.1.0").and_then(bounds_rect).is_none(), "row 45 should be off-screen before scrolling");

        let pre_shot = client.send(
            &json!({ "id": "3", "method": "ui.screenshot", "params": { "target": "full_window", "width_px": 1280, "height_px": 800, "dpi": 96 } }),
        )?;
        assert_eq!(pre_shot.get("ok"), Some(&json!(true)));
        let pre_png = decode_base64(pre_shot.pointer("/result/data_base64").and_then(Value::as_str).ok_or("missing pre data_base64")?)?;
        assert_eq!(png_dimensions(&pre_png), Some((1280, 800)));

        // 2. Scroll until row 45 is visible, then click its inspected bounds center.
        let target_bounds = scroll_to_target_cell(&mut client, top_bounds)?;
        let click = client.send(&json!({
            "id": "4", "method": "ui.pointer_gesture",
            "params": { "action": "click", "x": target_bounds[0] + target_bounds[2] * 0.5, "y": target_bounds[1] + target_bounds[3] * 0.5, "metrics": m }
        }))?;
        assert_eq!(click.get("ok"), Some(&json!(true)), "click failed: {click}");

        let post = client.send(&json!({ "id": "5", "method": "ui.inspect", "params": { "scope": "grid", "include_bounds": true, "metrics": m } }))?;
        assert_eq!(post.get("ok"), Some(&json!(true)));
        let active = post.pointer("/result/grid_selection/active_cell").ok_or("missing active_cell")?;
        assert_eq!((active.get("row"), active.get("col")), (Some(&json!(45)), Some(&json!(1))));

        // 3. Screenshot reflects scrolled + selected pixels.
        let post_shot = client.send(
            &json!({ "id": "6", "method": "ui.screenshot", "params": { "target": "full_window", "width_px": 1280, "height_px": 800, "dpi": 96 } }),
        )?;
        assert_eq!(post_shot.get("ok"), Some(&json!(true)));
        let post_png = decode_base64(post_shot.pointer("/result/data_base64").and_then(Value::as_str).ok_or("missing post data_base64")?)?;
        assert_eq!(png_dimensions(&post_png), Some((1280, 800)));
        assert_ne!(pre_png, post_png, "post-scroll + post-click screenshot must differ from pre-scroll screenshot");

        // 4. Resize + 1.5x DPI transition settles GeometryStamp before screenshot/inspect.
        verify_resize_geometry_stamp(&mut client, &resize_trigger_c, &resize_settled_c, target_bounds)?;

        let quit = client.send(&json!({ "id": "8", "method": "app.quit" }))?;
        assert_eq!(quit.pointer("/result/quit"), Some(&json!(true)));
        Ok(())
    });

    let ctx = new_egui_context();
    let mut native = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);
    let deadline = Instant::now() + Duration::from_secs(15);
    while !client_handle.is_finished() && Instant::now() < deadline {
        let resized = resize_trigger.load(Ordering::Acquire);
        let (size, scale) = if resized { (egui::vec2(1120.0, 700.0), 1.5) } else { (egui::vec2(1280.0, 800.0), 1.0) };
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            focused: true,
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(scale), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        native.drain_synthetic_events_into(&mut raw);
        let mut out = ctx.run_ui(raw, |ui| native.step_ui(ui));
        out.textures_delta.clear();
        if resized && native.geometry_stamp().is_some_and(|s| s.settled && s.physical_width_px == 1680 && s.physical_height_px == 1050) {
            resize_settled.store(true, Ordering::Release);
        }
        std::thread::sleep(Duration::from_millis(4));
    }

    bridge.signal_shutdown();
    client_handle.join().map_err(|_| "control client thread panicked")??;
    srv_handle.join().map_err(|_| "control server thread panicked")??;
    native.finish_exit();
    Ok(())
}
