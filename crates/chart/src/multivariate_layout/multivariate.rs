//! Multivariate 5-channel bubble scatter and categorical summary figure builders.

use curvebook_doc::{
    ColorChannelMapping, ErrorBarDirection, ErrorBarMetric, GraphSheet, GraphicalDataObject, LevelOrderSpec, MultivariateGraphEncoding,
    MultivariatePlotMode, ProjectDocument, ReplicateOverlayStyle, SizeChannelMapping, SummaryMeasure, TableArchetype, VariableColumn, VariableRole,
};
use curvebook_math::AxisScale;
use curvebook_math::dist::chi2_isf;
use curvebook_math::sum::covariance;
use curvebook_palette::{ColorRamp, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use curvebook_text::parse_markup;

use crate::column_grouped::{
    BoxWhiskerRenderCfg, FloatingBarRenderCfg, RawObs, ReplicateDotCtx, ViolinRenderCfg, emit_replicate_dots_oriented, render_box_whisker_obs,
    render_floating_bar_bounds, render_violin_obs,
};
use crate::markers::{emit_marker, rotated_ellipse_path};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive, SourceCellId, TextAnchor,
};
use crate::survival_cat::{
    LegendEntry, PlotMarginConfig, SegmentedFrameSpec, alpha_fill, draw_colorbar, draw_empty_open_frame, draw_legend_entries,
    layout_and_render_segmented_chrome, path_line, path_move, polygon_path, sanitize_range, solid_stroke,
};
use crate::xy::{
    AxisErrorTarget, SegmentedAxis, compute_central_tendency, compute_raw_error_bounds, emit_directional_error_bar, format_point_value, is_log_like,
};

const GROUP_SHAPES: [MarkerShape; 9] = [
    MarkerShape::Circle,
    MarkerShape::Square,
    MarkerShape::TriangleUp,
    MarkerShape::TriangleDown,
    MarkerShape::Diamond,
    MarkerShape::Hexagon,
    MarkerShape::Star,
    MarkerShape::Plus,
    MarkerShape::Cross,
];

// ============================================================================
// 1. MultipleVariables (5-Channel BubbleScatter & CategoricalSummary)
// ============================================================================

struct BubblePoint {
    row: usize,
    x: f64,
    y: f64,
    color_cat_idx: usize,
    color_scalar: Option<f64>,
    size_scalar: f64,
    label: Option<String>,
}

pub(super) fn compile_multiple_variables(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some(ds) = doc.data_sheet(&graph.spec.primary_source_sheet_id) else {
        draw_empty_open_frame(scene);
        return Ok(());
    };
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        scene.warnings.push(ChartWarning::new("incompatible_table", "MultipleVariables graph expects a MultipleVariables table"));
        draw_empty_open_frame(scene);
        return Ok(());
    };
    let encoding = default_mv_encoding(graph, mv);
    match &encoding.plot_mode {
        MultivariatePlotMode::BubbleScatter => compile_bubble_scatter(graph, mv, &encoding, doc, scene),
        MultivariatePlotMode::CategoricalSummary { gdo, y_error_bars } => {
            compile_categorical_summary(graph, mv, &encoding, gdo, *y_error_bars, doc, scene)
        }
    }
}

fn default_mv_encoding(graph: &GraphSheet, mv: &curvebook_doc::MultipleVariablesTable) -> MultivariateGraphEncoding {
    if let Some(enc) = &graph.spec.multivariate {
        return enc.clone();
    }
    MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: usize::from(mv.variables.len() > 1),
        color: ColorChannelMapping::Constant(graph.spec.palette.color_at(0)),
        size: SizeChannelMapping::Constant { radius_pt: 4.5 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    }
}

pub(super) fn column_categorical_levels(col: &VariableColumn, override_order: Option<&LevelOrderSpec>) -> Vec<String> {
    let spec = override_order.unwrap_or(&col.level_order);
    col.ordered_levels_with(spec).into_iter().map(|s| s.level).collect()
}

pub(super) fn cell_token(col: &VariableColumn, row: usize) -> Option<String> {
    let cell = col.cells.get(row)?;
    if cell.excluded {
        return None;
    }
    match &cell.value {
        curvebook_doc::CellValue::Empty => None,
        curvebook_doc::CellValue::Text(s) => {
            let tr = s.trim();
            (!tr.is_empty()).then(|| tr.to_owned())
        }
        curvebook_doc::CellValue::Number(v) if v.is_finite() => Some(format!("{v}")),
        curvebook_doc::CellValue::Number(_) => None,
    }
}

