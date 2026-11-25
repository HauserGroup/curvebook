#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

//! CurveBook desktop application library and native host event-loop integration.

pub mod desktop_host;
#[cfg(not(target_arch = "wasm32"))]
pub mod gui_window;
#[cfg(not(target_arch = "wasm32"))]
pub mod live_control;
#[cfg(not(target_arch = "wasm32"))]
pub mod mac_menu;
