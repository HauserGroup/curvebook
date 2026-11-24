//! Pure-Rust clean-room Bessel and hypergeometric special functions.
//!
//! J/Y: NIST DLMF §10 ascending series in 106-bit compensated double-double arithmetic, Hankel
//! asymptotic expansions, upward Y_n recurrence, and Steed/Lentz CF1 with Wronskian normalisation;
//! I/K: Abramowitz & Stegun (1964) 9.8.1–9.8.8; hypergeometric: DLMF §13 and §15.
//!
//! References:
//! - Abramowitz, M. & Stegun, I. A. (eds.) (1964). *Handbook of Mathematical Functions with Formulas, Graphs, and Mathematical Tables*. National Bureau of Standards.
//! - Olver, F. W. J. et al. (eds.) (2010). *NIST Handbook of Mathematical Functions* (NIST Digital Library of Mathematical Functions, <https://dlmf.nist.gov/>). Cambridge University Press.
//! - Thompson, I. J. & Barnett, A. R. (1986). Coulomb and Bessel functions of complex arguments and order. *Journal of Computational Physics* 64(2), 490–509.

use std::sync::OnceLock;

use curvebook_math::quad::{QuadOptions, integrate_adaptive};
use curvebook_math::special::{digamma, gamma, ln_factorial, ln_gamma, reg_beta};
use curvebook_math::sum::NeumaierSum;

use crate::dsl::limits::MAX_BESSEL_ORDER;

pub(in crate::dsl) const BESSEL_ASYMP_X: f64 = 20.0;

#[derive(Clone, Copy)]
pub(in crate::dsl) struct Dd {
    pub(in crate::dsl) hi: f64,
    pub(in crate::dsl) lo: f64,
}

impl Dd {
    const ZERO: Self = Self { hi: 0.0, lo: 0.0 };
    const ONE: Self = Self { hi: 1.0, lo: 0.0 };

    #[inline]
    pub(in crate::dsl) const fn from_f64(x: f64) -> Self {
        Self { hi: x, lo: 0.0 }
    }

    #[inline]
    pub(in crate::dsl) fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    #[inline]
    pub(in crate::dsl) fn two_sum(a: f64, b: f64) -> (f64, f64) {
        let s = a + b;
        let v = s - a;
        let e = (a - (s - v)) + (b - v);
        (s, e)
    }

    #[inline]
    pub(in crate::dsl) fn quick_two_sum(a: f64, b: f64) -> (f64, f64) {
        let s = a + b;
        let e = b - (s - a);
        (s, e)
    }

    #[inline]
    pub(in crate::dsl) fn add(self, rhs: Self) -> Self {
        let (s1, s2) = Self::two_sum(self.hi, rhs.hi);
        let (t1, t2) = Self::two_sum(self.lo, rhs.lo);
        let (v_hi, v_lo) = Self::quick_two_sum(s1, s2 + t1);
        let (hi, lo) = Self::quick_two_sum(v_hi, t2 + v_lo);
        Self { hi, lo }
    }

    #[inline]
    pub(in crate::dsl) fn sub(self, rhs: Self) -> Self {
        self.add(Self { hi: -rhs.hi, lo: -rhs.lo })
    }

    #[inline]
    pub(in crate::dsl) fn mul(self, rhs: Self) -> Self {
        let p1 = self.hi * rhs.hi;
        let p2 = self.hi.mul_add(rhs.hi, -p1);
        let c = p2 + self.hi * rhs.lo + self.lo * rhs.hi;
        let (hi, lo) = Self::quick_two_sum(p1, c);
        Self { hi, lo }
    }

    #[inline]
    pub(in crate::dsl) fn div(self, rhs: Self) -> Self {
        let q1 = self.hi / rhs.hi;
        let r = self.sub(Self::from_f64(q1).mul(rhs));
        let q2 = r.hi / rhs.hi;
        let r2 = r.sub(Self::from_f64(q2).mul(rhs));
        let q3 = r2.hi / rhs.hi;
        let (s1, s2) = Self::quick_two_sum(q1, q2);
        Self { hi: s1, lo: s2 }.add(Self::from_f64(q3))
    }
}

pub(in crate::dsl) struct BesselDdConsts {
    pub(in crate::dsl) pi_2: Dd,
    pub(in crate::dsl) pi_4: Dd,
    pub(in crate::dsl) two_over_pi: Dd,
    pub(in crate::dsl) ln2: Dd,
    pub(in crate::dsl) euler_gamma: Dd,
}

pub(in crate::dsl) static BESSEL_DD_CONSTS: OnceLock<BesselDdConsts> = OnceLock::new();

pub(in crate::dsl) fn atan_recip_dd(inv_x: f64) -> Dd {
    let x = Dd::ONE.div(Dd::from_f64(inv_x));
    let x2 = x.mul(x);
    let mut term = x;
    let mut sum = x;
    for k in 1..40_u32 {
        term = term.mul(x2);
        let delta = term.div(Dd::from_f64(f64::from(2 * k + 1)));
        sum = if k % 2 == 1 { sum.sub(delta) } else { sum.add(delta) };
        if delta.hi.abs() <= 1e-34 {
            break;
        }
    }
    sum
}

pub(in crate::dsl) fn bessel_dd_consts() -> &'static BesselDdConsts {
    BESSEL_DD_CONSTS.get_or_init(|| {
        // Machin's identity: pi / 4 = 4 * atan(1 / 5) - atan(1 / 239).
        let pi_4 = Dd::from_f64(4.0).mul(atan_recip_dd(5.0)).sub(atan_recip_dd(239.0));
        let pi_2 = pi_4.mul(Dd::from_f64(2.0));
        let pi = pi_2.mul(Dd::from_f64(2.0));
        let two_over_pi = Dd::from_f64(2.0).div(pi);

        // ln(2) = 2 * atanh(1 / 3) = 2 * sum_{k>=0} 1 / ((2k + 1) * 3^(2k + 1)).
        let inv9 = Dd::ONE.div(Dd::from_f64(9.0));
        let mut p = Dd::ONE.div(Dd::from_f64(3.0));
        let mut ln2_half = Dd::ZERO;
        for k in 0..40_u32 {
            let delta = p.div(Dd::from_f64(f64::from(2 * k + 1)));
            ln2_half = ln2_half.add(delta);
            if delta.hi.abs() <= 1e-34 {
                break;
            }
            p = p.mul(inv9);
        }
        let ln2 = ln2_half.mul(Dd::from_f64(2.0));

        // Euler-Mascheroni constant via Euler-Maclaurin at N = 1024 = 2^10 (remainder < 1e-32).
        const N: usize = 1024;
        let mut h_n = Dd::ZERO;
        for k in (1..=N).rev() {
            h_n = h_n.add(Dd::ONE.div(Dd::from_f64(k as f64)));
        }
        let n_dd = Dd::from_f64(N as f64);
        let inv_n = Dd::ONE.div(n_dd);
        let inv_n2 = inv_n.mul(inv_n);
        let inv_n4 = inv_n2.mul(inv_n2);
        let inv_n6 = inv_n4.mul(inv_n2);
        let inv_n8 = inv_n4.mul(inv_n4);
        let ln_n = Dd::from_f64(10.0).mul(ln2);
        let em = inv_n
            .mul(Dd::from_f64(-0.5))
            .add(inv_n2.div(Dd::from_f64(12.0)))
            .sub(inv_n4.div(Dd::from_f64(120.0)))
            .add(inv_n6.div(Dd::from_f64(252.0)))
            .sub(inv_n8.div(Dd::from_f64(240.0)));
        let euler_gamma = h_n.sub(ln_n).add(em);

        BesselDdConsts { pi_2, pi_4, two_over_pi, ln2, euler_gamma }
    })
}

