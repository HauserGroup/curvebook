//! Special mathematical functions for biostatistics and nonlinear curve fitting:
//! `ln_gamma`, `gamma`, `ln_beta`, `beta`, `digamma`, `ln_factorial`, `ln_choose`, `erf`, `erfc`,
//! `erfcx`, `erfc_inv`, regularized incomplete gamma (`igamma`/`P`, `igammac`/`Q`, `ln_P`, `ln_Q`,
//! inverses), regularized incomplete beta (`ibeta`/`I_x(a, b)`, two-component `reg_beta_xy`,
//! `ln_reg_beta_xy`, inverses), integer-order Bessel functions (`bessel_j`, `bessel_y`,
//! `bessel_i`, `bessel_k`), and hypergeometric functions (`hyp1f1`, `confluent_u`, `hyp2f1`).

pub mod bessel;
pub mod erf;
pub mod gamma;
pub mod hypergeom;
pub mod inc_beta;
pub mod inc_gamma;

pub use bessel::{bessel_i, bessel_j, bessel_k, bessel_y};
pub use erf::{erf, erfc, erfc_inv, erfcx, norm_isf_raw, norm_ppf_raw};
pub use gamma::{aln1p_minus_arg, beta, deviance_bd0, digamma, gamma, ln_beta, ln_choose, ln_factorial, ln_gamma, stirling_err};
pub use hypergeom::{confluent_u, hyp1f1, hyp2f1};
pub use inc_beta::{ibeta, ln_reg_beta_xy, reg_beta, reg_beta_c_inv, reg_beta_inv, reg_beta_inv_pair, reg_beta_xy};
pub use inc_gamma::{igamma, igammac, ln_reg_gamma_p, ln_reg_gamma_q, reg_gamma_p, reg_gamma_p_inv, reg_gamma_q, reg_gamma_q_inv};
