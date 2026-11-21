// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Copyright (c) 2026 CurveBook contributors
// Adapted from upstream MIT OR Apache-2.0 pure-egui helpers; see ATTRIBUTION.md and NOTICE.

//! Pure-`egui` widget primitives adapted for CurveBook's [`Tokens`] and `i18n::tr_or`.

use std::borrow::Cow;

use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::Tokens;
use crate::i18n;

/// Width of a [`split_button`]'s menu chevron segment.
pub const SPLIT_ARROW: f32 = 18.0;

/// Panel footer context stack tag used by [`bottom_bar`] and [`panel_footer`].
pub const PANEL_FOOTER: &str = "curvebook-panel-footer";
const FOOTER_HEIGHT: f32 = 31.0;

/// Seconds without wheel input that terminate an active wheel stream.
pub const WHEEL_STREAM_GAP: f64 = 0.5;

fn tr_text<'a>(ui: &Ui, key_or_text: &'a str) -> Cow<'a, str> {
    let locale = Tokens::get(ui.ctx()).locale;
    i18n::tr_or(locale, key_or_text)
}

fn next_label_id() -> egui::Id {
    egui::Id::unique("curvebook-next-input-label")
}

fn paint_chevron(ui: &Ui, rect: Rect, down: bool, color: Color32) {
    let c = rect.center();
    let r = (rect.width().min(rect.height()) * 0.28).max(2.5);
    let stroke = Stroke::new(1.5, color);
    if down {
        let left = pos2(c.x - r, c.y - r * 0.4);
        let tip = pos2(c.x, c.y + r * 0.55);
        let right = pos2(c.x + r, c.y - r * 0.4);
        ui.painter().line_segment([left, tip], stroke);
        ui.painter().line_segment([tip, right], stroke);
    } else {
        let top = pos2(c.x - r * 0.4, c.y - r);
        let tip = pos2(c.x + r * 0.55, c.y);
        let bottom = pos2(c.x - r * 0.4, c.y + r);
        ui.painter().line_segment([top, tip], stroke);
        ui.painter().line_segment([tip, bottom], stroke);
    }
}

fn paint_symbol(ui: &Ui, symbol: &str, rect: Rect, color: Color32) {
    match symbol {
        "chevron-down" => paint_chevron(ui, rect, true, color),
        "chevron-right" => paint_chevron(ui, rect, false, color),
        "plus" | "+" => {
            let c = rect.center();
            let half = (rect.width().min(rect.height()) * 0.32).max(3.0);
            let stroke = Stroke::new(1.5, color);
            ui.painter().line_segment([pos2(c.x - half, c.y), pos2(c.x + half, c.y)], stroke);
            ui.painter().line_segment([pos2(c.x, c.y - half), pos2(c.x, c.y + half)], stroke);
        }
        "minus" | "-" => {
            let c = rect.center();
            let half = (rect.width().min(rect.height()) * 0.32).max(3.0);
            ui.painter().line_segment([pos2(c.x - half, c.y), pos2(c.x + half, c.y)], Stroke::new(1.5, color));
        }
        "close" | "x" => {
            let c = rect.center();
            let half = (rect.width().min(rect.height()) * 0.28).max(3.0);
            let stroke = Stroke::new(1.5, color);
            ui.painter().line_segment([pos2(c.x - half, c.y - half), pos2(c.x + half, c.y + half)], stroke);
            ui.painter().line_segment([pos2(c.x - half, c.y + half), pos2(c.x + half, c.y - half)], stroke);
        }
        "check" => {
            let stroke = Stroke::new(1.6, color);
            ui.painter().line_segment([rect.left_center() + vec2(2.0, 0.0), rect.center_bottom() + vec2(-1.0, -2.5)], stroke);
            ui.painter().line_segment([rect.center_bottom() + vec2(-1.0, -2.5), rect.right_top() + vec2(-2.0, 2.5)], stroke);
        }
        "search" => {
            let c = rect.center() - vec2(1.5, 1.5);
            let r = (rect.width().min(rect.height()) * 0.26).max(3.0);
            let stroke = Stroke::new(1.5, color);
            ui.painter().circle_stroke(c, r, stroke);
            let dir = Vec2::splat(std::f32::consts::FRAC_1_SQRT_2);
            ui.painter().line_segment([c + dir * r, c + dir * (r + 4.0)], stroke);
        }
        other => {
            let size = (rect.height() * 0.72).clamp(10.0, 18.0);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, other, egui::FontId::proportional(size), color);
        }
    }
}

// ============================================================================
// Toggles (checkboxes and radio buttons)
// ============================================================================

/// Width a [`check`] or [`radio`] row takes with translated `label`.
#[must_use]
pub fn choice_width(ui: &Ui, label: &str) -> f32 {
    let text = tr_text(ui, label);
    18.0 + ui.painter().layout_no_wrap(text.into_owned(), egui::FontId::proportional(12.5), Color32::PLACEHOLDER).size().x
}

fn choice_row(ui: &mut Ui, text: &str, enabled: bool) -> (Rect, Response, Color32) {
    let t = Tokens::get(ui.ctx());
    let galley =
        ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.5), if enabled { t.text_primary } else { t.text_disabled });
    let (rect, resp) =
        ui.allocate_exact_size(vec2(18.0 + galley.size().x, 20.0f32.max(galley.size().y)), if enabled { Sense::click() } else { Sense::hover() });
    let bx = Rect::from_min_size(pos2(rect.left(), rect.center().y - 6.5), Vec2::splat(13.0));
    ui.painter().galley(pos2(bx.right() + 5.0, rect.center().y - galley.size().y / 2.0), galley, t.text_primary);
    let border = if !enabled {
        t.divider
    } else if resp.hovered() {
        t.text_primary
    } else {
        t.button_border
    };
    (bx, resp, border)
}

