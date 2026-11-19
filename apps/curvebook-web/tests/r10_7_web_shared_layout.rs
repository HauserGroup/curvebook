//! `R10.7` Interactive Web Shared `LayoutTree` Integration Tests (`ROADMAP.md` R10.7 bullet 1).
//!
//! Verifies that `apps/curvebook-web` consumes and renders shared
//! `curvebook_ui_egui::views::build_workspace_layout_tree` / `LayoutTree` primitives through
//! `WebTextBridge`, `BrowserCanvasTextMeasurer`, and `WebLayeredFrame` across:
//! 1. Menu bar & open menu dropdown popups,
//! 2. Toolbar buttons, tooltips & enablement,
//! 3. Dialogs / modals / command palette & modal focus trapping,
//! 4. Context menus & navigator / workspace slots, HiDPI, and RTL mirroring.
#![forbid(unsafe_code)]

mod web_parity_common;

use std::error::Error;
use std::fmt;

use curvebook_ui_egui::compositor::FramePalette;
use curvebook_ui_egui::layout::verify::{ParityViolation, verify_tree_invariants};
use curvebook_ui_egui::layout::{FocusScope, LayoutTree, WidgetAction};
use curvebook_ui_egui::serde_json::json;
use curvebook_ui_egui::text_bridge::DesktopTextMeasurer;
use curvebook_ui_egui::theme::FontBook;
use curvebook_ui_egui::views::build_workspace_layout_tree;
use curvebook_ui_egui::{ActiveDialog, KeyModifiers, PointerButton, UiInputEvent, execute_widget_action};
use curvebook_web::{
    WebBridgeCommand, WebBridgeEvent, WebBridgeSession, WebChromeLayout, WebExportFormat, WebTextAlign, WebTextDirection, ZLayer, ZLayerBand,
};

#[derive(Debug)]
struct TestMsg(String);

impl fmt::Display for TestMsg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for TestMsg {}

type TestRes<T = ()> = Result<T, Box<dyn Error>>;

fn fail<T>(msg: impl Into<String>) -> TestRes<T> {
    Err(Box::new(TestMsg(msg.into())))
}

fn check(cond: bool, msg: impl Into<String>) -> TestRes {
    if cond { Ok(()) } else { fail(msg) }
}

fn current_layout(session: &WebBridgeSession) -> WebChromeLayout {
    session.chrome.build_layout(session.host.app(), session.host.viewport())
}

fn verify_shared_tree_clean(session: &WebBridgeSession, context: &str) -> TestRes<LayoutTree> {
    let layout = current_layout(session);
    let report = verify_tree_invariants(&layout.tree);
    let filtered = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs(&layout.tree, report.violations);
    if !filtered.is_empty() {
        return fail(format!("LayoutTree invariants failed at {context}: {filtered:?}"));
    }
    Ok(layout.tree)
}

fn node_center_logical(session: &WebBridgeSession, node_id: &str) -> TestRes<[f64; 2]> {
    let layout = current_layout(session);
    let Some(node) = layout.tree.node(node_id) else {
        return fail(format!("shared LayoutTree node `{node_id}` not found"));
    };
    let [cx, cy] = node.bounds.visible_rect_px.center();
    Ok(session.host.viewport().physical_to_logical_px([f64::from(cx), f64::from(cy)]))
}

fn pointer_click_node(session: &mut WebBridgeSession, node_id: &str) -> TestRes<Vec<WebBridgeCommand>> {
    let [lx, ly] = node_center_logical(session, node_id)?;
    let _ =
        session.handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::PointerMove { x: lx, y: ly, modifiers: KeyModifiers::default() } })?;
    let _ = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?;
    Ok(session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerUp { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?)
}

fn pointer_right_click_node(session: &mut WebBridgeSession, node_id: &str) -> TestRes<Vec<WebBridgeCommand>> {
    let [lx, ly] = node_center_logical(session, node_id)?;
    let _ =
        session.handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::PointerMove { x: lx, y: ly, modifiers: KeyModifiers::default() } })?;
    Ok(session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: lx, y: ly, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
    })?)
}

fn hover_node(session: &mut WebBridgeSession, node_id: &str) -> TestRes<Vec<WebBridgeCommand>> {
    let [lx, ly] = node_center_logical(session, node_id)?;
    Ok(session.handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::PointerMove { x: lx, y: ly, modifiers: KeyModifiers::default() } })?)
}

fn scroll_container_until_visible(session: &mut WebBridgeSession, container_node_id: &str, target_node_id: &str) -> TestRes {
    for dir in [1.0, -1.0] {
        for _ in 0..24 {
            if current_layout(session).tree.node(target_node_id).is_some() {
                return Ok(());
            }
            let [lx, ly] = node_center_logical(session, container_node_id)?;
            let _ = session.handle_event(WebBridgeEvent::Wheel {
                x: lx,
                y: ly,
                delta_x: 0.0,
                delta_y: dir * 88.0,
                modifiers: KeyModifiers::default(),
            })?;
        }
    }
    fail(format!("node `{target_node_id}` did not enter `{container_node_id}` viewport after scrolling"))
}

type FrameTuple<'a> = (&'a [curvebook_web::WebRgbaLayer], &'a [curvebook_web::WebTextDrawOp], &'a curvebook_web::WebChromeSnapshot, Option<&'a str>);

