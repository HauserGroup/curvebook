//! Volcano plot (`-log10(p)` vs `log2FC`) and Bland-Altman agreement diagnostic plot builders.

use curvebook_doc::{
    AnalysisKind, DataSheet, GraphSheet, GraphSpec, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, TableArchetype,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_two_sided_isf;
use curvebook_math::sum::sample_sd;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::two_group::bland_altman;
use curvebook_text::{RichText, parse_markup};

use super::super::multivariate::cell_token;
use super::{
    DiagPoint, active_xy_pairs_with_sources, find_header_col, find_matching_result_checked, find_matching_result_where,
    find_matching_result_where_checked, has_blocked_matching_result, resolve_analysis_source_data_sheet, resolve_source_data_sheet_checked,
    result_cell_num, result_cell_p_or_num,
};
use crate::markers::emit_marker;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, ScenePrimitive, SourceCellId, TextAnchor,
};
use crate::survival_cat::{
    LegendEntry, PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, draw_legend_entries, layout_and_render_segmented_chrome, solid_stroke,
};
use crate::xy::format_point_value;

struct VolcanoPoint {
    diag: DiagPoint,
    significant: Option<bool>,
}

struct VolcanoExtracted {
    points: Vec<VolcanoPoint>,
    is_difference_units: bool,
}

// 2b. Volcano Plot (-log10(p) vs log2FC or Difference + Threshold Crosshairs)

