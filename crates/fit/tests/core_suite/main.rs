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
mod d1_custom_equations;
mod d1_edge_matrix;
mod d1_fit_uncertainty;
mod d1_nist_and_pharma_oracles;
mod d1_nist_reparam_builtins;
mod d1_non_nist_fitting;
mod d1_r18_1_solver_weighting_rout;
mod d1_r18_2_identifiability_compare;
mod d1_r18_3_extended_equations;
mod d1_r18_4_dsl_builtins_and_init_rules;
mod d1_r18_5_linear_deming;
mod dsl_fuzz_props;
mod equations_and_dsl_blocks;
mod pharma_binding_and_dsl;
mod prop_fit;
mod r017_cleanroom_oracles;
mod r13_1_d1_oracles;
mod r13_9_dataset_display_labels;
mod r45_robust_suppression;
mod r46_library_oracles;
mod r46_linear_oracles;
mod r46_shapiro_rk45_oracles;
mod r46_solver_oracles;
mod rv_ledger_fit;
mod s3a_fit_x2_regressions;