fn first_frame(cmds: &[WebBridgeCommand]) -> TestRes<FrameTuple<'_>> {
    for cmd in cmds {
        if let WebBridgeCommand::RenderRgbaFrame { rgba_layers, text_ops, chrome, action, .. } = cmd {
            return Ok((rgba_layers, text_ops, chrome, action.as_deref()));
        }
    }
    fail(format!("expected RenderRgbaFrame in commands: {cmds:?}"))
}

fn build_populated_session() -> TestRes<(WebBridgeSession, String, String, String)> {
    let mut session = WebBridgeSession::new();
    let _ = session.handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })?;
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand {
        command_id: String::from("sheet.createData"),
        params: json!({ "sheet_id": "d1", "kind": "column", "title": "Groups" }),
    })?;
    for (col, base) in [(0usize, 10.0), (1usize, 18.0)] {
        for row in 0..6usize {
            let _ = session.handle_event(WebBridgeEvent::ExecuteCommand {
                command_id: String::from("table.setCell"),
                params: json!({ "sheet_id": "d1", "row": row, "col": col, "value": base + row as f64 }),
            })?;
        }
    }
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand {
        command_id: String::from("graph.create"),
        params: json!({ "source_sheet_id": "d1", "sheet_id": "g1", "title": "Groups Plot" }),
    })?;
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand {
        command_id: String::from("analysis.run"),
        params: json!({ "source_sheet_id": "d1", "kind": "t_test" }),
    })?;
    let rs_id = session
        .host
        .app()
        .document()
        .result_sheets
        .first()
        .map(|s| s.id.as_str().to_owned())
        .ok_or_else(|| TestMsg(String::from("expected ResultSheet after ANOVA run")))?;
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": "d1" }) })?;
    Ok((session, String::from("d1"), String::from("g1"), rs_id))
}

#[test]
fn shared_tree_menu_bar_dropdowns_scroll_drag_cancel_and_qol7_tooltip() -> TestRes {
    let (mut session, _d1, _g1, _rs_id) = build_populated_session()?;

    // Compare WebChromeLayout geometry directly against `build_workspace_layout_tree`.
    let raw_tree = build_workspace_layout_tree(
        session.host.app(),
        session.host.viewport(),
        DesktopTextMeasurer::global(),
        &FramePalette::from_theme(&session.host.app().theme),
    );
    let web_tree = verify_shared_tree_clean(&session, "idle_workspace")?;
    let raw_file = raw_tree.node("menu.file").ok_or_else(|| TestMsg(String::from("raw menu.file missing")))?;
    let web_file = web_tree.node("menu.file").ok_or_else(|| TestMsg(String::from("web menu.file missing")))?;
    check(raw_file.bounds.rect_px == web_file.bounds.rect_px, "menu.file rect must come directly from build_workspace_layout_tree")?;

    // Open `menu.file` via physical PointerMove -> PointerDown -> PointerUp at node center.
    let open_cmds = pointer_click_node(&mut session, "menu.file")?;
    let (layers, text_ops, chrome, _) = first_frame(&open_cmds)?;
    check(chrome.open_menu_key.as_deref() == Some("menu.file"), "File menu must open cleanly")?;
    check(layers.iter().any(|l| l.z_band == ZLayerBand::PopupDropdown && !l.uses_base_frame), "PopupDropdown RGBA layer must exist")?;
    check(text_ops.iter().any(|op| op.z_band == ZLayerBand::PopupDropdown && op.clip_px.is_some()), "PopupDropdown clipped text ops must exist")?;
    let _ = verify_shared_tree_clean(&session, "open_menu_file")?;

    // Drag-off cancel: PointerDown on `menu.edit`, move onto workspace, PointerUp outside -> stays on `menu.file`.
    let [edit_x, edit_y] = node_center_logical(&session, "menu.edit")?;
    let _ = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: edit_x, y: edit_y, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?;
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::PointerMove { x: 600.0, y: 500.0, modifiers: KeyModifiers::default() } })?;
    let _ = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerUp { x: 600.0, y: 500.0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?;
    check(session.chrome.open_menu_key.as_deref() == Some("menu.file"), "dragging off menu.edit before releasing must cancel switch")?;

    // Clicking `menu.edit` directly while `menu.file` is open switches popup cleanly.
    let switch_cmds = pointer_click_node(&mut session, "menu.edit")?;
    let (_, _, switch_chrome, _) = first_frame(&switch_cmds)?;
    check(switch_chrome.open_menu_key.as_deref() == Some("menu.edit"), "clicking menu.edit switches active dropdown")?;

    // Reopen `menu.file`, wheel-scroll popup to `menu.project.open`, and click via coordinates.
    let _ = pointer_click_node(&mut session, "menu.file")?;
    scroll_container_until_visible(&mut session, "popup.menu.file", "menu.project.open")?;
    let picker_cmds = pointer_click_node(&mut session, "menu.project.open")?;
    check(
        matches!(picker_cmds.as_slice(), [WebBridgeCommand::TriggerFilePicker { directory: false, .. }]),
        "menu.project.open must emit TriggerFilePicker",
    )?;

    // Open `menu.view` and verify `menu.ui.event.dispatch` is omitted across entire scroll range.
    let _ = pointer_click_node(&mut session, "menu.view")?;
    for _ in 0..16 {
        let view_tree = verify_shared_tree_clean(&session, "open_menu_view_scroll")?;
        check(view_tree.node("menu.ui.event.dispatch").is_none(), "automation hook must never appear in web menu.view")?;
        let [vx, vy] = node_center_logical(&session, "popup.menu.view")?;
        let _ = session.handle_event(WebBridgeEvent::Wheel { x: vx, y: vy, delta_x: 0.0, delta_y: 88.0, modifiers: KeyModifiers::default() })?;
    }

    // Open `menu.analyze`, scroll to QOL-7 `menu.ui.result.applyRemediation`, verify disabled + hover tooltip in TopOverlay.
    let _ = pointer_click_node(&mut session, "menu.analyze")?;
    scroll_container_until_visible(&mut session, "popup.menu.analyze", "menu.ui.result.applyRemediation")?;
    let analyze_tree = verify_shared_tree_clean(&session, "open_menu_analyze")?;
    let rem_node = analyze_tree.node("menu.ui.result.applyRemediation").ok_or_else(|| TestMsg(String::from("applyRemediation node missing")))?;
    check(!rem_node.state.enablement.enabled, "applyRemediation must stay disabled per QOL-7")?;

    let hover_cmds = hover_node(&mut session, "menu.ui.result.applyRemediation")?;
    let (tip_layers, tip_text_ops, _, _) = first_frame(&hover_cmds)?;
    check(tip_layers.iter().any(|l| l.z_band == ZLayerBand::TopOverlay), "TopOverlay tooltip layer must render for disabled menu item")?;
    check(
        tip_text_ops.iter().any(|op| op.z_layer == ZLayer::Tooltip && op.text.contains("QOL-7")),
        "Tooltip text op must display QOL-7 disabled reason",
    )?;

    let disabled_click = pointer_click_node(&mut session, "menu.ui.result.applyRemediation")?;
    let (_, _, _, disabled_action) = first_frame(&disabled_click)?;
    check(
        disabled_action.is_some_and(|a| a.starts_with("chrome.disabled:")),
        "clicking disabled QOL-7 item must return disabled notice without mutating",
    )?;

    // Clicking workspace outside popup dismisses menu without click-through, and Escape dismisses if open.
    let dismiss_cmds = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 600.0, y: 400.0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?;
    let (_, _, after_dismiss, dismiss_act) = first_frame(&dismiss_cmds)?;
    check(after_dismiss.open_menu_key.is_none(), "clicking outside open menu dismisses popup")?;
    check(dismiss_act == Some("chrome.popup_dismissed"), "outside click must report popup_dismissed without click-through")?;

    let _ = pointer_click_node(&mut session, "menu.file")?;
    let esc_cmds = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;
    let (_, _, after_esc, _) = first_frame(&esc_cmds)?;
    check(after_esc.open_menu_key.is_none(), "Escape dismisses open menu")?;

    Ok(())
}

