#![forbid(unsafe_code)]
//! `R10.23` Beta desktop GUI human-path tests migrated cleanly onto native `egui`:
//! - Bullet 2 (**Shortcuts**): `Cmd/Ctrl+Z` undo, `Cmd/Ctrl+Shift+Z` redo, and Navigator
//!   (`Cmd/Ctrl+Shift+N`), Inspector (`Cmd/Ctrl+Shift+I`), and Family (`Cmd/Ctrl+Shift+F`) pane
//!   toggles driven strictly by raw `egui` key events through [`EguiHumanPathDriver`].
//! - Bullet 4 (**Graph and layout copy**): `Cmd/Ctrl+C` key events on a rendered [`GraphSheet`]
//!   and on a rendered [`LayoutSheet`] placing the `R5.4` multi-flavor vector [`ClipboardBundle`]
//!   (`%PDF-1.7`, `SVG`, `HTML`, `PNG`, `EMF`) through a stubbed [`PlatformClipboardAdapter`].
//! - Bullet 7 (**Translations at Beta — language picker**): `View > Language` picker labelling all
//!   15 non-English UI locales as `[Draft]` translations while keeping `"English (en)"` unmarked,
//!   plus fontless embedded-fallback verification without OS host fonts.

mod human_wf_common;

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, DescriptiveSpec, GraphKind, GraphSheet, InfoConstant, InfoSheet, LayoutItem, LayoutSheet,
    LayoutSpec, ProjectDocument, RectPt, ResultSheet, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::fonts::{FontDiscoveryConfig, build_egui_font_definitions_with_config};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::{
    ClipboardBundle, FontBook, LOCALES, OsAdapterAvailability, OsAdapterStatus, PlatformClipboardAdapter, RenderTargetSpec, ScreenshotRegionTarget,
    UiApp, UiThemeMode, is_draft_translation_locale, locale_display_name, locale_picker_label,
};
use egui::{Key, Modifiers};

fn build_r10_23_workspace() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R10.23 Shortcuts & Vector Copy").map_err(|e| e.to_string())?;

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Conditions").map_err(|e| e.to_string())?;
    info.constants
        .push(InfoConstant::new("Temperature_C", curvebook_doc::ConstantValue::Number(25.0), Some(String::from("C"))).map_err(|e| e.to_string())?);
    info.notes_markdown = String::from("Buffer equilibrated prior to measurement.");
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5).map_err(|e| e.to_string())?;
    xy.data_sets[0].title = String::from("Control");
    xy.data_sets[1].title = String::from("Compound A");
    for r in 0..5 {
        let x = f64::from(u32::try_from(r + 1).unwrap_or(1));
        xy.set_x_cell(r, Cell::number(x)).map_err(|e| e.to_string())?;
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 + x * 5.0);
        xy.data_sets[1].cells[r][0] = Cell::number(12.0 + x * 8.5);
    }
    let mut ds = DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
    ds.info_sheet_id = Some(SheetId::from("info-1"));
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;

    let spec = AnalysisSpec::new("res-1", "xy-1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    let mut rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    rs.analysis.set_custom_title(Some(String::from("Descriptive Summary"))).map_err(|e| e.to_string())?;
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let ds_ref = doc.data_sheet(&SheetId::from("xy-1")).ok_or_else(|| String::from("missing xy-1"))?.clone();
    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", &ds_ref).map_err(|e| e.to_string())?;
    gs.title = String::from("Dose Response Curve");
    gs.initialized = true;
    gs.spec.kind = GraphKind::Xy;
    gs.spec.title = String::from("Dose Response Curve");
    gs.spec.x_axis.title = String::from("Concentration (uM)");
    gs.spec.y_axis.title = String::from("Response (%)");
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

    doc.select_in_navigator(&SheetId::from("xy-1")).map_err(|e| e.to_string())?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.nav.welcome.open = false;
    app.sync_views();
    Ok(app)
}

fn cell_y0_value(app: &UiApp) -> Option<f64> {
    app.document().data_sheet(&SheetId::from("xy-1")).and_then(|ds| ds.cell(CellCoord::new(0, 0, 0))).and_then(Cell::active_number)
}

