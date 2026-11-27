#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `R15.4` Parity Gate: `R10.23` Beta Desktop Gap Checks on `EguiHumanPathDriver`
//! (`--ui-backend=egui`), `14` Beta locales, themes (`Light`, `Dark`, `HighContrast`),
//! `1.0x` / `2.0x` / `3.0x` HiDPI, RTL (`ar`, `he`), fontless embedded fallback,
//! and `gui-layout-spec.md` §9.4 responsive layout gates (`640×420` & `1440×600`).

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, InfoConstant, InfoSheet, LayoutItem, LayoutSheet,
    LayoutSpec, MultipleVariablesTable, OneWayAnovaSpec, ProjectDocument, RectPt, SheetFolderKind, SheetId, SubcolumnMode, TableArchetype,
    VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::driver::{BETA_LOCALE_CODES, EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::fonts::{FontDiscoveryConfig, build_egui_font_definitions_with_config};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::i18n::{LOCALES, locale_picker_label};
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{FontBook, RenderTargetSpec, UiApp};
use egui::{Key, Modifiers};
use serde_json::json;

use crate::common::{PNG_SIGNATURE, ShellTestState, TestRes, assert_clean_parity, cell_number, fill_col_values, make_shell_driver, widget_center};

fn make_driver_with_fonts(app: UiApp, metrics: ViewportMetrics, fonts: egui::FontDefinitions) -> EguiHumanPathDriver<'static, ShellTestState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(ShellTestState::new(app), metrics, tokens, fonts, |ui, meta, st| {
        let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        st.leading_rect_pt = rep.leading_dock_rect_pt;
        st.trailing_rect_pt = rep.trailing_dock_rect_pt;
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

fn assert_clean_section_9_4(driver: &EguiHumanPathDriver<'_, ShellTestState>, label: &str) -> TestRes {
    assert_clean_parity(driver, label)?;
    let vp = driver.viewport_rect();
    for entry in &driver.frame_meta().entries {
        if (entry.widget_id.starts_with("toolbar.") || entry.widget_id.starts_with("menu.")) && entry.clipped_rect.right() > vp.right() + 0.5 {
            return Err(format!("{label}: chrome widget `{}` overflowed {:.1}: {:?}", entry.widget_id, vp.right(), entry.clipped_rect));
        }
    }
    Ok(())
}

fn col1_cell(driver: &EguiHumanPathDriver<'_, ShellTestState>, row: usize, col: usize) -> Option<f64> {
    driver.state().app.document().data_sheet(&SheetId::from("col-1")).and_then(|s| cell_number(s, row, col))
}

fn build_r10_23_column_and_nav_app() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 Egui Workspace").map_err(|e| e.to_string())?;
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Conditions").map_err(|e| e.to_string())?;
    info.constants.push(InfoConstant::new("Temp_C", curvebook_doc::ConstantValue::Number(25.0), Some(String::from("C"))).map_err(|e| e.to_string())?);
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 6).map_err(|e| e.to_string())?;
    fill_col_values(&mut col.data_sets[0], "Control", &[10.0, 20.0, 30.0, 40.0, 50.0]);
    fill_col_values(&mut col.data_sets[1], "Drug A", &[15.0, 30.0, 45.0, 60.0, 75.0]);
    fill_col_values(&mut col.data_sets[2], "Drug B", &[20.0, 40.0, 60.0, 80.0, 100.0]);
    let mut ds_col = DataSheet::new("col-1", "Drug Cohort", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    ds_col.info_sheet_id = Some(SheetId::from("info-1"));
    doc.add_data_sheet(ds_col).map_err(|e| e.to_string())?;

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5).map_err(|e| e.to_string())?;
    for r in 0..5 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).map_err(|e| e.to_string())?;
    }
    fill_col_values(&mut xy.data_sets[0], "Vehicle", &[14.0, 18.0, 22.0, 26.0, 30.0]);
    fill_col_values(&mut xy.data_sets[1], "Treated", &[20.0, 26.0, 32.0, 38.0, 44.0]);
    doc.add_data_sheet(DataSheet::new("xy-1", "Dose Curve", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

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
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Composite Figure 1", lspec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("col-1")).map_err(|e| e.to_string())?;
    Ok(UiApp::from_session(Session::from_document(doc)))
}

#[test]
fn r15_4_r10_23_egui_inline_edit_col_cell_title_nav_rename_and_undo_toolbar() -> TestRes {
    let mut driver = make_shell_driver(build_r10_23_column_and_nav_app()?, ViewportMetrics::new(1280.0, 800.0, 1.0));
    assert_clean_section_9_4(&driver, "initial inline edit workspace")?;
    let spec = RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 96 };
    let initial_png = driver.state_mut().app.render_egui_shell_screenshot_png(spec, ScreenshotRegionTarget::FullWindow).map_err(|e| e.to_string())?.0;

    let cell_pos = widget_center(&driver, "grid.cell.0.0.0")?;
    driver.click_at(cell_pos)?;
    driver.click_at(cell_pos)?;
    assert!(driver.state().views.column_grid.inline_edit.is_some());
    driver.type_text("999.9")?;
    driver.press_key("Escape", KeyModifiers::default())?;
    assert!(driver.state().views.column_grid.inline_edit.is_none() && col1_cell(&driver, 0, 0) == Some(10.0));

    let cell_pos2 = widget_center(&driver, "grid.cell.0.0.0")?;
    driver.click_at(cell_pos2)?;
    driver.click_at(cell_pos2)?;
    for _ in 0..6 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("87.5")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(col1_cell(&driver, 0, 0), Some(87.5));

    let edited_png = driver.state_mut().app.render_egui_shell_screenshot_png(spec, ScreenshotRegionTarget::FullWindow).map_err(|e| e.to_string())?.0;
    assert_ne!(initial_png, edited_png);

    driver.click_widget_id("toolbar.undo")?;
    assert_eq!(col1_cell(&driver, 0, 0), Some(10.0));
    driver.click_widget_id("toolbar.redo")?;
    assert_eq!(col1_cell(&driver, 0, 0), Some(87.5));
    driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(col1_cell(&driver, 0, 0), Some(10.0));
    driver.press_egui_key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(col1_cell(&driver, 0, 0), Some(87.5));

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

    driver.click_widget_id("nav.sheet.col-1")?;
    driver.press_egui_key(Key::F2, Modifiers::NONE);
    assert!(driver.frame_meta().find_by_widget_id("nav.rename_input").is_some());
    driver.type_text(" Ignored")?;
    driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(driver.state().app.document().data_sheet(&SheetId::from("col-1")).map(|s| s.title.as_str()), Some("Drug Cohort"));

    driver.click_widget_id("nav.sheet.col-1")?;
    driver.press_egui_key(Key::F2, Modifiers::NONE);
    for _ in 0..16 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("Assay B")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(driver.state().app.document().data_sheet(&SheetId::from("col-1")).map(|s| s.title.as_str()), Some("Assay B"));

    assert_clean_section_9_4(&driver, "post inline edits")?;
    assert!(driver.max_input_to_paint_ms() < 500.0);
    Ok(())
}

