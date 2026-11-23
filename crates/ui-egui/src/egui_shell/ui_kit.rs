//! Unified `R15.10`-token-backed widget & modal primitives for `egui_shell`.
//!
//! Centralizes repeated hand-painted elided text, styled buttons/pills/chips,
//! checkbox/radio choice rows, modal inspection recorders, and modal shells
//! while preserving exact [`FrameInspectMeta`] widget IDs and AccessKit roles.

use std::sync::Arc;

use egui::scroll_area::ScrollBarVisibility;
use egui::{Color32, Context, CornerRadius, FontId, Galley, Pos2, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo};
use serde_json::{Value, json};

use crate::egui_shell::inspect_bridge::{ModalCaptureReport, ModalCaptureSpec, finalize_egui_modal_response};
use crate::egui_shell::{ActionRank, FrameInspectEntryInput, FrameInspectMeta, Tokens};
use crate::inspect::ZLayer;

/// Single-line elided [`Galley`] builder (`max_rows: 1`, `…` overflow).
#[must_use]
pub(super) fn elided_galley(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
    break_anywhere: bool,
) -> (Arc<Galley>, bool) {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(8.0), max_rows: 1, break_anywhere, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let elided = galley.elided;
    (galley, elided)
}

/// Check if an upcoming `size` slot intersects the visible clip rectangle.
#[must_use]
pub(super) fn slot_visible_in_clip(ui: &Ui, meta: Option<&FrameInspectMeta>, size: Vec2) -> bool {
    let clip = match meta {
        Some(m) if m.viewport_rect.is_positive() => ui.clip_rect().intersect(m.viewport_rect),
        _ => ui.clip_rect(),
    };
    let top_left = ui.cursor().min;
    let rect = Rect::from_min_size(top_left, size);
    rect.bottom() >= clip.top() && rect.top() <= clip.bottom() && rect.right() >= clip.left() && rect.left() <= clip.right()
}

/// Paint a single-line plain elided label without `FrameInspectMeta` registration.
pub(super) fn plain_clipped_label(ui: &mut Ui, text: &str, max_w: f32, min_h: f32, font: FontId, color: Color32, skip_offscreen: bool) {
    let (galley, elided) = elided_galley(ui.painter(), text, font, color, max_w.max(12.0), false);
    let h = galley.size().y.max(min_h);
    let w = galley.size().x.min(max_w).max(4.0);
    let size = Vec2::new(w, h);
    if skip_offscreen && !slot_visible_in_clip(ui, None, size) {
        ui.allocate_space(size);
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).galley(
        Pos2::new(rect.left(), rect.center().y - galley.size().y * 0.5),
        galley,
        color,
    );
    if elided {
        let _ = resp.on_hover_text(text);
    }
}

/// Configuration for [`clipped_label`].
pub(super) struct ClippedLabelSpec<'a> {
    pub widget_id: &'a str,
    pub role: &'a str,
    pub text: &'a str,
    pub max_w: f32,
    pub fixed_width: bool,
    pub min_h: f32,
    pub font: FontId,
    pub color: Color32,
    pub z_layer: ZLayer,
    pub tooltip: Option<&'a str>,
    pub record_meta: bool,
    pub record_value: bool,
    pub skip_offscreen: bool,
}

/// Render a single-line elided label with AccessKit role, truncation tooltip, and optional `FrameInspectMeta`.
pub(super) fn clipped_label(ui: &mut Ui, meta: &mut FrameInspectMeta, spec: ClippedLabelSpec<'_>) -> Option<Response> {
    let max_w = spec.max_w.max(16.0);
    let (galley, ellipsized) = elided_galley(ui.painter(), spec.text, spec.font, spec.color, max_w, false);
    let h = if spec.fixed_width { spec.min_h } else { galley.size().y.max(spec.min_h) };
    let w = if spec.fixed_width { max_w } else { galley.size().x.min(max_w) };
    let size = Vec2::new(w, h);
    if spec.skip_offscreen && !slot_visible_in_clip(ui, None, size) {
        ui.allocate_space(size);
        return None;
    }
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).galley(
        Pos2::new(rect.left(), rect.center().y - galley.size().y * 0.5),
        galley,
        spec.color,
    );
    let full = spec.text.to_owned();
    let enabled = ui.is_enabled();
    resp.widget_info(move || WidgetInfo::labeled(Role::Label, enabled, full.clone()));
    let resp = if let Some(tip) = spec.tooltip {
        resp.on_hover_text(tip)
    } else if ellipsized {
        resp.on_hover_text(spec.text)
    } else {
        resp
    };
    if spec.record_meta {
        let value = spec.record_value.then(|| json!(spec.text));
        record_visible_viewport(
            ui,
            meta,
            &resp,
            FrameInspectEntryInput {
                widget_id: spec.widget_id,
                role: spec.role,
                label: spec.text,
                command_id: None,
                value,
                enabled,
                disabled_reason: None,
                z_layer: spec.z_layer,
                ellipsized,
                full_label: ellipsized.then_some(spec.text),
            },
        );
    }
    Some(resp)
}

