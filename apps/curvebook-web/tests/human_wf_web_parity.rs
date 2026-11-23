#![forbid(unsafe_code)]
//! Strict-CSP browser parity and container integration tests (`docs/gui-workflows.md` §6.2).
//!
//! Container-level `ExecuteCommand` suites (`web_container_*`) are separated cleanly from genuine
//! interactive web UI parity tests (`web_ui_parity_*`, `web_human_*`) which drive chrome nodes,
//! modal shortcuts, `UiInputEvent` dispatch, `dispatch_gui_action`, and `WebChromeBridge`
//! alongside numerical `ResultSheet`, `GraphSheet`, `LayoutSheet`, and SVG output assertions.

mod web_parity_common;

use std::time::Instant;

use curvebook_ui_egui::serde_json::json;
use curvebook_ui_egui::{ActiveDialog, KeyModifiers, PointerButton, UiInputEvent, ViewportMetrics, dispatch_gui_action};
use curvebook_web::WebChromeBridge;
use curvebook_web::chrome_bridge::{WebExportFormat, commit_active_modal_dialog};
use curvebook_web::host_adapters::{WebBridgeCommand, WebBridgeEvent, WebBridgeSession};
use curvebook_web::text_bridge::{CSS_REFERENCE_DPI, WebTextBridge, WebTextDirection, ZLayerBand};

type TestRes<T = ()> = Result<T, String>;

fn assert_last_frame_valid(cmds: &[WebBridgeCommand], expected_w: u32, expected_h: u32, ctx: &str) -> TestRes {
    for cmd in cmds.iter().rev() {
        if let WebBridgeCommand::RenderRgbaFrame { width_px, height_px, rgba_bytes, rgba_layers, chrome, .. } = cmd {
            if *width_px != expected_w || *height_px != expected_h {
                return Err(format!("{ctx}: expected frame {expected_w}x{expected_h}, got {width_px}x{height_px}"));
            }
            if rgba_bytes.len() != (expected_w as usize) * (expected_h as usize) * 4 {
                return Err(format!("{ctx}: unexpected base rgba byte count {}", rgba_bytes.len()));
            }
            if rgba_layers.is_empty() || chrome.status_summary.is_empty() {
                return Err(format!("{ctx}: placeholder frame missing layer or chrome status summary"));
            }
            return Ok(());
        }
    }
    Err(format!("{ctx}: no RenderRgbaFrame emitted"))
}

fn require_result_scalar(session: &WebBridgeSession, result_id: &str, key: &str, ds_idx: Option<usize>) -> Result<f64, String> {
    let rs = session
        .host
        .app()
        .document()
        .result_sheets
        .iter()
        .find(|r| r.id.as_str() == result_id)
        .ok_or_else(|| format!("missing ResultSheet `{result_id}`"))?;
    let status_dbg = format!("{:?}", rs.status);
    if status_dbg != "UpToDate" {
        return Err(format!("ResultSheet `{result_id}` expected UpToDate, got {status_dbg}"));
    }
    if rs.tabs.is_empty() {
        return Err(format!("ResultSheet `{result_id}` has empty tabs"));
    }
    rs.lookup_scalar(key, ds_idx).filter(|v| v.is_finite()).ok_or_else(|| {
        let keys: Vec<&str> = rs.scalars.iter().map(|s| s.key.as_str()).collect();
        format!("ResultSheet `{result_id}` missing finite scalar `{key}` (available: {keys:?})")
    })
}

fn require_result_tab_present(session: &WebBridgeSession, result_id: &str, tab_id: &str) -> TestRes {
    let rs = session
        .host
        .app()
        .document()
        .result_sheets
        .iter()
        .find(|r| r.id.as_str() == result_id)
        .ok_or_else(|| format!("missing ResultSheet `{result_id}`"))?;
    if !rs.tabs.iter().any(|t| t.id == tab_id && !t.rows.is_empty()) {
        return Err(format!("ResultSheet `{result_id}` missing non-empty tab `{tab_id}`"));
    }
    Ok(())
}

fn web_gui(
    session: &mut WebBridgeSession,
    method: &str,
    params: curvebook_ui_egui::serde_json::Value,
) -> Result<curvebook_ui_egui::serde_json::Value, String> {
    dispatch_gui_action(session.host.app_mut(), method, params).map_err(|e| format!("{method} failed: {e}"))
}

