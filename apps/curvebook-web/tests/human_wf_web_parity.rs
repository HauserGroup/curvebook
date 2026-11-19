//! Web human-path parity tests (`R10.1` bullet 3 web spike + 5-stage `WF-50` CLD +
//! keyboard/IME + large-`N` wheel scroll + `1.0x`/`2.0x` HiDPI + RTL).
//!
//! All user interactions drive [`WebBridgeSession`] strictly through raw
//! [`WebBridgeEvent::UiInput`] (`PointerMove`, `PointerDown`, `PointerUp`, `DoubleClick`,
//! `KeyDown`, `TextInput`), [`WebBridgeEvent::Wheel`], [`WebBridgeEvent::ImePreedit`],
//! [`WebBridgeEvent::ImeCommit`], and [`WebBridgeEvent::ResizeViewport`] against hit-tested
//! shared [`LayoutTree`] coordinates — zero semantic command/chrome shortcuts.

#![forbid(unsafe_code)]
#![allow(clippy::cast_precision_loss)]

mod web_parity_common;

use curvebook_ui_egui::compositor::FramePalette;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, UiInputEvent};
use curvebook_ui_egui::layout::hit_test::hit_test_logical;
use curvebook_ui_egui::layout::verify::{ParityViolation, verify_pixel_to_hit_box_parity, verify_tree_invariants};
use curvebook_ui_egui::layout::{FocusScope, LayoutTree, WidgetNode};
use curvebook_ui_egui::text_bridge::DesktopTextMeasurer;
use curvebook_ui_egui::theme::FontBook;
use curvebook_ui_egui::views::build_workspace_layout_tree;
use curvebook_web::{WebBridgeCommand, WebBridgeEvent, WebBridgeSession, WebExportFormat};

type TestRes = Result<(), String>;

fn build_shared_web_tree(session: &WebBridgeSession) -> Result<LayoutTree, String> {
    let layout = session.chrome.build_layout(session.host.app(), session.host.viewport());
    let report = verify_tree_invariants(&layout.tree);
    let filtered = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs(&layout.tree, report.violations);
    if !filtered.is_empty() {
        return Err(format!("web LayoutTree invariants violated: {filtered:?}"));
    }
    Ok(layout.tree)
}

fn assert_web_pixel_parity_clean(session: &mut WebBridgeSession, context: &str) -> TestRes {
    let vp = session.host.viewport();
    let parity = verify_pixel_to_hit_box_parity(session.host.app_mut(), vp);
    let tree = session.host.app().layout_tree_cow(vp);
    let filtered = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs(&tree, parity.violations);
    if !filtered.is_empty() {
        return Err(format!("{context}: {filtered:?}"));
    }
    Ok(())
}

fn find_web_node<'a>(tree: &'a LayoutTree, widget_id: &str) -> Result<&'a WidgetNode, String> {
    tree.node(widget_id).ok_or_else(|| format!("missing web widget `{widget_id}`"))
}

fn logical_center_for_node(tree: &LayoutTree, widget_id: &str) -> Result<[f64; 2], String> {
    let node = find_web_node(tree, widget_id)?;
    let scale = tree.metrics.clamped_scale_factor();
    let [cx, cy] = node.bounds.rect_px.center();
    Ok([f64::from(cx) / scale, f64::from(cy) / scale])
}

fn dispatch_web(session: &mut WebBridgeSession, event: WebBridgeEvent) -> Result<Vec<WebBridgeCommand>, String> {
    session.handle_event(event).map_err(|e| format!("WebBridgeSession::handle_event failed: {e}"))
}

