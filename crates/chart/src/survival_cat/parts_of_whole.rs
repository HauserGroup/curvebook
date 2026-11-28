//! Parts-of-whole figure builders (pie, donut, and stacked fraction bar).

use std::f64::consts::PI;

use curvebook_doc::{GraphSheet, GraphSpec, GraphicalDataObject, PartsOfWholeLayout, ProjectDocument, SeriesSpec, TableArchetype};
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_text::{RichText, parse_markup};

use super::shared::{
    LegendEntry, PlotMarginConfig, draw_legend_entries, resolve_fresh_data_sheet_or_upstream, solid_stroke, trim_trailing_zeros,
    update_plot_rect_for_labels, wedge_path,
};
use crate::scene::{ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PointPt, PositionedText, RectPt, ScenePrimitive, TextAnchor};

// ============================================================================
// 3. Parts-of-Whole (Pie, Donut, StackedFractionBar)
// ============================================================================

pub(super) fn compile_parts_of_whole(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let (layout_mode, show_center_total) = resolve_parts_of_whole_style(&graph.spec.series);
    let slices = extract_parts_of_whole_slices(graph, doc, &mut scene.warnings);
    if slices.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_parts_of_whole", "no non-negative parts-of-whole slices available"));
        return Ok(());
    }

    let mut legend_entries = Vec::with_capacity(slices.len());
    for (idx, slice) in slices.iter().enumerate() {
        let color = graph.spec.palette.color_at(idx);
        let name = if slice.label.trim().is_empty() { format!("Slice {}", idx + 1) } else { slice.label.clone() };
        legend_entries.push(LegendEntry {
            label: format!("{name} ({:.1}%)", slice.fraction * 100.0),
            color,
            marker: None,
            dash: None,
            is_line: false,
        });
    }

    let info_id = doc.paired_info_sheet_id(&graph.spec.primary_source_sheet_id);
    let legend_box_width_pt =
        crate::scales::auto_legend_box_width_pt(&graph.spec.legend, legend_entries.iter().map(|e| e.label.as_str()), doc, info_id);
    let _ = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &[],
        PlotMarginConfig { has_legend: true, legend_box_width_pt, suppress_y_axis_labels: true, ..PlotMarginConfig::default() },
    );

    match layout_mode {
        PartsOfWholeLayout::Pie => render_pie_or_donut(scene, &graph.spec, &slices, 0.0, false),
        PartsOfWholeLayout::Donut => render_pie_or_donut(scene, &graph.spec, &slices, 0.56, show_center_total),
        PartsOfWholeLayout::StackedFractionBar => render_stacked_fraction_bar(scene, &graph.spec, &slices),
    }

    draw_legend_entries(scene, &graph.spec, &legend_entries, doc);
    Ok(())
}

fn resolve_parts_of_whole_style(series: &[SeriesSpec]) -> (PartsOfWholeLayout, bool) {
    for s in series {
        if let GraphicalDataObject::PartsOfWhole { layout, show_center_total } = s.gdo {
            return (layout, show_center_total);
        }
    }
    (PartsOfWholeLayout::Donut, true)
}

fn extract_parts_of_whole_slices(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Vec<curvebook_doc::SliceFraction> {
    let Some(ds) = resolve_fresh_data_sheet_or_upstream(doc, &graph.spec.primary_source_sheet_id, warnings) else { return Vec::new() };
    let series_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    match &ds.archetype {
        TableArchetype::PartsOfWhole(pow) => pow.fractions_for_series(series_idx).unwrap_or_default(),
        TableArchetype::Column(col) => {
            let Some(col_data) = col.data_sets.get(series_idx) else { return Vec::new() };
            let mut raw = Vec::new();
            let mut total = 0.0;
            for (r, subs) in col_data.cells.iter().enumerate() {
                if let Some(val) = subs.first().and_then(|c| c.active_number())
                    && val >= 0.0
                {
                    total += val;
                    let lbl = col.row_titles.get(r).cloned().unwrap_or_else(|| format!("Item {}", r + 1));
                    raw.push((r, lbl, val));
                }
            }
            raw.into_iter()
                .map(|(row, label, value)| curvebook_doc::SliceFraction {
                    row,
                    label,
                    value,
                    fraction: if total > 0.0 { value / total } else { 0.0 },
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn render_pie_or_donut(scene: &mut ChartScene, spec: &GraphSpec, slices: &[curvebook_doc::SliceFraction], inner_ratio: f64, show_center_total: bool) {
    let r = scene.plot_rect_pt;
    let cx = r.x + r.width * 0.5;
    let cy = r.y + r.height * 0.5;
    let r_outer = (r.width.min(r.height) * 0.43).max(16.0);
    let r_inner = r_outer * inner_ratio;
    let border = solid_stroke(Rgba8::WHITE, 1.0, LineDashStyle::Solid);

    let mut angle = -PI * 0.5;
    let mut total_val = 0.0;
    for (idx, slice) in slices.iter().enumerate() {
        total_val += slice.value;
        let sweep = slice.fraction * 2.0 * PI;
        if sweep <= 1e-6 {
            continue;
        }
        let end_angle = angle + sweep;
        let path = wedge_path(cx, cy, r_outer, r_inner, angle, end_angle);
        let fill = FillPaint::solid(spec.palette.color_at(idx));
        scene.primitives.push(ScenePrimitive::Path { path, fill: Some(fill), stroke: Some(border), clip: None });
        angle = end_angle;
    }

    if show_center_total && inner_ratio > 0.05 {
        let mut num_str = format!("{total_val:.2}");
        trim_trailing_zeros(&mut num_str);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&format!("<b>Total: {num_str}</b>")),
            anchor_pt: PointPt::new(cx, cy + 3.5),
            anchor: TextAnchor::Middle,
            font_size_pt: 9.5,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
}

fn render_stacked_fraction_bar(scene: &mut ChartScene, spec: &GraphSpec, slices: &[curvebook_doc::SliceFraction]) {
    let r = scene.plot_rect_pt;
    let bar_h = crate::clamp_f64_safe(r.height * 0.36, 24.0, 56.0);
    let bar_y = r.y + (r.height - bar_h) * 0.5;
    let border = solid_stroke(Rgba8::WHITE, 0.9, LineDashStyle::Solid);

    let mut cursor_x = r.x;
    for (idx, slice) in slices.iter().enumerate() {
        let seg_w = slice.fraction * r.width;
        if seg_w <= 0.5 {
            continue;
        }
        let rect = RectPt::new(cursor_x, bar_y, seg_w, bar_h);
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(rect),
            fill: Some(FillPaint::solid(spec.palette.color_at(idx))),
            stroke: Some(border),
            clip: None,
        });
        cursor_x += seg_w;
    }

    let axis_y = bar_y + bar_h + 10.0;
    let axis_stroke = solid_stroke(Rgba8::BLACK, 0.9, LineDashStyle::Solid);
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(r.x, axis_y), PointPt::new(r.right(), axis_y)),
        fill: None,
        stroke: Some(axis_stroke),
        clip: None,
    });
    for pct in [0, 25, 50, 75, 100] {
        let x = r.x + ((pct as f64) / 100.0) * r.width;
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x, axis_y), PointPt::new(x, axis_y + 3.5)),
            fill: None,
            stroke: Some(axis_stroke),
            clip: None,
        });
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(format!("{pct}%")),
            anchor_pt: PointPt::new(x, axis_y + 12.0),
            anchor: TextAnchor::Middle,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
}