pub(in crate::multivariate_layout) fn compile_volcano(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let extracted = extract_volcano_points(graph, doc, &mut scene.warnings);
    let pts = extracted.points;
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
    let is_difference_units = extracted.is_difference_units || mr_res.is_some();
    let has_user_x_thresholds = !graph.spec.x_axis.thresholds.is_empty();
    let has_user_y_thresholds = !graph.spec.y_axis.thresholds.is_empty();
    let (fc_cuts_opt, default_p_thresh) = volcano_thresholds(&graph.spec, is_difference_units);
    let p_thresh = if !has_user_y_thresholds { mr_threshold.unwrap_or(default_p_thresh) } else { default_p_thresh };
    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.diag.x), b.max(p.diag.x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.diag.y), b.max(p.diag.y)));
    let raw_x_abs = x_min.abs().max(x_max.abs());
    let x_abs = match fc_cuts_opt {
        Some((fc_neg, fc_pos)) => raw_x_abs.max(fc_neg.abs()).max(fc_pos.abs()).max(1.0) * 1.1,
        None => {
            if raw_x_abs > 1e-12 {
                raw_x_abs * 1.1
            } else {
                1.0
            }
        }
    };

    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title.trim().is_empty() || frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = String::from(if is_difference_units { "Difference (A - B)" } else { "log2 Fold Change" });
    }
    if frame_graph.spec.y_axis.title.trim().is_empty() || frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = String::from("-log10(P)");
    }
    let info_id = doc.paired_info_sheet_id(&frame_graph.spec.primary_source_sheet_id);
    let volcano_legend_labels = ["Up-regulated", "Down-regulated", "Not significant"];
    let legend_box_width_pt = crate::scales::auto_legend_box_width_pt(&frame_graph.spec.legend, volcano_legend_labels, doc, info_id);
    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (-x_abs, x_abs),
            y_bounds: (0.0_f64.min(y_min), (y_max.max(p_thresh + 0.5)) * 1.1),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig { has_legend: true, legend_box_width_pt, ..PlotMarginConfig::default() },
        },
    );
    let r = scene.plot_rect_pt;

    let crosshair_stroke = solid_stroke(Rgba8::rgb(110, 118, 128), 1.0, LineDashStyle::Dashed);
    if let Some((fc_neg, fc_pos)) = fc_cuts_opt {
        for xv in [fc_neg, fc_pos] {
            if let Some((_, xp)) = x_axis.map_value(xv) {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(xp, r.y), PointPt::new(xp, r.bottom())),
                    fill: None,
                    stroke: Some(crosshair_stroke),
                    clip: Some(r),
                });
            }
        }
    } else if !has_user_x_thresholds && let Some((_, xp0)) = x_axis.map_value(0.0) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(xp0, r.y), PointPt::new(xp0, r.bottom())),
            fill: None,
            stroke: Some(crosshair_stroke),
            clip: Some(r),
        });
    }
    if let Some((_, yp)) = y_axis.map_value(p_thresh) {
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
    let x_summary_prefix = if is_difference_units { "Difference" } else { "log2FC" };
    for pt in &pts {
        let (fc, neg_log_p) = (pt.diag.x, pt.diag.y);
        let (Some((_, px)), Some((_, py))) = (x_axis.map_value(fc), y_axis.map_value(neg_log_p)) else {
            continue;
        };
        let is_sig = if !has_user_y_thresholds && let Some(sig) = pt.significant { sig } else { neg_log_p >= p_thresh };
        let (is_up, is_down) = match fc_cuts_opt {
            Some((fc_neg, fc_pos)) => (is_sig && fc >= fc_pos, is_sig && fc <= fc_neg),
            None => (is_sig && fc > 0.0, is_sig && fc < 0.0),
        };
        let col = if is_up {
            up_col
        } else if is_down {
            down_col
        } else {
            ns_col
        };
        let base_marker = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: MarkerFillMode::Filled,
            size_pt: if is_up || is_down { 5.0 } else { 3.8 },
            fill_color: col,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.4,
        };
        let Some(marker) = crate::resolve_overridden_marker_for_col(
            &graph.spec,
            &pt.diag.source.sheet_id,
            pt.diag.source.col,
            pt.diag.source.row,
            Some(pt.diag.source.subcol),
            base_marker,
        ) else {
            continue;
        };
        let center_pt = PointPt::new(px, py);
        emit_marker(&mut scene.primitives, center_pt, &marker, Some(r));
        scene.plotted_points.push(PlottedPointMeta {
            center_pt,
            radius_pt: (marker.size_pt * 0.5).max(3.0),
            source_cell: pt.diag.source.clone(),
            series_label: pt.diag.series_label.clone(),
            value_summary: format!("{x_summary_prefix}={}, -log10P={}", format_point_value(fc), format_point_value(neg_log_p)),
        });
        if (is_up || is_down)
            && let Some(lbl) = &pt.diag.label
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
    draw_legend_entries(scene, &frame_graph.spec, &entries, doc);
    Ok(())
}

