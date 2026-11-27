//! Studentized range distribution `q(k, df)` (`ptukey` / `qtukey` family) and maximum of `r`
//! Studentized ranges `q(r, c, df)` for Tukey HSD, Tukey-Kramer, Newman-Keuls, Games-Howell, and
//! simple-effects multiple comparisons.
//!
//! Evaluates multi-panel 32-point Gauss-Legendre quadrature over Hartley's folded Gaussian range
//! integral (for lower-tail CDF) and saddle-point-adapted cancellation-free upper-tail quadrature
//! paired with outer chi-scale integration, achieving >= 11-14 digits for `2 <= k <= 200` and all
//! real `df > 0` (including fractional Welch-Satterthwaite degrees of freedom and `df = +inf`)
//! without `1.0 - cdf` upper-tail cancellation.
//!
//! References:
//! - Copenhaver, M. D., & Holland, B. (1988). Computation of the distribution of the maximum
//!   studentized range statistic with application to multiple significance testing of simple
//!   effects. *Journal of Statistical Computation and Simulation*, 30(1), 1-15.
//!   `doi:10.1080/00949658808811082`.
//! - Hartley, H. O. (1942). The range in random samples. *Biometrika*, 32(3/4), 334-348.
//! - Mason, J. C., & Handscomb, D. C. (2003). *Chebyshev Polynomials*. Chapman & Hall/CRC, §4.6.
//! - Trefethen, L. N. (2013). *Approximation Theory and Approximation Practice*. SIAM, Ch. 4 & 8.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::OnceLock;

use crate::dist::core::{norm_cdf, norm_isf, norm_ln_pdf, norm_ln_sf, norm_pdf, norm_sf, student_t_two_sided_isf, student_t_two_sided_p};
use crate::error::MathError;
use crate::quad::{gauss_legendre_32, gl32_nodes_weights};
use crate::roots::{RootOptions, brent_root};
use crate::special::gamma::{deviance_bd0, ln_gamma, stirling_err};
use crate::sum::NeumaierSum;

const NORMAL_Z_MIN: f64 = -10.0;
const NORMAL_Z_MAX: f64 = 9.5;
const HUGE_DF_CUTOFF: f64 = 1e8;
const MAX_BRACKET_STEPS: usize = 64;
/// Upper cutoff `sqrt(4 * 760)` matching `0.25 * w^2 > 760` in [`gaussian_range_sf`], beyond which
/// `1 - H_c(w)` underflows `f64` to `0.0`.
const GAUSSIAN_RANGE_ZERO_W: f64 = 55.136_195_008_360_89;
/// Cutoff `sqrt(4 * 560)` where [`gaussian_range_ln_sf`] switches from linear `sf.ln()` to
/// log-sum-exp quadrature; placing a Chebyshev panel boundary here prevents straddling the switch.
const GAUSSIAN_RANGE_LN_SWITCH_W: f64 = 47.328_638_264_796_93;
const CHEB_DEGREE: usize = 16;
const CHEB_POINTS: usize = CHEB_DEGREE + 1;
const CHEB_MAX_DEPTH: usize = 6;
const CHEB_MAX_PANELS: usize = 64;
const CHEB_ABS_TOL: f64 = 1e-12;
const CHEB_REL_TOL: f64 = 1e-13;

fn active_z_upper_bound(km1_f: f64) -> f64 {
    let tail_floor = (1e-20_f64).powf(1.0 / km1_f.max(1.0)).min(0.999_999);
    norm_isf(tail_floor).unwrap_or(NORMAL_Z_MAX).clamp(-2.0, NORMAL_Z_MAX)
}

#[inline]
fn pow_usize(base: f64, exp: usize) -> f64 {
    match i32::try_from(exp) {
        Ok(e) => base.powi(e),
        Err(_) => base.powf(exp as f64),
    }
}

/// Integrate `f` over `[lo, hi]` across non-degenerate sub-panels partitioned by `knots`.
fn sum_gl32_panels<F: FnMut(f64) -> f64>(mut f: F, lo: f64, hi: f64, knots: &[f64], min_gap: f64) -> f64 {
    if hi <= lo {
        return 0.0;
    }
    let mut sorted = [0.0_f64; 8];
    let mut count = 0_usize;
    for &x in knots.iter().take(sorted.len()) {
        if x.is_finite()
            && let Some(slot) = sorted.get_mut(count)
        {
            *slot = x;
            count += 1;
        }
    }
    if let Some(active) = sorted.get_mut(..count) {
        active.sort_by(f64::total_cmp);
    }

    let hi_guard = min_gap.max(1e-12 * hi.abs());
    let mut total = 0.0_f64;
    let mut cur = lo;
    for &x in sorted.get(..count).unwrap_or(&[]) {
        if x > cur + min_gap && x + hi_guard < hi {
            total += gauss_legendre_32(&mut f, cur, x);
            cur = x;
        }
    }
    total + gauss_legendre_32(&mut f, cur, hi)
}

fn integrate_z_sf_panels<F: FnMut(f64) -> f64>(f: F, w: f64, k: usize) -> f64 {
    let kf = k as f64;
    let km1_f = (k - 1) as f64;
    let z_star = -0.5 * w;
    let z_min = (z_star - 9.5).min(NORMAL_Z_MIN);
    let z_mode = -norm_isf(1.0 / kf).unwrap_or(0.0).min(5.0);
    let z_max = active_z_upper_bound(km1_f).max(z_mode + 2.0).min(NORMAL_Z_MAX);
    if k > 200 {
        let z_scale = (1.0 / z_mode.abs().max(1.0)).min(1.0);
        let knots = [z_star - 3.0, z_star, z_star + 3.0, z_mode - 3.0 * z_scale, z_mode, z_mode + 3.0 * z_scale];
        sum_gl32_panels(f, z_min, z_max, &knots, 0.25)
    } else {
        let knots = [z_star - 3.0, z_star, z_star + 3.0, z_mode];
        sum_gl32_panels(f, z_min, z_max, &knots, 0.25)
    }
}

fn validate_q_args(q: f64, k: usize, df: f64) -> Result<(), MathError> {
    if k < 2 {
        return Err(MathError::InvalidParameter { name: "k", value: k as f64 });
    }
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if q.is_nan() {
        return Err(MathError::DomainError { name: "q", value: q });
    }
    Ok(())
}

fn validate_max_q_args(q: f64, r: usize, c: usize, df: f64) -> Result<(), MathError> {
    if r < 1 {
        return Err(MathError::InvalidParameter { name: "r", value: r as f64 });
    }
    if c < 2 {
        return Err(MathError::InvalidParameter { name: "c", value: c as f64 });
    }
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if q.is_nan() {
        return Err(MathError::DomainError { name: "q", value: q });
    }
    Ok(())
}

/// Inner Gaussian range CDF `H_k(w) = P(max_{1..=k} Z_i - min_{1..=k} Z_i <= w)` for `w > 0`.
///
/// Uses Hartley's (1942) / Copenhaver & Holland (1988, Eq. 2.1-2.2) folded positive-half-line
/// integral `[Phi(w/2) - Phi(-w/2)]^k + 2 k int_{w/2}^{inf} phi(v) [Phi(v) - Phi(v - w)]^{k-1} dv`
/// for `w < 20`, switching to `1 - gaussian_range_sf(w, k)` in the far upper tail.
fn gaussian_range_cdf(w: f64, k: usize) -> f64 {
    if w <= 0.0 {
        return 0.0;
    }
    if k == 2 {
        return (1.0 - crate::special::erfc(0.5 * w)).clamp(0.0, 1.0);
    }
    if w >= 20.0 {
        return (1.0 - gaussian_range_sf(w, k)).clamp(0.0, 1.0);
    }
    let half_w = 0.5 * w;
    let center_prob = normal_interval_prob(-half_w, half_w);
    let center_term = if center_prob > 0.0 { pow_usize(center_prob, k) } else { 0.0 };

    let kf = k as f64;
    let km1 = k - 1;
    let km1_f = km1 as f64;
    let v_mode = norm_isf(1.0 / kf).unwrap_or(0.0).clamp(0.0, 5.5);
    let v_max = active_z_upper_bound(km1_f).max(v_mode + 2.5).max(half_w + 4.0).min(11.0);
    if half_w >= v_max {
        return center_term.clamp(0.0, 1.0);
    }

    let wing_integrand = |v: f64| -> f64 {
        let diff = normal_interval_prob(v - w, v);
        if diff <= 0.0 { 0.0 } else { 2.0 * kf * norm_pdf(v) * pow_usize(diff, km1) }
    };
    let mid = 0.5 * half_w + 0.5 * v_max;
    let knots = [v_mode, v_mode + 2.0, mid];
    let wings = sum_gl32_panels(wing_integrand, half_w, v_max, &knots, 0.25);
    (center_term + wings).clamp(0.0, 1.0)
}

/// Cancellation-free inner Gaussian range survival `1 - H_k(w) = P(Z_{(k)} - Z_{(1)} > w)`:
/// uses the exact identity `int_{-inf}^{inf} k phi(z) Phi(-z)^{k-1} dz = 1` so the integrand is
/// strictly non-negative everywhere:
/// `k * phi(z) * A^{k-1} * (1 - (1 - B/A)^{k-1})` where `A = Phi(-z)` and `B = Phi(-z - w)`.
fn gaussian_range_sf(w: f64, k: usize) -> f64 {
    if w <= 0.0 {
        return 1.0;
    }
    if k == 2 {
        return crate::special::erfc(0.5 * w);
    }
    if 0.25 * w * w > 760.0 {
        return 0.0;
    }
    let kf = k as f64;
    let km1 = k - 1;
    let km1_f = km1 as f64;

    let integrand = |z: f64| -> f64 {
        let a = norm_sf(z);
        if a <= 0.0 {
            return 0.0;
        }
        let b = norm_sf(z + w);
        if b <= 0.0 {
            return 0.0;
        }
        let bracket_gap = if b <= 0.5 * a {
            let ratio = b / a;
            -((-ratio).ln_1p() * km1_f).exp_m1()
        } else {
            let d = if z >= 0.0 {
                (a - b).max(0.0)
            } else if z + w <= 0.0 {
                (norm_cdf(z + w) - norm_cdf(z)).max(0.0)
            } else {
                (1.0 - (norm_cdf(z) + b)).max(0.0)
            };
            let da = (d / a).clamp(0.0, 1.0);
            1.0 - pow_usize(da, km1)
        };
        kf * norm_pdf(z) * pow_usize(a, km1) * bracket_gap.max(0.0)
    };
    integrate_z_sf_panels(integrand, w, k).clamp(0.0, 1.0)
}

/// Cancellation-free survival `1 - [H_c(w)]^r` of the maximum of `r` independent `c`-group Gaussian ranges.
fn max_gaussian_range_sf(w: f64, r: usize, c: usize) -> f64 {
    if r == 1 {
        return gaussian_range_sf(w, c);
    }
    let s = gaussian_range_sf(w, c);
    if s <= 0.0 {
        0.0
    } else if s >= 1.0 {
        1.0
    } else {
        (-((-s).ln_1p() * (r as f64)).exp_m1()).clamp(0.0, 1.0)
    }
}

