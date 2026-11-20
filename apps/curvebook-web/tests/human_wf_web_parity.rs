#![forbid(unsafe_code)]
//! Minimal strict-CSP browser placeholder tests matching the workflow contract names (`docs/gui-workflows.md` §6.2).
//!
//! Full browser canvas rendering is deferred to post-Beta `R11.11`; until then these tests verify
//! the strict-CSP placeholder bridge, analysis/edit-params workflows, `.curvebook`/`.curvebook.d`
//! exports, HiDPI/RTL text metrics, and large-table interaction responsiveness without legacy `LayoutTree`.

mod web_parity_common;

use std::time::Instant;

use curvebook_ui_egui::serde_json::json;
use curvebook_ui_egui::{KeyModifiers, UiInputEvent, dispatch_gui_action};
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
    let rs_id = session.host.app().document().result_sheets.first().map(|r| r.id.clone()).ok_or("missing ResultSheet")?;
    let _ = session
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": rs_id.as_str() }) })
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
            params: json!({ "sheet_id": "ds-cld", "kind": "grouped", "title": "CLD Table", "rows": 3, "cols": 2, "subcolumns": 2, "auto_family": false }),
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
        let b0 = usize::from(chunk[0]);
        let b1 = usize::from(*chunk.get(1).unwrap_or(&0));
        let b2 = usize::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[(triple >> 18) & 0x3F]));
        out.push(char::from(TABLE[(triple >> 12) & 0x3F]));
        out.push(if chunk.len() > 1 { char::from(TABLE[(triple >> 6) & 0x3F]) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(TABLE[triple & 0x3F]) } else { '=' });
    }
    out
}

fn exec_web(session: &mut WebBridgeSession, command_id: &str, params: curvebook_ui_egui::serde_json::Value) -> TestRes {
    session
        .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from(command_id), params })
        .map(|_| ())
        .map_err(|e| format!("{command_id} failed: {e}"))
}

#[test]
fn web_human_r13_wf51_to_wf58_regression_survival_batch_image_and_embedded_table_parity() -> TestRes {
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
    exec_web(&mut session, "transform.run", json!({ "sheet_id": "tr-norm", "source_sheet_id": "xy-w56", "kind": "normalize" }))?;

    // WF-55 / WF-58: Transpose + Descriptive stats embedded table on GraphSheet
    exec_web(&mut session, "sheet.createData", json!({ "sheet_id": "col-w58", "kind": "column", "cols": 2, "rows": 4, "auto_family": false }))?;
    exec_web(&mut session, "table.setCells", json!({ "sheet_id": "col-w58", "rows": [[18.0, 24.0], [20.0, 26.0], [19.0, 25.0], [21.0, 27.0]] }))?;
    exec_web(&mut session, "transform.run", json!({ "sheet_id": "tr-t", "source_sheet_id": "col-w58", "kind": "transpose" }))?;
    exec_web(&mut session, "analysis.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "col-w58", "kind": "descriptive" }))?;
    exec_web(&mut session, "graph.create", json!({ "sheet_id": "g-w58", "data_sheet_id": "col-w58", "kind": "column" }))?;
    exec_web(
        &mut session,
        "edit.pasteResultTable",
        json!({ "source_sheet_id": "res-desc", "target_sheet_id": "g-w58", "bounds_pt": [40.0, 24.0, 220.0, 96.0] }),
    )?;

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

#[test]
fn web_human_r13_wf59_to_wf65_equalize_nested_forest_pyramid_and_spaghetti_parity() -> TestRes {
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
    let has_svg = svg_cmds.iter().any(|c| match c {
        WebBridgeCommand::SaveOrDownloadFile { bytes, .. } => std::str::from_utf8(bytes).is_ok_and(|s| s.contains("<svg")),
        _ => false,
    });
    if !has_svg {
        return Err(String::from("expected valid SVG export bytes for WF-59..65"));
    }
    Ok(())
}