fn volcano_thresholds(spec: &GraphSpec, is_difference_units: bool) -> (Option<(f64, f64)>, f64) {
    let mut fc_cuts: Vec<f64> = spec.x_axis.thresholds.iter().map(|t| t.value).filter(|v| v.is_finite()).collect();
    fc_cuts.sort_by(f64::total_cmp);
    let cuts = match fc_cuts.as_slice() {
        [neg, pos, ..] => Some((*neg, *pos)),
        [one] => Some((-one.abs(), one.abs())),
        [] => (!is_difference_units).then_some((-1.0, 1.0)),
    };
    let p_cut = spec.y_axis.thresholds.first().map_or_else(|| -(0.05_f64.log10()), |t| t.value);
    (cuts, p_cut)
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

fn extract_volcano_points(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> VolcanoExtracted {
    let empty = VolcanoExtracted { points: Vec::new(), is_difference_units: false };
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, |r| matches!(r.analysis.kind, AnalysisKind::MultipleRowTTests(_)))
        .or_else(|| find_matching_result_checked(graph, doc, warnings))
        && (matches!(res.analysis.kind, AnalysisKind::MultipleRowTTests(_)) || doc.result_sheet(&graph.spec.primary_source_sheet_id).is_some())
    {
        let ds_id = res.analysis.source_sheet_id.clone();
        let ds_opt = resolve_analysis_source_data_sheet(res, graph, doc);
        let default_col = graph.spec.series.first().map_or(0, |s| s.data_set_index);
        let is_mr = matches!(res.analysis.kind, AnalysisKind::MultipleRowTTests(_));
        let from_pairs: Vec<VolcanoPoint> = res
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
                Some(VolcanoPoint {
                    diag: DiagPoint {
                        x: diff,
                        y: neg_log_p,
                        label: lbl,
                        source: SourceCellId { sheet_id: ds_id.clone(), row: rec.group_a, col: default_col, subcol: 0 },
                        series_label,
                    },
                    significant: Some(rec.significant),
                })
            })
            .collect();
        if !from_pairs.is_empty() {
            return VolcanoExtracted { points: from_pairs, is_difference_units: is_mr };
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
            let sig_col = find_header_col(&tab.column_headers, |h| h == "significant" || h.contains("discover"));
            if let (Some(dc), Some(pc)) = (diff_col, p_col) {
                let already_neg_log = tab.column_headers.get(pc).is_some_and(|h| h.to_ascii_lowercase().contains("-log10"));
                let hdr_is_diff = tab.column_headers.get(dc).is_some_and(|h| {
                    let l = h.to_ascii_lowercase();
                    l.contains("diff") && !l.contains("log2") && !l.contains("fold")
                });
                let from_tab: Vec<VolcanoPoint> = tab
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
                        let significant = sig_col.and_then(|sc| match row.get(sc) {
                            Some(ResultCell::Boolean(b)) => Some(*b),
                            _ => None,
                        });
                        let lbl = tab.row_labels.get(i).filter(|s| !s.trim().is_empty()).cloned();
                        let row_idx = parse_source_row_hint(tab, i, ds_opt);
                        let series_label = lbl.clone().unwrap_or_else(|| graph.spec.title.clone());
                        Some(VolcanoPoint {
                            diag: DiagPoint {
                                x: diff,
                                y: neg_log_p,
                                label: lbl,
                                source: SourceCellId { sheet_id: ds_id.clone(), row: row_idx, col: default_col, subcol: 0 },
                                series_label,
                            },
                            significant,
                        })
                    })
                    .collect();
                if !from_tab.is_empty() {
                    return VolcanoExtracted { points: from_tab, is_difference_units: is_mr || hdr_is_diff };
                }
            }
        }
    }

    if has_blocked_matching_result(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::MultipleRowTTests(_))) {
        return empty;
    }

    if let Some(ds) = resolve_source_data_sheet_checked(graph, doc, warnings) {
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
                            raw.push(VolcanoPoint {
                                diag: DiagPoint {
                                    x,
                                    y,
                                    label: lbl,
                                    source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: y_idx, subcol: 0 },
                                    series_label: yc.title.clone(),
                                },
                                significant: None,
                            });
                        }
                    }
                    let y_title_low = yc.title.to_ascii_lowercase();
                    let looks_raw_p = !y_title_low.contains("log") && !raw.is_empty() && raw.iter().all(|pt| pt.diag.y > 0.0 && pt.diag.y <= 1.0);
                    if looks_raw_p {
                        for pt in &mut raw {
                            pt.diag.y = -pt.diag.y.max(1e-300).log10();
                        }
                    }
                    let x_title_low = xc.title.to_ascii_lowercase();
                    let is_diff = x_title_low.contains("diff") && !x_title_low.contains("log2") && !x_title_low.contains("fold");
                    return VolcanoExtracted { points: raw, is_difference_units: is_diff };
                }
            }
            TableArchetype::Xy(xy) => {
                let pts = active_xy_pairs_with_sources(&ds.id, xy, 0)
                    .into_iter()
                    .map(|(x, y, source, series_label)| VolcanoPoint {
                        diag: DiagPoint { x, y, label: None, source, series_label },
                        significant: None,
                    })
                    .collect();
                return VolcanoExtracted { points: pts, is_difference_units: false };
            }
            _ => {}
        }
    }
    empty
}

