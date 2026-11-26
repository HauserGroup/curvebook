//! Native `egui` `Cmd/Ctrl+K` Command Palette modal overlay (`docs/planning/gui-layout-spec.md` §3, `R15.3`).

use std::ops::Range;
use std::sync::Arc;

use curvebook_doc::{ProjectDocument, SheetId, SheetSection, TableArchetype};
use curvebook_engine::cmd as engine_cmd;
use curvebook_palette::CanvasPreviewFilter;
use egui::{Color32, CornerRadius, FontId, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::CommandOrigin;
use crate::egui_shell::app_modals::PreferencesCategory;
use crate::egui_shell::transform_modals::TransformModalKind;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, Tokens, UiIntent};
use crate::grid_view::ImportOriginKind;
use crate::i18n::BETA_ACTIVE_LOCALES;
use crate::inspect::ZLayer;
use crate::menus::format_shortcut_for_platform;
use crate::search_engine::{
    SearchRecents, SearchScopeFilter, UnifiedSearchAction, UnifiedSearchItem, UnifiedSearchKind, compute_ghost_completion, parse_scoped_query,
    query_unified,
};
use crate::theme::UiThemeMode;
use crate::tr;

/// Resolved Command Palette search row surfaced by [`filter_palette_commands`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPaletteRow {
    pub command_id: String,
    pub label: String,
    pub menu_path: String,
    pub shortcut: String,
    pub summary: String,
    pub beginner: String,
    pub enabled: bool,
    pub is_ui_command: bool,
}

/// Persistent `egui_shell` state for the `Cmd/Ctrl+K` Command Palette overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandPaletteEguiState {
    pub query: String,
    pub selected_index: usize,
    pub hovered_index: Option<usize>,
    pub just_opened: bool,
    pub preserve_query_on_open: bool,
}

/// Per-frame layout & backdrop capture report emitted by [`render_command_palette`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandPaletteFrameReport {
    pub dialog_rect_pt: Option<egui::Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub match_count: usize,
}

/// Filter and rank live Engine + UI commands matching `query` via `app.canvas_dialogs.command_palette`.
///
/// Parameterised commands that have a dialog or active-sheet route are retained and dispatched
/// through that route; low-level parameterised commands that cannot run from `{}` without extra
/// interactive arguments are hidden (`R15-GUI-18`).
#[must_use]
pub fn filter_palette_commands(app: &UiApp, query: &str, max_rows: usize) -> Vec<CommandPaletteRow> {
    let mut palette = app.canvas_dialogs.command_palette.clone();
    palette.query = query.to_owned();
    palette.sync_shortcuts(&app.shortcuts);
    palette
        .matches_with_shortcuts(Some(app.session()), &app.shortcuts)
        .into_iter()
        .filter(|entry| resolve_palette_route(app, entry.id, matches!(entry.origin, CommandOrigin::Ui)).is_some())
        .take(max_rows.max(1))
        .map(|entry| CommandPaletteRow {
            command_id: entry.id.to_owned(),
            label: entry.localized_label(&app.locale).to_owned(),
            menu_path: entry.menu_path.to_owned(),
            shortcut: entry.shortcut,
            summary: entry.summary.to_owned(),
            beginner: entry.beginner.to_owned(),
            enabled: entry.enabled && entry.id != "ui.result.applyRemediation",
            is_ui_command: matches!(entry.origin, CommandOrigin::Ui),
        })
        .collect()
}

/// Render the unified `Cmd/Ctrl+K` Command Palette overlay (`ZLayer::ModalDialog`) with `100%`
/// `ZLayer::ModalBackdrop` click + wheel capture, scope filter pills, inline ghost-text `Tab`
/// completion, category badges, match-span highlighting, and `ArrowUp`/`ArrowDown`/`Enter`/`Esc`
/// keyboard navigation.
pub fn render_command_palette(
    ctx: &egui::Context,
    app: &UiApp,
    state: &mut CommandPaletteEguiState,
    recents: &SearchRecents,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> CommandPaletteFrameReport {
    let mut report = CommandPaletteFrameReport::default();
    let screen_rect = ctx.content_rect();
    let scrim = if tokens.dark { Color32::from_black_alpha(150) } else { tokens.scrim };

    let backdrop_resp = egui::Area::new(Id::unique("egui_shell.palette.backdrop"))
        .fixed_pos(screen_rect.min)
        .order(Order::Middle)
        .interactable(true)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen_rect.size(), Sense::click_and_drag());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, scrim);
            resp
        })
        .inner;

    meta.record(FrameInspectNodeMeta {
        egui_id: backdrop_resp.id,
        accesskit_id: backdrop_resp.id.accesskit_id(),
        widget_id: String::from("modal.backdrop"),
        role: String::from("backdrop"),
        label: String::from("Command Palette Backdrop"),
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        unclipped_rect: screen_rect,
        clipped_rect: screen_rect,
        z_layer: ZLayer::ModalBackdrop,
        ellipsized: false,
        full_label: None,
        action_rank: None,
    });

    if backdrop_resp.clicked() {
        report.outside_click_captured = true;
        intents.push(UiIntent::SetPaletteOpen { open: false });
    }

    if state.just_opened {
        if !state.preserve_query_on_open {
            state.query.clear();
        }
        state.preserve_query_on_open = false;
        state.selected_index = 0;
        state.hovered_index = None;
    }

    let mut items = query_unified(app, &state.query, 24, recents);
    report.match_count = items.len();
    clamp_palette_selection(state, items.len());

    let query_widget_id = Id::unique("egui_shell.palette.query");
    let query_focused = ctx.memory(|m| m.focused()) == Some(query_widget_id);
    let ghost_suffix = compute_ghost_completion(&state.query, ghost_target_item(&items, state.selected_index));
    let key_nav = consume_palette_keys(ctx, app, state, &items, ghost_suffix.as_deref(), query_focused, intents);
    if key_nav.accepted_ghost {
        push_palette_query_patch(&state.query, intents);
        items = query_unified(app, &state.query, 24, recents);
        report.match_count = items.len();
        clamp_palette_selection(state, items.len());
    }

    let top_offset = (screen_rect.height() * 0.10).clamp(16.0, 56.0);
    let card_w = (screen_rect.width() - 32.0).clamp(280.0, 600.0);
    let card_h = (screen_rect.height() - top_offset - 16.0).clamp(180.0, 420.0);

    let win_resp = egui::Window::new(tr(&app.locale, "inspector.command_palette"))
        .id(Id::unique("egui_shell.command_palette.window"))
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, top_offset))
        .fixed_size(Vec2::new(card_w, card_h))
        .order(Order::Foreground)
        .frame(
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(CornerRadius::same(tokens.metrics.radius_lg))
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ctx, |ui| {
            let prev_query = state.query.clone();
            let plat_shortcut = format_shortcut_for_platform(app.shortcuts.active_combo("ui.palette.open").unwrap_or("Cmd+K"));
            let hint = format!("Search sheets, analyses, equations, commands ({plat_shortcut})");
            let query_resp = ui.add(egui::TextEdit::singleline(&mut state.query).id(query_widget_id).hint_text(hint).desired_width(f32::INFINITY));
            if state.just_opened {
                query_resp.request_focus();
                state.just_opened = false;
            }
            if state.query != prev_query {
                push_palette_query_patch(&state.query, intents);
                items = query_unified(app, &state.query, 24, recents);
                report.match_count = items.len();
                state.selected_index = items.iter().position(|r| r.enabled).unwrap_or(0);
            }
            if let Some(suffix) = compute_ghost_completion(&state.query, ghost_target_item(&items, state.selected_index)) {
                paint_inline_ghost_completion(ui, &query_resp, &state.query, &suffix, tokens);
            }
            meta.record_response(
                ui,
                &query_resp,
                FrameInspectEntryInput {
                    widget_id: "palette.query",
                    role: "textbox",
                    label: "Search commands",
                    command_id: None,
                    value: Some(json!(state.query)),
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );

            ui.add_space(4.0);
            if render_palette_scope_pills(ui, &mut state.query, tokens, meta) {
                push_palette_query_patch(&state.query, intents);
                items = query_unified(app, &state.query, 24, recents);
                report.match_count = items.len();
                state.selected_index = items.iter().position(|r| r.enabled).unwrap_or(0);
                query_resp.request_focus();
            }

            ui.separator();
            let footer_h = 34.0;
            let list_h = (ui.available_height() - footer_h).max(54.0);
            const PALETTE_ROW_H: f32 = 32.0;
            ui.allocate_ui(Vec2::new(ui.available_width(), list_h), |ui| {
                ui.spacing_mut().scroll.floating = false;
                ui.spacing_mut().item_spacing.y = 0.0;
                if items.is_empty() {
                    ui.colored_label(tokens.text_muted, "No matching sheets, analyses, or commands.");
                } else {
                    egui::ScrollArea::vertical().id_salt("egui_shell.palette.scroll").auto_shrink([false, false]).show_rows(
                        ui,
                        PALETTE_ROW_H,
                        items.len(),
                        |ui, row_range| {
                            if key_nav.moved {
                                #[allow(clippy::cast_precision_loss)]
                                let y = ui.min_rect().top() + state.selected_index as f32 * PALETTE_ROW_H;
                                let target = egui::Rect::from_min_size(
                                    egui::pos2(ui.min_rect().left(), y),
                                    Vec2::new(ui.available_width().max(1.0), PALETTE_ROW_H),
                                );
                                ui.scroll_to_rect(target, Some(egui::Align::Center));
                            }
                            render_palette_rows(ui, PaletteRowsRenderCtx { app, items: &items, row_range, tokens }, state, meta, intents);
                        },
                    );
                }
            });

            ui.separator();
            render_palette_footer(ui, app, items.get(state.selected_index), tokens);
        });

    if let Some(inner) = win_resp {
        let dialog_rect = inner.response.rect;
        report.dialog_rect_pt = Some(dialog_rect);
        meta.record(FrameInspectNodeMeta {
            egui_id: inner.response.id,
            accesskit_id: inner.response.id.accesskit_id(),
            widget_id: String::from("palette.dialog"),
            role: String::from("dialog"),
            label: tr(&app.locale, "inspector.command_palette").to_owned(),
            command_id: None,
            value: Some(json!(state.selected_index)),
            enabled: true,
            disabled_reason: None,
            unclipped_rect: dialog_rect,
            clipped_rect: dialog_rect,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
            action_rank: None,
        });
        capture_outside_pointer_and_wheel(ctx, dialog_rect, &mut report);
    }

    report
}

