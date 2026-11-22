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

use std::sync::OnceLock;

use crate::dist::core::{chi2_cdf, chi2_sf, f_cdf, f_sf, mul_div3, norm_cdf, norm_isf, norm_ln_sf, norm_sf, student_t_cdf, student_t_sf};
use crate::error::MathError;
use crate::roots::{RootOptions, brent_root};
use crate::special::erf::{erfc, exp_neg_sq};
use crate::special::gamma::{deviance_bd0, ln_factorial, ln_gamma, stirling_err};
use crate::special::inc_beta::{ln_beta_kernel, ln_reg_beta_xy, reg_beta_xy};
use crate::special::inc_gamma::{ln_reg_gamma_p, ln_reg_gamma_q, reg_gamma_p, reg_gamma_q};

const MAX_POISSON_TERMS: usize = 4096;
const POISSON_SUM_MAX_LAM_HALF: f64 = 1_000_000.0;
const HALF_LN_2PI: f64 = 0.918_938_533_204_672_7;
const GL_ORDER: usize = 24;

static GL_24: OnceLock<([f64; GL_ORDER], [f64; GL_ORDER])> = OnceLock::new();

fn gauss_legendre_24() -> &'static ([f64; GL_ORDER], [f64; GL_ORDER]) {
    GL_24.get_or_init(|| {
        let mut nodes = [0.0_f64; GL_ORDER];
        let mut weights = [0.0_f64; GL_ORDER];
        let n_f = GL_ORDER as f64;
        for i in 0..(GL_ORDER / 2) {
            let mut x = (std::f64::consts::PI * ((i as f64) + 0.75) / (n_f + 0.5)).cos();
            let mut dp = 0.0_f64;
            for _ in 0..16 {
                let mut p0 = 1.0_f64;
                let mut p1 = x;
                for k in 2..=GL_ORDER {
                    let kf = k as f64;
                    let pk = ((2.0 * kf - 1.0) * x * p1 - (kf - 1.0) * p0) / kf;
                    p0 = p1;
                    p1 = pk;
                }
                dp = n_f * (x * p1 - p0) / (x * x - 1.0);
                let dx = p1 / dp;
                x -= dx;
                if dx.abs() <= 4.0 * f64::EPSILON {
                    break;
                }
            }
            let w = 2.0 / ((1.0 - x * x) * dp * dp);
            nodes[i] = -x;
            nodes[GL_ORDER - 1 - i] = x;
            weights[i] = w;
            weights[GL_ORDER - 1 - i] = w;
        }
        (nodes, weights)
    })
}

fn poisson_pmf_at(k: usize, lam_half: f64) -> f64 {
    if k == 0 {
        return (-lam_half).exp();
    }
    let kf = k as f64;
    if k < 12 {
        (-lam_half + kf * lam_half.ln() - ln_factorial(k as u64)).exp()
    } else {
        (-deviance_bd0(kf, lam_half) - HALF_LN_2PI - 0.5 * kf.ln() - stirling_err(kf)).exp()
    }
}

fn poisson_pdf_cont(k: f64, lam_half: f64) -> f64 {
    (-deviance_bd0(k, lam_half) - HALF_LN_2PI - 0.5 * k.ln() - stirling_err(k)).exp()
}

