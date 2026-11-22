//! Curve calculus: Area Under the Curve (AUC) with peak detection and SE propagation,
//! Savitzky-Golay / moving-average / LOWESS smoothing, differentiation, and integration.
//!
//! References (D1 citation pass 2026-11-18; Savitzky–Golay sources are cited inline):
//! - Yeh, K. C., & Kwan, K. C. (1978). A comparison of numerical integrating algorithms by
//!   trapezoidal, Lagrange, and spline approximation. *Journal of Pharmacokinetics and
//!   Biopharmaceutics*, 6(1), 79-98. `doi:10.1007/bf01066064`.
//! - Bailer, A. J. (1988). Testing for the equality of area under the curves when using destructive
//!   measurement techniques. *Journal of Pharmacokinetics and Biopharmaceutics*, 16(3), 303-309.
//!   `doi:10.1007/bf01062139`.
//! - Cleveland, W. S. (1979). Robust locally weighted regression and smoothing scatterplots.
//!   *Journal of the American Statistical Association*, 74(368), 829-836.
//!   `doi:10.1080/01621459.1979.10481038`.

use curvebook_doc::{Cell, DataSheet, SubcolumnMode, SummarizedPoint, TableArchetype, XColumnMode, XyTable};
use curvebook_math::{DenseMatrix, solve_linear_least_squares, sum};

use crate::error::TransformError;

/// Options controlling Area Under the Curve and peak detection.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AucOptions {
    /// Horizontal baseline threshold (default `0.0`).
    pub baseline: f64,
    /// Minimum peak height above baseline required to report a peak (default `0.0`).
    pub min_peak_height: f64,
    /// Minimum fraction of total positive area required to retain a peak (default `0.0`).
    pub min_area_fraction: f64,
}

impl Default for AucOptions {
    fn default() -> Self {
        Self { baseline: 0.0, min_peak_height: 0.0, min_area_fraction: 0.0 }
    }
}

/// Summary of a single detected peak excursion above the baseline.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PeakSummary {
    /// Independent *X* coordinate at the maximum height of the excursion.
    pub peak_x: f64,
    /// Raw *Y* coordinate at the peak apex (`peak_x`).
    pub peak_y: f64,
    /// Vertical amplitude above baseline (`peak_y - baseline >= 0.0`).
    pub height_above_baseline: f64,
    /// Interpolated *X* coordinate where the curve crosses upward above `baseline` (or first sample *X* if already above `baseline`).
    pub start_x: f64,
    /// Interpolated *X* coordinate where the curve crosses back down to `baseline` (or last sample *X* if still above `baseline`).
    pub end_x: f64,
    /// Positive trapezoidal area (`>= 0.0`, in `X * Y` units) enclosed between `[start_x, end_x]` and `baseline`.
    pub area: f64,
    /// Share `area / positive_area` in `[0.0, 1.0]` (`0.0` when `positive_area == 0.0`).
    pub area_fraction: f64,
}

/// Complete Area Under the Curve result with baseline partitioning, peak list, and optional SE.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AucResult {
    /// Total unsigned excursion area `positive_area + negative_area >= 0.0` relative to `baseline` (`X * Y` units).
    pub total_area: f64,
    /// Cumulative trapezoidal area (`>= 0.0`) of all curve segments lying above `baseline` (`Y > baseline`).
    pub positive_area: f64,
    /// Cumulative unsigned trapezoidal area (`>= 0.0`) of all curve segments lying below `baseline` (`Y < baseline`).
    pub negative_area: f64,
    /// Signed net trapezoidal integral `positive_area - negative_area = integral (Y(x) - baseline) dx`.
    pub net_area: f64,
    /// Sum of `area` across all retained [`Self::peaks`] passing `min_peak_height` and `min_area_fraction`.
    pub total_peak_area: f64,
    /// Analytical Bailer (1988) standard error `sqrt(sum_i w_i^2 SE_i^2)` of `net_area` assuming independent point estimates; `None` when point SEs were not supplied (`compute_auc` or single-replicate series).
    pub standard_error: Option<f64>,
    /// Chronologically ordered (`start_x` ascending) positive excursions above `baseline` meeting height and area-fraction filters.
    pub peaks: Vec<PeakSummary>,
}

/// Smoothing algorithm selection.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SmoothMethod {
    /// 5-point quadratic/cubic Savitzky–Golay local polynomial filter (Savitzky & Golay 1964).
    SavitzkyGolay5,
    /// 7-point quadratic/cubic Savitzky–Golay local polynomial filter (Savitzky & Golay 1964).
    SavitzkyGolay7,
    /// 9-point quadratic/cubic Savitzky–Golay local polynomial filter (Savitzky & Golay 1964).
    SavitzkyGolay9,
    /// Symmetric unweighted sliding-window arithmetic mean.
    MovingAverage {
        /// Odd/positive window width (`>= 1` points).
        window: usize,
    },
    /// Locally weighted linear regression smoother with tricube kernel and optional bisquare robustness iterations (Cleveland 1979).
    Lowess {
        /// Fraction of sample points in `(0.0, 1.0]` included in each local neighborhood fit.
        span: f64,
        /// Number of Tukey bisquare outlier-downweighting iterations (`0` for standard unweighted LOESS, `2`–`3` for robust LOWESS).
        robustness_iters: usize,
    },
}