/// Width sizing strategy for [`styled_button`].
#[derive(Debug, Clone, Copy)]
pub(super) enum ButtonWidth {
    /// Allocate exact width `w` (clamped to `>= 20.0`).
    Fixed(f32),
    /// Fit text width plus horizontal padding `pad_x`, clamped to `[min_w.min(max_w), max_w]`.
    FitText { min_w: f32, max_w: f32, pad_x: f32 },
}

/// Text alignment inside [`styled_button`].
#[derive(Debug, Clone, Copy)]
pub(super) enum ButtonTextAlign {
    Center,
    Left(f32),
}

/// Tooltip presentation mode for [`styled_button`] and [`choice_row`].
#[derive(Debug, Clone, Copy)]
pub(super) enum TooltipMode {
    /// Standard `resp.on_hover_text` (or `on_disabled_hover_text` when disabled).
    Standard,
    /// Immediate modal-safe `Tooltip::always_open` (avoids outer `ScrollArea` hover swallowing).
    ModalAlwaysOpen,
}

/// Configuration for [`styled_button`], unifying hand-painted buttons, pills, chips, and segmented tabs.
pub(super) struct StyledButtonSpec<'a> {
    pub widget_id: &'a str,
    pub role: &'a str,
    pub a11y_role: Role,
    pub a11y_selectable: bool,
    pub label: &'a str,
    pub tooltip: Option<&'a str>,
    pub command_id: Option<&'a str>,
    pub width: ButtonWidth,
    pub height: f32,
    pub font_size: f32,
    pub radius: u8,
    pub text_align: ButtonTextAlign,
    pub break_anywhere: bool,
    pub primary: bool,
    pub selected: bool,
    pub enabled: bool,
    pub accent_text_when_selected: bool,
    pub border_override: Option<Color32>,
    pub value: Option<Value>,
    pub disabled_reason: Option<&'a str>,
    pub z_layer: ZLayer,
    pub skip_offscreen: bool,
    pub clip_interact_to_viewport: bool,
    pub tooltip_mode: TooltipMode,
}

