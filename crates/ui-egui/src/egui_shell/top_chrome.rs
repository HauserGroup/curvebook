//! Wave 3 Two-Tier Top Chrome (`32 pt` Tier 1 Application Bar + `38 pt` Tier 2 Sheet-Adaptive
//! Contextual Action Strip, `docs/planning/gui-layout-spec.md` §3, `R15.3`).

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontId, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, SplitWorkspaceState};
use crate::egui_shell::inspect_bridge::HybridCoexistenceDecision;
use crate::egui_shell::top_bar_fit::{MIN_SWITCHER_PT, RightWidths, SearchVisibility, fit_right_group};
use crate::egui_shell::{
    FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, WorkspacePreset,
};
use crate::i18n::{LOCALES, locale_picker_label, normalize_locale};
use crate::inspect::ZLayer;
use crate::interaction::resolve_result_sheet_for_shortcut;
use crate::menus::{CommandTooltip, command_tooltip, menu_bar_tooltips};
use crate::tr;
use curvebook_doc::{AnalysisKind, MultivariateSpec, SheetId, TableArchetype};

/// Shared canonical variant lists for submenu-backed settings & layout commands.
pub fn submenu_variants_for_command(command_id: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match command_id {
        "ui.theme.set" => Some(&[("Light", "light"), ("Dark", "dark"), ("High Contrast", "high_contrast")]),
        "ui.previewFilter.set" => Some(&[
            ("Normal Vision", "normal"),
            ("Deuteranopia", "deuteranopia"),
            ("Protanopia", "protanopia"),
            ("Tritanopia", "tritanopia"),
            ("Grayscale Print", "grayscale"),
        ]),
        "ui.canvas.applyPreset" | "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => Some(&[
            ("Single Column (88 mm)", "SingleColumn88mm"),
            ("1.5 Column (130 mm)", "ColumnAndHalf130mm"),
            ("Double Column (180 mm)", "DoubleColumn180mm"),
            ("Full Page Portrait (180 × 240 mm)", "FullPagePortrait"),
            ("Presentation Slide 16:9", "Slide16x9"),
            ("Poster Panel (12 × 9 in)", "PosterPanel"),
        ]),
        _ => None,
    }
}

/// Extra desktop File menu actions surfaced whenever absent from `menu_bar_tooltips(app)`.
pub const EXTRA_FILE_MENU_ACTIONS: &[(&str, &str, &[&str])] = &[
    ("Open Unpacked Folder…", "file.open_unpacked_dir", &["file.open_unpacked_dir"]),
    ("Save As…", "file.save_as", &["project.saveAs", "file.save_as"]),
    ("Export PDF…", "export.pdf", &["export.pdf", "file.export_pdf"]),
    ("Export PNG…", "export.png", &["export.png", "file.export_png"]),
];

/// Cross-backend parity item descriptor shared between Tier 1 inline `egui::MenuBar` and `muda`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InWindowMenuItemParity {
    pub command_id: String,
    pub label: String,
    pub shortcut: String,
    pub summary: String,
    pub beginner: String,
    pub enabled: bool,
}

/// Cross-backend parity section descriptor shared between Tier 1 inline `egui::MenuBar` and `muda`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InWindowMenuSectionParity {
    pub menu_key: String,
    pub title: String,
    pub items: Vec<InWindowMenuItemParity>,
}

/// Canonical filtering & effective enablement check for menu items across all hosts.
#[must_use]
pub fn is_menu_item_visible(command_id: &str) -> bool {
    command_id != "ui.event.dispatch"
}

/// Effective menu enablement shared between native `muda` menus and inline `egui::MenuBar`.
#[must_use]
pub fn effective_menu_item_enabled(command_id: &str, base_enabled: bool) -> bool {
    base_enabled && command_id != "ui.result.applyRemediation"
}

/// Build canonical localized menu sections directly from [`menu_bar_tooltips`] for `egui::MenuBar`
/// and desktop menu-bar parity checks.
#[must_use]
pub fn collect_in_window_menu_sections(app: &UiApp) -> Vec<InWindowMenuSectionParity> {
    menu_bar_tooltips(app)
        .into_iter()
        .map(|sec| InWindowMenuSectionParity {
            menu_key: sec.menu_key.to_owned(),
            title: sec.title.to_owned(),
            items: sec
                .items
                .into_iter()
                .filter(|item| is_menu_item_visible(item.command_id))
                .map(|item| InWindowMenuItemParity {
                    command_id: item.command_id.to_owned(),
                    label: item.localized_label.to_owned(),
                    shortcut: item.shortcut,
                    summary: item.summary.to_owned(),
                    beginner: item.beginner.to_owned(),
                    enabled: effective_menu_item_enabled(item.command_id, item.enabled),
                })
                .collect(),
        })
        .collect()
}

/// Persistent `egui_shell` top chrome state (split/preset/overflow popups + active in-window menu).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopChromeEguiState {
    pub split_menu_open: bool,
    pub preset_menu_open: bool,
    pub overflow_menu_open: bool,
    pub active_in_window_menu: Option<String>,
}

impl TopChromeEguiState {
    fn close_all_popups(&mut self) {
        self.split_menu_open = false;
        self.preset_menu_open = false;
        self.overflow_menu_open = false;
        self.active_in_window_menu = None;
    }
}

/// Render context bundle for [`render_top_chrome`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct TopChromeContext<'a> {
    pub tokens: &'a Tokens,
    pub decision: &'a HybridCoexistenceDecision,
    pub layout: ResponsiveShellLayout,
    pub workspace_preset: WorkspacePreset,
    pub launcher_open: bool,
    pub host_draws_menu_bar: bool,
    pub project_title: &'a str,
    pub dirty: bool,
}

/// Per-frame geometry & popup report emitted by [`render_top_chrome`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TopChromeFrameReport {
    pub tier1_rect_pt: Option<egui::Rect>,
    pub tier2_rect_pt: Option<egui::Rect>,
    pub total_height_pt: f32,
    pub overflow_item_count: usize,
    pub menu_popup_open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier2SheetContext {
    EmptyWorkspace,
    DataStructured,
    DataMultipleVariables,
    InfoSheet,
    ResultSheet,
    GraphSheet,
    LayoutSheet,
}

#[derive(Debug, Clone)]
struct Tier2ActionSpec {
    widget_id: String,
    label: String,
    tooltip: String,
    command_id: String,
    params: Value,
    is_engine: bool,
    enabled: bool,
    disabled_reason: Option<String>,
    primary: bool,
    retarget_sheet_id: Option<SheetId>,
}

#[derive(Debug, Clone, Copy)]
struct Tier1PopupsInput<'a> {
    app: &'a UiApp,
    tokens: &'a Tokens,
    enabled: bool,
    workspace_preset: WorkspacePreset,
}