/// Compute `Phi(hi) - Phi(lo)` for `hi >= lo` without subtraction cancellation in either tail.
#[inline]
pub(crate) fn normal_interval_prob(lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        return 0.0;
    }
    if lo >= 0.0 {
        (norm_sf(lo) - norm_sf(hi)).max(0.0)
    } else if hi <= 0.0 {
        (norm_cdf(hi) - norm_cdf(lo)).max(0.0)
    } else {
        (1.0 - (norm_cdf(lo) + norm_sf(hi))).max(0.0)
    }
}

#[inline]
fn chi_scale_norm_const(a: f64, lg_half_df: f64) -> f64 {
    if a >= 12.0 { 0.5 * (a / std::f64::consts::TAU).ln() - stirling_err(a) } else { -lg_half_df }
}

/// Chi-scale density `h_{df}(u)` of `U = sqrt(chi^2_{df} / df)` evaluated stably using precomputed
/// `a = df / 2` and `norm_const = chi_scale_norm_const(a, lg_half_df)`.
#[inline]
fn chi_scale_pdf_fast(u: f64, a: f64, norm_const: f64) -> f64 {
    if u <= 0.0 {
        return 0.0;
    }
    let x = a * u * u;
    let ln_kern = if a >= 12.0 { -deviance_bd0(a, x) + norm_const } else { a * x.ln() - x + norm_const };
    (2.0 / u) * ln_kern.exp()
}

/// Integrate `inner(u)` against `U = sqrt(chi^2_{df} / df)` over `(0, inf)`.
pub(crate) fn integrate_over_chi_scale<F: FnMut(f64) -> f64>(df: f64, inner: F) -> Result<f64, MathError> {
    integrate_over_chi_scale_tail(df, 0.0, inner)
}

/// Integrate `inner(u)` against `U = sqrt(chi^2_{df} / df)` over `(0, inf)`, placing extra panel
/// resolution around the joint saddle point `u_* = sqrt((df - 1) / (df + tail_scale^2))` when
/// `inner(u)` decays like `exp(-0.5 * (tail_scale * u)^2)`.
pub(crate) fn integrate_over_chi_scale_tail<F: FnMut(f64) -> f64>(df: f64, tail_scale: f64, mut inner: F) -> Result<f64, MathError> {
    let a = 0.5 * df;
    let lg_a = if a < 12.0 { ln_gamma(a)? } else { 0.0 };
    let scale = if tail_scale.is_finite() { tail_scale.max(0.0) } else { f64::MAX };
    let rt_df = df.sqrt();
    let rt_eff = rt_df.hypot(scale);

    if df < 3.0 {
        // Substitute u = t^{1 / a} (a = df / 2): x = a u^2 = a t^{2 / a},
        // f_X(x) dx = 2 * a^{a - 1} * t * exp(-x - ln Gamma(a)) dt.
        // Near t = 0 the leading non-polynomial term is O(t^{1 + 2/a}) vs O(u^{df - 1}) in u-space,
        // which is strictly smoother for all df < 3 (a < 1.5, eliminating the u^{df-1} loss at df = 2.01).
        let p = 1.0 / a;
        let ratio_rt = rt_df / rt_eff;
        let t_star = ratio_rt.powf(a);
        let t_mid = (12.0 / a).max(2.0).powf(0.5 * a);
        let t_max = (65.0 / a).max(16.0).powf(0.5 * a);
        let mapped = |t: f64| -> f64 {
            if t <= 0.0 {
                return 0.0;
            }
            let u = t.powf(p);
            let x = a * u * u;
            let dens_dt = 2.0 * a.powf(a - 1.0) * t * (-x - lg_a).exp();
            dens_dt * inner(u)
        };
        let min_gap = (0.05 * t_star).clamp(1e-300, 0.02);
        let knots = [0.2 * t_star, t_star, 2.0 * t_star, 5.0 * t_star, 20.0 * t_star, 0.2, 1.0, t_mid];
        return Ok(sum_gl32_panels(mapped, 0.0, t_max, &knots, min_gap));
    }

    let rt_dm1 = (df - 1.0).sqrt();
    let u_mode = rt_dm1 / rt_df;
    let sigma_u = 1.0 / (2.0 * df).sqrt();
    let u_star = rt_dm1 / rt_eff;
    let sigma_star = 1.0 / (std::f64::consts::SQRT_2 * rt_eff);

    let u_min = (u_star - 11.0 * sigma_star).min(u_mode - 11.0 * sigma_u).max(0.0);
    let u_max = (u_mode + 13.5 * sigma_u).max(u_star + 11.0 * sigma_star);
    let min_gap = (0.25 * sigma_star.min(sigma_u)).max(1e-300);

    let norm_const = chi_scale_norm_const(a, lg_a);
    let weighted = |u: f64| -> f64 {
        let dens = chi_scale_pdf_fast(u, a, norm_const);
        if dens <= 1e-300 { 0.0 } else { dens * inner(u) }
    };
    let knots = [
        u_star - 3.5 * sigma_star,
        u_star,
        u_star + 3.5 * sigma_star,
        u_star + 10.0 * sigma_star,
        10.0 * u_star + 30.0 * sigma_star,
        u_mode - 3.5 * sigma_u,
        u_mode,
        u_mode + 4.0 * sigma_u,
    ];
    Ok(sum_gl32_panels(weighted, u_min, u_max, &knots, min_gap))
}

/// Lower-tail CDF `P(Q_{k, df} <= q)` of the Studentized range distribution for `k >= 2` groups
/// and real `df > 0` (or `df = +inf`).
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `q` is `NaN`.
pub fn studentized_range_cdf(q: f64, k: usize, df: f64) -> Result<f64, MathError> {
    validate_q_args(q, k, df)?;
    if q <= 0.0 {
        return Ok(0.0);
    }
    if q.is_infinite() {
        return Ok(1.0);
    }
    if k == 2 {
        return Ok(1.0 - student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df)?);
    }
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(gaussian_range_cdf(q, k));
    }
    Ok(integrate_over_chi_scale(df, |u| gaussian_range_cdf(q * u, k))?.clamp(0.0, 1.0))
}

/// Upper-tail *p*-value `P(Q_{k, df} > q)` of the Studentized range distribution evaluated directly
/// without `1.0 - cdf(q)` cancellation.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `q` is `NaN`.
pub fn studentized_range_sf(q: f64, k: usize, df: f64) -> Result<f64, MathError> {
    validate_q_args(q, k, df)?;
    if q <= 0.0 {
        return Ok(1.0);
    }
    if q.is_infinite() {
        return Ok(0.0);
    }
    if k == 2 {
        return student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df);
    }
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(gaussian_range_sf(q, k));
    }
    Ok(integrate_over_chi_scale_tail(df, q * std::f64::consts::FRAC_1_SQRT_2, |u| gaussian_range_sf(q * u, k))?.clamp(0.0, 1.0))
}

fn ln_one_minus_exp_neg(ln_val: f64) -> f64 {
    if ln_val >= 0.0 {
        f64::NEG_INFINITY
    } else if ln_val > -std::f64::consts::LN_2 {
        (-ln_val.exp_m1()).ln()
    } else {
        (-ln_val.exp()).ln_1p()
    }
}

fn gaussian_range_ln_integrand(z: f64, w: f64, ln_k: f64, km1_f: f64) -> f64 {
    let la = norm_ln_sf(z);
    let lb = norm_ln_sf(z + w);
    let delta = lb - la;
    let ln_gap = if delta < -40.0 {
        km1_f.ln() + delta
    } else if delta <= -std::f64::consts::LN_2 {
        ln_one_minus_exp_neg(km1_f * (-delta.exp()).ln_1p())
    } else {
        let l1 = norm_ln_sf(-(z + w));
        let l2 = norm_ln_sf(-z);
        let ldiff = l1 + ln_one_minus_exp_neg(l2 - l1);
        ln_one_minus_exp_neg(km1_f * (ldiff - la))
    };
    ln_k + norm_ln_pdf(z) + km1_f * la + ln_gap
}

/// Log-space inner Gaussian range survival `ln(1 - H_k(w)) = ln P(Z_{(k)} - Z_{(1)} > w)` for
/// arbitrary `w > 0`, remaining finite and accurate past `f64` probability underflow (`0.25 * w^2 > 760`).
fn gaussian_range_ln_sf(w: f64, k: usize) -> f64 {
    if w <= 0.0 {
        return 0.0;
    }
    if k == 2 {
        return -0.25 * w * w + crate::special::erfcx(0.5 * w).ln();
    }
    if 0.25 * w * w <= 560.0 {
        let sf = gaussian_range_sf(w, k);
        if sf > 1e-250 {
            return sf.ln();
        }
    }
    let kf = k as f64;
    let km1_f = (k - 1) as f64;
    let ln_k = kf.ln();
    let z_star = -0.5 * w;
    let z_mode = -norm_isf(1.0 / kf).unwrap_or(0.0).min(5.0);
    let z_lo = (z_star - 10.0).min(z_mode - 8.0).min(NORMAL_Z_MIN);
    let z_hi = (z_star + 10.0).max(active_z_upper_bound(km1_f).min(NORMAL_Z_MAX));
    let probes = [z_star - 1.5, z_star, z_star + 1.5, z_mode.clamp(z_lo, z_hi)];
    let mut ln_ref = f64::NEG_INFINITY;
    for &zp in &probes {
        ln_ref = ln_ref.max(gaussian_range_ln_integrand(zp, w, ln_k, km1_f));
    }
    if !ln_ref.is_finite() {
        return f64::NEG_INFINITY;
    }
    let knots = [z_star - 3.0, z_star, z_star + 3.0, z_star + 10.0, z_mode];
    let scaled = sum_gl32_panels(|z| (gaussian_range_ln_integrand(z, w, ln_k, km1_f) - ln_ref).exp(), z_lo, z_hi, &knots, 0.25);
    if scaled > 0.0 { (ln_ref + scaled.ln()).min(0.0) } else { f64::NEG_INFINITY }
}

fn ln_chi_w_density(w: f64, ln_q: f64, df: f64, lg_a: f64) -> f64 {
    if w <= 0.0 {
        return f64::NEG_INFINITY;
    }
    let a = 0.5 * df;
    let ln_w = w.ln();
    let ln_u = ln_w - ln_q;
    let ln_x = a.ln() + 2.0 * ln_u;
    let ln_kern = if ln_x < -40.0 {
        if a >= 12.0 { df * ln_u + a + 0.5 * (a / std::f64::consts::TAU).ln() - stirling_err(a) } else { a * ln_x - lg_a }
    } else {
        let x = ln_x.exp();
        if a >= 12.0 { -deviance_bd0(a, x) + 0.5 * (a / std::f64::consts::TAU).ln() - stirling_err(a) } else { a * ln_x - x - lg_a }
    };
    std::f64::consts::LN_2 - ln_w + ln_kern
}

