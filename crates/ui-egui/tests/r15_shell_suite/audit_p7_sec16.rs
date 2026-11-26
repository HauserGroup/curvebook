//! Phase 7 Stage 7.1 (`SEC-16`) Regression Tests (`FA-SEC16-02`..`FA-SEC16-12`).
#![forbid(unsafe_code)]

use curvebook_doc::{CellCoord, CellValue, ConstantValue, FloatingNote, RectPt, SheetId};
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view, render_canvas_view_for_sheet};
use curvebook_ui_egui::egui_shell::driver::build_driver_font_definitions;
use curvebook_ui_egui::egui_shell::fonts::build_egui_font_definitions;
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    HybridCoexistenceArbiter, HybridModalSurface, inspect_live_egui_snapshot, render_egui_shell_frame,
};
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::{UiApp, tr};
use serde_json::json;

use crate::common::{TestRes, widget_center};
use crate::helpers::{exec, has_widget, make_merge_driver, run_panel_driver, ui_cmd, widget_label};

fn make_xy_project() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ds-xy", "kind": "xy", "title": "Dose Response", "cols": 1, "rows": 3, "auto_graph": true, "auto_info": true }),
    )?;
    for (r, x, y) in [(0, 100.0, 5.0), (1, 200.0, 100.0), (2, 100.0, 9.0)] {
        exec(&mut app, "table.setXCells", json!({ "sheet_id": "ds-xy", "row": r, "value": x }))?;
        exec(&mut app, "table.setCell", json!({ "sheet_id": "ds-xy", "row": r, "col": 0, "value": y }))?;
    }
    exec(&mut app, "graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "ds-xy", "title": "Graph Two" }))?;
    exec(&mut app, "layout.create", json!({ "sheet_id": "lay-1", "title": "Layout One" }))?;
    exec(
        &mut app,
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-2", "bounds_pt": { "x": 40.0, "y": 40.0, "width": 160.0, "height": 120.0 } }),
    )?;
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    Ok(app)
}

fn collect_cmd_v_intents(app: &UiApp) -> Vec<UiIntent> {
    let ctx = egui::Context::default();
    ctx.set_fonts(build_driver_font_definitions());
    let mut views = app.egui_views.clone();
    let mut meta = FrameInspectMeta::new();
    let mut intents = PendingUiIntents::new();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 760.0))),
        events: vec![egui::Event::Key {
            key: egui::Key::V,
            physical_key: Some(egui::Key::V),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
        ..Default::default()
    };
    ctx.run_ui(raw, |ui| {
        let _ = render_egui_shell_frame(ui, app, &mut views, &mut meta, &mut intents);
    })
    .drop_without_applying_deltas();
    intents.intents().to_vec()
}

fn click_secondary_canvas_ctx(app: &UiApp, sheet_id: SheetId, view: CanvasViewEguiState, wid: &str) -> Result<Vec<UiIntent>, String> {
    let mut drv = run_panel_driver((app.clone(), view, PendingUiIntents::new()), 800.0, 600.0, move |ui, meta, (ap, vw, ints)| {
        let tokens = Tokens::for_mode(ap.theme.mode);
        let ctx = CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: true };
        let _ = render_canvas_view_for_sheet(ui, ap, Some(&sheet_id), vw, ctx, meta, ints);
    });
    drv.click_widget_id(wid)?;
    Ok(drv.state().2.intents().to_vec())
}

#[test]
fn fa_sec16_02_info_add_constant_creates_unique_empty_constants_without_overwriting() -> TestRes {
    let mut app = make_xy_project()?;
    let info_id = app.document().info_sheets.first().ok_or("missing info sheet")?.id.as_str().to_owned();
    exec(&mut app, "info.setConstant", json!({ "sheet_id": info_id, "name": "Constant", "value": 42.0 }))?;
    exec(&mut app, "info.setConstant", json!({ "sheet_id": info_id, "name": "Constant 1", "value": 99.0 }))?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": info_id }))?;
    app.sync_views();

    let mut driver = make_merge_driver(app, 1280.0, 760.0);
    for _ in 0..2 {
        driver.click_widget_id("toolbar.info_add_constant")?;
        assert_eq!(driver.state().last_intent_error, None);
    }
    let info = driver.state().app.document().info_sheet(&SheetId::new(&info_id)).ok_or("missing info")?;
    for (name, expected) in [
        ("Constant", ConstantValue::Number(42.0)),
        ("Constant 1", ConstantValue::Number(99.0)),
        ("Constant 2", ConstantValue::Empty),
        ("Constant 3", ConstantValue::Empty),
    ] {
        let c = info.constants.iter().find(|c| c.name == name).ok_or_else(|| format!("missing `{name}`"))?;
        assert_eq!(c.value, expected);
    }
    Ok(())
}