/// Render Tier 1 (`32 pt`) Application Bar and Tier 2 (`38 pt`) Contextual Action Strip cleanly.
///
/// Preserves exact widget IDs (`toolbar.home`, `toolbar.help.about`, `toolbar.command_palette`,
/// `toolbar.split`, `toolbar.preset`, `toolbar.presentation`, `toolbar.analyze`,
/// `toolbar.edit_params`, `toolbar.overflow`, `sheet.tab.<id>`).
pub fn render_top_chrome(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut TopChromeEguiState,
    ctx_in: TopChromeContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> TopChromeFrameReport {
    let mut report = TopChromeFrameReport::default();
    if ctx_in.layout.top_chrome_h_pt <= 0.0 {
        state.close_all_popups();
        return report;
    }

    let tokens = ctx_in.tokens;
    let _ = ctx_in.decision.modal_surface;
    let enabled = !ctx_in.decision.modal_backdrop_active;
    if !enabled {
        state.close_all_popups();
    }

    let tier1_h = ctx_in.layout.tier1_app_bar_h_pt.max(24.0);
    let tier2_h = ctx_in.layout.tier2_action_strip_h_pt.max(26.0);
    let mut overflow_items = Vec::new();
    let mut split_anchor = None;
    let mut preset_anchor = None;
    let mut overflow_anchor = None;

    let panel_resp = egui::Panel::top("egui_shell.top_chrome")
        .exact_size(tier1_h + tier2_h)
        .frame(egui::Frame::NONE.fill(tokens.app_bar).inner_margin(egui::Margin::symmetric(8, 0)))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let tier1_resp = ui.allocate_ui(Vec2::new(ui.available_width(), tier1_h), |ui| {
                render_tier1_bar(ui, app, state, &ctx_in, enabled, meta, intents, &mut split_anchor, &mut preset_anchor);
            });
            report.tier1_rect_pt = Some(tier1_resp.response.rect);

            let div_y = tier1_resp.response.rect.bottom();
            let panel_w = tier1_resp.response.rect.width();
            ui.painter().line_segment(
                [Pos2::new(tier1_resp.response.rect.left() - 8.0, div_y), Pos2::new(tier1_resp.response.rect.left() + panel_w + 8.0, div_y)],
                Stroke::new(1.0, tokens.divider),
            );

            let tier2_resp = ui.allocate_ui(Vec2::new(ui.available_width(), tier2_h), |ui| {
                render_tier2_strip(ui, app, state, tokens, enabled, meta, intents, &mut overflow_items, &mut overflow_anchor);
            });
            report.tier2_rect_pt = Some(tier2_resp.response.rect);

            let bottom_y = tier2_resp.response.rect.bottom();
            ui.painter().line_segment(
                [Pos2::new(tier2_resp.response.rect.left() - 8.0, bottom_y), Pos2::new(tier2_resp.response.rect.left() + panel_w + 8.0, bottom_y)],
                Stroke::new(1.0, tokens.divider),
            );
        });

    report.total_height_pt = panel_resp.response.rect.height();
    report.overflow_item_count = overflow_items.len();
    if overflow_items.is_empty() {
        state.overflow_menu_open = false;
    }

    let popup_ctx = Tier1PopupsInput { app, tokens, enabled, workspace_preset: ctx_in.workspace_preset };
    if state.split_menu_open
        && let Some(anchor) = split_anchor
    {
        render_split_preset_popup(ui.ctx(), &popup_ctx, state, anchor, meta, intents);
    }
    if state.preset_menu_open
        && let Some(anchor) = preset_anchor
    {
        render_workspace_preset_popup(ui.ctx(), &popup_ctx, state, anchor, meta, intents);
    }
    if state.overflow_menu_open
        && let Some(anchor) = overflow_anchor
        && !overflow_items.is_empty()
    {
        render_tier2_overflow_popup(ui.ctx(), &popup_ctx, state, &overflow_items, anchor, meta, intents);
    }

    report.menu_popup_open = state.active_in_window_menu.is_some() || state.split_menu_open || state.preset_menu_open || state.overflow_menu_open;
    report
}

/// Render minimal floating Presentation Mode exit chip (`toolbar.presentation.exit`) in top-right corner.
pub fn render_presentation_exit_chip(
    ctx: &egui::Context,
    app: &UiApp,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> Option<Rect> {
    if !app.presentation_mode {
        return None;
    }
    let label = match app.locale.as_str() {
        "de" => "Präsentation beenden (Esc)",
        "fr" => "Quitter le mode présentation (Esc)",
        "es" => "Salir de presentación (Esc)",
        "ja" => "プレゼン終了 (Esc)",
        _ => "Exit Presentation (Esc)",
    };
    let area = egui::Area::new(Id::unique("egui_shell.presentation.exit_chip"))
        .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-12.0, 10.0))
        .order(Order::Foreground)
        .show(ctx, |ui| {
            let (galley, _) = layout_single_line_job(ui.painter(), label, 220.0, tokens.text_strong, 11.5);
            let w = (galley.size().x + 18.0).clamp(96.0, 236.0);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), if enabled { Sense::click() } else { Sense::hover() });
            let bg = if resp.hovered() && enabled { tokens.hover_bg } else { tokens.panel_bg };
            ui.painter().rect_filled(rect, CornerRadius::same(13), bg);
            ui.painter().rect_stroke(rect, CornerRadius::same(13), Stroke::new(1.0, tokens.border), StrokeKind::Inside);
            ui.painter().with_clip_rect(rect.shrink(4.0)).galley(
                Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5),
                galley,
                tokens.text_strong,
            );
            let ak_label = label.to_owned();
            resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, ak_label.clone()));
            meta.record_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: "toolbar.presentation.exit",
                    role: "button",
                    label,
                    command_id: Some("ui.presentation.toggle"),
                    value: Some(json!(false)),
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::FloatingOverlay,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if resp.clicked() && enabled {
                intents.push_ui("ui.presentation.toggle", json!({ "enabled": false }));
            }
            rect
        });
    Some(area.inner)
}

#[allow(clippy::too_many_arguments)]
fn render_tier1_bar(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut TopChromeEguiState,
    ctx_in: &TopChromeContext<'_>,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    split_anchor: &mut Option<Rect>,
    preset_anchor: &mut Option<Rect>,
) {
    let tokens = ctx_in.tokens;
    let about_label = tr(&app.locale, "menu.help.about");
    let home_label = localized_home_label(&app.locale);
    let total_w = ui.available_width().max(40.0);

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (brand_rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
        crate::egui_shell::brand::paint_mark(ui, brand_rect);

        let home_resp =
            chrome_button_fitted(ui, tokens, meta, "toolbar.home", home_label, None, ctx_in.launcher_open, enabled, modal_reason(enabled), 84.0);
        if home_resp.clicked() && enabled {
            state.close_all_popups();
            intents.push_set_launcher_open(!ctx_in.launcher_open);
        }

        let about_resp = chrome_button_fitted(
            ui,
            tokens,
            meta,
            "toolbar.help.about",
            about_label,
            Some("app.about"),
            false,
            enabled,
            modal_reason(enabled),
            104.0,
        );
        if about_resp.clicked() && enabled {
            state.close_all_popups();
            intents.push_ui("app.about", json!({}));
        }

        // Compute compact right-group budget before placing middle menus/title so right cluster never clips.
        let right_min_reserve = if total_w >= 540.0 { 196.0 } else { 112.0 };
        let used_left = (brand_rect.width() + home_resp.rect.width() + about_resp.rect.width() + 24.0).min(total_w);
        let middle_budget = (total_w - used_left - right_min_reserve).max(0.0);

        state.active_in_window_menu = None;
        if !ctx_in.host_draws_menu_bar && middle_budget >= 54.0 {
            render_inline_menu_bar(ui, app, state, tokens, enabled, middle_budget, meta, intents);
        }

        let remaining_after_menus = (ui.available_width() - 6.0).max(0.0);
        let fitted = fit_right_group(&RightWidths {
            room: remaining_after_menus,
            has_pinned_badge: false,
            switcher: 84.0,
            field: 176.0,
            icon: 48.0,
            gap: 6.0,
            trailing_extra: 56.0,
        });
        let right_w = estimate_tier1_right_width(&fitted);
        let title_slack = (ui.available_width() - right_w - 8.0).max(0.0);
        if title_slack >= 48.0 {
            let title = if ctx_in.dirty { format!("{} •", ctx_in.project_title) } else { ctx_in.project_title.to_owned() };
            render_single_line_title_badge(ui, tokens, meta, &title, title_slack.min(220.0));
        }

        let filler = (ui.available_width() - right_w).max(0.0);
        if filler > 0.0 {
            ui.add_space(filler);
        }
        render_tier1_right_group(ui, app, state, ctx_in, &fitted, enabled, meta, intents, split_anchor, preset_anchor);
    });
}

