//! Per-dataset result assembly: parameter summaries, statistics, and diagnostics.
//!
//! References:
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.

use curvebook_doc::{CiBound, FitMethod};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::NeumaierSum;

use crate::diagnostics::{CovarianceDiagnostics, compute_rsdr, runs_test};
use crate::error::{FitIdentifiability, FitStatus, ParameterNote};
use crate::library::ParametricModel;
use crate::solver::curves::{CurveOverlayContext, build_curve_overlay, interpolate_x_from_y, interpolate_y_from_x, inverse_targets};
use crate::solver::inference::compute_r_squared;
use crate::solver::layout::{ObsRecord, ParamBinding, ParameterLayout, model_dataset_index};
use crate::solver::lm::{LmRunOutput, fd_step_size};
use crate::solver::results::{FittedDatasetResult, FittedParameterSummary, InferenceAvailability, InterpolatedUnknown, SolverConfig};
use crate::types::{DatasetSeries, DerivedScalar};

#[allow(clippy::too_many_arguments)]
pub(in crate::solver) fn build_fitted_parameter_summaries(
    layout: &ParameterLayout,
    theta: &[f64],
    cov: &CovarianceDiagnostics,
    hougaard: &[f64],
    profile_cis: &[(CiBound, CiBound)],
    unstable: &[bool],
    rel_bound_hits: &[bool],
    inference_availability: InferenceAvailability,
    report_all: bool,
) -> Vec<FittedParameterSummary> {
    let is_robust = inference_availability.is_not_applicable_in_robust_mode();
    let mut parameters = Vec::with_capacity(layout.k_free());
    for (idx, slot) in layout.free_slots.iter().enumerate() {
        let est = theta[idx];
        let mut notes = Vec::new();
        if slot.is_shared {
            notes.push(ParameterNote::SharedGlobal);
        }
        if slot.lower.is_some_and(|l| (est - l).abs() <= 1e-9 * est.abs().max(1.0)) || rel_bound_hits.get(idx).copied().unwrap_or(false) {
            notes.push(ParameterNote::HitLowerBound);
        }
        if slot.upper.is_some_and(|u| (u - est).abs() <= 1e-9 * est.abs().max(1.0)) {
            notes.push(ParameterNote::HitUpperBound);
        }
        if !report_all {
            if unstable.get(idx).copied().unwrap_or(false) {
                notes.push(ParameterNote::Unstable);
            } else if !is_robust && cov.dependencies.get(idx).copied().unwrap_or(0.0) > 0.9999 {
                notes.push(ParameterNote::Ambiguous);
            }
        }
        if !is_robust && (matches!(profile_cis[idx].0, CiBound::Open) || matches!(profile_cis[idx].1, CiBound::Open)) {
            notes.push(ParameterNote::UnboundedCi);
        }

        parameters.push(FittedParameterSummary {
            slot_index: idx,
            name: slot.name.clone(),
            dataset_index: slot.dataset_idx,
            estimate: est,
            se: if is_robust { f64::NAN } else { cov.standard_errors.get(idx).copied().unwrap_or(f64::NAN) },
            ci_lower: if is_robust { CiBound::Open } else { profile_cis[idx].0 },
            ci_upper: if is_robust { CiBound::Open } else { profile_cis[idx].1 },
            symmetric_ci_lower: if is_robust { f64::NAN } else { cov.symmetric_ci_lower.get(idx).copied().unwrap_or(f64::NAN) },
            symmetric_ci_upper: if is_robust { f64::NAN } else { cov.symmetric_ci_upper.get(idx).copied().unwrap_or(f64::NAN) },
            dependency: if is_robust { f64::NAN } else { cov.dependencies.get(idx).copied().unwrap_or(f64::NAN) },
            hougaard_g1: if is_robust { f64::NAN } else { hougaard.get(idx).copied().unwrap_or(f64::NAN) },
            notes,
            inference_availability,
        });
    }
    parameters
}

pub(in crate::solver) struct DatasetAssemblyConfig<'a> {
    pub(in crate::solver) cov_unscaled: Option<&'a DenseMatrix>,
    pub(in crate::solver) cov: &'a CovarianceDiagnostics,
    pub(in crate::solver) parameters: &'a [FittedParameterSummary],
    pub(in crate::solver) sigma_w: f64,
    pub(in crate::solver) t_crit: f64,
    pub(in crate::solver) rsd: f64,
    pub(in crate::solver) ambiguous: bool,
    pub(in crate::solver) solver: &'a SolverConfig,
}

