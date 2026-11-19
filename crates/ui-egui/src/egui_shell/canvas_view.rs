//! Native `egui` async `ChartScene` / `LayoutScene` viewport (`docs/planning/gui-layout-spec.md`
//! §8.11A & §8.12, `R15.3`).

use crate::UiApp;
use crate::egui_shell::inspect_bridge::{HybridHostTextureFrame, render_legacy_canvas_in_rect};
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use crate::layout::ZLayer;

/// Live plotted-point / heatmap / colorbar hover readout rendered in the canvas overlay and footer.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasHoverReadout {
    pub source_sheet_id: Option<curvebook_doc::SheetId>,
    pub point_index: Option<usize>,
    pub row: Option<usize>,
    pub col: Option<usize>,
    pub label: String,
    pub value_summary: String,
}

/// Cached scene raster/async status slot inside [`CanvasViewEguiState`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasSceneRenderSlot {
    pub cached_texture: Option<egui::TextureId>,
    pub last_sheet_id: Option<curvebook_doc::SheetId>,
    pub last_mutation_seq: u64,
    pub last_width_px: u32,
    pub last_height_px: u32,
    pub last_dpi: u16,
    pub last_filter_slug: String,
    pub rendering_in_flight: bool,
    pub layout_return_breadcrumb: Option<curvebook_doc::SheetId>,
}

/// Persistent `egui_shell` viewport state for `GraphSheet` and `LayoutSheet` scenes (`R15.3`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasViewEguiState {
    pub slot: CanvasSceneRenderSlot,
    pub hover_readout: Option<CanvasHoverReadout>,
    pub pan_pt: [f32; 2],
    pub selected_panel_index: Option<usize>,
}

/// Input bundle passed to [`render_canvas_view`] (keeping argument count within `<= 7`).
#[derive(Debug, Clone, Copy)]
pub struct CanvasViewContext<'a> {
    pub tokens: &'a Tokens,
    pub host_textures: Option<&'a HybridHostTextureFrame>,
    pub enabled: bool,
    pub is_secondary_split: bool,
}

/// Per-frame viewport report returned by [`render_canvas_view`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasViewFrameReport {
    pub canvas_rect_pt: Option<egui::Rect>,
    pub canvas_widget_id: Option<egui::Id>,
    pub hover_readout: Option<CanvasHoverReadout>,
    pub layout_breadcrumb_shown: bool,
}

/// Render an interactive `GraphSheet` or `LayoutSheet` inside `CenterStage` (`§8.11A` / `§8.12`).
///
/// The Wave 3 base paints the active sheet scene via the shared renderer/host texture fallback and
/// records `canvas.viewport` inspection metadata; `r15-stage-canvas` owns the background async
/// rasterizer, hover hit-testing, KM `Number at risk` alignment, live `TeX` preview, and
/// double-click Graph drill-down with `Return to Layout` breadcrumb.
pub fn render_canvas_view(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut CanvasViewEguiState,
    ctx_in: CanvasViewContext<'_>,
    meta: &mut FrameInspectMeta,
    _intents: &mut PendingUiIntents,
) -> CanvasViewFrameReport {
    let mut report = CanvasViewFrameReport::default();
    let canvas_rect = ui.available_rect_before_wrap();
    report.canvas_rect_pt = canvas_rect.is_positive().then_some(canvas_rect);
    if !canvas_rect.is_positive() {
        return report;
    }

    let host_tex = if ctx_in.is_secondary_split { None } else { ctx_in.host_textures };
    let resp = render_legacy_canvas_in_rect(ui, app, canvas_rect, host_tex);
    report.canvas_widget_id = Some(resp.id);
    report.hover_readout = state.hover_readout.clone();
    report.layout_breadcrumb_shown = state.slot.layout_return_breadcrumb.is_some();

    let wid = if ctx_in.is_secondary_split { "canvas.split.viewport" } else { "canvas.viewport" };
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: wid,
            role: "canvas",
            label: "Graph / Layout Canvas",
            command_id: None,
            value: None,
            enabled: ctx_in.enabled,
            disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );

    report
}
