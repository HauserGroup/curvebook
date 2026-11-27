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
use crate::egui_shell::{ActionRank, FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, Tokens};
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
        let tokens = Tokens::get(ui.ctx());
        let _ = show_tooltip_for_control(&resp, &tokens, text, elided, TooltipSpec::new(text));
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
    let tokens = Tokens::get(ui.ctx());
    let resp = show_resolved_tooltip(ui, &tokens, resp, enabled, None, ellipsized, spec.text, spec.tooltip, TooltipMode::Standard);
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

/// Tooltip presentation mode for [`styled_button`], [`choice_row`], and [`link_button`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum TooltipMode {
    /// Standard `resp.on_hover_text` (or `on_disabled_hover_text` when the parent `Ui` is disabled).
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
    styled_button_with_tooltip(ui, tokens, meta, spec, None)
}

/// Render a token-styled button/pill/chip with an optional explicit structured [`TooltipSpec`].
pub(super) fn styled_button_with_tooltip(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    spec: StyledButtonSpec<'_>,
    tooltip_override: Option<TooltipSpec<'_>>,
) -> (Option<Response>, bool) {
    let fg = if !spec.enabled {
        tokens.text_disabled
    } else if spec.primary {
        tokens.on_accent
    } else if spec.selected {
        if spec.accent_text_when_selected { tokens.selected_accent_text() } else { tokens.text_strong }
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
            let max_w = max_w.max(12.0);
            let (probe, _) =
                elided_galley(ui.painter(), spec.label, FontId::proportional(spec.font_size), fg, (max_w - pad_x).max(12.0), spec.break_anywhere);
            let computed = (probe.size().x + pad_x).clamp(min_w.min(max_w), max_w);
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
        let (rect, resp) = ui.allocate_exact_size(size, if spec.enabled { Sense::click() } else { Sense::hover() });
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

    resp = if let Some(spec_override) = tooltip_override {
        show_tooltip_for_control_with_mode(&resp, tokens, spec.label, ellipsized, spec_override, spec.tooltip_mode)
    } else {
        show_resolved_tooltip(ui, tokens, resp, spec.enabled, spec.disabled_reason, ellipsized, spec.label, spec.tooltip, spec.tooltip_mode)
    };

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
        tokens.accent
    } else if hovered {
        tokens.hover_border
    } else {
        tokens.button_border
    };
    (bg, border)
}

/// Canonical 3-band structured hover tooltip specification (`R20.21`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TooltipSpec<'a> {
    /// Primary strong title (Band 1 left).
    pub title: &'a str,
    /// Optional platform-formatted shortcut keycap pill (`11.5 pt`, Band 1 right).
    pub shortcut: Option<&'a str>,
    /// Optional concise single-line muted body explanation or precision readout (Band 2).
    pub summary: Option<&'a str>,
    /// Optional actionable disabled reason footer shown when a control is disabled (Band 3).
    pub disabled_reason: Option<&'a str>,
}

impl<'a> TooltipSpec<'a> {
    #[must_use]
    pub(crate) const fn new(title: &'a str) -> Self {
        Self { title, shortcut: None, summary: None, disabled_reason: None }
    }

    #[must_use]
    pub(crate) const fn with_shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    #[must_use]
    pub(crate) const fn with_summary(mut self, summary: &'a str) -> Self {
        self.summary = Some(summary);
        self
    }

    #[must_use]
    pub(crate) const fn with_disabled_reason(mut self, disabled_reason: &'a str) -> Self {
        self.disabled_reason = Some(disabled_reason);
        self
    }
}

#[derive(Clone, Copy)]
struct CleanedTooltip<'a> {
    title: &'a str,
    shortcut: Option<&'a str>,
    summary: Option<&'a str>,
    disabled_reason: Option<&'a str>,
}

fn clean_opt(opt: Option<&str>) -> Option<&str> {
    opt.map(str::trim).filter(|s| !s.is_empty())
}

fn clean_tooltip_spec(spec: TooltipSpec<'_>) -> Option<CleanedTooltip<'_>> {
    let title = spec.title.trim();
    let shortcut = clean_opt(spec.shortcut);
    let mut summary = clean_opt(spec.summary);
    let disabled_reason = clean_opt(spec.disabled_reason);
    if let Some(body) = summary
        && !title.is_empty()
        && body.eq_ignore_ascii_case(title)
    {
        summary = None;
    }
    if title.is_empty() && summary.is_none() && disabled_reason.is_none() {
        return None;
    }
    Some(CleanedTooltip { title, shortcut, summary, disabled_reason })
}

fn is_self_explanatory_label(label: &str) -> bool {
    let t = label.trim();
    matches!(t, "OK" | "Ok" | "Cancel" | "Close" | "Apply" | "Done" | "Yes" | "No" | "Dismiss")
}

/// Returns `true` when `spec` merely echoes an unclipped self-explanatory `control_label`
/// without adding a shortcut, non-echo summary, or disabled reason (`R20.21` no-echo invariant).
#[must_use]
pub(crate) fn is_echo_tooltip(control_label: &str, ellipsized: bool, spec: &TooltipSpec<'_>) -> bool {
    if ellipsized {
        return false;
    }
    let Some(cleaned) = clean_tooltip_spec(*spec) else {
        return true;
    };
    if cleaned.shortcut.is_some() || cleaned.disabled_reason.is_some() {
        return false;
    }
    let ctrl_trimmed = control_label.trim();
    if let Some(body) = cleaned.summary {
        return body.eq_ignore_ascii_case(ctrl_trimmed) || (is_self_explanatory_label(ctrl_trimmed) && body.eq_ignore_ascii_case(ctrl_trimmed));
    }
    cleaned.title.is_empty() || cleaned.title.eq_ignore_ascii_case(ctrl_trimmed) || is_self_explanatory_label(ctrl_trimmed)
}

/// Dismiss hover tooltips immediately when pointer buttons are held/pressed, scrolling occurs, or dragging starts.
#[must_use]
pub(crate) fn should_dismiss_tooltip(ctx: &Context, resp: &Response) -> bool {
    resp.dragged()
        || ctx.input(|i| {
            i.pointer.any_down()
                || i.pointer.any_pressed()
                || i.smooth_scroll_delta.length_sq() > 0.25
                || i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. }))
                || i.pointer.is_decidedly_dragging()
        })
}

fn paint_tooltip_shortcut_pill(ui: &mut Ui, tokens: &Tokens, shortcut: &str) {
    let m = tokens.metrics;
    let pad_x = m.space_xs as i8 + 1;
    let pad_y = m.space_xxs as i8;
    egui::Frame::NONE
        .fill(tokens.surface_recessed)
        .stroke(Stroke::new(m.stroke_thin_pt, tokens.border))
        .corner_radius(CornerRadius::same(m.radius_sm))
        .inner_margin(egui::Margin::symmetric(pad_x, pad_y))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(shortcut).size(super::fonts::SHORTCUT_PILL_FONT_PT).color(tokens.text_primary));
        });
}

fn render_tooltip_card(ui: &mut Ui, tokens: &Tokens, cleaned: CleanedTooltip<'_>) {
    let m = tokens.metrics;
    let max_w = (ui.style().spacing.tooltip_width).clamp(300.0, 320.0);
    ui.set_max_width(max_w);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = m.space_xs;
        if !cleaned.title.is_empty() || cleaned.shortcut.is_some() {
            if let Some(sc) = cleaned.shortcut {
                let sc_font = FontId::proportional(super::fonts::SHORTCUT_PILL_FONT_PT);
                let sc_galley = ui.painter().layout_no_wrap(sc.to_owned(), sc_font, tokens.text_primary);
                let pill_w = sc_galley.size().x + m.space_sm * 2.0 + m.stroke_thin_pt * 2.0;
                let title_cap = (max_w - pill_w - m.space_md).max(m.min_hit_target_pt * 3.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = m.space_md;
                    if !cleaned.title.is_empty() {
                        ui.scope(|ui| {
                            ui.set_max_width(title_cap);
                            ui.add(
                                egui::Label::new(egui::RichText::new(cleaned.title).size(m.font_ui_body_pt).strong().color(tokens.text_strong))
                                    .wrap(),
                            );
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        paint_tooltip_shortcut_pill(ui, tokens, sc);
                    });
                });
            } else {
                ui.add(egui::Label::new(egui::RichText::new(cleaned.title).size(m.font_ui_body_pt).strong().color(tokens.text_strong)).wrap());
            }
        }
        if let Some(body) = cleaned.summary {
            ui.add(egui::Label::new(egui::RichText::new(body).size(m.font_ui_small_pt).color(tokens.text_muted)).wrap());
        }
        if let Some(reason) = cleaned.disabled_reason {
            let div_w = ui.available_width().max(m.min_hit_target_pt);
            let (div_rect, _) = ui.allocate_exact_size(Vec2::new(div_w, m.stroke_thin_pt), Sense::hover());
            ui.painter().line_segment([div_rect.left_center(), div_rect.right_center()], Stroke::new(m.stroke_thin_pt, tokens.divider));
            ui.add(egui::Label::new(egui::RichText::new(reason).size(m.font_ui_small_pt).color(tokens.text_muted)).wrap());
        }
    });
}

/// Attach a token-styled 3-band structured hover tooltip onto `resp` (`R20.21`).
///
/// Respects the global `450 ms` `tooltip_delay`, dismisses immediately on click/scroll/drag,
/// strips Band 2 if it merely repeats `title`, and exposes each text band to AccessKit `Role::Tooltip`.
pub(crate) fn show_tooltip(resp: &Response, tokens: &Tokens, spec: TooltipSpec<'_>) -> Response {
    show_tooltip_with_mode(resp, tokens, spec, TooltipMode::Standard)
}

/// Attach `spec` unless it violates the no-echo invariant for `(control_label, ellipsized)`.
pub(crate) fn show_tooltip_for_control(resp: &Response, tokens: &Tokens, control_label: &str, ellipsized: bool, spec: TooltipSpec<'_>) -> Response {
    show_tooltip_for_control_with_mode(resp, tokens, control_label, ellipsized, spec, TooltipMode::Standard)
}

fn show_tooltip_for_control_with_mode(
    resp: &Response,
    tokens: &Tokens,
    control_label: &str,
    ellipsized: bool,
    spec: TooltipSpec<'_>,
    mode: TooltipMode,
) -> Response {
    if is_echo_tooltip(control_label, ellipsized, &spec) {
        return resp.clone();
    }
    show_tooltip_with_mode(resp, tokens, spec, mode)
}