fn compile_bubble_scatter(
    graph: &GraphSheet,
    mv: &curvebook_doc::MultipleVariablesTable,
    enc: &MultivariateGraphEncoding,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let (Some(x_col), Some(y_col)) = (mv.variables.get(enc.x_column), mv.variables.get(enc.y_column)) else {
        scene.warnings.push(ChartWarning::new("missing_mv_column", "bubble scatter X or Y column index out of bounds"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let x_is_cat = x_col.role == VariableRole::Categorical;
    let x_levels = if x_is_cat { column_categorical_levels(x_col, enc.x_level_order_override.as_ref()) } else { Vec::new() };

    let (cat_levels, cat_palette, continuous_ramp, domain_clamp) = resolve_color_channel(mv, enc, graph.spec.palette);
    let mut points = Vec::new();
    for row in 0..mv.row_count() {
        let Some(yv) = y_col.cells.get(row).and_then(|c| c.active_number()) else { continue };
        let xv = if x_is_cat {
            let Some(tok) = cell_token(x_col, row) else { continue };
            x_levels.iter().position(|l| l == &tok).map(|i| i as f64)
        } else {
            x_col.cells.get(row).and_then(|c| c.active_number())
        };
        let Some(xv) = xv else { continue };

        let (cat_idx, col_scalar) = extract_row_color(mv, &enc.color, &cat_levels, row);
        let size_scalar = extract_row_size(mv, enc.size, row);
        let label = enc.label_column.and_then(|lc| mv.variables.get(lc)).and_then(|c| cell_token(c, row));
        points.push(BubblePoint { row, x: xv, y: yv, color_cat_idx: cat_idx, color_scalar: col_scalar, size_scalar, label });
    }

    if points.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_scatter_data", "no valid complete observations for bubble scatter"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let (x_min, x_max) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.x), hi.max(p.x)));
    let (y_min, y_max) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
    let x_pad = if x_is_cat { 0.5 } else { (x_max - x_min).max(1.0) * 0.08 };
    let y_pad = (y_max - y_min).max(1.0) * 0.08;
    let has_colorbar = continuous_ramp.is_some();
    let x_cats_opt = x_is_cat.then(|| x_levels.iter().enumerate().map(|(i, lbl)| (i as f64, lbl.clone())).collect::<Vec<_>>());
    let info_id = doc.paired_info_sheet_id(&graph.spec.primary_source_sheet_id);
    let legend_box_width_pt = crate::scales::auto_legend_box_width_pt(&graph.spec.legend, cat_levels.iter().map(String::as_str), doc, info_id);
    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (x_min - x_pad, x_max + x_pad),
            y_bounds: (y_min - y_pad, y_max + y_pad),
            y2_bounds: None,
            cat_x: x_cats_opt.as_deref(),
            cat_y: None,
            margins: PlotMarginConfig { has_legend: !cat_levels.is_empty(), legend_box_width_pt, has_colorbar, ..PlotMarginConfig::default() },
        },
    );

    let r = scene.plot_rect_pt;
    let (col_lo, col_hi) = resolve_continuous_color_domain(&points, domain_clamp);
    let max_abs_s = points.iter().map(|p| p.size_scalar.abs()).fold(0.0_f64, f64::max);

    let mut grouped_screen: Vec<Vec<(f64, f64)>> = vec![Vec::new(); cat_levels.len().max(1)];
    let mut mapped_points = Vec::with_capacity(points.len());
    for pt in &points {
        let (Some((_, px)), Some((_, py))) = (x_axis.map_value(pt.x), y_axis.map_value(pt.y)) else {
            continue;
        };
        if let Some(slot) = grouped_screen.get_mut(pt.color_cat_idx) {
            slot.push((px, py));
        }
        mapped_points.push((px, py, pt));
    }

    draw_hulls_and_ellipses(scene, &grouped_screen, enc, cat_palette, r);

    for &(px, py, pt) in &mapped_points {
        let fill = match (&continuous_ramp, pt.color_scalar) {
            (Some(ramp), Some(v)) => {
                let denom = (col_hi - col_lo).abs().max(1e-12);
                ramp.sample(crate::clamp_f64_safe((v - col_lo) / denom, 0.0, 1.0))
            }
            _ => match enc.color {
                ColorChannelMapping::Constant(c) => c,
                _ => cat_palette.color_at(pt.color_cat_idx),
            },
        };
        let radius = bubble_radius_pt(enc.size, pt.size_scalar, max_abs_s);
        let shape = if cat_levels.is_empty() { MarkerShape::Circle } else { GROUP_SHAPES[pt.color_cat_idx % GROUP_SHAPES.len()] };
        let base_marker = MarkerStyle {
            shape,
            fill_mode: MarkerFillMode::Filled,
            size_pt: radius * 2.0,
            fill_color: Rgba8::rgba(fill.r, fill.g, fill.b, 210),
            stroke_color: Rgba8::rgba(25, 25, 25, 210),
            stroke_width_pt: 0.75,
        };
        let Some(marker) =
            crate::resolve_overridden_marker_for_col(&graph.spec, &graph.spec.primary_source_sheet_id, enc.y_column, pt.row, Some(0), base_marker)
        else {
            continue;
        };
        let center_pt = PointPt::new(px, py);
        emit_marker(&mut scene.primitives, center_pt, &marker, Some(r));
        scene.plotted_points.push(PlottedPointMeta {
            center_pt,
            radius_pt: (marker.size_pt * 0.5).max(3.0),
            source_cell: SourceCellId { sheet_id: graph.spec.primary_source_sheet_id.clone(), row: pt.row, col: enc.y_column, subcol: 0 },
            series_label: cat_levels.get(pt.color_cat_idx).cloned().unwrap_or_else(|| y_col.title.clone()),
            value_summary: format!("X={}, Y={}", format_point_value(pt.x), format_point_value(pt.y)),
        });
        if let Some(lbl) = &pt.label {
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: parse_markup(lbl),
                anchor_pt: PointPt::new((px + radius + 2.5).min(r.right() - 4.0), py - radius * 0.2),
                anchor: TextAnchor::Start,
                font_size_pt: 7.5,
                rotation_deg: 0.0,
                color: Rgba8::rgb(30, 30, 30),
            }));
        }
    }

    if let Some(ramp) = continuous_ramp {
        let title = match &enc.color {
            ColorChannelMapping::Continuous { column, .. } => mv.variables.get(*column).map_or("", |v| v.title.as_str()),
            _ => "",
        };
        draw_colorbar(scene, &ramp, col_lo, col_hi, title, RectPt::new(r.right() + 10.0, r.y + 4.0, 11.0, (r.height - 8.0).max(24.0)));
    } else if !cat_levels.is_empty() {
        let entries: Vec<LegendEntry> = cat_levels
            .iter()
            .enumerate()
            .map(|(idx, lbl)| {
                let col = cat_palette.color_at(idx);
                let shape = GROUP_SHAPES[idx % GROUP_SHAPES.len()];
                LegendEntry {
                    label: lbl.clone(),
                    color: col,
                    marker: Some(MarkerStyle {
                        shape,
                        fill_color: col,
                        stroke_color: Rgba8::BLACK,
                        stroke_width_pt: 0.6,
                        size_pt: 5.5,
                        fill_mode: MarkerFillMode::Filled,
                    }),
                    dash: None,
                    is_line: false,
                }
            })
            .collect();
        draw_legend_entries(scene, &graph.spec, &entries, doc);
    }

    Ok(())
}

