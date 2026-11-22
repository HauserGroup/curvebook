//! Exact multivariate Student `t` integrals and critical values for Dunnett's many-to-one
//! comparisons (two-sided and one-sided, equal or unequal group sizes, real `df > 0` or `+inf`).
//!
//! Conditions on the shared control-group normal variate `Z_0` and pooled scale `U = sqrt(chi^2_{df} / df)`
//! so treatment statistics become conditionally independent, achieving exact *p*-values and critical
//! values without table-interpolation caps (`statistics-implementation.md` §1.1).
//!
//! References:
//! - Dunnett, C. W. (1955). A multiple comparison procedure for comparing several treatments with a
//!   control. *Journal of the American Statistical Association*, 50(272), 1096-1121.
//! - Dunnett, C. W. (1964). New tables for multiple comparisons with a control.
//!   *Biometrics*, 20(3), 482-491.

use crate::dist::core::{norm_pdf, norm_sf, student_t_isf, student_t_sf, student_t_two_sided_isf, student_t_two_sided_p};
use crate::dist::studentized_range::integrate_over_chi_scale_tail;
use crate::error::MathError;
use crate::quad::gauss_legendre_32;
use crate::roots::{RootOptions, brent_root};

const Z0_LIMIT: f64 = 9.5;
const HUGE_DF_CUTOFF: f64 = 1e8;
const MAX_BRACKET_STEPS: usize = 64;

/// Convert control sample size `n_control > 0` and treatment sample sizes `n_treatments` (`n_i > 0`)
/// into Dunnett 1-factor correlation loadings `lambda_i = sqrt(n_i / (n_control + n_i))`.
///
/// # Errors
/// Returns [`MathError::EmptyInput`] if `n_treatments` is empty or [`MathError::InvalidParameter`]
/// if any group size is `<= 0` or non-finite.
pub fn dunnett_loadings_from_sizes(n_control: f64, n_treatments: &[f64]) -> Result<Vec<f64>, MathError> {
    let scales = dunnett_scales_from_sizes(n_control, n_treatments)?;
    Ok(scales.into_iter().map(|(lam, _)| lam).collect())
}

fn dunnett_scales_from_sizes(n_control: f64, n_treatments: &[f64]) -> Result<Vec<(f64, f64)>, MathError> {
    if n_treatments.is_empty() {
        return Err(MathError::EmptyInput);
    }
    if !n_control.is_finite() || n_control <= 0.0 {
        return Err(MathError::InvalidParameter { name: "n_control", value: n_control });
    }
    let mut scales = Vec::with_capacity(n_treatments.len());
    for &ni in n_treatments {
        if !ni.is_finite() || ni <= 0.0 {
            return Err(MathError::InvalidParameter { name: "n_treatment", value: ni });
        }
        let r = ni.sqrt() / n_control.sqrt();
        let inv_compl = 1.0_f64.hypot(r);
        let lam = r / inv_compl;
        scales.push((lam, inv_compl));
    }
    Ok(scales)
}

fn validate_loadings_and_df(loadings: &[f64], df: f64) -> Result<(), MathError> {
    if loadings.is_empty() {
        return Err(MathError::EmptyInput);
    }
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    for &lam in loadings {
        if !lam.is_finite() || !(0.0..=1.0).contains(&lam) {
            return Err(MathError::InvalidParameter { name: "lambda", value: lam });
        }
    }
    Ok(())
}