/// Render a token-styled hand-painted button/pill/chip with elided text, AccessKit metadata,
/// hover tooltip, and [`FrameInspectMeta`] registration.
pub(super) fn styled_button(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: StyledButtonSpec<'_>) -> (Option<Response>, bool) {
    let fg = if !spec.enabled {
        tokens.text_disabled
    } else if spec.primary {
        tokens.on_accent
    } else if spec.selected {
        if spec.accent_text_when_selected { tokens.accent_strong } else { tokens.text_strong }
    } else {
        tokens.text_primary
    };

    let (w, text_max_w) = match spec.width {
        ButtonWidth::Fixed(fw) => {
            let clamped = fw.max(20.0);
            let pad = match spec.text_align {
                ButtonTextAlign::Left(l) => (l * 2.0).max(12.0),
                ButtonTextAlign::Center => 14.0,
            };
            (clamped, (clamped - pad).max(12.0))
        }
        ButtonWidth::FitText { min_w, max_w, pad_x } => {
            let (probe, _) =
                elided_galley(ui.painter(), spec.label, FontId::proportional(spec.font_size), fg, (max_w - pad_x).max(12.0), spec.break_anywhere);
            let computed = (probe.size().x + pad_x).clamp(min_w.min(max_w), max_w.max(min_w));
            (computed, (computed - pad_x).max(12.0))
        }
    };

    let size = Vec2::new(w, spec.height);
    if spec.skip_offscreen {
        let vis_clip = if meta.viewport_rect.is_positive() { ui.clip_rect().intersect(meta.viewport_rect) } else { ui.clip_rect() };
        let row_top = ui.cursor().top();
        let row_bot = row_top + spec.height;
        let can_skip_top = !ui.layout().main_wrap() || ui.available_width() >= w;
        if row_top > vis_clip.bottom() || (can_skip_top && row_bot < vis_clip.top()) {
            ui.allocate_space(size);
            return (None, false);
        }
    }

    let (rect, visible, mut resp) = if spec.clip_interact_to_viewport {
        let rect = ui.allocate_space(size).1;
        let visible = rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
        if !visible.is_positive() || visible.width() < 4.0 || visible.height() < 4.0 {
            return (None, false);
        }
        let resp = ui.interact(
            visible,
            ui.make_persistent_id(("egui_shell.ui_kit_btn", spec.widget_id)),
            if spec.enabled { Sense::click() } else { Sense::hover() },
        );
        (rect, visible, resp)
    } else {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let resp = if spec.enabled { resp } else { resp.on_hover_cursor(egui::CursorIcon::Default) };
        (rect, rect.intersect(ui.clip_rect()), resp)
    };

    let layout_w =
        if spec.clip_interact_to_viewport && visible.width() < rect.width() - 0.5 { (visible.width() - 14.0).max(4.0) } else { text_max_w };
    let (galley, ellipsized) = elided_galley(ui.painter(), spec.label, FontId::proportional(spec.font_size), fg, layout_w, spec.break_anywhere);

    let (bg, border) = button_colors(tokens, spec.primary, spec.selected, spec.enabled, resp.hovered(), spec.border_override);
    let stroke_w = if spec.border_override.is_some() { 1.5 } else { 1.0 };
    let r = CornerRadius::same(spec.radius);
    let painter = if spec.clip_interact_to_viewport { ui.painter().with_clip_rect(visible) } else { ui.painter().clone() };
    painter.rect(rect, r, bg, Stroke::new(stroke_w, border), StrokeKind::Inside);

    let clip_inner = if spec.clip_interact_to_viewport { visible } else { rect.shrink(3.0).intersect(ui.clip_rect()) };
    let text_pos = match spec.text_align {
        ButtonTextAlign::Center => {
            let base = if spec.clip_interact_to_viewport { visible } else { rect };
            Pos2::new(base.left() + ((base.width() - galley.size().x) * 0.5).max(3.0), rect.center().y - galley.size().y * 0.5)
        }
        ButtonTextAlign::Left(pad_l) => {
            let base_left = if spec.clip_interact_to_viewport { visible.min.x } else { rect.left() };
            Pos2::new(base_left + pad_l, rect.center().y - galley.size().y * 0.5)
        }
    };
    if !spec.clip_interact_to_viewport || galley.size().x <= visible.width() - 8.0 || ellipsized {
        ui.painter().with_clip_rect(clip_inner).galley(text_pos, galley, fg);
    }

    let a11y_label = spec.label.to_owned();
    let enabled = spec.enabled;
    let selected = spec.selected;
    let a11y_role = spec.a11y_role;
    if spec.a11y_selectable {
        resp.widget_info(move || WidgetInfo::selected(a11y_role, enabled, selected, a11y_label.clone()));
    } else {
        resp.widget_info(move || WidgetInfo::labeled(a11y_role, enabled, a11y_label.clone()));
    }

    let tip = resolve_tooltip(spec.enabled, spec.disabled_reason, ellipsized, spec.label, spec.tooltip);
    resp = apply_tooltip(ui, resp, spec.enabled, tip, spec.tooltip_mode);

    let entry = FrameInspectEntryInput {
        widget_id: spec.widget_id,
        role: spec.role,
        label: spec.label,
        command_id: spec.command_id,
        value: spec.value,
        enabled: spec.enabled,
        disabled_reason: (!spec.enabled).then_some(spec.disabled_reason).flatten(),
        z_layer: spec.z_layer,
        ellipsized,
        full_label: ellipsized.then_some(spec.label),
    };
    if spec.clip_interact_to_viewport {
        meta.record_response_with_unclipped(ui, &resp, rect, entry);
    } else {
        record_visible_viewport(ui, meta, &resp, entry);
    }

    let clicked = spec.enabled && resp.clicked();
    (Some(resp), clicked)
}

