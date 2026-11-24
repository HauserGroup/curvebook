//! Native `egui` async `ChartScene` / `LayoutScene` viewport (`docs/planning/gui-layout-spec.md`
//! §8.11A & §8.12, `R15.3`).

use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};

use curvebook_chart::equation_box::resolve_model_equation_hit_bounds;
use curvebook_chart::{ChartScene, PathCmd, PointPt, RectPt, ScenePrimitive};
use curvebook_doc::{
    AnalysisKind, EquationDisplayMode, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject, LayoutItem, LayoutSheet, ProjectDocument,
    RectPt as DocRectPt, ResultTabKind, SheetId,
};
use curvebook_fit::tex_format::{format_model_equation_tex, format_polynomial_equation_tex};
use curvebook_palette::{CanvasPreviewFilter, Rgba8};
use curvebook_platform::sync::RwLock;
use curvebook_render::{RasterImage, RenderTargetSpec, apply_canvas_preview_filter, render_scene};
use curvebook_tools::GridCellCoord;
use curvebook_tools::canvas::{MAX_ZOOM, MIN_ZOOM, PhysicalUnit, SnapGuide, SnapOrientation, SnapSource};
use egui::{Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, rgba8_to_color32};
use crate::i18n::tr;
use crate::inspect::ZLayer;

const RULER_THICKNESS_PT: f32 = 24.0;
const BREADCRUMB_HEIGHT_PT: f32 = 28.0;

/// Active gesture kind on an embedded canvas image or annotation (`R13.4` / `R13.7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageGestureKind {
    Move,
    ResizeCorner { left: bool, top: bool },
    Rotate,
}

/// Interactive drag/resize/rotate state for embedded images & tables on `GraphSheet` and `LayoutSheet`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageTransformGestureState {
    pub sheet_id: SheetId,
    pub is_layout: bool,
    pub target_index: usize,
    pub kind: ImageGestureKind,
    pub start_scene_pt: [f64; 2],
    pub start_screen_pt: [f32; 2],
    pub initial_bounds_pt: DocRectPt,
    pub current_bounds_pt: DocRectPt,
    pub initial_rotation_deg: f64,
    pub current_rotation_deg: f64,
    pub initial_lock_aspect: bool,
    pub shift_unlocked: bool,
}

/// Live plotted-point / heatmap / colorbar hover readout rendered in the canvas overlay and footer.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasHoverReadout {
    pub source_sheet_id: Option<SheetId>,
    pub point_index: Option<usize>,
    pub row: Option<usize>,
    pub col: Option<usize>,
    pub label: String,
    pub value_summary: String,
}

/// Active interactive layout panel drag/resize gesture on `LayoutSheet`.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutPanelDragState {
    pub sheet_id: SheetId,
    pub item_index: usize,
    pub start_scene_pt: [f64; 2],
    pub initial_bounds_pt: DocRectPt,
    pub current_bounds_pt: DocRectPt,
    pub resizing_corner: bool,
}

/// Cached scene raster/async status slot inside [`CanvasViewEguiState`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasSceneRenderSlot {
    pub cached_texture: Option<egui::TextureId>,
    pub last_raster_failed: bool,
    pub last_sheet_id: Option<SheetId>,
    pub last_mutation_seq: u64,
    pub last_width_px: u32,
    pub last_height_px: u32,
    pub last_dpi: u16,
    pub last_filter_slug: String,
    pub rendering_in_flight: bool,
    pub layout_return_breadcrumb: Option<SheetId>,
}

/// Per-sheet persistent viewport pan & selection state (`FA-SEC16-10`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasPerSheetViewportState {
    pub pan_pt: [f32; 2],
    pub selected_panel_index: Option<usize>,
    pub selected_graph_annotation: Option<usize>,
}

/// Persistent `egui_shell` viewport state for `GraphSheet` and `LayoutSheet` scenes (`R15.3`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasViewEguiState {
    pub slot: CanvasSceneRenderSlot,
    pub hover_readout: Option<CanvasHoverReadout>,
    pub pan_pt: [f32; 2],
    pub selected_panel_index: Option<usize>,
    pub active_snap_guides: Vec<SnapGuide>,
    pub layout_drag: Option<LayoutPanelDragState>,
    pub last_snap_readout: Option<String>,
    pub km_at_risk_rows: usize,
    pub tex_equation_count: usize,
    /// Render uncached scenes inline synchronously (used by software screenshot capture).
    pub synchronous_raster: bool,
    /// Active layout panel or canvas right-click context menu `(layout_sheet_id, opt_item_index, anchor_pos_pt)`.
    pub layout_context_menu: Option<(SheetId, Option<usize>, Pos2)>,
    /// Selected annotation index on the active `GraphSheet` (`EmbeddedResultTable` or `EmbeddedImage`).
    pub selected_graph_annotation: Option<usize>,
    /// Active `GraphSheet` right-click context menu `(graph_sheet_id, opt_ann_index, screen_anchor, scene_pt)`.
    pub graph_context_menu: Option<(SheetId, Option<usize>, Pos2, [f64; 2])>,
    /// Active move / corner-resize / rotation gesture on a graph or layout embedded object.
    pub image_gesture: Option<ImageTransformGestureState>,
    /// Last rendered active canvas sheet and paper rect/scene dimensions (for accurate footer cursor coords).
    pub last_active_paper_sheet_id: Option<SheetId>,
    pub last_paper_rect_pt: Option<Rect>,
    pub last_scene_size_pt: Option<[f64; 2]>,
    /// Sheet whose pan & selection are currently materialized in `pan_pt` / `selected_*`.
    pub active_viewport_sheet: Option<SheetId>,
    /// Saved pan & selection state keyed per [`SheetId`].
    pub sheet_viewports: BTreeMap<SheetId, CanvasPerSheetViewportState>,
}

/// Input bundle passed to [`render_canvas_view`] (keeping argument count within `<= 7`).
#[derive(Debug, Clone, Copy)]
pub struct CanvasViewContext<'a> {
    pub tokens: &'a Tokens,
    pub enabled: bool,
    pub is_secondary_split: bool,
}

/// Per-frame viewport report returned by [`render_canvas_view`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasViewFrameReport {
    pub canvas_rect_pt: Option<Rect>,
    pub canvas_widget_id: Option<egui::Id>,
    pub hover_readout: Option<CanvasHoverReadout>,
    pub layout_breadcrumb_shown: bool,
    pub paper_rect_pt: Option<Rect>,
    pub km_track_rect_pt: Option<Rect>,
    pub snap_guide_count: usize,
    pub tex_equation_rects_pt: Vec<Rect>,
    pub colorbar_hovered: bool,
    pub async_rendering_in_flight: bool,
    pub grid_overlay_shown: bool,
}

#[derive(Clone)]
struct CachedCompiledScene {
    mutation_seq: u64,
    scene: Arc<ChartScene>,
}

/// Diagnostic snapshot of the single long-lived async canvas raster worker (`R15-GUI-04`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanvasAsyncWorkerStats {
    pub worker_spawn_count: u64,
    pub submitted_count: u64,
    pub rendered_count: u64,
}

/// Inspect runtime statistics for the per-context async canvas raster worker, if initialized.
#[must_use]
pub fn canvas_async_worker_stats(ctx: &egui::Context) -> Option<CanvasAsyncWorkerStats> {
    let worker_id = Id::unique("egui_shell.canvas_async_worker");
    let handle: AsyncRasterWorkerHandle = ctx.data(|d| d.get_temp(worker_id))?;
    let guard = handle.owner.results.read_or_recover().ok()?;
    Some(CanvasAsyncWorkerStats {
        worker_spawn_count: guard.worker_spawn_count,
        submitted_count: guard.submitted_count,
        rendered_count: guard.rendered_count,
    })
}

struct RasterJobRequest {
    slot_key: (String, SheetId),
    req_key: RasterRequestKey,
    scene: Arc<ChartScene>,
    filter: CanvasPreviewFilter,
    ctx_wake: egui::Context,
}

#[derive(Default)]
struct RasterWorkerResults {
    completed: Vec<((String, SheetId), RasterRequestKey, Option<RasterImage>)>,
    last_submitted: Vec<((String, SheetId), RasterRequestKey)>,
    submitted_count: u64,
    rendered_count: u64,
    worker_spawn_count: u64,
}

struct AsyncRasterWorkerOwner {
    tx: RwLock<Option<mpsc::Sender<RasterJobRequest>>>,
    results: Arc<RwLock<RasterWorkerResults>>,
    join: RwLock<Option<std::thread::JoinHandle<()>>>,
}

impl Drop for AsyncRasterWorkerOwner {
    fn drop(&mut self) {
        // Close the request channel first so the worker exits its `recv()` loop cleanly.
        if let Ok(mut tx_guard) = self.tx.write_or_recover() {
            let _ = tx_guard.take();
        }
        let handle = self.join.write_or_recover().ok().and_then(|mut guard| guard.take());
        if let Some(join_handle) = handle {
            // An in-flight job holds an `egui::Context` clone, and the context owns this worker
            // handle. When the worker drops the last context reference, this `Drop` runs on the
            // worker thread itself; joining there fails with `EDEADLK` and `JoinHandle::join`
            // panics. The closed channel already makes the worker return, so detach instead.
            if join_handle.thread().id() != std::thread::current().id() {
                let _ = join_handle.join();
            }
        }
    }
}

#[derive(Clone)]
struct AsyncRasterWorkerHandle {
    owner: Arc<AsyncRasterWorkerOwner>,
}

enum AsyncRasterPoll {
    Ready(Option<RasterImage>),
    InFlight,
    Unavailable,
}

impl AsyncRasterWorkerHandle {
    fn try_get(ctx: &egui::Context) -> Option<Self> {
        let worker_id = Id::unique("egui_shell.canvas_async_worker");
        ctx.data(|d| d.get_temp::<Self>(worker_id))
    }

    fn prune_deleted_sheets(&self, doc: &ProjectDocument) {
        let exists = |id: &SheetId| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some();
        if let Ok(mut guard) = self.owner.results.write_or_recover() {
            guard.completed.retain(|((_, sid), _, _)| exists(sid));
            guard.last_submitted.retain(|((_, sid), _)| exists(sid));
        }
    }

    fn get_or_spawn(ctx: &egui::Context) -> Option<Self> {
        let worker_id = Id::unique("egui_shell.canvas_async_worker");
        if let Some(existing) = ctx.data(|d| d.get_temp::<Self>(worker_id)) {
            return Some(existing);
        }
        let (tx, rx) = mpsc::channel();
        let results = Arc::new(RwLock::named(
            "egui_shell.canvas_raster_results",
            RasterWorkerResults { worker_spawn_count: 1, ..RasterWorkerResults::default() },
        ));
        let results_bg = Arc::clone(&results);
        let join_handle =
            std::thread::Builder::new().name(String::from("cb-canvas-raster")).spawn(move || raster_worker_loop(&rx, &results_bg)).ok()?;
        let handle = Self {
            owner: Arc::new(AsyncRasterWorkerOwner {
                tx: RwLock::named("egui_shell.canvas_raster_tx", Some(tx)),
                results,
                join: RwLock::named("egui_shell.canvas_raster_join", Some(join_handle)),
            }),
        };
        ctx.data_mut(|d| d.insert_temp(worker_id, handle.clone()));
        Some(handle)
    }

    fn poll_or_submit(
        &self,
        slot_key: (String, SheetId),
        req_key: RasterRequestKey,
        scene: &Arc<ChartScene>,
        filter: CanvasPreviewFilter,
        ctx_wake: &egui::Context,
    ) -> AsyncRasterPoll {
        let Ok(mut guard) = self.owner.results.write_or_recover() else {
            return AsyncRasterPoll::Unavailable;
        };
        if let Some(pos) = guard.completed.iter().position(|(sk, rk, _)| *sk == slot_key && *rk == req_key) {
            let (_, _, img) = guard.completed.remove(pos);
            return AsyncRasterPoll::Ready(img);
        }
        let already_submitted = guard.last_submitted.iter().any(|(sk, rk)| *sk == slot_key && *rk == req_key);
        if already_submitted {
            return AsyncRasterPoll::InFlight;
        }
        if let Ok(tx_guard) = self.owner.tx.read_or_recover()
            && let Some(tx) = tx_guard.as_ref()
        {
            let request = RasterJobRequest {
                slot_key: slot_key.clone(),
                req_key: req_key.clone(),
                scene: Arc::clone(scene),
                filter,
                ctx_wake: ctx_wake.clone(),
            };
            if tx.send(request).is_ok() {
                if let Some(entry) = guard.last_submitted.iter_mut().find(|(sk, _)| *sk == slot_key) {
                    entry.1 = req_key;
                } else {
                    guard.last_submitted.push((slot_key, req_key));
                }
                guard.submitted_count = guard.submitted_count.saturating_add(1);
                return AsyncRasterPoll::InFlight;
            }
        }
        AsyncRasterPoll::Unavailable
    }
}

fn coalesce_pending_raster_jobs(rx: &mpsc::Receiver<RasterJobRequest>, pending: &mut Vec<RasterJobRequest>) {
    while let Ok(next) = rx.try_recv() {
        if let Some(existing) = pending.iter_mut().find(|job| job.slot_key == next.slot_key) {
            // Supersede any unstarted request for the same slot before rendering begins.
            *existing = next;
        } else {
            pending.push(next);
        }
    }
}