type ColorChannelResolved = (Vec<String>, PaletteId, Option<ColorRamp>, Option<(f64, f64)>);

fn resolve_color_channel(
    mv: &curvebook_doc::MultipleVariablesTable,
    enc: &MultivariateGraphEncoding,
    default_pal: PaletteId,
) -> ColorChannelResolved {
    match &enc.color {
        ColorChannelMapping::Constant(_) => (Vec::new(), default_pal, None, None),
        ColorChannelMapping::Categorical { column, palette, level_order_override } => {
            let levels = mv.variables.get(*column).map_or_else(Vec::new, |c| column_categorical_levels(c, level_order_override.as_ref()));
            (levels, *palette, None, None)
        }
        ColorChannelMapping::Continuous { ramp, domain_clamp, .. } => (Vec::new(), default_pal, Some(ramp.clone()), *domain_clamp),
    }
}

fn extract_row_color(
    mv: &curvebook_doc::MultipleVariablesTable,
    color: &ColorChannelMapping,
    cat_levels: &[String],
    row: usize,
) -> (usize, Option<f64>) {
    match color {
        ColorChannelMapping::Constant(_) => (0, None),
        ColorChannelMapping::Categorical { column, .. } => {
            let idx =
                mv.variables.get(*column).and_then(|c| cell_token(c, row)).and_then(|tok| cat_levels.iter().position(|l| l == &tok)).unwrap_or(0);
            (idx, None)
        }
        ColorChannelMapping::Continuous { column, .. } => {
            let val = mv.variables.get(*column).and_then(|c| c.cells.get(row)).and_then(|cell| cell.active_number());
            (0, val)
        }
    }
}

fn extract_row_size(mv: &curvebook_doc::MultipleVariablesTable, size: SizeChannelMapping, row: usize) -> f64 {
    match size {
        SizeChannelMapping::Constant { .. } => 1.0,
        SizeChannelMapping::Variable { column, .. } => {
            mv.variables.get(column).and_then(|c| c.cells.get(row)).and_then(|cell| cell.active_number()).unwrap_or(1.0)
        }
    }
}

