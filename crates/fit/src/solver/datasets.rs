//! Shared-parameter global fits and independent per-dataset fits.
//!
//! References:
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.

use curvebook_doc::{CiBound, FitMethod, IdentifiabilityMode, OutlierHandling, ParameterRule, ParameterScope};
use curvebook_math::sum::NeumaierSum;

use crate::diagnostics::{
    CovarianceDiagnostics, RoutDetectionResult, compute_information_criteria, compute_information_criteria_with_log_weight_penalty, compute_rsdr,
};
use crate::error::{FitError, FitIdentifiability, FitStatus, ParameterNote};
use crate::library::ParametricModel;
use crate::solver::assembly::{DatasetAssemblyConfig, assemble_dataset_results, build_fitted_parameter_summaries};
use crate::solver::fit::{finalize_fit_uncertainty, solve_lm_by_method};
use crate::solver::identifiability;
use crate::solver::identifiability::Assessment;
use crate::solver::inference::{
    compute_r_squared, compute_residual_diagnostics, compute_studentized_residuals, information_criteria_from_poisson_ll,
    poisson_information_criteria, poisson_log_likelihood_from_obs, predicted_y_neg_log_weight_sum, rout_detect_outliers_mb2006,
};
use crate::solver::irls::{finalize_cauchy_weighted_residuals, fit_cauchy_irls, fit_poisson_irls, fit_predicted_y_irls};
use crate::solver::layout::{
    ObsRecord, ParamBinding, ParameterLayout, apply_predicted_y_weights, build_observations_with_source, collect_column_title_values,
    obs_replicate_count, obs_ss_within, rules_for_dataset, uses_replicate_summary_moments,
};
use crate::solver::lm::solve_with_warm_start;
use crate::solver::results::{FittedDatasetResult, InferenceAvailability, InterpolatedUnknown, NonlinearFitResult, SolverConfig};
use crate::types::{DatasetSeries, DerivedScalar, HookContext, per_dataset_parameter_key};