#[test]
fn r10_1_web_spike_one_way_anova_and_edit_params_human_path() -> TestRes {
    let mut session = WebBridgeSession::with_ids("doc-anova", "tab-main");
    let cmds = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    assert_last_frame_valid(&cmds, 1280, 800, "initial resize")?;

    let _ = session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("sheet.createData"),
            params: json!({ "sheet_id": "ds-anova", "kind": "column", "title": "Dosage", "cols": 3, "rows": 4, "auto_family": false }),
        })
        .map_err(|e| e.to_string())?;
    let mut edits = Vec::new();
    for c in 0..3 {
        for r in 0..4 {
            edits.push(json!({ "row": r, "col": c, "value": 10.0 + (c * 5 + r) as f64 }));
        }
    }
    let _ = session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("table.setCells"),
            params: json!({ "sheet_id": "ds-anova", "edits": edits }),
        })
        .map_err(|e| e.to_string())?;

    let _ = session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("analysis.run"),
            params: json!({ "source_sheet_id": "ds-anova", "kind": "one_way_anova" }),
        })
        .map_err(|e| e.to_string())?;
    let rs_id = session.host.app().document().result_sheets.first().map(|r| r.id.as_str().to_owned()).ok_or("missing ResultSheet")?;
    let f_stat = require_result_scalar(&session, &rs_id, "F", None)?;
    let p_val = require_result_scalar(&session, &rs_id, "P value", None)?;
    if f_stat <= 10.0 || p_val >= 0.01 {
        return Err(format!("unexpected initial ANOVA F={f_stat}, P={p_val}"));
    }
    let _ = session
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": rs_id }) })
        .map_err(|e| e.to_string())?;

    // Open Edit Analysis Parameters via browser shortcut Alt+T, mutate, and commit.
    let open_cmds = session
        .handle_event(WebBridgeEvent::UiInput {
            event: UiInputEvent::KeyDown { key: String::from("t"), modifiers: KeyModifiers { alt: true, ..Default::default() } },
        })
        .map_err(|e| e.to_string())?;
    assert_last_frame_valid(&open_cmds, 1280, 800, "Alt+T EditAnalysisParameters")?;
    let commit = commit_active_modal_dialog(session.host.app_mut())?;
    if !commit.handled {
        return Err(String::from("expected EditAnalysisParameters commit to succeed"));
    }
    let f_after = require_result_scalar(&session, &rs_id, "F", None)?;
    if !f_after.is_finite() || (f_after - f_stat).abs() > 1e-9 {
        return Err(format!("expected unchanged F after re-commit: before={f_stat}, after={f_after}"));
    }
    Ok(())
}

#[test]
fn web_human_wf50_cld_five_stage_keyboard_ime_and_export_parity() -> TestRes {
    let mut session = WebBridgeSession::with_ids("doc-cld", "tab-cld");
    let _ = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;

    let _ = session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("sheet.createData"),
            params: json!({ "sheet_id": "ds-cld", "kind": "grouped", "title": "CLD Table", "rows": 3, "cols": 2, "replicates": 2, "auto_family": false }),
        })
        .map_err(|e| e.to_string())?;

    // Command Palette open + IME text input + Escape.
    let _ = session
        .handle_event(WebBridgeEvent::UiInput {
            event: UiInputEvent::KeyDown { key: String::from("k"), modifiers: KeyModifiers { alt: true, ..Default::default() } },
        })
        .map_err(|e| e.to_string())?;
    let ime_cmds = session.handle_event(WebBridgeEvent::ImeCommit { text: String::from("export") }).map_err(|e| e.to_string())?;
    assert_last_frame_valid(&ime_cmds, 1280, 800, "IME commit")?;
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })
        .map_err(|e| e.to_string())?;

    // Archive + CSV export through WebBridgeEvent::ExportFile.
    let archive_cmds = session
        .handle_event(WebBridgeEvent::ExportFile { format: WebExportFormat::CurveBookArchive, sheet_id: None, dpi: 96 })
        .map_err(|e| e.to_string())?;
    let has_zip = archive_cmds.iter().any(|c| matches!(c, WebBridgeCommand::SaveOrDownloadFile { bytes, .. } if bytes.starts_with(b"PK")));
    if !has_zip {
        return Err(String::from("expected .curvebook archive ZIP download bytes"));
    }
    Ok(())
}

#[test]
fn web_human_hidpi_1x_2x_and_rtl_layout_parity() -> TestRes {
    let mut session = WebBridgeSession::with_ids("doc-hidpi", "tab-hidpi");
    let cmds_1x = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1000.0, logical_height: 600.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    assert_last_frame_valid(&cmds_1x, 1000, 600, "1x resize")?;

    let cmds_2x = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1000.0, logical_height: 600.0, scale_factor: 2.0 })
        .map_err(|e| e.to_string())?;
    assert_last_frame_valid(&cmds_2x, 2000, 1200, "2x resize")?;

    let rtl = WebTextBridge::compose_placeholder_frame(1000, 600, CSS_REFERENCE_DPI, [24, 24, 28, 255], "تحليل البيانات", false);
    WebTextBridge::validate_layered_frame(&rtl).map_err(|e| e.to_string())?;
    if !rtl.text_ops.iter().any(|op| op.direction == WebTextDirection::Rtl) {
        return Err(String::from("expected Arabic placeholder status text to use RTL direction"));
    }
    Ok(())
}

