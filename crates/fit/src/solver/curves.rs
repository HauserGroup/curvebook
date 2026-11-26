//! Fitted-curve overlays with confidence and prediction bands, and Brent inverse interpolation of unknowns.
//!
//! References:
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use curvebook_doc::{CiBound, CurveSamplePoint, FitMethod, FittedCurveOverlay, WeightingScheme};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::roots::{RootOptions, brent_root};
use curvebook_math::sum::NeumaierSum;

use crate::library::ParametricModel;
use crate::solver::layout::{ParamBinding, ParameterLayout, model_dataset_index};
use crate::solver::lm::{fd_step_size, mat_get};
use crate::solver::results::{InferenceAvailability, InterpolatedUnknown, SolverConfig};
use crate::types::{DatasetSeries, UnknownResponse};

pub(in crate::solver) struct CurveOverlayContext<'a> {
    pub(in crate::solver) model: &'a dyn ParametricModel,
    pub(in crate::solver) layout: &'a ParameterLayout,
    pub(in crate::solver) theta: &'a [f64],
    pub(in crate::solver) ds_params: &'a [f64],
    pub(in crate::solver) ds: &'a DatasetSeries,
    pub(in crate::solver) ds_local_idx: usize,
    pub(in crate::solver) cov_unscaled: Option<&'a DenseMatrix>,
    pub(in crate::solver) sigma: f64,
    pub(in crate::solver) t_crit: f64,
    pub(in crate::solver) converged: bool,
    pub(in crate::solver) ambiguous: bool,
    pub(in crate::solver) solver: &'a SolverConfig,
}

/// Variance inflation factor `1 / w_new(x, y_hat)` for a single future observation under `scheme`.
///
/// Returns `None` under [`WeightingScheme::ObservedSd`] or when observations carry custom
/// `explicit_weight` values, because no parametric variance function `Var(Y_new | x, y_hat)` is
/// defined at arbitrary `x` (`AUD-03-12`).
pub(in crate::solver) fn new_observation_inv_weight(scheme: WeightingScheme, x: f64, y_hat: f64, has_explicit_weights: bool) -> Option<f64> {
    if has_explicit_weights {
        return None;
    }
    match scheme {
        WeightingScheme::None => Some(1.0),
        WeightingScheme::ObservedSd => None,
        WeightingScheme::RelativeSquare => y_hat.is_finite().then_some(y_hat * y_hat).filter(|v| v.is_finite()),
        WeightingScheme::Relative => y_hat.is_finite().then_some(y_hat.abs()),
        WeightingScheme::GeneralPowerY(k) => {
            if !k.is_finite() || !y_hat.is_finite() {
                return None;
            }
            let inv = y_hat.abs().powf(k);
            (inv.is_finite() && inv >= 0.0).then_some(inv)
        }
        WeightingScheme::InverseX => x.is_finite().then_some(x.abs()),
        WeightingScheme::InverseX2 => x.is_finite().then_some(x * x).filter(|v| v.is_finite()),
    }
}

