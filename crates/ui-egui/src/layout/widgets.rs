//! Reusable retained UI widget builders and interactive state models (`R10.16`).
//!
//! Covers push buttons, icon+label toolbar buttons, tab/segmented strips, radio buttons,
//! checkboxes, scrollable dropdowns, numeric spinners, sliders, single/multi-line text fields
//! (caret, selection, clipboard, IME preedit), disclosure rows, splitters, vertical/horizontal
//! scrollbars, modal scrim/dialog shells, focus rings, and disabled-reason tooltips.

use std::collections::BTreeMap;

use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};

use super::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use super::scroll::{ScrollMeta, compute_scrollbar_thumb, scrollbar_thickness_px};
use super::{LayoutBuilder, LayoutRectPx, NodeSpec, TextRunSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::{TextMeasurer, UiTextStyle, ellipsize_text};

/// Option item inside a scrollable [`OpenDropdownState`] popup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropdownOption {
    pub value: String,
    pub label: String,
    pub enabled: bool,
}

/// Currently open dropdown menu popup rendered in `ZLayer::DropdownMenu` or `ZLayer::ModalDropdown`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenDropdownState {
    pub dropdown_id: String,
    pub label: String,
    pub z_layer: ZLayer,
    pub anchor_rect_px: LayoutRectPx,
    pub selected_value: String,
    pub options: Vec<DropdownOption>,
}

/// Interactive single/multi-line text editing buffer (caret, selection, clipboard, IME preedit).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextFieldEditState {
    pub text: String,
    pub cursor_char: usize,
    pub selection_anchor: Option<usize>,
    pub preedit: Option<String>,
    pub multiline: bool,
}

impl TextFieldEditState {
    #[must_use]
    pub fn new(initial: impl Into<String>, multiline: bool) -> Self {
        let text = initial.into();
        let len = text.chars().count();
        Self { text, cursor_char: len, selection_anchor: None, preedit: None, multiline }
    }

    pub fn select_all(&mut self) {
        let len = self.text.chars().count();
        self.selection_anchor = Some(0);
        self.cursor_char = len;
    }

    pub fn move_left(&mut self, extend_selection: bool) {
        if !extend_selection {
            self.selection_anchor = None;
        } else if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor_char);
        }
        self.cursor_char = self.cursor_char.saturating_sub(1);
    }

    pub fn move_right(&mut self, extend_selection: bool) {
        let len = self.text.chars().count();
        if !extend_selection {
            self.selection_anchor = None;
        } else if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor_char);
        }
        self.cursor_char = (self.cursor_char + 1).min(len);
    }

    pub fn insert_text(&mut self, input: &str) {
        self.delete_selection_internal();
        let sanitized = if self.multiline { input.to_owned() } else { input.replace(['\r', '\n'], " ") };
        let byte_pos = char_to_byte_idx(&self.text, self.cursor_char);
        self.text.insert_str(byte_pos, &sanitized);
        self.cursor_char += sanitized.chars().count();
        self.selection_anchor = None;
        self.preedit = None;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection_internal() {
            return;
        }
        if self.cursor_char == 0 {
            return;
        }
        let end_b = char_to_byte_idx(&self.text, self.cursor_char);
        let start_b = char_to_byte_idx(&self.text, self.cursor_char - 1);
        self.text.replace_range(start_b..end_b, "");
        self.cursor_char -= 1;
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection_internal() {
            return;
        }
        let len = self.text.chars().count();
        if self.cursor_char >= len {
            return;
        }
        let start_b = char_to_byte_idx(&self.text, self.cursor_char);
        let end_b = char_to_byte_idx(&self.text, self.cursor_char + 1);
        self.text.replace_range(start_b..end_b, "");
    }

    #[must_use]
    pub fn copy_selection(&self) -> Option<String> {
        let anchor = self.selection_anchor?;
        let lo = anchor.min(self.cursor_char);
        let hi = anchor.max(self.cursor_char);
        if lo == hi {
            return None;
        }
        let sb = char_to_byte_idx(&self.text, lo);
        let eb = char_to_byte_idx(&self.text, hi);
        Some(self.text[sb..eb].to_owned())
    }

    pub fn set_preedit(&mut self, preedit: Option<String>) {
        self.preedit = preedit.filter(|s| !s.is_empty());
    }

    pub fn commit_preedit(&mut self) {
        if let Some(p) = self.preedit.take() {
            self.insert_text(&p);
        }
    }

    fn delete_selection_internal(&mut self) -> bool {
        let Some(anchor) = self.selection_anchor.take() else {
            return false;
        };
        let lo = anchor.min(self.cursor_char);
        let hi = anchor.max(self.cursor_char);
        if lo == hi {
            return false;
        }
        let sb = char_to_byte_idx(&self.text, lo);
        let eb = char_to_byte_idx(&self.text, hi);
        self.text.replace_range(sb..eb, "");
        self.cursor_char = lo;
        true
    }
}

fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or(s.len(), |(idx, _)| idx)
}

/// Focus / state helper evaluating hovered, focused, and enabled styling for a widget.
#[must_use]
pub fn widget_state(builder: &LayoutBuilder<'_>, id: &str, enablement: ControlEnablement) -> WidgetVisualState {
    WidgetVisualState {
        hovered: builder.app.interaction.hovered_widget_id.as_deref() == Some(id),
        active: builder.app.interaction.pressed_widget_id.as_deref() == Some(id),
        focused: builder.app.focus_widget_id.as_deref() == Some(id),
        enablement,
        ellipsized: false,
    }
}

/// Placement and styling options for [`paint_text_fitted`].
pub struct FittedTextSpec<'b> {
    pub z_layer: ZLayer,
    pub rect: LayoutRectPx,
    pub pad_x: u32,
    pub text: &'b str,
    pub style: &'b UiTextStyle,
    pub color: Rgba8,
    pub scroll_container: Option<&'b str>,
}

/// Paint text clipped inside `spec.rect` with horizontal `pad_x`, automatically ellipsizing overflow.
pub fn paint_text_fitted(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, spec: FittedTextSpec<'_>) -> bool {
    let vis_rect = spec.rect.intersect(builder.active_clip_rect());
    if vis_rect.width <= spec.pad_x.saturating_mul(2) || vis_rect.height < 6 || spec.text.is_empty() {
        return false;
    }
    let avail_w = f64::from(vis_rect.width.saturating_sub(spec.pad_x.saturating_mul(2)));
    let (display, ellipsized) = ellipsize_text(builder.measurer, spec.text, spec.style, &builder.tree.locale, avail_w);
    let metrics = builder.measurer.measure(&display, spec.style, &builder.tree.locale);
    if display == "…" && metrics.width_px > f64::from(vis_rect.width.saturating_sub(spec.pad_x)) {
        return false;
    }
    if let Some(node) = builder.tree.nodes.iter_mut().rev().find(|n| n.id == *owner) {
        node.state.ellipsized = node.state.ellipsized || ellipsized;
    }
    let top_y = f64::from(spec.rect.y) + ((f64::from(spec.rect.height) - metrics.line_height_px) * 0.5).max(0.0);
    let x_px = f64::from(vis_rect.x + i32::try_from(spec.pad_x).unwrap_or(0));
    builder.paint_text(TextRunSpec {
        z_layer: spec.z_layer,
        owner,
        scroll_container_id: spec.scroll_container,
        pos_px: [x_px, top_y],
        text: &display,
        style: spec.style,
        color: spec.color,
        ellipsized,
    });
    ellipsized
}

/// Focus ring border rendered when `state.focused == true`.
pub fn maybe_paint_focus_ring(builder: &mut LayoutBuilder<'_>, z_layer: ZLayer, owner: &WidgetId, rect: LayoutRectPx, focused: bool) {
    if focused && rect.width >= 4 && rect.height >= 4 {
        builder.paint_stroke(z_layer, Some(owner), rect, builder.palette.accent, 2);
    }
}

/// Specification for rendering a push or toolbar button.
pub struct ButtonWidgetSpec<'b> {
    pub id: &'b str,
    pub parent: Option<WidgetId>,
    pub z_layer: ZLayer,
    pub rect: LayoutRectPx,
    pub label: &'b str,
    pub keyboard_path: &'b str,
    pub focus_scope: FocusScope,
    pub tab_order: Option<u32>,
    pub primary: bool,
    pub selected: bool,
    pub shortcut: Option<&'b str>,
    pub enablement: ControlEnablement,
    pub action: WidgetAction,
}

/// Emit a crisp push button / CTA / toolbar button with proportional typography, border, and focus ring.
pub fn emit_button(builder: &mut LayoutBuilder<'_>, spec: ButtonWidgetSpec<'_>) -> WidgetId {
    let mut state = widget_state(builder, spec.id, spec.enablement);
    let bg = button_bg(builder, &state, spec.primary, spec.selected);
    let fg = button_fg(builder, &state, spec.primary);
    let border = if spec.selected || state.focused { builder.palette.accent } else { builder.palette.border };

    let mut a11y = AccessibilityMeta::new(WidgetRole::Button, spec.label, spec.focus_scope).with_keyboard_path(spec.keyboard_path);
    a11y.selected = Some(spec.selected);
    a11y.shortcut = spec.shortcut.map(str::to_owned);
    a11y.tab_order = spec.tab_order;

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(spec.id),
        parent: spec.parent,
        z_layer: spec.z_layer,
        rect_px: spec.rect,
        a11y,
        state: state.clone(),
        action: if state.enablement.enabled { spec.action } else { WidgetAction::NoOp },
    });

    builder.paint_fill(spec.z_layer, Some(&wid), spec.rect, bg);
    builder.paint_stroke(spec.z_layer, Some(&wid), spec.rect, border, 1);
    maybe_paint_focus_ring(builder, spec.z_layer, &wid, spec.rect, state.focused);

    let size_px = (12.0 * builder.scale()).clamp(9.0, 26.0);
    let style = if spec.primary || spec.selected { UiTextStyle::ui_bold(size_px) } else { UiTextStyle::ui_sans(size_px) };
    state.ellipsized = paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec { z_layer: spec.z_layer, rect: spec.rect, pad_x: 6, text: spec.label, style: &style, color: fg, scroll_container: None },
    );
    let _ = state;
    wid
}

