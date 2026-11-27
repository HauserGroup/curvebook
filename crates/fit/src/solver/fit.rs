//! `fit_nonlinear` entry point, plot-only evaluation, method dispatch, and final uncertainty.
//!
//! References:
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.

use curvebook_doc::{CiBound, FitMethod, IdentifiabilityMode, OutlierHandling, ReplicateFitMode, WeightingScheme};
use curvebook_math::dist::{norm_two_sided_isf, student_t_two_sided_isf};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::NeumaierSum;

use crate::diagnostics::{
    CovarianceDiagnostics, RoutDetectionResult, compute_covariance_diagnostics, compute_deviance_profile_likelihood_cis_checked,
    compute_hougaard_skewness_into, compute_poisson_covariance_diagnostics, compute_profile_likelihood_cis_checked,
};
use crate::dsl::{DEFAULT_MAX_ODE_STEPS_PER_FIT, enter_ode_fit_budget};
use crate::error::{FitError, FitIdentifiability, FitStatus, ParameterNote};
use crate::library::ParametricModel;
use crate::solver::curves::{CurveOverlayContext, build_curve_overlay};
use crate::solver::datasets::{fit_independent_datasets, fit_single_or_shared};
use crate::solver::identifiability;
use crate::solver::identifiability::Assessment;
use crate::solver::inference::{
    compute_adj_r_squared, compute_r_squared, compute_rmse, rout_detect_outliers_mb2006, unscaled_cov_for_stable_subspace,
    unscaled_cov_from_diag_or_j, wald_ci_bounds,
};
use crate::solver::irls::{finalize_cauchy_weighted_residuals, fit_cauchy_irls, fit_poisson_irls, fit_predicted_y_irls};
use crate::solver::layout::{
    ObsRecord, ParamBinding, ParameterLayout, apply_predicted_y_weights, build_observations_with_source, model_dataset_index, obs_replicate_count,
    ode_budget_error, uses_replicate_summary_moments, uses_shared_global,
};
use crate::solver::lm::{LmRunOutput, eval_predictions_into, eval_residuals_with_domain, solve_with_warm_start};
use crate::solver::profile::{solve_profile_subproblem, solve_profile_subproblem_poisson};
use crate::solver::results::{FittedDatasetResult, FittedParameterSummary, InferenceAvailability, NonlinearFitResult, SolverConfig};
use crate::types::{DataPoint, DatasetSeries, HookContext, collapse_datasets_to_row_means, filter_datasets_by_range};