#[test]
fn fa_sec16_03_shell_and_leading_dock_shortcuts_honor_registry_rebindings_and_unbindings() -> TestRes {
    let mut app = make_xy_project()?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    for (cmd_id, combo) in [("ui.palette.open", "Ctrl+Shift+P"), ("ui.navigator.toggle", "Ctrl+B"), ("sheet.rename", "Ctrl+R")] {
        ui_cmd(&mut app, "ui.shortcut.set", json!({ "command_id": cmd_id, "combo": combo }))?;
    }
    app.sync_views();

    let mut driver = make_merge_driver(app, 1280.0, 760.0);
    driver.press_egui_key(egui::Key::K, egui::Modifiers::COMMAND);
    assert!(!matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    driver.press_egui_key(egui::Key::P, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));

    assert!(!driver.state().views.leading_collapsed_rail);
    driver.press_egui_key(egui::Key::N, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(!driver.state().views.leading_collapsed_rail);
    driver.press_egui_key(egui::Key::B, egui::Modifiers::CTRL);
    assert!(driver.state().views.leading_collapsed_rail);
    driver.press_egui_key(egui::Key::B, egui::Modifiers::CTRL);
    assert!(!driver.state().views.leading_collapsed_rail);

    driver.hover_at(widget_center(&driver, "nav.sheet.ds-xy")?)?;
    driver.press_egui_key(egui::Key::F2, egui::Modifiers::NONE);
    assert!(driver.state().views.leading_dock.renaming_sheet_id.is_none());
    driver.press_egui_key(egui::Key::R, egui::Modifiers::CTRL);
    assert_eq!(driver.state().views.leading_dock.renaming_sheet_id.as_ref().map(SheetId::as_str), Some("ds-xy"));
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);

    exec(&mut driver.state_mut().app, "table.setCell", json!({ "sheet_id": "ds-xy", "row": 0, "col": 0, "value": 777.0 }))?;
    ui_cmd(&mut driver.state_mut().app, "ui.shortcut.set", json!({ "command_id": "edit.undo", "combo": "" }))?;
    driver.run_settled();
    let undo_before = driver.state().app.session().undo_depth();
    driver.press_egui_key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before);

    let mut paste_app = make_xy_project()?;
    exec(&mut paste_app, "sheet.select", json!({ "sheet_id": "g-2" }))?;
    paste_app.sync_views();
    assert!(
        collect_cmd_v_intents(&paste_app)
            .iter()
            .any(|it| matches!(it, UiIntent::CanvasPaste { target_sheet_id, .. } if target_sheet_id.as_str() == "g-2"))
    );
    ui_cmd(&mut paste_app, "ui.shortcut.set", json!({ "command_id": "table.paste", "combo": "" }))?;
    let _ = ui_cmd(&mut paste_app, "ui.shortcut.set", json!({ "command_id": "edit.pasteImage", "combo": "" }));
    assert!(!collect_cmd_v_intents(&paste_app).iter().any(|it| matches!(it, UiIntent::CanvasPaste { .. })));
    Ok(())
}

