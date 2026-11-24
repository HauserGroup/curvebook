//! Shared UI recording helpers, length spinners, draft validation, and 14-locale labels
//! for `graph_modals`.

use curvebook_palette::{ColormapId, Rgba8};
use egui::{Color32, FontId, Response, Ui};

use crate::UiApp;
use crate::grid_view::parse_localized_number_input;
use crate::i18n::LocalePreset;

use super::super::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_unit_field};
use super::super::{FrameInspectMeta, PendingUiIntents, Tokens};
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

pub(super) const fn rgba_to_color32(c: Rgba8) -> Color32 {
    crate::egui_shell::rgba8_to_color32(c)
}

pub(super) fn layout_elided(
    painter: &egui::Painter,
    text: &str,
    max_width: f32,
    color: Color32,
    font_size: f32,
) -> (std::sync::Arc<egui::Galley>, bool) {
    crate::egui_shell::ui_kit::elided_galley(painter, text, FontId::proportional(font_size), color, max_width, true)
}

pub(super) fn record_button(ui: &mut Ui, meta: &mut FrameInspectMeta, entry: ButtonEntry<'_>, tokens: &Tokens) -> Response {
    crate::egui_shell::ui_kit::modal_button(
        ui,
        meta,
        crate::egui_shell::ui_kit::ModalButtonSpec {
            widget_id: entry.widget_id,
            label: entry.label,
            command_id: entry.command_id,
            selected: entry.selected,
            primary: entry.selected && entry.widget_id == "modal.ok",
            enabled: entry.enabled,
            disabled_reason: entry.disabled_reason,
            ranked: true,
        },
        tokens,
    )
}

pub(super) fn record_tab(ui: &mut Ui, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, selected: bool, tokens: &Tokens) -> Response {
    crate::egui_shell::ui_kit::modal_tab(ui, meta, widget_id, label, selected, tokens)
}

pub(super) fn record_checkbox(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, checked: bool) {
    crate::egui_shell::ui_kit::record_modal_checkbox(ui, resp, meta, widget_id, label, checked);
}

pub(super) struct CheckboxMeta<'a> {
    pub(super) widget_id: &'a str,
    pub(super) label: &'a str,
    pub(super) checked: bool,
    pub(super) enabled: bool,
    pub(super) disabled_reason: Option<&'a str>,
}

pub(super) fn record_checkbox_enabled(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, entry: CheckboxMeta<'_>) {
    crate::egui_shell::ui_kit::record_modal_checkbox_enabled(
        ui,
        resp,
        meta,
        entry.widget_id,
        entry.label,
        entry.checked,
        entry.enabled,
        entry.disabled_reason,
    );
}

pub(super) fn record_input(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, text: &str, enabled: bool) {
    crate::egui_shell::ui_kit::record_modal_input(ui, resp, meta, widget_id, label, text, enabled);
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
    crate::egui_shell::ui_kit::record_modal_text(ui, resp, meta, widget_id, text);
}
