//! Unified Top Chrome (`28 pt` optional in-window menu bar when `host_draws_menu_bar == false` +
//! `38 pt` unified contextual toolbar, `R20.9` + `R20.18`).

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontId, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, SplitWorkspaceState};
use crate::egui_shell::command_palette::{commit_unified_search_item, paint_inline_ghost_completion, paint_unified_search_row_content};
use crate::egui_shell::inspect_bridge::HybridCoexistenceDecision;
use crate::egui_shell::top_bar_fit::{RankedToolbarSlot, SearchVisibility, UnifiedToolbarFitInput, fit_unified_toolbar};
use crate::egui_shell::ui_kit::{
    ActionMenuAnchor, ActionMenuEntry, ActionMenuItem, ActionMenuSpec, LinkButtonSpec, TooltipSpec, action_menu_popup, link_button, show_tooltip,
    show_tooltip_for_control,
};
use crate::egui_shell::{
    ActionRank, FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, UiIntent,
    WorkspacePreset,
};
use crate::i18n::{LOCALES, locale_picker_label, normalize_locale};
use crate::inspect::ZLayer;
use crate::interaction::resolve_result_sheet_for_shortcut;
use crate::menus::{
    CommandTooltip, CuratedMenuNode, command_disabled_reason_for_app, command_tooltip, curated_menu_bar, default_command_action_rank,
    format_shortcut_for_platform,
};
use crate::search_engine::{SearchRecents, UnifiedSearchAction, compute_ghost_completion, query_unified};
use crate::tr;
use curvebook_doc::{AnalysisKind, InfoSheet, MultivariateSpec, SheetId, TableArchetype};

fn next_unused_constant_name(info: &InfoSheet) -> String {
    for idx in 1usize.. {
        let candidate = format!("Constant {idx}");
        if !info.constants.iter().any(|c| c.name.eq_ignore_ascii_case(&candidate)) {
            return candidate;
        }
    }
    String::from("Constant 1")
}

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
        "ui.workspacePreset.set" => Some(&[
            ("Default", "default"),
            ("Data Prep", "data_prep"),
            ("Modeling", "modeling"),
            ("Publication", "publication"),
            ("Compact", "compact"),
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

fn is_submenu_variant_selected(app: &UiApp, workspace_preset: WorkspacePreset, command_id: &str, variant: &str) -> Option<bool> {
    match command_id {
        "ui.theme.set" => {
            let active = match app.theme.mode {
                crate::theme::UiThemeMode::Light => "light",
                crate::theme::UiThemeMode::Dark => "dark",
                crate::theme::UiThemeMode::HighContrast => "high_contrast",
            };
            Some(active == variant)
        }
        "ui.previewFilter.set" => Some(app.preview_filter.as_str() == variant),
        "ui.workspacePreset.set" => Some(workspace_preset.as_str() == variant),
        _ => None,
    }
}

/// Hierarchical node inside [`InWindowMenuSectionParity`] (`R15-GUI-34`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InWindowMenuNode {
    Action(InWindowMenuItemParity),
    Separator,
    Submenu { id: String, title: String, children: Vec<Self> },
}

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
    pub nodes: Vec<InWindowMenuNode>,
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

fn convert_curated_nodes_for_window(nodes: Vec<CuratedMenuNode>, flat: &mut Vec<InWindowMenuItemParity>) -> Vec<InWindowMenuNode> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            CuratedMenuNode::Action(item) if is_menu_item_visible(item.command_id) => {
                let entry = InWindowMenuItemParity {
                    command_id: item.command_id.to_owned(),
                    label: item.localized_label.to_owned(),
                    shortcut: item.shortcut,
                    summary: item.summary.to_owned(),
                    beginner: item.beginner.to_owned(),
                    enabled: effective_menu_item_enabled(item.command_id, item.enabled),
                };
                flat.push(entry.clone());
                out.push(InWindowMenuNode::Action(entry));
            }
            CuratedMenuNode::Action(_) => {}
            CuratedMenuNode::Separator => out.push(InWindowMenuNode::Separator),
            CuratedMenuNode::Submenu { id, title, children, .. } => {
                let converted = convert_curated_nodes_for_window(children, flat);
                if !converted.is_empty() {
                    out.push(InWindowMenuNode::Submenu { id: id.to_owned(), title, children: converted });
                }
            }
        }
    }
    out
}

/// Build canonical localized hierarchical menu sections from [`curated_menu_bar`] for `egui::MenuBar`
/// and desktop menu-bar parity checks.
#[must_use]
pub fn collect_in_window_menu_sections(app: &UiApp) -> Vec<InWindowMenuSectionParity> {
    curated_menu_bar(app)
        .into_iter()
        .map(|sec| {
            let mut items = Vec::new();
            let nodes = convert_curated_nodes_for_window(sec.nodes, &mut items);
            InWindowMenuSectionParity { menu_key: sec.menu_key.to_owned(), title: sec.title.to_owned(), items, nodes }
        })
        .collect()
}

/// Persistent `egui_shell` top chrome state (split/preset/overflow popups, inline search, and active in-window menu).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopChromeEguiState {
    pub split_menu_open: bool,
    pub preset_menu_open: bool,
    pub overflow_menu_open: bool,
    pub active_in_window_menu: Option<String>,
    pub search_query: String,
    pub search_dropdown_open: bool,
    pub search_selected_index: usize,
}

impl TopChromeEguiState {
    fn close_all_popups(&mut self) {
        self.split_menu_open = false;
        self.preset_menu_open = false;
        self.overflow_menu_open = false;
        self.active_in_window_menu = None;
        self.search_dropdown_open = false;
    }

    fn close_menus_keep_search(&mut self) {
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
    pub search_recents: &'a SearchRecents,
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
    shortcut: String,
    tooltip: String,
    command_id: String,
    params: Value,
    is_engine: bool,
    enabled: bool,
    disabled_reason: Option<String>,
    primary: bool,
    rank: ActionRank,
    retarget_sheet_id: Option<SheetId>,
}

#[derive(Debug, Clone, Copy)]
struct Tier1PopupsInput<'a> {
    app: &'a UiApp,
    tokens: &'a Tokens,
    enabled: bool,
    search_recents: &'a SearchRecents,
}

#[derive(Clone, Copy)]
struct Tier1BarInput<'a> {
    app: &'a UiApp,
    ctx_in: TopChromeContext<'a>,
    enabled: bool,
}

#[derive(Default)]
struct Tier1Anchors {
    split: Option<Rect>,
    search_capsule: Option<Rect>,
}

#[derive(Default)]
struct Tier2OverflowState {
    items: Vec<Tier2ActionSpec>,
    anchor: Option<Rect>,
}

#[derive(Clone, Copy)]
struct TopMenuButtonSpec<'a> {
    app: &'a UiApp,
    tokens: &'a Tokens,
    workspace_preset: WorkspacePreset,
    sec: &'a InWindowMenuSectionParity,
    sec_w: f32,
    enabled: bool,
}

#[derive(Clone, Copy)]
struct MenuRenderCtx<'a> {
    app: &'a UiApp,
    tokens: &'a Tokens,
    locale: &'a str,
    workspace_preset: WorkspacePreset,
}

struct Tier2ActionInput<'a> {
    widget_id: &'a str,
    label: &'a str,
    command_id: &'a str,
    params: Value,
    enabled: bool,
    disabled_reason: Option<&'a str>,
    primary: bool,
}

#[derive(Clone, Copy)]
struct ChromeButtonSpec<'a> {
    widget_id: &'a str,
    label: &'a str,
    command_id: Option<&'a str>,
    shortcut: Option<&'a str>,
    selected: bool,
    enabled: bool,
    disabled_reason: Option<&'a str>,
    tooltip_title: Option<&'a str>,
    tooltip: Option<&'a str>,
    max_w: f32,
}

/// Render optional in-window menu bar (`28 pt`, strictly when `!host_draws_menu_bar`) and the
/// unified toolbar (`38 pt`) cleanly (`R20.9` + `R20.18`).
///
/// Preserves exact widget IDs (`toolbar.home`, `toolbar.new`, `toolbar.undo`, `toolbar.redo`,
/// `toolbar.command_palette`, `toolbar.search_input`, `toolbar.split`, `toolbar.analyze`,
/// `toolbar.edit_params`, `toolbar.overflow`).
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

    let show_menu_bar = !ctx_in.host_draws_menu_bar && ctx_in.layout.tier1_app_bar_h_pt > 0.0;
    let tier1_h = if show_menu_bar { ctx_in.layout.tier1_app_bar_h_pt.max(24.0) } else { 0.0 };
    let tier2_h = ctx_in.layout.tier2_action_strip_h_pt.max(tokens.metrics.min_toolbar_target_pt);
    let bar_input = Tier1BarInput { app, ctx_in, enabled };
    let mut tier1_anchors = Tier1Anchors::default();
    let mut tier2_overflow = Tier2OverflowState::default();

    let panel_resp = egui::Panel::top("egui_shell.top_chrome")
        .exact_size(tier1_h + tier2_h)
        .frame(egui::Frame::NONE.fill(tokens.app_bar).inner_margin(egui::Margin::symmetric(8, 0)))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if show_menu_bar {
                let tier1_resp = ui.allocate_ui(Vec2::new(ui.available_width(), tier1_h), |ui| {
                    render_tier1_menu_bar(ui, bar_input, state, meta, intents);
                });
                report.tier1_rect_pt = Some(tier1_resp.response.rect);

                let div_y = tier1_resp.response.rect.bottom();
                let panel_w = tier1_resp.response.rect.width();
                ui.painter().line_segment(
                    [Pos2::new(tier1_resp.response.rect.left() - 8.0, div_y), Pos2::new(tier1_resp.response.rect.left() + panel_w + 8.0, div_y)],
                    Stroke::new(1.0, tokens.divider),
                );
            } else {
                state.active_in_window_menu = None;
                report.tier1_rect_pt = None;
            }

            let tier2_resp = ui.allocate_ui(Vec2::new(ui.available_width(), tier2_h), |ui| {
                render_unified_toolbar(ui, bar_input, state, &mut tier1_anchors, &mut tier2_overflow, meta, intents);
            });
            report.tier2_rect_pt = Some(tier2_resp.response.rect);

            let bottom_y = tier2_resp.response.rect.bottom();
            let panel_w = tier2_resp.response.rect.width();
            ui.painter().line_segment(
                [Pos2::new(tier2_resp.response.rect.left() - 8.0, bottom_y), Pos2::new(tier2_resp.response.rect.left() + panel_w + 8.0, bottom_y)],
                Stroke::new(1.0, tokens.divider),
            );
        });

    report.total_height_pt = panel_resp.response.rect.height();
    report.overflow_item_count = tier2_overflow.items.len();
    if tier2_overflow.items.is_empty() {
        state.overflow_menu_open = false;
    }

    let popup_ctx = Tier1PopupsInput { app, tokens, enabled, search_recents: ctx_in.search_recents };
    if state.search_dropdown_open
        && enabled
        && let Some(anchor) = tier1_anchors.search_capsule
    {
        render_inline_search_dropdown(ui.ctx(), &popup_ctx, state, anchor, meta, intents);
    }
    if state.split_menu_open
        && let Some(anchor) = tier1_anchors.split
    {
        render_split_preset_popup(ui.ctx(), &popup_ctx, state, anchor, meta, intents);
    }
    if state.overflow_menu_open
        && let Some(anchor) = tier2_overflow.anchor
        && !tier2_overflow.items.is_empty()
    {
        render_tier2_overflow_popup(ui.ctx(), &popup_ctx, state, &tier2_overflow.items, anchor, meta, intents);
    }

    report.menu_popup_open = state.active_in_window_menu.is_some()
        || state.split_menu_open
        || state.preset_menu_open
        || state.overflow_menu_open
        || state.search_dropdown_open;
    report
}