fn assert_valid_vector_bundle(bundle: &ClipboardBundle, label: &str) -> Result<(), String> {
    if !bundle.pdf_bytes.starts_with(b"%PDF-1.7") {
        return Err(format!("{label}: expected `%PDF-1.7` header, got {:?}", bundle.pdf_bytes.get(..12)));
    }
    if !bundle.pdf_bytes.windows(5).any(|w| w == b"%%EOF") {
        return Err(format!("{label}: PDF payload missing `%%EOF` trailer"));
    }
    if bundle.pdf_bytes.len() <= 256 {
        return Err(format!("{label}: PDF payload unexpectedly tiny ({} bytes)", bundle.pdf_bytes.len()));
    }
    if !bundle.svg.contains("<svg") || !bundle.svg.contains("</svg>") {
        return Err(format!("{label}: SVG payload missing `<svg>...</svg>` root"));
    }
    let has_vector_geom =
        bundle.svg.contains("<path") || bundle.svg.contains("<rect") || bundle.svg.contains("<line") || bundle.svg.contains("<text");
    if !has_vector_geom {
        return Err(format!("{label}: SVG payload contains no vector geometry"));
    }
    if bundle.html.is_empty() {
        return Err(format!("{label}: HTML clipboard fragment should be populated"));
    }
    if !bundle.png_bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bundle.png_bytes.len() <= 256 {
        return Err(format!("{label}: invalid PNG fallback inside ClipboardBundle"));
    }
    if let Some(ref emf) = bundle.emf_bytes
        && emf.len() < 88
    {
        return Err(format!("{label}: EMF payload unexpectedly small ({} bytes)", emf.len()));
    }
    Ok(())
}

