//! `2 x 2` effect ratios and score/exact confidence intervals: Baptista–Pike conditional odds-ratio
//! CI, Koopman score relative-risk CI, Newcombe risk-difference CI, Altman Number Needed to Treat
//! (`NNTB`/`NNTH`), and Wilson diagnostic sensitivity/specificity/PPV/NPV intervals.
//!
//! References:
//! - Baptista, J., & Pike, M. C. (1977). Algorithm AS 115: Exact two-sided confidence limits for the
//!   odds ratio in a 2 x 2 table. *Journal of the Royal Statistical Society C*, 26(2), 214-220.
//! - Koopman, P. A. R. (1984). Confidence intervals for the ratio of two binomial proportions.
//!   *Biometrics*, 40(2), 513-517.
//! - Newcombe, R. G. (1998). Interval estimation for the difference between independent proportions:
//!   Comparison of eleven methods. *Statistics in Medicine*, 17(8), 873-890.
//! - Altman, D. G. (1998). Confidence intervals for the number needed to treat. *BMJ*, 317(7168),
//!   1309-1312.
//! - Wilson, E. B. (1927). Probable inference, the law of succession, and statistical inference.
//!   *Journal of the American Statistical Association*, 22(158), 209-212.

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_two_sided_isf;
use curvebook_math::roots::{RootOptions, brent_root};
use curvebook_math::special::{digamma, ln_factorial, ln_gamma};
use curvebook_math::sum::NeumaierSum;

use super::binomial::wilson_score_ci;
use super::types::{NntKind, TwoByTwoRatiosResult};
use crate::error::StatsError;

const NONCENTRAL_HYPERGEOM_MAX_TOTAL: u64 = 5_000_000;
const NONCENTRAL_HYPERGEOM_MAX_SPAN: u64 = 100_000;
const NONCENTRAL_HYPERGEOM_CUTOFF_LOG: f64 = -65.0;
const NONCENTRAL_HYPERGEOM_EXACT_WINDOW_MAX: u64 = 4_096;
const NONCENTRAL_HYPERGEOM_STIRLING_MIN: f64 = 20.0;
const NONCENTRAL_HYPERGEOM_EM_MIN_CELL: u64 = 100;

const GL8_NODES: [f64; 8] = [
    -0.960_289_856_497_536_3,
    -0.796_666_477_413_626_7,
    -0.525_532_409_916_329,
    -0.183_434_642_495_649_8,
    0.183_434_642_495_649_8,
    0.525_532_409_916_329,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];
const GL8_WEIGHTS: [f64; 8] = [
    0.101_228_536_290_376_3,
    0.222_381_034_453_374_5,
    0.313_706_645_877_887_3,
    0.362_683_783_378_362,
    0.362_683_783_378_362,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];

#[derive(Clone, Copy)]
struct HypergeomMargins {
    r1: u64,
    r2: u64,
    c1: u64,
}

#[derive(Clone, Copy)]
struct NoncentralHypergeomCtx {
    a_obs: u64,
    margins: HypergeomMargins,
    lo: u64,
    hi: u64,
    span: u64,
    windowed: bool,
}

/// Noncentral hypergeometric exact two-sided *p*-value as a function of odds ratio `or_val`.
#[cfg(test)]
fn noncentral_hypergeom_p(a_obs: u64, r1: u64, r2: u64, c1: u64, or_val: f64) -> Result<f64, StatsError> {
    let total = r1.checked_add(r2).ok_or(StatsError::InvalidParameter { name: "noncentral_hypergeom_total", value: f64::INFINITY })?;
    if total > NONCENTRAL_HYPERGEOM_MAX_TOTAL {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_max_total", value: total as f64 });
    }
    if c1 > total {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_col1", value: c1 as f64 });
    }
    let lo = c1.saturating_sub(r2);
    let hi = r1.min(c1);
    if lo > hi || a_obs < lo || a_obs > hi {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_cell", value: a_obs as f64 });
    }
    if !or_val.is_finite() || or_val < 0.0 {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_or", value: or_val });
    }
    let span = hi - lo + 1;
    if span > NONCENTRAL_HYPERGEOM_MAX_SPAN {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_max_span", value: span as f64 });
    }
    let span_usize = usize::try_from(span).map_err(|_| StatsError::InvalidParameter { name: "noncentral_hypergeom_max_span", value: span as f64 })?;
    if or_val == 0.0 {
        return Ok(if a_obs == lo { 1.0 } else { 0.0 });
    }
    let ctx = NoncentralHypergeomCtx { a_obs, margins: HypergeomMargins { r1, r2, c1 }, lo, hi, span, windowed: false };
    let (p_sum, _) = noncentral_hypergeom_p_and_mean_log_or(ctx, span_usize, or_val.ln())?;
    Ok(p_sum)
}

fn noncentral_hypergeom_p_and_mean_log_or(ctx: NoncentralHypergeomCtx, span_usize: usize, log_or: f64) -> Result<(f64, f64), StatsError> {
    let NoncentralHypergeomCtx { a_obs, margins: HypergeomMargins { r1, r2, c1 }, lo, hi, .. } = ctx;
    if !log_or.is_finite() {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_or", value: log_or });
    }
    let mut log_weights = Vec::with_capacity(span_usize);
    let mut max_lw = f64::NEG_INFINITY;
    let mut max_abs_lw = 0.0_f64;
    let mut obs_lw = f64::NEG_INFINITY;
    for x in lo..=hi {
        let bx = r1 - x;
        let cx = c1 - x;
        let dx = r2 - cx;
        let lw = (x as f64) * log_or - (ln_factorial(x) + ln_factorial(bx) + ln_factorial(cx) + ln_factorial(dx));
        max_lw = max_lw.max(lw);
        max_abs_lw = max_abs_lw.max(lw.abs());
        if x == a_obs {
            obs_lw = lw;
        }
        log_weights.push((x, lw));
    }
    let log_rel_tol = (1.0e-12_f64).max(16.0 * max_abs_lw * f64::EPSILON);
    let thresh = obs_lw + log_rel_tol;
    let mut norm = NeumaierSum::new();
    let mut mean_num = NeumaierSum::new();
    let mut tail_sum = NeumaierSum::new();
    for (x, lw) in log_weights {
        let w = (lw - max_lw).exp();
        norm.add(w);
        mean_num.add((x as f64) * w);
        if lw <= thresh {
            tail_sum.add(w);
        }
    }
    let norm_total = norm.total();
    if !norm_total.is_finite() || norm_total <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_norm", value: norm_total });
    }
    Ok(((tail_sum.total() / norm_total).clamp(0.0, 1.0), mean_num.total() / norm_total))
}

#[inline]
fn ln1p_minus_x(r: f64) -> f64 {
    if r.abs() < 1e-2 {
        let mut term = -r * r;
        let mut acc = 0.0_f64;
        for k in 2..=16 {
            acc += term / f64::from(k);
            term *= -r;
            if term.abs() <= 1e-18 * acc.abs() {
                break;
            }
        }
        acc
    } else {
        r.ln_1p() - r
    }
}

#[inline]
fn stirling_rem_diff(z: f64, t: f64) -> f64 {
    let zp = z + t;
    let inv_z = 1.0 / z;
    let inv_zp = 1.0 / zp;
    let d1 = -t / (12.0 * z * zp);
    let d3 = t * (inv_zp * inv_zp + inv_zp * inv_z + inv_z * inv_z) / (360.0 * z * zp);
    let d5 = (inv_zp.powi(5) - inv_z.powi(5)) / 1260.0;
    d1 + d3 + d5
}

/// Cancellation-free `ln Gamma(z + 1 + t) - ln Gamma(z + 1) - t * ln(z + 1)`.
fn ln_factorial_centered_diff(z: f64, t: f64) -> Result<f64, StatsError> {
    let z1 = z + 1.0;
    if z >= NONCENTRAL_HYPERGEOM_STIRLING_MIN && z + t >= NONCENTRAL_HYPERGEOM_STIRLING_MIN {
        return Ok((z1 + t - 0.5) * ln1p_minus_x(t / z1) + t * (t - 0.5) / z1 + stirling_rem_diff(z1, t));
    }
    Ok((ln_gamma(z1 + t)? - ln_gamma(z1)?) - t * z1.ln())
}

