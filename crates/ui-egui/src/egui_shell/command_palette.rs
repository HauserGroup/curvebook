//! Native `egui` `Cmd/Ctrl+K` Command Palette modal overlay (`docs/planning/gui-layout-spec.md` §3, `R15.3`).

use std::sync::Arc;

use curvebook_doc::{ProjectDocument, SheetId, SheetSection, TableArchetype};
use curvebook_engine::cmd as engine_cmd;
use curvebook_palette::CanvasPreviewFilter;
use egui::{Color32, CornerRadius, FontId, Id, Key, Modifiers, Order, Pos2, Sense, Stroke, StrokeKind, Vec2};
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

    let query_widget_id = Id::unique("egui_shell.palette.query");
    let query_focused = ctx.memory(|m| m.focused()) == Some(query_widget_id);
    let key_nav = consume_palette_keys(ctx, app, state, &rows, query_focused, intents);

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
                .corner_radius(CornerRadius::same(tokens.metrics.radius_lg))
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ctx, |ui| {
            let prev_query = state.query.clone();
            let plat_shortcut = format_shortcut_for_platform(app.shortcuts.active_combo("ui.palette.open").unwrap_or("Cmd+K"));
            let hint = format!("Search commands, analyses & sheets ({plat_shortcut})");
            let query_resp = ui.add(egui::TextEdit::singleline(&mut state.query).id(query_widget_id).hint_text(hint).desired_width(f32::INFINITY));
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
                            render_palette_rows(ui, PaletteRowsRenderCtx { app, rows: &rows, row_range, tokens }, state, meta, intents);
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
    app: &UiApp,
    state: &mut CommandPaletteEguiState,
    rows: &[CommandPaletteRow],
    query_focused: bool,
    intents: &mut PendingUiIntents,
) -> PaletteKeyNavResult {
    let mut moved = false;
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_set_palette_open(false);
        return PaletteKeyNavResult { moved };
    }
    let home_end_nav_allowed = !query_focused || state.query.is_empty();
    if !rows.is_empty() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            state.selected_index = (state.selected_index + 1) % rows.len();
            moved = true;
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            state.selected_index = if state.selected_index == 0 { rows.len() - 1 } else { state.selected_index - 1 };
            moved = true;
        } else if home_end_nav_allowed && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
            state.selected_index = 0;
            moved = true;
        } else if home_end_nav_allowed && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::End)) {
            state.selected_index = rows.len() - 1;
            moved = true;
        }
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
        && let Some(row) = rows.get(state.selected_index)
        && row.enabled
    {
        commit_palette_row(app, row, intents);
    }
    PaletteKeyNavResult { moved }
}

struct PaletteRowsRenderCtx<'a> {
    app: &'a UiApp,
    rows: &'a [CommandPaletteRow],
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
    let PaletteRowsRenderCtx { app, rows, row_range, tokens } = ctx;
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
            let plat_sc = format_shortcut_for_platform(&row.shortcut);
            let (sc_galley, _) = layout_palette_line(ui.painter(), &plat_sc, 86.0, tokens.text_muted, 11.0);
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
            commit_palette_row(app, row, intents);
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
        "ui.embeddedTable.format.open" => String::from("Paste a ResultSheet table onto a graph or layout first"),
        "ui.image.format.open" => String::from("Insert or paste an image onto a graph or layout first"),
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
            let params = match (resolve_data_sheet_id(doc), app.grid.tool.active) {
                (Some(id), Some(coord)) => json!({ "sheet_id": id.as_str(), "row": coord.row, "col": coord.col, "subcol": coord.subcol }),
                (Some(id), None) => json!({ "sheet_id": id.as_str(), "row": 0, "col": 0, "subcol": 0 }),
                (None, _) => json!({}),
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
        "ui.editParams.open" | "ui.result.copyMethodsReport" | "ui.result.inspectDiagnostics" | "ui.result.appendClusterColumn" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_result_sheet_id(doc)) })
        }
        "ui.analyze.open" | "ui.grid.excludeWithReason" | "ui.table.convertArchetype.open" | "ui.graph.create.open" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_data_sheet_id(doc)) })
        }
        "ui.table.openFormulaBar" => Some(PaletteCommandRoute::UiCommand {
            command_id: command_id.to_owned(),
            params: sheet_id_param(resolve_mv_sheet_id(doc).or_else(|| resolve_data_sheet_id(doc))),
        }),
        "ui.clonePipeline.open" | "ui.print.open" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_any_sheet_id(doc)) })
        }
        "ui.canvas.preflight" | "ui.export.figure.open" | "ui.embeddedTable.format.open" | "ui.image.format.open" => {
            Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_renderable_sheet_id(doc)) })
        }
        _ => Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: json!({}) }),
    }
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
        "table.importCsv" | "table.importPrism" | "table.importPzfx" => Some(PaletteCommandRoute::DirectIntent(UiIntent::OpenImportWizard {
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
                Some(id) => json!({ "result_sheet_id": id.as_str(), "format": "markdown" }),
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

fn commit_palette_row(app: &UiApp, row: &CommandPaletteRow, intents: &mut PendingUiIntents) {
    intents.push_set_palette_open(false);
    match resolve_palette_route(app, &row.command_id, row.is_ui_command) {
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
