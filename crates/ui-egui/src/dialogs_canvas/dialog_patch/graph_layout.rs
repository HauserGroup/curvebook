//! `serde` JSON patch normalizers, mergers, and post-merge hooks for graph and layout dialogs.

use curvebook_doc::{
    CartesianOrientation, ColorChannelMapping, HeatmapLabelSpec, LayoutSpec, MultivariatePlotMode, PairwiseDisplayMode, ProjectDocument,
    ReplicateLineMode, SheetId, SizeChannelMapping, SummaryMeasure,
};
use curvebook_palette::{ColormapId, PaletteId, Rgba8};
use serde_json::{Map, Value, json};

use super::engine::{FieldKind, FieldRule, insert_nested_field, merge_struct_json, try_apply_rule};
use super::value_parsers::{
    norm_key, parse_axis_scale_str, parse_axis_thresholds_value, parse_bool_field, parse_bracket_label_mode, parse_bracket_style_str,
    parse_break_ornament_str, parse_cld_ordering_str, parse_colormap_id_str, parse_equation_display_mode_str, parse_error_bar_metric_str,
    parse_export_format, parse_finite_f64_with_locale, parse_frame_preset_str, parse_graph_kind_str, parse_grouped_arrangement_str,
    parse_journal_preset_str, parse_marker_fill_mode_val, parse_nonneg_f64, parse_opt_finite_f64_with_locale, parse_page_orientation_str,
    parse_pairwise_display_mode_str, parse_palette_str, parse_panel_badge_style_str, parse_paper_fill_patch, parse_str_field,
    parse_summary_measure_str, parse_survival_metric_str, parse_table_kind_str, parse_threshold_color, parse_threshold_dash_style,
    parse_tick_direction_str, parse_usize_min, validate_marker_shape_str, validate_point_color_str,
};
use crate::dialogs_canvas::graph_type::build_gdo_for_option;
use crate::dialogs_canvas::{
    ApplyStyleFromState, AxisTarget, BubblePlotInspectorState, ChangeGraphTypeState, CreateGraphDialogState, EqualizeScalingDialogState,
    ExportFigureState, FormatAxesState, FormatEmbeddedTableDialogState, FormatGraphState, FormatImageDialogState, FormatSelectedPointsState,
    PageLayoutComposerState, PairwiseComparisonsState,
};
use crate::i18n::LocalePreset;

fn parse_axis_target_str(raw: &str) -> Result<AxisTarget, String> {
    match norm_key(raw).as_str() {
        "x" => Ok(AxisTarget::X),
        "y" => Ok(AxisTarget::Y),
        "y2" => Ok(AxisTarget::Y2),
        _ => Err(format!("unknown target_axis `{raw}`")),
    }
}

pub fn patch_format_axes(
    state: &mut FormatAxesState,
    fields: &Map<String, Value>,
    doc: Option<&ProjectDocument>,
    preset: &LocalePreset,
) -> Result<(), String> {
    let mut working = state.clone();
    let mut resolved_target: Option<AxisTarget> = None;
    for key in ["target_axis", "axes.target_axis"] {
        if let Some(target_val) = fields.get(key) {
            let raw = parse_str_field(target_val, key)?;
            let next = parse_axis_target_str(raw)?;
            if let Some(prev) = resolved_target
                && prev != next
            {
                return Err(String::from("conflicting `target_axis` and `axes.target_axis` values"));
            }
            resolved_target = Some(next);
        }
    }
    if let Some(next) = resolved_target {
        working.switch_target_axis_preserving_thresholds(next, doc);
    }

    let mut top_patch = Map::new();
    let mut axis_patch = Map::new();
    let mut seg0_patch = Map::new();
    let mut pending_min = working.axis_draft.segments.first().and_then(|s| s.min);
    let mut pending_max = working.axis_draft.segments.first().and_then(|s| s.max);
    let mut bounds_changed = false;
    let mut actions: Vec<AxesAction> = Vec::new();

    for (key, val) in fields {
        match key.as_str() {
            "target_axis" | "axes.target_axis" => {}
            "enable_y2" | "axes.enable_y2" => {
                top_patch.insert(String::from("enable_y2"), json!(parse_bool_field(val, "enable_y2")?));
            }
            "title" | "axis_title" | "axes.title" => {
                axis_patch.insert(String::from("title"), json!(parse_str_field(val, "title")?));
            }
            "scale" | "axes.scale" => {
                let scale = parse_axis_scale_str(parse_str_field(val, "scale")?)?;
                seg0_patch.insert(String::from("scale"), serde_json::to_value(scale).map_err(|e| e.to_string())?);
            }
            "min" | "axis_min" | "axes.min" => {
                pending_min = parse_opt_finite_f64_with_locale(val, key, preset)?;
                bounds_changed = true;
            }
            "max" | "axis_max" | "axes.max" => {
                pending_max = parse_opt_finite_f64_with_locale(val, key, preset)?;
                bounds_changed = true;
            }
            "major_tick_interval" | "axes.major_tick_interval" => {
                let interval = parse_opt_finite_f64_with_locale(val, key, preset)?;
                if let Some(v) = interval
                    && v <= 0.0
                {
                    return Err(format!("`{key}` ({v}) must be > 0"));
                }
                seg0_patch.insert(String::from("major_tick_interval"), json!(interval));
            }
            "minor_ticks_per_interval" | "axes.minor_ticks_per_interval" => {
                let minor = parse_usize_min(val, key, 0)?;
                if minor > 10 {
                    return Err(format!("`{key}` ({minor}) must be in 0..=10"));
                }
                seg0_patch.insert(String::from("minor_ticks_per_interval"), json!(minor));
            }
            "break_ornament" | "axes.break_ornament" => {
                let bo = parse_break_ornament_str(parse_str_field(val, "break_ornament")?)?;
                axis_patch.insert(String::from("break_ornament"), serde_json::to_value(bo).map_err(|e| e.to_string())?);
            }
            "gap_width_pt" | "axes.gap_width_pt" => {
                let g = parse_finite_f64_with_locale(val, key, preset)?.clamp(1.0, 48.0);
                axis_patch.insert(String::from("gap_width_pt"), json!(g));
            }
            "stroke_width_pt" | "axes.stroke_width_pt" => {
                let sw = parse_finite_f64_with_locale(val, key, preset)?.clamp(0.2, 8.0);
                axis_patch.insert(String::from("stroke_width_pt"), json!(sw));
            }
            "tick_direction" | "axes.tick_direction" => {
                let td = parse_tick_direction_str(parse_str_field(val, "tick_direction")?)?;
                axis_patch.insert(String::from("tick_direction"), serde_json::to_value(td).map_err(|e| e.to_string())?);
            }
            "show_tick_labels" | "axes.show_tick_labels" => {
                axis_patch.insert(String::from("show_tick_labels"), json!(parse_bool_field(val, "show_tick_labels")?));
            }
            "add_segment" | "axes.add_segment" => {
                if parse_bool_field(val, key)? {
                    actions.push(AxesAction::AddSegment);
                }
            }
            "remove_segment" | "axes.remove_segment" => {
                let idx = parse_usize_min(val, key, 0)?;
                actions.push(AxesAction::RemoveSegment(idx));
            }
            "thresholds" | "axes.thresholds" => actions.push(AxesAction::SetActiveThresholds(parse_axis_thresholds_value(val, key, preset)?)),
            "x_thresholds" | "axes.x_thresholds" => actions.push(AxesAction::SetXThresholds(parse_axis_thresholds_value(val, key, preset)?)),
            "y_thresholds" | "axes.y_thresholds" => actions.push(AxesAction::SetYThresholds(parse_axis_thresholds_value(val, key, preset)?)),
            "add_threshold" | "axes.add_threshold" => {
                let axis = match val.get("axis") {
                    None | Some(Value::Null) => working.target_axis,
                    Some(other) => parse_axis_target_str(parse_str_field(other, "axis")?)?,
                };
                actions.push(AxesAction::AppendThresholds(axis, parse_axis_thresholds_value(val, key, preset)?));
            }
            "add_x_threshold" | "axes.add_x_threshold" => {
                actions.push(AxesAction::AppendThresholds(AxisTarget::X, parse_axis_thresholds_value(val, key, preset)?));
            }
            "add_y_threshold" | "axes.add_y_threshold" => {
                actions.push(AxesAction::AppendThresholds(AxisTarget::Y, parse_axis_thresholds_value(val, key, preset)?));
            }
            "clear_thresholds" | "axes.clear_thresholds" => {
                if parse_bool_field(val, key)? {
                    actions.push(AxesAction::SetActiveThresholds(Vec::new()));
                }
            }
            other => return Err(format!("unknown Format Axes field `{other}`")),
        }
    }

    if !axis_patch.is_empty() {
        top_patch.insert(String::from("axis_draft"), Value::Object(axis_patch));
    }
    merge_struct_json(&mut working, &top_patch, &["axis_draft"])?;
    if !seg0_patch.is_empty()
        && let Some(seg0) = working.axis_draft.segments.first_mut()
    {
        merge_struct_json(seg0, &seg0_patch, &[])?;
    }
    if bounds_changed {
        working.set_segment_bounds(0, pending_min, pending_max)?;
    }
    for act in actions {
        match act {
            AxesAction::AddSegment => {
                if !working.add_segment() {
                    return Err(String::from("cannot add axis segment: maximum of 3 segments already reached"));
                }
            }
            AxesAction::RemoveSegment(idx) => {
                if !working.remove_segment(idx) {
                    return Err(format!("invalid `remove_segment` index {idx}: axis must retain at least 1 segment"));
                }
            }
            AxesAction::SetActiveThresholds(items) => working.set_active_thresholds(items),
            AxesAction::SetXThresholds(items) => working.set_x_thresholds(items),
            AxesAction::SetYThresholds(items) => working.set_y_thresholds(items),
            AxesAction::AppendThresholds(axis, items) => working.append_thresholds_for_target(axis, items),
        }
    }
    *state = working;
    Ok(())
}