fn raster_worker_loop(rx: &mpsc::Receiver<RasterJobRequest>, results: &RwLock<RasterWorkerResults>) {
    struct WakeGuard(Option<egui::Context>);
    impl Drop for WakeGuard {
        fn drop(&mut self) {
            if let Some(ctx) = self.0.take() {
                ctx.request_repaint();
            }
        }
    }

    let mut pending: Vec<RasterJobRequest> = Vec::new();
    loop {
        if pending.is_empty() {
            let Ok(first) = rx.recv() else {
                return;
            };
            pending.push(first);
        }
        coalesce_pending_raster_jobs(rx, &mut pending);
        let job = pending.remove(0);
        let mut wake_guard = WakeGuard(Some(job.ctx_wake.clone()));

        let rendered =
            render_scene(&job.scene, RenderTargetSpec { width_px: job.req_key.width_px, height_px: job.req_key.height_px, dpi: job.req_key.dpi })
                .ok()
                .map(|mut img| {
                    if job.filter != CanvasPreviewFilter::Normal {
                        apply_canvas_preview_filter(&mut img, job.filter);
                    }
                    img
                });

        coalesce_pending_raster_jobs(rx, &mut pending);
        let superseded = pending.iter().any(|p| p.slot_key == job.slot_key);
        if superseded {
            let _ = wake_guard.0.take();
        }
        if let Ok(mut guard) = results.write_or_recover() {
            guard.rendered_count = guard.rendered_count.saturating_add(1);
            if !superseded {
                if let Some(slot) = guard.completed.iter_mut().find(|(sk, _, _)| *sk == job.slot_key) {
                    *slot = (job.slot_key, job.req_key, rendered);
                } else {
                    guard.completed.push((job.slot_key, job.req_key, rendered));
                }
            }
        }
        if !superseded && let Some(ctx) = wake_guard.0.take() {
            ctx.request_repaint();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RasterRequestKey {
    sheet_id: SheetId,
    mutation_seq: u64,
    width_px: u32,
    height_px: u32,
    dpi: u16,
    filter_slug: String,
}

#[derive(Debug, Clone)]
struct ColorbarRegion {
    rect_pt: RectPt,
    steps: Vec<(RectPt, Rgba8)>,
    min_val: f64,
    max_val: f64,
}

/// Render the active `GraphSheet` or `LayoutSheet` inside `CenterStage` (`§8.11A` / `§8.12`).
pub fn render_canvas_view(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut CanvasViewEguiState,
    ctx_in: CanvasViewContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> CanvasViewFrameReport {
    let active_id = app.document().active_sheet_id.clone();
    render_canvas_view_for_sheet(ui, app, active_id.as_ref(), state, ctx_in, meta, intents)
}

/// Render a specific `GraphSheet` or `LayoutSheet` inside `ui` (`§8.11A` / `§8.12`).
pub fn render_canvas_view_for_sheet(
    ui: &mut egui::Ui,
    app: &UiApp,
    sheet_id: Option<&SheetId>,
    state: &mut CanvasViewEguiState,
    ctx_in: CanvasViewContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> CanvasViewFrameReport {
    let mut report = CanvasViewFrameReport::default();
    let full_rect = ui.available_rect_before_wrap();
    if !full_rect.is_positive() {
        return report;
    }

    let tokens = ctx_in.tokens;
    let doc = app.document();
    let Some(target_id) = sheet_id else {
        return report;
    };

    let gs_opt = doc.graph_sheet(target_id);
    let ls_opt = doc.layout_sheet(target_id);
    if gs_opt.is_none() && ls_opt.is_none() {
        return report;
    }

    sync_canvas_sheet_viewport_state(state, doc, target_id);
    if let Some(worker) = AsyncRasterWorkerHandle::try_get(ui.ctx()) {
        worker.prune_deleted_sheets(doc);
    }

    // Clear stale layout drill-down breadcrumb when target layout no longer exists or user returned to layout.
    if ls_opt.is_some() && state.slot.layout_return_breadcrumb.as_ref() == Some(target_id) {
        state.slot.layout_return_breadcrumb = None;
    } else if let Some(bc_id) = state.slot.layout_return_breadcrumb.as_ref()
        && doc.layout_sheet(bc_id).is_none()
    {
        state.slot.layout_return_breadcrumb = None;
    }

    let mut content_rect = full_rect;
    if gs_opt.is_some()
        && let Some(layout_id) = state.slot.layout_return_breadcrumb.clone()
        && let Some(ls) = doc.layout_sheet(&layout_id)
        && content_rect.height() >= BREADCRUMB_HEIGHT_PT + 40.0
    {
        let bc_rect = Rect::from_min_size(content_rect.min, vec2(content_rect.width(), BREADCRUMB_HEIGHT_PT));
        content_rect = Rect::from_min_max(pos2(content_rect.left(), bc_rect.bottom()), content_rect.max);
        render_layout_breadcrumb_bar(ui, bc_rect, (ls, gs_opt), state, &ctx_in, meta, intents);
        report.layout_breadcrumb_shown = true;
    }

    let mut mat_rect = content_rect;
    let show_rulers = app.canvas_dialogs.rulers_visible && mat_rect.width() > 96.0 && mat_rect.height() > 96.0;
    let (top_ruler_rect, left_ruler_rect) = if show_rulers {
        let top_r = Rect::from_min_size(
            pos2(mat_rect.left() + RULER_THICKNESS_PT, mat_rect.top()),
            vec2((mat_rect.width() - RULER_THICKNESS_PT).max(1.0), RULER_THICKNESS_PT),
        );
        let left_r = Rect::from_min_size(
            pos2(mat_rect.left(), mat_rect.top() + RULER_THICKNESS_PT),
            vec2(RULER_THICKNESS_PT, (mat_rect.height() - RULER_THICKNESS_PT).max(1.0)),
        );
        mat_rect = Rect::from_min_max(pos2(mat_rect.left() + RULER_THICKNESS_PT, mat_rect.top() + RULER_THICKNESS_PT), mat_rect.max);
        (Some(top_r), Some(left_r))
    } else {
        (None, None)
    };

    report.canvas_rect_pt = Some(mat_rect);
    let slot_tag = if ctx_in.is_secondary_split { "split" } else { "main" };
    let wid = if ctx_in.is_secondary_split { "canvas.split.viewport" } else { "canvas.viewport" };
    let resp = ui.allocate_rect(mat_rect, if ctx_in.enabled { Sense::click_and_drag() } else { Sense::hover() });
    let canvas_enabled = ctx_in.enabled;
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Canvas, canvas_enabled, String::from("Graph / Layout Canvas")));
    report.canvas_widget_id = Some(resp.id);
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

    ui.painter().rect_filled(mat_rect, CornerRadius::ZERO, tokens.canvas_mat);

    let scene = resolve_cached_scene(ui.ctx(), app, target_id, slot_tag);
    let (scene_w_pt, scene_h_pt) = scene.as_ref().map_or((480.0_f32, 340.0_f32), |s| (s.width_pt.max(60.0) as f32, s.height_pt.max(60.0) as f32));

    // Reserve space below paper for KM Number-at-Risk alignment track when Survival graph enables it
    let km_reserve_h = if gs_opt.is_some_and(|g| should_show_km_track(g, doc)) { 64.0 } else { 0.0 };
    #[allow(clippy::cast_possible_truncation)]
    let zoom = (app.canvas_dialogs.canvas.zoom as f32).clamp(MIN_ZOOM as f32, MAX_ZOOM as f32);
    let fit_scale =
        ((mat_rect.width() - 28.0).max(40.0) / scene_w_pt).min((mat_rect.height() - 28.0 - km_reserve_h).max(40.0) / scene_h_pt).clamp(0.2, 4.0);
    let eff_scale = fit_scale * zoom;
    let paper_size = vec2(scene_w_pt * eff_scale, scene_h_pt * eff_scale);
    let pan_x = app.canvas_dialogs.canvas.pan_pt.x as f32 + state.pan_pt[0];
    let pan_y = app.canvas_dialogs.canvas.pan_pt.y as f32 + state.pan_pt[1];
    let paper_center = mat_rect.center() + vec2(pan_x, pan_y - km_reserve_h * 0.4);
    let paper_rect = Rect::from_center_size(paper_center, paper_size);
    report.paper_rect_pt = Some(paper_rect);
    if !ctx_in.is_secondary_split {
        state.last_active_paper_sheet_id = Some(target_id.clone());
        state.last_paper_rect_pt = Some(paper_rect);
        state.last_scene_size_pt = Some([f64::from(scene_w_pt), f64::from(scene_h_pt)]);
    }

    if let (Some(tr_r), Some(lr_r)) = (top_ruler_rect, left_ruler_rect) {
        render_canvas_rulers(
            ui,
            CanvasRulersSpec { mat_rect, top_r: tr_r, left_r: lr_r, paper_rect, eff_scale, unit: app.canvas_dialogs.ruler_unit, tokens },
        );
    }

    // Paint paper drop shadow & white sheet backing inside clipped mat_rect
    let clipped_painter = ui.painter().with_clip_rect(mat_rect);
    clipped_painter.rect_filled(paper_rect.translate(vec2(2.0, 2.0)), CornerRadius::same(2), Color32::from_black_alpha(32));
    clipped_painter.rect_filled(paper_rect, CornerRadius::ZERO, tokens.paper_white);
    clipped_painter.rect_stroke(paper_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    if let Some(ref sc) = scene {
        composite_async_scene_texture(
            ui,
            app,
            CompositeSceneSpec { scene: sc, sheet_id: target_id, paper_rect, slot_tag, painter: &clipped_painter },
            state,
        );
    }
    report.async_rendering_in_flight = state.slot.rendering_in_flight;

    if app.canvas_dialogs.grid_overlay_visible {
        paint_canvas_grid_overlay(&clipped_painter, mat_rect, paper_rect, scene_w_pt, scene_h_pt, app.canvas_dialogs.grid_overlay_spacing_pt, tokens);
        report.grid_overlay_shown = true;
    }

    if let Some(gs) = gs_opt
        && let Some(ref sc) = scene
    {
        render_graph_hud_overlays(
            ui,
            GraphHudInput { painter: &clipped_painter, app, gs, scene: sc, mat_rect, paper_rect, canvas_resp: &resp, ctx_in: &ctx_in },
            state,
            meta,
            intents,
            &mut report,
        );
    } else if let Some(ls) = ls_opt {
        render_layout_hud_overlays(
            ui,
            LayoutHudInput {
                painter: &clipped_painter,
                app,
                ls,
                scene: scene.as_deref(),
                mat_rect,
                paper_rect,
                eff_scale,
                canvas_resp: &resp,
                ctx_in: &ctx_in,
            },
            state,
            meta,
            intents,
            &mut report,
        );
    }

    // Pan gesture when dragging empty canvas with middle-mouse or Pan tool
    if ctx_in.enabled && resp.dragged_by(egui::PointerButton::Middle) {
        state.pan_pt[0] += resp.drag_delta().x;
        state.pan_pt[1] += resp.drag_delta().y;
    }

    state.sheet_viewports.insert(
        target_id.clone(),
        CanvasPerSheetViewportState {
            pan_pt: state.pan_pt,
            selected_panel_index: state.selected_panel_index,
            selected_graph_annotation: state.selected_graph_annotation,
        },
    );

    report.hover_readout = state.hover_readout.clone();
    report
}

fn sync_canvas_sheet_viewport_state(state: &mut CanvasViewEguiState, doc: &ProjectDocument, target_id: &SheetId) {
    state.sheet_viewports.retain(|sid, _| doc.graph_sheet(sid).is_some() || doc.layout_sheet(sid).is_some());
    if state.active_viewport_sheet.as_ref() == Some(target_id) {
        return;
    }
    if let Some(prev_id) = state.active_viewport_sheet.as_ref() {
        state.sheet_viewports.insert(
            prev_id.clone(),
            CanvasPerSheetViewportState {
                pan_pt: state.pan_pt,
                selected_panel_index: state.selected_panel_index,
                selected_graph_annotation: state.selected_graph_annotation,
            },
        );
    }
    if let Some(saved) = state.sheet_viewports.get(target_id).cloned() {
        state.pan_pt = saved.pan_pt;
        state.selected_panel_index = saved.selected_panel_index;
        state.selected_graph_annotation = saved.selected_graph_annotation;
    } else if state.active_viewport_sheet.is_some() {
        state.pan_pt = [0.0, 0.0];
        state.selected_panel_index = None;
        state.selected_graph_annotation = None;
    }
    state.active_viewport_sheet = Some(target_id.clone());
}

fn render_layout_breadcrumb_bar(
    ui: &mut egui::Ui,
    bc_rect: Rect,
    (ls, gs_opt): (&LayoutSheet, Option<&GraphSheet>),
    state: &mut CanvasViewEguiState,
    ctx_in: &CanvasViewContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let enabled = ctx_in.enabled;
    ui.painter().rect_filled(bc_rect, CornerRadius::ZERO, tokens.header_bg);
    ui.painter().line_segment([bc_rect.left_bottom(), bc_rect.right_bottom()], Stroke::new(1.0, tokens.divider));

    ui.scope_builder(egui::UiBuilder::new().max_rect(bc_rect.shrink2(vec2(8.0, 2.0))), |ui| {
        ui.set_clip_rect(bc_rect);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let ret_lbl = "← Return to Layout";
            let ret_btn = egui::Button::new(egui::RichText::new(ret_lbl).strong().color(tokens.text_strong)).fill(tokens.active_bg);
            let ret_resp = ui.add_enabled(enabled, ret_btn);
            meta.record_response(
                ui,
                &ret_resp,
                FrameInspectEntryInput {
                    widget_id: "canvas.return_to_layout",
                    role: "button",
                    label: ret_lbl,
                    command_id: Some("sheet.select"),
                    value: Some(json!({ "layout_sheet_id": ls.id.as_str() })),
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if ret_resp.clicked() && enabled {
                let target_layout = ls.id.clone();
                state.slot.layout_return_breadcrumb = None;
                intents.push_engine("sheet.select", json!({ "sheet_id": target_layout.as_str() }));
            }

            let graph_title = gs_opt.map_or("Graph", |g| g.title.as_str());
            ui.colored_label(tokens.text_muted, format!("{}  ›  {}", ls.title, graph_title));
        });
    });
}

pub(super) fn resolve_cached_scene(ctx: &egui::Context, app: &UiApp, sheet_id: &SheetId, slot_tag: &str) -> Option<Arc<ChartScene>> {
    let seq = app.mutation_seq();
    let cache_id = Id::unique(("egui_shell.compiled_scene", slot_tag, sheet_id.as_str()));
    let cached: Option<CachedCompiledScene> = ctx.data(|d| d.get_temp(cache_id));
    if let Some(entry) = cached
        && entry.mutation_seq == seq
    {
        return Some(entry.scene);
    }

    let compiled = app.session().render_sheet_scene(sheet_id).ok().map(Arc::new)?;
    ctx.data_mut(|d| {
        d.insert_temp(cache_id, CachedCompiledScene { mutation_seq: seq, scene: compiled.clone() });
    });
    Some(compiled)
}

struct CompositeSceneSpec<'a> {
    scene: &'a Arc<ChartScene>,
    sheet_id: &'a SheetId,
    paper_rect: Rect,
    slot_tag: &'a str,
    painter: &'a egui::Painter,
}

fn composite_async_scene_texture(ui: &egui::Ui, app: &UiApp, spec: CompositeSceneSpec<'_>, state: &mut CanvasViewEguiState) {
    let CompositeSceneSpec { scene, sheet_id, paper_rect, slot_tag, painter } = spec;
    let ppp = ui.ctx().pixels_per_point().clamp(1.0, 4.0);
    #[allow(clippy::cast_precision_loss)]
    let max_side = ui.ctx().input(|i| i.max_texture_side).clamp(32, 4096) as f32;
    let raw_w = (paper_rect.width() * ppp).max(32.0);
    let raw_h = (paper_rect.height() * ppp).max(32.0);
    let downscale = (max_side / raw_w.max(raw_h)).min(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let width_px = (raw_w * downscale).round().clamp(32.0, max_side) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height_px = (raw_h * downscale).round().clamp(32.0, max_side) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let dpi = (f64::from(ppp) * 96.0).round().clamp(72.0, 600.0) as u16;
    let filter_slug = app.preview_filter.as_str().to_owned();
    let req_key =
        RasterRequestKey { sheet_id: sheet_id.clone(), mutation_seq: app.mutation_seq(), width_px, height_px, dpi, filter_slug: filter_slug.clone() };

    let slot_matches = state.slot.last_sheet_id.as_ref() == Some(sheet_id)
        && state.slot.last_mutation_seq == req_key.mutation_seq
        && state.slot.last_width_px == width_px
        && state.slot.last_height_px == height_px
        && state.slot.last_dpi == dpi
        && state.slot.last_filter_slug == filter_slug
        && (state.slot.cached_texture.is_some() || state.slot.last_raster_failed);

    if slot_matches {
        state.slot.rendering_in_flight = false;
        if let Some(tex_id) = state.slot.cached_texture {
            painter.image(tex_id, paper_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        } else {
            paint_chart_scene_vector_preview(painter, scene, paper_rect);
        }
        return;
    }

    let worker = (!state.synchronous_raster).then(|| AsyncRasterWorkerHandle::get_or_spawn(ui.ctx())).flatten();
    if state.synchronous_raster || worker.is_none() {
        let mut rendered = render_scene(scene, RenderTargetSpec { width_px, height_px, dpi }).ok();
        if let Some(ref mut img) = rendered
            && app.preview_filter != CanvasPreviewFilter::Normal
        {
            apply_canvas_preview_filter(img, app.preview_filter);
        }
        state.slot.rendering_in_flight = false;
        state.slot.last_sheet_id = Some(sheet_id.clone());
        state.slot.last_mutation_seq = req_key.mutation_seq;
        state.slot.last_width_px = width_px;
        state.slot.last_height_px = height_px;
        state.slot.last_dpi = dpi;
        state.slot.last_filter_slug = filter_slug.clone();
        if let Some(raster) = rendered {
            let color_img = egui::ColorImage::from_rgba_unmultiplied([raster.width_px as usize, raster.height_px as usize], &raster.rgba);
            let tex_id = upload_canvas_texture(ui.ctx(), slot_tag, color_img);
            state.slot.cached_texture = Some(tex_id);
            state.slot.last_raster_failed = false;
            painter.image(tex_id, paper_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        } else {
            state.slot.cached_texture = None;
            state.slot.last_raster_failed = true;
            paint_chart_scene_vector_preview(painter, scene, paper_rect);
        }
        return;
    }

    if let Some(worker) = worker {
        let slot_key = (slot_tag.to_owned(), sheet_id.clone());
        match worker.poll_or_submit(slot_key, req_key.clone(), scene, app.preview_filter, ui.ctx()) {
            AsyncRasterPoll::Ready(maybe_img) => {
                state.slot.rendering_in_flight = false;
                state.slot.last_sheet_id = Some(sheet_id.clone());
                state.slot.last_mutation_seq = req_key.mutation_seq;
                state.slot.last_width_px = width_px;
                state.slot.last_height_px = height_px;
                state.slot.last_dpi = dpi;
                state.slot.last_filter_slug = filter_slug;
                if let Some(raster) = maybe_img {
                    let color_img = egui::ColorImage::from_rgba_unmultiplied([raster.width_px as usize, raster.height_px as usize], &raster.rgba);
                    let tex_id = upload_canvas_texture(ui.ctx(), slot_tag, color_img);
                    state.slot.cached_texture = Some(tex_id);
                    state.slot.last_raster_failed = false;
                    painter.image(tex_id, paper_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                } else {
                    state.slot.cached_texture = None;
                    state.slot.last_raster_failed = true;
                    paint_chart_scene_vector_preview(painter, scene, paper_rect);
                }
                return;
            }
            AsyncRasterPoll::InFlight => {
                // Worker thread wakes `job.ctx_wake.request_repaint()` directly when done;
                // avoid polling the main thread on a 16 ms timer while rasterization runs.
                state.slot.rendering_in_flight = true;
            }
            AsyncRasterPoll::Unavailable => {
                state.slot.rendering_in_flight = false;
                state.slot.last_sheet_id = Some(sheet_id.clone());
                state.slot.last_mutation_seq = req_key.mutation_seq;
                state.slot.last_width_px = width_px;
                state.slot.last_height_px = height_px;
                state.slot.last_dpi = dpi;
                state.slot.last_filter_slug = filter_slug;
                state.slot.cached_texture = None;
                state.slot.last_raster_failed = true;
            }
        }
    }

    // While async worker finishes on frame 1, show previous cached texture or crisp vector scene preview
    if let Some(prev_tex) = state.slot.cached_texture
        && state.slot.last_sheet_id.as_ref() == Some(sheet_id)
    {
        painter.image(prev_tex, paper_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    } else {
        paint_chart_scene_vector_preview(painter, scene, paper_rect);
    }
}

fn upload_canvas_texture(ctx: &egui::Context, slot_tag: &str, color_img: egui::ColorImage) -> egui::TextureId {
    let tex_slot_id = Id::unique(("egui_shell.canvas_tex", slot_tag));
    let mut handle: Option<egui::TextureHandle> = ctx.data(|d| d.get_temp(tex_slot_id));
    match handle.as_mut() {
        Some(tex) => {
            tex.set(color_img, egui::TextureOptions::LINEAR);
            let id = tex.id();
            ctx.data_mut(|d| d.insert_temp(tex_slot_id, tex.clone()));
            id
        }
        None => {
            let tex = ctx.load_texture(format!("canvas_scene_{slot_tag}"), color_img, egui::TextureOptions::LINEAR);
            let id = tex.id();
            ctx.data_mut(|d| d.insert_temp(tex_slot_id, tex));
            id
        }
    }
}

fn paint_chart_scene_vector_preview(painter: &egui::Painter, scene: &ChartScene, paper_rect: Rect) {
    let sx = paper_rect.width() / (scene.width_pt.max(1.0) as f32);
    let sy = paper_rect.height() / (scene.height_pt.max(1.0) as f32);
    let map_pt = |pt: PointPt| pos2(paper_rect.left() + (pt.x as f32) * sx, paper_rect.top() + (pt.y as f32) * sy);

    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, fill, stroke, .. } = prim {
            let mut pts = Vec::new();
            let mut closed = false;
            for cmd in &path.commands {
                match *cmd {
                    PathCmd::MoveTo(p) => {
                        if pts.len() >= 2 {
                            paint_polyline(painter, &pts, closed, fill.as_ref(), stroke.as_ref(), sx);
                        }
                        pts.clear();
                        closed = false;
                        pts.push(map_pt(p));
                    }
                    PathCmd::LineTo(p) | PathCmd::QuadTo { to: p, .. } | PathCmd::CubicTo { to: p, .. } => {
                        pts.push(map_pt(p));
                    }
                    PathCmd::Close => closed = true,
                }
            }
            if pts.len() >= 2 {
                paint_polyline(painter, &pts, closed, fill.as_ref(), stroke.as_ref(), sx);
            }
        }
    }
}

fn paint_polyline(
    painter: &egui::Painter,
    pts: &[Pos2],
    closed: bool,
    fill: Option<&curvebook_chart::FillPaint>,
    stroke: Option<&curvebook_chart::StrokePaint>,
    scale: f32,
) {
    if closed && let Some(f) = fill {
        painter.add(egui::Shape::convex_polygon(pts.to_vec(), rgba8_to_color32(f.color), Stroke::NONE));
    }
    if let Some(s) = stroke {
        let st = Stroke::new((s.width_pt as f32 * scale).max(0.75), rgba8_to_color32(s.color));
        if closed {
            painter.add(egui::Shape::closed_line(pts.to_vec(), st));
        } else {
            painter.add(egui::Shape::line(pts.to_vec(), st));
        }
    }
}

struct GraphHudInput<'a> {
    painter: &'a egui::Painter,
    app: &'a UiApp,
    gs: &'a GraphSheet,
    scene: &'a ChartScene,
    mat_rect: Rect,
    paper_rect: Rect,
    canvas_resp: &'a egui::Response,
    ctx_in: &'a CanvasViewContext<'a>,
}

struct GraphOverlayCtx<'a> {
    painter: &'a egui::Painter,
    gs: &'a GraphSheet,
    doc: &'a ProjectDocument,
    scene: &'a ChartScene,
    paper_rect: Rect,
    tokens: &'a Tokens,
}

fn render_graph_hud_overlays(
    ui: &mut egui::Ui,
    input: GraphHudInput<'_>,
    state: &mut CanvasViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CanvasViewFrameReport,
) {
    let GraphHudInput { painter, app, gs, scene, mat_rect, paper_rect, canvas_resp, ctx_in } = input;
    let doc = app.document();
    let tokens = ctx_in.tokens;
    let sx = paper_rect.width() / (scene.width_pt.max(1.0) as f32);
    let sy = paper_rect.height() / (scene.height_pt.max(1.0) as f32);
    let scene_to_screen = |pt: PointPt| pos2(paper_rect.left() + (pt.x as f32) * sx, paper_rect.top() + (pt.y as f32) * sy);
    let screen_to_scene =
        |pos: Pos2| PointPt::new(f64::from((pos.x - paper_rect.left()) / sx.max(1e-4)), f64::from((pos.y - paper_rect.top()) / sy.max(1e-4)));

    let canvas_ov = CanvasOverlayCtx { app, painter, mat_rect, paper_rect, sx, sy, tokens, enabled: ctx_in.enabled };
    let (ann_claimed_click, ann_opened_ctx_menu) = render_graph_embedded_objects(ui, &canvas_ov, gs, state, meta, intents);

    if ctx_in.enabled && !ann_opened_ctx_menu && canvas_resp.secondary_clicked() {
        let anchor = ui.input(|i| i.pointer.interact_pos()).unwrap_or(paper_rect.center());
        let sc_pt = screen_to_scene(anchor);
        let max_x = (scene.width_pt - 40.0).max(4.0);
        let max_y = (scene.height_pt - 40.0).max(4.0);
        state.graph_context_menu = Some((gs.id.clone(), None, anchor, [sc_pt.x.clamp(4.0, max_x), sc_pt.y.clamp(4.0, max_y)]));
    }

    // 1. Highlight any active brushed points (`UiApp::brushed_points`)
    for brushed in &app.brushed_points {
        for pt in &scene.plotted_points {
            if &pt.source_cell == brushed {
                let center = scene_to_screen(pt.center_pt);
                let r = ((pt.radius_pt as f32) * sx + 4.0).clamp(6.0, 16.0);
                painter.circle_stroke(center, r, Stroke::new(2.0, tokens.focus_ring));
            }
        }
    }

    // 2. Hover Point & Colorbar Value Readout + Click Point Brushing
    let pointer_pos = ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos())).filter(|p| paper_rect.contains(*p));
    let colorbar_opt = detect_colorbar_region(scene);

    if let Some(pos) = pointer_pos {
        let scene_pt = screen_to_scene(pos);
        if let Some(cb) = &colorbar_opt
            && rect_pt_contains(cb.rect_pt, scene_pt)
        {
            report.colorbar_hovered = true;
            let readout = sample_colorbar_readout(cb, scene_pt);
            state.hover_readout = Some(readout.clone());
            paint_hover_hud_card(painter, pos + vec2(12.0, -24.0), &readout.value_summary, tokens);
        } else if let Some((idx, hit)) = hit_test_point_indexed(scene, scene_pt, 10.0 / f64::from(sx.max(0.25))) {
            let ds_opt = doc.data_sheet(&hit.source_cell.sheet_id);
            let sheet_title = ds_opt.map_or(hit.source_cell.sheet_id.as_str(), |d| d.title.as_str());
            let is_spaghetti = gs.spec.series.iter().any(|s| {
                matches!(
                    s.gdo,
                    GraphicalDataObject::ScatterAndLine { replicate_lines: curvebook_doc::ReplicateLineMode::IndividualReplicateTrajectories, .. }
                )
            });
            let (label, summary) = if is_spaghetti {
                let row_lbl = ds_opt
                    .and_then(|d| hover_row_title(&d.archetype, hit.source_cell.row))
                    .filter(|t| !t.trim().is_empty())
                    .map_or_else(|| format!("Row {}", hit.source_cell.row + 1), str::to_owned);
                let rep_idx = hit.source_cell.subcol + 1;
                (
                    format!("{} (Rep {rep_idx})", hit.series_label),
                    format!("{sheet_title} · {} · Rep {rep_idx} · {row_lbl} · {}", hit.series_label, hit.value_summary),
                )
            } else {
                (hit.series_label.clone(), format!("{sheet_title} · {} · Row {} · {}", hit.series_label, hit.source_cell.row + 1, hit.value_summary))
            };
            let readout = CanvasHoverReadout {
                source_sheet_id: Some(hit.source_cell.sheet_id.clone()),
                point_index: Some(idx),
                row: Some(hit.source_cell.row),
                col: Some(hit.source_cell.col),
                label,
                value_summary: summary.clone(),
            };
            state.hover_readout = Some(readout);
            let center = scene_to_screen(hit.center_pt);
            painter.circle_stroke(center, 7.0, Stroke::new(2.0, tokens.accent));
            paint_hover_hud_card(painter, center + vec2(10.0, -22.0), &summary, tokens);

            if ctx_in.enabled && !ann_claimed_click && canvas_resp.clicked() {
                let coord = GridCellCoord::new(hit.source_cell.row, hit.source_cell.col, hit.source_cell.subcol);
                intents.push_brush_point(hit.source_cell.sheet_id.clone(), coord);
                intents.push_select_grid_cell(coord);
            }
        } else {
            let readout = CanvasHoverReadout {
                source_sheet_id: Some(gs.id.clone()),
                point_index: None,
                row: None,
                col: None,
                label: String::from("Cursor"),
                value_summary: format!("X = {:.1} pt, Y = {:.1} pt", scene_pt.x, scene_pt.y),
            };
            state.hover_readout = Some(readout);
        }
    }

    let ov = GraphOverlayCtx { painter, gs, doc, scene, paper_rect, tokens };

    // 3. Pairwise Significance Brackets & Compact Letter Display (CLD) HUD
    let has_pairwise_results = doc.result_sheets.iter().any(|r| !r.pairwise_comparisons.is_empty());
    if !gs.spec.comparisons.is_empty() || gs.spec.compact_letter_display.is_some() || has_pairwise_results {
        render_pairwise_cld_hud(ui, &ov, ctx_in.enabled, meta, intents);
    }

    // 4. Axis-Synchronized Kaplan-Meier `Number at risk` Alignment Track
    if should_show_km_track(gs, doc) {
        let track_rect = render_km_number_at_risk_track(ui, &ov, state, meta);
        report.km_track_rect_pt = track_rect;
    } else {
        state.km_at_risk_rows = 0;
    }

    // 5. Live TeX Curve/Fit Equations via `curvebook-text` layout helpers
    let eq_rects = render_live_tex_equations(ui, &ov, (sx, sy), state, meta);
    report.tex_equation_rects_pt = eq_rects;

    render_graph_context_menu(ui, &canvas_ov, gs, state, meta, intents);
}

fn hit_test_point_indexed(scene: &ChartScene, pt: PointPt, tol_pt: f64) -> Option<(usize, &curvebook_chart::PlottedPointMeta)> {
    let mut best: Option<(usize, f64, &curvebook_chart::PlottedPointMeta)> = None;
    for (idx, meta) in scene.plotted_points.iter().enumerate() {
        let d = (meta.center_pt.x - pt.x).hypot(meta.center_pt.y - pt.y);
        let max_r = meta.radius_pt.max(tol_pt);
        if d <= max_r && best.is_none_or(|(_, bd, _)| d < bd) {
            best = Some((idx, d, meta));
        }
    }
    best.map(|(i, _, m)| (i, m))
}

fn hover_row_title(arch: &curvebook_doc::TableArchetype, row: usize) -> Option<&str> {
    match arch {
        curvebook_doc::TableArchetype::Xy(t) => t.row_titles.get(row).map(String::as_str),
        curvebook_doc::TableArchetype::Column(t) => t.row_titles.get(row).map(String::as_str),
        curvebook_doc::TableArchetype::Grouped(t) => t.row_titles.get(row).map(String::as_str),
        curvebook_doc::TableArchetype::Contingency(t) => t.row_titles.get(row).map(String::as_str),
        curvebook_doc::TableArchetype::Survival(t) => t.row_titles.get(row).map(String::as_str),
        curvebook_doc::TableArchetype::PartsOfWhole(t) => t.slice_labels.get(row).map(String::as_str),
        curvebook_doc::TableArchetype::MultipleVariables(t) => t.row_titles.get(row).map(String::as_str),
        curvebook_doc::TableArchetype::Nested(t) => t.row_titles.get(row).map(String::as_str),
    }
}

fn rect_pt_contains(r: RectPt, pt: PointPt) -> bool {
    pt.x >= r.x && pt.x <= r.right() && pt.y >= r.y && pt.y <= r.bottom()
}

fn detect_colorbar_region(scene: &ChartScene) -> Option<ColorbarRegion> {
    let mut bar_rects: Vec<(RectPt, Rgba8)> = Vec::new();
    for prim in &scene.primitives {
        let ScenePrimitive::Path { path, fill: Some(fill), stroke: None, .. } = prim else {
            continue;
        };
        if let Some(r) = extract_axis_aligned_rect(path)
            && r.x >= scene.plot_rect_pt.right() - 1.0
            && (6.0..=28.0).contains(&r.width)
            && r.height > 0.5
            && r.height <= 30.0
        {
            bar_rects.push((r, fill.color));
        }
    }
    if bar_rects.len() < 8 {
        return None;
    }
    let min_x = bar_rects.iter().map(|(r, _)| r.x).fold(f64::INFINITY, f64::min);
    let max_r = bar_rects.iter().map(|(r, _)| r.right()).fold(f64::NEG_INFINITY, f64::max);
    let min_y = bar_rects.iter().map(|(r, _)| r.y).fold(f64::INFINITY, f64::min);
    let max_b = bar_rects.iter().map(|(r, _)| r.bottom()).fold(f64::NEG_INFINITY, f64::max);
    let bar_rect = RectPt::new(min_x, min_y, (max_r - min_x).max(1.0), (max_b - min_y).max(1.0));

    let mut nums = Vec::new();
    for prim in &scene.primitives {
        if let ScenePrimitive::Text(t) = prim
            && t.anchor_pt.x >= bar_rect.right() - 2.0
            && t.anchor_pt.y >= bar_rect.y - 8.0
            && t.anchor_pt.y <= bar_rect.bottom() + 8.0
            && let Ok(v) = t.rich.plain_text().trim().parse::<f64>()
            && v.is_finite()
        {
            nums.push((t.anchor_pt.y, v));
        }
    }
    let (min_val, max_val) = if nums.len() >= 2 {
        let lo = nums.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
        let hi = nums.iter().map(|(_, v)| *v).fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    } else {
        (0.0, 1.0)
    };

    Some(ColorbarRegion { rect_pt: bar_rect, steps: bar_rects, min_val, max_val })
}

fn extract_axis_aligned_rect(path: &curvebook_chart::Path2D) -> Option<RectPt> {
    if path.commands.len() < 4 {
        return None;
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for cmd in &path.commands {
        match *cmd {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => {
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
            PathCmd::Close => {}
            _ => return None,
        }
    }
    (min_x.is_finite() && min_y.is_finite() && max_x > min_x && max_y > min_y).then(|| RectPt::new(min_x, min_y, max_x - min_x, max_y - min_y))
}

fn sample_colorbar_readout(cb: &ColorbarRegion, scene_pt: PointPt) -> CanvasHoverReadout {
    let t = (1.0 - (scene_pt.y - cb.rect_pt.y) / cb.rect_pt.height.max(1e-6)).clamp(0.0, 1.0);
    let val = cb.min_val + t * (cb.max_val - cb.min_val);
    let color =
        cb.steps.iter().find(|(r, _)| scene_pt.y >= r.y && scene_pt.y <= r.bottom()).or_else(|| cb.steps.first()).map_or(Rgba8::BLACK, |(_, c)| *c);
    let summary = format!("Colorbar: {val:.3} (#{:02X}{:02X}{:02X})", color.r, color.g, color.b);
    CanvasHoverReadout { source_sheet_id: None, point_index: None, row: None, col: None, label: String::from("Colorbar"), value_summary: summary }
}

fn paint_hover_hud_card(painter: &egui::Painter, anchor: Pos2, text: &str, tokens: &Tokens) {
    let font = FontId::proportional(11.5);
    let galley = painter.layout_no_wrap(text.to_owned(), font, tokens.text_strong);
    let card_rect = Rect::from_min_size(anchor, galley.size() + vec2(12.0, 8.0));
    painter.rect_filled(card_rect, CornerRadius::same(4), tokens.panel_bg);
    painter.rect_stroke(card_rect, CornerRadius::same(4), Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
    painter.galley(card_rect.min + vec2(6.0, 4.0), galley, tokens.text_strong);
}

fn render_pairwise_cld_hud(ui: &mut egui::Ui, ov: &GraphOverlayCtx<'_>, enabled: bool, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let GraphOverlayCtx { gs, doc, paper_rect, tokens, .. } = *ov;
    let mode_lbl = match (gs.spec.comparisons.is_empty(), gs.spec.compact_letter_display.is_some()) {
        (false, true) => format!("Brackets ({}) + CLD", gs.spec.comparisons.len()),
        (true, true) => String::from("CLD Active"),
        (false, false) => format!("Brackets ({})", gs.spec.comparisons.len()),
        (true, false) => String::from("Pairwise / CLD"),
    };
    let hud_w = (paper_rect.width() - 12.0).clamp(160.0, 268.0);
    let hud_rect =
        Rect::from_min_size(pos2((paper_rect.right() - hud_w - 6.0).max(paper_rect.left() + 6.0), paper_rect.top() + 6.0), vec2(hud_w, 26.0));
    if hud_rect.width() < 80.0 {
        return;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(hud_rect), |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let btn = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(&mode_lbl).small()));
            meta.record_response(
                ui,
                &btn,
                FrameInspectEntryInput {
                    widget_id: "canvas.pairwise_hud",
                    role: "button",
                    label: &mode_lbl,
                    command_id: Some("ui.graph.pairwise.open"),
                    value: None,
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::FloatingOverlay,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if btn.clicked() && enabled {
                intents.push_ui("ui.graph.pairwise.open", json!({ "sheet_id": gs.id.as_str() }));
            }

            if gs.spec.comparisons.len() > 6 && gs.spec.compact_letter_display.is_none() {
                let cld_btn = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new("Switch to CLD").small().color(tokens.accent)));
                meta.record_response(
                    ui,
                    &cld_btn,
                    FrameInspectEntryInput {
                        widget_id: "canvas.pairwise_cld_recommend",
                        role: "button",
                        label: "Switch to CLD",
                        command_id: Some("graph.setCompactLetterDisplay"),
                        value: None,
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::FloatingOverlay,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if cld_btn.clicked()
                    && enabled
                    && let Some(rs_id) = doc.result_sheets.iter().find(|r| !r.pairwise_comparisons.is_empty()).map(|r| &r.id)
                {
                    intents.push_engine(
                        "graph.setCompactLetterDisplay",
                        json!({ "sheet_id": gs.id.as_str(), "result_sheet_id": rs_id.as_str(), "enabled": true }),
                    );
                }
            }
        });
    });
}

fn should_show_km_track(gs: &GraphSheet, doc: &ProjectDocument) -> bool {
    gs.spec.kind == GraphKind::Survival
        && (gs.spec.survival_number_at_risk
            || doc.result_sheets.iter().any(|r| {
                r.analysis.source_sheet_id == gs.spec.primary_source_sheet_id
                    && matches!(r.analysis.kind, AnalysisKind::Survival(_))
                    && r.tabs.iter().any(|t| t.kind == ResultTabKind::NumberAtRisk)
            }))
}

#[derive(Debug, Clone)]
struct KmCohortRiskRow {
    label: String,
    color: Color32,
    counts: Vec<usize>,
}

fn render_km_number_at_risk_track(
    ui: &mut egui::Ui,
    ov: &GraphOverlayCtx<'_>,
    state: &mut CanvasViewEguiState,
    meta: &mut FrameInspectMeta,
) -> Option<Rect> {
    let GraphOverlayCtx { painter, gs, doc, scene, paper_rect, tokens } = *ov;
    let ticks = extract_x_axis_ticks_pt(gs, scene);
    if ticks.is_empty() {
        state.km_at_risk_rows = 0;
        return None;
    }
    let rows = build_km_risk_rows(gs, doc, &ticks, tokens);
    state.km_at_risk_rows = rows.len();
    if rows.is_empty() {
        return None;
    }

    let row_h = 15.0_f32;
    let track_h = 20.0 + (rows.len() as f32) * row_h;
    let track_rect = Rect::from_min_size(pos2(paper_rect.left(), paper_rect.bottom() + 4.0), vec2(paper_rect.width(), track_h));
    painter.rect_filled(track_rect, CornerRadius::same(2), tokens.panel_bg);
    painter.rect_stroke(track_rect, CornerRadius::same(2), Stroke::new(1.0, tokens.divider), StrokeKind::Inside);

    let sx = paper_rect.width() / (scene.width_pt.max(1.0) as f32);
    let label_right = paper_rect.left() + (scene.plot_rect_pt.x as f32) * sx - 6.0;
    painter.text(
        pos2(label_right.max(track_rect.left() + 82.0), track_rect.top() + 10.0),
        egui::Align2::RIGHT_CENTER,
        "Number at risk",
        FontId::proportional(tokens.metrics.font_ui_small_pt),
        tokens.text_strong,
    );

    // Paint tick headers synchronized directly to X-axis tick screen X coordinates
    for &(t_val, x_pt) in &ticks {
        let tx = paper_rect.left() + (x_pt as f32) * sx;
        painter.text(
            pos2(tx, track_rect.top() + 10.0),
            egui::Align2::CENTER_CENTER,
            format!("{t_val:.4}").trim_end_matches('0').trim_end_matches('.'),
            FontId::monospace(tokens.metrics.font_ui_small_pt),
            tokens.text_muted,
        );
    }

    for (r_idx, row) in rows.iter().enumerate() {
        let ry = track_rect.top() + 24.0 + (r_idx as f32) * row_h;
        painter.text(
            pos2(label_right.max(track_rect.left() + 82.0), ry),
            egui::Align2::RIGHT_CENTER,
            &row.label,
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            row.color,
        );
        for (&(_, x_pt), &count) in ticks.iter().zip(row.counts.iter()) {
            let tx = paper_rect.left() + (x_pt as f32) * sx;
            painter.text(
                pos2(tx, ry),
                egui::Align2::CENTER_CENTER,
                format!("{count}"),
                FontId::monospace(tokens.metrics.font_ui_small_pt),
                tokens.text_primary,
            );
        }
    }

    let track_vis = track_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
    if track_vis.width() >= 2.0 && track_vis.height() >= 2.0 {
        let track_resp = ui.allocate_rect(track_vis, Sense::hover());
        track_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, String::from("Kaplan-Meier Number at Risk")));
        meta.record_response(
            ui,
            &track_resp,
            FrameInspectEntryInput {
                widget_id: "canvas.km_number_at_risk",
                role: "label",
                label: "Kaplan-Meier Number at Risk",
                command_id: None,
                value: Some(json!({ "cohorts": rows.len(), "ticks": ticks.len() })),
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::FloatingOverlay,
                ellipsized: false,
                full_label: None,
            },
        );
    }

    Some(track_rect)
}

