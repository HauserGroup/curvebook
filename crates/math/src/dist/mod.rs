//! Tail-safe univariate and multivariate probability distributions:
//! - Standard normal (`norm_*`),
//! - Student's `t` (`student_t_*`, real fractional df),
//! - Snedecor's `F` (`f_*`, real df1, df2),
//! - Chi-square `chi^2` (`chi2_*`, real df),
//! - Noncentral `t`, `F`, and `chi^2` + exact effect-size CI solvers (`noncentral_t_ci`,
//!   `noncentral_f_ci`, `noncentral_chi2_ci`),
//! - Studentized range `q(k, df)` (`studentized_range_*`, `2 <= k <= 200`),
//! - Exact multivariate Student `t` for Dunnett's test with equal or unequal group sizes (`dunnett_*`).

pub mod core;
pub mod dunnett;
pub mod noncentral;
pub mod studentized_range;

pub use core::{
    chi2_cdf, chi2_isf, chi2_ln_pdf, chi2_ln_sf, chi2_pdf, chi2_ppf, chi2_sf, f_cdf, f_isf, f_ln_pdf, f_ln_sf, f_pdf, f_ppf, f_sf, norm_cdf,
    norm_isf, norm_ln_pdf, norm_ln_sf, norm_ln_two_sided_p, norm_pdf, norm_ppf, norm_sf, norm_two_sided_isf, norm_two_sided_p, student_t_cdf,
    student_t_isf, student_t_ln_pdf, student_t_ln_sf, student_t_ln_two_sided_p, student_t_pdf, student_t_ppf, student_t_sf, student_t_two_sided_isf,
    student_t_two_sided_p,
};
pub use dunnett::{
    dunnett_loadings_from_sizes, dunnett_one_sided_crit, dunnett_one_sided_crit_loadings, dunnett_one_sided_p, dunnett_one_sided_p_loadings,
    dunnett_two_sided_crit, dunnett_two_sided_crit_loadings, dunnett_two_sided_p, dunnett_two_sided_p_loadings,
};
pub use noncentral::{ncchi2_cdf, ncchi2_sf, ncf_cdf, ncf_sf, nct_cdf, nct_sf, noncentral_chi2_ci, noncentral_f_ci, noncentral_t_ci};
pub use studentized_range::{studentized_range_cdf, studentized_range_isf, studentized_range_ln_sf, studentized_range_ppf, studentized_range_sf};