fn resolve_continuous_color_domain(points: &[BubblePoint], clamp: Option<(f64, f64)>) -> (f64, f64) {
    if let Some((lo, hi)) = clamp {
        return sanitize_range(lo, hi, (0.0, 1.0));
    }
    let (lo, hi) = points.iter().filter_map(|p| p.color_scalar).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    sanitize_range(lo, hi, (0.0, 1.0))
}

fn bubble_radius_pt(size: SizeChannelMapping, scalar: f64, max_abs_s: f64) -> f64 {
    match size {
        SizeChannelMapping::Constant { radius_pt } => radius_pt.max(1.5),
        SizeChannelMapping::Variable { min_radius_pt, max_radius_pt, .. } => {
            let lo = min_radius_pt.min(max_radius_pt).max(1.2);
            let hi = max_radius_pt.max(min_radius_pt).max(lo);
            if max_abs_s <= 1e-12 { 0.5 * (lo + hi) } else { crate::clamp_f64_safe(hi * (scalar.abs() / max_abs_s).sqrt(), lo, hi) }
        }
    }
}

fn draw_hulls_and_ellipses(scene: &mut ChartScene, grouped: &[Vec<(f64, f64)>], enc: &MultivariateGraphEncoding, palette: PaletteId, clip: RectPt) {
    if !enc.show_convex_hulls && !enc.show_confidence_ellipses {
        return;
    }
    for (idx, pts) in grouped.iter().enumerate() {
        let color = match enc.color {
            ColorChannelMapping::Constant(c) => c,
            _ => palette.color_at(idx),
        };
        if enc.show_convex_hulls && pts.len() >= 3 {
            let hull = convex_hull(pts);
            if hull.len() >= 3 {
                scene.primitives.push(ScenePrimitive::Path {
                    path: polygon_path(&hull),
                    fill: Some(alpha_fill(color, 34)),
                    stroke: Some(solid_stroke(color, 1.0, LineDashStyle::Solid)),
                    clip: Some(clip),
                });
            }
        }
        if enc.show_confidence_ellipses && pts.len() >= 3 {
            emit_confidence_ellipse(scene, pts, color, clip);
        }
    }
}