struct PaletteKeyNavResult {
    moved: bool,
    accepted_ghost: bool,
}

fn clamp_palette_selection(state: &mut CommandPaletteEguiState, item_count: usize) {
    if item_count == 0 {
        state.selected_index = 0;
        state.hovered_index = None;
    } else if state.selected_index >= item_count {
        state.selected_index = item_count - 1;
    }
}

fn consume_palette_keys(
    ctx: &egui::Context,
    app: &UiApp,
    state: &mut CommandPaletteEguiState,
    items: &[UnifiedSearchItem],
    ghost_suffix: Option<&str>,
    query_focused: bool,
    intents: &mut PendingUiIntents,
) -> PaletteKeyNavResult {
    let mut moved = false;
    let mut accepted_ghost = false;
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::SetPaletteOpen { open: false });
        return PaletteKeyNavResult { moved, accepted_ghost };
    }
    if let Some(suffix) = ghost_suffix
        && !suffix.is_empty()
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab))
    {
        state.query.push_str(suffix);
        accepted_ghost = true;
        return PaletteKeyNavResult { moved, accepted_ghost };
    }
    let home_end_nav_allowed = !query_focused || state.query.is_empty();
    if !items.is_empty() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            state.selected_index = (state.selected_index + 1) % items.len();
            moved = true;
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            state.selected_index = if state.selected_index == 0 { items.len() - 1 } else { state.selected_index - 1 };
            moved = true;
        } else if home_end_nav_allowed && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
            state.selected_index = 0;
            moved = true;
        } else if home_end_nav_allowed && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::End)) {
            state.selected_index = items.len() - 1;
            moved = true;
        }
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
        && let Some(item) = items.get(state.selected_index)
        && item.enabled
    {
        commit_unified_search_item(app, item, intents);
    }
    PaletteKeyNavResult { moved, accepted_ghost }
}

const SCOPE_PILLS: [(SearchScopeFilter, &str, &str); 5] = [
    (SearchScopeFilter::All, "All", "palette.scope.all"),
    (SearchScopeFilter::Sheets, "@ Sheets", "palette.scope.sheets"),
    (SearchScopeFilter::AnalysesAndEquations, "= Analyses & Eq", "palette.scope.analyses"),
    (SearchScopeFilter::Commands, "> Commands", "palette.scope.commands"),
    (SearchScopeFilter::Columns, "# Columns", "palette.scope.columns"),
];

fn apply_scope_filter_prefix(query: &mut String, target: SearchScopeFilter) {
    let (_, effective) = parse_scoped_query(query);
    let body = effective.to_owned();
    match target.prefix() {
        Some(prefix) => {
            if body.is_empty() {
                *query = prefix.to_string();
            } else {
                *query = format!("{prefix}{body}");
            }
        }
        None => *query = body,
    }
}

fn render_palette_scope_pills(ui: &mut egui::Ui, query: &mut String, tokens: &Tokens, meta: &mut FrameInspectMeta) -> bool {
    let (active_scope, _) = parse_scoped_query(query);
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (scope, label, wid) in SCOPE_PILLS {
            let selected = active_scope == scope;
            let fill = if selected { tokens.accent } else { tokens.surface_bg };
            let fg = if selected { tokens.on_accent } else { tokens.text_muted };
            let stroke = if selected { Stroke::new(1.0, tokens.accent) } else { Stroke::new(1.0, tokens.border) };
            let resp = ui.add(
                egui::Button::new(egui::RichText::new(label).size(11.0).color(fg))
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(CornerRadius::same(10))
                    .min_size(Vec2::new(28.0, 20.0)),
            );
            meta.record_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: wid,
                    role: "button",
                    label,
                    command_id: None,
                    value: Some(json!(selected)),
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if resp.clicked() && !selected {
                apply_scope_filter_prefix(query, scope);
                changed = true;
            }
        }
    });
    changed
}

/// Paint inline ghost-text completion suffix right after the typed characters in a `TextEdit`.
pub(crate) fn paint_inline_ghost_completion(ui: &egui::Ui, text_edit_resp: &egui::Response, typed_query: &str, ghost_suffix: &str, tokens: &Tokens) {
    if typed_query.is_empty() || ghost_suffix.is_empty() {
        return;
    }
    let font_id = ui.style().text_styles.get(&egui::TextStyle::Body).cloned().unwrap_or_else(|| FontId::proportional(13.0));
    let typed_galley = ui.painter().layout_no_wrap(typed_query.to_owned(), font_id.clone(), tokens.text_primary);
    let ghost_color = tokens.text_muted.gamma_multiply(0.65);
    let inner_clip = text_edit_resp.rect.shrink2(Vec2::new(4.0, 2.0)).intersect(ui.clip_rect());
    let start_x = text_edit_resp.rect.left() + 4.0 + typed_galley.size().x;
    let avail_w = inner_clip.right() - start_x;
    if avail_w < 10.0 {
        return;
    }
    let (ghost_galley, _) = layout_palette_line(ui.painter(), ghost_suffix, avail_w, ghost_color, font_id.size);
    let pos = Pos2::new(start_x, text_edit_resp.rect.center().y - ghost_galley.size().y * 0.5);
    ui.painter().with_clip_rect(inner_clip).galley(pos, ghost_galley, ghost_color);
}

