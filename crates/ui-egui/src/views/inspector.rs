//! Live right-hand inspector view builder (`R10.18`).
//!
//! Renders 5 responsive sub-tabs (`Format Graph`, `Format Axes`, `Multivariable Channels`,
//! `Pairwise & CLD`, `Page Layout`) and interactive `Point & Drill-Back` controls,
//! dispatching instant undoable engine commands and reflecting live state on undo/redo.

use super::mv_formula::{LevelMoveOp, apply_explicit_level_move, resolve_selected_level_idx};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use curvebook_chart::{RectPt, SourceCellId};
use curvebook_doc::{
    AnalysisKind, AxisScale, AxisSegment, AxisSpec, BracketLabelMode, BreakOrnament, CanvasResizeMode, CldOrdering, ColorChannelMapping,
    CompactLetterDisplaySpec, ConnectingLineStyle, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FigureCanvasPreset, GraphSheet,
    GraphicalDataObject, GroupedArrangement, LayoutItem, LayoutSheet, LayoutSpec, LevelOrderKind, LevelOrderSpec, LineConnectionKind,
    MultivariateGraphEncoding, MultivariatePlotMode, PairwiseDisplayMode, PhysicalUnit, ProjectDocument, ReplicateOverlayStyle, SeriesSpec, SheetId,
    SizeChannelMapping, SummaryMeasure, TableArchetype, VariableRole, WhiskerMethod,
};
use curvebook_math::LogitDomain;
use curvebook_palette::{CanvasPreviewFilter, ColorRamp, ColormapId, LineDashStyle, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use curvebook_tools::GridCellCoord;

use crate::UiApp;
use crate::dialogs_canvas::{ActiveInspector, data_column_title};
use crate::interaction::UiEventOutcome;
use crate::workspace_nav::ActiveWorkspacePane;

const SCROLL_ID: &str = "scroll.inspector";
const SCALE_BAR_ITEM_ID: &str = "inspector-scale-bar";
const AXIS_MIN_INPUT_ID: &str = "inspector.axes.min.input";
const AXIS_MAX_INPUT_ID: &str = "inspector.axes.max.input";
const AXIS_INTERVAL_INPUT_ID: &str = "inspector.axes.interval.input";
const AXIS_TITLE_INPUT_ID: &str = "inspector.axes.title.input";
const MV_CLAMP_MIN_INPUT_ID: &str = "inspector.mv.color_clamp_min.input";
const MV_CLAMP_MAX_INPUT_ID: &str = "inspector.mv.color_clamp_max.input";

/// Transient view state owned by the right-hand inspector shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InspectorViewState {
    pub collapsed_sections: Vec<String>,
    #[serde(default)]
    pub expanded_sections: Vec<String>,
    #[serde(default)]
    pub open_popup: Option<String>,
    #[serde(default)]
    pub selected_point_cursor: usize,
    #[serde(default)]
    pub selected_point_summary: String,
    #[serde(default)]
    pub page_margin_mm: u16,
    #[serde(default)]
    pub scale_bar_mm: u16,
    #[serde(default)]
    pub target_graph_id: Option<String>,
    #[serde(default)]
    pub target_layout_id: Option<String>,
    #[serde(default)]
    pub regression_result_sheet_id: Option<String>,
    #[serde(default)]
    pub regression_all_series: bool,
    #[serde(default)]
    pub regression_confidence_band: bool,
    #[serde(default)]
    pub regression_shaded_ribbon: bool,
    #[serde(default)]
    pub last_canvas_preset: Option<String>,
    #[serde(default)]
    pub selected_fg_level_name: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SectionKind {
    DrillBack,
    FormatGraph,
    FormatAxes,
    Multivariable,
    PairwiseCld,
    PageLayout,
}

impl SectionKind {
    const fn id(self) -> &'static str {
        match self {
            Self::DrillBack => "drill_back",
            Self::FormatGraph => "format_graph",
            Self::FormatAxes => "format_axes",
            Self::Multivariable => "multivariable",
            Self::PairwiseCld => "pairwise",
            Self::PageLayout => "page_layout",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum AxisWhich {
    X,
    Y,
    Y2,
}

fn outcome_ok(payload: &str, cmd_id: Option<&str>) -> UiEventOutcome {
    UiEventOutcome { handled: true, action: format!("inspector.{payload}"), command_id: cmd_id.map(String::from), command_result: None }
}

fn section_for_active_inspector(app: &UiApp, state: &InspectorViewState) -> SectionKind {
    match app.canvas_dialogs.active_inspector {
        ActiveInspector::FormatGraph => SectionKind::FormatGraph,
        ActiveInspector::FormatAxes => SectionKind::FormatAxes,
        ActiveInspector::BubblePlot => SectionKind::Multivariable,
        ActiveInspector::PairwiseComparisons => SectionKind::PairwiseCld,
        ActiveInspector::PageLayout => SectionKind::PageLayout,
        ActiveInspector::ResultDetails => SectionKind::DrillBack,
        ActiveInspector::None => {
            let doc = app.document();
            if let Some(active_id) = &doc.active_sheet_id {
                if doc.layout_sheet(active_id).is_some() {
                    return SectionKind::PageLayout;
                }
                if let Some(gs) = doc.graph_sheet(active_id) {
                    if gs.spec.multivariate.is_some() {
                        return SectionKind::Multivariable;
                    }
                    return SectionKind::FormatGraph;
                }
            }
            if resolve_graph_sheet(doc, state).is_some() {
                SectionKind::FormatGraph
            } else if resolve_layout_sheet(doc, state).is_some() {
                SectionKind::PageLayout
            } else {
                SectionKind::FormatGraph
            }
        }
    }
}

fn is_section_expanded(app: &UiApp, state: &InspectorViewState, sec: SectionKind) -> bool {
    let id = sec.id();
    if state.collapsed_sections.iter().any(|s| s == id) {
        return false;
    }
    let default_sec = section_for_active_inspector(app, state);
    sec == default_sec || sec == SectionKind::DrillBack || state.expanded_sections.iter().any(|s| s == id)
}

fn resolve_graph_sheet<'a>(doc: &'a ProjectDocument, state: &InspectorViewState) -> Option<&'a GraphSheet> {
    if let Some(active) = &doc.active_sheet_id
        && let Some(gs) = doc.graph_sheet(active)
    {
        return Some(gs);
    }
    if let Some(tid) = &state.target_graph_id {
        let sid = SheetId::new(tid);
        if let Some(gs) = doc.graph_sheet(&sid) {
            return Some(gs);
        }
    }
    doc.graph_sheets.first()
}

fn resolve_layout_sheet<'a>(doc: &'a ProjectDocument, state: &InspectorViewState) -> Option<&'a LayoutSheet> {
    if let Some(active) = &doc.active_sheet_id
        && let Some(ls) = doc.layout_sheet(active)
    {
        return Some(ls);
    }
    if let Some(tid) = &state.target_layout_id {
        let sid = SheetId::new(tid);
        if let Some(ls) = doc.layout_sheet(&sid) {
            return Some(ls);
        }
    }
    doc.layout_sheets.first()
}

fn parse_hex_rgba8(hex: &str) -> Option<Rgba8> {
    let raw = hex.trim().trim_start_matches('#');
    if raw.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&raw[0..2], 16).ok()?;
    let g = u8::from_str_radix(&raw[2..4], 16).ok()?;
    let b = u8::from_str_radix(&raw[4..6], 16).ok()?;
    Some(Rgba8::rgb(r, g, b))
}

fn all_marker_shapes() -> &'static [MarkerShape] {
    &[
        MarkerShape::Circle,
        MarkerShape::Square,
        MarkerShape::TriangleUp,
        MarkerShape::TriangleDown,
        MarkerShape::Diamond,
        MarkerShape::Cross,
        MarkerShape::Plus,
        MarkerShape::Star,
    ]
}

fn series_primary_color(s: &SeriesSpec) -> Rgba8 {
    match &s.gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            marker.as_ref().map_or_else(|| line.as_ref().map_or(Rgba8::BLACK, |l| l.color), |m| m.fill_color)
        }
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => {
            if border_color.a > 0 {
                *border_color
            } else {
                *fill_color
            }
        }
        GraphicalDataObject::BoxAndWhisker { fill_color, .. } => *fill_color,
        GraphicalDataObject::Violin { fill_color, .. } => *fill_color,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color,
        GraphicalDataObject::BeforeAfterLines { line_color, .. } => *line_color,
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => *line_color,
        GraphicalDataObject::PartsOfWhole { .. } => Rgba8::BLACK,
    }
}

