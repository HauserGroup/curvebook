//! Consolidated R15–R18 egui shell, grid, modal, and ingestion integration test suite.
#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;

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
mod r18_5_linear_deming;
