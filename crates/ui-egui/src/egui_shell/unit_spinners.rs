//! Clean-room scrubbable numeric input and physical unit converter (`mm` <-> `in` <-> `pt`)
//! buffered for single-commit `Session::execute` (`Caller::Ui`) settlement (`R15.4`,
//! `gui-layout-spec.md` §9.3).

use egui::{Align2, CornerRadius, CursorIcon, FontId, Frame, Id, Key, Pos2, Rect, Sense, Stroke, TextEdit, Ui, Vec2};
use serde_json::json;

use crate::grid_view::parse_localized_number_input;
use crate::i18n::LocalePreset;
use crate::inspect::ZLayer;

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

    /// Parse a trailing unit suffix token (`pt`, `mm`, `in`, `%`) case-insensitively.
    #[must_use]
    pub fn parse_suffix(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "pt" | "pts" | "point" | "points" => Some(Self::Points),
            "mm" | "millimeter" | "millimeters" => Some(Self::Millimeters),
            "in" | "inch" | "inches" | "\"" => Some(Self::Inches),
            "%" | "pct" | "percent" => Some(Self::Percent),
            _ => None,
        }
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
    /// `Some(value)` strictly once on drag release, `Enter`, focus-loss, or discrete arrow step
    /// (never during active pointer drag).
    pub committed_value: Option<f64>,
}

/// Render a clean-room horizontal drag-to-scrub & text-editable unit spinner with single-commit buffering.
///
/// The returned `committed_value` fires only once when editing finishes (drag release, `Enter`, or focus loss)
/// so callers enqueue `PendingUiIntents` once per gesture rather than spamming undo history.
struct ScrubChipBounds {
    min_val: f64,
    max_val: f64,
    active_display: f64,
    clamped_in: f64,
    decimals: usize,
    chip_w: f32,
    unit_w: f32,
    focus_edit_flag: Id,
}

fn render_scrub_text_editor(
    ui: &mut Ui,
    bounds: &ScrubChipBounds,
    cfg: &ScrubUnitFieldConfig<'_>,
    preset: &LocalePreset,
    state: &mut ScrubUnitFieldState,
    committed_value: &mut Option<f64>,
) {
    let edit_id = Id::unique(("scrub_unit_edit", cfg.widget_id));
    let edit = ui.add(TextEdit::singleline(&mut state.draft_text).id(edit_id).desired_width(bounds.chip_w));
    if ui.ctx().data(|d| d.get_temp::<bool>(bounds.focus_edit_flag)).unwrap_or(false) {
        edit.request_focus();
        ui.ctx().data_mut(|d| d.remove::<bool>(bounds.focus_edit_flag));
    }
    let enter_pressed = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
    let escape_pressed = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    if escape_pressed {
        state.editing = false;
        state.draft_text.clear();
    } else if enter_pressed || edit.lost_focus() {
        if let Some((parsed_val, typed_unit)) = parse_scrub_text_input(&state.draft_text, state.unit, preset) {
            let converted = match typed_unit {
                Some(u) if u.is_length() && state.unit.is_length() => u.convert(parsed_val, state.unit),
                _ => parsed_val,
            };
            if converted.is_finite() {
                let finalized = converted.clamp(bounds.min_val, bounds.max_val);
                state.drag_current_value = finalized;
                state.last_committed_value = Some(finalized);
                *committed_value = Some(finalized);
            }
        }
        state.editing = false;
    }
}