fn series_marker_style(s: &SeriesSpec) -> MarkerStyle {
    match &s.gdo {
        GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } | GraphicalDataObject::BeforeAfterLines { marker: Some(m), .. } => *m,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => *marker,
        _ => {
            let c = series_primary_color(s);
            MarkerStyle { fill_color: c, stroke_color: c, ..MarkerStyle::default() }
        }
    }
}

fn set_series_primary_color(s: &mut SeriesSpec, rgba: Rgba8) {
    match &mut s.gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill, .. } => {
            if let Some(m) = marker {
                m.fill_color = rgba;
                m.stroke_color = rgba;
            }
            if let Some(l) = line {
                l.color = rgba;
            }
            if let Some(af) = area_fill {
                af.fill_color = rgba.with_alpha(64);
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => {
            *fill_color = rgba.with_alpha(200);
            *border_color = rgba;
        }
        GraphicalDataObject::BoxAndWhisker { fill_color, .. } => {
            *fill_color = rgba.with_alpha(190);
        }
        GraphicalDataObject::Violin { fill_color, stroke_color, .. } => {
            *fill_color = rgba.with_alpha(190);
            *stroke_color = rgba;
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.fill_color = rgba;
            marker.stroke_color = rgba;
        }
        GraphicalDataObject::BeforeAfterLines { line_color, marker, .. } => {
            *line_color = rgba;
            if let Some(m) = marker {
                m.fill_color = rgba;
                m.stroke_color = rgba;
            }
        }
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => {
            *line_color = rgba;
        }
        GraphicalDataObject::PartsOfWhole { .. } => {}
    }
}

fn set_series_marker_style(s: &mut SeriesSpec, next_m: MarkerStyle) {
    match &mut s.gdo {
        GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } => {
            *marker = Some(next_m);
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            *marker = next_m;
        }
        _ => {}
    }
}

fn default_connecting_line(color: Rgba8) -> ConnectingLineStyle {
    ConnectingLineStyle { kind: LineConnectionKind::default(), color, dash_style: LineDashStyle::Solid, stroke_width_pt: 1.5 }
}

fn retopologize_series(series: &[SeriesSpec], topo: &str) -> Vec<SeriesSpec> {
    series
        .iter()
        .map(|s| {
            let mut next = s.clone();
            let col = series_primary_color(s);
            let cur_marker = series_marker_style(s);
            next.gdo = match topo {
                "bar" => GraphicalDataObject::BarWithReplicates {
                    summary: SummaryMeasure::Mean,
                    fill_color: col.with_alpha(200),
                    border_color: col,
                    border_width_pt: 1.0,
                    replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
                },
                "box" => GraphicalDataObject::BoxAndWhisker {
                    whiskers: WhiskerMethod::Tukey1_5Iqr,
                    fill_color: col.with_alpha(190),
                    stroke_color: Rgba8::BLACK,
                    show_outliers: true,
                    show_all_points: false,
                },
                "violin" | "raincloud" => GraphicalDataObject::Violin {
                    fill_color: col.with_alpha(190),
                    stroke_color: col,
                    clip_to_observed_range: true,
                    show_median_and_quartiles: true,
                    half_violin_with_points: topo == "raincloud",
                },
                "dot" => GraphicalDataObject::AlignedDotPlot {
                    summary: SummaryMeasure::Mean,
                    marker: MarkerStyle { fill_color: col, stroke_color: col, ..cur_marker },
                    show_crossbar: true,
                },
                "scatter" => GraphicalDataObject::ScatterAndLine {
                    marker: Some(MarkerStyle { fill_color: col, stroke_color: col, ..cur_marker }),
                    line: Some(default_connecting_line(col)),
                    area_fill: None,
                    replicate_lines: curvebook_doc::ReplicateLineMode::default(),
                },
                _ => s.gdo.clone(),
            };
            next
        })
        .collect()
}

fn resolve_selected_series_index(app: &UiApp, gs: &GraphSheet) -> usize {
    let max_idx = gs.spec.series.len().saturating_sub(1);
    app.canvas_dialogs.format_graph.selected_series_index.min(max_idx)
}

struct MvLevelTarget {
    is_x: bool,
    levels: Vec<String>,
}

fn resolve_mv_level_target(doc: &ProjectDocument, gs: &GraphSheet) -> Option<MvLevelTarget> {
    let enc = gs.spec.multivariate.as_ref()?;
    let ds = doc.data_sheet(&gs.spec.primary_source_sheet_id)?;
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return None;
    };
    if let Some(col) = mv
        .variables
        .get(enc.x_column)
        .filter(|col| col.role == VariableRole::Categorical || matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }))
    {
        let spec = enc.x_level_order_override.as_ref().unwrap_or(&col.level_order);
        let levels: Vec<String> = col.ordered_levels_with(spec).into_iter().map(|l| l.level).collect();
        if !levels.is_empty() {
            return Some(MvLevelTarget { is_x: true, levels });
        }
    }
    if let ColorChannelMapping::Categorical { column, level_order_override, .. } = &enc.color
        && let Some(col) = mv.variables.get(*column)
    {
        let spec = level_order_override.as_ref().unwrap_or(&col.level_order);
        let levels: Vec<String> = col.ordered_levels_with(spec).into_iter().map(|l| l.level).collect();
        if !levels.is_empty() {
            return Some(MvLevelTarget { is_x: false, levels });
        }
    }
    None
}

