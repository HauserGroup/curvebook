#![forbid(unsafe_code)]

//! `R10.27` Human-Path Integration Gate: Layout Panel Picker & Corner Resize (`GWF-15`),
//! Theme Submenu Popup Widget IDs & `Edit > Preferences...` (`GWF-16`), and
//! Inline Editable Grouped/Contingency Row & Column Factor Titles (`table.setTitles`).

use crate::common::{ShellTestState, TestRes, assert_clean_parity, ensure, widget_center, widget_rect};
use crate::suite_helpers::{assert_widgets, inline_edit_widget, make_sync_driver, run_cmd};
use curvebook_doc::{LayoutItem, SheetId, TableArchetype};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton};
use curvebook_ui_egui::theme::UiThemeMode;
use egui::{pos2, vec2};
use serde_json::json;

fn assert_clean_frame(driver: &EguiHumanPathDriver<'_, ShellTestState>, label: &str) -> TestRes {
    ensure(driver.state().last_intent_error.is_none(), format!("{label}: intent error: {:?}", driver.state().last_intent_error))?;
    assert_clean_parity(driver, label)
}

fn layout_panel_bounds_by_graph(app: &UiApp, layout_id: &str, graph_id: &str) -> Result<(usize, f64, f64, f64, f64), String> {
    let ls = app.document().layout_sheet(&SheetId::from(layout_id)).ok_or("missing layout sheet")?;
    for (idx, item) in ls.spec.items.iter().enumerate() {
        if let LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, .. } = item
            && graph_sheet_id.as_str() == graph_id
        {
            return Ok((idx, bounds_pt.x, bounds_pt.y, bounds_pt.width, bounds_pt.height));
        }
    }
    Err(format!("missing LiveGraph panel for `{graph_id}` on `{layout_id}`"))
}

