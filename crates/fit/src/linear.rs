//! Simple linear regression (OLS, origin-forced, bands, unknown interpolation / calibration,
//! ANCOVA line comparison) and Model II Deming / Orthogonal regression with jackknife CIs.
//!
//! # References
//!
//! Short `Author (Year)` citations on individual items refer to the list below.
//!
//! Reference: Draper, N. R. & Smith, H. (1998). *Applied Regression Analysis*, 3rd ed. Wiley.
//!
//! Reference: Zar, J. H. (2010). *Biostatistical Analysis*, 5th ed. Pearson Prentice Hall.
//!
//! Reference: Fieller, E. C. (1954). Some problems in interval estimation. *Journal of the Royal Statistical Society, Series B* 16(2), 175–185.
//!
//! Reference: Seber, G. A. F. & Lee, A. J. (2003). *Linear Regression Analysis*, 2nd ed. Wiley.
//!
//! Reference: NIST/SEMATECH e-Handbook of Statistical Methods, §4.5.1.1 (Linear Calibration).
//!
//! Reference: Deming, W. E. (1943). *Statistical Adjustment of Data*. Wiley.
//!
//! Reference: Linnet, K. (1990). Estimation of the linear relationship between the measurements of two methods with proportional errors. *Statistics in Medicine* 9(12), 1463–1473. doi:10.1002/sim.4780091210
//!
//! Reference: Linnet, K. (1993). Evaluation of regression procedures for methods comparison studies. *Clinical Chemistry* 39(3), 424–432. doi:10.1093/clinchem/39.3.424
//!
//! Reference: Efron, B. & Tibshirani, R. J. (1993). *An Introduction to the Bootstrap*. Chapman & Hall.

use lumencraft_doc::{CiBound, CurveSamplePoint};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{f_sf, student_t_two_sided_isf};
use lumencraft_math::sum::{NeumaierSum, SummaryMoments};

use crate::diagnostics::{LackOfFitResult, RunsTestResult, replicates_lack_of_fit, runs_test};
use crate::error::FitError;
use crate::types::{DataPoint, validated_replicate_moments};

/// Configuration for [`simple_linear_regression`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SimpleLinearOptions {
    pub force_through_origin: bool,
    pub confidence_level: ConfidenceLevel,
    pub compute_bands: bool,
    pub band_grid_points: usize,
    pub unknown_y_values: Vec<f64>,
    pub unknown_x_values: Vec<f64>,
}

impl Default for SimpleLinearOptions {
    fn default() -> Self {
        Self {
            force_through_origin: false,
            confidence_level: ConfidenceLevel::DEFAULT_95,
            compute_bands: true,
            band_grid_points: 101,
            unknown_y_values: Vec::new(),
            unknown_x_values: Vec::new(),
        }
    }
}

/// Interpolated `X` from `Y` (or `Y` from `X`) with mean-response confidence and
/// prediction / calibration intervals.
///
/// - When predicting `Y` from `X`: `ci_lower..ci_upper` is the pointwise mean-response confidence
///   interval and `pred_lower..pred_upper` is the single-observation (`m = 1`) prediction interval
///   (Draper & Smith 1998).
/// - When calibrating `X` from observed `Y`: `ci_lower..ci_upper` is the Fieller (1954)
///   mean-response inverse confidence interval (`1/m = 0`; the `X` at which `E[Y | X] = y0`),
///   whereas `pred_lower..pred_upper` is the single-observation (`m = 1`) classical calibration /
///   inverse prediction interval (Seber & Lee 2003, §6.1.3; NIST/SEMATECH §4.5.1.1) when
///   finite (`g < 1`), or `None` when unbounded (`g >= 1`) or degenerate.
///   Use [`interpolate_x_from_y`] for `m >= 1` future replicates or full topological set
///   classification ([`InverseIntervalSet`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearInterpolation {
    pub input_value: f64,
    pub estimate: f64,
    pub ci_lower: CiBound,
    pub ci_upper: CiBound,
    pub pred_lower: Option<f64>,
    pub pred_upper: Option<f64>,
}

/// Complete topological solution set of the inverse calibration / Fieller quadratic inequality
/// `(y0 - (a + b x))^2 <= t^2 s^2 (k0 + (x - x_c)^2 / S_xx)` (Fieller 1954; Seber & Lee 2003, §6.1.3).
///
/// Here `g = t^2 s^2 / (b^2 S_xx)` tests whether the fitted slope differs significantly from `0`.
/// Depending on `g` and `y0`, the inverse confidence/calibration set can take any of five exact
/// geometric forms:
/// - [`InverseIntervalSet::Bounded`] (`g < 1`, or exact noiseless `s == 0`): closed interval `[lower, upper]`.
/// - [`InverseIntervalSet::HalfLine`] (`g == 1` with `x_hat != x_c`): single ray `[lower, +inf)` or `(-inf, upper]`.
/// - [`InverseIntervalSet::TwoRays`] (`g > 1` and `D' > 0`): exterior of two roots `(-inf, left_upper] U [right_lower, +inf)`.
/// - [`InverseIntervalSet::AllReals`] (`g > 1` and `D' <= 0`, or `g == 1` with `x_hat == x_c`): `(-inf, +inf)`.
/// - [`InverseIntervalSet::Undefined`]: flat line (`slope == 0`) or non-finite / overflowed inputs.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InverseIntervalSet {
    Bounded { lower: f64, upper: f64 },
    HalfLine { lower: Option<f64>, upper: Option<f64> },
    TwoRays { left_upper: f64, right_lower: f64 },
    AllReals,
    Undefined,
}

impl InverseIntervalSet {
    /// Return finite closed endpoints `(lower, upper)` when the set is [`Self::Bounded`].
    #[must_use]
    pub const fn finite_bounds(&self) -> Option<(f64, f64)> {
        match *self {
            Self::Bounded { lower, upper } if lower.is_finite() && upper.is_finite() => Some((lower, upper)),
            _ => None,
        }
    }

    /// Project the solution set onto scalar [`CiBound`] lower/upper bounds.
    ///
    /// Note: [`Self::TwoRays`] (`(-inf, u] U [l, +inf)`), [`Self::AllReals`], and
    /// [`Self::Undefined`] have unbounded convex hulls and project to `(CiBound::Open, CiBound::Open)`.
    /// Use `match` on `InverseIntervalSet` directly when distinguishing two exterior rays from `AllReals`.
    #[must_use]
    pub const fn to_ci_bounds(&self) -> (CiBound, CiBound) {
        match *self {
            Self::Bounded { lower, upper } if lower.is_finite() && upper.is_finite() => (CiBound::Finite(lower), CiBound::Finite(upper)),
            Self::HalfLine { lower: Some(lo), upper: None } if lo.is_finite() => (CiBound::Finite(lo), CiBound::Open),
            Self::HalfLine { lower: None, upper: Some(hi) } if hi.is_finite() => (CiBound::Open, CiBound::Finite(hi)),
            _ => (CiBound::Open, CiBound::Open),
        }
    }

    /// Test whether candidate `x` belongs to this solution set.
    #[must_use]
    pub fn contains(&self, x: f64) -> bool {
        if !x.is_finite() {
            return false;
        }
        match *self {
            Self::Bounded { lower, upper } => x >= lower && x <= upper,
            Self::HalfLine { lower, upper } => lower.is_none_or(|lo| x >= lo) && upper.is_none_or(|hi| x <= hi),
            Self::TwoRays { left_upper, right_lower } => x <= left_upper || x >= right_lower,
            Self::AllReals => true,
            Self::Undefined => false,
        }
    }
}