fn extract_x_axis_ticks_pt(gs: &GraphSheet, scene: &ChartScene) -> Vec<(f64, f64)> {
    let plot_r = scene.plot_rect_pt;
    let mut ticks = Vec::new();
    for prim in &scene.primitives {
        if let ScenePrimitive::Text(t) = prim
            && (t.anchor_pt.y - (plot_r.bottom() + 10.0)).abs() <= 14.0
            && t.anchor_pt.x >= plot_r.x - 4.0
            && t.anchor_pt.x <= plot_r.right() + 4.0
            && let Ok(val) = t.rich.plain_text().trim().parse::<f64>()
            && val.is_finite()
        {
            ticks.push((val, t.anchor_pt.x));
        }
    }
    ticks.sort_by(|a, b| a.1.total_cmp(&b.1));
    ticks.dedup_by(|a, b| (a.1 - b.1).abs() < 2.0);
    if !ticks.is_empty() {
        return ticks;
    }

    let first_seg = gs.spec.x_axis.segments.first();
    let lo = first_seg.and_then(|s| s.min).unwrap_or(0.0);
    let hi = first_seg.and_then(|s| s.max).unwrap_or(10.0).max(lo + 1.0);
    let step = first_seg.and_then(|s| s.major_tick_interval).filter(|s| s.is_finite() && *s > 0.0).unwrap_or((hi - lo) / 4.0);
    let mut out = Vec::new();
    let mut cur = lo;
    let mut guard = 0usize;
    while cur <= hi + 1e-9 && guard < 12 {
        let frac = (cur - lo) / (hi - lo);
        out.push((cur, plot_r.x + frac * plot_r.width));
        cur += step;
        guard += 1;
    }
    out
}