/// Render a token-styled structured floating tooltip card at `anchor` (for Graph Canvas point dwell readouts).
pub(crate) fn show_floating_tooltip(
    ctx: &Context,
    layer_id: egui::LayerId,
    id: egui::Id,
    anchor: egui::PopupAnchor,
    tokens: &Tokens,
    spec: TooltipSpec<'_>,
) {
    let Some(cleaned) = clean_tooltip_spec(spec) else {
        return;
    };
    egui::Tooltip::always_open(ctx.clone(), layer_id, id, anchor).show(|ui| {
        render_tooltip_card(ui, tokens, cleaned);
    });
}

fn show_tooltip_with_mode(resp: &Response, tokens: &Tokens, spec: TooltipSpec<'_>, mode: TooltipMode) -> Response {
    let Some(cleaned) = clean_tooltip_spec(spec) else {
        return resp.clone();
    };
    if should_dismiss_tooltip(&resp.ctx, resp) {
        return resp.clone();
    }
    let in_open_popup_layer = resp.layer_id.order == egui::Order::Foreground && egui::Popup::is_any_open(&resp.ctx);
    let use_always_open = matches!(mode, TooltipMode::ModalAlwaysOpen) || in_open_popup_layer;
    if !use_always_open {
        let r = resp.clone();
        if r.enabled() {
            return r.on_hover_ui(|ui| render_tooltip_card(ui, tokens, cleaned));
        }
        return r.on_disabled_hover_ui(|ui| render_tooltip_card(ui, tokens, cleaned));
    }
    let delay = resp.ctx.global_style().interaction.tooltip_delay;
    let (any_down, is_still, since_move, since_scroll) =
        resp.ctx.input(|i| (i.pointer.any_down(), i.pointer.is_still(), i.pointer.time_since_last_movement(), i.time_since_last_scroll()));
    let elapsed = since_move.min(since_scroll);
    if resp.hovered() && !any_down && is_still {
        if elapsed >= delay {
            let anchor =
                if matches!(mode, TooltipMode::ModalAlwaysOpen) { egui::PopupAnchor::Pointer } else { egui::PopupAnchor::ParentRect(resp.rect) };
            egui::Tooltip::always_open(resp.ctx.clone(), resp.layer_id, resp.id.with("hover_tooltip"), anchor)
                .show(|ui| render_tooltip_card(ui, tokens, cleaned));
        } else {
            resp.ctx.request_repaint_after_secs((delay - elapsed).max(0.0));
        }
    }
    resp.clone()
}

fn resolve_structured_tooltip<'a>(
    enabled: bool,
    disabled_reason: Option<&'a str>,
    ellipsized: bool,
    label: &'a str,
    custom_tip: Option<&'a str>,
) -> Option<TooltipSpec<'a>> {
    if !enabled {
        let reason = disabled_reason.map(str::trim).filter(|s| !s.is_empty())?;
        return Some(TooltipSpec::new(label).with_disabled_reason(reason));
    }
    let summary = custom_tip.map(str::trim).filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case(label.trim()));
    if ellipsized {
        let mut spec = TooltipSpec::new(label);
        if let Some(body) = summary {
            spec = spec.with_summary(body);
        }
        return Some(spec);
    }
    summary.map(|body| TooltipSpec::new(label).with_summary(body))
}

#[allow(clippy::too_many_arguments)]
fn show_resolved_tooltip(
    _ui: &Ui,
    tokens: &Tokens,
    resp: Response,
    enabled: bool,
    disabled_reason: Option<&str>,
    ellipsized: bool,
    label: &str,
    custom_tip: Option<&str>,
    mode: TooltipMode,
) -> Response {
    let Some(spec) = resolve_structured_tooltip(enabled, disabled_reason, ellipsized, label, custom_tip) else {
        return resp;
    };
    show_tooltip_for_control_with_mode(&resp, tokens, label, ellipsized, spec, mode)
}

/// Immediate modal-safe tooltip that resists outer `ScrollArea` hover capture.
#[allow(dead_code)]
pub(super) fn attach_always_open_tooltip(ui: &Ui, resp: Response, tip: &str) -> Response {
    let tokens = Tokens::get(ui.ctx());
    show_tooltip_with_mode(&resp, &tokens, TooltipSpec::new(tip), TooltipMode::ModalAlwaysOpen)
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

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(row_w, spec.row_height), if spec.enabled { Sense::click() } else { Sense::hover() });
    let resp = if spec.enabled { resp } else { resp.on_hover_cursor(egui::CursorIcon::Default) };
    let bx = Rect::from_min_size(Pos2::new(rect.left() + 1.0, rect.center().y - 6.5), Vec2::splat(13.0));
    let border = if !spec.enabled {
        tokens.divider
    } else if spec.checked {
        tokens.accent
    } else if resp.hovered() {
        tokens.text_primary
    } else {
        tokens.input_border
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

    let resp = show_resolved_tooltip(ui, tokens, resp, spec.enabled, spec.disabled_reason, ellipsized, spec.label, None, spec.tooltip_mode);

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

/// Configuration for [`segmented_control`] (`docs/design-language.md` §4).
#[allow(dead_code)]
pub(super) struct SegmentedOption<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub tooltip: Option<&'a str>,
    pub value: Option<Value>,
}

/// Render an equal-width segmented control on a shared `surface_recessed` track with `FrameInspectMeta` recording.
#[allow(dead_code)]
pub(super) fn segmented_control(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    options: &[SegmentedOption<'_>],
    selected: usize,
    enabled: bool,
    z_layer: ZLayer,
) -> Option<usize> {
    if options.is_empty() {
        return None;
    }
    let m = tokens.metrics;
    let count = options.len();
    let total_w = ui.available_width().max(m.min_hit_target_pt * (count as f32) + m.space_xs);
    let track_h = m.min_hit_target_pt + m.space_xs;
    let (track_rect, _) = ui.allocate_exact_size(Vec2::new(total_w, track_h), Sense::hover());
    let track_r = CornerRadius::same(m.radius_md);
    ui.painter().rect_filled(track_rect, track_r, tokens.surface_recessed);
    if matches!(tokens.mode, crate::theme::UiThemeMode::HighContrast) {
        ui.painter().rect_stroke(track_rect, track_r, Stroke::new(m.stroke_thin_pt, tokens.border), StrokeKind::Inside);
    }
    let inner = track_rect.shrink(m.space_xxs);
    let seg_w = (inner.width() / (count as f32)).max(m.min_hit_target_pt);
    let seg_r = CornerRadius::same(m.radius_sm);
    let mut clicked = None;
    for (idx, opt) in options.iter().enumerate() {
        let seg_rect = Rect::from_min_size(Pos2::new(inner.left() + seg_w * (idx as f32), inner.top()), Vec2::new(seg_w, inner.height()));
        let mut resp =
            ui.interact(seg_rect, ui.make_persistent_id(("egui_shell.seg", opt.widget_id)), if enabled { Sense::click() } else { Sense::hover() });
        let is_sel = idx == selected;
        if is_sel {
            ui.painter().rect_filled(seg_rect, seg_r, tokens.surface_bg);
            let border_col = if matches!(tokens.mode, crate::theme::UiThemeMode::HighContrast) { tokens.accent } else { tokens.border };
            ui.painter().rect_stroke(seg_rect, seg_r, Stroke::new(m.stroke_thin_pt, border_col), StrokeKind::Inside);
        } else if enabled && resp.hovered() {
            ui.painter().rect_filled(seg_rect, seg_r, tokens.hover_bg);
        }
        let fg = if !enabled {
            tokens.text_disabled
        } else if is_sel {
            tokens.text_strong
        } else {
            tokens.text_primary
        };
        let (galley, ellipsized) =
            elided_galley(ui.painter(), opt.label, FontId::proportional(m.font_ui_body_pt), fg, (seg_w - m.space_md).max(8.0), false);
        ui.painter().with_clip_rect(seg_rect.shrink(m.space_xxs)).galley(
            Pos2::new(seg_rect.center().x - galley.size().x * 0.5, seg_rect.center().y - galley.size().y * 0.5),
            galley,
            fg,
        );
        let a11y_label = opt.label.to_owned();
        resp.widget_info(move || WidgetInfo::selected(Role::RadioButton, enabled, is_sel, a11y_label.clone()));
        resp = show_resolved_tooltip(ui, tokens, resp, enabled, None, ellipsized, opt.label, opt.tooltip, TooltipMode::Standard);
        record_visible_viewport(
            ui,
            meta,
            &resp,
            FrameInspectEntryInput {
                widget_id: opt.widget_id,
                role: "radio",
                label: opt.label,
                command_id: None,
                value: opt.value.clone().or_else(|| Some(json!({ "selected": is_sel }))),
                enabled,
                disabled_reason: None,
                z_layer,
                ellipsized,
                full_label: ellipsized.then_some(opt.label),
            },
        );
        if enabled && resp.clicked() {
            clicked = Some(idx);
        }
    }
    clicked
}

/// Specification for one tab inside [`underline_tab_bar`].
#[allow(dead_code)]
pub(super) struct UnderlineTabSpec<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub tooltip: Option<&'a str>,
}

/// Render borderless full-word underline panel tabs with a `2 pt` bottom accent indicator (`R20.2` / `R20.4`).
#[allow(dead_code)]
pub(super) fn underline_tab_bar(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    tabs: &[UnderlineTabSpec<'_>],
    current: usize,
    z_layer: ZLayer,
) -> Option<usize> {
    if tabs.is_empty() {
        return None;
    }
    let m = tokens.metrics;
    let mut clicked = None;
    let avail_w = ui.available_width().max(m.min_hit_target_pt * (tabs.len() as f32));
    let tab_h = m.min_toolbar_target_pt;
    let gap = m.space_md;
    let total_gaps = gap * (tabs.len().saturating_sub(1) as f32);
    let measured: Vec<f32> = tabs
        .iter()
        .map(|tab| {
            let g = ui.painter().layout_no_wrap(tab.label.to_owned(), FontId::proportional(m.font_ui_body_pt), tokens.text_primary);
            (g.size().x + m.space_xs).max(m.min_hit_target_pt)
        })
        .collect();
    let sum_w: f32 = measured.iter().sum::<f32>() + total_gaps;
    let compact = sum_w > avail_w;
    let font_pt = if compact { m.font_ui_small_pt } else { m.font_ui_body_pt };
    let used_gap = if compact { m.space_xs } else { gap };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = used_gap;
        for (idx, tab) in tabs.iter().enumerate() {
            let sel = idx == current;
            let color = if sel { tokens.text_strong } else { tokens.text_muted };
            let g = ui.painter().layout_no_wrap(tab.label.to_owned(), FontId::proportional(font_pt), color);
            let w = (g.size().x + m.space_xs).max(m.min_hit_target_pt);
            let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, tab_h), Sense::click());
            let fg = if sel || resp.hovered() { tokens.text_strong } else { tokens.text_muted };
            if !sel && resp.hovered() {
                ui.painter().rect_filled(rect.shrink(m.space_xxs), CornerRadius::same(m.radius_sm), tokens.hover_bg);
            }
            let galley = ui.painter().layout_no_wrap(tab.label.to_owned(), FontId::proportional(font_pt), fg);
            ui.painter().galley(Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5 - 1.0), galley, fg);
            if sel {
                ui.painter().rect_filled(
                    Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - m.stroke_focus_pt), rect.right_bottom()),
                    CornerRadius::same(1),
                    tokens.accent,
                );
            }
            let a11y_label = tab.label.to_owned();
            resp.widget_info(move || WidgetInfo::selected(Role::Tab, true, sel, a11y_label.clone()));
            if let Some(tip) = tab.tooltip {
                resp = show_tooltip_for_control(&resp, tokens, tab.label, false, TooltipSpec::new(tab.label).with_summary(tip));
            }
            record_visible_viewport(
                ui,
                meta,
                &resp,
                FrameInspectEntryInput {
                    widget_id: tab.widget_id,
                    role: "tab",
                    label: tab.label,
                    command_id: None,
                    value: Some(json!({ "selected": sel })),
                    enabled: true,
                    disabled_reason: None,
                    z_layer,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if resp.clicked() {
                clicked = Some(idx);
            }
        }
    });
    clicked
}

