//! Theoretical continuous (`Normal`, `StudentT`, `FisherF`, `ChiSquare`) tail-shading polygon
//! and discrete (`Binomial`, `Poisson`) integer-step PMF lollipop / spike / bar overlay rendering (§6.2).

use curvebook_doc::{
    AnalysisKind, CurveSamplePoint, DistributionTailShade, FittedCurveOverlay, GraphicalDataObject, LineConnectionKind, ResultSheet, SeriesSpec,
    TheoreticalDistributionFamily, TheoreticalDistributionSpec,
};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};

use super::fit_overlay::{OverlayDrawTarget, interpolate_curve_sample};
use super::series::build_connecting_path;
use super::{SegmentedAxis, stroke_with_dash};
use crate::markers::emit_marker;
use crate::scene::{ChartWarning, FillPaint, Path2D, PathCmd, PointPt, RectPt, ScenePrimitive, StrokePaint};

/// One accompanying shaded tail or interval overlay attached to a primary §6.2 distribution curve.
#[derive(Debug, Clone)]
pub(super) struct AutoShadeOverlay {
    pub(super) shade_slot: usize,
    pub(super) label: String,
    pub(super) points: Vec<CurveSamplePoint>,
}

/// Distribution-specific rendering metadata resolved for a §6.2 theoretical distribution overlay.
#[derive(Debug, Clone)]
pub(super) struct DistributionOverlayMeta {
    pub(super) is_discrete: bool,
    pub(super) cumulative: bool,
    pub(super) is_shade_series: bool,
    pub(super) shade_slot: Option<usize>,
    pub(super) tail_shade: DistributionTailShade,
    /// Full unmasked main curve points (used to interpolate exact cutoff edges for shaded tails).
    pub(super) main_curve_points: Vec<CurveSamplePoint>,
    /// Accompanying shaded regions auto-rendered when no sibling series explicitly binds them.
    pub(super) auto_shades: Vec<AutoShadeOverlay>,
}

pub(super) fn distribution_spec_of(res: &ResultSheet) -> Option<&TheoreticalDistributionSpec> {
    match &res.analysis.kind {
        AnalysisKind::NonlinearRegression(nl) => nl.plot_distribution.as_ref(),
        _ => None,
    }
}

pub(super) fn is_discrete_distribution_eq(eq_id: &str) -> bool {
    eq_id == "dist_binomial" || eq_id == "dist_poisson" || eq_id.starts_with("dist_binomial_shade_") || eq_id.starts_with("dist_poisson_shade_")
}

pub(super) fn is_continuous_distribution_eq(eq_id: &str) -> bool {
    eq_id == "dist_normal"
        || eq_id == "dist_student_t"
        || eq_id == "dist_fisher_f"
        || eq_id == "dist_chi_square"
        || eq_id.starts_with("dist_normal_shade_")
        || eq_id.starts_with("dist_student_t_shade_")
        || eq_id.starts_with("dist_fisher_f_shade_")
        || eq_id.starts_with("dist_chi_square_shade_")
}

pub(super) fn is_distribution_overlay_eq(eq_id: &str) -> bool {
    is_discrete_distribution_eq(eq_id) || is_continuous_distribution_eq(eq_id)
}

fn parse_shade_slot(eq_id: &str) -> Option<usize> {
    let (_, tail) = eq_id.rsplit_once("_shade_")?;
    tail.parse::<usize>().ok().filter(|&n| n >= 1)
}

fn default_shade_label(tail_shade: DistributionTailShade, slot: usize) -> String {
    match tail_shade {
        DistributionTailShade::LowerTail { .. } => String::from("Lower tail"),
        DistributionTailShade::UpperTail { .. } => String::from("Upper tail"),
        DistributionTailShade::Between { .. } => String::from("Between"),
        DistributionTailShade::TwoTailed { .. } => {
            if slot == 2 {
                String::from("Upper tail")
            } else {
                String::from("Lower tail")
            }
        }
        DistributionTailShade::None => format!("Shaded region {slot}"),
    }
}