/// Numerical differentiation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DerivativeOrder {
    /// First derivative `dY / dX` via non-uniform three-point Lagrange finite differences (`Y / X` units).
    First,
    /// Second derivative `d^2Y / dX^2` (`Y / X^2` units).
    Second,
}

#[derive(Debug, Clone, Copy)]
struct CurvePoint {
    x: f64,
    y: f64,
    se: Option<f64>,
}

/// Compute linear trapezoidal Area Under the Curve (Yeh & Kwan 1978) and peak summaries from
/// `(x, y)` pairs.
pub fn compute_auc(points: &[(f64, f64)], opts: &AucOptions) -> Result<AucResult, TransformError> {
    let pts: Vec<(f64, f64, Option<f64>)> = points.iter().map(|&(x, y)| (x, y, None)).collect();
    compute_auc_internal(&pts, opts)
}

/// Compute trapezoidal AUC with analytical standard-error propagation from `(x, y, se)` tuples.
///
/// `Var(AUC) = sum_i w_i^2 SE_i^2` with trapezoid weights `w_i` (Bailer 1988). This assumes the
/// point estimates are independent (separate subjects per time point); it understates the SE for
/// repeated measurements on the same subjects.
pub fn compute_auc_with_se(points: &[(f64, f64, f64)], opts: &AucOptions) -> Result<AucResult, TransformError> {
    let pts: Vec<(f64, f64, Option<f64>)> = points.iter().map(|&(x, y, se)| (x, y, Some(se))).collect();
    compute_auc_internal(&pts, opts)
}

/// Compute AUC for each dataset (or selected dataset) in an `XyTable` [`DataSheet`].
pub fn compute_sheet_auc(sheet: &DataSheet, data_set: usize, opts: &AucOptions) -> Result<AucResult, TransformError> {
    let TableArchetype::Xy(xy) = &sheet.archetype else {
        return Err(TransformError::IncompatibleTable { operation: "AUC", kind: sheet.kind() });
    };
    let Some(ds) = xy.data_sets.get(data_set) else {
        return Err(TransformError::InvalidInput { message: format!("dataset index {data_set} out of bounds") });
    };

    let pts = if xy.y_subcolumn_mode.supports_parametric_moments() {
        collect_parametric_auc_points(xy, data_set)?
    } else if xy.y_subcolumn_mode.is_pre_summarized() {
        let mut central = Vec::new();
        for row in 0..xy.row_count() {
            let Some(x_raw) = xy.x_summary_at(row)?.filter(|v| v.is_finite()) else {
                continue;
            };
            let Some(subcells) = ds.cells.get(row) else {
                continue;
            };
            if let Some(center) = subcells.first().and_then(Cell::active_number) {
                let x = if x_raw == 0.0 { 0.0 } else { x_raw };
                central.push((x, center, None));
            }
        }
        central
    } else {
        collect_replicate_auc_points(xy, ds)?
    };
    compute_auc_internal(&pts, opts)
}