fn render_scrub_value_chip(
    ui: &mut Ui,
    bounds: &ScrubChipBounds,
    cfg: ScrubUnitFieldConfig<'_>,
    preset: &LocalePreset,
    tokens: &Tokens,
    state: &mut ScrubUnitFieldState,
    committed_value: &mut Option<f64>,
) {
    let formatted = format_display_number(bounds.active_display, bounds.decimals, preset);
    let (chip_rect, val_btn) = ui.allocate_exact_size(Vec2::new(bounds.chip_w, 22.0), Sense::click_and_drag());
    let chip_bg = if !cfg.enabled {
        tokens.panel_bg
    } else if state.dragging {
        tokens.active_bg
    } else if val_btn.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let r_sm = CornerRadius::same(tokens.metrics.radius_sm);
    ui.painter().rect_filled(chip_rect, r_sm, chip_bg);
    ui.painter().rect_stroke(chip_rect, r_sm, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);

    if cfg.enabled && state.dragging && bounds.max_val > bounds.min_val {
        let frac = ((bounds.active_display - bounds.min_val) / (bounds.max_val - bounds.min_val)).clamp(0.0, 1.0) as f32;
        let fill_w = (chip_rect.width() * frac).max(2.0);
        let bar = Rect::from_min_max(
            Pos2::new(chip_rect.left() + 1.0, chip_rect.bottom() - 3.0),
            Pos2::new((chip_rect.left() + fill_w).min(chip_rect.right() - 1.0), chip_rect.bottom() - 1.0),
        );
        ui.painter().rect_filled(bar, CornerRadius::same(1), tokens.accent);
    }

    ui.painter().with_clip_rect(chip_rect.shrink(2.0)).text(
        chip_rect.center(),
        Align2::CENTER_CENTER,
        &formatted,
        FontId::proportional(11.5),
        if cfg.enabled { tokens.text_primary } else { tokens.text_disabled },
    );

    if cfg.enabled {
        if val_btn.hovered() && !state.dragging {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        apply_drag_interaction(&val_btn, ui, cfg, bounds, state, committed_value);

        if val_btn.clicked() && !state.dragging {
            state.editing = true;
            state.draft_text = formatted;
            ui.ctx().data_mut(|d| d.insert_temp(bounds.focus_edit_flag, true));
        } else if val_btn.has_focus() {
            handle_chip_keyboard(ui, bounds, &cfg, preset, state, committed_value);
        }
    } else if let Some(reason) = cfg.disabled_reason {
        val_btn.on_hover_text(reason);
    }
}

fn handle_chip_keyboard(
    ui: &Ui,
    bounds: &ScrubChipBounds,
    cfg: &ScrubUnitFieldConfig<'_>,
    preset: &LocalePreset,
    state: &mut ScrubUnitFieldState,
    committed_value: &mut Option<f64>,
) {
    let step_dir = if ui.input(|i| i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::ArrowRight)) {
        1.0
    } else if ui.input(|i| i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::ArrowLeft)) {
        -1.0
    } else {
        0.0
    };
    if step_dir != 0.0 {
        let factor = if ui.input(|i| i.modifiers.shift) { 0.2 } else { 2.0 };
        let next = (bounds.clamped_in + step_dir * cfg.step_per_pt.max(1e-6) * factor).clamp(bounds.min_val, bounds.max_val);
        state.drag_current_value = next;
        state.last_committed_value = Some(next);
        *committed_value = Some(next);
    } else if ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Space)) {
        state.editing = true;
        state.draft_text = format_display_number(bounds.clamped_in, bounds.decimals, preset);
        ui.ctx().data_mut(|d| d.insert_temp(bounds.focus_edit_flag, true));
    }
}

