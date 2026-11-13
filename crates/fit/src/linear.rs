//! Simple linear regression (OLS, origin-forced, bands, unknown interpolation, ANCOVA line
//! comparison) and Model II Deming / Orthogonal regression with jackknife confidence intervals.

use lumencraft_doc::{CiBound, CurveSamplePoint};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{f_sf, student_t_two_sided_isf};
use lumencraft_math::sum::{NeumaierSum, SummaryMoments};

use crate::diagnostics::{LackOfFitResult, RunsTestResult, replicates_lack_of_fit, runs_test};
use crate::error::FitError;
use crate::types::DataPoint;

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

/// Interpolated `X` from `Y` (or `Y` from `X`) with confidence / prediction interval.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinearInterpolation {
    pub input_value: f64,
    pub estimate: f64,
    pub ci_lower: CiBound,
    pub ci_upper: CiBound,
    pub pred_lower: Option<f64>,
    pub pred_upper: Option<f64>,
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

/// Perform simple OLS linear regression (or origin-forced linear regression) on `(x, y)` pairs.
///
/// # Errors
/// Returns [`FitError::InsufficientData`] if too few observations are given, or
/// [`FitError::InvalidParameter`] if `X` has zero variance (or all `X == 0` when forced through origin).
pub fn simple_linear_regression(points: &[DataPoint], opts: &SimpleLinearOptions) -> Result<SimpleLinearResult, FitError> {
    let min_n = if opts.force_through_origin { 2 } else { 3 };
    if points.is_empty() {
        return Err(FitError::EmptyInput);
    }
    if points.len() < min_n {
        return Err(FitError::InsufficientData { required: min_n, actual: points.len() });
    }
    let xs: Vec<f64> = points.iter().map(DataPoint::x0).collect();
    let ys: Vec<f64> = points.iter().map(|p| p.y).collect();
    let n = points.len();

    let (slope, intercept, se_slope, se_intercept, s_xx, s_yy, s_xy, x_mean) =
        if opts.force_through_origin { fit_through_origin(&xs, &ys)? } else { fit_with_intercept(&xs, &ys)? };

    let df_res = if opts.force_through_origin { n - 1 } else { n - 2 };
    let mut rss_acc = NeumaierSum::new();
    let mut residuals = Vec::with_capacity(n);
    let mut raw_res = Vec::with_capacity(n);
    for (&x, &y) in xs.iter().zip(ys.iter()) {
        let pred = intercept + slope * x;
        let res = y - pred;
        rss_acc.add(res * res);
        raw_res.push(res);
        residuals.push(LinearResidual { x, y, predicted: pred, residual: res });
    }
    let rss = rss_acc.total().max(0.0);
    let sy_x = (rss / (df_res as f64)).sqrt();

    let (se_slope, se_intercept) = if opts.force_through_origin { (sy_x * se_slope, 0.0) } else { (sy_x * se_slope, sy_x * se_intercept) };

    let t_crit = student_t_two_sided_isf(opts.confidence_level.alpha(), df_res as f64)?;
    let ci_slope = (slope - t_crit * se_slope, slope + t_crit * se_slope);
    let ci_intercept = if opts.force_through_origin { (0.0, 0.0) } else { (intercept - t_crit * se_intercept, intercept + t_crit * se_intercept) };

    let r_squared = if s_yy > 0.0 { (1.0 - rss / s_yy).clamp(0.0, 1.0) } else { 1.0 };
    let r = if s_xx > 0.0 && s_yy > 0.0 { (s_xy / (s_xx * s_yy).sqrt()).clamp(-1.0, 1.0) } else { 0.0 };
    let ms_res = rss / (df_res as f64);
    let ss_reg = (s_yy - rss).max(0.0);
    let f_stat = if ms_res > 0.0 {
        ss_reg / ms_res
    } else if ss_reg > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let p_value = if f_stat.is_infinite() { 0.0 } else { f_sf(f_stat, 1.0, df_res as f64).unwrap_or(1.0) };

    // Sort residuals by X for runs test so sign ordering tracks predictor progression.
    let mut ordered_pairs: Vec<(f64, f64)> = xs.iter().copied().zip(raw_res.iter().copied()).collect();
    ordered_pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let sorted_res: Vec<f64> = ordered_pairs.into_iter().map(|(_, r)| r).collect();
    let runs = runs_test(&sorted_res);
    let lack_of_fit = replicates_lack_of_fit(points, rss, df_res);

    let bands = if opts.compute_bands {
        build_linear_bands(&xs, slope, intercept, sy_x, t_crit, x_mean, s_xx, opts.force_through_origin, opts.band_grid_points)
    } else {
        Vec::new()
    };

    let interpolated_x_from_y =
        interpolate_x_list(&opts.unknown_y_values, slope, intercept, sy_x, t_crit, n, x_mean, s_xx, opts.force_through_origin);
    let interpolated_y_from_x =
        interpolate_y_list(&opts.unknown_x_values, slope, intercept, sy_x, t_crit, n, x_mean, s_xx, opts.force_through_origin);

    Ok(SimpleLinearResult {
        n,
        df_residual: df_res,
        force_through_origin: opts.force_through_origin,
        slope,
        intercept,
        se_slope,
        se_intercept,
        ci_slope,
        ci_intercept,
        r_squared,
        r,
        sy_x,
        rss,
        ss_total: s_yy,
        f_stat,
        p_value,
        x_intercept: if slope != 0.0 { Some(-intercept / slope) } else { None },
        one_over_slope: if slope != 0.0 { Some(1.0 / slope) } else { None },
        runs_test: runs,
        lack_of_fit,
        bands,
        interpolated_x_from_y,
        interpolated_y_from_x,
        residuals,
    })
}