fn dispatch_format_graph_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let series_count = gs.spec.series.len();
    let sel_idx = resolve_selected_series_index(app, &gs);

    if let Some(target) = sub.strip_prefix("dataset:") {
        if series_count == 0 {
            return Ok(UiEventOutcome::default());
        }
        let next_idx = match target {
            "prev" => (sel_idx + series_count - 1) % series_count,
            "next" => (sel_idx + 1) % series_count,
            raw => raw.parse::<usize>().unwrap_or(sel_idx).min(series_count - 1),
        };
        app.canvas_dialogs.format_graph.selected_series_index = next_idx;
        if let Some(live_gs) = app.session.document().graph_sheet(&gs.id) {
            app.canvas_dialogs.format_graph.sync_from_graph(live_gs);
        }
        state.open_popup = None;
        return Ok(outcome_ok(sub, None));
    }

    if let Some(pal_key) = sub.strip_prefix("palette:") {
        let mut spec = gs.spec.clone();
        let n = spec.series.len().max(1);
        match pal_key {
            "Viridis" => {
                for (i, s) in spec.series.iter_mut().enumerate() {
                    let t = if n <= 1 { 0.5 } else { (i as f64) / ((n - 1) as f64) };
                    set_series_primary_color(s, ColormapId::Viridis.sample(t));
                }
            }
            _ => {
                let pal = match pal_key {
                    "PaulTolBright" => PaletteId::PaulTolBright,
                    "PaulTolMuted" => PaletteId::PaulTolMuted,
                    "PaulTolHighContrast" => PaletteId::PaulTolHighContrast,
                    "PaulTolVibrant" => PaletteId::PaulTolVibrant,
                    "MonochromePrint" => PaletteId::MonochromePrint,
                    _ => PaletteId::OkabeIto,
                };
                spec.palette = pal;
                for (i, s) in spec.series.iter_mut().enumerate() {
                    set_series_primary_color(s, pal.color_at(i));
                }
            }
        }
        app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "palette": spec.palette, "series": spec.series } }))
            .map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setSpec")));
    }

    if let Some(hex) = sub.strip_prefix("color:") {
        let Some(rgba) = parse_hex_rgba8(hex) else {
            return Ok(UiEventOutcome::default());
        };
        let Some(mut series) = gs.spec.series.get(sel_idx).cloned() else {
            return Ok(UiEventOutcome::default());
        };
        set_series_primary_color(&mut series, rgba);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "gdo": series.gdo }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(shape_raw) = sub.strip_prefix("shape:") {
        let Some(mut series) = gs.spec.series.get(sel_idx).cloned() else {
            return Ok(UiEventOutcome::default());
        };
        let mut marker = series_marker_style(&series);
        let next_shape = if shape_raw == "next" {
            let all = all_marker_shapes();
            let pos = all.iter().position(|s| *s == marker.shape).unwrap_or(0);
            all[(pos + 1) % all.len()]
        } else {
            match shape_raw {
                "Circle" => MarkerShape::Circle,
                "Square" => MarkerShape::Square,
                "TriangleUp" => MarkerShape::TriangleUp,
                "TriangleDown" => MarkerShape::TriangleDown,
                "Diamond" => MarkerShape::Diamond,
                "Cross" => MarkerShape::Cross,
                "Plus" => MarkerShape::Plus,
                "Star" => MarkerShape::Star,
                _ => return Ok(UiEventOutcome::default()),
            }
        };
        marker.shape = next_shape;
        set_series_marker_style(&mut series, marker);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "gdo": series.gdo }))
            .map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(size_dir) = sub.strip_prefix("size:") {
        let Some(mut series) = gs.spec.series.get(sel_idx).cloned() else {
            return Ok(UiEventOutcome::default());
        };
        let delta = match size_dir {
            "dec" => -1.0,
            "inc" => 1.0,
            _ => return Ok(UiEventOutcome::default()),
        };
        let mut marker = series_marker_style(&series);
        marker.size_pt = (marker.size_pt + delta).clamp(1.5, 16.0);
        set_series_marker_style(&mut series, marker);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "gdo": series.gdo }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(topo) = sub.strip_prefix("topology:") {
        let next_series = retopologize_series(&gs.spec.series, topo);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series": next_series })).map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if sub == "err_toggle" {
        let Some(series) = gs.spec.series.get(sel_idx) else {
            return Ok(UiEventOutcome::default());
        };
        let next_eb = if series.y_error_bars.is_some() { Value::Null } else { json!(app.canvas_dialogs.format_graph.error_bar_spec) };
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "y_error_bars": next_eb }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(metric_raw) = sub.strip_prefix("err_metric:") {
        let Some(series) = gs.spec.series.get(sel_idx) else {
            return Ok(UiEventOutcome::default());
        };
        let metric = match metric_raw {
            "Sd" => ErrorBarMetric::Sd,
            "Sem" => ErrorBarMetric::Sem,
            "ConfidenceInterval95" => ErrorBarMetric::ConfidenceInterval95,
            "Range" => ErrorBarMetric::Range,
            _ => return Ok(UiEventOutcome::default()),
        };
        let mut eb = series.y_error_bars.unwrap_or(app.canvas_dialogs.format_graph.error_bar_spec);
        eb.metric = metric;
        app.canvas_dialogs.format_graph.error_bar_spec = eb;
        app.canvas_dialogs.format_graph.error_bar_enabled = true;
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "y_error_bars": eb }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(dir_raw) = sub.strip_prefix("err_dir:") {
        let Some(series) = gs.spec.series.get(sel_idx) else {
            return Ok(UiEventOutcome::default());
        };
        let dir = match dir_raw {
            "Both" => ErrorBarDirection::Both,
            "UpperOnly" => ErrorBarDirection::UpperOnly,
            "LowerOnly" => ErrorBarDirection::LowerOnly,
            _ => return Ok(UiEventOutcome::default()),
        };
        let mut eb = series.y_error_bars.unwrap_or(app.canvas_dialogs.format_graph.error_bar_spec);
        eb.direction = dir;
        app.canvas_dialogs.format_graph.error_bar_spec = eb;
        app.canvas_dialogs.format_graph.error_bar_enabled = true;
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "y_error_bars": eb }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(arr_raw) = sub.strip_prefix("arrangement:") {
        let arr = match arr_raw {
            "Interleaved" => GroupedArrangement::Interleaved,
            "Separated" => GroupedArrangement::Separated,
            "Stacked" => GroupedArrangement::Stacked,
            "Superimposed" => GroupedArrangement::Superimposed,
            _ => return Ok(UiEventOutcome::default()),
        };
        app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "grouped_arrangement": arr } }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSpec")));
    }

    if sub == "apply_style_from" {
        let doc = app.document().clone();
        app.canvas_dialogs.open_apply_style_from(&doc, &gs.id);
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("ui.applyStyleFrom.open")));
    }

    if let Some(raw_idx) = sub.strip_prefix("level:select:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(target) = resolve_mv_level_target(app.document(), &gs)
            && let Some(lvl) = target.levels.get(idx)
        {
            state.selected_fg_level_name = Some(lvl.clone());
        }
        return Ok(outcome_ok(sub, None));
    }

    if let Some(move_raw) = sub.strip_prefix("level:move:") {
        let op = match move_raw {
            "top" => LevelMoveOp::Top,
            "up" => LevelMoveOp::Up,
            "down" => LevelMoveOp::Down,
            "end" => LevelMoveOp::End,
            _ => return Ok(UiEventOutcome::default()),
        };
        let Some(mut target) = resolve_mv_level_target(app.document(), &gs) else {
            return Ok(UiEventOutcome::default());
        };
        let mut enc = current_mv_encoding(app.document(), &gs);
        let len = target.levels.len();
        if apply_explicit_level_move(&mut target.levels, &mut state.selected_fg_level_name, op) {
            let new_idx = resolve_selected_level_idx(&target.levels, state.selected_fg_level_name.as_deref());
            let explicit_spec = LevelOrderSpec { order: LevelOrderKind::Explicit(target.levels), reversed: false };
            if target.is_x {
                enc.x_level_order_override = Some(explicit_spec);
            } else if let ColorChannelMapping::Categorical { level_order_override, .. } = &mut enc.color {
                *level_order_override = Some(explicit_spec);
            }
            app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "multivariate": enc })).map_err(|e| e.to_string())?;
            let doc_snap = app.document().clone();
            if let Some(live_gs) = doc_snap.graph_sheet(&gs.id) {
                app.canvas_dialogs.format_graph.sync_from_graph_with_doc(live_gs, Some(&doc_snap));
            }
            if len > 1 {
                match (app.focus_widget_id.as_deref(), new_idx) {
                    (Some("inspector.fg.level_top"), Some(0)) => app.focus_widget_id = Some(String::from("inspector.fg.level_end")),
                    (Some("inspector.fg.level_up"), Some(0)) => app.focus_widget_id = Some(String::from("inspector.fg.level_dn")),
                    (Some("inspector.fg.level_dn"), Some(i)) if i + 1 >= len => app.focus_widget_id = Some(String::from("inspector.fg.level_up")),
                    (Some("inspector.fg.level_end"), Some(i)) if i + 1 >= len => app.focus_widget_id = Some(String::from("inspector.fg.level_top")),
                    _ => {}
                }
            }
            return Ok(outcome_ok(sub, Some("graph.setSpec")));
        }
        return Ok(outcome_ok(sub, None));
    }

    Ok(UiEventOutcome::default())
}

fn dispatch_regression_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let sel_idx = resolve_selected_series_index(app, &gs);
    let cur_ov = gs
        .spec
        .series
        .get(sel_idx)
        .and_then(|s| s.regression_overlay.as_ref())
        .or_else(|| gs.spec.series.iter().find_map(|s| s.regression_overlay.as_ref()));
    let candidates: Vec<String> =
        app.document().result_sheets.iter().filter(|rs| !rs.fitted_curves.is_empty()).map(|rs| rs.id.as_str().to_string()).collect();

    let mut res_id = cur_ov.map(|o| o.result_sheet_id.as_str().to_string()).or_else(|| state.regression_result_sheet_id.clone());
    let mut all_series = if cur_ov.is_some() && !gs.spec.series.is_empty() {
        gs.spec.series.iter().all(|s| s.regression_overlay.is_some()) || state.regression_all_series
    } else {
        state.regression_all_series
    };
    let mut conf_band = cur_ov.map_or(state.regression_confidence_band, |o| o.show_confidence_band);
    let mut shaded_ribbon = cur_ov.map_or(state.regression_shaded_ribbon, |o| o.shaded_ribbon);

    let apply_now = match sub {
        "cycle_result" => {
            let mut ring: Vec<Option<String>> = candidates.iter().cloned().map(Some).collect();
            ring.push(None);
            let cur_pos = ring.iter().position(|entry| entry.as_deref() == res_id.as_deref()).unwrap_or(ring.len().saturating_sub(1));
            res_id = ring[(cur_pos + 1) % ring.len()].clone();
            state.regression_result_sheet_id = res_id.clone();
            true
        }
        "all_series" => {
            all_series = !all_series;
            state.regression_all_series = all_series;
            res_id.is_some()
        }
        "confidence_band" => {
            conf_band = !conf_band;
            state.regression_confidence_band = conf_band;
            res_id.is_some()
        }
        "shaded_ribbon" => {
            shaded_ribbon = !shaded_ribbon;
            state.regression_shaded_ribbon = shaded_ribbon;
            res_id.is_some()
        }
        "apply" => {
            if res_id.is_none() {
                res_id = candidates.first().cloned();
                state.regression_result_sheet_id = res_id.clone();
            }
            true
        }
        _ => return Ok(UiEventOutcome::default()),
    };

    if !apply_now || gs.spec.series.is_empty() {
        return Ok(outcome_ok(sub, None));
    }
    let payload = if let Some(rid) = res_id {
        json!({
            "sheet_id": gs.id.as_str(),
            "result_sheet_id": rid,
            "series_index": sel_idx,
            "all_series": all_series,
            "show_confidence_band": conf_band,
            "shaded_ribbon": shaded_ribbon,
        })
    } else {
        json!({
            "sheet_id": gs.id.as_str(),
            "clear": true,
            "series_index": sel_idx,
            "all_series": all_series,
        })
    };
    app.execute_command("graph.setRegressionOverlay", payload).map_err(|e| e.to_string())?;
    Ok(outcome_ok(sub, Some("graph.setRegressionOverlay")))
}