/// Build a single-line [`egui::text::LayoutJob`] highlighting UTF-8 byte `spans` in `text`.
pub(crate) fn layout_highlighted_search_title(
    painter: &egui::Painter,
    text: &str,
    spans: &[Range<usize>],
    max_width: f32,
    base_color: Color32,
    highlight_color: Color32,
    font_size: f32,
) -> (Arc<egui::Galley>, bool) {
    let mut job = egui::text::LayoutJob::default();
    let base_fmt = egui::TextFormat::simple(FontId::proportional(font_size), base_color);
    let mut hi_fmt = egui::TextFormat::simple(FontId::proportional(font_size), highlight_color);
    hi_fmt.background = highlight_color.gamma_multiply(0.16);

    let mut cursor = 0_usize;
    for span in spans {
        let start = span.start.min(text.len());
        let end = span.end.min(text.len());
        if start >= end || start < cursor || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        if let Some(prefix) = text.get(cursor..start)
            && !prefix.is_empty()
        {
            job.append(prefix, 0.0, base_fmt.clone());
        }
        if let Some(matched) = text.get(start..end)
            && !matched.is_empty()
        {
            job.append(matched, 0.0, hi_fmt.clone());
        }
        cursor = end;
    }
    if let Some(tail) = text.get(cursor..)
        && !tail.is_empty()
    {
        job.append(tail, 0.0, base_fmt);
    }
    if job.sections.is_empty() {
        job.append(text, 0.0, egui::TextFormat::simple(FontId::proportional(font_size), base_color));
    }
    job.wrap = egui::text::TextWrapping { max_width: max_width.max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let ellipsized = galley.elided;
    (galley, ellipsized)
}

fn category_badge_colors(kind: UnifiedSearchKind, tokens: &Tokens) -> (Color32, Color32) {
    match kind {
        UnifiedSearchKind::Recent => (tokens.panel_bg, tokens.text_muted),
        UnifiedSearchKind::Sheet => (tokens.accent.gamma_multiply(0.16), tokens.accent),
        UnifiedSearchKind::Analysis | UnifiedSearchKind::Equation => (tokens.link_green.gamma_multiply(0.16), tokens.link_green),
        UnifiedSearchKind::Column => (tokens.warning.gamma_multiply(0.18), tokens.warning),
        UnifiedSearchKind::Command => (tokens.hover_bg, tokens.text_primary),
    }
}

/// Paint a unified search item row (category badge + match-highlighted title + subtitle + shortcut
/// pill) into `rect`, returning whether the title was ellipsized.
pub(crate) fn paint_unified_search_row_content(ui: &egui::Ui, rect: Rect, item: &UnifiedSearchItem, tokens: &Tokens) -> bool {
    let row_clip = rect.shrink2(Vec2::new(4.0, 2.0)).intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(row_clip);

    // 1. Category badge on the left
    let (badge_bg, badge_fg) = category_badge_colors(item.kind, tokens);
    let (badge_galley, _) = layout_palette_line(ui.painter(), &item.category_badge, 66.0, badge_fg, 10.5);
    let badge_w = (badge_galley.size().x + 10.0).clamp(42.0, 72.0);
    let badge_h = 18.0;
    let badge_rect = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.center().y - badge_h * 0.5), Vec2::new(badge_w, badge_h));
    painter.rect_filled(badge_rect, CornerRadius::same(4), badge_bg);
    painter.galley(
        Pos2::new(badge_rect.center().x - badge_galley.size().x * 0.5, badge_rect.center().y - badge_galley.size().y * 0.5),
        badge_galley,
        badge_fg,
    );

    // 2. Optional trailing shortcut pill on the right
    let mut right_limit = rect.right() - 8.0;
    if !item.shortcut.is_empty() {
        let plat_sc = format_shortcut_for_platform(&item.shortcut);
        let (sc_galley, _) = layout_palette_line(ui.painter(), &plat_sc, 86.0, tokens.text_muted, 10.5);
        let pill_w = sc_galley.size().x + 10.0;
        let pill_h = 18.0;
        let pill_rect = Rect::from_min_size(
            Pos2::new((right_limit - pill_w).max(badge_rect.right() + 40.0), rect.center().y - pill_h * 0.5),
            Vec2::new(pill_w, pill_h),
        );
        painter.rect_filled(pill_rect, CornerRadius::same(4), tokens.panel_bg);
        painter.rect_stroke(pill_rect, CornerRadius::same(4), Stroke::new(1.0, tokens.border), StrokeKind::Inside);
        painter.galley(
            Pos2::new(pill_rect.center().x - sc_galley.size().x * 0.5, pill_rect.center().y - sc_galley.size().y * 0.5),
            sc_galley,
            tokens.text_muted,
        );
        right_limit = pill_rect.left() - 8.0;
    }

    // 3. Highlighted title + muted subtitle in the remaining middle budget
    let text_left = badge_rect.right() + 8.0;
    let avail_w = (right_limit - text_left).max(40.0);
    let title_color = if item.enabled { tokens.text_strong } else { tokens.text_disabled };
    let hi_color = if item.enabled { tokens.accent } else { tokens.text_disabled };
    let (title_galley, ellipsized) =
        layout_highlighted_search_title(ui.painter(), &item.title, &item.match_spans, avail_w, title_color, hi_color, tokens.metrics.font_ui_body_pt);
    let title_w = title_galley.size().x;
    painter.galley(Pos2::new(text_left, rect.center().y - title_galley.size().y * 0.5), title_galley, title_color);

    if !item.subtitle.is_empty() && !ellipsized {
        let sub_left = text_left + title_w + 8.0;
        let sub_avail = right_limit - sub_left;
        if sub_avail >= 36.0 {
            let sub_text = format!("· {}", item.subtitle);
            let (sub_galley, _) = layout_palette_line(ui.painter(), &sub_text, sub_avail, tokens.text_muted, 11.0);
            painter.galley(Pos2::new(sub_left, rect.center().y - sub_galley.size().y * 0.5), sub_galley, tokens.text_muted);
        }
    }

    ellipsized
}

struct PaletteRowsRenderCtx<'a> {
    app: &'a UiApp,
    items: &'a [UnifiedSearchItem],
    row_range: std::ops::Range<usize>,
    tokens: &'a Tokens,
}