/// Emit a tab/segmented strip control (`WidgetRole::Tab`).
pub fn emit_tab(builder: &mut LayoutBuilder<'_>, spec: ButtonWidgetSpec<'_>) -> WidgetId {
    let state = widget_state(builder, spec.id, spec.enablement);
    let bg = if spec.selected {
        builder.palette.surface
    } else if state.hovered {
        builder.palette.header
    } else {
        builder.palette.panel
    };
    let fg = if state.enablement.enabled { builder.palette.text } else { builder.palette.muted };

    let mut a11y = AccessibilityMeta::new(WidgetRole::Tab, spec.label, spec.focus_scope).with_keyboard_path(spec.keyboard_path);
    a11y.selected = Some(spec.selected);
    a11y.tab_order = spec.tab_order;

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(spec.id),
        parent: spec.parent,
        z_layer: spec.z_layer,
        rect_px: spec.rect,
        a11y,
        state: state.clone(),
        action: if state.enablement.enabled { spec.action } else { WidgetAction::NoOp },
    });

    builder.paint_fill(spec.z_layer, Some(&wid), spec.rect, bg);
    builder.paint_stroke(spec.z_layer, Some(&wid), spec.rect, builder.palette.border, 1);
    if spec.selected && spec.rect.height >= 4 {
        let bar = LayoutRectPx::new(spec.rect.x, spec.rect.bottom() - 2, spec.rect.width, 2);
        builder.paint_fill(spec.z_layer, Some(&wid), bar, builder.palette.accent);
    }
    maybe_paint_focus_ring(builder, spec.z_layer, &wid, spec.rect, state.focused);
    let size_px = (12.0 * builder.scale()).clamp(9.0, 26.0);
    let style = if spec.selected { UiTextStyle::ui_bold(size_px) } else { UiTextStyle::ui_sans(size_px) };
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec { z_layer: spec.z_layer, rect: spec.rect, pad_x: 6, text: spec.label, style: &style, color: fg, scroll_container: None },
    );
    wid
}

/// Specification for checkbox or radio button rows.
pub struct ToggleWidgetSpec<'b> {
    pub id: &'b str,
    pub parent: Option<WidgetId>,
    pub z_layer: ZLayer,
    pub rect: LayoutRectPx,
    pub label: &'b str,
    pub keyboard_path: &'b str,
    pub focus_scope: FocusScope,
    pub tab_order: Option<u32>,
    pub checked: bool,
    pub is_radio: bool,
    pub enablement: ControlEnablement,
    pub scroll_container: Option<&'b str>,
    pub action: WidgetAction,
}

/// Emit a reusable checkbox or radio button with box/circle glyph, checkmark/dot, label, and disabled state.
pub fn emit_toggle(builder: &mut LayoutBuilder<'_>, spec: ToggleWidgetSpec<'_>) -> WidgetId {
    let state = widget_state(builder, spec.id, spec.enablement);
    let role = if spec.is_radio { WidgetRole::RadioButton } else { WidgetRole::CheckBox };
    let mut a11y = AccessibilityMeta::new(role, spec.label, spec.focus_scope).with_keyboard_path(spec.keyboard_path);
    a11y.checked = Some(spec.checked);
    a11y.selected = spec.is_radio.then_some(spec.checked);
    a11y.tab_order = spec.tab_order;

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(spec.id),
        parent: spec.parent,
        z_layer: spec.z_layer,
        rect_px: spec.rect,
        a11y,
        state: state.clone(),
        action: if state.enablement.enabled { spec.action } else { WidgetAction::NoOp },
    });

    if state.hovered && state.enablement.enabled {
        builder.paint_fill(spec.z_layer, Some(&wid), spec.rect, builder.palette.panel);
    }
    let box_sz = (spec.rect.height.saturating_sub(6)).clamp(10, 18);
    let box_y = spec.rect.y + i32::try_from((spec.rect.height.saturating_sub(box_sz)) / 2).unwrap_or(0);
    let box_rect = LayoutRectPx::new(spec.rect.x + 4, box_y, box_sz, box_sz);
    builder.paint_fill(spec.z_layer, Some(&wid), box_rect, builder.palette.surface);
    let border_col = if spec.checked && state.enablement.enabled { builder.palette.accent } else { builder.palette.border };
    builder.paint_stroke(spec.z_layer, Some(&wid), box_rect, border_col, 1);
    if spec.checked {
        let inner = box_rect.inset(3);
        let mark_col = if state.enablement.enabled { builder.palette.accent } else { builder.palette.muted };
        builder.paint_fill(spec.z_layer, Some(&wid), inner, mark_col);
    }

    maybe_paint_focus_ring(builder, spec.z_layer, &wid, spec.rect, state.focused);
    let text_x = box_rect.right() + 6;
    let text_w = u32::try_from((spec.rect.right() - text_x).max(0)).unwrap_or(0);
    let text_rect = LayoutRectPx::new(text_x, spec.rect.y, text_w, spec.rect.height);
    let fg = if state.enablement.enabled { builder.palette.text } else { builder.palette.muted };
    let size_px = (12.0 * builder.scale()).clamp(9.0, 24.0);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: spec.z_layer,
            rect: text_rect,
            pad_x: 2,
            text: spec.label,
            style: &UiTextStyle::ui_sans(size_px),
            color: fg,
            scroll_container: spec.scroll_container,
        },
    );
    wid
}

