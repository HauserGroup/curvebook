#![forbid(unsafe_code)]
//! `R10.23` Beta desktop GUI human-path tests for:
//! - Bullet 2 (**Shortcuts**): `Cmd/Ctrl+Z` undo, `Cmd/Ctrl+Shift+Z` redo, and Navigator
//!   (`Cmd/Ctrl+Shift+N`), Inspector (`Cmd/Ctrl+Shift+I`), and Family (`Cmd/Ctrl+Shift+F`) pane
//!   toggles driven strictly by raw `UiInputEvent::KeyDown` events through [`HumanPathDriver`].
//! - Bullet 4 (**Graph and layout copy**): `Cmd/Ctrl+C` key events on a rendered [`GraphSheet`]
//!   and on a rendered [`LayoutSheet`] placing the `R5.4` multi-flavor vector [`ClipboardBundle`]
//!   (`%PDF-1.7`, `SVG`, `HTML`, `PNG`, `EMF`) through a stubbed [`PlatformClipboardAdapter`].

mod human_wf_common;

use std::sync::{Arc, RwLock};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, DescriptiveSpec, GraphKind, GraphSheet, InfoConstant, InfoSheet, LayoutItem, LayoutSheet,
    LayoutSpec, ProjectDocument, RectPt, ResultSheet, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::{ClipboardBundle, OsAdapterAvailability, OsAdapterStatus, PlatformClipboardAdapter, UiApp};

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
    Ok(UiApp::from_session(Session::from_document(doc)))
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
        bundle.svg.contains("<path") || bundle.svg.contains("<line") || bundle.svg.contains("<rect") || bundle.svg.contains("<circle");
    if !has_vector_geom {
        return Err(format!("{label}: SVG payload contains no vector geometry primitives"));
    }
    if !bundle.html.contains("<div class=\"curvebook-figure\"") || !bundle.html.contains("<svg") {
        return Err(format!("{label}: HTML wrapper missing figure container or embedded SVG"));
    }
    if !bundle.png_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(format!("{label}: PNG fallback missing PNG magic signature"));
    }
    let Some(emf) = bundle.emf_bytes.as_ref() else {
        return Err(format!("{label}: expected ECMA-234 EMF vector payload in bundle"));
    };
    if emf.len() < 88 || emf.get(40..44) != Some(b" EMF".as_slice()) {
        return Err(format!("{label}: invalid ECMA-234 EMF signature (len={})", emf.len()));
    }
    let expected_flavors = vec!["pdf", "svg", "html", "png", "emf"];
    if bundle.available_flavors() != expected_flavors {
        return Err(format!("{label}: expected flavors {expected_flavors:?}, got {:?}", bundle.available_flavors()));
    }
    if bundle.width_pt <= 0.0 || bundle.height_pt <= 0.0 {
        return Err(format!("{label}: invalid physical dimensions ({}, {})", bundle.width_pt, bundle.height_pt));
    }
    Ok(())
}