fn current_axis_which(app: &UiApp) -> AxisWhich {
    use crate::dialogs_canvas::AxisTarget;
    match app.canvas_dialogs.format_axes.target_axis {
        AxisTarget::X => AxisWhich::X,
        AxisTarget::Y => AxisWhich::Y,
        AxisTarget::Y2 => AxisWhich::Y2,
    }
}

fn axis_which_key(which: AxisWhich) -> &'static str {
    match which {
        AxisWhich::X => "x_axis",
        AxisWhich::Y => "y_axis",
        AxisWhich::Y2 => "y2_axis",
    }
}

fn read_live_axis_spec(gs: &GraphSheet, which: AxisWhich) -> AxisSpec {
    match which {
        AxisWhich::X => gs.spec.x_axis.clone(),
        AxisWhich::Y => gs.spec.y_axis.clone(),
        AxisWhich::Y2 => gs.spec.y2_axis.clone().unwrap_or_else(|| gs.spec.y_axis.clone()),
    }
}

fn axis_primary_seg(axis: &AxisSpec) -> AxisSegment {
    axis.segments.first().cloned().unwrap_or_default()
}

fn clear_axis_input_buffers(app: &mut UiApp) {
    app.interaction.text_field_buffers.remove(AXIS_MIN_INPUT_ID);
    app.interaction.text_field_buffers.remove(AXIS_MAX_INPUT_ID);
    app.interaction.text_field_buffers.remove(AXIS_INTERVAL_INPUT_ID);
}

fn build_segments_for_count(axis: &AxisSpec, count: usize) -> Vec<AxisSegment> {
    let base = axis_primary_seg(axis);
    let lo = base.min.unwrap_or(0.0);
    let hi = base.max.unwrap_or(100.0);
    let (a, b) = if hi > lo { (lo, hi) } else { (0.0, 100.0) };
    let span = b - a;
    if count <= 1 {
        return vec![AxisSegment {
            min: base.min,
            max: base.max,
            scale: base.scale,
            major_tick_interval: base.major_tick_interval,
            minor_ticks_per_interval: base.minor_ticks_per_interval,
            length_share: 1.0,
        }];
    }
    if count == 2 {
        vec![
            AxisSegment {
                min: Some(a),
                max: Some(a + span * 0.35),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.48,
            },
            AxisSegment {
                min: Some(a + span * 0.65),
                max: Some(b),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.48,
            },
        ]
    } else {
        vec![
            AxisSegment {
                min: Some(a),
                max: Some(a + span * 0.25),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.32,
            },
            AxisSegment {
                min: Some(a + span * 0.40),
                max: Some(a + span * 0.62),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.32,
            },
            AxisSegment {
                min: Some(a + span * 0.78),
                max: Some(b),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.32,
            },
        ]
    }
}

fn commit_axis_update(app: &mut UiApp, gs: &GraphSheet, which: AxisWhich, next_axis: AxisSpec) -> Result<(), String> {
    let mut payload = json!({ "sheet_id": gs.id.as_str() });
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(String::from(axis_which_key(which)), json!(next_axis));
    }
    app.execute_command("graph.setAxes", payload).map_err(|e| e.to_string())?;
    clear_axis_input_buffers(app);
    Ok(())
}

