//! Noncentral Student `t`, noncentral Snedecor `F`, and noncentral Chi-Square `chi^2` cumulative
//! and survival evaluators plus exact confidence-interval root-finders (`noncentral_t_ci`,
//! `noncentral_f_ci`, `noncentral_chi2_ci`) for standardized effect sizes (Cohen's `d`, Hedges' `g`,
//! Glass's `Delta`, `eta^2`, `eta_p^2`, Cohen's `f`, `phi`, Cramer's `V`).
//!
//! Uses Benton & Krishnamoorthy's (2003) bidirectional Poisson-weighted incomplete beta/gamma
//! expansions centered at the Poisson mode `j_0 = floor(lambda / 2)` so large noncentrality
//! parameters never underflow `exp(-lambda / 2)`.
//!
//! References:
//! - Lenth, R. V. (1989). Algorithm AS 243: Cumulative distribution function of the non-central `t`
//!   distribution. *Applied Statistics*, 38(1), 185-189.
//! - Benton, D., & Krishnamoorthy, K. (2003). Computing discrete mixtures of continuous
//!   distributions: noncentral chisquare, noncentral `t` and the distribution of the square of the
//!   sample multiple correlation coefficient. *CSDA*, 43(2), 249-267.

use crate::dist::core::{chi2_cdf, chi2_sf, f_cdf, f_sf, norm_cdf, norm_isf, norm_sf, student_t_cdf, student_t_sf};
use crate::error::MathError;
use crate::roots::{RootOptions, brent_root};
use crate::special::gamma::{ln_factorial, ln_gamma};
use crate::special::inc_beta::{ln_beta_kernel, reg_beta_xy};
use crate::special::inc_gamma::{reg_gamma_p, reg_gamma_q};

const MAX_POISSON_TERMS: usize = 4096;

fn alpha_from_confidence(confidence_percent: f64) -> Result<f64, MathError> {
    if !confidence_percent.is_finite() || !(0.01..=99.99).contains(&confidence_percent) {
        return Err(MathError::InvalidParameter { name: "confidence_percent", value: confidence_percent });
    }
    Ok((100.0 - confidence_percent) / 100.0)
}

// ============================================================================
// Noncentral Student's t Distribution
// ============================================================================

/// Lower-tail CDF `P(T_{df, delta} <= t)` of the noncentral Student `t` distribution (`df > 0`,
/// real noncentrality parameter `delta`).
///
/// # Errors
/// Returns an error if `df <= 0` or any argument is `NaN`.
pub fn nct_cdf(t: f64, df: f64, delta: f64) -> Result<f64, MathError> {
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if t.is_nan() || !delta.is_finite() {
        return Err(MathError::DomainError { name: "t_or_delta", value: t + delta });
    }
    if t.is_infinite() {
        return Ok(if t.is_sign_positive() { 1.0 } else { 0.0 });
    }
    if df.is_infinite() || df >= 1e9 {
        return Ok(norm_cdf(t - delta));
    }
    if delta == 0.0 {
        return student_t_cdf(t, df);
    }
    if t == 0.0 {
        return Ok(norm_sf(delta));
    }
    if t < 0.0 {
        return nct_sf(-t, df, -delta);
    }
    nct_positive_t_eval(t, df, delta, false)
}

/// Upper-tail survival probability `P(T_{df, delta} > t)` of the noncentral Student `t` distribution.
///
/// # Errors
/// Returns an error if `df <= 0` or any argument is `NaN`.
pub fn nct_sf(t: f64, df: f64, delta: f64) -> Result<f64, MathError> {
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if t.is_nan() || !delta.is_finite() {
        return Err(MathError::DomainError { name: "t_or_delta", value: t + delta });
    }
    if t.is_infinite() {
        return Ok(if t.is_sign_positive() { 0.0 } else { 1.0 });
    }
    if df.is_infinite() || df >= 1e9 {
        return Ok(norm_sf(t - delta));
    }
    if delta == 0.0 {
        return student_t_sf(t, df);
    }
    if t == 0.0 {
        return Ok(norm_cdf(delta));
    }
    if t < 0.0 {
        return nct_cdf(-t, df, -delta);
    }
    nct_positive_t_eval(t, df, delta, true)
}

