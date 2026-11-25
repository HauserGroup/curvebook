#![forbid(unsafe_code)]

#[path = "common/mod.rs"]
mod common;

use std::fs;

use curvebook::desktop_host::{
    CloseDisposition, DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, MenuChromeMode, current_epoch_ms,
    default_recovery_ttl_ms,
};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphKind, GraphSheet, InfoConstant, InfoSheet,
    ProjectDocument, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use curvebook_platform::process::ProcessCommand;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::json;

use common::{NativeHarness, TempDirGuard, build_xy_doc, build_zip_entries, open_host_window};

fn build_desktop_test_project() -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = build_xy_doc("data-xy", "Dose Response", &[(1.0, 12.5), (3.0, 38.0), (0.0, 0.0)])?;
    if let Some(ds) = doc.data_sheet_mut(&"data-xy".into()) {
        ds.set_cell(CellCoord::new(2, 0, 0), Cell::empty())?;
    }
    let mut ds_col =
        DataSheet::new("data-col", "Second Table", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?))?;
    ds_col.set_cell(CellCoord::new(0, 0, 0), Cell::number(10.0))?;
    ds_col.set_cell(CellCoord::new(0, 1, 0), Cell::number(20.0))?;
    doc.add_data_sheet(ds_col)?;

    let mut info = InfoSheet::new_preseeded("info-1", "Protocol Info")?;
    info.constants.push(InfoConstant::new("Target_pH", ConstantValue::Number(7.4), Some(String::from("pH")))?);
    info.notes_markdown.clear();
    doc.add_info_sheet(info)?;

    let ds_ref = doc.data_sheet(&"data-xy".into()).ok_or("missing data-xy sheet")?;
    let mut graph = GraphSheet::new_uninitialized_for_data("graph-xy", ds_ref)?;
    graph.initialized = true;
    graph.title.clear();
    graph.spec.kind = GraphKind::Xy;
    graph.spec.title.clear();
    graph.spec.x_axis.title = String::from("Concentration");
    graph.spec.y_axis.title = String::from("Response");
    doc.add_graph_sheet(graph)?;

    let _ = doc.select_in_navigator(&"data-xy".into());
    Ok(doc)
}

#[test]
fn in_window_menus_chrome_and_menu_action_vs_native_mode() -> Result<(), Box<dyn std::error::Error>> {
    let vp = ViewportMetrics::new(1200.0, 760.0, 1.0);
    let mut in_window_rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        locale_override: Some(String::from("de")),
        theme_override: Some(String::from("dark")),
        initial_viewport: vp,
        ..DesktopHostConfig::default()
    });
    let w_in = in_window_rt.open_window(None, vp)?;

    let chrome_in = in_window_rt.window(w_in)?.chrome_state();
    assert!(
        chrome_in.mode == MenuChromeMode::InWindowMenuBar
            && chrome_in.in_window_menus
            && chrome_in.menu_bar_visible
            && chrome_in.menu_bar_height_pt > 0.0
    );
    assert!(
        chrome_in.sections.iter().flat_map(|s| s.items.iter()).any(|item| item.command_id == "project.save" && item.localized_label == "Speichern")
    );

    let initial_nav = in_window_rt.window(w_in)?.app().nav.navigator_open;
    for cmd in ["ui.navigator.toggle", "ui.rulers.toggle", "ui.palette.open"] {
        assert!(in_window_rt.dispatch(DesktopHostEvent::MenuAction { window_id: w_in, command_id: cmd.to_owned() })?.handled);
    }
    assert_eq!(in_window_rt.window(w_in)?.app().nav.navigator_open, !initial_nav);
    assert!(in_window_rt.window(w_in)?.app().canvas_dialogs.rulers_visible);
    assert_eq!(in_window_rt.window(w_in)?.chrome_state().dispatched_actions, ["ui.navigator.toggle", "ui.rulers.toggle", "ui.palette.open"]);

    let (mut native_rt, w_native) = open_host_window(false, false, None, 1200.0, 760.0)?;
    let chrome_native = native_rt.window(w_native)?.chrome_state();
    assert!(
        chrome_native.mode == MenuChromeMode::NativeMenuBar
            && !chrome_native.in_window_menus
            && !chrome_native.menu_bar_visible
            && chrome_native.menu_bar_height_pt.abs() < f64::EPSILON
    );

    let about_out = native_rt.dispatch(DesktopHostEvent::MenuAction { window_id: w_native, command_id: String::from("app.about") })?;
    assert!(about_out.handled && about_out.command_result.as_ref().and_then(|v| v.get("dialog")).and_then(|v| v.as_str()) == Some("About"));
    Ok(())
}

