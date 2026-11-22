//! Live right-hand inspector view shard state and domain action dispatchers (`R10.18`).

use serde::{Deserialize, Serialize};
use serde_json::json;

use curvebook_chart::RectPt;
use curvebook_doc::{
    AnalysisKind, AxisSegment, AxisSpec, ColorChannelMapping, ConnectingLineStyle, ErrorBarSpec, GraphSheet, GraphicalDataObject, LayoutItem,
    LayoutSheet, LayoutSpec, LineConnectionKind, MultivariateGraphEncoding, MultivariatePlotMode, PhysicalUnit, ProjectDocument,
    ReplicateOverlayStyle, SeriesSpec, SheetId, SizeChannelMapping, SummaryMeasure, WhiskerMethod,
};
use curvebook_palette::{LineDashStyle, MarkerStyle, Rgba8};

use crate::UiApp;
use crate::dialogs_canvas::data_column_title;
use crate::interaction::UiEventOutcome;

const AXIS_MIN_INPUT_ID: &str = "inspector.axes.min.input";
const AXIS_MAX_INPUT_ID: &str = "inspector.axes.max.input";
const AXIS_INTERVAL_INPUT_ID: &str = "inspector.axes.interval.input";
const AXIS_TITLE_INPUT_ID: &str = "inspector.axes.title.input";

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
enum AxisWhich {
    X,
    Y,
    Y2,
}

fn outcome_ok(payload: &str, cmd_id: Option<&str>) -> UiEventOutcome {
    UiEventOutcome { handled: true, action: format!("inspector.{payload}"), command_id: cmd_id.map(String::from), command_result: None }
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
        GraphicalDataObject::BoxAndWhisker { fill_color, .. } | GraphicalDataObject::Violin { fill_color, .. } => *fill_color,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color,
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => *line_color,
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
                    replicate_lines: if let GraphicalDataObject::ScatterAndLine { replicate_lines, .. } = &s.gdo {
                        *replicate_lines
                    } else {
                        curvebook_doc::ReplicateLineMode::default()
                    },
                },
                _ => s.gdo.clone(),
            };
            next
        })
        .collect()
}

fn dispatch_format_graph_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    if let Some(topo) = sub.strip_prefix("topology:") {
        let next_series = retopologize_series(&gs.spec.series, topo);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series": next_series })).map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }
    Ok(UiEventOutcome::default())
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
    let specs: &[(f64, f64, f64)] =
        if count == 2 { &[(0.0, 0.35, 0.48), (0.65, 1.0, 0.48)] } else { &[(0.0, 0.25, 0.32), (0.40, 0.62, 0.32), (0.78, 1.0, 0.32)] };
    specs
        .iter()
        .enumerate()
        .map(|(idx, &(f_lo, f_hi, share))| AxisSegment {
            min: Some(if idx == 0 { a } else { a + span * f_lo }),
            max: Some(if idx + 1 == specs.len() { b } else { a + span * f_hi }),
            scale: base.scale,
            major_tick_interval: base.major_tick_interval,
            minor_ticks_per_interval: base.minor_ticks_per_interval,
            length_share: share,
        })
        .collect()
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

    if let Some(cnt_raw) = sub.strip_prefix("breaks:") {
        let count = cnt_raw.parse::<usize>().unwrap_or(1).clamp(1, 3);
        axis.segments = build_segments_for_count(&axis, count);
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
            return Err(format!("axis range max ({hi}) must be greater than min ({lo})"));
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

    Ok(UiEventOutcome::default())
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

    let Some(m_raw) = sub.strip_prefix("metric:") else {
        return Ok(UiEventOutcome::default());
    };
    let next_m = match m_raw {
        "Mean" => SummaryMeasure::Mean,
        "Median" => SummaryMeasure::Median,
        "GeometricMean" | "GeomMean" => SummaryMeasure::GeometricMean,
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

fn dispatch_pairwise_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    if sub != "sig_only:toggle" {
        return Ok(UiEventOutcome::default());
    }
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let mut pw = app.canvas_dialogs.pairwise.clone();
    pw.sync_from_graph(&gs, app.document());
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
        return Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")));
    }
    Ok(outcome_ok(sub, None))
}

fn mm_to_pt(mm: u16) -> f64 {
    f64::from(mm) * 72.0 / 25.4
}

fn layout_margin_mm(state: &InspectorViewState) -> u16 {
    if state.page_margin_mm == 0 { 12 } else { state.page_margin_mm.clamp(4, 40) }
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
    let min_rows = rows.max(1);
    let cols = cols.max(1);
    let cols_usize = usize::from(cols);
    let panel_count = spec.items.iter().filter(|it| matches!(it, LayoutItem::LiveGraph { .. })).count();
    let effective_rows = panel_count.div_ceil(cols_usize).max(usize::from(min_rows));
    let effective_rows_u8 = u8::try_from(effective_rows).unwrap_or(u8::MAX);
    spec.grid_template = Some((effective_rows_u8, cols));
    let (page_w_pt, page_h_pt) = layout_size_pt(spec);
    let usable_w = (page_w_pt - 2.0 * margin_pt - f64::from(cols.saturating_sub(1)) * gutter_pt).max(36.0);
    let usable_h = (page_h_pt - 2.0 * margin_pt - (effective_rows.saturating_sub(1) as f64) * gutter_pt).max(36.0);
    let cell_w = usable_w / f64::from(cols);
    let cell_h = usable_h / (effective_rows as f64);

    let mut panel_idx = 0usize;
    for item in &mut spec.items {
        if let LayoutItem::LiveGraph { bounds_pt, .. } = item {
            let r = panel_idx / cols_usize;
            let c = panel_idx % cols_usize;
            *bounds_pt =
                RectPt::new(margin_pt + (c as f64) * (cell_w + gutter_pt), margin_pt + (r as f64) * (cell_h + gutter_pt), cell_w, cell_h).into();
            panel_idx += 1;
        }
    }
}

fn dispatch_layout_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(grid_raw) = sub.strip_prefix("grid:") else {
        return Ok(UiEventOutcome::default());
    };
    let Some(ls) = resolve_layout_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_layout_id = Some(ls.id.as_str().to_string());
    let (rows, cols) = match grid_raw {
        "1x1" => (1u8, 1u8),
        "1x2" => (1, 2),
        "2x1" => (2, 1),
        "2x2" => (2, 2),
        "2x3" => (2, 3),
        _ => return Ok(UiEventOutcome::default()),
    };
    let mut spec = ls.spec.clone();
    rearrange_layout_grid(&mut spec, rows, cols, mm_to_pt(layout_margin_mm(state)), 12.0);
    app.execute_command("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "spec": spec })).map_err(|e| e.to_string())?;
    Ok(outcome_ok(sub, Some("layout.setSpec")))
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `inspector` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut InspectorViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if let Some(sub) = payload.strip_prefix("fg.") {
        return dispatch_format_graph_action(app, state, sub);
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
    Ok(UiEventOutcome::default())
}