#[test]
fn shared_tree_toolbar_enablement_tooltips_and_execution() -> TestRes {
    let mut empty_session = WebBridgeSession::new();
    let _ = empty_session.handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })?;
    let empty_tree = verify_shared_tree_clean(&empty_session, "empty_toolbar")?;
    for enabled_id in ["toolbar.new", "toolbar.open", "toolbar.palette"] {
        let node = empty_tree.node(enabled_id).ok_or_else(|| TestMsg(format!("missing {enabled_id}")))?;
        check(node.state.enablement.enabled, format!("{enabled_id} should be enabled on empty project"))?;
    }
    for disabled_id in ["toolbar.analyze", "toolbar.axes", "toolbar.export_svg"] {
        let node = empty_tree.node(disabled_id).ok_or_else(|| TestMsg(format!("missing {disabled_id}")))?;
        check(!node.state.enablement.enabled, format!("{disabled_id} should be disabled on empty project"))?;
        check(node.state.enablement.disabled_reason.is_some(), format!("{disabled_id} needs disabled_reason"))?;
    }

    // Hovering disabled `toolbar.export_svg` emits TopOverlay tooltip; clicking it gives disabled notice without export.
    let tip_cmds = hover_node(&mut empty_session, "toolbar.export_svg")?;
    let (layers, text_ops, _, _) = first_frame(&tip_cmds)?;
    check(layers.iter().any(|l| l.z_band == ZLayerBand::TopOverlay), "disabled toolbar hover emits TopOverlay")?;
    check(text_ops.iter().any(|op| op.z_layer == ZLayer::Tooltip), "disabled toolbar hover emits Tooltip text op")?;
    let disabled_cmds = pointer_click_node(&mut empty_session, "toolbar.export_svg")?;
    check(!disabled_cmds.iter().any(|c| matches!(c, WebBridgeCommand::SaveOrDownloadFile { .. })), "disabled toolbar.export_svg must not export")?;

    // Populated session enables analyze, axes, export_svg and renders enabled tooltip + runs actions.
    let (mut session, _d1, _g1, _rs_id) = build_populated_session()?;
    let pop_tree = verify_shared_tree_clean(&session, "populated_toolbar")?;
    check(pop_tree.node("toolbar.analyze").is_some_and(|n| n.state.enablement.enabled), "toolbar.analyze must enable once DataSheet exists")?;
    let en_hover = hover_node(&mut session, "toolbar.analyze")?;
    let (_, en_tips, _, _) = first_frame(&en_hover)?;
    check(en_tips.iter().any(|op| op.z_layer == ZLayer::Tooltip), "enabled toolbar hover emits Tooltip text op")?;

    let svg_dlg_cmds = pointer_click_node(&mut session, "toolbar.export_svg")?;
    let (_, _, svg_dlg_chrome, _) = first_frame(&svg_dlg_cmds)?;
    check(svg_dlg_chrome.active_dialog.contains("ExportFigure"), "toolbar.export_svg opens ExportFigure dialog")?;
    let svg_cmds = pointer_click_node(&mut session, "modal.ok")?;
    check(
        svg_cmds.iter().any(|c| matches!(c, WebBridgeCommand::SaveOrDownloadFile { format: WebExportFormat::Svg, .. })),
        "confirming ExportFigure dialog triggers SVG SaveOrDownloadFile",
    )?;

    let _ = pointer_click_node(&mut session, "menu.file")?;
    scroll_container_until_visible(&mut session, "popup.menu.file", "menu.export.svg")?;
    let direct_svg_cmds = pointer_click_node(&mut session, "menu.export.svg")?;
    check(
        matches!(direct_svg_cmds.as_slice(), [WebBridgeCommand::SaveOrDownloadFile { format: WebExportFormat::Svg, .. }]),
        "menu.export.svg triggers immediate SVG export",
    )?;

    let axes_cmds = pointer_click_node(&mut session, "toolbar.axes")?;
    let (axes_layers, _, axes_chrome, _) = first_frame(&axes_cmds)?;
    check(axes_chrome.active_dialog.contains("FormatAxes"), "toolbar.axes opens FormatAxes")?;
    check(axes_layers.iter().any(|l| l.z_band == ZLayerBand::ModalOverlay), "FormatAxes renders ModalOverlay")?;

    let close_cmds = pointer_click_node(&mut session, "modal.close")?;
    let (_, _, closed_chrome, _) = first_frame(&close_cmds)?;
    check(closed_chrome.active_dialog == "None", "modal.close closes FormatAxes cleanly")?;

    let _ = pointer_click_node(&mut session, "toolbar.new")?;
    let _ = pointer_click_node(&mut session, "toolbar.undo")?;
    let _ = pointer_click_node(&mut session, "toolbar.redo")?;
    Ok(())
}