/// Separated mean-response inverse confidence interval and classical calibration
/// (inverse prediction) interval for `unknown_replicates` (`m >= 1`) new observations of `Y`.
///
/// Follows Fieller (1954), Draper & Smith (1998, Ch. 3), Seber & Lee (2003, §6.1.3), and
/// NIST/SEMATECH e-Handbook §4.5.1.1.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InverseCalibrationResult {
    pub input_y: f64,
    pub estimate: f64,
    pub unknown_replicates: usize,
    pub fieller_g: f64,
    /// Mean-response inverse confidence interval (`1/m = 0`; excludes future-observation variance).
    pub mean_response_ci: (CiBound, CiBound),
    /// Exact topological solution set for the mean-response inverse CI (`1/m = 0`).
    pub mean_response_set: InverseIntervalSet,
    /// Classical calibration interval for `m` new replicates (includes `1/m` observation error).
    pub calibration_pi: (CiBound, CiBound),
    /// Exact topological solution set for the classical calibration interval (`1/m` included).
    pub calibration_set: InverseIntervalSet,
}

/// Per-point residual diagnostic record.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearResidual {
    pub x: f64,
    pub y: f64,
    pub predicted: f64,
    pub residual: f64,
}

/// Complete simple linear regression output.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SimpleLinearResult {
    pub n: usize,
    pub df_residual: usize,
    pub force_through_origin: bool,
    pub slope: f64,
    pub intercept: f64,
    pub se_slope: f64,
    pub se_intercept: f64,
    pub ci_slope: (f64, f64),
    pub ci_intercept: (f64, f64),
    pub r_squared: f64,
    pub r: f64,
    pub sy_x: f64,
    pub rss: f64,
    pub ss_total: f64,
    pub f_stat: f64,
    pub p_value: f64,
    pub x_intercept: Option<f64>,
    pub one_over_slope: Option<f64>,
    pub runs_test: RunsTestResult,
    pub lack_of_fit: Option<LackOfFitResult>,
    pub bands: Vec<CurveSamplePoint>,
    pub interpolated_x_from_y: Vec<LinearInterpolation>,
    pub interpolated_y_from_x: Vec<LinearInterpolation>,
    pub residuals: Vec<LinearResidual>,
}

/// ANCOVA comparison of slopes and intercepts across multiple linear regression datasets.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearComparisonResult {
    pub dataset_count: usize,
    pub f_slopes: f64,
    pub df_slopes: (usize, usize),
    pub p_slopes: f64,
    pub slopes_significantly_different: bool,
    pub pooled_slope: f64,
    pub f_intercepts: Option<f64>,
    pub df_intercepts: Option<(usize, usize)>,
    pub p_intercepts: Option<f64>,
    pub intercepts_significantly_different: Option<bool>,
}

/// Configuration for Model II Deming regression.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DemingOptions {
    /// Variance ratio `lambda = sigma_x^2 / sigma_y^2` (`1.0` for Orthogonal / Major Axis Deming).
    pub variance_ratio_lambda: f64,
    pub confidence_level: ConfidenceLevel,
}

impl Default for DemingOptions {
    fn default() -> Self {
        Self { variance_ratio_lambda: 1.0, confidence_level: ConfidenceLevel::DEFAULT_95 }
    }
}

/// Complete Model II Deming regression output with Leave-One-Out jackknife SEs and CIs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DemingResult {
    pub n: usize,
    pub df: usize,
    pub lambda: f64,
    pub slope: f64,
    pub intercept: f64,
    pub se_slope: f64,
    pub se_intercept: f64,
    pub ci_slope: (f64, f64),
    pub ci_intercept: (f64, f64),
    pub sy_x: f64,
    pub x_intercept: Option<f64>,
    pub residuals: Vec<LinearResidual>,
}

struct WeightedXySeries {
    xs: Vec<f64>,
    ys: Vec<f64>,
    ns: Vec<f64>,
    n_total: usize,
    ss_within: f64,
}

fn extract_linear_series(points: &[DataPoint]) -> Result<WeightedXySeries, FitError> {
    let mut xs = Vec::with_capacity(points.len());
    let mut ys = Vec::with_capacity(points.len());
    let mut ns = Vec::with_capacity(points.len());
    let mut n_total = 0_usize;
    let mut ss_within_acc = NeumaierSum::new();
    for pt in points {
        let x = pt.x0();
        let y = pt.y;
        if !x.is_finite() || !y.is_finite() {
            return Err(FitError::InvalidParameter { name: "xy", reason: String::from("non-finite value") });
        }
        let (n_f, n_u, ss_i) = validated_replicate_moments(pt)?;
        n_total = n_total
            .checked_add(n_u)
            .ok_or_else(|| FitError::InvalidParameter { name: "n", reason: String::from("total replicate count overflowed usize") })?;
        ss_within_acc.add(ss_i);
        xs.push(x);
        ys.push(y);
        ns.push(n_f);
    }
    let ss_within = ss_within_acc.total().max(0.0);
    if !ss_within.is_finite() {
        return Err(FitError::InvalidParameter { name: "sd", reason: String::from("within-replicate SS overflowed f64 range") });
    }
    Ok(WeightedXySeries { xs, ys, ns, n_total, ss_within })
}

struct OlsFitCore {
    series: WeightedXySeries,
    n: usize,
    df_res: usize,
    slope: f64,
    intercept: f64,
    se_slope: f64,
    se_intercept: f64,
    s_xx: f64,
    s_yy: f64,
    s_xy: f64,
    x_mean: f64,
    rss: f64,
    sy_x: f64,
    raw_res: Vec<f64>,
    residuals: Vec<LinearResidual>,
}

fn fit_ols_core(points: &[DataPoint], force_through_origin: bool) -> Result<OlsFitCore, FitError> {
    let min_n = if force_through_origin { 2 } else { 3 };
    if points.is_empty() {
        return Err(FitError::EmptyInput);
    }
    let series = extract_linear_series(points)?;
    let n = series.n_total;
    if n < min_n {
        return Err(FitError::InsufficientData { required: min_n, actual: n });
    }

    let (slope, intercept, se_slope_factor, se_intercept_factor, s_xx, s_yy, s_xy, x_mean) = if force_through_origin {
        fit_through_origin(&series.xs, &series.ys, &series.ns, series.ss_within)?
    } else {
        fit_with_intercept(&series.xs, &series.ys, &series.ns, series.n_total, series.ss_within)?
    };

    let df_res = if force_through_origin { n - 1 } else { n - 2 };
    let mut rss_acc = NeumaierSum::new();
    let mut residuals = Vec::with_capacity(series.xs.len());
    let mut raw_res = Vec::with_capacity(series.xs.len());
    for ((&x, &y), &w) in series.xs.iter().zip(series.ys.iter()).zip(series.ns.iter()) {
        let pred = intercept + slope * x;
        let res = y - pred;
        rss_acc.add(w * res * res);
        raw_res.push(res);
        residuals.push(LinearResidual { x, y, predicted: pred, residual: res });
    }
    rss_acc.add(series.ss_within);
    let rss = rss_acc.total().max(0.0);
    let sy_x = (rss / (df_res as f64)).sqrt();
    let (se_slope, se_intercept) =
        if force_through_origin { (sy_x * se_slope_factor, 0.0) } else { (sy_x * se_slope_factor, sy_x * se_intercept_factor) };

    Ok(OlsFitCore { series, n, df_res, slope, intercept, se_slope, se_intercept, s_xx, s_yy, s_xy, x_mean, rss, sy_x, raw_res, residuals })
}

