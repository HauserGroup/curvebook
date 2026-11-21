//! Multivariate bubble/categorical figures, all seven diagnostic plot subtypes, and
//! multi-panel publication layout scene compilation.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, ClusterDistance, ClusterLinkage, ColorChannelMapping, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, GraphSpec,
    GraphicalDataObject, HeatmapColumnLabelAngle, HeatmapColumnLabelMode, HeatmapRowLabelMode, LayoutItem, LayoutSheet, LevelOrderSpec,
    MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype, VariableColumn, VariableRole, WhiskerMethod,
};
use curvebook_math::dist::{chi2_isf, norm_ppf};
use curvebook_math::sum::{covariance, sample_sd};
use curvebook_math::{AxisScale, ConfidenceLevel};
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use curvebook_stats::common::quartiles_hinge;
use curvebook_stats::multivariate::{DendrogramMerge, hierarchical_clustering_with_metrics};
use curvebook_stats::roc::{RocCiMethod, roc_curve_analysis};
use curvebook_stats::two_group::bland_altman;
use curvebook_text::{RichText, parse_markup};

use crate::scales::AxisMapping;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive,
    SourceCellId, TextAnchor,
};
use crate::survival_cat::{
    CartesianFrame, LegendEntry, PlotMarginConfig, alpha_fill, draw_cartesian_frame, draw_categorical_x_ticks, draw_colorbar, draw_empty_open_frame,
    draw_graph_annotations, draw_legend_entries, emit_marker, nice_axis_ticks, path_close, path_line, path_move, polygon_path, resolve_axis_domain,
    resolve_graph_text, sanitize_range, solid_stroke, text_metrics, update_plot_rect_for_labels,
};
use crate::xy::{EmbeddedTableDrawSpec, format_point_value, render_styled_embedded_table};

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

/// Compile a multivariate or diagnostic graph sheet into vector primitives inside `scene`.
pub fn compile_multivariate_or_diagnostic_graph(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    if doc.data_sheet(&graph.spec.primary_source_sheet_id).is_none() && doc.result_sheet(&graph.spec.primary_source_sheet_id).is_none() {
        scene
            .warnings
            .push(ChartWarning::new("missing_source_sheet", format!("primary data sheet '{}' not found", graph.spec.primary_source_sheet_id)));
    }

    match graph.spec.kind {
        GraphKind::MultipleVariables => compile_multiple_variables(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap) => compile_heatmap(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::Volcano) => compile_volcano(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman) => compile_bland_altman(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve) => compile_roc_curve(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot) => compile_forest_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot) => compile_residual_plot(graph, doc, scene),
        GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot) => compile_normal_qq_plot(graph, doc, scene),
        _ => {
            scene.warnings.push(ChartWarning::new("unexpected_graph_kind", "non-multivariate graph kind dispatched"));
            draw_empty_open_frame(scene);
            Ok(())
        }
    }?;

    draw_graph_annotations(scene, &graph.spec.annotations, doc, doc.paired_info_sheet_id(&graph.spec.primary_source_sheet_id));
    Ok(())
}

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

fn compile_multiple_variables(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
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

fn column_categorical_levels(col: &VariableColumn, override_order: Option<&LevelOrderSpec>) -> Vec<String> {
    let spec = override_order.unwrap_or(&col.level_order);
    col.ordered_levels_with(spec).into_iter().map(|s| s.level).collect()
}

fn cell_token(col: &VariableColumn, row: usize) -> Option<String> {
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
    let (x_lo, x_hi) = resolve_axis_domain(&graph.spec.x_axis, x_min - x_pad, x_max + x_pad);
    let (y_lo, y_hi) = resolve_axis_domain(&graph.spec.y_axis, y_min - y_pad, y_max + y_pad);

    let x_ticks = if x_is_cat { Vec::new() } else { nice_axis_ticks(&graph.spec.x_axis, x_lo, x_hi, 6) };
    let y_ticks = nice_axis_ticks(&graph.spec.y_axis, y_lo, y_hi, 5);
    let has_colorbar = continuous_ramp.is_some();
    let max_y_w = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &y_ticks,
        PlotMarginConfig { has_legend: !cat_levels.is_empty(), has_colorbar, ..PlotMarginConfig::default() },
    );

    let r = scene.plot_rect_pt;
    let x_map = AxisMapping { min: x_lo, max: x_hi, scale: graph.spec.x_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale) };
    let y_map = AxisMapping { min: y_lo, max: y_hi, scale: graph.spec.y_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale) };
    let frame = CartesianFrame { spec: &graph.spec, x_ticks: &x_ticks, y_ticks: &y_ticks, x_map, y_map, max_y_label_w: max_y_w, doc };
    draw_cartesian_frame(scene, &frame);
    if x_is_cat {
        let cats: Vec<(f64, String)> =
            x_levels.iter().enumerate().filter_map(|(i, lbl)| x_map.map_value(i as f64, r.x, r.right()).map(|xp| (xp, lbl.clone()))).collect();
        draw_categorical_x_ticks(scene, &graph.spec, &cats, doc);
    }

    let (col_lo, col_hi) = resolve_continuous_color_domain(&points, domain_clamp);
    let max_abs_s = points.iter().map(|p| p.size_scalar.abs()).fold(0.0_f64, f64::max);

    let mut grouped_screen: Vec<Vec<(f64, f64)>> = vec![Vec::new(); cat_levels.len().max(1)];
    let mut mapped_points = Vec::with_capacity(points.len());
    for pt in &points {
        let (Some(px), Some(py)) = (x_map.map_value(pt.x, r.x, r.right()), y_map.map_value(pt.y, r.bottom(), r.y)) else {
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
        let Some(marker) = crate::resolve_overridden_marker_for_cell(&graph.spec, enc.y_column, pt.row, Some(0), base_marker) else {
            continue;
        };
        let center_pt = PointPt::new(px, py);
        emit_marker(scene, center_pt, marker, Some(r));
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
    let path = crate::survival_cat::ellipse_path(mx, my, rx, ry, angle);
    scene.primitives.push(ScenePrimitive::Path {
        path,
        fill: Some(alpha_fill(color, 26)),
        stroke: Some(solid_stroke(color, 1.1, LineDashStyle::Dashed)),
        clip: Some(clip),
    });
}

// Categorical × Continuous Summary (Gap G7)

fn resolve_categorical_cld_overlay(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    levels: &[String],
    groups: &[Vec<f64>],
    warnings: &mut Vec<ChartWarning>,
) -> Option<crate::column_grouped::ResolvedCldLabels> {
    let level_means: Vec<Option<f64>> =
        groups.iter().map(|vals| (!vals.is_empty()).then(|| vals.iter().sum::<f64>() / (vals.len() as f64))).collect();
    let cld_slots: Vec<(usize, &str, &str, Option<f64>, f64)> =
        levels.iter().enumerate().map(|(i, lvl)| (i, lvl.as_str(), lvl.as_str(), level_means[i], i as f64)).collect();
    crate::column_grouped::resolve_cld_overlay(graph, doc, &cld_slots, warnings)
}

#[allow(clippy::too_many_arguments)]
fn draw_categorical_summary_slot(
    scene: &mut ChartScene,
    gdo: &GraphicalDataObject,
    y_error_bars: Option<curvebook_doc::ErrorBarSpec>,
    color: Rgba8,
    r: RectPt,
    cx: f64,
    box_w: f64,
    y_zero: f64,
    values: &[f64],
    sorted: &[f64],
    center_y: f64,
    y_map: AxisMapping,
) -> f64 {
    let mut top_data_val = center_y;
    match gdo {
        GraphicalDataObject::BarWithReplicates { border_color, border_width_pt, replicate_overlay, .. } => {
            if let Some(y_top) = y_map.map_value(center_y, r.bottom(), r.y) {
                let rect = RectPt::new(cx - box_w * 0.5, y_top.min(y_zero), box_w, (y_zero - y_top).abs().max(0.5));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(alpha_fill(color, 185)),
                    stroke: Some(solid_stroke(*border_color, *border_width_pt, LineDashStyle::Solid)),
                    clip: Some(r),
                });
            }
            if replicate_overlay.is_some() {
                emit_replicate_dots(scene, cx, box_w * 0.6, sorted, y_map, r, Rgba8::rgb(35, 35, 35));
                if let Some(&max_v) = sorted.last() {
                    top_data_val = top_data_val.max(max_v);
                }
            }
        }
        GraphicalDataObject::BoxAndWhisker { whiskers, stroke_color, show_outliers, show_all_points, .. } => {
            draw_box_and_whisker_slot(
                scene,
                sorted,
                &BoxSlotSpec {
                    cx,
                    box_w,
                    whiskers: *whiskers,
                    fill_color: color,
                    stroke_color: *stroke_color,
                    show_outliers: *show_outliers,
                    show_all: *show_all_points,
                    y_map,
                },
            );
            if let Some(&max_v) = sorted.last() {
                top_data_val = max_v;
            }
        }
        GraphicalDataObject::Violin { stroke_color, show_median_and_quartiles, half_violin_with_points, .. } => {
            draw_violin_slot(
                scene,
                sorted,
                &ViolinSlotSpec {
                    cx,
                    half_w: box_w * 0.9,
                    fill_color: color,
                    stroke_color: *stroke_color,
                    show_med: *show_median_and_quartiles,
                    half_violin: *half_violin_with_points,
                    y_map,
                },
            );
            if let Some(&max_v) = sorted.last() {
                top_data_val = max_v;
            }
        }
        GraphicalDataObject::FloatingBar { border_color, border_width_pt, summary, .. } => {
            if let (Some(&lo), Some(&hi)) = (sorted.first(), sorted.last())
                && let (Some(y_lo_pt), Some(y_hi_pt)) = (y_map.map_value(lo, r.bottom(), r.y), y_map.map_value(hi, r.bottom(), r.y))
            {
                top_data_val = hi;
                let rect = RectPt::new(cx - box_w * 0.5, y_hi_pt.min(y_lo_pt), box_w, (y_lo_pt - y_hi_pt).abs().max(1.0));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(alpha_fill(color, 180)),
                    stroke: Some(solid_stroke(*border_color, *border_width_pt, LineDashStyle::Solid)),
                    clip: Some(r),
                });
                if let Some(m) = summary
                    && let Some(ym) = y_map.map_value(summary_center(values, sorted, *m), r.bottom(), r.y)
                {
                    scene.primitives.push(ScenePrimitive::Path {
                        path: Path2D::line(PointPt::new(cx - box_w * 0.5, ym), PointPt::new(cx + box_w * 0.5, ym)),
                        fill: None,
                        stroke: Some(solid_stroke(*border_color, 1.5, LineDashStyle::Solid)),
                        clip: Some(r),
                    });
                }
            }
        }
        GraphicalDataObject::AlignedDotPlot { show_crossbar, .. } => {
            emit_replicate_dots(scene, cx, box_w * 0.65, sorted, y_map, r, color);
            if let Some(&max_v) = sorted.last() {
                top_data_val = top_data_val.max(max_v);
            }
            if *show_crossbar && let Some(ym) = y_map.map_value(center_y, r.bottom(), r.y) {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(cx - box_w * 0.45, ym), PointPt::new(cx + box_w * 0.45, ym)),
                    fill: None,
                    stroke: Some(solid_stroke(Rgba8::BLACK, 1.6, LineDashStyle::Solid)),
                    clip: Some(r),
                });
            }
        }
        _ => {
            emit_replicate_dots(scene, cx, box_w * 0.65, sorted, y_map, r, color);
            if let Some(&max_v) = sorted.last() {
                top_data_val = top_data_val.max(max_v);
            }
        }
    }

    if let Some(eb) = y_error_bars {
        emit_error_bar(scene, cx, values, sorted, center_y, eb, y_map);
        let sd = sample_sd(values).unwrap_or(0.0);
        let err_top = match eb.metric {
            curvebook_doc::ErrorBarMetric::Sd | curvebook_doc::ErrorBarMetric::ExplicitSubcolumns => center_y + sd,
            curvebook_doc::ErrorBarMetric::Sem => center_y + sd / (values.len().max(1) as f64).sqrt(),
            curvebook_doc::ErrorBarMetric::ConfidenceInterval95 => center_y + 1.96 * sd / (values.len().max(1) as f64).sqrt(),
            curvebook_doc::ErrorBarMetric::InterquartileRange => quartiles_hinge(sorted).map_or(center_y, |(_, _, q3)| q3),
            curvebook_doc::ErrorBarMetric::Range => sorted.last().copied().unwrap_or(center_y),
        };
        if !matches!(eb.direction, curvebook_doc::ErrorBarDirection::LowerOnly) {
            top_data_val = top_data_val.max(err_top);
        }
    }
    top_data_val
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

    let mut groups: Vec<Vec<f64>> = vec![Vec::new(); levels.len()];
    for row in 0..mv.row_count() {
        let Some(tok) = cell_token(x_col, row) else { continue };
        let Some(yv) = y_col.cells.get(row).and_then(|c| c.active_number()) else { continue };
        if let Some(pos) = levels.iter().position(|l| l == &tok)
            && let Some(bucket) = groups.get_mut(pos)
        {
            bucket.push(yv);
        }
    }

    let mut data_lo = f64::INFINITY;
    let mut data_hi = f64::NEG_INFINITY;
    for g in &groups {
        for &v in g {
            data_lo = data_lo.min(v);
            data_hi = data_hi.max(v);
        }
    }
    if !data_lo.is_finite() {
        draw_empty_open_frame(scene);
        return Ok(());
    }
    if matches!(gdo, GraphicalDataObject::BarWithReplicates { .. }) {
        data_lo = data_lo.min(0.0);
        data_hi = data_hi.max(0.0);
    }
    let y_pad = (data_hi - data_lo).max(1.0) * 0.12;
    let resolved_cld = resolve_categorical_cld_overlay(graph, doc, &levels, &groups, &mut scene.warnings);
    let cld_top_margin = if resolved_cld.is_some() { crate::column_grouped::cld_reserved_top_margin_pt(&graph.spec) } else { 0.0 };
    let y_hi_pad = if resolved_cld.is_some() { (data_hi - data_lo).max(1.0) * 0.18 } else { y_pad };
    let (y_lo, y_hi) = resolve_axis_domain(
        &graph.spec.y_axis,
        data_lo - if matches!(gdo, GraphicalDataObject::BarWithReplicates { .. }) && data_lo >= 0.0 { 0.0 } else { y_pad },
        data_hi + y_hi_pad,
    );
    let y_ticks = nice_axis_ticks(&graph.spec.y_axis, y_lo, y_hi, 5);
    let max_y_w =
        update_plot_rect_for_labels(scene, &graph.spec, &y_ticks, PlotMarginConfig { extra_top_pt: cld_top_margin, ..PlotMarginConfig::default() });

    let r = scene.plot_rect_pt;
    let y_map = AxisMapping { min: y_lo, max: y_hi, scale: graph.spec.y_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale) };
    let slot_w = r.width / (levels.len() as f64);
    let cats: Vec<(f64, String)> = levels.iter().enumerate().map(|(i, l)| (r.x + ((i as f64) + 0.5) * slot_w, l.clone())).collect();
    let frame = CartesianFrame {
        spec: &graph.spec,
        x_ticks: &[],
        y_ticks: &y_ticks,
        x_map: AxisMapping { min: 0.0, max: levels.len() as f64, scale: AxisScale::Linear },
        y_map,
        max_y_label_w: max_y_w,
        doc,
    };
    draw_cartesian_frame(scene, &frame);
    draw_categorical_x_ticks(scene, &graph.spec, &cats, doc);

    let y_zero = crate::clamp_f64_safe(y_map.map_value(0.0, r.bottom(), r.y).unwrap_or(r.bottom()), r.y, r.bottom());
    let mut line_centers = Vec::new();
    let mut slot_bounds = Vec::with_capacity(groups.len());
    for (idx, values) in groups.iter().enumerate() {
        if values.is_empty() {
            continue;
        }
        let cx = r.x + ((idx as f64) + 0.5) * slot_w;
        let color = graph.spec.palette.color_at(idx);
        let box_w = (slot_w * 0.56).min(38.0);
        let mut sorted = values.clone();
        sorted.sort_by(f64::total_cmp);
        let center_y = summary_center(values, &sorted, summary_measure_from_gdo(gdo));
        let top_data_val = draw_categorical_summary_slot(scene, gdo, y_error_bars, color, r, cx, box_w, y_zero, values, &sorted, center_y, y_map);
        let top_y_pt = y_map.map_value(top_data_val, r.bottom(), r.y).map_or(r.y, |py| crate::clamp_f64_safe(py, r.y, r.bottom()));
        slot_bounds.push(crate::column_grouped::RenderedSlotBound {
            group_key: idx,
            label: levels[idx].clone(),
            alt_label: levels[idx].clone(),
            x_pt: cx,
            top_y_pt,
        });
        if let Some(yc) = y_map.map_value(center_y, r.bottom(), r.y) {
            line_centers.push((cx, yc));
        }
    }

    if let Some(cld) = &resolved_cld {
        crate::column_grouped::render_cld_labels(scene, graph, cld, &mut slot_bounds);
    }

    if matches!(gdo, GraphicalDataObject::ScatterAndLine { line: Some(_), .. } | GraphicalDataObject::BeforeAfterLines { .. })
        && line_centers.len() >= 2
    {
        let mut p = Path2D::new();
        if let Some(&(x0, y0)) = line_centers.first() {
            path_move(&mut p, PointPt::new(x0, y0));
            for &(x, y) in line_centers.iter().skip(1) {
                path_line(&mut p, PointPt::new(x, y));
            }
        }
        scene.primitives.push(ScenePrimitive::Path {
            path: p,
            fill: None,
            stroke: Some(solid_stroke(graph.spec.palette.color_at(0), 1.5, LineDashStyle::Solid)),
            clip: Some(r),
        });
    }

    Ok(())
}

fn summary_measure_from_gdo(gdo: &GraphicalDataObject) -> curvebook_doc::SummaryMeasure {
    match gdo {
        GraphicalDataObject::BarWithReplicates { summary, .. } | GraphicalDataObject::AlignedDotPlot { summary, .. } => *summary,
        GraphicalDataObject::FloatingBar { summary: Some(s), .. } => *s,
        _ => curvebook_doc::SummaryMeasure::Mean,
    }
}

fn summary_center(values: &[f64], sorted: &[f64], measure: curvebook_doc::SummaryMeasure) -> f64 {
    match measure {
        curvebook_doc::SummaryMeasure::Mean => values.iter().sum::<f64>() / (values.len().max(1) as f64),
        curvebook_doc::SummaryMeasure::Median => percentile_linear(sorted, 0.5),
        curvebook_doc::SummaryMeasure::GeometricMean => {
            if values.iter().all(|&v| v > 0.0) {
                (values.iter().map(|v| v.ln()).sum::<f64>() / (values.len() as f64)).exp()
            } else {
                values.iter().sum::<f64>() / (values.len().max(1) as f64)
            }
        }
    }
}

fn percentile_linear(sorted: &[f64], q: f64) -> f64 {
    let qc = crate::clamp_f64_safe(q, 0.0, 1.0);
    curvebook_math::quantile_type7_sorted(sorted, qc).unwrap_or(0.0)
}

fn emit_replicate_dots(scene: &mut ChartScene, cx: f64, spread_w: f64, sorted: &[f64], y_map: AxisMapping, clip: RectPt, color: Rgba8) {
    for (i, &val) in sorted.iter().enumerate() {
        let Some(py) = y_map.map_value(val, clip.bottom(), clip.y) else { continue };
        let phase = ((i as f64) * 0.618_033_988_75).fract() - 0.5;
        let px = cx + phase * spread_w;
        emit_marker(
            scene,
            PointPt::new(px, py),
            MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.5,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
            },
            Some(clip),
        );
    }
}

struct BoxSlotSpec {
    cx: f64,
    box_w: f64,
    whiskers: WhiskerMethod,
    fill_color: Rgba8,
    stroke_color: Rgba8,
    show_outliers: bool,
    show_all: bool,
    y_map: AxisMapping,
}