#[test]
fn fa_sec16_04_delete_confirm_coexists_with_arbiter_and_dialog_snapshot_unifies_shell_modals() -> TestRes {
    let mut driver = make_merge_driver(make_xy_project()?, 1280.0, 760.0);
    let ds_pos = widget_center(&driver, "nav.sheet.ds-xy")?;
    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("nav.ctx.delete")?;

    let decision = HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false);
    assert_eq!(decision.modal_surface, HybridModalSurface::EguiNavDeleteConfirm);
    assert!(decision.modal_backdrop_active && has_widget(&driver, "modal.backdrop") && has_widget(&driver, "dialog.nav_delete_confirm"));

    driver.state_mut().outside_click_captured = false;
    driver.click_at(ds_pos)?;
    assert!(driver.state().outside_click_captured && driver.state().views.leading_dock.delete_confirm.is_some());

    let mut snap_app = driver.state().app.clone();
    snap_app.egui_views = driver.state().views.clone();
    let metrics = ViewportMetrics::new(1280.0, 760.0, 1.0);
    assert_eq!(
        inspect_live_egui_snapshot(&snap_app, metrics, Some("dialog"), false).dialog.as_ref().map(|d| d.dialog_kind.as_str()),
        Some("nav_delete_confirm")
    );

    driver.click_widget_id("nav.delete_confirm.cancel")?;
    assert!(driver.state().views.leading_dock.delete_confirm.is_none());

    type ModalCase = (&'static str, fn(&mut UiApp), Option<&'static str>);
    let cases: [ModalCase; 5] = [
        ("preferences", |a| a.egui_views.app_modals.preferences_open = true, None),
        ("find_replace", |a| a.egui_views.app_modals.find_replace_open = true, None),
        ("command_palette", |a| a.canvas_dialogs.active_dialog = ActiveDialog::CommandPalette, Some("palette.query")),
        ("convert_archetype", |a| a.canvas_dialogs.active_dialog = ActiveDialog::ConvertArchetype { sheet_id: SheetId::new("ds-xy") }, None),
        ("exclude_with_reason", |a| a.canvas_dialogs.active_dialog = ActiveDialog::ExcludeWithReason { sheet_id: SheetId::new("ds-xy") }, None),
    ];
    for (expected_kind, setup, req_wid) in cases {
        let mut a = make_xy_project()?;
        setup(&mut a);
        let dlg = inspect_live_egui_snapshot(&a, metrics, Some("dialog"), false).dialog.ok_or_else(|| format!("missing {expected_kind}"))?;
        assert_eq!(dlg.dialog_kind, expected_kind);
        if let Some(wid) = req_wid {
            assert!(dlg.controls.iter().any(|c| c.widget_id == wid), "missing {wid}");
        }
    }
    Ok(())
}

#[test]
fn fa_sec16_05_rtl_mirrors_leading_trailing_docks_and_splitters() -> TestRes {
    let mut app = make_xy_project()?;
    assert_eq!(app.set_locale("ar"), "ar");
    app.sync_views();

    let mut driver = make_merge_driver(app, 1280.0, 760.0);
    let layout = driver.state().views.last_shell_layout.ok_or("missing shell layout")?;
    assert!(layout.rtl_mirrored);
    let trail_rect = driver.state().trailing_rect_pt.ok_or("missing trailing rect")?;
    let lead_rect = driver.state().leading_rect_pt.ok_or("missing leading rect")?;
    let canvas_rect = driver.state().canvas_rect_pt.ok_or("missing canvas rect")?;
    assert!(trail_rect.left() < canvas_rect.left() && canvas_rect.right() <= lead_rect.left() + 8.0);
    assert_eq!(widget_label(&driver, "nav.collapse_rail")?, "»");

    let nav_split = widget_center(&driver, "nav.dock_splitter")?;
    driver.drag(nav_split, egui::pos2(nav_split.x - 40.0, nav_split.y))?;
    assert!(driver.state().views.last_shell_layout.ok_or("layout")?.leading_dock_w_pt > layout.leading_dock_w_pt + 10.0);

    let trail_before = driver.state().views.last_shell_layout.ok_or("layout")?.trailing_dock_w_pt;
    let trail_split = widget_center(&driver, "inspector.dock_splitter")?;
    driver.drag(trail_split, egui::pos2(trail_split.x + 40.0, trail_split.y))?;
    assert!(driver.state().views.last_shell_layout.ok_or("layout")?.trailing_dock_w_pt > trail_before + 10.0);

    driver.click_widget_id("nav.collapse_rail")?;
    let expand_btn = driver.frame_meta().find_by_widget_id("nav.rail.expand").ok_or("missing nav.rail.expand")?;
    let collapsed_canvas = driver.state().canvas_rect_pt.ok_or("missing canvas rect")?;
    assert_eq!(expand_btn.label, tr("ar", "menu.view.navigator"));
    assert!(expand_btn.clipped_rect.left() >= collapsed_canvas.right());
    Ok(())
}