/// Perform simple OLS linear regression (or origin-forced linear regression) on `(x, y)` pairs.
///
/// Textbook least squares (Draper & Smith 1998; Zar 2010): `b = S_xy / S_xx`, `a = ybar - b xbar`,
/// `s = sqrt(RSS / df)`, `SE(b) = s / sqrt(S_xx)`, `SE(a) = s sqrt(1/n + xbar^2 / S_xx)`, and the
/// regression *F* test on `(1, df)`. Through the origin, raw (uncentred) sums are used with
/// `df = n - 1`; `R^2` there is reported against the uncentred `sum y^2`.
/// Also runs the runs test and replicates lack-of-fit test from [`crate::diagnostics`].
///
/// On `interpolated_x_from_y`, `ci_lower..ci_upper` holds the Fieller (1954) mean-response inverse
/// confidence interval (`1/m = 0`), while `pred_lower..pred_upper` holds the single-observation
/// (`m = 1`) classical calibration / inverse prediction interval (Seber & Lee 2003, §6.1.3) when
/// bounded (`g < 1`). Use [`interpolate_x_from_y`] for `m >= 1` replicates or full
/// [`InverseIntervalSet`] classification.
///
/// # Errors
/// Returns [`FitError::InsufficientData`] if too few observations are given, or
/// [`FitError::InvalidParameter`] if `X` has zero variance (or all `X == 0` when forced through origin).
pub fn simple_linear_regression(points: &[DataPoint], opts: &SimpleLinearOptions) -> Result<SimpleLinearResult, FitError> {
    let fit = fit_ols_core(points, opts.force_through_origin)?;
    let t_crit = student_t_two_sided_isf(opts.confidence_level.alpha(), fit.df_res as f64)?;
    let ci_slope = (fit.slope - t_crit * fit.se_slope, fit.slope + t_crit * fit.se_slope);
    let ci_intercept =
        if opts.force_through_origin { (0.0, 0.0) } else { (fit.intercept - t_crit * fit.se_intercept, fit.intercept + t_crit * fit.se_intercept) };

    let r_squared = if fit.s_yy > 0.0 { (1.0 - fit.rss / fit.s_yy).clamp(0.0, 1.0) } else { 1.0 };
    let r = if fit.s_xx > 0.0 && fit.s_yy > 0.0 { (fit.s_xy / (fit.s_xx * fit.s_yy).sqrt()).clamp(-1.0, 1.0) } else { 0.0 };
    let ms_res = fit.rss / (fit.df_res as f64);
    let ss_reg = (fit.s_yy - fit.rss).max(0.0);
    let f_stat = if ms_res > 0.0 {
        ss_reg / ms_res
    } else if ss_reg > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let p_value = if f_stat.is_infinite() { 0.0 } else { f_sf(f_stat, 1.0, fit.df_res as f64).unwrap_or(1.0) };

    // Sort residuals by X for runs test so sign ordering tracks predictor progression.
    let mut ordered_pairs: Vec<(f64, f64)> = fit.series.xs.iter().copied().zip(fit.raw_res.iter().copied()).collect();
    ordered_pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let sorted_res: Vec<f64> = ordered_pairs.into_iter().map(|(_, res)| res).collect();
    let runs = runs_test(&sorted_res);
    let lack_of_fit = replicates_lack_of_fit(points, fit.rss, fit.df_res);

    let bands = if opts.compute_bands {
        build_linear_bands(
            &fit.series.xs,
            fit.n,
            fit.slope,
            fit.intercept,
            fit.sy_x,
            t_crit,
            fit.x_mean,
            fit.s_xx,
            opts.force_through_origin,
            opts.band_grid_points,
        )
    } else {
        Vec::new()
    };

    let inv_ctx = InverseQuadraticContext::from_fit(&fit, t_crit, opts.force_through_origin);
    let interpolated_x_from_y = interpolate_x_list(&opts.unknown_y_values, &inv_ctx);
    let interpolated_y_from_x = interpolate_y_list(&opts.unknown_x_values, &inv_ctx);

    Ok(SimpleLinearResult {
        n: fit.n,
        df_residual: fit.df_res,
        force_through_origin: opts.force_through_origin,
        slope: fit.slope,
        intercept: fit.intercept,
        se_slope: fit.se_slope,
        se_intercept: fit.se_intercept,
        ci_slope,
        ci_intercept,
        r_squared,
        r,
        sy_x: fit.sy_x,
        rss: fit.rss,
        ss_total: fit.s_yy,
        f_stat,
        p_value,
        x_intercept: if fit.slope != 0.0 { Some(-fit.intercept / fit.slope) } else { None },
        one_over_slope: if fit.slope != 0.0 { Some(1.0 / fit.slope) } else { None },
        runs_test: runs,
        lack_of_fit,
        bands,
        interpolated_x_from_y,
        interpolated_y_from_x,
        residuals: fit.residuals,
    })
}