fn collect_parametric_auc_points(xy: &XyTable, data_set: usize) -> Result<Vec<(f64, f64, Option<f64>)>, TransformError> {
    let mut rows: Vec<(f64, SummarizedPoint)> = Vec::new();
    for row in 0..xy.row_count() {
        let Some(x_raw) = xy.x_summary_at(row)?.filter(|v| v.is_finite()) else {
            continue;
        };
        if let Some(pt) = xy.summarized_y_point(row, data_set)?
            && pt.mean.is_finite()
            && pt.n.is_finite()
            && pt.n > 0.0
        {
            let x = if x_raw == 0.0 { 0.0 } else { x_raw };
            rows.push((x, pt));
        }
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < rows.len() {
        let x_k = rows[i].0;
        let mut j = i + 1;
        while j < rows.len() && rows[j].0 == x_k {
            j += 1;
        }
        if j - i == 1 {
            let pt = &rows[i].1;
            out.push((x_k, pt.mean, Some(pt.sd / pt.n.sqrt())));
        } else {
            let mut n_acc = sum::NeumaierSum::new();
            let mut wmean_acc = sum::NeumaierSum::new();
            let mut ssw_acc = sum::NeumaierSum::new();
            for &(_, pt) in &rows[i..j] {
                n_acc.add(pt.n);
                wmean_acc.add(pt.n * pt.mean);
                ssw_acc.add(pt.ss_within);
            }
            let n_tot = n_acc.total();
            let mean_p = wmean_acc.total() / n_tot;
            let mut ssb_acc = sum::NeumaierSum::new();
            for &(_, pt) in &rows[i..j] {
                let d = pt.mean - mean_p;
                ssb_acc.add(pt.n * d * d);
            }
            let sd_p = if n_tot > 1.0 { ((ssw_acc.total() + ssb_acc.total()) / (n_tot - 1.0)).max(0.0).sqrt() } else { 0.0 };
            out.push((x_k, mean_p, Some(sd_p / n_tot.sqrt())));
        }
        i = j;
    }
    Ok(out)
}

fn collect_replicate_auc_points(xy: &XyTable, ds: &curvebook_doc::DataSetColumn) -> Result<Vec<(f64, f64, Option<f64>)>, TransformError> {
    let mut obs: Vec<(f64, f64)> = Vec::new();
    for row in 0..xy.row_count() {
        let Some(x_raw) = xy.x_summary_at(row)?.filter(|v| v.is_finite()) else {
            continue;
        };
        let Some(subcells) = ds.cells.get(row) else {
            continue;
        };
        let x = if x_raw == 0.0 { 0.0 } else { x_raw };
        for y in subcells.iter().filter_map(Cell::active_number) {
            obs.push((x, y));
        }
    }
    obs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < obs.len() {
        let x_k = obs[i].0;
        let mut j = i + 1;
        while j < obs.len() && obs[j].0 == x_k {
            j += 1;
        }
        let ys: Vec<f64> = obs[i..j].iter().map(|&(_, y)| y).collect();
        let mean = sum::mean(&ys)?;
        let se = if ys.len() >= 2 { Some(sum::SummaryMoments::from_slice(&ys)?.sem()?) } else { None };
        out.push((x_k, mean, se));
        i = j;
    }
    Ok(out)
}

fn compute_auc_internal(raw: &[(f64, f64, Option<f64>)], opts: &AucOptions) -> Result<AucResult, TransformError> {
    if !opts.baseline.is_finite() {
        return Err(TransformError::InvalidSpec { message: String::from("AUC baseline must be finite") });
    }
    if !opts.min_peak_height.is_finite() || opts.min_peak_height < 0.0 {
        return Err(TransformError::InvalidSpec { message: format!("AUC min_peak_height must be finite and >= 0, got {}", opts.min_peak_height) });
    }
    if !opts.min_area_fraction.is_finite() || !(0.0..=1.0).contains(&opts.min_area_fraction) {
        return Err(TransformError::InvalidSpec {
            message: format!("AUC min_area_fraction must be finite and in [0, 1], got {}", opts.min_area_fraction),
        });
    }
    let sorted = prepare_sorted_points(raw)?;
    let mut pts = collapse_tied_auc_points(&sorted)?;
    if pts.len() < 2 {
        return Err(TransformError::InvalidInput { message: String::from("AUC requires at least 2 distinct finite X points") });
    }

    let (pos_area, neg_area, raw_peaks) = integrate_segments_and_peaks(&pts, opts.baseline);
    let net_area = pos_area - neg_area;
    let total_area = pos_area + neg_area;
    let se = propagate_trapezoidal_se(&mut pts);

    let qualifying: Vec<PeakSummary> = raw_peaks
        .into_iter()
        .filter(|p| p.height_above_baseline >= opts.min_peak_height)
        .filter(|p| pos_area <= 0.0 || (p.area / pos_area) >= opts.min_area_fraction)
        .map(|mut p| {
            p.area_fraction = if pos_area > 0.0 { p.area / pos_area } else { 0.0 };
            p
        })
        .collect();
    let total_peak_area = sum::sum(&qualifying.iter().map(|p| p.area).collect::<Vec<_>>());

    Ok(AucResult { total_area, positive_area: pos_area, negative_area: neg_area, net_area, total_peak_area, standard_error: se, peaks: qualifying })
}

fn collapse_tied_auc_points(pts: &[CurvePoint]) -> Result<Vec<CurvePoint>, TransformError> {
    let mut knots = Vec::with_capacity(pts.len());
    let mut i = 0_usize;
    while i < pts.len() {
        let x_k = pts[i].x;
        let mut j = i + 1;
        while j < pts.len() && pts[j].x == x_k {
            j += 1;
        }
        if j - i == 1 {
            knots.push(pts[i]);
        } else {
            let slice = &pts[i..j];
            let ys: Vec<f64> = slice.iter().map(|p| p.y).collect();
            let mean_y = sum::mean(&ys)?;
            let se = if slice.iter().all(|p| p.se.is_some()) {
                let mut var_acc = sum::NeumaierSum::new();
                for p in slice {
                    let s = p.se.unwrap_or(0.0);
                    var_acc.add(s * s);
                }
                #[allow(clippy::cast_precision_loss)]
                let m = slice.len() as f64;
                Some(var_acc.total().max(0.0).sqrt() / m)
            } else {
                None
            };
            knots.push(CurvePoint { x: x_k, y: mean_y, se });
        }
        i = j;
    }
    Ok(knots)
}

fn prepare_sorted_points(raw: &[(f64, f64, Option<f64>)]) -> Result<Vec<CurvePoint>, TransformError> {
    let mut pts = Vec::with_capacity(raw.len());
    for &(x_raw, y, se) in raw {
        if !x_raw.is_finite() || !y.is_finite() {
            return Err(TransformError::InvalidInput { message: String::from("curve points must have finite x and y") });
        }
        if let Some(s) = se
            && (!s.is_finite() || s < 0.0)
        {
            return Err(TransformError::InvalidInput { message: format!("curve standard error must be finite and >= 0, got {s}") });
        }
        let x = if x_raw == 0.0 { 0.0 } else { x_raw };
        pts.push(CurvePoint { x, y, se });
    }
    pts.sort_by(|a, b| a.x.total_cmp(&b.x));
    Ok(pts)
}

struct ActivePeak {
    start_x: f64,
    peak_x: f64,
    peak_y: f64,
    area_acc: sum::NeumaierSum,
}

fn integrate_segments_and_peaks(pts: &[CurvePoint], baseline: f64) -> (f64, f64, Vec<PeakSummary>) {
    let mut pos_acc = sum::NeumaierSum::new();
    let mut neg_acc = sum::NeumaierSum::new();
    let mut peaks = Vec::new();
    let mut active: Option<ActivePeak> = if pts[0].y > baseline {
        Some(ActivePeak { start_x: pts[0].x, peak_x: pts[0].x, peak_y: pts[0].y, area_acc: sum::NeumaierSum::new() })
    } else {
        None
    };

    for pair in pts.windows(2) {
        let p0 = pair[0];
        let p1 = pair[1];
        let dx = p1.x - p0.x;
        if dx <= 0.0 {
            continue;
        }
        let d0 = p0.y - baseline;
        let d1 = p1.y - baseline;

        if d0 >= 0.0 && d1 >= 0.0 {
            let area = 0.5 * (d0 + d1) * dx;
            pos_acc.add(area);
            if d0 > 0.0 || d1 > 0.0 {
                let pk = active.get_or_insert_with(|| ActivePeak { start_x: p0.x, peak_x: p0.x, peak_y: p0.y, area_acc: sum::NeumaierSum::new() });
                pk.area_acc.add(area);
                if p1.y > pk.peak_y {
                    pk.peak_x = p1.x;
                    pk.peak_y = p1.y;
                }
            }
            if d1 == 0.0
                && let Some(finished) = active.take()
            {
                peaks.push(finish_peak(finished, p1.x, baseline));
            }
        } else if d0 <= 0.0 && d1 <= 0.0 {
            neg_acc.add(-0.5 * (d0 + d1) * dx);
            if let Some(finished) = active.take() {
                peaks.push(finish_peak(finished, p0.x, baseline));
            }
        } else {
            let frac = d0.abs() / (d0 - d1).abs();
            let x_cross = p0.x + frac * dx;
            if d0 > 0.0 {
                let area = 0.5 * d0 * (x_cross - p0.x);
                pos_acc.add(area);
                neg_acc.add(-0.5 * d1 * (p1.x - x_cross));
                if let Some(mut pk) = active.take() {
                    pk.area_acc.add(area);
                    peaks.push(finish_peak(pk, x_cross, baseline));
                }
            } else {
                neg_acc.add(-0.5 * d0 * (x_cross - p0.x));
                let area = 0.5 * d1 * (p1.x - x_cross);
                pos_acc.add(area);
                let mut pk = ActivePeak { start_x: x_cross, peak_x: p1.x, peak_y: p1.y, area_acc: sum::NeumaierSum::new() };
                pk.area_acc.add(area);
                active = Some(pk);
            }
        }
    }

    if let Some(finished) = active.take() {
        let end_x = pts.last().map_or(finished.start_x, |p| p.x);
        peaks.push(finish_peak(finished, end_x, baseline));
    }

    (pos_acc.total(), neg_acc.total(), peaks)
}

fn finish_peak(pk: ActivePeak, end_x: f64, baseline: f64) -> PeakSummary {
    PeakSummary {
        peak_x: pk.peak_x,
        peak_y: pk.peak_y,
        height_above_baseline: pk.peak_y - baseline,
        start_x: pk.start_x,
        end_x,
        area: pk.area_acc.total(),
        area_fraction: 0.0,
    }
}

fn propagate_trapezoidal_se(pts: &mut [CurvePoint]) -> Option<f64> {
    if pts.iter().any(|p| p.se.is_none()) {
        return None;
    }
    let n = pts.len();
    let mut var_acc = sum::NeumaierSum::new();
    for i in 0..n {
        let w = if i == 0 {
            0.5 * (pts[1].x - pts[0].x)
        } else if i + 1 == n {
            0.5 * (pts[n - 1].x - pts[n - 2].x)
        } else {
            0.5 * (pts[i + 1].x - pts[i - 1].x)
        };
        let se = pts[i].se.unwrap_or(0.0);
        var_acc.add((w * se) * (w * se));
    }
    Some(var_acc.total().max(0.0).sqrt())
}

/// Smooth an `(x, y)` series using Savitzky-Golay, moving average, or LOWESS.
pub fn smooth_xy(points: &[(f64, f64)], method: SmoothMethod) -> Result<Vec<(f64, f64)>, TransformError> {
    if points.is_empty() {
        return Ok(Vec::new());
    }
    let sorted = prepare_sorted_points(&points.iter().map(|&(x, y)| (x, y, None)).collect::<Vec<_>>())?;
    let xs: Vec<f64> = sorted.iter().map(|p| p.x).collect();
    let ys: Vec<f64> = sorted.iter().map(|p| p.y).collect();

    let smoothed_y = match method {
        SmoothMethod::SavitzkyGolay5 => savitzky_golay(&xs, &ys, 5)?,
        SmoothMethod::SavitzkyGolay7 => savitzky_golay(&xs, &ys, 7)?,
        SmoothMethod::SavitzkyGolay9 => savitzky_golay(&xs, &ys, 9)?,
        SmoothMethod::MovingAverage { window } => moving_average(&ys, window)?,
        SmoothMethod::Lowess { span, robustness_iters } => lowess(&xs, &ys, span, robustness_iters)?,
    };

    Ok(xs.into_iter().zip(smoothed_y).collect())
}

fn is_uniform_grid(xs: &[f64]) -> bool {
    if xs.len() < 3 {
        return true;
    }
    let h0 = xs[1] - xs[0];
    if h0 <= 0.0 {
        return false;
    }
    xs.windows(2).all(|w| ((w[1] - w[0]) - h0).abs() <= 1e-7 * h0.abs())
}

/// Polynomial order (`d = 3`, quadratic/cubic family) for Savitzky–Golay smoothing.
///
/// At the midpoint (`t = 0`) of a symmetric uniform stencil of width `2m + 1` (`5`, `7`, `9`),
/// odd monomials (`t`, `t^3`) are orthogonal to even monomials (`1`, `t^2`) because
/// `sum_{k=-m}^{m} k^{2p+1} = 0`, so quadratic (`d = 2`) and cubic (`d = 3`) least-squares fits
/// share identical central convolution weights (Savitzky & Golay, *Anal. Chem.*, 1964, 36(8),
/// 1627–1639, Table I). Evaluating local degree-3 (`SAVITZKY_GOLAY_DEGREE`) least-squares
/// polynomials at boundary offsets, short series (`n < width`), and non-uniform abscissas
/// matches the interior filter's exact cubic reproduction order across the entire domain
/// (Gorry, *Anal. Chem.*, 1990, 62(6), 570–573).
const SAVITZKY_GOLAY_DEGREE: usize = 3;

fn savitzky_golay(xs: &[f64], ys: &[f64], width: usize) -> Result<Vec<f64>, TransformError> {
    let n = ys.len();
    if n < width {
        let deg = n.saturating_sub(1).min(SAVITZKY_GOLAY_DEGREE);
        return Ok((0..n).map(|i| local_poly_eval(xs, ys, 0..n, xs[i], deg).unwrap_or(ys[i])).collect());
    }

    let half = width / 2;
    let uniform = is_uniform_grid(xs);
    let weights: &[f64] = match width {
        5 => &[-3.0 / 35.0, 12.0 / 35.0, 17.0 / 35.0, 12.0 / 35.0, -3.0 / 35.0],
        7 => &[-2.0 / 21.0, 3.0 / 21.0, 6.0 / 21.0, 7.0 / 21.0, 6.0 / 21.0, 3.0 / 21.0, -2.0 / 21.0],
        9 => &[-21.0 / 231.0, 14.0 / 231.0, 39.0 / 231.0, 54.0 / 231.0, 59.0 / 231.0, 54.0 / 231.0, 39.0 / 231.0, 14.0 / 231.0, -21.0 / 231.0],
        _ => return Err(TransformError::InvalidSpec { message: format!("unsupported Savitzky-Golay width {width}") }),
    };

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if uniform && i >= half && i + half < n {
            let mut acc = sum::NeumaierSum::new();
            for (k, &w) in weights.iter().enumerate() {
                acc.add(w * ys[i - half + k]);
            }
            out.push(acc.total());
        } else {
            let start = if i < half {
                0
            } else if i + half >= n {
                n - width
            } else {
                i - half
            };
            let end = start + width;
            let val = local_poly_eval(xs, ys, start..end, xs[i], SAVITZKY_GOLAY_DEGREE).unwrap_or(ys[i]);
            out.push(val);
        }
    }
    Ok(out)
}

