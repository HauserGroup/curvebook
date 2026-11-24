//! Permanent regression suite for Round 2 (`R0.19`) desktop host findings
//! (`desktop_host.md`: B2, B4, U7, U8, M2, M3, NEW-1, NEW-2, NEW-3) and
//! `R2-S11` native window findings (`01, 02, 07, 09, 14, 17`).
#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook_platform::sync::RwLock;

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, OsClipboardAdapter, dir_has_unrecovered_crash, find_stale_recovery_dirs,
};
use curvebook::gui_window::{CurveBookNativeApp, NATIVE_OPEN_FILE_EXTENSIONS, build_egui_font_definitions, menu_action_triggers_native_file_dialog};
use curvebook_doc::{CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_format::{FormatRegistry, save_project_file, save_project_unpacked_dir};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::{ClipboardBundle, OsAdapterAvailability, OsAdapterStatus};
use eframe::egui;
use serde_json::json;

static AUDIT_R2_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = AUDIT_R2_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-r2-desk-{label}-{}-{seq}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn default_viewport() -> ViewportMetrics {
    ViewportMetrics::new(1280.0, 800.0, 2.0)
}

#[derive(Debug)]
struct ScriptedOsClipboard {
    available: bool,
    fail_write_text: Option<String>,
    fail_write_bundle: Option<String>,
    fail_read_text: Option<String>,
    read_queue: RwLock<VecDeque<Option<String>>>,
    written_texts: RwLock<Vec<String>>,
    written_bundles: RwLock<Vec<String>>,
    written_slide_bundles: RwLock<Vec<ClipboardBundle>>,
}

impl ScriptedOsClipboard {
    fn available() -> Self {
        Self {
            available: true,
            fail_write_text: None,
            fail_write_bundle: None,
            fail_read_text: None,
            read_queue: RwLock::new(VecDeque::new()),
            written_texts: RwLock::new(Vec::new()),
            written_bundles: RwLock::new(Vec::new()),
            written_slide_bundles: RwLock::new(Vec::new()),
        }
    }

    fn unavailable() -> Self {
        Self { available: false, ..Self::available() }
    }

    fn set_external_text(&self, text: &str) {
        if let Ok(mut q) = self.read_queue.write() {
            q.push_back(Some(text.to_owned()));
        }
    }
}

impl OsClipboardAdapter for ScriptedOsClipboard {
    fn write_text(&self, text: &str) -> Result<OsAdapterStatus, String> {
        if let Some(err) = &self.fail_write_text {
            return Err(err.clone());
        }
        if !self.available {
            return Ok(OsAdapterStatus::unavailable("mock", "no clipboard daemon"));
        }
        if let Ok(mut guard) = self.written_texts.write() {
            guard.push(text.to_owned());
        }
        if let Ok(mut q) = self.read_queue.write() {
            q.clear();
            q.push_back(Some(text.to_owned()));
        }
        Ok(OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Submitted,
            backend: String::from("mock"),
            detail: String::from("ok"),
            text: None,
            job_id: None,
        })
    }

    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        if let Some(err) = &self.fail_write_bundle {
            return Err(err.clone());
        }
        if !self.available {
            return Ok(OsAdapterStatus::unavailable("mock", "no clipboard daemon"));
        }
        if let Ok(mut guard) = self.written_bundles.write() {
            guard.push(bundle.svg.clone());
        }
        if let Ok(mut q) = self.read_queue.write() {
            q.clear();
            q.push_back(Some(bundle.svg.clone()));
        }
        Ok(OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Submitted,
            backend: String::from("mock"),
            detail: String::from("ok"),
            text: None,
            job_id: None,
        })
    }

    fn write_slide_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        if let Some(err) = &self.fail_write_bundle {
            return Err(err.clone());
        }
        if !self.available {
            return Ok(OsAdapterStatus::unavailable("mock", "no clipboard daemon"));
        }
        if let Ok(mut guard) = self.written_slide_bundles.write() {
            guard.push(bundle.clone());
        }
        Ok(OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Submitted,
            backend: String::from("mock"),
            detail: String::from("slides-ok"),
            text: None,
            job_id: None,
        })
    }

    fn read_text(&self) -> Result<OsAdapterStatus, String> {
        if let Some(err) = &self.fail_read_text {
            return Err(err.clone());
        }
        if !self.available {
            return Ok(OsAdapterStatus::unavailable("mock", "no clipboard daemon"));
        }
        let next = self.read_queue.write().ok().and_then(|mut q| q.pop_front()).flatten();
        Ok(OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Read,
            backend: String::from("mock"),
            detail: String::from("ok"),
            text: next,
            job_id: None,
        })
    }
}

