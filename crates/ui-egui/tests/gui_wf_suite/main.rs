//! Consolidated strict `--gui-only` workflow integration test suite.
#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;
#[path = "../human_wf_common/mod.rs"]
mod human_wf_common;

mod gui_only_workflows;
mod gui_w22b_qol_and_r9;
mod gui_wf_50_cld;
mod gui_wf_51_58;
mod gui_wf_59_65;
mod gui_wf_advanced_statistical_graphs;
mod gui_wf_clustering_glm_and_pharma;
mod gui_wf_foundations_and_data;
mod gui_wf_group_comparisons_and_power;
mod gui_wf_multivariable_and_transforms;
mod gui_wf_regression_contingency_multivariate;
mod gui_wf_statistical_graphs;