fn draw_box_and_whisker_slot(scene: &mut ChartScene, sorted: &[f64], slot: &BoxSlotSpec) {
    let cx = slot.cx;
    let box_w = slot.box_w;
    let fill_color = slot.fill_color;
    let stroke_color = slot.stroke_color;
    let y_map = slot.y_map;
    let r = scene.plot_rect_pt;
    let Some((q1, med, q3)) = quartiles_hinge(sorted) else { return };
    let iqr = (q3 - q1).max(0.0);
    let (w_lo, w_hi) = match slot.whiskers {
        WhiskerMethod::MinToMax => (sorted.first().copied().unwrap_or(q1), sorted.last().copied().unwrap_or(q3)),
        WhiskerMethod::Percentile10To90 => (percentile_linear(sorted, 0.10), percentile_linear(sorted, 0.90)),
        WhiskerMethod::Percentile5To95 => (percentile_linear(sorted, 0.05), percentile_linear(sorted, 0.95)),
        WhiskerMethod::Percentile2_5To97_5 => (percentile_linear(sorted, 0.025), percentile_linear(sorted, 0.975)),
        WhiskerMethod::Percentile1To99 => (percentile_linear(sorted, 0.01), percentile_linear(sorted, 0.99)),
        WhiskerMethod::Tukey1_5Iqr => {
            let lo_fence = q1 - 1.5 * iqr;
            let hi_fence = q3 + 1.5 * iqr;
            let w_l = sorted.iter().copied().find(|&v| v >= lo_fence).unwrap_or(q1);
            let w_h = sorted.iter().rev().copied().find(|&v| v <= hi_fence).unwrap_or(q3);
            (w_l, w_h)
        }
    };

    let (Some(yq1), Some(ymed), Some(yq3), Some(yw_lo), Some(yw_hi)) = (
        y_map.map_value(q1, r.bottom(), r.y),
        y_map.map_value(med, r.bottom(), r.y),
        y_map.map_value(q3, r.bottom(), r.y),
        y_map.map_value(w_lo, r.bottom(), r.y),
        y_map.map_value(w_hi, r.bottom(), r.y),
    ) else {
        return;
    };

    let stroke = solid_stroke(stroke_color, 1.1, LineDashStyle::Solid);
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(cx - box_w * 0.5, yq3.min(yq1), box_w, (yq1 - yq3).abs().max(1.0))),
        fill: Some(alpha_fill(fill_color, 175)),
        stroke: Some(stroke),
        clip: Some(r),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(cx - box_w * 0.5, ymed), PointPt::new(cx + box_w * 0.5, ymed)),
        fill: None,
        stroke: Some(solid_stroke(stroke_color, 1.8, LineDashStyle::Solid)),
        clip: Some(r),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(cx, yq3), PointPt::new(cx, yw_hi)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(r),
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(cx, yq1), PointPt::new(cx, yw_lo)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(r),
    });
    for ycap in [yw_hi, yw_lo] {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(cx - box_w * 0.25, ycap), PointPt::new(cx + box_w * 0.25, ycap)),
            fill: None,
            stroke: Some(stroke),
            clip: Some(r),
        });
    }
    if slot.show_all {
        emit_replicate_dots(scene, cx, box_w * 0.5, sorted, y_map, r, stroke_color);
    } else if slot.show_outliers {
        for &v in sorted {
            if (v < w_lo || v > w_hi)
                && let Some(py) = y_map.map_value(v, r.bottom(), r.y)
            {
                emit_marker(scene, PointPt::new(cx, py), MarkerStyle { size_pt: 4.5, fill_color, stroke_color, ..MarkerStyle::default() }, Some(r));
            }
        }
    }
}

struct ViolinSlotSpec {
    cx: f64,
    half_w: f64,
    fill_color: Rgba8,
    stroke_color: Rgba8,
    show_med: bool,
    half_violin: bool,
    y_map: AxisMapping,
}

fn draw_violin_slot(scene: &mut ChartScene, sorted: &[f64], slot: &ViolinSlotSpec) {
    let cx = slot.cx;
    let half_w = slot.half_w;
    let fill_color = slot.fill_color;
    let stroke_color = slot.stroke_color;
    let y_map = slot.y_map;
    let r = scene.plot_rect_pt;
    let (Some(&min_v), Some(&max_v)) = (sorted.first(), sorted.last()) else { return };
    let sd = sample_sd(sorted).unwrap_or(1.0).max((max_v - min_v) * 0.08).max(1e-6);
    let bw = 1.06 * sd * (sorted.len() as f64).powf(-0.2);
    let steps = 24_usize;
    let mut ys = Vec::with_capacity(steps + 1);
    let mut densities = Vec::with_capacity(steps + 1);
    let span = (max_v - min_v).max(bw * 0.5);
    for i in 0..=steps {
        let y = min_v + span * ((i as f64) / (steps as f64));
        let mut dens = 0.0;
        for &v in sorted {
            let z = (y - v) / bw;
            dens += (-0.5 * z * z).exp();
        }
        ys.push(y);
        densities.push(dens);
    }
    let max_dens = densities.iter().copied().fold(0.0_f64, f64::max).max(1e-12);
    let mut poly = Path2D::new();
    let mut right_pts = Vec::with_capacity(steps + 1);
    let mut left_pts = Vec::with_capacity(steps + 1);
    for (&y, &d) in ys.iter().zip(densities.iter()) {
        let Some(py) = y_map.map_value(y, r.bottom(), r.y) else { continue };
        let w = (d / max_dens) * (half_w * 0.5);
        right_pts.push((cx + w, py));
        left_pts.push((if slot.half_violin { cx } else { cx - w }, py));
    }
    if let Some(&(x0, y0)) = right_pts.first() {
        path_move(&mut poly, PointPt::new(x0, y0));
        for &(x, y) in right_pts.iter().skip(1) {
            path_line(&mut poly, PointPt::new(x, y));
        }
        for &(x, y) in left_pts.iter().rev() {
            path_line(&mut poly, PointPt::new(x, y));
        }
        path_close(&mut poly);
    }
    scene.primitives.push(ScenePrimitive::Path {
        path: poly,
        fill: Some(alpha_fill(fill_color, 175)),
        stroke: Some(solid_stroke(stroke_color, 1.0, LineDashStyle::Solid)),
        clip: Some(r),
    });

    if slot.show_med
        && let Some((q1, med, q3)) = quartiles_hinge(sorted)
        && let (Some(yq1), Some(yq3), Some(ymed)) =
            (y_map.map_value(q1, r.bottom(), r.y), y_map.map_value(q3, r.bottom(), r.y), y_map.map_value(med, r.bottom(), r.y))
    {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(cx, yq1), PointPt::new(cx, yq3)),
            fill: None,
            stroke: Some(solid_stroke(stroke_color, 2.2, LineDashStyle::Solid)),
            clip: Some(r),
        });
        emit_marker(
            scene,
            PointPt::new(cx, ymed),
            MarkerStyle { size_pt: 4.0, fill_color: Rgba8::WHITE, stroke_color, ..MarkerStyle::default() },
            Some(r),
        );
    }
    if slot.half_violin {
        emit_replicate_dots(scene, cx - half_w * 0.25, half_w * 0.3, sorted, y_map, r, stroke_color);
    }
}

fn emit_error_bar(scene: &mut ChartScene, cx: f64, values: &[f64], sorted: &[f64], center: f64, eb: curvebook_doc::ErrorBarSpec, y_map: AxisMapping) {
    let clip = scene.plot_rect_pt;
    let n = values.len();
    if n == 0 {
        return;
    }
    let sd = sample_sd(values).unwrap_or(0.0);
    let (lo, hi) = match eb.metric {
        curvebook_doc::ErrorBarMetric::Sd => (center - sd, center + sd),
        curvebook_doc::ErrorBarMetric::Sem => {
            let sem = sd / (n as f64).sqrt();
            (center - sem, center + sem)
        }
        curvebook_doc::ErrorBarMetric::ConfidenceInterval95 => {
            let half = 1.96 * sd / (n as f64).sqrt();
            (center - half, center + half)
        }
        curvebook_doc::ErrorBarMetric::InterquartileRange => {
            if let Some((q1, _, q3)) = quartiles_hinge(sorted) {
                (q1, q3)
            } else {
                (center, center)
            }
        }
        curvebook_doc::ErrorBarMetric::Range => (sorted.first().copied().unwrap_or(center), sorted.last().copied().unwrap_or(center)),
        curvebook_doc::ErrorBarMetric::ExplicitSubcolumns => (center - sd, center + sd),
    };
    let y_lo_val = if matches!(eb.direction, curvebook_doc::ErrorBarDirection::UpperOnly) { center } else { lo };
    let y_hi_val = if matches!(eb.direction, curvebook_doc::ErrorBarDirection::LowerOnly) { center } else { hi };
    let (Some(y_lo), Some(y_hi)) = (y_map.map_value(y_lo_val, clip.bottom(), clip.y), y_map.map_value(y_hi_val, clip.bottom(), clip.y)) else {
        return;
    };
    let stroke = solid_stroke(eb.color.unwrap_or(Rgba8::BLACK), eb.stroke_width_pt, eb.dash_style);
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(cx, y_lo), PointPt::new(cx, y_hi)),
        fill: None,
        stroke: Some(stroke),
        clip: Some(clip),
    });
    let half_cap = eb.cap_width_pt * 0.5;
    for y_cap in [y_lo, y_hi] {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(cx - half_cap, y_cap), PointPt::new(cx + half_cap, y_cap)),
            fill: None,
            stroke: Some(solid_stroke(eb.color.unwrap_or(Rgba8::BLACK), eb.stroke_width_pt, LineDashStyle::Solid)),
            clip: Some(clip),
        });
    }
}

// ============================================================================
// 2. All 7 DiagnosticPlotKinds
// ============================================================================

fn find_matching_result<'a>(graph: &GraphSheet, doc: &'a ProjectDocument) -> Option<&'a ResultSheet> {
    doc.result_sheet(&graph.spec.primary_source_sheet_id)
        .or_else(|| doc.result_sheets.iter().find(|r| r.analysis.source_sheet_id == graph.spec.primary_source_sheet_id))
}

fn matching_results<'a>(graph: &GraphSheet, doc: &'a ProjectDocument) -> Vec<&'a ResultSheet> {
    if let Some(direct) = doc.result_sheet(&graph.spec.primary_source_sheet_id) {
        return vec![direct];
    }
    doc.result_sheets.iter().filter(|r| r.analysis.source_sheet_id == graph.spec.primary_source_sheet_id).collect()
}

fn find_matching_result_where<'a>(graph: &GraphSheet, doc: &'a ProjectDocument, pred: impl Fn(&ResultSheet) -> bool) -> Option<&'a ResultSheet> {
    matching_results(graph, doc).into_iter().find(|r| pred(r))
}

#[derive(Debug, Clone)]
struct DiagPoint {
    x: f64,
    y: f64,
    label: Option<String>,
    source: SourceCellId,
    series_label: String,
}

fn active_xy_pairs_with_sources(sheet_id: &SheetId, xy: &curvebook_doc::XyTable, data_set: usize) -> Vec<(f64, f64, SourceCellId, String)> {
    let Some(ds) = xy.data_sets.get(data_set) else {
        return Vec::new();
    };
    let label = ds.title.clone();
    let mut out = Vec::new();
    for (row, subcells) in ds.cells.iter().enumerate() {
        let Some(x) = xy.x_value_at(row).filter(|v| v.is_finite()) else {
            continue;
        };
        if xy.y_subcolumn_mode.is_pre_summarized() {
            if let Some(mean) = subcells.first().and_then(|c| c.active_number()).filter(|v| v.is_finite()) {
                out.push((x, mean, SourceCellId { sheet_id: sheet_id.clone(), row, col: data_set, subcol: 0 }, label.clone()));
            }
        } else {
            for (subcol, cell) in subcells.iter().enumerate() {
                if let Some(y) = cell.active_number().filter(|v| v.is_finite()) {
                    out.push((x, y, SourceCellId { sheet_id: sheet_id.clone(), row, col: data_set, subcol }, label.clone()));
                }
            }
        }
    }
    out
}

fn collect_ds_active_point_sources(ds: &DataSheet, col_idx: usize, include_replicates: bool) -> (Vec<SourceCellId>, String) {
    let mut out = Vec::new();
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            let c = col_idx.min(xy.data_sets.len().saturating_sub(1));
            let label = xy.data_sets.get(c).map_or_else(|| String::from("Y"), |d| d.title.clone());
            if let Some(d) = xy.data_sets.get(c) {
                for r in 0..xy.row_count() {
                    if !xy.x_value_at(r).is_some_and(f64::is_finite) {
                        continue;
                    }
                    let Some(row_cells) = d.cells.get(r) else {
                        continue;
                    };
                    if include_replicates && !xy.y_subcolumn_mode.is_pre_summarized() {
                        for (subcol, cell) in row_cells.iter().enumerate() {
                            if cell.active_number().is_some_and(f64::is_finite) {
                                out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol });
                            }
                        }
                    } else if xy.y_subcolumn_mode.is_pre_summarized() {
                        if row_cells.first().and_then(|cell| cell.active_number()).is_some_and(f64::is_finite) {
                            out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 });
                        }
                    } else if row_cells.iter().any(|cell| cell.active_number().is_some_and(f64::is_finite)) {
                        out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 });
                    }
                }
            }
            (out, label)
        }
        TableArchetype::Column(col) => {
            let c = col_idx.min(col.data_sets.len().saturating_sub(1));
            let label = col.data_sets.get(c).map_or_else(|| format!("Group {}", c + 1), |d| d.title.clone());
            if let Some(d) = col.data_sets.get(c) {
                for r in 0..col.row_count() {
                    let Some(row_cells) = d.cells.get(r) else {
                        continue;
                    };
                    if include_replicates {
                        for (subcol, cell) in row_cells.iter().enumerate() {
                            if cell.active_number().is_some_and(f64::is_finite) {
                                out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol });
                            }
                        }
                    } else if row_cells.first().and_then(|cell| cell.active_number()).is_some_and(f64::is_finite) {
                        out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 });
                    }
                }
            }
            (out, label)
        }
        TableArchetype::MultipleVariables(mv) => {
            let c = col_idx.min(mv.variables.len().saturating_sub(1));
            let label = mv.variables.get(c).map_or_else(|| format!("Var {}", c + 1), |v| v.title.clone());
            if let Some(v) = mv.variables.get(c) {
                for r in 0..mv.row_count() {
                    if v.cells.get(r).and_then(|cell| cell.active_number()).is_some_and(f64::is_finite) {
                        out.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 });
                    }
                }
            }
            (out, label)
        }
        _ => (out, String::new()),
    }
}

fn collect_ds_active_values_with_sources(ds: &DataSheet, col_idx: usize) -> (Vec<(f64, SourceCellId)>, String) {
    let mut out = Vec::new();
    match &ds.archetype {
        TableArchetype::Column(col) => {
            let c = col_idx.min(col.data_sets.len().saturating_sub(1));
            let label = col.data_sets.get(c).map_or_else(|| format!("Group {}", c + 1), |d| d.title.clone());
            if let Some(d) = col.data_sets.get(c) {
                for r in 0..col.row_count() {
                    if let Some(row_cells) = d.cells.get(r) {
                        for (subcol, cell) in row_cells.iter().enumerate() {
                            if let Some(v) = cell.active_number().filter(|x| x.is_finite()) {
                                out.push((v, SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol }));
                            }
                        }
                    }
                }
            }
            (out, label)
        }
        TableArchetype::MultipleVariables(mv) => {
            let c = col_idx.min(mv.variables.len().saturating_sub(1));
            let label = mv.variables.get(c).map_or_else(|| format!("Var {}", c + 1), |v| v.title.clone());
            if let Some(v) = mv.variables.get(c) {
                for r in 0..mv.row_count() {
                    if let Some(val) = v.cells.get(r).and_then(|cell| cell.active_number()).filter(|x| x.is_finite()) {
                        out.push((val, SourceCellId { sheet_id: ds.id.clone(), row: r, col: c, subcol: 0 }));
                    }
                }
            }
            (out, label)
        }
        TableArchetype::Xy(xy) => {
            let c = col_idx.min(xy.data_sets.len().saturating_sub(1));
            let pairs = active_xy_pairs_with_sources(&ds.id, xy, c);
            let label = xy.data_sets.get(c).map_or_else(|| String::from("Y"), |d| d.title.clone());
            out.extend(pairs.into_iter().map(|(_, y, src, _)| (y, src)));
            (out, label)
        }
        _ => (out, String::new()),
    }
}

fn resolve_source_data_sheet<'a>(graph: &GraphSheet, doc: &'a ProjectDocument) -> Option<&'a DataSheet> {
    doc.data_sheet(&graph.spec.primary_source_sheet_id)
        .or_else(|| doc.result_sheet(&graph.spec.primary_source_sheet_id).and_then(|r| doc.data_sheet(&r.analysis.source_sheet_id)))
}