#[test]
fn r10_23_undo_redo_shortcuts_cmd_and_ctrl_human_path() -> Result<(), String> {
    let app = build_r10_23_workspace()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    let initial_val = cell_y0_value(driver.app()).ok_or_else(|| String::from("missing initial cell (0,0,0)"))?;
    assert!((initial_val - 15.0).abs() < 1e-9);
    assert_eq!(driver.app().document().data_sheets.len(), 1);
    let frame_initial = driver.render_frame().map_err(|e| e.to_string())?;

    // 1. Double-click visible cell `grid.cell.0.0.0`, replace value with `87.5`, and press `Enter` to commit.
    let [cell_lx, cell_ly] = human_wf_common::widget_hit_center(&mut driver, &WidgetId::from("grid.cell.0.0.0"))?;
    let dbl = driver.double_click_logical(cell_lx, cell_ly)?;
    assert!(dbl.handled, "double-click on grid.cell.0.0.0 must open inline editor");
    assert!(driver.app().inline_editor.active);
    for _ in 0..8 {
        let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    }
    let _ = driver.type_text("87.5")?;
    let commit = driver.press_key("Enter", KeyModifiers::default())?;
    assert!(commit.handled);
    assert!(!driver.app().inline_editor.active);
    assert_eq!(cell_y0_value(driver.app()), Some(87.5));
    let cell_label_edited = driver
        .layout_tree()
        .node("grid.cell.0.0.0")
        .and_then(|n| n.a11y.value_text.clone().or_else(|| Some(n.a11y.label.clone())))
        .unwrap_or_default();
    assert!(cell_label_edited.contains("87.5"), "expected LayoutTree cell node to reflect 87.5, got `{cell_label_edited}`");
    let frame_after_edit = driver.render_frame().map_err(|e| e.to_string())?;
    assert_ne!(frame_after_edit.rgba, frame_initial.rgba, "committed cell edit must update rendered pixels");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // 2. Physically click `toolbar.sheet_duplicate` to create a second DataSheet (`data_sheets.len() == 2`).
    let dup = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("toolbar.sheet_duplicate"))?;
    assert!(dup.handled);
    assert_eq!(driver.app().document().data_sheets.len(), 2);
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // 3. `Cmd+Z` twice via raw `UiInputEvent::KeyDown`: first reverts sheet duplication, second reverts cell edit.
    let cmd_mod = KeyModifiers { meta: true, ..Default::default() };
    let cmd_shift_mod = KeyModifiers { meta: true, shift: true, ..Default::default() };

    let undo_sheet_cmd = driver.press_key("z", cmd_mod)?;
    assert!(undo_sheet_cmd.handled, "Cmd+Z must be handled");
    assert_eq!(driver.app().document().data_sheets.len(), 1, "first Cmd+Z must undo sheet duplication");
    assert_eq!(cell_y0_value(driver.app()), Some(87.5), "cell edit must still hold before second Cmd+Z");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    let undo_cell_cmd = driver.press_key("z", cmd_mod)?;
    assert!(undo_cell_cmd.handled, "second Cmd+Z must be handled");
    assert_eq!(cell_y0_value(driver.app()), Some(15.0), "second Cmd+Z must restore cell (0,0,0) to 15.0");
    let cell_label_restored = driver
        .layout_tree()
        .node("grid.cell.0.0.0")
        .and_then(|n| n.a11y.value_text.clone().or_else(|| Some(n.a11y.label.clone())))
        .unwrap_or_default();
    assert!(cell_label_restored.contains("15"), "expected LayoutTree cell node restored to 15, got `{cell_label_restored}`");
    let frame_after_undo = driver.render_frame().map_err(|e| e.to_string())?;
    assert_ne!(frame_after_undo.rgba, frame_after_edit.rgba, "Cmd+Z undo must repaint cell back from 87.5");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // 4. `Cmd+Shift+Z` twice via raw `UiInputEvent::KeyDown`: first redoes cell edit, second redoes sheet duplication.
    let redo_cell_cmd = driver.press_key("z", cmd_shift_mod)?;
    assert!(redo_cell_cmd.handled, "Cmd+Shift+Z must be handled");
    assert_eq!(cell_y0_value(driver.app()), Some(87.5), "first Cmd+Shift+Z must redo cell edit to 87.5");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    let redo_sheet_cmd = driver.press_key("Z", cmd_shift_mod)?;
    assert!(redo_sheet_cmd.handled, "second Cmd+Shift+Z must be handled");
    assert_eq!(driver.app().document().data_sheets.len(), 2, "second Cmd+Shift+Z must redo sheet duplication");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    // 5. `Ctrl+Z` and `Ctrl+Shift+Z` (Windows/Linux modifier parity) round-trip both edits cleanly.
    let ctrl_mod = KeyModifiers { ctrl: true, ..Default::default() };
    let ctrl_shift_mod = KeyModifiers { ctrl: true, shift: true, ..Default::default() };

    let _ = driver.press_key("z", ctrl_mod)?;
    assert_eq!(driver.app().document().data_sheets.len(), 1, "first Ctrl+Z must undo sheet duplication");
    let _ = driver.press_key("Z", ctrl_mod)?;
    assert_eq!(cell_y0_value(driver.app()), Some(15.0), "second Ctrl+Z must undo cell edit back to 15.0");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    let _ = driver.press_key("z", ctrl_shift_mod)?;
    assert_eq!(cell_y0_value(driver.app()), Some(87.5), "first Ctrl+Shift+Z must redo cell edit to 87.5");
    let _ = driver.press_key("Z", ctrl_shift_mod)?;
    assert_eq!(driver.app().document().data_sheets.len(), 2, "second Ctrl+Shift+Z must redo sheet duplication");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

