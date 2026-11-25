//! Permanent regression suite for Round 2 (`R0.19`) desktop host findings
//! (`desktop_host.md`: B2, B4, U7, U8, M2, M3, NEW-1, NEW-2, NEW-3) and
//! `R2-S11` native window findings (`01, 02, 07, 09, 14, 17`).
#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::fs;
use std::sync::Arc;

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, OsClipboardAdapter, dir_has_unrecovered_crash, find_stale_recovery_dirs,
};
use curvebook::gui_window::{CurveBookNativeApp, NATIVE_OPEN_FILE_EXTENSIONS, menu_action_triggers_native_file_dialog};
use curvebook_doc::{CellCoord, SheetId};
use curvebook_engine::Session;
use curvebook_format::{FormatRegistry, save_project_file, save_project_unpacked_dir};
use curvebook_platform::sync::RwLock;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{ClipboardBundle, OsAdapterAvailability, OsAdapterStatus};
use eframe::egui;
use serde_json::json;

use crate::common::{NativeHarness, TempDirGuard, build_col_doc};

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

    fn ok_status(availability: OsAdapterAvailability, detail: &str, text: Option<String>) -> OsAdapterStatus {
        OsAdapterStatus { available: true, availability, backend: String::from("mock"), detail: detail.to_owned(), text, job_id: None }
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
        if let Ok(mut g) = self.written_texts.write() {
            g.push(text.to_owned());
        }
        if let Ok(mut q) = self.read_queue.write() {
            q.clear();
            q.push_back(Some(text.to_owned()));
        }
        Ok(Self::ok_status(OsAdapterAvailability::Submitted, "ok", None))
    }

    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        if let Some(err) = &self.fail_write_bundle {
            return Err(err.clone());
        }
        if !self.available {
            return Ok(OsAdapterStatus::unavailable("mock", "no clipboard daemon"));
        }
        if let Ok(mut g) = self.written_bundles.write() {
            g.push(bundle.svg.clone());
        }
        if let Ok(mut q) = self.read_queue.write() {
            q.clear();
            q.push_back(Some(bundle.svg.clone()));
        }
        Ok(Self::ok_status(OsAdapterAvailability::Submitted, "ok", None))
    }

    fn write_slide_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        if let Some(err) = &self.fail_write_bundle {
            return Err(err.clone());
        }
        if !self.available {
            return Ok(OsAdapterStatus::unavailable("mock", "no clipboard daemon"));
        }
        if let Ok(mut g) = self.written_slide_bundles.write() {
            g.push(bundle.clone());
        }
        Ok(Self::ok_status(OsAdapterAvailability::Submitted, "slides-ok", None))
    }

    fn read_text(&self) -> Result<OsAdapterStatus, String> {
        if let Some(err) = &self.fail_read_text {
            return Err(err.clone());
        }
        if !self.available {
            return Ok(OsAdapterStatus::unavailable("mock", "no clipboard daemon"));
        }
        let next = self.read_queue.write().ok().and_then(|mut q| q.pop_front()).flatten();
        Ok(Self::ok_status(OsAdapterAvailability::Read, "ok", next))
    }
}