fn shade_label_for_result(res: &ResultSheet, tail_shade: DistributionTailShade, ds_idx: usize, slot: usize) -> String {
    if let Some(ds) = res.analyzable_data.as_ref()
        && let curvebook_doc::TableArchetype::Xy(xy) = &ds.archetype
        && let Some(col) = xy.data_sets.get(ds_idx)
        && !col.title.trim().is_empty()
    {
        return col.title.clone();
    }
    default_shade_label(tail_shade, slot)
}

/// Resolve [`DistributionOverlayMeta`] for a `FittedCurveOverlay` coming from `res`.
///
/// `has_sibling_shade_series` is `true` when another `SeriesSpec` in the same graph explicitly
/// binds `data_set_index >= 1` on `res` so auto-shading does not double-render shaded regions.
pub(super) fn resolve_distribution_overlay_meta(
    res: &ResultSheet,
    curve: &FittedCurveOverlay,
    has_sibling_shade_series: bool,
) -> Option<DistributionOverlayMeta> {
    let dist_spec = distribution_spec_of(res);
    if dist_spec.is_none() && !is_distribution_overlay_eq(&curve.equation_id) {
        return None;
    }
    let is_discrete = dist_spec.map_or_else(
        || is_discrete_distribution_eq(&curve.equation_id),
        |s| matches!(s.family, TheoreticalDistributionFamily::Binomial | TheoreticalDistributionFamily::Poisson),
    );
    let cumulative = dist_spec.is_some_and(|s| s.cumulative);
    let tail_shade = dist_spec.map_or(DistributionTailShade::None, |s| s.tail_shade);
    let shade_slot = parse_shade_slot(&curve.equation_id).or_else(|| (curve.data_set_index >= 1).then_some(curve.data_set_index));
    let is_shade_series = shade_slot.is_some();

    let main_curve_points = res
        .fitted_curves
        .iter()
        .find(|fc| fc.data_set_index == 0 && !fc.equation_id.contains("_shade_"))
        .map_or_else(|| curve.points.clone(), |fc| fc.points.clone());

    let auto_shades = if !is_shade_series && !has_sibling_shade_series {
        res.fitted_curves
            .iter()
            .filter(|fc| fc.data_set_index >= 1 || fc.equation_id.contains("_shade_"))
            .map(|fc| {
                let slot = parse_shade_slot(&fc.equation_id).unwrap_or(fc.data_set_index.max(1));
                AutoShadeOverlay {
                    shade_slot: slot,
                    label: shade_label_for_result(res, tail_shade, fc.data_set_index, slot),
                    points: fc.points.clone(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    Some(DistributionOverlayMeta { is_discrete, cumulative, is_shade_series, shade_slot, tail_shade, main_curve_points, auto_shades })
}

/// Resolve [`DistributionOverlayMeta`] when a `GraphKind::Xy` series plots `dist_res.analyzable_data`
/// directly (without an explicit `RegressionOverlaySpec`).
pub(super) fn resolve_direct_distribution_meta(
    res: &ResultSheet,
    data_set_index: usize,
    has_sibling_shade_series: bool,
) -> Option<(DistributionOverlayMeta, Vec<CurveSamplePoint>)> {
    let dist_spec = distribution_spec_of(res)?;
    let is_discrete = matches!(dist_spec.family, TheoreticalDistributionFamily::Binomial | TheoreticalDistributionFamily::Poisson);
    let shade_slot = (data_set_index >= 1).then_some(data_set_index);
    let is_shade_series = shade_slot.is_some();

    let curve_pts = res.fitted_curves.iter().find(|fc| fc.data_set_index == data_set_index).map(|fc| fc.points.clone()).unwrap_or_default();
    let main_curve_points = res.fitted_curves.iter().find(|fc| fc.data_set_index == 0).map_or_else(|| curve_pts.clone(), |fc| fc.points.clone());

    let auto_shades = if !is_shade_series && !has_sibling_shade_series {
        res.fitted_curves
            .iter()
            .filter(|fc| fc.data_set_index >= 1)
            .map(|fc| {
                let slot = parse_shade_slot(&fc.equation_id).unwrap_or(fc.data_set_index.max(1));
                AutoShadeOverlay {
                    shade_slot: slot,
                    label: shade_label_for_result(res, dist_spec.tail_shade, fc.data_set_index, slot),
                    points: fc.points.clone(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    Some((
        DistributionOverlayMeta {
            is_discrete,
            cumulative: dist_spec.cumulative,
            is_shade_series,
            shade_slot,
            tail_shade: dist_spec.tail_shade,
            main_curve_points,
            auto_shades,
        },
        curve_pts,
    ))
}

pub(super) fn shade_accent_color(palette: PaletteId, primary: Rgba8, explicit_series_color: Option<Rgba8>) -> Rgba8 {
    if let Some(c) = explicit_series_color {
        return c;
    }
    let alt = palette.color_at(1);
    if alt != primary { alt } else { palette.color_at(2) }
}

fn cutoff_bounds_for_slot(tail_shade: DistributionTailShade, slot: Option<usize>) -> Option<(f64, f64)> {
    match tail_shade {
        DistributionTailShade::None => None,
        DistributionTailShade::LowerTail { cutoff } => Some((f64::NEG_INFINITY, cutoff)),
        DistributionTailShade::UpperTail { cutoff } => Some((cutoff, f64::INFINITY)),
        DistributionTailShade::Between { lower, upper } => Some((lower, upper)),
        DistributionTailShade::TwoTailed { lower, upper } => {
            if slot == Some(2) {
                Some((upper, f64::INFINITY))
            } else {
                Some((f64::NEG_INFINITY, lower))
            }
        }
    }
}

fn sorted_finite_points(pts: &[CurveSamplePoint], dx_data: f64) -> Vec<CurveSamplePoint> {
    let mut out: Vec<CurveSamplePoint> = pts
        .iter()
        .copied()
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .map(|mut p| {
            p.x += dx_data;
            p
        })
        .filter(|p| p.x.is_finite())
        .collect();
    out.sort_by(|a, b| a.x.total_cmp(&b.x));
    out
}

fn extract_clipped_continuous_region(main_src: &[CurveSamplePoint], lo: f64, hi: f64) -> Vec<CurveSamplePoint> {
    if main_src.len() < 2 || lo > hi {
        return Vec::new();
    }
    let first_x = main_src[0].x;
    let last_x = main_src[main_src.len() - 1].x;
    let eff_lo = lo.max(first_x);
    let eff_hi = hi.min(last_x);
    if eff_hi < eff_lo - 1e-12 {
        return Vec::new();
    }

    let mut out = Vec::new();
    if eff_lo.is_finite()
        && eff_lo > first_x + 1e-12
        && let Some(p_lo) = interpolate_curve_sample(main_src, eff_lo, false)
    {
        out.push(p_lo);
    }
    for &p in main_src {
        if p.x >= eff_lo - 1e-12 && p.x <= eff_hi + 1e-12 {
            if out.last().is_some_and(|prev| (prev.x - p.x).abs() <= 1e-12) {
                continue;
            }
            out.push(p);
        }
    }
    if eff_hi.is_finite()
        && eff_hi < last_x - 1e-12
        && let Some(p_hi) = interpolate_curve_sample(main_src, eff_hi, false)
        && !out.last().is_some_and(|prev| (prev.x - p_hi.x).abs() <= 1e-12)
    {
        out.push(p_hi);
    }
    out
}

fn extract_positive_y_runs(pts: &[CurveSamplePoint]) -> Vec<Vec<CurveSamplePoint>> {
    let mut runs = Vec::new();
    let mut cur = Vec::new();
    for &p in pts {
        if p.y > 0.0 {
            cur.push(p);
        } else if cur.len() >= 2 {
            runs.push(std::mem::take(&mut cur));
        } else {
            cur.clear();
        }
    }
    if cur.len() >= 2 {
        runs.push(cur);
    }
    runs
}

/// Emit a continuous shaded tail polygon (and crisp dashed cutoff edge) onto `primitives`.
fn emit_continuous_tail_polygon(
    primitives: &mut Vec<ScenePrimitive>,
    region_runs: &[Vec<CurveSamplePoint>],
    cutoffs: &[f64],
    main_src: &[CurveSamplePoint],
    shade_color: Rgba8,
    target: OverlayDrawTarget<'_>,
) {
    let fill_alpha = if shade_color.a < 255 { shade_color.a } else { 88 };
    let fill = FillPaint::solid(shade_color.with_alpha(fill_alpha));
    let (_, raw_base_py, _) = target.y_axis.map_clamped_to_nearest(0.0);
    let base_py = raw_base_py + target.dy_pt;

    for run in region_runs {
        if run.len() < 2 {
            continue;
        }
        for seg_x in &target.x_axis.segments {
            let mut seg_pts: Vec<PointPt> = Vec::new();
            let mut active_y_seg: Option<usize> = None;

            let flush_poly = |prims: &mut Vec<ScenePrimitive>, pts: &mut Vec<PointPt>| {
                if pts.len() >= 2 {
                    let mut cmds = Vec::with_capacity(pts.len() + 4);
                    cmds.push(PathCmd::MoveTo(PointPt::new(pts[0].x, base_py)));
                    for &p in pts.iter() {
                        cmds.push(PathCmd::LineTo(p));
                    }
                    if let Some(last) = pts.last() {
                        cmds.push(PathCmd::LineTo(PointPt::new(last.x, base_py)));
                    }
                    cmds.push(PathCmd::Close);
                    prims.push(ScenePrimitive::Path { path: Path2D { commands: cmds }, fill: Some(fill), stroke: None, clip: Some(target.clip) });
                }
                pts.clear();
            };

            for sp in run {
                let Some(raw_px) = seg_x.map_strict(sp.x) else {
                    flush_poly(primitives, &mut seg_pts);
                    active_y_seg = None;
                    continue;
                };
                if let Some((sy, py)) = target.y_axis.map_value(sp.y) {
                    if active_y_seg.is_some() && active_y_seg != Some(sy) {
                        flush_poly(primitives, &mut seg_pts);
                    }
                    active_y_seg = Some(sy);
                    seg_pts.push(PointPt::new(raw_px + target.dx_pt, py + target.dy_pt));
                } else {
                    flush_poly(primitives, &mut seg_pts);
                    active_y_seg = None;
                }
            }
            flush_poly(primitives, &mut seg_pts);
        }
    }

    // Crisp dashed vertical cutoff rule(s) at finite cutoff boundaries.
    let cut_stroke = stroke_with_dash(shade_color.with_alpha(205), 0.9, LineDashStyle::Dashed);
    for &cut in cutoffs {
        if !cut.is_finite() {
            continue;
        }
        let Some(p_cut) = interpolate_curve_sample(main_src, cut, false) else {
            continue;
        };
        if let (Some((_, px)), Some((_, py))) = (target.x_axis.map_value(cut), target.y_axis.map_value(p_cut.y)) {
            let x_pt = px + target.dx_pt;
            let y_top = py + target.dy_pt;
            if (base_py - y_top).abs() >= 0.5 {
                primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(x_pt, base_py), PointPt::new(x_pt, y_top)),
                    fill: None,
                    stroke: Some(cut_stroke),
                    clip: Some(target.clip),
                });
            }
        }
    }
}

fn emit_continuous_curve_stroke(primitives: &mut Vec<ScenePrimitive>, src: &[CurveSamplePoint], color: Rgba8, target: OverlayDrawTarget<'_>) {
    if src.len() < 2 {
        return;
    }
    let stroke = stroke_with_dash(color, 1.8, LineDashStyle::Solid);
    for seg_x in &target.x_axis.segments {
        let mut pts = Vec::new();
        let mut active_y_seg: Option<usize> = None;

        let flush = |prims: &mut Vec<ScenePrimitive>, buf: &mut Vec<PointPt>| {
            if buf.len() >= 2
                && let Some(path) = build_connecting_path(buf, LineConnectionKind::PointToPoint)
            {
                prims.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: Some(target.clip) });
            }
            buf.clear();
        };

        for sp in src {
            let Some(raw_px) = seg_x.map_strict(sp.x) else {
                flush(primitives, &mut pts);
                active_y_seg = None;
                continue;
            };
            if let Some((sy, py)) = target.y_axis.map_value(sp.y) {
                if active_y_seg.is_some() && active_y_seg != Some(sy) {
                    flush(primitives, &mut pts);
                }
                active_y_seg = Some(sy);
                pts.push(PointPt::new(raw_px + target.dx_pt, py + target.dy_pt));
            } else {
                flush(primitives, &mut pts);
                active_y_seg = None;
            }
        }
        flush(primitives, &mut pts);
    }
}

fn min_mapped_x_spacing_pt(pts: &[CurveSamplePoint], x_axis: &SegmentedAxis) -> f64 {
    let mut xs: Vec<f64> = pts.iter().filter_map(|p| x_axis.map_value(p.x).map(|(_, px)| px)).collect();
    xs.sort_by(f64::total_cmp);
    let min_dx = xs.windows(2).map(|w| (w[1] - w[0]).abs()).filter(|d| *d > 1e-3).fold(f64::INFINITY, f64::min);
    if min_dx.is_finite() { min_dx } else { 18.0 }
}

fn emit_discrete_pmf_points(
    primitives: &mut Vec<ScenePrimitive>,
    pts: &[CurveSamplePoint],
    spec: &SeriesSpec,
    color: Rgba8,
    is_shaded_highlight: bool,
    target: OverlayDrawTarget<'_>,
) {
    if pts.is_empty() {
        return;
    }
    let (_, raw_base_py, _) = target.y_axis.map_clamped_to_nearest(0.0);
    let base_py = raw_base_py + target.dy_pt;
    let min_dx_pt = min_mapped_x_spacing_pt(pts, target.x_axis);

    if let GraphicalDataObject::BarWithReplicates { fill_color, border_color, border_width_pt, .. } = spec.gdo {
        let bar_w =
            spec.bar_width_override_pt.filter(|v| v.is_finite() && *v > 0.0).unwrap_or_else(|| crate::clamp_f64_safe(min_dx_pt * 0.58, 2.5, 20.0));
        let eff_fill = if is_shaded_highlight { color } else { fill_color };
        let border = (border_width_pt.is_finite() && border_width_pt > 0.0).then(|| StrokePaint::solid(border_color, border_width_pt));
        for sp in pts {
            if is_shaded_highlight && sp.y <= 0.0 {
                continue;
            }
            if let (Some((_, px)), Some((_, py))) = (target.x_axis.map_value(sp.x), target.y_axis.map_value(sp.y)) {
                let x_pt = px + target.dx_pt;
                let y_pt = py + target.dy_pt;
                let y0 = base_py.min(y_pt);
                let y1 = base_py.max(y_pt);
                let rect = RectPt::new(x_pt - 0.5 * bar_w, y0, bar_w, (y1 - y0).max(0.5));
                primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(FillPaint::solid(eff_fill)),
                    stroke: border,
                    clip: Some(target.clip),
                });
            }
        }
        return;
    }

    // Publication lollipop / spike overlay at exact integer steps.
    let spike_w = if is_shaded_highlight { 1.9 } else { 1.5 };
    let stem_stroke = stroke_with_dash(color, spike_w, LineDashStyle::Solid);
    let backdrop_w = crate::clamp_f64_safe(min_dx_pt * 0.44, 2.0, 12.0);
    let marker_size = crate::clamp_f64_safe(min_dx_pt * 0.34, 2.8, if is_shaded_highlight { 5.4 } else { 4.8 });
    let marker = MarkerStyle {
        shape: MarkerShape::Circle,
        fill_mode: MarkerFillMode::Filled,
        size_pt: marker_size,
        fill_color: color,
        stroke_color: Rgba8::WHITE,
        stroke_width_pt: 0.5,
    };

    for sp in pts {
        if is_shaded_highlight && sp.y <= 0.0 {
            continue;
        }
        let (Some((_, px)), Some((_, py))) = (target.x_axis.map_value(sp.x), target.y_axis.map_value(sp.y)) else {
            continue;
        };
        let x_pt = px + target.dx_pt;
        let y_pt = py + target.dy_pt;
        let h = (base_py - y_pt).abs();
        if is_shaded_highlight && h >= 0.4 {
            let rect = RectPt::new(x_pt - 0.5 * backdrop_w, base_py.min(y_pt), backdrop_w, h);
            primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(rect),
                fill: Some(FillPaint::solid(color.with_alpha(84))),
                stroke: None,
                clip: Some(target.clip),
            });
        }
        if h >= 0.4 {
            primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(x_pt, base_py), PointPt::new(x_pt, y_pt)),
                fill: None,
                stroke: Some(stem_stroke),
                clip: Some(target.clip),
            });
        }
        emit_marker(primitives, PointPt::new(x_pt, y_pt), &marker, Some(target.clip));
    }
}