#[inline]
fn noncentral_hypergeom_log_step(x: u64, margins: HypergeomMargins, log_or: f64) -> f64 {
    let HypergeomMargins { r1, r2, c1 } = margins;
    let num = ((r1 - x) as f64).ln() + ((c1 - x) as f64).ln();
    let den = ((x + 1) as f64).ln() + ((r2 - (c1 - x) + 1) as f64).ln();
    log_or + num - den
}

fn find_noncentral_hypergeom_mode(margins: HypergeomMargins, lo: u64, hi: u64, log_or: f64) -> u64 {
    if lo >= hi {
        return lo;
    }
    if noncentral_hypergeom_log_step(lo, margins, log_or) <= 0.0 {
        return lo;
    }
    if noncentral_hypergeom_log_step(hi - 1, margins, log_or) >= 0.0 {
        return hi;
    }
    let mut l = lo;
    let mut r = hi - 1;
    while l < r {
        let mid = l + (r - l) / 2;
        if noncentral_hypergeom_log_step(mid, margins, log_or) >= 0.0 {
            l = mid + 1;
        } else {
            r = mid;
        }
    }
    l
}

fn noncentral_hypergeom_lw_diff(xm: u64, delta: f64, margins: HypergeomMargins, log_or: f64) -> Result<f64, StatsError> {
    if delta == 0.0 {
        return Ok(0.0);
    }
    let HypergeomMargins { r1, r2, c1 } = margins;
    let bm = (r1 - xm) as f64;
    let cm = (c1 - xm) as f64;
    let dm = (r2 - (c1 - xm)) as f64;
    let xmf = xm as f64;
    let s0 = log_or + ((bm + 1.0) / (xmf + 1.0)).ln() + ((cm + 1.0) / (dm + 1.0)).ln();
    let c_diff = ln_factorial_centered_diff(xmf, delta)?
        + ln_factorial_centered_diff(bm, -delta)?
        + ln_factorial_centered_diff(cm, -delta)?
        + ln_factorial_centered_diff(dm, delta)?;
    Ok(delta * s0 - c_diff)
}

#[inline]
fn noncentral_hypergeom_delta_u64(x: u64, xm: u64) -> f64 {
    if x >= xm { (x - xm) as f64 } else { -((xm - x) as f64) }
}

#[inline]
fn noncentral_hypergeom_min_cell(x: u64, margins: HypergeomMargins) -> u64 {
    let HypergeomMargins { r1, r2, c1 } = margins;
    x.min(r1 - x).min(c1 - x).min(r2 - (c1 - x))
}

fn noncentral_hypergeom_lprime(x: f64, margins: HypergeomMargins, log_or: f64) -> Result<f64, StatsError> {
    let HypergeomMargins { r1, r2, c1 } = margins;
    Ok(log_or - digamma(x + 1.0)? + digamma((r1 as f64) - x + 1.0)? + digamma((c1 as f64) - x + 1.0)? - digamma((r2 as f64) - (c1 as f64) + x + 1.0)?)
}

fn sum_noncentral_hypergeom_range(
    a_start: u64,
    b_end: u64,
    xm: u64,
    margins: HypergeomMargins,
    log_or: f64,
    need_mean: bool,
) -> Result<(f64, f64), StatsError> {
    if a_start > b_end {
        return Ok((0.0, 0.0));
    }
    if b_end - a_start <= NONCENTRAL_HYPERGEOM_EXACT_WINDOW_MAX {
        let mut s0 = 0.0_f64;
        let mut s1 = 0.0_f64;
        for x in a_start..=b_end {
            let dx = noncentral_hypergeom_delta_u64(x, xm);
            let w = noncentral_hypergeom_lw_diff(xm, dx, margins, log_or)?.exp();
            s0 += w;
            if need_mean {
                s1 += dx * w;
            }
        }
        return Ok((s0, s1));
    }
    if noncentral_hypergeom_min_cell(a_start, margins) < NONCENTRAL_HYPERGEOM_EM_MIN_CELL {
        let split = a_start + NONCENTRAL_HYPERGEOM_EM_MIN_CELL + 28;
        let (s0a, s1a) = sum_noncentral_hypergeom_range(a_start, split, xm, margins, log_or, need_mean)?;
        let (s0b, s1b) = sum_noncentral_hypergeom_range(split + 1, b_end, xm, margins, log_or, need_mean)?;
        return Ok((s0a + s0b, s1a + s1b));
    }
    if noncentral_hypergeom_min_cell(b_end, margins) < NONCENTRAL_HYPERGEOM_EM_MIN_CELL {
        let split = b_end - NONCENTRAL_HYPERGEOM_EM_MIN_CELL - 28;
        let (s0a, s1a) = sum_noncentral_hypergeom_range(a_start, split - 1, xm, margins, log_or, need_mean)?;
        let (s0b, s1b) = sum_noncentral_hypergeom_range(split, b_end, xm, margins, log_or, need_mean)?;
        return Ok((s0a + s0b, s1a + s1b));
    }
    if a_start < xm && xm < b_end {
        let (s0a, s1a) = sum_noncentral_hypergeom_range(a_start, xm, xm, margins, log_or, need_mean)?;
        let (s0b, s1b) = sum_noncentral_hypergeom_range(xm + 1, b_end, xm, margins, log_or, need_mean)?;
        return Ok((s0a + s0b, s1a + s1b));
    }

    // Smooth unimodal window with interior margins >= 100: composite 32-panel 8-point Gauss-Legendre + Euler-Maclaurin.
    let panels = 32_usize;
    let af = a_start as f64;
    let bf = b_end as f64;
    let xmf = xm as f64;
    let step = (bf - af) / (panels as f64);
    let mut t0 = 0.0_f64;
    let mut t1 = 0.0_f64;
    for k in 0..panels {
        let x0 = af + (k as f64) * step;
        let x1 = if k + 1 < panels { af + ((k + 1) as f64) * step } else { bf };
        let mid = 0.5 * (x0 + x1);
        let half = 0.5 * (x1 - x0);
        for (&nd, &wt) in GL8_NODES.iter().zip(GL8_WEIGHTS.iter()) {
            let xv = mid + half * nd;
            let dx = xv - xmf;
            let fv = noncentral_hypergeom_lw_diff(xm, dx, margins, log_or)?.exp();
            t0 += half * wt * fv;
            if need_mean {
                t1 += half * wt * dx * fv;
            }
        }
    }
    let dxa = noncentral_hypergeom_delta_u64(a_start, xm);
    let dxb = noncentral_hypergeom_delta_u64(b_end, xm);
    let fa = noncentral_hypergeom_lw_diff(xm, dxa, margins, log_or)?.exp();
    let fb = noncentral_hypergeom_lw_diff(xm, dxb, margins, log_or)?.exp();
    let lpa = noncentral_hypergeom_lprime(af, margins, log_or)?;
    let lpb = noncentral_hypergeom_lprime(bf, margins, log_or)?;
    let s0 = t0 + 0.5 * (fa + fb) + (fb * lpb - fa * lpa) / 12.0;
    if !need_mean {
        return Ok((s0, 0.0));
    }
    let ga = dxa * fa;
    let gb = dxb * fb;
    let gpa = fa + dxa * fa * lpa;
    let gpb = fb + dxb * fb * lpb;
    let s1 = t1 + 0.5 * (ga + gb) + (gpb - gpa) / 12.0;
    Ok((s0, s1))
}

