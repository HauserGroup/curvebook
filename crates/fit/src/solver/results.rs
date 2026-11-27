//! Solver configuration and the public fit, parameter, interpolation, and model-comparison result types.
//!
//! References:
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.

use std::num::NonZeroUsize;

use curvebook_doc::{
    CiBound, ConvergenceStrictness, FitMethod, FittedCurveOverlay, IdentifiabilityMode, OutlierHandling, ParameterRule, RelationalConstraint,
    ReplicateFitMode, WeightingScheme, WeightingSource,
};
use curvebook_math::ConfidenceLevel;

use crate::diagnostics::{
    AiccComparisonResult, CovarianceDiagnostics, ExtraSumOfSquaresResult, HomoscedasticityResult, LackOfFitResult, LikelihoodRatioResult,
    NormalityTestResult, RoutDetectionResult, RunsTestResult,
};
use crate::error::{FitIdentifiability, FitStatus, ParameterNote};
use crate::types::DerivedScalar;

/// Configuration for executing nonlinear regression via [`fit_nonlinear`](crate::solver::fit_nonlinear).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SolverConfig {
    /// Fitting objective method (`LeastSquares`, `PoissonDeviance`, `RobustCauchy`, or `RoutOutlierElimination`).
    pub method: FitMethod,
    /// Variance weighting scheme (`None`, `OneOverY`, `OneOverY2`, `OneOverSd2`, `OneOverX`, `OneOverX2`, `OneOverSema`, `UserSupplied`).
    pub weighting: WeightingScheme,
    /// Per-parameter sharing scopes, constants, hooks, initial overrides, and inclusive box bounds.
    pub parameter_rules: Vec<ParameterRule>,
    /// Parameter-vs-parameter relational inequality constraints (e.g. `KFast > KSlow`, `KdLo >= KdHi + 1.0`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relational_constraints: Vec<RelationalConstraint>,
    /// Two-sided confidence level for Wald, profile-likelihood, and band intervals (default `0.95` / 95%).
    pub confidence: ConfidenceLevel,
    /// Maximum Levenberg–Marquardt trust-region / IRLS outer iterations before reporting `FitStatus::HitMaxIters`.
    pub max_iters: usize,
    /// Compute profile-likelihood confidence intervals (Venzon & Moolgavkar 1988) instead of only symmetric Wald intervals.
    pub compute_profile_ci: bool,
    /// Assess free-parameter identifiability (`FitIdentifiability` and `ParameterNote::Unstable`).
    pub identify_unstable_parameters: bool,
    /// Explicit three-way identifiability reporting mode (`IdentifyUnstableParameters`,
    /// `IdentifyAmbiguousFits`, or `ReportAll`; §2.5.1). `None` preserves legacy behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifiability_mode: Option<IdentifiabilityMode>,
    /// Compute signed Hougaard skewness `g1` per free parameter (`Hougaard 1985`; `Ratkowsky 1990`) via directional finite differences.
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
    /// Basis for response-dependent weighting (`ObservedY` static weights vs `PredictedY` curve-height IRLS).
    #[serde(default)]
    pub weighting_source: WeightingSource,
    /// Outlier handling policy (`None`, `IdentifyAndCountOnly`, or `Eliminate`).
    #[serde(default)]
    pub outlier_handling: OutlierHandling,
    /// Replicate handling mode (`IndividualReplicates` vs `RowMeansOnly`).
    #[serde(default)]
    pub replicate_fit_mode: ReplicateFitMode,
    /// Convergence strictness preset (`Default` MINPACK machine precision, `Quick`, `Medium`, or `Strict`).
    #[serde(default)]
    pub convergence_strictness: ConvergenceStrictness,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            method: FitMethod::LeastSquares,
            weighting: WeightingScheme::None,
            parameter_rules: Vec::new(),
            relational_constraints: Vec::new(),
            confidence: ConfidenceLevel::DEFAULT_95,
            max_iters: 1_000,
            compute_profile_ci: true,
            identify_unstable_parameters: false,
            identifiability_mode: None,
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
            weighting_source: WeightingSource::ObservedY,
            outlier_handling: OutlierHandling::None,
            replicate_fit_mode: ReplicateFitMode::IndividualReplicates,
            convergence_strictness: ConvergenceStrictness::Default,
        }
    }
}