fn click_web_widget(session: &mut WebBridgeSession, widget_id: &str) -> Result<Vec<WebBridgeCommand>, String> {
    let tree = build_shared_web_tree(session)?;
    let [x, y] = logical_center_for_node(&tree, widget_id)?;
    let resolved = hit_test_logical(&tree, x, y).map(|n| (n.id.as_str().to_owned(), n.z_layer, n.bounds.rect_px));
    if resolved.as_ref().map(|r| r.0.as_str()) != Some(widget_id) {
        let target_rect = tree.node(widget_id).map(|n| (n.z_layer, n.bounds.rect_px));
        return Err(format!("hit_test_logical at ({x:.1}, {y:.1}) resolved to {resolved:?}, expected `{widget_id}` ({target_rect:?})"));
    }
    let _ = dispatch_web(session, WebBridgeEvent::UiInput { event: UiInputEvent::PointerMove { x, y, modifiers: KeyModifiers::default() } })?;
    let _ = dispatch_web(
        session,
        WebBridgeEvent::UiInput { event: UiInputEvent::PointerDown { x, y, button: PointerButton::Primary, modifiers: KeyModifiers::default() } },
    )?;
    dispatch_web(
        session,
        WebBridgeEvent::UiInput { event: UiInputEvent::PointerUp { x, y, button: PointerButton::Primary, modifiers: KeyModifiers::default() } },
    )
}

fn hover_web_widget(session: &mut WebBridgeSession, widget_id: &str) -> Result<Vec<WebBridgeCommand>, String> {
    let tree = build_shared_web_tree(session)?;
    let [x, y] = logical_center_for_node(&tree, widget_id)?;
    dispatch_web(session, WebBridgeEvent::UiInput { event: UiInputEvent::PointerMove { x, y, modifiers: KeyModifiers::default() } })
}

fn scroll_web_until_visible(
    session: &mut WebBridgeSession,
    scroll_widget_id: &str,
    target_widget_id: &str,
    step_y: f64,
    max_steps: usize,
) -> TestRes {
    for _ in 0..max_steps {
        let tree = build_shared_web_tree(session)?;
        if tree.node(target_widget_id).is_some() {
            return Ok(());
        }
        let [sx, sy] = logical_center_for_node(&tree, scroll_widget_id)?;
        let _ = dispatch_web(session, WebBridgeEvent::Wheel { x: sx, y: sy, delta_x: 0.0, delta_y: step_y, modifiers: KeyModifiers::default() })?;
    }
    let tree = build_shared_web_tree(session)?;
    if tree.node(target_widget_id).is_some() { Ok(()) } else { Err(format!("scrolling `{scroll_widget_id}` never revealed `{target_widget_id}`")) }
}

fn press_web_key(session: &mut WebBridgeSession, key: &str, shift: bool, meta_or_ctrl: bool) -> Result<Vec<WebBridgeCommand>, String> {
    let modifiers = KeyModifiers { shift, ctrl: meta_or_ctrl, alt: false, meta: meta_or_ctrl };
    dispatch_web(session, WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: key.to_owned(), modifiers } })
}

fn replace_web_textbox(session: &mut WebBridgeSession, widget_id: &str, text: &str) -> TestRes {
    let _ = click_web_widget(session, widget_id)?;
    let _ = press_web_key(session, "a", false, true)?;
    let _ = press_web_key(session, "Backspace", false, false)?;
    let _ = dispatch_web(session, WebBridgeEvent::UiInput { event: UiInputEvent::TextInput { text: text.to_owned() } })?;
    Ok(())
}

fn create_column_table_via_welcome_web(session: &mut WebBridgeSession, title: &str, groups: usize, rows: usize) -> Result<String, String> {
    let _ = click_web_widget(session, "welcome.arch.column")?;
    let _ = click_web_widget(session, "welcome.tab.empty")?;
    replace_web_textbox(session, "welcome.title.input", title)?;

    while session.host.app().nav.welcome.table_draft.data_set_count < groups {
        let _ = click_web_widget(session, "welcome.datasets.inc")?;
    }
    while session.host.app().nav.welcome.table_draft.data_set_count > groups {
        let _ = click_web_widget(session, "welcome.datasets.dec")?;
    }

    if build_shared_web_tree(session)?.node("welcome.rows.inc").is_none() {
        scroll_web_until_visible(session, "welcome.right_body", "welcome.rows.inc", 80.0, 10)?;
    }
    while session.host.app().nav.welcome.table_draft.row_count < rows {
        let _ = click_web_widget(session, "welcome.rows.inc")?;
    }
    while session.host.app().nav.welcome.table_draft.row_count > rows {
        let _ = click_web_widget(session, "welcome.rows.dec")?;
    }

    let _ = click_web_widget(session, "welcome.create_cta")?;
    session
        .host
        .app()
        .document()
        .data_sheets
        .last()
        .map(|s| s.id.as_str().to_owned())
        .ok_or_else(|| String::from("Welcome create did not add a DataSheet on WebBridgeSession"))
}