/// 106-bit natural logarithm `ln(u)` for positive finite `u` via power-of-2 reduction into
/// `[sqrt(0.5), sqrt(2)]` and `atanh` series.
pub(in crate::dsl) fn ln_dd(u: f64, consts: &BesselDdConsts) -> Dd {
    let mut f = u;
    let mut k: i32 = 0;
    while f >= core::f64::consts::SQRT_2 {
        f *= 0.5;
        k += 1;
    }
    while f < core::f64::consts::FRAC_1_SQRT_2 {
        f *= 2.0;
        k -= 1;
    }
    let f_dd = Dd::from_f64(f);
    let t = f_dd.sub(Dd::ONE).div(f_dd.add(Dd::ONE));
    let t2 = t.mul(t);
    let mut pow = t;
    let mut acc = Dd::ZERO;
    for j in 0..24_u32 {
        let term = pow.div(Dd::from_f64(f64::from(2 * j + 1)));
        acc = acc.add(term);
        if term.hi.abs() <= 1e-34 {
            break;
        }
        pow = pow.mul(t2);
    }
    acc.mul(Dd::from_f64(2.0)).add(Dd::from_f64(f64::from(k)).mul(consts.ln2))
}

pub(in crate::dsl) fn parse_bessel_order(v: f64) -> Option<i32> {
    if !v.is_finite() {
        return None;
    }
    let rounded = v.round();
    if rounded.abs() > f64::from(MAX_BESSEL_ORDER) {
        return None;
    }
    Some(rounded as i32)
}

/// Compute `J0(ax)` and optionally `Y0(ax)` for `0 < ax <= BESSEL_ASYMP_X` from DLMF §10.2.2 &
/// §10.8.1 ascending series in 106-bit `DoubleDouble` precision (reaches full `f64` accuracy even
/// across the sign cancellations around `|x| ~ 20` and near zeros).
pub(in crate::dsl) fn bessel_j0_y0_series(ax: f64, need_y0: bool) -> (f64, Option<f64>) {
    if ax == 0.0 {
        return (1.0, None);
    }
    let z = Dd::from_f64(ax).mul(Dd::from_f64(0.5));
    let neg_z2 = Dd::ZERO.sub(z.mul(z));
    let mut term = Dd::ONE;
    let mut j0_sum = Dd::ONE;
    let mut h_k = Dd::ZERO;
    let mut y0_sum = Dd::ZERO;

    for k in 1..=64_u32 {
        let kf = f64::from(k);
        term = term.mul(neg_z2).div(Dd::from_f64(kf * kf));
        j0_sum = j0_sum.add(term);
        if need_y0 {
            h_k = h_k.add(Dd::ONE.div(Dd::from_f64(kf)));
            y0_sum = y0_sum.sub(term.mul(h_k));
        }
        if term.hi.abs() <= 1e-32 * j0_sum.hi.abs().max(1.0) && (!need_y0 || term.hi.abs() * h_k.hi.abs() <= 1e-32) {
            break;
        }
    }

    let j0_val = j0_sum.to_f64();
    if !need_y0 {
        return (j0_val, None);
    }
    let consts = bessel_dd_consts();
    let log_factor = ln_dd(0.5 * ax, consts).add(consts.euler_gamma);
    let y0_val = consts.two_over_pi.mul(log_factor.mul(j0_sum).add(y0_sum)).to_f64();
    (j0_val, y0_val.is_finite().then_some(y0_val))
}

/// Compute `J1(ax)` and optionally `Y1(ax)` for `0 < ax <= BESSEL_ASYMP_X` from DLMF §10.2.2 &
/// §10.8.1 ascending series in 106-bit `DoubleDouble` precision.
pub(in crate::dsl) fn bessel_j1_y1_series(ax: f64, need_y1: bool) -> (f64, Option<f64>) {
    if ax == 0.0 {
        return (0.0, None);
    }
    let z = Dd::from_f64(ax).mul(Dd::from_f64(0.5));
    let neg_z2 = Dd::ZERO.sub(z.mul(z));
    let mut term = z;
    let mut j1_sum = z;
    let mut h_kp1 = Dd::ONE;
    let mut y1_sum = z;

    for k in 1..=64_u32 {
        let kf = f64::from(k);
        let kp1 = f64::from(k + 1);
        term = term.mul(neg_z2).div(Dd::from_f64(kf * kp1));
        j1_sum = j1_sum.add(term);
        if need_y1 {
            let h_k = h_kp1;
            h_kp1 = h_kp1.add(Dd::ONE.div(Dd::from_f64(kp1)));
            y1_sum = y1_sum.add(term.mul(h_k.add(h_kp1)));
        }
        if term.hi.abs() <= 1e-32 * j1_sum.hi.abs().max(ax * 0.5) {
            break;
        }
    }

    let j1_val = j1_sum.to_f64();
    if !need_y1 {
        return (j1_val, None);
    }
    let consts = bessel_dd_consts();
    let log_factor = ln_dd(0.5 * ax, consts).add(consts.euler_gamma);
    let inv_ax = Dd::ONE.div(Dd::from_f64(ax));
    let y1_inner = log_factor.mul(j1_sum).sub(inv_ax).sub(y1_sum.mul(Dd::from_f64(0.5)));
    let y1_val = consts.two_over_pi.mul(y1_inner).to_f64();
    (j1_val, y1_val.is_finite().then_some(y1_val))
}

/// Evaluate Hankel's asymptotic polynomials `P(nu, x)` and `Q(nu, x)` (DLMF §10.17.1–§10.17.4)
/// for `ax > BESSEL_ASYMP_X = 20.0`, where optimal truncation error is `< 1e-16`.
pub(in crate::dsl) fn hankel_pq(mu_4nu2: f64, ax: f64) -> (f64, f64) {
    let mut p_acc = NeumaierSum::new();
    let mut q_acc = NeumaierSum::new();
    p_acc.add(1.0);
    let mut term = 1.0_f64;
    let mut prev_abs = f64::INFINITY;

    for k in 1..=64_u32 {
        let kf = f64::from(k);
        let odd = 2.0 * kf - 1.0;
        let factor = (mu_4nu2 - odd * odd) / (8.0 * kf * ax);
        term *= factor;
        let cur_abs = term.abs();
        if cur_abs > prev_abs && k > 4 {
            break;
        }
        prev_abs = cur_abs;
        if k % 2 == 1 {
            let sign = if ((k - 1) / 2) % 2 == 0 { 1.0 } else { -1.0 };
            q_acc.add(sign * term);
        } else {
            let sign = if (k / 2) % 2 == 0 { 1.0 } else { -1.0 };
            p_acc.add(sign * term);
        }
        if cur_abs <= 0.25 * f64::EPSILON {
            break;
        }
    }
    (p_acc.total(), q_acc.total())
}