/// Evaluate `ln P(Q_{k, df} > q)` via log-sum-exp quadrature in `w = q * u` when `studentized_range_sf`
/// underflows `f64`, capturing both the finite-`df` polynomial `O(q^{-df})` exact integral and the
/// large-`df` saddle-point regime without resorting to a loose Bonferroni bound.
fn integrate_over_chi_scale_ln_sf(q: f64, k: usize, df: f64) -> Result<f64, MathError> {
    let a = 0.5 * df;
    let lg_a = if a < 12.0 { ln_gamma(a)? } else { 0.0 };
    let ln_q = q.ln();
    let inv_q = 1.0 / q;
    let rate_w = df * inv_q * inv_q + 0.5;
    let w_range = 2.0 * (2.0 * (k as f64).ln().max(1.0)).sqrt();

    if df < 2.0 {
        // Substitute w = t^{1/a} so w^{df - 1} dw = (1/a) t dt removes the w -> 0 singularity.
        let inv_a = 1.0 / a;
        let w_scale = (1.0 / rate_w).sqrt();
        let w_max = (w_range + 14.0 * w_scale).max(36.0);
        let t_max = w_max.powf(a);
        let const_ln = std::f64::consts::LN_2 + (a - 1.0) * a.ln() - lg_a - df * ln_q;
        let ln_t_integrand = |t: f64| -> f64 {
            if t <= 0.0 {
                return f64::NEG_INFINITY;
            }
            let ln_w = t.ln() * inv_a;
            let w = ln_w.exp();
            let ln_x = a.ln() + 2.0 * (ln_w - ln_q);
            let x = if ln_x < -50.0 { 0.0 } else { ln_x.exp() };
            const_ln + t.ln() - x + gaussian_range_ln_sf(w, k)
        };
        let probes_w = [0.25 * w_scale, w_scale, 0.5 * w_range, w_range, 0.4 * w_max];
        let mut ln_ref = f64::NEG_INFINITY;
        for &wp in &probes_w {
            let tp = wp.clamp(1e-4, 0.95 * w_max).powf(a);
            ln_ref = ln_ref.max(ln_t_integrand(tp));
        }
        let knots = [(0.25 * w_scale).powf(a), w_scale.powf(a), w_range.powf(a), (0.5 * w_max).powf(a)];
        let scaled = sum_gl32_panels(|t| (ln_t_integrand(t) - ln_ref).exp(), 0.0, t_max, &knots, 1e-4 * t_max);
        if scaled > 0.0 {
            return Ok((ln_ref + scaled.ln()).min(0.0));
        }
    } else {
        let w_star = ((df - 1.0) / rate_w).sqrt();
        let sigma_star = (0.5 / rate_w).sqrt();
        let w_chi_mode = if q < 1e100 { q * ((df - 1.0) / df).sqrt() } else { f64::INFINITY };
        let w_peak = (w_range + (df - 1.0).sqrt()).min(w_chi_mode).max(w_star);
        let w_min = (w_star - 11.0 * sigma_star).max(0.0);
        let w_max = (w_peak + 13.0 * sigma_star).max(w_star + 11.0 * sigma_star);
        let min_gap = (0.25 * sigma_star).max(1e-6);

        let ln_w_integrand = |w: f64| -> f64 { ln_chi_w_density(w, ln_q, df, lg_a) + gaussian_range_ln_sf(w, k) };
        let probes =
            [(0.5 * w_star).max(w_min + min_gap), w_star, 0.5 * w_star + 0.5 * w_peak, w_peak, (w_peak + 2.0 * sigma_star).min(w_max - min_gap)];
        let mut ln_ref = f64::NEG_INFINITY;
        for &wp in &probes {
            if wp > w_min && wp < w_max {
                ln_ref = ln_ref.max(ln_w_integrand(wp));
            }
        }
        let knots = [w_star - 3.5 * sigma_star, w_star, w_star + 3.5 * sigma_star, w_peak - 3.5 * sigma_star, w_peak, w_peak + 3.5 * sigma_star];
        let scaled = sum_gl32_panels(|w| (ln_w_integrand(w) - ln_ref).exp(), w_min, w_max, &knots, min_gap);
        if scaled > 0.0 {
            return Ok((ln_ref + scaled.ln()).min(0.0));
        }
    }

    let pairs = 0.5 * (k as f64) * ((k - 1) as f64);
    Ok((pairs.ln() + crate::dist::core::student_t_ln_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df)?).min(0.0))
}

/// Natural logarithm of the Studentized range upper-tail probability `ln P(Q_{k, df} > q)`.
///
/// Evaluates log-sum-exp quadrature directly in `(w = q u, z)` space whenever `studentized_range_sf`
/// approaches or crosses `f64` underflow (`<= 1e-250`), avoiding the `O(ln(k^2))` overstatement of a
/// naive Bonferroni bound on finite degrees of freedom.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `q` is `NaN`.
pub fn studentized_range_ln_sf(q: f64, k: usize, df: f64) -> Result<f64, MathError> {
    validate_q_args(q, k, df)?;
    if q <= 0.0 {
        return Ok(0.0);
    }
    if q.is_infinite() {
        return Ok(f64::NEG_INFINITY);
    }
    if k == 2 {
        return crate::dist::core::student_t_ln_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df);
    }
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(gaussian_range_ln_sf(q, k));
    }
    let sf = studentized_range_sf(q, k, df)?;
    if sf > 1e-250 { Ok(sf.ln()) } else { integrate_over_chi_scale_ln_sf(q, k, df) }
}

/// Upper-tail critical value `q_{alpha, k, df}` solving `P(Q_{k, df} > q) = alpha` for `alpha in [0, 1]`.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, `alpha` lies outside `[0, 1]`, or bracketing fails.
pub fn studentized_range_isf(alpha: f64, k: usize, df: f64) -> Result<f64, MathError> {
    if k < 2 {
        return Err(MathError::InvalidParameter { name: "k", value: k as f64 });
    }
    max_studentized_range_isf(alpha, 1, k, df)
}

/// Lower-tail quantile `F^{-1}(p)` of the Studentized range distribution solving `P(Q_{k, df} <= q) = p`.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, or `p` lies outside `[0, 1]`.
pub fn studentized_range_ppf(p: f64, k: usize, df: f64) -> Result<f64, MathError> {
    if !p.is_finite() || !(0.0..=1.0).contains(&p) {
        return Err(MathError::DomainError { name: "p", value: p });
    }
    studentized_range_isf(1.0 - p, k, df)
}

/// Lower-tail CDF `F(q; r, c, df) = P(max_{1 <= i <= r} Q_i <= q)` of the maximum of `r >= 1`
/// independent `c`-group Studentized ranges (`c >= 2`) sharing a common variance estimate on `df`
/// degrees of freedom (Copenhaver & Holland, 1988, Eq. 2.3-2.4).
///
/// When `r == 1`, this coincides with [`studentized_range_cdf`].
///
/// # Errors
/// Returns an error if `r < 1`, `c < 2`, `df <= 0`, or `q` is `NaN`.
pub fn max_studentized_range_cdf(q: f64, r: usize, c: usize, df: f64) -> Result<f64, MathError> {
    validate_max_q_args(q, r, c, df)?;
    if r == 1 {
        return studentized_range_cdf(q, c, df);
    }
    if q <= 0.0 {
        return Ok(0.0);
    }
    if q.is_infinite() {
        return Ok(1.0);
    }
    let r_i = i32::try_from(r).unwrap_or(i32::MAX);
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(gaussian_range_cdf(q, c).powi(r_i).clamp(0.0, 1.0));
    }
    Ok(integrate_over_chi_scale(df, |u| gaussian_range_cdf(q * u, c).powi(r_i))?.clamp(0.0, 1.0))
}

/// Upper-tail *p*-value `P(max_{1 <= i <= r} Q_i > q)` of the maximum of `r >= 1` Studentized ranges
/// (`c >= 2` means each, common `df > 0`) evaluated directly without `1.0 - cdf` cancellation.
///
/// # Errors
/// Returns an error if `r < 1`, `c < 2`, `df <= 0`, or `q` is `NaN`.
pub fn max_studentized_range_sf(q: f64, r: usize, c: usize, df: f64) -> Result<f64, MathError> {
    validate_max_q_args(q, r, c, df)?;
    if r == 1 {
        return studentized_range_sf(q, c, df);
    }
    if q <= 0.0 {
        return Ok(1.0);
    }
    if q.is_infinite() {
        return Ok(0.0);
    }
    if df.is_infinite() || df >= HUGE_DF_CUTOFF {
        return Ok(max_gaussian_range_sf(q, r, c));
    }
    Ok(integrate_over_chi_scale_tail(df, q * std::f64::consts::FRAC_1_SQRT_2, |u| max_gaussian_range_sf(q * u, r, c))?.clamp(0.0, 1.0))
}

/// Upper-tail critical value `q(alpha; r, c, df)` solving `P(max_{1 <= i <= r} Q_i > q) = alpha`
/// for `r >= 1`, `c >= 2`, `df > 0`, and `alpha in [0, 1]` (Copenhaver & Holland, 1988).
///
/// # Errors
/// Returns an error if `r < 1`, `c < 2`, `df <= 0`, `alpha` lies outside `[0, 1]`, or bracketing fails.
pub fn max_studentized_range_isf(alpha: f64, r: usize, c: usize, df: f64) -> Result<f64, MathError> {
    if !df.is_infinite() && df < HUGE_DF_CUTOFF && c >= 3 && r >= 1 && df > 0.0 && alpha.is_finite() && (0.0..=1.0).contains(&alpha) {
        let mut cache = StudentizedRangeCache::new();
        return cache.max_isf(alpha, r, c, df);
    }
    max_studentized_range_isf_with(alpha, r, c, df, |q| max_studentized_range_sf(q, r, c, df))
}

