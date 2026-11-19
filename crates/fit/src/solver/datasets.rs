//! Shared-parameter global fits and independent per-dataset fits.
//!
//! References:
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.

use curvebook_doc::{CiBound, FitMethod, ParameterRule};
use curvebook_math::sum::NeumaierSum;

use crate::diagnostics::{CovarianceDiagnostics, RoutDetectionResult, compute_information_criteria, compute_rsdr};
use crate::error::{FitError, FitIdentifiability, FitStatus};
use crate::library::ParametricModel;
use crate::solver::assembly::{DatasetAssemblyConfig, assemble_dataset_results, build_fitted_parameter_summaries};
use crate::solver::fit::{finalize_fit_uncertainty, solve_lm_by_method};
use crate::solver::identifiability;
use crate::solver::identifiability::Assessment;
use crate::solver::inference::{
    compute_residual_diagnostics, compute_studentized_residuals, information_criteria_from_poisson_ll, poisson_information_criteria,
    poisson_log_likelihood_from_obs, rout_detect_outliers_mb2006,
};
use crate::solver::irls::{fit_cauchy_irls, fit_poisson_irls};
use crate::solver::layout::{
    ObsRecord, ParamBinding, ParameterLayout, build_observations, obs_replicate_count, obs_ss_within, rules_for_dataset,
    uses_replicate_summary_moments,
};
use crate::solver::lm::solve_with_warm_start;
use crate::solver::results::{FittedDatasetResult, InferenceAvailability, InterpolatedUnknown, NonlinearFitResult, SolverConfig};
use crate::types::{DataPoint, DatasetSeries, DerivedScalar, HookContext};

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
    let parameters = build_fitted_parameter_summaries(
        &layout,
        &lm_out.theta,
        &unc.cov,
        &unc.hougaard,
        &unc.profile_cis,
        &unc.identifiability.unstable,
        inference_availability,
    );

    let mut raw_rss_acc = NeumaierSum::new();
    let mut y_mean_acc = NeumaierSum::new();
    for (rec, &yh) in obs.iter().zip(lm_out.y_hat.iter()) {
        let diff = rec.point.y - yh;
        let w_rep = rec.rep_n as f64;
        raw_rss_acc.add(w_rep * diff * diff);
        raw_rss_acc.add(rec.ss_within);
        y_mean_acc.add(w_rep * rec.point.y);
    }
    let raw_rss = raw_rss_acc.total();
    let y_mean = y_mean_acc.total() / (n_obs as f64);
    let mut tss_acc = NeumaierSum::new();
    for rec in &obs {
        let d = rec.point.y - y_mean;
        tss_acc.add((rec.rep_n as f64) * d * d);
        tss_acc.add(rec.ss_within);
    }
    let tss = tss_acc.total();
    let is_robust = config.method == FitMethod::RobustCauchy;
    let r_squared = if tss > 0.0 { 1.0 - raw_rss / tss } else { 1.0 };
    let rsd = if df > 0 && !is_robust { (raw_rss / (df as f64)).sqrt() } else { f64::NAN };

    let ic = if is_robust {
        None
    } else if config.method == FitMethod::PoissonDeviance {
        poisson_information_criteria(&obs, &lm_out.y_hat, k_free)
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
    let clean_points: Vec<DataPoint> = obs.iter().map(|o| o.point.clone()).collect();
    let res_diag = compute_residual_diagnostics(&obs, &clean_points, &lm_out.y_hat, &raw_residuals_all, lm_out.weighted_ss, df, config);

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
        rsd,
        rsdr,
        r_squared,
        aic_c: ic.as_ref().map(|x| x.aicc),
        bic: ic.as_ref().map(|x| x.bic),
        parameters,
        fixed_constants: layout.fixed_constants,
        covariance: cov,
        datasets: assembled.ds_results,
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
    pub(in crate::solver) obs_moments: Vec<(f64, usize, f64)>,
    pub(in crate::solver) poisson_ll: Option<f64>,
}