fn result_cell_num(cell: &ResultCell) -> Option<f64> {
    match cell {
        ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn result_cell_p_or_num(cell: &ResultCell) -> Option<f64> {
    match cell {
        ResultCell::PValue { raw, .. } if raw.is_finite() => Some(*raw),
        ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn find_header_col(headers: &[String], pred: impl Fn(&str) -> bool) -> Option<usize> {
    headers.iter().position(|h| pred(&h.to_ascii_lowercase()))
}

fn extract_residuals_cols(headers: &[String]) -> (Option<usize>, Option<usize>, Option<usize>, Option<usize>) {
    let ds_col = find_header_col(headers, |h| h == "data set" || h == "dataset" || h == "group");
    let y_col = find_header_col(headers, |h| {
        (h.contains("residual") || h.contains("resid"))
            && !h.contains("weighted")
            && !h.contains("studentized")
            && !h.contains("normal")
            && !h.contains("qq")
            && !h.contains("q-q")
            && !h.contains("expected")
            && !h.contains("abs")
            && !h.contains("squared")
    })
    .or_else(|| {
        find_header_col(headers, |h| {
            (h.contains("residual") || h.contains("resid")) && !h.contains("normal") && !h.contains("qq") && !h.contains("q-q")
        })
    });
    let x_col = find_header_col(headers, |h| {
        (h.contains("predicted") && !h.contains("class"))
            || h.contains("fitted")
            || h.contains("y_hat")
            || h.contains("yhat")
            || h.contains("y-hat")
            || h.contains('\u{0177}')
            || h.contains("model y")
    })
    .or_else(|| find_header_col(headers, |h| h.contains("predicted")))
    .or_else(|| find_header_col(headers, |h| h == "x" || h.starts_with("x ") || h.ends_with(" (x)")));
    let obs_col = find_header_col(headers, |h| h.contains("observed"));
    (ds_col, x_col, y_col, obs_col)
}

fn extract_residuals_tab_points(res: &ResultSheet, tab: &ResultTab, graph: &GraphSheet, doc: &ProjectDocument) -> Vec<DiagPoint> {
    let (ds_col, x_col, y_col, obs_col) = extract_residuals_cols(&tab.column_headers);
    let target_label = selected_dataset_label_in_tab(tab, ds_col, graph, doc);
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let fallback_sheet_id =
        doc.data_sheet(&res.analysis.source_sheet_id).map(|d| d.id.clone()).unwrap_or_else(|| graph.spec.primary_source_sheet_id.clone());

    let pairs: Vec<(usize, f64, f64)> = tab
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
            {
                return None;
            }
            let (xv, yv) = match (x_col, y_col, obs_col) {
                (Some(xc), Some(yc), _) => (result_cell_num(row.get(xc)?)?, result_cell_num(row.get(yc)?)?),
                (Some(xc), None, Some(oc)) if xc != oc => {
                    let fitted = result_cell_num(row.get(xc)?)?;
                    let observed = result_cell_num(row.get(oc)?)?;
                    (fitted, observed - fitted)
                }
                (None, Some(yc), _) => {
                    let yv = result_cell_num(row.get(yc)?)?;
                    let xv = row.iter().enumerate().find_map(|(ci, c)| if ci != yc { result_cell_num(c) } else { None }).unwrap_or((i + 1) as f64);
                    (xv, yv)
                }
                _ => match (row.first().and_then(result_cell_num), row.get(1).and_then(result_cell_num)) {
                    (Some(a), Some(b)) => (a, b),
                    (Some(r), None) => ((i + 1) as f64, r),
                    _ => return None,
                },
            };
            Some((i, xv, yv))
        })
        .collect();

    if pairs.is_empty() {
        return Vec::new();
    }

    let source_col = match &res.analysis.kind {
        AnalysisKind::GeneralizedLinearModel(glm) => glm.response_column,
        _ => ds_idx,
    };
    let ds_opt = doc.data_sheet(&res.analysis.source_sheet_id).or_else(|| resolve_source_data_sheet(graph, doc));
    let (resolved_sources, series_label) = if let Some(ds) = ds_opt {
        let (all_rep_sources, lbl) = collect_ds_active_point_sources(ds, source_col, true);
        if all_rep_sources.len() == pairs.len() {
            (Some(all_rep_sources), lbl)
        } else {
            let (row_sources, lbl_row) = collect_ds_active_point_sources(ds, source_col, false);
            if row_sources.len() == pairs.len() { (Some(row_sources), lbl_row) } else { (None, lbl) }
        }
    } else {
        (None, target_label.clone().unwrap_or_else(|| graph.spec.title.clone()))
    };
    let resolved_label = if series_label.is_empty() { graph.spec.title.clone() } else { series_label };

    pairs
        .into_iter()
        .enumerate()
        .map(|(k, (tab_row_idx, x, y))| {
            let source = resolved_sources.as_ref().and_then(|v| v.get(k)).cloned().unwrap_or(SourceCellId {
                sheet_id: fallback_sheet_id.clone(),
                row: tab_row_idx,
                col: source_col,
                subcol: 0,
            });
            DiagPoint { x, y, label: tab.row_labels.get(tab_row_idx).cloned(), source, series_label: resolved_label.clone() }
        })
        .collect()
}

// 2a. Heatmap (+ Perceptual Colorbar + Optional Hierarchical Dendrogram)

struct HeatmapData {
    row_labels: Vec<String>,
    col_labels: Vec<String>,
    values: Vec<Vec<Option<f64>>>,
    merges: Vec<DendrogramMerge>,
    cluster_distance: ClusterDistance,
    cluster_linkage: ClusterLinkage,
    clustering_cue: Option<String>,
}

fn cluster_metric_labels(distance: ClusterDistance, linkage: ClusterLinkage) -> (&'static str, &'static str) {
    let d = match distance {
        ClusterDistance::Euclidean => "Euclidean",
        ClusterDistance::Manhattan => "Manhattan",
        ClusterDistance::Pearson => "Pearson",
        ClusterDistance::Cosine => "Cosine",
    };
    let l = match linkage {
        ClusterLinkage::Ward => "Ward",
        ClusterLinkage::Complete => "Complete",
        ClusterLinkage::Single => "Single",
        ClusterLinkage::Upgma => "UPGMA",
        ClusterLinkage::Wpgma => "WPGMA",
        ClusterLinkage::Centroid => "Centroid",
        ClusterLinkage::Median => "Median",
    };
    (d, l)
}

fn compile_heatmap(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some(mut hm) = extract_heatmap_data(graph, doc) else {
        scene.warnings.push(ChartWarning::new("empty_heatmap", "no numeric matrix available for heatmap"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let n_rows = hm.values.len();
    let n_cols = hm.col_labels.len();
    if n_rows == 0 || n_cols == 0 {
        draw_empty_open_frame(scene);
        return Ok(());
    }

    if !hm.values.iter().flatten().any(|v| v.is_some_and(f64::is_finite)) {
        scene.warnings.push(ChartWarning::new("empty_heatmap", "no finite numeric values available for heatmap"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    if hm.merges.is_empty() && (2..=128).contains(&n_rows) && n_cols >= 2 {
        hm.merges = cluster_heatmap_rows(&hm.values, n_cols, hm.cluster_distance, hm.cluster_linkage, &mut scene.warnings);
    }

    let row_order = if !hm.merges.is_empty() { dendrogram_leaf_order(&hm.merges, n_rows) } else { (0..n_rows).collect() };
    let has_dendro = !hm.merges.is_empty();

    let (min_v, max_v) = heatmap_min_max(&hm.values);
    let has_neg_and_pos = min_v < 0.0 && max_v > 0.0;
    let (ramp, dom_lo, dom_hi) = match graph.spec.multivariate.as_ref().map(|m| &m.color) {
        Some(ColorChannelMapping::Continuous { ramp, domain_clamp, .. }) => {
            let (lo, hi) = domain_clamp.map_or((min_v, max_v), |(a, b)| sanitize_range(a, b, (min_v, max_v)));
            (ramp.clone(), lo, hi)
        }
        _ if has_neg_and_pos => {
            let m = min_v.abs().max(max_v.abs()).max(1e-6);
            (ColorRamp::Named { id: ColormapId::DivergingBlueRed, reversed: false }, -m, m)
        }
        _ => (ColorRamp::Named { id: ColormapId::Viridis, reversed: false }, min_v, max_v),
    };

    let lbl_spec = graph.spec.heatmap_labels;
    let max_row_lbl_w = match lbl_spec.row_labels {
        HeatmapRowLabelMode::RowTitles => hm.row_labels.iter().map(|l| text_metrics(l, 7.5).width_pt).fold(0.0_f64, f64::max),
        HeatmapRowLabelMode::RowNumbers => text_metrics(&format!("{n_rows}"), 7.5).width_pt,
        HeatmapRowLabelMode::None => 0.0,
    };
    let resolved_col_labels: Vec<String> = match lbl_spec.column_labels {
        HeatmapColumnLabelMode::ColumnTitles => hm.col_labels.clone(),
        HeatmapColumnLabelMode::ColumnLetters => (0..n_cols).map(spreadsheet_column_letters).collect(),
        HeatmapColumnLabelMode::None => Vec::new(),
    };
    let max_col_lbl_w = resolved_col_labels.iter().map(|l| text_metrics(l, 7.5).width_pt).fold(0.0_f64, f64::max);
    let col_lbl_proj_h = if lbl_spec.column_labels == HeatmapColumnLabelMode::None || resolved_col_labels.is_empty() {
        0.0
    } else {
        match lbl_spec.column_angle {
            HeatmapColumnLabelAngle::Horizontal0 => 14.0,
            HeatmapColumnLabelAngle::Angled45 => (max_col_lbl_w * std::f64::consts::FRAC_1_SQRT_2 + 10.0).max(16.0),
            HeatmapColumnLabelAngle::Vertical90 => (max_col_lbl_w + 10.0).max(16.0),
        }
    };
    let cue_extra = if has_dendro && hm.clustering_cue.is_some() { 13.0 } else { 0.0 };
    let desired_bottom_extra = (col_lbl_proj_h + cue_extra - 10.0).max(if has_dendro && hm.clustering_cue.is_some() { 26.0 } else { 18.0 });
    let _ = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &[],
        PlotMarginConfig {
            extra_bottom_pt: desired_bottom_extra.min(scene.height_pt * 0.35),
            extra_left_pt: crate::clamp_f64_safe(max_row_lbl_w + if has_dendro { 34.0 } else { 8.0 }, 12.0, 90.0),
            has_colorbar: true,
            suppress_y_axis_labels: true,
            ..PlotMarginConfig::default()
        },
    );

    let r = scene.plot_rect_pt;
    let dendro_w = if has_dendro { crate::clamp_f64_safe(r.width * 0.16, 20.0, 48.0) } else { 0.0 };
    let grid = RectPt::new(r.x + dendro_w, r.y, (r.width - dendro_w).max(20.0), r.height);
    let cell_w = grid.width / (n_cols as f64);
    let cell_h = grid.height / (n_rows as f64);
    let gap = if lbl_spec.cell_gap_pt.is_finite() { lbl_spec.cell_gap_pt.clamp(0.0, (cell_w.min(cell_h) * 0.4).max(0.0)) } else { 0.0 };
    let half_gap = gap * 0.5;
    let cell_border = solid_stroke(Rgba8::rgba(255, 255, 255, 160), 0.5, LineDashStyle::Solid);
    let missing_border = solid_stroke(Rgba8::rgba(170, 176, 186, 180), 0.5, LineDashStyle::Solid);
    let missing_hatch = solid_stroke(Rgba8::rgba(150, 156, 166, 175), 0.65, LineDashStyle::Solid);
    let denom = (dom_hi - dom_lo).abs().max(1e-12);

    for (vis_row, &orig_row) in row_order.iter().enumerate() {
        let Some(row_vals) = hm.values.get(orig_row) else { continue };
        let y = grid.y + (vis_row as f64) * cell_h;
        for c in 0..n_cols {
            let x = grid.x + (c as f64) * cell_w;
            let tile_rect = RectPt::new(x + half_gap, y + half_gap, (cell_w - gap).max(1.0), (cell_h - gap).max(1.0));
            if let Some(val) = row_vals.get(c).copied().flatten().filter(|v| v.is_finite()) {
                let t = crate::clamp_f64_safe((val - dom_lo) / denom, 0.0, 1.0);
                let fill_color = ramp.sample(t);
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(tile_rect),
                    fill: Some(FillPaint::solid(fill_color)),
                    stroke: Some(cell_border),
                    clip: None,
                });
                if lbl_spec.annotate_cells {
                    let font_pt = crate::clamp_f64_safe(tile_rect.width.min(tile_rect.height) * 0.36, 5.5, 9.0);
                    let ink =
                        if Rgba8::WHITE.contrast_ratio(fill_color) >= Rgba8::BLACK.contrast_ratio(fill_color) { Rgba8::WHITE } else { Rgba8::BLACK };
                    scene.primitives.push(ScenePrimitive::Text(PositionedText {
                        rich: RichText::plain(crate::scales::format_scalar_clean(val)),
                        anchor_pt: PointPt::new(tile_rect.x + tile_rect.width * 0.5, tile_rect.y + tile_rect.height * 0.5 + font_pt * 0.35),
                        anchor: TextAnchor::Middle,
                        font_size_pt: font_pt,
                        rotation_deg: 0.0,
                        color: ink,
                    }));
                }
            } else {
                scene.primitives.push(ScenePrimitive::Path { path: Path2D::rect(tile_rect), fill: None, stroke: Some(missing_border), clip: None });
                let mut slash = Path2D::new();
                path_move(&mut slash, PointPt::new(tile_rect.x, tile_rect.y + tile_rect.height));
                path_line(&mut slash, PointPt::new(tile_rect.x + tile_rect.width, tile_rect.y));
                scene.primitives.push(ScenePrimitive::Path { path: slash, fill: None, stroke: Some(missing_hatch), clip: None });
            }
        }
        match lbl_spec.row_labels {
            HeatmapRowLabelMode::RowTitles => {
                let lbl = hm.row_labels.get(orig_row).map(String::as_str).unwrap_or("");
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: parse_markup(lbl),
                    anchor_pt: PointPt::new(r.x - 5.0, y + cell_h * 0.5 + 3.0),
                    anchor: TextAnchor::End,
                    font_size_pt: 7.5,
                    rotation_deg: 0.0,
                    color: Rgba8::BLACK,
                }));
            }
            HeatmapRowLabelMode::RowNumbers => {
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: RichText::plain(format!("{}", orig_row + 1)),
                    anchor_pt: PointPt::new(r.x - 5.0, y + cell_h * 0.5 + 3.0),
                    anchor: TextAnchor::End,
                    font_size_pt: 7.5,
                    rotation_deg: 0.0,
                    color: Rgba8::BLACK,
                }));
            }
            HeatmapRowLabelMode::None => {}
        }
    }

    for (c, lbl) in resolved_col_labels.iter().enumerate() {
        let cx = grid.x + ((c as f64) + 0.5) * cell_w;
        let (anchor_pt, anchor, rotation_deg) = match lbl_spec.column_angle {
            HeatmapColumnLabelAngle::Horizontal0 => (PointPt::new(cx, grid.bottom() + 11.0), TextAnchor::Middle, 0.0),
            HeatmapColumnLabelAngle::Angled45 => (PointPt::new(cx + 2.0, grid.bottom() + 6.0), TextAnchor::End, -45.0),
            HeatmapColumnLabelAngle::Vertical90 => (PointPt::new(cx + 2.5, grid.bottom() + 6.0), TextAnchor::End, -90.0),
        };
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(lbl),
            anchor_pt,
            anchor,
            font_size_pt: 7.5,
            rotation_deg,
            color: Rgba8::BLACK,
        }));
    }

    if has_dendro {
        draw_left_dendrogram(scene, RectPt::new(r.x, r.y, dendro_w - 3.0, grid.height), &hm.merges, &row_order, cell_h);
        if let Some(cue) = &hm.clustering_cue {
            let cue_y = (grid.bottom() + col_lbl_proj_h.max(14.0) + 7.5).min(scene.height_pt - 4.0);
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(cue.clone()),
                anchor_pt: PointPt::new(grid.x + grid.width * 0.5, cue_y),
                anchor: TextAnchor::Middle,
                font_size_pt: 7.0,
                rotation_deg: 0.0,
                color: Rgba8::rgb(85, 92, 102),
            }));
        }
    }

    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(grid),
        fill: None,
        stroke: Some(solid_stroke(Rgba8::BLACK, 0.85, LineDashStyle::Solid)),
        clip: None,
    });
    draw_colorbar(scene, &ramp, dom_lo, dom_hi, "", RectPt::new(grid.right() + 10.0, grid.y + 4.0, 12.0, (grid.height - 8.0).max(24.0)));
    Ok(())
}

fn spreadsheet_column_letters(mut index: usize) -> String {
    let mut chars = Vec::new();
    loop {
        chars.push((b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    chars.into_iter().rev().collect()
}

fn correlation_heatmap_data(res: &ResultSheet, tab: &ResultTab) -> HeatmapData {
    let mut matrix = Vec::with_capacity(tab.rows.len());
    for row in &tab.rows {
        let vals: Vec<Option<f64>> = row
            .iter()
            .map(|c| match c {
                ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
                _ => None,
            })
            .collect();
        matrix.push(vals);
    }
    let n_rows = matrix.len();
    let has_missing = matrix.iter().flatten().any(Option::is_none);
    let merges = if has_missing {
        Vec::new()
    } else {
        res.tabs.iter().find(|t| t.id == "hierarchical-dendrogram").map_or_else(Vec::new, |t| extract_dendro_merges_from_tab(t, n_rows))
    };
    HeatmapData {
        row_labels: tab.row_labels.clone(),
        col_labels: tab.column_headers.clone(),
        values: matrix,
        merges,
        cluster_distance: ClusterDistance::Euclidean,
        cluster_linkage: ClusterLinkage::Ward,
        clustering_cue: None,
    }
}

fn resolve_selected_heatmap_cols(selected: &[usize], len: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for &idx in selected {
        if idx < len && !out.contains(&idx) {
            out.push(idx);
        }
    }
    out
}

fn extract_heatmap_data(graph: &GraphSheet, doc: &ProjectDocument) -> Option<HeatmapData> {
    let direct_res = doc.result_sheet(&graph.spec.primary_source_sheet_id);
    if let Some(res) = direct_res
        && matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
    {
        if matches!(res.status, ResultStatus::Failed { .. }) {
            return None;
        }
        if let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary) {
            return Some(correlation_heatmap_data(res, tab));
        }
    }

    let clustering_res = find_matching_result_where(graph, doc, |r| {
        !matches!(r.status, ResultStatus::Failed { .. }) && matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. }))
    });

    if clustering_res.is_none()
        && direct_res.is_none()
        && let Some(res) = find_matching_result(graph, doc)
        && !matches!(res.status, ResultStatus::Failed { .. })
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary)
        && matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
    {
        return Some(correlation_heatmap_data(res, tab));
    }

    let selected_cols = clustering_res.map(|r| r.analysis.selected_data_sets.as_slice()).unwrap_or(&[]);
    let ds = resolve_source_data_sheet(graph, doc)?;
    let (mut hm, expect_rows) = match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let num_cols: Vec<(usize, String)> = if selected_cols.is_empty() {
                mv.variables
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.role == VariableRole::Continuous || v.role == VariableRole::Calculated)
                    .map(|(i, v)| (i, v.title.clone()))
                    .collect()
            } else {
                resolve_selected_heatmap_cols(selected_cols, mv.variables.len())
                    .into_iter()
                    .filter_map(|i| {
                        let v = mv.variables.get(i)?;
                        (v.role == VariableRole::Continuous || v.role == VariableRole::Calculated).then(|| (i, v.title.clone()))
                    })
                    .collect()
            };
            if num_cols.is_empty() {
                return None;
            }
            let label_col = graph
                .spec
                .multivariate
                .as_ref()
                .and_then(|m| m.label_column)
                .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Label));
            let mut row_labels = Vec::new();
            let mut values = Vec::new();
            for r in 0..mv.row_count() {
                let row_vals: Vec<Option<f64>> = num_cols
                    .iter()
                    .map(|(ci, _)| mv.variables.get(*ci).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()).filter(|x| x.is_finite()))
                    .collect();
                let lbl = label_col
                    .and_then(|lc| mv.variables.get(lc))
                    .and_then(|v| cell_token(v, r))
                    .or_else(|| mv.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned())
                    .unwrap_or_else(|| format!("R{}", r + 1));
                row_labels.push(lbl);
                values.push(row_vals);
            }
            let n_rows = values.len();
            let col_labels = num_cols.into_iter().map(|(_, t)| t).collect();
            (
                HeatmapData {
                    row_labels,
                    col_labels,
                    values,
                    merges: Vec::new(),
                    cluster_distance: ClusterDistance::Euclidean,
                    cluster_linkage: ClusterLinkage::Ward,
                    clustering_cue: None,
                },
                n_rows,
            )
        }
        TableArchetype::Column(col) => {
            let col_indices: Vec<usize> = if selected_cols.is_empty() {
                (0..col.data_sets.len()).collect()
            } else {
                resolve_selected_heatmap_cols(selected_cols, col.data_sets.len())
            };
            if col_indices.is_empty() {
                return None;
            }
            let col_labels: Vec<String> = col_indices.iter().filter_map(|&c| col.data_sets.get(c).map(|d| d.title.clone())).collect();
            let nrows = col.row_count();
            let row_labels =
                (0..nrows).map(|r| col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("R{}", r + 1))).collect();
            let values = (0..nrows)
                .map(|r| {
                    col_indices
                        .iter()
                        .map(|&c| {
                            col.data_sets
                                .get(c)
                                .and_then(|d| d.cells.get(r))
                                .and_then(|s| s.first())
                                .and_then(|c| c.active_number())
                                .filter(|x| x.is_finite())
                        })
                        .collect()
                })
                .collect();
            (
                HeatmapData {
                    row_labels,
                    col_labels,
                    values,
                    merges: Vec::new(),
                    cluster_distance: ClusterDistance::Euclidean,
                    cluster_linkage: ClusterLinkage::Ward,
                    clustering_cue: None,
                },
                nrows,
            )
        }
        TableArchetype::Grouped(g) => {
            let col_indices: Vec<usize> = if selected_cols.is_empty() {
                (0..g.data_sets.len()).collect()
            } else {
                resolve_selected_heatmap_cols(selected_cols, g.data_sets.len())
            };
            if col_indices.is_empty() {
                return None;
            }
            let col_labels: Vec<String> = col_indices.iter().filter_map(|&c| g.data_sets.get(c).map(|d| d.title.clone())).collect();
            let nrows = g.row_count();
            let row_labels =
                (0..nrows).map(|r| g.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("R{}", r + 1))).collect();
            let values = (0..nrows)
                .map(|r| {
                    col_indices
                        .iter()
                        .map(|&c| {
                            let reps: Vec<f64> = g.active_replicates(r, c).into_iter().filter(|x| x.is_finite()).collect();
                            if reps.is_empty() {
                                None
                            } else {
                                let mean = reps.iter().sum::<f64>() / (reps.len() as f64);
                                mean.is_finite().then_some(mean)
                            }
                        })
                        .collect()
                })
                .collect();
            (
                HeatmapData {
                    row_labels,
                    col_labels,
                    values,
                    merges: Vec::new(),
                    cluster_distance: ClusterDistance::Euclidean,
                    cluster_linkage: ClusterLinkage::Ward,
                    clustering_cue: None,
                },
                nrows,
            )
        }
        _ => return None,
    };

    if let Some(cres) = clustering_res
        && let AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance, linkage, .. }) = cres.analysis.kind
    {
        hm.cluster_distance = distance;
        hm.cluster_linkage = linkage;
        let (dist_lbl, link_lbl) = cluster_metric_labels(distance, linkage);
        hm.clustering_cue = Some(format!("Clustered: {dist_lbl} / {link_lbl}"));
        let has_missing = hm.values.iter().flatten().any(Option::is_none);
        if !has_missing {
            let tab_merges = cres
                .tabs
                .iter()
                .find(|t| t.id == "hierarchical-dendrogram")
                .map_or_else(Vec::new, |t| extract_dendro_merges_from_tab(t, expect_rows));
            if !tab_merges.is_empty() && tab_merges.len() + 1 == expect_rows {
                hm.merges = tab_merges;
            }
        }
    }

    Some(hm)
}

fn parse_dendro_node_id(raw: f64, step: usize, n_leaves: usize) -> Option<isize> {
    if !raw.is_finite() || raw.fract() != 0.0 || raw.abs() > 9_007_199_254_740_991.0 {
        return None;
    }
    let min_leaf_id = isize::try_from(n_leaves).ok().and_then(isize::checked_neg)?;
    let step_i = isize::try_from(step).ok()?;
    let id = raw as isize;
    ((id as f64) == raw && id >= min_leaf_id && id < step_i).then_some(id)
}

fn decode_dendro_leaf(id: isize, n_leaves: usize) -> Option<usize> {
    if id >= 0 {
        return None;
    }
    let pos = id.checked_neg()?;
    let leaf = pos.checked_sub(1)?;
    let idx = usize::try_from(leaf).ok()?;
    (idx < n_leaves).then_some(idx)
}

fn extract_dendro_merges_from_tab(tab: &ResultTab, n_leaves: usize) -> Vec<DendrogramMerge> {
    if n_leaves < 2 || tab.rows.len() + 1 != n_leaves {
        return Vec::new();
    }
    let mut merges = Vec::with_capacity(tab.rows.len());
    for (step, row) in tab.rows.iter().enumerate() {
        let (Some(l), Some(r), Some(d), Some(sz)) = (
            row.first().and_then(result_cell_num),
            row.get(1).and_then(result_cell_num),
            row.get(2).and_then(result_cell_num),
            row.get(3).and_then(result_cell_num),
        ) else {
            return Vec::new();
        };
        let (Some(left), Some(right)) = (parse_dendro_node_id(l, step, n_leaves), parse_dendro_node_id(r, step, n_leaves)) else {
            return Vec::new();
        };
        if !d.is_finite() || d < 0.0 || left == right || !sz.is_finite() || sz.fract() != 0.0 || sz < 1.0 || sz > 9_007_199_254_740_991.0 {
            return Vec::new();
        }
        let cluster_size = sz as usize;
        if (cluster_size as f64) != sz || cluster_size == 0 || cluster_size > n_leaves {
            return Vec::new();
        }
        merges.push(DendrogramMerge { left, right, distance: d, cluster_size });
    }
    merges
}

fn heatmap_min_max(values: &[Vec<Option<f64>>]) -> (f64, f64) {
    let (lo, hi) = values
        .iter()
        .flat_map(|r| r.iter())
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    sanitize_range(lo, hi, (0.0, 1.0))
}

fn cluster_heatmap_rows(
    rows: &[Vec<Option<f64>>],
    n_cols: usize,
    distance: ClusterDistance,
    linkage: ClusterLinkage,
    warnings: &mut Vec<ChartWarning>,
) -> Vec<DendrogramMerge> {
    let is_complete = rows.iter().all(|r| r.len() >= n_cols && r[..n_cols].iter().all(|v| v.is_some_and(f64::is_finite)));
    if is_complete {
        let dense: Vec<Vec<f64>> = rows.iter().map(|r| r.iter().take(n_cols).flatten().copied().collect()).collect();
        return hierarchical_clustering_with_metrics(&dense, distance, linkage).unwrap_or_default();
    }
    if let Some(merges) = pairwise_hierarchical_clustering(rows, n_cols, distance, linkage) {
        return merges;
    }
    warnings.push(ChartWarning::new(
        "clustering_skipped_missing_data",
        "hierarchical clustering skipped because rows lack overlapping finite observations",
    ));
    Vec::new()
}