fn enter_web_grid_cell(session: &mut WebBridgeSession, ds_id: &str, row: usize, col: usize, value: &str) -> TestRes {
    let cell_id = format!("grid.cell.{row}.{col}.0");
    let scroll_id = format!("grid.sheet.{ds_id}");
    if build_shared_web_tree(session)?.node(&cell_id).is_none() {
        scroll_web_until_visible(session, &scroll_id, &cell_id, 120.0, 50)?;
    }
    let tree = build_shared_web_tree(session)?;
    let [x, y] = logical_center_for_node(&tree, &cell_id)?;
    let _ = dispatch_web(
        session,
        WebBridgeEvent::UiInput { event: UiInputEvent::DoubleClick { x, y, button: PointerButton::Primary, modifiers: KeyModifiers::default() } },
    )?;
    let _ = dispatch_web(session, WebBridgeEvent::UiInput { event: UiInputEvent::TextInput { text: value.to_owned() } })?;
    let _ = press_web_key(session, "Enter", false, false)?;
    Ok(())
}

fn rename_web_data_col(session: &mut WebBridgeSession, col: usize, title: &str) -> TestRes {
    let _ = click_web_widget(session, &format!("grid.col_title.{col}"))?;
    replace_web_textbox(session, "grid.title_editor.input", title)?;
    let _ = click_web_widget(session, "grid.title_editor.apply")?;
    Ok(())
}

fn switch_web_sheet(session: &mut WebBridgeSession, sheet_id: &str) -> TestRes {
    let nav_id = format!("nav.sheet.{sheet_id}");
    if build_shared_web_tree(session)?.node(&nav_id).is_none() {
        scroll_web_until_visible(session, "navigator.tree", &nav_id, 90.0, 20)?;
    }
    let _ = click_web_widget(session, &nav_id)?;
    Ok(())
}

fn open_web_top_menu_item(session: &mut WebBridgeSession, menu_id: &str, item_id: &str) -> Result<Vec<WebBridgeCommand>, String> {
    let _ = click_web_widget(session, menu_id)?;
    if build_shared_web_tree(session)?.node(item_id).is_none() {
        scroll_web_until_visible(session, &format!("popup.{menu_id}"), item_id, 96.0, 24)?;
    }
    click_web_widget(session, item_id)
}

fn assert_last_frame_non_empty(cmds: &[WebBridgeCommand], expected_w: u32, expected_h: u32, ctx: &str) -> TestRes {
    let frame = cmds
        .iter()
        .rev()
        .find_map(|c| match c {
            WebBridgeCommand::RenderRgbaFrame { width_px, height_px, rgba_bytes, .. } => Some((*width_px, *height_px, rgba_bytes)),
            _ => None,
        })
        .ok_or_else(|| format!("{ctx}: expected RenderRgbaFrame in bridge commands"))?;
    if frame.0 != expected_w || frame.1 != expected_h {
        return Err(format!("{ctx}: expected {expected_w}x{expected_h} RGBA frame, got {}x{}", frame.0, frame.1));
    }
    let non_white = frame.2.as_chunks::<4>().0.iter().filter(|px| px[3] > 0 && (px[0] < 250 || px[1] < 250 || px[2] < 250)).count();
    if non_white < 50 {
        return Err(format!("{ctx}: frame had too few non-white pixels ({non_white})"));
    }
    Ok(())
}

