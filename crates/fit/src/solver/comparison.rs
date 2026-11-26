//! Nested-model comparison by extra-sum-of-squares *F* test, AICc, and likelihood ratio.
//!
//! References:
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.

use std::collections::BTreeSet;

use curvebook_doc::{
    FitFlagRules, FitMethod, ModelComparisonMethod, ModelComparisonQuestion, ModelComparisonSpec, OutlierHandling, ParameterRule, ParameterScope,
    ReplicateFitMode,
};

use crate::diagnostics::{
    compare_models_aicc, compare_models_f_test, compare_models_lrt, compare_models_lrt_deviance, compare_models_lrt_from_neg2ll,
    gaussian_neg2_ll_core,
};
use crate::error::{FitError, FitStatus};
use crate::library::{ParametricModel, get_builtin_model};
use crate::solver::fit::fit_nonlinear;
use crate::solver::identifiability::is_fit_well_defined;
use crate::solver::inference::compare_poisson_aicc;
use crate::solver::results::{InferenceAvailability, ModelComparisonOutcome, ModelComparisonResult, NonlinearFitResult, SolverConfig};
use crate::types::{DataPoint, DatasetSeries, HookContext, collapse_datasets_to_row_means, filter_datasets_by_range};

pub(in crate::solver) fn comparison_test_label(spec: &ModelComparisonSpec) -> &'static str {
    match spec {
        ModelComparisonSpec::ExtraSumOfSquaresF { .. } => "Extra sum-of-squares F test",
        ModelComparisonSpec::Aicc { .. } => "AICc",
        ModelComparisonSpec::LikelihoodRatioTest { .. } => "Likelihood Ratio Test",
    }
}

/// Resolve the `(simpler_null_fit, complex_alt_fit, swapped)` role assignment for nested
/// model comparisons (`ExtraSumOfSquaresF` / `LikelihoodRatioTest`), where the model with more
/// residual degrees of freedom serves as the simpler null hypothesis.
#[must_use]
pub fn ordered_nested_roles<'a>(
    primary: &'a NonlinearFitResult,
    alt: &'a NonlinearFitResult,
) -> (&'a NonlinearFitResult, &'a NonlinearFitResult, bool) {
    if primary.df >= alt.df { (primary, alt, false) } else { (alt, primary, true) }
}

/// Filter `config.parameter_rules` down to parameter names defined on `alt_model`,
/// preserving strict `#114` validation in [`ParameterLayout::build`](crate::solver::layout::ParameterLayout::build) while allowing
/// model comparisons across models with different parameter sets (e.g. 4PL vs 3PL).
fn filter_config_rules_for_alt_model(config: &SolverConfig, alt_model: &dyn ParametricModel) -> SolverConfig {
    let alt_params = alt_model.param_names();
    let mut alt_config = config.clone();
    alt_config.parameter_rules.retain(|rule| alt_params.iter().any(|p| p.eq_ignore_ascii_case(&rule.name)));
    alt_config
}

