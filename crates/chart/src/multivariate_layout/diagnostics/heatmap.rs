//! Clustered heatmap diagnostic plot with perceptual colorbar and optional hierarchical dendrogram.

use curvebook_doc::{
    AnalysisKind, ClusterDistance, ClusterLinkage, ColorChannelMapping, GraphSheet, HeatmapColumnLabelAngle, HeatmapColumnLabelMode,
    HeatmapRowLabelMode, MultivariateSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, TableArchetype,
    VariableRole,
};
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, Rgba8};
use curvebook_stats::multivariate::{DendrogramMerge, hierarchical_clustering_pairwise_complete, hierarchical_clustering_with_metrics};
use curvebook_text::{RichText, parse_markup};

use super::super::multivariate::cell_token;
use super::{find_matching_result_where_checked, has_blocked_matching_result, resolve_source_data_sheet_checked, result_cell_num};
use crate::scene::{ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PointPt, PositionedText, RectPt, ScenePrimitive, TextAnchor};
use crate::survival_cat::{
    PlotMarginConfig, draw_colorbar, draw_empty_open_frame, path_line, path_move, sanitize_range, solid_stroke, text_metrics,
    update_plot_rect_for_labels,
};

/// Maximum vector heatmap tiles emitted per compiled scene (`GAP-02-13` OOM DoS guard).
const MAX_HEATMAP_RENDER_CELLS: usize = 25_000;
const MAX_HEATMAP_RENDER_ROWS: usize = 1_000;
const MAX_HEATMAP_RENDER_COLS: usize = 250;
/// Maximum cells for which per-cell text value annotations are rendered.
const MAX_HEATMAP_ANNOTATED_CELLS: usize = 2_500;

pub(super) fn cap_heatmap_dimensions(orig_rows: usize, orig_cols: usize, warnings: &mut Vec<ChartWarning>) -> (usize, usize, bool) {
    if orig_rows == 0 || orig_cols == 0 {
        return (orig_rows, orig_cols, false);
    }
    let mut rows = orig_rows.min(MAX_HEATMAP_RENDER_ROWS);
    let mut cols = orig_cols.min(MAX_HEATMAP_RENDER_COLS);
    if rows.saturating_mul(cols) > MAX_HEATMAP_RENDER_CELLS {
        if orig_cols <= MAX_HEATMAP_RENDER_COLS && MAX_HEATMAP_RENDER_CELLS / cols >= 1 {
            rows = (MAX_HEATMAP_RENDER_CELLS / cols).clamp(1, MAX_HEATMAP_RENDER_ROWS);
        } else if orig_rows <= MAX_HEATMAP_RENDER_ROWS && MAX_HEATMAP_RENDER_CELLS / rows >= 1 {
            cols = (MAX_HEATMAP_RENDER_CELLS / rows).clamp(1, MAX_HEATMAP_RENDER_COLS);
        } else {
            let scale = ((MAX_HEATMAP_RENDER_CELLS as f64) / ((rows as f64) * (cols as f64))).sqrt();
            rows = ((rows as f64) * scale).floor().max(1.0) as usize;
            cols = (MAX_HEATMAP_RENDER_CELLS / rows).max(1).min(cols);
            while rows.saturating_mul(cols) > MAX_HEATMAP_RENDER_CELLS && rows > 1 {
                rows -= 1;
            }
        }
    }
    let truncated = rows < orig_rows || cols < orig_cols;
    if truncated {
        warnings.push(ChartWarning::new(
            "heatmap_cell_budget_exceeded",
            format!(
                "heatmap matrix ({orig_rows}x{orig_cols} = {} cells) exceeded rendering budget ({MAX_HEATMAP_RENDER_CELLS} cells); truncated to {rows}x{cols}",
                orig_rows.saturating_mul(orig_cols),
            ),
        ));
    }
    (rows, cols, truncated)
}

/// Largest `k` with `k * k <= MAX_HEATMAP_RENDER_CELLS` and `k <= MAX_HEATMAP_RENDER_COLS` (158 for the current budget).
fn max_square_heatmap_side() -> usize {
    let mut k = MAX_HEATMAP_RENDER_COLS.min(MAX_HEATMAP_RENDER_ROWS);
    while k > 1 && k.saturating_mul(k) > MAX_HEATMAP_RENDER_CELLS {
        k -= 1;
    }
    k
}

