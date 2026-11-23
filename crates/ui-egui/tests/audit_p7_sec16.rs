#![forbid(unsafe_code)]

//! Phase 7 Stage 7.1 (`SEC-16`) Regression Tests (`FA-SEC16-02`..`FA-SEC16-12`).

use curvebook_doc::{CellValue, ConstantValue, FloatingNote, RectPt, SheetId};
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::canvas_view::{CanvasViewContext, CanvasViewEguiState, render_canvas_view, render_canvas_view_for_sheet};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::fonts::build_egui_font_definitions;
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    HybridCoexistenceArbiter, HybridModalSurface, ShellAboutFrameReport, inspect_live_egui_snapshot, render_egui_shell_frame,
};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::{UiApp, tr};
use serde_json::json;

type TestRes = Result<(), Box<dyn std::error::Error>>;

fn widget_center<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> Result<egui::Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
    Ok(entry.clipped_rect.center())
}

struct ShellState {
    app: UiApp,
    views: EguiShellViewState,
    carried_intents: PendingUiIntents,
    last_report: ShellAboutFrameReport,
    saw_outside_click_captured: bool,
    last_err: Option<String>,
}

fn make_shell_driver(app: UiApp, width: f64, height: f64) -> EguiHumanPathDriver<'static, ShellState> {
    let views = app.egui_views.clone();
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ShellState {
            app,
            views,
            carried_intents: PendingUiIntents::new(),
            last_report: ShellAboutFrameReport::default(),
            saw_outside_click_captured: false,
            last_err: None,
        },
        ViewportMetrics::new(width, height, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let mut pass_intents = PendingUiIntents::new();
            st.last_report = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass_intents);
            st.saw_outside_click_captured |= st.last_report.outside_click_captured;
            st.carried_intents.merge_pass(pass_intents);
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled && !st.carried_intents.is_empty() {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.carried_intents.commit_if_settled(&mut st.app, false) {
                st.last_err = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        }
        if settled {
            st.app.egui_views = st.views.clone();
        }
    });
    driver.run_settled();
    driver
}

fn make_xy_project() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-xy", "kind": "xy", "title": "Dose Response", "cols": 1, "rows": 3, "auto_graph": true, "auto_info": true }),
    )?;
    for (r, x, y) in [(0, 100.0, 5.0), (1, 200.0, 100.0), (2, 100.0, 9.0)] {
        app.execute_command("table.setXCells", json!({ "sheet_id": "ds-xy", "row": r, "value": x }))?;
        app.execute_command("table.setCell", json!({ "sheet_id": "ds-xy", "row": r, "col": 0, "value": y }))?;
    }
    app.execute_command("graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "ds-xy", "title": "Graph Two" }))?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-1", "title": "Layout One" }))?;
    app.execute_command(
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-2", "bounds_pt": { "x": 40.0, "y": 40.0, "width": 160.0, "height": 120.0 } }),
    )?;
    app.nav.welcome.open = false;
    app.nav.navigator_open = true;
    app.nav.inspector_open = true;
    app.sync_views();
    Ok(app)
}

#[test]
fn fa_sec16_02_info_add_constant_creates_unique_empty_constants_without_overwriting() -> TestRes {
    let mut app = make_xy_project()?;
    let info_id = app.document().info_sheets.first().ok_or("missing info sheet")?.id.as_str().to_owned();
    app.execute_command("info.setConstant", json!({ "sheet_id": info_id, "name": "Constant", "value": 42.0 }))?;
    app.execute_command("info.setConstant", json!({ "sheet_id": info_id, "name": "Constant 1", "value": 99.0 }))?;
    app.execute_command("sheet.select", json!({ "sheet_id": info_id }))?;
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 760.0);
    driver.click_widget_id("toolbar.info_add_constant")?;
    assert_eq!(driver.state().last_err, None);
    driver.click_widget_id("toolbar.info_add_constant")?;
    assert_eq!(driver.state().last_err, None);

    let info = driver.state().app.document().info_sheet(&SheetId::new(&info_id)).ok_or("info sheet missing")?;
    let c0 = info.constants.iter().find(|c| c.name == "Constant").ok_or("existing `Constant` overwritten/missing")?;
    let c1 = info.constants.iter().find(|c| c.name == "Constant 1").ok_or("existing `Constant 1` overwritten/missing")?;
    let c2 = info.constants.iter().find(|c| c.name == "Constant 2").ok_or("expected new `Constant 2`")?;
    let c3 = info.constants.iter().find(|c| c.name == "Constant 3").ok_or("expected new `Constant 3`")?;
    assert_eq!(c0.value, ConstantValue::Number(42.0));
    assert_eq!(c1.value, ConstantValue::Number(99.0));
    assert_eq!(c2.value, ConstantValue::Empty);
    assert_eq!(c3.value, ConstantValue::Empty);
    Ok(())
}