fn gdo_primary_color(gdo: &GraphicalDataObject) -> Option<Rgba8> {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill, .. } => {
            marker.as_ref().map(|m| m.fill_color).or_else(|| line.as_ref().map(|l| l.color)).or_else(|| area_fill.as_ref().map(|a| a.fill_color))
        }
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => Some(*fill_color),
        GraphicalDataObject::AlignedDotPlot { marker, .. } => Some(marker.fill_color),
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => Some(*line_color),
        GraphicalDataObject::PartsOfWhole { .. } => None,
    }
}

fn build_km_risk_rows(gs: &GraphSheet, doc: &ProjectDocument, ticks: &[(f64, f64)], tokens: &Tokens) -> Vec<KmCohortRiskRow> {
    let mut out = Vec::new();
    let palette = crate::egui_shell::OKABE_ITO_SERIES_PALETTE;
    let ds_opt = doc.data_sheet(&gs.spec.primary_source_sheet_id);
    let Some(ds) = ds_opt else {
        return out;
    };
    if let curvebook_doc::TableArchetype::Survival(surv) = &ds.archetype {
        for (c_idx, cohort) in surv.cohorts.iter().enumerate() {
            let times: Vec<f64> = match surv.cohort_observations(c_idx) {
                Ok(obs) => obs.into_iter().map(|o| o.time).filter(|v| v.is_finite()).collect(),
                Err(_) => (0..surv.row_count()).filter_map(|row| surv.times.get(row).and_then(|c| c.value.finite_f64())).collect(),
            };
            let counts = ticks.iter().map(|&(t_val, _)| times.iter().filter(|&&t| t >= t_val - 1e-9).count()).collect();
            let col = gs
                .spec
                .series
                .get(c_idx)
                .and_then(|s| gdo_primary_color(&s.gdo))
                .map(rgba8_to_color32)
                .unwrap_or_else(|| palette[c_idx % palette.len()]);
            let label = if cohort.title.trim().is_empty() { format!("Cohort {}", c_idx + 1) } else { cohort.title.clone() };
            out.push(KmCohortRiskRow { label, color: col, counts });
        }
    }
    let _ = tokens;
    out
}

fn render_live_tex_equations(
    ui: &mut egui::Ui,
    ov: &GraphOverlayCtx<'_>,
    scale_xy: (f32, f32),
    state: &mut CanvasViewEguiState,
    meta: &mut FrameInspectMeta,
) -> Vec<Rect> {
    let GraphOverlayCtx { gs, doc, scene, paper_rect, .. } = *ov;
    let (sx, sy) = scale_xy;
    let mut rects = Vec::new();
    let mut eq_idx = 0usize;

    for ann in &gs.spec.annotations {
        let GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered } = ann else {
            continue;
        };
        let Some(tex) = resolve_live_equation_tex(doc, result_sheet_id, *data_set_index, *mode) else {
            continue;
        };
        if tex.trim().is_empty() {
            continue;
        }
        let eff_bounds = resolve_model_equation_hit_bounds(scene, *bounds_pt, *font_size_pt, *bordered).unwrap_or_else(|| RectPt::from(*bounds_pt));
        let box_rect = Rect::from_min_size(
            pos2(paper_rect.left() + (eff_bounds.x as f32) * sx, paper_rect.top() + (eff_bounds.y as f32) * sy),
            vec2((eff_bounds.width as f32 * sx).max(16.0), (eff_bounds.height as f32 * sy).max(12.0)),
        );
        let wid = format!("canvas.tex_equation.{eq_idx}");
        let box_vis = box_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
        if box_vis.width() >= 2.0 && box_vis.height() >= 2.0 {
            let resp = ui.allocate_rect(box_vis, Sense::hover());
            let ak_tex = tex.clone();
            resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, true, ak_tex.clone()));
            meta.record_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "label",
                    label: &tex,
                    command_id: None,
                    value: Some(json!({ "tex": tex })),
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::FloatingOverlay,
                    ellipsized: false,
                    full_label: None,
                },
            );
        }
        rects.push(box_rect);
        eq_idx += 1;
    }

    state.tex_equation_count = eq_idx;
    rects
}

fn resolve_live_equation_tex(doc: &ProjectDocument, result_sheet_id: &SheetId, data_set_index: usize, mode: EquationDisplayMode) -> Option<String> {
    let rs = doc.result_sheet(result_sheet_id)?;
    let substituted = matches!(mode, EquationDisplayMode::Substituted);
    let params: Vec<(String, f64)> = rs
        .scalars
        .iter()
        .filter(|s| s.data_set_index == Some(data_set_index) || s.data_set_index.is_none())
        .map(|s| (s.key.clone(), s.value))
        .collect();

    match &rs.analysis.kind {
        AnalysisKind::NonlinearRegression(nl) => {
            let eq_id =
                rs.fitted_curves.iter().find(|c| c.data_set_index == data_set_index).map_or(nl.equation_id.as_str(), |c| c.equation_id.as_str());
            if eq_id.trim().is_empty() {
                return None;
            }
            Some(format_model_equation_tex(eq_id, nl.custom_dsl.as_deref(), &params, substituted, 4))
        }
        AnalysisKind::LinearRegression(lin) => {
            let through_origin = lin.force_through_origin && !lin.deming_model_ii;
            Some(format_polynomial_equation_tex(1, through_origin, &params, substituted, 4))
        }
        _ => rs
            .fitted_curves
            .iter()
            .find(|c| c.data_set_index == data_set_index)
            .map(|c| format_model_equation_tex(&c.equation_id, None, &params, substituted, 4)),
    }
}

struct LayoutHudInput<'a> {
    painter: &'a egui::Painter,
    app: &'a UiApp,
    ls: &'a LayoutSheet,
    scene: Option<&'a ChartScene>,
    mat_rect: Rect,
    paper_rect: Rect,
    eff_scale: f32,
    canvas_resp: &'a egui::Response,
    ctx_in: &'a CanvasViewContext<'a>,
}

fn collect_layout_snap_plot_origins(ctx: &egui::Context, app: &UiApp, ls: &LayoutSheet, state: &CanvasViewEguiState) -> Vec<(usize, PointPt)> {
    if state.layout_drag.is_none() || !super::trailing_studio::studio_snap_enabled(ctx) {
        return Vec::new();
    }
    ls.spec
        .items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            if let LayoutItem::LiveGraph { graph_sheet_id, .. } = item {
                let gs_scene = resolve_cached_scene(ctx, app, graph_sheet_id, "layout_hud")?;
                Some((idx, PointPt::new(gs_scene.plot_rect_pt.x, gs_scene.plot_rect_pt.y)))
            } else {
                None
            }
        })
        .collect()
}