#[test]
fn new_instance_isolated_recovery_journals_and_prompt_unsaved_close_guard() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("cb-w13-new-instance")?;
    let vp = ViewportMetrics::new(1024.0, 640.0, 1.0);
    let cfg = DesktopHostConfig {
        in_window_menus: true,
        new_instance: true,
        recovery_root: Some(dir.child("recovery-root")),
        initial_viewport: vp,
        ..DesktopHostConfig::default()
    };
    let mut rt = DesktopHostRuntime::new(cfg.clone());

    let w1 = rt.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport: vp })?.opened_window_id.ok_or("missing w1")?;
    let w2 = rt.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport: vp })?.opened_window_id.ok_or("missing w2")?;
    assert_eq!(rt.focused_window_id(), Some(w2));

    let jdir_1 = rt.window(w1)?.recovery_dir().ok_or("missing jdir1")?.to_path_buf();
    let jdir_2 = rt.window(w2)?.recovery_dir().ok_or("missing jdir2")?.to_path_buf();
    assert_ne!(jdir_1, jdir_2);
    assert_eq!(jdir_1.file_name().and_then(|n| n.to_str()), Some(format!("instance-{}-win-{}", std::process::id(), w1.0).as_str()));
    assert_eq!(jdir_2.file_name().and_then(|n| n.to_str()), Some(format!("instance-{}-win-{}", std::process::id(), w2.0).as_str()));

    let mut concurrent_rt = DesktopHostRuntime::new(cfg);
    let wc = concurrent_rt.open_window(None, ViewportMetrics::new(800.0, 600.0, 1.0))?;
    let jdir_c = concurrent_rt.window(wc)?.recovery_dir().ok_or("missing jdir_c")?.to_path_buf();
    assert!(jdir_c != jdir_1 && jdir_c != jdir_2);
    concurrent_rt.close_all_force()?;

    let _ =
        rt.window_mut(w1)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "wal-ds", "title": "Isolated WAL Table", "kind": "xy" }))?;
    assert!(rt.window(w1)?.has_unsaved_changes());
    assert_eq!(
        rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: w1, force: false })?.close_disposition,
        Some(CloseDisposition::PromptUnsaved { window_id: w1, dirty_commands: vec![String::from("sheet.createData")] })
    );
    assert_eq!(
        rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: w2, force: false })?.close_disposition,
        Some(CloseDisposition::Closed)
    );
    assert_eq!((rt.window_count(), rt.focused_window_id()), (1, Some(w1)));

    drop(rt);
    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_w = rec_rt.recover_window_from_dir(jdir_1, vp, current_epoch_ms().saturating_add(default_recovery_ttl_ms()).saturating_add(100))?;
    assert!(rec_rt.window(rec_w)?.recovered_from_crash());
    assert!(rec_rt.window(rec_w)?.app().document().data_sheets.iter().any(|s| s.id.as_str() == "wal-ds"));
    assert!(matches!(
        rec_rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: rec_w, force: false })?.close_disposition,
        Some(CloseDisposition::PromptUnsaved { .. })
    ));
    assert_eq!(
        rec_rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: rec_w, force: true })?.close_disposition,
        Some(CloseDisposition::Closed)
    );
    assert_eq!(rec_rt.window_count(), 0);
    Ok(())
}

#[test]
fn live_retina_dpi_transitions_resize_rgba_framebuffer() -> Result<(), Box<dyn std::error::Error>> {
    let vp = ViewportMetrics::new(720.0, 480.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { initial_viewport: vp, ..DesktopHostConfig::default() });
    let win = rt.open_window_with_session(Session::from_document(build_desktop_test_project()?), vp, false)?;

    for scale in [1.0_f32, 2.0, 3.0] {
        assert!(rt.dispatch(DesktopHostEvent::ScaleFactorChanged { window_id: win, scale_factor: scale })?.handled);
        let frame = rt.dispatch(DesktopHostEvent::RedrawRequested { window_id: win })?.frame.ok_or("expected frame")?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (ew, eh) = ((720.0 * f64::from(scale)).round() as u32, (480.0 * f64::from(scale)).round() as u32);
        assert_eq!((frame.window_id, frame.width_px, frame.height_px, frame.rgba_bytes.len()), (win, ew, eh, (ew * eh * 4) as usize));
        assert!((frame.scale_factor - scale).abs() < 1e-6 && frame.frame_ms >= 0.0 && frame.window_title.contains("CurveBook"));
    }

    assert!(rt.dispatch(DesktopHostEvent::Resized { window_id: win, width_px: 1920, height_px: 1080 })?.handled);
    let fr = rt.dispatch(DesktopHostEvent::RedrawRequested { window_id: win })?.frame.ok_or("expected resized frame")?;
    assert_eq!((fr.width_px, fr.height_px, fr.rgba_bytes.len()), (1920, 1080, 1920 * 1080 * 4));
    assert!((fr.scale_factor - 3.0).abs() < 1e-6);
    Ok(())
}