/// Compute `(cos(ax - pi/4), sin(ax - pi/4))` without losing phase precision at large `ax`.
pub(in crate::dsl) fn hankel_phase_nu0(ax: f64) -> (f64, f64) {
    if ax <= 1e12 {
        let consts = bessel_dd_consts();
        let q_f64 = ((ax - core::f64::consts::FRAC_PI_4) * core::f64::consts::FRAC_2_PI).round();
        let r = Dd::from_f64(ax).sub(consts.pi_4).sub(Dd::from_f64(q_f64).mul(consts.pi_2)).to_f64();
        let (sr, cr) = r.sin_cos();
        let quad = (q_f64 as i64).rem_euclid(4);
        match quad {
            0 => (cr, sr),
            1 => (-sr, cr),
            2 => (-cr, -sr),
            _ => (sr, -cr),
        }
    } else {
        let (s, c) = ax.sin_cos();
        ((c + s) * core::f64::consts::FRAC_1_SQRT_2, (s - c) * core::f64::consts::FRAC_1_SQRT_2)
    }
}

/// Bessel `J0(x)`: 106-bit compensated ascending series (DLMF §10.2.2) for `|x| <= 20` and Hankel
/// asymptotic expansion (DLMF §10.17.3) with `DoubleDouble` phase reduction for `|x| > 20`.
/// Reaches near-`f64` accuracy (`~1e-15` relative error away from zeros, `~1e-16` absolute near zeros).
pub(in crate::dsl) fn bessel_j0(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let ax = x.abs();
    if ax <= BESSEL_ASYMP_X {
        bessel_j0_y0_series(ax, false).0
    } else {
        let (p, q) = hankel_pq(0.0, ax);
        let (cos0, sin0) = hankel_phase_nu0(ax);
        (core::f64::consts::FRAC_2_PI / ax).sqrt() * (cos0 * p - sin0 * q)
    }
}

/// Bessel `J1(x)`: 106-bit compensated ascending series (DLMF §10.2.2) for `|x| <= 20` and Hankel
/// asymptotic expansion (DLMF §10.17.3) for `|x| > 20`. Near-`f64` accuracy.
pub(in crate::dsl) fn bessel_j1(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let ax = x.abs();
    let mag = if ax <= BESSEL_ASYMP_X {
        bessel_j1_y1_series(ax, false).0
    } else {
        let (p, q) = hankel_pq(4.0, ax);
        let (cos0, sin0) = hankel_phase_nu0(ax);
        // chi_1 = chi_0 - pi/2 => cos(chi_1) = sin(chi_0), sin(chi_1) = -cos(chi_0).
        (core::f64::consts::FRAC_2_PI / ax).sqrt() * (sin0 * p + cos0 * q)
    };
    if x < 0.0 { -mag } else { mag }
}

/// Steed / modified Lentz continued fraction `f_n = J'_n(x) / J_n(x)` for `0 < x < n`
/// (DLMF §10.10.1 + §10.6.2): `f_n = n/x - 1 / (2(n+1)/x - 1 / (2(n+2)/x - ...))`.
pub(in crate::dsl) fn bessel_cf1_jprime_over_j(n: i32, ax: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let nf = f64::from(n);
    let mut f = nf / ax;
    let mut c = f;
    let mut d = 0.0_f64;
    for k in 1..=10_000_usize {
        let b_k = 2.0 * (nf + k as f64) / ax;
        d = b_k - d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b_k - 1.0 / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() <= 0.5 * f64::EPSILON {
            break;
        }
    }
    f
}

/// Integer-order `J_n(x)` from NIST DLMF §10:
/// - Upward recurrence from `J0`, `J1` in the oscillatory regime `|x| >= n` (DLMF §10.6.1);
/// - For `0 < |x| < n`: small-argument series (`|x| < 1e-10`), DLMF §10.14.4 early underflow bound,
///   or Steed/Lentz continued fraction `J'_n / J_n` (DLMF §10.10.1) + power-of-2-scaled upward `Y_k`
///   recurrence + Wronskian normalization `J_n = 2 / (pi * x * (Y'_n - f_n Y_n))` (DLMF §10.5.2).
///
/// Achieves near-`f64` accuracy (`~1e-15`–`1e-12` depending on `n`) without Miller downward loops.
pub(in crate::dsl) fn bessel_jn(n: i32, x: f64) -> f64 {
    if n.unsigned_abs() > MAX_BESSEL_ORDER as u32 || !x.is_finite() {
        return f64::NAN;
    }
    if n < 0 {
        let pos = bessel_jn(-n, x);
        return if (-n) % 2 == 0 { pos } else { -pos };
    }
    if n == 0 {
        return bessel_j0(x);
    }
    if n == 1 {
        return bessel_j1(x);
    }
    if x == 0.0 {
        return 0.0;
    }
    let ax = x.abs();
    let nf = f64::from(n);
    let ans = if ax >= nf {
        let tox = 2.0 / ax;
        let mut bjm = bessel_j0(ax);
        let mut bj = bessel_j1(ax);
        for j in 1..n {
            let bjp = (f64::from(j) * tox).mul_add(bj, -bjm);
            bjm = bj;
            bj = bjp;
        }
        bj
    } else {
        bessel_jn_evanescent(n, nf, ax)
    };
    if x < 0.0 && n % 2 != 0 { -ans } else { ans }
}

pub(in crate::dsl) fn bessel_jn_evanescent(n: i32, nf: f64, ax: f64) -> f64 {
    // DLMF §10.14.4 upper bound |J_n(x)| <= (x / 2)^n / n!: guarantees exact f64 underflow to 0.
    let Ok(lg_np1) = ln_gamma(nf + 1.0) else {
        return 0.0;
    };
    let ln_lead = nf * (0.5 * ax).ln() - lg_np1;
    if ln_lead < -746.0 {
        return 0.0;
    }
    if ax < 1e-10 {
        let z2 = 0.25 * ax * ax;
        let poly = 1.0 - (z2 / (nf + 1.0)) * (1.0 - z2 / (2.0 * (nf + 2.0)));
        return ln_lead.exp() * poly;
    }

    let f_n = bessel_cf1_jprime_over_j(n, ax);
    let (Some(mut bym), Some(mut by)) = (bessel_y0(ax), bessel_y1(ax)) else {
        return 0.0;
    };
    let mut scale_exp2: i32 = 0;
    const RESCALE_LIMIT: f64 = 1e100;
    // Exact 2^-512 power-of-2 scaling introduces zero mantissa rounding error.
    let scale_down = 2.0_f64.powi(-512);
    for j in 1..n {
        let byp = (f64::from(2 * j) / ax).mul_add(by, -bym);
        bym = by;
        by = byp;
        if by.abs() > RESCALE_LIMIT {
            by *= scale_down;
            bym *= scale_down;
            scale_exp2 = scale_exp2.saturating_add(512);
            if scale_exp2 > 2_000 {
                return 0.0;
            }
        }
    }
    let y_prime_scaled = bym - (nf / ax) * by;
    let denom_scaled = y_prime_scaled - f_n * by;
    let mut val = (core::f64::consts::FRAC_2_PI / ax) / denom_scaled;
    let mut rem = scale_exp2;
    while rem > 0 && val != 0.0 {
        let step = rem.min(512);
        val *= 2.0_f64.powi(-step);
        rem -= step;
    }
    val
}