/// Caps a symmetric `n x n` matrix (correlation matrix) to its leading `k x k` principal submatrix.
///
/// Unlike [`cap_heatmap_dimensions`], rows and columns are truncated together so cell `(i, j)`
/// keeps meaning `r(variable i, variable j)` and the unit diagonal stays on the diagonal.
fn cap_symmetric_heatmap_dimension(n: usize, warnings: &mut Vec<ChartWarning>) -> (usize, bool) {
    let k = n.min(max_square_heatmap_side());
    let truncated = k < n;
    if truncated {
        warnings.push(ChartWarning::new(
            "heatmap_cell_budget_exceeded",
            format!(
                "symmetric heatmap matrix ({n}x{n} = {} cells) exceeded rendering budget ({MAX_HEATMAP_RENDER_CELLS} cells); truncated to the leading {k}x{k} submatrix",
                n.saturating_mul(n),
            ),
        ));
    }
    (k, truncated)
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
    /// Square matrix whose rows and columns index the same variables (correlation matrix): any
    /// leaf ordering is applied to rows and columns together, and it is never auto-clustered.
    symmetric: bool,
    /// Suppresses the chart-side default clustering (truncated matrix, or a linked clustering
    /// result that is stale or cannot be applied), so rows stay in table order.
    suppress_auto_cluster: bool,
}

