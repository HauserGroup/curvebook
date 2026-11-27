//! Tibshirani, Walther & Hastie (2001) Gap statistic diagnostic plot (`Gap(k) ± s_k` vs `k`
//! with optimal-`k` reference marker) from K-Means consensus / Gap coordinates tabs.
//!
//! Reference: Tibshirani, R., Walther, G., & Hastie, T. (2001). Estimating the number of
//! clusters in a data set via the gap statistic. *Journal of the Royal Statistical Society:
//! Series B (Statistical Methodology)*, 63(2), 411–423.

use curvebook_doc::{GraphSheet, KMeansAlgorithm, ProjectDocument, ResultSheet, ResultTab, TableArchetype, VariableRole};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::multivariate::compute_gap_statistic;
use curvebook_text::RichText;

use super::{find_header_col, find_matching_result_where_checked, has_blocked_matching_result, resolve_source_data_sheet_checked, result_cell_num};
use crate::markers::emit_marker;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, ScenePrimitive, SourceCellId, TextAnchor,
};
use crate::survival_cat::{
    PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, layout_and_render_segmented_chrome, path_line, path_move, solid_stroke,
};
use crate::xy::format_point_value;

#[derive(Debug, Clone, Copy)]
struct GapPoint {
    k: f64,
    gap: f64,
    se: f64,
}

struct GapExtracted {
    points: Vec<GapPoint>,
    optimal_k: Option<f64>,
}

fn find_gap_tab(res: &ResultSheet) -> Option<&ResultTab> {
    res.tabs.iter().find(|t| t.id == "gap-statistic").or_else(|| {
        res.tabs.iter().find(|t| {
            let has_k = find_header_col(&t.column_headers, |h| h.contains("cluster") || h == "k" || h.ends_with("(k)")).is_some();
            let has_gap = find_header_col(&t.column_headers, |h| h.contains("gap")).is_some();
            has_k && has_gap
        })
    })
}

fn is_gap_result(res: &ResultSheet) -> bool {
    find_gap_tab(res).is_some()
}

fn select_first_se_max_k(points: &[GapPoint]) -> Option<f64> {
    if points.is_empty() {
        return None;
    }
    let last_idx = points.len() - 1;
    let mut first_local_max = last_idx;
    for i in 0..last_idx {
        if points[i].gap >= points[i + 1].gap {
            first_local_max = i;
            break;
        }
    }
    let threshold = points[first_local_max].gap - points[first_local_max].se;
    for pt in &points[..=first_local_max] {
        if pt.gap >= threshold {
            return Some(pt.k);
        }
    }
    Some(points[first_local_max].k)
}