/// Bessel `Y0(x)` for `x > 0`: 106-bit compensated DLMF §10.8.1 series (`x <= 20`) and Hankel
/// asymptotic expansion (`x > 20`, DLMF §10.17.4). Near-`f64` accuracy.
pub(in crate::dsl) fn bessel_y0(x: f64) -> Option<f64> {
    if !x.is_finite() || x <= 0.0 {
        return None;
    }
    if x <= BESSEL_ASYMP_X {
        bessel_j0_y0_series(x, true).1
    } else {
        let (p, q) = hankel_pq(0.0, x);
        let (cos0, sin0) = hankel_phase_nu0(x);
        let ans = (core::f64::consts::FRAC_2_PI / x).sqrt() * (sin0 * p + cos0 * q);
        ans.is_finite().then_some(ans)
    }
}

/// Bessel `Y1(x)` for `x > 0`: 106-bit compensated DLMF §10.8.1 series (`x <= 20`) and Hankel
/// asymptotic expansion (`x > 20`, DLMF §10.17.4). Near-`f64` accuracy.
pub(in crate::dsl) fn bessel_y1(x: f64) -> Option<f64> {
    if !x.is_finite() || x <= 0.0 {
        return None;
    }
    if x <= BESSEL_ASYMP_X {
        bessel_j1_y1_series(x, true).1
    } else {
        let (p, q) = hankel_pq(4.0, x);
        let (cos0, sin0) = hankel_phase_nu0(x);
        // chi_1 = chi_0 - pi/2 => sin(chi_1) = -cos(chi_0), cos(chi_1) = sin(chi_0).
        let ans = (core::f64::consts::FRAC_2_PI / x).sqrt() * (-cos0 * p + sin0 * q);
        ans.is_finite().then_some(ans)
    }
}

/// Integer-order `Y_n(x)` by (stable) upward recurrence from `Y0`, `Y1` (DLMF §10.6.1 / A&S 9.1.27),
/// returning `None` if `x <= 0` or the dominant solution overflows `f64`.
pub(in crate::dsl) fn bessel_yn(n: i32, x: f64) -> Option<f64> {
    if n.unsigned_abs() > MAX_BESSEL_ORDER as u32 || !x.is_finite() || x <= 0.0 {
        return None;
    }
    if n < 0 {
        let pos = bessel_yn(-n, x)?;
        return Some(if (-n) % 2 == 0 { pos } else { -pos });
    }
    if n == 0 {
        return bessel_y0(x);
    }
    if n == 1 {
        return bessel_y1(x);
    }
    let tox = 2.0 / x;
    let mut bym = bessel_y0(x)?;
    let mut by = bessel_y1(x)?;
    for j in 1..n {
        let byp = (f64::from(j) * tox).mul_add(by, -bym);
        bym = by;
        by = byp;
        if !by.is_finite() {
            return None;
        }
    }
    Some(by)
}

/// Modified Bessel `I0(x)`: polynomial approximations A&S 9.8.1 (`|x| < 3.75`) and 9.8.2
/// (`|x| >= 3.75`), whose tabulated error bounds are of order `1e-7` (A&S 1964).
pub(in crate::dsl) fn bessel_i0(x: f64) -> Option<f64> {
    let ax = x.abs();
    if ax > 700.0 {
        return None;
    }
    let ans = if ax < 3.75 {
        let y = (x / 3.75).powi(2);
        1.0 + y * (3.515_622_9 + y * (3.089_942_4 + y * (1.206_749_2 + y * (0.265_973_2 + y * (0.036_076_8 + y * 0.004_581_3)))))
    } else {
        let y = 3.75 / ax;
        (ax.exp() / ax.sqrt())
            * (0.398_942_28
                + y * (0.013_285_92
                    + y * (0.002_253_19
                        + y * (-0.001_575_65
                            + y * (0.009_162_81 + y * (-0.020_577_06 + y * (0.026_355_37 + y * (-0.016_476_33 + y * 0.003_923_77))))))))
    };
    if ans.is_finite() { Some(ans) } else { None }
}

/// Modified Bessel `I1(x)`: A&S 9.8.3 / 9.8.4 polynomial approximations (error order `1e-7`).
pub(in crate::dsl) fn bessel_i1(x: f64) -> Option<f64> {
    let ax = x.abs();
    if ax > 700.0 {
        return None;
    }
    let ans = if ax < 3.75 {
        let y = (x / 3.75).powi(2);
        ax * (0.5 + y * (0.878_905_94 + y * (0.514_988_69 + y * (0.150_849_34 + y * (0.026_587_33 + y * (0.003_015_32 + y * 0.000_324_11))))))
    } else {
        let y = 3.75 / ax;
        (ax.exp() / ax.sqrt())
            * (0.398_942_28
                + y * (-0.039_880_24
                    + y * (-0.003_620_18
                        + y * (0.001_638_01
                            + y * (-0.010_315_55 + y * (0.022_829_67 + y * (-0.028_953_12 + y * (0.017_876_54 - y * 0.004_200_59))))))))
    };
    let signed = if x < 0.0 { -ans } else { ans };
    if signed.is_finite() { Some(signed) } else { None }
}

/// Modified Bessel `K0(x)`, `x > 0`: A&S 9.8.5 (`x <= 2`) / 9.8.6 (`x > 2`) (error order `1e-7`).
pub(in crate::dsl) fn bessel_k0(x: f64) -> Option<f64> {
    if x <= 0.0 {
        return None;
    }
    let ans = if x <= 2.0 {
        let y = x * x / 4.0;
        -(x / 2.0).ln() * bessel_i0(x)?
            + (-core::f64::consts::EULER_GAMMA
                + y * (0.422_784_20 + y * (0.230_697_56 + y * (0.034_885_90 + y * (0.002_626_98 + y * (0.000_107_50 + y * 0.000_007_40))))))
    } else {
        let y = 2.0 / x;
        ((-x).exp() / x.sqrt())
            * (1.253_314_14
                + y * (-0.078_323_58 + y * (0.021_895_68 + y * (-0.010_624_46 + y * (0.005_878_72 + y * (-0.002_515_40 + y * 0.000_532_08))))))
    };
    if ans.is_finite() { Some(ans) } else { None }
}

/// Modified Bessel `K1(x)`, `x > 0`: A&S 9.8.7 (`x <= 2`) / 9.8.8 (`x > 2`) (error order `1e-7`).
pub(in crate::dsl) fn bessel_k1(x: f64) -> Option<f64> {
    if x <= 0.0 {
        return None;
    }
    let ans = if x <= 2.0 {
        let y = x * x / 4.0;
        (x / 2.0).ln() * bessel_i1(x)?
            + (1.0 / x)
                * (1.0
                    + y * (0.154_431_44 + y * (-0.672_785_79 + y * (-0.181_568_97 + y * (-0.019_194_02 + y * (-0.001_104_04 - y * 0.000_046_86))))))
    } else {
        let y = 2.0 / x;
        ((-x).exp() / x.sqrt())
            * (1.253_314_14
                + y * (0.234_986_19 + y * (-0.036_556_20 + y * (0.015_042_68 + y * (-0.007_803_53 + y * (0.003_256_14 - y * 0.000_682_45))))))
    };
    if ans.is_finite() { Some(ans) } else { None }
}