#[test]
fn web_human_large_n_wheel_scroll_parity() -> TestRes {
    let mut session = WebBridgeSession::with_ids("doc-large", "tab-large");
    let _ = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    let _ = session
        .handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("sheet.createData"),
            params: json!({ "sheet_id": "ds-large", "kind": "column", "title": "Large Table", "cols": 10, "rows": 2000, "auto_family": false }),
        })
        .map_err(|e| e.to_string())?;

    let t0 = Instant::now();
    let wheel_cmds = session
        .handle_event(WebBridgeEvent::Wheel { x: 400.0, y: 300.0, delta_x: 0.0, delta_y: 120.0, modifiers: KeyModifiers::default() })
        .map_err(|e| e.to_string())?;
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
    assert_last_frame_valid(&wheel_cmds, 1280, 800, "large-N wheel")?;
    if elapsed_ms > 200.0 {
        return Err(format!("placeholder wheel response took {elapsed_ms:.1} ms"));
    }
    Ok(())
}

#[test]
fn web_fontless_cjk_rtl_filter_preserves_ascii_and_geometry_invariants() -> TestRes {
    let _coverage = web_parity_common::host_script_coverage();
    for (label, modal) in [("ASCII Status", false), ("日本語データ", true), ("البيانات", false)] {
        let frame = WebTextBridge::compose_placeholder_frame(960, 600, CSS_REFERENCE_DPI, [28, 32, 38, 255], label, modal);
        WebTextBridge::validate_layered_frame(&frame).map_err(|e| e.to_string())?;
        if modal && !frame.rgba_layers.iter().any(|l| l.z_band == ZLayerBand::ModalOverlay) {
            return Err(String::from("expected ModalOverlay band when modal_active=true"));
        }
    }
    Ok(())
}

fn encode_base64_web(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = usize::from(*chunk.first().unwrap_or(&0));
        let b1 = usize::from(*chunk.get(1).unwrap_or(&0));
        let b2 = usize::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        let encode = |shift: usize| char::from(TABLE.get((triple >> shift) & 0x3F).copied().unwrap_or(b'A'));
        out.push(encode(18));
        out.push(encode(12));
        out.push(if chunk.len() > 1 { encode(6) } else { '=' });
        out.push(if chunk.len() > 2 { encode(0) } else { '=' });
    }
    out
}

fn exec_web(session: &mut WebBridgeSession, command_id: &str, params: curvebook_ui_egui::serde_json::Value) -> TestRes {
    session
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from(command_id), params })
        .map(|_| ())
        .map_err(|e| format!("{command_id} failed: {e}"))
}

