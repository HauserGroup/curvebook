//! Model-free XY curve fitting: Natural Cubic Spline, Akima Subspline, Reinsch Smoothing Spline,
//! Cleveland LOWESS, dense data-space Point-to-Point resampling, and nonparametric standard-curve
//! unknown interpolation (`docs/implementation/curve-fitting-implementation.md` §6.1).
//!
//! References:
//! - de Boor, C. (1978). *A Practical Guide to Splines*. Applied Mathematical Sciences, Vol. 27,
//!   Springer-Verlag. `doi:10.1007/978-1-4612-6333-3`.
//! - Akima, H. (1970). A new method of interpolation and smooth curve fitting based on local
//!   procedures. *Journal of the ACM*, 17(4), 589-602. `doi:10.1145/321607.321609`.
//! - Reinsch, C. H. (1967). Smoothing by spline functions. *Numerische Mathematik*, 10(3),
//!   177-183. `doi:10.1007/bf02162161`.
//! - Hutchinson, M. F., & de Hoog, F. R. (1985). Smoothing noisy data with spline functions.
//!   *Numerische Mathematik*, 47(1), 99-106. `doi:10.1007/bf01389878`.
//! - Cleveland, W. S. (1979). Robust locally weighted regression and smoothing scatterplots.
//!   *Journal of the American Statistical Association*, 74(368), 829-836.
//!   `doi:10.1080/01621459.1979.10481038`.
//! - Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.

use curvebook_doc::{
    AnalysisSpec, Cell, ChecklistItem, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay, ResultCell, ResultScalarEntry, ResultSheet,
    ResultStatus, ResultTab, ResultTabKind, SplineLowessMethod, SplineLowessSpec, TableArchetype, XyTable,
};
use curvebook_math::{DenseMatrix, solve_linear_least_squares, sum};

use crate::error::TransformError;

/// Single piecewise cubic polynomial `a + b*u + c*u^2 + d*u^3` on `[x0, x1]` with `u = x - x0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicPiece {
    /// Left knot abscissa `x_i`.
    pub x0: f64,
    /// Right knot abscissa `x_{i+1}`.
    pub x1: f64,
    /// Constant coefficient `S(x_i)`.
    pub a: f64,
    /// Linear coefficient `S'(x_i)`.
    pub b: f64,
    /// Quadratic coefficient `S''(x_i) / 2`.
    pub c: f64,
    /// Cubic coefficient `S'''(x_i) / 6`.
    pub d: f64,
}

impl CubicPiece {
    /// Evaluate the cubic polynomial at `x` via Horner's scheme.
    #[must_use]
    pub fn eval(&self, x: f64) -> f64 {
        let u = x - self.x0;
        ((self.d * u + self.c) * u + self.b) * u + self.a
    }

    /// Evaluate the first derivative `S'(x)`.
    #[must_use]
    pub fn deriv(&self, x: f64) -> f64 {
        let u = x - self.x0;
        (3.0 * self.d * u + 2.0 * self.c) * u + self.b
    }
}

/// Fitted model-free XY curve supporting evaluation and inverse root-finding.
#[derive(Debug, Clone, PartialEq)]
pub enum FittedCurveModel {
    /// Piecewise cubic spline (`NaturalCubicSpline`, `AkimaSubspline`, or `SmoothingSpline`).
    PiecewiseCubic {
        /// Ordered cubic pieces covering `[x_min, x_max]`.
        pieces: Vec<CubicPiece>,
        /// Boundary slope at `x_min` used for linear extrapolation when `x < x_min`.
        left_slope: f64,
        /// Boundary slope at `x_max` used for linear extrapolation when `x > x_max`.
        right_slope: f64,
        /// Lower knot bound `x_0`.
        x_min: f64,
        /// Upper knot bound `x_{m-1}`.
        x_max: f64,
    },
    /// Fitted Cleveland (1979) LOWESS local linear smoother with finalized robustness weights.
    Lowess {
        /// Sorted abscissas of the training observations.
        xs: Vec<f64>,
        /// Corresponding ordinates of the training observations.
        ys: Vec<f64>,
        /// Finalized Tukey bisquare robustness weights at each observation.
        rob_weights: Vec<f64>,
        /// Neighborhood size `q = clamp(round(f * N), 2, N)`.
        q: usize,
        /// LOWESS fitted values at each training abscissa.
        fitted_y: Vec<f64>,
    },
    /// Piecewise linear interpolant through sorted distinct knots `(x_g, y_bar_g)`.
    PiecewiseLinear {
        /// Strictly increasing knot abscissas.
        xs: Vec<f64>,
        /// Knot ordinates.
        ys: Vec<f64>,
    },
}

/// Complete output of [`fit_smoothing_spline`] including Reinsch smoothing diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct SmoothingSplineFit {
    /// Fitted natural cubic smoothing spline model.
    pub model: FittedCurveModel,
    /// Achieved effective degrees of freedom `tr(S_lambda)` in `[2, G]`.
    pub effective_df: f64,
    /// Penalty weight `lambda >= 0` solving `tr(S_lambda) = target_df`.
    pub lambda: f64,
    /// Weighted residual sum of squares `sum_g w_g (y_bar_g - S(x_g))^2`.
    pub weighted_rss: f64,
}

impl FittedCurveModel {
    /// Domain `[x_min, x_max]` spanned by the fitted data knots.
    #[must_use]
    pub fn domain(&self) -> (f64, f64) {
        match self {
            Self::PiecewiseCubic { x_min, x_max, .. } => (*x_min, *x_max),
            Self::Lowess { xs, .. } | Self::PiecewiseLinear { xs, .. } => {
                let lo = xs.first().copied().unwrap_or(0.0);
                let hi = xs.last().copied().unwrap_or(lo);
                (lo, hi)
            }
        }
    }

    /// Evaluate the fitted curve `S(x)` at an arbitrary finite `x`.
    #[must_use]
    pub fn eval(&self, x: f64) -> f64 {
        match self {
            Self::PiecewiseCubic { pieces, left_slope, right_slope, x_min, x_max } => {
                let Some(first) = pieces.first() else {
                    return f64::NAN;
                };
                if x < *x_min {
                    return first.a + left_slope * (x - *x_min);
                }
                if x > *x_max {
                    let Some(last) = pieces.last() else {
                        return f64::NAN;
                    };
                    return last.eval(*x_max) + right_slope * (x - *x_max);
                }
                let idx = pieces.partition_point(|p| p.x1 < x).min(pieces.len().saturating_sub(1));
                pieces[idx].eval(x)
            }
            Self::Lowess { xs, ys, rob_weights, q, .. } => eval_lowess_at(xs, ys, rob_weights, *q, x),
            Self::PiecewiseLinear { xs, ys } => eval_piecewise_linear(xs, ys, x),
        }
    }

    /// Solve `S(x) = y_target` for all real roots `x` inside `[x_lo, x_hi]`, sorted ascending and deduplicated.
    #[must_use]
    pub fn solve_inverse(&self, y_target: f64, x_lo: f64, x_hi: f64) -> Vec<f64> {
        if !y_target.is_finite() || !x_lo.is_finite() || !x_hi.is_finite() || x_hi < x_lo {
            return Vec::new();
        }
        let span = (x_hi - x_lo).abs().max(1.0);
        let dedup_tol = 1e-9 * span;
        let mut roots = Vec::new();

        match self {
            Self::PiecewiseCubic { pieces, .. } => {
                for piece in pieces {
                    let seg_lo = piece.x0.max(x_lo);
                    let seg_hi = piece.x1.min(x_hi);
                    if seg_hi < seg_lo {
                        continue;
                    }
                    solve_cubic_piece_roots(piece, y_target, seg_lo, seg_hi, &mut roots);
                }
            }
            Self::PiecewiseLinear { xs, ys } => {
                for i in 0..xs.len().saturating_sub(1) {
                    let (xa, xb) = (xs[i], xs[i + 1]);
                    let seg_lo = xa.max(x_lo);
                    let seg_hi = xb.min(x_hi);
                    if seg_hi < seg_lo || xb <= xa {
                        continue;
                    }
                    let ya = eval_piecewise_linear(xs, ys, seg_lo) - y_target;
                    let yb = eval_piecewise_linear(xs, ys, seg_hi) - y_target;
                    if ya == 0.0 {
                        roots.push(seg_lo);
                    } else if yb == 0.0 {
                        roots.push(seg_hi);
                    } else if ya * yb < 0.0 {
                        let frac = -ya / (yb - ya);
                        let xr = seg_lo + frac * (seg_hi - seg_lo);
                        if xr.is_finite() {
                            roots.push(xr);
                        }
                    }
                }
            }
            Self::Lowess { xs, .. } => {
                let n_steps = (4 * xs.len()).clamp(200, 2_000);
                let step = (x_hi - x_lo) / (n_steps as f64);
                let mut prev_x = x_lo;
                let mut prev_f = self.eval(prev_x) - y_target;
                if prev_f.abs() <= 1e-12 {
                    roots.push(prev_x);
                }
                for k in 1..=n_steps {
                    let curr_x = if k == n_steps { x_hi } else { x_lo + (k as f64) * step };
                    let curr_f = self.eval(curr_x) - y_target;
                    if curr_f.abs() <= 1e-12 {
                        roots.push(curr_x);
                    } else if prev_f.is_finite()
                        && curr_f.is_finite()
                        && prev_f * curr_f < 0.0
                        && let Some(r) = brent_root(|x| self.eval(x) - y_target, prev_x, curr_x, prev_f, curr_f)
                    {
                        roots.push(r);
                    }
                    prev_x = curr_x;
                    prev_f = curr_f;
                }
            }
        }

        roots.retain(|r| r.is_finite() && *r >= x_lo - dedup_tol && *r <= x_hi + dedup_tol);
        roots.sort_by(f64::total_cmp);
        let mut deduped: Vec<f64> = Vec::with_capacity(roots.len());
        for r in roots {
            if deduped.last().is_none_or(|&prev| (r - prev).abs() > dedup_tol) {
                deduped.push(r.clamp(x_lo, x_hi));
            }
        }
        deduped
    }
}