#[derive(Debug)]
struct SharedClip(Arc<ScriptedOsClipboard>);
impl OsClipboardAdapter for SharedClip {
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

#[test]
fn b2_menu_ui_command_checkpoints_recovery_journal_across_crash() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("b2-checkpoint")?;
    let rec_root = dir.child("recovery");
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
        let before_w = rt.window(win_id)?.app().document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.canvas_size_pt.width;
        let outcome = rt.dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.canvas.applyPreset") })?;
        assert!(outcome.handled && outcome.action == "menu.ui_command");
        let win = rt.window(win_id)?;
        let after_w = win.app().document().graph_sheet(&graph_id).ok_or("missing graph")?.spec.canvas_size_pt.width;
        assert!((after_w - before_w).abs() > 1.0);
        assert!(win.dirty_commands().iter().any(|c| c == "ui.canvas.applyPreset"));
        graph_id
    };

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(rec_root, vp, u64::MAX / 2)?;
    let rec_win = rec_rt.window(rec_id)?;
    assert!(rec_win.recovered_from_crash());
    let w = rec_win.app().document().graph_sheet(&target_graph_id).ok_or("missing recovered graph")?.spec.canvas_size_pt.width;
    assert!((w - (88.0 * 72.0 / 25.4)).abs() < 0.25, "menu width must survive crash recovery, got {w}");
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
        let gid = win.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
        win.execute_and_journal("sheet.select", json!({ "sheet_id": gid.as_str() }))?;
    }

    let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, wid, None));
    h.settle(2);
    let center = h.app.inspect_meta().find_by_widget_id("canvas.viewport").ok_or("missing canvas.viewport")?.clipped_rect.center();

    // 1-2. Unmodified vertical & horizontal wheel pan without zooming.
    let zoom0 = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    let pan0 = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt;
    h.send_wheel(center, egui::vec2(0.0, -36.0), egui::Modifiers::NONE);
    assert!((h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom0).abs() < 1e-9);
    let pan1 = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt;
    assert_ne!(pan1, pan0);

    h.send_wheel(center, egui::vec2(24.0, 0.0), egui::Modifiers::NONE);
    assert!((h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom0).abs() < 1e-9);
    assert_ne!(h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt, pan1);

    // 3. Ctrl+Wheel and Meta+Wheel zoom in/out.
    let ctrl_mods = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
    let meta_mods = egui::Modifiers { mac_cmd: true, command: true, ..Default::default() };
    h.send_wheel(center, egui::vec2(0.0, 48.0), ctrl_mods);
    let zoom1 = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    assert!(zoom1 > zoom0);
    h.send_wheel(center, egui::vec2(0.0, 48.0), meta_mods);
    let zoom2 = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    assert!(zoom2 > zoom1);
    h.send_wheel(center, egui::vec2(0.0, -48.0), ctrl_mods);
    assert!(h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom < zoom2);

    // 4. Open modal suppresses wheel cleanly.
    h.app.runtime_mut().window_mut(wid)?.app_mut().canvas_dialogs.active_dialog =
        ActiveDialog::AnalyzeSheet { source_sheet_id: SheetId::new("b4-ds") };
    h.settle(1);
    let (zoom_modal, pan_modal) =
        (h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom, h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt);
    h.send_wheel(center, egui::vec2(0.0, 48.0), ctrl_mods);
    assert!((h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom_modal).abs() < 1e-9);
    assert_eq!(h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.pan_pt, pan_modal);
    h.app.runtime_mut().window_mut(wid)?.app_mut().canvas_dialogs.active_dialog = ActiveDialog::None;
    h.settle(1);

    // 5. Split-pane DataGrid scrolls grid without altering graph zoom.
    h.app.runtime_mut().window_mut(wid)?.execute_and_journal("sheet.select", json!({ "sheet_id": "b4-ds" }))?;
    h.app.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: wid, command_id: String::from("ui.split.toggle") })?;
    h.settle(2);
    let hdr_pt = h.app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing split grid.cell.0.0.0")?.clipped_rect.center();
    let zoom_split = h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom;
    h.send_wheel(hdr_pt, egui::vec2(0.0, -48.0), egui::Modifiers::NONE);
    assert!((h.app.runtime().window(wid)?.app().canvas_dialogs.pan_zoom.zoom - zoom_split).abs() < 1e-9);
    assert!(h.app.runtime().window(wid)?.app().egui_views.column_grid.scroll_offset_pt.y > 0.0);
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

    // 1. OS available: external OS pasteboard wins over stale in-app cache (`U7`).
    let mock = ScriptedOsClipboard::available();
    mock.set_external_text("555.25");
    rt.set_clipboard_adapter(Box::new(mock));
    let copy_ok = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy_ok.action, "clipboard.copied_tsv");
    assert_eq!(copy_ok.command_result.as_ref().and_then(|v| v.pointer("/os_clipboard/available")).and_then(serde_json::Value::as_bool), Some(true));

    let ext = ScriptedOsClipboard::available();
    ext.set_external_text("777.5");
    rt.set_clipboard_adapter(Box::new(ext));
    assert!(rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?.handled);
    let cell_after = rt
        .window(wid)?
        .app()
        .document()
        .data_sheet(&SheetId::new("cb-ds"))
        .and_then(|ds| ds.cell(CellCoord::new(0, 0, 0)))
        .and_then(|c| c.active_number());
    assert_eq!(cell_after, Some(777.5));

    // 2. Graph bundle copy -> paste into DataSheet blocked (`U7`).
    let gid = rt.window(wid)?.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
    rt.window_mut(wid)?.execute_and_journal("sheet.select", json!({ "sheet_id": gid.as_str() }))?;
    let copy_bundle = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy_bundle.action, "clipboard.copied_bundle");
    assert!(rt.clipboard_bundle().is_some() && rt.clipboard_text().is_none());

    rt.window_mut(wid)?.execute_and_journal("sheet.select", json!({ "sheet_id": "cb-ds" }))?;
    let paste_bundle = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?;
    assert!(!paste_bundle.handled && paste_bundle.action == "clipboard.paste_bundle_not_tabular");

    // 3. OS unavailable falls back to TSV cache & surfaces `_os_unavailable` (`U7` + `U8`).
    rt.set_clipboard_adapter(Box::new(ScriptedOsClipboard::unavailable()));
    let copy_unavail = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy_unavail.action, "clipboard.copied_tsv_os_unavailable");
    assert_eq!(
        copy_unavail.command_result.as_ref().and_then(|v| v.pointer("/os_clipboard/available")).and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert!(rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?.handled);

    // 4. OS write/read errors surface distinctly both with and without cache fallback (`U8`).
    let mut failing = ScriptedOsClipboard::available();
    failing.fail_write_text = Some(String::from("pbcopy exited 1"));
    failing.fail_read_text = Some(String::from("pbpaste exited 1"));
    rt.set_clipboard_adapter(Box::new(failing));
    let copy_err = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: wid })?;
    assert_eq!(copy_err.action, "clipboard.copied_tsv_os_failed");
    assert_eq!(
        copy_err.command_result.as_ref().and_then(|v| v.pointer("/os_clipboard/error")).and_then(serde_json::Value::as_str),
        Some("pbcopy exited 1")
    );
    let paste_fb = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: wid, text_override: None })?;
    assert!(paste_fb.handled);
    assert_eq!(
        paste_fb.command_result.as_ref().and_then(|v| v.pointer("/os_clipboard/error")).and_then(serde_json::Value::as_str),
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
    assert!(!paste_err.handled && paste_err.action == "clipboard.paste_os_failed");
    assert_eq!(
        paste_err.command_result.as_ref().and_then(|v| v.pointer("/os_clipboard/error")).and_then(serde_json::Value::as_str),
        Some("pbpaste exited 1")
    );
    Ok(())
}