/// Compile [`curvebook_doc::DiagnosticPlotKind::GapStatisticPlot`] (`G(k) ± s_k` vs `k`).
pub(in crate::multivariate_layout) fn compile_gap_statistic_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let mut extracted = extract_gap_data(graph, doc, &mut scene.warnings);
    extracted.points.retain(|p| p.k.is_finite() && p.gap.is_finite());
    for pt in &mut extracted.points {
        if !pt.se.is_finite() || pt.se < 0.0 {
            pt.se = 0.0;
        }
    }
    if extracted.points.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_gap_statistic", "no Gap statistic G(k) coordinates available"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    extracted.points.sort_by(|a, b| a.k.total_cmp(&b.k));
    let pts = &extracted.points;
    let optimal_k =
        extracted.optimal_k.filter(|ok| ok.is_finite() && pts.iter().any(|p| (p.k - *ok).abs() <= 1e-6)).or_else(|| select_first_se_max_k(pts));

    let k_min = pts.first().map_or(1.0, |p| p.k);
    let k_max = pts.last().map_or(1.0, |p| p.k);
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.gap - p.se), b.max(p.gap + p.se)));
    let y_pad = (y_max - y_min).max(0.25) * 0.16;

    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title.trim().is_empty() || frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = String::from("Number of Clusters (k)");
    }
    if frame_graph.spec.y_axis.title.trim().is_empty() || frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = String::from("Gap Statistic G(k)");
    }
    let all_integer_k = pts.iter().all(|p| (p.k - p.k.round()).abs() <= 1e-9);
    if all_integer_k
        && (k_max - k_min) <= 15.0
        && let Some(seg) = frame_graph.spec.x_axis.segments.first_mut()
        && seg.major_tick_interval.is_none()
    {
        seg.major_tick_interval = Some(1.0);
    }

    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (k_min - 0.5, k_max + 0.5),
            y_bounds: (y_min - y_pad, y_max + y_pad),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig::default(),
        },
    );
    let r = scene.plot_rect_pt;

    let primary = graph.spec.palette.color_at(0);
    let accent = graph.spec.palette.color_at(1);

    // 1. Optimal-k vertical dashed marker line + inside-plot callout.
    if let Some(k_opt) = optimal_k
        && let Some((_, x_opt_pt)) = x_axis.map_value(k_opt)
    {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x_opt_pt, r.y), PointPt::new(x_opt_pt, r.bottom())),
            fill: None,
            stroke: Some(solid_stroke(accent, 1.1, LineDashStyle::Dashed)),
            clip: Some(r),
        });
        let k_text = if (k_opt - k_opt.round()).abs() <= 1e-9 { format!("k* = {}", k_opt.round() as i64) } else { format!("k* = {k_opt:.2}") };
        let place_left = x_opt_pt > r.x + r.width * 0.72;
        let (lbl_x, anchor) = if place_left {
            ((x_opt_pt - 4.0).max(r.x + 28.0), TextAnchor::End)
        } else {
            ((x_opt_pt + 4.0).min(r.right() - 28.0), TextAnchor::Start)
        };
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(k_text),
            anchor_pt: PointPt::new(lbl_x, r.y + 11.0),
            anchor,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: accent,
        }));
    }

    // 2. Connecting polyline across (k, Gap(k)).
    if pts.len() >= 2 {
        let mut line = Path2D::new();
        let mut started = false;
        for pt in pts {
            if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(pt.k), y_axis.map_value(pt.gap)) {
                let p = PointPt::new(px, py);
                if !started {
                    path_move(&mut line, p);
                    started = true;
                } else {
                    path_line(&mut line, p);
                }
            }
        }
        if started {
            scene.primitives.push(ScenePrimitive::Path {
                path: line,
                fill: None,
                stroke: Some(solid_stroke(primary, 1.6, LineDashStyle::Solid)),
                clip: Some(r),
            });
        }
    }

    // 3. ± s_k error bars + point markers (highlighting optimal k).
    let sheet_id = graph.spec.primary_source_sheet_id.clone();
    let err_stroke = solid_stroke(primary, 1.0, LineDashStyle::Solid);
    let cap_half = 3.2;
    for (idx, pt) in pts.iter().enumerate() {
        let (Some((_, px)), Some((_, py))) = (x_axis.map_value(pt.k), y_axis.map_value(pt.gap)) else {
            continue;
        };
        let is_opt = optimal_k.is_some_and(|ok| (pt.k - ok).abs() <= 1e-6);

        if pt.se > 0.0 {
            let (_, py_lo, _) = y_axis.map_clamped_to_nearest(pt.gap - pt.se);
            let (_, py_hi, _) = y_axis.map_clamped_to_nearest(pt.gap + pt.se);
            let stroke = if is_opt { solid_stroke(accent, 1.15, LineDashStyle::Solid) } else { err_stroke };
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(px, py_lo), PointPt::new(px, py_hi)),
                fill: None,
                stroke: Some(stroke),
                clip: Some(r),
            });
            for cap_y in [py_lo, py_hi] {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(px - cap_half, cap_y), PointPt::new(px + cap_half, cap_y)),
                    fill: None,
                    stroke: Some(stroke),
                    clip: Some(r),
                });
            }
        }

        let marker = if is_opt {
            MarkerStyle {
                shape: MarkerShape::Diamond,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 6.6,
                fill_color: accent,
                stroke_color: Rgba8::BLACK,
                stroke_width_pt: 0.75,
            }
        } else {
            MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 5.0,
                fill_color: primary,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.6,
            }
        };
        let center_pt = PointPt::new(px, py);
        emit_marker(&mut scene.primitives, center_pt, &marker, Some(r));
        scene.plotted_points.push(PlottedPointMeta {
            center_pt,
            radius_pt: (marker.size_pt * 0.5).max(3.0),
            source_cell: SourceCellId { sheet_id: sheet_id.clone(), row: idx, col: 0, subcol: 0 },
            series_label: String::from("Gap(k)"),
            value_summary: format!("k={}, Gap={} ± {}", format_point_value(pt.k), format_point_value(pt.gap), format_point_value(pt.se)),
        });
    }

    Ok(())
}