/// Classical linear inverse calibration (`X` from observed `Y`) separating the mean-response
/// inverse confidence interval (`1/m = 0`) from the calibration / inverse prediction interval for
/// the mean of `unknown_replicates` (`m >= 1`) new observations (Fieller 1954; Draper & Smith 1998;
/// Seber & Lee 2003, §6.1.3; NIST/SEMATECH §4.5.1.1).
///
/// Exact quadratic inequality solutions are reported in [`InverseCalibrationResult::mean_response_set`]
/// and [`InverseCalibrationResult::calibration_set`], preserving [`InverseIntervalSet::Bounded`],
/// [`InverseIntervalSet::HalfLine`] (`g == 1`), [`InverseIntervalSet::TwoRays`] (`g > 1, D' > 0`),
/// and [`InverseIntervalSet::AllReals`] (`g > 1, D' <= 0`).
///
/// # Errors
/// Returns [`FitError::InvalidParameter`] if `unknown_replicates == 0` or predictor `X` is degenerate,
/// or [`FitError::InsufficientData`] if too few observations are supplied.
pub fn interpolate_x_from_y(
    points: &[DataPoint],
    force_through_origin: bool,
    confidence_level: ConfidenceLevel,
    unknown_ys: &[f64],
    unknown_replicates: usize,
) -> Result<Vec<InverseCalibrationResult>, FitError> {
    if unknown_replicates == 0 {
        return Err(FitError::InvalidParameter { name: "unknown_replicates", reason: String::from("must be at least 1") });
    }
    let fit = fit_ols_core(points, force_through_origin)?;
    let t_crit = student_t_two_sided_isf(confidence_level.alpha(), fit.df_res as f64)?;
    let ctx = InverseQuadraticContext::from_fit(&fit, t_crit, force_through_origin);
    let inv_m = 1.0 / (unknown_replicates as f64);
    let k_mean = ctx.n_inv;
    let k_cal = ctx.n_inv + inv_m;

    let mut out = Vec::with_capacity(unknown_ys.len());
    for &y0 in unknown_ys {
        let (x_hat, mean_set, cal_set) = if !y0.is_finite() || ctx.slope == 0.0 || !ctx.slope.is_finite() {
            (f64::NAN, InverseIntervalSet::Undefined, InverseIntervalSet::Undefined)
        } else {
            let x_hat = (y0 - ctx.intercept) / ctx.slope;
            if !x_hat.is_finite() {
                (f64::NAN, InverseIntervalSet::Undefined, InverseIntervalSet::Undefined)
            } else {
                let mean_set = solve_inverse_quadratic(&ctx, y0, x_hat, k_mean);
                let cal_set = solve_inverse_quadratic(&ctx, y0, x_hat, k_cal);
                (x_hat, mean_set, cal_set)
            }
        };
        out.push(InverseCalibrationResult {
            input_y: y0,
            estimate: x_hat,
            unknown_replicates,
            fieller_g: ctx.g,
            mean_response_ci: mean_set.to_ci_bounds(),
            mean_response_set: mean_set,
            calibration_pi: cal_set.to_ci_bounds(),
            calibration_set: cal_set,
        });
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
fn fit_with_intercept(
    xs: &[f64],
    ys: &[f64],
    ns: &[f64],
    n_total: usize,
    ss_within: f64,
) -> Result<(f64, f64, f64, f64, f64, f64, f64, f64), FitError> {
    let nf = n_total as f64;
    let mut sum_x = NeumaierSum::new();
    let mut sum_y = NeumaierSum::new();
    for ((&x, &y), &w) in xs.iter().zip(ys.iter()).zip(ns.iter()) {
        sum_x.add(w * x);
        sum_y.add(w * y);
    }
    let x_mean = sum_x.total() / nf;
    let y_mean = sum_y.total() / nf;

    let mut dx_acc = NeumaierSum::new();
    let mut dy_acc = NeumaierSum::new();
    let mut sxx_acc = NeumaierSum::new();
    let mut syy_acc = NeumaierSum::new();
    let mut xy_acc = NeumaierSum::new();
    for ((&x, &y), &w) in xs.iter().zip(ys.iter()).zip(ns.iter()) {
        let dx = x - x_mean;
        let dy = y - y_mean;
        dx_acc.add(w * dx);
        dy_acc.add(w * dy);
        sxx_acc.add(w * dx * dx);
        syy_acc.add(w * dy * dy);
        let p = w * dx * dy;
        xy_acc.add(p);
        xy_acc.add((w * dx).mul_add(dy, -p));
    }
    let dx_tot = dx_acc.total();
    let dy_tot = dy_acc.total();
    let s_xx = (sxx_acc.total() - (dx_tot * dx_tot) / nf).max(0.0);
    if s_xx <= 0.0 || !s_xx.is_finite() {
        return Err(FitError::InvalidParameter { name: "x", reason: String::from("predictor X has zero variance") });
    }
    let s_yy = (syy_acc.total() - (dy_tot * dy_tot) / nf).max(0.0) + ss_within;
    let s_xy = xy_acc.total() - (dx_tot * dy_tot) / nf;
    let slope = s_xy / s_xx;
    let intercept = y_mean - slope * x_mean;
    let se_slope_factor = (1.0 / s_xx).sqrt();
    let se_intercept_factor = (1.0 / nf + (x_mean * x_mean) / s_xx).sqrt();
    Ok((slope, intercept, se_slope_factor, se_intercept_factor, s_xx, s_yy, s_xy, x_mean))
}

#[allow(clippy::type_complexity)]
fn fit_through_origin(xs: &[f64], ys: &[f64], ns: &[f64], ss_within: f64) -> Result<(f64, f64, f64, f64, f64, f64, f64, f64), FitError> {
    let mut s_xx_acc = NeumaierSum::new();
    let mut s_yy_acc = NeumaierSum::new();
    let mut s_xy_acc = NeumaierSum::new();
    for ((&x, &y), &w) in xs.iter().zip(ys.iter()).zip(ns.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return Err(FitError::InvalidParameter { name: "xy", reason: String::from("non-finite value") });
        }
        s_xx_acc.add(w * x * x);
        s_yy_acc.add(w * y * y);
        s_xy_acc.add(w * x * y);
    }
    let s_xx = s_xx_acc.total();
    let s_yy = s_yy_acc.total() + ss_within;
    let s_xy = s_xy_acc.total();
    if s_xx <= 0.0 || !s_xx.is_finite() {
        return Err(FitError::InvalidParameter { name: "x", reason: String::from("sum of X^2 must be positive for origin-forced regression") });
    }
    let slope = s_xy / s_xx;
    let se_slope_factor = (1.0 / s_xx).sqrt();
    Ok((slope, 0.0, se_slope_factor, 0.0, s_xx, s_yy, s_xy, 0.0))
}

/// Pointwise confidence band `yhat +/- t s sqrt(h)` and prediction band
/// `yhat +/- t s sqrt(1 + h)`, with `h = 1/n + (x - xbar)^2 / S_xx` (or `x^2 / S_xx` through the
/// origin) (Draper & Smith 1998). Not simultaneous (Working–Hotelling) bands.
#[allow(clippy::too_many_arguments)]
fn build_linear_bands(
    xs: &[f64],
    n_total: usize,
    slope: f64,
    intercept: f64,
    sy_x: f64,
    t_crit: f64,
    x_mean: f64,
    s_xx: f64,
    origin: bool,
    grid_points: usize,
) -> Vec<CurveSamplePoint> {
    let count = grid_points.max(2);
    let x_min = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let x_max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let n_inv = if origin { 0.0 } else { 1.0 / (n_total as f64) };
    let span = x_max - x_min;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let frac = (i as f64) / ((count - 1) as f64);
        let x = x_min + frac * span;
        let y = intercept + slope * x;
        let dx = if origin { x } else { x - x_mean };
        let h = n_inv + (dx * dx) / s_xx;
        let ci_half = t_crit * sy_x * h.sqrt();
        let pi_half = t_crit * sy_x * (1.0 + h).sqrt();
        out.push(CurveSamplePoint {
            x,
            y,
            ci_lower: Some(y - ci_half),
            ci_upper: Some(y + ci_half),
            pred_lower: Some(y - pi_half),
            pred_upper: Some(y + pi_half),
        });
    }
    out
}

/// Tolerance around Fieller `g = 1` so exact algebraic `g == 1` inputs distorted by a few
/// ULPs during `t_crit` / OLS rounding hit the degenerate linear-inequality half-line branch.
const FIELLER_G_UNITY_EPS: f64 = 1e-13;

struct InverseQuadraticContext {
    slope: f64,
    intercept: f64,
    sy_x: f64,
    t_crit: f64,
    n_inv: f64,
    x_center: f64,
    s_xx: f64,
    sqrt_sxx: f64,
    g: f64,
}

