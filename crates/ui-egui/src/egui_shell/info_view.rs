//! Native `egui` `InfoSheet` workspace (`docs/planning/gui-layout-spec.md` §8.4, `R15.3`):
//! `38%` structured Key/Value/Hook-Badge constants table + `62%` formatted laboratory notes.

use std::borrow::Cow;

use curvebook_doc::{ConstantValue, InfoConstant, InfoSheet, ProjectDocument, SheetId, extract_template_hooks};
use curvebook_text::expand_template_hooks;
use egui::{CornerRadius, FontId, Id, Key, Rect, RichText, Sense, Stroke, StrokeKind, TextEdit, Vec2, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use crate::grid_view::parse_localized_number_input;
use crate::i18n::{LocalePreset, normalize_locale, try_tr};
use crate::inspect::ZLayer;

const DEFAULT_SPLIT_RATIO: f32 = 0.38;
const MIN_SPLIT_RATIO: f32 = 0.24;
const MAX_SPLIT_RATIO: f32 = 0.62;
const DIVIDER_VISUAL_W: f32 = 4.0;
const DIVIDER_HIT_SLOP_W: f32 = 8.0;

/// Value type selector for inline constant editing (`§8.4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InfoConstantKind {
    #[default]
    Auto,
    Text,
    Number,
    Date,
}

impl InfoConstantKind {
    const fn from_value(value: &ConstantValue) -> Self {
        match value {
            ConstantValue::Empty => Self::Auto,
            ConstantValue::Number(_) => Self::Number,
            ConstantValue::Text(_) => Self::Text,
            ConstantValue::Date(_) => Self::Date,
        }
    }

    const fn badge(self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Text => "TXT",
            Self::Number => "NUM",
            Self::Date => "DATE",
        }
    }

    const fn as_param(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Text => Some("text"),
            Self::Number => Some("number"),
            Self::Date => Some("date"),
        }
    }

    const fn slug(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Text => "text",
            Self::Number => "number",
            Self::Date => "date",
        }
    }
}

/// Downstream document target referencing an [`InfoConstant`] via `HookSource::InfoConstant` or `{{info:...}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoHookReference {
    pub target_sheet_id: SheetId,
    pub target_title: String,
    pub context: String,
}

/// Persistent `egui_shell` editor state for `InfoSheet` (`R15.3`).
#[derive(Debug, Clone, PartialEq)]
pub struct InfoViewEguiState {
    pub selected_row: Option<usize>,
    pub editing_key: Option<String>,
    pub editing_value: String,
    pub notes_draft: Option<(SheetId, String)>,
    pub split_ratio: f32,
    pub editing_units: String,
    pub editing_kind: InfoConstantKind,
    pub editing_focus_seen: bool,
    pub notes_preview_mode: bool,
    pub notes_dirty: bool,
    pub notes_focus_seen: bool,
}

impl Default for InfoViewEguiState {
    fn default() -> Self {
        Self {
            selected_row: None,
            editing_key: None,
            editing_value: String::new(),
            notes_draft: None,
            split_ratio: DEFAULT_SPLIT_RATIO,
            editing_units: String::new(),
            editing_kind: InfoConstantKind::Auto,
            editing_focus_seen: false,
            notes_preview_mode: false,
            notes_dirty: false,
            notes_focus_seen: false,
        }
    }
}

/// Per-frame region report returned by [`render_info_view`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InfoViewFrameReport {
    pub table_rect_pt: Option<Rect>,
    pub notes_rect_pt: Option<Rect>,
    pub divider_rect_pt: Option<Rect>,
    pub split_ratio: f32,
    pub hook_badge_count: usize,
}

fn info_tr<'a>(locale: &str, key: &str, fallback: &'a str) -> Cow<'a, str> {
    let norm = normalize_locale(locale);
    if let Some(val) = try_tr(norm, key).or_else(|| try_tr("en", key)) { Cow::Borrowed(val) } else { Cow::Borrowed(fallback) }
}

fn matches_info_constant_hook(
    doc: &ProjectDocument,
    info_sheet_id: &SheetId,
    constant_name: &str,
    hook_info_id: Option<&SheetId>,
    hook_name: &str,
    owner_source_data_id: Option<&SheetId>,
) -> bool {
    if !hook_name.trim().eq_ignore_ascii_case(constant_name.trim()) {
        return false;
    }
    match hook_info_id {
        Some(explicit) => explicit == info_sheet_id,
        None => {
            if let Some(ds_id) = owner_source_data_id
                && let Some(ds) = doc.data_sheet(ds_id)
                && let Some(bound_info) = ds.info_sheet_id.as_ref()
            {
                return bound_info == info_sheet_id;
            }
            doc.info_sheets.first().is_some_and(|first| &first.id == info_sheet_id)
        }
    }
}