fn temme_pq_large_a(a: f64, x: f64) -> (f64, f64) {
    let lambda = x / a;
    let mu = lambda - 1.0;
    let half_eta2 = deviance_bd0(1.0, lambda);
    let eta = (2.0 * half_eta2).sqrt().copysign(mu);
    let (c0, c1, c2) = if eta.abs() < 0.02 {
        let e = eta;
        let c0 = -1.0 / 3.0 + e * (1.0 / 12.0 + e * (-2.0 / 135.0 + e * (1.0 / 864.0 + e / 2835.0)));
        let c1 = -1.0 / 540.0 + e * (-1.0 / 288.0 + e * (25.0 / 12_096.0));
        let c2 = 25.0 / 6048.0;
        (c0, c1, c2)
    } else {
        let inv_mu = 1.0 / mu;
        let inv_mu3 = inv_mu * inv_mu * inv_mu;
        let inv_mu5 = inv_mu3 * inv_mu * inv_mu;
        let inv_eta = 1.0 / eta;
        let inv_eta3 = inv_eta * inv_eta * inv_eta;
        let inv_eta5 = inv_eta3 * inv_eta * inv_eta;
        let c0 = inv_mu - inv_eta;
        let c1 = inv_eta3 - lambda * inv_mu3 - inv_mu / 12.0;
        let c2 = -3.0 * inv_eta5 + lambda * (2.0 * lambda + 1.0) * inv_mu5 + lambda * inv_mu3 / 12.0 + inv_mu / 288.0;
        (c0, c1, c2)
    };
    let inv_a = 1.0 / a;
    let series = c0 + inv_a * (c1 + inv_a * c2);
    let z = eta * (0.5 * a).sqrt();
    let r_a = exp_neg_sq(z) * series / (std::f64::consts::TAU * a).sqrt();
    let q = (0.5 * erfc(z) + r_a).clamp(0.0, 1.0);
    let p = (0.5 * erfc(-z) - r_a).clamp(0.0, 1.0);
    (p, q)
}

fn reg_gamma_eval_safe(a: f64, z: f64, upper: bool) -> Result<f64, MathError> {
    let res = if upper { reg_gamma_q(a, z) } else { reg_gamma_p(a, z) };
    match res {
        Ok(v) => Ok(v),
        Err(MathError::NotConverged { .. }) if a >= 4096.0 => {
            let (p, q) = temme_pq_large_a(a, z);
            Ok(if upper { q } else { p })
        }
        Err(e) => Err(e),
    }
}

fn reg_beta_eval_safe(x: f64, y: f64, a: f64, b: f64) -> Result<f64, MathError> {
    match reg_beta_xy(x, y, a, b) {
        Ok(v) => Ok(v),
        Err(MathError::NotConverged { .. }) if a >= 1e6 && b >= 1e6 => {
            let half_a = 0.5 * a;
            let half_b = 0.5 * b;
            let half_s = half_a + half_b;
            let pm = half_a / half_s;
            let qm = half_b / half_s;
            let sd = (0.5 * pm * qm / (half_s + 0.5)).sqrt();
            let diff = (half_b * x - half_a * y) / (half_s * (x + y));
            Ok(norm_cdf(diff / sd))
        }
        Err(e) => Err(e),
    }
}

fn ln_reg_beta_eval_safe(x: f64, y: f64, a: f64, b: f64) -> Result<f64, MathError> {
    match ln_reg_beta_xy(x, y, a, b) {
        Ok(v) => Ok(v),
        Err(MathError::NotConverged { .. }) if a >= 1e6 && b >= 1e6 => {
            let half_a = 0.5 * a;
            let half_b = 0.5 * b;
            let half_s = half_a + half_b;
            let pm = half_a / half_s;
            let qm = half_b / half_s;
            let sd = (0.5 * pm * qm / (half_s + 0.5)).sqrt();
            let diff = (half_b * x - half_a * y) / (half_s * (x + y));
            let z = diff / sd;
            if z <= 0.0 {
                Ok(norm_ln_sf(-z))
            } else {
                let sf = norm_sf(z);
                Ok((-sf).ln_1p())
            }
        }
        Err(e) => Err(e),
    }
}

fn ln_poisson_pmf_at(k: usize, lam_half: f64) -> Result<f64, MathError> {
    if lam_half == 0.0 {
        return Ok(if k == 0 { 0.0 } else { f64::NEG_INFINITY });
    }
    if k == 0 {
        return Ok(-lam_half);
    }
    let kf = k as f64;
    if k >= 12 {
        Ok(-deviance_bd0(kf, lam_half) - HALF_LN_2PI - 0.5 * kf.ln() - stirling_err(kf))
    } else {
        Ok(-lam_half + kf * lam_half.ln() - ln_factorial(k as u64))
    }
}