fn convex_hull(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut sorted: Vec<(f64, f64)> = points.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    sorted.dedup_by(|a, b| (a.0 - b.0).abs() <= 1e-9 && (a.1 - b.1).abs() <= 1e-9);
    if sorted.len() < 3 {
        return sorted;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &p in &sorted {
        while lower.len() >= 2 {
            let n = lower.len();
            if cross(lower[n - 2], lower[n - 1], p) <= 0.0 {
                let _ = lower.pop();
            } else {
                break;
            }
        }
        lower.push(p);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &p in sorted.iter().rev() {
        while upper.len() >= 2 {
            let n = upper.len();
            if cross(upper[n - 2], upper[n - 1], p) <= 0.0 {
                let _ = upper.pop();
            } else {
                break;
            }
        }
        upper.push(p);
    }
    let _ = lower.pop();
    let _ = upper.pop();
    lower.extend(upper);
    lower
}

fn emit_confidence_ellipse(scene: &mut ChartScene, pts: &[(f64, f64)], color: Rgba8, clip: RectPt) {
    let n = pts.len();
    if n < 3 {
        return;
    }
    let xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
    let nf = n as f64;
    let mx = xs.iter().sum::<f64>() / nf;
    let my = ys.iter().sum::<f64>() / nf;
    let cxx = xs.iter().map(|x| (x - mx) * (x - mx)).sum::<f64>() / (nf - 1.0);
    let cyy = ys.iter().map(|y| (y - my) * (y - my)).sum::<f64>() / (nf - 1.0);
    let cxy = covariance(&xs, &ys).unwrap_or(0.0);

    let trace = cxx + cyy;
    let diff = cxx - cyy;
    let disc = (diff * diff + 4.0 * cxy * cxy).sqrt();
    let lam1 = (0.5 * (trace + disc)).max(1.0);
    let lam2 = (0.5 * (trace - disc)).max(1.0);
    let angle = 0.5 * (2.0 * cxy).atan2(diff);
    let chi2_crit = chi2_isf(0.05, 2.0).unwrap_or(5.991_464_547);
    let k = chi2_crit.sqrt();

    let rx = k * lam1.sqrt();
    let ry = k * lam2.sqrt();
    let path = rotated_ellipse_path(mx, my, rx, ry, angle);
    scene.primitives.push(ScenePrimitive::Path {
        path,
        fill: Some(alpha_fill(color, 26)),
        stroke: Some(solid_stroke(color, 1.1, LineDashStyle::Dashed)),
        clip: Some(clip),
    });
}

// Categorical × Continuous Summary (Gap G7)

struct CatSlotComputed {
    sorted_obs: Vec<RawObs>,
    center_opt: Option<f64>,
    err_bounds: Option<(f64, f64)>,
}

struct CatSlotDraw<'a> {
    graph: &'a GraphSheet,
    override_series_index: Option<usize>,
    level_label: &'a str,
    gdo: &'a GraphicalDataObject,
    y_error_bars: Option<curvebook_doc::ErrorBarSpec>,
    color: Rgba8,
    cx: f64,
    box_w: f64,
    y_zero: f64,
    y_axis: &'a SegmentedAxis,
}

fn resolve_categorical_cld_overlay(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    levels: &[String],
    slots: &[CatSlotComputed],
    warnings: &mut Vec<ChartWarning>,
) -> Option<crate::column_grouped::ResolvedCldLabels> {
    let level_means: Vec<Option<f64>> = slots
        .iter()
        .map(|s| {
            s.center_opt
                .or_else(|| (!s.sorted_obs.is_empty()).then(|| s.sorted_obs.iter().map(|o| o.value).sum::<f64>() / (s.sorted_obs.len() as f64)))
        })
        .collect();
    let cld_slots: Vec<(usize, &str, &str, Option<f64>, f64)> =
        levels.iter().enumerate().map(|(i, lvl)| (i, lvl.as_str(), lvl.as_str(), level_means[i], i as f64)).collect();
    crate::column_grouped::resolve_cld_overlay(graph, doc, &cld_slots, warnings)
}

fn gdo_summary_measure(gdo: &GraphicalDataObject) -> Option<SummaryMeasure> {
    match gdo {
        GraphicalDataObject::BarWithReplicates { summary, .. } | GraphicalDataObject::AlignedDotPlot { summary, .. } => Some(*summary),
        GraphicalDataObject::FloatingBar { summary, .. } => *summary,
        GraphicalDataObject::ScatterAndLine { .. } | GraphicalDataObject::BeforeAfterLines { .. } => Some(SummaryMeasure::Mean),
        GraphicalDataObject::BoxAndWhisker { .. }
        | GraphicalDataObject::Violin { .. }
        | GraphicalDataObject::KaplanMeierStep { .. }
        | GraphicalDataObject::PartsOfWhole { .. } => None,
    }
}

fn emit_cat_slot_dots(
    scene: &mut ChartScene,
    obs: &[RawObs],
    draw: &CatSlotDraw<'_>,
    max_half_w: f64,
    style: ReplicateOverlayStyle,
    marker: &MarkerStyle,
) {
    let ctx = ReplicateDotCtx {
        spec: &draw.graph.spec,
        override_series_index: draw.override_series_index,
        cx: draw.cx,
        max_half_w,
        val_axis: draw.y_axis,
        style,
        marker,
        series_label: draw.level_label,
        is_horizontal: false,
    };
    let _ = emit_replicate_dots_oriented(scene, obs, &ctx);
}

fn draw_categorical_summary_slot(scene: &mut ChartScene, slot: &CatSlotComputed, draw: &CatSlotDraw<'_>) -> f64 {
    let r = scene.plot_rect_pt;
    let cx = draw.cx;
    let box_w = draw.box_w;
    let y_axis = draw.y_axis;
    let color = draw.color;
    let default_dot = MarkerStyle {
        shape: MarkerShape::Circle,
        fill_mode: MarkerFillMode::Filled,
        size_pt: 4.5,
        fill_color: color,
        stroke_color: Rgba8::WHITE,
        stroke_width_pt: 0.5,
    };
    let max_obs_val = slot.sorted_obs.last().map(|o| o.value);
    let mut top_data_val = slot.center_opt.or(max_obs_val).unwrap_or(0.0);

    match draw.gdo {
        GraphicalDataObject::BarWithReplicates { border_color, border_width_pt, replicate_overlay, .. } => {
            if let Some(center_y) = slot.center_opt
                && let Some((_, y_top)) = y_axis.map_value(center_y)
            {
                let rect = RectPt::new(cx - box_w * 0.5, y_top.min(draw.y_zero), box_w, (draw.y_zero - y_top).abs().max(0.5));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(alpha_fill(color, 185)),
                    stroke: Some(solid_stroke(*border_color, *border_width_pt, LineDashStyle::Solid)),
                    clip: Some(r),
                });
            }
            if let Some(overlay_style) = *replicate_overlay {
                let bar_dot = MarkerStyle { fill_color: Rgba8::rgb(35, 35, 35), ..default_dot };
                emit_cat_slot_dots(scene, &slot.sorted_obs, draw, box_w * 0.30, overlay_style, &bar_dot);
                if let Some(max_v) = max_obs_val {
                    top_data_val = top_data_val.max(max_v);
                }
            }
        }
        GraphicalDataObject::BoxAndWhisker { whiskers, stroke_color, show_outliers, show_all_points, .. } => {
            let _ = render_box_whisker_obs(
                scene,
                &slot.sorted_obs,
                &BoxWhiskerRenderCfg {
                    spec: &draw.graph.spec,
                    override_series_index: draw.override_series_index,
                    cx,
                    box_w,
                    whiskers: *whiskers,
                    fill_color: color.with_alpha(175),
                    stroke_color: *stroke_color,
                    show_outliers: *show_outliers,
                    show_all_points: *show_all_points,
                    val_axis: y_axis,
                    series_label: draw.level_label,
                    is_horizontal: false,
                },
            );
            if let Some(max_v) = max_obs_val {
                top_data_val = max_v;
            }
        }
        GraphicalDataObject::Violin { stroke_color, clip_to_observed_range, show_median_and_quartiles, half_violin_with_points, .. } => {
            let _ = render_violin_obs(
                scene,
                &slot.sorted_obs,
                &ViolinRenderCfg {
                    spec: &draw.graph.spec,
                    override_series_index: draw.override_series_index,
                    cx,
                    half_max_w: box_w * 0.45,
                    fill_color: color.with_alpha(175),
                    stroke_color: *stroke_color,
                    clip_to_observed_range: *clip_to_observed_range,
                    show_median_and_quartiles: *show_median_and_quartiles,
                    half_violin_with_points: *half_violin_with_points,
                    val_axis: y_axis,
                    series_label: draw.level_label,
                    is_horizontal: false,
                },
            );
            if let Some(max_v) = max_obs_val {
                top_data_val = max_v;
            }
        }
        GraphicalDataObject::FloatingBar { border_color, border_width_pt, summary, .. } => {
            if let (Some(lo), Some(hi)) = (slot.sorted_obs.first().map(|o| o.value), max_obs_val) {
                top_data_val = hi;
                let _ = render_floating_bar_bounds(
                    scene,
                    lo,
                    hi,
                    summary.and(slot.center_opt),
                    &FloatingBarRenderCfg {
                        cx,
                        slot_w_pt: box_w,
                        fill_color: color.with_alpha(180),
                        border_color: *border_color,
                        border_width_pt: *border_width_pt,
                        val_axis: y_axis,
                        is_horizontal: false,
                    },
                );
            }
        }
        GraphicalDataObject::AlignedDotPlot { show_crossbar, marker, .. } => {
            emit_cat_slot_dots(scene, &slot.sorted_obs, draw, box_w * 0.325, ReplicateOverlayStyle::DeterministicBeeswarm, marker);
            if let Some(max_v) = max_obs_val {
                top_data_val = top_data_val.max(max_v);
            }
            if *show_crossbar
                && let Some(center_y) = slot.center_opt
                && let Some((_, ym)) = y_axis.map_value(center_y)
            {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(cx - box_w * 0.45, ym), PointPt::new(cx + box_w * 0.45, ym)),
                    fill: None,
                    stroke: Some(solid_stroke(Rgba8::BLACK, 1.6, LineDashStyle::Solid)),
                    clip: Some(r),
                });
            }
        }
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker.or_else(|| line.is_none().then_some(default_dot)) {
                emit_cat_slot_dots(scene, &slot.sorted_obs, draw, box_w * 0.325, ReplicateOverlayStyle::DeterministicBeeswarm, &m);
            }
            if let Some(max_v) = max_obs_val {
                top_data_val = top_data_val.max(max_v);
            }
        }
        GraphicalDataObject::BeforeAfterLines { marker, .. } => {
            if let Some(m) = marker {
                emit_cat_slot_dots(scene, &slot.sorted_obs, draw, box_w * 0.325, ReplicateOverlayStyle::DeterministicBeeswarm, m);
            }
            if let Some(max_v) = max_obs_val {
                top_data_val = top_data_val.max(max_v);
            }
        }
        _ => {
            emit_cat_slot_dots(scene, &slot.sorted_obs, draw, box_w * 0.325, ReplicateOverlayStyle::DeterministicBeeswarm, &default_dot);
            if let Some(max_v) = max_obs_val {
                top_data_val = top_data_val.max(max_v);
            }
        }
    }

    if let (Some(eb), Some(center_y), Some((err_lo, err_hi))) = (draw.y_error_bars, slot.center_opt, slot.err_bounds) {
        emit_directional_error_bar(
            scene,
            cx,
            center_y,
            err_lo,
            err_hi,
            AxisErrorTarget { axis: draw.y_axis, is_vertical: true, spec: &eb, default_color: Rgba8::BLACK, clip: r },
        );
        if !matches!(eb.direction, ErrorBarDirection::LowerOnly) {
            top_data_val = top_data_val.max(err_hi);
        }
    }
    top_data_val
}

