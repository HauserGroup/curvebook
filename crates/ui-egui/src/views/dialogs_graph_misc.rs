//! Graph/layout canvas view, rulers, command palette, and descriptor-driven modal helpers (`R10.3`, `R10.6`, `R10.15`).

use std::sync::Arc;

use curvebook_chart::{ChartScene, PointPt, compile_graph, preflight_check_scene};
use curvebook_doc::{
    EquationDisplayMode, ErrorBarMetric, GraphKind, JournalWidthPreset, LayoutItem, PageOrientation, PanelBadgeStyle, RectPt, SheetId, SummaryMeasure,
};
use curvebook_palette::{CanvasPreviewFilter, Rgba8};
use curvebook_render::{RasterImage, RenderTargetSpec, TiffCompression, apply_canvas_preview_filter, render_scene};
use curvebook_tools::canvas::{SnapOrientation, SnapSource};
use curvebook_tools::{PhysicalUnit, build_ruler_ticks};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::compositor::{FramePalette, WorkspacePixelRect};
use crate::dialogs_canvas::{ActiveDialog, ExportFigureFormat, GRAPH_TYPE_OPTIONS, GraphTypeOption, ThumbnailGlyphKind, variable_role_color};
use crate::dialogs_r9::RandomizationSchemeKind;
use crate::interaction::{CanvasDragTarget, ScenePaperMapping, UiEventOutcome};
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::canvas_viewport::{
    CanvasViewportGeometry, compute_aspect_ratio_paper_mapping, should_show_graph_series_chip, should_show_layout_panel_chrome,
};
use crate::layout::paint::{PaintOp, PaintOpKind};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldSpec, ToggleWidgetSpec, emit_button, emit_scrollbars_for_meta, emit_tab, emit_text_field, emit_toggle,
    paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

use super::a11y::labeled_meta;
use super::analyze_picker::scaled_u32;

const DOMAIN_W5G1: &str = "dialogs_graph_misc";

const fn default_true() -> bool {
    true
}

fn default_export_color_space() -> String {
    String::from("sRGB")
}

fn default_export_unit() -> String {
    String::from("px")
}

fn w5_tr<'a>(locale: &str, key: &str, fallback: &'a str) -> &'a str {
    let norm = crate::i18n::normalize_locale(locale);
    crate::i18n::try_tr(norm, key).or_else(|| crate::i18n::try_tr("en", key)).unwrap_or(fallback)
}

/// Transient view state owned by the graph / layout / miscellaneous dialogs shard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogsGraphMiscViewState {
    #[serde(default)]
    pub active_section: String,
    #[serde(default)]
    pub zoom_menu_slot: Option<String>,
    #[serde(default = "default_true")]
    pub hook_expand_info: bool,
    #[serde(default = "default_true")]
    pub hook_expand_analysis: bool,
    #[serde(default = "default_true")]
    pub hook_expand_document: bool,
    #[serde(default)]
    pub graph_type_show_all: bool,
    #[serde(default)]
    pub export_transparent_bg: bool,
    #[serde(default = "default_export_color_space")]
    pub export_color_space: String,
    #[serde(default = "default_export_unit")]
    pub export_unit: String,
    #[serde(default)]
    pub sim_selected_col_idx: usize,
    #[serde(default)]
    pub layout_revision: u64,
}

impl Default for DialogsGraphMiscViewState {
    fn default() -> Self {
        Self {
            active_section: String::new(),
            zoom_menu_slot: None,
            hook_expand_info: true,
            hook_expand_analysis: true,
            hook_expand_document: true,
            graph_type_show_all: false,
            export_transparent_bg: false,
            export_color_space: default_export_color_space(),
            export_unit: default_export_unit(),
            sim_selected_col_idx: 0,
            layout_revision: 0,
        }
    }
}

fn handled_outcome(mutated: bool) -> Result<UiEventOutcome, String> {
    let act = if mutated { "dialogs_graph_misc.updated" } else { "dialogs_graph_misc.ui" };
    Ok(UiEventOutcome { handled: true, action: String::from(act), ..UiEventOutcome::default() })
}

fn active_text<'a>(app: &'a UiApp, widget_id: &str, fallback: &'a str) -> &'a str {
    app.interaction.text_field_buffers.get(widget_id).map_or(fallback, |b| b.text.as_str())
}

fn buffer_text(app: &UiApp, widget_id: &str) -> Option<String> {
    app.interaction.text_field_buffers.get(widget_id).map(|b| b.text.clone())
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `dialogs_graph_misc` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let res = if let Some(res) = handle_zoom_pan_action(app, state, payload) {
        res
    } else if let Some(res) = handle_palette_action(app, payload) {
        res
    } else if let Some(res) = handle_graph_hook_export_action(app, state, payload) {
        res
    } else if let Some(res) = handle_remaining_dialog_action(app, state, payload) {
        res
    } else {
        Ok(UiEventOutcome::default())
    };
    if let Ok(ref out) = res
        && out.handled
    {
        // `compute_ui_state_fingerprint` hashes `app.view_shards`, so bumping `layout_revision`
        // ensures cached `LayoutTree` frames rebuild immediately after any `canvas_dialogs` mutation.
        state.layout_revision = state.layout_revision.wrapping_add(1);
    }
    res
}

fn apply_zoom_and_pan(app: &mut UiApp, zoom: Option<f64>, reset_pan: bool, pan_delta: Option<[f64; 2]>) {
    if let Some(z) = zoom {
        let clamped = z.clamp(0.25, 4.0);
        app.canvas_dialogs.canvas.zoom = clamped;
        app.canvas_dialogs.pan_zoom.zoom = clamped;
    }
    if reset_pan {
        app.canvas_dialogs.canvas.pan_pt = PointPt::new(0.0, 0.0);
        app.canvas_dialogs.pan_zoom.pan_pt = [0.0, 0.0];
    } else if let Some([dx, dy]) = pan_delta {
        let nx = (app.canvas_dialogs.canvas.pan_pt.x + dx).clamp(-1200.0, 1200.0);
        let ny = (app.canvas_dialogs.canvas.pan_pt.y + dy).clamp(-1200.0, 1200.0);
        app.canvas_dialogs.canvas.pan_pt = PointPt::new(nx, ny);
        app.canvas_dialogs.pan_zoom.pan_pt = [nx, ny];
    }
}

fn handle_zoom_pan_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(slot) = payload.strip_prefix("zoom:toggle_menu:") {
        state.zoom_menu_slot = if state.zoom_menu_slot.as_deref() == Some(slot) { None } else { Some(slot.to_owned()) };
        return Some(handled_outcome(false));
    }
    if let Some(rest) = payload.strip_prefix("zoom:set:")
        && let Some((_, preset)) = rest.split_once(':')
    {
        state.zoom_menu_slot = None;
        if preset == "fit" {
            apply_zoom_and_pan(app, Some(1.0), true, None);
            return Some(handled_outcome(true));
        }
        if let Ok(pct) = preset.parse::<f64>() {
            apply_zoom_and_pan(app, Some(pct / 100.0), false, None);
            return Some(handled_outcome(true));
        }
    }
    if let Some(rest) = payload.strip_prefix("zoom:step:")
        && let Some((_, dir)) = rest.split_once(':')
    {
        state.zoom_menu_slot = None;
        let delta = if dir == "in" { 0.25 } else { -0.25 };
        let next = app.canvas_dialogs.canvas.zoom + delta;
        apply_zoom_and_pan(app, Some(next), false, None);
        return Some(handled_outcome(true));
    }
    if let Some(rest) = payload.strip_prefix("pan:reset:") {
        let _ = rest;
        apply_zoom_and_pan(app, None, true, None);
        return Some(handled_outcome(true));
    }
    if let Some(rest) = payload.strip_prefix("pan:nudge:") {
        let mut parts = rest.split(':');
        let _slot = parts.next();
        if let (Some(dx_s), Some(dy_s)) = (parts.next(), parts.next())
            && let (Ok(dx), Ok(dy)) = (dx_s.parse::<f64>(), dy_s.parse::<f64>())
        {
            apply_zoom_and_pan(app, None, false, Some([dx, dy]));
            return Some(handled_outcome(true));
        }
    }
    if payload.starts_with("pan:scroll_h:") {
        let delta = if payload.ends_with(":dec") { -48.0 } else { 48.0 };
        apply_zoom_and_pan(app, None, false, Some([delta, 0.0]));
        return Some(handled_outcome(true));
    }
    if payload.starts_with("pan:scroll_v:") {
        let delta = if payload.ends_with(":dec") { -48.0 } else { 48.0 };
        apply_zoom_and_pan(app, None, false, Some([0.0, delta]));
        return Some(handled_outcome(true));
    }
    if let Some(panel_id) = payload.strip_prefix("chrome:select_panel:") {
        let next = if app.canvas_dialogs.canvas.selected_object_id.as_deref() == Some(panel_id) { None } else { Some(panel_id.to_owned()) };
        app.canvas_dialogs.canvas.selected_object_id = next;
        return Some(handled_outcome(true));
    }
    if let Some(s_idx) = payload.strip_prefix("chrome:select_series:") {
        let key = format!("series.{s_idx}");
        let next = if app.canvas_dialogs.canvas.selected_object_id.as_deref() == Some(key.as_str()) { None } else { Some(key) };
        app.canvas_dialogs.canvas.selected_object_id = next;
        return Some(handled_outcome(true));
    }
    None
}

fn handle_palette_action(app: &mut UiApp, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    let matches = app.canvas_dialogs.command_palette.matches_for_session(app.session());
    match payload {
        "palette:prev" => {
            if !matches.is_empty() {
                let idx = app.canvas_dialogs.command_palette.selected_index.min(matches.len() - 1);
                app.canvas_dialogs.command_palette.selected_index = if idx == 0 { matches.len() - 1 } else { idx - 1 };
            }
            Some(handled_outcome(false))
        }
        "palette:next" => {
            if !matches.is_empty() {
                let idx = app.canvas_dialogs.command_palette.selected_index;
                app.canvas_dialogs.command_palette.selected_index = (idx + 1) % matches.len();
            }
            Some(handled_outcome(false))
        }
        "palette:run" => {
            let idx = app.canvas_dialogs.command_palette.selected_index;
            let chosen = matches.get(idx).or_else(|| matches.first()).map(|e| e.id.to_owned());
            if let Some(cmd_id) = chosen {
                app.canvas_dialogs.close_dialog();
                app.focus_widget_id = None;
                return Some(
                    app.execute_command(&cmd_id, json!({}))
                        .map(|_| UiEventOutcome {
                            handled: true,
                            action: format!("palette.run:{cmd_id}"),
                            command_id: Some(cmd_id),
                            ..UiEventOutcome::default()
                        })
                        .map_err(|err| err.to_string()),
                );
            }
            Some(handled_outcome(false))
        }
        _ => payload.strip_prefix("palette:select:").and_then(|raw| raw.parse::<usize>().ok()).map(|idx| {
            if idx < matches.len() {
                app.canvas_dialogs.command_palette.selected_index = idx;
            }
            handled_outcome(false)
        }),
    }
}

fn handle_graph_hook_export_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(opt_id) = payload.strip_prefix("graph_type:select:") {
        app.canvas_dialogs.change_graph_type.selected_option_id = opt_id.to_owned();
        return Some(handled_outcome(true));
    }
    if payload == "graph_type:toggle_all" {
        state.graph_type_show_all = !state.graph_type_show_all;
        return Some(handled_outcome(false));
    }
    if let Some(summary) = payload.strip_prefix("graph_type:summary:") {
        app.canvas_dialogs.change_graph_type.summary_measure = match summary {
            "median" => SummaryMeasure::Median,
            "geomean" => SummaryMeasure::GeometricMean,
            _ => SummaryMeasure::Mean,
        };
        return Some(handled_outcome(true));
    }
    if let Some(err_metric) = payload.strip_prefix("graph_type:errorbar:") {
        app.canvas_dialogs.change_graph_type.error_bar_metric = match err_metric {
            "sem" => ErrorBarMetric::Sem,
            "ci95" => ErrorBarMetric::ConfidenceInterval95,
            "iqr" => ErrorBarMetric::InterquartileRange,
            "range" => ErrorBarMetric::Range,
            _ => ErrorBarMetric::Sd,
        };
        return Some(handled_outcome(true));
    }
    if payload == "graph_type:apply" {
        if let Some(gid) = app.canvas_dialogs.change_graph_type.graph_sheet_id.clone() {
            let opt_id = app.canvas_dialogs.change_graph_type.selected_option_id.clone();
            let graph_kind = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == opt_id).map_or(GraphKind::Xy, |o| o.graph_kind);
            let res = app.execute_command("graph.setKind", json!({ "sheet_id": gid.as_str(), "kind": graph_kind }));
            return Some(match res {
                Ok(_) => handled_outcome(true),
                Err(err) => Err(err.to_string()),
            });
        }
        return Some(handled_outcome(false));
    }
    if let Some(cat) = payload.strip_prefix("hook:toggle_cat:") {
        match cat {
            "info" => state.hook_expand_info = !state.hook_expand_info,
            "analysis" => state.hook_expand_analysis = !state.hook_expand_analysis,
            "document" => state.hook_expand_document = !state.hook_expand_document,
            _ => {}
        }
        return Some(handled_outcome(false));
    }
    if payload == "hook:clear_search" {
        app.canvas_dialogs.hook_picker.search_query.clear();
        app.interaction.text_field_buffers.remove("hook.search_input");
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("hook:select:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(txt) = buffer_text(app, "hook.search_input") {
            app.canvas_dialogs.hook_picker.search_query = txt;
        }
        if idx < app.canvas_dialogs.hook_picker.options.len() {
            app.canvas_dialogs.hook_picker.selected_index = Some(idx);
        }
        return Some(handled_outcome(true));
    }
    if let Some(fmt) = payload.strip_prefix("export:fmt:") {
        app.canvas_dialogs.export_figure.format = match fmt {
            "svg" => ExportFigureFormat::Svg,
            "pdf" => ExportFigureFormat::Pdf,
            "png" => ExportFigureFormat::Png,
            "tiff_lzw" => ExportFigureFormat::Tiff(TiffCompression::Lzw),
            "tiff_none" => ExportFigureFormat::Tiff(TiffCompression::None),
            _ => ExportFigureFormat::Tiff(TiffCompression::Deflate),
        };
        if matches!(app.canvas_dialogs.export_figure.format, ExportFigureFormat::Pdf) {
            state.export_transparent_bg = false;
        }
        return Some(handled_outcome(true));
    }
    if let Some(raw_dpi) = payload.strip_prefix("export:dpi:")
        && let Ok(dpi) = raw_dpi.parse::<u16>()
    {
        app.canvas_dialogs.export_figure.dpi = dpi.clamp(72, 2400);
        return Some(handled_outcome(true));
    }
    if payload == "export:dpi_dec" || payload == "export:dpi_inc" {
        let cur = i32::from(app.canvas_dialogs.export_figure.dpi);
        let next = if payload == "export:dpi_inc" { cur + 50 } else { cur - 50 };
        app.canvas_dialogs.export_figure.dpi = u16::try_from(next.clamp(72, 1200)).unwrap_or(300);
        return Some(handled_outcome(true));
    }
    if let Some(unit) = payload.strip_prefix("export:unit:") {
        state.export_unit = match unit {
            "mm" => String::from("mm"),
            "in" => String::from("in"),
            _ => String::from("px"),
        };
        return Some(handled_outcome(false));
    }
    if let Some(dim) = payload.strip_prefix("export:size:") {
        let step: i32 = match state.export_unit.as_str() {
            "mm" => ((10.0 / 25.4) * f64::from(app.canvas_dialogs.export_figure.dpi)).round() as i32,
            "in" => (0.5 * f64::from(app.canvas_dialogs.export_figure.dpi)).round() as i32,
            _ => 120,
        }
        .max(16);
        match dim {
            "w_dec" => {
                app.canvas_dialogs.export_figure.width_px = app.canvas_dialogs.export_figure.width_px.saturating_sub(step as u32).max(160);
            }
            "w_inc" => {
                app.canvas_dialogs.export_figure.width_px = app.canvas_dialogs.export_figure.width_px.saturating_add(step as u32).min(9600);
            }
            "h_dec" => {
                app.canvas_dialogs.export_figure.height_px = app.canvas_dialogs.export_figure.height_px.saturating_sub(step as u32).max(120);
            }
            "h_inc" => {
                app.canvas_dialogs.export_figure.height_px = app.canvas_dialogs.export_figure.height_px.saturating_add(step as u32).min(9600);
            }
            _ => {}
        }
        return Some(handled_outcome(true));
    }
    if payload == "export:transparent_bg" {
        if !matches!(app.canvas_dialogs.export_figure.format, ExportFigureFormat::Pdf) {
            state.export_transparent_bg = !state.export_transparent_bg;
        }
        return Some(handled_outcome(true));
    }
    if let Some(cs) = payload.strip_prefix("export:cs:") {
        state.export_color_space = match cs {
            "p3" => String::from("Display-P3"),
            "gray" => String::from("Grayscale"),
            "cmyk" => String::from("CMYK-Proof"),
            _ => String::from("sRGB"),
        };
        return Some(handled_outcome(true));
    }
    None
}

fn handle_remaining_dialog_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(res) = handle_dup_change_fmt_exclude_action(app, state, payload) {
        return Some(res);
    }
    if let Some(res) = handle_layout_eq_rand_sim_print_action(app, state, payload) {
        return Some(res);
    }
    if let Some(sec) = payload.strip_prefix("about:tab:") {
        state.active_section = sec.to_owned();
        return Some(handled_outcome(false));
    }
    None
}

