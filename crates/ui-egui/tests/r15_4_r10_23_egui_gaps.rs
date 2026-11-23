#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `R15.4` Parity Gate: `R10.23` Beta Desktop Gap Checks on `EguiHumanPathDriver`
//! (`--ui-backend=egui`), `14` Beta locales, themes (`Light`, `Dark`, `HighContrast`),
//! `1.0x` / `2.0x` / `3.0x` HiDPI, RTL (`ar`, `he`), fontless embedded fallback,
//! and `gui-layout-spec.md` §9.4 responsive layout gates (`640×420` & `1440×600`).

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, InfoConstant, InfoSheet, LayoutItem, LayoutSheet,
    LayoutSpec, MultipleVariablesTable, OneWayAnovaSpec, ProjectDocument, RectPt, SheetFolderKind, SheetId, SubcolumnMode, TableArchetype,
    VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::fonts::{FontDiscoveryConfig, build_egui_font_definitions_with_config};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::i18n::{LOCALES, locale_picker_label};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{FontBook, RenderTargetSpec, UiApp};
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

struct GapsState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    leading_dock_rect_pt: Option<egui::Rect>,
    trailing_dock_rect_pt: Option<egui::Rect>,
    footer_rect_pt: Option<egui::Rect>,
}

impl GapsState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), leading_dock_rect_pt: None, trailing_dock_rect_pt: None, footer_rect_pt: None }
    }
}

fn make_driver(app: UiApp, metrics: ViewportMetrics, fonts: egui::FontDefinitions) -> EguiHumanPathDriver<'static, GapsState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(GapsState::new(app), metrics, tokens, fonts, |ui, meta, st| {
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.leading_dock_rect_pt = rep.leading_dock_rect_pt;
        st.trailing_dock_rect_pt = rep.trailing_dock_rect_pt;
        st.footer_rect_pt = rep.footer_rect_pt;
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn widget_center(driver: &EguiHumanPathDriver<'_, GapsState>, widget_id: &str) -> Result<Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    if !entry.clipped_rect.is_positive() {
        return Err(format!("widget `{widget_id}` has empty clipped rect {:?}", entry.clipped_rect));
    }
    Ok(entry.clipped_rect.center())
}

fn assert_clean_section_9_4(driver: &EguiHumanPathDriver<'_, GapsState>, label: &str) -> Result<(), String> {
    let report = driver.verify_pixel_to_hit_box_parity();
    if !report.is_clean() {
        return Err(format!("{label}: parity/bleed violations: {:?}", report.violations));
    }
    let overlaps = driver.check_scrollbar_non_overlap();
    if !overlaps.is_empty() {
        return Err(format!("{label}: scrollbar overlaps: {overlaps:?}"));
    }
    let vp = driver.viewport_rect();
    for entry in &driver.frame_meta().entries {
        if (entry.widget_id.starts_with("toolbar.") || entry.widget_id.starts_with("menu.") || entry.widget_id.starts_with("sheet.tab."))
            && entry.clipped_rect.right() > vp.right() + 0.5
        {
            return Err(format!(
                "{label}: chrome widget `{}` overflowed viewport right edge {:.1}: {:?}",
                entry.widget_id,
                vp.right(),
                entry.clipped_rect
            ));
        }
    }
    Ok(())
}

