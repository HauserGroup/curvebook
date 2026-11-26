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
    ResultStatus, ResultTab, ResultTabKind, SplineLowessMethod, SplineLowessSpec, TableArchetype, XColumnMode, XyTable,
};
use curvebook_math::sum;

use crate::error::TransformError;

/// Exact-evaluation threshold for LOWESS: series with `n <= LOWESS_EXACT_MAX_N` or
/// `n * q <= LOWESS_EXACT_WORK_LIMIT` evaluate every distinct knot directly (`delta = 0`);
/// larger series switch to Cleveland (1979) delta-anchor linear interpolation to prevent `O(N^2)` hangs.
const LOWESS_EXACT_MAX_N: usize = 200;
const LOWESS_EXACT_WORK_LIMIT: usize = 500_000;
const LOWESS_ANCHOR_TARGET: usize = 250;
/// Safe upper ceiling on `ln(lambda)` in Reinsch smoothing-spline root search (`exp(700) < f64::MAX`).
const MAX_SMOOTHING_LOG_LAMBDA: f64 = 700.0;

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
            Self::Lowess { xs, ys, rob_weights, q, fitted_y } => {
                if fitted_y.len() == xs.len()
                    && xs.len() > LOWESS_EXACT_MAX_N
                    && xs.len().saturating_mul(*q) > LOWESS_EXACT_WORK_LIMIT
                    && let (Some(&x0), Some(&xn)) = (xs.first(), xs.last())
                    && x >= x0
                    && x <= xn
                {
                    eval_sorted_linear_interp(xs, fitted_y, x)
                } else {
                    eval_lowess_sorted_at(xs, ys, rob_weights, *q, x)
                }
            }
            Self::PiecewiseLinear { xs, ys } => eval_piecewise_linear(xs, ys, x),
        }
    }

    /// Solve `S(x) = y_target` for all real roots `x` inside `[x_lo, x_hi]`, sorted ascending and deduplicated.
    #[must_use]
    pub fn solve_inverse(&self, y_target: f64, x_lo: f64, x_hi: f64) -> Vec<f64> {
        if !y_target.is_finite() || !x_lo.is_finite() || !x_hi.is_finite() || x_hi < x_lo {
            return Vec::new();
        }
        let span = (x_hi - x_lo).abs();
        let x_scale = span.max(x_lo.abs()).max(x_hi.abs());
        let dedup_tol = (1e-9 * span).max(16.0 * f64::EPSILON * x_scale);
        let mut roots = Vec::new();

        match self {
            Self::PiecewiseCubic { pieces, left_slope, right_slope, x_min, x_max } => {
                if let Some(first) = pieces.first()
                    && x_lo < *x_min
                {
                    solve_linear_tail_root(*x_min, first.a, *left_slope, y_target, x_lo, x_hi.min(*x_min), x_scale, &mut roots);
                }
                for piece in pieces {
                    let seg_lo = piece.x0.max(x_lo);
                    let seg_hi = piece.x1.min(x_hi);
                    if seg_hi < seg_lo {
                        continue;
                    }
                    solve_cubic_piece_roots(piece, y_target, seg_lo, seg_hi, &mut roots);
                }
                if let Some(last) = pieces.last()
                    && x_hi > *x_max
                {
                    solve_linear_tail_root(*x_max, last.eval(*x_max), *right_slope, y_target, x_lo.max(*x_max), x_hi, x_scale, &mut roots);
                }
            }
            Self::PiecewiseLinear { xs, ys } => {
                solve_piecewise_linear_roots(xs, ys, y_target, x_lo, x_hi, x_scale, &mut roots);
            }
            Self::Lowess { xs, ys, fitted_y, .. } => {
                let mut y_scale = y_target.abs();
                for &y in ys.iter().chain(fitted_y.iter()) {
                    if y.is_finite() {
                        y_scale = y_scale.max(y.abs());
                    }
                }
                let res_tol = 1e-12 * y_scale;
                if x_hi == x_lo {
                    let f0 = self.eval(x_lo) - y_target;
                    if f0.is_finite() && f0.abs() <= res_tol {
                        roots.push(x_lo);
                    }
                } else {
                    let n_steps = (4 * xs.len()).clamp(200, 2_000);
                    let step = (x_hi - x_lo) / (n_steps as f64);
                    let mut prev_x = x_lo;
                    let mut prev_f = self.eval(prev_x) - y_target;
                    if prev_f.is_finite() && prev_f.abs() <= res_tol {
                        roots.push(prev_x);
                    }
                    for k in 1..=n_steps {
                        let curr_x = if k == n_steps { x_hi } else { x_lo + (k as f64) * step };
                        let curr_f = self.eval(curr_x) - y_target;
                        if curr_f.is_finite() && curr_f.abs() <= res_tol {
                            roots.push(curr_x);
                        } else if prev_f.is_finite()
                            && curr_f.is_finite()
                            && prev_f.abs() > res_tol
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
        let h = xs[i + 1] - xs[i];
        if !h.is_finite() || h <= 0.0 {
            return Err(TransformError::InvalidInput { message: String::from("Akima knots must be strictly increasing with finite spacing") });
        }
        let s = (ys[i + 1] - ys[i]) / h;
        if !s.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("Akima secant slope overflowed to non-finite value") });
        }
        sec[i + 2] = s;
    }
    // Boundary chord extensions per Akima (1970) Eq. (5)-(6).
    sec[1] = 2.0 * sec[2] - sec[3];
    sec[0] = 2.0 * sec[1] - sec[2];
    sec[n_seg + 2] = 2.0 * sec[n_seg + 1] - sec[n_seg];
    sec[n_seg + 3] = 2.0 * sec[n_seg + 2] - sec[n_seg + 1];
    if sec.iter().any(|v| !v.is_finite()) {
        return Err(TransformError::InvalidInput { message: String::from("Akima boundary secant extension overflowed to non-finite value") });
    }

    let mut slopes = vec![0.0_f64; m];
    for i in 0..m {
        let d_m2 = sec[i];
        let d_m1 = sec[i + 1];
        let d_0 = sec[i + 2];
        let d_p1 = sec[i + 3];
        let w1 = (d_p1 - d_0).abs();
        let w2 = (d_m1 - d_m2).abs();
        let denom = w1 + w2;
        slopes[i] = if denom > 0.0 && denom.is_finite() {
            (w1 / denom) * d_m1 + (w2 / denom) * d_0
        } else if !denom.is_finite() {
            let scale = w1.max(w2);
            if scale.is_finite() && scale > 0.0 {
                let s1 = w1 / scale;
                let s2 = w2 / scale;
                let s_den = s1 + s2;
                (s1 / s_den) * d_m1 + (s2 / s_den) * d_0
            } else {
                0.5 * d_m1 + 0.5 * d_0
            }
        } else {
            0.5 * d_m1 + 0.5 * d_0
        };
    }

    let mut pieces = Vec::with_capacity(n_seg);
    for i in 0..n_seg {
        let h = xs[i + 1] - xs[i];
        let delta = sec[i + 2];
        let t0 = slopes[i];
        let t1 = slopes[i + 1];
        let c = (3.0 * delta - 2.0 * t0 - t1) / h;
        let d = ((t0 + t1 - 2.0 * delta) / h) / h;
        pieces.push(CubicPiece { x0: xs[i], x1: xs[i + 1], a: ys[i], b: t0, c, d });
    }
    let left_slope = slopes[0];
    let right_slope = slopes[m - 1];
    validate_piecewise_cubic(pieces, left_slope, right_slope, xs[0], xs[m - 1])
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
        return fit_smoothing_linear_limit(&xs, &ys, &ws);
    }

    let reinsch = ReinschSystem::build(&xs, &ys, &ws)?;
    let (lambda, achieved_df) = reinsch.solve_lambda_for_df(df_goal)?;
    if !lambda.is_finite() {
        return fit_smoothing_linear_limit(&xs, &ys, &ws);
    }
    let (g_vals, gammas) = reinsch.solve_for_lambda(lambda)?;
    let model = build_cubic_from_values_and_second_derivs(&xs, &g_vals, &gammas)?;
    let rss = weighted_rss(&ys, &g_vals, &ws);
    Ok(SmoothingSplineFit { model, effective_df: achieved_df, lambda, weighted_rss: rss })
}