fn estimate_tier1_right_width(fitted: &crate::egui_shell::top_bar_fit::RightGroup) -> f32 {
    let search_w = match fitted.search {
        SearchVisibility::Field => 176.0,
        SearchVisibility::Icon => 48.0,
        SearchVisibility::Hidden => 0.0,
    };
    let mut parts = Vec::new();
    if search_w > 0.0 {
        parts.push(search_w);
    }
    if fitted.split_toggle {
        parts.push(58.0);
    }
    if fitted.switcher {
        parts.push(fitted.switcher_w.max(MIN_SWITCHER_PT));
    }
    if fitted.presentation_toggle {
        parts.push(64.0);
    }
    let gaps = parts.len().saturating_sub(1) as f32 * 6.0;
    parts.into_iter().sum::<f32>() + gaps
}

#[allow(clippy::too_many_arguments)]
fn render_inline_menu_bar(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut TopChromeEguiState,
    tokens: &Tokens,
    enabled: bool,
    budget_w: f32,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let sections = collect_in_window_menu_sections(app);
    if sections.is_empty() {
        return;
    }

    let overflow_lbl = "⋯";
    let overflow_btn_w = 34.0;
    let mut visible_count = 0usize;
    let mut acc_w = 0.0_f32;
    let mut measured_widths = Vec::with_capacity(sections.len());
    for sec in &sections {
        let (galley, _) = layout_single_line_job(ui.painter(), &sec.title, 96.0, tokens.text_primary, 12.0);
        measured_widths.push((galley.size().x + 14.0).clamp(32.0, 108.0));
    }
    for (idx, w) in measured_widths.iter().enumerate() {
        let has_more_after = idx + 1 < sections.len();
        let reserve = if has_more_after { overflow_btn_w + 4.0 } else { 0.0 };
        if acc_w + *w + reserve <= budget_w {
            acc_w += *w + 4.0;
            visible_count += 1;
        } else {
            break;
        }
    }

    let actual_w = (acc_w + if visible_count < sections.len() { overflow_btn_w } else { 0.0 }).clamp(34.0, budget_w);
    ui.allocate_ui(Vec2::new(actual_w, 24.0), |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (sec, &sec_w) in sections.iter().take(visible_count).zip(measured_widths.iter()) {
                render_top_menu_button(ui, state, tokens, sec, sec_w, enabled, meta, intents);
            }
            if visible_count < sections.len() {
                let ov_resp = ui.add_enabled(enabled, egui::Button::new(overflow_lbl).min_size(Vec2::new(28.0, 22.0)));
                let ak_lbl = tr(&app.locale, "toolbar.overflow").to_owned();
                ov_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, ak_lbl.clone()));
                meta.record_response(
                    ui,
                    &ov_resp,
                    FrameInspectEntryInput {
                        widget_id: "menu.bar.overflow",
                        role: "menu",
                        label: tr(&app.locale, "toolbar.overflow"),
                        command_id: None,
                        value: None,
                        enabled,
                        disabled_reason: modal_reason(enabled),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if enabled {
                    egui::Popup::menu(&ov_resp).show(|ui| {
                        state.active_in_window_menu = Some(String::from("menu.bar.overflow"));
                        for sec in &sections[visible_count..] {
                            ui.menu_button(&sec.title, |ui| {
                                render_section_menu_items(ui, &sec.menu_key, &sec.items, tokens.locale, meta, intents);
                            });
                        }
                    });
                }
            }
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn render_top_menu_button(
    ui: &mut egui::Ui,
    state: &mut TopChromeEguiState,
    tokens: &Tokens,
    sec: &InWindowMenuSectionParity,
    sec_w: f32,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let (galley, ellipsized) = layout_single_line_job(ui.painter(), &sec.title, (sec_w - 10.0).max(16.0), tokens.text_primary, 12.0);
    let resp = ui.add_enabled_ui(enabled, |ui| ui.allocate_response(Vec2::new(sec_w, 22.0), Sense::click())).inner;
    let is_open = enabled && egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let bg = if is_open {
        tokens.active_bg
    } else if resp.hovered() && enabled {
        tokens.hover_bg
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(resp.rect, CornerRadius::same(4), bg);
    }
    let fg = if enabled { tokens.text_primary } else { tokens.text_disabled };
    ui.painter().with_clip_rect(resp.rect.shrink(3.0)).galley(
        Pos2::new(resp.rect.center().x - galley.size().x * 0.5, resp.rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    let ak_label = sec.title.clone();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::MenuItem, enabled, ak_label.clone()));
    if resp.rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: &sec.menu_key,
                role: "menu",
                label: &sec.title,
                command_id: None,
                value: Some(json!(is_open)),
                enabled,
                disabled_reason: modal_reason(enabled),
                z_layer: ZLayer::Workspace,
                ellipsized,
                full_label: ellipsized.then_some(sec.title.as_str()),
            },
        );
    }

    if enabled {
        egui::Popup::menu(&resp).show(|ui| {
            state.active_in_window_menu = Some(sec.menu_key.clone());
            render_section_menu_items(ui, &sec.menu_key, &sec.items, tokens.locale, meta, intents);
            let popup_rect = ui.min_rect().intersect(ui.clip_rect()).intersect(meta.viewport_rect);
            if popup_rect.is_positive() {
                let popup_id = ui.scope_id().with(("popup", &sec.menu_key));
                meta.record(FrameInspectNodeMeta {
                    egui_id: popup_id,
                    accesskit_id: popup_id.accesskit_id(),
                    widget_id: format!("popup.{}", sec.menu_key),
                    role: String::from("menu"),
                    label: sec.title.clone(),
                    command_id: None,
                    value: Some(json!(true)),
                    enabled: true,
                    disabled_reason: None,
                    clipped_rect: popup_rect,
                    z_layer: ZLayer::DropdownMenu,
                    ellipsized: false,
                    full_label: None,
                });
            }
        });
    }
}

struct MenuPopupInspect<'a> {
    widget_id: &'a str,
    label: &'a str,
    command_id: Option<&'a str>,
    selected: Option<bool>,
    enabled: bool,
}

