//! High-accuracy 1D numerical quadrature (32/64-point Gauss-Legendre and bounded stack-based
//! adaptive 7/15-point Gauss-Kronrod integration).
//!
//! References:
//! - Piessens, R., de Doncker-Kapenga, E., Uberhuber, C. W., & Kahaner, D. K. (1983).
//!   *QUADPACK: A Subroutine Package for Automatic Integration*. Springer.
//! - NIST Digital Library of Mathematical Functions, §3.5 (Quadrature).

use std::sync::OnceLock;

use crate::error::MathError;
use crate::sum::NeumaierSum;

/// Symmetric positive abscissae (including `0.0` at index 7) of the 15-point Gauss-Kronrod rule.
const XGK_15: [f64; 8] = [
    0.991_455_371_120_812_6,
    0.949_107_912_342_758_5,
    0.864_864_423_359_769_1,
    0.741_531_185_599_394_4,
    0.586_087_235_467_691_1,
    0.405_845_151_377_397_2,
    0.207_784_955_007_898_5,
    0.0,
];

/// Kronrod 15-point weights matching [`XGK_15`].
const WGK_15: [f64; 8] = [
    0.022_935_322_010_529_225,
    0.063_092_092_629_978_56,
    0.104_790_010_322_250_18,
    0.140_653_259_715_525_92,
    0.169_004_726_639_267_9,
    0.190_350_578_064_785_42,
    0.204_432_940_075_298_9,
    0.209_482_141_084_727_83,
];

/// Embedded 7-point Gauss weights at odd indices `[1, 3, 5]` and center `7` of [`XGK_15`].
const WG_7: [f64; 4] = [0.129_484_966_168_869_7, 0.279_705_391_489_276_7, 0.381_830_050_505_118_9, 0.417_959_183_673_469_4];

struct GlRule<const HALF: usize> {
    nodes: [f64; HALF],
    weights: [f64; HALF],
}

static GL_32: OnceLock<GlRule<16>> = OnceLock::new();
static GL_64: OnceLock<GlRule<32>> = OnceLock::new();

fn build_even_gauss_legendre<const HALF: usize>() -> GlRule<HALF> {
    let n = HALF * 2;
    let nf = n as f64;
    let mut nodes = [0.0_f64; HALF];
    let mut weights = [0.0_f64; HALF];
    for (idx, (node_slot, weight_slot)) in nodes.iter_mut().zip(weights.iter_mut()).enumerate() {
        let i = idx + 1;
        let theta = std::f64::consts::PI * ((i as f64) - 0.25) / (nf + 0.5);
        let mut x = theta.cos();
        let mut dp = 1.0_f64;
        for _ in 0..16 {
            let (pn, pn_prev) = legendre_pn_and_prev(n, x);
            dp = nf * (x * pn - pn_prev) / (x * x - 1.0);
            let dx = pn / dp;
            x -= dx;
            if dx.abs() <= 0.5 * f64::EPSILON {
                break;
            }
        }
        *node_slot = x;
        *weight_slot = 2.0 / ((1.0 - x * x) * dp * dp);
    }
    GlRule { nodes, weights }
}

fn legendre_pn_and_prev(n: usize, x: f64) -> (f64, f64) {
    let mut p_prev = 1.0_f64;
    let mut p_curr = x;
    for k in 2..=n {
        let kf = k as f64;
        let p_next = ((2.0 * kf - 1.0) * x * p_curr - (kf - 1.0) * p_prev) / kf;
        p_prev = p_curr;
        p_curr = p_next;
    }
    (p_curr, p_prev)
}

/// Integrate `f` over `[a, b]` using a 32-point Gauss-Legendre rule with compensated summation.
pub fn gauss_legendre_32<F: FnMut(f64) -> f64>(mut f: F, a: f64, b: f64) -> f64 {
    let rule = GL_32.get_or_init(build_even_gauss_legendre::<16>);
    eval_symmetric_rule(&rule.nodes, &rule.weights, &mut f, a, b)
}