/// Render a borderless ghost action button with `FrameInspectMeta` recording (`docs/design-language.md` §4).
#[allow(dead_code, clippy::too_many_arguments)]
pub(super) fn ghost_action_button(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    command_id: Option<&str>,
    selected: bool,
    enabled: bool,
    z_layer: ZLayer,
) -> (Response, bool) {
    let m = tokens.metrics;
    let fg = if !enabled {
        tokens.text_disabled
    } else if selected {
        tokens.selected_accent_text()
    } else {
        tokens.text_primary
    };
    let (galley, ellipsized) =
        elided_galley(ui.painter(), label, FontId::proportional(m.font_ui_body_pt), fg, ui.available_width().max(m.min_hit_target_pt), false);
    let w = (galley.size().x + m.space_lg).max(m.min_hit_target_pt);
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, m.min_hit_target_pt), if enabled { Sense::click() } else { Sense::hover() });
    let r = CornerRadius::same(m.radius_md);
    let bg = if !enabled {
        Color32::TRANSPARENT
    } else if resp.is_pointer_button_down_on() || selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, r, bg);
    }
    if enabled && (selected || resp.hovered()) && matches!(tokens.mode, crate::theme::UiThemeMode::HighContrast) {
        let sc = if selected { tokens.accent } else { tokens.hover_border };
        ui.painter().rect_stroke(rect, r, Stroke::new(m.stroke_thin_pt, sc), StrokeKind::Inside);
    }
    ui.painter().galley(Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5), galley, fg);
    let a11y_label = label.to_owned();
    resp.widget_info(move || {
        if selected {
            WidgetInfo::selected(Role::Button, enabled, true, a11y_label.clone())
        } else {
            WidgetInfo::labeled(Role::Button, enabled, a11y_label.clone())
        }
    });
    if ellipsized {
        resp = show_tooltip(&resp, tokens, TooltipSpec::new(label));
    }
    record_visible_viewport_ranked(
        ui,
        meta,
        &resp,
        FrameInspectEntryInput {
            widget_id,
            role: "button",
            label,
            command_id,
            value: selected.then(|| json!({ "selected": true })),
            enabled,
            disabled_reason: None,
            z_layer,
            ellipsized,
            full_label: ellipsized.then_some(label),
        },
        ActionRank::Secondary,
    );
    let clicked = enabled && resp.clicked();
    (resp, clicked)
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

fn record_visible_viewport_ranked(ui: &Ui, meta: &mut FrameInspectMeta, resp: &Response, input: FrameInspectEntryInput<'_>, rank: ActionRank) {
    let clipped = if meta.viewport_rect.is_positive() {
        resp.interact_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect)
    } else {
        resp.interact_rect.intersect(ui.clip_rect())
    };
    if clipped.is_positive() {
        meta.record_ranked_response(ui, resp, input, rank);
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
        text = text.color(tokens.text_strong).strong();
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
        egui::Button::new(spec.label).fill(tokens.active_bg).stroke(Stroke::new(1.0, tokens.accent))
    } else {
        egui::Button::new(spec.label)
    };
    let mut resp = ui.add_enabled(spec.enabled, btn);
    if let Some(reason) = spec.disabled_reason {
        resp = show_tooltip(&resp, tokens, TooltipSpec::new(spec.label).with_disabled_reason(reason));
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

/// Draw a dashed 4 pt / 3 pt rectangular outline around `rect` (used for High Contrast unfocused emphasis).
pub(super) fn paint_dashed_rect_stroke(painter: &egui::Painter, rect: egui::Rect, stroke: Stroke) {
    let r = rect.shrink(stroke.width * 0.5);
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return;
    }
    let dash = 4.0_f32;
    let step = dash + 3.0_f32;
    let edges =
        [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())];
    for (p0, p1) in edges {
        let delta = p1 - p0;
        let len = delta.length();
        if len <= f32::EPSILON {
            continue;
        }
        let dir = delta / len;
        let mut t = 0.0_f32;
        while t < len {
            let t1 = (t + dash).min(len);
            painter.line_segment([p0 + dir * t, p0 + dir * t1], stroke);
            t += step;
        }
    }
}

/// Render a borderless underline tab button of width `width_pt` with active `2.0 pt` accent underline.
#[allow(clippy::too_many_arguments)]
pub(super) fn underline_tab_button(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    width_pt: f32,
    height_pt: f32,
    selected: bool,
    enabled: bool,
    value: Option<serde_json::Value>,
) -> (Response, bool) {
    let desired = egui::vec2(width_pt.max(tokens.metrics.min_hit_target_pt), height_pt.max(tokens.metrics.min_hit_target_pt));
    let (rect, mut resp) = ui.allocate_exact_size(desired, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let hovered = enabled && resp.hovered();
    if ui.is_rect_visible(rect) {
        let bg = if hovered { tokens.hover_bg } else { Color32::TRANSPARENT };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, CornerRadius::same(tokens.metrics.radius_sm), bg);
        }
        let text_color = if !enabled {
            tokens.text_disabled
        } else if selected {
            tokens.text_strong
        } else if hovered {
            tokens.text_primary
        } else {
            tokens.text_muted
        };
        let font_id = if selected && super::fonts::has_bold_font(ui.ctx()) {
            FontId::new(tokens.metrics.font_ui_small_pt, egui::FontFamily::Name(super::fonts::BOLD_FONT_FAMILY.into()))
        } else {
            FontId::proportional(tokens.metrics.font_ui_small_pt)
        };
        let pad_x = tokens.metrics.space_sm.max(8.0);
        let max_text_w = (rect.width() - pad_x * 2.0).max(8.0);
        let (galley, elided) = elided_galley(ui.painter(), label, font_id, text_color, max_text_w, false);
        let text_pos = egui::Pos2::new(rect.left() + pad_x, rect.center().y - galley.size().y * 0.5);
        ui.painter().galley(text_pos, galley, text_color);
        if selected {
            let line_h = 2.0_f32;
            let bar_rect = egui::Rect::from_min_max(
                egui::Pos2::new(rect.left() + pad_x * 0.5, rect.bottom() - line_h),
                egui::Pos2::new(rect.right() - pad_x * 0.5, rect.bottom()),
            );
            ui.painter().rect_filled(bar_rect, CornerRadius::ZERO, tokens.accent);
        }
        if elided {
            resp = show_tooltip(&resp, tokens, TooltipSpec::new(label));
        }
        resp.widget_info(|| WidgetInfo::selected(Role::Tab, enabled, selected, label));
        record_if_visible(
            ui,
            &resp,
            meta,
            FrameInspectEntryInput {
                widget_id,
                role: "tab",
                label,
                command_id: None,
                value: Some(value.unwrap_or_else(|| json!({ "selected": selected }))),
                enabled,
                disabled_reason: (!enabled).then_some("disabled"),
                z_layer: ZLayer::Workspace,
                ellipsized: elided,
                full_label: elided.then_some(label),
            },
        );
    } else {
        resp.widget_info(|| WidgetInfo::selected(Role::Tab, enabled, selected, label));
    }
    let clicked = enabled && resp.clicked();
    (resp, clicked)
}

/// Render a modal tab button with `Role::Tab` AccessKit info and `FrameInspectMeta` recording.
pub(super) fn modal_tab(ui: &mut Ui, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, selected: bool, tokens: &Tokens) -> Response {
    let btn = egui::Button::new(label)
        .fill(if selected { tokens.active_bg } else { tokens.surface_bg })
        .stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { tokens.accent } else { tokens.border }));
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
    record_modal_checkbox_enabled(ui, resp, meta, widget_id, label, checked, true, None);
}

/// Record a modal checkbox response with explicit enablement metadata when visible.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_modal_checkbox_enabled(
    ui: &Ui,
    resp: &Response,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    checked: bool,
    enabled: bool,
    disabled_reason: Option<&str>,
) {
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
            enabled,
            disabled_reason,
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

// ============================================================================
// Canonical Content-Measured Action / Overflow / Context Menu Primitive (R20.16)
// ============================================================================

/// Placement strategy for [`action_menu_popup`].
#[derive(Debug, Clone, Copy)]
pub(crate) enum ActionMenuAnchor {
    /// Trigger-button anchored dropdown or overflow menu: opens below by default, flips above
    /// `anchor.top() - 4.0` when space below is insufficient, and right-aligns to `anchor.right()`
    /// when `anchor.center().x > viewport.center().x`.
    Trigger(Rect),
    /// Trigger-button anchored menu preferring upward placement above `anchor.top() - 4.0`
    /// (e.g. bottom status footer popups), flipping below only when room above is insufficient.
    TriggerAbove(Rect),
    /// Pointer-anchored context menu clamped cleanly inside `viewport_rect.shrink(4.0)`.
    Pointer(Pos2),
}

/// Optional leading icon glyph or check indicator rendered in an [`action_menu_popup`] row.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum ActionMenuIcon<'a> {
    /// Short monochrome glyph painted in the leading icon column.
    Glyph(&'a str),
    /// Checkmark drawn when `true` (always reserves the leading icon column for alignment).
    Check(bool),
}

/// Single interactive (or disabled) item inside [`action_menu_popup`].
pub(crate) struct ActionMenuItem<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub role: &'a str,
    pub a11y_role: Role,
    pub command_id: Option<&'a str>,
    pub value: Option<Value>,
    pub shortcut: Option<&'a str>,
    pub icon: Option<ActionMenuIcon<'a>>,
    pub selected: bool,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
    pub tooltip: Option<&'a str>,
    pub text_color_override: Option<Color32>,
}