/// Execute nonlinear regression for one or more datasets using any built-in or DSL [`ParametricModel`].
///
/// Least squares is minimised by trust-region Levenberg–Marquardt (Levenberg 1944;
/// Marquardt 1963; Moré 1978). Inference follows the linearisation theory of Bates & Watts (1988)
/// and Seber & Wild (1989): `s^2 = SS_w / (N - K)`, `Cov = s^2 (J^T J)^{-1}`, symmetric `t`
/// intervals, profile-likelihood intervals, Hougaard (1985) skewness, and delta-method curve
/// bands. Goodness-of-fit, AICc/BIC, runs, replicates, and normality diagnostics are documented
/// in [`crate::diagnostics`].
pub fn fit_nonlinear(
    model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
) -> Result<NonlinearFitResult, FitError> {
    if datasets.is_empty() {
        return Err(FitError::EmptyInput);
    }
    let _ode_budget = enter_ode_fit_budget(DEFAULT_MAX_ODE_STEPS_PER_FIT.saturating_mul(datasets.len().clamp(1, 4)));
    if config.plot_function_only {
        return fit_plot_function_only(model, datasets, hooks, config);
    }

    let outlier_handling = config.effective_outlier_handling();
    if let FitMethod::RoutOutlierElimination { fdr_q_percent } = config.method
        && (!fdr_q_percent.is_finite() || fdr_q_percent <= 0.0 || fdr_q_percent > 50.0)
    {
        return Err(FitError::InvalidParameter { name: "fdr_q_percent", reason: format!("FDR Q% must lie in (0, 50], got {fdr_q_percent}") });
    }
    if (config.method == FitMethod::RobustCauchy || outlier_handling != OutlierHandling::None) && datasets.iter().any(|d| d.is_pre_summarized) {
        return Err(FitError::RequiresRawReplicates);
    }

    let mut transformed_storage: Option<Vec<DatasetSeries>> = None;
    if config.replicate_fit_mode == ReplicateFitMode::RowMeansOnly || config.x_range_filter.is_some() || config.y_range_filter.is_some() {
        let mut copy = datasets.to_vec();
        if config.replicate_fit_mode == ReplicateFitMode::RowMeansOnly {
            collapse_datasets_to_row_means(&mut copy);
        }
        if config.x_range_filter.is_some() || config.y_range_filter.is_some() {
            filter_datasets_by_range(&mut copy, config.x_range_filter, config.y_range_filter);
        }
        if copy.is_empty() {
            return Err(FitError::EmptyInput);
        }
        transformed_storage = Some(copy);
    }
    let active_datasets = transformed_storage.as_deref().unwrap_or(datasets);
    if config.method == FitMethod::PoissonDeviance {
        if config.weighting != WeightingScheme::None {
            return Err(FitError::InvalidParameter {
                name: "weighting",
                reason: "Poisson deviance regression defines variance Var(Y) = mu internally and cannot be combined with least-squares weighting schemes; use WeightingScheme::None".to_owned(),
            });
        }
        if active_datasets.iter().flat_map(|d| d.points.iter()).any(|pt| pt.explicit_weight.is_some()) {
            return Err(FitError::InvalidParameter {
                name: "weighting",
                reason: "Poisson deviance regression cannot be combined with explicit observation weights".to_owned(),
            });
        }
    }

    let layout = ParameterLayout::build_with_constraints(model, active_datasets, hooks, &config.parameter_rules, &config.relational_constraints)?;
    let k_free = layout.k_free();
    let use_rep_moments = uses_replicate_summary_moments(config);
    let obs = build_observations_with_source(active_datasets, config.weighting, config.weighting_source, use_rep_moments)?;
    let n_total_obs = obs_replicate_count(&obs);
    let min_required = if outlier_handling != OutlierHandling::None { k_free + 1 } else { k_free };
    if n_total_obs < min_required {
        return Err(FitError::InsufficientData { required: min_required, actual: n_total_obs });
    }

    if active_datasets.len() > 1 && !uses_shared_global(&config.parameter_rules) && !layout.free_slots.iter().any(|s| s.is_shared) {
        return fit_independent_datasets(model, active_datasets, hooks, config, layout, obs);
    }
    fit_single_or_shared(model, active_datasets, config, layout, obs)
}

