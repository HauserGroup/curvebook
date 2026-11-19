//! 5-mode `CenterStage` orchestrator (`Welcome`, `ColumnGrid`, `InfoSheet`, `ResultSheet`,
//! `CanvasViewport`, `PortfolioGallery`, Split Comparison, Floating Notes, Diagnostic Banner,
//! and `chrono`-free Progress Modal; `docs/planning/gui-layout-spec.md` §5, `R15.3`).

use egui::{CornerRadius, Stroke, StrokeKind};

use crate::UiApp;
use crate::egui_shell::canvas_view::{CanvasViewContext, render_canvas_view};
use crate::egui_shell::data_grid::{ColumnGridRenderContext, render_column_grid};
use crate::egui_shell::info_view::render_info_view;
use crate::egui_shell::inspect_bridge::{HybridCoexistenceDecision, HybridHostTextureFrame, render_legacy_canvas_in_rect};
use crate::egui_shell::results_view::render_results_view;
use crate::egui_shell::welcome_hub::render_welcome_hub;
use crate::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, SplitOrientation, Tokens};
use crate::workspace_nav::ActiveWorkspacePane;

/// Resolved presentation mode for the primary `CenterStage` pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CenterStageMode {
    #[default]
    Welcome,
    ColumnGrid,
    InfoSheet,
    ResultSheet,
    CanvasViewport,
    Gallery,
    LegacyDeferredSheet,
}

/// Non-blocking / blocking async progress modal state (`chrono`-free; monotonic progress `[0, 1]`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressModalState {
    pub operation_label: String,
    pub detail: String,
    pub progress_0_to_1: Option<f32>,
    pub cancellable: bool,
    pub cancel_requested: bool,
}

/// Persistent `egui_shell` state owned by the `CenterStage` orchestrator.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CenterStageEguiState {
    pub split_dragging: bool,
    pub dismissed_banner_sheets: Vec<curvebook_doc::SheetId>,
    pub active_floating_note_drag: Option<(curvebook_doc::SheetId, usize)>,
    pub last_active_mode: CenterStageMode,
}

/// Input context bundle for [`render_center_stage`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct CenterStageContext<'a> {
    pub tokens: &'a Tokens,
    pub decision: &'a HybridCoexistenceDecision,
    pub layout: ResponsiveShellLayout,
    pub host_textures: Option<&'a HybridHostTextureFrame>,
    pub focused_pane: ActiveWorkspacePane,
    pub enabled: bool,
}

/// Per-frame geometry & backend handling report emitted by [`render_center_stage`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CenterStageFrameReport {
    pub primary_rect_pt: Option<egui::Rect>,
    pub secondary_rect_pt: Option<egui::Rect>,
    pub scrollbar_rect_pt: Option<egui::Rect>,
    pub legacy_canvas_texture_rect_pt: Option<egui::Rect>,
    pub legacy_canvas_widget_id: Option<egui::Id>,
    pub diagnostic_banner_rect_pt: Option<egui::Rect>,
    pub handled_natively: bool,
}

/// Resolve which [`CenterStageMode`] owns the primary stage pane for `(app, views)`.
#[must_use]
pub fn resolve_center_stage_mode(app: &UiApp, views: &EguiShellViewState) -> CenterStageMode {
    let doc = app.document();
    if views.launcher_open || doc.total_sheet_count() == 0 {
        return CenterStageMode::Welcome;
    }
    if app.nav.gallery.open {
        return CenterStageMode::Gallery;
    }
    if ctx_decision_renders_column(doc) {
        return CenterStageMode::ColumnGrid;
    }
    if let Some(active_id) = doc.active_sheet_id.as_ref() {
        if doc.info_sheet(active_id).is_some() {
            return CenterStageMode::InfoSheet;
        }
        if doc.result_sheet(active_id).is_some() {
            return CenterStageMode::ResultSheet;
        }
        if doc.graph_sheet(active_id).is_some() || doc.layout_sheet(active_id).is_some() {
            return CenterStageMode::CanvasViewport;
        }
    }
    CenterStageMode::LegacyDeferredSheet
}

fn ctx_decision_renders_column(doc: &curvebook_doc::ProjectDocument) -> bool {
    doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id)).is_some_and(|ds| matches!(ds.archetype, curvebook_doc::TableArchetype::Column(_)))
}