fn max_studentized_range_isf_with<F>(alpha: f64, r: usize, c: usize, df: f64, mut eval_sf: F) -> Result<f64, MathError>
where
    F: FnMut(f64) -> Result<f64, MathError>,
{
    if r < 1 {
        return Err(MathError::InvalidParameter { name: "r", value: r as f64 });
    }
    if c < 2 {
        return Err(MathError::InvalidParameter { name: "c", value: c as f64 });
    }
    if df.is_nan() || df <= 0.0 {
        return Err(MathError::InvalidParameter { name: "df", value: df });
    }
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return Err(MathError::DomainError { name: "alpha", value: alpha });
    }
    if alpha == 0.0 {
        return Ok(f64::INFINITY);
    }
    if alpha == 1.0 {
        return Ok(0.0);
    }
    if r == 1 && c == 2 {
        return Ok(std::f64::consts::SQRT_2 * student_t_two_sided_isf(alpha, df)?);
    }

    // Rigorous mathematical bounds:
    // Lower bound: single-pair range (r = 1, c = 2) sqrt(2) * t_{alpha, df}.
    // Upper bound: Bonferroni over all r * c * (c - 1) / 2 pairwise comparisons.
    let pairs_per_range = 0.5 * (c as f64) * ((c - 1) as f64);
    let total_pairs = (r as f64) * pairs_per_range;
    let lo_base = std::f64::consts::SQRT_2 * student_t_two_sided_isf(alpha, df)?;
    let hi_base = std::f64::consts::SQRT_2 * student_t_two_sided_isf((alpha / total_pairs).max(f64::MIN_POSITIVE), df)?;
    let mut lo = (0.99 * lo_base).max(0.0);
    let mut hi = 1.02 * hi_base + 0.1;

    for _ in 0..MAX_BRACKET_STEPS {
        if eval_sf(lo)? >= alpha || lo == 0.0 {
            break;
        }
        lo *= 0.5;
    }
    for _ in 0..MAX_BRACKET_STEPS {
        if eval_sf(hi)? <= alpha {
            break;
        }
        hi *= 1.5;
    }
    if eval_sf(lo)? < alpha || eval_sf(hi)? > alpha {
        return Err(MathError::NotConverged { algorithm: "max_studentized_range_isf", iterations: MAX_BRACKET_STEPS });
    }

    brent_root(|q| eval_sf(q).unwrap_or(f64::NAN) - alpha, lo, hi, RootOptions { rel_tol: 1e-11, abs_tol: 1e-12, max_iters: 96 })
}

// ============================================================================
// Batched & Reusable Studentized-Range Survival Evaluation
// ============================================================================

struct ChebRule16 {
    nodes: [f64; CHEB_POINTS],
    dct_weights: [[f64; CHEB_POINTS]; CHEB_POINTS],
}

static CHEB_16: OnceLock<ChebRule16> = OnceLock::new();

fn cheb_rule_16() -> &'static ChebRule16 {
    CHEB_16.get_or_init(|| {
        let n_f = CHEB_DEGREE as f64;
        let mut nodes = [0.0_f64; CHEB_POINTS];
        for (j, slot) in nodes.iter_mut().enumerate() {
            *slot = (std::f64::consts::PI * (j as f64) / n_f).cos();
        }
        let mut dct_weights = [[0.0_f64; CHEB_POINTS]; CHEB_POINTS];
        for (k, row) in dct_weights.iter_mut().enumerate() {
            let dk = if k == 0 || k == CHEB_DEGREE { 2.0 } else { 1.0 };
            for (j, cell) in row.iter_mut().enumerate() {
                let dj = if j == 0 || j == CHEB_DEGREE { 2.0 } else { 1.0 };
                *cell = (2.0 / (n_f * dk * dj)) * (std::f64::consts::PI * (j as f64) * (k as f64) / n_f).cos();
            }
        }
        ChebRule16 { nodes, dct_weights }
    })
}

/// Rigorous lower cutoff `w_lo(c)` below which `H_c(w) <= 1e-17` (hence `1 - H_c(w) = 1` and
/// `1 - H_c(w)^r = 1` to within `1e-17 < 0.1 ULP` for every `r >= 1`).
///
/// Proof: `H_c(w) = c int phi(z) [Phi(z + w) - Phi(z)]^{c - 1} dz`. For every `w > 0`,
/// `Phi(z + w) - Phi(z)` attains its global maximum at `z = -w / 2` with value
/// `Phi(w / 2) - Phi(-w / 2) = 1 - 2 Phi(-w / 2)`. Hence `H_c(w) <= c [1 - 2 Phi(-w / 2)]^{c - 1}`.
fn gaussian_range_saturation_lower_bound(c: usize) -> f64 {
    if c <= 2 {
        return 0.0;
    }
    let tau = 1e-17_f64;
    let log_ratio = (tau / (c as f64)).ln() / ((c - 1) as f64);
    let p0 = -log_ratio.exp_m1();
    if !p0.is_finite() || p0 <= 0.0 || p0 >= 1.0 {
        return 0.0;
    }
    match norm_isf(0.5 * p0) {
        Ok(z) if z.is_finite() && z > 0.0 => {
            let w_lo = 2.0 * z;
            if w_lo < 1e-4 { 0.0 } else { w_lo }
        }
        _ => 0.0,
    }
}

#[inline]
fn eval_psi_sample(w: f64, c: usize) -> f64 {
    if w <= 0.0 {
        return 0.0;
    }
    if c == 2 {
        return crate::special::erfcx(0.5 * w).ln();
    }
    gaussian_range_ln_sf(w, c) + 0.25 * w * w
}

#[inline]
fn max_sf_from_ln_g(ln_g: f64, r: usize) -> f64 {
    if ln_g >= 0.0 {
        return 1.0;
    }
    if r == 1 {
        return ln_g.exp().clamp(0.0, 1.0);
    }
    if ln_g < -std::f64::consts::LN_2 {
        let s = ln_g.exp();
        if s <= 0.0 { 0.0 } else { (-((-s).ln_1p() * (r as f64)).exp_m1()).clamp(0.0, 1.0) }
    } else {
        let h = (-ln_g.exp_m1()).clamp(0.0, 1.0);
        if h <= 0.0 { 1.0 } else { (1.0 - pow_usize(h, r)).clamp(0.0, 1.0) }
    }
}

#[derive(Debug, Clone)]
enum PanelKind {
    Chebyshev { center: f64, inv_half: f64, coeffs: [f64; CHEB_POINTS] },
    ExactFallback,
}

#[derive(Debug, Clone)]
struct RangePanel {
    a: f64,
    b: f64,
    kind: PanelKind,
}

impl RangePanel {
    #[inline]
    fn eval_max_sf(&self, w: f64, r: usize, c: usize) -> f64 {
        match &self.kind {
            PanelKind::Chebyshev { center, inv_half, coeffs } => {
                let x = ((w - *center) * *inv_half).clamp(-1.0, 1.0);
                let two_x = 2.0 * x;
                let [c0, rest @ ..] = coeffs;
                let mut b1 = 0.0_f64;
                let mut b2 = 0.0_f64;
                for &ck in rest.iter().rev() {
                    let b0 = two_x * b1 - b2 + ck;
                    b2 = b1;
                    b1 = b0;
                }
                let psi = x * b1 - b2 + *c0;
                let ln_g = (psi - 0.25 * w * w).min(0.0);
                max_sf_from_ln_g(ln_g, r)
            }
            PanelKind::ExactFallback => max_gaussian_range_sf(w, r, c),
        }
    }
}

/// Adaptive piecewise degree-16 Chebyshev-Lobatto interpolant of the scaled log-survival
/// `psi_c(w) = ln(1 - H_c(w)) + w^2 / 4` over `[w_lo(c), w_covered]`.
///
/// Extensions always advance in canonical coarse breakpoint chunks so repeated small ascending
/// queries never create micro-panels, never drop endpoints, and never exhaust the panel budget.
///
/// Error bounds (Trefethen 2013, Ch. 4 & 8; Mason & Handscomb 2003, §4.6; Copenhaver & Holland 1988):
/// - For `w <= w_lo(c)`: proven `H_c(w) <= 1e-17`, so `1 - H_c(w)^r = 1` within `1e-17`.
/// - For `w >= GAUSSIAN_RANGE_ZERO_W` (`0.25 * w^2 > 760`): `1 - H_c(w) = 0` in `f64`.
/// - On each accepted Chebyshev panel, `|c_13| + |c_14| + |c_15| + |c_16| <= max(1e-12, 1e-13 * ||psi_c||_inf)`,
///   bounding `|ln_hat - ln_exact| <= 3e-12`. Any panel that cannot split to meet tolerance falls
///   back to exact [`max_gaussian_range_sf`] rather than serving an unconverged interpolant.
///   Because `|d ln(1 - (1 - e^L)^r) / dL| <= 1` for all `L < 0` and `r >= 1`, the relative error
///   in `G_{r,c}(w)` (and in any non-negative outer quadrature sum over `u`) is strictly bounded
///   by `exp(eps) - 1 <= 3e-12`.
#[derive(Debug, Clone)]
struct GaussianRangeTable {
    c: usize,
    w_lo: f64,
    w_covered: f64,
    breakpoints: Vec<f64>,
    next_bp_idx: usize,
    last_endpoint_psi: Option<f64>,
    panels: Vec<RangePanel>,
}

impl GaussianRangeTable {
    fn new(c: usize) -> Self {
        let w_lo = gaussian_range_saturation_lower_bound(c);
        let breakpoints = build_canonical_breakpoints(c, w_lo);
        Self { c, w_lo, w_covered: w_lo, breakpoints, next_bp_idx: 1, last_endpoint_psi: None, panels: Vec::new() }
    }

    fn ensure_up_to(&mut self, w_target: f64) {
        let target = w_target.min(GAUSSIAN_RANGE_ZERO_W);
        if target <= self.w_covered {
            return;
        }
        while let Some(&seg_end) = self.breakpoints.get(self.next_bp_idx) {
            if self.w_covered >= target {
                break;
            }
            let seg_start = self.w_covered;
            if self.panels.len() >= CHEB_MAX_PANELS {
                self.panels.push(RangePanel { a: seg_start, b: seg_end, kind: PanelKind::ExactFallback });
                self.last_endpoint_psi = None;
            } else {
                let f_start = self.last_endpoint_psi.unwrap_or_else(|| eval_psi_sample(seg_start, self.c));
                let f_end = eval_psi_sample(seg_end, self.c);
                self.append_adaptive_interval(seg_start, seg_end, f_start, f_end);
                self.last_endpoint_psi = Some(f_end);
            }
            self.w_covered = self.panels.last().map_or(self.w_lo, |p| p.b);
            self.next_bp_idx += 1;
        }
    }

    fn append_adaptive_interval(&mut self, a: f64, b: f64, fa: f64, fb: f64) {
        let rule = cheb_rule_16();
        let mut stack: Vec<(f64, f64, f64, f64, usize)> = Vec::with_capacity(16);
        stack.push((a, b, fa, fb, 0));

        while let Some((lo, hi, f_lo, f_hi, depth)) = stack.pop() {
            let center = 0.5 * lo + 0.5 * hi;
            let half = 0.5 * hi - 0.5 * lo;
            if half <= 0.0 {
                continue;
            }
            let mut vals = [0.0_f64; CHEB_POINTS];
            let mut max_abs = f_lo.abs().max(f_hi.abs());
            for (j, (val_slot, &xj)) in vals.iter_mut().zip(rule.nodes.iter()).enumerate() {
                let v = if j == 0 {
                    f_hi
                } else if j == CHEB_DEGREE {
                    f_lo
                } else {
                    eval_psi_sample(center + half * xj, self.c)
                };
                *val_slot = v;
                max_abs = max_abs.max(v.abs());
            }

            let mut coeffs = [0.0_f64; CHEB_POINTS];
            for (ck, row) in coeffs.iter_mut().zip(rule.dct_weights.iter()) {
                let mut acc = NeumaierSum::new();
                for (&w_kj, &fj) in row.iter().zip(vals.iter()) {
                    acc.add(w_kj * fj);
                }
                *ck = acc.total();
            }

            let tail = coeffs.get(CHEB_DEGREE - 3..).map_or(0.0, |s| s.iter().map(|v| v.abs()).sum::<f64>());
            let tol = CHEB_ABS_TOL.max(CHEB_REL_TOL * max_abs);
            let can_split = depth < CHEB_MAX_DEPTH && (hi - lo) > 1e-5 && self.panels.len() + stack.len() + 2 <= CHEB_MAX_PANELS;

            if tail <= tol {
                self.panels.push(RangePanel { a: lo, b: hi, kind: PanelKind::Chebyshev { center, inv_half: 1.0 / half, coeffs } });
            } else if can_split {
                let f_mid = vals.get(CHEB_DEGREE / 2).copied().unwrap_or_else(|| eval_psi_sample(center, self.c));
                // Push right half first so left half pops first, keeping `self.panels` sorted.
                stack.push((center, hi, f_mid, f_hi, depth + 1));
                stack.push((lo, center, f_lo, f_mid, depth + 1));
            } else {
                self.panels.push(RangePanel { a: lo, b: hi, kind: PanelKind::ExactFallback });
            }
        }
    }

