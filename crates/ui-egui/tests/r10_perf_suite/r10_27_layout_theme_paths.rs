#![forbid(unsafe_code)]
#![allow(clippy::needless_pass_by_value)]

//! `R10.27` Human-Path Integration Gate: Layout Panel Picker & Corner Resize (`GWF-15`),
//! Theme Submenu Popup Widget IDs & `Edit > Preferences...` (`GWF-16`), and
//! Inline Editable Grouped/Contingency Row & Column Factor Titles (`table.setTitles`).

use curvebook_doc::{LayoutItem, SheetId, TableArchetype};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::theme::UiThemeMode;
use egui::{Pos2, pos2, vec2};
use serde_json::json;

struct R1027State {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    intent_errors: Vec<String>,
}

impl R1027State {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), intent_errors: Vec::new() }
    }
}

fn make_driver(app: UiApp) -> EguiHumanPathDriver<'static, R1027State> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let fonts = build_driver_font_definitions();
    let metrics = ViewportMetrics::new(1440.0, 820.0, 1.0);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(R1027State::new(app), metrics, tokens, fonts, |ui, meta, st| {
        let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.intent_errors.push(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn widget_center(driver: &EguiHumanPathDriver<'_, R1027State>, widget_id: &str) -> Result<Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    if !entry.clipped_rect.is_positive() {
        return Err(format!("widget `{widget_id}` has non-positive clipped rect: {:?}", entry.clipped_rect));
    }
    Ok(entry.clipped_rect.center())
}

fn assert_clean_frame(driver: &EguiHumanPathDriver<'_, R1027State>, label: &str) -> Result<(), String> {
    if !driver.state().intent_errors.is_empty() {
        return Err(format!("{label}: intent errors: {:?}", driver.state().intent_errors));
    }
    let report = driver.verify_pixel_to_hit_box_parity();
    if !report.is_clean() {
        return Err(format!("{label}: parity/bleed violations: {:?}", report.violations));
    }
    let overlaps = driver.check_scrollbar_non_overlap();
    if !overlaps.is_empty() {
        return Err(format!("{label}: scrollbar overlaps: {overlaps:?}"));
    }
    Ok(())
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
fn gwf15_layout_add_graph_panel_studio_ctx_corner_resize_and_zorder() -> Result<(), String> {
    let mut app = UiApp::new();
    app.egui_views.canvas_view.synchronous_raster = true;
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-1", "title": "Cohort Data", "kind": "column", "cols": 2, "rows": 4 }))
        .map_err(|e| e.to_string())?;
    for (r, v) in [12.0, 15.0, 18.0, 21.0].into_iter().enumerate() {
        app.execute_command("table.setCell", json!({ "sheet_id": "col-1", "row": r, "col": 0, "value": v })).map_err(|e| e.to_string())?;
        app.execute_command("table.setCell", json!({ "sheet_id": "col-1", "row": r, "col": 1, "value": v + 4.0 })).map_err(|e| e.to_string())?;
    }
    app.execute_command("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "col-1", "title": "Dose Graph", "kind": "column" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "col-1", "title": "Summary Graph", "kind": "column" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("layout.create", json!({ "sheet_id": "lay-1", "title": "Figure Layout" })).map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" })).map_err(|e| e.to_string())?;

    let mut driver = make_driver(app);
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref(), Some(&SheetId::from("lay-1")));

    // 1. Add first graph panel via Trailing Studio picker (`studio.layout.add_panel.0`).
    let add_0 = driver.frame_meta().find_by_widget_id("studio.layout.add_panel.0").ok_or("missing studio.layout.add_panel.0")?;
    assert_eq!(add_0.command_id.as_deref(), Some("layout.addPanel"));
    assert!(driver.frame_meta().find_by_widget_id("studio.layout.add_panel.1").is_some(), "missing studio.layout.add_panel.1");
    driver.click_widget_id("studio.layout.add_panel.0")?;
    let (idx0, x0, y0, w0, h0) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?;
    assert_eq!(idx0, 0);

    // 2. Add second graph panel via Layout Canvas right-click context menu (`canvas.layout_ctx.add_panel.1`).
    let vp_rect = driver.frame_meta().find_by_widget_id("canvas.viewport").ok_or("missing canvas.viewport")?.clipped_rect;
    let p0_rect = driver.frame_meta().find_by_widget_id("canvas.layout_panel.0").ok_or("missing canvas.layout_panel.0")?.clipped_rect;
    let empty_pos = if !p0_rect.contains(pos2(vp_rect.right() - 60.0, vp_rect.top() + 60.0)) {
        pos2(vp_rect.right() - 60.0, vp_rect.top() + 60.0)
    } else {
        pos2(vp_rect.right() - 60.0, vp_rect.bottom() - 60.0)
    };
    driver.click_at_button(empty_pos, PointerButton::Secondary, KeyModifiers::default())?;
    let ctx_add_1 = driver.frame_meta().find_by_widget_id("canvas.layout_ctx.add_panel.1").ok_or("missing canvas.layout_ctx.add_panel.1")?;
    assert_eq!(ctx_add_1.z_layer, ZLayer::ContextMenu);
    driver.click_widget_id("canvas.layout_ctx.add_panel.1")?;
    let (idx1, _, _, _, _) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-2")?;
    assert_eq!(idx1, 1);

    // 3. Select panel 0, verify SE corner resize handle appears, drag SE corner, verify 0.1 pt precision + min bounds.
    driver.click_widget_id("canvas.layout_panel.0")?;
    assert_eq!(driver.state().views.canvas_view.selected_panel_index, Some(0));
    let se_entry = driver.frame_meta().find_by_widget_id("canvas.layout_panel.0.resize_se").ok_or("missing canvas.layout_panel.0.resize_se")?;
    assert_eq!(se_entry.z_layer, ZLayer::FloatingOverlay);
    let se_pos = se_entry.clipped_rect.center();
    driver.drag(se_pos, se_pos + vec2(42.0, 28.0))?;
    let (_, x1, y1, w1, h1) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?;
    assert!((x1 - x0).abs() < 1e-6 && (y1 - y0).abs() < 1e-6, "SE corner resize must keep origin fixed");
    assert!(w1 > w0 + 5.0 && h1 > h0 + 5.0, "SE resize must enlarge width ({w0} -> {w1}) and height ({h0} -> {h1})");
    assert!(((w1 * 10.0).round() - w1 * 10.0).abs() < 1e-6, "width {w1} must be rounded to 0.1 pt");
    assert!(((h1 * 10.0).round() - h1 * 10.0).abs() < 1e-6, "height {h1} must be rounded to 0.1 pt");

    // Drag inward aggressively -> clamps cleanly to positive min bounds >= 16.0 pt.
    let se_pos_2 = widget_center(&driver, "canvas.layout_panel.0.resize_se")?;
    driver.drag(se_pos_2, se_pos_2 - vec2(1200.0, 1200.0))?;
    let (_, _, _, w_min, h_min) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?;
    assert!(w_min >= 16.0 && h_min >= 16.0, "corner resize must enforce positive min bounds >= 16.0 pt, got ({w_min}, {h_min})");

    // 4. Verify Z-order controls in Trailing Studio AND Layout Canvas context menu work end-to-end.
    driver.click_widget_id("canvas.layout_panel.0")?;
    driver.click_widget_id("inspector.layout.zorder.front")?;
    let (front_idx, _, _, _, _) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?;
    assert_eq!(front_idx, 1, "Bring to Front in Studio must move panel g-1 to top index 1");

    let p1_center = widget_center(&driver, "canvas.layout_panel.1")?;
    driver.click_at_button(p1_center, PointerButton::Secondary, KeyModifiers::default())?;
    assert!(driver.frame_meta().find_by_widget_id("canvas.layout_ctx.back").is_some(), "context menu must offer Z-order Send to Back");
    driver.click_widget_id("canvas.layout_ctx.back")?;
    let (back_idx, _, _, _, _) = layout_panel_bounds_by_graph(&driver.state().app, "lay-1", "g-1")?;
    assert_eq!(back_idx, 0, "Send to Back in context menu must move panel g-1 back to index 0");

    assert_clean_frame(&driver, "gwf15_layout_panels")
}

