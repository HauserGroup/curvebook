//! Horizontal-orientation column/grouped graph compiler, horizontal CLD labels, and right-side brackets.

use curvebook_doc::{AxisSpec, GraphSheet, GraphicalDataObject, GroupedArrangement, PairwiseDisplayMode, ProjectDocument};
use curvebook_palette::Rgba8;
use curvebook_text::{RichText, measure_text_pt, parse_markup};

use super::brackets_cld::{ResolvedCldLabels, count_active_brackets, find_matched_index, item_summary_value, resolve_brackets, resolve_cld_overlay};
use super::items::{BeforeAfterBounds, render_before_after_connectors_oriented, render_single_item_oriented};
use super::{ConnectorAxes, RenderedSlotBound, StackAccum, build_legend_items, collect_domain_values, extract_items, prepare_superimposed_items};
use crate::scene::{ChartError, ChartScene, ChartWarning, Path2D, PathCmd, PointPt, PositionedText, ScenePrimitive, StrokePaint, TextAnchor};
use crate::xy::{
    ChromeAxes, SegmentedAxis, configure_plot_rect, primary_info_sheet_id, render_cartesian_chrome_full, render_legend_and_annotations_with_offset,
};

#[derive(Debug, Clone)]
pub(super) struct HorizontalSlotBound {
    pub(super) group_key: usize,
    pub(super) label: String,
    pub(super) alt_label: String,
    pub(super) y_pt: f64,
    pub(super) right_x_pt: f64,
}

