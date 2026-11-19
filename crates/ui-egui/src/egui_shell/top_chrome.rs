//! Two-tier `egui` Top Chrome: Tier 1 Application Bar (`32 pt`) + Tier 2 Sheet-Adaptive
//! Contextual Action Strip (`38 pt`, `docs/planning/gui-layout-spec.md` §3, `R15.3`).

use egui::{CornerRadius, FontId, Pos2, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::inspect_bridge::HybridCoexistenceDecision;
use crate::egui_shell::top_bar_fit::{BAR_ICON_PT, RightWidths, SearchVisibility, fit_right_group};
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, WorkspacePreset};
use crate::interaction::resolve_result_sheet_for_shortcut;
use crate::layout::ZLayer;
use crate::tr;

/// Persistent UI state for Tier 1 menus, preset/split popups, and Tier 2 overflow drawer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopChromeEguiState {
    pub split_menu_open: bool,
    pub preset_menu_open: bool,
    pub overflow_menu_open: bool,
    pub active_in_window_menu: Option<String>,
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

/// Render Tier 1 (`32 pt` App Bar) and Tier 2 (`38 pt` Sheet-Adaptive Contextual Action Strip).
///
/// The Wave 3 base renders the compact Tier 1 / Tier 2 chrome preserving canonical Wave 1/2 widget
/// IDs (`toolbar.home`, `toolbar.help.about`, `toolbar.command_palette`, `toolbar.split`,
/// `toolbar.preset`, `toolbar.presentation`, `toolbar.analyze`, `toolbar.edit_params`,
/// `sheet.tab.<id>`); `r15-top-chrome` builds out the full in-window `egui::MenuBar` parity strip,
/// 6-context contextual groups, archetype quick-run pills, and `⋯` overflow menu.
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
        return report;
    }

    let tokens = ctx_in.tokens;
    let modal_blocked = ctx_in.decision.modal_backdrop_active;

    let panel_resp = egui::Panel::top("egui_shell.top_chrome")
        .frame(egui::Frame::NONE.fill(tokens.app_bar).stroke(Stroke::new(1.0, tokens.divider)).inner_margin(egui::Margin::symmetric(8, 3)))
        .show(ui, |ui| {
            let tier1_resp = ui.allocate_ui(Vec2::new(ui.available_width(), ctx_in.layout.tier1_app_bar_h_pt.max(24.0)), |ui| {
                render_tier1_bar(ui, app, state, &ctx_in, !modal_blocked, meta, intents);
            });
            report.tier1_rect_pt = Some(tier1_resp.response.rect);

            ui.add_space(2.0);
            let tier2_resp = ui.allocate_ui(Vec2::new(ui.available_width(), ctx_in.layout.tier2_action_strip_h_pt.max(26.0)), |ui| {
                render_tier2_strip(ui, app, tokens, !modal_blocked, meta, intents);
            });
            report.tier2_rect_pt = Some(tier2_resp.response.rect);
        });

    report.total_height_pt = panel_resp.response.rect.height();
    report.menu_popup_open = state.active_in_window_menu.is_some() || state.split_menu_open || state.preset_menu_open || state.overflow_menu_open;
    report
}

fn render_tier1_bar(
    ui: &mut egui::Ui,
    app: &UiApp,
    _state: &mut TopChromeEguiState,
    ctx_in: &TopChromeContext<'_>,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let about_label = tr(&app.locale, "menu.help.about");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        let home_resp = chrome_button(ui, tokens, meta, "toolbar.home", "Home", None, ctx_in.launcher_open, enabled, modal_reason(enabled));
        if home_resp.clicked() && enabled {
            intents.push_set_launcher_open(!ctx_in.launcher_open);
        }

        let about_resp = chrome_button(ui, tokens, meta, "toolbar.help.about", about_label, Some("app.about"), false, enabled, modal_reason(enabled));
        if about_resp.clicked() && enabled {
            intents.push_ui("app.about", json!({}));
        }

        let title = if ctx_in.dirty { format!("{} •", ctx_in.project_title) } else { ctx_in.project_title.to_owned() };
        ui.colored_label(tokens.text_strong, title);

        let avail = ui.available_width().max(0.0);
        let fitted = fit_right_group(&RightWidths {
            room: avail,
            has_pinned_badge: false,
            switcher: 84.0,
            field: 190.0,
            icon: BAR_ICON_PT,
            gap: 6.0,
            trailing_extra: BAR_ICON_PT,
        });
        let search_w = match fitted.search {
            SearchVisibility::Field => 190.0,
            SearchVisibility::Icon => 54.0,
            SearchVisibility::Hidden => 0.0,
        };
        let extra_w = (if fitted.split_toggle { 58.0 } else { 0.0 })
            + (if fitted.switcher { fitted.switcher_w } else { 0.0 })
            + (if fitted.presentation_toggle { 68.0 } else { 0.0 });
        let used_right = search_w + extra_w + 18.0;
        if ui.available_width() > used_right {
            ui.add_space((ui.available_width() - used_right).max(0.0));
        }

        if !matches!(fitted.search, SearchVisibility::Hidden) {
            let search_lbl = match fitted.search {
                SearchVisibility::Field => "Search commands (Cmd+K)",
                SearchVisibility::Icon | SearchVisibility::Hidden => "Cmd+K",
            };
            let pal_resp = chrome_button(
                ui,
                tokens,
                meta,
                "toolbar.command_palette",
                search_lbl,
                Some("ui.palette.open"),
                false,
                enabled,
                modal_reason(enabled),
            );
            if pal_resp.clicked() && enabled {
                intents.push_set_palette_open(true);
            }
        }

        if fitted.split_toggle {
            let split_resp =
                chrome_button(ui, tokens, meta, "toolbar.split", "Split", Some("ui.split.toggle"), app.split.enabled, enabled, modal_reason(enabled));
            if split_resp.clicked() && enabled {
                intents.push_ui("ui.split.toggle", json!({}));
            }
        }

        if fitted.switcher {
            let preset_resp =
                chrome_button(ui, tokens, meta, "toolbar.preset", ctx_in.workspace_preset.label(), None, false, enabled, modal_reason(enabled));
            if preset_resp.clicked() && enabled {
                let all = WorkspacePreset::ALL;
                let idx = all.iter().position(|p| *p == ctx_in.workspace_preset).unwrap_or(0);
                intents.push_apply_workspace_preset(all[(idx + 1) % all.len()]);
            }
        }

        if fitted.presentation_toggle {
            let pres_resp = chrome_button(
                ui,
                tokens,
                meta,
                "toolbar.presentation",
                "Present",
                Some("ui.presentation.toggle"),
                app.presentation_mode,
                enabled,
                modal_reason(enabled),
            );
            if pres_resp.clicked() && enabled {
                intents.push_ui("ui.presentation.toggle", json!({}));
            }
        }
    });
}