fn render_layout_hud_overlays(
    ui: &mut egui::Ui,
    input: LayoutHudInput<'_>,
    state: &mut CanvasViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CanvasViewFrameReport,
) {
    let LayoutHudInput { painter, app, ls, scene, mat_rect, paper_rect, eff_scale, canvas_resp, ctx_in } = input;
    let doc = app.document();
    let tokens = ctx_in.tokens;
    let scene_w = scene.map_or(ls.spec.page_size_pt().width.max(1.0), |s| s.width_pt.max(1.0)) as f32;
    let scene_h = scene.map_or(ls.spec.page_size_pt().height.max(1.0), |s| s.height_pt.max(1.0)) as f32;
    let sx = paper_rect.width() / scene_w;
    let sy = paper_rect.height() / scene_h;
    let _ = eff_scale;
    let ov = CanvasOverlayCtx { app, painter, mat_rect, paper_rect, sx, sy, tokens, enabled: ctx_in.enabled };

    let selected_img = state.selected_panel_index.and_then(|sel_idx| match ls.spec.items.get(sel_idx) {
        Some(LayoutItem::EmbeddedImage { bounds_pt, rotation_deg, lock_aspect_ratio, .. }) => {
            Some((sel_idx, *bounds_pt, *rotation_deg, *lock_aspect_ratio))
        }
        _ => None,
    });
    let ptr_pos = ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos()));
    let resize_or_rot_active = state.image_gesture.as_ref().is_some_and(|g| g.is_layout && g.kind != ImageGestureKind::Move);
    let pointer_in_img_handle = selected_img.is_some_and(|(sel_idx, init_bounds, _, _)| {
        let disp = active_gesture_bounds(state, &ls.id, true, sel_idx, init_bounds);
        let sel_r = doc_rect_to_screen(paper_rect, sx, sy, disp);
        let h_rects = image_handle_rects(sel_r, mat_rect);
        ptr_pos.is_some_and(|p| h_rects.iter().any(|hr| hr.contains(p)))
    });
    let (selected_non_img_idx, pointer_in_panel_resize, panel_resize_active) =
        selected_non_image_panel_state(ls, state, paper_rect, sx, sy, ptr_pos, selected_img.is_some());

    let mut panel_rects = Vec::with_capacity(ls.spec.items.len());
    for (idx, item) in ls.spec.items.iter().enumerate() {
        let mut bounds = layout_item_bounds_doc(item);
        if let Some(drag) = &state.layout_drag
            && drag.sheet_id == ls.id
            && drag.item_index == idx
        {
            bounds = drag.current_bounds_pt;
        } else {
            bounds = active_gesture_bounds(state, &ls.id, true, idx, bounds);
        }
        let r = doc_rect_to_screen(paper_rect, sx, sy, bounds);
        panel_rects.push((idx, item, bounds, r));
    }

    let plot_origins = collect_layout_snap_plot_origins(ui.ctx(), app, ls, state);

    let mut panel_opened_ctx_menu = false;
    for (idx, item, bounds, r) in panel_rects {
        let selected = state.selected_panel_index == Some(idx);
        let suppress_drag = selected && (pointer_in_img_handle || resize_or_rot_active || pointer_in_panel_resize || panel_resize_active);
        let sense = if ctx_in.enabled && !suppress_drag { Sense::click_and_drag() } else { Sense::hover() };
        let p_resp = ui.interact(r, Id::unique(("layout_panel", ls.id.as_str(), idx)), sense);

        let stroke_col = if selected {
            tokens.accent
        } else if p_resp.hovered() {
            tokens.focus_ring
        } else {
            Color32::TRANSPARENT
        };
        if stroke_col != Color32::TRANSPARENT {
            painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(if selected { 2.0 } else { 1.0 }, stroke_col), StrokeKind::Inside);
        }

        let badge = ls.spec.panel_badges.badge_for(idx).unwrap_or_else(|| format!("P{}", idx + 1));
        if selected || p_resp.hovered() {
            let chip_rect = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(30.0, 20.0));
            painter.rect_filled(chip_rect, CornerRadius::same(tokens.metrics.radius_sm), tokens.accent_strong);
            painter.text(
                chip_rect.center(),
                egui::Align2::CENTER_CENTER,
                &badge,
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                tokens.on_accent,
            );
        }

        let wid = format!("canvas.layout_panel.{idx}");
        let panel_label = format!("Layout Panel {badge}");
        let panel_ak = panel_label.clone();
        let panel_enabled = ctx_in.enabled;
        p_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, panel_enabled, panel_ak.clone()));
        meta.record_response(
            ui,
            &p_resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "button",
                label: &panel_label,
                command_id: None,
                value: Some(json!({ "selected": selected, "x": bounds.x, "y": bounds.y })),
                enabled: ctx_in.enabled,
                disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                z_layer: ZLayer::FloatingOverlay,
                ellipsized: false,
                full_label: None,
            },
        );

        if ctx_in.enabled {
            if p_resp.double_clicked() || p_resp.triple_clicked() {
                match item {
                    LayoutItem::LiveGraph { graph_sheet_id, .. } if doc.graph_sheet(graph_sheet_id).is_some() => {
                        state.slot.layout_return_breadcrumb = Some(ls.id.clone());
                        intents.push_engine("sheet.select", json!({ "sheet_id": graph_sheet_id.as_str() }));
                        return;
                    }
                    LayoutItem::LiveTable { .. } => {
                        intents.push_ui("ui.embeddedTable.format.open", json!({ "sheet_id": ls.id.as_str(), "panel_index": idx }));
                        return;
                    }
                    LayoutItem::EmbeddedImage { .. } => {
                        intents.push_ui("ui.image.format.open", json!({ "sheet_id": ls.id.as_str(), "panel_index": idx }));
                        return;
                    }
                    _ => {}
                }
            }
            if p_resp.clicked() {
                state.selected_panel_index = Some(idx);
            }
            if p_resp.secondary_clicked() {
                let anchor = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.center());
                state.selected_panel_index = Some(idx);
                state.layout_context_menu = Some((ls.id.clone(), Some(idx), anchor));
                panel_opened_ctx_menu = true;
            }
            if !suppress_drag && p_resp.drag_started() {
                state.selected_panel_index = Some(idx);
                let ptr = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.center());
                let scene_pt =
                    PointPt::new(f64::from((ptr.x - paper_rect.left()) / sx.max(1e-4)), f64::from((ptr.y - paper_rect.top()) / sy.max(1e-4)));
                state.layout_drag = Some(LayoutPanelDragState {
                    sheet_id: ls.id.clone(),
                    item_index: idx,
                    start_scene_pt: [scene_pt.x, scene_pt.y],
                    initial_bounds_pt: bounds,
                    current_bounds_pt: bounds,
                    resizing_corner: false,
                });
            }
            if p_resp.dragged()
                && let Some(drag) = state.layout_drag.as_mut()
                && drag.item_index == idx
                && !drag.resizing_corner
            {
                let ptr = ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos())).unwrap_or(r.center());
                let scene_pt =
                    PointPt::new(f64::from((ptr.x - paper_rect.left()) / sx.max(1e-4)), f64::from((ptr.y - paper_rect.top()) / sy.max(1e-4)));
                let dx = scene_pt.x - drag.start_scene_pt[0];
                let dy = scene_pt.y - drag.start_scene_pt[1];
                let raw_x = round_tenth_pt(drag.initial_bounds_pt.x + dx);
                let raw_y = round_tenth_pt(drag.initial_bounds_pt.y + dy);
                let cand = curvebook_chart::RectPt::new(raw_x, raw_y, drag.initial_bounds_pt.width, drag.initial_bounds_pt.height);
                let (snapped, guides) = if super::trailing_studio::studio_snap_enabled(ui.ctx()) {
                    snap_panel_with_tenth_precision(app, ls, idx, cand, &plot_origins)
                } else {
                    (cand, Vec::new())
                };
                drag.current_bounds_pt = DocRectPt::new(snapped.x, snapped.y, snapped.width, snapped.height);
                state.active_snap_guides = guides;
                state.last_snap_readout = if super::trailing_studio::studio_snap_enabled(ui.ctx()) {
                    Some(format!("Snap: X={:.1} pt, Y={:.1} pt (0.1 pt)", snapped.x, snapped.y))
                } else {
                    None
                };
            }
            if p_resp.drag_stopped()
                && state.layout_drag.as_ref().is_some_and(|d| d.item_index == idx && !d.resizing_corner)
                && let Some(drag) = state.layout_drag.take()
            {
                if drag.current_bounds_pt != drag.initial_bounds_pt {
                    let mut updated_items = ls.spec.items.clone();
                    if let Some(target_item) = updated_items.get_mut(idx) {
                        set_layout_item_bounds_doc(target_item, drag.current_bounds_pt);
                        intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "spec": { "items": updated_items } }));
                    }
                }
                state.active_snap_guides.clear();
            }
        }
    }

    if ctx_in.enabled && !panel_opened_ctx_menu && canvas_resp.secondary_clicked() {
        let anchor = ui.input(|i| i.pointer.interact_pos()).unwrap_or(paper_rect.center());
        state.layout_context_menu = Some((ls.id.clone(), None, anchor));
    }

    if let Some((sel_idx, init_bounds, rot_deg, lock_aspect)) = selected_img {
        render_selected_image_handles(
            ui,
            &ov,
            SelectedImageTarget { sheet_id: &ls.id, is_layout: true, target_idx: sel_idx, init_bounds, init_rot_deg: rot_deg, lock_aspect },
            state,
            meta,
            intents,
        );
    } else if ctx_in.enabled
        && let Some(sel_idx) = selected_non_img_idx
    {
        render_layout_panel_resize_handle(ui, &ov, ls, state, sel_idx, meta, intents);
    }

    // Draw active 0.1 pt snap guides (both internal live drag & CanvasToolState active_snap_guides)
    let guides_to_draw = if !state.active_snap_guides.is_empty() { &state.active_snap_guides } else { &app.canvas_dialogs.canvas.active_snap_guides };
    report.snap_guide_count = guides_to_draw.len();
    for guide in guides_to_draw {
        let col = if guide.source == SnapSource::PlotOrigin { tokens.error } else { tokens.accent };
        match guide.orientation {
            SnapOrientation::Vertical => {
                let x = paper_rect.left() + (guide.position_pt as f32) * sx;
                let y0 = paper_rect.top() + (guide.span_start_pt as f32) * sy;
                let y1 = paper_rect.top() + (guide.span_end_pt as f32) * sy;
                painter.line_segment([pos2(x, y0), pos2(x, y1)], Stroke::new(1.5, col));
            }
            SnapOrientation::Horizontal => {
                let y = paper_rect.top() + (guide.position_pt as f32) * sy;
                let x0 = paper_rect.left() + (guide.span_start_pt as f32) * sx;
                let x1 = paper_rect.left() + (guide.span_end_pt as f32) * sx;
                painter.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(1.5, col));
            }
        }
    }
    if !guides_to_draw.is_empty()
        && let Some(readout) = &state.last_snap_readout
    {
        paint_hover_hud_card(painter, pos2(paper_rect.left() + 8.0, paper_rect.top() + 8.0), readout, tokens);
    }

    render_layout_context_menu_popup(ui, &ov, ls, state, meta, intents);
}

fn snap_panel_with_tenth_precision(
    app: &UiApp,
    ls: &LayoutSheet,
    moving_index: usize,
    candidate: RectPt,
    plot_origins: &[(usize, PointPt)],
) -> (RectPt, Vec<SnapGuide>) {
    let sibling_origins: Vec<PointPt> = plot_origins
        .iter()
        .filter(|(idx, _)| *idx != moving_index)
        .filter_map(|(idx, off)| {
            ls.spec.items.get(*idx).map(|it| {
                let b = layout_item_bounds_doc(it);
                PointPt::new(b.x + off.x, b.y + off.y)
            })
        })
        .collect();
    let snap = app.canvas_dialogs.canvas.compute_layout_snap(&ls.spec, Some(moving_index), candidate, &sibling_origins);
    let mut snapped = snap.snapped_bounds_pt;
    let mut guides = snap.guides;

    // 0.1 pt plot-axis frame alignment against sibling plot frames
    if let Some((_, own_off)) = plot_origins.iter().find(|(idx, _)| *idx == moving_index) {
        let tol = app.canvas_dialogs.canvas.snap_tolerance_pt.max(2.0);
        let page = ls.spec.page_size_pt();
        for origin in &sibling_origins {
            if ((snapped.x + own_off.x) - origin.x).abs() <= tol {
                snapped.x = round_tenth_pt(origin.x - own_off.x);
                guides.push(SnapGuide {
                    orientation: SnapOrientation::Vertical,
                    position_pt: round_tenth_pt(origin.x),
                    span_start_pt: 0.0,
                    span_end_pt: page.height,
                    source: SnapSource::PlotOrigin,
                });
            }
            if ((snapped.y + own_off.y) - origin.y).abs() <= tol {
                snapped.y = round_tenth_pt(origin.y - own_off.y);
                guides.push(SnapGuide {
                    orientation: SnapOrientation::Horizontal,
                    position_pt: round_tenth_pt(origin.y),
                    span_start_pt: 0.0,
                    span_end_pt: page.width,
                    source: SnapSource::PlotOrigin,
                });
            }
        }
    }

    snapped.x = round_tenth_pt(snapped.x);
    snapped.y = round_tenth_pt(snapped.y);
    (snapped, guides)
}

fn round_tenth_pt(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

const fn layout_item_bounds_doc(item: &LayoutItem) -> DocRectPt {
    match item {
        LayoutItem::LiveGraph { bounds_pt, .. }
        | LayoutItem::LiveTable { bounds_pt, .. }
        | LayoutItem::TextCallout { bounds_pt, .. }
        | LayoutItem::EmbeddedImage { bounds_pt, .. } => *bounds_pt,
    }
}

fn set_layout_item_bounds_doc(item: &mut LayoutItem, bounds: DocRectPt) {
    match item {
        LayoutItem::LiveGraph { bounds_pt, .. }
        | LayoutItem::LiveTable { bounds_pt, .. }
        | LayoutItem::TextCallout { bounds_pt, .. }
        | LayoutItem::EmbeddedImage { bounds_pt, .. } => {
            *bounds_pt = bounds;
        }
    }
}

struct CanvasRulersSpec<'a> {
    mat_rect: Rect,
    top_r: Rect,
    left_r: Rect,
    paper_rect: Rect,
    eff_scale: f32,
    unit: PhysicalUnit,
    tokens: &'a Tokens,
}

fn render_canvas_rulers(ui: &egui::Ui, spec: CanvasRulersSpec<'_>) {
    let CanvasRulersSpec { mat_rect, top_r, left_r, paper_rect, eff_scale, unit, tokens } = spec;
    let outer_clip = ui.painter().clip_rect();
    let corner = Rect::from_min_size(pos2(left_r.left(), top_r.top()), vec2(RULER_THICKNESS_PT, RULER_THICKNESS_PT));
    let corner_clip = outer_clip.intersect(corner);
    let top_clip = outer_clip.intersect(top_r);
    let left_clip = outer_clip.intersect(left_r);
    let corner_painter = ui.painter().with_clip_rect(corner_clip);
    let top_painter = ui.painter().with_clip_rect(top_clip);
    let left_painter = ui.painter().with_clip_rect(left_clip);
    let unit_lbl = match unit {
        PhysicalUnit::Points => "pt",
        PhysicalUnit::Inches => "in",
        PhysicalUnit::Millimeters => "mm",
    };
    corner_painter.rect_filled(corner, CornerRadius::ZERO, tokens.header_bg);
    top_painter.rect_filled(top_r, CornerRadius::ZERO, tokens.header_bg);
    left_painter.rect_filled(left_r, CornerRadius::ZERO, tokens.header_bg);
    let unit_galley = corner_painter.layout_no_wrap(unit_lbl.to_owned(), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    let unit_pos = corner.center() - unit_galley.size() * 0.5;
    if unit_pos.x >= corner_clip.left()
        && unit_pos.y >= corner_clip.top()
        && unit_pos.x + unit_galley.size().x <= corner_clip.right()
        && unit_pos.y + unit_galley.size().y <= corner_clip.bottom()
    {
        corner_painter.galley(unit_pos, unit_galley, tokens.text_muted);
    }

    let pt_per_unit = match unit {
        PhysicalUnit::Points => 72.0_f32,
        PhysicalUnit::Inches => 72.0_f32,
        PhysicalUnit::Millimeters => 72.0_f32 / 25.4_f32 * 10.0_f32,
    };
    let step_px = (pt_per_unit * eff_scale).max(28.0);
    let mut x = paper_rect.left();
    let mut tick_idx = 0i32;
    while x <= mat_rect.right() && tick_idx < 32 {
        if x >= top_r.left() {
            top_painter.line_segment([pos2(x, top_r.bottom() - 6.0), pos2(x, top_r.bottom())], Stroke::new(1.0, tokens.border));
            let val = match unit {
                PhysicalUnit::Points => tick_idx * 72,
                PhysicalUnit::Inches => tick_idx,
                PhysicalUnit::Millimeters => tick_idx * 10,
            };
            let galley = top_painter.layout_no_wrap(format!("{val}"), FontId::monospace(9.0), tokens.text_muted);
            let label_pos = pos2(x + 2.0, top_r.top() + 3.0);
            if label_pos.x >= top_clip.left()
                && label_pos.y >= top_clip.top()
                && label_pos.x + galley.size().x <= top_clip.right() - 1.0
                && label_pos.y + galley.size().y <= top_clip.bottom()
            {
                top_painter.galley(label_pos, galley, tokens.text_muted);
            }
        }
        x += step_px;
        tick_idx += 1;
    }

    let mut y = paper_rect.top();
    let mut y_idx = 0i32;
    while y <= mat_rect.bottom() && y_idx < 32 {
        if y >= left_r.top() {
            left_painter.line_segment([pos2(left_r.right() - 6.0, y), pos2(left_r.right(), y)], Stroke::new(1.0, tokens.border));
            let val = match unit {
                PhysicalUnit::Points => y_idx * 72,
                PhysicalUnit::Inches => y_idx,
                PhysicalUnit::Millimeters => y_idx * 10,
            };
            let label = format!("{val}");
            let mut galley = left_painter.layout_no_wrap(label.clone(), FontId::monospace(9.0), tokens.text_muted);
            if galley.size().x > left_clip.width() - 2.0 {
                galley = left_painter.layout_no_wrap(label, FontId::monospace(8.5), tokens.text_muted);
            }
            let label_pos = pos2(left_r.left() + 2.0, y + 2.0);
            if label_pos.x >= left_clip.left()
                && label_pos.y >= left_clip.top()
                && label_pos.y + galley.size().y <= left_clip.bottom() - 1.0
                && label_pos.x + galley.size().x <= left_clip.right()
            {
                left_painter.galley(label_pos, galley, tokens.text_muted);
            }
        }
        y += step_px;
        y_idx += 1;
    }
}

struct CanvasOverlayCtx<'a> {
    app: &'a UiApp,
    painter: &'a egui::Painter,
    mat_rect: Rect,
    paper_rect: Rect,
    sx: f32,
    sy: f32,
    tokens: &'a Tokens,
    enabled: bool,
}