/// Modified Bessel function of the first kind `I_n(x)` for integer order `n`.
///
/// Uses the strictly positive ascending power series (DLMF §10.25.2) for `|x| <= 25`,
/// and Miller's downward recurrence `I_{k-1}(x) = I_{k+1}(x) + (2k/x) I_k(x)` normalized
/// by the generating-function identity `I_0(x) + 2 sum_{k>=1} I_k(x) = exp(x)` (DLMF §10.29.1,
/// §10.35.1; Press et al. 2007 §6.6) with log-scale rescaling for `|x| > 25`.
/// Symmetry: `I_{-n}(x) = I_n(x)` (DLMF §10.27.1), `I_n(-x) = (-1)^n I_n(x)` (DLMF §10.34.1).
pub(in crate::dsl) fn bessel_in(n: i32, x: f64) -> Option<f64> {
    if !x.is_finite() {
        return None;
    }
    let m = n.unsigned_abs();
    let ax = x.abs();
    if ax == 0.0 {
        return Some(if m == 0 { 1.0 } else { 0.0 });
    }
    let sign = if x < 0.0 && m % 2 == 1 { -1.0 } else { 1.0 };
    if ax <= 25.0 {
        let mf = f64::from(m);
        let ln_lead = mf * (0.5 * ax).ln() - ln_factorial(u64::from(m));
        let q = 0.25 * ax * ax;
        let mut sum = 1.0_f64;
        let mut term = 1.0_f64;
        for k in 1..=256_u32 {
            let kf = f64::from(k);
            term *= q / (kf * (mf + kf));
            sum += term;
            if term <= sum * 1e-16 {
                break;
            }
        }
        let ln_val = ln_lead + sum.ln();
        if !ln_val.is_finite() || ln_val > 709.78 {
            return None;
        }
        if ln_val < -745.0 {
            return Some(0.0);
        }
        let val = sign * ln_val.exp();
        return val.is_finite().then_some(val);
    }
    // Guard against extreme arguments that would make Miller's starting order exceed bounded work.
    if ax > 500_000.0 {
        return None;
    }
    let m_base = m.max(ax.ceil() as u32);
    let extra = (40.0 * ax.max(f64::from(m))).sqrt().ceil() as u32 + 32;
    let m_start = m_base.saturating_add(extra);
    let two_over_x = 2.0 / ax;
    const RESCALE: f64 = 1e-100;
    const LN_RESCALE: f64 = -230.258_509_299_404_6; // -100 * ln(10)

    let mut bip = 0.0_f64;
    let mut bi = 1.0_f64;
    let mut sum = 0.0_f64;
    let mut ans = 0.0_f64;
    let mut have_ans = false;
    let mut ans_log_scale = 0.0_f64;

    for k in (1..=m_start).rev() {
        if k == m {
            ans = bi;
            have_ans = true;
        }
        sum += 2.0 * bi;
        let bim = bip + f64::from(k) * two_over_x * bi;
        bip = bi;
        bi = bim;
        if bi > 1e100 {
            bi *= RESCALE;
            bip *= RESCALE;
            sum *= RESCALE;
            if have_ans {
                ans_log_scale += LN_RESCALE;
            }
        }
    }
    if m == 0 {
        ans = bi;
        have_ans = true;
    }
    sum += bi;
    if !have_ans || ans <= 0.0 || sum <= 0.0 || !sum.is_finite() {
        return None;
    }
    let ln_val = ans.ln() + ans_log_scale + ax - sum.ln();
    if !ln_val.is_finite() || ln_val > 709.78 {
        return None;
    }
    if ln_val < -745.0 {
        return Some(0.0);
    }
    let val = sign * ln_val.exp();
    val.is_finite().then_some(val)
}

/// Modified Bessel function of the second kind `K_n(x)` for integer order `n` and `x > 0`.
///
/// Uses symmetry `K_{-n}(x) = K_n(x)` (DLMF §10.27.3) and the unconditionally stable upward
/// recurrence `K_{k+1}(x) = K_{k-1}(x) + (2k/x) K_k(x)` seeded from `K_0(x)` and `K_1(x)`
/// (DLMF §10.29.1; Press et al. 2007 §6.6).
pub(in crate::dsl) fn bessel_kn(n: i32, x: f64) -> Option<f64> {
    if !x.is_finite() || x <= 0.0 {
        return None;
    }
    let m = n.unsigned_abs();
    if m == 0 {
        return bessel_k0(x);
    }
    if m == 1 {
        return bessel_k1(x);
    }
    let mut k_prev = bessel_k0(x)?;
    let mut k_curr = bessel_k1(x)?;
    let two_over_x = 2.0 / x;
    for k in 1..m {
        let k_next = k_prev + f64::from(k) * two_over_x * k_curr;
        if !k_next.is_finite() {
            return None;
        }
        k_prev = k_curr;
        k_curr = k_next;
    }
    k_curr.is_finite().then_some(k_curr)
}

/// Factorial `n!` for non-negative integers `n` (`0 <= n <= 170`).
///
/// Reference: Abramowitz & Stegun (1964) §6.1.6.
pub(in crate::dsl) fn factorial_f64(x: f64) -> Option<f64> {
    if !x.is_finite() || x < 0.0 {
        return None;
    }
    let r = x.round();
    if (x - r).abs() > 1e-12 || r > 170.0 {
        return None;
    }
    let n = r as u32;
    let mut acc = 1.0_f64;
    for k in 2..=n {
        acc *= f64::from(k);
    }
    acc.is_finite().then_some(acc)
}

/// Binomial upper cumulative tail `P(X >= ceil(k)) = sum_{j = ceil(k)}^{n} C(n, j) p^j (1 - p)^{n - j}`.
///
/// Evaluated via the exact regularized incomplete beta identity `I_p(j0, n - j0 + 1)`
/// (Abramowitz & Stegun 1964 §26.5.24; DLMF §8.17.5).
pub(in crate::dsl) fn binomial_upper_tail(k: f64, n: f64, p: f64) -> Option<f64> {
    if !k.is_finite() || !n.is_finite() || !p.is_finite() {
        return None;
    }
    if !(0.0..=1.0).contains(&p) || n < 0.0 || n > 1e15 {
        return None;
    }
    let n_int = n.round();
    if (n - n_int).abs() > 1e-12 {
        return None;
    }
    let j0 = k.ceil();
    if j0 <= 0.0 {
        return Some(1.0);
    }
    if j0 > n_int {
        return Some(0.0);
    }
    reg_beta(p, j0, n_int - j0 + 1.0).ok()
}

pub(in crate::dsl) fn is_non_positive_integer(v: f64) -> bool {
    v <= 0.0 && (v - v.round()).abs() < 1e-12
}