/// Discover every downstream analysis, graph annotation, layout callout, and floating note
/// referencing `constant_name` on `info_sheet_id`.
#[must_use]
pub fn count_constant_hook_references(doc: &ProjectDocument, info_sheet_id: &SheetId, constant_name: &str) -> Vec<InfoHookReference> {
    let mut out = Vec::new();
    let needle = constant_name.trim();
    if needle.is_empty() {
        return out;
    }

    for rs in &doc.result_sheets {
        let mut count = 0usize;
        for hook in rs.analysis.kind.hooked_sources() {
            if let curvebook_doc::HookSource::InfoConstant { info_sheet_id: hid, constant_name: cname } = &hook
                && matches_info_constant_hook(doc, info_sheet_id, needle, hid.as_ref(), cname, Some(&rs.analysis.source_sheet_id))
            {
                count += 1;
            }
        }
        if count > 0 {
            out.push(InfoHookReference {
                target_sheet_id: rs.id.clone(),
                target_title: rs.title().to_owned(),
                context: format!("Analysis ({count} hook{})", if count == 1 { "" } else { "s" }),
            });
        }
    }

    for gs in &doc.graph_sheets {
        let mut texts: Vec<&str> = Vec::new();
        texts.push(gs.spec.title.as_str());
        texts.push(gs.spec.x_axis.title.as_str());
        texts.push(gs.spec.y_axis.title.as_str());
        if let Some(y2) = &gs.spec.y2_axis {
            texts.push(y2.title.as_str());
        }
        for ann in &gs.spec.annotations {
            if let curvebook_doc::GraphAnnotation::TextBox { markdown, .. } = ann {
                texts.push(markdown.as_str());
            }
        }
        for note in &gs.floating_notes {
            texts.push(note.body_markdown.as_str());
        }
        let hits = count_inline_hooks_in_texts(doc, info_sheet_id, needle, &texts, Some(&gs.spec.primary_source_sheet_id));
        if hits > 0 {
            out.push(InfoHookReference {
                target_sheet_id: gs.id.clone(),
                target_title: gs.title.clone(),
                context: format!("Graph ({hits} token{})", if hits == 1 { "" } else { "s" }),
            });
        }
    }

    for ls in &doc.layout_sheets {
        let mut texts: Vec<&str> = Vec::new();
        for item in &ls.spec.items {
            if let curvebook_doc::LayoutItem::TextCallout { markdown, .. } = item {
                texts.push(markdown.as_str());
            }
        }
        for note in &ls.floating_notes {
            texts.push(note.body_markdown.as_str());
        }
        let hits = count_inline_hooks_in_texts(doc, info_sheet_id, needle, &texts, None);
        if hits > 0 {
            out.push(InfoHookReference {
                target_sheet_id: ls.id.clone(),
                target_title: ls.title.clone(),
                context: format!("Layout ({hits} token{})", if hits == 1 { "" } else { "s" }),
            });
        }
    }

    for ds in &doc.data_sheets {
        let texts: Vec<&str> = ds.floating_notes.iter().map(|n| n.body_markdown.as_str()).collect();
        let hits = count_inline_hooks_in_texts(doc, info_sheet_id, needle, &texts, Some(&ds.id));
        if hits > 0 {
            out.push(InfoHookReference {
                target_sheet_id: ds.id.clone(),
                target_title: ds.title.clone(),
                context: format!("Data Note ({hits} token{})", if hits == 1 { "" } else { "s" }),
            });
        }
    }

    out
}

fn count_inline_hooks_in_texts(
    doc: &ProjectDocument,
    info_sheet_id: &SheetId,
    constant_name: &str,
    texts: &[&str],
    owner_data_id: Option<&SheetId>,
) -> usize {
    let mut hits = 0usize;
    for txt in texts {
        for hook in extract_template_hooks(txt) {
            if let curvebook_doc::HookSource::InfoConstant { info_sheet_id: hid, constant_name: cname } = hook
                && matches_info_constant_hook(doc, info_sheet_id, constant_name, hid.as_ref(), &cname, owner_data_id)
            {
                hits += 1;
            }
        }
    }
    hits
}