/// Panel-style checkbox with a disabled state. Returns `true` when toggled.
pub fn check(ui: &mut Ui, label: &str, value: bool, enabled: bool) -> bool {
    check3(ui, label, Some(value), enabled)
}

/// [`check`] with hover tooltip `tip`. Returns `true` when toggled.
pub fn check_tip(ui: &mut Ui, label: &str, value: bool, enabled: bool, tip: &str) -> bool {
    check3_tip(ui, label, Some(value), enabled, tip)
}

/// [`check3`] with hover tooltip `tip`. Returns `true` when clicked.
pub fn check3_tip(ui: &mut Ui, label: &str, value: Option<bool>, enabled: bool, tip: &str) -> bool {
    let text = tr_text(ui, label).into_owned();
    let tip_text = tr_text(ui, tip).into_owned();
    let (clicked, resp) = check_row(ui, &text, value, enabled);
    if !tip_text.is_empty() {
        resp.on_hover_text(tip_text);
    }
    clicked
}

/// Three-state [`check`]: `None` shows an indeterminate dash. Returns `true` when clicked.
pub fn check3(ui: &mut Ui, label: &str, value: Option<bool>, enabled: bool) -> bool {
    let text = tr_text(ui, label).into_owned();
    check_row(ui, &text, value, enabled).0
}

/// [`check`] for untranslated user data names (sheet titles, datasets). Returns `true` when toggled.
pub fn check_name(ui: &mut Ui, name: &str, value: bool) -> bool {
    check_row(ui, name, Some(value), true).0
}

/// Checkbox without visible label text, exposing `name` to screen readers.
pub fn check_box(ui: &mut Ui, name: &str, value: bool, enabled: bool) -> bool {
    check_row_named(ui, "", name, Some(value), enabled).0
}

fn check_row(ui: &mut Ui, text: &str, value: Option<bool>, enabled: bool) -> (bool, Response) {
    check_row_named(ui, text, text, value, enabled)
}

fn check_row_named(ui: &mut Ui, text: &str, name: &str, value: Option<bool>, enabled: bool) -> (bool, Response) {
    let t = Tokens::get(ui.ctx());
    let (bx, resp, border) = choice_row(ui, text, enabled);
    let checked = value == Some(true);
    ui.painter().rect_filled(bx, CornerRadius::same(2), if checked && enabled { t.accent_strong } else { t.input_bg });
    ui.painter().rect_stroke(bx, CornerRadius::same(2), Stroke::new(1.0, border), StrokeKind::Inside);
    match value {
        Some(true) => {
            let c = if enabled { Color32::WHITE } else { t.text_disabled };
            ui.painter().line_segment([bx.left_center() + vec2(3.0, 0.0), bx.center_bottom() + vec2(-1.0, -3.5)], Stroke::new(1.6, c));
            ui.painter().line_segment([bx.center_bottom() + vec2(-1.0, -3.5), bx.right_top() + vec2(-3.0, 3.0)], Stroke::new(1.6, c));
        }
        None => {
            let c = if enabled { t.text_primary } else { t.text_disabled };
            ui.painter().line_segment([bx.left_center() + vec2(3.0, 0.0), bx.right_center() - vec2(3.0, 0.0)], Stroke::new(1.6, c));
        }
        Some(false) => {}
    }
    let accessible_name = name.to_owned();
    let row_enabled = enabled && ui.is_enabled();
    resp.widget_info(move || match value {
        Some(on) => egui::WidgetInfo::selected(egui::Role::CheckBox, row_enabled, on, accessible_name.clone()),
        None => egui::WidgetInfo::labeled(egui::Role::CheckBox, row_enabled, accessible_name.clone()),
    });
    (enabled && resp.clicked(), resp)
}

/// Radio button matching [`check`] with an explicit `enabled` state.
pub fn radio(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> bool {
    let text = tr_text(ui, label).into_owned();
    radio_row(ui, &text, &text, selected, enabled)
}

/// Radio button without visible label text, exposing `name` to screen readers.
pub fn radio_button(ui: &mut Ui, name: &str, selected: bool, enabled: bool) -> bool {
    radio_row(ui, "", name, selected, enabled)
}

fn radio_row(ui: &mut Ui, text: &str, name: &str, selected: bool, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (bx, resp, border) = choice_row(ui, text, enabled);
    let c = bx.center();
    ui.painter().circle(c, 6.0, if selected && enabled { t.accent_strong } else { t.input_bg }, Stroke::new(1.0, border));
    if selected {
        ui.painter().circle_filled(c, 2.5, if enabled { Color32::WHITE } else { t.text_disabled });
    }
    let accessible_name = name.to_owned();
    let row_enabled = enabled && ui.is_enabled();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::RadioButton, row_enabled, selected, accessible_name.clone()));
    enabled && resp.clicked()
}

// ============================================================================
// Buttons & Interactive Chrome (§10.3 & §10.6)
// ============================================================================

/// Parameters controlling [`paint_interactive_chrome`].
#[derive(Clone, Copy, Debug)]
pub struct InteractiveChromeSpec {
    pub radius: CornerRadius,
    pub base_bg: Color32,
    pub base_border: Option<Color32>,
    pub hovered: bool,
    pub selected: bool,
    pub pressed: bool,
    pub enabled: bool,
}