fn noncentral_hypergeom_p_and_mean_windowed(ctx: NoncentralHypergeomCtx, log_or: f64, need_mean: bool) -> Result<(f64, f64), StatsError> {
    let NoncentralHypergeomCtx { a_obs, margins, lo, hi, .. } = ctx;
    if !log_or.is_finite() {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_or", value: log_or });
    }
    let xm = find_noncentral_hypergeom_mode(margins, lo, hi, log_or);
    let w_lo = if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(lo, xm), margins, log_or)? >= NONCENTRAL_HYPERGEOM_CUTOFF_LOG {
        lo
    } else {
        let mut l = lo;
        let mut r = xm;
        while l < r {
            let mid = l + (r - l) / 2;
            if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(mid, xm), margins, log_or)? >= NONCENTRAL_HYPERGEOM_CUTOFF_LOG {
                r = mid;
            } else {
                l = mid + 1;
            }
        }
        l
    };
    let w_hi = if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(hi, xm), margins, log_or)? >= NONCENTRAL_HYPERGEOM_CUTOFF_LOG {
        hi
    } else {
        let mut l = xm;
        let mut r = hi;
        while l < r {
            let mid = l + (r - l).div_ceil(2);
            if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(mid, xm), margins, log_or)? >= NONCENTRAL_HYPERGEOM_CUTOFF_LOG {
                l = mid;
            } else {
                r = mid - 1;
            }
        }
        l
    };

    let (norm, mean_num) = sum_noncentral_hypergeom_range(w_lo, w_hi, xm, margins, log_or, need_mean)?;
    if !norm.is_finite() || norm <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_norm", value: norm });
    }
    let mean_val = (xm as f64) + mean_num / norm;
    let la = noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(a_obs, xm), margins, log_or)?;
    if la >= -1e-12 {
        return Ok((1.0, mean_val));
    }
    if la < NONCENTRAL_HYPERGEOM_CUTOFF_LOG {
        return Ok((0.0, mean_val));
    }
    let thresh = la + 1e-12;
    if w_hi - w_lo <= NONCENTRAL_HYPERGEOM_EXACT_WINDOW_MAX {
        let mut tail = 0.0_f64;
        for x in w_lo..=w_hi {
            let lwv = noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(x, xm), margins, log_or)?;
            if lwv <= thresh {
                tail += lwv.exp();
            }
        }
        return Ok(((tail / norm).clamp(0.0, 1.0), mean_val));
    }

    let (t_left, t_right) = if a_obs < xm {
        let mut l = a_obs;
        let mut r = xm;
        while l < r {
            let mid = l + (r - l).div_ceil(2);
            if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(mid, xm), margins, log_or)? <= thresh {
                l = mid;
            } else {
                r = mid - 1;
            }
        }
        let x_left = l;
        let (tl, _) = sum_noncentral_hypergeom_range(w_lo, x_left, xm, margins, log_or, false)?;
        let tr = if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(w_hi, xm), margins, log_or)? > thresh {
            0.0
        } else {
            let mut rl = xm;
            let mut rr = w_hi;
            while rl < rr {
                let mid = rl + (rr - rl) / 2;
                if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(mid, xm), margins, log_or)? <= thresh {
                    rr = mid;
                } else {
                    rl = mid + 1;
                }
            }
            sum_noncentral_hypergeom_range(rl, w_hi, xm, margins, log_or, false)?.0
        };
        (tl, tr)
    } else {
        let mut rl = xm;
        let mut rr = a_obs;
        while rl < rr {
            let mid = rl + (rr - rl) / 2;
            if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(mid, xm), margins, log_or)? <= thresh {
                rr = mid;
            } else {
                rl = mid + 1;
            }
        }
        let x_right = rl;
        let (tr, _) = sum_noncentral_hypergeom_range(x_right, w_hi, xm, margins, log_or, false)?;
        let tl = if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(w_lo, xm), margins, log_or)? > thresh {
            0.0
        } else {
            let mut ll = w_lo;
            let mut lr = xm;
            while ll < lr {
                let mid = ll + (lr - ll).div_ceil(2);
                if noncentral_hypergeom_lw_diff(xm, noncentral_hypergeom_delta_u64(mid, xm), margins, log_or)? <= thresh {
                    ll = mid;
                } else {
                    lr = mid - 1;
                }
            }
            sum_noncentral_hypergeom_range(w_lo, ll, xm, margins, log_or, false)?.0
        };
        (tl, tr)
    };
    Ok((((t_left + t_right) / norm).clamp(0.0, 1.0), mean_val))
}

fn eval_noncentral_hypergeom(ctx: NoncentralHypergeomCtx, log_or: f64, need_mean: bool) -> Result<(f64, f64), StatsError> {
    if ctx.windowed {
        noncentral_hypergeom_p_and_mean_windowed(ctx, log_or, need_mean)
    } else {
        let span_usize =
            usize::try_from(ctx.span).map_err(|_| StatsError::InvalidParameter { name: "noncentral_hypergeom_max_span", value: ctx.span as f64 })?;
        noncentral_hypergeom_p_and_mean_log_or(ctx, span_usize, log_or)
    }
}

fn baptista_pike_anchor_log_or(ctx: NoncentralHypergeomCtx, alpha: f64, or_pt: f64) -> Option<f64> {
    if or_pt.is_finite() && or_pt > 0.0 {
        let u_try = or_pt.ln();
        if let Ok((p, _)) = eval_noncentral_hypergeom(ctx, u_try, false)
            && p >= alpha
        {
            return Some(u_try);
        }
    }
    // Locate the conditional MLE log-OR where E[X | e^u] == a_obs; by discrete log-concavity,
    // an integer mean equals the mode so p(u_mle) == 1.0 >= alpha.
    let target = ctx.a_obs as f64;
    let mut u_lo = -40.0_f64;
    let mut u_hi = 40.0_f64;
    while u_lo > -640.0 {
        let (_, m) = eval_noncentral_hypergeom(ctx, u_lo, true).ok()?;
        if m <= target {
            break;
        }
        u_lo *= 2.0;
    }
    while u_hi < 640.0 {
        let (_, m) = eval_noncentral_hypergeom(ctx, u_hi, true).ok()?;
        if m >= target {
            break;
        }
        u_hi *= 2.0;
    }
    let mut best_u = 0.5 * (u_lo + u_hi);
    for _ in 0..64 {
        let mid = 0.5 * (u_lo + u_hi);
        best_u = mid;
        let (p, m) = eval_noncentral_hypergeom(ctx, mid, true).ok()?;
        if (m - target).abs() <= 1e-12 && p >= alpha {
            return Some(mid);
        }
        if m < target {
            u_lo = mid;
        } else {
            u_hi = mid;
        }
    }
    let (p_best, _) = eval_noncentral_hypergeom(ctx, best_u, false).ok()?;
    (p_best >= alpha).then_some(best_u)
}

fn baptista_pike_solve_side(ctx: NoncentralHypergeomCtx, alpha: f64, u_center: f64, lower_branch: bool) -> Option<f64> {
    let a_obs = ctx.a_obs;
    let b_obs = ctx.margins.r1 - a_obs;
    let c_obs = ctx.margins.c1 - a_obs;
    let d_obs = ctx.margins.r2 - c_obs;
    let sigma_u = (1.0 / (a_obs.max(1) as f64) + 1.0 / (b_obs.max(1) as f64) + 1.0 / (c_obs.max(1) as f64) + 1.0 / (d_obs.max(1) as f64)).sqrt();
    let mut u_in = u_center;
    let mut step = (4.0 * sigma_u).clamp(1e-7, 1.0);
    let mut bracket = None;
    for _ in 0..48 {
        let u_out = if lower_branch { (u_center - step).max(-700.0) } else { (u_center + step).min(700.0) };
        if u_out == u_in {
            break;
        }
        let (p_out, _) = eval_noncentral_hypergeom(ctx, u_out, false).ok()?;
        if p_out < alpha {
            bracket = Some(if lower_branch { (u_out, u_in) } else { (u_in, u_out) });
            break;
        }
        u_in = u_out;
        if u_out.abs() >= 700.0 {
            break;
        }
        step *= 2.0;
    }
    let (u_a, u_b) = bracket?;
    let abs_tol = (1e-11 * sigma_u.min(1.0)).max(1e-15);
    let u_root = brent_root(
        |u| match eval_noncentral_hypergeom(ctx, u, false) {
            Ok((p, _)) => p - alpha,
            Err(_) => f64::NAN,
        },
        u_a,
        u_b,
        RootOptions { rel_tol: 1e-10, abs_tol, max_iters: 100 },
    )
    .ok()?;
    let val = u_root.exp();
    (val.is_finite() && val >= 0.0).then_some(val)
}