fn pairwise_row_distance(a: &[Option<f64>], b: &[Option<f64>], n_cols: usize, metric: ClusterDistance) -> Option<f64> {
    match metric {
        ClusterDistance::Euclidean | ClusterDistance::Manhattan => {
            let mut count = 0usize;
            let mut sum = 0.0;
            for c in 0..n_cols {
                let Some(va) = a.get(c).copied().flatten().filter(|x| x.is_finite()) else { continue };
                let Some(vb) = b.get(c).copied().flatten().filter(|x| x.is_finite()) else { continue };
                let diff = (va - vb).abs();
                sum += if matches!(metric, ClusterDistance::Euclidean) { diff * diff } else { diff };
                count += 1;
            }
            if count == 0 {
                return None;
            }
            let scaled = sum * ((n_cols as f64) / (count as f64));
            let dist = if matches!(metric, ClusterDistance::Euclidean) { scaled.sqrt() } else { scaled };
            dist.is_finite().then_some(dist)
        }
        ClusterDistance::Cosine => {
            let (mut count, mut dot, mut norm_a, mut norm_b) = (0usize, 0.0, 0.0, 0.0);
            for c in 0..n_cols {
                let Some(va) = a.get(c).copied().flatten().filter(|x| x.is_finite()) else { continue };
                let Some(vb) = b.get(c).copied().flatten().filter(|x| x.is_finite()) else { continue };
                dot += va * vb;
                norm_a += va * va;
                norm_b += vb * vb;
                count += 1;
            }
            if count == 0 {
                return None;
            }
            let dist = if norm_a <= 1e-30 && norm_b <= 1e-30 {
                0.0
            } else if norm_a <= 1e-30 || norm_b <= 1e-30 {
                1.0
            } else {
                let cos = (dot / (norm_a * norm_b).sqrt()).clamp(-1.0, 1.0);
                (1.0 - cos).clamp(0.0, 2.0)
            };
            dist.is_finite().then_some(dist)
        }
        ClusterDistance::Pearson => {
            let (mut count, mut sum_a, mut sum_b) = (0usize, 0.0, 0.0);
            for c in 0..n_cols {
                let Some(va) = a.get(c).copied().flatten().filter(|x| x.is_finite()) else { continue };
                let Some(vb) = b.get(c).copied().flatten().filter(|x| x.is_finite()) else { continue };
                sum_a += va;
                sum_b += vb;
                count += 1;
            }
            if count < 2 {
                return None;
            }
            let mean_a = sum_a / (count as f64);
            let mean_b = sum_b / (count as f64);
            let (mut num, mut da, mut db, mut max_diff) = (0.0, 0.0, 0.0, 0.0_f64);
            for c in 0..n_cols {
                let Some(va) = a.get(c).copied().flatten().filter(|x| x.is_finite()) else { continue };
                let Some(vb) = b.get(c).copied().flatten().filter(|x| x.is_finite()) else { continue };
                let xa = va - mean_a;
                let xb = vb - mean_b;
                num += xa * xb;
                da += xa * xa;
                db += xb * xb;
                max_diff = max_diff.max((va - vb).abs());
            }
            let denom = (da * db).sqrt();
            let dist = if !denom.is_finite() || denom <= 1e-30 {
                if max_diff < 1e-12 { 0.0 } else { 1.0 }
            } else {
                let r = (num / denom).clamp(-1.0, 1.0);
                (1.0 - r).clamp(0.0, 2.0)
            };
            dist.is_finite().then_some(dist)
        }
    }
}

fn lance_williams_distance(linkage: ClusterLinkage, d_iu: f64, d_ju: f64, d_ij: f64, n_i: f64, n_j: f64, n_u: f64) -> f64 {
    match linkage {
        ClusterLinkage::Single => d_iu.min(d_ju),
        ClusterLinkage::Complete => d_iu.max(d_ju),
        ClusterLinkage::Upgma => (n_i * d_iu + n_j * d_ju) / (n_i + n_j),
        ClusterLinkage::Wpgma => 0.5 * (d_iu + d_ju),
        ClusterLinkage::Centroid => {
            let n_k = n_i + n_j;
            let sq = (n_i * d_iu * d_iu + n_j * d_ju * d_ju) / n_k - (n_i * n_j * d_ij * d_ij) / (n_k * n_k);
            sq.max(0.0).sqrt()
        }
        ClusterLinkage::Median => (0.5 * d_iu * d_iu + 0.5 * d_ju * d_ju - 0.25 * d_ij * d_ij).max(0.0).sqrt(),
        ClusterLinkage::Ward => {
            let denom = n_i + n_j + n_u;
            let sq = ((n_i + n_u) * d_iu * d_iu + (n_j + n_u) * d_ju * d_ju - n_u * d_ij * d_ij) / denom;
            sq.max(0.0).sqrt()
        }
    }
}

fn pairwise_hierarchical_clustering(
    rows: &[Vec<Option<f64>>],
    n_cols: usize,
    distance: ClusterDistance,
    linkage: ClusterLinkage,
) -> Option<Vec<DendrogramMerge>> {
    let n = rows.len();
    if n < 2 || n_cols < 2 {
        return None;
    }

    let mut dist = vec![0.0_f64; n * n];
    for i in 0..n {
        let row_i = rows.get(i)?;
        for j in (i + 1)..n {
            let row_j = rows.get(j)?;
            let d = pairwise_row_distance(row_i, row_j, n_cols, distance)?;
            if !d.is_finite() {
                return None;
            }
            if let Some(s) = dist.get_mut(i * n + j) {
                *s = d;
            }
            if let Some(s) = dist.get_mut(j * n + i) {
                *s = d;
            }
        }
    }

    let mut active = vec![true; n];
    let mut sizes = vec![1usize; n];
    let mut node_ids: Vec<isize> =
        (0..n).map(|i| isize::try_from(i).ok().and_then(|v| v.checked_add(1)).and_then(isize::checked_neg)).collect::<Option<Vec<isize>>>()?;
    let mut merges = Vec::with_capacity(n - 1);

    for step in 0..(n - 1) {
        let mut best: Option<(usize, usize, f64)> = None;
        for i in 0..n {
            if !active.get(i).copied().unwrap_or(false) {
                continue;
            }
            for j in (i + 1)..n {
                if !active.get(j).copied().unwrap_or(false) {
                    continue;
                }
                let d = dist.get(i * n + j).copied().unwrap_or(f64::INFINITY);
                if best.is_none_or(|(_, _, bd)| d < bd) {
                    best = Some((i, j, d));
                }
            }
        }
        let (ci, cj, min_d) = best?;
        let n_i = sizes.get(ci).copied().unwrap_or(1);
        let n_j = sizes.get(cj).copied().unwrap_or(1);
        let left = node_ids.get(ci).copied().unwrap_or(0);
        let right = node_ids.get(cj).copied().unwrap_or(0);
        merges.push(DendrogramMerge { left, right, distance: min_d, cluster_size: n_i + n_j });

        for u in 0..n {
            if u == ci || u == cj || !active.get(u).copied().unwrap_or(false) {
                continue;
            }
            let d_iu = dist.get(ci * n + u).copied().unwrap_or(0.0);
            let d_ju = dist.get(cj * n + u).copied().unwrap_or(0.0);
            let n_u = sizes.get(u).copied().unwrap_or(1) as f64;
            let d_new = lance_williams_distance(linkage, d_iu, d_ju, min_d, n_i as f64, n_j as f64, n_u);
            if let Some(s) = dist.get_mut(ci * n + u) {
                *s = d_new;
            }
            if let Some(s) = dist.get_mut(u * n + ci) {
                *s = d_new;
            }
        }
        if let Some(sz) = sizes.get_mut(ci) {
            *sz = n_i + n_j;
        }
        if let Some(id) = node_ids.get_mut(ci) {
            *id = isize::try_from(step).ok()?;
        }
        if let Some(act) = active.get_mut(cj) {
            *act = false;
        }
    }

    Some(merges)
}

fn dendrogram_leaf_order(merges: &[DendrogramMerge], n_leaves: usize) -> Vec<usize> {
    if merges.is_empty() || n_leaves == 0 {
        return (0..n_leaves).collect();
    }
    let Ok(root_id) = isize::try_from(merges.len() - 1) else {
        return (0..n_leaves).collect();
    };
    let mut order = Vec::with_capacity(n_leaves);
    let mut seen_leaves = vec![false; n_leaves];
    let mut visited_merges = vec![false; merges.len()];
    let mut stack: Vec<isize> = vec![root_id];
    let mut guard = 0_usize;
    let max_steps = n_leaves.saturating_mul(4).saturating_add(16);
    while let Some(node) = stack.pop() {
        guard += 1;
        if guard > max_steps {
            break;
        }
        if node < 0 {
            if let Some(leaf) = decode_dendro_leaf(node, n_leaves)
                && let Some(seen) = seen_leaves.get_mut(leaf)
                && !*seen
            {
                *seen = true;
                order.push(leaf);
            }
        } else if let Ok(merge_idx) = usize::try_from(node)
            && let Some(visited) = visited_merges.get_mut(merge_idx)
            && !*visited
            && let Some(m) = merges.get(merge_idx)
        {
            *visited = true;
            stack.push(m.right);
            stack.push(m.left);
        }
    }
    for (i, seen) in seen_leaves.iter().enumerate() {
        if !*seen {
            order.push(i);
        }
    }
    order
}

fn draw_left_dendrogram(scene: &mut ChartScene, box_rect: RectPt, merges: &[DendrogramMerge], leaf_order: &[usize], cell_h: f64) {
    let max_dist =
        merges.iter().filter_map(|m| (m.distance.is_finite() && m.distance >= 0.0).then_some(m.distance)).fold(0.0_f64, f64::max).max(1e-9);
    let n_leaves = leaf_order.len();
    let mut node_y = vec![0.0_f64; n_leaves];
    for (vis_idx, &orig_leaf) in leaf_order.iter().enumerate() {
        if let Some(slot) = node_y.get_mut(orig_leaf) {
            *slot = box_rect.y + ((vis_idx as f64) + 0.5) * cell_h;
        }
    }
    let mut merge_coords: Vec<(f64, f64)> = vec![(box_rect.right(), box_rect.y); merges.len()];
    let lookup_pos = |id: isize, mc: &[(f64, f64)]| -> (f64, f64) {
        if id < 0 {
            let y = decode_dendro_leaf(id, n_leaves).and_then(|leaf| node_y.get(leaf).copied()).unwrap_or(box_rect.y);
            (box_rect.right(), y)
        } else {
            usize::try_from(id).ok().and_then(|idx| mc.get(idx).copied()).unwrap_or((box_rect.right(), box_rect.y))
        }
    };
    let stroke = solid_stroke(Rgba8::rgb(65, 70, 80), 0.9, LineDashStyle::Solid);
    for (step, m) in merges.iter().enumerate() {
        let (x_l, y_l) = lookup_pos(m.left, &merge_coords);
        let (x_r, y_r) = lookup_pos(m.right, &merge_coords);
        let dist_frac = if m.distance.is_finite() { (m.distance / max_dist).clamp(0.0, 1.0) } else { 0.0 };
        let x_m = box_rect.right() - dist_frac * box_rect.width;
        let mut p = Path2D::new();
        path_move(&mut p, PointPt::new(x_l, y_l));
        path_line(&mut p, PointPt::new(x_m, y_l));
        path_line(&mut p, PointPt::new(x_m, y_r));
        path_line(&mut p, PointPt::new(x_r, y_r));
        scene.primitives.push(ScenePrimitive::Path { path: p, fill: None, stroke: Some(stroke), clip: None });
        if let Some(slot) = merge_coords.get_mut(step) {
            *slot = (x_m, 0.5 * (y_l + y_r));
        }
    }
}

// 2b. Volcano Plot (-log10(p) vs log2FC + Threshold Crosshairs)

fn compile_volcano(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let pts = extract_volcano_points(graph, doc);
    if pts.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_volcano", "no fold-change / p-value observations for volcano plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let mr_res = find_matching_result_where(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::MultipleRowTTests(_)));
    let mr_threshold = mr_res.and_then(|r| match &r.analysis.kind {
        AnalysisKind::MultipleRowTTests(mr)
            if mr.multiplicity.threshold.is_finite() && mr.multiplicity.threshold > 0.0 && mr.multiplicity.threshold < 1.0 =>
        {
            Some(-mr.multiplicity.threshold.log10())
        }
        _ => None,
    });
    let (fc_neg, fc_pos, default_p_thresh) = volcano_thresholds(&graph.spec);
    let p_thresh = if graph.spec.y_axis.thresholds.is_empty() { mr_threshold.unwrap_or(default_p_thresh) } else { default_p_thresh };
    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let x_abs = x_min.abs().max(x_max.abs()).max(fc_pos).max(1.0) * 1.1;
    let (x_lo, x_hi) = resolve_axis_domain(&graph.spec.x_axis, -x_abs, x_abs);
    let (y_lo, y_hi) = resolve_axis_domain(&graph.spec.y_axis, 0.0_f64.min(y_min), (y_max.max(p_thresh + 0.5)) * 1.1);

    let x_ticks = nice_axis_ticks(&graph.spec.x_axis, x_lo, x_hi, 6);
    let y_ticks = nice_axis_ticks(&graph.spec.y_axis, y_lo, y_hi, 5);
    let max_y_w = update_plot_rect_for_labels(scene, &graph.spec, &y_ticks, PlotMarginConfig { has_legend: true, ..PlotMarginConfig::default() });
    let r = scene.plot_rect_pt;
    let x_map = AxisMapping { min: x_lo, max: x_hi, scale: AxisScale::Linear };
    let y_map = AxisMapping { min: y_lo, max: y_hi, scale: AxisScale::Linear };

    let mut frame_spec = graph.spec.clone();
    if frame_spec.x_axis.title.trim().is_empty() || frame_spec.x_axis.title == "X" {
        frame_spec.x_axis.title = String::from(if mr_threshold.is_some() { "Difference (A - B)" } else { "log2 Fold Change" });
    }
    if frame_spec.y_axis.title.trim().is_empty() || frame_spec.y_axis.title == "Y" {
        frame_spec.y_axis.title = String::from("-log10(P)");
    }
    let frame = CartesianFrame { spec: &frame_spec, x_ticks: &x_ticks, y_ticks: &y_ticks, x_map, y_map, max_y_label_w: max_y_w, doc };
    draw_cartesian_frame(scene, &frame);

    let crosshair_stroke = solid_stroke(Rgba8::rgb(110, 118, 128), 1.0, LineDashStyle::Dashed);
    for xv in [fc_neg, fc_pos] {
        if let Some(xp) = x_map.map_value(xv, r.x, r.right())
            && xp >= r.x
            && xp <= r.right()
        {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(xp, r.y), PointPt::new(xp, r.bottom())),
                fill: None,
                stroke: Some(crosshair_stroke),
                clip: Some(r),
            });
        }
    }
    if let Some(yp) = y_map.map_value(p_thresh, r.bottom(), r.y)
        && yp >= r.y
        && yp <= r.bottom()
    {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(r.x, yp), PointPt::new(r.right(), yp)),
            fill: None,
            stroke: Some(crosshair_stroke),
            clip: Some(r),
        });
    }

    let up_col = Rgba8::rgb(213, 94, 0);
    let down_col = Rgba8::rgb(0, 114, 178);
    let ns_col = Rgba8::rgb(145, 153, 163);
    for pt in &pts {
        let (fc, neg_log_p) = (pt.x, pt.y);
        let (Some(px), Some(py)) = (x_map.map_value(fc, r.x, r.right()), y_map.map_value(neg_log_p, r.bottom(), r.y)) else {
            continue;
        };
        let is_sig = neg_log_p >= p_thresh;
        let col = if is_sig && fc >= fc_pos {
            up_col
        } else if is_sig && fc <= fc_neg {
            down_col
        } else {
            ns_col
        };
        let base_marker = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: MarkerFillMode::Filled,
            size_pt: if is_sig { 5.0 } else { 3.8 },
            fill_color: col,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.4,
        };
        let Some(marker) = crate::resolve_overridden_marker_for_cell(&graph.spec, pt.source.col, pt.source.row, Some(pt.source.subcol), base_marker)
        else {
            continue;
        };
        let center_pt = PointPt::new(px, py);
        emit_marker(scene, center_pt, marker, Some(r));
        scene.plotted_points.push(PlottedPointMeta {
            center_pt,
            radius_pt: (marker.size_pt * 0.5).max(3.0),
            source_cell: pt.source.clone(),
            series_label: pt.series_label.clone(),
            value_summary: format!("log2FC={}, -log10P={}", format_point_value(fc), format_point_value(neg_log_p)),
        });
        if is_sig
            && (fc >= fc_pos || fc <= fc_neg)
            && let Some(lbl) = &pt.label
        {
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: parse_markup(lbl),
                anchor_pt: PointPt::new(px + 4.0, py - 3.0),
                anchor: TextAnchor::Start,
                font_size_pt: 7.0,
                rotation_deg: 0.0,
                color: Rgba8::BLACK,
            }));
        }
    }

    let entries = vec![
        LegendEntry {
            label: String::from("Up-regulated"),
            color: up_col,
            marker: Some(MarkerStyle { fill_color: up_col, stroke_color: up_col, size_pt: 5.0, ..MarkerStyle::default() }),
            dash: None,
            is_line: false,
        },
        LegendEntry {
            label: String::from("Down-regulated"),
            color: down_col,
            marker: Some(MarkerStyle { fill_color: down_col, stroke_color: down_col, size_pt: 5.0, ..MarkerStyle::default() }),
            dash: None,
            is_line: false,
        },
        LegendEntry {
            label: String::from("Not significant"),
            color: ns_col,
            marker: Some(MarkerStyle { fill_color: ns_col, stroke_color: ns_col, size_pt: 4.5, ..MarkerStyle::default() }),
            dash: None,
            is_line: false,
        },
    ];
    draw_legend_entries(scene, &frame_spec, &entries, doc);
    Ok(())
}

fn volcano_thresholds(spec: &GraphSpec) -> (f64, f64, f64) {
    let mut fc_cuts: Vec<f64> = spec.x_axis.thresholds.iter().map(|t| t.value).filter(|v| v.is_finite()).collect();
    fc_cuts.sort_by(f64::total_cmp);
    let (neg_fc, pos_fc) = match fc_cuts.as_slice() {
        [neg, pos, ..] => (*neg, *pos),
        [one] => (-one.abs(), one.abs()),
        [] => (-1.0, 1.0),
    };
    let p_cut = spec.y_axis.thresholds.first().map_or_else(|| -(0.05_f64.log10()), |t| t.value);
    (neg_fc, pos_fc, p_cut)
}

fn parse_source_row_hint(tab: &ResultTab, tab_idx: usize, ds_opt: Option<&DataSheet>) -> usize {
    if let Some(lbl) = tab.row_labels.get(tab_idx).map(|s| s.trim())
        && !lbl.is_empty()
    {
        if let Some(ds) = ds_opt {
            let titles: &[String] = match &ds.archetype {
                TableArchetype::Xy(xy) => &xy.row_titles,
                TableArchetype::Column(col) => &col.row_titles,
                TableArchetype::Grouped(g) => &g.row_titles,
                TableArchetype::MultipleVariables(mv) => &mv.row_titles,
                _ => &[],
            };
            if let Some(pos) = titles.iter().position(|t| t.trim() == lbl) {
                return pos;
            }
        }
        if let Some(num) = lbl.strip_prefix("Row ").and_then(|rest| rest.trim().parse::<usize>().ok())
            && num >= 1
        {
            return num - 1;
        }
    }
    tab_idx
}

