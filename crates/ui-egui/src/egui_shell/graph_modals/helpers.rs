//! Shared UI recording helpers, length spinners, draft validation, and 14-locale labels
//! for `graph_modals`.

use curvebook_palette::{ColormapId, Rgba8};
use egui::{Color32, FontId, Response, Stroke, Ui};
use serde_json::{Value, json};

use crate::UiApp;
use crate::grid_view::parse_localized_number_input;
use crate::i18n::LocalePreset;
use crate::inspect::ZLayer;

use super::super::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_unit_field};
use super::super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use super::state::GraphModalEguiState;

pub(super) struct GraphModalCtx<'a> {
    pub(super) app: &'a UiApp,
    pub(super) tokens: &'a Tokens,
    pub(super) locale: &'a str,
    pub(super) meta: &'a mut FrameInspectMeta,
    pub(super) intents: &'a mut PendingUiIntents,
    pub(super) body_w: f32,
}

pub(super) struct ButtonEntry<'a> {
    pub(super) widget_id: &'a str,
    pub(super) label: &'a str,
    pub(super) command_id: Option<&'a str>,
    pub(super) selected: bool,
    pub(super) enabled: bool,
    pub(super) disabled_reason: Option<&'a str>,
}

pub(super) fn format_localized_draft_f64(value: f64, preset: &LocalePreset) -> String {
    let normalized = if value == 0.0 { 0.0 } else { value };
    let text = format!("{normalized}");
    if preset.decimal_separator != '.' { text.replace('.', &preset.decimal_separator.to_string()) } else { text }
}

pub(super) fn validate_heatmap_range(state: &GraphModalEguiState, preset: &LocalePreset) -> Result<Option<(f64, f64)>, String> {
    if state.heatmap_auto_range {
        return Ok(None);
    }
    let min_raw = state.heatmap_min_draft.trim();
    let max_raw = state.heatmap_max_draft.trim();
    if min_raw.is_empty() && max_raw.is_empty() {
        return Ok(None);
    }
    if min_raw.is_empty() || max_raw.is_empty() {
        return Err(String::from("Enter both Min and Max bounds, or enable Automatic Domain Bounds"));
    }
    let Some(lo) = parse_localized_number_input(min_raw, preset) else {
        return Err(format!("Invalid heatmap Min value `{min_raw}`"));
    };
    let Some(hi) = parse_localized_number_input(max_raw, preset) else {
        return Err(format!("Invalid heatmap Max value `{max_raw}`"));
    };
    if lo >= hi {
        return Err(format!("Heatmap Min ({lo}) must be strictly less than Max ({hi})"));
    }
    Ok(Some((lo, hi)))
}

pub(super) fn reset_format_axes_session_state(state: &mut GraphModalEguiState) {
    state.format_axes_session_synced = false;
    state.format_axes_last_target = None;
    state.format_axes_pending_tab = None;
    state.format_axes_per_axis_cache = [None, None, None];
    state.format_axes_frame_preset_dirty = false;
    state.additional_ticks.open = false;
    state.additional_tick_status = None;
    state.axis_bounds_error = None;
    state.axis_ticks_error = None;
}

pub(super) fn reset_format_graph_local_drafts(state: &mut GraphModalEguiState) {
    state.format_graph_session_synced = false;
    state.format_graph_last_sheet_id = None;
    state.mv_clamp_min_draft.clear();
    state.mv_clamp_max_draft.clear();
    state.heatmap_colormap = ColormapId::Viridis;
    state.heatmap_reversed = false;
    state.heatmap_auto_range = true;
    state.heatmap_min_draft = String::from("-1.0");
    state.heatmap_max_draft = String::from("1.0");
    state.heatmap_range_error = None;
    state.heatmap_annotate_cells = true;
    state.heatmap_cell_gap_pt = 1.5;
    state.heatmap_tick_angle_deg = 0.0;
    state.heatmap_color_dirty = false;
    state.heatmap_tick_angle_dirty = false;
    state.heatmap_synced_color_snapshot = None;
    state.heatmap_synced_tick_angle_deg = 0.0;
}