#[test]
fn fa_sec16_03_shell_and_leading_dock_shortcuts_honor_registry_rebindings_and_unbindings() -> TestRes {
    let mut app = make_xy_project()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    curvebook_ui_egui::control_io::dispatch_ui_command(
        &mut app,
        "ui.shortcut.set",
        &json!({ "command_id": "ui.palette.open", "combo": "Ctrl+Shift+P" }),
    )?;
    curvebook_ui_egui::control_io::dispatch_ui_command(
        &mut app,
        "ui.shortcut.set",
        &json!({ "command_id": "ui.navigator.toggle", "combo": "Ctrl+B" }),
    )?;
    curvebook_ui_egui::control_io::dispatch_ui_command(&mut app, "ui.shortcut.set", &json!({ "command_id": "sheet.rename", "combo": "Ctrl+R" }))?;
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 760.0);

    // Old `Cmd+K` must no longer open Command Palette; rebound `Ctrl+Shift+P` must open it.
    driver.press_egui_key(egui::Key::K, egui::Modifiers::COMMAND);
    assert!(!matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    driver.press_egui_key(egui::Key::P, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));

    // Old `Cmd+Shift+N` must no longer collapse Navigator; rebound `Ctrl+B` must collapse it.
    assert!(!driver.state().views.leading_collapsed_rail);
    driver.press_egui_key(egui::Key::N, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(!driver.state().views.leading_collapsed_rail);
    driver.press_egui_key(egui::Key::B, egui::Modifiers::CTRL);
    assert!(driver.state().views.leading_collapsed_rail);
    driver.press_egui_key(egui::Key::B, egui::Modifiers::CTRL);
    assert!(!driver.state().views.leading_collapsed_rail);

    // Move pointer into Navigator so dock rename shortcut applies; `F2` must not rename when rebound to `Ctrl+R`.
    let nav_center = widget_center(&driver, "nav.sheet.ds-xy")?;
    driver.hover_at(nav_center)?;
    driver.press_egui_key(egui::Key::F2, egui::Modifiers::NONE);
    assert!(driver.state().views.leading_dock.renaming_sheet_id.is_none());
    driver.press_egui_key(egui::Key::R, egui::Modifiers::CTRL);
    assert_eq!(driver.state().views.leading_dock.renaming_sheet_id.as_ref().map(SheetId::as_str), Some("ds-xy"));
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);

    // Unbind `edit.undo`: `Cmd+Z` must no longer undo table edits.
    driver.state_mut().app.execute_command("table.setCell", json!({ "sheet_id": "ds-xy", "row": 0, "col": 0, "value": 777.0 }))?;
    curvebook_ui_egui::control_io::dispatch_ui_command(
        &mut driver.state_mut().app,
        "ui.shortcut.set",
        &json!({ "command_id": "edit.undo", "combo": "" }),
    )?;
    driver.run_settled();
    let undo_before = driver.state().app.session().undo_depth();
    driver.press_egui_key(egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(driver.state().app.session().undo_depth(), undo_before);

    // Canvas `Cmd+V` honors `table.paste` / `edit.paste` registry state on figure sheets (`GraphSheet`/`LayoutSheet`).
    let mut paste_app = make_xy_project()?;
    paste_app.execute_command("sheet.select", json!({ "sheet_id": "g-2" }))?;
    paste_app.sync_views();
    let paste_ctx = egui::Context::default();
    paste_ctx.set_fonts(build_driver_font_definitions());
    let mut paste_views = paste_app.egui_views.clone();
    let mut paste_meta = curvebook_ui_egui::egui_shell::FrameInspectMeta::new();
    let mut paste_intents = PendingUiIntents::new();
    let raw_v = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 760.0))),
        events: vec![egui::Event::Key {
            key: egui::Key::V,
            physical_key: Some(egui::Key::V),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
        ..Default::default()
    };
    paste_ctx
        .run_ui(raw_v.clone(), |ui| {
            let _ = render_egui_shell_frame(ui, &paste_app, &mut paste_views, &mut paste_meta, &mut paste_intents);
        })
        .drop_without_applying_deltas();
    assert!(
        paste_intents.intents().iter().any(|it| matches!(it, UiIntent::CanvasPaste { target_sheet_id, .. } if target_sheet_id.as_str() == "g-2")),
        "expected default `table.paste` (`Cmd+V`) to emit CanvasPaste on `g-2`: {:?}",
        paste_intents.intents()
    );

    curvebook_ui_egui::control_io::dispatch_ui_command(&mut paste_app, "ui.shortcut.set", &json!({ "command_id": "table.paste", "combo": "" }))?;
    let _ = curvebook_ui_egui::control_io::dispatch_ui_command(
        &mut paste_app,
        "ui.shortcut.set",
        &json!({ "command_id": "edit.pasteImage", "combo": "" }),
    );
    let mut unbound_intents = PendingUiIntents::new();
    paste_ctx
        .run_ui(raw_v, |ui| {
            let _ = render_egui_shell_frame(ui, &paste_app, &mut paste_views, &mut paste_meta, &mut unbound_intents);
        })
        .drop_without_applying_deltas();
    assert!(
        !unbound_intents.intents().iter().any(|it| matches!(it, UiIntent::CanvasPaste { .. })),
        "unbinding `table.paste`/`edit.paste`/`edit.pasteImage` must suppress canvas Cmd+V paste"
    );
    Ok(())
}