#[test]
fn navigator_drag_reorder_split_divider_drag_and_ime_unicode_commits() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 2.0);
    let cfg = DesktopHostConfig { initial_viewport: metrics, ..DesktopHostConfig::default() };
    let (mut h, win) = NativeHarness::from_config(cfg, Some(Session::from_document(build_desktop_test_project()?)))?;
    h = h.with_scale(2.0);
    h.settle(2);

    // 1. Navigator pointer drag-reorder.
    let ds0 = h.app.inspect_meta().find_by_widget_id("nav.sheet.data-xy").ok_or("missing data-xy")?.clipped_rect.center();
    let ds1 = h.app.inspect_meta().find_by_widget_id("nav.sheet.data-col").ok_or("missing data-col")?.clipped_rect.center();
    h.drag_between(ds0, ds1);
    assert_eq!(h.app.runtime().window(win)?.app().document().data_sheets[0].id.as_str(), "data-col");

    // 2. Split pane divider drag.
    let _ = h.app.runtime_mut().window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "data-xy" }))?;
    h.app.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: win, command_id: String::from("ui.split.toggle") })?;
    h.settle(2);
    assert!(h.app.runtime().window(win)?.app().split.enabled);
    let ratio0 = h.app.runtime().window(win)?.app().split.ratio;
    let div = h.app.inspect_meta().find_by_widget_id("split.divider").ok_or("missing split.divider")?.clipped_rect.center();
    h.drag_between(div, div + egui::vec2(64.0, 0.0));
    assert!((h.app.runtime().window(win)?.app().split.ratio - ratio0).abs() > 0.01);
    h.app.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: win, command_id: String::from("ui.split.toggle") })?;
    h.settle(2);

    // 3. Inline cell IME preedit + commit.
    let cell_pt = h.app.inspect_meta().find_by_widget_id("grid.cell.2.0.0").ok_or("missing grid.cell.2.0.0")?.clipped_rect.center();
    h.double_click_pos(cell_pt);
    let _ = h.step_events(vec![
        egui::Event::Ime(egui::ImeEvent::Preedit { text: String::from("濃"), active_range_chars: Some(0..1) }),
        egui::Event::Ime(egui::ImeEvent::Commit(String::from("濃度 μmol/L"))),
        egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
    ]);
    let _ = h.step_events(Vec::new());
    let stored = h
        .app
        .runtime()
        .window(win)?
        .app()
        .document()
        .data_sheet(&"data-xy".into())
        .and_then(|ds| ds.cell(CellCoord::new(2, 0, 0)))
        .ok_or("missing cell")?;
    assert_eq!(stored.value, CellValue::Text(String::from("濃度 μmol/L")));

    // 4. InfoSheet notes IME commit.
    let _ = h.app.runtime_mut().window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "info-1" }))?;
    h.settle(2);
    let _ = h.click_widget("info.notes_editor")?;
    let _ = h.step_events(vec![egui::Event::Ime(egui::ImeEvent::Commit(String::from("العربية")))]);
    let _ = h.click_widget("info.notes.save")?;
    assert_eq!(h.app.runtime().window(win)?.app().document().info_sheet(&"info-1".into()).map(|i| i.notes_markdown.as_str()), Some("العربية"));

    // 5. Navigator sheet title rename IME commit.
    let graph_nav_pt = h.app.inspect_meta().find_by_widget_id("nav.sheet.graph-xy").ok_or("missing nav.sheet.graph-xy")?.clipped_rect.center();
    h.double_click_pos(graph_nav_pt);
    if h.app.inspect_meta().find_by_widget_id("nav.rename_input").is_none() {
        let _ = h.key_down(egui::Key::F2, egui::Modifiers::NONE);
    }
    assert!(h.app.inspect_meta().find_by_widget_id("nav.rename_input").is_some());
    let _ = h.step_events(vec![
        egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND },
        egui::Event::Ime(egui::ImeEvent::Commit(String::from("濃度 μmol/L — العربية"))),
        egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
    ]);
    let _ = h.step_events(Vec::new());
    assert_eq!(
        h.app.runtime().window(win)?.app().document().graph_sheet(&"graph-xy".into()).map(|g| g.title.as_str()),
        Some("濃度 μmol/L — العربية")
    );
    Ok(())
}