/// Baptista–Pike (1977) exact conditional odds-ratio confidence interval for `2 x 2` table `[[a, b], [c, d]]`.
///
/// Uses full log-space summation for `total <= 5_000_000` and `span <= 100_000`, and automatically
/// switches to mode-windowed log-space summation (with cancellation-free Stirling differences and
/// composite Gauss–Legendre / Euler–Maclaurin summation on wide smooth windows) for larger tables.
pub fn baptista_pike_or_ci(a: u64, b: u64, c: u64, d: u64, confidence: ConfidenceLevel) -> Option<(f64, f64)> {
    let r1 = a.checked_add(b)?;
    let r2 = c.checked_add(d)?;
    let c1 = a.checked_add(c)?;
    let total = r1.checked_add(r2)?;
    let lo_x = c1.saturating_sub(r2);
    let hi_x = r1.min(c1);
    let span = hi_x.checked_sub(lo_x).and_then(|v| v.checked_add(1))?;
    if r1 == 0 || r2 == 0 || lo_x >= hi_x {
        return None;
    }
    let windowed = total > NONCENTRAL_HYPERGEOM_MAX_TOTAL || span > NONCENTRAL_HYPERGEOM_MAX_SPAN;
    let alpha = confidence.alpha();
    // Point OR estimate with Haldane correction only as an initial hint for locating an interior point.
    let or_pt = if b == 0 || c == 0 {
        ((a as f64 + 0.5) * (d as f64 + 0.5)) / ((b as f64 + 0.5) * (c as f64 + 0.5))
    } else if a == 0 || d == 0 {
        0.5 / (((b as f64) * (c as f64)).max(1.0))
    } else {
        ((a as f64) * (d as f64)) / ((b as f64) * (c as f64))
    };

    let ctx = NoncentralHypergeomCtx { a_obs: a, margins: HypergeomMargins { r1, r2, c1 }, lo: lo_x, hi: hi_x, span, windowed };
    let u_center = baptista_pike_anchor_log_or(ctx, alpha, or_pt)?;
    let lo_bound = if a <= lo_x { 0.0 } else { baptista_pike_solve_side(ctx, alpha, u_center, true)? };
    let hi_bound = if a >= hi_x { f64::INFINITY } else { baptista_pike_solve_side(ctx, alpha, u_center, false)? };
    if lo_bound > hi_bound {
        return None;
    }

    Some((lo_bound, hi_bound))
}

/// Koopman (1984) asymptotic score confidence interval for relative risk `RR = (a/n1) / (c/n2)`.
pub fn koopman_rr_ci(a: u64, b: u64, c: u64, d: u64, confidence: ConfidenceLevel) -> Option<(f64, f64)> {
    let n1 = (u128::from(a) + u128::from(b)) as f64;
    let n2 = (u128::from(c) + u128::from(d)) as f64;
    if n1 <= 0.0 || n2 <= 0.0 || (a == 0 && c == 0) {
        return None;
    }
    let z2 = norm_two_sided_isf(confidence.alpha()).ok()?.powi(2);
    let score = |phi: f64| koopman_chi2_score(a, b, c, d, z2, phi);

    let rr_pt = if c == 0 { f64::INFINITY } else { ((a as f64) / n1) / ((c as f64) / n2) };

    let lo = if a == 0 {
        0.0
    } else if c == 0 {
        koopman_solve_lower_c_zero(&score)?
    } else {
        koopman_solve_lower(&score, rr_pt)?
    };

    let hi = if c == 0 {
        f64::INFINITY
    } else if a == 0 {
        koopman_solve_upper_a_zero(&score)?
    } else {
        koopman_solve_upper(&score, rr_pt)?
    };

    Some((lo, hi))
}

fn koopman_chi2_score(a: u64, b: u64, c: u64, d: u64, z2: f64, phi: f64) -> f64 {
    if !phi.is_finite() || phi <= 0.0 {
        return f64::INFINITY;
    }
    let x1 = a as f64;
    let x2 = c as f64;
    let n1 = (u128::from(a) + u128::from(b)) as f64;
    let n2 = (u128::from(c) + u128::from(d)) as f64;
    let aq = n1 + n2;
    let abs_b = phi * (n1 + x2) + (x1 + n2); // = -b_q > 0
    let cq = phi * (x1 + x2);
    let disc = if abs_b > 1e150 {
        let r1 = (4.0 * aq) / abs_b;
        let r2 = cq / abs_b;
        abs_b * (1.0 - r1 * r2).max(0.0).sqrt()
    } else {
        (abs_b * abs_b - 4.0 * aq * cq).max(0.0).sqrt()
    };
    // Quadratic solves for constrained MLE p1_tilde (= phi * p2_tilde); Citardauq's formula avoids cancellation.
    let denom = abs_b + disc;
    let p1_raw = (2.0 * cq) / denom;
    let p2_raw = (2.0 * (x1 + x2)) / denom;

    koopman_cell_term(a, b, n1, x1, p1_raw) + koopman_cell_term(c, d, n2, x2, p2_raw) - z2
}

fn koopman_cell_term(events: u64, non_events: u64, n: f64, x: f64, p_raw: f64) -> f64 {
    if events == 0 {
        n * p_raw / (1.0 - p_raw).max(1e-14)
    } else if non_events == 0 {
        n * (1.0 - p_raw).max(0.0) / p_raw.max(1e-100)
    } else {
        let p = p_raw.clamp(1e-100, 1.0 - 1e-15);
        (x - n * p).powi(2) / (n * p * (1.0 - p))
    }
}

const KOOPMAN_BRACKET_STEPS: usize = 120;
const KOOPMAN_ROOT_OPTIONS: RootOptions = RootOptions { rel_tol: 1e-10, abs_tol: 1e-30, max_iters: 128 };

fn koopman_find_negative_score_point<F: Fn(f64) -> f64>(score: &F, scale: f64) -> Option<(f64, f64)> {
    let mut pt = 1.0_f64;
    for _ in 0..KOOPMAN_BRACKET_STEPS {
        if !pt.is_finite() || pt <= 0.0 {
            return None;
        }
        let val = score(pt);
        if !val.is_finite() {
            return None;
        }
        if val <= 0.0 {
            return Some((pt, val));
        }
        pt *= scale;
    }
    None
}

fn koopman_bracket_and_solve<F: Fn(f64) -> f64>(score: &F, neg_start: f64, s_neg_start: f64, scale: f64, lower_branch: bool) -> Option<f64> {
    if !neg_start.is_finite() || neg_start <= 0.0 || !s_neg_start.is_finite() || s_neg_start > 0.0 {
        return None;
    }
    if s_neg_start == 0.0 {
        return Some(neg_start);
    }
    let mut neg_pt = neg_start;
    let mut pos_pt = neg_start * scale;
    let mut bracketed = false;
    for _ in 0..KOOPMAN_BRACKET_STEPS {
        if !pos_pt.is_finite() || pos_pt <= 0.0 {
            return None;
        }
        let s_pos = score(pos_pt);
        if !s_pos.is_finite() {
            return None;
        }
        if s_pos >= 0.0 {
            bracketed = true;
            break;
        }
        neg_pt = pos_pt;
        pos_pt *= scale;
    }
    if !bracketed {
        return None;
    }
    let (lo, hi) = if lower_branch { (pos_pt, neg_pt) } else { (neg_pt, pos_pt) };
    brent_root(score, lo, hi, KOOPMAN_ROOT_OPTIONS).ok()
}

fn koopman_solve_lower<F: Fn(f64) -> f64>(score: &F, rr_pt: f64) -> Option<f64> {
    if !rr_pt.is_finite() || rr_pt <= 0.0 {
        return None;
    }
    let s_neg = score(rr_pt);
    koopman_bracket_and_solve(score, rr_pt, s_neg, 0.5, true)
}

fn koopman_solve_upper<F: Fn(f64) -> f64>(score: &F, rr_pt: f64) -> Option<f64> {
    if !rr_pt.is_finite() || rr_pt <= 0.0 {
        return None;
    }
    let s_neg = score(rr_pt);
    koopman_bracket_and_solve(score, rr_pt, s_neg, 2.0, false)
}

fn koopman_solve_lower_c_zero<F: Fn(f64) -> f64>(score: &F) -> Option<f64> {
    let (neg_pt, s_neg) = koopman_find_negative_score_point(score, 2.0)?;
    koopman_bracket_and_solve(score, neg_pt, s_neg, 0.5, true)
}