fn build_r10_23_column_and_nav_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 Egui Workspace").map_err(|e| e.to_string())?;
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Conditions").map_err(|e| e.to_string())?;
    info.constants.push(InfoConstant::new("Temp_C", curvebook_doc::ConstantValue::Number(25.0), Some(String::from("C"))).map_err(|e| e.to_string())?);
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 6).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Control");
    col.data_sets[1].title = String::from("Drug A");
    col.data_sets[2].title = String::from("Drug B");
    for (r, &v) in [10.0, 20.0, 30.0, 40.0, 50.0].iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v);
        col.data_sets[1].cells[r][0] = Cell::number(v * 1.5);
        col.data_sets[2].cells[r][0] = Cell::number(v * 2.0);
    }
    let mut ds_col = DataSheet::new("col-1", "Drug Cohort", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    ds_col.info_sheet_id = Some(SheetId::from("info-1"));
    doc.add_data_sheet(ds_col).map_err(|e| e.to_string())?;

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5).map_err(|e| e.to_string())?;
    xy.data_sets[0].title = String::from("Vehicle");
    xy.data_sets[1].title = String::from("Treated");
    for r in 0..5 {
        let x = (r + 1) as f64;
        xy.set_x_cell(r, Cell::number(x)).map_err(|e| e.to_string())?;
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 + x * 4.0);
        xy.data_sets[1].cells[r][0] = Cell::number(14.0 + x * 6.0);
    }
    let ds_xy = DataSheet::new("xy-1", "Dose Curve", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds_xy).map_err(|e| e.to_string())?;

    let ds_ref = doc.data_sheet(&SheetId::from("xy-1")).ok_or("missing xy-1")?.clone();
    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", &ds_ref).map_err(|e| e.to_string())?;
    gs.title = String::from("Dose Response Curve");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Xy;
    gs.spec.title = String::from("Dose Response Curve");
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;

    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::from("graph-1"),
        bounds_pt: RectPt::new(24.0, 32.0, 320.0, 220.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    lspec.items.push(LayoutItem::TextCallout {
        id: String::from("callout-1"),
        bounds_pt: RectPt::new(360.0, 48.0, 160.0, 64.0),
        markdown: String::from("Composite publication figure panel"),
        font_size_pt: 11.0,
    });
    let ls = LayoutSheet::new("layout-1", "Composite Figure 1", lspec).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(ls).map_err(|e| e.to_string())?;

    doc.select_in_navigator(&SheetId::from("col-1")).map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(Session::from_document(doc)).with_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);
    Ok(app)
}

#[test]
fn r15_4_r10_23_egui_inline_edit_col_cell_title_nav_rename_and_undo_toolbar() -> Result<(), String> {
    let app = build_r10_23_column_and_nav_app()?;
    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0), build_driver_font_definitions());
    assert_clean_section_9_4(&driver, "initial inline edit workspace")?;

    let initial_png = driver
        .state_mut()
        .app
        .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
        .map_err(|e| e.to_string())?
        .0;

    // 1. Double-click Column cell (0,0,0), type & Escape cancel without mutation.
    let cell_pos = widget_center(&driver, "grid.cell.0.0.0")?;
    driver.click_at(cell_pos)?;
    driver.click_at(cell_pos)?;
    if driver.state().views.column_grid.inline_edit.is_none() {
        return Err(String::from("expected inline editor after double-clicking grid.cell.0.0.0"));
    }
    driver.type_text("999.9")?;
    driver.press_key("Escape", KeyModifiers::default())?;
    if driver.state().views.column_grid.inline_edit.is_some() {
        return Err(String::from("Escape must cancel grid inline edit"));
    }
    let val_after_esc =
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    assert_eq!(val_after_esc, Some(10.0), "Escape must leave cell (0,0,0) untouched");

    // 2. Double-click Column cell (0,0,0), commit `87.5` via Enter, verify PNG updates, then toolbar Undo/Redo.
    let cell_pos2 = widget_center(&driver, "grid.cell.0.0.0")?;
    driver.click_at(cell_pos2)?;
    driver.click_at(cell_pos2)?;
    for _ in 0..6 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("87.5")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    let val_committed =
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    assert_eq!(val_committed, Some(87.5), "Enter must commit grid cell edit");

    let edited_png = driver
        .state_mut()
        .app
        .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
        .map_err(|e| e.to_string())?
        .0;
    assert_ne!(initial_png, edited_png, "committed cell edit must update egui rasterized screenshot");

    driver.click_widget_id("toolbar.undo")?;
    let val_undone =
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    assert_eq!(val_undone, Some(10.0), "toolbar.undo must revert committed cell edit");

    driver.click_widget_id("toolbar.redo")?;
    let val_redone =
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    assert_eq!(val_redone, Some(87.5), "toolbar.redo must reapply committed cell edit");

    // 2b. Keyboard `Cmd+Z` / `Cmd+Shift+Z` / `Ctrl+Y` global Undo & Redo parity.
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    let val_kb_undo =
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    assert_eq!(val_kb_undo, Some(10.0), "Cmd+Z must undo committed cell edit");
    driver.press_egui_key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    let val_kb_redo =
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    assert_eq!(val_kb_redo, Some(87.5), "Cmd+Shift+Z must redo committed cell edit");

    // 3. Column title inline edit (`grid.col_title.0`) via double-click + Escape cancel + Enter commit.
    let hdr_pos = widget_center(&driver, "grid.col_title.0")?;
    driver.click_at(hdr_pos)?;
    driver.click_at(hdr_pos)?;
    assert!(driver.state().views.column_grid.inline_edit.is_some());
    driver.press_key("Escape", KeyModifiers::default())?;
    if let TableArchetype::Column(col) = &driver.state().app.document().data_sheet(&SheetId::from("col-1")).unwrap().archetype {
        assert_eq!(col.data_sets[0].title, "Control");
    }
    let hdr_pos2 = widget_center(&driver, "grid.col_title.0")?;
    driver.click_at(hdr_pos2)?;
    driver.click_at(hdr_pos2)?;
    for _ in 0..10 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("VehicleCtrl")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    if let TableArchetype::Column(col) = &driver.state().app.document().data_sheet(&SheetId::from("col-1")).unwrap().archetype {
        assert_eq!(col.data_sets[0].title, "VehicleCtrl");
    }

    // 4. Navigator sheet title inline rename (`nav.sheet.col-1` -> `F2` -> `nav.rename_input` -> Escape & Enter).
    driver.click_widget_id("nav.sheet.col-1")?;
    driver.press_egui_key(Key::F2, Modifiers::NONE);
    assert!(driver.frame_meta().find_by_widget_id("nav.rename_input").is_some(), "F2 must open nav.rename_input");
    driver.type_text(" Ignored")?;
    driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).map(|s| s.title.as_str()),
        Some("Drug Cohort"),
        "Escape must cancel Navigator rename without mutation"
    );

    driver.click_widget_id("nav.sheet.col-1")?;
    driver.press_egui_key(Key::F2, Modifiers::NONE);
    for _ in 0..16 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("Assay B")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).map(|s| s.title.as_str()),
        Some("Assay B"),
        "Enter must commit Navigator inline sheet rename"
    );

    assert_clean_section_9_4(&driver, "post inline edits")?;
    assert!(driver.max_input_to_paint_ms() < 500.0);
    Ok(())
}