/// Fit a natural cubic spline (`S''(x_0) = S''(x_{m-1}) = 0`; de Boor 1978) through sorted distinct `(x_g, y_bar_g)` knots.
pub fn fit_natural_cubic_spline(points: &[(f64, f64)]) -> Result<FittedCurveModel, TransformError> {
    let knots = collapse_distinct_knots(points)?;
    if knots.len() < 2 {
        return Err(TransformError::InvalidInput { message: String::from("natural cubic spline requires at least 2 distinct finite X values") });
    }
    let xs: Vec<f64> = knots.iter().map(|k| k.0).collect();
    let ys: Vec<f64> = knots.iter().map(|k| k.1).collect();
    build_natural_cubic_from_knots(&xs, &ys)
}

/// Fit an Akima subspline (`C^1` piecewise cubic with 5-point secant slope weighting; Akima 1970) through sorted distinct `(x_g, y_bar_g)` knots.
pub fn fit_akima_subspline(points: &[(f64, f64)]) -> Result<FittedCurveModel, TransformError> {
    let knots = collapse_distinct_knots(points)?;
    let m = knots.len();
    if m < 2 {
        return Err(TransformError::InvalidInput { message: String::from("Akima subspline requires at least 2 distinct finite X values") });
    }
    let xs: Vec<f64> = knots.iter().map(|k| k.0).collect();
    let ys: Vec<f64> = knots.iter().map(|k| k.1).collect();
    if m == 2 {
        return build_linear_cubic_piece(xs[0], ys[0], xs[1], ys[1]);
    }

    let n_seg = m - 1;
    let mut sec = vec![0.0_f64; n_seg + 4];
    for i in 0..n_seg {
        sec[i + 2] = (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]);
    }
    // Boundary chord extensions per Akima (1970) Eq. (5)-(6).
    sec[1] = 2.0 * sec[2] - sec[3];
    sec[0] = 2.0 * sec[1] - sec[2];
    sec[n_seg + 2] = 2.0 * sec[n_seg + 1] - sec[n_seg];
    sec[n_seg + 3] = 2.0 * sec[n_seg + 2] - sec[n_seg + 1];

    let mut slopes = vec![0.0_f64; m];
    for i in 0..m {
        let d_m2 = sec[i];
        let d_m1 = sec[i + 1];
        let d_0 = sec[i + 2];
        let d_p1 = sec[i + 3];
        let w1 = (d_p1 - d_0).abs();
        let w2 = (d_m1 - d_m2).abs();
        let denom = w1 + w2;
        slopes[i] = if denom > 0.0 { (w1 * d_m1 + w2 * d_0) / denom } else { 0.5 * (d_m1 + d_0) };
    }

    let mut pieces = Vec::with_capacity(n_seg);
    for i in 0..n_seg {
        let h = xs[i + 1] - xs[i];
        let delta = sec[i + 2];
        let t0 = slopes[i];
        let t1 = slopes[i + 1];
        let c = (3.0 * delta - 2.0 * t0 - t1) / h;
        let d = (t0 + t1 - 2.0 * delta) / (h * h);
        pieces.push(CubicPiece { x0: xs[i], x1: xs[i + 1], a: ys[i], b: t0, c, d });
    }
    let left_slope = slopes[0];
    let right_slope = slopes[m - 1];
    Ok(FittedCurveModel::PiecewiseCubic { pieces, left_slope, right_slope, x_min: xs[0], x_max: xs[m - 1] })
}

/// Fit a Reinsch (1967) penalized cubic smoothing spline minimizing
/// `sum_g w_g (y_bar_g - S(x_g))^2 + lambda * integral [S''(x)]^2 dx`
/// parameterized by target effective degrees of freedom `df = tr(S_lambda)` in `[2, G]`
/// (using Hutchinson & de Hoog 1985 `O(G)` banded Takahashi trace recursion).
pub fn fit_smoothing_spline(points: &[(f64, f64)], weights: Option<&[f64]>, target_df: Option<f64>) -> Result<SmoothingSplineFit, TransformError> {
    let knots = collapse_weighted_knots(points, weights)?;
    let m = knots.len();
    if m < 2 {
        return Err(TransformError::InvalidInput { message: String::from("smoothing spline requires at least 2 distinct finite X values") });
    }
    let xs: Vec<f64> = knots.iter().map(|k| k.0).collect();
    let ys: Vec<f64> = knots.iter().map(|k| k.1).collect();
    let ws: Vec<f64> = knots.iter().map(|k| k.2).collect();

    if m == 2 {
        let model = build_linear_cubic_piece(xs[0], ys[0], xs[1], ys[1])?;
        return Ok(SmoothingSplineFit { model, effective_df: 2.0, lambda: 0.0, weighted_rss: 0.0 });
    }

    let m_f = m as f64;
    let df_goal = match target_df {
        Some(df) => {
            if !df.is_finite() || df < 2.0 {
                return Err(TransformError::InvalidSpec { message: format!("smoothing spline df must be finite and >= 2, got {df}") });
            }
            df.min(m_f)
        }
        None => (0.5 * m_f).clamp(2.0, m_f.min(10.0)),
    };

    if df_goal >= m_f - 1e-9 {
        let model = build_natural_cubic_from_knots(&xs, &ys)?;
        return Ok(SmoothingSplineFit { model, effective_df: m_f, lambda: 0.0, weighted_rss: 0.0 });
    }
    if df_goal <= 2.0 + 1e-9 {
        let (intercept, slope) = weighted_ols_line(&xs, &ys, &ws)?;
        let fitted: Vec<f64> = xs.iter().map(|&x| intercept + slope * x).collect();
        let rss = weighted_rss(&ys, &fitted, &ws);
        let model = build_linear_cubic_piece(xs[0], fitted[0], xs[m - 1], fitted[m - 1])?;
        return Ok(SmoothingSplineFit { model, effective_df: 2.0, lambda: f64::MAX, weighted_rss: rss });
    }

    let reinsch = ReinschSystem::build(&xs, &ys, &ws)?;
    let (lambda, achieved_df) = reinsch.solve_lambda_for_df(df_goal)?;
    let (g_vals, gammas) = reinsch.solve_for_lambda(lambda)?;
    let model = build_cubic_from_values_and_second_derivs(&xs, &g_vals, &gammas)?;
    let rss = weighted_rss(&ys, &g_vals, &ws);
    Ok(SmoothingSplineFit { model, effective_df: achieved_df, lambda, weighted_rss: rss })
}

