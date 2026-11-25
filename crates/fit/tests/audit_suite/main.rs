#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
pub(crate) mod common;

mod audit_dsl_fixes;
mod audit_p3_sec07;
mod audit_p3_sec08;
mod audit_r1_s4a_fit;
mod audit_r1_s4b_fit;
mod audit_r2_s03;
mod audit_solver_fixes;
mod audit_wave2_regressions;
mod rv_ledger_fit;
mod s3a_fit_x2_regressions;