/// Paint canonical §10.6 rest / hover / active / selected / disabled background and border chrome.
pub fn paint_interactive_chrome(ui: &Ui, rect: Rect, tokens: &Tokens, spec: InteractiveChromeSpec) {
    let (bg, stroke) = if !spec.enabled {
        (spec.base_bg, spec.base_border.map_or(Stroke::NONE, |c| Stroke::new(1.0, c)))
    } else if spec.pressed {
        (tokens.active_bg, Stroke::new(2.0, tokens.focus_ring))
    } else if spec.selected {
        let sc = if spec.hovered { tokens.focus_ring } else { tokens.accent };
        (tokens.active_bg, Stroke::new(1.5, sc))
    } else if spec.hovered {
        (tokens.hover_bg, Stroke::new(1.0, tokens.hover_border))
    } else {
        (spec.base_bg, spec.base_border.map_or(Stroke::NONE, |c| Stroke::new(1.0, c)))
    };
    if bg != Color32::TRANSPARENT || stroke != Stroke::NONE {
        ui.painter().rect(rect, spec.radius, bg, stroke, StrokeKind::Inside);
    }
}

/// Paint canonical §10.6 hover/selected chrome for list rows, menu headers, and transparent buttons.
pub fn paint_hover_row(ui: &Ui, rect: Rect, radius: CornerRadius, hovered: bool, selected: bool, tokens: &Tokens) {
    paint_interactive_chrome(
        ui,
        rect,
        tokens,
        InteractiveChromeSpec { radius, base_bg: Color32::TRANSPARENT, base_border: None, hovered, selected, pressed: false, enabled: true },
    );
}

/// Square icon/symbol button; `selected` draws the active well.
pub fn icon_button(ui: &mut Ui, symbol: &str, tip: &str, selected: bool, size: f32) -> Response {
    let (_, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_icon_button(ui, resp, symbol, tip, selected)
}

/// Draw [`icon_button`] onto an existing `resp`.
pub fn paint_icon_button(ui: &mut Ui, resp: Response, symbol: &str, tip: &str, selected: bool) -> Response {
    let text = tr_text(ui, tip).into_owned();
    paint_icon_button_named(ui, resp, symbol, &text, &text, selected)
}

/// Draw an icon button with explicit screen-reader `name` and hover `tip`.
pub fn paint_icon_button_named(ui: &mut Ui, resp: Response, symbol: &str, name: &str, tip: &str, selected: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let rect = resp.rect;
    let size = rect.width();
    let enabled = ui.is_enabled();
    paint_interactive_chrome(
        ui,
        rect,
        &t,
        InteractiveChromeSpec {
            radius: CornerRadius::same(t.metrics.radius_md),
            base_bg: Color32::TRANSPARENT,
            base_border: None,
            hovered: resp.hovered(),
            selected,
            pressed: resp.is_pointer_button_down_on(),
            enabled,
        },
    );
    let pad = (size * 0.2).round();
    paint_symbol(ui, symbol, rect.shrink(pad), if selected { t.text_strong } else { t.icon });
    let accessible_name = name.to_owned();
    resp.widget_info(move || icon_button_info(enabled, selected, &accessible_name));
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// Compact flat text button with fixed `width`.
pub fn flat_button(ui: &mut Ui, text: &str, width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, text).into_owned();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    let r = CornerRadius::same(t.metrics.radius_md);
    ui.painter().rect_filled(rect, r, t.button_bg);
    flat_fill(ui, rect, &resp);
    ui.painter().with_clip_rect(rect).text(rect.center(), egui::Align2::CENTER_CENTER, &label, egui::FontId::proportional(12.5), t.text_strong);
    let ui_enabled = ui.is_enabled();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, ui_enabled, label.clone()));
    resp
}

/// [`flat_button`] expanding horizontally to fit localized labels wider than `min_width`.
pub fn flat_button_fit(ui: &mut Ui, text: &str, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, text).into_owned();
    let width = flat_button_fit_width(ui, &label, min_width);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    let r = CornerRadius::same(t.metrics.radius_md);
    ui.painter().rect_filled(rect, r, t.button_bg);
    flat_fill(ui, rect, &resp);
    ui.painter().with_clip_rect(rect).text(rect.center(), egui::Align2::CENTER_CENTER, &label, egui::FontId::proportional(12.5), t.text_strong);
    let ui_enabled = ui.is_enabled();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, ui_enabled, label.clone()));
    resp
}

/// Width computed by [`flat_button_fit`] for `text`.
#[must_use]
pub fn flat_button_fit_width(ui: &Ui, text: &str, min_width: f32) -> f32 {
    let label = tr_text(ui, text).into_owned();
    let galley = ui.painter().layout_no_wrap(label, egui::FontId::proportional(12.5), Color32::PLACEHOLDER);
    (galley.size().x + 24.0).max(min_width)
}

/// Split button with primary click area and trailing menu chevron (`(main, arrow)`).
pub fn split_button(ui: &mut Ui, text: &str, width: f32) -> (Response, Response) {
    split_button_named(ui, text, "", width)
}

/// [`split_button`] with explicit tooltip and accessible name `arrow_tip` on the chevron segment.
pub fn split_button_named(ui: &mut Ui, text: &str, arrow_tip: &str, width: f32) -> (Response, Response) {
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, text).into_owned();
    let arrow_label = tr_text(ui, arrow_tip).into_owned();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let rect = rect.shrink2(vec2(0.0, 0.5));
    let split = rect.right() - SPLIT_ARROW;
    let main_rect = Rect::from_min_max(rect.min, pos2(split, rect.bottom()));
    let arrow_rect = Rect::from_min_max(pos2(split, rect.top()), rect.max);
    let main = ui.interact(main_rect, ui.scope_id().with((text, "split")), Sense::click());
    let arrow = ui.interact(arrow_rect, ui.scope_id().with((text, "split-arrow")), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(t.metrics.radius_md), t.button_bg);
    flat_fill(ui, main_rect, &main);
    flat_fill(ui, arrow_rect, &arrow);
    ui.painter().vline(split, rect.y_range().shrink(5.0), Stroke::new(1.0, t.panel_bg));
    ui.painter().with_clip_rect(main_rect).text(
        main_rect.center(),
        egui::Align2::CENTER_CENTER,
        &label,
        egui::FontId::proportional(12.5),
        t.text_strong,
    );
    paint_chevron(ui, Rect::from_center_size(arrow_rect.center(), vec2(12.0, 12.0)), true, t.text_muted);
    let ui_enabled = ui.is_enabled();
    main.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, ui_enabled, label.clone()));
    if arrow_label.is_empty() {
        return (main, arrow);
    }
    let arrow_tip_hover = arrow_label.clone();
    arrow.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, ui_enabled, arrow_label.clone()));
    (main, arrow.on_hover_text(arrow_tip_hover))
}

