//! Trust-region Levenberg–Marquardt nonlinear regression engine, global sharing, constraints,
//! Nelder–Mead simplex warm-start, weighting, Poisson deviance IRLS, Cauchy robust IRLS,
//! ROUT outlier elimination, curve bands, and Brent inverse unknown interpolation.
//!
//! # Methods and references
//!
//! - Damped least squares: Levenberg (1944) and Marquardt (1963); the trust-region form with
//!   adaptive column scaling, step-bound update, and `ftol`/`xtol`/`gtol` tests follows
//!   Moré (1978) and the MINPACK-1 implementation (Moré, Garbow & Hillstrom 1980). See
//!   `run_trust_region_lm` for the documented deviations.
//! - Simplex rescue: Nelder & Mead (1965) with the outside/inside contraction rules of
//!   Lagarias et al. (1998).
//! - Finite-difference Jacobian step sizes (`~eps^(1/3)` central, `~eps^(1/2)` forward):
//!   Nocedal & Wright (2006), Ch. 8.
//! - Weighting, asymptotic standard errors, Wald and profile intervals, confidence/prediction
//!   bands, and model comparison: Bates & Watts (1988), Seber & Wild (1989), and
//!   Motulsky & Christopoulos (2004).
//! - Poisson regression by iteratively reweighted least squares: Green (1984);
//!   McCullagh & Nelder (1989).
//! - Robust (Cauchy / Lorentzian) fitting and ROUT outlier removal: Motulsky & Brown (2006),
//!   solved here by IRLS (Holland & Welsch 1977).
//! - Root finding for interpolation: Brent (1973), Ch. 4.
//!
//! Reference: Levenberg, K. (1944). A method for the solution of certain non-linear problems in least squares. *Quarterly of Applied Mathematics* 2(2), 164–168. doi:10.1090/qam/10666
//!
//! Reference: Marquardt, D. W. (1963). An algorithm for least-squares estimation of nonlinear parameters. *Journal of the Society for Industrial and Applied Mathematics* 11(2), 431–441. doi:10.1137/0111030
//!
//! Reference: Moré, J. J. (1978). The Levenberg–Marquardt algorithm: implementation and theory. In G. A. Watson (ed.), *Numerical Analysis*, Lecture Notes in Mathematics 630, 105–116. Springer. doi:10.1007/BFb0067700
//!
//! Reference: Moré, J. J., Garbow, B. S. & Hillstrom, K. E. (1980). *User Guide for MINPACK-1*. Report ANL-80-74, Argonne National Laboratory.
//!
//! Reference: Nelder, J. A. & Mead, R. (1965). A simplex method for function minimization. *The Computer Journal* 7(4), 308–313. doi:10.1093/comjnl/7.4.308
//!
//! Reference: Lagarias, J. C., Reeds, J. A., Wright, M. H. & Wright, P. E. (1998). Convergence properties of the Nelder–Mead simplex method in low dimensions. *SIAM Journal on Optimization* 9(1), 112–147. doi:10.1137/S1052623496303470
//!
//! Reference: Nocedal, J. & Wright, S. J. (2006). *Numerical Optimization*, 2nd ed. Springer.
//!
//! Reference: Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//!
//! Reference: Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//!
//! Reference: Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//!
//! Reference: Green, P. J. (1984). Iteratively reweighted least squares for maximum likelihood estimation, and some robust and resistant alternatives. *Journal of the Royal Statistical Society, Series B* 46(2), 149–192.
//!
//! Reference: McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
//!
//! Reference: Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear regression – a new method based on robust nonlinear regression and the false discovery rate. *BMC Bioinformatics* 7, 123. doi:10.1186/1471-2105-7-123
//!
//! Reference: Holland, P. W. & Welsch, R. E. (1977). Robust regression using iteratively reweighted least-squares. *Communications in Statistics – Theory and Methods* 6(9), 813–827. doi:10.1080/03610927708827533
//!
//! Reference: Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.

mod identifiability;

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use curvebook_doc::{
    CiBound, CurveSamplePoint, FitMethod, FittedCurveOverlay, ModelComparisonSpec, ParameterRule, ParameterScope, WeightingScheme,
    parse_column_title_numeric_hook,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{f_sf, norm_two_sided_isf, student_t_two_sided_isf, student_t_two_sided_p};
use curvebook_math::linalg::{DenseMatrix, qr, qr_col_pivoted};
use curvebook_math::roots::{RootOptions, brent_root};
use curvebook_math::special::{deviance_bd0, ln_gamma};
use curvebook_math::sum::NeumaierSum;

use crate::diagnostics::{
    AiccComparisonResult, CovarianceDiagnostics, ExtraSumOfSquaresResult, HomoscedasticityResult, InformationCriteria, LackOfFitResult,
    LikelihoodRatioResult, NormalityTestResult, ProfileStep, RoutDetectionResult, RoutPointAudit, RunsTestResult, anderson_darling_test,
    combine_runs_tests, compare_models_aicc, compare_models_f_test, compare_models_lrt, compare_models_lrt_deviance, compute_covariance_diagnostics,
    compute_deviance_profile_likelihood_cis_checked, compute_hougaard_skewness_into, compute_information_criteria,
    compute_poisson_covariance_diagnostics, compute_profile_likelihood_cis_checked, compute_rsdr, dagostino_pearson_test,
    homoscedasticity_spearman_test, lilliefors_ks_test, runs_test, shapiro_wilk_test,
};
use crate::dsl::{DEFAULT_MAX_ODE_STEPS_PER_FIT, enter_ode_fit_budget, ode_fit_budget_exceeded};
use crate::error::{FitError, FitIdentifiability, FitStatus, ParameterNote};
use crate::library::ParametricModel;
use crate::types::{DataPoint, DatasetSeries, DerivedScalar, HookContext, UnknownResponse, filter_datasets_by_range, validated_replicate_moments};
use identifiability::Assessment;

/// Configuration for executing nonlinear regression via [`fit_nonlinear`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SolverConfig {
    /// Fitting objective method (`LeastSquares`, `PoissonDeviance`, `RobustCauchy`, or `RoutOutlierElimination`).
    pub method: FitMethod,
    /// Variance weighting scheme (`None`, `OneOverY`, `OneOverY2`, `OneOverSd2`, `OneOverX`, `OneOverX2`, `OneOverSema`, `UserSupplied`).
    pub weighting: WeightingScheme,
    /// Per-parameter sharing scopes, constants, hooks, initial overrides, and inclusive box bounds.
    pub parameter_rules: Vec<ParameterRule>,
    /// Two-sided confidence level for Wald, profile-likelihood, and band intervals (default `0.95` / 95%).
    pub confidence: ConfidenceLevel,
    /// Maximum Levenberg–Marquardt trust-region / IRLS outer iterations before reporting `FitStatus::HitMaxIters`.
    pub max_iters: usize,
    /// Compute profile-likelihood confidence intervals (Venzon & Moolgavkar 1988) instead of only symmetric Wald intervals.
    pub compute_profile_ci: bool,
    /// Assess free-parameter identifiability (`FitIdentifiability` and `ParameterNote::Unstable`).
    pub identify_unstable_parameters: bool,
    /// Compute Hougaard skewness `|g1|` per free parameter (`Ratkowsky 1990`) via directional finite differences.
    pub compute_hougaard: bool,
    /// Compute smooth mean-response confidence bands along `FittedCurveOverlay`.
    pub compute_bands: bool,
    /// Include future single-observation variance inside curve overlay bands (`95%` prediction bands in addition to confidence bands).
    pub compute_prediction_bands: bool,
    /// Run Wald–Wolfowitz runs test on sorted residuals.
    pub compute_runs_test: bool,
    /// Run pure-error vs lack-of-fit *F*-test when replicate observations share identical predictor coordinates.
    pub compute_lack_of_fit: bool,
    /// Run residual normality tests (D'Agostino–Pearson $K^2$, Anderson–Darling, Lilliefors KS).
    pub compute_normality: bool,
    /// Run Spearman rank homoscedasticity check on `|residual|` vs predicted `Y`.
    pub compute_homoscedasticity: bool,
    /// Interpolate unknown `Y` and/or `X` points found in input series (`unknowns_y` / `unknown_x`) with Fieller/delta CIs and calibration PIs.
    pub interpolate_unknowns: bool,
    /// Run a Nelder–Mead simplex warm-start rescue when heuristic initial parameter values produce non-finite residuals or poor initial fit.
    pub use_simplex_warm_start: bool,
    /// Optional inclusive `(min_x, max_x)` domain filter applied before fitting (`None` endpoints are unbounded).
    pub x_range_filter: Option<(Option<f64>, Option<f64>)>,
    /// Optional inclusive `(min_y, max_y)` response filter applied before fitting (`None` endpoints are unbounded).
    pub y_range_filter: Option<(Option<f64>, Option<f64>)>,
    /// Evaluate model curve overlay directly from fixed/initial parameters without optimizing free parameters (`0` iterations).
    pub plot_function_only: bool,
    /// Optional lower `X` endpoint override for emitted `FittedCurveOverlay`; `None` uses observed data minimum.
    pub plot_x_min: Option<f64>,
    /// Optional upper `X` endpoint override for emitted `FittedCurveOverlay`; `None` uses observed data maximum.
    pub plot_x_max: Option<f64>,
    /// Number of grid segments used to sample smooth `FittedCurveOverlay` curves.
    pub curve_overlay_segments: usize,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            method: FitMethod::LeastSquares,
            weighting: WeightingScheme::None,
            parameter_rules: Vec::new(),
            confidence: ConfidenceLevel::DEFAULT_95,
            max_iters: 1_000,
            compute_profile_ci: true,
            identify_unstable_parameters: false,
            compute_hougaard: true,
            compute_bands: true,
            compute_prediction_bands: false,
            compute_runs_test: true,
            compute_lack_of_fit: false,
            compute_normality: false,
            compute_homoscedasticity: false,
            interpolate_unknowns: false,
            use_simplex_warm_start: true,
            x_range_filter: None,
            y_range_filter: None,
            plot_function_only: false,
            plot_x_min: None,
            plot_x_max: None,
            curve_overlay_segments: 256,
        }
    }
}

fn default_fit_method() -> FitMethod {
    FitMethod::LeastSquares
}

fn deserialize_f64_or_nan<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(<Option<f64> as serde::Deserialize>::deserialize(deserializer)?.unwrap_or(f64::NAN))
}

fn deserialize_opt_f64_vec<'de, D>(deserializer: D) -> Result<Vec<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <Vec<Option<f64>> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect())
}

fn deserialize_opt_f64_matrix<'de, D>(deserializer: D) -> Result<Vec<Vec<f64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <Vec<Vec<Option<f64>>> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|row| row.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect()).collect())
}

#[derive(serde::Deserialize)]
struct CovarianceDiagnosticsCompat {
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    standard_errors: Vec<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    symmetric_ci_lower: Vec<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    symmetric_ci_upper: Vec<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_matrix")]
    covariance_matrix: Vec<Vec<f64>>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_matrix")]
    correlation_matrix: Vec<Vec<f64>>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    dependencies: Vec<f64>,
    #[serde(default)]
    numerical_rank: usize,
    #[serde(default)]
    ambiguous: bool,
}

fn deserialize_covariance_diagnostics<'de, D>(deserializer: D) -> Result<CovarianceDiagnostics, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let compat = <CovarianceDiagnosticsCompat as serde::Deserialize>::deserialize(deserializer)?;
    Ok(CovarianceDiagnostics {
        standard_errors: compat.standard_errors,
        symmetric_ci_lower: compat.symmetric_ci_lower,
        symmetric_ci_upper: compat.symmetric_ci_upper,
        covariance_matrix: compat.covariance_matrix,
        correlation_matrix: compat.correlation_matrix,
        dependencies: compat.dependencies,
        numerical_rank: compat.numerical_rank,
        ambiguous: compat.ambiguous,
    })
}

/// Availability of parametric least-squares / likelihood statistical inference for a fit or comparison.
///
/// Under [`FitMethod::RobustCauchy`], Cauchy IRLS regression minimizes a robust Lorentzian merit
/// rather than a Gaussian least-squares likelihood. Parameter point estimates, residuals, `RSDR`,
/// and interpolated point predictions remain valid, whereas least-squares inference (parameter SEs
/// and CIs, covariance/correlation matrices, confidence and prediction bands, `AICc`/`BIC`, and
/// `Extra-SS F` / `AICc` / `LRT` model comparisons) is [`Self::NotApplicableInRobustMode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum InferenceAvailability {
    /// Parametric least-squares / likelihood inference is valid and populated.
    #[default]
    Available,
    /// Least-squares inference is suppressed because regression used [`FitMethod::RobustCauchy`].
    NotApplicableInRobustMode,
}

impl InferenceAvailability {
    /// Return `true` when parametric least-squares / likelihood inference is available.
    #[must_use]
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }

    /// Return `true` when inference was suppressed for pure Robust (Cauchy) regression.
    #[must_use]
    pub const fn is_not_applicable_in_robust_mode(self) -> bool {
        matches!(self, Self::NotApplicableInRobustMode)
    }

    /// Canonical machine-readable status string for diagnostics, JSON payloads, and reports.
    #[must_use]
    pub const fn status_code(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::NotApplicableInRobustMode => "NotApplicableInRobustMode",
        }
    }
}

/// Fitted value for a single free parameter slot (local or globally shared).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FittedParameterSummary {
    /// Zero-based index into the flattened free parameter vector `theta_free` and covariance matrix.
    pub slot_index: usize,
    /// Parameter name matched against `ParametricModel::param_names`.
    pub name: String,
    /// Originating zero-based dataset index for local parameters, or `None` when globally shared across datasets.
    pub dataset_index: Option<usize>,
    /// Point estimate at convergence (subject to configured bounds).
    pub estimate: f64,
    /// Linearized asymptotic standard error (`NaN` in `FitMethod::RobustCauchy` mode or rank-deficient fits).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub se: f64,
    /// Lower bound of the two-sided profile-likelihood confidence interval (falls back to symmetric Wald or [`CiBound::Open`] if unbounded / `RobustCauchy`).
    pub ci_lower: CiBound,
    /// Upper bound of the two-sided profile-likelihood confidence interval ([`CiBound::Open`] if unbounded / `RobustCauchy`).
    pub ci_upper: CiBound,
    /// Lower bound of the symmetric asymptotic `t(df)` Wald confidence interval at configured `ConfidenceLevel` (`NaN` when SE unavailable).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub symmetric_ci_lower: f64,
    /// Upper bound of the symmetric asymptotic `t(df)` Wald confidence interval at configured `ConfidenceLevel` (`NaN` when SE unavailable).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub symmetric_ci_upper: f64,
    /// Parameter multicollinearity dependency index $1 - 1 / ((J^\top J)^{-1}_{jj} (J^\top J)_{jj}) \in [0, 1]$ (`NaN` when unidentifiable / `RobustCauchy`).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub dependency: f64,
    /// Absolute Hougaard skewness measure $|g_1|$ assessing nonlinearity bias (`NaN` if uncomputed or `RobustCauchy`; `|g1| < 0.1` close-to-linear per Ratkowsky 1990).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub hougaard_g1: f64,
    /// Diagnostic flags attached to this parameter (e.g. `HitLowerBound`, `Ambiguous`, `UnboundedCi`).
    pub notes: Vec<ParameterNote>,
    /// Indicates whether parametric asymptotic SEs/CIs are populated or suppressed in `RobustCauchy` mode.
    #[serde(default)]
    pub inference_availability: InferenceAvailability,
}

impl FittedParameterSummary {
    /// Typed parameter standard error (`None` in pure Robust Cauchy mode or when non-finite).
    #[must_use]
    pub fn se_option(&self) -> Option<f64> {
        (self.inference_availability.is_available() && self.se.is_finite()).then_some(self.se)
    }

    /// Typed profile/Wald confidence interval bounds (`None` in pure Robust Cauchy mode).
    #[must_use]
    pub fn ci_bounds_option(&self) -> Option<(CiBound, CiBound)> {
        self.inference_availability.is_available().then_some((self.ci_lower, self.ci_upper))
    }

    /// Typed symmetric Wald confidence interval (`None` in pure Robust Cauchy mode or when non-finite).
    #[must_use]
    pub fn symmetric_ci_option(&self) -> Option<(f64, f64)> {
        (self.inference_availability.is_available() && self.symmetric_ci_lower.is_finite() && self.symmetric_ci_upper.is_finite())
            .then_some((self.symmetric_ci_lower, self.symmetric_ci_upper))
    }

    /// Typed multicollinearity dependency index (`None` in pure Robust Cauchy mode or when non-finite).
    #[must_use]
    pub fn dependency_option(&self) -> Option<f64> {
        (self.inference_availability.is_available() && self.dependency.is_finite()).then_some(self.dependency)
    }

    /// Typed Hougaard skewness `|g1|` (`None` in pure Robust Cauchy mode or when non-finite).
    #[must_use]
    pub fn hougaard_g1_option(&self) -> Option<f64> {
        (self.inference_availability.is_available() && self.hougaard_g1.is_finite()).then_some(self.hougaard_g1)
    }
}

/// Default [`CiBound::Open`] helper for backwards-compatible deserialization of pre-R4.6 documents.
const fn default_open_ci_bound() -> CiBound {
    CiBound::Open
}

/// Interpolated unknown (`X` from `Y` or `Y` from `X`) with mean-response and calibration/prediction intervals.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InterpolatedUnknown {
    /// Zero-based dataset index where the unknown row originated.
    pub data_set_index: usize,
    /// Zero-based table row index of the unknown observation.
    pub row: usize,
    /// Source zero-based replicate subcolumn for an individual raw replicate; `None` for single/summarized or row-mean targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replicate_subcolumn: Option<usize>,
    /// Count of active responses averaged for inverse calibration of a replicate row; `None` for individual replicate or non-replicate rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replicate_mean_count: Option<NonZeroUsize>,
    /// Known coordinate (`Y` for inverse `X`-from-`Y` interpolation, or `X` for forward `Y`-from-`X` prediction).
    pub input_value: f64,
    /// Solved/evaluated point estimate (`NaN` when inversion fails or falls outside monotone bracket).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub output_estimate: f64,
    /// Lower bound of the two-sided delta-method mean-response confidence interval (`CiBound::Open` when unestimable / `RobustCauchy`).
    pub ci_lower: CiBound,
    /// Upper bound of the two-sided delta-method mean-response confidence interval (`CiBound::Open` when unestimable / `RobustCauchy`).
    pub ci_upper: CiBound,
    /// Lower bound of the single-observation calibration / prediction interval (new-observation variance included).
    #[serde(default = "default_open_ci_bound")]
    pub pred_lower: CiBound,
    /// Upper bound of the single-observation calibration / prediction interval (new-observation variance included).
    #[serde(default = "default_open_ci_bound")]
    pub pred_upper: CiBound,
    /// Optional human-readable warning when extrapolation, flat slope, or non-finite SE affected interpolation; `None` on nominal interior interpolation.
    pub note: Option<String>,
    /// Indicates whether delta-method confidence/prediction intervals are populated or suppressed under `RobustCauchy`.
    #[serde(default)]
    pub inference_availability: InferenceAvailability,
}

impl InterpolatedUnknown {
    /// Typed mean-response confidence interval for the interpolated value (`None` in pure Robust Cauchy mode).
    #[must_use]
    pub fn ci_bounds_option(&self) -> Option<(CiBound, CiBound)> {
        self.inference_availability.is_available().then_some((self.ci_lower, self.ci_upper))
    }

    /// Typed calibration / prediction interval for a single future observation (`None` in pure Robust Cauchy mode).
    #[must_use]
    pub fn pred_bounds_option(&self) -> Option<(CiBound, CiBound)> {
        self.inference_availability.is_available().then_some((self.pred_lower, self.pred_upper))
    }
}