/// Sample the fitted curve with pointwise delta-method bands (Bates & Watts 1988;
/// Seber & Wild 1989): with `g = d f(x) / d theta` and `C = (J^T J)^{-1}`,
/// confidence band `y +/- t_{df} s sqrt(g^T C g)` and prediction band
/// `y +/- t_{df} s sqrt(1 + g^T C g)`. These are pointwise bands, not simultaneous
/// (Scheffé-type) bands over the whole curve.
pub(in crate::solver) fn build_curve_overlay(ctx: &CurveOverlayContext<'_>) -> FittedCurveOverlay {
    let mut data_x_min = f64::INFINITY;
    let mut data_x_max = f64::NEG_INFINITY;
    for pt in &ctx.ds.points {
        let x0 = pt.x0();
        if x0.is_finite() {
            data_x_min = data_x_min.min(x0);
            data_x_max = data_x_max.max(x0);
        }
    }
    let x_lo_opt = ctx.solver.plot_x_min.filter(|v| v.is_finite()).or(if data_x_min.is_finite() { Some(data_x_min) } else { None });
    let x_hi_opt = ctx.solver.plot_x_max.filter(|v| v.is_finite()).or(if data_x_max.is_finite() { Some(data_x_max) } else { None });
    let (x_min, x_max) = match (x_lo_opt, x_hi_opt) {
        (Some(lo), Some(hi)) if (hi - lo).abs() >= 1e-15 => (lo.min(hi), lo.max(hi)),
        (Some(lo), Some(_)) => (lo - 1.0, lo + 1.0),
        (Some(lo), None) => (lo, lo + 2.0),
        (None, Some(hi)) => (hi - 2.0, hi),
        (None, None) => (-1.0, 1.0),
    };

    let use_log = x_min > 0.0 && x_max > x_min && (x_max / x_min) >= 100.0;
    let log_min = if use_log { x_min.ln() } else { 0.0 };
    let log_span = if use_log { x_max.ln() - log_min } else { 0.0 };

    let n_grid = ctx.solver.curve_overlay_segments.max(2);
    let mut points = Vec::with_capacity(n_grid);
    let k = ctx.layout.k_free();
    let p_m = ctx.layout.p_model;
    let mut grad_buf = vec![0.0_f64; p_m];
    let mut g = vec![0.0_f64; k];
    let mut work_theta = ctx.theta.to_vec();
    let mut work_params = vec![0.0_f64; p_m];

    let active_cov =
        if ctx.solver.method == FitMethod::RobustCauchy || ctx.solver.plot_function_only || !ctx.solver.compute_bands || !ctx.t_crit.is_finite() {
            None
        } else {
            ctx.cov_unscaled
        };

    let has_explicit_weights = ctx.ds.points.iter().any(|pt| pt.explicit_weight.is_some());
    let model_ds_idx = model_dataset_index(ctx.model, ctx.ds_local_idx, ctx.ds.data_set_index);

    for i in 0..n_grid {
        let frac = (i as f64) / ((n_grid - 1) as f64);
        let x = if use_log { (log_min + frac * log_span).exp().clamp(x_min, x_max) } else { x_min + frac * (x_max - x_min) };
        let Some(y) = ctx.model.eval(&[x], ctx.ds_params, model_ds_idx) else {
            continue;
        };
        if !y.is_finite() {
            continue;
        }
        let (band_lo, band_hi, pred_lo, pred_hi) = if let Some(cov_u) = active_cov {
            grad_buf.fill(0.0);
            g.fill(0.0);
            let mut grad_ok = false;
            if ctx.model.grad(&[x], ctx.ds_params, model_ds_idx, &mut grad_buf) {
                grad_ok = true;
                for (p_idx, &gv) in grad_buf.iter().enumerate().take(p_m) {
                    if let ParamBinding::Free(slot) = ctx.layout.bindings[ctx.ds_local_idx][p_idx] {
                        let next_g = g[slot] + gv;
                        if !gv.is_finite() || !next_g.is_finite() {
                            grad_ok = false;
                            break;
                        }
                        g[slot] = next_g;
                    }
                }
            }
            if !grad_ok {
                grad_ok = delta_grad_at_x_into(
                    ctx.model,
                    ctx.layout,
                    ctx.theta,
                    ctx.ds_local_idx,
                    model_ds_idx,
                    x,
                    Some(y),
                    &mut work_theta,
                    &mut work_params,
                    &mut g,
                );
            }
            let raw_quad = if grad_ok { quadratic_form(cov_u, &g) } else { f64::NAN };
            if raw_quad.is_finite() && ctx.sigma.is_finite() {
                let quad = raw_quad.max(0.0);
                let se_f = ctx.sigma * quad.sqrt();
                let b_lo = (y - ctx.t_crit * se_f).is_finite().then_some(y - ctx.t_crit * se_f);
                let b_hi = (y + ctx.t_crit * se_f).is_finite().then_some(y + ctx.t_crit * se_f);
                let (p_lo, p_hi) = if ctx.solver.compute_prediction_bands {
                    if ctx.solver.method == FitMethod::PoissonDeviance {
                        let se_p = (y.max(0.0) + ctx.sigma * ctx.sigma * quad).sqrt();
                        (Some(y - ctx.t_crit * se_p), Some(y + ctx.t_crit * se_p))
                    } else if let Some(inv_w) = new_observation_inv_weight(ctx.solver.weighting, x, y, has_explicit_weights) {
                        let se_p = ctx.sigma * (inv_w + quad).sqrt();
                        (
                            (y - ctx.t_crit * se_p).is_finite().then_some(y - ctx.t_crit * se_p),
                            (y + ctx.t_crit * se_p).is_finite().then_some(y + ctx.t_crit * se_p),
                        )
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };
                (b_lo, b_hi, p_lo, p_hi)
            } else {
                (None, None, None, None)
            }
        } else {
            (None, None, None, None)
        };
        points.push(CurveSamplePoint { x, y, ci_lower: band_lo, ci_upper: band_hi, pred_lower: pred_lo, pred_upper: pred_hi });
    }

    FittedCurveOverlay {
        data_set_index: ctx.ds.data_set_index,
        equation_id: ctx.model.id().to_owned(),
        converged: ctx.converged,
        ambiguous: ctx.ambiguous,
        points,
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::solver) fn delta_grad_at_x_into(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_local_idx: usize,
    model_ds_idx: usize,
    x: f64,
    y0: Option<f64>,
    work: &mut [f64],
    work_params: &mut [f64],
    g_out: &mut [f64],
) -> bool {
    let k = layout.k_free();
    g_out.fill(0.0);
    work.copy_from_slice(theta);
    let mut cached_y0 = y0.filter(|v| v.is_finite());
    for slot in 0..k {
        let info = &layout.free_slots[slot];
        if let Some(local_d) = info.ds_local_idx
            && local_d != ds_local_idx
        {
            continue;
        }
        let xj = theta[slot];
        let (lb, ub) = (info.lower, info.upper);
        let h_c = fd_step_size(xj, 1e-6, lb, ub);
        let in_ub_c = h_c > 0.0 && ub.is_none_or(|u| xj + h_c <= u);
        let in_lb_c = h_c > 0.0 && lb.is_none_or(|l| xj - h_c >= l);

        let fp = if in_ub_c {
            work[slot] = xj + h_c;
            layout.expand_params_into(work, ds_local_idx, work_params);
            model.eval(&[x], work_params, model_ds_idx).filter(|v| v.is_finite())
        } else {
            None
        };
        let fm = if in_lb_c {
            work[slot] = xj - h_c;
            layout.expand_params_into(work, ds_local_idx, work_params);
            model.eval(&[x], work_params, model_ds_idx).filter(|v| v.is_finite())
        } else {
            None
        };
        work[slot] = xj;

        if let (Some(p), Some(m)) = (fp, fm) {
            let deriv = (p - m) / (2.0 * h_c);
            if !deriv.is_finite() {
                return false;
            }
            g_out[slot] = deriv;
            continue;
        }

        let base_y = match cached_y0 {
            Some(v) => v,
            None => {
                layout.expand_params_into(theta, ds_local_idx, work_params);
                let Some(v) = model.eval(&[x], work_params, model_ds_idx).filter(|v| v.is_finite()) else {
                    return false;
                };
                cached_y0 = Some(v);
                v
            }
        };

        let h_f = fd_step_size(xj, 1.5e-8, lb, ub);
        let signs = if (fp.is_none() && fm.is_some()) || ub.is_some_and(|u| xj + h_f > u) { [-1.0_f64, 1.0_f64] } else { [1.0_f64, -1.0_f64] };
        let mut set = false;
        for &sign in &signs {
            let x_try = xj + sign * h_f;
            if h_f <= 0.0 || lb.is_some_and(|l| x_try < l) || ub.is_some_and(|u| x_try > u) {
                continue;
            }
            work[slot] = x_try;
            layout.expand_params_into(work, ds_local_idx, work_params);
            let f_try = model.eval(&[x], work_params, model_ds_idx).filter(|v| v.is_finite());
            work[slot] = xj;
            if let Some(val) = f_try {
                let deriv = (val - base_y) * (sign / h_f);
                if deriv.is_finite() {
                    g_out[slot] = deriv;
                    set = true;
                    break;
                }
            }
        }
        if !set {
            let fallback = if let Some(p) = fp {
                (p - base_y) / h_c
            } else if let Some(m) = fm {
                (base_y - m) / h_c
            } else {
                f64::NAN
            };
            if !fallback.is_finite() {
                return false;
            }
            g_out[slot] = fallback;
        }
    }
    true
}

pub(in crate::solver) fn delta_grad_at_x(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_params: &[f64],
    ds_local_idx: usize,
    ds: &DatasetSeries,
    x: f64,
) -> Option<Vec<f64>> {
    let k = layout.k_free();
    let p_m = layout.p_model;
    let model_ds_idx = model_dataset_index(model, ds_local_idx, ds.data_set_index);
    let mut g = vec![0.0_f64; k];
    let mut grad_buf = vec![0.0_f64; p_m];
    if model.grad(&[x], ds_params, model_ds_idx, &mut grad_buf) {
        let mut ok = true;
        for (p_idx, &gv) in grad_buf.iter().enumerate().take(p_m) {
            if let ParamBinding::Free(slot) = layout.bindings[ds_local_idx][p_idx] {
                let next_g = g[slot] + gv;
                if !gv.is_finite() || !next_g.is_finite() {
                    ok = false;
                    break;
                }
                g[slot] = next_g;
            }
        }
        if ok {
            return Some(g);
        }
        g.fill(0.0);
    }
    let mut work = theta.to_vec();
    let mut work_params = vec![0.0_f64; p_m];
    delta_grad_at_x_into(model, layout, theta, ds_local_idx, model_ds_idx, x, None, &mut work, &mut work_params, &mut g).then_some(g)
}

pub(in crate::solver) fn quadratic_form(m: &DenseMatrix, v: &[f64]) -> f64 {
    let n = v.len().min(m.nrows()).min(m.ncols());
    let mut acc = NeumaierSum::new();
    for i in 0..n {
        for j in 0..n {
            acc.add(v[i] * mat_get(m, i, j) * v[j]);
        }
    }
    acc.total()
}

pub(in crate::solver) struct DatasetXBounds {
    pub(in crate::solver) real_bounds: Option<(f64, f64)>,
    pub(in crate::solver) single_x: Option<f64>,
    pub(in crate::solver) scan_x_min: f64,
    pub(in crate::solver) scan_x_max: f64,
    pub(in crate::solver) span: f64,
}

pub(in crate::solver) fn dataset_x_bounds(ds: &DatasetSeries) -> DatasetXBounds {
    let mut d_min = f64::INFINITY;
    let mut d_max = f64::NEG_INFINITY;
    for pt in &ds.points {
        let xv = pt.x0();
        if xv.is_finite() {
            d_min = d_min.min(xv);
            d_max = d_max.max(xv);
        }
    }
    if d_min.is_finite() && d_max.is_finite() && (d_max - d_min).abs() > 0.0 {
        DatasetXBounds { real_bounds: Some((d_min, d_max)), single_x: None, scan_x_min: d_min, scan_x_max: d_max, span: (d_max - d_min).abs() }
    } else if d_min.is_finite() {
        let half = d_min.abs().max(1.0);
        DatasetXBounds { real_bounds: None, single_x: Some(d_min), scan_x_min: d_min - half, scan_x_max: d_min + half, span: 2.0 * half }
    } else {
        DatasetXBounds { real_bounds: None, single_x: None, scan_x_min: -10.0, scan_x_max: 10.0, span: 20.0 }
    }
}

pub(in crate::solver) fn extrapolation_note_for_x(bounds: &DatasetXBounds, x_val: f64, plateau_hint: Option<&'static str>) -> Option<String> {
    let prefix = plateau_hint.map(|tag| format!("{tag}: ")).unwrap_or_default();
    match (bounds.real_bounds, bounds.single_x) {
        (Some((d_min, d_max)), _) => {
            let tol = 1e-9 * bounds.span.max(d_min.abs().max(d_max.abs())).max(f64::MIN_POSITIVE);
            if x_val < d_min - tol {
                Some(format!("{prefix}Extrapolated below measured standard X range [{d_min}, {d_max}]"))
            } else if x_val > d_max + tol {
                Some(format!("{prefix}Extrapolated above measured standard X range [{d_min}, {d_max}]"))
            } else {
                None
            }
        }
        (None, Some(x0)) => {
            if (x_val - x0).abs() > 1e-9 * x0.abs().max(f64::MIN_POSITIVE) {
                Some(format!("{prefix}Extrapolated outside single-X standard data"))
            } else {
                None
            }
        }
        (None, None) => Some(format!("{prefix}Extrapolated without finite standard X range")),
    }
}

pub(in crate::solver) fn plateau_tag_for_target(
    model: &dyn ParametricModel,
    ds_params: &[f64],
    ds_index: usize,
    y_target: f64,
    x_a: f64,
    x_b: f64,
) -> Option<&'static str> {
    let ya = model.eval(&[x_a], ds_params, ds_index)?;
    let yb = model.eval(&[x_b], ds_params, ds_index)?;
    if !ya.is_finite() || !yb.is_finite() {
        return None;
    }
    let y_lo = ya.min(yb);
    let y_hi = ya.max(yb);
    if y_target > y_hi + 1e-12 {
        Some("AboveTopPlateau")
    } else if y_target < y_lo - 1e-12 {
        Some("BelowBottomPlateau")
    } else {
        None
    }
}

/// Bracket the inverse confidence/prediction interval around `x_hat` by finding the nearest
/// band crossing in `[scan_lo, x_hat]` for the lower bound and the nearest band crossing in
/// `[x_hat, scan_hi]` for the upper bound, guaranteeing `lower <= x_hat <= upper` even on
/// non-monotone curves (`AUD-03-11`).
pub(in crate::solver) fn band_crossing_interval<F1, F2>(
    band_lo_fn: F1,
    band_hi_fn: F2,
    x_hat: f64,
    scan_lo: f64,
    scan_hi: f64,
    y_scale: f64,
) -> (CiBound, CiBound)
where
    F1: Fn(f64) -> f64,
    F2: Fn(f64) -> f64,
{
    let nearest_of = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(va), Some(vb)) => Some(if (va - x_hat).abs() <= (vb - x_hat).abs() { va } else { vb }),
        (Some(va), None) => Some(va),
        (None, Some(vb)) => Some(vb),
        (None, None) => None,
    };
    let lower = if scan_lo < x_hat {
        let l1 = find_nearest_root(&band_lo_fn, x_hat, scan_lo, x_hat, y_scale);
        let l2 = find_nearest_root(&band_hi_fn, x_hat, scan_lo, x_hat, y_scale);
        nearest_of(l1, l2).map_or(CiBound::Open, CiBound::Finite)
    } else {
        CiBound::Open
    };
    let upper = if x_hat < scan_hi {
        let r1 = find_nearest_root(&band_lo_fn, x_hat, x_hat, scan_hi, y_scale);
        let r2 = find_nearest_root(&band_hi_fn, x_hat, x_hat, scan_hi, y_scale);
        nearest_of(r1, r2).map_or(CiBound::Open, CiBound::Finite)
    } else {
        CiBound::Open
    };
    (lower, upper)
}