pub(in crate::solver) fn solve_single_dataset_unit(
    model: &dyn ParametricModel,
    ds: &DatasetSeries,
    hooks: &HookContext,
    ds_rules: &[ParameterRule],
    config: &SolverConfig,
    seed_override: Option<&[f64]>,
) -> Result<SingleDatasetOutcome, FitError> {
    let ds_slice = std::slice::from_ref(ds);
    let ds_layout = ParameterLayout::build(model, ds_slice, hooks, ds_rules)?;
    let p_d = ds_layout.k_free();
    let use_rep_moments = uses_replicate_summary_moments(config);
    let ds_obs = build_observations(ds_slice, config.weighting, use_rep_moments)?;
    let n_d = obs_replicate_count(&ds_obs);
    let min_d = if matches!(config.method, FitMethod::RoutOutlierElimination { .. }) { p_d + 1 } else { p_d };
    if n_d < min_d {
        return Err(FitError::InsufficientData { required: min_d, actual: n_d });
    }
    let ss_within_d = if use_rep_moments { obs_ss_within(&ds_obs) } else { 0.0 };
    let init_theta: Vec<f64> = seed_override.map_or_else(|| ds_layout.free_slots.iter().map(|s| s.init).collect(), |s| s.to_vec());

    let mut lm_out = match config.method {
        FitMethod::LeastSquares | FitMethod::RoutOutlierElimination { .. } => solve_with_warm_start(model, &ds_layout, &ds_obs, &init_theta, config)?,
        FitMethod::PoissonDeviance => fit_poisson_irls(model, &ds_layout, &ds_obs, &init_theta, config)?,
        FitMethod::RobustCauchy => fit_cauchy_irls(model, &ds_layout, &ds_obs, &init_theta, config)?,
    };

    lm_out.weighted_ss += ss_within_d;
    let df_d = n_d.saturating_sub(p_d);
    let unc = finalize_fit_uncertainty(model, &ds_layout, &ds_obs, &mut lm_out, df_d, ss_within_d, config)?;
    let cov = unc.cov;

    let mut raw_rss_d = NeumaierSum::new();
    for (rec, &yh) in ds_obs.iter().zip(lm_out.y_hat.iter()) {
        let diff = rec.point.y - yh;
        raw_rss_d.add((rec.rep_n as f64) * diff * diff);
        raw_rss_d.add(rec.ss_within);
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

    let ds_result = assembled.ds_results.into_iter().next().ok_or(FitError::EmptyInput)?;
    let derived = assembled.derived_scalars.into_iter().next().unwrap_or_default();
    let poisson_ll = if config.method == FitMethod::PoissonDeviance { poisson_log_likelihood_from_obs(&ds_obs, &lm_out.y_hat) } else { None };
    let obs_moments = ds_obs.iter().map(|o| (o.sqrt_weight, o.rep_n, o.ss_within)).collect();

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
    let (clean_storage, cauchy_seeds, rout_result, n_outliers_excluded) = if let FitMethod::RoutOutlierElimination { fdr_q_percent } = config.method {
        let mut cauchy_cfg = config.clone();
        cauchy_cfg.method = FitMethod::RobustCauchy;
        cauchy_cfg.compute_hougaard = false;
        cauchy_cfg.compute_profile_ci = false;
        cauchy_cfg.compute_bands = false;
        cauchy_cfg.interpolate_unknowns = false;

        let mut cleaned = Vec::with_capacity(datasets.len());
        let mut seeds = Vec::with_capacity(datasets.len());
        let mut combined_outliers = Vec::new();
        let mut combined_audit = Vec::new();
        let mut rsdr_weighted_sq = 0.0_f64;
        let mut total_rout_df = 0_usize;
        let mut remaining_obs = 0_usize;
        let mut total_pts = 0_usize;
        let mut pt_offset = 0_usize;

        for ds in datasets {
            let ds_rules = rules_for_dataset(&config.parameter_rules, ds.data_set_index);
            let sub = solve_single_dataset_unit(model, ds, hooks, &ds_rules, &cauchy_cfg, None)?;
            let p_d = sub.layout.k_free();
            let sqrt_w_d: Vec<f64> = sub.obs_moments.iter().map(|&(sw, _, _)| sw).collect();
            let w_res_d: Vec<f64> = sub.ds_result.residuals.iter().zip(sqrt_w_d.iter()).map(|(&r, &sw)| r * sw).collect();
            let rout_d = rout_detect_outliers_mb2006(&ds.points, &w_res_d, &sqrt_w_d, p_d, fdr_q_percent)?;

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
            let mut idx_iter = is_out.iter().copied();
            ds_c.points.retain(|_| !idx_iter.next().unwrap_or(false));
            if ds_c.points.len() <= p_d {
                return Err(FitError::InsufficientData { required: p_d + 1, actual: ds_c.points.len() });
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
        (Some(cleaned), Some(seeds), Some(rout), excluded)
    } else {
        (None, None, None, 0)
    };

    let active_datasets = clean_storage.as_deref().unwrap_or(datasets);
    let mut sub_fits = Vec::with_capacity(active_datasets.len());
    for (d_idx, ds) in active_datasets.iter().enumerate() {
        let ds_rules = rules_for_dataset(&config.parameter_rules, ds.data_set_index);
        let seed = cauchy_seeds.as_ref().map(|v| v[d_idx].as_slice());
        sub_fits.push(solve_single_dataset_unit(model, ds, hooks, &ds_rules, config, seed)?);
    }

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
    let mut any_diverged = false;
    let mut any_ambiguous = false;
    let mut any_max_iters = false;
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

    for (d_idx, (sub, ds)) in sub_fits.into_iter().zip(active_datasets.iter()).enumerate() {
        let p_d = sub.layout.k_free();
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
            all_obs.push(ObsRecord { point: pt.clone(), ds_local_idx: d_idx, sqrt_weight: obs_mom.0, rep_n: obs_mom.1, ss_within: obs_mom.2 });
            all_points.push(pt.clone());
            all_y_hat.push(yh);
            all_raw_residuals.push(res);
            all_moments.push((obs_mom.1, obs_mom.2));
            total_y_sum.add((obs_mom.1 as f64) * pt.y);
            n_obs += obs_mom.1;
        }

        ds_results.push(sub.ds_result);
        derived_scalars.push(sub.derived_scalars);
        interpolated_x.extend(sub.interpolated_x);
        interpolated_y.extend(sub.interpolated_y);
    }

    let status = if any_diverged {
        FitStatus::Diverged
    } else if any_ambiguous {
        FitStatus::Ambiguous
    } else if any_max_iters {
        FitStatus::HitMaxIters
    } else {
        FitStatus::Converged
    };
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
    let parameters = build_fitted_parameter_summaries(
        &global_layout,
        &global_theta,
        &cov,
        &global_hougaard,
        &global_profile_cis,
        &global_unstable,
        inference_availability,
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
    let r_squared = if tss > 0.0 { 1.0 - raw_rss / tss } else { 1.0 };
    let rsd = if df > 0 && !is_robust { (raw_rss / (df as f64)).sqrt() } else { f64::NAN };
    let rsdr = if is_robust {
        if ds_results.len() == 1 {
            ds_results[0].rsdr
        } else {
            let abs_res: Vec<f64> = all_obs.iter().zip(all_raw_residuals.iter()).map(|(o, r)| (r * o.sqrt_weight).abs()).collect();
            Some(compute_rsdr(&abs_res, k_free))
        }
    } else {
        None
    };
    let ic = if is_robust {
        None
    } else if config.method == FitMethod::PoissonDeviance {
        all_poisson_ll.and_then(|ll| information_criteria_from_poisson_ll(ll, n_obs, k_free))
    } else {
        compute_information_criteria(weighted_ss, n_obs, k_free).ok()
    };

    let res_diag = compute_residual_diagnostics(&all_obs, &all_points, &all_y_hat, &all_raw_residuals, weighted_ss, df, config);

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
        rsd,
        rsdr,
        r_squared,
        aic_c: ic.as_ref().map(|x| x.aicc),
        bic: ic.as_ref().map(|x| x.bic),
        parameters,
        fixed_constants: global_layout.fixed_constants,
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
    })
}