    #[inline]
    fn eval_max_sf_with_hint(&self, w: f64, r: usize, panel_hint: &mut usize) -> f64 {
        if w <= self.w_lo {
            return 1.0;
        }
        if w >= GAUSSIAN_RANGE_ZERO_W {
            return 0.0;
        }
        if w > self.w_covered || self.panels.is_empty() {
            return max_gaussian_range_sf(w, r, self.c);
        }
        let idx = match self.panels.get(*panel_hint) {
            Some(p) if w >= p.a && w <= p.b => *panel_hint,
            Some(p) if w > p.b => {
                let mut i = *panel_hint + 1;
                while let Some(next) = self.panels.get(i) {
                    if w <= next.b {
                        break;
                    }
                    i += 1;
                }
                i.min(self.panels.len().saturating_sub(1))
            }
            _ => self.panels.partition_point(|p| p.b < w).min(self.panels.len().saturating_sub(1)),
        };
        *panel_hint = idx;
        match self.panels.get(idx) {
            Some(panel) => panel.eval_max_sf(w, r, self.c),
            None => max_gaussian_range_sf(w, r, self.c),
        }
    }

    #[inline]
    fn eval_max_sf(&self, w: f64, r: usize) -> f64 {
        let mut hint = 0_usize;
        self.eval_max_sf_with_hint(w, r, &mut hint)
    }
}

fn build_canonical_breakpoints(c: usize, w_lo: f64) -> Vec<f64> {
    if w_lo >= GAUSSIAN_RANGE_ZERO_W {
        return vec![w_lo];
    }
    let w_mid = if c > 2 { 2.0 * norm_isf(1.0 / (c as f64)).unwrap_or(0.0).max(0.0) } else { 0.0 };
    let mut candidates = [w_mid, w_mid + 1.5, w_mid + 3.5, w_mid + 7.0, w_mid + 14.0, 28.0, GAUSSIAN_RANGE_LN_SWITCH_W];
    candidates.sort_by(f64::total_cmp);

    let mut breaks = Vec::with_capacity(candidates.len() + 2);
    breaks.push(w_lo);
    let min_span = 0.25_f64;
    for bp in candidates {
        if bp <= w_lo + min_span || bp >= GAUSSIAN_RANGE_ZERO_W - min_span {
            continue;
        }
        let prev = breaks.last().copied().unwrap_or(w_lo);
        if bp >= prev + min_span {
            breaks.push(bp);
        } else if (bp - GAUSSIAN_RANGE_LN_SWITCH_W).abs() <= 1e-12 && breaks.len() > 1 {
            // Preserve the ln_sf branch switch boundary without leaving a tiny sliver before it.
            breaks.pop();
            breaks.push(bp);
        }
    }
    breaks.push(GAUSSIAN_RANGE_ZERO_W);
    breaks
}

/// Precomputed sorted outer chi-scale quadrature nodes `(u_j, w_j * h_{df}(u_j))` and cumulative
/// weight sums for `df >= 3.0`, reusable across all bulk-regime `q` values sharing `df`.
#[derive(Debug, Clone)]
struct ChiScaleNodes {
    df: f64,
    u_mode: f64,
    sigma_u: f64,
    u_min: f64,
    u: Vec<f64>,
    w: Vec<f64>,
    prefix_w: Vec<f64>,
}

impl ChiScaleNodes {
    fn build(df: f64) -> Result<Self, MathError> {
        let a = 0.5 * df;
        let lg_a = if a < 12.0 { ln_gamma(a)? } else { 0.0 };
        let norm_const = chi_scale_norm_const(a, lg_a);
        let rt_df = df.sqrt();
        let rt_dm1 = (df - 1.0).sqrt();
        let u_mode = rt_dm1 / rt_df;
        let sigma_u = 1.0 / (2.0 * df).sqrt();
        let u_min = (u_mode - 11.5 * sigma_u).max(0.0);
        let u_max = u_mode + 13.5 * sigma_u;
        let min_gap = (0.25 * sigma_u).max(1e-300);

        let raw_knots = [
            0.25 * u_mode,
            0.5 * u_mode,
            u_mode - 7.0 * sigma_u,
            u_mode - 3.5 * sigma_u,
            u_mode - 1.5 * sigma_u,
            u_mode,
            u_mode + 1.5 * sigma_u,
            u_mode + 4.0 * sigma_u,
        ];
        let mut knots = Vec::with_capacity(raw_knots.len() + 2);
        knots.push(u_min);
        for &x in &raw_knots {
            if x.is_finite() && x > u_min + min_gap && x + min_gap < u_max {
                knots.push(x);
            }
        }
        knots.push(u_max);
        knots.sort_by(f64::total_cmp);

        let mut edges = Vec::with_capacity(knots.len());
        let hi_guard = min_gap.max(1e-12 * u_max.abs());
        edges.push(u_min);
        let mut cur = u_min;
        for &x in &knots {
            if x > cur + min_gap && x + hi_guard < u_max {
                edges.push(x);
                cur = x;
            }
        }
        edges.push(u_max);

        let (gl_nodes, gl_weights) = gl32_nodes_weights();
        let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(edges.len().saturating_sub(1) * 32);
        for seg in edges.windows(2) {
            let [lo, hi] = seg else { continue };
            let center = 0.5 * lo + 0.5 * hi;
            let half = 0.5 * hi - 0.5 * lo;
            for (&xi, &wi) in gl_nodes.iter().zip(gl_weights.iter()) {
                let off = half * xi;
                for u_val in [center - off, center + off] {
                    let dens = chi_scale_pdf_fast(u_val, a, norm_const);
                    if dens > 1e-300 {
                        pairs.push((u_val, half * wi * dens));
                    }
                }
            }
        }
        pairs.sort_by(|lhs, rhs| lhs.0.total_cmp(&rhs.0));

        let mut u = Vec::with_capacity(pairs.len());
        let mut w = Vec::with_capacity(pairs.len());
        let mut prefix_w = Vec::with_capacity(pairs.len() + 1);
        let mut pref = NeumaierSum::new();
        prefix_w.push(0.0);
        for (ui, wi) in pairs {
            u.push(ui);
            w.push(wi);
            pref.add(wi);
            prefix_w.push(pref.total());
        }

        Ok(Self { df, u_mode, sigma_u, u_min, u, w, prefix_w })
    }

    /// True when the joint chi-scale saddle point and its support stay inside the finely resolved
    /// bulk panels of `self`, so reusing precomputed nodes preserves full quadrature accuracy.
    #[inline]
    fn is_bulk_eligible(&self, q: f64) -> bool {
        if q * self.sigma_u > 1.5 {
            return false;
        }
        let scale = q * std::f64::consts::FRAC_1_SQRT_2;
        let rt_eff = self.df.sqrt().hypot(scale);
        let rt_dm1 = (self.df - 1.0).sqrt();
        let u_star = rt_dm1 / rt_eff;
        let sigma_star = 1.0 / (std::f64::consts::SQRT_2 * rt_eff);
        u_star >= self.u_mode - 2.5 * self.sigma_u && sigma_star >= 0.6 * self.sigma_u && (u_star - 11.0 * sigma_star).max(0.0) >= self.u_min
    }

    fn eval_with_table(&self, q: f64, r: usize, table: &GaussianRangeTable) -> f64 {
        let u_sat = table.w_lo / q;
        let u_zero = GAUSSIAN_RANGE_ZERO_W / q;
        let lo_idx = self.u.partition_point(|&u| u <= u_sat);
        let hi_idx = self.u.partition_point(|&u| u < u_zero);
        let sat_mass = self.prefix_w.get(lo_idx).copied().unwrap_or(0.0);
        if lo_idx >= hi_idx {
            return sat_mass.clamp(0.0, 1.0);
        }
        let mut acc = NeumaierSum::new();
        acc.add(sat_mass);
        let mut panel_hint = 0_usize;
        if let (Some(u_slice), Some(w_slice)) = (self.u.get(lo_idx..hi_idx), self.w.get(lo_idx..hi_idx)) {
            for (&ui, &wi) in u_slice.iter().zip(w_slice.iter()) {
                acc.add(wi * table.eval_max_sf_with_hint(q * ui, r, &mut panel_hint));
            }
        }
        acc.total().clamp(0.0, 1.0)
    }
}

fn outer_u_max_for_df(df: f64) -> f64 {
    if df < 3.0 {
        let a = 0.5 * df;
        (65.0 / a).max(16.0).sqrt()
    } else {
        let u_mode = ((df - 1.0) / df).sqrt();
        let sigma_u = 1.0 / (2.0 * df).sqrt();
        u_mode + 13.5 * sigma_u
    }
}

/// Reusable evaluator cache for Studentized-range and maximum-Studentized-range survival
/// probabilities and critical values across a post-test family or across multiple slices.
///
/// Caches:
/// - Adaptive piecewise degree-16 Chebyshev tables of `psi_c(w) = ln(1 - H_c(w)) + w^2 / 4` keyed
///   by group count `c`, with documented pointwise error `<= 3e-12` in log space (hence relative
///   error `<= 3e-12` in `1 - H_c(w)^r` for all `r >= 1`, propagated without amplification through
///   non-negative outer quadrature weights).
/// - Precomputed sorted outer chi-scale quadrature nodes `(u_j, w_j h_{df}(u_j))` keyed by
///   `df.to_bits()` for bulk-regime `q` values when `df >= 3.0`, falling back automatically to
///   saddle-point-adapted outer quadrature (still backed by the Chebyshev table) for extreme tails
///   and `df < 3.0`.
#[derive(Debug, Clone, Default)]
pub struct StudentizedRangeCache {
    tables: BTreeMap<usize, GaussianRangeTable>,
    chi_nodes: BTreeMap<u64, ChiScaleNodes>,
}