fn record_menu_popup_response(ui: &egui::Ui, resp: &egui::Response, meta: &mut FrameInspectMeta, spec: MenuPopupInspect<'_>) {
    if !resp.rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        return;
    }
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput {
            widget_id: spec.widget_id,
            role: "menuitem",
            label: spec.label,
            command_id: spec.command_id,
            value: spec.selected.map(|s| json!(s)),
            enabled: spec.enabled,
            disabled_reason: modal_reason(spec.enabled),
            z_layer: ZLayer::DropdownMenu,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn render_section_menu_items(
    ui: &mut egui::Ui,
    menu_key: &str,
    items: &[InWindowMenuItemParity],
    current_locale: &str,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if matches!(menu_key, "menu.file" | "file") {
        let is_primary_file_cmd =
            |cmd: &str| matches!(cmd, "project.new" | "file.new" | "project.open" | "project.save" | "export.svg" | "export.pdf" | "export.png");
        for item in items.iter().filter(|it| is_primary_file_cmd(&it.command_id)) {
            render_menu_leaf_item(ui, item, current_locale, meta, intents);
        }
        let mut added_extra = false;
        for &(label, cmd, aliases) in EXTRA_FILE_MENU_ACTIONS {
            if items.iter().any(|existing| aliases.contains(&existing.command_id.as_str())) {
                continue;
            }
            if !added_extra {
                ui.separator();
                added_extra = true;
            }
            let resp = ui.button(label);
            record_menu_popup_response(
                ui,
                &resp,
                meta,
                MenuPopupInspect { widget_id: &format!("menu.{cmd}"), label, command_id: Some(cmd), selected: None, enabled: true },
            );
            if resp.clicked() {
                push_menu_command_intent(cmd, None, intents);
                ui.close();
            }
        }
        ui.separator();
        for item in items.iter().filter(|it| !is_primary_file_cmd(&it.command_id)) {
            render_menu_leaf_item(ui, item, current_locale, meta, intents);
        }
        return;
    }
    for item in items {
        render_menu_leaf_item(ui, item, current_locale, meta, intents);
    }
}

fn render_menu_leaf_item(
    ui: &mut egui::Ui,
    item: &InWindowMenuItemParity,
    current_locale: &str,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tooltip = if item.beginner.is_empty() { item.summary.clone() } else { format!("{}\n{}", item.summary, item.beginner) };
    if let Some(variants) = submenu_variants_for_command(&item.command_id) {
        let sub = ui.menu_button(&item.label, |ui| {
            for &(v_label, variant) in variants {
                let mut btn = egui::Button::new(v_label);
                if item.command_id == "ui.canvas.applyPreset" && !item.shortcut.is_empty() && variant == "SingleColumn88mm" {
                    btn = btn.shortcut_text(&item.shortcut);
                }
                let resp = ui.add_enabled(item.enabled, btn).on_hover_text(&tooltip).on_disabled_hover_text(&tooltip);
                if resp.clicked() {
                    push_menu_command_intent(&item.command_id, Some(variant), intents);
                    ui.close();
                }
            }
        });
        record_menu_popup_response(
            ui,
            &sub.response,
            meta,
            MenuPopupInspect {
                widget_id: &format!("menu.{}", item.command_id),
                label: &item.label,
                command_id: Some(&item.command_id),
                selected: None,
                enabled: item.enabled,
            },
        );
        return;
    }

    if item.command_id == "ui.locale.set" {
        let active_canonical = normalize_locale(current_locale);
        let sub = ui.menu_button(&item.label, |ui| {
            let scroll_out = egui::ScrollArea::vertical().id_salt("popup.menu.ui.locale.set").max_height(240.0).show(ui, |ui| {
                for (idx, &loc) in LOCALES.iter().enumerate() {
                    let canonical = normalize_locale(loc);
                    let caption = locale_picker_label(loc);
                    let opt_resp = ui.button(&caption).on_hover_text(&tooltip);
                    record_menu_popup_response(
                        ui,
                        &opt_resp,
                        meta,
                        MenuPopupInspect {
                            widget_id: &format!("popup.menu.ui.locale.set.opt.{idx}"),
                            label: &caption,
                            command_id: Some("ui.locale.set"),
                            selected: Some(canonical == active_canonical),
                            enabled: item.enabled,
                        },
                    );
                    if opt_resp.clicked() {
                        push_menu_command_intent(&item.command_id, Some(canonical), intents);
                        ui.close();
                    }
                }
            });
            let popup_rect = scroll_out.inner_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
            if popup_rect.is_positive() {
                let popup_id = ui.scope_id().with("popup.menu.ui.locale.set");
                meta.record(FrameInspectNodeMeta {
                    egui_id: popup_id,
                    accesskit_id: popup_id.accesskit_id(),
                    widget_id: String::from("popup.menu.ui.locale.set"),
                    role: String::from("menu"),
                    label: item.label.clone(),
                    command_id: Some(String::from("ui.locale.set")),
                    value: None,
                    enabled: item.enabled,
                    disabled_reason: None,
                    clipped_rect: popup_rect,
                    z_layer: ZLayer::DropdownMenu,
                    ellipsized: false,
                    full_label: None,
                });
            }
        });
        record_menu_popup_response(
            ui,
            &sub.response,
            meta,
            MenuPopupInspect {
                widget_id: "menu.ui.locale.set",
                label: &item.label,
                command_id: Some("ui.locale.set"),
                selected: None,
                enabled: item.enabled,
            },
        );
        return;
    }

    let mut btn = egui::Button::new(&item.label);
    if !item.shortcut.is_empty() {
        btn = btn.shortcut_text(&item.shortcut);
    }
    let resp = ui.add_enabled(item.enabled, btn).on_hover_text(&tooltip).on_disabled_hover_text(&tooltip);
    record_menu_popup_response(
        ui,
        &resp,
        meta,
        MenuPopupInspect {
            widget_id: &format!("menu.{}", item.command_id),
            label: &item.label,
            command_id: Some(&item.command_id),
            selected: None,
            enabled: item.enabled,
        },
    );
    if resp.clicked() {
        push_menu_command_intent(&item.command_id, None, intents);
        ui.close();
    }
}

fn push_menu_command_intent(command_id: &str, variant: Option<&str>, intents: &mut PendingUiIntents) {
    if let Some(var) = variant {
        match command_id {
            "ui.theme.set" => intents.push_ui("ui.theme.set", json!({ "mode": var })),
            "ui.locale.set" => intents.push_ui("ui.locale.set", json!({ "locale": var })),
            "ui.previewFilter.set" => intents.push_ui("ui.previewFilter.set", json!({ "filter": var })),
            "ui.canvas.applyPreset" | "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => {
                intents.push_ui("ui.canvas.applyPreset", json!({ "preset": var }));
            }
            other => intents.push_ui(other, json!({ "value": var })),
        }
        return;
    }
    if command_id.starts_with("ui.") || command_id.starts_with("app.") || crate::menus::is_desktop_host_routed_command(command_id) {
        intents.push_ui(command_id, json!({}));
    } else {
        intents.push_engine(command_id, json!({}));
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tier1_right_group(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut TopChromeEguiState,
    ctx_in: &TopChromeContext<'_>,
    fitted: &crate::egui_shell::top_bar_fit::RightGroup,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    split_anchor: &mut Option<Rect>,
    preset_anchor: &mut Option<Rect>,
) {
    let tokens = ctx_in.tokens;
    if !matches!(fitted.search, SearchVisibility::Hidden) {
        let search_lbl = match fitted.search {
            SearchVisibility::Field => format!("{} (Cmd+K)", tr(&app.locale, "toolbar.palette")),
            SearchVisibility::Icon | SearchVisibility::Hidden => String::from("Cmd+K"),
        };
        let max_w = if matches!(fitted.search, SearchVisibility::Field) { 176.0 } else { 52.0 };
        let pal_resp = chrome_button_fitted(
            ui,
            tokens,
            meta,
            "toolbar.command_palette",
            &search_lbl,
            Some("ui.palette.open"),
            matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette),
            enabled,
            modal_reason(enabled),
            max_w,
        );
        if pal_resp.clicked() && enabled {
            state.close_all_popups();
            intents.push_set_palette_open(true);
        }
    }

    if fitted.split_toggle {
        let split_lbl = if app.split.enabled { "Split •" } else { "Split" };
        let split_resp = chrome_button_fitted(
            ui,
            tokens,
            meta,
            "toolbar.split",
            split_lbl,
            Some("ui.split.toggle"),
            app.split.enabled || state.split_menu_open,
            enabled,
            modal_reason(enabled),
            68.0,
        );
        *split_anchor = Some(split_resp.rect);
        if split_resp.clicked() && enabled {
            let next = !state.split_menu_open;
            state.close_all_popups();
            state.split_menu_open = next;
        }
    }

    if fitted.switcher {
        let preset_resp = chrome_button_fitted(
            ui,
            tokens,
            meta,
            "toolbar.preset",
            ctx_in.workspace_preset.label(),
            None,
            state.preset_menu_open,
            enabled,
            modal_reason(enabled),
            fitted.switcher_w.max(MIN_SWITCHER_PT),
        );
        *preset_anchor = Some(preset_resp.rect);
        if preset_resp.clicked() && enabled {
            let next = !state.preset_menu_open;
            state.close_all_popups();
            state.preset_menu_open = next;
        }
    }

    if fitted.presentation_toggle {
        let pres_lbl = localized_present_label(&app.locale);
        let pres_resp = chrome_button_fitted(
            ui,
            tokens,
            meta,
            "toolbar.presentation",
            pres_lbl,
            Some("ui.presentation.toggle"),
            app.presentation_mode,
            enabled,
            modal_reason(enabled),
            76.0,
        );
        if pres_resp.clicked() && enabled {
            state.close_all_popups();
            intents.push_ui("ui.presentation.toggle", json!({}));
        }
    }
}

fn render_split_preset_popup(
    ctx: &egui::Context,
    input: &Tier1PopupsInput<'_>,
    state: &mut TopChromeEguiState,
    anchor: Rect,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = input.app.document();
    let first_ds = doc.data_sheets.first().map(|s| s.id.as_str().to_owned());
    let first_gs = doc.graph_sheets.first().map(|s| s.id.as_str().to_owned());
    let first_rs = doc.result_sheets.first().map(|s| s.id.as_str().to_owned());
    let two_up_pair = collect_two_up_pair(doc, &input.app.split);
    let (cur_primary, cur_secondary) = input.app.split.resolve_sheets(doc);
    let tokens = input.tokens;

    let pos = clamp_popup_pos(ctx, Pos2::new(anchor.left(), anchor.bottom() + 4.0), 196.0, 176.0);
    let area = egui::Area::new(Id::unique("egui_shell.split.preset_popup")).fixed_pos(pos).order(Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(tokens.panel_bg)
            .stroke(Stroke::new(1.0, tokens.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.set_min_width(184.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                let single =
                    popup_action_button(ui, tokens, meta, "toolbar.split.preset.single", "Single Pane (Off)", !input.app.split.enabled, true, None);
                if single.clicked() {
                    intents.push_ui("ui.split.toggle", json!({ "enabled": false }));
                    state.split_menu_open = false;
                }

                let dg_enabled = first_ds.is_some() && first_gs.is_some();
                let dg = popup_action_button(
                    ui,
                    tokens,
                    meta,
                    "toolbar.split.preset.data_graph",
                    "Data + Graph",
                    false,
                    dg_enabled,
                    (!dg_enabled).then_some("Requires a DataSheet and a GraphSheet"),
                );
                if dg.clicked()
                    && let (Some(ds), Some(gs)) = (first_ds.as_ref(), first_gs.as_ref())
                {
                    intents.push_ui("ui.split.toggle", json!({ "enabled": true, "primary_sheet_id": ds, "secondary_sheet_id": gs, "ratio": 0.5 }));
                    state.split_menu_open = false;
                }

                let dr_enabled = first_ds.is_some() && first_rs.is_some();
                let dr = popup_action_button(
                    ui,
                    tokens,
                    meta,
                    "toolbar.split.preset.data_results",
                    "Data + Results",
                    false,
                    dr_enabled,
                    (!dr_enabled).then_some("Requires a DataSheet and a ResultSheet"),
                );
                if dr.clicked()
                    && let (Some(ds), Some(rs)) = (first_ds.as_ref(), first_rs.as_ref())
                {
                    intents.push_ui("ui.split.toggle", json!({ "enabled": true, "primary_sheet_id": ds, "secondary_sheet_id": rs, "ratio": 0.5 }));
                    state.split_menu_open = false;
                }

                let two_enabled = two_up_pair.is_some();
                let two = popup_action_button(
                    ui,
                    tokens,
                    meta,
                    "toolbar.split.preset.two_up",
                    "2-Up (50 / 50)",
                    input.app.split.enabled,
                    two_enabled,
                    (!two_enabled).then_some("Open at least two sheets for 2-Up view"),
                );
                if two.clicked()
                    && let Some((p, s)) = two_up_pair.as_ref()
                {
                    intents.push_ui("ui.split.toggle", json!({ "enabled": true, "primary_sheet_id": p, "secondary_sheet_id": s, "ratio": 0.5 }));
                    state.split_menu_open = false;
                }

                let swap_enabled = input.app.split.enabled && cur_primary.is_some() && cur_secondary.is_some();
                let swap = popup_action_button(
                    ui,
                    tokens,
                    meta,
                    "toolbar.split.preset.swap",
                    "Swap Panes",
                    false,
                    swap_enabled,
                    (!swap_enabled).then_some("Enable split view with two sheets to swap"),
                );
                if swap.clicked()
                    && let (Some(p), Some(s)) = (cur_primary.as_ref(), cur_secondary.as_ref())
                {
                    intents.push_ui("ui.split.toggle", json!({ "enabled": true, "primary_sheet_id": s.as_str(), "secondary_sheet_id": p.as_str() }));
                    state.split_menu_open = false;
                }
            });
    });

    if !input.enabled || (popup_outside_clicked(ctx, area.response.rect, anchor)) {
        state.split_menu_open = false;
    }
}

fn render_workspace_preset_popup(
    ctx: &egui::Context,
    input: &Tier1PopupsInput<'_>,
    state: &mut TopChromeEguiState,
    anchor: Rect,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = input.tokens;
    let pos = clamp_popup_pos(ctx, Pos2::new(anchor.left(), anchor.bottom() + 4.0), 172.0, 160.0);
    let area = egui::Area::new(Id::unique("egui_shell.workspace.preset_popup")).fixed_pos(pos).order(Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(tokens.panel_bg)
            .stroke(Stroke::new(1.0, tokens.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.set_min_width(160.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                for preset in WorkspacePreset::ALL {
                    let selected = preset == input.workspace_preset;
                    let label = if selected { format!("✓ {}", preset.label()) } else { preset.label().to_owned() };
                    let wid = preset_widget_id(preset);
                    let resp = popup_action_button(ui, tokens, meta, wid, &label, selected, true, None);
                    if resp.clicked() {
                        intents.push_apply_workspace_preset(preset);
                        state.preset_menu_open = false;
                    }
                }
            });
    });

    if !input.enabled || popup_outside_clicked(ctx, area.response.rect, anchor) {
        state.preset_menu_open = false;
    }
}

fn render_tier2_overflow_popup(
    ctx: &egui::Context,
    input: &Tier1PopupsInput<'_>,
    state: &mut TopChromeEguiState,
    items: &[Tier2ActionSpec],
    anchor: Rect,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = input.tokens;
    let est_h = (items.len() as f32 * 28.0 + 16.0).clamp(48.0, 260.0);
    let pos = clamp_popup_pos(ctx, Pos2::new(anchor.right() - 210.0, anchor.bottom() + 4.0), 216.0, est_h);
    let area = egui::Area::new(Id::unique("egui_shell.tier2.overflow_popup")).fixed_pos(pos).order(Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(tokens.panel_bg)
            .stroke(Stroke::new(1.0, tokens.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.set_min_width(204.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                for (idx, act) in items.iter().enumerate() {
                    let wid = format!("toolbar.overflow.item.{idx}");
                    let resp = popup_action_button(ui, tokens, meta, &wid, &act.label, false, act.enabled, act.disabled_reason.as_deref());
                    if resp.clicked() && act.enabled {
                        emit_tier2_action_intent(act, intents);
                        state.overflow_menu_open = false;
                    }
                }
            });
    });

    if !input.enabled || popup_outside_clicked(ctx, area.response.rect, anchor) {
        state.overflow_menu_open = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tier2_strip(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut TopChromeEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    overflow_out: &mut Vec<Tier2ActionSpec>,
    overflow_anchor: &mut Option<Rect>,
) {
    let actions = collect_tier2_actions(app, enabled);
    let tab_entries = collect_all_sheet_tabs(app);
    let strip_w = ui.available_width().max(40.0);

    // Reserve tab strip and overflow room so every visible button & tab remains strictly inside `strip_w`.
    let tabs_reserve = if tab_entries.is_empty() { 0.0 } else { (strip_w * 0.34).clamp(96.0, 280.0) };
    let action_budget = (strip_w - tabs_reserve).max(110.0);

    let mut measured = Vec::with_capacity(actions.len());
    for act in &actions {
        let (galley, _) = layout_single_line_job(ui.painter(), &act.label, 152.0, tokens.text_primary, 12.0);
        measured.push((galley.size().x + 16.0).clamp(40.0, 168.0));
    }

    let overflow_btn_w = 34.0;
    let mut fit_count = 0usize;
    let mut used_w = 0.0_f32;
    for (idx, &w) in measured.iter().enumerate() {
        let has_more = idx + 1 < actions.len();
        let ov_reserve = if has_more { overflow_btn_w + 6.0 } else { 0.0 };
        if used_w + w + ov_reserve <= action_budget || idx == 0 {
            used_w += w + 6.0;
            fit_count += 1;
        } else {
            break;
        }
    }

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for act in actions.iter().take(fit_count) {
            let resp = chrome_button_fitted(
                ui,
                tokens,
                meta,
                &act.widget_id,
                &act.label,
                Some(&act.command_id),
                act.primary,
                act.enabled,
                act.disabled_reason.as_deref(),
                168.0,
            )
            .on_hover_text(&act.tooltip);
            if resp.clicked() && act.enabled {
                state.close_all_popups();
                emit_tier2_action_intent(act, intents);
            }
        }

        if fit_count < actions.len() {
            *overflow_out = actions[fit_count..].to_vec();
            let ov_resp = chrome_button_fitted(
                ui,
                tokens,
                meta,
                "toolbar.overflow",
                "⋯",
                None,
                state.overflow_menu_open,
                enabled,
                modal_reason(enabled),
                overflow_btn_w,
            )
            .on_hover_text(tr(&app.locale, "toolbar.overflow"));
            *overflow_anchor = Some(ov_resp.rect);
            if ov_resp.clicked() && enabled {
                let next = !state.overflow_menu_open;
                state.close_all_popups();
                state.overflow_menu_open = next;
            }
        }

        let rem_tabs_w = ui.available_width().max(0.0);
        if !tab_entries.is_empty() && rem_tabs_w >= 44.0 {
            ui.separator();
            render_tier2_sheet_tabs(ui, tokens, meta, intents, &tab_entries, enabled);
        }
    });
}

fn render_tier2_sheet_tabs(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    tabs: &[(String, String, bool)],
    enabled: bool,
) {
    let switcher_w = ui.available_width().max(32.0);
    let max_vis_tabs = ((switcher_w / 62.0).floor() as usize).clamp(1, tabs.len());
    let gap = 6.0 * max_vis_tabs.saturating_sub(1) as f32;
    let max_tab_w = ((switcher_w - gap) / max_vis_tabs as f32).clamp(32.0, 148.0);
    for (sheet_id, title, is_active) in tabs.iter().take(max_vis_tabs) {
        let wid = format!("sheet.tab.{sheet_id}");
        let resp = chrome_tab(ui, tokens, meta, &wid, title, *is_active, enabled, max_tab_w);
        if resp.clicked() && enabled && !*is_active {
            intents.push_engine("sheet.select", json!({ "sheet_id": sheet_id }));
        }
    }
}

fn collect_all_sheet_tabs(app: &UiApp) -> Vec<(String, String, bool)> {
    let doc = app.document();
    let active = doc.active_sheet_id.as_ref();
    let mut tabs = Vec::new();
    for ds in &doc.data_sheets {
        tabs.push((ds.id.as_str().to_owned(), ds.title.clone(), active == Some(&ds.id)));
    }
    for info in &doc.info_sheets {
        tabs.push((info.id.as_str().to_owned(), info.title.clone(), active == Some(&info.id)));
    }
    for rs in &doc.result_sheets {
        tabs.push((rs.id.as_str().to_owned(), rs.title().to_owned(), active == Some(&rs.id)));
    }
    for gs in &doc.graph_sheets {
        tabs.push((gs.id.as_str().to_owned(), gs.title.clone(), active == Some(&gs.id)));
    }
    for ls in &doc.layout_sheets {
        tabs.push((ls.id.as_str().to_owned(), ls.title.clone(), active == Some(&ls.id)));
    }
    tabs
}

fn detect_tier2_context(app: &UiApp) -> Tier2SheetContext {
    let doc = app.document();
    if doc.total_sheet_count() == 0 {
        return Tier2SheetContext::EmptyWorkspace;
    }
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return Tier2SheetContext::EmptyWorkspace;
    };
    if let Some(ds) = doc.data_sheet(active_id) {
        return if matches!(ds.archetype, TableArchetype::MultipleVariables(_)) {
            Tier2SheetContext::DataMultipleVariables
        } else {
            Tier2SheetContext::DataStructured
        };
    }
    if doc.info_sheet(active_id).is_some() {
        return Tier2SheetContext::InfoSheet;
    }
    if doc.result_sheet(active_id).is_some() {
        return Tier2SheetContext::ResultSheet;
    }
    if doc.graph_sheet(active_id).is_some() {
        return Tier2SheetContext::GraphSheet;
    }
    if doc.layout_sheet(active_id).is_some() {
        return Tier2SheetContext::LayoutSheet;
    }
    Tier2SheetContext::EmptyWorkspace
}