#[derive(Clone, Copy)]
pub(in crate::solver) struct InverseTarget {
    pub(in crate::solver) response: UnknownResponse,
    pub(in crate::solver) replicate_mean_count: Option<NonZeroUsize>,
}

/// Invert the average response of a replicate row once and also retain each active subcolumn.
/// Seber & Wild (1989) describe inverse calibration from mean future measurements.
pub(in crate::solver) fn inverse_targets(responses: &[UnknownResponse]) -> impl Iterator<Item = InverseTarget> + '_ {
    let mut by_row: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    for &response in responses {
        if let UnknownResponse::Replicate { row, value, .. } = response {
            by_row.entry(row).or_default().push(value);
        }
    }
    let means = by_row.into_iter().filter_map(|(row, values)| {
        let count = NonZeroUsize::new(values.len())?;
        let mut sum = NeumaierSum::new();
        for value in values {
            sum.add(value / count.get() as f64);
        }
        Some(InverseTarget { response: UnknownResponse::Single((row, sum.total())), replicate_mean_count: Some(count) })
    });
    means.chain(responses.iter().copied().map(|response| InverseTarget { response, replicate_mean_count: None }))
}

/// Interpolate unknown `X` from target `Y` along the fitted curve using scan + `brent_root`,
/// preferring roots within measured `[x_min, x_max]`, flagging out-of-range extrapolation and
/// plateau violations (`AboveTopPlateau` / `BelowBottomPlateau`), and returning both:
/// - `ci_lower..ci_upper`: inverse **mean-response** confidence interval (treats `y_target` as exact)
/// - `pred_lower..pred_upper`: inverse **calibration / prediction** interval (includes new-observation error variance, Seber & Wild 1989).
#[allow(clippy::too_many_arguments)]
pub(in crate::solver) fn interpolate_x_from_y(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_params: &[f64],
    ds: &DatasetSeries,
    ds_local_idx: usize,
    target: InverseTarget,
    cov_unscaled: Option<&DenseMatrix>,
    sigma: f64,
    t_crit: f64,
    inference_availability: InferenceAvailability,
    solver: &SolverConfig,
) -> InterpolatedUnknown {
    let (row, replicate_subcolumn) = match target.response {
        UnknownResponse::Single((row, _)) => (row, None),
        UnknownResponse::Replicate { row, subcol, .. } => (row, Some(subcol)),
    };
    let y_target = target.response.value();
    let observations = target.replicate_mean_count.map_or(1, NonZeroUsize::get) as f64;
    let has_explicit_weights = ds.points.iter().any(|pt| pt.explicit_weight.is_some());
    let model_ds_idx = model_dataset_index(model, ds_local_idx, ds.data_set_index);
    let xb = dataset_x_bounds(ds);
    let y_scale = ds.points.iter().map(|p| p.y.abs()).fold(y_target.abs(), f64::max).max(1e-12);
    let scan_lo = xb.scan_x_min - 0.5 * xb.span;
    let scan_hi = xb.scan_x_max + 0.5 * xb.span;

    let f_diff = |x: f64| -> f64 { model.eval(&[x], ds_params, model_ds_idx).filter(|y| y.is_finite()).map_or(f64::NAN, |y| y - y_target) };
    let (in_range_root, in_range_flips) = scan_interval_first_root_and_crossings(&f_diff, xb.scan_x_min, xb.scan_x_max, y_scale);

    let x_hat_opt = in_range_root.or_else(|| {
        let (lo_root, _) = scan_interval_first_root_and_crossings(&f_diff, scan_lo, xb.scan_x_min, y_scale);
        let (hi_root, _) = scan_interval_first_root_and_crossings(&f_diff, xb.scan_x_max, scan_hi, y_scale);
        match (lo_root, hi_root) {
            (Some(rl), Some(rh)) => {
                if (xb.scan_x_min - rl).abs() <= (rh - xb.scan_x_max).abs() {
                    Some(rl)
                } else {
                    Some(rh)
                }
            }
            (Some(rl), None) => Some(rl),
            (None, Some(rh)) => Some(rh),
            (None, None) => None,
        }
    });

    let Some(x_hat) = x_hat_opt else {
        let mut y_min = f64::INFINITY;
        let mut y_max = f64::NEG_INFINITY;
        for x in build_scan_grid(scan_lo, scan_hi, None) {
            if let Some(y) = model.eval(&[x], ds_params, model_ds_idx)
                && y.is_finite()
            {
                y_min = y_min.min(y);
                y_max = y_max.max(y);
            }
        }
        let note = if y_min.is_finite() && y_max.is_finite() {
            if y_target > y_max {
                "AboveTopPlateau: Target Y falls outside fitted curve range".to_owned()
            } else if y_target < y_min {
                "BelowBottomPlateau: Target Y falls outside fitted curve range".to_owned()
            } else {
                "Target Y falls outside fitted curve range".to_owned()
            }
        } else {
            "Target Y falls outside fitted curve range".to_owned()
        };
        return InterpolatedUnknown {
            data_set_index: ds.data_set_index,
            row,
            replicate_subcolumn,
            replicate_mean_count: target.replicate_mean_count,
            input_value: y_target,
            output_estimate: f64::NAN,
            ci_lower: CiBound::Open,
            ci_upper: CiBound::Open,
            pred_lower: CiBound::Open,
            pred_upper: CiBound::Open,
            note: Some(note),
            inference_availability,
        };
    };

    let ((ci_lower, ci_upper), (pred_lower, pred_upper)) = if inference_availability.is_available()
        && let Some(cov_u) = cov_unscaled
        && t_crit.is_finite()
        && sigma.is_finite()
    {
        let band_lo = scan_lo.min(x_hat - xb.span);
        let band_hi = scan_hi.max(x_hat + xb.span);
        let eval_mean_se = |x: f64| -> Option<(f64, f64)> {
            let y = model.eval(&[x], ds_params, model_ds_idx).filter(|v| v.is_finite())?;
            let g = delta_grad_at_x(model, layout, theta, ds_params, ds_local_idx, ds, x)?;
            let raw_quad = quadratic_form(cov_u, &g);
            if !raw_quad.is_finite() {
                return None;
            }
            Some((y, raw_quad.max(0.0)))
        };
        let mean_lo_fn = |x: f64| -> f64 {
            let Some((y, quad)) = eval_mean_se(x) else {
                return f64::NAN;
            };
            (y - t_crit * sigma * quad.sqrt()) - y_target
        };
        let mean_hi_fn = |x: f64| -> f64 {
            let Some((y, quad)) = eval_mean_se(x) else {
                return f64::NAN;
            };
            (y + t_crit * sigma * quad.sqrt()) - y_target
        };
        let has_pred_variance = solver.method == FitMethod::PoissonDeviance
            || new_observation_inv_weight(solver.weighting, x_hat, y_target, has_explicit_weights).is_some();
        let pred_bounds = if has_pred_variance {
            let pred_lo_fn = |x: f64| -> f64 {
                let Some((y, quad)) = eval_mean_se(x) else {
                    return f64::NAN;
                };
                let se_p = if solver.method == FitMethod::PoissonDeviance {
                    (y.max(0.0) / observations + sigma * sigma * quad).sqrt()
                } else {
                    let Some(inv_w_raw) = new_observation_inv_weight(solver.weighting, x, y, has_explicit_weights) else {
                        return f64::NAN;
                    };
                    sigma * (inv_w_raw / observations + quad).sqrt()
                };
                (y - t_crit * se_p) - y_target
            };
            let pred_hi_fn = |x: f64| -> f64 {
                let Some((y, quad)) = eval_mean_se(x) else {
                    return f64::NAN;
                };
                let se_p = if solver.method == FitMethod::PoissonDeviance {
                    (y.max(0.0) / observations + sigma * sigma * quad).sqrt()
                } else {
                    let Some(inv_w_raw) = new_observation_inv_weight(solver.weighting, x, y, has_explicit_weights) else {
                        return f64::NAN;
                    };
                    sigma * (inv_w_raw / observations + quad).sqrt()
                };
                (y + t_crit * se_p) - y_target
            };
            band_crossing_interval(pred_lo_fn, pred_hi_fn, x_hat, band_lo, band_hi, y_scale)
        } else {
            (CiBound::Open, CiBound::Open)
        };
        (band_crossing_interval(mean_lo_fn, mean_hi_fn, x_hat, band_lo, band_hi, y_scale), pred_bounds)
    } else {
        ((CiBound::Open, CiBound::Open), (CiBound::Open, CiBound::Open))
    };

    let plateau_hint = plateau_tag_for_target(model, ds_params, model_ds_idx, y_target, xb.scan_x_min, xb.scan_x_max);
    let mut note = extrapolation_note_for_x(&xb, x_hat, plateau_hint);
    if note.is_none() && in_range_flips > 1 {
        note = Some("Non-monotone fitted curve: multiple X roots exist inside measured standard range".to_owned());
    }

    InterpolatedUnknown {
        data_set_index: ds.data_set_index,
        row,
        replicate_subcolumn,
        replicate_mean_count: target.replicate_mean_count,
        input_value: y_target,
        output_estimate: x_hat,
        ci_lower,
        ci_upper,
        pred_lower,
        pred_upper,
        note,
        inference_availability,
    }
}