fn extract_volcano_points(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<DiagPoint> {
    if let Some(res) = find_matching_result_where(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::MultipleRowTTests(_)))
        .or_else(|| doc.result_sheet(&graph.spec.primary_source_sheet_id))
        && (matches!(res.analysis.kind, AnalysisKind::MultipleRowTTests(_)) || doc.result_sheet(&graph.spec.primary_source_sheet_id).is_some())
    {
        let ds_id = res.analysis.source_sheet_id.clone();
        let ds_opt = doc.data_sheet(&ds_id);
        let default_col = graph.spec.series.first().map_or(0, |s| s.data_set_index);
        let from_pairs: Vec<DiagPoint> = res
            .pairwise_comparisons
            .iter()
            .filter_map(|rec| {
                let diff = rec.estimate_diff?;
                if !diff.is_finite() || !rec.raw_p_or_q.is_finite() || rec.raw_p_or_q < 0.0 {
                    return None;
                }
                let neg_log_p = -crate::clamp_f64_safe(rec.raw_p_or_q, 1e-300, 1.0).log10();
                let lbl = (!rec.label_a.trim().is_empty()).then(|| rec.label_a.clone());
                let series_label = lbl.clone().unwrap_or_else(|| graph.spec.title.clone());
                Some(DiagPoint {
                    x: diff,
                    y: neg_log_p,
                    label: lbl,
                    source: SourceCellId { sheet_id: ds_id.clone(), row: rec.group_a, col: default_col, subcol: 0 },
                    series_label,
                })
            })
            .collect();
        if !from_pairs.is_empty() {
            return from_pairs;
        }

        if let Some(tab) = res
            .tabs
            .iter()
            .find(|t| t.kind == ResultTabKind::MultipleComparisons)
            .or_else(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary))
        {
            let diff_col = find_header_col(&tab.column_headers, |h| {
                h.contains("diff") || h.contains("log2") || h.contains("fold") || h == "fc" || h.contains("effect")
            });
            let p_col =
                find_header_col(&tab.column_headers, |h| h.contains("adj") || h.contains("q value") || h.contains("q-value") || h.contains("p / q"))
                    .or_else(|| find_header_col(&tab.column_headers, |h| h.contains("raw p") || h.contains("p value") || h.contains("p-value")))
                    .or_else(|| tab.rows.first().and_then(|r| r.iter().position(|c| matches!(c, ResultCell::PValue { .. }))));
            if let (Some(dc), Some(pc)) = (diff_col, p_col) {
                let already_neg_log = tab.column_headers.get(pc).is_some_and(|h| h.to_ascii_lowercase().contains("-log10"));
                let from_tab: Vec<DiagPoint> = tab
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(i, row)| {
                        let diff = result_cell_num(row.get(dc)?)?;
                        let p_raw = result_cell_p_or_num(row.get(pc)?)?;
                        let neg_log_p = if already_neg_log {
                            p_raw
                        } else if p_raw >= 0.0 {
                            -crate::clamp_f64_safe(p_raw, 1e-300, 1.0).log10()
                        } else {
                            return None;
                        };
                        let lbl = tab.row_labels.get(i).filter(|s| !s.trim().is_empty()).cloned();
                        let row_idx = parse_source_row_hint(tab, i, ds_opt);
                        let series_label = lbl.clone().unwrap_or_else(|| graph.spec.title.clone());
                        Some(DiagPoint {
                            x: diff,
                            y: neg_log_p,
                            label: lbl,
                            source: SourceCellId { sheet_id: ds_id.clone(), row: row_idx, col: default_col, subcol: 0 },
                            series_label,
                        })
                    })
                    .collect();
                if !from_tab.is_empty() {
                    return from_tab;
                }
            }
        }
    }

    if let Some(ds) = resolve_source_data_sheet(graph, doc) {
        match &ds.archetype {
            TableArchetype::MultipleVariables(mv) => {
                let x_idx = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
                let y_idx = graph.spec.multivariate.as_ref().map_or(1, |m| m.y_column);
                let lbl_idx = graph.spec.multivariate.as_ref().and_then(|m| m.label_column);
                if let (Some(xc), Some(yc)) = (mv.variables.get(x_idx), mv.variables.get(y_idx)) {
                    let mut raw = Vec::new();
                    for r in 0..mv.row_count() {
                        if let (Some(x), Some(y)) = (xc.cells.get(r).and_then(|c| c.active_number()), yc.cells.get(r).and_then(|c| c.active_number()))
                        {
                            let lbl = lbl_idx.and_then(|li| mv.variables.get(li)).and_then(|c| cell_token(c, r));
                            raw.push(DiagPoint {
                                x,
                                y,
                                label: lbl,
                                source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: y_idx, subcol: 0 },
                                series_label: yc.title.clone(),
                            });
                        }
                    }
                    let y_title_low = yc.title.to_ascii_lowercase();
                    let looks_raw_p = !y_title_low.contains("log") && !raw.is_empty() && raw.iter().all(|pt| pt.y > 0.0 && pt.y <= 1.0);
                    if looks_raw_p {
                        for pt in &mut raw {
                            pt.y = -pt.y.max(1e-300).log10();
                        }
                    }
                    return raw;
                }
            }
            TableArchetype::Xy(xy) => {
                return active_xy_pairs_with_sources(&ds.id, xy, 0)
                    .into_iter()
                    .map(|(x, y, source, series_label)| DiagPoint { x, y, label: None, source, series_label })
                    .collect();
            }
            _ => {}
        }
    }
    Vec::new()
}

// 2c. Bland-Altman Agreement Plot (Difference vs Average + Bias ± 1.96*SD LoA)

fn compile_bland_altman(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some(BlandAltmanData { points: pts, bias, loa_lo, loa_hi, percent_difference }) = extract_bland_altman_data(graph, doc) else {
        scene.warnings.push(ChartWarning::new("empty_bland_altman", "insufficient paired observations for Bland-Altman plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (mut y_min, mut y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y), b.max(p.y)));
    y_min = y_min.min(loa_lo);
    y_max = y_max.max(loa_hi);

    let x_pad = (x_max - x_min).max(1.0) * 0.1;
    let y_pad = (y_max - y_min).max(1.0) * 0.14;
    let (x_lo, x_hi) = resolve_axis_domain(&graph.spec.x_axis, x_min - x_pad, x_max + x_pad);
    let (y_lo, y_hi) = resolve_axis_domain(&graph.spec.y_axis, y_min - y_pad, y_max + y_pad);

    let x_ticks = nice_axis_ticks(&graph.spec.x_axis, x_lo, x_hi, 6);
    let y_ticks = nice_axis_ticks(&graph.spec.y_axis, y_lo, y_hi, 5);
    let max_y_w = update_plot_rect_for_labels(scene, &graph.spec, &y_ticks, PlotMarginConfig::default());

    let mut frame_spec = graph.spec.clone();
    if frame_spec.x_axis.title == "X" {
        frame_spec.x_axis.title = String::from("Average of Methods");
    }
    if frame_spec.y_axis.title == "Y" {
        frame_spec.y_axis.title = String::from(if percent_difference { "Percent Difference (%)" } else { "Difference (A - B)" });
    }

    let r = scene.plot_rect_pt;
    let x_map = AxisMapping { min: x_lo, max: x_hi, scale: AxisScale::Linear };
    let y_map = AxisMapping { min: y_lo, max: y_hi, scale: AxisScale::Linear };
    let frame = CartesianFrame { spec: &frame_spec, x_ticks: &x_ticks, y_ticks: &y_ticks, x_map, y_map, max_y_label_w: max_y_w, doc };
    draw_cartesian_frame(scene, &frame);

    let color = graph.spec.palette.color_at(0);
    for pt in &pts {
        let (avg, diff) = (pt.x, pt.y);
        if let (Some(px), Some(py)) = (x_map.map_value(avg, r.x, r.right()), y_map.map_value(diff, r.bottom(), r.y)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 5.0,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.6,
            };
            if let Some(marker) =
                crate::resolve_overridden_marker_for_cell(&graph.spec, pt.source.col, pt.source.row, Some(pt.source.subcol), base_marker)
            {
                let center_pt = PointPt::new(px, py);
                emit_marker(scene, center_pt, marker, Some(r));
                scene.plotted_points.push(PlottedPointMeta {
                    center_pt,
                    radius_pt: (marker.size_pt * 0.5).max(3.0),
                    source_cell: pt.source.clone(),
                    series_label: pt.series_label.clone(),
                    value_summary: format!("Average={}, Difference={}", format_point_value(avg), format_point_value(diff)),
                });
            }
        }
    }

    let ref_specs = [
        (bias, solid_stroke(Rgba8::rgb(0, 114, 178), 1.4, LineDashStyle::Solid), format!("Bias: {bias:.2}")),
        (loa_hi, solid_stroke(Rgba8::rgb(213, 94, 0), 1.2, LineDashStyle::Dashed), format!("+1.96 SD: {loa_hi:.2}")),
        (loa_lo, solid_stroke(Rgba8::rgb(213, 94, 0), 1.2, LineDashStyle::Dashed), format!("-1.96 SD: {loa_lo:.2}")),
    ];
    for (val, stroke, label) in ref_specs {
        if let Some(yp) = y_map.map_value(val, r.bottom(), r.y)
            && yp >= r.y
            && yp <= r.bottom()
        {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(r.x, yp), PointPt::new(r.right(), yp)),
                fill: None,
                stroke: Some(stroke),
                clip: Some(r),
            });
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(label),
                anchor_pt: PointPt::new(r.right() - 4.0, (yp - 3.5).max(r.y + 8.0)),
                anchor: TextAnchor::End,
                font_size_pt: 7.5,
                rotation_deg: 0.0,
                color: stroke.color,
            }));
        }
    }
    Ok(())
}

struct BlandAltmanData {
    points: Vec<DiagPoint>,
    bias: f64,
    loa_lo: f64,
    loa_hi: f64,
    percent_difference: bool,
}

fn find_scalar_ci(res: &ResultSheet, key_pred: impl Fn(&str) -> bool) -> Option<f64> {
    res.scalars.iter().find(|s| key_pred(&s.key.to_ascii_lowercase()) && s.value.is_finite()).map(|s| s.value)
}

fn find_tab_row_value(res: &ResultSheet, label_pred: impl Fn(&str) -> bool) -> Option<f64> {
    for tab in &res.tabs {
        for (idx, lbl) in tab.row_labels.iter().enumerate() {
            if label_pred(&lbl.to_ascii_lowercase())
                && let Some(val) = tab.rows.get(idx).and_then(|r| r.iter().find_map(result_cell_num))
            {
                return Some(val);
            }
        }
    }
    None
}

struct BlandAltmanExtractedPairs {
    sources: Vec<SourceCellId>,
    a: Vec<f64>,
    b: Vec<f64>,
    series_label: String,
}

fn extract_bland_altman_pairs_from_ds(ds: &DataSheet, col_a: usize, col_b: usize, graph: &GraphSheet) -> Option<BlandAltmanExtractedPairs> {
    match &ds.archetype {
        TableArchetype::Column(col) if col.data_sets.len() >= 2 => {
            let mut sources = Vec::new();
            let mut va = Vec::new();
            let mut vb = Vec::new();
            let label = col.data_sets.get(col_a).map_or_else(|| String::from("Method A"), |d| d.title.clone());
            for r in 0..col.row_count() {
                if let (Some(x), Some(y)) = (
                    col.data_sets.get(col_a).and_then(|d| d.cells.get(r)).and_then(|s| s.first()).and_then(|c| c.active_number()),
                    col.data_sets.get(col_b).and_then(|d| d.cells.get(r)).and_then(|s| s.first()).and_then(|c| c.active_number()),
                ) {
                    sources.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: col_a, subcol: 0 });
                    va.push(x);
                    vb.push(y);
                }
            }
            Some(BlandAltmanExtractedPairs { sources, a: va, b: vb, series_label: label })
        }
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(col_a, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(col_b, |m| m.y_column);
            let label = mv.variables.get(xi).map_or_else(|| String::from("Method A"), |v| v.title.clone());
            let mut sources = Vec::new();
            let mut va = Vec::new();
            let mut vb = Vec::new();
            for r in 0..mv.row_count() {
                if let (Some(x), Some(y)) = (
                    mv.variables.get(xi).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                    mv.variables.get(yi).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                ) {
                    sources.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: xi, subcol: 0 });
                    va.push(x);
                    vb.push(y);
                }
            }
            Some(BlandAltmanExtractedPairs { sources, a: va, b: vb, series_label: label })
        }
        TableArchetype::Xy(xy) => {
            let pairs = active_xy_pairs_with_sources(&ds.id, xy, col_a);
            let label = xy.data_sets.get(col_a).map_or_else(|| String::from("Method A"), |d| d.title.clone());
            let mut sources = Vec::with_capacity(pairs.len());
            let mut va = Vec::with_capacity(pairs.len());
            let mut vb = Vec::with_capacity(pairs.len());
            for (x, y, src, _) in pairs {
                sources.push(src);
                va.push(x);
                vb.push(y);
            }
            Some(BlandAltmanExtractedPairs { sources, a: va, b: vb, series_label: label })
        }
        _ => None,
    }
}

fn extract_bland_altman_data(graph: &GraphSheet, doc: &ProjectDocument) -> Option<BlandAltmanData> {
    let title_pct_diff = graph.spec.y_axis.title.contains('%') || graph.spec.y_axis.title.to_ascii_lowercase().contains("percent");

    if let Some(res) = find_matching_result_where(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)))
        && matches!(res.analysis.kind, AnalysisKind::BlandAltman(_))
    {
        let (col_a, col_b, spec_pct) = match &res.analysis.kind {
            AnalysisKind::BlandAltman(ba) => (ba.column_a, ba.column_b, ba.percent_difference),
            _ => (0, 1, title_pct_diff),
        };
        let mut raw_pts: Vec<(f64, f64)> = res
            .fitted_curves
            .iter()
            .find(|c| c.equation_id == "BlandAltman")
            .map(|c| c.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| (p.x, p.y)).collect())
            .unwrap_or_default();

        if raw_pts.is_empty()
            && let Some(tab) = res
                .tabs
                .iter()
                .find(|t| t.id.contains("bland-altman") && t.kind == ResultTabKind::Residuals)
                .or_else(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals))
        {
            let x_col = find_header_col(&tab.column_headers, |h| h.contains("average") || h.contains("mean")).unwrap_or(0);
            let y_col = find_header_col(&tab.column_headers, |h| h.contains("difference") || h.contains("diff")).unwrap_or(1);
            raw_pts = tab.rows.iter().filter_map(|row| Some((result_cell_num(row.get(x_col)?)?, result_cell_num(row.get(y_col)?)?))).collect();
        }

        let mut bias_opt = find_scalar_ci(res, |k| k == "bias").or_else(|| find_tab_row_value(res, |l| l == "bias"));
        let mut loa_lo_opt = find_scalar_ci(res, |k| k == "loalower" || k == "loa_lower")
            .or_else(|| find_tab_row_value(res, |l| l.contains("lower") && l.contains("loa")));
        let mut loa_hi_opt = find_scalar_ci(res, |k| k == "loaupper" || k == "loa_upper")
            .or_else(|| find_tab_row_value(res, |l| l.contains("upper") && l.contains("loa")));

        let ds_opt = doc.data_sheet(&res.analysis.source_sheet_id).or_else(|| resolve_source_data_sheet(graph, doc));
        let ds_pairs = ds_opt.and_then(|ds| extract_bland_altman_pairs_from_ds(ds, col_a, col_b, graph));
        if (raw_pts.is_empty() || bias_opt.is_none() || loa_lo_opt.is_none() || loa_hi_opt.is_none())
            && let Some(ref extracted) = ds_pairs
            && let Ok(ba) = bland_altman(&extracted.a, &extracted.b, spec_pct, res.analysis.confidence_level)
        {
            if raw_pts.is_empty() {
                raw_pts = ba.points;
            }
            bias_opt = bias_opt.or(Some(ba.bias));
            loa_lo_opt = loa_lo_opt.or(Some(ba.loa_lower));
            loa_hi_opt = loa_hi_opt.or(Some(ba.loa_upper));
        }

        if raw_pts.len() >= 2 {
            let diffs: Vec<f64> = raw_pts.iter().map(|p| p.1).collect();
            let bias = bias_opt.unwrap_or_else(|| diffs.iter().sum::<f64>() / (diffs.len() as f64));
            let sd_fallback = sample_sd(&diffs).unwrap_or(0.0);
            let sd_bias = find_scalar_ci(res, |k| k == "sdbias" || k == "sd_bias")
                .or_else(|| find_tab_row_value(res, |l| l.contains("sd of bias")))
                .unwrap_or(sd_fallback);
            let loa_lo = loa_lo_opt.unwrap_or(bias - 1.96 * sd_bias);
            let loa_hi = loa_hi_opt.unwrap_or(bias + 1.96 * sd_bias);
            let fallback_id = res.analysis.source_sheet_id.clone();
            let points: Vec<DiagPoint> = match ds_pairs {
                Some(extracted) if extracted.sources.len() == raw_pts.len() => raw_pts
                    .into_iter()
                    .zip(extracted.sources)
                    .map(|((x, y), source)| DiagPoint { x, y, label: None, source, series_label: extracted.series_label.clone() })
                    .collect(),
                Some(extracted) => raw_pts
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (x, y))| DiagPoint {
                        x,
                        y,
                        label: None,
                        source: SourceCellId { sheet_id: fallback_id.clone(), row: idx, col: col_a, subcol: 0 },
                        series_label: extracted.series_label.clone(),
                    })
                    .collect(),
                None => raw_pts
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (x, y))| DiagPoint {
                        x,
                        y,
                        label: None,
                        source: SourceCellId { sheet_id: fallback_id.clone(), row: idx, col: col_a, subcol: 0 },
                        series_label: graph.spec.title.clone(),
                    })
                    .collect(),
            };
            return Some(BlandAltmanData { points, bias, loa_lo, loa_hi, percent_difference: spec_pct });
        }
    }

    let ds = resolve_source_data_sheet(graph, doc)?;
    if let TableArchetype::Xy(xy) = &ds.archetype {
        let pairs = active_xy_pairs_with_sources(&ds.id, xy, 0);
        if graph.spec.y_axis.title.to_ascii_lowercase().contains("diff") && pairs.len() >= 2 {
            let diffs: Vec<f64> = pairs.iter().map(|p| p.1).collect();
            let bias = diffs.iter().sum::<f64>() / (diffs.len() as f64);
            let sd = sample_sd(&diffs).unwrap_or(0.0);
            let points = pairs.into_iter().map(|(x, y, source, series_label)| DiagPoint { x, y, label: None, source, series_label }).collect();
            return Some(BlandAltmanData { points, bias, loa_lo: bias - 1.96 * sd, loa_hi: bias + 1.96 * sd, percent_difference: title_pct_diff });
        }
    }
    let extracted = extract_bland_altman_pairs_from_ds(ds, 0, 1, graph)?;
    let ba = bland_altman(&extracted.a, &extracted.b, title_pct_diff, ConfidenceLevel::DEFAULT_95).ok()?;
    let points = ba
        .points
        .into_iter()
        .zip(extracted.sources)
        .map(|((x, y), source)| DiagPoint { x, y, label: None, source, series_label: extracted.series_label.clone() })
        .collect();
    Some(BlandAltmanData { points, bias: ba.bias, loa_lo: ba.loa_lower, loa_hi: ba.loa_upper, percent_difference: title_pct_diff })
}

// 2d. ROC Curve (Sensitivity vs 1-Specificity + Diagonal + AUC)

fn compile_roc_curve(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some((points, auc)) = extract_roc_data(graph, doc) else {
        scene.warnings.push(ChartWarning::new("empty_roc", "no ROC operating points or controls/cases columns available"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let mut frame_spec = graph.spec.clone();
    if frame_spec.x_axis.title == "X" {
        frame_spec.x_axis.title = String::from("1 - Specificity (FPR)");
    }
    if frame_spec.y_axis.title == "Y" {
        frame_spec.y_axis.title = String::from("Sensitivity (TPR)");
    }

    let x_ticks = nice_axis_ticks(&frame_spec.x_axis, 0.0, 1.0, 6);
    let y_ticks = nice_axis_ticks(&frame_spec.y_axis, 0.0, 1.0, 6);
    let max_y_w = update_plot_rect_for_labels(scene, &frame_spec, &y_ticks, PlotMarginConfig::default());
    let r = scene.plot_rect_pt;
    let x_map = AxisMapping { min: 0.0, max: 1.0, scale: AxisScale::Linear };
    let y_map = AxisMapping { min: 0.0, max: 1.0, scale: AxisScale::Linear };
    let frame = CartesianFrame { spec: &frame_spec, x_ticks: &x_ticks, y_ticks: &y_ticks, x_map, y_map, max_y_label_w: max_y_w, doc };
    draw_cartesian_frame(scene, &frame);

    if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
        x_map.map_value(0.0, r.x, r.right()),
        y_map.map_value(0.0, r.bottom(), r.y),
        x_map.map_value(1.0, r.x, r.right()),
        y_map.map_value(1.0, r.bottom(), r.y),
    ) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x0, y0), PointPt::new(x1, y1)),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(130, 138, 148), 1.0, LineDashStyle::Dashed)),
            clip: Some(r),
        });
    }

    let color = graph.spec.palette.color_at(0);
    let mut curve_p = Path2D::new();
    let mut area_p = Path2D::new();
    let mut started = false;
    for &(fpr, tpr) in &points {
        let (Some(px), Some(py)) = (
            x_map.map_value(crate::clamp_f64_safe(fpr, 0.0, 1.0), r.x, r.right()),
            y_map.map_value(crate::clamp_f64_safe(tpr, 0.0, 1.0), r.bottom(), r.y),
        ) else {
            continue;
        };
        if !started {
            path_move(&mut curve_p, PointPt::new(px, py));
            path_move(&mut area_p, PointPt::new(px, r.bottom()));
            path_line(&mut area_p, PointPt::new(px, py));
            started = true;
        } else {
            path_line(&mut curve_p, PointPt::new(px, py));
            path_line(&mut area_p, PointPt::new(px, py));
        }
    }
    if started {
        path_line(&mut area_p, PointPt::new(r.right(), r.bottom()));
        path_close(&mut area_p);
        scene.primitives.push(ScenePrimitive::Path { path: area_p, fill: Some(alpha_fill(color, 34)), stroke: None, clip: Some(r) });
        scene.primitives.push(ScenePrimitive::Path {
            path: curve_p,
            fill: None,
            stroke: Some(solid_stroke(color, 2.0, LineDashStyle::Solid)),
            clip: Some(r),
        });
    }

    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup(&format!("<b>AUC = {auc:.3}</b>")),
        anchor_pt: PointPt::new(r.right() - 8.0, r.bottom() - 12.0),
        anchor: TextAnchor::End,
        font_size_pt: 9.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    Ok(())
}