impl InverseQuadraticContext {
    fn from_fit(fit: &OlsFitCore, t_crit: f64, origin: bool) -> Self {
        let sqrt_sxx = if fit.s_xx > 0.0 && fit.s_xx.is_finite() { fit.s_xx.sqrt() } else { 0.0 };
        let g = if fit.slope != 0.0 && fit.slope.is_finite() && sqrt_sxx > 0.0 && fit.sy_x.is_finite() && t_crit.is_finite() {
            let ratio = (t_crit * fit.sy_x) / (fit.slope.abs() * sqrt_sxx);
            ratio * ratio
        } else {
            f64::INFINITY
        };
        Self {
            slope: fit.slope,
            intercept: fit.intercept,
            sy_x: fit.sy_x,
            t_crit,
            n_inv: if origin { 0.0 } else { 1.0 / (fit.n as f64) },
            x_center: if origin { 0.0 } else { fit.x_mean },
            s_xx: fit.s_xx,
            sqrt_sxx,
            g,
        }
    }
}

/// Solve `(y0 - (a + b x))^2 <= t^2 s^2 (k0 + (x - x_c)^2 / S_xx)` over all regimes of `g`
/// (Fieller 1954; Draper & Smith 1998, Ch. 3; Seber & Lee 2003, §6.1.3).
fn solve_inverse_quadratic(ctx: &InverseQuadraticContext, y0: f64, x_hat: f64, k0: f64) -> InverseIntervalSet {
    if !y0.is_finite()
        || !x_hat.is_finite()
        || ctx.slope == 0.0
        || !ctx.slope.is_finite()
        || ctx.s_xx <= 0.0
        || !ctx.sy_x.is_finite()
        || !ctx.t_crit.is_finite()
        || ctx.t_crit < 0.0
    {
        return InverseIntervalSet::Undefined;
    }
    if ctx.sy_x == 0.0 || ctx.t_crit == 0.0 {
        return InverseIntervalSet::Bounded { lower: x_hat, upper: x_hat };
    }
    if !ctx.g.is_finite() {
        return InverseIntervalSet::Undefined;
    }

    let y_at_center = ctx.intercept + ctx.slope * ctx.x_center;
    let d = if y_at_center.is_finite() { (y0 - y_at_center) / ctx.slope } else { x_hat - ctx.x_center };
    if !d.is_finite() {
        return InverseIntervalSet::Undefined;
    }

    if (1.0 - ctx.g).abs() <= FIELLER_G_UNITY_EPS {
        solve_inverse_g_eq_1(ctx, x_hat, d, k0)
    } else if ctx.g < 1.0 {
        solve_inverse_g_lt_1(ctx, d, k0)
    } else {
        solve_inverse_g_gt_1(ctx, d, k0)
    }
}

fn solve_inverse_g_lt_1(ctx: &InverseQuadraticContext, d: f64, k0: f64) -> InverseIntervalSet {
    let one_minus_g = 1.0 - ctx.g;
    let x_mid = ctx.x_center + d / one_minus_g;
    let rad = (one_minus_g * k0).max(0.0).sqrt().hypot(d / ctx.sqrt_sxx);
    let half = ctx.t_crit * ctx.sy_x * rad / (ctx.slope.abs() * one_minus_g);
    let lo = x_mid - half;
    let hi = x_mid + half;
    if lo.is_finite() && hi.is_finite() {
        InverseIntervalSet::Bounded { lower: lo.min(hi), upper: lo.max(hi) }
    } else {
        InverseIntervalSet::Undefined
    }
}

fn solve_inverse_g_eq_1(ctx: &InverseQuadraticContext, x_hat: f64, d: f64, k0: f64) -> InverseIntervalSet {
    // At g == 1 the quadratic term (1 - g) delta^2 drops out: 2 d delta >= d^2 - S_xx k0.
    let zero_tol = 64.0 * f64::EPSILON * (ctx.x_center.abs() + x_hat.abs());
    if d == 0.0 || d.abs() <= zero_tol {
        return InverseIntervalSet::AllReals;
    }
    let bound = ctx.x_center + 0.5 * (d - (ctx.s_xx * k0) / d);
    if !bound.is_finite() {
        return InverseIntervalSet::Undefined;
    }
    if d > 0.0 {
        InverseIntervalSet::HalfLine { lower: Some(bound), upper: None }
    } else {
        InverseIntervalSet::HalfLine { lower: None, upper: Some(bound) }
    }
}

fn solve_inverse_g_gt_1(ctx: &InverseQuadraticContext, d: f64, k0: f64) -> InverseIntervalSet {
    let g_minus_1 = ctx.g - 1.0;
    let abs_d = d.abs();
    let t1 = abs_d / ctx.sqrt_sxx;
    let t2 = (g_minus_1 * k0).max(0.0).sqrt();
    if t1 <= t2 {
        return InverseIntervalSet::AllReals;
    }

    let scale_gsxx = (ctx.t_crit * ctx.sy_x) / ctx.slope.abs();
    let s_root = scale_gsxx * (t1 - t2).sqrt() * (t1 + t2).sqrt();
    let sum_denom = abs_d + s_root;
    if sum_denom <= 0.0 || !sum_denom.is_finite() {
        return InverseIntervalSet::Undefined;
    }

    // Solve (g - 1) delta^2 + 2 d delta + (g S_xx k0 - d^2) >= 0 with cancellation-free Vieta root.
    let far_root = if d >= 0.0 { -sum_denom / g_minus_1 } else { sum_denom / g_minus_1 };
    let r2 = ctx.g * ctx.s_xx * k0;
    let near_root = if abs_d < 1e150 && r2.is_finite() {
        let c_quad = r2 - d * d;
        if d >= 0.0 { -c_quad / sum_denom } else { c_quad / sum_denom }
    } else {
        let q = r2.sqrt();
        let fac = (q - abs_d) * ((q + abs_d) / sum_denom);
        if d >= 0.0 { -fac } else { fac }
    };

    let delta_lo = far_root.min(near_root);
    let delta_hi = far_root.max(near_root);
    let left_upper = ctx.x_center + delta_lo;
    let right_lower = ctx.x_center + delta_hi;
    if left_upper.is_finite() && right_lower.is_finite() {
        InverseIntervalSet::TwoRays { left_upper, right_lower }
    } else {
        InverseIntervalSet::Undefined
    }
}

/// Inverse estimation of `X` from `Y`: populates both the Fieller (1954) mean-response inverse
/// confidence interval (`ci_lower..ci_upper`, `1/m = 0`) and the single-observation (`m = 1`)
/// classical calibration / inverse prediction interval (`pred_lower..pred_upper`, Seber & Lee 2003,
/// §6.1.3) when bounded (`g < 1`).
fn interpolate_x_list(unknown_ys: &[f64], ctx: &InverseQuadraticContext) -> Vec<LinearInterpolation> {
    let mut out = Vec::with_capacity(unknown_ys.len());
    let k_mean = ctx.n_inv;
    let k_cal = ctx.n_inv + 1.0;

    for &y0 in unknown_ys {
        let x_hat = if y0.is_finite() && ctx.slope != 0.0 && ctx.slope.is_finite() { (y0 - ctx.intercept) / ctx.slope } else { f64::NAN };
        if !x_hat.is_finite() {
            out.push(LinearInterpolation {
                input_value: y0,
                estimate: f64::NAN,
                ci_lower: CiBound::Open,
                ci_upper: CiBound::Open,
                pred_lower: None,
                pred_upper: None,
            });
            continue;
        }
        let mean_set = solve_inverse_quadratic(ctx, y0, x_hat, k_mean);
        let cal_set = solve_inverse_quadratic(ctx, y0, x_hat, k_cal);
        let (ci_lower, ci_upper) = mean_set.to_ci_bounds();
        let (pred_lower, pred_upper) = match cal_set.finite_bounds() {
            Some((lo, hi)) => (Some(lo), Some(hi)),
            None => (None, None),
        };
        out.push(LinearInterpolation { input_value: y0, estimate: x_hat, ci_lower, ci_upper, pred_lower, pred_upper });
    }
    out
}

