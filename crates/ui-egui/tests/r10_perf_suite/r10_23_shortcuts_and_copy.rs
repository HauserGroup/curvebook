#![forbid(unsafe_code)]
//! `R10.23` Beta desktop GUI human-path tests on native `egui`:
//! undo/redo & pane-toggle shortcuts, vector clipboard bundles, language picker `[Draft]` labels,
//! fontless embedded-fallback verification, and `R17.2` Copy for Slides.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::common::{PNG_SIGNATURE, ShellTestState, TestRes, assert_clean_parity, cell_number, ensure, seed_xy_doc, widget_rect};
use crate::human_wf_common::{EguiWfHarness, assert_input_to_paint_budget, assert_zero_shortcut_compliance, default_viewport};
use crate::suite_helpers::{add_init_graph, new_shell_app};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, DescriptiveSpec, GraphKind, InfoConstant, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, RectPt, ResultSheet,
    SheetId,
};
use curvebook_platform::sync::RwLock;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::fonts::{FontDiscoveryConfig, build_egui_font_definitions_with_config};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::{
    ClipboardBundle, FontBook, LOCALES, OsAdapterAvailability, OsAdapterStatus, PlatformClipboardAdapter, PointerButton, RenderTargetSpec,
    ScreenshotRegionTarget, UiApp, UiThemeMode, dispatch_ui_command, is_draft_translation_locale, locale_display_name, locale_picker_label,
};
use egui::{Key, Modifiers};
use serde_json::json;