#[test]
fn fa_sec16_06_fonts_register_rtl_families_and_disambiguate_font_keys() {
    let built = build_egui_font_definitions();
    assert!(built.definitions.families.get(&egui::FontFamily::Proportional).is_some_and(|c| !c.is_empty()));
    assert!(built.definitions.families.get(&egui::FontFamily::Monospace).is_some_and(|c| !c.is_empty()));
    if cfg!(target_os = "macos") {
        let ctx = egui::Context::default();
        ctx.set_fonts(built.definitions);
        ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.ctx().fonts_mut(|fonts| {
                for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    let id = egui::FontId::new(13.0, fam);
                    assert!(fonts.has_glyph(&id, 'م') && fonts.has_glyph(&id, 'ש'), "missing RTL glyphs in {id:?}");
                }
            });
        })
        .drop_without_applying_deltas();
    }
}

#[test]
fn fa_sec16_07_command_palette_preserves_text_home_caret_and_routes_export_figure() -> TestRes {
    let mut app = make_xy_project()?;
    let g_id = app.document().graph_sheets[0].id.as_str().to_owned();
    exec(&mut app, "sheet.select", json!({ "sheet_id": g_id }))?;
    app.sync_views();

    let mut driver = make_merge_driver(app, 1280.0, 760.0);
    driver.press_egui_key(egui::Key::K, egui::Modifiers::COMMAND);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    driver.type_text("fit")?;
    driver.press_egui_key(egui::Key::Home, egui::Modifiers::NONE);
    driver.type_text("x")?;
    assert_eq!(driver.state().views.command_palette.query, "xfit");

    driver.state_mut().views.command_palette.query = String::from("Export Figure");
    driver.run_settled();
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }));
    Ok(())
}

#[test]
fn fa_sec16_08_find_replace_scans_x_cells_and_advances_with_wrap() -> TestRes {
    let mut app = make_xy_project()?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    app.sync_views();

    let mut driver = make_merge_driver(app, 1280.0, 760.0);
    driver.state_mut().views.app_modals.find_replace_open = true;
    driver.state_mut().views.app_modals.find_replace.find_query = String::from("100");
    driver.state_mut().views.app_modals.find_replace.replace_with = String::from("999");
    driver.run_settled();

    for (expect_x, expect_y) in [(Some((0, 0)), None), (None, Some(GridCellCoord::new(1, 0, 0))), (Some((2, 0)), None), (Some((0, 0)), None)] {
        driver.click_widget_id("find_replace.find")?;
        assert_eq!(driver.state().views.app_modals.find_replace.match_count, 3);
        assert_eq!(driver.state().app.grid.active_x_cell, expect_x);
        if let Some(coord) = expect_y {
            assert_eq!(driver.state().app.grid.tool.active, Some(coord));
        }
    }

    driver.click_widget_id("find_replace.replace_all")?;
    assert_eq!(driver.state().last_intent_error, None);
    let ds = driver.state().app.document().data_sheet(&SheetId::new("ds-xy")).ok_or("missing ds-xy")?;
    assert_eq!(ds.x_or_time_cell(0, 0).map(|c| &c.value), Some(&CellValue::Number(999.0)));
    assert_eq!(ds.x_or_time_cell(2, 0).map(|c| &c.value), Some(&CellValue::Number(999.0)));
    assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).map(|c| &c.value), Some(&CellValue::Number(999.0)));

    exec(
        &mut driver.state_mut().app,
        "sheet.createData",
        json!({ "sheet_id": "ds-surv", "kind": "survival", "title": "Survival Time", "cols": 1, "rows": 2, "auto_graph": false, "auto_info": false }),
    )?;
    exec(&mut driver.state_mut().app, "table.setXCells", json!({ "sheet_id": "ds-surv", "row": 1, "value": 42.0 }))?;
    exec(&mut driver.state_mut().app, "sheet.select", json!({ "sheet_id": "ds-surv" }))?;
    driver.state_mut().views.app_modals.find_replace.find_query = String::from("42");
    driver.state_mut().views.app_modals.find_replace.replace_with = String::from("84");
    driver.run_settled();
    driver.click_widget_id("find_replace.find")?;
    assert_eq!(driver.state().app.grid.active_x_cell, Some((1, 0)));
    driver.click_widget_id("find_replace.replace_all")?;
    let surv = driver.state().app.document().data_sheet(&SheetId::new("ds-surv")).ok_or("missing ds-surv")?;
    assert_eq!(surv.x_or_time_cell(1, 0).map(|c| &c.value), Some(&CellValue::Number(84.0)));
    Ok(())
}