fn koopman_solve_upper_a_zero<F: Fn(f64) -> f64>(score: &F) -> Option<f64> {
    let (neg_pt, s_neg) = koopman_find_negative_score_point(score, 0.5)?;
    koopman_bracket_and_solve(score, neg_pt, s_neg, 2.0, false)
}

/// Compute Altman (1998) `NNTB` / `NNTH` point summary, CI label, and finite CI bounds (when CI
/// excludes zero) from risk difference `rd` and its CI `(rd_low, rd_high)`.
#[must_use]
pub fn altman_nnt_from_rd(rd: f64, rd_low: f64, rd_high: f64) -> (NntKind, Option<f64>, String, String, Option<(f64, f64)>) {
    const EPS: f64 = 1e-15;
    let (kind, value, point_label) = if rd > EPS {
        let v = 1.0 / rd;
        (NntKind::Benefit, Some(v), format!("NNTB {v:.2}"))
    } else if rd < -EPS {
        let v = 1.0 / rd.abs();
        (NntKind::Harm, Some(v), format!("NNTH {v:.2}"))
    } else {
        (NntKind::Infinite, None, String::from("∞"))
    };

    let (ci_label, ci_bounds) = if rd_low > EPS {
        let lo = 1.0 / rd_high;
        let hi = 1.0 / rd_low;
        (format!("NNTB {lo:.2} to {hi:.2}"), Some((lo, hi)))
    } else if rd_high < -EPS {
        let lo = 1.0 / rd_lo_abs(rd_low);
        let hi = 1.0 / rd_lo_abs(rd_high);
        (format!("NNTH {lo:.2} to {hi:.2}"), Some((lo, hi)))
    } else {
        let harm_str = if rd_low < -EPS { format!("{:.2}", 1.0 / rd_low.abs()) } else { String::from("∞") };
        let benefit_str = if rd_high > EPS { format!("{:.2}", 1.0 / rd_high) } else { String::from("∞") };
        (format!("NNTH {harm_str} to ∞ to NNTB {benefit_str}"), None)
    };

    (kind, value, point_label, ci_label, ci_bounds)
}

#[inline]
fn rd_lo_abs(v: f64) -> f64 {
    v.abs().max(1e-300)
}

/// Compute complete `2 x 2` risk/odds ratios (`[[a, b], [c, d]]`), Baptista-Pike OR CI, Koopman
/// RR CI, Newcombe risk-difference CI, NNT, and Wilson CIs for sensitivity/specificity/PPV/NPV.
///
/// Undefined `0 / 0` quantities (including zero-margin odds ratio, zero-risk relative risk, and
/// zero-column sensitivity or specificity with its Wilson interval) are returned as `None`.
pub fn analyze_2x2_ratios(a: u64, b: u64, c: u64, d: u64, confidence: ConfidenceLevel) -> Result<TwoByTwoRatiosResult, StatsError> {
    let r1 = a.checked_add(b).ok_or(StatsError::InvalidParameter { name: "contingency_row_sum", value: f64::INFINITY })?;
    let r2 = c.checked_add(d).ok_or(StatsError::InvalidParameter { name: "contingency_row_sum", value: f64::INFINITY })?;
    let c1 = a.checked_add(c).ok_or(StatsError::InvalidParameter { name: "contingency_col_sum", value: f64::INFINITY })?;
    let c2 = b.checked_add(d).ok_or(StatsError::InvalidParameter { name: "contingency_col_sum", value: f64::INFINITY })?;
    let _total = r1.checked_add(r2).ok_or(StatsError::InvalidParameter { name: "contingency_total", value: f64::INFINITY })?;
    if r1 == 0 || r2 == 0 {
        return Err(StatsError::EmptyInput);
    }
    let p1 = (a as f64) / (r1 as f64);
    let p2 = (c as f64) / (r2 as f64);

    let or_val = if b > 0 && c > 0 {
        Some(((a as f64) * (d as f64)) / ((b as f64) * (c as f64)))
    } else if a > 0 && d > 0 {
        Some(f64::INFINITY)
    } else {
        None
    };
    let rr_val = if c > 0 {
        Some(p1 / p2)
    } else if a > 0 {
        Some(f64::INFINITY)
    } else {
        None
    };
    let rd = p1 - p2;
    let nnt = if rd.abs() > 1e-15 { Some(1.0 / rd) } else { None };

    let (l1, u1) = wilson_score_ci(a, r1, confidence);
    let (l2, u2) = wilson_score_ci(c, r2, confidence);
    let rd_lo = (rd - ((p1 - l1).powi(2) + (u2 - p2).powi(2)).sqrt()).clamp(-1.0, 1.0);
    let rd_hi = (rd + ((u1 - p1).powi(2) + (p2 - l2).powi(2)).sqrt()).clamp(-1.0, 1.0);
    let (nnt_kind, nnt_value, nnt_point_label, nnt_ci_label, nnt_ci_bounds) = altman_nnt_from_rd(rd, rd_lo, rd_hi);

    // Standard diagnostic 2x2 orientation: Rows = Test +/-, Cols = Disease +/- (TP=a, FP=b, FN=c, TN=d).
    let (sens, sens_ci) = if c1 > 0 { (Some((a as f64) / (c1 as f64)), Some(wilson_score_ci(a, c1, confidence))) } else { (None, None) };
    let (spec, spec_ci) = if c2 > 0 { (Some((d as f64) / (c2 as f64)), Some(wilson_score_ci(d, c2, confidence))) } else { (None, None) };
    let ppv = p1;
    let npv = (d as f64) / (r2 as f64);

    Ok(TwoByTwoRatiosResult {
        odds_ratio: or_val,
        or_baptista_pike_ci: baptista_pike_or_ci(a, b, c, d, confidence),
        relative_risk: rr_val,
        rr_koopman_ci: koopman_rr_ci(a, b, c, d, confidence),
        risk_difference: rd,
        rd_newcombe_ci: Some((rd_lo, rd_hi)),
        nnt,
        nnt_kind,
        nnt_value,
        nnt_point_label,
        nnt_ci_label,
        nnt_ci_bounds,
        sensitivity: sens,
        sensitivity_wilson_ci: sens_ci,
        specificity: spec,
        specificity_wilson_ci: spec_ci,
        ppv,
        ppv_wilson_ci: wilson_score_ci(a, r1, confidence),
        npv,
        npv_wilson_ci: wilson_score_ci(d, r2, confidence),
    })
}

#[cfg(test)]
mod tests {
    use curvebook_math::ConfidenceLevel;
    use curvebook_math::dist::norm_two_sided_isf;

    use super::super::contingency::{checked_table_margins, fisher_exact_2x2};
    use super::{
        HypergeomMargins, NoncentralHypergeomCtx, StatsError, baptista_pike_or_ci, koopman_chi2_score, koopman_rr_ci, koopman_solve_lower,
        koopman_solve_lower_c_zero, koopman_solve_upper, koopman_solve_upper_a_zero, noncentral_hypergeom_p, noncentral_hypergeom_p_and_mean_log_or,
        noncentral_hypergeom_p_and_mean_windowed,
    };