fn button_colors(
    tokens: &Tokens,
    primary: bool,
    selected: bool,
    enabled: bool,
    hovered: bool,
    border_override: Option<Color32>,
) -> (Color32, Color32) {
    if !enabled {
        return (tokens.panel_bg, tokens.divider);
    }
    let bg = if primary {
        if hovered { tokens.accent_hover } else { tokens.accent_strong }
    } else if selected {
        tokens.active_bg
    } else if hovered {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let border = if let Some(ov) = border_override {
        ov
    } else if primary {
        if hovered { tokens.hover_border } else { tokens.accent_strong }
    } else if selected {
        tokens.accent_strong
    } else if hovered {
        tokens.hover_border
    } else {
        tokens.button_border
    };
    (bg, border)
}

fn resolve_tooltip<'a>(
    enabled: bool,
    disabled_reason: Option<&'a str>,
    ellipsized: bool,
    label: &'a str,
    custom_tip: Option<&'a str>,
) -> Option<&'a str> {
    if !enabled {
        return disabled_reason.filter(|s| !s.is_empty());
    }
    if ellipsized {
        return Some(label);
    }
    custom_tip.filter(|t| !t.is_empty() && *t != label)
}

fn apply_tooltip(ui: &Ui, resp: Response, enabled: bool, tip: Option<&str>, mode: TooltipMode) -> Response {
    let Some(tip) = tip else {
        return resp;
    };
    match mode {
        TooltipMode::Standard => {
            if enabled {
                resp.on_hover_text(tip)
            } else {
                resp.on_disabled_hover_text(tip)
            }
        }
        TooltipMode::ModalAlwaysOpen => attach_always_open_tooltip(ui, resp, tip),
    }
}

/// Immediate modal-safe tooltip that resists outer `ScrollArea` hover capture.
pub(super) fn attach_always_open_tooltip(ui: &Ui, resp: Response, tip: &str) -> Response {
    if tip.is_empty() {
        return resp;
    }
    let delay = ui.style().interaction.tooltip_delay;
    let pointer_ready = ui.input(|i| !i.pointer.any_down() && i.pointer.is_still() && i.pointer.time_since_last_movement() >= delay);
    if resp.hovered() && pointer_ready {
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), resp.id.with("hover_tooltip"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(tip);
        });
    }
    resp
}

/// Configuration for [`choice_row`] (checkbox or radio button with elided label).
pub(super) struct ChoiceRowSpec<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub checked: bool,
    pub is_radio: bool,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
    pub font_size: f32,
    pub row_height: f32,
    pub z_layer: ZLayer,
    pub value: Value,
    pub tooltip_mode: TooltipMode,
}