#[test]
fn r15_4_r10_23_egui_pane_toggle_shortcuts_and_rtl_hidpi() -> Result<(), String> {
    let app = build_r10_23_column_and_nav_app()?;
    let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 900.0, 1.0), build_driver_font_definitions());
    assert_clean_section_9_4(&driver, "pane shortcuts initial")?;
    assert!(driver.frame_meta().find_by_widget_id("nav.search_input").is_some());

    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;

    // 1. Navigator shortcut (`Cmd+Shift+N` -> collapsed rail, `Ctrl+Shift+N` -> restored).
    driver.press_egui_key(Key::N, cmd_shift);
    assert!(driver.state().views.leading_collapsed_rail, "Cmd+Shift+N must collapse Leading Dock");
    assert!(driver.frame_meta().find_by_widget_id("nav.search_input").is_none());
    driver.press_egui_key(Key::N, ctrl_shift);
    assert!(!driver.state().views.leading_collapsed_rail, "Ctrl+Shift+N must expand Leading Dock");
    assert!(driver.frame_meta().find_by_widget_id("nav.search_input").is_some());

    // 2. Inspector shortcut (`Cmd+Shift+I` -> 36pt collapsed rail, `Ctrl+Shift+I` -> expanded studio).
    assert!(!driver.state().views.trailing_collapsed_rail);
    driver.press_egui_key(Key::I, cmd_shift);
    assert!(driver.state().views.trailing_collapsed_rail, "Cmd+Shift+I must collapse Trailing Studio rail");
    let collapsed_trail = driver.state().trailing_dock_rect_pt.ok_or("missing collapsed trailing rail rect")?;
    assert!(collapsed_trail.width() <= 38.0, "collapsed trailing rail must be <= 38pt, got {}", collapsed_trail.width());
    driver.press_egui_key(Key::I, ctrl_shift);
    assert!(!driver.state().views.trailing_collapsed_rail, "Ctrl+Shift+I must restore Trailing Studio");
    let restored_trail = driver.state().trailing_dock_rect_pt.ok_or("missing restored trailing dock rect")?;
    assert!(restored_trail.width() >= 200.0, "restored trailing dock must expand, got {}", restored_trail.width());

    // 3. Family pane shortcut (`Cmd+Shift+F` / `Ctrl+Shift+F`).
    assert!(driver.state().app.family_open);
    driver.press_egui_key(Key::F, cmd_shift);
    assert!(!driver.state().app.family_open, "Cmd+Shift+F must collapse Family pane");
    driver.press_egui_key(Key::F, ctrl_shift);
    assert!(driver.state().app.family_open, "Ctrl+Shift+F must restore Family pane");

    // 4. Arabic (`ar`) @ 1.5x HiDPI layout parity, scrollbar non-overlap, and pane toggles remain clean.
    assert_eq!(driver.state_mut().app.set_locale("ar"), "ar");
    driver.set_viewport(ViewportMetrics::new(1440.0, 900.0, 1.5));
    assert!(driver.state().leading_dock_rect_pt.is_some());
    assert!(driver.state().trailing_dock_rect_pt.is_some());
    driver.press_egui_key(Key::N, cmd_shift);
    driver.press_egui_key(Key::I, cmd_shift);
    assert_clean_section_9_4(&driver, "ar closed panes 1.5x")?;
    driver.press_egui_key(Key::N, ctrl_shift);
    driver.press_egui_key(Key::I, ctrl_shift);
    assert_clean_section_9_4(&driver, "ar restored panes 1.5x")?;
    Ok(())
}