#[test]
fn r10_23_undo_redo_shortcuts_cmd_and_ctrl_human_path() -> Result<(), String> {
    let app = build_r10_23_workspace()?;
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, human_wf_common::default_viewport());
    human_wf_common::assert_zero_shortcut_compliance(&mut harness)?;

    let initial_val = cell_y0_value(harness.app()).ok_or_else(|| String::from("missing initial cell (0,0,0)"))?;
    assert!((initial_val - 15.0).abs() < 1e-9);
    assert_eq!(harness.app().document().data_sheets.len(), 1);
    let initial_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;

    // 1. Double-click visible cell `grid.cell.0.0.0`, replace value with `87.5`, and press `Enter` to commit.
    let cell_pos = harness
        .driver
        .frame_meta()
        .find_by_widget_id("grid.cell.0.0.0")
        .map(|e| e.clipped_rect.center())
        .ok_or_else(|| String::from("missing grid.cell.0.0.0"))?;
    harness.driver.click_at(cell_pos)?;
    harness.driver.click_at(cell_pos)?;
    for _ in 0..8 {
        harness.driver.press_key("Backspace", KeyModifiers::default())?;
    }
    harness.driver.type_text("87.5")?;
    harness.driver.press_key("Enter", KeyModifiers::default())?;
    assert_eq!(cell_y0_value(harness.app()), Some(87.5));
    let edited_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    assert_ne!(edited_png, initial_png, "committed cell edit must update rendered egui screenshot");

    // 2. Duplicate DataSheet via drawn Data > Duplicate Sheet menu item (`data_sheets.len() == 2`).
    harness.click("menu.data")?;
    harness.click("menu.sheet.duplicate")?;
    assert_eq!(harness.app().document().data_sheets.len(), 2);

    // 3. `Cmd+Z` twice via raw egui keys: reverts duplication, then reverts cell edit.
    harness.driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(harness.app().document().data_sheets.len(), 1, "first Cmd+Z must undo sheet duplication");
    assert_eq!(cell_y0_value(harness.app()), Some(87.5));

    harness.driver.press_egui_key(Key::Z, Modifiers::COMMAND);
    assert_eq!(cell_y0_value(harness.app()), Some(15.0), "second Cmd+Z must restore cell (0,0,0) to 15.0");
    let undone_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    assert_ne!(undone_png, edited_png, "Cmd+Z undo must repaint cell back from 87.5");

    // 4. `Cmd+Shift+Z` twice: redoes cell edit, then redoes sheet duplication.
    harness.driver.press_egui_key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(cell_y0_value(harness.app()), Some(87.5), "first Cmd+Shift+Z must redo cell edit");
    harness.driver.press_egui_key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(harness.app().document().data_sheets.len(), 2, "second Cmd+Shift+Z must redo sheet duplication");

    // 5. `Ctrl+Z` and `Ctrl+Shift+Z` round-trip both edits cleanly.
    harness.driver.press_egui_key(Key::Z, Modifiers::CTRL);
    assert_eq!(harness.app().document().data_sheets.len(), 1, "first Ctrl+Z must undo duplication");
    harness.driver.press_egui_key(Key::Z, Modifiers::CTRL);
    assert_eq!(cell_y0_value(harness.app()), Some(15.0), "second Ctrl+Z must undo cell edit");

    harness.driver.press_egui_key(Key::Z, Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(cell_y0_value(harness.app()), Some(87.5), "first Ctrl+Shift+Z must redo cell edit");
    harness.driver.press_egui_key(Key::Z, Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(harness.app().document().data_sheets.len(), 2, "second Ctrl+Shift+Z must redo duplication");

    human_wf_common::assert_zero_shortcut_compliance(&mut harness)?;
    human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
    Ok(())
}

#[test]
fn r10_23_pane_toggle_shortcuts_navigator_inspector_family_human_path() -> Result<(), String> {
    let app = build_r10_23_workspace()?;
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::new(1440.0, 900.0, 1.0));

    harness.click("nav.chip.graphs")?;
    harness.click("nav.sheet.graph-1")?;
    assert_eq!(harness.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    assert!(harness.driver.frame_meta().find_by_widget_id("nav.search_input").is_some());
    assert!(harness.driver.frame_meta().entries.iter().any(|e| e.widget_id.starts_with("inspector.")));

    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
    let open_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;

    // Navigator shortcut (`Cmd+Shift+N` -> rail, `Ctrl+Shift+N` -> expanded).
    harness.driver.press_egui_key(Key::N, cmd_shift);
    assert!(harness.driver.state().views.leading_collapsed_rail, "Cmd+Shift+N must collapse Leading Dock");
    assert!(harness.driver.frame_meta().find_by_widget_id("nav.search_input").is_none());
    assert_ne!(harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?, open_png);

    harness.driver.press_egui_key(Key::N, ctrl_shift);
    assert!(!harness.driver.state().views.leading_collapsed_rail, "Ctrl+Shift+N must expand Leading Dock");
    assert!(harness.driver.frame_meta().find_by_widget_id("nav.search_input").is_some());

    // Inspector shortcut (`Cmd+Shift+I` -> rail, `Ctrl+Shift+I` -> expanded).
    harness.driver.press_egui_key(Key::I, cmd_shift);
    assert!(harness.driver.state().views.trailing_collapsed_rail, "Cmd+Shift+I must collapse Trailing Studio");
    assert_ne!(harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?, open_png);

    harness.driver.press_egui_key(Key::I, ctrl_shift);
    assert!(!harness.driver.state().views.trailing_collapsed_rail, "Ctrl+Shift+I must expand Trailing Studio");

    // Family shortcut (`Cmd+Shift+F` / `Ctrl+Shift+F`).
    assert!(harness.app().family_open);
    harness.driver.press_egui_key(Key::F, cmd_shift);
    assert!(!harness.app().family_open, "Cmd+Shift+F must collapse Family pane");
    harness.driver.press_egui_key(Key::F, ctrl_shift);
    assert!(harness.app().family_open, "Ctrl+Shift+F must restore Family pane");

    // Arabic (`ar`) @ 1.5x HiDPI toggle parity.
    let _ = harness.app_mut().set_locale("ar");
    harness.driver.set_viewport(ViewportMetrics::new(1440.0, 900.0, 1.5));
    harness.driver.press_egui_key(Key::N, cmd_shift);
    harness.driver.press_egui_key(Key::I, cmd_shift);
    harness.assert_parity_and_layout("ar closed panes 1.5x")?;
    harness.driver.press_egui_key(Key::N, ctrl_shift);
    harness.driver.press_egui_key(Key::I, ctrl_shift);
    harness.assert_parity_and_layout("ar restored panes 1.5x")?;
    human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
    Ok(())
}

#[derive(Debug, Default)]
struct CustomBundleCaptureAdapter {
    captured: RwLock<Vec<ClipboardBundle>>,
}

impl PlatformClipboardAdapter for CustomBundleCaptureAdapter {
    fn write_text(&self, _text: &str) -> Result<OsAdapterStatus, String> {
        Ok(OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Submitted,
            backend: String::from("custom-test-stub"),
            detail: String::from("text captured"),
            text: None,
            job_id: None,
        })
    }

    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        let mut guard = self.captured.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.push(bundle.clone());
        Ok(OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Submitted,
            backend: String::from("custom-test-stub"),
            detail: String::from("bundle captured"),
            text: None,
            job_id: None,
        })
    }
}