fn collect_tier2_actions(app: &UiApp, modal_enabled: bool) -> Vec<Tier2ActionSpec> {
    let doc = app.document();
    let context = detect_tier2_context(app);
    let mut actions = Vec::new();

    // Fixed universal left anchors (`+ New`, `Undo`, `Redo`).
    actions.push(make_engine_action(
        app,
        modal_enabled,
        "toolbar.new",
        tr(&app.locale, "toolbar.new"),
        "sheet.createData",
        json!({ "kind": "column", "cols": 3, "rows": 8 }),
        true,
        None,
        matches!(context, Tier2SheetContext::EmptyWorkspace),
    ));

    let can_undo = app.session().undo_depth() > 0;
    let undo_ok = modal_enabled && can_undo;
    actions.push(make_engine_action(
        app,
        undo_ok,
        "toolbar.undo",
        tr(&app.locale, "toolbar.undo"),
        "edit.undo",
        json!({}),
        can_undo,
        Some(if modal_enabled { "Nothing to undo" } else { "modal dialog is active" }),
        false,
    ));

    let can_redo = app.session().redo_depth() > 0;
    let redo_ok = modal_enabled && can_redo;
    actions.push(make_engine_action(
        app,
        redo_ok,
        "toolbar.redo",
        tr(&app.locale, "toolbar.redo"),
        "edit.redo",
        json!({}),
        can_redo,
        Some(if modal_enabled { "Nothing to redo" } else { "modal dialog is active" }),
        false,
    ));

    let active_ds_id = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()).map(|id| id.as_str().to_owned());
    let analyze_ready = modal_enabled && can_analyze_active(app);
    let analyze_reason = if !modal_enabled {
        Some("modal dialog is active")
    } else if !analyze_ready {
        Some("Select a DataSheet before analyzing")
    } else {
        None
    };

    let on_result_sheet = resolve_result_sheet_for_shortcut(app, true).is_some();
    let edit_reason = if !modal_enabled {
        Some("modal dialog is active")
    } else if !on_result_sheet {
        Some("Open a ResultSheet to edit analysis parameters (Cmd+T)")
    } else {
        None
    };

    match context {
        Tier2SheetContext::EmptyWorkspace => {
            actions.push(make_ui_action(app, modal_enabled, "toolbar.open", tr(&app.locale, "toolbar.open"), "project.open", json!({}), None, false));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.empty.import",
                tr(&app.locale, "menu.file.import"),
                "ui.import.reshape.open",
                json!({}),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                analyze_ready,
                "toolbar.analyze",
                tr(&app.locale, "menu.analyze.analyze"),
                "ui.analyze.open",
                json!({}),
                analyze_reason,
                false,
            ));
        }
        Tier2SheetContext::DataStructured => {
            actions.push(make_ui_action(
                app,
                analyze_ready,
                "toolbar.analyze",
                tr(&app.locale, "menu.analyze.analyze"),
                "ui.analyze.open",
                json!({}),
                analyze_reason,
                true,
            ));
            if let Some(ds_id) = active_ds_id {
                actions.push(make_engine_action(
                    app,
                    modal_enabled,
                    "toolbar.new_graph",
                    tr(&app.locale, "menu.insert.new_graph"),
                    "graph.create",
                    json!({ "data_sheet_id": ds_id }),
                    true,
                    None,
                    false,
                ));
            }
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.convert_archetype",
                tr(&app.locale, "cmd.ui.table.convertArchetype.open"),
                "ui.table.convertArchetype.open",
                json!({}),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled && on_result_sheet,
                "toolbar.edit_params",
                tr(&app.locale, "toolbar.parameters"),
                "ui.editParams.open",
                json!({}),
                edit_reason,
                false,
            ));
        }
        Tier2SheetContext::DataMultipleVariables => {
            actions.push(make_ui_action(
                app,
                analyze_ready,
                "toolbar.analyze",
                tr(&app.locale, "menu.analyze.analyze"),
                "ui.analyze.open",
                json!({}),
                analyze_reason,
                true,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.formula_bar",
                tr(&app.locale, "cmd.ui.table.openFormulaBar"),
                "ui.table.openFormulaBar",
                json!({}),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.randomize_groups",
                "Randomize Groups",
                "ui.table.randomizeGroups",
                json!({}),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.sample_rows",
                "Sample Rows",
                "ui.table.randomSampleRows",
                json!({}),
                None,
                false,
            ));
        }
        Tier2SheetContext::ResultSheet => {
            actions.push(make_ui_action(
                app,
                modal_enabled && on_result_sheet,
                "toolbar.edit_params",
                tr(&app.locale, "toolbar.parameters"),
                "ui.editParams.open",
                json!({}),
                edit_reason,
                true,
            ));
            let rs_id = resolve_result_sheet_for_shortcut(app, true);
            let mut change_data = make_ui_action(
                app,
                modal_enabled && rs_id.is_some(),
                "toolbar.change_data",
                tr(&app.locale, "toolbar.data_analyzed"),
                "ui.analyze.open",
                json!({}),
                edit_reason,
                false,
            );
            change_data.retarget_sheet_id = rs_id.clone();
            actions.push(change_data);
            actions.push(make_ui_action(
                app,
                modal_enabled && on_result_sheet,
                "toolbar.methods_report",
                tr(&app.locale, "cmd.ui.result.copyMethodsReport"),
                "ui.result.copyMethodsReport",
                json!({}),
                edit_reason,
                false,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled && on_result_sheet,
                "toolbar.diagnostics",
                tr(&app.locale, "cmd.ui.result.inspectDiagnostics"),
                "ui.result.inspectDiagnostics",
                json!({}),
                edit_reason,
                false,
            ));
            let is_clustering = rs_id
                .as_ref()
                .and_then(|id| doc.result_sheet(id))
                .is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })));
            if is_clustering {
                actions.push(make_ui_action(
                    app,
                    modal_enabled,
                    "toolbar.append_cluster",
                    tr(&app.locale, "cmd.ui.result.appendClusterColumn"),
                    "ui.result.appendClusterColumn",
                    json!({}),
                    None,
                    false,
                ));
            }
            actions.push(make_ui_action(
                app,
                analyze_ready,
                "toolbar.analyze",
                tr(&app.locale, "menu.analyze.analyze"),
                "ui.analyze.open",
                json!({}),
                analyze_reason,
                false,
            ));
        }
        Tier2SheetContext::GraphSheet => {
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.graph_type",
                tr(&app.locale, "toolbar.graph_type"),
                "ui.changeGraphType.open",
                json!({}),
                None,
                true,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.axes",
                tr(&app.locale, "toolbar.axes"),
                "ui.graph.axes.open",
                json!({}),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.format_graph",
                tr(&app.locale, "menu.graph.format_graph"),
                "ui.graph.format.open",
                json!({}),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.export_svg",
                tr(&app.locale, "toolbar.export_svg"),
                "ui.export.figure.open",
                json!({}),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                analyze_ready,
                "toolbar.analyze",
                tr(&app.locale, "menu.analyze.analyze"),
                "ui.analyze.open",
                json!({}),
                analyze_reason,
                false,
            ));
        }
        Tier2SheetContext::LayoutSheet => {
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.layout_composer",
                tr(&app.locale, "cmd.ui.layout.composer.open"),
                "ui.layout.composer.open",
                json!({}),
                None,
                true,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.rulers",
                tr(&app.locale, "toolbar.rulers"),
                "ui.rulers.toggle",
                json!({}),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.canvas_preset",
                "Page Preset",
                "ui.canvas.applyPreset",
                json!({ "preset": "ColumnAndHalf130mm" }),
                None,
                false,
            ));
            actions.push(make_ui_action(
                app,
                modal_enabled,
                "toolbar.export_svg",
                tr(&app.locale, "toolbar.export_svg"),
                "ui.export.figure.open",
                json!({}),
                None,
                false,
            ));
        }
        Tier2SheetContext::InfoSheet => {
            let info_id = doc.active_sheet_id.as_ref().map(|id| id.as_str()).unwrap_or_default();
            actions.push(make_engine_action(
                app,
                modal_enabled,
                "toolbar.info_add_constant",
                "+ Constant",
                "info.setConstant",
                json!({ "sheet_id": info_id, "name": "Constant", "value": 1.0 }),
                true,
                None,
                true,
            ));
            let first_ds = doc.data_sheets.first().map(|ds| ds.id.as_str().to_owned());
            let can_new_graph = first_ds.is_some();
            actions.push(make_engine_action(
                app,
                modal_enabled && can_new_graph,
                "toolbar.new_graph",
                tr(&app.locale, "menu.insert.new_graph"),
                "graph.create",
                first_ds.as_ref().map_or_else(|| json!({}), |id| json!({ "data_sheet_id": id })),
                can_new_graph,
                Some(if modal_enabled { "Create a DataSheet before creating a Graph" } else { "modal dialog is active" }),
                false,
            ));
            actions.push(make_ui_action(
                app,
                analyze_ready,
                "toolbar.analyze",
                tr(&app.locale, "menu.analyze.analyze"),
                "ui.analyze.open",
                json!({}),
                analyze_reason,
                false,
            ));
        }
    }

    actions
}