fn extract_download_bytes(cmds: &[WebBridgeCommand], expected_format: WebExportFormat, ctx: &str) -> Result<Vec<u8>, String> {
    cmds.iter()
        .rev()
        .find_map(|c| match c {
            WebBridgeCommand::SaveOrDownloadFile { bytes, format, .. } if *format == expected_format => Some(bytes.clone()),
            _ => None,
        })
        .ok_or_else(|| format!("{ctx}: expected SaveOrDownloadFile({expected_format:?}) command"))
}

// ============================================================================
// R10.1 Web Spike: End-to-End One-Way ANOVA + Cmd+T EditParams + ResultSheet
// ============================================================================

#[test]
fn r10_1_web_spike_one_way_anova_and_edit_params_human_path() -> TestRes {
    let mut session = WebBridgeSession::new();

    // Stage 1 & 2: Create 3-group Column table & populate via raw UiInput events.
    let ds_id = create_column_table_via_welcome_web(&mut session, "Web ANOVA Spike", 3, 4)?;
    rename_web_data_col(&mut session, 0, "Vehicle")?;
    rename_web_data_col(&mut session, 1, "LowDose")?;
    rename_web_data_col(&mut session, 2, "HighDose")?;

    let cols = [["10", "12", "11", "13"], ["16", "18", "17", "19"], ["28", "30", "29", "31"]];
    for (c, vals) in cols.iter().enumerate() {
        for (r, val) in vals.iter().enumerate() {
            enter_web_grid_cell(&mut session, &ds_id, r, c, val)?;
        }
    }

    // Stage 3: Click Toolbar Analyze -> One-Way ANOVA -> modal.ok (`AnalyzeSheet` commits via chrome_bridge).
    let _ = click_web_widget(&mut session, "toolbar.analyze")?;
    if build_shared_web_tree(&session)?.node("analyze.entry.stats.one_way_anova").is_none() {
        scroll_web_until_visible(&mut session, "analyze.entries_list", "analyze.entry.stats.one_way_anova", 100.0, 24)?;
    }
    let _ = click_web_widget(&mut session, "analyze.entry.stats.one_way_anova")?;
    let analyze_cmds = click_web_widget(&mut session, "modal.ok")?;
    assert_last_frame_non_empty(&analyze_cmds, 1280, 800, "R10.1 Web ANOVA ResultSheet")?;

    let rs_id =
        session.host.app().document().result_sheets.last().map(|r| r.id.as_str().to_owned()).ok_or("expected One-Way ANOVA ResultSheet on Web")?;
    switch_web_sheet(&mut session, &rs_id)?;

    // Stage 4: Press Cmd+T -> verify EditAnalysisParameters opens, ModalBackdrop blocks clicks, Tab stays trapped in modal,
    // toggle `edit.assume_gaussian` off -> hover disabled `edit.post_hoc.tukey` -> select Dunn -> OK -> Kruskal-Wallis.
    let _ = press_web_key(&mut session, "t", false, true)?;
    if !matches!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(format!("expected EditAnalysisParameters after Cmd+T, got {:?}", session.host.app().canvas_dialogs.active_dialog));
    }

    // Verify backdrop blocks clicking underlying chrome coordinates while modal is active.
    let tree = build_shared_web_tree(&session)?;
    let [tb_x, tb_y] = logical_center_for_node(&tree, "toolbar.analyze")?;
    let blocked_cmds = dispatch_web(
        &mut session,
        WebBridgeEvent::UiInput {
            event: UiInputEvent::PointerDown { x: tb_x, y: tb_y, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        },
    )?;
    let blocked_action = blocked_cmds
        .iter()
        .rev()
        .find_map(|c| match c {
            WebBridgeCommand::RenderRgbaFrame { action, .. } => action.clone(),
            _ => None,
        })
        .unwrap_or_default();
    if blocked_action != "modal.backdrop_blocked" {
        return Err(format!("expected ModalBackdrop to block underlying toolbar click, got `{blocked_action}`"));
    }

    // Focus trap check: pressing Tab keeps focused widget inside ModalDialog scope.
    let _ = press_web_key(&mut session, "Tab", false, false)?;
    let tree_after_tab = build_shared_web_tree(&session)?;
    if let Some(fid) = session.host.app().focus_widget_id.as_deref()
        && let Some(node) = tree_after_tab.node(fid)
        && node.a11y.focus_scope != FocusScope::ModalDialog
    {
        return Err(format!("Tab escaped ModalDialog focus scope onto `{fid}`"));
    }

    let _ = click_web_widget(&mut session, "edit.assume_gaussian")?;
    let _ = click_web_widget(&mut session, "edit.tab.multiple_comparisons")?;
    let hover_cmds = hover_web_widget(&mut session, "edit.post_hoc.tukey")?;
    let tukey_disabled = build_shared_web_tree(&session)?.node("edit.post_hoc.tukey").is_some_and(|n| !n.state.enablement.enabled);
    if !tukey_disabled {
        return Err(String::from("expected R2 gate to disable `edit.post_hoc.tukey` in non-Gaussian mode on Web"));
    }
    let _ = hover_cmds;

    let _ = click_web_widget(&mut session, "edit.post_hoc.dunn")?;
    let kw_cmds = click_web_widget(&mut session, "modal.ok")?;
    assert_last_frame_non_empty(&kw_cmds, 1280, 800, "R10.1 Web Kruskal-Wallis")?;

    let kw_debug = format!("{:?}", session.host.app().document().result_sheets.last().map(|r| &r.analysis.kind));
    if !kw_debug.contains("KruskalWallis") {
        return Err(format!("expected KruskalWallis analysis after switching non-Gaussian on Web, got {kw_debug}"));
    }

    // Stage 5: Live upstream DataSheet cell edit refreshes ResultSheet reactively on Web.
    switch_web_sheet(&mut session, &ds_id)?;
    enter_web_grid_cell(&mut session, &ds_id, 0, 2, "45")?;
    switch_web_sheet(&mut session, &rs_id)?;
    let refreshed_cmds = dispatch_web(&mut session, WebBridgeEvent::RenderFrame { dpi: 96 })?;
    assert_last_frame_non_empty(&refreshed_cmds, 1280, 800, "R10.1 Web after upstream edit")?;

    assert_web_pixel_parity_clean(&mut session, "Web pixel-to-hit-box parity violated")?;
    Ok(())
}