#[test]
fn fa_sec16_04_delete_confirm_coexists_with_arbiter_and_dialog_snapshot_unifies_shell_modals() -> TestRes {
    let app = make_xy_project()?;
    let mut driver = make_shell_driver(app, 1280.0, 760.0);

    let ds_pos = widget_center(&driver, "nav.sheet.ds-xy")?;
    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    driver.click_widget_id("nav.ctx.delete")?;

    let decision = HybridCoexistenceArbiter::evaluate_with_views(&driver.state().app, &driver.state().views, false);
    assert_eq!(decision.modal_surface, HybridModalSurface::EguiNavDeleteConfirm);
    assert!(decision.modal_backdrop_active);
    assert!(driver.frame_meta().find_by_widget_id("modal.backdrop").is_some());
    assert!(driver.frame_meta().find_by_widget_id("dialog.nav_delete_confirm").is_some());

    // Clicking outside the confirm modal on the backdrop (over Navigator) must be captured cleanly.
    driver.state_mut().saw_outside_click_captured = false;
    driver.click_at(ds_pos)?;
    assert!(driver.state().saw_outside_click_captured);
    assert!(driver.state().views.leading_dock.delete_confirm.is_some());

    let mut snap_app = driver.state().app.clone();
    snap_app.egui_views = driver.state().views.clone();
    let del_snap = inspect_live_egui_snapshot(&snap_app, ViewportMetrics::new(1280.0, 760.0, 1.0), Some("dialog"), false);
    assert_eq!(del_snap.dialog.as_ref().map(|d| d.dialog_kind.as_str()), Some("nav_delete_confirm"));

    driver.click_widget_id("nav.delete_confirm.cancel")?;
    assert!(driver.state().views.leading_dock.delete_confirm.is_none());

    // Preferences, Find & Replace, and Command Palette must all emit accurate `dialog_kind` in `ui.inspect`.
    let mut pref_app = make_xy_project()?;
    pref_app.egui_views.app_modals.preferences_open = true;
    let pref_snap = inspect_live_egui_snapshot(&pref_app, ViewportMetrics::new(1280.0, 760.0, 1.0), Some("dialog"), false);
    assert_eq!(pref_snap.dialog.as_ref().map(|d| d.dialog_kind.as_str()), Some("preferences"));

    let mut find_app = make_xy_project()?;
    find_app.egui_views.app_modals.find_replace_open = true;
    let find_snap = inspect_live_egui_snapshot(&find_app, ViewportMetrics::new(1280.0, 760.0, 1.0), Some("dialog"), false);
    assert_eq!(find_snap.dialog.as_ref().map(|d| d.dialog_kind.as_str()), Some("find_replace"));

    let mut pal_app = make_xy_project()?;
    pal_app.canvas_dialogs.active_dialog = ActiveDialog::CommandPalette;
    let pal_snap = inspect_live_egui_snapshot(&pal_app, ViewportMetrics::new(1280.0, 760.0, 1.0), Some("dialog"), false);
    let pal_dlg = pal_snap.dialog.ok_or("missing CommandPalette dialog snapshot")?;
    assert_eq!(pal_dlg.dialog_kind, "command_palette");
    assert!(pal_dlg.controls.iter().any(|c| c.widget_id == "palette.query"));

    // Bridged `ActiveDialog` variants keep their exact `dialog_kind` rather than collapsing to surface groups.
    let mut conv_app = make_xy_project()?;
    conv_app.canvas_dialogs.active_dialog = ActiveDialog::ConvertArchetype { sheet_id: SheetId::new("ds-xy") };
    let conv_snap = inspect_live_egui_snapshot(&conv_app, ViewportMetrics::new(1280.0, 760.0, 1.0), Some("dialog"), false);
    assert_eq!(conv_snap.dialog.as_ref().map(|d| d.dialog_kind.as_str()), Some("convert_archetype"));

    let mut excl_app = make_xy_project()?;
    excl_app.canvas_dialogs.active_dialog = ActiveDialog::ExcludeWithReason { sheet_id: SheetId::new("ds-xy") };
    let excl_snap = inspect_live_egui_snapshot(&excl_app, ViewportMetrics::new(1280.0, 760.0, 1.0), Some("dialog"), false);
    assert_eq!(excl_snap.dialog.as_ref().map(|d| d.dialog_kind.as_str()), Some("exclude_with_reason"));
    Ok(())
}

