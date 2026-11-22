//! `fit_nonlinear` entry point, plot-only evaluation, method dispatch, and final uncertainty.
//!
//! References:
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.

use curvebook_doc::{CiBound, FitMethod, WeightingScheme};
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
use crate::solver::inference::{rout_detect_outliers_mb2006, unscaled_cov_from_diag_or_j, wald_ci_bounds};
use crate::solver::irls::{fit_cauchy_irls, fit_poisson_irls};
use crate::solver::layout::{
    ObsRecord, ParamBinding, ParameterLayout, build_observations, obs_replicate_count, ode_budget_error, uses_replicate_summary_moments,
    uses_shared_global,
};
use crate::solver::lm::{LmRunOutput, eval_predictions_into, solve_with_warm_start};
use crate::solver::profile::{solve_profile_subproblem, solve_profile_subproblem_poisson};
use crate::solver::results::{FittedDatasetResult, FittedParameterSummary, InferenceAvailability, NonlinearFitResult, SolverConfig};
use crate::types::{DataPoint, DatasetSeries, HookContext, filter_datasets_by_range};

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

    let mut filtered_storage: Option<Vec<DatasetSeries>> = None;
    if config.x_range_filter.is_some() || config.y_range_filter.is_some() {
        let mut copy = datasets.to_vec();
        filter_datasets_by_range(&mut copy, config.x_range_filter, config.y_range_filter);
        if copy.is_empty() {
            return Err(FitError::EmptyInput);
        }
        filtered_storage = Some(copy);
    }
    let active_datasets = filtered_storage.as_deref().unwrap_or(datasets);

    if let FitMethod::RoutOutlierElimination { fdr_q_percent } = config.method
        && (!fdr_q_percent.is_finite() || fdr_q_percent <= 0.0 || fdr_q_percent > 50.0)
    {
        return Err(FitError::InvalidParameter { name: "fdr_q_percent", reason: format!("FDR Q% must lie in (0, 50], got {fdr_q_percent}") });
    }
    if matches!(config.method, FitMethod::RobustCauchy | FitMethod::RoutOutlierElimination { .. })
        && active_datasets.iter().any(|d| d.is_pre_summarized)
    {
        return Err(FitError::RequiresRawReplicates);
    }
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

    let layout = ParameterLayout::build(model, active_datasets, hooks, &config.parameter_rules)?;
    let k_free = layout.k_free();
    let use_rep_moments = uses_replicate_summary_moments(config);
    let obs = build_observations(active_datasets, config.weighting, use_rep_moments)?;
    let n_total_obs = obs_replicate_count(&obs);
    let min_required = if matches!(config.method, FitMethod::RoutOutlierElimination { .. }) { k_free + 1 } else { k_free };
    if n_total_obs < min_required {
        return Err(FitError::InsufficientData { required: min_required, actual: n_total_obs });
    }

    if active_datasets.len() > 1 && !uses_shared_global(&config.parameter_rules) {
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
    let layout = ParameterLayout::build(model, datasets, hooks, &config.parameter_rules)?;
    let k_free = layout.k_free();
    let theta: Vec<f64> = layout.free_slots.iter().map(|s| s.init).collect();
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
            hougaard_g1: if is_robust { f64::NAN } else { 0.0 },
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
        derived_scalars.push(model.derived_transforms(&ds_params));
        let mut ds_yh = Vec::with_capacity(ds.points.len());
        let mut ds_res = Vec::with_capacity(ds.points.len());
        let mut ds_y_vals = Vec::with_capacity(ds.points.len());
        let mut ds_rss = NeumaierSum::new();
        for pt in &ds.points {
            let Some(yh) = model.eval(&pt.x, &ds_params, ds.data_set_index) else {
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
        let ds_r2 = if ds_y_vals.len() > 1 {
            let ym = ds_y_vals.iter().sum::<f64>() / (ds_y_vals.len() as f64);
            let dtss: f64 = ds_y_vals.iter().map(|v| (v - ym) * (v - ym)).sum();
            if dtss > 0.0 { 1.0 - ds_ss / dtss } else { 1.0 }
        } else {
            1.0
        };
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
    let r_squared = if y_vals_all.len() > 1 {
        let ym = y_vals_all.iter().sum::<f64>() / (y_vals_all.len() as f64);
        let tss: f64 = y_vals_all.iter().map(|v| (v - ym) * (v - ym)).sum();
        if tss > 0.0 { 1.0 - raw_rss / tss } else { 1.0 }
    } else {
        1.0
    };

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
        rsd,
        rsdr: None,
        r_squared,
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
    match config.method {
        FitMethod::LeastSquares => Ok((solve_with_warm_start(model, layout, obs, init_theta, config)?, None, 0)),
        FitMethod::PoissonDeviance => Ok((fit_poisson_irls(model, layout, obs, init_theta, config)?, None, 0)),
        FitMethod::RobustCauchy => Ok((fit_cauchy_irls(model, layout, obs, init_theta, config)?, None, 0)),
        FitMethod::RoutOutlierElimination { fdr_q_percent } => {
            let cauchy_out = fit_cauchy_irls(model, layout, obs, init_theta, config)?;
            let pts: Vec<DataPoint> = obs.iter().map(|o| o.point.clone()).collect();
            let sqrt_w: Vec<f64> = obs.iter().map(|o| o.sqrt_weight).collect();
            let w_res: Vec<f64> = obs.iter().zip(cauchy_out.y_hat.iter()).map(|(o, yh)| (o.point.y - yh) * o.sqrt_weight).collect();
            let rout = rout_detect_outliers_mb2006(&pts, &w_res, &sqrt_w, k_free, fdr_q_percent)?;
            let mut is_out = vec![false; obs.len()];
            for &idx in &rout.outlier_indices {
                is_out[idx] = true;
            }
            let clean_obs: Vec<ObsRecord> = obs.iter().enumerate().filter(|&(i, _)| !is_out[i]).map(|(_, r)| r.clone()).collect();
            if clean_obs.len() <= k_free {
                return Err(FitError::InsufficientData { required: k_free + 1, actual: clean_obs.len() });
            }
            let excluded = obs.len() - clean_obs.len();
            *obs = clean_obs;
            let mut ols = solve_with_warm_start(model, layout, obs, &cauchy_out.theta, config)?;
            if let Ok(alt) = solve_with_warm_start(model, layout, obs, init_theta, config)
                && ((alt.status == FitStatus::Converged && ols.status != FitStatus::Converged)
                    || (alt.status == FitStatus::Converged && alt.weighted_ss < ols.weighted_ss * (1.0 - 1e-12)))
            {
                ols = alt;
            }
            Ok((ols, Some(rout), excluded))
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
    let cov = if is_poisson {
        compute_poisson_covariance_diagnostics(&lm_out.j_mat, df, &lm_out.theta, config.confidence)?
    } else {
        compute_covariance_diagnostics(&lm_out.j_mat, lm_out.weighted_ss, df, &lm_out.theta, config.confidence)?
    };
    if cov.ambiguous && lm_out.status == FitStatus::Converged {
        lm_out.status = FitStatus::Ambiguous;
    }

    let (cov_unscaled, sigma_w, t_crit) = if is_poisson {
        (
            unscaled_cov_from_diag_or_j(&cov, 1, 1.0, &lm_out.j_mat),
            if df > 0 { 1.0 } else { f64::NAN },
            if df > 0 { norm_two_sided_isf(config.confidence.alpha()).unwrap_or(f64::NAN) } else { f64::NAN },
        )
    } else {
        (
            unscaled_cov_from_diag_or_j(&cov, df, lm_out.weighted_ss, &lm_out.j_mat),
            if df > 0 { (lm_out.weighted_ss / (df as f64)).sqrt() } else { f64::NAN },
            if df > 0 { student_t_two_sided_isf(config.confidence.alpha(), df as f64).unwrap_or(f64::NAN) } else { f64::NAN },
        )
    };

    let hougaard = if config.compute_hougaard && k_free > 0 && k_free <= 10 && !cov.ambiguous && df > 0 {
        if let Some(cov_u) = cov_unscaled.as_ref() {
            let sqrt_w_opt: Option<Vec<f64>> = if is_poisson {
                if lm_out.y_hat.iter().all(|&yh| yh.is_finite() && yh > 0.0) {
                    Some(lm_out.y_hat.iter().map(|&yh| 1.0 / yh.sqrt()).collect())
                } else {
                    None
                }
            } else {
                Some(obs.iter().map(|o| o.sqrt_weight).collect())
            };
            if let Some(sqrt_w) = sqrt_w_opt {
                let mut flat_p = vec![0.0_f64; layout.n_datasets * layout.p_model];
                compute_hougaard_skewness_into(&lm_out.theta, &sqrt_w, &lm_out.j_mat, cov_u, sigma_w, |th, out| {
                    eval_predictions_into(model, layout, obs, th, &mut flat_p, out)
                })
            } else {
                vec![0.0; k_free]
            }
        } else {
            vec![0.0; k_free]
        }
    } else {
        vec![0.0; k_free]
    };

    let bounds = layout.bounds();
    let profile_computed = config.compute_profile_ci && k_free > 0 && df > 0 && !cov.ambiguous;
    let profile_cis = match config.method {
        FitMethod::RobustCauchy => vec![(CiBound::Open, CiBound::Open); k_free],
        FitMethod::PoissonDeviance => {
            if profile_computed {
                compute_deviance_profile_likelihood_cis_checked(
                    &lm_out.theta,
                    lm_out.weighted_ss,
                    &cov,
                    &bounds,
                    config.confidence,
                    |fixed_idx, fixed_val, seed| solve_profile_subproblem_poisson(model, layout, obs, seed, fixed_idx, fixed_val),
                )
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
                    |fixed_idx, fixed_val, seed| solve_profile_subproblem(model, layout, obs, seed, fixed_idx, fixed_val, ss_within_total),
                )
            } else {
                wald_ci_bounds(&cov, df)
            }
        }
    };

    let mut identifiability = if config.identify_unstable_parameters && k_free > 0 && lm_out.status != FitStatus::Diverged {
        identifiability::assess(layout, obs, &lm_out.j_mat, df, &cov.dependencies, profile_computed.then_some(profile_cis.as_slice()))?
    } else {
        Assessment::default()
    };
    if !matches!(lm_out.status, FitStatus::Converged | FitStatus::Ambiguous) && matches!(identifiability.fit, FitIdentifiability::Identifiable) {
        identifiability.fit = FitIdentifiability::NotAssessed;
    }
    Ok(FinalizedUncertainty { cov, identifiability, cov_unscaled, hougaard, profile_cis, t_crit, sigma_w })
}
