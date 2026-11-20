//! Native `egui` `Cmd/Ctrl+K` Command Palette modal overlay (`docs/planning/gui-layout-spec.md` §3, `R15.3`).

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontId, Id, Key, Modifiers, Order, Pos2, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::CommandOrigin;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, Tokens};
use crate::inspect::ZLayer;
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
#[must_use]
pub fn filter_palette_commands(app: &UiApp, query: &str, max_rows: usize) -> Vec<CommandPaletteRow> {
    let mut palette = app.canvas_dialogs.command_palette.clone();
    palette.query = query.to_owned();
    palette.sync_shortcuts(&app.shortcuts);
    palette
        .matches_with_shortcuts(Some(app.session()), &app.shortcuts)
        .into_iter()
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

/// Render the `Cmd/Ctrl+K` Command Palette overlay (`ZLayer::ModalDialog`) with `100%`
/// `ZLayer::ModalBackdrop` click + wheel capture and full `ArrowUp`/`ArrowDown`/`Enter`/`Esc`
/// keyboard navigation.
pub fn render_command_palette(
    ctx: &egui::Context,
    app: &UiApp,
    state: &mut CommandPaletteEguiState,
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
        clipped_rect: screen_rect,
        z_layer: ZLayer::ModalBackdrop,
        ellipsized: false,
        full_label: None,
    });

    if backdrop_resp.clicked() {
        report.outside_click_captured = true;
        intents.push_set_palette_open(false);
    }

    if state.just_opened {
        state.query.clear();
        state.selected_index = 0;
        state.hovered_index = None;
    }

    let mut rows = filter_palette_commands(app, &state.query, 24);
    report.match_count = rows.len();
    clamp_palette_selection(state, &rows);

    let key_nav = consume_palette_keys(ctx, state, &rows, intents);

    let top_offset = (screen_rect.height() * 0.10).clamp(16.0, 56.0);
    let card_w = (screen_rect.width() - 32.0).clamp(240.0, 560.0);
    let card_h = (screen_rect.height() - top_offset - 16.0).clamp(160.0, 380.0);

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
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ctx, |ui| {
            let prev_query = state.query.clone();
            let query_resp = ui.add(
                egui::TextEdit::singleline(&mut state.query).hint_text("Search commands, analyses & sheets (Cmd+K)").desired_width(f32::INFINITY),
            );
            if state.just_opened {
                query_resp.request_focus();
                state.just_opened = false;
            }
            if state.query != prev_query {
                rows = filter_palette_commands(app, &state.query, 24);
                report.match_count = rows.len();
                state.selected_index = rows.iter().position(|r| r.enabled).unwrap_or(0);
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

            ui.separator();
            let footer_h = 34.0;
            let list_h = (ui.available_height() - footer_h).max(54.0);
            const PALETTE_ROW_H: f32 = 30.0;
            ui.allocate_ui(Vec2::new(ui.available_width(), list_h), |ui| {
                ui.spacing_mut().scroll.floating = false;
                ui.spacing_mut().item_spacing.y = 0.0;
                if rows.is_empty() {
                    ui.colored_label(tokens.text_muted, "No matching commands.");
                } else {
                    egui::ScrollArea::vertical().id_salt("egui_shell.palette.scroll").auto_shrink([false, false]).show_rows(
                        ui,
                        PALETTE_ROW_H,
                        rows.len(),
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
                            render_palette_rows(ui, app, state, &rows, row_range, tokens, meta, intents);
                        },
                    );
                }
            });

            ui.separator();
            render_palette_footer(ui, app, rows.get(state.selected_index), tokens);
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
            clipped_rect: dialog_rect,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        });
        capture_outside_pointer_and_wheel(ctx, dialog_rect, &mut report);
    }

    report
}

struct PaletteKeyNavResult {
    moved: bool,
}

fn clamp_palette_selection(state: &mut CommandPaletteEguiState, rows: &[CommandPaletteRow]) {
    if rows.is_empty() {
        state.selected_index = 0;
        state.hovered_index = None;
    } else if state.selected_index >= rows.len() {
        state.selected_index = rows.len() - 1;
    }
}

fn consume_palette_keys(
    ctx: &egui::Context,
    state: &mut CommandPaletteEguiState,
    rows: &[CommandPaletteRow],
    intents: &mut PendingUiIntents,
) -> PaletteKeyNavResult {
    let mut moved = false;
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_set_palette_open(false);
        return PaletteKeyNavResult { moved };
    }
    if !rows.is_empty() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            state.selected_index = (state.selected_index + 1) % rows.len();
            moved = true;
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            state.selected_index = if state.selected_index == 0 { rows.len() - 1 } else { state.selected_index - 1 };
            moved = true;
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
            state.selected_index = 0;
            moved = true;
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::End)) {
            state.selected_index = rows.len() - 1;
            moved = true;
        }
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
        && let Some(row) = rows.get(state.selected_index)
        && row.enabled
    {
        commit_palette_row(row, intents);
    }
    PaletteKeyNavResult { moved }
}