/// Evaluate `sum_{k=0}^inf P(Poisson(lam_half) = k) * T_k` in log-space via unimodal peak search
/// + separate `k = 0` evaluation (handling tiny-shape non-unimodality when `a0 < 1`) and log-sum-exp.
fn sum_poisson_mixture_log_space<F: FnMut(usize) -> Result<f64, MathError>>(
    lam_half: f64,
    k0: usize,
    upper: bool,
    mut eval_ln_tail: F,
) -> Result<f64, MathError> {
    let mut ln_term = |k: usize| -> Result<f64, MathError> {
        let lp = ln_poisson_pmf_at(k, lam_half)?;
        let lt = eval_ln_tail(k)?;
        Ok(lp + lt)
    };

    let l_0 = ln_term(0)?;
    let (mut lo, mut hi) = if !upper {
        (1_usize, k0.max(1))
    } else {
        let mut prev_prev = k0.max(1);
        let mut cur = prev_prev;
        let mut prev_val = ln_term(cur)?;
        let mut step = 1_usize;
        let bracket_hi;
        let limit = k0.saturating_add(2_000_000);
        loop {
            let next = cur.saturating_add(step);
            if next > limit {
                bracket_hi = limit;
                break;
            }
            let next_val = ln_term(next)?;
            if next_val <= prev_val {
                bracket_hi = next;
                break;
            }
            prev_prev = cur;
            cur = next;
            prev_val = next_val;
            step = step.saturating_mul(2);
        }
        (prev_prev, bracket_hi)
    };

    if k0 == 0 && !upper {
        let mut sum = 1.0_f64;
        if !l_0.is_finite() || l_0 < -755.0 {
            return Ok(0.0);
        }
        for k in 1..=50_000 {
            let diff = ln_term(k)? - l_0;
            if diff < -42.0 {
                break;
            }
            sum += diff.exp();
        }
        return Ok((l_0 + sum.ln()).exp().clamp(0.0, 1.0));
    }

    while hi - lo > 3 {
        let m1 = lo + (hi - lo) / 3;
        let m2 = hi - (hi - lo) / 3;
        if ln_term(m1)? < ln_term(m2)? {
            lo = m1 + 1;
        } else {
            hi = m2 - 1;
        }
    }

    let mut k_star = lo;
    let mut l_star = ln_term(lo)?;
    for k in (lo + 1)..=hi {
        let val = ln_term(k)?;
        if val > l_star {
            l_star = val;
            k_star = k;
        }
    }

    let l_max = if !upper || k0 == 0 { l_0.max(l_star) } else { l_star };
    if !l_max.is_finite() || l_max < -755.0 {
        return Ok(0.0);
    }

    let mut sum = (l_star - l_max).exp();
    if (!upper || k0 == 0) && l_0 - l_max >= -42.0 {
        sum += (l_0 - l_max).exp();
    }
    for k in (1..k_star).rev() {
        let diff = ln_term(k)? - l_max;
        if diff < -42.0 {
            break;
        }
        sum += diff.exp();
    }
    let up_limit = k_star.saturating_add(50_000);
    for k in (k_star + 1)..=up_limit {
        let diff = ln_term(k)? - l_max;
        if diff < -42.0 {
            break;
        }
        sum += diff.exp();
    }
    Ok((l_max + sum.ln()).exp().clamp(0.0, 1.0))
}

fn poisson_gl_expectation<F: FnMut(f64) -> Result<f64, MathError>>(lam_half: f64, mut f: F) -> Result<f64, MathError> {
    const SUBINTERVALS: [(f64, f64); 4] = [(-11.0, -5.5), (-5.5, 0.0), (0.0, 5.5), (5.5, 11.0)];
    let (nodes, weights) = gauss_legendre_24();
    let sigma = lam_half.sqrt();
    let mut total = 0.0_f64;
    for &(lo, hi) in &SUBINTERVALS {
        let mid = 0.5 * (lo + hi);
        let half = 0.5 * (hi - lo);
        for (&t, &w) in nodes.iter().zip(weights.iter()) {
            let u = mid + half * t;
            let k = lam_half + sigma * u;
            let weight = half * w * sigma * poisson_pdf_cont(k, lam_half);
            if weight > 0.0 {
                total += weight * f(k)?;
            }
        }
    }
    Ok(total.clamp(0.0, 1.0))
}

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