fn render_tier1_menu_bar(
    ui: &mut egui::Ui,
    input: Tier1BarInput<'_>,
    state: &mut TopChromeEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let Tier1BarInput { app, ctx_in, enabled: _ } = input;
    let tokens = ctx_in.tokens;
    let total_w = ui.available_width().max(40.0);

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        state.active_in_window_menu = None;
        let sections = collect_in_window_menu_sections(app);
        let title = if ctx_in.dirty { format!("{} •", ctx_in.project_title) } else { ctx_in.project_title.to_owned() };
        let (title_galley, _) = layout_single_line_job(ui.painter(), &title, 220.0, tokens.text_strong, 12.0);
        let desired_title_w = (title_galley.size().x + 12.0).clamp(48.0, 220.0);

        let all_menus_w: f32 = sections
            .iter()
            .map(|sec| {
                let (galley, _) = layout_single_line_job(ui.painter(), &sec.title, 96.0, tokens.text_primary, 12.0);
                (galley.size().x + 14.0).clamp(32.0, 108.0) + 4.0
            })
            .sum();
        let title_reserve = if total_w >= all_menus_w + 64.0 { (total_w - all_menus_w - 8.0).min(desired_title_w) } else { 0.0 };
        let menu_budget = (total_w - title_reserve - if title_reserve > 0.0 { 8.0 } else { 0.0 }).max(54.0);
        render_inline_menu_bar(ui, input, &sections, menu_budget, state, meta, intents);

        let title_slack = (ui.available_width() - 4.0).max(0.0);
        if title_reserve > 0.0 && title_slack >= 48.0 {
            let filler = (ui.available_width() - title_slack.min(desired_title_w)).max(0.0);
            if filler > 0.0 {
                ui.add_space(filler);
            }
            render_single_line_title_badge(ui, tokens, meta, &title, title_slack.min(220.0));
        }
    });
}

fn render_inline_menu_bar(
    ui: &mut egui::Ui,
    input: Tier1BarInput<'_>,
    sections: &[InWindowMenuSectionParity],
    budget_w: f32,
    state: &mut TopChromeEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let Tier1BarInput { app, ctx_in, enabled } = input;
    let tokens = ctx_in.tokens;
    if sections.is_empty() {
        return;
    }

    let overflow_lbl = "⋯";
    let overflow_btn_w = 34.0;
    let mut visible_count = 0usize;
    let mut acc_w = 0.0_f32;
    let mut measured_widths = Vec::with_capacity(sections.len());
    for sec in sections {
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
    ui.allocate_ui(Vec2::new(actual_w, tokens.metrics.min_hit_target_pt), |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (sec, &sec_w) in sections.iter().take(visible_count).zip(measured_widths.iter()) {
                render_top_menu_button(
                    ui,
                    TopMenuButtonSpec { app, tokens, workspace_preset: ctx_in.workspace_preset, sec, sec_w, enabled },
                    state,
                    meta,
                    intents,
                );
            }
            if visible_count < sections.len() {
                let ov_btn = egui::Button::new(overflow_lbl)
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::splat(tokens.metrics.min_hit_target_pt));
                let ov_resp = ui.add_enabled(enabled, ov_btn);
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
                    let mctx = MenuRenderCtx { app, tokens, locale: tokens.locale, workspace_preset: ctx_in.workspace_preset };
                    egui::Popup::menu(&ov_resp).show(|ui| {
                        state.active_in_window_menu = Some(String::from("menu.bar.overflow"));
                        for sec in &sections[visible_count..] {
                            ui.menu_button(&sec.title, |ui| {
                                render_section_menu_nodes(ui, mctx, &sec.nodes, meta, intents);
                            });
                        }
                    });
                }
            }
        });
    });
}