fn flat_fill(ui: &Ui, rect: Rect, resp: &Response) {
    let t = Tokens::get(ui.ctx());
    paint_interactive_chrome(
        ui,
        rect,
        &t,
        InteractiveChromeSpec {
            radius: CornerRadius::same(t.metrics.radius_md),
            base_bg: Color32::TRANSPARENT,
            base_border: None,
            hovered: resp.hovered(),
            selected: false,
            pressed: resp.is_pointer_button_down_on(),
            enabled: ui.is_enabled(),
        },
    );
}

/// Width [`primary_button`] and [`secondary_button`] allocate for `text`.
#[must_use]
pub fn button_width(ui: &Ui, text: &str) -> f32 {
    let label = tr_text(ui, text).into_owned();
    let galley = ui.painter().layout_no_wrap(label, egui::FontId::proportional(12.5), Color32::PLACEHOLDER);
    (galley.size().x + 32.0).max(72.0)
}

/// Accent call-to-action button (e.g. `OK`, `Run Analysis`).
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    primary_button_enabled(ui, text, true)
}

/// [`primary_button`] rendered disabled when `enabled` is `false`.
pub fn primary_button_enabled(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    primary_button_sized(ui, text, enabled, 72.0)
}

pub fn primary_button_width(ui: &mut Ui, text: &str, min_width: f32) -> Response {
    primary_button_sized(ui, text, true, min_width)
}

fn primary_button_sized(ui: &mut Ui, text: &str, enabled: bool, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, text).into_owned();
    let fg = if enabled { t.on_accent } else { t.text_disabled };
    let galley = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(12.5), fg);
    let w = galley.size().x + 32.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(min_width), 28.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = match (enabled, resp.hovered()) {
        (false, _) => t.input_bg,
        (true, true) => t.accent_hover,
        (true, false) => t.accent_strong,
    };
    let radius = CornerRadius::same(t.metrics.radius_md);
    ui.painter().rect_filled(rect, radius, bg);
    if !enabled {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, t.hover_border), StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    let row_enabled = enabled && ui.is_enabled();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, row_enabled, label.clone()));
    resp
}

/// Secondary action button (e.g. `Cancel`).
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    secondary_button_width_enabled(ui, text, 72.0, true)
}

/// [`secondary_button`] rendered disabled when `enabled` is `false`.
pub fn secondary_button_enabled(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    secondary_button_width_enabled(ui, text, 72.0, enabled)
}

pub fn secondary_button_width(ui: &mut Ui, text: &str, min_width: f32) -> Response {
    secondary_button_width_enabled(ui, text, min_width, true)
}

fn secondary_button_width_enabled(ui: &mut Ui, text: &str, min_width: f32, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, text).into_owned();
    let fg = if enabled { t.text_primary } else { t.text_disabled };
    let galley = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(12.5), fg);
    let w = galley.size().x + 32.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(min_width), 28.0), if enabled { Sense::click() } else { Sense::hover() });
    paint_interactive_chrome(
        ui,
        rect,
        &t,
        InteractiveChromeSpec {
            radius: CornerRadius::same(t.metrics.radius_md),
            base_bg: if enabled { t.button_bg } else { t.panel_bg },
            base_border: Some(if enabled { t.button_border } else { t.divider }),
            hovered: resp.hovered(),
            selected: false,
            pressed: resp.is_pointer_button_down_on(),
            enabled,
        },
    );
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    let ui_enabled = enabled && ui.is_enabled();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, ui_enabled, label.clone()));
    resp
}

/// Toggleable symbol button returning `true` when clicked.
pub fn toggle_icon(ui: &mut Ui, on_symbol: &str, on: bool, size: f32, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, tip).into_owned();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if on {
        paint_symbol(ui, on_symbol, rect.shrink(size * 0.2), if resp.hovered() { t.text_primary } else { t.icon });
    } else if resp.hovered() {
        paint_symbol(ui, on_symbol, rect.shrink(size * 0.2), t.text_disabled);
    }
    let accessible = label.clone();
    let ui_enabled = ui.is_enabled();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Button, ui_enabled, on, accessible.clone()));
    resp.on_hover_text(label).clicked()
}

/// Icon button that disables interaction when `enabled` is `false`.
pub fn icon_button_enabled(ui: &mut Ui, symbol: &str, tip: &str, selected: bool, enabled: bool, size: f32) -> Response {
    if enabled {
        return icon_button(ui, symbol, tip, selected, size);
    }
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, tip).into_owned();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let pad = (size * 0.2).round();
    paint_symbol(ui, symbol, rect.shrink(pad), t.text_disabled);
    let accessible = label.clone();
    resp.widget_info(move || icon_button_info(false, selected, &accessible));
    if label.is_empty() { resp } else { resp.on_hover_text(label) }
}

pub fn icon_button_info(enabled: bool, selected: bool, name: &str) -> egui::WidgetInfo {
    egui::WidgetInfo { selected: selected.then_some(true), ..egui::WidgetInfo::labeled(egui::Role::Button, enabled, name) }
}

/// Menu row with optional check mark and disabled state.
pub fn menu_item(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    let text = tr_text(ui, label).into_owned();
    menu_item_name(ui, &text, enabled, checked)
}