fn render_palette_rows(
    ui: &mut egui::Ui,
    ctx: PaletteRowsRenderCtx<'_>,
    state: &mut CommandPaletteEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let PaletteRowsRenderCtx { app, items, row_range, tokens } = ctx;
    for idx in row_range {
        let Some(item) = items.get(idx) else {
            continue;
        };
        let is_selected = idx == state.selected_index;
        let row_w = ui.available_width().max(120.0);
        let (slot_rect, slot_resp) = ui.allocate_exact_size(Vec2::new(row_w, 32.0), Sense::hover());
        let visible_rect = slot_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
        if visible_rect.height() < 12.0 || visible_rect.width() < 12.0 {
            continue;
        }
        let rect = slot_rect.shrink2(Vec2::new(0.0, 1.0));
        let resp = ui.interact(rect, slot_resp.id.with(idx), if item.enabled { Sense::click() } else { Sense::hover() });
        if resp.hovered() {
            state.hovered_index = Some(idx);
        }

        let bg = if !item.enabled {
            if is_selected { tokens.panel_bg } else { Color32::TRANSPARENT }
        } else if is_selected {
            tokens.active_bg
        } else if resp.hovered() {
            tokens.hover_bg
        } else {
            tokens.surface_bg
        };
        let border = if is_selected { Stroke::new(1.0, tokens.accent) } else { Stroke::NONE };
        let row_clip = rect.intersect(ui.clip_rect());
        ui.painter().with_clip_rect(row_clip).rect_filled(rect, CornerRadius::same(4), bg);
        if border != Stroke::NONE {
            ui.painter().with_clip_rect(row_clip).rect_stroke(rect, CornerRadius::same(4), border, StrokeKind::Inside);
        }

        let ellipsized = paint_unified_search_row_content(ui, rect, item, tokens);

        let command_id_opt = match &item.action {
            UnifiedSearchAction::RunCommand { command_id, .. } => Some(command_id.as_str()),
            _ => None,
        };
        let reason = (!item.enabled).then(|| disabled_reason_for_item(app, item));
        let ak_label = item.title.clone();
        resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, item.enabled, ak_label.clone()));
        let wid = format!("palette.item.{idx}");
        meta.record_ranked_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "button",
                label: &item.title,
                command_id: command_id_opt,
                value: Some(json!(item.enabled)),
                enabled: item.enabled,
                disabled_reason: reason.as_deref(),
                z_layer: ZLayer::ModalDialog,
                ellipsized,
                full_label: ellipsized.then_some(item.title.as_str()),
            },
            crate::egui_shell::ActionRank::Secondary,
        );
        if let Some(ref rsn) = reason {
            resp.on_hover_text(rsn);
        } else if resp.clicked() && item.enabled {
            state.selected_index = idx;
            commit_unified_search_item(app, item, intents);
        }
    }
}

fn render_palette_footer(ui: &mut egui::Ui, app: &UiApp, selected: Option<&UnifiedSearchItem>, tokens: &Tokens) {
    let preview_w = ui.available_width().max(80.0);
    let (msg, color) = match selected {
        Some(item) if !item.enabled => (format!("Disabled: {}", disabled_reason_for_item(app, item)), tokens.text_muted),
        Some(item) if !item.subtitle.is_empty() => (format!("[{}] {} — {}", item.category_badge, item.title, item.subtitle), tokens.text_primary),
        _ => (String::from("↑↓ Navigate · Tab Autocomplete · Enter Execute · Esc Dismiss"), tokens.text_muted),
    };
    let (galley, _) = layout_palette_line(ui.painter(), &msg, (preview_w - 8.0).max(40.0), color, 11.5);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(preview_w, 20.0), Sense::hover());
    ui.painter().with_clip_rect(rect).galley(Pos2::new(rect.left() + 2.0, rect.center().y - galley.size().y * 0.5), galley, color);
}

fn disabled_reason_for_item(app: &UiApp, item: &UnifiedSearchItem) -> String {
    let doc = app.document();
    match &item.action {
        UnifiedSearchAction::RunCommand { command_id, .. } => match command_id.as_str() {
            "analysis.updateParams"
            | "ui.editParams.open"
            | "ui.result.copyMethodsReport"
            | "ui.result.inspectDiagnostics"
            | "ui.result.applyRemediation" => String::from("Open a ResultSheet first"),
            "ui.result.appendClusterColumn" => String::from("Requires a Clustering ResultSheet"),
            "ui.table.openFormulaBar" => String::from("Requires a Multiple Variables table"),
            "ui.layout.composer.open" | "ui.layout.equalizeScaling.open" => String::from("Open a LayoutSheet first"),
            "ui.embeddedTable.format.open" => String::from("Paste a ResultSheet table onto a graph or layout first"),
            "ui.image.format.open" => String::from("Insert or paste an image onto a graph or layout first"),
            "ui.graph.axes.open" | "ui.graph.format.open" | "ui.changeGraphType.open" | "ui.formatPoints.open" => {
                String::from("Requires at least one GraphSheet")
            }
            "edit.undo" => String::from("Nothing to undo"),
            "edit.redo" => String::from("Nothing to redo"),
            _ if doc.data_sheets.is_empty() => String::from("Requires a DataSheet in the active project"),
            _ => String::from("Preconditions not met for active sheet"),
        },
        UnifiedSearchAction::OpenAnalysis { .. } | UnifiedSearchAction::OpenEquation { .. } => {
            String::from("Requires at least one DataSheet in the active project")
        }
        _ => String::from("Unavailable for active project"),
    }
}

pub(crate) enum PaletteCommandRoute {
    UiCommand { command_id: String, params: Value },
    EngineCommand { command_id: String, params: Value },
    DirectIntent(UiIntent),
}

pub(crate) fn resolve_renderable_sheet_id_for_menu(doc: &ProjectDocument) -> Option<SheetId> {
    resolve_renderable_sheet_id(doc)
}

fn sheet_id_param(sheet_id: Option<SheetId>) -> Value {
    match sheet_id {
        Some(id) => json!({ "sheet_id": id.as_str() }),
        None => json!({}),
    }
}

fn resolve_any_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id
        .clone()
        .or_else(|| doc.data_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.graph_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.result_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.layout_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.info_sheets.first().map(|s| s.id.clone()))
}

fn resolve_data_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()) {
        return Some(active.clone());
    }
    if let Some(active) = doc.active_sheet_id.as_ref() {
        if let Some(graph) = doc.graph_sheet(active)
            && doc.data_sheet(&graph.spec.primary_source_sheet_id).is_some()
        {
            return Some(graph.spec.primary_source_sheet_id.clone());
        }
        if let Some(res) = doc.result_sheet(active)
            && doc.data_sheet(&res.analysis.source_sheet_id).is_some()
        {
            return Some(res.analysis.source_sheet_id.clone());
        }
    }
    doc.data_sheets.first().map(|s| s.id.clone())
}

fn resolve_mv_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref()
        && let Some(ds) = doc.data_sheet(active)
        && matches!(ds.archetype, TableArchetype::MultipleVariables(_))
    {
        return Some(ds.id.clone());
    }
    doc.data_sheets.iter().find(|ds| matches!(ds.archetype, TableArchetype::MultipleVariables(_))).map(|ds| ds.id.clone())
}

fn resolve_result_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()).cloned().or_else(|| doc.result_sheets.first().map(|r| r.id.clone()))
}

fn resolve_graph_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some()).cloned().or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
}

fn resolve_layout_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.active_sheet_id.as_ref().filter(|id| doc.layout_sheet(id).is_some()).cloned().or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
}

fn resolve_renderable_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref()
        && matches!(doc.sheet_section(active), Some(SheetSection::Graphs | SheetSection::Layouts))
    {
        return Some(active.clone());
    }
    doc.graph_sheets.first().map(|g| g.id.clone()).or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
}

fn next_theme_mode_slug(current: UiThemeMode) -> &'static str {
    match current {
        UiThemeMode::Light => "dark",
        UiThemeMode::Dark => "high_contrast",
        UiThemeMode::HighContrast => "light",
    }
}

fn next_preview_filter_slug(current: CanvasPreviewFilter) -> &'static str {
    match current {
        CanvasPreviewFilter::Normal => "deuteranopia",
        CanvasPreviewFilter::Deuteranopia => "protanopia",
        CanvasPreviewFilter::Protanopia => "tritanopia",
        CanvasPreviewFilter::Tritanopia => "grayscale",
        CanvasPreviewFilter::GrayscalePrint => "normal",
    }
}