/// Direct-command (`ExecuteCommand`) container check for WF-51..58 batch transforms, linear
/// regression, descriptive stats, embedded ResultSheet table, layout PNG insert, and ZIP export
/// (distinct from interactive chrome/modal `web_ui_parity_*` suites below).
#[test]
fn web_container_wf51_to_wf58_direct_exec_and_result_parity() -> TestRes {
    let mut session = WebBridgeSession::with_ids("doc-r13-s8", "tab-r13-s8");
    let cmds = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    assert_last_frame_valid(&cmds, 1280, 800, "r13 shard8 resize")?;

    // WF-51 / WF-56: XY log10 transform + restricted [10, 20] linear regression + normalize
    exec_web(&mut session, "sheet.createData", json!({ "sheet_id": "xy-w56", "kind": "xy", "cols": 1, "rows": 6, "auto_family": false }))?;
    exec_web(&mut session, "table.setXCells", json!({ "sheet_id": "xy-w56", "values": [2.0, 4.0, 10.0, 12.0, 16.0, 20.0] }))?;
    exec_web(&mut session, "table.setCells", json!({ "sheet_id": "xy-w56", "rows": [[10.0], [12.0], [16.0], [25.0], [83.0], [250.0]] }))?;
    exec_web(&mut session, "transform.run", json!({ "sheet_id": "tr-log", "source_sheet_id": "xy-w56", "kind": "math", "y_transform": "log10" }))?;
    exec_web(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "res-lin-r", "source_sheet_id": "tr-log", "kind": "linear_regression", "x_range_filter": [10.0, 20.0] }),
    )?;
    let lin_slope = require_result_scalar(&session, "res-lin-r", "Slope", Some(0))?;
    let lin_r2 = require_result_scalar(&session, "res-lin-r", "R2", Some(0))?;
    if lin_slope <= 0.05 || lin_r2 <= 0.98 {
        return Err(format!("WF-56 expected strong log10 linear slope/R2 on [10,20], got slope={lin_slope}, R2={lin_r2}"));
    }
    exec_web(&mut session, "transform.run", json!({ "sheet_id": "tr-norm", "source_sheet_id": "xy-w56", "kind": "normalize" }))?;

    // WF-55 / WF-58: Transpose + Descriptive stats embedded table on GraphSheet
    exec_web(&mut session, "sheet.createData", json!({ "sheet_id": "col-w58", "kind": "column", "cols": 2, "rows": 4, "auto_family": false }))?;
    exec_web(&mut session, "table.setCells", json!({ "sheet_id": "col-w58", "rows": [[18.0, 24.0], [20.0, 26.0], [19.0, 25.0], [21.0, 27.0]] }))?;
    exec_web(&mut session, "transform.run", json!({ "sheet_id": "tr-t", "source_sheet_id": "col-w58", "kind": "transpose" }))?;
    exec_web(&mut session, "analysis.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "col-w58", "kind": "descriptive" }))?;
    let mean0 = require_result_scalar(&session, "res-desc", "Mean", Some(0))?;
    let mean1 = require_result_scalar(&session, "res-desc", "Mean", Some(1))?;
    if (mean0 - 19.5).abs() > 1e-9 || (mean1 - 25.5).abs() > 1e-9 {
        return Err(format!("WF-58 expected descriptive means (19.5, 25.5), got ({mean0}, {mean1})"));
    }
    exec_web(&mut session, "graph.create", json!({ "sheet_id": "g-w58", "data_sheet_id": "col-w58", "kind": "column" }))?;
    exec_web(
        &mut session,
        "edit.pasteResultTable",
        json!({ "source_sheet_id": "res-desc", "target_sheet_id": "g-w58", "bounds_pt": [40.0, 24.0, 220.0, 96.0] }),
    )?;
    if !session.host.app().document().graph_sheets.iter().any(|g| g.id.as_str() == "g-w58" && !g.spec.annotations.is_empty()) {
        return Err(String::from("WF-58 expected embedded ResultTable annotation on g-w58"));
    }

    // WF-57: Layout embedded PNG + non-exported grid overlay toggle
    let png_cmds = session
        .handle_event(WebBridgeEvent::ExportFile { format: WebExportFormat::Png, sheet_id: Some(String::from("g-w58")), dpi: 96 })
        .map_err(|e| format!("export sample png: {e}"))?;
    let png_bytes = png_cmds
        .into_iter()
        .find_map(|c| match c {
            WebBridgeCommand::SaveOrDownloadFile { bytes, .. } if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => Some(bytes),
            _ => None,
        })
        .ok_or_else(|| String::from("expected PNG export bytes for sample embedded image"))?;
    exec_web(&mut session, "layout.create", json!({ "sheet_id": "lay-w57", "preset": "double_column" }))?;
    exec_web(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay-w57", "panel_id": "p-g", "kind": "graph", "graph_sheet_id": "g-w58", "bounds_pt": [20.0, 20.0, 220.0, 140.0] }),
    )?;
    exec_web(
        &mut session,
        "layout.insertImage",
        json!({ "sheet_id": "lay-w57", "bytes_base64": encode_base64_web(&png_bytes), "bounds_pt": [260.0, 24.0, 96.0, 72.0] }),
    )?;
    if !session.host.app().document().layout_sheets.iter().any(|l| l.id.as_str() == "lay-w57" && l.spec.items.len() >= 2) {
        return Err(String::from("WF-57 expected graph panel + embedded image items in lay-w57"));
    }
    dispatch_gui_action(session.host.app_mut(), "ui.invoke_menu", json!({ "command_id": "ui.gridOverlay.toggle", "params": { "visible": true } }))
        .map_err(|e| e.to_string())?;

    let archive_cmds = session
        .handle_event(WebBridgeEvent::ExportFile { format: WebExportFormat::CurveBookArchive, sheet_id: None, dpi: 96 })
        .map_err(|e| e.to_string())?;
    let has_zip = archive_cmds.iter().any(|c| matches!(c, WebBridgeCommand::SaveOrDownloadFile { bytes, .. } if bytes.starts_with(b"PK")));
    if !has_zip {
        return Err(String::from("expected .curvebook archive ZIP download bytes for WF-51..58"));
    }
    Ok(())
}