/// Evaluate `P(T <= t)` (`upper = false`) or `P(T > t)` (`upper = true`) for `t > 0`.
fn nct_positive_t_large_delta_quad(t: f64, df: f64, delta: f64, upper: bool) -> Result<f64, MathError> {
    if df > 1e6 || delta.abs() > 1e9 {
        // For df > 1e6, U = sqrt(chi2_df / df) is within O(1/df) of Gaussian and Stirling expansion
        // avoids catastrophic cancellation in ln_gamma differences and 1 - E[U]^2.
        let (eu, var_u) = if df > 1e5 {
            let inv_df = 1.0 / df;
            let inv_df2 = inv_df * inv_df;
            let eu = 1.0 - 0.25 * inv_df + (1.0 / 32.0) * inv_df2;
            let var_u = (0.5 * inv_df - 0.125 * inv_df2).max(0.0);
            (eu, var_u)
        } else {
            let half_df = 0.5 * df;
            let eu = half_df.sqrt() * (ln_gamma(half_df + 0.5)? - ln_gamma(half_df)?).exp() / half_df;
            (eu, (1.0 - eu * eu).max(0.0))
        };
        let z_eff = (t * eu - delta) / (1.0 + t * t * var_u).sqrt();
        return Ok(if upper { norm_sf(z_eff) } else { norm_cdf(z_eff) });
    }

    // Exact 1D quadrature in s = 0.5 * ln(chi2_df / df) so U = e^s > 0:
    // P(T <= t) = int Phi(t * e^s - delta) f_S(s) ds, preserving exact chi skewness for moderate df.
    let half_df = 0.5 * df;
    let ln_norm = if half_df >= 10.0 {
        0.5 * (df / std::f64::consts::PI).ln() - crate::special::stirling_err(half_df)
    } else {
        std::f64::consts::LN_2 + half_df * (half_df.ln() - 1.0) - ln_gamma(half_df)?
    };
    let log_factor = |s: f64| -> f64 {
        let y = 2.0 * s;
        if y.abs() < 1e-4 { -y * y * (0.5 + y * (1.0 / 6.0 + y * (1.0 / 24.0 + y * (1.0 / 120.0 + y / 720.0)))) } else { y - y.exp_m1() }
    };

    // Support [s_lo, s_hi] where log_chi_density drops by exp(-45) (~3e-20).
    let tau = 45.0 / half_df;
    let mut lo_a = 0.0_f64;
    let mut lo_b = (4.0 * tau.sqrt()).min(tau + 1.0);
    for _ in 0..32 {
        let mid = 0.5 * (lo_a + lo_b);
        if (-mid).exp() - 1.0 + mid >= tau {
            lo_b = mid;
        } else {
            lo_a = mid;
        }
    }
    let s_lo = -0.5 * lo_b;

    let mut hi_a = 0.0_f64;
    let mut hi_b = (2.0 * tau).sqrt().min(tau.ln_1p() + 3.0);
    for _ in 0..32 {
        let mid = 0.5 * (hi_a + hi_b);
        if mid.exp_m1() - mid >= tau {
            hi_b = mid;
        } else {
            hi_a = mid;
        }
    }
    let s_hi = 0.5 * hi_b;

    let mut pts = Vec::with_capacity(4);
    pts.push(s_lo);
    if delta > 0.0 {
        let s0 = (delta / t).ln();
        let w = (14.0 / delta).max(1e-7);
        let a = s0 - w;
        let b = s0 + w;
        if a > s_lo && a < s_hi {
            pts.push(a);
        }
        if b > s_lo && b < s_hi {
            pts.push(b);
        }
    }
    pts.push(s_hi);

    let opts = crate::quad::QuadOptions { abs_tol: 1e-13, rel_tol: 1e-12, max_subdivisions: 256 };
    let mut total = 0.0_f64;
    for win in pts.windows(2) {
        let a = win[0];
        let b = win[1];
        if b - a <= 0.0 {
            continue;
        }
        let sub = crate::quad::integrate_adaptive(
            |s| {
                let u = s.exp();
                let dens = (ln_norm + half_df * log_factor(s)).exp();
                let prob = if upper { norm_sf(t * u - delta) } else { norm_cdf(t * u - delta) };
                dens * prob
            },
            a,
            b,
            opts,
        )?;
        total += sub;
    }
    Ok(total.clamp(0.0, 1.0))
}

