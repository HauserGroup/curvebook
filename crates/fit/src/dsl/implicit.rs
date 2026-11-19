//! Root-finding fallback for implicit equations: bracket scan with Brent, then damped Newton.
//!
//! References:
//! - Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.
//! - Nocedal, J. & Wright, S. J. (2006). *Numerical Optimization*, 2nd ed. Springer.

pub(in crate::dsl) fn implicit_probes_and_scale(x: &[f64], params: &[f64]) -> (Vec<f64>, f64) {
    const MAGS: [f64; 34] = [
        1e-30, 1e-24, 1e-18, 1e-15, 1e-14, 1e-13, 1e-12, 1e-11, 1e-10, 1e-9, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 0.05, 0.1, 0.25, 0.5, 1.0,
        2.0, 5.0, 10.0, 100.0, 1e3, 1e4, 1e6, 1e9, 1e15, 1e18, 1e24, 1e30,
    ];
    let mut pos_probes = Vec::with_capacity(64);
    pos_probes.extend_from_slice(&MAGS);
    let mut max_data_scale = 0.0_f64;
    for &v in x.iter().chain(params.iter()) {
        let av = v.abs();
        if v.is_finite() && av > 0.0 {
            max_data_scale = max_data_scale.max(av);
            for factor in [0.1, 0.5, 1.0, 2.0, 10.0] {
                let scaled = av * factor;
                if scaled.is_finite() && scaled > 0.0 {
                    pos_probes.push(scaled);
                }
            }
        }
    }
    pos_probes.sort_by(f64::total_cmp);
    pos_probes.dedup_by(|a, b| (*a - *b).abs() <= 1e-12 * a.abs().max(b.abs()));
    (pos_probes, max_data_scale.max(1e-12))
}

pub(in crate::dsl) fn implicit_newton_fallback<F, B>(f: &F, brent_scaled: &B, best_starts: &[(f64, f64)], anchor_scale: f64) -> Option<f64>
where
    F: Fn(f64) -> f64,
    B: Fn(f64, f64) -> Option<f64>,
{
    for &(start_y, start_fy) in best_starts {
        let mut y = start_y;
        let mut fy = start_fy;
        let init_f_abs = fy.abs();
        for _ in 0..48 {
            if !fy.is_finite() {
                break;
            }
            if fy == 0.0 {
                return Some(y);
            }
            let h = 1e-6 * if y != 0.0 { y.abs() } else { anchor_scale };
            let fp = f(y + h);
            let fm = f(y - h);
            let df = if fp.is_finite() && fm.is_finite() {
                (fp - fm) / (2.0 * h)
            } else if fp.is_finite() {
                (fp - fy) / h
            } else if fm.is_finite() {
                (fy - fm) / h
            } else {
                break;
            };
            if !df.is_finite() || df == 0.0 {
                break;
            }
            let delta = fy / df;
            if !delta.is_finite() {
                break;
            }
            let y_tol = 1e-11 * y.abs().max(anchor_scale * 1e-6);
            let ref_f = init_f_abs.max(df.abs() * y.abs().max(anchor_scale * 1e-6)).max(f64::MIN_POSITIVE);
            if fy.abs() <= 1e-12 * ref_f && delta.abs() <= y_tol {
                return Some(y);
            }
            let mut alpha = 1.0_f64;
            let mut accepted = false;
            for _ in 0..16 {
                let cand = y - alpha * delta;
                if cand.is_finite() {
                    let f_cand = f(cand);
                    if f_cand.is_finite() {
                        if f_cand == 0.0 {
                            return Some(cand);
                        }
                        if (f_cand > 0.0) != (fy > 0.0)
                            && let Some(root) = brent_scaled(y.min(cand), y.max(cand))
                        {
                            return Some(root);
                        }
                        if f_cand.abs() < fy.abs() {
                            y = cand;
                            fy = f_cand;
                            accepted = true;
                            break;
                        }
                    }
                }
                alpha *= 0.5;
            }
            if !accepted {
                break;
            }
        }
        let h_end = 1e-6 * if y != 0.0 { y.abs() } else { anchor_scale };
        let df_end = ((f(y + h_end) - f(y - h_end)) / (2.0 * h_end)).abs();
        let ref_f = init_f_abs.max(if df_end.is_finite() { df_end * y.abs().max(anchor_scale * 1e-6) } else { 0.0 }).max(f64::MIN_POSITIVE);
        if fy.is_finite()
            && df_end.is_finite()
            && df_end > 0.0
            && fy.abs() <= 1e-11 * ref_f
            && (fy / df_end).abs() <= 1e-10 * y.abs().max(anchor_scale * 1e-6)
        {
            return Some(y);
        }
    }
    None
}