fn build_categorical_slots(
    groups: Vec<Vec<RawObs>>,
    summary_for_center: Option<SummaryMeasure>,
    y_error_bars: Option<curvebook_doc::ErrorBarSpec>,
    is_log_y: bool,
    warnings: &mut Vec<ChartWarning>,
) -> (Vec<CatSlotComputed>, f64, f64) {
    let mut data_lo = f64::INFINITY;
    let mut data_hi = f64::NEG_INFINITY;
    let mut slots = Vec::with_capacity(groups.len());
    for mut obs in groups {
        obs.sort_by(|a, b| a.value.total_cmp(&b.value).then_with(|| a.source.row.cmp(&b.source.row)));
        let raw_vals: Vec<f64> = obs.iter().map(|o| o.value).collect();
        for &v in &raw_vals {
            data_lo = data_lo.min(v);
            data_hi = data_hi.max(v);
        }
        let center_opt = summary_for_center.and_then(|s| compute_central_tendency(&raw_vals, s, warnings));
        let err_bounds = if let (Some(center), Some(eb)) = (center_opt, y_error_bars) {
            if eb.metric == ErrorBarMetric::ConfidenceInterval95 && raw_vals.len() < 2 {
                warnings.push(ChartWarning::new(
                    "insufficient_sample_size_for_ci",
                    format!("95% CI requires N >= 2 (got N = {}); error bar omitted", raw_vals.len()),
                ));
                None
            } else {
                let (raw_lo, raw_hi) = compute_raw_error_bounds(&raw_vals, center, eb.metric, 0.05);
                let eff_lo = if matches!(eb.direction, ErrorBarDirection::UpperOnly) { center } else { raw_lo.min(center) };
                let eff_hi = if matches!(eb.direction, ErrorBarDirection::LowerOnly) { center } else { raw_hi.max(center) };
                if !is_log_y || eff_lo > 0.0 {
                    data_lo = data_lo.min(eff_lo);
                }
                if !is_log_y || eff_hi > 0.0 {
                    data_hi = data_hi.max(eff_hi);
                }
                Some((eff_lo, eff_hi))
            }
        } else {
            None
        };
        slots.push(CatSlotComputed { sorted_obs: obs, center_opt, err_bounds });
    }
    (slots, data_lo, data_hi)
}

