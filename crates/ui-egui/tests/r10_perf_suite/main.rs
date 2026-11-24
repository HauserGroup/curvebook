//! Consolidated R10/R13 UI latency, virtualization, and performance integration test suite.
#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;
#[path = "../human_wf_common/mod.rs"]
mod human_wf_common;

mod grid_perf_s1;
mod r10_23_beta_desktop_gaps;
mod r10_23_shortcuts_and_copy;
mod r10_26_workflow_run_self_test;
mod r10_27_gap_sweep_controls;
mod r10_27_layout_theme_paths;
mod r10_28_graph_gaps;
mod r10_28_modal_gaps;
mod r10_core_and_spike;
mod r10_human_wf_scaffold_smoke;
mod r10_phase7_gates;
mod r10_w7_mv_ui;
mod r10_w7_r36_rm_reml;
mod r13_10_format_graph_controls;
mod r13_8_desktop_views_p1;
mod r13_8_desktop_views_p2;
