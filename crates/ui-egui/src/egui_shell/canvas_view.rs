//! Native `egui` async `ChartScene` / `LayoutScene` viewport (`docs/planning/gui-layout-spec.md`
//! §8.11A & §8.12, `R15.3`).

use std::sync::{Arc, OnceLock};

use curvebook_chart::equation_box::resolve_model_equation_hit_bounds;
use curvebook_chart::{ChartScene, PathCmd, PointPt, RectPt, ScenePrimitive};
use curvebook_doc::{
    AnalysisKind, EquationDisplayMode, GraphAnnotation, GraphKind, GraphSheet, GraphicalDataObject, LayoutItem, LayoutSheet, ProjectDocument,
    RectPt as DocRectPt, ResultTabKind, SheetId,
};
use curvebook_fit::tex_format::{format_model_equation_tex, format_polynomial_equation_tex};
use curvebook_palette::{CanvasPreviewFilter, Rgba8};
use curvebook_render::{RasterImage, RenderTargetSpec, apply_canvas_preview_filter, render_scene};
use curvebook_text::{EquationSegment, SpanScript, SpanSlope, SpanWeight, parse_equation_segments};
use curvebook_tools::GridCellCoord;
use curvebook_tools::canvas::{PhysicalUnit, SnapGuide, SnapOrientation, SnapSource};
use egui::{Color32, CornerRadius, FontFamily, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::inspect_bridge::HybridHostTextureFrame;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, rgba8_to_color32};
use crate::layout::ZLayer;

const RULER_THICKNESS_PT: f32 = 20.0;
const BREADCRUMB_HEIGHT_PT: f32 = 28.0;

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
    pub last_sheet_id: Option<SheetId>,
    pub last_mutation_seq: u64,
    pub last_width_px: u32,
    pub last_height_px: u32,
    pub last_dpi: u16,
    pub last_filter_slug: String,
    pub rendering_in_flight: bool,
    pub layout_return_breadcrumb: Option<SheetId>,
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
}

#[derive(Clone)]
struct CachedCompiledScene {
    mutation_seq: u64,
    scene: Arc<ChartScene>,
}

#[derive(Clone)]
struct AsyncRasterJob {
    key: RasterRequestKey,
    output: Arc<OnceLock<Option<RasterImage>>>,
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
#[allow(clippy::too_many_arguments)]
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
    let show_rulers = app.canvas_dialogs.rulers_visible && !app.presentation_mode && mat_rect.width() > 96.0 && mat_rect.height() > 96.0;
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
    let zoom = (app.canvas_dialogs.canvas.zoom as f32).clamp(0.1, 8.0);
    let fit_scale =
        ((mat_rect.width() - 28.0).max(40.0) / scene_w_pt).min((mat_rect.height() - 28.0 - km_reserve_h).max(40.0) / scene_h_pt).clamp(0.2, 4.0);
    let eff_scale = fit_scale * zoom;
    let paper_size = vec2(scene_w_pt * eff_scale, scene_h_pt * eff_scale);
    let pan_x = app.canvas_dialogs.canvas.pan_pt.x as f32 + state.pan_pt[0];
    let pan_y = app.canvas_dialogs.canvas.pan_pt.y as f32 + state.pan_pt[1];
    let paper_center = mat_rect.center() + vec2(pan_x, pan_y - km_reserve_h * 0.4);
    let paper_rect = Rect::from_center_size(paper_center, paper_size);
    report.paper_rect_pt = Some(paper_rect);

    if let (Some(tr_r), Some(lr_r)) = (top_ruler_rect, left_ruler_rect) {
        render_canvas_rulers(ui, mat_rect, tr_r, lr_r, paper_rect, eff_scale, app.canvas_dialogs.ruler_unit, tokens);
    }

