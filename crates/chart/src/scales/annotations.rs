//! Figure legends, canvas `GraphAnnotation` primitive emission, and warning summaries.

use std::fmt::Write as FmtWrite;

use curvebook_doc::{AnnotationShapeKind, GraphAnnotation, GraphSpec, LegendSpec, ProjectDocument, SheetId, TableArchetype};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_text::{measure_text_pt, parse_markup};

use super::ticks::{resolve_paired_info_id, resolve_template_hooks_with_info};
use crate::markers::ellipse_path;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive,
    StrokePaint, TextAnchor,
};

// ============================================================================
// 4. Legends & Canvas GraphAnnotations
// ============================================================================

/// Visual swatch + label entry rendered inside a figure legend.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendEntry {
    pub label: String,
    pub swatch_color: Rgba8,
    pub stroke_color: Option<Rgba8>,
    pub marker_shape: Option<MarkerShape>,
    pub marker_fill_mode: MarkerFillMode,
    pub dash_style: Option<LineDashStyle>,
    pub is_box_swatch: bool,
}

/// Extract default [`LegendEntry`] rows from [`GraphSpec`] series bindings and dataset titles.
#[must_use]
pub fn legend_entries_from_graph_spec(graph: &GraphSpec, doc: &ProjectDocument) -> Vec<LegendEntry> {
    let mut entries = Vec::with_capacity(graph.series.len());
    for (idx, series) in graph.series.iter().enumerate() {
        let label = series
            .legend_label
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                doc.data_sheet(&series.source_sheet_id)
                    .or_else(|| doc.result_sheet(&series.source_sheet_id).and_then(|res| res.analyzable_data.as_ref()))
                    .and_then(|ds| data_set_title(&ds.archetype, series.data_set_index))
                    .map(str::to_owned)
                    .filter(|s| !s.trim().is_empty())
            })
            .unwrap_or_else(|| format!("Series {}", idx + 1));

        let entry = entry_from_gdo(label, &series.gdo, graph.palette.color_at(idx));
        entries.push(entry);
    }
    entries
}

