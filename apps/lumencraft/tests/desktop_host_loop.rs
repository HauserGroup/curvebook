#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use lumencraft::desktop_host::{
    CloseDisposition, DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, MenuChromeMode, current_epoch_ms,
    default_recovery_ttl_ms,
};
use lumencraft_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphKind, GraphSheet, InfoConstant, InfoSheet,
    ProjectDocument, SheetFolderKind, SubcolumnMode, TableArchetype, XyTable,
};
use lumencraft_engine::Session;
use lumencraft_format::save_project_file;
use lumencraft_platform::process::ProcessCommand;
use lumencraft_ui_egui::compositor::WorkspacePixelRect;
use lumencraft_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use serde_json::json;

fn temp_test_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn rect_center_logical(rect: WorkspacePixelRect, metrics: ViewportMetrics) -> [f64; 2] {
    let px = f64::from(rect.x) + f64::from(rect.width) * 0.5;
    let py = f64::from(rect.y) + f64::from(rect.height) * 0.5;
    metrics.physical_to_logical_px([px, py])
}

fn build_desktop_test_project() -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5)?;
    xy.data_sets[0].title = String::from("Signal");
    let mut ds_xy = DataSheet::new("data-xy", "Dose Response", TableArchetype::Xy(xy))?;
    ds_xy.set_x_or_time_cell(0, 0, Cell::number(1.0))?;
    ds_xy.set_cell(CellCoord::new(0, 0, 0), Cell::number(12.5))?;
    ds_xy.set_x_or_time_cell(1, 0, Cell::number(3.0))?;
    ds_xy.set_cell(CellCoord::new(1, 0, 0), Cell::number(38.0))?;
    doc.add_data_sheet(ds_xy)?;

    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    let mut ds_col = DataSheet::new("data-col", "Second Table", TableArchetype::Column(col))?;
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
    let mut in_window_rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: false,
        locale_override: Some(String::from("de")),
        theme_override: Some(String::from("dark")),
        recovery_root: None,
        initial_viewport: ViewportMetrics::new(1200.0, 760.0, 1.0),
    });
    let w_in = in_window_rt.open_window(None, ViewportMetrics::new(1200.0, 760.0, 1.0))?;

    let chrome_in = in_window_rt.window(w_in)?.chrome_state();
    assert_eq!(chrome_in.mode, MenuChromeMode::InWindowMenuBar);
    assert!(chrome_in.in_window_menus);
    assert!(chrome_in.menu_bar_visible);
    assert!(chrome_in.menu_bar_height_pt > 0.0);
    assert!(
        chrome_in
            .sections
            .iter()
            .flat_map(|s| s.items.iter())
            .any(|item| item.command_id == "project.save" && item.localized_label == "Projekt speichern")
    );

    let initial_nav = in_window_rt.window(w_in)?.app().nav.navigator_open;
    let nav_out = in_window_rt.dispatch(DesktopHostEvent::MenuAction { window_id: w_in, command_id: String::from("ui.navigator.toggle") })?;
    assert!(nav_out.handled);
    assert_eq!(in_window_rt.window(w_in)?.app().nav.navigator_open, !initial_nav);

    let ruler_out = in_window_rt.dispatch(DesktopHostEvent::MenuAction { window_id: w_in, command_id: String::from("ui.rulers.toggle") })?;
    assert!(ruler_out.handled);
    assert!(in_window_rt.window(w_in)?.app().canvas_dialogs.rulers_visible);

    let palette_out = in_window_rt.dispatch(DesktopHostEvent::MenuAction { window_id: w_in, command_id: String::from("ui.palette.open") })?;
    assert!(palette_out.handled);

    let dispatched = &in_window_rt.window(w_in)?.chrome_state().dispatched_actions;
    assert_eq!(dispatched, &["ui.navigator.toggle", "ui.rulers.toggle", "ui.palette.open"]);

    let mut native_rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: false,
        new_instance: false,
        gui_only: false,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: ViewportMetrics::new(1200.0, 760.0, 1.0),
    });
    let w_native = native_rt.open_window(None, ViewportMetrics::new(1200.0, 760.0, 1.0))?;
    let chrome_native = native_rt.window(w_native)?.chrome_state();
    assert_eq!(chrome_native.mode, MenuChromeMode::NativeMenuBar);
    assert!(!chrome_native.in_window_menus);
    assert!(!chrome_native.menu_bar_visible);
    assert!((chrome_native.menu_bar_height_pt - 0.0).abs() < f64::EPSILON);

    let about_out = native_rt.dispatch(DesktopHostEvent::MenuAction { window_id: w_native, command_id: String::from("app.about") })?;
    assert!(about_out.handled);
    assert_eq!(about_out.command_result.as_ref().and_then(|v| v.get("dialog")).and_then(|v| v.as_str()), Some("About"));
    Ok(())
}

