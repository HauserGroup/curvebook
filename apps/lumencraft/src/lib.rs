#![forbid(unsafe_code)]

//! LumenCraft desktop application library and native host event-loop integration.

pub mod desktop_host;

/// Enforce or relax strict `--gui-only` automation policy on a desktop [`UiApp`](lumencraft_ui_egui::UiApp).
pub fn configure_gui_only(app: &mut lumencraft_ui_egui::UiApp, gui_only: bool) {
    app.set_gui_only(gui_only);
}