fn dispatch_axes_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    use crate::dialogs_canvas::AxisTarget;

    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());

    if let Some(target) = sub.strip_prefix("target:") {
        let next = match target {
            "x" => AxisTarget::X,
            "y" => AxisTarget::Y,
            "y2" => AxisTarget::Y2,
            _ => return Ok(UiEventOutcome::default()),
        };
        app.canvas_dialogs.format_axes.target_axis = next;
        app.canvas_dialogs.format_axes.sync_from_graph(&gs, next);
        clear_axis_input_buffers(app);
        app.interaction.text_field_buffers.remove(AXIS_TITLE_INPUT_ID);
        state.open_popup = None;
        return Ok(outcome_ok(sub, None));
    }

    let which = current_axis_which(app);
    let mut axis = read_live_axis_spec(&gs, which);
    let mut seg = axis_primary_seg(&axis);

    if sub == "apply_title" {
        if let Some(buf) = app.interaction.text_field_buffers.get(AXIS_TITLE_INPUT_ID) {
            axis.title = buf.text.trim().to_owned();
        }
        commit_axis_update(app, &gs, which, axis)?;
        app.interaction.text_field_buffers.remove(AXIS_TITLE_INPUT_ID);
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if sub == "y2_toggle" {
        let y2_val = if gs.spec.y2_axis.is_some() { Value::Null } else { json!(gs.spec.y_axis) };
        app.execute_command("graph.setAxes", json!({ "sheet_id": gs.id.as_str(), "y2_axis": y2_val })).map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if let Some(scale_raw) = sub.strip_prefix("scale:") {
        let scale = match scale_raw {
            "Linear" => AxisScale::Linear,
            "Log10" => AxisScale::Log10,
            "Log2" => AxisScale::Log2,
            "Ln" => AxisScale::Ln,
            "Logit" => AxisScale::Logit(LogitDomain::Percent),
            "Probit" => AxisScale::Probit,
            "Reciprocal" => AxisScale::Reciprocal,
            "Nonlinear" => match seg.scale {
                AxisScale::Ln => AxisScale::Logit(LogitDomain::Percent),
                AxisScale::Logit(_) => AxisScale::Probit,
                AxisScale::Probit => AxisScale::Reciprocal,
                _ => AxisScale::Ln,
            },
            _ => return Ok(UiEventOutcome::default()),
        };
        if axis.segments.is_empty() {
            axis.segments.push(seg.clone());
        }
        for s in &mut axis.segments {
            s.scale = scale;
        }
        commit_axis_update(app, &gs, which, axis)?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if let Some(cnt_raw) = sub.strip_prefix("breaks:") {
        let count = cnt_raw.parse::<usize>().unwrap_or(1).clamp(1, 3);
        axis.segments = build_segments_for_count(&axis, count);
        commit_axis_update(app, &gs, which, axis)?;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if let Some(orn_raw) = sub.strip_prefix("ornament:") {
        axis.break_ornament = match orn_raw {
            "DoubleSlash" => BreakOrnament::DoubleSlash,
            "Zigzag" => BreakOrnament::Zigzag,
            "BlankGap" => BreakOrnament::BlankGap,
            _ => return Ok(UiEventOutcome::default()),
        };
        if axis.segments.len() < 2 {
            axis.segments = build_segments_for_count(&axis, 2);
        }
        commit_axis_update(app, &gs, which, axis)?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if sub == "auto_range" {
        seg.min = None;
        seg.max = None;
        seg.major_tick_interval = None;
        axis.segments = vec![seg];
        commit_axis_update(app, &gs, which, axis)?;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if sub == "apply_range" {
        let parse_buf = |wid_str: &str, fallback: Option<f64>| -> Option<f64> {
            if let Some(buf) = app.interaction.text_field_buffers.get(wid_str) {
                let trimmed = buf.text.trim();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
                    None
                } else {
                    crate::grid_view::parse_localized_number_input(trimmed, &app.locale_preset)
                        .or_else(|| trimmed.parse::<f64>().ok())
                        .filter(|v| v.is_finite())
                        .or(fallback)
                }
            } else {
                fallback
            }
        };
        let next_min = parse_buf(AXIS_MIN_INPUT_ID, seg.min);
        let next_max = parse_buf(AXIS_MAX_INPUT_ID, seg.max);
        let next_iv = parse_buf(AXIS_INTERVAL_INPUT_ID, seg.major_tick_interval).filter(|v| *v > 0.0);
        if let (Some(lo), Some(hi)) = (next_min, next_max)
            && hi <= lo
        {
            return Ok(UiEventOutcome::default());
        }
        seg.min = next_min;
        seg.max = next_max;
        seg.major_tick_interval = next_iv;
        if axis.segments.is_empty() {
            axis.segments.push(seg);
        } else {
            axis.segments[0] = seg;
        }
        commit_axis_update(app, &gs, which, axis)?;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    let step = {
        let lo = seg.min.unwrap_or(0.0);
        let hi = seg.max.unwrap_or(100.0);
        let span = (hi - lo).abs();
        if span > 0.0 { (span / 5.0).max(1.0) } else { 5.0 }
    };

    match sub {
        "min:dec" => {
            seg.min = Some(seg.min.unwrap_or(0.0) - step);
        }
        "min:inc" => {
            let next = seg.min.unwrap_or(0.0) + step;
            if seg.max.is_none_or(|m| next < m) {
                seg.min = Some(next);
            }
        }
        "max:dec" => {
            let next = seg.max.unwrap_or(100.0) - step;
            if seg.min.is_none_or(|m| next > m) {
                seg.max = Some(next);
            }
        }
        "max:inc" => {
            seg.max = Some(seg.max.unwrap_or(100.0) + step);
        }
        "interval:dec" => {
            let cur = seg.major_tick_interval.unwrap_or(step);
            seg.major_tick_interval = Some((cur * 0.5).max(0.5));
        }
        "interval:inc" => {
            let cur = seg.major_tick_interval.unwrap_or(step);
            seg.major_tick_interval = Some((cur * 2.0).max(1.0));
        }
        _ => return Ok(UiEventOutcome::default()),
    }
    if axis.segments.is_empty() {
        axis.segments.push(seg);
    } else {
        axis.segments[0] = seg;
    }
    commit_axis_update(app, &gs, which, axis)?;
    Ok(outcome_ok(sub, Some("graph.setAxes")))
}

fn source_columns_for_graph(doc: &ProjectDocument, gs: &GraphSheet) -> Vec<(usize, String)> {
    let Some(ds) = doc.data_sheet(&gs.spec.primary_source_sheet_id) else {
        return Vec::new();
    };
    let count = ds.archetype.column_count();
    (0..count)
        .map(|idx| {
            let title = data_column_title(ds, idx).map(str::to_owned).unwrap_or_else(|| format!("Col {}", idx + 1));
            (idx, title)
        })
        .collect()
}

fn current_mv_encoding(doc: &ProjectDocument, gs: &GraphSheet) -> MultivariateGraphEncoding {
    if let Some(enc) = &gs.spec.multivariate {
        return enc.clone();
    }
    let cols = source_columns_for_graph(doc, gs);
    let x_column = 0usize;
    let y_column = cols.get(1).or_else(|| cols.first()).map_or(0, |(c, _)| *c);
    MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column,
        y_column,
        color: ColorChannelMapping::Constant(gs.spec.palette.color_at(0)),
        size: SizeChannelMapping::Constant { radius_pt: 4.5 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    }
}

fn default_mv_categorical_summary(gs: &GraphSheet, summary: SummaryMeasure) -> MultivariatePlotMode {
    MultivariatePlotMode::CategoricalSummary {
        gdo: GraphicalDataObject::BarWithReplicates {
            summary,
            fill_color: gs.spec.palette.color_at(0),
            border_color: Rgba8::BLACK,
            border_width_pt: 0.75,
            replicate_overlay: None,
        },
        y_error_bars: Some(ErrorBarSpec::default()),
    }
}

fn dispatch_multivariable_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let mut enc = current_mv_encoding(app.document(), &gs);

    if let Some(mode) = sub.strip_prefix("mode:") {
        match mode {
            "bubble" => enc.plot_mode = MultivariatePlotMode::BubbleScatter,
            "cat_summary" => {
                if matches!(enc.plot_mode, MultivariatePlotMode::BubbleScatter) {
                    enc.plot_mode = default_mv_categorical_summary(&gs, SummaryMeasure::Mean);
                    if let Some(TableArchetype::MultipleVariables(mv)) =
                        app.document().data_sheet(&gs.spec.primary_source_sheet_id).map(|ds| &ds.archetype)
                    {
                        let x_is_cat = mv.variables.get(enc.x_column).is_some_and(|v| v.role == VariableRole::Categorical);
                        if !x_is_cat
                            && let (Some(cat_col), Some(cont_col)) = (
                                mv.variables.iter().position(|v| v.role == VariableRole::Categorical),
                                mv.variables
                                    .iter()
                                    .position(|v| v.role == VariableRole::Continuous)
                                    .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Calculated)),
                            )
                        {
                            enc.x_column = cat_col;
                            enc.y_column = cont_col;
                        }
                    }
                }
            }
            _ => return Ok(UiEventOutcome::default()),
        }
    } else if let Some(m_raw) = sub.strip_prefix("metric:") {
        let next_m = match m_raw {
            "Mean" => SummaryMeasure::Mean,
            "Median" => SummaryMeasure::Median,
            "Count" => SummaryMeasure::GeometricMean,
            _ => return Ok(UiEventOutcome::default()),
        };
        match &mut enc.plot_mode {
            MultivariatePlotMode::BubbleScatter => enc.plot_mode = default_mv_categorical_summary(&gs, next_m),
            MultivariatePlotMode::CategoricalSummary { gdo, .. } => match gdo {
                GraphicalDataObject::BarWithReplicates { summary, .. } | GraphicalDataObject::AlignedDotPlot { summary, .. } => *summary = next_m,
                GraphicalDataObject::FloatingBar { summary, .. } => *summary = Some(next_m),
                other => {
                    *other = GraphicalDataObject::BarWithReplicates {
                        summary: next_m,
                        fill_color: gs.spec.palette.color_at(0),
                        border_color: Rgba8::BLACK,
                        border_width_pt: 0.75,
                        replicate_overlay: None,
                    };
                }
            },
        }
    } else if sub == "clamp_apply" {
        let parse_clamp = |wid_str: &str| -> Option<f64> {
            let raw = app.interaction.text_field_buffers.get(wid_str)?.text.trim().to_owned();
            if raw.is_empty() {
                return None;
            }
            crate::grid_view::parse_localized_number_input(&raw, &app.locale_preset).or_else(|| raw.parse::<f64>().ok()).filter(|v| v.is_finite())
        };
        let lo = parse_clamp(MV_CLAMP_MIN_INPUT_ID);
        let hi = parse_clamp(MV_CLAMP_MAX_INPUT_ID);
        let next_clamp = match (lo, hi) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            (None, None) => None,
            _ => return Ok(UiEventOutcome::default()),
        };
        match &mut enc.color {
            ColorChannelMapping::Continuous { domain_clamp, .. } => *domain_clamp = next_clamp,
            _ => {
                let cur_col = match &enc.color {
                    ColorChannelMapping::Categorical { column, .. } => *column,
                    ColorChannelMapping::Constant(_) | ColorChannelMapping::Continuous { .. } => {
                        source_columns_for_graph(app.document(), &gs).last().map_or(0, |(c, _)| *c)
                    }
                };
                enc.color = ColorChannelMapping::Continuous {
                    column: cur_col,
                    ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
                    domain_clamp: next_clamp,
                };
            }
        }
        app.interaction.text_field_buffers.remove(MV_CLAMP_MIN_INPUT_ID);
        app.interaction.text_field_buffers.remove(MV_CLAMP_MAX_INPUT_ID);
    } else if let Some(raw) = sub.strip_prefix("x:") {
        if let Ok(col) = raw.parse::<usize>() {
            enc.x_column = col;
        }
    } else if let Some(raw) = sub.strip_prefix("y:") {
        if let Ok(col) = raw.parse::<usize>() {
            enc.y_column = col;
        }
    } else if let Some(raw) = sub.strip_prefix("color:") {
        if raw == "none" {
            enc.color = ColorChannelMapping::Constant(gs.spec.palette.color_at(0));
        } else if let Ok(col) = raw.parse::<usize>() {
            enc.color = ColorChannelMapping::Categorical { column: col, palette: gs.spec.palette, level_order_override: None };
        }
    } else if let Some(mode) = sub.strip_prefix("color_mode:") {
        let cur_col = match &enc.color {
            ColorChannelMapping::Categorical { column, .. } | ColorChannelMapping::Continuous { column, .. } => *column,
            ColorChannelMapping::Constant(_) => source_columns_for_graph(app.document(), &gs).last().map_or(0, |(c, _)| *c),
        };
        enc.color = match mode {
            "cat" => ColorChannelMapping::Categorical { column: cur_col, palette: gs.spec.palette, level_order_override: None },
            "cont" => ColorChannelMapping::Continuous {
                column: cur_col,
                ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
                domain_clamp: None,
            },
            _ => return Ok(UiEventOutcome::default()),
        };
    } else if let Some(raw) = sub.strip_prefix("size:") {
        if raw == "none" {
            enc.size = SizeChannelMapping::Constant { radius_pt: 4.5 };
        } else if let Ok(col) = raw.parse::<usize>() {
            enc.size = SizeChannelMapping::Variable { column: col, min_radius_pt: 2.5, max_radius_pt: 9.5 };
        }
    } else if let Some(raw) = sub.strip_prefix("group:") {
        if raw == "none" {
            enc.label_column = None;
        } else if let Ok(col) = raw.parse::<usize>() {
            enc.label_column = Some(col);
        }
    } else if let Some(rest) = sub.strip_prefix("role:")
        && let Some((raw_col, role)) = rest.split_once(':')
        && let Ok(col) = raw_col.parse::<usize>()
    {
        let src_id = gs.spec.primary_source_sheet_id.clone();
        app.execute_command("table.setVariableRole", json!({ "sheet_id": src_id.as_str(), "column": col, "role": role }))
            .map_err(|e| e.to_string())?;
        if let Some(ds) = app.document().data_sheet(&src_id).cloned()
            && app.grid.variables_dialog.open
        {
            app.grid.variables_dialog.sync_from_sheet(&ds);
        }
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("table.setVariableRole")));
    } else if sub == "ellipse:toggle" {
        enc.show_confidence_ellipses = !enc.show_confidence_ellipses;
    } else if sub == "hull:toggle" {
        enc.show_convex_hulls = !enc.show_convex_hulls;
    } else {
        return Ok(UiEventOutcome::default());
    }

    app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "multivariate": enc })).map_err(|e| e.to_string())?;
    state.open_popup = None;
    Ok(outcome_ok(sub, Some("graph.setSpec")))
}