enum AxesAction {
    AddSegment,
    RemoveSegment(usize),
    SetActiveThresholds(Vec<curvebook_doc::AxisThreshold>),
    SetXThresholds(Vec<curvebook_doc::AxisThreshold>),
    SetYThresholds(Vec<curvebook_doc::AxisThreshold>),
    AppendThresholds(AxisTarget, Vec<curvebook_doc::AxisThreshold>),
}

pub fn patch_format_graph(state: &mut FormatGraphState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    let mut patch = Map::new();
    let mut eq_mode_action: Option<curvebook_doc::EquationDisplayMode> = None;
    let mut toggle_eq_mode = false;
    let mut reorder_actions: Vec<&'static str> = Vec::new();

    for (key, val) in fields {
        match key.as_str() {
            "show_legend" | "format_graph.show_legend" => {
                patch.insert(String::from("show_legend"), json!(parse_bool_field(val, "show_legend")?));
            }
            "legend_bordered" | "format_graph.legend_bordered" => {
                patch.insert(String::from("legend_bordered"), json!(parse_bool_field(val, "legend_bordered")?));
            }
            "error_bar_enabled" | "format_graph.error_bar_enabled" => {
                patch.insert(String::from("error_bar_enabled"), json!(parse_bool_field(val, "error_bar_enabled")?));
            }
            "marker_size_pt" | "format_graph.marker_size_pt" => {
                let sz = parse_finite_f64_with_locale(val, key, preset)?;
                if sz <= 0.0 {
                    return Err(String::from("`marker_size_pt` must be a finite positive number"));
                }
                patch.insert(String::from("marker_size_pt"), json!(sz.clamp(1.0, 24.0)));
                patch.insert(String::from("marker_dirty"), json!(true));
            }
            "marker_fill_mode" | "format_graph.marker_fill_mode" => {
                let fm = parse_marker_fill_mode_val(val, key)?;
                patch.insert(String::from("marker_fill_mode"), serde_json::to_value(fm).map_err(|e| e.to_string())?);
                patch.insert(String::from("marker_dirty"), json!(true));
            }
            "line_dash" | "dash_style" | "format_graph.line_dash" => {
                let dash = parse_threshold_dash_style(val, key)?;
                patch.insert(String::from("line_dash"), serde_json::to_value(dash).map_err(|e| e.to_string())?);
                patch.insert(String::from("line_dash_dirty"), json!(true));
            }
            "selected_series_index" | "series_index" | "format_graph.selected_series_index" => {
                let idx = val
                    .as_u64()
                    .or_else(|| val.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
                    .ok_or_else(|| String::from("`selected_series_index` must be a non-negative integer"))?;
                patch.insert(String::from("selected_series_index"), json!(usize::try_from(idx).unwrap_or(usize::MAX)));
            }
            "grouped_arrangement" | "format_graph.grouped_arrangement" => {
                let ga = parse_grouped_arrangement_str(parse_str_field(val, "grouped_arrangement")?)?;
                patch.insert(String::from("grouped_arrangement"), serde_json::to_value(ga).map_err(|e| e.to_string())?);
            }
            "graph_kind" | "kind" | "format_graph.graph_kind" => {
                let gk = parse_graph_kind_str(parse_str_field(val, "graph_kind")?)?;
                patch.insert(String::from("graph_kind"), serde_json::to_value(Some(gk)).map_err(|e| e.to_string())?);
            }
            "palette" | "format_graph.palette" => {
                let pal = parse_palette_str(parse_str_field(val, "palette")?)?;
                patch.insert(String::from("palette"), serde_json::to_value(pal).map_err(|e| e.to_string())?);
            }
            "frame_preset" | "format_graph.frame_preset" => {
                let fp = parse_frame_preset_str(parse_str_field(val, "frame_preset")?)?;
                patch.insert(String::from("frame_preset"), serde_json::to_value(fp).map_err(|e| e.to_string())?);
            }
            "half_violin_with_points" | "raincloud" | "format_graph.half_violin_with_points" => {
                patch.insert(String::from("half_violin_with_points"), json!(parse_bool_field(val, "half_violin_with_points")?));
            }
            "model_equation_mode" | "equation_mode" | "format_graph.model_equation_mode" => {
                let raw = val.as_str().ok_or_else(|| String::from("`model_equation_mode` must be `substituted` or `symbolic`"))?;
                eq_mode_action = Some(parse_equation_display_mode_str(raw)?);
            }
            "toggle_model_equation_mode" | "format_graph.toggle_model_equation_mode" => {
                if parse_bool_field(val, key)? {
                    toggle_eq_mode = true;
                }
            }
            "selected_level" | "format_graph.selected_level" => {
                patch.insert(String::from("mv_selected_level"), json!(parse_str_field(val, "selected_level")?));
            }
            "move_to_top" | "format_graph.move_to_top" => {
                if parse_bool_field(val, key)? {
                    reorder_actions.push("top");
                }
            }
            "move_up" | "format_graph.move_up" => {
                if parse_bool_field(val, key)? {
                    reorder_actions.push("up");
                }
            }
            "move_down" | "format_graph.move_down" => {
                if parse_bool_field(val, key)? {
                    reorder_actions.push("down");
                }
            }
            "move_to_end" | "format_graph.move_to_end" => {
                if parse_bool_field(val, key)? {
                    reorder_actions.push("end");
                }
            }
            _ => normalize_extended_format_graph_field(key, val, preset, &mut patch)?,
        }
    }

    let mut working = state.clone();
    merge_struct_json(&mut working, &patch, &[])?;
    if let Some(mode) = eq_mode_action {
        working.set_model_equation_mode(mode);
    } else if toggle_eq_mode {
        let next = match working.model_equation_mode.unwrap_or_default() {
            curvebook_doc::EquationDisplayMode::Substituted => curvebook_doc::EquationDisplayMode::Symbolic,
            curvebook_doc::EquationDisplayMode::Symbolic => curvebook_doc::EquationDisplayMode::Substituted,
        };
        working.set_model_equation_mode(next);
    }
    for dir in reorder_actions {
        let _ = working.reorder_mv_level(dir)?;
    }
    *state = working;
    Ok(())
}

fn normalize_extended_format_graph_field(key: &str, val: &Value, preset: &LocalePreset, patch: &mut Map<String, Value>) -> Result<(), String> {
    const RULES: &[FieldRule] = &[
        FieldRule::new(&["show_raw_data", "format_graph.show_raw_data"], "show_raw_data", FieldKind::Bool),
        FieldRule::new(&["show_fitted_curve", "format_graph.show_fitted_curve"], "show_fitted_curve", FieldKind::Bool),
        FieldRule::new(
            &["apply_all_data_sets", "apply_all", "format_graph.apply_all_data_sets", "format_graph.apply_globally"],
            "apply_all_data_sets",
            FieldKind::Bool,
        ),
        FieldRule::with_dirty(&["show_symbols", "format_graph.show_symbols"], "show_symbols", FieldKind::Bool, "show_symbols_dirty"),
        FieldRule::with_dirty(&["bind_to_right_y2", "format_graph.bind_to_right_y2"], "bind_to_right_y2", FieldKind::Bool, "bind_to_right_y2_dirty"),
        FieldRule::with_dirty(&["offset_x_data", "format_graph.offset_x_data"], "offset_x_data", FieldKind::FiniteF64, "offset_x_data_dirty"),
        FieldRule::with_dirty(
            &["bar_border_width_pt", "border_width_pt", "format_graph.bar_border_width_pt"],
            "bar_border_width_pt",
            FieldKind::NonNegF64,
            "bar_border_width_dirty",
        ),
        FieldRule::with_dirty(
            &["forest_show_pooled_diamond", "format_graph.forest_show_pooled_diamond"],
            "forest_show_pooled_diamond",
            FieldKind::Bool,
            "forest_show_pooled_diamond_dirty",
        ),
    ];
    if try_apply_rule(key, val, RULES, preset, patch)? {
        return Ok(());
    }
    match key {
        "series_alpha_pct" | "alpha_pct" | "alpha" | "format_graph.alpha" => {
            let raw = parse_finite_f64_with_locale(val, key, preset)?;
            let pct = if raw <= 1.0 && !matches!(key, "series_alpha_pct" | "alpha_pct") { raw * 100.0 } else { raw };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let clamped = pct.round().clamp(0.0, 100.0) as u8;
            patch.insert(String::from("series_alpha_pct"), json!(clamped));
            patch.insert(String::from("alpha_dirty"), json!(true));
        }
        "paper_fill" | "format_graph.paper_fill" => {
            let pf = parse_paper_fill_patch(val)?;
            patch.insert(String::from("paper_fill"), serde_json::to_value(pf).map_err(|e| e.to_string())?);
            patch.insert(String::from("paper_fill_dirty"), json!(true));
        }
        "heatmap_labels" | "format_graph.heatmap_labels" => {
            let hl = serde_json::from_value::<HeatmapLabelSpec>(val.clone()).map_err(|err| format!("invalid `heatmap_labels`: {err}"))?;
            patch.insert(String::from("heatmap_labels"), serde_json::to_value(hl).map_err(|e| e.to_string())?);
            patch.insert(String::from("heatmap_labels_dirty"), json!(true));
        }
        "orientation" | "format_graph.orientation" => {
            let orient = match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                Some("vertical") => CartesianOrientation::Vertical,
                Some("horizontal") => CartesianOrientation::Horizontal,
                _ => serde_json::from_value::<CartesianOrientation>(val.clone()).map_err(|err| format!("invalid `orientation`: {err}"))?,
            };
            patch.insert(String::from("orientation"), serde_json::to_value(orient).map_err(|e| e.to_string())?);
            patch.insert(String::from("orientation_dirty"), json!(true));
        }
        "replicate_lines" | "format_graph.replicate_lines" => {
            let mode = match val.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                Some("summary_center" | "summary") => ReplicateLineMode::SummaryCenter,
                Some("individual_replicate_trajectories" | "individual" | "individual_trajectories" | "spaghetti") => {
                    ReplicateLineMode::IndividualReplicateTrajectories
                }
                _ => serde_json::from_value::<ReplicateLineMode>(val.clone()).map_err(|err| format!("invalid `replicate_lines`: {err}"))?,
            };
            patch.insert(String::from("replicate_lines"), serde_json::to_value(mode).map_err(|e| e.to_string())?);
            patch.insert(String::from("replicate_lines_dirty"), json!(true));
        }
        other => return Err(format!("unknown Format Graph field `{other}`")),
    }
    Ok(())
}