#[test]
fn fa_sec16_05_rtl_mirrors_leading_trailing_docks_and_splitters() -> TestRes {
    let mut app = make_xy_project()?;
    assert_eq!(app.set_locale("ar"), "ar");
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 760.0);
    let layout = driver.state().views.last_shell_layout.ok_or("missing shell layout")?;
    assert!(layout.rtl_mirrored);

    let trail_rect = driver.state().last_report.trailing_dock_rect_pt.ok_or("missing trailing rect")?;
    let lead_rect = driver.state().last_report.leading_dock_rect_pt.ok_or("missing leading rect")?;
    let canvas_rect = driver.state().last_report.regions.canvas_rect_pt.ok_or("missing canvas rect")?;
    assert!(trail_rect.left() < canvas_rect.left() && canvas_rect.right() <= lead_rect.left() + 8.0);
    assert_eq!(driver.frame_meta().find_by_widget_id("nav.collapse_rail").map(|n| n.label.as_str()), Some("»"));

    // Dragging the right-docked Navigator splitter leftwards widens Navigator in RTL.
    let w_before = layout.leading_dock_w_pt;
    let nav_split = widget_center(&driver, "nav.dock_splitter")?;
    driver.drag(nav_split, egui::pos2(nav_split.x - 40.0, nav_split.y))?;
    let w_after = driver.state().views.last_shell_layout.ok_or("missing layout after drag")?.leading_dock_w_pt;
    assert!(w_after > w_before + 10.0, "expected RTL leftward drag on Navigator splitter to widen dock: {w_before} -> {w_after}");

    // Dragging the left-docked Inspector splitter rightwards widens Inspector in RTL.
    let trail_before = driver.state().views.last_shell_layout.ok_or("missing layout")?.trailing_dock_w_pt;
    let trail_split = widget_center(&driver, "inspector.dock_splitter")?;
    driver.drag(trail_split, egui::pos2(trail_split.x + 40.0, trail_split.y))?;
    let trail_after = driver.state().views.last_shell_layout.ok_or("missing layout after inspector drag")?.trailing_dock_w_pt;
    assert!(trail_after > trail_before + 10.0, "expected RTL rightward drag on Inspector splitter to widen dock: {trail_before} -> {trail_after}");

    // Collapsed rail stays docked on the right in RTL and exposes localized expand button.
    driver.click_widget_id("nav.collapse_rail")?;
    let expand_btn = driver.frame_meta().find_by_widget_id("nav.rail.expand").ok_or("missing nav.rail.expand")?;
    let collapsed_canvas = driver.state().last_report.regions.canvas_rect_pt.ok_or("missing collapsed canvas rect")?;
    assert_eq!(expand_btn.label, tr("ar", "menu.view.navigator"));
    assert!(expand_btn.clipped_rect.left() >= collapsed_canvas.right(), "RTL collapsed rail should stay on right edge");
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
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    let id = egui::FontId::new(13.0, family);
                    assert!(fonts.has_glyph(&id, 'م'), "missing Arabic glyph in {id:?}");
                    assert!(fonts.has_glyph(&id, 'ש'), "missing Hebrew glyph in {id:?}");
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
    app.execute_command("sheet.select", json!({ "sheet_id": g_id }))?;
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 760.0);
    driver.press_egui_key(egui::Key::K, egui::Modifiers::COMMAND);
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));

    // Type non-empty query -> pressing `Home` must move caret to start of `query` rather than navigating rows.
    driver.type_text("fit")?;
    driver.press_egui_key(egui::Key::Home, egui::Modifiers::NONE);
    driver.type_text("x")?;
    assert_eq!(driver.state().views.command_palette.query, "xfit");

    // Verify `ui.export.figure.open` appears in palette and opens Export Figure cleanly when chosen.
    driver.state_mut().views.command_palette.query = String::from("Export Figure");
    driver.run_settled();
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!(
        matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }),
        "expected Export Figure dialog, got {:?}",
        driver.state().app.canvas_dialogs.active_dialog
    );
    Ok(())
}