fn find_pairwise_result_sheet_id(doc: &ProjectDocument, gs: &GraphSheet) -> Option<SheetId> {
    if let Some(cld) = &gs.spec.compact_letter_display
        && doc.result_sheet(&cld.result_sheet_id).is_some()
    {
        return Some(cld.result_sheet_id.clone());
    }
    if let Some(br) = gs.spec.comparisons.first()
        && doc.result_sheet(&br.result_sheet_id).is_some()
    {
        return Some(br.result_sheet_id.clone());
    }
    doc.result_sheets
        .iter()
        .rev()
        .find(|rs| {
            matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(_) | AnalysisKind::FactorialAnova(_) | AnalysisKind::NestedComparison(_))
                || !rs.pairwise_comparisons.is_empty()
        })
        .or_else(|| doc.result_sheets.last())
        .map(|rs| rs.id.clone())
}

fn cld_or_default(gs: &GraphSheet, rid: &SheetId) -> CompactLetterDisplaySpec {
    gs.spec.compact_letter_display.clone().unwrap_or_else(|| CompactLetterDisplaySpec::new(rid.clone()))
}

fn is_cld_active(gs: &GraphSheet) -> bool {
    gs.spec.compact_letter_display.is_some()
        && matches!(
            gs.spec.pairwise_display_mode,
            PairwiseDisplayMode::CompactLetterDisplayOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay
        )
}

fn dispatch_pairwise_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let mut pw = app.canvas_dialogs.pairwise.clone();
    pw.sync_from_graph(&gs, app.document());

    match sub {
        "auto_brackets" => {
            let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
                return Ok(UiEventOutcome::default());
            };
            app.execute_command(
                "graph.addComparisonBrackets",
                json!({
                    "sheet_id": gs.id.as_str(),
                    "result_sheet_id": rid.as_str(),
                    "clear_existing": true,
                    "style": pw.bracket_style,
                    "label_mode": pw.label_mode,
                    "hide_if_non_significant": pw.hide_non_significant,
                    "significant_only": pw.hide_non_significant,
                }),
            )
            .map_err(|e| e.to_string())?;
            return Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")));
        }
        "clear_brackets" => {
            app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "comparisons": [] } })).map_err(|e| e.to_string())?;
            return Ok(outcome_ok(sub, Some("graph.setSpec")));
        }
        "sig_only:toggle" => {
            pw.hide_non_significant = !pw.hide_non_significant;
            app.canvas_dialogs.pairwise = pw.clone();
            if let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs)
                && !gs.spec.comparisons.is_empty()
            {
                app.execute_command(
                    "graph.addComparisonBrackets",
                    json!({
                        "sheet_id": gs.id.as_str(),
                        "result_sheet_id": rid.as_str(),
                        "clear_existing": true,
                        "style": pw.bracket_style,
                        "label_mode": pw.label_mode,
                        "hide_if_non_significant": pw.hide_non_significant,
                        "significant_only": pw.hide_non_significant,
                    }),
                )
                .map_err(|e| e.to_string())?;
            }
            return Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")));
        }
        "cld_toggle" => {
            let next_enabled = !is_cld_active(&gs);
            let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
                return Ok(UiEventOutcome::default());
            };
            let cld = cld_or_default(&gs, &rid);
            let next_mode = if !next_enabled {
                PairwiseDisplayMode::BracketsOnly
            } else if gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly {
                PairwiseDisplayMode::CompactLetterDisplayOnly
            } else {
                gs.spec.pairwise_display_mode
            };
            app.execute_command(
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": gs.id.as_str(),
                    "enabled": next_enabled,
                    "result_sheet_id": rid.as_str(),
                    "pairwise_display_mode": next_mode,
                    "ordering": cld.ordering,
                    "uppercase": cld.uppercase,
                }),
            )
            .map_err(|e| e.to_string())?;
            return Ok(outcome_ok(sub, Some("graph.setCompactLetterDisplay")));
        }
        "cld_case:toggle" => {
            let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
                return Ok(UiEventOutcome::default());
            };
            let cld = cld_or_default(&gs, &rid);
            let next_mode = if gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly {
                PairwiseDisplayMode::CompactLetterDisplayOnly
            } else {
                gs.spec.pairwise_display_mode
            };
            app.execute_command(
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": gs.id.as_str(),
                    "enabled": true,
                    "result_sheet_id": rid.as_str(),
                    "pairwise_display_mode": next_mode,
                    "ordering": cld.ordering,
                    "uppercase": !cld.uppercase,
                }),
            )
            .map_err(|e| e.to_string())?;
            return Ok(outcome_ok(sub, Some("graph.setCompactLetterDisplay")));
        }
        _ => {}
    }

    if let Some(lbl_raw) = sub.strip_prefix("label_mode:") {
        let mode = match lbl_raw {
            "stars" => BracketLabelMode::SummaryStars,
            "numeric" => BracketLabelMode::NumericPOrQ,
            _ => return Ok(UiEventOutcome::default()),
        };
        pw.label_mode = mode;
        app.canvas_dialogs.pairwise = pw.clone();
        if let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) {
            app.execute_command(
                "graph.addComparisonBrackets",
                json!({
                    "sheet_id": gs.id.as_str(),
                    "result_sheet_id": rid.as_str(),
                    "clear_existing": true,
                    "style": pw.bracket_style,
                    "label_mode": lbl_raw,
                    "hide_if_non_significant": pw.hide_non_significant,
                    "significant_only": pw.hide_non_significant,
                }),
            )
            .map_err(|e| e.to_string())?;
        }
        return Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")));
    }

    if let Some(mode_raw) = sub.strip_prefix("cld_mode:") {
        let mode = match mode_raw {
            "BracketsOnly" => PairwiseDisplayMode::BracketsOnly,
            "CompactLetterDisplayOnly" => PairwiseDisplayMode::CompactLetterDisplayOnly,
            "BracketsAndCompactLetterDisplay" => PairwiseDisplayMode::BracketsAndCompactLetterDisplay,
            _ => return Ok(UiEventOutcome::default()),
        };
        let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
            return Ok(UiEventOutcome::default());
        };
        let cld = cld_or_default(&gs, &rid);
        let enabled = !matches!(mode, PairwiseDisplayMode::BracketsOnly);
        app.execute_command(
            "graph.setCompactLetterDisplay",
            json!({
                "sheet_id": gs.id.as_str(),
                "enabled": enabled,
                "result_sheet_id": rid.as_str(),
                "pairwise_display_mode": mode,
                "ordering": cld.ordering,
                "uppercase": cld.uppercase,
            }),
        )
        .map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setCompactLetterDisplay")));
    }

    if let Some(ord_raw) = sub.strip_prefix("cld_order:") {
        let ordering = match ord_raw {
            "AscendingMean" => CldOrdering::AscendingMean,
            "DescendingMean" => CldOrdering::DescendingMean,
            _ => return Ok(UiEventOutcome::default()),
        };
        let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
            return Ok(UiEventOutcome::default());
        };
        let cld = cld_or_default(&gs, &rid);
        let next_mode = if gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly {
            PairwiseDisplayMode::CompactLetterDisplayOnly
        } else {
            gs.spec.pairwise_display_mode
        };
        app.execute_command(
            "graph.setCompactLetterDisplay",
            json!({
                "sheet_id": gs.id.as_str(),
                "enabled": true,
                "result_sheet_id": rid.as_str(),
                "pairwise_display_mode": next_mode,
                "ordering": ordering,
                "uppercase": cld.uppercase,
            }),
        )
        .map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setCompactLetterDisplay")));
    }

    Ok(UiEventOutcome::default())
}

fn mm_to_pt(mm: u16) -> f64 {
    f64::from(mm) * 72.0 / 25.4
}

fn layout_margin_mm(state: &InspectorViewState) -> u16 {
    if state.page_margin_mm == 0 { 12 } else { state.page_margin_mm.clamp(4, 40) }
}

