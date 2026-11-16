//! Browser WebAssembly entry point for CurveBook.
#![forbid(unsafe_code)]

use curvebook_ui_egui::UiApp;
use curvebook_web::{WebAdapterError, WebBridgeCommand, WebBridgeEvent, WebBridgeSession, WebNoticeLevel, WebWorkspaceHost};

/// Construct and initialize the browser WASM host without any host OS, filesystem, or socket calls.
#[must_use]
pub fn init_web_host() -> WebWorkspaceHost {
    WebWorkspaceHost::new()
}

/// Construct and initialize the browser WASM [`UiApp`] without any host OS, filesystem, or socket calls.
#[must_use]
pub fn init_web_app() -> UiApp {
    init_web_host().app().clone()
}

/// Construct a [`WebBridgeSession`] with zero-telemetry local storage, locks, FS access, and touch adapters.
#[must_use]
pub fn init_web_bridge_session(doc_id: &str, tab_id: &str) -> WebBridgeSession {
    WebBridgeSession::with_ids(doc_id, tab_id)
}

/// Initialize the default browser bridge session and verify its initial lock + framebuffer bootstrap commands.
///
/// # Errors
/// Returns [`WebAdapterError`] if bootstrap lock acquisition or framebuffer rasterization fails.
pub fn bootstrap_web_session() -> Result<(WebBridgeSession, Vec<WebBridgeCommand>), WebAdapterError> {
    let mut session = WebBridgeSession::new();
    let mut commands =
        session.handle_event(WebBridgeEvent::AcquireLock { doc_id: String::from("default"), tab_id: String::from("tab-1"), now_ms: 0 })?;
    let mut frame_cmds = session.handle_event(WebBridgeEvent::ResizeViewport {
        logical_width: curvebook_web::DEFAULT_VIEWPORT_WIDTH,
        logical_height: curvebook_web::DEFAULT_VIEWPORT_HEIGHT,
        scale_factor: 1.0,
    })?;
    commands.append(&mut frame_cmds);
    Ok((session, commands))
}

/// Dispatch a raw JSON bridge event and emit a structured `NotifyStatus` command when handling fails (`R2-S11-05`).
#[must_use]
pub fn dispatch_bridge_event_json_or_notice(session: &mut WebBridgeSession, raw_json: &str) -> String {
    match session.handle_event_json(raw_json) {
        Ok(reply) => reply,
        Err(err) => {
            let notice = WebBridgeCommand::NotifyStatus { level: WebNoticeLevel::Error, code: String::from(err.code()), message: err.to_string() };
            WebBridgeSession::commands_to_json_string(&[notice]).unwrap_or_else(|_| String::from("[]"))
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn connect_wasm_bridge() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use js_sys::{Function, Reflect};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};

    let Some(win) = web_sys::window() else {
        return;
    };
    let Ok(host) = Reflect::get(&win, &JsValue::from_str("CurveBookHost")) else {
        return;
    };
    let Ok(connect_val) = Reflect::get(&host, &JsValue::from_str("connectBridge")) else {
        return;
    };
    let Ok(connect_fn) = connect_val.dyn_into::<Function>() else {
        return;
    };

    let session = Rc::new(RefCell::new(WebBridgeSession::new()));
    let callback_session = Rc::clone(&session);
    let handler = Closure::<dyn FnMut(String) -> String>::new(move |raw_json: String| {
        let Ok(mut guard) = callback_session.try_borrow_mut() else {
            return String::from("[]");
        };
        dispatch_bridge_event_json_or_notice(&mut guard, &raw_json)
    });

    let _ = connect_fn.call1(&host, handler.as_ref());
    handler.forget();
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    connect_wasm_bridge();
    #[cfg(not(target_arch = "wasm32"))]
    let _bootstrap = bootstrap_web_session();
}

#[cfg(test)]
mod tests {
    use super::{bootstrap_web_session, dispatch_bridge_event_json_or_notice, init_web_app};
    use curvebook_web::{TabAccessMode, WebBridgeCommand, WebBridgeSession, WebNoticeLevel};

    #[test]
    fn web_app_initializes_and_dispatches_control_commands_without_os_calls() {
        let mut app = init_web_app();
        let reply = app.dispatch_control_line("{\"id\":\"w1\",\"method\":\"ui.theme.set\",\"params\":{\"mode\":\"dark\"}}");
        assert!(reply.contains("\"ok\":true"));
        assert!(reply.contains("\"dark\""));
        assert_eq!(app.locale, "en");

        assert!(matches!(app.handle_key_combo("F6"), Ok(Some(_))));
        assert!(matches!(app.handle_key_combo("Shift+F6"), Ok(Some(_))));

        let rulers = app.dispatch_control_line("{\"id\":\"w2\",\"method\":\"ui.rulers.toggle\",\"params\":{\"unit\":\"mm\"}}");
        assert!(rulers.contains("\"ok\":true"));
        assert!(rulers.contains("\"rulers_visible\":true"));

        let ar_locale = app.dispatch_control_line("{\"id\":\"w3\",\"method\":\"ui.locale.set\",\"params\":{\"locale\":\"ar\"}}");
        assert!(ar_locale.contains("\"ok\":true"));
        assert_eq!(app.locale, "ar");
    }

    #[test]
    fn web_bridge_session_bootstraps_lock_and_initial_frame() {
        let (mut session, commands) = bootstrap_web_session().expect("bootstrap succeeds on native host");
        assert_eq!(session.access_mode(), &TabAccessMode::ExclusiveReadWrite);
        assert!(commands.iter().any(|cmd| matches!(cmd, WebBridgeCommand::SetReadOnlyBanner { read_only: false, holder_tab_id: None, .. })));
        assert!(commands.iter().any(|cmd| matches!(
            cmd,
            WebBridgeCommand::RenderRgbaFrame { width_px, height_px, rgba_bytes, rgba_layers, text_ops, chrome, .. }
                if *width_px > 0
                    && *height_px > 0
                    && !rgba_bytes.is_empty()
                    && !rgba_layers.is_empty()
                    && !text_ops.is_empty()
                    && chrome.menu_bar_visible
        )));

        let err_json = dispatch_bridge_event_json_or_notice(&mut session, "{invalid-json");
        let parsed = WebBridgeSession::commands_from_json_str(&err_json).expect("error command array decodes");
        assert!(matches!(parsed.as_slice(), [WebBridgeCommand::NotifyStatus { level: WebNoticeLevel::Error, .. }]));
    }
}