fn render_tier2_strip(ui: &mut egui::Ui, app: &UiApp, tokens: &Tokens, enabled: bool, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let doc = app.document();
    let on_result_sheet = resolve_result_sheet_for_shortcut(app, true).is_some();
    let analyze_enabled = enabled
        && doc.active_sheet_id.as_ref().is_some_and(|id| {
            doc.data_sheet(id).is_some()
                || doc.result_sheet(id).is_some_and(|rs| rs.analyzable_data.is_some() || rs.tabs.iter().any(|t| !t.rows.is_empty()))
        });
    let analyze_reason = if !enabled {
        Some("modal dialog is active")
    } else if !analyze_enabled {
        Some("Select a DataSheet before analyzing")
    } else {
        None
    };

    let edit_enabled = enabled && on_result_sheet;
    let edit_reason = if !enabled {
        Some("modal dialog is active")
    } else if !on_result_sheet {
        Some("Open a ResultSheet to edit analysis parameters (Cmd+T)")
    } else {
        None
    };

    let analyze_label = tr(&app.locale, "menu.analyze.analyze");
    let edit_label = tr(&app.locale, "toolbar.parameters");

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let analyze_resp =
            chrome_button(ui, tokens, meta, "toolbar.analyze", analyze_label, Some("ui.analyze.open"), false, analyze_enabled, analyze_reason);
        if analyze_resp.clicked() && analyze_enabled {
            intents.push_ui("ui.analyze.open", json!({}));
        }

        let edit_resp =
            chrome_button(ui, tokens, meta, "toolbar.edit_params", edit_label, Some("ui.editParams.open"), false, edit_enabled, edit_reason);
        if edit_resp.clicked() && edit_enabled {
            intents.push_ui("ui.editParams.open", json!({}));
        }

        if !doc.data_sheets.is_empty() || !doc.result_sheets.is_empty() {
            ui.separator();
            let active_id = doc.active_sheet_id.as_ref();
            let total_tabs = (doc.data_sheets.len() + doc.result_sheets.len()).max(1) as f32;
            let switcher_w = ui.available_width().max(40.0);
            let gap = 6.0 * (total_tabs - 1.0);
            let max_tab_w = ((switcher_w - gap) / total_tabs).clamp(36.0, 148.0);
            ui.allocate_ui(Vec2::new(switcher_w, 24.0), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for ds in &doc.data_sheets {
                        let is_active = active_id == Some(&ds.id);
                        let wid = format!("sheet.tab.{}", ds.id.as_str());
                        let resp = chrome_tab(ui, tokens, meta, &wid, &ds.title, is_active, enabled, max_tab_w);
                        if resp.clicked() && enabled && !is_active {
                            intents.push_engine("sheet.select", json!({ "sheet_id": ds.id.as_str() }));
                        }
                    }
                    for rs in &doc.result_sheets {
                        let is_active = active_id == Some(&rs.id);
                        let wid = format!("sheet.tab.{}", rs.id.as_str());
                        let resp = chrome_tab(ui, tokens, meta, &wid, rs.title(), is_active, enabled, max_tab_w);
                        if resp.clicked() && enabled && !is_active {
                            intents.push_engine("sheet.select", json!({ "sheet_id": rs.id.as_str() }));
                        }
                    }
                });
            });
        }
    });
}

const fn modal_reason(enabled: bool) -> Option<&'static str> {
    if enabled { None } else { Some("modal dialog is active") }
}

#[allow(clippy::too_many_arguments)]
fn chrome_button(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    command_id: Option<&str>,
    selected: bool,
    enabled: bool,
    disabled_reason: Option<&str>,
) -> egui::Response {
    let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(FontId::proportional(12.0), tokens.text_primary));
    job.wrap = egui::text::TextWrapping { max_width: 168.0, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let ellipsized = galley.elided;
    let w = (galley.size().x + 14.0).clamp(40.0, 176.0);
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
    let text_max_w = (max_w - 12.0).max(16.0);
    let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(FontId::proportional(11.5), tokens.text_primary));
    job.wrap = egui::text::TextWrapping { max_width: text_max_w, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let ellipsized = galley.elided;
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