/// Predicted `Y` at each `X` with mean-response confidence and single-observation prediction
/// intervals (Draper & Smith 1998).
fn interpolate_y_list(unknown_xs: &[f64], ctx: &InverseQuadraticContext) -> Vec<LinearInterpolation> {
    let mut out = Vec::with_capacity(unknown_xs.len());
    for &x0 in unknown_xs {
        let y_hat = if x0.is_finite() { ctx.intercept + ctx.slope * x0 } else { f64::NAN };
        if !y_hat.is_finite() || !ctx.sy_x.is_finite() || !ctx.t_crit.is_finite() || ctx.s_xx <= 0.0 {
            out.push(LinearInterpolation {
                input_value: x0,
                estimate: f64::NAN,
                ci_lower: CiBound::Open,
                ci_upper: CiBound::Open,
                pred_lower: None,
                pred_upper: None,
            });
            continue;
        }
        let dx = x0 - ctx.x_center;
        let h = ctx.n_inv + (dx * dx) / ctx.s_xx;
        let ci_half = ctx.t_crit * ctx.sy_x * h.sqrt();
        let pi_half = ctx.t_crit * ctx.sy_x * (1.0 + h).sqrt();
        let (ci_lo, ci_hi) = (y_hat - ci_half, y_hat + ci_half);
        let (ci_lower, ci_upper) =
            if ci_lo.is_finite() && ci_hi.is_finite() { (CiBound::Finite(ci_lo), CiBound::Finite(ci_hi)) } else { (CiBound::Open, CiBound::Open) };
        let (pi_lo, pi_hi) = (y_hat - pi_half, y_hat + pi_half);
        let (pred_lower, pred_upper) = if pi_lo.is_finite() && pi_hi.is_finite() { (Some(pi_lo), Some(pi_hi)) } else { (None, None) };
        out.push(LinearInterpolation { input_value: x0, estimate: y_hat, ci_lower, ci_upper, pred_lower, pred_upper });
    }
    out
}

/// Compare slopes and intercepts (elevations) across `k >= 2` linear regression datasets via ANCOVA.
///
/// Follows the comparison of simple regression lines in Zar (2010): slopes are tested with
/// `F = ((SS_common - SS_separate) / (k - 1)) / (SS_separate / (N - 2k))`, where `SS_common` uses
/// the pooled within-group slope; if slopes are treated as equal, elevations are tested with
/// `F = ((SS_total - SS_common) / (k - 1)) / (SS_common / (N - k - 1))`, where `SS_total` is
/// the residual SS of one line fitted to all points. Through the origin only the slope test is
/// reported (`df = (k - 1, N - k)`). Significance flags use a fixed `0.05`.
///
/// # Errors
/// Returns [`FitError::InsufficientData`] if fewer than 2 datasets or too few points exist.
pub fn compare_linear_lines(datasets: &[&[DataPoint]], force_through_origin: bool) -> Result<LinearComparisonResult, FitError> {
    let k = datasets.len();
    if k < 2 {
        return Err(FitError::InsufficientData { required: 2, actual: k });
    }
    let min_per = if force_through_origin { 2 } else { 3 };
    let mut ss_unpooled = 0.0_f64;
    let mut n_total = 0_usize;
    let mut ss_within_total = 0.0_f64;
    let mut s_xx_pool = 0.0_f64;
    let mut s_xy_pool = 0.0_f64;
    let mut s_yy_pool = 0.0_f64;
    let mut all_xs = Vec::new();
    let mut all_ys = Vec::new();
    let mut all_ns = Vec::new();

    for &ds in datasets {
        let series = extract_linear_series(ds)?;
        if series.n_total < min_per {
            return Err(FitError::InsufficientData { required: min_per, actual: series.n_total });
        }
        let (slope, intercept, _, _, s_xx, s_yy, s_xy, _) = if force_through_origin {
            fit_through_origin(&series.xs, &series.ys, &series.ns, series.ss_within)?
        } else {
            fit_with_intercept(&series.xs, &series.ys, &series.ns, series.n_total, series.ss_within)?
        };
        let rss_j: f64 =
            series.xs.iter().zip(series.ys.iter()).zip(series.ns.iter()).map(|((&x, &y), &w)| w * (y - (intercept + slope * x)).powi(2)).sum::<f64>()
                + series.ss_within;
        ss_unpooled += rss_j;
        n_total += series.n_total;
        ss_within_total += series.ss_within;
        s_xx_pool += s_xx;
        s_xy_pool += s_xy;
        s_yy_pool += s_yy;
        all_xs.extend_from_slice(&series.xs);
        all_ys.extend_from_slice(&series.ys);
        all_ns.extend_from_slice(&series.ns);
    }

    let pooled_slope = s_xy_pool / s_xx_pool;
    let ss_common = (s_yy_pool - pooled_slope * pooled_slope * s_xx_pool).max(0.0);

    let df_num_s = k - 1;
    let df_den_s = if force_through_origin { n_total - k } else { n_total - 2 * k };
    if df_den_s == 0 {
        return Err(FitError::InsufficientData { required: n_total + 1, actual: n_total });
    }
    let ms_den_s = ss_unpooled / (df_den_s as f64);
    let ms_num_s = (ss_common - ss_unpooled).max(0.0) / (df_num_s as f64);
    let f_slopes = if ms_den_s > 0.0 {
        ms_num_s / ms_den_s
    } else if ms_num_s > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let p_slopes = if f_slopes.is_infinite() { 0.0 } else { f_sf(f_slopes, df_num_s as f64, df_den_s as f64)? };

    if force_through_origin {
        return Ok(LinearComparisonResult {
            dataset_count: k,
            f_slopes,
            df_slopes: (df_num_s, df_den_s),
            p_slopes,
            slopes_significantly_different: p_slopes < 0.05,
            pooled_slope,
            f_intercepts: None,
            df_intercepts: None,
            p_intercepts: None,
            intercepts_significantly_different: None,
        });
    }

    let (all_slope, all_int, _, _, _, _, _, _) = fit_with_intercept(&all_xs, &all_ys, &all_ns, n_total, ss_within_total)?;
    let ss_total_line: f64 =
        all_xs.iter().zip(all_ys.iter()).zip(all_ns.iter()).map(|((&x, &y), &w)| w * (y - (all_int + all_slope * x)).powi(2)).sum::<f64>()
            + ss_within_total;
    let df_den_i = n_total - k - 1;
    let ms_den_i = ss_common / (df_den_i as f64);
    let ms_num_i = (ss_total_line - ss_common).max(0.0) / (df_num_s as f64);
    let f_int = if ms_den_i > 0.0 {
        ms_num_i / ms_den_i
    } else if ms_num_i > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let p_int = if f_int.is_infinite() { 0.0 } else { f_sf(f_int, df_num_s as f64, df_den_i as f64)? };

    Ok(LinearComparisonResult {
        dataset_count: k,
        f_slopes,
        df_slopes: (df_num_s, df_den_s),
        p_slopes,
        slopes_significantly_different: p_slopes < 0.05,
        pooled_slope,
        f_intercepts: Some(f_int),
        df_intercepts: Some((df_num_s, df_den_i)),
        p_intercepts: Some(p_int),
        intercepts_significantly_different: Some(p_int < 0.05),
    })
}