fn handle_dup_change_fmt_exclude_action(
    app: &mut UiApp,
    _state: &mut DialogsGraphMiscViewState,
    payload: &str,
) -> Option<Result<UiEventOutcome, String>> {
    if let Some(raw_idx) = payload.strip_prefix("change_data:cand:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(cand) = app.canvas_dialogs.change_data.candidates.get(idx)
            && cand.compatible
        {
            app.canvas_dialogs.change_data.selected_target_id = Some(cand.sheet_id.clone());
        }
        return Some(handled_outcome(true));
    }
    if let Some(raw_ds) = payload.strip_prefix("change_data:ds:")
        && let Ok(ds_idx) = raw_ds.parse::<usize>()
    {
        let sets = &mut app.canvas_dialogs.change_data.selected_data_sets;
        if let Some(pos) = sets.iter().position(|&x| x == ds_idx) {
            sets.remove(pos);
        } else {
            sets.push(ds_idx);
            sets.sort_unstable();
        }
        return Some(handled_outcome(true));
    }
    if let Some(color) = payload.strip_prefix("fmt_pts:color:") {
        let hex = match color {
            "black" => Some("#111827"),
            "red" => Some("#DC2626"),
            "blue" => Some("#2563EB"),
            "green" => Some("#16A34A"),
            "orange" => Some("#EA580C"),
            "purple" => Some("#7C3AED"),
            _ => None,
        };
        app.canvas_dialogs.format_points.color = hex.map(str::to_owned);
        app.canvas_dialogs.format_points.fill_color = hex.map(str::to_owned);
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if let Some(sym) = payload.strip_prefix("fmt_pts:sym:") {
        app.canvas_dialogs.format_points.symbol = match sym {
            "none" => None,
            other => Some(other.to_owned()),
        };
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:size_dec" || payload == "fmt_pts:size_inc" {
        let cur = app.canvas_dialogs.format_points.size_pt.unwrap_or(5.0);
        let next = if payload == "fmt_pts:size_inc" { cur + 1.5 } else { cur - 1.5 };
        app.canvas_dialogs.format_points.size_pt = Some(next.clamp(2.0, 24.0));
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:bw_dec" || payload == "fmt_pts:bw_inc" {
        let cur = app.canvas_dialogs.format_points.border_width_pt.unwrap_or(1.0);
        let next = if payload == "fmt_pts:bw_inc" { cur + 0.5 } else { cur - 0.5 };
        app.canvas_dialogs.format_points.border_width_pt = Some(next.clamp(0.5, 6.0));
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:fill_mode" {
        let next = if app.canvas_dialogs.format_points.fill_mode.as_deref() == Some("open") { "filled" } else { "open" };
        app.canvas_dialogs.format_points.fill_mode = Some(String::from(next));
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:hidden" {
        let next = !app.canvas_dialogs.format_points.hidden.unwrap_or(false);
        app.canvas_dialogs.format_points.hidden = Some(next);
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:show_row_labels" {
        app.canvas_dialogs.format_points.show_row_labels = !app.canvas_dialogs.format_points.show_row_labels;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:clear_overrides" {
        app.canvas_dialogs.format_points.clear_overrides = !app.canvas_dialogs.format_points.clear_overrides;
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("exclude:tag:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(txt) = buffer_text(app, "exclude.note_input") {
            app.canvas_dialogs.exclude_with_reason.note = txt;
        }
        if let Some(tag) = crate::dialogs_canvas::ExcludeWithReasonDialogState::preset_reason_tags().get(idx) {
            app.canvas_dialogs.exclude_with_reason.reason_tag = (*tag).to_owned();
        }
        return Some(handled_outcome(true));
    }
    if payload == "exclude:apply_note" {
        if let Some(txt) = buffer_text(app, "exclude.note_input") {
            app.canvas_dialogs.exclude_with_reason.note = txt;
        }
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("apply_style:donor:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some((sid, _)) = app.canvas_dialogs.apply_style.donor_candidates.get(idx) {
            app.canvas_dialogs.apply_style.donor_graph_id = Some(sid.clone());
        }
        return Some(handled_outcome(true));
    }
    if let Some(field) = payload.strip_prefix("apply_style:toggle:") {
        match field {
            "palette" => app.canvas_dialogs.apply_style.clone_palette = !app.canvas_dialogs.apply_style.clone_palette,
            "frame" => app.canvas_dialogs.apply_style.clone_frame_and_axes = !app.canvas_dialogs.apply_style.clone_frame_and_axes,
            "legend" => {
                app.canvas_dialogs.apply_style.clone_error_bars_and_legend = !app.canvas_dialogs.apply_style.clone_error_bars_and_legend;
            }
            _ => {}
        }
        return Some(handled_outcome(true));
    }
    None
}

fn handle_layout_eq_rand_sim_print_action(
    app: &mut UiApp,
    state: &mut DialogsGraphMiscViewState,
    payload: &str,
) -> Option<Result<UiEventOutcome, String>> {
    if let Some(res) = handle_page_layout_eq_action(app, payload) {
        return Some(res);
    }
    if let Some(res) = handle_rand_sim_action(app, state, payload) {
        return Some(res);
    }
    handle_print_action(app, payload)
}

fn handle_page_layout_eq_action(app: &mut UiApp, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(raw_idx) = payload.strip_prefix("page_layout:preset:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        let presets = [
            JournalWidthPreset::SingleColumn85Mm,
            JournalWidthPreset::OneAndHalfColumn120Mm,
            JournalWidthPreset::DoubleColumn174Mm,
            JournalWidthPreset::A4Portrait,
            JournalWidthPreset::A4Landscape,
        ];
        if let Some(&preset) = presets.get(idx) {
            app.canvas_dialogs.page_layout.apply_preset(preset);
        }
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:orientation" {
        app.canvas_dialogs.page_layout.orientation = match app.canvas_dialogs.page_layout.orientation {
            PageOrientation::Portrait => PageOrientation::Landscape,
            PageOrientation::Landscape => PageOrientation::Portrait,
        };
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:rows_dec" || payload == "page_layout:rows_inc" {
        let cur = app.canvas_dialogs.page_layout.rows;
        app.canvas_dialogs.page_layout.rows = if payload == "page_layout:rows_inc" { (cur + 1).min(8) } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:cols_dec" || payload == "page_layout:cols_inc" {
        let cur = app.canvas_dialogs.page_layout.cols;
        app.canvas_dialogs.page_layout.cols = if payload == "page_layout:cols_inc" { (cur + 1).min(8) } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:arrange" {
        let graph_ids: Vec<SheetId> = app.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
        app.canvas_dialogs.page_layout.arrange_grid(&graph_ids, 24.0, 12.0);
        return Some(handled_outcome(true));
    }
    if let Some(badge) = payload.strip_prefix("page_layout:badge:") {
        app.canvas_dialogs.page_layout.panel_badges = match badge {
            "lower" => PanelBadgeStyle::LowerCaseLetters,
            "none" => PanelBadgeStyle::None,
            _ => PanelBadgeStyle::UpperCaseLetters,
        };
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:add_callout" {
        if let Some(txt) = buffer_text(app, "page_layout.callout_input") {
            app.canvas_dialogs.page_layout.new_callout_markdown = txt;
        }
        let md = app.canvas_dialogs.page_layout.new_callout_markdown.trim().to_owned();
        if !md.is_empty() {
            let next_id = format!("callout-{}", app.canvas_dialogs.page_layout.items.len() + 1);
            app.canvas_dialogs.page_layout.items.push(LayoutItem::TextCallout {
                id: next_id,
                markdown: md,
                bounds_pt: RectPt::new(24.0, 24.0, 180.0, 28.0),
                font_size_pt: 11.0,
            });
        }
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("eq:result:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(rid) = app.canvas_dialogs.insert_model_equation.available_result_ids.get(idx).cloned() {
            app.canvas_dialogs.insert_model_equation.result_sheet_id = Some(rid);
        }
        return Some(handled_outcome(true));
    }
    if payload == "eq:ds_dec" || payload == "eq:ds_inc" {
        let cur = app.canvas_dialogs.insert_model_equation.data_set_index;
        app.canvas_dialogs.insert_model_equation.data_set_index = if payload == "eq:ds_inc" { cur + 1 } else { cur.saturating_sub(1) };
        return Some(handled_outcome(true));
    }
    if payload == "eq:font_dec" || payload == "eq:font_inc" {
        let cur = app.canvas_dialogs.insert_model_equation.font_size_pt;
        let next = if payload == "eq:font_inc" { cur + 1.0 } else { cur - 1.0 };
        app.canvas_dialogs.insert_model_equation.font_size_pt = next.clamp(6.0, 36.0);
        return Some(handled_outcome(true));
    }
    if payload == "eq:bordered" {
        app.canvas_dialogs.insert_model_equation.bordered = !app.canvas_dialogs.insert_model_equation.bordered;
        return Some(handled_outcome(true));
    }
    if payload == "eq:toggle_mode" {
        let _ = app.canvas_dialogs.insert_model_equation.toggle_mode();
        return Some(handled_outcome(true));
    }
    if payload == "eq:update_existing" {
        app.canvas_dialogs.insert_model_equation.update_existing = !app.canvas_dialogs.insert_model_equation.update_existing;
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:align_origins" {
        app.canvas_dialogs.page_layout.align_plot_origins = !app.canvas_dialogs.page_layout.align_plot_origins;
        return Some(handled_outcome(true));
    }
    None
}

fn handle_rand_sim_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(preset) = payload.strip_prefix("rand_groups:arms:") {
        let raw = match preset {
            "three" => "Control, Low Dose, High Dose",
            "weighted" => "Control:1, Treatment:2",
            _ => "Control, Treatment",
        };
        let _ = app.canvas_dialogs.randomize_groups.set_groups_from_str(raw);
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:apply_text" {
        if let Some(txt) = buffer_text(app, "rand_groups.arms_input") {
            let _ = app.canvas_dialogs.randomize_groups.set_groups_from_str(&txt);
        }
        if let Some(txt) = buffer_text(app, "rand_groups.title_input") {
            app.canvas_dialogs.randomize_groups.title = txt;
        }
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:n_dec" || payload == "rand_groups:n_inc" {
        let cur = app.canvas_dialogs.randomize_groups.subject_count;
        app.canvas_dialogs.randomize_groups.subject_count =
            if payload == "rand_groups:n_inc" { (cur + 4).min(500) } else { cur.saturating_sub(4).max(2) };
        return Some(handled_outcome(true));
    }
    if let Some(scheme) = payload.strip_prefix("rand_groups:scheme:") {
        app.canvas_dialogs.randomize_groups.scheme =
            if scheme == "block" { RandomizationSchemeKind::PermutedBlock } else { RandomizationSchemeKind::CompleteBalanced };
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:block_dec" || payload == "rand_groups:block_inc" {
        let cur = app.canvas_dialogs.randomize_groups.block_sizes.first().copied().unwrap_or(4);
        let next = if payload == "rand_groups:block_inc" { (cur + 2).min(64) } else { cur.saturating_sub(2).max(2) };
        app.canvas_dialogs.randomize_groups.block_sizes = vec![next];
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:seed_dec" || payload == "rand_groups:seed_inc" {
        let cur = app.canvas_dialogs.randomize_groups.seed;
        app.canvas_dialogs.randomize_groups.seed = if payload == "rand_groups:seed_inc" { cur + 1 } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:toggle_info" {
        let next = if app.canvas_dialogs.randomize_groups.target_info_sheet_id.is_some() {
            None
        } else {
            app.document().info_sheets.first().map(|s| s.id.clone())
        };
        app.canvas_dialogs.randomize_groups.target_info_sheet_id = next;
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("rand_sample:src:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(ds) = app.document().data_sheets.get(idx) {
            app.canvas_dialogs.random_sample_rows.source_sheet_id = Some(ds.id.clone());
        }
        return Some(handled_outcome(true));
    }
    if payload == "rand_sample:n_dec" || payload == "rand_sample:n_inc" {
        let cur = app.canvas_dialogs.random_sample_rows.sample_size;
        app.canvas_dialogs.random_sample_rows.sample_size =
            if payload == "rand_sample:n_inc" { (cur + 5).min(5000) } else { cur.saturating_sub(5).max(1) };
        return Some(handled_outcome(true));
    }
    if payload == "rand_sample:seed_dec" || payload == "rand_sample:seed_inc" {
        let cur = app.canvas_dialogs.random_sample_rows.seed;
        app.canvas_dialogs.random_sample_rows.seed = if payload == "rand_sample:seed_inc" { cur + 1 } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if let Some(raw) = payload.strip_prefix("rand_sample:stratify:") {
        app.canvas_dialogs.random_sample_rows.stratify_column = if raw == "none" { None } else { raw.parse::<usize>().ok() };
        return Some(handled_outcome(true));
    }
    if payload == "rand_sample:proportional" {
        app.canvas_dialogs.random_sample_rows.proportional_allocation = !app.canvas_dialogs.random_sample_rows.proportional_allocation;
        return Some(handled_outcome(true));
    }
    if payload == "sim_dist:multi_var" {
        app.canvas_dialogs.simulate_distributions.use_multiple_variables = !app.canvas_dialogs.simulate_distributions.use_multiple_variables;
        return Some(handled_outcome(true));
    }
    if payload == "sim_dist:rows_dec" || payload == "sim_dist:rows_inc" {
        let cur = app.canvas_dialogs.simulate_distributions.n_rows;
        app.canvas_dialogs.simulate_distributions.n_rows =
            if payload == "sim_dist:rows_inc" { (cur + 25).min(10_000) } else { cur.saturating_sub(25).max(5) };
        return Some(handled_outcome(true));
    }
    if payload == "sim_dist:seed_dec" || payload == "sim_dist:seed_inc" {
        let cur = app.canvas_dialogs.simulate_distributions.seed;
        app.canvas_dialogs.simulate_distributions.seed = if payload == "sim_dist:seed_inc" { cur + 1 } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("sim_dist:col:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        state.sim_selected_col_idx = idx.min(app.canvas_dialogs.simulate_distributions.columns.len().saturating_sub(1));
        return Some(handled_outcome(false));
    }
    if payload == "sim_dist:add_col" {
        let idx = app.canvas_dialogs.simulate_distributions.columns.len();
        if idx < 12 {
            let draft = crate::dialogs_r9::SimulatedDistColumnDraft { name: format!("Var {}", idx + 1), ..Default::default() };
            app.canvas_dialogs.simulate_distributions.columns.push(draft);
            state.sim_selected_col_idx = idx;
        }
        return Some(handled_outcome(true));
    }
    if let Some(dist_s) = payload.strip_prefix("sim_dist:dist:") {
        let idx = state.sim_selected_col_idx.min(app.canvas_dialogs.simulate_distributions.columns.len().saturating_sub(1));
        if let Some(col) = app.canvas_dialogs.simulate_distributions.columns.get_mut(idx) {
            col.distribution = match dist_s {
                "uniform" => crate::dialogs_r9::SimDistKind::Uniform,
                "lognormal" => crate::dialogs_r9::SimDistKind::LogNormal,
                "binomial" => crate::dialogs_r9::SimDistKind::Binomial,
                "poisson" => crate::dialogs_r9::SimDistKind::Poisson,
                "exponential" => crate::dialogs_r9::SimDistKind::Exponential,
                _ => crate::dialogs_r9::SimDistKind::Gaussian,
            };
        }
        return Some(handled_outcome(true));
    }
    if let Some(p_op) = payload.strip_prefix("sim_dist:param:") {
        return Some(apply_sim_dist_param_step(app, state, p_op));
    }
    None
}

fn apply_sim_dist_param_step(app: &mut UiApp, state: &DialogsGraphMiscViewState, p_op: &str) -> Result<UiEventOutcome, String> {
    let idx = state.sim_selected_col_idx.min(app.canvas_dialogs.simulate_distributions.columns.len().saturating_sub(1));
    if let Some(col) = app.canvas_dialogs.simulate_distributions.columns.get_mut(idx) {
        let s = if p_op.ends_with("inc") { 1.0 } else { -1.0 };
        if p_op.starts_with("p1_") {
            match col.distribution {
                crate::dialogs_r9::SimDistKind::Uniform => col.min += s,
                crate::dialogs_r9::SimDistKind::Gaussian => col.mean += s,
                crate::dialogs_r9::SimDistKind::LogNormal => col.mean_log += s * 0.25,
                crate::dialogs_r9::SimDistKind::Binomial => {
                    let next = i64::try_from(col.trials).unwrap_or(10) + if s > 0.0 { 1 } else { -1 };
                    col.trials = u64::try_from(next.clamp(1, 500)).unwrap_or(10);
                }
                crate::dialogs_r9::SimDistKind::Poisson => col.lambda = (col.lambda + s).clamp(0.5, 500.0),
                crate::dialogs_r9::SimDistKind::Exponential => col.rate = (col.rate + s * 0.25).clamp(0.05, 100.0),
            }
        } else if p_op.starts_with("p2_") {
            match col.distribution {
                crate::dialogs_r9::SimDistKind::Uniform => col.max = (col.max + s).max(col.min + 0.5),
                crate::dialogs_r9::SimDistKind::Gaussian => col.sd = (col.sd + s * 0.5).clamp(0.1, 1000.0),
                crate::dialogs_r9::SimDistKind::LogNormal => col.sd_log = (col.sd_log + s * 0.1).clamp(0.05, 10.0),
                crate::dialogs_r9::SimDistKind::Binomial => col.p = (col.p + s * 0.05).clamp(0.05, 0.95),
                crate::dialogs_r9::SimDistKind::Poisson | crate::dialogs_r9::SimDistKind::Exponential => {}
            }
        }
    }
    handled_outcome(true)
}

fn handle_print_action(app: &mut UiApp, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if payload == "print:printer:default" {
        app.canvas_dialogs.print.select_printer(None);
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("print:printer:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        let p = app.canvas_dialogs.print.discovered_printers.get(idx).cloned();
        app.canvas_dialogs.print.select_printer(p);
        return Some(handled_outcome(true));
    }
    if payload == "print:copies_dec" || payload == "print:copies_inc" {
        let cur = app.canvas_dialogs.print.copies;
        let next = if payload == "print:copies_inc" { cur.saturating_add(1) } else { cur.saturating_sub(1) };
        app.canvas_dialogs.print.update_copies(next);
        return Some(handled_outcome(true));
    }
    if payload == "print:scale_dec" || payload == "print:scale_inc" {
        let cur = app.canvas_dialogs.print.job.scale_percent;
        let next = if payload == "print:scale_inc" { cur + 10.0 } else { cur - 10.0 };
        app.canvas_dialogs.print.set_scale_percent(next);
        return Some(handled_outcome(true));
    }
    if payload == "print:bleed_dec" || payload == "print:bleed_inc" {
        let cur = app.canvas_dialogs.print.job.bleed_pt;
        let next = if payload == "print:bleed_inc" { cur + 2.0 } else { cur - 2.0 };
        app.canvas_dialogs.print.job.bleed_pt = next.clamp(0.0, 36.0);
        return Some(handled_outcome(true));
    }
    if payload == "print:crop_marks" {
        app.canvas_dialogs.print.job.crop_marks = !app.canvas_dialogs.print.job.crop_marks;
        return Some(handled_outcome(true));
    }
    if payload == "print:submit_to_os" {
        if !app.is_gui_only() {
            app.canvas_dialogs.print.submit_to_os = !app.canvas_dialogs.print.submit_to_os;
        }
        return Some(handled_outcome(true));
    }
    None
}

/// Draw canvas top/left physical rulers when visible and return the remaining center viewport rectangle.
pub fn draw_rulers_if_visible(builder: &mut LayoutBuilder<'_>, canvas: WorkspacePixelRect) -> LayoutRectPx {
    if !builder.app.presentation_mode && builder.app.canvas_dialogs.rulers_visible && canvas.width >= 48 && canvas.height >= 48 {
        let ruler_h = 16u32;
        let ruler_w = 24u32;
        let top = LayoutRectPx::from_workspace_rect(WorkspacePixelRect::new(canvas.x + ruler_w, canvas.y, canvas.width - ruler_w, ruler_h));
        let left = LayoutRectPx::from_workspace_rect(WorkspacePixelRect::new(canvas.x, canvas.y + ruler_h, ruler_w, canvas.height - ruler_h));
        builder.paint_fill(ZLayer::Workspace, None, top, builder.palette.panel);
        builder.paint_fill(ZLayer::Workspace, None, left, builder.palette.panel);
        let ticks = build_ruler_ticks(PhysicalUnit::Millimeters, 420.0, builder.app.canvas_dialogs.canvas.zoom, 0.0, 1.0);
        let tick_step = (top.width / ticks.len().max(1) as u32).max(16);
        for i in 0..ticks.len().min(6) {
            let tx = top.x + i32::try_from(i as u32 * tick_step).unwrap_or(0);
            builder.paint_line(ZLayer::Workspace, None, [tx, top.bottom() - 4], [tx, top.bottom()], builder.palette.border, 1);
        }
        LayoutRectPx::from_workspace_rect(WorkspacePixelRect::new(
            canvas.x + ruler_w,
            canvas.y + ruler_h,
            canvas.width - ruler_w,
            canvas.height - ruler_h,
        ))
    } else {
        LayoutRectPx::from_workspace_rect(canvas)
    }
}

/// Render an aspect-ratio-preserving graph or page-layout sheet canvas into `outer_rect`.
pub fn build_canvas_or_layout_view(builder: &mut LayoutBuilder<'_>, sheet_id: &SheetId, outer_rect: LayoutRectPx, slot_tag: &str) {
    let scene_res = builder.app.session().render_sheet_scene(sheet_id);
    let (scene_w_pt, scene_h_pt, scene_opt) = match scene_res {
        Ok(scene) => (scene.width_pt, scene.height_pt, Some(scene)),
        Err(_) => {
            if let Some(ls) = builder.app.document().layout_sheet(sheet_id) {
                let p = ls.spec.page_size_pt();
                (p.width, p.height, None)
            } else {
                (420.0, 297.0, None)
            }
        }
    };

    let geom = compute_aspect_ratio_paper_mapping(sheet_id.clone(), outer_rect, scene_w_pt, scene_h_pt, builder.app, builder.scale());
    builder.paint_fill(ZLayer::Workspace, None, geom.mat_rect_px, builder.palette.canvas_mat);

    let canvas_id = format!("canvas.surface.{slot_tag}.{}", sheet_id.as_str());
    let canvas_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(canvas_id),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: geom.mat_rect_px,
        a11y: labeled_meta(WidgetRole::Canvas, format!("Canvas {}", sheet_id.as_str()), FocusScope::Workspace, "Cmd+0"),
        state: WidgetVisualState::default(),
        action: WidgetAction::FocusWidget(format!("canvas.surface.{slot_tag}.{}", sheet_id.as_str())),
    });

    builder.tree.scene_papers.push(geom.mapping.clone());
    if !builder.app.presentation_mode && geom.mat_rect_px.width >= 60 && geom.mat_rect_px.height >= 60 {
        let p = geom.paper_rect_px;
        if slot_tag != "secondary" || builder.tree.canvas_title_rect.is_none() {
            builder.tree.canvas_title_rect = Some(WorkspacePixelRect::new(
                u32::try_from((geom.mat_rect_px.x + 8).max(0)).unwrap_or(0),
                u32::try_from((geom.mat_rect_px.y + 4).max(0)).unwrap_or(0),
                geom.mat_rect_px.width.saturating_sub(16),
                16,
            ));
        }
        if slot_tag != "secondary" || builder.tree.x_axis_title_rect.is_none() {
            let xr = LayoutRectPx::new(p.x + i32::try_from(p.width / 4).unwrap_or(0), p.bottom() - 24, (p.width / 2).max(16), 20);
            builder.tree.x_axis_title_rect = Some(xr.to_workspace_rect());
        }
        if slot_tag != "secondary" || builder.tree.y_axis_title_rect.is_none() {
            let y_local_x = crate::i18n::mirror_x_for_rtl(4, 24, p.width, builder.tree.is_rtl);
            let yr = LayoutRectPx::new(
                p.x + i32::try_from(y_local_x).unwrap_or(4),
                p.y + i32::try_from(p.height / 4).unwrap_or(0),
                24,
                (p.height / 2).max(16),
            );
            builder.tree.y_axis_title_rect = Some(yr.to_workspace_rect());
        }
    }

    if let Some(scene) = scene_opt {
        let spec = RenderTargetSpec {
            width_px: geom.base_paper_rect_px.width.max(1),
            height_px: geom.base_paper_rect_px.height.max(1),
            dpi: (builder.tree.metrics.scale_factor * 96.0).round().clamp(72.0, 384.0) as u16,
        };
        if let Ok(mut img) = render_scene(&scene, spec) {
            if builder.app.preview_filter != CanvasPreviewFilter::Normal {
                apply_canvas_preview_filter(&mut img, builder.app.preview_filter);
            }
            draw_scene_overlays(&mut img, builder.app, sheet_id, &scene, &geom, builder.palette);

            builder.push_clip(geom.mat_rect_px);
            let order = builder.next_order();
            builder.tree.paint_ops.push(PaintOp {
                z_layer: ZLayer::Workspace,
                paint_order: order,
                clip: Some(builder.active_clip_id()),
                owner_widget_id: Some(canvas_wid),
                kind: PaintOpKind::BlitImage { dst: geom.paper_rect_px, image: Arc::new(img) },
            });
            builder.pop_clip();
        }
    }

    draw_canvas_zoom_controls(builder, &geom, slot_tag);
}