fn build_r10_23_workspace() -> Result<UiApp, String> {
    let (mut doc, xy_id) = seed_xy_doc(
        "R10.23 Shortcuts & Vector Copy",
        "xy-1",
        "Dose Response",
        "X",
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[("Control", &[15.0, 20.0, 25.0, 30.0, 35.0]), ("Compound A", &[20.5, 29.0, 37.5, 46.0, 54.5])],
    )?;
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Conditions").map_err(|e| e.to_string())?;
    info.constants.push(InfoConstant::new("Temperature_C", curvebook_doc::ConstantValue::Number(25.0), Some("C".into())).map_err(|e| e.to_string())?);
    info.notes_markdown = "Buffer equilibrated prior to measurement.".into();
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    if let Some(ds) = doc.data_sheet_mut(&xy_id) {
        ds.info_sheet_id = Some(SheetId::from("info-1"));
    }
    let spec = AnalysisSpec::new("res-1", "xy-1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    let mut rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    rs.analysis.set_custom_title(Some("Descriptive Summary".into())).map_err(|e| e.to_string())?;
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let gid = add_init_graph(&mut doc, &xy_id, "graph-1", GraphKind::Xy)?;
    if let Some(gs) = doc.graph_sheet_mut(&gid) {
        gs.title = "Dose Response Curve".into();
        gs.spec.title = "Dose Response Curve".into();
        gs.spec.x_axis.title = "Concentration (uM)".into();
        gs.spec.y_axis.title = "Response (%)".into();
    }
    let mut lspec = LayoutSpec::default();
    lspec.items.push(LayoutItem::LiveGraph {
        id: "panel-a".into(),
        graph_sheet_id: gid,
        bounds_pt: RectPt::new(24.0, 32.0, 320.0, 220.0),
        scale_percent: 100.0,
        badge_override: Some("A".into()),
    });
    lspec.items.push(LayoutItem::TextCallout {
        id: "callout-1".into(),
        bounds_pt: RectPt::new(360.0, 48.0, 160.0, 64.0),
        markdown: "Composite publication figure panel".into(),
        font_size_pt: 11.0,
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Composite Figure 1", lspec).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&xy_id).map_err(|e| e.to_string())?;
    Ok(new_shell_app(doc))
}

fn cell_y0_value(app: &UiApp) -> Option<f64> {
    app.document().data_sheet(&SheetId::from("xy-1")).and_then(|ds| cell_number(ds, 0, 0))
}

fn assert_valid_vector_bundle(bundle: &ClipboardBundle, label: &str) -> TestRes {
    ensure(
        bundle.pdf_bytes.starts_with(b"%PDF-1.7") && bundle.pdf_bytes.windows(5).any(|w| w == b"%%EOF") && bundle.pdf_bytes.len() > 256,
        format!("{label}: invalid PDF"),
    )?;
    let has_geom = ["<path", "<rect", "<line", "<text"].iter().any(|t| bundle.svg.contains(t));
    ensure(bundle.svg.contains("<svg") && bundle.svg.contains("</svg>") && has_geom, format!("{label}: invalid SVG"))?;
    ensure(!bundle.html.is_empty(), format!("{label}: empty HTML"))?;
    ensure(bundle.png_bytes.starts_with(PNG_SIGNATURE) && bundle.png_bytes.len() > 256, format!("{label}: invalid PNG"))?;
    ensure(bundle.emf_bytes.as_ref().is_none_or(|emf| emf.len() >= 88), format!("{label}: tiny EMF"))
}

#[test]
fn r10_23_undo_redo_shortcuts_cmd_and_ctrl_human_path() -> TestRes {
    let mut h = EguiWfHarness::from_app(build_r10_23_workspace()?, default_viewport());
    assert_zero_shortcut_compliance(&mut h)?;
    assert!((cell_y0_value(h.app()).ok_or("missing (0,0,0)")? - 15.0).abs() < 1e-9 && h.app().document().data_sheets.len() == 1);
    let initial_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;

    h.type_replace_cell("grid.cell.0.0.0", "87.5")?;
    assert_eq!(cell_y0_value(h.app()), Some(87.5));
    let edited_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    assert_ne!(edited_png, initial_png);

    h.click("menu.data")?;
    h.click("menu.sheet.duplicate")?;
    assert_eq!(h.app().document().data_sheets.len(), 2);

    // Round-trip both `Cmd` and `Ctrl` undo/redo pairs.
    for (base_mod, check_png) in [(Modifiers::COMMAND, true), (Modifiers::CTRL, false)] {
        h.driver.press_egui_key(Key::Z, base_mod);
        assert_eq!((h.app().document().data_sheets.len(), cell_y0_value(h.app())), (1, Some(87.5)));
        h.driver.press_egui_key(Key::Z, base_mod);
        assert_eq!(cell_y0_value(h.app()), Some(15.0));
        if check_png {
            assert_ne!(h.screenshot_png(ScreenshotRegionTarget::FullWindow)?, edited_png);
        }
        h.driver.press_egui_key(Key::Z, base_mod | Modifiers::SHIFT);
        assert_eq!(cell_y0_value(h.app()), Some(87.5));
        h.driver.press_egui_key(Key::Z, base_mod | Modifiers::SHIFT);
        assert_eq!(h.app().document().data_sheets.len(), 2);
    }
    assert_zero_shortcut_compliance(&mut h)?;
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn r10_23_pane_toggle_shortcuts_navigator_inspector_family_human_path() -> TestRes {
    let mut h = EguiWfHarness::from_app(build_r10_23_workspace()?, ViewportMetrics::new(1440.0, 900.0, 1.0));
    h.select_nav_kind_filter("nav.chip.graphs")?;
    h.click("nav.sheet.graph-1")?;
    assert_eq!(h.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    assert!(h.driver.frame_meta().find_by_widget_id("nav.search_input").is_some());
    assert!(h.driver.frame_meta().entries.iter().any(|e| e.widget_id.starts_with("inspector.")));

    let (cmd_s, ctrl_s) = (Modifiers::COMMAND | Modifiers::SHIFT, Modifiers::CTRL | Modifiers::SHIFT);
    let open_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;

    h.driver.press_egui_key(Key::B, Modifiers::COMMAND);
    assert!(h.driver.state().views.leading_collapsed_rail && h.driver.frame_meta().find_by_widget_id("nav.search_input").is_none());
    assert_ne!(h.screenshot_png(ScreenshotRegionTarget::FullWindow)?, open_png);
    h.driver.press_egui_key(Key::B, Modifiers::CTRL);
    assert!(!h.driver.state().views.leading_collapsed_rail && h.driver.frame_meta().find_by_widget_id("nav.search_input").is_some());

    h.driver.press_egui_key(Key::I, cmd_s);
    assert!(h.driver.state().views.trailing_collapsed_rail && h.screenshot_png(ScreenshotRegionTarget::FullWindow)? != open_png);
    h.driver.press_egui_key(Key::I, ctrl_s);
    assert!(!h.driver.state().views.trailing_collapsed_rail);

    assert!(h.app().family_open);
    h.driver.press_egui_key(Key::L, cmd_s);
    assert!(!h.app().family_open);
    h.driver.press_egui_key(Key::L, ctrl_s);
    assert!(h.app().family_open);

    // Arabic (`ar`) @ 1.5x HiDPI toggle parity.
    assert_eq!(h.app_mut().set_locale("ar"), "ar");
    h.driver.set_viewport(ViewportMetrics::new(1440.0, 900.0, 1.5));
    for (nav_mods, insp_mods, lbl) in [(Modifiers::COMMAND, cmd_s, "ar closed panes 1.5x"), (Modifiers::CTRL, ctrl_s, "ar restored panes 1.5x")] {
        h.driver.press_egui_key(Key::B, nav_mods);
        h.driver.press_egui_key(Key::I, insp_mods);
        h.assert_parity_and_layout(lbl)?;
    }
    assert_input_to_paint_budget(&h, 50.0)
}

#[derive(Debug, Default)]
struct CustomBundleCaptureAdapter {
    captured: RwLock<Vec<ClipboardBundle>>,
}

impl CustomBundleCaptureAdapter {
    fn ok_status(detail: &str) -> OsAdapterStatus {
        OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Submitted,
            backend: "custom-test-stub".into(),
            detail: detail.into(),
            text: None,
            job_id: None,
        }
    }
}

impl PlatformClipboardAdapter for CustomBundleCaptureAdapter {
    fn write_text(&self, _text: &str) -> Result<OsAdapterStatus, String> {
        Ok(Self::ok_status("text captured"))
    }
    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        self.captured.write_or_recover().map_err(|e| e.to_string())?.push(bundle.clone());
        Ok(Self::ok_status("bundle captured"))
    }
}

#[test]
fn r10_23_graph_and_layout_cmd_ctrl_c_vector_clipboard_bundle_human_path() -> TestRes {
    let mut app = build_r10_23_workspace()?;
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip_stub.clone());
    let mut h = EguiWfHarness::from_app(app, default_viewport());

    h.select_nav_kind_filter("nav.chip.graphs")?;
    h.click("nav.sheet.graph-1")?;
    assert_eq!(h.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    let graph_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    h.driver.press_egui_key(Key::C, Modifiers::COMMAND);
    h.driver.press_egui_key(Key::C, Modifiers::CTRL);
    let g_bundles = clip_stub.captured_bundles();
    assert_eq!(g_bundles.len(), 2);
    assert_valid_vector_bundle(&g_bundles[0], "GraphSheet Cmd+C")?;
    assert_eq!((&g_bundles[1].pdf_bytes, &g_bundles[1].svg), (&g_bundles[0].pdf_bytes, &g_bundles[0].svg));

    h.select_nav_kind_filter("nav.chip.layouts")?;
    h.click("nav.sheet.layout-1")?;
    assert_eq!(h.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));
    assert_ne!(h.screenshot_png(ScreenshotRegionTarget::FullWindow)?, graph_png);
    h.driver.press_egui_key(Key::C, Modifiers::COMMAND);
    h.driver.press_egui_key(Key::C, Modifiers::CTRL);
    let all = clip_stub.captured_bundles();
    assert_eq!(all.len(), 4);
    assert_valid_vector_bundle(&all[2], "LayoutSheet Cmd+C")?;
    assert!(
        all[2].pdf_bytes != g_bundles[0].pdf_bytes && all[2].svg != g_bundles[0].svg && all[2].svg.contains("Composite publication figure panel")
    );
    assert_eq!(all[3].pdf_bytes, all[2].pdf_bytes);

    // Custom PlatformClipboardAdapter at 2.0x HiDPI (288 DPI).
    let custom = Arc::new(CustomBundleCaptureAdapter::default());
    let mut custom_app = build_r10_23_workspace()?;
    custom_app.set_clipboard_adapter(Arc::clone(&custom) as Arc<dyn PlatformClipboardAdapter>);
    let mut ch = EguiWfHarness::from_app(custom_app, ViewportMetrics::new(1280.0, 800.0, 2.0));
    ch.select_nav_kind_filter("nav.chip.layouts")?;
    ch.click("nav.sheet.layout-1")?;
    ch.driver.press_egui_key(Key::C, Modifiers::COMMAND);
    let cap = custom.captured.read_or_recover().map_err(|e| e.to_string())?;
    assert_eq!((cap.len(), cap[0].dpi), (1, 288));
    assert_valid_vector_bundle(&cap[0], "HiDPI 2.0x LayoutSheet Cmd+C")?;
    drop(cap);
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn r10_23_language_picker_marks_15_non_english_locales_as_draft_human_path() -> TestRes {
    let mut h = EguiWfHarness::from_app(build_r10_23_workspace()?, default_viewport());
    let initial_png = h.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    let open_lang_menu = |h: &mut EguiWfHarness| -> TestRes {
        h.click("menu.view")?;
        h.driver.hover_at(h.widget_center("menu.ui.locale.set")?)
    };

    open_lang_menu(&mut h)?;
    let mut seen_labels = BTreeMap::<usize, String>::new();
    for step in 0..=4 {
        for idx in 0..LOCALES.len() {
            if let Some(entry) = h.driver.frame_meta().find_by_widget_id(&format!("popup.menu.ui.locale.set.opt.{idx}")) {
                seen_labels.insert(idx, entry.label.clone());
            }
        }
        if seen_labels.len() == LOCALES.len() || step == 4 {
            break;
        }
        let pos = h.widget_center("popup.menu.ui.locale.set")?;
        h.driver.wheel_at(pos, egui::vec2(0.0, -160.0))?;
    }
    assert_eq!((seen_labels.len(), seen_labels.get(&0).map(String::as_str), is_draft_translation_locale("en")), (16, Some("English (en)"), false));
    for (idx, &loc) in LOCALES.iter().enumerate().skip(1) {
        let expected = format!("{} ({loc}) [Draft]", locale_display_name(loc));
        assert!(is_draft_translation_locale(loc) && locale_picker_label(loc) == expected && seen_labels.get(&idx) == Some(&expected));
    }

    h.click("popup.menu.ui.locale.set.opt.14")?;
    assert_eq!(h.app().locale, "ar");
    assert_ne!(h.screenshot_png(ScreenshotRegionTarget::FullWindow)?, initial_png);

    open_lang_menu(&mut h)?;
    for _ in 0..=4 {
        if h.driver.frame_meta().find_by_widget_id("popup.menu.ui.locale.set.opt.0").is_some() {
            break;
        }
        let pos = h.widget_center("popup.menu.ui.locale.set")?;
        h.driver.wheel_at(pos, egui::vec2(0.0, 200.0))?;
    }
    h.click("popup.menu.ui.locale.set.opt.0")?;
    assert_eq!(h.app().locale, "en");
    assert_input_to_paint_budget(&h, 50.0)
}

#[test]
fn r10_23_fontless_embedded_fallback_scenarios_pass_without_host_fonts() -> TestRes {
    let book = FontBook::with_embedded_fallback();
    assert!(book.face_names().iter().all(|n| !n.starts_with("/System/Library") && !n.starts_with("/usr/share/fonts")));
    let cleanroom_fonts =
        build_egui_font_definitions_with_config(&FontDiscoveryConfig { fonts_dir: None, include_os_fonts: false, include_bundled_fonts: false });
    assert!(cleanroom_fonts.report.cleanroom_sans_loaded && cleanroom_fonts.report.symbol_fallback_loaded);

    for (locale, mode, scale) in [("en", UiThemeMode::Light, 1.0), ("de", UiThemeMode::Dark, 2.0), ("ar", UiThemeMode::HighContrast, 3.0)] {
        let mut app = build_r10_23_workspace()?;
        assert_eq!(app.set_locale(locale), locale);
        app.set_theme_mode(mode);
        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            ShellTestState::new(app),
            ViewportMetrics::new(1280.0, 800.0, scale),
            tokens,
            cleanroom_fonts.definitions.clone(),
            |ui, meta, st| {
                st.dialog_rect_pt = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents).regions.dialog_rect_pt;
            },
        );
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        driver.run_settled();
        assert_clean_parity(&driver, &format!("fontless ({locale}, {mode:?}, {scale}x)"))?;
        let (png, _, _) = driver
            .state_mut()
            .app
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 640, height_px: 420, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
            .map_err(|e| e.to_string())?;
        assert!(png.starts_with(PNG_SIGNATURE) && png.len() > 256);
    }
    Ok(())
}

#[test]
fn r17_2_copy_for_slides_shortcut_context_menu_and_dispatch_ui_command() -> TestRes {
    let mut app = build_r10_23_workspace()?;
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip_stub.clone());
    let mut h = EguiWfHarness::from_app(app, default_viewport());

    h.select_nav_kind_filter("nav.chip.graphs")?;
    h.click("nav.sheet.graph-1")?;
    assert_input_to_paint_budget(&h, 50.0)?;
    h.driver.press_egui_key(Key::C, Modifiers::COMMAND | Modifiers::SHIFT);
    h.driver.press_egui_key(Key::C, Modifiers::CTRL | Modifiers::SHIFT);
    let slides = clip_stub.captured_slide_bundles();
    assert_eq!((slides.len(), slides[0].dpi), (2, 300));
    assert!(slides[0].tiff_bytes.as_deref().is_some_and(|b| b.starts_with(b"II*\0")));
    assert!(slides[0].html.contains("<img src=\"data:image/png;base64,"));
    assert_valid_vector_bundle(&slides[0], "GraphSheet Cmd+Shift+C")?;
    assert_eq!(h.app().interaction.last_clipboard_text, None);

    for (chip, sheet, ctx_wid) in [
        (None, None, "canvas.graph_ctx.copy_for_slides"),
        (Some("nav.chip.layouts"), Some("nav.sheet.layout-1"), "canvas.layout_ctx.copy_for_slides"),
    ] {
        if let (Some(c), Some(s)) = (chip, sheet) {
            h.select_nav_kind_filter(c)?;
            h.click(s)?;
        }
        let rect = widget_rect(&h.driver, "canvas.viewport")?;
        h.driver.click_at_button(rect.center(), PointerButton::Secondary, KeyModifiers::default())?;
        h.click(ctx_wid)?;
    }
    let slides_ctx = clip_stub.captured_slide_bundles();
    assert_eq!((slides_ctx.len(), slides_ctx[3].dpi), (4, 300));
    assert!(slides_ctx[3].svg.contains("Composite publication figure panel"));

    // Programmatic `ui.clipboard.copyForSlides` with custom 600 DPI.
    let mut direct_app = build_r10_23_workspace()?;
    let direct_stub = Arc::new(StubPlatformClipboardAdapter::new());
    direct_app.set_clipboard_adapter(direct_stub.clone());
    let val = dispatch_ui_command(&mut direct_app, "ui.clipboard.copyForSlides", &json!({ "sheet_id": "layout-1", "dpi": 600 }))
        .map_err(|e| e.to_string())?
        .ok_or("ui.clipboard.copyForSlides not registered")?;
    assert_eq!(
        (val.get("dpi").and_then(serde_json::Value::as_u64), val.get("omits_plain_text_svg").and_then(serde_json::Value::as_bool)),
        (Some(600), Some(true))
    );
    assert_eq!(
        (direct_stub.captured_slide_bundles().len(), direct_stub.captured_slide_bundles()[0].dpi, direct_app.interaction.last_clipboard_text),
        (1, 600, None)
    );
    Ok(())
}