pub(in crate::solver) fn fit_plot_function_only(
    model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
) -> Result<NonlinearFitResult, FitError> {
    let layout = ParameterLayout::build_with_constraints(model, datasets, hooks, &config.parameter_rules, &config.relational_constraints)?;
    let k_free = layout.k_free();
    let theta: Vec<f64> = layout.u_to_theta(&layout.u_init());
    let is_robust = config.method == FitMethod::RobustCauchy;
    let inference_availability = if is_robust { InferenceAvailability::NotApplicableInRobustMode } else { InferenceAvailability::Available };
    let mut parameters = Vec::with_capacity(k_free);
    for (idx, slot) in layout.free_slots.iter().enumerate() {
        parameters.push(FittedParameterSummary {
            slot_index: idx,
            name: slot.name.clone(),
            dataset_index: slot.dataset_idx,
            estimate: theta[idx],
            se: f64::NAN,
            ci_lower: CiBound::Open,
            ci_upper: CiBound::Open,
            symmetric_ci_lower: f64::NAN,
            symmetric_ci_upper: f64::NAN,
            dependency: if is_robust { f64::NAN } else { 0.0 },
            hougaard_g1: f64::NAN,
            notes: if is_robust { Vec::new() } else { vec![ParameterNote::UnboundedCi] },
            inference_availability,
        });
    }

    let mut slot_ds_count = vec![0_usize; k_free];
    for row in &layout.bindings {
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
    let mut ds_dfs = Vec::with_capacity(datasets.len());
    let mut derived_scalars = Vec::with_capacity(datasets.len());
    let mut total_obs = 0_usize;
    let mut raw_rss_acc = NeumaierSum::new();
    let mut y_vals_all = Vec::new();

    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        let ds_params = layout.expand_params(&theta, ds_local_idx);
        let eval_ds_idx = model_dataset_index(model, ds_local_idx, ds.data_set_index);
        derived_scalars.push(model.derived_transforms_for_dataset(&ds_params, eval_ds_idx));
        let mut ds_yh = Vec::with_capacity(ds.points.len());
        let mut ds_res = Vec::with_capacity(ds.points.len());
        let mut ds_y_vals = Vec::with_capacity(ds.points.len());
        let mut ds_rss = NeumaierSum::new();
        for pt in &ds.points {
            let Some(yh) = model.eval(&pt.x, &ds_params, eval_ds_idx) else {
                if let Some(err) = ode_budget_error() {
                    return Err(err);
                }
                continue;
            };
            if yh.is_finite() && pt.y.is_finite() {
                let r = pt.y - yh;
                ds_yh.push(yh);
                ds_res.push(r);
                ds_y_vals.push(pt.y);
                ds_rss.add(r * r);
                raw_rss_acc.add(r * r);
                y_vals_all.push(pt.y);
                total_obs += 1;
            }
        }
        let ds_ss = ds_rss.total();
        let dtss = if ds_y_vals.len() > 1 {
            let ym = ds_y_vals.iter().sum::<f64>() / (ds_y_vals.len() as f64);
            ds_y_vals.iter().map(|v| (v - ym) * (v - ym)).sum()
        } else {
            0.0
        };
        let ds_r2 = compute_r_squared(ds_ss, dtss, ds_y_vals.len());
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
        ds_dfs.push((ds_ss, (ds_y_vals.len() as f64) - k_eff_d));
        let overlay = build_curve_overlay(&CurveOverlayContext {
            model,
            layout: &layout,
            theta: &theta,
            ds_params: &ds_params,
            ds,
            ds_local_idx,
            cov_unscaled: None,
            sigma: 0.0,
            t_crit: f64::NAN,
            converged: true,
            ambiguous: false,
            solver: config,
        });
        ds_results.push(FittedDatasetResult {
            data_set_index: ds.data_set_index,
            label: ds.label.clone(),
            params: ds_params,
            param_names: model.param_names().to_vec(),
            observed_x_range: None,
            y_hat: ds_yh,
            residuals: ds_res.clone(),
            weighted_residuals: ds_res.clone(),
            studentized_residuals: if is_robust { Vec::new() } else { vec![0.0; ds_res.len()] },
            ss_residual: ds_ss,
            r_squared: ds_r2,
            sy_x: f64::NAN,
            rsdr: None,
            runs_test: None,
            status: FitStatus::Converged,
            identifiability: FitIdentifiability::NotAssessed,
            ambiguous: false,
            iterations: 0,
            n_obs: ds.points.len(),
            k_free: 0,
            df: ds.points.len(),
            weighted_ss: ds_ss,
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
            curve_overlay: Some(overlay),
        });
    }

    if total_obs == 0 && datasets.iter().any(|d| !d.points.is_empty()) {
        return Err(FitError::BadInitialValues {
            reason: "Initial/fixed parameters failed to evaluate finite predictions at any observed point".to_owned(),
        });
    }

    let raw_rss = raw_rss_acc.total();
    let df = total_obs.saturating_sub(k_free);
    let rsd = if df > 0 && !is_robust { (raw_rss / (df as f64)).sqrt() } else { f64::NAN };
    for (ds_res, &(ds_ss, ds_df)) in ds_results.iter_mut().zip(ds_dfs.iter()) {
        ds_res.sy_x = if is_robust {
            f64::NAN
        } else if ds_df > 1e-9 {
            (ds_ss / ds_df).sqrt()
        } else {
            rsd
        };
    }
    let tss = if y_vals_all.len() > 1 {
        let ym = y_vals_all.iter().sum::<f64>() / (y_vals_all.len() as f64);
        y_vals_all.iter().map(|v| (v - ym) * (v - ym)).sum()
    } else {
        0.0
    };
    let r_squared = compute_r_squared(raw_rss, tss, y_vals_all.len());
    let adj_r_squared = compute_adj_r_squared(r_squared, total_obs, df);
    let rmse = compute_rmse(raw_rss, total_obs);

    Ok(NonlinearFitResult {
        equation_id: model.id().to_owned(),
        equation_name: model.name().to_owned(),
        fit_method: config.method,
        inference_availability,
        status: FitStatus::Converged,
        identifiability: FitIdentifiability::NotAssessed,
        iterations: 0,
        n_obs: total_obs,
        n_outliers_excluded: 0,
        k_free,
        df,
        rss: raw_rss,
        weighted_ss: raw_rss,
        predicted_y_neg_log_weight_sum: None,
        rsd,
        rsdr: None,
        r_squared,
        adj_r_squared,
        rmse,
        weighted_r_squared: None,
        weighted_adj_r_squared: None,
        weighted_rmse: None,
        weighted_sy_x: None,
        poisson_gof: None,
        aic_c: None,
        bic: None,
        parameters,
        fixed_constants: layout.fixed_constants,
        covariance: CovarianceDiagnostics {
            standard_errors: vec![f64::NAN; k_free],
            symmetric_ci_lower: vec![f64::NAN; k_free],
            symmetric_ci_upper: vec![f64::NAN; k_free],
            covariance_matrix: if is_robust { Vec::new() } else { vec![vec![0.0; k_free]; k_free] },
            correlation_matrix: if is_robust { Vec::new() } else { vec![vec![0.0; k_free]; k_free] },
            dependencies: if is_robust { vec![f64::NAN; k_free] } else { vec![0.0; k_free] },
            numerical_rank: 0,
            ambiguous: false,
        },
        datasets: ds_results,
        derived_scalars,
        runs_test: None,
        lack_of_fit: None,
        normality: None,
        normality_anderson_darling: None,
        normality_lilliefors_ks: None,
        homoscedasticity: None,
        rout: None,
        interpolated_x: Vec::new(),
        interpolated_y: Vec::new(),
        has_shared_parameters: layout.free_slots.iter().any(|s| s.is_shared),
    })
}