/// Model II Deming regression for errors in both `X` and `Y` (`lambda = sigma_x^2 / sigma_y^2`,
/// where `lambda = 1.0` is Orthogonal / Major Axis regression), with Leave-One-Out jackknife SEs and CIs.
///
/// Unweighted Deming (1943) regression with jackknife standard errors and `t_{n-2}` intervals
/// as evaluated for method-comparison studies by Linnet (1993). Intervals are centred on the
/// full-sample estimates, not on the jackknife bias-corrected estimates. Weighted (constant-CV)
/// Deming regression is not implemented here.
///
/// # Errors
/// Returns [`FitError::InsufficientData`] if `points.len() < 3` or [`FitError::InvalidParameter`]
/// if `lambda <= 0` or `X`/`Y` covariance is degenerate.
pub fn deming_regression(points: &[DataPoint], opts: &DemingOptions) -> Result<DemingResult, FitError> {
    if points.is_empty() {
        return Err(FitError::EmptyInput);
    }
    if points.len() < 3 {
        return Err(FitError::InsufficientData { required: 3, actual: points.len() });
    }
    if !opts.variance_ratio_lambda.is_finite() || opts.variance_ratio_lambda <= 0.0 {
        return Err(FitError::InvalidParameter { name: "variance_ratio_lambda", reason: String::from("lambda must be finite and > 0") });
    }

    let xs: Vec<f64> = points.iter().map(DataPoint::x0).collect();
    let ys: Vec<f64> = points.iter().map(|p| p.y).collect();
    if xs.iter().chain(ys.iter()).any(|v| !v.is_finite()) {
        return Err(FitError::InvalidParameter { name: "xy", reason: String::from("non-finite value") });
    }
    let n = points.len();
    let (slope, intercept, sy_x) = fit_deming_core(&xs, &ys, opts.variance_ratio_lambda)?;

    // Leave-One-Out Jackknife SEs for slope & intercept.
    let mut jack_slopes = Vec::with_capacity(n);
    let mut jack_ints = Vec::with_capacity(n);
    let mut sub_x = Vec::with_capacity(n - 1);
    let mut sub_y = Vec::with_capacity(n - 1);
    for skip in 0..n {
        sub_x.clear();
        sub_y.clear();
        for i in 0..n {
            if i != skip {
                sub_x.push(xs[i]);
                sub_y.push(ys[i]);
            }
        }
        let (js, ji, _) = fit_deming_core(&sub_x, &sub_y, opts.variance_ratio_lambda)?;
        jack_slopes.push(js);
        jack_ints.push(ji);
    }

    let se_slope = jackknife_se(&jack_slopes);
    let se_intercept = jackknife_se(&jack_ints);
    let df = n - 2;
    let t_crit = student_t_two_sided_isf(opts.confidence_level.alpha(), df as f64)?;
    let ci_slope = (slope - t_crit * se_slope, slope + t_crit * se_slope);
    let ci_intercept = (intercept - t_crit * se_intercept, intercept + t_crit * se_intercept);

    let residuals = xs
        .iter()
        .zip(ys.iter())
        .map(|(&x, &y)| {
            let pred = intercept + slope * x;
            LinearResidual { x, y, predicted: pred, residual: y - pred }
        })
        .collect();

    Ok(DemingResult {
        n,
        df,
        lambda: opts.variance_ratio_lambda,
        slope,
        intercept,
        se_slope,
        se_intercept,
        ci_slope,
        ci_intercept,
        sy_x,
        x_intercept: if slope != 0.0 { Some(-intercept / slope) } else { None },
        residuals,
    })
}

/// Closed-form Deming regression (`lambda = sigma_x^2 / sigma_y^2`) minimizing
/// `sum [(y_i - Yhat_i)^2 + (x_i - Xhat_i)^2 / lambda] = sum (y_i - (b0 + b1 x_i))^2 / (1 + lambda b1^2)`
/// (Deming 1943, Ch. 9; Linnet 1990, eq. 2; Linnet 1993):
/// `b1 = ((lambda S_yy - S_xx) + sqrt((lambda S_yy - S_xx)^2 + 4 lambda S_xy^2)) / (2 lambda S_xy)`,
/// evaluated in overflow-safe cancellation-free form for both `lambda <= 1` and `lambda > 1` (via `1/lambda`).
///
/// As `lambda -> 0+` (`X` noiseless), `b1 -> S_xy / S_xx` (OLS of `Y` on `X`);
/// as `lambda -> +inf` (`Y` noiseless), `b1 -> S_yy / S_xy` (OLS of `X` on `Y`, inverted).
/// Residual SD `sy_x = sqrt(sum (y_i - yhat_i)^2 / ((1 + lambda b1^2) (n - 2)))` estimates `sigma_y`
/// and satisfies the swap identity `sy_x' = sqrt(lambda) sy_x` under `(x, y, lambda) <-> (y, x, 1/lambda)`.
fn fit_deming_core(xs: &[f64], ys: &[f64], lambda: f64) -> Result<(f64, f64, f64), FitError> {
    let mx = SummaryMoments::from_slice(xs)?;
    let my = SummaryMoments::from_slice(ys)?;
    let mut dx_acc = NeumaierSum::new();
    let mut dy_acc = NeumaierSum::new();
    let mut xy_acc = NeumaierSum::new();
    for (&x, &y) in xs.iter().zip(ys.iter()) {
        let dx = x - mx.mean;
        let dy = y - my.mean;
        dx_acc.add(dx);
        dy_acc.add(dy);
        let p = dx * dy;
        xy_acc.add(p);
        xy_acc.add(dx.mul_add(dy, -p));
    }
    let s_xx = mx.css;
    let s_yy = my.css;
    let s_xy = xy_acc.total() - (dx_acc.total() * dy_acc.total()) / (xs.len() as f64);
    if !s_xx.is_finite() || !s_yy.is_finite() || !s_xy.is_finite() {
        return Err(FitError::InvalidParameter { name: "data", reason: String::from("centered covariance overflowed f64 range") });
    }

    let slope = if s_xy == 0.0 {
        let vertical_axis = if lambda <= 1.0 { lambda * s_yy >= s_xx } else { s_yy >= s_xx / lambda };
        if vertical_axis {
            return Err(FitError::InvalidParameter { name: "data", reason: String::from("zero X-Y covariance with vertical Deming axis") });
        }
        0.0
    } else if lambda <= 1.0 {
        let u = lambda * s_yy - s_xx;
        let disc = u.hypot(2.0 * lambda.sqrt() * s_xy);
        if u >= 0.0 { (u + disc) / (2.0 * lambda * s_xy) } else { (2.0 * s_xy) / (-u + disc) }
    } else {
        // Reformulate with inv_l = 1/lambda = sigma_y^2 / sigma_x^2 in (0, 1) to avoid lambda * s_yy overflow.
        let inv_l = 1.0 / lambda;
        let u = s_yy - inv_l * s_xx;
        let disc = u.hypot(2.0 * inv_l.sqrt() * s_xy);
        if u >= 0.0 { (u + disc) / (2.0 * s_xy) } else { (2.0 * inv_l * s_xy) / (-u + disc) }
    };
    if !slope.is_finite() {
        return Err(FitError::InvalidParameter { name: "data", reason: String::from("Deming slope is non-finite") });
    }
    let intercept = my.mean - slope * mx.mean;

    let sy_x = if xs.len() > 2 {
        let scale = 1.0_f64.hypot(lambda.sqrt() * slope.abs());
        let mut d_ss = NeumaierSum::new();
        for (&x, &y) in xs.iter().zip(ys.iter()) {
            let r = (y - (intercept + slope * x)) / scale;
            d_ss.add(r * r);
        }
        (d_ss.total().max(0.0) / ((xs.len() - 2) as f64)).sqrt()
    } else {
        0.0
    };
    Ok((slope, intercept, sy_x))
}