#[test]
fn shared_tree_modals_command_palette_focus_trapping_ime_and_backdrop() -> TestRes {
    let (mut session, _d1, _g1, rs_id) = build_populated_session()?;

    // 1. AnalyzeSheet: open via toolbar.analyze, type filter, click category & entry, confirm -> ResultSheet; re-run triggers DuplicateAnalysisPrompt.
    let _ = pointer_click_node(&mut session, "toolbar.analyze")?;
    let analyze_tree = verify_shared_tree_clean(&session, "analyze_modal")?;
    check(analyze_tree.active_focus_scope == FocusScope::ModalDialog, "AnalyzeSheet uses ModalDialog focus scope")?;
    let _ = pointer_click_node(&mut session, "analyze.cat.column")?;
    for ch in ["d", "e", "s", "c"] {
        let _ = session
            .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from(ch), modifiers: KeyModifiers::default() } })?;
    }
    scroll_container_until_visible(&mut session, "analyze.entries_list", "analyze.entry.stats.descriptive")?;
    let before_rs_count = session.host.app().document().result_sheets.len();
    let _ = pointer_click_node(&mut session, "analyze.entry.stats.descriptive")?;
    let _ = pointer_click_node(&mut session, "modal.ok")?;
    if matches!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        let _ = pointer_click_node(&mut session, "modal.ok")?;
    }
    check(session.host.app().document().result_sheets.len() > before_rs_count, "AnalyzeSheet confirmation should produce a new ResultSheet")?;

    // Re-running stats.descriptive on the same source opens DuplicateAnalysisPrompt cleanly.
    let _ = pointer_click_node(&mut session, "toolbar.analyze")?;
    scroll_container_until_visible(&mut session, "analyze.entries_list", "analyze.entry.stats.descriptive")?;
    let _ = pointer_click_node(&mut session, "analyze.entry.stats.descriptive")?;
    let dup_cmds = pointer_click_node(&mut session, "modal.ok")?;
    let (_, _, dup_chrome, _) = first_frame(&dup_cmds)?;
    check(dup_chrome.active_dialog.contains("DuplicateAnalysisPrompt"), "Re-running existing analysis triggers DuplicateAnalysisPrompt")?;
    let _ = pointer_click_node(&mut session, "modal.cancel")?;

    // 2. EditAnalysisParameters: open via shared `result.edit_params_btn`, test tab switch + checkbox + confirm.
    let _ =
        session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": rs_id }) })?;
    let _ = pointer_click_node(&mut session, "result.edit_params_btn")?;
    let edit_tree = verify_shared_tree_clean(&session, "edit_params_modal")?;
    check(edit_tree.node("edit.assume_gaussian").is_some(), "edit.assume_gaussian must render in EditAnalysisParameters")?;
    let _ = pointer_click_node(&mut session, "edit.assume_gaussian")?;
    if current_layout(&session).tree.node("edit.tab.multiple_comparisons").is_some() {
        let _ = pointer_click_node(&mut session, "edit.tab.multiple_comparisons")?;
    }
    let _ = pointer_click_node(&mut session, "edit.tab.experimental_design")?;
    let _ = pointer_click_node(&mut session, "modal.ok")?;
    check(current_layout(&session).snapshot.active_dialog == "None", "modal.ok closes EditAnalysisParameters cleanly")?;

    // 3. PowerAndSampleSize wizard stages render in shared LayoutTree.
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("ui.powerWizard.open"), params: json!({}) })?;
    let power_tree = verify_shared_tree_clean(&session, "power_wizard_modal")?;
    check(power_tree.node("power.stage.1").is_some() && power_tree.node("power.stage.4").is_some(), "PowerWizard stages present")?;
    let _ = pointer_click_node(&mut session, "modal.cancel")?;

    // 4. CommandPalette: Alt+K opens, focuses `palette.query`, Tab traps inside modal, backdrop blocks click/double-click/wheel, IME commits & Enter executes.
    let pal_open = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::KeyDown { key: String::from("k"), modifiers: KeyModifiers { alt: true, ..Default::default() } },
    })?;
    let (_, _, pal_chrome, _) = first_frame(&pal_open)?;
    check(pal_chrome.active_dialog.contains("CommandPalette"), "Alt+K opens CommandPalette")?;
    check(pal_chrome.focused_node_id.as_deref() == Some("palette.query"), "CommandPalette focuses palette.query")?;
    let pal_tree = verify_shared_tree_clean(&session, "command_palette")?;

    // Tab cycle must remain strictly inside ModalDialog nodes.
    for _ in 0..8 {
        let tab_cmds = session.handle_event(WebBridgeEvent::UiInput {
            event: UiInputEvent::KeyDown { key: String::from("Tab"), modifiers: KeyModifiers::default() },
        })?;
        let (_, _, tab_chrome, _) = first_frame(&tab_cmds)?;
        if let Some(fid) = tab_chrome.focused_node_id.as_deref() {
            let fnode = pal_tree.node(fid).ok_or_else(|| TestMsg(format!("focused modal node {fid} missing")))?;
            check(fnode.z_layer >= curvebook_ui_egui::layout::ZLayer::ModalDialog, format!("Tab focus escaped modal to {fid}"))?;
        }
    }

    // Backdrop blocks PointerDown, DoubleClick, and Wheel behind modal.
    let blk_down = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 4.0, y: 4.0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?;
    check(first_frame(&blk_down)?.3 == Some("modal.backdrop_blocked"), "backdrop blocks PointerDown")?;
    let blk_dbl = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::DoubleClick { x: 4.0, y: 4.0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?;
    check(first_frame(&blk_dbl)?.3 == Some("modal.backdrop_blocked"), "backdrop blocks DoubleClick")?;
    let blk_wheel =
        session.handle_event(WebBridgeEvent::Wheel { x: 4.0, y: 4.0, delta_x: 0.0, delta_y: 40.0, modifiers: KeyModifiers::default() })?;
    check(first_frame(&blk_wheel)?.3 == Some("modal.backdrop_blocked"), "backdrop blocks background Wheel")?;

    // IME preedit + commit + key typing populate `palette.query` and `Enter` executes selected hit.
    let preedit_cmds = session.handle_event(WebBridgeEvent::ImePreedit { text: String::from("cre") })?;
    check(first_frame(&preedit_cmds)?.2.ime_preedit.as_deref() == Some("cre"), "IME preedit reflected in snapshot")?;
    let commit_cmds = session.handle_event(WebBridgeEvent::ImeCommit { text: String::from("sheet.createData") })?;
    check(first_frame(&commit_cmds)?.2.ime_preedit.is_none(), "IME commit clears preedit")?;
    let before_ds = session.host.app().document().data_sheets.len();
    let enter_cmds = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Enter"), modifiers: KeyModifiers::default() } })?;
    check(first_frame(&enter_cmds)?.2.active_dialog == "None", "Enter closes CommandPalette after running hit")?;
    check(session.host.app().document().data_sheets.len() == before_ds + 1, "CommandPalette created new DataSheet")?;

    Ok(())
}