impl<'a> ActionMenuItem<'a> {
    /// Construct a standard `"menuitem"` (`Role::MenuItem`) entry.
    #[must_use]
    pub const fn menu_item(widget_id: &'a str, label: &'a str) -> Self {
        Self {
            widget_id,
            label,
            role: "menuitem",
            a11y_role: Role::MenuItem,
            command_id: None,
            value: None,
            shortcut: None,
            icon: None,
            selected: false,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            text_color_override: None,
        }
    }

    /// Construct a `"button"` (`Role::Button`) popup action entry (e.g. Split preset / Toolbar overflow).
    #[must_use]
    pub const fn button_item(widget_id: &'a str, label: &'a str) -> Self {
        Self {
            widget_id,
            label,
            role: "button",
            a11y_role: Role::Button,
            command_id: None,
            value: None,
            shortcut: None,
            icon: None,
            selected: false,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            text_color_override: None,
        }
    }

    #[must_use]
    pub const fn command_id(mut self, command_id: Option<&'a str>) -> Self {
        self.command_id = command_id;
        self
    }

    #[must_use]
    pub fn value(mut self, value: Option<Value>) -> Self {
        self.value = value;
        self
    }

    #[must_use]
    #[allow(dead_code)]
    pub const fn shortcut(mut self, shortcut: Option<&'a str>) -> Self {
        self.shortcut = shortcut;
        self
    }

    #[must_use]
    #[allow(dead_code)]
    pub const fn icon(mut self, icon: Option<ActionMenuIcon<'a>>) -> Self {
        self.icon = icon;
        self
    }

    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub const fn disabled_reason(mut self, disabled_reason: Option<&'a str>) -> Self {
        self.disabled_reason = disabled_reason;
        self
    }

    #[must_use]
    #[allow(dead_code)]
    pub const fn tooltip(mut self, tooltip: Option<&'a str>) -> Self {
        self.tooltip = tooltip;
        self
    }

    #[must_use]
    pub const fn text_color(mut self, color: Option<Color32>) -> Self {
        self.text_color_override = color;
        self
    }
}

/// Entry rendered inside [`action_menu_popup`].
pub(crate) enum ActionMenuEntry<'a> {
    Item(ActionMenuItem<'a>),
    Header(&'a str),
    Separator,
}

/// Configuration for [`action_menu_popup`].
pub(crate) struct ActionMenuSpec<'a> {
    pub area_id: egui::Id,
    pub widget_id: &'a str,
    pub label: &'a str,
    pub anchor: ActionMenuAnchor,
    pub z_layer: ZLayer,
    pub min_w: f32,
    pub max_w: f32,
    pub max_height: Option<f32>,
    pub allow_outside_dismiss: bool,
}

impl<'a> ActionMenuSpec<'a> {
    /// Construct a trigger-anchored `ZLayer::DropdownMenu` specification with canonical sizing bounds.
    #[must_use]
    pub const fn dropdown(area_id: egui::Id, widget_id: &'a str, label: &'a str, anchor: ActionMenuAnchor) -> Self {
        Self {
            area_id,
            widget_id,
            label,
            anchor,
            z_layer: ZLayer::DropdownMenu,
            min_w: 112.0,
            max_w: f32::INFINITY,
            max_height: None,
            allow_outside_dismiss: true,
        }
    }

    /// Construct a pointer-anchored `ZLayer::ContextMenu` specification with canonical sizing bounds.
    #[must_use]
    pub const fn context(area_id: egui::Id, widget_id: &'a str, label: &'a str, pointer: Pos2) -> Self {
        Self {
            area_id,
            widget_id,
            label,
            anchor: ActionMenuAnchor::Pointer(pointer),
            z_layer: ZLayer::ContextMenu,
            min_w: 112.0,
            max_w: f32::INFINITY,
            max_height: None,
            allow_outside_dismiss: true,
        }
    }

    #[must_use]
    pub const fn max_height(mut self, max_height: f32) -> Self {
        self.max_height = Some(max_height);
        self
    }

    #[must_use]
    pub const fn allow_outside_dismiss(mut self, allow: bool) -> Self {
        self.allow_outside_dismiss = allow;
        self
    }
}

/// Rendered result of [`action_menu_popup`].
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct ActionMenuOutput {
    /// Index into `entries` for the enabled [`ActionMenuEntry::Item`] clicked this frame, if any.
    pub clicked_entry: Option<usize>,
    /// `true` when `Escape` or an outside pointer click dismissed the popup this frame.
    pub dismissed: bool,
    /// Outer rectangle of the rendered popup `Area`.
    pub popup_rect: Rect,
}

#[derive(Debug, Clone, Copy)]
struct ActionMenuLayout {
    pos: Pos2,
    outer_w: f32,
    popup_h: f32,
    content_w: f32,
    row_w: f32,
    max_inner_h: f32,
    needs_scroll: bool,
    icon_col_w: f32,
    shortcut_col_w: f32,
    row_h: f32,
    header_h: f32,
    sep_h: f32,
    spacing_y: f32,
    font_size: f32,
    margin_i8: i8,
}

fn measure_action_menu_content(ctx: &Context, tokens: &Tokens, entries: &[ActionMenuEntry<'_>], font_size: f32) -> (f32, f32, f32, f32) {
    let m = tokens.metrics;
    let label_font = FontId::proportional(font_size);
    let small_font = FontId::proportional(m.font_ui_small_pt);
    let row_pad_x = m.space_md * 2.0;
    let has_icon = entries.iter().any(|e| matches!(e, ActionMenuEntry::Item(it) if it.icon.is_some()));
    let icon_col_w = if has_icon { m.space_xl + m.space_xs } else { 0.0 };
    let shortcut_gap = m.space_xl;
    let row_h = m.min_hit_target_pt;
    let header_h = m.space_xl + m.space_xxs;
    let sep_h = m.space_sm + m.stroke_thin_pt;
    let spacing_y = m.space_xxs;

    let (max_row_w, shortcut_col_w) = ctx.fonts_mut(|fonts| {
        let mut max_sc_w = 0.0_f32;
        for entry in entries {
            if let ActionMenuEntry::Item(it) = entry
                && let Some(sc) = it.shortcut
                && !sc.is_empty()
            {
                let w = fonts.layout_no_wrap(sc.to_owned(), small_font.clone(), tokens.text_muted).size().x.ceil();
                max_sc_w = max_sc_w.max(w);
            }
        }
        let sc_col = if max_sc_w > 0.0 { shortcut_gap + max_sc_w } else { 0.0 };
        let mut max_w = 0.0_f32;
        for entry in entries {
            let row_w = match entry {
                ActionMenuEntry::Item(it) => {
                    let lw = fonts.layout_no_wrap(it.label.to_owned(), label_font.clone(), tokens.text_primary).size().x.ceil();
                    row_pad_x + icon_col_w + lw + m.space_xxs + sc_col
                }
                ActionMenuEntry::Header(hdr) => {
                    let hw = fonts.layout_no_wrap((*hdr).to_owned(), small_font.clone(), tokens.text_muted).size().x.ceil();
                    row_pad_x + hw + m.space_xxs
                }
                ActionMenuEntry::Separator => 0.0,
            };
            max_w = max_w.max(row_w);
        }
        (max_w, sc_col)
    });

    let mut inner_h = 0.0_f32;
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            inner_h += spacing_y;
        }
        inner_h += match entry {
            ActionMenuEntry::Item(_) => row_h,
            ActionMenuEntry::Header(_) => header_h,
            ActionMenuEntry::Separator => sep_h,
        };
    }
    (max_row_w, inner_h.max(row_h), icon_col_w, shortcut_col_w)
}

fn solve_action_menu_layout(ctx: &Context, tokens: &Tokens, spec: &ActionMenuSpec<'_>, entries: &[ActionMenuEntry<'_>]) -> ActionMenuLayout {
    let m = tokens.metrics;
    let font_size = (m.font_ui_small_pt + 0.5).max(12.0);
    #[allow(clippy::cast_possible_truncation)]
    let margin_i8 = m.space_sm.round() as i8;
    let frame_pad = (f32::from(margin_i8) + m.stroke_thin_pt) * 2.0;
    let row_h = m.min_hit_target_pt;
    let header_h = m.space_xl + m.space_xxs;
    let sep_h = m.space_sm + m.stroke_thin_pt;
    let spacing_y = m.space_xxs;
    let gap_y = m.space_xs;

    let (max_row_w, natural_inner_h, icon_col_w, shortcut_col_w) = measure_action_menu_content(ctx, tokens, entries, font_size);
    let viewport = ctx.content_rect();
    let safe = viewport.shrink(m.space_xs);

    let min_outer_h = row_h + frame_pad;
    let viewport_max_outer_h = (viewport.height() - m.space_xl).max(min_outer_h);
    let base_max_outer_h = spec.max_height.map_or(viewport_max_outer_h, |mh| mh.min(viewport_max_outer_h)).max(min_outer_h);
    let natural_outer_h = (natural_inner_h + frame_pad).ceil();

    let (place_above, effective_max_outer_h) = match spec.anchor {
        ActionMenuAnchor::Trigger(anchor) => {
            let space_below = (safe.bottom() - (anchor.bottom() + gap_y)).max(0.0);
            let space_above = ((anchor.top() - gap_y) - safe.top()).max(0.0);
            let above = natural_outer_h > space_below && space_above > space_below;
            let side = if above { space_above } else { space_below };
            let cap = if side >= min_outer_h { base_max_outer_h.min(side) } else { base_max_outer_h };
            (above, cap)
        }
        ActionMenuAnchor::TriggerAbove(anchor) => {
            let space_above = ((anchor.top() - gap_y) - safe.top()).max(0.0);
            let space_below = (safe.bottom() - (anchor.bottom() + gap_y)).max(0.0);
            let above = !(natural_outer_h > space_above && space_below > space_above);
            let side = if above { space_above } else { space_below };
            let cap = if side >= min_outer_h { base_max_outer_h.min(side) } else { base_max_outer_h };
            (above, cap)
        }
        ActionMenuAnchor::Pointer(_) => (false, base_max_outer_h),
    };

    let max_inner_h = (effective_max_outer_h - frame_pad).floor().max(row_h);
    let needs_scroll = natural_inner_h > max_inner_h + 0.5;
    let scroll_reserve = if needs_scroll { m.scroll_bar_thickness + m.space_sm } else { 0.0 };

    let viewport_max_outer_w = (viewport.width() - m.space_xl).max(64.0);
    let max_outer_w = viewport_max_outer_w.min(spec.max_w).max(64.0);
    let min_outer_w = spec.min_w.min(max_outer_w);
    let outer_w = (max_row_w + scroll_reserve + frame_pad).ceil().clamp(min_outer_w, max_outer_w);
    let content_w = (outer_w - frame_pad).max(48.0);
    let row_w = if needs_scroll { (content_w - scroll_reserve).max(48.0) } else { content_w };
    let popup_h = (if needs_scroll { max_inner_h } else { natural_inner_h } + frame_pad).ceil();

    let max_x = (safe.right() - outer_w).max(safe.left());
    let max_y = (safe.bottom() - popup_h).max(safe.top());
    let (x, y) = match spec.anchor {
        ActionMenuAnchor::Trigger(anchor) | ActionMenuAnchor::TriggerAbove(anchor) => {
            let raw_x = if anchor.center().x > viewport.center().x { anchor.right() - outer_w } else { anchor.left() };
            let raw_y = if place_above { anchor.top() - gap_y - popup_h } else { anchor.bottom() + gap_y };
            (raw_x.clamp(safe.left(), max_x), raw_y.clamp(safe.top(), max_y))
        }
        ActionMenuAnchor::Pointer(pt) => (pt.x.clamp(safe.left(), max_x), pt.y.clamp(safe.top(), max_y)),
    };

    ActionMenuLayout {
        pos: Pos2::new(x.round(), y.round()),
        outer_w,
        popup_h,
        content_w,
        row_w,
        max_inner_h,
        needs_scroll,
        icon_col_w,
        shortcut_col_w,
        row_h,
        header_h,
        sep_h,
        spacing_y,
        font_size,
        margin_i8,
    }
}