#[test]
fn trackpad_magnify_pinch_zoom_inertial_coasting_and_clipboard() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1200.0, 800.0, 2.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { initial_viewport: metrics, ..DesktopHostConfig::default() });
    let win = rt.open_window_with_session(Session::from_document(build_desktop_test_project()?), metrics, false)?;
    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "graph-xy" }))?;

    let mut h = NativeHarness::with_screen(CurveBookNativeApp::new(rt, win, None), egui::vec2(1200.0, 800.0)).with_scale(2.0);
    h.settle(2);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let center = h.app.inspect_meta().find_by_widget_id("canvas.viewport").ok_or("missing canvas.viewport")?.clipped_rect.intersect(screen).center();

    let zoom0 = h.app.runtime().window(win)?.app().canvas_dialogs.pan_zoom.zoom;
    let _ = h.step_events(vec![egui::Event::PointerMoved(center), egui::Event::Zoom(1.5)]);
    let _ = h.step_events(Vec::new());
    assert!((h.app.runtime().window(win)?.app().canvas_dialogs.pan_zoom.zoom - zoom0 * 1.5).abs() < 1e-4);

    let pan0 = h.app.runtime().window(win)?.app().canvas_dialogs.pan_zoom.pan_pt;
    h.send_wheel(center, egui::vec2(30.0, -15.0), egui::Modifiers::NONE);
    assert_ne!(h.app.runtime().window(win)?.app().canvas_dialogs.pan_zoom.pan_pt, pan0);

    let rt = h.app.runtime_mut();
    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "data-xy" }))?;
    assert!(rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: win })?.handled);
    let copied = rt.clipboard_text().ok_or("missing copied TSV")?;
    assert!(copied.contains("12.5") || copied.contains("38"));

    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "data-col" }))?;
    let paste_out = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: win, text_override: Some(String::from("99.5\t101.25")) })?;
    assert!(paste_out.handled && paste_out.command_id.as_deref() == Some("table.paste"));
    let pasted = rt
        .window(win)?
        .app()
        .document()
        .data_sheet(&"data-col".into())
        .and_then(|ds| ds.cell(CellCoord::new(0, 0, 0)))
        .ok_or("missing pasted cell")?;
    assert_eq!(pasted.value, CellValue::Number(99.5));
    Ok(())
}

#[test]
fn file_hover_and_file_dropped_routing_csv_and_curvebook() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("cb-w13-filedrop")?;
    let csv_path = dir.child("measurements.csv");
    fs::write(&csv_path, "Dose,Response\n0.1,10.0\n1.0,45.0\n")?;
    let archive_path = dir.child("experiment.curvebook");
    save_project_file(&archive_path, &build_xy_doc("drop-proj-ds", "Archived Project Table", &[(0.0, 42.0)])?)?;

    let (mut rt, w1) = open_host_window(false, false, None, 1024.0, 640.0)?;
    rt.dispatch(DesktopHostEvent::FileHovered { window_id: w1, paths: vec![csv_path.clone()] })?;
    assert_eq!(rt.window(w1)?.hovered_files(), std::slice::from_ref(&csv_path));
    rt.dispatch(DesktopHostEvent::FileHoverCancelled { window_id: w1 })?;
    assert!(rt.window(w1)?.hovered_files().is_empty());

    let clean_drop = rt.dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: archive_path.clone() })?;
    assert!(clean_drop.opened_window_id.is_none() && rt.window_count() == 1);
    assert!(rt.window(w1)?.app().document().data_sheet(&"drop-proj-ds".into()).is_some() && !rt.window(w1)?.has_unsaved_changes());

    let csv_drop = rt.dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: csv_path })?;
    assert_eq!(csv_drop.command_id.as_deref(), Some("table.importCsv"));
    assert!(rt.window_count() == 1 && rt.window(w1)?.app().document().data_sheets.len() == 2 && rt.window(w1)?.has_unsaved_changes());

    let dirty_drop = rt.dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: archive_path.clone() })?;
    assert!(dirty_drop.opened_window_id.is_some_and(|w2| w2 != w1) && rt.window_count() == 2);

    let (mut multi_rt, mw1) = open_host_window(false, true, None, 1024.0, 640.0)?;
    assert!(
        multi_rt.dispatch(DesktopHostEvent::FileDropped { window_id: mw1, path: archive_path })?.opened_window_id.is_some()
            && multi_rt.window_count() == 2
    );
    Ok(())
}