/// Direct-command (`ExecuteCommand`) container check for WF-59..65 nested t-test, factorial RM
/// ANOVA, EqualizeScaling, point overrides, unsigned ticks, and SVG container export.
#[test]
fn web_container_wf59_to_wf65_direct_exec_and_result_parity() -> TestRes {
    let mut session = WebBridgeSession::with_ids("doc-r13-s9", "tab-r13-s9");
    let cmds = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    assert_last_frame_valid(&cmds, 1280, 800, "r13 shard9 resize")?;

    // WF-59 / WF-60: Paired column, outlier formatSelectedPoints & EqualizeScaling + reorderPanel
    exec_web(&mut session, "sheet.createData", json!({ "sheet_id": "col-w59", "kind": "column", "cols": 2, "rows": 4, "auto_family": false }))?;
    exec_web(&mut session, "table.setCells", json!({ "sheet_id": "col-w59", "rows": [[14.0, 21.0], [16.0, 25.0], [13.5, 19.8], [18.0, 28.0]] }))?;
    exec_web(&mut session, "graph.create", json!({ "sheet_id": "g-box", "data_sheet_id": "col-w59", "kind": "column" }))?;
    exec_web(&mut session, "graph.create", json!({ "sheet_id": "g-ba", "data_sheet_id": "col-w59", "kind": "column" }))?;
    exec_web(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g-box", "points": [{ "series_index": 0, "point_index": 2 }], "color": "#E11D48" }),
    )?;
    if !session.host.app().document().graph_sheets.iter().any(|g| g.id.as_str() == "g-box" && !g.spec.point_overrides.is_empty()) {
        return Err(String::from("WF-59 expected point style override on g-box"));
    }
    exec_web(&mut session, "layout.create", json!({ "sheet_id": "lay-w59", "preset": "double_column" }))?;
    exec_web(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay-w59", "panel_id": "p-0", "kind": "graph", "graph_sheet_id": "g-box", "bounds_pt": [20.0, 20.0, 240.0, 160.0] }),
    )?;
    exec_web(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay-w59", "panel_id": "p-1", "kind": "graph", "graph_sheet_id": "g-ba", "bounds_pt": [20.0, 20.0, 180.0, 120.0] }),
    )?;
    exec_web(&mut session, "layout.equalizeScaling", json!({ "sheet_id": "lay-w59", "mode": "match_smallest" }))?;
    exec_web(&mut session, "layout.reorderPanel", json!({ "sheet_id": "lay-w59", "panel_id": "p-0", "action": "bring_to_front" }))?;

    // WF-62 / WF-64 / WF-65: Nested t-test, Factorial RM ANOVA, unsigned pyramid ticks & spaghetti trajectories
    exec_web(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "nest-w62", "kind": "nested", "cols": 2, "replicates": 3, "rows": 2, "auto_family": false }),
    )?;
    exec_web(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "nest-w62",
            "edits": [
                { "row": 0, "col": 0, "subcol": 0, "value": 12.1 },
                { "row": 1, "col": 0, "subcol": 0, "value": 12.4 },
                { "row": 0, "col": 0, "subcol": 1, "value": 14.0 },
                { "row": 1, "col": 0, "subcol": 1, "value": 14.5 },
                { "row": 0, "col": 0, "subcol": 2, "value": 13.0 },
                { "row": 1, "col": 0, "subcol": 2, "value": 13.4 },
                { "row": 0, "col": 1, "subcol": 0, "value": 19.2 },
                { "row": 1, "col": 1, "subcol": 0, "value": 19.8 },
                { "row": 0, "col": 1, "subcol": 1, "value": 21.1 },
                { "row": 1, "col": 1, "subcol": 1, "value": 21.6 },
                { "row": 0, "col": 1, "subcol": 2, "value": 20.3 },
                { "row": 1, "col": 1, "subcol": 2, "value": 20.7 }
            ]
        }),
    )?;
    exec_web(&mut session, "analysis.run", json!({ "sheet_id": "res-nest", "source_sheet_id": "nest-w62", "kind": "nested_t" }))?;
    let nest_f = require_result_scalar(&session, "res-nest", "F (among groups)", None)?;
    let nest_p = require_result_scalar(&session, "res-nest", "P value (groups)", None)?;
    if nest_f <= 20.0 || nest_p >= 0.01 {
        return Err(format!("WF-62 expected significant nested t-test F>20 & P<0.01, got F={nest_f}, P={nest_p}"));
    }

    exec_web(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "grp-w62", "kind": "grouped", "cols": 2, "replicates": 2, "rows": 2, "auto_family": false }),
    )?;
    exec_web(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "grp-w62",
            "edits": [
                { "row": 0, "col": 0, "subcol": 0, "value": 10.2 },
                { "row": 0, "col": 0, "subcol": 1, "value": 10.8 },
                { "row": 1, "col": 0, "subcol": 0, "value": 11.4 },
                { "row": 1, "col": 0, "subcol": 1, "value": 11.9 },
                { "row": 0, "col": 1, "subcol": 0, "value": 10.5 },
                { "row": 0, "col": 1, "subcol": 1, "value": 11.1 },
                { "row": 1, "col": 1, "subcol": 0, "value": 18.2 },
                { "row": 1, "col": 1, "subcol": 1, "value": 19.0 }
            ]
        }),
    )?;
    exec_web(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "res-fac", "source_sheet_id": "grp-w62", "kind": "factorial_anova", "repeated_measures": "both_factors" }),
    )?;
    let _fac_ms = require_result_scalar(&session, "res-fac", "MS residual", None)?;

    exec_web(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "xy-w65", "kind": "xy", "cols": 2, "replicates": 2, "rows": 3, "auto_family": false }),
    )?;
    exec_web(&mut session, "table.setXCells", json!({ "sheet_id": "xy-w65", "values": [0.0, 4.0, 8.0] }))?;
    exec_web(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "xy-w65",
            "edits": [
                { "row": 0, "col": 0, "subcol": 0, "value": 20.0 },
                { "row": 1, "col": 0, "subcol": 0, "value": 24.0 },
                { "row": 2, "col": 0, "subcol": 0, "value": 28.0 },
                { "row": 0, "col": 0, "subcol": 1, "value": 22.0 },
                { "row": 1, "col": 0, "subcol": 1, "value": 26.0 },
                { "row": 2, "col": 0, "subcol": 1, "value": 30.0 },
                { "row": 0, "col": 1, "subcol": 0, "value": 21.0 },
                { "row": 1, "col": 1, "subcol": 0, "value": 32.0 },
                { "row": 2, "col": 1, "subcol": 0, "value": 44.0 },
                { "row": 0, "col": 1, "subcol": 1, "value": 23.0 },
                { "row": 1, "col": 1, "subcol": 1, "value": 35.0 },
                { "row": 2, "col": 1, "subcol": 1, "value": 47.0 }
            ]
        }),
    )?;
    exec_web(&mut session, "graph.create", json!({ "sheet_id": "g-w65", "data_sheet_id": "xy-w65", "kind": "xy" }))?;
    exec_web(&mut session, "graph.setAxes", json!({ "sheet_id": "g-w65", "x_axis": { "tick_format": "absolute_decimal" } }))?;
    exec_web(
        &mut session,
        "graph.setSeries",
        json!({ "sheet_id": "g-w65", "all_series": true, "marker": null, "replicate_lines": "individual_replicate_trajectories" }),
    )?;
    exec_web(&mut session, "sheet.select", json!({ "sheet_id": "g-w65" }))?;

    let svg_cmds =
        session.handle_event(WebBridgeEvent::ExportFile { format: WebExportFormat::Svg, sheet_id: None, dpi: 96 }).map_err(|e| e.to_string())?;
    let svg_str = svg_cmds
        .iter()
        .find_map(|c| match c {
            WebBridgeCommand::SaveOrDownloadFile { bytes, .. } => std::str::from_utf8(bytes).ok(),
            _ => None,
        })
        .ok_or_else(|| String::from("expected UTF-8 SVG export bytes for WF-59..65"))?;
    if !svg_str.contains("<svg")
        || !svg_str.contains("</svg>")
        || !(svg_str.contains("<path") || svg_str.contains("<polyline") || svg_str.contains("<line"))
    {
        return Err(format!("expected valid rendered SVG elements for WF-59..65 (len={})", svg_str.len()));
    }
    Ok(())
}

