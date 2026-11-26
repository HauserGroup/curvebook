//! Fitted regression curve resampling and confidence/prediction band overlay emission for XY scenes.

use curvebook_doc::{CurveSamplePoint, FittedCurveOverlay, LineConnectionKind, ProjectDocument, RegressionOverlaySpec, ResultStatus, SeriesSpec};
use curvebook_math::AxisScale;
use curvebook_palette::{LineDashStyle, Rgba8};

use super::series::build_connecting_path;
use super::{SegmentedAxis, is_log_like, stroke_with_dash};
use crate::scales::{ResolvedSegment, scale_inverse, scale_transform};
use crate::scene::{ChartWarning, FillPaint, Path2D, PathCmd, PointPt, RectPt, ScenePrimitive, SourceCellId};

#[derive(Debug, Clone)]
pub(super) struct FittedOverlayData {
    pub(super) spec: RegressionOverlaySpec,
    pub(super) curve: FittedCurveOverlay,
}

pub(super) fn resolve_fitted_overlay<'a>(
    doc: &'a ProjectDocument,
    series: &SeriesSpec,
    warnings: &mut Vec<ChartWarning>,
) -> Option<(FittedOverlayData, &'a [SourceCellId])> {
    let spec = series.regression_overlay.as_ref()?;
    let Some(res) = doc.result_sheet(&spec.result_sheet_id) else {
        warnings.push(ChartWarning::new("missing_regression_result", format!("regression result sheet '{}' not found", spec.result_sheet_id)));
        return None;
    };
    if let ResultStatus::Failed { message, .. } = &res.status {
        warnings.push(ChartWarning::new("failed_regression_result", format!("regression result sheet '{}' failed: {message}", spec.result_sheet_id)));
        return None;
    }
    if !matches!(res.status, ResultStatus::UpToDate) {
        // Cached curve points describe the data before the pending recalculation; drawing them would
        // present an outdated fit as current.
        warnings.push(ChartWarning::new(
            "pending_regression_result",
            format!("regression result sheet '{}' is pending recalculation; fitted curve omitted", spec.result_sheet_id),
        ));
        return None;
    }
    let Some(curve) = res.fitted_curves.iter().find(|fc| fc.data_set_index == series.data_set_index && !fc.equation_id.eq_ignore_ascii_case("ROC"))
    else {
        warnings.push(ChartWarning::new(
            "missing_fitted_curve",
            format!("regression result sheet '{}' has no fitted curve for data set {}", spec.result_sheet_id, series.data_set_index),
        ));
        return None;
    };
    if !curve.converged {
        warnings.push(ChartWarning::new("unconverged_fitted_curve", format!("fitted curve '{}' did not converge", curve.equation_id)));
    } else if curve.ambiguous {
        warnings.push(ChartWarning::new("ambiguous_fitted_curve", format!("fitted curve '{}' has ambiguous parameters", curve.equation_id)));
    }
    let mut effective_spec = spec.clone();
    let mut effective_curve = curve.clone();
    if matches!(&res.analysis.kind, curvebook_doc::AnalysisKind::NonlinearRegression(nl) if nl.method == curvebook_doc::FitMethod::RobustCauchy) {
        if effective_spec.show_confidence_band || effective_spec.show_prediction_band {
            warnings.push(ChartWarning::new(
                "robust_bands_suppressed",
                format!(
                    "confidence and prediction bands suppressed for Robust (Cauchy) regression result '{}' (NotApplicableInRobustMode)",
                    spec.result_sheet_id
                ),
            ));
        }
        effective_spec.show_confidence_band = false;
        effective_spec.show_prediction_band = false;
        for pt in &mut effective_curve.points {
            pt.ci_lower = None;
            pt.ci_upper = None;
            pt.pred_lower = None;
            pt.pred_upper = None;
        }
    }
    let outliers = if matches!(res.status, ResultStatus::UpToDate) { res.eliminated_outliers.as_slice() } else { &[] };
    Some((FittedOverlayData { spec: effective_spec, curve: effective_curve }, outliers))
}