#[test]
fn fa_sec16_09_footer_cursor_uses_paper_coords_and_popovers_dismiss_on_esc_click_and_modal() -> TestRes {
    let mut app = make_xy_project()?;
    let g_id = app.document().graph_sheets[0].id.clone();
    exec(&mut app, "sheet.select", json!({ "sheet_id": g_id.as_str() }))?;
    app.sync_views();

    let mut driver = make_merge_driver(app, 1280.0, 760.0);
    let paper = driver.state().views.canvas_view.last_paper_rect_pt.ok_or("missing paper rect")?;
    let scene_sz = driver.state().views.canvas_view.last_scene_size_pt.ok_or("missing scene size")?;

    driver.hover_at(paper.center())?;
    let expected_x = format!("X: {:.1}, Y: {:.1} pt", scene_sz[0] * 0.5, scene_sz[1] * 0.5);
    assert!(widget_label(&driver, "status.summary")?.contains(&expected_x));

    driver.click_widget_id("nav.collapse_rail")?;
    driver.click_widget_id("status.sheet_switcher")?;
    assert!(driver.state().views.status_footer.sheet_switcher_open);
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!driver.state().views.status_footer.sheet_switcher_open);

    driver.click_widget_id("status.sheet_switcher")?;
    assert!(driver.state().views.status_footer.sheet_switcher_open);
    driver.click_at(paper.center())?;
    assert!(!driver.state().views.status_footer.sheet_switcher_open);

    exec(&mut driver.state_mut().app, "sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    driver.run_settled();
    driver.click_widget_id("status.summary")?;
    assert!(driver.state().views.status_footer.stats_popover_open);
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::About;
    driver.run_settled();
    assert!(!driver.state().views.status_footer.stats_popover_open);
    Ok(())
}

#[test]
fn fa_sec16_10_insert_image_targets_pane_sheet_zero_drag_no_dirty_and_per_sheet_state() -> TestRes {
    let mut app = make_xy_project()?;
    let (g1, g2, lay1) = (app.document().graph_sheets[0].id.clone(), SheetId::new("g-2"), SheetId::new("lay-1"));
    let _ = app.execute_command("graph.addAnnotation", json!({ "sheet_id": g1.as_str(), "annotation": { "kind": "embedded_result_table", "result_sheet_id": g1.as_str(), "bounds_pt": { "x": 40.0, "y": 40.0, "width": 120.0, "height": 80.0 } } }));
    exec(&mut app, "sheet.select", json!({ "sheet_id": g1.as_str() }))?;
    app.sync_views();

    // 1. Secondary graph & layout context menus route insert_image to the pane's sheet_id.
    let mut g_view = CanvasViewEguiState { synchronous_raster: true, ..Default::default() };
    g_view.graph_context_menu = Some((g2.clone(), None, egui::pos2(120.0, 120.0), [60.0, 60.0]));
    let mut l_view = CanvasViewEguiState { synchronous_raster: true, ..Default::default() };
    l_view.layout_context_menu = Some((lay1.clone(), None, egui::pos2(120.0, 120.0)));
    for (sid, vw, wid, cmd) in [
        (g2, g_view, "canvas.graph_ctx.insert_image", "graph.insertImage"),
        (lay1.clone(), l_view, "canvas.layout_ctx.insert_image", "layout.insertImage"),
    ] {
        let intents = click_secondary_canvas_ctx(&app, sid.clone(), vw, wid)?;
        assert!(
            intents
                .iter()
                .any(|it| matches!(it, UiIntent::EngineCommand { command_id, params } if command_id == cmd && params["sheet_id"] == sid.as_str())),
            "expected `{cmd}` for {sid}: {intents:?}"
        );
    }

    // 2. Layout 0-displacement drag emits no undo entry.
    exec(&mut app, "sheet.select", json!({ "sheet_id": lay1.as_str() }))?;
    let undo_before = app.session().undo_depth();
    let mut lay_drv = make_merge_driver(app.clone(), 1280.0, 760.0);
    let panel_center = widget_center(&lay_drv, "canvas.layout_panel.0")?;
    lay_drv.drag(panel_center, panel_center)?;
    assert_eq!(lay_drv.state().app.session().undo_depth(), undo_before);

    // 3. Per-sheet pan & selection isolation across g1 <-> g2.
    let sheet_state = CanvasViewEguiState { synchronous_raster: true, pan_pt: [40.0, 25.0], selected_panel_index: Some(0), ..Default::default() };
    exec(&mut app, "sheet.select", json!({ "sheet_id": g1.as_str() }))?;
    let mut pan_drv = run_panel_driver((app, sheet_state, PendingUiIntents::new()), 800.0, 600.0, |ui, meta, (ap, vw, ints)| {
        let tokens = Tokens::for_mode(ap.theme.mode);
        let ctx = CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false };
        let _ = render_canvas_view(ui, ap, vw, ctx, meta, ints);
    });
    assert_eq!(pan_drv.state().1.pan_pt, [40.0, 25.0]);
    exec(&mut pan_drv.state_mut().0, "sheet.select", json!({ "sheet_id": "g-2" }))?;
    pan_drv.run_settled();
    assert_eq!((pan_drv.state().1.pan_pt, pan_drv.state().1.selected_panel_index), ([0.0, 0.0], None));
    exec(&mut pan_drv.state_mut().0, "sheet.select", json!({ "sheet_id": g1.as_str() }))?;
    pan_drv.run_settled();
    assert_eq!((pan_drv.state().1.pan_pt, pan_drv.state().1.selected_panel_index), ([40.0, 25.0], Some(0)));
    Ok(())
}