/// Specification for closed dropdown / combobox trigger button.
pub struct DropdownSpec<'b> {
    pub id: &'b str,
    pub parent: Option<WidgetId>,
    pub z_layer: ZLayer,
    pub rect: LayoutRectPx,
    pub label: &'b str,
    pub selected_value: &'b str,
    pub keyboard_path: &'b str,
    pub focus_scope: FocusScope,
    pub tab_order: Option<u32>,
    pub enablement: ControlEnablement,
}

/// Emit a closed dropdown trigger button (opening its scrollable popup into `ZLayer::DropdownMenu` / `ModalDropdown`).
pub fn emit_dropdown(builder: &mut LayoutBuilder<'_>, spec: DropdownSpec<'_>) -> WidgetId {
    let state = widget_state(builder, spec.id, spec.enablement);
    let mut a11y = AccessibilityMeta::new(WidgetRole::ComboBox, spec.label, spec.focus_scope).with_keyboard_path(spec.keyboard_path);
    a11y.value_text = Some(spec.selected_value.to_owned());
    a11y.tab_order = spec.tab_order;
    a11y.expanded = Some(builder.app.interaction.open_dropdown.as_ref().is_some_and(|d| d.dropdown_id == spec.id));

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(spec.id),
        parent: spec.parent,
        z_layer: spec.z_layer,
        rect_px: spec.rect,
        a11y,
        state: state.clone(),
        action: if state.enablement.enabled { WidgetAction::OpenDropdown(spec.id.to_owned()) } else { WidgetAction::NoOp },
    });

    let bg = if state.hovered { builder.palette.header } else { builder.palette.surface };
    builder.paint_fill(spec.z_layer, Some(&wid), spec.rect, bg);
    builder.paint_stroke(spec.z_layer, Some(&wid), spec.rect, builder.palette.border, 1);
    maybe_paint_focus_ring(builder, spec.z_layer, &wid, spec.rect, state.focused);

    let text_rect = LayoutRectPx::new(spec.rect.x, spec.rect.y, spec.rect.width.saturating_sub(18), spec.rect.height);
    let fg = if state.enablement.enabled { builder.palette.text } else { builder.palette.muted };
    let size_px = (12.0 * builder.scale()).clamp(9.0, 24.0);
    let text_label = format!("{}: {}", spec.label, spec.selected_value);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: spec.z_layer,
            rect: text_rect,
            pad_x: 6,
            text: &text_label,
            style: &UiTextStyle::ui_sans(size_px),
            color: fg,
            scroll_container: None,
        },
    );

    let cx = spec.rect.right() - 10;
    let cy = spec.rect.center()[1];
    builder.paint_line(spec.z_layer, Some(&wid), [cx - 4, cy - 2], [cx, cy + 2], fg, 1);
    builder.paint_line(spec.z_layer, Some(&wid), [cx, cy + 2], [cx + 4, cy - 2], fg, 1);
    wid
}