fn nct_positive_t_eval(t: f64, df: f64, delta: f64, upper: bool) -> Result<f64, MathError> {
    let lambda_half = 0.5 * delta * delta;
    if lambda_half > 1e5 {
        return nct_positive_t_large_delta_quad(t, df, delta, upper);
    }

    let r = t / df.sqrt();
    let (y, one_minus_y) = if r <= 1.0 {
        let r2 = r * r;
        (r2 / (1.0 + r2), 1.0 / (1.0 + r2))
    } else {
        let inv_r2 = 1.0 / (r * r);
        (1.0 / (1.0 + inv_r2), inv_r2 / (1.0 + inv_r2))
    };

    let b = 0.5 * df;
    let k0 = (lambda_half.floor() as usize).min(200_000);
    let k0_f = k0 as f64;

    let p_k0 = if lambda_half == 0.0 { 1.0 } else { (-lambda_half + k0_f * lambda_half.ln() - ln_factorial(k0 as u64)).exp() };
    let q_k0 = if delta == 0.0 {
        0.0
    } else {
        let sign = delta.signum();
        sign * (-lambda_half + (k0_f + 0.5) * lambda_half.ln() - ln_gamma(k0_f + 1.5)?).exp()
    };

    // When upper == true and delta >= 0, sum complement I_{1-y}(b, j + 0.5) & I_{1-y}(b, j + 1) directly
    // so every single term is positive with zero `1 - cdf` cancellation!
    if upper && delta >= 0.0 {
        return Ok(nct_bidirectional_upper_sum(k0, lambda_half, p_k0, q_k0, y, one_minus_y, b)?.clamp(0.0, 1.0));
    }

    let series_half = nct_bidirectional_lower_sum(k0, lambda_half, p_k0, q_k0, y, one_minus_y, b)?;
    let cdf_val = (norm_sf(delta) + 0.5 * series_half).clamp(0.0, 1.0);
    Ok(if upper { (1.0 - cdf_val).clamp(0.0, 1.0) } else { cdf_val })
}

fn nct_bidirectional_lower_sum(k0: usize, lambda_half: f64, p_k0: f64, q_k0: f64, y: f64, one_minus_y: f64, b: f64) -> Result<f64, MathError> {
    let a_even0 = (k0 as f64) + 0.5;
    let a_odd0 = (k0 as f64) + 1.0;
    let ix_even0 = reg_beta_xy(y, one_minus_y, a_even0, b)?;
    let ix_odd0 = reg_beta_xy(y, one_minus_y, a_odd0, b)?;
    let term_even0 = (ln_beta_kernel(y, one_minus_y, a_even0, b)? - a_even0.ln()).exp();
    let term_odd0 = (ln_beta_kernel(y, one_minus_y, a_odd0, b)? - a_odd0.ln()).exp();

    let mut total = p_k0 * ix_even0 + q_k0 * ix_odd0;

    // Forward k = k0 + 1, k0 + 2, ...
    let mut pk = p_k0;
    let mut qk = q_k0;
    let mut ix_e = ix_even0;
    let mut ix_o = ix_odd0;
    let mut gx_e = term_even0;
    let mut gx_o = term_odd0;
    for step in 1..=MAX_POISSON_TERMS {
        let j = (k0 + step) as f64;
        pk *= lambda_half / j;
        qk *= lambda_half / (j + 0.5);
        ix_e = (ix_e - gx_e).max(0.0);
        ix_o = (ix_o - gx_o).max(0.0);
        gx_e *= y * (j - 0.5 + b) / (j + 0.5);
        gx_o *= y * (j + b) / (j + 1.0);
        let contrib = pk * ix_e + qk * ix_o;
        total += contrib;
        if (pk.abs() + qk.abs()) <= 0.25 * f64::EPSILON * total.abs().max(1e-300) {
            break;
        }
    }

    // Backward k = k0 - 1, k0 - 2, ..., 0
    let mut pk_b = p_k0;
    let mut qk_b = q_k0;
    let mut ix_eb = ix_even0;
    let mut ix_ob = ix_odd0;
    let mut gx_eb = term_even0;
    let mut gx_ob = term_odd0;
    for k_curr in (1..=k0).rev() {
        let jf = k_curr as f64;
        pk_b *= jf / lambda_half;
        qk_b *= (jf + 0.5) / lambda_half;
        gx_eb *= (jf + 0.5) / (y * (jf - 0.5 + b));
        gx_ob *= (jf + 1.0) / (y * (jf + b));
        ix_eb = (ix_eb + gx_eb).min(1.0);
        ix_ob = (ix_ob + gx_ob).min(1.0);
        let contrib = pk_b * ix_eb + qk_b * ix_ob;
        total += contrib;
        if (pk_b.abs() + qk_b.abs()) <= 0.25 * f64::EPSILON * total.abs().max(1e-300) {
            break;
        }
    }
    Ok(total)
}