fn integrate_z0_with_scales<F: FnMut(f64) -> f64>(mut integrand: F, w: f64, scales: &[(f64, f64)], two_sided: bool) -> f64 {
    if !scales.iter().any(|&(_, inv_compl)| inv_compl > 25.0) {
        let i1 = gauss_legendre_32(&mut integrand, -Z0_LIMIT, -2.5);
        let i2 = gauss_legendre_32(&mut integrand, -2.5, 0.0);
        let i3 = gauss_legendre_32(&mut integrand, 0.0, 2.5);
        let i4 = gauss_legendre_32(&mut integrand, 2.5, Z0_LIMIT);
        return (i1 + i2 + i3 + i4).clamp(0.0, 1.0);
    }
    let mut pts = Vec::with_capacity(5 + 6 * scales.len());
    pts.extend_from_slice(&[-Z0_LIMIT, -2.5, 0.0, 2.5, Z0_LIMIT]);
    for &(lam, inv_compl) in scales {
        if inv_compl <= 25.0 || lam <= 0.0 {
            continue;
        }
        let z_step = w / lam;
        let delta = if inv_compl.is_finite() { (10.0 / inv_compl).min(1.0) } else { 0.0 };
        let push_around = |pts: &mut Vec<f64>, center: f64| {
            if center.abs() < Z0_LIMIT + delta {
                pts.push(center.clamp(-Z0_LIMIT, Z0_LIMIT));
                if delta > 0.0 {
                    pts.push((center - delta).clamp(-Z0_LIMIT, Z0_LIMIT));
                    pts.push((center + delta).clamp(-Z0_LIMIT, Z0_LIMIT));
                }
            }
        };
        push_around(&mut pts, -z_step);
        if two_sided {
            push_around(&mut pts, z_step);
        }
    }
    pts.sort_by(f64::total_cmp);
    let mut total = 0.0_f64;
    let mut prev = -Z0_LIMIT;
    for b in pts {
        if b - prev > 1e-14 * (1.0 + prev.abs()) {
            total += gauss_legendre_32(&mut integrand, prev, b);
            prev = b;
        }
    }
    total.clamp(0.0, 1.0)
}

/// Cancellation-free conditional upper-tail probability `1 - prod_{i=1}^m P(|Z_i^*| <= w | Z_0 = z0)`
/// integrated over `z0 ~ N(0, 1)` for a fixed scaled threshold `w = d * u > 0`.
fn dunnett_two_sided_gaussian_sf(w: f64, scales: &[(f64, f64)]) -> f64 {
    if w <= 0.0 {
        return 1.0;
    }
    let integrand = |z0: f64| -> f64 {
        let mut sum_ln_keep = 0.0_f64;
        for &(lam, inv_compl) in scales {
            let shift = lam * z0;
            let tau = if inv_compl.is_finite() {
                (norm_sf((w - shift) * inv_compl) + norm_sf((w + shift) * inv_compl)).min(1.0)
            } else if w < shift.abs() {
                1.0
            } else {
                0.0
            };
            if tau >= 1.0 {
                return norm_pdf(z0);
            }
            sum_ln_keep += (-tau).ln_1p();
        }
        norm_pdf(z0) * (-sum_ln_keep.exp_m1()).max(0.0)
    };
    integrate_z0_with_scales(integrand, w, scales, true)
}

/// Cancellation-free conditional one-sided upper-tail `1 - prod_{i=1}^m P(Z_i^* <= w | Z_0 = z0)`
/// integrated over `z0 ~ N(0, 1)` for `w = d * u`.
fn dunnett_one_sided_gaussian_sf(w: f64, scales: &[(f64, f64)]) -> f64 {
    let integrand = |z0: f64| -> f64 {
        let mut sum_ln_keep = 0.0_f64;
        for &(lam, inv_compl) in scales {
            // T_i = sqrt(1 - lam^2) Z_i - lam Z_0 <= w  <=>  Z_i <= (w + lam z0) / sqrt(1 - lam^2).
            let shift = lam * z0;
            let tau = if inv_compl.is_finite() {
                norm_sf((w + shift) * inv_compl).min(1.0)
            } else if w + shift < 0.0 {
                1.0
            } else {
                0.0
            };
            if tau >= 1.0 {
                return norm_pdf(z0);
            }
            sum_ln_keep += (-tau).ln_1p();
        }
        norm_pdf(z0) * (-sum_ln_keep.exp_m1()).max(0.0)
    };
    integrate_z0_with_scales(integrand, w, scales, false)
}

fn precompute_loading_scales(loadings: &[f64]) -> Vec<(f64, f64)> {
    loadings
        .iter()
        .map(|&lam| {
            let compl = ((1.0 - lam) * (1.0 + lam)).max(0.0).sqrt();
            let inv_compl = if compl > 0.0 { 1.0 / compl } else { f64::INFINITY };
            (lam, inv_compl)
        })
        .collect()
}

fn dunnett_two_sided_p_scales(d: f64, df: f64, scales: &[(f64, f64)]) -> Result<f64, MathError> {
    if d.is_nan() {
        return Err(MathError::DomainError { name: "d", value: d });
    }
    let ad = d.abs();
    if ad == 0.0 {
        return Ok(1.0);
    }
    if ad.is_infinite() {
        return Ok(0.0);
    }
    if scales.len() == 1 {
        return student_t_two_sided_p(ad, df);
    }
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(dunnett_two_sided_gaussian_sf(ad, scales));
    }
    Ok(integrate_over_chi_scale_tail(df, ad, |u| dunnett_two_sided_gaussian_sf(ad * u, scales))?.clamp(0.0, 1.0))
}