impl HeatmapData {
    fn new(row_labels: Vec<String>, col_labels: Vec<String>, values: Vec<Vec<Option<f64>>>) -> Self {
        Self {
            row_labels,
            col_labels,
            values,
            merges: Vec::new(),
            cluster_distance: ClusterDistance::Euclidean,
            cluster_linkage: ClusterLinkage::Ward,
            clustering_cue: None,
            symmetric: false,
            suppress_auto_cluster: false,
        }
    }
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

pub(in crate::multivariate_layout) fn compile_heatmap(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some(mut hm) = extract_heatmap_data(graph, doc, &mut scene.warnings) else {
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

    // Default row clustering of raw data matrices only: a symmetric correlation matrix keeps the
    // user's variable order (clustering its rows alone would scramble the diagonal), and a truncated
    // or stale-linked matrix is never silently re-clustered on a partial view of the data.
    if hm.merges.is_empty() && !hm.symmetric && !hm.suppress_auto_cluster && (2..=128).contains(&n_rows) && n_cols >= 2 {
        hm.merges = cluster_heatmap_rows(&hm.values, n_cols, hm.cluster_distance, hm.cluster_linkage, &mut scene.warnings);
    }

    let row_order = if !hm.merges.is_empty() { dendrogram_leaf_order(&hm.merges, n_rows) } else { (0..n_rows).collect() };
    let has_dendro = !hm.merges.is_empty();
    // A symmetric matrix is permuted on both axes so cell (i, j) keeps meaning (row label, column label).
    let col_order: Vec<usize> = if hm.symmetric && has_dendro && n_rows == n_cols { row_order.clone() } else { (0..n_cols).collect() };

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
    let total_cells = n_rows.saturating_mul(n_cols);
    let annotate_cells = lbl_spec.annotate_cells && total_cells <= MAX_HEATMAP_ANNOTATED_CELLS;
    if lbl_spec.annotate_cells && !annotate_cells {
        scene.warnings.push(ChartWarning::new(
            "heatmap_cell_annotations_capped",
            format!("heatmap cell text annotations suppressed for {total_cells} cells (limit {MAX_HEATMAP_ANNOTATED_CELLS})"),
        ));
    }
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
        for (vis_col, &c) in col_order.iter().enumerate() {
            let x = grid.x + (vis_col as f64) * cell_w;
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
                if annotate_cells {
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

    for (vis_col, lbl) in col_order.iter().filter_map(|&c| resolved_col_labels.get(c)).enumerate() {
        let cx = grid.x + ((vis_col as f64) + 0.5) * cell_w;
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

pub(super) fn spreadsheet_column_letters(mut index: usize) -> String {
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

fn correlation_heatmap_data(res: &ResultSheet, tab: &ResultTab, warnings: &mut Vec<ChartWarning>) -> HeatmapData {
    let orig_rows = tab.rows.len();
    let orig_cols = tab.column_headers.len();
    // A correlation matrix is square with the same variables on both axes; keep it square when
    // capping so the leading principal submatrix (not a row-major slab) is shown.
    let symmetric = orig_rows == orig_cols && orig_rows > 0;
    let (n_rows, n_cols, truncated) = if symmetric {
        let (k, truncated) = cap_symmetric_heatmap_dimension(orig_rows, warnings);
        (k, k, truncated)
    } else {
        cap_heatmap_dimensions(orig_rows, orig_cols, warnings)
    };
    let mut matrix = Vec::with_capacity(n_rows);
    for row in tab.rows.iter().take(n_rows) {
        let vals: Vec<Option<f64>> = row
            .iter()
            .take(n_cols)
            .map(|c| match c {
                ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
                _ => None,
            })
            .collect();
        matrix.push(vals);
    }
    let has_missing = matrix.iter().flatten().any(Option::is_none);
    let merges = if has_missing || truncated {
        Vec::new()
    } else {
        res.tabs.iter().find(|t| t.id == "hierarchical-dendrogram").map_or_else(Vec::new, |t| extract_dendro_merges_from_tab(t, n_rows))
    };
    let mut hm =
        HeatmapData::new(tab.row_labels.iter().take(n_rows).cloned().collect(), tab.column_headers.iter().take(n_cols).cloned().collect(), matrix);
    hm.merges = merges;
    hm.symmetric = symmetric;
    hm.suppress_auto_cluster = true;
    hm
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

fn extract_heatmap_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<HeatmapData> {
    let direct_res = doc.result_sheet(&graph.spec.primary_source_sheet_id);
    if let Some(res) = direct_res
        && matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
    {
        if matches!(res.status, ResultStatus::Failed { .. }) {
            warnings.push(ChartWarning::new("failed_result_sheet", format!("result sheet '{}' failed; omitted stale heatmap matrix", res.id)));
            return None;
        }
        if !res.has_displayable_output() {
            warnings.push(ChartWarning::new(
                "pending_analysis_result",
                format!("result sheet '{}' is pending recalculation; omitted stale heatmap matrix", res.id),
            ));
            return None;
        }
        if let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary) {
            return Some(correlation_heatmap_data(res, tab, warnings));
        }
    }

    let clustering_res = find_matching_result_where_checked(graph, doc, warnings, |r| {
        matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. }))
    });

    if clustering_res.is_none()
        && direct_res.is_none()
        && let Some(res) = find_matching_result_where_checked(graph, doc, warnings, |r| {
            matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
        })
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary)
    {
        return Some(correlation_heatmap_data(res, tab, warnings));
    }

    let selected_cols = clustering_res.map(|r| r.analysis.selected_data_sets.as_slice()).unwrap_or(&[]);
    let ds = resolve_source_data_sheet_checked(graph, doc, warnings)?;
    let (mut hm, dims) = match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let mut num_cols: Vec<(usize, String)> = if selected_cols.is_empty() {
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
            let orig_num_cols = num_cols.len();
            let (n_rows, n_cols, _) = cap_heatmap_dimensions(mv.row_count(), orig_num_cols, warnings);
            num_cols.truncate(n_cols);
            let label_col = graph
                .spec
                .multivariate
                .as_ref()
                .and_then(|m| m.label_column)
                .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Label));
            let mut row_labels = Vec::with_capacity(n_rows);
            let mut values = Vec::with_capacity(n_rows);
            for r in 0..n_rows {
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
            let col_labels = num_cols.into_iter().map(|(_, t)| t).collect();
            (
                HeatmapData::new(row_labels, col_labels, values),
                HeatmapDims { rows: n_rows, cols: n_cols, orig_rows: mv.row_count(), orig_cols: orig_num_cols },
            )
        }
        TableArchetype::Column(col) => {
            let mut col_indices: Vec<usize> = if selected_cols.is_empty() {
                (0..col.data_sets.len()).collect()
            } else {
                resolve_selected_heatmap_cols(selected_cols, col.data_sets.len())
            };
            if col_indices.is_empty() {
                return None;
            }
            let (orig_rows, orig_cols) = (col.row_count(), col_indices.len());
            let (nrows, ncols, _) = cap_heatmap_dimensions(orig_rows, orig_cols, warnings);
            col_indices.truncate(ncols);
            let col_labels: Vec<String> = col_indices.iter().filter_map(|&c| col.data_sets.get(c).map(|d| d.title.clone())).collect();
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
            (HeatmapData::new(row_labels, col_labels, values), HeatmapDims { rows: nrows, cols: ncols, orig_rows, orig_cols })
        }
        TableArchetype::Grouped(g) => {
            let mut col_indices: Vec<usize> = if selected_cols.is_empty() {
                (0..g.data_sets.len()).collect()
            } else {
                resolve_selected_heatmap_cols(selected_cols, g.data_sets.len())
            };
            if col_indices.is_empty() {
                return None;
            }
            let (orig_rows, orig_cols) = (g.row_count(), col_indices.len());
            let (nrows, ncols, _) = cap_heatmap_dimensions(orig_rows, orig_cols, warnings);
            col_indices.truncate(ncols);
            let col_labels: Vec<String> = col_indices.iter().filter_map(|&c| g.data_sets.get(c).map(|d| d.title.clone())).collect();
            let row_labels =
                (0..nrows).map(|r| g.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("R{}", r + 1))).collect();
            let values = (0..nrows)
                .map(|r| {
                    col_indices
                        .iter()
                        .map(|&c| {
                            if g.subcolumn_mode.is_pre_summarized() {
                                g.observation(r, c, 0).and_then(curvebook_doc::Cell::active_number).filter(|x| x.is_finite())
                            } else {
                                let reps: Vec<f64> = g.active_replicates(r, c).into_iter().filter(|x| x.is_finite()).collect();
                                if reps.is_empty() {
                                    None
                                } else {
                                    let mean = reps.iter().sum::<f64>() / (reps.len() as f64);
                                    mean.is_finite().then_some(mean)
                                }
                            }
                        })
                        .collect()
                })
                .collect();
            (HeatmapData::new(row_labels, col_labels, values), HeatmapDims { rows: nrows, cols: ncols, orig_rows, orig_cols })
        }
        _ => return None,
    };

