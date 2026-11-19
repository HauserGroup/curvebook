//! Native `egui` `Cmd/Ctrl+K` Command Palette modal overlay (`docs/planning/gui-layout-spec.md` §3, `R15.3`).

use egui::{Color32, CornerRadius, FontId, Id, Key, Order, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::CommandOrigin;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, Tokens};
use crate::layout::ZLayer;
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
            enabled: entry.enabled,
            is_ui_command: matches!(entry.origin, CommandOrigin::Ui),
        })
        .collect()
}

/// Render the `Cmd/Ctrl+K` Command Palette overlay with `100%` `ZLayer::ModalBackdrop` capture.
///
/// The Wave 3 base renders the backdrop, searchable `palette.query` input, match list, and `Esc`
/// dismiss; `r15-top-chrome` enriches keyboard arrow navigation, shortcut pills, and rich previews.
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

    let card_w = (screen_rect.width() - 32.0).clamp(240.0, 560.0);
    let card_h = (screen_rect.height() - 48.0).clamp(180.0, 380.0);
    let rows = filter_palette_commands(app, &state.query, 12);
    report.match_count = rows.len();

    let win_resp = egui::Window::new(tr(&app.locale, "inspector.command_palette"))
        .id(Id::unique("egui_shell.command_palette.window"))
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 64.0))
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
            let query_resp = ui.add(
                egui::TextEdit::singleline(&mut state.query).hint_text("Search commands, analyses & sheets (Cmd+K)").desired_width(f32::INFINITY),
            );
            if state.just_opened {
                query_resp.request_focus();
                state.just_opened = false;
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
            egui::ScrollArea::vertical().id_salt("egui_shell.palette.scroll").auto_shrink([false, false]).show(ui, |ui| {
                for (idx, row) in rows.iter().enumerate() {
                    let row_lbl = if row.shortcut.is_empty() { row.label.clone() } else { format!("{}  ({})", row.label, row.shortcut) };
                    let btn = ui.add_enabled(
                        row.enabled,
                        egui::Button::new(egui::RichText::new(&row_lbl).font(FontId::proportional(tokens.metrics.font_ui_body_pt)))
                            .min_size(Vec2::new(ui.available_width(), 26.0)),
                    );
                    let wid = format!("palette.item.{idx}");
                    meta.record_response(
                        ui,
                        &btn,
                        FrameInspectEntryInput {
                            widget_id: &wid,
                            role: "button",
                            label: &row.label,
                            command_id: Some(&row.command_id),
                            value: Some(json!(row.enabled)),
                            enabled: row.enabled,
                            disabled_reason: (!row.enabled).then_some("Preconditions not met for active sheet"),
                            z_layer: ZLayer::ModalDialog,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if btn.clicked() && row.enabled {
                        intents.push_set_palette_open(false);
                        if row.is_ui_command {
                            intents.push_ui(&row.command_id, json!({}));
                        } else {
                            intents.push_engine(&row.command_id, json!({}));
                        }
                    }
                }
            });
            ui.painter().rect_stroke(ui.max_rect(), CornerRadius::same(4), Stroke::NONE, StrokeKind::Inside);
        });

    if let Some(inner) = win_resp {
        let dialog_rect = inner.response.rect;
        report.dialog_rect_pt = Some(dialog_rect);
        let has_wheel = ctx.input(|input| {
            let pointer_outside = input.pointer.latest_pos().or_else(|| input.pointer.interact_pos()).is_some_and(|p| !dialog_rect.contains(p));
            if pointer_outside && input.pointer.any_pressed() {
                report.outside_click_captured = true;
            }
            pointer_outside
                && (input.smooth_scroll_delta.length_sq() > 0.0 || input.events.iter().any(|ev| matches!(ev, egui::Event::MouseWheel { .. })))
        });
        if has_wheel {
            ctx.input_mut(|i| {
                i.smooth_scroll_delta = Vec2::ZERO;
                i.events.retain(|ev| !matches!(ev, egui::Event::MouseWheel { .. }));
            });
            report.outside_wheel_captured = true;
        }
    }

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push_set_palette_open(false);
    }

    report
}