#[test]
fn gwf15_layout_add_graph_panel_studio_ctx_corner_resize_and_zorder() -> TestRes {
    let mut app = UiApp::new();
    run_cmd(&mut app, "sheet.createData", json!({ "sheet_id": "col-1", "title": "Cohort Data", "kind": "column", "cols": 2, "rows": 4 }))?;
    for (r, v) in [12.0, 15.0, 18.0, 21.0].into_iter().enumerate() {
        run_cmd(&mut app, "table.setCell", json!({ "sheet_id": "col-1", "row": r, "col": 0, "value": v }))?;
        run_cmd(&mut app, "table.setCell", json!({ "sheet_id": "col-1", "row": r, "col": 1, "value": v + 4.0 }))?;
    }
    for (sid, title) in [("g-1", "Dose Graph"), ("g-2", "Summary Graph")] {
        run_cmd(&mut app, "graph.create", json!({ "sheet_id": sid, "data_sheet_id": "col-1", "title": title, "kind": "column" }))?;
    }
    run_cmd(&mut app, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure Layout" }))?;
    run_cmd(&mut app, "sheet.select", json!({ "sheet_id": "lay-1" }))?;

    let mut driver = make_sync_driver(app, 1440.0, 820.0);
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref(), Some(&SheetId::from("lay-1")));

    // 1. Add first graph panel via Trailing Studio picker (`studio.layout.add_panel.0`).
    let add_0 = driver.frame_meta().find_by_widget_id("studio.layout.add_panel.0").ok_or("missing studio.layout.add_panel.0")?;
    assert_eq!(add_0.command_id.as_deref(), Some("layout.addPanel"));
    assert_widgets(&driver, &["studio.layout.add_panel.1"])?;
    driver.click_widget_id("studio.layout.add_panel.0")?;
    let (idx0, x0, y0, w0, h0) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?;
    assert_eq!(idx0, 0);

    // 2. Add second graph panel via Layout Canvas right-click context menu (`canvas.layout_ctx.add_panel.1`).
    let (vp_rect, p0_rect) = (widget_rect(&driver, "canvas.viewport")?, widget_rect(&driver, "canvas.layout_panel.0")?);
    let cand = pos2(vp_rect.right() - 60.0, vp_rect.top() + 60.0);
    let empty_pos = if !p0_rect.contains(cand) { cand } else { pos2(vp_rect.right() - 60.0, vp_rect.bottom() - 60.0) };
    driver.click_at_button(empty_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert_eq!(driver.frame_meta().find_by_widget_id("canvas.layout_ctx.add_panel.1").ok_or("missing ctx add_panel.1")?.z_layer, ZLayer::ContextMenu);
    driver.click_widget_id("canvas.layout_ctx.add_panel.1")?;
    assert_eq!(layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-2")?.0, 1);

    // 3. Select panel 0 -> SE corner resize handle -> drag enlarge (0.1 pt precision) -> drag inward clamp >= 16 pt.
    driver.click_widget_id("canvas.layout_panel.0")?;
    assert_eq!(driver.state().views.canvas_view.selected_panel_index, Some(0));
    let se_entry = driver.frame_meta().find_by_widget_id("canvas.layout_panel.0.resize_se").ok_or("missing resize_se")?.clone();
    assert_eq!(se_entry.z_layer, ZLayer::FloatingOverlay);
    let se_pos = se_entry.clipped_rect.center();
    driver.drag(se_pos, se_pos + vec2(42.0, 28.0))?;
    let (_, x1, y1, w1, h1) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?;
    assert!((x1 - x0).abs() < 1e-6 && (y1 - y0).abs() < 1e-6 && w1 > w0 + 5.0 && h1 > h0 + 5.0);
    assert!(((w1 * 10.0).round() - w1 * 10.0).abs() < 1e-6 && ((h1 * 10.0).round() - h1 * 10.0).abs() < 1e-6);

    let se_pos_2 = widget_center(&driver, "canvas.layout_panel.0.resize_se")?;
    driver.drag(se_pos_2, se_pos_2 - vec2(1200.0, 1200.0))?;
    let (_, _, _, w_min, h_min) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?;
    assert!(w_min >= 16.0 && h_min >= 16.0, "got ({w_min}, {h_min})");

    // 4. Z-order controls in Trailing Studio AND Layout Canvas context menu.
    driver.click_widget_id("canvas.layout_panel.0")?;
    driver.click_widget_id("inspector.layout.zorder.front")?;
    assert_eq!(layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?.0, 1);

    let p1_center = widget_center(&driver, "canvas.layout_panel.1")?;
    driver.click_at_button(p1_center, PointerButton::Secondary, KeyModifiers::default())?;
    assert_widgets(&driver, &["canvas.layout_ctx.back"])?;
    driver.click_widget_id("canvas.layout_ctx.back")?;
    assert_eq!(layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?.0, 0);
    assert_clean_frame(&driver, "gwf15_layout_panels")
}

#[test]
fn gwf16_theme_submenu_popup_ids_and_edit_preferences_menu() -> TestRes {
    let mut app = UiApp::new();
    run_cmd(&mut app, "sheet.createData", json!({ "sheet_id": "col-1", "title": "Theme Data", "kind": "column", "cols": 2, "rows": 3 }))?;
    let mut driver = make_sync_driver(app, 1440.0, 820.0);
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Light);

    driver.click_widget_id("menu.view")?;
    driver.hover_at(widget_center(&driver, "menu.ui.theme.set")?)?;
    for (wid, active) in [
        ("popup.menu.ui.theme.set.opt.light", true),
        ("popup.menu.ui.theme.set.opt.dark", false),
        ("popup.menu.ui.theme.set.opt.high_contrast", false),
    ] {
        assert_eq!(driver.frame_meta().find_by_widget_id(wid).ok_or(wid)?.value, Some(json!(active)));
    }
    driver.hover_at(widget_center(&driver, "menu.ui.previewFilter.set")?)?;
    assert_widgets(&driver, &["popup.menu.ui.previewFilter.set.opt.normal", "popup.menu.ui.previewFilter.set.opt.deuteranopia"])?;

    for (opt, mode, reopen_view) in [
        ("popup.menu.ui.theme.set.opt.dark", UiThemeMode::Dark, false),
        ("popup.menu.ui.theme.set.opt.high_contrast", UiThemeMode::HighContrast, true),
    ] {
        if reopen_view {
            driver.click_widget_id("menu.view")?;
        }
        driver.hover_at(widget_center(&driver, "menu.ui.theme.set")?)?;
        driver.click_widget_id(opt)?;
        assert_eq!(driver.state().app.theme.mode, mode);
        let tokens = Tokens::from_theme_spec(&driver.state().app.theme, &driver.state().app.locale);
        driver.set_tokens(tokens);
    }

    driver.click_widget_id("menu.edit")?;
    let prefs = driver.frame_meta().find_by_widget_id("menu.app.preferences").ok_or("missing menu.app.preferences")?;
    assert!(prefs.enabled && prefs.command_id.as_deref() == Some("app.preferences"));
    driver.click_widget_id("menu.app.preferences")?;
    assert!(driver.state().app.egui_views.app_modals.preferences_open);
    assert_clean_frame(&driver, "gwf16_theme_and_preferences_open")?;
    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_clean_frame(&driver, "gwf16_after_close")
}

#[test]
fn factor_titles_inline_edit_grouped_and_contingency() -> TestRes {
    let mut app = UiApp::new();
    run_cmd(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "grp-1", "title": "Grouped Factors", "kind": "grouped", "cols": 2, "rows": 2, "replicates": 2 }),
    )?;
    run_cmd(&mut app, "table.setTitles", json!({ "sheet_id": "grp-1", "row_factor_title": "Dose", "column_factor_title": "Treatment" }))?;
    run_cmd(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "ctg-1", "title": "Contingency Factors", "kind": "contingency", "cols": 2, "rows": 2 }),
    )?;
    run_cmd(&mut app, "table.setTitles", json!({ "sheet_id": "ctg-1", "row_factor_title": "Exposure", "column_factor_title": "" }))?;
    run_cmd(&mut app, "sheet.select", json!({ "sheet_id": "grp-1" }))?;

    let mut driver = make_sync_driver(app, 1440.0, 820.0);
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("band")?.label, "Dose");

    // 1. Grouped: double-click Row Factor opens textbox editor; Escape cancels; Enter commits row & column factor titles.
    crate::suite_helpers::dbl_click_widget(&mut driver, "grid.row_titles.band")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("editor meta")?.role, "textbox");
    driver.type_text(" Ignored")?;
    driver.press_key("Escape", KeyModifiers::default())?;
    if let TableArchetype::Grouped(g) = &driver.state().app.document().data_sheet(&SheetId::from("grp-1")).ok_or("grp-1")?.archetype {
        assert_eq!(g.row_factor_title, "Dose");
    }
    inline_edit_widget(&mut driver, "grid.row_titles.band", "Timepoint", "Enter")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("col_super.0")?.label, "Treatment A");
    inline_edit_widget(&mut driver, "grid.col_super.0", "Genotype", "Enter")?;
    if let TableArchetype::Grouped(g) = &driver.state().app.document().data_sheet(&SheetId::from("grp-1")).ok_or("grp-1")?.archetype {
        assert_eq!((g.row_factor_title.as_str(), g.column_factor_title.as_str()), ("Timepoint", "Genotype"));
    }
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("band")?.label, "Timepoint");
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("super 0")?.label, "Genotype A");
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.col_super.1").ok_or("super 1")?.label, "Genotype B");

    // 2. Contingency: edit blank column_factor_title & row_factor_title.
    driver.click_widget_id("nav.sheet.ctg-1")?;
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("ctg super 0")?.label, "Group A");
    inline_edit_widget(&mut driver, "grid.col_super.0", "Outcome", "Enter")?;
    inline_edit_widget(&mut driver, "grid.row_titles.band", "Cohort", "Enter")?;
    if let TableArchetype::Contingency(c) = &driver.state().app.document().data_sheet(&SheetId::from("ctg-1")).ok_or("ctg-1")?.archetype {
        assert_eq!((c.column_factor_title.as_str(), c.row_factor_title.as_str()), ("Outcome", "Cohort"));
    }
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("ctg super 0")?.label, "Outcome A");
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("ctg band")?.label, "Cohort");
    assert_clean_frame(&driver, "factor_titles_inline_edit")
}