fn fit_smoothing_linear_limit(xs: &[f64], ys: &[f64], ws: &[f64]) -> Result<SmoothingSplineFit, TransformError> {
    let m = xs.len();
    let (intercept, slope) = weighted_ols_line(xs, ys, ws)?;
    let fitted: Vec<f64> = xs.iter().map(|&x| intercept + slope * x).collect();
    if fitted.iter().any(|v| !v.is_finite()) {
        return Err(TransformError::InvalidInput { message: String::from("smoothing spline linear limit overflowed to non-finite values") });
    }
    let rss = weighted_rss(ys, &fitted, ws);
    let model = build_linear_cubic_piece(xs[0], fitted[0], xs[m - 1], fitted[m - 1])?;
    Ok(SmoothingSplineFit { model, effective_df: 2.0, lambda: f64::INFINITY, weighted_rss: rss })
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
    let (fitted_y, rob_weights) = fit_lowess_sorted_values(&xs, &ys, q, robustness_iters);

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
    for i in 0..xs.len() - 1 {
        let h = xs[i + 1] - xs[i];
        if !h.is_finite() || h <= 0.0 || !((ys[i + 1] - ys[i]) / h).is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("point-to-point curve segment slope overflowed to non-finite value") });
        }
    }
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
        if let FittedCurveModel::Lowess { ys, fitted_y, .. } = &model
            && ys.len() == fitted_y.len()
            && ys.len() == obs.len()
        {
            for (&y, &fy) in ys.iter().zip(fitted_y.iter()) {
                let r = y - fy;
                rss_acc.add(r * r);
            }
        } else {
            for &(x, y) in &obs {
                let r = y - model.eval(x);
                rss_acc.add(r * r);
            }
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

fn x_row_is_blank_unknown(xy: &XyTable, row: usize) -> bool {
    match &xy.x_mode {
        XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => {
            xy.x_cells.get(row).is_some_and(|subs| !subs.is_empty() && subs.iter().all(|cell| !cell.excluded && cell.is_empty()))
        }
        XColumnMode::GeneratedSequence { .. } => false,
    }
}

fn y_cells_are_blank_unknown(subcells: &[Cell]) -> bool {
    !subcells.is_empty() && subcells.iter().all(|cell| !cell.excluded && cell.is_empty())
}

fn extract_xy_observations_and_unknowns(xy: &XyTable, ds_idx: usize) -> Result<ExtractedXySeries, TransformError> {
    let ds = &xy.data_sets[ds_idx];
    let mut obs = Vec::new();
    let mut forward_unknowns = Vec::new();
    let mut inverse_unknowns = Vec::new();

    for row in 0..xy.row_count() {
        let x_opt = xy.x_summary_at(row)?.filter(|v| v.is_finite()).map(|x| if x == 0.0 { 0.0 } else { x });
        let x_blank = x_opt.is_none() && x_row_is_blank_unknown(xy, row);
        // Excluded or non-blank non-numeric X rows must be skipped outright rather than
        // masquerading as blank X cells queued for inverse interpolation (parity with `fit/src/types.rs`).
        if x_opt.is_none() && !x_blank {
            continue;
        }
        let subcells = ds.cells.get(row).map_or(&[][..], Vec::as_slice);

        if xy.y_subcolumn_mode.supports_parametric_moments() {
            if x_blank {
                if let Some(y) = subcells.first().and_then(Cell::active_number) {
                    inverse_unknowns.push((row, y));
                }
                continue;
            }
            let pt_opt = xy.summarized_y_point(row, ds_idx)?;
            match (x_opt, pt_opt) {
                (Some(x), Some(pt)) if pt.mean.is_finite() => obs.push((x, pt.mean)),
                (Some(x), None) if y_cells_are_blank_unknown(subcells) => forward_unknowns.push((row, x)),
                _ => {}
            }
        } else if xy.y_subcolumn_mode.is_pre_summarized() {
            match (x_opt, subcells.first().and_then(Cell::active_number)) {
                (Some(x), Some(y)) => obs.push((x, y)),
                (None, Some(y)) if x_blank => inverse_unknowns.push((row, y)),
                (Some(x), None) if y_cells_are_blank_unknown(subcells) => forward_unknowns.push((row, x)),
                _ => {}
            }
        } else {
            let y_vals: Vec<f64> = subcells.iter().filter_map(Cell::active_number).collect();
            match (x_opt, y_vals.is_empty()) {
                (Some(x), false) => {
                    for y in y_vals {
                        obs.push((x, y));
                    }
                }
                (Some(x), true) if y_cells_are_blank_unknown(subcells) => {
                    forward_unknowns.push((row, x));
                }
                (None, false) if x_blank => {
                    let y_mean = sum::mean(&y_vals)?;
                    inverse_unknowns.push((row, y_mean));
                }
                _ => {}
            }
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

fn validate_piecewise_cubic(
    pieces: Vec<CubicPiece>,
    left_slope: f64,
    right_slope: f64,
    x_min: f64,
    x_max: f64,
) -> Result<FittedCurveModel, TransformError> {
    if !left_slope.is_finite() || !right_slope.is_finite() || !x_min.is_finite() || !x_max.is_finite() {
        return Err(TransformError::InvalidInput { message: String::from("spline boundary parameters overflowed to non-finite values") });
    }
    for p in &pieces {
        if !p.x0.is_finite() || !p.x1.is_finite() || !p.a.is_finite() || !p.b.is_finite() || !p.c.is_finite() || !p.d.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("spline polynomial coefficients overflowed to non-finite values") });
        }
    }
    Ok(FittedCurveModel::PiecewiseCubic { pieces, left_slope, right_slope, x_min, x_max })
}

fn build_linear_cubic_piece(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<FittedCurveModel, TransformError> {
    let dx = x1 - x0;
    if !dx.is_finite() || dx <= 0.0 {
        return Err(TransformError::InvalidInput { message: String::from("curve requires strictly increasing X endpoints") });
    }
    let slope = (y1 - y0) / dx;
    let piece = CubicPiece { x0, x1, a: y0, b: slope, c: 0.0, d: 0.0 };
    validate_piecewise_cubic(vec![piece], slope, slope, x0, x1)
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
        if !hi.is_finite() || hi <= 0.0 {
            return Err(TransformError::InvalidInput { message: String::from("spline knots must be strictly increasing") });
        }
        let d = (ys[i + 1] - ys[i]) / hi;
        if !d.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("spline secant slope overflowed to non-finite value") });
        }
        h.push(hi);
        delta.push(d);
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
        if !hi.is_finite() || hi <= 0.0 {
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
    validate_piecewise_cubic(pieces, left_slope, right_slope, xs[0], xs[m - 1])
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
    if x.iter().any(|v| !v.is_finite()) {
        return Err(TransformError::InvalidInput { message: String::from("tridiagonal spline solve produced non-finite values") });
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

        if r_diag.iter().any(|v| !v.is_finite())
            || r_sup1.iter().any(|v| !v.is_finite())
            || b_diag.iter().any(|v| !v.is_finite())
            || b_sup1.iter().any(|v| !v.is_finite())
            || b_sup2.iter().any(|v| !v.is_finite())
            || qt_y.iter().any(|v| !v.is_finite())
        {
            return Err(TransformError::InvalidInput {
                message: String::from("smoothing spline system coefficients overflowed to non-finite values"),
            });
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
        let max_b = self.b_diag.iter().chain(self.b_sup1.iter()).chain(self.b_sup2.iter()).fold(0.0_f64, |m, &v| m.max(v.abs()));
        let max_log_lambda = if max_b > 0.0 && max_b.is_finite() {
            (1e290_f64 / max_b).ln().clamp(-MAX_SMOOTHING_LOG_LAMBDA, MAX_SMOOTHING_LOG_LAMBDA)
        } else {
            MAX_SMOOTHING_LOG_LAMBDA
        };
        let min_log_lambda = -MAX_SMOOTHING_LOG_LAMBDA;

        let scale = if tr_b > 0.0 && tr_r > 0.0 && tr_r.is_finite() && tr_b.is_finite() { tr_r / tr_b } else { 1.0 };
        let log_mid = if scale.is_finite() && scale > 0.0 { scale.ln().clamp(min_log_lambda, max_log_lambda) } else { 0.0 };

        let mut lo = log_mid;
        let mut hi = log_mid;
        let mut f_lo = self.effective_df(lo.exp())? - target_df;
        let mut f_hi = f_lo;

        let mut step = 1.0_f64;
        for _ in 0..80 {
            if f_lo < 0.0 && lo > min_log_lambda {
                lo = (lo - step).max(min_log_lambda);
                f_lo = self.effective_df(lo.exp())? - target_df;
                step *= 1.5;
            } else {
                break;
            }
        }
        step = 1.0;
        for _ in 0..80 {
            if f_hi > 0.0 && hi < max_log_lambda {
                let next_hi = (hi + step).min(max_log_lambda);
                match self.effective_df(next_hi.exp()) {
                    Ok(df_next) => {
                        hi = next_hi;
                        f_hi = df_next - target_df;
                        step *= 1.5;
                    }
                    Err(_) => {
                        // Extreme penalty conditioning reached; fall back cleanly to the linear limit.
                        return Ok((f64::INFINITY, 2.0));
                    }
                }
            } else {
                break;
            }
        }

        if f_hi > 1e-10 {
            // Even at the maximum safe finite penalty, effective df remains above target_df -> linear limit.
            return Ok((f64::INFINITY, 2.0));
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

pub(crate) fn fit_lowess_sorted_values(xs: &[f64], ys: &[f64], q: usize, robustness_iters: usize) -> (Vec<f64>, Vec<f64>) {
    let n = xs.len();
    let mut rob_weights = vec![1.0_f64; n];
    let mut fitted_y = ys.to_vec();
    if n == 0 {
        return (fitted_y, rob_weights);
    }

    for iter in 0..=robustness_iters {
        lowess_pass_sorted(xs, ys, &rob_weights, q, &mut fitted_y);
        if iter < robustness_iters {
            update_bisquare_weights(ys, &fitted_y, &mut rob_weights);
        }
    }
    (fitted_y, rob_weights)
}

fn lowess_pass_sorted(xs: &[f64], ys: &[f64], rob_weights: &[f64], q: usize, fitted_y: &mut [f64]) {
    let n = xs.len();
    if n <= LOWESS_EXACT_MAX_N || n.saturating_mul(q) <= LOWESS_EXACT_WORK_LIMIT {
        for i in 0..n {
            if i > 0 && xs[i] == xs[i - 1] {
                fitted_y[i] = fitted_y[i - 1];
            } else {
                fitted_y[i] = eval_lowess_sorted_at(xs, ys, rob_weights, q, xs[i]);
            }
        }
        return;
    }

    // Cleveland (1979) delta-anchor linear interpolation for large series (`n * q > 500_000`).
    let x_span = (xs[n - 1] - xs[0]).abs();
    let delta = if x_span.is_finite() { 0.005 * x_span } else { 0.0 };
    let max_idx_step = (n / LOWESS_ANCHOR_TARGET).max(1);

    let mut last = 0_usize;
    fitted_y[0] = eval_lowess_sorted_at(xs, ys, rob_weights, q, xs[0]);
    while last + 1 < n {
        let limit_idx = (last + max_idx_step).min(n - 1);
        let target_x = xs[last] + delta;
        let mut next = last + 1;
        while next < limit_idx && xs[next + 1] <= target_x {
            next += 1;
        }
        if xs[next] == xs[last] {
            fitted_y[next] = fitted_y[last];
        } else {
            fitted_y[next] = eval_lowess_sorted_at(xs, ys, rob_weights, q, xs[next]);
        }
        let x0 = xs[last];
        let x1 = xs[next];
        let y0 = fitted_y[last];
        let y1 = fitted_y[next];
        let dx = x1 - x0;
        for k in (last + 1)..next {
            fitted_y[k] = if dx > 0.0 && dx.is_finite() {
                let t = ((xs[k] - x0) / dx).clamp(0.0, 1.0);
                y0 + t * (y1 - y0)
            } else {
                y0
            };
        }
        last = next;
    }
}

fn lowess_window_bounds(xs: &[f64], x_target: f64, q_eff: usize) -> (usize, usize) {
    let n = xs.len();
    let mut hi = xs.partition_point(|&x| x < x_target);
    let mut lo = hi;
    while hi - lo < q_eff {
        if lo == 0 {
            hi = q_eff.min(n);
            break;
        }
        if hi == n {
            lo = n.saturating_sub(q_eff);
            break;
        }
        let dl = (x_target - xs[lo - 1]).abs();
        let dr = (xs[hi] - x_target).abs();
        if dl <= dr {
            lo -= 1;
        } else {
            hi += 1;
        }
    }
    (lo, hi)
}

/// Closed-form centered 2x2 weighted linear regression in normalized coordinates `u = (x - x_target) / max_d`.
///
/// Requires non-degenerate weighted variance (`s_uu > 1e-24 * w_sum`, rank 2) for slope estimation;
/// degrades cleanly to the weighted local mean `y_bar` at rank 1, and returns `None` when total
/// weight is zero so the caller can apply pure-tricube or unweighted fallbacks without collapsing to `0.0`.
fn lowess_local_linear_pass(xs: &[f64], ys: &[f64], rob_weights: Option<&[f64]>, lo: usize, hi: usize, x_target: f64, max_d: f64) -> Option<f64> {
    let y_ref = if ys[lo].abs() < 1e300 && ys[lo..hi].iter().all(|&y| (y - ys[lo]).is_finite()) { ys[lo] } else { 0.0 };

    let mut w_sum = sum::NeumaierSum::new();
    let mut wu_sum = sum::NeumaierSum::new();
    let mut wdy_sum = sum::NeumaierSum::new();
    for j in lo..hi {
        let u = (xs[j] - x_target) / max_d;
        let abs_u = u.abs().min(1.0);
        let tricube = (1.0 - abs_u * abs_u * abs_u).powi(3);
        let rw = rob_weights.map_or(1.0, |r| r[j].max(0.0));
        let w = (tricube * rw).max(0.0);
        if w > 0.0 {
            w_sum.add(w);
            wu_sum.add(w * u);
            wdy_sum.add(w * (ys[j] - y_ref));
        }
    }

    let tw = w_sum.total();
    if !tw.is_finite() || tw <= 0.0 {
        return None;
    }
    let u_bar = wu_sum.total() / tw;
    let dy_bar = wdy_sum.total() / tw;
    let y_bar = y_ref + dy_bar;
    if !u_bar.is_finite() || !y_bar.is_finite() {
        return None;
    }

    let mut s_uu = sum::NeumaierSum::new();
    let mut s_uy = sum::NeumaierSum::new();
    for j in lo..hi {
        let u = (xs[j] - x_target) / max_d;
        let abs_u = u.abs().min(1.0);
        let tricube = (1.0 - abs_u * abs_u * abs_u).powi(3);
        let rw = rob_weights.map_or(1.0, |r| r[j].max(0.0));
        let w = (tricube * rw).max(0.0);
        if w > 0.0 {
            let du = u - u_bar;
            let ddy = (ys[j] - y_ref) - dy_bar;
            s_uu.add(w * du * du);
            s_uy.add(w * du * ddy);
        }
    }

    let denom = s_uu.total();
    if denom.is_finite() && denom > 1e-24 * tw {
        let slope = s_uy.total() / denom;
        let y_hat = y_bar - slope * u_bar;
        if slope.is_finite() && y_hat.is_finite() {
            return Some(y_hat);
        }
    }
    Some(y_bar)
}

fn lowess_fallback_mean(ys: &[f64], rob_weights: &[f64], lo: usize, hi: usize) -> f64 {
    let mut w_sum = sum::NeumaierSum::new();
    let mut wy_sum = sum::NeumaierSum::new();
    for j in lo..hi {
        let rw = rob_weights[j].max(0.0);
        if rw > 0.0 {
            w_sum.add(rw);
            wy_sum.add(rw * ys[j]);
        }
    }
    let tw = w_sum.total();
    if tw > 0.0 && tw.is_finite() {
        let weighted = wy_sum.total() / tw;
        if weighted.is_finite() {
            return weighted;
        }
    }
    sum::mean(&ys[lo..hi]).ok().filter(|v| v.is_finite()).unwrap_or(ys[lo])
}

pub(crate) fn eval_lowess_sorted_at(xs: &[f64], ys: &[f64], rob_weights: &[f64], q: usize, x_target: f64) -> f64 {
    let n = xs.len();
    if n == 0 || !x_target.is_finite() {
        return f64::NAN;
    }
    let q_eff = q.clamp(1, n);
    let (lo, hi) = lowess_window_bounds(xs, x_target, q_eff);
    let max_d = (x_target - xs[lo]).abs().max((xs[hi - 1] - x_target).abs());
    if !max_d.is_finite() {
        return lowess_fallback_mean(ys, rob_weights, lo, hi);
    }
    if max_d == 0.0 {
        return tied_weighted_mean(xs, ys, rob_weights, x_target);
    }

    if let Some(y_hat) = lowess_local_linear_pass(xs, ys, Some(rob_weights), lo, hi, x_target, max_d) {
        return y_hat;
    }
    if let Some(y_hat) = lowess_local_linear_pass(xs, ys, None, lo, hi, x_target, max_d) {
        return y_hat;
    }
    lowess_fallback_mean(ys, rob_weights, lo, hi)
}

fn eval_sorted_linear_interp(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    let n = xs.len();
    if n == 0 {
        return f64::NAN;
    }
    let hi = xs.partition_point(|&xk| xk < x);
    if hi == 0 {
        return ys[0];
    }
    if hi >= n {
        return ys[n - 1];
    }
    if xs[hi] == x {
        return ys[hi];
    }
    let lo = hi - 1;
    let dx = xs[hi] - xs[lo];
    if dx > 0.0 && dx.is_finite() {
        let t = ((x - xs[lo]) / dx).clamp(0.0, 1.0);
        ys[lo] + t * (ys[hi] - ys[lo])
    } else {
        ys[lo]
    }
}

fn tied_weighted_mean(xs: &[f64], ys: &[f64], rob_weights: &[f64], x_target: f64) -> f64 {
    let eq_lo = xs.partition_point(|&xj| xj < x_target);
    let eq_hi = xs.partition_point(|&xj| xj <= x_target);
    if eq_lo >= eq_hi {
        return f64::NAN;
    }
    let mut w_sum = sum::NeumaierSum::new();
    let mut wy_sum = sum::NeumaierSum::new();
    for j in eq_lo..eq_hi {
        let w = rob_weights[j].max(0.0);
        if w > 0.0 {
            w_sum.add(w);
            wy_sum.add(w * ys[j]);
        }
    }
    let tw = w_sum.total();
    if tw > 0.0 && tw.is_finite() {
        let weighted = wy_sum.total() / tw;
        if weighted.is_finite() {
            return weighted;
        }
    }
    sum::mean(&ys[eq_lo..eq_hi]).unwrap_or(f64::NAN)
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

#[allow(clippy::too_many_arguments)]
fn solve_linear_tail_root(x_anchor: f64, y_anchor: f64, slope: f64, y_target: f64, seg_lo: f64, seg_hi: f64, x_scale: f64, out: &mut Vec<f64>) {
    if !y_anchor.is_finite() || !slope.is_finite() || seg_hi < seg_lo {
        return;
    }
    let y_lo = y_anchor + slope * (seg_lo - x_anchor);
    let y_hi = y_anchor + slope * (seg_hi - x_anchor);
    let y_scale =
        y_anchor.abs().max(y_target.abs()).max(if y_lo.is_finite() { y_lo.abs() } else { 0.0 }).max(if y_hi.is_finite() { y_hi.abs() } else { 0.0 });
    let res_tol = 1e-12 * y_scale;
    if slope == 0.0 {
        if (y_anchor - y_target).abs() <= res_tol {
            out.push(seg_lo);
        }
        return;
    }
    let xr = x_anchor + (y_target - y_anchor) / slope;
    let tail_tol = (1e-12 * (seg_hi - seg_lo).abs()).max(16.0 * f64::EPSILON * x_scale);
    if xr.is_finite() && xr >= seg_lo - tail_tol && xr <= seg_hi + tail_tol {
        out.push(xr.clamp(seg_lo, seg_hi));
    }
}

fn solve_piecewise_linear_roots(xs: &[f64], ys: &[f64], y_target: f64, x_lo: f64, x_hi: f64, x_scale: f64, out: &mut Vec<f64>) {
    let n = xs.len();
    if n < 2 {
        return;
    }
    if x_lo < xs[0] && xs[1] > xs[0] {
        let left_slope = (ys[1] - ys[0]) / (xs[1] - xs[0]);
        solve_linear_tail_root(xs[0], ys[0], left_slope, y_target, x_lo, x_hi.min(xs[0]), x_scale, out);
    }
    for i in 0..n - 1 {
        let (xa, xb) = (xs[i], xs[i + 1]);
        let seg_lo = xa.max(x_lo);
        let seg_hi = xb.min(x_hi);
        if seg_hi < seg_lo || xb <= xa {
            continue;
        }
        let val_a = eval_piecewise_linear(xs, ys, seg_lo);
        let val_b = eval_piecewise_linear(xs, ys, seg_hi);
        let y_scale = val_a.abs().max(val_b.abs()).max(y_target.abs());
        let res_tol = 1e-12 * y_scale;
        let ya = val_a - y_target;
        let yb = val_b - y_target;
        if ya.abs() <= res_tol {
            out.push(seg_lo);
        }
        if yb.abs() <= res_tol {
            out.push(seg_hi);
        }
        if ya.abs() > res_tol && yb.abs() > res_tol && ya * yb < 0.0 {
            let m = ya.abs().max(yb.abs());
            let sa = ya / m;
            let sb = yb / m;
            let frac = (-sa / (sb - sa)).clamp(0.0, 1.0);
            let xr = seg_lo + frac * (seg_hi - seg_lo);
            if xr.is_finite() {
                out.push(xr);
            }
        }
    }
    if x_hi > xs[n - 1] && xs[n - 1] > xs[n - 2] {
        let right_slope = (ys[n - 1] - ys[n - 2]) / (xs[n - 1] - xs[n - 2]);
        solve_linear_tail_root(xs[n - 1], ys[n - 1], right_slope, y_target, x_lo.max(xs[n - 1]), x_hi, x_scale, out);
    }
}

/// Find all real roots of `piece.eval(x) == y_target` in `[seg_lo, seg_hi]` by partitioning at
/// the exact roots of `piece.deriv(x) == 0` in dimensionless normalized coordinates `t = (x - x0) / h`.
fn solve_cubic_piece_roots(piece: &CubicPiece, y_target: f64, seg_lo: f64, seg_hi: f64, out: &mut Vec<f64>) {
    let h = piece.x1 - piece.x0;
    if !h.is_finite() || h <= 0.0 {
        return;
    }
    let mut breaks = Vec::with_capacity(4);
    breaks.push(seg_lo);

    // Transform P(x) = a + b*u + c*u^2 + d*u^3 to dimensionless t = u / h in [0, 1]:
    // P'(t) = 3*(d*h^3)*t^2 + 2*(c*h^2)*t + (b*h) = qa*t^2 + qb*t + qc (all in Y units).
    let qa_raw = 3.0 * piece.d * h * h * h;
    let qb_raw = 2.0 * piece.c * h * h;
    let qc_raw = piece.b * h;
    let deriv_scale = qa_raw.abs().max(qb_raw.abs()).max(qc_raw.abs());
    let x_eps = 1e-12 * h;
    if deriv_scale > 0.0 && deriv_scale.is_finite() {
        let qa = qa_raw / deriv_scale;
        let qb = qb_raw / deriv_scale;
        let qc = qc_raw / deriv_scale;
        if qa.abs() > 1e-12 {
            let disc = qb * qb - 4.0 * qa * qc;
            if disc >= 0.0 {
                let sqrt_d = disc.sqrt();
                let sign = if qb >= 0.0 { 1.0 } else { -1.0 };
                let q = -0.5 * (qb + sign * sqrt_d);
                let (t1, t2) = if q != 0.0 { (q / qa, qc / q) } else { (0.0, 0.0) };
                let x1 = piece.x0 + t1 * h;
                let x2 = piece.x0 + t2 * h;
                if x1.is_finite() && x1 > seg_lo && x1 < seg_hi {
                    breaks.push(x1);
                }
                if x2.is_finite() && x2 > seg_lo && x2 < seg_hi && (x2 - x1).abs() > x_eps {
                    breaks.push(x2);
                }
            }
        } else if qb.abs() > 1e-12 {
            let t1 = -qc / qb;
            let x1 = piece.x0 + t1 * h;
            if x1.is_finite() && x1 > seg_lo && x1 < seg_hi {
                breaks.push(x1);
            }
        }
    }
    breaks.push(seg_hi);
    breaks.sort_by(f64::total_cmp);

    let piece_y_scale = piece.a.abs().max(piece.eval(piece.x1).abs()).max(y_target.abs());
    for pair in breaks.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let ya = piece.eval(a);
        let yb = piece.eval(b);
        let local_y_scale = piece_y_scale.max(ya.abs()).max(yb.abs());
        let res_tol = 1e-12 * local_y_scale;
        let fa = ya - y_target;
        let fb = yb - y_target;
        let a_hit = fa.is_finite() && fa.abs() <= res_tol;
        let b_hit = fb.is_finite() && fb.abs() <= res_tol;
        if a_hit {
            out.push(a);
        }
        if b_hit {
            out.push(b);
        }
        if !a_hit
            && !b_hit
            && fa.is_finite()
            && fb.is_finite()
            && fa * fb < 0.0
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
    let tol = 1e-13 * (b - a).abs().max(a.abs().max(b.abs()));

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
            let (p, q) = if a == c {
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, DataSheet, ResultCell, ResultTabKind, SplineLowessMethod, SplineLowessSpec, SubcolumnMode, TableArchetype,
        XyTable,
    };

    use super::{
        FittedCurveModel, execute_spline_lowess, fit_akima_subspline, fit_lowess_curve, fit_natural_cubic_spline, fit_point_to_point,
        fit_smoothing_spline,
    };
    use crate::error::TransformError;

    #[test]
    fn aud_04_04_lowess_constant_midpoint_and_bisquare_peak_never_collapse_to_zero() {
        // 1. q = 2 on constant Y = 100.0 evaluated at exact midpoints and interior offsets.
        let pts: Vec<(f64, f64)> = (0..6).map(|i| (i as f64, 100.0)).collect();
        let model = fit_lowess_curve(&pts, 0.25, 0).expect("q=2 LOWESS fit");
        for xq in [0.5, 1.5, 2.5, 3.5, 4.5, 0.25, 2.75] {
            let y = model.eval(xq);
            assert!((y - 100.0).abs() < 1e-12, "LOWESS q=2 collapsed at x={xq}: got {y}");
        }

        // 2. Bisquare robustness passes on flat baseline (Y = 50.0) + narrow peak (Y = 200.0).
        let mut peak_pts: Vec<(f64, f64)> = (0..21).map(|i| (i as f64, 50.0)).collect();
        peak_pts[10].1 = 200.0;
        peak_pts[11].1 = 200.0;
        let peak_model = fit_lowess_curve(&peak_pts, 0.15, 3).expect("bisquare peak LOWESS");
        for i in 0..21 {
            let y = peak_model.eval(i as f64);
            assert!(y.is_finite() && y >= 49.0, "LOWESS peak/baseline collapsed at x={i}: got {y}");
        }
    }

    #[test]
    fn aud_04_05_solve_inverse_scale_invariance_and_extrapolated_tails() {
        // (a) Large X scale arch: interior maximum (exact analytical S(1.5e8) = 1.15) must not be skipped.
        let arch = fit_natural_cubic_spline(&[(0.0, 0.0), (1e8, 1.0), (2e8, 1.0), (3e8, 0.0)]).expect("large-X spline");
        let mid_y = arch.eval(1.5e8);
        assert!((mid_y - 1.15).abs() < 1e-12, "expected arch maximum 1.15, got {mid_y}");
        let arch_roots = arch.solve_inverse(1.1, 0.0, 3e8);
        assert_eq!(arch_roots.len(), 2, "must find both roots on large-X arch: {arch_roots:?}");
        for r in arch_roots {
            assert!((arch.eval(r) - 1.1).abs() < 1e-9, "arch root residual too large at x={r}");
        }

        // (b) Small Y scale (1e-13): out-of-range query 5e-13 must return zero bogus roots.
        let small_y = fit_natural_cubic_spline(&[(0.0, 0.0), (1.0, 1e-13), (2.0, 0.0), (3.0, 1e-13)]).expect("small-Y spline");
        assert!(small_y.solve_inverse(5e-13, 0.0, 3.0).is_empty(), "must not emit bogus roots on small-Y spline");
        let small_lowess = fit_lowess_curve(&[(0.0, 0.0), (1.0, 1e-13), (2.0, 0.0), (3.0, 1e-13)], 0.75, 0).expect("small-Y LOWESS");
        assert!(small_lowess.solve_inverse(5e-13, 0.0, 3.0).is_empty(), "must not emit bogus roots on small-Y LOWESS");

        // (c) Nanomolar & picomolar X scales: distinct roots preserved and accurately refined.
        let nano = fit_natural_cubic_spline(&[(0.0, 0.0), (5e-10, 1.0), (1e-9, 0.0)]).expect("nano-X spline");
        let nano_roots = nano.solve_inverse(0.5, 0.0, 1e-9);
        assert_eq!(nano_roots.len(), 2, "nanomolar roots must not collapse: {nano_roots:?}");
        for r in nano_roots {
            assert!((nano.eval(r) - 0.5).abs() < 1e-10, "nanomolar root inaccuracy at x={r}");
        }
        let pico = fit_natural_cubic_spline(&[(0.0, 0.0), (5e-13, 1.0), (1e-12, 0.0)]).expect("pico-X spline");
        let pico_roots = pico.solve_inverse(0.5, 0.0, 1e-12);
        assert_eq!(pico_roots.len(), 2, "picomolar roots must refine accurately: {pico_roots:?}");
        for r in pico_roots {
            assert!((pico.eval(r) - 0.5).abs() < 1e-10, "picomolar root inaccuracy at x={r}");
        }

        // (d) Extrapolated linear tails for PiecewiseCubic and PiecewiseLinear.
        let lin_cubic = fit_natural_cubic_spline(&[(0.0, 0.0), (1.0, 2.0), (2.0, 4.0)]).expect("linear cubic");
        let left_r = lin_cubic.solve_inverse(-4.0, -5.0, 5.0);
        let right_r = lin_cubic.solve_inverse(8.0, -5.0, 5.0);
        assert_eq!(left_r.len(), 1);
        assert!((left_r[0] - (-2.0)).abs() < 1e-12);
        assert_eq!(right_r.len(), 1);
        assert!((right_r[0] - 4.0).abs() < 1e-12);

        let p2p = fit_point_to_point(&[(0.0, 0.0), (1.0, 2.0), (2.0, 4.0)]).expect("p2p");
        let p2p_left = p2p.solve_inverse(-4.0, -5.0, 5.0);
        let p2p_right = p2p.solve_inverse(8.0, -5.0, 5.0);
        assert_eq!(p2p_left.len(), 1);
        assert!((p2p_left[0] - (-2.0)).abs() < 1e-12);
        assert_eq!(p2p_right.len(), 1);
        assert!((p2p_right[0] - 4.0).abs() < 1e-12);
    }

    #[test]
    fn aud_04_06_excluded_and_text_cells_not_fabricated_as_unknowns() {
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).expect("xy");
        let pts = [(0.0, 0.0), (1.0, 2.0), (2.0, 4.0), (3.0, 6.0)];
        for (r, &(x, y)) in pts.iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x)).expect("x");
            xy.data_sets[0].cells[r][0] = Cell::number(y);
        }
        // Row 4: excluded Y outlier with active X -> must NOT become forward unknown.
        xy.set_x_cell(4, Cell::number(1.5)).expect("x4");
        xy.data_sets[0].cells[4][0] = Cell::excluded_number(999.0);
        // Row 5: excluded X with active Y -> must NOT become inverse unknown.
        xy.set_x_cell(5, Cell::excluded_number(2.5)).expect("x5");
        xy.data_sets[0].cells[5][0] = Cell::number(5.0);
        // Row 6: non-empty text Y ("BLQ") with active X -> must NOT become forward unknown.
        xy.set_x_cell(6, Cell::number(2.0)).expect("x6");
        xy.data_sets[0].cells[6][0] = Cell::text("BLQ");
        // Row 7: genuinely blank Y with active X -> MUST be interpolated as forward unknown.
        xy.set_x_cell(7, Cell::number(1.5)).expect("x7");

        let sheet = DataSheet::new("excl-unk", "Excl", TableArchetype::Xy(xy)).expect("sheet");
        let spec = SplineLowessSpec { method: SplineLowessMethod::NaturalCubicSpline, interpolate_unknowns: true, ..SplineLowessSpec::default() };
        let analysis = AnalysisSpec::new("res", "excl-unk", AnalysisKind::FitSplineLowess(spec.clone())).expect("spec");
        let res = execute_spline_lowess(&sheet, &analysis, &spec).expect("execute");
        let unk_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::InterpolatedUnknowns).expect("unk tab");
        assert_eq!(unk_tab.rows.len(), 1, "only genuine blank row 7 should be interpolated, got {:?}", unk_tab.row_labels);
        assert_eq!(unk_tab.row_labels[0], "Row 8");
    }

    #[test]
    fn aud_13_20_presummarized_y_validation_rejects_malformed_summary_rows() {
        let mut xy = XyTable::new(SubcolumnMode::MeanSdN, 1, 4).expect("xy MeanSdN");
        xy.set_x_cell(0, Cell::number(0.0)).expect("x0");
        xy.data_sets[0].cells[0] = vec![Cell::number(1.0), Cell::number(0.1), Cell::number(3.0)];
        xy.set_x_cell(1, Cell::number(1.0)).expect("x1");
        xy.data_sets[0].cells[1] = vec![Cell::number(2.0), Cell::number(-0.5), Cell::number(3.0)]; // invalid negative SD
        xy.set_x_cell(2, Cell::number(2.0)).expect("x2");
        xy.data_sets[0].cells[2] = vec![Cell::number(3.0), Cell::number(0.2), Cell::number(3.0)];
        // Row 3: blank X with mean-only Y for inverse unknown must be allowed once row 1 is fixed.
        xy.data_sets[0].cells[3] = vec![Cell::number(1.5), Cell::empty(), Cell::empty()];

        let spec = SplineLowessSpec { method: SplineLowessMethod::NaturalCubicSpline, interpolate_unknowns: true, ..SplineLowessSpec::default() };
        let analysis = AnalysisSpec::new("res-ps", "ps-bad", AnalysisKind::FitSplineLowess(spec.clone())).expect("analysis");
        let bad_sheet = DataSheet::new("ps-bad", "Bad", TableArchetype::Xy(xy.clone())).expect("bad sheet");
        assert!(
            matches!(execute_spline_lowess(&bad_sheet, &analysis, &spec), Err(TransformError::SummaryPoint(_))),
            "negative SD in MeanSdN must return SummaryPoint error"
        );

        // Fix row 1 SD -> should succeed and interpolate inverse unknown row 3.
        xy.data_sets[0].cells[1][1] = Cell::number(0.2);
        let good_sheet = DataSheet::new("ps-bad", "Good", TableArchetype::Xy(xy)).expect("good sheet");
        let res = execute_spline_lowess(&good_sheet, &analysis, &spec).expect("valid MeanSdN fit");
        let unk_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::InterpolatedUnknowns).expect("unk tab");
        assert_eq!(unk_tab.rows.len(), 1);
        assert_eq!(unk_tab.row_labels[0], "Row 4");
    }

    #[test]
    fn aud_04_19_large_lowess_completes_quickly_and_reuses_fitted_y() {
        let n = 4_000_usize;
        let pts: Vec<(f64, f64)> = (0..n).map(|i| (i as f64, (i as f64 * 0.01).sin())).collect();
        let model = fit_lowess_curve(&pts, 0.25, 2).expect("large LOWESS");
        let FittedCurveModel::Lowess { fitted_y, .. } = &model else {
            panic!("expected Lowess model");
        };
        assert_eq!(fitted_y.len(), n);
        assert!(fitted_y.iter().all(|v| v.is_finite()));
        assert!(model.eval(2_000.5).is_finite());
    }

    #[test]
    fn aud_04_26_and_40_akima_overflow_guard_reinsch_clamp_and_infinite_lambda() {
        // 1. Akima with extreme spacing: must either produce finite coefficients or reject cleanly (never Ok with NaN/Inf).
        let tiny_dx = [(0.0, 0.0), (1e-160, 1.0), (2e-160, 0.5), (3e-160, 2.0)];
        if let Ok(FittedCurveModel::PiecewiseCubic { pieces, left_slope, right_slope, .. }) = fit_akima_subspline(&tiny_dx) {
            assert!(left_slope.is_finite() && right_slope.is_finite());
            for p in pieces {
                assert!(p.a.is_finite() && p.b.is_finite() && p.c.is_finite() && p.d.is_finite());
            }
        }
        // Moderate large slopes (~1e100) where old w1*d_m1 overflowed (1e200 ok, 1e160 -> w*d = 1e320 overflowed!)
        // Check y ~ 1e160 with h = 1.0: c and d are ~1e160 (finite!), while old w1*d_m1 = 1e320 overflowed!
        let large_y = [(0.0, 0.0), (1.0, 1e160), (2.0, 0.5e160), (3.0, 2e160), (4.0, 1.5e160)];
        let ak = fit_akima_subspline(&large_y).expect("normalized Akima weights must handle 1e160 ordinates");
        let FittedCurveModel::PiecewiseCubic { pieces, left_slope, right_slope, .. } = ak else {
            panic!("expected PiecewiseCubic");
        };
        assert!(left_slope.is_finite() && right_slope.is_finite());
        for p in pieces {
            assert!(p.a.is_finite() && p.b.is_finite() && p.c.is_finite() && p.d.is_finite());
        }

        // 2. Reinsch solve_lambda_for_df near df = 2.0 on large X scale must not overflow hi.exp().
        let large_x = [(0.0, 1.0), (1e6, 2.1), (2e6, 2.9), (3e6, 4.2), (4e6, 5.0)];
        let fit_near = fit_smoothing_spline(&large_x, None, Some(2.00001)).expect("near-linear smoothing spline");
        assert!(fit_near.model.eval(1.5e6).is_finite());

        // 3. AUD-04-40: df = 2.0 returns lambda = +Inf and renders "Infinity (linear limit)" without leaking f64::MAX into scalars.
        let fit_lin = fit_smoothing_spline(&large_x, None, Some(2.0)).expect("df=2 smoothing spline");
        assert!(fit_lin.lambda.is_infinite() && fit_lin.lambda.is_sign_positive());

        let mut xy = XyTable::new(SubcolumnMode::Single, 1, large_x.len()).expect("xy");
        for (r, &(x, y)) in large_x.iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x)).expect("x");
            xy.data_sets[0].cells[r][0] = Cell::number(y);
        }
        let sheet = DataSheet::new("ss-lin", "SS Lin", TableArchetype::Xy(xy)).expect("sheet");
        let spec = SplineLowessSpec { method: SplineLowessMethod::SmoothingSpline, smoothing_df: Some(2.0), ..SplineLowessSpec::default() };
        let analysis = AnalysisSpec::new("res-ss", "ss-lin", AnalysisKind::FitSplineLowess(spec.clone())).expect("analysis");
        let res = execute_spline_lowess(&sheet, &analysis, &spec).expect("execute ss");
        assert!(!res.scalars.iter().any(|s| s.key == "smoothing_lambda"), "non-finite linear-limit lambda must not leak into scalars");
        let summary = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary).expect("summary");
        let lam_row = summary.row_labels.iter().position(|l| l.contains("lambda")).expect("lambda row");
        assert_eq!(summary.rows[lam_row][0], ResultCell::Text(String::from("Infinity (linear limit)")));
    }
}