/// Integrate `f` over `[a, b]` using a 64-point Gauss-Legendre rule with compensated summation.
pub fn gauss_legendre_64<F: FnMut(f64) -> f64>(mut f: F, a: f64, b: f64) -> f64 {
    let rule = GL_64.get_or_init(build_even_gauss_legendre::<32>);
    eval_symmetric_rule(&rule.nodes, &rule.weights, &mut f, a, b)
}

fn eval_symmetric_rule<F: FnMut(f64) -> f64>(nodes: &[f64], weights: &[f64], f: &mut F, a: f64, b: f64) -> f64 {
    let center = 0.5 * a + 0.5 * b;
    let half = 0.5 * b - 0.5 * a;
    let mut acc = NeumaierSum::new();
    for (&xi, &wi) in nodes.iter().zip(weights.iter()) {
        let offset = half * xi;
        let y = f(center - offset) + f(center + offset);
        acc.add(wi * y);
    }
    half * acc.total()
}

/// Evaluate a single 15-point Gauss-Kronrod panel over `[a, b]`, returning `(integral, abs_error)`.
pub fn gauss_kronrod_15_panel<F: FnMut(f64) -> f64>(f: &mut F, a: f64, b: f64) -> (f64, f64) {
    let center = 0.5 * a + 0.5 * b;
    let half = 0.5 * b - 0.5 * a;
    let abs_half = half.abs();

    let f_center = f(center);
    let mut res_gauss = WG_7[3] * f_center;
    let mut res_kronrod = WGK_15[7] * f_center;
    let mut res_abs = WGK_15[7] * f_center.abs();

    for j in 0..7 {
        let absc = half * XGK_15[j];
        let f1 = f(center - absc);
        let f2 = f(center + absc);
        let fsum = f1 + f2;
        res_kronrod += WGK_15[j] * fsum;
        res_abs += WGK_15[j] * (f1.abs() + f2.abs());
        if j % 2 == 1 {
            res_gauss += WG_7[j / 2] * fsum;
        }
    }

    let integral = res_kronrod * half;
    let res_asc = abs_half * res_abs;
    let raw_diff = ((res_kronrod - res_gauss) * half).abs();
    // QUADPACK empirical error scaling (Piessens et al. 1983).
    let err = if res_asc != 0.0 && raw_diff != 0.0 {
        let scaled = res_asc * (200.0 * raw_diff / res_asc).powf(1.5).min(1.0);
        let floor = 50.0 * f64::EPSILON * res_asc;
        scaled.max(floor)
    } else {
        raw_diff
    };
    (integral, err)
}

/// Options for stack-based adaptive Gauss-Kronrod integration.
#[derive(Debug, Clone, Copy)]
pub struct QuadOptions {
    /// Absolute error tolerance.
    pub abs_tol: f64,
    /// Relative error tolerance.
    pub rel_tol: f64,
    /// Maximum number of interval bisections.
    pub max_subdivisions: usize,
}

impl Default for QuadOptions {
    fn default() -> Self {
        Self { abs_tol: 1e-13, rel_tol: 1e-12, max_subdivisions: 256 }
    }
}

#[derive(Clone, Copy)]
struct SubInterval {
    a: f64,
    b: f64,
    est: f64,
    err: f64,
}

