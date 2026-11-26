#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod audit_mcp_live_formats;
mod audit_mcp_protocol;
mod audit_p8_sec21;
mod audit_r1_s3b;
mod audit_r1_s4a_mcp_formats;
mod audit_r1_s4a_x1_03;
mod audit_r2_s12;
mod audit_w1_x1_01;
mod gui_tools_surface;
mod issue_59_pzfx_decimal_comma;
mod mcp_e2e;
mod mcp_surface;
mod protocol_props;
mod r17_1_mcp_xlsx;
mod r17_4_mcp_usability;
mod r19_5_mcp_export_warnings;
mod r45_robust_mcp;
mod rem_hosts;