/// Predict `Y` at `x_val` with both the pointwise delta-method mean-response confidence interval
/// (Bates & Watts 1988) and the single-observation prediction interval.
#[allow(clippy::too_many_arguments)]
pub(in crate::solver) fn interpolate_y_from_x(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_params: &[f64],
    ds: &DatasetSeries,
    ds_local_idx: usize,
    row: usize,
    x_val: f64,
    cov_unscaled: Option<&DenseMatrix>,
    sigma: f64,
    t_crit: f64,
    inference_availability: InferenceAvailability,
    solver: &SolverConfig,
) -> InterpolatedUnknown {
    let xb = dataset_x_bounds(ds);
    let has_explicit_weights = ds.points.iter().any(|pt| pt.explicit_weight.is_some());
    let model_ds_idx = model_dataset_index(model, ds_local_idx, ds.data_set_index);
    let Some(y_hat) = model.eval(&[x_val], ds_params, model_ds_idx).filter(|y| y.is_finite()) else {
        return InterpolatedUnknown {
            data_set_index: ds.data_set_index,
            row,
            replicate_subcolumn: None,
            replicate_mean_count: None,
            input_value: x_val,
            output_estimate: f64::NAN,
            ci_lower: CiBound::Open,
            ci_upper: CiBound::Open,
            pred_lower: CiBound::Open,
            pred_upper: CiBound::Open,
            note: Some("X outside fitted model domain".to_owned()),
            inference_availability,
        };
    };
    let ((ci_lower, ci_upper), (pred_lower, pred_upper)) = if inference_availability.is_available()
        && let Some(cov_u) = cov_unscaled
        && t_crit.is_finite()
        && sigma.is_finite()
        && let Some(g) = delta_grad_at_x(model, layout, theta, ds_params, ds_local_idx, ds, x_val)
    {
        let raw_quad = quadratic_form(cov_u, &g);
        if raw_quad.is_finite() {
            let quad = raw_quad.max(0.0);
            let se = sigma * quad.sqrt();
            let mean_ci = if se.is_finite() {
                (CiBound::Finite(y_hat - t_crit * se), CiBound::Finite(y_hat + t_crit * se))
            } else {
                (CiBound::Open, CiBound::Open)
            };
            let pred_pi = if solver.method == FitMethod::PoissonDeviance {
                let se_p = (y_hat.max(0.0) + sigma * sigma * quad).sqrt();
                if se_p.is_finite() {
                    (CiBound::Finite(y_hat - t_crit * se_p), CiBound::Finite(y_hat + t_crit * se_p))
                } else {
                    (CiBound::Open, CiBound::Open)
                }
            } else if let Some(inv_w) = new_observation_inv_weight(solver.weighting, x_val, y_hat, has_explicit_weights) {
                let se_p = sigma * (inv_w + quad).sqrt();
                if se_p.is_finite() {
                    (CiBound::Finite(y_hat - t_crit * se_p), CiBound::Finite(y_hat + t_crit * se_p))
                } else {
                    (CiBound::Open, CiBound::Open)
                }
            } else {
                (CiBound::Open, CiBound::Open)
            };
            (mean_ci, pred_pi)
        } else {
            ((CiBound::Open, CiBound::Open), (CiBound::Open, CiBound::Open))
        }
    } else {
        ((CiBound::Open, CiBound::Open), (CiBound::Open, CiBound::Open))
    };
    let note = extrapolation_note_for_x(&xb, x_val, None);
    InterpolatedUnknown {
        data_set_index: ds.data_set_index,
        row,
        replicate_subcolumn: None,
        replicate_mean_count: None,
        input_value: x_val,
        output_estimate: y_hat,
        ci_lower,
        ci_upper,
        pred_lower,
        pred_upper,
        note,
        inference_availability,
    }
}