#[test]
fn new_instance_isolated_recovery_journals_and_prompt_unsaved_close_guard() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_test_dir("lc-w13-new-instance")?;
    let recovery_root = dir.join("recovery-root");

    let cfg = DesktopHostConfig {
        in_window_menus: true,
        new_instance: true,
        gui_only: false,
        locale_override: None,
        theme_override: None,
        recovery_root: Some(recovery_root.clone()),
        initial_viewport: ViewportMetrics::new(1024.0, 640.0, 1.0),
    };
    let mut rt = DesktopHostRuntime::new(cfg.clone());

    let open_1 = rt.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport: ViewportMetrics::new(1024.0, 640.0, 1.0) })?;
    let w1 = open_1.opened_window_id.ok_or("missing window 1")?;

    let open_2 = rt.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport: ViewportMetrics::new(1024.0, 640.0, 1.0) })?;
    let w2 = open_2.opened_window_id.ok_or("missing window 2")?;
    assert_eq!(rt.focused_window_id(), Some(w2));

    let jdir_1 = rt.window(w1)?.recovery_dir().ok_or("missing recovery dir 1")?.to_path_buf();
    let jdir_2 = rt.window(w2)?.recovery_dir().ok_or("missing recovery dir 2")?.to_path_buf();
    assert_ne!(jdir_1, jdir_2);
    assert_eq!(jdir_1.file_name().and_then(|n| n.to_str()), Some(format!("instance-{}-win-{}", std::process::id(), w1.0).as_str()));
    assert_eq!(jdir_2.file_name().and_then(|n| n.to_str()), Some(format!("instance-{}-win-{}", std::process::id(), w2.0).as_str()));

    // Concurrent second runtime sharing recovery_root must not clobber Window 1's active lock.
    let mut concurrent_rt = DesktopHostRuntime::new(cfg);
    let w_concurrent = concurrent_rt.open_window(None, ViewportMetrics::new(800.0, 600.0, 1.0))?;
    let jdir_c = concurrent_rt.window(w_concurrent)?.recovery_dir().ok_or("missing concurrent recovery dir")?.to_path_buf();
    assert_ne!(jdir_c, jdir_1);
    assert_ne!(jdir_c, jdir_2);
    concurrent_rt.close_all_force()?;

    // Mutate Window 1 -> PromptUnsaved on close unless force: true.
    let _ =
        rt.window_mut(w1)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "wal-ds", "title": "Isolated WAL Table", "kind": "xy" }))?;
    assert!(rt.window(w1)?.has_unsaved_changes());

    let prompt_close = rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: w1, force: false })?;
    assert_eq!(
        prompt_close.close_disposition,
        Some(CloseDisposition::PromptUnsaved { window_id: w1, dirty_commands: vec![String::from("sheet.createData")] })
    );
    assert_eq!(rt.window_count(), 2);

    // Clean close on Window 2 (still clean) succeeds immediately without force.
    let clean_close_w2 = rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: w2, force: false })?;
    assert_eq!(clean_close_w2.close_disposition, Some(CloseDisposition::Closed));
    assert_eq!(rt.window_count(), 1);
    assert_eq!(rt.focused_window_id(), Some(w1));

    // Drop runtime without force-closing Window 1 to simulate sudden crash, then recover WAL.
    drop(rt);
    let mut recovery_rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, ..DesktopHostConfig::default() });
    let recovered_w = recovery_rt.recover_window_from_dir(
        jdir_1,
        ViewportMetrics::new(1024.0, 640.0, 1.0),
        current_epoch_ms().saturating_add(default_recovery_ttl_ms()).saturating_add(100),
    )?;
    assert!(recovery_rt.window(recovered_w)?.recovered_from_crash());
    assert!(recovery_rt.window(recovered_w)?.app().document().data_sheets.iter().any(|s| s.id.as_str() == "wal-ds"));

    // Recovered dirty commands still trigger PromptUnsaved until saved or forced.
    let prompt_recovered = recovery_rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: recovered_w, force: false })?;
    assert!(matches!(prompt_recovered.close_disposition, Some(CloseDisposition::PromptUnsaved { .. })));
    let force_closed = recovery_rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: recovered_w, force: true })?;
    assert_eq!(force_closed.close_disposition, Some(CloseDisposition::Closed));
    assert_eq!(recovery_rt.window_count(), 0);

    let _ = fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn live_retina_dpi_transitions_resize_rgba_framebuffer() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt =
        DesktopHostRuntime::new(DesktopHostConfig { initial_viewport: ViewportMetrics::new(720.0, 480.0, 1.0), ..DesktopHostConfig::default() });
    let win = rt.open_window_with_session(Session::from_document(build_desktop_test_project()?), ViewportMetrics::new(720.0, 480.0, 1.0), false)?;

    for scale in [1.0_f32, 2.0_f32, 3.0_f32] {
        let scale_out = rt.dispatch(DesktopHostEvent::ScaleFactorChanged { window_id: win, scale_factor: scale })?;
        assert!(scale_out.handled);

        let redraw = rt.dispatch(DesktopHostEvent::RedrawRequested { window_id: win })?;
        let frame = redraw.frame.ok_or("expected frame output")?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let expected_w = (720.0 * f64::from(scale)).round() as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let expected_h = (480.0 * f64::from(scale)).round() as u32;

        assert_eq!(frame.window_id, win);
        assert_eq!(frame.width_px, expected_w);
        assert_eq!(frame.height_px, expected_h);
        assert!((frame.scale_factor - scale).abs() < 1e-6);
        assert_eq!(frame.rgba_bytes.len(), (expected_w * expected_h * 4) as usize);
        assert!(frame.frame_ms >= 0.0);
        assert!(frame.window_title.contains("LumenCraft"));
    }

    // Resized adjusts physical dimensions cleanly while preserving current 3.0x scale.
    let resize_out = rt.dispatch(DesktopHostEvent::Resized { window_id: win, width_px: 1920, height_px: 1080 })?;
    assert!(resize_out.handled);
    let redraw_resized = rt.dispatch(DesktopHostEvent::RedrawRequested { window_id: win })?;
    let frame_resized = redraw_resized.frame.ok_or("expected resized frame")?;
    assert_eq!(frame_resized.width_px, 1920);
    assert_eq!(frame_resized.height_px, 1080);
    assert!((frame_resized.scale_factor - 3.0).abs() < 1e-6);
    assert_eq!(frame_resized.rgba_bytes.len(), 1920 * 1080 * 4);
    Ok(())
}