/// Optimal-truncation asymptotic series `2F0(a, a - b + 1; ; inv_z)` (DLMF §13.7.1–§13.7.3)
/// where `inv_z = -1 / z` (`-1 / z < 0` for `U(a, b, z)` at `z > 0`, and `-1 / z > 0` for
/// `1F1(a; b; z)` at `z < 0`). Returns `Some(sum)` only when optimal truncation reaches
/// `|last_term| <= 1e-13 * |sum|` and alternating cancellation remains `<= 1e10`.
pub(in crate::dsl) fn hyp2f0_asymp(a: f64, b: f64, inv_z: f64) -> Option<f64> {
    const MAX_TERMS: usize = 128;
    let c2 = a - b + 1.0;
    let mut sum = NeumaierSum::new();
    sum.add(1.0);
    let mut term = 1.0_f64;
    let mut prev_abs = 1.0_f64;
    let mut max_abs = 1.0_f64;

    for k in 0..MAX_TERMS {
        let kf = k as f64;
        let factor = ((a + kf) * (c2 + kf) / (kf + 1.0)) * inv_z;
        let next = term * factor;
        if !next.is_finite() {
            return None;
        }
        if next == 0.0 {
            let total = sum.total();
            return (total.is_finite() && max_abs <= 1e10 * total.abs().max(f64::MIN_POSITIVE)).then_some(total);
        }
        let next_abs = next.abs();
        let cur_total = sum.total();
        if k >= 2 && next_abs > prev_abs {
            return (cur_total.is_finite() && prev_abs <= 1e-13 * cur_total.abs().max(f64::MIN_POSITIVE) && max_abs <= 1e10 * cur_total.abs())
                .then_some(cur_total);
        }
        term = next;
        prev_abs = next_abs;
        max_abs = max_abs.max(next_abs);
        sum.add(term);
        let updated = sum.total();
        if !updated.is_finite() {
            return None;
        }
        max_abs = max_abs.max(updated.abs());
        if k >= 2 && next_abs <= 1e-15 * updated.abs().max(f64::MIN_POSITIVE) {
            return (max_abs <= 1e10 * updated.abs().max(f64::MIN_POSITIVE)).then_some(updated);
        }
    }
    None
}

/// Confluent hypergeometric function `1F1(a; b; z)` (Kummer's `M(a, b, z)`, DLMF §13.2 & §13.7):
///
/// - Rejects non-positive integer poles `b in {0, -1, -2, ...}` (`None`);
/// - Uses exact identities `1F1(0; b; z) = 1`, `1F1(a; b; 0) = 1`, `1F1(b; b; z) = exp(z)`;
/// - Uses Kummer's transformation `1F1(a; b; z) = exp(z) 1F1(b - a; b; -z)` for `z < -1`
///   (DLMF §13.2.39), falling back for large negative `z <= -30` when the transformed series
///   overflows (`z < -709`) or suffers `> 1e12` cancellation to DLMF §13.7.2 asymptotic
///   `(Gamma(b)/Gamma(b-a)) (-z)^(-a) 2F0(a, a-b+1; ; -1/z)` whenever the `exp(z)` branch is
///   `< 1e-16` relative to the algebraic branch and optimal truncation error is `<= 1e-13`;
/// - Uses Neumaier-compensated Kummer power series (up to `4000` terms) elsewhere, returning `None`
///   if `f64` overflows or cancellation exceeds `1e12` (`error <= ~1e-13` relative when accepted).
pub(in crate::dsl) fn hyp1f1(a: f64, b: f64, z: f64) -> Option<f64> {
    if !a.is_finite() || !b.is_finite() || !z.is_finite() || is_non_positive_integer(b) {
        return None;
    }
    if z == 0.0 || a == 0.0 {
        return Some(1.0);
    }
    if (a - b).abs() == 0.0 {
        let v = z.exp();
        return v.is_finite().then_some(v);
    }
    if z < -1.0 {
        if let Some(transformed) = hyp1f1_series(b - a, b, -z) {
            let scaled = z.exp() * transformed;
            if scaled.is_finite() {
                return Some(scaled);
            }
        }
        if z <= -30.0
            && let Some(asymp) = hyp1f1_large_neg_z(a, b, z)
        {
            return Some(asymp);
        }
    }
    hyp1f1_series(a, b, z)
}

pub(in crate::dsl) fn hyp1f1_large_neg_z(a: f64, b: f64, z: f64) -> Option<f64> {
    // At z <= -30, the algebraic term dominates the exp(z) term only when b - a is not a pole
    // (if b - a <= 0 integer, M(b-a, b, -z) is a polynomial) and the log-ratio < -37 (< 1e-16).
    let bma = b - a;
    if is_non_positive_integer(bma) {
        return None;
    }
    let neg_z = -z;
    if !is_non_positive_integer(a) {
        let gbma = gamma(bma).ok()?;
        let ga = gamma(a).ok()?;
        let log_exp_over_alg = (gbma.abs() / ga.abs()).ln() + z + (2.0 * a - b) * neg_z.ln();
        if log_exp_over_alg > -37.0 {
            return None;
        }
    }
    let asymp_series = hyp2f0_asymp(a, b, -1.0 / z)?;
    let gb = gamma(b).ok()?;
    let gbma = gamma(bma).ok()?;
    let val = (gb / gbma) * neg_z.powf(-a) * asymp_series;
    val.is_finite().then_some(val)
}

pub(in crate::dsl) fn hyp1f1_series(a: f64, b: f64, z: f64) -> Option<f64> {
    const MAX_ITERS: usize = 4_000;
    let mut term = 1.0_f64;
    let mut sum = NeumaierSum::new();
    sum.add(1.0);
    let mut max_abs = 1.0_f64;
    let mut prev_small = false;
    for k in 0..MAX_ITERS {
        let kf = k as f64;
        let denom = b + kf;
        if denom == 0.0 {
            return None;
        }
        let factor = ((a + kf) / denom) * (z / (kf + 1.0));
        term *= factor;
        if !term.is_finite() {
            return None;
        }
        if term == 0.0 {
            let total = sum.total();
            return if max_abs > 1e12 * total.abs().max(1.0) { None } else { Some(total) };
        }
        max_abs = max_abs.max(term.abs());
        sum.add(term);
        let total = sum.total();
        if !total.is_finite() {
            return None;
        }
        max_abs = max_abs.max(total.abs());
        let is_small = term.abs() <= 1e-14 * total.abs().max(f64::MIN_POSITIVE);
        let past_growth = (a + kf > 0.0 || kf + 1.0 > z.abs()) && b + kf > 0.0 && factor.abs() < 0.9;
        if k >= 1 && is_small && prev_small && past_growth {
            return if max_abs > 1e12 * total.abs().max(f64::MIN_POSITIVE) { None } else { Some(total) };
        }
        prev_small = is_small;
    }
    None
}

/// Gauss hypergeometric function `2F1(a, b; c; z)` for `z < 1` via Pfaff continuation (`z < -0.5`, A&S 15.3.4)
/// and the Gauss series (A&S 15.1.1), returning `None` on non-convergence.
pub(in crate::dsl) fn hyp2f1(a: f64, b: f64, c: f64, z: f64) -> Option<f64> {
    if !a.is_finite() || !b.is_finite() || !c.is_finite() || !z.is_finite() || z >= 1.0 || (c <= 0.0 && (c - c.round()).abs() < 1e-12) {
        return None;
    }
    if z == 0.0 || a == 0.0 || b == 0.0 {
        return Some(1.0);
    }
    if z < -0.5 {
        let w = z / (z - 1.0);
        if w.is_finite() && w < 1.0 {
            let one_minus_z = 1.0 - z;
            if let Some(series_val) = hyp2f1_series(a, c - b, c, w) {
                let res = one_minus_z.powf(-a) * series_val;
                if res.is_finite() {
                    return Some(res);
                }
            }
            if let Some(series_val) = hyp2f1_series(c - a, b, c, w) {
                let res = one_minus_z.powf(-b) * series_val;
                if res.is_finite() {
                    return Some(res);
                }
            }
        }
        if z <= -1.0 {
            return None;
        }
    }
    hyp2f1_series(a, b, c, z)
}