/// [`menu_item`] taking a literal untranslated display name.
pub fn menu_item_name(ui: &mut Ui, label: &str, enabled: bool, checked: bool) -> bool {
    let text = if checked { format!("✓ {label}") } else { format!("   {label}") };
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).size(12.5)).frame_when_inactive(false)).clicked()
}

// ============================================================================
// Labels & Accessibility Pairing
// ============================================================================

/// Single-line width of `text` as drawn by `ui`.
#[must_use]
pub fn text_width(ui: &Ui, text: egui::RichText) -> f32 {
    egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body).size().x
}

#[must_use]
pub fn dim_label_width(ui: &Ui, text: &str) -> f32 {
    let shown = tr_text(ui, text).into_owned();
    text_width(ui, egui::RichText::new(shown).size(12.5))
}

/// Bold section heading in inspector/panel forms (§10.2 `type.section` or `14 pt` regular fallback).
pub fn section_header(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let shown = tr_text(ui, text).into_owned();
    let font = super::fonts::section_heading_font_id(ui.ctx(), &t);
    ui.add_space(t.metrics.space_sm);
    ui.label(egui::RichText::new(shown).font(font).color(t.text_strong));
    ui.add_space(t.metrics.space_xs);
}

/// Collapsible section header with disclosure chevron; returns `true` when clicked.
pub fn section_toggle(ui: &mut Ui, text: &str, open: bool) -> bool {
    let shown = tr_text(ui, text).into_owned();
    section_toggle_name(ui, &shown, open).clicked()
}

/// [`section_toggle`] taking a literal untranslated name and returning the full `Response`.
pub fn section_toggle_name(ui: &mut Ui, text: &str, open: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    paint_hover_row(ui, rect, CornerRadius::same(t.metrics.radius_md), resp.hovered(), false, &t);
    let chevron = Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(12.0, 12.0));
    paint_chevron(ui, chevron, open, if resp.hovered() { t.text_strong } else { t.icon });
    let font = super::fonts::section_heading_font_id(ui.ctx(), &t);
    ui.painter().with_clip_rect(rect).text(pos2(rect.left() + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, text, font, t.text_strong);
    let accessible = text.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, true, accessible.clone()));
    ui.ctx().accesskit_node_builder(resp.id, |node| node.set_expanded(open));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Full-width 1 px horizontal divider with vertical spacing.
pub fn divider(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.divider);
    ui.add_space(6.0);
}

#[derive(Clone, Copy)]
struct NextLabel {
    id: egui::Id,
    viewport: egui::ViewportId,
    pass: u64,
    at: Pos2,
    next_auto_id: egui::Id,
}

fn remember_next_label(ui: &Ui, id: egui::Id) {
    let label = NextLabel {
        id,
        viewport: ui.ctx().viewport_id(),
        pass: ui.ctx().cumulative_pass_nr(),
        at: ui.next_widget_position(),
        next_auto_id: ui.next_auto_id(),
    };
    ui.ctx().data_mut(|d| d.insert_temp(next_label_id(), label));
}

pub struct PreviousLabel {
    id: egui::Id,
    at: Pos2,
    immediate: bool,
}

pub fn take_previous_label(ui: &Ui) -> Option<PreviousLabel> {
    let label = ui.ctx().data_mut(|d| {
        let label = d.get_temp::<NextLabel>(next_label_id());
        d.remove::<NextLabel>(next_label_id());
        label
    })?;
    (label.viewport == ui.ctx().viewport_id() && label.pass == ui.ctx().cumulative_pass_nr()).then(|| PreviousLabel {
        id: label.id,
        at: label.at,
        immediate: ui.next_auto_id() == label.next_auto_id,
    })
}

pub fn label_field(resp: Response, label: Option<PreviousLabel>) -> Response {
    let Some(label) = label else { return resp };
    let adjacent = label.immediate || resp.rect.min.distance(label.at) <= 16.0 || resp.rect.contains(label.at);
    if adjacent { resp.labelled_by(label.id) } else { resp }
}

/// Secondary field label remembered for the immediately following input widget.
pub fn dim_label(ui: &mut Ui, text: &str) -> Response {
    let shown = tr_text(ui, text).into_owned();
    let resp = dim_name(ui, &shown);
    remember_next_label(ui, resp.id);
    resp
}

/// Styled field label remembered for the immediately following input widget.
pub fn field_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    let resp = ui.label(text);
    remember_next_label(ui, resp.id);
    resp
}

/// Literal untranslated field label.
pub fn dim_name(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).color(t.text_primary).size(12.5))
}

/// Horizontal form row with fixed-width `label` column paired to the control emitted by `add`.
///
/// Caps label width at `40%` of the panel width per §10.4; stacks above `add` instead of truncating.
pub fn label_row(ui: &mut Ui, label: &str, label_width: f32, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    let shown = tr_text(ui, label).into_owned();
    let cap = (ui.available_width() * 0.40).max(48.0);
    let capped_w = label_width.min(cap);
    let measured_w = if shown.trim().is_empty() {
        0.0
    } else {
        ui.painter().layout_no_wrap(shown.clone(), egui::FontId::proportional(t.metrics.font_ui_body_pt), t.text_primary).size().x
    };
    if !shown.trim().is_empty() && measured_w > cap {
        ui.vertical(|ui| {
            let resp = ui.label(egui::RichText::new(&shown).size(t.metrics.font_ui_body_pt).color(t.text_primary));
            ui.label_inputs_by(resp.id, add);
        });
        return;
    }
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(capped_w, 24.0), Sense::hover());
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, &shown, egui::FontId::proportional(12.5), t.text_primary);
        let accessible = shown.clone();
        let ui_enabled = ui.is_enabled();
        resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, ui_enabled, accessible.clone()));
        if label.trim().is_empty() {
            add(ui);
        } else {
            ui.label_inputs_by(resp.id, add);
        }
    });
}