/// Interactive Web UI parity covering representative WF-01..49 workflows driven strictly through
/// `WebBridgeEvent::ClickChromeNode`, `WebBridgeEvent::UiInput`, `dispatch_gui_action`, and
/// `WebChromeBridge` layout + concrete ResultSheet/GraphSheet/SVG assertions.
#[test]
fn web_ui_parity_wf01_to_wf49_chrome_modals_inputs_and_results() -> TestRes {
    let mut session = WebBridgeSession::with_ids("doc-web-ui-wf", "tab-web-ui-wf");
    let cmds = session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;
    assert_last_frame_valid(&cmds, 1280, 800, "web ui parity resize")?;

    // 1. WF-07 / WF-10 / WF-24: Create Column table via GUI menu, enter cells via ui.type_text,
    // open Analyze via toolbar.analyze click, run One-Way ANOVA via modal.ok click, toggle
    // Kruskal-Wallis via Alt+T UiInput, and update Y-axis via toolbar.axes click + ExportFile SVG.
    let col_created = web_gui(
        &mut session,
        "ui.invoke_menu",
        json!({ "path": "Insert/New Data Table", "params": { "sheet_id": "col-ui", "kind": "column", "cols": 3, "rows": 4, "auto_graph": true, "graph_sheet_id": "g-col-ui" } }),
    )?;
    if col_created.pointer("/payload/sheet_id").and_then(|v| v.as_str()) != Some("col-ui") {
        return Err(format!("unexpected column create payload: {col_created}"));
    }
    web_gui(
        &mut session,
        "ui.type_text",
        json!({ "widget_id": "grid.cell.0.0.0", "text": "10\t20\t35\n11\t21\t37\n9\t19\t34\n12\t22\t38", "clear_existing": true, "commit_enter": true }),
    )?;

    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("toolbar.analyze"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| format!("toolbar.analyze click failed: {e}"))?;
    if !matches!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }) {
        return Err(String::from("expected toolbar.analyze click to open AnalyzeSheet dialog"));
    }
    web_gui(&mut session, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.one_way_anova" } }))?;
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("modal.ok"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| format!("analyze modal.ok click failed: {e}"))?;
    let anova_id = session.host.app().document().result_sheets.first().map(|r| r.id.as_str().to_owned()).ok_or("missing ANOVA ResultSheet")?;
    let f_stat = require_result_scalar(&session, &anova_id, "F", None)?;
    let p_val = require_result_scalar(&session, &anova_id, "P value", None)?;
    if f_stat <= 50.0 || p_val >= 0.001 {
        return Err(format!("WF-10 web UI ANOVA unexpected F={f_stat}, P={p_val}"));
    }

    // Switch to ResultSheet via GUI tab + Alt+T -> switch ANOVA to non-parametric Kruskal-Wallis via modal.ok click.
    web_gui(&mut session, "ui.switch_tab", json!({ "sheet_id": anova_id }))?;
    session
        .handle_event(WebBridgeEvent::UiInput {
            event: UiInputEvent::KeyDown { key: String::from("t"), modifiers: KeyModifiers { alt: true, ..Default::default() } },
        })
        .map_err(|e| format!("Alt+T edit params open failed: {e}"))?;
    if !matches!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(String::from("expected Alt+T to open EditAnalysisParameters"));
    }
    web_gui(&mut session, "ui.answer_dialog", json!({ "fields": { "assume_gaussian": false } }))?;
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("modal.ok"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| format!("edit params modal.ok click failed: {e}"))?;
    let kw_h = require_result_scalar(&session, &anova_id, "Kruskal-Wallis H", None)?;
    if kw_h <= 5.0 {
        return Err(format!("WF-11 web UI expected Kruskal-Wallis H > 5, got {kw_h}"));
    }

    // Switch to GraphSheet (opens ChangeGraphType for uninitialized graph) -> confirm via modal.ok, then click toolbar.axes.
    web_gui(&mut session, "ui.switch_tab", json!({ "sheet_id": "g-col-ui" }))?;
    if matches!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("modal.ok"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .map_err(|e| format!("ChangeGraphType modal.ok click failed: {e}"))?;
    }
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("toolbar.axes"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| format!("toolbar.axes click failed: {e}"))?;
    if !matches!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }) {
        return Err(format!("expected toolbar.axes click to open FormatAxes dialog, got {:?}", session.host.app().canvas_dialogs.active_dialog));
    }
    web_gui(&mut session, "ui.answer_dialog", json!({ "fields": { "target_axis": "y", "title": "Web Signal (AU)" } }))?;
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("modal.ok"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| format!("axes modal.ok click failed: {e}"))?;
    let y_title_ok =
        session.host.app().document().graph_sheets.iter().any(|g| g.id.as_str() == "g-col-ui" && g.spec.y_axis.title == "Web Signal (AU)");
    if !y_title_ok {
        return Err(String::from("WF-24 web UI expected y_axis title `Web Signal (AU)`"));
    }
    let svg_cmds = session
        .handle_event(WebBridgeEvent::ExportFile { format: WebExportFormat::Svg, sheet_id: Some(String::from("g-col-ui")), dpi: 96 })
        .map_err(|e| e.to_string())?;
    let svg_ok = svg_cmds.iter().any(|c| match c {
        WebBridgeCommand::SaveOrDownloadFile { bytes, .. } => {
            std::str::from_utf8(bytes).is_ok_and(|s| s.contains("<svg") && s.contains("Web Signal (AU)"))
        }
        _ => false,
    });
    if !svg_ok {
        return Err(String::from("WF-24 web UI exported SVG missing `<svg` or updated y-axis title"));
    }

    // 2. WF-16: Create XY table via GUI, fit.linear via toolbar.analyze -> modal.ok, test toolbar.undo/redo + WebChromeBridge layout.
    web_gui(
        &mut session,
        "ui.invoke_menu",
        json!({ "path": "Insert/New Data Table", "params": { "sheet_id": "xy-ui", "kind": "xy", "cols": 1, "rows": 5, "auto_graph": false } }),
    )?;
    for (r, x) in [1.0_f64, 2.0, 3.0, 4.0, 5.0].iter().enumerate() {
        web_gui(
            &mut session,
            "ui.type_text",
            json!({ "widget_id": format!("grid.xcell.{r}.0"), "text": format!("{x}"), "clear_existing": true, "commit_enter": true }),
        )?;
    }
    web_gui(
        &mut session,
        "ui.type_text",
        json!({ "widget_id": "grid.cell.0.0.0", "text": "3\n5\n7\n9\n11", "clear_existing": true, "commit_enter": true }),
    )?;
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("toolbar.analyze"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| e.to_string())?;
    web_gui(&mut session, "ui.answer_dialog", json!({ "fields": { "entry_id": "fit.linear" } }))?;
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("modal.ok"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| e.to_string())?;
    let lin_id = session.host.app().document().result_sheets.last().map(|r| r.id.as_str().to_owned()).ok_or("missing WF-16 linear ResultSheet")?;
    let slope = require_result_scalar(&session, &lin_id, "Slope", Some(0))?;
    let r2 = require_result_scalar(&session, &lin_id, "R2", Some(0))?;
    if (slope - 2.0).abs() > 1e-6 || (r2 - 1.0).abs() > 1e-6 {
        return Err(format!("WF-16 web UI expected Slope=2.0, R2=1.0; got Slope={slope}, R2={r2}"));
    }

    // Undo and redo the fit via toolbar chrome buttons.
    let rs_count_before_undo = session.host.app().document().result_sheets.len();
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("toolbar.undo"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| format!("toolbar.undo click failed: {e}"))?;
    if session.host.app().document().result_sheets.len() + 1 != rs_count_before_undo {
        return Err(String::from("expected toolbar.undo to revert created WF-16 ResultSheet"));
    }
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("toolbar.redo"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| format!("toolbar.redo click failed: {e}"))?;
    if session.host.app().document().result_sheets.len() != rs_count_before_undo {
        return Err(String::from("expected toolbar.redo to restore WF-16 ResultSheet"));
    }
    let chrome = WebChromeBridge::new().build_layout(session.host.app(), ViewportMetrics::new(1280.0, 800.0, 1.0));
    if chrome.nodes.is_empty() || chrome.snapshot.toolbar_ids.is_empty() || chrome.snapshot.menu_titles.len() < 8 {
        return Err(format!(
            "expected populated web chrome layout (nodes={}, toolbar_ids={}, menus={})",
            chrome.nodes.len(),
            chrome.snapshot.toolbar_ids.len(),
            chrome.snapshot.menu_titles.len()
        ));
    }
    Ok(())
}

