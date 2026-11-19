//! Clean-room scrubbable numeric input and physical unit converter (`mm` <-> `in` <-> `pt`)
//! buffered for single-commit `Session::execute` (`Caller::Ui`) settlement (`R15.4`,
//! `gui-layout-spec.md` §9.3).

use egui::{CornerRadius, CursorIcon, Frame, Id, Key, Sense, Stroke, TextEdit, Ui, Vec2};
use serde_json::json;

use crate::grid_view::parse_localized_number_input;
use crate::i18n::LocalePreset;
use crate::layout::ZLayer;

use super::{FrameInspectEntryInput, FrameInspectMeta, Tokens};

const POINTS_PER_INCH: f64 = 72.0;
const MM_PER_INCH: f64 = 25.4;

/// Physical or dimensionless unit attached to a scrubbable numeric spinner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScrubUnitKind {
    #[default]
    Points,
    Millimeters,
    Inches,
    Percent,
    Generic,
}

impl ScrubUnitKind {
    /// Length units offered by physical size converters (`Points`, `Millimeters`, `Inches`).
    pub const LENGTH_UNITS: [Self; 3] = [Self::Points, Self::Millimeters, Self::Inches];

    /// Display suffix rendered next to the numeric readout.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Points => "pt",
            Self::Millimeters => "mm",
            Self::Inches => "in",
            Self::Percent => "%",
            Self::Generic => "",
        }
    }

    /// Return `true` when this unit converts freely among `pt`, `mm`, and `in`.
    #[must_use]
    pub const fn is_length(self) -> bool {
        matches!(self, Self::Points | Self::Millimeters | Self::Inches)
    }

    /// Convert a quantity expressed in `self` into typographic points (`pt`).
    #[must_use]
    pub fn to_points(self, value: f64) -> f64 {
        match self {
            Self::Points | Self::Percent | Self::Generic => value,
            Self::Millimeters => value * (POINTS_PER_INCH / MM_PER_INCH),
            Self::Inches => value * POINTS_PER_INCH,
        }
    }

    /// Convert a quantity expressed in typographic points (`pt`) into `self`.
    #[must_use]
    pub fn from_points(self, points: f64) -> f64 {
        match self {
            Self::Points | Self::Percent | Self::Generic => points,
            Self::Millimeters => points * (MM_PER_INCH / POINTS_PER_INCH),
            Self::Inches => points / POINTS_PER_INCH,
        }
    }

    /// Convert `value` from `self` to `target` (preserving dimensionless values when either is non-length).
    #[must_use]
    pub fn convert(self, value: f64, target: Self) -> f64 {
        if self == target || !self.is_length() || !target.is_length() {
            return value;
        }
        target.from_points(self.to_points(value))
    }

    /// Cycle to the next length unit (`Points` -> `Millimeters` -> `Inches` -> `Points`).
    #[must_use]
    pub const fn next_length_unit(self) -> Self {
        match self {
            Self::Points => Self::Millimeters,
            Self::Millimeters => Self::Inches,
            Self::Inches => Self::Points,
            other => other,
        }
    }
}

/// Per-widget buffered draft and pointer-scrub state for `render_scrub_unit_field`.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrubUnitFieldState {
    pub unit: ScrubUnitKind,
    pub editing: bool,
    pub draft_text: String,
    pub dragging: bool,
    pub drag_anchor_value: f64,
    pub drag_current_value: f64,
    pub drag_accum_dx_pt: f32,
    pub last_committed_value: Option<f64>,
}

impl Default for ScrubUnitFieldState {
    fn default() -> Self {
        Self::new(ScrubUnitKind::Points)
    }
}

impl ScrubUnitFieldState {
    /// Create a fresh buffered spinner state initialized with `unit`.
    #[must_use]
    pub const fn new(unit: ScrubUnitKind) -> Self {
        Self {
            unit,
            editing: false,
            draft_text: String::new(),
            dragging: false,
            drag_anchor_value: 0.0,
            drag_current_value: 0.0,
            drag_accum_dx_pt: 0.0,
            last_committed_value: None,
        }
    }

    /// Reset transient draft when an external model value changes outside active drag/text editing.
    pub fn sync_external(&mut self, value: f64) {
        if !self.editing && !self.dragging {
            self.drag_anchor_value = value;
            self.drag_current_value = value;
        }
    }
}

/// Visual and behavioral configuration for `render_scrub_unit_field`.
#[derive(Debug, Clone, Copy)]
pub struct ScrubUnitFieldConfig<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub command_id: Option<&'a str>,
    pub min: f64,
    pub max: f64,
    pub step_per_pt: f64,
    pub decimals: usize,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
    pub allow_unit_switch: bool,
    pub z_layer: ZLayer,
}

impl<'a> ScrubUnitFieldConfig<'a> {
    /// Construct a bounded scrubbable unit configuration with sane defaults.
    #[must_use]
    pub const fn new(widget_id: &'a str, label: &'a str, min: f64, max: f64) -> Self {
        Self {
            widget_id,
            label,
            command_id: None,
            min,
            max,
            step_per_pt: 0.5,
            decimals: 2,
            enabled: true,
            disabled_reason: None,
            allow_unit_switch: true,
            z_layer: ZLayer::ModalDialog,
        }
    }
}

/// Output produced by `render_scrub_unit_field` for one settled/layout frame.
#[derive(Debug, Clone)]
pub struct ScrubUnitFieldOutput {
    pub response: egui::Response,
    pub display_value: f64,
    pub display_unit: ScrubUnitKind,
    /// `Some(value)` strictly once on drag release, `Enter`, or focus-loss (never during active drag).
    pub committed_value: Option<f64>,
}