/// Per-dataset fitted curve summary.
///
/// Note (`RV-96`): per Motulsky & Christopoulos (2004), `ss_residual`, `r_squared`, and `sy_x`
/// are reported in unweighted original $Y$ units even when differential weighting is active,
/// while global covariance, parameter SEs, AICc/BIC, lack-of-fit, and model comparisons use
/// `weighted_ss` (or Poisson deviance).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FittedDatasetResult {
    /// Zero-based dataset column index (`0 = A`, `1 = B`, ...).
    pub data_set_index: usize,
    /// Dataset column label from the upstream table.
    pub label: String,
    /// Full parameter vector `[theta_1, ..., theta_p]` ordered by `param_names` (including fixed and shared slots resolved for this dataset).
    pub params: Vec<f64>,
    /// Ordered equation parameter names corresponding 1-to-1 with `params`.
    pub param_names: Vec<String>,
    /// Inclusive X extent of the observations used in a scalar-predictor fit.
    /// Absent for multivariate datasets, empty datasets, and plot-only evaluations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_x_range: Option<(f64, f64)>,
    /// Fitted model predictions $\hat{y}_i$ at each included point in this dataset.
    pub y_hat: Vec<f64>,
    /// Unweighted residuals $y_i - \hat{y}_i$ in original $Y$ units.
    pub residuals: Vec<f64>,
    /// Objective-weighted residuals $\sqrt{w_i}(y_i - \hat{y}_i)$ (or signed deviance residuals for `PoissonDeviance`).
    pub weighted_residuals: Vec<f64>,
    /// Internally studentized residuals $r_i / (\hat{\sigma}_w \sqrt{1 - h_{ii}})$ (`NaN` where leverage `h_ii >= 1` or under `RobustCauchy`).
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    pub studentized_residuals: Vec<f64>,
    /// Unweighted residual sum of squares $\sum (y_i - \hat{y}_i)^2$ for this dataset in original $Y$ units.
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub ss_residual: f64,
    /// Unweighted coefficient of determination $1 - \text{SS}_{\text{res}, d} / \text{SS}_{\text{tot}, d}$ in original $Y$ units
    /// even when weighted least squares or Poisson deviance is active (Motulsky & Christopoulos 2004).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub r_squared: f64,
    /// Unweighted per-dataset residual SD $\sqrt{\text{SS}_{\text{res}, d} / (n_d - k_{\text{eff}, d})}$ in original $Y$ units,
    /// using the dataset's effective parameter count $k_{\text{eff}, d}$ (`NaN` in Robust Cauchy mode).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub sy_x: f64,
    /// Robust Standard Deviation of Residuals (`RSDR`) when `RobustCauchy` or `RoutOutlierElimination` is active (`None` otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsdr: Option<f64>,
    /// Within-dataset Wald–Wolfowitz runs test computed in this dataset's own X order (`None` when disabled or plot-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_test: Option<RunsTestResult>,
    /// Sampled fitted curve polyline and optional two-sided confidence/prediction bands (`None` when `compute_bands` is `false` and overlay omitted).
    pub curve_overlay: Option<FittedCurveOverlay>,
}

/// Model comparison output (`Extra-SS F`, `AICc`, Likelihood Ratio Test, or `NotApplicableInRobustMode`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ModelComparisonOutcome {
    /// Nested least-squares Extra-Sum-of-Squares *F*-test outcome.
    ExtraSumOfSquaresF(ExtraSumOfSquaresResult),
    /// Information-theoretic $\text{AIC}_c$ weight and evidence-ratio comparison.
    Aicc(AiccComparisonResult),
    /// Nested likelihood-ratio $\chi^2$ test (typically used for `PoissonDeviance` fits).
    LikelihoodRatioTest(LikelihoodRatioResult),
    /// Least-squares / likelihood model comparison is not applicable when either model uses Robust (Cauchy) regression.
    NotApplicableInRobustMode {
        /// Identifier or label of the comparison test requested by the caller.
        requested_test: String,
        /// Explanation of why statistical comparison is undefined in Robust (Cauchy) regression.
        reason: String,
    },
}

impl ModelComparisonOutcome {
    /// Return `true` if a statistical model comparison test was computed.
    #[must_use]
    pub const fn is_applicable(&self) -> bool {
        !matches!(self, Self::NotApplicableInRobustMode { .. })
    }

    /// Typed inference availability status for this comparison outcome.
    #[must_use]
    pub const fn inference_availability(&self) -> InferenceAvailability {
        match self {
            Self::NotApplicableInRobustMode { .. } => InferenceAvailability::NotApplicableInRobustMode,
            Self::ExtraSumOfSquaresF(_) | Self::Aicc(_) | Self::LikelihoodRatioTest(_) => InferenceAvailability::Available,
        }
    }
}

/// Complete model comparison response pairing availability status, alternative fit, and comparison outcome.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelComparisonResult {
    /// Indicates whether parametric model comparison inference is valid or suppressed in `RobustCauchy` mode.
    #[serde(default)]
    pub inference_availability: InferenceAvailability,
    /// Statistical model comparison test result (`Extra-SS F`, `AICc`, `LRT`, or `NotApplicableInRobustMode`).
    pub outcome: ModelComparisonOutcome,
    /// Complete fitted result for the alternative (second) candidate model compared against the baseline fit.
    pub alternative_fit: NonlinearFitResult,
}

impl ModelComparisonResult {
    /// Return `true` when statistical model comparison inference was computed.
    #[must_use]
    pub const fn is_applicable(&self) -> bool {
        self.inference_availability.is_available() && self.outcome.is_applicable()
    }
}

/// Complete nonlinear regression result across one or multiple globally fitted datasets.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NonlinearFitResult {
    /// Canonical equation identifier used for the fit (or custom DSL equation id).
    pub equation_id: String,
    /// Human-readable display name of the equation.
    pub equation_name: String,
    /// Objective/robustness method executed (`LeastSquares`, `PoissonDeviance`, `RobustCauchy`, or `RoutOutlierElimination`).
    #[serde(default = "default_fit_method")]
    pub fit_method: FitMethod,
    /// Parametric inference availability flag (`NotApplicableInRobustMode` disables SE/CI/covariance/AICc reporting).
    #[serde(default)]
    pub inference_availability: InferenceAvailability,
    /// Convergence status of the nonlinear optimizer (`Converged`, `Ambiguous`, `HitMaxIters`, etc.).
    pub status: FitStatus,
    /// Optional parameter identifiability assessment across fitted datasets.
    #[serde(default)]
    pub identifiability: FitIdentifiability,
    /// Total number of optimization outer steps executed by Levenberg–Marquardt / IRLS.
    pub iterations: usize,
    /// Number of active non-excluded non-outlier observations included in the final fit.
    pub n_obs: usize,
    /// Count of points flagged and excluded by ROUT outlier elimination (`0` when ROUT is inactive).
    pub n_outliers_excluded: usize,
    /// Total count of free parameters optimized across all datasets (excludes fixed constants, hooked constants, and equal bounds).
    pub k_free: usize,
    /// Residual degrees of freedom `n_obs.saturating_sub(k_free)`.
    pub df: usize,
    /// Unweighted residual sum of squares $\sum (y_i - \hat{y}_i)^2$ in original $Y$ units.
    pub rss: f64,
    /// Objective value minimised by the fit: weighted residual sum of squares $\sum w_i (y_i - \hat{y}_i)^2$
    /// for least-squares fits (equal to `rss` when unweighted) or Poisson deviance for `PoissonDeviance`.
    pub weighted_ss: f64,
    /// Unweighted residual standard deviation (`Sy.x = sqrt(rss / df)`) in original $Y$ units (`NaN` when `df == 0` or `RobustCauchy`);
    /// weighted least-squares parameter covariance, studentized residuals, and bands use
    /// $\sigma_w = \sqrt{\text{weighted\_ss} / \text{df}}$ per Motulsky & Christopoulos (2004).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub rsd: f64,
    /// Robust Standard Deviation of Residuals (`RSDR`, Motulsky & Brown 2006) populated under `RobustCauchy` or `RoutOutlierElimination` (`None` otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsdr: Option<f64>,
    /// Unweighted coefficient of determination $R^2 = 1 - \text{rss} / \text{TSS}$ in original $Y$ units
    /// even when weighted least squares or Poisson deviance is active (`NaN` when total SS is zero; Motulsky & Christopoulos 2004).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub r_squared: f64,
    /// Corrected Akaike Information Criterion ($\text{AIC}_c$, Burnham & Anderson 2002); `None` when `RobustCauchy` or insufficient residual df.
    pub aic_c: Option<f64>,
    /// Schwarz Bayesian Information Criterion ($\text{BIC}$); `None` when `RobustCauchy` or zero observations.
    pub bic: Option<f64>,
    /// Estimated free parameters with SEs, profile/Wald CIs, dependencies, Hougaard skewness, and boundary notes.
    pub parameters: Vec<FittedParameterSummary>,
    /// Fixed constants, hooked values, and equal-bound parameters `(name, value, notes)` consuming `0` free parameters.
    pub fixed_constants: Vec<(String, f64, Vec<ParameterNote>)>,
    /// Joint asymptotic covariance, correlation matrix, rank, and collinearity diagnostics over free parameters.
    #[serde(deserialize_with = "deserialize_covariance_diagnostics")]
    pub covariance: CovarianceDiagnostics,
    /// Per-dataset parameter vectors, predictions, residuals, per-curve $R^2$/$S_{y.x}$, and curve overlays.
    pub datasets: Vec<FittedDatasetResult>,
    /// Equation-specific derived physiological/pharmacological scalars (`EC50`, `t_half`, etc.) indexed by dataset slot.
    pub derived_scalars: Vec<Vec<DerivedScalar>>,
    /// Wald–Wolfowitz runs test result when `compute_runs_test` is enabled and enough non-zero residuals exist, else `None`.
    pub runs_test: Option<RunsTestResult>,
    /// Pure-error vs lack-of-fit *F*-test when `compute_lack_of_fit` is enabled and replicate `X` values allow, else `None`.
    pub lack_of_fit: Option<LackOfFitResult>,
    /// D'Agostino–Pearson $K^2$ residual normality test when `compute_normality` is enabled and sample size suffices, else `None`.
    pub normality: Option<NormalityTestResult>,
    /// Anderson–Darling residual normality test when `compute_normality` is enabled and sample size suffices, else `None`.
    pub normality_anderson_darling: Option<NormalityTestResult>,
    /// Lilliefors (Kolmogorov–Smirnov) residual normality test when `compute_normality` is enabled and sample size suffices, else `None`.
    pub normality_lilliefors_ks: Option<NormalityTestResult>,
    /// Spearman rank homoscedasticity check when `compute_homoscedasticity` is enabled and `n_obs >= 4`, else `None`.
    pub homoscedasticity: Option<HomoscedasticityResult>,
    /// ROUT outlier detection summary and per-point audit when `FitMethod::RoutOutlierElimination` is active, else `None`.
    pub rout: Option<RoutDetectionResult>,
    /// Unknown `X` values interpolated from observed `Y` responses when `interpolate_unknowns` is enabled.
    pub interpolated_x: Vec<InterpolatedUnknown>,
    /// Unknown `Y` values predicted forward from observed `X` values when `interpolate_unknowns` is enabled.
    pub interpolated_y: Vec<InterpolatedUnknown>,
}

impl NonlinearFitResult {
    /// Return `true` when parametric least-squares inference is suppressed due to Robust (Cauchy) fitting.
    #[must_use]
    pub fn is_robust_cauchy(&self) -> bool {
        self.fit_method == FitMethod::RobustCauchy || self.inference_availability.is_not_applicable_in_robust_mode()
    }

    /// Typed residual standard deviation (`None` in pure Robust Cauchy mode or when non-finite).
    #[must_use]
    pub fn rsd_option(&self) -> Option<f64> {
        (!self.is_robust_cauchy() && self.rsd.is_finite()).then_some(self.rsd)
    }

    /// Typed corrected Akaike information criterion (`None` in pure Robust Cauchy mode).
    #[must_use]
    pub fn aic_c_option(&self) -> Option<f64> {
        if self.is_robust_cauchy() { None } else { self.aic_c }
    }

    /// Typed Bayesian information criterion (`None` in pure Robust Cauchy mode).
    #[must_use]
    pub fn bic_option(&self) -> Option<f64> {
        if self.is_robust_cauchy() { None } else { self.bic }
    }

    /// Typed asymptotic covariance access, returning `Err(InferenceAvailability::NotApplicableInRobustMode)`
    /// in pure Robust (Cauchy) regression.
    pub fn covariance_if_available(&self) -> Result<&CovarianceDiagnostics, InferenceAvailability> {
        if self.is_robust_cauchy() { Err(InferenceAvailability::NotApplicableInRobustMode) } else { Ok(&self.covariance) }
    }
}

#[derive(Debug, Clone)]
struct FreeSlot {
    ds_local_idx: Option<usize>,
    dataset_idx: Option<usize>,
    name: String,
    lower: Option<f64>,
    upper: Option<f64>,
    init: f64,
    is_shared: bool,
}

#[derive(Debug, Clone)]
enum ParamBinding {
    Free(usize),
    Fixed(f64),
}

#[derive(Debug, Clone)]
struct ParameterLayout {
    p_model: usize,
    n_datasets: usize,
    bindings: Vec<Vec<ParamBinding>>,
    free_slots: Vec<FreeSlot>,
    fixed_constants: Vec<(String, f64, Vec<ParameterNote>)>,
}

impl ParameterLayout {
    fn build(model: &dyn ParametricModel, datasets: &[DatasetSeries], hooks: &HookContext, rules: &[ParameterRule]) -> Result<Self, FitError> {
        let param_names = model.param_names();
        let mut seen_rule_names: Vec<&str> = Vec::with_capacity(rules.len());
        for rule in rules {
            if !param_names.iter().any(|p| p.eq_ignore_ascii_case(&rule.name)) {
                return Err(FitError::InvalidParameter {
                    name: "parameter_rules",
                    reason: format!("Unknown parameter rule `{}` (expected one of: {})", rule.name, param_names.join(", ")),
                });
            }
            if seen_rule_names.iter().any(|prev| prev.eq_ignore_ascii_case(&rule.name)) {
                return Err(FitError::InvalidParameter { name: "parameter_rules", reason: format!("Duplicate parameter rule for `{}`", rule.name) });
            }
            seen_rule_names.push(&rule.name);
        }
        let p_model = param_names.len();
        let n_datasets = datasets.len();
        let all_points: Vec<DataPoint> = datasets.iter().flat_map(|d| d.points.iter().cloned()).collect();
        let default_init = model.initial_guess(&all_points);
        let default_bounds = model.default_bounds();
        let default_rules = model.default_parameter_rules(&all_points);

        let per_ds_init: Vec<Vec<f64>> = datasets.iter().map(|d| model.initial_guess(&d.points)).collect();

        let mut bindings = vec![vec![ParamBinding::Fixed(0.0); p_model]; n_datasets];
        let mut free_slots = Vec::new();
        let mut fixed_constants = Vec::new();

        for (p_idx, pname) in param_names.iter().enumerate() {
            let explicit_rule = rules.iter().find(|r| r.name.eq_ignore_ascii_case(pname));
            let rule = explicit_rule.or_else(|| default_rules.iter().find(|r| r.name.eq_ignore_ascii_case(pname)));
            let def_b = default_bounds.get(p_idx).copied().unwrap_or((None, None));
            let lb = rule.and_then(|r| r.lower_bound).or(def_b.0);
            let ub = rule.and_then(|r| r.upper_bound).or(def_b.1);
            if lb.is_some_and(|l| !l.is_finite()) || ub.is_some_and(|u| !u.is_finite()) {
                return Err(FitError::InvalidParameter { name: "parameter_rules", reason: format!("Non-finite bound for `{pname}`") });
            }
            if let (Some(l), Some(u)) = (lb, ub)
                && l > u
            {
                return Err(FitError::InvalidParameter {
                    name: "parameter_rules",
                    reason: format!("Lower bound ({l}) > upper bound ({u}) for `{pname}`"),
                });
            }
            let equal_bound = match (lb, ub) {
                (Some(l), Some(u)) if l == u => Some(l),
                _ => None,
            };

            let scope = rule.map(|r| &r.scope);
            match scope {
                Some(ParameterScope::Constant(val)) => {
                    if !val.is_finite() {
                        return Err(FitError::InvalidParameter { name: "parameter_rules", reason: format!("Non-finite constant for `{pname}`") });
                    }
                    for ds_row in &mut bindings {
                        ds_row[p_idx] = ParamBinding::Fixed(*val);
                    }
                    fixed_constants.push((pname.clone(), *val, vec![ParameterNote::Constant]));
                }
                Some(ParameterScope::HookedConstant(src)) => {
                    let val = hooks.resolve(src)?;
                    for ds_row in &mut bindings {
                        ds_row[p_idx] = ParamBinding::Fixed(val);
                    }
                    fixed_constants.push((pname.clone(), val, vec![ParameterNote::Hooked]));
                }
                Some(ParameterScope::PerDatasetConstant(pairs)) => {
                    for &(_, val) in pairs {
                        if !val.is_finite() {
                            return Err(FitError::InvalidParameter {
                                name: "parameter_rules",
                                reason: format!("Non-finite per-dataset constant for `{pname}`"),
                            });
                        }
                    }
                    for (ds_idx, ds) in datasets.iter().enumerate() {
                        if let Some(&(_, val)) = pairs.iter().rev().find(|&&(idx, _)| idx == ds.data_set_index) {
                            bindings[ds_idx][p_idx] = ParamBinding::Fixed(val);
                            let label = if n_datasets > 1 { format!("{pname} [DS{}]", ds.data_set_index + 1) } else { pname.clone() };
                            fixed_constants.push((label, val, vec![ParameterNote::Constant]));
                        } else if let Some(eq_val) = equal_bound {
                            bindings[ds_idx][p_idx] = ParamBinding::Fixed(eq_val);
                            let label = if n_datasets > 1 { format!("{pname} [DS{}]", ds.data_set_index + 1) } else { pname.clone() };
                            fixed_constants.push((label, eq_val, vec![ParameterNote::Constant]));
                        } else {
                            let slot = free_slots.len();
                            let init_v =
                                rule.and_then(|r| r.initial_override).unwrap_or_else(|| per_ds_init[ds_idx].get(p_idx).copied().unwrap_or(1.0));
                            free_slots.push(FreeSlot {
                                ds_local_idx: Some(ds_idx),
                                dataset_idx: Some(ds.data_set_index),
                                name: pname.clone(),
                                lower: lb,
                                upper: ub,
                                init: clamp_to_bounds(init_v, lb, ub),
                                is_shared: false,
                            });
                            bindings[ds_idx][p_idx] = ParamBinding::Free(slot);
                        }
                    }
                }
                Some(ParameterScope::ColumnTitleConstant) => {
                    for (ds_idx, ds) in datasets.iter().enumerate() {
                        let val =
                            ds.column_title_value.or_else(|| parse_column_title_numeric_hook(&ds.label).ok()).filter(|v| v.is_finite()).ok_or_else(
                                || FitError::HookResolutionError {
                                    message: format!("Column title `{}` for `{pname}` has no finite numeric constant", ds.label),
                                },
                            )?;
                        bindings[ds_idx][p_idx] = ParamBinding::Fixed(val);
                        let label = if n_datasets > 1 { format!("{pname} [DS{}]", ds.data_set_index + 1) } else { pname.clone() };
                        fixed_constants.push((label, val, vec![ParameterNote::Hooked]));
                    }
                }
                Some(ParameterScope::SharedGlobal) => {
                    if let Some(eq_val) = equal_bound {
                        for ds_row in &mut bindings {
                            ds_row[p_idx] = ParamBinding::Fixed(eq_val);
                        }
                        fixed_constants.push((pname.clone(), eq_val, vec![ParameterNote::Constant]));
                    } else {
                        let slot = free_slots.len();
                        let init_v = rule.and_then(|r| r.initial_override).unwrap_or_else(|| default_init.get(p_idx).copied().unwrap_or(1.0));
                        free_slots.push(FreeSlot {
                            ds_local_idx: None,
                            dataset_idx: None,
                            name: pname.clone(),
                            lower: lb,
                            upper: ub,
                            init: clamp_to_bounds(init_v, lb, ub),
                            is_shared: n_datasets > 1,
                        });
                        for ds_row in &mut bindings {
                            ds_row[p_idx] = ParamBinding::Free(slot);
                        }
                    }
                }
                None | Some(ParameterScope::Local) => {
                    if let Some(eq_val) = equal_bound {
                        for ds_row in &mut bindings {
                            ds_row[p_idx] = ParamBinding::Fixed(eq_val);
                        }
                        fixed_constants.push((pname.clone(), eq_val, vec![ParameterNote::Constant]));
                    } else if n_datasets <= 1 {
                        let slot = free_slots.len();
                        let init_v = rule.and_then(|r| r.initial_override).unwrap_or_else(|| default_init.get(p_idx).copied().unwrap_or(1.0));
                        let ds_ext_idx = datasets.first().map_or(0, |d| d.data_set_index);
                        free_slots.push(FreeSlot {
                            ds_local_idx: Some(0),
                            dataset_idx: Some(ds_ext_idx),
                            name: pname.clone(),
                            lower: lb,
                            upper: ub,
                            init: clamp_to_bounds(init_v, lb, ub),
                            is_shared: false,
                        });
                        if let Some(ds_row) = bindings.first_mut() {
                            ds_row[p_idx] = ParamBinding::Free(slot);
                        }
                    } else {
                        for (ds_idx, ds) in datasets.iter().enumerate() {
                            let slot = free_slots.len();
                            let init_v =
                                rule.and_then(|r| r.initial_override).unwrap_or_else(|| per_ds_init[ds_idx].get(p_idx).copied().unwrap_or(1.0));
                            free_slots.push(FreeSlot {
                                ds_local_idx: Some(ds_idx),
                                dataset_idx: Some(ds.data_set_index),
                                name: pname.clone(),
                                lower: lb,
                                upper: ub,
                                init: clamp_to_bounds(init_v, lb, ub),
                                is_shared: false,
                            });
                            bindings[ds_idx][p_idx] = ParamBinding::Free(slot);
                        }
                    }
                }
            }
        }

        Ok(Self { p_model, n_datasets, bindings, free_slots, fixed_constants })
    }