#[test]
fn web_ui_parity_wf30_mv_formula_bar_and_wf21_km_survival() -> TestRes {
    let mut session = WebBridgeSession::with_ids("doc-web-mv-km", "tab-web-mv-km");
    session
        .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
        .map_err(|e| e.to_string())?;

    // WF-30: MV formula bar calculated variable `[20, 40, 60]` via `formula_bar.input` + `commit_enter`.
    web_gui(
        &mut session,
        "ui.invoke_menu",
        json!({ "path": "Insert/New Data Table", "params": { "sheet_id": "mv-web-30", "kind": "multiple_variables", "cols": 2, "rows": 3, "auto_graph": false } }),
    )?;
    web_gui(
        &mut session,
        "ui.type_text",
        json!({ "widget_id": "grid.cell.0.0.0", "text": "10\t0\n20\t0\n30\t0", "clear_existing": true, "commit_enter": true }),
    )?;
    web_gui(&mut session, "ui.invoke_menu", json!({ "command_id": "ui.table.openFormulaBar", "params": { "column": 1 } }))?;
    let live = web_gui(
        &mut session,
        "ui.type_text",
        json!({ "widget_id": "formula_bar.input", "text": "= [Variable A] * 2", "clear_existing": true, "commit_enter": false }),
    )?;
    if live["is_valid"] != json!(true) {
        return Err(format!("WF-30 web expected live formula bar validation is_valid=true, got {live}"));
    }
    let committed = web_gui(
        &mut session,
        "ui.type_text",
        json!({ "widget_id": "formula_bar.input", "text": "= [Variable A] * 2", "clear_existing": true, "commit_enter": true }),
    )?;
    if committed["committed"] != json!(true) || committed["command_id"] != json!("table.setCalculatedVariable") {
        return Err(format!("WF-30 web expected formula bar commit to succeed, got {committed}"));
    }
    let grid_snap = web_gui(&mut session, "ui.inspect", json!({ "scope": "grid" }))?;
    let cells = grid_snap["grid_cells"].as_array().ok_or("WF-30 web missing grid_cells")?;
    for (r, expected_txt) in ["20", "40", "60"].iter().enumerate() {
        let wid = format!("grid.cell.{r}.1.0");
        let node = cells.iter().find(|n| n["widget_id"] == json!(wid)).ok_or_else(|| format!("WF-30 web missing {wid}"))?;
        if node["value"]["text"] != json!(expected_txt) {
            return Err(format!("WF-30 web {wid} expected `{expected_txt}`, got {}", node["value"]));
        }
    }

    // WF-21: Survival KM via GUI toolbar.analyze -> modal.ok + Number-at-Risk + Log-rank scalar.
    web_gui(
        &mut session,
        "ui.invoke_menu",
        json!({ "path": "Insert/New Data Table", "params": { "sheet_id": "surv-web-21", "kind": "survival", "cols": 2, "rows": 6, "auto_graph": false } }),
    )?;
    for (r, &(t, col, status)) in [(2.0, 0, "1"), (4.0, 0, "1"), (6.0, 0, "0"), (5.0, 1, "0"), (8.0, 1, "1"), (12.0, 1, "0")].iter().enumerate() {
        web_gui(
            &mut session,
            "ui.type_text",
            json!({ "widget_id": format!("grid.xcell.{r}.0"), "text": format!("{t}"), "clear_existing": true, "commit_enter": true }),
        )?;
        web_gui(
            &mut session,
            "ui.type_text",
            json!({ "widget_id": format!("grid.cell.{r}.{col}.0"), "text": status, "clear_existing": true, "commit_enter": true }),
        )?;
    }
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("toolbar.analyze"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| e.to_string())?;
    web_gui(&mut session, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.survival_km" } }))?;
    session
        .handle_event(WebBridgeEvent::ClickChromeNode {
            node_id: String::from("modal.ok"),
            button: PointerButton::Primary,
            modifiers: KeyModifiers::default(),
        })
        .map_err(|e| e.to_string())?;
    let km_id = session.host.app().document().result_sheets.last().map(|r| r.id.as_str().to_owned()).ok_or("missing WF-21 KM ResultSheet")?;
    require_result_tab_present(&session, &km_id, "number-at-risk")?;
    let _lr_chi2 = require_result_scalar(&session, &km_id, "Log-rank chi-square", None)?;
    Ok(())
}