fn emit_categorical_connecting_line(scene: &mut ChartScene, gdo: &GraphicalDataObject, line_centers: &[(f64, f64)], clip_rect: RectPt) {
    let line_stroke_opt = match gdo {
        GraphicalDataObject::ScatterAndLine { line: Some(ls), .. } if ls.stroke_width_pt.is_finite() && ls.stroke_width_pt > 0.0 => {
            Some(solid_stroke(ls.color, ls.stroke_width_pt, ls.dash_style))
        }
        GraphicalDataObject::BeforeAfterLines { line_color, stroke_width_pt, .. } if stroke_width_pt.is_finite() && *stroke_width_pt > 0.0 => {
            Some(solid_stroke(*line_color, *stroke_width_pt, LineDashStyle::Solid))
        }
        _ => None,
    };
    if let Some(stroke) = line_stroke_opt
        && let Some(&(x0, y0)) = line_centers.first()
        && line_centers.len() >= 2
    {
        let mut p = Path2D::new();
        path_move(&mut p, PointPt::new(x0, y0));
        for &(x, y) in line_centers.iter().skip(1) {
            path_line(&mut p, PointPt::new(x, y));
        }
        scene.primitives.push(ScenePrimitive::Path { path: p, fill: None, stroke: Some(stroke), clip: Some(clip_rect) });
    }
}