fn data_set_title(archetype: &TableArchetype, idx: usize) -> Option<&str> {
    match archetype {
        TableArchetype::Xy(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(idx).map(|c| c.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(idx).map(|c| c.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(idx).map(|v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(idx).map(|g| g.title.as_str()),
    }
}

fn entry_from_gdo(label: String, gdo: &curvebook_doc::GraphicalDataObject, fallback: Rgba8) -> LegendEntry {
    use curvebook_doc::GraphicalDataObject as Gdo;
    match gdo {
        Gdo::ScatterAndLine { marker, line, .. } => LegendEntry {
            label,
            swatch_color: marker.map_or_else(|| line.map_or(fallback, |l| l.color), |m| m.fill_color),
            stroke_color: marker.map(|m| m.stroke_color).or_else(|| line.map(|l| l.color)),
            marker_shape: marker.map(|m| m.shape),
            marker_fill_mode: marker.map_or(MarkerFillMode::Filled, |m| m.fill_mode),
            dash_style: line.map(|l| l.dash_style),
            is_box_swatch: false,
        },
        Gdo::BarWithReplicates { fill_color, border_color, .. } | Gdo::FloatingBar { fill_color, border_color, .. } => LegendEntry {
            label,
            swatch_color: *fill_color,
            stroke_color: Some(*border_color),
            marker_shape: None,
            marker_fill_mode: MarkerFillMode::Filled,
            dash_style: None,
            is_box_swatch: true,
        },
        Gdo::BoxAndWhisker { fill_color, stroke_color, .. } | Gdo::Violin { fill_color, stroke_color, .. } => LegendEntry {
            label,
            swatch_color: *fill_color,
            stroke_color: Some(*stroke_color),
            marker_shape: None,
            marker_fill_mode: MarkerFillMode::Filled,
            dash_style: None,
            is_box_swatch: true,
        },
        Gdo::AlignedDotPlot { marker, .. } => LegendEntry {
            label,
            swatch_color: marker.fill_color,
            stroke_color: Some(marker.stroke_color),
            marker_shape: Some(marker.shape),
            marker_fill_mode: marker.fill_mode,
            dash_style: None,
            is_box_swatch: false,
        },
        Gdo::BeforeAfterLines { marker, line_color, .. } => LegendEntry {
            label,
            swatch_color: *line_color,
            stroke_color: Some(*line_color),
            marker_shape: marker.map(|m| m.shape),
            marker_fill_mode: marker.map_or(MarkerFillMode::Filled, |m| m.fill_mode),
            dash_style: Some(LineDashStyle::Solid),
            is_box_swatch: false,
        },
        Gdo::KaplanMeierStep { line_color, .. } => LegendEntry {
            label,
            swatch_color: *line_color,
            stroke_color: Some(*line_color),
            marker_shape: None,
            marker_fill_mode: MarkerFillMode::Filled,
            dash_style: Some(LineDashStyle::Solid),
            is_box_swatch: false,
        },
        Gdo::PartsOfWhole { .. } => LegendEntry {
            label,
            swatch_color: fallback,
            stroke_color: Some(Rgba8::BLACK),
            marker_shape: None,
            marker_fill_mode: MarkerFillMode::Filled,
            dash_style: None,
            is_box_swatch: true,
        },
    }
}

const LEGEND_FONT_SIZE_PT: f64 = 8.5;
const LEGEND_ROW_H_PT: f64 = 13.0;
const LEGEND_SWATCH_W_PT: f64 = 18.0;
const LEGEND_PAD_PT: f64 = 6.0;
const LEGEND_SWATCH_TEXT_GAP_PT: f64 = 6.0;
const LEGEND_OUTSIDE_GAP_PT: f64 = 8.0;
const LEGEND_OUTSIDE_RIGHT_BUFFER_PT: f64 = 2.0;

fn legend_box_width_for_text(max_text_w: f64) -> f64 {
    LEGEND_PAD_PT * 2.0 + LEGEND_SWATCH_W_PT + LEGEND_SWATCH_TEXT_GAP_PT + max_text_w
}

/// Measure the auto-placed legend box width (in points) for `labels` after expanding template hooks.
/// Returns `None` if `legend` is hidden, has an explicit user `position_pt`, or `labels` is empty.
pub(crate) fn auto_legend_box_width_pt<'a>(
    legend: &LegendSpec,
    labels: impl IntoIterator<Item = &'a str>,
    doc: &ProjectDocument,
    info_id: Option<&SheetId>,
) -> Option<f64> {
    if !legend.visible || legend.position_pt.is_some() {
        return None;
    }
    let mut count = 0usize;
    let mut max_text_w = 12.0_f64;
    for label in labels {
        count += 1;
        let expanded = resolve_template_hooks_with_info(label, doc, info_id);
        let rich = parse_markup(&expanded);
        max_text_w = max_text_w.max(measure_text_pt(&rich, LEGEND_FONT_SIZE_PT).width_pt);
    }
    (count > 0).then(|| legend_box_width_for_text(max_text_w))
}

/// Placement hints for automatic figure legend layout.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LegendPlacement {
    pub outside_right: bool,
    pub outside_right_offset_pt: f64,
}

/// Render a figure legend from `legend` and `entries` into `scene`.
pub fn render_legend(legend: &LegendSpec, entries: &[LegendEntry], doc: &ProjectDocument, primary_id: Option<&SheetId>, scene: &mut ChartScene) {
    let info_id = resolve_paired_info_id(doc, primary_id);
    let _ = render_legend_with_placement(legend, entries, doc, info_id, LegendPlacement::default(), scene);
}

fn auto_legend_origin(scene: &ChartScene, box_w: f64, placement: LegendPlacement) -> (f64, f64) {
    let r = scene.plot_rect_pt;
    let start_x = r.right() + placement.outside_right_offset_pt.max(0.0) + LEGEND_OUTSIDE_GAP_PT;
    if placement.outside_right && scene.width_pt - start_x + 0.5 >= box_w + LEGEND_OUTSIDE_RIGHT_BUFFER_PT {
        (start_x, r.y)
    } else if placement.outside_right {
        ((r.right() - box_w - 4.0).max(r.x + 2.0), r.y + 4.0)
    } else {
        ((r.right() - box_w - 6.0).max(r.x + 4.0), r.y + 6.0)
    }
}

pub(crate) fn render_legend_with_placement(
    legend: &LegendSpec,
    entries: &[LegendEntry],
    doc: &ProjectDocument,
    info_id: Option<&SheetId>,
    placement: LegendPlacement,
    scene: &mut ChartScene,
) -> Option<RectPt> {
    if !legend.visible || entries.is_empty() {
        return None;
    }
    let mut font_size = LEGEND_FONT_SIZE_PT;
    let mut row_h = LEGEND_ROW_H_PT;
    let swatch_w = LEGEND_SWATCH_W_PT;
    let pad = LEGEND_PAD_PT;

    let mut parsed_labels = Vec::with_capacity(entries.len());
    let mut max_text_w = 12.0_f64;
    for entry in entries {
        let expanded = resolve_template_hooks_with_info(&entry.label, doc, info_id);
        let rich = parse_markup(&expanded);
        max_text_w = max_text_w.max(measure_text_pt(&rich, font_size).width_pt);
        parsed_labels.push(rich);
    }

    let mut box_w = legend_box_width_for_text(max_text_w);
    if placement.outside_right && legend.position_pt.is_none() {
        let avail_right = scene.width_pt - (scene.plot_rect_pt.right() + placement.outside_right_offset_pt.max(0.0));
        let max_outside_box_w = avail_right - (LEGEND_OUTSIDE_GAP_PT + LEGEND_OUTSIDE_RIGHT_BUFFER_PT);
        let chrome_w = LEGEND_PAD_PT * 2.0 + LEGEND_SWATCH_W_PT + LEGEND_SWATCH_TEXT_GAP_PT;
        if box_w > max_outside_box_w && max_outside_box_w > chrome_w + 12.0 {
            let target_text_w = (max_outside_box_w - chrome_w - 0.5).max(12.0);
            let scale = (target_text_w / max_text_w).clamp(0.65, 1.0);
            font_size = (LEGEND_FONT_SIZE_PT * scale).max(5.5);
            row_h = (LEGEND_ROW_H_PT * scale).max(9.5);
            max_text_w = parsed_labels.iter().map(|rich| measure_text_pt(rich, font_size).width_pt).fold(12.0_f64, f64::max);
            box_w = legend_box_width_for_text(max_text_w);
        }
    }
    let box_h = pad * 2.0 + (entries.len() as f64) * row_h;
    let (bx, by) = match legend.position_pt {
        Some((px, py)) if px.is_finite() && py.is_finite() => (px, py),
        Some((px, py)) => {
            scene.warnings.push(ChartWarning::new(
                "non_finite_legend_position",
                format!("ignored non-finite legend position ({px}, {py}); placed automatically"),
            ));
            auto_legend_origin(scene, box_w, placement)
        }
        None => auto_legend_origin(scene, box_w, placement),
    };
    let box_rect = RectPt::new(bx, by, box_w, box_h);

    if legend.bordered {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(box_rect),
            fill: Some(FillPaint::solid(Rgba8::rgba(255, 255, 255, 235))),
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.8)),
            clip: None,
        });
    }

    for (idx, (entry, rich)) in entries.iter().zip(parsed_labels).enumerate() {
        let row_cy = by + pad + ((idx as f64) + 0.5) * row_h;
        let sx0 = bx + pad;
        emit_legend_swatch(entry, sx0, row_cy, swatch_w, scene);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(sx0 + swatch_w + 6.0, row_cy + font_size * 0.34),
            anchor: TextAnchor::Start,
            font_size_pt: font_size,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
    Some(box_rect)
}