pub(super) fn rgba_to_color32(c: Rgba8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

pub(super) fn layout_elided(
    painter: &egui::Painter,
    text: &str,
    max_width: f32,
    color: Color32,
    font_size: f32,
) -> (std::sync::Arc<egui::Galley>, bool) {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(FontId::proportional(font_size), color));
    job.wrap = egui::text::TextWrapping { max_width: max_width.max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let elided = galley.elided;
    (galley, elided)
}

pub(super) fn record_if_visible(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, input: FrameInspectEntryInput<'_>) {
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() > 0.0 && clipped.height() > 0.0 {
        meta.record_response(ui, resp, input);
    }
}

pub(super) fn record_button(ui: &mut Ui, meta: &mut FrameInspectMeta, entry: ButtonEntry<'_>, tokens: &Tokens) -> Response {
    let is_primary_footer = entry.selected && entry.widget_id == "modal.ok";
    let btn = if is_primary_footer && entry.enabled {
        egui::Button::new(egui::RichText::new(entry.label).color(tokens.on_accent))
            .fill(tokens.accent_strong)
            .stroke(Stroke::new(1.0, tokens.accent_strong))
    } else if entry.selected {
        egui::Button::new(entry.label).fill(tokens.active_bg).stroke(Stroke::new(1.0, tokens.accent_strong))
    } else {
        egui::Button::new(entry.label)
    };
    let mut resp = ui.add_enabled(entry.enabled, btn);
    if let Some(reason) = entry.disabled_reason {
        resp = resp.on_disabled_hover_text(reason);
    }
    record_if_visible(
        ui,
        &resp,
        meta,
        FrameInspectEntryInput {
            widget_id: entry.widget_id,
            role: "button",
            label: entry.label,
            command_id: entry.command_id,
            value: Some(json!({ "selected": entry.selected })),
            enabled: entry.enabled,
            disabled_reason: entry.disabled_reason,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
    resp
}

pub(super) fn record_tab(ui: &mut Ui, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, selected: bool, tokens: &Tokens) -> Response {
    let btn = egui::Button::new(label)
        .fill(if selected { tokens.active_bg } else { tokens.surface_bg })
        .stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { tokens.accent_strong } else { tokens.border }));
    let resp = ui.add(btn);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    record_if_visible(
        ui,
        &resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "tab",
            label,
            command_id: None,
            value: Some(json!({ "selected": selected })),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
    resp
}

pub(super) fn record_checkbox(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, checked: bool) {
    record_if_visible(
        ui,
        resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "checkbox",
            label,
            command_id: None,
            value: Some(Value::Bool(checked)),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

pub(super) fn record_input(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, text: &str, enabled: bool) {
    let ak_lbl = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::TextInput, enabled, ak_lbl.clone()));
    record_if_visible(
        ui,
        resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "textbox",
            label,
            command_id: None,
            value: Some(Value::String(text.to_owned())),
            enabled,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

pub(in crate::egui_shell) struct LengthSpinnerSpec<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub value_pt: f64,
    pub min_pt: f64,
    pub max_pt: f64,
    pub step_pt: f64,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
}

pub(in crate::egui_shell) fn render_length_spinner_pt(
    ui: &mut Ui,
    spec: LengthSpinnerSpec<'_>,
    state: &mut ScrubUnitFieldState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) -> Option<f64> {
    let unit = if state.unit.is_length() { state.unit } else { ScrubUnitKind::Points };
    let disp_in = unit.from_points(spec.value_pt);
    let disp_min = unit.from_points(spec.min_pt);
    let disp_max = unit.from_points(spec.max_pt);
    let step_disp = unit.from_points(spec.step_pt).max(0.001);
    let dec = match unit {
        ScrubUnitKind::Millimeters => 2,
        ScrubUnitKind::Inches => 3,
        _ => 1,
    };
    let cfg = ScrubUnitFieldConfig {
        step_per_pt: step_disp,
        decimals: dec,
        enabled: spec.enabled,
        disabled_reason: spec.disabled_reason,
        allow_unit_switch: true,
        ..ScrubUnitFieldConfig::new(spec.widget_id, spec.label, disp_min, disp_max)
    };
    let out = render_scrub_unit_field(ui, disp_in, state, cfg, tokens, meta);
    out.committed_value.map(|v| out.display_unit.to_points(v).clamp(spec.min_pt, spec.max_pt))
}

pub(super) fn record_label(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, text: &str) {
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput {
            widget_id,
            role: "text",
            label: text,
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}