fn local_poly_eval(xs: &[f64], ys: &[f64], range: std::ops::Range<usize>, x_target: f64, degree: usize) -> Option<f64> {
    let m = range.len();
    if m == 0 {
        return None;
    }
    let max_dt = range.clone().map(|idx| (xs[idx] - x_target).abs()).fold(0.0_f64, f64::max);
    if !max_dt.is_finite() {
        return None;
    }
    if max_dt == 0.0 {
        return sum::mean(ys.get(range)?).ok().filter(|v| v.is_finite());
    }
    let p = degree.min(m - 1) + 1;
    let mut data = Vec::with_capacity(m * p);
    let mut b = Vec::with_capacity(m);
    for idx in range {
        // Normalize offsets to [-1, 1] so the intercept c_0 = P(x_target) is invariant while the
        // local Vandermonde columns stay O(1)-conditioned across tiny or huge X scales.
        let dt = (xs[idx] - x_target) / max_dt;
        if !dt.is_finite() {
            return None;
        }
        let mut pow = 1.0_f64;
        for _ in 0..p {
            data.push(pow);
            pow *= dt;
        }
        b.push(ys[idx]);
    }
    let a = DenseMatrix::from_row_major(m, p, data).ok()?;
    let (sol, _) = solve_linear_least_squares(&a, &b).ok()?;
    sol.coefficients.first().copied().filter(|v| v.is_finite())
}