fn emit_discrete_cdf_step(
    primitives: &mut Vec<ScenePrimitive>,
    pts: &[CurveSamplePoint],
    color: Rgba8,
    is_shaded: bool,
    target: OverlayDrawTarget<'_>,
) {
    if pts.is_empty() {
        return;
    }
    let (_, raw_base_py, _) = target.y_axis.map_clamped_to_nearest(0.0);
    let base_py = raw_base_py + target.dy_pt;
    let mapped: Vec<PointPt> = pts
        .iter()
        .filter(|p| !is_shaded || p.y > 0.0)
        .filter_map(|p| {
            let (_, px) = target.x_axis.map_value(p.x)?;
            let (_, py) = target.y_axis.map_value(p.y)?;
            Some(PointPt::new(px + target.dx_pt, py + target.dy_pt))
        })
        .collect();
    if mapped.is_empty() {
        return;
    }
    if is_shaded {
        if mapped.len() >= 2
            && let Some(step_path) = build_connecting_path(&mapped, LineConnectionKind::Staircase)
        {
            let mut cmds = Vec::with_capacity(step_path.commands.len() + 3);
            cmds.push(PathCmd::MoveTo(PointPt::new(mapped[0].x, base_py)));
            for cmd in step_path.commands {
                match cmd {
                    PathCmd::MoveTo(pt) => cmds.push(PathCmd::LineTo(pt)),
                    other => cmds.push(other),
                }
            }
            if let Some(last) = mapped.last() {
                cmds.push(PathCmd::LineTo(PointPt::new(last.x, base_py)));
            }
            cmds.push(PathCmd::Close);
            primitives.push(ScenePrimitive::Path {
                path: Path2D { commands: cmds },
                fill: Some(FillPaint::solid(color.with_alpha(84))),
                stroke: None,
                clip: Some(target.clip),
            });
        }
        return;
    }

    if mapped.len() >= 2
        && let Some(path) = build_connecting_path(&mapped, LineConnectionKind::Staircase)
    {
        primitives.push(ScenePrimitive::Path {
            path,
            fill: None,
            stroke: Some(stroke_with_dash(color, 1.7, LineDashStyle::Solid)),
            clip: Some(target.clip),
        });
    }
    let marker = MarkerStyle {
        shape: MarkerShape::Circle,
        fill_mode: MarkerFillMode::Filled,
        size_pt: 4.2,
        fill_color: color,
        stroke_color: Rgba8::WHITE,
        stroke_width_pt: 0.45,
    };
    for &pt in &mapped {
        emit_marker(primitives, pt, &marker, Some(target.clip));
    }
}