/// Render a token-styled checkbox or radio choice row with elided label, AccessKit metadata, and `FrameInspectMeta`.
pub(super) fn choice_row(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: ChoiceRowSpec<'_>) -> bool {
    let avail_w = ui.available_width().max(48.0);
    let fg = if spec.enabled { tokens.text_primary } else { tokens.text_disabled };
    let (galley, ellipsized) = elided_galley(ui.painter(), spec.label, FontId::proportional(spec.font_size), fg, (avail_w - 24.0).max(16.0), false);
    let row_w = (24.0 + galley.size().x).min(avail_w);
    let vis_clip = if meta.viewport_rect.is_positive() { ui.clip_rect().intersect(meta.viewport_rect) } else { ui.clip_rect() };
    let row_top = ui.cursor().top();
    let row_bot = row_top + spec.row_height;
    if row_top > vis_clip.bottom() || row_bot < vis_clip.top() {
        ui.allocate_space(Vec2::new(row_w, spec.row_height));
        return false;
    }

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(row_w, spec.row_height), Sense::click());
    let resp = if spec.enabled { resp } else { resp.on_hover_cursor(egui::CursorIcon::Default) };
    let bx = Rect::from_min_size(Pos2::new(rect.left() + 1.0, rect.center().y - 6.5), Vec2::splat(13.0));
    let border = if !spec.enabled {
        tokens.divider
    } else if resp.hovered() {
        tokens.text_primary
    } else {
        tokens.button_border
    };

    if spec.is_radio {
        let c = bx.center();
        ui.painter().circle(c, 6.0, if spec.checked && spec.enabled { tokens.accent_strong } else { tokens.input_bg }, Stroke::new(1.0, border));
        if spec.checked {
            ui.painter().circle_filled(c, 2.5, if spec.enabled { tokens.on_accent } else { tokens.text_disabled });
        }
    } else {
        let radius = CornerRadius::same(tokens.metrics.radius_sm);
        ui.painter().rect_filled(bx, radius, if spec.checked && spec.enabled { tokens.accent_strong } else { tokens.input_bg });
        ui.painter().rect_stroke(bx, radius, Stroke::new(1.0, border), StrokeKind::Inside);
        if spec.checked {
            let ck = if spec.enabled { tokens.on_accent } else { tokens.text_disabled };
            let p0 = Pos2::new(bx.left() + 3.0, bx.center().y);
            let p1 = Pos2::new(bx.center().x - 1.0, bx.bottom() - 3.5);
            let p2 = Pos2::new(bx.right() - 3.0, bx.top() + 3.0);
            ui.painter().line_segment([p0, p1], Stroke::new(1.6, ck));
            ui.painter().line_segment([p1, p2], Stroke::new(1.6, ck));
        }
    }

    let text_rect = Rect::from_min_max(Pos2::new(bx.right() + 5.0, rect.top()), rect.max);
    ui.painter().with_clip_rect(text_rect.intersect(ui.clip_rect())).galley(
        Pos2::new(bx.right() + 5.0, rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    let ak_role = if spec.is_radio { Role::RadioButton } else { Role::CheckBox };
    let ak_name = spec.label.to_owned();
    let enabled = spec.enabled;
    let checked = spec.checked;
    resp.widget_info(move || WidgetInfo::selected(ak_role, enabled, checked, ak_name.clone()));

    let tip = resolve_tooltip(spec.enabled, spec.disabled_reason, ellipsized, spec.label, None);
    let resp = apply_tooltip(ui, resp, spec.enabled, tip, spec.tooltip_mode);

    record_visible_viewport(
        ui,
        meta,
        &resp,
        FrameInspectEntryInput {
            widget_id: spec.widget_id,
            role: if spec.is_radio { "radio" } else { "checkbox" },
            label: spec.label,
            command_id: None,
            value: Some(spec.value),
            enabled: spec.enabled,
            disabled_reason: spec.disabled_reason,
            z_layer: spec.z_layer,
            ellipsized,
            full_label: ellipsized.then_some(spec.label),
        },
    );
    spec.enabled && resp.clicked()
}

/// Record a widget in `meta` only when its interaction rect is visible in `ui.clip_rect()` and `meta.viewport_rect`.
pub(super) fn record_visible_viewport(ui: &Ui, meta: &mut FrameInspectMeta, resp: &Response, input: FrameInspectEntryInput<'_>) {
    let clipped = if meta.viewport_rect.is_positive() {
        resp.interact_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect)
    } else {
        resp.interact_rect.intersect(ui.clip_rect())
    };
    if clipped.is_positive() {
        meta.record_response(ui, resp, input);
    }
}

/// Record a widget in `meta` only when its interaction rect intersects `ui.clip_rect()`.
pub(super) fn record_if_visible(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, input: FrameInspectEntryInput<'_>) {
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() > 0.0 && clipped.height() > 0.0 {
        meta.record_response(ui, resp, input);
    }
}

/// Shared selectable pill button used across import/transform modals.
pub(super) fn choice_pill(ui: &mut Ui, tokens: &Tokens, label: &str, selected: bool, enabled: bool) -> Response {
    let mut text = egui::RichText::new(label);
    if selected {
        text = text.color(tokens.accent).strong();
    }
    ui.add_enabled(enabled, egui::Button::new(text).selected(selected))
}

/// Configuration for [`modal_button`] (shared by `app_modals` and `graph_modals`).
pub(super) struct ModalButtonSpec<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub command_id: Option<&'a str>,
    pub selected: bool,
    pub primary: bool,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
    pub ranked: bool,
}