/// Render a clean-room horizontal drag-to-scrub & text-editable unit spinner with single-commit buffering.
///
/// The returned `committed_value` fires only once when editing finishes (drag release, `Enter`, or focus loss)
/// so callers enqueue `PendingUiIntents` once per gesture rather than spamming undo history.
pub fn render_scrub_unit_field(
    ui: &mut Ui,
    value_in_unit: f64,
    state: &mut ScrubUnitFieldState,
    cfg: ScrubUnitFieldConfig<'_>,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) -> ScrubUnitFieldOutput {
    let min_val = cfg.min.min(cfg.max);
    let max_val = cfg.max.max(cfg.min);
    let clamped_in = if value_in_unit.is_finite() { value_in_unit.clamp(min_val, max_val) } else { min_val };
    state.sync_external(clamped_in);

    let preset = LocalePreset::for_locale(tokens.locale);
    let decimals = cfg.decimals.min(6);
    let mut committed_value: Option<f64> = None;
    let active_display = if state.dragging { state.drag_current_value } else { clamped_in };

    let outer = Frame::NONE
        .fill(if cfg.enabled { tokens.input_bg } else { tokens.panel_bg })
        .stroke(Stroke::new(1.0, if cfg.enabled { tokens.input_border } else { tokens.border }))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Vec2::new(4.0, 2.0));

    let resp = outer
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;

                let label_btn = ui.add_enabled(
                    cfg.enabled,
                    egui::Label::new(egui::RichText::new(cfg.label).color(if cfg.enabled { tokens.text_primary } else { tokens.text_disabled }))
                        .sense(Sense::click_and_drag()),
                );
                if cfg.enabled && (label_btn.hovered() || label_btn.dragged()) {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                }
                if cfg.enabled && label_btn.drag_started() {
                    state.dragging = true;
                    state.editing = false;
                    state.drag_anchor_value = clamped_in;
                    state.drag_current_value = clamped_in;
                    state.drag_accum_dx_pt = 0.0;
                }
                if cfg.enabled && state.dragging && label_btn.dragged() {
                    let dx = label_btn.drag_delta().x;
                    state.drag_accum_dx_pt += dx;
                    let sensitivity = if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1.0 };
                    let next = (state.drag_anchor_value + f64::from(state.drag_accum_dx_pt) * cfg.step_per_pt * sensitivity).clamp(min_val, max_val);
                    state.drag_current_value = next;
                }
                if state.dragging && label_btn.drag_stopped() {
                    state.dragging = false;
                    let finalized = state.drag_current_value.clamp(min_val, max_val);
                    state.last_committed_value = Some(finalized);
                    committed_value = Some(finalized);
                }

                if state.editing && cfg.enabled {
                    let edit =
                        ui.add(TextEdit::singleline(&mut state.draft_text).id(Id::unique(("scrub_unit_edit", cfg.widget_id))).desired_width(68.0));
                    let enter_pressed = ui.input(|i| i.key_pressed(Key::Enter));
                    let escape_pressed = ui.input(|i| i.key_pressed(Key::Escape));
                    if escape_pressed {
                        state.editing = false;
                        state.draft_text.clear();
                    } else if enter_pressed || edit.lost_focus() {
                        if let Some(parsed) = parse_localized_number_input(&state.draft_text, &preset).filter(|v| v.is_finite()) {
                            let finalized = parsed.clamp(min_val, max_val);
                            state.drag_current_value = finalized;
                            state.last_committed_value = Some(finalized);
                            committed_value = Some(finalized);
                        }
                        state.editing = false;
                    }
                } else {
                    let formatted = format_display_number(active_display, decimals, &preset);
                    let val_btn = ui.add_enabled(
                        cfg.enabled,
                        egui::Button::new(egui::RichText::new(&formatted).color(if cfg.enabled {
                            tokens.text_primary
                        } else {
                            tokens.text_disabled
                        }))
                        .min_size(Vec2::new(64.0, 22.0)),
                    );
                    if cfg.enabled && val_btn.clicked() {
                        state.editing = true;
                        state.draft_text = formatted;
                    }
                }

                if state.unit.is_length() && cfg.allow_unit_switch {
                    let unit_btn = ui.add_enabled(cfg.enabled, egui::Button::new(state.unit.suffix()).min_size(Vec2::new(28.0, 22.0)));
                    if cfg.enabled && unit_btn.clicked() {
                        state.unit = state.unit.next_length_unit();
                    }
                } else if !state.unit.suffix().is_empty() {
                    ui.label(egui::RichText::new(state.unit.suffix()).small().color(tokens.text_muted));
                }
            })
            .response
        })
        .inner;

    let display_after = if state.dragging { state.drag_current_value } else { committed_value.unwrap_or(clamped_in) };
    let val_json = json!({ "value": display_after, "unit": state.unit.suffix() });
    let mut final_resp = resp;
    if let Some(reason) = cfg.disabled_reason {
        final_resp = final_resp.on_disabled_hover_text(reason);
    }
    final_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, cfg.enabled, cfg.label));
    meta.record_response(
        ui,
        &final_resp,
        FrameInspectEntryInput {
            widget_id: cfg.widget_id,
            role: "spinbutton",
            label: cfg.label,
            command_id: cfg.command_id,
            value: Some(val_json),
            enabled: cfg.enabled,
            disabled_reason: cfg.disabled_reason,
            z_layer: cfg.z_layer,
            ellipsized: false,
            full_label: None,
        },
    );

    ScrubUnitFieldOutput { response: final_resp, display_value: display_after, display_unit: state.unit, committed_value }
}

fn format_display_number(value: f64, decimals: usize, preset: &LocalePreset) -> String {
    let raw = format!("{value:.prec$}", prec = decimals);
    if preset.decimal_separator == '.' { raw } else { raw.replace('.', &preset.decimal_separator.to_string()) }
}