/// Render the active [`curvebook_doc::InfoSheet`] (`38%` structured constants table + `62%` notes pane).
pub fn render_info_view(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut InfoViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> InfoViewFrameReport {
    let active_id = app.document().active_sheet_id.clone();
    render_info_view_for_sheet(ui, app, active_id.as_ref(), state, tokens, enabled, meta, intents)
}

/// Render a specific [`curvebook_doc::InfoSheet`] inside `ui` (`38%` / `62%` split with draggable divider).
#[allow(clippy::too_many_arguments)]
pub fn render_info_view_for_sheet(
    ui: &mut egui::Ui,
    app: &UiApp,
    sheet_id: Option<&SheetId>,
    state: &mut InfoViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> InfoViewFrameReport {
    let mut report = InfoViewFrameReport { split_ratio: sanitize_split_ratio(state.split_ratio), ..InfoViewFrameReport::default() };
    let doc = app.document();
    let Some(target_id) = sheet_id else {
        return report;
    };
    let Some(info) = doc.info_sheet(target_id) else {
        return report;
    };

    sync_notes_draft_with_sheet(state, info);

    let avail = ui.available_rect_before_wrap();
    if avail.width() < 60.0 || avail.height() < 60.0 {
        return report;
    }

    let ratio = sanitize_split_ratio(state.split_ratio);
    let inner_w = (avail.width() - DIVIDER_VISUAL_W).max(20.0);
    let min_left = (inner_w * MIN_SPLIT_RATIO).max(160.0).min(inner_w * 0.55);
    let max_left = (inner_w * MAX_SPLIT_RATIO).max(min_left);
    let left_w = (inner_w * ratio).clamp(min_left, max_left);
    let table_rect = Rect::from_min_max(avail.min, pos2(avail.left() + left_w, avail.bottom()));
    let divider_rect =
        Rect::from_min_max(pos2(table_rect.right(), avail.top()), pos2((table_rect.right() + DIVIDER_VISUAL_W).min(avail.right()), avail.bottom()));
    let notes_rect = Rect::from_min_max(pos2(divider_rect.right(), avail.top()), avail.max);

    report.table_rect_pt = Some(table_rect);
    report.divider_rect_pt = divider_rect.is_positive().then_some(divider_rect);
    report.notes_rect_pt = notes_rect.is_positive().then_some(notes_rect);

    ensure_solid_scrollbar_style(ui, tokens);

    // Left Pane (38%): Structured Constants Table with Hook Badges + Typed Inline Edit
    ui.scope_builder(egui::UiBuilder::new().max_rect(table_rect), |ui| {
        ui.set_clip_rect(table_rect);
        ui.painter().rect_filled(table_rect, CornerRadius::ZERO, tokens.surface_bg);
        report.hook_badge_count = render_constants_pane(ui, app, info, state, tokens, enabled, meta, intents);
    });

    // Draggable 4pt Divider with 8pt hit slop
    if divider_rect.is_positive() {
        let hit_rect = divider_rect.expand2(vec2((DIVIDER_HIT_SLOP_W - DIVIDER_VISUAL_W) * 0.5, 0.0));
        let div_resp = ui.interact(hit_rect, Id::unique(("info_split_divider", info.id.as_str())), Sense::click_and_drag());
        if div_resp.double_clicked() && enabled {
            state.split_ratio = DEFAULT_SPLIT_RATIO;
        } else if div_resp.dragged() && enabled {
            let next_left = (table_rect.width() + div_resp.drag_delta().x) / inner_w.max(1.0);
            state.split_ratio = next_left.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO);
        }
        report.split_ratio = sanitize_split_ratio(state.split_ratio);

        ui.painter().rect_filled(divider_rect, CornerRadius::ZERO, tokens.panel_bg);
        let line_x = divider_rect.center().x;
        let stroke_col = if div_resp.hovered() || div_resp.dragged() { tokens.accent } else { tokens.divider };
        ui.painter().line_segment([pos2(line_x, divider_rect.top()), pos2(line_x, divider_rect.bottom())], Stroke::new(1.5, stroke_col));

        meta.record_response(
            ui,
            &div_resp,
            FrameInspectEntryInput {
                widget_id: "info.split_divider",
                role: "splitter",
                label: "Info Constants/Notes Splitter",
                command_id: None,
                value: Some(json!({ "ratio": report.split_ratio })),
                enabled,
                disabled_reason: (!enabled).then_some("modal dialog is active"),
                z_layer: ZLayer::SplitHandle,
                ellipsized: false,
                full_label: None,
            },
        );
    }

    // Right Pane (62%): Formatted Protocol & Experiment Notes
    if notes_rect.is_positive() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(notes_rect), |ui| {
            ui.set_clip_rect(notes_rect);
            ui.painter().rect_filled(notes_rect, CornerRadius::ZERO, tokens.surface_bg);
            render_notes_pane(ui, doc, info, state, tokens, enabled, meta, intents);
        });
    }

    report
}

fn next_unused_constant_name(info: &InfoSheet) -> String {
    let mut n = 1usize;
    loop {
        let candidate = format!("Constant {n}");
        if !info.constants.iter().any(|c| c.name.trim().eq_ignore_ascii_case(&candidate)) {
            return candidate;
        }
        n = n.saturating_add(1);
    }
}

