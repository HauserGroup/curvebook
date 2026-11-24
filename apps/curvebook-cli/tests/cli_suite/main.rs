#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod audit_p8_cli;
mod audit_r1_s3b;
mod audit_r1_s4a_x1_03;
mod audit_r1_s4b_cli_info;
mod audit_r1_s4c_s12_10;
mod audit_r2_s12;
mod cli_v2_and_mcp_smoke;
mod mcp_gui_only_parity;
mod path_policy_and_mcp_workdir;
mod r16_5_cli_and_callers;
mod r45_robust_cli;
mod r73_mcp_gui_only_wf_01_15;
mod r73_mcp_gui_only_wf_16_28;
mod r73_mcp_gui_only_wf_29_42;
mod r73_mcp_gui_only_wf_43_50;
mod r73_mcp_gui_only_wf_51_65;
mod r73_mcp_surface_and_headless;
mod slice0_vertical_gate;