fn verify_navigator_shortcut_toggle(
    driver: &mut curvebook_ui_egui::HumanPathDriver,
    base_nav_w: u32,
    base_canvas_w: u32,
    open_rgba: &[u8],
) -> Result<(), String> {
    let cmd_shift = KeyModifiers { meta: true, shift: true, ..Default::default() };
    let ctrl_shift = KeyModifiers { ctrl: true, shift: true, ..Default::default() };

    let _ = human_wf_common::click_widget_center(driver, &WidgetId::from("navigator.search_input"))?;
    assert_eq!(driver.app().focus_widget_id.as_deref(), Some("navigator.search_input"));

    let nav_close = driver.press_key("N", cmd_shift)?;
    assert!(nav_close.handled, "Cmd+Shift+N must toggle Navigator closed");
    assert!(!driver.app().nav.navigator_open);
    assert!(driver.app().focus_widget_id.is_none(), "closing Navigator must clear focus on hidden navigator.search_input");
    let tree_nav_closed = driver.layout_tree();
    assert_eq!(tree_nav_closed.frame.navigator.width, 0);
    assert_eq!(tree_nav_closed.frame.canvas.x, 0);
    assert_eq!(tree_nav_closed.frame.canvas.width, base_canvas_w + base_nav_w);
    assert!(tree_nav_closed.node("navigator.search_input").is_none());
    let frame_nav_closed = driver.render_frame().map_err(|e| e.to_string())?;
    assert_ne!(frame_nav_closed.rgba.as_slice(), open_rgba);
    let _ = human_wf_common::assert_zero_shortcut_compliance(driver)?;

    let nav_reopen = driver.press_key("n", ctrl_shift)?;
    assert!(nav_reopen.handled, "Ctrl+Shift+N must reopen Navigator");
    assert!(driver.app().nav.navigator_open);
    let tree_nav_reopened = driver.layout_tree();
    assert_eq!(tree_nav_reopened.frame.navigator.width, base_nav_w);
    assert_eq!(tree_nav_reopened.frame.canvas.width, base_canvas_w);
    assert!(tree_nav_reopened.node("navigator.search_input").is_some_and(|n| n.drawn_in_frame));
    let _ = human_wf_common::assert_zero_shortcut_compliance(driver)?;
    Ok(())
}

fn verify_inspector_shortcut_toggle(
    driver: &mut curvebook_ui_egui::HumanPathDriver,
    base_insp_w: u32,
    base_canvas_w: u32,
    open_rgba: &[u8],
) -> Result<(), String> {
    let cmd_shift = KeyModifiers { meta: true, shift: true, ..Default::default() };
    let ctrl_shift = KeyModifiers { ctrl: true, shift: true, ..Default::default() };

    let insp_close = driver.press_key("I", cmd_shift)?;
    assert!(insp_close.handled, "Cmd+Shift+I must toggle Inspector closed");
    assert!(!driver.app().nav.inspector_open);
    let tree_insp_closed = driver.layout_tree();
    assert_eq!(tree_insp_closed.frame.inspector.width, 0);
    assert_eq!(tree_insp_closed.frame.canvas.width, base_canvas_w + base_insp_w);
    assert!(!tree_insp_closed.nodes.iter().any(|n| n.id.as_str().starts_with("inspector.") && n.drawn_in_frame));
    let frame_insp_closed = driver.render_frame().map_err(|e| e.to_string())?;
    assert_ne!(frame_insp_closed.rgba.as_slice(), open_rgba);
    let _ = human_wf_common::assert_zero_shortcut_compliance(driver)?;

    let insp_reopen = driver.press_key("i", ctrl_shift)?;
    assert!(insp_reopen.handled, "Ctrl+Shift+I must reopen Inspector");
    assert!(driver.app().nav.inspector_open);
    let tree_insp_reopened = driver.layout_tree();
    assert_eq!(tree_insp_reopened.frame.inspector.width, base_insp_w);
    assert_eq!(tree_insp_reopened.frame.canvas.width, base_canvas_w);
    assert!(tree_insp_reopened.nodes.iter().any(|n| n.id.as_str().starts_with("inspector.") && n.drawn_in_frame));
    let _ = human_wf_common::assert_zero_shortcut_compliance(driver)?;
    Ok(())
}