/// Panel section sub-header styled with `type.section` (`13 pt` bold or `14 pt` regular fallback).
pub fn subheader(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let shown = tr_text(ui, text).into_owned();
    let font = super::fonts::section_heading_font_id(ui.ctx(), &t);
    ui.label(egui::RichText::new(shown).font(font).color(t.text_strong));
}

/// Label rendered with a dotted underline acting as an interactive link button.
pub fn link_label(ui: &mut Ui, text: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let shown = tr_text(ui, text).into_owned();
    let galley = ui.painter().layout_no_wrap(shown.clone(), egui::FontId::proportional(12.5), t.text_strong);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(0.0, 3.0), Sense::click());
    let y = rect.top() + galley.size().y + 1.0;
    ui.painter().galley(rect.min, galley, t.text_strong);
    let mut x = rect.left();
    while x < rect.right() - 1.0 {
        ui.painter().line_segment([pos2(x, y), pos2((x + 1.0).min(rect.right()), y)], Stroke::new(1.0, t.text_muted));
        x += 2.5;
    }
    let ui_enabled = ui.is_enabled();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, ui_enabled, shown.clone()));
    resp
}

// ============================================================================
// Panel Chrome (footers, bordered lists, tab bars, layout/orientation toggles)
// ============================================================================

#[must_use]
pub fn panel_footer_bottom(ui: &Ui) -> Option<f32> {
    ui.stack().iter().find_map(|stack| stack.tags().get_downcast::<Option<f32>>(PANEL_FOOTER)).copied().flatten()
}

#[must_use]
pub fn panel_list_height(ui: &Ui, fallback: f32, after: f32) -> f32 {
    panel_footer_bottom(ui).map_or(fallback, |bottom| (bottom - ui.next_widget_position().y - FOOTER_HEIGHT - after).max(0.0))
}

pub fn panel_footer_space(ui: &mut Ui, height: f32) {
    if let Some(bottom) = panel_footer_bottom(ui) {
        ui.add_space((bottom - ui.next_widget_position().y - height).max(0.0));
    }
}

/// Pin a custom footer bar at the bottom edge of a sized dock/panel.
pub fn panel_footer(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui)) {
    let Some(bottom) = panel_footer_bottom(ui) else {
        add(ui);
        return;
    };
    let top = bottom - height;
    if ui.next_widget_position().y > top {
        ui.add_space(height);
    }
    let rect = Rect::from_min_size(pos2(ui.max_rect().left(), top), vec2(ui.available_width(), height));
    ui.painter().rect_filled(rect, 0.0, Tokens::get(ui.ctx()).panel_bg);
    let _background = ui.interact(rect, ui.scope_id().with("panel-footer-background"), Sense::CLICK);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), add);
    ui.advance_cursor_after_rect(rect);
}

/// Bordered container frame for list views.
pub fn list_box<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.stroke(Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(1)).show(ui, add).inner
}

/// Bottom button bar separated by a divider, blocking click-through onto scrolled content behind it.
pub fn bottom_bar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = Tokens::get(ui.ctx());
    if let Some(bottom) = panel_footer_bottom(ui) {
        let top = bottom - FOOTER_HEIGHT;
        if ui.next_widget_position().y > top {
            ui.add_space(FOOTER_HEIGHT);
        }
        let footer = Rect::from_min_size(pos2(ui.max_rect().left(), top), vec2(ui.available_width(), FOOTER_HEIGHT));
        ui.painter().rect_filled(footer, 0.0, t.panel_bg);
        let _background = ui.interact(footer, ui.scope_id().with("panel-footer-background"), Sense::CLICK);
        ui.painter().hline(footer.x_range(), top + 4.0, Stroke::new(1.0, t.divider));
        let row = Rect::from_min_max(pos2(footer.left(), top + 7.0), footer.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            add(ui);
        });
        ui.advance_cursor_after_rect(footer);
        return;
    }
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.divider);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        add(ui);
    });
}

/// Underline tab strip returning the clicked tab index if changed.
pub fn tab_bar(ui: &mut Ui, tabs: &[&str], current: usize) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (i, tab) in tabs.iter().enumerate() {
            let sel = i == current;
            let label = tr_text(ui, tab).into_owned();
            let galley = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(13.0), t.text_primary);
            let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x, 30.0), Sense::click());
            let color = if sel || resp.hovered() { t.text_strong } else { t.text_muted };
            ui.painter().galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0 - 2.0), galley, color);
            if sel {
                ui.painter().rect_filled(
                    Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.right_bottom()),
                    CornerRadius::same(1),
                    t.accent,
                );
            }
            let ui_enabled = ui.is_enabled();
            resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Tab, ui_enabled, sel, label.clone()));
            if resp.clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// Vector page-orientation toggle button (`portrait` / `landscape`); returns `true` when clicked.
pub fn orientation_button(ui: &mut Ui, landscape: bool, selected: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, tip).into_owned();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    paint_hover_row(ui, rect, CornerRadius::same(t.metrics.radius_md), resp.hovered(), selected, &t);
    let size = if landscape { vec2(16.0, 12.0) } else { vec2(12.0, 16.0) };
    let page = Rect::from_center_size(rect.center(), size);
    let color = if selected { t.accent } else { t.icon };
    let fold = 4.0;
    let outline =
        vec![page.left_top(), page.right_top() - vec2(fold, 0.0), page.right_top() + vec2(0.0, fold), page.right_bottom(), page.left_bottom()];
    let fill = if selected { t.accent } else { Color32::TRANSPARENT };
    ui.painter().add(egui::Shape::convex_polygon(outline, fill, Stroke::new(1.2, color)));
    let corner = [page.right_top() - vec2(fold, 0.0), page.right_top() + vec2(-fold, fold), page.right_top() + vec2(0.0, fold)];
    ui.painter().add(egui::Shape::line(corner.to_vec(), Stroke::new(1.2, if selected { t.panel_bg } else { color })));
    let accessible = label.clone();
    let ui_enabled = ui.is_enabled();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Button, ui_enabled, selected, accessible.clone()));
    resp.on_hover_text(label).clicked()
}