fn dunnett_one_sided_p_scales(d: f64, df: f64, scales: &[(f64, f64)]) -> Result<f64, MathError> {
    if d.is_nan() {
        return Err(MathError::DomainError { name: "d", value: d });
    }
    if d.is_infinite() {
        return Ok(if d.is_sign_positive() { 0.0 } else { 1.0 });
    }
    if scales.len() == 1 {
        return student_t_sf(d, df);
    }
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(dunnett_one_sided_gaussian_sf(d, scales));
    }
    Ok(integrate_over_chi_scale_tail(df, d.max(0.0), |u| dunnett_one_sided_gaussian_sf(d * u, scales))?.clamp(0.0, 1.0))
}

/// Exact two-sided Dunnett *p*-value `P(max_{1<=i<=m} |T_i| >= |d|)` from factor loadings
/// `lambda_i in [0, 1]` (`Corr(T_i, T_j | S) = lambda_i * lambda_j`) and real `df > 0` (or `+inf`).
///
/// # Errors
/// Returns an error if `loadings` is empty, any `lambda_i` lies outside `[0, 1]`, `df <= 0`, or `d` is `NaN`.
pub fn dunnett_two_sided_p_loadings(d: f64, df: f64, loadings: &[f64]) -> Result<f64, MathError> {
    validate_loadings_and_df(loadings, df)?;
    let scales = precompute_loading_scales(loadings);
    dunnett_two_sided_p_scales(d, df, &scales)
}

/// Exact two-sided Dunnett *p*-value given control size `n_control` and treatment sizes `n_treatments`.
///
/// # Errors
/// Returns an error if any sample size is `<= 0`, `n_treatments` is empty, `df <= 0`, or `d` is `NaN`.
pub fn dunnett_two_sided_p(d: f64, df: f64, n_control: f64, n_treatments: &[f64]) -> Result<f64, MathError> {
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    let scales = dunnett_scales_from_sizes(n_control, n_treatments)?;
    dunnett_two_sided_p_scales(d, df, &scales)
}

/// Exact one-sided Dunnett *p*-value `P(max_{1<=i<=m} T_i >= d)` (equivalently `P(min T_i <= -d)`)
/// from factor loadings `lambda_i in [0, 1]` and real `df > 0` (or `+inf`).
///
/// # Errors
/// Returns an error if `loadings` is empty, any `lambda_i` lies outside `[0, 1]`, `df <= 0`, or `d` is `NaN`.
pub fn dunnett_one_sided_p_loadings(d: f64, df: f64, loadings: &[f64]) -> Result<f64, MathError> {
    validate_loadings_and_df(loadings, df)?;
    let scales = precompute_loading_scales(loadings);
    dunnett_one_sided_p_scales(d, df, &scales)
}

/// Exact one-sided Dunnett *p*-value given control size `n_control` and treatment sizes `n_treatments`.
///
/// # Errors
/// Returns an error if any sample size is `<= 0`, `n_treatments` is empty, `df <= 0`, or `d` is `NaN`.
pub fn dunnett_one_sided_p(d: f64, df: f64, n_control: f64, n_treatments: &[f64]) -> Result<f64, MathError> {
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    let scales = dunnett_scales_from_sizes(n_control, n_treatments)?;
    dunnett_one_sided_p_scales(d, df, &scales)
}

fn dunnett_two_sided_crit_scales(alpha: f64, df: f64, scales: &[(f64, f64)]) -> Result<f64, MathError> {
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return Err(MathError::DomainError { name: "alpha", value: alpha });
    }
    if alpha == 0.0 {
        return Ok(f64::INFINITY);
    }
    if alpha == 1.0 {
        return Ok(0.0);
    }
    if scales.len() == 1 {
        return student_t_two_sided_isf(alpha, df);
    }

    // Bracketed tightly between single-comparison Student-t and Bonferroni Student-t!
    let m = scales.len() as f64;
    let mut lo = (0.99 * student_t_two_sided_isf(alpha, df)?).max(0.0);
    let mut hi = 1.02 * student_t_two_sided_isf((alpha / m).max(f64::MIN_POSITIVE), df)? + 0.05;

    for _ in 0..MAX_BRACKET_STEPS {
        if dunnett_two_sided_p_scales(lo, df, scales)? >= alpha || lo == 0.0 {
            break;
        }
        lo *= 0.5;
    }
    for _ in 0..MAX_BRACKET_STEPS {
        if dunnett_two_sided_p_scales(hi, df, scales)? <= alpha {
            break;
        }
        hi *= 1.5;
    }
    if dunnett_two_sided_p_scales(lo, df, scales)? < alpha || dunnett_two_sided_p_scales(hi, df, scales)? > alpha {
        return Err(MathError::NotConverged { algorithm: "dunnett_two_sided_crit_loadings", iterations: MAX_BRACKET_STEPS });
    }

    brent_root(
        |d| dunnett_two_sided_p_scales(d, df, scales).unwrap_or(f64::NAN) - alpha,
        lo,
        hi,
        RootOptions { rel_tol: 1e-11, abs_tol: 1e-12, max_iters: 96 },
    )
}