fn moving_average(ys: &[f64], window: usize) -> Result<Vec<f64>, TransformError> {
    if window == 0 {
        return Err(TransformError::InvalidSpec { message: String::from("moving average window must be >= 1") });
    }
    let n = ys.len();
    let half_left = (window - 1) / 2;
    let half_right = window / 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = i.saturating_sub(half_left);
        let end = (i + half_right + 1).min(n);
        out.push(sum::sum(&ys[start..end]) / ((end - start) as f64));
    }
    Ok(out)
}

/// LOWESS of Cleveland (1979): tricube-weighted local linear fits over the nearest
/// `round(span * n)` points, with bisquare robustness weights from residuals scaled by `6 * MAD`.
fn lowess(xs: &[f64], ys: &[f64], span: f64, robustness_iters: usize) -> Result<Vec<f64>, TransformError> {
    if !span.is_finite() || span <= 0.0 || span > 1.0 {
        return Err(TransformError::InvalidSpec { message: format!("LOWESS span must be in (0, 1], got {span}") });
    }
    let n = xs.len();
    if n <= 2 {
        return Ok(ys.to_vec());
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let q = ((span * (n as f64)).round() as usize).clamp(2, n);
    let mut rob_weights = vec![1.0_f64; n];
    let mut fitted = ys.to_vec();

    for iter in 0..=robustness_iters {
        for (i, slot) in fitted.iter_mut().enumerate().take(n) {
            *slot = lowess_point(xs, ys, &rob_weights, i, q);
        }
        if iter < robustness_iters {
            update_lowess_robust_weights(ys, &fitted, &mut rob_weights);
        }
    }
    Ok(fitted)
}

fn lowess_point(xs: &[f64], ys: &[f64], rob_weights: &[f64], i: usize, q: usize) -> f64 {
    let xi = xs[i];
    let mut dists: Vec<(usize, f64)> = xs.iter().enumerate().map(|(j, &xj)| (j, (xj - xi).abs())).collect();
    dists.sort_by(|a, b| a.1.total_cmp(&b.1));
    let max_d = dists[q - 1].1;
    if max_d == 0.0 {
        return tied_lowess_mean(xs, ys, rob_weights, xi, ys[i]);
    }

    let mut data = Vec::with_capacity(q * 2);
    let mut b = Vec::with_capacity(q);
    for &(j, d) in &dists[..q] {
        let u = (d / max_d).min(1.0);
        let tricube = (1.0 - u * u * u).powi(3);
        let w = (tricube * rob_weights[j]).max(0.0).sqrt();
        data.push(w);
        data.push(w * (xs[j] - xi));
        b.push(w * ys[j]);
    }
    let Ok(a) = DenseMatrix::from_row_major(q, 2, data) else {
        return ys[i];
    };
    solve_linear_least_squares(&a, &b).ok().and_then(|(s, _)| s.coefficients.first().copied()).filter(|v| v.is_finite()).unwrap_or(ys[i])
}

fn tied_lowess_mean(xs: &[f64], ys: &[f64], rob_weights: &[f64], xi: f64, fallback: f64) -> f64 {
    let mut w_sum = sum::NeumaierSum::new();
    let mut wy_sum = sum::NeumaierSum::new();
    let mut tied_ys = Vec::new();
    for (&xj, (&yj, &rw)) in xs.iter().zip(ys.iter().zip(rob_weights.iter())) {
        if (xj - xi).abs() == 0.0 {
            tied_ys.push(yj);
            let w = rw.max(0.0);
            if w > 0.0 {
                w_sum.add(w);
                wy_sum.add(w * yj);
            }
        }
    }
    let total_w = w_sum.total();
    if total_w > 0.0 {
        let weighted = wy_sum.total() / total_w;
        if weighted.is_finite() {
            return weighted;
        }
    }
    sum::mean(&tied_ys).ok().filter(|v| v.is_finite()).unwrap_or(fallback)
}

fn update_lowess_robust_weights(ys: &[f64], fitted: &[f64], rob_weights: &mut [f64]) {
    let mut abs_res: Vec<f64> = ys.iter().zip(fitted.iter()).map(|(y, f)| (y - f).abs()).collect();
    let mut sorted = abs_res.clone();
    sorted.sort_by(f64::total_cmp);
    let mad = if sorted.len().is_multiple_of(2) { 0.5 * (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) } else { sorted[sorted.len() / 2] };
    let scale = 6.0 * mad;
    if scale <= 0.0 {
        rob_weights.fill(1.0);
        return;
    }
    for (w, r) in rob_weights.iter_mut().zip(abs_res.drain(..)) {
        let u = r / scale;
        *w = if u < 1.0 { (1.0 - u * u).powi(2) } else { 0.0 };
    }
}

/// Compute numerical first (`dy/dx`) or second (`d2y/dx2`) derivative of an `(x, y)` curve.
///
/// Uses exact 3-point Lagrange quadratic differentiation on both uniform and non-uniform grids.
pub fn differentiate_xy(points: &[(f64, f64)], order: DerivativeOrder) -> Result<Vec<(f64, f64)>, TransformError> {
    let pts = prepare_sorted_points(&points.iter().map(|&(x, y)| (x, y, None)).collect::<Vec<_>>())?;
    let n = pts.len();
    if n < 2 {
        return Err(TransformError::InvalidInput { message: String::from("differentiation requires at least 2 points") });
    }
    if n == 2 {
        let dx = pts[1].x - pts[0].x;
        if dx == 0.0 {
            return Err(TransformError::InvalidInput { message: String::from("duplicate x coordinates in differentiation") });
        }
        let slope = match order {
            DerivativeOrder::First => (pts[1].y - pts[0].y) / dx,
            DerivativeOrder::Second => 0.0,
        };
        return Ok(vec![(pts[0].x, slope), (pts[1].x, slope)]);
    }

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (i0, i1, i2) = if i == 0 {
            (0, 1, 2)
        } else if i + 1 == n {
            (n - 3, n - 2, n - 1)
        } else {
            (i - 1, i, i + 1)
        };
        let (x0, y0) = (pts[i0].x, pts[i0].y);
        let (x1, y1) = (pts[i1].x, pts[i1].y);
        let (x2, y2) = (pts[i2].x, pts[i2].y);
        let d0 = (x0 - x1) * (x0 - x2);
        let d1 = (x1 - x0) * (x1 - x2);
        let d2 = (x2 - x0) * (x2 - x1);
        if d0 == 0.0 || d1 == 0.0 || d2 == 0.0 {
            return Err(TransformError::InvalidInput { message: String::from("duplicate x coordinates in differentiation") });
        }
        let xt = pts[i].x;
        let val = match order {
            DerivativeOrder::First => y0 * ((2.0 * xt - x1 - x2) / d0) + y1 * ((2.0 * xt - x0 - x2) / d1) + y2 * ((2.0 * xt - x0 - x1) / d2),
            DerivativeOrder::Second => 2.0 * (y0 / d0 + y1 / d1 + y2 / d2),
        };
        out.push((xt, val));
    }
    Ok(out)
}