#[test]
fn gwf16_theme_submenu_popup_ids_and_edit_preferences_menu() -> Result<(), String> {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "col-1", "title": "Theme Data", "kind": "column", "cols": 2, "rows": 3 }))
        .map_err(|e| e.to_string())?;
    let mut driver = make_driver(app);
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Light);

    // 1. Open View -> Theme submenu and verify all three canonical variant widget IDs exist.
    driver.click_widget_id("menu.view")?;
    let theme_btn = widget_center(&driver, "menu.ui.theme.set")?;
    driver.hover_at(theme_btn)?;
    let light_opt = driver.frame_meta().find_by_widget_id("popup.menu.ui.theme.set.opt.light").ok_or("missing opt.light")?;
    assert_eq!(light_opt.value, Some(json!(true)));
    let dark_opt = driver.frame_meta().find_by_widget_id("popup.menu.ui.theme.set.opt.dark").ok_or("missing opt.dark")?;
    assert_eq!(dark_opt.value, Some(json!(false)));
    let hc_opt = driver.frame_meta().find_by_widget_id("popup.menu.ui.theme.set.opt.high_contrast").ok_or("missing opt.high_contrast")?;
    assert_eq!(hc_opt.value, Some(json!(false)));

    // Also verify CVD preview filter submenu records its variant options cleanly.
    let cvd_btn = widget_center(&driver, "menu.ui.previewFilter.set")?;
    driver.hover_at(cvd_btn)?;
    assert!(driver.frame_meta().find_by_widget_id("popup.menu.ui.previewFilter.set.opt.normal").is_some());
    assert!(driver.frame_meta().find_by_widget_id("popup.menu.ui.previewFilter.set.opt.deuteranopia").is_some());

    // Switch theme to Dark via View > Theme > Dark.
    let theme_btn2 = widget_center(&driver, "menu.ui.theme.set")?;
    driver.hover_at(theme_btn2)?;
    driver.click_widget_id("popup.menu.ui.theme.set.opt.dark")?;
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::Dark, "View > Theme > Dark must switch app theme to Dark");
    let dark_tokens = Tokens::from_theme_spec(&driver.state().app.theme, &driver.state().app.locale);
    driver.set_tokens(dark_tokens);

    // Switch theme to High Contrast via View > Theme > High Contrast.
    driver.click_widget_id("menu.view")?;
    let theme_btn3 = widget_center(&driver, "menu.ui.theme.set")?;
    driver.hover_at(theme_btn3)?;
    driver.click_widget_id("popup.menu.ui.theme.set.opt.high_contrast")?;
    assert_eq!(driver.state().app.theme.mode, UiThemeMode::HighContrast, "View > Theme > High Contrast must switch app theme");
    let hc_tokens = Tokens::from_theme_spec(&driver.state().app.theme, &driver.state().app.locale);
    driver.set_tokens(hc_tokens);

    // 2. Open Edit > Preferences... (`menu.app.preferences`) and verify Preferences modal opens cleanly.
    driver.click_widget_id("menu.edit")?;
    let prefs_item = driver.frame_meta().find_by_widget_id("menu.app.preferences").ok_or("missing menu.app.preferences")?;
    assert!(prefs_item.enabled);
    assert_eq!(prefs_item.command_id.as_deref(), Some("app.preferences"));
    driver.click_widget_id("menu.app.preferences")?;
    assert!(driver.state().app.egui_views.app_modals.preferences_open, "Edit > Preferences... must open Preferences modal");
    assert_clean_frame(&driver, "gwf16_theme_and_preferences_open")?;

    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open, "Cancel must dismiss Preferences modal");
    assert_clean_frame(&driver, "gwf16_after_close")
}