fn render_scrub_unit_badge(
    ui: &mut Ui,
    bounds: &ScrubChipBounds,
    cfg: &ScrubUnitFieldConfig<'_>,
    preset: &LocalePreset,
    tokens: &Tokens,
    state: &mut ScrubUnitFieldState,
) -> bool {
    let mut converted = false;
    if state.unit.is_length() && cfg.allow_unit_switch {
        let unit_lbl = state.unit.suffix();
        let (u_rect, unit_btn) = ui.allocate_exact_size(Vec2::new(bounds.unit_w, 22.0), Sense::click());
        let u_bg = if !cfg.enabled {
            tokens.panel_bg
        } else if unit_btn.hovered() {
            tokens.hover_bg
        } else {
            tokens.button_bg
        };
        let r_sm = CornerRadius::same(tokens.metrics.radius_sm);
        ui.painter().rect_filled(u_rect, r_sm, u_bg);
        ui.painter().rect_stroke(u_rect, r_sm, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);
        ui.painter().text(
            u_rect.center(),
            Align2::CENTER_CENTER,
            unit_lbl,
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            if cfg.enabled { tokens.accent } else { tokens.text_disabled },
        );
        if cfg.enabled && unit_btn.clicked() {
            let prev = state.unit;
            let next = prev.next_length_unit();
            state.drag_anchor_value = prev.convert(state.drag_anchor_value, next);
            state.drag_current_value = prev.convert(state.drag_current_value, next);
            state.unit = next;
            converted = true;
            if state.editing {
                state.draft_text = format_display_number(prev.convert(bounds.clamped_in, next), bounds.decimals, preset);
            }
        } else if !cfg.enabled
            && let Some(reason) = cfg.disabled_reason
        {
            unit_btn.on_hover_text(reason);
        }
    } else if !state.unit.suffix().is_empty() {
        let color = if cfg.enabled { tokens.text_muted } else { tokens.text_disabled };
        ui.add_sized(Vec2::new(bounds.unit_w, 20.0), egui::Label::new(egui::RichText::new(state.unit.suffix()).small().color(color)));
    }
    converted
}

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
    let mut unit_switched_conversion = false;
    let active_display = if state.dragging { state.drag_current_value } else { clamped_in };

    let focus_edit_flag = Id::unique(("scrub_unit_request_focus", cfg.widget_id));
    let avail_w = ui.available_width().max(110.0);
    let has_unit_widget = (state.unit.is_length() && cfg.allow_unit_switch) || !state.unit.suffix().is_empty();
    let unit_w = if state.unit.is_length() && cfg.allow_unit_switch {
        30.0
    } else if !state.unit.suffix().is_empty() {
        22.0
    } else {
        0.0
    };
    let label_cap = (avail_w - unit_w - 68.0).clamp(40.0, 150.0);
    let chip_w = if cfg.label.trim().is_empty() {
        (avail_w - unit_w - 16.0).clamp(48.0, 96.0)
    } else {
        (avail_w - label_cap - unit_w - 22.0).clamp(46.0, 84.0)
    };
    let bounds = ScrubChipBounds { min_val, max_val, active_display, clamped_in, decimals, chip_w, unit_w, focus_edit_flag };

    let border_col = if !cfg.enabled {
        tokens.border
    } else if state.dragging || state.editing {
        tokens.focus_ring
    } else {
        tokens.input_border
    };

    let outer = Frame::NONE
        .fill(if cfg.enabled { tokens.input_bg } else { tokens.panel_bg })
        .stroke(Stroke::new(if state.dragging || state.editing { 1.5 } else { 1.0 }, border_col))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Vec2::new(5.0, 2.0));

    let resp = outer
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;

                if !cfg.label.trim().is_empty() {
                    let mut label_galley = egui::text::LayoutJob::single_section(
                        cfg.label.to_owned(),
                        egui::TextFormat::simple(FontId::proportional(11.5), if cfg.enabled { tokens.text_primary } else { tokens.text_disabled }),
                    );
                    label_galley.wrap =
                        egui::text::TextWrapping { max_width: label_cap, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
                    let galley = ui.fonts_mut(|f| f.layout_job(label_galley));
                    let lbl_w = galley.size().x.min(label_cap);
                    let (lbl_rect, label_btn) = ui.allocate_exact_size(Vec2::new(lbl_w, 20.0), Sense::click_and_drag());
                    ui.painter().with_clip_rect(lbl_rect).galley(
                        Pos2::new(lbl_rect.left(), lbl_rect.center().y - galley.size().y * 0.5),
                        galley,
                        if cfg.enabled { tokens.text_primary } else { tokens.text_disabled },
                    );
                    apply_drag_interaction(&label_btn, ui, cfg, &bounds, state, &mut committed_value);
                }

                if state.editing && cfg.enabled {
                    render_scrub_text_editor(ui, &bounds, &cfg, &preset, state, &mut committed_value);
                } else {
                    render_scrub_value_chip(ui, &bounds, cfg, &preset, tokens, state, &mut committed_value);
                }

                if has_unit_widget {
                    unit_switched_conversion = render_scrub_unit_badge(ui, &bounds, &cfg, &preset, tokens, state);
                }
            })
            .response
        })
        .inner;

    let display_after = if state.dragging || unit_switched_conversion { state.drag_current_value } else { committed_value.unwrap_or(clamped_in) };
    let val_json = json!({
        "value": display_after,
        "unit": state.unit.suffix(),
        "dragging": state.dragging,
        "editing": state.editing,
    });
    let mut final_resp = resp;
    if !cfg.enabled {
        if final_resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
        }
        if let Some(reason) = cfg.disabled_reason {
            final_resp = final_resp.on_hover_text(reason).on_disabled_hover_text(reason);
        }
    }
    let a11y_label = cfg.label.to_owned();
    final_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::SpinButton, cfg.enabled, a11y_label.clone()));
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

/// Convenience wrapper over [`render_scrub_unit_field`] when the underlying model value and
/// `[min_pt, max_pt]` bounds are stored canonically in typographic points (`pt`).
///
/// Displays and edits values in `state.unit` (`pt` / `mm` / `in`) and converts any finalized
/// `committed_value` back into points (`pt`) for single-commit `PendingUiIntents` updates.
pub fn render_scrub_length_pt_field(
    ui: &mut Ui,
    value_pt: f64,
    state: &mut ScrubUnitFieldState,
    mut cfg_in_pt: ScrubUnitFieldConfig<'_>,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) -> ScrubUnitFieldOutput {
    let min_pt = cfg_in_pt.min.min(cfg_in_pt.max);
    let max_pt = cfg_in_pt.min.max(cfg_in_pt.max);
    let unit = if state.unit.is_length() { state.unit } else { ScrubUnitKind::Points };
    if state.unit != unit {
        state.unit = unit;
    }
    let unit_per_pt = unit.from_points(1.0);
    cfg_in_pt.min = unit.from_points(min_pt);
    cfg_in_pt.max = unit.from_points(max_pt);
    cfg_in_pt.step_per_pt *= unit_per_pt;

    let mut out = render_scrub_unit_field(ui, unit.from_points(value_pt), state, cfg_in_pt, tokens, meta);
    if let Some(committed_in_unit) = out.committed_value {
        let clamped_pt = out.display_unit.to_points(committed_in_unit).clamp(min_pt, max_pt);
        out.committed_value = Some(clamped_pt);
    }
    out
}