pub(in crate::solver) fn fit_single_or_shared(
    model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    config: &SolverConfig,
    layout: ParameterLayout,
    mut obs: Vec<ObsRecord>,
) -> Result<NonlinearFitResult, FitError> {
    let k_free = layout.k_free();
    let use_rep_moments = uses_replicate_summary_moments(config);
    let init_theta: Vec<f64> = layout.free_slots.iter().map(|s| s.init).collect();
    let (mut lm_out, rout_result, n_outliers_excluded) = solve_lm_by_method(model, &layout, &mut obs, &init_theta, config)?;

    let ss_within_total = if use_rep_moments { obs_ss_within(&obs) } else { 0.0 };
    lm_out.weighted_ss += ss_within_total;
    let n_obs = obs_replicate_count(&obs);
    let df = n_obs.saturating_sub(k_free);

    let is_robust = config.method == FitMethod::RobustCauchy;
    let inference_availability = if is_robust { InferenceAvailability::NotApplicableInRobustMode } else { InferenceAvailability::Available };
    let unc = finalize_fit_uncertainty(model, &layout, &obs, &mut lm_out, df, ss_within_total, config)?;
    let report_all = config.identifiability_mode == Some(IdentifiabilityMode::ReportAll);
    let parameters = build_fitted_parameter_summaries(
        &layout,
        &lm_out.theta,
        &unc.cov,
        &unc.hougaard,
        &unc.profile_cis,
        &unc.identifiability.unstable,
        inference_availability,
        report_all,
    );

    let mut raw_rss_acc = NeumaierSum::new();
    let mut y_mean_acc = NeumaierSum::new();
    for (rec, &yh) in obs.iter().zip(lm_out.y_hat.iter()) {
        let diff = rec.point.y - yh;
        let w_rep = rec.rep_n as f64;
        raw_rss_acc.add(w_rep * diff * diff);
        raw_rss_acc.add(rec.raw_ss_within);
        y_mean_acc.add(w_rep * rec.point.y);
    }
    let raw_rss = raw_rss_acc.total();
    let y_mean = y_mean_acc.total() / (n_obs as f64);
    let mut tss_acc = NeumaierSum::new();
    for rec in &obs {
        let d = rec.point.y - y_mean;
        tss_acc.add((rec.rep_n as f64) * d * d);
        tss_acc.add(rec.raw_ss_within);
    }
    let tss = tss_acc.total();
    let is_robust = config.method == FitMethod::RobustCauchy;
    let r_squared = compute_r_squared(raw_rss, tss, n_obs);
    let rsd = if df > 0 && !is_robust { (raw_rss / (df as f64)).sqrt() } else { f64::NAN };

    let predicted_y_neg_log_weight_sum = if !is_robust && config.method != FitMethod::PoissonDeviance && config.uses_predicted_y_weighting() {
        predicted_y_neg_log_weight_sum(&obs)
    } else {
        None
    };
    let ic = if is_robust {
        None
    } else if config.method == FitMethod::PoissonDeviance {
        poisson_information_criteria(&obs, &lm_out.y_hat, k_free)
    } else if let Some(neg_log_w) = predicted_y_neg_log_weight_sum {
        compute_information_criteria_with_log_weight_penalty(lm_out.weighted_ss, neg_log_w, n_obs, k_free).ok()
    } else {
        compute_information_criteria(lm_out.weighted_ss, n_obs, k_free).ok()
    };
    let cov = unc.cov;

    let all_stud = if is_robust {
        Vec::new()
    } else {
        compute_studentized_residuals(&lm_out.j_mat, &lm_out.weighted_res, unc.cov_unscaled.as_ref(), unc.sigma_w, cov.ambiguous)
    };
    let assembled = assemble_dataset_results(
        model,
        &layout,
        datasets,
        &obs,
        &lm_out,
        &all_stud,
        &DatasetAssemblyConfig {
            cov_unscaled: if df > 0 { unc.cov_unscaled.as_ref() } else { None },
            sigma_w: unc.sigma_w,
            t_crit: unc.t_crit,
            rsd,
            ambiguous: cov.ambiguous || unc.identifiability.fit.is_ambiguous(),
            solver: config,
        },
    );

    let raw_residuals_all: Vec<f64> = obs.iter().zip(lm_out.y_hat.iter()).map(|(o, yh)| o.point.y - yh).collect();
    let rsdr = if is_robust {
        let abs_res: Vec<f64> = obs.iter().zip(raw_residuals_all.iter()).map(|(o, r)| (r * o.sqrt_weight).abs()).collect();
        Some(compute_rsdr(&abs_res, k_free))
    } else {
        None
    };
    let res_diag = compute_residual_diagnostics(&obs, &assembled.ds_results, &lm_out.y_hat, &raw_residuals_all, lm_out.weighted_ss, df, config);
    let has_shared_parameters = assembled.ds_results.len() > 1;
    let mut ds_results = assembled.ds_results;
    if ds_results.len() == 1 {
        let ds0 = &mut ds_results[0];
        ds0.identifiability = unc.identifiability.fit.clone();
        ds0.k_free = k_free;
        ds0.df = df;
        ds0.weighted_ss = lm_out.weighted_ss;
        ds0.aic_c = ic.as_ref().map(|x| x.aicc);
        ds0.bic = ic.as_ref().map(|x| x.bic);
        ds0.normality = res_diag.normality.clone();
        ds0.normality_anderson_darling = res_diag.normality_anderson_darling.clone();
        ds0.normality_lilliefors_ks = res_diag.normality_lilliefors_ks.clone();
        ds0.homoscedasticity = res_diag.homoscedasticity.clone();
        ds0.lack_of_fit = res_diag.lack_of_fit.clone();
        ds0.n_outliers = rout_result.as_ref().map_or(n_outliers_excluded, |r| r.outlier_indices.len().max(n_outliers_excluded));
        ds0.predicted_y_neg_log_weight_sum = predicted_y_neg_log_weight_sum;
    } else {
        for ds_res in &mut ds_results {
            ds_res.identifiability = unc.identifiability.fit.clone();
        }
    }

    Ok(NonlinearFitResult {
        equation_id: model.id().to_owned(),
        equation_name: model.name().to_owned(),
        fit_method: config.method,
        inference_availability,
        status: lm_out.status,
        identifiability: unc.identifiability.fit,
        iterations: lm_out.iterations,
        n_obs,
        n_outliers_excluded,
        k_free,
        df,
        rss: raw_rss,
        weighted_ss: lm_out.weighted_ss,
        predicted_y_neg_log_weight_sum,
        rsd,
        rsdr,
        r_squared,
        aic_c: ic.as_ref().map(|x| x.aicc),
        bic: ic.as_ref().map(|x| x.bic),
        parameters,
        fixed_constants: layout.fixed_constants,
        covariance: cov,
        datasets: ds_results,
        derived_scalars: assembled.derived_scalars,
        runs_test: res_diag.runs_test,
        lack_of_fit: res_diag.lack_of_fit,
        normality: res_diag.normality,
        normality_anderson_darling: res_diag.normality_anderson_darling,
        normality_lilliefors_ks: res_diag.normality_lilliefors_ks,
        homoscedasticity: res_diag.homoscedasticity,
        rout: rout_result,
        interpolated_x: assembled.interpolated_x,
        interpolated_y: assembled.interpolated_y,
        has_shared_parameters,
    })
}