/// Dispatch on [`FitMethod`]. ROUT follows the three stages of Motulsky & Brown (2006): robust
/// Cauchy fit, outlier detection on its residuals (`rout_detect_outliers_mb2006`), then an ordinary
/// (optionally weighted) least-squares refit of the remaining points, seeded from the robust fit.
pub(in crate::solver) fn solve_lm_by_method(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &mut Vec<ObsRecord>,
    init_theta: &[f64],
    config: &SolverConfig,
) -> Result<(LmRunOutput, Option<RoutDetectionResult>, usize), FitError> {
    let k_free = layout.k_free();
    let outlier_handling = config.effective_outlier_handling();
    if outlier_handling != OutlierHandling::None {
        let fdr_q_percent = config.effective_rout_q_percent();
        let mut cauchy_out = fit_cauchy_irls(model, layout, obs, init_theta, config)?;
        if matches!(cauchy_out.status, FitStatus::ImpossibleWeights | FitStatus::Diverged | FitStatus::BadInitialValues) {
            return Ok((cauchy_out, None, 0));
        }
        if config.uses_predicted_y_weighting() {
            if !apply_predicted_y_weights(obs, &cauchy_out.y_hat, config.weighting) {
                cauchy_out.status = FitStatus::ImpossibleWeights;
                return Ok((cauchy_out, None, 0));
            }
            if let Some((w_res, _, ss)) = eval_residuals_with_domain(model, layout, obs, &cauchy_out.theta, false) {
                cauchy_out.weighted_res = w_res;
                cauchy_out.weighted_ss = ss;
            } else {
                cauchy_out.status = FitStatus::ImpossibleWeights;
                return Ok((cauchy_out, None, 0));
            }
        }
        let pts: Vec<DataPoint> = obs.iter().map(|o| o.point.clone()).collect();
        let sqrt_w: Vec<f64> = obs.iter().map(|o| o.sqrt_weight).collect();
        let w_res: Vec<f64> = obs.iter().zip(cauchy_out.y_hat.iter()).map(|(o, yh)| (o.point.y - yh) * o.sqrt_weight).collect();
        let rout = rout_detect_outliers_mb2006(&pts, &w_res, &sqrt_w, k_free, fdr_q_percent)?;
        let excluded = if outlier_handling == OutlierHandling::Eliminate {
            let mut is_out = vec![false; obs.len()];
            for &idx in &rout.outlier_indices {
                is_out[idx] = true;
            }
            let clean_obs: Vec<ObsRecord> = obs.iter().enumerate().filter(|&(i, _)| !is_out[i]).map(|(_, r)| r.clone()).collect();
            if clean_obs.len() <= k_free {
                return Err(FitError::InsufficientData { required: k_free + 1, actual: clean_obs.len() });
            }
            let ex = obs.len() - clean_obs.len();
            *obs = clean_obs;
            ex
        } else {
            0
        };
        let mut ols = if config.uses_predicted_y_weighting() {
            fit_predicted_y_irls(model, layout, obs, &cauchy_out.theta, config)?
        } else {
            solve_with_warm_start(model, layout, obs, &cauchy_out.theta, config)?
        };
        if ols.status != FitStatus::ImpossibleWeights {
            let mut alt_obs = obs.clone();
            let alt_res = if config.uses_predicted_y_weighting() {
                fit_predicted_y_irls(model, layout, &mut alt_obs, init_theta, config)
            } else {
                solve_with_warm_start(model, layout, &alt_obs, init_theta, config)
            };
            if let Ok(alt) = alt_res
                && ((alt.status == FitStatus::Converged && ols.status != FitStatus::Converged)
                    || (alt.status == FitStatus::Converged && alt.weighted_ss < ols.weighted_ss * (1.0 - 1e-12)))
            {
                *obs = alt_obs;
                ols = alt;
            }
        }
        return Ok((ols, Some(rout), excluded));
    }

    match config.method {
        FitMethod::LeastSquares => {
            let out = if config.uses_predicted_y_weighting() {
                fit_predicted_y_irls(model, layout, obs, init_theta, config)?
            } else {
                solve_with_warm_start(model, layout, obs, init_theta, config)?
            };
            Ok((out, None, 0))
        }
        FitMethod::PoissonDeviance => Ok((fit_poisson_irls(model, layout, obs, init_theta, config)?, None, 0)),
        FitMethod::RobustCauchy => {
            let mut out = fit_cauchy_irls(model, layout, obs, init_theta, config)?;
            if config.uses_predicted_y_weighting() {
                if !apply_predicted_y_weights(obs, &out.y_hat, config.weighting) {
                    out.status = FitStatus::ImpossibleWeights;
                } else {
                    finalize_cauchy_weighted_residuals(obs, &mut out, k_free);
                }
            }
            Ok((out, None, 0))
        }
        FitMethod::RoutOutlierElimination { .. } => {
            let out = if config.uses_predicted_y_weighting() {
                fit_predicted_y_irls(model, layout, obs, init_theta, config)?
            } else {
                solve_with_warm_start(model, layout, obs, init_theta, config)?
            };
            Ok((out, None, 0))
        }
    }
}

