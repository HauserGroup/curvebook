#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Wave 2 (`R15.2`) / R15.5 live `--control` Column grid acceptance tests (`apps/curvebook/tests/r15_2_live_control_grid.rs`):
//! - Drives `CurveBookNativeApp` + `LiveControlBridge` (`serve_live_control_io`) interactively
//!   on the egui host:
//!   1. Creates/selects an `80`-row `Column` grid so row `45` starts off-screen.
//!   2. Verifies pre-scroll `ui.inspect` (`scope: "grid"`) omits off-screen `grid.cell.45.1.0`
//!      and captures baseline `ui.screenshot`.
//!   3. Scrolls via `ui.pointer_gesture` (`action: "scroll"`) until `grid.cell.45.1.0` enters view.
//!   4. Runs `ui.inspect` to verify the newly revealed cell's bounds and text (`"987.65"`).
//!   5. Clicks the revealed cell via `ui.pointer_gesture` at its inspected coordinates, verifying
//!      selection state and pixel-level change in `ui.screenshot`.
//!   6. Resizes viewport + DPI (`1120×700 @ 1.5x` -> `1680×1050 px`) and verifies `GeometryStamp`
//!      prevents stale pre-resize `ui.screenshot` / `ui.inspect` bounds.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook::live_control::{LiveControlBridge, serve_live_control_io};
use curvebook_ui_egui::ControlAuth;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::{Value, json};

struct ChannelPipeReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: VecDeque<u8>,
}

impl ChannelPipeReader {
    fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self { rx, buf: VecDeque::new() }
    }
}

impl Read for ChannelPipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.buf.is_empty() {
            match self.rx.recv_timeout(Duration::from_millis(20)) {
                Ok(chunk) if chunk.is_empty() => continue,
                Ok(chunk) => self.buf.extend(chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(io::Error::from(io::ErrorKind::WouldBlock)),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        let mut n = 0usize;
        for dst in out.iter_mut() {
            let Some(b) = self.buf.pop_front() else {
                break;
            };
            *dst = b;
            n += 1;
        }
        Ok(n)
    }
}

struct ChannelPipeWriter {
    tx: mpsc::Sender<Vec<u8>>,
}

impl Write for ChannelPipeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.tx.send(data.to_vec()).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "channel closed"))?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct InteractiveControlClient {
    reader: BufReader<ChannelPipeReader>,
    writer: ChannelPipeWriter,
}

impl InteractiveControlClient {
    fn send(&mut self, payload: &Value) -> Result<Value, String> {
        let line = serde_json::to_string(payload).map_err(|e| e.to_string())?;
        self.writer.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        self.writer.write_all(b"\n").map_err(|e| e.to_string())?;
        self.writer.flush().map_err(|e| e.to_string())?;

        let mut reply_line = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.reader.read_line(&mut reply_line) {
                Ok(0) => return Err(String::from("EOF waiting for control response")),
                Ok(_) => break,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(err) => return Err(err.to_string()),
            }
        }
        serde_json::from_str(&reply_line).map_err(|e| format!("invalid JSON reply `{reply_line}`: {e}"))
    }
}

use crate::common::{decode_base64, png_dimensions};

fn find_grid_cell<'a>(inspect_resp: &'a Value, widget_id: &str) -> Option<&'a Value> {
    inspect_resp.pointer("/result/grid_cells")?.as_array()?.iter().find(|n| n.get("widget_id").and_then(Value::as_str) == Some(widget_id))
}

fn bounds_rect(node: &Value) -> Option<[f64; 4]> {
    let arr = node.get("bounds")?.as_array()?;
    Some([arr.first()?.as_f64()?, arr.get(1)?.as_f64()?, arr.get(2)?.as_f64()?, arr.get(3)?.as_f64()?])
}