fn render_top_menu_button(
    ui: &mut egui::Ui,
    spec: TopMenuButtonSpec<'_>,
    state: &mut TopChromeEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let TopMenuButtonSpec { app, tokens, workspace_preset, sec, sec_w, enabled } = spec;
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
    let r = CornerRadius::same(tokens.metrics.radius_md);
    if is_open {
        ui.painter().rect(resp.rect, r, bg, Stroke::new(1.5, tokens.focus_ring), StrokeKind::Inside);
    } else if resp.hovered() && enabled {
        ui.painter().rect(resp.rect, r, bg, Stroke::new(1.0, tokens.hover_border), StrokeKind::Inside);
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
        let mctx = MenuRenderCtx { app, tokens, locale: tokens.locale, workspace_preset };
        egui::Popup::menu(&resp).show(|ui| {
            state.active_in_window_menu = Some(sec.menu_key.clone());
            render_section_menu_nodes(ui, mctx, &sec.nodes, meta, intents);
            let popup_rect = ui.min_rect().intersect(ui.clip_rect()).intersect(meta.viewport_rect);
            if popup_rect.is_positive() {
                let popup_id = ui.scope_id().with(("popup", &sec.menu_key));
                let p_resp = ui.interact(popup_rect, popup_id, Sense::hover());
                let p_lbl = sec.title.clone();
                p_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Menu, true, p_lbl.clone()));
                meta.record(FrameInspectNodeMeta {
                    egui_id: p_resp.id,
                    accesskit_id: p_resp.id.accesskit_id(),
                    widget_id: format!("popup.{}", sec.menu_key),
                    role: String::from("menu"),
                    label: sec.title.clone(),
                    command_id: None,
                    value: Some(json!(true)),
                    enabled: true,
                    disabled_reason: None,
                    unclipped_rect: ui.min_rect(),
                    clipped_rect: popup_rect,
                    z_layer: ZLayer::DropdownMenu,
                    ellipsized: false,
                    full_label: None,
                    action_rank: None,
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

fn render_section_menu_nodes(
    ui: &mut egui::Ui,
    mctx: MenuRenderCtx<'_>,
    nodes: &[InWindowMenuNode],
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    for node in nodes {
        match node {
            InWindowMenuNode::Action(item) => render_menu_leaf_item(ui, mctx, item, meta, intents),
            InWindowMenuNode::Separator => {
                ui.separator();
            }
            InWindowMenuNode::Submenu { id, title, children } => {
                let sub = ui.menu_button(title, |ui| {
                    render_section_menu_nodes(ui, mctx, children, meta, intents);
                });
                record_menu_popup_response(
                    ui,
                    &sub.response,
                    meta,
                    MenuPopupInspect { widget_id: id, label: title, command_id: None, selected: None, enabled: true },
                );
            }
        }
    }
}

fn render_menu_leaf_item(
    ui: &mut egui::Ui,
    mctx: MenuRenderCtx<'_>,
    item: &InWindowMenuItemParity,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    // R20.21 Locked Decision #13: GUI tooltips surface CommandTooltip.summary only (preserving .beginner for MCP/docs).
    let plat_shortcut = (!item.shortcut.is_empty()).then(|| format_shortcut_for_platform(&item.shortcut));
    let summary = (!item.summary.trim().is_empty()).then_some(item.summary.as_str());
    let disabled_reason = (!item.enabled).then(|| command_disabled_reason_for_app(mctx.app, &item.command_id));
    if let Some(variants) = submenu_variants_for_command(&item.command_id) {
        let sub = ui.menu_button(&item.label, |ui| {
            for &(v_label, variant) in variants {
                let is_selected = is_submenu_variant_selected(mctx.app, mctx.workspace_preset, &item.command_id, variant);
                let mut btn = egui::Button::new(v_label);
                if is_selected == Some(true) {
                    btn = btn.selected(true);
                }
                let opt_shortcut = (item.command_id == "ui.canvas.applyPreset" && !item.shortcut.is_empty() && variant == "SingleColumn88mm")
                    .then(|| format_shortcut_for_platform(&item.shortcut));
                if let Some(sc) = opt_shortcut.as_deref() {
                    btn = btn.shortcut_text(sc);
                }
                let mut resp = ui.add_enabled(item.enabled, btn);
                resp = show_tooltip_for_control(
                    &resp,
                    mctx.tokens,
                    v_label,
                    false,
                    TooltipSpec { title: v_label, shortcut: opt_shortcut.as_deref(), summary, disabled_reason },
                );
                record_menu_popup_response(
                    ui,
                    &resp,
                    meta,
                    MenuPopupInspect {
                        widget_id: &format!("popup.menu.{}.opt.{}", item.command_id, variant),
                        label: v_label,
                        command_id: Some(&item.command_id),
                        selected: is_selected,
                        enabled: item.enabled,
                    },
                );
                if resp.clicked() {
                    push_menu_command_intent(mctx.app, &item.command_id, Some(variant), intents);
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
        let active_canonical = normalize_locale(mctx.locale);
        let sub = ui
            .add_enabled_ui(item.enabled, |ui| {
                ui.menu_button(&item.label, |ui| {
                    let scroll_out = egui::ScrollArea::vertical().id_salt("popup.menu.ui.locale.set").max_height(240.0).show(ui, |ui| {
                        for (idx, &loc) in LOCALES.iter().enumerate() {
                            let canonical = normalize_locale(loc);
                            let caption = locale_picker_label(loc);
                            let mut opt_resp = ui.add_enabled(item.enabled, egui::Button::new(&caption));
                            opt_resp = show_tooltip_for_control(
                                &opt_resp,
                                mctx.tokens,
                                &caption,
                                false,
                                TooltipSpec { title: &caption, shortcut: None, summary, disabled_reason },
                            );
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
                            if opt_resp.clicked() && item.enabled {
                                push_menu_command_intent(mctx.app, &item.command_id, Some(canonical), intents);
                                ui.close();
                            }
                        }
                    });
                    let popup_rect = scroll_out.inner_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
                    if popup_rect.is_positive() {
                        let popup_id = ui.scope_id().with("popup.menu.ui.locale.set");
                        let p_resp = ui.interact(popup_rect, popup_id, Sense::hover());
                        let p_lbl = item.label.clone();
                        let p_en = item.enabled;
                        p_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Menu, p_en, p_lbl.clone()));
                        meta.record(FrameInspectNodeMeta {
                            egui_id: p_resp.id,
                            accesskit_id: p_resp.id.accesskit_id(),
                            widget_id: String::from("popup.menu.ui.locale.set"),
                            role: String::from("menu"),
                            label: item.label.clone(),
                            command_id: Some(String::from("ui.locale.set")),
                            value: None,
                            enabled: item.enabled,
                            disabled_reason: None,
                            unclipped_rect: scroll_out.inner_rect,
                            clipped_rect: popup_rect,
                            z_layer: ZLayer::DropdownMenu,
                            ellipsized: false,
                            full_label: None,
                            action_rank: None,
                        });
                    }
                })
            })
            .inner;
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
    if let Some(sc) = plat_shortcut.as_deref() {
        btn = btn.shortcut_text(sc);
    }
    let mut resp = ui.add_enabled(item.enabled, btn);
    resp = show_tooltip_for_control(
        &resp,
        mctx.tokens,
        &item.label,
        false,
        TooltipSpec { title: &item.label, shortcut: plat_shortcut.as_deref(), summary, disabled_reason },
    );
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
        push_menu_command_intent(mctx.app, &item.command_id, None, intents);
        ui.close();
    }
}

fn push_menu_command_intent(app: &UiApp, command_id: &str, variant: Option<&str>, intents: &mut PendingUiIntents) {
    if let Some(var) = variant {
        match command_id {
            "ui.theme.set" => intents.push_ui("ui.theme.set", json!({ "mode": var })),
            "ui.locale.set" => intents.push_ui("ui.locale.set", json!({ "locale": var })),
            "ui.previewFilter.set" => intents.push_ui("ui.previewFilter.set", json!({ "filter": var })),
            "ui.workspacePreset.set" => {
                if let Some(preset) = WorkspacePreset::parse(var) {
                    intents.push(UiIntent::ApplyWorkspacePreset { preset });
                }
            }
            "ui.canvas.applyPreset" | "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => {
                let mut params = json!({ "preset": var, "resize_mode": "KeepTypographyPt" });
                if let Some(sid) = super::command_palette::resolve_renderable_sheet_id_for_menu(app.document())
                    && let Some(obj) = params.as_object_mut()
                {
                    obj.insert(String::from("sheet_id"), json!(sid.as_str()));
                }
                intents.push_ui("ui.canvas.applyPreset", params);
            }
            other => intents.push_ui(other, json!({ "value": var })),
        }
        return;
    }
    if crate::menus::is_desktop_host_routed_command(command_id) {
        intents.push_ui(command_id, json!({}));
        return;
    }
    let is_ui = command_id.starts_with("ui.") || command_id.starts_with("app.");
    match super::command_palette::resolve_shell_command_route(app, command_id, is_ui) {
        Some(super::command_palette::PaletteCommandRoute::UiCommand { command_id, params }) => {
            intents.push_ui(command_id, params);
        }
        Some(super::command_palette::PaletteCommandRoute::EngineCommand { command_id, params }) => {
            intents.push_engine(command_id, params);
        }
        Some(super::command_palette::PaletteCommandRoute::DirectIntent(intent)) => {
            intents.push(intent);
        }
        None => {}
    }
}

#[derive(Clone, Copy)]
struct UnifiedRightGroupSpec<'a> {
    input: Tier1BarInput<'a>,
    search_vis: SearchVisibility,
    split_toggle: bool,
    split_w: f32,
}

fn render_unified_right_group(
    ui: &mut egui::Ui,
    spec: UnifiedRightGroupSpec<'_>,
    state: &mut TopChromeEguiState,
    anchors: &mut Tier1Anchors,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let UnifiedRightGroupSpec { input: Tier1BarInput { app, ctx_in, enabled }, search_vis, split_toggle, split_w } = spec;
    let tokens = ctx_in.tokens;
    if !matches!(search_vis, SearchVisibility::Hidden) {
        let base_label = tr(&app.locale, "toolbar.palette");
        let plat_shortcut = format_shortcut_for_platform(app.shortcuts.active_combo("ui.palette.open").unwrap_or("Cmd+K"));
        let selected = matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette);
        let tip_text = command_tooltip(app, "ui.palette.open").map(|t| t.summary).unwrap_or("");
        match search_vis {
            SearchVisibility::Field => {
                let capsule_rect = render_inline_search_capsule(
                    ui,
                    InlineSearchCapsuleSpec {
                        app,
                        tokens,
                        search_recents: ctx_in.search_recents,
                        base_label,
                        plat_shortcut: &plat_shortcut,
                        tip_text,
                        selected,
                        enabled,
                    },
                    state,
                    meta,
                    intents,
                );
                anchors.search_capsule = Some(capsule_rect);
            }
            SearchVisibility::Icon | SearchVisibility::Hidden => {
                state.search_dropdown_open = false;
                let pal_resp = render_command_palette_icon_button(
                    ui,
                    tokens,
                    meta,
                    CommandPaletteIconSpec { label: base_label, shortcut: &plat_shortcut, summary: tip_text, selected, enabled },
                );
                if pal_resp.clicked() && enabled {
                    state.close_all_popups();
                    intents.push(UiIntent::SetPaletteOpen { open: true });
                }
            }
        }
    } else {
        state.search_dropdown_open = false;
    }

    if split_toggle {
        let split_lbl = "Split";
        let split_sc = app.shortcuts.active_combo("ui.split.toggle").filter(|s| !s.trim().is_empty()).map(format_shortcut_for_platform);
        let split_resp = chrome_button_fitted(
            ui,
            tokens,
            meta,
            ChromeButtonSpec {
                widget_id: "toolbar.split",
                label: split_lbl,
                command_id: Some("ui.split.toggle"),
                shortcut: split_sc.as_deref(),
                selected: app.split.enabled || state.split_menu_open,
                enabled,
                disabled_reason: modal_reason(enabled),
                tooltip_title: None,
                tooltip: Some(tr(&app.locale, "tooltip.split.summary")),
                max_w: split_w.max(34.0),
            },
        );
        anchors.split = Some(split_resp.rect);
        if split_resp.clicked() && enabled {
            let next = !state.split_menu_open;
            state.close_all_popups();
            state.split_menu_open = next;
        }
    }
}

#[derive(Clone, Copy)]
struct InlineSearchCapsuleSpec<'a> {
    app: &'a UiApp,
    tokens: &'a Tokens,
    search_recents: &'a SearchRecents,
    base_label: &'a str,
    plat_shortcut: &'a str,
    tip_text: &'a str,
    selected: bool,
    enabled: bool,
}

fn render_inline_search_capsule(
    ui: &mut egui::Ui,
    spec: InlineSearchCapsuleSpec<'_>,
    state: &mut TopChromeEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> Rect {
    let InlineSearchCapsuleSpec { app, tokens, search_recents, base_label, plat_shortcut, tip_text, selected, enabled } = spec;
    let search_input_id = Id::unique("egui_shell.toolbar.search_input");
    let input_focused = ui.ctx().memory(|m| m.focused()) == Some(search_input_id);

    if enabled && (state.search_dropdown_open || input_focused) {
        consume_inline_search_keys(ui.ctx(), app, state, search_recents, search_input_id, intents);
    }

    let sc_font_pt = crate::egui_shell::fonts::SHORTCUT_PILL_FONT_PT;
    let (sc_galley, _) = layout_single_line_job(ui.painter(), plat_shortcut, 64.0, tokens.text_muted, sc_font_pt);
    let kbd_btn_w = (sc_galley.size().x + 12.0).clamp(32.0, 68.0);
    let capsule_w = 176.0;
    let capsule_h = tokens.metrics.min_toolbar_target_pt;
    let input_w = (capsule_w - kbd_btn_w - 10.0).max(72.0);

    let frame_stroke = if input_focused && enabled { Stroke::new(1.5, tokens.focus_ring) } else { Stroke::new(1.0, tokens.input_border) };
    let frame_fill = if enabled { tokens.surface_bg } else { tokens.panel_bg };

    let capsule_resp = egui::Frame::NONE
        .fill(frame_fill)
        .stroke(frame_stroke)
        .corner_radius(CornerRadius::same(tokens.metrics.radius_md))
        .inner_margin(egui::Margin::symmetric(4, 1))
        .show(ui, |ui| {
            ui.set_height(capsule_h - 4.0);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let prev_query = state.search_query.clone();
                let edit_resp = ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut state.search_query)
                        .id(search_input_id)
                        .frame(egui::Frame::NONE)
                        .hint_text("Search sheets, analyses, commands")
                        .desired_width(input_w),
                );
                if enabled && (edit_resp.gained_focus() || edit_resp.changed() || edit_resp.clicked()) {
                    state.close_menus_keep_search();
                    state.search_dropdown_open = true;
                }
                if state.search_query != prev_query {
                    state.search_selected_index = 0;
                }
                if enabled && !state.search_query.is_empty() && (edit_resp.has_focus() || state.search_dropdown_open) {
                    let top_hits = query_unified(app, &state.search_query, 1, search_recents);
                    if let Some(suffix) = compute_ghost_completion(&state.search_query, top_hits.first()) {
                        paint_inline_ghost_completion(ui, &edit_resp, &state.search_query, &suffix, tokens);
                    }
                }
                meta.record_response(
                    ui,
                    &edit_resp,
                    FrameInspectEntryInput {
                        widget_id: "toolbar.search_input",
                        role: "textbox",
                        label: "Search sheets, analyses, commands",
                        command_id: None,
                        value: Some(json!(state.search_query)),
                        enabled,
                        disabled_reason: modal_reason(enabled),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );

                let pill_h = (capsule_h - tokens.metrics.space_md).max(20.0);
                let (pal_rect, mut pal_resp) =
                    ui.allocate_exact_size(Vec2::new(kbd_btn_w, pill_h), if enabled { Sense::click() } else { Sense::hover() });
                if !enabled {
                    pal_resp = pal_resp.on_hover_cursor(egui::CursorIcon::Default);
                }
                let pill_radius = CornerRadius::same(tokens.metrics.radius_sm);
                super::widgets::paint_interactive_chrome(
                    ui,
                    pal_rect,
                    tokens,
                    super::widgets::InteractiveChromeSpec {
                        radius: pill_radius,
                        base_bg: if enabled { tokens.surface_recessed } else { tokens.panel_bg },
                        base_border: matches!(tokens.mode, crate::theme::UiThemeMode::HighContrast).then_some(tokens.border),
                        hovered: pal_resp.hovered(),
                        selected,
                        pressed: pal_resp.is_pointer_button_down_on(),
                        enabled,
                    },
                );
                if enabled && pal_resp.has_focus() {
                    ui.painter().rect_stroke(
                        pal_rect,
                        pill_radius,
                        Stroke::new(tokens.metrics.stroke_focus_pt, tokens.focus_ring),
                        StrokeKind::Inside,
                    );
                }
                let pill_fg = if !enabled {
                    tokens.text_disabled
                } else if selected {
                    tokens.text_strong
                } else {
                    tokens.text_muted
                };
                let (pill_galley, _) = layout_single_line_job(ui.painter(), plat_shortcut, (kbd_btn_w - 6.0).max(16.0), pill_fg, sc_font_pt);
                ui.painter().with_clip_rect(pal_rect.shrink(1.0)).galley(
                    Pos2::new(pal_rect.center().x - pill_galley.size().x * 0.5, pal_rect.center().y - pill_galley.size().y * 0.5),
                    pill_galley,
                    pill_fg,
                );
                let pal_resp = show_tooltip(
                    &pal_resp,
                    tokens,
                    TooltipSpec {
                        title: base_label,
                        shortcut: Some(plat_shortcut),
                        summary: (!tip_text.is_empty()).then_some(tip_text),
                        disabled_reason: modal_reason(enabled),
                    },
                );
                let ak_lbl = base_label.to_owned();
                pal_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, ak_lbl.clone()));
                meta.record_response(
                    ui,
                    &pal_resp,
                    FrameInspectEntryInput {
                        widget_id: "toolbar.command_palette",
                        role: "button",
                        label: base_label,
                        command_id: Some("ui.palette.open"),
                        value: Some(json!(selected)),
                        enabled,
                        disabled_reason: modal_reason(enabled),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if pal_resp.clicked() && enabled {
                    let carry = state.search_query.clone();
                    state.close_all_popups();
                    if carry.is_empty() {
                        intents.push(UiIntent::SetPaletteOpen { open: true });
                    } else {
                        intents.push(UiIntent::OpenPaletteWithQuery { query: carry });
                    }
                }
            });
        });

    capsule_resp.response.rect
}

fn consume_inline_search_keys(
    ctx: &egui::Context,
    app: &UiApp,
    state: &mut TopChromeEguiState,
    search_recents: &SearchRecents,
    search_input_id: Id,
    intents: &mut PendingUiIntents,
) {
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        state.search_dropdown_open = false;
        ctx.memory_mut(|m| m.surrender_focus(search_input_id));
        return;
    }
    let items = query_unified(app, &state.search_query, 10, search_recents);
    if !items.is_empty() && state.search_selected_index >= items.len() {
        state.search_selected_index = items.len() - 1;
    }
    if let Some(suffix) = compute_ghost_completion(&state.search_query, items.first())
        && !suffix.is_empty()
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab))
    {
        state.search_query.push_str(&suffix);
        state.search_selected_index = 0;
        return;
    }
    if !items.is_empty() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            state.search_selected_index = (state.search_selected_index + 1) % items.len();
            state.search_dropdown_open = true;
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            state.search_selected_index = if state.search_selected_index == 0 { items.len() - 1 } else { state.search_selected_index - 1 };
            state.search_dropdown_open = true;
        }
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
        && let Some(item) = items.get(state.search_selected_index)
        && item.enabled
    {
        commit_unified_search_item(app, item, intents);
        state.search_dropdown_open = false;
        state.search_query.clear();
        ctx.memory_mut(|m| m.surrender_focus(search_input_id));
    }
}