/// Emit an open scrollable dropdown popup list in `open.z_layer` (`DropdownMenu` or `ModalDropdown`).
pub fn emit_open_dropdown_popup(builder: &mut LayoutBuilder<'_>, open: &OpenDropdownState) {
    let row_h = (22.0 * builder.scale()).round().clamp(18.0, 36.0) as u32;
    let visible_rows = (open.options.len() as u32).clamp(1, 8);
    let pop_h = visible_rows * row_h + 4;
    let pop_w = open.anchor_rect_px.width.max(180);
    let max_bottom = i32::try_from(builder.tree.metrics.physical_height_px()).unwrap_or(900);
    let below_y = open.anchor_rect_px.bottom() + 2;
    let pop_y = if below_y + i32::try_from(pop_h).unwrap_or(0) <= max_bottom {
        below_y
    } else {
        (open.anchor_rect_px.y - i32::try_from(pop_h).unwrap_or(0) - 2).max(4)
    };
    let popup_rect = LayoutRectPx::new(open.anchor_rect_px.x, pop_y, pop_w, pop_h);
    let content_h = (open.options.len() as u32).saturating_mul(row_h);
    let container_id = format!("dropdown_popup.{}", open.dropdown_id);
    let scroll_off = builder.scroll_offset(&container_id);
    let scroll_meta = ScrollMeta::new(container_id.clone(), popup_rect, [pop_w, content_h], scroll_off, super::ScrollAxes::Vertical);

    let mut cont_a11y = AccessibilityMeta::new(WidgetRole::ContextMenu, open.label.clone(), FocusScope::ModalDialog);
    cont_a11y.keyboard_path = Some(format!("dropdown.{}", open.dropdown_id));
    let cont_id = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("popup.{}", open.dropdown_id)),
        parent: None,
        z_layer: open.z_layer,
        rect_px: popup_rect,
        a11y: cont_a11y,
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&cont_id, scroll_meta.clone());
    builder.paint_fill(open.z_layer, Some(&cont_id), popup_rect, builder.palette.surface);
    builder.paint_stroke(open.z_layer, Some(&cont_id), popup_rect, builder.palette.accent, 1);

    let inner_clip = popup_rect.inset(2);
    builder.push_clip(inner_clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let size_px = (12.0 * builder.scale()).clamp(9.0, 24.0);
    for (idx, opt) in open.options.iter().enumerate() {
        let ry = inner_clip.y + i32::try_from(idx).unwrap_or(0) * i32::try_from(row_h).unwrap_or(22) - y_scroll;
        if ry + i32::try_from(row_h).unwrap_or(22) < inner_clip.y || ry > inner_clip.bottom() {
            continue;
        }
        let item_rect = LayoutRectPx::new(inner_clip.x, ry, inner_clip.width, row_h);
        let item_id = format!("popup.{}.opt.{}", open.dropdown_id, idx);
        let selected = opt.value == open.selected_value;
        let st = widget_state(
            builder,
            &item_id,
            if opt.enabled { ControlEnablement::enabled() } else { ControlEnablement::disabled("Option unavailable") },
        );
        let mut a11y = AccessibilityMeta::new(WidgetRole::MenuItem, opt.label.clone(), FocusScope::ModalDialog)
            .with_keyboard_path(format!("dropdown.{}.{}", open.dropdown_id, opt.value));
        a11y.selected = Some(selected);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(item_id),
            parent: Some(cont_id.clone()),
            z_layer: open.z_layer,
            rect_px: item_rect,
            a11y,
            state: st.clone(),
            action: if opt.enabled {
                WidgetAction::SelectDropdownOption { dropdown_id: open.dropdown_id.clone(), value: opt.value.clone() }
            } else {
                WidgetAction::NoOp
            },
        });
        let bg = if selected {
            builder.palette.accent
        } else if st.hovered {
            builder.palette.panel
        } else {
            builder.palette.surface
        };
        let fg = if selected { Rgba8::WHITE } else { builder.palette.text };
        builder.paint_fill(open.z_layer, Some(&wid), item_rect, bg);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: open.z_layer,
                rect: item_rect,
                pad_x: 6,
                text: &opt.label,
                style: &UiTextStyle::ui_sans(size_px),
                color: fg,
                scroll_container: Some(&container_id),
            },
        );
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, open.z_layer, &scroll_meta, Some(cont_id));
}

/// Emit numeric spinner (`-`, value display, `+`) returning widget IDs `(dec, value, inc)`.
pub struct SpinnerSpec<'b> {
    pub id_prefix: &'b str,
    pub parent: Option<WidgetId>,
    pub z_layer: ZLayer,
    pub rect: LayoutRectPx,
    pub label: &'b str,
    pub value_text: &'b str,
    pub step: f64,
    pub focus_scope: FocusScope,
    pub tab_order: Option<u32>,
    pub enablement: ControlEnablement,
}