#[test]
fn m2_plain_directory_drop_reports_unsupported_directory_instead_of_manifest_error() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("m2-plain-dir")?;
    let plain_dir = dir.child("not-a-project");
    fs::create_dir_all(&plain_dir)?;

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let wid = rt.open_window(None, default_viewport())?;
    let err = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: plain_dir }).expect_err("plain directory must fail");
    assert!(err.contains("unsupported directory") && !err.contains("failed to open unpacked"), "unexpected error: {err}");

    let unpacked_dir = dir.child("valid.curvebook.d");
    save_project_unpacked_dir(&build_col_doc("u1", "Unpacked", &[("A", &[1.0, 2.0])])?, &unpacked_dir)?;
    assert!(rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: unpacked_dir })?.handled);
    assert!(rt.window(wid)?.app().document().data_sheet(&SheetId::new("u1")).is_some());
    Ok(())
}

#[test]
fn m3_render_frame_shares_rgba_arc_zero_copy_with_last_frame() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let wid = rt.open_window(None, ViewportMetrics::new(640.0, 400.0, 2.0))?;
    let frame = rt.dispatch(DesktopHostEvent::RedrawRequested { window_id: wid })?.frame.ok_or("missing rendered frame")?;
    let cached = rt.window(wid)?.last_frame().ok_or("missing cached last_frame")?;
    assert!(Arc::ptr_eq(&frame.rgba_bytes, &cached.rgba_bytes));

    let decoded: curvebook::desktop_host::DesktopFrameOutput = serde_json::from_str(&serde_json::to_string(&frame)?)?;
    assert_eq!((decoded.width_px, decoded.rgba_bytes.len()), (frame.width_px, frame.rgba_bytes.len()));
    Ok(())
}