#[test]
fn b2_menu_ui_command_checkpoints_recovery_journal_across_crash() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("b2-checkpoint")?;
    let rec_root = dir.path.join("recovery");
    let vp = default_viewport();

    let target_graph_id = {
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(rec_root.clone()), ..DesktopHostConfig::default() });
        let win_id = rt.open_window(None, vp)?;
        let graph_id = {
            let win = rt.window_mut(win_id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "b2-ds", "kind": "column", "cols": 1, "rows": 3, "auto_graph": true }))?;
            let gid = win.app().document().graph_sheets.first().ok_or("missing auto graph")?.id.clone();
            win.execute_and_journal("sheet.select", json!({ "sheet_id": gid.as_str() }))?;
            gid
        };
        let before_width = rt.window(win_id)?.app().document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.canvas_size_pt.width;

        let outcome = rt.dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.canvas.applyPreset") })?;
        assert!(outcome.handled);
        assert_eq!(outcome.action, "menu.ui_command");
        let win = rt.window(win_id)?;
        let after_width = win.app().document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.canvas_size_pt.width;
        assert!((after_width - before_width).abs() > 1.0, "ui.canvas.applyPreset should update graph dimensions");
        assert!(
            win.dirty_commands().iter().any(|c| c == "ui.canvas.applyPreset"),
            "dirty_commands must record menu UI mutation: {:?}",
            win.dirty_commands()
        );
        graph_id
        // Simulate SIGKILL (drop without close_all_force).
    };

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(rec_root, vp, u64::MAX / 2)?;
    let rec_win = rec_rt.window(rec_id)?;
    assert!(rec_win.recovered_from_crash());
    let recovered_graph = rec_win.app().document().graph_sheet(&target_graph_id).ok_or("missing recovered graph")?;
    assert!(
        (recovered_graph.spec.canvas_size_pt.width - (88.0 * 72.0 / 25.4)).abs() < 0.25,
        "menu-originated ui.canvas.applyPreset width must survive crash recovery, got {}",
        recovered_graph.spec.canvas_size_pt.width
    );
    rec_rt.close_all_force()?;
    Ok(())
}

