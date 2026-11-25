#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod audit_anova_multiplicity_fixes;
mod audit_cat_roc_pow_fixes;
mod audit_contracts_and_guards;
mod audit_reg_surv_mv_fixes;
mod audit_twogroup_col_fixes;