fn nct_bidirectional_upper_sum(k0: usize, lambda_half: f64, p_k0: f64, q_k0: f64, y: f64, one_minus_y: f64, b: f64) -> Result<f64, MathError> {
    let a_even0 = (k0 as f64) + 0.5;
    let a_odd0 = (k0 as f64) + 1.0;
    let iy_even0 = reg_beta_xy(one_minus_y, y, b, a_even0)?;
    let iy_odd0 = reg_beta_xy(one_minus_y, y, b, a_odd0)?;
    let term_even0 = (ln_beta_kernel(y, one_minus_y, a_even0, b)? - a_even0.ln()).exp();
    let term_odd0 = (ln_beta_kernel(y, one_minus_y, a_odd0, b)? - a_odd0.ln()).exp();

    let mut total = p_k0 * iy_even0 + q_k0 * iy_odd0;

    let mut pk = p_k0;
    let mut qk = q_k0;
    let mut iy_e = iy_even0;
    let mut iy_o = iy_odd0;
    let mut gx_e = term_even0;
    let mut gx_o = term_odd0;
    for step in 1..=MAX_POISSON_TERMS {
        let j = (k0 + step) as f64;
        pk *= lambda_half / j;
        qk *= lambda_half / (j + 0.5);
        iy_e = (iy_e + gx_e).min(1.0);
        iy_o = (iy_o + gx_o).min(1.0);
        gx_e *= y * (j - 0.5 + b) / (j + 0.5);
        gx_o *= y * (j + b) / (j + 1.0);
        let contrib = pk * iy_e + qk * iy_o;
        total += contrib;
        if (pk + qk) <= 0.25 * f64::EPSILON * total.max(1e-300) {
            break;
        }
    }

    let mut pk_b = p_k0;
    let mut qk_b = q_k0;
    let mut iy_eb = iy_even0;
    let mut iy_ob = iy_odd0;
    let mut gx_eb = term_even0;
    let mut gx_ob = term_odd0;
    for k_curr in (1..=k0).rev() {
        let jf = k_curr as f64;
        pk_b *= jf / lambda_half;
        qk_b *= (jf + 0.5) / lambda_half;
        gx_eb *= (jf + 0.5) / (y * (jf - 0.5 + b));
        gx_ob *= (jf + 1.0) / (y * (jf + b));
        iy_eb = (iy_eb - gx_eb).max(0.0);
        iy_ob = (iy_ob - gx_ob).max(0.0);
        let contrib = pk_b * iy_eb + qk_b * iy_ob;
        total += contrib;
        if (pk_b + qk_b) <= 0.25 * f64::EPSILON * total.max(1e-300) {
            break;
        }
    }
    Ok(0.5 * total)
}