/// Compare primary fitted model against an alternative nested/non-nested model via Extra-SS *F*,
/// $\text{AIC}_c$, or Likelihood Ratio Test (`ModelComparisonSpec`).
///
/// Extra-sum-of-squares *F* (Seber & Wild 1989; Motulsky & Christopoulos 2004), AICc
/// (Hurvich & Tsai 1989; Burnham & Anderson 2002), and the Gaussian or deviance likelihood
/// ratio test (Wilks 1938). Full references are in [`crate::diagnostics`]. The *F* and LRT
/// paths order the two fits by residual degrees of freedom (`ordered_nested_roles`) and assume
/// they are nested on identical observations (`primary.n_obs == alt.n_obs`, identical ROUT
/// excluded-point sets).
///
/// Note: for least-squares fits, all three statistics are computed from `weighted_ss`.
/// Under `FitMethod::PoissonDeviance`, `weighted_ss` is the Poisson deviance (used for
/// deviance LRT), `Aicc` uses exact Poisson log-likelihood with `K = k_free` (dispersion = 1),
/// and `ExtraSumOfSquaresF` is rejected because OLS extra-SS *F* is invalid for Poisson errors.
/// Mixed error models (`PoissonDeviance` vs Gaussian/OLS/WLS) and mismatched observation/outlier
/// subsets (for example two ROUT fits that excluded different points) are likewise rejected. If
/// **either** the primary or alternative model uses `FitMethod::RobustCauchy`, returns
/// `FitError::InvalidParameter` carrying `NotApplicableInRobustMode` (use
/// [`compare_models_checked`] when a typed [`ModelComparisonResult`] is desired).
pub fn compare_models(
    primary: &NonlinearFitResult,
    alt_model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
    spec: &ModelComparisonSpec,
) -> Result<(NonlinearFitResult, ModelComparisonOutcome), FitError> {
    if !matches!(primary.status, FitStatus::Converged | FitStatus::Ambiguous) {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!("Cannot compare models when the primary fit did not converge ({:?})", primary.status),
        });
    }
    if primary.is_robust_cauchy() || config.method == FitMethod::RobustCauchy {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: "NotApplicableInRobustMode: F, AICc, and LRT model comparisons are not applicable under Robust (Cauchy) regression".to_owned(),
        });
    }
    let primary_is_poisson = primary.fit_method == FitMethod::PoissonDeviance;
    let config_is_poisson = config.method == FitMethod::PoissonDeviance;
    if primary_is_poisson != config_is_poisson {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: "Cannot compare models fitted under different error models (PoissonDeviance vs Gaussian/least-squares)".to_owned(),
        });
    }
    if config_is_poisson && matches!(spec, ModelComparisonSpec::ExtraSumOfSquaresF { .. }) {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: "Extra sum-of-squares F test is undefined for PoissonDeviance fits; use LikelihoodRatioTest or Aicc instead".to_owned(),
        });
    }
    let alt_config = filter_config_rules_for_alt_model(config, alt_model);
    let alt = fit_nonlinear(alt_model, datasets, hooks, &alt_config)?;
    if !matches!(alt.status, FitStatus::Converged | FitStatus::Ambiguous) {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!("Cannot compare models when the alternative fit did not converge ({:?})", alt.status),
        });
    }
    if alt.is_robust_cauchy() {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: "NotApplicableInRobustMode: F, AICc, and LRT model comparisons are not applicable under Robust (Cauchy) regression".to_owned(),
        });
    }
    if primary.fit_method != alt.fit_method {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!("Cannot compare models fitted under different methods ({:?} vs {:?})", primary.fit_method, alt.fit_method),
        });
    }
    if primary.n_obs != alt.n_obs || primary.n_outliers_excluded != alt.n_outliers_excluded {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!(
                "Cannot compare models fitted to different observation counts ({} vs {}) or excluded outlier counts ({} vs {})",
                primary.n_obs, alt.n_obs, primary.n_outliers_excluded, alt.n_outliers_excluded
            ),
        });
    }
    if (primary.n_outliers_excluded > 0 || alt.n_outliers_excluded > 0)
        && let (Some(rout_a), Some(rout_b)) = (&primary.rout, &alt.rout)
    {
        let mut idx_a = rout_a.outlier_indices.clone();
        let mut idx_b = rout_b.outlier_indices.clone();
        idx_a.sort_unstable();
        idx_b.sort_unstable();
        if idx_a != idx_b {
            return Err(FitError::InvalidParameter {
                name: "comparison",
                reason: String::from("Cannot compare ROUT models that excluded different outlier points"),
            });
        }
    }
    let method = spec_to_method(spec);
    let outcome = compare_validated_pair(primary, &alt, config, method)?;
    Ok((alt, outcome))
}

fn spec_to_method(spec: &ModelComparisonSpec) -> ModelComparisonMethod {
    match spec {
        ModelComparisonSpec::ExtraSumOfSquaresF { .. } => ModelComparisonMethod::ExtraSumOfSquaresF,
        ModelComparisonSpec::Aicc { .. } => ModelComparisonMethod::Aicc,
        ModelComparisonSpec::LikelihoodRatioTest { .. } => ModelComparisonMethod::LikelihoodRatioTest,
    }
}