    fn k_free(&self) -> usize {
        self.free_slots.len()
    }

    fn bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        self.free_slots.iter().map(|s| (s.lower, s.upper)).collect()
    }

    fn expand_params(&self, theta: &[f64], ds_idx: usize) -> Vec<f64> {
        let mut out = vec![0.0_f64; self.p_model];
        self.expand_params_into(theta, ds_idx, &mut out);
        out
    }

    fn expand_params_into(&self, theta: &[f64], ds_idx: usize, out: &mut [f64]) {
        let row = &self.bindings[ds_idx.min(self.n_datasets - 1)];
        for (dst, b) in out.iter_mut().zip(row.iter()) {
            *dst = match b {
                ParamBinding::Free(slot) => theta[*slot],
                ParamBinding::Fixed(v) => *v,
            };
        }
    }

    fn fill_all_params(&self, theta: &[f64], flat_out: &mut [f64]) {
        for ds_idx in 0..self.n_datasets {
            let start = ds_idx * self.p_model;
            self.expand_params_into(theta, ds_idx, &mut flat_out[start..(start + self.p_model)]);
        }
    }
}

fn uses_shared_global(rules: &[ParameterRule]) -> bool {
    rules.iter().any(|r| matches!(r.scope, ParameterScope::SharedGlobal))
}

fn rules_for_dataset(rules: &[ParameterRule], data_set_index: usize) -> Vec<ParameterRule> {
    rules
        .iter()
        .map(|r| match &r.scope {
            ParameterScope::PerDatasetConstant(pairs) => {
                if let Some(&(_, v)) = pairs.iter().rev().find(|&&(d, _)| d == data_set_index) {
                    ParameterRule { scope: ParameterScope::Constant(v), ..r.clone() }
                } else {
                    ParameterRule { scope: ParameterScope::Local, ..r.clone() }
                }
            }
            _ => r.clone(),
        })
        .collect()
}

#[derive(Debug, Clone)]
struct ObsRecord {
    point: DataPoint,
    ds_local_idx: usize,
    sqrt_weight: f64,
    rep_n: usize,
    ss_within: f64,
}

fn uses_replicate_summary_moments(config: &SolverConfig) -> bool {
    config.method == FitMethod::LeastSquares && config.weighting == WeightingScheme::None
}

fn obs_replicate_count(obs: &[ObsRecord]) -> usize {
    obs.iter().map(|o| o.rep_n).sum()
}

fn obs_ss_within(obs: &[ObsRecord]) -> f64 {
    let mut acc = NeumaierSum::new();
    for o in obs {
        acc.add(o.ss_within);
    }
    acc.total().max(0.0)
}

fn clamp_to_bounds(val: f64, lb: Option<f64>, ub: Option<f64>) -> f64 {
    let mut v = if val.is_finite() { val } else { 0.0 };
    if let Some(l) = lb {
        v = v.max(l);
    }
    if let Some(u) = ub {
        v = v.min(u);
    }
    v
}

fn project_theta(theta: &mut [f64], bounds: &[(Option<f64>, Option<f64>)]) {
    for (v, &(lb, ub)) in theta.iter_mut().zip(bounds.iter()) {
        *v = clamp_to_bounds(*v, lb, ub);
    }
}

/// Observation weight `w_i` for weighted least squares (minimise `sum w_i (y_i - f_i)^2`).
///
/// Relative (`1/Y^2`, `1/|Y|`, `1/|Y|^k`), `1/X`, `1/X^2`, and inverse-variance (`1/SD^2`)
/// weighting follow the schemes described by Motulsky & Christopoulos (2004) and the weighted
/// least-squares framework of Seber & Wild (1989). Weights are computed from the observed
/// `Y` (not the fitted curve), and zero / non-finite denominators are rejected as
/// [`FitError::ImpossibleWeights`] rather than clamped.
fn compute_point_weight(pt: &DataPoint, scheme: WeightingScheme) -> Result<f64, FitError> {
    if let Some(w_exp) = pt.explicit_weight {
        if !w_exp.is_finite() || w_exp <= 0.0 {
            return Err(FitError::ImpossibleWeights { reason: "Explicit observation weight must be finite and strictly positive".to_owned() });
        }
        return Ok(w_exp);
    }
    let w = match scheme {
        WeightingScheme::None => 1.0,
        WeightingScheme::RelativeSquare => {
            if pt.y == 0.0 || !pt.y.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: "1/Y^2 weighting is undefined where Y == 0".to_owned() });
            }
            1.0 / (pt.y * pt.y)
        }
        WeightingScheme::Relative => {
            if pt.y == 0.0 || !pt.y.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: "1/|Y| weighting is undefined where Y == 0".to_owned() });
            }
            1.0 / pt.y.abs()
        }
        WeightingScheme::GeneralPowerY(k) => {
            if pt.y == 0.0 && k > 0.0 {
                return Err(FitError::ImpossibleWeights { reason: format!("1/|Y|^{k} weighting is undefined where Y == 0") });
            }
            let denom = pt.y.abs().powf(k);
            if !denom.is_finite() || denom <= 0.0 {
                return Err(FitError::ImpossibleWeights { reason: "General power Y weight underflowed or overflowed".to_owned() });
            }
            1.0 / denom
        }
        WeightingScheme::InverseX => {
            let x0 = pt.x0();
            if x0 == 0.0 || !x0.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: "1/|X| weighting is undefined where X == 0".to_owned() });
            }
            1.0 / x0.abs()
        }
        WeightingScheme::InverseX2 => {
            let x0 = pt.x0();
            if x0 == 0.0 || !x0.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: "1/X^2 weighting is undefined where X == 0".to_owned() });
            }
            1.0 / (x0 * x0)
        }
        WeightingScheme::ObservedSd => {
            let Some(sd) = pt.sd else {
                return Err(FitError::ImpossibleWeights { reason: "1/SD^2 weighting requires pre-summarized SD values".to_owned() });
            };
            if !sd.is_finite() || sd <= 0.0 {
                return Err(FitError::ImpossibleWeights { reason: "1/SD^2 weighting is undefined where SD <= 0".to_owned() });
            }
            1.0 / (sd * sd)
        }
    };
    if w.is_finite() && w > 0.0 { Ok(w) } else { Err(FitError::ImpossibleWeights { reason: "Weight overflowed f64 range".to_owned() }) }
}

/// Flatten datasets into active observations with verified square-root weights.
fn build_observations(datasets: &[DatasetSeries], scheme: WeightingScheme, honor_rep_moments: bool) -> Result<Vec<ObsRecord>, FitError> {
    let mut obs = Vec::new();
    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        for pt in &ds.points {
            let (sqrt_weight, rep_n, ss_within) = if honor_rep_moments && pt.explicit_weight.is_none() {
                let (n_f, n_u, ss_i) = validated_replicate_moments(pt)?;
                (n_f.sqrt(), n_u, ss_i)
            } else {
                let w = compute_point_weight(pt, scheme)?;
                (w.sqrt(), 1, 0.0)
            };
            obs.push(ObsRecord { point: pt.clone(), ds_local_idx, sqrt_weight, rep_n, ss_within });
        }
    }
    if obs.is_empty() {
        return Err(FitError::EmptyInput);
    }
    Ok(obs)
}

fn ode_budget_error() -> Option<FitError> {
    ode_fit_budget_exceeded().map(|max_steps| FitError::OdeIntegrationFailed {
        reason: format!("ODE integration step budget exceeded ({max_steps} adaptive RK45 steps per fit)"),
    })
}

fn obs_is_sweep_ordered(obs: &[ObsRecord], slot_ds: Option<usize>) -> bool {
    let mut prev: Option<(usize, f64)> = None;
    for rec in obs {
        if slot_ds.is_some_and(|d| d != rec.ds_local_idx) {
            continue;
        }
        let curr = (rec.ds_local_idx, rec.point.x0());
        if let Some(p) = prev
            && (curr.0 < p.0 || (curr.0 == p.0 && curr.1.total_cmp(&p.1).is_lt()))
        {
            return false;
        }
        prev = Some(curr);
    }
    true
}

fn eval_predictions_ordered(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    flat_params: &mut [f64],
    slot_ds: Option<usize>,
    y_hat_out: &mut [f64],
) -> bool {
    layout.fill_all_params(theta, flat_params);
    let p_m = layout.p_model;

    if obs_is_sweep_ordered(obs, slot_ds) {
        for (dst, rec) in y_hat_out.iter_mut().zip(obs.iter()) {
            if slot_ds.is_none_or(|d| d == rec.ds_local_idx) {
                let start = rec.ds_local_idx * p_m;
                let Some(pred) = model.eval(&rec.point.x, &flat_params[start..(start + p_m)], rec.point.data_set_index) else {
                    return false;
                };
                if !pred.is_finite() {
                    return false;
                }
                *dst = pred;
            }
        }
        return true;
    }

    // Evaluate unsorted observations in per-dataset ascending X order so stateful/ODE sweeps reuse
    // integration trajectories across points while writing back into original observation indices.
    let mut order: Vec<usize> = obs.iter().enumerate().filter(|(_, rec)| slot_ds.is_none_or(|d| d == rec.ds_local_idx)).map(|(i, _)| i).collect();
    order.sort_by(|&a, &b| obs[a].ds_local_idx.cmp(&obs[b].ds_local_idx).then_with(|| obs[a].point.x0().total_cmp(&obs[b].point.x0())));
    for idx in order {
        let rec = &obs[idx];
        let start = rec.ds_local_idx * p_m;
        let Some(pred) = model.eval(&rec.point.x, &flat_params[start..(start + p_m)], rec.point.data_set_index) else {
            return false;
        };
        if !pred.is_finite() {
            return false;
        }
        y_hat_out[idx] = pred;
    }
    true
}

/// Evaluate weighted residual vector `r_i = sqrt_w_i * (y_i - f(x_i; theta))` and predictions.
fn eval_residuals_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    require_positive_mean: bool,
) -> Option<(Vec<f64>, Vec<f64>, f64)> {
    let mut flat_params = vec![0.0_f64; layout.n_datasets * layout.p_model];
    let mut y_hat = vec![0.0_f64; obs.len()];
    if !eval_predictions_ordered(model, layout, obs, theta, &mut flat_params, None, &mut y_hat) {
        return None;
    }
    if require_positive_mean && y_hat.iter().any(|&pred| !pred.is_finite() || pred <= 0.0) {
        return None;
    }

    let mut weighted_res = Vec::with_capacity(obs.len());
    let mut ss = NeumaierSum::new();
    for (rec, &pred) in obs.iter().zip(y_hat.iter()) {
        let r = rec.sqrt_weight * (rec.point.y - pred);
        if !r.is_finite() {
            return None;
        }
        weighted_res.push(r);
        ss.add(r * r);
    }
    let total_ss = ss.total();
    if total_ss.is_finite() { Some((weighted_res, y_hat, total_ss)) } else { None }
}

/// Evaluate weighted residual vector `r_i = sqrt_w_i * (y_i - f(x_i; theta))` and predictions.
fn eval_residuals(model: &dyn ParametricModel, layout: &ParameterLayout, obs: &[ObsRecord], theta: &[f64]) -> Option<(Vec<f64>, Vec<f64>, f64)> {
    eval_residuals_with_domain(model, layout, obs, theta, false)
}

fn eval_predictions_into(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    flat_params: &mut [f64],
    y_hat_out: &mut [f64],
) -> bool {
    eval_predictions_ordered(model, layout, obs, theta, flat_params, None, y_hat_out)
}

fn mat_get(m: &DenseMatrix, r: usize, c: usize) -> f64 {
    m.get(r, c).unwrap_or(0.0)
}

fn mat_set(m: &mut DenseMatrix, r: usize, c: usize, val: f64) {
    if let Some(slot) = m.get_mut(r, c) {
        *slot = val;
    }
}

/// Compute weighted Jacobian `J_{i, j} = sqrt_w_i * df_i / dtheta_j` (`N x K`) using analytical
/// `model.grad` when supported, or central/forward finite differences otherwise.
///
/// Step sizes `h = 1e-6 * max(|theta_j|, 1)` (central, near `eps^(1/3)`) and
/// `h = 1.5e-8 * max(|theta_j|, 1)` (one-sided, near `eps^(1/2)`) follow the truncation /
/// round-off balance in Nocedal & Wright (2006), Ch. 8.
fn compute_jacobian(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    y_hat0: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
) -> Option<DenseMatrix> {
    let n = obs.len();
    let k = layout.k_free();
    let mut j_mat = DenseMatrix::zeros(n, k).ok()?;
    if k == 0 {
        return Some(j_mat);
    }

    // Try analytical gradient path first across all observations.
    let p_m = layout.p_model;
    let mut flat_params = vec![0.0_f64; layout.n_datasets * p_m];
    layout.fill_all_params(theta, &mut flat_params);

    let mut grad_buf = vec![0.0_f64; p_m];
    let mut analytical_ok = true;
    for (i, rec) in obs.iter().enumerate() {
        grad_buf.fill(0.0);
        let start = rec.ds_local_idx * p_m;
        if !model.grad(&rec.point.x, &flat_params[start..(start + p_m)], rec.point.data_set_index, &mut grad_buf) {
            analytical_ok = false;
            break;
        }
        for (p_idx, &g_val) in grad_buf.iter().enumerate().take(p_m) {
            if let ParamBinding::Free(slot) = layout.bindings[rec.ds_local_idx][p_idx] {
                let prev = mat_get(&j_mat, i, slot);
                mat_set(&mut j_mat, i, slot, prev + rec.sqrt_weight * g_val);
            }
        }
    }
    if analytical_ok {
        return Some(j_mat);
    }

    // Central difference Jacobian (with bounded one-sided fallback when near domain/box edges).
    j_mat = DenseMatrix::zeros(n, k).ok()?;
    let eps_central = 1e-6_f64;
    let eps_forward = 1.5e-8_f64;
    let mut work = theta.to_vec();
    let mut yp = vec![0.0_f64; n];
    let mut ym = vec![0.0_f64; n];

    for slot in 0..k {
        let info = &layout.free_slots[slot];
        let xj = theta[slot];
        let h_c = eps_central * xj.abs().max(1.0);
        let (lb, ub) = bounds[slot];

        let mut can_plus = ub.is_none_or(|u| xj + h_c <= u);
        let mut can_minus = lb.is_none_or(|l| xj - h_c >= l);

        if can_plus {
            work[slot] = xj + h_c;
            can_plus = eval_predictions_for_slot_into(model, layout, obs, &work, &mut flat_params, info.ds_local_idx, &mut yp);
        }
        if can_minus {
            work[slot] = xj - h_c;
            can_minus = eval_predictions_for_slot_into(model, layout, obs, &work, &mut flat_params, info.ds_local_idx, &mut ym);
        }
        work[slot] = xj;

        if can_plus && can_minus {
            let inv_2h = 1.0 / (2.0 * h_c);
            for (i, rec) in obs.iter().enumerate() {
                if info.ds_local_idx.is_none_or(|d| d == rec.ds_local_idx) {
                    mat_set(&mut j_mat, i, slot, rec.sqrt_weight * (yp[i] - ym[i]) * inv_2h);
                }
            }
            continue;
        }

        // One-sided fallback with smaller step `eps_forward`.
        let h_f = eps_forward * xj.abs().max(1.0);
        let sign = if ub.is_some_and(|u| xj + h_f > u) { -1.0 } else { 1.0 };
        work[slot] = xj + sign * h_f;
        let ok = eval_predictions_for_slot_into(model, layout, obs, &work, &mut flat_params, info.ds_local_idx, &mut yp);
        work[slot] = xj;
        if !ok {
            return None;
        }
        let inv_h = sign / h_f;
        for (i, rec) in obs.iter().enumerate() {
            if info.ds_local_idx.is_none_or(|d| d == rec.ds_local_idx) {
                mat_set(&mut j_mat, i, slot, rec.sqrt_weight * (yp[i] - y_hat0[i]) * inv_h);
            }
        }
    }
    Some(j_mat)
}

fn eval_predictions_for_slot_into(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    flat_params: &mut [f64],
    slot_ds: Option<usize>,
    out: &mut [f64],
) -> bool {
    eval_predictions_ordered(model, layout, obs, theta, flat_params, slot_ds, out)
}

#[derive(Debug, Clone)]
struct LmRunOutput {
    theta: Vec<f64>,
    weighted_res: Vec<f64>,
    y_hat: Vec<f64>,
    weighted_ss: f64,
    j_mat: DenseMatrix,
    status: FitStatus,
    iterations: usize,
}