pub fn emit_numeric_spinner(builder: &mut LayoutBuilder<'_>, spec: SpinnerSpec<'_>) {
    let btn_w = (spec.rect.height).clamp(18, 52);
    let dec_rect = LayoutRectPx::new(spec.rect.x, spec.rect.y, btn_w, spec.rect.height);
    let inc_rect = LayoutRectPx::new(spec.rect.right() - i32::try_from(btn_w).unwrap_or(20), spec.rect.y, btn_w, spec.rect.height);
    let val_w = spec.rect.width.saturating_sub(btn_w.saturating_mul(2)).max(24);
    let val_rect = LayoutRectPx::new(dec_rect.right(), spec.rect.y, val_w, spec.rect.height);

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{}.dec", spec.id_prefix),
            parent: spec.parent.clone(),
            z_layer: spec.z_layer,
            rect: dec_rect,
            label: "-",
            keyboard_path: &format!("{}.dec", spec.id_prefix),
            focus_scope: spec.focus_scope,
            tab_order: spec.tab_order,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: spec.enablement.clone(),
            action: WidgetAction::StepSpinner { field_id: spec.id_prefix.to_owned(), delta: -spec.step },
        },
    );

    let val_id = format!("{}.val", spec.id_prefix);
    let st = widget_state(builder, &val_id, spec.enablement.clone());
    let mut a11y = AccessibilityMeta::new(WidgetRole::SpinButton, spec.label, spec.focus_scope).with_keyboard_path(format!("{}.val", spec.id_prefix));
    a11y.value_text = Some(spec.value_text.to_owned());
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(val_id),
        parent: spec.parent.clone(),
        z_layer: spec.z_layer,
        rect_px: val_rect,
        a11y,
        state: st,
        action: WidgetAction::FocusWidget(format!("{}.val", spec.id_prefix)),
    });
    builder.paint_fill(spec.z_layer, Some(&wid), val_rect, builder.palette.surface);
    builder.paint_stroke(spec.z_layer, Some(&wid), val_rect, builder.palette.border, 1);
    let size_px = (12.0 * builder.scale()).clamp(9.0, 24.0);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: spec.z_layer,
            rect: val_rect,
            pad_x: 4,
            text: spec.value_text,
            style: &UiTextStyle::grid_tabular(size_px),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{}.inc", spec.id_prefix),
            parent: spec.parent,
            z_layer: spec.z_layer,
            rect: inc_rect,
            label: "+",
            keyboard_path: &format!("{}.inc", spec.id_prefix),
            focus_scope: spec.focus_scope,
            tab_order: spec.tab_order.map(|t| t + 1),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: spec.enablement,
            action: WidgetAction::StepSpinner { field_id: spec.id_prefix.to_owned(), delta: spec.step },
        },
    );
}

/// Emit a single or multi-line interactive text field with caret, selection, and IME preedit display.
pub struct TextFieldSpec<'b> {
    pub id: &'b str,
    pub parent: Option<WidgetId>,
    pub z_layer: ZLayer,
    pub rect: LayoutRectPx,
    pub label: &'b str,
    pub fallback_text: &'b str,
    pub keyboard_path: &'b str,
    pub focus_scope: FocusScope,
    pub tab_order: Option<u32>,
    pub multiline: bool,
    pub enablement: ControlEnablement,
}

pub fn emit_text_field(builder: &mut LayoutBuilder<'_>, spec: TextFieldSpec<'_>) -> WidgetId {
    let st = widget_state(builder, spec.id, spec.enablement);
    let buf = builder.app.interaction.text_field_buffers.get(spec.id);
    let display_text = buf.map_or(spec.fallback_text, |b| b.text.as_str());
    let mut a11y = AccessibilityMeta::new(WidgetRole::TextBox, spec.label, spec.focus_scope).with_keyboard_path(spec.keyboard_path);
    a11y.value_text = Some(display_text.to_owned());
    a11y.tab_order = spec.tab_order;

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(spec.id),
        parent: spec.parent,
        z_layer: spec.z_layer,
        rect_px: spec.rect,
        a11y,
        state: st.clone(),
        action: if st.enablement.enabled { WidgetAction::FocusWidget(spec.id.to_owned()) } else { WidgetAction::NoOp },
    });

    builder.paint_fill(spec.z_layer, Some(&wid), spec.rect, builder.palette.surface);
    let border_col = if st.focused { builder.palette.accent } else { builder.palette.border };
    builder.paint_stroke(spec.z_layer, Some(&wid), spec.rect, border_col, if st.focused { 2 } else { 1 });

    let size_px = (12.0 * builder.scale()).clamp(9.0, 24.0);
    let style = UiTextStyle::ui_sans(size_px);
    let preedit_suffix = buf.and_then(|b| b.preedit.as_deref()).unwrap_or("");
    let combined = if preedit_suffix.is_empty() { display_text.to_owned() } else { format!("{display_text}{preedit_suffix}") };

    if let Some(edit_state) = buf
        && let Some(anchor) = edit_state.selection_anchor
        && st.focused
    {
        let lo = anchor.min(edit_state.cursor_char);
        let hi = anchor.max(edit_state.cursor_char);
        if hi > lo {
            let pre: String = display_text.chars().take(lo).collect();
            let sel: String = display_text.chars().skip(lo).take(hi - lo).collect();
            let x0 = spec.rect.x + 6 + builder.measurer.measure(&pre, &style, &builder.tree.locale).width_px.round() as i32;
            let sw = builder.measurer.measure(&sel, &style, &builder.tree.locale).width_px.round().max(4.0) as u32;
            let sel_rect = LayoutRectPx::new(x0, spec.rect.y + 3, sw.min(spec.rect.width.saturating_sub(8)), spec.rect.height.saturating_sub(6));
            builder.paint_fill(spec.z_layer, Some(&wid), sel_rect, Rgba8::rgba(59, 130, 246, 80));
        }
    }

    let fg = if st.enablement.enabled { builder.palette.text } else { builder.palette.muted };
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec { z_layer: spec.z_layer, rect: spec.rect, pad_x: 6, text: &combined, style: &style, color: fg, scroll_container: None },
    );

    if st.focused {
        let cursor_chars = buf.map_or_else(|| combined.chars().count(), |b| b.cursor_char);
        let prefix: String = combined.chars().take(cursor_chars).collect();
        let cx_off = builder.measurer.measure(&prefix, &style, &builder.tree.locale).width_px.round() as i32;
        let cx = (spec.rect.x + 6 + cx_off).min(spec.rect.right() - 4);
        builder.paint_line(spec.z_layer, Some(&wid), [cx, spec.rect.y + 4], [cx, spec.rect.bottom() - 4], builder.palette.accent, 1);
    }
    wid
}