#[test]
fn new1_new2_new3_recovery_attaches_cli_file_preserves_crashes_and_scans_stale_dirs() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("new123-rec")?;
    let rec_root = dir.child("recovery-root");
    let vp = default_viewport();
    let proj_path = dir.child("opened.curvebook");
    save_project_file(&proj_path, &build_col_doc("open-s1", "Opened", &[("A", &[1.0, 2.0])])?)?;

    // 1. NEW-1: open_window(Some(file)) with recovery_root=Some attaches a recovery journal.
    let clean_sub = dir.child("clean-sub-root");
    {
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(clean_sub.clone()), ..DesktopHostConfig::default() });
        let w1 = rt.open_window(Some(proj_path.clone()), vp)?;
        assert!(rt.window(w1)?.has_recovery_journal());
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

    // 2. NEW-2: Pre-seed crash journals in `win-2` and `instance-99999-win-1`.
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
    assert!(dir_has_unrecovered_crash(&win2_crash_dir, now_stale) && dir_has_unrecovered_crash(&pid_crash_dir, now_stale));
    assert_eq!(find_stale_recovery_dirs(&rec_root, now_stale), vec![pid_crash_dir, win2_crash_dir.clone()]);

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(rec_root.clone()), ..DesktopHostConfig::default() });
    let w1 = rt.open_window(None, vp)?;
    assert_eq!(rt.window(w1)?.recovery_dir().map(std::path::Path::to_path_buf), Some(rec_root));
    let w2 = rt.open_window(Some(proj_path), vp)?;
    assert!(!rt.window(w2)?.recovered_from_crash());
    assert_ne!(rt.window(w2)?.recovery_dir(), Some(win2_crash_dir.as_path()));
    assert!(dir_has_unrecovered_crash(&win2_crash_dir, now_stale));

    // 3. NEW-3: recover_stale_journals recovers BOTH stale crash dirs.
    let recovered = rt.recover_stale_journals(vp, now_stale)?;
    assert_eq!(recovered.len(), 2);
    assert!(recovered.iter().any(|&id| rt.window(id).is_ok_and(|w| w.app().document().data_sheet(&SheetId::new("inst-lost")).is_some())));
    assert!(recovered.iter().any(|&id| {
        rt.window(id).is_ok_and(|w| {
            w.app().document().data_sheet(&SheetId::new("win2-lost")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(|c| c.active_number())
                == Some(64.5)
        })
    }));
    rt.close_all_force()?;
    Ok(())
}