    // Paint paper drop shadow & white sheet backing inside clipped mat_rect
    let clipped_painter = ui.painter().with_clip_rect(mat_rect);
    clipped_painter.rect_filled(paper_rect.translate(vec2(2.0, 2.0)), CornerRadius::same(2), Color32::from_black_alpha(32));
    clipped_painter.rect_filled(paper_rect, CornerRadius::ZERO, tokens.paper_white);
    clipped_painter.rect_stroke(paper_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    if let Some(ref sc) = scene {
        composite_async_scene_texture(ui, app, sc, target_id, paper_rect, ctx_in.host_textures, slot_tag, state, &clipped_painter);
    }
    report.async_rendering_in_flight = state.slot.rendering_in_flight;

    if let Some(gs) = gs_opt
        && let Some(ref sc) = scene
    {
        render_graph_hud_overlays(ui, &clipped_painter, app, gs, sc, paper_rect, &resp, state, &ctx_in, meta, intents, &mut report);
    } else if let Some(ls) = ls_opt {
        render_layout_hud_overlays(
            ui,
            &clipped_painter,
            app,
            ls,
            scene.as_deref(),
            paper_rect,
            eff_scale,
            &resp,
            state,
            &ctx_in,
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

    report.hover_readout = state.hover_readout.clone();
    report
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

fn resolve_cached_scene(ctx: &egui::Context, app: &UiApp, sheet_id: &SheetId, slot_tag: &str) -> Option<Arc<ChartScene>> {
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

#[allow(clippy::too_many_arguments)]
fn composite_async_scene_texture(
    ui: &egui::Ui,
    app: &UiApp,
    scene: &Arc<ChartScene>,
    sheet_id: &SheetId,
    paper_rect: Rect,
    host_textures: Option<&HybridHostTextureFrame>,
    slot_tag: &str,
    state: &mut CanvasViewEguiState,
    painter: &egui::Painter,
) {
    if let Some(tex_id) = host_textures.and_then(|h| h.canvas_texture) {
        state.slot.cached_texture = Some(tex_id);
        state.slot.rendering_in_flight = false;
        painter.image(tex_id, paper_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        return;
    }

    let ppp = ui.ctx().pixels_per_point().clamp(1.0, 4.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let width_px = (paper_rect.width() * ppp).round().clamp(32.0, 4096.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height_px = (paper_rect.height() * ppp).round().clamp(32.0, 4096.0) as u32;
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
        && state.slot.cached_texture.is_some();

    if slot_matches && let Some(tex_id) = state.slot.cached_texture {
        state.slot.rendering_in_flight = false;
        painter.image(tex_id, paper_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        return;
    }

    let job_id = Id::unique(("egui_shell.canvas_async_job", slot_tag, sheet_id.as_str()));
    let existing_job: Option<AsyncRasterJob> = ui.ctx().data(|d| d.get_temp(job_id));
    let job = match existing_job {
        Some(j) if j.key == req_key => j,
        _ => {
            let output = Arc::new(OnceLock::new());
            let out_worker = output.clone();
            let scene_clone = scene.clone();
            let filter = app.preview_filter;
            let ctx_wake = ui.ctx().clone();
            std::thread::spawn(move || {
                let rendered = render_scene(&scene_clone, RenderTargetSpec { width_px, height_px, dpi }).ok().map(|mut img| {
                    if filter != CanvasPreviewFilter::Normal {
                        apply_canvas_preview_filter(&mut img, filter);
                    }
                    img
                });
                let _ = out_worker.set(rendered);
                ctx_wake.request_repaint();
            });
            let new_job = AsyncRasterJob { key: req_key.clone(), output };
            ui.ctx().data_mut(|d| d.insert_temp(job_id, new_job.clone()));
            new_job
        }
    };

    if let Some(maybe_img) = job.output.get() {
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
            painter.image(tex_id, paper_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            return;
        }
    } else {
        state.slot.rendering_in_flight = true;
        ui.ctx().request_repaint();
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

#[allow(clippy::too_many_arguments)]
fn render_graph_hud_overlays(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    app: &UiApp,
    gs: &GraphSheet,
    scene: &ChartScene,
    paper_rect: Rect,
    canvas_resp: &egui::Response,
    state: &mut CanvasViewEguiState,
    ctx_in: &CanvasViewContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CanvasViewFrameReport,
) {
    let doc = app.document();
    let tokens = ctx_in.tokens;
    let sx = paper_rect.width() / (scene.width_pt.max(1.0) as f32);
    let sy = paper_rect.height() / (scene.height_pt.max(1.0) as f32);
    let scene_to_screen = |pt: PointPt| pos2(paper_rect.left() + (pt.x as f32) * sx, paper_rect.top() + (pt.y as f32) * sy);
    let screen_to_scene =
        |pos: Pos2| PointPt::new(f64::from((pos.x - paper_rect.left()) / sx.max(1e-4)), f64::from((pos.y - paper_rect.top()) / sy.max(1e-4)));

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
            let sheet_title = doc.data_sheet(&hit.source_cell.sheet_id).map_or(hit.source_cell.sheet_id.as_str(), |d| d.title.as_str());
            let summary = format!("{sheet_title} · {} · Row {} · {}", hit.series_label, hit.source_cell.row + 1, hit.value_summary);
            let readout = CanvasHoverReadout {
                source_sheet_id: Some(hit.source_cell.sheet_id.clone()),
                point_index: Some(idx),
                row: Some(hit.source_cell.row),
                col: Some(hit.source_cell.col),
                label: hit.series_label.clone(),
                value_summary: summary.clone(),
            };
            state.hover_readout = Some(readout);
            let center = scene_to_screen(hit.center_pt);
            painter.circle_stroke(center, 7.0, Stroke::new(2.0, tokens.accent));
            paint_hover_hud_card(painter, center + vec2(10.0, -22.0), &summary, tokens);

            if ctx_in.enabled && canvas_resp.clicked() {
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

    // 3. Pairwise Significance Brackets & Compact Letter Display (CLD) HUD
    let has_pairwise_results = doc.result_sheets.iter().any(|r| !r.pairwise_comparisons.is_empty());
    if !gs.spec.comparisons.is_empty() || gs.spec.compact_letter_display.is_some() || has_pairwise_results {
        render_pairwise_cld_hud(ui, gs, doc, paper_rect, ctx_in.enabled, tokens, meta, intents);
    }

    // 4. Axis-Synchronized Kaplan-Meier `Number at risk` Alignment Track
    if should_show_km_track(gs, doc) {
        let track_rect = render_km_number_at_risk_track(ui, painter, gs, doc, scene, paper_rect, state, tokens, meta);
        report.km_track_rect_pt = track_rect;
    } else {
        state.km_at_risk_rows = 0;
    }

    // 5. Live TeX Curve/Fit Equations via `curvebook-text` layout helpers
    let eq_rects = render_live_tex_equations(ui, painter, gs, doc, scene, paper_rect, sx, sy, state, tokens, meta);
    report.tex_equation_rects_pt = eq_rects;
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

#[allow(clippy::too_many_arguments)]
fn render_pairwise_cld_hud(
    ui: &mut egui::Ui,
    gs: &GraphSheet,
    doc: &ProjectDocument,
    paper_rect: Rect,
    enabled: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
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

#[allow(clippy::too_many_arguments)]
fn render_km_number_at_risk_track(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    gs: &GraphSheet,
    doc: &ProjectDocument,
    scene: &ChartScene,
    paper_rect: Rect,
    state: &mut CanvasViewEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) -> Option<Rect> {
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
        FontId::proportional(10.5),
        tokens.text_strong,
    );

    // Paint tick headers synchronized directly to X-axis tick screen X coordinates
    for &(t_val, x_pt) in &ticks {
        let tx = paper_rect.left() + (x_pt as f32) * sx;
        painter.text(
            pos2(tx, track_rect.top() + 10.0),
            egui::Align2::CENTER_CENTER,
            format!("{t_val:.4}").trim_end_matches('0').trim_end_matches('.'),
            FontId::monospace(10.0),
            tokens.text_muted,
        );
    }

    for (r_idx, row) in rows.iter().enumerate() {
        let ry = track_rect.top() + 24.0 + (r_idx as f32) * row_h;
        painter.text(
            pos2(label_right.max(track_rect.left() + 82.0), ry),
            egui::Align2::RIGHT_CENTER,
            &row.label,
            FontId::proportional(10.5),
            row.color,
        );
        for (&(_, x_pt), &count) in ticks.iter().zip(row.counts.iter()) {
            let tx = paper_rect.left() + (x_pt as f32) * sx;
            painter.text(pos2(tx, ry), egui::Align2::CENTER_CENTER, format!("{count}"), FontId::monospace(10.5), tokens.text_primary);
        }
    }

    let track_resp = ui.allocate_rect(track_rect, Sense::hover());
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
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill } => {
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

#[allow(clippy::too_many_arguments)]
fn render_live_tex_equations(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    gs: &GraphSheet,
    doc: &ProjectDocument,
    scene: &ChartScene,
    paper_rect: Rect,
    sx: f32,
    sy: f32,
    state: &mut CanvasViewEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) -> Vec<Rect> {
    let mut rects = Vec::new();
    let mut eq_idx = 0usize;

    for ann in &gs.spec.annotations {
        let GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered } = ann else {
            continue;
        };
        let Some(tex) = resolve_live_equation_tex(doc, result_sheet_id, *data_set_index, *mode) else {
            continue;
        };
        let segments = parse_equation_segments(&tex);
        if segments.is_empty() {
            continue;
        }
        let eff_bounds = resolve_model_equation_hit_bounds(scene, *bounds_pt, *font_size_pt, *bordered).unwrap_or_else(|| RectPt::from(*bounds_pt));
        let origin = pos2(paper_rect.left() + (eff_bounds.x as f32) * sx + 4.0, paper_rect.top() + (eff_bounds.y as f32) * sy + 4.0);
        let scaled_font = (*font_size_pt as f32 * sx).clamp(9.0, 26.0);
        let rendered = paint_equation_segments(ui, painter, origin, &segments, scaled_font, tokens.text_strong);
        let box_rect = rendered.expand(4.0);
        if *bordered {
            painter.rect_stroke(box_rect, CornerRadius::same(2), Stroke::new(1.0, tokens.border), StrokeKind::Inside);
        }
        let wid = format!("canvas.tex_equation.{eq_idx}");
        let resp = ui.allocate_rect(box_rect, Sense::hover());
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

fn measure_equation_segments(ui: &egui::Ui, segments: &[EquationSegment], font_size: f32) -> (f32, f32) {
    let mut total_w = 0.0_f32;
    let mut max_h = font_size * 1.15;
    for seg in segments {
        match seg {
            EquationSegment::Inline(rich) => {
                for span in &rich.spans {
                    let scale = match span.script {
                        SpanScript::Baseline => 1.0,
                        SpanScript::Superscript | SpanScript::Subscript => 0.72,
                    };
                    let font = FontId::new((font_size * scale).max(7.0), FontFamily::Proportional);
                    let galley = ui.fonts_mut(|f| f.layout_no_wrap(span.text.clone(), font, Color32::BLACK));
                    total_w += galley.size().x;
                    max_h = max_h.max(galley.size().y);
                }
            }
            EquationSegment::Fraction { numerator, denominator } => {
                let (nw, nh) = measure_equation_segments(ui, numerator, font_size * 0.85);
                let (dw, dh) = measure_equation_segments(ui, denominator, font_size * 0.85);
                total_w += nw.max(dw) + 8.0;
                max_h = max_h.max(nh + dh + 6.0);
            }
        }
    }
    (total_w, max_h)
}

fn paint_equation_segments(
    ui: &egui::Ui,
    painter: &egui::Painter,
    origin: Pos2,
    segments: &[EquationSegment],
    font_size: f32,
    color: Color32,
) -> Rect {
    let (total_w, total_h) = measure_equation_segments(ui, segments, font_size);
    let cy = origin.y + total_h * 0.5;
    let mut cx = origin.x;

    for seg in segments {
        match seg {
            EquationSegment::Inline(rich) => {
                for span in &rich.spans {
                    let (scale, dy) = match span.script {
                        SpanScript::Baseline => (1.0, 0.0),
                        SpanScript::Superscript => (0.72, -font_size * 0.28),
                        SpanScript::Subscript => (0.72, font_size * 0.22),
                    };
                    let mut job = egui::text::LayoutJob::single_section(
                        span.text.clone(),
                        egui::text::TextFormat {
                            font_id: FontId::new((font_size * scale).max(7.0), FontFamily::Proportional),
                            color,
                            italics: span.slope == SpanSlope::Italic,
                            extra_letter_spacing: if span.weight == SpanWeight::Bold { 0.2 } else { 0.0 },
                            ..Default::default()
                        },
                    );
                    job.wrap = egui::text::TextWrapping::no_max_width();
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let w = galley.size().x;
                    let h = galley.size().y;
                    painter.galley(pos2(cx, cy - h * 0.5 + dy), galley, color);
                    cx += w;
                }
            }
            EquationSegment::Fraction { numerator, denominator } => {
                let sub_size = (font_size * 0.85).max(7.0);
                let (nw, nh) = measure_equation_segments(ui, numerator, sub_size);
                let (dw, dh) = measure_equation_segments(ui, denominator, sub_size);
                let frac_w = nw.max(dw) + 6.0;
                paint_equation_segments(ui, painter, pos2(cx + (frac_w - nw) * 0.5, cy - nh - 2.0), numerator, sub_size, color);
                painter.line_segment([pos2(cx + 1.0, cy), pos2(cx + frac_w - 1.0, cy)], Stroke::new(1.0, color));
                paint_equation_segments(ui, painter, pos2(cx + (frac_w - dw) * 0.5, cy + 2.0), denominator, sub_size, color);
                let _ = dh;
                cx += frac_w + 2.0;
            }
        }
    }

    Rect::from_min_size(origin, vec2(total_w.max(1.0), total_h.max(1.0)))
}

#[allow(clippy::too_many_arguments)]
fn render_layout_hud_overlays(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    app: &UiApp,
    ls: &LayoutSheet,
    scene: Option<&ChartScene>,
    paper_rect: Rect,
    eff_scale: f32,
    _canvas_resp: &egui::Response,
    state: &mut CanvasViewEguiState,
    ctx_in: &CanvasViewContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    report: &mut CanvasViewFrameReport,
) {
    let doc = app.document();
    let tokens = ctx_in.tokens;
    let scene_w = scene.map_or(ls.spec.page_size_pt().width.max(1.0), |s| s.width_pt.max(1.0)) as f32;
    let scene_h = scene.map_or(ls.spec.page_size_pt().height.max(1.0), |s| s.height_pt.max(1.0)) as f32;
    let sx = paper_rect.width() / scene_w;
    let sy = paper_rect.height() / scene_h;
    let _ = eff_scale;

    let mut panel_rects = Vec::with_capacity(ls.spec.items.len());
    for (idx, item) in ls.spec.items.iter().enumerate() {
        let mut bounds = layout_item_bounds_doc(item);
        if let Some(drag) = &state.layout_drag
            && drag.sheet_id == ls.id
            && drag.item_index == idx
        {
            bounds = drag.current_bounds_pt;
        }
        let r = Rect::from_min_size(
            pos2(paper_rect.left() + (bounds.x as f32) * sx, paper_rect.top() + (bounds.y as f32) * sy),
            vec2((bounds.width as f32) * sx, (bounds.height as f32) * sy),
        );
        panel_rects.push((idx, item, bounds, r));
    }

    // Sibling inner plot-axis frame offsets for 0.1 pt alignment snapping
    let plot_origins: Vec<(usize, PointPt)> = ls
        .spec
        .items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            if let LayoutItem::LiveGraph { graph_sheet_id, .. } = item {
                let p_off = app
                    .session()
                    .render_sheet_scene(graph_sheet_id)
                    .ok()
                    .map(|gs_scene| PointPt::new(gs_scene.plot_rect_pt.x, gs_scene.plot_rect_pt.y))?;
                Some((idx, p_off))
            } else {
                None
            }
        })
        .collect();

    for (idx, item, bounds, r) in panel_rects {
        let selected = state.selected_panel_index == Some(idx);
        let p_resp =
            ui.interact(r, Id::unique(("layout_panel", ls.id.as_str(), idx)), if ctx_in.enabled { Sense::click_and_drag() } else { Sense::hover() });

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
            let chip_rect = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(26.0, 18.0));
            painter.rect_filled(chip_rect, CornerRadius::same(2), tokens.accent);
            painter.text(chip_rect.center(), egui::Align2::CENTER_CENTER, &badge, FontId::proportional(10.0), tokens.paper_white);
        }

        let wid = format!("canvas.layout_panel.{idx}");
        meta.record_response(
            ui,
            &p_resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "button",
                label: &format!("Layout Panel {badge}"),
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
            if (p_resp.double_clicked() || p_resp.triple_clicked())
                && let LayoutItem::LiveGraph { graph_sheet_id, .. } = item
                && doc.graph_sheet(graph_sheet_id).is_some()
            {
                state.slot.layout_return_breadcrumb = Some(ls.id.clone());
                intents.push_engine("sheet.select", json!({ "sheet_id": graph_sheet_id.as_str() }));
                return;
            }
            if p_resp.clicked() {
                state.selected_panel_index = Some(idx);
            }
            if p_resp.drag_started() {
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
            {
                let ptr = ui.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos())).unwrap_or(r.center());
                let scene_pt =
                    PointPt::new(f64::from((ptr.x - paper_rect.left()) / sx.max(1e-4)), f64::from((ptr.y - paper_rect.top()) / sy.max(1e-4)));
                let dx = scene_pt.x - drag.start_scene_pt[0];
                let dy = scene_pt.y - drag.start_scene_pt[1];
                let raw_x = round_tenth_pt(drag.initial_bounds_pt.x + dx);
                let raw_y = round_tenth_pt(drag.initial_bounds_pt.y + dy);
                let cand = curvebook_chart::RectPt::new(raw_x, raw_y, drag.initial_bounds_pt.width, drag.initial_bounds_pt.height);
                let (snapped, guides) = snap_panel_with_tenth_precision(app, ls, idx, cand, &plot_origins);
                drag.current_bounds_pt = DocRectPt::new(snapped.x, snapped.y, snapped.width, snapped.height);
                state.active_snap_guides = guides;
                state.last_snap_readout = Some(format!("Snap: X={:.1} pt, Y={:.1} pt (0.1 pt)", snapped.x, snapped.y));
            }
            if p_resp.drag_stopped()
                && let Some(drag) = state.layout_drag.take()
                && drag.item_index == idx
            {
                let mut updated_items = ls.spec.items.clone();
                if let Some(target_item) = updated_items.get_mut(idx) {
                    set_layout_item_bounds_doc(target_item, drag.current_bounds_pt);
                    intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "spec": { "items": updated_items } }));
                }
                state.active_snap_guides.clear();
            }
        }
    }

    // Draw active 0.1 pt snap guides (both internal live drag & CanvasToolState active_snap_guides)
    let guides_to_draw = if !state.active_snap_guides.is_empty() { &state.active_snap_guides } else { &app.canvas_dialogs.canvas.active_snap_guides };
    report.snap_guide_count = guides_to_draw.len();
    for guide in guides_to_draw {
        let col = if guide.source == SnapSource::PlotOrigin { Color32::from_rgb(220, 38, 38) } else { tokens.accent };
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
        LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => *bounds_pt,
    }
}

fn set_layout_item_bounds_doc(item: &mut LayoutItem, bounds: DocRectPt) {
    match item {
        LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
            *bounds_pt = bounds;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_canvas_rulers(
    ui: &egui::Ui,
    mat_rect: Rect,
    top_r: Rect,
    left_r: Rect,
    paper_rect: Rect,
    eff_scale: f32,
    unit: PhysicalUnit,
    tokens: &Tokens,
) {
    let corner = Rect::from_min_size(pos2(left_r.left(), top_r.top()), vec2(RULER_THICKNESS_PT, RULER_THICKNESS_PT));
    let unit_lbl = match unit {
        PhysicalUnit::Points => "pt",
        PhysicalUnit::Inches => "in",
        PhysicalUnit::Millimeters => "mm",
    };
    ui.painter().rect_filled(corner, CornerRadius::ZERO, tokens.header_bg);
    ui.painter().rect_filled(top_r, CornerRadius::ZERO, tokens.header_bg);
    ui.painter().rect_filled(left_r, CornerRadius::ZERO, tokens.header_bg);
    ui.painter().text(corner.center(), egui::Align2::CENTER_CENTER, unit_lbl, FontId::proportional(9.5), tokens.text_muted);

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
            ui.painter().line_segment([pos2(x, top_r.bottom() - 6.0), pos2(x, top_r.bottom())], Stroke::new(1.0, tokens.border));
            let val = match unit {
                PhysicalUnit::Points => tick_idx * 72,
                PhysicalUnit::Inches => tick_idx,
                PhysicalUnit::Millimeters => tick_idx * 10,
            };
            ui.painter().text(pos2(x + 2.0, top_r.top() + 3.0), egui::Align2::LEFT_TOP, format!("{val}"), FontId::monospace(9.0), tokens.text_muted);
        }
        x += step_px;
        tick_idx += 1;
    }

    let mut y = paper_rect.top();
    let mut y_idx = 0i32;
    while y <= mat_rect.bottom() && y_idx < 32 {
        if y >= left_r.top() {
            ui.painter().line_segment([pos2(left_r.right() - 6.0, y), pos2(left_r.right(), y)], Stroke::new(1.0, tokens.border));
            let val = match unit {
                PhysicalUnit::Points => y_idx * 72,
                PhysicalUnit::Inches => y_idx,
                PhysicalUnit::Millimeters => y_idx * 10,
            };
            ui.painter().text(
                pos2(left_r.left() + 2.0, y + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{val}"),
                FontId::monospace(9.0),
                tokens.text_muted,
            );
        }
        y += step_px;
        y_idx += 1;
    }
}
