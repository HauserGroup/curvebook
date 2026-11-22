//! Nested-model comparison by extra-sum-of-squares *F* test, AICc, and likelihood ratio.
//!
//! References:
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.

use curvebook_doc::{FitMethod, ModelComparisonSpec};

use crate::diagnostics::{compare_models_aicc, compare_models_f_test, compare_models_lrt, compare_models_lrt_deviance};
use crate::error::{FitError, FitStatus};
use crate::library::ParametricModel;
use crate::solver::fit::fit_nonlinear;
use crate::solver::inference::compare_poisson_aicc;
use crate::solver::results::{InferenceAvailability, ModelComparisonOutcome, ModelComparisonResult, NonlinearFitResult, SolverConfig};
use crate::types::{DatasetSeries, HookContext};

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
    if let (Some(rout_a), Some(rout_b)) = (&primary.rout, &alt.rout) {
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
    let outcome = match spec {
        ModelComparisonSpec::ExtraSumOfSquaresF { .. } => {
            let (null_fit, complex_fit, _) = ordered_nested_roles(primary, &alt);
            ensure_nested_ss_order(null_fit, complex_fit)?;
            ModelComparisonOutcome::ExtraSumOfSquaresF(compare_models_f_test(
                null_fit.weighted_ss,
                null_fit.df,
                complex_fit.weighted_ss,
                complex_fit.df,
                config.confidence.alpha(),
            )?)
        }
        ModelComparisonSpec::Aicc { .. } => {
            if config.method == FitMethod::PoissonDeviance {
                let aicc_a = primary.aic_c.ok_or_else(|| FitError::InsufficientData { required: primary.k_free + 2, actual: primary.n_obs })?;
                let aicc_b = alt.aic_c.ok_or_else(|| FitError::InsufficientData { required: alt.k_free + 2, actual: alt.n_obs })?;
                ModelComparisonOutcome::Aicc(compare_poisson_aicc(aicc_a, aicc_b)?)
            } else {
                ModelComparisonOutcome::Aicc(compare_models_aicc(primary.weighted_ss, primary.k_free, alt.weighted_ss, alt.k_free, primary.n_obs)?)
            }
        }
        ModelComparisonSpec::LikelihoodRatioTest { .. } => {
            let (null_fit, complex_fit, _) = ordered_nested_roles(primary, &alt);
            ensure_nested_ss_order(null_fit, complex_fit)?;
            let res = if config.method == FitMethod::PoissonDeviance {
                compare_models_lrt_deviance(null_fit.weighted_ss, null_fit.df, complex_fit.weighted_ss, complex_fit.df, config.confidence.alpha())?
            } else {
                compare_models_lrt(
                    null_fit.weighted_ss,
                    null_fit.df,
                    complex_fit.weighted_ss,
                    complex_fit.df,
                    primary.n_obs,
                    config.confidence.alpha(),
                )?
            };
            ModelComparisonOutcome::LikelihoodRatioTest(res)
        }
    };
    Ok((alt, outcome))
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