/// Compute the cumulative trapezoidal integral curve relative to `baseline`, starting at `(x_0, 0.0)`.
pub fn integrate_xy(points: &[(f64, f64)], baseline: f64) -> Result<Vec<(f64, f64)>, TransformError> {
    if !baseline.is_finite() {
        return Err(TransformError::InvalidSpec { message: String::from("integration baseline must be finite") });
    }
    if points.is_empty() {
        return Ok(Vec::new());
    }
    let pts = prepare_sorted_points(&points.iter().map(|&(x, y)| (x, y, None)).collect::<Vec<_>>())?;
    let mut out = Vec::with_capacity(pts.len());
    let mut acc = sum::NeumaierSum::new();
    out.push((pts[0].x, 0.0));
    for pair in pts.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let avg_y = 0.5 * ((pair[0].y - baseline) + (pair[1].y - baseline));
        acc.add(dx * avg_y);
        out.push((pair[1].x, acc.total()));
    }
    Ok(out)
}

/// Curve-calculus transform wrapper producing a derived `XyTable` [`DataSheet`].
pub fn apply_curve_calculus_to_sheet(sheet: &DataSheet, op: CurveSheetOp) -> Result<DataSheet, TransformError> {
    let TableArchetype::Xy(xy) = &sheet.archetype else {
        return Err(TransformError::IncompatibleTable { operation: "curve calculus", kind: sheet.kind() });
    };
    if xy.data_sets.is_empty() {
        return Err(TransformError::InvalidInput { message: String::from("XY table has no datasets") });
    }

    let mut series_curves = Vec::with_capacity(xy.data_sets.len());
    for ds_idx in 0..xy.data_sets.len() {
        series_curves.push(transform_dataset_curve(xy, ds_idx, op)?);
    }

    let (x_grid, row_map) = build_aligned_x_grid(&series_curves);
    let mut out_xy = XyTable::new_with_x_mode(
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
        SubcolumnMode::Single,
        xy.data_sets.len(),
        x_grid.len(),
    )?;
    out_xy.x_title = xy.x_title.clone();
    for (out_row, &(_, x)) in x_grid.iter().enumerate() {
        out_xy.set_x_cell(out_row, Cell::number(x))?;
    }
    for (ds_idx, curve) in series_curves.iter().enumerate() {
        if let (Some(src_ds), Some(dst_ds)) = (xy.data_sets.get(ds_idx), out_xy.data_sets.get_mut(ds_idx)) {
            dst_ds.title = src_ds.title.clone();
            for &(src_row, _, y) in curve {
                if let Some(&out_row) = row_map.get(&src_row)
                    && let Some(cell) = dst_ds.cells.get_mut(out_row).and_then(|sub| sub.first_mut())
                {
                    *cell = Cell::number(y);
                }
            }
        }
    }

    Ok(DataSheet::new(format!("{}-curve", sheet.id.as_str()), format!("{} (Curve)", sheet.title), TableArchetype::Xy(out_xy))?)
}