    #[test]
    fn koopman_extreme_boundary_cases_bracket_and_solve_score_to_zero() {
        let conf = ConfidenceLevel::DEFAULT_95;
        let z2 = norm_two_sided_isf(conf.alpha()).unwrap().powi(2);

        assert!(koopman_rr_ci(0, 10, 0, 10, conf).is_none());
        assert!(koopman_rr_ci(0, 0, 5, 5, conf).is_none());
        assert!(koopman_rr_ci(5, 5, 0, 0, conf).is_none());

        let cases = [
            (14_u64, 6_u64, 5_u64, 15_u64),
            (10, 0, 5, 5),
            (5, 5, 10, 0),
            (10, 0, 10, 0),
            (1, 0, 1, 0),
            (0, 10, 5, 5),
            (0, 10_000_000, 5, 5),
            (0, 10, 10, 0),
            (9, 1, 0, 10),
            (10, 0, 0, 10),
            (1, 999, 0, 10),
            (1, 10_000_000_000, 1, 1),
            (1, 1, 1, 10_000_000_000),
            (1, 10_000_000_000, 1, 10_000_000_000),
        ];

        for (a, b, c, d) in cases {
            let (lo, hi) = koopman_rr_ci(a, b, c, d, conf).unwrap();
            assert!(lo >= 0.0 && lo < hi, "invalid interval ({lo}, {hi}) for ({a},{b},{c},{d})");
            if a == 0 {
                assert_eq!(lo, 0.0);
            } else {
                assert!(lo.is_finite() && lo > 0.0);
                let err_lo = koopman_chi2_score(a, b, c, d, z2, lo).abs();
                assert!(err_lo < 1e-7, "non-zero score at lo={lo}: {err_lo} for ({a},{b},{c},{d})");
            }
            if c == 0 {
                assert!(hi.is_infinite());
            } else {
                assert!(hi.is_finite());
                let err_hi = koopman_chi2_score(a, b, c, d, z2, hi).abs();
                assert!(err_hi < 1e-7, "non-zero score at hi={hi}: {err_hi} for ({a},{b},{c},{d})");
            }
            if a > 0 && c > 0 {
                let n1 = (a + b) as f64;
                let n2 = (c + d) as f64;
                let rr_pt = ((a as f64) / n1) / ((c as f64) / n2);
                assert!(lo < rr_pt && rr_pt < hi, "point estimate {rr_pt} outside ({lo}, {hi})");
            }
        }

        let (lo_all, hi_all) = koopman_rr_ci(10, 0, 10, 0, conf).unwrap();
        assert!((lo_all - 10.0 / (10.0 + z2)).abs() < 1e-9);
        assert!((hi_all - (10.0 + z2) / 10.0).abs() < 1e-9);

        let (lo_sep, hi_sep) = koopman_rr_ci(10, 0, 0, 10, conf).unwrap();
        assert!((lo_sep - (1.0 + 10.0 / z2)).abs() < 1e-9);
        assert!(hi_sep.is_infinite());
    }

    #[test]
    fn koopman_published_reference_and_zero_cell_values() {
        let conf_95 = ConfidenceLevel::DEFAULT_95;
        let conf_99 = ConfidenceLevel::PERCENT_99;
        let conf_9999 = ConfidenceLevel::from_percent(99.99).unwrap();

        // Koopman (1984), Biometrics 40:513-517, p. 516 example: x1=36/40, x2=16/80 (RR = 4.5).
        let (lo_36_95, hi_36_95) = koopman_rr_ci(36, 4, 16, 64, conf_95).unwrap();
        assert!((lo_36_95 - 2.939_569_192_214_994).abs() < 1e-9, "lo={lo_36_95}");
        assert!((hi_36_95 - 7.152_223_434_866_03).abs() < 1e-9, "hi={hi_36_95}");

        let (lo_36_99, hi_36_99) = koopman_rr_ci(36, 4, 16, 64, conf_99).unwrap();
        assert!((lo_36_99 - 2.598_049_751_056_122).abs() < 1e-9, "lo99={lo_36_99}");
        assert!((hi_36_99 - 8.284_221_357_270_2).abs() < 1e-9, "hi99={hi_36_99}");

        let (lo_16, hi_16) = koopman_rr_ci(16, 4, 2, 18, conf_95).unwrap();
        assert!((lo_16 - 2.565_778_683_398_19).abs() < 1e-9, "lo16={lo_16}");
        assert!((hi_16 - 29.093_901_260_350_407).abs() < 1e-8, "hi16={hi_16}");

        // Zero-cell edge cases: a = 0 gives [0, upper], c = 0 gives [lower, +inf), symmetric under group swap.
        let a_zero_cases =
            [(0_u64, 10_u64, 5_u64, 5_u64, 0.612_860_577_425_767), (0, 15, 4, 11, 0.841_877_258_244_93), (0, 10, 10, 0, 0.277_532_799_862_889)];
        for (a, b, c, d, expected_hi) in a_zero_cases {
            let (lo, hi) = koopman_rr_ci(a, b, c, d, conf_95).unwrap();
            assert_eq!(lo, 0.0);
            assert!((hi - expected_hi).abs() < 1e-9, "a=0 hi={hi} expected={expected_hi}");

            let (swap_lo, swap_hi) = koopman_rr_ci(c, d, a, b, conf_95).unwrap();
            assert!((swap_lo - 1.0 / expected_hi).abs() < 1e-9, "swap_lo={swap_lo} expected={}", 1.0 / expected_hi);
            assert!(swap_hi.is_infinite());
        }

        let (lo_9999, hi_9999) = koopman_rr_ci(0, 10, 10, 0, conf_9999).unwrap();
        assert_eq!(lo_9999, 0.0);
        assert!((hi_9999 - 0.756_835_261_331_17).abs() < 1e-9, "hi9999={hi_9999}");

        let (lo_c0_a, hi_c0_a) = koopman_rr_ci(5, 5, 0, 10, conf_95).unwrap();
        assert!((lo_c0_a - 1.631_692_487_384_906).abs() < 1e-9);
        assert!(hi_c0_a.is_infinite());

        let (lo_c0_b, hi_c0_b) = koopman_rr_ci(9, 1, 0, 10, conf_95).unwrap();
        assert!((lo_c0_b - 3.186_579_700_647_302).abs() < 1e-9);
        assert!(hi_c0_b.is_infinite());
    }

    #[test]
    fn koopman_solvers_return_none_on_non_bracketing_scores() {
        assert!((koopman_solve_lower(&|phi| 0.5 - phi, 1.0).unwrap() - 0.5).abs() < 1e-9);
        assert!((koopman_solve_upper(&|phi| phi - 2.5, 1.0).unwrap() - 2.5).abs() < 1e-9);
        assert!((koopman_solve_lower_c_zero(&|phi| 0.25 - phi).unwrap() - 0.25).abs() < 1e-9);
        assert!((koopman_solve_upper_a_zero(&|phi| phi - 4.0).unwrap() - 4.0).abs() < 1e-9);

        for rr_bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(koopman_solve_lower(&|phi| 0.5 - phi, rr_bad).is_none());
            assert!(koopman_solve_upper(&|phi| phi - 2.5, rr_bad).is_none());
        }

        let always_neg = |_: f64| -1.0;
        let always_pos = |_: f64| 1.0;
        let unreachable_lo = |phi: f64| if phi < 1e-45 { 1.0 } else { -1.0 };
        let unreachable_hi = |phi: f64| if phi > 1e45 { 1.0 } else { -1.0 };

        assert!(koopman_solve_lower(&always_neg, 1.0).is_none());
        assert!(koopman_solve_lower(&always_pos, 1.0).is_none());
        assert!(koopman_solve_lower(&unreachable_lo, 1.0).is_none());

        assert!(koopman_solve_upper(&always_neg, 1.0).is_none());
        assert!(koopman_solve_upper(&always_pos, 1.0).is_none());
        assert!(koopman_solve_upper(&unreachable_hi, 1.0).is_none());

        assert!(koopman_solve_lower_c_zero(&always_neg).is_none());
        assert!(koopman_solve_lower_c_zero(&always_pos).is_none());
        assert!(koopman_solve_lower_c_zero(&unreachable_lo).is_none());