/// Build an ordered scan grid combining linear steps with multi-decade log-spaced samples on
/// positive ranges (and an optional anchor point such as `x_hat`) so inverse interpolation never
/// skips narrow positive concentration decades (`AUD-03-11`).
fn build_scan_grid(lo: f64, hi: f64, anchor: Option<f64>) -> Vec<f64> {
    if !lo.is_finite() || !hi.is_finite() || hi < lo {
        return Vec::new();
    }
    if hi == lo {
        return vec![lo];
    }
    let linear_steps = 200_usize;
    let mut grid = Vec::with_capacity(420);
    for i in 0..=linear_steps {
        let x = lo + (i as f64) / (linear_steps as f64) * (hi - lo);
        if x.is_finite() {
            grid.push(x);
        }
    }
    if hi > 0.0 {
        let pos_lo = if lo > 0.0 {
            lo
        } else {
            let base = anchor.filter(|&a| a > 0.0).map_or(hi * 1e-8, |a| (a * 1e-4).min(hi * 1e-6));
            base.max(f64::MIN_POSITIVE)
        };
        if pos_lo > 0.0 && hi / pos_lo >= 10.0 {
            let log_steps = 200_usize;
            let ln_lo = pos_lo.ln();
            let ln_span = hi.ln() - ln_lo;
            for i in 0..=log_steps {
                let x = (ln_lo + (i as f64) / (log_steps as f64) * ln_span).exp().clamp(pos_lo, hi);
                if x.is_finite() && x >= lo && x <= hi {
                    grid.push(x);
                }
            }
        }
    }
    if let Some(a) = anchor
        && a.is_finite()
        && a >= lo
        && a <= hi
    {
        grid.push(a);
    }
    grid.sort_by(f64::total_cmp);
    grid.dedup_by(|a, b| (*a - *b).abs() <= 1e-15 * a.abs().max(b.abs()).max(f64::MIN_POSITIVE));
    grid
}