/// Grid arrangement preview button drawing mini panel tiles and flow order.
pub fn layout_button(ui: &mut Ui, tiles: &[(u8, u8)], selected: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let enabled = ui.is_enabled();
    paint_interactive_chrome(
        ui,
        rect,
        &t,
        InteractiveChromeSpec {
            radius: CornerRadius::same(t.metrics.radius_md),
            base_bg: Color32::TRANSPARENT,
            base_border: None,
            hovered: resp.hovered(),
            selected,
            pressed: resp.is_pointer_button_down_on(),
            enabled,
        },
    );
    let color = if !enabled {
        t.text_disabled
    } else if selected {
        t.accent
    } else {
        t.icon
    };
    let (cols, rows) = tiles.iter().fold((1u8, 1u8), |(c, r), &(x, y)| (c.max(x.saturating_add(1)), r.max(y.saturating_add(1))));
    let cell = vec2((20.0 / f32::from(cols)).min(10.0), (20.0 / f32::from(rows)).min(10.0));
    let origin = rect.center() - vec2(f32::from(cols) * cell.x, f32::from(rows) * cell.y) / 2.0;
    let centre = |&(x, y): &(u8, u8)| origin + vec2((f32::from(x) + 0.5) * cell.x, (f32::from(y) + 0.5) * cell.y);
    for tile in tiles {
        ui.painter().rect_filled(Rect::from_center_size(centre(tile), cell - vec2(2.0, 2.0)), 1.0, color.gamma_multiply(0.35));
    }
    let path: Vec<Pos2> = tiles.iter().map(centre).collect();
    if let ([first, ..], [.., from, to]) = (path.as_slice(), path.as_slice()) {
        let (first, from, to) = (*first, *from, *to);
        let dir = (to - from).normalized();
        let side = dir.rot90() * 2.5;
        let head = vec![to + dir * 1.5, to - dir * 2.5 + side, to - dir * 2.5 - side];
        ui.painter().circle_filled(first, 1.6, color);
        ui.painter().add(egui::Shape::line(path, Stroke::new(1.2, color)));
        ui.painter().add(egui::Shape::convex_polygon(head, color, Stroke::NONE));
    }
    let accessible = tip.to_owned();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, accessible.clone()));
    resp.on_hover_text(tip).clicked()
}

pub fn alt_down_in_field(ui: &Ui, field: Rect) -> Option<egui::Id> {
    let focused = ui.memory(|m| m.focused())?;
    let in_field = ui.ctx().read_response(focused).is_some_and(|r| field.expand(1.0).contains_rect(r.rect));
    let alt_down = |e: &egui::Event| {
        matches!(
            e,
            egui::Event::Key {
                key: egui::Key::ArrowDown,
                pressed: true,
                modifiers,
                ..
            } if modifiers.matches_exact(egui::Modifiers::ALT)
        )
    };
    let pressed = in_field
        && ui.is_enabled()
        && ui.input_mut(|i| {
            let before = i.events.len();
            i.events.retain(|e| !alt_down(e));
            i.events.len() != before
        });
    pressed.then_some(focused)
}

pub fn open_presets_from_keys(ui: &Ui, chevron: &Response, field: Option<egui::Id>) -> bool {
    let Some(field) = field else { return false };
    let popup = egui::Popup::default_response_id(chevron);
    egui::Popup::open_id(ui.ctx(), popup);
    ui.data_mut(|d| d.insert_temp(popup.with("return focus"), field));
    true
}

pub fn presets_open(ui: &Ui, chevron: &Response) -> bool {
    egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(chevron))
}

pub fn keep_enter_in_presets(ui: &Ui, was_open: bool) {
    if was_open {
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    }
}

pub fn return_focus_from_presets(ui: &Ui, chevron: &Response) {
    let popup = egui::Popup::default_response_id(chevron);
    let key = popup.with("return focus");
    let Some(field) = ui.data(|d| d.get_temp::<egui::Id>(key)) else {
        return;
    };
    if egui::Popup::is_id_open(ui.ctx(), popup) {
        return;
    }
    ui.data_mut(|d| d.remove::<egui::Id>(key));
    let elsewhere = ui.memory(|m| m.focused()).and_then(|id| ui.ctx().read_response(id)).is_some_and(|r| r.layer_id.id != popup);
    if !elsewhere {
        ui.memory_mut(|m| m.request_focus(field));
    }
}

pub fn entry_to_focus(opened_from_keys: bool, chosen: impl IntoIterator<Item = bool>) -> Option<usize> {
    opened_from_keys.then(|| chosen.into_iter().position(|c| c).unwrap_or(0))
}

/// Trailing preset dropdown chevron attached to the right edge of a field.
pub fn chevron_cell(ui: &mut Ui, h: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    let label = tr_text(ui, "Presets").into_owned();
    let (dr, dresp) = ui.allocate_exact_size(vec2(20.0, h), Sense::CLICK);
    ui.painter().rect_filled(dr, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, t.input_bg);
    ui.painter().rect_stroke(dr, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
    let c = if !ui.is_enabled() {
        t.text_disabled
    } else if dresp.hovered() {
        t.text_strong
    } else {
        t.icon
    };
    paint_chevron(ui, Rect::from_center_size(dr.center(), Vec2::splat(12.0)), true, c);
    let ui_enabled = ui.is_enabled();
    dresp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, ui_enabled, label.clone()));
    dresp
}

// ============================================================================
// Wheel Stream Ownership
// ============================================================================

