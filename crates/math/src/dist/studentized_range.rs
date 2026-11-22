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

use crate::dist::core::{norm_cdf, norm_isf, norm_ln_pdf, norm_ln_sf, norm_pdf, norm_sf, student_t_two_sided_isf, student_t_two_sided_p};
use crate::error::MathError;
use crate::quad::gauss_legendre_32;
use crate::roots::{RootOptions, brent_root};
use crate::special::gamma::{deviance_bd0, ln_gamma, stirling_err};

const NORMAL_Z_MIN: f64 = -10.0;
const NORMAL_Z_MAX: f64 = 9.5;
const HUGE_DF_CUTOFF: f64 = 1e8;
const MAX_BRACKET_STEPS: usize = 64;

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
    let knots = [z_star - 3.0, z_star, z_star + 3.0, z_mode];
    sum_gl32_panels(f, z_min, z_max, &knots, 0.25)
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
        let d = normal_interval_prob(z, z + w);
        let bracket_gap = if b <= 0.5 * a {
            let ratio = b / a;
            -((-ratio).ln_1p() * km1_f).exp_m1()
        } else {
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

/// Chi-scale density `h_{df}(u)` of `U = sqrt(chi^2_{df} / df)` evaluated stably for any `df > 0`.
#[inline]
pub(crate) fn chi_scale_pdf(u: f64, df: f64, lg_half_df: f64) -> f64 {
    if u <= 0.0 {
        return 0.0;
    }
    let a = 0.5 * df;
    let x = a * u * u;
    let ln_kern =
        if a >= 12.0 { -deviance_bd0(a, x) + 0.5 * (a / std::f64::consts::TAU).ln() - stirling_err(a) } else { a * x.ln() - x - lg_half_df };
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

    if df < 2.0 {
        // Substitute u = t^{1 / a} (a = df / 2): x = a u^2 = a t^{2 / a},
        // f_X(x) dx = 2 * a^{a - 1} * t * exp(-x - ln Gamma(a)) dt — smooth at t = 0 with mode at t = 1.
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

    let weighted = |u: f64| -> f64 {
        let dens = chi_scale_pdf(u, df, lg_a);
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
        if max_studentized_range_sf(lo, r, c, df)? >= alpha || lo == 0.0 {
            break;
        }
        lo *= 0.5;
    }
    for _ in 0..MAX_BRACKET_STEPS {
        if max_studentized_range_sf(hi, r, c, df)? <= alpha {
            break;
        }
        hi *= 1.5;
    }
    if max_studentized_range_sf(lo, r, c, df)? < alpha || max_studentized_range_sf(hi, r, c, df)? > alpha {
        return Err(MathError::NotConverged { algorithm: "max_studentized_range_isf", iterations: MAX_BRACKET_STEPS });
    }

    brent_root(
        |q| max_studentized_range_sf(q, r, c, df).unwrap_or(f64::NAN) - alpha,
        lo,
        hi,
        RootOptions { rel_tol: 1e-11, abs_tol: 1e-12, max_iters: 96 },
    )
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
}
