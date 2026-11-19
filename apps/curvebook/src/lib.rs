#![forbid(unsafe_code)]

//! CurveBook desktop application library and native host event-loop integration.

pub mod desktop_host;
#[cfg(not(target_arch = "wasm32"))]
pub mod gui_window;
#[cfg(not(target_arch = "wasm32"))]
pub mod live_control;
#[cfg(not(target_arch = "wasm32"))]
pub mod mac_menu;

/// Enforce or relax strict `--gui-only` automation policy on a desktop [`UiApp`](curvebook_ui_egui::UiApp).
pub fn configure_gui_only(app: &mut curvebook_ui_egui::UiApp, gui_only: bool) {
    app.set_gui_only(gui_only);
}
