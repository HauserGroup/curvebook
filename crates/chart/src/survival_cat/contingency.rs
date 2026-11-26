//! Contingency grouped and stacked bar builders (raw counts and 100% proportions).

use curvebook_doc::{
    CategoricalTickSource, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, ProjectDocument, TableArchetype, TickLabelFormat,
};
use curvebook_palette::{LineDashStyle, Rgba8};

use super::shared::{
    LegendEntry, PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, draw_legend_entries, layout_and_render_segmented_chrome,
    resolve_fresh_data_sheet_or_upstream, solid_stroke,
};
use crate::scene::{ChartError, ChartScene, ChartWarning, FillPaint, Path2D, RectPt, ScenePrimitive};

// ============================================================================
// 2. Contingency Grouped & Stacked Bars (Counts & 100% Proportions)
// ============================================================================

struct ContingencyMatrix {
    category_labels: Vec<String>,
    series_labels: Vec<String>,
    series_colors: Vec<Rgba8>,
    /// `counts[cat_idx][series_idx]`
    counts: Vec<Vec<f64>>,
}

pub(super) fn compile_contingency(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some(mat) = extract_contingency_matrix(graph, doc, &mut scene.warnings) else {
        scene.warnings.push(ChartWarning::new("empty_contingency_data", "no contingency counts found"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let proportion_scale = detect_proportion_scale(&graph.spec);
    let is_stacked = matches!(graph.spec.grouped_arrangement, GroupedArrangement::Stacked);
    let is_superimposed = matches!(graph.spec.grouped_arrangement, GroupedArrangement::Superimposed);

    let mut values = mat.counts.clone();
    if let Some(scale_top) = proportion_scale {
        for row in &mut values {
            let row_sum: f64 = row.iter().sum();
            if row_sum > 0.0 {
                for cell in row.iter_mut() {
                    *cell = (*cell / row_sum) * scale_top;
                }
            }
        }
    }

    let data_max = if let Some(scale_top) = proportion_scale {
        if is_stacked { scale_top } else { values.iter().flat_map(|r| r.iter()).copied().fold(0.0_f64, f64::max).max(1.0) }
    } else if is_stacked {
        values.iter().map(|r| r.iter().sum::<f64>()).fold(0.0_f64, f64::max).max(1.0)
    } else {
        values.iter().flat_map(|r| r.iter()).copied().fold(0.0_f64, f64::max).max(1.0)
    };

    let y_hi_bound = if proportion_scale.is_some() && is_stacked { data_max } else { data_max * 1.1 };
    let n_cats = mat.category_labels.len().max(1);
    let x_cat_positions: Vec<(f64, String)> = mat.category_labels.iter().enumerate().map(|(idx, lbl)| ((idx as f64) + 0.5, lbl.clone())).collect();

    let info_id = doc.paired_info_sheet_id(&graph.spec.primary_source_sheet_id);
    let legend_box_width_pt = crate::scales::auto_legend_box_width_pt(&graph.spec.legend, mat.series_labels.iter().map(String::as_str), doc, info_id);
    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (0.0, n_cats as f64),
            y_bounds: (0.0, y_hi_bound),
            y2_bounds: None,
            cat_x: Some(&x_cat_positions),
            cat_y: None,
            margins: PlotMarginConfig { has_legend: !mat.series_labels.is_empty(), legend_box_width_pt, ..PlotMarginConfig::default() },
        },
    );

    let r = scene.plot_rect_pt;
    let bar_border = solid_stroke(Rgba8::rgb(40, 40, 40), 0.75, LineDashStyle::Solid);
    let y_base_pt = y_axis.map_value(0.0).map_or(r.bottom(), |(_, pt)| crate::clamp_f64_safe(pt, r.y, r.bottom()));
    let mut warned_log_clip = false;

    for (cat_idx, row_vals) in values.iter().enumerate() {
        let Some((_, center_x)) = x_axis.map_value((cat_idx as f64) + 0.5) else {
            continue;
        };
        let slot_left = x_axis.map_value(cat_idx as f64).map_or(r.x, |(_, pt)| pt);
        let slot_right = x_axis.map_value((cat_idx + 1) as f64).map_or(r.right(), |(_, pt)| pt);
        let slot_w = (slot_right - slot_left).abs().max(8.0);
        if is_stacked {
            let bar_w = (slot_w * 0.62).min(48.0);
            let mut acc = 0.0;
            for (s_idx, &val) in row_vals.iter().enumerate() {
                if !val.is_finite() || val <= 0.0 {
                    if val.is_finite() {
                        acc += val;
                    }
                    continue;
                }
                let top_val = acc + val;
                let Some((_, y_top)) = y_axis.map_value(top_val) else {
                    acc = top_val;
                    continue;
                };
                let y_bot = match y_axis.map_value(acc) {
                    Some((_, yb)) => yb,
                    None if acc <= 0.0 => {
                        if !warned_log_clip {
                            scene.warnings.push(ChartWarning::new(
                                "log_axis_floor_clip",
                                "clipped stacked contingency base count 0.0 to lower axis floor on positive-domain Y axis",
                            ));
                            warned_log_clip = true;
                        }
                        y_base_pt
                    }
                    None => {
                        acc = top_val;
                        continue;
                    }
                };
                let rect = RectPt::new(center_x - bar_w * 0.5, y_top.min(y_bot), bar_w, (y_bot - y_top).abs().max(0.5));
                let color = mat.series_colors.get(s_idx).copied().unwrap_or_else(|| graph.spec.palette.color_at(s_idx));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(FillPaint::solid(color)),
                    stroke: Some(bar_border),
                    clip: Some(r),
                });
                acc = top_val;
            }
        } else if is_superimposed {
            let base_w = (slot_w * 0.62).min(44.0);
            let mut order: Vec<(usize, f64)> = row_vals.iter().copied().enumerate().collect();
            order.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()).then_with(|| a.0.cmp(&b.0)));
            for (rank, &(s_idx, val)) in order.iter().enumerate() {
                let Some((_, y_top)) = y_axis.map_value(val) else { continue };
                let bar_w = (base_w * (1.0 - 0.12 * (rank as f64).min(4.0))).max(4.0);
                let rect = RectPt::new(center_x - bar_w * 0.5, y_top.min(y_base_pt), bar_w, (y_base_pt - y_top).abs().max(0.5));
                let color = mat.series_colors.get(s_idx).copied().unwrap_or_else(|| graph.spec.palette.color_at(s_idx));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(FillPaint::solid(color)),
                    stroke: Some(bar_border),
                    clip: Some(r),
                });
            }
        } else {
            let n_s = mat.series_labels.len().max(1);
            let group_ratio = if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Separated) { 0.58 } else { 0.74 };
            let group_w = slot_w * group_ratio;
            let bar_w = (group_w / (n_s as f64)).min(32.0);
            let total_w = bar_w * (n_s as f64);
            let start_x = center_x - total_w * 0.5;
            for (s_idx, &val) in row_vals.iter().enumerate() {
                let Some((_, y_top)) = y_axis.map_value(val) else { continue };
                let x_left = start_x + (s_idx as f64) * bar_w;
                let rect = RectPt::new(x_left, y_top.min(y_base_pt), (bar_w - 1.0).max(2.0), (y_base_pt - y_top).abs().max(0.5));
                let color = mat.series_colors.get(s_idx).copied().unwrap_or_else(|| graph.spec.palette.color_at(s_idx));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(FillPaint::solid(color)),
                    stroke: Some(bar_border),
                    clip: Some(r),
                });
            }
        }
    }

    let legend_entries: Vec<LegendEntry> = mat
        .series_labels
        .iter()
        .zip(mat.series_colors.iter())
        .map(|(lbl, &col)| LegendEntry { label: lbl.clone(), color: col, marker: None, dash: None, is_line: false })
        .collect();
    draw_legend_entries(scene, &graph.spec, &legend_entries, doc);
    Ok(())
}