impl SolverConfig {
    /// Return `true` when response-dependent weighting uses fitted curve heights `y_hat` via IRLS.
    #[must_use]
    pub const fn uses_predicted_y_weighting(&self) -> bool {
        matches!(self.weighting_source, WeightingSource::PredictedY)
            && matches!(self.weighting, WeightingScheme::RelativeSquare | WeightingScheme::Relative | WeightingScheme::GeneralPowerY(_))
    }

    /// Resolve effective ROUT outlier handling policy from `method` and `outlier_handling`.
    #[must_use]
    pub const fn effective_outlier_handling(&self) -> OutlierHandling {
        match (self.method, self.outlier_handling) {
            (FitMethod::RoutOutlierElimination { .. }, OutlierHandling::IdentifyAndCountOnly) => OutlierHandling::IdentifyAndCountOnly,
            (FitMethod::RoutOutlierElimination { .. }, OutlierHandling::None | OutlierHandling::Eliminate) => OutlierHandling::Eliminate,
            (FitMethod::LeastSquares, OutlierHandling::IdentifyAndCountOnly) => OutlierHandling::IdentifyAndCountOnly,
            (FitMethod::LeastSquares, OutlierHandling::Eliminate) => OutlierHandling::Eliminate,
            _ => OutlierHandling::None,
        }
    }

    /// Resolve the ROUT FDR threshold `Q` (in percentage points, defaulting to `1.0%` when `method` is `LeastSquares`).
    #[must_use]
    pub const fn effective_rout_q_percent(&self) -> f64 {
        match self.method {
            FitMethod::RoutOutlierElimination { fdr_q_percent } => fdr_q_percent,
            _ => 1.0,
        }
    }
}

pub(in crate::solver) fn default_fit_method() -> FitMethod {
    FitMethod::LeastSquares
}

pub(in crate::solver) const fn default_nan() -> f64 {
    f64::NAN
}

/// Poisson nonlinear regression likelihood, deviance, pseudo-$R^2$, and dispersion diagnostics.
///
/// Reference: McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PoissonNonlinearGof {
    /// Negative log-likelihood at the fitted Poisson MLE: $-\sum \ln P(y_i \mid \hat{\mu}_i)$.
    pub nll: f64,
    /// Saturated model negative log-likelihood: $-\sum \ln P(y_i \mid y_i)$.
    pub nll_saturated: f64,
    /// Null (intercept-only $\mu_0 = \bar{y}$) negative log-likelihood.
    pub nll_null: f64,
    /// Residual Poisson deviance $D = 2 (\text{NLL} - \text{NLL}_{\text{sat}})$.
    pub deviance: f64,
    /// Null model Poisson deviance $D_0 = 2 (\text{NLL}_{\text{null}} - \text{NLL}_{\text{sat}})$.
    pub null_deviance: f64,
    /// Deviance pseudo-$R^2 = 1 - D / D_0$ (`NaN` when $D_0 \le 0$).
    #[serde(default = "default_nan", deserialize_with = "deserialize_f64_or_nan")]
    pub pseudo_r2_deviance: f64,
    /// Pearson chi-square statistic $\chi^2 = \sum (y_i - \hat{\mu}_i)^2 / \hat{\mu}_i$.
    pub pearson_chi2: f64,
    /// Estimated dispersion parameter $\hat{\phi} = \chi^2 / \text{df}$ (`NaN` when `df == 0`).
    #[serde(default = "default_nan", deserialize_with = "deserialize_f64_or_nan")]
    pub dispersion: f64,
}

pub(in crate::solver) fn deserialize_f64_or_nan<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(<Option<f64> as serde::Deserialize>::deserialize(deserializer)?.unwrap_or(f64::NAN))
}

pub(in crate::solver) fn deserialize_opt_f64_vec<'de, D>(deserializer: D) -> Result<Vec<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <Vec<Option<f64>> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect())
}

pub(in crate::solver) fn deserialize_opt_f64_matrix<'de, D>(deserializer: D) -> Result<Vec<Vec<f64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <Vec<Vec<Option<f64>>> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|row| row.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect()).collect())
}

#[derive(serde::Deserialize)]
pub(in crate::solver) struct CovarianceDiagnosticsCompat {
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    pub(in crate::solver) standard_errors: Vec<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    pub(in crate::solver) symmetric_ci_lower: Vec<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    pub(in crate::solver) symmetric_ci_upper: Vec<f64>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_matrix")]
    pub(in crate::solver) covariance_matrix: Vec<Vec<f64>>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_matrix")]
    pub(in crate::solver) correlation_matrix: Vec<Vec<f64>>,
    #[serde(default, deserialize_with = "deserialize_opt_f64_vec")]
    pub(in crate::solver) dependencies: Vec<f64>,
    #[serde(default)]
    pub(in crate::solver) numerical_rank: usize,
    #[serde(default)]
    pub(in crate::solver) ambiguous: bool,
}

