//! Special mathematical functions for biostatistics and nonlinear curve fitting:
//! `ln_gamma`, `gamma`, `ln_beta`, `beta`, `digamma`, `ln_factorial`, `ln_choose`, `erf`, `erfc`,
//! `erfcx`, `erfc_inv`, regularized incomplete gamma (`P`, `Q`, `ln_P`, `ln_Q`, inverses), and
//! regularized incomplete beta (`I_x(a, b)`, two-component `reg_beta_xy`, `ln_reg_beta_xy`, inverses).

pub mod erf;
pub mod gamma;
pub mod inc_beta;
pub mod inc_gamma;

pub use erf::{erf, erfc, erfc_inv, erfcx, norm_isf_raw, norm_ppf_raw};
pub use gamma::{aln1p_minus_arg, beta, deviance_bd0, digamma, gamma, ln_beta, ln_choose, ln_factorial, ln_gamma, stirling_err};
pub use inc_beta::{ln_reg_beta_xy, reg_beta, reg_beta_c_inv, reg_beta_inv, reg_beta_inv_pair, reg_beta_xy};
pub use inc_gamma::{ln_reg_gamma_p, ln_reg_gamma_q, reg_gamma_p, reg_gamma_p_inv, reg_gamma_q, reg_gamma_q_inv};