#[test]
fn navigator_drag_reorder_split_divider_drag_and_ime_unicode_commits() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1280.0, 800.0, 2.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { initial_viewport: metrics, ..DesktopHostConfig::default() });
    let win = rt.open_window_with_session(Session::from_document(build_desktop_test_project()?), metrics, false)?;

    // 1. Navigator pointer drag-reorder (`data-xy` onto `data-col`).
    let nav_map = rt.window(win)?.app().layout_map(metrics);
    let ds0 = nav_map
        .navigator_entries
        .iter()
        .find(|e| e.folder == SheetFolderKind::Data && e.index_in_folder == 0)
        .ok_or("missing first data nav entry")?;
    let ds1 = nav_map
        .navigator_entries
        .iter()
        .find(|e| e.folder == SheetFolderKind::Data && e.index_in_folder == 1)
        .ok_or("missing second data nav entry")?;
    let start_pt = rect_center_logical(ds0.rect, metrics);
    let end_pt = rect_center_logical(ds1.rect, metrics);

    rt.dispatch(DesktopHostEvent::PointerButton {
        window_id: win,
        x_pt: start_pt[0],
        y_pt: start_pt[1],
        button: PointerButton::Primary,
        pressed: true,
        click_count: 1,
        modifiers: KeyModifiers::default(),
    })?;
    rt.dispatch(DesktopHostEvent::PointerMoved { window_id: win, x_pt: end_pt[0], y_pt: end_pt[1], modifiers: KeyModifiers::default() })?;
    let drop_out = rt.dispatch(DesktopHostEvent::PointerButton {
        window_id: win,
        x_pt: end_pt[0],
        y_pt: end_pt[1],
        button: PointerButton::Primary,
        pressed: false,
        click_count: 1,
        modifiers: KeyModifiers::default(),
    })?;
    assert_eq!(drop_out.command_id.as_deref(), Some("sheet.reorder"));
    assert_eq!(rt.window(win)?.app().document().data_sheets[0].id.as_str(), "data-col");

    // 2. Enable split pane and drag split divider via desktop pointer events.
    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "data-xy" }))?;
    rt.dispatch(DesktopHostEvent::MenuAction { window_id: win, command_id: String::from("ui.split.toggle") })?;
    assert!(rt.window(win)?.app().split.enabled);
    let initial_split_ratio = rt.window(win)?.app().split.ratio;
    let split_map = rt.window(win)?.app().layout_map(metrics);
    let divider_rect = split_map.split_divider.ok_or("missing split divider rect")?;
    let div_pt = rect_center_logical(divider_rect, metrics);

    rt.dispatch(DesktopHostEvent::PointerButton {
        window_id: win,
        x_pt: div_pt[0],
        y_pt: div_pt[1],
        button: PointerButton::Primary,
        pressed: true,
        click_count: 1,
        modifiers: KeyModifiers::default(),
    })?;
    rt.dispatch(DesktopHostEvent::PointerMoved { window_id: win, x_pt: div_pt[0] + 64.0, y_pt: div_pt[1], modifiers: KeyModifiers::default() })?;
    let div_end = rt.dispatch(DesktopHostEvent::PointerButton {
        window_id: win,
        x_pt: div_pt[0] + 64.0,
        y_pt: div_pt[1],
        button: PointerButton::Primary,
        pressed: false,
        click_count: 1,
        modifiers: KeyModifiers::default(),
    })?;
    assert_eq!(div_end.action, "split_divider.drag_end");
    assert!((rt.window(win)?.app().split.ratio - initial_split_ratio).abs() > 0.01);
    rt.dispatch(DesktopHostEvent::MenuAction { window_id: win, command_id: String::from("ui.split.toggle") })?;

    // 3. Inline cell IME preedit + commit (`"濃度 μmol/L"`).
    let grid_map = rt.window(win)?.app().layout_map(metrics);
    let cell_rect = grid_map.grid_geometry.as_ref().and_then(|g| g.cell_rect(2, 0)).ok_or("missing empty cell (2, 0)")?;
    let cell_pt = rect_center_logical(cell_rect, metrics);
    rt.dispatch(DesktopHostEvent::PointerButton {
        window_id: win,
        x_pt: cell_pt[0],
        y_pt: cell_pt[1],
        button: PointerButton::Primary,
        pressed: true,
        click_count: 2,
        modifiers: KeyModifiers::default(),
    })?;
    assert!(rt.window(win)?.app().inline_editor.active);

    rt.dispatch(DesktopHostEvent::ImePreedit { window_id: win, text: String::from("濃"), cursor: Some((0, 3)) })?;
    assert_eq!(rt.window(win)?.ime_preedit().map(|p| p.text.as_str()), Some("濃"));
    assert_eq!(rt.window(win)?.app().inline_editor.buffer, "");

    rt.dispatch(DesktopHostEvent::ImeCommit { window_id: win, text: String::from("濃度 μmol/L") })?;
    assert!(rt.window(win)?.ime_preedit().is_none());
    assert_eq!(rt.window(win)?.app().inline_editor.buffer, "濃度 μmol/L");

    let commit_cell = rt.dispatch(DesktopHostEvent::KeyboardInput {
        window_id: win,
        key: String::from("Enter"),
        pressed: true,
        modifiers: KeyModifiers::default(),
        text: None,
    })?;
    assert_eq!(commit_cell.command_id.as_deref(), Some("table.setCell"));
    let stored_cell = rt
        .window(win)?
        .app()
        .document()
        .data_sheet(&"data-xy".into())
        .and_then(|ds| ds.cell(CellCoord::new(2, 0, 0)))
        .ok_or("missing committed cell")?;
    assert_eq!(stored_cell.value, CellValue::Text(String::from("濃度 μmol/L")));

    // 4. InfoSheet notes IME commit (`"العربية"`).
    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "info-1" }))?;
    let info_map = rt.window(win)?.app().layout_map(metrics);
    let notes_rect = info_map.info_notes_rect.ok_or("missing info notes rect")?;
    let notes_pt = rect_center_logical(notes_rect, metrics);
    rt.dispatch(DesktopHostEvent::PointerButton {
        window_id: win,
        x_pt: notes_pt[0],
        y_pt: notes_pt[1],
        button: PointerButton::Primary,
        pressed: true,
        click_count: 1,
        modifiers: KeyModifiers::default(),
    })?;
    rt.dispatch(DesktopHostEvent::ImePreedit { window_id: win, text: String::from("ال"), cursor: Some((0, 4)) })?;
    let _ = rt.dispatch(DesktopHostEvent::RedrawRequested { window_id: win })?;
    rt.dispatch(DesktopHostEvent::ImeCommit { window_id: win, text: String::from("العربية") })?;
    let commit_info = rt.dispatch(DesktopHostEvent::KeyboardInput {
        window_id: win,
        key: String::from("Enter"),
        pressed: true,
        modifiers: KeyModifiers::default(),
        text: None,
    })?;
    assert_eq!(commit_info.command_id.as_deref(), Some("info.setNote"));
    assert_eq!(rt.window(win)?.app().document().info_sheet(&"info-1".into()).map(|i| i.notes_markdown.as_str()), Some("العربية"));

    // 5. Canvas title IME commit (`"濃度 μmol/L"` + `" — العربية"`).
    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "graph-xy" }))?;
    let graph_map = rt.window(win)?.app().layout_map(metrics);
    let title_rect = graph_map.canvas_title_rect.ok_or("missing canvas title rect")?;
    let title_pt = rect_center_logical(title_rect, metrics);
    rt.dispatch(DesktopHostEvent::PointerButton {
        window_id: win,
        x_pt: title_pt[0],
        y_pt: title_pt[1],
        button: PointerButton::Primary,
        pressed: true,
        click_count: 2,
        modifiers: KeyModifiers::default(),
    })?;
    rt.dispatch(DesktopHostEvent::ImeCommit { window_id: win, text: String::from("濃度 μmol/L — العربية") })?;
    let commit_title = rt.dispatch(DesktopHostEvent::KeyboardInput {
        window_id: win,
        key: String::from("Enter"),
        pressed: true,
        modifiers: KeyModifiers::default(),
        text: None,
    })?;
    assert_eq!(commit_title.command_id.as_deref(), Some("graph.setSpec"));
    assert_eq!(rt.window(win)?.app().document().graph_sheet(&"graph-xy".into()).map(|g| g.spec.title.as_str()), Some("濃度 μmol/L — العربية"));
    Ok(())
}