fn extract_roc_data(graph: &GraphSheet, doc: &ProjectDocument) -> Option<(Vec<(f64, f64)>, f64)> {
    if let Some(res) = find_matching_result(graph, doc)
        && let Some(curve) = res.fitted_curves.iter().find(|fc| fc.equation_id.eq_ignore_ascii_case("ROC"))
    {
        let pts: Vec<(f64, f64)> = curve.points.iter().map(|p| (p.x, p.y)).collect();
        let auc = res.scalars.iter().find(|s| s.key.to_ascii_lowercase().contains("auc")).map_or_else(|| trapezoidal_auc(&pts), |s| s.value);
        return Some((pts, auc));
    }

    let ds = doc.data_sheet(&graph.spec.primary_source_sheet_id)?;
    match &ds.archetype {
        TableArchetype::Column(col) if col.data_sets.len() >= 2 => {
            let controls: Vec<f64> = col.data_sets.first()?.cells.iter().filter_map(|s| s.first().and_then(|c| c.active_number())).collect();
            let cases: Vec<f64> = col.data_sets.get(1)?.cells.iter().filter_map(|s| s.first().and_then(|c| c.active_number())).collect();
            let res = roc_curve_analysis(&controls, &cases, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).ok()?;
            Some((res.points.into_iter().map(|p| (p.one_minus_specificity, p.sensitivity)).collect(), res.auc))
        }
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(1, |m| m.y_column);
            let (Some(c0), Some(c1)) = (mv.variables.get(xi), mv.variables.get(yi)) else { return None };
            if c0.role == VariableRole::Categorical || c1.role == VariableRole::Categorical {
                let (cat_col, score_col) = if c0.role == VariableRole::Categorical { (c0, c1) } else { (c1, c0) };
                let levels = column_categorical_levels(cat_col, None);
                if levels.len() >= 2 {
                    let mut controls = Vec::new();
                    let mut cases = Vec::new();
                    for r in 0..mv.row_count() {
                        if let (Some(tok), Some(sc)) = (cell_token(cat_col, r), score_col.cells.get(r).and_then(|c| c.active_number())) {
                            if levels.first().is_some_and(|l| l == &tok) {
                                controls.push(sc);
                            } else if levels.get(1).is_some_and(|l| l == &tok) {
                                cases.push(sc);
                            }
                        }
                    }
                    if !controls.is_empty() && !cases.is_empty() {
                        let res = roc_curve_analysis(&controls, &cases, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).ok()?;
                        return Some((res.points.into_iter().map(|p| (p.one_minus_specificity, p.sensitivity)).collect(), res.auc));
                    }
                }
            }
            let controls: Vec<f64> = c0.cells.iter().filter_map(|c| c.active_number()).collect();
            let cases: Vec<f64> = c1.cells.iter().filter_map(|c| c.active_number()).collect();
            let res = roc_curve_analysis(&controls, &cases, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).ok()?;
            Some((res.points.into_iter().map(|p| (p.one_minus_specificity, p.sensitivity)).collect(), res.auc))
        }
        TableArchetype::Xy(xy) => {
            let mut pts = xy.active_xy_pairs(0);
            if pts.is_empty() {
                return None;
            }
            pts.sort_by(|a, b| a.0.total_cmp(&b.0));
            let auc = trapezoidal_auc(&pts);
            Some((pts, auc))
        }
        _ => None,
    }
}

fn trapezoidal_auc(pts: &[(f64, f64)]) -> f64 {
    let mut auc = 0.0;
    for win in pts.windows(2) {
        if let (Some(&(x0, y0)), Some(&(x1, y1))) = (win.first(), win.get(1)) {
            auc += (x1 - x0) * 0.5 * (y0 + y1);
        }
    }
    crate::clamp_f64_safe(auc, 0.0, 1.0)
}

// 2e. Forest Plot (OR/HR CIs + Diamond Pooled Estimate)

struct ForestEntry {
    label: String,
    estimate: f64,
    lower: f64,
    upper: f64,
    is_pooled: bool,
}

fn compile_forest_plot(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let mut entries = extract_forest_entries(graph, doc, &mut scene.warnings);
    if entries.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_forest", "no effect estimates & confidence intervals for forest plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let x_scale = graph.spec.x_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale);
    let is_log = matches!(x_scale, AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln);
    let all_positive = entries.iter().all(|e| e.lower > 0.0 && e.estimate > 0.0 && e.upper > 0.0);
    let ref_x = if is_log
        || (all_positive
            && (graph.spec.x_axis.title.contains("OR") || graph.spec.x_axis.title.contains("HR") || graph.spec.x_axis.title.contains("Ratio")))
    {
        1.0
    } else {
        0.0
    };

    if graph.spec.forest_show_pooled_diamond
        && entries.len() >= 2
        && !entries.iter().any(|e| e.is_pooled)
        && let Some(pooled) = compute_pooled_forest_entry(&entries, is_log || ref_x == 1.0)
    {
        entries.push(pooled);
    }

    let mut x_min = entries.iter().map(|e| e.lower).fold(f64::INFINITY, f64::min).min(ref_x);
    let mut x_max = entries.iter().map(|e| e.upper).fold(f64::NEG_INFINITY, f64::max).max(ref_x);
    for tick in &graph.spec.x_axis.additional_ticks {
        if tick.value.is_finite() && (!is_log || tick.value > 0.0) {
            x_min = x_min.min(tick.value);
            x_max = x_max.max(tick.value);
        }
    }
    for thr in &graph.spec.x_axis.thresholds {
        if thr.value.is_finite() && (!is_log || thr.value > 0.0) {
            x_min = x_min.min(thr.value);
            x_max = x_max.max(thr.value);
        }
        if let Some(end_v) = thr.shade_to
            && end_v.is_finite()
            && (!is_log || end_v > 0.0)
        {
            x_min = x_min.min(end_v);
            x_max = x_max.max(end_v);
        }
    }
    let (x_lo, x_hi) = if is_log {
        resolve_axis_domain(&graph.spec.x_axis, x_min * 0.8, x_max * 1.25)
    } else {
        let pad = (x_max - x_min).max(1.0) * 0.1;
        resolve_axis_domain(&graph.spec.x_axis, x_min - pad, x_max + pad)
    };

    let x_ticks = nice_axis_ticks(&graph.spec.x_axis, x_lo, x_hi, 5);
    let max_lbl_w = entries.iter().map(|e| text_metrics(&e.label, 8.0).width_pt).fold(0.0_f64, f64::max);
    let _ = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &[],
        PlotMarginConfig {
            extra_left_pt: crate::clamp_f64_safe(max_lbl_w + 10.0, 32.0, 96.0),
            has_legend: true,
            suppress_y_axis_labels: true,
            ..PlotMarginConfig::default()
        },
    );

    let r = scene.plot_rect_pt;
    let x_map = AxisMapping { min: x_lo, max: x_hi, scale: x_scale };
    let y_map = AxisMapping { min: 0.0, max: entries.len() as f64, scale: AxisScale::Linear };
    let frame = CartesianFrame { spec: &graph.spec, x_ticks: &x_ticks, y_ticks: &[], x_map, y_map, max_y_label_w: 0.0, doc };
    draw_cartesian_frame(scene, &frame);

    let user_has_ref_line = graph.spec.x_axis.additional_ticks.iter().any(|t| t.draw_gridline && (t.value - ref_x).abs() <= 1e-9)
        || graph.spec.x_axis.thresholds.iter().any(|t| (t.value - ref_x).abs() <= 1e-9);
    if !user_has_ref_line
        && let Some(x_ref_pt) = x_map.map_value(ref_x, r.x, r.right())
        && x_ref_pt >= r.x
        && x_ref_pt <= r.right()
    {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x_ref_pt, r.y), PointPt::new(x_ref_pt, r.bottom())),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(120, 126, 134), 1.0, LineDashStyle::Dashed)),
            clip: Some(r),
        });
    }

    let row_h = r.height / (entries.len() as f64);
    let primary_col = graph.spec.palette.color_at(0);
    let pooled_col = graph.spec.palette.color_at(1);
    for (idx, entry) in entries.iter().enumerate() {
        let cy = r.y + ((idx as f64) + 0.5) * row_h;
        let lbl_markup = if entry.is_pooled { format!("<b>{}</b>", entry.label) } else { entry.label.clone() };
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&lbl_markup),
            anchor_pt: PointPt::new(r.x - 6.0, cy + 3.0),
            anchor: TextAnchor::End,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));

        let (Some(x_lo_pt), Some(x_est_pt), Some(x_hi_pt)) = (
            x_map.map_value(entry.lower, r.x, r.right()),
            x_map.map_value(entry.estimate, r.x, r.right()),
            x_map.map_value(entry.upper, r.x, r.right()),
        ) else {
            continue;
        };

        if entry.is_pooled {
            let dh = crate::clamp_f64_safe(row_h * 0.28, 3.5, 7.0);
            scene.primitives.push(ScenePrimitive::Path {
                path: polygon_path(&[(x_lo_pt, cy), (x_est_pt, cy - dh), (x_hi_pt, cy), (x_est_pt, cy + dh)]),
                fill: Some(FillPaint::solid(pooled_col)),
                stroke: Some(solid_stroke(Rgba8::BLACK, 0.8, LineDashStyle::Solid)),
                clip: Some(r),
            });
        } else {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(x_lo_pt, cy), PointPt::new(x_hi_pt, cy)),
                fill: None,
                stroke: Some(solid_stroke(primary_col, 1.2, LineDashStyle::Solid)),
                clip: Some(r),
            });
            emit_marker(
                scene,
                PointPt::new(x_est_pt, cy),
                MarkerStyle {
                    shape: MarkerShape::Square,
                    fill_mode: MarkerFillMode::Filled,
                    size_pt: 5.5,
                    fill_color: primary_col,
                    stroke_color: Rgba8::BLACK,
                    stroke_width_pt: 0.6,
                },
                Some(r),
            );
        }

        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(format!("{:.2} [{:.2}, {:.2}]", entry.estimate, entry.lower, entry.upper)),
            anchor_pt: PointPt::new(r.right() + 6.0, cy + 3.0),
            anchor: TextAnchor::Start,
            font_size_pt: 7.5,
            rotation_deg: 0.0,
            color: Rgba8::rgb(35, 35, 35),
        }));
    }
    Ok(())
}

fn push_validated_forest_entry(entries: &mut Vec<ForestEntry>, warnings: &mut Vec<ChartWarning>, label: String, est: f64, s1: f64, s2: f64) {
    let lo = s1.min(s2);
    let hi = s1.max(s2);
    if !est.is_finite() || !lo.is_finite() || !hi.is_finite() || est < lo - 1e-12 || est > hi + 1e-12 {
        warnings
            .push(ChartWarning::new("invalid_forest_interval", format!("omitted forest row '{label}': estimate {est} lies outside [{lo}, {hi}]")));
        return;
    }
    entries.push(ForestEntry {
        is_pooled: label.to_ascii_lowercase().contains("pooled"),
        label,
        estimate: est,
        lower: lo.min(est),
        upper: hi.max(est),
    });
}

fn extract_subcolumn_forest_entries(
    mode: SubcolumnMode,
    rows: &[Vec<curvebook_doc::Cell>],
    row_titles: &[String],
    warnings: &mut Vec<ChartWarning>,
) -> Vec<ForestEntry> {
    let mut entries = Vec::new();
    for (r, subs) in rows.iter().enumerate() {
        if let (Some(est), Some(s1), Some(s2)) =
            (subs.first().and_then(|c| c.active_number()), subs.get(1).and_then(|c| c.active_number()), subs.get(2).and_then(|c| c.active_number()))
        {
            let label = row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
            let (lo, hi) = if mode == SubcolumnMode::MeanLowHigh {
                if !est.is_finite() || !s1.is_finite() || !s2.is_finite() || s1 < 0.0 || s2 < 0.0 {
                    warnings.push(ChartWarning::new(
                        "invalid_forest_interval",
                        format!("omitted forest row '{label}': negative or non-finite error delta ({s1}, {s2})"),
                    ));
                    continue;
                }
                (est - s1, est + s2)
            } else {
                (s1, s2)
            };
            push_validated_forest_entry(&mut entries, warnings, label, est, lo, hi);
        }
    }
    entries
}

fn extract_forest_entries(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Vec<ForestEntry> {
    if let Some(res) = find_matching_result(graph, doc)
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals || t.kind == ResultTabKind::MainSummary)
    {
        let mut entries = Vec::new();
        let est_idx = tab
            .column_headers
            .iter()
            .position(|h| {
                let l = h.to_ascii_lowercase();
                l.contains("ratio") || l.contains("estimate") || l.contains("diff") || l.contains("beta")
            })
            .unwrap_or(0);
        for (r, row) in tab.rows.iter().enumerate() {
            if let Some((lo, hi)) = row.iter().find_map(|c| match c {
                ResultCell::ConfidenceInterval { lower: curvebook_doc::CiBound::Finite(lo), upper: curvebook_doc::CiBound::Finite(hi) } => {
                    Some((*lo, *hi))
                }
                _ => None,
            }) {
                let est = match row.get(est_idx) {
                    Some(ResultCell::Number { value, .. }) if value.is_finite() => *value,
                    _ => 0.5 * (lo + hi),
                };
                let label = tab.row_labels.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
                push_validated_forest_entry(&mut entries, warnings, label, est, lo, hi);
            }
        }
        if !entries.is_empty() {
            return entries;
        }
    }

    let Some(ds) = doc.data_sheet(&graph.spec.primary_source_sheet_id) else { return Vec::new() };
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 3 => {
            let num_cols: Vec<usize> = mv
                .variables
                .iter()
                .enumerate()
                .filter(|(_, v)| v.role != VariableRole::Label && v.role != VariableRole::Categorical)
                .map(|(i, _)| i)
                .collect();
            let lbl_col = graph
                .spec
                .multivariate
                .as_ref()
                .and_then(|m| m.label_column)
                .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Label || v.role == VariableRole::Categorical));
            let (Some(&c_est), Some(&c_lo), Some(&c_hi)) = (num_cols.first(), num_cols.get(1), num_cols.get(2)) else {
                return Vec::new();
            };
            let mut entries = Vec::new();
            for r in 0..mv.row_count() {
                if let (Some(est), Some(s1), Some(s2)) = (
                    mv.variables.get(c_est).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                    mv.variables.get(c_lo).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                    mv.variables.get(c_hi).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                ) {
                    let label = lbl_col
                        .and_then(|lc| mv.variables.get(lc))
                        .and_then(|v| cell_token(v, r))
                        .or_else(|| mv.row_titles.get(r).filter(|s| !s.is_empty()).cloned())
                        .unwrap_or_else(|| format!("Study {}", r + 1));
                    push_validated_forest_entry(&mut entries, warnings, label, est, s1, s2);
                }
            }
            entries
        }
        TableArchetype::Grouped(grouped) if matches!(grouped.subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) => {
            let Some(ds_col) = grouped.data_sets.first() else { return Vec::new() };
            extract_subcolumn_forest_entries(grouped.subcolumn_mode, &ds_col.cells, &grouped.row_titles, warnings)
        }
        TableArchetype::Column(col) if matches!(col.subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) => {
            let Some(ds_col) = col.data_sets.first() else { return Vec::new() };
            extract_subcolumn_forest_entries(col.subcolumn_mode, &ds_col.cells, &col.row_titles, warnings)
        }
        TableArchetype::Column(col) if col.data_sets.len() >= 3 => {
            let mut entries = Vec::new();
            for r in 0..col.row_count() {
                if let (Some(est), Some(s1), Some(s2)) = (
                    col.data_sets.first().and_then(|d| d.cells.get(r)).and_then(|c| c.first()).and_then(|c| c.active_number()),
                    col.data_sets.get(1).and_then(|d| d.cells.get(r)).and_then(|c| c.first()).and_then(|c| c.active_number()),
                    col.data_sets.get(2).and_then(|d| d.cells.get(r)).and_then(|c| c.first()).and_then(|c| c.active_number()),
                ) {
                    let label = col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
                    push_validated_forest_entry(&mut entries, warnings, label, est, s1, s2);
                }
            }
            entries
        }
        TableArchetype::Xy(xy) if matches!(xy.y_subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) => {
            let Some(ds_col) = xy.data_sets.first() else { return Vec::new() };
            extract_subcolumn_forest_entries(xy.y_subcolumn_mode, &ds_col.cells, &xy.row_titles, warnings)
        }
        _ => Vec::new(),
    }
}

fn compute_pooled_forest_entry(entries: &[ForestEntry], on_log_scale: bool) -> Option<ForestEntry> {
    let mut sum_w = 0.0;
    let mut sum_wy = 0.0;
    for e in entries {
        if on_log_scale && (e.estimate <= 0.0 || e.lower <= 0.0 || e.upper <= 0.0) {
            return None;
        }
        let (y, se) = if on_log_scale {
            (e.estimate.ln(), ((e.upper.ln() - e.lower.ln()) / (2.0 * 1.96)).max(1e-6))
        } else {
            (e.estimate, ((e.upper - e.lower) / (2.0 * 1.96)).max(1e-6))
        };
        let w = 1.0 / (se * se);
        sum_w += w;
        sum_wy += w * y;
    }
    if sum_w <= 0.0 {
        return None;
    }
    let y_pool = sum_wy / sum_w;
    let se_pool = (1.0 / sum_w).sqrt();
    let (est, lo, hi) = if on_log_scale {
        (y_pool.exp(), (y_pool - 1.96 * se_pool).exp(), (y_pool + 1.96 * se_pool).exp())
    } else {
        (y_pool, y_pool - 1.96 * se_pool, y_pool + 1.96 * se_pool)
    };
    Some(ForestEntry { label: String::from("Pooled (FE)"), estimate: est, lower: lo, upper: hi, is_pooled: true })
}

// 2f. Residual Plot (Fitted vs Residual + Zero Baseline)

fn compile_residual_plot(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let pts = extract_residual_points(graph, doc);
    if pts.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_residuals", "no fitted/residual pairs available"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y).min(0.0), b.max(p.y).max(0.0)));
    let x_pad = (x_max - x_min).max(1.0) * 0.08;
    let y_pad = (y_max - y_min).max(1.0) * 0.12;
    let (x_lo, x_hi) = resolve_axis_domain(&graph.spec.x_axis, x_min - x_pad, x_max + x_pad);
    let (y_lo, y_hi) = resolve_axis_domain(&graph.spec.y_axis, y_min - y_pad, y_max + y_pad);

    let mut frame_spec = graph.spec.clone();
    if frame_spec.x_axis.title == "X" {
        frame_spec.x_axis.title = String::from("Fitted Value");
    }
    if frame_spec.y_axis.title == "Y" {
        frame_spec.y_axis.title = String::from("Residual");
    }

    let x_ticks = nice_axis_ticks(&frame_spec.x_axis, x_lo, x_hi, 6);
    let y_ticks = nice_axis_ticks(&frame_spec.y_axis, y_lo, y_hi, 5);
    let max_y_w = update_plot_rect_for_labels(scene, &frame_spec, &y_ticks, PlotMarginConfig::default());
    let r = scene.plot_rect_pt;
    let x_map = AxisMapping { min: x_lo, max: x_hi, scale: AxisScale::Linear };
    let y_map = AxisMapping { min: y_lo, max: y_hi, scale: AxisScale::Linear };
    let frame = CartesianFrame { spec: &frame_spec, x_ticks: &x_ticks, y_ticks: &y_ticks, x_map, y_map, max_y_label_w: max_y_w, doc };
    draw_cartesian_frame(scene, &frame);

    if let Some(y0) = y_map.map_value(0.0, r.bottom(), r.y)
        && y0 >= r.y
        && y0 <= r.bottom()
    {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(r.x, y0), PointPt::new(r.right(), y0)),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(110, 118, 128), 1.0, LineDashStyle::Dashed)),
            clip: Some(r),
        });
    }

    let color = graph.spec.palette.color_at(0);
    for pt in &pts {
        let (xv, yv) = (pt.x, pt.y);
        if let (Some(px), Some(py)) = (x_map.map_value(xv, r.x, r.right()), y_map.map_value(yv, r.bottom(), r.y)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.8,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
            };
            if let Some(marker) =
                crate::resolve_overridden_marker_for_cell(&graph.spec, pt.source.col, pt.source.row, Some(pt.source.subcol), base_marker)
            {
                let center_pt = PointPt::new(px, py);
                emit_marker(scene, center_pt, marker, Some(r));
                scene.plotted_points.push(PlottedPointMeta {
                    center_pt,
                    radius_pt: (marker.size_pt * 0.5).max(3.0),
                    source_cell: pt.source.clone(),
                    series_label: pt.series_label.clone(),
                    value_summary: format!("Fitted={}, Residual={}", format_point_value(xv), format_point_value(yv)),
                });
            }
        }
    }
    Ok(())
}

fn extract_residual_points(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<DiagPoint> {
    if let Some(res) = find_matching_result_where(graph, doc, |r| {
        !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)) && r.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals)
    }) && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals)
    {
        let pts = extract_residuals_tab_points(res, tab, graph, doc);
        if !pts.is_empty() {
            return pts;
        }
    }

    let Some(ds) = resolve_source_data_sheet(graph, doc) else { return Vec::new() };
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let pairs_with_sources: Vec<(f64, f64, SourceCellId, String)> = match &ds.archetype {
        TableArchetype::Xy(xy) => active_xy_pairs_with_sources(&ds.id, xy, ds_idx),
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(1, |m| m.y_column);
            let y_lbl = mv.variables.get(yi).map_or_else(|| String::from("Residual"), |v| v.title.clone());
            let raw: Vec<(f64, f64, SourceCellId, String)> = (0..mv.row_count())
                .filter_map(|r| {
                    let x = mv.variables.get(xi)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                    let y = mv.variables.get(yi)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                    Some((x, y, SourceCellId { sheet_id: ds.id.clone(), row: r, col: yi, subcol: 0 }, y_lbl.clone()))
                })
                .collect();
            let y_title_low = y_lbl.to_ascii_lowercase();
            if y_title_low.contains("resid") {
                return raw.into_iter().map(|(x, y, source, series_label)| DiagPoint { x, y, label: None, source, series_label }).collect();
            }
            raw
        }
        TableArchetype::Column(col) => {
            let (val_sources, lbl) = collect_ds_active_values_with_sources(ds, ds_idx);
            let vals: Vec<f64> = val_sources.iter().map(|(v, _)| *v).collect();
            let mean = if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / (vals.len() as f64) };
            let _ = col;
            return val_sources
                .into_iter()
                .enumerate()
                .map(|(i, (v, source))| DiagPoint { x: (i + 1) as f64, y: v - mean, label: None, source, series_label: lbl.clone() })
                .collect();
        }
        _ => Vec::new(),
    };
    let raw_pairs: Vec<(f64, f64)> = pairs_with_sources.iter().map(|(x, y, _, _)| (*x, *y)).collect();
    ols_residuals(&raw_pairs)
        .into_iter()
        .zip(pairs_with_sources)
        .map(|((fitted, resid), (_, _, source, series_label))| DiagPoint { x: fitted, y: resid, label: None, source, series_label })
        .collect()
}