fn render_inline_search_dropdown(
    ctx: &egui::Context,
    input: &Tier1PopupsInput<'_>,
    state: &mut TopChromeEguiState,
    anchor: Rect,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let items = query_unified(input.app, &state.search_query, 10, input.search_recents);
    if items.is_empty() {
        state.search_selected_index = 0;
    } else if state.search_selected_index >= items.len() {
        state.search_selected_index = items.len() - 1;
    }

    let screen = ctx.content_rect();
    let tokens = input.tokens;
    let plat_shortcut = format_shortcut_for_platform(input.app.shortcuts.active_combo("ui.palette.open").unwrap_or("Cmd+K"));
    let footer_label = format!("Open full search ({plat_shortcut})");
    let empty_label = "No matching sheets, analyses, or commands.";
    let measured_w = ctx.fonts_mut(|f| {
        let title_font = FontId::proportional(tokens.metrics.font_ui_body_pt);
        let small_font = FontId::proportional(tokens.metrics.font_ui_small_pt);
        let mut max_w = f.layout_no_wrap(footer_label.clone(), small_font.clone(), tokens.text_primary).size().x.ceil() + 24.0;
        if items.is_empty() {
            max_w = max_w.max(f.layout_no_wrap(empty_label.to_owned(), title_font.clone(), tokens.text_muted).size().x.ceil() + 24.0);
        } else {
            for item in &items {
                let bw = (f.layout_no_wrap(item.category_badge.clone(), small_font.clone(), tokens.text_primary).size().x + 10.0).clamp(42.0, 72.0);
                let tw = f.layout_no_wrap(item.title.clone(), title_font.clone(), tokens.text_strong).size().x.ceil();
                let sw = if item.subtitle.is_empty() {
                    0.0
                } else {
                    f.layout_no_wrap(format!("· {}", item.subtitle), small_font.clone(), tokens.text_muted).size().x.ceil() + 8.0
                };
                let scw = if item.shortcut.is_empty() {
                    0.0
                } else {
                    let psc = format_shortcut_for_platform(&item.shortcut);
                    f.layout_no_wrap(psc, small_font.clone(), tokens.text_muted).size().x.ceil() + 18.0
                };
                max_w = max_w.max(bw + tw + sw + scw + 36.0);
            }
        }
        max_w
    });
    let max_popup_w = (screen.width() - 16.0).clamp(180.0, 420.0);
    let min_popup_w = 180.0_f32.min(max_popup_w);
    let popup_w = (measured_w + 14.0).ceil().clamp(min_popup_w, max_popup_w);
    let inner_w = (popup_w - 14.0).max(120.0);
    let est_h = (if items.is_empty() { 64.0 } else { (items.len() as f32) * 32.0 + 44.0 }).min((screen.height() - 16.0).max(80.0));
    let desired_x = (anchor.right() - popup_w).clamp(screen.left() + 4.0, (screen.right() - popup_w - 4.0).max(screen.left() + 4.0));
    let desired_y = (anchor.bottom() + 4.0).clamp(screen.top() + 4.0, (screen.bottom() - est_h - 4.0).max(screen.top() + 4.0));

    // popup-size-exempt: bounded live-search results popover with custom badge/subtitle rows and explicit set_width
    let area_resp = egui::Area::new(Id::unique("egui_shell.toolbar.search_dropdown"))
        .fixed_pos(Pos2::new(desired_x.round(), desired_y.round()))
        .default_size(Vec2::new(popup_w, est_h))
        .fade_in(false)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(CornerRadius::same(tokens.metrics.radius_lg))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(inner_w);
                    ui.set_max_width(inner_w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if items.is_empty() {
                        ui.colored_label(tokens.text_muted, empty_label);
                    } else {
                        for (idx, item) in items.iter().enumerate() {
                            let is_selected = idx == state.search_selected_index;
                            let row_w = inner_w;
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(row_w, 30.0), if item.enabled { Sense::click() } else { Sense::hover() });
                            let bg = if !item.enabled {
                                Color32::TRANSPARENT
                            } else if is_selected {
                                tokens.active_bg
                            } else if resp.hovered() {
                                tokens.hover_bg
                            } else {
                                tokens.surface_bg
                            };
                            let border = if is_selected { Stroke::new(1.0, tokens.accent) } else { Stroke::NONE };
                            ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
                            if border != Stroke::NONE {
                                ui.painter().rect_stroke(rect, CornerRadius::same(4), border, StrokeKind::Inside);
                            }
                            let ellipsized = paint_unified_search_row_content(ui, rect, item, tokens);
                            let command_id_opt = match &item.action {
                                UnifiedSearchAction::RunCommand { command_id, .. } => Some(command_id.as_str()),
                                _ => None,
                            };
                            let wid = format!("toolbar.search_item.{idx}");
                            let disabled_reason = (!item.enabled).then(|| {
                                command_id_opt
                                    .map(|cid| command_disabled_reason_for_app(input.app, cid))
                                    .unwrap_or_else(|| tr(&input.app.locale, "tooltip.disabled.select_command_sheet"))
                            });
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
                                    disabled_reason,
                                    z_layer: ZLayer::DropdownMenu,
                                    ellipsized,
                                    full_label: ellipsized.then_some(item.title.as_str()),
                                },
                                ActionRank::Secondary,
                            );
                            if resp.clicked() && item.enabled {
                                commit_unified_search_item(input.app, item, intents);
                                state.search_dropdown_open = false;
                                state.search_query.clear();
                            }
                        }
                    }

                    ui.separator();
                    let footer_resp = link_button(
                        ui,
                        tokens,
                        meta,
                        LinkButtonSpec {
                            font_size: tokens.metrics.font_ui_small_pt,
                            command_id: Some("ui.palette.open"),
                            min_width: Some(inner_w),
                            center_text: true,
                            pad_x: 6.0,
                            ..LinkButtonSpec::new("toolbar.search_open_palette", &footer_label, ZLayer::DropdownMenu)
                        },
                    );
                    if footer_resp.clicked() {
                        let carry = state.search_query.clone();
                        state.search_dropdown_open = false;
                        intents.push(UiIntent::OpenPaletteWithQuery { query: carry });
                    }
                });
        });

    let popup_rect = area_resp.response.rect;
    area_resp.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Menu, true, "Search Suggestions"));
    meta.record(FrameInspectNodeMeta {
        egui_id: area_resp.response.id,
        accesskit_id: area_resp.response.id.accesskit_id(),
        widget_id: String::from("toolbar.search_dropdown"),
        role: String::from("menu"),
        label: String::from("Search Suggestions"),
        command_id: None,
        value: Some(json!({ "count": items.len(), "anchor_rect": [anchor.min.x, anchor.min.y, anchor.max.x, anchor.max.y] })),
        enabled: true,
        disabled_reason: None,
        unclipped_rect: popup_rect,
        clipped_rect: meta.viewport_rect.intersect(popup_rect),
        z_layer: ZLayer::DropdownMenu,
        ellipsized: false,
        full_label: None,
        action_rank: None,
    });

    let clicked_outside = ctx.input(|i| {
        i.pointer.any_click()
            && i.pointer.latest_pos().or_else(|| i.pointer.interact_pos()).is_some_and(|p| !popup_rect.contains(p) && !anchor.contains(p))
    });
    if clicked_outside {
        state.search_dropdown_open = false;
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

    let single_sel = !input.app.split.enabled;
    let dg_enabled = first_ds.is_some() && first_gs.is_some();
    let dr_enabled = first_ds.is_some() && first_rs.is_some();
    let two_enabled = two_up_pair.is_some();
    let two_sel = input.app.split.enabled;
    let swap_enabled = input.app.split.enabled && cur_primary.is_some() && cur_secondary.is_some();

    let entries = [
        ActionMenuEntry::Item(
            ActionMenuItem::button_item("toolbar.split.preset.single", "Single Pane (Off)").selected(single_sel).value(Some(json!(single_sel))),
        ),
        ActionMenuEntry::Item(
            ActionMenuItem::button_item("toolbar.split.preset.data_graph", "Data + Graph")
                .enabled(dg_enabled)
                .disabled_reason((!dg_enabled).then_some("Requires a DataSheet and a GraphSheet"))
                .value(Some(json!(false))),
        ),
        ActionMenuEntry::Item(
            ActionMenuItem::button_item("toolbar.split.preset.data_results", "Data + Results")
                .enabled(dr_enabled)
                .disabled_reason((!dr_enabled).then_some("Requires a DataSheet and a ResultSheet"))
                .value(Some(json!(false))),
        ),
        ActionMenuEntry::Item(
            ActionMenuItem::button_item("toolbar.split.preset.two_up", "2-Up (50 / 50)")
                .selected(two_sel)
                .enabled(two_enabled)
                .disabled_reason((!two_enabled).then_some("Open at least two sheets for 2-Up view"))
                .value(Some(json!(two_sel))),
        ),
        ActionMenuEntry::Item(
            ActionMenuItem::button_item("toolbar.split.preset.swap", "Swap Panes")
                .enabled(swap_enabled)
                .disabled_reason((!swap_enabled).then_some("Enable split view with two sheets to swap"))
                .value(Some(json!(false))),
        ),
    ];

    let out = action_menu_popup(
        ctx,
        tokens,
        Some(meta),
        ActionMenuSpec::dropdown(
            Id::unique("egui_shell.split.preset_popup"),
            "popup.toolbar.split",
            "Split View Presets",
            ActionMenuAnchor::Trigger(anchor),
        ),
        &entries,
    );

    match out.clicked_entry {
        Some(0) => {
            intents.push_ui("ui.split.toggle", json!({ "enabled": false }));
            state.split_menu_open = false;
        }
        Some(1) => {
            if let (Some(ds), Some(gs)) = (first_ds.as_ref(), first_gs.as_ref()) {
                intents.push_ui(
                    "ui.split.toggle",
                    json!({ "enabled": true, "primary_sheet_id": ds, "secondary_sheet_id": gs, "ratio": 0.4, "auto_pin_graph": true }),
                );
                state.split_menu_open = false;
            }
        }
        Some(2) => {
            if let (Some(ds), Some(rs)) = (first_ds.as_ref(), first_rs.as_ref()) {
                intents.push_ui(
                    "ui.split.toggle",
                    json!({ "enabled": true, "primary_sheet_id": ds, "secondary_sheet_id": rs, "ratio": 0.5, "auto_pin_graph": false }),
                );
                state.split_menu_open = false;
            }
        }
        Some(3) => {
            if let Some((p, s)) = two_up_pair.as_ref() {
                intents.push_ui(
                    "ui.split.toggle",
                    json!({ "enabled": true, "primary_sheet_id": p, "secondary_sheet_id": s, "ratio": 0.5, "auto_pin_graph": false }),
                );
                state.split_menu_open = false;
            }
        }
        Some(4) => {
            if let (Some(p), Some(s)) = (cur_primary.as_ref(), cur_secondary.as_ref()) {
                intents.push_ui("ui.split.toggle", json!({ "enabled": true, "primary_sheet_id": s.as_str(), "secondary_sheet_id": p.as_str() }));
                state.split_menu_open = false;
            }
        }
        _ if !input.enabled || out.dismissed => {
            state.split_menu_open = false;
        }
        _ => {}
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
    let wids: Vec<String> = (0..items.len()).map(|idx| format!("toolbar.overflow.item.{idx}")).collect();
    let shortcuts: Vec<String> = items
        .iter()
        .map(|act| {
            command_tooltip(input.app, &act.command_id)
                .filter(|t| !t.shortcut.is_empty())
                .map(|t| format_shortcut_for_platform(&t.shortcut))
                .unwrap_or_default()
        })
        .collect();
    let entries: Vec<ActionMenuEntry<'_>> = items
        .iter()
        .zip(wids.iter().zip(shortcuts.iter()))
        .map(|(act, (wid, sc))| {
            ActionMenuEntry::Item(
                ActionMenuItem::button_item(wid, &act.label)
                    .command_id(Some(&act.command_id))
                    .shortcut((!sc.is_empty()).then_some(sc.as_str()))
                    .tooltip((!act.tooltip.is_empty()).then_some(act.tooltip.as_str()))
                    .enabled(act.enabled)
                    .disabled_reason(act.disabled_reason.as_deref())
                    .value(Some(json!(false))),
            )
        })
        .collect();

    let out = action_menu_popup(
        ctx,
        input.tokens,
        Some(meta),
        ActionMenuSpec::dropdown(
            Id::unique("egui_shell.tier2.overflow_popup"),
            "popup.toolbar.overflow",
            tr(&input.app.locale, "toolbar.overflow"),
            ActionMenuAnchor::Trigger(anchor),
        ),
        &entries,
    );

    if let Some(hit) = out.clicked_entry
        && let Some(act) = items.get(hit)
        && act.enabled
    {
        emit_tier2_action_intent(act, intents);
        state.overflow_menu_open = false;
    } else if !input.enabled || out.dismissed {
        state.overflow_menu_open = false;
    }
}

fn render_unified_toolbar(
    ui: &mut egui::Ui,
    input: Tier1BarInput<'_>,
    state: &mut TopChromeEguiState,
    anchors: &mut Tier1Anchors,
    overflow: &mut Tier2OverflowState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let Tier1BarInput { app, ctx_in, enabled } = input;
    let tokens = ctx_in.tokens;
    let gap = tokens.metrics.space_xs;
    let icon_pt = tokens.metrics.min_toolbar_target_pt;
    let actions = collect_tier2_actions(app, enabled);

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let home_resp = render_brand_home_button(ui, app, tokens, ctx_in.launcher_open, enabled, meta);
        if home_resp.clicked() && enabled {
            state.close_all_popups();
            intents.push(UiIntent::SetLauncherOpen { open: !ctx_in.launcher_open });
        }
        paint_toolbar_divider(ui, tokens);

        let total_room_pt = ui.available_width().max(40.0);
        let (split_galley, _) = layout_single_line_job(ui.painter(), "Split", 68.0, tokens.text_strong, 12.0);
        let split_w = (split_galley.size().x + 14.0).clamp(34.0, 68.0);
        let field_w = 176.0_f32;
        let group_divider_w_pt = tokens.metrics.space_xs * 2.0 + tokens.metrics.stroke_thin_pt;

        let mut slots = Vec::with_capacity(actions.len());
        for act in &actions {
            let desired_width_pt = if act.widget_id == "toolbar.undo" || act.widget_id == "toolbar.redo" {
                icon_pt
            } else {
                let (galley, _) = layout_single_line_job(ui.painter(), &act.label, 152.0, tokens.text_primary, tokens.metrics.font_ui_body_pt);
                (galley.size().x + tokens.metrics.space_xl).clamp(icon_pt, 168.0)
            };
            slots.push(RankedToolbarSlot { rank: act.rank, desired_width_pt });
        }

        let overflow_btn_w = 34.0;
        let fit = fit_unified_toolbar(&UnifiedToolbarFitInput {
            total_room_pt,
            gap_pt: gap,
            overflow_btn_w_pt: overflow_btn_w,
            min_target_w_pt: icon_pt,
            max_visible: 8,
            right_field_group_w_pt: field_w + gap + split_w,
            right_icon_group_w_pt: icon_pt + gap + split_w,
            right_icon_only_w_pt: icon_pt,
            flex_gap_pt: tokens.metrics.space_md,
            group_divider_w_pt,
            contextual_start_idx: 3,
            slots: &slots,
        });

        let has_visible_anchor = fit.ranked.visible_indices.iter().any(|&idx| idx < 3);
        let mut painted_contextual_divider = false;
        for &idx in &fit.ranked.visible_indices {
            if !painted_contextual_divider && idx >= 3 && has_visible_anchor {
                paint_toolbar_divider(ui, tokens);
                painted_contextual_divider = true;
            }
            let act = &actions[idx];
            let resp = if act.widget_id == "toolbar.undo" || act.widget_id == "toolbar.redo" {
                render_undo_redo_icon_button(ui, tokens, meta, act, act.widget_id == "toolbar.redo")
            } else {
                chrome_button_fitted_with_rank(
                    ui,
                    tokens,
                    meta,
                    ChromeButtonSpec {
                        widget_id: &act.widget_id,
                        label: &act.label,
                        command_id: Some(&act.command_id),
                        shortcut: (!act.shortcut.is_empty()).then_some(act.shortcut.as_str()),
                        selected: act.primary,
                        enabled: act.enabled,
                        disabled_reason: act.disabled_reason.as_deref(),
                        tooltip_title: None,
                        tooltip: (!act.tooltip.is_empty()).then_some(act.tooltip.as_str()),
                        max_w: 168.0,
                    },
                    Some(act.rank),
                )
            };
            if resp.clicked() && act.enabled {
                state.close_all_popups();
                emit_tier2_action_intent(act, intents);
            }
        }

        if !fit.ranked.overflow_indices.is_empty() {
            overflow.items = fit.ranked.overflow_indices.iter().map(|&i| actions[i].clone()).collect();
            let ov_resp = chrome_button_fitted_with_rank(
                ui,
                tokens,
                meta,
                ChromeButtonSpec {
                    widget_id: "toolbar.overflow",
                    label: "⋯",
                    command_id: None,
                    shortcut: None,
                    selected: state.overflow_menu_open,
                    enabled,
                    disabled_reason: modal_reason(enabled),
                    tooltip_title: Some(tr(&app.locale, "toolbar.overflow")),
                    tooltip: Some(tr(&app.locale, "tooltip.overflow.summary")),
                    max_w: overflow_btn_w,
                },
                Some(ActionRank::Secondary),
            );
            overflow.anchor = Some(ov_resp.rect);
            if ov_resp.clicked() && enabled {
                let next = !state.overflow_menu_open;
                state.close_all_popups();
                state.overflow_menu_open = next;
            }
        }

        let search_w = match fit.search {
            SearchVisibility::Field => field_w,
            SearchVisibility::Icon => icon_pt,
            SearchVisibility::Hidden => 0.0,
        };
        let right_w = match (search_w > 0.0, fit.split_toggle) {
            (true, true) => search_w + gap + split_w,
            (true, false) => search_w,
            (false, true) => split_w,
            (false, false) => 0.0,
        };
        let filler = (ui.available_width() - right_w).max(0.0);
        if filler > 0.0 {
            ui.add_space(filler);
        }
        render_unified_right_group(
            ui,
            UnifiedRightGroupSpec { input, search_vis: fit.search, split_toggle: fit.split_toggle, split_w },
            state,
            anchors,
            meta,
            intents,
        );
    });
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
        Tier2ActionInput {
            widget_id: "toolbar.new",
            label: tr(&app.locale, "toolbar.new"),
            command_id: "sheet.createData",
            params: json!({ "kind": "column", "cols": 3, "rows": 8 }),
            enabled: modal_enabled,
            disabled_reason: None,
            primary: matches!(context, Tier2SheetContext::EmptyWorkspace),
        },
        true,
    ));

    let can_undo = app.session().undo_depth() > 0;
    let undo_ok = modal_enabled && can_undo;
    let undo_disabled_reason = (!undo_ok).then_some(if modal_enabled { "Nothing to undo" } else { "modal dialog is active" });
    let undo_action = make_engine_action(
        app,
        Tier2ActionInput {
            widget_id: "toolbar.undo",
            label: tr(&app.locale, "toolbar.undo"),
            command_id: "edit.undo",
            params: json!({}),
            enabled: undo_ok,
            disabled_reason: undo_disabled_reason,
            primary: false,
        },
        can_undo,
    );
    actions.push(undo_action);

    let can_redo = app.session().redo_depth() > 0;
    let redo_ok = modal_enabled && can_redo;
    let redo_disabled_reason = (!redo_ok).then_some(if modal_enabled { "Nothing to redo" } else { "modal dialog is active" });
    let redo_action = make_engine_action(
        app,
        Tier2ActionInput {
            widget_id: "toolbar.redo",
            label: tr(&app.locale, "toolbar.redo"),
            command_id: "edit.redo",
            params: json!({}),
            enabled: redo_ok,
            disabled_reason: redo_disabled_reason,
            primary: false,
        },
        can_redo,
    );
    actions.push(redo_action);

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
    let edit_reason: Option<String> = if !modal_enabled {
        Some(String::from("modal dialog is active"))
    } else if !on_result_sheet {
        Some(String::from("Open a ResultSheet to edit analysis parameters"))
    } else {
        None
    };

    match context {
        Tier2SheetContext::EmptyWorkspace => {
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.open",
                    label: tr(&app.locale, "toolbar.open"),
                    command_id: "project.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.empty.import",
                    label: tr(&app.locale, "menu.file.import"),
                    command_id: "ui.import.reshape.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            let mut chain_analyze = make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.analyze",
                    label: tr(&app.locale, "menu.analyze.analyze"),
                    command_id: "ui.analyze.open",
                    params: json!({}),
                    enabled: analyze_ready,
                    disabled_reason: analyze_reason,
                    primary: false,
                },
            );
            chain_analyze.rank = ActionRank::Overflow;
            actions.push(chain_analyze);
        }
        Tier2SheetContext::DataStructured => {
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.analyze",
                    label: tr(&app.locale, "menu.analyze.analyze"),
                    command_id: "ui.analyze.open",
                    params: json!({}),
                    enabled: analyze_ready,
                    disabled_reason: analyze_reason,
                    primary: true,
                },
            ));
            if let Some(ds_id) = active_ds_id {
                actions.push(make_engine_action(
                    app,
                    Tier2ActionInput {
                        widget_id: "toolbar.new_graph",
                        label: tr(&app.locale, "menu.insert.new_graph"),
                        command_id: "graph.create",
                        params: json!({ "data_sheet_id": ds_id }),
                        enabled: modal_enabled,
                        disabled_reason: None,
                        primary: false,
                    },
                    true,
                ));
            }
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.convert_archetype",
                    label: tr(&app.locale, "cmd.ui.table.convertArchetype.open"),
                    command_id: "ui.table.convertArchetype.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            if on_result_sheet {
                actions.push(make_ui_action(
                    app,
                    Tier2ActionInput {
                        widget_id: "toolbar.edit_params",
                        label: tr(&app.locale, "toolbar.parameters"),
                        command_id: "ui.editParams.open",
                        params: json!({}),
                        enabled: modal_enabled && on_result_sheet,
                        disabled_reason: edit_reason.as_deref(),
                        primary: false,
                    },
                ));
            }
            if let Some(note_act) = make_add_floating_note_action(app, modal_enabled) {
                actions.push(note_act);
            }
        }
        Tier2SheetContext::DataMultipleVariables => {
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.analyze",
                    label: tr(&app.locale, "menu.analyze.analyze"),
                    command_id: "ui.analyze.open",
                    params: json!({}),
                    enabled: analyze_ready,
                    disabled_reason: analyze_reason,
                    primary: true,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.formula_bar",
                    label: tr(&app.locale, "cmd.ui.table.openFormulaBar"),
                    command_id: "ui.table.openFormulaBar",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            if let Some(note_act) = make_add_floating_note_action(app, modal_enabled) {
                actions.push(note_act);
            }
        }
        Tier2SheetContext::ResultSheet => {
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.edit_params",
                    label: tr(&app.locale, "toolbar.parameters"),
                    command_id: "ui.editParams.open",
                    params: json!({}),
                    enabled: modal_enabled && on_result_sheet,
                    disabled_reason: edit_reason.as_deref(),
                    primary: true,
                },
            ));
            let rs_id = resolve_result_sheet_for_shortcut(app, true);
            let mut change_data = make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.change_data",
                    label: tr(&app.locale, "toolbar.data_analyzed"),
                    command_id: "ui.analyze.open",
                    params: json!({}),
                    enabled: modal_enabled && rs_id.is_some(),
                    disabled_reason: edit_reason.as_deref(),
                    primary: false,
                },
            );
            change_data.retarget_sheet_id = rs_id.clone();
            actions.push(change_data);
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.methods_report",
                    label: tr(&app.locale, "cmd.ui.result.copyMethodsReport"),
                    command_id: "ui.result.copyMethodsReport",
                    params: json!({}),
                    enabled: modal_enabled && on_result_sheet,
                    disabled_reason: edit_reason.as_deref(),
                    primary: false,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.diagnostics",
                    label: tr(&app.locale, "cmd.ui.result.inspectDiagnostics"),
                    command_id: "ui.result.inspectDiagnostics",
                    params: json!({}),
                    enabled: modal_enabled && on_result_sheet,
                    disabled_reason: edit_reason.as_deref(),
                    primary: false,
                },
            ));
            let is_clustering = rs_id
                .as_ref()
                .and_then(|id| doc.result_sheet(id))
                .is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. })));
            if is_clustering {
                actions.push(make_ui_action(
                    app,
                    Tier2ActionInput {
                        widget_id: "toolbar.append_cluster",
                        label: tr(&app.locale, "cmd.ui.result.appendClusterColumn"),
                        command_id: "ui.result.appendClusterColumn",
                        params: json!({}),
                        enabled: modal_enabled,
                        disabled_reason: None,
                        primary: false,
                    },
                ));
            }
            let mut chain_analyze = make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.analyze",
                    label: tr(&app.locale, "menu.analyze.analyze"),
                    command_id: "ui.analyze.open",
                    params: json!({}),
                    enabled: analyze_ready,
                    disabled_reason: analyze_reason,
                    primary: false,
                },
            );
            chain_analyze.rank = ActionRank::Overflow;
            actions.push(chain_analyze);
            if let Some(note_act) = make_add_floating_note_action(app, modal_enabled) {
                actions.push(note_act);
            }
        }
        Tier2SheetContext::GraphSheet => {
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.graph_type",
                    label: tr(&app.locale, "toolbar.graph_type"),
                    command_id: "ui.changeGraphType.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: true,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.axes",
                    label: tr(&app.locale, "toolbar.axes"),
                    command_id: "ui.graph.axes.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.format_graph",
                    label: tr(&app.locale, "menu.graph.format_graph"),
                    command_id: "ui.graph.format.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.export_figure",
                    label: tr(&app.locale, "cmd.ui.export.figure.open"),
                    command_id: "ui.export.figure.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            let mut chain_analyze = make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.analyze",
                    label: tr(&app.locale, "menu.analyze.analyze"),
                    command_id: "ui.analyze.open",
                    params: json!({}),
                    enabled: analyze_ready,
                    disabled_reason: analyze_reason,
                    primary: false,
                },
            );
            chain_analyze.rank = ActionRank::Overflow;
            actions.push(chain_analyze);
            if let Some(note_act) = make_add_floating_note_action(app, modal_enabled) {
                actions.push(note_act);
            }
        }
        Tier2SheetContext::LayoutSheet => {
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.layout_composer",
                    label: tr(&app.locale, "cmd.ui.layout.composer.open"),
                    command_id: "ui.layout.composer.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: true,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.rulers",
                    label: tr(&app.locale, "toolbar.rulers"),
                    command_id: "ui.rulers.toggle",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.canvas_preset",
                    label: "Page Preset",
                    command_id: "ui.canvas.applyPreset",
                    params: json!({ "preset": "ColumnAndHalf130mm" }),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            actions.push(make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.export_figure",
                    label: tr(&app.locale, "cmd.ui.export.figure.open"),
                    command_id: "ui.export.figure.open",
                    params: json!({}),
                    enabled: modal_enabled,
                    disabled_reason: None,
                    primary: false,
                },
            ));
            if let Some(note_act) = make_add_floating_note_action(app, modal_enabled) {
                actions.push(note_act);
            }
        }
        Tier2SheetContext::InfoSheet => {
            let info_opt = doc.active_sheet_id.as_ref().and_then(|id| doc.info_sheet(id));
            let info_id = info_opt.map(|s| s.id.as_str()).unwrap_or_default();
            let const_name = info_opt.map(next_unused_constant_name).unwrap_or_else(|| String::from("Constant 1"));
            actions.push(make_engine_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.info_add_constant",
                    label: "+ Constant",
                    command_id: "info.setConstant",
                    params: json!({ "sheet_id": info_id, "name": const_name, "value": Value::Null }),
                    enabled: modal_enabled && info_opt.is_some(),
                    disabled_reason: None,
                    primary: true,
                },
                true,
            ));
            let first_ds = doc.data_sheets.first().map(|ds| ds.id.as_str().to_owned());
            let can_new_graph = first_ds.is_some();
            actions.push(make_engine_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.new_graph",
                    label: tr(&app.locale, "menu.insert.new_graph"),
                    command_id: "graph.create",
                    params: first_ds.as_ref().map_or_else(|| json!({}), |id| json!({ "data_sheet_id": id })),
                    enabled: modal_enabled && can_new_graph,
                    disabled_reason: Some(if modal_enabled { "Create a DataSheet before creating a Graph" } else { "modal dialog is active" }),
                    primary: false,
                },
                can_new_graph,
            ));
            let mut chain_analyze = make_ui_action(
                app,
                Tier2ActionInput {
                    widget_id: "toolbar.analyze",
                    label: tr(&app.locale, "menu.analyze.analyze"),
                    command_id: "ui.analyze.open",
                    params: json!({}),
                    enabled: analyze_ready,
                    disabled_reason: analyze_reason,
                    primary: false,
                },
            );
            chain_analyze.rank = ActionRank::Overflow;
            actions.push(chain_analyze);
            if let Some(note_act) = make_add_floating_note_action(app, modal_enabled) {
                actions.push(note_act);
            }
        }
    }

    actions
}