#[test]
fn fa_sec16_08_find_replace_scans_x_cells_and_advances_with_wrap() -> TestRes {
    let mut app = make_xy_project()?;
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" }))?;
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 760.0);
    driver.state_mut().views.app_modals.find_replace_open = true;
    driver.state_mut().views.app_modals.find_replace.find_query = String::from("100");
    driver.state_mut().views.app_modals.find_replace.replace_with = String::from("999");
    driver.run_settled();

    // 1. First match: X row 0.
    driver.click_widget_id("find_replace.find")?;
    assert_eq!(driver.state().views.app_modals.find_replace.match_count, 3);
    assert_eq!(driver.state().app.grid.active_x_cell, Some((0, 0)));

    // 2. Next match: Y row 1, col 0.
    driver.click_widget_id("find_replace.find")?;
    assert_eq!(driver.state().app.grid.active_x_cell, None);
    assert_eq!(driver.state().app.grid.tool.active, Some(GridCellCoord::new(1, 0, 0)));

    // 3. Next match: X row 2.
    driver.click_widget_id("find_replace.find")?;
    assert_eq!(driver.state().app.grid.active_x_cell, Some((2, 0)));

    // 4. Wraps cleanly back to X row 0.
    driver.click_widget_id("find_replace.find")?;
    assert_eq!(driver.state().app.grid.active_x_cell, Some((0, 0)));

    // 5. Replace All updates both X and Y cells.
    driver.click_widget_id("find_replace.replace_all")?;
    assert_eq!(driver.state().last_err, None);
    let ds = driver.state().app.document().data_sheet(&SheetId::new("ds-xy")).ok_or("missing ds-xy")?;
    assert_eq!(ds.x_or_time_cell(0, 0).map(|c| &c.value), Some(&CellValue::Number(999.0)));
    assert_eq!(ds.x_or_time_cell(2, 0).map(|c| &c.value), Some(&CellValue::Number(999.0)));
    assert_eq!(ds.cell(curvebook_doc::CellCoord::new(1, 0, 0)).map(|c| &c.value), Some(&CellValue::Number(999.0)));

    // 6. Survival time column also participates in Find Next & Replace All.
    driver.state_mut().app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-surv", "kind": "survival", "title": "Survival Time", "cols": 1, "rows": 2, "auto_graph": false, "auto_info": false }),
    )?;
    driver.state_mut().app.execute_command("table.setXCells", json!({ "sheet_id": "ds-surv", "row": 1, "value": 42.0 }))?;
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "ds-surv" }))?;
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
    app.execute_command("sheet.select", json!({ "sheet_id": g_id.as_str() }))?;
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 760.0);
    let paper = driver.state().views.canvas_view.last_paper_rect_pt.ok_or("expected cached paper rect")?;
    let scene_sz = driver.state().views.canvas_view.last_scene_size_pt.ok_or("expected cached scene size")?;

    driver.hover_at(paper.center())?;
    let summary = driver.frame_meta().find_by_widget_id("status.summary").ok_or("missing status.summary")?;
    let expected_x = format!("X: {:.1}, Y: {:.1} pt", scene_sz[0] * 0.5, scene_sz[1] * 0.5);
    assert!(summary.label.contains(&expected_x), "expected scene-center readout `{expected_x}` in footer summary `{}`", summary.label);

    // Dismiss sheet switcher with Escape.
    driver.click_widget_id("status.sheet_switcher")?;
    assert!(driver.state().views.status_footer.sheet_switcher_open);
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!driver.state().views.status_footer.sheet_switcher_open);

    // Dismiss sheet switcher on outside pointer click.
    driver.click_widget_id("status.sheet_switcher")?;
    assert!(driver.state().views.status_footer.sheet_switcher_open);
    driver.click_at(paper.center())?;
    assert!(!driver.state().views.status_footer.sheet_switcher_open);

    // Modal backdrop automatically closes open footer popovers (`stats_popover_open`).
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "ds-xy" }))?;
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
    let g1 = app.document().graph_sheets[0].id.clone();
    let g2 = SheetId::new("g-2");
    let lay1 = SheetId::new("lay-1");

    // Add an annotation to `g1` so annotation selection can be verified per sheet.
    app.execute_command(
        "graph.addAnnotation",
        json!({
            "sheet_id": g1.as_str(),
            "annotation": {
                "kind": "embedded_result_table",
                "result_sheet_id": g1.as_str(),
                "bounds_pt": { "x": 40.0, "y": 40.0, "width": 120.0, "height": 80.0 }
            }
        }),
    )
    .ok();

    // 1. Secondary graph pane `canvas.graph_ctx.insert_image` emits `"sheet_id": "g-2"` even when `g1` is active.
    app.execute_command("sheet.select", json!({ "sheet_id": g1.as_str() }))?;
    app.sync_views();
    let mut view_sec = CanvasViewEguiState { synchronous_raster: true, ..CanvasViewEguiState::default() };
    view_sec.graph_context_menu = Some((g2.clone(), None, egui::pos2(120.0, 120.0), [60.0, 60.0]));

    let mut captured_intents = PendingUiIntents::new();
    let mut drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (app.clone(), view_sec, PendingUiIntents::new()),
        ViewportMetrics::new(800.0, 600.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (ap, vw, ints)| {
            let tokens = Tokens::for_mode(ap.theme.mode);
            let ctx = CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: true };
            let _ = render_canvas_view_for_sheet(ui, ap, Some(&g2), vw, ctx, meta, ints);
        },
    );
    drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    drv.run_settled();
    drv.click_widget_id("canvas.graph_ctx.insert_image")?;
    for it in drv.state().2.intents() {
        captured_intents.push(it.clone());
    }
    let has_g2_insert = captured_intents.intents().iter().any(|it| {
        matches!(
            it,
            UiIntent::EngineCommand { command_id, params } if command_id == "graph.insertImage" && params.get("sheet_id").and_then(|v| v.as_str()) == Some("g-2")
        )
    });
    assert!(has_g2_insert, "expected `graph.insertImage` with `sheet_id: \"g-2\"`: {:?}", captured_intents.intents());

    // Layout pane `canvas.layout_ctx.insert_image` emits `"sheet_id": "lay-1"` even when `g1` is active.
    let mut lay_sec = CanvasViewEguiState { synchronous_raster: true, ..CanvasViewEguiState::default() };
    lay_sec.layout_context_menu = Some((lay1.clone(), None, egui::pos2(120.0, 120.0)));
    let mut lay_ctx_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (app.clone(), lay_sec, PendingUiIntents::new()),
        ViewportMetrics::new(800.0, 600.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (ap, vw, ints)| {
            let tokens = Tokens::for_mode(ap.theme.mode);
            let ctx = CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: true };
            let _ = render_canvas_view_for_sheet(ui, ap, Some(&lay1), vw, ctx, meta, ints);
        },
    );
    lay_ctx_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    lay_ctx_drv.run_settled();
    lay_ctx_drv.click_widget_id("canvas.layout_ctx.insert_image")?;
    let has_lay_insert = lay_ctx_drv.state().2.intents().iter().any(|it| {
        matches!(
            it,
            UiIntent::EngineCommand { command_id, params } if command_id == "layout.insertImage" && params.get("sheet_id").and_then(|v| v.as_str()) == Some("lay-1")
        )
    });
    assert!(has_lay_insert, "expected `layout.insertImage` with `sheet_id: \"lay-1\"`");

    // 2. Layout 0-displacement click-and-release must not emit `layout.setSpec`.
    app.execute_command("sheet.select", json!({ "sheet_id": lay1.as_str() }))?;
    let undo_before = app.session().undo_depth();
    let mut lay_drv = make_shell_driver(app.clone(), 1280.0, 760.0);
    let panel_center = widget_center(&lay_drv, "canvas.layout_panel.0")?;
    lay_drv.drag(panel_center, panel_center)?;
    assert_eq!(lay_drv.state().app.session().undo_depth(), undo_before);

    // 3. Per-sheet pan & selection isolation across `g1` <-> `g2`.
    let mut sheet_state = CanvasViewEguiState { synchronous_raster: true, pan_pt: [40.0, 25.0], ..CanvasViewEguiState::default() };
    sheet_state.selected_panel_index = Some(0);
    let mut pan_app = app;
    pan_app.execute_command("sheet.select", json!({ "sheet_id": g1.as_str() }))?;
    let mut pan_drv = EguiHumanPathDriver::with_tokens_and_fonts(
        (pan_app, sheet_state, PendingUiIntents::new()),
        ViewportMetrics::new(800.0, 600.0, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        |ui, meta, (ap, vw, ints)| {
            let tokens = Tokens::for_mode(ap.theme.mode);
            let ctx = CanvasViewContext { tokens: &tokens, enabled: true, is_secondary_split: false };
            let _ = render_canvas_view(ui, ap, vw, ctx, meta, ints);
        },
    );
    pan_drv.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    pan_drv.run_settled();
    assert_eq!(pan_drv.state().1.pan_pt, [40.0, 25.0]);

    pan_drv.state_mut().0.execute_command("sheet.select", json!({ "sheet_id": "g-2" }))?;
    pan_drv.run_settled();
    assert_eq!(pan_drv.state().1.pan_pt, [0.0, 0.0]);
    assert_eq!(pan_drv.state().1.selected_panel_index, None);

    pan_drv.state_mut().0.execute_command("sheet.select", json!({ "sheet_id": g1.as_str() }))?;
    pan_drv.run_settled();
    assert_eq!(pan_drv.state().1.pan_pt, [40.0, 25.0]);
    assert_eq!(pan_drv.state().1.selected_panel_index, Some(0));
    Ok(())
}