pub(in crate::solver) struct SingleDatasetOutcome {
    pub(in crate::solver) layout: ParameterLayout,
    pub(in crate::solver) theta: Vec<f64>,
    pub(in crate::solver) status: FitStatus,
    pub(in crate::solver) iterations: usize,
    pub(in crate::solver) weighted_ss: f64,
    pub(in crate::solver) cov: CovarianceDiagnostics,
    pub(in crate::solver) hougaard: Vec<f64>,
    pub(in crate::solver) profile_cis: Vec<(CiBound, CiBound)>,
    pub(in crate::solver) identifiability: Assessment,
    pub(in crate::solver) ds_result: FittedDatasetResult,
    pub(in crate::solver) derived_scalars: Vec<DerivedScalar>,
    pub(in crate::solver) interpolated_x: Vec<InterpolatedUnknown>,
    pub(in crate::solver) interpolated_y: Vec<InterpolatedUnknown>,
    pub(in crate::solver) obs_moments: Vec<(f64, usize, f64, f64)>,
    pub(in crate::solver) poisson_ll: Option<f64>,
}

pub(in crate::solver) fn solve_single_dataset_unit(
    model: &dyn ParametricModel,
    ds: &DatasetSeries,
    hooks: &HookContext,
    ds_rules: &[ParameterRule],
    config: &SolverConfig,
    seed_override: Option<&[f64]>,
    column_titles: Option<&[f64]>,
) -> Result<SingleDatasetOutcome, FitError> {
    let ds_slice = std::slice::from_ref(ds);
    let ds_layout = ParameterLayout::build_with_column_titles(model, ds_slice, hooks, ds_rules, column_titles)?;
    let p_d = ds_layout.k_free();
    let use_rep_moments = uses_replicate_summary_moments(config);
    let mut ds_obs = build_observations_with_source(ds_slice, config.weighting, config.weighting_source, use_rep_moments)?;
    let n_d = obs_replicate_count(&ds_obs);
    let min_d = if config.effective_outlier_handling() != OutlierHandling::None { p_d + 1 } else { p_d };
    if n_d < min_d {
        return Err(FitError::InsufficientData { required: min_d, actual: n_d });
    }
    let init_theta: Vec<f64> = seed_override.map_or_else(|| ds_layout.free_slots.iter().map(|s| s.init).collect(), |s| s.to_vec());

    let mut lm_out = match config.method {
        FitMethod::LeastSquares | FitMethod::RoutOutlierElimination { .. } => {
            if config.uses_predicted_y_weighting() {
                fit_predicted_y_irls(model, &ds_layout, &mut ds_obs, &init_theta, config)?
            } else {
                solve_with_warm_start(model, &ds_layout, &ds_obs, &init_theta, config)?
            }
        }
        FitMethod::PoissonDeviance => fit_poisson_irls(model, &ds_layout, &ds_obs, &init_theta, config)?,
        FitMethod::RobustCauchy => {
            let mut out = fit_cauchy_irls(model, &ds_layout, &ds_obs, &init_theta, config)?;
            if config.uses_predicted_y_weighting() {
                if !apply_predicted_y_weights(&mut ds_obs, &out.y_hat, config.weighting) {
                    out.status = FitStatus::ImpossibleWeights;
                } else {
                    finalize_cauchy_weighted_residuals(&ds_obs, &mut out, p_d);
                }
            }
            out
        }
    };

    let ss_within_d = if use_rep_moments { obs_ss_within(&ds_obs) } else { 0.0 };
    lm_out.weighted_ss += ss_within_d;
    let df_d = n_d.saturating_sub(p_d);
    let unc = finalize_fit_uncertainty(model, &ds_layout, &ds_obs, &mut lm_out, df_d, ss_within_d, config)?;
    let cov = unc.cov;

    let mut raw_rss_d = NeumaierSum::new();
    for (rec, &yh) in ds_obs.iter().zip(lm_out.y_hat.iter()) {
        let diff = rec.point.y - yh;
        raw_rss_d.add((rec.rep_n as f64) * diff * diff);
        raw_rss_d.add(rec.raw_ss_within);
    }
    let rsd_d = if df_d > 0 && config.method != FitMethod::RobustCauchy { (raw_rss_d.total() / (df_d as f64)).sqrt() } else { f64::NAN };
    let stud_d = if config.method == FitMethod::RobustCauchy {
        Vec::new()
    } else {
        compute_studentized_residuals(&lm_out.j_mat, &lm_out.weighted_res, unc.cov_unscaled.as_ref(), unc.sigma_w, cov.ambiguous)
    };

    let assembled = assemble_dataset_results(
        model,
        &ds_layout,
        ds_slice,
        &ds_obs,
        &lm_out,
        &stud_d,
        &DatasetAssemblyConfig {
            cov_unscaled: if df_d > 0 { unc.cov_unscaled.as_ref() } else { None },
            sigma_w: unc.sigma_w,
            t_crit: unc.t_crit,
            rsd: rsd_d,
            ambiguous: cov.ambiguous || unc.identifiability.fit.is_ambiguous(),
            solver: config,
        },
    );

    let mut ds_result = assembled.ds_results.into_iter().next().ok_or(FitError::EmptyInput)?;
    let derived = assembled.derived_scalars.into_iter().next().unwrap_or_default();
    let poisson_ll = if config.method == FitMethod::PoissonDeviance { poisson_log_likelihood_from_obs(&ds_obs, &lm_out.y_hat) } else { None };
    let obs_moments = ds_obs.iter().map(|o| (o.sqrt_weight, o.rep_n, o.ss_within, o.raw_ss_within)).collect();
    let (ds_pred_nlw, ds_ic) = compute_independent_ic(config, &ds_obs, lm_out.weighted_ss, n_d, p_d, poisson_ll);
    let ds_diag = compute_residual_diagnostics(
        &ds_obs,
        std::slice::from_ref(&ds_result),
        &lm_out.y_hat,
        &ds_result.residuals,
        lm_out.weighted_ss,
        df_d,
        config,
    );
    ds_result.status = lm_out.status;
    ds_result.identifiability = unc.identifiability.fit.clone();
    ds_result.ambiguous = cov.ambiguous || unc.identifiability.fit.is_ambiguous() || lm_out.status == FitStatus::Ambiguous;
    ds_result.iterations = lm_out.iterations;
    ds_result.n_obs = n_d;
    ds_result.k_free = p_d;
    ds_result.df = df_d;
    ds_result.weighted_ss = lm_out.weighted_ss;
    ds_result.aic_c = ds_ic.as_ref().map(|x| x.aicc);
    ds_result.bic = ds_ic.as_ref().map(|x| x.bic);
    ds_result.normality = ds_diag.normality;
    ds_result.normality_anderson_darling = ds_diag.normality_anderson_darling;
    ds_result.normality_lilliefors_ks = ds_diag.normality_lilliefors_ks;
    ds_result.homoscedasticity = ds_diag.homoscedasticity;
    ds_result.lack_of_fit = ds_diag.lack_of_fit;
    ds_result.predicted_y_neg_log_weight_sum = ds_pred_nlw;
    ds_result.poisson_ll = poisson_ll;

    Ok(SingleDatasetOutcome {
        layout: ds_layout,
        theta: lm_out.theta,
        status: lm_out.status,
        iterations: lm_out.iterations,
        weighted_ss: lm_out.weighted_ss,
        cov,
        hougaard: unc.hougaard,
        profile_cis: unc.profile_cis,
        identifiability: unc.identifiability,
        ds_result,
        derived_scalars: derived,
        interpolated_x: assembled.interpolated_x,
        interpolated_y: assembled.interpolated_y,
        obs_moments,
        poisson_ll,
    })
}