/// Render `CenterStage` including Split View (`Cmd/Ctrl+\`), active pane focus ring (`F6`),
/// Portfolio Gallery, `InfoSheet`, `ResultSheet`, `Column` grid, async `CanvasViewport`, and
/// deferred `R15.4` legacy fallback.
pub fn render_center_stage(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: CenterStageContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> CenterStageFrameReport {
    let mut report = CenterStageFrameReport::default();
    let mode = resolve_center_stage_mode(app, views);
    views.center_stage.last_active_mode = mode;

    let stage_rect = ui.available_rect_before_wrap();
    if !stage_rect.is_positive() {
        return report;
    }

    let (primary_rect, secondary_rect) = if app.split.enabled && !matches!(mode, CenterStageMode::Welcome | CenterStageMode::Gallery) {
        split_stage_rects(stage_rect, app.split.ratio as f32, ctx_in.layout.split_orientation)
    } else {
        (stage_rect, None)
    };
    report.primary_rect_pt = Some(primary_rect);
    report.secondary_rect_pt = secondary_rect;

    ui.scope_builder(egui::UiBuilder::new().max_rect(primary_rect), |ui| {
        let sub_rep = render_primary_mode(ui, app, views, mode, &ctx_in, meta, intents);
        report.handled_natively = sub_rep.handled_natively;
        report.scrollbar_rect_pt = sub_rep.scrollbar_rect_pt;
        report.legacy_canvas_texture_rect_pt = sub_rep.legacy_canvas_texture_rect_pt;
        report.legacy_canvas_widget_id = sub_rep.legacy_canvas_widget_id;
        if ctx_in.focused_pane == ActiveWorkspacePane::MainCanvasOrGrid {
            ui.painter().rect_stroke(primary_rect, CornerRadius::same(2), Stroke::new(2.0, ctx_in.tokens.focus_ring), StrokeKind::Inside);
        }
    });

    if let Some(sec_rect) = secondary_rect {
        ui.scope_builder(egui::UiBuilder::new().max_rect(sec_rect), |ui| {
            let _ = render_canvas_view(
                ui,
                app,
                &mut views.canvas_view,
                CanvasViewContext { tokens: ctx_in.tokens, host_textures: None, enabled: ctx_in.enabled, is_secondary_split: true },
                meta,
                intents,
            );
        });
    }

    report
}

fn render_primary_mode(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    mode: CenterStageMode,
    ctx_in: &CenterStageContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> CenterStageFrameReport {
    let mut report = CenterStageFrameReport::default();
    let primary_rect = ui.max_rect();
    match mode {
        CenterStageMode::Welcome => {
            report.handled_natively = true;
            let _ = render_welcome_hub(ui, app, &mut views.welcome_hub, ctx_in.tokens, ctx_in.enabled, meta, intents);
        }
        CenterStageMode::ColumnGrid => {
            report.handled_natively = true;
            let grid_rep = render_column_grid(
                ui,
                app,
                &mut views.column_grid,
                ColumnGridRenderContext::new(ctx_in.tokens, ctx_in.decision.blocks_egui_grid),
                meta,
                intents,
            );
            report.scrollbar_rect_pt = grid_rep.vertical_scrollbar_rect_pt.or(grid_rep.horizontal_scrollbar_rect_pt);
        }
        CenterStageMode::InfoSheet => {
            report.handled_natively = true;
            let _ = render_info_view(ui, app, &mut views.info_view, ctx_in.tokens, ctx_in.enabled, meta, intents);
        }
        CenterStageMode::ResultSheet => {
            report.handled_natively = true;
            let _ = render_results_view(ui, app, &mut views.results_view, ctx_in.tokens, ctx_in.enabled, meta, intents);
        }
        CenterStageMode::CanvasViewport => {
            report.handled_natively = true;
            let c_rep = render_canvas_view(
                ui,
                app,
                &mut views.canvas_view,
                CanvasViewContext { tokens: ctx_in.tokens, host_textures: ctx_in.host_textures, enabled: ctx_in.enabled, is_secondary_split: false },
                meta,
                intents,
            );
            report.legacy_canvas_texture_rect_pt = c_rep.canvas_rect_pt;
            report.legacy_canvas_widget_id = c_rep.canvas_widget_id;
        }
        CenterStageMode::Gallery | CenterStageMode::LegacyDeferredSheet => {
            report.handled_natively = false;
            let resp = render_legacy_canvas_in_rect(ui, app, primary_rect, ctx_in.host_textures);
            report.legacy_canvas_texture_rect_pt = Some(primary_rect);
            report.legacy_canvas_widget_id = Some(resp.id);
        }
    }
    report
}

fn split_stage_rects(stage: egui::Rect, ratio: f32, orientation: SplitOrientation) -> (egui::Rect, Option<egui::Rect>) {
    let clamped = if ratio.is_finite() { ratio.clamp(0.15, 0.85) } else { 0.5 };
    match orientation {
        SplitOrientation::Horizontal => {
            let split_x = stage.left() + (stage.width() - 4.0).max(0.0) * clamped;
            let left = egui::Rect::from_min_max(stage.min, egui::pos2(split_x, stage.bottom()));
            let right = egui::Rect::from_min_max(egui::pos2((split_x + 4.0).min(stage.right()), stage.top()), stage.max);
            (left, right.is_positive().then_some(right))
        }
        SplitOrientation::Vertical => {
            let split_y = stage.top() + (stage.height() - 4.0).max(0.0) * clamped;
            let top = egui::Rect::from_min_max(stage.min, egui::pos2(stage.right(), split_y));
            let bottom = egui::Rect::from_min_max(egui::pos2(stage.left(), (split_y + 4.0).min(stage.bottom())), stage.max);
            (top, bottom.is_positive().then_some(bottom))
        }
    }
}