/// Emit zoom-to-fit, zoom-percentage buttons, and pan scrollbars for `geom`.
pub fn draw_canvas_zoom_controls(builder: &mut LayoutBuilder<'_>, geom: &CanvasViewportGeometry, slot_tag: &str) {
    if !builder.app.presentation_mode && geom.zoom_bar_rect_px.height > 0 {
        builder.paint_fill(ZLayer::Workspace, None, geom.zoom_bar_rect_px, builder.palette.panel);
        builder.paint_line(
            ZLayer::Workspace,
            None,
            [geom.zoom_bar_rect_px.x, geom.zoom_bar_rect_px.y],
            [geom.zoom_bar_rect_px.right(), geom.zoom_bar_rect_px.y],
            builder.palette.border,
            1,
        );
        let pct = (geom.mapping.zoom * 100.0).round();
        let menu_open = builder.app.view_shards.dialogs_graph_misc.zoom_menu_slot.as_deref() == Some(slot_tag);
        let fit_label = w5_tr(&builder.app.locale, "w5.zoom.fit", "Fit Sheet");
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("canvas.{slot_tag}.zoom_fit"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: geom.zoom_fit_btn_px,
                label: fit_label,
                keyboard_path: "Cmd+0",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: (geom.mapping.zoom - 1.0).abs() < 0.02,
                shortcut: Some("Cmd+0"),
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::SetCanvasZoomFit,
            },
        );
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("canvas.{slot_tag}.zoom_pct"),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: geom.zoom_pct_btn_px,
                label: &format!("{pct:.0}%"),
                keyboard_path: "Cmd+=",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::CycleCanvasZoomPreset,
            },
        );

        let bar = geom.zoom_bar_rect_px;
        let btn_y = geom.zoom_pct_btn_px.y.max(bar.y + 2);
        let btn_h = geom.zoom_pct_btn_px.height.max(18).min(bar.height.saturating_sub(4));
        let occ_left = geom.zoom_fit_btn_px.x.min(geom.zoom_pct_btn_px.x);
        let occ_right = geom.zoom_fit_btn_px.right().max(geom.zoom_pct_btn_px.right());
        let left_free = (occ_left - (bar.x + 6)).max(0);
        let right_free = (bar.right() - 6 - occ_right).max(0);
        let use_right = right_free >= left_free;
        let min_x = if use_right { occ_right + 6 } else { bar.x + 6 };
        let max_x = if use_right { bar.right() - 6 } else { occ_left - 4 };
        let mut cursor_x = if use_right { min_x } else { max_x };
        let mut alloc_slot = |w: i32, gap: i32| -> Option<i32> {
            if use_right {
                if cursor_x + w <= max_x {
                    let rx = cursor_x;
                    cursor_x += w + gap;
                    Some(rx)
                } else {
                    None
                }
            } else if cursor_x - w >= min_x {
                let rx = cursor_x - w;
                cursor_x = rx - gap;
                Some(rx)
            } else {
                None
            }
        };
        let mut select_btn_rect = None;

        let zoom_sel_w = 88;
        if let Some(rx) = alloc_slot(zoom_sel_w, 4) {
            let rect = LayoutRectPx::new(rx, btn_y, zoom_sel_w as u32, btn_h);
            select_btn_rect = Some(rect);
            let sel_prefix = w5_tr(&builder.app.locale, "w5.zoom.selector", "Zoom");
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("canvas.{slot_tag}.zoom_select"),
                    parent: None,
                    z_layer: ZLayer::Workspace,
                    rect,
                    label: &format!("{sel_prefix} {pct:.0}%"),
                    keyboard_path: "ZoomPreset",
                    focus_scope: FocusScope::Workspace,
                    tab_order: None,
                    primary: false,
                    selected: menu_open,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from(DOMAIN_W5G1), payload: format!("zoom:toggle_menu:{slot_tag}") },
                },
            );
        }

        let extra_btns: &[(&str, i32, &str, &str)] = &[
            ("zoom_out", 22, "-", "out"),
            ("zoom_in", 22, "+", "in"),
            ("pan.reset", 54, w5_tr(&builder.app.locale, "w5.pan.reset", "Center"), "reset"),
            ("pan.left", 22, "<", "pan_l"),
            ("pan.right", 22, ">", "pan_r"),
            ("pan.up", 22, "^", "pan_u"),
            ("pan.down", 22, "v", "pan_d"),
        ];
        for (suffix, w, lbl, kind) in extra_btns {
            let Some(rx) = alloc_slot(*w, 3) else {
                break;
            };
            let payload = match *kind {
                "in" => format!("zoom:step:{slot_tag}:in"),
                "out" => format!("zoom:step:{slot_tag}:out"),
                "reset" => format!("pan:reset:{slot_tag}"),
                "pan_r" => format!("pan:nudge:{slot_tag}:40:0"),
                "pan_l" => format!("pan:nudge:{slot_tag}:-40:0"),
                "pan_d" => format!("pan:nudge:{slot_tag}:0:40"),
                _ => format!("pan:nudge:{slot_tag}:0:-40"),
            };
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("canvas.{slot_tag}.{suffix}"),
                    parent: None,
                    z_layer: ZLayer::Workspace,
                    rect: LayoutRectPx::new(rx, btn_y, *w as u32, btn_h),
                    label: lbl,
                    keyboard_path: suffix,
                    focus_scope: FocusScope::Workspace,
                    tab_order: None,
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from(DOMAIN_W5G1), payload },
                },
            );
        }

        if menu_open && let Some(anchor) = select_btn_rect {
            emit_zoom_preset_popup(builder, geom, anchor, slot_tag);
        }
    }

    if let Some((track, _thumb)) = geom.h_scrollbar {
        let h_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(format!("canvas.{slot_tag}.h_scrollbar")),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: track,
            a11y: labeled_meta(WidgetRole::ScrollBar, "Horizontal Canvas Pan", FocusScope::Workspace, "ArrowLeft/Right"),
            state: widget_state(builder, &format!("canvas.{slot_tag}.h_scrollbar"), ControlEnablement::enabled()),
            action: WidgetAction::Custom { domain: String::from(DOMAIN_W5G1), payload: format!("pan:scroll_h:{slot_tag}:inc") },
        });
        builder.paint_fill(ZLayer::Workspace, Some(&h_wid), track, builder.palette.panel);
        let thumb_w = (track.width / 3).max(8).min(track.width);
        let frac = ((0.5 - geom.mapping.pan_pt[0] / 800.0).clamp(0.0, 1.0)) * f64::from(track.width.saturating_sub(thumb_w));
        let thumb_rect = LayoutRectPx::new(track.x + frac.round() as i32, track.y + 1, thumb_w, track.height.saturating_sub(2).max(2));
        builder.paint_fill(ZLayer::Workspace, Some(&h_wid), thumb_rect, builder.palette.muted);
    }
    if let Some((track, _thumb)) = geom.v_scrollbar {
        let v_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(format!("canvas.{slot_tag}.v_scrollbar")),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: track,
            a11y: labeled_meta(WidgetRole::ScrollBar, "Vertical Canvas Pan", FocusScope::Workspace, "ArrowUp/Down"),
            state: widget_state(builder, &format!("canvas.{slot_tag}.v_scrollbar"), ControlEnablement::enabled()),
            action: WidgetAction::Custom { domain: String::from(DOMAIN_W5G1), payload: format!("pan:scroll_v:{slot_tag}:inc") },
        });
        builder.paint_fill(ZLayer::Workspace, Some(&v_wid), track, builder.palette.panel);
        let thumb_h = (track.height / 3).max(8).min(track.height);
        let frac = ((0.5 - geom.mapping.pan_pt[1] / 800.0).clamp(0.0, 1.0)) * f64::from(track.height.saturating_sub(thumb_h));
        let thumb_rect = LayoutRectPx::new(track.x + 1, track.y + frac.round() as i32, track.width.saturating_sub(2).max(2), thumb_h);
        builder.paint_fill(ZLayer::Workspace, Some(&v_wid), thumb_rect, builder.palette.muted);
    }

    if !builder.app.presentation_mode && slot_tag == "main" {
        emit_scene_chrome_hotspots(builder, geom);
    }
}

fn emit_zoom_preset_popup(builder: &mut LayoutBuilder<'_>, geom: &CanvasViewportGeometry, anchor: LayoutRectPx, slot_tag: &str) {
    let presets: &[(&str, &str, f64)] = &[
        ("fit", "Fit (100%)", 1.0),
        ("50", "50%", 0.50),
        ("75", "75%", 0.75),
        ("100", "100%", 1.00),
        ("125", "125%", 1.25),
        ("150", "150%", 1.50),
        ("200", "200%", 2.00),
    ];
    let row_h = 22u32;
    let menu_w = 112u32;
    let menu_h = u32::try_from(presets.len()).unwrap_or(7) * row_h + 8;
    let vp = geom.outer_pane_px;
    let max_x = vp.right() - i32::try_from(menu_w).unwrap_or(112) - 4;
    let menu_x = anchor.x.clamp(vp.x + 4, max_x.max(vp.x + 4));
    let menu_y = (anchor.y - i32::try_from(menu_h).unwrap_or(162) - 4).max(vp.y + 4);
    let popup_rect = LayoutRectPx::new(menu_x, menu_y, menu_w, menu_h);

    let popup_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("canvas.{slot_tag}.zoom_menu")),
        parent: None,
        z_layer: ZLayer::DropdownMenu,
        rect_px: popup_rect,
        a11y: labeled_meta(WidgetRole::ContextMenu, "Zoom Presets", FocusScope::Workspace, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::DropdownMenu, Some(&popup_wid), popup_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::DropdownMenu, Some(&popup_wid), popup_rect, builder.palette.accent, 1);

    for (idx, (key, lbl, val)) in presets.iter().enumerate() {
        let ry = menu_y + 4 + i32::try_from(idx).unwrap_or(0) * i32::try_from(row_h).unwrap_or(22);
        let active = if *key == "fit" {
            (geom.mapping.zoom - 1.0).abs() < 0.02 && geom.mapping.pan_pt[0].abs() < 0.5 && geom.mapping.pan_pt[1].abs() < 0.5
        } else {
            (geom.mapping.zoom - *val).abs() < 0.02
        };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("canvas.{slot_tag}.zoom_preset.{key}"),
                parent: Some(popup_wid.clone()),
                z_layer: ZLayer::DropdownMenu,
                rect: LayoutRectPx::new(menu_x + 4, ry, menu_w - 8, row_h - 2),
                label: lbl,
                keyboard_path: key,
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: active,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from(DOMAIN_W5G1), payload: format!("zoom:set:{slot_tag}:{key}") },
            },
        );
    }
}

fn emit_scene_chrome_hotspots(builder: &mut LayoutBuilder<'_>, geom: &CanvasViewportGeometry) {
    let paper = geom.paper_rect_px;
    if paper.width < 48 || paper.height < 48 {
        return;
    }
    let paper_cx = paper.x + i32::try_from(paper.width / 2).unwrap_or(0);
    let paper_cy = paper.y + i32::try_from(paper.height / 2).unwrap_or(0);
    let stealth_surface = builder.palette.surface.with_alpha(1);

    if let Some(gs) = builder.app.document().graph_sheet(&geom.mapping.sheet_id) {
        let series_count = builder
            .app
            .document()
            .data_sheet(&gs.spec.primary_source_sheet_id)
            .map_or(0, |ds| crate::inspect::table_dimensions_summary(&ds.archetype).1)
            .min(12);
        for s_idx in 0..series_count {
            let rx = paper.x + 6 + i32::try_from(s_idx).unwrap_or(0) * 16;
            let ry = paper.y + 6;
            let rect = LayoutRectPx::new(rx, ry, 14, 14);
            if rect.right() > paper.right() || rect.bottom() > paper.bottom() || rect.contains_px(paper_cx, paper_cy) {
                continue;
            }
            let id = format!("canvas.series.{s_idx}");
            let active = should_show_graph_series_chip(builder.app, s_idx);
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(id.as_str()),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: rect,
                a11y: labeled_meta(WidgetRole::Button, format!("Series {}", s_idx + 1), FocusScope::Workspace, "Enter"),
                state: widget_state(builder, &id, ControlEnablement::enabled()),
                action: WidgetAction::Custom { domain: String::from(DOMAIN_W5G1), payload: format!("chrome:select_series:{s_idx}") },
            });
            if active {
                builder.paint_stroke(ZLayer::Workspace, Some(&wid), rect, builder.palette.accent, 1);
            } else {
                builder.paint_fill(ZLayer::Workspace, Some(&wid), rect, stealth_surface);
            }
        }
    }

    if let Some(ls) = builder.app.document().layout_sheet(&geom.mapping.sheet_id) {
        for (idx, item) in ls.spec.items.iter().enumerate() {
            let panel_id = format!("panel.{idx}");
            let bounds_pt = match item {
                LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
                    *bounds_pt
                }
            };
            let Some([px0, py0]) = geom.mapping.scene_pt_to_pixel(PointPt::new(bounds_pt.x, bounds_pt.y)) else {
                continue;
            };
            let rx = (px0.round() as i32 + 4).clamp(paper.x + 2, (paper.right() - 28).max(paper.x + 2));
            let ry = (py0.round() as i32 + 4).clamp(paper.y + 2, (paper.bottom() - 20).max(paper.y + 2));
            let mut rect = LayoutRectPx::new(rx, ry, 24, 16);
            if rect.contains_px(paper_cx, paper_cy) {
                if paper.x + 4 + 24 < paper_cx {
                    rect.x = paper.x + 4;
                } else {
                    continue;
                }
            }
            let id = format!("canvas.layout_panel.{panel_id}");
            let active = should_show_layout_panel_chrome(builder.app, &panel_id);
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(id.as_str()),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: rect,
                a11y: labeled_meta(WidgetRole::Button, format!("Layout Panel {}", idx + 1), FocusScope::Workspace, "Enter"),
                state: widget_state(builder, &id, ControlEnablement::enabled()),
                action: WidgetAction::Custom { domain: String::from(DOMAIN_W5G1), payload: format!("chrome:select_panel:{panel_id}") },
            });
            if active {
                let badge = ls.spec.panel_badges.badge_for(idx).unwrap_or_else(|| format!("P{}", idx + 1));
                builder.paint_fill(ZLayer::Workspace, Some(&wid), rect, builder.palette.accent);
                let st = UiTextStyle::ui_sans((10.0 * builder.scale()).clamp(8.5, 16.0));
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect,
                        pad_x: 3,
                        text: &badge,
                        style: &st,
                        color: builder.palette.surface,
                        scroll_container: None,
                    },
                );
            } else {
                builder.paint_fill(ZLayer::Workspace, Some(&wid), rect, stealth_surface);
            }
        }
    }
}

/// Draw active snap guides, hover/selected series chips, and layout panel selection chrome onto `img`.
pub fn draw_scene_overlays(
    img: &mut RasterImage,
    app: &UiApp,
    sheet_id: &SheetId,
    scene: &ChartScene,
    geom: &CanvasViewportGeometry,
    pal: &FramePalette,
) {
    let pw = img.width_px;
    let ph = img.height_px;
    let paper_local = WorkspacePixelRect::new(0, 0, pw, ph);
    let local_map = ScenePaperMapping {
        sheet_id: sheet_id.clone(),
        paper: paper_local,
        scene_width_pt: scene.width_pt.max(1.0),
        scene_height_pt: scene.height_pt.max(1.0),
        zoom: 1.0,
        pan_pt: [0.0, 0.0],
    };

    for guide in &app.canvas_dialogs.canvas.active_snap_guides {
        let color = if guide.source == SnapSource::PlotOrigin { Rgba8::rgb(220, 38, 38) } else { pal.accent };
        match guide.orientation {
            SnapOrientation::Vertical => {
                if let (Some([px, sy]), Some([_, ey])) = (
                    local_map.scene_pt_to_pixel(PointPt::new(guide.position_pt, guide.span_start_pt)),
                    local_map.scene_pt_to_pixel(PointPt::new(guide.position_pt, guide.span_end_pt)),
                ) {
                    let x = (px.round().max(0.0) as u32).min(pw.saturating_sub(1));
                    let y0 = (sy.min(ey).round().max(0.0) as u32).min(ph);
                    let y1 = (sy.max(ey).round().max(0.0) as u32).min(ph);
                    for py in y0..y1 {
                        img.blend_pixel(x, py, color);
                    }
                }
            }
            SnapOrientation::Horizontal => {
                if let (Some([sx, py]), Some([ex, _])) = (
                    local_map.scene_pt_to_pixel(PointPt::new(guide.span_start_pt, guide.position_pt)),
                    local_map.scene_pt_to_pixel(PointPt::new(guide.span_end_pt, guide.position_pt)),
                ) {
                    let y = (py.round().max(0.0) as u32).min(ph.saturating_sub(1));
                    let x0 = (sx.min(ex).round().max(0.0) as u32).min(pw);
                    let x1 = (sx.max(ex).round().max(0.0) as u32).min(pw);
                    for px in x0..x1 {
                        img.blend_pixel(px, y, color);
                    }
                }
            }
        }
    }

    if let Some(gs) = app.document().graph_sheet(sheet_id) {
        let series_count = app
            .document()
            .data_sheet(&gs.spec.primary_source_sheet_id)
            .map_or(0, |ds| crate::inspect::table_dimensions_summary(&ds.archetype).1)
            .min(12);
        for s_idx in 0..series_count {
            if should_show_graph_series_chip(app, s_idx) {
                let bx = 8 + (s_idx as u32) * 14;
                if bx + 10 < pw && 18 < ph {
                    let col = variable_role_color(curvebook_doc::VariableRole::Continuous, &app.theme);
                    for y in 8..16 {
                        for x in bx..bx + 10 {
                            img.blend_pixel(x, y, col);
                        }
                    }
                }
            }
        }
    }
    if let Some(ls) = app.document().layout_sheet(sheet_id) {
        for (idx, item) in ls.spec.items.iter().enumerate() {
            let panel_id = format!("panel.{idx}");
            if should_show_layout_panel_chrome(app, &panel_id) {
                let bounds_pt = match item {
                    LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
                        *bounds_pt
                    }
                };
                if let (Some([x0, y0]), Some([x1, y1])) = (
                    local_map.scene_pt_to_pixel(PointPt::new(bounds_pt.x, bounds_pt.y)),
                    local_map.scene_pt_to_pixel(PointPt::new(bounds_pt.x + bounds_pt.width, bounds_pt.y + bounds_pt.height)),
                ) {
                    let rx0 = (x0.round().max(0.0) as u32).min(pw.saturating_sub(1));
                    let ry0 = (y0.round().max(0.0) as u32).min(ph.saturating_sub(1));
                    let rx1 = (x1.round().max(0.0) as u32).min(pw.saturating_sub(1));
                    let ry1 = (y1.round().max(0.0) as u32).min(ph.saturating_sub(1));
                    for x in rx0..=rx1 {
                        img.blend_pixel(x, ry0, pal.accent);
                        img.blend_pixel(x, ry0.saturating_add(1).min(ph.saturating_sub(1)), pal.accent);
                        img.blend_pixel(x, ry1, pal.accent);
                    }
                    for y in ry0..=ry1 {
                        img.blend_pixel(rx0, y, pal.accent);
                        img.blend_pixel(rx0.saturating_add(1).min(pw.saturating_sub(1)), y, pal.accent);
                        img.blend_pixel(rx1, y, pal.accent);
                    }
                    for hy in ry0..=(ry0 + 5).min(ph.saturating_sub(1)) {
                        for hx in rx0..=(rx0 + 5).min(pw.saturating_sub(1)) {
                            img.blend_pixel(hx, hy, pal.accent);
                        }
                    }
                }
            }
        }
    }
    if let Some(CanvasDragTarget::LayoutItem { preview_rect_pt: Some(preview), .. }) = &app.interaction.canvas_drag
        && let (Some([x0, y0]), Some([x1, y1])) = (
            local_map.scene_pt_to_pixel(PointPt::new(preview.x, preview.y)),
            local_map.scene_pt_to_pixel(PointPt::new(preview.x + preview.width, preview.y + preview.height)),
        )
    {
        let rx0 = (x0.round().max(0.0) as u32).min(pw.saturating_sub(1));
        let ry0 = (y0.round().max(0.0) as u32).min(ph.saturating_sub(1));
        let rx1 = (x1.round().max(0.0) as u32).min(pw.saturating_sub(1));
        let ry1 = (y1.round().max(0.0) as u32).min(ph.saturating_sub(1));
        for x in rx0..=rx1 {
            img.blend_pixel(x, ry0, pal.accent);
            img.blend_pixel(x, ry1, pal.accent);
        }
        for y in ry0..=ry1 {
            img.blend_pixel(rx0, y, pal.accent);
            img.blend_pixel(rx1, y, pal.accent);
        }
    }
    let _ = preflight_check_scene(scene);
    let _ = geom;
}