#[test]
fn b4_wheel_pans_on_unmodified_scroll_zooms_with_ctrl_meta_and_scrolls_split_grid() -> Result<(), Box<dyn std::error::Error>> {
    let vp = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let wid = rt.open_window(None, vp)?;
    {
        let win = rt.window_mut(wid)?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "b4-ds", "kind": "column", "cols": 2, "rows": 80, "auto_graph": true }))?;
        let graph_id = win.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
        win.execute_and_journal("sheet.select", json!({ "sheet_id": graph_id.as_str() }))?;
    }

    let snap = curvebook_ui_egui::inspect_ui_tree_for_metrics(rt.window(wid)?.app(), vp, Some("canvas"), true);
    let viewport_bounds = snap.find_widget("canvas.viewport").and_then(|n| n.bounds).ok_or("missing canvas.viewport bounds")?;
    let center = vp.physical_to_logical_px([viewport_bounds[0] + viewport_bounds[2] * 0.5, viewport_bounds[1] + viewport_bounds[3] * 0.5]);

    // 1. Unmodified vertical wheel MUST pan, not zoom.
    let zoom0 = rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    let pan0 = rt.window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt;
    let vert = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: center[0],
        y_pt: center[1],
        delta_x: 0.0,
        delta_y: -36.0,
        modifiers: KeyModifiers::default(),
    })?;
    assert!(vert.handled);
    assert!((rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom0).abs() < 1e-9);
    assert_ne!(rt.window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt, pan0);

    // 2. Unmodified horizontal wheel MUST pan without zooming.
    let pan1 = rt.window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt;
    let horiz = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: center[0],
        y_pt: center[1],
        delta_x: 24.0,
        delta_y: 0.0,
        modifiers: KeyModifiers::default(),
    })?;
    assert!(horiz.handled);
    assert!((rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom0).abs() < 1e-9);
    assert_ne!(rt.window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt, pan1);

    // 3. Ctrl+Wheel and Meta+Wheel MUST zoom (negative delta_y zooms in, positive zooms out).
    let ctrl_zoom = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: center[0],
        y_pt: center[1],
        delta_x: 0.0,
        delta_y: -48.0,
        modifiers: KeyModifiers { ctrl: true, ..KeyModifiers::default() },
    })?;
    assert!(ctrl_zoom.handled);
    let zoom1 = rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    assert!(zoom1 > zoom0);

    let meta_zoom = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: center[0],
        y_pt: center[1],
        delta_x: 0.0,
        delta_y: -48.0,
        modifiers: KeyModifiers { meta: true, ..KeyModifiers::default() },
    })?;
    assert!(meta_zoom.handled);
    let zoom2 = rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    assert!(zoom2 > zoom1);

    let ctrl_zoom_out = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: center[0],
        y_pt: center[1],
        delta_x: 0.0,
        delta_y: 48.0,
        modifiers: KeyModifiers { ctrl: true, ..KeyModifiers::default() },
    })?;
    assert!(ctrl_zoom_out.handled);
    assert!(rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom < zoom2);

    // 4. Open modal suppresses wheel cleanly.
    rt.window_mut(wid)?.app_mut().canvas_dialogs.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::new("b4-ds") };
    let modal_ignored = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: center[0],
        y_pt: center[1],
        delta_x: 0.0,
        delta_y: 48.0,
        modifiers: KeyModifiers { ctrl: true, ..KeyModifiers::default() },
    })?;
    assert!(!modal_ignored.handled);
    assert_eq!(modal_ignored.action, "wheel.dialog_open_ignored");
    rt.window_mut(wid)?.app_mut().canvas_dialogs.active_dialog = ActiveDialog::None;

    // 5. Split-pane DataGrid header/gutter within grid_geometry.sheet_rect scrolls grid cleanly without altering graph zoom.
    rt.dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.split.toggle") })?;
    let split_snap = curvebook_ui_egui::inspect_ui_tree_for_metrics(rt.window(wid)?.app(), vp, None, true);
    let g_rect = split_snap.find_widget("center.pane.primary").and_then(|n| n.bounds).ok_or("missing center.pane.primary bounds")?;
    let hdr_pt = vp.physical_to_logical_px([g_rect[0] + 12.0, g_rect[1] + g_rect[3] * 0.5]);
    let zoom_split = rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    let grid_scroll = rt.dispatch(DesktopHostEvent::MouseWheel {
        window_id: wid,
        x_pt: hdr_pt[0],
        y_pt: hdr_pt[1],
        delta_x: 0.0,
        delta_y: -48.0,
        modifiers: KeyModifiers::default(),
    })?;
    assert!(grid_scroll.handled);
    assert_eq!(grid_scroll.action, "grid.scroll");
    assert!((rt.window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom_split).abs() < 1e-9);
    assert!(rt.window(wid)?.app().grid.tool.viewport.scroll_y_px > 0.0);
    Ok(())
}