impl StudentizedRangeCache {
    /// Create an empty Studentized-range evaluator cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_chi_nodes(&mut self, df: f64) -> Result<u64, MathError> {
        let key = df.to_bits();
        if let Entry::Vacant(slot) = self.chi_nodes.entry(key) {
            let built = ChiScaleNodes::build(df)?;
            slot.insert(built);
        }
        Ok(key)
    }

    fn ensure_table(&mut self, c: usize, w_target: f64) {
        let table = self.tables.entry(c).or_insert_with(|| GaussianRangeTable::new(c));
        table.ensure_up_to(w_target);
    }

    /// Evaluate a single upper-tail *p*-value `P(max_{1 <= i <= r} Q_i > q)` reusing cached tables
    /// and outer chi-scale nodes.
    ///
    /// # Errors
    /// Returns an error if `r < 1`, `c < 2`, `df <= 0`, `df` is `NaN`, or `q` is `NaN`.
    pub fn max_sf(&mut self, q: f64, r: usize, c: usize, df: f64) -> Result<f64, MathError> {
        validate_max_q_args(q, r, c, df)?;
        if q <= 0.0 {
            return Ok(1.0);
        }
        if q.is_infinite() {
            return Ok(0.0);
        }
        if r == 1 && c == 2 {
            return student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df);
        }
        if df.is_infinite() || df >= HUGE_DF_CUTOFF {
            if c == 2 {
                return Ok(max_gaussian_range_sf(q, r, 2));
            }
            if let Some(table) = self.tables.get_mut(&c) {
                table.ensure_up_to(q);
                return Ok(table.eval_max_sf(q, r));
            }
            return Ok(max_gaussian_range_sf(q, r, c));
        }

        let w_needed = (q * outer_u_max_for_df(df)).min(GAUSSIAN_RANGE_ZERO_W);
        self.ensure_table(c, w_needed);
        if df >= 3.0 {
            let key = self.ensure_chi_nodes(df)?;
            if let (Some(nodes), Some(table)) = (self.chi_nodes.get(&key), self.tables.get(&c))
                && nodes.is_bulk_eligible(q)
            {
                return Ok(nodes.eval_with_table(q, r, table));
            }
        }
        if let Some(table) = self.tables.get(&c) {
            return Ok(integrate_over_chi_scale_tail(df, q * std::f64::consts::FRAC_1_SQRT_2, |u| table.eval_max_sf(q * u, r))?.clamp(0.0, 1.0));
        }
        max_studentized_range_sf(q, r, c, df)
    }

    /// Evaluate a single Studentized-range upper-tail *p*-value `P(Q_{k, df} > q)` (`r = 1`).
    ///
    /// # Errors
    /// Returns an error if `k < 2`, `df <= 0`, `df` is `NaN`, or `q` is `NaN`.
    pub fn sf(&mut self, q: f64, k: usize, df: f64) -> Result<f64, MathError> {
        validate_q_args(q, k, df)?;
        self.max_sf(q, 1, k, df)
    }

    /// Evaluate a batch of upper-tail *p*-values `P(max_{1 <= i <= r} Q_i > q)` for fixed
    /// `(r, c, df)` in one sorted pass, reusing outer chi-scale quadrature nodes and the cached
    /// Chebyshev table of `ln(1 - H_c(w)) + w^2 / 4`.
    ///
    /// Output order matches `qs` exactly.
    ///
    /// # Errors
    /// Returns an error if `r < 1`, `c < 2`, `df <= 0`, `df` is `NaN`, or any element of `qs` is `NaN`.
    pub fn max_sf_batch(&mut self, qs: &[f64], r: usize, c: usize, df: f64) -> Result<Vec<f64>, MathError> {
        if r < 1 {
            return Err(MathError::InvalidParameter { name: "r", value: r as f64 });
        }
        if c < 2 {
            return Err(MathError::InvalidParameter { name: "c", value: c as f64 });
        }
        if df.is_nan() || df <= 0.0 {
            return Err(MathError::InvalidParameter { name: "df", value: df });
        }
        for &q in qs {
            if q.is_nan() {
                return Err(MathError::DomainError { name: "q", value: q });
            }
        }
        if qs.is_empty() {
            return Ok(Vec::new());
        }

        let mut out = vec![0.0_f64; qs.len()];
        let mut pending: Vec<(usize, f64)> = Vec::new();
        for (idx, (&q, slot)) in qs.iter().zip(out.iter_mut()).enumerate() {
            if q <= 0.0 {
                *slot = 1.0;
            } else if q.is_infinite() {
                *slot = 0.0;
            } else if r == 1 && c == 2 {
                *slot = student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df)?;
            } else {
                pending.push((idx, q));
            }
        }
        if pending.is_empty() {
            return Ok(out);
        }

        pending.sort_by(|lhs, rhs| lhs.1.total_cmp(&rhs.1));

        if df.is_infinite() || df >= HUGE_DF_CUTOFF {
            self.eval_batch_inf_df(&pending, r, c, &mut out);
            return Ok(out);
        }

        let q_max = pending.last().map_or(0.0, |&(_, q)| q);
        let w_max_needed = (q_max * outer_u_max_for_df(df)).min(GAUSSIAN_RANGE_ZERO_W);
        self.ensure_table(c, w_max_needed);
        let chi_key = if df >= 3.0 { Some(self.ensure_chi_nodes(df)?) } else { None };
        let Some(table) = self.tables.get(&c) else {
            return Err(MathError::DomainError { name: "c", value: c as f64 });
        };
        let nodes_opt = chi_key.and_then(|k| self.chi_nodes.get(&k));

        let mut prev_bits: Option<u64> = None;
        let mut prev_val = 0.0_f64;
        for &(idx, q) in &pending {
            let bits = q.to_bits();
            let val = if prev_bits == Some(bits) {
                prev_val
            } else {
                let computed = if let Some(nodes) = nodes_opt
                    && nodes.is_bulk_eligible(q)
                {
                    nodes.eval_with_table(q, r, table)
                } else {
                    integrate_over_chi_scale_tail(df, q * std::f64::consts::FRAC_1_SQRT_2, |u| table.eval_max_sf(q * u, r))?.clamp(0.0, 1.0)
                };
                prev_bits = Some(bits);
                prev_val = computed;
                computed
            };
            if let Some(slot) = out.get_mut(idx) {
                *slot = val;
            }
        }
        Ok(out)
    }

    fn eval_batch_inf_df(&mut self, pending_sorted: &[(usize, f64)], r: usize, c: usize, out: &mut [f64]) {
        let use_table = c >= 3 && (pending_sorted.len() > 12 || self.tables.contains_key(&c));
        if use_table {
            let q_max = pending_sorted.last().map_or(0.0, |&(_, q)| q);
            self.ensure_table(c, q_max);
        }
        let table_opt = if use_table { self.tables.get(&c) } else { None };
        let mut panel_hint = 0_usize;
        let mut prev_bits: Option<u64> = None;
        let mut prev_val = 0.0_f64;
        for &(idx, q) in pending_sorted {
            let bits = q.to_bits();
            let val = if prev_bits == Some(bits) {
                prev_val
            } else {
                let computed = match table_opt {
                    Some(table) => table.eval_max_sf_with_hint(q, r, &mut panel_hint),
                    None => max_gaussian_range_sf(q, r, c),
                };
                prev_bits = Some(bits);
                prev_val = computed;
                computed
            };
            if let Some(slot) = out.get_mut(idx) {
                *slot = val;
            }
        }
    }

    /// Evaluate a batch of Studentized-range upper-tail *p*-values `P(Q_{k, df} > q)` (`r = 1`).
    ///
    /// # Errors
    /// Returns an error if `k < 2`, `df <= 0`, `df` is `NaN`, or any element of `qs` is `NaN`.
    pub fn sf_batch(&mut self, qs: &[f64], k: usize, df: f64) -> Result<Vec<f64>, MathError> {
        if k < 2 {
            return Err(MathError::InvalidParameter { name: "k", value: k as f64 });
        }
        self.max_sf_batch(qs, 1, k, df)
    }

    /// Upper-tail critical value `q(alpha; r, c, df)` solving `P(max_{1 <= i <= r} Q_i > q) = alpha`,
    /// reusing cached tables and outer chi-scale nodes across the root search and subsequent calls.
    ///
    /// # Errors
    /// Returns an error if `r < 1`, `c < 2`, `df <= 0`, `alpha` lies outside `[0, 1]`, or bracketing fails.
    pub fn max_isf(&mut self, alpha: f64, r: usize, c: usize, df: f64) -> Result<f64, MathError> {
        max_studentized_range_isf_with(alpha, r, c, df, |q| self.max_sf(q, r, c, df))
    }

    /// Upper-tail Studentized-range critical value `q_{alpha, k, df}` (`r = 1`) reusing cached tables.
    ///
    /// # Errors
    /// Returns an error if `k < 2`, `df <= 0`, `alpha` lies outside `[0, 1]`, or bracketing fails.
    pub fn isf(&mut self, alpha: f64, k: usize, df: f64) -> Result<f64, MathError> {
        if k < 2 {
            return Err(MathError::InvalidParameter { name: "k", value: k as f64 });
        }
        self.max_isf(alpha, 1, k, df)
    }
}

/// Evaluate a batch of maximum-Studentized-range upper-tail *p*-values `P(max_{1 <= i <= r} Q_i > q)`
/// for fixed `(r, c, df)` in a single sorted pass, reusing outer chi-scale quadrature nodes and an
/// adaptive piecewise degree-16 Chebyshev table of `ln(1 - H_c(w)) + w^2 / 4` (Copenhaver & Holland
/// 1988; Trefethen 2013).
///
/// Guarantees relative agreement `<= 1e-11` with [`max_studentized_range_sf`] across `r >= 1`,
/// `c >= 2`, `df > 0` (including `df = +inf`), and all representable tails. Output order matches `qs`.
///
/// # Errors
/// Returns an error if `r < 1`, `c < 2`, `df <= 0`, `df` is `NaN`, or any element of `qs` is `NaN`.
pub fn max_studentized_range_sf_batch(qs: &[f64], r: usize, c: usize, df: f64) -> Result<Vec<f64>, MathError> {
    let mut cache = StudentizedRangeCache::new();
    cache.max_sf_batch(qs, r, c, df)
}

/// Evaluate a batch of Studentized-range upper-tail *p*-values `P(Q_{k, df} > q)` (`r = 1`) for
/// fixed `(k, df)` in a single sorted pass.
///
/// Guarantees relative agreement `<= 1e-11` with [`studentized_range_sf`]. Output order matches `qs`.
///
/// # Errors
/// Returns an error if `k < 2`, `df <= 0`, `df` is `NaN`, or any element of `qs` is `NaN`.
pub fn studentized_range_sf_batch(qs: &[f64], k: usize, df: f64) -> Result<Vec<f64>, MathError> {
    if k < 2 {
        return Err(MathError::InvalidParameter { name: "k", value: k as f64 });
    }
    max_studentized_range_sf_batch(qs, 1, k, df)
}

