#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod audit_anova_multiplicity_fixes;
mod audit_cat_roc_pow_fixes;
mod audit_contracts_and_guards;
mod audit_reg_surv_mv_fixes;
mod audit_twogroup_col_fixes;
mod d1_edge_matrix;
mod d1_extended_stats_oracles;
mod d1_poisson_cox_result_sheets;
mod d1_r18_6_simple_logistic;
mod d1_reference_oracles;
mod engine_smoke;
mod parity_g3_g7;
mod parity_g8_barnard_pb_mi;
mod power_oracle;
mod prop_stats;
mod r017_lilliefors_oracles;
mod r1010_pvalue_stack_oracles;
mod r1011_cld_piepho_oracles;
mod r13_2_survival_group_exclusion;
mod r19_2_levene_edge_cases;
mod r34_unstructured_reml_oracles;
mod r35_anova_reml_oracles;
mod r35_power_multiplicity_oracles;
mod r35_survival_twogroup_oracles;
mod r36_rm_reml_oracles;
mod rem_aud05_anova;
mod rem_aud05_core;
mod rem_aud05_reg_cat_surv;
mod wave21_r97_r912_clustering;
mod wave22c_wf29_wf43;
