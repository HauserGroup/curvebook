#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook::desktop_host::{
    CloseDisposition, DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, MenuChromeMode, current_epoch_ms,
    default_recovery_ttl_ms,
};
use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
use curvebook_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphKind, GraphSheet, InfoConstant, InfoSheet,
    ProjectDocument, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use curvebook_platform::process::ProcessCommand;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::json;

fn temp_test_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn build_desktop_test_project() -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::default();

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5)?;
    if let Some(ds0) = xy.data_sets.first_mut() {
        ds0.title = String::from("Signal");
    }
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
        ..DesktopHostConfig::default()
    });
    let w_in = in_window_rt.open_window(None, ViewportMetrics::new(1200.0, 760.0, 1.0))?;

    let chrome_in = in_window_rt.window(w_in)?.chrome_state();
    assert_eq!(chrome_in.mode, MenuChromeMode::InWindowMenuBar);
    assert!(chrome_in.in_window_menus);
    assert!(chrome_in.menu_bar_visible);
    assert!(chrome_in.menu_bar_height_pt > 0.0);
    assert!(
        chrome_in.sections.iter().flat_map(|s| s.items.iter()).any(|item| item.command_id == "project.save" && item.localized_label == "Speichern")
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
        ..DesktopHostConfig::default()
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
    let dir = temp_test_dir("cb-w13-new-instance")?;
    let recovery_root = dir.join("recovery-root");

    let cfg = DesktopHostConfig {
        in_window_menus: true,
        new_instance: true,
        gui_only: false,
        locale_override: None,
        theme_override: None,
        recovery_root: Some(recovery_root.clone()),
        initial_viewport: ViewportMetrics::new(1024.0, 640.0, 1.0),
        ..DesktopHostConfig::default()
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
        assert!(frame.window_title.contains("CurveBook"));
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

    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    ctx.set_pixels_per_point(2.0);
    let mut native = CurveBookNativeApp::new(rt, win, None);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let step = |app_host: &mut CurveBookNativeApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| app_host.step_ui(ui));
        out.textures_delta.clear();
    };
    let settle = |app_host: &mut CurveBookNativeApp| {
        step(app_host, Vec::new());
        step(app_host, Vec::new());
    };
    let drag_between = |app_host: &mut CurveBookNativeApp, from: egui::Pos2, to: egui::Pos2| {
        let mid = from.lerp(to, 0.5);
        step(
            app_host,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            ],
        );
        step(app_host, vec![egui::Event::PointerMoved(mid)]);
        step(app_host, vec![egui::Event::PointerMoved(to)]);
        step(
            app_host,
            vec![egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
        );
        step(app_host, Vec::new());
    };
    settle(&mut native);

    // 1. Navigator pointer drag-reorder (`data-xy` onto `data-col`).
    let ds0_center = native.inspect_meta().find_by_widget_id("nav.sheet.data-xy").ok_or("missing nav.sheet.data-xy")?.clipped_rect.center();
    let ds1_center = native.inspect_meta().find_by_widget_id("nav.sheet.data-col").ok_or("missing nav.sheet.data-col")?.clipped_rect.center();
    drag_between(&mut native, ds0_center, ds1_center);
    assert_eq!(native.runtime().window(win)?.app().document().data_sheets[0].id.as_str(), "data-col");

    // 2. Enable split pane and drag split divider via egui pointer events.
    let _ = native.runtime_mut().window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "data-xy" }))?;
    native.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: win, command_id: String::from("ui.split.toggle") })?;
    settle(&mut native);
    assert!(native.runtime().window(win)?.app().split.enabled);
    let initial_split_ratio = native.runtime().window(win)?.app().split.ratio;
    let div_center = native.inspect_meta().find_by_widget_id("split.divider").ok_or("missing split.divider")?.clipped_rect.center();
    drag_between(&mut native, div_center, div_center + egui::vec2(64.0, 0.0));
    assert!((native.runtime().window(win)?.app().split.ratio - initial_split_ratio).abs() > 0.01);
    native.runtime_mut().dispatch(DesktopHostEvent::MenuAction { window_id: win, command_id: String::from("ui.split.toggle") })?;
    settle(&mut native);

    // 3. Inline cell IME preedit + commit (`"濃度 μmol/L"`).
    let cell_pt = native.inspect_meta().find_by_widget_id("grid.cell.2.0.0").ok_or("missing grid.cell.2.0.0")?.clipped_rect.center();
    step(
        &mut native,
        vec![
            egui::Event::PointerMoved(cell_pt),
            egui::Event::PointerButton { pos: cell_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: cell_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: cell_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: cell_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
    );
    step(&mut native, Vec::new());
    step(
        &mut native,
        vec![
            egui::Event::Ime(egui::ImeEvent::Preedit { text: String::from("濃"), active_range_chars: Some(0..1) }),
            egui::Event::Ime(egui::ImeEvent::Commit(String::from("濃度 μmol/L"))),
            egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        ],
    );
    step(&mut native, Vec::new());
    let stored_cell = native
        .runtime()
        .window(win)?
        .app()
        .document()
        .data_sheet(&"data-xy".into())
        .and_then(|ds| ds.cell(CellCoord::new(2, 0, 0)))
        .ok_or("missing committed cell")?;
    assert_eq!(stored_cell.value, CellValue::Text(String::from("濃度 μmol/L")));

    // 4. InfoSheet notes IME commit (`"العربية"`).
    let _ = native.runtime_mut().window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "info-1" }))?;
    settle(&mut native);
    let notes_pt = native.inspect_meta().find_by_widget_id("info.notes_editor").ok_or("missing info.notes_editor")?.clipped_rect.center();
    step(
        &mut native,
        vec![
            egui::Event::PointerMoved(notes_pt),
            egui::Event::PointerButton { pos: notes_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: notes_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
    );
    step(&mut native, vec![egui::Event::Ime(egui::ImeEvent::Commit(String::from("العربية")))]);
    let save_pt = native.inspect_meta().find_by_widget_id("info.notes.save").ok_or("missing info.notes.save")?.clipped_rect.center();
    step(
        &mut native,
        vec![
            egui::Event::PointerMoved(save_pt),
            egui::Event::PointerButton { pos: save_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: save_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
    );
    step(&mut native, Vec::new());
    assert_eq!(native.runtime().window(win)?.app().document().info_sheet(&"info-1".into()).map(|i| i.notes_markdown.as_str()), Some("العربية"));

    // 5. Navigator sheet title rename IME commit (`"濃度 μmol/L — العربية"`).
    let graph_nav_pt = native.inspect_meta().find_by_widget_id("nav.sheet.graph-xy").ok_or("missing nav.sheet.graph-xy")?.clipped_rect.center();
    step(
        &mut native,
        vec![
            egui::Event::PointerMoved(graph_nav_pt),
            egui::Event::PointerButton { pos: graph_nav_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: graph_nav_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
    );
    step(
        &mut native,
        vec![
            egui::Event::PointerButton { pos: graph_nav_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: graph_nav_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
    );
    step(&mut native, Vec::new());
    if native.inspect_meta().find_by_widget_id("nav.rename_input").is_none() {
        step(
            &mut native,
            vec![egui::Event::Key { key: egui::Key::F2, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        );
        step(&mut native, Vec::new());
    }
    assert!(native.inspect_meta().find_by_widget_id("nav.rename_input").is_some(), "expected nav.rename_input open");
    step(
        &mut native,
        vec![
            egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND },
            egui::Event::Ime(egui::ImeEvent::Commit(String::from("濃度 μmol/L — العربية"))),
            egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        ],
    );
    step(&mut native, Vec::new());
    assert_eq!(
        native.runtime().window(win)?.app().document().graph_sheet(&"graph-xy".into()).map(|g| g.title.as_str()),
        Some("濃度 μmol/L — العربية")
    );
    Ok(())
}

#[test]
fn trackpad_magnify_pinch_zoom_inertial_coasting_and_clipboard() -> Result<(), Box<dyn std::error::Error>> {
    let metrics = ViewportMetrics::new(1200.0, 800.0, 2.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { initial_viewport: metrics, ..DesktopHostConfig::default() });
    let win = rt.open_window_with_session(Session::from_document(build_desktop_test_project()?), metrics, false)?;

    // Pinch zoom + trackpad wheel pan on graph-xy via live CurveBookNativeApp (`egui` shell).
    let _ = rt.window_mut(win)?.app_mut().execute_command("sheet.select", json!({ "sheet_id": "graph-xy" }))?;
    let ctx = egui::Context::default();
    ctx.set_fonts(build_egui_font_definitions(None));
    ctx.set_pixels_per_point(2.0);
    let mut native = CurveBookNativeApp::new(rt, win, None);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let step = |host: &mut CurveBookNativeApp, events: Vec<egui::Event>| {
        let mut raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        if let Some(vp) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
            vp.native_pixels_per_point = Some(2.0);
        }
        let mut out = ctx.run_ui(raw, |ui| host.step_ui(ui));
        out.textures_delta.clear();
    };
    step(&mut native, Vec::new());
    step(&mut native, Vec::new());
    let center = native.inspect_meta().find_by_widget_id("canvas.viewport").ok_or("missing canvas.viewport")?.clipped_rect.intersect(screen).center();

    let zoom_before = native.runtime().window(win)?.app().canvas_dialogs.pan_zoom.zoom;
    step(&mut native, vec![egui::Event::PointerMoved(center), egui::Event::Zoom(1.5)]);
    step(&mut native, Vec::new());
    let zoom_after = native.runtime().window(win)?.app().canvas_dialogs.pan_zoom.zoom;
    assert!((zoom_after - zoom_before * 1.5).abs() < 1e-4, "expected 1.5x zoom from {zoom_before}, got {zoom_after}");

    let pan_before = native.runtime().window(win)?.app().canvas_dialogs.pan_zoom.pan_pt;
    step(
        &mut native,
        vec![
            egui::Event::PointerMoved(center),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(30.0, -15.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
            },
        ],
    );
    step(&mut native, Vec::new());
    let pan_after_scroll = native.runtime().window(win)?.app().canvas_dialogs.pan_zoom.pan_pt;
    assert_ne!(pan_after_scroll, pan_before, "trackpad scroll over canvas at center={center:?} must pan (pan_before={pan_before:?})");

    // ClipboardCopy + ClipboardPaste round-trip on data-xy -> data-col.
    let rt = native.runtime_mut();
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
fn file_hover_and_file_dropped_routing_csv_and_curvebook() -> Result<(), Box<dyn std::error::Error>> {
    let dir = temp_test_dir("cb-w13-filedrop")?;
    let csv_path = dir.join("measurements.csv");
    fs::write(&csv_path, "Dose,Response\n0.1,10.0\n1.0,45.0\n")?;

    let mut proj_doc = ProjectDocument::default();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2)?;
    let mut ds = DataSheet::new("drop-proj-ds", "Archived Project Table", TableArchetype::Xy(xy.clone()))?;
    ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(42.0))?;
    xy.data_sets[0].title = String::from("Y");
    proj_doc.add_data_sheet(ds)?;
    let archive_path = dir.join("experiment.curvebook");
    save_project_file(&archive_path, &proj_doc)?;

    // Single-window mode (`new_instance: false`): drop .curvebook into clean window loads in place.
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

    // Dropping .curvebook onto a dirty window must open a new DesktopWindowSession instead of discarding edits.
    let dirty_drop = rt.dispatch(DesktopHostEvent::FileDropped { window_id: w1, path: archive_path.clone() })?;
    let w2 = dirty_drop.opened_window_id.ok_or("dirty window drop should open new window")?;
    assert_ne!(w2, w1);
    assert_eq!(rt.window_count(), 2);
    assert_eq!(rt.window(w1)?.app().document().data_sheets.len(), 2);

    // With new_instance: true, dropping .curvebook onto even a clean window opens a new window.
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
    let dir = temp_test_dir("cb-w13-cli")?;
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
    let bin = env!("CARGO_BIN_EXE_curvebook");
    let out = ProcessCommand::new(bin)
        .arg("--in-window-menus")
        .arg("--new-instance")
        .arg("--headless-once")
        .arg("--portable-dir")
        .arg(portable_dir.to_string_lossy().as_ref())
        .arg("--event-script")
        .arg(script_path.to_string_lossy().as_ref())
        .run()?;
    assert!(out.success, "curvebook CLI failed (status {:?}): {}", out.status_code, String::from_utf8_lossy(&out.stderr));

    let _ = fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn file_dropped_pzfx_and_zip_archive_open_cleanly() -> Result<(), Box<dyn std::error::Error>> {
    use curvebook_doc::{CellCoord, TableArchetype};
    use curvebook_format::ZipWriter;

    let dir = temp_test_dir("curvebook-dnd-pzfx-zip")?;
    let pzfx_path = dir.join("dropped.pzfx");
    fs::write(
        &pzfx_path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile SchemaVersion="2">
  <Title>Dropped PZFX Study</Title>
  <TableSequence>
    <Table ID="pz-1" TableType="OneWay">
      <Title>Dropped XML Columns</Title>
      <YColumn><Title>Baseline</Title><Subcolumn><d>3.5</d><d>4.5</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#,
    )?;

    let archive_path = dir.join("dropped.prism");
    {
        let mut writer = ZipWriter::new();
        writer.add_file(
            "manifest.json",
            serde_json::to_vec(&json!({
                "title": "Dropped Archive Container",
                "sheets": [{
                    "id": "pr-1",
                    "title": "Dropped Archive Trace",
                    "kind": "XY",
                    "subcolumn_mode": "Single",
                    "csv_path": "data/trace.csv"
                }]
            }))?,
        )?;
        writer.add_file("data/trace.csv", b"Time,Signal\n0,1.25\n10,9.75\n".to_vec())?;
        fs::write(&archive_path, writer.finish()?)?;
    }

    let metrics = ViewportMetrics::new(1100.0, 720.0, 1.0);
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
        in_window_menus: true,
        new_instance: false,
        gui_only: false,
        locale_override: None,
        theme_override: None,
        recovery_root: None,
        initial_viewport: metrics,
        ..DesktopHostConfig::default()
    });
    let wid = rt.open_window(None, metrics)?;

    // Drop .pzfx
    let pzfx_out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: pzfx_path })?;
    assert!(pzfx_out.handled);
    let pzfx_win = pzfx_out.opened_window_id.unwrap_or(wid);
    {
        let doc = rt.window(pzfx_win)?.app().document();
        let ds = doc.data_sheets.iter().find(|d| d.title == "Dropped XML Columns").ok_or("missing dropped .pzfx sheet")?;
        assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(3.5));
        assert!(rt.window(pzfx_win)?.app().session().file_path().is_none(), "foreign .pzfx drop detaches native archive path");
    }

    // Drop .prism
    let zip_out = rt.dispatch(DesktopHostEvent::FileDropped { window_id: wid, path: archive_path })?;
    assert!(zip_out.handled);
    let zip_win = zip_out.opened_window_id.unwrap_or(wid);
    {
        let doc = rt.window(zip_win)?.app().document();
        let ds = doc.data_sheets.iter().find(|d| d.title == "Dropped Archive Trace").ok_or("missing dropped .prism sheet")?;
        let TableArchetype::Xy(_) = &ds.archetype else {
            return Err(String::from("expected XY archetype from dropped .prism").into());
        };
        assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(9.75));
        assert!(rt.window(zip_win)?.app().session().file_path().is_none(), "foreign .prism drop detaches native archive path");
    }

    let _ = fs::remove_dir_all(&dir);
    Ok(())
}