fn compare_validated_pair(
    fit_a: &NonlinearFitResult,
    fit_b: &NonlinearFitResult,
    config: &SolverConfig,
    method: ModelComparisonMethod,
) -> Result<ModelComparisonOutcome, FitError> {
    let uses_pred_y_penalty = fit_a.predicted_y_neg_log_weight_sum.is_some() || fit_b.predicted_y_neg_log_weight_sum.is_some();
    match method {
        ModelComparisonMethod::ExtraSumOfSquaresF => {
            let (null_fit, complex_fit, _) = ordered_nested_roles(fit_a, fit_b);
            ensure_nested_ss_order(null_fit, complex_fit)?;
            Ok(ModelComparisonOutcome::ExtraSumOfSquaresF(compare_models_f_test(
                null_fit.weighted_ss,
                null_fit.df,
                complex_fit.weighted_ss,
                complex_fit.df,
                config.confidence.alpha(),
            )?))
        }
        ModelComparisonMethod::Aicc => {
            if config.method == FitMethod::PoissonDeviance || uses_pred_y_penalty {
                let aicc_a = fit_a.aic_c.ok_or_else(|| FitError::InsufficientData { required: fit_a.k_free + 2, actual: fit_a.n_obs })?;
                let aicc_b = fit_b.aic_c.ok_or_else(|| FitError::InsufficientData { required: fit_b.k_free + 2, actual: fit_b.n_obs })?;
                Ok(ModelComparisonOutcome::Aicc(compare_poisson_aicc(aicc_a, aicc_b)?))
            } else {
                Ok(ModelComparisonOutcome::Aicc(compare_models_aicc(fit_a.weighted_ss, fit_a.k_free, fit_b.weighted_ss, fit_b.k_free, fit_a.n_obs)?))
            }
        }
        ModelComparisonMethod::LikelihoodRatioTest => {
            let (null_fit, complex_fit, _) = ordered_nested_roles(fit_a, fit_b);
            let res = if config.method == FitMethod::PoissonDeviance {
                ensure_nested_ss_order(null_fit, complex_fit)?;
                compare_models_lrt_deviance(null_fit.weighted_ss, null_fit.df, complex_fit.weighted_ss, complex_fit.df, config.confidence.alpha())?
            } else if let (Some(nlw_null), Some(nlw_complex)) = (null_fit.predicted_y_neg_log_weight_sum, complex_fit.predicted_y_neg_log_weight_sum)
            {
                let neg2ll_null = gaussian_neg2_ll_core(null_fit.weighted_ss, nlw_null, fit_a.n_obs)?;
                let neg2ll_complex = gaussian_neg2_ll_core(complex_fit.weighted_ss, nlw_complex, fit_a.n_obs)?;
                if complex_fit.df < null_fit.df && neg2ll_complex > neg2ll_null + 1e-6 * neg2ll_null.abs().max(1.0) {
                    return Err(FitError::InvalidParameter {
                        name: "comparison",
                        reason: format!(
                            "Nested model comparison failed: complex model (df = {}, -2 ln L = {:.6}) fits worse than simpler model (df = {}, -2 ln L = {:.6})",
                            complex_fit.df, neg2ll_complex, null_fit.df, neg2ll_null
                        ),
                    });
                }
                compare_models_lrt_from_neg2ll(neg2ll_null, null_fit.df, neg2ll_complex, complex_fit.df, config.confidence.alpha())?
            } else {
                ensure_nested_ss_order(null_fit, complex_fit)?;
                compare_models_lrt(
                    null_fit.weighted_ss,
                    null_fit.df,
                    complex_fit.weighted_ss,
                    complex_fit.df,
                    fit_a.n_obs,
                    config.confidence.alpha(),
                )?
            };
            Ok(ModelComparisonOutcome::LikelihoodRatioTest(res))
        }
    }
}

fn ensure_nested_ss_order(null_fit: &NonlinearFitResult, complex_fit: &NonlinearFitResult) -> Result<(), FitError> {
    if complex_fit.df < null_fit.df && complex_fit.weighted_ss > null_fit.weighted_ss + 1e-6 * null_fit.weighted_ss.abs().max(1.0) {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!(
                "Nested model comparison failed: complex model (df = {}, objective = {:.6}) fits worse than simpler model (df = {}, objective = {:.6})",
                complex_fit.df, complex_fit.weighted_ss, null_fit.df, null_fit.weighted_ss
            ),
        });
    }
    Ok(())
}