#[test]
fn jsonl_script_playback_and_cli_runtime_flags_integration() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("cb-w13-cli")?;
    let (portable_dir, script_path) = (dir.child("portable"), dir.child("script.jsonl"));
    let events = [
        DesktopHostEvent::WindowOpened { initial_file: None, viewport: ViewportMetrics::new(900.0, 600.0, 1.0) },
        DesktopHostEvent::ScaleFactorChanged { window_id: DesktopWindowId(1), scale_factor: 2.0 },
        DesktopHostEvent::MenuAction { window_id: DesktopWindowId(1), command_id: String::from("ui.rulers.toggle") },
        DesktopHostEvent::RedrawRequested { window_id: DesktopWindowId(1) },
    ];
    let mut jsonl = String::from("# Deterministic R8.3 script\n");
    for ev in &events {
        jsonl.push_str(&serde_json::to_string(ev)?);
        jsonl.push('\n');
    }
    fs::write(&script_path, &jsonl)?;

    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let outcomes = rt.run_event_script_jsonl(&jsonl)?;
    assert_eq!(outcomes.len(), 4);
    let redrawn = outcomes[3].frame.as_ref().ok_or("missing redrawn frame")?;
    assert_eq!((redrawn.width_px, redrawn.height_px), (1800, 1200));
    assert!(rt.window(DesktopWindowId(1))?.app().canvas_dialogs.rulers_visible);
    rt.close_all_force()?;

    let out = ProcessCommand::new(env!("CARGO_BIN_EXE_curvebook"))
        .arg("--in-window-menus")
        .arg("--new-instance")
        .arg("--headless-once")
        .arg("--portable-dir")
        .arg(portable_dir.to_string_lossy().as_ref())
        .arg("--event-script")
        .arg(script_path.to_string_lossy().as_ref())
        .run()?;
    assert!(out.success, "curvebook CLI failed (status {:?}): {}", out.status_code, String::from_utf8_lossy(&out.stderr));
    Ok(())
}

#[test]
fn file_dropped_pzfx_and_zip_archive_open_cleanly() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("curvebook-dnd-pzfx-zip")?;
    let pzfx_path = dir.child("dropped.pzfx");
    fs::write(
        &pzfx_path,
        r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile SchemaVersion="2"><Title>Dropped PZFX Study</Title><TableSequence><Table ID="pz-1" TableType="OneWay"><Title>Dropped XML Columns</Title><YColumn><Title>Baseline</Title><Subcolumn><d>3.5</d><d>4.5</d></Subcolumn></YColumn></Table></TableSequence></PrismFile>"#,
    )?;
    let archive_path = dir.child("dropped.prism");
    let manifest = serde_json::to_string(
        &json!({ "title": "Dropped Archive Container", "sheets": [{ "id": "pr-1", "title": "Dropped Archive Trace", "kind": "XY", "subcolumn_mode": "Single", "csv_path": "data/trace.csv" }] }),
    )?;
    fs::write(&archive_path, build_zip_entries(&[("manifest.json", &manifest), ("data/trace.csv", "Time,Signal\n0,1.25\n10,9.75\n")])?)?;

    let (mut rt, wid) = open_host_window(true, false, None, 1100.0, 720.0)?;
    let pzfx_out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: pzfx_path })?;
    let pzfx_win = pzfx_out.opened_window_id.unwrap_or(wid);
    assert!(pzfx_out.handled && rt.window(pzfx_win)?.app().session().file_path().is_none());
    let pzfx_ds = rt.window(pzfx_win)?.app().document().data_sheets.iter().find(|d| d.title == "Dropped XML Columns").ok_or("missing .pzfx sheet")?;
    assert_eq!(pzfx_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(3.5));

    let empty_wid = rt.open_window(None, ViewportMetrics::new(1100.0, 720.0, 1.0))?;
    let zip_out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: empty_wid, path: archive_path })?;
    let zip_win = zip_out.opened_window_id.unwrap_or(empty_wid);
    assert!(zip_out.handled && rt.window(zip_win)?.app().session().file_path().is_none());
    let zip_ds =
        rt.window(zip_win)?.app().document().data_sheets.iter().find(|d| d.title == "Dropped Archive Trace").ok_or("missing .prism sheet")?;
    assert!(matches!(zip_ds.archetype, TableArchetype::Xy(_)));
    assert_eq!(zip_ds.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(9.75));
    Ok(())
}