fn can_analyze_active(app: &UiApp) -> bool {
    let doc = app.document();
    doc.active_sheet_id.as_ref().is_some_and(|id| {
        doc.data_sheet(id).is_some()
            || doc.result_sheet(id).is_some_and(|rs| rs.analyzable_data.is_some() || rs.tabs.iter().any(|t| !t.rows.is_empty()))
    })
}

fn format_command_tooltip(tip: Option<CommandTooltip>, fallback: &str) -> String {
    match tip {
        Some(t) if !t.beginner.is_empty() => format!("{}\n{}", t.summary, t.beginner),
        Some(t) if !t.summary.is_empty() => t.summary.to_owned(),
        _ => fallback.to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn make_engine_action(
    app: &UiApp,
    enabled: bool,
    widget_id: &str,
    label: &str,
    command_id: &str,
    params: Value,
    base_enabled: bool,
    disabled_reason: Option<&str>,
    primary: bool,
) -> Tier2ActionSpec {
    let eff_enabled = enabled && base_enabled;
    Tier2ActionSpec {
        widget_id: widget_id.to_owned(),
        label: label.to_owned(),
        tooltip: format_command_tooltip(command_tooltip(app, command_id), label),
        command_id: command_id.to_owned(),
        params,
        is_engine: true,
        enabled: eff_enabled,
        disabled_reason: (!eff_enabled).then(|| disabled_reason.unwrap_or("modal dialog is active").to_owned()),
        primary,
        retarget_sheet_id: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_ui_action(
    app: &UiApp,
    enabled: bool,
    widget_id: &str,
    label: &str,
    command_id: &str,
    params: Value,
    disabled_reason: Option<&str>,
    primary: bool,
) -> Tier2ActionSpec {
    Tier2ActionSpec {
        widget_id: widget_id.to_owned(),
        label: label.to_owned(),
        tooltip: format_command_tooltip(command_tooltip(app, command_id), label),
        command_id: command_id.to_owned(),
        params,
        is_engine: false,
        enabled,
        disabled_reason: (!enabled).then(|| disabled_reason.unwrap_or("modal dialog is active").to_owned()),
        primary,
        retarget_sheet_id: None,
    }
}

fn emit_tier2_action_intent(act: &Tier2ActionSpec, intents: &mut PendingUiIntents) {
    if let Some(rs_id) = act.retarget_sheet_id.as_ref() {
        intents.push_open_retarget_dialog(rs_id.clone(), None);
        return;
    }
    if act.is_engine {
        intents.push_engine(&act.command_id, act.params.clone());
    } else {
        intents.push_ui(&act.command_id, act.params.clone());
    }
}

fn collect_two_up_pair(doc: &curvebook_doc::ProjectDocument, split: &SplitWorkspaceState) -> Option<(String, String)> {
    let mut ids = Vec::new();
    for ds in &doc.data_sheets {
        ids.push(ds.id.as_str().to_owned());
    }
    for rs in &doc.result_sheets {
        ids.push(rs.id.as_str().to_owned());
    }
    for gs in &doc.graph_sheets {
        ids.push(gs.id.as_str().to_owned());
    }
    for ls in &doc.layout_sheets {
        ids.push(ls.id.as_str().to_owned());
    }
    if ids.len() < 2 {
        return None;
    }
    let primary = doc.active_sheet_id.as_ref().or(split.primary_sheet_id.as_ref()).map_or_else(|| ids[0].clone(), |id| id.as_str().to_owned());
    let secondary = ids.iter().find(|id| *id != &primary).cloned().unwrap_or_else(|| ids[1].clone());
    Some((primary, secondary))
}

const fn preset_widget_id(preset: WorkspacePreset) -> &'static str {
    match preset {
        WorkspacePreset::Default => "toolbar.preset.default",
        WorkspacePreset::DataPrep => "toolbar.preset.data_prep",
        WorkspacePreset::Modeling => "toolbar.preset.modeling",
        WorkspacePreset::Publication => "toolbar.preset.publication",
        WorkspacePreset::Compact => "toolbar.preset.compact",
    }
}

fn clamp_popup_pos(ctx: &egui::Context, desired: Pos2, w: f32, h: f32) -> Pos2 {
    let screen = ctx.content_rect();
    let max_x = (screen.right() - w - 8.0).max(screen.left() + 4.0);
    let max_y = (screen.bottom() - h - 8.0).max(screen.top() + 4.0);
    Pos2::new(desired.x.clamp(screen.left() + 4.0, max_x), desired.y.clamp(screen.top() + 4.0, max_y))
}

fn popup_outside_clicked(ctx: &egui::Context, popup_rect: Rect, anchor_rect: Rect) -> bool {
    ctx.input(|i| i.pointer.any_released() && i.pointer.interact_pos().is_some_and(|p| !popup_rect.contains(p) && !anchor_rect.contains(p)))
}

fn localized_home_label(locale: &str) -> &'static str {
    match locale {
        "de" => "Start",
        "fr" => "Accueil",
        "es" => "Inicio",
        "it" => "Home",
        "pt-BR" => "Início",
        "nl" => "Start",
        "pl" => "Start",
        "ru" => "Главная",
        "tr" => "Giriş",
        "ja" => "ホーム",
        "ko" => "홈",
        "zh-CN" => "主页",
        "zh-TW" => "首頁",
        _ => "Home",
    }
}

fn localized_present_label(locale: &str) -> &'static str {
    match locale {
        "de" => "Präs.",
        "fr" => "Prés.",
        "es" => "Pres.",
        "ru" => "Показ",
        "ja" => "発表",
        "ko" => "발표",
        "zh-CN" => "演示",
        "zh-TW" => "簡報",
        _ => "Present",
    }
}

const fn modal_reason(enabled: bool) -> Option<&'static str> {
    if enabled { None } else { Some("modal dialog is active") }
}