pub(in crate::solver) fn fit_independent_datasets(
    model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
    global_layout: ParameterLayout,
    _initial_obs: Vec<ObsRecord>,
) -> Result<NonlinearFitResult, FitError> {
    let k_free = global_layout.k_free();
    let all_column_titles = collect_column_title_values(datasets);
    let outlier_handling = config.effective_outlier_handling();
    let (clean_storage, cauchy_seeds, rout_result, n_outliers_excluded, per_ds_outliers, aborted_sub_fits) =
        if outlier_handling != OutlierHandling::None {
            match run_independent_rout_prefilter(
                model,
                datasets,
                hooks,
                config,
                &all_column_titles,
                config.effective_rout_q_percent(),
                outlier_handling == OutlierHandling::Eliminate,
            )? {
                IndependentRoutPrefilter::Clean { cleaned, seeds, rout, excluded, per_ds_outliers } => {
                    (Some(cleaned), Some(seeds), Some(rout), excluded, per_ds_outliers, None)
                }
                IndependentRoutPrefilter::Aborted { sub_fits } => (None, None, None, 0, Vec::new(), Some(sub_fits)),
            }
        } else {
            (None, None, None, 0, Vec::new(), None)
        };

    let active_datasets = clean_storage.as_deref().unwrap_or(datasets);
    let sub_fits = if let Some(failed_subs) = aborted_sub_fits {
        failed_subs
    } else {
        let mut built = Vec::with_capacity(active_datasets.len());
        for (d_idx, ds) in active_datasets.iter().enumerate() {
            let ds_rules = rules_for_dataset(&config.parameter_rules, ds.data_set_index);
            let seed = cauchy_seeds.as_ref().map(|v| v[d_idx].as_slice());
            built.push(solve_single_dataset_unit(model, ds, hooks, &ds_rules, config, seed, Some(&all_column_titles))?);
        }
        built
    };

    let is_robust = config.method == FitMethod::RobustCauchy;
    let mut global_theta = vec![0.0_f64; k_free];
    let mut global_se = vec![f64::NAN; k_free];
    let mut global_sym_lo = vec![f64::NAN; k_free];
    let mut global_sym_hi = vec![f64::NAN; k_free];
    let mut global_deps = vec![f64::NAN; k_free];
    let mut global_hougaard = vec![f64::NAN; k_free];
    let mut global_profile_cis = vec![(CiBound::Open, CiBound::Open); k_free];
    let mut global_unstable = vec![false; k_free];
    let mut identifiability = FitIdentifiability::NotAssessed;
    let mut global_cov_mat = if is_robust { Vec::new() } else { vec![vec![0.0_f64; k_free]; k_free] };
    let mut global_corr_mat = if is_robust { Vec::new() } else { vec![vec![0.0_f64; k_free]; k_free] };

    let mut total_rank = 0_usize;
    let mut any_impossible_weights = false;
    let mut any_diverged = false;
    let mut any_ambiguous = false;
    let mut any_max_iters = false;
    let mut any_unassessed_free = false;
    let mut max_iterations = 0_usize;
    let mut total_weighted_ss = NeumaierSum::new();
    let mut total_raw_rss = NeumaierSum::new();
    let mut total_y_sum = NeumaierSum::new();
    let mut n_obs = 0_usize;

    let mut ds_results = Vec::with_capacity(sub_fits.len());
    let mut derived_scalars = Vec::with_capacity(sub_fits.len());
    let mut interpolated_x = Vec::new();
    let mut interpolated_y = Vec::new();
    let mut all_points = Vec::new();
    let mut all_obs = Vec::new();
    let mut all_moments = Vec::new();
    let mut all_y_hat = Vec::new();
    let mut all_raw_residuals = Vec::new();
    let mut all_poisson_ll: Option<f64> = Some(0.0);

    let fixed_constants = collect_independent_fixed_constants(model, active_datasets, &sub_fits, config);

    for (d_idx, (mut sub, ds)) in sub_fits.into_iter().zip(active_datasets.iter()).enumerate() {
        sub.ds_result.n_outliers = per_ds_outliers.get(d_idx).copied().unwrap_or(0);
        let p_d = sub.layout.k_free();
        if p_d > 0 && matches!(sub.identifiability.fit, FitIdentifiability::NotAssessed) {
            any_unassessed_free = true;
        }
        let mut l2g = vec![0_usize; p_d];
        for p_idx in 0..global_layout.p_model {
            if let (ParamBinding::Free(g_slot), ParamBinding::Free(l_slot)) = (&global_layout.bindings[d_idx][p_idx], &sub.layout.bindings[0][p_idx])
            {
                l2g[*l_slot] = *g_slot;
                global_theta[*g_slot] = sub.theta[*l_slot];
                global_se[*g_slot] = sub.cov.standard_errors[*l_slot];
                global_sym_lo[*g_slot] = sub.cov.symmetric_ci_lower[*l_slot];
                global_sym_hi[*g_slot] = sub.cov.symmetric_ci_upper[*l_slot];
                global_deps[*g_slot] = sub.cov.dependencies[*l_slot];
                global_hougaard[*g_slot] = sub.hougaard[*l_slot];
                global_profile_cis[*g_slot] = sub.profile_cis[*l_slot];
                global_unstable[*g_slot] = sub.identifiability.unstable.get(*l_slot).copied().unwrap_or(false);
            }
        }
        if !is_robust && !sub.cov.covariance_matrix.is_empty() {
            for (a, &g_a) in l2g.iter().enumerate() {
                for (b, &g_b) in l2g.iter().enumerate() {
                    global_cov_mat[g_a][g_b] = sub.cov.covariance_matrix[a][b];
                    global_corr_mat[g_a][g_b] = sub.cov.correlation_matrix[a][b];
                }
            }
        }

        identifiability::include_dataset(&mut identifiability, sub.identifiability.fit, &ds.label);
        total_rank += sub.cov.numerical_rank;
        any_impossible_weights |= sub.status == FitStatus::ImpossibleWeights;
        any_diverged |= sub.status == FitStatus::Diverged;
        any_ambiguous |= sub.cov.ambiguous || sub.status == FitStatus::Ambiguous;
        any_max_iters |= sub.status == FitStatus::HitMaxIters;
        max_iterations = max_iterations.max(sub.iterations);
        total_weighted_ss.add(sub.weighted_ss);
        total_raw_rss.add(sub.ds_result.ss_residual);
        all_poisson_ll = all_poisson_ll.zip(sub.poisson_ll).map(|(a, b)| a + b);

        for (((pt, &yh), &res), &obs_mom) in
            ds.points.iter().zip(sub.ds_result.y_hat.iter()).zip(sub.ds_result.residuals.iter()).zip(sub.obs_moments.iter())
        {
            all_obs.push(ObsRecord {
                point: pt.clone(),
                ds_local_idx: d_idx,
                sqrt_weight: obs_mom.0,
                rep_n: obs_mom.1,
                ss_within: obs_mom.2,
                raw_ss_within: obs_mom.3,
            });
            all_points.push(pt.clone());
            all_y_hat.push(yh);
            all_raw_residuals.push(res);
            all_moments.push((obs_mom.1, obs_mom.3));
            total_y_sum.add((obs_mom.1 as f64) * pt.y);
            n_obs += obs_mom.1;
        }

        ds_results.push(sub.ds_result);
        derived_scalars.push(sub.derived_scalars);
        interpolated_x.extend(sub.interpolated_x);
        interpolated_y.extend(sub.interpolated_y);
    }

    if any_unassessed_free && matches!(identifiability, FitIdentifiability::Identifiable) {
        identifiability = FitIdentifiability::NotAssessed;
    }
    let status = resolve_independent_status(any_impossible_weights, any_diverged, any_max_iters, any_ambiguous);
    let cov = CovarianceDiagnostics {
        standard_errors: global_se,
        symmetric_ci_lower: global_sym_lo,
        symmetric_ci_upper: global_sym_hi,
        covariance_matrix: global_cov_mat,
        correlation_matrix: global_corr_mat,
        dependencies: global_deps,
        numerical_rank: total_rank,
        ambiguous: any_ambiguous,
    };
    let is_robust = config.method == FitMethod::RobustCauchy;
    let inference_availability = if is_robust { InferenceAvailability::NotApplicableInRobustMode } else { InferenceAvailability::Available };
    let report_all = config.identifiability_mode == Some(IdentifiabilityMode::ReportAll);
    let parameters = build_fitted_parameter_summaries(
        &global_layout,
        &global_theta,
        &cov,
        &global_hougaard,
        &global_profile_cis,
        &global_unstable,
        inference_availability,
        report_all,
    );

    let raw_rss = total_raw_rss.total();
    let weighted_ss = total_weighted_ss.total();
    let df = n_obs.saturating_sub(k_free);
    let y_mean = if n_obs > 0 { total_y_sum.total() / (n_obs as f64) } else { 0.0 };
    let mut tss_acc = NeumaierSum::new();
    for (pt, &(rep_n, ss_w)) in all_points.iter().zip(all_moments.iter()) {
        let d = pt.y - y_mean;
        tss_acc.add((rep_n as f64) * d * d);
        tss_acc.add(ss_w);
    }
    let tss = tss_acc.total();
    let r_squared = compute_r_squared(raw_rss, tss, n_obs);
    let rsd = if df > 0 && !is_robust { (raw_rss / (df as f64)).sqrt() } else { f64::NAN };
    let rsdr = compute_independent_rsdr(is_robust, &ds_results, &all_obs, &all_raw_residuals, k_free);
    let (predicted_y_neg_log_weight_sum, ic) = compute_independent_ic(config, &all_obs, weighted_ss, n_obs, k_free, all_poisson_ll);

    let res_diag = compute_residual_diagnostics(&all_obs, &ds_results, &all_y_hat, &all_raw_residuals, weighted_ss, df, config);

    Ok(NonlinearFitResult {
        equation_id: model.id().to_owned(),
        equation_name: model.name().to_owned(),
        fit_method: config.method,
        inference_availability,
        status,
        identifiability,
        iterations: max_iterations,
        n_obs,
        n_outliers_excluded,
        k_free,
        df,
        rss: raw_rss,
        weighted_ss,
        predicted_y_neg_log_weight_sum,
        rsd,
        rsdr,
        r_squared,
        aic_c: ic.as_ref().map(|x| x.aicc),
        bic: ic.as_ref().map(|x| x.bic),
        parameters,
        fixed_constants,
        covariance: cov,
        datasets: ds_results,
        derived_scalars,
        runs_test: res_diag.runs_test,
        lack_of_fit: res_diag.lack_of_fit,
        normality: res_diag.normality,
        normality_anderson_darling: res_diag.normality_anderson_darling,
        normality_lilliefors_ks: res_diag.normality_lilliefors_ks,
        homoscedasticity: res_diag.homoscedasticity,
        rout: rout_result,
        interpolated_x,
        interpolated_y,
        has_shared_parameters: false,
    })
}