#[allow(clippy::type_complexity)]
fn fit_with_intercept(xs: &[f64], ys: &[f64]) -> Result<(f64, f64, f64, f64, f64, f64, f64, f64), FitError> {
    let mx = SummaryMoments::from_slice(xs)?;
    let my = SummaryMoments::from_slice(ys)?;
    if mx.css <= 0.0 {
        return Err(FitError::InvalidParameter { name: "x", reason: String::from("predictor X has zero variance") });
    }
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
    let s_xy = xy_acc.total() - (dx_acc.total() * dy_acc.total()) / (xs.len() as f64);
    let slope = s_xy / mx.css;
    let intercept = my.mean - slope * mx.mean;
    let se_slope_factor = (1.0 / mx.css).sqrt();
    let se_intercept_factor = (1.0 / (xs.len() as f64) + (mx.mean * mx.mean) / mx.css).sqrt();
    Ok((slope, intercept, se_slope_factor, se_intercept_factor, mx.css, my.css, s_xy, mx.mean))
}

#[allow(clippy::type_complexity)]
fn fit_through_origin(xs: &[f64], ys: &[f64]) -> Result<(f64, f64, f64, f64, f64, f64, f64, f64), FitError> {
    let mut s_xx_acc = NeumaierSum::new();
    let mut s_yy_acc = NeumaierSum::new();
    let mut s_xy_acc = NeumaierSum::new();
    for (&x, &y) in xs.iter().zip(ys.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return Err(FitError::InvalidParameter { name: "xy", reason: String::from("non-finite value") });
        }
        s_xx_acc.add(x * x);
        s_yy_acc.add(y * y);
        s_xy_acc.add(x * y);
    }
    let s_xx = s_xx_acc.total();
    let s_yy = s_yy_acc.total();
    let s_xy = s_xy_acc.total();
    if s_xx <= 0.0 {
        return Err(FitError::InvalidParameter { name: "x", reason: String::from("sum of X^2 must be positive for origin-forced regression") });
    }
    let slope = s_xy / s_xx;
    let se_slope_factor = (1.0 / s_xx).sqrt();
    Ok((slope, 0.0, se_slope_factor, 0.0, s_xx, s_yy, s_xy, 0.0))
}