fn custom_action(payload: String) -> WidgetAction {
    WidgetAction::Custom { domain: String::from(DOMAIN_W5G1), payload }
}

fn ui_text_style(builder: &LayoutBuilder<'_>, pts: f64) -> UiTextStyle {
    UiTextStyle::ui_sans((pts * builder.scale()).clamp(8.5, 28.0))
}

fn mpx(base: u32, scale: f64) -> u32 {
    let min_px = if base == 0 { 0 } else { 1 };
    scaled_u32(f64::from(base), scale, min_px, base.saturating_mul(4).max(min_px))
}

fn mi(base: u32, scale: f64) -> i32 {
    i32::try_from(mpx(base, scale)).unwrap_or(base as i32)
}

#[allow(clippy::too_many_arguments)]
fn emit_modal_label(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    id: &str,
    rect: LayoutRectPx,
    text: &str,
    color: Rgba8,
    text_style: &UiTextStyle,
    en: ControlEnablement,
    scroll_id: Option<&str>,
) {
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, text.to_owned(), FocusScope::ModalDialog),
        state: widget_state(builder, id, en),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec { z_layer: ZLayer::ModalDialog, rect, pad_x: 4, text, style: text_style, color, scroll_container: scroll_id },
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_cbtn(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    id: &str,
    rect: LayoutRectPx,
    label: &str,
    selected: bool,
    primary: bool,
    tab_order: u32,
    en: ControlEnablement,
    action: WidgetAction,
) {
    emit_button(
        builder,
        ButtonWidgetSpec {
            id,
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect,
            label,
            keyboard_path: id,
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(tab_order),
            primary,
            selected,
            shortcut: None,
            enablement: en,
            action,
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_ctoggle(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    id: &str,
    rect: LayoutRectPx,
    label: &str,
    checked: bool,
    is_radio: bool,
    tab_order: u32,
    en: ControlEnablement,
    scroll_id: &str,
    action: WidgetAction,
) {
    emit_toggle(
        builder,
        ToggleWidgetSpec {
            id,
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect,
            label,
            keyboard_path: id,
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(tab_order),
            checked,
            is_radio,
            enablement: en,
            scroll_container: Some(scroll_id),
            action,
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn emit_stepper_row(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    id_prefix: &str,
    rect: LayoutRectPx,
    label: &str,
    value_text: &str,
    dec_payload: String,
    inc_payload: String,
    tab_order: u32,
    en: ControlEnablement,
    st: &UiTextStyle,
    scroll_id: &str,
) {
    let is_rtl = builder.tree.is_rtl;
    let scale = builder.scale();
    let controls_w = mi(136, scale).min((rect.width as i32) / 2).max(mi(96, scale));
    let lbl_gap = mpx(6, scale);
    let label_w = (rect.width as i32 - controls_w - lbl_gap as i32).max(mi(60, scale)) as u32;
    let lbl_x = if is_rtl { rect.right() - i32::try_from(label_w).unwrap_or(60) } else { rect.x };
    emit_modal_label(
        builder,
        parent,
        &format!("{id_prefix}.lbl"),
        LayoutRectPx::new(lbl_x, rect.y, label_w, rect.height),
        label,
        builder.palette.text,
        st,
        en.clone(),
        Some(scroll_id),
    );
    let cx = if is_rtl { rect.x } else { rect.x + i32::try_from(label_w + lbl_gap).unwrap_or(86) };
    let btn_w = mpx(24, scale);
    let btn_gap = mpx(4, scale);
    let val_w = (controls_w as u32).saturating_sub(btn_w * 2 + btn_gap * 2).max(mpx(40, scale));
    emit_cbtn(
        builder,
        parent,
        &format!("{id_prefix}.dec"),
        LayoutRectPx::new(cx, rect.y, btn_w, rect.height),
        "-",
        false,
        false,
        tab_order,
        en.clone(),
        custom_action(dec_payload),
    );
    let val_rect = LayoutRectPx::new(cx + i32::try_from(btn_w + btn_gap).unwrap_or(28), rect.y, val_w, rect.height);
    let val_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("{id_prefix}.val")),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: val_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, format!("{label}: {value_text}"), FocusScope::ModalDialog),
        state: widget_state(builder, &format!("{id_prefix}.val"), en.clone()),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&val_wid), val_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&val_wid), val_rect, builder.palette.border, 1);
    paint_text_fitted(
        builder,
        &val_wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: val_rect,
            pad_x: 4,
            text: value_text,
            style: st,
            color: builder.palette.text,
            scroll_container: Some(scroll_id),
        },
    );
    emit_cbtn(
        builder,
        parent,
        &format!("{id_prefix}.inc"),
        LayoutRectPx::new(val_rect.right() + i32::try_from(btn_gap).unwrap_or(4), rect.y, btn_w, rect.height),
        "+",
        false,
        false,
        tab_order + 1,
        en,
        custom_action(inc_payload),
    );
}

struct PillItem<'a> {
    id: String,
    label: &'a str,
    selected: bool,
    payload: String,
    en: ControlEnablement,
}

fn emit_pill_row(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, items: &[PillItem<'_>], base_tab: u32) {
    let is_rtl = builder.tree.is_rtl;
    let scale = builder.scale();
    if items.is_empty() || rect.width < mpx(40, scale) {
        return;
    }
    let gap = mpx(4, scale);
    let count = u32::try_from(items.len()).unwrap_or(1).max(1);
    let pill_w = (rect.width.saturating_sub(gap.saturating_mul(count.saturating_sub(1))) / count).max(mpx(40, scale));
    let mut x = if is_rtl { rect.right() } else { rect.x };
    for (idx, item) in items.iter().enumerate() {
        let (bx, w) = if is_rtl {
            let w = if idx + 1 == items.len() { (x - rect.x).max(mi(32, scale)) as u32 } else { pill_w.min((x - rect.x).max(0) as u32) };
            (x - i32::try_from(w).unwrap_or(40), w)
        } else {
            let w = if idx + 1 == items.len() { (rect.right() - x).max(mi(32, scale)) as u32 } else { pill_w.min((rect.right() - x).max(0) as u32) };
            (x, w)
        };
        if w < mpx(24, scale) || (is_rtl && bx < rect.x) || (!is_rtl && x >= rect.right()) {
            break;
        }
        emit_cbtn(
            builder,
            parent,
            &item.id,
            LayoutRectPx::new(bx, rect.y, w, rect.height),
            item.label,
            item.selected,
            false,
            base_tab + idx as u32,
            item.en.clone(),
            custom_action(item.payload.clone()),
        );
        if is_rtl {
            x -= i32::try_from(w + gap).unwrap_or(44);
        } else {
            x += i32::try_from(w + gap).unwrap_or(44);
        }
    }
}

fn begin_modal_scroll<'a>(
    builder: &mut LayoutBuilder<'a>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    scroll_id: &str,
    panel_id: &str,
    content_h: u32,
) -> (WidgetId, ScrollMeta, LayoutRectPx, i32) {
    let scale = builder.scale();
    let scroll_off = builder.scroll_offset(scroll_id);
    let total_h = mpx(content_h, scale).max(rect.height.saturating_sub(8));
    let meta = ScrollMeta::new(String::from(scroll_id), rect, [rect.width, total_h], scroll_off, ScrollAxes::Vertical);
    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(panel_id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Dialog Content", FocusScope::ModalDialog, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&panel_wid, meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.panel);
    let sb_reserve = scaled_u32(18.0, scale, 16, 36);
    let clip = LayoutRectPx::new(rect.x + 4, rect.y + 4, rect.width.saturating_sub(sb_reserve), rect.height.saturating_sub(8));
    builder.push_clip(clip);
    let y_scroll = meta.scroll_offset_px[1].round() as i32;
    (panel_wid, meta, clip, y_scroll)
}

fn end_modal_scroll(builder: &mut LayoutBuilder<'_>, panel_wid: WidgetId, meta: &ScrollMeta) {
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, meta, Some(panel_wid));
}

fn row_visible(ry: i32, rh: u32, clip: LayoutRectPx) -> bool {
    ry >= clip.y && ry + i32::try_from(rh).unwrap_or(20) <= clip.bottom()
}

/// Build the `Cmd/Ctrl+K` command palette search input and matched command rows inside `body_rect`.
pub fn build_command_palette_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let search_h = mpx(26, scale);
    let search_rect = LayoutRectPx::new(body_rect.x, body_rect.y, body_rect.width, search_h);
    emit_text_field(
        builder,
        TextFieldSpec {
            id: "palette.query",
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: search_rect,
            label: "Command Search",
            fallback_text: &builder.app.canvas_dialogs.command_palette.query,
            keyboard_path: "Cmd+K",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(10),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );

    let matches = builder.app.canvas_dialogs.command_palette.matches_for_session(builder.app.session());
    let sel_idx = if matches.is_empty() { 0 } else { builder.app.canvas_dialogs.command_palette.selected_index.min(matches.len() - 1) };
    let bar_y = body_rect.y + mi(30, scale);
    let bar_h = mpx(20, scale);
    let st = ui_text_style(builder, 10.5);
    let locale = builder.app.locale.clone();
    let prev_lbl = w5_tr(&locale, "w5.palette.prev", "Prev");
    let next_lbl = w5_tr(&locale, "w5.palette.next", "Next");
    let run_lbl = w5_tr(&locale, "w5.palette.run", "Run");
    let has_matches = !matches.is_empty();
    let empty_reason = ControlEnablement::disabled("No matching commands for the active filter.");
    let nav_en = if has_matches { ControlEnablement::enabled() } else { empty_reason };

    let is_rtl = builder.tree.is_rtl;
    let btn_w = mpx(56, scale);
    let run_w = mpx(64, scale);
    let gap = mpx(4, scale);
    if body_rect.height >= mpx(58, scale) {
        let prev_x = if is_rtl { body_rect.right() - i32::try_from(btn_w).unwrap_or(56) } else { body_rect.x };
        let next_x = if is_rtl { prev_x - i32::try_from(btn_w + gap).unwrap_or(60) } else { body_rect.x + i32::try_from(btn_w + gap).unwrap_or(60) };
        let run_x =
            if is_rtl { next_x - i32::try_from(run_w + gap).unwrap_or(68) } else { body_rect.x + i32::try_from((btn_w + gap) * 2).unwrap_or(120) };
        emit_cbtn(
            builder,
            &parent,
            "palette.prev",
            LayoutRectPx::new(prev_x, bar_y, btn_w, bar_h),
            prev_lbl,
            false,
            false,
            12,
            nav_en.clone(),
            custom_action(String::from("palette:prev")),
        );
        emit_cbtn(
            builder,
            &parent,
            "palette.next",
            LayoutRectPx::new(next_x, bar_y, btn_w, bar_h),
            next_lbl,
            false,
            false,
            13,
            nav_en.clone(),
            custom_action(String::from("palette:next")),
        );
        let run_action = matches.get(sel_idx).map_or_else(
            || custom_action(String::from("palette:run")),
            |item| WidgetAction::InvokeCommand { command_id: item.id.to_owned(), params: json!({}) },
        );
        emit_cbtn(builder, &parent, "palette.run", LayoutRectPx::new(run_x, bar_y, run_w, bar_h), run_lbl, false, true, 14, nav_en, run_action);
        let (status_x, status_w) = if is_rtl {
            (body_rect.x, (run_x - i32::try_from(gap).unwrap_or(4) - body_rect.x).max(0) as u32)
        } else {
            let sx = body_rect.x + mi(190, scale);
            (sx, (body_rect.right() - sx).max(0) as u32)
        };
        if status_w >= mpx(48, scale) {
            let status = if matches.is_empty() {
                String::from(w5_tr(&locale, "w5.palette.empty", "No matching commands"))
            } else {
                format!("{}/{} ({})", sel_idx + 1, matches.len(), matches[sel_idx].id)
            };
            emit_modal_label(
                builder,
                &parent,
                "palette.status",
                LayoutRectPx::new(status_x, bar_y, status_w, bar_h),
                &status,
                builder.palette.muted,
                &st,
                ControlEnablement::enabled(),
                None,
            );
        }
    }

    let list_y = body_rect.y + mi(54, scale);
    let list_h = (body_rect.bottom() - list_y).max(0) as u32;
    if list_h < mpx(22, scale) {
        return;
    }
    let list_rect = LayoutRectPx::new(body_rect.x, list_y, body_rect.width, list_h);
    let row_h = mpx(26, scale);
    let total_h = u32::try_from(matches.len()).unwrap_or(1).max(1) * 26 + 8;
    let scroll_id = "scroll.palette.list";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, &parent, list_rect, scroll_id, "palette.list_panel", total_h);

    if matches.is_empty() {
        let empty_rect = LayoutRectPx::new(clip.x, clip.y + 4, clip.width, mpx(22, scale));
        if row_visible(empty_rect.y, empty_rect.height, clip) {
            emit_modal_label(
                builder,
                &panel_wid,
                "palette.empty_notice",
                empty_rect,
                w5_tr(&locale, "w5.palette.empty", "No matching commands"),
                builder.palette.muted,
                &st,
                ControlEnablement::enabled(),
                Some(scroll_id),
            );
        }
        end_modal_scroll(builder, panel_wid, &meta);
        return;
    }

    let item_h = row_h.saturating_sub(mpx(3, scale));
    for (idx, item) in matches.iter().enumerate() {
        let ry = clip.y + i32::try_from(idx as u32 * row_h).unwrap_or(0) - y_scroll;
        if !row_visible(ry, item_h, clip) {
            continue;
        }
        let en = if item.enabled {
            ControlEnablement::enabled()
        } else {
            ControlEnablement::disabled(format!("Command '{}' is disabled in the current sheet context ({})", item.id, item.menu_path))
        };
        let label = format!("{} — {}", item.label, item.summary);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("palette.hit.{idx}"),
                parent: Some(panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(clip.x, ry, clip.width, item_h),
                label: &label,
                keyboard_path: &format!("palette.{idx}"),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(20 + idx as u32),
                primary: false,
                selected: idx == sel_idx,
                shortcut: (!item.shortcut.is_empty()).then_some(item.shortcut.as_str()),
                enablement: en,
                action: WidgetAction::InvokeCommand { command_id: item.id.to_owned(), params: json!({}) },
            },
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

/// Build an interactive modal body for [`ActiveDialog`], dispatching specialized `R10.3` views
/// before falling back to descriptor-driven grids (`stat_params`, `fit_params`, `power_wizard`, etc.).
pub fn build_descriptor_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    let dialog = builder.app.canvas_dialogs.active_dialog.clone();
    match dialog {
        ActiveDialog::ChangeGraphType { .. } => build_change_graph_type_body(builder, &parent, body_rect),
        ActiveDialog::ConstantHookPicker { .. } => build_hook_constant_body(builder, &parent, body_rect),
        ActiveDialog::ExportFigure { .. } => build_export_figure_body(builder, &parent, body_rect),
        ActiveDialog::DuplicateAnalysisPrompt { .. } => {
            super::analyze_picker::build_duplicate_analysis_modal_body(builder, parent, body_rect);
        }
        ActiveDialog::ChangeDataAnalyzed { .. } => build_change_data_analyzed_body(builder, &parent, body_rect),
        ActiveDialog::FormatSelectedPoints { .. } => build_format_selected_points_body(builder, &parent, body_rect),
        ActiveDialog::ExcludeWithReason { .. } => build_exclude_with_reason_body(builder, &parent, body_rect),
        ActiveDialog::ApplyStyleFrom { .. } => build_apply_style_from_body(builder, &parent, body_rect),
        ActiveDialog::PageLayoutComposer { .. } => build_page_layout_composer_body(builder, &parent, body_rect),
        ActiveDialog::InsertModelEquation { .. } => build_insert_model_equation_body(builder, &parent, body_rect),
        ActiveDialog::RandomizeGroups => build_randomize_groups_body(builder, &parent, body_rect),
        ActiveDialog::RandomSampleRows { .. } => build_random_sample_rows_body(builder, &parent, body_rect),
        ActiveDialog::SimulateDistributions => build_simulate_distributions_body(builder, &parent, body_rect),
        ActiveDialog::Print { .. } => build_print_dialog_body(builder, &parent, body_rect),
        ActiveDialog::About => build_about_dialog_body(builder, &parent, body_rect),
        _ => build_fallback_descriptor_grid(builder, &parent, body_rect),
    }
}

