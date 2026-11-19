//! Native `egui` 7-context Trailing Studio Dock (`280 pt -> 36 pt`,
//! `docs/planning/gui-layout-spec.md` §6, `R15.3`).

use curvebook_doc::TableArchetype;
use egui::{Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens};
use crate::layout::ZLayer;

/// Seven automatic Trailing Studio surfaces defined in `docs/planning/gui-layout-spec.md` §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StudioContextSurface {
    MultipleVariablesFormula,
    #[default]
    TableSelection,
    MetadataHooks,
    ResultsInterpret,
    GraphInspector,
    DataPointDrillBack,
    PageLayoutComposer,
}

/// Active tab inside [`StudioContextSurface::ResultsInterpret`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ResultsStudioTab {
    #[default]
    Assumptions,
    Guidance,
    Methods,
}

/// Active tab inside [`StudioContextSurface::GraphInspector`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GraphStudioTab {
    #[default]
    Data,
    Axes,
    Legends,
    Annotations,
    Canvas,
}

/// Persistent `egui_shell` UI state for the Trailing Studio Dock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrailingStudioEguiState {
    pub results_tab: ResultsStudioTab,
    pub graph_tab: GraphStudioTab,
    pub mv_formula_draft: String,
    pub selected_subset_rows_summary: String,
    pub last_copied_methods: bool,
}

/// Input context bundle for [`render_trailing_studio`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct TrailingStudioContext<'a> {
    pub tokens: &'a Tokens,
    pub layout: ResponsiveShellLayout,
    pub enabled: bool,
    pub focused: bool,
}

/// Per-frame geometry & active-surface report emitted by [`render_trailing_studio`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrailingStudioFrameReport {
    pub dock_rect_pt: Option<egui::Rect>,
    pub surface: StudioContextSurface,
}

/// Resolve the active [`StudioContextSurface`] strictly from live document sheet & brushing state.
#[must_use]
pub fn resolve_studio_surface(app: &UiApp) -> StudioContextSurface {
    let doc = app.document();
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return StudioContextSurface::TableSelection;
    };
    if doc.layout_sheet(active_id).is_some() {
        return StudioContextSurface::PageLayoutComposer;
    }
    if doc.graph_sheet(active_id).is_some() {
        return if app.brushed_cell.is_some() { StudioContextSurface::DataPointDrillBack } else { StudioContextSurface::GraphInspector };
    }
    if doc.result_sheet(active_id).is_some() {
        return StudioContextSurface::ResultsInterpret;
    }
    if doc.info_sheet(active_id).is_some() {
        return StudioContextSurface::MetadataHooks;
    }
    if let Some(ds) = doc.data_sheet(active_id)
        && matches!(ds.archetype, TableArchetype::MultipleVariables(_))
    {
        return StudioContextSurface::MultipleVariablesFormula;
    }
    StudioContextSurface::TableSelection
}

/// Render the collapsible `280 pt -> 36 pt` Trailing Studio Dock for the active surface.
///
/// The Wave 3 base implements deterministic [`resolve_studio_surface`] routing and collapsed-rail
/// toggling; `r15-studio-footer` builds out all 7 context-sensitive panels.
pub fn render_trailing_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> TrailingStudioFrameReport {
    let surface = resolve_studio_surface(app);
    let mut report = TrailingStudioFrameReport { dock_rect_pt: None, surface };
    if !ctx_in.layout.trailing_visible || ctx_in.layout.trailing_dock_w_pt <= 0.0 {
        return report;
    }

    let tokens = ctx_in.tokens;
    let w = ctx_in.layout.trailing_dock_w_pt;
    let dock_frame = egui::Frame::NONE.fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.divider)).inner_margin(egui::Margin::same(6));

    let resp = egui::Panel::right("egui_shell.trailing_studio").exact_size(w).resizable(false).frame(dock_frame).show(ui, |ui| {
        if ctx_in.layout.trailing_collapsed_to_rail {
            let expand_resp = ui.add_enabled(ctx_in.enabled, egui::Button::new("<").min_size(Vec2::new(24.0, 24.0)));
            meta.record_response(
                ui,
                &expand_resp,
                FrameInspectEntryInput {
                    widget_id: "inspector.expand_rail",
                    role: "button",
                    label: "Expand Studio",
                    command_id: Some("ui.inspector.toggle"),
                    value: None,
                    enabled: ctx_in.enabled,
                    disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if expand_resp.clicked() && ctx_in.enabled {
                intents.push_toggle_trailing_collapsed_rail();
            }
            return;
        }

        let avail = ui.available_rect_before_wrap();
        report.dock_rect_pt = Some(avail);
        if ctx_in.focused {
            ui.painter().rect_stroke(avail, egui::CornerRadius::same(2), Stroke::new(2.0, tokens.focus_ring), StrokeKind::Inside);
        }

        let title = match surface {
            StudioContextSurface::MultipleVariablesFormula => "Formula & Variable Studio",
            StudioContextSurface::TableSelection => "Table & Selection Studio",
            StudioContextSurface::MetadataHooks => "Metadata & Hooks",
            StudioContextSurface::ResultsInterpret => "Interpret & Diagnostics",
            StudioContextSurface::GraphInspector => "Graph Inspector",
            StudioContextSurface::DataPointDrillBack => "Data Point Drill-Back",
            StudioContextSurface::PageLayoutComposer => "Page Layout Composer",
        };
        ui.horizontal(|ui| {
            ui.strong(title);
            let collapse_resp = ui.add_enabled(ctx_in.enabled, egui::Button::new(">"));
            meta.record_response(
                ui,
                &collapse_resp,
                FrameInspectEntryInput {
                    widget_id: "inspector.collapse_rail",
                    role: "button",
                    label: "Collapse Studio",
                    command_id: Some("ui.inspector.toggle"),
                    value: None,
                    enabled: ctx_in.enabled,
                    disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if collapse_resp.clicked() && ctx_in.enabled {
                intents.push_toggle_trailing_collapsed_rail();
            }
        });
        ui.separator();
        ui.colored_label(tokens.text_muted, format!("Active surface: {surface:?}"));
        if matches!(surface, StudioContextSurface::ResultsInterpret) {
            let _ = state.results_tab;
            let copy_resp = ui.add_enabled(ctx_in.enabled, egui::Button::new("Copy Methods"));
            if copy_resp.clicked() && ctx_in.enabled {
                intents.push_ui("ui.result.copyMethodsReport", json!({}));
            }
        }
    });

    if report.dock_rect_pt.is_none() {
        report.dock_rect_pt = Some(resp.response.rect);
    }
    report
}