/// Exact two-sided confidence interval `(delta_lower, delta_upper)` for the noncentrality parameter
/// `delta` of `T_{df, delta}` given observed `t_obs`, degrees of freedom `df > 0`, and
/// `confidence_percent` in `[0.01, 99.99]`.
///
/// Used directly to construct exact confidence intervals for Cohen's `d`, Hedges' `g`, and Glass's `Delta`.
///
/// # Errors
/// Returns an error if `df <= 0`, `t_obs` is non-finite, or `confidence_percent` is outside `[0.01, 99.99]`.
pub fn noncentral_t_ci(t_obs: f64, df: f64, confidence_percent: f64) -> Result<(f64, f64), MathError> {
    if !t_obs.is_finite() {
        return Err(MathError::DomainError { name: "t_obs", value: t_obs });
    }
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    let alpha = alpha_from_confidence(confidence_percent)?;
    let half_alpha = 0.5 * alpha;
    let z_crit = norm_isf(half_alpha)?;
    if df.is_infinite() || df >= 1e9 {
        return Ok((t_obs - z_crit, t_obs + z_crit));
    }

    let lo_delta = solve_nct_delta(t_obs, df, half_alpha, true, z_crit)?;
    let hi_delta = solve_nct_delta(t_obs, df, half_alpha, false, z_crit)?;
    Ok((lo_delta, hi_delta))
}

/// Solve `P(T_{df, delta} >= t_obs) = tail` (`lower_bound = true`) or
/// `P(T_{df, delta} <= t_obs) = tail` (`lower_bound = false`) for `delta`.
fn solve_nct_delta(t_obs: f64, df: f64, tail: f64, lower_bound: bool, z_crit: f64) -> Result<f64, MathError> {
    let eval = |delta: f64| -> f64 {
        if lower_bound { nct_sf(t_obs, df, delta).unwrap_or(f64::NAN) - tail } else { tail - nct_cdf(t_obs, df, delta).unwrap_or(f64::NAN) }
    };
    let span = (z_crit * (1.0 + t_obs.abs() / df.sqrt()) + 2.0).max(4.0);
    let center = if lower_bound { t_obs - z_crit } else { t_obs + z_crit };
    let mut a = center - span;
    let mut b = center + span;
    for _ in 0..40 {
        let fa = eval(a);
        let fb = eval(b);
        if fa <= 0.0 && fb >= 0.0 {
            break;
        }
        if fa > 0.0 {
            b = a;
            a -= (b.abs() + span) * 2.0;
        } else {
            a = b;
            b += (a.abs() + span) * 2.0;
        }
    }
    brent_root(eval, a, b, RootOptions { rel_tol: 1e-12, abs_tol: 1e-13, max_iters: 128 })
}

// ============================================================================
// Noncentral Snedecor's F Distribution
// ============================================================================

/// Lower-tail CDF `P(F_{df1, df2}(lambda) <= f)` for `df1 > 0, df2 > 0, lambda >= 0`.
///
/// # Errors
/// Returns an error on invalid degrees of freedom, `lambda < 0`, or `f < 0`.
pub fn ncf_cdf(f: f64, df1: f64, df2: f64, lambda: f64) -> Result<f64, MathError> {
    ncf_eval(f, df1, df2, lambda, false)
}

/// Upper-tail survival probability `P(F_{df1, df2}(lambda) > f)` evaluated directly with strictly
/// positive terms (zero `1.0 - cdf` cancellation).
///
/// # Errors
/// Returns an error on invalid degrees of freedom, `lambda < 0`, or `f < 0`.
pub fn ncf_sf(f: f64, df1: f64, df2: f64, lambda: f64) -> Result<f64, MathError> {
    ncf_eval(f, df1, df2, lambda, true)
}

