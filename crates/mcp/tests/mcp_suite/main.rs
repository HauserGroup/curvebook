#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod gui_tools_surface;
mod mcp_e2e;
mod mcp_surface;
mod protocol_props;
mod r45_robust_mcp;