/// Curve-calculus operation applicable to an entire `XyTable`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveSheetOp {
    /// Smooth each series using [`SmoothMethod`].
    Smooth(SmoothMethod),
    /// Compute numerical [`DerivativeOrder::First`] (`dY/dX`) or [`DerivativeOrder::Second`] (`d^2Y/dX^2`).
    Differentiate(DerivativeOrder),
    /// Compute running cumulative trapezoidal integral `integral_{x_0}^x (Y(u) - baseline) du`.
    Integrate {
        /// Horizontal baseline subtracted from *Y* prior to cumulative integration.
        baseline: f64,
    },
}

fn transform_dataset_curve(xy: &XyTable, ds_idx: usize, op: CurveSheetOp) -> Result<Vec<(usize, f64, f64)>, TransformError> {
    let entries = extract_xy_mean_entries(xy, ds_idx)?;
    let pairs: Vec<(f64, f64)> = entries.iter().map(|&(_, x, y)| (x, y)).collect();
    let transformed = match op {
        CurveSheetOp::Smooth(m) => smooth_xy(&pairs, m)?,
        CurveSheetOp::Differentiate(order) => differentiate_xy(&pairs, order)?,
        CurveSheetOp::Integrate { baseline } => integrate_xy(&pairs, baseline)?,
    };
    if transformed.len() != entries.len() {
        return Err(TransformError::InvalidInput {
            message: format!("curve calculus point count mismatch: expected {}, got {}", entries.len(), transformed.len()),
        });
    }
    Ok(entries.into_iter().zip(transformed).map(|((row, _, _), (tx, ty))| (row, tx, ty)).collect())
}