/// Core Moré (1978) trust-region Levenberg–Marquardt solver using QR decomposition on the
/// augmented Jacobian `[J; sqrt(lambda) * D]` (`curvebook_math::linalg::qr`).
///
/// Follows Moré (1978) / MINPACK-1 `lmder` (Moré, Garbow & Hillstrom 1980) for: column scaling
/// `D_j = max(D_j, ||J_{:,j}||)`, initial radius `factor * ||D theta||`, the gain ratio
/// `rho = actual / predicted reduction`, the radius update (shrink by a clamped quadratic
/// `temp` in `[0.1, 0.5]` when `rho < 0.25`, expand to `2 ||D p||` when `rho > 0.75` or
/// `lambda == 0`), step acceptance at `rho >= 1e-4`, and the `ftol`/`xtol`/`gtol` tests.
///
/// Deviations from Moré (1978): the Levenberg–Marquardt parameter is located by the simplified
/// safeguarded iteration in [`solve_trust_region_subproblem`] rather than Moré's Newton
/// iteration; the expansion step uses `max(delta, 2 ||D p||)` (MINPACK sets `2 ||D p||`);
/// and box constraints are handled by projecting each trial point onto the bounds (not part of
/// Moré's unconstrained algorithm), with the predicted reduction evaluated on the projected step.
fn run_trust_region_lm_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    max_iters: usize,
    init_factor: f64,
    require_positive_mean: bool,
) -> Result<LmRunOutput, FitError> {
    let n = obs.len();
    let k = layout.k_free();
    let bounds = layout.bounds();
    let mut theta = theta_init.to_vec();
    project_theta(&mut theta, &bounds);

    let Some((mut w_res, mut y_hat, mut ss_curr)) = eval_residuals_with_domain(model, layout, obs, &theta, require_positive_mean) else {
        if let Some(err) = ode_budget_error() {
            return Err(err);
        }
        if require_positive_mean && eval_residuals_with_domain(model, layout, obs, &theta, false).is_some() {
            return Err(FitError::ImpossibleWeights {
                reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
            });
        }
        return Err(FitError::BadInitialValues { reason: "Initial parameters evaluate outside model domain".to_owned() });
    };

    if k == 0 {
        return Ok(LmRunOutput {
            theta,
            weighted_res: w_res,
            y_hat,
            weighted_ss: ss_curr,
            j_mat: DenseMatrix::zeros(n, 0)?,
            status: FitStatus::Converged,
            iterations: 0,
        });
    }

    let ftol = 1e-15_f64;
    let xtol = 1e-14_f64;
    let gtol = 1e-13_f64;
    let stall_gtol = 1e-4_f64;
    let y_norm = obs_y_ss(obs).sqrt();

    let mut diag = vec![0.0_f64; k];
    let mut delta = 0.0_f64;
    let mut iter = 0_usize;
    let mut last_max_g_cos = f64::INFINITY;
    let mut j_mat = DenseMatrix::zeros(n, k)?;
    let mut damp_ws = LmDampWorkspace::new(n, k)?;

    while iter < max_iters {
        j_mat = match compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds) {
            Some(jm) => jm,
            None => {
                if let Some(err) = ode_budget_error() {
                    return Err(err);
                }
                return Err(FitError::BadInitialValues { reason: "Jacobian evaluation returned non-finite values".to_owned() });
            }
        };

        if ss_curr == 0.0 {
            return Ok(LmRunOutput {
                theta,
                weighted_res: w_res,
                y_hat,
                weighted_ss: ss_curr,
                j_mat,
                status: FitStatus::Converged,
                iterations: iter,
            });
        }
        let max_g_cos = update_lm_diag_and_projected_grad(&j_mat, &w_res, &theta, &bounds, ss_curr, &mut diag);
        if iter == 0 {
            let xnorm = scaled_l2_norm(&diag, &theta);
            delta = init_factor * if xnorm > 0.0 { xnorm } else { 1.0 };
        }
        last_max_g_cos = max_g_cos;
        if max_g_cos <= gtol {
            return Ok(LmRunOutput {
                theta,
                weighted_res: w_res,
                y_hat,
                weighted_ss: ss_curr,
                j_mat,
                status: FitStatus::Converged,
                iterations: iter,
            });
        }

        let mut accepted = false;
        let mut lambda = 0.0_f64;
        for _ in 0..40 {
            let (step, lam_used) = solve_trust_region_subproblem(&j_mat, &w_res, &diag, &theta, delta, lambda, &mut damp_ws)?;
            lambda = lam_used;

            let ctx = LmStepContext { j_mat: &j_mat, w_res: &w_res, diag: &diag, theta: &theta };
            let (theta_trial, actual_step, p_norm) = project_tr_trial(&ctx, &step, &bounds, delta, &mut lambda)?;
            if iter == 0 && p_norm > 0.0 {
                delta = delta.min(p_norm);
            }

            // Predicted reduction: 2 r^T (J p) - ||J p||^2 (valid under active-bound projection too).
            let pred_num = quadratic_pred_num(&j_mat, &w_res, &actual_step).max(0.0);
            let pred_red = if ss_curr > 0.0 { pred_num / ss_curr } else { 0.0 };

            let (act_red, trial_eval) = match eval_residuals_with_domain(model, layout, obs, &theta_trial, require_positive_mean) {
                Some((r_t, y_t, ss_t)) => {
                    let act = if ss_t < ss_curr { 1.0 - ss_t / ss_curr } else { -1.0 };
                    (act, Some((r_t, y_t, ss_t)))
                }
                None => {
                    if let Some(err) = ode_budget_error() {
                        return Err(err);
                    }
                    (-1.0, None)
                }
            };

            let rho = if pred_red > 0.0 && trial_eval.is_some() {
                act_red / pred_red
            } else if act_red > 0.0 {
                1.0
            } else {
                -1.0
            };

            let ss_trial_val = trial_eval.as_ref().map(|(_, _, ss_t)| *ss_t);
            let outcome = TrStepOutcome { actual_step: &actual_step, p_norm, ss_curr, ss_trial: ss_trial_val, rho };
            update_tr_radius_more(&ctx, &outcome, &mut delta, &mut lambda);

            if rho >= 1e-4
                && let Some((r_t, y_t, ss_t)) = trial_eval
            {
                theta = theta_trial;
                w_res = r_t;
                y_hat = y_t;
                ss_curr = ss_t;
                iter += 1;
                accepted = true;

                let raw_xnorm = scaled_l2_norm(&diag, &theta);
                let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
                if (act_red.abs() <= ftol && pred_red <= ftol && rho <= 2.0) || p_norm <= xtol * x_norm {
                    let j_final = compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds).unwrap_or(j_mat);
                    return Ok(LmRunOutput {
                        theta,
                        weighted_res: w_res,
                        y_hat,
                        weighted_ss: ss_curr,
                        j_mat: j_final,
                        status: FitStatus::Converged,
                        iterations: iter,
                    });
                }
                break;
            }

            let raw_xnorm = scaled_l2_norm(&diag, &theta);
            let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
            if p_norm <= xtol * x_norm || delta <= 1e-15 * x_norm {
                let status = stalled_lm_status(max_g_cos, ss_curr, y_norm, stall_gtol);
                return Ok(LmRunOutput { theta, weighted_res: w_res, y_hat, weighted_ss: ss_curr, j_mat, status, iterations: iter });
            }
        }

        if !accepted {
            break;
        }
    }

    let j_final = compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds).unwrap_or(j_mat);
    let status = if iter >= max_iters { FitStatus::HitMaxIters } else { stalled_lm_status(last_max_g_cos, ss_curr, y_norm, stall_gtol) };
    Ok(LmRunOutput { theta, weighted_res: w_res, y_hat, weighted_ss: ss_curr, j_mat: j_final, status, iterations: iter })
}

fn update_lm_diag_and_projected_grad(
    j_mat: &DenseMatrix,
    w_res: &[f64],
    theta: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
    ss_curr: f64,
    diag: &mut [f64],
) -> f64 {
    let n = j_mat.nrows();
    let fnorm = ss_curr.sqrt();
    let mut max_g_cos = 0.0_f64;
    for (j, d_j) in diag.iter_mut().enumerate() {
        let mut col_ss = NeumaierSum::new();
        let mut col_dot = NeumaierSum::new();
        for (i, &w_r) in w_res.iter().enumerate().take(n) {
            let jv = mat_get(j_mat, i, j);
            col_ss.add(jv * jv);
            col_dot.add(jv * w_r);
        }
        let cnorm = col_ss.total().sqrt();
        let eff = if cnorm > f64::MIN_POSITIVE.sqrt() { cnorm } else { 1.0 };
        *d_j = if *d_j == 0.0 { eff } else { d_j.max(eff) };
        if cnorm > 0.0 && fnorm > 0.0 {
            let grad_j = col_dot.total();
            let at_lo = bounds[j].0.is_some_and(|l| (theta[j] - l).abs() <= 1e-12 * l.abs().max(f64::MIN_POSITIVE) && grad_j < 0.0);
            let at_hi = bounds[j].1.is_some_and(|u| (u - theta[j]).abs() <= 1e-12 * u.abs().max(f64::MIN_POSITIVE) && grad_j > 0.0);
            if !at_lo && !at_hi {
                max_g_cos = max_g_cos.max((grad_j / (cnorm * fnorm)).abs());
            }
        }
    }
    max_g_cos
}

fn stalled_lm_status(max_g_cos: f64, ss_curr: f64, y_norm: f64, stall_gtol: f64) -> FitStatus {
    if ss_curr == 0.0 {
        return FitStatus::Converged;
    }
    let fnorm = ss_curr.sqrt();
    let eff_g = if max_g_cos.is_finite() { max_g_cos * (fnorm / fnorm.max(1e-8 * y_norm)) } else { f64::INFINITY };
    if eff_g <= stall_gtol { FitStatus::Converged } else { FitStatus::Diverged }
}

struct LmStepContext<'a> {
    j_mat: &'a DenseMatrix,
    w_res: &'a [f64],
    diag: &'a [f64],
    theta: &'a [f64],
}

struct TrStepOutcome<'a> {
    actual_step: &'a [f64],
    p_norm: f64,
    ss_curr: f64,
    ss_trial: Option<f64>,
    rho: f64,
}

fn project_tr_trial(
    ctx: &LmStepContext<'_>,
    step: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
    delta: f64,
    lambda: &mut f64,
) -> Result<(Vec<f64>, Vec<f64>, f64), FitError> {
    let mut trial: Vec<f64> = ctx.theta.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
    project_theta(&mut trial, bounds);
    let mut actual_step: Vec<f64> = trial.iter().zip(ctx.theta.iter()).map(|(a, b)| a - b).collect();
    let mut p_norm = scaled_l2_norm(ctx.diag, &actual_step);
    let needs_face_step = has_active_bound_step(ctx.theta, step, bounds)
        && (p_norm <= 1e-14 * delta.max(1.0) || quadratic_pred_num(ctx.j_mat, ctx.w_res, &actual_step) <= 0.0);
    if needs_face_step
        && let Some((face_step, face_lam)) = solve_active_face_subproblem(ctx.j_mat, ctx.w_res, ctx.diag, ctx.theta, step, bounds, delta, *lambda)?
    {
        let mut face_trial: Vec<f64> = ctx.theta.iter().zip(face_step.iter()).map(|(a, b)| a + b).collect();
        project_theta(&mut face_trial, bounds);
        let face_actual: Vec<f64> = face_trial.iter().zip(ctx.theta.iter()).map(|(a, b)| a - b).collect();
        let face_norm = scaled_l2_norm(ctx.diag, &face_actual);
        if face_norm > 0.0 {
            trial = face_trial;
            actual_step = face_actual;
            p_norm = face_norm;
            *lambda = face_lam;
        }
    }
    Ok((trial, actual_step, p_norm))
}

fn update_tr_radius_more(ctx: &LmStepContext<'_>, outcome: &TrStepOutcome<'_>, delta: &mut f64, lambda: &mut f64) {
    if outcome.rho < 0.25 {
        let mut temp = 0.5_f64;
        if let Some(ss_t) = outcome.ss_trial
            && ss_t > outcome.ss_curr
        {
            let jp = matvec(ctx.j_mat, outcome.actual_step);
            let r_dot_jp: f64 = ctx.w_res.iter().zip(jp.iter()).map(|(r, j)| r * j).sum();
            let dir_deriv = -r_dot_jp;
            let ratio_t = 0.5 * dir_deriv / (dir_deriv + 0.5 * (outcome.ss_curr - ss_t));
            if ratio_t.is_finite() {
                temp = ratio_t.clamp(0.1, 0.5);
            }
        }
        let bound_scale = if *lambda == 0.0 { outcome.p_norm } else { outcome.p_norm * 10.0 };
        *delta = temp * delta.min(bound_scale).max(*delta * 0.1);
        if *lambda == 0.0 {
            let raw_xnorm = scaled_l2_norm(ctx.diag, ctx.theta);
            let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
            *delta = delta.min(0.8 * outcome.p_norm).max(1e-15 * x_norm);
        }
    } else if *lambda == 0.0 || outcome.rho > 0.75 {
        *delta = delta.max(2.0 * outcome.p_norm);
        *lambda *= 0.5;
    }
}

struct LmDampWorkspace {
    aug: DenseMatrix,
    rhs: Vec<f64>,
    col_scales: Vec<f64>,
}

impl LmDampWorkspace {
    fn new(n: usize, k: usize) -> Result<Self, FitError> {
        Ok(Self { aug: DenseMatrix::zeros(n + k, k)?, rhs: vec![0.0_f64; n + k], col_scales: vec![1.0_f64; k] })
    }
}

fn equilibrate_jacobian(j_mat: &DenseMatrix) -> Result<(DenseMatrix, Vec<f64>), FitError> {
    let n = j_mat.nrows();
    let k = j_mat.ncols();
    let mut scales = vec![0.0_f64; k];
    let mut j_eq = DenseMatrix::zeros(n, k)?;
    for (j, sc) in scales.iter_mut().enumerate() {
        let cn = j_mat.col_norm2(j);
        *sc = cn;
        if cn > 0.0 && cn.is_finite() {
            let inv = 1.0 / cn;
            for i in 0..n {
                mat_set(&mut j_eq, i, j, mat_get(j_mat, i, j) * inv);
            }
        }
    }
    Ok((j_eq, scales))
}

/// Solve the Moré trust-region subproblem `(J^T J + lambda D^2) p = J^T r` with `||D p|| <= delta`
/// via column-equilibrated QR decomposition of `J` and `[J; sqrt(lambda) D]`.
///
/// Follows Moré (1978, *Numerical Analysis*, Lecture Notes in Math. 630) Algorithm 4.1 /
/// MINPACK-1 `lmpar`:
/// 1. Equilibrates columns of `J` by Euclidean norm before `qr_col_pivoted` so mixed parameter
///    units never trigger false rank deficiency (`pqr.rank() < k`).
/// 2. Accepts the full-rank Gauss–Newton step (`lambda = 0`) whenever `||D p_gn|| <= 1.1 delta`.
/// 3. Computes exact upper bound `paru = ||D^{-1} J^T r|| / delta` and lower bound `parl` via
///    `R_eq^T s = P^T ((D^2 p_gn / ||D p_gn||) / col_scale)` when `J` has full column rank (`0` otherwise).
/// 4. Refines `lambda` using Moré's rational-model Newton update `parc = phi / (delta * ||s||^2)`
///    where `R_eq^T s = (D^2 p / ||D p||) / col_scale` from the equilibrated QR factor `R_eq` of `[J; sqrt(lambda) D]`.
fn solve_trust_region_subproblem(
    j_mat: &DenseMatrix,
    r: &[f64],
    diag: &[f64],
    _theta: &[f64],
    delta: f64,
    lambda_hint: f64,
    ws: &mut LmDampWorkspace,
) -> Result<(Vec<f64>, f64), FitError> {
    let k = j_mat.ncols();
    let (j_eq, col_scales) = equilibrate_jacobian(j_mat)?;
    let pqr = qr_col_pivoted(&j_eq)?;
    let mut parl = 0.0_f64;
    let mut gn_norm_for_init = 0.0_f64;
    if pqr.rank() == k
        && col_scales.iter().all(|&c| c > 0.0 && c.is_finite())
        && let Ok(mut p_gn) = pqr.solve_basic(r)
    {
        for (pj, &cj) in p_gn.iter_mut().zip(col_scales.iter()) {
            *pj /= cj;
        }
        if p_gn.iter().all(|v| v.is_finite()) {
            let p_norm = scaled_l2_norm(diag, &p_gn);
            if p_norm <= 1.1 * delta {
                return Ok((p_gn, 0.0));
            }
            gn_norm_for_init = p_norm;
            if let Ok(r_gn) = pqr.r()
                && let Some(s_norm_sq) = more_r_inv_t_norm_sq(&r_gn, diag, &p_gn, p_norm, Some(pqr.permutation()), Some(&col_scales))
            {
                parl = ((p_norm - delta) / (delta.max(f64::MIN_POSITIVE) * s_norm_sq)).max(0.0);
            }
        }
    }

    // Scaled gradient norm ||D^{-1} J^T r||_2 and upper bracket paru = ||D^{-1} J^T r|| / delta.
    let mut grad_scaled_ss = 0.0_f64;
    for (j, &dj) in diag.iter().enumerate().take(k) {
        let mut g_j = 0.0_f64;
        for (i, &ri) in r.iter().enumerate() {
            g_j += mat_get(j_mat, i, j) * ri;
        }
        let s = g_j / dj;
        grad_scaled_ss += s * s;
    }
    let gnorm = grad_scaled_ss.sqrt();
    if gnorm == 0.0 || !gnorm.is_finite() {
        return Ok((vec![0.0_f64; k], 0.0));
    }
    let mut paru = (gnorm / delta.max(f64::MIN_POSITIVE)).max(1e-12);
    // When rank-deficient (parl == 0), keep sqrt(lambda) safely above QR rounding noise in nullspace.
    let lam_floor = if pqr.rank() < k { (1e-10 * paru).max(1e-12).min(paru) } else { (1e-14 * paru).max(1e-24).min(paru) };
    let mut lam = lambda_hint.max(parl).min(paru);
    if lam < lam_floor {
        let fallback = if gn_norm_for_init > 0.0 { gnorm / gn_norm_for_init } else { 0.001 * paru };
        lam = fallback.clamp(parl.max(lam_floor), paru);
    }

    let mut best_p = vec![0.0_f64; k];
    let mut fp_prev = 0.0_f64;
    for iter in 0..10 {
        if lam < lam_floor {
            lam = (0.001 * paru).max(parl).max(lam_floor).min(paru);
        }
        let (p, r_mat) = solve_damped_qr_with_r(j_mat, r, diag, lam, ws)?;
        let p_norm = scaled_l2_norm(diag, &p);
        let fp = p_norm - delta;
        best_p = p;
        if fp.abs() <= 0.1 * delta || (parl == 0.0 && fp <= fp_prev && fp_prev < 0.0) || iter == 9 {
            return Ok((best_p, lam));
        }
        let parc = match more_r_inv_t_norm_sq(&r_mat, diag, &best_p, p_norm, None, Some(&ws.col_scales)) {
            Some(s_norm_sq) => fp / (delta.max(f64::MIN_POSITIVE) * s_norm_sq),
            None => 0.0,
        };
        if fp > 0.0 {
            parl = parl.max(lam);
        } else if fp < 0.0 {
            paru = paru.min(lam);
        }
        let next_lam = parl.max(lam + parc).clamp(lam_floor.min(paru), paru);
        if (next_lam - lam).abs() <= 1e-12 * lam.max(1e-30) || (paru - parl) <= 1e-12 * paru.max(1e-30) {
            return Ok((best_p, next_lam));
        }
        lam = next_lam;
        fp_prev = fp;
    }
    Ok((best_p, lam))
}

fn solve_damped_qr_with_r(
    j_mat: &DenseMatrix,
    r: &[f64],
    diag: &[f64],
    lambda: f64,
    ws: &mut LmDampWorkspace,
) -> Result<(Vec<f64>, DenseMatrix), FitError> {
    let n = j_mat.nrows();
    let k = j_mat.ncols();
    let sqrt_lam = lambda.sqrt();
    for j in 0..k {
        let dj = diag.get(j).copied().unwrap_or(1.0);
        let raw_sj = j_mat.col_norm2(j).hypot(sqrt_lam * dj);
        let sj = if raw_sj > 0.0 && raw_sj.is_finite() { raw_sj } else { 1.0 };
        ws.col_scales[j] = sj;
        let inv_sj = 1.0 / sj;
        for i in 0..n {
            mat_set(&mut ws.aug, i, j, mat_get(j_mat, i, j) * inv_sj);
        }
        for i in 0..k {
            mat_set(&mut ws.aug, n + i, j, if i == j { sqrt_lam * dj * inv_sj } else { 0.0 });
        }
    }
    ws.rhs[..n].copy_from_slice(r);
    ws.rhs[n..(n + k)].fill(0.0);

    let qr_res = qr(&ws.aug)?;
    let mut p = qr_res.solve_basic(&ws.rhs)?;
    let r_mat = qr_res.r()?;
    for (pj, &sj) in p.iter_mut().zip(ws.col_scales.iter()) {
        *pj /= sj;
    }
    Ok((p, r_mat))
}