pub(in crate::solver) struct FinalizedUncertainty {
    pub(in crate::solver) cov: CovarianceDiagnostics,
    pub(in crate::solver) identifiability: Assessment,
    pub(in crate::solver) cov_unscaled: Option<DenseMatrix>,
    pub(in crate::solver) hougaard: Vec<f64>,
    pub(in crate::solver) profile_cis: Vec<(CiBound, CiBound)>,
    pub(in crate::solver) t_crit: f64,
    pub(in crate::solver) sigma_w: f64,
}

fn mask_unstable_cov_slot(cov: &mut CovarianceDiagnostics, u: usize) {
    if let Some(v) = cov.standard_errors.get_mut(u) {
        *v = f64::NAN;
    }
    if let Some(v) = cov.symmetric_ci_lower.get_mut(u) {
        *v = f64::NAN;
    }
    if let Some(v) = cov.symmetric_ci_upper.get_mut(u) {
        *v = f64::NAN;
    }
    if let Some(row) = cov.covariance_matrix.get_mut(u) {
        row.fill(f64::NAN);
    }
    for row in &mut cov.covariance_matrix {
        if let Some(cell) = row.get_mut(u) {
            *cell = f64::NAN;
        }
    }
    if let Some(row) = cov.correlation_matrix.get_mut(u) {
        row.fill(f64::NAN);
    }
    for row in &mut cov.correlation_matrix {
        if let Some(cell) = row.get_mut(u) {
            *cell = f64::NAN;
        }
    }
}