#[test]
fn r15_4_r10_23_egui_pane_toggle_shortcuts_and_rtl_hidpi() -> TestRes {
    let mut driver = make_shell_driver(build_r10_23_column_and_nav_app()?, ViewportMetrics::new(1440.0, 900.0, 1.0));
    assert_clean_section_9_4(&driver, "pane shortcuts initial")?;
    assert!(driver.frame_meta().find_by_widget_id("nav.search_input").is_some());

    let (cmd_shift, ctrl_shift) = (Modifiers::COMMAND | Modifiers::SHIFT, Modifiers::CTRL | Modifiers::SHIFT);

    driver.press_egui_key(Key::B, Modifiers::COMMAND);
    assert!(driver.state().views.leading_collapsed_rail && driver.frame_meta().find_by_widget_id("nav.search_input").is_none());
    driver.press_egui_key(Key::B, Modifiers::CTRL);
    assert!(!driver.state().views.leading_collapsed_rail && driver.frame_meta().find_by_widget_id("nav.search_input").is_some());

    assert!(!driver.state().views.trailing_collapsed_rail);
    driver.press_egui_key(Key::I, cmd_shift);
    assert!(driver.state().views.trailing_collapsed_rail && driver.state().trailing_rect_pt.ok_or("collapsed trail")?.width() <= 38.0);
    driver.press_egui_key(Key::I, ctrl_shift);
    assert!(!driver.state().views.trailing_collapsed_rail && driver.state().trailing_rect_pt.ok_or("restored trail")?.width() >= 200.0);

    assert!(driver.state().app.family_open);
    driver.press_egui_key(Key::L, cmd_shift);
    assert!(!driver.state().app.family_open);
    driver.press_egui_key(Key::L, ctrl_shift);
    assert!(driver.state().app.family_open);

    assert_eq!(driver.state_mut().app.set_locale("ar"), "ar");
    driver.set_viewport(ViewportMetrics::new(1440.0, 900.0, 1.5));
    assert!(driver.state().leading_rect_pt.is_some() && driver.state().trailing_rect_pt.is_some());
    driver.press_egui_key(Key::B, Modifiers::COMMAND);
    driver.press_egui_key(Key::I, cmd_shift);
    assert_clean_section_9_4(&driver, "ar closed panes 1.5x")?;
    driver.press_egui_key(Key::B, Modifiers::CTRL);
    driver.press_egui_key(Key::I, ctrl_shift);
    assert_clean_section_9_4(&driver, "ar restored panes 1.5x")
}