fn more_r_inv_t_norm_sq(
    r_mat: &DenseMatrix,
    diag: &[f64],
    p: &[f64],
    p_norm: f64,
    perm: Option<&[usize]>,
    col_scales: Option<&[f64]>,
) -> Option<f64> {
    if p_norm <= 0.0 || !p_norm.is_finite() {
        return None;
    }
    let k = diag.len();
    let mut s = vec![0.0_f64; k];
    for j in 0..k {
        let orig_j = perm.map_or(j, |pm| pm[j]);
        let mut sum = (diag[orig_j] * diag[orig_j] * p[orig_j]) / p_norm;
        if let Some(cs) = col_scales {
            let c = cs[orig_j];
            if !c.is_finite() || c <= 0.0 {
                return None;
            }
            sum /= c;
        }
        for (i, &si) in s.iter().enumerate().take(j) {
            sum -= mat_get(r_mat, i, j) * si;
        }
        let rjj = mat_get(r_mat, j, j);
        if !rjj.is_finite() || rjj.abs() <= 1e-300 {
            return None;
        }
        s[j] = sum / rjj;
    }
    let norm_sq: f64 = s.iter().map(|v| v * v).sum();
    (norm_sq.is_finite() && norm_sq > 0.0).then_some(norm_sq)
}

fn quadratic_pred_num(j_mat: &DenseMatrix, w_res: &[f64], step: &[f64]) -> f64 {
    let jp = matvec(j_mat, step);
    let jp_ss: f64 = jp.iter().map(|v| v * v).sum();
    let r_dot_jp: f64 = w_res.iter().zip(jp.iter()).map(|(r, j)| r * j).sum();
    2.0 * r_dot_jp - jp_ss
}

fn is_bound_actively_blocked(theta_j: f64, step_j: f64, bound: (Option<f64>, Option<f64>)) -> bool {
    let at_lo = bound.0.is_some_and(|lo| {
        let tol = 1e-12 * lo.abs().max(f64::MIN_POSITIVE);
        theta_j <= lo + tol && step_j < 0.0
    });
    let at_hi = bound.1.is_some_and(|hi| {
        let tol = 1e-12 * hi.abs().max(f64::MIN_POSITIVE);
        theta_j >= hi - tol && step_j > 0.0
    });
    at_lo || at_hi
}

fn has_active_bound_step(theta: &[f64], step: &[f64], bounds: &[(Option<f64>, Option<f64>)]) -> bool {
    theta.iter().zip(step.iter()).zip(bounds.iter()).any(|((&tj, &sj), &b)| is_bound_actively_blocked(tj, sj, b))
}

#[allow(clippy::too_many_arguments)]
fn solve_active_face_subproblem(
    j_mat: &DenseMatrix,
    r: &[f64],
    diag: &[f64],
    theta: &[f64],
    unconstrained_step: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
    delta: f64,
    lambda_hint: f64,
) -> Result<Option<(Vec<f64>, f64)>, FitError> {
    let n = j_mat.nrows();
    let k = j_mat.ncols();
    let free_cols: Vec<usize> = (0..k).filter(|&j| !is_bound_actively_blocked(theta[j], unconstrained_step[j], bounds[j])).collect();
    if free_cols.is_empty() || free_cols.len() == k {
        return Ok(None);
    }
    let kf = free_cols.len();
    let mut j_free = DenseMatrix::zeros(n, kf)?;
    let mut diag_free = Vec::with_capacity(kf);
    let mut theta_free = Vec::with_capacity(kf);
    for (col_idx, &orig_j) in free_cols.iter().enumerate() {
        diag_free.push(diag[orig_j]);
        theta_free.push(theta[orig_j]);
        for i in 0..n {
            mat_set(&mut j_free, i, col_idx, mat_get(j_mat, i, orig_j));
        }
    }
    let mut ws_free = LmDampWorkspace::new(n, kf)?;
    let (sub_step, sub_lam) = solve_trust_region_subproblem(&j_free, r, &diag_free, &theta_free, delta, lambda_hint, &mut ws_free)?;
    let mut full_step = vec![0.0_f64; k];
    for (col_idx, &orig_j) in free_cols.iter().enumerate() {
        full_step[orig_j] = sub_step[col_idx];
    }
    Ok(Some((full_step, sub_lam)))
}

fn scaled_l2_norm(diag: &[f64], v: &[f64]) -> f64 {
    let mut s = NeumaierSum::new();
    for (&d, &x) in diag.iter().zip(v.iter()) {
        let prod = d * x;
        s.add(prod * prod);
    }
    s.total().sqrt()
}

fn matvec(a: &DenseMatrix, v: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0_f64; a.nrows()];
    for (i, row_out) in out.iter_mut().enumerate() {
        let mut acc = NeumaierSum::new();
        for (j, &vj) in v.iter().enumerate() {
            acc.add(mat_get(a, i, j) * vj);
        }
        *row_out = acc.total();
    }
    out
}

fn run_trust_region_lm(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    max_iters: usize,
    init_factor: f64,
) -> Result<LmRunOutput, FitError> {
    run_trust_region_lm_with_domain(model, layout, obs, theta_init, max_iters, init_factor, false)
}

/// Nelder–Mead simplex warm-start / local rescue for difficult starts or non-finite initial points.
///
/// Nelder & Mead (1965) with standard coefficients (reflection 1, expansion 2, contraction 0.5,
/// shrink 0.5) and the ordering, greedy-expansion, and outside/inside contraction rules of
/// Lagarias et al. (1998). Box constraints are enforced by projecting vertices before
/// evaluation (a pragmatic extension, not part of either reference). Termination is on the
/// relative spread of objective values or the evaluation budget only.
fn nelder_mead_optimize<F>(layout: &ParameterLayout, seed: &[f64], max_evals: usize, zero_step: f64, mut eval_obj: F) -> Option<Vec<f64>>
where
    F: FnMut(&[f64]) -> f64,
{
    let k = layout.k_free();
    if k == 0 {
        return Some(Vec::new());
    }
    let bounds = layout.bounds();
    let mut simplex: Vec<(Vec<f64>, f64)> = Vec::with_capacity(k + 1);
    let mut v0 = seed.to_vec();
    project_theta(&mut v0, &bounds);
    let f0 = eval_obj(&v0);
    if ode_fit_budget_exceeded().is_some() {
        return None;
    }
    simplex.push((v0.clone(), f0));

    for j in 0..k {
        let mut vj = v0.clone();
        let step = if vj[j].abs() > 1e-8 { 0.1 * vj[j] } else { zero_step };
        vj[j] += step;
        project_theta(&mut vj, &bounds);
        let fj = eval_obj(&vj);
        if ode_fit_budget_exceeded().is_some() {
            return None;
        }
        simplex.push((vj, fj));
    }

    let mut evals = k + 1;
    while evals < max_evals {
        if ode_fit_budget_exceeded().is_some() {
            return None;
        }
        simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
        if simplex[0].1.is_finite() && simplex[k].1.is_finite() {
            let spread = (simplex[k].1 - simplex[0].1).abs();
            if spread <= 1e-12 * simplex[0].1.abs().max(1.0) {
                break;
            }
        }

        let mut centroid = vec![0.0_f64; k];
        for (v, _) in &simplex[..k] {
            for (c, &x) in centroid.iter_mut().zip(v.iter()) {
                *c += x;
            }
        }
        for c in &mut centroid {
            *c /= k as f64;
        }

        let worst = &simplex[k].0;
        let xr: Vec<f64> = centroid.iter().zip(worst.iter()).map(|(c, w)| c + (c - w)).collect();
        let fr = eval_obj(&xr);
        evals += 1;

        if fr < simplex[0].1 {
            let xe: Vec<f64> = centroid.iter().zip(xr.iter()).map(|(c, r)| c + 2.0 * (r - c)).collect();
            let fe = eval_obj(&xe);
            evals += 1;
            simplex[k] = if fe < fr { (xe, fe) } else { (xr, fr) };
        } else if fr < simplex[k - 1].1 {
            simplex[k] = (xr, fr);
        } else {
            let use_outside = fr < simplex[k].1;
            let base = if use_outside { &xr } else { &simplex[k].0 };
            let f_base = if use_outside { fr } else { simplex[k].1 };
            let xc: Vec<f64> = centroid.iter().zip(base.iter()).map(|(c, b)| c + 0.5 * (b - c)).collect();
            let fc = eval_obj(&xc);
            evals += 1;
            if fc <= f_base {
                simplex[k] = (xc, fc);
            } else {
                let best = simplex[0].0.clone();
                for vert in simplex.iter_mut().take(k + 1).skip(1) {
                    for (vj, &b) in vert.0.iter_mut().zip(best.iter()) {
                        *vj = b + 0.5 * (*vj - b);
                    }
                    project_theta(&mut vert.0, &bounds);
                    vert.1 = eval_obj(&vert.0);
                    evals += 1;
                }
            }
        }
    }

    simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
    if simplex[0].1.is_finite() {
        let mut best = simplex[0].0.clone();
        project_theta(&mut best, &bounds);
        Some(best)
    } else {
        None
    }
}

/// Nelder–Mead simplex warm-start / local rescue for difficult starts or non-finite initial points.
///
/// Nelder & Mead (1965) with standard coefficients (reflection 1, expansion 2, contraction 0.5,
/// shrink 0.5) and the ordering, greedy-expansion, and outside/inside contraction rules of
/// Lagarias et al. (1998). Box constraints are enforced by projecting vertices before
/// evaluation (a pragmatic extension, not part of either reference). Termination is on the
/// relative spread of objective values or the evaluation budget only.
fn nelder_mead_rescue_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
    max_evals: usize,
    require_positive_mean: bool,
) -> Option<Vec<f64>> {
    let bounds = layout.bounds();
    nelder_mead_optimize(layout, seed, max_evals, 0.05, |th| {
        if ode_fit_budget_exceeded().is_some() {
            return f64::INFINITY;
        }
        let mut clamped = th.to_vec();
        project_theta(&mut clamped, &bounds);
        eval_residuals_with_domain(model, layout, obs, &clamped, require_positive_mean).map_or(f64::INFINITY, |(_, _, ss)| ss)
    })
}

fn nelder_mead_poisson_feasibility_rescue(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
) -> Option<Vec<f64>> {
    let bounds = layout.bounds();
    let y_scale = obs.iter().map(|o| o.point.y.abs()).fold(0.0_f64, f64::max).max(1e-6);
    let eps_pos = 1e-4 * y_scale;
    let cand = nelder_mead_optimize(layout, seed, 400, 1.0, |th| {
        if ode_fit_budget_exceeded().is_some() {
            return f64::INFINITY;
        }
        let mut clamped = th.to_vec();
        project_theta(&mut clamped, &bounds);
        match eval_residuals_with_domain(model, layout, obs, &clamped, false) {
            Some((_, y_hat, ss)) => {
                let mut penalty = 0.0_f64;
                let mut all_pos = true;
                for &mu in &y_hat {
                    if mu <= 0.0 {
                        all_pos = false;
                    }
                    if mu < eps_pos {
                        let d = eps_pos - mu;
                        penalty += d * d;
                    }
                }
                if all_pos { ss } else { 1e50 + 1e30 * penalty + ss.min(1e20) }
            }
            None => f64::INFINITY,
        }
    })?;
    eval_residuals_with_domain(model, layout, obs, &cand, true).map(|_| cand)
}

/// Multi-factor trust-region LM driver with optional simplex rescue for hard starts.
fn solve_with_warm_start(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    solve_with_warm_start_with_domain(model, layout, obs, theta_init, config, config.method == FitMethod::PoissonDeviance)
}

fn solve_with_warm_start_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    config: &SolverConfig,
    require_positive_mean: bool,
) -> Result<LmRunOutput, FitError> {
    let mut start = theta_init.to_vec();
    if eval_residuals_with_domain(model, layout, obs, &start, require_positive_mean).is_none() {
        if let Some(err) = ode_budget_error() {
            return Err(err);
        }
        if config.use_simplex_warm_start
            && let Some(rescued) = nelder_mead_rescue_with_domain(model, layout, obs, &start, 240, require_positive_mean)
        {
            start = rescued;
        } else if let Some(err) = ode_budget_error() {
            return Err(err);
        } else if require_positive_mean && eval_residuals_with_domain(model, layout, obs, &start, false).is_some() {
            if !config.use_simplex_warm_start && layout.k_free() > 0 {
                return Err(FitError::BadInitialValues {
                    reason: "Starting parameters produce non-positive Poisson fitted means (mu <= 0)".to_owned(),
                });
            }
            return Err(FitError::ImpossibleWeights {
                reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
            });
        } else {
            return Err(FitError::BadInitialValues { reason: "Starting parameters evaluate outside model domain".to_owned() });
        }
    }

    let mut best = run_trust_region_lm_with_domain(model, layout, obs, &start, config.max_iters, 100.0, require_positive_mean)?;
    // If primary run hit max iters or suspiciously poor gradient / flat plateau, try smaller step
    // trust-region factor `1.0` and optional Nelder-Mead restart purely from `start` or `best.theta`.
    if config.use_simplex_warm_start && (best.status != FitStatus::Converged || best.weighted_ss > 1e-3 * obs_y_ss(obs)) {
        match run_trust_region_lm_with_domain(model, layout, obs, &start, config.max_iters, 1.0, require_positive_mean) {
            Ok(alt1)
                if alt1.weighted_ss < best.weighted_ss
                    || (best.status != FitStatus::Converged && alt1.status == FitStatus::Converged && alt1.weighted_ss <= best.weighted_ss) =>
            {
                best = alt1;
            }
            Err(err @ FitError::OdeIntegrationFailed { .. }) => return Err(err),
            _ => {}
        }
        if let Some(nm) = nelder_mead_rescue_with_domain(model, layout, obs, &best.theta, 400, require_positive_mean) {
            match run_trust_region_lm_with_domain(model, layout, obs, &nm, config.max_iters, 1.0, require_positive_mean) {
                Ok(alt2)
                    if alt2.weighted_ss < best.weighted_ss
                        || (best.status != FitStatus::Converged && alt2.status == FitStatus::Converged && alt2.weighted_ss <= best.weighted_ss) =>
                {
                    best = alt2;
                }
                Err(err @ FitError::OdeIntegrationFailed { .. }) => return Err(err),
                _ => {}
            }
        } else if let Some(err) = ode_budget_error() {
            return Err(err);
        }
        if best.weighted_ss > 1e-2 * obs_y_ss(obs) || best.status != FitStatus::Converged {
            if let Some(nm_start) = nelder_mead_rescue_with_domain(model, layout, obs, &start, 600, require_positive_mean) {
                match run_trust_region_lm_with_domain(model, layout, obs, &nm_start, config.max_iters, 0.5, require_positive_mean) {
                    Ok(alt3)
                        if alt3.weighted_ss < best.weighted_ss
                            || (best.status != FitStatus::Converged
                                && alt3.status == FitStatus::Converged
                                && alt3.weighted_ss <= best.weighted_ss) =>
                    {
                        best = alt3;
                    }
                    Err(err @ FitError::OdeIntegrationFailed { .. }) => return Err(err),
                    _ => {}
                }
            } else if let Some(err) = ode_budget_error() {
                return Err(err);
            }
        }
    }
    Ok(best)
}

fn obs_y_ss(obs: &[ObsRecord]) -> f64 {
    let mut acc = NeumaierSum::new();
    for o in obs {
        acc.add(o.sqrt_weight * o.sqrt_weight * o.point.y * o.point.y);
    }
    acc.total().max(f64::MIN_POSITIVE)
}

/// Symmetric Wald intervals `theta_j +/- t_{df, alpha/2} * SE_j` from the linearised covariance
/// (Bates & Watts 1988; Seber & Wild 1989), reported as open when the fit is ambiguous.
fn wald_ci_bounds(cov: &CovarianceDiagnostics, df: usize) -> Vec<(CiBound, CiBound)> {
    cov.standard_errors
        .iter()
        .zip(cov.symmetric_ci_lower.iter().zip(cov.symmetric_ci_upper.iter()))
        .map(|(&se, (&lo, &hi))| {
            if !cov.ambiguous && df > 0 && se.is_finite() && lo.is_finite() && hi.is_finite() {
                (CiBound::Finite(lo), CiBound::Finite(hi))
            } else {
                (CiBound::Open, CiBound::Open)
            }
        })
        .collect()
}

fn unscaled_cov_from_diag_or_j(cov: &CovarianceDiagnostics, df: usize, weighted_ss: f64, j_mat: &DenseMatrix) -> Option<DenseMatrix> {
    let k = cov.covariance_matrix.len();
    if cov.ambiguous || k == 0 || j_mat.ncols() == 0 {
        return None;
    }
    let sigma2 = if df > 0 { weighted_ss / (df as f64) } else { 0.0 };
    if sigma2 > 0.0 && sigma2.is_finite() {
        let mut mat = DenseMatrix::zeros(k, k).ok()?;
        let inv_s2 = 1.0 / sigma2;
        for (i, row) in cov.covariance_matrix.iter().enumerate().take(k) {
            for (j, &v) in row.iter().enumerate().take(k) {
                mat_set(&mut mat, i, j, v * inv_s2);
            }
        }
        return Some(mat);
    }
    compute_unscaled_cov(j_mat)
}

/// Internally studentised residuals `e_i / (s * sqrt(1 - h_ii))` with tangent-plane leverage
/// `h_ii = J_i (J^T J)^{-1} J_i^T` from the weighted Jacobian (Cook & Weisberg 1982, applied to
/// the linearised model). Leverage is set to zero (plain standardised residuals) when the fit
/// is ambiguous or `K > 64`.
///
/// Reference: Cook, R. D. & Weisberg, S. (1982). *Residuals and Influence in Regression*. Chapman & Hall.
fn compute_studentized_residuals(j_mat: &DenseMatrix, w_res: &[f64], cov_unscaled: Option<&DenseMatrix>, sigma_w: f64, ambiguous: bool) -> Vec<f64> {
    let n = w_res.len();
    if !sigma_w.is_finite() || sigma_w <= 0.0 {
        return vec![0.0; n];
    }
    let k = j_mat.ncols();
    let use_leverage = !ambiguous && cov_unscaled.is_some() && k > 0 && k <= 64 && j_mat.nrows() == n;
    let mut out = Vec::with_capacity(n);
    for (i, &wr) in w_res.iter().enumerate() {
        let h_ii = if use_leverage && let Some(cov_u) = cov_unscaled {
            let mut h = 0.0_f64;
            for a in 0..k {
                let ja = mat_get(j_mat, i, a);
                for b in 0..k {
                    h += ja * mat_get(cov_u, a, b) * mat_get(j_mat, i, b);
                }
            }
            h.clamp(0.0, 1.0 - 1e-12)
        } else {
            0.0
        };
        out.push(wr / (sigma_w * (1.0 - h_ii).sqrt()));
    }
    out
}

fn canonical_x_bits(x: &[f64]) -> Vec<u64> {
    x.iter().map(|&v| if v == 0.0 { 0.0_f64.to_bits() } else { v.to_bits() }).collect()
}

fn poisson_log_likelihood_from_obs(obs: &[ObsRecord], y_hat: &[f64]) -> Option<f64> {
    if obs.len() != y_hat.len() || obs.is_empty() {
        return None;
    }
    let mut sum = NeumaierSum::new();
    for (rec, &mu) in obs.iter().zip(y_hat.iter()) {
        let y = rec.point.y;
        if !y.is_finite() || y < 0.0 || !mu.is_finite() || mu <= 0.0 {
            return None;
        }
        let lg = ln_gamma(y + 1.0).ok()?;
        let y_ln_mu = if y > 0.0 { y * mu.ln() } else { 0.0 };
        sum.add(y_ln_mu - mu - lg);
    }
    let ll = sum.total();
    ll.is_finite().then_some(ll)
}