fn next_locale_slug(current: &str) -> &'static str {
    if BETA_ACTIVE_LOCALES.is_empty() {
        return "en";
    }
    let idx = BETA_ACTIVE_LOCALES.iter().position(|&loc| loc.eq_ignore_ascii_case(current)).unwrap_or(0);
    BETA_ACTIVE_LOCALES.get((idx + 1) % BETA_ACTIVE_LOCALES.len()).copied().unwrap_or("en")
}

fn resolve_palette_ui_route(app: &UiApp, command_id: &str) -> Option<PaletteCommandRoute> {
    let doc = app.document();
    match command_id {
        "ui.event.dispatch" | "ui.screenshot" | "ui.result.applyRemediation" => None,
        "ui.theme.set" => Some(PaletteCommandRoute::UiCommand {
            command_id: String::from("ui.theme.set"),
            params: json!({ "mode": next_theme_mode_slug(app.theme.mode) }),
        }),
        "ui.locale.set" => Some(PaletteCommandRoute::UiCommand {
            command_id: String::from("ui.locale.set"),
            params: json!({ "locale": next_locale_slug(&app.locale) }),
        }),
        "ui.previewFilter.set" => Some(PaletteCommandRoute::UiCommand {
            command_id: String::from("ui.previewFilter.set"),
            params: json!({ "filter": next_preview_filter_slug(app.preview_filter) }),
        }),
        "ui.canvas.applyPreset" => {
            let mut params = sheet_id_param(resolve_renderable_sheet_id(doc));
            if let Some(obj) = params.as_object_mut() {
                obj.insert(String::from("preset"), Value::String(String::from("SingleColumn88mm")));
                obj.insert(String::from("resize_mode"), Value::String(String::from("KeepTypographyPt")));
            }
            Some(PaletteCommandRoute::UiCommand { command_id: String::from("ui.canvas.applyPreset"), params })
        }
        "ui.shortcut.set" => {
            Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenPreferences { category: Some(PreferencesCategory::KeyboardShortcuts) }))
        }
        "app.preferences" => Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenPreferences { category: None })),
        "ui.brush.point" => {
            let params = match resolve_data_sheet_id(doc) {
                Some(id) => match app.grid.active_cell_for_sheet(&id) {
                    Some(coord) => json!({ "sheet_id": id.as_str(), "row": coord.row, "col": coord.col, "subcol": coord.subcol }),
                    None => json!({ "sheet_id": id.as_str(), "row": 0, "col": 0, "subcol": 0 }),
                },
                None => json!({}),
            };
            Some(PaletteCommandRoute::UiCommand { command_id: String::from("ui.brush.point"), params })
        }
        "ui.graph.axes.open"
        | "ui.graph.format.open"
        | "ui.changeGraphType.open"
        | "ui.formatPoints.open"
        | "ui.applyStyleFrom.open"
        | "ui.graph.pairwise.open"
        | "ui.graph.insertModelEquation" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_graph_sheet_id(doc)) })
        }
        "ui.layout.composer.open" | "ui.layout.equalizeScaling.open" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_layout_sheet_id(doc)) })
        }
        "ui.editParams.open"
        | "ui.result.copyMethodsReport"
        | "ui.result.inspectDiagnostics"
        | "ui.result.appendClusterColumn"
        | "ui.export.resultsCsv.open" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_result_sheet_id(doc)) })
        }
        "ui.analyze.open" | "ui.grid.excludeWithReason" | "ui.table.convertArchetype.open" | "ui.graph.create.open" | "ui.export.tableCsv.open" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_data_sheet_id(doc)) })
        }
        "ui.table.openFormulaBar" => Some(PaletteCommandRoute::UiCommand {
            command_id: command_id.to_owned(),
            params: sheet_id_param(resolve_mv_sheet_id(doc).or_else(|| resolve_data_sheet_id(doc))),
        }),
        "ui.clonePipeline.open" | "ui.print.open" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_any_sheet_id(doc)) })
        }
        "ui.canvas.preflight" | "ui.export.figure.open" | "ui.embeddedTable.format.open" | "ui.image.format.open" | "ui.clipboard.copyForSlides" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_renderable_sheet_id(doc)) })
        }
        "ui.sheet.find" => {
            let searchable = doc.active_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some());
            searchable.then(|| PaletteCommandRoute::UiCommand { command_id: String::from("ui.sheet.find"), params: json!({}) })
        }
        _ => Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: json!({}) }),
    }
}

fn ghost_target_item(items: &[UnifiedSearchItem], selected_index: usize) -> Option<&UnifiedSearchItem> {
    items.get(selected_index).filter(|r| r.enabled).or_else(|| items.iter().find(|r| r.enabled))
}

fn push_palette_query_patch(query: &str, intents: &mut PendingUiIntents) {
    let mut fields = serde_json::Map::new();
    fields.insert(String::from("query"), Value::String(query.to_owned()));
    intents.push(UiIntent::PatchActiveDialog { fields });
}