pub(super) fn compile_horizontal_column_or_grouped(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
    primary_opt: Option<&curvebook_doc::DataSheet>,
) -> Result<(), ChartError> {
    let has_y2 = graph.spec.y2_axis.is_some()
        || graph.spec.series.iter().any(|s| s.bind_to_right_y2)
        || matches!(graph.spec.frame_preset, curvebook_doc::FramePreset::DualYSecondaryRight);
    let mut tick_positions: Vec<(f64, String)> = Vec::new();
    let mut items = primary_opt.map(|primary| extract_items(graph, primary, &mut tick_positions, &mut scene.warnings)).unwrap_or_default();

    let draw_brackets =
        matches!(graph.spec.pairwise_display_mode, PairwiseDisplayMode::BracketsOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay);
    let active_bracket_est = if draw_brackets { count_active_brackets(graph, doc, &items) } else { 0 };
    let slot_metas: Vec<(usize, &str, &str, Option<f64>, f64)> =
        items.iter().map(|it| (it.group_key, it.match_label.as_str(), it.alt_label.as_str(), item_summary_value(it), it.x_pos)).collect();
    let resolved_cld = resolve_cld_overlay(graph, doc, &slot_metas, &mut scene.warnings);

    let legend_items = primary_opt.map(|primary| build_legend_items(graph, primary)).unwrap_or_default();
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let legend_box_w = crate::scales::auto_legend_box_width_pt(&graph.spec.legend, legend_items.iter().map(|it| it.label.as_str()), doc, info_id);
    configure_plot_rect(graph, scene, has_y2, 0.0, legend_box_w);
    let has_y_title = !graph.spec.y_axis.title.trim().is_empty();
    let max_cat_w_pt = tick_positions.iter().map(|(_, lbl)| measure_text_pt(&parse_markup(lbl), 8.0).width_pt).fold(0.0_f64, f64::max);
    if max_cat_w_pt > 0.0 {
        let min_left = if has_y_title { 28.0 + max_cat_w_pt + 10.0 } else { 14.0 + max_cat_w_pt + 8.0 };
        let right = scene.plot_rect_pt.right();
        let max_left = (scene.width_pt * 0.42).min((right - 40.0).max(scene.plot_rect_pt.x));
        let new_left = scene.plot_rect_pt.x.max(min_left).min(max_left);
        scene.plot_rect_pt.x = new_left;
        scene.plot_rect_pt.width = (right - new_left).max(40.0);
    }
    let extra_right = (if active_bracket_est > 0 { ((active_bracket_est.min(4) as f64) * 18.0).min(72.0) } else { 0.0 })
        + if resolved_cld.is_some() { 24.0 } else { 0.0 };
    let applied_extra_right = if extra_right > 0.0 {
        let shrink = extra_right.min((scene.plot_rect_pt.width - 40.0).max(0.0));
        scene.plot_rect_pt.width -= shrink;
        shrink
    } else {
        0.0
    };

    let Some(primary) = primary_opt else {
        let r = scene.plot_rect_pt;
        let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &[0.0, 1.0], false, 0.0);
        let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &[0.0, 1.0], false, 0.0);
        let y2_axis = has_y2.then(|| {
            let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2"));
            SegmentedAxis::build(&y2_spec, r.bottom(), r.y, &[0.0, 1.0], false, 0.0)
        });
        render_cartesian_chrome_full(scene, graph, doc, &ChromeAxes { x: &x_axis, y: &y_axis, y2: y2_axis.as_ref(), cat_x: None, cat_y: None });
        return Ok(());
    };

    if items.iter().any(|it| it.bind_to_right_y2) {
        scene.warnings.push(ChartWarning::new(
            "horizontal_y2_unsupported",
            "right Y2 series binding is not supported in horizontal orientation; plotted against horizontal value axis",
        ));
    }

    let (cat_vals, mut val_vals, y2_vals) = collect_domain_values(graph, &items, &tick_positions, &mut scene.warnings);
    val_vals.extend(y2_vals);
    if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Superimposed) {
        prepare_superimposed_items(&mut items);
    }

    let include_zero_x = items.iter().any(|it| matches!(it.gdo, GraphicalDataObject::BarWithReplicates { .. }));
    let headroom = if active_bracket_est > 0 || resolved_cld.is_some() { 0.14 } else { 0.05 };
    let r = scene.plot_rect_pt;
    let x_axis = SegmentedAxis::build(&graph.spec.x_axis, r.x, r.right(), &val_vals, include_zero_x, headroom);
    let y_axis = SegmentedAxis::build(&graph.spec.y_axis, r.bottom(), r.y, &cat_vals, false, 0.0);
    let y2_axis = has_y2.then(|| {
        let y2_spec = graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2"));
        SegmentedAxis::build(&y2_spec, r.bottom(), r.y, &cat_vals, false, 0.0)
    });

    let cat_ticks_opt = if tick_positions.is_empty() { None } else { Some(tick_positions.as_slice()) };
    render_cartesian_chrome_full(scene, graph, doc, &ChromeAxes { x: &x_axis, y: &y_axis, y2: y2_axis.as_ref(), cat_x: None, cat_y: cat_ticks_opt });

    let mut horiz_bounds: Vec<HorizontalSlotBound> = Vec::with_capacity(items.len());
    let connector_axes = ConnectorAxes { x: &y_axis, y: &x_axis, y2: None };
    render_before_after_connectors_oriented(scene, &graph.spec, primary, &items, connector_axes, BeforeAfterBounds::Horizontal(&mut horiz_bounds));

    let mut stacks = StackAccum::default();
    for item in &mut items {
        if matches!(item.gdo, GraphicalDataObject::BeforeAfterLines { .. }) {
            continue;
        }
        let right_x = render_single_item_oriented(scene, &graph.spec, item, &y_axis, &x_axis, &mut stacks, true);
        if let Some((_, y_pt)) = y_axis.map_value(item.x_pos) {
            horiz_bounds.push(HorizontalSlotBound {
                group_key: item.group_key,
                label: item.match_label.clone(),
                alt_label: item.alt_label.clone(),
                y_pt,
                right_x_pt: right_x.unwrap_or(r.x),
            });
        }
    }

    if let Some(cld) = &resolved_cld {
        render_horizontal_cld_labels(scene, cld, &mut horiz_bounds);
    }
    if draw_brackets {
        render_horizontal_pairwise_brackets(scene, graph, doc, &horiz_bounds);
    }

    let _ = render_legend_and_annotations_with_offset(scene, graph, doc, &legend_items, applied_extra_right)?;
    Ok(())
}