/// Fit a Cleveland (1979) LOWESS local linear smoother with tricube kernel and Tukey bisquare robustness passes.
pub fn fit_lowess_curve(points: &[(f64, f64)], fraction: f64, robustness_iters: usize) -> Result<FittedCurveModel, TransformError> {
    if !fraction.is_finite() || fraction <= 0.0 || fraction > 1.0 {
        return Err(TransformError::InvalidSpec { message: format!("LOWESS fraction must be finite and in (0, 1], got {fraction}") });
    }
    if robustness_iters > SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS {
        return Err(TransformError::InvalidSpec {
            message: format!("LOWESS robustness_iters must be <= {}, got {robustness_iters}", SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS),
        });
    }
    let mut sorted = validate_and_sort_points(points)?;
    let n = sorted.len();
    if n < 2 {
        return Err(TransformError::InvalidInput { message: String::from("LOWESS requires at least 2 finite observations") });
    }
    let xs: Vec<f64> = sorted.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = sorted.drain(..).map(|p| p.1).collect();
    if (xs[n - 1] - xs[0]).abs() <= 0.0 {
        return Err(TransformError::InvalidInput { message: String::from("LOWESS requires at least 2 distinct X values") });
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let q = ((fraction * (n as f64)).round() as usize).clamp(2, n);
    let mut rob_weights = vec![1.0_f64; n];
    let mut fitted_y = ys.clone();

    for iter in 0..=robustness_iters {
        for i in 0..n {
            fitted_y[i] = eval_lowess_at(&xs, &ys, &rob_weights, q, xs[i]);
        }
        if iter < robustness_iters {
            update_bisquare_weights(&ys, &fitted_y, &mut rob_weights);
        }
    }

    Ok(FittedCurveModel::Lowess { xs, ys, rob_weights, q, fitted_y })
}

/// Fit a data-space piecewise-linear curve through sorted distinct `(x_g, y_bar_g)` knots.
pub fn fit_point_to_point(points: &[(f64, f64)]) -> Result<FittedCurveModel, TransformError> {
    let knots = collapse_distinct_knots(points)?;
    if knots.len() < 2 {
        return Err(TransformError::InvalidInput { message: String::from("point-to-point curve requires at least 2 distinct finite X values") });
    }
    let xs: Vec<f64> = knots.iter().map(|k| k.0).collect();
    let ys: Vec<f64> = knots.iter().map(|k| k.1).collect();
    Ok(FittedCurveModel::PiecewiseLinear { xs, ys })
}

/// Sample `segment_count` polyline vertices uniformly across `[x_min, x_max]`.
pub fn sample_curve(model: &FittedCurveModel, x_min: f64, x_max: f64, segment_count: usize) -> Result<Vec<(f64, f64)>, TransformError> {
    if !x_min.is_finite() || !x_max.is_finite() || x_max <= x_min {
        return Err(TransformError::InvalidSpec { message: format!("curve evaluation domain requires x_min < x_max, got [{x_min}, {x_max}]") });
    }
    if !(SplineLowessSpec::MIN_SEGMENTS..=SplineLowessSpec::MAX_SEGMENTS).contains(&segment_count) {
        return Err(TransformError::InvalidSpec {
            message: format!("segment_count must be in {}..={}, got {segment_count}", SplineLowessSpec::MIN_SEGMENTS, SplineLowessSpec::MAX_SEGMENTS),
        });
    }
    let mut out = Vec::with_capacity(segment_count);
    let denom = (segment_count - 1) as f64;
    for i in 0..segment_count {
        let x = if i == 0 {
            x_min
        } else if i + 1 == segment_count {
            x_max
        } else {
            x_min + (x_max - x_min) * ((i as f64) / denom)
        };
        let y = model.eval(x);
        if !y.is_finite() {
            return Err(TransformError::InvalidInput { message: format!("curve evaluation produced non-finite value at X = {x}") });
        }
        out.push((x, y));
    }
    Ok(out)
}

/// Execute a [`SplineLowessSpec`] analysis against an `XY` [`DataSheet`] and return a populated [`ResultSheet`].
pub fn execute_spline_lowess(source: &DataSheet, analysis: &AnalysisSpec, spec: &SplineLowessSpec) -> Result<ResultSheet, TransformError> {
    let TableArchetype::Xy(xy) = &source.archetype else {
        return Err(TransformError::IncompatibleTable { operation: "Fit Spline / LOWESS", kind: source.kind() });
    };
    validate_spline_spec(spec)?;

    let active_cols = resolve_active_columns(xy.data_sets.len(), &analysis.selected_data_sets)?;
    if active_cols.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("XY table has no datasets to fit") });
    }

    let mut sheet = ResultSheet::new(analysis.clone())?;
    sheet.status = ResultStatus::UpToDate;

    let mut summary_cols = Vec::with_capacity(active_cols.len());
    let mut series_summaries: Vec<SeriesFitSummary> = Vec::with_capacity(active_cols.len());
    let mut coord_series: Vec<(String, Vec<(f64, f64)>)> = Vec::new();
    let mut interp_rows: Vec<Vec<ResultCell>> = Vec::new();
    let mut interp_row_labels: Vec<String> = Vec::new();

    for &ds_idx in &active_cols {
        let ds_title = xy.data_sets[ds_idx].title.clone();
        let extracted = extract_xy_observations_and_unknowns(xy, ds_idx)?;
        let obs = extracted.obs;
        let forward_unknowns = extracted.forward_unknowns;
        let inverse_unknowns = extracted.inverse_unknowns;
        if obs.len() < 2 {
            return Err(TransformError::InvalidInput { message: format!("dataset `{ds_title}` has fewer than 2 active finite (X, Y) observations") });
        }
        let n_obs = obs.len();
        let distinct_x = count_distinct_x(&obs);
        if distinct_x < 2 {
            return Err(TransformError::InvalidInput { message: format!("dataset `{ds_title}` requires at least 2 distinct finite X values") });
        }

        let effective_segments = if spec.method == SplineLowessMethod::Lowess {
            let min_lowess = n_obs.saturating_mul(4);
            if min_lowess > SplineLowessSpec::MAX_SEGMENTS {
                sheet.checklist.push(ChecklistItem {
                    assumption: String::from("LOWESS segment ceiling"),
                    status: ChecklistStatus::Info,
                    diagnostic_metric: Some(format!("4*N = {min_lowess}, capped at {}", SplineLowessSpec::MAX_SEGMENTS)),
                    explanation: format!(
                        "Dataset `{ds_title}` requested at least 4*N LOWESS polyline vertices; output was capped at the maximum safe limit ({}).",
                        SplineLowessSpec::MAX_SEGMENTS
                    ),
                    remediation: None,
                });
            }
            spec.segment_count.max(min_lowess).clamp(SplineLowessSpec::MIN_SEGMENTS, SplineLowessSpec::MAX_SEGMENTS)
        } else {
            spec.segment_count.clamp(SplineLowessSpec::MIN_SEGMENTS, SplineLowessSpec::MAX_SEGMENTS)
        };

        if spec.method == SplineLowessMethod::Lowess && n_obs < SplineLowessSpec::LOWESS_MIN_RECOMMENDED_N {
            sheet.checklist.push(ChecklistItem {
                assumption: String::from("LOWESS sample size"),
                status: ChecklistStatus::Warn,
                diagnostic_metric: Some(format!("{ds_title}: N = {n_obs} (< {})", SplineLowessSpec::LOWESS_MIN_RECOMMENDED_N)),
                explanation: format!(
                    "LOWESS local linear smoothing is recommended for at least {} observations (dataset `{ds_title}` has N = {n_obs}); consider Natural Cubic Spline, Akima Subspline, or Smoothing Spline for small samples.",
                    SplineLowessSpec::LOWESS_MIN_RECOMMENDED_N
                ),
                remediation: None,
            });
        }

        let (model, smoothing_df_opt, smoothing_lambda_opt) = fit_single_series(&obs, spec)?;
        let (data_x_min, data_x_max) = model.domain();
        let eval_x_min = spec.plot_x_min.unwrap_or(data_x_min);
        let eval_x_max = spec.plot_x_max.unwrap_or(data_x_max);
        let sampled = sample_curve(&model, eval_x_min, eval_x_max, effective_segments)?;

        let mut rss_acc = sum::NeumaierSum::new();
        for &(x, y) in &obs {
            let r = y - model.eval(x);
            rss_acc.add(r * r);
        }
        let rss = rss_acc.total().max(0.0);
        let rmse = (rss / (n_obs as f64)).sqrt();

        sheet.fitted_curves.push(FittedCurveOverlay {
            data_set_index: ds_idx,
            equation_id: spec.method.equation_id().to_owned(),
            converged: true,
            ambiguous: false,
            points: sampled
                .iter()
                .map(|&(x, y)| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None })
                .collect(),
        });

        sheet.scalars.push(ResultScalarEntry { key: String::from("n_points"), data_set_index: Some(ds_idx), value: n_obs as f64, units: None });
        sheet.scalars.push(ResultScalarEntry {
            key: String::from("distinct_x"),
            data_set_index: Some(ds_idx),
            value: distinct_x as f64,
            units: None,
        });
        sheet.scalars.push(ResultScalarEntry {
            key: String::from("segments"),
            data_set_index: Some(ds_idx),
            value: effective_segments as f64,
            units: None,
        });
        sheet.scalars.push(ResultScalarEntry { key: String::from("rss"), data_set_index: Some(ds_idx), value: rss, units: None });
        sheet.scalars.push(ResultScalarEntry { key: String::from("rmse"), data_set_index: Some(ds_idx), value: rmse, units: None });
        if let Some(df) = smoothing_df_opt {
            sheet.scalars.push(ResultScalarEntry { key: String::from("effective_df"), data_set_index: Some(ds_idx), value: df, units: None });
        }
        if let Some(lam) = smoothing_lambda_opt
            && lam.is_finite()
        {
            sheet.scalars.push(ResultScalarEntry { key: String::from("smoothing_lambda"), data_set_index: Some(ds_idx), value: lam, units: None });
        }
        if spec.method == SplineLowessMethod::Lowess {
            sheet.scalars.push(ResultScalarEntry {
                key: String::from("lowess_span"),
                data_set_index: Some(ds_idx),
                value: spec.effective_lowess_fraction(),
                units: None,
            });
        }

        if spec.emit_curve_coordinates_tab {
            coord_series.push((ds_title.clone(), sampled));
        }

        if spec.interpolate_unknowns {
            append_interpolated_unknown_rows(
                &ds_title,
                &model,
                eval_x_min,
                eval_x_max,
                &forward_unknowns,
                &inverse_unknowns,
                &mut interp_row_labels,
                &mut interp_rows,
            );
        }

        summary_cols.push(ds_title);
        series_summaries.push(SeriesFitSummary {
            n_obs,
            distinct_x,
            effective_segments,
            x_min: eval_x_min,
            x_max: eval_x_max,
            rss,
            rmse,
            effective_df: smoothing_df_opt,
            smoothing_lambda: smoothing_lambda_opt,
        });
    }

    sheet.tabs.push(build_main_summary_tab(spec, summary_cols, &series_summaries));

    if spec.emit_curve_coordinates_tab && !coord_series.is_empty() {
        sheet.tabs.push(build_curve_coordinates_tab(&coord_series));
    }

    if spec.interpolate_unknowns {
        sheet.tabs.push(ResultTab {
            id: String::from("interpolated-unknowns"),
            title: String::from("Interpolated unknowns"),
            kind: ResultTabKind::InterpolatedUnknowns,
            column_headers: vec![
                String::from("Dataset"),
                String::from("Direction"),
                String::from("Known input"),
                String::from("Interpolated value"),
                String::from("Status"),
            ],
            row_labels: interp_row_labels,
            rows: interp_rows,
            footnotes: vec![String::from(
                "Forward interpolation evaluates S(X) for (X, blank Y) rows; inverse interpolation solves S(X) = Y across the fitted curve domain for (blank X, Y) rows.",
            )],
        });
    }

    Ok(sheet)
}