/// Propagate standard errors and confidence intervals onto [`DerivedScalar`] transforms:
/// - **1-parameter monotonic transforms** (e.g. `10^LogEC50`, `10^(-pKB)`, `ln(2)/K`, `1/Wavelength`,
///   `2*Width`): delta-method SE `|dg/dtheta| * SE_theta` and exact endpoint transformation of the
///   parameter's active CI, reversing bounds whenever the derivative is negative (and omitting the
///   CI when the interval crosses a domain singularity or non-monotonic extremum).
/// - **Multi-parameter transforms** (e.g. `Span = Top - Bottom`, Logistic/Gompertz `InflectionPoint`,
///   multi-parameter DSL `[Transforms]`): first-order delta-method `SE = sqrt(g^T Cov g)` and
///   two-sided `est +/- t_crit * SE`.
/// - **Robust (Cauchy) mode (`R4.5`)**: suppresses `se`, `ci_lower`, and `ci_upper` (`None`).
fn lookup_finite_scalar(list: &[DerivedScalar], name: &str) -> Option<f64> {
    list.iter().find(|s| s.name == name).map(|s| s.value).filter(|v| v.is_finite())
}

struct DerivedEvalCtx<'a> {
    model: &'a dyn ParametricModel,
    layout: &'a ParameterLayout,
    theta: &'a [f64],
    ds_local_idx: usize,
    model_ds_idx: usize,
}

fn compute_derived_scalar_grads(ctx: &DerivedEvalCtx<'_>, scalars: &[DerivedScalar]) -> (Vec<Vec<f64>>, Vec<bool>) {
    let k = ctx.layout.k_free();
    let n_sc = scalars.len();
    let mut grads = vec![vec![0.0_f64; k]; n_sc];
    let mut grad_valid = vec![true; n_sc];
    let mut work_theta = ctx.theta.to_vec();
    let mut work_params = vec![0.0_f64; ctx.layout.p_model];

    for slot in 0..k {
        let info = &ctx.layout.free_slots[slot];
        if let Some(local_d) = info.ds_local_idx
            && local_d != ctx.ds_local_idx
        {
            continue;
        }
        let xj = ctx.theta[slot];
        let h_c = fd_step_size(xj, 1e-6, info.lower, info.upper);
        let in_ub = h_c > 0.0 && info.upper.is_none_or(|u| xj + h_c <= u);
        let in_lb = h_c > 0.0 && info.lower.is_none_or(|l| xj - h_c >= l);

        let plus_list = if in_ub {
            work_theta[slot] = xj + h_c;
            ctx.layout.expand_params_into(&work_theta, ctx.ds_local_idx, &mut work_params);
            Some(ctx.model.derived_transforms_for_dataset(&work_params, ctx.model_ds_idx))
        } else {
            None
        };
        let minus_list = if in_lb {
            work_theta[slot] = xj - h_c;
            ctx.layout.expand_params_into(&work_theta, ctx.ds_local_idx, &mut work_params);
            Some(ctx.model.derived_transforms_for_dataset(&work_params, ctx.model_ds_idx))
        } else {
            None
        };
        work_theta[slot] = xj;

        for (m, sc) in scalars.iter().enumerate() {
            if !sc.value.is_finite() {
                grad_valid[m] = false;
                continue;
            }
            let vp = plus_list.as_deref().and_then(|l| lookup_finite_scalar(l, &sc.name));
            let vm = minus_list.as_deref().and_then(|l| lookup_finite_scalar(l, &sc.name));
            if let (Some(p), Some(q)) = (vp, vm) {
                let d = (p - q) / (2.0 * h_c);
                if d.is_finite() {
                    grads[m][slot] = d;
                    continue;
                }
            }
            // One-sided fallback near bounds or domain boundaries.
            let h_f = fd_step_size(xj, 1.5e-8, info.lower, info.upper);
            let mut found = None;
            for sign in [1.0_f64, -1.0_f64] {
                let xt = xj + sign * h_f;
                if h_f <= 0.0 || info.lower.is_some_and(|l| xt < l) || info.upper.is_some_and(|u| xt > u) {
                    continue;
                }
                work_theta[slot] = xt;
                ctx.layout.expand_params_into(&work_theta, ctx.ds_local_idx, &mut work_params);
                let trial = ctx.model.derived_transforms_for_dataset(&work_params, ctx.model_ds_idx);
                work_theta[slot] = xj;
                if let Some(vt) = lookup_finite_scalar(&trial, &sc.name) {
                    let d = (vt - sc.value) / (sign * h_f);
                    if d.is_finite() {
                        found = Some(d);
                        break;
                    }
                }
            }
            if let Some(d) = found {
                grads[m][slot] = d;
            } else {
                grad_valid[m] = false;
            }
        }
    }
    (grads, grad_valid)
}