fn render_horizontal_cld_labels(scene: &mut ChartScene, cld: &ResolvedCldLabels, bounds: &mut [HorizontalSlotBound]) {
    let max_x_floor = (scene.width_pt - 8.0).max(scene.plot_rect_pt.x + 12.0);
    for entry in &cld.labels {
        let Some(pos) =
            find_matched_index(bounds, entry.group_key, Some(entry.label.as_str()), |b| (b.group_key, b.label.as_str(), b.alt_label.as_str()))
        else {
            continue;
        };
        let bound = &mut bounds[pos];
        let rich = RichText::plain(entry.letters.clone());
        let metrics = measure_text_pt(&rich, cld.font_size_pt);
        let label_w = metrics.width_pt.max(cld.font_size_pt * 0.6);
        let desired_x = (bound.right_x_pt + cld.offset_pt).min((max_x_floor - label_w).max(scene.plot_rect_pt.x + 4.0));
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(desired_x, bound.y_pt + cld.font_size_pt * 0.35),
            anchor: TextAnchor::Start,
            font_size_pt: cld.font_size_pt,
            rotation_deg: 0.0,
            color: cld.color,
        }));
        bound.right_x_pt = bound.right_x_pt.max(desired_x + label_w + 2.0);
    }
}

fn render_horizontal_pairwise_brackets(scene: &mut ChartScene, graph: &GraphSheet, doc: &ProjectDocument, bounds: &[HorizontalSlotBound]) {
    if graph.spec.comparisons.is_empty() || bounds.is_empty() {
        return;
    }
    let proxy: Vec<RenderedSlotBound> = bounds
        .iter()
        .map(|b| RenderedSlotBound {
            group_key: b.group_key,
            label: b.label.clone(),
            alt_label: b.alt_label.clone(),
            x_pt: b.y_pt,
            top_y_pt: b.right_x_pt,
        })
        .collect();
    let mut resolved = resolve_brackets(graph, doc, &proxy, &mut scene.warnings);
    if resolved.is_empty() {
        return;
    }
    resolved.sort_by(|a, b| {
        let span_a = a.slot_a_idx.abs_diff(a.slot_b_idx);
        let span_b = b.slot_a_idx.abs_diff(b.slot_b_idx);
        span_a.cmp(&span_b).then_with(|| a.x_lo.total_cmp(&b.x_lo))
    });

    let mut placed: Vec<(f64, f64, f64)> = Vec::with_capacity(resolved.len());
    let max_bar_x = (scene.width_pt - 14.0).max(scene.plot_rect_pt.x + 12.0);
    for br in resolved {
        let data_right_x =
            bounds.iter().filter(|b| b.y_pt >= br.x_lo - 1.0 && b.y_pt <= br.x_hi + 1.0).map(|b| b.right_x_pt).fold(scene.plot_rect_pt.x, f64::max);
        let metrics = measure_text_pt(&br.label, br.font_size_pt);
        let label_w = metrics.width_pt.max(br.font_size_pt);
        let min_step = br.step_pt.max(label_w + 8.0);
        let mut bar_x = (data_right_x + 8.0).max(scene.plot_rect_pt.x + 8.0);
        for &(py0, py1, p_right) in &placed {
            if py1 >= br.x_lo - 2.0 && py0 <= br.x_hi + 2.0 {
                bar_x = bar_x.max(p_right + min_step);
            }
        }
        bar_x = crate::clamp_f64_safe(bar_x, scene.plot_rect_pt.x + 8.0, (max_bar_x - label_w).max(scene.plot_rect_pt.x + 8.0));
        let stroke = StrokePaint::solid(Rgba8::BLACK, br.stroke_width_pt);
        let path = Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(bar_x - 4.0, br.x_lo)),
                PathCmd::LineTo(PointPt::new(bar_x, br.x_lo)),
                PathCmd::LineTo(PointPt::new(bar_x, br.x_hi)),
                PathCmd::LineTo(PointPt::new(bar_x - 4.0, br.x_hi)),
            ],
        };
        scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: None });
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: br.label,
            anchor_pt: PointPt::new(bar_x + 4.0, 0.5 * (br.x_lo + br.x_hi) + br.font_size_pt * 0.35),
            anchor: TextAnchor::Start,
            font_size_pt: br.font_size_pt,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
        placed.push((br.x_lo, br.x_hi, bar_x + label_w + 4.0));
    }
}