fn paint_action_menu_icon(ui: &Ui, rect: Rect, icon: ActionMenuIcon<'_>, color: Color32, font_size: f32) {
    match icon {
        ActionMenuIcon::Check(true) => {
            let stroke = Stroke::new(1.6, color);
            let p0 = Pos2::new(rect.left() + 3.0, rect.center().y);
            let p1 = Pos2::new(rect.center().x - 1.0, rect.bottom() - 3.5);
            let p2 = Pos2::new(rect.right() - 2.5, rect.top() + 3.0);
            ui.painter().line_segment([p0, p1], stroke);
            ui.painter().line_segment([p1, p2], stroke);
        }
        ActionMenuIcon::Check(false) => {}
        ActionMenuIcon::Glyph(glyph) if !glyph.is_empty() => {
            let g = ui.painter().layout_no_wrap(glyph.to_owned(), FontId::proportional(font_size), color);
            ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).galley(
                Pos2::new(rect.center().x - g.size().x * 0.5, rect.center().y - g.size().y * 0.5),
                g,
                color,
            );
        }
        ActionMenuIcon::Glyph(_) => {}
    }
}

fn render_action_menu_item_row(
    ui: &mut Ui,
    tokens: &Tokens,
    layout: &ActionMenuLayout,
    z_layer: ZLayer,
    item: &ActionMenuItem<'_>,
    meta: &mut Option<&mut FrameInspectMeta>,
) -> bool {
    let m = tokens.metrics;
    let row_size = Vec2::new(layout.row_w, layout.row_h);
    if layout.needs_scroll && !slot_visible_in_clip(ui, None, row_size) {
        ui.allocate_space(row_size);
        return false;
    }

    let sense = if item.enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut resp) = ui.allocate_exact_size(row_size, sense);
    if !item.enabled {
        resp = resp.on_hover_cursor(egui::CursorIcon::Default);
    }

    super::widgets::paint_interactive_chrome(
        ui,
        rect,
        tokens,
        super::widgets::InteractiveChromeSpec {
            radius: CornerRadius::same(m.radius_md),
            base_bg: Color32::TRANSPARENT,
            base_border: None,
            hovered: resp.hovered(),
            selected: item.selected,
            pressed: resp.is_pointer_button_down_on(),
            enabled: item.enabled,
        },
    );

    let fg = if !item.enabled {
        tokens.text_disabled
    } else if let Some(ov) = item.text_color_override {
        ov
    } else if item.selected || resp.hovered() {
        tokens.text_strong
    } else {
        tokens.text_primary
    };

    if layout.icon_col_w > 0.0
        && let Some(icon) = item.icon
    {
        let icon_side = m.space_xl;
        let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + m.space_md, rect.center().y - icon_side * 0.5), Vec2::new(icon_side, icon_side));
        paint_action_menu_icon(ui, icon_rect, icon, fg, layout.font_size);
    }

    if let Some(sc) = item.shortcut
        && !sc.is_empty()
    {
        let sc_color = if item.enabled { tokens.text_muted } else { tokens.text_disabled };
        let sc_galley = ui.painter().layout_no_wrap(sc.to_owned(), FontId::proportional(m.font_ui_small_pt), sc_color);
        let sc_pos = Pos2::new(rect.right() - m.space_md - sc_galley.size().x, rect.center().y - sc_galley.size().y * 0.5);
        ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).galley(sc_pos, sc_galley, sc_color);
    }

    let text_left = rect.left() + m.space_md + layout.icon_col_w;
    let text_right = (rect.right() - m.space_md - layout.shortcut_col_w).max(text_left + m.space_xl);
    let text_max_w = (text_right - text_left).max(m.space_xl);
    let (galley, ellipsized) = elided_galley(ui.painter(), item.label, FontId::proportional(layout.font_size), fg, text_max_w, true);
    let text_clip = Rect::from_min_max(Pos2::new(text_left - 1.0, rect.top()), Pos2::new(text_right + 1.0, rect.bottom())).intersect(ui.clip_rect());
    ui.painter().with_clip_rect(text_clip).galley(Pos2::new(text_left, rect.center().y - galley.size().y * 0.5), galley, fg);

    let ak_label = item.label.to_owned();
    let ak_role = item.a11y_role;
    let ak_enabled = item.enabled;
    resp.widget_info(move || WidgetInfo::labeled(ak_role, ak_enabled, ak_label.clone()));

    resp = show_resolved_tooltip(ui, tokens, resp, item.enabled, item.disabled_reason, ellipsized, item.label, item.tooltip, TooltipMode::Standard);

    if let Some(meta_ref) = meta.as_deref_mut() {
        let vis = if meta_ref.viewport_rect.is_positive() {
            resp.interact_rect.intersect(ui.clip_rect()).intersect(meta_ref.viewport_rect)
        } else {
            resp.interact_rect.intersect(ui.clip_rect())
        };
        if vis.width() >= 1.0 && vis.height() >= 1.0 {
            meta_ref.record_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: item.widget_id,
                    role: item.role,
                    label: item.label,
                    command_id: item.command_id,
                    value: item.value.clone(),
                    enabled: item.enabled,
                    disabled_reason: (!item.enabled).then_some(item.disabled_reason).flatten(),
                    z_layer,
                    ellipsized,
                    full_label: ellipsized.then_some(item.label),
                },
            );
        }
    }

    item.enabled && resp.clicked()
}

fn render_action_menu_entries(
    ui: &mut Ui,
    tokens: &Tokens,
    layout: &ActionMenuLayout,
    z_layer: ZLayer,
    entries: &[ActionMenuEntry<'_>],
    meta: &mut Option<&mut FrameInspectMeta>,
) -> Option<usize> {
    let m = tokens.metrics;
    let mut clicked = None;
    for (idx, entry) in entries.iter().enumerate() {
        match entry {
            ActionMenuEntry::Item(item) => {
                if render_action_menu_item_row(ui, tokens, layout, z_layer, item, meta) {
                    clicked = Some(idx);
                }
            }
            ActionMenuEntry::Header(title) => {
                let sz = Vec2::new(layout.row_w, layout.header_h);
                if layout.needs_scroll && !slot_visible_in_clip(ui, None, sz) {
                    ui.allocate_space(sz);
                } else {
                    let (rect, _) = ui.allocate_exact_size(sz, Sense::hover());
                    let text_max_w = (layout.row_w - m.space_md * 2.0).max(m.space_xl);
                    let (galley, _) =
                        elided_galley(ui.painter(), title, FontId::proportional(m.font_ui_small_pt), tokens.text_muted, text_max_w, true);
                    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).galley(
                        Pos2::new(rect.left() + m.space_md, rect.center().y - galley.size().y * 0.5),
                        galley,
                        tokens.text_muted,
                    );
                }
            }
            ActionMenuEntry::Separator => {
                let sz = Vec2::new(layout.row_w, layout.sep_h);
                if layout.needs_scroll && !slot_visible_in_clip(ui, None, sz) {
                    ui.allocate_space(sz);
                } else {
                    let (rect, _) = ui.allocate_exact_size(sz, Sense::hover());
                    let y = rect.center().y;
                    ui.painter().line_segment(
                        [Pos2::new(rect.left() + m.space_xs, y), Pos2::new(rect.right() - m.space_xs, y)],
                        Stroke::new(m.stroke_thin_pt, tokens.divider),
                    );
                }
            }
        }
    }
    clicked
}

