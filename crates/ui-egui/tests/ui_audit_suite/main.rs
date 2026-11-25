//! Consolidated UI security audit, i18n/theme, dialogs, and interaction integration test suite.
#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;
#[path = "../human_wf_common/mod.rs"]
mod human_wf_common;

mod helpers;

mod audit_da_06_hosts;
mod audit_p6_atomic_undo;
mod audit_p6_sec13;
mod audit_p6_sec14;
mod audit_p6_sec15;
mod audit_p7_sec16;
mod audit_p7_sec17;
mod audit_p7_sec18;
mod audit_p8_web_vws;
mod audit_r1_s4b_ui;
mod audit_r1_s4c_s10_09;
mod audit_r2_s10;
mod audit_w1_sec13_01;
mod audit_w1_sec18_01_export;
mod audit_w1_sec18_modals;
mod cjk_rtl_hidpi_desktop_qa;
mod control_auth_and_panic;
mod dialogs_canvas;
mod feedback_github_issue;
mod i18n_theme;
mod interaction_hidpi_qol;
mod rv_ledger_third_pass_ui;
mod ui_parity_ux;
mod zz_overlap_probe;