fn scroll_to_target_cell(client: &mut InteractiveControlClient, top_bounds: [f64; 4]) -> Result<[f64; 4], String> {
    let scroll_x = top_bounds[0] + top_bounds[2] * 0.5;
    let scroll_y = top_bounds[1] + top_bounds[3] * 0.5;
    let mut last_cells_summary = String::new();
    for step in 0..16 {
        let scr = client.send(&json!({
            "id": format!("scroll-{step}"),
            "method": "ui.pointer_gesture",
            "params": {
                "action": "scroll",
                "x": scroll_x,
                "y": scroll_y,
                "delta_x": 0.0,
                "delta_y": -180.0,
                "metrics": { "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 }
            }
        }))?;
        assert_eq!(scr.get("ok"), Some(&json!(true)), "scroll step {step} failed: {scr}");

        let mid_inspect = client.send(&json!({
            "id": format!("mid-inspect-{step}"),
            "method": "ui.inspect",
            "params": { "scope": "grid", "include_bounds": true, "metrics": { "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 } }
        }))?;
        if step == 0 || step == 15 {
            let vis_ids: Vec<&str> = mid_inspect
                .pointer("/result/grid_cells")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| c.get("widget_id").and_then(Value::as_str))
                .filter(|id| id.starts_with("grid.cell."))
                .take(6)
                .collect();
            last_cells_summary =
                format!("step={step} top={top_bounds:?} sample={vis_ids:?} c45={:?}", find_grid_cell(&mid_inspect, "grid.cell.45.1.0"));
        }
        if let Some(cell) = find_grid_cell(&mid_inspect, "grid.cell.45.1.0")
            && let Some(b) = bounds_rect(cell)
            && b[2] > 0.0
            && b[3] > 0.0
            && b[1] + b[3] <= 660.0
        {
            assert_eq!(cell.pointer("/value/text"), Some(&json!("987.65")), "unexpected text for revealed cell: {cell}");
            return Ok(b);
        }
    }
    Err(format!("scrolling did not reveal grid.cell.45.1.0 within 16 steps ({last_cells_summary})"))
}

fn verify_resize_geometry_stamp(
    client: &mut InteractiveControlClient,
    resize_trigger: &std::sync::atomic::AtomicBool,
    resize_settled: &std::sync::atomic::AtomicBool,
    pre_resize_bounds: [f64; 4],
) -> Result<(), String> {
    resize_trigger.store(true, std::sync::atomic::Ordering::Release);
    let wait_start = Instant::now();
    while !resize_settled.load(std::sync::atomic::Ordering::Acquire) {
        if wait_start.elapsed() > Duration::from_secs(5) {
            return Err(String::from("timed out waiting for GeometryStamp resize settle"));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let resized_shot = client.send(&json!({
        "id": "7",
        "method": "ui.screenshot",
        "params": { "target": "full_window" }
    }))?;
    assert_eq!(resized_shot.get("ok"), Some(&json!(true)), "resized screenshot failed: {resized_shot}");
    assert_eq!(resized_shot.pointer("/result/width_px"), Some(&json!(1680)));
    assert_eq!(resized_shot.pointer("/result/height_px"), Some(&json!(1050)));
    assert_eq!(resized_shot.pointer("/result/dpi"), Some(&json!(144)));
    let r_png = decode_base64(resized_shot.pointer("/result/data_base64").and_then(Value::as_str).ok_or("missing resized data_base64")?)?;
    assert_eq!(png_dimensions(&r_png), Some((1680, 1050)));

    let resized_inspect = client.send(&json!({
        "id": "7b",
        "method": "ui.inspect",
        "params": { "scope": "grid", "include_bounds": true }
    }))?;
    assert_eq!(resized_inspect.get("ok"), Some(&json!(true)), "resized inspect failed: {resized_inspect}");
    let post_resize_cell =
        find_grid_cell(&resized_inspect, "grid.cell.45.1.0").ok_or("expected grid.cell.45.1.0 to remain inspectable after 1.5x DPI resize")?;
    let resized_bounds = bounds_rect(post_resize_cell).ok_or("missing resized cell bounds")?;
    assert!(resized_bounds[2] > 0.0 && resized_bounds[3] > 0.0);
    assert!(resized_bounds[0] + resized_bounds[2] <= 1680.0 && resized_bounds[1] + resized_bounds[3] <= 1050.0);
    assert_ne!(resized_bounds, pre_resize_bounds, "post-resize GeometryStamp bounds must not stay stale pre-resize 1.0x bounds");
    Ok(())
}

fn run_live_control_column_scroll_inspect_click_screenshot() -> Result<(), Box<dyn std::error::Error>> {
    let initial = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: false,
        new_instance: false,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: initial,
        ..DesktopHostConfig::default()
    });
    let win_id = rt.open_window(None, initial)?;

    let auth = ControlAuth::generate()?;
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
    let resize_trigger = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let resize_settled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let resize_trigger_c = std::sync::Arc::clone(&resize_trigger);
    let resize_settled_c = std::sync::Arc::clone(&resize_settled);

    let client_handle = std::thread::spawn(move || -> Result<(), String> {
        let mut client =
            InteractiveControlClient { reader: BufReader::new(ChannelPipeReader::new(s2c_rx)), writer: ChannelPipeWriter { tx: c2s_tx } };

        let r_auth = client.send(&json!({ "id": "0", "method": "auth", "params": { "token": token } }))?;
        assert_eq!(r_auth.get("ok"), Some(&json!(true)));

        let r_create = client.send(&json!({
            "id": "1",
            "method": "sheet.createData",
            "params": { "sheet_id": "col-ctrl", "title": "Control Scroll Column", "kind": "column", "cols": 4, "rows": 80, "auto_graph": false }
        }))?;
        assert_eq!(r_create.get("ok"), Some(&json!(true)), "createData failed: {r_create}");

        for (r, c, val) in [(0usize, 0usize, 10.5_f64), (45, 1, 987.65)] {
            let set_resp = client.send(&json!({
                "id": format!("cell-{r}-{c}"),
                "method": "table.setCell",
                "params": { "sheet_id": "col-ctrl", "row": r, "col": c, "value": val }
            }))?;
            assert_eq!(set_resp.get("ok"), Some(&json!(true)), "setCell failed: {set_resp}");
        }

        // 1. Pre-scroll inspect: cell (0,0,0) visible, row 45 off-screen (no visible bounds).
        let pre_inspect = client.send(&json!({
            "id": "2",
            "method": "ui.inspect",
            "params": { "scope": "grid", "include_bounds": true, "metrics": { "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 } }
        }))?;
        assert_eq!(pre_inspect.get("ok"), Some(&json!(true)));
        let top_cell = find_grid_cell(&pre_inspect, "grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0 before scroll")?;
        let top_bounds = bounds_rect(top_cell).ok_or("missing bounds for grid.cell.0.0.0")?;
        assert!(top_bounds[2] > 0.0 && top_bounds[3] > 0.0);
        let hidden_before = find_grid_cell(&pre_inspect, "grid.cell.45.1.0").and_then(bounds_rect);
        assert!(hidden_before.is_none(), "row 45 should be off-screen before scrolling: {hidden_before:?}");

        let pre_shot = client.send(&json!({
            "id": "3",
            "method": "ui.screenshot",
            "params": { "target": "full_window", "width_px": 1280, "height_px": 800, "dpi": 96 }
        }))?;
        assert_eq!(pre_shot.get("ok"), Some(&json!(true)));
        let pre_png = decode_base64(pre_shot.pointer("/result/data_base64").and_then(Value::as_str).ok_or("missing pre data_base64")?)?;
        assert_eq!(png_dimensions(&pre_png), Some((1280, 800)));

        // 2. Scroll the Column grid downward until row 45 is revealed on-screen (`egui` uses delta_y < 0).
        let target_bounds = scroll_to_target_cell(&mut client, top_bounds)?;

        // 3. Click target cell via ui.pointer_gesture at its inspected bounds center.
        let click_x = target_bounds[0] + target_bounds[2] * 0.5;
        let click_y = target_bounds[1] + target_bounds[3] * 0.5;
        let click_resp = client.send(&json!({
            "id": "4",
            "method": "ui.pointer_gesture",
            "params": {
                "action": "click",
                "x": click_x,
                "y": click_y,
                "metrics": { "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 }
            }
        }))?;
        assert_eq!(click_resp.get("ok"), Some(&json!(true)), "click failed: {click_resp}");

        let post_inspect = client.send(&json!({
            "id": "5",
            "method": "ui.inspect",
            "params": { "scope": "grid", "include_bounds": true, "metrics": { "logical_width": 1280.0, "logical_height": 800.0, "scale_factor": 1.0 } }
        }))?;
        assert_eq!(post_inspect.get("ok"), Some(&json!(true)));
        let active = post_inspect.pointer("/result/grid_selection/active_cell").ok_or("missing active_cell")?;
        assert_eq!(active.get("row"), Some(&json!(45)), "expected active row 45 after clicking cell: {post_inspect}");
        assert_eq!(active.get("col"), Some(&json!(1)), "expected active col 1 after clicking cell: {post_inspect}");

        // 4. Verify updated ui.screenshot reflects scrolled + selected pixels.
        let post_shot = client.send(&json!({
            "id": "6",
            "method": "ui.screenshot",
            "params": { "target": "full_window", "width_px": 1280, "height_px": 800, "dpi": 96 }
        }))?;
        assert_eq!(post_shot.get("ok"), Some(&json!(true)));
        let post_png = decode_base64(post_shot.pointer("/result/data_base64").and_then(Value::as_str).ok_or("missing post data_base64")?)?;
        assert_eq!(png_dimensions(&post_png), Some((1280, 800)));
        assert_ne!(pre_png, post_png, "post-scroll + post-click screenshot must differ from pre-scroll screenshot");

        // 5. Trigger resize + 1.5x DPI transition and verify GeometryStamp settles before screenshot/inspect.
        verify_resize_geometry_stamp(&mut client, &resize_trigger_c, &resize_settled_c, target_bounds)?;

        let quit = client.send(&json!({ "id": "8", "method": "app.quit" }))?;
        assert_eq!(quit.pointer("/result/quit"), Some(&json!(true)));
        Ok(())
    });

    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    let mut native = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

    let deadline = Instant::now() + Duration::from_secs(15);
    while !client_handle.is_finished() && Instant::now() < deadline {
        let resized = resize_trigger.load(std::sync::atomic::Ordering::Acquire);
        let mut raw_input = if resized {
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1120.0, 700.0))),
                focused: true,
                viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(1.5), ..Default::default() })]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }
        } else {
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                focused: true,
                viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(1.0), ..Default::default() })]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }
        };
        native.drain_synthetic_events_into(&mut raw_input);
        let mut out = ctx.run_ui(raw_input, |ui| native.step_ui(ui));
        out.textures_delta.clear();
        if resized && native.geometry_stamp().is_some_and(|s| s.settled && s.physical_width_px == 1680 && s.physical_height_px == 1050) {
            resize_settled.store(true, std::sync::atomic::Ordering::Release);
        }
        std::thread::sleep(Duration::from_millis(4));
    }

    bridge.signal_shutdown();
    client_handle.join().map_err(|_| "control client thread panicked")??;
    srv_handle.join().map_err(|_| "control server thread panicked")??;
    native.finish_exit();
    Ok(())
}

#[test]
fn egui_live_control_column_scroll_inspect_click_screenshot() -> Result<(), Box<dyn std::error::Error>> {
    run_live_control_column_scroll_inspect_click_screenshot()
}