// ============================================================================
// Web Human-Path 5-Stage Parity + WF-50 CLD + IME + Export SVG/PDF
// ============================================================================

#[test]
fn web_human_wf50_cld_five_stage_keyboard_ime_and_export_parity() -> TestRes {
    let mut session = WebBridgeSession::new();

    // Stage 1 & 2: Create 3-group Column table & populate via keyboard + IME composition commit on cell/title.
    let ds_id = create_column_table_via_welcome_web(&mut session, "Web WF-50 CLD", 3, 3)?;
    let _ = click_web_widget(&mut session, "grid.col_title.0")?;
    let _ = press_web_key(&mut session, "a", false, true)?;
    let _ = press_web_key(&mut session, "Backspace", false, false)?;
    let _ = dispatch_web(&mut session, WebBridgeEvent::ImePreedit { text: String::from("あ") })?;
    let _ = dispatch_web(&mut session, WebBridgeEvent::ImeCommit { text: String::from("群Alpha") })?;
    let _ = click_web_widget(&mut session, "grid.title_editor.apply")?;
    rename_web_data_col(&mut session, 1, "Beta")?;
    rename_web_data_col(&mut session, 2, "Gamma")?;

    let cols = [["8", "12", "16"], ["13", "17", "21"], ["21", "25", "29"]];
    for (c, vals) in cols.iter().enumerate() {
        for (r, val) in vals.iter().enumerate() {
            enter_web_grid_cell(&mut session, &ds_id, r, c, val)?;
        }
    }

    // Stage 3: Run One-Way ANOVA -> enable Tukey HSD via Cmd+T (`OneWayAnovaSpec::default()` has MultiplicityMethod::None)
    // -> GraphSheet -> right-hand Pairwise Inspector CLD controls (`WF-50`).
    let _ = click_web_widget(&mut session, "toolbar.analyze")?;
    let _ = click_web_widget(&mut session, "analyze.entry.stats.one_way_anova")?;
    let _ = click_web_widget(&mut session, "modal.ok")?;

    let rs_id = session.host.app().document().result_sheets.last().map(|r| r.id.as_str().to_owned()).ok_or("expected Web WF-50 ResultSheet")?;
    switch_web_sheet(&mut session, &rs_id)?;
    let _ = press_web_key(&mut session, "t", false, true)?;
    let _ = click_web_widget(&mut session, "edit.tab.multiple_comparisons")?;
    let _ = click_web_widget(&mut session, "edit.post_hoc.tukey")?;
    let _ = click_web_widget(&mut session, "modal.ok")?;

    let gs_id = session.host.app().document().graph_sheets.last().map(|g| g.id.as_str().to_owned()).ok_or("missing Web WF-50 GraphSheet")?;
    switch_web_sheet(&mut session, &gs_id)?;
    if matches!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = click_web_widget(&mut session, "modal.ok")?;
    }

    let _ = click_web_widget(&mut session, "inspector.tab.pairwise")?;
    let _ = click_web_widget(&mut session, "inspector.pw.auto_brackets")?;
    let _ = click_web_widget(&mut session, "inspector.pw.cld_toggle")?;
    let _ = click_web_widget(&mut session, "inspector.pw.cld_mode_btn")?;
    let _ = click_web_widget(&mut session, "inspector.popup.cld_mode.item_1")?;

    // Stage 4: Export SVG & %PDF-1.7 cleanly via ExportFigure modal -> inspect CLD letters in SVG bytes.
    let _ = open_web_top_menu_item(&mut session, "menu.file", "menu.ui.export.figure.open")?;
    let _ = click_web_widget(&mut session, "export.fmt.svg")?;
    let svg_cmds = click_web_widget(&mut session, "modal.ok")?;
    let svg_bytes = extract_download_bytes(&svg_cmds, WebExportFormat::Svg, "Web WF-50 CLD SVG")?;
    let svg_before = String::from_utf8(svg_bytes.clone()).map_err(|e| format!("invalid SVG UTF-8: {e}"))?;
    if !svg_before.contains("群Alpha") {
        return Err(String::from("Web WF-50 CLD SVG missing expected IME column title `群Alpha`"));
    }
    if !svg_before.contains("ab") {
        return Err(String::from("Web WF-50 CLD SVG missing expected Piepho CLD letter `ab`"));
    }
    let _ = click_web_widget(&mut session, "inspector.pw.order_asc")?;
    let _ = click_web_widget(&mut session, "inspector.pw.cld_case")?;

    let _ = open_web_top_menu_item(&mut session, "menu.file", "menu.ui.export.figure.open")?;
    let _ = click_web_widget(&mut session, "export.fmt.pdf")?;
    let pdf_cmds = click_web_widget(&mut session, "modal.ok")?;
    let pdf_bytes = extract_download_bytes(&pdf_cmds, WebExportFormat::Pdf, "Web WF-50 CLD PDF")?;
    if !pdf_bytes.starts_with(b"%PDF-1.7") {
        return Err(String::from("Web WF-50 export did not emit valid %PDF-1.7 bytes"));
    }

    // Stage 5: Live upstream DataSheet cell edit refreshes Web GraphSheet CLD letters in exported SVG.
    switch_web_sheet(&mut session, &ds_id)?;
    enter_web_grid_cell(&mut session, &ds_id, 0, 0, "80")?;
    switch_web_sheet(&mut session, &gs_id)?;
    let _ = open_web_top_menu_item(&mut session, "menu.file", "menu.ui.export.figure.open")?;
    let _ = click_web_widget(&mut session, "export.fmt.svg")?;
    let svg_after_cmds = click_web_widget(&mut session, "modal.ok")?;
    let svg_after_bytes = extract_download_bytes(&svg_after_cmds, WebExportFormat::Svg, "Web WF-50 refreshed SVG")?;
    if svg_after_bytes == svg_bytes {
        return Err(String::from("Web WF-50 CLD SVG did not update after upstream cell edit"));
    }
    Ok(())
}