fn build_fallback_descriptor_grid(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let Some((_, _, _, tabs, controls, _)) = crate::inspect::dialog_widget_descriptors(builder.app) else {
        return;
    };
    let scale = builder.scale();
    let tab_h = scaled_u32(22.0, scale, 20, 44);
    let tab_step = scaled_u32(26.0, scale, 24, 52);
    let min_tab_w = scaled_u32(56.0, scale, 48, 120);
    let min_ctrl_h = scaled_u32(20.0, scale, 18, 40);
    let mut content_y = body_rect.y;
    if !tabs.is_empty() && body_rect.height >= tab_step.saturating_add(min_ctrl_h) {
        let tab_w = (body_rect.width.saturating_sub(8) / (tabs.len() as u32).max(1)).max(min_tab_w);
        for (idx, tab) in tabs.iter().enumerate() {
            let tx = body_rect.x + i32::try_from(idx as u32 * (tab_w + 4)).unwrap_or(0);
            if tx + i32::try_from(tab_w).unwrap_or(56) > body_rect.right() {
                break;
            }
            let selected = tab.value.as_ref().and_then(|v| v.get("selected")).and_then(|v| v.as_bool()).unwrap_or(false);
            let en = ControlEnablement { enabled: tab.enabled, disabled_reason: tab.disabled_reason.clone() };
            emit_tab(
                builder,
                ButtonWidgetSpec {
                    id: &tab.widget_id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: LayoutRectPx::new(tx, body_rect.y, tab_w, tab_h),
                    label: &tab.label,
                    keyboard_path: &tab.widget_id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(10 + idx as u32),
                    primary: false,
                    selected,
                    shortcut: None,
                    enablement: en,
                    action: WidgetAction::ClickDialogControl(tab.widget_id.clone()),
                },
            );
        }
        content_y += i32::try_from(tab_step).unwrap_or(26);
    }
    let ctrl_h = (body_rect.bottom() - content_y).max(0) as u32;
    if ctrl_h >= min_ctrl_h {
        emit_descriptor_controls_grid(builder, parent, LayoutRectPx::new(body_rect.x, content_y, body_rect.width, ctrl_h), &controls, 30);
    }
}

fn paint_plot_family_glyph(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, kind: ThumbnailGlyphKind) {
    let pad = 4i32;
    let gx0 = rect.x + pad;
    let gy0 = rect.y + pad;
    let gx1 = rect.right() - pad;
    let gy1 = rect.bottom() - pad;
    if gx1 <= gx0 + 6 || gy1 <= gy0 + 6 {
        return;
    }
    let ax_col = builder.palette.muted;
    let p1 = builder.palette.accent;
    let p2 = builder.palette.link_blue;
    let p3 = builder.palette.link_green;
    let p4 = builder.palette.link_red;
    let w = f64::from(gx1 - gx0);
    let h = f64::from(gy1 - gy0);
    let pt = |fx: f64, fy: f64| -> [i32; 2] { [gx0 + (fx * w).round() as i32, gy1 - (fy * h).round() as i32] };

    builder.paint_fill(ZLayer::ModalDialog, Some(owner), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(owner), rect, builder.palette.border, 1);
    builder.paint_line(ZLayer::ModalDialog, Some(owner), [gx0, gy0], [gx0, gy1], ax_col, 1);
    builder.paint_line(ZLayer::ModalDialog, Some(owner), [gx0, gy1], [gx1, gy1], ax_col, 1);

    let dot = |b: &mut LayoutBuilder<'_>, fx: f64, fy: f64, r: i32, col: Rgba8| {
        let [cx, cy] = pt(fx, fy);
        b.paint_fill(ZLayer::ModalDialog, Some(owner), LayoutRectPx::new(cx - r, cy - r, (r * 2 + 1) as u32, (r * 2 + 1) as u32), col);
    };

    match kind {
        ThumbnailGlyphKind::Scatter | ThumbnailGlyphKind::Volcano | ThumbnailGlyphKind::BlandAltman | ThumbnailGlyphKind::ResidualPlot => {
            for (fx, fy, c) in [(0.18, 0.25, p1), (0.35, 0.45, p1), (0.52, 0.62, p2), (0.72, 0.78, p4), (0.86, 0.85, p1)] {
                dot(builder, fx, fy, 2, c);
            }
            if matches!(kind, ThumbnailGlyphKind::BlandAltman | ThumbnailGlyphKind::ResidualPlot) {
                builder.paint_line(ZLayer::ModalDialog, Some(owner), pt(0.05, 0.50), pt(0.95, 0.50), ax_col, 1);
            }
        }
        ThumbnailGlyphKind::ScatterLine | ThumbnailGlyphKind::RocCurve | ThumbnailGlyphKind::NormalQqPlot => {
            let pts = [(0.12, 0.18), (0.34, 0.46), (0.58, 0.66), (0.86, 0.88)];
            for win in pts.windows(2) {
                builder.paint_line(ZLayer::ModalDialog, Some(owner), pt(win[0].0, win[0].1), pt(win[1].0, win[1].1), p1, 2);
            }
            for (fx, fy) in pts {
                dot(builder, fx, fy, 2, p2);
            }
        }
        ThumbnailGlyphKind::BarReplicates | ThumbnailGlyphKind::GroupedInterleaved | ThumbnailGlyphKind::StackedBar => {
            let bars = if matches!(kind, ThumbnailGlyphKind::GroupedInterleaved) {
                [(0.15, 0.60, p1), (0.33, 0.80, p2), (0.58, 0.48, p1), (0.76, 0.72, p2)]
            } else {
                [(0.16, 0.48, p1), (0.42, 0.78, p2), (0.68, 0.60, p3), (0.70, 0.35, p4)]
            };
            for (idx, (fx, fh, c)) in bars.iter().enumerate() {
                if !matches!(kind, ThumbnailGlyphKind::GroupedInterleaved) && idx == 3 {
                    continue;
                }
                let [bx, by] = pt(*fx, *fh);
                let bw = ((w * 0.16).round() as u32).max(4);
                let bh = (gy1 - by).max(2) as u32;
                builder.paint_fill(ZLayer::ModalDialog, Some(owner), LayoutRectPx::new(bx, by, bw, bh), *c);
            }
        }
        ThumbnailGlyphKind::BeforeAfter => {
            for (y0, y1, c) in [(0.3, 0.65, p1), (0.5, 0.82, p2), (0.2, 0.45, p3)] {
                builder.paint_line(ZLayer::ModalDialog, Some(owner), pt(0.22, y0), pt(0.78, y1), c, 1);
                dot(builder, 0.22, y0, 2, c);
                dot(builder, 0.78, y1, 2, c);
            }
        }
        ThumbnailGlyphKind::BoxWhisker | ThumbnailGlyphKind::Violin | ThumbnailGlyphKind::AlignedDot | ThumbnailGlyphKind::ForestPlot => {
            for (fx, lo, mid, hi, c) in [(0.28, 0.22, 0.48, 0.72, p1), (0.68, 0.35, 0.62, 0.85, p2)] {
                builder.paint_line(ZLayer::ModalDialog, Some(owner), pt(fx, lo - 0.1), pt(fx, hi + 0.1), c, 1);
                let [bx0, by0] = pt(fx - 0.09, hi);
                let [bx1, by1] = pt(fx + 0.09, lo);
                builder.paint_stroke(
                    ZLayer::ModalDialog,
                    Some(owner),
                    LayoutRectPx::new(bx0, by0, (bx1 - bx0).max(4) as u32, (by1 - by0).max(4) as u32),
                    c,
                    1,
                );
                builder.paint_line(ZLayer::ModalDialog, Some(owner), pt(fx - 0.09, mid), pt(fx + 0.09, mid), c, 2);
            }
        }
        ThumbnailGlyphKind::KaplanMeier => {
            let steps = [(0.08, 0.90), (0.32, 0.90), (0.32, 0.65), (0.62, 0.65), (0.62, 0.40), (0.90, 0.40)];
            for win in steps.windows(2) {
                builder.paint_line(ZLayer::ModalDialog, Some(owner), pt(win[0].0, win[0].1), pt(win[1].0, win[1].1), p1, 2);
            }
        }
        ThumbnailGlyphKind::Donut => {
            let [cx, cy] = pt(0.50, 0.50);
            let r = (w.min(h) * 0.32).round() as i32;
            builder.paint_stroke(ZLayer::ModalDialog, Some(owner), LayoutRectPx::new(cx - r, cy - r, (r * 2) as u32, (r * 2) as u32), p1, 3);
            let ir = (r / 2).max(2);
            builder.paint_stroke(ZLayer::ModalDialog, Some(owner), LayoutRectPx::new(cx - ir, cy - ir, (ir * 2) as u32, (ir * 2) as u32), p2, 2);
        }
        ThumbnailGlyphKind::BubbleScatter => {
            dot(builder, 0.26, 0.35, 3, p1);
            dot(builder, 0.54, 0.65, 5, p2);
            dot(builder, 0.78, 0.48, 2, p3);
        }
        ThumbnailGlyphKind::Heatmap => {
            let colors = [p1, p2, p3, p4];
            let cell_w = ((w * 0.40).round() as i32).max(4);
            let cell_h = ((h * 0.40).round() as i32).max(4);
            for r in 0..2 {
                for c in 0..2 {
                    builder.paint_fill(
                        ZLayer::ModalDialog,
                        Some(owner),
                        LayoutRectPx::new(gx0 + 3 + c * cell_w, gy0 + 3 + r * cell_h, (cell_w - 2).max(2) as u32, (cell_h - 2).max(2) as u32),
                        colors[(r * 2 + c) as usize % colors.len()],
                    );
                }
            }
        }
    }
}