/// Exact two-sided Dunnett critical value `d_{alpha, df} >= 0` solving
/// `P(max_{1<=i<=m} |T_i| >= d) = alpha` for `alpha in [0, 1]` and arbitrary loadings `lambda_i`.
///
/// # Errors
/// Returns an error if `loadings` is invalid, `df <= 0`, or `alpha` lies outside `[0, 1]`.
pub fn dunnett_two_sided_crit_loadings(alpha: f64, df: f64, loadings: &[f64]) -> Result<f64, MathError> {
    validate_loadings_and_df(loadings, df)?;
    let scales = precompute_loading_scales(loadings);
    dunnett_two_sided_crit_scales(alpha, df, &scales)
}

/// Exact two-sided Dunnett critical value given control size `n_control` and treatment sizes `n_treatments`.
///
/// # Errors
/// Returns an error on invalid group sizes, `df <= 0`, or `alpha` outside `[0, 1]`.
pub fn dunnett_two_sided_crit(alpha: f64, df: f64, n_control: f64, n_treatments: &[f64]) -> Result<f64, MathError> {
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    let scales = dunnett_scales_from_sizes(n_control, n_treatments)?;
    dunnett_two_sided_crit_scales(alpha, df, &scales)
}

fn dunnett_one_sided_crit_scales(alpha: f64, df: f64, scales: &[(f64, f64)]) -> Result<f64, MathError> {
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return Err(MathError::DomainError { name: "alpha", value: alpha });
    }
    if alpha == 0.0 {
        return Ok(f64::INFINITY);
    }
    if alpha == 1.0 {
        return Ok(f64::NEG_INFINITY);
    }
    if scales.len() == 1 {
        return student_t_isf(alpha, df);
    }

    let m = scales.len() as f64;
    let t_single = student_t_isf(alpha, df)?;
    let t_bonf = student_t_isf((alpha / m).max(f64::MIN_POSITIVE), df)?;
    let mut lo = t_single.min(t_bonf) - 0.5;
    let mut hi = t_single.max(t_bonf) + 0.5;

    for _ in 0..MAX_BRACKET_STEPS {
        if dunnett_one_sided_p_scales(lo, df, scales)? >= alpha {
            break;
        }
        lo -= lo.abs() + 2.0;
    }
    for _ in 0..MAX_BRACKET_STEPS {
        if dunnett_one_sided_p_scales(hi, df, scales)? <= alpha {
            break;
        }
        hi += hi.abs() + 2.0;
    }
    if dunnett_one_sided_p_scales(lo, df, scales)? < alpha || dunnett_one_sided_p_scales(hi, df, scales)? > alpha {
        return Err(MathError::NotConverged { algorithm: "dunnett_one_sided_crit_loadings", iterations: MAX_BRACKET_STEPS });
    }

    brent_root(
        |d| dunnett_one_sided_p_scales(d, df, scales).unwrap_or(f64::NAN) - alpha,
        lo,
        hi,
        RootOptions { rel_tol: 1e-11, abs_tol: 1e-12, max_iters: 96 },
    )
}

/// Exact one-sided Dunnett critical value `d_{alpha, df}` solving `P(max_{1<=i<=m} T_i >= d) = alpha`.
///
/// # Errors
/// Returns an error if `loadings` is invalid, `df <= 0`, or `alpha` lies outside `[0, 1]`.
pub fn dunnett_one_sided_crit_loadings(alpha: f64, df: f64, loadings: &[f64]) -> Result<f64, MathError> {
    validate_loadings_and_df(loadings, df)?;
    let scales = precompute_loading_scales(loadings);
    dunnett_one_sided_crit_scales(alpha, df, &scales)
}