fn information_criteria_from_poisson_ll(ll: f64, n_obs: usize, k_free: usize) -> Option<InformationCriteria> {
    // Poisson likelihood fixes dispersion phi = 1, so effective K = k_free (no +1 variance parameter).
    if !ll.is_finite() || n_obs <= k_free + 1 {
        return None;
    }
    let k = k_free as f64;
    let n = n_obs as f64;
    let neg2_ll = -2.0 * ll;
    let aic = neg2_ll + 2.0 * k;
    let aicc = aic + (2.0 * k * (k + 1.0)) / (n - k - 1.0);
    let bic = neg2_ll + k * n.ln();
    Some(InformationCriteria { aicc, bic, n_obs, k_params_plus_sigma: k_free })
}

fn poisson_information_criteria(obs: &[ObsRecord], y_hat: &[f64], k_free: usize) -> Option<InformationCriteria> {
    poisson_log_likelihood_from_obs(obs, y_hat).and_then(|ll| information_criteria_from_poisson_ll(ll, obs.len(), k_free))
}

fn compare_poisson_aicc(aicc_a: f64, aicc_b: f64) -> Result<AiccComparisonResult, FitError> {
    if !aicc_a.is_finite() || !aicc_b.is_finite() {
        return Err(FitError::InvalidParameter { name: "comparison", reason: "Poisson AICc values must be finite for model comparison".to_owned() });
    }
    let delta_aicc = aicc_b - aicc_a;
    let min_aicc = aicc_a.min(aicc_b);
    let exp_a = (-0.5 * (aicc_a - min_aicc)).exp();
    let exp_b = (-0.5 * (aicc_b - min_aicc)).exp();
    let denom = exp_a + exp_b;
    let weight_a = exp_a / denom;
    let weight_b = exp_b / denom;
    let preferred_index = usize::from(aicc_b < aicc_a);
    let evidence_ratio = (0.5 * delta_aicc.abs()).exp();
    Ok(AiccComparisonResult { aicc_a, aicc_b, delta_aicc, weight_a, weight_b, evidence_ratio, preferred_index })
}

fn group_pure_error_ss_ls(obs: &[ObsRecord], idxs: &[usize]) -> Option<f64> {
    let mut sum_within = 0.0_f64;
    let mut sum_w = 0.0_f64;
    let mut sum_wy = 0.0_f64;
    for &i in idxs {
        let w = obs[i].sqrt_weight * obs[i].sqrt_weight;
        let y = obs[i].point.y;
        if !w.is_finite() || w < 0.0 || !y.is_finite() || !obs[i].ss_within.is_finite() || obs[i].ss_within < 0.0 {
            return None;
        }
        sum_within += obs[i].ss_within;
        sum_w += w;
        sum_wy += w * y;
    }
    if idxs.len() <= 1 {
        return Some(sum_within);
    }
    if !sum_w.is_finite() || sum_w <= 0.0 || !sum_wy.is_finite() {
        return None;
    }
    let y_bar_w = sum_wy / sum_w;
    let mut ss_between = 0.0_f64;
    for &i in idxs {
        let w = obs[i].sqrt_weight * obs[i].sqrt_weight;
        let diff = obs[i].point.y - y_bar_w;
        ss_between += w * diff * diff;
    }
    let total = sum_within + ss_between;
    total.is_finite().then_some(total)
}

fn group_pure_error_deviance_poisson(obs: &[ObsRecord], idxs: &[usize]) -> Option<f64> {
    if idxs.len() <= 1 {
        return Some(0.0);
    }
    let mut sum_y = 0.0_f64;
    for &i in idxs {
        let y = obs[i].point.y;
        if !y.is_finite() || y < 0.0 {
            return None;
        }
        sum_y += y;
    }
    let y_bar = sum_y / (idxs.len() as f64);
    if !y_bar.is_finite() || y_bar <= 0.0 {
        return (sum_y == 0.0).then_some(0.0);
    }
    let mut acc = NeumaierSum::new();
    for &i in idxs {
        acc.add(2.0 * deviance_bd0(obs[i].point.y, y_bar));
    }
    let dev = acc.total();
    dev.is_finite().then_some(dev)
}

/// Observation-consistent replicates lack-of-fit test honoring replicate summary moments
/// (`sqrt(n)`, `(n-1)*sd^2`), WLS weights (`1/Y^2`, `1/SD^2`, etc.), and Poisson deviance.
fn replicates_lack_of_fit_obs(obs: &[ObsRecord], weighted_ss: f64, df: usize, method: FitMethod) -> Option<LackOfFitResult> {
    if obs.is_empty() || !weighted_ss.is_finite() || weighted_ss < 0.0 || df == 0 {
        return None;
    }
    let mut groups: std::collections::BTreeMap<(usize, Vec<u64>), Vec<usize>> = std::collections::BTreeMap::new();
    let mut n_total: usize = 0;
    for (idx, rec) in obs.iter().enumerate() {
        if rec.point.x.iter().any(|x| !x.is_finite()) {
            return None;
        }
        n_total = n_total.saturating_add(rec.rep_n);
        groups.entry((rec.ds_local_idx, canonical_x_bits(&rec.point.x))).or_default().push(idx);
    }
    let n_groups = groups.len();
    let df_pure_error = n_total.saturating_sub(n_groups);
    if df_pure_error == 0 {
        return None;
    }
    let df_lack_of_fit = df.saturating_sub(df_pure_error);
    if df_lack_of_fit == 0 {
        return None;
    }

    let mut pe_acc = NeumaierSum::new();
    for idxs in groups.values() {
        let g_pe =
            if method == FitMethod::PoissonDeviance { group_pure_error_deviance_poisson(obs, idxs)? } else { group_pure_error_ss_ls(obs, idxs)? };
        pe_acc.add(g_pe);
    }
    let ss_pure_error = pe_acc.total();
    if !ss_pure_error.is_finite() {
        return None;
    }
    let ss_lack_of_fit = (weighted_ss - ss_pure_error).max(0.0);
    let ms_pe = ss_pure_error / (df_pure_error as f64);
    let ms_lof = ss_lack_of_fit / (df_lack_of_fit as f64);
    if !ms_pe.is_finite() || !ms_lof.is_finite() || ms_pe <= 0.0 {
        return None;
    }
    let f_stat = ms_lof / ms_pe;
    let p_value = f_sf(f_stat, df_lack_of_fit as f64, df_pure_error as f64).ok()?;
    Some(LackOfFitResult { ss_pure_error, df_pure_error, ss_lack_of_fit, df_lack_of_fit, f_stat, p_value, significant_lack_of_fit: p_value < 0.05 })
}

/// ROUT outlier detection per Motulsky & Brown (2006, *BMC Bioinformatics* 7:123): computes RSDR
/// on the base-weighted residuals `r_{w,i} = (y_i - \hat{y}_i) * sqrt(w_i)` so heteroscedastic
/// schemes (such as `1/Y^2`) are standardized to uniform error scale, evaluates two-sided *t*-test
/// *p*-values vs. RSDR, and applies Benjamini–Hochberg step-up FDR **only** across the top 30% of
/// points furthest from the fitted curve (`start = floor(0.70 * N)`). Bottom ~70% residuals are
/// left untested (`q_value = 1.0`, `is_outlier = false`).
fn rout_detect_outliers_mb2006(
    points: &[DataPoint],
    weighted_residuals: &[f64],
    sqrt_weights: &[f64],
    k_free: usize,
    fdr_q_percent: f64,
) -> Result<RoutDetectionResult, FitError> {
    let n = points.len();
    if weighted_residuals.len() != n || sqrt_weights.len() != n {
        return Err(FitError::InvalidParameter {
            name: "weighted_residuals",
            reason: format!("expected {n} weighted residuals and weights, got {} / {}", weighted_residuals.len(), sqrt_weights.len()),
        });
    }
    if n <= k_free {
        return Err(FitError::InsufficientData { required: k_free + 1, actual: n });
    }
    if !fdr_q_percent.is_finite() || fdr_q_percent <= 0.0 || fdr_q_percent > 50.0 {
        return Err(FitError::InvalidParameter { name: "fdr_q_percent", reason: format!("FDR Q% must lie in (0, 50], got {fdr_q_percent}") });
    }

    let df = n - k_free;
    let abs_res: Vec<f64> = weighted_residuals.iter().map(|r| r.abs()).collect();
    let w_y_scale = points.iter().zip(sqrt_weights.iter()).map(|(p, &sw)| (p.y * sw).abs()).fold(0.0_f64, f64::max).max(f64::MIN_POSITIVE);
    let rsdr = compute_rsdr(&abs_res, k_free).max(1e-10 * w_y_scale);
    let zero_floor = (1e-30 * w_y_scale).max(f64::MIN_POSITIVE);
    let mut p_values = Vec::with_capacity(n);
    for &r in weighted_residuals {
        if !r.is_finite() {
            return Err(FitError::InvalidParameter { name: "weighted_residuals", reason: "ROUT residuals must all be finite".to_owned() });
        }
        let p = if rsdr <= zero_floor { if r.abs() <= zero_floor { 1.0 } else { 0.0 } } else { student_t_two_sided_p((r / rsdr).abs(), df as f64)? };
        p_values.push(p);
    }

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        p_values[a].total_cmp(&p_values[b]).then_with(|| weighted_residuals[b].abs().total_cmp(&weighted_residuals[a].abs())).then_with(|| a.cmp(&b))
    });

    // Motulsky & Brown (2006): test only top ~30% furthest points (ranks 1..=m_test out of N).
    let start = ((0.70 * (n as f64)).floor() as usize).min(n - 1);
    let m_test = n - start;
    let q_alpha = fdr_q_percent / 100.0;

    let mut q_values = vec![1.0_f64; n];
    let mut running_min = 1.0_f64;
    for j in (0..m_test).rev() {
        let rank = (j + 1) as f64;
        let raw_q = ((n as f64) / rank * p_values[order[j]]).min(1.0);
        running_min = running_min.min(raw_q);
        q_values[order[j]] = running_min;
    }

    let mut last_rejected: Option<usize> = None;
    for j in 0..m_test {
        let rank = (j + 1) as f64;
        let threshold = q_alpha * rank / (n as f64);
        if p_values[order[j]] <= threshold {
            last_rejected = Some(j);
        }
    }

    let mut is_outlier = vec![false; n];
    if let Some(j_max) = last_rejected {
        for &idx in &order[..=j_max] {
            is_outlier[idx] = true;
        }
    }

    let mut outlier_indices = Vec::new();
    let mut audit = Vec::with_capacity(n);
    for (idx, pt) in points.iter().enumerate() {
        if is_outlier[idx] {
            outlier_indices.push(idx);
        }
        audit.push(RoutPointAudit {
            index: idx,
            data_set_index: pt.data_set_index,
            row: pt.row,
            source_subcolumn: pt.source_subcolumn,
            x0: pt.x0(),
            y: pt.y,
            residual: weighted_residuals[idx],
            p_value: p_values[idx],
            q_value: q_values[idx],
            is_outlier: is_outlier[idx],
        });
    }

    Ok(RoutDetectionResult { rsdr, df, q_threshold_percent: fdr_q_percent, outlier_indices, audit })
}

struct ResidualDiagnosticsBundle {
    runs_test: Option<RunsTestResult>,
    lack_of_fit: Option<LackOfFitResult>,
    normality: Option<NormalityTestResult>,
    normality_anderson_darling: Option<NormalityTestResult>,
    normality_lilliefors_ks: Option<NormalityTestResult>,
    homoscedasticity: Option<HomoscedasticityResult>,
}

fn compute_residual_diagnostics(
    obs: &[ObsRecord],
    ds_results: &[FittedDatasetResult],
    y_hat_all: &[f64],
    raw_residuals_all: &[f64],
    weighted_ss: f64,
    df: usize,
    config: &SolverConfig,
) -> ResidualDiagnosticsBundle {
    let runs_res = if config.compute_runs_test {
        let per_ds_runs: Vec<RunsTestResult> = ds_results.iter().filter_map(|d| d.runs_test.clone()).collect();
        combine_runs_tests(&per_ds_runs)
    } else {
        None
    };

    let lof_res = if config.compute_lack_of_fit && config.method != FitMethod::RobustCauchy {
        replicates_lack_of_fit_obs(obs, weighted_ss, df, config.method)
    } else {
        None
    };

    let diag_residuals: Vec<f64> = if config.method == FitMethod::PoissonDeviance {
        obs.iter()
            .zip(y_hat_all.iter())
            .zip(raw_residuals_all.iter())
            .map(|((o, &yh), &r)| if yh.is_finite() && yh > 0.0 { r / yh.sqrt() } else { r * o.sqrt_weight })
            .collect()
    } else {
        obs.iter().zip(raw_residuals_all.iter()).map(|(o, &r)| r * o.sqrt_weight).collect()
    };

    let (norm_main, norm_ad, norm_ks) = if config.compute_normality {
        (
            shapiro_wilk_test(&diag_residuals).or_else(|| dagostino_pearson_test(&diag_residuals)),
            anderson_darling_test(&diag_residuals),
            lilliefors_ks_test(&diag_residuals),
        )
    } else {
        (None, None, None)
    };

    let homo_res =
        if config.compute_homoscedasticity { homoscedasticity_spearman_test(y_hat_all, &diag_residuals, config.confidence.alpha()) } else { None };

    ResidualDiagnosticsBundle {
        runs_test: runs_res,
        lack_of_fit: lof_res,
        normality: norm_main,
        normality_anderson_darling: norm_ad,
        normality_lilliefors_ks: norm_ks,
        homoscedasticity: homo_res,
    }
}

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