#[test]
fn trackpad_magnify_pinch_zoom_inertial_coasting_and_clipboard() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1200.0, 800.0, 2.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { initial_viewport: metrics, ..DesktopHostConfig::default() });
    let win = rt.open_window_with_session(Session::from_document(build_desktop_test_project()?), metrics, false)?;

    // TrackpadMagnify + TrackpadScroll + AnimationTick coasting on graph-xy.
    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "graph-xy" }))?;
    let gmap = rt.window(win)?.app().layout_map(metrics);
    let paper = gmap.scene_paper.as_ref().ok_or("missing scene paper")?;
    let center = rect_center_logical(paper.paper, metrics);

    let zoom_before = rt.window(win)?.app().canvas_dialogs.pan_zoom.zoom;
    let mag_out =
        rt.dispatch(DesktopHostEvent::TrackpadMagnify { window_id: win, anchor_x_pt: center[0], anchor_y_pt: center[1], magnification_delta: 0.5 })?;
    assert!(mag_out.handled);
    let zoom_after = rt.window(win)?.app().canvas_dialogs.pan_zoom.zoom;
    assert!((zoom_after - zoom_before * 1.5).abs() < 1e-6);

    rt.dispatch(DesktopHostEvent::TrackpadScroll { window_id: win, delta_x: 30.0, delta_y: -15.0, dt_sec: 0.016, phase_ended: true })?;
    assert!(rt.window(win)?.is_coasting());
    let pan_after_scroll = rt.window(win)?.app().canvas_dialogs.pan_zoom.pan_pt;
    assert!(pan_after_scroll[0].abs() > 1.0);

    let tick_out = rt.dispatch(DesktopHostEvent::AnimationTick { dt_sec: 0.016 })?;
    assert!(tick_out.handled);
    let pan_after_coast = rt.window(win)?.app().canvas_dialogs.pan_zoom.pan_pt;
    assert!(pan_after_coast[0] > pan_after_scroll[0]);

    // ClipboardCopy + ClipboardPaste round-trip on data-xy -> data-col.
    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "data-xy" }))?;
    let copy_out = rt.dispatch(DesktopHostEvent::ClipboardCopy { window_id: win })?;
    assert!(copy_out.handled);
    let copied = rt.clipboard_text().ok_or("missing copied TSV")?;
    assert!(copied.contains("12.5") || copied.contains("38"));

    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "data-col" }))?;
    let paste_out = rt.dispatch(DesktopHostEvent::ClipboardPaste { window_id: win, text_override: Some(String::from("99.5\t101.25")) })?;
    assert!(paste_out.handled);
    assert_eq!(paste_out.command_id.as_deref(), Some("table.paste"));
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
fn file_hover_and_file_dropped_routing_csv_and_lumencraft() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_test_dir("lc-w13-filedrop")?;
    let csv_path = dir.join("measurements.csv");
    fs::write(&csv_path, "Dose,Response\n0.1,10.0\n1.0,45.0\n")?;

    let mut proj_doc = ProjectDocument::default();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2)?;
    let mut ds = DataSheet::new("drop-proj-ds", "Archived Project Table", TableArchetype::Xy(xy.clone()))?;
    ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(42.0))?;
    xy.data_sets[0].title = String::from("Y");
    proj_doc.add_data_sheet(ds)?;
    let archive_path = dir.join("experiment.lumencraft");
    save_project_file(&archive_path, &proj_doc)?;

    // Single-window mode (`new_instance: false`): drop .lumencraft into clean window loads in place.
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { new_instance: false, ..DesktopHostConfig::default() });
    let w1 = rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;

    rt.dispatch(DesktopHostEvent::FileHovered { window_id: w1, paths: vec![csv_path.clone()] })?;
    assert_eq!(rt.window(w1)?.hovered_files(), std::slice::from_ref(&csv_path));
    rt.dispatch(DesktopHostEvent::FileHoverCancelled { window_id: w1 })?;
    assert!(rt.window(w1)?.hovered_files().is_empty());

    let clean_drop = rt.dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: archive_path.clone() })?;
    assert!(clean_drop.opened_window_id.is_none());
    assert_eq!(rt.window_count(), 1);
    assert!(rt.window(w1)?.app().document().data_sheet(&"drop-proj-ds".into()).is_some());
    assert!(!rt.window(w1)?.has_unsaved_changes());

    // Dropping .csv imports directly as a new DataSheet into target window and marks window dirty.
    let csv_drop = rt.dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: csv_path })?;
    assert_eq!(csv_drop.command_id.as_deref(), Some("table.importCsv"));
    assert_eq!(rt.window_count(), 1);
    assert_eq!(rt.window(w1)?.app().document().data_sheets.len(), 2);
    assert!(rt.window(w1)?.has_unsaved_changes());

    // Dropping .lumencraft onto a dirty window must open a new DesktopWindowSession instead of discarding edits.
    let dirty_drop = rt.dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: archive_path.clone() })?;
    let w2 = dirty_drop.opened_window_id.ok_or("dirty window drop should open new window")?;
    assert_ne!(w2, w1);
    assert_eq!(rt.window_count(), 2);
    assert_eq!(rt.window(w1)?.app().document().data_sheets.len(), 2);

    // With new_instance: true, dropping .lumencraft onto even a clean window opens a new window.
    let mut multi_rt = DesktopHostRuntime::new(DesktopHostConfig { new_instance: true, ..DesktopHostConfig::default() });
    let mw1 = multi_rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
    let new_inst_drop = multi_rt.dispatch(DesktopHostEvent::FileDropped { window_id: mw1, path: archive_path })?;
    assert!(new_inst_drop.opened_window_id.is_some());
    assert_eq!(multi_rt.window_count(), 2);

    let _ = fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn jsonl_script_playback_and_cli_runtime_flags_integration() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_test_dir("lc-w13-cli")?;
    let portable_dir = dir.join("portable");
    let script_path = dir.join("script.jsonl");

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

    // In-process JSONL playback verification.
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let outcomes = rt.run_event_script_jsonl(&jsonl)?;
    assert_eq!(outcomes.len(), 4);
    let redrawn = outcomes[3].frame.as_ref().ok_or("missing redrawn frame")?;
    assert_eq!(redrawn.width_px, 1800);
    assert_eq!(redrawn.height_px, 1200);
    assert!(rt.window(DesktopWindowId(1))?.app().canvas_dialogs.rulers_visible);
    rt.close_all_force()?;

    // Real CLI binary smoke run with --in-window-menus, --new-instance, --headless-once, --portable-dir, --event-script.
    let bin = env!("CARGO_BIN_EXE_lumencraft");
    let out = ProcessCommand::new(bin)
        .arg("--in-window-menus")
        .arg("--new-instance")
        .arg("--headless-once")
        .arg("--portable-dir")
        .arg(portable_dir.to_string_lossy().as_ref())
        .arg("--event-script")
        .arg(script_path.to_string_lossy().as_ref())
        .run()?;
    assert!(out.success, "lumencraft CLI failed (status {:?}): {}", out.status_code, String::from_utf8_lossy(&out.stderr));

    let _ = fs::remove_dir_all(&dir);
    Ok(())
}