fn detect_proportion_scale(spec: &GraphSpec) -> Option<f64> {
    // Only normalize contingency counts to row proportions when the Y axis title explicitly asks
    // for proportions/percentages (never infer normalization from a manual Y-axis max of 1 or 100,
    // which would silently rewrite raw counts on a "Count" chart).
    let title = spec.y_axis.title.to_ascii_lowercase();
    if title.contains("count") {
        return None;
    }
    if title.contains("proportion") || title.contains("fraction") {
        Some(1.0)
    } else if title.contains('%') || title.contains("percent") {
        Some(100.0)
    } else {
        None
    }
}

fn extract_contingency_matrix(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<ContingencyMatrix> {
    let ds = resolve_fresh_data_sheet_or_upstream(doc, &graph.spec.primary_source_sheet_id, warnings)?;
    let by_columns = matches!(graph.spec.x_axis.tick_format, TickLabelFormat::Categorical(CategoricalTickSource::ColumnTitles));
    match &ds.archetype {
        TableArchetype::Contingency(cont) => {
            let nrows = cont.row_count();
            let ncols = cont.data_sets.len();
            if nrows == 0 || ncols == 0 {
                return None;
            }
            if by_columns {
                let category_labels = cont.data_sets.iter().map(|c| c.title.clone()).collect();
                let series_labels: Vec<String> = (0..nrows)
                    .map(|r| cont.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1)))
                    .collect();
                let series_colors = (0..nrows).map(|r| series_color_or_palette(graph, r)).collect();
                let mut counts = vec![vec![0.0; nrows]; ncols];
                for (c, col_counts) in counts.iter_mut().enumerate() {
                    for (r, slot) in col_counts.iter_mut().enumerate() {
                        *slot = cont.count_at(r, c).ok().flatten().unwrap_or(0) as f64;
                    }
                }
                Some(ContingencyMatrix { category_labels, series_labels, series_colors, counts })
            } else {
                let category_labels: Vec<String> = (0..nrows)
                    .map(|r| cont.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1)))
                    .collect();
                let series_labels: Vec<String> = (0..ncols)
                    .map(|c| {
                        graph
                            .spec
                            .series
                            .iter()
                            .find(|s| s.data_set_index == c)
                            .and_then(|s| s.legend_label.clone())
                            .unwrap_or_else(|| cont.data_sets.get(c).map_or_else(|| format!("Series {c}"), |d| d.title.clone()))
                    })
                    .collect();
                let series_colors = (0..ncols).map(|c| series_color_or_palette(graph, c)).collect();
                let mut counts = vec![vec![0.0; ncols]; nrows];
                for (r, row_counts) in counts.iter_mut().enumerate() {
                    for (c, slot) in row_counts.iter_mut().enumerate() {
                        *slot = cont.count_at(r, c).ok().flatten().unwrap_or(0) as f64;
                    }
                }
                Some(ContingencyMatrix { category_labels, series_labels, series_colors, counts })
            }
        }
        TableArchetype::Column(col) => {
            let nrows = col.row_count();
            let ncols = col.data_sets.len();
            if nrows == 0 || ncols == 0 {
                return None;
            }
            let category_labels =
                (0..nrows).map(|r| col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1))).collect();
            let series_labels = col.data_sets.iter().map(|d| d.title.clone()).collect();
            let series_colors = (0..ncols).map(|c| series_color_or_palette(graph, c)).collect();
            let mut counts = vec![vec![0.0; ncols]; nrows];
            for (r, row_counts) in counts.iter_mut().enumerate() {
                for (c, ds_col) in col.data_sets.iter().enumerate() {
                    row_counts[c] = ds_col.cells.get(r).and_then(|s| s.first()).and_then(|cell| cell.active_number()).unwrap_or(0.0).max(0.0);
                }
            }
            Some(ContingencyMatrix { category_labels, series_labels, series_colors, counts })
        }
        _ => None,
    }
}

fn series_color_or_palette(graph: &GraphSheet, idx: usize) -> Rgba8 {
    if let Some(s) = graph.spec.series.iter().find(|s| s.data_set_index == idx).or_else(|| graph.spec.series.get(idx)) {
        match s.gdo {
            GraphicalDataObject::BarWithReplicates { fill_color, .. } | GraphicalDataObject::FloatingBar { fill_color, .. } => return fill_color,
            GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } => return m.fill_color,
            _ => {}
        }
    }
    graph.spec.palette.color_at(idx)
}