// 2c. Bland-Altman Agreement Plot (Difference vs Average + Bias ± 1.96*SD LoA)

pub(in crate::multivariate_layout) fn compile_bland_altman(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let Some(BlandAltmanData { points: pts, bias, loa_lo, loa_hi, loa_lo_label, loa_hi_label, percent_difference }) =
        extract_bland_altman_data(graph, doc, &mut scene.warnings)
    else {
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
    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = String::from("Average of Methods");
    }
    if frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = String::from(if percent_difference { "Percent Difference (%)" } else { "Difference (A - B)" });
    }
    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (x_min - x_pad, x_max + x_pad),
            y_bounds: (y_min - y_pad, y_max + y_pad),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig::default(),
        },
    );

    let r = scene.plot_rect_pt;
    let color = graph.spec.palette.color_at(0);
    for pt in &pts {
        let (avg, diff) = (pt.x, pt.y);
        if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(avg), y_axis.map_value(diff)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 5.0,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.6,
            };
            if let Some(marker) = crate::resolve_overridden_marker_for_col(
                &graph.spec,
                &pt.source.sheet_id,
                pt.source.col,
                pt.source.row,
                Some(pt.source.subcol),
                base_marker,
            ) {
                let center_pt = PointPt::new(px, py);
                emit_marker(&mut scene.primitives, center_pt, &marker, Some(r));
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
        (loa_hi, solid_stroke(Rgba8::rgb(213, 94, 0), 1.2, LineDashStyle::Dashed), loa_hi_label),
        (loa_lo, solid_stroke(Rgba8::rgb(213, 94, 0), 1.2, LineDashStyle::Dashed), loa_lo_label),
    ];
    for (val, stroke, label) in ref_specs {
        if let Some((_, yp)) = y_axis.map_value(val) {
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
    loa_lo_label: String,
    loa_hi_label: String,
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

fn format_bland_altman_loa_labels(
    bias: f64,
    loa_lo: f64,
    loa_hi: f64,
    sd_bias_opt: Option<f64>,
    z_expected: f64,
    force_nonparametric: bool,
) -> (String, String) {
    if !force_nonparametric {
        if let Some(sd) = sd_bias_opt.filter(|s| s.is_finite() && *s > 1e-12) {
            let k_hi = (loa_hi - bias) / sd;
            let k_lo = (bias - loa_lo) / sd;
            if k_hi.is_finite() && k_lo.is_finite() && k_hi > 0.0 && k_lo > 0.0 && (k_hi - k_lo).abs() <= 1e-2 {
                let k = 0.5 * (k_hi + k_lo);
                return (format!("+{k:.2} SD: {loa_hi:.2}"), format!("-{k:.2} SD: {loa_lo:.2}"));
            }
        } else {
            let d_hi = loa_hi - bias;
            let d_lo = bias - loa_lo;
            if d_hi.is_finite() && d_lo.is_finite() && d_hi > 0.0 && d_lo > 0.0 && (d_hi - d_lo).abs() <= 1e-6 * bias.abs().max(d_hi).max(1.0) {
                return (format!("+{z_expected:.2} SD: {loa_hi:.2}"), format!("-{z_expected:.2} SD: {loa_lo:.2}"));
            }
        }
    }
    (format!("Upper LoA: {loa_hi:.2}"), format!("Lower LoA: {loa_lo:.2}"))
}

fn is_nonparametric_bland_altman(res: &ResultSheet) -> bool {
    let is_np_text = |text: &str| {
        let lower = text.to_ascii_lowercase();
        lower.contains("quantile") || lower.contains("percentile") || lower.contains("nonparametric") || lower.contains("non-parametric")
    };
    res.scalars.iter().any(|s| is_np_text(&s.key))
        || res.tabs.iter().any(|tab| is_np_text(&tab.title) || tab.row_labels.iter().any(|lbl| is_np_text(lbl)))
}

fn extract_bland_altman_pairs_from_ds(
    ds: &DataSheet,
    col_a: usize,
    col_b: usize,
    xy_compare_x_vs_y: Option<bool>,
    from_analysis: bool,
    graph: &GraphSheet,
) -> Option<BlandAltmanExtractedPairs> {
    match &ds.archetype {
        TableArchetype::Column(col) if col.data_sets.len() >= 2 => {
            if col_a == col_b {
                return None;
            }
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
            let (xi, yi) = if from_analysis {
                (col_a, col_b)
            } else {
                (graph.spec.multivariate.as_ref().map_or(col_a, |m| m.x_column), graph.spec.multivariate.as_ref().map_or(col_b, |m| m.y_column))
            };
            if xi == yi {
                return None;
            }
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
            let compare_x_vs_y = xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1);
            if compare_x_vs_y {
                let target_col = if col_a < xy.data_sets.len() { col_a } else { 0 };
                let pairs = active_xy_pairs_with_sources(&ds.id, xy, target_col);
                let label = xy.data_sets.get(target_col).map_or_else(|| String::from("Method A"), |d| d.title.clone());
                let mut sources = Vec::with_capacity(pairs.len());
                let mut va = Vec::with_capacity(pairs.len());
                let mut vb = Vec::with_capacity(pairs.len());
                for (x, y, src, _) in pairs {
                    sources.push(src);
                    va.push(x);
                    vb.push(y);
                }
                Some(BlandAltmanExtractedPairs { sources, a: va, b: vb, series_label: label })
            } else {
                if col_a == col_b {
                    return None;
                }
                let ds_a = xy.data_sets.get(col_a)?;
                let ds_b = xy.data_sets.get(col_b)?;
                let mut sources = Vec::new();
                let mut va = Vec::new();
                let mut vb = Vec::new();
                for r in 0..xy.row_count() {
                    if let (Some(x), Some(y)) = (
                        ds_a.cells.get(r).and_then(|s| s.first()).and_then(|c| c.active_number()).filter(|v| v.is_finite()),
                        ds_b.cells.get(r).and_then(|s| s.first()).and_then(|c| c.active_number()).filter(|v| v.is_finite()),
                    ) {
                        sources.push(SourceCellId { sheet_id: ds.id.clone(), row: r, col: col_a, subcol: 0 });
                        va.push(x);
                        vb.push(y);
                    }
                }
                Some(BlandAltmanExtractedPairs { sources, a: va, b: vb, series_label: ds_a.title.clone() })
            }
        }
        _ => None,
    }
}

fn extract_bland_altman_from_result(res: &ResultSheet, graph: &GraphSheet, doc: &ProjectDocument, title_pct_diff: bool) -> Option<BlandAltmanData> {
    let AnalysisKind::BlandAltman(ba_spec) = &res.analysis.kind else {
        return None;
    };
    let (col_a, col_b, spec_pct, xy_compare_x_vs_y) = (ba_spec.column_a, ba_spec.column_b, ba_spec.percent_difference, ba_spec.xy_compare_x_vs_y);
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
    let mut sd_bias_opt = find_scalar_ci(res, |k| k == "sdbias" || k == "sd_bias").or_else(|| find_tab_row_value(res, |l| l.contains("sd of bias")));

    let ds_opt = resolve_analysis_source_data_sheet(res, graph, doc);
    let ds_pairs = ds_opt.and_then(|ds| extract_bland_altman_pairs_from_ds(ds, col_a, col_b, xy_compare_x_vs_y, true, graph));
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
        sd_bias_opt = sd_bias_opt.or(Some(ba.sd_bias));
    }

    if raw_pts.len() < 2 {
        let _ = title_pct_diff;
        return None;
    }

    let diffs: Vec<f64> = raw_pts.iter().map(|p| p.1).collect();
    let bias = bias_opt.unwrap_or_else(|| diffs.iter().sum::<f64>() / (diffs.len() as f64));
    let sd_fallback = sample_sd(&diffs).unwrap_or(0.0);
    let sd_bias = sd_bias_opt.unwrap_or(sd_fallback);
    let z_expected = norm_two_sided_isf(res.analysis.confidence_level.alpha()).unwrap_or(1.959_963_984_540_054);
    let loa_lo = loa_lo_opt.unwrap_or(bias - z_expected * sd_bias);
    let loa_hi = loa_hi_opt.unwrap_or(bias + z_expected * sd_bias);
    let force_nonparametric = is_nonparametric_bland_altman(res);
    let (loa_hi_label, loa_lo_label) =
        format_bland_altman_loa_labels(bias, loa_lo, loa_hi, sd_bias_opt.or(Some(sd_fallback)), z_expected, force_nonparametric);
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
    Some(BlandAltmanData { points, bias, loa_lo, loa_hi, loa_lo_label, loa_hi_label, percent_difference: spec_pct })
}