fn emit_legend_swatch(entry: &LegendEntry, x0: f64, cy: f64, width: f64, scene: &mut ChartScene) {
    if entry.is_box_swatch {
        let swatch_rect = RectPt::new(x0 + 2.0, cy - 4.0, (width - 4.0).max(6.0), 8.0);
        let stroke = entry.stroke_color.map(|c| StrokePaint::solid(c, 0.8));
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(swatch_rect),
            fill: Some(FillPaint::solid(entry.swatch_color)),
            stroke,
            clip: None,
        });
        return;
    }

    if let Some(dash) = entry.dash_style {
        let col = entry.stroke_color.unwrap_or(entry.swatch_color);
        let stroke = StrokePaint { color: col, width_pt: 1.4, dash, cap: LineCap::Butt, join: LineJoin::Miter };
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x0, cy), PointPt::new(x0 + width, cy)),
            fill: None,
            stroke: Some(stroke),
            clip: None,
        });
    }

    if entry.marker_shape.is_some() || entry.dash_style.is_none() {
        let cx = x0 + 0.5 * width;
        let shape = entry.marker_shape.unwrap_or(MarkerShape::Circle);
        let style = MarkerStyle {
            shape,
            size_pt: 6.0,
            fill_mode: entry.marker_fill_mode,
            fill_color: entry.swatch_color,
            stroke_color: entry.stroke_color.unwrap_or(entry.swatch_color),
            stroke_width_pt: if entry.stroke_color.is_some() { 0.9 } else { 0.0 },
        };
        crate::markers::emit_marker(&mut scene.primitives, PointPt::new(cx, cy), &style, None);
    }
}