fn resolve_independent_status(any_impossible_weights: bool, any_diverged: bool, any_max_iters: bool, any_ambiguous: bool) -> FitStatus {
    if any_impossible_weights {
        FitStatus::ImpossibleWeights
    } else if any_diverged {
        FitStatus::Diverged
    } else if any_max_iters {
        FitStatus::HitMaxIters
    } else if any_ambiguous {
        FitStatus::Ambiguous
    } else {
        FitStatus::Converged
    }
}

fn compute_independent_rsdr(
    is_robust: bool,
    ds_results: &[FittedDatasetResult],
    all_obs: &[ObsRecord],
    all_raw_residuals: &[f64],
    k_free: usize,
) -> Option<f64> {
    if !is_robust {
        return None;
    }
    if ds_results.len() == 1 {
        return ds_results[0].rsdr;
    }
    let abs_res: Vec<f64> = all_obs.iter().zip(all_raw_residuals.iter()).map(|(o, r)| (r * o.sqrt_weight).abs()).collect();
    Some(compute_rsdr(&abs_res, k_free))
}

fn compute_independent_ic(
    config: &SolverConfig,
    all_obs: &[ObsRecord],
    weighted_ss: f64,
    n_obs: usize,
    k_free: usize,
    all_poisson_ll: Option<f64>,
) -> (Option<f64>, Option<crate::diagnostics::InformationCriteria>) {
    let is_robust = config.method == FitMethod::RobustCauchy;
    let pred_y_nlw = if !is_robust && config.method != FitMethod::PoissonDeviance && config.uses_predicted_y_weighting() {
        predicted_y_neg_log_weight_sum(all_obs)
    } else {
        None
    };
    let ic = if is_robust {
        None
    } else if config.method == FitMethod::PoissonDeviance {
        all_poisson_ll.and_then(|ll| information_criteria_from_poisson_ll(ll, n_obs, k_free))
    } else if let Some(neg_log_w) = pred_y_nlw {
        compute_information_criteria_with_log_weight_penalty(weighted_ss, neg_log_w, n_obs, k_free).ok()
    } else {
        compute_information_criteria(weighted_ss, n_obs, k_free).ok()
    };
    (pred_y_nlw, ic)
}