pub fn patch_pairwise(
    state: &mut PairwiseComparisonsState,
    fields: &Map<String, Value>,
    doc: Option<&ProjectDocument>,
    preset: &LocalePreset,
) -> Result<(), String> {
    let mut patch = Map::new();
    let mut auto_add = false;
    let mut explicit_mode = None;
    let mut explicit_enabled = None;

    for (key, val) in fields {
        match key.as_str() {
            "result_sheet_id" | "selected_result_sheet_id" | "pairwise.result_sheet_id" => {
                let s = parse_str_field(val, "result_sheet_id")?.trim();
                if !s.is_empty() {
                    let sid = SheetId::try_new(s).map_err(|e| e.to_string())?;
                    patch.insert(String::from("selected_result_sheet_id"), serde_json::to_value(Some(sid)).map_err(|e| e.to_string())?);
                }
            }
            "bracket_style" | "style" | "pairwise.bracket_style" | "pairwise.style" => {
                let bs = parse_bracket_style_str(parse_str_field(val, "bracket_style")?)?;
                patch.insert(String::from("bracket_style"), serde_json::to_value(bs).map_err(|e| e.to_string())?);
                patch.insert(String::from("brackets_dirty"), json!(true));
            }
            "label_mode" | "pairwise.label_mode" => {
                let lm = parse_bracket_label_mode(val)?;
                patch.insert(String::from("label_mode"), serde_json::to_value(lm).map_err(|e| e.to_string())?);
                patch.insert(String::from("brackets_dirty"), json!(true));
            }
            "hide_non_significant" | "hide_if_non_significant" | "pairwise.hide_non_significant" => {
                patch.insert(String::from("hide_non_significant"), json!(parse_bool_field(val, "hide_non_significant")?));
                patch.insert(String::from("brackets_dirty"), json!(true));
            }
            "auto_add" | "pairwise.auto_add" => {
                if parse_bool_field(val, key)? {
                    auto_add = true;
                }
            }
            "display_mode" | "pairwise_display_mode" | "pairwise.display_mode" => {
                let mode = parse_pairwise_display_mode_str(parse_str_field(val, "display_mode")?)?;
                patch.insert(String::from("display_mode"), serde_json::to_value(mode).map_err(|e| e.to_string())?);
                explicit_mode = Some(mode);
            }
            "cld_enabled" | "enabled" | "compact_letter_display" | "pairwise.cld_enabled" => {
                let en = parse_bool_field(val, "cld_enabled")?;
                patch.insert(String::from("cld_enabled"), json!(en));
                explicit_enabled = Some(en);
            }
            "cld_ordering" | "ordering" | "pairwise.cld_ordering" => {
                let ord = parse_cld_ordering_str(parse_str_field(val, "cld_ordering")?)?;
                patch.insert(String::from("cld_ordering"), serde_json::to_value(ord).map_err(|e| e.to_string())?);
            }
            "cld_uppercase" | "uppercase" | "pairwise.cld_uppercase" => {
                patch.insert(String::from("cld_uppercase"), json!(parse_bool_field(val, "cld_uppercase")?));
            }
            "cld_alpha_override" | "alpha_override" | "pairwise.cld_alpha_override" => {
                let alpha = parse_opt_finite_f64_with_locale(val, "cld_alpha_override", preset)?;
                if let Some(a) = alpha
                    && (a <= 0.0 || a >= 1.0)
                {
                    return Err(format!("`cld_alpha_override` must lie in (0, 1), got {a}"));
                }
                patch.insert(String::from("cld_alpha_override"), json!(alpha));
            }
            "cld_font_size_pt" | "font_size_pt" | "pairwise.cld_font_size_pt" => {
                let fs = parse_finite_f64_with_locale(val, "cld_font_size_pt", preset)?;
                if fs <= 0.0 || fs > 144.0 {
                    return Err(format!("`cld_font_size_pt` must lie in (0, 144], got {fs}"));
                }
                patch.insert(String::from("cld_font_size_pt"), json!(fs));
            }
            "cld_offset_pt" | "offset_pt" | "pairwise.cld_offset_pt" => {
                let off = parse_finite_f64_with_locale(val, "cld_offset_pt", preset)?;
                if !(0.0..=200.0).contains(&off) {
                    return Err(format!("`cld_offset_pt` must lie in [0, 200], got {off}"));
                }
                patch.insert(String::from("cld_offset_pt"), json!(off));
            }
            other => return Err(format!("unknown Pairwise Comparisons field `{other}`")),
        }
    }

    if let (Some(en), Some(mode)) = (explicit_enabled, explicit_mode)
        && en != (mode != PairwiseDisplayMode::BracketsOnly)
    {
        return Err(format!("conflicting `cld_enabled` ({en}) and `display_mode` ({mode:?})"));
    }

    merge_struct_json(state, &patch, &[])?;
    if let Some(en) = explicit_enabled
        && explicit_mode.is_none()
    {
        if en {
            if state.display_mode == PairwiseDisplayMode::BracketsOnly {
                state.display_mode = if state.brackets.is_empty() {
                    PairwiseDisplayMode::CompactLetterDisplayOnly
                } else {
                    PairwiseDisplayMode::BracketsAndCompactLetterDisplay
                };
            }
        } else {
            state.display_mode = PairwiseDisplayMode::BracketsOnly;
        }
    } else if let Some(mode) = explicit_mode
        && explicit_enabled.is_none()
    {
        state.cld_enabled = mode != PairwiseDisplayMode::BracketsOnly;
    }
    for b in &mut state.brackets {
        b.style = state.bracket_style;
        b.label_mode = state.label_mode.clone();
        b.hide_if_non_significant = state.hide_non_significant;
    }
    if auto_add && let Some(d) = doc {
        let _ = state.auto_add_from_analysis(d);
        state.brackets_dirty = true;
    }
    Ok(())
}

