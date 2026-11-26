#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod audit_p4_sec09;
mod audit_r1_s3b_xy;
mod audit_r1_s4a_chart_col;
mod audit_r1_s4a_chart_xy;
mod audit_r1_s4b_chart_scales;
mod audit_r2_s08_s09;
mod rem_aud13_chart_seam;
mod sec09_s1_regressions;