#[allow(clippy::too_many_arguments)]
fn render_palette_rows(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut CommandPaletteEguiState,
    rows: &[CommandPaletteRow],
    row_range: std::ops::Range<usize>,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    for idx in row_range {
        let Some(row) = rows.get(idx) else {
            continue;
        };
        let is_selected = idx == state.selected_index;
        let row_w = ui.available_width().max(120.0);
        let (slot_rect, slot_resp) = ui.allocate_exact_size(Vec2::new(row_w, 30.0), Sense::hover());
        let visible_rect = slot_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
        if visible_rect.height() < 12.0 || visible_rect.width() < 12.0 {
            continue;
        }
        let rect = slot_rect.shrink2(Vec2::new(0.0, 1.0));
        let resp = ui.interact(rect, slot_resp.id.with(idx), if row.enabled { Sense::click() } else { Sense::hover() });
        if resp.hovered() {
            state.hovered_index = Some(idx);
        }

        let bg = if !row.enabled {
            if is_selected { tokens.panel_bg } else { Color32::TRANSPARENT }
        } else if is_selected {
            tokens.active_bg
        } else if resp.hovered() {
            tokens.hover_bg
        } else {
            tokens.surface_bg
        };
        let border = if is_selected { Stroke::new(1.0, tokens.accent_strong) } else { Stroke::NONE };
        let row_clip = rect.intersect(ui.clip_rect());
        ui.painter().with_clip_rect(row_clip).rect_filled(rect, CornerRadius::same(4), bg);
        if border != Stroke::NONE {
            ui.painter().with_clip_rect(row_clip).rect_stroke(rect, CornerRadius::same(4), border, StrokeKind::Inside);
        }

        let shortcut_w = if row.shortcut.is_empty() { 0.0 } else { 92.0 };
        let label_max_w = (rect.width() - shortcut_w - 20.0).max(40.0);
        let title_color = if row.enabled { tokens.text_strong } else { tokens.text_disabled };
        let (galley, ellipsized) = layout_palette_line(ui.painter(), &row.label, label_max_w, title_color, tokens.metrics.font_ui_body_pt);
        ui.painter().with_clip_rect(rect.shrink(4.0).intersect(ui.clip_rect())).galley(
            Pos2::new(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5),
            galley,
            title_color,
        );

        if !row.shortcut.is_empty() {
            let (sc_galley, _) = layout_palette_line(ui.painter(), &row.shortcut, 86.0, tokens.text_muted, 11.0);
            let sc_pos = Pos2::new((rect.right() - sc_galley.size().x - 10.0).max(rect.left() + 40.0), rect.center().y - sc_galley.size().y * 0.5);
            ui.painter().with_clip_rect(rect.shrink(4.0).intersect(ui.clip_rect())).galley(sc_pos, sc_galley, tokens.text_muted);
        }

        let reason = (!row.enabled).then(|| disabled_reason_for_row(app, row));
        let ak_label = row.label.clone();
        resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, row.enabled, ak_label.clone()));
        let wid = format!("palette.item.{idx}");
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "button",
                label: &row.label,
                command_id: Some(&row.command_id),
                value: Some(json!(row.enabled)),
                enabled: row.enabled,
                disabled_reason: reason.as_deref(),
                z_layer: ZLayer::ModalDialog,
                ellipsized,
                full_label: ellipsized.then_some(row.label.as_str()),
            },
        );
        if let Some(ref rsn) = reason {
            resp.on_hover_text(rsn);
        } else if resp.clicked() && row.enabled {
            state.selected_index = idx;
            commit_palette_row(row, intents);
        }
    }
}

fn render_palette_footer(ui: &mut egui::Ui, app: &UiApp, selected: Option<&CommandPaletteRow>, tokens: &Tokens) {
    let preview_w = ui.available_width().max(80.0);
    let (msg, color) = match selected {
        Some(row) if !row.enabled => (format!("Disabled: {}", disabled_reason_for_row(app, row)), tokens.text_muted),
        Some(row) if !row.menu_path.is_empty() && !row.summary.is_empty() => (format!("{} — {}", row.menu_path, row.summary), tokens.text_primary),
        Some(row) if !row.summary.is_empty() => (row.summary.clone(), tokens.text_primary),
        _ => (String::from("↑↓ Navigate · Enter Execute · Esc Dismiss"), tokens.text_muted),
    };
    let (galley, _) = layout_palette_line(ui.painter(), &msg, (preview_w - 8.0).max(40.0), color, 11.5);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(preview_w, 20.0), Sense::hover());
    ui.painter().with_clip_rect(rect).galley(Pos2::new(rect.left() + 2.0, rect.center().y - galley.size().y * 0.5), galley, color);
}

fn disabled_reason_for_row(app: &UiApp, row: &CommandPaletteRow) -> String {
    let doc = app.document();
    match row.command_id.as_str() {
        "analysis.updateParams"
        | "ui.editParams.open"
        | "ui.result.copyMethodsReport"
        | "ui.result.inspectDiagnostics"
        | "ui.result.applyRemediation" => String::from("Open a ResultSheet first"),
        "ui.result.appendClusterColumn" => String::from("Requires a Clustering ResultSheet"),
        "ui.table.openFormulaBar" => String::from("Requires a Multiple Variables table"),
        "ui.layout.composer.open" | "ui.layout.equalizeScaling.open" => String::from("Open a LayoutSheet first"),
        "ui.graph.axes.open" | "ui.graph.format.open" | "ui.changeGraphType.open" | "ui.formatPoints.open" => {
            String::from("Requires at least one GraphSheet")
        }
        "edit.undo" => String::from("Nothing to undo"),
        "edit.redo" => String::from("Nothing to redo"),
        _ if doc.data_sheets.is_empty() => String::from("Requires a DataSheet in the active project"),
        _ if !row.beginner.is_empty() => row.beginner.clone(),
        _ => String::from("Preconditions not met for active sheet"),
    }
}

fn commit_palette_row(row: &CommandPaletteRow, intents: &mut PendingUiIntents) {
    intents.push_set_palette_open(false);
    if row.is_ui_command {
        intents.push_ui(&row.command_id, json!({}));
    } else {
        intents.push_engine(&row.command_id, json!({}));
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