/// Render a §6.2 theoretical distribution overlay (continuous curve + shaded tail polygons,
/// or discrete integer-step PMF lollipops / spikes / bars / CDF steps).
pub(super) fn render_distribution_overlay(
    primitives: &mut Vec<ScenePrimitive>,
    _warnings: &mut Vec<ChartWarning>,
    points: &[CurveSamplePoint],
    meta: &DistributionOverlayMeta,
    spec: &SeriesSpec,
    palette: PaletteId,
    target: OverlayDrawTarget<'_>,
) {
    let src = sorted_finite_points(points, target.dx_data);
    let main_src = if meta.main_curve_points.is_empty() { src.clone() } else { sorted_finite_points(&meta.main_curve_points, target.dx_data) };

    if meta.is_discrete {
        if meta.cumulative {
            if meta.is_shade_series {
                emit_discrete_cdf_step(primitives, &src, target.color, true, target);
            } else {
                let accent = shade_accent_color(palette, target.color, None);
                for sh in &meta.auto_shades {
                    let sh_src = sorted_finite_points(&sh.points, target.dx_data);
                    emit_discrete_cdf_step(primitives, &sh_src, accent, true, target);
                }
                emit_discrete_cdf_step(primitives, &src, target.color, false, target);
            }
        } else if meta.is_shade_series {
            emit_discrete_pmf_points(primitives, &src, spec, target.color, true, target);
        } else {
            emit_discrete_pmf_points(primitives, &src, spec, target.color, false, target);
            let accent = shade_accent_color(palette, target.color, None);
            for sh in &meta.auto_shades {
                let sh_src = sorted_finite_points(&sh.points, target.dx_data);
                emit_discrete_pmf_points(primitives, &sh_src, spec, accent, true, target);
            }
        }
        return;
    }

    // Continuous distribution (Normal, StudentT, FisherF, ChiSquare).
    if meta.is_shade_series {
        let explicit_fill = match &spec.gdo {
            GraphicalDataObject::ScatterAndLine { area_fill: Some(af), .. } => Some(af.fill_color),
            _ => Some(target.color),
        };
        let shade_col = shade_accent_color(palette, target.color, explicit_fill);
        let (runs, cutoffs) = match cutoff_bounds_for_slot(meta.tail_shade, meta.shade_slot) {
            Some((lo, hi)) => {
                let lo_s = lo + target.dx_data;
                let hi_s = hi + target.dx_data;
                let clipped = extract_clipped_continuous_region(&main_src, lo_s, hi_s);
                let cuts = [lo_s, hi_s].into_iter().filter(|v| v.is_finite()).collect::<Vec<_>>();
                (if clipped.len() >= 2 { vec![clipped] } else { extract_positive_y_runs(&src) }, cuts)
            }
            None => (extract_positive_y_runs(&src), Vec::new()),
        };
        emit_continuous_tail_polygon(primitives, &runs, &cutoffs, &main_src, shade_col, target);
    } else {
        let accent = shade_accent_color(palette, target.color, None);
        for sh in &meta.auto_shades {
            let sh_src = sorted_finite_points(&sh.points, target.dx_data);
            let (runs, cutoffs) = match cutoff_bounds_for_slot(meta.tail_shade, Some(sh.shade_slot)) {
                Some((lo, hi)) => {
                    let lo_s = lo + target.dx_data;
                    let hi_s = hi + target.dx_data;
                    let clipped = extract_clipped_continuous_region(&main_src, lo_s, hi_s);
                    let cuts = [lo_s, hi_s].into_iter().filter(|v| v.is_finite()).collect::<Vec<_>>();
                    (if clipped.len() >= 2 { vec![clipped] } else { extract_positive_y_runs(&sh_src) }, cuts)
                }
                None => (extract_positive_y_runs(&sh_src), Vec::new()),
            };
            emit_continuous_tail_polygon(primitives, &runs, &cutoffs, &main_src, accent, target);
        }
        emit_continuous_curve_stroke(primitives, &src, target.color, target);
    }
}