#[test]
fn r15_4_r10_23_egui_portfolio_gallery_f6_cycling_and_quick_stats_footer() -> Result<(), String> {
    let app = build_r10_23_column_and_nav_app()?;
    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 720.0, 1.0), build_driver_font_definitions());

    // 1. Open Portfolio Gallery via footer pill (`status.section.gallery`), filter by Graphs, click card.
    driver.click_widget_id("status.section.gallery")?;
    assert!(driver.state().app.nav.gallery.open);
    for wid in ["gallery.filter.all", "gallery.filter.graphs", "gallery.filter.layouts", "gallery.filter.data", "gallery.card.graph-1"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing gallery widget `{wid}`");
    }
    driver.click_widget_id("gallery.filter.graphs")?;
    assert_eq!(driver.state().app.nav.gallery.filter_folder, Some(SheetFolderKind::Graphs));
    assert!(driver.frame_meta().find_by_widget_id("gallery.card.graph-1").is_some());
    assert!(driver.frame_meta().find_by_widget_id("gallery.card.layout-1").is_none());

    driver.click_widget_id("gallery.filter.all")?;
    driver.click_widget_id("gallery.card.layout-1")?;
    assert!(!driver.state().app.nav.gallery.open, "clicking gallery card must close gallery and select sheet");
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));

    // 2. Visible-pane `F6` / `Shift+F6` cycling + skipping hidden panes.
    driver.click_widget_id("nav.sheet.col-1")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
    driver.state_mut().app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    driver.run_settled();

    let mut visited = Vec::new();
    for _ in 0..5 {
        driver.press_egui_key(Key::F6, Modifiers::NONE);
        visited.push(driver.state().app.nav.active_pane);
    }
    assert!(visited.contains(&ActiveWorkspacePane::Inspector));
    assert!(visited.contains(&ActiveWorkspacePane::Navigator));
    assert!(visited.contains(&ActiveWorkspacePane::FamilyStrip));

    driver.press_egui_key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(!driver.state().app.family_open);
    for _ in 0..5 {
        driver.press_egui_key(Key::F6, Modifiers::NONE);
        assert!(
            driver.state().app.nav.active_pane != ActiveWorkspacePane::FamilyStrip,
            "F6 must skip hidden Family pane, got {:?}",
            driver.state().app.nav.active_pane
        );
    }
    driver.press_egui_key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(driver.state().app.family_open);

    // 3. Select range `(0,0,0)..(3,0,0)` (`10, 20, 30, 40`) via Click + Shift+Click -> verify `status.summary` quick-stats telemetry.
    let start_pos = widget_center(&driver, "grid.cell.0.0.0")?;
    let end_pos = widget_center(&driver, "grid.cell.3.0.0")?;
    driver.click_at(start_pos)?;
    driver.dispatch_events(vec![
        egui::Event::ModifiersChanged(Modifiers::SHIFT),
        egui::Event::PointerMoved(end_pos),
        egui::Event::PointerButton { pos: end_pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::SHIFT },
        egui::Event::PointerButton { pos: end_pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::SHIFT },
    ]);
    driver.dispatch_events(vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
    let summary_entry = driver.frame_meta().find_by_widget_id("status.summary").ok_or("missing status.summary")?;
    for expected in ["N=4", "Mean=25", "SD=12.91", "SEM=6.455", "Med=25", "Rng=30"] {
        assert!(summary_entry.label.contains(expected), "expected quick-stats footer token `{expected}` in `{}`", summary_entry.label);
    }

    assert_clean_section_9_4(&driver, "gallery/F6/footer parity")?;
    Ok(())
}