#[derive(Clone, Copy)]
pub(super) struct OverlayDrawTarget<'a> {
    pub(super) x_axis: &'a SegmentedAxis,
    pub(super) y_axis: &'a SegmentedAxis,
    pub(super) color: Rgba8,
    pub(super) clip: RectPt,
    pub(super) dx_data: f64,
    pub(super) dx_pt: f64,
    pub(super) dy_pt: f64,
}

pub(super) fn render_fitted_curve_overlay(
    primitives: &mut Vec<ScenePrimitive>,
    warnings: &mut Vec<ChartWarning>,
    overlay: &FittedOverlayData,
    target: OverlayDrawTarget<'_>,
) {
    let mut src: Vec<CurveSamplePoint> = overlay
        .curve
        .points
        .iter()
        .copied()
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .map(|mut p| {
            p.x += target.dx_data;
            p
        })
        .filter(|p| p.x.is_finite())
        .collect();
    if src.len() < 2 {
        return;
    }
    src.sort_by(|a, b| a.x.total_cmp(&b.x));
    let total_samples = usize::from(overlay.spec.sample_segments.clamp(256, 1024));
    let total_x_span: f64 = target.x_axis.segments.iter().map(|s| (s.span_end_pt - s.span_start_pt).abs()).sum::<f64>().max(1.0);

    for seg_x in &target.x_axis.segments {
        let seg_weight = ((seg_x.span_end_pt - seg_x.span_start_pt).abs() / total_x_span).clamp(0.1, 1.0);
        let seg_samples = ((total_samples as f64) * seg_weight).round().max(16.0) as usize;
        let resampled = resample_curve_for_x_segment(&src, seg_x, seg_samples);
        if resampled.len() < 2 {
            continue;
        }
        let ctx = OverlayRenderCtx {
            spec: &overlay.spec,
            ambiguous: overlay.curve.ambiguous,
            color: target.color,
            clip: target.clip,
            dx_pt: target.dx_pt,
            dy_pt: target.dy_pt,
        };
        emit_overlay_bands_and_curve(primitives, warnings, &resampled, seg_x, target.y_axis, ctx);
    }
}

fn resample_curve_for_x_segment(src: &[CurveSamplePoint], seg_x: &ResolvedSegment, target_n: usize) -> Vec<CurveSamplePoint> {
    let Some(first_x) = src.first().map(|p| p.x) else {
        return Vec::new();
    };
    let Some(last_x) = src.last().map(|p| p.x) else {
        return Vec::new();
    };
    let Some(g_seg_min) = scale_transform(seg_x.scale, seg_x.domain_min) else {
        return Vec::new();
    };
    let Some(g_seg_max) = scale_transform(seg_x.scale, seg_x.domain_max) else {
        return Vec::new();
    };

    let seg_lo = g_seg_min.min(g_seg_max);
    let seg_hi = g_seg_min.max(g_seg_max);
    let (lo_t, hi_t) = if matches!(seg_x.scale, AxisScale::Reciprocal) && first_x * last_x <= 0.0 {
        if seg_x.domain_min > 0.0 {
            if last_x <= 0.0 {
                return Vec::new();
            }
            let Some(gx1) = scale_transform(seg_x.scale, last_x) else {
                return Vec::new();
            };
            (gx1.max(seg_lo), seg_hi)
        } else {
            if first_x >= 0.0 {
                return Vec::new();
            }
            let Some(gx0) = scale_transform(seg_x.scale, first_x) else {
                return Vec::new();
            };
            (seg_lo, gx0.min(seg_hi))
        }
    } else if matches!(seg_x.scale, AxisScale::Reciprocal) {
        if first_x * seg_x.domain_min <= 0.0 {
            return Vec::new();
        }
        match (scale_transform(seg_x.scale, first_x), scale_transform(seg_x.scale, last_x)) {
            (Some(gx0), Some(gx1)) => (gx0.min(gx1).max(seg_lo), gx0.max(gx1).min(seg_hi)),
            _ => return Vec::new(),
        }
    } else {
        match (scale_transform(seg_x.scale, first_x), scale_transform(seg_x.scale, last_x)) {
            (Some(gx0), Some(gx1)) => (gx0.min(gx1).max(seg_lo), gx0.max(gx1).min(seg_hi)),
            (None, Some(gx1)) => (seg_lo, gx1.min(seg_hi)),
            (Some(gx0), None) => (gx0.max(seg_lo), seg_hi),
            (None, None) => (seg_lo, seg_hi),
        }
    };
    if hi_t <= lo_t {
        return Vec::new();
    }

    let n = target_n.max(2);
    let use_log_interp = is_log_like(seg_x.scale) && src.iter().all(|p| p.x > 0.0);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let frac = (i as f64) / ((n - 1) as f64);
        let t = lo_t + frac * (hi_t - lo_t);
        let Some(x) = scale_inverse(seg_x.scale, t) else {
            continue;
        };
        if let Some(pt) = interpolate_curve_sample(src, x, use_log_interp) {
            out.push(pt);
        }
    }
    out
}