struct SeriesFitSummary {
    n_obs: usize,
    distinct_x: usize,
    effective_segments: usize,
    x_min: f64,
    x_max: f64,
    rss: f64,
    rmse: f64,
    effective_df: Option<f64>,
    smoothing_lambda: Option<f64>,
}

fn validate_spline_spec(spec: &SplineLowessSpec) -> Result<(), TransformError> {
    if !(SplineLowessSpec::MIN_SEGMENTS..=SplineLowessSpec::MAX_SEGMENTS).contains(&spec.segment_count) {
        return Err(TransformError::InvalidSpec {
            message: format!(
                "segment_count must be in {}..={}, got {}",
                SplineLowessSpec::MIN_SEGMENTS,
                SplineLowessSpec::MAX_SEGMENTS,
                spec.segment_count
            ),
        });
    }
    if let Some(knots) = spec.smoothing_knots
        && knots < 2
    {
        return Err(TransformError::InvalidSpec { message: format!("smoothing_knots must be >= 2, got {knots}") });
    }
    if let Some(df) = spec.smoothing_df
        && (!df.is_finite() || df < 2.0)
    {
        return Err(TransformError::InvalidSpec { message: format!("smoothing_df must be finite and >= 2, got {df}") });
    }
    if let Some(frac) = spec.lowess_fraction
        && (!frac.is_finite() || frac <= 0.0 || frac > 1.0)
    {
        return Err(TransformError::InvalidSpec { message: format!("lowess_fraction must be finite and in (0, 1], got {frac}") });
    }
    if spec.lowess_robustness_iters > SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS {
        return Err(TransformError::InvalidSpec {
            message: format!(
                "lowess_robustness_iters must be <= {}, got {}",
                SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS,
                spec.lowess_robustness_iters
            ),
        });
    }
    if let Some(lo) = spec.plot_x_min
        && !lo.is_finite()
    {
        return Err(TransformError::InvalidSpec { message: String::from("plot_x_min must be finite") });
    }
    if let Some(hi) = spec.plot_x_max
        && !hi.is_finite()
    {
        return Err(TransformError::InvalidSpec { message: String::from("plot_x_max must be finite") });
    }
    if let (Some(lo), Some(hi)) = (spec.plot_x_min, spec.plot_x_max)
        && hi <= lo
    {
        return Err(TransformError::InvalidSpec { message: format!("plot_x_min ({lo}) must be strictly less than plot_x_max ({hi})") });
    }
    Ok(())
}

fn resolve_active_columns(total_cols: usize, selected: &[usize]) -> Result<Vec<usize>, TransformError> {
    if selected.is_empty() {
        return Ok((0..total_cols).collect());
    }
    let mut out = Vec::with_capacity(selected.len().min(total_cols));
    for &idx in selected {
        if idx >= total_cols {
            return Err(TransformError::InvalidSpec { message: format!("selected dataset index {idx} exceeds column count {total_cols}") });
        }
        if !out.contains(&idx) {
            out.push(idx);
        }
    }
    Ok(out)
}

struct ExtractedXySeries {
    obs: Vec<(f64, f64)>,
    forward_unknowns: Vec<(usize, f64)>,
    inverse_unknowns: Vec<(usize, f64)>,
}

fn extract_xy_observations_and_unknowns(xy: &XyTable, ds_idx: usize) -> Result<ExtractedXySeries, TransformError> {
    let ds = &xy.data_sets[ds_idx];
    let mut obs = Vec::new();
    let mut forward_unknowns = Vec::new();
    let mut inverse_unknowns = Vec::new();

    for row in 0..xy.row_count() {
        let x_opt = xy.x_summary_at(row)?.filter(|v| v.is_finite()).map(|x| if x == 0.0 { 0.0 } else { x });
        let y_vals: Vec<f64> = if xy.y_subcolumn_mode.is_pre_summarized() {
            ds.cells.get(row).and_then(|sub| sub.first()).and_then(Cell::active_number).into_iter().collect()
        } else {
            ds.cells.get(row).map(|sub| sub.iter().filter_map(Cell::active_number).collect()).unwrap_or_default()
        };

        match (x_opt, y_vals.is_empty()) {
            (Some(x), false) => {
                for y in y_vals {
                    obs.push((x, y));
                }
            }
            (Some(x), true) => {
                forward_unknowns.push((row, x));
            }
            (None, false) => {
                let y_mean = sum::mean(&y_vals)?;
                inverse_unknowns.push((row, y_mean));
            }
            (None, true) => {}
        }
    }
    Ok(ExtractedXySeries { obs, forward_unknowns, inverse_unknowns })
}

fn fit_single_series(obs: &[(f64, f64)], spec: &SplineLowessSpec) -> Result<(FittedCurveModel, Option<f64>, Option<f64>), TransformError> {
    match spec.method {
        SplineLowessMethod::NaturalCubicSpline => Ok((fit_natural_cubic_spline(obs)?, None, None)),
        SplineLowessMethod::AkimaSubspline => Ok((fit_akima_subspline(obs)?, None, None)),
        SplineLowessMethod::SmoothingSpline => {
            let target_df = spec.smoothing_df.or_else(|| spec.smoothing_knots.map(|k| k as f64));
            let fit = fit_smoothing_spline(obs, None, target_df)?;
            Ok((fit.model, Some(fit.effective_df), Some(fit.lambda)))
        }
        SplineLowessMethod::Lowess => {
            let frac = spec.effective_lowess_fraction();
            let model = fit_lowess_curve(obs, frac, spec.lowess_robustness_iters)?;
            Ok((model, None, None))
        }
        SplineLowessMethod::PointToPoint => Ok((fit_point_to_point(obs)?, None, None)),
    }
}