/// Context data key recording the timestamp of wheel events consumed from raw input.
pub fn taken_from_raw_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("wheel-taken-from-raw").with(viewport)
}

/// Field currently armed to capture a wheel scroll stream.
#[derive(Clone, Copy)]
pub struct ArmedWheelField {
    pub id: egui::Id,
    pub rect: Rect,
    pub pass: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelStreamOwner {
    Field(egui::Id),
    Elsewhere,
}

/// Continuous wheel input gesture tracking ownership between fields and scroll containers.
#[derive(Clone, Copy, Default)]
pub struct WheelStream {
    last: Option<f64>,
    pub owner: Option<WheelStreamOwner>,
}

impl WheelStream {
    pub fn note_input(&mut self, now: f64) {
        if !self.last.is_some_and(|last| (0.0..WHEEL_STREAM_GAP).contains(&(now - last))) {
            self.owner = None;
        }
        self.last = Some(now);
    }

    pub fn note_taken_elsewhere(&mut self, at: f64) {
        if self.last.is_none_or(|last| last <= at) {
            self.note_input(at);
        }
        self.owner = Some(WheelStreamOwner::Elsewhere);
    }

    pub fn release(&mut self, field: egui::Id) {
        if self.owner == Some(WheelStreamOwner::Field(field)) {
            self.owner = Some(WheelStreamOwner::Elsewhere);
        }
    }

    pub fn release_any(&mut self) {
        if let Some(WheelStreamOwner::Field(field)) = self.owner {
            self.release(field);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PANEL_FOOTER, Tokens, WheelStream, WheelStreamOwner, bottom_bar, panel_footer};

    fn test_font_definitions() -> egui::FontDefinitions {
        let mut defs = egui::FontDefinitions::empty();
        let minimal = curvebook_text::build_minimal_truetype_font();
        let key = String::from("minimal-fallback");
        defs.font_data.insert(key.clone(), std::sync::Arc::new(egui::FontData::from_owned(minimal)));
        defs.families.insert(egui::FontFamily::Proportional, vec![key.clone()]);
        defs.families.insert(egui::FontFamily::Monospace, vec![key]);
        defs
    }

    #[test]
    fn blank_footer_space_does_not_click_a_row_scrolled_behind_it() {
        use egui::{Event, PointerButton, Rect, Sense, UiBuilder, UiStackInfo, pos2};
        for custom in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_fonts(test_font_definitions());
            Tokens::default().apply_to_ctx(&ctx);
            let mut clicked = false;
            let at = pos2(150.0, 190.0);
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            for events in [vec![], vec![], vec![Event::PointerMoved(at), button(true)], vec![button(false)]] {
                let input =
                    egui::RawInput { screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 200.0))), events, ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| {
                    let info = UiStackInfo::default().with_tag_value(PANEL_FOOTER, Some(200.0_f32));
                    ui.scope_builder(UiBuilder::new().ui_stack_info(info), |ui| {
                        clicked |= ui.allocate_rect(Rect::from_min_max(pos2(0.0, 100.0), pos2(200.0, 200.0)), Sense::click()).clicked();
                        let contents = |ui: &mut egui::Ui| {
                            let _button = ui.button("Footer button");
                        };
                        if custom {
                            panel_footer(ui, 31.0, contents);
                        } else {
                            bottom_bar(ui, contents);
                        }
                    });
                });
                output.textures_delta.clear();
            }
            assert!(!clicked, "custom={custom}: a hidden row received the footer click");
        }
    }

    #[test]
    fn wheel_stream_resets_owner_after_gap() {
        let mut stream = WheelStream::default();
        let field = egui::Id::unique("field");
        stream.note_input(1.0);
        stream.owner = Some(WheelStreamOwner::Field(field));
        stream.note_input(1.2);
        assert_eq!(stream.owner, Some(WheelStreamOwner::Field(field)));
        stream.release(field);
        assert_eq!(stream.owner, Some(WheelStreamOwner::Elsewhere));
        stream.owner = Some(WheelStreamOwner::Field(field));
        stream.release_any();
        assert_eq!(stream.owner, Some(WheelStreamOwner::Elsewhere));
        stream.note_taken_elsewhere(1.3);
        assert_eq!(stream.owner, Some(WheelStreamOwner::Elsewhere));
        stream.note_input(2.5);
        assert_eq!(stream.owner, None);
    }

    #[test]
    fn pure_egui_widget_primitives_render_without_panics() {
        let ctx = egui::Context::default();
        ctx.set_fonts(test_font_definitions());
        Tokens::default().apply_to_ctx(&ctx);
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 420.0))), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            let _ = super::check(ui, "menu.file.save", true, true);
            let _ = super::check3_tip(ui, "Mixed", None, true, "Tooltip");
            let _ = super::radio(ui, "Option A", true, true);
            let _ = super::icon_button(ui, "search", "Search", false, 28.0);
            let _ = super::flat_button_fit(ui, "Analyze", 72.0);
            let _ = super::split_button_named(ui, "Export", "More formats", 96.0);
            let _ = super::primary_button_enabled(ui, "Run", true);
            let _ = super::secondary_button(ui, "Cancel");
            let _ = super::section_toggle(ui, "Details", true);
            super::divider(ui);
            let _ = super::dim_label(ui, "Label");
            let _ = super::take_previous_label(ui);
            super::label_row(ui, "Alpha", 64.0, |ui| {
                let _ = ui.button("0.05");
            });
            let _ = super::link_label(ui, "Options");
            let _ = super::tab_bar(ui, &["Data", "Options"], 0);
            let _ = super::orientation_button(ui, true, true, "Landscape");
            let _ = super::layout_button(ui, &[(0, 0), (1, 0)], true, "2x1");
            let _ = super::chevron_cell(ui, 24.0);
        });
        output.textures_delta.clear();
    }
}