// ============================================================================
// Web Large-N Wheel Scrolling + 1.0x/2.0x HiDPI + RTL Layout Parity
// ============================================================================

#[test]
fn web_human_hidpi_1x_2x_and_rtl_layout_parity() -> TestRes {
    let mut session = WebBridgeSession::new();
    let ds_id = create_column_table_via_welcome_web(&mut session, "Web HiDPI RTL", 2, 4)?;
    enter_web_grid_cell(&mut session, &ds_id, 0, 0, "12.5")?;

    let frame_1x = dispatch_web(&mut session, WebBridgeEvent::RenderFrame { dpi: 96 })?;
    assert_last_frame_non_empty(&frame_1x, 1280, 800, "Web 1.0x frame")?;

    let frame_2x = dispatch_web(&mut session, WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 2.0 })?;
    assert_last_frame_non_empty(&frame_2x, 2560, 1600, "Web 2.0x HiDPI frame")?;
    assert_web_pixel_parity_clean(&mut session, "Web 2.0x HiDPI parity violated")?;

    // RTL mirroring check: navigator panel mirrors right of canvas under Arabic (`ar`) locale.
    let _ = session.host.app_mut().set_locale("ar");
    let frame_rtl = dispatch_web(&mut session, WebBridgeEvent::RenderFrame { dpi: 192 })?;
    assert_last_frame_non_empty(&frame_rtl, 2560, 1600, "Web 2.0x RTL frame")?;
    let rtl_tree = build_shared_web_tree(&session)?;
    assert_web_pixel_parity_clean(&mut session, "Web 2.0x RTL parity violated")?;
    if !rtl_tree.is_rtl || rtl_tree.frame.navigator.x <= rtl_tree.frame.canvas.x {
        return Err(format!(
            "expected RTL navigator mirrored right of canvas, got is_rtl={}, nav_x={}, canvas_x={}",
            rtl_tree.is_rtl, rtl_tree.frame.navigator.x, rtl_tree.frame.canvas.x
        ));
    }
    Ok(())
}