fn build_main_summary_tab(spec: &SplineLowessSpec, col_headers: Vec<String>, summaries: &[SeriesFitSummary]) -> ResultTab {
    let mut row_labels = vec![
        String::from("Method"),
        String::from("Active observations (N)"),
        String::from("Distinct X knots (G)"),
        String::from("Polyline vertices"),
        String::from("Domain X min"),
        String::from("Domain X max"),
        String::from("Residual sum of squares (RSS)"),
        String::from("Root mean square error (RMSE)"),
    ];
    let mut rows: Vec<Vec<ResultCell>> = vec![
        summaries.iter().map(|_| ResultCell::Text(spec.method.label().to_owned())).collect(),
        summaries.iter().map(|s| num_cell(s.n_obs as f64)).collect(),
        summaries.iter().map(|s| num_cell(s.distinct_x as f64)).collect(),
        summaries.iter().map(|s| num_cell(s.effective_segments as f64)).collect(),
        summaries.iter().map(|s| num_cell(s.x_min)).collect(),
        summaries.iter().map(|s| num_cell(s.x_max)).collect(),
        summaries.iter().map(|s| num_cell(s.rss)).collect(),
        summaries.iter().map(|s| num_cell(s.rmse)).collect(),
    ];

    match spec.method {
        SplineLowessMethod::SmoothingSpline => {
            row_labels.push(String::from("Effective degrees of freedom (df)"));
            rows.push(summaries.iter().map(|s| s.effective_df.map_or(ResultCell::Empty, num_cell)).collect());
            row_labels.push(String::from("Smoothing penalty (lambda)"));
            rows.push(
                summaries
                    .iter()
                    .map(|s| match s.smoothing_lambda {
                        Some(v) if v.is_finite() => num_cell(v),
                        Some(_) => ResultCell::Text(String::from("Infinity (linear limit)")),
                        None => ResultCell::Empty,
                    })
                    .collect(),
            );
        }
        SplineLowessMethod::Lowess => {
            let span = spec.effective_lowess_fraction();
            row_labels.push(String::from("LOWESS span fraction (f)"));
            rows.push(summaries.iter().map(|_| num_cell(span)).collect());
            row_labels.push(String::from("Bisquare robustness passes"));
            rows.push(summaries.iter().map(|_| num_cell(spec.lowess_robustness_iters as f64)).collect());
        }
        SplineLowessMethod::NaturalCubicSpline | SplineLowessMethod::AkimaSubspline | SplineLowessMethod::PointToPoint => {}
    }

    let footnote = match spec.method {
        SplineLowessMethod::NaturalCubicSpline => {
            "Natural cubic spline (de Boor 1978): C^2 piecewise cubic through distinct sorted (X, mean Y) knots with S''(X_min) = S''(X_max) = 0."
        }
        SplineLowessMethod::AkimaSubspline => {
            "Akima subspline (Akima 1970): C^1 piecewise cubic using five-point secant slope weights to suppress oscillation near sharp transitions."
        }
        SplineLowessMethod::SmoothingSpline => {
            "Penalized cubic smoothing spline (Reinsch 1967; Hutchinson & de Hoog 1985): minimizes weighted RSS + lambda * integral [S''(x)]^2 dx."
        }
        SplineLowessMethod::Lowess => {
            "LOWESS (Cleveland 1979): tricube-weighted local linear regression with Tukey bisquare robustness iterations and at least 4*N sampled vertices."
        }
        SplineLowessMethod::PointToPoint => {
            "Dense data-space point-to-point curve: piecewise linear interpolation in data space resampled onto polyline vertices (curved on log axes)."
        }
    };

    ResultTab {
        id: String::from("summary"),
        title: String::from("Spline / LOWESS summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: col_headers,
        row_labels,
        rows,
        footnotes: vec![String::from(footnote)],
    }
}

fn build_curve_coordinates_tab(series: &[(String, Vec<(f64, f64)>)]) -> ResultTab {
    let max_len = series.iter().map(|(_, pts)| pts.len()).max().unwrap_or(0);
    let mut column_headers = Vec::with_capacity(series.len() * 2);
    for (title, _) in series {
        column_headers.push(format!("X ({title})"));
        column_headers.push(format!("Y ({title})"));
    }
    let mut row_labels = Vec::with_capacity(max_len);
    let mut rows = Vec::with_capacity(max_len);
    for r in 0..max_len {
        row_labels.push(format!("{}", r + 1));
        let mut row = Vec::with_capacity(series.len() * 2);
        for (_, pts) in series {
            if let Some(&(x, y)) = pts.get(r) {
                row.push(num_cell(x));
                row.push(num_cell(y));
            } else {
                row.push(ResultCell::Empty);
                row.push(ResultCell::Empty);
            }
        }
        rows.push(row);
    }
    ResultTab {
        id: String::from("curve-coordinates"),
        title: String::from("Curve coordinates"),
        kind: ResultTabKind::CurveCoordinates,
        column_headers,
        row_labels,
        rows,
        footnotes: vec![String::from("Sampled (X, Y) coordinates along the fitted model-free curve.")],
    }
}

#[allow(clippy::too_many_arguments)]
fn append_interpolated_unknown_rows(
    ds_title: &str,
    model: &FittedCurveModel,
    x_lo: f64,
    x_hi: f64,
    forward_unknowns: &[(usize, f64)],
    inverse_unknowns: &[(usize, f64)],
    row_labels: &mut Vec<String>,
    rows: &mut Vec<Vec<ResultCell>>,
) {
    for &(row_idx, x_unk) in forward_unknowns {
        row_labels.push(format!("Row {}", row_idx + 1));
        let in_domain = x_unk >= x_lo && x_unk <= x_hi;
        let y_hat = model.eval(x_unk);
        let (val_cell, status) = if in_domain && y_hat.is_finite() {
            (num_cell(y_hat), String::from("Interpolated"))
        } else if y_hat.is_finite() {
            (ResultCell::Empty, format!("Out of domain [{x_lo:.4}, {x_hi:.4}]"))
        } else {
            (ResultCell::Empty, String::from("Undefined"))
        };
        rows.push(vec![
            ResultCell::Text(ds_title.to_owned()),
            ResultCell::Text(String::from("Y from X")),
            num_cell(x_unk),
            val_cell,
            ResultCell::Text(status),
        ]);
    }

    for &(row_idx, y_unk) in inverse_unknowns {
        row_labels.push(format!("Row {}", row_idx + 1));
        let roots = model.solve_inverse(y_unk, x_lo, x_hi);
        let (val_cell, status) = match roots.as_slice() {
            [] => (ResultCell::Empty, String::from("Out of curve range")),
            [x_sol] => (num_cell(*x_sol), String::from("Interpolated")),
            [first, rest @ ..] => {
                let all_str: Vec<String> = std::iter::once(*first).chain(rest.iter().copied()).map(|v| format!("{v:.6}")).collect();
                (num_cell(*first), format!("Multiple crossings ({}): {}", roots.len(), all_str.join(", ")))
            }
        };
        rows.push(vec![
            ResultCell::Text(ds_title.to_owned()),
            ResultCell::Text(String::from("X from Y")),
            num_cell(y_unk),
            val_cell,
            ResultCell::Text(status),
        ]);
    }
}

fn num_cell(value: f64) -> ResultCell {
    ResultCell::Number { value, prefix: None, units: None }
}

// ============================================================================
// Internal Numerical Solvers
// ============================================================================

fn validate_and_sort_points(points: &[(f64, f64)]) -> Result<Vec<(f64, f64)>, TransformError> {
    let mut pts = Vec::with_capacity(points.len());
    for &(x_raw, y) in points {
        if !x_raw.is_finite() || !y.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("curve points must have finite x and y") });
        }
        let x = if x_raw == 0.0 { 0.0 } else { x_raw };
        pts.push((x, y));
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(pts)
}

fn collapse_distinct_knots(points: &[(f64, f64)]) -> Result<Vec<(f64, f64)>, TransformError> {
    let sorted = validate_and_sort_points(points)?;
    let mut knots = Vec::with_capacity(sorted.len());
    let mut i = 0_usize;
    while i < sorted.len() {
        let x_k = sorted[i].0;
        let mut j = i + 1;
        while j < sorted.len() && sorted[j].0 == x_k {
            j += 1;
        }
        if j - i == 1 {
            knots.push(sorted[i]);
        } else {
            let ys: Vec<f64> = sorted[i..j].iter().map(|p| p.1).collect();
            knots.push((x_k, sum::mean(&ys)?));
        }
        i = j;
    }
    Ok(knots)
}