pub(in crate::dsl) fn hyp2f1_series(a: f64, b: f64, c: f64, z: f64) -> Option<f64> {
    const MAX_ITERS: usize = 50_000;
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    let mut max_abs = 1.0_f64;
    let mut prev_small = false;
    for k in 0..MAX_ITERS {
        let kf = k as f64;
        let denom = c + kf;
        if denom == 0.0 {
            return None;
        }
        let factor = ((a + kf) * (b + kf) / denom) * (z / (kf + 1.0));
        term *= factor;
        if !term.is_finite() {
            return None;
        }
        if term == 0.0 {
            return if max_abs > 1e12 * sum.abs().max(1.0) { None } else { Some(sum) };
        }
        max_abs = max_abs.max(term.abs());
        sum += term;
        if !sum.is_finite() {
            return None;
        }
        max_abs = max_abs.max(sum.abs());
        let is_small = term.abs() <= 1e-14 * sum.abs().max(f64::MIN_POSITIVE);
        let past_pole = c + kf > 0.0 && (a + kf > 0.0 || b + kf > 0.0 || kf + 1.0 > a.abs().max(b.abs()));
        if k >= 1 && is_small && prev_small && past_pole && factor.abs() < 0.9999 {
            return if max_abs > 1e12 * sum.abs().max(f64::MIN_POSITIVE) { None } else { Some(sum) };
        }
        prev_small = is_small;
    }
    None
}

/// Exact terminating polynomial `U(-m, b, z)` for integer `m >= 0` (DLMF §13.3.7 recurrence).
pub(in crate::dsl) fn confluent_u_neg_int_a(m: usize, b: f64, z: f64) -> Option<f64> {
    if m == 0 {
        return Some(1.0);
    }
    let mut u_prev = 1.0_f64;
    let mut u_cur = z - b;
    for k in 1..m {
        let kf = k as f64;
        let u_next = (z - b - 2.0 * kf) * u_cur - kf * (b + kf - 1.0) * u_prev;
        if !u_next.is_finite() {
            return None;
        }
        u_prev = u_cur;
        u_cur = u_next;
    }
    u_cur.is_finite().then_some(u_cur)
}

/// Exact integer-`b` logarithmic/digamma series (DLMF §13.2.9 / A&S 13.1.6) for `b = n + 1 >= 1`
/// and moderate `z in (0, 4]`, replacing the unverified `b + 1e-7` perturbation.
pub(in crate::dsl) fn confluent_u_integer_b_series(a: f64, n: usize, z: f64) -> Option<f64> {
    let nf = n as f64;
    if a <= 0.0 && a.fract() == 0.0 {
        return None;
    }
    let ga = gamma(a).ok()?;
    let g_an = gamma(a - nf).ok()?;
    let psi_a0 = digamma(a).ok()?;
    let mut psi_a = psi_a0;
    let mut psi_1 = -core::f64::consts::EULER_GAMMA;
    let mut psi_np1 = -core::f64::consts::EULER_GAMMA;
    for j in 1..=n {
        psi_np1 += 1.0 / (j as f64);
    }

    let mut fact_n = 1.0_f64;
    for j in 2..=n {
        fact_n *= j as f64;
    }

    // Infinite series: sum_{k>=0} ((a)_k / ((n+1)_k k!)) * (psi(a+k) - psi(1+k) - psi(1+n+k)) z^k
    let mut poch_term = 1.0_f64;
    let mut m_sum = NeumaierSum::new();
    let mut psi_sum = NeumaierSum::new();
    m_sum.add(1.0);
    psi_sum.add(psi_a - psi_1 - psi_np1);
    let mut max_psi_abs = (psi_a - psi_1 - psi_np1).abs();

    for k in 0..512_usize {
        let kf = k as f64;
        poch_term *= ((a + kf) / (nf + 1.0 + kf)) * (z / (kf + 1.0));
        if !poch_term.is_finite() {
            return None;
        }
        psi_a += 1.0 / (a + kf);
        psi_1 += 1.0 / (kf + 1.0);
        psi_np1 += 1.0 / (nf + 1.0 + kf);
        let d_psi = psi_a - psi_1 - psi_np1;
        let psi_term = poch_term * d_psi;
        if !psi_term.is_finite() {
            return None;
        }
        max_psi_abs = max_psi_abs.max(psi_term.abs());
        m_sum.add(poch_term);
        psi_sum.add(psi_term);
        if poch_term.abs() <= 1e-15 * m_sum.total().abs().max(1.0) && psi_term.abs() <= 1e-15 * psi_sum.total().abs().max(1.0) {
            break;
        }
    }

    let sign = if n.is_multiple_of(2) { -1.0 } else { 1.0 };
    let log_part = (sign / (fact_n * g_an)) * (m_sum.total() * z.ln() + psi_sum.total());

    let finite_part = if n == 0 {
        0.0
    } else {
        let fact_nm1 = fact_n / nf;
        let mut f_term = 1.0_f64;
        let mut f_sum = NeumaierSum::new();
        f_sum.add(1.0);
        for k in 0..(n - 1) {
            let kf = k as f64;
            f_term *= ((a - nf + kf) / (1.0 - nf + kf)) * (z / (kf + 1.0));
            f_sum.add(f_term);
        }
        (fact_nm1 / ga) * z.powi(-(n as i32)) * f_sum.total()
    };

    let ans = log_part + finite_part;
    if !ans.is_finite() || (max_psi_abs / (fact_n * g_an).abs() + finite_part.abs()) > 1e8 * ans.abs().max(1e-15) {
        return None;
    }
    Some(ans)
}

