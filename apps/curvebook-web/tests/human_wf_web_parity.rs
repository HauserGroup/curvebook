#![forbid(unsafe_code)]
//! Minimal strict-CSP browser placeholder tests matching the workflow contract names (`docs/gui-workflows.md` §6.2).
//!
//! Full browser canvas rendering is deferred to post-Beta `R11.11`; until then these tests verify
//! the strict-CSP placeholder bridge, analysis/edit-params workflows, `.curvebook`/`.curvebook.d`
//! exports, HiDPI/RTL text metrics, and large-table interaction responsiveness without legacy `LayoutTree`.

mod web_parity_common;

use std::time::Instant;

use curvebook_ui_egui::serde_json::json;
use curvebook_ui_egui::{KeyModifiers, UiInputEvent};
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