/// Leave-one-out jackknife standard error `sqrt((n - 1)/n * sum (theta_(i) - theta_bar)^2)`
/// (Efron & Tibshirani 1993), equal to the SE of the mean of jackknife pseudo-values.
fn jackknife_se(vals: &[f64]) -> f64 {
    let n = vals.len();
    if n < 2 {
        return 0.0;
    }
    let Ok(m) = SummaryMoments::from_slice(vals) else {
        return f64::NAN;
    };
    let factor = ((n - 1) as f64) / (n as f64);
    (factor * m.css).sqrt()
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::CiBound;

    use super::{InverseIntervalSet, InverseQuadraticContext, fit_deming_core, solve_inverse_quadratic};

    #[test]
    fn solve_inverse_quadratic_exact_g_eq_1_and_extreme_regimes() {
        // Exact g = 1.0 rational unit test: a = 0, b = 1, x_c = 0, S_xx = 16, k0 in {0.25, 1.25}, t*s = 4 => g = 16 / 16 = 1.0.
        let ctx = InverseQuadraticContext {
            slope: 1.0,
            intercept: 0.0,
            sy_x: 2.0,
            t_crit: 2.0,
            n_inv: 0.25,
            x_center: 0.0,
            s_xx: 16.0,
            sqrt_sxx: 4.0,
            g: 1.0,
        };
        // d = 5, k_mean = 0.25 -> [0.5*(5 - 4/5), +inf) = [2.1, +inf)
        let s_pos_mean = solve_inverse_quadratic(&ctx, 5.0, 5.0, 0.25);
        assert_eq!(s_pos_mean, InverseIntervalSet::HalfLine { lower: Some(2.1), upper: None });
        assert_eq!(s_pos_mean.to_ci_bounds(), (CiBound::Finite(2.1), CiBound::Open));
        assert!(s_pos_mean.contains(2.1) && s_pos_mean.contains(1e6) && !s_pos_mean.contains(2.0));

        // d = 5, k_cal (m=1) = 1.25 -> [0.5*(5 - 20/5), +inf) = [0.5, +inf)
        let s_pos_cal = solve_inverse_quadratic(&ctx, 5.0, 5.0, 1.25);
        assert_eq!(s_pos_cal, InverseIntervalSet::HalfLine { lower: Some(0.5), upper: None });

        // d = -5, k_cal (m=1) = 1.25 -> (-inf, -0.5]
        let s_neg_cal = solve_inverse_quadratic(&ctx, -5.0, -5.0, 1.25);
        assert_eq!(s_neg_cal, InverseIntervalSet::HalfLine { lower: None, upper: Some(-0.5) });
        assert_eq!(s_neg_cal.to_ci_bounds(), (CiBound::Open, CiBound::Finite(-0.5)));
        assert!(s_neg_cal.contains(-0.5) && s_neg_cal.contains(-1e6) && !s_neg_cal.contains(0.0));

        // d = 0 => AllReals
        assert_eq!(solve_inverse_quadratic(&ctx, 0.0, 0.0, 1.25), InverseIntervalSet::AllReals);

        // Subtle g > 1 regime where (g - 1) S_xx k0 < d^2 < g S_xx k0: both rays sit on the negative side when d > 0!
        // Choose b = 1, a = 0, x_c = 0, S_xx = 16, t*s = 8 => g = 64 / 16 = 4.0, k0 = 1.0:
        // (g - 1)*16*1 = 48, g*16*1 = 64 => pick d = 7.5 (d^2 = 56.25 in (48, 64))!
        // Quadratic: 3 x^2 + 15 x + (64 - 56.25) = 3 x^2 + 15 x + 7.75 >= 0, roots (-15 +- sqrt(225 - 93))/6 = (-15 +- sqrt(132))/6 < 0!
        let ctx_gt = InverseQuadraticContext {
            slope: 1.0,
            intercept: 0.0,
            sy_x: 4.0,
            t_crit: 2.0,
            n_inv: 0.0,
            x_center: 0.0,
            s_xx: 16.0,
            sqrt_sxx: 4.0,
            g: 4.0,
        };
        let two_neg_rays = solve_inverse_quadratic(&ctx_gt, 7.5, 7.5, 1.0);
        let InverseIntervalSet::TwoRays { left_upper, right_lower } = two_neg_rays else {
            panic!("expected TwoRays, got {two_neg_rays:?}");
        };
        let r1 = (-15.0 - 132.0_f64.sqrt()) / 6.0;
        let r2 = (-15.0 + 132.0_f64.sqrt()) / 6.0;
        assert!((left_upper - r1).abs() < 1e-13);
        assert!((right_lower - r2).abs() < 1e-13);
        assert!(left_upper < right_lower && right_lower < 0.0);
        assert!(two_neg_rays.contains(left_upper) && two_neg_rays.contains(right_lower) && !two_neg_rays.contains(-2.0));

        // Overflow-safe extreme y0 = 1e200 for g < 1 and g > 1: must remain finite without NaN/Inf.
        let ctx_lt = InverseQuadraticContext {
            slope: 1.0,
            intercept: 0.0,
            sy_x: 1.0,
            t_crit: 2.0,
            n_inv: 0.25,
            x_center: 0.0,
            s_xx: 16.0,
            sqrt_sxx: 4.0,
            g: 0.25,
        };
        assert!(
            matches!(solve_inverse_quadratic(&ctx_lt, 1e200, 1e200, 1.25), InverseIntervalSet::Bounded { lower, upper } if lower.is_finite() && upper.is_finite())
        );
        assert!(
            matches!(solve_inverse_quadratic(&ctx_gt, 1e200, 1e200, 1.0), InverseIntervalSet::TwoRays { left_upper, right_lower } if left_upper.is_finite() && right_lower.is_finite())
        );

        // Deming extreme lambda = 1e100 / 1e-100: zero overflow/NaN.
        let xs = [1.0, 2.0, 3.0, 4.0];
        let ys = [2.1, 3.9, 6.2, 7.8];
        let (b_lo, _, syx_lo) = fit_deming_core(&xs, &ys, 1e-100).unwrap();
        let (b_hi, _, syx_hi) = fit_deming_core(&xs, &ys, 1e100).unwrap();
        assert!(b_lo.is_finite() && syx_lo.is_finite() && syx_lo > 0.0);
        assert!(b_hi.is_finite() && syx_hi.is_finite() && b_hi > b_lo);
    }
}