fn make_add_floating_note_action(app: &UiApp, modal_enabled: bool) -> Option<Tier2ActionSpec> {
    let doc = app.document();
    let active_id = doc.active_sheet_id.as_ref()?;
    let _section = doc.sheet_section(active_id)?;
    let frozen = doc.data_sheet(active_id).is_some_and(|ds| ds.frozen);
    let disabled_reason = if !modal_enabled {
        Some("modal dialog is active")
    } else if frozen {
        Some("Data sheet is frozen (read-only).")
    } else {
        None
    };
    let mut act = make_engine_action(
        app,
        Tier2ActionInput {
            widget_id: "toolbar.add_floating_note",
            label: tr(&app.locale, "toolbar.add_note"),
            command_id: "sheet.addFloatingNote",
            params: json!({ "sheet_id": active_id.as_str() }),
            enabled: modal_enabled,
            disabled_reason,
            primary: false,
        },
        !frozen,
    );
    if act.tooltip.is_empty() {
        act.tooltip = String::from("Add a floating note to the active sheet.");
    }
    act.rank = ActionRank::Overflow;
    Some(act)
}

fn can_analyze_active(app: &UiApp) -> bool {
    let doc = app.document();
    doc.active_sheet_id.as_ref().is_some_and(|id| {
        doc.data_sheet(id).is_some()
            || doc.result_sheet(id).is_some_and(|rs| rs.analyzable_data.is_some() || rs.tabs.iter().any(|t| !t.rows.is_empty()))
    })
}