fn apply_greedy_hessian_reduction(
    j_mat: &DenseMatrix,
    theta: &[f64],
    weighted_ss: f64,
    df: usize,
    is_poisson: bool,
    config: &SolverConfig,
    cov: &mut CovarianceDiagnostics,
) -> Result<identifiability::GreedyHessianReduction, FitError> {
    let h = identifiability::assemble_objective_hessian(j_mat, is_poisson)?;
    let red = identifiability::greedy_hessian_reduction(&h, identifiability::hessian_kappa_threshold())?;
    if !red.unstable_indices.is_empty() && !red.stable_indices.is_empty() {
        let j_stable = identifiability::submatrix_columns(j_mat, &red.stable_indices)?;
        let theta_stable: Vec<f64> = red.stable_indices.iter().map(|&i| theta[i]).collect();
        let cov_stable = if is_poisson {
            compute_poisson_covariance_diagnostics(&j_stable, df, &theta_stable, config.confidence)?
        } else {
            compute_covariance_diagnostics(&j_stable, weighted_ss, df, &theta_stable, config.confidence)?
        };
        for (sub_i, &orig_i) in red.stable_indices.iter().enumerate() {
            if let Some(&se) = cov_stable.standard_errors.get(sub_i) {
                cov.standard_errors[orig_i] = se;
            }
            if let Some(&lo) = cov_stable.symmetric_ci_lower.get(sub_i) {
                cov.symmetric_ci_lower[orig_i] = lo;
            }
            if let Some(&hi) = cov_stable.symmetric_ci_upper.get(sub_i) {
                cov.symmetric_ci_upper[orig_i] = hi;
            }
            if let Some(&dep) = cov_stable.dependencies.get(sub_i) {
                cov.dependencies[orig_i] = dep;
            }
            for (sub_j, &orig_j) in red.stable_indices.iter().enumerate() {
                if let Some(v) = cov_stable.covariance_matrix.get(sub_i).and_then(|r| r.get(sub_j)).copied()
                    && let Some(cell) = cov.covariance_matrix.get_mut(orig_i).and_then(|r| r.get_mut(orig_j))
                {
                    *cell = v;
                }
                if let Some(v) = cov_stable.correlation_matrix.get(sub_i).and_then(|r| r.get(sub_j)).copied()
                    && let Some(cell) = cov.correlation_matrix.get_mut(orig_i).and_then(|r| r.get_mut(orig_j))
                {
                    *cell = v;
                }
            }
        }
        cov.numerical_rank = cov_stable.numerical_rank;
    } else if !red.unstable_indices.is_empty() && red.stable_indices.is_empty() {
        cov.numerical_rank = 0;
    }
    for &u in &red.unstable_indices {
        mask_unstable_cov_slot(cov, u);
    }
    Ok(red)
}

#[allow(clippy::too_many_arguments)]
fn compute_fit_hougaard(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    lm_out: &LmRunOutput,
    cov_unscaled: Option<&DenseMatrix>,
    sigma_w: f64,
    is_poisson: bool,
    enabled: bool,
) -> Vec<f64> {
    let k_free = layout.k_free();
    if !enabled || k_free == 0 || k_free > 10 {
        return vec![f64::NAN; k_free];
    }
    let Some(cov_u) = cov_unscaled else {
        return vec![f64::NAN; k_free];
    };
    let sqrt_w_opt: Option<Vec<f64>> = if is_poisson {
        if lm_out.y_hat.iter().all(|&yh| yh.is_finite() && yh > 0.0) { Some(lm_out.y_hat.iter().map(|&yh| 1.0 / yh.sqrt()).collect()) } else { None }
    } else {
        Some(obs.iter().map(|o| o.sqrt_weight).collect())
    };
    let Some(sqrt_w) = sqrt_w_opt else {
        return vec![f64::NAN; k_free];
    };
    let mut flat_p = vec![0.0_f64; layout.n_datasets * layout.p_model];
    compute_hougaard_skewness_into(&lm_out.theta, &sqrt_w, &lm_out.j_mat, cov_u, sigma_w, |th, out| {
        eval_predictions_into(model, layout, obs, th, &mut flat_p, out)
    })
}

#[allow(clippy::too_many_arguments)]
fn reconcile_unstable_uncertainty(
    identifiability: &Assessment,
    unstable_from_reduction: &[usize],
    k_free: usize,
    cov_df: usize,
    cov_ss: f64,
    j_mat: &DenseMatrix,
    cov: &mut CovarianceDiagnostics,
    profile_cis: &mut [(CiBound, CiBound)],
    hougaard: &mut [f64],
    cov_unscaled: &mut Option<DenseMatrix>,
) {
    let mut newly_masked = false;
    for (u, &is_u) in identifiability.unstable.iter().enumerate() {
        if is_u {
            mask_unstable_cov_slot(cov, u);
            if let Some(slot) = profile_cis.get_mut(u) {
                *slot = (CiBound::Open, CiBound::Open);
            }
            if let Some(h) = hougaard.get_mut(u) {
                *h = f64::NAN;
            }
            if !unstable_from_reduction.contains(&u) {
                newly_masked = true;
            }
        }
    }
    if newly_masked {
        let remaining_stable: Vec<usize> = (0..k_free).filter(|&i| !identifiability.unstable.get(i).copied().unwrap_or(false)).collect();
        *cov_unscaled =
            if remaining_stable.is_empty() { None } else { unscaled_cov_for_stable_subspace(cov, cov_df, cov_ss, j_mat, &remaining_stable) };
    }
}