#[test]
fn u7_u8_clipboard_reads_os_first_blocks_bundle_into_table_and_surfaces_os_status() -> Result<(), Box<dyn std::error::Error>> {
    let vp = default_viewport();
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let wid = rt.open_window(None, vp)?;
    {
        let win = rt.window_mut(wid)?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "cb-ds", "kind": "column", "cols": 2, "rows": 3, "auto_graph": true }))?;
        win.execute_and_journal("table.setCell", json!({ "sheet_id": "cb-ds", "row": 0, "col": 0, "value": 10.0 }))?;
        win.app_mut().grid.select_single_sync(CellCoord::new(0, 0, 0).into());
    }

    // 1. OS available: copy TSV then external OS pasteboard change MUST win over stale in-app cache (`U7`).
    let mock = ScriptedOsClipboard::available();
    mock.set_external_text("555.25");
    rt.set_clipboard_adapter(Box::new(mock));
    let copy_ok = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy_ok.action, "clipboard.copied_tsv");
    assert_eq!(
        copy_ok.command_result.as_ref().and_then(|v| v.get("os_clipboard")).and_then(|s| s.get("available")).and_then(|b| b.as_bool()),
        Some(true)
    );

    // Replace adapter with external text without calling ClipboardCopy in-app.
    let ext = ScriptedOsClipboard::available();
    ext.set_external_text("777.5");
    rt.set_clipboard_adapter(Box::new(ext));
    let paste_ext = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?;
    assert!(paste_ext.handled);
    let cell_after = rt
        .window(wid)?
        .app()
        .document()
        .data_sheet(&SheetId::new("cb-ds"))
        .and_then(|ds| ds.cell(CellCoord::new(0, 0, 0)))
        .and_then(|c| c.active_number());
    assert_eq!(cell_after, Some(777.5));

    // 2. Graph bundle copy -> paste into DataSheet MUST NOT insert SVG/HTML (`U7`).
    let graph_id = rt.window(wid)?.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
    rt.window_mut(wid)?.execute_and_journal("sheet.select", json!({ "sheet_id": graph_id.as_str() }))?;
    let copy_bundle = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy_bundle.action, "clipboard.copied_bundle");
    assert!(rt.clipboard_bundle().is_some());
    assert!(rt.clipboard_text().is_none());

    rt.window_mut(wid)?.execute_and_journal("sheet.select", json!({ "sheet_id": "cb-ds" }))?;
    let paste_bundle = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?;
    assert!(!paste_bundle.handled);
    assert_eq!(paste_bundle.action, "clipboard.paste_bundle_not_tabular");

    // 3. OS unavailable falls back to plain-text TSV cache AND surfaces `_os_unavailable` (`U7` + `U8`).
    rt.set_clipboard_adapter(Box::new(ScriptedOsClipboard::unavailable()));
    let copy_unavail = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy_unavail.action, "clipboard.copied_tsv_os_unavailable");
    assert_eq!(
        copy_unavail.command_result.as_ref().and_then(|v| v.get("os_clipboard")).and_then(|s| s.get("available")).and_then(|b| b.as_bool()),
        Some(false)
    );
    let paste_fallback = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?;
    assert!(paste_fallback.handled);

    // 4. OS write/read errors surface distinctly (`U8`): both when cache fallback is used AND when cache is empty.
    let mut failing = ScriptedOsClipboard::available();
    failing.fail_write_text = Some(String::from("pbcopy exited 1"));
    failing.fail_read_text = Some(String::from("pbpaste exited 1"));
    rt.set_clipboard_adapter(Box::new(failing));
    let copy_err = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy_err.action, "clipboard.copied_tsv_os_failed");
    assert_eq!(
        copy_err.command_result.as_ref().and_then(|v| v.get("os_clipboard")).and_then(|s| s.get("error")).and_then(|s| s.as_str()),
        Some("pbcopy exited 1")
    );
    let paste_fallback_with_err = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?;
    assert!(paste_fallback_with_err.handled, "cached plain TSV fallback should still paste in headless/sandboxed OS");
    assert_eq!(
        paste_fallback_with_err.command_result.as_ref().and_then(|v| v.get("os_clipboard")).and_then(|s| s.get("error")).and_then(|s| s.as_str()),
        Some("pbpaste exited 1")
    );

    let mut fresh_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let fresh_wid = fresh_rt.open_window(None, vp)?;
    fresh_rt
        .window_mut(fresh_wid)?
        .execute_and_journal("sheet.createData", json!({ "sheet_id": "fresh-ds", "kind": "column", "cols": 1, "rows": 2 }))?;
    let mut empty_fail = ScriptedOsClipboard::available();
    empty_fail.fail_read_text = Some(String::from("pbpaste exited 1"));
    fresh_rt.set_clipboard_adapter(Box::new(empty_fail));
    let paste_err = fresh_rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: fresh_wid, text_override: None })?;
    assert!(!paste_err.handled);
    assert_eq!(paste_err.action, "clipboard.paste_os_failed");
    assert_eq!(
        paste_err.command_result.as_ref().and_then(|v| v.get("os_clipboard")).and_then(|s| s.get("error")).and_then(|s| s.as_str()),
        Some("pbpaste exited 1")
    );
    Ok(())
}