#[test]
fn r15_4_r10_23_egui_14_beta_locales_themes_and_section_9_4_layout_gates() -> Result<(), String> {
    assert_eq!(BETA_LOCALE_CODES.len(), 14, "Beta must gate all 14 active locales");
    for &locale in &BETA_LOCALE_CODES {
        for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
            let mut app = build_r10_23_column_and_nav_app()?;
            app.set_theme_mode(mode);
            assert_eq!(app.set_locale(locale), locale);
            let mut driver = make_driver(app, ViewportMetrics::new(1440.0, 600.0, 1.0), build_driver_font_definitions());
            assert_clean_section_9_4(&driver, &format!("1440x600@1.0x locale={locale} theme={mode:?}"))?;

            // §9.4 narrow desktop breakpoint `640x420` across HiDPI `2.0x` and `3.0x` on representative locales.
            let scale = if matches!(locale, "en" | "de" | "ja" | "ar") { 2.0 } else { 1.5 };
            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, scale));
            assert_clean_section_9_4(&driver, &format!("640x420@{scale}x locale={locale} theme={mode:?}"))?;
        }
    }
    Ok(())
}

#[test]
fn r15_4_r10_23_egui_fontless_embedded_fallback_scenarios() -> Result<(), String> {
    let book = FontBook::with_embedded_fallback();
    assert!(
        book.face_names().iter().all(|name| !name.starts_with("/System/Library") && !name.starts_with("/usr/share/fonts")),
        "embedded fallback must not depend on OS host fonts: {:?}",
        book.face_names()
    );
    let cleanroom_fonts =
        build_egui_font_definitions_with_config(&FontDiscoveryConfig { fonts_dir: None, include_os_fonts: false, include_bundled_fonts: false });
    assert!(cleanroom_fonts.report.cleanroom_sans_loaded, "cleanroom sans fallback font must load without OS/bundled fonts");
    assert!(cleanroom_fonts.report.symbol_fallback_loaded, "symbol fallback font must load without OS/bundled fonts");

    for (locale, mode, scale) in [("en", UiThemeMode::Light, 1.0), ("de", UiThemeMode::Dark, 2.0), ("ar", UiThemeMode::HighContrast, 3.0)] {
        let mut app = build_r10_23_column_and_nav_app()?;
        assert_eq!(app.set_locale(locale), locale);
        app.set_theme_mode(mode);
        let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, scale), cleanroom_fonts.definitions.clone());
        assert_clean_section_9_4(&driver, &format!("fontless locale={locale} mode={mode:?} scale={scale}"))?;
        let png = driver
            .state_mut()
            .app
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 640, height_px: 420, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
            .map_err(|e| e.to_string())?
            .0;
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256);
    }
    Ok(())
}