/// Render an `egui::Button` for modal dialogs with `R15.10` token styling and `FrameInspectMeta` recording.
pub(super) fn modal_button(ui: &mut Ui, meta: &mut FrameInspectMeta, spec: ModalButtonSpec<'_>, tokens: &Tokens) -> Response {
    let btn = if spec.primary && spec.enabled {
        egui::Button::new(egui::RichText::new(spec.label).color(tokens.on_accent))
            .fill(tokens.accent_strong)
            .stroke(Stroke::new(1.0, tokens.accent_strong))
    } else if spec.selected {
        egui::Button::new(spec.label).fill(tokens.active_bg).stroke(Stroke::new(1.0, tokens.accent_strong))
    } else {
        egui::Button::new(spec.label)
    };
    let mut resp = ui.add_enabled(spec.enabled, btn);
    if let Some(reason) = spec.disabled_reason {
        resp = resp.on_disabled_hover_text(reason);
    }
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() > 0.0 && clipped.height() > 0.0 {
        let entry = FrameInspectEntryInput {
            widget_id: spec.widget_id,
            role: "button",
            label: spec.label,
            command_id: spec.command_id,
            value: Some(json!({ "selected": spec.selected })),
            enabled: spec.enabled,
            disabled_reason: spec.disabled_reason,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        };
        if spec.ranked {
            let rank = if spec.primary { ActionRank::Primary } else { ActionRank::Secondary };
            meta.record_ranked_response(ui, &resp, entry, rank);
        } else {
            meta.record_response(ui, &resp, entry);
        }
    }
    resp
}

/// Render a modal tab button with `Role::Tab` AccessKit info and `FrameInspectMeta` recording.
pub(super) fn modal_tab(ui: &mut Ui, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, selected: bool, tokens: &Tokens) -> Response {
    let btn = egui::Button::new(label)
        .fill(if selected { tokens.active_bg } else { tokens.surface_bg })
        .stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { tokens.accent_strong } else { tokens.border }));
    let resp = ui.add(btn);
    resp.widget_info(|| WidgetInfo::selected(Role::Tab, true, selected, label));
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

/// Record a modal checkbox response into `meta` when visible.
pub(super) fn record_modal_checkbox(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, checked: bool) {
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

/// Record a modal text input response with `Role::TextInput` AccessKit info when visible.
pub(super) fn record_modal_input(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, text: &str, enabled: bool) {
    let ak_lbl = label.to_owned();
    resp.widget_info(move || WidgetInfo::labeled(Role::TextInput, enabled, ak_lbl.clone()));
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

/// Record a modal static text label into `meta`.
pub(super) fn record_modal_text(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, text: &str) {
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

/// Configuration for [`show_scrolled_modal`], used by `app_modals` dialogs.
pub(super) struct ScrolledModalCfg<'a> {
    pub tokens: &'a Tokens,
    pub modal_id: egui::Id,
    pub scroll_id: &'a str,
    pub widget_id: &'a str,
    pub title: &'a str,
    pub inner_w: f32,
    pub max_h: f32,
    pub body_w: f32,
    pub body_max_h: f32,
    pub footer_separator: bool,
}

/// Show a standard heading + vertical scroll body + optional separator + footer modal with `Role::Dialog` capture.
///
/// Threads `shared: &mut S` sequentially through `header_extra`, `body_fn`, and `footer_fn` so all three
/// phases can mutate dialog state and `PendingUiIntents` without closure borrow conflicts.
pub(super) fn show_scrolled_modal<S, H, B, F>(
    ctx: &Context,
    meta: &mut FrameInspectMeta,
    cfg: ScrolledModalCfg<'_>,
    shared: &mut S,
    header_extra: H,
    body_fn: B,
    footer_fn: F,
) -> ModalCaptureReport
where
    H: FnOnce(&mut Ui, &mut FrameInspectMeta, &mut S),
    B: FnOnce(&mut Ui, &mut FrameInspectMeta, &mut S),
    F: FnOnce(&mut Ui, &mut FrameInspectMeta, &mut S),
{
    let modal_resp = egui::Modal::new(cfg.modal_id).backdrop_color(cfg.tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(cfg.inner_w);
        ui.set_max_width(cfg.inner_w);
        ui.set_max_height(cfg.max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.heading(cfg.title);
        ui.add_space(4.0);
        header_extra(ui, meta, shared);

        egui::ScrollArea::vertical()
            .id_salt(cfg.scroll_id)
            .max_height(cfg.body_max_h)
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(cfg.body_w);
                ui.set_max_width(cfg.body_w);
                body_fn(ui, meta, shared);
            });

        if cfg.footer_separator {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);
        }
        footer_fn(ui, meta, shared);
    });

    modal_resp.response.widget_info(|| WidgetInfo::labeled(Role::Dialog, true, cfg.title));
    finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: cfg.widget_id, title: cfg.title, z_dialog: ZLayer::ModalDialog },
        meta,
    )
}