#[cfg(test)]
mod tests {
    use super::{
        gaussian_range_ln_sf, gaussian_range_sf, integrate_over_chi_scale_ln_sf, integrate_over_chi_scale_tail, max_studentized_range_cdf,
        max_studentized_range_isf, max_studentized_range_sf, studentized_range_cdf, studentized_range_isf, studentized_range_ln_sf,
        studentized_range_sf,
    };
    use crate::dist::core::student_t_two_sided_p;
    use crate::dist::dunnett::dunnett_two_sided_p_loadings;

    #[test]
    fn quadrature_k2_matches_exact_sqrt2_student_t_to_12_plus_digits() {
        // Test the full 2D quadrature engine directly on k = 2 (bypassing the k == 2 fast-path)
        // against exact sqrt(2) * |t_{df}| across integer, fractional, and deep-tail inputs.
        for &df in &[1.5, 4.0, 12.7, 60.0] {
            for &q in &[1.2, 3.5, 6.0, 12.0] {
                let quad_sf = integrate_over_chi_scale_tail(df, 0.5 * q * q, |u| gaussian_range_sf(q * u, 2)).unwrap();
                let exact_sf = student_t_two_sided_p(q * std::f64::consts::FRAC_1_SQRT_2, df).unwrap();
                assert!((quad_sf - exact_sf).abs() <= 1e-11 * exact_sf, "df={df}, q={q}: quad={quad_sf:.15e} vs exact={exact_sf:.15e}");
            }
        }

        // Verify extreme Gaussian upper tail w = 24.0 on k = 3 (saddle point z_* = -12 < -10):
        // P(Z_{(3)} - Z_{(1)} > w) ~ 3 * erfc(w / 2) = 3 * erfc(12) to > 12 digits!
        let sf_w24 = studentized_range_sf(24.0, 3, f64::INFINITY).unwrap();
        let asymp_w24 = 3.0 * crate::special::erfc(12.0);
        assert!((sf_w24 - asymp_w24).abs() <= 1e-11 * asymp_w24, "w=24: {sf_w24:.15e} vs {asymp_w24:.15e}");
    }