fn layout_single_line_job(painter: &egui::Painter, text: &str, max_width: f32, color: Color32, font_size: f32) -> (Arc<egui::Galley>, bool) {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(FontId::proportional(font_size), color));
    job.wrap = egui::text::TextWrapping { max_width: max_width.max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let ellipsized = galley.elided;
    (galley, ellipsized)
}

fn render_single_line_title_badge(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, title: &str, max_w: f32) {
    let text_max = (max_w - 8.0).max(16.0);
    let (galley, ellipsized) = layout_single_line_job(ui.painter(), title, text_max, tokens.text_strong, 12.0);
    let w = (galley.size().x + 8.0).min(max_w).max(24.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::hover());
    ui.painter().with_clip_rect(rect.shrink(2.0)).galley(
        Pos2::new(rect.left() + 4.0, rect.center().y - galley.size().y * 0.5),
        galley,
        tokens.text_strong,
    );
    let ak_label = title.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, true, ak_label.clone()));
    let resp = if ellipsized { resp.on_hover_text(title) } else { resp };
    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: "toolbar.project_title",
                role: "label",
                label: title,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized,
                full_label: ellipsized.then_some(title),
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn chrome_button_fitted(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    command_id: Option<&str>,
    selected: bool,
    enabled: bool,
    disabled_reason: Option<&str>,
    max_w: f32,
) -> egui::Response {
    let text_max = (max_w - 14.0).max(12.0);
    let (galley, ellipsized) = layout_single_line_job(ui.painter(), label, text_max, tokens.text_primary, 12.0);
    let min_w = max_w.min(34.0);
    let w = (galley.size().x + 14.0).clamp(min_w, max_w);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = if !enabled {
        tokens.panel_bg
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let fg = if enabled { tokens.text_strong } else { tokens.text_disabled };
    let border = if !enabled {
        tokens.divider
    } else if selected {
        tokens.accent_strong
    } else {
        tokens.button_border
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().with_clip_rect(rect.shrink(3.0)).galley(
        Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    let ak_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, ak_label.clone()));
    let resp = if let Some(reason) = disabled_reason {
        resp.on_hover_text(reason)
    } else if ellipsized {
        resp.on_hover_text(label)
    } else {
        resp
    };

    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id,
                role: "button",
                label,
                command_id,
                value: None,
                enabled,
                disabled_reason,
                z_layer: ZLayer::Workspace,
                ellipsized,
                full_label: ellipsized.then_some(label),
            },
        );
    }
    resp
}

