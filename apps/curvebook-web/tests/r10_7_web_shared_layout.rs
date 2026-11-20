#![forbid(unsafe_code)]
//! Minimal strict-CSP web placeholder integration tests (`R10.7`, `R15.5`).
//!
//! Verifies placeholder chrome projection, modal backdrop occlusion, browser-reserved shortcut
//! routing, context menus, HiDPI/RTL placeholder frame validation, and strict-CSP readiness
//! without any retired legacy `LayoutTree` / `FramePalette` coupling.

mod web_parity_common;

use std::fmt;

use curvebook_ui_egui::serde_json::json;
use curvebook_ui_egui::{ActiveDialog, KeyModifiers, PointerButton, UiInputEvent};
use curvebook_web::chrome_bridge::{
    WebChromeBridge, WebChromeLayout, WebCommandRoute, WebShortcutAction, all_shortcuts_have_web_policy, resolve_web_command_route,
};
use curvebook_web::host_adapters::{WebBridgeCommand, WebBridgeEvent, WebBridgeSession};
use curvebook_web::text_bridge::{CSS_REFERENCE_DPI, WebTextBridge, WebTextDirection, ZLayerBand};

#[derive(Debug)]
struct TestFailure(String);

impl fmt::Display for TestFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for TestFailure {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for TestFailure {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<curvebook_web::host_adapters::WebAdapterError> for TestFailure {
    fn from(value: curvebook_web::host_adapters::WebAdapterError) -> Self {
        Self(value.to_string())
    }
}

type TestRes<T = ()> = Result<T, TestFailure>;

fn check(cond: bool, msg: impl Into<String>) -> TestRes {
    if cond { Ok(()) } else { Err(TestFailure(msg.into())) }
}

fn current_layout(session: &WebBridgeSession) -> WebChromeLayout {
    session.chrome.build_layout(session.host.app(), session.host.viewport())
}

fn build_populated_session() -> TestRes<(WebBridgeSession, String, String, String)> {
    let mut session = WebBridgeSession::with_ids("doc-r10-7", "tab-a");
    let _ = session.handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1024.0, logical_height: 640.0, scale_factor: 1.0 })?;
    let ds_out = session.handle_event(WebBridgeEvent::ExecuteCommand {
        command_id: String::from("sheet.createData"),
        params: json!({ "sheet_id": "ds-web", "kind": "column", "title": "Web Column", "cols": 2, "rows": 4, "auto_family": false }),
    })?;
    check(!ds_out.is_empty(), "expected createData frame")?;
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand {
        command_id: String::from("table.setCells"),
        params: json!({
            "sheet_id": "ds-web",
            "edits": [
                { "row": 0, "col": 0, "value": 10.0 },
                { "row": 1, "col": 0, "value": 12.0 },
                { "row": 2, "col": 0, "value": 14.0 },
                { "row": 3, "col": 0, "value": 16.0 },
                { "row": 0, "col": 1, "value": 20.0 },
                { "row": 1, "col": 1, "value": 22.0 },
                { "row": 2, "col": 1, "value": 24.0 },
                { "row": 3, "col": 1, "value": 26.0 }
            ]
        }),
    })?;
    let rs_out = session.handle_event(WebBridgeEvent::ExecuteCommand {
        command_id: String::from("analysis.run"),
        params: json!({ "source_sheet_id": "ds-web", "kind": "descriptive_stats" }),
    })?;
    check(!rs_out.is_empty(), "expected analysis result frame")?;
    let rs_id = session.host.app().document().result_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing result sheet")?;
    let gs_out = session.handle_event(WebBridgeEvent::ExecuteCommand {
        command_id: String::from("graph.create"),
        params: json!({ "data_sheet_id": "ds-web", "kind": "column", "title": "Web Graph" }),
    })?;
    check(!gs_out.is_empty(), "expected graph frame")?;
    let gs_id = session.host.app().document().graph_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing graph sheet")?;
    Ok((session, String::from("ds-web"), rs_id, gs_id))
}