impl CanvasOverlayCtx<'_> {
    fn screen_to_scene(&self, pos: Pos2) -> PointPt {
        PointPt::new(f64::from((pos.x - self.paper_rect.left()) / self.sx.max(1e-4)), f64::from((pos.y - self.paper_rect.top()) / self.sy.max(1e-4)))
    }

    fn push_scene_paste_table(&self, target_sheet_id: &SheetId, scene_anchor: [f64; 2], intents: &mut PendingUiIntents) {
        let Some(copied) = self.app.interaction.last_copied_result_table.as_ref() else {
            return;
        };
        let mut params = serde_json::Map::new();
        params.insert(String::from("source_sheet_id"), json!(copied.source_sheet_id.as_str()));
        params.insert(String::from("target_sheet_id"), json!(target_sheet_id.as_str()));
        if let Some(tab_id) = &copied.tab_id {
            params.insert(String::from("tab_id"), json!(tab_id));
        }
        if let Some(rows) = &copied.selected_rows {
            params.insert(String::from("selected_rows"), json!(rows));
        }
        if let Some(cols) = &copied.selected_cols {
            params.insert(String::from("selected_cols"), json!(cols));
        }
        params.insert(String::from("bounds_pt"), json!([scene_anchor[0].max(4.0), scene_anchor[1].max(4.0), 180.0, 90.0]));
        intents.push_engine("edit.pasteResultTable", serde_json::Value::Object(params));
    }
}

fn doc_rect_to_screen(paper_rect: Rect, sx: f32, sy: f32, bounds: DocRectPt) -> Rect {
    Rect::from_min_size(
        pos2(paper_rect.left() + (bounds.x as f32) * sx, paper_rect.top() + (bounds.y as f32) * sy),
        vec2((bounds.width as f32) * sx, (bounds.height as f32) * sy),
    )
}

fn active_gesture_bounds(state: &CanvasViewEguiState, sheet_id: &SheetId, is_layout: bool, target_idx: usize, fallback: DocRectPt) -> DocRectPt {
    if let Some(g) = &state.image_gesture
        && g.is_layout == is_layout
        && &g.sheet_id == sheet_id
        && g.target_index == target_idx
        && matches!(g.kind, ImageGestureKind::Move | ImageGestureKind::ResizeCorner { .. })
    {
        g.current_bounds_pt
    } else {
        fallback
    }
}

fn active_gesture_rotation(state: &CanvasViewEguiState, sheet_id: &SheetId, is_layout: bool, target_idx: usize, fallback: f64) -> f64 {
    if let Some(g) = &state.image_gesture
        && g.is_layout == is_layout
        && &g.sheet_id == sheet_id
        && g.target_index == target_idx
        && g.kind == ImageGestureKind::Rotate
    {
        g.current_rotation_deg
    } else {
        fallback
    }
}

fn paint_canvas_grid_overlay(
    painter: &egui::Painter,
    mat_rect: Rect,
    paper_rect: Rect,
    scene_w_pt: f32,
    scene_h_pt: f32,
    spacing_pt: f64,
    tokens: &Tokens,
) {
    let grid_clip = paper_rect.intersect(mat_rect);
    if !grid_clip.is_positive() {
        return;
    }
    let gp = painter.with_clip_rect(grid_clip);
    let spacing = spacing_pt.clamp(4.0, 200.0) as f32;
    let step_x = (spacing * paper_rect.width() / scene_w_pt.max(1.0)).max(6.0);
    let step_y = (spacing * paper_rect.height() / scene_h_pt.max(1.0)).max(6.0);
    let col = crate::egui_shell::rgba8_to_color32(crate::egui_shell::color32_to_rgba8(tokens.accent).with_alpha(48));
    let stroke = Stroke::new(0.75, col);

    let mut x = paper_rect.left() + step_x;
    let mut guard = 0usize;
    while x < paper_rect.right() - 1.0 && guard < 256 {
        gp.line_segment([pos2(x, grid_clip.top()), pos2(x, grid_clip.bottom())], stroke);
        x += step_x;
        guard += 1;
    }
    let mut y = paper_rect.top() + step_y;
    let mut y_guard = 0usize;
    while y < paper_rect.bottom() - 1.0 && y_guard < 256 {
        gp.line_segment([pos2(grid_clip.left(), y), pos2(grid_clip.right(), y)], stroke);
        y += step_y;
        y_guard += 1;
    }
}

fn graph_annotation_embedded_bounds(ann: &GraphAnnotation) -> Option<(DocRectPt, bool, f64, bool)> {
    match ann {
        GraphAnnotation::EmbeddedResultTable { bounds_pt, .. } => Some((*bounds_pt, false, 0.0, false)),
        GraphAnnotation::EmbeddedImage { bounds_pt, rotation_deg, lock_aspect_ratio, .. } => {
            Some((*bounds_pt, true, *rotation_deg, *lock_aspect_ratio))
        }
        _ => None,
    }
}

fn render_graph_embedded_objects(
    ui: &mut egui::Ui,
    ov: &CanvasOverlayCtx<'_>,
    gs: &GraphSheet,
    state: &mut CanvasViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> (bool, bool) {
    if let Some(sel) = state.selected_graph_annotation
        && gs.spec.annotations.get(sel).and_then(graph_annotation_embedded_bounds).is_none()
    {
        state.selected_graph_annotation = None;
    }

    let selected_img = state.selected_graph_annotation.and_then(|sel_idx| {
        let ann = gs.spec.annotations.get(sel_idx)?;
        let (bounds, is_img, rot, lock) = graph_annotation_embedded_bounds(ann)?;
        is_img.then_some((sel_idx, bounds, rot, lock))
    });
    let ptr_pos = ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos()));
    let resize_or_rot_active = state.image_gesture.as_ref().is_some_and(|g| !g.is_layout && g.kind != ImageGestureKind::Move);
    let pointer_in_handle = selected_img.is_some_and(|(sel_idx, init_bounds, _, _)| {
        let disp = active_gesture_bounds(state, &gs.id, false, sel_idx, init_bounds);
        let sel_r = doc_rect_to_screen(ov.paper_rect, ov.sx, ov.sy, disp);
        let h_rects = image_handle_rects(sel_r, ov.mat_rect);
        ptr_pos.is_some_and(|p| h_rects.iter().any(|hr| hr.contains(p)))
    });

    let mut claimed_click = false;
    let mut opened_ctx_menu = false;
    for (ann_idx, ann) in gs.spec.annotations.iter().enumerate() {
        let Some((init_bounds, is_image, _, _)) = graph_annotation_embedded_bounds(ann) else {
            continue;
        };
        let bounds = active_gesture_bounds(state, &gs.id, false, ann_idx, init_bounds);
        let r = doc_rect_to_screen(ov.paper_rect, ov.sx, ov.sy, bounds);
        let r_vis = r.intersect(ov.mat_rect).intersect(ui.clip_rect()).intersect(meta.viewport_rect);
        if r_vis.width() < 4.0 || r_vis.height() < 4.0 {
            continue;
        }
        let selected = state.selected_graph_annotation == Some(ann_idx);
        let suppress_drag = selected && is_image && (pointer_in_handle || resize_or_rot_active);
        let sense = if ov.enabled && !suppress_drag { Sense::click_and_drag() } else { Sense::hover() };
        let resp = ui.interact(r_vis, Id::unique(("graph_ann", gs.id.as_str(), ann_idx)), sense);
        if selected || resp.hovered() {
            let stroke_col = if selected { ov.tokens.accent } else { ov.tokens.focus_ring };
            ov.painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(if selected { 2.0 } else { 1.0 }, stroke_col), StrokeKind::Inside);
        }

        let wid = format!("canvas.graph_ann.{ann_idx}");
        let lbl = if is_image { "Embedded Image" } else { "Embedded Result Table" };
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "button",
                label: lbl,
                command_id: None,
                value: Some(json!({ "selected": selected, "x": bounds.x, "y": bounds.y })),
                enabled: ov.enabled,
                disabled_reason: (!ov.enabled).then_some("modal dialog is active"),
                z_layer: ZLayer::FloatingOverlay,
                ellipsized: false,
                full_label: None,
            },
        );

        if !ov.enabled {
            continue;
        }
        if resp.clicked() {
            state.selected_graph_annotation = Some(ann_idx);
            claimed_click = true;
        }
        if resp.double_clicked() || resp.triple_clicked() {
            state.selected_graph_annotation = Some(ann_idx);
            claimed_click = true;
            let cmd = if is_image { "ui.image.format.open" } else { "ui.embeddedTable.format.open" };
            intents.push_ui(cmd, json!({ "sheet_id": gs.id.as_str(), "annotation_index": ann_idx }));
        }
        if resp.secondary_clicked() {
            let anchor = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r_vis.center());
            let sc_pt = ov.screen_to_scene(anchor);
            state.selected_graph_annotation = Some(ann_idx);
            state.graph_context_menu = Some((gs.id.clone(), Some(ann_idx), anchor, [sc_pt.x, sc_pt.y]));
            opened_ctx_menu = true;
        }
        if !suppress_drag && resp.drag_started() {
            state.selected_graph_annotation = Some(ann_idx);
            let ptr = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r_vis.center());
            let sc = ov.screen_to_scene(ptr);
            state.image_gesture = Some(ImageTransformGestureState {
                sheet_id: gs.id.clone(),
                is_layout: false,
                target_index: ann_idx,
                kind: ImageGestureKind::Move,
                start_scene_pt: [sc.x, sc.y],
                start_screen_pt: [ptr.x, ptr.y],
                initial_bounds_pt: init_bounds,
                current_bounds_pt: init_bounds,
                initial_rotation_deg: 0.0,
                current_rotation_deg: 0.0,
                initial_lock_aspect: true,
                shift_unlocked: false,
            });
        }
        if resp.dragged()
            && let Some(g) = state.image_gesture.as_mut()
            && !g.is_layout
            && g.sheet_id == gs.id
            && g.target_index == ann_idx
            && g.kind == ImageGestureKind::Move
        {
            let ptr = ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos())).unwrap_or(r_vis.center());
            let sc = ov.screen_to_scene(ptr);
            g.current_bounds_pt.x = round_tenth_pt(g.initial_bounds_pt.x + (sc.x - g.start_scene_pt[0]));
            g.current_bounds_pt.y = round_tenth_pt(g.initial_bounds_pt.y + (sc.y - g.start_scene_pt[1]));
        }
        let finish_move = resp.drag_stopped()
            && state
                .image_gesture
                .as_ref()
                .is_some_and(|g| !g.is_layout && g.sheet_id == gs.id && g.target_index == ann_idx && g.kind == ImageGestureKind::Move);
        if finish_move && let Some(g) = state.image_gesture.take() {
            commit_graph_embedded_move(gs, ann_idx, is_image, g.current_bounds_pt, intents);
        }
    }

    if let Some((sel_idx, init_bounds, rot_deg, lock_aspect)) = selected_img {
        render_selected_image_handles(
            ui,
            ov,
            SelectedImageTarget { sheet_id: &gs.id, is_layout: false, target_idx: sel_idx, init_bounds, init_rot_deg: rot_deg, lock_aspect },
            state,
            meta,
            intents,
        );
    }

    (claimed_click, opened_ctx_menu)
}

fn commit_graph_embedded_move(gs: &GraphSheet, ann_idx: usize, is_image: bool, bounds: DocRectPt, intents: &mut PendingUiIntents) {
    if is_image {
        intents.push_engine(
            "graph.updateImage",
            json!({
                "sheet_id": gs.id.as_str(),
                "annotation_index": ann_idx,
                "bounds_pt": [bounds.x, bounds.y, bounds.width, bounds.height],
            }),
        );
        return;
    }
    let mut updated = gs.spec.annotations.clone();
    if let Some(GraphAnnotation::EmbeddedResultTable { bounds_pt, .. }) = updated.get_mut(ann_idx) {
        *bounds_pt = bounds;
        intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "annotations": updated } }));
    }
}

fn render_context_menu_item(
    ui: &mut egui::Ui,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    command_id: Option<&str>,
    enabled: bool,
) -> bool {
    let btn = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(label).small()).fill(Color32::TRANSPARENT).min_size(vec2(228.0, 26.0)));
    meta.record_response(
        ui,
        &btn,
        FrameInspectEntryInput {
            widget_id,
            role: "menuitem",
            label,
            command_id,
            value: None,
            enabled,
            disabled_reason: (!enabled).then_some("Copy a result table selection first"),
            z_layer: ZLayer::ContextMenu,
            ellipsized: false,
            full_label: None,
        },
    );
    btn.clicked() && enabled
}