fn build_change_graph_type_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let locale = builder.app.locale.clone();
    let show_all = builder.app.view_shards.dialogs_graph_misc.graph_type_show_all;
    let cgt = &builder.app.canvas_dialogs.change_graph_type;

    let is_rtl = builder.tree.is_rtl;
    let options: Vec<&GraphTypeOption> = if show_all { GRAPH_TYPE_OPTIONS.iter().collect() } else { cgt.recommended_options() };
    let two_pane = body_rect.width >= mpx(420, scale) && body_rect.height >= mpx(160, scale);
    let left_w = if two_pane { (body_rect.width * 58) / 100 } else { body_rect.width };
    let right_w = body_rect.width.saturating_sub(left_w + mpx(8, scale));
    let left_x = if two_pane && is_rtl { body_rect.x + i32::try_from(right_w + mpx(8, scale)).unwrap_or(0) } else { body_rect.x };
    let left_rect = LayoutRectPx::new(left_x, body_rect.y, left_w, body_rect.height);

    let cols = if left_w >= mpx(340, scale) { 2u32 } else { 1u32 };
    let card_h_logical = 54u32;
    let card_h = mpx(card_h_logical, scale);
    let card_pad = mpx(4, scale);
    let rows = u32::try_from(options.len()).unwrap_or(1).div_ceil(cols);
    let content_h = 30 + rows * card_h_logical + 12;
    let scroll_id = "scroll.graph_type.cards";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, left_rect, scroll_id, "graph_type.panel", content_h);

    let row_22 = mpx(22, scale);
    let toggle_rect = LayoutRectPx::new(clip.x, clip.y - y_scroll, clip.width, row_22);
    if row_visible(toggle_rect.y, toggle_rect.height, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "graph_type.show_all",
            toggle_rect,
            "Show all plot families",
            show_all,
            false,
            10,
            ControlEnablement::enabled(),
            scroll_id,
            custom_action(String::from("graph_type:toggle_all")),
        );
    }

    let col_gap = mpx(6, scale);
    let card_w = (clip.width.saturating_sub(cols.saturating_sub(1) * col_gap) / cols).max(mpx(110, scale));
    for (idx, opt) in options.iter().enumerate() {
        let r = u32::try_from(idx).unwrap_or(0) / cols;
        let c = u32::try_from(idx).unwrap_or(0) % cols;
        let cy = clip.y + mi(28, scale) + i32::try_from(r * card_h).unwrap_or(0) - y_scroll;
        if !row_visible(cy, card_h - card_pad, clip) {
            continue;
        }
        let cx = clip.x + i32::try_from(c * (card_w + col_gap)).unwrap_or(0);
        let card_rect = LayoutRectPx::new(cx, cy, card_w, card_h - card_pad);
        let selected = opt.id == cgt.selected_option_id;
        let card_id = format!("graph_type.card.{}", opt.id);
        let card_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(card_id.as_str()),
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: card_rect,
            a11y: labeled_meta(WidgetRole::Button, opt.label, FocusScope::ModalDialog, "Enter").with_tab_order(20 + idx as u32),
            state: widget_state(builder, &card_id, ControlEnablement::enabled()),
            action: custom_action(format!("graph_type:select:{}", opt.id)),
        });
        builder.paint_fill(
            ZLayer::ModalDialog,
            Some(&card_wid),
            card_rect,
            if selected { builder.palette.accent.with_alpha(38) } else { builder.palette.surface },
        );
        builder.paint_stroke(
            ZLayer::ModalDialog,
            Some(&card_wid),
            card_rect,
            if selected { builder.palette.accent } else { builder.palette.border },
            if selected { 2 } else { 1 },
        );
        let thumb_rect = LayoutRectPx::new(card_rect.x + 4, card_rect.y + 4, mpx(42, scale), card_rect.height.saturating_sub(8));
        paint_plot_family_glyph(builder, &card_wid, thumb_rect, opt.thumbnail);
        let text_x = thumb_rect.right() + mi(6, scale);
        let text_w = (card_rect.right() - text_x - 4).max(20) as u32;
        paint_text_fitted(
            builder,
            &card_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(text_x, card_rect.y + 4, text_w, mpx(18, scale)),
                pad_x: 2,
                text: opt.label,
                style: &st,
                color: builder.palette.text,
                scroll_container: Some(scroll_id),
            },
        );
        paint_text_fitted(
            builder,
            &card_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(text_x, card_rect.y + mi(24, scale), text_w, mpx(18, scale)),
                pad_x: 2,
                text: opt.subtitle,
                style: &ui_text_style(builder, 9.0),
                color: builder.palette.muted,
                scroll_container: Some(scroll_id),
            },
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);

    if two_pane && right_w >= mpx(140, scale) {
        let rx = if is_rtl { body_rect.x } else { body_rect.x + i32::try_from(left_w + mpx(8, scale)).unwrap_or(0) };
        let right_rect = LayoutRectPx::new(rx, body_rect.y, right_w, body_rect.height);
        let r_wid = builder.push_node(NodeSpec {
            id: WidgetId::from("graph_type.preview_pane"),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: right_rect,
            a11y: labeled_meta(WidgetRole::Dialog, "Live Graph Preview", FocusScope::ModalDialog, "Tab"),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        builder.paint_fill(ZLayer::ModalDialog, Some(&r_wid), right_rect, builder.palette.panel);

        emit_modal_label(
            builder,
            &r_wid,
            "graph_type.preview_title",
            LayoutRectPx::new(rx + 4, body_rect.y + 4, right_w.saturating_sub(8), mpx(18, scale)),
            w5_tr(&locale, "w5.graph_type.preview", "Live Preview"),
            builder.palette.text,
            &st,
            ControlEnablement::enabled(),
            None,
        );
        let pv_h = (body_rect.height / 2).clamp(mpx(72, scale), mpx(136, scale));
        let pv_rect = LayoutRectPx::new(rx + 6, body_rect.y + mi(24, scale), right_w.saturating_sub(12), pv_h);
        let selected_opt = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == cgt.selected_option_id);
        let mut blitted = false;
        if let Some(gs) = cgt
            .graph_sheet_id
            .as_ref()
            .and_then(|gid| builder.app.document().graph_sheet(gid))
            .or_else(|| builder.app.document().graph_sheets.first())
        {
            let mut preview_gs = gs.clone();
            cgt.apply_to_spec(&mut preview_gs.spec);
            if let Ok(scene) = compile_graph(&preview_gs, builder.app.document()) {
                let spec = RenderTargetSpec { width_px: pv_rect.width.max(32), height_px: pv_rect.height.max(32), dpi: 96 };
                if let Ok(img) = render_scene(&scene, spec) {
                    builder.push_clip(pv_rect);
                    let order = builder.next_order();
                    builder.tree.paint_ops.push(PaintOp {
                        z_layer: ZLayer::ModalDialog,
                        paint_order: order,
                        clip: Some(builder.active_clip_id()),
                        owner_widget_id: Some(r_wid.clone()),
                        kind: PaintOpKind::BlitImage { dst: pv_rect, image: Arc::new(img) },
                    });
                    builder.pop_clip();
                    builder.paint_stroke(ZLayer::ModalDialog, Some(&r_wid), pv_rect, builder.palette.border, 1);
                    blitted = true;
                }
            }
        }
        if !blitted {
            let glyph = selected_opt.map_or(ThumbnailGlyphKind::Scatter, |o| o.thumbnail);
            paint_plot_family_glyph(builder, &r_wid, pv_rect, glyph);
        }

        let lbl_h = mpx(16, scale);
        let lbl_step = mi(18, scale);
        let row_h = mpx(22, scale);
        let row_step = mi(26, scale);
        let mut cy = pv_rect.bottom() + mi(8, scale);
        if cy + mi(20, scale) <= right_rect.bottom() {
            emit_modal_label(
                builder,
                &r_wid,
                "graph_type.summary_lbl",
                LayoutRectPx::new(rx + 4, cy, right_w.saturating_sub(8), lbl_h),
                w5_tr(&locale, "w5.graph_type.summary", "Summary"),
                builder.palette.muted,
                &st,
                ControlEnablement::enabled(),
                None,
            );
            cy += lbl_step;
        }
        if cy + mi(22, scale) <= right_rect.bottom() {
            emit_pill_row(
                builder,
                &r_wid,
                LayoutRectPx::new(rx + 4, cy, right_w.saturating_sub(8), row_h),
                &[
                    PillItem {
                        id: String::from("graph_type.sum.mean"),
                        label: w5_tr(&locale, "inspector.mean", "Mean"),
                        selected: cgt.summary_measure == SummaryMeasure::Mean,
                        payload: String::from("graph_type:summary:mean"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("graph_type.sum.median"),
                        label: w5_tr(&locale, "inspector.median", "Median"),
                        selected: cgt.summary_measure == SummaryMeasure::Median,
                        payload: String::from("graph_type:summary:median"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("graph_type.sum.geomean"),
                        label: "Geo",
                        selected: cgt.summary_measure == SummaryMeasure::GeometricMean,
                        payload: String::from("graph_type:summary:geomean"),
                        en: ControlEnablement::enabled(),
                    },
                ],
                80,
            );
            cy += row_step;
        }
        if cy + lbl_step <= right_rect.bottom() {
            emit_modal_label(
                builder,
                &r_wid,
                "graph_type.eb_lbl",
                LayoutRectPx::new(rx + 4, cy, right_w.saturating_sub(8), lbl_h),
                w5_tr(&locale, "w5.graph_type.error_bars", "Error Bars"),
                builder.palette.muted,
                &st,
                ControlEnablement::enabled(),
                None,
            );
            cy += lbl_step;
        }
        if cy + mi(22, scale) <= right_rect.bottom() {
            emit_pill_row(
                builder,
                &r_wid,
                LayoutRectPx::new(rx + 4, cy, right_w.saturating_sub(8), row_h),
                &[
                    PillItem {
                        id: String::from("graph_type.eb.sd"),
                        label: w5_tr(&locale, "inspector.sd", "SD"),
                        selected: cgt.error_bar_metric == ErrorBarMetric::Sd,
                        payload: String::from("graph_type:errorbar:sd"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("graph_type.eb.sem"),
                        label: w5_tr(&locale, "inspector.sem", "SEM"),
                        selected: cgt.error_bar_metric == ErrorBarMetric::Sem,
                        payload: String::from("graph_type:errorbar:sem"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("graph_type.eb.ci95"),
                        label: "95% CI",
                        selected: cgt.error_bar_metric == ErrorBarMetric::ConfidenceInterval95,
                        payload: String::from("graph_type:errorbar:ci95"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("graph_type.eb.iqr"),
                        label: "IQR",
                        selected: cgt.error_bar_metric == ErrorBarMetric::InterquartileRange,
                        payload: String::from("graph_type:errorbar:iqr"),
                        en: ControlEnablement::enabled(),
                    },
                ],
                90,
            );
        }
    }
}

fn build_hook_constant_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let locale = builder.app.locale.clone();
    let hp = &builder.app.canvas_dialogs.hook_picker;
    let vs = &builder.app.view_shards.dialogs_graph_misc;
    let query_str = active_text(builder.app, "hook.search_input", &hp.search_query);

    let clear_w = mpx(68, scale);
    let search_h = mpx(24, scale);
    let search_w = body_rect.width.saturating_sub(clear_w + 4).max(mpx(120, scale));
    emit_text_field(
        builder,
        TextFieldSpec {
            id: "hook.search_input",
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: LayoutRectPx::new(body_rect.x, body_rect.y, search_w, search_h),
            label: "Search constants",
            fallback_text: &hp.search_query,
            keyboard_path: "hook.search_input",
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(10),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );
    emit_cbtn(
        builder,
        parent,
        "hook.clear",
        LayoutRectPx::new(body_rect.x + i32::try_from(search_w + 4).unwrap_or(124), body_rect.y, clear_w, search_h),
        w5_tr(&locale, "analyze_picker.clear", "Clear"),
        false,
        false,
        11,
        ControlEnablement::enabled(),
        custom_action(String::from("hook:clear_search")),
    );

    let tok_preview = hp.selected_option().map_or_else(
        || String::from("Select a constant from Info, Analysis, or Document branches"),
        |opt| format!("Selected: {} ({}) = {}", opt.label, opt.template_token, opt.preview_value),
    );
    if body_rect.height >= mpx(76, scale) {
        emit_modal_label(
            builder,
            parent,
            "hook.preview",
            LayoutRectPx::new(body_rect.x, body_rect.y + mi(28, scale), body_rect.width, mpx(18, scale)),
            &tok_preview,
            builder.palette.accent,
            &st,
            ControlEnablement::enabled(),
            None,
        );
    }

    let tree_y = body_rect.y + mi(50, scale);
    let tree_h = (body_rect.bottom() - tree_y).max(0) as u32;
    if tree_h < mpx(28, scale) {
        return;
    }

    let q_low = query_str.trim().to_ascii_lowercase();
    let mut info_items = Vec::new();
    let mut analysis_items = Vec::new();
    let mut doc_items = Vec::new();
    for (idx, opt) in hp.options.iter().enumerate() {
        let matched = q_low.is_empty()
            || opt.label.to_ascii_lowercase().contains(&q_low)
            || opt.template_token.to_ascii_lowercase().contains(&q_low)
            || opt.preview_value.to_ascii_lowercase().contains(&q_low);
        if !matched {
            continue;
        }
        match opt.source {
            curvebook_doc::HookSource::InfoConstant { .. } => info_items.push((idx, opt)),
            curvebook_doc::HookSource::ResultScalar { .. } => analysis_items.push((idx, opt)),
            curvebook_doc::HookSource::ColumnTitleValue { .. } => doc_items.push((idx, opt)),
        }
    }

    let row_h = mpx(24, scale);
    let item_h = row_h.saturating_sub(mpx(2, scale));
    let indent = mpx(16, scale);
    let mut total_rows = 3u32;
    if vs.hook_expand_info {
        total_rows += u32::try_from(info_items.len().max(1)).unwrap_or(1);
    }
    if vs.hook_expand_analysis {
        total_rows += u32::try_from(analysis_items.len().max(1)).unwrap_or(1);
    }
    if vs.hook_expand_document {
        total_rows += u32::try_from(doc_items.len().max(1)).unwrap_or(1);
    }

    let scroll_id = "scroll.hook.tree";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(
        builder,
        parent,
        LayoutRectPx::new(body_rect.x, tree_y, body_rect.width, tree_h),
        scroll_id,
        "hook.tree_panel",
        total_rows * 24 + 8,
    );

    struct HookBranch<'a> {
        key: &'static str,
        title: &'a str,
        expanded: bool,
        items: &'a [(usize, &'a crate::dialogs_canvas::AvailableHookOption)],
    }

    let branches = [
        HookBranch { key: "info", title: w5_tr(&locale, "w5.hook.cat.info", "Info Constants"), expanded: vs.hook_expand_info, items: &info_items },
        HookBranch {
            key: "analysis",
            title: w5_tr(&locale, "w5.hook.cat.analysis", "Analysis Results"),
            expanded: vs.hook_expand_analysis,
            items: &analysis_items,
        },
        HookBranch {
            key: "document",
            title: w5_tr(&locale, "w5.hook.cat.document", "Document Titles"),
            expanded: vs.hook_expand_document,
            items: &doc_items,
        },
    ];
    let mut cur_y = clip.y - y_scroll;
    let mut tab = 20u32;
    for br in &branches {
        let cat_rect = LayoutRectPx::new(clip.x, cur_y, clip.width, item_h);
        if row_visible(cat_rect.y, cat_rect.height, clip) {
            let arrow = if br.expanded { "v" } else { ">" };
            emit_cbtn(
                builder,
                &panel_wid,
                &format!("hook.cat.{}", br.key),
                cat_rect,
                &format!("{arrow} {} ({})", br.title, br.items.len()),
                br.expanded,
                false,
                tab,
                ControlEnablement::enabled(),
                custom_action(format!("hook:toggle_cat:{}", br.key)),
            );
        }
        cur_y += i32::try_from(row_h).unwrap_or(24);
        tab += 1;
        if br.expanded {
            if br.items.is_empty() {
                let empty_rect = LayoutRectPx::new(clip.x + indent as i32, cur_y, clip.width.saturating_sub(indent), item_h);
                if row_visible(empty_rect.y, empty_rect.height, clip) {
                    emit_modal_label(
                        builder,
                        &panel_wid,
                        &format!("hook.empty.{}", br.key),
                        empty_rect,
                        "No constants in this group",
                        builder.palette.muted,
                        &st,
                        ControlEnablement::enabled(),
                        Some(scroll_id),
                    );
                }
                cur_y += i32::try_from(row_h).unwrap_or(24);
            } else {
                for &(orig_idx, opt) in br.items {
                    let item_rect = LayoutRectPx::new(clip.x + indent as i32, cur_y, clip.width.saturating_sub(indent), item_h);
                    if row_visible(item_rect.y, item_rect.height, clip) {
                        let lbl = format!("{} = {}  {}", opt.label, opt.preview_value, opt.template_token);
                        emit_cbtn(
                            builder,
                            &panel_wid,
                            &format!("hook.item.{orig_idx}"),
                            item_rect,
                            &lbl,
                            hp.selected_index == Some(orig_idx),
                            false,
                            tab,
                            ControlEnablement::enabled(),
                            custom_action(format!("hook:select:{orig_idx}")),
                        );
                    }
                    cur_y += i32::try_from(row_h).unwrap_or(24);
                    tab += 1;
                }
            }
        }
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_export_figure_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let locale = builder.app.locale.clone();
    let ef = &builder.app.canvas_dialogs.export_figure;
    let vs = &builder.app.view_shards.dialogs_graph_misc;
    let scroll_id = "scroll.export.body";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "export.panel", 290);
    let row_h = mi(26, scale);
    let lbl_h = mpx(18, scale);
    let ctl_h = mpx(22, scale);
    let pil_h = mpx(24, scale);
    let gap_2 = mi(2, scale);
    let step_20 = mi(20, scale);
    let mut y = clip.y - y_scroll;

    let is_vector = matches!(ef.format, ExportFigureFormat::Svg | ExportFigureFormat::Pdf);
    let is_pdf = matches!(ef.format, ExportFigureFormat::Pdf);

    if row_visible(y, lbl_h, clip) {
        emit_modal_label(
            builder,
            &panel_wid,
            "export.fmt_lbl",
            LayoutRectPx::new(clip.x, y, clip.width, lbl_h),
            "Format",
            builder.palette.muted,
            &st,
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
    }
    y += step_20;
    if row_visible(y, pil_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, pil_h),
            &[
                PillItem {
                    id: String::from("export.fmt.svg"),
                    label: "SVG",
                    selected: matches!(ef.format, ExportFigureFormat::Svg),
                    payload: String::from("export:fmt:svg"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.fmt.pdf"),
                    label: "PDF",
                    selected: matches!(ef.format, ExportFigureFormat::Pdf),
                    payload: String::from("export:fmt:pdf"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.fmt.png"),
                    label: "PNG",
                    selected: matches!(ef.format, ExportFigureFormat::Png),
                    payload: String::from("export:fmt:png"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.fmt.tiff_deflate"),
                    label: "TIFF Deflate",
                    selected: matches!(ef.format, ExportFigureFormat::Tiff(TiffCompression::Deflate)),
                    payload: String::from("export:fmt:tiff_deflate"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.fmt.tiff_lzw"),
                    label: "TIFF LZW",
                    selected: matches!(ef.format, ExportFigureFormat::Tiff(TiffCompression::Lzw)),
                    payload: String::from("export:fmt:tiff_lzw"),
                    en: ControlEnablement::enabled(),
                },
            ],
            10,
        );
    }
    y += row_h + gap_2;

    let dpi_en = if is_vector {
        ControlEnablement::disabled("Vector SVG/PDF is resolution-independent (DPI applies to PNG/TIFF).")
    } else {
        ControlEnablement::enabled()
    };
    if row_visible(y, ctl_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "export.dpi",
            LayoutRectPx::new(clip.x, y, clip.width, ctl_h),
            "Resolution (DPI)",
            &format!("{} DPI", ef.dpi),
            String::from("export:dpi_dec"),
            String::from("export:dpi_inc"),
            20,
            dpi_en.clone(),
            &st,
            scroll_id,
        );
    }
    y += row_h;
    if row_visible(y, ctl_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, ctl_h),
            &[
                PillItem {
                    id: String::from("export.dpi.72"),
                    label: "72 DPI",
                    selected: ef.dpi == 72,
                    payload: String::from("export:dpi:72"),
                    en: dpi_en.clone(),
                },
                PillItem {
                    id: String::from("export.dpi.150"),
                    label: "150 DPI",
                    selected: ef.dpi == 150,
                    payload: String::from("export:dpi:150"),
                    en: dpi_en.clone(),
                },
                PillItem {
                    id: String::from("export.dpi.300"),
                    label: "300 DPI",
                    selected: ef.dpi == 300,
                    payload: String::from("export:dpi:300"),
                    en: dpi_en.clone(),
                },
                PillItem {
                    id: String::from("export.dpi.600"),
                    label: "600 DPI",
                    selected: ef.dpi == 600,
                    payload: String::from("export:dpi:600"),
                    en: dpi_en,
                },
            ],
            25,
        );
    }
    y += row_h + gap_2;

    if row_visible(y, ctl_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, ctl_h),
            &[
                PillItem {
                    id: String::from("export.unit.px"),
                    label: "Pixels (px)",
                    selected: vs.export_unit == "px",
                    payload: String::from("export:unit:px"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.unit.mm"),
                    label: "Millimeters (mm)",
                    selected: vs.export_unit == "mm",
                    payload: String::from("export:unit:mm"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.unit.in"),
                    label: "Inches (in)",
                    selected: vs.export_unit == "in",
                    payload: String::from("export:unit:in"),
                    en: ControlEnablement::enabled(),
                },
            ],
            32,
        );
    }
    y += row_h;

    let fmt_dim = |px: u32| -> String {
        let d = f64::from(ef.dpi.max(72));
        match vs.export_unit.as_str() {
            "mm" => format!("{:.1} mm", (f64::from(px) / d) * 25.4),
            "in" => format!("{:.2} in", f64::from(px) / d),
            _ => format!("{px} px"),
        }
    };
    let size_lbl = w5_tr(&locale, "w5.export.physical_size", "Physical Size");
    if row_visible(y, ctl_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "export.width",
            LayoutRectPx::new(clip.x, y, clip.width, ctl_h),
            &format!("{size_lbl} Width"),
            &fmt_dim(ef.width_px),
            String::from("export:size:w_dec"),
            String::from("export:size:w_inc"),
            40,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += row_h;
    if row_visible(y, ctl_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "export.height",
            LayoutRectPx::new(clip.x, y, clip.width, ctl_h),
            &format!("{size_lbl} Height"),
            &fmt_dim(ef.height_px),
            String::from("export:size:h_dec"),
            String::from("export:size:h_inc"),
            44,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += row_h + gap_2;

    let transp_en = if is_pdf {
        ControlEnablement::disabled("PDF print export uses an opaque page background; select SVG, PNG, or TIFF for transparency.")
    } else {
        ControlEnablement::enabled()
    };
    if row_visible(y, ctl_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "export.transparent_bg",
            LayoutRectPx::new(clip.x, y, clip.width, ctl_h),
            w5_tr(&locale, "w5.export.transparency", "Transparent Background"),
            vs.export_transparent_bg,
            false,
            50,
            transp_en,
            scroll_id,
            custom_action(String::from("export:transparent_bg")),
        );
    }
    y += row_h;

    if row_visible(y, lbl_h, clip) {
        emit_modal_label(
            builder,
            &panel_wid,
            "export.cs_lbl",
            LayoutRectPx::new(clip.x, y, clip.width, lbl_h),
            w5_tr(&locale, "w5.export.colorspace", "Color Space"),
            builder.palette.muted,
            &st,
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
    }
    y += step_20;
    if row_visible(y, ctl_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, ctl_h),
            &[
                PillItem {
                    id: String::from("export.cs.srgb"),
                    label: "sRGB",
                    selected: vs.export_color_space == "sRGB",
                    payload: String::from("export:cs:srgb"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.cs.p3"),
                    label: "Display-P3",
                    selected: vs.export_color_space == "Display-P3",
                    payload: String::from("export:cs:p3"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.cs.gray"),
                    label: "Grayscale",
                    selected: vs.export_color_space == "Grayscale",
                    payload: String::from("export:cs:gray"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("export.cs.cmyk"),
                    label: "CMYK-Proof",
                    selected: vs.export_color_space == "CMYK-Proof",
                    payload: String::from("export:cs:cmyk"),
                    en: ControlEnablement::enabled(),
                },
            ],
            55,
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_change_data_analyzed_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let cd = &builder.app.canvas_dialogs.change_data;
    let scroll_id = "scroll.change_data";
    let content_h = 80 + u32::try_from(cd.candidates.len()).unwrap_or(1) * 26 + 34;
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "change_data.panel", content_h);
    let mut y = clip.y - y_scroll;
    let hdr_h = mpx(20, scale);
    let row_h = mpx(22, scale);
    let lbl_h = mpx(18, scale);
    let step_24 = mi(24, scale);
    let step_20 = mi(20, scale);

    let hdr = format!(
        "Current source: {}  ->  Target: {}",
        cd.current_source_id.as_ref().map_or("None", SheetId::as_str),
        cd.selected_target_id.as_ref().map_or("Select candidate", SheetId::as_str)
    );
    if row_visible(y, hdr_h, clip) {
        emit_modal_label(
            builder,
            &panel_wid,
            "change_data.hdr",
            LayoutRectPx::new(clip.x, y, clip.width, hdr_h),
            &hdr,
            builder.palette.text,
            &st,
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
    }
    y += step_24;

    for (idx, cand) in cd.candidates.iter().enumerate() {
        if row_visible(y, row_h, clip) {
            let en = if cand.compatible {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled(
                    cand.incompatible_reason.clone().unwrap_or_else(|| String::from("Table archetype is incompatible with this analysis.")),
                )
            };
            let selected = cd.selected_target_id.as_ref() == Some(&cand.sheet_id);
            let lbl = format!("{} ({:?})", cand.title, cand.table_kind);
            emit_ctoggle(
                builder,
                &panel_wid,
                &format!("change_data.cand.{idx}"),
                LayoutRectPx::new(clip.x, y, clip.width, row_h),
                &lbl,
                selected,
                true,
                10 + idx as u32,
                en,
                scroll_id,
                custom_action(format!("change_data:cand:{idx}")),
            );
        }
        y += step_24;
    }

    if row_visible(y, lbl_h, clip) {
        emit_modal_label(
            builder,
            &panel_wid,
            "change_data.ds_hdr",
            LayoutRectPx::new(clip.x, y, clip.width, lbl_h),
            "Included data sets:",
            builder.palette.muted,
            &st,
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
    }
    y += step_20;
    let ds_count = cd
        .selected_target_id
        .as_ref()
        .or(cd.current_source_id.as_ref())
        .and_then(|sid| builder.app.document().data_sheet(sid))
        .map_or(2, |ds| crate::inspect::table_dimensions_summary(&ds.archetype).1.clamp(1, 6));
    if row_visible(y, row_h, clip) {
        let pills: Vec<PillItem<'_>> = (0..ds_count)
            .map(|i| PillItem {
                id: format!("change_data.ds.{i}"),
                label: match i {
                    0 => "Set A",
                    1 => "Set B",
                    2 => "Set C",
                    3 => "Set D",
                    4 => "Set E",
                    _ => "Set F",
                },
                selected: cd.selected_data_sets.contains(&i),
                payload: format!("change_data:ds:{i}"),
                en: ControlEnablement::enabled(),
            })
            .collect();
        emit_pill_row(builder, &panel_wid, LayoutRectPx::new(clip.x, y, clip.width, row_h), &pills, 40);
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_format_selected_points_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let fp = &builder.app.canvas_dialogs.format_points;
    let scroll_id = "scroll.fmt_pts";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "fmt_pts.panel", 260);
    let mut y = clip.y - y_scroll;
    let hdr_h = mpx(20, scale);
    let row_h = mpx(22, scale);
    let step_24 = mi(24, scale);
    let step_26 = mi(26, scale);

    let pt_en = if fp.selected_points.is_empty() {
        ControlEnablement::disabled("Select at least one data point on the graph first.")
    } else {
        ControlEnablement::enabled()
    };
    if row_visible(y, hdr_h, clip) {
        emit_modal_label(
            builder,
            &panel_wid,
            "fmt_pts.count",
            LayoutRectPx::new(clip.x, y, clip.width, hdr_h),
            &format!("{} selected data point(s) targeted", fp.selected_points.len()),
            builder.palette.text,
            &st,
            pt_en.clone(),
            Some(scroll_id),
        );
    }
    y += step_24;

    if row_visible(y, row_h, clip) {
        let cur = fp.color.as_deref().unwrap_or("");
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            &[
                PillItem {
                    id: String::from("fmt_pts.col.black"),
                    label: "Black",
                    selected: cur == "#111827",
                    payload: String::from("fmt_pts:color:black"),
                    en: pt_en.clone(),
                },
                PillItem {
                    id: String::from("fmt_pts.col.red"),
                    label: "Red",
                    selected: cur == "#DC2626",
                    payload: String::from("fmt_pts:color:red"),
                    en: pt_en.clone(),
                },
                PillItem {
                    id: String::from("fmt_pts.col.blue"),
                    label: "Blue",
                    selected: cur == "#2563EB",
                    payload: String::from("fmt_pts:color:blue"),
                    en: pt_en.clone(),
                },
                PillItem {
                    id: String::from("fmt_pts.col.green"),
                    label: "Green",
                    selected: cur == "#16A34A",
                    payload: String::from("fmt_pts:color:green"),
                    en: pt_en.clone(),
                },
                PillItem {
                    id: String::from("fmt_pts.col.orange"),
                    label: "Orange",
                    selected: cur == "#EA580C",
                    payload: String::from("fmt_pts:color:orange"),
                    en: pt_en.clone(),
                },
                PillItem {
                    id: String::from("fmt_pts.col.purple"),
                    label: "Purple",
                    selected: cur == "#7C3AED",
                    payload: String::from("fmt_pts:color:purple"),
                    en: pt_en.clone(),
                },
            ],
            10,
        );
    }
    y += step_26;

    if row_visible(y, row_h, clip) {
        let cur_sym = fp.symbol.as_deref().unwrap_or("");
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            &[
                PillItem {
                    id: String::from("fmt_pts.sym.circle"),
                    label: "Circle",
                    selected: cur_sym == "circle",
                    payload: String::from("fmt_pts:sym:circle"),
                    en: pt_en.clone(),
                },
                PillItem {
                    id: String::from("fmt_pts.sym.square"),
                    label: "Square",
                    selected: cur_sym == "square",
                    payload: String::from("fmt_pts:sym:square"),
                    en: pt_en.clone(),
                },
                PillItem {
                    id: String::from("fmt_pts.sym.triangle"),
                    label: "Triangle",
                    selected: cur_sym == "triangle_up",
                    payload: String::from("fmt_pts:sym:triangle_up"),
                    en: pt_en.clone(),
                },
                PillItem {
                    id: String::from("fmt_pts.sym.diamond"),
                    label: "Diamond",
                    selected: cur_sym == "diamond",
                    payload: String::from("fmt_pts:sym:diamond"),
                    en: pt_en.clone(),
                },
            ],
            20,
        );
    }
    y += step_26;

    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "fmt_pts.size",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Marker size (pt)",
            &format!("{:.1} pt", fp.size_pt.unwrap_or(5.0)),
            String::from("fmt_pts:size_dec"),
            String::from("fmt_pts:size_inc"),
            30,
            pt_en.clone(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "fmt_pts.bw",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Border width (pt)",
            &format!("{:.1} pt", fp.border_width_pt.unwrap_or(1.0)),
            String::from("fmt_pts:bw_dec"),
            String::from("fmt_pts:bw_inc"),
            34,
            pt_en.clone(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "fmt_pts.fill_open",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Open symbol fill (hollow)",
            fp.fill_mode.as_deref() == Some("open"),
            false,
            40,
            pt_en.clone(),
            scroll_id,
            custom_action(String::from("fmt_pts:fill_mode")),
        );
    }
    y += step_24;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "fmt_pts.hidden",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Hide selected points",
            fp.hidden.unwrap_or(false),
            false,
            41,
            pt_en.clone(),
            scroll_id,
            custom_action(String::from("fmt_pts:hidden")),
        );
    }
    y += step_24;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "fmt_pts.labels",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Show row labels on selected points",
            fp.show_row_labels,
            false,
            42,
            pt_en.clone(),
            scroll_id,
            custom_action(String::from("fmt_pts:show_row_labels")),
        );
    }
    y += step_24;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "fmt_pts.clear",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Clear all per-point style overrides",
            fp.clear_overrides,
            false,
            43,
            pt_en,
            scroll_id,
            custom_action(String::from("fmt_pts:clear_overrides")),
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_exclude_with_reason_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let ex = &builder.app.canvas_dialogs.exclude_with_reason;
    let scroll_id = "scroll.exclude";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "exclude.panel", 200);
    let mut y = clip.y - y_scroll;
    let hdr_h = mpx(20, scale);
    let row_h = mpx(22, scale);
    let field_h = mpx(24, scale);
    let step_24 = mi(24, scale);
    let step_26 = mi(26, scale);

    if row_visible(y, hdr_h, clip) {
        emit_modal_label(
            builder,
            &panel_wid,
            "exclude.target",
            LayoutRectPx::new(clip.x, y, clip.width, hdr_h),
            &format!("Targeting {} cell(s) — active tag: {}", ex.coords.len(), ex.reason_tag),
            builder.palette.text,
            &st,
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
    }
    y += step_24;

    let tags = crate::dialogs_canvas::ExcludeWithReasonDialogState::preset_reason_tags();
    for chunk_idx in [0usize, 3] {
        let end = (chunk_idx + 3).min(tags.len());
        if chunk_idx < end && row_visible(y, row_h, clip) {
            let pills: Vec<PillItem<'_>> = (chunk_idx..end)
                .map(|idx| PillItem {
                    id: format!("exclude.tag.{idx}"),
                    label: tags[idx],
                    selected: ex.reason_tag == tags[idx],
                    payload: format!("exclude:tag:{idx}"),
                    en: ControlEnablement::enabled(),
                })
                .collect();
            emit_pill_row(builder, &panel_wid, LayoutRectPx::new(clip.x, y, clip.width, row_h), &pills, 10 + chunk_idx as u32);
        }
        y += step_26;
    }

    if row_visible(y, field_h, clip) {
        let save_w = mpx(88, scale);
        let field_w = clip.width.saturating_sub(save_w + 4).max(mpx(120, scale));
        emit_text_field(
            builder,
            TextFieldSpec {
                id: "exclude.note_input",
                parent: Some(panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(clip.x, y, field_w, field_h),
                label: "Audit Note",
                fallback_text: &ex.note,
                keyboard_path: "exclude.note_input",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(25),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        emit_cbtn(
            builder,
            &panel_wid,
            "exclude.apply_note",
            LayoutRectPx::new(clip.x + i32::try_from(field_w + 4).unwrap_or(124), y, save_w, field_h),
            "Save Note",
            false,
            false,
            26,
            ControlEnablement::enabled(),
            custom_action(String::from("exclude:apply_note")),
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_apply_style_from_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let asf = &builder.app.canvas_dialogs.apply_style;
    let scroll_id = "scroll.apply_style";
    let content_h = 120 + u32::try_from(asf.donor_candidates.len().max(1)).unwrap_or(1) * 24;
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "apply_style.panel", content_h);
    let mut y = clip.y - y_scroll;
    let lbl_h = mpx(18, scale);
    let row_h = mpx(22, scale);
    let step_20 = mi(20, scale);
    let step_24 = mi(24, scale);
    let step_26 = mi(26, scale);

    if row_visible(y, lbl_h, clip) {
        emit_modal_label(
            builder,
            &panel_wid,
            "apply_style.hdr",
            LayoutRectPx::new(clip.x, y, clip.width, lbl_h),
            "Donor Graph Sheet:",
            builder.palette.muted,
            &st,
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
    }
    y += step_20;

    if asf.donor_candidates.is_empty() {
        if row_visible(y, row_h, clip) {
            emit_cbtn(
                builder,
                &panel_wid,
                "apply_style.no_donors",
                LayoutRectPx::new(clip.x, y, clip.width, row_h),
                "No other graph sheets available",
                false,
                false,
                10,
                ControlEnablement::disabled("Create at least two graph sheets to clone formatting from a donor graph."),
                WidgetAction::NoOp,
            );
        }
        y += step_26;
    } else {
        for (idx, (sid, title)) in asf.donor_candidates.iter().enumerate() {
            if row_visible(y, row_h, clip) {
                let selected = asf.donor_graph_id.as_ref() == Some(sid);
                emit_ctoggle(
                    builder,
                    &panel_wid,
                    &format!("apply_style.donor.{idx}"),
                    LayoutRectPx::new(clip.x, y, clip.width, row_h),
                    &format!("{title} ({})", sid.as_str()),
                    selected,
                    true,
                    10 + idx as u32,
                    ControlEnablement::enabled(),
                    scroll_id,
                    custom_action(format!("apply_style:donor:{idx}")),
                );
            }
            y += step_24;
        }
    }

    for (wid, toggle_key, label, checked, tab) in [
        ("apply_style.clone_palette", "palette", "Clone palette & CVD preview colors", asf.clone_palette, 40u32),
        ("apply_style.clone_frame_and_axes", "frame", "Clone frame preset, axis scale, and ticks", asf.clone_frame_and_axes, 41),
        ("apply_style.clone_error_bars_and_legend", "legend", "Clone error-bar metric and legend visibility", asf.clone_error_bars_and_legend, 42),
    ] {
        if row_visible(y, row_h, clip) {
            emit_ctoggle(
                builder,
                &panel_wid,
                wid,
                LayoutRectPx::new(clip.x, y, clip.width, row_h),
                label,
                checked,
                false,
                tab,
                ControlEnablement::enabled(),
                scroll_id,
                custom_action(format!("apply_style:toggle:{toggle_key}")),
            );
        }
        y += step_24;
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_page_layout_composer_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let pl = &builder.app.canvas_dialogs.page_layout;
    let scroll_id = "scroll.page_layout";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "page_layout.panel", 260);
    let mut y = clip.y - y_scroll;
    let lbl_h = mpx(18, scale);
    let row_h = mpx(22, scale);
    let field_h = mpx(24, scale);
    let step_20 = mi(20, scale);
    let step_26 = mi(26, scale);

    if row_visible(y, lbl_h, clip) {
        emit_modal_label(
            builder,
            &panel_wid,
            "page_layout.preset_lbl",
            LayoutRectPx::new(clip.x, y, clip.width, lbl_h),
            "Journal Page Preset:",
            builder.palette.muted,
            &st,
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
    }
    y += step_20;
    if row_visible(y, row_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            &[
                PillItem {
                    id: String::from("page_layout.p.0"),
                    label: "85mm",
                    selected: pl.preset == JournalWidthPreset::SingleColumn85Mm,
                    payload: String::from("page_layout:preset:0"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("page_layout.p.1"),
                    label: "120mm",
                    selected: pl.preset == JournalWidthPreset::OneAndHalfColumn120Mm,
                    payload: String::from("page_layout:preset:1"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("page_layout.p.2"),
                    label: "174mm",
                    selected: pl.preset == JournalWidthPreset::DoubleColumn174Mm,
                    payload: String::from("page_layout:preset:2"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("page_layout.p.3"),
                    label: "A4 Port",
                    selected: pl.preset == JournalWidthPreset::A4Portrait,
                    payload: String::from("page_layout:preset:3"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("page_layout.p.4"),
                    label: "A4 Land",
                    selected: pl.preset == JournalWidthPreset::A4Landscape,
                    payload: String::from("page_layout:preset:4"),
                    en: ControlEnablement::enabled(),
                },
            ],
            10,
        );
    }
    y += step_26;

    if row_visible(y, row_h, clip) {
        let orient_lbl = match pl.orientation {
            PageOrientation::Portrait => "Orientation: Portrait (click to switch)",
            PageOrientation::Landscape => "Orientation: Landscape (click to switch)",
        };
        emit_cbtn(
            builder,
            &panel_wid,
            "page_layout.orientation",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            orient_lbl,
            pl.orientation == PageOrientation::Landscape,
            false,
            20,
            ControlEnablement::enabled(),
            custom_action(String::from("page_layout:orientation")),
        );
    }
    y += step_26;

    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "page_layout.rows",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Grid Rows",
            &pl.rows.to_string(),
            String::from("page_layout:rows_dec"),
            String::from("page_layout:rows_inc"),
            22,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "page_layout.cols",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Grid Columns",
            &pl.cols.to_string(),
            String::from("page_layout:cols_dec"),
            String::from("page_layout:cols_inc"),
            24,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "layout.align_plot_origins",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Align plot origins across grid panels",
            pl.align_plot_origins,
            false,
            26,
            ControlEnablement::enabled(),
            scroll_id,
            custom_action(String::from("page_layout:align_origins")),
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            &[
                PillItem {
                    id: String::from("page_layout.badge.upper"),
                    label: "Badges A, B",
                    selected: pl.panel_badges == PanelBadgeStyle::UpperCaseLetters,
                    payload: String::from("page_layout:badge:upper"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("page_layout.badge.lower"),
                    label: "Badges a, b",
                    selected: pl.panel_badges == PanelBadgeStyle::LowerCaseLetters,
                    payload: String::from("page_layout:badge:lower"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("page_layout.badge.none"),
                    label: "No Badges",
                    selected: pl.panel_badges == PanelBadgeStyle::None,
                    payload: String::from("page_layout:badge:none"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("page_layout.arrange"),
                    label: &format!("Auto-Grid ({})", pl.items.len()),
                    selected: false,
                    payload: String::from("page_layout:arrange"),
                    en: ControlEnablement::enabled(),
                },
            ],
            30,
        );
    }
    y += step_26;
    if row_visible(y, field_h, clip) {
        let add_w = mpx(102, scale);
        let field_w = clip.width.saturating_sub(add_w + 4).max(mpx(110, scale));
        emit_text_field(
            builder,
            TextFieldSpec {
                id: "page_layout.callout_input",
                parent: Some(panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(clip.x, y, field_w, field_h),
                label: "Callout Markdown",
                fallback_text: &pl.new_callout_markdown,
                keyboard_path: "page_layout.callout_input",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(38),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        emit_cbtn(
            builder,
            &panel_wid,
            "page_layout.add_callout",
            LayoutRectPx::new(clip.x + i32::try_from(field_w + 4).unwrap_or(114), y, add_w, field_h),
            "Add Callout",
            false,
            false,
            39,
            ControlEnablement::enabled(),
            custom_action(String::from("page_layout:add_callout")),
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_insert_model_equation_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let eq = &builder.app.canvas_dialogs.insert_model_equation;
    let scroll_id = "scroll.insert_eq";
    let content_h = 204 + u32::try_from(eq.available_result_ids.len().max(1)).unwrap_or(1) * 24;
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "eq.panel", content_h);
    let mut y = clip.y - y_scroll;
    let row_h = mpx(22, scale);
    let step_24 = mi(24, scale);
    let step_26 = mi(26, scale);

    if eq.available_result_ids.is_empty() {
        if row_visible(y, row_h, clip) {
            emit_cbtn(
                builder,
                &panel_wid,
                "eq.no_result",
                LayoutRectPx::new(clip.x, y, clip.width, row_h),
                "No regression/fit result sheet found",
                false,
                false,
                10,
                ControlEnablement::disabled("Run linear, nonlinear, or GLM regression before inserting a model equation."),
                WidgetAction::NoOp,
            );
        }
        y += step_26;
    } else {
        for (idx, rid) in eq.available_result_ids.iter().enumerate() {
            if row_visible(y, row_h, clip) {
                emit_ctoggle(
                    builder,
                    &panel_wid,
                    &format!("eq.result.{idx}"),
                    LayoutRectPx::new(clip.x, y, clip.width, row_h),
                    &format!("Upstream fit result: {}", rid.as_str()),
                    eq.result_sheet_id.as_ref() == Some(rid),
                    true,
                    10 + idx as u32,
                    ControlEnablement::enabled(),
                    scroll_id,
                    custom_action(format!("eq:result:{idx}")),
                );
            }
            y += step_24;
        }
    }

    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "eq.ds",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Data Set Index",
            &eq.data_set_index.to_string(),
            String::from("eq:ds_dec"),
            String::from("eq:ds_inc"),
            25,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "eq.font",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Equation Font (pt)",
            &format!("{:.0} pt", eq.font_size_pt),
            String::from("eq:font_dec"),
            String::from("eq:font_inc"),
            27,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        let mode_lbl = match eq.mode {
            EquationDisplayMode::Substituted => "Display Mode: Substituted Parameter Values",
            EquationDisplayMode::Symbolic => "Display Mode: Symbolic Model Form",
        };
        emit_cbtn(
            builder,
            &panel_wid,
            "equation.toggle_mode",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            mode_lbl,
            eq.mode == EquationDisplayMode::Substituted,
            false,
            30,
            ControlEnablement::enabled(),
            custom_action(String::from("eq:toggle_mode")),
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "equation.bordered",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Frame equation callout with border",
            eq.bordered,
            false,
            31,
            ControlEnablement::enabled(),
            scroll_id,
            custom_action(String::from("eq:bordered")),
        );
    }
    y += step_24;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "equation.update_existing",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Update existing equation callout on graph",
            eq.update_existing,
            false,
            32,
            ControlEnablement::enabled(),
            scroll_id,
            custom_action(String::from("eq:update_existing")),
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_randomize_groups_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let rg = &builder.app.canvas_dialogs.randomize_groups;
    let scroll_id = "scroll.rand_groups";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "rand_groups.panel", 230);
    let mut y = clip.y - y_scroll;
    let row_h = mpx(22, scale);
    let field_h = mpx(24, scale);
    let step_28 = mi(28, scale);
    let step_26 = mi(26, scale);

    let arms_text = rg.groups.iter().map(|(n, w)| if *w == 1 { n.clone() } else { format!("{n}:{w}") }).collect::<Vec<_>>().join(", ");
    if row_visible(y, field_h, clip) {
        let apply_w = mpx(92, scale);
        let field_w = clip.width.saturating_sub(apply_w + 4).max(mpx(110, scale));
        emit_text_field(
            builder,
            TextFieldSpec {
                id: "rand_groups.arms_input",
                parent: Some(panel_wid.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(clip.x, y, field_w, field_h),
                label: "Treatment Arms",
                fallback_text: &arms_text,
                keyboard_path: "rand_groups.arms_input",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(10),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        emit_cbtn(
            builder,
            &panel_wid,
            "rand_groups.apply_text",
            LayoutRectPx::new(clip.x + i32::try_from(field_w + 4).unwrap_or(114), y, apply_w, field_h),
            "Apply Arms",
            false,
            false,
            11,
            ControlEnablement::enabled(),
            custom_action(String::from("rand_groups:apply_text")),
        );
    }
    y += step_28;
    if row_visible(y, row_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            &[
                PillItem {
                    id: String::from("rand_groups.p2"),
                    label: "2 Arms",
                    selected: rg.groups.len() == 2,
                    payload: String::from("rand_groups:arms:two"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("rand_groups.p3"),
                    label: "3 Arms",
                    selected: rg.groups.len() == 3,
                    payload: String::from("rand_groups:arms:three"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("rand_groups.pw"),
                    label: "1:2 Weighted",
                    selected: rg.groups.iter().any(|(_, w)| *w > 1),
                    payload: String::from("rand_groups:arms:weighted"),
                    en: ControlEnablement::enabled(),
                },
            ],
            14,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "rand_groups.n",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Subject Count",
            &rg.subject_count.to_string(),
            String::from("rand_groups:n_dec"),
            String::from("rand_groups:n_inc"),
            20,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            &[
                PillItem {
                    id: String::from("rand_groups.sch.balanced"),
                    label: "Complete Balanced",
                    selected: rg.scheme == RandomizationSchemeKind::CompleteBalanced,
                    payload: String::from("rand_groups:scheme:balanced"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("rand_groups.sch.block"),
                    label: "Permuted Block",
                    selected: rg.scheme == RandomizationSchemeKind::PermutedBlock,
                    payload: String::from("rand_groups:scheme:block"),
                    en: ControlEnablement::enabled(),
                },
            ],
            24,
        );
    }
    y += step_26;
    let block_en = if rg.scheme == RandomizationSchemeKind::PermutedBlock {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Block size applies only when Permuted Block randomization is selected.")
    };
    if row_visible(y, row_h, clip) {
        let bs = rg.block_sizes.first().copied().unwrap_or(4);
        emit_stepper_row(
            builder,
            &panel_wid,
            "rand_groups.block",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Block Size",
            &bs.to_string(),
            String::from("rand_groups:block_dec"),
            String::from("rand_groups:block_inc"),
            28,
            block_en,
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "rand_groups.seed",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "RNG Seed",
            &rg.seed.to_string(),
            String::from("rand_groups:seed_dec"),
            String::from("rand_groups:seed_inc"),
            32,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        let info_en = if builder.app.document().info_sheets.is_empty() {
            ControlEnablement::disabled("Add an Info sheet to link randomization constants.")
        } else {
            ControlEnablement::enabled()
        };
        emit_ctoggle(
            builder,
            &panel_wid,
            "rand_groups.target_info",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Write allocation summary to Info sheet",
            rg.target_info_sheet_id.is_some(),
            false,
            36,
            info_en,
            scroll_id,
            custom_action(String::from("rand_groups:toggle_info")),
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_random_sample_rows_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let rs = &builder.app.canvas_dialogs.random_sample_rows;
    let scroll_id = "scroll.rand_sample";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "rand_sample.panel", 200);
    let mut y = clip.y - y_scroll;
    let row_h = mpx(22, scale);
    let step_26 = mi(26, scale);

    let sheets = &builder.app.document().data_sheets;
    if row_visible(y, row_h, clip) && !sheets.is_empty() {
        let pills: Vec<PillItem<'_>> = sheets
            .iter()
            .take(4)
            .enumerate()
            .map(|(i, ds)| PillItem {
                id: format!("rand_sample.src.{i}"),
                label: &ds.title,
                selected: rs.source_sheet_id.as_ref() == Some(&ds.id),
                payload: format!("rand_sample:src:{i}"),
                en: ControlEnablement::enabled(),
            })
            .collect();
        emit_pill_row(builder, &panel_wid, LayoutRectPx::new(clip.x, y, clip.width, row_h), &pills, 10);
    }
    y += step_26;

    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "rand_sample.n",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Sample Size (rows)",
            &rs.sample_size.to_string(),
            String::from("rand_sample:n_dec"),
            String::from("rand_sample:n_inc"),
            16,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "rand_sample.seed",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "RNG Seed",
            &rs.seed.to_string(),
            String::from("rand_sample:seed_dec"),
            String::from("rand_sample:seed_inc"),
            20,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        let mut pills = vec![PillItem {
            id: String::from("rand_sample.strat.none"),
            label: "Unstratified",
            selected: rs.stratify_column.is_none(),
            payload: String::from("rand_sample:stratify:none"),
            en: ControlEnablement::enabled(),
        }];
        for (i, col) in rs.available_columns.iter().take(3).enumerate() {
            pills.push(PillItem {
                id: format!("rand_sample.strat.{i}"),
                label: col,
                selected: rs.stratify_column == Some(i),
                payload: format!("rand_sample:stratify:{i}"),
                en: ControlEnablement::enabled(),
            });
        }
        emit_pill_row(builder, &panel_wid, LayoutRectPx::new(clip.x, y, clip.width, row_h), &pills, 24);
    }
    y += step_26;
    let prop_en = if rs.stratify_column.is_some() {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Select a stratification column above to enable proportional allocation.")
    };
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "sample.proportional_allocation",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Proportional stratum allocation",
            rs.proportional_allocation,
            false,
            30,
            prop_en,
            scroll_id,
            custom_action(String::from("rand_sample:proportional")),
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_simulate_distributions_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let sd = &builder.app.canvas_dialogs.simulate_distributions;
    let vs = &builder.app.view_shards.dialogs_graph_misc;
    let scroll_id = "scroll.sim_dist";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "sim_dist.panel", 230);
    let mut y = clip.y - y_scroll;
    let row_h = mpx(22, scale);
    let step_26 = mi(26, scale);

    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "sim_dist.rows",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Row Count (n)",
            &sd.n_rows.to_string(),
            String::from("sim_dist:rows_dec"),
            String::from("sim_dist:rows_inc"),
            10,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "sim_dist.seed",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "RNG Seed",
            &sd.seed.to_string(),
            String::from("sim_dist:seed_dec"),
            String::from("sim_dist:seed_inc"),
            12,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "simulate.use_multiple_variables",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Generate as Multiple Variables table",
            sd.use_multiple_variables,
            false,
            14,
            ControlEnablement::enabled(),
            scroll_id,
            custom_action(String::from("sim_dist:multi_var")),
        );
    }
    y += step_26;

    let col_idx = vs.sim_selected_col_idx.min(sd.columns.len().saturating_sub(1));
    if row_visible(y, row_h, clip) {
        let mut pills: Vec<PillItem<'_>> = sd
            .columns
            .iter()
            .take(4)
            .enumerate()
            .map(|(i, col)| PillItem {
                id: format!("sim_dist.col.{i}"),
                label: &col.name,
                selected: i == col_idx,
                payload: format!("sim_dist:col:{i}"),
                en: ControlEnablement::enabled(),
            })
            .collect();
        pills.push(PillItem {
            id: String::from("sim_dist.add_col"),
            label: "+ Col",
            selected: false,
            payload: String::from("sim_dist:add_col"),
            en: ControlEnablement::enabled(),
        });
        emit_pill_row(builder, &panel_wid, LayoutRectPx::new(clip.x, y, clip.width, row_h), &pills, 20);
    }
    y += step_26;

    if let Some(col) = sd.columns.get(col_idx) {
        if row_visible(y, row_h, clip) {
            emit_pill_row(
                builder,
                &panel_wid,
                LayoutRectPx::new(clip.x, y, clip.width, row_h),
                &[
                    PillItem {
                        id: String::from("sim_dist.d.uni"),
                        label: "Uniform",
                        selected: col.distribution == crate::dialogs_r9::SimDistKind::Uniform,
                        payload: String::from("sim_dist:dist:uniform"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("sim_dist.d.gauss"),
                        label: "Gaussian",
                        selected: col.distribution == crate::dialogs_r9::SimDistKind::Gaussian,
                        payload: String::from("sim_dist:dist:gaussian"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("sim_dist.d.logn"),
                        label: "LogNormal",
                        selected: col.distribution == crate::dialogs_r9::SimDistKind::LogNormal,
                        payload: String::from("sim_dist:dist:lognormal"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("sim_dist.d.bin"),
                        label: "Binomial",
                        selected: col.distribution == crate::dialogs_r9::SimDistKind::Binomial,
                        payload: String::from("sim_dist:dist:binomial"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("sim_dist.d.pois"),
                        label: "Poisson",
                        selected: col.distribution == crate::dialogs_r9::SimDistKind::Poisson,
                        payload: String::from("sim_dist:dist:poisson"),
                        en: ControlEnablement::enabled(),
                    },
                    PillItem {
                        id: String::from("sim_dist.d.exp"),
                        label: "Exp",
                        selected: col.distribution == crate::dialogs_r9::SimDistKind::Exponential,
                        payload: String::from("sim_dist:dist:exponential"),
                        en: ControlEnablement::enabled(),
                    },
                ],
                30,
            );
        }
        y += step_26;
        let (p1_lbl, p1_val, p2_spec): (&str, String, Option<(&str, String)>) = match col.distribution {
            crate::dialogs_r9::SimDistKind::Uniform => ("Minimum", format!("{:.1}", col.min), Some(("Maximum", format!("{:.1}", col.max)))),
            crate::dialogs_r9::SimDistKind::Gaussian => ("Mean", format!("{:.2}", col.mean), Some(("Std Dev", format!("{:.2}", col.sd)))),
            crate::dialogs_r9::SimDistKind::LogNormal => {
                ("Mean (log)", format!("{:.2}", col.mean_log), Some(("SD (log)", format!("{:.2}", col.sd_log))))
            }
            crate::dialogs_r9::SimDistKind::Binomial => ("Trials (N)", format!("{}", col.trials), Some(("Prob (p)", format!("{:.2}", col.p)))),
            crate::dialogs_r9::SimDistKind::Poisson => ("Lambda", format!("{:.1}", col.lambda), None),
            crate::dialogs_r9::SimDistKind::Exponential => ("Rate", format!("{:.2}", col.rate), None),
        };
        if row_visible(y, row_h, clip) {
            emit_stepper_row(
                builder,
                &panel_wid,
                "sim_dist.p1",
                LayoutRectPx::new(clip.x, y, clip.width, row_h),
                p1_lbl,
                &p1_val,
                String::from("sim_dist:param:p1_dec"),
                String::from("sim_dist:param:p1_inc"),
                40,
                ControlEnablement::enabled(),
                &st,
                scroll_id,
            );
        }
        y += step_26;
        if let Some((p2_lbl, p2_val)) = p2_spec
            && row_visible(y, row_h, clip)
        {
            emit_stepper_row(
                builder,
                &panel_wid,
                "sim_dist.p2",
                LayoutRectPx::new(clip.x, y, clip.width, row_h),
                p2_lbl,
                &p2_val,
                String::from("sim_dist:param:p2_dec"),
                String::from("sim_dist:param:p2_inc"),
                42,
                ControlEnablement::enabled(),
                &st,
                scroll_id,
            );
        }
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_print_dialog_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let pr = &builder.app.canvas_dialogs.print;
    let scroll_id = "scroll.print";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "print.panel", 230);
    let mut y = clip.y - y_scroll;
    let row_h = mpx(22, scale);
    let step_26 = mi(26, scale);

    if row_visible(y, row_h, clip) {
        let mut pills = vec![PillItem {
            id: String::from("print.printer.default"),
            label: "OS Default (PDF Spool)",
            selected: pr.selected_printer.is_none(),
            payload: String::from("print:printer:default"),
            en: ControlEnablement::enabled(),
        }];
        for (i, p) in pr.discovered_printers.iter().take(2).enumerate() {
            pills.push(PillItem {
                id: format!("print.printer.{i}"),
                label: p,
                selected: pr.selected_printer.as_deref() == Some(p.as_str()),
                payload: format!("print:printer:{i}"),
                en: ControlEnablement::enabled(),
            });
        }
        emit_pill_row(builder, &panel_wid, LayoutRectPx::new(clip.x, y, clip.width, row_h), &pills, 10);
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "print.copies",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Copies",
            &pr.copies.to_string(),
            String::from("print:copies_dec"),
            String::from("print:copies_inc"),
            15,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "print.scale",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Physical Scale (%)",
            &format!("{:.0}%", pr.job.scale_percent),
            String::from("print:scale_dec"),
            String::from("print:scale_inc"),
            18,
            ControlEnablement::enabled(),
            &st,
            scroll_id,
        );
    }
    y += step_26;
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "print.crop_marks",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Include corner crop marks",
            pr.job.crop_marks,
            false,
            22,
            ControlEnablement::enabled(),
            scroll_id,
            custom_action(String::from("print:crop_marks")),
        );
    }
    y += step_26;
    let bleed_en = if pr.job.crop_marks {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Enable crop marks to configure print bleed margin.")
    };
    if row_visible(y, row_h, clip) {
        emit_stepper_row(
            builder,
            &panel_wid,
            "print.bleed",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Bleed Margin (pt)",
            &format!("{:.0} pt", pr.job.bleed_pt),
            String::from("print:bleed_dec"),
            String::from("print:bleed_inc"),
            24,
            bleed_en,
            &st,
            scroll_id,
        );
    }
    y += step_26;
    let os_en = if builder.app.is_gui_only() {
        ControlEnablement::disabled("Submitting print jobs to the OS queue is blocked under --gui-only.")
    } else {
        ControlEnablement::enabled()
    };
    if row_visible(y, row_h, clip) {
        emit_ctoggle(
            builder,
            &panel_wid,
            "print.submit_to_os",
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            "Submit directly to OS print queue",
            pr.submit_to_os,
            false,
            28,
            os_en,
            scroll_id,
            custom_action(String::from("print:submit_to_os")),
        );
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

fn build_about_dialog_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body_rect: LayoutRectPx) {
    let scale = builder.scale();
    let st = ui_text_style(builder, 10.5);
    let locale = builder.app.locale.clone();
    let sec = builder.app.view_shards.dialogs_graph_misc.active_section.as_str();
    let active_tab = match sec {
        "fonts" => "fonts",
        "shortcuts" => "shortcuts",
        _ => "overview",
    };
    let scroll_id = "scroll.about";
    let (panel_wid, meta, clip, y_scroll) = begin_modal_scroll(builder, parent, body_rect, scroll_id, "about.panel", 220);
    let mut y = clip.y - y_scroll;

    let logo_w = mpx(52, scale);
    let logo_h = mpx(42, scale);
    let logo_rect = LayoutRectPx::new(clip.x + 2, y, logo_w, logo_h);
    let txt_off = logo_w + mpx(14, scale);
    if row_visible(logo_rect.y, logo_rect.height, clip) {
        paint_plot_family_glyph(builder, &panel_wid, logo_rect, ThumbnailGlyphKind::ScatterLine);
        emit_modal_label(
            builder,
            &panel_wid,
            "about.title",
            LayoutRectPx::new(logo_rect.right() + mi(10, scale), y, clip.width.saturating_sub(txt_off), mpx(20, scale)),
            &format!("CurveBook Desktop  v{}", env!("CARGO_PKG_VERSION")),
            builder.palette.text,
            &ui_text_style(builder, 13.0),
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
        emit_modal_label(
            builder,
            &panel_wid,
            "about.tagline",
            LayoutRectPx::new(logo_rect.right() + mi(10, scale), y + mi(22, scale), clip.width.saturating_sub(txt_off), mpx(18, scale)),
            w5_tr(&locale, "w5.about.tagline", "Publication-grade scientific data analysis & graphing"),
            builder.palette.muted,
            &st,
            ControlEnablement::enabled(),
            Some(scroll_id),
        );
    }
    y += mi(48, scale);

    let row_h = mpx(22, scale);
    if row_visible(y, row_h, clip) {
        emit_pill_row(
            builder,
            &panel_wid,
            LayoutRectPx::new(clip.x, y, clip.width, row_h),
            &[
                PillItem {
                    id: String::from("about.tab.overview"),
                    label: "Overview",
                    selected: active_tab == "overview",
                    payload: String::from("about:tab:overview"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("about.tab.fonts"),
                    label: "Typography & Themes",
                    selected: active_tab == "fonts",
                    payload: String::from("about:tab:fonts"),
                    en: ControlEnablement::enabled(),
                },
                PillItem {
                    id: String::from("about.tab.shortcuts"),
                    label: "Key Bindings",
                    selected: active_tab == "shortcuts",
                    payload: String::from("about:tab:shortcuts"),
                    en: ControlEnablement::enabled(),
                },
            ],
            10,
        );
    }
    y += mi(28, scale);

    let lines: &[&str] = match active_tab {
        "fonts" => &[
            "Typography: Open-licensed proportional sans & tabular numeric coverage engine.",
            "Color Safety: Okabe-Ito & Viridis palettes with live CVD simulation filters.",
            "Themes: WCAG 2.1 AA contrast-verified Light, Dark, and High-Contrast themes.",
        ],
        "shortcuts" => {
            &["Cmd/Ctrl+K : Open Command Palette", "Cmd/Ctrl+P : Physical-Scale Print & Spool", "Cmd/Ctrl+0 / Cmd/Ctrl+= : Fit Sheet / Zoom Presets"]
        }
        _ => &[
            "Engine: Deterministic statistical & curve-fitting kernel with full audit trail.",
            "Vector Artwork: Clean-room native SVG, PDF, PNG, and TIFF journal figure export.",
            "Localization: 16-locale LTR/RTL adaptive layout & number formatting.",
        ],
    };
    let line_h = mpx(20, scale);
    let line_step = mi(22, scale);
    for (idx, text) in lines.iter().enumerate() {
        if row_visible(y, line_h, clip) {
            emit_modal_label(
                builder,
                &panel_wid,
                &format!("about.line.{idx}"),
                LayoutRectPx::new(clip.x, y, clip.width, line_h),
                text,
                builder.palette.text,
                &st,
                ControlEnablement::enabled(),
                Some(scroll_id),
            );
        }
        y += line_step;
    }
    end_modal_scroll(builder, panel_wid, &meta);
}

/// Emit a scrollable multi-column grid of descriptor controls inside `rect`.
pub fn emit_descriptor_controls_grid(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    controls: &[crate::inspect::UiWidgetNode],
    base_tab_order: u32,
) {
    let remaining: Vec<&crate::inspect::UiWidgetNode> = controls.iter().filter(|c| builder.tree.node(&c.widget_id).is_none()).collect();
    if remaining.is_empty() || rect.width < 60 || rect.height < 18 {
        return;
    }
    let scale = builder.scale();
    let cols = if rect.width >= scaled_u32(500.0, scale, 400, 1100) {
        3u32
    } else if rect.width >= scaled_u32(300.0, scale, 240, 680) {
        2u32
    } else {
        1u32
    };
    let row_h = scaled_u32(22.0, scale, 20, 48);
    let total_rows = (remaining.len() as u32).div_ceil(cols);
    let content_h = total_rows.saturating_mul(row_h) + scaled_u32(6.0, scale, 4, 14);
    let scroll_id = String::from("scroll.modal.controls");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("modal.controls_panel"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Dialog Controls", FocusScope::ModalDialog, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&panel_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.panel);

    let sb_reserve = crate::layout::scroll::scrollbar_thickness_px(scale).saturating_add(4);
    let clip = LayoutRectPx::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(sb_reserve), rect.height.saturating_sub(4));
    builder.push_clip(clip);
    let col_gap = scaled_u32(6.0, scale, 4, 14);
    let col_w = (clip.width.saturating_sub(cols.saturating_sub(1) * col_gap) / cols).max(60);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (11.0 * scale).clamp(8.5, 20.0);
    let st = UiTextStyle::ui_sans(size_px);
    let row_pad = scaled_u32(2.0, scale, 2, 6);

    let is_rtl = builder.tree.is_rtl;
    for (idx, ctrl) in remaining.iter().enumerate() {
        let r_idx = (idx as u32) / cols;
        let c_idx = (idx as u32) % cols;
        let cy = clip.y + i32::try_from(r_idx * row_h).unwrap_or(0) - y_scroll;
        if !row_visible(cy, row_h - row_pad, clip) {
            continue;
        }
        let cx = if is_rtl {
            clip.right() - i32::try_from(col_w).unwrap_or(60) - i32::try_from(c_idx * (col_w + col_gap)).unwrap_or(0)
        } else {
            clip.x + i32::try_from(c_idx * (col_w + col_gap)).unwrap_or(0)
        };
        let item_rect = LayoutRectPx::new(cx, cy, col_w, row_h - row_pad);
        let en = ControlEnablement { enabled: ctrl.enabled, disabled_reason: ctrl.disabled_reason.clone() };
        emit_single_descriptor_control(builder, &panel_wid, item_rect, ctrl, en, base_tab_order + idx as u32, &st, &scroll_id);
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(panel_wid));
}

/// Emit a single descriptor-backed dialog control widget inside `rect`.
#[allow(clippy::too_many_arguments)]
pub fn emit_single_descriptor_control(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    ctrl: &crate::inspect::UiWidgetNode,
    en: ControlEnablement,
    tab_order: u32,
    text_style: &UiTextStyle,
    scroll_id: &str,
) {
    let val_str = ctrl.value.as_ref().map_or_else(String::new, |v| match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    });
    match ctrl.role.as_str() {
        "checkbox" | "radio" => {
            let checked = ctrl.value.as_ref().and_then(|v| v.as_bool()).unwrap_or(false);
            emit_toggle(
                builder,
                crate::layout::widgets::ToggleWidgetSpec {
                    id: &ctrl.widget_id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    label: &ctrl.label,
                    keyboard_path: &ctrl.widget_id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(tab_order),
                    checked,
                    is_radio: ctrl.role == "radio",
                    enablement: en,
                    scroll_container: Some(scroll_id),
                    action: WidgetAction::ClickDialogControl(ctrl.widget_id.clone()),
                },
            );
        }
        "textbox" => {
            emit_text_field(
                builder,
                TextFieldSpec {
                    id: &ctrl.widget_id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    label: &ctrl.label,
                    fallback_text: &val_str,
                    keyboard_path: &ctrl.widget_id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(tab_order),
                    multiline: false,
                    enablement: en,
                },
            );
        }
        "label" => {
            let lbl = if val_str.is_empty() { ctrl.label.clone() } else { format!("{}: {}", ctrl.label, val_str) };
            emit_modal_label(builder, parent, &ctrl.widget_id, rect, &lbl, builder.palette.text, text_style, en, Some(scroll_id));
        }
        _ => {
            let lbl = if val_str.is_empty() { ctrl.label.clone() } else { format!("{}: {}", ctrl.label, val_str) };
            emit_cbtn(
                builder,
                parent,
                &ctrl.widget_id,
                rect,
                &lbl,
                false,
                false,
                tab_order,
                en,
                WidgetAction::ClickDialogControl(ctrl.widget_id.clone()),
            );
        }
    }
}

/// Return the canonical window title string for an [`ActiveDialog`] in English.
#[must_use]
pub fn modal_title(dialog: &ActiveDialog) -> String {
    modal_title_for_locale(dialog, "en")
}

/// Return the localized window title string for an [`ActiveDialog`].
#[must_use]
pub fn modal_title_for_locale(dialog: &ActiveDialog, locale: &str) -> String {
    let title = match dialog {
        ActiveDialog::None => return String::new(),
        ActiveDialog::AnalyzeSheet { .. } => "Analyze Data",
        ActiveDialog::EditAnalysisParameters { .. } => "Edit Analysis Parameters",
        ActiveDialog::DuplicateAnalysisPrompt { .. } => "Analysis Already Exists",
        ActiveDialog::ChangeDataAnalyzed { .. } => w5_tr(locale, "btn.change_data_analyzed", "Change Data Analyzed"),
        ActiveDialog::FormatAxes { .. } => w5_tr(locale, "inspector.format_axes", "Format Axes"),
        ActiveDialog::FormatGraph { .. } => w5_tr(locale, "inspector.format_graph", "Format Graph"),
        ActiveDialog::ChangeGraphType { .. } => "Change Graph Type",
        ActiveDialog::PairwiseComparisons { .. } => w5_tr(locale, "inspector.pairwise_comparisons", "Pairwise Comparisons"),
        ActiveDialog::BubblePlotInspector { .. } => "Bubble Plot Channels",
        ActiveDialog::ApplyStyleFrom { .. } => w5_tr(locale, "btn.apply_style_from", "Apply Style From"),
        ActiveDialog::PageLayoutComposer { .. } => "Page Layout Composer",
        ActiveDialog::ExportFigure { .. } => "Export Figure",
        ActiveDialog::Print { .. } => w5_tr(locale, "toolbar.print", "Print"),
        ActiveDialog::PowerAndSampleSize => "Power & Sample Size",
        ActiveDialog::CommandPalette => w5_tr(locale, "inspector.command_palette", "Command Palette"),
        ActiveDialog::ConvertArchetype { .. } => "Convert Table Archetype",
        ActiveDialog::ImportReshape => "Import Reshape",
        ActiveDialog::ExcludeWithReason { .. } => "Exclude with Reason",
        ActiveDialog::FormatSelectedPoints { .. } => "Format Selected Points",
        ActiveDialog::RandomizeGroups => "Randomize Groups",
        ActiveDialog::RandomSampleRows { .. } => "Random Sample Rows",
        ActiveDialog::SimulateDistributions => "Simulate Distributions",
        ActiveDialog::InsertModelEquation { .. } => "Insert Model Equation",
        ActiveDialog::ConstantHookPicker { .. } => "Hook Constant",
        ActiveDialog::ResultViewer { .. } => "Result Viewer",
        ActiveDialog::About => w5_tr(locale, "menu.help.about", "About CurveBook"),
    };
    String::from(title)
}