fn verified_brent_root<F: Fn(f64) -> f64>(f: &F, a: f64, b: f64, abs_tol: f64, rel_tol: f64, y_tol: f64) -> Option<f64> {
    let root = brent_root(f, a, b, RootOptions { abs_tol, rel_tol, max_iters: 80 }).ok()?;
    if !root.is_finite() {
        return None;
    }
    // Reject vertical-asymptote poles where Brent stops on bracket width with huge |f(root)| (`AUD-03-11`).
    let residual = f(root);
    (residual.is_finite() && residual.abs() <= y_tol).then_some(root)
}

pub(in crate::solver) fn scan_interval_first_root_and_crossings<F: Fn(f64) -> f64>(f: &F, lo: f64, hi: f64, y_scale: f64) -> (Option<f64>, usize) {
    let grid = build_scan_grid(lo, hi, None);
    let Some(&first_x) = grid.first() else {
        return (None, 0);
    };
    let y_tol = 1e-6 * y_scale.max(1e-12);
    let zero_tol = 1e-13 * y_scale.max(f64::MIN_POSITIVE);
    let x_tol = (1e-11 * (hi - lo).abs().max(1e-12)).min(1e-11);
    let mut prev_x = first_x;
    let mut prev_y = f(prev_x);
    let mut first_root = if prev_y.is_finite() && prev_y.abs() <= zero_tol { Some(prev_x) } else { None };
    let mut sign_flips = usize::from(first_root.is_some());

    for &x in grid.iter().skip(1) {
        let y = f(x);
        if prev_y.is_finite() && y.is_finite() {
            if prev_y.abs() <= zero_tol {
                if first_root.is_none() {
                    first_root = Some(prev_x);
                }
            } else if prev_y * y <= 0.0
                && let Some(root) = verified_brent_root(f, prev_x, x, x_tol, 1e-11, y_tol)
            {
                sign_flips += 1;
                if first_root.is_none() {
                    first_root = Some(root);
                }
            }
        }
        prev_x = x;
        prev_y = y;
    }
    if first_root.is_none() && prev_y.is_finite() && prev_y.abs() <= zero_tol {
        first_root = Some(prev_x);
    }
    (first_root, sign_flips)
}

pub(in crate::solver) fn find_nearest_root<F: Fn(f64) -> f64>(f: F, anchor: f64, lo: f64, hi: f64, y_scale: f64) -> Option<f64> {
    let grid = build_scan_grid(lo, hi, Some(anchor));
    let &first_x = grid.first()?;
    let y_tol = 1e-6 * y_scale.max(1e-12);
    let x_tol = (1e-10 * (hi - lo).abs().max(1e-12)).min(1e-10);
    let mut best: Option<f64> = None;
    let mut prev_x = first_x;
    let mut prev_y = f(prev_x);
    if prev_y.is_finite() && prev_y.abs() <= 1e-13 * y_scale.max(f64::MIN_POSITIVE) {
        best = Some(prev_x);
    }
    for &x in grid.iter().skip(1) {
        let y = f(x);
        if prev_y.is_finite()
            && y.is_finite()
            && prev_y * y <= 0.0
            && let Some(root) = verified_brent_root(&f, prev_x, x, x_tol, 1e-10, y_tol)
            && best.is_none_or(|b| (root - anchor).abs() < (b - anchor).abs())
        {
            best = Some(root);
        }
        prev_x = x;
        prev_y = y;
    }
    best
}