/// Compare two models and return a typed [`ModelComparisonResult`] that cleanly carries
/// [`InferenceAvailability::NotApplicableInRobustMode`] instead of failing whenever either model
/// uses [`FitMethod::RobustCauchy`].
pub fn compare_models_checked(
    primary: &NonlinearFitResult,
    alt_model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
    spec: &ModelComparisonSpec,
) -> Result<ModelComparisonResult, FitError> {
    if primary.is_robust_cauchy() || config.method == FitMethod::RobustCauchy {
        let alt_config = filter_config_rules_for_alt_model(config, alt_model);
        let alternative_fit = fit_nonlinear(alt_model, datasets, hooks, &alt_config)?;
        return Ok(ModelComparisonResult {
            inference_availability: InferenceAvailability::NotApplicableInRobustMode,
            outcome: ModelComparisonOutcome::NotApplicableInRobustMode {
                requested_test: comparison_test_label(spec).to_owned(),
                reason: "NotApplicableInRobustMode: F, AICc, and LRT model comparisons are not applicable when either model uses Robust (Cauchy) regression".to_owned(),
            },
            alternative_fit,
        });
    }
    let (alternative_fit, outcome) = compare_models(primary, alt_model, datasets, hooks, config, spec)?;
    Ok(ModelComparisonResult { inference_availability: outcome.inference_availability(), outcome, alternative_fit })
}

/// Request payload for [`compare_models_synchronized`], supporting union-of-outliers (`O_1 U O_2`)
/// synchronization when comparing two models under ROUT elimination (Motulsky & Brown 2006).
pub struct SynchronizedComparisonRequest<'a> {
    /// Primary parametric model.
    pub primary_model: &'a dyn ParametricModel,
    /// Converged fit of `primary_model`.
    pub primary_fit: &'a NonlinearFitResult,
    /// Alternative competing parametric model.
    pub alt_model: &'a dyn ParametricModel,
    /// Input datasets passed to the primary fit.
    pub datasets: &'a [DatasetSeries],
    /// Hook resolution context.
    pub hooks: &'a HookContext<'a>,
    /// Solver configuration shared by both models.
    pub config: &'a SolverConfig,
    /// Model comparison criterion (`ExtraSumOfSquaresF`, `Aicc`, or `LikelihoodRatioTest`).
    pub spec: &'a ModelComparisonSpec,
}

fn remove_union_outliers(datasets: &[DatasetSeries], config: &SolverConfig, union: &BTreeSet<usize>) -> Result<Vec<DatasetSeries>, FitError> {
    let mut cleaned = datasets.to_vec();
    if config.replicate_fit_mode == ReplicateFitMode::RowMeansOnly {
        collapse_datasets_to_row_means(&mut cleaned);
    }
    if config.x_range_filter.is_some() || config.y_range_filter.is_some() {
        filter_datasets_by_range(&mut cleaned, config.x_range_filter, config.y_range_filter);
    }
    let mut global_idx = 0_usize;
    for ds in &mut cleaned {
        let mut kept = Vec::with_capacity(ds.points.len());
        for pt in ds.points.drain(..) {
            if !union.contains(&global_idx) {
                kept.push(pt);
            }
            global_idx += 1;
        }
        ds.points = kept;
    }
    cleaned.retain(|ds| !ds.points.is_empty());
    if cleaned.is_empty() {
        return Err(FitError::EmptyInput);
    }
    Ok(cleaned)
}

fn refit_config_without_rout(config: &SolverConfig) -> SolverConfig {
    let mut refit_cfg = config.clone();
    refit_cfg.method = FitMethod::LeastSquares;
    refit_cfg.outlier_handling = OutlierHandling::None;
    refit_cfg.replicate_fit_mode = ReplicateFitMode::IndividualReplicates;
    refit_cfg.x_range_filter = None;
    refit_cfg.y_range_filter = None;
    refit_cfg
}

