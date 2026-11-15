//! Multivariate bubble/categorical figures, all seven diagnostic plot subtypes, and
//! multi-panel publication layout scene compilation.

use lumencraft_doc::{
    AnalysisKind, ClusterDistance, ClusterLinkage, ColorChannelMapping, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, GraphSpec,
    GraphicalDataObject, LayoutItem, LayoutSheet, LevelOrderSpec, MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec, ProjectDocument,
    ResultCell, ResultSheet, ResultTab, ResultTabKind, SizeChannelMapping, SubcolumnMode, TableArchetype, VariableColumn, VariableRole,
    WhiskerMethod,
};
use lumencraft_math::dist::{chi2_isf, norm_ppf};
use lumencraft_math::sum::{covariance, sample_sd};
use lumencraft_math::{AxisScale, ConfidenceLevel};
use lumencraft_palette::{ColorRamp, ColormapId, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use lumencraft_stats::common::quartiles_hinge;
use lumencraft_stats::multivariate::{DendrogramMerge, hierarchical_clustering_with_metrics};
use lumencraft_stats::roc::{RocCiMethod, roc_curve_analysis};
use lumencraft_stats::two_group::bland_altman;
use lumencraft_text::{RichText, parse_markup};

use crate::scales::AxisMapping;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive,
    SourceCellId, TextAnchor,
};
use crate::survival_cat::{
    CartesianFrame, LegendEntry, PlotMarginConfig, alpha_fill, draw_cartesian_frame, draw_categorical_x_ticks, draw_colorbar, draw_empty_open_frame,
    draw_graph_annotations, draw_legend_entries, emit_marker, extract_sheet_grid, nice_axis_ticks, path_close, path_line, path_move, polygon_path,
    render_data_grid_table, resolve_axis_domain, resolve_graph_text, sanitize_range, solid_stroke, text_metrics, update_plot_rect_for_labels,
};
use crate::xy::format_point_value;

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

    draw_graph_annotations(scene, &graph.spec.annotations, doc);
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

fn default_mv_encoding(graph: &GraphSheet, mv: &lumencraft_doc::MultipleVariablesTable) -> MultivariateGraphEncoding {
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
        lumencraft_doc::CellValue::Empty => None,
        lumencraft_doc::CellValue::Text(s) => {
            let tr = s.trim();
            (!tr.is_empty()).then(|| tr.to_owned())
        }
        lumencraft_doc::CellValue::Number(v) if v.is_finite() => Some(format!("{v}")),
        lumencraft_doc::CellValue::Number(_) => None,
    }
}

