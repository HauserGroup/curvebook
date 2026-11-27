#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "../common/mod.rs"]
mod common;

mod adv_layout;
mod audit_p4_sec09;
mod audit_r1_s3b_xy;
mod audit_r1_s4a_chart_col;
mod audit_r1_s4a_chart_xy;
mod audit_r1_s4b_chart_scales;
mod audit_r2_s08_s09;
mod chart_v9_provenance_and_no_n;
mod column_summary_modes;
mod gap_chart_core_fixes;
mod gap_chart_sci_fixes;
mod point_brushing_preflight;
mod r1019_format_determinism;
mod r1019_typeset_equations;
mod r13_phase_a_charts;
mod r18_6_simple_logistic_overlay;
mod r20_5_fit_summary_autoranging;
mod scales_axes;
mod sec09_s1_regressions;
mod ticks_and_diag_parity;
mod wave22c_wf37_39_40_42;
mod xy_col;