#[test]
fn fa_sec16_11_split_preview_focus_recorded_and_note_pinned_persists_across_sheets() -> TestRes {
    let mut app = UiApp::new();
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "col-big", "kind": "column", "title": "Big Column", "cols": 3, "rows": 800, "auto_graph": true }),
    )?;
    let g_id = app.document().graph_sheets[0].id.as_str().to_owned();
    let mut doc = app.document().clone();
    doc.graph_sheet_mut(&SheetId::new(&g_id)).ok_or("missing g_id")?.floating_notes.push(FloatingNote::new(
        "pin-1",
        RectPt::new(16.0, 16.0, 160.0, 90.0),
        "Pinned observation",
    ));
    doc.active_sheet_id = Some(SheetId::new(&g_id));
    *app.session_mut() = curvebook_engine::Session::from_document(doc);
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new(&g_id));
    app.split.secondary_sheet_id = Some(SheetId::new("col-big"));
    app.nav.welcome.open = false;
    app.sync_views();

    let mut driver = make_merge_driver(app, 1280.0, 760.0);
    assert!(has_widget(&driver, "grid.cell.0.0.0"));
    driver.click_widget_id(&format!("floating_note.{g_id}.pin-1.pin"))?;
    driver.click_widget_id("split.close")?;
    exec(&mut driver.state_mut().app, "sheet.select", json!({ "sheet_id": "col-big" }))?;
    driver.run_settled();
    assert!(has_widget(&driver, &format!("floating_note.{g_id}.pin-1.card")));
    Ok(())
}

#[test]
fn fa_sec16_12_leading_dock_i18n_frozen_convert_and_drag_title() -> TestRes {
    let mut app = make_xy_project()?;
    exec(&mut app, "sheet.freeze", json!({ "sheet_id": "ds-xy" }))?;
    assert_eq!(app.set_locale("de"), "de");
    app.sync_views();

    let mut driver = make_merge_driver(app, 1280.0, 760.0);
    let new_data = widget_label(&driver, "nav.new.data")?;
    assert_eq!(new_data, format!("+ {}", tr("de", "menu.insert.new_data_table")));
    assert!(!new_data.contains("New Data"));

    driver.click_at_button(widget_center(&driver, "nav.sheet.ds-xy")?, PointerButton::Secondary, KeyModifiers::default())?;
    for (wid, key) in [("nav.ctx.convert_archetype", "cmd.ui.table.convertArchetype.open"), ("nav.ctx.rename", "cmd.sheet.rename")] {
        let entry = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing {wid}"))?;
        assert!(!entry.enabled && entry.label == tr("de", key), "{wid}: enabled={} label={}", entry.enabled, entry.label);
    }
    assert!(!tr("de", "nav.disabled_need_info").contains("Create a Data") && !tr("de", "nav.moving_prefix").contains("Moving"));
    Ok(())
}