fn ols_residuals(pairs: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = pairs.len();
    if n < 2 {
        return pairs.to_vec();
    }
    let nf = n as f64;
    let mx = pairs.iter().map(|p| p.0).sum::<f64>() / nf;
    let my = pairs.iter().map(|p| p.1).sum::<f64>() / nf;
    let sxx = pairs.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum::<f64>();
    let sxy = pairs.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>();
    let slope = if sxx > 1e-12 { sxy / sxx } else { 0.0 };
    let intercept = my - slope * mx;
    pairs
        .iter()
        .map(|&(x, y)| {
            let y_hat = intercept + slope * x;
            (y_hat, y - y_hat)
        })
        .collect()
}

// 2g. Normal Q-Q Plot (Theoretical Normal Quantiles vs Sorted Observations)

fn compile_normal_qq_plot(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let mut pts = extract_qq_points(graph, doc);
    pts.retain(|pt| pt.x.is_finite() && pt.y.is_finite());
    if pts.len() < 3 {
        scene.warnings.push(ChartWarning::new("insufficient_qq_sample", "at least 3 finite observations required for Normal Q-Q plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y)));
    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let x_pad = (x_max - x_min).max(1.0) * 0.1;
    let y_pad = (y_max - y_min).max(1.0) * 0.1;
    let (x_lo, x_hi) = resolve_axis_domain(&graph.spec.x_axis, x_min - x_pad, x_max + x_pad);
    let (y_lo, y_hi) = resolve_axis_domain(&graph.spec.y_axis, y_min - y_pad, y_max + y_pad);

    let mut frame_spec = graph.spec.clone();
    if frame_spec.x_axis.title == "X" {
        frame_spec.x_axis.title = String::from("Theoretical Normal Quantiles");
    }
    if frame_spec.y_axis.title == "Y" {
        frame_spec.y_axis.title = String::from("Sample Quantiles");
    }

    let x_ticks = nice_axis_ticks(&frame_spec.x_axis, x_lo, x_hi, 5);
    let y_ticks = nice_axis_ticks(&frame_spec.y_axis, y_lo, y_hi, 5);
    let max_y_w = update_plot_rect_for_labels(scene, &frame_spec, &y_ticks, PlotMarginConfig::default());
    let r = scene.plot_rect_pt;
    let x_map = AxisMapping { min: x_lo, max: x_hi, scale: AxisScale::Linear };
    let y_map = AxisMapping { min: y_lo, max: y_hi, scale: AxisScale::Linear };
    let frame = CartesianFrame { spec: &frame_spec, x_ticks: &x_ticks, y_ticks: &y_ticks, x_map, y_map, max_y_label_w: max_y_w, doc };
    draw_cartesian_frame(scene, &frame);

    let mut xs: Vec<f64> = pts.iter().map(|p| p.x).collect();
    let mut ys: Vec<f64> = pts.iter().map(|p| p.y).collect();
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    if let (Some((qx1, _, qx3)), Some((qy1, _, qy3))) = (quartiles_hinge(&xs), quartiles_hinge(&ys))
        && (qx3 - qx1).abs() > 1e-12
    {
        let slope = (qy3 - qy1) / (qx3 - qx1);
        let intercept = qy1 - slope * qx1;
        if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
            x_map.map_value(x_lo, r.x, r.right()),
            y_map.map_value(intercept + slope * x_lo, r.bottom(), r.y),
            x_map.map_value(x_hi, r.x, r.right()),
            y_map.map_value(intercept + slope * x_hi, r.bottom(), r.y),
        ) {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(x0, y0), PointPt::new(x1, y1)),
                fill: None,
                stroke: Some(solid_stroke(Rgba8::rgb(120, 128, 138), 1.1, LineDashStyle::Dashed)),
                clip: Some(r),
            });
        }
    }

    let color = graph.spec.palette.color_at(0);
    for pt in &pts {
        let (z, y) = (pt.x, pt.y);
        if let (Some(px), Some(py)) = (x_map.map_value(z, r.x, r.right()), y_map.map_value(y, r.bottom(), r.y)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.8,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
            };
            if let Some(marker) =
                crate::resolve_overridden_marker_for_cell(&graph.spec, pt.source.col, pt.source.row, Some(pt.source.subcol), base_marker)
            {
                let center_pt = PointPt::new(px, py);
                emit_marker(scene, center_pt, marker, Some(r));
                scene.plotted_points.push(PlottedPointMeta {
                    center_pt,
                    radius_pt: (marker.size_pt * 0.5).max(3.0),
                    source_cell: pt.source.clone(),
                    series_label: pt.series_label.clone(),
                    value_summary: format!("Theoretical={}, Sample={}", format_point_value(z), format_point_value(y)),
                });
            }
        }
    }
    Ok(())
}

fn selected_dataset_label_in_tab(tab: &ResultTab, ds_col: Option<usize>, graph: &GraphSheet, doc: &ProjectDocument) -> Option<String> {
    let dc = ds_col?;
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let mut seen: Vec<String> = Vec::new();
    for row in &tab.rows {
        if let Some(ResultCell::Text(s)) = row.get(dc)
            && !s.is_empty()
            && !seen.iter().any(|existing| existing == s)
        {
            seen.push(s.clone());
        }
    }
    if seen.len() <= 1 {
        return seen.into_iter().next();
    }
    if let Some(ds) = resolve_source_data_sheet(graph, doc) {
        let desired_title = match &ds.archetype {
            TableArchetype::Column(col) => col.data_sets.get(ds_idx).map(|d| d.title.as_str()),
            TableArchetype::Xy(xy) => xy.data_sets.get(ds_idx).map(|d| d.title.as_str()),
            TableArchetype::MultipleVariables(mv) => mv.variables.get(ds_idx).map(|v| v.title.as_str()),
            _ => None,
        };
        if let Some(title) = desired_title
            && seen.iter().any(|s| s == title)
        {
            return Some(title.to_owned());
        }
    }
    seen.get(ds_idx).or_else(|| seen.first()).cloned()
}

fn values_to_qq_diag_points(mut vals: Vec<(f64, SourceCellId, String)>) -> Vec<DiagPoint> {
    vals.retain(|(v, _, _)| v.is_finite());
    vals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.row.cmp(&b.1.row)).then_with(|| a.1.subcol.cmp(&b.1.subcol)));
    let n = vals.len();
    if n < 3 {
        return Vec::new();
    }
    vals.into_iter()
        .enumerate()
        .filter_map(|(i, (y, source, series_label))| {
            let p = ((i as f64) + 0.5) / (n as f64);
            let z = norm_ppf(p).ok()?;
            Some(DiagPoint { x: z, y, label: None, source, series_label })
        })
        .collect()
}

fn extract_qq_points(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<DiagPoint> {
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);

    // 1. Prefer explicit ResultTabKind::NormalQq (excluding BlandAltman).
    if let Some(res) = find_matching_result_where(graph, doc, |r| {
        !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)) && r.tabs.iter().any(|t| t.kind == ResultTabKind::NormalQq)
    }) && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::NormalQq)
    {
        let ds_col = find_header_col(&tab.column_headers, |h| h == "data set" || h == "dataset" || h == "group");
        let target_label = selected_dataset_label_in_tab(tab, ds_col, graph, doc);
        let exp_col = find_header_col(&tab.column_headers, |h| {
            (h.contains("expected") || h.contains("theoretical") || h.contains("normal q")) && !h.contains("lognormal")
        })
        .or_else(|| find_header_col(&tab.column_headers, |h| h.contains("expected") || h.contains("theoretical")));
        let obs_col = find_header_col(&tab.column_headers, |h| {
            (h.contains("observed") || h.contains("sample") || h.contains("residual")) && !h.contains("lognormal")
        })
        .or_else(|| find_header_col(&tab.column_headers, |h| h.contains("observed") || h.contains("sample") || h.contains("residual")));

        let (xc, yc) = match (exp_col, obs_col) {
            (Some(e), Some(o)) => (Some(e), Some(o)),
            _ => {
                let mut num_cols = Vec::new();
                for (ci, h) in tab.column_headers.iter().enumerate() {
                    let hl = h.to_ascii_lowercase();
                    if ds_col == Some(ci) || hl.contains("order") || hl == "index" {
                        continue;
                    }
                    if tab.rows.first().and_then(|r| r.get(ci)).and_then(result_cell_num).is_some() {
                        num_cols.push(ci);
                    }
                }
                if num_cols.is_empty()
                    && let Some(first_row) = tab.rows.first()
                {
                    for (ci, cell) in first_row.iter().enumerate() {
                        if result_cell_num(cell).is_some() {
                            num_cols.push(ci);
                        }
                    }
                }
                match num_cols.as_slice() {
                    [a, b, ..] => (Some(*a), Some(*b)),
                    [a] => (None, Some(*a)),
                    [] => (None, None),
                }
            }
        };

        let fallback_sheet_id = res.analysis.source_sheet_id.clone();
        let ds_opt = doc.data_sheet(&fallback_sheet_id).or_else(|| resolve_source_data_sheet(graph, doc));
        let ds_col_idx = match ds_opt.map(|d| &d.archetype) {
            Some(TableArchetype::MultipleVariables(_)) => graph.spec.multivariate.as_ref().map_or(ds_idx, |m| m.x_column),
            _ => ds_idx,
        };
        let mut sorted_ds_vals = ds_opt.map_or_else(|| (Vec::new(), String::new()), |ds| collect_ds_active_values_with_sources(ds, ds_col_idx));
        sorted_ds_vals.0.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.row.cmp(&b.1.row)).then_with(|| a.1.subcol.cmp(&b.1.subcol)));
        let series_label =
            if sorted_ds_vals.1.is_empty() { target_label.clone().unwrap_or_else(|| graph.spec.title.clone()) } else { sorted_ds_vals.1.clone() };

        if let (Some(ec), Some(oc)) = (xc, yc) {
            let pairs: Vec<(usize, f64, f64)> = tab
                .rows
                .iter()
                .enumerate()
                .filter_map(|(i, row)| {
                    if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                        && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
                    {
                        return None;
                    }
                    Some((i, result_cell_num(row.get(ec)?)?, result_cell_num(row.get(oc)?)?))
                })
                .collect();
            if pairs.len() >= 3 {
                let can_zip_sorted = sorted_ds_vals.0.len() == pairs.len()
                    && sorted_ds_vals.0.iter().zip(pairs.iter()).all(|((dv, _), (_, _, obs))| (dv - obs).abs() <= 1e-6 * dv.abs().max(1.0));
                return pairs
                    .into_iter()
                    .enumerate()
                    .map(|(k, (tab_i, exp, obs))| {
                        let source = if can_zip_sorted {
                            sorted_ds_vals.0[k].1.clone()
                        } else {
                            SourceCellId { sheet_id: fallback_sheet_id.clone(), row: tab_i, col: ds_col_idx, subcol: 0 }
                        };
                        DiagPoint { x: exp, y: obs, label: None, source, series_label: series_label.clone() }
                    })
                    .collect();
            }
        } else if let Some(oc) = yc {
            let vals: Vec<(usize, f64)> = tab
                .rows
                .iter()
                .enumerate()
                .filter_map(|(i, row)| {
                    if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                        && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
                    {
                        return None;
                    }
                    Some((i, result_cell_num(row.get(oc)?)?))
                })
                .collect();
            if vals.len() >= 3 {
                let mut sorted_raw = sorted_ds_vals.0.clone();
                let mut tab_sorted: Vec<(usize, f64)> = vals.clone();
                tab_sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
                let can_zip = sorted_raw.len() == tab_sorted.len()
                    && sorted_raw.iter().zip(tab_sorted.iter()).all(|((dv, _), (_, ov))| (dv - ov).abs() <= 1e-6 * dv.abs().max(1.0));
                let inputs: Vec<(f64, SourceCellId, String)> = if can_zip {
                    sorted_raw.drain(..).map(|(v, src)| (v, src, series_label.clone())).collect()
                } else {
                    vals.into_iter()
                        .map(|(tab_i, v)| {
                            (v, SourceCellId { sheet_id: fallback_sheet_id.clone(), row: tab_i, col: ds_col_idx, subcol: 0 }, series_label.clone())
                        })
                        .collect()
                };
                let pts = values_to_qq_diag_points(inputs);
                if pts.len() >= 3 {
                    return pts;
                }
            }
        }
    }

    // 2. Fallback to ResultTabKind::Residuals (excluding BlandAltman pair coordinate tabs).
    if let Some(res) = find_matching_result_where(graph, doc, |r| {
        !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)) && r.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals)
    }) && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals)
    {
        let vals: Vec<(f64, SourceCellId, String)> =
            extract_residuals_tab_points(res, tab, graph, doc).into_iter().map(|pt| (pt.y, pt.source, pt.series_label)).collect();
        let pairs = values_to_qq_diag_points(vals);
        if pairs.len() >= 3 {
            return pairs;
        }
    }

    // 3. Fallback to raw DataSheet observations.
    let Some(ds) = resolve_source_data_sheet(graph, doc) else { return Vec::new() };
    let target_col = match &ds.archetype {
        TableArchetype::MultipleVariables(_) => graph.spec.multivariate.as_ref().map_or(ds_idx, |m| m.x_column),
        _ => ds_idx,
    };
    let (val_sources, label) = collect_ds_active_values_with_sources(ds, target_col);
    let series_label = if label.is_empty() { graph.spec.title.clone() } else { label };
    let tagged = val_sources.into_iter().map(|(v, src)| (v, src, series_label.clone())).collect();
    values_to_qq_diag_points(tagged)
}

// ============================================================================
// 3. Multi-Panel LayoutSheet Compilation (Gap G11)
// ============================================================================

/// Hard ceiling on total emitted scene primitives within a compiled multi-panel layout.
pub const MAX_LAYOUT_PRIMITIVES: usize = 500_000;

struct CompiledGraphEntry {
    scene: Option<ChartScene>,
    warning: Option<(&'static str, String)>,
}

struct PlacedPanel {
    panel_idx: usize,
    bounds: RectPt,
    badge: Option<String>,
    cache_idx: Option<usize>,
    scale: f64,
    tx: f64,
    ty: f64,
}

pub(crate) fn check_layout_primitive_budget(current: usize, added: usize, context: &str) -> Result<(), ChartError> {
    let next = current.saturating_add(added);
    if next > MAX_LAYOUT_PRIMITIVES {
        return Err(ChartError::ResourceLimit { message: format!("{context} primitive count {next} exceeds layout budget {MAX_LAYOUT_PRIMITIVES}") });
    }
    Ok(())
}

enum LayoutPaintOrder<'a> {
    Panel(usize),
    Other(&'a LayoutItem),
}

/// Compile a multi-panel [`LayoutSheet`] into a unified physical-page [`ChartScene`].
pub fn compile_layout_scene(layout: &LayoutSheet, doc: &ProjectDocument) -> Result<ChartScene, ChartError> {
    layout.spec.validate_item_count().map_err(|err| ChartError::ResourceLimit { message: err.to_string() })?;

    let page_pt = layout.spec.page_size_pt();
    let mut scene = ChartScene::new(layout.id.as_str(), layout.title.clone(), page_pt.width, page_pt.height, layout.spec.background_fill);

    let mut compiled_graphs: Vec<CompiledGraphEntry> = Vec::new();
    let mut graph_cache: BTreeMap<&SheetId, usize> = BTreeMap::new();
    let mut panels: Vec<PlacedPanel> = Vec::new();
    let mut paint_order: Vec<LayoutPaintOrder<'_>> = Vec::with_capacity(layout.spec.items.len());
    let mut panel_counter = 0_usize;

    for item in &layout.spec.items {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, scale_percent, badge_override, .. } => {
                let bounds = RectPt::from(*bounds_pt);
                let badge = badge_override.clone().or_else(|| layout.spec.panel_badges.badge_for(panel_counter));
                let cache_idx = if let Some(&existing) = graph_cache.get(graph_sheet_id) {
                    existing
                } else {
                    let entry = match doc.graph_sheet(graph_sheet_id) {
                        Some(gs) => match crate::compile_graph(gs, doc) {
                            Ok(compiled) => CompiledGraphEntry { scene: Some(compiled), warning: None },
                            Err(e) => CompiledGraphEntry { scene: None, warning: Some(("panel_compile_error", e.to_string())) },
                        },
                        None => CompiledGraphEntry {
                            scene: None,
                            warning: Some(("missing_layout_graph", format!("referenced graph sheet '{graph_sheet_id}' not found"))),
                        },
                    };
                    let idx = compiled_graphs.len();
                    compiled_graphs.push(entry);
                    graph_cache.insert(graph_sheet_id, idx);
                    idx
                };
                if let Some(entry) = compiled_graphs.get(cache_idx)
                    && let Some((code, message)) = &entry.warning
                {
                    scene.warnings.push(ChartWarning::new(*code, message.clone()));
                }
                let scale = compiled_graphs.get(cache_idx).and_then(|entry| entry.scene.as_ref()).map_or(1.0, |sub| {
                    let pct = if scale_percent.is_finite() && *scale_percent > 0.0 { *scale_percent / 100.0 } else { 1.0 };
                    let fit = (bounds.width / sub.width_pt.max(1.0)).min(bounds.height / sub.height_pt.max(1.0));
                    (fit * pct).max(0.05)
                });
                let p_vec_idx = panels.len();
                panels.push(PlacedPanel { panel_idx: panel_counter, bounds, badge, cache_idx: Some(cache_idx), scale, tx: bounds.x, ty: bounds.y });
                paint_order.push(LayoutPaintOrder::Panel(p_vec_idx));
                panel_counter += 1;
            }
            LayoutItem::LiveTable { .. } | LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => {
                paint_order.push(LayoutPaintOrder::Other(item));
            }
        }
    }

    if layout.spec.align_plot_origins {
        align_placed_panel_origins(&mut panels, &compiled_graphs);
    }

    let mut extra_table_work: usize = 0;
    let mut layout_image_pixels: u64 = 0;
    for entry in paint_order {
        match entry {
            LayoutPaintOrder::Panel(p_idx) => {
                let panel = &panels[p_idx];
                let sub = panel.cache_idx.and_then(|idx| compiled_graphs.get(idx)).and_then(|entry| entry.scene.as_ref());
                emit_placed_panel(&mut scene, panel, sub)?;
            }
            LayoutPaintOrder::Other(LayoutItem::LiveTable { source_sheet_id, tab_id, bounds_pt, selected_rows, selected_cols, style, .. }) => {
                render_styled_embedded_table(
                    &mut scene,
                    doc,
                    &EmbeddedTableDrawSpec {
                        sheet_id: source_sheet_id,
                        tab_id: tab_id.as_deref(),
                        bounds: RectPt::from(*bounds_pt),
                        selected_rows: selected_rows.as_deref(),
                        selected_cols: selected_cols.as_deref(),
                        style,
                        missing_warning_code: "missing_layout_table",
                    },
                    Some(&mut extra_table_work),
                )?;
            }
            LayoutPaintOrder::Other(LayoutItem::TextCallout { markdown, bounds_pt, font_size_pt, .. }) => {
                let expanded = resolve_graph_text(markdown, doc, None);
                let bounds = RectPt::from(*bounds_pt);
                let fs = if font_size_pt.is_finite() && *font_size_pt > 0.0 { *font_size_pt } else { 10.0 };
                let line_h = fs * 1.3;
                let line_count = expanded.lines().count();
                check_layout_primitive_budget(
                    scene.primitives.len().saturating_add(extra_table_work),
                    line_count.saturating_add(2),
                    "layout text callout",
                )?;
                scene.primitives.push(ScenePrimitive::PushClip { rect: bounds });
                for (line_idx, line) in expanded.lines().enumerate() {
                    scene.primitives.push(ScenePrimitive::Text(PositionedText {
                        rich: parse_markup(line),
                        anchor_pt: PointPt::new(bounds.x, bounds.y + fs + (line_idx as f64) * line_h),
                        anchor: TextAnchor::Start,
                        font_size_pt: fs,
                        rotation_deg: 0.0,
                        color: Rgba8::BLACK,
                    }));
                }
                scene.primitives.push(ScenePrimitive::PopClip);
            }
            LayoutPaintOrder::Other(LayoutItem::EmbeddedImage { asset_id, bounds_pt, rotation_deg, crop, border, badge_override, .. }) => {
                check_layout_primitive_budget(scene.primitives.len().saturating_add(extra_table_work), 3, "layout embedded image")?;
                crate::image_decode::emit_embedded_image_primitive(
                    &mut scene,
                    doc,
                    &crate::image_decode::EmbeddedImageDrawSpec {
                        asset_id,
                        bounds_pt: *bounds_pt,
                        rotation_deg: *rotation_deg,
                        crop: *crop,
                        border: *border,
                        clip: None,
                        badge_override: badge_override.as_deref(),
                    },
                    Some(&mut layout_image_pixels),
                );
            }
            LayoutPaintOrder::Other(LayoutItem::LiveGraph { .. }) => {}
        }
    }

    Ok(scene)
}