        assert!(koopman_solve_upper_a_zero(&always_neg).is_none());
        assert!(koopman_solve_upper_a_zero(&always_pos).is_none());
        assert!(koopman_solve_upper_a_zero(&unreachable_hi).is_none());
    }

    #[test]
    fn koopman_solvers_return_none_on_non_finite_and_non_convergent_scores() {
        let lower_nan_mid = |phi: f64| {
            if phi < 0.4 {
                1.0
            } else if phi < 0.6 {
                f64::NAN
            } else {
                -1.0
            }
        };
        let upper_neginf_mid = |phi: f64| {
            if phi > 3.0 {
                1.0
            } else if phi > 1.5 {
                f64::NEG_INFINITY
            } else {
                -1.0
            }
        };
        let nan_inside_lo_bracket = |phi: f64| {
            if phi <= 0.5 {
                1.0
            } else if phi >= 1.0 {
                -1.0
            } else {
                f64::NAN
            }
        };
        let nan_inside_hi_bracket = |phi: f64| {
            if phi <= 1.0 {
                -1.0
            } else if phi >= 2.0 {
                1.0
            } else {
                f64::NAN
            }
        };

        assert!(koopman_solve_lower(&|_| f64::NAN, 1.0).is_none());
        assert!(koopman_solve_lower(&|phi| if phi < 0.6 { f64::INFINITY } else { -1.0 }, 1.0).is_none());
        assert!(koopman_solve_lower(&lower_nan_mid, 1.0).is_none());

        assert!(koopman_solve_upper(&|_| f64::NAN, 1.0).is_none());
        assert!(koopman_solve_upper(&|phi| if phi > 1.5 { f64::INFINITY } else { -1.0 }, 1.0).is_none());
        assert!(koopman_solve_upper(&upper_neginf_mid, 1.0).is_none());

        assert!(koopman_solve_lower_c_zero(&|phi| if phi >= 1.0 { f64::NAN } else { -1.0 }).is_none());
        assert!(koopman_solve_lower_c_zero(&|phi| if phi <= 0.6 { f64::INFINITY } else { -1.0 }).is_none());

        assert!(koopman_solve_upper_a_zero(&|phi| if phi <= 1.0 { f64::NAN } else { -1.0 }).is_none());
        assert!(koopman_solve_upper_a_zero(&|phi| if phi >= 1.5 { f64::INFINITY } else { -1.0 }).is_none());

        assert!(koopman_solve_lower(&nan_inside_lo_bracket, 1.0).is_none());
        assert!(koopman_solve_upper(&nan_inside_hi_bracket, 1.0).is_none());
        assert!(koopman_solve_lower_c_zero(&nan_inside_lo_bracket).is_none());
        assert!(koopman_solve_upper_a_zero(&nan_inside_hi_bracket).is_none());
    }

    #[test]
    fn noncentral_hypergeom_and_table_margin_overflow_guards() {
        // Total overflow, total cap, span cap, invalid col sum, out-of-range cell, and bad OR.
        assert!(matches!(
            noncentral_hypergeom_p(1, u64::MAX, 1, 1, 1.0),
            Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_total", .. })
        ));
        assert!(matches!(
            noncentral_hypergeom_p(1, 3_000_000, 3_000_000, 2, 1.0),
            Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_max_total", .. })
        ));
        assert!(matches!(
            noncentral_hypergeom_p(50_000, 200_000, 200_000, 200_000, 1.0),
            Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_max_span", .. })
        ));
        assert!(matches!(noncentral_hypergeom_p(1, 5, 5, 11, 1.0), Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_col1", .. })));
        assert!(matches!(noncentral_hypergeom_p(6, 5, 5, 6, 1.0), Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_cell", .. })));
        assert!(matches!(noncentral_hypergeom_p(2, 5, 5, 5, f64::NAN), Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_or", .. })));
        assert!(matches!(noncentral_hypergeom_p(2, 5, 5, 5, -1.0), Err(StatsError::InvalidParameter { name: "noncentral_hypergeom_or", .. })));
        // Valid small case stays close to central hypergeometric p at OR = 1.0.
        let p_unit = noncentral_hypergeom_p(8, 10, 10, 10, 1.0).unwrap();
        let p_fisher = fisher_exact_2x2(8, 2, 2, 8).unwrap();
        assert!((p_unit - p_fisher).abs() < 1e-12);

        // Checked table margins reject row overflow, col overflow, table total overflow, and ragged rows.
        assert!(matches!(
            checked_table_margins(&[vec![u64::MAX, 1], vec![0, 0]]),
            Err(StatsError::InvalidParameter { name: "contingency_row_total", .. })
        ));
        assert!(matches!(
            checked_table_margins(&[vec![1_u64 << 63, 0], vec![1_u64 << 63, 0]]),
            Err(StatsError::InvalidParameter { name: "contingency_table_total", .. })
        ));
        assert!(matches!(
            checked_table_margins(&[vec![1_u64 << 63, 0], vec![0, 1_u64 << 63]]),
            Err(StatsError::InvalidParameter { name: "contingency_table_total", .. })
        ));
        assert!(matches!(
            checked_table_margins(&[vec![1, 2], vec![3]]),
            Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: 2, actual: 1 }))
        ));
    }

    #[test]
    fn baptista_pike_extreme_or_and_zero_cell_root_roundtrip() {
        let alpha = ConfidenceLevel::DEFAULT_95.alpha();
        let assert_inf_sup_bracket = |a: u64, r1: u64, r2: u64, c1: u64, lo: Option<f64>, hi: Option<f64>| {
            if let Some(l) = lo {
                let p_out = noncentral_hypergeom_p(a, r1, r2, c1, l * (1.0 - 1e-6)).unwrap();
                let p_in = noncentral_hypergeom_p(a, r1, r2, c1, l * (1.0 + 1e-6)).unwrap();
                assert!(p_out <= alpha + 1e-9 && p_in + 1e-9 >= alpha, "lower bound {l} not at threshold: p_out={p_out}, p_in={p_in}");
            }
            if let Some(h) = hi {
                let p_in = noncentral_hypergeom_p(a, r1, r2, c1, h * (1.0 - 1e-6)).unwrap();
                let p_out = noncentral_hypergeom_p(a, r1, r2, c1, h * (1.0 + 1e-6)).unwrap();
                assert!(p_in + 1e-9 >= alpha && p_out <= alpha + 1e-9, "upper bound {h} not at threshold: p_in={p_in}, p_out={p_out}");
            }
        };

        // Interior extreme tiny OR (`[[1, 1000], [1000, 1]]`): finite CI on {theta: p(theta) >= alpha}.
        let (lo_tiny, hi_tiny) = baptista_pike_or_ci(1, 1000, 1000, 1, ConfidenceLevel::DEFAULT_95).expect("interior tiny OR CI");
        assert!(lo_tiny > 0.0 && lo_tiny < 1e-6 && hi_tiny > 1e-6 && hi_tiny < 1.0, "got ({lo_tiny}, {hi_tiny})");
        assert_inf_sup_bracket(1, 1001, 1001, 1001, Some(lo_tiny), Some(hi_tiny));

        // Zero-cell upper boundary (`a == hi_x`): finite lower bound, infinite upper bound.
        let (lo_b0, hi_b0) = baptista_pike_or_ci(2, 0, 50, 50, ConfidenceLevel::DEFAULT_95).expect("b=0 CI");
        assert!(lo_b0 > 0.0 && hi_b0.is_infinite(), "got ({lo_b0}, {hi_b0})");
        assert_inf_sup_bracket(2, 2, 100, 52, Some(lo_b0), None);

        // Zero-cell lower boundary (`a == lo_x`): lower bound == 0.0, finite upper bound.
        let (lo_a0, hi_a0) = baptista_pike_or_ci(0, 50, 2, 100, ConfidenceLevel::DEFAULT_95).expect("a=0 CI");
        assert_eq!(lo_a0, 0.0);
        assert!(hi_a0.is_finite() && hi_a0 > 0.0);
        assert_inf_sup_bracket(0, 50, 102, 2, None, Some(hi_a0));

        // Explicit or_val == 0.0 handling in noncentral_hypergeom_p.
        assert_eq!(noncentral_hypergeom_p(0, 5, 5, 4, 0.0).unwrap(), 1.0);
        assert_eq!(noncentral_hypergeom_p(1, 5, 5, 4, 0.0).unwrap(), 0.0);

        // Balanced moderate table roundtrip.
        let (lo_bal, hi_bal) = baptista_pike_or_ci(10, 5, 3, 12, ConfidenceLevel::DEFAULT_95).expect("balanced CI");
        assert_inf_sup_bracket(10, 15, 15, 13, Some(lo_bal), Some(hi_bal));
    }

    #[test]
    fn baptista_pike_windowed_matches_unwindowed_and_handles_large_spans_and_totals() {
        // 1. Direct windowed vs full-span summation parity on exact discrete boundary tables.
        for &(a, b, c, d, log_or) in &[(14_u64, 6_u64, 5_u64, 15_u64, 1.9_f64), (1, 1000, 1000, 1, -13.5), (5_000, 15_000, 10_000, 20_000, -0.45)] {
            let r1 = a + b;
            let r2 = c + d;
            let c1 = a + c;
            let lo = c1.saturating_sub(r2);
            let hi = r1.min(c1);
            let span = hi - lo + 1;
            let span_usize = usize::try_from(span).unwrap();
            let ctx = NoncentralHypergeomCtx { a_obs: a, margins: HypergeomMargins { r1, r2, c1 }, lo, hi, span, windowed: false };
            let (p_full, m_full) = noncentral_hypergeom_p_and_mean_log_or(ctx, span_usize, log_or).unwrap();
            let (p_win, m_win) = noncentral_hypergeom_p_and_mean_windowed(ctx, log_or, true).unwrap();
            assert!((p_full - p_win).abs() < 1e-11, "p mismatch: full={p_full} win={p_win}");
            assert!((m_full - m_win).abs() < 1e-9, "mean mismatch: full={m_full} win={m_win}");
        }

        // 2. Exceeding NONCENTRAL_HYPERGEOM_MAX_SPAN (100_000) with total <= 5_000_000:
        //    (50_000, 150_000, 150_000, 50_000) has span=200_001 and exact reciprocal symmetry.
        let (lo_sym, hi_sym) = baptista_pike_or_ci(50_000, 150_000, 150_000, 50_000, ConfidenceLevel::DEFAULT_95).unwrap();
        let (lo_flip, hi_flip) = baptista_pike_or_ci(150_000, 50_000, 50_000, 150_000, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!((lo_sym * hi_flip - 1.0).abs() < 1e-9 && (hi_sym * lo_flip - 1.0).abs() < 1e-9);
        assert!(lo_sym < 1.0 / 9.0 && 1.0 / 9.0 < hi_sym);

        // Verify exact conditional inf/sup bracket p(theta) crosses alpha at both roots on the span > 100_000 table.
        let alpha = ConfidenceLevel::DEFAULT_95.alpha();
        let sym_ctx = NoncentralHypergeomCtx {
            a_obs: 50_000,
            margins: HypergeomMargins { r1: 200_000, r2: 200_000, c1: 200_000 },
            lo: 0,
            hi: 200_000,
            span: 200_001,
            windowed: true,
        };
        let (p_lo_out, _) = noncentral_hypergeom_p_and_mean_windowed(sym_ctx, (lo_sym * (1.0 - 1e-6)).ln(), false).unwrap();
        let (p_lo_in, _) = noncentral_hypergeom_p_and_mean_windowed(sym_ctx, (lo_sym * (1.0 + 1e-6)).ln(), false).unwrap();
        let (p_hi_in, _) = noncentral_hypergeom_p_and_mean_windowed(sym_ctx, (hi_sym * (1.0 - 1e-6)).ln(), false).unwrap();
        let (p_hi_out, _) = noncentral_hypergeom_p_and_mean_windowed(sym_ctx, (hi_sym * (1.0 + 1e-6)).ln(), false).unwrap();
        assert!(p_lo_out <= alpha + 1e-9 && p_lo_in + 1e-9 >= alpha, "p_lo_out={p_lo_out}, p_lo_in={p_lo_in}");
        assert!(p_hi_in + 1e-9 >= alpha && p_hi_out <= alpha + 1e-9, "p_hi_in={p_hi_in}, p_hi_out={p_hi_out}");

        // 3. Asymmetric large-count table exceeding both MAX_TOTAL and MAX_SPAN (1M/3M/2M/12M, OR=2.0).
        let (lo_asym, hi_asym) = baptista_pike_or_ci(1_000_000, 3_000_000, 2_000_000, 12_000_000, ConfidenceLevel::DEFAULT_95).unwrap();
        let (lo_aflip, hi_aflip) = baptista_pike_or_ci(3_000_000, 1_000_000, 12_000_000, 2_000_000, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!(lo_asym < 2.0 && 2.0 < hi_asym);
        assert!((lo_asym * hi_aflip - 1.0).abs() < 1e-8 && (hi_asym * lo_aflip - 1.0).abs() < 1e-8);

        // 4. Sparse / zero-cell large-total tables (> 5_000_000 total).
        let (lo_tiny_lg, hi_tiny_lg) = baptista_pike_or_ci(1, 10_000_000, 10_000_000, 1, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!(lo_tiny_lg > 0.0 && lo_tiny_lg < 1e-13 && hi_tiny_lg > 1e-13 && hi_tiny_lg < 1e-4);
        let (lo_z0, hi_z0) = baptista_pike_or_ci(0, 10_000_000, 5, 10_000_000, ConfidenceLevel::DEFAULT_95).unwrap();
        assert_eq!(lo_z0, 0.0);
        assert!(hi_z0.is_finite() && hi_z0 > 0.0);
        let (lo_zb, hi_zb) = baptista_pike_or_ci(5, 0, 10_000_000, 10_000_000, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!(lo_zb > 0.0 && lo_zb.is_finite() && hi_zb.is_infinite());
    }

    #[test]
    fn noncentral_hypergeom_extreme_tail_uses_relative_log_space_tolerance() {
        // D1 Python exact rational / Decimal(prec=60) references for Fisher's noncentral hypergeometric two-sided tail P:
        // - (40, 40, 40, 40, OR=1.0): 2 / C(80, 40) = 1.8603403656036264e-23
        // - (35, 35, 35, 35, OR=0.5): 1.6644787521087209e-26
        // - (30, 30, 30, 30, OR=2.0): 1.2510776526992096e-13
        // - (40, 40, 40, 40, OR=0.5): 2.644451551880381e-30 (beyond windowed -65.0 log cutoff; unwindowed stays exact)
        let windowed_cases = [
            (40_u64, 40_u64, 40_u64, 40_u64, 1.0_f64, 1.860_340_365_603_626_4e-23_f64),
            (35, 35, 35, 35, 0.5, 1.664_478_752_108_721e-26),
            (30, 30, 30, 30, 2.0, 1.251_077_652_699_209_6e-13),
        ];
        for (a, r1, r2, c1, or_val, expected_p) in windowed_cases {
            let p_pub = noncentral_hypergeom_p(a, r1, r2, c1, or_val).unwrap();
            let lo = c1.saturating_sub(r2);
            let hi = r1.min(c1);
            let span = hi - lo + 1;
            let span_usize = usize::try_from(span).unwrap();
            let ctx = NoncentralHypergeomCtx { a_obs: a, margins: HypergeomMargins { r1, r2, c1 }, lo, hi, span, windowed: false };
            let (p_log, m_log) = noncentral_hypergeom_p_and_mean_log_or(ctx, span_usize, or_val.ln()).unwrap();
            let (p_win, m_win) = noncentral_hypergeom_p_and_mean_windowed(ctx, or_val.ln(), true).unwrap();

            assert!((p_pub - expected_p).abs() <= 1e-11 * expected_p, "p_pub={p_pub} expected={expected_p}");
            assert!((p_log - expected_p).abs() <= 1e-11 * expected_p, "p_log={p_log} expected={expected_p}");
            assert!((p_win - expected_p).abs() <= 1e-10 * expected_p, "p_win={p_win} expected={expected_p}");
            assert!((m_log - m_win).abs() < 1e-11, "mean mismatch: {m_log} vs {m_win}");
        }

        let p_40_half = noncentral_hypergeom_p(40, 40, 40, 40, 0.5).unwrap();
        let ctx_40 =
            NoncentralHypergeomCtx { a_obs: 40, margins: HypergeomMargins { r1: 40, r2: 40, c1: 40 }, lo: 0, hi: 40, span: 41, windowed: false };
        let (p_40_half_log, _) = noncentral_hypergeom_p_and_mean_log_or(ctx_40, 41, 0.5_f64.ln()).unwrap();
        let expected_40_half = 2.644_451_551_880_381e-30_f64;
        assert!((p_40_half - expected_40_half).abs() <= 1e-11 * expected_40_half, "p_40_half={p_40_half}");
        assert!((p_40_half_log - expected_40_half).abs() <= 1e-11 * expected_40_half, "p_40_half_log={p_40_half_log}");

        // Exact unit-OR parity against Fisher's exact test even far below 1e-12.
        let p_fisher_40 = fisher_exact_2x2(40, 0, 0, 40).unwrap();
        let p_nchg_40 = noncentral_hypergeom_p(40, 40, 40, 40, 1.0).unwrap();
        assert!((p_nchg_40 - p_fisher_40).abs() <= 1e-11 * p_fisher_40, "nchg={p_nchg_40} fisher={p_fisher_40}");
    }
}