fn fit_plot_function_only(
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

    let mut ds_results = Vec::with_capacity(datasets.len());
    let mut derived_scalars = Vec::with_capacity(datasets.len());
    let mut total_obs = 0_usize;
    let mut raw_rss_acc = NeumaierSum::new();
    let mut y_vals_all = Vec::new();

    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        let ds_params = layout.expand_params(&theta, ds_local_idx);
        derived_scalars.push(model.derived_transforms(&ds_params));
        let mut ds_yh = Vec::with_capacity(ds.points.len());
        let mut ds_res = Vec::with_capacity(ds.points.len());
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
                ds_rss.add(r * r);
                raw_rss_acc.add(r * r);
                y_vals_all.push(pt.y);
                total_obs += 1;
            }
        }
        let ds_ss = ds_rss.total();
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
            r_squared: 1.0,
            sy_x: if is_robust { f64::NAN } else { 0.0 },
            rsdr: None,
            runs_test: None,
            curve_overlay: Some(overlay),
        });
    }

    let raw_rss = raw_rss_acc.total();
    let df = total_obs.saturating_sub(k_free);
    let rsd = if df > 0 && !is_robust { (raw_rss / (df as f64)).sqrt() } else { f64::NAN };
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
fn solve_lm_by_method(
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

struct FinalizedUncertainty {
    cov: CovarianceDiagnostics,
    identifiability: Assessment,
    cov_unscaled: Option<DenseMatrix>,
    hougaard: Vec<f64>,
    profile_cis: Vec<(CiBound, CiBound)>,
    t_crit: f64,
    sigma_w: f64,
}

fn finalize_fit_uncertainty(
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

    let identifiability = if config.identify_unstable_parameters && k_free > 0 && lm_out.status != FitStatus::Diverged {
        identifiability::assess(layout, obs, &lm_out.j_mat, df, &cov.dependencies, profile_computed.then_some(profile_cis.as_slice()))?
    } else {
        Assessment::default()
    };
    Ok(FinalizedUncertainty { cov, identifiability, cov_unscaled, hougaard, profile_cis, t_crit, sigma_w })
}

fn fit_single_or_shared(
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
    let res_diag = compute_residual_diagnostics(&obs, &assembled.ds_results, &lm_out.y_hat, &raw_residuals_all, lm_out.weighted_ss, df, config);

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

struct SingleDatasetOutcome {
    layout: ParameterLayout,
    theta: Vec<f64>,
    status: FitStatus,
    iterations: usize,
    weighted_ss: f64,
    cov: CovarianceDiagnostics,
    hougaard: Vec<f64>,
    profile_cis: Vec<(CiBound, CiBound)>,
    identifiability: Assessment,
    ds_result: FittedDatasetResult,
    derived_scalars: Vec<DerivedScalar>,
    interpolated_x: Vec<InterpolatedUnknown>,
    interpolated_y: Vec<InterpolatedUnknown>,
    obs_moments: Vec<(f64, usize, f64)>,
    poisson_ll: Option<f64>,
}

fn solve_single_dataset_unit(
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

fn fit_independent_datasets(
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

fn build_fitted_parameter_summaries(
    layout: &ParameterLayout,
    theta: &[f64],
    cov: &CovarianceDiagnostics,
    hougaard: &[f64],
    profile_cis: &[(CiBound, CiBound)],
    unstable: &[bool],
    inference_availability: InferenceAvailability,
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
        if unstable.get(idx).copied().unwrap_or(false) {
            notes.push(ParameterNote::Unstable);
        } else if !is_robust && cov.dependencies.get(idx).copied().unwrap_or(0.0) > 0.9999 {
            notes.push(ParameterNote::Ambiguous);
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

struct DatasetAssemblyConfig<'a> {
    cov_unscaled: Option<&'a DenseMatrix>,
    sigma_w: f64,
    t_crit: f64,
    rsd: f64,
    ambiguous: bool,
    solver: &'a SolverConfig,
}

struct AssembledDatasets {
    ds_results: Vec<FittedDatasetResult>,
    derived_scalars: Vec<Vec<DerivedScalar>>,
    interpolated_x: Vec<InterpolatedUnknown>,
    interpolated_y: Vec<InterpolatedUnknown>,
}

fn assemble_dataset_results(
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
                ds_rss.add(rec.ss_within);
                ds_n += rec.rep_n;
                ds_y_sum.add(w_rep * rec.point.y);
                ds_y_vals.push((rec.point.y, rec.rep_n, rec.ss_within));
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

fn comparison_test_label(spec: &ModelComparisonSpec) -> &'static str {
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
/// preserving strict `#114` validation in [`ParameterLayout::build`] while allowing
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

/// Validate strictly positive Poisson fitted means `mu_i > 0` and assign IRLS weights `1 / sqrt(mu_i)`.
fn assign_poisson_irls_weights(work_obs: &mut [ObsRecord], y_hat: &[f64]) -> Result<(), FitError> {
    if work_obs.len() != y_hat.len() || y_hat.is_empty() {
        return Err(FitError::ImpossibleWeights {
            reason: "Poisson deviance regression requires non-empty strictly positive fitted means".to_owned(),
        });
    }
    for (rec, &yh) in work_obs.iter_mut().zip(y_hat.iter()) {
        if !yh.is_finite() || yh <= 0.0 {
            return Err(FitError::ImpossibleWeights {
                reason: format!("Poisson deviance regression requires strictly positive fitted means (mu > 0, got {yh})"),
            });
        }
        let inv_sqrt = 1.0 / yh.sqrt();
        if !inv_sqrt.is_finite() {
            return Err(FitError::ImpossibleWeights { reason: "Poisson IRLS weight 1 / sqrt(mu) overflowed f64 range".to_owned() });
        }
        rec.sqrt_weight = inv_sqrt;
    }
    Ok(())
}

/// Poisson maximum-likelihood regression by iteratively reweighted least squares
/// (Green 1984; McCullagh & Nelder 1989): weights `w_i = 1 / mu_i` are frozen at the current
/// fitted means and the weighted least-squares problem is re-solved by Levenberg–Marquardt
/// until the deviance stabilises. A fixed point satisfies the Poisson score equations
/// `sum (y_i - mu_i) / mu_i * d mu_i / d theta = 0`. The reported `weighted_ss` is the deviance.
fn fit_poisson_irls(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    init_theta: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    if config.weighting != WeightingScheme::None || obs.iter().any(|rec| rec.point.explicit_weight.is_some()) {
        return Err(FitError::InvalidParameter {
            name: "weighting",
            reason: "Poisson deviance regression defines variance Var(Y) = mu internally and cannot be combined with least-squares weighting schemes or explicit observation weights".to_owned(),
        });
    }
    for rec in obs {
        if rec.point.y < 0.0 || !rec.point.y.is_finite() {
            return Err(FitError::ImpossibleWeights { reason: "Poisson deviance regression requires non-negative counts (Y >= 0)".to_owned() });
        }
    }
    let mut theta = init_theta.to_vec();
    let mut work_obs = obs.to_vec();
    let mut last_out = if eval_residuals_with_domain(model, layout, &work_obs, &theta, true).is_some() {
        solve_with_warm_start_with_domain(model, layout, &work_obs, &theta, config, true)?
    } else if layout.k_free() == 0 {
        return Err(FitError::ImpossibleWeights {
            reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
        });
    } else if !config.use_simplex_warm_start {
        if eval_residuals_with_domain(model, layout, &work_obs, &theta, false).is_some() {
            return Err(FitError::BadInitialValues {
                reason: "Starting parameters produce non-positive Poisson predicted means (mu <= 0)".to_owned(),
            });
        }
        return Err(FitError::BadInitialValues { reason: "Starting parameters evaluate outside model domain".to_owned() });
    } else {
        // When initial parameters give mu <= 0, rescue via unconstrained LS pre-fit and/or a
        // feasibility-barrier Nelder-Mead search inside parameter bounds before rejecting as impossible.
        let mut ls_cand = None;
        if eval_residuals_with_domain(model, layout, &work_obs, &theta, false).is_some() {
            match solve_with_warm_start_with_domain(model, layout, &work_obs, &theta, config, false) {
                Ok(out) => ls_cand = Some(out),
                Err(err @ FitError::OdeIntegrationFailed { .. }) => return Err(err),
                Err(_) => {}
            }
        }
        let ls_outcome = ls_cand
            .map(|out| if eval_residuals_with_domain(model, layout, &work_obs, &out.theta, true).is_some() { Ok(out) } else { Err(out.theta) });
        match ls_outcome {
            Some(Ok(feasible_ls)) => feasible_ls,
            other => {
                let ls_seed = other.as_ref().and_then(|res| res.as_ref().err().map(Vec::as_slice));
                let feasible = nelder_mead_poisson_feasibility_rescue(model, layout, &work_obs, &theta)
                    .or_else(|| ls_seed.and_then(|s| nelder_mead_poisson_feasibility_rescue(model, layout, &work_obs, s)));
                if let Some(start_pos) = feasible {
                    solve_with_warm_start_with_domain(model, layout, &work_obs, &start_pos, config, true)?
                } else if let Some(err) = ode_budget_error() {
                    return Err(err);
                } else {
                    return Err(FitError::ImpossibleWeights {
                        reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
                    });
                }
            }
        }
    };
    let mut prev_dev = poisson_deviance(&work_obs, &last_out.y_hat).ok_or_else(|| FitError::ImpossibleWeights {
        reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
    })?;

    for _ in 0..12 {
        theta = last_out.theta.clone();
        assign_poisson_irls_weights(&mut work_obs, &last_out.y_hat)?;
        let next = run_trust_region_lm_with_domain(model, layout, &work_obs, &theta, config.max_iters, 10.0, true)?;
        let dev = poisson_deviance(&work_obs, &next.y_hat).ok_or_else(|| FitError::ImpossibleWeights {
            reason: "Poisson deviance regression encountered non-positive fitted means (mu <= 0)".to_owned(),
        })?;
        let rel_change = (prev_dev - dev).abs() / prev_dev.max(1.0);
        last_out = next;
        prev_dev = dev;
        if rel_change < 1e-10 {
            break;
        }
    }
    assign_poisson_irls_weights(&mut work_obs, &last_out.y_hat)?;
    if let (Some((wr, _, _)), Some(jm)) = (
        eval_residuals_with_domain(model, layout, &work_obs, &last_out.theta, true),
        compute_jacobian(model, layout, &work_obs, &last_out.theta, &last_out.y_hat, &layout.bounds()),
    ) {
        last_out.weighted_res = wr;
        last_out.j_mat = jm;
    }
    last_out.weighted_ss = poisson_deviance(&work_obs, &last_out.y_hat).ok_or_else(|| FitError::ImpossibleWeights {
        reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
    })?;
    Ok(last_out)
}

/// Poisson deviance `D = 2 sum [ y ln(y / mu) - (y - mu) ]` (McCullagh & Nelder 1989), evaluated
/// stably via `deviance_bd0`. Returns `None` if any observed count is negative/non-finite or any
/// fitted mean `mu <= 0`.
fn poisson_deviance(obs: &[ObsRecord], y_hat: &[f64]) -> Option<f64> {
    if obs.len() != y_hat.len() || obs.is_empty() {
        return None;
    }
    let mut acc = NeumaierSum::new();
    for (rec, &mu) in obs.iter().zip(y_hat.iter()) {
        if !rec.point.y.is_finite() || rec.point.y < 0.0 || !mu.is_finite() || mu <= 0.0 {
            return None;
        }
        acc.add(2.0 * deviance_bd0(rec.point.y, mu));
    }
    let dev = acc.total();
    dev.is_finite().then_some(dev)
}

/// Robust Cauchy (Lorentzian) fit minimising `sum ln(1 + (r_{w,i} / RSDR)^2)`, the merit function
/// of Motulsky & Brown (2006, *BMC Bioinformatics* 7:123), with `RSDR` re-estimated from the
/// base-weighted residuals `r_{w,i} = (y_i - \hat{y}_i) * sqrt(w_i)` at every iteration so
/// heteroscedastic weighting schemes (`1/Y^2`, `1/|Y|`, etc.) are standardized consistently.
///
/// Deviation: Motulsky & Brown minimise the Lorentzian merit with a modified Marquardt
/// algorithm; this routine instead uses IRLS with Cauchy weights `w_i = 1 / (1 + z_i^2)`,
/// `z_i = |r_{w,i}| / RSDR` (Holland & Welsch 1977), each weighted subproblem solved by
/// Levenberg–Marquardt. For a fixed scale both reach the same stationary point. Standard
/// errors from this fit are not robust-theory SEs and are not used by ROUT.
fn cauchy_marginal_nll(obs: &[ObsRecord], y_hat: &[f64], k_free: usize, w_y_scale: f64) -> f64 {
    let abs_res: Vec<f64> = obs.iter().zip(y_hat.iter()).map(|(o, yh)| ((o.point.y - yh) * o.sqrt_weight).abs()).collect();
    let s = compute_rsdr(&abs_res, k_free).max(1e-6 * w_y_scale);
    let penalty: f64 = abs_res
        .iter()
        .map(|&r| {
            let z = r / s;
            (1.0 + z * z).ln()
        })
        .sum();
    (obs.len() as f64) * s.ln() + penalty
}

fn run_cauchy_irls_loop(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    mut last_out: LmRunOutput,
    config: &SolverConfig,
    w_y_scale: f64,
) -> Result<LmRunOutput, FitError> {
    let k_free = layout.k_free();
    let mut work_obs = obs.to_vec();
    for _ in 0..15 {
        let prev_theta = last_out.theta.clone();
        let abs_res: Vec<f64> = obs.iter().zip(last_out.y_hat.iter()).map(|(o, yh)| ((o.point.y - yh) * o.sqrt_weight).abs()).collect();
        let rsdr = compute_rsdr(&abs_res, k_free).max(1e-12 * w_y_scale);
        for (i, rec) in work_obs.iter_mut().enumerate() {
            let z = abs_res[i] / rsdr;
            let cauchy_w = 1.0 / (1.0 + z * z);
            rec.sqrt_weight = obs[i].sqrt_weight * cauchy_w.sqrt();
        }
        last_out = run_trust_region_lm(model, layout, &work_obs, &prev_theta, config.max_iters, 10.0)?;
        let max_step = prev_theta
            .iter()
            .zip(last_out.theta.iter())
            .map(|(a, b)| (a - b).abs() / b.abs().max(a.abs()).max(f64::MIN_POSITIVE))
            .fold(0.0_f64, f64::max);
        if max_step < 1e-8 {
            break;
        }
    }
    Ok(last_out)
}

fn fit_cauchy_irls(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    init_theta: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    let k_free = layout.k_free();
    let w_y_scale = obs.iter().map(|o| (o.point.y * o.sqrt_weight).abs()).fold(0.0_f64, f64::max).max(f64::MIN_POSITIVE);
    let ols_seed = solve_with_warm_start(model, layout, obs, init_theta, config)?;
    let mut best = run_cauchy_irls_loop(model, layout, obs, ols_seed, config, w_y_scale)?;

    // If L2 OLS warm start chased an outlier into an unrepresentative steep step, also launch
    // Cauchy IRLS directly from a Lorentzian-merit simplex search seeded at `init_theta`.
    if k_free > 0 && config.use_simplex_warm_start {
        let bounds = layout.bounds();
        let nm_seed = nelder_mead_optimize(layout, init_theta, 500, 0.05, |th| {
            if ode_fit_budget_exceeded().is_some() {
                return f64::INFINITY;
            }
            let mut clamped = th.to_vec();
            project_theta(&mut clamped, &bounds);
            eval_residuals_with_domain(model, layout, obs, &clamped, false)
                .map_or(f64::INFINITY, |(_, y_hat, _)| cauchy_marginal_nll(obs, &y_hat, k_free, w_y_scale))
        });
        if let Some(seed_th) = nm_seed
            && let Some((w_res, y_hat, ss)) = eval_residuals_with_domain(model, layout, obs, &seed_th, false)
            && let Ok(j_mat) = DenseMatrix::zeros(obs.len(), k_free)
        {
            let seed_out =
                LmRunOutput { theta: seed_th, weighted_res: w_res, y_hat, weighted_ss: ss, j_mat, status: FitStatus::Converged, iterations: 0 };
            if let Ok(alt) = run_cauchy_irls_loop(model, layout, obs, seed_out, config, w_y_scale) {
                let best_nll = cauchy_marginal_nll(obs, &best.y_hat, k_free, w_y_scale);
                let alt_nll = cauchy_marginal_nll(obs, &alt.y_hat, k_free, w_y_scale);
                if alt_nll + 1e-6 < best_nll {
                    best = alt;
                }
            }
        }
    }
    Ok(best)
}

fn compute_unscaled_cov(j_mat: &DenseMatrix) -> Option<DenseMatrix> {
    let p = j_mat.ncols();
    let nrows = j_mat.nrows();
    if p == 0 || nrows < p {
        return None;
    }
    let mut col_norms = vec![0.0_f64; p];
    let mut j_scaled = DenseMatrix::zeros(nrows, p).ok()?;
    for (c, cn_slot) in col_norms.iter_mut().enumerate().take(p) {
        let cn = j_mat.col_norm2(c);
        if !cn.is_finite() || cn <= 0.0 {
            return None;
        }
        *cn_slot = cn;
        let inv_cn = 1.0 / cn;
        for r in 0..nrows {
            if let Some(slot) = j_scaled.get_mut(r, c) {
                *slot = j_mat.get(r, c).unwrap_or(0.0) * inv_cn;
            }
        }
    }
    let pqr = qr_col_pivoted(&j_scaled).ok()?;
    if pqr.rank() < p {
        return None;
    }
    let inv_scaled = pqr.xtx_inverse().ok()?;
    let mut inv = DenseMatrix::zeros(p, p).ok()?;
    for i in 0..p {
        let di = col_norms[i];
        for j in 0..p {
            let dj = col_norms[j];
            if let Some(slot) = inv.get_mut(i, j) {
                *slot = (inv_scaled.get(i, j).unwrap_or(0.0) / di) / dj;
            }
        }
    }
    Some(inv)
}

struct ReducedLmState {
    y_hat: Vec<f64>,
    ss_curr: f64,
    converged: bool,
}

#[allow(clippy::too_many_arguments)]
fn run_reduced_lm_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    work: &mut [f64],
    map_indices: &[usize],
    full_bounds: &[(Option<f64>, Option<f64>)],
    max_iters: usize,
    require_positive_mean: bool,
) -> Option<ReducedLmState> {
    let sub_k = map_indices.len();
    let sub_bounds: Vec<(Option<f64>, Option<f64>)> = map_indices.iter().map(|&i| full_bounds[i]).collect();
    let mut sub_theta: Vec<f64> = map_indices.iter().map(|&i| work[i]).collect();
    project_theta(&mut sub_theta, &sub_bounds);

    let ftol = 1e-12_f64;
    let xtol = 1e-12_f64;
    let gtol = 1e-11_f64;
    let stall_gtol = 1e-4_f64;
    let y_norm = obs_y_ss(obs).sqrt();

    let mut diag = vec![0.0_f64; sub_k];
    let mut delta = 0.0_f64;
    let mut iter = 0_usize;
    let mut last_max_g_cos = f64::INFINITY;
    let mut lm_ws = LmDampWorkspace::new(obs.len(), sub_k).ok()?;
    for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
        work[orig] = v;
    }
    let (mut w_res, mut y_hat, mut ss_curr) = eval_residuals_with_domain(model, layout, obs, work, require_positive_mean)?;

    while iter < max_iters {
        for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
            work[orig] = v;
        }
        let j_full = compute_jacobian(model, layout, obs, work, &y_hat, full_bounds)?;
        let mut j_sub = DenseMatrix::zeros(obs.len(), sub_k).ok()?;
        for (sub_j, &orig_j) in map_indices.iter().enumerate() {
            for i in 0..obs.len() {
                mat_set(&mut j_sub, i, sub_j, mat_get(&j_full, i, orig_j));
            }
        }
        if ss_curr == 0.0 {
            return Some(ReducedLmState { y_hat, ss_curr, converged: true });
        }
        let max_g_cos = update_lm_diag_and_projected_grad(&j_sub, &w_res, &sub_theta, &sub_bounds, ss_curr, &mut diag);
        last_max_g_cos = max_g_cos;
        if max_g_cos <= gtol {
            return Some(ReducedLmState { y_hat, ss_curr, converged: true });
        }
        if iter == 0 {
            let theta_scale = scaled_l2_norm(&diag, &sub_theta);
            delta = 10.0 * if theta_scale > 0.0 { theta_scale } else { 1.0 };
        }

        let mut accepted = false;
        let mut lam = 0.0_f64;
        for _ in 0..25 {
            let Ok((step, lam_u)) = solve_trust_region_subproblem(&j_sub, &w_res, &diag, &sub_theta, delta, lam, &mut lm_ws) else {
                break;
            };
            lam = lam_u;
            let ctx = LmStepContext { j_mat: &j_sub, w_res: &w_res, diag: &diag, theta: &sub_theta };
            let Ok((trial, actual_step, p_norm)) = project_tr_trial(&ctx, &step, &sub_bounds, delta, &mut lam) else {
                break;
            };
            if iter == 0 && p_norm > 0.0 {
                delta = delta.min(p_norm);
            }

            let pred_num = quadratic_pred_num(&j_sub, &w_res, &actual_step).max(0.0);
            let pred_red = if ss_curr > 0.0 { pred_num / ss_curr } else { 0.0 };

            for (&orig, &v) in map_indices.iter().zip(trial.iter()) {
                work[orig] = v;
            }
            let trial_eval = eval_residuals_with_domain(model, layout, obs, work, require_positive_mean);
            let act_red = match &trial_eval {
                Some((_, _, ss_t)) if *ss_t < ss_curr => 1.0 - *ss_t / ss_curr,
                _ => -1.0,
            };
            let rho = if pred_red > 0.0 && trial_eval.is_some() {
                act_red / pred_red
            } else if act_red > 0.0 {
                1.0
            } else {
                -1.0
            };

            let ss_trial_val = trial_eval.as_ref().map(|(_, _, ss_t)| *ss_t);
            let outcome = TrStepOutcome { actual_step: &actual_step, p_norm, ss_curr, ss_trial: ss_trial_val, rho };
            update_tr_radius_more(&ctx, &outcome, &mut delta, &mut lam);

            if rho >= 1e-4
                && let Some((r_t, y_t, ss_t)) = trial_eval
            {
                sub_theta = trial;
                w_res = r_t;
                y_hat = y_t;
                ss_curr = ss_t;
                iter += 1;
                accepted = true;
                let raw_xnorm = scaled_l2_norm(&diag, &sub_theta);
                let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
                if (act_red.abs() <= ftol && pred_red <= ftol && rho <= 2.0) || p_norm <= xtol * x_norm {
                    for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
                        work[orig] = v;
                    }
                    return Some(ReducedLmState { y_hat, ss_curr, converged: true });
                }
                break;
            }

            let raw_xnorm = scaled_l2_norm(&diag, &sub_theta);
            let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
            if p_norm <= xtol * x_norm || delta <= 1e-15 * x_norm {
                for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
                    work[orig] = v;
                }
                let converged = stalled_lm_status(max_g_cos, ss_curr, y_norm, stall_gtol) == FitStatus::Converged;
                return Some(ReducedLmState { y_hat, ss_curr, converged });
            }
        }
        if !accepted {
            break;
        }
    }
    for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
        work[orig] = v;
    }
    let converged = iter < max_iters && stalled_lm_status(last_max_g_cos, ss_curr, y_norm, stall_gtol) == FitStatus::Converged;
    Some(ReducedLmState { y_hat, ss_curr, converged })
}

fn check_profile_fixed_param(layout: &ParameterLayout, fixed_idx: usize, fixed_val: f64) -> Option<Vec<(Option<f64>, Option<f64>)>> {
    let k = layout.k_free();
    if k == 0 || fixed_idx >= k || !fixed_val.is_finite() {
        return None;
    }
    let full_bounds = layout.bounds();
    if let Some(lb) = full_bounds[fixed_idx].0
        && fixed_val < lb
    {
        return None;
    }
    if let Some(ub) = full_bounds[fixed_idx].1
        && fixed_val > ub
    {
        return None;
    }
    Some(full_bounds)
}

/// Optimize the remaining `K - 1` nuisance parameters with `theta[fixed_idx] = fixed_val` held constant.
///
/// Returns the profile sum of squares `S~(theta_k)` of Bates & Watts (1988) along with
/// reduced-LM convergence status via [`run_reduced_lm_with_domain`] (Moré 1978 trust-region
/// gain-ratio step acceptance and `ftol`/`xtol`/`gtol` stopping).
fn solve_profile_subproblem(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
    fixed_idx: usize,
    fixed_val: f64,
    ss_within: f64,
) -> Option<ProfileStep> {
    let full_bounds = check_profile_fixed_param(layout, fixed_idx, fixed_val)?;
    let k = layout.k_free();
    let mut work = seed.to_vec();
    work[fixed_idx] = fixed_val;
    if k == 1 {
        let (_, _, ss) = eval_residuals(model, layout, obs, &work)?;
        return Some(ProfileStep { objective: ss + ss_within, theta: work, converged: true });
    }

    let map_indices: Vec<usize> = (0..k).filter(|&i| i != fixed_idx).collect();
    let state = run_reduced_lm_with_domain(model, layout, obs, &mut work, &map_indices, &full_bounds, 200, false)?;
    Some(ProfileStep { objective: state.ss_curr + ss_within, theta: work, converged: state.converged })
}

