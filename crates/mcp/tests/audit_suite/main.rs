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