/// Shared per-scene resource budgets and decoded-image cache for graph annotations.
#[derive(Debug, Default)]
pub(crate) struct AnnotationBudget {
    pub image_pixels: u64,
    pub table_work: usize,
    pub image_cache: crate::image_decode::DecodedImageCache,
}

fn rect_is_finite_nonneg(rect: RectPt) -> bool {
    rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite() && rect.width >= 0.0 && rect.height >= 0.0
}

/// Render all [`GraphAnnotation`] items (`TextBox`, `LineOrArrow`, `Shape`, `EmbeddedResultTable`,
/// `ModelEquation`, `EmbeddedImage`) onto `scene`.
pub fn render_annotations(annotations: &[GraphAnnotation], doc: &ProjectDocument, primary_id: Option<&SheetId>, scene: &mut ChartScene) {
    let info_id = resolve_paired_info_id(doc, primary_id);
    let mut budget = AnnotationBudget::default();
    if let Err(err) = render_annotations_with_budget(annotations, doc, info_id, &mut budget, scene) {
        scene.warnings.push(ChartWarning::new("embedded_table_budget_exceeded", err.to_string()));
    }
}

/// Shared graph-annotation renderer enforcing cumulative pixel, table primitive, finite-geometry,
/// and per-scene decoded-image caching budgets across all graph families.
pub(crate) fn render_annotations_with_budget(
    annotations: &[GraphAnnotation],
    doc: &ProjectDocument,
    info_id: Option<&SheetId>,
    budget: &mut AnnotationBudget,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    for ann in annotations {
        match ann {
            GraphAnnotation::TextBox { bounds_pt, markdown, bordered } => {
                render_textbox_annotation(RectPt::from(*bounds_pt), markdown, *bordered, doc, info_id, scene);
            }
            GraphAnnotation::LineOrArrow { start_pt, end_pt, arrowhead, dash, color } => {
                render_line_or_arrow_annotation(*start_pt, *end_pt, *arrowhead, *dash, *color, scene);
            }
            GraphAnnotation::Shape { kind, bounds_pt, stroke_color, stroke_width_pt, dash, fill_color } => {
                render_shape_annotation(*kind, RectPt::from(*bounds_pt), *stroke_color, *stroke_width_pt, *dash, *fill_color, scene);
            }
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, tab_id, bounds_pt, selected_rows, selected_cols, style } => {
                let bounds = RectPt::from(*bounds_pt);
                if !rect_is_finite_nonneg(bounds) {
                    scene.warnings.push(ChartWarning::new("non_finite_annotation", "skipped EmbeddedResultTable annotation with invalid bounds"));
                    continue;
                }
                crate::xy::render_styled_embedded_table(
                    scene,
                    doc,
                    &crate::xy::EmbeddedTableDrawSpec {
                        sheet_id: result_sheet_id,
                        tab_id: tab_id.as_deref(),
                        bounds,
                        selected_rows: selected_rows.as_deref(),
                        selected_cols: selected_cols.as_deref(),
                        style,
                        missing_warning_code: "missing_embedded_result_table",
                    },
                    Some(&mut budget.table_work),
                )?;
            }
            GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered } => {
                let bounds = RectPt::from(*bounds_pt);
                if !rect_is_finite_nonneg(bounds) || !font_size_pt.is_finite() || *font_size_pt <= 0.0 {
                    scene.warnings.push(ChartWarning::new("non_finite_annotation", "skipped ModelEquation annotation with non-finite geometry"));
                    continue;
                }
                crate::equation_box::render_model_equation_annotation(
                    scene,
                    doc,
                    &crate::equation_box::ModelEquationParams {
                        result_sheet_id,
                        data_set_index: *data_set_index,
                        mode: *mode,
                        bounds_pt: *bounds_pt,
                        font_size_pt: *font_size_pt,
                        bordered: *bordered,
                    },
                );
            }
            GraphAnnotation::EmbeddedImage { asset_id, bounds_pt, rotation_deg, crop, border, .. } => {
                let bounds = RectPt::from(*bounds_pt);
                if !rect_is_finite_nonneg(bounds) || !rotation_deg.is_finite() {
                    scene.warnings.push(ChartWarning::new("non_finite_annotation", "skipped EmbeddedImage annotation with non-finite geometry"));
                    continue;
                }
                crate::image_decode::emit_embedded_image_primitive_cached(
                    scene,
                    doc,
                    &crate::image_decode::EmbeddedImageDrawSpec {
                        asset_id,
                        bounds_pt: *bounds_pt,
                        rotation_deg: *rotation_deg,
                        crop: *crop,
                        border: *border,
                        clip: None,
                        badge_override: None,
                    },
                    Some(&mut budget.image_pixels),
                    Some(&mut budget.image_cache),
                );
            }
        }
    }
    Ok(())
}

