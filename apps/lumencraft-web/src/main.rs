//! Browser WebAssembly entry point for LumenCraft.
#![forbid(unsafe_code)]

use lumencraft_ui_egui::UiApp;
use lumencraft_ui_egui::theme::install_fonts;

/// Construct and initialize the browser WASM shell without any host OS, filesystem, or socket calls.
#[must_use]
pub fn init_web_app() -> UiApp {
    let _fonts_loaded = install_fonts();
    let _ = lumencraft_ui_egui::bundled_fonts_enabled();
    let _ = lumencraft_format::EXTENSION;
    let mut app = UiApp::new();
    app.sync_views();
    app
}

fn main() {
    let _app = init_web_app();
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
    }
}