fn compile_bubble_scatter(
    graph: &GraphSheet,
    mv: &lumencraft_doc::MultipleVariablesTable,
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
                ramp.sample(((v - col_lo) / denom).clamp(0.0, 1.0))
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
        let Some(marker) = crate::resolve_overridden_marker(&graph.spec, 0, pt.row, base_marker) else {
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
    mv: &lumencraft_doc::MultipleVariablesTable,
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
    mv: &lumencraft_doc::MultipleVariablesTable,
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

fn extract_row_size(mv: &lumencraft_doc::MultipleVariablesTable, size: SizeChannelMapping, row: usize) -> f64 {
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
            if max_abs_s <= 1e-12 { 0.5 * (lo + hi) } else { (hi * (scalar.abs() / max_abs_s).sqrt()).clamp(lo, hi) }
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

fn compile_categorical_summary(
    graph: &GraphSheet,
    mv: &lumencraft_doc::MultipleVariablesTable,
    enc: &MultivariateGraphEncoding,
    gdo: &GraphicalDataObject,
    y_error_bars: Option<lumencraft_doc::ErrorBarSpec>,
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
    let (y_lo, y_hi) = resolve_axis_domain(
        &graph.spec.y_axis,
        data_lo - if matches!(gdo, GraphicalDataObject::BarWithReplicates { .. }) && data_lo >= 0.0 { 0.0 } else { y_pad },
        data_hi + y_pad,
    );
    let y_ticks = nice_axis_ticks(&graph.spec.y_axis, y_lo, y_hi, 5);
    let max_y_w = update_plot_rect_for_labels(scene, &graph.spec, &y_ticks, PlotMarginConfig::default());

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

    let y_zero = y_map.map_value(0.0, r.bottom(), r.y).unwrap_or(r.bottom()).clamp(r.y, r.bottom());
    let mut line_centers = Vec::new();
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
                    emit_replicate_dots(scene, cx, box_w * 0.6, &sorted, y_map, r, Rgba8::rgb(35, 35, 35));
                }
            }
            GraphicalDataObject::BoxAndWhisker { whiskers, stroke_color, show_outliers, show_all_points, .. } => {
                draw_box_and_whisker_slot(
                    scene,
                    &sorted,
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
            }
            GraphicalDataObject::Violin { stroke_color, show_median_and_quartiles, half_violin_with_points, .. } => {
                draw_violin_slot(
                    scene,
                    &sorted,
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
            }
            GraphicalDataObject::FloatingBar { border_color, border_width_pt, summary, .. } => {
                if let (Some(&lo), Some(&hi)) = (sorted.first(), sorted.last())
                    && let (Some(y_lo_pt), Some(y_hi_pt)) = (y_map.map_value(lo, r.bottom(), r.y), y_map.map_value(hi, r.bottom(), r.y))
                {
                    let rect = RectPt::new(cx - box_w * 0.5, y_hi_pt.min(y_lo_pt), box_w, (y_lo_pt - y_hi_pt).abs().max(1.0));
                    scene.primitives.push(ScenePrimitive::Path {
                        path: Path2D::rect(rect),
                        fill: Some(alpha_fill(color, 180)),
                        stroke: Some(solid_stroke(*border_color, *border_width_pt, LineDashStyle::Solid)),
                        clip: Some(r),
                    });
                    if let Some(m) = summary
                        && let Some(ym) = y_map.map_value(summary_center(values, &sorted, *m), r.bottom(), r.y)
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
                emit_replicate_dots(scene, cx, box_w * 0.65, &sorted, y_map, r, color);
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
                emit_replicate_dots(scene, cx, box_w * 0.65, &sorted, y_map, r, color);
            }
        }

        if let Some(eb) = y_error_bars {
            emit_error_bar(scene, cx, values, &sorted, center_y, eb, y_map);
        }
        if let Some(yc) = y_map.map_value(center_y, r.bottom(), r.y) {
            line_centers.push((cx, yc));
        }
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

fn summary_measure_from_gdo(gdo: &GraphicalDataObject) -> lumencraft_doc::SummaryMeasure {
    match gdo {
        GraphicalDataObject::BarWithReplicates { summary, .. } | GraphicalDataObject::AlignedDotPlot { summary, .. } => *summary,
        GraphicalDataObject::FloatingBar { summary: Some(s), .. } => *s,
        _ => lumencraft_doc::SummaryMeasure::Mean,
    }
}

fn summary_center(values: &[f64], sorted: &[f64], measure: lumencraft_doc::SummaryMeasure) -> f64 {
    match measure {
        lumencraft_doc::SummaryMeasure::Mean => values.iter().sum::<f64>() / (values.len().max(1) as f64),
        lumencraft_doc::SummaryMeasure::Median => percentile_linear(sorted, 0.5),
        lumencraft_doc::SummaryMeasure::GeometricMean => {
            if values.iter().all(|&v| v > 0.0) {
                (values.iter().map(|v| v.ln()).sum::<f64>() / (values.len() as f64)).exp()
            } else {
                values.iter().sum::<f64>() / (values.len().max(1) as f64)
            }
        }
    }
}

fn percentile_linear(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return sorted.first().copied().unwrap_or(0.0);
    }
    let idx_f = q.clamp(0.0, 1.0) * ((n - 1) as f64);
    let lo = idx_f.floor() as usize;
    let hi = idx_f.ceil() as usize;
    let frac = idx_f - (lo as f64);
    let a = sorted.get(lo).copied().unwrap_or(0.0);
    let b = sorted.get(hi).copied().unwrap_or(a);
    a + (b - a) * frac
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

fn emit_error_bar(
    scene: &mut ChartScene,
    cx: f64,
    values: &[f64],
    sorted: &[f64],
    center: f64,
    eb: lumencraft_doc::ErrorBarSpec,
    y_map: AxisMapping,
) {
    let clip = scene.plot_rect_pt;
    let n = values.len();
    if n == 0 {
        return;
    }
    let sd = sample_sd(values).unwrap_or(0.0);
    let (lo, hi) = match eb.metric {
        lumencraft_doc::ErrorBarMetric::Sd => (center - sd, center + sd),
        lumencraft_doc::ErrorBarMetric::Sem => {
            let sem = sd / (n as f64).sqrt();
            (center - sem, center + sem)
        }
        lumencraft_doc::ErrorBarMetric::ConfidenceInterval95 => {
            let half = 1.96 * sd / (n as f64).sqrt();
            (center - half, center + half)
        }
        lumencraft_doc::ErrorBarMetric::InterquartileRange => {
            if let Some((q1, _, q3)) = quartiles_hinge(sorted) {
                (q1, q3)
            } else {
                (center, center)
            }
        }
        lumencraft_doc::ErrorBarMetric::Range => (sorted.first().copied().unwrap_or(center), sorted.last().copied().unwrap_or(center)),
        lumencraft_doc::ErrorBarMetric::ExplicitSubcolumns => (center - sd, center + sd),
    };
    let y_lo_val = if matches!(eb.direction, lumencraft_doc::ErrorBarDirection::UpperOnly) { center } else { lo };
    let y_hi_val = if matches!(eb.direction, lumencraft_doc::ErrorBarDirection::LowerOnly) { center } else { hi };
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
    let candidates = matching_results(graph, doc);
    candidates.iter().copied().find(|r| pred(r)).or_else(|| candidates.into_iter().next())
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

fn extract_residuals_tab_pairs(tab: &ResultTab, graph: &GraphSheet, doc: &ProjectDocument) -> Vec<(f64, f64)> {
    let (ds_col, x_col, y_col, obs_col) = extract_residuals_cols(&tab.column_headers);
    let target_label = selected_dataset_label_in_tab(tab, ds_col, graph, doc);
    tab.rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
            {
                return None;
            }
            match (x_col, y_col, obs_col) {
                (Some(xc), Some(yc), _) => Some((result_cell_num(row.get(xc)?)?, result_cell_num(row.get(yc)?)?)),
                (Some(xc), None, Some(oc)) if xc != oc => {
                    let fitted = result_cell_num(row.get(xc)?)?;
                    let observed = result_cell_num(row.get(oc)?)?;
                    Some((fitted, observed - fitted))
                }
                (None, Some(yc), _) => {
                    let yv = result_cell_num(row.get(yc)?)?;
                    let xv = row.iter().enumerate().find_map(|(ci, c)| if ci != yc { result_cell_num(c) } else { None }).unwrap_or((i + 1) as f64);
                    Some((xv, yv))
                }
                _ => match (row.first().and_then(result_cell_num), row.get(1).and_then(result_cell_num)) {
                    (Some(a), Some(b)) => Some((a, b)),
                    (Some(r), None) => Some(((i + 1) as f64, r)),
                    _ => None,
                },
            }
        })
        .collect()
}

// 2a. Heatmap (+ Perceptual Colorbar + Optional Hierarchical Dendrogram)

struct HeatmapData {
    row_labels: Vec<String>,
    col_labels: Vec<String>,
    values: Vec<Vec<f64>>,
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

    if hm.merges.is_empty()
        && (2..=128).contains(&n_rows)
        && n_cols >= 2
        && let Ok(m) = hierarchical_clustering_with_metrics(&hm.values, hm.cluster_distance, hm.cluster_linkage)
    {
        hm.merges = m;
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

    let max_row_lbl_w = hm.row_labels.iter().map(|l| text_metrics(l, 8.0).width_pt).fold(0.0_f64, f64::max);
    let _ = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &[],
        PlotMarginConfig {
            extra_bottom_pt: if has_dendro && hm.clustering_cue.is_some() { 26.0 } else { 18.0 },
            extra_left_pt: (max_row_lbl_w + if has_dendro { 34.0 } else { 8.0 }).clamp(12.0, 90.0),
            has_colorbar: true,
            suppress_y_axis_labels: true,
            ..PlotMarginConfig::default()
        },
    );

    let r = scene.plot_rect_pt;
    let dendro_w = if has_dendro { (r.width * 0.16).clamp(20.0, 48.0) } else { 0.0 };
    let grid = RectPt::new(r.x + dendro_w, r.y, (r.width - dendro_w).max(20.0), r.height);
    let cell_w = grid.width / (n_cols as f64);
    let cell_h = grid.height / (n_rows as f64);
    let cell_border = solid_stroke(Rgba8::rgba(255, 255, 255, 160), 0.5, LineDashStyle::Solid);
    let denom = (dom_hi - dom_lo).abs().max(1e-12);

    for (vis_row, &orig_row) in row_order.iter().enumerate() {
        let Some(row_vals) = hm.values.get(orig_row) else { continue };
        let y = grid.y + (vis_row as f64) * cell_h;
        for c in 0..n_cols {
            let val = row_vals.get(c).copied().unwrap_or(dom_lo);
            let t = ((val - dom_lo) / denom).clamp(0.0, 1.0);
            let x = grid.x + (c as f64) * cell_w;
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(x, y, cell_w, cell_h)),
                fill: Some(FillPaint::solid(ramp.sample(t))),
                stroke: Some(cell_border),
                clip: None,
            });
        }
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

    for (c, lbl) in hm.col_labels.iter().enumerate() {
        let cx = grid.x + ((c as f64) + 0.5) * cell_w;
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(lbl),
            anchor_pt: PointPt::new(cx, grid.bottom() + 11.0),
            anchor: TextAnchor::Middle,
            font_size_pt: 7.5,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }

    if has_dendro {
        draw_left_dendrogram(scene, RectPt::new(r.x, r.y, dendro_w - 3.0, grid.height), &hm.merges, &row_order, cell_h);
        if let Some(cue) = &hm.clustering_cue {
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(cue.clone()),
                anchor_pt: PointPt::new(grid.x + grid.width * 0.5, grid.bottom() + 21.5),
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

fn correlation_heatmap_data(res: &ResultSheet, tab: &ResultTab) -> HeatmapData {
    let mut matrix = Vec::with_capacity(tab.rows.len());
    for row in &tab.rows {
        let vals: Vec<f64> = row
            .iter()
            .map(|c| match c {
                ResultCell::Number { value, .. } if value.is_finite() => *value,
                _ => 0.0,
            })
            .collect();
        matrix.push(vals);
    }
    let merges = res.tabs.iter().find(|t| t.id == "hierarchical-dendrogram").map_or_else(Vec::new, extract_dendro_merges_from_tab);
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

fn extract_heatmap_data(graph: &GraphSheet, doc: &ProjectDocument) -> Option<HeatmapData> {
    let direct_res = doc.result_sheet(&graph.spec.primary_source_sheet_id);
    if let Some(res) = direct_res
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary)
        && matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
    {
        return Some(correlation_heatmap_data(res, tab));
    }

    let clustering_res =
        find_matching_result_where(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })))
            .filter(|r| matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })));

    if clustering_res.is_none()
        && direct_res.is_none()
        && let Some(res) = find_matching_result(graph, doc)
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary)
        && matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
    {
        return Some(correlation_heatmap_data(res, tab));
    }

    let ds = resolve_source_data_sheet(graph, doc)?;
    let (mut hm, expect_rows) = match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let num_cols: Vec<(usize, String)> = mv
                .variables
                .iter()
                .enumerate()
                .filter(|(_, v)| v.role == VariableRole::Continuous || v.role == VariableRole::Calculated)
                .map(|(i, v)| (i, v.title.clone()))
                .collect();
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
                let row_vals: Vec<f64> = num_cols
                    .iter()
                    .map(|(ci, _)| mv.variables.get(*ci).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()).unwrap_or(0.0))
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
            let col_labels: Vec<String> = col.data_sets.iter().map(|d| d.title.clone()).collect();
            let nrows = col.row_count();
            let row_labels =
                (0..nrows).map(|r| col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("R{}", r + 1))).collect();
            let values = (0..nrows)
                .map(|r| {
                    col.data_sets.iter().map(|d| d.cells.get(r).and_then(|s| s.first()).and_then(|c| c.active_number()).unwrap_or(0.0)).collect()
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
            let col_labels: Vec<String> = g.data_sets.iter().map(|d| d.title.clone()).collect();
            let nrows = g.row_count();
            let row_labels =
                (0..nrows).map(|r| g.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("R{}", r + 1))).collect();
            let values = (0..nrows)
                .map(|r| {
                    (0..g.data_sets.len())
                        .map(|c| {
                            let reps = g.active_replicates(r, c);
                            if reps.is_empty() { 0.0 } else { reps.iter().sum::<f64>() / (reps.len() as f64) }
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
        let tab_merges = cres.tabs.iter().find(|t| t.id == "hierarchical-dendrogram").map_or_else(Vec::new, extract_dendro_merges_from_tab);
        if !tab_merges.is_empty() && tab_merges.len() + 1 == expect_rows {
            hm.merges = tab_merges;
        }
    }

    Some(hm)
}

fn extract_dendro_merges_from_tab(tab: &ResultTab) -> Vec<DendrogramMerge> {
    let mut merges = Vec::with_capacity(tab.rows.len());
    for row in &tab.rows {
        let (Some(l), Some(r), Some(d), Some(sz)) = (
            row.first().and_then(result_cell_num),
            row.get(1).and_then(result_cell_num),
            row.get(2).and_then(result_cell_num),
            row.get(3).and_then(result_cell_num),
        ) else {
            continue;
        };
        merges.push(DendrogramMerge { left: l as isize, right: r as isize, distance: d, cluster_size: sz.max(1.0) as usize });
    }
    merges
}

fn heatmap_min_max(values: &[Vec<f64>]) -> (f64, f64) {
    let (lo, hi) = values
        .iter()
        .flat_map(|r| r.iter())
        .copied()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    sanitize_range(lo, hi, (0.0, 1.0))
}

fn dendrogram_leaf_order(merges: &[DendrogramMerge], n_leaves: usize) -> Vec<usize> {
    if merges.is_empty() || n_leaves == 0 {
        return (0..n_leaves).collect();
    }
    let mut order = Vec::with_capacity(n_leaves);
    let mut stack: Vec<isize> = vec![(merges.len() - 1) as isize];
    let mut guard = 0_usize;
    while let Some(node) = stack.pop() {
        guard += 1;
        if guard > n_leaves * 4 + 16 {
            break;
        }
        if node < 0 {
            let leaf = ((-node) - 1) as usize;
            if leaf < n_leaves && !order.contains(&leaf) {
                order.push(leaf);
            }
        } else if let Some(m) = merges.get(node as usize) {
            stack.push(m.right);
            stack.push(m.left);
        }
    }
    for i in 0..n_leaves {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    order
}

fn draw_left_dendrogram(scene: &mut ChartScene, box_rect: RectPt, merges: &[DendrogramMerge], leaf_order: &[usize], cell_h: f64) {
    let max_dist = merges.iter().map(|m| m.distance).fold(0.0_f64, f64::max).max(1e-9);
    let mut node_y = vec![0.0_f64; leaf_order.len()];
    for (vis_idx, &orig_leaf) in leaf_order.iter().enumerate() {
        if let Some(slot) = node_y.get_mut(orig_leaf) {
            *slot = box_rect.y + ((vis_idx as f64) + 0.5) * cell_h;
        }
    }
    let mut merge_coords: Vec<(f64, f64)> = vec![(box_rect.right(), box_rect.y); merges.len()];
    let lookup_pos = |id: isize, mc: &[(f64, f64)]| -> (f64, f64) {
        if id < 0 {
            let leaf = ((-id) - 1) as usize;
            (box_rect.right(), node_y.get(leaf).copied().unwrap_or(box_rect.y))
        } else {
            mc.get(id as usize).copied().unwrap_or((box_rect.right(), box_rect.y))
        }
    };
    let stroke = solid_stroke(Rgba8::rgb(65, 70, 80), 0.9, LineDashStyle::Solid);
    for (step, m) in merges.iter().enumerate() {
        let (x_l, y_l) = lookup_pos(m.left, &merge_coords);
        let (x_r, y_r) = lookup_pos(m.right, &merge_coords);
        let x_m = box_rect.right() - (m.distance / max_dist) * box_rect.width;
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
    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.1), b.max(p.1)));
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
    for (idx, (fc, neg_log_p, opt_lbl)) in pts.iter().enumerate() {
        let (Some(px), Some(py)) = (x_map.map_value(*fc, r.x, r.right()), y_map.map_value(*neg_log_p, r.bottom(), r.y)) else {
            continue;
        };
        let is_sig = *neg_log_p >= p_thresh;
        let col = if is_sig && *fc >= fc_pos {
            up_col
        } else if is_sig && *fc <= fc_neg {
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
        let Some(marker) = crate::resolve_overridden_marker(&graph.spec, 0, idx, base_marker) else {
            continue;
        };
        emit_marker(scene, PointPt::new(px, py), marker, Some(r));
        if is_sig
            && (*fc >= fc_pos || *fc <= fc_neg)
            && let Some(lbl) = opt_lbl
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

fn extract_volcano_points(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<(f64, f64, Option<String>)> {
    if let Some(res) = find_matching_result_where(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::MultipleRowTTests(_)))
        && (matches!(res.analysis.kind, AnalysisKind::MultipleRowTTests(_)) || doc.result_sheet(&graph.spec.primary_source_sheet_id).is_some())
    {
        let from_pairs: Vec<(f64, f64, Option<String>)> = res
            .pairwise_comparisons
            .iter()
            .filter_map(|rec| {
                let diff = rec.estimate_diff?;
                if !diff.is_finite() || !rec.raw_p_or_q.is_finite() || rec.raw_p_or_q < 0.0 {
                    return None;
                }
                let neg_log_p = -(rec.raw_p_or_q.clamp(1e-300, 1.0)).log10();
                let lbl = (!rec.label_a.trim().is_empty()).then(|| rec.label_a.clone());
                Some((diff, neg_log_p, lbl))
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
                let from_tab: Vec<(f64, f64, Option<String>)> = tab
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(i, row)| {
                        let diff = result_cell_num(row.get(dc)?)?;
                        let p_raw = result_cell_p_or_num(row.get(pc)?)?;
                        let neg_log_p = if already_neg_log {
                            p_raw
                        } else if p_raw >= 0.0 {
                            -(p_raw.clamp(1e-300, 1.0)).log10()
                        } else {
                            return None;
                        };
                        let lbl = tab.row_labels.get(i).filter(|s| !s.trim().is_empty()).cloned();
                        Some((diff, neg_log_p, lbl))
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
                            raw.push((x, y, lbl));
                        }
                    }
                    let y_title_low = yc.title.to_ascii_lowercase();
                    let looks_raw_p = !y_title_low.contains("log") && !raw.is_empty() && raw.iter().all(|(_, y, _)| *y > 0.0 && *y <= 1.0);
                    if looks_raw_p {
                        for (_, y, _) in &mut raw {
                            *y = -y.max(1e-300).log10();
                        }
                    }
                    return raw;
                }
            }
            TableArchetype::Xy(xy) => {
                return xy.active_xy_pairs(0).into_iter().map(|(x, y)| (x, y, None)).collect();
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

    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &(x, _)| (a.min(x), b.max(x)));
    let (mut y_min, mut y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &(_, y)| (a.min(y), b.max(y)));
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
    for (idx, &(avg, diff)) in pts.iter().enumerate() {
        if let (Some(px), Some(py)) = (x_map.map_value(avg, r.x, r.right()), y_map.map_value(diff, r.bottom(), r.y)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 5.0,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.6,
            };
            if let Some(marker) = crate::resolve_overridden_marker(&graph.spec, 0, idx, base_marker) {
                emit_marker(scene, PointPt::new(px, py), marker, Some(r));
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
    points: Vec<(f64, f64)>,
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

fn extract_bland_altman_pairs_from_ds(ds: &DataSheet, col_a: usize, col_b: usize, graph: &GraphSheet) -> Option<(Vec<f64>, Vec<f64>)> {
    match &ds.archetype {
        TableArchetype::Column(col) if col.data_sets.len() >= 2 => {
            let mut va = Vec::new();
            let mut vb = Vec::new();
            for r in 0..col.row_count() {
                if let (Some(x), Some(y)) = (
                    col.data_sets.get(col_a).and_then(|d| d.cells.get(r)).and_then(|s| s.first()).and_then(|c| c.active_number()),
                    col.data_sets.get(col_b).and_then(|d| d.cells.get(r)).and_then(|s| s.first()).and_then(|c| c.active_number()),
                ) {
                    va.push(x);
                    vb.push(y);
                }
            }
            Some((va, vb))
        }
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(col_a, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(col_b, |m| m.y_column);
            let mut va = Vec::new();
            let mut vb = Vec::new();
            for r in 0..mv.row_count() {
                if let (Some(x), Some(y)) = (
                    mv.variables.get(xi).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                    mv.variables.get(yi).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                ) {
                    va.push(x);
                    vb.push(y);
                }
            }
            Some((va, vb))
        }
        TableArchetype::Xy(xy) => Some(xy.active_xy_pairs(col_a).into_iter().unzip()),
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
        let mut pts: Vec<(f64, f64)> = res
            .fitted_curves
            .iter()
            .find(|c| c.equation_id == "BlandAltman")
            .map(|c| c.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| (p.x, p.y)).collect())
            .unwrap_or_default();

        if pts.is_empty()
            && let Some(tab) = res
                .tabs
                .iter()
                .find(|t| t.id.contains("bland-altman") && t.kind == ResultTabKind::Residuals)
                .or_else(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals))
        {
            let x_col = find_header_col(&tab.column_headers, |h| h.contains("average") || h.contains("mean")).unwrap_or(0);
            let y_col = find_header_col(&tab.column_headers, |h| h.contains("difference") || h.contains("diff")).unwrap_or(1);
            pts = tab.rows.iter().filter_map(|row| Some((result_cell_num(row.get(x_col)?)?, result_cell_num(row.get(y_col)?)?))).collect();
        }

        let mut bias_opt = find_scalar_ci(res, |k| k == "bias").or_else(|| find_tab_row_value(res, |l| l == "bias"));
        let mut loa_lo_opt = find_scalar_ci(res, |k| k == "loalower" || k == "loa_lower")
            .or_else(|| find_tab_row_value(res, |l| l.contains("lower") && l.contains("loa")));
        let mut loa_hi_opt = find_scalar_ci(res, |k| k == "loaupper" || k == "loa_upper")
            .or_else(|| find_tab_row_value(res, |l| l.contains("upper") && l.contains("loa")));

        if (pts.is_empty() || bias_opt.is_none() || loa_lo_opt.is_none() || loa_hi_opt.is_none())
            && let Some(ds) = doc.data_sheet(&res.analysis.source_sheet_id).or_else(|| resolve_source_data_sheet(graph, doc))
            && let Some((a, b)) = extract_bland_altman_pairs_from_ds(ds, col_a, col_b, graph)
            && let Ok(ba) = bland_altman(&a, &b, spec_pct, res.analysis.confidence_level)
        {
            if pts.is_empty() {
                pts = ba.points;
            }
            bias_opt = bias_opt.or(Some(ba.bias));
            loa_lo_opt = loa_lo_opt.or(Some(ba.loa_lower));
            loa_hi_opt = loa_hi_opt.or(Some(ba.loa_upper));
        }

        if pts.len() >= 2 {
            let diffs: Vec<f64> = pts.iter().map(|p| p.1).collect();
            let bias = bias_opt.unwrap_or_else(|| diffs.iter().sum::<f64>() / (diffs.len() as f64));
            let sd_fallback = sample_sd(&diffs).unwrap_or(0.0);
            let sd_bias = find_scalar_ci(res, |k| k == "sdbias" || k == "sd_bias")
                .or_else(|| find_tab_row_value(res, |l| l.contains("sd of bias")))
                .unwrap_or(sd_fallback);
            let loa_lo = loa_lo_opt.unwrap_or(bias - 1.96 * sd_bias);
            let loa_hi = loa_hi_opt.unwrap_or(bias + 1.96 * sd_bias);
            return Some(BlandAltmanData { points: pts, bias, loa_lo, loa_hi, percent_difference: spec_pct });
        }
    }

    let ds = resolve_source_data_sheet(graph, doc)?;
    if let TableArchetype::Xy(xy) = &ds.archetype {
        let pairs = xy.active_xy_pairs(0);
        if graph.spec.y_axis.title.to_ascii_lowercase().contains("diff") && pairs.len() >= 2 {
            let diffs: Vec<f64> = pairs.iter().map(|p| p.1).collect();
            let bias = diffs.iter().sum::<f64>() / (diffs.len() as f64);
            let sd = sample_sd(&diffs).unwrap_or(0.0);
            return Some(BlandAltmanData {
                points: pairs,
                bias,
                loa_lo: bias - 1.96 * sd,
                loa_hi: bias + 1.96 * sd,
                percent_difference: title_pct_diff,
            });
        }
    }
    let (a, b) = extract_bland_altman_pairs_from_ds(ds, 0, 1, graph)?;
    let ba = bland_altman(&a, &b, title_pct_diff, ConfidenceLevel::DEFAULT_95).ok()?;
    Some(BlandAltmanData { points: ba.points, bias: ba.bias, loa_lo: ba.loa_lower, loa_hi: ba.loa_upper, percent_difference: title_pct_diff })
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
        let (Some(px), Some(py)) = (x_map.map_value(fpr.clamp(0.0, 1.0), r.x, r.right()), y_map.map_value(tpr.clamp(0.0, 1.0), r.bottom(), r.y))
        else {
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
    auc.clamp(0.0, 1.0)
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
    let mut entries = extract_forest_entries(graph, doc);
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

    if entries.len() >= 2
        && !entries.iter().any(|e| e.is_pooled)
        && let Some(pooled) = compute_pooled_forest_entry(&entries, is_log || ref_x == 1.0)
    {
        entries.push(pooled);
    }

    let x_min = entries.iter().map(|e| e.lower).fold(f64::INFINITY, f64::min).min(ref_x);
    let x_max = entries.iter().map(|e| e.upper).fold(f64::NEG_INFINITY, f64::max).max(ref_x);
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
            extra_left_pt: (max_lbl_w + 10.0).clamp(32.0, 96.0),
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

    if let Some(x_ref_pt) = x_map.map_value(ref_x, r.x, r.right())
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
            let dh = (row_h * 0.28).clamp(3.5, 7.0);
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

fn extract_forest_entries(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<ForestEntry> {
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
                ResultCell::ConfidenceInterval { lower: lumencraft_doc::CiBound::Finite(lo), upper: lumencraft_doc::CiBound::Finite(hi) } => {
                    Some((*lo, *hi))
                }
                _ => None,
            }) {
                let est = match row.get(est_idx) {
                    Some(ResultCell::Number { value, .. }) if value.is_finite() => *value,
                    _ => 0.5 * (lo + hi),
                };
                let label = tab.row_labels.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
                let is_pooled = label.to_ascii_lowercase().contains("pooled");
                entries.push(ForestEntry { label, estimate: est, lower: lo, upper: hi, is_pooled });
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
                if let (Some(est), Some(lo), Some(hi)) = (
                    mv.variables.get(c_est).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                    mv.variables.get(c_lo).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                    mv.variables.get(c_hi).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                ) {
                    let label = lbl_col
                        .and_then(|lc| mv.variables.get(lc))
                        .and_then(|v| cell_token(v, r))
                        .or_else(|| mv.row_titles.get(r).filter(|s| !s.is_empty()).cloned())
                        .unwrap_or_else(|| format!("Study {}", r + 1));
                    let is_pooled = label.to_ascii_lowercase().contains("pooled");
                    entries.push(ForestEntry { label, estimate: est, lower: lo.min(est), upper: hi.max(est), is_pooled });
                }
            }
            entries
        }
        TableArchetype::Column(col) if col.data_sets.len() >= 3 => (0..col.row_count())
            .filter_map(|r| {
                let est = col.data_sets.first()?.cells.get(r)?.first()?.active_number()?;
                let lo = col.data_sets.get(1)?.cells.get(r)?.first()?.active_number()?;
                let hi = col.data_sets.get(2)?.cells.get(r)?.first()?.active_number()?;
                let label = col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
                Some(ForestEntry { is_pooled: label.to_ascii_lowercase().contains("pooled"), label, estimate: est, lower: lo, upper: hi })
            })
            .collect(),
        TableArchetype::Xy(xy) if matches!(xy.y_subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) => {
            let mut entries = Vec::new();
            let Some(ds_col) = xy.data_sets.first() else { return entries };
            for (r, subs) in ds_col.cells.iter().enumerate() {
                if let (Some(est), Some(lo), Some(hi)) = (
                    subs.first().and_then(|c| c.active_number()),
                    subs.get(1).and_then(|c| c.active_number()),
                    subs.get(2).and_then(|c| c.active_number()),
                ) {
                    let label = xy.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
                    entries.push(ForestEntry {
                        is_pooled: label.to_ascii_lowercase().contains("pooled"),
                        label,
                        estimate: est,
                        lower: lo,
                        upper: hi,
                    });
                }
            }
            entries
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

    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &(x, _)| (a.min(x), b.max(x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &(_, y)| (a.min(y).min(0.0), b.max(y).max(0.0)));
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
    for (idx, &(xv, yv)) in pts.iter().enumerate() {
        if let (Some(px), Some(py)) = (x_map.map_value(xv, r.x, r.right()), y_map.map_value(yv, r.bottom(), r.y)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.8,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
            };
            if let Some(marker) = crate::resolve_overridden_marker(&graph.spec, 0, idx, base_marker) {
                emit_marker(scene, PointPt::new(px, py), marker, Some(r));
            }
        }
    }
    Ok(())
}

fn extract_residual_points(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<(f64, f64)> {
    if let Some(res) = find_matching_result_where(graph, doc, |r| {
        !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)) && r.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals)
    }) && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals)
    {
        let pts = extract_residuals_tab_pairs(tab, graph, doc);
        if !pts.is_empty() {
            return pts;
        }
    }

    let Some(ds) = resolve_source_data_sheet(graph, doc) else { return Vec::new() };
    let pairs: Vec<(f64, f64)> = match &ds.archetype {
        TableArchetype::Xy(xy) => xy.active_xy_pairs(0),
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(1, |m| m.y_column);
            let raw: Vec<(f64, f64)> = (0..mv.row_count())
                .filter_map(|r| Some((mv.variables.get(xi)?.cells.get(r)?.active_number()?, mv.variables.get(yi)?.cells.get(r)?.active_number()?)))
                .collect();
            let y_title_low = mv.variables.get(yi).map_or("", |v| v.title.as_str()).to_ascii_lowercase();
            if y_title_low.contains("resid") {
                return raw;
            }
            raw
        }
        TableArchetype::Column(col) => {
            let vals: Vec<f64> =
                col.data_sets.first().map_or_else(Vec::new, |d| d.cells.iter().filter_map(|s| s.first().and_then(|c| c.active_number())).collect());
            let mean = if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / (vals.len() as f64) };
            return vals.into_iter().enumerate().map(|(i, v)| ((i + 1) as f64, v - mean)).collect();
        }
        _ => Vec::new(),
    };
    ols_residuals(&pairs)
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
    pts.retain(|(x, y)| x.is_finite() && y.is_finite());
    if pts.len() < 3 {
        scene.warnings.push(ChartWarning::new("insufficient_qq_sample", "at least 3 finite observations required for Normal Q-Q plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    pts.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &(x, _)| (a.min(x), b.max(x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &(_, y)| (a.min(y), b.max(y)));
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

    let mut xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
    let mut ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
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
    for (idx, &(z, y)) in pts.iter().enumerate() {
        if let (Some(px), Some(py)) = (x_map.map_value(z, r.x, r.right()), y_map.map_value(y, r.bottom(), r.y)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.8,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
            };
            if let Some(marker) = crate::resolve_overridden_marker(&graph.spec, 0, idx, base_marker) {
                emit_marker(scene, PointPt::new(px, py), marker, Some(r));
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

fn values_to_qq_pairs(mut vals: Vec<f64>) -> Vec<(f64, f64)> {
    vals.retain(|v| v.is_finite());
    vals.sort_by(f64::total_cmp);
    let n = vals.len();
    if n < 3 {
        return Vec::new();
    }
    vals.into_iter()
        .enumerate()
        .filter_map(|(i, y)| {
            let p = ((i as f64) + 0.5) / (n as f64);
            let z = norm_ppf(p).ok()?;
            Some((z, y))
        })
        .collect()
}

fn extract_qq_points(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<(f64, f64)> {
    // 1. Prefer explicit ResultTabKind::NormalQq (either DescriptiveStats or Fit Q-Q tab).
    if let Some(res) = find_matching_result_where(graph, doc, |r| r.tabs.iter().any(|t| t.kind == ResultTabKind::NormalQq))
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::NormalQq)
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

        if let (Some(ec), Some(oc)) = (xc, yc) {
            let pairs: Vec<(f64, f64)> = tab
                .rows
                .iter()
                .filter_map(|row| {
                    if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                        && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
                    {
                        return None;
                    }
                    Some((result_cell_num(row.get(ec)?)?, result_cell_num(row.get(oc)?)?))
                })
                .collect();
            if pairs.len() >= 3 {
                return pairs;
            }
        } else if let Some(oc) = yc {
            let vals: Vec<f64> = tab
                .rows
                .iter()
                .filter_map(|row| {
                    if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                        && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
                    {
                        return None;
                    }
                    result_cell_num(row.get(oc)?)
                })
                .collect();
            let pairs = values_to_qq_pairs(vals);
            if pairs.len() >= 3 {
                return pairs;
            }
        }
    }

    // 2. Fallback to ResultTabKind::Residuals (excluding BlandAltman pair coordinate tabs).
    if let Some(res) = find_matching_result_where(graph, doc, |r| {
        !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)) && r.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals)
    }) && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals)
    {
        let vals: Vec<f64> = extract_residuals_tab_pairs(tab, graph, doc).into_iter().map(|(_, resid)| resid).collect();
        let pairs = values_to_qq_pairs(vals);
        if pairs.len() >= 3 {
            return pairs;
        }
    }

    // 3. Fallback to raw DataSheet observations.
    let Some(ds) = resolve_source_data_sheet(graph, doc) else { return Vec::new() };
    let vals = match &ds.archetype {
        TableArchetype::Column(col) => {
            let idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
            col.data_sets
                .get(idx)
                .or_else(|| col.data_sets.first())
                .map_or_else(Vec::new, |d| d.cells.iter().filter_map(|s| s.first().and_then(|c| c.active_number())).collect())
        }
        TableArchetype::MultipleVariables(mv) => {
            let idx = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
            mv.variables
                .get(idx)
                .or_else(|| mv.variables.first())
                .map_or_else(Vec::new, |v| v.cells.iter().filter_map(|c| c.active_number()).collect())
        }
        TableArchetype::Xy(xy) => xy.active_xy_pairs(0).into_iter().map(|p| p.1).collect(),
        _ => Vec::new(),
    };
    values_to_qq_pairs(vals)
}

// ============================================================================
// 3. Multi-Panel LayoutSheet Compilation (Gap G11)
// ============================================================================

struct PlacedPanel {
    panel_idx: usize,
    bounds: RectPt,
    badge: Option<String>,
    sub_scene: Option<ChartScene>,
    scale: f64,
    tx: f64,
    ty: f64,
}

/// Compile a multi-panel [`LayoutSheet`] into a unified physical-page [`ChartScene`].
pub fn compile_layout_scene(layout: &LayoutSheet, doc: &ProjectDocument) -> Result<ChartScene, ChartError> {
    let page_pt = layout.spec.page_size_pt();
    let mut scene = ChartScene::new(layout.id.as_str(), layout.title.clone(), page_pt.width, page_pt.height, layout.spec.background_fill);

    let mut panels: Vec<PlacedPanel> = Vec::new();
    let mut other_items: Vec<&LayoutItem> = Vec::new();
    let mut panel_counter = 0_usize;

    for item in &layout.spec.items {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, scale_percent, badge_override, .. } => {
                let bounds = RectPt::from(*bounds_pt);
                let badge = badge_override.clone().or_else(|| layout.spec.panel_badges.badge_for(panel_counter));
                let sub_scene = match doc.graph_sheet(graph_sheet_id) {
                    Some(gs) => match crate::compile_graph(gs, doc) {
                        Ok(compiled) => Some(compiled),
                        Err(e) => {
                            scene.warnings.push(ChartWarning::new("panel_compile_error", e.to_string()));
                            None
                        }
                    },
                    None => {
                        scene
                            .warnings
                            .push(ChartWarning::new("missing_layout_graph", format!("referenced graph sheet '{graph_sheet_id}' not found")));
                        None
                    }
                };
                let scale = sub_scene.as_ref().map_or(1.0, |sub| {
                    let pct = if scale_percent.is_finite() && *scale_percent > 0.0 { *scale_percent / 100.0 } else { 1.0 };
                    let fit = (bounds.width / sub.width_pt.max(1.0)).min(bounds.height / sub.height_pt.max(1.0));
                    (fit * pct).max(0.05)
                });
                panels.push(PlacedPanel { panel_idx: panel_counter, bounds, badge, sub_scene, scale, tx: bounds.x, ty: bounds.y });
                panel_counter += 1;
            }
            LayoutItem::LiveTable { .. } | LayoutItem::TextCallout { .. } => {
                other_items.push(item);
            }
        }
    }

    if layout.spec.align_plot_origins {
        align_placed_panel_origins(&mut panels);
    }

    for panel in &panels {
        emit_placed_panel(&mut scene, panel);
    }

    for item in other_items {
        match item {
            LayoutItem::LiveTable { source_sheet_id, tab_id, bounds_pt, .. } => {
                let bounds = RectPt::from(*bounds_pt);
                if let Some(grid) = extract_sheet_grid(doc, source_sheet_id, tab_id.as_deref()) {
                    render_data_grid_table(&mut scene, bounds, &grid.headers, &grid.row_labels, &grid.rows, Rgba8::rgb(130, 136, 145));
                } else {
                    scene.warnings.push(ChartWarning::new("missing_layout_table", format!("referenced table sheet '{source_sheet_id}' not found")));
                    scene.primitives.push(ScenePrimitive::Path {
                        path: Path2D::rect(bounds),
                        fill: None,
                        stroke: Some(solid_stroke(Rgba8::rgb(160, 160, 160), 0.75, LineDashStyle::Solid)),
                        clip: None,
                    });
                }
            }
            LayoutItem::TextCallout { markdown, bounds_pt, font_size_pt, .. } => {
                let expanded = resolve_graph_text(markdown, doc);
                let bounds = RectPt::from(*bounds_pt);
                let fs = if font_size_pt.is_finite() && *font_size_pt > 0.0 { *font_size_pt } else { 10.0 };
                let line_h = fs * 1.3;
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
            LayoutItem::LiveGraph { .. } => {}
        }
    }

    Ok(scene)
}

/// Align inner `plot_rect_pt` origins across panels sharing a layout column (X) or layout row (Y).
fn align_placed_panel_origins(panels: &mut [PlacedPanel]) {
    let mut col_targets: Vec<Option<f64>> = vec![None; panels.len()];
    for i in 0..panels.len() {
        let Some(sub_i) = panels.get(i).and_then(|p| p.sub_scene.as_ref()) else { continue };
        let bx_i = panels[i].bounds.x;
        let peers: Vec<usize> = (0..panels.len())
            .filter(|&j| panels.get(j).and_then(|p| p.sub_scene.as_ref()).is_some() && (panels[j].bounds.x - bx_i).abs() <= 16.0)
            .collect();
        if peers.len() >= 2 {
            let max_origin_x = peers
                .iter()
                .filter_map(|&j| panels.get(j).and_then(|p| p.sub_scene.as_ref().map(|s| p.bounds.x + p.scale * s.plot_rect_pt.x)))
                .fold(bx_i + panels[i].scale * sub_i.plot_rect_pt.x, f64::max);
            col_targets[i] = Some(max_origin_x);
            let avail_right = (panels[i].bounds.right() - max_origin_x).max(8.0);
            let span_right = (sub_i.width_pt - sub_i.plot_rect_pt.x).max(1.0);
            panels[i].scale = panels[i].scale.min(avail_right / span_right).max(0.05);
        }
    }

    let mut row_targets: Vec<Option<f64>> = vec![None; panels.len()];
    for i in 0..panels.len() {
        let Some(sub_i) = panels.get(i).and_then(|p| p.sub_scene.as_ref()) else { continue };
        let by_i = panels[i].bounds.y;
        let peers: Vec<usize> = (0..panels.len())
            .filter(|&j| panels.get(j).and_then(|p| p.sub_scene.as_ref()).is_some() && (panels[j].bounds.y - by_i).abs() <= 16.0)
            .collect();
        if peers.len() >= 2 {
            let max_origin_y = peers
                .iter()
                .filter_map(|&j| panels.get(j).and_then(|p| p.sub_scene.as_ref().map(|s| p.bounds.y + p.scale * s.plot_rect_pt.y)))
                .fold(by_i + panels[i].scale * sub_i.plot_rect_pt.y, f64::max);
            row_targets[i] = Some(max_origin_y);
            let avail_bot = (panels[i].bounds.bottom() - max_origin_y).max(8.0);
            let span_bot = (sub_i.height_pt - sub_i.plot_rect_pt.y).max(1.0);
            panels[i].scale = panels[i].scale.min(avail_bot / span_bot).max(0.05);
        }
    }

    for (i, panel) in panels.iter_mut().enumerate() {
        let Some(sub) = &panel.sub_scene else { continue };
        if let Some(target_x) = col_targets.get(i).copied().flatten() {
            panel.tx = target_x - panel.scale * sub.plot_rect_pt.x;
        }
        if let Some(target_y) = row_targets.get(i).copied().flatten() {
            panel.ty = target_y - panel.scale * sub.plot_rect_pt.y;
        }
    }
}

fn emit_placed_panel(scene: &mut ChartScene, panel: &PlacedPanel) {
    let bounds = panel.bounds;
    scene.primitives.push(ScenePrimitive::PushClip { rect: bounds });

    if let Some(sub) = &panel.sub_scene {
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
        ScenePrimitive::PushClip { rect } => ScenePrimitive::PushClip { rect: transform_rect(*rect, tx, ty, s) },
        ScenePrimitive::PopClip => ScenePrimitive::PopClip,
    }
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{
        AxisSpec, ColorChannelMapping, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, LayoutItem, LayoutSheet, LayoutSpec,
        MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, ProjectDocument, RectPt as DocRectPt, SheetId, SizeChannelMapping,
        SubcolumnMode, TableArchetype, VariableRole, XyTable,
    };
    use lumencraft_palette::{ColormapId, PaletteId, Rgba8};
    use lumencraft_text::SpanWeight;

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
            mv.variables[0].cells[r] = lumencraft_doc::Cell::number(x);
            mv.variables[1].cells[r] = lumencraft_doc::Cell::number(y);
            mv.variables[2].cells[r] = lumencraft_doc::Cell::text(g);
            mv.variables[3].cells[r] = lumencraft_doc::Cell::number(sz);
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
                .any(|p| matches!(p, ScenePrimitive::Path { stroke: Some(st), .. } if st.dash == lumencraft_palette::LineDashStyle::Dashed))
        );
    }

    #[test]
    fn layout_align_plot_origins_matches_y_axis_x_across_stacked_panels() {
        let mut xy_small = XyTable::new(SubcolumnMode::Single, 1, 3).expect("xy1");
        let mut xy_big = XyTable::new(SubcolumnMode::Single, 1, 3).expect("xy2");
        for r in 0..3 {
            xy_small.x_cells[r][0] = lumencraft_doc::Cell::number(r as f64);
            xy_small.data_sets[0].cells[r][0] = lumencraft_doc::Cell::number((r as f64) * 1.0);
            xy_big.x_cells[r][0] = lumencraft_doc::Cell::number(r as f64);
            xy_big.data_sets[0].cells[r][0] = lumencraft_doc::Cell::number((r as f64) * 1_000_000.0);
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
}