fn render_textbox_annotation(rect: RectPt, markdown: &str, bordered: bool, doc: &ProjectDocument, info_id: Option<&SheetId>, scene: &mut ChartScene) {
    if !rect_is_finite_nonneg(rect) {
        scene.warnings.push(ChartWarning::new("non_finite_annotation", "skipped TextBox annotation with non-finite bounds"));
        return;
    }
    let has_box = rect.width > 0.0 && rect.height > 0.0;
    if bordered && has_box {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(rect),
            fill: Some(FillPaint::solid(Rgba8::rgba(255, 255, 255, 240))),
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.8)),
            clip: None,
        });
    }
    let expanded = resolve_template_hooks_with_info(markdown, doc, info_id);
    let font_size = 8.5_f64;
    let line_h = font_size * 1.3;
    if has_box {
        scene.primitives.push(ScenePrimitive::PushClip { rect });
    }
    for (idx, line) in expanded.lines().enumerate() {
        let y_pt = rect.y + 4.0 + font_size + (idx as f64) * line_h;
        if has_box && y_pt > rect.bottom() + line_h {
            break;
        }
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(line),
            anchor_pt: PointPt::new(rect.x + 4.0, y_pt),
            anchor: TextAnchor::Start,
            font_size_pt: font_size,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
    if has_box {
        scene.primitives.push(ScenePrimitive::PopClip);
    }
}