/// Canonical content-measured action, overflow, and context menu popup (`R20.16`).
///
/// Measures every visible item label plus optional icon and right-aligned shortcut columns with
/// `layout_no_wrap` before painting, clamps width once to `[min_w, (viewport_w - 16.0).min(max_w)]`,
/// sets `ui.set_width(content_w)` explicitly for Frame-1-to-Frame-2 stability, right-aligns triggers
/// on the right half of the viewport, flips above `anchor.top() - 4.0` when needed, clamps within
/// `viewport_rect.shrink(4.0)`, scrolls vertically only when taller than `(viewport_h - 16.0)`,
/// handles `Escape` and outside clicks uniformly, and always registers a `role: "menu"` container
/// node in [`FrameInspectMeta`] on `ZLayer::DropdownMenu` or `ZLayer::ContextMenu`.
pub(crate) fn action_menu_popup(
    ctx: &Context,
    tokens: &Tokens,
    mut meta: Option<&mut FrameInspectMeta>,
    spec: ActionMenuSpec<'_>,
    entries: &[ActionMenuEntry<'_>],
) -> ActionMenuOutput {
    let m = tokens.metrics;
    let layout = solve_action_menu_layout(ctx, tokens, &spec, entries);
    let mut clicked_entry = None;

    let area = egui::Area::new(spec.area_id)
        .fixed_pos(layout.pos)
        .default_size(Vec2::new(layout.outer_w, layout.popup_h))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .show(ctx, |area_ui| {
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(m.stroke_thin_pt, tokens.border))
                .corner_radius(CornerRadius::same(m.radius_lg))
                .inner_margin(egui::Margin::same(layout.margin_i8))
                .show(area_ui, |ui| {
                    ui.set_width(layout.content_w);
                    ui.set_max_width(layout.content_w);
                    ui.spacing_mut().item_spacing.y = layout.spacing_y;
                    if layout.needs_scroll {
                        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
                        egui::ScrollArea::vertical()
                            .id_salt(spec.area_id.with("scroll"))
                            .max_height(layout.max_inner_h)
                            .auto_shrink([false, true])
                            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                            .show(ui, |scroll_ui| {
                                scroll_ui.set_width(layout.row_w);
                                scroll_ui.set_max_width(layout.row_w);
                                scroll_ui.spacing_mut().item_spacing.y = layout.spacing_y;
                                clicked_entry = render_action_menu_entries(scroll_ui, tokens, &layout, spec.z_layer, entries, &mut meta);
                            });
                    } else {
                        clicked_entry = render_action_menu_entries(ui, tokens, &layout, spec.z_layer, entries, &mut meta);
                    }
                });
        });

    let popup_rect = area.response.rect;
    let menu_a11y_label = spec.label.to_owned();
    area.response.widget_info(move || WidgetInfo::labeled(Role::Menu, true, menu_a11y_label.clone()));

    if let Some(meta_ref) = meta {
        let clipped_rect = if meta_ref.viewport_rect.is_positive() { meta_ref.viewport_rect.intersect(popup_rect) } else { popup_rect };
        if clipped_rect.is_positive() {
            let value = match spec.anchor {
                ActionMenuAnchor::Trigger(a) | ActionMenuAnchor::TriggerAbove(a) => {
                    Some(json!({ "open": true, "anchor_rect": [a.min.x, a.min.y, a.max.x, a.max.y] }))
                }
                ActionMenuAnchor::Pointer(_) => Some(json!({ "open": true })),
            };
            meta_ref.record(FrameInspectNodeMeta {
                egui_id: area.response.id,
                accesskit_id: area.response.id.accesskit_id(),
                widget_id: spec.widget_id.to_owned(),
                role: String::from("menu"),
                label: spec.label.to_owned(),
                command_id: None,
                value,
                enabled: true,
                disabled_reason: None,
                unclipped_rect: popup_rect,
                clipped_rect,
                z_layer: spec.z_layer,
                ellipsized: false,
                full_label: None,
                action_rank: None,
            });
        }
    }

    let esc_pressed = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let outside_clicked = spec.allow_outside_dismiss
        && match spec.anchor {
            ActionMenuAnchor::Trigger(anchor) | ActionMenuAnchor::TriggerAbove(anchor) => ctx.input(|i| {
                i.pointer.any_released()
                    && i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()).is_some_and(|p| !popup_rect.contains(p) && !anchor.contains(p))
                    && i.pointer.press_origin().is_none_or(|o| !popup_rect.contains(o) && !anchor.contains(o))
            }),
            ActionMenuAnchor::Pointer(_) => ctx.input(|i| {
                i.pointer.primary_clicked() && i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()).is_some_and(|p| !popup_rect.contains(p))
            }),
        };

    ActionMenuOutput { clicked_entry, dismissed: esc_pressed || outside_clicked, popup_rect }
}

/// Configuration for [`link_button`] (canonical borderless clickable text link, `R20.14`).
pub(crate) struct LinkButtonSpec<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub font_size: f32,
    pub command_id: Option<&'a str>,
    pub value: Option<Value>,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
    pub tooltip: Option<TooltipSpec<'a>>,
    pub z_layer: ZLayer,
    /// Minimum allocated width (for uniform-width menu rows such as the search dropdown footer).
    pub min_width: Option<f32>,
    /// Optional maximum width before single-line ellipsis truncation.
    pub max_width: Option<f32>,
    /// Minimum allocated height (`>= 24.0 pt` per WCAG 2.5.8).
    pub min_height: f32,
    /// Horizontal text padding inside the link target (`>= 0.0`).
    pub pad_x: f32,
    /// Whether to center text inside the allocated hit target.
    pub center_text: bool,
    /// Whether to record this control in [`FrameInspectMeta`].
    pub record_in_meta: bool,
    /// Structured role label in [`FrameInspectMeta`] (defaults to `"link"`).
    pub meta_role: &'a str,
    /// Tooltip scheduling mode (`Standard` or `ModalAlwaysOpen`).
    pub tooltip_mode: TooltipMode,
}

impl<'a> LinkButtonSpec<'a> {
    /// Create a canonical clickable link button specification (`widget_id`, `label`, `z_layer`).
    #[must_use]
    pub(crate) const fn new(widget_id: &'a str, label: &'a str, z_layer: ZLayer) -> Self {
        Self {
            widget_id,
            label,
            font_size: 13.0,
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            z_layer,
            min_width: None,
            max_width: None,
            min_height: 24.0,
            pad_x: 4.0,
            center_text: false,
            record_in_meta: true,
            meta_role: "link",
            tooltip_mode: TooltipMode::Standard,
        }
    }
}