#[test]
fn r10_23_graph_and_layout_cmd_ctrl_c_vector_clipboard_bundle_human_path() -> Result<(), String> {
    let mut app = build_r10_23_workspace()?;
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip_stub.clone());
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, human_wf_common::default_viewport());

    // 1. Select `graph-1` and press `Cmd+C` & `Ctrl+C`.
    harness.click("nav.chip.graphs")?;
    harness.click("nav.sheet.graph-1")?;
    assert_eq!(harness.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    let graph_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;

    harness.driver.press_egui_key(Key::C, Modifiers::COMMAND);
    harness.driver.press_egui_key(Key::C, Modifiers::CTRL);
    let bundles_after_graph = clip_stub.captured_bundles();
    assert_eq!(bundles_after_graph.len(), 2, "expected 2 captured ClipboardBundles after Cmd+C & Ctrl+C on GraphSheet");
    let graph_bundle = &bundles_after_graph[0];
    assert_valid_vector_bundle(graph_bundle, "GraphSheet Cmd+C")?;
    assert_eq!(&bundles_after_graph[1].pdf_bytes, &graph_bundle.pdf_bytes);
    assert_eq!(&bundles_after_graph[1].svg, &graph_bundle.svg);

    // 2. Select `layout-1` and press `Cmd+C` & `Ctrl+C`.
    harness.click("nav.chip.layouts")?;
    harness.click("nav.sheet.layout-1")?;
    assert_eq!(harness.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));
    let layout_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    assert_ne!(layout_png, graph_png, "LayoutSheet frame must differ visually from GraphSheet frame");

    harness.driver.press_egui_key(Key::C, Modifiers::COMMAND);
    harness.driver.press_egui_key(Key::C, Modifiers::CTRL);
    let all_bundles = clip_stub.captured_bundles();
    assert_eq!(all_bundles.len(), 4, "expected 4 captured bundles total (2 GraphSheet + 2 LayoutSheet)");
    let layout_bundle = &all_bundles[2];
    assert_valid_vector_bundle(layout_bundle, "LayoutSheet Cmd+C")?;
    assert_ne!(layout_bundle.pdf_bytes, graph_bundle.pdf_bytes);
    assert_ne!(layout_bundle.svg, graph_bundle.svg);
    assert!(
        layout_bundle.svg.contains("Composite publication figure panel"),
        "LayoutSheet vector SVG must include rendered LayoutSheet callout text"
    );
    assert_eq!(&all_bundles[3].pdf_bytes, &layout_bundle.pdf_bytes);

    // 3. Verify caller-supplied custom `PlatformClipboardAdapter` works at `2.0x` HiDPI (`288` DPI).
    let custom_adapter = Arc::new(CustomBundleCaptureAdapter::default());
    let mut custom_app = build_r10_23_workspace()?;
    custom_app.set_clipboard_adapter(Arc::clone(&custom_adapter) as Arc<dyn PlatformClipboardAdapter>);
    let mut custom_harness = human_wf_common::EguiWfHarness::from_app(custom_app, ViewportMetrics::new(1280.0, 800.0, 2.0));
    custom_harness.click("nav.chip.layouts")?;
    custom_harness.click("nav.sheet.layout-1")?;
    custom_harness.driver.press_egui_key(Key::C, Modifiers::COMMAND);
    let custom_captured = custom_adapter.captured.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(custom_captured.len(), 1);
    assert_eq!(custom_captured[0].dpi, 288, "2.0x HiDPI viewport should encode 288 DPI raster fallback");
    assert_valid_vector_bundle(&custom_captured[0], "HiDPI 2.0x LayoutSheet Cmd+C")?;
    drop(custom_captured);

    human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
    Ok(())
}