#[test]
fn shared_tree_context_menus_navigator_workspace_slots_hidpi_and_rtl() -> TestRes {
    let (mut session, d1, g1, rs_id) = build_populated_session()?;

    // 1. Right-click DataSheet grid -> ZLayer::ContextMenu in TopOverlay + Tab traps inside PopupMenu + Escape dismisses.
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": d1 }) })?;
    let ctx_grid_cmds = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 520.0, y: 360.0, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
    })?;
    let (ctx_layers, ctx_text, ctx_chrome, _) = first_frame(&ctx_grid_cmds)?;
    check(ctx_chrome.context_menu_open, "right-click on grid opens context menu")?;
    check(ctx_layers.iter().any(|l| l.z_band == ZLayerBand::TopOverlay), "ContextMenu renders in TopOverlay RGBA band")?;
    check(ctx_text.iter().any(|op| op.z_layer == ZLayer::ContextMenu), "ContextMenu emits ContextMenu text ops")?;
    let ctx_tree = verify_shared_tree_clean(&session, "grid_context_menu")?;
    check(ctx_tree.node("ctx.grid.exclude_reason").is_some(), "grid context menu contains ctx.grid.exclude_reason")?;

    let tab_ctx = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Tab"), modifiers: KeyModifiers::default() } })?;
    let focused_ctx = first_frame(&tab_ctx)?.2.focused_node_id.clone().unwrap_or_default();
    check(focused_ctx.starts_with("ctx."), format!("Tab in ContextMenu focuses ctx.* item, got {focused_ctx}"))?;

    let ctx_click = pointer_click_node(&mut session, "ctx.grid.exclude_reason")?;
    check(first_frame(&ctx_click)?.2.active_dialog.contains("ExcludeWithReason"), "ctx.grid.exclude_reason launches ExcludeWithReason")?;
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;

    // 2. Right-click Navigator entry -> opens "navigator" context menu; click ctx.nav.duplicate.
    let nav_ctx = pointer_right_click_node(&mut session, &format!("nav.sheet.{d1}"))?;
    check(first_frame(&nav_ctx)?.2.context_menu_open, "right-click on navigator entry opens context menu")?;
    let nav_ctx_tree = verify_shared_tree_clean(&session, "navigator_context_menu")?;
    check(nav_ctx_tree.node("ctx.nav.duplicate").is_some(), "navigator context menu includes ctx.nav.duplicate")?;
    let ds_before = session.host.app().document().data_sheets.len();
    let _ = pointer_click_node(&mut session, "ctx.nav.duplicate")?;
    check(session.host.app().document().data_sheets.len() == ds_before + 1, "ctx.nav.duplicate creates duplicate DataSheet")?;

    // 3. Right-click GraphSheet canvas -> opens "graph_object" context menu; outside click dismisses; ctx.graph.export_svg -> ExportFigure SVG export.
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": g1 }) })?;
    let canvas_ctx = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 600.0, y: 360.0, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
    })?;
    check(first_frame(&canvas_ctx)?.2.context_menu_open, "right-click on graph canvas opens context menu")?;
    let outside_dismiss = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 10.0, y: 780.0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?;
    check(!first_frame(&outside_dismiss)?.2.context_menu_open, "outside click dismisses context menu")?;
    let _ = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 600.0, y: 360.0, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
    })?;
    let ctx_export_dlg = pointer_click_node(&mut session, "ctx.graph.export_svg")?;
    check(first_frame(&ctx_export_dlg)?.2.active_dialog.contains("ExportFigure"), "ctx.graph.export_svg opens ExportFigure modal")?;
    let ctx_export = pointer_click_node(&mut session, "modal.ok")?;
    check(
        ctx_export.iter().any(|c| matches!(c, WebBridgeCommand::SaveOrDownloadFile { format: WebExportFormat::Svg, .. })),
        "confirming ExportFigure from context menu triggers SVG export",
    )?;

    // 4. Navigator collapse, family toggle, overflow wheel scroll, inspector mode, zoom controls, grid inline edit.
    let _ = pointer_click_node(&mut session, "navigator.family_toggle")?;
    let _ = pointer_click_node(&mut session, "nav.folder.0")?;
    let _ = pointer_click_node(&mut session, "nav.folder.0")?;
    let _ = pointer_click_node(&mut session, &format!("nav.sheet.{g1}"))?;
    check(
        session.host.app().document().active_sheet_id.as_ref().map(|s| s.as_str()) == Some(g1.as_str()),
        "clicking nav.sheet switches active sheet",
    )?;
    if matches!(session.host.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = pointer_click_node(&mut session, "modal.ok")?;
    }
    check(session.host.app().canvas_dialogs.active_dialog == ActiveDialog::None, "ChangeGraphType modal should close after modal.ok confirmation")?;
    if current_layout(&session).tree.node("canvas.primary.zoom_pct").is_some() {
        let _ = pointer_click_node(&mut session, "canvas.primary.zoom_pct")?;
        check((session.host.app().canvas_dialogs.canvas.zoom - 1.0).abs() > 0.01, "zoom_pct cycles zoom preset")?;
        let _ = pointer_click_node(&mut session, "canvas.primary.zoom_fit")?;
        check((session.host.app().canvas_dialogs.canvas.zoom - 1.0).abs() < 0.01, "zoom_fit resets zoom to 1.0")?;
    }
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": d1 }) })?;
    if current_layout(&session).tree.node("grid.cell.0.0.0").is_some() {
        let [cx, cy] = node_center_logical(&session, "grid.cell.0.0.0")?;
        let _ = session.handle_event(WebBridgeEvent::UiInput {
            event: UiInputEvent::DoubleClick { x: cx, y: cy, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
        })?;
        check(session.host.app().inline_editor.active, "double-clicking grid.cell.0.0.0 starts inline edit")?;
        for key in ["Backspace", "Backspace", "4", "2", ".", "5"] {
            let _ = session.handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from(key), modifiers: KeyModifiers::default() },
            })?;
        }
        let _ = session.handle_event(WebBridgeEvent::UiInput {
            event: UiInputEvent::KeyDown { key: String::from("Enter"), modifiers: KeyModifiers::default() },
        })?;
        let cell_text = current_layout(&session)
            .tree
            .node("grid.cell.0.0.0")
            .and_then(|n| n.a11y.value_text.clone())
            .ok_or_else(|| TestMsg(String::from("updated grid.cell.0.0.0 value_text missing")))?;
        check(cell_text == "42.5", format!("expected committed cell value \"42.5\" in LayoutTree, got {cell_text:?}"))?;
    }
    let _ =
        session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": rs_id }) })?;
    if current_layout(&session).tree.node("inspector.mode.1").is_some() {
        let _ = pointer_click_node(&mut session, "inspector.mode.1")?;
    }

    // Create enough extra sheets to overflow navigator and verify `scroll.navigator` handles Wheel events.
    for i in 0..20 {
        let _ = session.handle_event(WebBridgeEvent::ExecuteCommand {
            command_id: String::from("sheet.createData"),
            params: json!({ "sheet_id": format!("extra-{i}"), "kind": "column", "title": format!("Extra {i}") }),
        })?;
    }
    let nav_pos = node_center_logical(&session, "navigator.tree")?;
    let nav_wheel = session.handle_event(WebBridgeEvent::Wheel {
        x: nav_pos[0],
        y: nav_pos[1],
        delta_x: 0.0,
        delta_y: 80.0,
        modifiers: KeyModifiers::default(),
    })?;
    let wheel_act = first_frame(&nav_wheel)?.3.map(str::to_owned);
    check(
        wheel_act.as_deref() == Some("scroll.wheel:scroll.navigator"),
        format!(
            "Wheel over navigator should scroll scroll.navigator, got {wheel_act:?}, active_dialog={:?}, context_menu={:?}, nav_pos={nav_pos:?}",
            session.host.app().canvas_dialogs.active_dialog,
            session.chrome.context_menu
        ),
    )?;

    // 5. RTL Arabic locale at 1.0x passes full LayoutTree invariants, then HiDPI 2.0x verifies 2048x1280 scaling & hit-test.
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("ui.locale.set"), params: json!({ "locale": "ar" }) })?;
    let rtl_cmds = pointer_click_node(&mut session, "menu.file")?;
    let (_, rtl_ops, rtl_chrome, _) = first_frame(&rtl_cmds)?;
    check(rtl_chrome.open_menu_key.as_deref() == Some("menu.file"), "RTL click opens menu.file")?;
    let rtl_tree = verify_shared_tree_clean(&session, "rtl_1x_open_menu")?;
    check(rtl_tree.is_rtl, "Arabic locale marks LayoutTree is_rtl")?;
    check(
        rtl_ops.iter().any(|op| op.direction == WebTextDirection::Rtl && op.align == WebTextAlign::Left),
        "RTL text ops pair WebTextDirection::Rtl with WebTextAlign::Left origin anchor",
    )?;
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;

    let _ = session.handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1024.0, logical_height: 640.0, scale_factor: 2.0 })?;
    let hidpi_cmds = pointer_click_node(&mut session, "menu.file")?;
    let Some(WebBridgeCommand::RenderRgbaFrame { width_px, height_px, text_ops, chrome, .. }) = hidpi_cmds.first() else {
        return fail("expected RenderRgbaFrame on 2.0x HiDPI frame");
    };
    check(*width_px == 2048 && *height_px == 1280, "HiDPI 2.0x scales physical buffer to 2048x1280")?;
    check(chrome.open_menu_key.as_deref() == Some("menu.file"), "2.0x scaled logical click hits menu.file")?;
    check(
        text_ops.iter().any(|op| op.direction == WebTextDirection::Rtl && op.align == WebTextAlign::Left),
        "HiDPI RTL text ops keep Rtl + Left origin anchor",
    )?;
    let _ = verify_shared_tree_clean(&session, "rtl_2x_open_menu")?;

    Ok(())
}