fn verify_family_shortcut_toggle(driver: &mut curvebook_ui_egui::HumanPathDriver, open_rgba: &[u8]) -> Result<(), String> {
    let cmd_shift = KeyModifiers { meta: true, shift: true, ..Default::default() };
    let ctrl_shift = KeyModifiers { ctrl: true, shift: true, ..Default::default() };

    let nav_scroll_h_open =
        driver.layout_tree().node("navigator.tree").map(|n| n.bounds.rect_px.height).ok_or_else(|| String::from("missing navigator.tree"))?;
    let fam_close = driver.press_key("F", cmd_shift)?;
    assert!(fam_close.handled, "Cmd+Shift+F must collapse Family pane");
    assert!(!driver.app().family_open);
    let tree_fam_closed = driver.layout_tree();
    assert!(tree_fam_closed.node("family.tree").is_none(), "collapsed Family pane must omit family.tree list");
    let fam_btn_closed = tree_fam_closed.node("navigator.family_toggle").ok_or_else(|| String::from("missing navigator.family_toggle"))?;
    assert_eq!(fam_btn_closed.a11y.selected, Some(false));
    assert!(fam_btn_closed.a11y.label.starts_with('▸'));
    let nav_scroll_h_closed =
        tree_fam_closed.node("navigator.tree").map(|n| n.bounds.rect_px.height).ok_or_else(|| String::from("missing navigator.tree"))?;
    assert!(nav_scroll_h_closed > nav_scroll_h_open, "Navigator tree scroll height must expand when Family pane collapses");
    let frame_fam_closed = driver.render_frame().map_err(|e| e.to_string())?;
    assert_ne!(frame_fam_closed.rgba.as_slice(), open_rgba);
    let _ = human_wf_common::assert_zero_shortcut_compliance(driver)?;

    let fam_reopen = driver.press_key("f", ctrl_shift)?;
    assert!(fam_reopen.handled, "Ctrl+Shift+F must reopen Family pane");
    assert!(driver.app().family_open);
    let tree_fam_reopened = driver.layout_tree();
    assert!(tree_fam_reopened.node("family.tree").is_some_and(|n| n.drawn_in_frame));
    let fam_btn_open = tree_fam_reopened.node("navigator.family_toggle").ok_or_else(|| String::from("missing navigator.family_toggle"))?;
    assert_eq!(fam_btn_open.a11y.selected, Some(true));
    assert!(fam_btn_open.a11y.label.starts_with('▾'));
    let _ = human_wf_common::assert_zero_shortcut_compliance(driver)?;
    Ok(())
}

fn verify_rtl_hidpi_pane_shortcuts(driver: &mut curvebook_ui_egui::HumanPathDriver) -> Result<(), String> {
    let cmd_shift = KeyModifiers { meta: true, shift: true, ..Default::default() };
    let ctrl_shift = KeyModifiers { ctrl: true, shift: true, ..Default::default() };

    let _ = driver.app_mut().set_locale("ar");
    driver.set_metrics(ViewportMetrics::new(1440.0, 900.0, 1.5));
    let rtl_open = driver.layout_tree();
    assert!(rtl_open.is_rtl);
    assert!(rtl_open.frame.navigator.x > rtl_open.frame.inspector.x, "RTL must place Navigator on the right and Inspector on the left");
    let _ = driver.press_key("N", cmd_shift)?;
    let _ = driver.press_key("I", cmd_shift)?;
    let rtl_both_closed = driver.layout_tree();
    assert_eq!(rtl_both_closed.frame.navigator.width, 0);
    assert_eq!(rtl_both_closed.frame.inspector.width, 0);
    assert_eq!(rtl_both_closed.frame.canvas.width, rtl_both_closed.frame.total_width);
    let rtl_rep = driver.verify_parity();
    assert!(rtl_rep.is_clean(), "RTL closed-panes parity failed: {:?}", rtl_rep.violations);
    let _ = driver.press_key("n", ctrl_shift)?;
    let _ = driver.press_key("i", ctrl_shift)?;
    let rtl_restored = driver.verify_parity();
    assert!(rtl_restored.is_clean(), "RTL restored-panes parity failed: {:?}", rtl_restored.violations);
    Ok(())
}