/// Render a canonical borderless clickable text link (`R20.14`):
/// blue [`Tokens::text_link`] foreground, `PointingHand` cursor, hover/focus underline,
/// `>= 24 pt` target height, AccessKit `Role::Link`, structured tooltip, and [`FrameInspectMeta`] entry.
pub(crate) fn link_button(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: LinkButtonSpec<'_>) -> Response {
    let fg = if spec.enabled { tokens.text_link } else { tokens.text_disabled };
    let hit_h = spec.min_height.max(tokens.metrics.min_hit_target_pt.max(24.0));
    let avail_w = spec.max_width.map_or_else(|| ui.available_width().max(24.0), |mw| mw.max(24.0));
    let pad_x = spec.pad_x.max(0.0);
    let layout_max_w = (avail_w - pad_x * 2.0).max(12.0);
    let (galley, ellipsized) = elided_galley(ui.painter(), spec.label, FontId::proportional(spec.font_size), fg, layout_max_w, false);

    let natural_w = galley.size().x + pad_x * 2.0;
    let mut alloc_w = natural_w.max(spec.min_width.unwrap_or(0.0));
    if let Some(max_w) = spec.max_width {
        alloc_w = alloc_w.min(max_w.max(spec.min_width.unwrap_or(0.0)));
    }
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(alloc_w.max(1.0), hit_h), if spec.enabled { Sense::click() } else { Sense::hover() });
    resp = if spec.enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp.on_hover_cursor(egui::CursorIcon::Default) };

    let text_x = if spec.center_text { rect.left() + ((rect.width() - galley.size().x) * 0.5).max(pad_x) } else { rect.left() + pad_x };
    let text_y = rect.center().y - galley.size().y * 0.5;
    let text_pos = Pos2::new(text_x, text_y);
    let visible = rect.intersect(ui.clip_rect());
    if visible.is_positive() {
        let painter = ui.painter().with_clip_rect(visible);
        painter.galley(text_pos, galley.clone(), fg);
        if spec.enabled && (resp.hovered() || resp.has_focus()) {
            let underline_y = (text_y + galley.size().y - 0.5).min(rect.bottom() - 1.0);
            painter.line_segment([Pos2::new(text_x, underline_y), Pos2::new(text_x + galley.size().x, underline_y)], Stroke::new(1.0, fg));
        }
        if resp.has_focus() {
            painter.rect_stroke(
                rect.shrink(1.0),
                CornerRadius::same(tokens.metrics.radius_sm),
                Stroke::new(tokens.metrics.stroke_focus_pt, tokens.focus_ring),
                StrokeKind::Inside,
            );
        }
    }

    let a11y_label = spec.label.to_owned();
    let a11y_enabled = spec.enabled;
    resp.widget_info(move || WidgetInfo::labeled(Role::Link, a11y_enabled, a11y_label.clone()));

    resp = if let Some(tip_spec) = spec.tooltip {
        let final_tip = if !spec.enabled {
            if let Some(reason) = spec.disabled_reason.map(str::trim).filter(|s| !s.is_empty()) {
                tip_spec.with_disabled_reason(reason)
            } else {
                tip_spec
            }
        } else {
            tip_spec
        };
        show_tooltip_for_control_with_mode(&resp, tokens, spec.label, ellipsized, final_tip, spec.tooltip_mode)
    } else {
        show_resolved_tooltip(ui, tokens, resp, spec.enabled, spec.disabled_reason, ellipsized, spec.label, None, spec.tooltip_mode)
    };

    if spec.record_in_meta {
        record_visible_viewport(
            ui,
            meta,
            &resp,
            FrameInspectEntryInput {
                widget_id: spec.widget_id,
                role: spec.meta_role,
                label: spec.label,
                command_id: spec.command_id,
                value: spec.value,
                enabled: spec.enabled,
                disabled_reason: (!spec.enabled).then_some(spec.disabled_reason).flatten(),
                z_layer: spec.z_layer,
                ellipsized,
                full_label: ellipsized.then_some(spec.label),
            },
        );
    }

    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::UiThemeMode;

    fn collect_shape_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                egui::Shape::Vec(v) => {
                    for s in v {
                        walk(s, out);
                    }
                }
                _ => {}
            }
        }
        for cs in shapes {
            walk(&cs.shape, out);
        }
    }

    #[test]
    fn da_05_01_disabled_styled_button_and_choice_row_show_standard_tooltip_with_hover_sense() {
        let ctx = egui::Context::default();
        let tokens = Tokens::for_mode(UiThemeMode::Light);
        let mut btn_rect = Rect::NOTHING;
        let mut row_rect = Rect::NOTHING;
        let mut btn_sense = Sense::click();
        let mut btn_clicked = false;
        let mut row_clicked = false;

        let run_frame = |pointer: Option<Pos2>,
                         time: f64,
                         btn_rect: &mut Rect,
                         row_rect: &mut Rect,
                         btn_sense: &mut Sense,
                         btn_clicked: &mut bool,
                         row_clicked: &mut bool| {
            let mut raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
                time: Some(time),
                ..Default::default()
            };
            if let Some(p) = pointer {
                raw.events.push(egui::Event::PointerMoved(p));
            }
            let mut out = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut meta = FrameInspectMeta::new();
                    let (resp, clicked) = styled_button(
                        ui,
                        &tokens,
                        &mut meta,
                        StyledButtonSpec {
                            widget_id: "test.disabled_btn",
                            role: "button",
                            a11y_role: Role::Button,
                            a11y_selectable: false,
                            label: "Run Analysis",
                            tooltip: None,
                            command_id: None,
                            width: ButtonWidth::Fixed(120.0),
                            height: 24.0,
                            font_size: 12.0,
                            radius: 4,
                            text_align: ButtonTextAlign::Center,
                            break_anywhere: false,
                            primary: false,
                            selected: false,
                            enabled: false,
                            accent_text_when_selected: false,
                            border_override: None,
                            value: None,
                            disabled_reason: Some("Frozen datasheet cannot be modified"),
                            z_layer: ZLayer::ModalDialog,
                            skip_offscreen: false,
                            clip_interact_to_viewport: false,
                            tooltip_mode: TooltipMode::Standard,
                        },
                    );
                    let resp = resp.expect("styled_button response");
                    *btn_rect = resp.rect;
                    *btn_sense = resp.sense;
                    *btn_clicked = clicked;

                    *row_clicked = choice_row(
                        ui,
                        &tokens,
                        &mut meta,
                        ChoiceRowSpec {
                            widget_id: "test.disabled_choice",
                            label: "One-sided test",
                            checked: false,
                            is_radio: true,
                            enabled: false,
                            disabled_reason: Some("Requires two-group comparison"),
                            font_size: 12.0,
                            row_height: 22.0,
                            z_layer: ZLayer::ModalDialog,
                            value: json!(false),
                            tooltip_mode: TooltipMode::Standard,
                        },
                    );
                    if let Some(entry) = meta.find_by_widget_id("test.disabled_choice") {
                        *row_rect = entry.clipped_rect;
                    }
                });
            });
            out.textures_delta.clear();
            out
        };

        // Frame 0: lay out widgets.
        let _ = run_frame(None, 0.0, &mut btn_rect, &mut row_rect, &mut btn_sense, &mut btn_clicked, &mut row_clicked);
        assert_eq!(btn_sense, Sense::hover(), "disabled styled_button must use Sense::hover()");
        assert!(!btn_clicked && !row_clicked);

        // Hover disabled styled_button: move pointer, then dwell without moving past tooltip_delay.
        let _ = run_frame(Some(btn_rect.center()), 0.1, &mut btn_rect, &mut row_rect, &mut btn_sense, &mut btn_clicked, &mut row_clicked);
        let _ = run_frame(None, 1.5, &mut btn_rect, &mut row_rect, &mut btn_sense, &mut btn_clicked, &mut row_clicked);
        let out_btn = run_frame(None, 1.55, &mut btn_rect, &mut row_rect, &mut btn_sense, &mut btn_clicked, &mut row_clicked);
        let mut texts_btn = Vec::new();
        collect_shape_texts(&out_btn.shapes, &mut texts_btn);
        assert!(
            texts_btn.iter().any(|t| t.contains("Frozen datasheet cannot be modified")),
            "disabled styled_button must render TooltipMode::Standard tooltip, got {texts_btn:?}"
        );

        // Hover disabled choice_row: move pointer, then dwell without moving past tooltip_delay.
        let _ = run_frame(Some(row_rect.center()), 2.0, &mut btn_rect, &mut row_rect, &mut btn_sense, &mut btn_clicked, &mut row_clicked);
        let _ = run_frame(None, 3.5, &mut btn_rect, &mut row_rect, &mut btn_sense, &mut btn_clicked, &mut row_clicked);
        let out_row = run_frame(None, 3.55, &mut btn_rect, &mut row_rect, &mut btn_sense, &mut btn_clicked, &mut row_clicked);
        let mut texts_row = Vec::new();
        collect_shape_texts(&out_row.shapes, &mut texts_row);
        assert!(
            texts_row.iter().any(|t| t.contains("Requires two-group comparison")),
            "disabled choice_row must render TooltipMode::Standard tooltip, got {texts_row:?}"
        );
    }

    #[test]
    fn da_05_07_fit_text_respects_narrow_max_w_below_min_w() {
        let ctx = egui::Context::default();
        let tokens = Tokens::for_mode(UiThemeMode::Light);
        let mut narrow_w = 0.0_f32;
        let mut tiny_w = 0.0_f32;

        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let mut meta = FrameInspectMeta::new();
                let (r1, _) = styled_button(
                    ui,
                    &tokens,
                    &mut meta,
                    StyledButtonSpec {
                        widget_id: "test.fit_narrow",
                        role: "button",
                        a11y_role: Role::Button,
                        a11y_selectable: false,
                        label: "Very Long Action Button Label",
                        tooltip: None,
                        command_id: None,
                        width: ButtonWidth::FitText { min_w: 96.0, max_w: 44.0, pad_x: 16.0 },
                        height: 24.0,
                        font_size: 12.0,
                        radius: 4,
                        text_align: ButtonTextAlign::Center,
                        break_anywhere: false,
                        primary: false,
                        selected: false,
                        enabled: true,
                        accent_text_when_selected: false,
                        border_override: None,
                        value: None,
                        disabled_reason: None,
                        z_layer: ZLayer::ModalDialog,
                        skip_offscreen: false,
                        clip_interact_to_viewport: false,
                        tooltip_mode: TooltipMode::Standard,
                    },
                );
                narrow_w = r1.expect("r1").rect.width();

                let (r2, _) = styled_button(
                    ui,
                    &tokens,
                    &mut meta,
                    StyledButtonSpec {
                        widget_id: "test.fit_tiny",
                        role: "button",
                        a11y_role: Role::Button,
                        a11y_selectable: false,
                        label: "OK",
                        tooltip: None,
                        command_id: None,
                        width: ButtonWidth::FitText { min_w: 66.0, max_w: 4.0, pad_x: 16.0 },
                        height: 24.0,
                        font_size: 12.0,
                        radius: 4,
                        text_align: ButtonTextAlign::Center,
                        break_anywhere: false,
                        primary: false,
                        selected: false,
                        enabled: true,
                        accent_text_when_selected: false,
                        border_override: None,
                        value: None,
                        disabled_reason: None,
                        z_layer: ZLayer::ModalDialog,
                        skip_offscreen: false,
                        clip_interact_to_viewport: false,
                        tooltip_mode: TooltipMode::Standard,
                    },
                );
                tiny_w = r2.expect("r2").rect.width();
            });
        });
        out.textures_delta.clear();

        assert!(narrow_w <= 44.0 + 1e-3, "FitText must clamp to max_w (44.0) even when min_w (96.0) is larger; got {narrow_w}");
        assert!((tiny_w - 12.0).abs() <= 1e-3, "FitText with max_w < 12.0 must guard to 12.0 without panicking; got {tiny_w}");
    }

    #[test]
    fn r20_16_action_menu_popup_measures_right_aligns_flips_scrolls_and_is_two_frame_stable() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let tokens = Tokens::for_mode(UiThemeMode::Light);
        let vp = Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 420.0));

        // 1. Right-half trigger right-aligns, measures icon + shortcut columns, and is 2-frame stable.
        let right_trigger = Rect::from_min_size(Pos2::new(560.0, 12.0), Vec2::new(64.0, 24.0));
        let mut rects = Vec::new();
        let mut has_menu_role = false;
        let mut item_widths = Vec::new();
        for frame in 0..2 {
            let raw = egui::RawInput { screen_rect: Some(vp), time: Some(f64::from(frame) * 0.016), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let mut meta = FrameInspectMeta::new();
                meta.begin_pass(ui.ctx());
                let entries = [
                    ActionMenuEntry::Item(
                        ActionMenuItem::button_item("test.split.a", "Single Pane (Off)")
                            .icon(Some(ActionMenuIcon::Check(true)))
                            .shortcut(Some("Cmd+\\"))
                            .selected(true),
                    ),
                    ActionMenuEntry::Separator,
                    ActionMenuEntry::Item(ActionMenuItem::button_item("test.split.b", "Data + Graph").icon(Some(ActionMenuIcon::Check(false)))),
                ];
                let res = action_menu_popup(
                    ui.ctx(),
                    &tokens,
                    Some(&mut meta),
                    ActionMenuSpec::dropdown(
                        egui::Id::unique("test_right_menu"),
                        "popup.test.right",
                        "Test Right Menu",
                        ActionMenuAnchor::Trigger(right_trigger),
                    ),
                    &entries,
                );
                rects.push(res.popup_rect);
                has_menu_role = meta.find_by_widget_id("popup.test.right").is_some_and(|e| e.role == "menu" && e.z_layer == ZLayer::DropdownMenu);
                item_widths = meta.entries.iter().filter(|e| e.role == "button").map(|e| e.unclipped_rect.width()).collect();
            });
            out.textures_delta.clear();
        }
        assert!(has_menu_role, "action_menu_popup must always register a role=\"menu\" container node");
        assert_eq!(rects.len(), 2);
        let (r1, r2) = (rects[0], rects[1]);
        assert!((r1.min.x - r2.min.x).abs() <= 0.5 && (r1.max.x - r2.max.x).abs() <= 0.5, "width drifted: {r1:?} vs {r2:?}");
        assert!((r1.min.y - r2.min.y).abs() <= 0.5 && (r1.max.y - r2.max.y).abs() <= 0.5, "height drifted: {r1:?} vs {r2:?}");
        assert!(r1.right() <= right_trigger.right() + 2.0 && r1.left() < right_trigger.left(), "right-half trigger must right-align: {r1:?}");
        assert!((r1.top() - right_trigger.bottom() - 4.0).abs() <= 1.5, "must open 4pt below trigger: {r1:?}");
        assert_eq!(item_widths.len(), 2);
        assert!((item_widths[0] - item_widths[1]).abs() <= 0.5, "rows must share equal width: {item_widths:?}");

        // 2. Bottom-edge trigger flips above anchor.top() - 4.0.
        let bottom_trigger = Rect::from_min_size(Pos2::new(20.0, 390.0), Vec2::new(90.0, 24.0));
        let mut flipped_rect = Rect::NOTHING;
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(vp), ..Default::default() }, |ui| {
            let mut meta = FrameInspectMeta::new();
            meta.begin_pass(ui.ctx());
            let entries = [
                ActionMenuEntry::Item(ActionMenuItem::menu_item("test.b.0", "50%")),
                ActionMenuEntry::Item(ActionMenuItem::menu_item("test.b.1", "100%")),
                ActionMenuEntry::Item(ActionMenuItem::menu_item("test.b.2", "Fit")),
            ];
            let res = action_menu_popup(
                ui.ctx(),
                &tokens,
                Some(&mut meta),
                ActionMenuSpec::dropdown(egui::Id::unique("test_flip_menu"), "popup.test.flip", "Zoom", ActionMenuAnchor::Trigger(bottom_trigger)),
                &entries,
            );
            flipped_rect = res.popup_rect;
        });
        out.textures_delta.clear();
        assert!(flipped_rect.bottom() <= bottom_trigger.top() - 3.0, "near bottom edge must flip above trigger: {flipped_rect:?}");
        assert!(flipped_rect.width() <= 180.0, "short menu must remain compact: {flipped_rect:?}");

        // 3. Tall context menu enables scroll and stays clamped within viewport_rect.shrink(4.0).
        let labels: Vec<String> = (0..30).map(|i| format!("Long Context Menu Entry {i}")).collect();
        let wids: Vec<String> = (0..30).map(|i| format!("test.tall.{i}")).collect();
        let mut tall_rect = Rect::NOTHING;
        let mut out_tall = ctx.run_ui(egui::RawInput { screen_rect: Some(vp), ..Default::default() }, |ui| {
            let mut meta = FrameInspectMeta::new();
            meta.begin_pass(ui.ctx());
            let entries: Vec<ActionMenuEntry<'_>> =
                labels.iter().zip(wids.iter()).map(|(l, w)| ActionMenuEntry::Item(ActionMenuItem::menu_item(w, l))).collect();
            let res = action_menu_popup(
                ui.ctx(),
                &tokens,
                Some(&mut meta),
                ActionMenuSpec::context(egui::Id::unique("test_tall_ctx"), "popup.test.tall", "Tall Menu", Pos2::new(620.0, 400.0)),
                &entries,
            );
            tall_rect = res.popup_rect;
        });
        out_tall.textures_delta.clear();
        assert!(vp.shrink(3.0).contains_rect(tall_rect), "tall context menu must stay inside viewport.shrink(4): {tall_rect:?}");
    }

    #[test]
    fn r20_21_show_tooltip_bands_450ms_delay_immediate_dismiss_and_no_echo_invariant() {
        assert!(is_echo_tooltip("Cancel", false, &TooltipSpec::new("Cancel")));
        assert!(is_echo_tooltip("OK", false, &TooltipSpec::new("OK").with_summary("OK")));
        assert!(!is_echo_tooltip("Very Long Sheet Name", true, &TooltipSpec::new("Very Long Sheet Name")));
        assert!(!is_echo_tooltip("Undo", false, &TooltipSpec::new("Undo").with_shortcut("⌘Z").with_summary("Revert last edit")));

        let ctx = Context::default();
        let tokens = Tokens::for_mode(UiThemeMode::Dark);
        ctx.all_styles_mut(|s| tokens.configure_style(s));
        assert!((ctx.global_style().interaction.tooltip_delay - 0.45).abs() <= f32::EPSILON);

        let run_pass = |events: Vec<egui::Event>, time: f64| -> (Vec<String>, Rect, Rect) {
            let mut b_rect = Rect::NOTHING;
            let mut e_rect = Rect::NOTHING;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 320.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let mut meta = FrameInspectMeta::new();
                        meta.begin_pass(ui.ctx());
                        let (r_cmd, _) = styled_button_with_tooltip(
                            ui,
                            &tokens,
                            &mut meta,
                            StyledButtonSpec {
                                widget_id: "test.cmd",
                                role: "button",
                                a11y_role: Role::Button,
                                a11y_selectable: false,
                                label: "Analyze",
                                tooltip: None,
                                command_id: Some("ui.analyze.open"),
                                width: ButtonWidth::Fixed(100.0),
                                height: 26.0,
                                font_size: 12.0,
                                radius: 4,
                                text_align: ButtonTextAlign::Center,
                                break_anywhere: false,
                                primary: false,
                                selected: false,
                                enabled: false,
                                accent_text_when_selected: false,
                                border_override: None,
                                value: None,
                                disabled_reason: Some("Create a Data table first"),
                                z_layer: ZLayer::Workspace,
                                skip_offscreen: false,
                                clip_interact_to_viewport: false,
                                tooltip_mode: TooltipMode::Standard,
                            },
                            Some(
                                TooltipSpec::new("Analyze")
                                    .with_shortcut("⌘⇧A")
                                    .with_summary("Run statistical fit or test")
                                    .with_disabled_reason("Create a Data table first"),
                            ),
                        );
                        b_rect = r_cmd.expect("cmd btn").rect;

                        let cancel_resp = ui.button("Cancel");
                        let cancel_resp = show_tooltip_for_control(&cancel_resp, &tokens, "Cancel", false, TooltipSpec::new("Cancel"));
                        e_rect = cancel_resp.rect;
                    });
                },
            );
            out.textures_delta.clear();
            let mut texts = Vec::new();
            collect_shape_texts(&out.shapes, &mut texts);
            (texts, b_rect, e_rect)
        };

        let (_, btn_rect, echo_rect) = run_pass(Vec::new(), 0.0);
        // Move over Analyze and wait 0.20s (< 450 ms delay): must NOT open yet.
        let _ = run_pass(vec![egui::Event::PointerMoved(btn_rect.center())], 1.0);
        let (early, _, _) = run_pass(Vec::new(), 1.20);
        assert!(!early.iter().any(|t| t.contains("Run statistical fit")), "tooltip opened before 450ms: {early:?}");

        // Dwell >= 0.45s + settle pass: renders 3 bands (Title, Shortcut pill, Summary, Disabled footer).
        let _ = run_pass(Vec::new(), 1.50);
        let (open, _, _) = run_pass(Vec::new(), 1.55);
        assert!(open.iter().any(|t| t.contains("⌘⇧A")), "missing shortcut pill: {open:?}");
        assert!(open.iter().any(|t| t.contains("Run statistical fit")), "missing summary band: {open:?}");
        assert!(open.iter().any(|t| t.contains("Create a Data table first")), "missing disabled footer: {open:?}");

        // Immediate dismissal on scroll wheel, pointer press, or drag.
        let (on_scroll, _, _) = run_pass(
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -24.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
                source: egui::MouseWheelSource::Wheel,
            }],
            1.57,
        );
        assert!(!on_scroll.iter().any(|t| t.contains("Run statistical fit")), "tooltip must dismiss on scroll: {on_scroll:?}");

        // Let smooth_scroll_delta decay naturally across a few short frames, then re-hover cleanly.
        for step in 1..=6 {
            let _ = run_pass(Vec::new(), 1.57 + (step as f64) * 0.05);
        }
        let _ = run_pass(vec![egui::Event::PointerMoved(echo_rect.center())], 1.9);
        let _ = run_pass(vec![egui::Event::PointerMoved(btn_rect.center())], 2.0);
        let _ = run_pass(Vec::new(), 2.50);
        let (reopened, _, _) = run_pass(Vec::new(), 2.55);
        assert!(reopened.iter().any(|t| t.contains("Run statistical fit")));
        let (on_press, _, _) = run_pass(
            vec![egui::Event::PointerButton {
                pos: btn_rect.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            2.57,
        );
        assert!(!on_press.iter().any(|t| t.contains("Run statistical fit")), "tooltip must dismiss immediately on press: {on_press:?}");
        let _ = run_pass(
            vec![egui::Event::PointerButton {
                pos: btn_rect.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            2.60,
        );

        // Hover unclipped self-explanatory "Cancel" Past 450ms -> must NOT emit redundant echo popup.
        let _ = run_pass(vec![egui::Event::PointerMoved(echo_rect.center())], 3.0);
        let _ = run_pass(Vec::new(), 3.55);
        let (cancel_hover, _, _) = run_pass(Vec::new(), 3.60);
        let cancel_count = cancel_hover.iter().filter(|t| t.trim() == "Cancel").count();
        assert_eq!(cancel_count, 1, "expected only button text 'Cancel' with zero echo tooltip: {cancel_hover:?}");
    }

    #[test]
    fn r20_14_link_button_uses_text_link_pointing_hand_hover_underline_min_24pt_and_link_role() {
        fn has_underline_stroke(shapes: &[egui::epaint::ClippedShape], link_color: Color32) -> bool {
            fn walk(shape: &egui::Shape, link_color: Color32) -> bool {
                match shape {
                    egui::Shape::LineSegment { stroke, .. } => stroke.color == link_color && (stroke.width - 1.0).abs() <= 0.1,
                    egui::Shape::Path(p) => p.stroke.color == egui::epaint::ColorMode::Solid(link_color),
                    egui::Shape::Vec(v) => v.iter().any(|s| walk(s, link_color)),
                    _ => false,
                }
            }
            shapes.iter().any(|cs| walk(&cs.shape, link_color))
        }

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let tokens = Tokens::for_mode(UiThemeMode::Light);
        let run_pass = |events: Vec<egui::Event>, time: f64, enabled: bool| -> (egui::FullOutput, FrameInspectMeta, Rect) {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 240.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut meta = FrameInspectMeta::new();
            let mut link_rect = Rect::NOTHING;
            let mut out = ctx.run_ui(raw, |ui| {
                tokens.apply_theme_to_ctx(ui.ctx());
                meta.begin_pass(ui.ctx());
                egui::CentralPanel::default().show(ui, |ui| {
                    let resp = link_button(
                        ui,
                        &tokens,
                        &mut meta,
                        LinkButtonSpec {
                            enabled,
                            disabled_reason: (!enabled).then_some("Read-only"),
                            ..LinkButtonSpec::new("test.link", "Open Documentation", ZLayer::Workspace)
                        },
                    );
                    link_rect = resp.rect;
                });
            });
            out.textures_delta.clear();
            (out, meta, link_rect)
        };

        let (rest_out, rest_meta, rect) = run_pass(Vec::new(), 0.0, true);
        assert!(rect.height() >= 24.0, "link_button height must be >= 24 pt (WCAG 2.5.8), got {}", rect.height());
        let meta_entry = rest_meta.find_by_widget_id("test.link").expect("meta entry");
        assert_eq!(meta_entry.role, "link");
        assert!(!has_underline_stroke(&rest_out.shapes, tokens.text_link), "link should not underline at rest");
        let ak_has_link = rest_out.platform_output.accesskit_update.as_ref().is_some_and(|upd| upd.nodes.iter().any(|(_, n)| n.role() == Role::Link));
        assert!(ak_has_link, "AccessKit tree must expose Role::Link for link_button");

        let (hover_out, _, _) = run_pass(vec![egui::Event::PointerMoved(rect.center())], 0.1, true);
        assert_eq!(hover_out.platform_output.cursor_icon, egui::CursorIcon::PointingHand);
        assert!(has_underline_stroke(&hover_out.shapes, tokens.text_link), "hovered link must draw 1 pt underline in tokens.text_link");

        let (dis_out, _, _) = run_pass(vec![egui::Event::PointerMoved(rect.center())], 0.2, false);
        assert_ne!(dis_out.platform_output.cursor_icon, egui::CursorIcon::PointingHand);
        assert!(!has_underline_stroke(&dis_out.shapes, tokens.text_link));
    }
}
