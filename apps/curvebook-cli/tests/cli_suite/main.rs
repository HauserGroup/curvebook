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
mod brand_surface_sweep;
mod cli_v2_and_mcp_smoke;
mod mcp_live_attach_parity;
mod path_policy_and_mcp_workdir;
mod r12_8_pzfx_landing_page;
mod r16_5_cli_and_callers;
mod r17_1_cli_xlsx;
mod r17_4_stdio_workflows;
mod r17_5_cli_warnings;
mod r45_robust_cli;
mod r73_mcp_surface_and_headless;
mod slice0_vertical_gate;