/// Integrate `f` adaptively over finite interval `[a, b]` using a bounded priority worklist of
/// 15-point Gauss-Kronrod panels (zero call-stack recursion).
///
/// # Errors
/// Returns [`MathError::DomainError`] if `a`, `b`, or any integrand evaluation is non-finite, or
/// [`MathError::NotConverged`] if `options.max_subdivisions` is exceeded before meeting tolerance.
pub fn integrate_adaptive<F: FnMut(f64) -> f64>(mut f: F, a: f64, b: f64, options: QuadOptions) -> Result<f64, MathError> {
    if !a.is_finite() || !b.is_finite() {
        return Err(MathError::DomainError { name: "interval", value: a + b });
    }
    if a == b {
        return Ok(0.0);
    }
    let (init_est, init_err) = gauss_kronrod_15_panel(&mut f, a, b);
    if !init_est.is_finite() || !init_err.is_finite() {
        return Err(MathError::DomainError { name: "integrand", value: init_est });
    }
    if init_err <= options.abs_tol.max(options.rel_tol * init_est.abs()) {
        return Ok(init_est);
    }

    let cap = options.max_subdivisions.clamp(1, 4096);
    let mut pool = Vec::with_capacity(cap + 1);
    pool.push(SubInterval { a, b, est: init_est, err: init_err });
    let mut total_est = init_est;
    let mut total_err = init_err;

    for _ in 0..cap {
        let tol = options.abs_tol.max(options.rel_tol * total_est.abs());
        if total_err <= tol {
            break;
        }
        let Some((worst_idx, _)) = pool.iter().enumerate().max_by(|(_, x), (_, y)| x.err.total_cmp(&y.err)) else {
            break;
        };
        let worst = pool.swap_remove(worst_idx);
        let mid = 0.5 * worst.a + 0.5 * worst.b;
        if mid == worst.a || mid == worst.b {
            pool.push(worst);
            break;
        }
        let (est_l, err_l) = gauss_kronrod_15_panel(&mut f, worst.a, mid);
        let (est_r, err_r) = gauss_kronrod_15_panel(&mut f, mid, worst.b);
        if !est_l.is_finite() || !est_r.is_finite() {
            return Err(MathError::DomainError { name: "integrand", value: est_l + est_r });
        }
        total_est += (est_l + est_r) - worst.est;
        total_err += (err_l + err_r) - worst.err;
        pool.push(SubInterval { a: worst.a, b: mid, est: est_l, err: err_l });
        pool.push(SubInterval { a: mid, b: worst.b, est: est_r, err: err_r });
    }

    let mut sum_acc = NeumaierSum::new();
    for item in &pool {
        sum_acc.add(item.est);
    }
    let refined = sum_acc.total();
    let final_tol = options.abs_tol.max(options.rel_tol * refined.abs());
    if total_err <= final_tol { Ok(refined) } else { Err(MathError::NotConverged { algorithm: "integrate_adaptive", iterations: cap }) }
}

#[cfg(test)]
mod tests {
    use super::{QuadOptions, gauss_kronrod_15_panel, gauss_legendre_32, gauss_legendre_64, integrate_adaptive};

    #[test]
    fn gauss_legendre_exact_for_high_degree_polynomials() {
        for m in 0..30_i32 {
            let exact = 2.0 / (2.0 * f64::from(m) + 1.0);
            let approx = gauss_legendre_32(|x| x.powi(2 * m), -1.0, 1.0);
            assert!((approx - exact).abs() < 1e-14, "m={m}: {approx} vs {exact}");
        }
        for m in 0..60_i32 {
            let exact = 2.0 / (2.0 * f64::from(m) + 1.0);
            let approx = gauss_legendre_64(|x| x.powi(2 * m), -1.0, 1.0);
            assert!((approx - exact).abs() < 1e-14, "m={m}: {approx} vs {exact}");
        }
    }

    #[test]
    fn gauss_kronrod_exact_to_degree_21_and_adaptive_converges() {
        for m in 0..11_i32 {
            let exact = 2.0 / (2.0 * f64::from(m) + 1.0);
            let (approx, err) = gauss_kronrod_15_panel(&mut |x| x.powi(2 * m), -1.0, 1.0);
            assert!((approx - exact).abs() < 1e-14, "m={m}: {approx} vs {exact} (err={err})");
        }
        let val = integrate_adaptive(|x| 1.0 / (1.0 + x * x), 0.0, 1.0, QuadOptions::default()).unwrap();
        assert!((val - std::f64::consts::FRAC_PI_4).abs() < 1e-14);
    }
}