/// Specification for rendering a horizontal continuous slider control (`WidgetRole::Slider`).
pub struct SliderWidgetSpec<'b> {
    pub id: &'b str,
    pub parent: Option<WidgetId>,
    pub z_layer: ZLayer,
    pub rect: LayoutRectPx,
    pub label: &'b str,
    pub normalized: f64,
    pub enablement: ControlEnablement,
}

/// Emit a continuous horizontal slider control (`WidgetRole::Slider`).
pub fn emit_slider(builder: &mut LayoutBuilder<'_>, spec: SliderWidgetSpec<'_>) -> WidgetId {
    let t = if spec.normalized.is_finite() { spec.normalized.clamp(0.0, 1.0) } else { 0.0 };
    let st = widget_state(builder, spec.id, spec.enablement);
    let mut a11y = AccessibilityMeta::new(WidgetRole::Slider, spec.label, FocusScope::Workspace).with_keyboard_path(spec.id);
    a11y.value_text = Some(format!("{:.0}%", t * 100.0));
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(spec.id),
        parent: spec.parent,
        z_layer: spec.z_layer,
        rect_px: spec.rect,
        a11y,
        state: st.clone(),
        action: if st.enablement.enabled { WidgetAction::SetSlider { field_id: spec.id.to_owned(), normalized: t } } else { WidgetAction::NoOp },
    });
    let cy = spec.rect.center()[1];
    let track = LayoutRectPx::new(spec.rect.x + 6, cy - 2, spec.rect.width.saturating_sub(12).max(4), 4);
    builder.paint_fill(spec.z_layer, Some(&wid), track, builder.palette.border);
    let fill_w = (f64::from(track.width) * t).round() as u32;
    if fill_w > 0 {
        builder.paint_fill(spec.z_layer, Some(&wid), LayoutRectPx::new(track.x, track.y, fill_w, track.height), builder.palette.accent);
    }
    let thumb_x = track.x + i32::try_from(fill_w).unwrap_or(0) - 5;
    builder.paint_fill(spec.z_layer, Some(&wid), LayoutRectPx::new(thumb_x, cy - 6, 10, 12), builder.palette.surface);
    builder.paint_stroke(spec.z_layer, Some(&wid), LayoutRectPx::new(thumb_x, cy - 6, 10, 12), builder.palette.accent, 1);
    wid
}