fn resolve_command_shortcut_for_platform(app: &UiApp, command_id: &str, tip: Option<&CommandTooltip>) -> String {
    tip.map(|t| t.shortcut.as_str())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| app.shortcuts.active_combo(command_id))
        .filter(|s| !s.trim().is_empty())
        .map(format_shortcut_for_platform)
        .unwrap_or_default()
}

fn format_command_tooltip(tip: Option<&CommandTooltip>) -> String {
    // R20.21 Locked Decision #13: render CommandTooltip.summary only in the GUI; leave .beginner for MCP/docs.
    match tip {
        Some(t) if !t.summary.trim().is_empty() => t.summary.to_owned(),
        _ => String::new(),
    }
}

fn slot_action_rank(command_id: &str, primary: bool) -> ActionRank {
    if primary { ActionRank::Primary } else { default_command_action_rank(command_id).max(ActionRank::Secondary) }
}

fn make_engine_action(app: &UiApp, input: Tier2ActionInput<'_>, base_enabled: bool) -> Tier2ActionSpec {
    let Tier2ActionInput { widget_id, label, command_id, params, enabled, disabled_reason, primary } = input;
    let eff_enabled = enabled && base_enabled;
    let rank = slot_action_rank(command_id, primary);
    let tip = command_tooltip(app, command_id);
    let shortcut = resolve_command_shortcut_for_platform(app, command_id, tip.as_ref());
    Tier2ActionSpec {
        widget_id: widget_id.to_owned(),
        label: label.to_owned(),
        shortcut,
        tooltip: format_command_tooltip(tip.as_ref()),
        command_id: command_id.to_owned(),
        params,
        is_engine: true,
        enabled: eff_enabled,
        disabled_reason: (!eff_enabled).then(|| disabled_reason.unwrap_or("modal dialog is active").to_owned()),
        primary,
        rank,
        retarget_sheet_id: None,
    }
}