#[allow(clippy::too_many_arguments)]
fn build_linear_bands(
    xs: &[f64],
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
    let n_inv = if origin { 0.0 } else { 1.0 / (xs.len() as f64) };
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

#[allow(clippy::too_many_arguments)]
fn interpolate_x_list(
    unknown_ys: &[f64],
    slope: f64,
    intercept: f64,
    sy_x: f64,
    t_crit: f64,
    n: usize,
    x_mean: f64,
    s_xx: f64,
    origin: bool,
) -> Vec<LinearInterpolation> {
    let mut out = Vec::with_capacity(unknown_ys.len());
    let g = if slope != 0.0 && s_xx > 0.0 { (t_crit * sy_x).powi(2) / (slope * slope * s_xx) } else { f64::INFINITY };
    let n_inv = if origin { 0.0 } else { 1.0 / (n as f64) };

    for &y0 in unknown_ys {
        if !y0.is_finite() || slope == 0.0 {
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
        let x_hat = (y0 - intercept) / slope;
        let (ci_lower, ci_upper) = if g < 1.0 {
            let dx = if origin { x_hat } else { x_hat - x_mean };
            let x_mid = if origin { x_hat / (1.0 - g) } else { x_mean + dx / (1.0 - g) };
            let rad = ((1.0 - g) * n_inv + (dx * dx) / s_xx).max(0.0).sqrt();
            let half = t_crit * sy_x * rad / (slope.abs() * (1.0 - g));
            (CiBound::Finite(x_mid - half), CiBound::Finite(x_mid + half))
        } else {
            (CiBound::Open, CiBound::Open)
        };
        out.push(LinearInterpolation { input_value: y0, estimate: x_hat, ci_lower, ci_upper, pred_lower: None, pred_upper: None });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn interpolate_y_list(
    unknown_xs: &[f64],
    slope: f64,
    intercept: f64,
    sy_x: f64,
    t_crit: f64,
    n: usize,
    x_mean: f64,
    s_xx: f64,
    origin: bool,
) -> Vec<LinearInterpolation> {
    let n_inv = if origin { 0.0 } else { 1.0 / (n as f64) };
    let mut out = Vec::with_capacity(unknown_xs.len());
    for &x0 in unknown_xs {
        let y_hat = intercept + slope * x0;
        let dx = if origin { x0 } else { x0 - x_mean };
        let h = n_inv + (dx * dx) / s_xx;
        let ci_half = t_crit * sy_x * h.sqrt();
        let pi_half = t_crit * sy_x * (1.0 + h).sqrt();
        out.push(LinearInterpolation {
            input_value: x0,
            estimate: y_hat,
            ci_lower: CiBound::Finite(y_hat - ci_half),
            ci_upper: CiBound::Finite(y_hat + ci_half),
            pred_lower: Some(y_hat - pi_half),
            pred_upper: Some(y_hat + pi_half),
        });
    }
    out
}

/// Compare slopes and intercepts (elevations) across `k >= 2` linear regression datasets via ANCOVA.
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
    let mut s_xx_pool = 0.0_f64;
    let mut s_xy_pool = 0.0_f64;
    let mut s_yy_pool = 0.0_f64;
    let mut all_points = Vec::new();

    for &ds in datasets {
        if ds.len() < min_per {
            return Err(FitError::InsufficientData { required: min_per, actual: ds.len() });
        }
        let xs: Vec<f64> = ds.iter().map(DataPoint::x0).collect();
        let ys: Vec<f64> = ds.iter().map(|p| p.y).collect();
        let (slope, intercept, _, _, s_xx, s_yy, s_xy, _) =
            if force_through_origin { fit_through_origin(&xs, &ys)? } else { fit_with_intercept(&xs, &ys)? };
        let rss_j: f64 = xs.iter().zip(ys.iter()).map(|(&x, &y)| (y - (intercept + slope * x)).powi(2)).sum();
        ss_unpooled += rss_j;
        n_total += ds.len();
        s_xx_pool += s_xx;
        s_xy_pool += s_xy;
        s_yy_pool += s_yy;
        all_points.extend_from_slice(ds);
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

    let all_xs: Vec<f64> = all_points.iter().map(DataPoint::x0).collect();
    let all_ys: Vec<f64> = all_points.iter().map(|p| p.y).collect();
    let (all_slope, all_int, _, _, _, _, _, _) = fit_with_intercept(&all_xs, &all_ys)?;
    let ss_total_line: f64 = all_xs.iter().zip(all_ys.iter()).map(|(&x, &y)| (y - (all_int + all_slope * x)).powi(2)).sum();
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

    let slope = if s_xy == 0.0 {
        if s_yy >= lambda * s_xx {
            return Err(FitError::InvalidParameter { name: "data", reason: String::from("zero X-Y covariance with vertical Deming axis") });
        }
        0.0
    } else {
        let u = s_yy - lambda * s_xx;
        let disc = (u * u + 4.0 * lambda * s_xy * s_xy).sqrt();
        if u >= 0.0 { (u + disc) / (2.0 * s_xy) } else { (2.0 * lambda * s_xy) / (-u + disc) }
    };
    let intercept = my.mean - slope * mx.mean;

    // Deming perpendicular-scaled residual SD: sum (y_i - (b0 + b1 x_i))^2 / (1 + b1^2 / lambda) / (n - 2).
    let denom = 1.0 + (slope * slope) / lambda;
    let mut d_ss = NeumaierSum::new();
    for (&x, &y) in xs.iter().zip(ys.iter()) {
        let r = y - (intercept + slope * x);
        d_ss.add((r * r) / denom);
    }
    let sy_x = (d_ss.total().max(0.0) / ((xs.len() - 2) as f64)).sqrt();
    Ok((slope, intercept, sy_x))
}

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