fn collect_independent_fixed_constants(
    model: &dyn ParametricModel,
    active_datasets: &[DatasetSeries],
    sub_fits: &[SingleDatasetOutcome],
    config: &SolverConfig,
) -> Vec<(String, f64, Vec<ParameterNote>)> {
    if active_datasets.len() <= 1 {
        return sub_fits.first().map(|s| s.layout.fixed_constants.clone()).unwrap_or_default();
    }
    let default_bounds = model.default_bounds();
    let mut fixed_constants = Vec::new();
    for (p_idx, pname) in model.param_names().iter().enumerate() {
        let mut entries: Vec<(usize, f64, Vec<ParameterNote>)> = Vec::new();
        for (ds, sub) in active_datasets.iter().zip(sub_fits.iter()) {
            if let Some((_, val, notes)) = sub.layout.fixed_constants.iter().find(|(n, _, _)| n == pname) {
                entries.push((ds.data_set_index, *val, notes.clone()));
            }
        }
        if entries.is_empty() {
            continue;
        }
        let explicit_rule = config.parameter_rules.iter().find(|r| r.name.eq_ignore_ascii_case(pname));
        let is_global_constant_rule = match explicit_rule.map(|r| &r.scope) {
            Some(ParameterScope::Constant(_) | ParameterScope::HookedConstant(_)) => true,
            Some(ParameterScope::PerDatasetConstant(_) | ParameterScope::ColumnTitleConstant) => false,
            None | Some(ParameterScope::Local | ParameterScope::SharedGlobal) => {
                let def_b = default_bounds.get(p_idx).copied().unwrap_or((None, None));
                let lb = explicit_rule.and_then(|r| r.lower_bound).or(def_b.0);
                let ub = explicit_rule.and_then(|r| r.upper_bound).or(def_b.1);
                matches!((lb, ub), (Some(l), Some(u)) if l == u)
            }
        };
        if is_global_constant_rule {
            fixed_constants.push((pname.clone(), entries[0].1, entries[0].2.clone()));
        } else {
            for (ds_ext_idx, val, notes) in entries {
                fixed_constants.push((per_dataset_parameter_key(pname, ds_ext_idx), val, notes));
            }
        }
    }
    fixed_constants
}

