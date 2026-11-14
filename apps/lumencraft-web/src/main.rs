//! Browser WebAssembly entry point for LumenCraft.
#![forbid(unsafe_code)]

use lumencraft_ui_egui::UiApp;
use lumencraft_web::WebWorkspaceHost;

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

fn main() {
    let _host = init_web_host();
}

#[cfg(test)]
mod tests {
    use super::init_web_app;

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
}