fn collapse_weighted_knots(points: &[(f64, f64)], weights: Option<&[f64]>) -> Result<Vec<(f64, f64, f64)>, TransformError> {
    if let Some(ws) = weights
        && ws.len() != points.len()
    {
        return Err(TransformError::InvalidInput { message: format!("weights length ({}) must match points length ({})", ws.len(), points.len()) });
    }
    let mut items = Vec::with_capacity(points.len());
    for (idx, &(x_raw, y)) in points.iter().enumerate() {
        if !x_raw.is_finite() || !y.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("curve points must have finite x and y") });
        }
        let w = weights.map_or(1.0, |ws| ws[idx]);
        if !w.is_finite() || w <= 0.0 {
            return Err(TransformError::InvalidInput { message: format!("smoothing spline weight must be finite and > 0, got {w}") });
        }
        let x = if x_raw == 0.0 { 0.0 } else { x_raw };
        items.push((x, y, w));
    }
    items.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut knots = Vec::with_capacity(items.len());
    let mut i = 0_usize;
    while i < items.len() {
        let x_k = items[i].0;
        let mut j = i + 1;
        while j < items.len() && items[j].0 == x_k {
            j += 1;
        }
        if j - i == 1 {
            knots.push(items[i]);
        } else {
            let mut w_acc = sum::NeumaierSum::new();
            let mut wy_acc = sum::NeumaierSum::new();
            for &(_, y, w) in &items[i..j] {
                w_acc.add(w);
                wy_acc.add(w * y);
            }
            let w_tot = w_acc.total();
            knots.push((x_k, wy_acc.total() / w_tot, w_tot));
        }
        i = j;
    }
    Ok(knots)
}

fn count_distinct_x(obs: &[(f64, f64)]) -> usize {
    let mut xs: Vec<f64> = obs.iter().map(|&(x, _)| if x == 0.0 { 0.0 } else { x }).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    xs.len()
}

