//! Per-dataset result assembly: parameter summaries, statistics, and diagnostics.
//!
//! References:
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.

use curvebook_doc::{CiBound, FitMethod};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::NeumaierSum;

use crate::diagnostics::{CovarianceDiagnostics, compute_rsdr, runs_test};
use crate::error::{FitStatus, ParameterNote};
use crate::library::ParametricModel;
use crate::solver::curves::{CurveOverlayContext, build_curve_overlay, interpolate_x_from_y, interpolate_y_from_x, inverse_targets};
use crate::solver::layout::{ObsRecord, ParamBinding, ParameterLayout};
use crate::solver::lm::LmRunOutput;
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
        if slot.lower.is_some_and(|l| (est - l).abs() <= 1e-9 * est.abs().max(1.0)) {
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
    pub(in crate::solver) sigma_w: f64,
    pub(in crate::solver) t_crit: f64,
    pub(in crate::solver) rsd: f64,
    pub(in crate::solver) ambiguous: bool,
    pub(in crate::solver) solver: &'a SolverConfig,
}

pub(in crate::solver) struct AssembledDatasets {
    pub(in crate::solver) ds_results: Vec<FittedDatasetResult>,
    pub(in crate::solver) derived_scalars: Vec<Vec<DerivedScalar>>,
    pub(in crate::solver) interpolated_x: Vec<InterpolatedUnknown>,
    pub(in crate::solver) interpolated_y: Vec<InterpolatedUnknown>,
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
    let k_free = layout.k_free();
    let mut active_ds = vec![false; datasets.len()];
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
    let mut ds_results = Vec::with_capacity(datasets.len());
    let mut derived_scalars = Vec::with_capacity(datasets.len());
    let mut interpolated_x = Vec::new();
    let mut interpolated_y = Vec::new();

    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        let ds_params = layout.expand_params(&lm_out.theta, ds_local_idx);
        derived_scalars.push(model.derived_transforms(&ds_params));

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
        let ds_r2 = if ds_n > 1 {
            let m = ds_y_sum.total() / (ds_n as f64);
            let mut dtss_acc = NeumaierSum::new();
            for &(v, rep_n, ss_w) in &ds_y_vals {
                let d = v - m;
                dtss_acc.add((rep_n as f64) * d * d);
                dtss_acc.add(ss_w);
            }
            let dtss = dtss_acc.total();
            if dtss > 0.0 { 1.0 - ds_ss / dtss } else { 1.0 }
        } else {
            1.0
        };
        let is_robust = cfg.solver.method == FitMethod::RobustCauchy;
        let inference_availability = if is_robust { InferenceAvailability::NotApplicableInRobustMode } else { InferenceAvailability::Available };
        let mut k_eff_d = 0.0_f64;
        let mut seen_d = vec![false; k_free];
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
        let ds_df = (ds_n as f64) - k_eff_d;
        let ds_sy_x = if is_robust {
            f64::NAN
        } else if ds_df > 1e-9 {
            (ds_ss / ds_df).sqrt()
        } else {
            cfg.rsd
        };
        let ds_rsdr = if is_robust {
            let k_ds = layout.bindings[ds_local_idx].iter().filter(|b| matches!(b, ParamBinding::Free(_))).count();
            Some(compute_rsdr(&ds_base_w_abs_res, k_ds))
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
            curve_overlay,
        });
    }

    AssembledDatasets { ds_results, derived_scalars, interpolated_x, interpolated_y }
}