fn interpolate_curve_sample(src: &[CurveSamplePoint], x: f64, use_log_x: bool) -> Option<CurveSamplePoint> {
    let first = src.first()?;
    let last = src.last()?;
    if x < first.x - 1e-12 || x > last.x + 1e-12 {
        return None;
    }
    let idx = src.partition_point(|p| p.x <= x);
    if idx == 0 {
        return Some(*first);
    }
    if idx >= src.len() {
        return Some(*last);
    }
    let a = src[idx - 1];
    let b = src[idx];
    let w = if use_log_x && a.x > 0.0 && b.x > 0.0 && x > 0.0 {
        let span = b.x.ln() - a.x.ln();
        if span.abs() > 1e-15 { ((x.ln() - a.x.ln()) / span).clamp(0.0, 1.0) } else { 0.0 }
    } else {
        let span = b.x - a.x;
        if span.abs() > 1e-15 { ((x - a.x) / span).clamp(0.0, 1.0) } else { 0.0 }
    };
    let lerp = |va: f64, vb: f64| va + w * (vb - va);
    let lerp_opt = |oa: Option<f64>, ob: Option<f64>| match (oa, ob) {
        (Some(va), Some(vb)) => Some(lerp(va, vb)),
        _ => None,
    };
    Some(CurveSamplePoint {
        x,
        y: lerp(a.y, b.y),
        ci_lower: lerp_opt(a.ci_lower, b.ci_lower),
        ci_upper: lerp_opt(a.ci_upper, b.ci_upper),
        pred_lower: lerp_opt(a.pred_lower, b.pred_lower),
        pred_upper: lerp_opt(a.pred_upper, b.pred_upper),
    })
}

#[derive(Clone, Copy)]
struct OverlayRenderCtx<'a> {
    spec: &'a RegressionOverlaySpec,
    ambiguous: bool,
    color: Rgba8,
    clip: RectPt,
    dx_pt: f64,
    dy_pt: f64,
}

#[derive(Default)]
struct OverlayBandBuffers {
    pred: Vec<(PointPt, PointPt)>,
    ci: Vec<(PointPt, PointPt)>,
    curve: Vec<PointPt>,
}

