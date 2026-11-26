//! Consolidated CurveBook desktop host, CLI, recovery, and UI wiring integration test suite.
#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;

mod audit_p8_sec19;
mod audit_r2_desktop_host;
mod audit_w1_sec19_s1;
mod cli_startup_flags;
mod gap_desktop_fixes;
mod r15_1_host_backend;
mod r15_2_live_control_grid;
mod r15_3_menus_chrome;
mod r17_1_dnd_and_paste;
mod r17_1_drop_import_prompt;
mod r17_3_idle_repaint;
mod r73_shared_window;
mod recovery_wiring;