    let rows_truncated = dims.rows < dims.orig_rows;
    let cols_truncated = dims.cols < dims.orig_cols;
    // Never let the chart silently substitute its own default clustering for a matrix the user
    // cannot see in full, or for a linked clustering result that is stale or failed.
    if rows_truncated || cols_truncated {
        hm.suppress_auto_cluster = true;
    }
    if clustering_res.is_none()
        && has_blocked_matching_result(graph, doc, |r| matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })))
    {
        hm.suppress_auto_cluster = true;
    }

    if let Some(cres) = clustering_res
        && let AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance, linkage, .. }) = cres.analysis.kind
    {
        let (dist_lbl, link_lbl) = cluster_metric_labels(distance, linkage);
        if rows_truncated {
            // Dendrogram leaves index every clustered row; with rows dropped the merge tree no longer
            // maps onto the drawn rows, so show table order rather than a re-clustered partial view.
            hm.suppress_auto_cluster = true;
            warnings.push(ChartWarning::new(
                "heatmap_clustering_not_shown_truncated",
                format!(
                    "clustering result '{}' ({dist_lbl} / {link_lbl}) is not shown because the heatmap was truncated to {} of {} rows; rows are drawn in table order",
                    cres.id, dims.rows, dims.orig_rows
                ),
            ));
        } else {
            // Only columns were dropped: the user's row dendrogram was computed on all variables and its
            // leaves still map one-to-one onto the drawn rows, so it is shown unchanged, with a cue that
            // states it was computed on every variable while only the leading columns are drawn.
            hm.cluster_distance = distance;
            hm.cluster_linkage = linkage;
            hm.clustering_cue = Some(if cols_truncated {
                format!("Clustered: {dist_lbl} / {link_lbl} (on all {} variables; first {} shown)", dims.orig_cols, dims.cols)
            } else {
                format!("Clustered: {dist_lbl} / {link_lbl}")
            });
            let has_missing = hm.values.iter().flatten().any(Option::is_none);
            if !has_missing {
                let tab_merges = cres
                    .tabs
                    .iter()
                    .find(|t| t.id == "hierarchical-dendrogram")
                    .map_or_else(Vec::new, |t| extract_dendro_merges_from_tab(t, dims.rows));
                if !tab_merges.is_empty() && tab_merges.len() + 1 == dims.rows {
                    hm.merges = tab_merges;
                }
            }
            if cols_truncated && hm.merges.is_empty() {
                warnings.push(ChartWarning::new(
                    "heatmap_clustering_not_shown_truncated",
                    format!(
                        "clustering result '{}' ({dist_lbl} / {link_lbl}) could not be mapped onto the truncated heatmap; rows are drawn in table order",
                        cres.id
                    ),
                ));
            }
        }
    }

    Some(hm)
}