fn emit_overlay_bands_and_curve(
    primitives: &mut Vec<ScenePrimitive>,
    warnings: &mut Vec<ChartWarning>,
    samples: &[CurveSamplePoint],
    seg_x: &ResolvedSegment,
    y_axis: &SegmentedAxis,
    ctx: OverlayRenderCtx<'_>,
) {
    // Split runs cleanly per Y AxisSegment so discontinuous Y segments never join across gaps.
    let mut bufs = OverlayBandBuffers::default();
    let mut active_y_seg: Option<usize> = None;
    let mut warned_clip = false;

    for sp in samples {
        let Some(raw_px) = seg_x.map_strict(sp.x) else {
            continue;
        };
        let px = raw_px + ctx.dx_pt;
        let mapped_y = y_axis.map_value(sp.y);
        if let Some((sy, py)) = mapped_y {
            if active_y_seg.is_some() && active_y_seg != Some(sy) {
                flush_overlay_run(primitives, &mut bufs, ctx);
            }
            active_y_seg = Some(sy);
            bufs.curve.push(PointPt::new(px, py + ctx.dy_pt));
            if let Some(y_seg) = y_axis.segments.get(sy) {
                if ctx.spec.show_prediction_band
                    && let (Some(lo), Some(hi)) = (sp.pred_lower, sp.pred_upper)
                {
                    let (py_lo, c1) = y_seg.map_clamped(lo);
                    let (py_hi, c2) = y_seg.map_clamped(hi);
                    if (c1 || c2) && !warned_clip && is_log_like(y_seg.scale) && (lo <= 0.0 || hi <= 0.0) {
                        warned_clip = true;
                        warnings.push(ChartWarning::new("log_axis_floor_clip", "prediction band clipped cleanly to log-axis floor"));
                    }
                    bufs.pred.push((PointPt::new(px, py_lo + ctx.dy_pt), PointPt::new(px, py_hi + ctx.dy_pt)));
                }
                if ctx.spec.show_confidence_band
                    && let (Some(lo), Some(hi)) = (sp.ci_lower, sp.ci_upper)
                {
                    let (py_lo, c1) = y_seg.map_clamped(lo);
                    let (py_hi, c2) = y_seg.map_clamped(hi);
                    if (c1 || c2) && !warned_clip && is_log_like(y_seg.scale) && (lo <= 0.0 || hi <= 0.0) {
                        warned_clip = true;
                        warnings.push(ChartWarning::new("log_axis_floor_clip", "confidence band clipped cleanly to log-axis floor"));
                    }
                    bufs.ci.push((PointPt::new(px, py_lo + ctx.dy_pt), PointPt::new(px, py_hi + ctx.dy_pt)));
                }
            }
        } else {
            flush_overlay_run(primitives, &mut bufs, ctx);
            active_y_seg = None;
        }
    }
    flush_overlay_run(primitives, &mut bufs, ctx);
}

fn flush_overlay_run(primitives: &mut Vec<ScenePrimitive>, bufs: &mut OverlayBandBuffers, ctx: OverlayRenderCtx<'_>) {
    emit_band_geometry(primitives, &bufs.pred, ctx.spec.shaded_ribbon, ctx.color.with_alpha(38), LineDashStyle::Dotted, ctx.clip);
    emit_band_geometry(primitives, &bufs.ci, ctx.spec.shaded_ribbon, ctx.color.with_alpha(68), LineDashStyle::Dashed, ctx.clip);
    if bufs.curve.len() >= 2
        && let Some(path) = build_connecting_path(&bufs.curve, LineConnectionKind::PointToPoint)
    {
        let dash = if ctx.ambiguous { LineDashStyle::Dashed } else { LineDashStyle::Solid };
        primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke_with_dash(ctx.color, 1.75, dash)), clip: Some(ctx.clip) });
    }
    bufs.pred.clear();
    bufs.ci.clear();
    bufs.curve.clear();
}

fn emit_band_geometry(
    primitives: &mut Vec<ScenePrimitive>,
    pairs: &[(PointPt, PointPt)],
    shaded: bool,
    fill_color: Rgba8,
    dash: LineDashStyle,
    clip: RectPt,
) {
    if pairs.len() < 2 {
        return;
    }
    if shaded {
        let mut cmds = Vec::with_capacity(pairs.len() * 2 + 1);
        cmds.push(PathCmd::MoveTo(pairs[0].1));
        for &(_, upper) in &pairs[1..] {
            cmds.push(PathCmd::LineTo(upper));
        }
        for &(lower, _) in pairs.iter().rev() {
            cmds.push(PathCmd::LineTo(lower));
        }
        cmds.push(PathCmd::Close);
        primitives.push(ScenePrimitive::Path {
            path: Path2D { commands: cmds },
            fill: Some(FillPaint::solid(fill_color)),
            stroke: None,
            clip: Some(clip),
        });
    } else {
        let stroke = Some(stroke_with_dash(fill_color.with_alpha(180), 0.9, dash));
        let lo_pts: Vec<PointPt> = pairs.iter().map(|p| p.0).collect();
        let hi_pts: Vec<PointPt> = pairs.iter().map(|p| p.1).collect();
        for pts in [lo_pts, hi_pts] {
            if let Some(path) = build_connecting_path(&pts, LineConnectionKind::PointToPoint) {
                primitives.push(ScenePrimitive::Path { path, fill: None, stroke, clip: Some(clip) });
            }
        }
    }
}