fn render_graph_context_menu(
    ui: &mut egui::Ui,
    ov: &CanvasOverlayCtx<'_>,
    gs: &GraphSheet,
    state: &mut CanvasViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if !ov.enabled {
        state.graph_context_menu = None;
        return;
    }
    let Some((ref menu_sheet, opt_ann_idx, anchor, scene_anchor)) = state.graph_context_menu.clone() else {
        return;
    };
    if menu_sheet != &gs.id {
        state.graph_context_menu = None;
        return;
    }
    let screen = ui.ctx().content_rect();
    let pos = pos2(
        anchor.x.clamp(screen.left() + 4.0, (screen.right() - 244.0).max(screen.left() + 4.0)),
        anchor.y.clamp(screen.top() + 4.0, (screen.bottom() - 200.0).max(screen.top() + 4.0)),
    );
    let target_is_table =
        opt_ann_idx.and_then(|i| gs.spec.annotations.get(i)).is_some_and(|a| matches!(a, GraphAnnotation::EmbeddedResultTable { .. }));
    let target_is_image = opt_ann_idx.and_then(|i| gs.spec.annotations.get(i)).is_some_and(|a| matches!(a, GraphAnnotation::EmbeddedImage { .. }));
    let has_copied_table = ov.app.interaction.last_copied_result_table.is_some();
    let locale = ov.tokens.locale;

    let mut close_menu = false;
    let area_resp =
        egui::Area::new(Id::unique(("graph_ctx_menu", gs.id.as_str()))).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |menu_ui| {
            egui::Frame::popup(menu_ui.style())
                .fill(ov.tokens.surface_bg)
                .stroke(Stroke::new(1.0, ov.tokens.border))
                .corner_radius(CornerRadius::same(4))
                .show(menu_ui, |ui| {
                    ui.set_min_width(232.0);
                    if render_context_menu_item(ui, meta, "canvas.graph_ctx.copy_image", "Copy as Image", Some("edit.copy"), true) {
                        intents.push_workspace_copy(144);
                        close_menu = true;
                    }
                    if render_context_menu_item(
                        ui,
                        meta,
                        "canvas.graph_ctx.copy_for_slides",
                        "Copy for Slides & Documents",
                        Some("ui.clipboard.copyForSlides"),
                        true,
                    ) {
                        intents.push_workspace_copy_for_slides(300);
                        close_menu = true;
                    }
                    ui.separator();
                    if render_context_menu_item(
                        ui,
                        meta,
                        "canvas.graph_ctx.paste_table",
                        tr(locale, "r13.canvas.paste_table"),
                        Some("edit.pasteResultTable"),
                        has_copied_table,
                    ) {
                        ov.push_scene_paste_table(&gs.id, scene_anchor, intents);
                        close_menu = true;
                    }
                    if render_context_menu_item(
                        ui,
                        meta,
                        "canvas.graph_ctx.paste_image",
                        tr(locale, "r13.canvas.paste_image"),
                        Some("edit.pasteImage"),
                        true,
                    ) {
                        intents.push_paste_image_from_clipboard(gs.id.clone(), Some(scene_anchor));
                        close_menu = true;
                    }
                    if render_context_menu_item(
                        ui,
                        meta,
                        "canvas.graph_ctx.insert_image",
                        tr(locale, "r13.canvas.insert_image"),
                        Some("graph.insertImage"),
                        true,
                    ) {
                        intents.push_engine("graph.insertImage", json!({ "sheet_id": gs.id.as_str() }));
                        close_menu = true;
                    }
                    if let Some(ann_idx) = opt_ann_idx
                        && target_is_table
                        && render_context_menu_item(
                            ui,
                            meta,
                            "canvas.graph_ctx.format_embedded_table",
                            tr(locale, "r13.canvas.format_embedded_table"),
                            Some("ui.embeddedTable.format.open"),
                            true,
                        )
                    {
                        intents.push_ui("ui.embeddedTable.format.open", json!({ "sheet_id": gs.id.as_str(), "annotation_index": ann_idx }));
                        close_menu = true;
                    }
                    if let Some(ann_idx) = opt_ann_idx
                        && target_is_image
                        && render_context_menu_item(
                            ui,
                            meta,
                            "canvas.graph_ctx.format_image",
                            tr(locale, "r13.canvas.format_image"),
                            Some("ui.image.format.open"),
                            true,
                        )
                    {
                        intents.push_ui("ui.image.format.open", json!({ "sheet_id": gs.id.as_str(), "annotation_index": ann_idx }));
                        close_menu = true;
                    }
                    ui.separator();
                    if render_context_menu_item(
                        ui,
                        meta,
                        "canvas.graph_ctx.toggle_grid",
                        tr(locale, "r13.canvas.grid_overlay"),
                        Some("ui.gridOverlay.toggle"),
                        true,
                    ) {
                        intents.push_ui("ui.gridOverlay.toggle", json!({}));
                        close_menu = true;
                    }
                });
        });

    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let outside_click = ui.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| !area_resp.response.rect.contains(p)));
    if close_menu || esc || outside_click {
        state.graph_context_menu = None;
    }
}

fn append_layout_context_menu_extras(
    ui: &mut egui::Ui,
    ov: &CanvasOverlayCtx<'_>,
    ls: &LayoutSheet,
    opt_panel_idx: Option<usize>,
    anchor: Pos2,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> bool {
    let locale = ov.tokens.locale;
    let sc_pt = ov.screen_to_scene(anchor);
    let item_opt = opt_panel_idx.and_then(|idx| ls.spec.items.get(idx));
    let mut clicked = false;

    if render_context_menu_item(ui, meta, "canvas.layout_ctx.copy_image", "Copy as Image", Some("edit.copy"), true) {
        intents.push_workspace_copy(144);
        clicked = true;
    }
    if render_context_menu_item(
        ui,
        meta,
        "canvas.layout_ctx.copy_for_slides",
        "Copy for Slides & Documents",
        Some("ui.clipboard.copyForSlides"),
        true,
    ) {
        intents.push_workspace_copy_for_slides(300);
        clicked = true;
    }
    ui.separator();

    if let Some(panel_idx) = opt_panel_idx {
        if matches!(item_opt, Some(LayoutItem::LiveTable { .. }))
            && render_context_menu_item(
                ui,
                meta,
                "canvas.layout_ctx.format_embedded_table",
                tr(locale, "r13.canvas.format_embedded_table"),
                Some("ui.embeddedTable.format.open"),
                true,
            )
        {
            intents.push_ui("ui.embeddedTable.format.open", json!({ "sheet_id": ls.id.as_str(), "panel_index": panel_idx }));
            clicked = true;
        }
        if matches!(item_opt, Some(LayoutItem::EmbeddedImage { .. }))
            && render_context_menu_item(
                ui,
                meta,
                "canvas.layout_ctx.format_image",
                tr(locale, "r13.canvas.format_image"),
                Some("ui.image.format.open"),
                true,
            )
        {
            intents.push_ui("ui.image.format.open", json!({ "sheet_id": ls.id.as_str(), "panel_index": panel_idx }));
            clicked = true;
        }
    }
    if render_context_menu_item(ui, meta, "canvas.layout_ctx.paste_image", tr(locale, "r13.canvas.paste_image"), Some("edit.pasteImage"), true) {
        intents.push_paste_image_from_clipboard(ls.id.clone(), Some([sc_pt.x, sc_pt.y]));
        clicked = true;
    }
    if render_context_menu_item(ui, meta, "canvas.layout_ctx.insert_image", tr(locale, "r13.canvas.insert_image"), Some("layout.insertImage"), true) {
        intents.push_engine("layout.insertImage", json!({ "sheet_id": ls.id.as_str() }));
        clicked = true;
    }
    if render_context_menu_item(ui, meta, "canvas.layout_ctx.toggle_grid", tr(locale, "r13.canvas.grid_overlay"), Some("ui.gridOverlay.toggle"), true)
    {
        intents.push_ui("ui.gridOverlay.toggle", json!({}));
        clicked = true;
    }
    ui.separator();
    ui.label(egui::RichText::new("Add Graph Panel").small().color(ov.tokens.text_muted));
    let graphs = &ov.app.document().graph_sheets;
    if graphs.is_empty() {
        let _ = render_context_menu_item(ui, meta, "canvas.layout_ctx.add_panel.empty", "No Graph Sheets", Some("layout.addPanel"), false);
    } else if graphs.len() > 4 {
        egui::ScrollArea::vertical().id_salt("layout_ctx_add_panel_scroll").max_height(130.0).show(ui, |ui| {
            for (idx, gs) in graphs.iter().enumerate() {
                if render_add_graph_panel_ctx_item(ui, ls.id.as_str(), idx, gs, meta, intents) {
                    clicked = true;
                }
            }
        });
    } else {
        for (idx, gs) in graphs.iter().enumerate() {
            if render_add_graph_panel_ctx_item(ui, ls.id.as_str(), idx, gs, meta, intents) {
                clicked = true;
            }
        }
    }
    clicked
}

fn render_add_graph_panel_ctx_item(
    ui: &mut egui::Ui,
    layout_sheet_id: &str,
    idx: usize,
    gs: &GraphSheet,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> bool {
    let wid = format!("canvas.layout_ctx.add_panel.{idx}");
    let lbl = format!("+ {}", gs.title);
    if render_context_menu_item(ui, meta, &wid, &lbl, Some("layout.addPanel"), true) {
        intents.push_engine(
            "layout.addPanel",
            json!({
                "sheet_id": layout_sheet_id,
                "kind": "graph",
                "graph_sheet_id": gs.id.as_str(),
            }),
        );
        return true;
    }
    false
}

fn render_layout_context_menu_popup(
    ui: &mut egui::Ui,
    ov: &CanvasOverlayCtx<'_>,
    ls: &LayoutSheet,
    state: &mut CanvasViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if !ov.enabled {
        state.layout_context_menu = None;
        return;
    }
    let Some((ref menu_sheet, opt_panel_idx, anchor)) = state.layout_context_menu.clone() else {
        return;
    };
    if menu_sheet != &ls.id || opt_panel_idx.is_some_and(|idx| idx >= ls.spec.items.len()) {
        state.layout_context_menu = None;
        return;
    }
    let screen = ui.ctx().content_rect();
    let pos = pos2(
        anchor.x.clamp(screen.left() + 4.0, (screen.right() - 242.0).max(screen.left() + 4.0)),
        anchor.y.clamp(screen.top() + 4.0, (screen.bottom() - 360.0).max(screen.top() + 4.0)),
    );
    let mut close_menu = false;
    let area_resp = egui::Area::new(Id::unique(("layout_panel_ctx_menu", ls.id.as_str()))).order(egui::Order::Foreground).fixed_pos(pos).show(
        ui.ctx(),
        |menu_ui| {
            egui::Frame::popup(menu_ui.style())
                .fill(ov.tokens.surface_bg)
                .stroke(Stroke::new(1.0, ov.tokens.border))
                .corner_radius(CornerRadius::same(4))
                .show(menu_ui, |ui| {
                    ui.set_min_width(232.0);
                    if let Some(panel_idx) = opt_panel_idx {
                        for (slug, lbl, action_str) in [
                            ("front", "Bring to Front", "bring_to_front"),
                            ("forward", "Bring Forward", "bring_forward"),
                            ("backward", "Send Backward", "send_backward"),
                            ("back", "Send to Back", "send_to_back"),
                        ] {
                            let wid = format!("canvas.layout_ctx.{slug}");
                            let btn = ui.add_sized([228.0, 26.0], egui::Button::new(lbl).fill(Color32::TRANSPARENT));
                            meta.record_response(
                                ui,
                                &btn,
                                FrameInspectEntryInput {
                                    widget_id: &wid,
                                    role: "menuitem",
                                    label: lbl,
                                    command_id: Some("layout.reorderPanel"),
                                    value: Some(json!(action_str)),
                                    enabled: true,
                                    disabled_reason: None,
                                    z_layer: ZLayer::ContextMenu,
                                    ellipsized: false,
                                    full_label: None,
                                },
                            );
                            if btn.clicked() {
                                intents.push_engine(
                                    "layout.reorderPanel",
                                    json!({
                                        "sheet_id": ls.id.as_str(),
                                        "panel_index": panel_idx,
                                        "action": action_str,
                                    }),
                                );
                                close_menu = true;
                            }
                        }
                        ui.separator();
                    }
                    if append_layout_context_menu_extras(ui, ov, ls, opt_panel_idx, anchor, meta, intents) {
                        close_menu = true;
                    }
                });
        },
    );
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let outside_click = ui.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| !area_resp.response.rect.contains(p)));
    if close_menu || esc || outside_click {
        state.layout_context_menu = None;
    }
}

fn selected_non_image_panel_state(
    ls: &LayoutSheet,
    state: &CanvasViewEguiState,
    paper_rect: Rect,
    sx: f32,
    sy: f32,
    ptr_pos: Option<Pos2>,
    selected_is_image: bool,
) -> (Option<usize>, bool, bool) {
    let panel_resize_active = state.layout_drag.as_ref().is_some_and(|d| d.sheet_id == ls.id && d.resizing_corner);
    if selected_is_image {
        return (None, false, panel_resize_active);
    }
    let Some(sel_idx) = state.selected_panel_index else {
        return (None, false, panel_resize_active);
    };
    let Some(item) = ls.spec.items.get(sel_idx) else {
        return (None, false, panel_resize_active);
    };
    let b = state
        .layout_drag
        .as_ref()
        .filter(|d| d.sheet_id == ls.id && d.item_index == sel_idx)
        .map_or_else(|| layout_item_bounds_doc(item), |d| d.current_bounds_pt);
    let sel_r = doc_rect_to_screen(paper_rect, sx, sy, b);
    let se_rect = Rect::from_center_size(sel_r.right_bottom(), vec2(10.0, 10.0));
    let ptr_in_handle = ptr_pos.is_some_and(|p| se_rect.contains(p));
    (Some(sel_idx), ptr_in_handle, panel_resize_active)
}

fn render_layout_panel_resize_handle(
    ui: &mut egui::Ui,
    ov: &CanvasOverlayCtx<'_>,
    ls: &LayoutSheet,
    state: &mut CanvasViewEguiState,
    sel_idx: usize,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let Some(item) = ls.spec.items.get(sel_idx) else {
        return;
    };
    let bounds = state
        .layout_drag
        .as_ref()
        .filter(|d| d.sheet_id == ls.id && d.item_index == sel_idx)
        .map_or_else(|| layout_item_bounds_doc(item), |d| d.current_bounds_pt);
    let sel_r = doc_rect_to_screen(ov.paper_rect, ov.sx, ov.sy, bounds);
    let clip_all = ov.mat_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
    let se_rect = Rect::from_center_size(sel_r.right_bottom(), vec2(10.0, 10.0));
    let se_vis = se_rect.intersect(clip_all);
    if se_vis.width() < 4.0 || se_vis.height() < 4.0 {
        return;
    }

    let resizing_this = state.layout_drag.as_ref().is_some_and(|d| d.sheet_id == ls.id && d.item_index == sel_idx && d.resizing_corner);
    if resizing_this {
        ov.painter.rect_stroke(sel_r, CornerRadius::ZERO, Stroke::new(1.5, ov.tokens.accent), StrokeKind::Outside);
    }

    let wid = format!("canvas.layout_panel.{sel_idx}.resize_se");
    let h_resp = ui
        .interact(se_vis, Id::unique(("layout_panel_resize_se", ls.id.as_str(), sel_idx)), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
    h_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ov.enabled, "Resize Panel Corner (SE)"));
    let fill = if h_resp.hovered() || resizing_this { ov.tokens.accent } else { ov.tokens.surface_bg };
    ov.painter.rect(se_rect, CornerRadius::same(2), fill, Stroke::new(1.5, ov.tokens.accent), StrokeKind::Middle);
    meta.record_response(
        ui,
        &h_resp,
        FrameInspectEntryInput {
            widget_id: &wid,
            role: "button",
            label: "Resize Panel Corner (SE)",
            command_id: Some("layout.setSpec"),
            value: Some(json!({ "bounds_pt": [bounds.x, bounds.y, bounds.width, bounds.height] })),
            enabled: ov.enabled,
            disabled_reason: None,
            z_layer: ZLayer::FloatingOverlay,
            ellipsized: false,
            full_label: None,
        },
    );

    if h_resp.drag_started() {
        let ptr = ui.input(|i| i.pointer.interact_pos()).unwrap_or(se_rect.center());
        let sc = ov.screen_to_scene(ptr);
        let base_bounds = layout_item_bounds_doc(item);
        state.selected_panel_index = Some(sel_idx);
        state.layout_drag = Some(LayoutPanelDragState {
            sheet_id: ls.id.clone(),
            item_index: sel_idx,
            start_scene_pt: [sc.x, sc.y],
            initial_bounds_pt: base_bounds,
            current_bounds_pt: base_bounds,
            resizing_corner: true,
        });
    }
    if h_resp.dragged()
        && let Some(drag) = state.layout_drag.as_mut()
        && drag.sheet_id == ls.id
        && drag.item_index == sel_idx
        && drag.resizing_corner
    {
        let ptr = ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos())).unwrap_or(se_rect.center());
        let sc = ov.screen_to_scene(ptr);
        let new_w = round_tenth_pt((drag.initial_bounds_pt.width + (sc.x - drag.start_scene_pt[0])).max(16.0));
        let new_h = round_tenth_pt((drag.initial_bounds_pt.height + (sc.y - drag.start_scene_pt[1])).max(16.0));
        drag.current_bounds_pt = DocRectPt::new(drag.initial_bounds_pt.x, drag.initial_bounds_pt.y, new_w, new_h);
        state.last_snap_readout = Some(format!("Size: W={new_w:.1} pt, H={new_h:.1} pt (0.1 pt)"));
    }
    if h_resp.drag_stopped()
        && state.layout_drag.as_ref().is_some_and(|d| d.sheet_id == ls.id && d.item_index == sel_idx && d.resizing_corner)
        && let Some(drag) = state.layout_drag.take()
    {
        if drag.current_bounds_pt != drag.initial_bounds_pt {
            let mut updated_items = ls.spec.items.clone();
            if let Some(target_item) = updated_items.get_mut(sel_idx) {
                set_layout_item_bounds_doc(target_item, drag.current_bounds_pt);
                intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "spec": { "items": updated_items } }));
            }
        }
        state.active_snap_guides.clear();
        state.last_snap_readout = None;
    }
}