fn ncf_eval(f: f64, df1: f64, df2: f64, lambda: f64, upper: bool) -> Result<f64, MathError> {
    if !df1.is_finite() || df1 <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df1", value: df1 });
    }
    if df2.is_nan() || df2 <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df2", value: df2 });
    }
    if !lambda.is_finite() || lambda < 0.0 {
        return Err(MathError::InvalidParameter { name: "lambda", value: lambda });
    }
    if f.is_nan() || f < 0.0 {
        return Err(MathError::DomainError { name: "f", value: f });
    }
    if lambda == 0.0 {
        return if upper { f_sf(f, df1, df2) } else { f_cdf(f, df1, df2) };
    }
    if f == 0.0 {
        return Ok(if upper { 1.0 } else { 0.0 });
    }
    if f.is_infinite() {
        return Ok(if upper { 0.0 } else { 1.0 });
    }
    if df2.is_infinite() {
        return if upper { ncchi2_sf(df1 * f, df1, lambda) } else { ncchi2_cdf(df1 * f, df1, lambda) };
    }

    let ratio = (df1 / df2) * f;
    let (x, y) = if ratio <= 1.0 {
        (ratio / (1.0 + ratio), 1.0 / (1.0 + ratio))
    } else {
        let inv = 1.0 / ratio;
        (1.0 / (1.0 + inv), inv / (1.0 + inv))
    };

    let lam_half = 0.5 * lambda;
    let k0 = (lam_half.floor() as usize).min(500_000);
    let k0_f = k0 as f64;
    let p_k0 = (-lam_half + k0_f * lam_half.ln() - ln_factorial(k0 as u64)).exp();

    let a0 = 0.5 * df1 + k0_f;
    let b = 0.5 * df2;
    let i0 = if upper { reg_beta_xy(y, x, b, a0)? } else { reg_beta_xy(x, y, a0, b)? };
    let g0 = (ln_beta_kernel(x, y, a0, b)? - a0.ln()).exp();

    let mut total = p_k0 * i0;
    let mut pk = p_k0;
    let mut ik = i0;
    let mut gk = g0;
    for step in 1..=MAX_POISSON_TERMS {
        let j = (k0 + step) as f64;
        let a_prev = 0.5 * df1 + j - 1.0;
        pk *= lam_half / j;
        ik = if upper { (ik + gk).min(1.0) } else { (ik - gk).max(0.0) };
        gk *= x * (a_prev + b) / (a_prev + 1.0);
        total += pk * ik;
        if pk <= 0.25 * f64::EPSILON * total.max(1e-300) {
            break;
        }
    }

    let mut pk_b = p_k0;
    let mut ik_b = i0;
    let mut gk_b = g0;
    for k_curr in (1..=k0).rev() {
        let jf = k_curr as f64;
        let a_prev = 0.5 * df1 + jf - 1.0;
        pk_b *= jf / lam_half;
        gk_b *= (a_prev + 1.0) / (x * (a_prev + b));
        ik_b = if upper { (ik_b - gk_b).max(0.0) } else { (ik_b + gk_b).min(1.0) };
        total += pk_b * ik_b;
        if pk_b <= 0.25 * f64::EPSILON * total.max(1e-300) {
            break;
        }
    }
    Ok(total.clamp(0.0, 1.0))
}

/// Exact two-sided confidence interval `(lambda_lower, lambda_upper) >= 0` for the noncentrality
/// parameter `lambda` of `F_{df1, df2}(lambda)` (used for `eta^2`, `eta_p^2`, and Cohen's `f` CIs).
///
/// # Errors
/// Returns an error if `f_obs < 0`, `df1 <= 0`, `df2 <= 0`, or `confidence_percent` is out of `[0.01, 99.99]`.
pub fn noncentral_f_ci(f_obs: f64, df1: f64, df2: f64, confidence_percent: f64) -> Result<(f64, f64), MathError> {
    if !f_obs.is_finite() || f_obs < 0.0 {
        return Err(MathError::DomainError { name: "f_obs", value: f_obs });
    }
    let alpha = alpha_from_confidence(confidence_percent)?;
    let half_alpha = 0.5 * alpha;
    if f_obs == 0.0 {
        return Ok((0.0, 0.0));
    }

    let sf0 = f_sf(f_obs, df1, df2)?;
    let lo = if sf0 >= half_alpha {
        0.0
    } else {
        solve_positive_noncentrality(|lam| ncf_sf(f_obs, df1, df2, lam).unwrap_or(f64::NAN) - half_alpha, df1 * f_obs)?
    };

    let cdf0 = f_cdf(f_obs, df1, df2)?;
    let hi = if cdf0 <= half_alpha {
        0.0
    } else {
        solve_positive_noncentrality(|lam| half_alpha - ncf_cdf(f_obs, df1, df2, lam).unwrap_or(f64::NAN), df1 * f_obs + df1 + 4.0)?
    };
    Ok((lo, hi.max(lo)))
}

// ============================================================================
// Noncentral Chi-Square Distribution
// ============================================================================

