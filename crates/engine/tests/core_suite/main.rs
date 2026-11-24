#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;

mod budgets_and_guards;
mod dag_reactivity;
mod e2e_workflows;
mod parity_wave3_workflows;
mod path_policy_regression;
mod r45_robust_e2e;
mod recovery_wal_regression;
mod s3a_x2_regressions;
mod services_and_declarative_patches;
mod undo_and_methods_qol;
mod wave22_integration;