fn propagate_single_param_derived_ci(ctx: &DerivedEvalCtx<'_>, sc: &mut DerivedScalar, slot: usize, grad_s: f64, cfg: &DatasetAssemblyConfig<'_>) {
    let var_s = cfg.cov.covariance_matrix[slot][slot];
    if var_s >= 0.0 && var_s.is_finite() {
        let se = grad_s.abs() * var_s.sqrt();
        if se.is_finite() {
            sc.se = Some(se);
        }
    }
    let (CiBound::Finite(p_lo), CiBound::Finite(p_hi)) = (cfg.parameters[slot].ci_lower, cfg.parameters[slot].ci_upper) else {
        return;
    };
    if !p_lo.is_finite() || !p_hi.is_finite() || p_lo > p_hi {
        return;
    }
    let mut work_theta = ctx.theta.to_vec();
    let mut work_params = vec![0.0_f64; ctx.layout.p_model];
    let mut eval_at = |val: f64| -> Option<f64> {
        work_theta[slot] = val;
        ctx.layout.expand_params_into(&work_theta, ctx.ds_local_idx, &mut work_params);
        let list = ctx.model.derived_transforms_for_dataset(&work_params, ctx.model_ds_idx);
        work_theta[slot] = ctx.theta[slot];
        lookup_finite_scalar(&list, &sc.name)
    };
    // Verify strict monotonicity across [p_lo, p_hi] so singularities (e.g. K crossing 0) suppress CI.
    let mut sample_xs = [p_lo, p_lo + 0.25 * (p_hi - p_lo), 0.5 * (p_lo + p_hi), p_lo + 0.75 * (p_hi - p_lo), p_hi];
    sample_xs.sort_by(f64::total_cmp);
    let mut sample_ys = [0.0_f64; 5];
    for (dst, &sx) in sample_ys.iter_mut().zip(sample_xs.iter()) {
        let Some(sy) = eval_at(sx) else {
            return;
        };
        *dst = sy;
    }
    let monotone = if grad_s > 0.0 {
        sample_ys.windows(2).all(|w| w[1] >= w[0]) && sample_ys[4] >= sample_ys[0]
    } else {
        sample_ys.windows(2).all(|w| w[1] <= w[0]) && sample_ys[0] >= sample_ys[4]
    };
    if monotone {
        let (lo_v, hi_v) = if grad_s > 0.0 { (sample_ys[0], sample_ys[4]) } else { (sample_ys[4], sample_ys[0]) };
        if lo_v.is_finite() && hi_v.is_finite() && lo_v <= hi_v {
            sc.ci_lower = Some(lo_v);
            sc.ci_upper = Some(hi_v);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_derived_scalars_with_ci(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_params: &[f64],
    ds_local_idx: usize,
    model_ds_idx: usize,
    cfg: &DatasetAssemblyConfig<'_>,
) -> Vec<DerivedScalar> {
    let mut scalars = model.derived_transforms_for_dataset(ds_params, model_ds_idx);
    let is_robust = cfg.solver.method == FitMethod::RobustCauchy;
    let k = layout.k_free();
    if scalars.is_empty()
        || is_robust
        || cfg.ambiguous
        || !cfg.t_crit.is_finite()
        || k == 0
        || cfg.cov.covariance_matrix.len() < k
        || cfg.parameters.len() < k
    {
        return scalars;
    }

    let ctx = DerivedEvalCtx { model, layout, theta, ds_local_idx, model_ds_idx };
    let (grads, grad_valid) = compute_derived_scalar_grads(&ctx, &scalars);

    for (m, sc) in scalars.iter_mut().enumerate() {
        if !grad_valid[m] || !sc.value.is_finite() {
            continue;
        }
        let g = &grads[m];
        let mut weighted_contrib = vec![0.0_f64; k];
        let mut max_contrib = 0.0_f64;
        for s in 0..k {
            let gs = g[s].abs();
            if !gs.is_finite() || gs == 0.0 {
                continue;
            }
            let var_s = cfg.cov.covariance_matrix.get(s).and_then(|r| r.get(s)).copied().unwrap_or(0.0);
            let se_scale = if var_s.is_finite() && var_s > 0.0 { var_s.sqrt() } else { 0.0 };
            let param_scale = se_scale.max(theta[s].abs() * 1e-12).max(f64::MIN_POSITIVE);
            let contrib = gs * param_scale;
            if contrib.is_finite() {
                weighted_contrib[s] = contrib;
                if contrib > max_contrib {
                    max_contrib = contrib;
                }
            }
        }
        if !max_contrib.is_finite() || max_contrib == 0.0 {
            continue;
        }
        let sig_slots: Vec<usize> = (0..k).filter(|&s| g[s] != 0.0 && weighted_contrib[s] > 1e-12 * max_contrib).collect();
        if sig_slots.is_empty() {
            continue;
        }
        let cov_ok = sig_slots
            .iter()
            .all(|&a| sig_slots.iter().all(|&b| cfg.cov.covariance_matrix.get(a).and_then(|r| r.get(b)).is_some_and(|v| v.is_finite())));
        if !cov_ok {
            continue;
        }

        if sig_slots.len() == 1 {
            let s = sig_slots[0];
            propagate_single_param_derived_ci(&ctx, sc, s, g[s], cfg);
        } else {
            // Multi-parameter first-order delta method: Var(h) = g^T Cov g.
            let mut var_acc = NeumaierSum::new();
            for &a in &sig_slots {
                for &b in &sig_slots {
                    var_acc.add(g[a] * cfg.cov.covariance_matrix[a][b] * g[b]);
                }
            }
            let var_h = var_acc.total();
            if var_h.is_finite() && var_h >= 0.0 {
                let se = var_h.sqrt();
                if se.is_finite() {
                    sc.se = Some(se);
                    let lo = sc.value - cfg.t_crit * se;
                    let hi = sc.value + cfg.t_crit * se;
                    if lo.is_finite() && hi.is_finite() {
                        sc.ci_lower = Some(lo);
                        sc.ci_upper = Some(hi);
                    }
                }
            }
        }
    }

    scalars
}

pub(in crate::solver) struct AssembledDatasets {
    pub(in crate::solver) ds_results: Vec<FittedDatasetResult>,
    pub(in crate::solver) derived_scalars: Vec<Vec<DerivedScalar>>,
    pub(in crate::solver) interpolated_x: Vec<InterpolatedUnknown>,
    pub(in crate::solver) interpolated_y: Vec<InterpolatedUnknown>,
}

fn compute_slot_dataset_counts(layout: &ParameterLayout, obs: &[ObsRecord], n_datasets: usize) -> Vec<usize> {
    let k_free = layout.k_free();
    let mut active_ds = vec![false; n_datasets];
    for rec in obs {
        if let Some(flag) = active_ds.get_mut(rec.ds_local_idx) {
            *flag = true;
        }
    }
    let mut slot_ds_count = vec![0_usize; k_free];
    for (d_idx, row) in layout.bindings.iter().enumerate() {
        if !active_ds.get(d_idx).copied().unwrap_or(false) {
            continue;
        }
        let mut seen = vec![false; k_free];
        for b in row {
            if let ParamBinding::Free(slot) = *b
                && let Some(flag) = seen.get_mut(slot)
                && !*flag
            {
                *flag = true;
                slot_ds_count[slot] += 1;
            }
        }
    }
    slot_ds_count
}

fn effective_dataset_free_params(layout: &ParameterLayout, ds_local_idx: usize, slot_ds_count: &[usize]) -> f64 {
    let mut k_eff_d = 0.0_f64;
    let mut seen_d = vec![false; layout.k_free()];
    if let Some(row) = layout.bindings.get(ds_local_idx) {
        for b in row {
            if let ParamBinding::Free(slot) = *b
                && let Some(flag) = seen_d.get_mut(slot)
                && !*flag
            {
                *flag = true;
                let share_count = slot_ds_count.get(slot).copied().unwrap_or(1).max(1) as f64;
                k_eff_d += 1.0 / share_count;
            }
        }
    }
    k_eff_d
}

pub(in crate::solver) fn assemble_dataset_results(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    datasets: &[DatasetSeries],
    obs: &[ObsRecord],
    lm_out: &LmRunOutput,
    studentized_all: &[f64],
    cfg: &DatasetAssemblyConfig<'_>,
) -> AssembledDatasets {
    let slot_ds_count = compute_slot_dataset_counts(layout, obs, datasets.len());
    let mut ds_results = Vec::with_capacity(datasets.len());
    let mut derived_scalars = Vec::with_capacity(datasets.len());
    let mut interpolated_x = Vec::new();
    let mut interpolated_y = Vec::new();

    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        let ds_params = layout.expand_params(&lm_out.theta, ds_local_idx);
        let model_ds_idx = model_dataset_index(model, ds_local_idx, ds.data_set_index);
        derived_scalars.push(compute_derived_scalars_with_ci(model, layout, &lm_out.theta, &ds_params, ds_local_idx, model_ds_idx, cfg));

        let mut ds_y_hat = Vec::new();
        let mut ds_res = Vec::new();
        let mut ds_x_res: Vec<(f64, f64)> = Vec::new();
        let mut ds_base_w_abs_res = Vec::new();
        let mut ds_w_res = Vec::new();
        let mut ds_stud_res = Vec::new();
        let mut ds_rss = NeumaierSum::new();
        let mut ds_y_vals: Vec<(f64, usize, f64)> = Vec::new();
        let mut ds_n = 0_usize;
        let mut ds_y_sum = NeumaierSum::new();
        let mut scalar_x = true;
        let mut x_min = f64::INFINITY;
        let mut x_max = f64::NEG_INFINITY;
        for (i, rec) in obs.iter().enumerate() {
            if rec.ds_local_idx == ds_local_idx {
                match rec.point.x.as_slice() {
                    [x] if x.is_finite() => {
                        x_min = x_min.min(*x);
                        x_max = x_max.max(*x);
                    }
                    _ => scalar_x = false,
                }
                let yh = lm_out.y_hat[i];
                let r = rec.point.y - yh;
                let w_rep = rec.rep_n as f64;
                ds_y_hat.push(yh);
                ds_res.push(r);
                ds_x_res.push((rec.point.x0(), r));
                ds_base_w_abs_res.push((r * rec.sqrt_weight).abs());
                ds_w_res.push(lm_out.weighted_res[i]);
                if cfg.solver.method != FitMethod::RobustCauchy {
                    ds_stud_res.push(studentized_all.get(i).copied().unwrap_or(0.0));
                }
                ds_rss.add(w_rep * r * r);
                ds_rss.add(rec.raw_ss_within);
                ds_n += rec.rep_n;
                ds_y_sum.add(w_rep * rec.point.y);
                ds_y_vals.push((rec.point.y, rec.rep_n, rec.raw_ss_within));
            }
        }
        let ds_ss = ds_rss.total();
        let dtss = if ds_n > 1 {
            let m = ds_y_sum.total() / (ds_n as f64);
            let mut dtss_acc = NeumaierSum::new();
            for &(v, rep_n, ss_w) in &ds_y_vals {
                let d = v - m;
                dtss_acc.add((rep_n as f64) * d * d);
                dtss_acc.add(ss_w);
            }
            dtss_acc.total()
        } else {
            0.0
        };
        let ds_r2 = compute_r_squared(ds_ss, dtss, ds_n);
        let is_robust = cfg.solver.method == FitMethod::RobustCauchy;
        let inference_availability = if is_robust { InferenceAvailability::NotApplicableInRobustMode } else { InferenceAvailability::Available };
        let k_eff_d = effective_dataset_free_params(layout, ds_local_idx, &slot_ds_count);
        let ds_df = (ds_n as f64) - k_eff_d;
        let ds_sy_x = if is_robust {
            f64::NAN
        } else if ds_df > 1e-9 {
            (ds_ss / ds_df).sqrt()
        } else {
            cfg.rsd
        };
        let ds_rsdr = if is_robust {
            let n_d = ds_base_w_abs_res.len() as f64;
            if n_d > 0.0 {
                let p68 = compute_rsdr(&ds_base_w_abs_res, 0);
                Some(p68 * (n_d / (n_d - k_eff_d).max(1.0)))
            } else {
                Some(0.0)
            }
        } else {
            None
        };

        let converged_or_ambig = lm_out.status == FitStatus::Converged || lm_out.status == FitStatus::Ambiguous;
        let curve_overlay = (cfg.solver.curve_overlay_segments > 0).then(|| {
            build_curve_overlay(&CurveOverlayContext {
                model,
                layout,
                theta: &lm_out.theta,
                ds_params: &ds_params,
                ds,
                ds_local_idx,
                cov_unscaled: if is_robust { None } else { cfg.cov_unscaled },
                sigma: if is_robust { f64::NAN } else { cfg.sigma_w },
                t_crit: if is_robust { f64::NAN } else { cfg.t_crit },
                converged: converged_or_ambig,
                ambiguous: if is_robust { false } else { cfg.ambiguous },
                solver: cfg.solver,
            })
        });

        if cfg.solver.interpolate_unknowns {
            let interp_cov = if is_robust { None } else { cfg.cov_unscaled };
            let interp_sigma = if is_robust { f64::NAN } else { cfg.sigma_w };
            let interp_t = if is_robust { f64::NAN } else { cfg.t_crit };
            for target in inverse_targets(&ds.unknowns_y) {
                interpolated_x.push(interpolate_x_from_y(
                    model,
                    layout,
                    &lm_out.theta,
                    &ds_params,
                    ds,
                    ds_local_idx,
                    target,
                    interp_cov,
                    interp_sigma,
                    interp_t,
                    inference_availability,
                    cfg.solver,
                ));
            }
            for &(row, x_val) in &ds.unknowns_x {
                interpolated_y.push(interpolate_y_from_x(
                    model,
                    layout,
                    &lm_out.theta,
                    &ds_params,
                    ds,
                    ds_local_idx,
                    row,
                    x_val,
                    interp_cov,
                    interp_sigma,
                    interp_t,
                    inference_availability,
                    cfg.solver,
                ));
            }
        }

        let ds_runs_test = if cfg.solver.compute_runs_test {
            ds_x_res.sort_by(|a, b| a.0.total_cmp(&b.0));
            let ordered_ds_res: Vec<f64> = ds_x_res.into_iter().map(|(_, r)| r).collect();
            Some(runs_test(&ordered_ds_res))
        } else {
            None
        };

        let mut ds_w_ss_acc = NeumaierSum::new();
        let mut ds_ss_within = NeumaierSum::new();
        for i in 0..obs.len() {
            if obs[i].ds_local_idx == ds_local_idx {
                let wr = lm_out.weighted_res[i];
                ds_w_ss_acc.add(wr * wr);
                ds_ss_within.add(obs[i].ss_within);
            }
        }
        let ds_weighted_ss = ds_w_ss_acc.total() + ds_ss_within.total();

        ds_results.push(FittedDatasetResult {
            data_set_index: ds.data_set_index,
            label: ds.label.clone(),
            params: ds_params,
            param_names: model.param_names().to_vec(),
            observed_x_range: (scalar_x && x_min.is_finite() && x_max.is_finite()).then_some((x_min, x_max)),
            y_hat: ds_y_hat,
            residuals: ds_res,
            weighted_residuals: ds_w_res,
            studentized_residuals: ds_stud_res,
            ss_residual: ds_ss,
            r_squared: ds_r2,
            sy_x: ds_sy_x,
            rsdr: ds_rsdr,
            runs_test: ds_runs_test,
            status: lm_out.status,
            identifiability: FitIdentifiability::NotAssessed,
            ambiguous: cfg.ambiguous || lm_out.status == FitStatus::Ambiguous,
            iterations: lm_out.iterations,
            n_obs: ds_n,
            k_free: k_eff_d.max(0.0).round() as usize,
            df: ds_df.max(0.0).round() as usize,
            weighted_ss: ds_weighted_ss,
            aic_c: None,
            bic: None,
            normality: None,
            normality_anderson_darling: None,
            normality_lilliefors_ks: None,
            homoscedasticity: None,
            lack_of_fit: None,
            n_outliers: 0,
            predicted_y_neg_log_weight_sum: None,
            poisson_ll: None,
            curve_overlay,
        });
    }

    AssembledDatasets { ds_results, derived_scalars, interpolated_x, interpolated_y }
}