#[test]
fn r10_23_pane_toggle_shortcuts_navigator_inspector_family_human_path() -> Result<(), String> {
    let app = build_r10_23_workspace()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    // Switch to `graph-1` by physically clicking its Navigator row so Inspector renders graph controls too.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("nav.sheet.graph-1"))?;
    assert_eq!(driver.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    let base_tree = driver.layout_tree();
    let base_nav_w = base_tree.frame.navigator.width;
    let base_insp_w = base_tree.frame.inspector.width;
    let base_canvas_w = base_tree.frame.canvas.width;
    assert!(base_nav_w > 0 && base_insp_w > 0, "expected both Navigator and Inspector open initially");
    assert!(base_tree.node("navigator.search_input").is_some_and(|n| n.drawn_in_frame));
    assert!(base_tree.node("family.tree").is_some_and(|n| n.drawn_in_frame));
    assert!(base_tree.nodes.iter().any(|n| n.id.as_str().starts_with("inspector.") && n.drawn_in_frame));
    let frame_all_open = driver.render_frame().map_err(|e| e.to_string())?;

    verify_navigator_shortcut_toggle(&mut driver, base_nav_w, base_canvas_w, &frame_all_open.rgba)?;
    verify_inspector_shortcut_toggle(&mut driver, base_insp_w, base_canvas_w, &frame_all_open.rgba)?;
    verify_family_shortcut_toggle(&mut driver, &frame_all_open.rgba)?;
    verify_rtl_hidpi_pane_shortcuts(&mut driver)?;
    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
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
    let app = build_r10_23_workspace()?;
    let mut driver = human_wf_common::new_human_driver(app, human_wf_common::default_viewport());

    let cmd_c = KeyModifiers { meta: true, ..Default::default() };
    let ctrl_c = KeyModifiers { ctrl: true, ..Default::default() };

    // 1. Physically select `graph-1` in Navigator, verify rendered `GraphSheet` canvas + parity, and press `Cmd+C` & `Ctrl+C`.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("nav.sheet.graph-1"))?;
    assert_eq!(driver.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    assert!(
        driver.layout_tree().scene_papers.iter().any(|p| p.sheet_id.as_str() == "graph-1" && p.paper.width > 0 && p.paper.height > 0),
        "expected GraphSheet scene paper rendered in LayoutTree"
    );
    let graph_frame = driver.render_frame().map_err(|e| e.to_string())?;
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    let out_graph_cmd = driver.press_key("c", cmd_c)?;
    assert!(out_graph_cmd.handled, "Cmd+C on rendered GraphSheet must be handled");
    assert_eq!(out_graph_cmd.action, "clipboard.copied_bundle");

    let bundles_after_cmd = driver.clipboard_stub().captured_bundles();
    assert_eq!(bundles_after_cmd.len(), 1, "expected 1 captured ClipboardBundle after Cmd+C on GraphSheet");
    let graph_bundle = &bundles_after_cmd[0];
    assert_valid_vector_bundle(graph_bundle, "GraphSheet Cmd+C")?;

    let out_graph_ctrl = driver.press_key("C", ctrl_c)?;
    assert!(out_graph_ctrl.handled, "Ctrl+C on rendered GraphSheet must be handled");
    assert_eq!(out_graph_ctrl.action, "clipboard.copied_bundle");
    let bundles_after_ctrl = driver.clipboard_stub().captured_bundles();
    assert_eq!(bundles_after_ctrl.len(), 2, "expected 2 captured ClipboardBundles after Ctrl+C on GraphSheet");
    assert_eq!(&bundles_after_ctrl[1].pdf_bytes, &graph_bundle.pdf_bytes);
    assert_eq!(&bundles_after_ctrl[1].svg, &graph_bundle.svg);

    // 2. Physically select `layout-1` in Navigator, verify rendered `LayoutSheet` canvas + parity, and press `Cmd+C` & `Ctrl+C`.
    let _ = human_wf_common::click_widget_center(&mut driver, &WidgetId::from("nav.sheet.layout-1"))?;
    assert_eq!(driver.app().document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));
    assert!(
        driver.layout_tree().scene_papers.iter().any(|p| p.sheet_id.as_str() == "layout-1" && p.paper.width > 0 && p.paper.height > 0),
        "expected LayoutSheet scene paper rendered in LayoutTree"
    );
    let layout_frame = driver.render_frame().map_err(|e| e.to_string())?;
    assert_ne!(layout_frame.rgba, graph_frame.rgba, "LayoutSheet frame must differ visually from single GraphSheet frame");
    let _ = human_wf_common::assert_zero_shortcut_compliance(&mut driver)?;

    let out_layout_cmd = driver.press_key("c", cmd_c)?;
    assert!(out_layout_cmd.handled, "Cmd+C on rendered LayoutSheet must be handled");
    assert_eq!(out_layout_cmd.action, "clipboard.copied_bundle");

    let out_layout_ctrl = driver.press_key("C", ctrl_c)?;
    assert!(out_layout_ctrl.handled, "Ctrl+C on rendered LayoutSheet must be handled");
    assert_eq!(out_layout_ctrl.action, "clipboard.copied_bundle");

    let all_bundles = driver.clipboard_stub().captured_bundles();
    assert_eq!(all_bundles.len(), 4, "expected 4 captured bundles total (2 GraphSheet + 2 LayoutSheet)");
    let layout_bundle = &all_bundles[2];
    assert_valid_vector_bundle(layout_bundle, "LayoutSheet Cmd+C")?;
    assert_ne!(layout_bundle.pdf_bytes, graph_bundle.pdf_bytes, "LayoutSheet PDF vector payload must differ from standalone GraphSheet PDF");
    assert_ne!(layout_bundle.svg, graph_bundle.svg, "LayoutSheet SVG vector payload must differ from standalone GraphSheet SVG");
    assert!(
        layout_bundle.svg.contains("Composite publication figure panel"),
        "LayoutSheet vector SVG must include rendered LayoutSheet callout text"
    );
    assert_eq!(&all_bundles[3].pdf_bytes, &layout_bundle.pdf_bytes);

    // 3. Verify caller-supplied custom `PlatformClipboardAdapter` pre-installed on `UiApp` is preserved by `HumanPathDriver::new`.
    let custom_adapter = Arc::new(CustomBundleCaptureAdapter::default());
    let mut custom_app = build_r10_23_workspace()?;
    custom_app.set_clipboard_adapter(Arc::clone(&custom_adapter) as Arc<dyn PlatformClipboardAdapter>);
    let mut custom_driver = human_wf_common::new_human_driver(custom_app, ViewportMetrics::new(1280.0, 800.0, 2.0));
    let _ = human_wf_common::click_widget_center(&mut custom_driver, &WidgetId::from("nav.sheet.layout-1"))?;
    let hidpi_copy = custom_driver.press_key("c", cmd_c)?;
    assert!(hidpi_copy.handled);
    let custom_captured = custom_adapter.captured.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(custom_captured.len(), 1);
    assert_eq!(custom_captured[0].dpi, 288, "2.0x HiDPI viewport should encode 288 DPI raster fallback alongside vector PDF/SVG/EMF");
    assert_valid_vector_bundle(&custom_captured[0], "HiDPI 2.0x LayoutSheet Cmd+C")?;
    drop(custom_captured);

    human_wf_common::assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}