fn apply_drag_interaction(
    resp: &egui::Response,
    ui: &Ui,
    cfg: ScrubUnitFieldConfig<'_>,
    bounds: &ScrubChipBounds,
    state: &mut ScrubUnitFieldState,
    committed_value: &mut Option<f64>,
) {
    let ScrubChipBounds { min_val, max_val, clamped_in, .. } = *bounds;
    if !cfg.enabled {
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
        }
        if let Some(reason) = cfg.disabled_reason {
            resp.clone().on_hover_text(reason);
        }
        return;
    }

    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if resp.drag_started() {
        state.dragging = true;
        state.editing = false;
        state.drag_anchor_value = clamped_in;
        state.drag_current_value = clamped_in;
        state.drag_accum_dx_pt = 0.0;
    }
    if state.dragging && resp.dragged() {
        let dx = resp.drag_delta().x;
        state.drag_accum_dx_pt += dx;
        let factor = ui.input(|i| {
            if i.modifiers.shift {
                0.1
            } else if i.modifiers.alt {
                10.0
            } else {
                1.0
            }
        });
        let next = (state.drag_anchor_value + f64::from(state.drag_accum_dx_pt) * cfg.step_per_pt * factor).clamp(min_val, max_val);
        state.drag_current_value = next;
    }
    if state.dragging && resp.drag_stopped() {
        let finalized = state.drag_current_value.clamp(min_val, max_val);
        state.dragging = false;
        state.last_committed_value = Some(finalized);
        *committed_value = Some(finalized);
    }
}

/// Parse localized numeric text plus an optional unit suffix (`pt`, `mm`, `in`, `%`).
#[must_use]
pub fn parse_scrub_text_input(raw: &str, default_unit: ScrubUnitKind, preset: &LocalePreset) -> Option<(f64, Option<ScrubUnitKind>)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let suffix_candidates = ["points", "point", "pts", "pt", "millimeters", "millimeter", "mm", "inches", "inch", "in", "percent", "pct", "%", "\""];
    let lower = trimmed.to_ascii_lowercase();
    for sfx in suffix_candidates {
        if let Some(prefix_len) = lower.strip_suffix(sfx).map(str::len)
            && let Some(unit) = ScrubUnitKind::parse_suffix(sfx)
            && let Some(num_raw) = trimmed.get(..prefix_len)
        {
            let num_part = num_raw.trim();
            if let Some(val) = parse_localized_number_input(num_part, preset).filter(|v| v.is_finite()) {
                return Some((val, Some(unit)));
            }
        }
    }

    let _ = default_unit;
    parse_localized_number_input(trimmed, preset).filter(|v| v.is_finite()).map(|v| (v, None))
}

fn format_display_number(value: f64, decimals: usize, preset: &LocalePreset) -> String {
    let raw = format!("{value:.prec$}", prec = decimals);
    if preset.decimal_separator == '.' { raw } else { raw.replace('.', &preset.decimal_separator.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_conversion_roundtrips_pt_mm_and_inches() {
        let one_inch_pt = ScrubUnitKind::Inches.convert(1.0, ScrubUnitKind::Points);
        assert!((one_inch_pt - 72.0).abs() < 1e-9);

        let one_inch_mm = ScrubUnitKind::Inches.convert(1.0, ScrubUnitKind::Millimeters);
        assert!((one_inch_mm - 25.4).abs() < 1e-9);

        let roundtrip_pt = ScrubUnitKind::Millimeters.convert(one_inch_mm, ScrubUnitKind::Points);
        assert!((roundtrip_pt - 72.0).abs() < 1e-9);

        // Percent and Generic pass values unchanged across conversions.
        assert!((ScrubUnitKind::Percent.convert(42.5, ScrubUnitKind::Points) - 42.5).abs() < 1e-12);
    }

    #[test]
    fn parse_scrub_text_supports_localized_numbers_and_unit_suffixes() {
        let de = LocalePreset::for_locale("de-DE");
        let (v, u) = parse_scrub_text_input("25,4 mm", ScrubUnitKind::Points, &de).expect("de-DE mm input");
        assert!((v - 25.4).abs() < 1e-9);
        assert_eq!(u, Some(ScrubUnitKind::Millimeters));

        let en = LocalePreset::for_locale("en-US");
        let (v2, u2) = parse_scrub_text_input("1.5in", ScrubUnitKind::Millimeters, &en).expect("en-US inch input");
        assert!((v2 - 1.5).abs() < 1e-9);
        assert_eq!(u2, Some(ScrubUnitKind::Inches));
        let in_mm = ScrubUnitKind::Inches.convert(v2, ScrubUnitKind::Millimeters);
        assert!((in_mm - 38.1).abs() < 1e-9);
    }
}