pub(in crate::solver) fn deserialize_covariance_diagnostics<'de, D>(deserializer: D) -> Result<CovarianceDiagnostics, D::Error>
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
    /// Signed Hougaard skewness measure $g_1$ assessing nonlinearity bias (`NaN` if uncomputed or `RobustCauchy`; $|g_1| < 0.1$ close-to-linear per Ratkowsky 1990; displayed as $|g_1|$ under `"Hougaard |g1|"` in result tables).
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

    /// Typed signed Hougaard skewness `g1` (`None` in pure Robust Cauchy mode or when non-finite).
    #[must_use]
    pub fn hougaard_g1_option(&self) -> Option<f64> {
        (self.inference_availability.is_available() && self.hougaard_g1.is_finite()).then_some(self.hougaard_g1)
    }
}

/// Default [`CiBound::Open`] helper for backwards-compatible deserialization of pre-R4.6 documents.
pub(in crate::solver) const fn default_open_ci_bound() -> CiBound {
    CiBound::Open
}

/// Default [`FitStatus::Converged`] helper for backwards-compatible deserialization.
pub(in crate::solver) const fn default_fit_status() -> FitStatus {
    FitStatus::Converged
}

/// Default `NaN` helper for backwards-compatible deserialization of optional dataset metrics.
pub(in crate::solver) const fn default_nan_f64() -> f64 {
    f64::NAN
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
    /// Objective-weighted residuals $\sqrt{w_i}(y_i - \hat{y}_i)$ (or Pearson residuals $(y_i - \mu_i) / \sqrt{\mu_i}$ for `PoissonDeviance`).
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
    /// Robust Standard Deviation of Residuals (`RSDR`) when `RobustCauchy` is active (`None` otherwise; ROUT RSDR is stored on `NonlinearFitResult::rout`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsdr: Option<f64>,
    /// Within-dataset Wald–Wolfowitz runs test computed in this dataset's own X order (`None` when disabled or plot-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_test: Option<RunsTestResult>,
    /// Solver convergence status for this dataset fit (equals the global status for shared/single fits).
    #[serde(default = "default_fit_status")]
    pub status: FitStatus,
    /// Per-dataset parameter identifiability assessment.
    #[serde(default)]
    pub identifiability: FitIdentifiability,
    /// Whether this dataset's covariance or parameter identifiability was flagged ambiguous.
    #[serde(default)]
    pub ambiguous: bool,
    /// Optimizer iterations executed for this dataset fit.
    #[serde(default)]
    pub iterations: usize,
    /// Active non-excluded observation count analyzed in this dataset.
    #[serde(default)]
    pub n_obs: usize,
    /// Free parameter count for this dataset (for shared fits, effective rounded free parameter share).
    #[serde(default)]
    pub k_free: usize,
    /// Residual degrees of freedom `n_obs.saturating_sub(k_free)` for this dataset.
    #[serde(default)]
    pub df: usize,
    /// Objective value (weighted residual SS or Poisson deviance) for this dataset (`NaN` if unpopulated).
    #[serde(default = "default_nan_f64", deserialize_with = "deserialize_f64_or_nan")]
    pub weighted_ss: f64,
    /// Per-dataset corrected Akaike Information Criterion (`None` when robust Cauchy or insufficient DF).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aic_c: Option<f64>,
    /// Per-dataset Schwarz Bayesian Information Criterion (`None` when robust Cauchy or zero obs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bic: Option<f64>,
    /// Per-dataset D'Agostino–Pearson / Shapiro–Wilk residual normality test (`None` when disabled/insufficient N).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normality: Option<NormalityTestResult>,
    /// Per-dataset Anderson–Darling residual normality test (`None` when disabled/insufficient N).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normality_anderson_darling: Option<NormalityTestResult>,
    /// Per-dataset Lilliefors (KS) residual normality test (`None` when disabled/insufficient N).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normality_lilliefors_ks: Option<NormalityTestResult>,
    /// Per-dataset Spearman rank homoscedasticity test (`None` when disabled/insufficient N).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homoscedasticity: Option<HomoscedasticityResult>,
    /// Per-dataset pure-error lack-of-fit *F*-test (`None` when disabled/no replicates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lack_of_fit: Option<LackOfFitResult>,
    /// Count of ROUT outliers flagged/excluded in this dataset.
    #[serde(default)]
    pub n_outliers: usize,
    /// Heteroscedastic `PredictedY` log-weight normalizing constant for this dataset (`None` otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_y_neg_log_weight_sum: Option<f64>,
    /// Exact Poisson log-likelihood for this dataset when `FitMethod::PoissonDeviance` is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poisson_ll: Option<f64>,
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
    /// `prefer_well_defined_fit` pre-check selected the clean/well-defined candidate without
    /// running Extra-SS *F*, LRT, or `AICc` because the competing fit was `Ambiguous`,
    /// `Unstable`, or `"Flagged"` (§2.5.2).
    PreferredWellDefinedFit {
        /// Label of the clean model selected immediately.
        preferred_label: String,
        /// Label of the rejected ill-defined or flagged model.
        rejected_label: String,
        /// `true` when the primary/null candidate was selected; `false` when the alternative was selected.
        preferred_is_primary: bool,
        /// Diagnostic reasons explaining why the rejected candidate was not well-defined.
        rejection_reasons: Vec<String>,
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
            Self::ExtraSumOfSquaresF(_) | Self::Aicc(_) | Self::LikelihoodRatioTest(_) | Self::PreferredWellDefinedFit { .. } => {
                InferenceAvailability::Available
            }
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
    /// Robust Standard Deviation of Residuals (`RSDR`, Motulsky & Brown 2006) populated under `RobustCauchy` (`None` otherwise; ROUT RSDR is stored on `self.rout`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsdr: Option<f64>,
    /// Exact heteroscedastic Gaussian log-weight normalizing sum $-\sum_i n_i \ln w_i$ under
    /// `WeightingSource::PredictedY` (`None` for data-fixed weights or Poisson/Robust fits; `AUD-03-09`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicted_y_neg_log_weight_sum: Option<f64>,
    /// Unweighted coefficient of determination $R^2 = 1 - \text{rss} / \text{TSS}$ in original $Y$ units
    /// even when weighted least squares or Poisson deviance is active (`NaN` when total SS is zero; Motulsky & Christopoulos 2004).
    #[serde(deserialize_with = "deserialize_f64_or_nan")]
    pub r_squared: f64,
    /// Unweighted adjusted coefficient of determination $R^2_{\text{adj}} = 1 - (1 - R^2)(n - 1)/\text{df}$
    /// (`NaN` when `df == 0` or `n <= 1`).
    #[serde(default = "default_nan", deserialize_with = "deserialize_f64_or_nan")]
    pub adj_r_squared: f64,
    /// Unweighted root-mean-square error $\text{RMSE} = \sqrt{\text{rss} / n}$ in original $Y$ units (`NaN` when `n == 0`).
    #[serde(default = "default_nan", deserialize_with = "deserialize_f64_or_nan")]
    pub rmse: f64,
    /// Weighted coefficient of determination $R^2_w = 1 - \text{SS}_w / \text{TSS}_w$
    /// (Willett & Singer 1988) for least-squares fits; `None` for Poisson or Robust fits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weighted_r_squared: Option<f64>,
    /// Weighted adjusted $R^2_{w,\text{adj}} = 1 - (1 - R^2_w)(n - 1)/\text{df}$ for least-squares fits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weighted_adj_r_squared: Option<f64>,
    /// Weighted root-mean-square error $\sqrt{\text{SS}_w / n}$ for least-squares fits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weighted_rmse: Option<f64>,
    /// Weighted residual standard deviation $S_{y.x,w} = \sqrt{\text{SS}_w / \text{df}}$ for least-squares fits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weighted_sy_x: Option<f64>,
    /// Poisson nonlinear regression likelihood, deviance, pseudo-$R^2$, and dispersion diagnostics
    /// (populated under [`FitMethod::PoissonDeviance`], `None` otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poisson_gof: Option<PoissonNonlinearGof>,
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
    /// `true` when one or more parameters are shared globally across multiple datasets.
    #[serde(default)]
    pub has_shared_parameters: bool,
}

impl NonlinearFitResult {
    /// Return `true` when multiple datasets were fitted independently without shared global parameters.
    #[must_use]
    pub fn is_independent_multi_dataset(&self) -> bool {
        !self.has_shared_parameters && self.datasets.len() > 1
    }
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