fn extract_gap_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> GapExtracted {
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, is_gap_result)
        && let Some(tab) = find_gap_tab(res)
    {
        let k_col = find_header_col(&tab.column_headers, |h| h.contains("cluster") || h == "k" || h.ends_with("(k)")).unwrap_or(0);
        let gap_col = find_header_col(&tab.column_headers, |h| h.contains("gap")).unwrap_or(3);
        let se_col = find_header_col(&tab.column_headers, |h| h.contains("se") || h.contains("s_k") || h.contains("sd"));

        let points: Vec<GapPoint> = tab
            .rows
            .iter()
            .filter_map(|row| {
                let k = result_cell_num(row.get(k_col)?)?;
                let gap = result_cell_num(row.get(gap_col)?)?;
                let se = se_col.and_then(|sc| row.get(sc)).and_then(result_cell_num).filter(|&v| v >= 0.0).unwrap_or(0.0);
                Some(GapPoint { k, gap, se })
            })
            .collect();

        let voted_gap_k = res.tabs.iter().find(|t| t.id == "consensus-k-votes").and_then(|vtab| {
            vtab.row_labels
                .iter()
                .position(|lbl| lbl.trim().eq_ignore_ascii_case("gap"))
                .and_then(|ri| vtab.rows.get(ri))
                .and_then(|r| r.first())
                .and_then(result_cell_num)
        });
        return GapExtracted { points, optimal_k: voted_gap_k };
    }

    if has_blocked_matching_result(graph, doc, is_gap_result) {
        return GapExtracted { points: Vec::new(), optimal_k: None };
    }

    let Some(ds) = resolve_source_data_sheet_checked(graph, doc, warnings) else {
        return GapExtracted { points: Vec::new(), optimal_k: None };
    };
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let titles: Vec<String> = mv.variables.iter().map(|v| v.title.clone()).collect();
            let k_col = find_header_col(&titles, |h| h.contains("cluster") || h == "k" || h.ends_with("(k)"));
            let gap_col = find_header_col(&titles, |h| h.contains("gap"));
            if let (Some(kc), Some(gc)) = (k_col, gap_col) {
                let se_col = find_header_col(&titles, |h| h.contains("se") || h.contains("s_k") || h.contains("sd"));
                let points: Vec<GapPoint> = (0..mv.row_count())
                    .filter_map(|r| {
                        let k = mv.variables.get(kc)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                        let gap = mv.variables.get(gc)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                        let se = se_col
                            .and_then(|sc| mv.variables.get(sc)?.cells.get(r)?.active_number())
                            .filter(|v| v.is_finite() && *v >= 0.0)
                            .unwrap_or(0.0);
                        Some(GapPoint { k, gap, se })
                    })
                    .collect();
                return GapExtracted { points, optimal_k: None };
            }

            // Compute Gap statistic directly from a numeric multivariate matrix (4..=200 complete rows).
            let num_cols: Vec<usize> = mv
                .variables
                .iter()
                .enumerate()
                .filter(|(_, v)| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated))
                .map(|(i, _)| i)
                .collect();
            if num_cols.is_empty() {
                return GapExtracted { points: Vec::new(), optimal_k: None };
            }
            let rows: Vec<Vec<f64>> = (0..mv.row_count())
                .filter_map(|r| {
                    let vals: Option<Vec<f64>> =
                        num_cols.iter().map(|&c| mv.variables.get(c)?.cells.get(r)?.active_number().filter(|v| v.is_finite())).collect();
                    vals
                })
                .collect();
            if (4..=200).contains(&rows.len()) {
                let k_max = 6.min(rows.len() - 1).max(2);
                if let Ok((curve, k_opt)) = compute_gap_statistic(&rows, k_max, 20, KMeansAlgorithm::Lloyd, 0xC105_7E57) {
                    let points = curve.into_iter().map(|g| GapPoint { k: g.k as f64, gap: g.gap, se: g.se_sim }).collect();
                    return GapExtracted { points, optimal_k: Some(k_opt as f64) };
                }
            }
            GapExtracted { points: Vec::new(), optimal_k: None }
        }
        TableArchetype::Xy(xy) => {
            let mut points = Vec::new();
            let gap_ds = xy.data_sets.first();
            let se_ds = xy.data_sets.get(1);
            if let Some(gds) = gap_ds {
                for r in 0..xy.row_count() {
                    let Some(k) = xy.x_summary_at(r).ok().flatten().filter(|v| v.is_finite()) else { continue };
                    let Some(gap) = gds.cells.get(r).and_then(|s| s.first()).and_then(|c| c.active_number()).filter(|v| v.is_finite()) else {
                        continue;
                    };
                    let se = se_ds
                        .and_then(|sds| sds.cells.get(r))
                        .and_then(|s| s.first())
                        .and_then(|c| c.active_number())
                        .filter(|v| v.is_finite() && *v >= 0.0)
                        .unwrap_or(0.0);
                    points.push(GapPoint { k, gap, se });
                }
            }
            GapExtracted { points, optimal_k: None }
        }
        _ => GapExtracted { points: Vec::new(), optimal_k: None },
    }
}