/// Exact one-sided Dunnett critical value given control size `n_control` and treatment sizes `n_treatments`.
///
/// # Errors
/// Returns an error on invalid group sizes, `df <= 0`, or `alpha` outside `[0, 1]`.
pub fn dunnett_one_sided_crit(alpha: f64, df: f64, n_control: f64, n_treatments: &[f64]) -> Result<f64, MathError> {
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    let scales = dunnett_scales_from_sizes(n_control, n_treatments)?;
    dunnett_one_sided_crit_scales(alpha, df, &scales)
}

#[cfg(test)]
mod tests {
    use super::{
        dunnett_one_sided_crit, dunnett_one_sided_p, dunnett_one_sided_p_loadings, dunnett_two_sided_crit, dunnett_two_sided_p,
        dunnett_two_sided_p_loadings, precompute_loading_scales,
    };
    use crate::dist::core::{norm_two_sided_p, student_t_two_sided_p};
    use crate::dist::studentized_range::integrate_over_chi_scale;

    #[test]
    fn single_treatment_quadrature_is_invariant_to_loading_and_equals_student_t() {
        // Test the full 2D Dunnett quadrature engine (bypassing len == 1 shortcut via internal helper)
        // for arbitrary correlation loadings lambda in {0.2, 0.7071, 0.9}: marginal must equal Student t!
        for &lam in &[0.2, std::f64::consts::FRAC_1_SQRT_2, 0.9] {
            let scales = precompute_loading_scales(&[lam]);
            let p_quad = integrate_over_chi_scale(15.0, |u| super::dunnett_two_sided_gaussian_sf(2.75 * u, &scales)).unwrap();
            let p_exact = student_t_two_sided_p(2.75, 15.0).unwrap();
            assert!((p_quad - p_exact).abs() <= 1e-11 * p_exact, "lam={lam}: {p_quad} vs {p_exact}");
        }
        // Zero loadings at df = inf must equal exact Sidak 1 - (1 - p)^m!
        let p_z = norm_two_sided_p(2.4);
        let sidak_exact = 1.0 - (1.0 - p_z).powi(4);
        let p_dunnett0 = dunnett_two_sided_p_loadings(2.4, f64::INFINITY, &[0.0, 0.0, 0.0, 0.0]).unwrap();
        assert!((p_dunnett0 - sidak_exact).abs() < 1e-13);
    }

    #[test]
    fn dunnett_1964_equal_and_unequal_group_critical_values() {
        // Dunnett (1964) Biometrics 20:482 / Bechhofer-Tamhane certified equal-n (rho = 0.5) tables:
        // Two-sided alpha = 0.05, m = 3 treatments vs control, df = 20 -> d = 2.540... (specifically 2.54037)
        let d_two = dunnett_two_sided_crit(0.05, 20.0, 6.0, &[6.0, 6.0, 6.0]).unwrap();
        assert!((d_two - 2.540_37).abs() < 5e-5, "d_two = {d_two}");
        assert!((dunnett_two_sided_p(d_two, 20.0, 6.0, &[6.0, 6.0, 6.0]).unwrap() - 0.05).abs() < 1e-10);

        // One-sided alpha = 0.05, m = 3 treatments vs control, df = 20 -> Dunnett (1964) table 2.19 (exact 2.19228346...):
        let d_one = dunnett_one_sided_crit(0.05, 20.0, 6.0, &[6.0, 6.0, 6.0]).unwrap();
        assert!((d_one - 2.192_283_46).abs() < 1e-6, "d_one = {d_one}");
        assert!((dunnett_one_sided_p(d_one, 20.0, 6.0, &[6.0, 6.0, 6.0]).unwrap() - 0.05).abs() < 1e-10);

        // Unequal group sizes (larger control reduces correlation & critical value slightly vs smaller control):
        let d_unequal = dunnett_two_sided_crit(0.01, 28.0, 12.0, &[5.0, 8.0, 15.0, 4.0]).unwrap();
        let p_back = dunnett_two_sided_p(d_unequal, 28.0, 12.0, &[5.0, 8.0, 15.0, 4.0]).unwrap();
        assert!((p_back - 0.01).abs() < 1e-10);
        assert!(dunnett_one_sided_p_loadings(0.0, 10.0, &[0.5, 0.5]).unwrap() > 0.5);
    }
}