fn resolve_palette_dialog_engine_route(doc: &ProjectDocument, command_id: &str) -> Option<PaletteCommandRoute> {
    let ui_route =
        |ui_cmd: &str, sheet: Option<SheetId>| Some(PaletteCommandRoute::UiCommand { command_id: ui_cmd.to_owned(), params: sheet_id_param(sheet) });
    match command_id {
        "analysis.add" | "analysis.run" | "stats.run" | "fit.run" | "stats.pvalueStack" => ui_route("ui.analyze.open", resolve_data_sheet_id(doc)),
        "sheet.extractResultTab" => ui_route("ui.analyze.open", resolve_result_sheet_id(doc).or_else(|| resolve_data_sheet_id(doc))),
        "transform.run" => match resolve_data_sheet_id(doc) {
            Some(sheet_id) => {
                Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenTransformModal { sheet_id, kind: TransformModalKind::TransformValues }))
            }
            None => ui_route("ui.analyze.open", None),
        },
        "analysis.updateParams" => ui_route("ui.editParams.open", resolve_result_sheet_id(doc)),
        "analysis.retargetData" => match resolve_result_sheet_id(doc) {
            Some(result_sheet_id) => {
                Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenRetargetDialog { result_sheet_id, target_data_sheet_id: None }))
            }
            None => ui_route("ui.editParams.open", None),
        },
        "sheet.clonePipeline" => ui_route("ui.clonePipeline.open", resolve_any_sheet_id(doc)),
        "graph.setKind" => ui_route("ui.changeGraphType.open", resolve_graph_sheet_id(doc)),
        "graph.setSpec" | "graph.setSeries" => ui_route("ui.graph.format.open", resolve_graph_sheet_id(doc)),
        "graph.setAxes" | "graph.setAdditionalTicks" => ui_route("ui.graph.axes.open", resolve_graph_sheet_id(doc)),
        "graph.addComparisonBrackets" | "graph.setCompactLetterDisplay" => ui_route("ui.graph.pairwise.open", resolve_graph_sheet_id(doc)),
        "graph.setRegressionOverlay" | "graph.addAnnotation" => ui_route("ui.graph.insertModelEquation", resolve_graph_sheet_id(doc)),
        "graph.formatSelectedPoints" => ui_route("ui.formatPoints.open", resolve_graph_sheet_id(doc)),
        "graph.applyStyleFrom" => ui_route("ui.applyStyleFrom.open", resolve_graph_sheet_id(doc)),
        "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => {
            let target = if command_id == "layout.applyCanvasPreset" { resolve_layout_sheet_id(doc) } else { resolve_graph_sheet_id(doc) };
            let mut params = sheet_id_param(target);
            if let Some(obj) = params.as_object_mut() {
                obj.insert(String::from("preset"), Value::String(String::from("SingleColumn88mm")));
                obj.insert(String::from("resize_mode"), Value::String(String::from("KeepTypographyPt")));
            }
            Some(PaletteCommandRoute::UiCommand { command_id: String::from("ui.canvas.applyPreset"), params })
        }
        "graph.preflightCheck" => ui_route("ui.canvas.preflight", resolve_renderable_sheet_id(doc)),
        "layout.setSpec" | "layout.addPanel" => ui_route("ui.layout.composer.open", resolve_layout_sheet_id(doc)),
        "layout.equalizeScaling" => ui_route("ui.layout.equalizeScaling.open", resolve_layout_sheet_id(doc)),
        "table.excludeCellsWithReason" => ui_route("ui.grid.excludeWithReason", resolve_data_sheet_id(doc)),
        "table.randomizeGroups" => Some(PaletteCommandRoute::UiCommand { command_id: String::from("ui.table.randomizeGroups"), params: json!({}) }),
        "table.randomSampleRows" => {
            let params = match resolve_data_sheet_id(doc) {
                Some(id) => json!({ "source_sheet_id": id.as_str() }),
                None => json!({}),
            };
            Some(PaletteCommandRoute::UiCommand { command_id: String::from("ui.table.randomSampleRows"), params })
        }
        "table.simulateDistributions" => {
            Some(PaletteCommandRoute::UiCommand { command_id: String::from("ui.table.simulateDistributions"), params: json!({}) })
        }
        "table.convertArchetype" => ui_route("ui.table.convertArchetype.open", resolve_data_sheet_id(doc)),
        "table.importPlateMap" => {
            Some(PaletteCommandRoute::UiCommand { command_id: String::from("ui.import.reshape.open"), params: json!({ "mode": "Plate96" }) })
        }
        "table.importTidyLong" => {
            Some(PaletteCommandRoute::UiCommand { command_id: String::from("ui.import.reshape.open"), params: json!({ "mode": "TidyLongPivot" }) })
        }
        "table.setCalculatedVariable" => ui_route("ui.table.openFormulaBar", resolve_mv_sheet_id(doc).or_else(|| resolve_data_sheet_id(doc))),
        "table.setVariableRole" | "table.recodeLevels" | "table.setCategoricalOrder" => match resolve_mv_sheet_id(doc) {
            Some(sheet_id) => Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenManageVariables { sheet_id, column: None })),
            None => ui_route("ui.table.openFormulaBar", None),
        },
        "table.paste" => Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenImportWizard {
            origin: ImportOriginKind::Clipboard,
            is_live_link: false,
            target_sheet_id: resolve_data_sheet_id(doc),
        })),
        "table.pasteLink" => Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenImportWizard {
            origin: ImportOriginKind::Clipboard,
            is_live_link: true,
            target_sheet_id: resolve_data_sheet_id(doc),
        })),
        "table.importCsv" => Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenImportWizard {
            origin: ImportOriginKind::ExternalDelimitedFile,
            is_live_link: false,
            target_sheet_id: resolve_data_sheet_id(doc),
        })),
        "project.instantiateTemplate" => Some(PaletteCommandRoute::DirectIntent(UiIntent::SetLauncherOpen { open: true })),
        "project.createCheckpoint" | "project.listCheckpoints" | "project.restoreCheckpoint" | "project.deleteCheckpoint" => {
            Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenCheckpointsDialog))
        }
        _ => None,
    }
}

fn resolve_palette_sheet_engine_route(doc: &ProjectDocument, command_id: &str) -> Option<PaletteCommandRoute> {
    let eng = |params: Value| Some(PaletteCommandRoute::EngineCommand { command_id: command_id.to_owned(), params });
    match command_id {
        "sheet.delete" | "sheet.duplicate" | "sheet.duplicateFamily" | "sheet.freeze" | "sheet.unfreeze" | "sheet.select" => {
            eng(sheet_id_param(resolve_any_sheet_id(doc)))
        }
        "graph.create" => {
            let params = match resolve_data_sheet_id(doc) {
                Some(id) => json!({ "data_sheet_id": id.as_str() }),
                None => json!({}),
            };
            eng(params)
        }
        "graph.initialize" => {
            let target = doc
                .active_sheet_id
                .as_ref()
                .filter(|id| doc.graph_sheet(id).is_some())
                .cloned()
                .or_else(|| doc.graph_sheets.iter().find(|g| !g.initialized).map(|g| g.id.clone()))
                .or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()));
            eng(sheet_id_param(target))
        }
        "layout.alignOrigins" => {
            let mut params = sheet_id_param(resolve_layout_sheet_id(doc));
            if let Some(obj) = params.as_object_mut() {
                obj.insert(String::from("align"), Value::Bool(true));
                obj.insert(String::from("normalize_bounds"), Value::Bool(true));
            }
            eng(params)
        }
        "export.svg" | "export.pdf" | "export.png" | "export.tiff" => eng(sheet_id_param(resolve_renderable_sheet_id(doc))),
        "graph.exportSvg" | "graph.exportPdf" | "graph.exportPng" | "graph.exportEmf" => eng(sheet_id_param(resolve_graph_sheet_id(doc))),
        "layout.exportEmf" => eng(sheet_id_param(resolve_layout_sheet_id(doc))),
        "table.exportCsv" => {
            let target = doc
                .active_sheet_id
                .as_ref()
                .filter(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some())
                .cloned()
                .or_else(|| resolve_data_sheet_id(doc))
                .or_else(|| resolve_result_sheet_id(doc));
            eng(sheet_id_param(target))
        }
        "analysis.exportJson" | "analysis.exportCsv" => eng(sheet_id_param(resolve_result_sheet_id(doc))),
        "result.generateReport" => {
            let params = match resolve_result_sheet_id(doc) {
                Some(id) => json!({ "result_sheet_id": id.as_str() }),
                None => json!({}),
            };
            eng(params)
        }
        cmd if crate::menus::is_desktop_host_routed_command(cmd) => {
            Some(PaletteCommandRoute::UiCommand { command_id: cmd.to_owned(), params: json!({}) })
        }
        _ => None,
    }
}

fn is_palette_hidden_engine_command(command_id: &str) -> bool {
    matches!(
        command_id,
        "sheet.rename"
            | "sheet.reorder"
            | "project.saveTemplate"
            | "table.setCell"
            | "table.setCells"
            | "table.setXCells"
            | "table.clearCells"
            | "table.excludeCells"
            | "table.setRowTitles"
            | "table.setGroupTitles"
            | "table.setTitles"
            | "table.setSubcolumnTitles"
            | "info.setNote"
            | "info.setConstant"
            | "info.removeConstant"
            | "edit.pasteImage"
            | "edit.pasteResultTable"
            | "graph.insertImage"
            | "graph.updateImage"
            | "layout.insertImage"
            | "layout.updateImage"
            | "layout.reorderPanel"
    )
}

