//! Native `egui` Leading Dock: Project Navigator (`~65%`) & Scoped Lineage `Family` (`~35%`)
//! (`docs/planning/gui-layout-spec.md` §4, `R15.3`).

use egui::{Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens};
use crate::layout::ZLayer;
use crate::tr;

/// Quick kind filter chip in the Navigator filter header (`§4.1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NavigatorKindChip {
    #[default]
    All,
    Data,
    Info,
    Results,
    Graphs,
    Layouts,
}

/// Persistent `egui_shell` UI state for the Leading Dock (`Navigator` + `Family`).
#[derive(Debug, Clone, PartialEq)]
pub struct LeadingDockEguiState {
    pub search_draft: String,
    pub kind_chip: NavigatorKindChip,
    pub nav_split_fraction: f32,
    pub renaming_sheet_id: Option<curvebook_doc::SheetId>,
    pub rename_draft: String,
    pub dragging_sheet_id: Option<curvebook_doc::SheetId>,
    pub hover_preview_sheet_id: Option<curvebook_doc::SheetId>,
    /// Monotonic `egui` frame clock (`ctx.input(|i| i.time)`) when hover started (`>= 0.45 s` triggers preview).
    pub hover_started_sec: Option<f64>,
    pub hover_preview_active: bool,
}

impl Default for LeadingDockEguiState {
    fn default() -> Self {
        Self {
            search_draft: String::new(),
            kind_chip: NavigatorKindChip::All,
            nav_split_fraction: 0.65,
            renaming_sheet_id: None,
            rename_draft: String::new(),
            dragging_sheet_id: None,
            hover_preview_sheet_id: None,
            hover_started_sec: None,
            hover_preview_active: false,
        }
    }
}

/// Input context bundle for [`render_leading_dock`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct LeadingDockContext<'a> {
    pub tokens: &'a Tokens,
    pub layout: ResponsiveShellLayout,
    pub enabled: bool,
    pub focused: bool,
}

/// Per-frame geometry report emitted by [`render_leading_dock`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeadingDockFrameReport {
    pub dock_rect_pt: Option<egui::Rect>,
    pub navigator_rect_pt: Option<egui::Rect>,
    pub family_rect_pt: Option<egui::Rect>,
    pub splitter_rect_pt: Option<egui::Rect>,
    pub hover_preview_rect_pt: Option<egui::Rect>,
}

/// Render the collapsible `240 pt -> 36 pt` Leading Dock containing `Navigator` and `Family`.
///
/// The Wave 3 base renders the collapsed rail (`36 pt`) or live Navigator + Family skeleton and
/// handles sheet selection (`sheet.select`); `r15-welcome-nav` adds search chips, 5-Folder vs
/// Nested `Data with Results` trees, `+ New <Kind>...` rows, `F2` rename, drag-reorder, and
/// `>= 450 ms` `Family` hover preview.
pub fn render_leading_dock(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut LeadingDockEguiState,
    ctx_in: LeadingDockContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> LeadingDockFrameReport {
    let mut report = LeadingDockFrameReport::default();
    if !ctx_in.layout.leading_visible || ctx_in.layout.leading_dock_w_pt <= 0.0 {
        return report;
    }

    let tokens = ctx_in.tokens;
    let w = ctx_in.layout.leading_dock_w_pt;
    let dock_frame = egui::Frame::NONE.fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.divider)).inner_margin(egui::Margin::same(6));

    let resp = egui::Panel::left("egui_shell.leading_dock").exact_size(w).resizable(false).frame(dock_frame).show(ui, |ui| {
        if ctx_in.layout.leading_collapsed_to_rail {
            let expand_resp = ui.add_enabled(ctx_in.enabled, egui::Button::new(">").min_size(Vec2::new(24.0, 24.0)));
            meta.record_response(
                ui,
                &expand_resp,
                FrameInspectEntryInput {
                    widget_id: "nav.expand_rail",
                    role: "button",
                    label: "Expand Navigator",
                    command_id: Some("ui.navigator.toggle"),
                    value: None,
                    enabled: ctx_in.enabled,
                    disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if expand_resp.clicked() && ctx_in.enabled {
                intents.push_toggle_leading_collapsed_rail();
            }
            return;
        }

        let avail = ui.available_rect_before_wrap();
        report.dock_rect_pt = Some(avail);
        if ctx_in.focused {
            ui.painter().rect_stroke(avail, egui::CornerRadius::same(2), Stroke::new(2.0, tokens.focus_ring), StrokeKind::Inside);
        }

        let family_h = if ctx_in.layout.family_visible { (avail.height() * (1.0 - state.nav_split_fraction)).clamp(84.0, 240.0) } else { 0.0 };
        let nav_h = (avail.height() - family_h - if family_h > 0.0 { 6.0 } else { 0.0 }).max(48.0);
        let nav_rect = egui::Rect::from_min_size(avail.min, Vec2::new(avail.width(), nav_h));
        report.navigator_rect_pt = Some(nav_rect);

        ui.scope_builder(egui::UiBuilder::new().max_rect(nav_rect), |ui| {
            ui.horizontal(|ui| {
                ui.strong(tr(&app.locale, "nav.navigator"));
                if ui.add_enabled(ctx_in.enabled, egui::Button::new("<")).clicked() && ctx_in.enabled {
                    intents.push_toggle_leading_collapsed_rail();
                }
            });
            let active_id = app.document().active_sheet_id.as_ref();
            egui::ScrollArea::vertical().id_salt("egui_shell.nav.scroll").auto_shrink([false, false]).show(ui, |ui| {
                for entry in &app.nav.entries {
                    let selected = active_id == Some(&entry.sheet_id);
                    let wid = format!("nav.sheet.{}", entry.sheet_id.as_str());
                    let btn = ui.add_enabled(
                        ctx_in.enabled,
                        egui::Button::new(&entry.title).selected(selected).min_size(Vec2::new(ui.available_width(), 24.0)),
                    );
                    meta.record_response(
                        ui,
                        &btn,
                        FrameInspectEntryInput {
                            widget_id: &wid,
                            role: "treeitem",
                            label: &entry.title,
                            command_id: Some("sheet.select"),
                            value: Some(json!(selected)),
                            enabled: ctx_in.enabled,
                            disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                            z_layer: ZLayer::Workspace,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if btn.clicked() && ctx_in.enabled && !selected {
                        intents.push_engine("sheet.select", json!({ "sheet_id": entry.sheet_id.as_str() }));
                    }
                }
            });
        });

        if family_h > 0.0 {
            let fam_rect = egui::Rect::from_min_max(egui::pos2(avail.left(), nav_rect.bottom() + 4.0), avail.max);
            report.family_rect_pt = Some(fam_rect);
            ui.scope_builder(egui::UiBuilder::new().max_rect(fam_rect), |ui| {
                ui.separator();
                ui.strong(tr(&app.locale, "nav.family"));
                for node in &app.nav.family_nodes {
                    ui.label(&node.title);
                }
            });
        }
    });

    if report.dock_rect_pt.is_none() {
        report.dock_rect_pt = Some(resp.response.rect);
    }
    report
}