#[test]
fn m2_plain_directory_drop_reports_unsupported_directory_instead_of_manifest_error() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("m2-plain-dir")?;
    let plain_dir = dir.path.join("not-a-project");
    fs::create_dir_all(&plain_dir)?;

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let wid = rt.open_window(None, default_viewport())?;
    let err = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: plain_dir }).expect_err("plain directory must fail");
    assert!(err.contains("unsupported directory"), "unexpected error: {err}");
    assert!(!err.contains("failed to open unpacked"), "must not misroute plain dir to .curvebook.d importer: {err}");

    // Genuine unpacked .curvebook.d directory still loads cleanly.
    let unpacked_dir = dir.path.join("valid.curvebook.d");
    let mut doc = ProjectDocument::new("Valid Unpacked")?;
    doc.add_data_sheet(DataSheet::new("u1", "Unpacked", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::default(), 1, 2)?))?)?;
    save_project_unpacked_dir(&doc, &unpacked_dir)?;
    let ok = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: unpacked_dir })?;
    assert!(ok.handled);
    assert!(rt.window(wid)?.app().document().data_sheet(&SheetId::new("u1")).is_some());
    Ok(())
}

#[test]
fn m3_render_frame_shares_rgba_arc_zero_copy_with_last_frame() -> Result<(), Box<dyn std::error::Error>> {
    let vp = ViewportMetrics::new(640.0, 400.0, 2.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let wid = rt.open_window(None, vp)?;
    let frame = rt.dispatch(DesktopHostEvent::RedrawRequested { window_id: wid })?.frame.ok_or("missing rendered frame")?;
    let cached = rt.window(wid)?.last_frame().ok_or("missing cached last_frame")?;
    assert!(Arc::ptr_eq(&frame.rgba_bytes, &cached.rgba_bytes), "last_frame must share Arc<[u8]> zero-copy");

    let encoded = serde_json::to_string(&frame)?;
    let decoded: curvebook::desktop_host::DesktopFrameOutput = serde_json::from_str(&encoded)?;
    assert_eq!(decoded.width_px, frame.width_px);
    assert_eq!(decoded.rgba_bytes.len(), frame.rgba_bytes.len());
    Ok(())
}

#[test]
fn new1_new2_new3_recovery_attaches_cli_file_preserves_crashes_and_scans_stale_dirs() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("new123-rec")?;
    let rec_root = dir.path.join("recovery-root");
    let vp = default_viewport();

    let proj_path = dir.path.join("opened.curvebook");
    let mut opened_doc = ProjectDocument::new("Opened File")?;
    opened_doc.add_data_sheet(DataSheet::new("open-s1", "Opened", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::default(), 1, 2)?))?)?;
    save_project_file(&proj_path, &opened_doc)?;

    // 1. NEW-1: open_window(Some(file)) with new_instance=false and recovery_root=Some MUST attach a recovery journal.
    let clean_sub = dir.path.join("clean-sub-root");
    {
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(clean_sub.clone()), ..DesktopHostConfig::default() });
        let w1 = rt.open_window(Some(proj_path.clone()), vp)?;
        assert!(rt.window(w1)?.has_recovery_journal(), "NEW-1: initial file window must attach recovery journal");
        rt.window_mut(w1)?.execute_and_journal("table.setCell", json!({ "sheet_id": "open-s1", "row": 0, "col": 0, "value": 31.25 }))?;
    }
    let mut recheck = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_open = recheck.recover_window_from_dir(clean_sub, vp, u64::MAX / 2)?;
    assert_eq!(
        recheck
            .window(rec_open)?
            .app()
            .document()
            .data_sheet(&SheetId::new("open-s1"))
            .and_then(|s| s.cell(CellCoord::new(0, 0, 0)))
            .and_then(|c| c.active_number()),
        Some(31.25)
    );
    recheck.close_all_force()?;

    // 2. NEW-2: Pre-seed an unrecovered crash journal inside `rec_root/win-2` AND a crashed `--new-instance` journal in `instance-99999-win-1`.
    let win2_crash_dir = rec_root.join("win-2");
    {
        let (mut s2, mut j2, _) = Session::open_dir_with_recovery(&win2_crash_dir, "crashed-win2", 1_000, 100)?;
        s2.execute_and_record(&mut j2, "sheet.createData", json!({ "sheet_id": "win2-lost", "title": "Win2 Crashed" }))?;
        s2.execute_and_record(&mut j2, "table.setCell", json!({ "sheet_id": "win2-lost", "row": 0, "col": 0, "value": 64.5 }))?;
        drop(j2.abandon());
    }
    let pid_crash_dir = rec_root.join("instance-99999-win-1");
    {
        let (mut sp, mut jp, _) = Session::open_dir_with_recovery(&pid_crash_dir, "crashed-inst", 1_000, 100)?;
        sp.execute_and_record(&mut jp, "sheet.createData", json!({ "sheet_id": "inst-lost", "title": "Inst Crashed" }))?;
        drop(jp.abandon());
    }

    let now_stale = 10_000_000;
    assert!(dir_has_unrecovered_crash(&win2_crash_dir, now_stale));
    assert!(dir_has_unrecovered_crash(&pid_crash_dir, now_stale));
    assert_eq!(find_stale_recovery_dirs(&rec_root, now_stale), vec![pid_crash_dir.clone(), win2_crash_dir.clone()]);

    // Open win-1 (clean) + win-2 with `proj_path` (document-bearing session).
    // NEW-2: win-2 MUST NOT overwrite `rec_root/win-2` or inherit its `recovered_from_crash` flag!
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(rec_root.clone()), ..DesktopHostConfig::default() });
    let w1 = rt.open_window(None, vp)?;
    assert_eq!(rt.window(w1)?.recovery_dir().map(|p| p.to_path_buf()), Some(rec_root.clone()));
    let w2 = rt.open_window(Some(proj_path), vp)?;
    assert!(!rt.window(w2)?.recovered_from_crash(), "newly opened file window must not masquerade as recovered crash");
    assert_ne!(rt.window(w2)?.recovery_dir(), Some(win2_crash_dir.as_path()));
    assert!(dir_has_unrecovered_crash(&win2_crash_dir, now_stale), "NEW-2: win-2 crash journal must stay intact");

    // 3. NEW-3: recover_stale_journals discovers & recovers BOTH `instance-99999-win-1` and preserved `win-2` without touching active w1/w2!
    let recovered = rt.recover_stale_journals(vp, now_stale)?;
    assert_eq!(recovered.len(), 2);
    let has_inst = recovered.iter().any(|&id| rt.window(id).is_ok_and(|w| w.app().document().data_sheet(&SheetId::new("inst-lost")).is_some()));
    let has_win2 = recovered.iter().any(|&id| {
        rt.window(id).is_ok_and(|w| {
            w.app().document().data_sheet(&SheetId::new("win2-lost")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(|c| c.active_number())
                == Some(64.5)
        })
    });
    assert!(has_inst, "NEW-3: stale --new-instance journal must recover");
    assert!(has_win2, "NEW-2+NEW-3: preserved win-2 crash journal must recover");

    rt.close_all_force()?;
    Ok(())
}