pub fn patch_bubble_plot(state: &mut BubblePlotInspectorState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    let mut color_mode: Option<String> = None;
    let mut color_col: Option<usize> = None;
    let mut color_col_explicit = false;
    let mut palette_override: Option<PaletteId> = None;
    let mut colormap_override: Option<ColormapId> = None;

    for (key, val) in fields {
        match key.as_str() {
            "x_column" | "bubble.x_column" => {
                patch.insert(String::from("x_column"), json!(parse_usize_min(val, key, 0)?));
            }
            "y_column" | "bubble.y_column" => {
                patch.insert(String::from("y_column"), json!(parse_usize_min(val, key, 0)?));
            }
            "size_column" | "bubble.size_column" => {
                let sz = if val.is_null() {
                    SizeChannelMapping::Constant { radius_pt: 5.0 }
                } else {
                    SizeChannelMapping::Variable { column: parse_usize_min(val, key, 0)?, min_radius_pt: 2.5, max_radius_pt: 14.0 }
                };
                patch.insert(String::from("size_channel"), serde_json::to_value(sz).map_err(|e| e.to_string())?);
            }
            "label_column" | "group_column" | "bubble.label_column" => {
                let lc = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                patch.insert(String::from("label_column"), json!(lc));
            }
            "color_mode" | "bubble.color_mode" => {
                let raw = parse_str_field(val, key)?;
                let norm = norm_key(raw);
                match norm.as_str() {
                    "constant" | "continuous" | "categorical" => color_mode = Some(norm),
                    _ => return Err(format!("unknown bubble plot color_mode `{raw}`")),
                }
            }
            "color_column" | "bubble.color_column" => {
                color_col_explicit = true;
                color_col = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
            }
            "palette" | "bubble.palette" => {
                palette_override = Some(parse_palette_str(parse_str_field(val, "palette")?)?);
            }
            "colormap" | "bubble.colormap" => {
                colormap_override = Some(parse_colormap_id_str(parse_str_field(val, "colormap")?)?);
            }
            "show_convex_hulls" | "bubble.show_convex_hulls" => {
                patch.insert(String::from("show_convex_hulls"), json!(parse_bool_field(val, "show_convex_hulls")?));
            }
            "show_confidence_ellipses" | "bubble.show_confidence_ellipses" => {
                patch.insert(String::from("show_confidence_ellipses"), json!(parse_bool_field(val, "show_confidence_ellipses")?));
            }
            "plot_mode" | "bubble.plot_mode" => {
                let raw = parse_str_field(val, key)?;
                let pm = match norm_key(raw).as_str() {
                    "categoricalsummary" | "categorical" => MultivariatePlotMode::CategoricalSummary {
                        gdo: build_gdo_for_option("col.box_whisker", PaletteId::OkabeIto.color_at(0), Default::default(), SummaryMeasure::Mean),
                        y_error_bars: None,
                    },
                    "bubblescatter" | "bubble" | "scatter" => MultivariatePlotMode::BubbleScatter,
                    _ => return Err(format!("unknown bubble plot_mode `{raw}`")),
                };
                patch.insert(String::from("plot_mode"), serde_json::to_value(pm).map_err(|e| e.to_string())?);
            }
            "categorical_summary" => {
                let is_cat = if let Some(b) = val.as_bool() {
                    b
                } else {
                    let raw = parse_str_field(val, key)?;
                    match norm_key(raw).as_str() {
                        "categoricalsummary" | "categorical" => true,
                        "bubblescatter" | "bubble" | "scatter" => false,
                        _ => return Err(format!("unknown bubble plot_mode `{raw}`")),
                    }
                };
                let pm = if is_cat {
                    MultivariatePlotMode::CategoricalSummary {
                        gdo: build_gdo_for_option("col.box_whisker", PaletteId::OkabeIto.color_at(0), Default::default(), SummaryMeasure::Mean),
                        y_error_bars: None,
                    }
                } else {
                    MultivariatePlotMode::BubbleScatter
                };
                patch.insert(String::from("plot_mode"), serde_json::to_value(pm).map_err(|e| e.to_string())?);
            }
            "bubble.color_channel" | "color_channel" => {
                if let Ok(ch) = serde_json::from_value::<ColorChannelMapping>(val.clone()) {
                    patch.insert(String::from("color_channel"), serde_json::to_value(ch).map_err(|e| e.to_string())?);
                } else if val.is_null() {
                    let ch = ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0));
                    patch.insert(String::from("color_channel"), serde_json::to_value(ch).map_err(|e| e.to_string())?);
                } else if let Some(s) = val.as_str() {
                    let norm = norm_key(s);
                    if norm.contains("constant") {
                        color_mode = Some(String::from("constant"));
                    } else if norm.contains("continuous") {
                        color_mode = Some(String::from("continuous"));
                    } else if norm.contains("categorical") {
                        color_mode = Some(String::from("categorical"));
                    } else {
                        return Err(format!("unsupported `{key}` value `{s}`"));
                    }
                } else {
                    return Err(format!("`{key}` must be a ColorChannelMapping object, mode string, or null"));
                }
            }
            "bubble.size_channel" | "size_channel" => {
                if let Ok(sz) = serde_json::from_value::<SizeChannelMapping>(val.clone()) {
                    patch.insert(String::from("size_channel"), serde_json::to_value(sz).map_err(|e| e.to_string())?);
                } else if val.is_null() {
                    let sz = SizeChannelMapping::Constant { radius_pt: 5.0 };
                    patch.insert(String::from("size_channel"), serde_json::to_value(sz).map_err(|e| e.to_string())?);
                } else if let Ok(col) = parse_usize_min(val, key, 0) {
                    let sz = SizeChannelMapping::Variable { column: col, min_radius_pt: 2.5, max_radius_pt: 14.0 };
                    patch.insert(String::from("size_channel"), serde_json::to_value(sz).map_err(|e| e.to_string())?);
                } else {
                    return Err(format!("`{key}` must be a SizeChannelMapping object, column index, or null"));
                }
            }
            other => return Err(format!("unknown Bubble Plot field `{other}`")),
        }
    }

    let mut working = state.clone();
    merge_struct_json(&mut working, &patch, &[])?;
    if color_mode.is_some() || color_col_explicit || palette_override.is_some() || colormap_override.is_some() {
        match color_mode.as_deref() {
            Some("constant") => {
                let pal = palette_override.unwrap_or(PaletteId::OkabeIto);
                working.color_channel = ColorChannelMapping::Constant(pal.color_at(0));
            }
            Some("continuous") => {
                let col = color_col.unwrap_or(working.y_column);
                working.set_color_continuous(col, colormap_override.unwrap_or(ColormapId::Viridis), None);
            }
            Some("categorical") => {
                let col = color_col.unwrap_or(working.label_column.unwrap_or(0));
                working.set_color_categorical(col, palette_override.unwrap_or(PaletteId::OkabeIto));
            }
            _ if color_col_explicit && color_col.is_none() => {
                let pal = palette_override.unwrap_or(PaletteId::OkabeIto);
                working.color_channel = ColorChannelMapping::Constant(pal.color_at(0));
            }
            _ if colormap_override.is_some() => {
                let existing_col = match &working.color_channel {
                    ColorChannelMapping::Continuous { column, .. } => *column,
                    _ => working.y_column,
                };
                let col = color_col.unwrap_or(existing_col);
                working.set_color_continuous(col, colormap_override.unwrap_or(ColormapId::Viridis), None);
            }
            _ if let Some(col) = color_col => match &mut working.color_channel {
                ColorChannelMapping::Continuous { column, .. } if palette_override.is_none() => {
                    *column = col;
                }
                ColorChannelMapping::Categorical { column, palette, .. } => {
                    *column = col;
                    if let Some(pal) = palette_override {
                        *palette = pal;
                    }
                }
                _ => {
                    working.set_color_categorical(col, palette_override.unwrap_or(PaletteId::OkabeIto));
                }
            },
            _ if let Some(pal) = palette_override => match &mut working.color_channel {
                ColorChannelMapping::Categorical { palette, .. } => {
                    *palette = pal;
                }
                ColorChannelMapping::Constant(c) => {
                    *c = pal.color_at(0);
                }
                ColorChannelMapping::Continuous { .. } => {
                    return Err(String::from("`palette` applies to categorical or constant color channels; use `colormap` for continuous mode"));
                }
            },
            _ => {}
        }
    }
    *state = working;
    Ok(())
}