/// Lower-tail CDF `P(chi^2_{df}(lambda) <= x)` for `df > 0, lambda >= 0, x >= 0`.
///
/// # Errors
/// Returns an error if `df <= 0`, `lambda < 0`, or `x < 0`.
pub fn ncchi2_cdf(x: f64, df: f64, lambda: f64) -> Result<f64, MathError> {
    ncchi2_eval(x, df, lambda, false)
}

/// Upper-tail survival probability `P(chi^2_{df}(lambda) > x)` with strictly positive summands.
///
/// # Errors
/// Returns an error if `df <= 0`, `lambda < 0`, or `x < 0`.
pub fn ncchi2_sf(x: f64, df: f64, lambda: f64) -> Result<f64, MathError> {
    ncchi2_eval(x, df, lambda, true)
}

fn ncchi2_eval(x: f64, df: f64, lambda: f64, upper: bool) -> Result<f64, MathError> {
    if !df.is_finite() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if !lambda.is_finite() || lambda < 0.0 {
        return Err(MathError::InvalidParameter { name: "lambda", value: lambda });
    }
    if x.is_nan() || x < 0.0 {
        return Err(MathError::DomainError { name: "x", value: x });
    }
    if lambda == 0.0 {
        return if upper { chi2_sf(x, df) } else { chi2_cdf(x, df) };
    }
    if x == 0.0 {
        return Ok(if upper { 1.0 } else { 0.0 });
    }
    if x.is_infinite() {
        return Ok(if upper { 0.0 } else { 1.0 });
    }

    let z = 0.5 * x;
    let lam_half = 0.5 * lambda;
    let k0 = (lam_half.floor() as usize).min(500_000);
    let k0_f = k0 as f64;
    let p_k0 = (-lam_half + k0_f * lam_half.ln() - ln_factorial(k0 as u64)).exp();

    let a0 = 0.5 * df + k0_f;
    let g_fn0 = if upper { reg_gamma_q(a0, z)? } else { reg_gamma_p(a0, z)? };
    let step0 = (a0 * z.ln() - z - ln_gamma(a0 + 1.0)?).exp();

    let mut total = p_k0 * g_fn0;
    let mut pk = p_k0;
    let mut gk = g_fn0;
    let mut tk = step0;
    for step in 1..=MAX_POISSON_TERMS {
        let j = (k0 + step) as f64;
        let a_curr = 0.5 * df + j;
        pk *= lam_half / j;
        gk = if upper { (gk + tk).min(1.0) } else { (gk - tk).max(0.0) };
        tk *= z / a_curr;
        total += pk * gk;
        if pk <= 0.25 * f64::EPSILON * total.max(1e-300) {
            break;
        }
    }

    let mut pk_b = p_k0;
    let mut gk_b = g_fn0;
    let mut tk_b = step0;
    for k_curr in (1..=k0).rev() {
        let jf = k_curr as f64;
        let a_curr = 0.5 * df + jf;
        pk_b *= jf / lam_half;
        tk_b *= a_curr / z;
        gk_b = if upper { (gk_b - tk_b).max(0.0) } else { (gk_b + tk_b).min(1.0) };
        total += pk_b * gk_b;
        if pk_b <= 0.25 * f64::EPSILON * total.max(1e-300) {
            break;
        }
    }
    Ok(total.clamp(0.0, 1.0))
}

/// Exact two-sided confidence interval `(lambda_lower, lambda_upper) >= 0` for the noncentrality
/// parameter `lambda` of `chi^2_{df}(lambda)` (used for `phi` and Cramer's `V` CIs).
///
/// # Errors
/// Returns an error if `chi2_obs < 0`, `df <= 0`, or `confidence_percent` is outside `[0.01, 99.99]`.
pub fn noncentral_chi2_ci(chi2_obs: f64, df: f64, confidence_percent: f64) -> Result<(f64, f64), MathError> {
    if !chi2_obs.is_finite() || chi2_obs < 0.0 {
        return Err(MathError::DomainError { name: "chi2_obs", value: chi2_obs });
    }
    let alpha = alpha_from_confidence(confidence_percent)?;
    let half_alpha = 0.5 * alpha;
    if chi2_obs == 0.0 {
        return Ok((0.0, 0.0));
    }

    let sf0 = chi2_sf(chi2_obs, df)?;
    let lo = if sf0 >= half_alpha {
        0.0
    } else {
        solve_positive_noncentrality(|lam| ncchi2_sf(chi2_obs, df, lam).unwrap_or(f64::NAN) - half_alpha, chi2_obs)?
    };

    let cdf0 = chi2_cdf(chi2_obs, df)?;
    let hi = if cdf0 <= half_alpha {
        0.0
    } else {
        solve_positive_noncentrality(|lam| half_alpha - ncchi2_cdf(chi2_obs, df, lam).unwrap_or(f64::NAN), chi2_obs + df + 8.0)?
    };
    Ok((lo, hi.max(lo)))
}