#[test]
fn web_chrome_snapshot_projects_aria_tree_and_modal_hidden_parity() -> TestRes {
    let mut session = WebBridgeSession::new();
    let _ = session.handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })?;
    let init_cmds = session.handle_event(WebBridgeEvent::ExecuteCommand {
        command_id: String::from("sheet.createData"),
        params: json!({ "sheet_id": "aria-xy", "kind": "xy", "title": "ARIA XY", "data_sets": 1, "rows": 4 }),
    })?;
    let (_, _, chrome, _) = first_frame(&init_cmds)?;
    let a11y_json = chrome.a11y_tree.as_ref().ok_or_else(|| TestMsg(String::from("WebChromeSnapshot.a11y_tree missing")))?;
    let nodes = a11y_json["nodes"].as_array().ok_or_else(|| TestMsg(String::from("a11y_tree.nodes must be array")))?;
    let analyze_ws =
        nodes.iter().find(|n| n["widget_id"].as_str() == Some("toolbar.analyze")).ok_or_else(|| TestMsg(String::from("missing toolbar.analyze")))?;
    check(analyze_ws["attributes"].get("aria-hidden").is_none(), "workspace toolbar.analyze must not be aria-hidden when modal is closed")?;

    // Open AnalyzeSheet via toolbar click and verify modal aria-modal + background aria-hidden parity.
    let modal_cmds = pointer_click_node(&mut session, "toolbar.analyze")?;
    let (_, _, modal_chrome, _) = first_frame(&modal_cmds)?;
    let modal_a11y = modal_chrome.a11y_tree.as_ref().ok_or_else(|| TestMsg(String::from("modal a11y_tree missing")))?;
    let modal_nodes = modal_a11y["nodes"].as_array().ok_or_else(|| TestMsg(String::from("modal nodes must be array")))?;
    let dlg_node =
        modal_nodes.iter().find(|n| n["widget_id"].as_str() == Some("modal.dialog")).ok_or_else(|| TestMsg(String::from("missing modal.dialog")))?;
    check(dlg_node["attributes"]["aria-modal"].as_str() == Some("true"), "open modal.dialog must project aria-modal=\"true\"")?;
    let analyze_bg = modal_nodes
        .iter()
        .find(|n| n["widget_id"].as_str() == Some("toolbar.analyze"))
        .ok_or_else(|| TestMsg(String::from("missing background toolbar.analyze")))?;
    check(analyze_bg["attributes"]["aria-hidden"].as_str() == Some("true"), "background toolbar.analyze must project aria-hidden=\"true\"")?;

    Ok(())
}