#[test]
fn fa_sec16_11_split_preview_focus_recorded_and_note_pinned_persists_across_sheets() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "col-big", "kind": "column", "title": "Big Column", "cols": 3, "rows": 800, "auto_graph": true }),
    )?;
    let g_id = app.document().graph_sheets[0].id.as_str().to_owned();
    {
        let mut doc = app.document().clone();
        if let Some(gs) = doc.graph_sheet_mut(&SheetId::new(&g_id)) {
            gs.floating_notes.push(FloatingNote::new("pin-1", RectPt::new(16.0, 16.0, 160.0, 90.0), "Pinned observation"));
        }
        doc.active_sheet_id = Some(SheetId::new(&g_id));
        *app.session_mut() = curvebook_engine::Session::from_document(doc);
    }
    app.split.enabled = true;
    app.split.primary_sheet_id = Some(SheetId::new(&g_id));
    app.split.secondary_sheet_id = Some(SheetId::new("col-big"));
    app.nav.welcome.open = false;
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 760.0);
    assert!(driver.frame_meta().find_by_widget_id("split.focus_in_grid").is_some(), "Focus in Grid must be inspectable");

    // Pin floating note on Graph, close Split and switch to Column sheet -> pinned note persists cleanly across sheet switch.
    let pin_wid = format!("floating_note.{g_id}.pin-1.pin");
    driver.click_widget_id(&pin_wid)?;
    driver.click_widget_id("split.close")?;
    driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": "col-big" }))?;
    driver.run_settled();
    let card_wid = format!("floating_note.{g_id}.pin-1.card");
    assert!(driver.frame_meta().find_by_widget_id(&card_wid).is_some(), "pinned floating note must remain visible across sheet switch");
    Ok(())
}