#[test]
fn r2_s11_01_and_17_secondary_window_close_and_menu_bar_visibility() -> Result<(), Box<dyn std::error::Error>> {
    let vp = ViewportMetrics::new(960.0, 640.0, 1.0);
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));

    // 1. R2-S11-01: Closing a clean secondary window must NOT confirm app exit or discard dirty window 1.
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, new_instance: true, ..DesktopHostConfig::default() });
    let w1 = rt.open_window(None, vp)?;
    rt.window_mut(w1)?.execute_and_journal("table.importCsv", json!({ "csv": "X,Y\n1,2\n" }))?;
    let w2 = rt.open_window(None, vp)?;
    assert_eq!(rt.window_count(), 2);
    assert_eq!(rt.focused_window_id(), Some(w2));

    let mut app = CurveBookNativeApp::new(rt, w1, None);
    let mut close_w2_input =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(960.0, 640.0))), ..Default::default() };
    close_w2_input.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    let mut out1 = ctx.run_ui(close_w2_input, |ui| app.step_ui(ui));
    out1.textures_delta.clear();

    assert_eq!(app.runtime().window_count(), 1, "closing clean window 2 must leave dirty window 1 open");
    assert!(!app.close_confirmed(), "closing secondary window must not exit eframe");
    assert!(app.runtime().window(w1)?.has_unsaved_changes());
    assert!(
        out1.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::CancelClose)),
        "closing secondary window must emit CancelClose to keep OS viewport alive"
    );

    // Next close on remaining dirty window 1 opens unsaved modal, NOT exit.
    let mut close_w1_input =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(960.0, 640.0))), ..Default::default() };
    close_w1_input.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    let mut out2 = ctx.run_ui(close_w1_input, |ui| app.step_ui(ui));
    out2.textures_delta.clear();
    assert!(app.close_dialog_open());
    assert!(!app.close_confirmed());

    // 2. R2-S11-17: in_window_menus=false (--no-in-window-menus) hides the egui top menu bar completely.
    let mut rt_visible = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, ..DesktopHostConfig::default() });
    let wv = rt_visible.open_window(None, vp)?;
    assert!(rt_visible.window(wv)?.chrome_state().menu_bar_visible);
    let mut app_visible = CurveBookNativeApp::new(rt_visible, wv, None);
    let raw_vis =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(960.0, 640.0))), ..Default::default() };
    let mut out_vis = ctx.run_ui(raw_vis, |ui| app_visible.step_ui(ui));
    out_vis.textures_delta.clear();
    let visible_canvas_h = app_visible.runtime().window(wv)?.viewport().logical_height;
    assert!(
        visible_canvas_h < 630.0 && app_visible.inspect_meta().find_by_widget_id("menu.file").is_some(),
        "when menu_bar_visible=true, inline egui menu bar must be visible inside top chrome (got canvas_h={visible_canvas_h})"
    );

    let mut rt_hidden = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: false, ..DesktopHostConfig::default() });
    let wh = rt_hidden.open_window(None, vp)?;
    assert!(!rt_hidden.window(wh)?.chrome_state().menu_bar_visible);
    let mut app_hidden = CurveBookNativeApp::new(rt_hidden, wh, None);
    let raw_hidden =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(960.0, 640.0))), ..Default::default() };
    let mut out_hidden = ctx.run_ui(raw_hidden, |ui| app_hidden.step_ui(ui));
    out_hidden.textures_delta.clear();
    let hidden_canvas_h = app_hidden.runtime().window(wh)?.viewport().logical_height;
    assert!(
        hidden_canvas_h >= visible_canvas_h && app_hidden.inspect_meta().find_by_widget_id("menu.file").is_none(),
        "when menu_bar_visible=false, inline menu bar must be hidden and consume no extra height (visible={visible_canvas_h}, hidden={hidden_canvas_h})"
    );
    Ok(())
}