/// Rendered vs original heatmap matrix dimensions after [`cap_heatmap_dimensions`].
struct HeatmapDims {
    rows: usize,
    cols: usize,
    orig_rows: usize,
    orig_cols: usize,
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

pub(super) fn decode_dendro_leaf(id: isize, n_leaves: usize) -> Option<usize> {
    if id >= 0 {
        return None;
    }
    let pos = id.checked_neg()?;
    let leaf = pos.checked_sub(1)?;
    let idx = usize::try_from(leaf).ok()?;
    (idx < n_leaves).then_some(idx)
}

pub(super) fn extract_dendro_merges_from_tab(tab: &ResultTab, n_leaves: usize) -> Vec<DendrogramMerge> {
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
    if let Ok(Some(merges)) = hierarchical_clustering_pairwise_complete(rows, n_cols, distance, linkage) {
        return merges;
    }
    warnings.push(ChartWarning::new(
        "clustering_skipped_missing_data",
        "hierarchical clustering skipped because rows lack overlapping finite observations",
    ));
    Vec::new()
}

pub(super) fn dendrogram_leaf_order(merges: &[DendrogramMerge], n_leaves: usize) -> Vec<usize> {
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

pub(super) fn draw_left_dendrogram(scene: &mut ChartScene, box_rect: RectPt, merges: &[DendrogramMerge], leaf_order: &[usize], cell_h: f64) {
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

#[cfg(test)]
mod tests {
    use crate::scene::{ChartScene, ScenePrimitive};

    #[test]
    fn cap_dimensions_cover_every_budget_branch() {
        use super::{cap_heatmap_dimensions, cap_symmetric_heatmap_dimension};

        let cap = |rows: usize, cols: usize| {
            let mut warnings = Vec::new();
            let out = cap_heatmap_dimensions(rows, cols, &mut warnings);
            assert_eq!(out.2, !warnings.is_empty(), "truncation must warn exactly when it truncates ({rows}x{cols})");
            out
        };
        // Exactly on the 25_000-cell budget with 250 columns: untouched, no warning.
        assert_eq!(cap(100, 250), (100, 250, false));
        // Columns over the 250 limit only: 10 x 250 = 2_500 cells fits, so only columns shrink.
        assert_eq!(cap(10, 300), (10, 250, true));
        // Columns over the limit and the capped 200 x 250 = 50_000 still over budget: rows kept,
        // columns = floor(25_000 / 200) = 125.
        assert_eq!(cap(200, 300), (200, 125, true));
        // Rows within limit, columns within limit, product over budget: 130 x 200 -> rows = 25_000 / 200 = 125.
        assert_eq!(cap(130, 200), (125, 200, true));
        // Both axes over their limits: 1_000 x 250 = 250_000 cells, scale = sqrt(25_000 / 250_000) = 0.3162,
        // rows = floor(1_000 * 0.3162) = 316, cols = floor(25_000 / 316) = 79 (316 * 79 = 24_964).
        assert_eq!(cap(2_000, 400), (316, 79, true));

        // Symmetric matrices keep a square leading block: floor(sqrt(25_000)) = 158 (158^2 = 24_964, 159^2 = 25_281).
        let sym = |n: usize| {
            let mut warnings = Vec::new();
            let out = cap_symmetric_heatmap_dimension(n, &mut warnings);
            assert_eq!(out.1, !warnings.is_empty());
            out
        };
        assert_eq!(sym(158), (158, false));
        assert_eq!(sym(159), (158, true));
        assert_eq!(sym(300), (158, true));
        assert_eq!(sym(4), (4, false));
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
}
