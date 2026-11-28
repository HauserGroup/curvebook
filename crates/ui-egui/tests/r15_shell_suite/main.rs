//! Consolidated R15–R18 egui shell, grid, modal, and ingestion integration test suite.
#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;
#[path = "../human_wf_common/mod.rs"]
mod human_wf_common;

mod ancova_multivariate_ui;
mod audit_da_06_hosts;
mod audit_p6_atomic_undo;
mod audit_p6_sec13;
mod audit_p6_sec14;
mod audit_p6_sec15;
mod audit_p7_sec16;
mod audit_p7_sec17;
mod audit_p7_sec18;
mod audit_p8_web_vws;
mod audit_r1_s4b_ui;
mod audit_r1_s4c_s10_09;
mod audit_r2_s10;
mod audit_w1_sec13_01;
mod audit_w1_sec18_01_export;
mod audit_w1_sec18_modals;
mod cjk_rtl_hidpi_desktop_qa;
mod control_auth_and_panic;
mod descriptive_custom_percentile_ui;
mod dialogs_canvas;
mod feedback_github_issue;
mod frequency_histogram_ui;
mod gap_desktop_ui_fixes;
mod helpers;
mod i18n_theme;
mod interaction_hidpi_qol;
mod popup_matrix;
mod r15_1_driver_and_i18n;
mod r15_1_host_inspect_raster;
mod r15_2_anova_coexist;
mod r15_2_column_grid;
mod r15_3_stage_canvas;
mod r15_3_studio_footer;
mod r15_3_top_chrome;
mod r15_3_welcome_nav;
mod r15_4_graph_and_app_modals;
mod r15_4_grids_and_ingestion;
mod r15_4_r10_23_egui_gaps;
mod r15_4_stats_fit_power;
mod r15_6_phase1_pipeline;
mod r15_6_wave_b_grid_vars;
mod r15_6_wave_b_info_graph;
mod r15_6_wave_b_sci_modals;
mod r15_6_wave_b_shell_import_cleanup;
mod r15_7_gui_addendum;
mod r15_8_curated_menus;
mod r15_9_visual_design_tokens;
mod r16_19_growable_dropdown;
mod r17_1_xlsx_and_paste;
mod r17_3_kittest_idle;
mod r17_7_plate_auc_gui;
mod r18_5_linear_deming;
mod r18_7_spline_lowess;
mod r19_2_levene_ui;
mod rv_ledger_third_pass_ui;
mod ui_parity_ux;
mod zz_overlap_probe;