#[test]
fn web_human_large_n_wheel_scroll_parity() -> TestRes {
    let mut session = WebBridgeSession::new();
    let ds_id = create_column_table_via_welcome_web(&mut session, "Web LargeN", 2, 48)?;

    // Scroll large-N grid with raw WebBridgeEvent::Wheel and edit bottom row 45 via DoubleClick + TextInput.
    enter_web_grid_cell(&mut session, &ds_id, 45, 1, "98.75")?;
    let scrolled_tree = build_shared_web_tree(&session)?;
    let row45_node = find_web_node(&scrolled_tree, "grid.cell.45.1.0")?;
    let cell_text = row45_node.a11y.value_text.as_deref().unwrap_or(row45_node.a11y.label.as_str());
    if !cell_text.contains("98.75") {
        return Err(format!("expected 98.75 at scrolled `grid.cell.45.1.0`, got `{cell_text}`"));
    }
    Ok(())
}

#[test]
fn web_fontless_cjk_rtl_filter_preserves_ascii_and_geometry_invariants() -> TestRes {
    let fallback_measurer = DesktopTextMeasurer::with_font_book(FontBook::with_embedded_fallback());
    let mut session = WebBridgeSession::new();
    let _ds_id = create_column_table_via_welcome_web(&mut session, "Web Fontless Filter", 2, 3)?;

    // 1. Reproduce the headless CI `grid.title_editor.input` (`群Alpha`) failure under embedded fallback.
    let _ = click_web_widget(&mut session, "grid.col_title.0")?;
    let _ = press_web_key(&mut session, "a", false, true)?;
    let _ = press_web_key(&mut session, "Backspace", false, false)?;
    let _ = dispatch_web(&mut session, WebBridgeEvent::ImeCommit { text: String::from("群Alpha") })?;

    let palette = FramePalette::from_theme(&session.host.app().theme);
    let tree = build_workspace_layout_tree(session.host.app(), session.host.viewport(), &fallback_measurer, &palette);
    let raw_report = verify_tree_invariants(&tree);
    if !raw_report.violations.iter().any(|v| v.widget_id == "grid.title_editor.input" && v.check == "text_glyphs") {
        return Err(format!("expected fontless fallback to trigger text_glyphs on `grid.title_editor.input`, got {:?}", raw_report.violations));
    }

    // Unbacked CJK host coverage filters only the unbacked CJK `text_glyphs` violation.
    let filtered_none = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &tree,
        raw_report.violations.clone(),
        web_parity_common::HostScriptCoverage::none(),
    );
    if !filtered_none.is_empty() {
        return Err(format!("expected fontless CJK violations to filter cleanly, got {filtered_none:?}"));
    }

    // When host coverage claims CJK support, CJK missing glyphs are strictly preserved as failures.
    let preserved_when_backed = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &tree,
        raw_report.violations,
        web_parity_common::HostScriptCoverage::all(),
    );
    if !preserved_when_backed.iter().any(|v| v.widget_id == "grid.title_editor.input" && v.check == "text_glyphs") {
        return Err(String::from("expected CJK text_glyphs violation to remain when host reports full coverage"));
    }

    // 2. Non-`text_glyphs` geometric violations and ASCII/Latin `text_glyphs` failures are never filtered.
    let mut synthetic = vec![
        ParityViolation {
            widget_id: String::from("grid.title_editor.input"),
            check: String::from("text_unclipped"),
            detail: String::from("synthetic clip overflow"),
        },
        ParityViolation {
            widget_id: String::from("grid.title_editor.apply"),
            check: String::from("text_glyphs"),
            detail: String::from("text run `Apply` has 1 missing glyph(s)"),
        },
    ];
    // Also verify mixed CJK + unbacked non-CJK/PUA codepoint inside a real tree run is preserved.
    let mut pua_session = session.clone();
    let _ = press_web_key(&mut pua_session, "a", false, true)?;
    let _ = press_web_key(&mut pua_session, "Backspace", false, false)?;
    let _ = dispatch_web(&mut pua_session, WebBridgeEvent::ImeCommit { text: String::from("群\u{E000}") })?;
    let pua_tree = build_workspace_layout_tree(pua_session.host.app(), pua_session.host.viewport(), &fallback_measurer, &palette);
    let pua_report = verify_tree_invariants(&pua_tree);
    let pua_filtered = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &pua_tree,
        pua_report.violations,
        web_parity_common::HostScriptCoverage::none(),
    );
    if !pua_filtered.iter().any(|v| v.widget_id == "grid.title_editor.input" && v.check == "text_glyphs") {
        return Err(String::from("expected mixed CJK + non-CJK missing glyph run to remain un-filtered"));
    }

    let kept = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &tree,
        std::mem::take(&mut synthetic),
        web_parity_common::HostScriptCoverage::none(),
    );
    if kept.len() != 2 {
        return Err(format!("expected geometric + ASCII violations to be preserved, got {kept:?}"));
    }

    // 3. Apply `群Alpha`, switch to Arabic (`ar`) RTL, and verify fontless tree passes after CJK/RTL filtering.
    let _ = click_web_widget(&mut session, "grid.title_editor.apply")?;
    let _ = session.host.app_mut().set_locale("ar");
    let rtl_tree = build_workspace_layout_tree(session.host.app(), session.host.viewport(), &fallback_measurer, &palette);
    let rtl_report = verify_tree_invariants(&rtl_tree);
    if !rtl_report.violations.iter().any(|v| v.check == "text_glyphs") {
        return Err(String::from("expected fontless Arabic locale to emit text_glyphs violations"));
    }
    let rtl_filtered = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &rtl_tree,
        rtl_report.violations,
        web_parity_common::HostScriptCoverage::none(),
    );
    if !rtl_filtered.is_empty() {
        return Err(format!("expected fontless Arabic RTL tree to have zero non-CJK/RTL violations, got {rtl_filtered:?}"));
    }
    Ok(())
}