/// Cancellation-free 1D saddlepoint log-sum-exp quadrature for opposite-tail `P(T_{df, delta} > t)`
/// when `t > 0` and `delta < 0` across arbitrary finite `df < 1e9`, avoiding `1 - cdf` cancellation.
fn nct_sf_pos_t_neg_delta_quad(t: f64, df: f64, delta: f64) -> Result<f64, MathError> {
    let ad = -delta;
    if ad >= 45.0 {
        // Since P((Z - ad) / U > t) <= P(Z > ad) < 1e-440 for ad >= 45, exact in f64.
        return Ok(0.0);
    }
    let half_df = 0.5 * df;
    let ln_norm = if half_df >= 10.0 {
        0.5 * (df / std::f64::consts::PI).ln() - stirling_err(half_df)
    } else {
        std::f64::consts::LN_2 + half_df * (half_df.ln() - 1.0) - ln_gamma(half_df)?
    };
    let log_factor = |s: f64| -> f64 {
        let y = 2.0 * s;
        if y.abs() < 1e-4 { -y * y * (0.5 + y * (1.0 / 6.0 + y * (1.0 / 24.0 + y * (1.0 / 120.0 + y / 720.0)))) } else { y - y.exp_m1() }
    };
    let ln_t = t.ln();
    let ln_int = |s: f64| -> f64 {
        let arg = s + ln_t;
        if arg > 700.0 {
            return f64::NEG_INFINITY;
        }
        let z = arg.exp() + ad;
        let lsf = norm_ln_sf(z);
        if !lsf.is_finite() {
            return f64::NEG_INFINITY;
        }
        ln_norm + half_df * log_factor(s) + lsf
    };

    let rt_df = df.sqrt();
    let ad_t = ad * t;
    let u_star = if ad_t.is_finite() {
        let denom = ad_t + ad_t.hypot(2.0 * rt_df * rt_df.hypot(t));
        2.0 * df / denom
    } else {
        (df.ln() - ad.ln() - ln_t).exp()
    };
    let s_star = u_star.max(1e-300).ln();
    let neg_g2 = (2.0 * (df + t * t) * u_star * u_star + ad * t * u_star).max(1e-300);
    let sigma = 1.0 / neg_g2.sqrt();

    // Refine mode s_peak within a narrow neighborhood around saddlepoint u_star using golden section.
    let mut g_lo = s_star - 2.0 * sigma;
    let mut g_hi = s_star + 2.0 * sigma;
    const INV_PHI: f64 = 0.618_033_988_749_894_9;
    let mut x1 = g_hi - INV_PHI * (g_hi - g_lo);
    let mut x2 = g_lo + INV_PHI * (g_hi - g_lo);
    let mut f1 = ln_int(x1);
    let mut f2 = ln_int(x2);
    for _ in 0..16 {
        if f1 < f2 {
            g_lo = x1;
            x1 = x2;
            f1 = f2;
            x2 = g_lo + INV_PHI * (g_hi - g_lo);
            f2 = ln_int(x2);
        } else {
            g_hi = x2;
            x2 = x1;
            f2 = f1;
            x1 = g_hi - INV_PHI * (g_hi - g_lo);
            f1 = ln_int(x1);
        }
    }
    let s_peak = 0.5 * g_lo + 0.5 * g_hi;
    let ln_ref = ln_int(s_peak).max(ln_int(s_star));
    if !ln_ref.is_finite() || ln_ref < -760.0 {
        return Ok(0.0);
    }

    let mut step_l = sigma.max(1.0 / df);
    let mut s_lo = s_peak - step_l;
    for _ in 0..24 {
        if ln_int(s_lo) - ln_ref <= -50.0 {
            break;
        }
        step_l *= 2.0;
        s_lo = s_peak - step_l;
    }
    let mut step_r = sigma;
    let mut s_hi = s_peak + step_r;
    for _ in 0..24 {
        if ln_int(s_hi) - ln_ref <= -50.0 {
            break;
        }
        step_r *= 2.0;
        s_hi = s_peak + step_r;
    }

    let opts = crate::quad::QuadOptions { abs_tol: 1e-13, rel_tol: 1e-12, max_subdivisions: 256 };
    let integrand = |s: f64| -> f64 {
        let diff = ln_int(s) - ln_ref;
        if diff < -55.0 { 0.0 } else { diff.min(50.0).exp() }
    };
    let left = if s_peak > s_lo { crate::quad::integrate_adaptive(integrand, s_lo, s_peak, opts)? } else { 0.0 };
    let right = if s_hi > s_peak { crate::quad::integrate_adaptive(integrand, s_peak, s_hi, opts)? } else { 0.0 };
    let scaled = left + right;
    if scaled <= 0.0 {
        return Ok(0.0);
    }
    Ok((ln_ref + scaled.ln()).exp().clamp(0.0, 1.0))
}