#[allow(clippy::too_many_arguments)]
fn popup_action_button(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    selected: bool,
    enabled: bool,
    disabled_reason: Option<&str>,
) -> egui::Response {
    let w = ui.available_width().max(120.0);
    let (galley, ellipsized) = layout_single_line_job(ui.painter(), label, (w - 14.0).max(40.0), tokens.text_primary, 12.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = if !enabled {
        tokens.panel_bg
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.surface_bg
    };
    let fg = if enabled { tokens.text_strong } else { tokens.text_disabled };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().with_clip_rect(rect.shrink(4.0)).galley(Pos2::new(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5), galley, fg);

    let ak_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, ak_label.clone()));
    let resp = if let Some(reason) = disabled_reason {
        resp.on_hover_text(reason)
    } else if ellipsized {
        resp.on_hover_text(label)
    } else {
        resp
    };

    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id,
                role: "button",
                label,
                command_id: None,
                value: Some(json!(selected)),
                enabled,
                disabled_reason,
                z_layer: ZLayer::DropdownMenu,
                ellipsized,
                full_label: ellipsized.then_some(label),
            },
        );
    }
    resp
}

#[allow(clippy::too_many_arguments)]
fn chrome_tab(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    selected: bool,
    enabled: bool,
    max_w: f32,
) -> egui::Response {
    let text_max_w = (max_w - 14.0).max(16.0);
    let (galley, ellipsized) = layout_single_line_job(ui.painter(), label, text_max_w, tokens.text_primary, 11.5);
    let w = (galley.size().x + 14.0).clamp(32.0_f32.min(max_w), max_w);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = if !enabled {
        tokens.panel_bg
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.surface_bg
    };
    let fg = if !enabled {
        tokens.text_disabled
    } else if selected {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let border = if !enabled {
        tokens.divider
    } else if selected {
        tokens.accent_strong
    } else {
        tokens.border
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().with_clip_rect(rect.shrink(3.0)).galley(
        Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    let ak_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Tab, enabled, selected, ak_label.clone()));
    let resp = if !enabled {
        resp.on_hover_text("modal dialog is active")
    } else if ellipsized {
        resp.on_hover_text(label)
    } else {
        resp
    };

    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id,
                role: "tab",
                label,
                command_id: None,
                value: Some(json!(selected)),
                enabled,
                disabled_reason: (!enabled).then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized,
                full_label: ellipsized.then_some(label),
            },
        );
    }
    resp
}