fn solve_positive_noncentrality<F: FnMut(f64) -> f64>(mut f: F, scale_hint: f64) -> Result<f64, MathError> {
    let mut lo = 0.0_f64;
    let mut hi = scale_hint.max(1.0);
    for _ in 0..48 {
        let f_hi = f(hi);
        if f_hi >= 0.0 {
            return brent_root(&mut f, lo, hi, RootOptions { rel_tol: 1e-12, abs_tol: 1e-13, max_iters: 128 });
        }
        lo = hi;
        hi *= 2.5;
    }
    Err(MathError::NotConverged { algorithm: "solve_positive_noncentrality", iterations: 48 })
}

#[cfg(test)]
mod tests {
    use super::{ncchi2_cdf, ncchi2_sf, ncf_cdf, ncf_sf, nct_cdf, nct_sf, noncentral_chi2_ci, noncentral_f_ci, noncentral_t_ci};

    #[test]
    fn noncentral_t_identities_and_ci_roundtrip() {
        // Symmetry: P(T_{df, delta} <= t) + P(T_{df, delta} > t) == 1, and T_{df, delta}^2 ~ F_{1, df}(delta^2) when symmetric!
        // Specifically P(|T_{df, delta}| <= t) = nct_cdf(t, df, delta) - nct_cdf(-t, df, delta) == ncf_cdf(t^2, 1, df, delta^2)!
        for &(t, df, delta) in &[(2.5, 10.0, 1.8), (4.0, 25.5, -2.3), (1.2, 5.0, 0.7)] {
            let c_pos = nct_cdf(t, df, delta).unwrap();
            let s_pos = nct_sf(t, df, delta).unwrap();
            assert!(((c_pos + s_pos) - 1.0).abs() < 1e-13);

            let c_neg = nct_cdf(-t, df, delta).unwrap();
            let f_val = ncf_cdf(t * t, 1.0, df, delta * delta).unwrap();
            assert!(((c_pos - c_neg) - f_val).abs() < 1e-12, "diff={} vs f={f_val}", c_pos - c_neg);
        }

        let (dl, du) = noncentral_t_ci(3.2, 18.0, 95.0).unwrap();
        assert!((nct_sf(3.2, 18.0, dl).unwrap() - 0.025).abs() < 1e-11);
        assert!((nct_cdf(3.2, 18.0, du).unwrap() - 0.025).abs() < 1e-11);
    }

    #[test]
    fn noncentral_f_and_chi2_ci_roundtrip() {
        let (fl, fu) = noncentral_f_ci(8.5, 3.0, 24.0, 95.0).unwrap();
        assert!((ncf_sf(8.5, 3.0, 24.0, fl).unwrap() - 0.025).abs() < 1e-11);
        assert!((ncf_cdf(8.5, 3.0, 24.0, fu).unwrap() - 0.025).abs() < 1e-11);
        assert!(((ncf_cdf(4.2, 2.0, 30.0, 5.0).unwrap() + ncf_sf(4.2, 2.0, 30.0, 5.0).unwrap()) - 1.0).abs() < 1e-13);

        let (cl, cu) = noncentral_chi2_ci(14.0, 3.0, 95.0).unwrap();
        assert!((ncchi2_sf(14.0, 3.0, cl).unwrap() - 0.025).abs() < 1e-11);
        assert!((ncchi2_cdf(14.0, 3.0, cu).unwrap() - 0.025).abs() < 1e-11);
        assert!(((ncchi2_cdf(9.0, 4.0, 6.0).unwrap() + ncchi2_sf(9.0, 4.0, 6.0).unwrap()) - 1.0).abs() < 1e-13);
    }
}