/// Compare two models with automatic union-of-outliers (`O_1 U O_2`) synchronization under ROUT
/// elimination (Motulsky & Brown 2006): when `M_1` and `M_2` flag different outlier sets `O_1 != O_2`,
/// removes `O_1 U O_2` from the active observations and refits both `M_1` and `M_2` on the identical
/// retained set `I \ (O_1 U O_2)` before computing Extra-SS *F*, `AICc`, or LRT.
///
/// Returns `(effective_primary_fit, comparison_result)`.
pub fn compare_models_synchronized(req: &SynchronizedComparisonRequest<'_>) -> Result<(NonlinearFitResult, ModelComparisonResult), FitError> {
    let SynchronizedComparisonRequest { primary_model, primary_fit, alt_model, datasets, hooks, config, spec } = *req;
    if config.effective_outlier_handling() != OutlierHandling::Eliminate || primary_fit.is_robust_cauchy() || config.method == FitMethod::RobustCauchy
    {
        let comp = compare_models_checked(primary_fit, alt_model, datasets, hooks, config, spec)?;
        return Ok((primary_fit.clone(), comp));
    }

    let alt_config = filter_config_rules_for_alt_model(config, alt_model);
    let alt_initial = fit_nonlinear(alt_model, datasets, hooks, &alt_config)?;
    let set_a: BTreeSet<usize> = primary_fit.rout.as_ref().map(|r| r.outlier_indices.iter().copied().collect()).unwrap_or_default();
    let set_b: BTreeSet<usize> = alt_initial.rout.as_ref().map(|r| r.outlier_indices.iter().copied().collect()).unwrap_or_default();
    if set_a == set_b {
        let outcome = compare_fitted_pair(primary_fit, &alt_initial, config, spec_to_method(spec))?;
        let comp = ModelComparisonResult { inference_availability: outcome.inference_availability(), outcome, alternative_fit: alt_initial };
        return Ok((primary_fit.clone(), comp));
    }

    let union: BTreeSet<usize> = set_a.union(&set_b).copied().collect();
    let cleaned = remove_union_outliers(datasets, config, &union)?;
    let refit_cfg = refit_config_without_rout(config);

    let mut primary_refit = fit_nonlinear(primary_model, &cleaned, hooks, &refit_cfg)?;
    let mut comp = compare_models_checked(&primary_refit, alt_model, &cleaned, hooks, &refit_cfg, spec)?;
    primary_refit.n_outliers_excluded = union.len();
    comp.alternative_fit.n_outliers_excluded = union.len();
    Ok((primary_refit, comp))
}

/// Complete result for a 4-question model comparison (§2.5.2), pairing descriptive candidate labels,
/// both fitted models, and the statistical or well-defined-fit selection outcome.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelComparisonQuestionResult {
    /// Human-readable label for candidate A (null / simpler or primary model).
    pub model_a_label: String,
    /// Human-readable label for candidate B (alternative / fuller model).
    pub model_b_label: String,
    /// Fitted result for candidate A.
    pub model_a_fit: NonlinearFitResult,
    /// Fitted result for candidate B.
    pub model_b_fit: NonlinearFitResult,
    /// Parametric inference availability status.
    pub inference_availability: InferenceAvailability,
    /// Statistical test outcome, `PreferredWellDefinedFit`, or `NotApplicableInRobustMode`.
    pub outcome: ModelComparisonOutcome,
}

fn method_label(method: ModelComparisonMethod) -> &'static str {
    match method {
        ModelComparisonMethod::ExtraSumOfSquaresF => "Extra sum-of-squares F test",
        ModelComparisonMethod::Aicc => "AICc",
        ModelComparisonMethod::LikelihoodRatioTest => "Likelihood Ratio Test",
    }
}

fn is_constant_scope(scope: &ParameterScope) -> bool {
    matches!(
        scope,
        ParameterScope::Constant(_) | ParameterScope::HookedConstant(_) | ParameterScope::PerDatasetConstant(_) | ParameterScope::ColumnTitleConstant
    )
}

fn upsert_param_scope(rules: &mut Vec<ParameterRule>, canonical_name: &str, scope: ParameterScope, clear_bounds: bool) {
    if let Some(rule) = rules.iter_mut().find(|r| r.name.eq_ignore_ascii_case(canonical_name)) {
        rule.scope = scope;
        if clear_bounds {
            rule.lower_bound = None;
            rule.upper_bound = None;
            rule.initial_override = None;
        }
    } else {
        rules.push(ParameterRule { name: canonical_name.to_owned(), scope, initial_override: None, lower_bound: None, upper_bound: None });
    }
}

fn resolve_canonical_param_name<'a>(model: &'a dyn ParametricModel, requested: &str) -> Result<&'a str, FitError> {
    model.param_names().iter().find(|p| p.eq_ignore_ascii_case(requested)).map(String::as_str).ok_or_else(|| FitError::InvalidParameter {
        name: "comparison_question",
        reason: format!("Parameter `{requested}` is not defined on equation `{}`", model.id()),
    })
}