/// Emit interactive vertical and/or horizontal scrollbars for `scroll_meta`.
pub fn emit_scrollbars_for_meta(builder: &mut LayoutBuilder<'_>, z_layer: ZLayer, scroll_meta: &ScrollMeta, parent: Option<WidgetId>) {
    let thick = scrollbar_thickness_px(builder.scale());
    if scroll_meta.has_vertical_overflow() && scroll_meta.viewport_px.width > thick + 8 {
        let track_rect = LayoutRectPx::new(
            scroll_meta.viewport_px.right() - i32::try_from(thick).unwrap_or(10),
            scroll_meta.viewport_px.y,
            thick,
            scroll_meta.viewport_px.height,
        );
        if let Some((track, thumb)) =
            compute_scrollbar_thumb(track_rect, scroll_meta.viewport_px.height, scroll_meta.content_size_px[1], scroll_meta.scroll_offset_px[1], true)
        {
            let v_id = format!("{}.vscroll", scroll_meta.container_id.as_str());
            let mut a11y =
                AccessibilityMeta::new(WidgetRole::ScrollBar, "Vertical Scrollbar", FocusScope::Workspace).with_keyboard_path(v_id.clone());
            a11y.value_text = Some(format!("{:.0}", scroll_meta.scroll_offset_px[1]));
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(v_id),
                parent: parent.clone(),
                z_layer,
                rect_px: track,
                a11y,
                state: WidgetVisualState::default(),
                action: WidgetAction::ScrollbarTrackJump { container_id: scroll_meta.container_id.0.clone(), vertical: true, ratio: 0.5 },
            });
            builder.paint_fill(z_layer, Some(&wid), track, builder.palette.panel);
            builder.paint_fill(z_layer, Some(&wid), thumb, builder.palette.muted);
        }
    }
    if scroll_meta.has_horizontal_overflow() && scroll_meta.viewport_px.height > thick + 8 {
        let track_rect = LayoutRectPx::new(
            scroll_meta.viewport_px.x,
            scroll_meta.viewport_px.bottom() - i32::try_from(thick).unwrap_or(10),
            scroll_meta.viewport_px.width,
            thick,
        );
        if let Some((track, thumb)) =
            compute_scrollbar_thumb(track_rect, scroll_meta.viewport_px.width, scroll_meta.content_size_px[0], scroll_meta.scroll_offset_px[0], false)
        {
            let h_id = format!("{}.hscroll", scroll_meta.container_id.as_str());
            let mut a11y =
                AccessibilityMeta::new(WidgetRole::ScrollBar, "Horizontal Scrollbar", FocusScope::Workspace).with_keyboard_path(h_id.clone());
            a11y.value_text = Some(format!("{:.0}", scroll_meta.scroll_offset_px[0]));
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(h_id),
                parent,
                z_layer,
                rect_px: track,
                a11y,
                state: WidgetVisualState::default(),
                action: WidgetAction::ScrollbarTrackJump { container_id: scroll_meta.container_id.0.clone(), vertical: false, ratio: 0.5 },
            });
            builder.paint_fill(z_layer, Some(&wid), track, builder.palette.panel);
            builder.paint_fill(z_layer, Some(&wid), thumb, builder.palette.muted);
        }
    }
}

/// Emit hovered tooltip overlay at `ZLayer::Tooltip` (for disabled-reason or hover command hints).
pub fn emit_active_tooltip(builder: &mut LayoutBuilder<'_>, full_rect: LayoutRectPx) {
    let tip_text: Option<String> = if let Some(hovered_id) = &builder.app.interaction.hovered_widget_id
        && let Some(node) = builder.tree.nodes.iter().find(|n| n.id.as_str() == hovered_id)
        && let Some(reason) = &node.state.enablement.disabled_reason
    {
        Some(reason.clone())
    } else if let Some(excl) = &builder.app.grid.hover_exclusion_tooltip {
        Some(excl.clone())
    } else {
        builder
            .app
            .hover_command_id
            .as_deref()
            .and_then(|cmd_id| crate::menus::command_tooltip(builder.app, cmd_id))
            .map(|t| t.format_inline_tooltip())
    };
    let Some(text) = tip_text.filter(|s| !s.is_empty()) else {
        return;
    };
    if full_rect.width < 120 || full_rect.height < 60 {
        return;
    }
    let size_px = (11.5 * builder.scale()).clamp(9.0, 22.0);
    let style = UiTextStyle::ui_sans(size_px);
    let max_w = full_rect.width.saturating_sub(24).min(360);
    let (display, ellipsized) = ellipsize_text(builder.measurer, &text, &style, &builder.tree.locale, f64::from(max_w.saturating_sub(12)));
    let tw = (builder.measurer.measure(&display, &style, &builder.tree.locale).width_px.ceil() as u32 + 14).clamp(60, max_w);
    let th = (size_px * 1.6).ceil() as u32;
    let rect = LayoutRectPx::new(full_rect.x + 12, full_rect.bottom() - i32::try_from(th + 8).unwrap_or(28), tw, th);
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from("overlay.tooltip"),
        parent: None,
        z_layer: ZLayer::Tooltip,
        rect_px: rect,
        a11y: AccessibilityMeta::new(WidgetRole::Tooltip, display.clone(), builder.tree.active_focus_scope),
        state: WidgetVisualState { ellipsized, ..Default::default() },
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Tooltip, Some(&wid), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Tooltip, Some(&wid), rect, builder.palette.accent, 1);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::Tooltip,
            rect,
            pad_x: 6,
            text: &display,
            style: &style,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
}

fn button_bg(builder: &LayoutBuilder<'_>, state: &WidgetVisualState, primary: bool, selected: bool) -> Rgba8 {
    if !state.enablement.enabled {
        return builder.palette.panel;
    }
    if primary {
        return builder.palette.accent;
    }
    if selected || state.hovered || state.active {
        return builder.palette.header;
    }
    builder.palette.surface
}

fn button_fg(builder: &LayoutBuilder<'_>, state: &WidgetVisualState, primary: bool) -> Rgba8 {
    if !state.enablement.enabled {
        return builder.palette.muted;
    }
    if primary { Rgba8::WHITE } else { builder.palette.text }
}

/// Collection type alias for per-widget edit states.
pub type TextFieldBufferMap = BTreeMap<String, TextFieldEditState>;