pub(in crate::solver) fn finalize_fit_uncertainty(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    lm_out: &mut LmRunOutput,
    df: usize,
    ss_within_total: f64,
    config: &SolverConfig,
) -> Result<FinalizedUncertainty, FitError> {
    let k_free = layout.k_free();
    if config.method == FitMethod::RobustCauchy {
        return Ok(FinalizedUncertainty {
            cov: CovarianceDiagnostics {
                standard_errors: vec![f64::NAN; k_free],
                symmetric_ci_lower: vec![f64::NAN; k_free],
                symmetric_ci_upper: vec![f64::NAN; k_free],
                covariance_matrix: Vec::new(),
                correlation_matrix: Vec::new(),
                dependencies: vec![f64::NAN; k_free],
                numerical_rank: 0,
                ambiguous: false,
            },
            identifiability: Assessment::default(),
            cov_unscaled: None,
            hougaard: vec![f64::NAN; k_free],
            profile_cis: vec![(CiBound::Open, CiBound::Open); k_free],
            t_crit: f64::NAN,
            sigma_w: f64::NAN,
        });
    }
    let is_poisson = config.method == FitMethod::PoissonDeviance;
    let effective_mode =
        config.identifiability_mode.or_else(|| config.identify_unstable_parameters.then_some(IdentifiabilityMode::IdentifyUnstableParameters));
    let report_all = effective_mode == Some(IdentifiabilityMode::ReportAll);
    let mut cov = if is_poisson {
        compute_poisson_covariance_diagnostics(&lm_out.j_mat, df, &lm_out.theta, config.confidence)?
    } else {
        compute_covariance_diagnostics(&lm_out.j_mat, lm_out.weighted_ss, df, &lm_out.theta, config.confidence)?
    };
    let full_cov_ambiguous = cov.ambiguous;

    // Greedy Hessian reduction under `IdentifiabilityMode::IdentifyUnstableParameters` (§2.5.1):
    // drop maximally ill-conditioned rows/columns while keeping well-conditioned parameters' SE/CIs.
    let greedy_reduction =
        if effective_mode == Some(IdentifiabilityMode::IdentifyUnstableParameters) && k_free > 0 && df > 0 && lm_out.status != FitStatus::Diverged {
            Some(apply_greedy_hessian_reduction(&lm_out.j_mat, &lm_out.theta, lm_out.weighted_ss, df, is_poisson, config, &mut cov)?)
        } else {
            None
        };

    if report_all {
        cov.ambiguous = false;
    } else if full_cov_ambiguous && lm_out.status == FitStatus::Converged {
        lm_out.status = FitStatus::Ambiguous;
    }

    let unstable_from_reduction: Vec<usize> = greedy_reduction.as_ref().map(|r| r.unstable_indices.clone()).unwrap_or_default();
    let has_reduced_stable = greedy_reduction.as_ref().is_some_and(|r| !r.unstable_indices.is_empty() && !r.stable_indices.is_empty());
    let (cov_df, cov_ss) = if is_poisson { (1, 1.0) } else { (df, lm_out.weighted_ss) };
    let mut cov_unscaled = if let Some(red) = greedy_reduction.as_ref().filter(|r| !r.unstable_indices.is_empty()) {
        if red.stable_indices.is_empty() { None } else { unscaled_cov_for_stable_subspace(&cov, cov_df, cov_ss, &lm_out.j_mat, &red.stable_indices) }
    } else {
        unscaled_cov_from_diag_or_j(&cov, cov_df, cov_ss, &lm_out.j_mat)
    };
    let (sigma_w, t_crit) = if is_poisson {
        (if df > 0 { 1.0 } else { f64::NAN }, if df > 0 { norm_two_sided_isf(config.confidence.alpha()).unwrap_or(f64::NAN) } else { f64::NAN })
    } else {
        (
            if df > 0 { (lm_out.weighted_ss / (df as f64)).sqrt() } else { f64::NAN },
            if df > 0 { student_t_two_sided_isf(config.confidence.alpha(), df as f64).unwrap_or(f64::NAN) } else { f64::NAN },
        )
    };

    let full_rank = cov.numerical_rank == k_free;
    let hougaard_enabled = config.compute_hougaard
        && full_rank
        && !cov.ambiguous
        && df > 0
        && !matches!(lm_out.status, FitStatus::Diverged | FitStatus::ImpossibleWeights | FitStatus::BadInitialValues);
    let mut hougaard = compute_fit_hougaard(model, layout, obs, lm_out, cov_unscaled.as_ref(), sigma_w, is_poisson, hougaard_enabled);

    let bounds = layout.bounds();
    let profile_computed = config.compute_profile_ci
        && k_free > 0
        && df > 0
        && ((full_rank && !cov.ambiguous) || has_reduced_stable)
        && lm_out.status != FitStatus::ImpossibleWeights;
    let mut profile_cis = match config.method {
        FitMethod::RobustCauchy => vec![(CiBound::Open, CiBound::Open); k_free],
        FitMethod::PoissonDeviance => {
            if profile_computed {
                compute_deviance_profile_likelihood_cis_checked(
                    &lm_out.theta,
                    lm_out.weighted_ss,
                    &cov,
                    &bounds,
                    config.confidence,
                    |fixed_idx, fixed_val, seed| {
                        solve_profile_subproblem_poisson(model, layout, obs, seed, fixed_idx, fixed_val, &unstable_from_reduction)
                    },
                )
            } else if has_reduced_stable {
                let mut cov_tmp = cov.clone();
                cov_tmp.ambiguous = false;
                wald_ci_bounds(&cov_tmp, df)
            } else {
                wald_ci_bounds(&cov, df)
            }
        }
        FitMethod::LeastSquares | FitMethod::RoutOutlierElimination { .. } => {
            if profile_computed {
                let mut wy2_acc = NeumaierSum::new();
                for o in obs {
                    let wy = o.sqrt_weight * o.point.y;
                    wy2_acc.add(wy * wy);
                }
                wy2_acc.add(ss_within_total);
                compute_profile_likelihood_cis_checked(
                    &lm_out.theta,
                    lm_out.weighted_ss,
                    Some(wy2_acc.total()),
                    df,
                    &cov,
                    &bounds,
                    config.confidence,
                    |fixed_idx, fixed_val, seed| {
                        solve_profile_subproblem(model, layout, obs, seed, fixed_idx, fixed_val, &unstable_from_reduction, ss_within_total)
                    },
                )
            } else if has_reduced_stable {
                let mut cov_tmp = cov.clone();
                cov_tmp.ambiguous = false;
                wald_ci_bounds(&cov_tmp, df)
            } else {
                wald_ci_bounds(&cov, df)
            }
        }
    };
    for &u in &unstable_from_reduction {
        if let Some(slot) = profile_cis.get_mut(u) {
            *slot = (CiBound::Open, CiBound::Open);
        }
        if let Some(h) = hougaard.get_mut(u) {
            *h = f64::NAN;
        }
    }

    let mut identifiability = if let Some(mode) = effective_mode
        && k_free > 0
        && lm_out.status != FitStatus::Diverged
    {
        identifiability::assess_for_mode(
            mode,
            layout,
            obs,
            &lm_out.j_mat,
            df,
            &cov.dependencies,
            profile_computed.then_some(profile_cis.as_slice()),
            greedy_reduction.as_ref(),
            full_cov_ambiguous,
        )?
    } else {
        Assessment::default()
    };
    if effective_mode == Some(IdentifiabilityMode::IdentifyUnstableParameters) && !identifiability.unstable.is_empty() {
        reconcile_unstable_uncertainty(
            &identifiability,
            &unstable_from_reduction,
            k_free,
            cov_df,
            cov_ss,
            &lm_out.j_mat,
            &mut cov,
            &mut profile_cis,
            &mut hougaard,
            &mut cov_unscaled,
        );
    }
    if !matches!(lm_out.status, FitStatus::Converged | FitStatus::Ambiguous) && matches!(identifiability.fit, FitIdentifiability::Identifiable) {
        identifiability.fit = FitIdentifiability::NotAssessed;
    }
    Ok(FinalizedUncertainty { cov, identifiability, cov_unscaled, hougaard, profile_cis, t_crit, sigma_w })
}