fn make_ui_action(app: &UiApp, input: Tier2ActionInput<'_>) -> Tier2ActionSpec {
    let Tier2ActionInput { widget_id, label, command_id, params, enabled, disabled_reason, primary } = input;
    let rank = slot_action_rank(command_id, primary);
    let tip = command_tooltip(app, command_id);
    let shortcut = resolve_command_shortcut_for_platform(app, command_id, tip.as_ref());
    Tier2ActionSpec {
        widget_id: widget_id.to_owned(),
        label: label.to_owned(),
        shortcut,
        tooltip: format_command_tooltip(tip.as_ref()),
        command_id: command_id.to_owned(),
        params,
        is_engine: false,
        enabled,
        disabled_reason: (!enabled).then(|| disabled_reason.unwrap_or("modal dialog is active").to_owned()),
        primary,
        rank,
        retarget_sheet_id: None,
    }
}

fn emit_tier2_action_intent(act: &Tier2ActionSpec, intents: &mut PendingUiIntents) {
    if let Some(rs_id) = act.retarget_sheet_id.as_ref() {
        intents.push(UiIntent::OpenRetargetDialog { result_sheet_id: rs_id.clone(), target_data_sheet_id: None });
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
        "ar" => "الرئيسية",
        "he" => "דף הבית",
        _ => "Home",
    }
}

fn paint_toolbar_divider(ui: &mut egui::Ui, tokens: &Tokens) {
    let div_w = tokens.metrics.space_xs * 2.0 + tokens.metrics.stroke_thin_pt;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(div_w, tokens.metrics.min_toolbar_target_pt), Sense::hover());
    let x = rect.center().x;
    let inset = tokens.metrics.space_xs;
    ui.painter().line_segment(
        [Pos2::new(x, rect.top() + inset), Pos2::new(x, rect.bottom() - inset)],
        Stroke::new(tokens.metrics.stroke_thin_pt, tokens.divider),
    );
}

