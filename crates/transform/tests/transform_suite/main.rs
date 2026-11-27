#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod audit_r1_s4a;
mod audit_r1_s4b_transform;
mod audit_r1_s4b_transform_s3;
mod audit_r2_s04;
mod audit_w2_sec03_fixes;
mod audit_w2_transform_ops;
mod d1_curve_transforms;
mod d1_normalization;
mod d1_r18_7_spline_lowess;
mod d1_randomization;
mod prop_transform;
mod qol_reshape_convert;
mod r17_7_auc;
mod r17_7_plate_origins;
mod r63_window_and_conditional;
mod rv_ledger_formula;
mod rv_ledger_ops;
mod summary_mode_matrix_oracles;
mod transform_v9_no_n_modes;
mod unit_transform;