    #[test]
    fn hsd_critical_values_match_harter_and_copenhaver_holland_tables() {
        // Harter (1960) / NIST / Copenhaver & Holland (1988) standard certified critical values:
        // alpha = 0.05, k = 3, df = 10 -> q = 3.876777...
        let q_3_10 = studentized_range_isf(0.05, 3, 10.0).unwrap();
        assert!((q_3_10 - 3.876_776_7).abs() < 1e-6, "q_3_10 = {q_3_10}");

        // alpha = 0.05, k = 5, df = 20 -> q = 4.231856...
        let q_5_20 = studentized_range_isf(0.05, 5, 20.0).unwrap();
        assert!((q_5_20 - 4.231_856_5).abs() < 1e-6, "q_5_20 = {q_5_20}");

        // alpha = 0.01, k = 10, df = inf -> Harter (1960) table 5.16 (exact 5.156634960090...);
        // cross-verify directly against adaptive 15-pt Gauss-Kronrod quadrature to 1e-13!
        let q_10_inf = studentized_range_isf(0.01, 10, f64::INFINITY).unwrap();
        assert!((q_10_inf - 5.156_634_960_09).abs() < 1e-9, "q_10_inf = {q_10_inf}");
        let adaptive_cdf = crate::quad::integrate_adaptive(
            |z| 10.0 * crate::dist::core::norm_pdf(z) * super::normal_interval_prob(z, z + q_10_inf).powi(9),
            -10.0,
            6.0,
            crate::quad::QuadOptions { abs_tol: 1e-14, rel_tol: 1e-13, max_subdivisions: 512 },
        )
        .unwrap();
        assert!(((1.0 - adaptive_cdf) - 0.01).abs() < 1e-11, "adaptive sf = {}", 1.0 - adaptive_cdf);

        // Large group count k = 200 (per statistics-implementation.md §1.1: 2 <= k <= 200):
        let q_200_60 = studentized_range_isf(0.05, 200, 60.0).unwrap();
        let sf_back = studentized_range_sf(q_200_60, 200, 60.0).unwrap();
        assert!((sf_back - 0.05).abs() < 1e-10, "k=200 roundtrip: q={q_200_60}, sf={sf_back}");
        let cdf_back = studentized_range_cdf(q_200_60, 200, 60.0).unwrap();
        assert!(((cdf_back + sf_back) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn copenhaver_holland_1988_published_tables_and_sugar_cane_example() {
        // Published numerical values from Copenhaver, M. D., & Holland, B. (1988),
        // "Computation of the distribution of the maximum studentized range statistic with
        // application to multiple significance testing of simple effects", Journal of Statistical
        // Computation and Simulation, 30(1), 1-15, doi:10.1080/00949658808811082.
        //
        // Note: Table 1's printed heading reads "0.005", a well-known publisher misprint for
        // alpha = 0.05 (confirmed both by [H_3(3.68)]^2 = 0.95 at df = inf and by Section 5 p. 14).
        let table_1_alpha_05: &[(usize, usize, f64, f64)] = &[
            (2, 3, 5.0, 5.37),
            (2, 3, 10.0, 4.42),
            (2, 3, 20.0, 4.02),
            (2, 3, 60.0, 3.79),
            (2, 3, f64::INFINITY, 3.68),
            (3, 4, 15.0, 4.82),
            (4, 5, 20.0, 5.08),
            (5, 6, 40.0, 5.08),
            (10, 10, 120.0, 5.57),
            (20, 20, f64::INFINITY, 6.11),
        ];
        for &(r, c, df, published) in table_1_alpha_05 {
            let q = max_studentized_range_isf(0.05, r, c, df).unwrap();
            assert!((q - published).abs() <= 0.006, "Table 1 (alpha=0.05, r={r}, c={c}, df={df}): got {q:.6} vs {published:.2}");
        }

        // Table 2 (pp. 9-10): experimentwise type I error rate alpha = 0.10.
        let table_2_alpha_10: &[(usize, usize, f64, f64)] = &[
            (2, 3, 5.0, 4.41),
            (2, 3, 10.0, 3.80),
            (2, 3, 20.0, 3.54),
            (2, 3, f64::INFINITY, 3.30),
            (3, 3, 15.0, 3.90),
            (4, 4, 20.0, 4.34),
            (6, 5, 60.0, 4.52),
            (10, 8, 120.0, 5.08),
            (20, 20, f64::INFINITY, 5.87),
        ];
        for &(r, c, df, published) in table_2_alpha_10 {
            let q = max_studentized_range_isf(0.10, r, c, df).unwrap();
            assert!((q - published).abs() <= 0.006, "Table 2 (alpha=0.10, r={r}, c={c}, df={df}): got {q:.6} vs {published:.2}");
        }

        // Section 3 (p. 11) identity: when c = 2, max_{1..=r} Q_i / sqrt(2) follows the Studentized
        // maximum modulus distribution |T|_{r, df} with zero off-diagonal correlations (Dunnett lambda = 0).
        for &(r, df, q) in &[(4_usize, 18.0_f64, 3.6_f64), (6_usize, f64::INFINITY, 4.1_f64)] {
            let sf_q = max_studentized_range_sf(q, r, 2, df).unwrap();
            let sf_smm = dunnett_two_sided_p_loadings(q * std::f64::consts::FRAC_1_SQRT_2, df, &vec![0.0; r]).unwrap();
            assert!((sf_q - sf_smm).abs() < 1e-11, "SMM identity r={r}, df={df}: {sf_q} vs {sf_smm}");
            let cdf_q = max_studentized_range_cdf(q, r, 2, df).unwrap();
            assert!(((cdf_q + sf_q) - 1.0).abs() < 1e-12);
        }

        // Section 5 (p. 14) Cochran & Cox (1957) 3x3 sugar cane randomized-block example (4 blocks):
        // r = 3, c = 3, RCBD error df = (4 - 1) * (9 - 1) = 24, S / sqrt(n) = 3.3132;
        // paper reports critical differences 13.93 (alpha = 0.05) and 12.43 (alpha = 0.10).
        let se = 3.3132_f64;
        let rhs_05 = se * max_studentized_range_isf(0.05, 3, 3, 24.0).unwrap();
        let rhs_10 = se * max_studentized_range_isf(0.10, 3, 3, 24.0).unwrap();
        assert!((rhs_05 - 13.93).abs() < 0.005, "sugar cane alpha=0.05 RHS = {rhs_05:.4}");
        assert!((rhs_10 - 12.43).abs() < 0.005, "sugar cane alpha=0.10 RHS = {rhs_10:.4}");
    }

    #[test]
    fn ln_sf_quadrature_matches_linear_sf_and_resolves_deep_underflow_without_bonferroni_bias() {
        // Direct cross-check on (q = 60, k = 50, df = 5): log-space quadrature agrees with linear sf.ln()
        // and lies ~2.98 log-units (~19.7x) below the loose Bonferroni upper bound.
        let lsf_60 = studentized_range_ln_sf(60.0, 50, 5.0).unwrap();
        let quad_ln_60 = integrate_over_chi_scale_ln_sf(60.0, 50, 5.0).unwrap();
        assert!((lsf_60 - (-11.668_195)).abs() < 1e-4, "lsf_60 = {lsf_60}");
        assert!((quad_ln_60 - lsf_60).abs() < 1e-10, "quad_ln_60={quad_ln_60} vs lsf_60={lsf_60}");

        // Deep underflow on finite df = 5 (q = 1e80 -> sf = 0.0 in f64):
        // True finite-df tail scales as q^{-df} * (2 (df/2)^{df/2} / Gamma(df/2)) * int w^{df-1} (1 - H_k(w)) dw,
        // which stays ~2.97 log-units below the Bonferroni upper bound ln(1225) + ln P(|t_5| > q / sqrt(2)).
        assert_eq!(studentized_range_sf(1e80, 50, 5.0).unwrap(), 0.0);
        let lsf_1e80 = studentized_range_ln_sf(1e80, 50, 5.0).unwrap();
        let bonf_1e80 = (1225.0_f64).ln() + crate::dist::core::student_t_ln_two_sided_p(1e80 * std::f64::consts::FRAC_1_SQRT_2, 5.0).unwrap();
        assert!((lsf_1e80 - (-912.217_9)).abs() < 1e-3, "lsf_1e80 = {lsf_1e80}");
        assert!(bonf_1e80 - lsf_1e80 > 2.95 && bonf_1e80 - lsf_1e80 < 3.00, "gap = {}", bonf_1e80 - lsf_1e80);

        // Deep Gaussian range tail (q = 60, k = 50, df = inf, where sf ~ e^{-896.86} underflows f64):
        assert_eq!(studentized_range_sf(60.0, 50, f64::INFINITY).unwrap(), 0.0);
        let lsf_inf = studentized_range_ln_sf(60.0, 50, f64::INFINITY).unwrap();
        let g_ln = gaussian_range_ln_sf(60.0, 50);
        assert_eq!(lsf_inf, g_ln);
        assert!((lsf_inf - (-896.863_421)).abs() < 1e-4, "lsf_inf = {lsf_inf}");
    }

    fn assert_batch_matches_scalar(qs: &[f64], r: usize, c: usize, df: f64) {
        let batched = super::max_studentized_range_sf_batch(qs, r, c, df).unwrap();
        assert_eq!(batched.len(), qs.len());
        if r == 1 {
            assert_eq!(super::studentized_range_sf_batch(qs, c, df).unwrap(), batched);
        }
        for (&q, &p_batch) in qs.iter().zip(batched.iter()) {
            let p_scalar = max_studentized_range_sf(q, r, c, df).unwrap();
            assert!(p_batch.is_finite() && (0.0..=1.0).contains(&p_batch), "r={r} c={c} df={df} q={q}: bad p_batch={p_batch}");
            if q <= 0.0 {
                assert_eq!(p_batch, 1.0, "q <= 0 must be exact 1.0");
            } else if q.is_infinite() {
                assert_eq!(p_batch, 0.0, "q = +inf must be exact 0.0");
            } else if p_scalar == 0.0 {
                assert!(p_batch <= 1e-300, "r={r} c={c} df={df} q={q}: p_scalar=0 vs p_batch={p_batch:e}");
            } else {
                let abs_err = (p_batch - p_scalar).abs();
                let tol = 1e-14_f64.max(1e-11 * p_scalar);
                assert!(
                    abs_err <= tol,
                    "r={r} c={c} df={df} q={q}: batch={p_batch:.16e} vs scalar={p_scalar:.16e} (abs={abs_err:.3e}, rel={:.3e})",
                    abs_err / p_scalar
                );
            }
        }
    }

    #[test]
    fn issue_82_batched_validation_and_cache_extension() {
        use super::{StudentizedRangeCache, max_studentized_range_sf_batch, studentized_range_sf_batch};
        use crate::error::MathError;

        // Parameter validation errors: c = 1, r = 0, invalid df, NaN in qs.
        assert!(matches!(max_studentized_range_sf_batch(&[2.0], 1, 1, 10.0), Err(MathError::InvalidParameter { name: "c", .. })));
        assert!(matches!(max_studentized_range_sf_batch(&[], 1, 1, 10.0), Err(MathError::InvalidParameter { name: "c", .. })));
        assert!(matches!(studentized_range_sf_batch(&[2.0], 1, 10.0), Err(MathError::InvalidParameter { name: "k", .. })));
        assert!(matches!(max_studentized_range_sf_batch(&[2.0], 0, 3, 10.0), Err(MathError::InvalidParameter { name: "r", .. })));
        assert!(matches!(max_studentized_range_sf_batch(&[2.0], 2, 3, 0.0), Err(MathError::InvalidParameter { name: "df", .. })));
        assert!(matches!(max_studentized_range_sf_batch(&[2.0], 2, 3, -5.0), Err(MathError::InvalidParameter { name: "df", .. })));
        assert!(matches!(max_studentized_range_sf_batch(&[2.0], 2, 3, f64::NAN), Err(MathError::InvalidParameter { name: "df", .. })));
        assert!(matches!(max_studentized_range_sf_batch(&[0.0, 3.0, f64::NAN, 5.0], 2, 4, 20.0), Err(MathError::DomainError { name: "q", .. })));
        assert!(max_studentized_range_sf_batch(&[], 2, 4, 20.0).unwrap().is_empty());

        // Small-batch df = +inf direct path (<= 12 points).
        let qs_small_inf = [0.0, 2.5, 5.0, 12.0, f64::INFINITY];
        let batch_small_inf = max_studentized_range_sf_batch(&qs_small_inf, 2, 12, f64::INFINITY).unwrap();
        for (&q, &p_batch) in qs_small_inf.iter().zip(batch_small_inf.iter()) {
            assert_eq!(p_batch.to_bits(), max_studentized_range_sf(q, 2, 12, f64::INFINITY).unwrap().to_bits(), "small df=inf q={q}");
        }

        // Incremental cache extension: query small q first, then larger q later on the same cache.
        let mut cache = StudentizedRangeCache::new();
        let p_lo = cache.max_sf(3.2, 2, 12, 30.0).unwrap();
        let p_hi = cache.max_sf(18.5, 2, 12, 30.0).unwrap();
        let s_lo = max_studentized_range_sf(3.2, 2, 12, 30.0).unwrap();
        let s_hi = max_studentized_range_sf(18.5, 2, 12, 30.0).unwrap();
        assert!((p_lo - s_lo).abs() <= 1e-14_f64.max(1e-11 * s_lo));
        assert!((p_hi - s_hi).abs() <= 1e-14_f64.max(1e-11 * s_hi));
    }

    #[test]
    fn issue_82_batched_matches_scalar_across_edge_cases_and_parameter_regimes() {
        let qs_unsorted: &[f64] = &[
            f64::INFINITY,
            0.0,
            -2.5,
            f64::NEG_INFINITY,
            1e-9,
            0.15,
            1.2,
            2.8,
            4.5,
            4.5, // exact duplicate
            6.5,
            9.0,
            14.0,
            22.0,
            40.0,
            80.0,
            5_000.0,
            1e160,
        ];

        let configs: &[(usize, usize, f64)] = &[
            (1, 2, 1.5),
            (1, 2, 20.0),
            (1, 2, f64::INFINITY),
            (4, 2, 1.5),
            (4, 2, 18.0),
            (48, 2, 192.0),
            (6, 2, f64::INFINITY),
            (1, 3, 0.8),
            (1, 3, 1.99),
            (1, 3, 2.0),
            (1, 3, 2.01),
            (1, 3, 2.99),
            (1, 3, 3.0),
            (1, 3, 3.01),
            (1, 3, 10.0),
            (1, 3, 1e5),
            (1, 3, 1e8),
            (1, 3, f64::INFINITY),
            (2, 5, 12.7),
            (3, 10, 40.0),
            (2, 48, 192.0),
            (2, 48, f64::INFINITY),
            (1, 200, 60.0),
        ];

        for &(r, c, df) in configs {
            assert_batch_matches_scalar(qs_unsorted, r, c, df);
        }

        // Large group count c = 70_000 (verifying rigorous lower-saturation cutoff w_lo(c)).
        assert_batch_matches_scalar(&[0.0, 2.0, 5.0, 8.5, 11.0, 18.0, f64::INFINITY], 1, 70_000, 10.0);
    }

    fn assert_table_contiguous_and_bounded(table: &super::GaussianRangeTable) {
        assert!(!table.panels.is_empty(), "expected at least one panel");
        assert!(table.panels.len() <= super::CHEB_MAX_PANELS + 8, "unbounded panel growth: {}", table.panels.len());
        assert_eq!(table.panels.first().unwrap().a.to_bits(), table.w_lo.to_bits());
        assert_eq!(table.panels.last().unwrap().b.to_bits(), table.w_covered.to_bits());
        for pair in table.panels.windows(2) {
            assert_eq!(pair[0].b.to_bits(), pair[1].a.to_bits(), "panel gap between {:?} and {:?}", pair[0].b, pair[1].a);
        }
    }

    #[test]
    fn issue_82_incremental_extensions_tiny_steps_and_many_ascending_queries() {
        use super::{GAUSSIAN_RANGE_ZERO_W, GaussianRangeTable, PanelKind, RangePanel, StudentizedRangeCache, max_gaussian_range_sf};

        // (a) Tiny incremental extensions (<= 1e-9 steps) must never create uncovered gaps or
        //     drift `w_covered` past the last panel's upper bound.
        for &c in &[3_usize, 48_usize] {
            let mut tbl = GaussianRangeTable::new(c);
            let mut w = tbl.w_lo;
            for step in 1..=20 {
                w += 5e-10;
                tbl.ensure_up_to(w);
                assert_table_contiguous_and_bounded(&tbl);
                let got = tbl.eval_max_sf(w, 2);
                let exact = max_gaussian_range_sf(w, 2, c);
                let tol = 1e-14_f64.max(1e-11 * exact);
                assert!((got - exact).abs() <= tol, "tiny step {step} c={c} w={w}: got={got:.16e} vs exact={exact:.16e}");
            }
            // Also verify tiny steps right past the first canonical breakpoint.
            let boundary = tbl.w_covered;
            for step in 1..=10 {
                let w_probe = boundary + (step as f64) * 4e-10;
                tbl.ensure_up_to(w_probe);
                assert_table_contiguous_and_bounded(&tbl);
                let got = tbl.eval_max_sf(w_probe, 2);
                let exact = max_gaussian_range_sf(w_probe, 2, c);
                let tol = 1e-14_f64.max(1e-11 * exact);
                assert!((got - exact).abs() <= tol, "boundary step {step} c={c} w={w_probe}: got={got:.16e} vs exact={exact:.16e}");
            }
        }

        // (b) > 64 ascending cache.sf / cache.max_sf queries (as in Games-Howell / Dunnett T3 on
        //     ordered group means) crossing w_mid up to GAUSSIAN_RANGE_ZERO_W must neither exhaust
        //     CHEB_MAX_PANELS nor lose accuracy against scalar `max_studentized_range_sf`.
        for &(r, c, base_df) in &[(1_usize, 5_usize, 18.0_f64), (2, 48, 80.0), (10, 2, 15.0)] {
            let mut cache = StudentizedRangeCache::new();
            let n_steps = 96_usize;
            for step in 1..=n_steps {
                let frac = (step as f64) / (n_steps as f64);
                let q = 0.25 + frac * (GAUSSIAN_RANGE_ZERO_W + 4.0);
                // Vary df slightly across queries to exercise per-pair Welch-style calls too.
                let df = base_df + 0.15 * ((step % 11) as f64);
                let got = if r == 1 { cache.sf(q, c, df).unwrap() } else { cache.max_sf(q, r, c, df).unwrap() };
                // Cross-check scalar on a thorough representative stride (plus endpoints).
                if step % 8 == 0 || step == 1 || step == n_steps {
                    let scalar = max_studentized_range_sf(q, r, c, df).unwrap();
                    if scalar == 0.0 {
                        assert!(got <= 1e-300, "r={r} c={c} df={df} q={q}: got={got:e} vs scalar=0");
                    } else {
                        let tol = 1e-14_f64.max(1e-11 * scalar);
                        assert!((got - scalar).abs() <= tol, "r={r} c={c} df={df} q={q}: got={got:.16e} vs scalar={scalar:.16e}");
                    }
                }
            }
            let tbl = cache.tables.get(&c).unwrap();
            assert_table_contiguous_and_bounded(tbl);
            assert!(tbl.panels.len() <= super::CHEB_MAX_PANELS, "c={c}: unexpected panel count {}", tbl.panels.len());
        }

        // (c) Verify PanelKind::ExactFallback evaluates identically to `max_gaussian_range_sf`.
        let fb = RangePanel { a: 1.0, b: 10.0, kind: PanelKind::ExactFallback };
        assert_eq!(fb.eval_max_sf(4.25, 3, 8).to_bits(), max_gaussian_range_sf(4.25, 3, 8).to_bits());
    }
}