/// Align inner `plot_rect_pt` origins across panels sharing a layout column (X) or layout row (Y).
fn align_placed_panel_origins(panels: &mut [PlacedPanel], compiled_graphs: &[CompiledGraphEntry]) {
    let sub_scene_of = |panel: &PlacedPanel| -> Option<&ChartScene> {
        panel.cache_idx.and_then(|idx| compiled_graphs.get(idx)).and_then(|entry| entry.scene.as_ref())
    };
    let mut col_targets: Vec<Option<f64>> = vec![None; panels.len()];
    for i in 0..panels.len() {
        let Some(sub_i) = sub_scene_of(&panels[i]) else { continue };
        let bx_i = panels[i].bounds.x;
        let peers: Vec<usize> =
            (0..panels.len()).filter(|&j| sub_scene_of(&panels[j]).is_some() && (panels[j].bounds.x - bx_i).abs() <= 16.0).collect();
        if peers.len() >= 2 {
            let max_origin_x = peers
                .iter()
                .filter_map(|&j| sub_scene_of(&panels[j]).map(|s| panels[j].bounds.x + panels[j].scale * s.plot_rect_pt.x))
                .fold(bx_i + panels[i].scale * sub_i.plot_rect_pt.x, f64::max);
            col_targets[i] = Some(max_origin_x);
            let avail_right = (panels[i].bounds.right() - max_origin_x).max(8.0);
            let span_right = (sub_i.width_pt - sub_i.plot_rect_pt.x).max(1.0);
            panels[i].scale = panels[i].scale.min(avail_right / span_right).max(0.05);
        }
    }

    let mut row_targets: Vec<Option<f64>> = vec![None; panels.len()];
    for i in 0..panels.len() {
        let Some(sub_i) = sub_scene_of(&panels[i]) else { continue };
        let by_i = panels[i].bounds.y;
        let peers: Vec<usize> =
            (0..panels.len()).filter(|&j| sub_scene_of(&panels[j]).is_some() && (panels[j].bounds.y - by_i).abs() <= 16.0).collect();
        if peers.len() >= 2 {
            let max_origin_y = peers
                .iter()
                .filter_map(|&j| sub_scene_of(&panels[j]).map(|s| panels[j].bounds.y + panels[j].scale * s.plot_rect_pt.y))
                .fold(by_i + panels[i].scale * sub_i.plot_rect_pt.y, f64::max);
            row_targets[i] = Some(max_origin_y);
            let avail_bot = (panels[i].bounds.bottom() - max_origin_y).max(8.0);
            let span_bot = (sub_i.height_pt - sub_i.plot_rect_pt.y).max(1.0);
            panels[i].scale = panels[i].scale.min(avail_bot / span_bot).max(0.05);
        }
    }

    for (i, panel) in panels.iter_mut().enumerate() {
        let Some(sub) = sub_scene_of(panel) else { continue };
        if let Some(target_x) = col_targets.get(i).copied().flatten() {
            panel.tx = target_x - panel.scale * sub.plot_rect_pt.x;
        }
        if let Some(target_y) = row_targets.get(i).copied().flatten() {
            panel.ty = target_y - panel.scale * sub.plot_rect_pt.y;
        }
    }
}

fn emit_placed_panel(scene: &mut ChartScene, panel: &PlacedPanel, sub_scene: Option<&ChartScene>) -> Result<(), ChartError> {
    let badge_prims = usize::from(panel.badge.is_some());
    let content_prims = sub_scene.map_or(1, |sub| usize::from(sub.background.is_some()).saturating_add(sub.primitives.len()));
    let added_prims = 2_usize.saturating_add(content_prims).saturating_add(badge_prims);
    check_layout_primitive_budget(scene.primitives.len(), added_prims, "layout panel")?;

    if let Some(sub) = sub_scene {
        let next_points = scene.plotted_points.len().saturating_add(sub.plotted_points.len());
        if next_points > MAX_LAYOUT_PRIMITIVES {
            return Err(ChartError::ResourceLimit {
                message: format!("layout plotted point metadata count {next_points} exceeds layout budget {MAX_LAYOUT_PRIMITIVES}"),
            });
        }
    }

    let bounds = panel.bounds;
    scene.primitives.push(ScenePrimitive::PushClip { rect: bounds });

    if let Some(sub) = sub_scene {
        if let Some(bg) = sub.background {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(panel.tx, panel.ty, sub.width_pt * panel.scale, sub.height_pt * panel.scale)),
                fill: Some(FillPaint::solid(bg)),
                stroke: None,
                clip: None,
            });
        }
        for prim in &sub.primitives {
            scene.primitives.push(transform_primitive(prim, panel.tx, panel.ty, panel.scale));
        }
        for p in &sub.plotted_points {
            scene.plotted_points.push(PlottedPointMeta {
                center_pt: transform_point(p.center_pt, panel.tx, panel.ty, panel.scale),
                radius_pt: (p.radius_pt * panel.scale).max(2.0),
                source_cell: p.source_cell.clone(),
                series_label: p.series_label.clone(),
                value_summary: p.value_summary.clone(),
            });
        }
        for w in &sub.warnings {
            scene.warnings.push(ChartWarning::new(w.code.clone(), format!("panel {}: {}", panel.panel_idx + 1, w.message)));
        }
    } else {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(bounds),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(180, 180, 180), 0.75, LineDashStyle::Solid)),
            clip: None,
        });
    }

    scene.primitives.push(ScenePrimitive::PopClip);

    if let Some(label) = &panel.badge {
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&format!("<b>{label}</b>")),
            anchor_pt: PointPt::new(bounds.x + 4.0, bounds.y + 12.0),
            anchor: TextAnchor::Start,
            font_size_pt: 11.5,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
    Ok(())
}

fn transform_point(p: PointPt, tx: f64, ty: f64, s: f64) -> PointPt {
    PointPt::new(tx + p.x * s, ty + p.y * s)
}

fn transform_rect(r: RectPt, tx: f64, ty: f64, s: f64) -> RectPt {
    RectPt::new(tx + r.x * s, ty + r.y * s, r.width * s, r.height * s)
}

fn transform_primitive(prim: &ScenePrimitive, tx: f64, ty: f64, s: f64) -> ScenePrimitive {
    match prim {
        ScenePrimitive::Path { path, fill, stroke, clip } => {
            let cmds = path
                .commands
                .iter()
                .map(|c| match *c {
                    PathCmd::MoveTo(p) => PathCmd::MoveTo(transform_point(p, tx, ty, s)),
                    PathCmd::LineTo(p) => PathCmd::LineTo(transform_point(p, tx, ty, s)),
                    PathCmd::QuadTo { control, to } => {
                        PathCmd::QuadTo { control: transform_point(control, tx, ty, s), to: transform_point(to, tx, ty, s) }
                    }
                    PathCmd::CubicTo { c1, c2, to } => PathCmd::CubicTo {
                        c1: transform_point(c1, tx, ty, s),
                        c2: transform_point(c2, tx, ty, s),
                        to: transform_point(to, tx, ty, s),
                    },
                    PathCmd::Close => PathCmd::Close,
                })
                .collect();
            let mut scaled_stroke = *stroke;
            if let Some(st) = &mut scaled_stroke {
                st.width_pt = (st.width_pt * s).max(0.25);
            }
            ScenePrimitive::Path {
                path: Path2D { commands: cmds },
                fill: *fill,
                stroke: scaled_stroke,
                clip: clip.map(|c| transform_rect(c, tx, ty, s)),
            }
        }
        ScenePrimitive::Text(t) => ScenePrimitive::Text(PositionedText {
            rich: t.rich.clone(),
            anchor_pt: transform_point(t.anchor_pt, tx, ty, s),
            anchor: t.anchor,
            font_size_pt: (t.font_size_pt * s).max(4.0),
            rotation_deg: t.rotation_deg,
            color: t.color,
        }),
        ScenePrimitive::RasterImage(img) => ScenePrimitive::RasterImage(crate::scene::SceneRasterImage {
            asset_id: img.asset_id.clone(),
            media_type: img.media_type,
            pixel_size: img.pixel_size,
            rgba: img.rgba.clone(),
            bounds_pt: transform_rect(img.bounds_pt, tx, ty, s),
            crop: curvebook_doc::CropInsetsPt {
                left_pt: img.crop.left_pt * s,
                right_pt: img.crop.right_pt * s,
                top_pt: img.crop.top_pt * s,
                bottom_pt: img.crop.bottom_pt * s,
            },
            rotation_deg: img.rotation_deg,
            clip: img.clip.map(|c| transform_rect(c, tx, ty, s)),
        }),
        ScenePrimitive::PushClip { rect } => ScenePrimitive::PushClip { rect: transform_rect(*rect, tx, ty, s) },
        ScenePrimitive::PopClip => ScenePrimitive::PopClip,
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AxisSpec, ColorChannelMapping, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, LayoutItem, LayoutSheet, LayoutSpec,
        MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, ProjectDocument, RectPt as DocRectPt, SheetId, SizeChannelMapping,
        SubcolumnMode, TableArchetype, VariableRole, XyTable,
    };
    use curvebook_palette::{ColormapId, PaletteId, Rgba8};
    use curvebook_text::SpanWeight;

    use super::{compile_layout_scene, compile_multivariate_or_diagnostic_graph};
    use crate::scene::{ChartScene, ScenePrimitive};

    #[test]
    fn bubble_scatter_compiles_hulls_ellipses_and_colorbar() {
        let mut mv = MultipleVariablesTable::new(4, 6).expect("mv");
        mv.variables[0].title = String::from("X");
        mv.variables[1].title = String::from("Y");
        mv.variables[2].title = String::from("Group");
        mv.variables[2].lock_role(VariableRole::Categorical);
        mv.variables[3].title = String::from("Mass");
        for (r, (x, y, g, sz)) in
            [(1.0, 2.0, "A", 4.0), (2.0, 4.0, "A", 16.0), (3.0, 3.0, "A", 9.0), (6.0, 7.0, "B", 4.0), (7.0, 9.0, "B", 9.0), (8.0, 8.0, "B", 16.0)]
                .into_iter()
                .enumerate()
        {
            mv.variables[0].cells[r] = curvebook_doc::Cell::number(x);
            mv.variables[1].cells[r] = curvebook_doc::Cell::number(y);
            mv.variables[2].cells[r] = curvebook_doc::Cell::text(g);
            mv.variables[3].cells[r] = curvebook_doc::Cell::number(sz);
        }
        let ds = DataSheet::new(SheetId::new("mv-1"), "Bubble", TableArchetype::MultipleVariables(mv)).expect("ds");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-mv"), &ds).expect("gs");
        gs.spec.multivariate = Some(MultivariateGraphEncoding {
            plot_mode: MultivariatePlotMode::BubbleScatter,
            x_column: 0,
            y_column: 1,
            color: ColorChannelMapping::Categorical { column: 2, palette: PaletteId::OkabeIto, level_order_override: None },
            size: SizeChannelMapping::Variable { column: 3, min_radius_pt: 3.0, max_radius_pt: 12.0 },
            label_column: Some(2),
            x_level_order_override: None,
            show_convex_hulls: true,
            show_confidence_ellipses: true,
        });
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add");
        let mut scene = ChartScene::new("g-mv", "Bubble", 420.0, 300.0, None);
        compile_multivariate_or_diagnostic_graph(&gs, &doc, &mut scene).expect("compile");
        assert!(scene.primitives.len() > 20);
        assert!(
            scene
                .primitives
                .iter()
                .any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(st), .. } if st.dash == curvebook_palette::LineDashStyle::Dashed))
        );
    }

    #[test]
    fn categorical_summary_renders_cld_letters_above_error_bars() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, CompactLetterDisplaySpec, ErrorBarSpec, GraphicalDataObject, OneWayAnovaSpec, PairwiseComparisonRecord,
            PairwiseDisplayMode, ResultSheet, ResultStatus, SummaryMeasure,
        };

        let mut mv = MultipleVariablesTable::new(2, 6).expect("mv");
        mv.variables[0].title = String::from("Group");
        mv.variables[0].lock_role(VariableRole::Categorical);
        mv.variables[1].title = String::from("Response");
        for (r, (g, y)) in [("Ctrl", 10.0), ("Ctrl", 12.0), ("Drug", 24.0), ("Drug", 26.0), ("Ref", 18.0), ("Ref", 20.0)].into_iter().enumerate() {
            mv.variables[0].cells[r] = curvebook_doc::Cell::text(g);
            mv.variables[1].cells[r] = curvebook_doc::Cell::number(y);
        }
        let ds = DataSheet::new(SheetId::new("mv-cat"), "Cat Summary", TableArchetype::MultipleVariables(mv)).expect("ds");

        let spec =
            AnalysisSpec::new(SheetId::new("res-cat"), SheetId::new("mv-cat"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("spec");
        let mut res = ResultSheet::new(spec).expect("res");
        res.status = ResultStatus::UpToDate;
        res.pairwise_comparisons = vec![
            PairwiseComparisonRecord {
                group_a: 0,
                group_b: 1,
                label_a: String::from("Ctrl"),
                label_b: String::from("Drug"),
                estimate_diff: Some(-14.0),
                raw_p_or_q: 0.002,
                is_fdr_q: false,
                significant: true,
            },
            PairwiseComparisonRecord {
                group_a: 0,
                group_b: 2,
                label_a: String::from("Ctrl"),
                label_b: String::from("Ref"),
                estimate_diff: Some(-8.0),
                raw_p_or_q: 0.14,
                is_fdr_q: false,
                significant: false,
            },
            PairwiseComparisonRecord {
                group_a: 1,
                group_b: 2,
                label_a: String::from("Drug"),
                label_b: String::from("Ref"),
                estimate_diff: Some(6.0),
                raw_p_or_q: 0.16,
                is_fdr_q: false,
                significant: false,
            },
        ];

        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-cat"), &ds).expect("gs");
        gs.spec.title = String::from("Categorical CLD");
        gs.spec.pairwise_display_mode = PairwiseDisplayMode::CompactLetterDisplayOnly;
        gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res-cat")));
        gs.spec.multivariate = Some(MultivariateGraphEncoding {
            plot_mode: MultivariatePlotMode::CategoricalSummary {
                gdo: GraphicalDataObject::BarWithReplicates {
                    summary: SummaryMeasure::Mean,
                    fill_color: Rgba8::rgb(0, 114, 178),
                    border_color: Rgba8::BLACK,
                    border_width_pt: 0.8,
                    replicate_overlay: None,
                },
                y_error_bars: Some(ErrorBarSpec::default()),
            },
            x_column: 0,
            y_column: 1,
            color: ColorChannelMapping::Constant(Rgba8::rgb(0, 114, 178)),
            size: SizeChannelMapping::Constant { radius_pt: 4.0 },
            label_column: None,
            x_level_order_override: None,
            show_convex_hulls: false,
            show_confidence_ellipses: false,
        });

        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add ds");
        doc.add_result_sheet(res).expect("add res");
        let mut scene = ChartScene::new("g-cat", "Categorical CLD", 360.0, 240.0, None);
        compile_multivariate_or_diagnostic_graph(&gs, &doc, &mut scene).expect("compile");
        assert!(scene.warnings.is_empty(), "warnings: {:?}", scene.warnings);
        let texts: Vec<String> = scene
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&String::from("a")), "missing 'a' in {texts:?}");
        assert!(texts.contains(&String::from("ab")), "missing 'ab' in {texts:?}");
        assert!(texts.contains(&String::from("b")), "missing 'b' in {texts:?}");
    }

    #[test]
    fn layout_align_plot_origins_matches_y_axis_x_across_stacked_panels() {
        let mut xy_small = XyTable::new(SubcolumnMode::Single, 1, 3).expect("xy1");
        let mut xy_big = XyTable::new(SubcolumnMode::Single, 1, 3).expect("xy2");
        for r in 0..3 {
            xy_small.x_cells[r][0] = curvebook_doc::Cell::number(r as f64);
            xy_small.data_sets[0].cells[r][0] = curvebook_doc::Cell::number((r as f64) * 1.0);
            xy_big.x_cells[r][0] = curvebook_doc::Cell::number(r as f64);
            xy_big.data_sets[0].cells[r][0] = curvebook_doc::Cell::number((r as f64) * 1_000_000.0);
        }
        let d1 = DataSheet::new(SheetId::new("d1"), "Small", TableArchetype::Xy(xy_small)).expect("d1");
        let d2 = DataSheet::new(SheetId::new("d2"), "Big", TableArchetype::Xy(xy_big)).expect("d2");
        let mut g1 = GraphSheet::new_uninitialized_for_data(SheetId::new("g1"), &d1).expect("g1");
        let mut g2 = GraphSheet::new_uninitialized_for_data(SheetId::new("g2"), &d2).expect("g2");
        g1.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
        g2.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot);
        g1.spec.y_axis = AxisSpec::linear("Y");
        g2.spec.y_axis = AxisSpec::linear("Y");

        let layout_spec = LayoutSpec {
            align_plot_origins: true,
            items: vec![
                LayoutItem::LiveGraph {
                    id: String::from("p1"),
                    graph_sheet_id: SheetId::new("g1"),
                    bounds_pt: DocRectPt::new(20.0, 15.0, 220.0, 150.0),
                    scale_percent: 100.0,
                    badge_override: None,
                },
                LayoutItem::LiveGraph {
                    id: String::from("p2"),
                    graph_sheet_id: SheetId::new("g2"),
                    bounds_pt: DocRectPt::new(20.0, 180.0, 220.0, 150.0),
                    scale_percent: 100.0,
                    badge_override: None,
                },
            ],
            ..LayoutSpec::default()
        };
        let layout = LayoutSheet::new(SheetId::new("lay-1"), "Aligned", layout_spec).expect("layout");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(d1).expect("add");
        doc.add_data_sheet(d2).expect("add");
        doc.add_graph_sheet(g1).expect("add");
        doc.add_graph_sheet(g2).expect("add");
        doc.add_layout_sheet(layout.clone()).expect("add");

        let compiled = compile_layout_scene(&layout, &doc).expect("compile layout");
        let has_bold_a = compiled
            .primitives
            .iter()
            .any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "A" && t.rich.spans.iter().any(|s| s.weight == SpanWeight::Bold)));
        let has_bold_b = compiled
            .primitives
            .iter()
            .any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "B" && t.rich.spans.iter().any(|s| s.weight == SpanWeight::Bold)));
        assert!(has_bold_a && has_bold_b);

        // Ensure ColormapId & Rgba8 used in test are exercised
        assert_ne!(ColormapId::Cividis.sample(0.0), Rgba8::WHITE);
    }

    #[test]
    fn dendrogram_node_conversion_guards_extreme_and_malformed_ids() {
        use curvebook_doc::{ResultCell, ResultTab, ResultTabKind};
        use curvebook_stats::multivariate::DendrogramMerge;

        use super::{dendrogram_leaf_order, draw_left_dendrogram, extract_dendro_merges_from_tab};
        use crate::scene::{PathCmd, RectPt};

        for bad_row in [
            vec![
                ResultCell::Number { value: -1e300, prefix: None, units: None },
                ResultCell::Number { value: 1e300, prefix: None, units: None },
                ResultCell::Number { value: 0.5, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
            ],
            vec![
                ResultCell::Number { value: -1.5, prefix: None, units: None },
                ResultCell::Number { value: -2.0, prefix: None, units: None },
                ResultCell::Number { value: 0.25, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
            ],
            vec![
                ResultCell::Number { value: isize::MIN as f64, prefix: None, units: None },
                ResultCell::Number { value: -1.0, prefix: None, units: None },
                ResultCell::Number { value: f64::NAN, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
            ],
        ] {
            let tab = ResultTab {
                id: String::from("hierarchical-dendrogram"),
                title: String::from("Dendrogram"),
                kind: ResultTabKind::MainSummary,
                column_headers: vec![],
                row_labels: vec![],
                rows: vec![bad_row],
                footnotes: vec![],
            };
            assert!(extract_dendro_merges_from_tab(&tab, 2).is_empty());
        }

        let malformed = [
            DendrogramMerge { left: isize::MIN, right: isize::MAX, distance: f64::NAN, cluster_size: usize::MAX },
            DendrogramMerge { left: 0, right: 1, distance: f64::INFINITY, cluster_size: 2 },
            DendrogramMerge { left: -999, right: 0, distance: -4.0, cluster_size: 3 },
        ];
        let order = dendrogram_leaf_order(&malformed, 4);
        assert_eq!(order.len(), 4);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3]);

        let mut scene = ChartScene::new("hm-bad", "Bad Dendro", 320.0, 240.0, None);
        draw_left_dendrogram(&mut scene, RectPt::new(20.0, 20.0, 40.0, 120.0), &malformed, &order, 30.0);
        assert!(!scene.primitives.is_empty());
        for prim in &scene.primitives {
            if let ScenePrimitive::Path { path, .. } = prim {
                for cmd in &path.commands {
                    if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = cmd {
                        assert!(pt.x.is_finite() && pt.y.is_finite(), "non-finite dendrogram pt: {pt:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn percentile_linear_preserves_empty_default_exact_rank_and_extreme_span() {
        use super::percentile_linear;

        assert_eq!(percentile_linear(&[], 0.5), 0.0);
        assert_eq!(percentile_linear(&[7.5], 0.5), 7.5);
        assert_eq!(percentile_linear(&[10.0, 20.0, 30.0, 40.0], 0.25), 17.5);
        assert_eq!(percentile_linear(&[10.0, 20.0, 30.0, 40.0], 0.5), 25.0);
        assert_eq!(percentile_linear(&[10.0, 20.0, 30.0, 40.0], f64::NAN), 10.0);
        assert_eq!(percentile_linear(&[10.0, 20.0, 30.0, 40.0], f64::NEG_INFINITY), 10.0);
        assert_eq!(percentile_linear(&[10.0, 20.0, 30.0, 40.0], f64::INFINITY), 40.0);

        // Exact-rank endpoints and non-exact midpoint on extreme finite spans must not produce NaN or Inf.
        let extreme = [f64::MIN, f64::MAX];
        assert_eq!(percentile_linear(&extreme, 0.0), f64::MIN);
        assert_eq!(percentile_linear(&extreme, 1.0), f64::MAX);
        assert_eq!(percentile_linear(&extreme, 0.5), 0.0);
    }
}