#[test]
fn shared_tree_fontless_rtl_and_language_picker_autonyms_preserve_ascii_and_geometry() -> TestRes {
    let fallback_measurer = DesktopTextMeasurer::with_font_book(FontBook::with_embedded_fallback());
    let (mut session, _d1, _g1, _rs_id) = build_populated_session()?;

    // 1. Arabic RTL workspace + open menu under embedded fallback only misses Arabic glyphs.
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("ui.locale.set"), params: json!({ "locale": "ar" }) })?;
    let _ = pointer_click_node(&mut session, "menu.file")?;
    let palette = FramePalette::from_theme(&session.host.app().theme);
    let rtl_tree = build_workspace_layout_tree(session.host.app(), session.host.viewport(), &fallback_measurer, &palette);
    let rtl_report = verify_tree_invariants(&rtl_tree);
    check(rtl_report.violations.iter().any(|v| v.check == "text_glyphs"), "fontless RTL tree should trigger text_glyphs for Arabic strings")?;
    let rtl_filtered = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &rtl_tree,
        rtl_report.violations.clone(),
        web_parity_common::HostScriptCoverage::none(),
    );
    check(rtl_filtered.is_empty(), format!("expected fontless Arabic RTL tree to filter cleanly, got {rtl_filtered:?}"))?;
    let rtl_backed = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &rtl_tree,
        rtl_report.violations,
        web_parity_common::HostScriptCoverage::all(),
    );
    check(!rtl_backed.is_empty(), "expected Arabic text_glyphs violations to stay when host claims full coverage")?;

    // 2. Open language picker (`popup.menu.ui.locale.set`) in English with CJK/RTL autonyms under embedded fallback.
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("ui.locale.set"), params: json!({ "locale": "en" }) })?;
    let viewport = session.host.viewport();
    let _ = execute_widget_action(
        session.host.app_mut(),
        &WidgetAction::InvokeCommand { command_id: String::from("ui.locale.set"), params: json!({}) },
        viewport,
    )
    .map_err(TestMsg)?;
    let picker_tree = build_workspace_layout_tree(session.host.app(), viewport, &fallback_measurer, &palette);
    check(
        picker_tree.node("popup.menu.ui.locale.set").is_some_and(|n| n.drawn_in_frame),
        "language picker popup must render cleanly in shared LayoutTree",
    )?;
    let picker_report = verify_tree_invariants(&picker_tree);
    check(
        picker_report.violations.iter().any(|v| v.check == "text_glyphs" && v.widget_id.starts_with("popup.menu.ui.locale.set.opt.")),
        "fontless language picker should report missing glyphs for CJK/RTL autonyms",
    )?;
    let picker_filtered = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &picker_tree,
        picker_report.violations,
        web_parity_common::HostScriptCoverage::none(),
    );
    check(picker_filtered.is_empty(), format!("expected fontless language picker to filter only CJK/RTL autonym glyphs, got {picker_filtered:?}"))?;

    // 3. Preserve ASCII `text_glyphs` (both inside real `text_runs` and standalone) and non-`text_glyphs` violations even when CJK/RTL is unbacked.
    let mut ascii_broken_tree = picker_tree.clone();
    let en_run = ascii_broken_tree
        .text_runs
        .iter_mut()
        .find(|r| r.owner_widget_id.as_str() == "popup.menu.ui.locale.set.opt.0")
        .ok_or_else(|| TestMsg(String::from("missing popup.menu.ui.locale.set.opt.0 text run")))?;
    if let Some(first_glyph) = en_run.shaped.glyphs.first_mut() {
        first_glyph.missing_glyph = true;
    }
    en_run.shaped.missing_glyph_count = 1;
    let ascii_broken_filtered = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &ascii_broken_tree,
        verify_tree_invariants(&ascii_broken_tree).violations,
        web_parity_common::HostScriptCoverage::none(),
    );
    check(
        ascii_broken_filtered.iter().any(|v| v.widget_id == "popup.menu.ui.locale.set.opt.0" && v.check == "text_glyphs"),
        format!("expected real ASCII `text_glyphs` failure to remain preserved, got {ascii_broken_filtered:?}"),
    )?;

    let kept = web_parity_common::filter_unbacked_cjk_rtl_text_glyphs_with_coverage(
        &picker_tree,
        vec![
            ParityViolation {
                widget_id: String::from("popup.menu.ui.locale.set.opt.0"),
                check: String::from("text_glyphs"),
                detail: String::from("text run `English (en)` has 1 missing glyph(s)"),
            },
            ParityViolation {
                widget_id: String::from("popup.menu.ui.locale.set"),
                check: String::from("parent_containment"),
                detail: String::from("synthetic bounds violation"),
            },
        ],
        web_parity_common::HostScriptCoverage::none(),
    );
    check(kept.len() == 2, format!("expected ASCII and geometric violations to be preserved, got {kept:?}"))?;
    Ok(())
}