fn render_brand_home_button(
    ui: &mut egui::Ui,
    app: &UiApp,
    tokens: &Tokens,
    launcher_open: bool,
    enabled: bool,
    meta: &mut FrameInspectMeta,
) -> egui::Response {
    let home_label = localized_home_label(&app.locale);
    let mark_size = crate::egui_shell::brand::TOOLBAR_MARK_SIZE;
    let btn_size = Vec2::new(mark_size.x + tokens.metrics.space_md, tokens.metrics.min_toolbar_target_pt);
    let (rect, mut resp) = ui.allocate_exact_size(btn_size, if enabled { Sense::click() } else { Sense::hover() });
    resp = resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });

    let radius = CornerRadius::same(tokens.metrics.radius_md);
    super::widgets::paint_interactive_chrome(
        ui,
        rect,
        tokens,
        super::widgets::InteractiveChromeSpec {
            radius,
            base_bg: Color32::TRANSPARENT,
            base_border: None,
            hovered: resp.hovered(),
            selected: launcher_open,
            pressed: resp.is_pointer_button_down_on(),
            enabled,
        },
    );
    if enabled && resp.has_focus() {
        ui.painter().rect_stroke(rect, radius, Stroke::new(tokens.metrics.stroke_focus_pt, tokens.focus_ring), StrokeKind::Inside);
    }

    let mark_rect = Rect::from_center_size(rect.center(), mark_size);
    crate::egui_shell::brand::paint_mark(ui, mark_rect);

    let ak_label = home_label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Button, enabled, launcher_open, ak_label.clone()));
    resp = show_tooltip(
        &resp,
        tokens,
        TooltipSpec {
            title: home_label,
            shortcut: None,
            summary: Some(tr(&app.locale, "tooltip.home.summary")),
            disabled_reason: modal_reason(enabled),
        },
    );

    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: "toolbar.home",
                role: "button",
                label: home_label,
                command_id: None,
                value: Some(json!(launcher_open)),
                enabled,
                disabled_reason: modal_reason(enabled),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
    }
    resp
}

fn render_undo_redo_icon_button(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    act: &Tier2ActionSpec,
    is_redo: bool,
) -> egui::Response {
    let (rect, mut resp) =
        ui.allocate_exact_size(Vec2::splat(tokens.metrics.min_toolbar_target_pt), if act.enabled { Sense::click() } else { Sense::hover() });
    if !act.enabled {
        resp = resp.on_hover_cursor(egui::CursorIcon::Default);
    }
    let radius = CornerRadius::same(tokens.metrics.radius_md);
    super::widgets::paint_interactive_chrome(
        ui,
        rect,
        tokens,
        super::widgets::InteractiveChromeSpec {
            radius,
            base_bg: Color32::TRANSPARENT,
            base_border: None,
            hovered: resp.hovered(),
            selected: false,
            pressed: resp.is_pointer_button_down_on(),
            enabled: act.enabled,
        },
    );
    if act.enabled && resp.has_focus() {
        ui.painter().rect_stroke(rect, radius, Stroke::new(tokens.metrics.stroke_focus_pt, tokens.focus_ring), StrokeKind::Inside);
    }

    let fg = if act.enabled { tokens.text_strong } else { tokens.text_disabled };
    paint_curved_history_arrow(ui.painter(), rect.center(), is_redo, fg);

    let ak_label = act.label.clone();
    let ak_enabled = act.enabled;
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, ak_enabled, ak_label.clone()));
    resp = show_tooltip(
        &resp,
        tokens,
        TooltipSpec {
            title: &act.label,
            shortcut: (!act.shortcut.is_empty()).then_some(act.shortcut.as_str()),
            summary: (!act.tooltip.is_empty()).then_some(act.tooltip.as_str()),
            disabled_reason: act.disabled_reason.as_deref(),
        },
    );

    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_ranked_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: &act.widget_id,
                role: "button",
                label: &act.label,
                command_id: Some(&act.command_id),
                value: None,
                enabled: act.enabled,
                disabled_reason: act.disabled_reason.as_deref(),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
            act.rank,
        );
    }
    resp
}

fn paint_curved_history_arrow(painter: &egui::Painter, c: Pos2, is_redo: bool, color: Color32) {
    let sx = if is_redo { -1.0_f32 } else { 1.0_f32 };
    let pt = |dx: f32, dy: f32| Pos2::new(c.x + dx * sx, c.y + dy);
    let stroke = Stroke::new(1.5, color);
    // Smooth open curved arc from lower tail around the side and top toward the tip.
    let arc = [pt(-5.0, -1.5), pt(-2.5, -3.8), pt(0.8, -4.3), pt(3.8, -2.8), pt(5.1, 0.0), pt(4.1, 3.0), pt(1.4, 4.4), pt(-1.0, 4.4)];
    painter.add(egui::Shape::line(arc.to_vec(), stroke));
    let tip = pt(-5.2, -1.4);
    painter.line_segment([pt(-4.7, -5.0), tip], stroke);
    painter.line_segment([tip, pt(-1.4, -1.0)], stroke);
}

const fn modal_reason(enabled: bool) -> Option<&'static str> {
    if enabled { None } else { Some("modal dialog is active") }
}

fn layout_single_line_job(painter: &egui::Painter, text: &str, max_width: f32, color: Color32, font_size: f32) -> (Arc<egui::Galley>, bool) {
    super::ui_kit::elided_galley(painter, text, FontId::proportional(font_size), color, max_width, true)
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
    let resp = if ellipsized { show_tooltip(&resp, tokens, TooltipSpec::new(title)) } else { resp };
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

fn chrome_button_fitted(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: ChromeButtonSpec<'_>) -> egui::Response {
    chrome_button_fitted_with_rank(ui, tokens, meta, spec, None)
}

fn chrome_button_fitted_with_rank(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    spec: ChromeButtonSpec<'_>,
    rank: Option<ActionRank>,
) -> egui::Response {
    let ChromeButtonSpec { widget_id, label, command_id, shortcut, selected, enabled, disabled_reason, tooltip_title, tooltip, max_w } = spec;
    let is_primary_cta = rank == Some(ActionRank::Primary) && enabled;
    let text_max = (max_w - 14.0).max(12.0);
    let fg = if is_primary_cta {
        tokens.on_accent
    } else if enabled {
        tokens.text_strong
    } else {
        tokens.text_disabled
    };
    let (galley, ellipsized) = layout_single_line_job(ui.painter(), label, text_max, fg, 12.0);
    let min_w = max_w.min(34.0);
    let w = (galley.size().x + 14.0).clamp(min_w, max_w);
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(w, tokens.metrics.min_toolbar_target_pt), if enabled { Sense::click() } else { Sense::hover() });
    let radius = CornerRadius::same(tokens.metrics.radius_md);
    if is_primary_cta {
        let fill = if resp.is_pointer_button_down_on() || resp.hovered() { tokens.accent_hover } else { tokens.accent_strong };
        let stroke = if resp.has_focus() {
            Stroke::new(tokens.metrics.stroke_focus_pt, tokens.focus_ring)
        } else {
            Stroke::new(tokens.metrics.stroke_thin_pt, tokens.accent_hover)
        };
        ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
    } else {
        super::widgets::paint_interactive_chrome(
            ui,
            rect,
            tokens,
            super::widgets::InteractiveChromeSpec {
                radius,
                base_bg: Color32::TRANSPARENT,
                base_border: None,
                hovered: resp.hovered(),
                selected,
                pressed: resp.is_pointer_button_down_on(),
                enabled,
            },
        );
        if enabled && resp.has_focus() {
            ui.painter().rect_stroke(rect, radius, Stroke::new(tokens.metrics.stroke_focus_pt, tokens.focus_ring), StrokeKind::Inside);
        }
    }
    ui.painter().with_clip_rect(rect.shrink(3.0)).galley(
        Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    let ak_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, ak_label.clone()));
    let tip_title = tooltip_title.unwrap_or(label);
    let summary = tooltip.map(str::trim).filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case(tip_title));
    let tip_spec = TooltipSpec { title: tip_title, shortcut, summary, disabled_reason };
    let resp = show_tooltip_for_control(&resp, tokens, label, ellipsized, tip_spec);

    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        let input = FrameInspectEntryInput {
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
        };
        if let Some(r) = rank {
            meta.record_ranked_response(ui, &resp, input, r);
        } else {
            meta.record_response(ui, &resp, input);
        }
    }
    resp
}

struct CommandPaletteIconSpec<'a> {
    label: &'a str,
    shortcut: &'a str,
    summary: &'a str,
    selected: bool,
    enabled: bool,
}

fn render_command_palette_icon_button(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    spec: CommandPaletteIconSpec<'_>,
) -> egui::Response {
    let CommandPaletteIconSpec { label, shortcut, summary, selected, enabled } = spec;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::splat(tokens.metrics.min_toolbar_target_pt), if enabled { Sense::click() } else { Sense::hover() });
    let radius = CornerRadius::same(tokens.metrics.radius_md);
    super::widgets::paint_interactive_chrome(
        ui,
        rect,
        tokens,
        super::widgets::InteractiveChromeSpec {
            radius,
            base_bg: Color32::TRANSPARENT,
            base_border: None,
            hovered: resp.hovered(),
            selected,
            pressed: resp.is_pointer_button_down_on(),
            enabled,
        },
    );
    if enabled && resp.has_focus() {
        ui.painter().rect_stroke(rect, radius, Stroke::new(tokens.metrics.stroke_focus_pt, tokens.focus_ring), StrokeKind::Inside);
    }
    let fg = if enabled { tokens.text_strong } else { tokens.text_disabled };

    // Monochrome vector magnifying-glass icon centered inside the button.
    let lens_center = rect.center() + Vec2::new(-1.5, -1.5);
    ui.painter().circle_stroke(lens_center, 4.5, Stroke::new(1.5, fg));
    ui.painter().line_segment([lens_center + Vec2::splat(3.1), lens_center + Vec2::splat(6.5)], Stroke::new(1.5, fg));

    let ak_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, ak_label.clone()));
    if rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: "toolbar.command_palette",
                role: "button",
                label,
                command_id: Some("ui.palette.open"),
                value: Some(json!(selected)),
                enabled,
                disabled_reason: modal_reason(enabled),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
    }
    show_tooltip(
        &resp,
        tokens,
        TooltipSpec {
            title: label,
            shortcut: (!shortcut.is_empty()).then_some(shortcut),
            summary: (!summary.is_empty()).then_some(summary),
            disabled_reason: modal_reason(enabled),
        },
    )
}