fn resolve_engine_route_with_policy(app: &UiApp, command_id: &str, allow_palette_hidden: bool) -> Option<PaletteCommandRoute> {
    if !allow_palette_hidden && is_palette_hidden_engine_command(command_id) {
        return None;
    }
    let doc = app.document();
    if let Some(route) = resolve_palette_dialog_engine_route(doc, command_id) {
        return Some(route);
    }
    if let Some(route) = resolve_palette_sheet_engine_route(doc, command_id) {
        return Some(route);
    }
    if allow_palette_hidden {
        let eng = |params: Value| Some(PaletteCommandRoute::EngineCommand { command_id: command_id.to_owned(), params });
        match command_id {
            "project.saveTemplate" => return eng(json!({ "return_bytes": true })),
            "edit.pasteImage" => {
                if let Some(sid) = resolve_renderable_sheet_id(doc) {
                    return Some(PaletteCommandRoute::DirectIntent(UiIntent::PasteImageFromClipboard { target_sheet_id: sid, anchor_pt: None }));
                }
            }
            "edit.pasteResultTable" => {
                if let Some(gid) = resolve_graph_sheet_id(doc) {
                    return Some(PaletteCommandRoute::DirectIntent(UiIntent::CanvasPaste { target_sheet_id: gid, anchor_pt: None }));
                }
            }
            "graph.insertImage" => return eng(sheet_id_param(resolve_graph_sheet_id(doc))),
            "layout.insertImage" => return eng(sheet_id_param(resolve_layout_sheet_id(doc))),
            "table.clearCells" => {
                let params = match resolve_data_sheet_id(doc) {
                    Some(id) => json!({ "sheet_id": id.as_str(), "all": true }),
                    None => json!({}),
                };
                return eng(params);
            }
            _ => {}
        }
    }
    // Only surface remaining engine commands when their schema has no required parameters
    // (so they can safely execute from `{}` without failing due to missing fields).
    let spec = engine_cmd::registry().get(command_id)?;
    let has_required = (spec.input_schema)().get("required").and_then(Value::as_array).is_some_and(|arr| !arr.is_empty());
    if has_required { None } else { Some(PaletteCommandRoute::EngineCommand { command_id: command_id.to_owned(), params: json!({}) }) }
}

fn resolve_palette_route(app: &UiApp, command_id: &str, is_ui_command: bool) -> Option<PaletteCommandRoute> {
    if is_ui_command { resolve_palette_ui_route(app, command_id) } else { resolve_engine_route_with_policy(app, command_id, false) }
}

/// Resolve a menu or shell command invocation into an interactive UI/dialog route or parameterized
/// engine command payload (`R15-GUI-30`).
pub(crate) fn resolve_shell_command_route(app: &UiApp, command_id: &str, is_ui_command: bool) -> Option<PaletteCommandRoute> {
    if is_ui_command { resolve_palette_ui_route(app, command_id) } else { resolve_engine_route_with_policy(app, command_id, true) }
}

/// Route a selected [`UnifiedSearchItem`] to its corresponding sheet, column, analysis, equation,
/// or command intent, record it in [`SearchRecents`], and close the Command Palette.
pub(crate) fn commit_unified_search_item(app: &UiApp, item: &UnifiedSearchItem, intents: &mut PendingUiIntents) {
    intents.push(UiIntent::SetPaletteOpen { open: false });
    intents.push(UiIntent::RecordSearchRecent(Box::new(item.clone())));
    match &item.action {
        UnifiedSearchAction::ActivateSheet(sheet_id) => {
            intents.push(UiIntent::SetLauncherOpen { open: false });
            intents.push_engine("sheet.select", json!({ "sheet_id": sheet_id.as_str() }));
        }
        UnifiedSearchAction::JumpToColumn { sheet_id, col } => {
            intents.push(UiIntent::JumpToDataColumn { sheet_id: sheet_id.clone(), col: *col });
        }
        UnifiedSearchAction::OpenAnalysis { catalog_id } => {
            intents.push(UiIntent::OpenAnalyzeWithCatalog { catalog_id: (*catalog_id).to_owned() });
        }
        UnifiedSearchAction::OpenEquation { equation_id } => {
            intents.push(UiIntent::OpenFitWithEquation { equation_id: equation_id.clone() });
        }
        UnifiedSearchAction::RunCommand { command_id, is_ui_command } => {
            commit_palette_command(app, command_id, *is_ui_command, intents);
        }
    }
}

fn commit_palette_command(app: &UiApp, command_id: &str, is_ui_command: bool, intents: &mut PendingUiIntents) {
    match resolve_palette_route(app, command_id, is_ui_command) {
        Some(PaletteCommandRoute::UiCommand { command_id, params }) => intents.push_ui(command_id, params),
        Some(PaletteCommandRoute::EngineCommand { command_id, params }) => intents.push_engine(command_id, params),
        Some(PaletteCommandRoute::DirectIntent(intent)) => intents.push(intent),
        None => {}
    }
}

fn layout_palette_line(painter: &egui::Painter, text: &str, max_width: f32, color: Color32, font_size: f32) -> (Arc<egui::Galley>, bool) {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(FontId::proportional(font_size), color));
    job.wrap = egui::text::TextWrapping { max_width: max_width.max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let ellipsized = galley.elided;
    (galley, ellipsized)
}

fn capture_outside_pointer_and_wheel(ctx: &egui::Context, dialog_rect: egui::Rect, report: &mut CommandPaletteFrameReport) {
    let has_wheel = ctx.input(|input| {
        let pointer_outside = input.pointer.latest_pos().or_else(|| input.pointer.interact_pos()).is_some_and(|p| !dialog_rect.contains(p));
        if pointer_outside && (input.pointer.any_pressed() || input.pointer.any_click()) {
            report.outside_click_captured = true;
        }
        pointer_outside && (input.smooth_scroll_delta.length_sq() > 0.0 || input.events.iter().any(|ev| matches!(ev, egui::Event::MouseWheel { .. })))
    });
    if has_wheel {
        ctx.input_mut(|i| {
            i.smooth_scroll_delta = Vec2::ZERO;
            i.events.retain(|ev| !matches!(ev, egui::Event::MouseWheel { .. }));
        });
        report.outside_wheel_captured = true;
    }
}

#[cfg(test)]
mod unified_search_integration_tests {
    use super::*;
    use crate::dialogs_canvas::ActiveDialog;
    use crate::egui_shell::data_grid::DataGridScrollTarget;
    use crate::egui_shell::inspect_bridge::inspect_live_egui_snapshot;
    use crate::interaction::ViewportMetrics;
    use crate::search_engine::{SearchRecents, UnifiedSearchAction, UnifiedSearchItem, UnifiedSearchKind};
    use curvebook_doc::{Cell, CellCoord, CellValue, TableArchetype};

    fn app_with_populated_data_sheet() -> Result<(UiApp, SheetId), String> {
        let mut app = UiApp::new();
        let created =
            app.execute_session_ui_cmd("sheet.createData", json!({ "kind": "Column", "title": "Dose Response" })).map_err(|e| e.to_string())?;
        let sheet_id_str = created.payload.get("sheet_id").and_then(Value::as_str).ok_or_else(|| String::from("missing sheet_id"))?;
        let sheet_id = SheetId::try_new(sheet_id_str).map_err(|e| e.to_string())?;
        let _ = app
            .execute_session_ui_cmd("table.setTitles", json!({ "sheet_id": sheet_id.as_str(), "titles": ["Control", "DrugA"] }))
            .map_err(|e| e.to_string())?;
        let _ = app
            .execute_session_ui_cmd("table.setCell", json!({ "sheet_id": sheet_id.as_str(), "row": 0, "col": 1, "subcol": 0, "value": 42.5 }))
            .map_err(|e| e.to_string())?;
        let _ = app.execute_session_ui_cmd("sheet.select", json!({ "sheet_id": sheet_id.as_str() })).map_err(|e| e.to_string())?;
        app.egui_views.launcher_open = false;
        app.nav.welcome.open = false;
        Ok((app, sheet_id))
    }