#[test]
fn r2_s11_14_open_filter_matches_format_registry_and_exposes_unpacked_folder_action() {
    let reg = FormatRegistry::default_registry();
    let mut registry_file_exts = Vec::new();
    for desc in reg.supported_import_formats() {
        if desc.id == "curvebook.d" {
            continue;
        }
        for ext in desc.extensions {
            if !registry_file_exts.contains(ext) {
                registry_file_exts.push(*ext);
            }
        }
    }
    for ext in &registry_file_exts {
        assert!(NATIVE_OPEN_FILE_EXTENSIONS.contains(ext), "NATIVE_OPEN_FILE_EXTENSIONS missing registry ext `{ext}`");
    }
    assert!(NATIVE_OPEN_FILE_EXTENSIONS.contains(&"xlsx"));
    assert!(!NATIVE_OPEN_FILE_EXTENSIONS.contains(&"json"));
    assert!(menu_action_triggers_native_file_dialog("file.open_unpacked_dir"));
}

#[test]
fn r17_2_desktop_host_copy_for_slides_routes_at_300_dpi_without_plain_text_svg() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let clip = Arc::new(ScriptedOsClipboard::available());
    #[derive(Debug)]
    struct BorrowedClip(Arc<ScriptedOsClipboard>);
    impl OsClipboardAdapter for BorrowedClip {
        fn write_text(&self, text: &str) -> Result<OsAdapterStatus, String> {
            self.0.write_text(text)
        }
        fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
            self.0.write_bundle(bundle)
        }
        fn write_slide_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
            self.0.write_slide_bundle(bundle)
        }
        fn read_text(&self) -> Result<OsAdapterStatus, String> {
            self.0.read_text()
        }
    }
    rt.set_clipboard_adapter(Box::new(BorrowedClip(Arc::clone(&clip))));
    let win_id = rt.open_window(None, default_viewport())?;
    {
        let win = rt.window_mut(win_id)?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-slides", "kind": "column", "cols": 2, "rows": 3, "auto_graph": true }))?;
        let gid = win.app().document().graph_sheets.first().ok_or("missing auto graph")?.id.clone();
        win.execute_and_journal("sheet.select", json!({ "sheet_id": gid.as_str() }))?;
    }

    // 1. Menu action `ui.clipboard.copyForSlides` writes slide bundle at 300 DPI.
    let menu_out = rt.dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.clipboard.copyForSlides") })?;
    assert!(menu_out.handled);
    assert_eq!(menu_out.action, "clipboard.copied_slide_bundle");
    assert_eq!(rt.clipboard_text(), None, "Copy for Slides must clear plain-text cache rather than staging raw SVG XML");
    let res = menu_out.command_result.as_ref().ok_or("missing command_result")?;
    assert_eq!(res.get("dpi").and_then(serde_json::Value::as_u64), Some(300));
    assert_eq!(res.get("omits_plain_text_svg").and_then(serde_json::Value::as_bool), Some(true));
    assert_eq!(res.get("profile").and_then(serde_json::Value::as_str), Some("slides_and_documents"));

    // 2. Host keyboard shortcut `Cmd+Shift+C` routes through `ui.clipboard.copyForSlides`.
    let key_out = rt.dispatch(DesktopHostEvent::KeyboardInput {
        window_id: win_id,
        key: String::from("C"),
        pressed: true,
        modifiers: KeyModifiers { meta: true, shift: true, ..KeyModifiers::default() },
        text: None,
    })?;
    assert!(key_out.handled);
    assert_eq!(key_out.command_id.as_deref(), Some("ui.clipboard.copyForSlides"));

    let slides = clip.written_slide_bundles.read().map_err(|_| "clip lock")?;
    assert_eq!(slides.len(), 2);
    assert_eq!(slides[0].dpi, 300);
    assert!(slides[0].tiff_bytes.as_deref().is_some_and(|b| b.starts_with(b"II*\0")));
    assert!(slides[0].html.contains("<img src=\"data:image/png;base64,"));
    assert!(clip.written_bundles.read().map_err(|_| "clip lock")?.is_empty(), "slide copy must invoke write_slide_bundle");
    Ok(())
}