/// Strictly positive Laplace integral (DLMF §13.4.4) for `a > 0, z > 0, b in R`:
/// `U(a, b, z) = (z^-a / Gamma(a)) * int_0^inf exp(-u) u^(a-1) (1 + u/z)^(b-a-1) du`,
/// evaluated in log-variable `s = ln(u)` centered on the exact log-integrand peak `u^*` using
/// adaptive 15-point Gauss–Kronrod quadrature (`error <= ~1e-12` relative, zero integer-`b` pole).
pub(in crate::dsl) fn confluent_u_laplace_pos_a(a: f64, b: f64, z: f64) -> Option<f64> {
    if a <= 0.0 || z <= 0.0 {
        return None;
    }
    let lg_a = ln_gamma(a).ok()?;
    let p = b - a - 1.0;
    let c = b - 1.0 - z;
    let disc = c * c + 4.0 * a * z;
    let u_star = if c >= 0.0 { 0.5 * (c + disc.sqrt()) } else { (2.0 * a * z) / (-c + disc.sqrt()) };
    if !u_star.is_finite() || u_star <= 0.0 {
        return None;
    }
    let s_star = u_star.ln();
    let log_integrand = |s: f64| -> f64 {
        let u = s.exp();
        -u + a * s + p * (u / z).ln_1p()
    };
    let g_max = log_integrand(s_star);
    if !g_max.is_finite() {
        return None;
    }

    let delta = (1e-15 * z.min(1.0) / p.abs().max(1.0)).max(1e-300);
    let s_delta = delta.ln();
    let s_target_lo = s_star - 42.0 / a.min(100.0);
    let (s_min, small_tail) = if s_target_lo >= s_delta {
        let mut slo = s_target_lo;
        for _ in 0..8 {
            if log_integrand(slo) - g_max < -42.0 {
                break;
            }
            slo -= 2.0 / a.min(10.0);
        }
        (slo.max(s_delta), 0.0)
    } else {
        // Integrate [0, delta] analytically as delta^a / a since exp(-u)(1+u/z)^p = 1 + O(1e-15).
        let log_small = a * s_delta - a.ln() - g_max;
        let tail = if log_small < -745.0 { 0.0 } else { log_small.exp() };
        (s_delta, tail)
    };

    let mut s_max = s_star + 1.0;
    for _ in 0..64 {
        if log_integrand(s_max) - g_max < -42.0 {
            break;
        }
        s_max += 1.0;
    }

    let opts = QuadOptions { abs_tol: 1e-14, rel_tol: 1e-12, max_subdivisions: 256 };
    let integral = integrate_adaptive(
        |s| {
            let d = log_integrand(s) - g_max;
            if d < -745.0 { 0.0 } else { d.exp() }
        },
        s_min,
        s_max,
        opts,
    )
    .ok()?;
    let total_scaled = integral + small_tail;
    if !total_scaled.is_finite() || total_scaled <= 0.0 {
        return None;
    }
    let ln_u = -a * z.ln() - lg_a + g_max + total_scaled.ln();
    let val = if ln_u < -745.0 { 0.0 } else { ln_u.exp() };
    val.is_finite().then_some(val)
}

/// Evaluate `U(a, b, z)` for `z > 0` via Laplace quadrature (`a > 0`) or stable downward
/// `a`-recurrence (DLMF §13.3.7) from `a + m > 0` when `a < 0`.
pub(in crate::dsl) fn confluent_u_via_laplace(a: f64, b: f64, z: f64) -> Option<f64> {
    if a > 0.0 {
        return confluent_u_laplace_pos_a(a, b, z);
    }
    let shift = (1.0 - a).ceil();
    if !shift.is_finite() || shift > 200.0 {
        return None;
    }
    let m = shift as usize;
    let a_hi = a + (m as f64);
    let mut u_hi = confluent_u_laplace_pos_a(a_hi + 1.0, b, z)?;
    let mut u_cur = confluent_u_laplace_pos_a(a_hi, b, z)?;
    for k in (1..=m).rev() {
        let ak = a + (k as f64);
        let u_lo = (z + 2.0 * ak - b) * u_cur - ak * (ak - b + 1.0) * u_hi;
        if !u_lo.is_finite() {
            return None;
        }
        u_hi = u_cur;
        u_cur = u_lo;
    }
    Some(u_cur)
}

/// Tricomi confluent hypergeometric `U(a, b, z)` for `z > 0` (DLMF §13.2, §13.4, §13.7):
///
/// - Exact terminating polynomials for non-positive integer `a` or non-negative integer `b - a - 1`
///   (including `U(a, a + 1, z) = z^(-a)`, exact for all real `a` and `z > 0`);
/// - DLMF §13.7.3 asymptotic expansion `z^(-a) 2F0(a, a - b + 1; ; -1/z)` for `z >= 25` when optimal
///   truncation achieves `<= 1e-13` relative remainder (avoids `exp(+z)` cancellation in `1F1` difference);
/// - Exact DLMF §13.2.9 (A&S 13.1.6) logarithmic/digamma series at integer `b` for small `z <= 4`
///   (with DLMF §13.2.40 reflection `U(a, 1 - m, z) = z^m U(a + m, 1 + m, z)` for `b <= 0`);
/// - DLMF §13.2.42 (A&S 13.1.3) `1F1` connection formula for non-integer `b` (`dist(b, Z) >= 0.05`,
///   `z <= 4`, cancellation `<= 1e8`);
/// - Strictly positive Laplace integral (DLMF §13.4.4) + adaptive Gauss–Kronrod quadrature (plus
///   stable downward `a`-recurrence DLMF §13.3.7 when `a < 0`) for near-integer `b` and moderate `z`.
///
/// Achieves `<= ~1e-12` relative accuracy across all covered `(a, b, z > 0)` domains.
pub(in crate::dsl) fn confluent_u(a: f64, b_in: f64, z: f64) -> Option<f64> {
    if !a.is_finite() || !b_in.is_finite() || !z.is_finite() || z <= 0.0 {
        return None;
    }
    if a == 0.0 {
        return Some(1.0);
    }
    if a < 0.0 && a.fract() == 0.0 && a >= -200.0 {
        return confluent_u_neg_int_a((-a) as usize, b_in, z);
    }
    let k_poly = b_in - a - 1.0;
    if k_poly >= 0.0 && k_poly.fract() == 0.0 && k_poly <= 200.0 {
        let poly = confluent_u_neg_int_a(k_poly as usize, 2.0 - b_in, z)?;
        let val = z.powf(1.0 - b_in) * poly;
        return val.is_finite().then_some(val);
    }

    if z >= 25.0
        && let Some(asymp) = hyp2f0_asymp(a, b_in, -1.0 / z)
    {
        let val = z.powf(-a) * asymp;
        if val.is_finite() {
            return Some(val);
        }
    }

    if z <= 4.0 {
        if b_in.fract() == 0.0 && b_in.abs() <= 100.0 && a.abs() <= 50.0 {
            if b_in <= 0.0 {
                let m = (1.0 - b_in) as usize;
                if let Some(u_pos) = confluent_u_integer_b_series(a + m as f64, m, z) {
                    let val = z.powi(m as i32) * u_pos;
                    if val.is_finite() {
                        return Some(val);
                    }
                }
            } else {
                let n = (b_in - 1.0) as usize;
                if let Some(val) = confluent_u_integer_b_series(a, n, z) {
                    return Some(val);
                }
            }
        } else if (b_in - b_in.round()).abs() >= 0.05 {
            let pi = core::f64::consts::PI;
            if let (Some(m1), Some(m2), Ok(g1), Ok(g2), Ok(g3), Ok(g4)) =
                (hyp1f1(a, b_in, z), hyp1f1(1.0 + a - b_in, 2.0 - b_in, z), gamma(1.0 + a - b_in), gamma(b_in), gamma(a), gamma(2.0 - b_in))
            {
                let p1 = m1 / (g1 * g2);
                let p2 = z.powf(1.0 - b_in) * m2 / (g3 * g4);
                let diff = p1 - p2;
                let res = (pi / (pi * b_in).sin()) * diff;
                if res.is_finite() && (p1.abs() + p2.abs()) <= 1e8 * diff.abs().max(1e-300) {
                    return Some(res);
                }
            }
        }
    }

    confluent_u_via_laplace(a, b_in, z)
}