/// Optimize the remaining `K - 1` nuisance parameters under Poisson deviance with `theta[fixed_idx] = fixed_val`
/// held constant via reduced IRLS + deviance monotonicity guard.
fn solve_profile_subproblem_poisson(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
    fixed_idx: usize,
    fixed_val: f64,
) -> Option<ProfileStep> {
    let full_bounds = check_profile_fixed_param(layout, fixed_idx, fixed_val)?;
    let k = layout.k_free();
    let mut work = seed.to_vec();
    work[fixed_idx] = fixed_val;
    let (_, init_y_hat, _) = eval_residuals_with_domain(model, layout, obs, &work, true)?;
    if k == 1 {
        let dev = poisson_deviance(obs, &init_y_hat)?;
        return Some(ProfileStep { objective: dev, theta: work, converged: true });
    }

    let map_indices: Vec<usize> = (0..k).filter(|&i| i != fixed_idx).collect();
    let mut work_obs = obs.to_vec();
    let mut curr_y_hat = init_y_hat;
    let mut curr_dev = poisson_deviance(&work_obs, &curr_y_hat)?;
    let mut irls_converged = false;

    for _ in 0..20 {
        if assign_poisson_irls_weights(&mut work_obs, &curr_y_hat).is_err() {
            return None;
        }
        let mut trial = work.clone();
        let Some(state) = run_reduced_lm_with_domain(model, layout, &work_obs, &mut trial, &map_indices, &full_bounds, 80, true) else {
            irls_converged = false;
            break;
        };
        if let Some(next_dev) = poisson_deviance(&work_obs, &state.y_hat)
            && next_dev <= curr_dev
        {
            let rel_change = (curr_dev - next_dev).abs() / curr_dev.max(1.0);
            work = trial;
            curr_y_hat = state.y_hat;
            curr_dev = next_dev;
            if rel_change < 1e-10 && state.converged {
                irls_converged = true;
                break;
            }
            continue;
        }
        // Step-halving line search on IRLS step if linearization briefly overshot deviance.
        let mut improved = false;
        let mut alpha = 0.5_f64;
        for _ in 0..5 {
            let mut blended = work.clone();
            for &idx in &map_indices {
                blended[idx] = work[idx] + alpha * (trial[idx] - work[idx]);
            }
            if let Some((_, blend_yh, _)) = eval_residuals_with_domain(model, layout, &work_obs, &blended, true)
                && let Some(blend_dev) = poisson_deviance(&work_obs, &blend_yh)
                && blend_dev < curr_dev
            {
                let rel_change = (curr_dev - blend_dev).abs() / curr_dev.max(1.0);
                work = blended;
                curr_y_hat = blend_yh;
                curr_dev = blend_dev;
                improved = true;
                if rel_change < 1e-10 && state.converged {
                    irls_converged = true;
                }
                break;
            }
            alpha *= 0.5;
        }
        if irls_converged || !improved {
            if !improved && state.converged {
                irls_converged = true;
            }
            break;
        }
    }
    poisson_deviance(&work_obs, &curr_y_hat).map(|dev| ProfileStep { objective: dev, theta: work, converged: irls_converged })
}

struct CurveOverlayContext<'a> {
    model: &'a dyn ParametricModel,
    layout: &'a ParameterLayout,
    theta: &'a [f64],
    ds_params: &'a [f64],
    ds: &'a DatasetSeries,
    ds_local_idx: usize,
    cov_unscaled: Option<&'a DenseMatrix>,
    sigma: f64,
    t_crit: f64,
    converged: bool,
    ambiguous: bool,
    solver: &'a SolverConfig,
}

/// Variance inflation factor `1 / w_new(x, y_hat)` for a single future observation under `scheme`.
fn new_observation_inv_weight(scheme: WeightingScheme, x: f64, y_hat: f64) -> f64 {
    match scheme {
        WeightingScheme::None | WeightingScheme::ObservedSd => 1.0,
        WeightingScheme::RelativeSquare => {
            if y_hat.is_finite() {
                y_hat * y_hat
            } else {
                1.0
            }
        }
        WeightingScheme::Relative => {
            if y_hat.is_finite() {
                y_hat.abs()
            } else {
                1.0
            }
        }
        WeightingScheme::GeneralPowerY(k) => {
            let inv = y_hat.abs().powf(k);
            if inv.is_finite() && inv >= 0.0 { inv } else { 1.0 }
        }
        WeightingScheme::InverseX => {
            if x.is_finite() {
                x.abs()
            } else {
                1.0
            }
        }
        WeightingScheme::InverseX2 => {
            if x.is_finite() {
                x * x
            } else {
                1.0
            }
        }
    }
}

/// Sample the fitted curve with pointwise delta-method bands (Bates & Watts 1988;
/// Seber & Wild 1989): with `g = d f(x) / d theta` and `C = (J^T J)^{-1}`,
/// confidence band `y +/- t_{df} s sqrt(g^T C g)` and prediction band
/// `y +/- t_{df} s sqrt(1 + g^T C g)`. These are pointwise bands, not simultaneous
/// (Scheffé-type) bands over the whole curve.
fn build_curve_overlay(ctx: &CurveOverlayContext<'_>) -> FittedCurveOverlay {
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

    for i in 0..n_grid {
        let frac = (i as f64) / ((n_grid - 1) as f64);
        let x = if use_log { (log_min + frac * log_span).exp().clamp(x_min, x_max) } else { x_min + frac * (x_max - x_min) };
        let Some(y) = ctx.model.eval(&[x], ctx.ds_params, ctx.ds.data_set_index) else {
            continue;
        };
        if !y.is_finite() {
            continue;
        }
        let (band_lo, band_hi, pred_lo, pred_hi) = if let Some(cov_u) = active_cov {
            grad_buf.fill(0.0);
            g.fill(0.0);
            if ctx.model.grad(&[x], ctx.ds_params, ctx.ds.data_set_index, &mut grad_buf) {
                for (p_idx, &gv) in grad_buf.iter().enumerate().take(p_m) {
                    if let ParamBinding::Free(slot) = ctx.layout.bindings[ctx.ds_local_idx][p_idx] {
                        g[slot] += gv;
                    }
                }
            } else {
                delta_grad_at_x_into(
                    ctx.model,
                    ctx.layout,
                    ctx.theta,
                    ctx.ds_local_idx,
                    ctx.ds.data_set_index,
                    x,
                    &mut work_theta,
                    &mut work_params,
                    &mut g,
                );
            }
            let quad = quadratic_form(cov_u, &g).max(0.0);
            let se_f = ctx.sigma * quad.sqrt();
            let b_lo = Some(y - ctx.t_crit * se_f);
            let b_hi = Some(y + ctx.t_crit * se_f);
            let (p_lo, p_hi) = if ctx.solver.compute_prediction_bands {
                let inv_w = new_observation_inv_weight(ctx.solver.weighting, x, y);
                let se_p = if ctx.solver.method == FitMethod::PoissonDeviance {
                    (y.max(0.0) + ctx.sigma * ctx.sigma * quad).sqrt()
                } else {
                    ctx.sigma * (inv_w + quad).sqrt()
                };
                (Some(y - ctx.t_crit * se_p), Some(y + ctx.t_crit * se_p))
            } else {
                (None, None)
            };
            (b_lo, b_hi, p_lo, p_hi)
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
fn delta_grad_at_x_into(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_local_idx: usize,
    ds_data_idx: usize,
    x: f64,
    work: &mut [f64],
    work_params: &mut [f64],
    g_out: &mut [f64],
) {
    let k = layout.k_free();
    g_out.fill(0.0);
    work.copy_from_slice(theta);
    for slot in 0..k {
        let info = &layout.free_slots[slot];
        if let Some(local_d) = info.ds_local_idx
            && local_d != ds_local_idx
        {
            continue;
        }
        let h = 1e-6 * theta[slot].abs().max(1.0);
        work[slot] = theta[slot] + h;
        layout.expand_params_into(work, ds_local_idx, work_params);
        let fp = model.eval(&[x], work_params, ds_data_idx);
        work[slot] = theta[slot] - h;
        layout.expand_params_into(work, ds_local_idx, work_params);
        let fm = model.eval(&[x], work_params, ds_data_idx);
        work[slot] = theta[slot];
        if let (Some(p), Some(m)) = (fp, fm) {
            g_out[slot] = (p - m) / (2.0 * h);
        }
    }
}

fn delta_grad_at_x(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_params: &[f64],
    ds_local_idx: usize,
    ds: &DatasetSeries,
    x: f64,
) -> Vec<f64> {
    let k = layout.k_free();
    let p_m = layout.p_model;
    let mut g = vec![0.0_f64; k];
    let mut grad_buf = vec![0.0_f64; p_m];
    if model.grad(&[x], ds_params, ds.data_set_index, &mut grad_buf) {
        for (p_idx, &gv) in grad_buf.iter().enumerate().take(p_m) {
            if let ParamBinding::Free(slot) = layout.bindings[ds_local_idx][p_idx] {
                g[slot] += gv;
            }
        }
        return g;
    }
    let mut work = theta.to_vec();
    let mut work_params = vec![0.0_f64; p_m];
    delta_grad_at_x_into(model, layout, theta, ds_local_idx, ds.data_set_index, x, &mut work, &mut work_params, &mut g);
    g
}

fn quadratic_form(m: &DenseMatrix, v: &[f64]) -> f64 {
    let n = v.len().min(m.nrows()).min(m.ncols());
    let mut acc = NeumaierSum::new();
    for i in 0..n {
        for j in 0..n {
            acc.add(v[i] * mat_get(m, i, j) * v[j]);
        }
    }
    acc.total()
}

struct DatasetXBounds {
    real_bounds: Option<(f64, f64)>,
    single_x: Option<f64>,
    scan_x_min: f64,
    scan_x_max: f64,
    span: f64,
}

fn dataset_x_bounds(ds: &DatasetSeries) -> DatasetXBounds {
    let mut d_min = f64::INFINITY;
    let mut d_max = f64::NEG_INFINITY;
    for pt in &ds.points {
        let xv = pt.x0();
        if xv.is_finite() {
            d_min = d_min.min(xv);
            d_max = d_max.max(xv);
        }
    }
    if d_min.is_finite() && d_max.is_finite() && (d_max - d_min).abs() >= 1e-12 {
        DatasetXBounds { real_bounds: Some((d_min, d_max)), single_x: None, scan_x_min: d_min, scan_x_max: d_max, span: (d_max - d_min).max(1.0) }
    } else if d_min.is_finite() {
        DatasetXBounds { real_bounds: None, single_x: Some(d_min), scan_x_min: -10.0, scan_x_max: 10.0, span: 20.0 }
    } else {
        DatasetXBounds { real_bounds: None, single_x: None, scan_x_min: -10.0, scan_x_max: 10.0, span: 20.0 }
    }
}

fn extrapolation_note_for_x(bounds: &DatasetXBounds, x_val: f64, plateau_hint: Option<&'static str>) -> Option<String> {
    let prefix = plateau_hint.map(|tag| format!("{tag}: ")).unwrap_or_default();
    match (bounds.real_bounds, bounds.single_x) {
        (Some((d_min, d_max)), _) => {
            let tol = 1e-9 * bounds.span.max(1.0);
            if x_val < d_min - tol {
                Some(format!("{prefix}Extrapolated below measured standard X range [{d_min}, {d_max}]"))
            } else if x_val > d_max + tol {
                Some(format!("{prefix}Extrapolated above measured standard X range [{d_min}, {d_max}]"))
            } else {
                None
            }
        }
        (None, Some(x0)) => {
            if (x_val - x0).abs() > 1e-9 * x0.abs().max(1.0) {
                Some(format!("{prefix}Extrapolated outside single-X standard data"))
            } else {
                None
            }
        }
        (None, None) => Some(format!("{prefix}Extrapolated without finite standard X range")),
    }
}

fn plateau_tag_for_target(
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

fn band_crossing_interval<F1, F2>(band_lo_fn: F1, band_hi_fn: F2, x_hat: f64, scan_lo: f64, scan_hi: f64) -> (CiBound, CiBound)
where
    F1: Fn(f64) -> f64,
    F2: Fn(f64) -> f64,
{
    let r1 = find_nearest_root(band_lo_fn, x_hat, scan_lo, scan_hi);
    let r2 = find_nearest_root(band_hi_fn, x_hat, scan_lo, scan_hi);
    match (r1, r2) {
        (Some(a), Some(b)) => (CiBound::Finite(a.min(b)), CiBound::Finite(a.max(b))),
        (Some(a), None) => {
            if a <= x_hat {
                (CiBound::Finite(a), CiBound::Open)
            } else {
                (CiBound::Open, CiBound::Finite(a))
            }
        }
        (None, Some(b)) => {
            if b <= x_hat {
                (CiBound::Finite(b), CiBound::Open)
            } else {
                (CiBound::Open, CiBound::Finite(b))
            }
        }
        (None, None) => (CiBound::Open, CiBound::Open),
    }
}

#[derive(Clone, Copy)]
struct InverseTarget {
    response: UnknownResponse,
    replicate_mean_count: Option<NonZeroUsize>,
}

/// Invert the average response of a replicate row once and also retain each active subcolumn.
/// Seber & Wild (1989) describe inverse calibration from mean future measurements.
fn inverse_targets(responses: &[UnknownResponse]) -> impl Iterator<Item = InverseTarget> + '_ {
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
fn interpolate_x_from_y(
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
    let xb = dataset_x_bounds(ds);
    let scan_lo = xb.scan_x_min - 0.5 * xb.span;
    let scan_hi = xb.scan_x_max + 0.5 * xb.span;

    let f_diff = |x: f64| -> f64 { model.eval(&[x], ds_params, ds.data_set_index).map_or(f64::NAN, |y| y - y_target) };
    let (in_range_root, in_range_flips) = scan_interval_first_root_and_crossings(&f_diff, xb.scan_x_min, xb.scan_x_max);

    let x_hat_opt = in_range_root.or_else(|| {
        let (lo_root, _) = scan_interval_first_root_and_crossings(&f_diff, scan_lo, xb.scan_x_min);
        let (hi_root, _) = scan_interval_first_root_and_crossings(&f_diff, xb.scan_x_max, scan_hi);
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
        let samples = 40_usize;
        for i in 0..=samples {
            let x = scan_lo + (i as f64) / (samples as f64) * (scan_hi - scan_lo);
            if let Some(y) = model.eval(&[x], ds_params, ds.data_set_index)
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
    {
        let band_lo = scan_lo.min(x_hat - xb.span);
        let band_hi = scan_hi.max(x_hat + xb.span);
        let mean_lo_fn = |x: f64| -> f64 {
            let Some(y) = model.eval(&[x], ds_params, ds.data_set_index) else {
                return f64::NAN;
            };
            let g = delta_grad_at_x(model, layout, theta, ds_params, ds_local_idx, ds, x);
            let se = sigma * quadratic_form(cov_u, &g).max(0.0).sqrt();
            (y - t_crit * se) - y_target
        };
        let mean_hi_fn = |x: f64| -> f64 {
            let Some(y) = model.eval(&[x], ds_params, ds.data_set_index) else {
                return f64::NAN;
            };
            let g = delta_grad_at_x(model, layout, theta, ds_params, ds_local_idx, ds, x);
            let se = sigma * quadratic_form(cov_u, &g).max(0.0).sqrt();
            (y + t_crit * se) - y_target
        };
        let pred_lo_fn = |x: f64| -> f64 {
            let Some(y) = model.eval(&[x], ds_params, ds.data_set_index) else {
                return f64::NAN;
            };
            let g = delta_grad_at_x(model, layout, theta, ds_params, ds_local_idx, ds, x);
            let quad = quadratic_form(cov_u, &g).max(0.0);
            let inv_w = new_observation_inv_weight(solver.weighting, x, y) / observations;
            let se_p = if solver.method == FitMethod::PoissonDeviance {
                (y.max(0.0) / observations + sigma * sigma * quad).sqrt()
            } else {
                sigma * (inv_w + quad).sqrt()
            };
            (y - t_crit * se_p) - y_target
        };
        let pred_hi_fn = |x: f64| -> f64 {
            let Some(y) = model.eval(&[x], ds_params, ds.data_set_index) else {
                return f64::NAN;
            };
            let g = delta_grad_at_x(model, layout, theta, ds_params, ds_local_idx, ds, x);
            let quad = quadratic_form(cov_u, &g).max(0.0);
            let inv_w = new_observation_inv_weight(solver.weighting, x, y) / observations;
            let se_p = if solver.method == FitMethod::PoissonDeviance {
                (y.max(0.0) / observations + sigma * sigma * quad).sqrt()
            } else {
                sigma * (inv_w + quad).sqrt()
            };
            (y + t_crit * se_p) - y_target
        };
        (
            band_crossing_interval(mean_lo_fn, mean_hi_fn, x_hat, band_lo, band_hi),
            band_crossing_interval(pred_lo_fn, pred_hi_fn, x_hat, band_lo, band_hi),
        )
    } else {
        ((CiBound::Open, CiBound::Open), (CiBound::Open, CiBound::Open))
    };

    let plateau_hint = plateau_tag_for_target(model, ds_params, ds.data_set_index, y_target, xb.scan_x_min, xb.scan_x_max);
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
fn interpolate_y_from_x(
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
    let Some(y_hat) = model.eval(&[x_val], ds_params, ds.data_set_index) else {
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
    {
        let g = delta_grad_at_x(model, layout, theta, ds_params, ds_local_idx, ds, x_val);
        let quad = quadratic_form(cov_u, &g).max(0.0);
        let se = sigma * quad.sqrt();
        let inv_w = new_observation_inv_weight(solver.weighting, x_val, y_hat);
        let se_p =
            if solver.method == FitMethod::PoissonDeviance { (y_hat.max(0.0) + sigma * sigma * quad).sqrt() } else { sigma * (inv_w + quad).sqrt() };
        (
            (CiBound::Finite(y_hat - t_crit * se), CiBound::Finite(y_hat + t_crit * se)),
            (CiBound::Finite(y_hat - t_crit * se_p), CiBound::Finite(y_hat + t_crit * se_p)),
        )
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

fn scan_interval_first_root_and_crossings<F: Fn(f64) -> f64>(f: &F, lo: f64, hi: f64) -> (Option<f64>, usize) {
    let steps = 200_usize;
    let mut prev_x = lo;
    let mut prev_y = f(prev_x);
    let mut prev_nz = if prev_y.is_finite() && prev_y.abs() > 1e-12 { Some(prev_y) } else { None };
    let mut first_root = if prev_y.is_finite() && prev_y.abs() < 1e-13 { Some(prev_x) } else { None };
    let mut sign_flips = 0_usize;

    for i in 1..=steps {
        let x = lo + (i as f64) / (steps as f64) * (hi - lo);
        let y = f(x);
        if y.is_finite() && y.abs() > 1e-12 {
            if let Some(pnz) = prev_nz
                && pnz * y < 0.0
            {
                sign_flips += 1;
            }
            prev_nz = Some(y);
        }
        if first_root.is_none() && prev_y.is_finite() && y.is_finite() {
            if prev_y.abs() < 1e-13 {
                first_root = Some(prev_x);
            } else if prev_y * y <= 0.0
                && let Ok(root) = brent_root(f, prev_x, x, RootOptions { abs_tol: 1e-11, rel_tol: 1e-11, max_iters: 80 })
            {
                first_root = Some(root);
            }
        }
        prev_x = x;
        prev_y = y;
    }
    if first_root.is_none() && prev_y.is_finite() && prev_y.abs() < 1e-13 {
        first_root = Some(prev_x);
    }
    (first_root, sign_flips)
}

fn find_nearest_root<F: Fn(f64) -> f64>(f: F, anchor: f64, lo: f64, hi: f64) -> Option<f64> {
    let steps = 200_usize;
    let mut best: Option<f64> = None;
    let mut prev_x = lo;
    let mut prev_y = f(prev_x);
    for i in 1..=steps {
        let x = lo + (i as f64) / (steps as f64) * (hi - lo);
        let y = f(x);
        if prev_y.is_finite()
            && y.is_finite()
            && prev_y * y <= 0.0
            && let Ok(root) = brent_root(&f, prev_x, x, RootOptions { abs_tol: 1e-10, rel_tol: 1e-10, max_iters: 80 })
            && best.is_none_or(|b| (root - anchor).abs() < (b - anchor).abs())
        {
            best = Some(root);
        }
        prev_x = x;
        prev_y = y;
    }
    best
}