pub fn patch_apply_style(state: &mut ApplyStyleFromState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "donor_graph_id" | "donor_sheet_id" | "style.donor_graph_id" | "apply_style.donor_graph_id" => {
                let s = parse_str_field(val, "donor_graph_id")?.trim();
                let sid = SheetId::try_new(s).map_err(|e| format!("invalid `donor_graph_id`: {e}"))?;
                patch.insert(String::from("donor_graph_id"), json!(sid.as_str()));
            }
            "target_sheet_ids" | "target_graph_ids" | "apply_style.target_sheet_ids" => {
                let arr = val.as_array().ok_or_else(|| String::from("`target_sheet_ids` must be an array"))?;
                let mut ids = Vec::with_capacity(arr.len());
                for (idx, entry) in arr.iter().enumerate() {
                    let s = entry.as_str().ok_or_else(|| format!("`target_sheet_ids[{idx}]` must be a string"))?.trim();
                    let sid = SheetId::try_new(s).map_err(|e| format!("invalid `target_sheet_ids[{idx}]`: {e}"))?;
                    if !ids.contains(&sid) {
                        ids.push(sid);
                    }
                }
                patch.insert(String::from("target_graph_id"), json!(ids.first().map(SheetId::as_str)));
                patch.insert(String::from("target_graph_ids"), serde_json::to_value(ids).map_err(|e| e.to_string())?);
            }
            "clone_palette" | "style.clone_palette" | "apply_style.clone_palette" => {
                patch.insert(String::from("clone_palette"), json!(parse_bool_field(val, "clone_palette")?));
            }
            "clone_frame_and_axes" | "style.clone_frame_and_axes" | "apply_style.clone_frame_and_axes" => {
                patch.insert(String::from("clone_frame_and_axes"), json!(parse_bool_field(val, "clone_frame_and_axes")?));
            }
            "clone_error_bars_and_legend" | "style.clone_error_bars_and_legend" | "apply_style.clone_error_bars_and_legend" => {
                patch.insert(String::from("clone_error_bars_and_legend"), json!(parse_bool_field(val, "clone_error_bars_and_legend")?));
            }
            other => return Err(format!("unknown Apply Style From field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_page_layout(state: &mut PageLayoutComposerState, fields: &Map<String, Value>, doc: Option<&ProjectDocument>) -> Result<(), String> {
    let mut patch = Map::new();
    let mut grid_changed = false;
    let mut explicit_graph_ids: Option<Vec<SheetId>> = None;
    let has_explicit_orient = fields.contains_key("orientation") || fields.contains_key("layout.orientation");

    for (key, val) in fields {
        match key.as_str() {
            "preset" | "layout.preset" => {
                let preset = parse_journal_preset_str(parse_str_field(val, "preset")?)?;
                let derived = LayoutSpec::from_preset(preset);
                patch.insert(String::from("preset"), serde_json::to_value(preset).map_err(|e| e.to_string())?);
                patch.insert(String::from("preset_dirty"), json!(true));
                patch.insert(String::from("page_width"), json!(derived.width));
                patch.insert(String::from("page_height"), json!(derived.height));
                patch.insert(String::from("page_unit"), serde_json::to_value(derived.unit).map_err(|e| e.to_string())?);
                if !has_explicit_orient {
                    patch.insert(String::from("orientation"), serde_json::to_value(derived.orientation).map_err(|e| e.to_string())?);
                }
            }
            "orientation" | "layout.orientation" => {
                let orient = parse_page_orientation_str(parse_str_field(val, "orientation")?)?;
                patch.insert(String::from("orientation"), serde_json::to_value(orient).map_err(|e| e.to_string())?);
            }
            "rows" | "layout.rows" => {
                let rows = u8::try_from(parse_usize_min(val, key, 1)?.min(12)).unwrap_or(12);
                patch.insert(String::from("rows"), json!(rows));
                grid_changed = true;
            }
            "cols" | "columns" | "layout.cols" => {
                let cols = u8::try_from(parse_usize_min(val, key, 1)?.min(12)).unwrap_or(12);
                patch.insert(String::from("cols"), json!(cols));
                grid_changed = true;
            }
            "align_plot_origins" | "layout.align_plot_origins" => {
                patch.insert(String::from("align_plot_origins"), json!(parse_bool_field(val, "align_plot_origins")?));
            }
            "panel_badges" | "layout.panel_badges" => {
                let pb = parse_panel_badge_style_str(parse_str_field(val, "panel_badges")?)?;
                patch.insert(String::from("panel_badges"), serde_json::to_value(pb).map_err(|e| e.to_string())?);
            }
            "new_callout_markdown" | "callout" | "layout.new_callout_markdown" => {
                patch.insert(String::from("new_callout_markdown"), json!(parse_str_field(val, key)?));
            }
            "arrange_graph_ids" | "graph_ids" | "layout.arrange_graph_ids" => {
                let arr = val.as_array().ok_or_else(|| format!("`{key}` must be an array of sheet IDs"))?;
                let mut ids = Vec::with_capacity(arr.len());
                for (idx, item) in arr.iter().enumerate() {
                    let raw = item.as_str().ok_or_else(|| format!("`{key}[{idx}]` must be a string"))?;
                    ids.push(SheetId::try_new(raw).map_err(|e| e.to_string())?);
                }
                explicit_graph_ids = Some(ids);
            }
            other => return Err(format!("unknown Page Layout field `{other}`")),
        }
    }

    merge_struct_json(state, &patch, &[])?;
    if let Some(ids) = explicit_graph_ids {
        state.arrange_grid(&ids, 18.0, 12.0);
    } else if grid_changed && let Some(d) = doc {
        let graph_ids: Vec<SheetId> = d.graph_sheets.iter().map(|g| g.id.clone()).collect();
        state.arrange_grid(&graph_ids, 18.0, 12.0);
    }
    Ok(())
}

pub fn patch_export_figure(state: &mut ExportFigureState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "format" | "export.format" => {
                let fmt = parse_export_format(val)?;
                patch.insert(String::from("format"), serde_json::to_value(fmt).map_err(|e| e.to_string())?);
            }
            "width_px" | "export.width_px" => {
                let w = u32::try_from(parse_usize_min(val, key, 16)?).unwrap_or(u32::MAX);
                patch.insert(String::from("width_px"), json!(w));
            }
            "height_px" | "export.height_px" => {
                let h = u32::try_from(parse_usize_min(val, key, 16)?).unwrap_or(u32::MAX);
                patch.insert(String::from("height_px"), json!(h));
            }
            "dpi" | "export.dpi" => {
                let d = u16::try_from(parse_usize_min(val, key, 72)?.clamp(72, 2400)).unwrap_or(600);
                patch.insert(String::from("dpi"), json!(d));
            }
            "transparent_bg" | "export.transparent_bg" => {
                patch.insert(String::from("transparent_bg"), json!(parse_bool_field(val, key)?));
            }
            other => return Err(format!("unknown Export Figure field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_create_graph(state: &mut CreateGraphDialogState, fields: &Map<String, Value>) -> Result<(), String> {
    const RULES: &[FieldRule] = &[
        FieldRule::new(&["data_sheet_id", "source_sheet_id", "create_graph.source_sheet_id"], "source_sheet_id", FieldKind::Str),
        FieldRule::new(&["one_per_data_set", "create_graph.one_per_data_set"], "one_per_data_set", FieldKind::Bool),
        FieldRule::new(&["y_axis_title", "create_graph.y_axis_title"], "y_axis_title", FieldKind::Str),
        FieldRule::new(&["title", "graph_title", "create_graph.title"], "graph_title", FieldKind::Str),
    ];
    let preset = LocalePreset::default();
    let mut patch = Map::new();
    for (key, val) in fields {
        if try_apply_rule(key, val, RULES, &preset, &mut patch)? {
            continue;
        }
        match key.as_str() {
            "kind" | "kind_slug" | "create_graph.kind" => {
                patch.insert(String::from("kind_slug"), json!(parse_str_field(val, "kind")?.trim().to_ascii_lowercase()));
            }
            other => return Err(format!("unknown CreateGraph field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_equalize_scaling(state: &mut EqualizeScalingDialogState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "mode" | "equalize_scaling.mode" => {
                let s = parse_str_field(val, "mode")?;
                let mode = match s.trim().to_ascii_lowercase().as_str() {
                    "match_smallest" | "smallest" => "match_smallest",
                    "match_largest" | "largest" => "match_largest",
                    "explicit_percent" | "explicit" => "explicit_percent",
                    other => return Err(format!("unknown EqualizeScaling mode `{other}`")),
                };
                patch.insert(String::from("mode"), json!(mode));
            }
            "percent" | "explicit_percent" | "equalize_scaling.percent" => {
                let p = parse_finite_f64_with_locale(val, "percent", preset)?;
                if !(5.0..=500.0).contains(&p) {
                    return Err(String::from("`percent` must be finite in [5.0, 500.0]"));
                }
                patch.insert(String::from("explicit_percent"), json!(p));
            }
            "panel_ids" | "selected_panel_ids" | "equalize_scaling.panel_ids" => {
                let arr = val.as_array().ok_or_else(|| String::from("`panel_ids` must be an array"))?;
                let mut next = Vec::with_capacity(arr.len());
                for entry in arr {
                    next.push(entry.as_str().ok_or_else(|| String::from("panel ID must be a string"))?.to_owned());
                }
                patch.insert(String::from("selected_panel_ids"), json!(next));
            }
            other => return Err(format!("unknown EqualizeScaling field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub fn patch_format_embedded_table(
    state: &mut FormatEmbeddedTableDialogState,
    fields: &Map<String, Value>,
    preset: &LocalePreset,
) -> Result<(), String> {
    const RULES: &[FieldRule] = &[
        FieldRule::new(&["show_column_titles", "embedded_table.show_column_titles"], "style.show_column_titles", FieldKind::Bool),
        FieldRule::new(&["show_row_titles", "embedded_table.show_row_titles"], "style.show_row_titles", FieldKind::Bool),
        FieldRule::new(&["transpose", "embedded_table.transpose"], "style.transpose", FieldKind::Bool),
        FieldRule::new(&["bold_headers", "embedded_table.bold_headers"], "style.bold_headers", FieldKind::Bool),
    ];
    let mut patch = Map::new();
    for (key, val) in fields {
        if try_apply_rule(key, val, RULES, preset, &mut patch)? {
            continue;
        }
        match key.as_str() {
            "font_size_pt" | "embedded_table.font_size" => {
                let f = parse_finite_f64_with_locale(val, key, preset)?;
                if f <= 0.0 || f > 72.0 {
                    return Err(format!("`{key}` must be in (0, 72]"));
                }
                insert_nested_field(&mut patch, "style", "font_size_pt", json!(f));
            }
            "outer_border_width_pt" | "embedded_table.outer_border_width_pt" | "gridline_width_pt" | "embedded_table.gridline_width_pt" => {
                let f = parse_finite_f64_with_locale(val, key, preset)?;
                if !(0.0..=12.0).contains(&f) {
                    return Err(format!("`{key}` must be in [0, 12]"));
                }
                let field = if key.contains("gridline") { "gridline_width_pt" } else { "outer_border_width_pt" };
                insert_nested_field(&mut patch, "style", field, json!(f));
            }
            "border_color" | "embedded_table.border_color" => {
                let c = parse_threshold_color(val, key)?;
                insert_nested_field(&mut patch, "style", "border_color", serde_json::to_value(c).map_err(|e| e.to_string())?);
            }
            "fill_color" | "embedded_table.fill_color" => {
                let fc: Option<Rgba8> = if val.is_null() { None } else { Some(parse_threshold_color(val, key)?) };
                insert_nested_field(&mut patch, "style", "fill_color", serde_json::to_value(fc).map_err(|e| e.to_string())?);
            }
            other => return Err(format!("unknown FormatEmbeddedTable field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &["style"])
}

pub fn patch_format_image(state: &mut FormatImageDialogState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    const RULES: &[FieldRule] = &[
        FieldRule::new(&["lock_aspect_ratio", "format_image.lock_aspect_ratio"], "lock_aspect_ratio", FieldKind::Bool),
        FieldRule::new(&["rotation_deg", "format_image.rotation_deg"], "rotation_deg", FieldKind::FiniteF64),
        FieldRule::new(&["crop_left_pt", "format_image.crop.left"], "crop.left_pt", FieldKind::NonNegF64),
        FieldRule::new(&["crop_right_pt", "format_image.crop.right"], "crop.right_pt", FieldKind::NonNegF64),
        FieldRule::new(&["crop_top_pt", "format_image.crop.top"], "crop.top_pt", FieldKind::NonNegF64),
        FieldRule::new(&["crop_bottom_pt", "format_image.crop.bottom"], "crop.bottom_pt", FieldKind::NonNegF64),
    ];
    let mut patch = Map::new();
    let mut new_w: Option<f64> = None;
    let mut new_h: Option<f64> = None;
    let mut explicit_border_enabled: Option<bool> = None;
    let mut touched_border_attr = false;

    for (key, val) in fields {
        if try_apply_rule(key, val, RULES, preset, &mut patch)? {
            continue;
        }
        match key.as_str() {
            "width_pt" | "format_image.width_pt" => {
                let w = parse_finite_f64_with_locale(val, key, preset)?;
                if w <= 0.0 {
                    return Err(format!("`{key}` must be finite and > 0"));
                }
                insert_nested_field(&mut patch, "bounds_pt", "width", json!(w));
                new_w = Some(w);
            }
            "height_pt" | "format_image.height_pt" => {
                let h = parse_finite_f64_with_locale(val, key, preset)?;
                if h <= 0.0 {
                    return Err(format!("`{key}` must be finite and > 0"));
                }
                insert_nested_field(&mut patch, "bounds_pt", "height", json!(h));
                new_h = Some(h);
            }
            "border_enabled" | "format_image.border.enabled" => {
                let b = parse_bool_field(val, key)?;
                explicit_border_enabled = Some(b);
                patch.insert(String::from("border_enabled"), json!(b));
            }
            "border_width_pt" | "format_image.border.width" => {
                patch.insert(String::from("border_width_pt"), json!(parse_nonneg_f64(val, key, preset)?));
                touched_border_attr = true;
            }
            "border_color" | "format_image.border.color" => {
                let c = parse_threshold_color(val, key)?;
                patch.insert(String::from("border_color"), serde_json::to_value(c).map_err(|e| e.to_string())?);
                touched_border_attr = true;
            }
            "border_dash" | "format_image.border.dash" => {
                let d = parse_threshold_dash_style(val, key)?;
                patch.insert(String::from("border_dash"), serde_json::to_value(d).map_err(|e| e.to_string())?);
                touched_border_attr = true;
            }
            other => return Err(format!("unknown FormatImage field `{other}`")),
        }
    }

    if touched_border_attr && explicit_border_enabled.is_none() {
        patch.insert(String::from("border_enabled"), json!(true));
    }
    let ratio = state.aspect_h_over_w();
    merge_struct_json(state, &patch, &["bounds_pt", "crop"])?;
    if state.lock_aspect_ratio {
        if let Some(w) = new_w {
            state.bounds_pt.height = (w * ratio).max(1.0);
        } else if let Some(h) = new_h {
            state.bounds_pt.width = (h / ratio.max(1e-9)).max(1.0);
        }
    }
    Ok(())
}

pub fn patch_format_selected_points(state: &mut FormatSelectedPointsState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    const RULES: &[FieldRule] = &[
        FieldRule::new(&["hidden", "format_points.hidden", "fmt_pts.hidden"], "hidden", FieldKind::OptBool),
        FieldRule::new(&["show_row_labels", "format_points.show_row_labels"], "show_row_labels", FieldKind::Bool),
        FieldRule::new(&["clear", "clear_overrides", "format_points.clear"], "clear_overrides", FieldKind::Bool),
    ];
    let mut patch = Map::new();
    let mut next_targets: Option<Vec<(usize, usize, Option<usize>)>> = None;

    for (key, val) in fields {
        if try_apply_rule(key, val, RULES, preset, &mut patch)? {
            continue;
        }
        match key.as_str() {
            "color" | "symbol_color" | "format_points.color" => {
                let parsed = if val.is_null() { None } else { Some(validate_point_color_str(parse_str_field(val, key)?, key)?) };
                patch.insert(String::from("color"), json!(parsed));
            }
            "fill_color" | "format_points.fill_color" => {
                let parsed = if val.is_null() { None } else { Some(validate_point_color_str(parse_str_field(val, key)?, key)?) };
                patch.insert(String::from("fill_color"), json!(parsed));
            }
            "symbol" | "shape" | "symbol_shape" | "format_points.symbol" => {
                let parsed = if val.is_null() { None } else { Some(validate_marker_shape_str(parse_str_field(val, key)?, key)?) };
                patch.insert(String::from("symbol"), json!(parsed));
            }
            "size_pt" | "format_points.size_pt" => {
                let parsed = parse_opt_finite_f64_with_locale(val, key, preset)?;
                if let Some(sz) = parsed
                    && sz <= 0.0
                {
                    return Err(format!("`{key}` must be finite and > 0"));
                }
                patch.insert(String::from("size_pt"), json!(parsed));
            }
            "border_width_pt" | "stroke_width_pt" | "format_points.border_width_pt" => {
                let parsed = parse_opt_finite_f64_with_locale(val, key, preset)?;
                if let Some(bw) = parsed
                    && bw < 0.0
                {
                    return Err(format!("`{key}` must be finite and >= 0"));
                }
                patch.insert(String::from("border_width_pt"), json!(parsed));
            }
            "fill_mode" | "format_points.fill_mode" => {
                let parsed = if val.is_null() {
                    None
                } else {
                    let s = parse_str_field(val, key)?;
                    let _ = parse_marker_fill_mode_val(val, key)?;
                    Some(s.trim().to_owned())
                };
                patch.insert(String::from("fill_mode"), json!(parsed));
            }
            "points" | "selected_points" => {
                if !val.is_null() {
                    next_targets = Some(crate::control_io::parse_selected_points_with_subcols(val, key)?);
                }
            }
            other => return Err(format!("unknown Format Selected Points field `{other}`")),
        }
    }

    merge_struct_json(state, &patch, &[])?;
    if let Some(targets) = next_targets {
        state.set_selected_targets(targets);
    }
    Ok(())
}

pub fn patch_change_graph_type(state: &mut ChangeGraphTypeState, fields: &Map<String, Value>) -> Result<(), String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        match key.as_str() {
            "option_id" | "selected_option_id" | "graph_type.selected" => {
                let opt = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                patch.insert(String::from("selected_option_id"), json!(opt));
            }
            "summary_measure" | "graph_type.summary_measure" => {
                let raw = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                let sm = parse_summary_measure_str(raw)?;
                patch.insert(String::from("summary_measure"), serde_json::to_value(sm).map_err(|e| e.to_string())?);
            }
            "error_bar_metric" | "graph_type.error_bar_metric" => {
                let raw = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                let ebm = parse_error_bar_metric_str(raw)?;
                patch.insert(String::from("error_bar_metric"), serde_json::to_value(ebm).map_err(|e| e.to_string())?);
            }
            "survival_metric" | "graph_type.survival.metric" => {
                let raw = val.as_str().ok_or_else(|| format!("`{key}` must be a string"))?;
                let sm = parse_survival_metric_str(raw)?;
                patch.insert(String::from("survival_metric"), serde_json::to_value(sm).map_err(|e| e.to_string())?);
            }
            "survival_staircase" | "graph_type.survival.staircase" => {
                let b = val.as_bool().ok_or_else(|| format!("`{key}` must be boolean"))?;
                patch.insert(String::from("survival_staircase"), json!(b));
            }
            "survival_error_display" | "graph_type.survival.error_display" => {
                let sed = if val.is_null() {
                    None
                } else {
                    let raw = val.as_str().ok_or_else(|| format!("`{key}` must be a string or null"))?;
                    if raw.trim().eq_ignore_ascii_case("none") { None } else { Some(parse_error_bar_metric_str(raw)?) }
                };
                patch.insert(String::from("survival_error_display"), serde_json::to_value(sed).map_err(|e| e.to_string())?);
            }
            other => return Err(format!("unknown ChangeGraphType field `{other}`")),
        }
    }
    merge_struct_json(state, &patch, &[])
}

pub(super) fn convert_target_kind_from_val(val: &Value) -> Result<curvebook_doc::TableKind, String> {
    let raw = parse_str_field(val, "target_kind")?;
    parse_table_kind_str(raw)
}