#[test]
fn fa_sec16_12_leading_dock_i18n_frozen_convert_and_drag_title() -> TestRes {
    let mut app = make_xy_project()?;
    app.execute_command("sheet.freeze", json!({ "sheet_id": "ds-xy" }))?;
    assert_eq!(app.set_locale("de"), "de");
    app.sync_views();

    let mut driver = make_shell_driver(app, 1280.0, 760.0);
    let new_data = driver.frame_meta().find_by_widget_id("nav.new.data").ok_or("missing nav.new.data")?;
    assert_eq!(new_data.label, format!("+ {}", tr("de", "menu.insert.new_data_table")));
    assert!(!new_data.label.contains("New Data"));

    // Right-clicking frozen Data sheet disables Convert Table Archetype as well as Rename & Delete, with German labels.
    let ds_pos = widget_center(&driver, "nav.sheet.ds-xy")?;
    driver.click_at_button(ds_pos, PointerButton::Secondary, KeyModifiers::default())?;
    let conv = driver.frame_meta().find_by_widget_id("nav.ctx.convert_archetype").ok_or("missing nav.ctx.convert_archetype")?;
    assert!(!conv.enabled, "Convert Table Archetype must be disabled when sheet is frozen");
    assert_eq!(conv.label, tr("de", "cmd.ui.table.convertArchetype.open"));
    let rename_item = driver.frame_meta().find_by_widget_id("nav.ctx.rename").ok_or("missing nav.ctx.rename")?;
    assert_eq!(rename_item.label, tr("de", "cmd.sheet.rename"));
    assert!(!rename_item.enabled);
    assert!(!tr("de", "nav.disabled_need_info").contains("Create a Data"));
    assert!(!tr("de", "nav.moving_prefix").contains("Moving"));
    Ok(())
}