fn compile_categorical_summary(
    graph: &GraphSheet,
    mv: &curvebook_doc::MultipleVariablesTable,
    enc: &MultivariateGraphEncoding,
    gdo: &GraphicalDataObject,
    y_error_bars: Option<curvebook_doc::ErrorBarSpec>,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let (Some(x_col), Some(y_col)) = (mv.variables.get(enc.x_column), mv.variables.get(enc.y_column)) else {
        scene.warnings.push(ChartWarning::new("missing_mv_column", "categorical summary column index missing"));
        draw_empty_open_frame(scene);
        return Ok(());
    };
    let levels = column_categorical_levels(x_col, enc.x_level_order_override.as_ref());
    if levels.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_categorical_levels", "no categorical levels found"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let sheet_id = &graph.spec.primary_source_sheet_id;
    let override_series_index = crate::override_series_index_for_col(&graph.spec, sheet_id, enc.y_column);
    let mut groups: Vec<Vec<RawObs>> = vec![Vec::new(); levels.len()];
    for row in 0..mv.row_count() {
        let Some(tok) = cell_token(x_col, row) else { continue };
        let Some(yv) = y_col.cells.get(row).and_then(|c| c.active_number()).filter(|v| v.is_finite()) else { continue };
        if let Some(pos) = levels.iter().position(|l| l == &tok)
            && let Some(bucket) = groups.get_mut(pos)
        {
            bucket.push(RawObs { value: yv, source: SourceCellId { sheet_id: sheet_id.clone(), row, col: enc.y_column, subcol: 0 } });
        }
    }

    let y_scale = graph.spec.y_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale);
    let is_log_y = is_log_like(y_scale);
    let summary_for_center = gdo_summary_measure(gdo).or_else(|| y_error_bars.map(|_| SummaryMeasure::Mean));
    let (slots, mut data_lo, mut data_hi) = build_categorical_slots(groups, summary_for_center, y_error_bars, is_log_y, &mut scene.warnings);
    if !data_lo.is_finite() {
        draw_empty_open_frame(scene);
        return Ok(());
    }
    if matches!(gdo, GraphicalDataObject::BarWithReplicates { .. }) && !is_log_y {
        data_lo = data_lo.min(0.0);
        data_hi = data_hi.max(0.0);
    }

    let resolved_cld = resolve_categorical_cld_overlay(graph, doc, &levels, &slots, &mut scene.warnings);
    let cld_top_margin = if resolved_cld.is_some() { crate::column_grouped::cld_reserved_top_margin_pt(&graph.spec) } else { 0.0 };
    let (dom_in_lo, dom_in_hi) = if is_log_y {
        (data_lo * 0.85, data_hi * if resolved_cld.is_some() { 1.35 } else { 1.15 })
    } else {
        let raw_span = (data_hi - data_lo).abs();
        let pad_base = if raw_span > 0.0 { raw_span } else { data_hi.abs().max(1.0) };
        let y_pad = pad_base * 0.12;
        let y_hi_pad = if resolved_cld.is_some() { pad_base * 0.18 } else { y_pad };
        (data_lo - if matches!(gdo, GraphicalDataObject::BarWithReplicates { .. }) && data_lo >= 0.0 { 0.0 } else { y_pad }, data_hi + y_hi_pad)
    };
    let x_cats: Vec<(f64, String)> = levels.iter().enumerate().map(|(i, l)| ((i as f64) + 0.5, l.clone())).collect();
    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (0.0, levels.len() as f64),
            y_bounds: (dom_in_lo, dom_in_hi),
            y2_bounds: None,
            cat_x: Some(&x_cats),
            cat_y: None,
            margins: PlotMarginConfig { extra_top_pt: cld_top_margin, ..PlotMarginConfig::default() },
        },
    );

    let r = scene.plot_rect_pt;
    let y_zero = y_axis.map_value(0.0).map_or(r.bottom(), |(_, py)| crate::clamp_f64_safe(py, r.y, r.bottom()));
    let mut line_centers = Vec::new();
    let mut slot_bounds = Vec::with_capacity(slots.len());
    for (idx, slot) in slots.iter().enumerate() {
        if slot.sorted_obs.is_empty() {
            continue;
        }
        let Some((_, cx)) = x_axis.map_value((idx as f64) + 0.5) else {
            continue;
        };
        let slot_lo = x_axis.map_value(idx as f64).map_or(r.x, |(_, pt)| pt);
        let slot_hi = x_axis.map_value((idx + 1) as f64).map_or(r.right(), |(_, pt)| pt);
        let slot_w = (slot_hi - slot_lo).abs().max(8.0);
        let color = graph.spec.palette.color_at(idx);
        let box_w = (slot_w * 0.56).min(38.0);
        let top_data_val = draw_categorical_summary_slot(
            scene,
            slot,
            &CatSlotDraw { graph, override_series_index, level_label: &levels[idx], gdo, y_error_bars, color, cx, box_w, y_zero, y_axis: &y_axis },
        );
        let top_y_pt = y_axis.map_value(top_data_val).map_or(r.y, |(_, py)| crate::clamp_f64_safe(py, r.y, r.bottom()));
        slot_bounds.push(crate::column_grouped::RenderedSlotBound {
            group_key: idx,
            label: levels[idx].clone(),
            alt_label: levels[idx].clone(),
            x_pt: cx,
            top_y_pt,
        });
        if let Some(center_y) = slot.center_opt
            && let Some((_, yc)) = y_axis.map_value(center_y)
        {
            line_centers.push((cx, yc));
        }
    }

    if let Some(cld) = &resolved_cld {
        crate::column_grouped::render_cld_labels(scene, graph, cld, &mut slot_bounds);
    }
    emit_categorical_connecting_line(scene, gdo, &line_centers, r);

    Ok(())
}