fn format_localized_constant_value(value: &ConstantValue, locale_preset: &LocalePreset) -> String {
    match value {
        ConstantValue::Number(v) if v.is_finite() => {
            let normalized = if *v == 0.0 { 0.0 } else { *v };
            let text = format!("{normalized}");
            if locale_preset.decimal_separator != '.' { text.replace('.', &locale_preset.decimal_separator.to_string()) } else { text }
        }
        _ => value.display_text(),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_constants_pane(
    ui: &mut egui::Ui,
    app: &UiApp,
    info: &InfoSheet,
    state: &mut InfoViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> usize {
    let doc = app.document();
    let locale_preset = &app.locale_preset;
    let mut hook_badge_count = 0usize;
    let padded = ui.available_rect_before_wrap().shrink(8.0);
    if !padded.is_positive() {
        return 0;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(padded), |ui| {
        ui.set_clip_rect(padded);
        let const_heading = info_tr(tokens.locale, "info.constants_heading", "Constants");

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let title_lbl = ui.add(egui::Label::new(RichText::new(const_heading.as_ref()).strong().color(tokens.text_strong)).truncate());
            title_lbl.on_hover_text(const_heading.as_ref());

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let add_lbl = "+ Add Constant";
                let add_resp = ui.add_enabled(enabled, egui::Button::new(add_lbl));
                meta.record_response(
                    ui,
                    &add_resp,
                    FrameInspectEntryInput {
                        widget_id: "info.add_constant",
                        role: "button",
                        label: add_lbl,
                        command_id: Some("info.setConstant"),
                        value: None,
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if add_resp.clicked() && enabled {
                    let new_name = next_unused_constant_name(info);
                    intents
                        .push_engine("info.setConstant", json!({ "sheet_id": info.id.as_str(), "name": new_name, "value": serde_json::Value::Null }));
                    state.selected_row = Some(info.constants.len());
                    begin_constant_edit(state, &InfoConstant { name: new_name, value: ConstantValue::Empty, units: None }, locale_preset);
                }
            });
        });

        ui.add_space(4.0);
        // Column header strip
        let hdr_rect = ui.allocate_space(vec2(ui.available_width().max(1.0), 22.0)).1;
        ui.painter().rect_filled(hdr_rect, CornerRadius::same(2), tokens.header_bg);
        ui.painter().rect_stroke(hdr_rect, CornerRadius::same(2), Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
        let name_w = (hdr_rect.width() * 0.42).max(60.0);
        let badge_w = 72.0_f32.min((hdr_rect.width() * 0.28).max(48.0));
        let val_w = (hdr_rect.width() - name_w - badge_w).max(40.0);
        ui.painter().text(
            pos2(hdr_rect.left() + 6.0, hdr_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Constant Name",
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_strong,
        );
        ui.painter().text(
            pos2(hdr_rect.left() + name_w + 4.0, hdr_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Value",
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_strong,
        );
        ui.painter().text(
            pos2(hdr_rect.left() + name_w + val_w + 4.0, hdr_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Type / 🔗",
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_strong,
        );
        ui.add_space(2.0);

        let scroll_out =
            egui::ScrollArea::vertical().id_salt(("egui_shell.info.constants", info.id.as_str())).auto_shrink([false, false]).show(ui, |ui| {
                let row_w = ui.available_width().max(80.0);
                let mut rows_to_render = Vec::with_capacity(info.constants.len());
                for (idx, c) in info.constants.iter().enumerate() {
                    let refs = count_constant_hook_references(doc, &info.id, &c.name);
                    rows_to_render.push((idx, c, refs));
                }

                for (idx, constant, refs) in rows_to_render {
                    let is_selected = state.selected_row == Some(idx);
                    let is_editing = state.editing_key.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(&constant.name));
                    if !refs.is_empty() {
                        hook_badge_count += 1;
                    }

                    if is_editing {
                        let target = InlineConstantTarget { info, constant, locale_preset };
                        render_inline_constant_editor(ui, target, state, tokens, enabled, meta, intents);
                        continue;
                    }

                    let row_h = 26.0;
                    let (row_rect, row_resp) = ui.allocate_exact_size(vec2(row_w, row_h), if enabled { Sense::click() } else { Sense::hover() });
                    let bg = if is_selected {
                        tokens.active_bg
                    } else if row_resp.hovered() {
                        tokens.hover_bg
                    } else if idx % 2 == 1 {
                        tokens.panel_bg
                    } else {
                        tokens.surface_bg
                    };
                    ui.painter().rect_filled(row_rect, CornerRadius::same(2), bg);
                    if is_selected {
                        ui.painter().rect_stroke(row_rect, CornerRadius::same(2), Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
                    }

                    let c_name_w = (row_rect.width() * 0.42).max(56.0);
                    let c_badge_w = 72.0_f32.min((row_rect.width() * 0.28).max(48.0));
                    let c_val_w = (row_rect.width() - c_name_w - c_badge_w).max(36.0);

                    let name_rect = Rect::from_min_size(row_rect.min, vec2(c_name_w, row_h));
                    let val_rect = Rect::from_min_size(pos2(name_rect.right(), row_rect.top()), vec2(c_val_w, row_h));
                    let badge_rect = Rect::from_min_size(pos2(val_rect.right(), row_rect.top()), vec2(c_badge_w, row_h));

                    let name_elided = paint_ellipsized_cell_text(
                        ui,
                        name_rect.shrink2(vec2(6.0, 2.0)),
                        &constant.name,
                        tokens.text_strong,
                        tokens.metrics.font_ui_small_pt,
                    );
                    let unit_suffix = constant.units.as_deref().filter(|u| !u.is_empty()).map_or(String::new(), |u| format!(" {u}"));
                    let val_display = if constant.value.display_text().is_empty() {
                        String::from("—")
                    } else {
                        format!("{}{unit_suffix}", constant.value.display_text())
                    };
                    let val_col = if constant.value.display_text().is_empty() { tokens.text_muted } else { tokens.text_primary };
                    let _ = paint_ellipsized_cell_text(ui, val_rect.shrink2(vec2(4.0, 2.0)), &val_display, val_col, tokens.metrics.font_ui_small_pt);

                    // Type pill & 🔗 Hook Badge inside trailing cell
                    if !refs.is_empty() {
                        let hook_lbl = format!("🔗 {}", refs.len());
                        let hook_resp = ui.put(
                            badge_rect.shrink2(vec2(2.0, 3.0)),
                            egui::Button::new(RichText::new(&hook_lbl).small().color(tokens.link_blue)).fill(tokens.panel_bg),
                        );
                        let tip = refs.iter().map(|r| format!("{} — {}", r.target_title, r.context)).collect::<Vec<_>>().join("\n");
                        let hook_wid = format!("info.hook_badge.{idx}");
                        meta.record_response(
                            ui,
                            &hook_resp,
                            FrameInspectEntryInput {
                                widget_id: &hook_wid,
                                role: "button",
                                label: &hook_lbl,
                                command_id: Some("sheet.select"),
                                value: Some(json!({ "constant": constant.name, "ref_count": refs.len() })),
                                enabled,
                                disabled_reason: (!enabled).then_some("modal dialog is active"),
                                z_layer: ZLayer::Workspace,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        hook_resp.clone().on_hover_text(tip);
                        if hook_resp.clicked() && enabled {
                            state.selected_row = Some(idx);
                            intents.push_engine("sheet.select", json!({ "sheet_id": refs[0].target_sheet_id.as_str() }));
                        }
                    } else {
                        let kind_badge = InfoConstantKind::from_value(&constant.value).badge();
                        ui.painter().text(
                            pos2(badge_rect.center().x, badge_rect.center().y),
                            egui::Align2::CENTER_CENTER,
                            kind_badge,
                            FontId::proportional(10.5),
                            tokens.text_muted,
                        );
                    }

                    let row_wid = format!("info.const.{idx}");
                    meta.record_response(
                        ui,
                        &row_resp,
                        FrameInspectEntryInput {
                            widget_id: &row_wid,
                            role: "button",
                            label: &constant.name,
                            command_id: Some("info.setConstant"),
                            value: Some(json!({ "name": constant.name, "value": val_display, "hooks": refs.len() })),
                            enabled,
                            disabled_reason: (!enabled).then_some("modal dialog is active"),
                            z_layer: ZLayer::Workspace,
                            ellipsized: name_elided,
                            full_label: name_elided.then_some(constant.name.as_str()),
                        },
                    );
                    if row_resp.double_clicked() && enabled {
                        state.selected_row = Some(idx);
                        begin_constant_edit(state, constant, locale_preset);
                    } else if row_resp.clicked() && enabled {
                        if state.selected_row == Some(idx) {
                            begin_constant_edit(state, constant, locale_preset);
                        } else {
                            state.selected_row = Some(idx);
                        }
                    }
                    if name_elided {
                        row_resp.on_hover_text(&constant.name);
                    }
                }
            });
        let _ = scroll_out.inner_rect;
    });

    hook_badge_count
}

struct InlineConstantTarget<'a> {
    info: &'a InfoSheet,
    constant: &'a InfoConstant,
    locale_preset: &'a LocalePreset,
}

fn begin_constant_edit(state: &mut InfoViewEguiState, constant: &InfoConstant, locale_preset: &LocalePreset) {
    state.editing_key = Some(constant.name.clone());
    state.editing_value = format_localized_constant_value(&constant.value, locale_preset);
    state.editing_units = constant.units.clone().unwrap_or_default();
    state.editing_kind = InfoConstantKind::from_value(&constant.value);
    state.editing_focus_seen = false;
}

fn render_inline_constant_editor(
    ui: &mut egui::Ui,
    target: InlineConstantTarget<'_>,
    state: &mut InfoViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let info = target.info;
    let constant = target.constant;
    let locale_preset = target.locale_preset;
    let editor_frame = egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(1.5, tokens.accent))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::same(6));

    editor_frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong(&constant.name);
            for kind in [InfoConstantKind::Auto, InfoConstantKind::Number, InfoConstantKind::Date, InfoConstantKind::Text] {
                let kind_resp = ui.selectable_label(state.editing_kind == kind, kind.badge());
                let kind_wid = format!("info.const.kind.{}", kind.slug());
                meta.record_response(
                    ui,
                    &kind_resp,
                    FrameInspectEntryInput {
                        widget_id: &kind_wid,
                        role: "button",
                        label: kind.badge(),
                        command_id: None,
                        value: Some(json!(state.editing_kind == kind)),
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if kind_resp.clicked() && enabled {
                    state.editing_kind = kind;
                }
            }
        });

        let valid_input = validate_constant_draft(state.editing_kind, &state.editing_value, locale_preset);
        ui.horizontal(|ui| {
            let edit_resp = ui.add_enabled(
                enabled,
                TextEdit::singleline(&mut state.editing_value)
                    .id(Id::unique(("info_const_value", info.id.as_str(), &constant.name)))
                    .desired_width(120.0),
            );
            if !state.editing_focus_seen {
                edit_resp.request_focus();
                state.editing_focus_seen = true;
            }
            meta.record_response(
                ui,
                &edit_resp,
                FrameInspectEntryInput {
                    widget_id: "info.const.editor_value",
                    role: "textbox",
                    label: &constant.name,
                    command_id: Some("info.setConstant"),
                    value: Some(json!(state.editing_value)),
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );

            ui.add_enabled(enabled, TextEdit::singleline(&mut state.editing_units).hint_text("units").desired_width(48.0));

            let save_enabled = enabled && valid_input;
            let save_resp = ui.add_enabled(save_enabled, egui::Button::new("Save"));
            meta.record_response(
                ui,
                &save_resp,
                FrameInspectEntryInput {
                    widget_id: "info.const.editor_save",
                    role: "button",
                    label: "Save Constant",
                    command_id: Some("info.setConstant"),
                    value: None,
                    enabled: save_enabled,
                    disabled_reason: (!save_enabled).then_some("Enter a valid value for selected constant type"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );

            let cancel_resp = ui.add_enabled(enabled, egui::Button::new("Cancel"));
            let enter_commit = edit_resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let esc_cancel = ui.input(|i| i.key_pressed(Key::Escape));

            if esc_cancel || cancel_resp.clicked() {
                state.editing_key = None;
            } else if (save_resp.clicked() || enter_commit) && save_enabled {
                commit_constant_edit(info, constant, state, locale_preset, intents);
            }
        });

        if !valid_input {
            let err_msg = match state.editing_kind {
                InfoConstantKind::Number => "Expected finite number (e.g., 12.5)",
                InfoConstantKind::Date => "Expected ISO date YYYY-MM-DD",
                _ => "Invalid value",
            };
            let err_resp = ui.colored_label(tokens.link_red, err_msg);
            meta.record_response(
                ui,
                &err_resp,
                FrameInspectEntryInput {
                    widget_id: "info.const.editor_error",
                    role: "text",
                    label: err_msg,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        }
    });
}

fn validate_constant_draft(kind: InfoConstantKind, draft: &str, locale_preset: &LocalePreset) -> bool {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return true;
    }
    match kind {
        InfoConstantKind::Auto | InfoConstantKind::Text => true,
        InfoConstantKind::Number => parse_localized_number_input(trimmed, locale_preset).is_some(),
        InfoConstantKind::Date => is_valid_iso_date(trimmed),
    }
}

/// Validate strict 10-byte ASCII ISO date shape `YYYY-MM-DD` (`0001..=9999`, `01..=12`, `01..=31`).
fn is_valid_iso_date(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if !bytes[0..4].iter().all(u8::is_ascii_digit) || !bytes[5..7].iter().all(u8::is_ascii_digit) || !bytes[8..10].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let Ok(year) = raw[0..4].parse::<u16>() else { return false };
    let Ok(month) = raw[5..7].parse::<u8>() else { return false };
    let Ok(day) = raw[8..10].parse::<u8>() else { return false };
    (1..=9999).contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn commit_constant_edit(
    info: &InfoSheet,
    constant: &InfoConstant,
    state: &mut InfoViewEguiState,
    locale_preset: &LocalePreset,
    intents: &mut PendingUiIntents,
) {
    let trimmed = state.editing_value.trim();
    let units_val = state.editing_units.trim();
    let kind = state.editing_kind;
    let parsed_num = parse_localized_number_input(trimmed, locale_preset);

    let value_json = if trimmed.is_empty() {
        serde_json::Value::Null
    } else if kind == InfoConstantKind::Number || (kind == InfoConstantKind::Auto && parsed_num.is_some()) {
        json!(parsed_num.unwrap_or(0.0))
    } else {
        json!(trimmed)
    };

    let mut params = serde_json::Map::new();
    params.insert(String::from("sheet_id"), json!(info.id.as_str()));
    params.insert(String::from("name"), json!(constant.name));
    params.insert(String::from("value"), value_json);
    params.insert(String::from("units"), if units_val.is_empty() { serde_json::Value::Null } else { json!(units_val) });
    if let Some(kind_str) = kind.as_param()
        && !trimmed.is_empty()
    {
        params.insert(String::from("kind"), json!(kind_str));
    } else if kind == InfoConstantKind::Auto && is_valid_iso_date(trimmed) {
        params.insert(String::from("kind"), json!("date"));
    }

    intents.push_engine("info.setConstant", serde_json::Value::Object(params));
    state.editing_key = None;
}

fn sync_notes_draft_with_sheet(state: &mut InfoViewEguiState, info: &InfoSheet) {
    let same_sheet = state.notes_draft.as_ref().is_some_and(|(id, _)| id == &info.id);
    if !same_sheet {
        state.notes_draft = Some((info.id.clone(), info.notes_markdown.clone()));
        state.notes_dirty = false;
        state.notes_focus_seen = false;
    } else if !state.notes_dirty
        && let Some((_, text)) = state.notes_draft.as_mut()
        && text != &info.notes_markdown
    {
        *text = info.notes_markdown.clone();
    }
}

#[allow(clippy::too_many_arguments)]
fn render_notes_pane(
    ui: &mut egui::Ui,
    doc: &ProjectDocument,
    info: &InfoSheet,
    state: &mut InfoViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let padded = ui.available_rect_before_wrap().shrink(8.0);
    if !padded.is_positive() {
        return;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(padded), |ui| {
        ui.set_clip_rect(padded);
        let notes_heading = info_tr(tokens.locale, "info.notes_heading", "Experiment Notes");
        let mut commit_now = false;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let hdr_lbl = ui.add(egui::Label::new(RichText::new(notes_heading.as_ref()).strong().color(tokens.text_strong)).truncate());
            hdr_lbl.on_hover_text(notes_heading.as_ref());

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save_enabled = enabled && state.notes_dirty;
                let save_resp = ui.add_enabled(save_enabled, egui::Button::new("Save"));
                meta.record_response(
                    ui,
                    &save_resp,
                    FrameInspectEntryInput {
                        widget_id: "info.notes.save",
                        role: "button",
                        label: "Save Notes",
                        command_id: Some("info.setNote"),
                        value: None,
                        enabled: save_enabled,
                        disabled_reason: (!save_enabled).then_some("No unsaved notes changes"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if save_resp.clicked() && save_enabled {
                    commit_now = true;
                }

                let preview_lbl = if state.notes_preview_mode { "Edit" } else { "Preview" };
                let prev_resp = ui.add_enabled(enabled, egui::Button::new(preview_lbl).selected(state.notes_preview_mode));
                meta.record_response(
                    ui,
                    &prev_resp,
                    FrameInspectEntryInput {
                        widget_id: "info.notes.preview_toggle",
                        role: "button",
                        label: preview_lbl,
                        command_id: None,
                        value: Some(json!(state.notes_preview_mode)),
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if prev_resp.clicked() && enabled {
                    state.notes_preview_mode = !state.notes_preview_mode;
                }
            });
        });

        ui.add_space(2.0);
        // Markdown formatting toolbar
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let formats: &[(&str, &str, &str)] = &[
                ("info.format.heading", "H", "\n## Heading\n"),
                ("info.format.bold", "B", "**bold**"),
                ("info.format.italic", "I", "*italic*"),
                ("info.format.mono", "Code", "`code`"),
                ("info.format.bullet", "• List", "\n- Item"),
                ("info.format.numbered", "1. List", "\n1. Step"),
            ];
            for &(wid, lbl, snippet) in formats {
                let btn = ui.add_enabled(enabled && !state.notes_preview_mode, egui::Button::new(lbl).min_size(Vec2::new(26.0, 22.0)));
                meta.record_response(
                    ui,
                    &btn,
                    FrameInspectEntryInput {
                        widget_id: wid,
                        role: "button",
                        label: lbl,
                        command_id: None,
                        value: None,
                        enabled: enabled && !state.notes_preview_mode,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if btn.clicked()
                    && enabled
                    && let Some((_, draft)) = state.notes_draft.as_mut()
                {
                    draft.push_str(snippet);
                    state.notes_dirty = true;
                }
            }
        });

        ui.separator();

        let scroll_out =
            egui::ScrollArea::vertical().id_salt(("egui_shell.info.notes", info.id.as_str())).auto_shrink([false, false]).show(ui, |ui| {
                if state.notes_preview_mode {
                    let raw = state.notes_draft.as_ref().map_or(info.notes_markdown.as_str(), |(_, s)| s.as_str());
                    let expanded = expand_template_hooks(raw, |token| resolve_info_hook_preview(doc, info, token));
                    render_formatted_markdown_preview(ui, &expanded, tokens);
                } else if let Some((_, draft)) = state.notes_draft.as_mut() {
                    let avail_w = ui.available_width().max(80.0);
                    let edit = TextEdit::multiline(draft)
                        .id(Id::unique(("info_notes_editor", info.id.as_str())))
                        .desired_width(avail_w)
                        .desired_rows(10)
                        .hint_text("Protocol conditions, reagent lots, notebook references...");
                    let resp = ui.add_enabled(enabled, edit);
                    if resp.changed() {
                        state.notes_dirty = true;
                    }
                    if resp.has_focus() {
                        state.notes_focus_seen = true;
                    }
                    let cmd_enter = resp.has_focus() && ui.input(|i| (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(Key::Enter));
                    let focus_commit = state.notes_focus_seen && resp.lost_focus();
                    if (cmd_enter || focus_commit) && state.notes_dirty && enabled {
                        commit_now = true;
                    }
                    meta.record_response(
                        ui,
                        &resp,
                        FrameInspectEntryInput {
                            widget_id: "info.notes_editor",
                            role: "textbox",
                            label: notes_heading.as_ref(),
                            command_id: Some("info.setNote"),
                            value: Some(json!({ "len": draft.len(), "dirty": state.notes_dirty })),
                            enabled,
                            disabled_reason: (!enabled).then_some("modal dialog is active"),
                            z_layer: ZLayer::Workspace,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                }
            });
        let _ = scroll_out.inner_rect;

        if commit_now
            && state.notes_dirty
            && let Some((_, draft)) = &state.notes_draft
        {
            intents.push_engine("info.setNote", json!({ "sheet_id": info.id.as_str(), "text": draft, "append": false }));
            state.notes_dirty = false;
            state.notes_focus_seen = false;
        }
    });
}

fn resolve_info_hook_preview(doc: &ProjectDocument, current_info: &InfoSheet, token: &str) -> Option<String> {
    let trimmed = token.trim();
    if let Some(rest) = trimmed.strip_prefix("info:") {
        let mut parts = rest.splitn(2, ':');
        let first = parts.next()?.trim();
        if let Some(second) = parts.next() {
            return doc.info_sheet(&SheetId::new(first)).and_then(|s| s.get_constant(second)).map(|c| c.value.display_text());
        }
        return current_info.get_constant(first).map(|c| c.value.display_text());
    }
    if let Some(rest) = trimmed.strip_prefix("result:") {
        let parts: Vec<&str> = rest.split(':').map(str::trim).collect();
        if let [sid, key] = parts.as_slice() {
            return doc.result_sheet(&SheetId::new(*sid)).and_then(|rs| rs.lookup_scalar(key, None)).map(|v| format!("{v:.4}"));
        }
        if let [sid, ds, key] = parts.as_slice()
            && let Ok(idx) = ds.parse::<usize>()
        {
            return doc.result_sheet(&SheetId::new(*sid)).and_then(|rs| rs.lookup_scalar(key, Some(idx))).map(|v| format!("{v:.4}"));
        }
    }
    None
}

fn render_formatted_markdown_preview(ui: &mut egui::Ui, markdown: &str, tokens: &Tokens) {
    if markdown.trim().is_empty() {
        ui.colored_label(tokens.text_muted, "(No experimental notes recorded yet)");
        return;
    }
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            ui.add_space(4.0);
        } else if let Some(heading) = trimmed.strip_prefix("## ").or_else(|| trimmed.strip_prefix("# ")) {
            ui.label(RichText::new(heading).strong().size(15.0).color(tokens.text_strong));
        } else if let Some(bullet) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            ui.label(RichText::new(format!("• {bullet}")).color(tokens.text_primary));
        } else if let Some(code) = trimmed.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
            ui.label(RichText::new(code).monospace().color(tokens.text_primary));
        } else {
            ui.label(RichText::new(trimmed).color(tokens.text_primary));
        }
    }
}

fn paint_ellipsized_cell_text(ui: &egui::Ui, rect: Rect, text: &str, color: egui::Color32, size_pt: f32) -> bool {
    if text.is_empty() || rect.width() <= 2.0 || rect.height() <= 2.0 {
        return false;
    }
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::text::TextFormat { font_id: FontId::proportional(size_pt), color, ..Default::default() },
    );
    job.wrap = egui::text::TextWrapping { max_width: rect.width().max(1.0), max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let elided = galley.elided;
    let y = rect.center().y - galley.size().y * 0.5;
    ui.painter().with_clip_rect(rect).galley(pos2(rect.left(), y), galley, color);
    elided
}

const fn sanitize_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() { ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO) } else { DEFAULT_SPLIT_RATIO }
}

fn ensure_solid_scrollbar_style(ui: &mut egui::Ui, tokens: &Tokens) {
    let scroll = &mut ui.style_mut().spacing.scroll;
    *scroll = egui::style::ScrollStyle::solid();
    scroll.floating = false;
    scroll.bar_width = tokens.metrics.scroll_bar_thickness.max(10.0);
    scroll.bar_inner_margin = 2.0;
    scroll.bar_outer_margin = 2.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_shape_validation_accepts_valid_and_rejects_edges() {
        assert!(is_valid_iso_date("2026-11-20"));
        assert!(is_valid_iso_date("0001-01-01"));
        assert!(is_valid_iso_date("9999-12-31"));
        assert!(is_valid_iso_date("2026-01-31"));

        assert!(!is_valid_iso_date("+123-01-01"));
        assert!(!is_valid_iso_date("-123-01-01"));
        assert!(!is_valid_iso_date("0000-12-31"));
        assert!(!is_valid_iso_date("10000-01-01"));
        assert!(!is_valid_iso_date("2026-13-01"));
        assert!(!is_valid_iso_date("2026-00-01"));
        assert!(!is_valid_iso_date("2026-01-00"));
        assert!(!is_valid_iso_date("2026-01-32"));
        assert!(!is_valid_iso_date("2026-1-01"));
        assert!(!is_valid_iso_date("2026/01/01"));
        assert!(!is_valid_iso_date("2026-+1-01"));
        assert!(!is_valid_iso_date("2026-01-+1"));
        assert!(!is_valid_iso_date(""));
    }

    #[test]
    fn next_unused_constant_name_avoids_case_insensitive_collisions() {
        let mut info = InfoSheet::new_preseeded("info-test", "Test").unwrap();
        info.constants.clear();
        assert_eq!(next_unused_constant_name(&info), "Constant 1");

        info.constants.push(InfoConstant::new("Constant 2", ConstantValue::Number(2.0), None).unwrap());
        info.constants.push(InfoConstant::new("Constant 3", ConstantValue::Number(3.0), None).unwrap());
        assert_eq!(next_unused_constant_name(&info), "Constant 1");

        info.constants.push(InfoConstant::new("constant 1", ConstantValue::Number(1.0), None).unwrap());
        assert_eq!(next_unused_constant_name(&info), "Constant 4");
    }

    #[test]
    fn localized_number_draft_validation_accepts_comma_decimals_in_de_locale() {
        let de = LocalePreset::for_locale("de");
        assert!(validate_constant_draft(InfoConstantKind::Number, "1,5", &de));
        assert!(validate_constant_draft(InfoConstantKind::Number, "-1.234,5", &de));
        assert!(!validate_constant_draft(InfoConstantKind::Number, "abc", &de));
    }
}