#[test]
fn r10_23_language_picker_marks_15_non_english_locales_as_draft_human_path() -> Result<(), String> {
    let app = build_r10_23_workspace()?;
    let mut harness = human_wf_common::EguiWfHarness::from_app(app, human_wf_common::default_viewport());
    let initial_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;

    harness.click("menu.view")?;
    let lang_pos = harness
        .driver
        .frame_meta()
        .find_by_widget_id("menu.ui.locale.set")
        .map(|e| e.clipped_rect.center())
        .ok_or_else(|| String::from("missing menu.ui.locale.set"))?;
    harness.driver.hover_at(lang_pos)?;

    let mut seen_labels = BTreeMap::<usize, String>::new();
    for step in 0..=4 {
        for (idx, _loc) in LOCALES.iter().enumerate() {
            let opt_id = format!("popup.menu.ui.locale.set.opt.{idx}");
            if let Some(entry) = harness.driver.frame_meta().find_by_widget_id(&opt_id) {
                seen_labels.insert(idx, entry.label.clone());
            }
        }
        if seen_labels.len() == LOCALES.len() || step == 4 {
            break;
        }
        let popup_pos = harness
            .driver
            .frame_meta()
            .find_by_widget_id("popup.menu.ui.locale.set")
            .map(|e| e.clipped_rect.center())
            .ok_or_else(|| String::from("missing popup.menu.ui.locale.set"))?;
        harness.driver.wheel_at(popup_pos, egui::vec2(0.0, -160.0))?;
    }
    assert_eq!(seen_labels.len(), 16, "all 16 locale options must be reachable by wheel scroll: {seen_labels:?}");
    assert_eq!(seen_labels.get(&0).map(String::as_str), Some("English (en)"));
    assert!(!is_draft_translation_locale("en"));
    for (idx, &loc) in LOCALES.iter().enumerate().skip(1) {
        assert!(is_draft_translation_locale(loc));
        let expected = format!("{} ({loc}) [Draft]", locale_display_name(loc));
        assert_eq!(locale_picker_label(loc), expected);
        assert_eq!(seen_labels.get(&idx), Some(&expected));
    }

    // Select Arabic (`ar`, index 14), verify locale change & repaint, then switch back to `en`.
    harness.click("popup.menu.ui.locale.set.opt.14")?;
    assert_eq!(harness.app().locale, "ar");
    let rtl_png = harness.screenshot_png(ScreenshotRegionTarget::FullWindow)?;
    assert_ne!(rtl_png, initial_png, "selecting `ar` must repaint workspace");

    harness.click("menu.view")?;
    let lang_pos2 = harness
        .driver
        .frame_meta()
        .find_by_widget_id("menu.ui.locale.set")
        .map(|e| e.clipped_rect.center())
        .ok_or_else(|| String::from("missing menu.ui.locale.set on reopen"))?;
    harness.driver.hover_at(lang_pos2)?;
    for _ in 0..=4 {
        if harness.driver.frame_meta().find_by_widget_id("popup.menu.ui.locale.set.opt.0").is_some() {
            break;
        }
        let popup_pos2 = harness
            .driver
            .frame_meta()
            .find_by_widget_id("popup.menu.ui.locale.set")
            .map(|e| e.clipped_rect.center())
            .ok_or_else(|| String::from("missing popup.menu.ui.locale.set on reopen"))?;
        harness.driver.wheel_at(popup_pos2, egui::vec2(0.0, 200.0))?;
    }
    harness.click("popup.menu.ui.locale.set.opt.0")?;
    assert_eq!(harness.app().locale, "en");
    human_wf_common::assert_input_to_paint_budget(&harness, 50.0)?;
    Ok(())
}

#[test]
fn r10_23_fontless_embedded_fallback_scenarios_pass_without_host_fonts() -> Result<(), String> {
    let book = FontBook::with_embedded_fallback();
    assert!(
        book.face_names().iter().all(|name| !name.starts_with("/System/Library") && !name.starts_with("/usr/share/fonts")),
        "embedded fallback must not load OS host fonts: {:?}",
        book.face_names()
    );
    let cleanroom_fonts =
        build_egui_font_definitions_with_config(&FontDiscoveryConfig { fonts_dir: None, include_os_fonts: false, include_bundled_fonts: false });
    assert!(cleanroom_fonts.report.cleanroom_sans_loaded, "cleanroom sans fallback font must load without OS/bundled fonts");
    assert!(cleanroom_fonts.report.symbol_fallback_loaded, "symbol fallback font must load without OS/bundled fonts");

    for (locale, mode, scale) in [("en", UiThemeMode::Light, 1.0), ("de", UiThemeMode::Dark, 2.0), ("ar", UiThemeMode::HighContrast, 3.0)] {
        let mut app = build_r10_23_workspace()?;
        let _ = app.set_locale(locale);
        app.set_theme_mode(mode);
        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            human_wf_common::WfShellState::new(app),
            ViewportMetrics::new(1280.0, 800.0, scale),
            tokens,
            cleanroom_fonts.definitions.clone(),
            |ui, meta, st| {
                let rep = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
                st.dialog_rect_pt = rep.regions.dialog_rect_pt;
            },
        );
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        driver.run_settled();
        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "fontless parity failed ({locale}, {mode:?}, {scale}x): {:?}", parity.violations);
        let overlaps = driver.check_scrollbar_non_overlap();
        assert!(overlaps.is_empty(), "fontless scrollbar overlap ({locale}, {mode:?}, {scale}x): {overlaps:?}");

        let (png, _, _) = driver
            .state_mut()
            .app
            .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 640, height_px: 420, dpi: 96 }, ScreenshotRegionTarget::FullWindow)
            .map_err(|e| e.to_string())?;
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 256);
    }
    Ok(())
}