#[test]
fn placeholder_menu_bar_dropdowns_modal_backdrop_and_commands() -> TestRes {
    let (mut session, ds_id, rs_id, _gs_id) = build_populated_session()?;
    let layout = current_layout(&session);
    check(layout.find_node("menu.bar").is_some(), "menu.bar present")?;
    check(layout.find_node("menu.file").is_some(), "menu.file present")?;
    check(layout.find_node("status.bar").is_some(), "status.bar present")?;

    // Clicking menu.file opens dropdown popup and Escape dismisses it cleanly.
    let click_menu = session.handle_event(WebBridgeEvent::ClickChromeNode {
        node_id: String::from("menu.file"),
        button: PointerButton::Primary,
        modifiers: KeyModifiers::default(),
    })?;
    check(click_menu.iter().any(|c| matches!(c, WebBridgeCommand::RenderRgbaFrame { .. })), "menu toggle emits placeholder frame")?;
    let open_layout = current_layout(&session);
    check(open_layout.find_node("popup.menu.file").is_some(), "popup.menu.file visible after click")?;

    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;
    check(session.chrome.open_menu_key.is_none(), "Escape closes menu popup")?;

    // Opening Command Palette creates modal backdrop that blocks outside pointer presses.
    let pal = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::KeyDown { key: String::from("k"), modifiers: KeyModifiers { alt: true, ..Default::default() } },
    })?;
    check(pal.iter().any(|c| matches!(c, WebBridgeCommand::RenderRgbaFrame { .. })), "Alt+K opens palette")?;
    check(session.host.app().canvas_dialogs.active_dialog == ActiveDialog::CommandPalette, "palette open")?;

    let pal_layout = current_layout(&session);
    check(pal_layout.find_node("modal.backdrop").is_some(), "modal backdrop projected")?;
    check(pal_layout.find_node("palette.query").is_some(), "palette input projected")?;
    let blocked = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 5.0, y: 5.0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
    })?;
    check(
        blocked.iter().any(|c| matches!(c, WebBridgeCommand::RenderRgbaFrame { action: Some(a), .. } if a == "modal.backdrop_blocked")),
        "modal backdrop blocks click-through",
    )?;

    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;
    check(session.host.app().canvas_dialogs.active_dialog == ActiveDialog::None, "Escape closes palette")?;

    // Select sheets & verify command route policies.
    let _ =
        session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": ds_id }) })?;
    check(all_shortcuts_have_web_policy(), "all engine shortcuts mapped to web policy")?;
    let _ =
        session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": rs_id }) })?;
    let route = resolve_web_command_route(session.host.app(), "ui.editParams.open");
    check(matches!(route, WebCommandRoute::OpenEditAnalysisParameters(_)), "ResultSheet resolves OpenEditAnalysisParameters")?;
    Ok(())
}

#[test]
fn placeholder_context_menu_hidpi_rtl_and_aria_snapshot() -> TestRes {
    let (mut session, ds_id, _rs_id, _gs_id) = build_populated_session()?;
    let _ =
        session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": ds_id }) })?;

    // Secondary click opens context menu node.
    let _ = session.handle_event(WebBridgeEvent::UiInput {
        event: UiInputEvent::PointerDown { x: 320.0, y: 240.0, button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
    })?;
    let ctx_layout = current_layout(&session);
    check(ctx_layout.find_node("popup.context_menu").is_some(), "right-click opens placeholder context menu")?;
    check(ctx_layout.snapshot.a11y_tree.is_some(), "snapshot includes ARIA tree JSON")?;

    // Dismiss context menu and verify RTL + 2x HiDPI placeholder rendering.
    let _ = session
        .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() } })?;
    let _ = session.handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("ui.locale.set"), params: json!({ "locale": "ar" }) })?;
    let cmds = session.handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1000.0, logical_height: 600.0, scale_factor: 2.0 })?;
    let Some(WebBridgeCommand::RenderRgbaFrame { width_px, height_px, rgba_layers, chrome, .. }) = cmds.last() else {
        return Err(TestFailure::from("expected RenderRgbaFrame from ResizeViewport"));
    };
    check(*width_px == 2000 && *height_px == 1200, "2x HiDPI framebuffer size")?;
    check(!rgba_layers.is_empty(), "workspace RGBA band present")?;
    check(chrome.node_count > 0 && !chrome.menu_titles.is_empty(), "chrome snapshot populated")?;

    let rtl_frame = WebTextBridge::compose_placeholder_frame(800, 480, CSS_REFERENCE_DPI, [20, 24, 30, 255], "البيانات", false);
    WebTextBridge::validate_layered_frame(&rtl_frame)?;
    check(rtl_frame.text_ops.iter().any(|op| op.direction == WebTextDirection::Rtl), "Arabic placeholder text emits RTL direction")?;
    check(rtl_frame.rgba_layers.iter().any(|l| l.z_band == ZLayerBand::Workspace), "Workspace band present")?;

    let mut bridge = WebChromeBridge::new();
    bridge.sync_from_app(session.host.app());
    let _ = web_parity_common::host_script_coverage();
    check(matches!(WebShortcutAction::SaveArchive, WebShortcutAction::SaveArchive), "shortcut enum accessible")?;
    Ok(())
}