#[test]
fn r2_s11_01_and_17_secondary_window_close_and_menu_bar_visibility() -> Result<(), Box<dyn std::error::Error>> {
    let vp = ViewportMetrics::new(960.0, 640.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, new_instance: true, ..DesktopHostConfig::default() });
    let w1 = rt.open_window(None, vp)?;
    rt.window_mut(w1)?.execute_and_journal("table.importCsv", json!({ "csv": "X,Y\n1,2\n" }))?;
    let w2 = rt.open_window(None, vp)?;
    assert_eq!((rt.window_count(), rt.focused_window_id()), (2, Some(w2)));

    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, w1, None), egui::vec2(960.0, 640.0));
    let close_raw = || {
        let mut raw = egui::RawInput::default();
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        raw
    };
    let out1 = h.step_raw(close_raw());
    assert_eq!(h.app.runtime().window_count(), 1);
    assert!(!h.app.close_confirmed() && h.app.runtime().window(w1)?.has_unsaved_changes());
    assert!(out1.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::CancelClose)));

    let _ = h.step_raw(close_raw());
    assert!(h.app.close_dialog_open() && !h.app.close_confirmed());

    // R2-S11-17: in_window_menus=false hides top menu bar cleanly.
    let (mut h_vis, wv) =
        NativeHarness::from_config(DesktopHostConfig { in_window_menus: true, initial_viewport: vp, ..DesktopHostConfig::default() }, None)?;
    assert!(h_vis.app.runtime().window(wv)?.chrome_state().menu_bar_visible);
    let _ = h_vis.step_events(Vec::new());
    let visible_canvas_h = h_vis.app.runtime().window(wv)?.viewport().logical_height;
    assert!(visible_canvas_h < 630.0 && h_vis.app.inspect_meta().find_by_widget_id("menu.file").is_some());

    let (mut h_hid, wh) =
        NativeHarness::from_config(DesktopHostConfig { in_window_menus: false, initial_viewport: vp, ..DesktopHostConfig::default() }, None)?;
    assert!(!h_hid.app.runtime().window(wh)?.chrome_state().menu_bar_visible);
    let _ = h_hid.step_events(Vec::new());
    let hidden_canvas_h = h_hid.app.runtime().window(wh)?.viewport().logical_height;
    assert!(hidden_canvas_h >= visible_canvas_h && h_hid.app.inspect_meta().find_by_widget_id("menu.file").is_none());
    Ok(())
}

#[test]
fn r2_s11_14_open_filter_matches_format_registry_and_exposes_unpacked_folder_action() {
    let reg = FormatRegistry::default_registry();
    for desc in reg.supported_import_formats().into_iter().filter(|d| d.id != "curvebook.d") {
        for ext in desc.extensions {
            assert!(NATIVE_OPEN_FILE_EXTENSIONS.contains(ext), "NATIVE_OPEN_FILE_EXTENSIONS missing registry ext `{ext}`");
        }
    }
    assert!(NATIVE_OPEN_FILE_EXTENSIONS.contains(&"xlsx") && !NATIVE_OPEN_FILE_EXTENSIONS.contains(&"json"));
    assert!(menu_action_triggers_native_file_dialog("file.open_unpacked_dir"));
}

#[test]
fn r17_2_desktop_host_copy_for_slides_routes_at_300_dpi_without_plain_text_svg() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let clip = Arc::new(ScriptedOsClipboard::available());
    rt.set_clipboard_adapter(Box::new(SharedClip(Arc::clone(&clip))));
    let win_id = rt.open_window(None, default_viewport())?;
    {
        let win = rt.window_mut(win_id)?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-slides", "kind": "column", "cols": 2, "rows": 3, "auto_graph": true }))?;
        let gid = win.app().document().graph_sheets.first().ok_or("missing auto graph")?.id.clone();
        win.execute_and_journal("sheet.select", json!({ "sheet_id": gid.as_str() }))?;
    }

    let menu_out = rt.dispatch(DesktopHostEvent::MenuAction { window_id: win_id, command_id: String::from("ui.clipboard.copyForSlides") })?;
    assert!(menu_out.handled && menu_out.action == "clipboard.copied_slide_bundle" && rt.clipboard_text().is_none());
    let res = menu_out.command_result.as_ref().ok_or("missing command_result")?;
    assert_eq!(res.get("dpi").and_then(serde_json::Value::as_u64), Some(300));
    assert_eq!(res.get("omits_plain_text_svg").and_then(serde_json::Value::as_bool), Some(true));
    assert_eq!(res.get("profile").and_then(serde_json::Value::as_str), Some("slides_and_documents"));

    let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, win_id, None));
    h.settle(1);
    let _ = h.key_down(egui::Key::C, egui::Modifiers { command: true, shift: true, ..Default::default() });

    let slides = clip.written_slide_bundles.read().map_err(|_| "clip lock")?;
    assert_eq!(slides.len(), 2);
    assert_eq!(slides[0].dpi, 300);
    assert!(slides[0].tiff_bytes.as_deref().is_some_and(|b| b.starts_with(b"II*\0")));
    assert!(slides[0].html.contains("<img src=\"data:image/png;base64,"));
    assert!(clip.written_bundles.read().map_err(|_| "clip lock")?.is_empty());
    Ok(())
}