fn extract_bland_altman_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<BlandAltmanData> {
    let title_pct_diff = graph.spec.y_axis.title.contains('%') || graph.spec.y_axis.title.to_ascii_lowercase().contains("percent");

    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, |r| matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)))
        && let Some(data) = extract_bland_altman_from_result(res, graph, doc, title_pct_diff)
    {
        return Some(data);
    }
    if has_blocked_matching_result(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::BlandAltman(_))) {
        return None;
    }

    let z_default = norm_two_sided_isf(ConfidenceLevel::DEFAULT_95.alpha()).unwrap_or(1.959_963_984_540_054);
    let ds = resolve_source_data_sheet_checked(graph, doc, warnings)?;
    if let TableArchetype::Xy(xy) = &ds.archetype
        && xy.data_sets.len() == 1
        && graph.spec.y_axis.title.to_ascii_lowercase().contains("diff")
    {
        let pairs = active_xy_pairs_with_sources(&ds.id, xy, 0);
        if pairs.len() >= 2 {
            let diffs: Vec<f64> = pairs.iter().map(|p| p.1).collect();
            let bias = diffs.iter().sum::<f64>() / (diffs.len() as f64);
            let sd = sample_sd(&diffs).unwrap_or(0.0);
            let loa_lo = bias - z_default * sd;
            let loa_hi = bias + z_default * sd;
            let (loa_hi_label, loa_lo_label) = format_bland_altman_loa_labels(bias, loa_lo, loa_hi, Some(sd), z_default, false);
            let points = pairs.into_iter().map(|(x, y, source, series_label)| DiagPoint { x, y, label: None, source, series_label }).collect();
            return Some(BlandAltmanData { points, bias, loa_lo, loa_hi, loa_lo_label, loa_hi_label, percent_difference: title_pct_diff });
        }
    }
    let extracted = extract_bland_altman_pairs_from_ds(ds, 0, 1, None, false, graph)?;
    let ba = bland_altman(&extracted.a, &extracted.b, title_pct_diff, ConfidenceLevel::DEFAULT_95).ok()?;
    let (loa_hi_label, loa_lo_label) = format_bland_altman_loa_labels(ba.bias, ba.loa_lower, ba.loa_upper, Some(ba.sd_bias), z_default, false);
    let points = ba
        .points
        .into_iter()
        .zip(extracted.sources)
        .map(|((x, y), source)| DiagPoint { x, y, label: None, source, series_label: extracted.series_label.clone() })
        .collect();
    Some(BlandAltmanData {
        points,
        bias: ba.bias,
        loa_lo: ba.loa_lower,
        loa_hi: ba.loa_upper,
        loa_lo_label,
        loa_hi_label,
        percent_difference: title_pct_diff,
    })
}