/// Evaluate `P(T <= t)` (`upper = false`) or `P(T > t)` (`upper = true`) for `t > 0`.
fn nct_positive_t_large_delta_quad(t: f64, df: f64, delta: f64, upper: bool) -> Result<f64, MathError> {
    if df > 1e6 {
        // Two-moment Gaussian approximation for U = sqrt(chi2_df / df):
        //   E[U] = 1 - 1/(4*df) + 1/(32*df^2) + O(df^-3)
        //   Var(U) = 1/(2*df) - 1/(8*df^2) + O(df^-3)
        //   skew(U) = 1/sqrt(2*df) + O(df^-3/2).
        // Stirling expansion avoids catastrophic cancellation in ln_gamma differences and 1 - E[U]^2.
        let inv_df = 1.0 / df;
        let inv_df2 = inv_df * inv_df;
        let eu = 1.0 - 0.25 * inv_df + (1.0 / 32.0) * inv_df2;
        let var_u = (0.5 * inv_df - 0.125 * inv_df2).max(0.0);
        let sigma_u = var_u.sqrt();
        let s_t = t * sigma_u;
        let z_eff = if s_t.is_finite() { (t * eu - delta) / 1.0_f64.hypot(s_t) } else { (eu / sigma_u).copysign(t) };
        return Ok(if upper { norm_sf(z_eff) } else { norm_cdf(z_eff) });
    }
    if !upper && delta < -45.0 {
        return Ok(1.0);
    }
    if delta > 1e12 {
        // For delta > 1e12 and df <= 1e6, P(Z/delta + 1 <= (t/delta)*U) matches P(chi2_df >= df*(delta/t)^2)
        // to O(df / delta^2) <= 1e-18 without 53-bit s0 = ln(delta/t) step quantization.
        let ratio = mul_div3(delta, 1.0, t);
        let ratio2 = if ratio.is_finite() { mul_div3(ratio, ratio, 1.0) } else { f64::INFINITY };
        let x = if ratio2.is_finite() { mul_div3(df, ratio2, 1.0) } else { f64::INFINITY };
        if x.is_infinite() {
            return Ok(if upper { 1.0 } else { 0.0 });
        }
        if x == 0.0 {
            if df < 1e-2 {
                let a = 0.5 * df;
                let ln_p = (a * (a.ln() + 2.0 * (delta.ln() - t.ln())) - ln_gamma(a + 1.0)?).min(0.0);
                return Ok(if upper { ln_p.exp().clamp(0.0, 1.0) } else { (-ln_p.exp_m1()).clamp(0.0, 1.0) });
            }
            return Ok(if upper { 0.0 } else { 1.0 });
        }
        return if upper { chi2_cdf(x, df) } else { chi2_sf(x, df) };
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
    let mut lo_b = (4.0 * tau.sqrt()).max(tau + 2.0);
    for _ in 0..32 {
        let mid = 0.5 * lo_a + 0.5 * lo_b;
        let f_lo = if mid < 1e-3 { mid * mid * (0.5 - mid * (1.0 / 6.0 - mid * (1.0 / 24.0 - mid / 120.0))) } else { (-mid).exp_m1() + mid };
        if f_lo >= tau {
            lo_b = mid;
        } else {
            lo_a = mid;
        }
    }
    let s_lo = -0.5 * lo_b;

    let mut hi_a = 0.0_f64;
    let mut hi_b = (2.0 * tau).sqrt().min(tau.ln_1p() + 3.0);
    for _ in 0..32 {
        let mid = 0.5 * hi_a + 0.5 * hi_b;
        let f_hi = if mid < 1e-3 { mid * mid * (0.5 + mid * (1.0 / 6.0 + mid * (1.0 / 24.0 + mid / 120.0))) } else { mid.exp_m1() - mid };
        if f_hi >= tau {
            hi_b = mid;
        } else {
            hi_a = mid;
        }
    }
    let s_hi = 0.5 * hi_b;

    let s0 = if delta > 0.0 { delta.ln() - t.ln() } else { 0.0 };
    let mut pts = Vec::with_capacity(5);
    pts.push(s_lo);
    if delta > 0.0 {
        let w = (14.0 / delta).max(4.0 * f64::EPSILON * (1.0 + s0.abs()));
        for candidate in [s0 - w, s0, s0 + w] {
            if candidate > pts.last().copied().unwrap_or(s_lo) && candidate < s_hi {
                pts.push(candidate);
            }
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
                let z = if delta > 0.0 && (s - s0).abs() < 1e-3 { delta * (s - s0).exp_m1() } else { t * u - delta };
                let prob = if upper { norm_sf(z) } else { norm_cdf(z) };
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
    if upper && delta < 0.0 {
        return nct_sf_pos_t_neg_delta_quad(t, df, delta);
    }
    let lambda_half = 0.5 * delta * delta;
    if lambda_half > 1e5 || df > 1e6 {
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
        let x_chi2 = mul_div3(df1, f, 1.0);
        return if upper { ncchi2_sf(x_chi2, df1, lambda) } else { ncchi2_cdf(x_chi2, df1, lambda) };
    }

    let ratio = mul_div3(df1, f, df2);
    let (x, y) = if ratio <= 1.0 {
        (ratio / (1.0 + ratio), 1.0 / (1.0 + ratio))
    } else {
        let inv = 1.0 / ratio;
        (1.0 / (1.0 + inv), inv / (1.0 + inv))
    };

    let lam_half = 0.5 * lambda;
    let b = 0.5 * df2;
    if lam_half > POISSON_SUM_MAX_LAM_HALF {
        let a_base = 0.5 * df1;
        return poisson_gl_expectation(lam_half, |k| {
            let a = a_base + k;
            if upper { reg_beta_eval_safe(y, x, b, a) } else { reg_beta_eval_safe(x, y, a, b) }
        });
    }

    let k0 = lam_half.floor() as usize;
    let k0_f = k0 as f64;
    let p_k0 = poisson_pmf_at(k0, lam_half);

    let a_base = 0.5 * df1;
    let a0 = a_base + k0_f;
    let i0 = if upper { reg_beta_eval_safe(y, x, b, a0)? } else { reg_beta_eval_safe(x, y, a0, b)? };
    let g0 = (ln_beta_kernel(x, y, a0, b)? - a0.ln()).exp();

    if p_k0 == 0.0 || i0 < 1e-20 || (i0 < 0.5 && g0 == 0.0) {
        return sum_poisson_mixture_log_space(lam_half, k0, upper, |k| {
            let ak = a_base + (k as f64);
            if upper { ln_reg_beta_eval_safe(y, x, b, ak) } else { ln_reg_beta_eval_safe(x, y, ak, b) }
        });
    }

    let max_steps = MAX_POISSON_TERMS.max((12.0 * lam_half.sqrt()).ceil() as usize + 128);
    let mut total = p_k0 * i0;
    let mut pk = p_k0;
    let mut ik = i0;
    let mut gk = g0;
    for step in 1..=max_steps {
        let j = (k0 + step) as f64;
        let a_prev = a_base + j - 1.0;
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
    for k_curr in (1..=k0.min(max_steps)).map(|d| k0 - d + 1) {
        let jf = k_curr as f64;
        let a_prev = a_base + jf - 1.0;
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
    if !df1.is_finite() || df1 <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df1", value: df1 });
    }
    if df2.is_nan() || df2 <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df2", value: df2 });
    }
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
    if lam_half > POISSON_SUM_MAX_LAM_HALF {
        let a_base = 0.5 * df;
        return poisson_gl_expectation(lam_half, |k| reg_gamma_eval_safe(a_base + k, z, upper));
    }

    let k0 = lam_half.floor() as usize;
    let k0_f = k0 as f64;
    let p_k0 = poisson_pmf_at(k0, lam_half);

    let a_base = 0.5 * df;
    let a0 = a_base + k0_f;
    let g_fn0 = reg_gamma_eval_safe(a0, z, upper)?;
    let ln_step0 =
        if a0 >= 12.0 { -deviance_bd0(a0, z) - HALF_LN_2PI - 0.5 * a0.ln() - stirling_err(a0) } else { a0 * z.ln() - z - ln_gamma(a0 + 1.0)? };
    let step0 = ln_step0.exp();

    if p_k0 == 0.0 || g_fn0 < 1e-20 || (g_fn0 < 0.5 && step0 == 0.0) {
        return sum_poisson_mixture_log_space(lam_half, k0, upper, |k| {
            let ak = a_base + (k as f64);
            if upper { ln_reg_gamma_q(ak, z) } else { ln_reg_gamma_p(ak, z) }
        });
    }

    let max_steps = MAX_POISSON_TERMS.max((12.0 * lam_half.sqrt()).ceil() as usize + 128);
    let mut total = p_k0 * g_fn0;
    let mut pk = p_k0;
    let mut gk = g_fn0;
    let mut tk = step0;
    for step in 1..=max_steps {
        let j = (k0 + step) as f64;
        let a_curr = a_base + j;
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
    for k_curr in (1..=k0.min(max_steps)).map(|d| k0 - d + 1) {
        let jf = k_curr as f64;
        let a_curr = a_base + jf;
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
    if !df.is_finite() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
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
    for _ in 0..64 {
        let f_hi = f(hi);
        if f_hi.is_nan() {
            return Err(MathError::DomainError { name: "noncentrality", value: f_hi });
        }
        if f_hi >= 0.0 {
            return brent_root(&mut f, lo, hi, RootOptions { rel_tol: 1e-12, abs_tol: 1e-13, max_iters: 128 });
        }
        lo = hi;
        hi *= 2.5;
        if !hi.is_finite() {
            break;
        }
    }
    Err(MathError::NotConverged { algorithm: "solve_positive_noncentrality", iterations: 64 })
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

    fn exact_nct_quad_reference(t: f64, df: f64, delta: f64, upper: bool) -> f64 {
        let half_df = 0.5 * df;
        let ln_norm = 0.5 * (df / std::f64::consts::PI).ln() - crate::special::gamma::stirling_err(half_df);
        let log_factor = |s: f64| -> f64 {
            let y = 2.0 * s;
            if y.abs() < 1e-4 { -y * y * (0.5 + y * (1.0 / 6.0 + y * (1.0 / 24.0 + y * (1.0 / 120.0 + y / 720.0)))) } else { y - y.exp_m1() }
        };
        let lo_fn = |m: f64| -> f64 { if m < 1e-3 { m * m * (0.5 - m * (1.0 / 6.0 - m * (1.0 / 24.0 - m / 120.0))) } else { (-m).exp_m1() + m } };
        let hi_fn = |m: f64| -> f64 { if m < 1e-3 { m * m * (0.5 + m * (1.0 / 6.0 + m * (1.0 / 24.0 + m / 120.0))) } else { m.exp_m1() - m } };
        let tau = 45.0 / half_df;
        let (mut lo_a, mut lo_b) = (0.0_f64, (4.0 * tau.sqrt()).min(tau + 1.0));
        for _ in 0..48 {
            let mid = 0.5 * (lo_a + lo_b);
            if lo_fn(mid) >= tau {
                lo_b = mid;
            } else {
                lo_a = mid;
            }
        }
        let s_lo = -0.5 * lo_b;
        let (mut hi_a, mut hi_b) = (0.0_f64, (2.0 * tau).sqrt().min(tau.ln_1p() + 3.0));
        for _ in 0..48 {
            let mid = 0.5 * (hi_a + hi_b);
            if hi_fn(mid) >= tau {
                hi_b = mid;
            } else {
                hi_a = mid;
            }
        }
        let s_hi = 0.5 * hi_b;
        let mut pts = vec![s_lo];
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
            if win[1] <= win[0] {
                continue;
            }
            total += crate::quad::integrate_adaptive(
                |s| {
                    let u = s.exp();
                    let dens = (ln_norm + half_df * log_factor(s)).exp();
                    let prob = if upper { super::norm_sf(t * u - delta) } else { super::norm_cdf(t * u - delta) };
                    dens * prob
                },
                win[0],
                win[1],
                opts,
            )
            .unwrap();
        }
        total.clamp(0.0, 1.0)
    }

    #[test]
    fn s4_nct_large_df_gaussian_approx_numerical_bound() {
        let mut max_abs_err = 0.0_f64;
        for &df in &[1.01e6_f64, 1e7, 1e8] {
            // Analytical leading Edgeworth bound: 1 / (12 * sqrt(pi * df))
            let bound = 1.0 / (12.0 * (std::f64::consts::PI * df).sqrt());
            for &delta in &[448.0_f64, 1000.0, 1e4, 1e5, 1e6, 1e8] {
                let sd_approx = (1.0 + delta * delta / (2.0 * df)).sqrt();
                for &z in &[-5.0_f64, -3.0, -1.0, 0.0, 1.0, 3.0, 5.0] {
                    let t = delta + z * sd_approx;
                    if t <= 0.0 {
                        continue;
                    }
                    let approx_cdf = nct_cdf(t, df, delta).unwrap();
                    let approx_sf = nct_sf(t, df, delta).unwrap();
                    let exact_cdf = exact_nct_quad_reference(t, df, delta, false);
                    let exact_sf = exact_nct_quad_reference(t, df, delta, true);
                    let err_cdf = (approx_cdf - exact_cdf).abs();
                    let err_sf = (approx_sf - exact_sf).abs();
                    max_abs_err = max_abs_err.max(err_cdf).max(err_sf);
                    assert!(
                        err_cdf <= bound * 1.05 && err_sf <= bound * 1.05,
                        "df={df:.2e} delta={delta:.2e} z={z:+.1}: err_cdf={err_cdf:.3e}, err_sf={err_sf:.3e}, bound={bound:.3e}"
                    );
                    if z.abs() <= 5.0 {
                        let tail_exact = if z <= 0.0 { exact_cdf } else { exact_sf };
                        let tail_err = if z <= 0.0 { err_cdf } else { err_sf };
                        if tail_exact > 1e-10 {
                            let rel = tail_err / tail_exact;
                            let rel_cap = 0.02 * (1.01e6 / df).sqrt();
                            assert!(rel <= rel_cap, "df={df:.2e} delta={delta:.2e} z={z:+.1}: rel={rel:.3e} > {rel_cap:.3e}");
                        }
                    }
                }
            }
        }
        assert!(max_abs_err < 5.0e-5, "max_abs_err={max_abs_err}");

        // Seam continuity across df = 1e6 (quadrature vs two-moment Gaussian approximation).
        for &(t, delta) in &[(500.0_f64, 500.0), (1_000.0, 1_000.0), (998.0, 1_000.0)] {
            let left = nct_cdf(t, 999_999.0, delta).unwrap();
            let right = nct_cdf(t, 1_000_001.0, delta).unwrap();
            assert!((left - right).abs() < 5.0e-5, "seam jump at df=1e6 ({t}, {delta}): {left} vs {right}");
        }
    }
}