#[test]
fn r15_4_r10_23_egui_portfolio_gallery_f6_cycling_and_quick_stats_footer() -> TestRes {
    let mut driver = make_shell_driver(build_r10_23_column_and_nav_app()?, ViewportMetrics::new(1280.0, 720.0, 1.0));

    driver.click_widget_id("status.section.gallery")?;
    assert!(driver.state().app.nav.gallery.open);
    for wid in ["gallery.filter.all", "gallery.filter.graphs", "gallery.filter.layouts", "gallery.filter.data", "gallery.card.graph-1"] {
        assert!(driver.frame_meta().find_by_widget_id(wid).is_some(), "missing `{wid}`");
    }
    driver.click_widget_id("gallery.filter.graphs")?;
    assert_eq!(driver.state().app.nav.gallery.filter_folder, Some(SheetFolderKind::Graphs));
    assert!(
        driver.frame_meta().find_by_widget_id("gallery.card.graph-1").is_some()
            && driver.frame_meta().find_by_widget_id("gallery.card.layout-1").is_none()
    );

    driver.click_widget_id("gallery.filter.all")?;
    driver.click_widget_id("gallery.card.layout-1")?;
    assert!(!driver.state().app.nav.gallery.open);
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));

    driver.click_widget_id("nav.sheet.col-1")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
    driver.state_mut().app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
    driver.run_settled();

    let mut visited = Vec::new();
    for _ in 0..5 {
        driver.press_egui_key(Key::F6, Modifiers::NONE);
        visited.push(driver.state().app.nav.active_pane);
    }
    for pane in [ActiveWorkspacePane::Inspector, ActiveWorkspacePane::Navigator, ActiveWorkspacePane::FamilyStrip] {
        assert!(visited.contains(&pane), "F6 missed {pane:?}");
    }

    driver.press_egui_key(Key::L, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(!driver.state().app.family_open);
    for _ in 0..5 {
        driver.press_egui_key(Key::F6, Modifiers::NONE);
        assert_ne!(driver.state().app.nav.active_pane, ActiveWorkspacePane::FamilyStrip);
    }
    driver.press_egui_key(Key::L, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(driver.state().app.family_open);

    let (start_pos, end_pos) = (widget_center(&driver, "grid.cell.0.0.0")?, widget_center(&driver, "grid.cell.3.0.0")?);
    driver.click_at(start_pos)?;
    driver.dispatch_events(vec![
        egui::Event::ModifiersChanged(Modifiers::SHIFT),
        egui::Event::PointerMoved(end_pos),
        egui::Event::PointerButton { pos: end_pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::SHIFT },
        egui::Event::PointerButton { pos: end_pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::SHIFT },
    ]);
    driver.dispatch_events(vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
    let summary = driver.frame_meta().find_by_widget_id("status.summary").ok_or("missing status.summary")?;
    for tok in ["N=4", "Mean=25", "SD=12.91", "SEM=6.455", "Median=25", "Range=30"] {
        assert!(summary.label.contains(tok), "missing `{tok}` in `{}`", summary.label);
    }
    assert_clean_section_9_4(&driver, "gallery/F6/footer parity")
}

#[test]
fn r15_4_r10_23_egui_14_beta_locales_themes_and_section_9_4_layout_gates() -> TestRes {
    assert_eq!(BETA_LOCALE_CODES.len(), 14);
    for &locale in &BETA_LOCALE_CODES {
        for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
            let mut app = build_r10_23_column_and_nav_app()?;
            app.set_theme_mode(mode);
            assert_eq!(app.set_locale(locale), locale);
            let mut driver = make_shell_driver(app, ViewportMetrics::new(1440.0, 600.0, 1.0));
            assert_clean_section_9_4(&driver, &format!("1440x600 locale={locale} theme={mode:?}"))?;
            let scale = if matches!(locale, "en" | "de" | "ja" | "ar") { 2.0 } else { 1.5 };
            driver.set_viewport(ViewportMetrics::new(640.0, 420.0, scale));
            assert_clean_section_9_4(&driver, &format!("640x420@{scale}x locale={locale} theme={mode:?}"))?;
        }
    }
    Ok(())
}

#[test]
fn r15_4_r10_23_egui_fontless_embedded_fallback_scenarios() -> TestRes {
    let book = FontBook::with_embedded_fallback();
    assert!(book.face_names().iter().all(|n| !n.starts_with("/System/Library") && !n.starts_with("/usr/share/fonts")));
    let cleanroom_fonts =
        build_egui_font_definitions_with_config(&FontDiscoveryConfig { fonts_dir: None, include_os_fonts: false, include_bundled_fonts: false });
    assert!(cleanroom_fonts.report.cleanroom_sans_loaded && cleanroom_fonts.report.symbol_fallback_loaded);

    for (locale, mode, scale) in [("en", UiThemeMode::Light, 1.0), ("de", UiThemeMode::Dark, 2.0), ("ar", UiThemeMode::HighContrast, 3.0)] {
        let mut app = build_r10_23_column_and_nav_app()?;
        assert_eq!(app.set_locale(locale), locale);
        app.set_theme_mode(mode);
        let mut driver = make_driver_with_fonts(app, ViewportMetrics::new(1280.0, 800.0, scale), cleanroom_fonts.definitions.clone());
        assert_clean_section_9_4(&driver, &format!("fontless {locale}/{mode:?}@{scale}x"))?;
        let png = driver
            .state_mut()
            .app
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 640, height_px: 420, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
            .map_err(|e| e.to_string())?
            .0;
        assert!(png.starts_with(PNG_SIGNATURE) && png.len() > 256);
    }
    Ok(())
}

#[test]
fn r15_4_r10_23_egui_drag_select_clipboard_bundle_language_draft_and_wave4_grids() -> TestRes {
    let mut app = build_r10_23_column_and_nav_app()?;
    let clipboard = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clipboard.clone());

    {
        let mut doc = app.document().clone();
        let mut mv = MultipleVariablesTable::new(3, 2).map_err(|e| e.to_string())?;
        for (idx, (title, role)) in
            [("Group", VariableRole::Categorical), ("Response", VariableRole::Continuous), ("Weight", VariableRole::Continuous)]
                .into_iter()
                .enumerate()
        {
            mv.variables[idx].title = title.to_owned();
            mv.variables[idx].role = role;
        }
        mv.variables[0].cells = vec![Cell::text("Ctrl"), Cell::text("Drug")];
        mv.variables[1].cells = vec![Cell::number(10.0), Cell::number(25.0)];
        mv.variables[2].cells = vec![Cell::number(1.0), Cell::number(2.0)];
        doc.add_data_sheet(DataSheet::new("mv-1", "MV Gap Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }
    let kind_val = serde_json::to_value(AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    assert!(
        app.execute_command("analysis.run", json!({ "source_sheet_id": "col-1", "sheet_id": "res-gap", "kind": kind_val }))
            .map_err(|e| e.to_string())?
            .payload
            .get("sheet_id")
            .is_some()
    );
    app.execute_command("sheet.select", json!({ "sheet_id": "col-1" })).map_err(|e| e.to_string())?;

    let mut driver = make_shell_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));

    let (c0, c2) = (widget_center(&driver, "grid.cell.0.0.0")?, widget_center(&driver, "grid.cell.2.0.0")?);
    driver.drag(c0, c2)?;
    let summary = driver.frame_meta().find_by_widget_id("status.summary").ok_or("missing status.summary")?;
    for tok in ["N=3", "Mean=20", "Median=20", "Range=20"] {
        assert!(summary.label.contains(tok), "drag-select missing `{tok}`");
    }

    driver.click_widget_id("grid.cell.1.0.0")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    driver.click_widget_id("grid.cell.0.1.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    assert_eq!(col1_cell(&driver, 0, 1), Some(20.0));

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.graphs")?;
    driver.click_widget_id("nav.sheet.graph-1")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    assert_eq!(clipboard.captured_bundles().len(), 1);
    let g_bundle = clipboard.last_bundle().ok_or("missing graph bundle")?;
    assert!(!g_bundle.svg.is_empty() && !g_bundle.png_bytes.is_empty() && g_bundle.pdf_bytes.starts_with(b"%PDF"));

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.layouts")?;
    driver.click_widget_id("nav.sheet.layout-1")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    assert_eq!(clipboard.captured_bundles().len(), 2);

    driver.click_widget_id("menu.view")?;
    driver.hover_at(widget_center(&driver, "menu.ui.locale.set")?)?;
    assert_eq!(LOCALES.len(), 16);
    let mut seen = vec![false; LOCALES.len()];
    for step in 0..=4 {
        for (idx, &loc) in LOCALES.iter().enumerate() {
            if let Some(entry) = driver.frame_meta().find_by_widget_id(&format!("popup.menu.ui.locale.set.opt.{idx}")) {
                assert_eq!(entry.label, locale_picker_label(loc));
                if loc != "en" {
                    assert!(entry.label.contains("[Draft]"));
                }
                seen[idx] = true;
            }
        }
        if seen.iter().all(|&s| s) || step == 4 {
            break;
        }
        driver.wheel_at(widget_center(&driver, "popup.menu.ui.locale.set")?, egui::vec2(0.0, -160.0))?;
    }
    assert!(seen.iter().all(|&s| s));
    driver.wheel_at(widget_center(&driver, "popup.menu.ui.locale.set")?, egui::vec2(0.0, 400.0))?;
    driver.click_widget_id("popup.menu.ui.locale.set.opt.1")?;
    assert_eq!(driver.state().app.locale, "de");
    assert_eq!(driver.state_mut().app.set_locale("en"), "en");
    driver.run_settled();

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.data")?;
    driver.click_widget_id("nav.sheet.xy-1")?;
    driver.click_widget_id("grid.cell.1.0.0")?;
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    driver.click_widget_id("grid.cell.0.0.0")?;
    driver.press_egui_key(Key::V, Modifiers::COMMAND);
    assert_eq!(driver.state().app.document().data_sheet(&SheetId::from("xy-1")).and_then(|s| cell_number(s, 0, 0)), Some(18.0));
    let x_pos = widget_center(&driver, "grid.xcell.0.0")?;
    driver.click_at(x_pos)?;
    driver.click_at(x_pos)?;
    for _ in 0..8 {
        driver.press_key("Backspace", KeyModifiers::default())?;
    }
    driver.type_text("7.5")?;
    driver.press_key("Enter", KeyModifiers::default())?;
    if let TableArchetype::Xy(xy) = &driver.state().app.document().data_sheet(&SheetId::from("xy-1")).unwrap().archetype {
        assert_eq!(xy.x_cells[0][0].active_number(), Some(7.5));
    }

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.results")?;
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
        assert_clean_section_9_4(&driver, &format!("ResultSheet 640x420@{scale}x {loc}/{mode:?}"))?;
        driver.set_viewport(ViewportMetrics::new(1440.0, 600.0, 1.0));
        assert_clean_section_9_4(&driver, &format!("ResultSheet 1440x600 {loc}/{mode:?}"))?;
    }

    assert_eq!(driver.state_mut().app.set_locale("en"), "en");
    driver.state_mut().app.set_theme_mode(UiThemeMode::Light);
    driver.set_viewport(ViewportMetrics::new(1280.0, 800.0, 1.0));
    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.data")?;
    driver.click_widget_id("nav.sheet.mv-1")?;
    assert!(driver.frame_meta().find_by_widget_id("mv_formula.role_badge.0").is_some());
    driver.click_widget_id("mv_formula.role_badge.0")?;
    driver.click_widget_id("mv_formula.role_manage.0")?;
    assert!(driver.state().app.grid.variables_dialog.open && driver.frame_meta().find_by_widget_id("dialog.manage_variables").is_some());
    assert_clean_section_9_4(&driver, "Manage Variables modal 1280x800")?;
    driver.click_widget_id("modal.ok")?;
    assert!(!driver.state().app.grid.variables_dialog.open);
    Ok(())
}