#[test]
fn factor_titles_inline_edit_grouped_and_contingency() -> Result<(), String> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "grp-1", "title": "Grouped Factors", "kind": "grouped", "cols": 2, "rows": 2, "replicates": 2 }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setTitles", json!({ "sheet_id": "grp-1", "row_factor_title": "Dose", "column_factor_title": "Treatment" }))
        .map_err(|e| e.to_string())?;
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ctg-1", "title": "Contingency Factors", "kind": "contingency", "cols": 2, "rows": 2 }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setTitles", json!({ "sheet_id": "ctg-1", "row_factor_title": "Exposure", "column_factor_title": "" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "grp-1" })).map_err(|e| e.to_string())?;

    let mut driver = make_driver(app);

    // 1. Grouped table: double-click Row Factor (`grid.row_titles.band`), Escape cancels, Enter commits.
    let rt_band = driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("missing grid.row_titles.band")?;
    assert_eq!(rt_band.label, "Dose");
    let rt_pos = rt_band.clipped_rect.center();
    driver.click_at(rt_pos)?;
    driver.click_at(rt_pos)?;
    let editor_meta = driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("missing editor meta")?;
    assert_eq!(editor_meta.role, "textbox");
    driver.type_text(" Ignored")?;
    driver.press_key("Escape", KeyModifiers::default())?;
    if let TableArchetype::Grouped(g) = &driver.state().app.document().data_sheet(&SheetId::from("grp-1")).ok_or("missing grp-1")?.archetype {
        assert_eq!(g.row_factor_title, "Dose", "Escape must cancel row factor edit");
    }

    let rt_pos2 = widget_center(&driver, "grid.row_titles.band")?;
    driver.click_at(rt_pos2)?;
    driver.click_at(rt_pos2)?;
    for _ in 0..8 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("Timepoint")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    if let TableArchetype::Grouped(g) = &driver.state().app.document().data_sheet(&SheetId::from("grp-1")).ok_or("missing grp-1")?.archetype {
        assert_eq!(g.row_factor_title, "Timepoint", "Enter must commit row_factor_title via table.setTitles");
    }
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("missing band")?.label, "Timepoint");

    // 2. Grouped table: double-click Column Factor (`grid.col_super.0`), commit "Genotype" -> updates all super-columns.
    let col_super0 = driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("missing grid.col_super.0")?;
    assert_eq!(col_super0.label, "Treatment A");
    let cs_pos = col_super0.clipped_rect.center();
    driver.click_at(cs_pos)?;
    driver.click_at(cs_pos)?;
    for _ in 0..12 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("Genotype")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    if let TableArchetype::Grouped(g) = &driver.state().app.document().data_sheet(&SheetId::from("grp-1")).ok_or("missing grp-1")?.archetype {
        assert_eq!(g.column_factor_title, "Genotype", "Enter must commit column_factor_title via table.setTitles");
    }
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("super 0")?.label, "Genotype A");
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.col_super.1").ok_or("super 1")?.label, "Genotype B");

    // 3. Contingency table: edit blank column_factor_title & row_factor_title cleanly.
    driver.click_widget_id("nav.sheet.ctg-1")?;
    let ctg_super0 = driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("missing ctg grid.col_super.0")?;
    assert_eq!(ctg_super0.label, "Group A");
    let ctg_cs_pos = ctg_super0.clipped_rect.center();
    driver.click_at(ctg_cs_pos)?;
    driver.click_at(ctg_cs_pos)?;
    driver.type_text("Outcome")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    let ctg_rt_pos = widget_center(&driver, "grid.row_titles.band")?;
    driver.click_at(ctg_rt_pos)?;
    driver.click_at(ctg_rt_pos)?;
    for _ in 0..10 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("Cohort")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    if let TableArchetype::Contingency(c) = &driver.state().app.document().data_sheet(&SheetId::from("ctg-1")).ok_or("missing ctg-1")?.archetype {
        assert_eq!(c.column_factor_title, "Outcome");
        assert_eq!(c.row_factor_title, "Cohort");
    }
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.col_super.0").ok_or("ctg super 0")?.label, "Outcome A");
    assert_eq!(driver.frame_meta().find_by_widget_id("grid.row_titles.band").ok_or("ctg band")?.label, "Cohort");

    assert_clean_frame(&driver, "factor_titles_inline_edit")
}