enum IndependentRoutPrefilter {
    Clean { cleaned: Vec<DatasetSeries>, seeds: Vec<Vec<f64>>, rout: RoutDetectionResult, excluded: usize, per_ds_outliers: Vec<usize> },
    Aborted { sub_fits: Vec<SingleDatasetOutcome> },
}

fn run_independent_rout_prefilter(
    model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
    column_titles: &[f64],
    fdr_q_percent: f64,
    eliminate_outliers: bool,
) -> Result<IndependentRoutPrefilter, FitError> {
    let mut cauchy_cfg = config.clone();
    cauchy_cfg.method = FitMethod::RobustCauchy;
    cauchy_cfg.outlier_handling = OutlierHandling::None;
    cauchy_cfg.compute_hougaard = false;
    cauchy_cfg.compute_profile_ci = false;
    cauchy_cfg.compute_bands = false;
    cauchy_cfg.interpolate_unknowns = false;

    let mut stage1_subs = Vec::with_capacity(datasets.len());
    let mut any_stage1_failed = false;
    for ds in datasets {
        let ds_rules = rules_for_dataset(&config.parameter_rules, ds.data_set_index);
        let sub = solve_single_dataset_unit(model, ds, hooks, &ds_rules, &cauchy_cfg, None, Some(column_titles))?;
        if matches!(sub.status, FitStatus::ImpossibleWeights | FitStatus::Diverged | FitStatus::BadInitialValues) {
            any_stage1_failed = true;
        }
        stage1_subs.push(sub);
    }
    if any_stage1_failed {
        return Ok(IndependentRoutPrefilter::Aborted { sub_fits: stage1_subs });
    }

    let mut cleaned = Vec::with_capacity(datasets.len());
    let mut seeds = Vec::with_capacity(datasets.len());
    let mut per_ds_outliers = Vec::with_capacity(datasets.len());
    let mut combined_outliers = Vec::new();
    let mut combined_audit = Vec::new();
    let mut rsdr_weighted_sq = 0.0_f64;
    let mut total_rout_df = 0_usize;
    let mut remaining_obs = 0_usize;
    let mut total_pts = 0_usize;
    let mut pt_offset = 0_usize;

    for (ds, sub) in datasets.iter().zip(stage1_subs) {
        let p_d = sub.layout.k_free();
        let sqrt_w_d: Vec<f64> = sub.obs_moments.iter().map(|&(sw, _, _, _)| sw).collect();
        let w_res_d: Vec<f64> = sub.ds_result.residuals.iter().zip(sqrt_w_d.iter()).map(|(&r, &sw)| r * sw).collect();
        let rout_d = rout_detect_outliers_mb2006(&ds.points, &w_res_d, &sqrt_w_d, p_d, fdr_q_percent)?;
        per_ds_outliers.push(rout_d.outlier_indices.len());

        let mut is_out = vec![false; ds.points.len()];
        for &local_idx in &rout_d.outlier_indices {
            is_out[local_idx] = true;
            combined_outliers.push(pt_offset + local_idx);
        }
        for mut entry in rout_d.audit {
            entry.index += pt_offset;
            combined_audit.push(entry);
        }
        if rout_d.df > 0 && rout_d.rsdr.is_finite() {
            rsdr_weighted_sq += (rout_d.df as f64) * rout_d.rsdr * rout_d.rsdr;
            total_rout_df += rout_d.df;
        }

        let mut ds_c = ds.clone();
        if eliminate_outliers {
            let mut idx_iter = is_out.iter().copied();
            ds_c.points.retain(|_| !idx_iter.next().unwrap_or(false));
            if ds_c.points.len() <= p_d {
                return Err(FitError::InsufficientData { required: p_d + 1, actual: ds_c.points.len() });
            }
        }
        total_pts += ds.points.len();
        remaining_obs += ds_c.points.len();
        pt_offset += ds.points.len();
        cleaned.push(ds_c);
        seeds.push(sub.theta);
    }

    let combined_rsdr = if total_rout_df > 0 { (rsdr_weighted_sq / (total_rout_df as f64)).sqrt() } else { 0.0 };
    let rout = RoutDetectionResult {
        rsdr: combined_rsdr,
        df: total_rout_df,
        q_threshold_percent: fdr_q_percent,
        outlier_indices: combined_outliers,
        audit: combined_audit,
    };
    let excluded = total_pts - remaining_obs;
    Ok(IndependentRoutPrefilter::Clean { cleaned, seeds, rout, excluded, per_ds_outliers })
}