fn layout_scale_bar_mm(state: &InspectorViewState) -> u16 {
    if state.scale_bar_mm == 0 { 10 } else { state.scale_bar_mm }
}

fn has_scale_bar_item(spec: &LayoutSpec) -> bool {
    spec.items.iter().any(|item| matches!(item, LayoutItem::TextCallout { id, .. } if id == SCALE_BAR_ITEM_ID))
}

fn layout_size_pt(spec: &LayoutSpec) -> (f64, f64) {
    let scale = match spec.unit {
        PhysicalUnit::Millimeters => 72.0 / 25.4,
        PhysicalUnit::Inches => 72.0,
        PhysicalUnit::Centimeters => 72.0 / 2.54,
        PhysicalUnit::Points => 1.0,
    };
    (spec.width * scale, spec.height * scale)
}

fn rearrange_layout_grid(spec: &mut LayoutSpec, rows: u8, cols: u8, margin_pt: f64, gutter_pt: f64) {
    let rows = rows.max(1);
    let cols = cols.max(1);
    spec.grid_template = Some((rows, cols));
    let (page_w_pt, page_h_pt) = layout_size_pt(spec);
    let usable_w = (page_w_pt - 2.0 * margin_pt - f64::from(cols.saturating_sub(1)) * gutter_pt).max(36.0);
    let usable_h = (page_h_pt - 2.0 * margin_pt - f64::from(rows.saturating_sub(1)) * gutter_pt).max(36.0);
    let cell_w = usable_w / f64::from(cols);
    let cell_h = usable_h / f64::from(rows);

    let mut panel_idx = 0usize;
    for item in &mut spec.items {
        if let LayoutItem::LiveGraph { bounds_pt, .. } = item {
            let r = (panel_idx / usize::from(cols)) % usize::from(rows);
            let c = panel_idx % usize::from(cols);
            *bounds_pt =
                RectPt::new(margin_pt + (c as f64) * (cell_w + gutter_pt), margin_pt + (r as f64) * (cell_h + gutter_pt), cell_w, cell_h).into();
            panel_idx += 1;
        }
    }
}

fn upsert_scale_bar_item(spec: &mut LayoutSpec, margin_pt: f64, length_mm: u16, enabled: bool) {
    spec.items.retain(|it| !matches!(it, LayoutItem::TextCallout { id, .. } if id == SCALE_BAR_ITEM_ID));
    if !enabled {
        return;
    }
    let (page_w_pt, page_h_pt) = layout_size_pt(spec);
    let bar_w_pt = mm_to_pt(length_mm).clamp(24.0, (page_w_pt * 0.4).max(36.0));
    let bar_h_pt = 18.0;
    let x = (page_w_pt - margin_pt - bar_w_pt).max(margin_pt);
    let y = (page_h_pt - margin_pt - bar_h_pt).max(margin_pt);
    spec.items.push(LayoutItem::TextCallout {
        id: String::from(SCALE_BAR_ITEM_ID),
        bounds_pt: RectPt::new(x, y, bar_w_pt, bar_h_pt).into(),
        markdown: format!("|==== {length_mm} mm ====|"),
        font_size_pt: 8.5,
    });
}