fn compare_fitted_pair(
    fit_a: &NonlinearFitResult,
    fit_b: &NonlinearFitResult,
    config: &SolverConfig,
    method: ModelComparisonMethod,
) -> Result<ModelComparisonOutcome, FitError> {
    if !matches!(fit_a.status, FitStatus::Converged | FitStatus::Ambiguous) {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!("Cannot compare models when candidate A did not converge ({:?})", fit_a.status),
        });
    }
    if !matches!(fit_b.status, FitStatus::Converged | FitStatus::Ambiguous) {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!("Cannot compare models when candidate B did not converge ({:?})", fit_b.status),
        });
    }
    if fit_a.fit_method != fit_b.fit_method {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!("Cannot compare models fitted under different methods ({:?} vs {:?})", fit_a.fit_method, fit_b.fit_method),
        });
    }
    if config.method == FitMethod::PoissonDeviance && method == ModelComparisonMethod::ExtraSumOfSquaresF {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: "Extra sum-of-squares F test is undefined for PoissonDeviance fits; use LikelihoodRatioTest or Aicc instead".to_owned(),
        });
    }
    if fit_a.n_obs != fit_b.n_obs || fit_a.n_outliers_excluded != fit_b.n_outliers_excluded {
        return Err(FitError::InvalidParameter {
            name: "comparison",
            reason: format!(
                "Cannot compare models fitted to different observation counts ({} vs {}) or excluded outlier counts ({} vs {})",
                fit_a.n_obs, fit_b.n_obs, fit_a.n_outliers_excluded, fit_b.n_outliers_excluded
            ),
        });
    }
    if (fit_a.n_outliers_excluded > 0 || fit_b.n_outliers_excluded > 0)
        && let (Some(rout_a), Some(rout_b)) = (&fit_a.rout, &fit_b.rout)
    {
        let mut idx_a = rout_a.outlier_indices.clone();
        let mut idx_b = rout_b.outlier_indices.clone();
        idx_a.sort_unstable();
        idx_b.sort_unstable();
        if idx_a != idx_b {
            return Err(FitError::InvalidParameter {
                name: "comparison",
                reason: String::from("Cannot compare ROUT models that excluded different outlier points"),
            });
        }
    }
    compare_validated_pair(fit_a, fit_b, config, method)
}

fn is_fixed_parameter_for_model(
    pname: &str,
    p_idx: usize,
    explicit_rules: &[ParameterRule],
    default_rules: &[ParameterRule],
    default_bounds: &[(Option<f64>, Option<f64>)],
) -> bool {
    let explicit = explicit_rules.iter().find(|r| r.name.eq_ignore_ascii_case(pname));
    let def_rule = default_rules.iter().find(|r| r.name.eq_ignore_ascii_case(pname));
    let effective_scope = explicit.map(|r| &r.scope).or_else(|| def_rule.map(|r| &r.scope));
    if effective_scope.is_some_and(is_constant_scope) {
        return true;
    }
    let def_b = default_bounds.get(p_idx).copied().unwrap_or((None, None));
    let lb = explicit.and_then(|r| r.lower_bound).or_else(|| def_rule.and_then(|r| r.lower_bound)).or(def_b.0);
    let ub = explicit.and_then(|r| r.upper_bound).or_else(|| def_rule.and_then(|r| r.upper_bound)).or(def_b.1);
    matches!((lb, ub), (Some(l), Some(u)) if l == u)
}