fn render_line_or_arrow_annotation(
    start_pt: (f64, f64),
    end_pt: (f64, f64),
    arrowhead: bool,
    dash: LineDashStyle,
    color: Rgba8,
    scene: &mut ChartScene,
) {
    if !start_pt.0.is_finite() || !start_pt.1.is_finite() || !end_pt.0.is_finite() || !end_pt.1.is_finite() {
        scene.warnings.push(ChartWarning::new("non_finite_annotation", "skipped LineOrArrow annotation with non-finite coordinates"));
        return;
    }
    let p0 = PointPt::new(start_pt.0, start_pt.1);
    let p1 = PointPt::new(end_pt.0, end_pt.1);
    let stroke = StrokePaint { color, width_pt: 1.25, dash, cap: LineCap::Round, join: LineJoin::Round };
    scene.primitives.push(ScenePrimitive::Path { path: Path2D::line(p0, p1), fill: None, stroke: Some(stroke), clip: None });

    if arrowhead {
        let dx = p1.x - p0.x;
        let dy = p1.y - p0.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len > 1e-6 {
            let ux = dx / len;
            let uy = dy / len;
            let nx = -uy;
            let ny = ux;
            let head_len = 7.0_f64.min(len * 0.6);
            let half_w = 3.0_f64;
            let base_x = p1.x - ux * head_len;
            let base_y = p1.y - uy * head_len;
            let head = Path2D {
                commands: vec![
                    PathCmd::MoveTo(p1),
                    PathCmd::LineTo(PointPt::new(base_x + nx * half_w, base_y + ny * half_w)),
                    PathCmd::LineTo(PointPt::new(base_x - nx * half_w, base_y - ny * half_w)),
                    PathCmd::Close,
                ],
            };
            scene.primitives.push(ScenePrimitive::Path { path: head, fill: Some(FillPaint::solid(color)), stroke: None, clip: None });
        }
    }
}

fn render_shape_annotation(
    kind: AnnotationShapeKind,
    rect: RectPt,
    stroke_color: Rgba8,
    stroke_width_pt: f64,
    dash: LineDashStyle,
    fill_color: Option<Rgba8>,
    scene: &mut ChartScene,
) {
    if !rect_is_finite_nonneg(rect) || !stroke_width_pt.is_finite() || stroke_width_pt < 0.0 {
        scene.warnings.push(ChartWarning::new("non_finite_annotation", "skipped Shape annotation with non-finite geometry"));
        return;
    }
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let path = match kind {
        AnnotationShapeKind::Rectangle => Path2D::rect(rect),
        AnnotationShapeKind::RoundedRectangle => rounded_rect_path(rect),
        AnnotationShapeKind::Ellipse => ellipse_path(rect),
    };
    let fill = fill_color.map(FillPaint::solid);
    let stroke = (stroke_width_pt.is_finite() && stroke_width_pt > 0.0).then_some(StrokePaint {
        color: stroke_color,
        width_pt: stroke_width_pt,
        dash,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
    });
    scene.primitives.push(ScenePrimitive::Path { path, fill, stroke, clip: None });
}

fn rounded_rect_path(r: RectPt) -> Path2D {
    let rad = crate::clamp_f64_safe(r.width.min(r.height) * 0.15, 1.0, 8.0);
    let x0 = r.x;
    let x1 = r.right();
    let y0 = r.y;
    let y1 = r.bottom();
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(x0 + rad, y0)),
            PathCmd::LineTo(PointPt::new(x1 - rad, y0)),
            PathCmd::QuadTo { control: PointPt::new(x1, y0), to: PointPt::new(x1, y0 + rad) },
            PathCmd::LineTo(PointPt::new(x1, y1 - rad)),
            PathCmd::QuadTo { control: PointPt::new(x1, y1), to: PointPt::new(x1 - rad, y1) },
            PathCmd::LineTo(PointPt::new(x0 + rad, y1)),
            PathCmd::QuadTo { control: PointPt::new(x0, y1), to: PointPt::new(x0, y1 - rad) },
            PathCmd::LineTo(PointPt::new(x0, y0 + rad)),
            PathCmd::QuadTo { control: PointPt::new(x0, y0), to: PointPt::new(x0 + rad, y0) },
            PathCmd::Close,
        ],
    }
}

/// Format an arbitrary slice of [`ChartWarning`] entries into a deterministic diagnostic report line.
#[must_use]
pub fn summarize_warnings(warnings: &[ChartWarning]) -> String {
    let mut out = String::new();
    for (idx, w) in warnings.iter().enumerate() {
        if idx > 0 {
            out.push_str("; ");
        }
        let _ = write!(out, "[{}] {}", w.code, w.message);
    }
    out
}