fn commit_layout_spec(app: &mut UiApp, ls: &LayoutSheet, spec: &LayoutSpec) -> Result<(), String> {
    app.execute_command(
        "layout.setSpec",
        json!({
            "sheet_id": ls.id.as_str(),
            "align_plot_origins": spec.align_plot_origins,
            "spec": spec,
        }),
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn dispatch_layout_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(ls) = resolve_layout_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_layout_id = Some(ls.id.as_str().to_string());
    let mut spec = ls.spec.clone();
    let (cur_rows, cur_cols) = spec.grid_template.unwrap_or((1, 2));

    if let Some(grid_raw) = sub.strip_prefix("grid:") {
        let (rows, cols) = match grid_raw {
            "1x1" => (1u8, 1u8),
            "1x2" => (1, 2),
            "2x1" => (2, 1),
            "2x2" => (2, 2),
            "2x3" => (2, 3),
            _ => return Ok(UiEventOutcome::default()),
        };
        let margin_pt = mm_to_pt(layout_margin_mm(state));
        rearrange_layout_grid(&mut spec, rows, cols, margin_pt, 12.0);
        commit_layout_spec(app, &ls, &spec)?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    if let Some(m_raw) = sub.strip_prefix("margin:") {
        let cur_mm = layout_margin_mm(state);
        let next_mm = match m_raw {
            "dec" => cur_mm.saturating_sub(2).max(4),
            "inc" => cur_mm.saturating_add(2).min(40),
            val => val.parse::<u16>().unwrap_or(cur_mm).clamp(4, 40),
        };
        state.page_margin_mm = next_mm;
        let margin_pt = mm_to_pt(next_mm);
        rearrange_layout_grid(&mut spec, cur_rows, cur_cols, margin_pt, 12.0);
        if has_scale_bar_item(&spec) {
            upsert_scale_bar_item(&mut spec, margin_pt, layout_scale_bar_mm(state), true);
        }
        commit_layout_spec(app, &ls, &spec)?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    if sub == "align_origins:toggle" {
        let next_align = !spec.align_plot_origins;
        app.execute_command("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "align_plot_origins": next_align })).map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    if sub == "scale_bar:toggle" {
        let next_on = !has_scale_bar_item(&spec);
        let margin_pt = mm_to_pt(layout_margin_mm(state));
        upsert_scale_bar_item(&mut spec, margin_pt, layout_scale_bar_mm(state), next_on);
        commit_layout_spec(app, &ls, &spec)?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    if let Some(sb_raw) = sub.strip_prefix("scale_bar_mm:") {
        let next_mm = sb_raw.parse::<u16>().unwrap_or(10).clamp(1, 200);
        state.scale_bar_mm = next_mm;
        let margin_pt = mm_to_pt(layout_margin_mm(state));
        upsert_scale_bar_item(&mut spec, margin_pt, next_mm, true);
        commit_layout_spec(app, &ls, &spec)?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    Ok(UiEventOutcome::default())
}

fn resolve_renderable_canvas_target(doc: &ProjectDocument, state: &InspectorViewState) -> Option<(SheetId, bool)> {
    if let Some(active) = &doc.active_sheet_id {
        if doc.layout_sheet(active).is_some() {
            return Some((active.clone(), true));
        }
        if doc.graph_sheet(active).is_some() {
            return Some((active.clone(), false));
        }
    }
    if let Some(ls) = resolve_layout_sheet(doc, state) {
        return Some((ls.id.clone(), true));
    }
    resolve_graph_sheet(doc, state).map(|gs| (gs.id.clone(), false))
}

fn dispatch_canvas_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let apply_preset_for_target = |app: &mut UiApp, state: &mut InspectorViewState, preset: FigureCanvasPreset| -> Result<bool, String> {
        let doc_snap = app.document().clone();
        let Some((sheet_id, is_layout)) = resolve_renderable_canvas_target(&doc_snap, state) else {
            return Ok(false);
        };
        let intent = app.canvas_dialogs.apply_canvas_preset_intent(&sheet_id, is_layout, preset);
        app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
        Ok(true)
    };

    if let Some(preset_raw) = sub.strip_prefix("preset:") {
        let Some(preset) = FigureCanvasPreset::from_name(preset_raw) else {
            return Ok(UiEventOutcome::default());
        };
        state.last_canvas_preset = Some(format!("{preset:?}"));
        let applied = apply_preset_for_target(app, state, preset)?;
        return Ok(outcome_ok(sub, if applied { Some("ui.canvas.applyPreset") } else { None }));
    }

    if let Some(mode_raw) = sub.strip_prefix("resize:") {
        let Some(mode) = CanvasResizeMode::from_name(mode_raw) else {
            return Ok(UiEventOutcome::default());
        };
        app.canvas_dialogs.canvas_resize_mode = mode;
        let preset = app
            .canvas_dialogs
            .active_canvas_preset
            .or_else(|| state.last_canvas_preset.as_deref().and_then(FigureCanvasPreset::from_name))
            .unwrap_or(FigureCanvasPreset::SingleColumn88mm);
        let applied = apply_preset_for_target(app, state, preset)?;
        return Ok(outcome_ok(sub, if applied { Some("ui.canvas.applyPreset") } else { None }));
    }

    Ok(UiEventOutcome::default())
}

fn active_graph_scene_and_points(app: &UiApp, state: &InspectorViewState) -> (Option<GraphSheet>, Vec<curvebook_chart::PlottedPointMeta>) {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return (None, Vec::new());
    };
    let pts = app.session().render_sheet_scene(&gs.id).map(|sc| sc.plotted_points).unwrap_or_default();
    (Some(gs), pts)
}

fn resolve_drill_target(app: &UiApp, state: &InspectorViewState, pts: &[curvebook_chart::PlottedPointMeta]) -> Option<SourceCellId> {
    if let Some(brushed) = &app.brushed_cell
        && app.document().data_sheet(&brushed.sheet_id).is_some()
    {
        return Some(brushed.clone());
    }
    if let Some(pt) = pts.get(state.selected_point_cursor).or_else(|| pts.first()) {
        return Some(pt.source_cell.clone());
    }
    resolve_graph_sheet(app.document(), state).and_then(|gs| {
        app.document().data_sheet(&gs.spec.primary_source_sheet_id).map(|ds| SourceCellId { sheet_id: ds.id.clone(), row: 0, col: 0, subcol: 0 })
    })
}

fn resolve_drill_target_fast(app: &UiApp, gs: Option<&GraphSheet>) -> Option<SourceCellId> {
    if let Some(brushed) = &app.brushed_cell
        && app.document().data_sheet(&brushed.sheet_id).is_some()
    {
        return Some(brushed.clone());
    }
    gs.and_then(|g| {
        app.document().data_sheet(&g.spec.primary_source_sheet_id).map(|ds| SourceCellId { sheet_id: ds.id.clone(), row: 0, col: 0, subcol: 0 })
    })
}

fn dispatch_drill_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let maybe_gs = resolve_graph_sheet(app.document(), state).cloned();
    if let Some(gs) = &maybe_gs {
        state.target_graph_id = Some(gs.id.as_str().to_string());
    }

    if let Some(pick) = sub.strip_prefix("point:") {
        let (_, pts) = active_graph_scene_and_points(app, state);
        if pts.is_empty() {
            return Ok(UiEventOutcome::default());
        }
        let n = pts.len();
        let cur = state.selected_point_cursor.min(n - 1);
        let next = match pick {
            "prev" => (cur + n - 1) % n,
            "next" => (cur + 1) % n,
            raw => raw.parse::<usize>().unwrap_or(cur).min(n - 1),
        };
        state.selected_point_cursor = next;
        let pt = &pts[next];
        state.selected_point_summary = format!("{} ({})", pt.series_label, pt.value_summary);
        app.brushed_cell = Some(pt.source_cell.clone());
        app.brushed_points = vec![pt.source_cell.clone()];
        if let Some(gs) = &maybe_gs
            && let Some(idx) = gs.spec.series.iter().position(|s| s.legend_label.as_deref() == Some(pt.series_label.as_str()))
        {
            app.canvas_dialogs.format_graph.selected_series_index = idx;
        }
        return Ok(outcome_ok(sub, None));
    }

    if sub == "go_to_cell" {
        let target = if let Some(b) = resolve_drill_target_fast(app, maybe_gs.as_ref())
            && app.brushed_cell.is_some()
        {
            b
        } else {
            let (_, pts) = active_graph_scene_and_points(app, state);
            let Some(t) = resolve_drill_target(app, state, &pts) else {
                return Ok(UiEventOutcome::default());
            };
            t
        };
        let coord = GridCellCoord::new(target.row, target.col, target.subcol);
        app.execute_command("sheet.select", json!({ "sheet_id": target.sheet_id.as_str() })).map_err(|e| e.to_string())?;
        app.grid.select_single_sync(coord);
        if let Some(ds) = app.session.document().data_sheet(&target.sheet_id) {
            app.grid.tool.ensure_coord_visible(ds, coord);
        }
        app.grid.refresh_selection_stats(app.session.document());
        app.brushed_cell = Some(target.clone());
        app.brushed_points = vec![target];
        app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
        app.focus_widget_id = None;
        return Ok(outcome_ok(sub, Some("sheet.select")));
    }

    if sub == "return_to_graph" {
        if let Some(gs) = &maybe_gs {
            app.execute_command("sheet.select", json!({ "sheet_id": gs.id.as_str() })).map_err(|e| e.to_string())?;
            app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
            return Ok(outcome_ok(sub, Some("sheet.select")));
        }
        return Ok(UiEventOutcome::default());
    }

    Ok(UiEventOutcome::default())
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `inspector` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut InspectorViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if let Some(tab) = payload.strip_prefix("tab:") {
        let (next_insp, sec) = match tab {
            "graph" => (ActiveInspector::FormatGraph, SectionKind::FormatGraph),
            "axes" => (ActiveInspector::FormatAxes, SectionKind::FormatAxes),
            "multi" => (ActiveInspector::BubblePlot, SectionKind::Multivariable),
            "pairwise" => (ActiveInspector::PairwiseComparisons, SectionKind::PairwiseCld),
            "layout" => (ActiveInspector::PageLayout, SectionKind::PageLayout),
            _ => return Ok(UiEventOutcome::default()),
        };
        app.canvas_dialogs.active_inspector = next_insp;
        let sec_id = sec.id();
        state.collapsed_sections.retain(|s| s != sec_id);
        state.open_popup = None;
        app.interaction.scroll_offsets.offsets.remove(SCROLL_ID);
        match next_insp {
            ActiveInspector::FormatGraph | ActiveInspector::FormatAxes => {
                let doc_snap = app.document().clone();
                if let Some(gs) = resolve_graph_sheet(&doc_snap, state).cloned() {
                    state.target_graph_id = Some(gs.id.as_str().to_string());
                    if next_insp == ActiveInspector::FormatGraph {
                        app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc_snap));
                    } else {
                        let target = app.canvas_dialogs.format_axes.target_axis;
                        app.canvas_dialogs.format_axes.sync_from_graph(&gs, target);
                    }
                }
            }
            ActiveInspector::BubblePlot | ActiveInspector::PairwiseComparisons => {
                let doc_snap = app.document().clone();
                if let Some(gs) = resolve_graph_sheet(&doc_snap, state) {
                    state.target_graph_id = Some(gs.id.as_str().to_string());
                    if next_insp == ActiveInspector::BubblePlot {
                        app.canvas_dialogs.bubble_plot.sync_from_graph(gs, &doc_snap);
                    } else {
                        app.canvas_dialogs.pairwise.sync_from_graph(gs, &doc_snap);
                    }
                }
            }
            ActiveInspector::PageLayout => {
                if let Some(ls) = resolve_layout_sheet(app.document(), state).cloned() {
                    state.target_layout_id = Some(ls.id.as_str().to_string());
                    app.canvas_dialogs.page_layout.sync_from_layout(&ls);
                }
            }
            ActiveInspector::ResultDetails | ActiveInspector::None => {}
        }
        return Ok(outcome_ok(payload, None));
    }

    if let Some(sec_raw) = payload.strip_prefix("section:") {
        let sec = match sec_raw {
            "drill_back" => SectionKind::DrillBack,
            "format_graph" => SectionKind::FormatGraph,
            "format_axes" => SectionKind::FormatAxes,
            "multivariable" => SectionKind::Multivariable,
            "pairwise" => SectionKind::PairwiseCld,
            "page_layout" => SectionKind::PageLayout,
            _ => return Ok(UiEventOutcome::default()),
        };
        let currently_open = is_section_expanded(app, state, sec);
        let id_str = String::from(sec.id());
        if currently_open {
            state.expanded_sections.retain(|s| s != sec.id());
            if !state.collapsed_sections.contains(&id_str) {
                state.collapsed_sections.push(id_str);
            }
        } else {
            state.collapsed_sections.retain(|s| s != sec.id());
            if !state.expanded_sections.contains(&id_str) {
                state.expanded_sections.push(id_str);
            }
        }
        state.open_popup = None;
        return Ok(outcome_ok(payload, None));
    }

    if let Some(pop_kind) = payload.strip_prefix("popup:") {
        if pop_kind == "close" || state.open_popup.as_deref() == Some(pop_kind) {
            state.open_popup = None;
        } else {
            state.open_popup = Some(String::from(pop_kind));
        }
        return Ok(outcome_ok(payload, None));
    }

    if let Some(sub) = payload.strip_prefix("fg.") {
        return dispatch_format_graph_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("regression.") {
        return dispatch_regression_action(app, state, sub);
    }
    if let Some(filter_raw) = payload.strip_prefix("cvd:") {
        let Some(filter) = CanvasPreviewFilter::from_name(filter_raw) else {
            return Ok(UiEventOutcome::default());
        };
        app.preview_filter = filter;
        return Ok(outcome_ok(payload, Some("ui.previewFilter.set")));
    }
    if let Some(sub) = payload.strip_prefix("axes.") {
        return dispatch_axes_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("mv.") {
        return dispatch_multivariable_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("pw.") {
        return dispatch_pairwise_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("layout.") {
        return dispatch_layout_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("canvas.") {
        return dispatch_canvas_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("drill.") {
        return dispatch_drill_action(app, state, sub);
    }

    Ok(UiEventOutcome::default())
}