/// Evaluate any of the 4 scientific [`ModelComparisonQuestion`] hypotheses (`AlternativeEquation`,
/// `ParameterInvarianceAcrossDatasets`, `SingleGlobalCurveVsSeparate`, `ParameterEqualsTheoretical`)
/// with optional `prefer_well_defined_fit` pre-check (§2.5.2).
#[allow(clippy::too_many_arguments)]
pub fn compare_models_question(
    primary_model: &dyn ParametricModel,
    primary_fit: &NonlinearFitResult,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
    method: ModelComparisonMethod,
    question: &ModelComparisonQuestion,
    prefer_well_defined_fit: bool,
    flag_rules: Option<&FitFlagRules>,
) -> Result<ModelComparisonQuestionResult, FitError> {
    let mut alt_model_storage: Option<Box<dyn ParametricModel>> = None;
    let (model_a_label, mut model_a_fit, cfg_a, model_b_label, mut model_b_fit, cfg_b) = match question {
        ModelComparisonQuestion::AlternativeEquation { alternative_equation_id } => {
            let alt_model =
                get_builtin_model(alternative_equation_id).ok_or_else(|| FitError::UnknownEquation { id: alternative_equation_id.clone() })?;
            let alt_config = filter_config_rules_for_alt_model(config, alt_model.as_ref());
            let alt_fit = fit_nonlinear(alt_model.as_ref(), datasets, hooks, &alt_config)?;
            alt_model_storage = Some(alt_model);
            (primary_fit.equation_name.clone(), primary_fit.clone(), config.clone(), alt_fit.equation_name.clone(), alt_fit, alt_config)
        }
        ModelComparisonQuestion::ParameterInvarianceAcrossDatasets { shared_params } => {
            if datasets.len() < 2 {
                return Err(FitError::InvalidParameter {
                    name: "comparison_question",
                    reason: "ParameterInvarianceAcrossDatasets requires at least 2 datasets".to_owned(),
                });
            }
            if shared_params.is_empty() {
                return Err(FitError::InvalidParameter {
                    name: "comparison_question",
                    reason: "ParameterInvarianceAcrossDatasets requires at least one parameter name".to_owned(),
                });
            }
            let mut canonical_names = Vec::with_capacity(shared_params.len());
            for req in shared_params {
                canonical_names.push(resolve_canonical_param_name(primary_model, req)?);
            }
            let mut null_config = config.clone();
            let mut alt_config = config.clone();
            for &name in &canonical_names {
                upsert_param_scope(&mut null_config.parameter_rules, name, ParameterScope::SharedGlobal, false);
                upsert_param_scope(&mut alt_config.parameter_rules, name, ParameterScope::Local, false);
            }
            let fit_null = fit_nonlinear(primary_model, datasets, hooks, &null_config)?;
            let fit_alt = fit_nonlinear(primary_model, datasets, hooks, &alt_config)?;
            let joined = canonical_names.join(", ");
            (format!("Shared {joined} across datasets"), fit_null, null_config, format!("Separate {joined} per dataset"), fit_alt, alt_config)
        }
        ModelComparisonQuestion::SingleGlobalCurveVsSeparate => {
            if datasets.len() < 2 {
                return Err(FitError::InvalidParameter {
                    name: "comparison_question",
                    reason: "SingleGlobalCurveVsSeparate requires at least 2 datasets".to_owned(),
                });
            }
            let all_points: Vec<DataPoint> = datasets.iter().flat_map(|d| d.points.iter().cloned()).collect();
            let default_rules = primary_model.default_parameter_rules(&all_points);
            let default_bounds = primary_model.default_bounds();
            let mut null_config = config.clone();
            let mut alt_config = config.clone();
            for (p_idx, pname) in primary_model.param_names().iter().enumerate() {
                if !is_fixed_parameter_for_model(pname.as_str(), p_idx, &config.parameter_rules, &default_rules, &default_bounds) {
                    upsert_param_scope(&mut null_config.parameter_rules, pname.as_str(), ParameterScope::SharedGlobal, false);
                    upsert_param_scope(&mut alt_config.parameter_rules, pname.as_str(), ParameterScope::Local, false);
                }
            }
            let fit_null = fit_nonlinear(primary_model, datasets, hooks, &null_config)?;
            let fit_alt = fit_nonlinear(primary_model, datasets, hooks, &alt_config)?;
            (
                format!("One global curve for all datasets ({})", primary_model.name()),
                fit_null,
                null_config,
                format!("Separate curve for each dataset ({})", primary_model.name()),
                fit_alt,
                alt_config,
            )
        }
        ModelComparisonQuestion::ParameterEqualsTheoretical { param, theoretical_value } => {
            if !theoretical_value.is_finite() {
                return Err(FitError::InvalidParameter {
                    name: "theoretical_value",
                    reason: format!("Theoretical parameter value must be finite, got {theoretical_value}"),
                });
            }
            let canonical = resolve_canonical_param_name(primary_model, param)?;
            let mut null_config = config.clone();
            upsert_param_scope(&mut null_config.parameter_rules, canonical, ParameterScope::Constant(*theoretical_value), true);
            let mut alt_config = config.clone();
            if let Some(existing) = alt_config.parameter_rules.iter_mut().find(|r| r.name.eq_ignore_ascii_case(canonical))
                && is_constant_scope(&existing.scope)
            {
                existing.scope = ParameterScope::Local;
            }
            let fit_null = fit_nonlinear(primary_model, datasets, hooks, &null_config)?;
            let fit_alt = fit_nonlinear(primary_model, datasets, hooks, &alt_config)?;
            (
                format!("{canonical} = {theoretical_value} (constrained)"),
                fit_null,
                null_config,
                format!("{canonical} unconstrained"),
                fit_alt,
                alt_config,
            )
        }
    };

    if model_a_fit.is_robust_cauchy() || model_b_fit.is_robust_cauchy() || config.method == FitMethod::RobustCauchy {
        let outcome = ModelComparisonOutcome::NotApplicableInRobustMode {
            requested_test: method_label(method).to_owned(),
            reason:
                "NotApplicableInRobustMode: F, AICc, and LRT model comparisons are not applicable when either model uses Robust (Cauchy) regression"
                    .to_owned(),
        };
        return Ok(ModelComparisonQuestionResult {
            model_a_label,
            model_b_label,
            model_a_fit,
            model_b_fit,
            inference_availability: InferenceAvailability::NotApplicableInRobustMode,
            outcome,
        });
    }

    // Union-of-outliers (`O_1 U O_2`) synchronization under ROUT elimination (Motulsky & Brown 2006; AUD-03-24).
    let mut effective_compare_cfg = config.clone();
    if config.effective_outlier_handling() == OutlierHandling::Eliminate {
        let set_a: BTreeSet<usize> = model_a_fit.rout.as_ref().map(|r| r.outlier_indices.iter().copied().collect()).unwrap_or_default();
        let set_b: BTreeSet<usize> = model_b_fit.rout.as_ref().map(|r| r.outlier_indices.iter().copied().collect()).unwrap_or_default();
        if set_a != set_b {
            let union: BTreeSet<usize> = set_a.union(&set_b).copied().collect();
            let cleaned = remove_union_outliers(datasets, config, &union)?;
            let refit_a_cfg = refit_config_without_rout(&cfg_a);
            let refit_b_cfg = refit_config_without_rout(&cfg_b);
            let model_b_ref: &dyn ParametricModel = alt_model_storage.as_deref().unwrap_or(primary_model);
            model_a_fit = fit_nonlinear(primary_model, &cleaned, hooks, &refit_a_cfg)?;
            model_b_fit = fit_nonlinear(model_b_ref, &cleaned, hooks, &refit_b_cfg)?;
            model_a_fit.n_outliers_excluded = union.len();
            model_b_fit.n_outliers_excluded = union.len();
            effective_compare_cfg = refit_config_without_rout(config);
        }
    }

    if prefer_well_defined_fit {
        let (a_clean, a_issues) = is_fit_well_defined(&model_a_fit, flag_rules);
        let (b_clean, b_issues) = is_fit_well_defined(&model_b_fit, flag_rules);
        if a_clean && !b_clean {
            let outcome = ModelComparisonOutcome::PreferredWellDefinedFit {
                preferred_label: model_a_label.clone(),
                rejected_label: model_b_label.clone(),
                preferred_is_primary: true,
                rejection_reasons: b_issues,
            };
            return Ok(ModelComparisonQuestionResult {
                model_a_label,
                model_b_label,
                model_a_fit,
                model_b_fit,
                inference_availability: InferenceAvailability::Available,
                outcome,
            });
        }
        if b_clean && !a_clean {
            let outcome = ModelComparisonOutcome::PreferredWellDefinedFit {
                preferred_label: model_b_label.clone(),
                rejected_label: model_a_label.clone(),
                preferred_is_primary: false,
                rejection_reasons: a_issues,
            };
            return Ok(ModelComparisonQuestionResult {
                model_a_label,
                model_b_label,
                model_a_fit,
                model_b_fit,
                inference_availability: InferenceAvailability::Available,
                outcome,
            });
        }
    }

    let outcome = compare_fitted_pair(&model_a_fit, &model_b_fit, &effective_compare_cfg, method)?;
    Ok(ModelComparisonQuestionResult {
        model_a_label,
        model_b_label,
        model_a_fit,
        model_b_fit,
        inference_availability: outcome.inference_availability(),
        outcome,
    })
}