fn build_aligned_x_grid(curves: &[Vec<(usize, f64, f64)>]) -> (Vec<(usize, f64)>, std::collections::HashMap<usize, usize>) {
    let mut all_rows: Vec<(usize, f64)> = curves.iter().flat_map(|c| c.iter().map(|&(row, x, _)| (row, x))).collect();
    all_rows.sort_by_key(|&(row, _)| row);
    all_rows.dedup_by_key(|&mut (row, _)| row);
    all_rows.sort_by(|&(r_a, x_a), &(r_b, x_b)| x_a.total_cmp(&x_b).then_with(|| r_a.cmp(&r_b)));
    let mut row_map = std::collections::HashMap::with_capacity(all_rows.len());
    for (out_row, &(src_row, _)) in all_rows.iter().enumerate() {
        row_map.insert(src_row, out_row);
    }
    (all_rows, row_map)
}

fn extract_xy_mean_entries(xy: &XyTable, ds_idx: usize) -> Result<Vec<(usize, f64, f64)>, TransformError> {
    let Some(ds) = xy.data_sets.get(ds_idx) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (row, subcells) in ds.cells.iter().enumerate() {
        let Some(x) = xy.x_summary_at(row)? else {
            continue;
        };
        if xy.y_subcolumn_mode.is_pre_summarized() {
            if let Some(m) = subcells.first().and_then(Cell::active_number) {
                entries.push((row, x, m));
            }
        } else {
            let active: Vec<f64> = subcells.iter().filter_map(Cell::active_number).collect();
            if !active.is_empty()
                && let Ok(m) = sum::mean(&active)
            {
                entries.push((row, x, m));
            }
        }
    }
    // Match `prepare_sorted_points` ordering so `transformed` aligns 1:1 with `entries`.
    entries.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    Ok(entries)
}