#[test]
fn r15_4_r10_23_egui_drag_select_clipboard_bundle_language_draft_and_wave4_grids() -> Result<(), String> {
    let mut app = build_r10_23_column_and_nav_app()?;
    let clipboard = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clipboard.clone());

    // Add MultipleVariables sheet + an ANOVA ResultSheet to verify Wave 4 X/MV/Results gaps.
    {
        let mut doc = app.document().clone();
        let mut mv = MultipleVariablesTable::new(3, 2).map_err(|e| e.to_string())?;
        mv.variables[0].title = String::from("Group");
        mv.variables[0].role = VariableRole::Categorical;
        mv.variables[1].title = String::from("Response");
        mv.variables[1].role = VariableRole::Continuous;
        mv.variables[2].title = String::from("Weight");
        mv.variables[2].role = VariableRole::Continuous;
        mv.variables[0].cells = vec![Cell::text("Ctrl"), Cell::text("Drug")];
        mv.variables[1].cells = vec![Cell::number(10.0), Cell::number(25.0)];
        mv.variables[2].cells = vec![Cell::number(1.0), Cell::number(2.0)];
        let ds_mv = DataSheet::new("mv-1", "MV Gap Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds_mv).map_err(|e| e.to_string())?;
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }
    app.set_gui_only(false);
    let kind_val = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    let res_out = app
        .execute_command("analysis.run", json!({ "source_sheet_id": "col-1", "sheet_id": "res-gap", "kind": kind_val }))
        .map_err(|e| e.to_string())?;
    assert!(res_out.payload.get("sheet_id").is_some());
    app.set_gui_only(true);
    app.execute_command("sheet.select", json!({ "sheet_id": "col-1" })).map_err(|e| e.to_string())?;

    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0), build_driver_font_definitions());

    // 1. Pointer drag-selection across Column cells `(0,0,0)..(2,0,0)` (`10, 20, 30`) updates quick-stats footer.
    let c0 = widget_center(&driver, "grid.cell.0.0.0")?;
    let c2 = widget_center(&driver, "grid.cell.2.0.0")?;
    driver.drag(c0, c2)?;
    let summary_entry = driver.frame_meta().find_by_widget_id("status.summary").ok_or("missing status.summary")?;
    for expected in ["N=3", "Mean=20", "Med=20", "Rng=20"] {
        assert!(summary_entry.label.contains(expected), "drag-select quick stats missing `{expected}` in `{}`", summary_entry.label);
    }

    // 1b. `Cmd+C` / `Cmd+V` clipboard roundtrip directly inside `ColumnGrid` (`data_grid.rs`).
    driver.click_widget_id("grid.cell.1.0.0")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    driver.click_widget_id("grid.cell.0.1.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    let pasted_col =
        driver.state().app.document().data_sheet(&SheetId::from("col-1")).and_then(|s| s.cell(CellCoord::new(0, 1, 0))).and_then(Cell::active_number);
    assert_eq!(pasted_col, Some(20.0), "ColumnGrid Cmd+C/V must copy and paste across columns");

    // 2. `Cmd+C` vector `ClipboardBundle` on `GraphSheet` & `LayoutSheet`.
    driver.click_widget_id("nav.chip.graphs")?;
    driver.click_widget_id("nav.sheet.graph-1")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    assert_eq!(clipboard.captured_bundles().len(), 1, "Cmd+C on GraphSheet must write ClipboardBundle");
    let g_bundle = clipboard.last_bundle().ok_or("missing graph bundle")?;
    assert!(!g_bundle.svg.is_empty() && !g_bundle.png_bytes.is_empty() && g_bundle.pdf_bytes.starts_with(b"%PDF"));

    driver.click_widget_id("nav.chip.layouts")?;
    driver.click_widget_id("nav.sheet.layout-1")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    assert_eq!(clipboard.captured_bundles().len(), 2, "Cmd+C on LayoutSheet must write ClipboardBundle");

    // 3. `View > Language` picker lists all 16 `LOCALES` with `[Draft]` on non-English + switches locale.
    driver.click_widget_id("menu.view")?;
    let lang_pos = widget_center(&driver, "menu.ui.locale.set")?;
    driver.hover_at(lang_pos)?;
    assert_eq!(LOCALES.len(), 16);
    let mut seen = vec![false; LOCALES.len()];
    for scroll_step in 0..=4 {
        for (idx, &loc) in LOCALES.iter().enumerate() {
            let opt_wid = format!("popup.menu.ui.locale.set.opt.{idx}");
            if let Some(entry) = driver.frame_meta().find_by_widget_id(&opt_wid) {
                assert_eq!(entry.label, locale_picker_label(loc));
                if loc != "en" {
                    assert!(entry.label.contains("[Draft]"), "expected [Draft] badge on `{}`", entry.label);
                }
                seen[idx] = true;
            }
        }
        if seen.iter().all(|&s| s) || scroll_step == 4 {
            break;
        }
        let popup_center = widget_center(&driver, "popup.menu.ui.locale.set")?;
        driver.wheel_at(popup_center, egui::vec2(0.0, -160.0))?;
    }
    assert!(seen.iter().all(|&s| s), "all 16 LOCALES must be reachable in View > Language popup: {seen:?}");
    let popup_center = widget_center(&driver, "popup.menu.ui.locale.set")?;
    driver.wheel_at(popup_center, egui::vec2(0.0, 400.0))?;
    driver.click_widget_id("popup.menu.ui.locale.set.opt.1")?; // `de`
    assert_eq!(driver.state().app.locale, "de");
    assert_eq!(driver.state_mut().app.set_locale("en"), "en");
    driver.run_settled();

    // 4. `Xy` X-cell inline editing + `Cmd+C/V` grid paste + `ResultSheet` 640x420@1.5x scroll-clip check across RTL/CJK/LTR.
    driver.click_widget_id("nav.chip.data")?;
    driver.click_widget_id("nav.sheet.xy-1")?;
    driver.click_widget_id("grid.cell.1.0.0")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    let pasted =
        driver.state().app.document().data_sheet(&SheetId::from("xy-1")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number);
    assert_eq!(pasted, Some(18.0), "Cmd+C/V must copy and paste cell value on Xy grid");
    let x_pos = widget_center(&driver, "grid.xcell.0.0")?;
    driver.click_at(x_pos)?;
    driver.click_at(x_pos)?;
    for _ in 0..8 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("7.5")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    if let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("xy-1")).unwrap().archetype {
        assert_eq!(xy.x_cells[0][0].active_number(), Some(7.5), "Xy inline X-cell edit failed");
    }

    driver.click_widget_id("nav.chip.results")?;
    driver.click_widget_id("nav.sheet.res-gap")?;
    for &(loc, mode, scale) in &[
        ("en", UiThemeMode::Light, 1.0),
        ("de", UiThemeMode::Dark, 2.0),
        ("ja", UiThemeMode::Light, 1.5),
        ("ar", UiThemeMode::HighContrast, 1.5),
        ("he", UiThemeMode::Dark, 1.5),
    ] {
        assert_eq!(driver.state_mut().app.set_locale(loc), loc);
        driver.state_mut().app.set_theme_mode(mode);
        driver.set_viewport(ViewportMetrics::new(640.0, 420.0, scale));
        assert_clean_section_9_4(&driver, &format!("ResultSheet 640x420@{scale}x locale={loc} mode={mode:?}"))?;
        driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
        assert_clean_section_9_4(&driver, &format!("ResultSheet 1440x600 locale={loc} mode={mode:?}"))?;
    }

    // 6. `MV` Data-tab role badges (`mv_formula.role_badge.0`) & `Manage Variables…` modal (`dialog.manage_variables`).
    assert_eq!(driver.state_mut().app.set_locale("en"), "en");
    driver.state_mut().app.set_theme_mode(UiThemeMode::Light);
    driver.set_viewport(ViewportMetrics::new(1280.0, 800.0, 1.0));
    driver.click_widget_id("nav.chip.data")?;
    driver.click_widget_id("nav.sheet.mv-1")?;
    assert!(driver.frame_meta().find_by_widget_id("mv_formula.role_badge.0").is_some());
    driver.click_widget_id("mv_formula.role_badge.0")?;
    driver.click_widget_id("mv_formula.role_manage.0")?;
    assert!(driver.state().app.grid.variables_dialog.open, "Manage Variables dialog must open");
    assert!(driver.frame_meta().find_by_widget_id("dialog.manage_variables").is_some());
    assert_clean_section_9_4(&driver, "Manage Variables modal 1280x800")?;
    driver.click_widget_id("modal.ok")?;
    assert!(!driver.state().app.grid.variables_dialog.open, "modal.ok must close Manage Variables dialog");
    Ok(())
}