fn build_linear_cubic_piece(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<FittedCurveModel, TransformError> {
    let dx = x1 - x0;
    if dx <= 0.0 {
        return Err(TransformError::InvalidInput { message: String::from("curve requires strictly increasing X endpoints") });
    }
    let slope = (y1 - y0) / dx;
    let piece = CubicPiece { x0, x1, a: y0, b: slope, c: 0.0, d: 0.0 };
    Ok(FittedCurveModel::PiecewiseCubic { pieces: vec![piece], left_slope: slope, right_slope: slope, x_min: x0, x_max: x1 })
}

fn build_natural_cubic_from_knots(xs: &[f64], ys: &[f64]) -> Result<FittedCurveModel, TransformError> {
    let m = xs.len();
    if m < 2 {
        return Err(TransformError::InvalidInput { message: String::from("natural cubic spline requires at least 2 distinct knots") });
    }
    if m == 2 {
        return build_linear_cubic_piece(xs[0], ys[0], xs[1], ys[1]);
    }
    let n_seg = m - 1;
    let mut h = Vec::with_capacity(n_seg);
    let mut delta = Vec::with_capacity(n_seg);
    for i in 0..n_seg {
        let hi = xs[i + 1] - xs[i];
        if hi <= 0.0 {
            return Err(TransformError::InvalidInput { message: String::from("spline knots must be strictly increasing") });
        }
        h.push(hi);
        delta.push((ys[i + 1] - ys[i]) / hi);
    }

    // Solve strictly diagonally dominant tridiagonal system for interior second derivatives M_1..M_{m-2}.
    let k = m - 2;
    let mut diag = Vec::with_capacity(k);
    let mut sub = Vec::with_capacity(k.saturating_sub(1));
    let mut sup = Vec::with_capacity(k.saturating_sub(1));
    let mut rhs = Vec::with_capacity(k);
    for j in 0..k {
        diag.push(2.0 * (h[j] + h[j + 1]));
        rhs.push(6.0 * (delta[j + 1] - delta[j]));
        if j + 1 < k {
            sup.push(h[j + 1]);
            sub.push(h[j + 1]);
        }
    }
    let interior_m = solve_tridiagonal(&sub, &diag, &sup, &rhs)?;
    build_cubic_from_values_and_second_derivs(xs, ys, &interior_m)
}

fn build_cubic_from_values_and_second_derivs(xs: &[f64], vals: &[f64], interior_m: &[f64]) -> Result<FittedCurveModel, TransformError> {
    let m = xs.len();
    let n_seg = m - 1;
    let mut full_m = Vec::with_capacity(m);
    full_m.push(0.0);
    full_m.extend_from_slice(interior_m);
    full_m.push(0.0);

    let mut pieces = Vec::with_capacity(n_seg);
    for i in 0..n_seg {
        let hi = xs[i + 1] - xs[i];
        if hi <= 0.0 {
            return Err(TransformError::InvalidInput { message: String::from("spline knots must be strictly increasing") });
        }
        let m0 = full_m[i];
        let m1 = full_m[i + 1];
        let delta = (vals[i + 1] - vals[i]) / hi;
        let b = delta - (hi / 6.0) * (2.0 * m0 + m1);
        let c = 0.5 * m0;
        let d = (m1 - m0) / (6.0 * hi);
        pieces.push(CubicPiece { x0: xs[i], x1: xs[i + 1], a: vals[i], b, c, d });
    }
    let left_slope = pieces[0].b;
    let right_slope = pieces[n_seg - 1].deriv(xs[m - 1]);
    Ok(FittedCurveModel::PiecewiseCubic { pieces, left_slope, right_slope, x_min: xs[0], x_max: xs[m - 1] })
}

fn solve_tridiagonal(sub: &[f64], diag: &[f64], sup: &[f64], rhs: &[f64]) -> Result<Vec<f64>, TransformError> {
    let n = diag.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut c_prime = vec![0.0_f64; n];
    let mut d_prime = vec![0.0_f64; n];

    let mut denom = diag[0];
    if !denom.is_finite() || denom.abs() <= f64::MIN_POSITIVE {
        return Err(TransformError::InvalidInput { message: String::from("singular tridiagonal spline system") });
    }
    if n > 1 {
        c_prime[0] = sup[0] / denom;
    }
    d_prime[0] = rhs[0] / denom;

    for i in 1..n {
        let a_i = sub[i - 1];
        denom = diag[i] - a_i * c_prime[i - 1];
        if !denom.is_finite() || denom.abs() <= f64::MIN_POSITIVE {
            return Err(TransformError::InvalidInput { message: String::from("singular tridiagonal spline system") });
        }
        if i + 1 < n {
            c_prime[i] = sup[i] / denom;
        }
        d_prime[i] = (rhs[i] - a_i * d_prime[i - 1]) / denom;
    }

    let mut x = vec![0.0_f64; n];
    x[n - 1] = d_prime[n - 1];
    for i in (0..n - 1).rev() {
        x[i] = d_prime[i] - c_prime[i] * x[i + 1];
    }
    Ok(x)
}

/// Precomputed Reinsch (1967) tridiagonal `R` and pentadiagonal `B = Q^T W^{-1} Q` matrices.
struct ReinschSystem {
    m: usize,
    ys: Vec<f64>,
    inv_w: Vec<f64>,
    q0: Vec<f64>,
    q1: Vec<f64>,
    q2: Vec<f64>,
    r_diag: Vec<f64>,
    r_sup1: Vec<f64>,
    b_diag: Vec<f64>,
    b_sup1: Vec<f64>,
    b_sup2: Vec<f64>,
    qt_y: Vec<f64>,
}

struct BandedLdlt {
    d: Vec<f64>,
    e: Vec<f64>,
    f: Vec<f64>,
}

impl ReinschSystem {
    fn build(xs: &[f64], ys: &[f64], ws: &[f64]) -> Result<Self, TransformError> {
        let m = xs.len();
        let k = m - 2;
        let mut h = Vec::with_capacity(m - 1);
        for i in 0..m - 1 {
            let hi = xs[i + 1] - xs[i];
            if hi <= 0.0 {
                return Err(TransformError::InvalidInput { message: String::from("smoothing spline knots must be strictly increasing") });
            }
            h.push(hi);
        }

        let inv_w: Vec<f64> = ws.iter().map(|&w| 1.0 / w).collect();
        let mut q0 = Vec::with_capacity(k);
        let mut q1 = Vec::with_capacity(k);
        let mut q2 = Vec::with_capacity(k);
        let mut r_diag = Vec::with_capacity(k);
        let mut r_sup1 = Vec::with_capacity(k.saturating_sub(1));
        let mut qt_y = Vec::with_capacity(k);

        for j in 0..k {
            let inv_h0 = 1.0 / h[j];
            let inv_h1 = 1.0 / h[j + 1];
            q0.push(inv_h0);
            q1.push(-(inv_h0 + inv_h1));
            q2.push(inv_h1);
            r_diag.push((h[j] + h[j + 1]) / 3.0);
            if j + 1 < k {
                r_sup1.push(h[j + 1] / 6.0);
            }
            qt_y.push(inv_h0 * ys[j] - (inv_h0 + inv_h1) * ys[j + 1] + inv_h1 * ys[j + 2]);
        }

        let mut b_diag = Vec::with_capacity(k);
        let mut b_sup1 = Vec::with_capacity(k.saturating_sub(1));
        let mut b_sup2 = Vec::with_capacity(k.saturating_sub(2));
        for j in 0..k {
            b_diag.push(q0[j] * q0[j] * inv_w[j] + q1[j] * q1[j] * inv_w[j + 1] + q2[j] * q2[j] * inv_w[j + 2]);
            if j + 1 < k {
                b_sup1.push(q1[j] * q0[j + 1] * inv_w[j + 1] + q2[j] * q1[j + 1] * inv_w[j + 2]);
            }
            if j + 2 < k {
                b_sup2.push(q2[j] * q0[j + 2] * inv_w[j + 2]);
            }
        }

        Ok(Self { m, ys: ys.to_vec(), inv_w, q0, q1, q2, r_diag, r_sup1, b_diag, b_sup1, b_sup2, qt_y })
    }

    /// Factor `A(lambda) = R + lambda * B = L D L^T` where `L` is unit lower triangular with bandwidth 2.
    fn factor_ldlt(&self, lambda: f64) -> Result<BandedLdlt, TransformError> {
        let k = self.r_diag.len();
        let mut d = vec![0.0_f64; k];
        let mut e = vec![0.0_f64; k.saturating_sub(1)];
        let mut f = vec![0.0_f64; k.saturating_sub(2)];

        for i in 0..k {
            let a_ii = self.r_diag[i] + lambda * self.b_diag[i];
            let mut val = a_ii;
            if i >= 1 {
                val -= e[i - 1] * e[i - 1] * d[i - 1];
            }
            if i >= 2 {
                val -= f[i - 2] * f[i - 2] * d[i - 2];
            }
            if !val.is_finite() || val <= 0.0 {
                return Err(TransformError::InvalidInput { message: String::from("smoothing spline normal equations are not positive definite") });
            }
            d[i] = val;

            if i + 1 < k {
                let mut a_i1 = self.r_sup1[i] + lambda * self.b_sup1[i];
                if i >= 1 {
                    a_i1 -= f[i - 1] * e[i - 1] * d[i - 1];
                }
                e[i] = a_i1 / d[i];
            }
            if i + 2 < k {
                let a_i2 = lambda * self.b_sup2[i];
                f[i] = a_i2 / d[i];
            }
        }
        Ok(BandedLdlt { d, e, f })
    }

    /// Compute `df(lambda) = m - lambda * tr(A(lambda)^{-1} B)` in `O(m)` via Takahashi recursion
    /// (Hutchinson & de Hoog 1985).
    fn effective_df(&self, lambda: f64) -> Result<f64, TransformError> {
        let k = self.r_diag.len();
        let BandedLdlt { d, e, f } = self.factor_ldlt(lambda)?;
        let mut z0 = vec![0.0_f64; k];
        let mut z1 = vec![0.0_f64; k.saturating_sub(1)];
        let mut z2 = vec![0.0_f64; k.saturating_sub(2)];

        for i in (0..k).rev() {
            let mut val2 = 0.0;
            if i + 2 < k {
                val2 = -e[i] * z1[i + 1] - f[i] * z0[i + 2];
                z2[i] = val2;
            }
            let mut val1 = 0.0;
            if i + 1 < k {
                val1 = -e[i] * z0[i + 1];
                if i + 2 < k {
                    val1 -= f[i] * z1[i + 1];
                }
                z1[i] = val1;
            }
            let mut val0 = 1.0 / d[i];
            if i + 1 < k {
                val0 -= e[i] * val1;
            }
            if i + 2 < k {
                val0 -= f[i] * val2;
            }
            z0[i] = val0;
        }

        let mut tr = sum::NeumaierSum::new();
        for i in 0..k {
            tr.add(z0[i] * self.b_diag[i]);
            if i + 1 < k {
                tr.add(2.0 * z1[i] * self.b_sup1[i]);
            }
            if i + 2 < k {
                tr.add(2.0 * z2[i] * self.b_sup2[i]);
            }
        }
        let df = (self.m as f64) - lambda * tr.total();
        Ok(df.clamp(2.0, self.m as f64))
    }

    fn solve_lambda_for_df(&self, target_df: f64) -> Result<(f64, f64), TransformError> {
        let tr_r: f64 = self.r_diag.iter().sum();
        let tr_b: f64 = self.b_diag.iter().sum();
        let scale = if tr_b > 0.0 && tr_r > 0.0 { tr_r / tr_b } else { 1.0 };
        let log_mid = scale.ln();

        let mut lo = log_mid;
        let mut hi = log_mid;
        let mut f_lo = self.effective_df(lo.exp())? - target_df;
        let mut f_hi = f_lo;

        let mut step = 1.0_f64;
        for _ in 0..80 {
            if f_lo < 0.0 {
                lo -= step;
                f_lo = self.effective_df(lo.exp())? - target_df;
                step *= 1.5;
            } else {
                break;
            }
        }
        step = 1.0;
        for _ in 0..80 {
            if f_hi > 0.0 {
                hi += step;
                f_hi = self.effective_df(hi.exp())? - target_df;
                step *= 1.5;
            } else {
                break;
            }
        }

        if f_lo.abs() <= 1e-10 {
            let lam = lo.exp();
            return Ok((lam, target_df + f_lo));
        }
        if f_hi.abs() <= 1e-10 {
            let lam = hi.exp();
            return Ok((lam, target_df + f_hi));
        }

        for _ in 0..100 {
            let mid = 0.5 * (lo + hi);
            let f_mid = self.effective_df(mid.exp())? - target_df;
            if f_mid.abs() <= 1e-10 || (hi - lo).abs() <= 1e-12 {
                let lam = mid.exp();
                return Ok((lam, target_df + f_mid));
            }
            if f_mid > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let lam = (0.5 * (lo + hi)).exp();
        let df = self.effective_df(lam)?;
        Ok((lam, df))
    }

    fn solve_for_lambda(&self, lambda: f64) -> Result<(Vec<f64>, Vec<f64>), TransformError> {
        let k = self.r_diag.len();
        let BandedLdlt { d, e, f } = self.factor_ldlt(lambda)?;

        // Forward substitution: L z = Q^T y.
        let mut z = self.qt_y.clone();
        for i in 0..k {
            if i + 1 < k {
                z[i + 1] -= e[i] * z[i];
            }
            if i + 2 < k {
                z[i + 2] -= f[i] * z[i];
            }
        }
        // Diagonal scaling: D y' = z.
        for i in 0..k {
            z[i] /= d[i];
        }
        // Backward substitution: L^T gamma = y'.
        for i in (0..k).rev() {
            if i + 1 < k {
                z[i] -= e[i] * z[i + 1];
            }
            if i + 2 < k {
                z[i] -= f[i] * z[i + 2];
            }
        }

        // Compute g = y - lambda * W^{-1} Q gamma.
        let mut q_gamma = vec![0.0_f64; self.m];
        for j in 0..k {
            let gj = z[j];
            q_gamma[j] += self.q0[j] * gj;
            q_gamma[j + 1] += self.q1[j] * gj;
            q_gamma[j + 2] += self.q2[j] * gj;
        }
        let mut g = Vec::with_capacity(self.m);
        for i in 0..self.m {
            g.push(self.ys[i] - lambda * self.inv_w[i] * q_gamma[i]);
        }
        Ok((g, z))
    }
}

fn weighted_ols_line(xs: &[f64], ys: &[f64], ws: &[f64]) -> Result<(f64, f64), TransformError> {
    let mut sw = sum::NeumaierSum::new();
    let mut swx = sum::NeumaierSum::new();
    let mut swy = sum::NeumaierSum::new();
    for ((&x, &y), &w) in xs.iter().zip(ys.iter()).zip(ws.iter()) {
        sw.add(w);
        swx.add(w * x);
        swy.add(w * y);
    }
    let w_tot = sw.total();
    let x_bar = swx.total() / w_tot;
    let y_bar = swy.total() / w_tot;
    let mut sxx = sum::NeumaierSum::new();
    let mut sxy = sum::NeumaierSum::new();
    for ((&x, &y), &w) in xs.iter().zip(ys.iter()).zip(ws.iter()) {
        let dx = x - x_bar;
        sxx.add(w * dx * dx);
        sxy.add(w * dx * (y - y_bar));
    }
    let denom = sxx.total();
    if denom <= 0.0 {
        return Err(TransformError::InvalidInput { message: String::from("zero X variance in smoothing spline linear limit") });
    }
    let slope = sxy.total() / denom;
    let intercept = y_bar - slope * x_bar;
    Ok((intercept, slope))
}

fn weighted_rss(ys: &[f64], fitted: &[f64], ws: &[f64]) -> f64 {
    let mut acc = sum::NeumaierSum::new();
    for ((&y, &f), &w) in ys.iter().zip(fitted.iter()).zip(ws.iter()) {
        let r = y - f;
        acc.add(w * r * r);
    }
    acc.total().max(0.0)
}

fn eval_lowess_at(xs: &[f64], ys: &[f64], rob_weights: &[f64], q: usize, x_target: f64) -> f64 {
    let n = xs.len();
    if n == 0 {
        return f64::NAN;
    }
    let mut dists: Vec<(usize, f64)> = xs.iter().enumerate().map(|(j, &xj)| (j, (xj - x_target).abs())).collect();
    dists.sort_by(|a, b| a.1.total_cmp(&b.1));
    let q_eff = q.clamp(1, n);
    let max_d = dists[q_eff - 1].1;
    if max_d == 0.0 {
        return tied_weighted_mean(xs, ys, rob_weights, x_target);
    }

    let mut data = Vec::with_capacity(q_eff * 2);
    let mut b = Vec::with_capacity(q_eff);
    let mut w_sum = sum::NeumaierSum::new();
    let mut wy_sum = sum::NeumaierSum::new();
    for &(j, d) in &dists[..q_eff] {
        let u = (d / max_d).min(1.0);
        let tricube = (1.0 - u * u * u).powi(3);
        let total_w = (tricube * rob_weights[j]).max(0.0);
        w_sum.add(total_w);
        wy_sum.add(total_w * ys[j]);
        let sw = total_w.sqrt();
        data.push(sw);
        data.push(sw * (xs[j] - x_target));
        b.push(sw * ys[j]);
    }
    if let Ok(a) = DenseMatrix::from_row_major(q_eff, 2, data)
        && let Ok((sol, _)) = solve_linear_least_squares(&a, &b)
        && let Some(&y0) = sol.coefficients.first()
        && y0.is_finite()
    {
        return y0;
    }
    let tw = w_sum.total();
    if tw > 0.0 { wy_sum.total() / tw } else { ys[dists[0].0] }
}

fn tied_weighted_mean(xs: &[f64], ys: &[f64], rob_weights: &[f64], x_target: f64) -> f64 {
    let mut w_sum = sum::NeumaierSum::new();
    let mut wy_sum = sum::NeumaierSum::new();
    let mut tied = Vec::new();
    for (&xj, (&yj, &rw)) in xs.iter().zip(ys.iter().zip(rob_weights.iter())) {
        if (xj - x_target).abs() == 0.0 {
            tied.push(yj);
            let w = rw.max(0.0);
            if w > 0.0 {
                w_sum.add(w);
                wy_sum.add(w * yj);
            }
        }
    }
    let tw = w_sum.total();
    if tw > 0.0 {
        let weighted = wy_sum.total() / tw;
        if weighted.is_finite() {
            return weighted;
        }
    }
    sum::mean(&tied).unwrap_or(f64::NAN)
}

fn update_bisquare_weights(ys: &[f64], fitted: &[f64], rob_weights: &mut [f64]) {
    let abs_res: Vec<f64> = ys.iter().zip(fitted.iter()).map(|(y, f)| (y - f).abs()).collect();
    let mut sorted = abs_res.clone();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    let mad = if sorted.len().is_multiple_of(2) { 0.5 * (sorted[mid - 1] + sorted[mid]) } else { sorted[mid] };
    let scale = 6.0 * mad;
    if scale <= 0.0 {
        rob_weights.fill(1.0);
        return;
    }
    for (w, r) in rob_weights.iter_mut().zip(abs_res) {
        let u = r / scale;
        *w = if u < 1.0 { (1.0 - u * u).powi(2) } else { 0.0 };
    }
}

fn eval_piecewise_linear(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    let n = xs.len();
    if n < 2 {
        return ys.first().copied().unwrap_or(f64::NAN);
    }
    if x <= xs[0] {
        let slope = (ys[1] - ys[0]) / (xs[1] - xs[0]);
        return ys[0] + slope * (x - xs[0]);
    }
    if x >= xs[n - 1] {
        let slope = (ys[n - 1] - ys[n - 2]) / (xs[n - 1] - xs[n - 2]);
        return ys[n - 1] + slope * (x - xs[n - 1]);
    }
    let idx = xs.partition_point(|&xk| xk < x).saturating_sub(1).min(n - 2);
    let t = (x - xs[idx]) / (xs[idx + 1] - xs[idx]);
    ys[idx] + t * (ys[idx + 1] - ys[idx])
}

/// Find all real roots of `piece.eval(x) == y_target` in `[seg_lo, seg_hi]` by partitioning at
/// the exact roots of `piece.deriv(x) == 0` (at most 2 interior critical points).
fn solve_cubic_piece_roots(piece: &CubicPiece, y_target: f64, seg_lo: f64, seg_hi: f64, out: &mut Vec<f64>) {
    let mut breaks = Vec::with_capacity(4);
    breaks.push(seg_lo);

    // Solve 3*d*u^2 + 2*c*u + b = 0 for interior extrema u = x - piece.x0.
    let qa = 3.0 * piece.d;
    let qb = 2.0 * piece.c;
    let qc = piece.b;
    if qa.abs() > 1e-15 {
        let disc = qb * qb - 4.0 * qa * qc;
        if disc >= 0.0 {
            let sqrt_d = disc.sqrt();
            let u1 = (-qb - sqrt_d) / (2.0 * qa);
            let u2 = (-qb + sqrt_d) / (2.0 * qa);
            let x1 = piece.x0 + u1;
            let x2 = piece.x0 + u2;
            if x1 > seg_lo && x1 < seg_hi {
                breaks.push(x1);
            }
            if x2 > seg_lo && x2 < seg_hi && (x2 - x1).abs() > 1e-14 {
                breaks.push(x2);
            }
        }
    } else if qb.abs() > 1e-15 {
        let x1 = piece.x0 - qc / qb;
        if x1 > seg_lo && x1 < seg_hi {
            breaks.push(x1);
        }
    }
    breaks.push(seg_hi);
    breaks.sort_by(f64::total_cmp);

    for pair in breaks.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let fa = piece.eval(a) - y_target;
        let fb = piece.eval(b) - y_target;
        if fa.abs() <= 1e-12 {
            out.push(a);
        }
        if fb.abs() <= 1e-12 {
            out.push(b);
        }
        if fa * fb < 0.0
            && let Some(r) = brent_root(|x| piece.eval(x) - y_target, a, b, fa, fb)
        {
            out.push(r);
        }
    }
}

/// Brent's (1973) derivative-free bracketed root finder on `[a, b]` with `fa * fb < 0`.
fn brent_root<F: Fn(f64) -> f64>(f: F, mut a: f64, mut b: f64, mut fa: f64, mut fb: f64) -> Option<f64> {
    if !fa.is_finite() || !fb.is_finite() || fa * fb > 0.0 {
        return None;
    }
    let mut c = a;
    let mut fc = fa;
    let mut d = b - a;
    let mut e = d;
    let tol = 1e-13 * (a.abs().max(b.abs()).max(1.0));

    for _ in 0..100 {
        if (fb > 0.0 && fc > 0.0) || (fb < 0.0 && fc < 0.0) {
            c = a;
            fc = fa;
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }
        let tol1 = 2.0 * f64::EPSILON * b.abs() + 0.5 * tol;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 || fb == 0.0 {
            return Some(b);
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            let s = fb / fa;
            let (p, q) = if (a - c).abs() <= f64::EPSILON {
                (2.0 * xm * s, 1.0 - s)
            } else {
                let q0 = fa / fc;
                let r = fb / fc;
                (s * (2.0 * xm * q0 * (q0 - r) - (b - a) * (r - 1.0)), (q0 - 1.0) * (r - 1.0) * (s - 1.0))
            };
            let (p_adj, q_adj) = if p > 0.0 { (p, -q) } else { (-p, q) };
            let min1 = 3.0 * xm * q_adj - (tol1 * q_adj).abs();
            let min2 = (e * q_adj).abs();
            if 2.0 * p_adj < min1.min(min2) {
                e = d;
                d = p_adj / q_adj;
            } else {
                d = xm;
                e = d;
            }
        } else {
            d = xm;
            e = d;
        }
        a = b;
        fa = fb;
        if d.abs() > tol1 {
            b += d;
        } else {
            b += tol1.copysign(xm);
        }
        fb = f(b);
        if !fb.is_finite() {
            return None;
        }
    }
    Some(b)
}