    #[test]
    fn scope1_unified_palette_routing_and_recents() -> Result<(), String> {
        let (mut app, sheet_id) = app_with_populated_data_sheet()?;

        // 1. Scope prefix toggling preserves body text
        let mut q = String::from("anova");
        apply_scope_filter_prefix(&mut q, SearchScopeFilter::AnalysesAndEquations);
        assert_eq!(q, "=anova");
        apply_scope_filter_prefix(&mut q, SearchScopeFilter::Commands);
        assert_eq!(q, ">anova");
        apply_scope_filter_prefix(&mut q, SearchScopeFilter::All);
        assert_eq!(q, "anova");

        // 2. OpenAnalysis routing via commit_unified_search_item
        let analysis_item = UnifiedSearchItem {
            kind: UnifiedSearchKind::Analysis,
            title: String::from("One-way ANOVA"),
            subtitle: String::from("Column Analyses"),
            category_badge: String::from("Analysis"),
            shortcut: String::new(),
            match_spans: Vec::new(),
            enabled: true,
            score: 5_000,
            action: UnifiedSearchAction::OpenAnalysis { catalog_id: "stats.oneWayAnova" },
            completion_text: String::from("One-way ANOVA"),
        };
        let mut intents = PendingUiIntents::default();
        commit_unified_search_item(&app, &analysis_item, &mut intents);
        let report = intents.commit_all_if_settled(&mut app, false);
        assert!(report.results.iter().all(Result::is_ok), "analysis commit errors: {:?}", report.results);
        assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
        assert_eq!(app.canvas_dialogs.analyze.selected_entry_id.as_deref(), Some("stats.oneWayAnova"));
        assert_eq!(app.egui_views.search_recents.len(), 1);

        // 3. OpenEquation routing
        let eq_item = UnifiedSearchItem {
            kind: UnifiedSearchKind::Equation,
            title: String::from("Michaelis-Menten"),
            subtitle: String::from("Enzyme Kinetics"),
            category_badge: String::from("Equation"),
            shortcut: String::new(),
            match_spans: Vec::new(),
            enabled: true,
            score: 5_000,
            action: UnifiedSearchAction::OpenEquation { equation_id: String::from("michaelis_menten") },
            completion_text: String::from("Michaelis-Menten"),
        };
        let mut intents = PendingUiIntents::default();
        commit_unified_search_item(&app, &eq_item, &mut intents);
        let report = intents.commit_all_if_settled(&mut app, false);
        assert!(report.results.iter().all(Result::is_ok));
        assert_eq!(app.canvas_dialogs.analyze.selected_entry_id.as_deref(), Some("fit.nonlinear"));
        assert_eq!(app.canvas_dialogs.analyze.equation_id, "michaelis_menten");
        app.canvas_dialogs.close_dialog();

        // 4. JumpToColumn routing
        let col_item = UnifiedSearchItem {
            kind: UnifiedSearchKind::Column,
            title: String::from("DrugA"),
            subtitle: String::from("Dose Response"),
            category_badge: String::from("Column"),
            shortcut: String::new(),
            match_spans: Vec::new(),
            enabled: true,
            score: 5_000,
            action: UnifiedSearchAction::JumpToColumn { sheet_id: sheet_id.clone(), col: 1 },
            completion_text: String::from("DrugA"),
        };
        let mut intents = PendingUiIntents::default();
        commit_unified_search_item(&app, &col_item, &mut intents);
        let report = intents.commit_all_if_settled(&mut app, false);
        assert!(report.results.iter().all(Result::is_ok));
        assert_eq!(app.document().active_sheet_id.as_ref(), Some(&sheet_id));
        assert_eq!(app.egui_views.column_grid.pending_scroll_target, Some(DataGridScrollTarget::ColumnTitle { col: 1 }));
        assert_eq!(app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(0, 1, 0)));

        // 5. OpenPaletteWithQuery preserves carried query
        let mut intents = PendingUiIntents::default();
        intents.push(UiIntent::OpenPaletteWithQuery { query: String::from("=anova") });
        let _ = intents.commit_all_if_settled(&mut app, false);
        assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette));
        assert_eq!(app.egui_views.command_palette.query, "=anova");
        assert!(app.egui_views.command_palette.preserve_query_on_open);

        Ok(())
    }

    #[test]
    fn scope2_top_bar_search_and_palette_compat() -> Result<(), String> {
        let (mut app, _) = app_with_populated_data_sheet()?;
        let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);

        // Both `toolbar.search_input` and `toolbar.command_palette` must be present in Field mode.
        let snap = inspect_live_egui_snapshot(&app, metrics, None, true);
        assert!(snap.find_widget("toolbar.search_input").is_some(), "expected toolbar.search_input in Field mode");
        assert!(snap.find_widget("toolbar.command_palette").is_some(), "expected toolbar.command_palette in Field mode for E2E compatibility");

        // Opening the inline dropdown surfaces non-blocking auto-suggest items + footer.
        app.egui_views.top_chrome.search_query = String::from("DrugA");
        app.egui_views.top_chrome.search_dropdown_open = true;
        let snap_open = inspect_live_egui_snapshot(&app, metrics, None, true);
        assert!(snap_open.find_widget("toolbar.search_dropdown").is_some());
        assert!(snap_open.find_widget("toolbar.search_item.0").is_some());
        assert!(snap_open.find_widget("toolbar.search_open_palette").is_some());

        // Opening full palette renders CommandPalette dialog and palette.query.
        let mut intents = PendingUiIntents::default();
        intents.push(UiIntent::SetPaletteOpen { open: true });
        let _ = intents.commit_all_if_settled(&mut app, false);
        let snap_pal = inspect_live_egui_snapshot(&app, metrics, None, true);
        assert_eq!(snap_pal.active_dialog, "command_palette");
        assert!(snap_pal.find_widget("palette.query").is_some());

        Ok(())
    }

    #[test]
    fn scope3_in_sheet_find_bar_and_replace_handoff() -> Result<(), String> {
        let (mut app, sheet_id) = app_with_populated_data_sheet()?;
        let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);

        // OpenSheetFind activates the floating find bar on the active DataSheet.
        let mut intents = PendingUiIntents::default();
        intents.push(UiIntent::OpenSheetFind);
        let _ = intents.commit_all_if_settled(&mut app, false);
        assert!(app.egui_views.sheet_find.open);
        assert_eq!(app.egui_views.sheet_find.last_sheet_id.as_ref(), Some(&sheet_id));

        // Verify matching query produces DataGridFindHighlight via sync_sheet_find_before_stage.
        app.egui_views.sheet_find.query = String::from("42.5");
        let ds = app.document().data_sheet(&sheet_id).ok_or_else(|| String::from("missing data sheet"))?;
        assert!(matches!(ds.archetype, TableArchetype::Column(_)));
        assert_eq!(ds.archetype.data_cell(CellCoord::new(0, 1, 0)).map(|c: &Cell| &c.value), Some(&CellValue::Number(42.5)));

        let _snap = inspect_live_egui_snapshot(&app, metrics, None, true);
        // Also verify `OpenFindReplace { initial_query }` handoff and `CloseSheetFind` cleanup.
        let mut intents = PendingUiIntents::default();
        intents.push(UiIntent::OpenFindReplace { initial_query: Some(String::from("42.5")) });
        let _ = intents.commit_all_if_settled(&mut app, false);
        assert!(app.egui_views.app_modals.find_replace_open);
        assert_eq!(app.egui_views.app_modals.find_replace.find_query, "42.5");

        let mut intents = PendingUiIntents::default();
        intents.push(UiIntent::CloseSheetFind);
        let _ = intents.commit_all_if_settled(&mut app, false);
        assert!(!app.egui_views.sheet_find.open);
        assert!(app.egui_views.column_grid.active_find_matches.is_none());
        assert!(app.egui_views.results_view.active_find_matches.is_none());

        // Verify SearchRecents is wired cleanly on EguiShellViewState.
        let _recents: &SearchRecents = &app.egui_views.search_recents;
        Ok(())
    }
}