fn image_handle_rects(r: Rect, mat_rect: Rect) -> [Rect; 5] {
    let nw = Rect::from_center_size(r.left_top(), vec2(10.0, 10.0));
    let ne = Rect::from_center_size(r.right_top(), vec2(10.0, 10.0));
    let sw = Rect::from_center_size(r.left_bottom(), vec2(10.0, 10.0));
    let se = Rect::from_center_size(r.right_bottom(), vec2(10.0, 10.0));
    let rot_center = pos2(r.center().x, (r.top() - 18.0).max(mat_rect.top() + 8.0));
    let rot = Rect::from_center_size(rot_center, vec2(10.0, 10.0));
    [nw, ne, sw, se, rot]
}

fn shift_modifier_held(ui: &egui::Ui) -> bool {
    ui.input(|i| {
        i.events
            .iter()
            .rev()
            .find_map(|ev| match ev {
                egui::Event::PointerButton { modifiers, .. } => Some(modifiers.shift),
                egui::Event::ModifiersChanged(m) => Some(m.shift),
                _ => None,
            })
            .unwrap_or(i.modifiers.shift)
    })
}

fn normalize_deg_180(deg: f64) -> f64 {
    let mut d = ((deg + 180.0).rem_euclid(360.0)) - 180.0;
    if d <= -180.0 {
        d += 360.0;
    }
    d
}

fn pointer_angle_deg(center: Pos2, pos: Pos2) -> f64 {
    f64::from((pos.y - center.y).atan2(pos.x - center.x)).to_degrees() + 90.0
}

struct SelectedImageTarget<'a> {
    sheet_id: &'a SheetId,
    is_layout: bool,
    target_idx: usize,
    init_bounds: DocRectPt,
    init_rot_deg: f64,
    lock_aspect: bool,
}

fn render_selected_image_handles(
    ui: &mut egui::Ui,
    ov: &CanvasOverlayCtx<'_>,
    target: SelectedImageTarget<'_>,
    state: &mut CanvasViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let SelectedImageTarget { sheet_id, is_layout, target_idx, init_bounds, init_rot_deg, lock_aspect } = target;
    if !ov.enabled {
        return;
    }
    let disp_bounds = active_gesture_bounds(state, sheet_id, is_layout, target_idx, init_bounds);
    let disp_rot = active_gesture_rotation(state, sheet_id, is_layout, target_idx, init_rot_deg);
    let r = doc_rect_to_screen(ov.paper_rect, ov.sx, ov.sy, disp_bounds);
    let clip_all = ov.mat_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);

    if state.image_gesture.as_ref().is_some_and(|g| g.sheet_id == *sheet_id && g.is_layout == is_layout && g.target_index == target_idx) {
        ov.painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5, ov.tokens.accent), StrokeKind::Inside);
    }

    let corners = [
        ("nw", true, true, r.left_top()),
        ("ne", false, true, r.right_top()),
        ("sw", true, false, r.left_bottom()),
        ("se", false, false, r.right_bottom()),
    ];
    for (slug, left, top, c_pos) in corners {
        let h_rect = Rect::from_center_size(c_pos, vec2(10.0, 10.0));
        let h_vis = h_rect.intersect(clip_all);
        if h_vis.width() < 4.0 || h_vis.height() < 4.0 {
            continue;
        }
        ov.painter.rect_filled(h_vis, CornerRadius::same(1), ov.tokens.paper_white);
        ov.painter.rect_stroke(h_vis, CornerRadius::same(1), Stroke::new(1.5, ov.tokens.accent), StrokeKind::Inside);
        let h_resp = ui.interact(h_vis, Id::unique(("img_handle", sheet_id.as_str(), is_layout, target_idx, slug)), Sense::drag());
        let wid = format!("canvas.image.handle.{slug}");
        meta.record_response(
            ui,
            &h_resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "button",
                label: &format!("Resize Handle {}", slug.to_ascii_uppercase()),
                command_id: None,
                value: Some(json!({ "bounds_pt": [disp_bounds.x, disp_bounds.y, disp_bounds.width, disp_bounds.height] })),
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::FloatingOverlay,
                ellipsized: false,
                full_label: None,
            },
        );
        if h_resp.drag_started() {
            let ptr = ui.input(|i| i.pointer.interact_pos()).unwrap_or(c_pos);
            let sc = ov.screen_to_scene(ptr);
            state.image_gesture = Some(ImageTransformGestureState {
                sheet_id: sheet_id.clone(),
                is_layout,
                target_index: target_idx,
                kind: ImageGestureKind::ResizeCorner { left, top },
                start_scene_pt: [sc.x, sc.y],
                start_screen_pt: [ptr.x, ptr.y],
                initial_bounds_pt: init_bounds,
                current_bounds_pt: init_bounds,
                initial_rotation_deg: init_rot_deg,
                current_rotation_deg: init_rot_deg,
                initial_lock_aspect: lock_aspect,
                shift_unlocked: shift_modifier_held(ui),
            });
        }
        let mut commit_bounds = None;
        if (h_resp.dragged() || h_resp.drag_stopped())
            && let Some(g) = state.image_gesture.as_mut()
            && g.sheet_id == *sheet_id
            && g.is_layout == is_layout
            && g.target_index == target_idx
            && g.kind == (ImageGestureKind::ResizeCorner { left, top })
        {
            if let Some(pos) = ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos())) {
                update_corner_resize_gesture(ui, ov, g, pos, left, top);
            }
            if h_resp.drag_stopped() {
                commit_bounds = Some(g.current_bounds_pt);
            }
        }
        if let Some(b) = commit_bounds {
            state.image_gesture = None;
            let cmd = if is_layout { "layout.updateImage" } else { "graph.updateImage" };
            let idx_key = if is_layout { "item_index" } else { "annotation_index" };
            intents.push_engine(
                cmd,
                json!({
                    "sheet_id": sheet_id.as_str(),
                    idx_key: target_idx,
                    "bounds_pt": [b.x, b.y, b.width, b.height],
                }),
            );
        }
    }

    // Top-center rotation handle
    let rot_center = pos2(r.center().x, (r.top() - 18.0).max(ov.mat_rect.top() + 8.0));
    let rot_rect = Rect::from_center_size(rot_center, vec2(10.0, 10.0));
    let rot_vis = rot_rect.intersect(clip_all);
    if rot_vis.width() >= 4.0 && rot_vis.height() >= 4.0 {
        ov.painter.line_segment([pos2(r.center().x, r.top()), rot_center], Stroke::new(1.25, ov.tokens.accent));
        ov.painter.circle_filled(rot_center, 5.0, ov.tokens.paper_white);
        ov.painter.circle_stroke(rot_center, 5.0, Stroke::new(1.5, ov.tokens.accent));
        let rot_resp = ui.interact(rot_vis, Id::unique(("img_rot_handle", sheet_id.as_str(), is_layout, target_idx)), Sense::drag());
        meta.record_response(
            ui,
            &rot_resp,
            FrameInspectEntryInput {
                widget_id: "canvas.image.handle.rotate",
                role: "button",
                label: "Rotate Handle",
                command_id: None,
                value: Some(json!({ "rotation_deg": disp_rot })),
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::FloatingOverlay,
                ellipsized: false,
                full_label: None,
            },
        );
        if rot_resp.drag_started() {
            let ptr = ui.input(|i| i.pointer.interact_pos()).unwrap_or(rot_center);
            let sc = ov.screen_to_scene(ptr);
            state.image_gesture = Some(ImageTransformGestureState {
                sheet_id: sheet_id.clone(),
                is_layout,
                target_index: target_idx,
                kind: ImageGestureKind::Rotate,
                start_scene_pt: [sc.x, sc.y],
                start_screen_pt: [ptr.x, ptr.y],
                initial_bounds_pt: init_bounds,
                current_bounds_pt: init_bounds,
                initial_rotation_deg: init_rot_deg,
                current_rotation_deg: init_rot_deg,
                initial_lock_aspect: lock_aspect,
                shift_unlocked: shift_modifier_held(ui),
            });
        }
        let mut commit_rot = None;
        if (rot_resp.dragged() || rot_resp.drag_stopped())
            && let Some(g) = state.image_gesture.as_mut()
            && g.sheet_id == *sheet_id
            && g.is_layout == is_layout
            && g.target_index == target_idx
            && g.kind == ImageGestureKind::Rotate
        {
            if let Some(pos) = ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos())) {
                let start_pos = pos2(g.start_screen_pt[0], g.start_screen_pt[1]);
                let delta = normalize_deg_180(pointer_angle_deg(r.center(), pos) - pointer_angle_deg(r.center(), start_pos));
                let raw_deg = normalize_deg_180(g.initial_rotation_deg + delta);
                g.current_rotation_deg = if shift_modifier_held(ui) { (raw_deg / 15.0).round() * 15.0 } else { round_tenth_pt(raw_deg) };
                paint_hover_hud_card(ov.painter, pos + vec2(12.0, -18.0), &format!("{:.1}°", g.current_rotation_deg), ov.tokens);
            }
            if rot_resp.drag_stopped() {
                commit_rot = Some(g.current_rotation_deg);
            }
        }
        if let Some(next_rot) = commit_rot {
            state.image_gesture = None;
            let cmd = if is_layout { "layout.updateImage" } else { "graph.updateImage" };
            let idx_key = if is_layout { "item_index" } else { "annotation_index" };
            intents.push_engine(cmd, json!({ "sheet_id": sheet_id.as_str(), idx_key: target_idx, "rotation_deg": next_rot }));
        }
    }
}

fn update_corner_resize_gesture(ui: &egui::Ui, ov: &CanvasOverlayCtx<'_>, g: &mut ImageTransformGestureState, pos: Pos2, left: bool, top: bool) {
    let sc = ov.screen_to_scene(pos);
    let init = g.initial_bounds_pt;
    let dx = sc.x - g.start_scene_pt[0];
    let dy = sc.y - g.start_scene_pt[1];
    let raw_w = if left { init.width - dx } else { init.width + dx }.max(12.0);
    let raw_h = if top { init.height - dy } else { init.height + dy }.max(12.0);
    let shift_held = shift_modifier_held(ui);
    g.shift_unlocked = shift_held;

    let (new_w, new_h) = if g.initial_lock_aspect && !shift_held {
        let aspect = (init.width / init.height.max(1e-6)).max(1e-4);
        let rel_dw = (raw_w - init.width).abs() / init.width.max(1.0);
        let rel_dh = (raw_h - init.height).abs() / init.height.max(1.0);
        if rel_dw >= rel_dh { (raw_w, (raw_w / aspect).max(12.0)) } else { ((raw_h * aspect).max(12.0), raw_h) }
    } else {
        (raw_w, raw_h)
    };

    let anchor_x = init.x + init.width;
    let anchor_y = init.y + init.height;
    let new_x = if left { anchor_x - new_w } else { init.x };
    let new_y = if top { anchor_y - new_h } else { init.y };
    g.current_bounds_pt = DocRectPt::new(round_tenth_pt(new_x), round_tenth_pt(new_y), round_tenth_pt(new_w), round_tenth_pt(new_h));
}

#[cfg(test)]
mod raster_worker_drop_tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use curvebook_platform::sync::RwLock;

    use super::{AsyncRasterWorkerOwner, RasterWorkerResults};

    /// Dropping the last owner reference on the worker thread itself must detach rather than
    /// self-join (which panics with `EDEADLK`), so the thread runs to completion.
    #[test]
    fn owner_dropped_on_its_own_worker_thread_does_not_self_join() {
        let (owner_tx, owner_rx) = mpsc::channel::<Arc<AsyncRasterWorkerOwner>>();
        let finished = Arc::new(AtomicBool::new(false));
        let finished_bg = Arc::clone(&finished);
        let spawned = std::thread::Builder::new().spawn(move || {
            if let Ok(owner) = owner_rx.recv() {
                drop(owner);
                finished_bg.store(true, Ordering::SeqCst);
            }
        });
        let Ok(join_handle) = spawned else {
            return;
        };
        let owner = Arc::new(AsyncRasterWorkerOwner {
            tx: RwLock::new(None),
            results: Arc::new(RwLock::new(RasterWorkerResults::default())),
            join: RwLock::new(Some(join_handle)),
        });
        assert!(owner_tx.send(owner).is_ok(), "owner hand-off to worker thread failed");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !finished.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(finished.load(Ordering::SeqCst), "worker thread panicked or hung while dropping its own owner");
    }

    #[test]
    fn async_failed_raster_clears_stale_texture_and_stops_polling() {
        use std::sync::atomic::AtomicUsize;

        use curvebook_chart::ChartScene;
        use curvebook_doc::SheetId;
        use curvebook_palette::Rgba8;

        use super::{CanvasViewEguiState, CompositeSceneSpec, composite_async_scene_texture};
        use crate::UiApp;

        let ctx = egui::Context::default();
        let repaints = Arc::new(AtomicUsize::new(0));
        let repaints_cb = Arc::clone(&repaints);
        ctx.set_request_repaint_callback(move |_| {
            repaints_cb.fetch_add(1, Ordering::Relaxed);
        });

        let app = UiApp::new();
        let mut state = CanvasViewEguiState::default();
        // Seed a stale texture from an earlier valid sheet to verify failed raster clears it.
        state.slot.cached_texture = Some(egui::TextureId::Managed(42));
        state.slot.last_sheet_id = Some(SheetId::from("old-sheet"));

        // Scene with invalid width_pt = 0.0 causes render_scene() to return Err -> worker yields None.
        let mut raw_bad = ChartScene::new("bad-gs", "Bad", 100.0, 100.0, Some(Rgba8::WHITE));
        raw_bad.width_pt = 0.0;
        let bad_scene = Arc::new(raw_bad);
        let bad_id = SheetId::from("bad-gs");
        let paper = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(300.0, 220.0));

        // Warm up initial egui frame before measuring repaints from the async raster submission.
        let mut sim = 0.5_f64;
        let mut warm = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))),
                time: Some(sim),
                focused: true,
                ..Default::default()
            },
            |_| {},
        );
        warm.textures_delta.clear();
        repaints.store(0, Ordering::SeqCst);

        // Frame 0: submits async raster job without scheduling a 16 ms polling timer.
        sim = 1.0_f64;
        let mut out0 = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))),
                time: Some(sim),
                focused: true,
                ..Default::default()
            },
            |ui| {
                composite_async_scene_texture(
                    ui,
                    &app,
                    CompositeSceneSpec { scene: &bad_scene, sheet_id: &bad_id, paper_rect: paper, slot_tag: "main", painter: ui.painter() },
                    &mut state,
                );
            },
        );
        out0.textures_delta.clear();
        let d0 = out0.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
        assert_ne!(d0, Some(Duration::from_millis(16)), "must not poll at 16 ms while async worker runs");

        // Wait up to 2s for BOTH worker completion (`rendered_count >= 1`) AND wakeup callback.
        let worker = super::AsyncRasterWorkerHandle::try_get(&ctx).expect("worker handle must be installed");
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let worker_done = worker.owner.results.read_or_recover().is_ok_and(|g| g.rendered_count >= 1);
            if worker_done && repaints.load(Ordering::SeqCst) >= 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.owner.results.read_or_recover().is_ok_and(|g| g.rendered_count >= 1), "async worker must finish failed raster within 2s");
        assert!(repaints.load(Ordering::SeqCst) >= 1, "worker must wake ctx when job finishes");

        for pass in 0..10 {
            sim += 0.05;
            let before = repaints.load(Ordering::Relaxed);
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0))),
                    time: Some(sim),
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    composite_async_scene_texture(
                        ui,
                        &app,
                        CompositeSceneSpec { scene: &bad_scene, sheet_id: &bad_id, paper_rect: paper, slot_tag: "main", painter: ui.painter() },
                        &mut state,
                    );
                },
            );
            let delay = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
            out.textures_delta.clear();
            let after = repaints.load(Ordering::Relaxed);
            assert!(!state.slot.rendering_in_flight, "failed raster must not leave rendering_in_flight=true on pass {pass}");
            assert!(state.slot.last_raster_failed, "failed raster flag must be recorded");
            assert!(state.slot.cached_texture.is_none(), "stale texture from previous sheet must be cleared");
            if pass >= 2 {
                assert_eq!(after - before, 0, "idle frame {pass} after failed raster must emit zero repaints");
                assert_eq!(delay, Some(Duration::MAX));
            }
        }
    }
}
