//! Deterministic software frame compositor (`RasterImage` + PNG for `ui.screenshot`),
//! physical/RTL viewport layout mapping ([`WorkspaceLayoutMap`]), Split View panes (`QOL-4`),
//! two-way point brushing (`QOL-5`), Assumption Diagnostic banner (`QOL-7`), preflight badge (`QOL-8`),
//! non-destructive CVD/grayscale viewport filter (`QOL-9`), and presentation mode (`QOL-10`).

use lumencraft_chart::{ChartScene, PointPt, preflight_check_scene};
use lumencraft_doc::{
    ChecklistStatus, DataSheet, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, LinkBorderColor, ProjectDocument, ResultSheet, SheetFolderKind,
    SheetId, SheetSection, TableArchetype, XColumnMode,
};
use lumencraft_palette::{CanvasPreviewFilter, Rgba8};
use lumencraft_render::{RasterImage, RenderError, RenderTargetSpec, apply_canvas_preview_filter, encode_png, render_scene};
use lumencraft_text::{TextDirection, reorder_visual_bidi};
use lumencraft_tools::canvas::{CanvasHitTarget, SnapOrientation, SnapSource};
use lumencraft_tools::{GridCellCoord, PhysicalUnit, build_ruler_ticks};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::dialogs_canvas::{
    ActiveDialog, ActiveInspector, AssumptionDiagnosticBanner, ResultSheetViewState, data_column_title, format_compact_f64, variable_role_color,
};
use crate::grid_view::{format_cell_display, resolve_flat_coord};
use crate::interaction::{
    CanvasDragTarget, GridHitGeometry, InfoRowHitRect, InlineEditorTarget, NavigatorEntryHitRect, ScenePaperMapping, ViewportMetrics,
    WorkspaceLayoutMap,
};
use crate::workspace_nav::{ActiveWorkspacePane, PortfolioCard};

/// Integer pixel rectangle inside a composited workspace frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct WorkspacePixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl WorkspacePixelRect {
    #[must_use]
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    #[must_use]
    pub const fn right(self) -> u32 {
        self.x.saturating_add(self.width)
    }

    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y.saturating_add(self.height)
    }

    #[must_use]
    pub const fn contains(self, px: u32, py: u32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

/// Partitioned pixel geometry of every chrome and canvas region in a workspace screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkspaceFrameLayout {
    pub total_width: u32,
    pub total_height: u32,
    pub toolbar: WorkspacePixelRect,
    pub navigator: WorkspacePixelRect,
    pub canvas: WorkspacePixelRect,
    pub inspector: WorkspacePixelRect,
    pub status_bar: WorkspacePixelRect,
    pub modal_overlay: Option<WorkspacePixelRect>,
}

#[derive(Clone, Copy)]
pub(crate) struct FramePalette {
    pub(crate) panel: Rgba8,
    pub(crate) surface: Rgba8,
    pub(crate) text: Rgba8,
    pub(crate) muted: Rgba8,
    pub(crate) accent: Rgba8,
    pub(crate) header: Rgba8,
    pub(crate) border: Rgba8,
    pub(crate) canvas_mat: Rgba8,
    pub(crate) excluded: Rgba8,
    pub(crate) link_blue: Rgba8,
    pub(crate) link_green: Rgba8,
    pub(crate) link_red: Rgba8,
}

impl FramePalette {
    pub(crate) fn from_theme(theme: &crate::theme::UiThemeSpec) -> Self {
        let is_dark = theme.surface_bg.relative_luminance() < 0.25;
        let delta = if is_dark { 14 } else { -12 };
        let mat_delta = if is_dark { -8 } else { -18 };
        Self {
            panel: theme.panel_bg,
            surface: theme.surface_bg,
            text: theme.text_primary,
            muted: theme.text_muted,
            accent: theme.accent,
            header: shift_luma(theme.panel_bg, delta),
            border: theme.border,
            canvas_mat: shift_luma(theme.surface_bg, mat_delta),
            excluded: theme.excluded_cell_text,
            link_blue: theme.link_blue,
            link_green: theme.link_green,
            link_red: theme.link_red,
        }
    }
}

fn shift_luma(color: Rgba8, delta: i16) -> Rgba8 {
    let shift = |ch: u8| -> u8 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let c = i16::from(ch).saturating_add(delta).clamp(0, 255) as u8;
        c
    };
    Rgba8::rgba(shift(color.r), shift(color.g), shift(color.b), color.a)
}

/// Clamp `value` into `[min, max]` without panicking when `min > max`.
const fn clamp_u32_safe(value: u32, min: u32, max: u32) -> u32 {
    let lo = if min < max { min } else { max };
    if value < lo {
        lo
    } else if value > max {
        max
    } else {
        value
    }
}

/// Clamp `value` into `[min, max]` without panicking when bounds invert or include `NaN`.
fn clamp_f64_safe(value: f64, min: f64, max: f64) -> f64 {
    if min.is_nan() && max.is_nan() {
        return if value.is_nan() { 0.0 } else { value };
    }
    if min.is_nan() {
        return if value.is_nan() { max } else { value.min(max) };
    }
    if max.is_nan() {
        return if value.is_nan() { min } else { value.max(min) };
    }
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    if value.is_nan() { lo } else { value.clamp(lo, hi) }
}

/// Compute deterministic UI region bounds for a frame of `(width_px, height_px)`.
///
/// Mirrors horizontal Navigator / Canvas / Inspector panes via [`crate::i18n::mirror_x_for_rtl`]
/// when `app.locale` resolves to a right-to-left script (`ar`, `he`, `fa`, `ur`).
/// When `app.presentation_mode` is enabled (`QOL-10`), chrome regions collapse so the figure
/// occupies the entire viewport.
#[must_use]
pub fn compute_workspace_layout(app: &UiApp, width_px: u32, height_px: u32) -> WorkspaceFrameLayout {
    if app.presentation_mode {
        return WorkspaceFrameLayout {
            total_width: width_px,
            total_height: height_px,
            toolbar: WorkspacePixelRect::new(0, 0, width_px, 0),
            navigator: WorkspacePixelRect::new(0, 0, 0, height_px),
            canvas: WorkspacePixelRect::new(0, 0, width_px, height_px),
            inspector: WorkspacePixelRect::new(width_px, 0, 0, height_px),
            status_bar: WorkspacePixelRect::new(0, height_px, width_px, 0),
            modal_overlay: None,
        };
    }

    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    let toolbar_h = clamp_u32_safe(height_px / 14, 20, 38).min(height_px);
    let status_h = clamp_u32_safe(height_px / 22, 16, 26).min(height_px.saturating_sub(toolbar_h));
    let mid_y = toolbar_h;
    let mid_h = height_px.saturating_sub(toolbar_h).saturating_sub(status_h);

    let nav_w = if app.nav.navigator_open && width_px >= 320 { clamp_u32_safe(width_px / 5, 136, 240) } else { 0 };
    let insp_w = if app.nav.inspector_open && width_px >= 420 { clamp_u32_safe(width_px / 4, 176, 280) } else { 0 };
    let canvas_w = width_px.saturating_sub(nav_w).saturating_sub(insp_w);

    let nav_x = crate::i18n::mirror_x_for_rtl(0, nav_w, width_px, is_rtl);
    let canvas_x = crate::i18n::mirror_x_for_rtl(nav_w, canvas_w, width_px, is_rtl);
    let insp_x = crate::i18n::mirror_x_for_rtl(nav_w.saturating_add(canvas_w), insp_w, width_px, is_rtl);

    let modal_overlay = if app.canvas_dialogs.active_dialog != ActiveDialog::None {
        let max_mw = width_px.saturating_sub(24).max(1).min(width_px);
        let max_mh = height_px.saturating_sub(24).max(1).min(height_px);
        let mw = clamp_u32_safe(width_px.saturating_mul(3) / 5, 180, max_mw);
        let mh = clamp_u32_safe(height_px.saturating_mul(3) / 5, 120, max_mh);
        let mx = crate::i18n::mirror_x_for_rtl((width_px.saturating_sub(mw)) / 2, mw, width_px, is_rtl);
        Some(WorkspacePixelRect::new(mx, (height_px.saturating_sub(mh)) / 2, mw, mh))
    } else {
        None
    };

    WorkspaceFrameLayout {
        total_width: width_px,
        total_height: height_px,
        toolbar: WorkspacePixelRect::new(0, 0, width_px, toolbar_h),
        navigator: WorkspacePixelRect::new(nav_x, mid_y, nav_w, mid_h),
        canvas: WorkspacePixelRect::new(canvas_x, mid_y, canvas_w, mid_h),
        inspector: WorkspacePixelRect::new(insp_x, mid_y, insp_w, mid_h),
        status_bar: WorkspacePixelRect::new(0, toolbar_h.saturating_add(mid_h), width_px, status_h),
        modal_overlay,
    }
}

/// Compute the full physical-pixel layout & hit-testing map for `app` under `metrics` (`G6`, `M7`).
#[must_use]
pub fn compute_workspace_layout_map(app: &UiApp, metrics: ViewportMetrics) -> WorkspaceLayoutMap {
    let width_px = metrics.physical_width_px();
    let height_px = metrics.physical_height_px();
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    let frame = compute_workspace_layout(app, width_px, height_px);
    let navigator_entries =
        if !app.presentation_mode && frame.navigator.width > 0 { compute_navigator_hit_rects(frame.navigator, app) } else { Vec::new() };

    let viewport_rect = center_viewport_rect(frame.canvas, app);
    let (split_primary, split_divider, split_secondary) =
        if app.split.enabled { compute_split_panes(viewport_rect, app.split.ratio, is_rtl) } else { (None, None, None) };

    let mut map = WorkspaceLayoutMap {
        metrics,
        is_rtl,
        frame,
        navigator_entries,
        split_primary,
        split_divider,
        split_secondary,
        grid_geometry: None,
        scene_paper: None,
        result_banner_rect: None,
        result_banner_sheet_id: None,
        canvas_title_rect: None,
        x_axis_title_rect: None,
        y_axis_title_rect: None,
        info_constant_rows: Vec::new(),
        info_notes_rect: None,
        info_sheet_id: None,
    };

    if app.nav.gallery.open {
        return map;
    }

    if app.split.enabled
        && let (Some(prim_rect), Some(sec_rect)) = (split_primary, split_secondary)
    {
        let (prim_id, sec_id) = app.split.resolve_sheets(app.document());
        if let Some(pid) = &prim_id {
            populate_pane_map(&mut map, app, pid, prim_rect, is_rtl);
        }
        if let Some(sid) = &sec_id {
            populate_pane_map(&mut map, app, sid, sec_rect, is_rtl);
        }
    } else if let Some(active_id) = &app.document().active_sheet_id {
        populate_pane_map(&mut map, app, active_id, viewport_rect, is_rtl);
    }

    map
}

fn populate_pane_map(map: &mut WorkspaceLayoutMap, app: &UiApp, sheet_id: &SheetId, rect: WorkspacePixelRect, is_rtl: bool) {
    let doc = app.document();
    if doc.graph_sheet(sheet_id).is_some() || doc.layout_sheet(sheet_id).is_some() {
        let geom = compute_scene_viewport_geometry(rect, app, sheet_id, is_rtl);
        map.scene_paper = Some(geom.paper_mapping);
        map.canvas_title_rect = geom.title_rect;
        map.x_axis_title_rect = geom.x_axis_title_rect;
        map.y_axis_title_rect = geom.y_axis_title_rect;
    } else if let Some(ds) = doc.data_sheet(sheet_id) {
        map.grid_geometry = Some(compute_grid_geometry(rect, ds, app, is_rtl));
    } else if let Some(rs) = doc.result_sheet(sheet_id) {
        if let Some(banner_rect) = compute_result_banner_rect(rect, rs) {
            map.result_banner_rect = Some(banner_rect);
            map.result_banner_sheet_id = Some(rs.id.clone());
        }
    } else if let Some(info) = doc.info_sheet(sheet_id) {
        let info_geom = compute_info_geometry(rect, info, is_rtl);
        map.info_constant_rows = info_geom.rows;
        map.info_notes_rect = Some(info_geom.notes_rect);
        map.info_sheet_id = Some(info.id.clone());
    }
}

fn center_viewport_rect(canvas: WorkspacePixelRect, app: &UiApp) -> WorkspacePixelRect {
    if !app.presentation_mode && app.canvas_dialogs.rulers_visible && canvas.width >= 48 && canvas.height >= 48 {
        let ruler_h = 16u32;
        let ruler_w = 24u32;
        WorkspacePixelRect::new(
            canvas.x.saturating_add(ruler_w),
            canvas.y.saturating_add(ruler_h),
            canvas.width.saturating_sub(ruler_w),
            canvas.height.saturating_sub(ruler_h),
        )
    } else {
        canvas
    }
}

fn compute_split_panes(
    rect: WorkspacePixelRect,
    ratio: f64,
    is_rtl: bool,
) -> (Option<WorkspacePixelRect>, Option<WorkspacePixelRect>, Option<WorkspacePixelRect>) {
    let divider_w = 4u32;
    if rect.width < 120 || rect.height < 24 {
        return (None, None, None);
    }
    let usable = rect.width.saturating_sub(divider_w);
    let clamped = if ratio.is_finite() { clamp_f64_safe(ratio, 0.2, 0.8) } else { 0.5 };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let raw_primary = (f64::from(usable) * clamped).round() as u32;
    let primary_w = raw_primary.max(24).min(usable.saturating_sub(24));
    let secondary_w = usable.saturating_sub(primary_w);

    let px_local = crate::i18n::mirror_x_for_rtl(0, primary_w, rect.width, is_rtl);
    let dx_local = crate::i18n::mirror_x_for_rtl(primary_w, divider_w, rect.width, is_rtl);
    let sx_local = crate::i18n::mirror_x_for_rtl(primary_w.saturating_add(divider_w), secondary_w, rect.width, is_rtl);

    (
        Some(WorkspacePixelRect::new(rect.x.saturating_add(px_local), rect.y, primary_w, rect.height)),
        Some(WorkspacePixelRect::new(rect.x.saturating_add(dx_local), rect.y, divider_w, rect.height)),
        Some(WorkspacePixelRect::new(rect.x.saturating_add(sx_local), rect.y, secondary_w, rect.height)),
    )
}

fn section_to_folder(section: SheetSection) -> SheetFolderKind {
    match section {
        SheetSection::Data => SheetFolderKind::Data,
        SheetSection::Info => SheetFolderKind::Info,
        SheetSection::Analyses => SheetFolderKind::Results,
        SheetSection::Graphs => SheetFolderKind::Graphs,
        SheetSection::Layouts => SheetFolderKind::Layouts,
    }
}

fn compute_navigator_hit_rects(rect: WorkspacePixelRect, app: &UiApp) -> Vec<NavigatorEntryHitRect> {
    let list_bottom = rect.y + (rect.height * 2 / 3);
    let mut y = rect.y + 24;
    let mut counts = [0usize; 5];
    let mut out = Vec::new();
    for entry in &app.nav.entries {
        if y + 18 >= list_bottom {
            break;
        }
        let folder = section_to_folder(entry.section);
        let bucket = match folder {
            SheetFolderKind::Data => 0,
            SheetFolderKind::Info => 1,
            SheetFolderKind::Results => 2,
            SheetFolderKind::Graphs => 3,
            SheetFolderKind::Layouts => 4,
        };
        let index_in_folder = counts[bucket];
        counts[bucket] += 1;

        let row = WorkspacePixelRect::new(rect.x + 6, y, rect.width.saturating_sub(12), 16);
        out.push(NavigatorEntryHitRect { sheet_id: entry.sheet_id.clone(), folder, index_in_folder, rect: row });
        y = y.saturating_add(18);
    }
    out
}

struct SceneViewportGeometry {
    paper: WorkspacePixelRect,
    title_rect: Option<WorkspacePixelRect>,
    x_axis_title_rect: Option<WorkspacePixelRect>,
    y_axis_title_rect: Option<WorkspacePixelRect>,
    paper_mapping: ScenePaperMapping,
}

fn compute_scene_viewport_geometry(rect: WorkspacePixelRect, app: &UiApp, sheet_id: &SheetId, is_rtl: bool) -> SceneViewportGeometry {
    let pad = 16u32.min(rect.width / 8).min(rect.height / 8);
    let header_reserve = if app.presentation_mode { 0 } else { 20 };
    let inner_w = rect.width.saturating_sub(pad * 2).max(1);
    let inner_h = rect.height.saturating_sub(pad * 2 + header_reserve).max(1);
    let paper = WorkspacePixelRect::new(rect.x + pad, rect.y + pad + header_reserve.saturating_sub(2), inner_w, inner_h);

    let title_rect =
        if app.presentation_mode { None } else { Some(WorkspacePixelRect::new(rect.x + pad, rect.y + 4, rect.width.saturating_sub(pad * 2), 16)) };

    let (x_axis_title_rect, y_axis_title_rect) =
        if !app.presentation_mode && app.document().graph_sheet(sheet_id).is_some() && paper.width >= 60 && paper.height >= 60 {
            let xr = WorkspacePixelRect::new(paper.x + paper.width / 4, paper.bottom().saturating_sub(24), (paper.width / 2).max(16), 20);
            let y_local_x = crate::i18n::mirror_x_for_rtl(4, 24, paper.width, is_rtl);
            let yr = WorkspacePixelRect::new(paper.x.saturating_add(y_local_x), paper.y + paper.height / 4, 24, (paper.height / 2).max(16));
            (Some(xr), Some(yr))
        } else {
            (None, None)
        };

    let (scene_width_pt, scene_height_pt) = if let Ok(scene) = app.session().render_sheet_scene(sheet_id) {
        (scene.width_pt.max(1.0), scene.height_pt.max(1.0))
    } else if let Some(ls) = app.document().layout_sheet(sheet_id) {
        let p = ls.spec.page_size_pt();
        (p.width.max(1.0), p.height.max(1.0))
    } else {
        (360.0, 260.0)
    };

    let (zoom, pan_pt) = app.canvas_dialogs.effective_zoom_and_pan();
    SceneViewportGeometry {
        paper,
        title_rect,
        x_axis_title_rect,
        y_axis_title_rect,
        paper_mapping: ScenePaperMapping { sheet_id: sheet_id.clone(), paper, scene_width_pt, scene_height_pt, zoom, pan_pt },
    }
}

fn compute_grid_geometry(rect: WorkspacePixelRect, ds: &DataSheet, app: &UiApp, is_rtl: bool) -> GridHitGeometry {
    let sheet_rect = WorkspacePixelRect::new(rect.x + 8, rect.y + 8, rect.width.saturating_sub(16), rect.height.saturating_sub(16));
    let frame = app.grid.tool.format_visible_window(ds);
    let row_h = 18u32;
    let hdr_y = sheet_rect.y + 22;
    let body_y = hdr_y + row_h + 4;
    let col_w = 72u32;
    let gutter_w = 34u32;
    let (x_subcols, x_col_w) = match &ds.archetype {
        TableArchetype::Xy(xy) => (xy.x_mode.subcolumn_count().max(1), 60u32),
        TableArchetype::Survival(_) => (1, 60u32),
        _ => (0, 0),
    };
    let x_total_w = u32::try_from(x_subcols).unwrap_or(0).saturating_mul(x_col_w);

    let max_cols = (sheet_rect.width.saturating_sub(gutter_w + x_total_w + 8) / col_w).max(1) as usize;
    let max_rows = (sheet_rect.bottom().saturating_sub(body_y) / row_h) as usize;
    let visible_rows = max_rows.min(frame.window.end_row.saturating_sub(frame.window.start_row));
    let end_row = frame.window.start_row.saturating_add(visible_rows);

    let mut visible_cols = Vec::new();
    for flat_col in (frame.window.start_col..frame.window.end_col).take(max_cols) {
        let (col, subcol) = resolve_flat_coord(&ds.archetype, flat_col).unwrap_or((flat_col, 0));
        visible_cols.push((flat_col, GridCellCoord::new(0, col, subcol)));
    }

    GridHitGeometry {
        sheet_id: ds.id.clone(),
        sheet_rect,
        hdr_y,
        body_y,
        row_h,
        gutter_w,
        col_w,
        x_subcols,
        x_col_w,
        start_row: frame.window.start_row,
        end_row,
        start_col: frame.window.start_col,
        visible_cols,
        is_rtl,
    }
}

fn compute_result_banner_rect(rect: WorkspacePixelRect, rs: &ResultSheet) -> Option<WorkspacePixelRect> {
    let _banner = AssumptionDiagnosticBanner::from_result(rs)?;
    let box_rect = WorkspacePixelRect::new(rect.x + 8, rect.y + 8, rect.width.saturating_sub(16), rect.height.saturating_sub(16));
    if box_rect.width < 80 || box_rect.height < 90 {
        return None;
    }
    Some(WorkspacePixelRect::new(box_rect.x + 10, box_rect.y + 44, box_rect.width.saturating_sub(20), 18))
}

struct InfoSheetGeometry {
    rows: Vec<InfoRowHitRect>,
    notes_rect: WorkspacePixelRect,
}

fn compute_info_geometry(rect: WorkspacePixelRect, info: &InfoSheet, is_rtl: bool) -> InfoSheetGeometry {
    let box_rect = WorkspacePixelRect::new(rect.x + 8, rect.y + 8, rect.width.saturating_sub(16), rect.height.saturating_sub(16));
    let half_w = box_rect.width / 2;
    let const_local_x = crate::i18n::mirror_x_for_rtl(0, half_w, box_rect.width, is_rtl);
    let notes_local_x = crate::i18n::mirror_x_for_rtl(half_w, box_rect.width.saturating_sub(half_w), box_rect.width, is_rtl);

    let const_pane_x = box_rect.x.saturating_add(const_local_x);
    let notes_pane_x = box_rect.x.saturating_add(notes_local_x);

    let mut rows = Vec::new();
    let mut y = box_rect.y + 28;
    for c in info.constants.iter().take(8) {
        if y + 14 >= box_rect.bottom() {
            break;
        }
        rows.push(InfoRowHitRect {
            sheet_id: info.id.clone(),
            name: c.name.clone(),
            value_text: c.value.display_text(),
            rect: WorkspacePixelRect::new(const_pane_x + 6, y, half_w.saturating_sub(12), 14),
        });
        y = y.saturating_add(15);
    }

    let notes_rect = WorkspacePixelRect::new(
        notes_pane_x + 6,
        box_rect.y + 24,
        box_rect.width.saturating_sub(half_w).saturating_sub(12),
        box_rect.height.saturating_sub(28),
    );
    InfoSheetGeometry { rows, notes_rect }
}

/// Render the full application frame (chrome, active graph/layout/grid/result view, split view,
/// inspector, status bar, and open modal overlay) deterministically into an RGBA [`RasterImage`].
pub fn render_workspace_rgba(app: &UiApp, target: RenderTargetSpec) -> Result<RasterImage, RenderError> {
    let mut img = RasterImage::new(target.width_px, target.height_px, target.dpi, app.theme.surface_bg)?;
    let layout = compute_workspace_layout(app, target.width_px, target.height_px);
    let palette = FramePalette::from_theme(&app.theme);
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);

    if !app.presentation_mode {
        draw_toolbar(&mut img, layout.toolbar, app, &palette);
        if layout.navigator.width > 0 {
            draw_navigator(&mut img, layout.navigator, app, &palette);
        }
    }

    let viewport_rect = if !app.presentation_mode && app.canvas_dialogs.rulers_visible && layout.canvas.width >= 48 && layout.canvas.height >= 48 {
        draw_physical_rulers(&mut img, layout.canvas, app, &palette)
    } else {
        layout.canvas
    };
    draw_center_viewport(&mut img, viewport_rect, app, target.dpi, &palette, is_rtl);

    if !app.presentation_mode {
        let hover_cmd_tip =
            app.hover_command_id.as_deref().and_then(|cmd_id| crate::menus::command_tooltip(app, cmd_id)).map(|tip| tip.format_inline_tooltip());
        let active_tooltip = app.grid.hover_exclusion_tooltip.as_deref().or(hover_cmd_tip.as_deref());
        if let Some(tip) = active_tooltip
            && viewport_rect.width >= 100
            && viewport_rect.height >= 40
        {
            let bw = viewport_rect.width.saturating_sub(24).min(300);
            let tip_rect = WorkspacePixelRect::new(viewport_rect.x + 12, viewport_rect.bottom().saturating_sub(24), bw, 16);
            fill_rect(&mut img, tip_rect, palette.panel);
            stroke_rect(&mut img, tip_rect, palette.accent);
            draw_text_clipped_dir(
                &mut img,
                DrawTextClipped {
                    x: tip_rect.x + 6,
                    y: tip_rect.y + 4,
                    text: tip,
                    color: palette.text,
                    max_width_px: bw.saturating_sub(12),
                    rtl: is_rtl,
                },
            );
        }
        if layout.inspector.width > 0 {
            draw_inspector_pane(&mut img, layout.inspector, app, &palette);
        }
        draw_status_bar(&mut img, layout.status_bar, app, &palette);
        draw_active_pane_focus(&mut img, &layout, app, &palette);
        if let Some(modal_rect) = layout.modal_overlay {
            draw_modal_overlay(&mut img, layout.canvas, modal_rect, app, &palette);
        }
    }

    Ok(img)
}

/// Screenshot crop target supported by `ui.screenshot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenshotRegionTarget {
    #[default]
    FullWindow,
    Canvas,
    Dialog,
}

/// Render the full application frame deterministically and encode it as PNG bytes (`ui.screenshot`).
pub fn render_workspace_frame_png(app: &UiApp, target: RenderTargetSpec) -> Result<Vec<u8>, RenderError> {
    let image = render_workspace_rgba(app, target)?;
    encode_png(&image)
}

/// Render either the full window, active canvas region, or active modal overlay to PNG bytes and dimensions.
///
/// # Errors
/// Returns [`RenderError`] if dimensions are invalid, the requested modal region is not open, or PNG encoding fails.
pub fn render_workspace_region_png(
    app: &UiApp,
    target: RenderTargetSpec,
    region: ScreenshotRegionTarget,
) -> Result<(Vec<u8>, u32, u32), RenderError> {
    let image = render_workspace_rgba(app, target)?;
    let layout = compute_workspace_layout(app, image.width_px, image.height_px);
    let crop = match region {
        ScreenshotRegionTarget::FullWindow => WorkspacePixelRect::new(0, 0, image.width_px, image.height_px),
        ScreenshotRegionTarget::Canvas => layout.canvas,
        ScreenshotRegionTarget::Dialog => layout
            .modal_overlay
            .ok_or_else(|| RenderError::InvalidTarget { message: String::from("cannot capture `dialog` screenshot when no modal dialog is open") })?,
    };
    let sub = crop_raster_image(&image, crop)?;
    let bytes = encode_png(&sub)?;
    Ok((bytes, sub.width_px, sub.height_px))
}

fn crop_raster_image(src: &RasterImage, rect: WorkspacePixelRect) -> Result<RasterImage, RenderError> {
    let x0 = rect.x.min(src.width_px.saturating_sub(1));
    let y0 = rect.y.min(src.height_px.saturating_sub(1));
    let w = rect.width.min(src.width_px.saturating_sub(x0)).max(1);
    let h = rect.height.min(src.height_px.saturating_sub(y0)).max(1);
    if w == src.width_px && h == src.height_px && x0 == 0 && y0 == 0 {
        return Ok(src.clone());
    }
    let mut out = RasterImage::new(w, h, src.dpi, Rgba8::TRANSPARENT)?;
    for row in 0..h {
        let src_start = ((y0 + row) as usize * src.width_px as usize + x0 as usize) * 4;
        let src_end = src_start + (w as usize) * 4;
        let dst_start = (row as usize * w as usize) * 4;
        let dst_end = dst_start + (w as usize) * 4;
        if let (Some(src_slice), Some(dst_slice)) = (src.rgba.get(src_start..src_end), out.rgba.get_mut(dst_start..dst_end)) {
            dst_slice.copy_from_slice(src_slice);
        }
    }
    Ok(out)
}

fn draw_toolbar(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    fill_rect(img, rect, pal.header);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.bottom().saturating_sub(1), rect.width, 1), pal.border);

    let brand_w = 56u32.min(rect.width.saturating_sub(8));
    let brand_local_x = crate::i18n::mirror_x_for_rtl(8, brand_w, rect.width, is_rtl);
    let brand_rect = WorkspacePixelRect::new(rect.x.saturating_add(brand_local_x), rect.y + 5, brand_w, rect.height.saturating_sub(10).max(1));
    fill_rect(img, brand_rect, pal.accent);
    draw_text_5x7_dir(img, brand_rect.x + 6, brand_rect.y + 4, "LUMEN", Rgba8::WHITE, is_rtl);

    let reserved_buttons_px = draw_toolbar_buttons(img, rect, app, pal, is_rtl);
    let title = app.document().active_sheet_id.as_ref().map_or(app.document().title.as_str(), |id| active_sheet_title(app.document(), id));
    let (title_x, title_max_w) = if is_rtl {
        let left_bound = rect.x.saturating_add(reserved_buttons_px).saturating_add(8);
        let right_bound = brand_rect.x.saturating_sub(12);
        (left_bound, right_bound.saturating_sub(left_bound))
    } else {
        let left_bound = brand_rect.right().saturating_add(12);
        let right_bound = rect.right().saturating_sub(reserved_buttons_px).saturating_sub(8);
        (left_bound, right_bound.saturating_sub(left_bound))
    };
    if title_max_w >= 24 {
        draw_text_clipped_dir(
            img,
            DrawTextClipped { x: title_x, y: rect.y + 10, text: title, color: pal.text, max_width_px: title_max_w, rtl: is_rtl },
        );
    }
}

fn draw_toolbar_buttons(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette, is_rtl: bool) -> u32 {
    let buttons = [
        ("Analyze", matches!(app.canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. })),
        ("Axes", matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. })),
        ("Cmd+K", matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette)),
    ];
    let min_gap_guard = 80u32;
    let mut rx = rect.width.saturating_sub(12);
    for (label, active) in buttons.into_iter().rev() {
        #[allow(clippy::cast_possible_truncation)]
        let bw = ((label.len() as u32) * 6 + 14).min(80);
        if rx < min_gap_guard.saturating_add(bw) {
            break;
        }
        rx = rx.saturating_sub(bw);
        let btn_local_x = crate::i18n::mirror_x_for_rtl(rx, bw, rect.width, is_rtl);
        let btn = WorkspacePixelRect::new(rect.x.saturating_add(btn_local_x), rect.y + 5, bw, rect.height.saturating_sub(10).max(1));
        fill_rect(img, btn, if active { pal.accent } else { pal.surface });
        stroke_rect(img, btn, pal.border);
        draw_text_5x7_dir(img, btn.x + 6, btn.y + 4, label, if active { Rgba8::WHITE } else { pal.text }, is_rtl);
        rx = rx.saturating_sub(6);
    }
    rect.width.saturating_sub(rx)
}

fn active_sheet_title<'a>(doc: &'a ProjectDocument, id: &'a SheetId) -> &'a str {
    doc.data_sheet(id)
        .map(|s| s.title.as_str())
        .or_else(|| doc.info_sheet(id).map(|s| s.title.as_str()))
        .or_else(|| doc.result_sheet(id).map(ResultSheet::title))
        .or_else(|| doc.graph_sheet(id).map(|s| s.title.as_str()))
        .or_else(|| doc.layout_sheet(id).map(|s| s.title.as_str()))
        .unwrap_or(id.as_str())
}

fn draw_navigator(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    fill_rect(img, rect, pal.panel);
    let border_x = if is_rtl { rect.x } else { rect.right().saturating_sub(1) };
    fill_rect(img, WorkspacePixelRect::new(border_x, rect.y, 1, rect.height), pal.border);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 10,
            y: rect.y + 8,
            text: "PROJECT NAVIGATOR",
            color: pal.muted,
            max_width_px: rect.width.saturating_sub(20),
            rtl: is_rtl,
        },
    );

    let list_bottom = rect.y + (rect.height * 2 / 3);
    let mut y = rect.y + 24;
    for entry in &app.nav.entries {
        if y + 18 >= list_bottom {
            break;
        }
        let row = WorkspacePixelRect::new(rect.x + 6, y, rect.width.saturating_sub(12), 16);
        if entry.is_active {
            fill_rect(img, row, pal.accent);
        }
        let tag = match entry.section {
            SheetSection::Data => "D",
            SheetSection::Info => "I",
            SheetSection::Analyses => "A",
            SheetSection::Graphs => "G",
            SheetSection::Layouts => "L",
        };
        let fg = if entry.is_active { Rgba8::WHITE } else { pal.text };
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: row.x + 4,
                y: row.y + 4,
                text: &format!("[{tag}] {}", entry.title),
                color: fg,
                max_width_px: row.width.saturating_sub(8),
                rtl: is_rtl,
            },
        );
        y = y.saturating_add(18);
    }

    let fam_y = list_bottom.saturating_add(4);
    fill_rect(img, WorkspacePixelRect::new(rect.x + 6, fam_y, rect.width.saturating_sub(12), 1), pal.border);
    if fam_y + 18 < rect.bottom() {
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: rect.x + 10,
                y: fam_y + 6,
                text: "FAMILY",
                color: pal.muted,
                max_width_px: rect.width.saturating_sub(20),
                rtl: is_rtl,
            },
        );
        let mut fy = fam_y + 20;
        for rel in &app.nav.family_related_ids {
            if fy + 12 >= rect.bottom() {
                break;
            }
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 12,
                    y: fy,
                    text: rel.as_str(),
                    color: pal.link_blue,
                    max_width_px: rect.width.saturating_sub(20),
                    rtl: is_rtl,
                },
            );
            fy = fy.saturating_add(13);
        }
    }
}

fn draw_physical_rulers(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) -> WorkspacePixelRect {
    let ruler_h = 16u32;
    let ruler_w = 24u32;
    let corner = WorkspacePixelRect::new(rect.x, rect.y, ruler_w, ruler_h);
    let top_bar = WorkspacePixelRect::new(rect.x.saturating_add(ruler_w), rect.y, rect.width.saturating_sub(ruler_w), ruler_h);
    let left_bar = WorkspacePixelRect::new(rect.x, rect.y.saturating_add(ruler_h), ruler_w, rect.height.saturating_sub(ruler_h));
    let inner = WorkspacePixelRect::new(
        rect.x.saturating_add(ruler_w),
        rect.y.saturating_add(ruler_h),
        rect.width.saturating_sub(ruler_w),
        rect.height.saturating_sub(ruler_h),
    );

    fill_rect(img, corner, pal.header);
    fill_rect(img, top_bar, pal.header);
    fill_rect(img, left_bar, pal.header);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.y.saturating_add(ruler_h).saturating_sub(1), rect.width, 1), pal.border);
    fill_rect(img, WorkspacePixelRect::new(rect.x.saturating_add(ruler_w).saturating_sub(1), rect.y, 1, rect.height), pal.border);

    let unit_tag = match app.canvas_dialogs.ruler_unit {
        PhysicalUnit::Millimeters => "MM",
        PhysicalUnit::Inches => "IN",
        PhysicalUnit::Points => "PT",
    };
    draw_text_5x7(img, corner.x + 4, corner.y + 4, unit_tag, pal.accent);

    let (doc_w_pt, doc_h_pt) = active_ruler_doc_span_pt(app);
    let (zoom, pan_pt) = app.canvas_dialogs.effective_zoom_and_pan();
    draw_horizontal_ruler_ticks(img, top_bar, app.canvas_dialogs.ruler_unit, doc_w_pt, zoom, pan_pt[0], pal);
    draw_vertical_ruler_ticks(img, left_bar, app.canvas_dialogs.ruler_unit, doc_h_pt, zoom, pan_pt[1], pal);
    inner
}

fn active_ruler_doc_span_pt(app: &UiApp) -> (f64, f64) {
    if let Some(preset) = app.canvas_dialogs.journal_preset {
        return preset.size_pt();
    }
    let doc = app.document();
    if let Some(active_id) = &doc.active_sheet_id
        && let Some(ls) = doc.layout_sheet(active_id)
    {
        let page = ls.spec.page_size_pt();
        if page.width > 0.0 && page.height > 0.0 {
            return (page.width, page.height);
        }
    }
    lumencraft_tools::JournalWidthPreset::DoubleColumn174Mm.size_pt()
}

fn ruler_px_offset(view_px: f64, span_px: u32) -> Option<u32> {
    if !view_px.is_finite() || view_px < 0.0 || span_px == 0 {
        return None;
    }
    let max_off = f64::from(span_px.saturating_sub(1));
    if view_px > max_off {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(view_px.round() as u32)
}

fn draw_horizontal_ruler_ticks(
    img: &mut RasterImage,
    bar: WorkspacePixelRect,
    unit: PhysicalUnit,
    doc_span_pt: f64,
    zoom: f64,
    pan_pt: f64,
    pal: &FramePalette,
) {
    if bar.width == 0 {
        return;
    }
    let px_per_pt = (f64::from(bar.width) / doc_span_pt.max(1.0)).max(0.05);
    let ticks = build_ruler_ticks(unit, doc_span_pt, zoom, pan_pt, px_per_pt);
    for tick in ticks {
        let Some(off_x) = ruler_px_offset(tick.view_px, bar.width) else {
            continue;
        };
        let tick_h = if tick.major { 7 } else { 3 };
        let color = if tick.major { pal.text } else { pal.muted };
        fill_rect(img, WorkspacePixelRect::new(bar.x.saturating_add(off_x), bar.bottom().saturating_sub(tick_h + 1), 1, tick_h), color);
        if let Some(label) = tick.label.as_deref()
            && off_x.saturating_add(10) < bar.width
        {
            draw_text_clipped(
                img,
                bar.x.saturating_add(off_x).saturating_add(2),
                bar.y + 2,
                label,
                pal.text,
                bar.width.saturating_sub(off_x + 3).min(24),
            );
        }
    }
}

fn draw_vertical_ruler_ticks(
    img: &mut RasterImage,
    bar: WorkspacePixelRect,
    unit: PhysicalUnit,
    doc_span_pt: f64,
    zoom: f64,
    pan_pt: f64,
    pal: &FramePalette,
) {
    if bar.height == 0 {
        return;
    }
    let px_per_pt = (f64::from(bar.height) / doc_span_pt.max(1.0)).max(0.05);
    let ticks = build_ruler_ticks(unit, doc_span_pt, zoom, pan_pt, px_per_pt);
    for tick in ticks {
        let Some(off_y) = ruler_px_offset(tick.view_px, bar.height) else {
            continue;
        };
        let tick_w = if tick.major { 7 } else { 3 };
        let color = if tick.major { pal.text } else { pal.muted };
        fill_rect(img, WorkspacePixelRect::new(bar.right().saturating_sub(tick_w + 1), bar.y.saturating_add(off_y), tick_w, 1), color);
        if let Some(label) = tick.label.as_deref()
            && off_y.saturating_add(8) < bar.height
        {
            draw_text_clipped(img, bar.x + 2, bar.y.saturating_add(off_y).saturating_add(2), label, pal.text, bar.width.saturating_sub(10));
        }
    }
}

fn draw_center_viewport(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, dpi: u16, pal: &FramePalette, is_rtl: bool) {
    fill_rect(img, rect, pal.canvas_mat);
    if app.nav.gallery.open {
        draw_portfolio_gallery(img, rect, app, pal, is_rtl);
        return;
    }

    if app.split.enabled
        && let (Some(prim_rect), Some(div_rect), Some(sec_rect)) = compute_split_panes(rect, app.split.ratio, is_rtl)
    {
        let (prim_id, sec_id) = app.split.resolve_sheets(app.document());
        if let Some(pid) = &prim_id {
            draw_sheet_pane(img, prim_rect, app, pid, dpi, pal);
        } else {
            draw_empty_studio(img, prim_rect, pal, is_rtl);
        }
        let div_color = if app.split.dragging_divider || app.interaction.dragging_split_divider { pal.accent } else { pal.border };
        fill_rect(img, div_rect, div_color);
        if let Some(sid) = &sec_id {
            draw_sheet_pane(img, sec_rect, app, sid, dpi, pal);
        } else {
            draw_empty_studio(img, sec_rect, pal, is_rtl);
        }
        return;
    }

    let Some(active_id) = &app.document().active_sheet_id else {
        draw_empty_studio(img, rect, pal, is_rtl);
        return;
    };
    draw_sheet_pane(img, rect, app, active_id, dpi, pal);
}

fn draw_sheet_pane(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, sheet_id: &SheetId, dpi: u16, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    let doc = app.document();
    if doc.graph_sheet(sheet_id).is_some() || doc.layout_sheet(sheet_id).is_some() {
        draw_scene_viewport(img, rect, app, sheet_id, dpi, pal);
    } else if let Some(ds) = doc.data_sheet(sheet_id) {
        draw_data_grid(img, rect, ds, app, pal);
    } else if let Some(rs) = doc.result_sheet(sheet_id) {
        draw_result_view(img, rect, rs, &app.canvas_dialogs.result_view, pal, is_rtl);
    } else if let Some(info) = doc.info_sheet(sheet_id) {
        draw_info_view(img, rect, info, app, pal, is_rtl);
    } else {
        draw_empty_studio(img, rect, pal, is_rtl);
    }
}

fn draw_portfolio_gallery(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette, is_rtl: bool) {
    let header_title = format!("PORTFOLIO GALLERY ({})", app.nav.gallery.cards.len());
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 12,
            y: rect.y + 8,
            text: &header_title,
            color: pal.text,
            max_width_px: rect.width.saturating_sub(24),
            rtl: is_rtl,
        },
    );
    fill_rect(img, WorkspacePixelRect::new(rect.x + 10, rect.y + 22, rect.width.saturating_sub(20), 1), pal.border);

    let body = WorkspacePixelRect::new(rect.x + 12, rect.y + 28, rect.width.saturating_sub(24), rect.height.saturating_sub(36));
    if body.width < 40 || body.height < 32 {
        return;
    }
    if app.nav.gallery.cards.is_empty() {
        draw_text_clipped_dir(
            img,
            DrawTextClipped { x: body.x, y: body.y + 12, text: "NO SHEETS MATCH FILTER", color: pal.muted, max_width_px: body.width, rtl: is_rtl },
        );
        return;
    }

    let gap = 10u32;
    let cols = clamp_u32_safe(body.width / 132, 1, 4);
    let card_w = body.width.saturating_sub(gap.saturating_mul(cols.saturating_sub(1))) / cols;
    let card_h = 74u32.min(body.height);
    for (idx, card) in app.nav.gallery.cards.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let col = (idx as u32) % cols;
        #[allow(clippy::cast_possible_truncation)]
        let row = (idx as u32) / cols;
        let cy = body.y.saturating_add(row.saturating_mul(card_h.saturating_add(gap)));
        if cy.saturating_add(card_h) > body.bottom() {
            break;
        }
        let local_cx = col.saturating_mul(card_w.saturating_add(gap));
        let cx = body.x.saturating_add(crate::i18n::mirror_x_for_rtl(local_cx, card_w, body.width, is_rtl));
        let card_rect = WorkspacePixelRect::new(cx, cy, card_w, card_h);
        draw_gallery_card(img, card_rect, card, app.nav.gallery.selected_ids.contains(&card.sheet_id), pal, is_rtl);
    }
}

fn draw_gallery_card(img: &mut RasterImage, rect: WorkspacePixelRect, card: &PortfolioCard, is_selected: bool, pal: &FramePalette, is_rtl: bool) {
    fill_rect(img, rect, pal.surface);
    let accent_band = card.color_tag.unwrap_or(pal.accent);
    let stripe_x = rect.x.saturating_add(crate::i18n::mirror_x_for_rtl(0, 4, rect.width, is_rtl));
    fill_rect(img, WorkspacePixelRect::new(stripe_x, rect.y + 1, 4, rect.height.saturating_sub(2)), accent_band);

    let thumb = WorkspacePixelRect::new(rect.x + 8, rect.y + 6, rect.width.saturating_sub(16), rect.height.saturating_sub(36).max(8));
    fill_rect(img, thumb, pal.panel);
    stroke_rect(img, thumb, pal.border);
    if thumb.width >= 20 && thumb.height >= 12 {
        fill_rect(img, WorkspacePixelRect::new(thumb.x + 4, thumb.y + 4, thumb.width.saturating_sub(8) / 2, 4), accent_band);
        fill_rect(img, WorkspacePixelRect::new(thumb.x + 4, thumb.y + 11, thumb.width.saturating_sub(12), 2), pal.muted);
    }

    let text_y = thumb.bottom().saturating_add(4);
    let label = format!("[{}] {}", card.badge_label, card.title);
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: rect.x + 8, y: text_y, text: &label, color: pal.text, max_width_px: rect.width.saturating_sub(16), rtl: is_rtl },
    );
    if text_y + 18 <= rect.bottom() {
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: rect.x + 8,
                y: text_y + 10,
                text: &card.subtitle,
                color: pal.muted,
                max_width_px: rect.width.saturating_sub(16),
                rtl: is_rtl,
            },
        );
    }

    let selected = is_selected || card.selected;
    let border_color = if selected {
        pal.accent
    } else if card.active {
        pal.link_blue
    } else {
        pal.border
    };
    stroke_rect(img, rect, border_color);
    if selected && rect.width > 4 && rect.height > 4 {
        stroke_rect(img, WorkspacePixelRect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2), pal.accent);
    }
}

fn draw_active_pane_focus(img: &mut RasterImage, layout: &WorkspaceFrameLayout, app: &UiApp, pal: &FramePalette) {
    let focus_color = Rgba8::rgba(pal.accent.r, pal.accent.g, pal.accent.b, 190);
    let target = match app.nav.active_pane {
        ActiveWorkspacePane::Navigator if layout.navigator.width > 4 => Some(layout.navigator),
        ActiveWorkspacePane::FamilyStrip if layout.navigator.width > 4 && layout.navigator.height > 24 => {
            let fam_y = layout.navigator.y + (layout.navigator.height * 2 / 3).saturating_add(2);
            Some(WorkspacePixelRect::new(layout.navigator.x, fam_y, layout.navigator.width, layout.navigator.bottom().saturating_sub(fam_y)))
        }
        ActiveWorkspacePane::MainCanvasOrGrid if layout.canvas.width > 4 => Some(layout.canvas),
        ActiveWorkspacePane::Inspector if layout.inspector.width > 4 => Some(layout.inspector),
        ActiveWorkspacePane::FloatingNotes if layout.canvas.width >= 96 && layout.canvas.height >= 40 => {
            let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
            let badge_w = 88u32;
            let local_x = crate::i18n::mirror_x_for_rtl(layout.canvas.width.saturating_sub(badge_w + 8), badge_w, layout.canvas.width, is_rtl);
            let notes_rect = WorkspacePixelRect::new(layout.canvas.x.saturating_add(local_x), layout.canvas.bottom().saturating_sub(24), badge_w, 18);
            fill_rect(img, notes_rect, pal.panel);
            stroke_rect(img, notes_rect, pal.accent);
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: notes_rect.x + 6,
                    y: notes_rect.y + 5,
                    text: "NOTES PANE",
                    color: pal.accent,
                    max_width_px: badge_w - 12,
                    rtl: is_rtl,
                },
            );
            None
        }
        _ => None,
    };

    if let Some(pane) = target
        && pane.width >= 4
        && pane.height >= 4
    {
        fill_rect(img, WorkspacePixelRect::new(pane.x, pane.y, pane.width, 2), focus_color);
        stroke_rect(img, WorkspacePixelRect::new(pane.x + 1, pane.y + 1, pane.width - 2, pane.height - 2), focus_color);
    }
}

fn draw_empty_studio(img: &mut RasterImage, rect: WorkspacePixelRect, pal: &FramePalette, is_rtl: bool) {
    let cw = rect.width.saturating_sub(48).min(260);
    let ch = rect.height.saturating_sub(48).min(96);
    let card = WorkspacePixelRect::new(rect.x + (rect.width.saturating_sub(cw)) / 2, rect.y + (rect.height.saturating_sub(ch)) / 2, cw, ch);
    fill_rect(img, card, pal.panel);
    stroke_rect(img, card, pal.border);
    let max_w = card.width.saturating_sub(32);
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: card.x + 16, y: card.y + 18, text: "LUMEN SCIENTIFIC STUDIO", color: pal.text, max_width_px: max_w, rtl: is_rtl },
    );
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: card.x + 16, y: card.y + 38, text: "NEW DATA TABLE: CMD+T", color: pal.muted, max_width_px: max_w, rtl: is_rtl },
    );
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: card.x + 16, y: card.y + 56, text: "COMMAND PALETTE: CMD+K", color: pal.accent, max_width_px: max_w, rtl: is_rtl },
    );
}

fn draw_scene_viewport(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, sheet_id: &SheetId, dpi: u16, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    let geom = compute_scene_viewport_geometry(rect, app, sheet_id, is_rtl);
    let paper = geom.paper;

    if !app.presentation_mode
        && let Some(title_rect) = geom.title_rect
    {
        #[allow(clippy::cast_possible_truncation)]
        let zoom_pct = (geom.paper_mapping.zoom * 100.0).round() as i64;
        let header_label = match (&app.inline_editor.active, &app.inline_editor.target) {
            (true, Some(InlineEditorTarget::GraphTitle { sheet_id: edit_id })) if edit_id == sheet_id => {
                stroke_rect(img, title_rect, pal.accent);
                format!("[EDIT] {}", app.inline_editor.buffer)
            }
            _ => format!("CANVAS: {} ({zoom_pct}%)", active_sheet_title(app.document(), sheet_id)),
        };
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: title_rect.x,
                y: title_rect.y + 4,
                text: &header_label,
                color: pal.text,
                max_width_px: title_rect.width,
                rtl: is_rtl,
            },
        );
    }

    let mut preflight_count = 0usize;
    if let Ok(scene) = app.session().render_sheet_scene(sheet_id) {
        preflight_count = preflight_check_scene(&scene).len();
        if let Ok(mut rendered) = render_scene(&scene, RenderTargetSpec { width_px: paper.width, height_px: paper.height, dpi: dpi.max(72) }) {
            if app.preview_filter != CanvasPreviewFilter::Normal {
                apply_canvas_preview_filter(&mut rendered, app.preview_filter);
            }
            blit_scene_with_mapping(img, paper, &rendered, &geom.paper_mapping, pal.canvas_mat);
        }
        draw_brushed_point_highlights(img, paper, &scene, app, &geom.paper_mapping, pal);
    } else {
        fill_rect(img, paper, Rgba8::WHITE);
    }
    stroke_rect(img, paper, pal.border);

    if let Some(gs) = app.document().graph_sheet(sheet_id) {
        draw_graph_summary_chips(img, paper, gs, pal);
    } else if let Some(ls) = app.document().layout_sheet(sheet_id) {
        draw_layout_summary_badges(img, paper, ls, &geom.paper_mapping, pal);
    }

    if !app.presentation_mode {
        draw_snap_guides(img, paper, app, &geom.paper_mapping, pal);
        draw_live_canvas_drag_and_handles(img, paper, app, sheet_id, &geom.paper_mapping, pal);

        if matches!(
            (&app.inline_editor.active, &app.inline_editor.target),
            (true, Some(InlineEditorTarget::AxisTitle { sheet_id: eid, axis })) if eid == sheet_id && axis == "x"
        ) && let Some(xr) = geom.x_axis_title_rect
        {
            fill_rect(img, xr, pal.panel);
            stroke_rect(img, xr, pal.accent);
            draw_text_clipped(img, xr.x + 4, xr.y + 6, &app.inline_editor.buffer, pal.text, xr.width.saturating_sub(8));
        } else if matches!(
            (&app.inline_editor.active, &app.inline_editor.target),
            (true, Some(InlineEditorTarget::AxisTitle { sheet_id: eid, axis })) if eid == sheet_id && axis == "y"
        ) && let Some(yr) = geom.y_axis_title_rect
        {
            fill_rect(img, yr, pal.panel);
            stroke_rect(img, yr, pal.accent);
            draw_text_clipped(img, yr.x + 2, yr.y + 6, &app.inline_editor.buffer, pal.text, yr.width.saturating_sub(4));
        }

        if app.preview_filter != CanvasPreviewFilter::Normal && paper.width > 96 {
            let filter_badge = WorkspacePixelRect::new(paper.x + 8, paper.y + 20, paper.width.saturating_sub(16).min(126), 14);
            fill_rect(img, filter_badge, pal.panel);
            stroke_rect(img, filter_badge, pal.accent);
            draw_text_clipped(
                img,
                filter_badge.x + 4,
                filter_badge.y + 3,
                &format!("PREVIEW: {}", app.preview_filter.as_str()),
                pal.accent,
                filter_badge.width.saturating_sub(8),
            );
        }

        if preflight_count > 0 && paper.width >= 110 && paper.height >= 40 {
            let bw = 88u32.min(paper.width.saturating_sub(12));
            let bx = paper.x.saturating_add(crate::i18n::mirror_x_for_rtl(paper.width.saturating_sub(bw + 6), bw, paper.width, is_rtl));
            let badge = WorkspacePixelRect::new(bx, paper.y + 6, bw, 14);
            fill_rect(img, badge, Rgba8::rgb(0xD5, 0x5E, 0x00));
            draw_text_clipped(img, badge.x + 4, badge.y + 3, &format!("!{preflight_count} PREFLIGHT"), Rgba8::WHITE, bw.saturating_sub(8));
        }
    }
}

fn blit_scene_with_mapping(
    dst: &mut RasterImage,
    paper: WorkspacePixelRect,
    rendered: &RasterImage,
    mapping: &ScenePaperMapping,
    bg_fallback: Rgba8,
) {
    let is_identity = (mapping.zoom - 1.0).abs() < 1e-6 && mapping.pan_pt[0].abs() < 1e-6 && mapping.pan_pt[1].abs() < 1e-6;
    if is_identity {
        blit_image(dst, paper.x, paper.y, rendered);
        return;
    }

    let max_dx = paper.width.min(dst.width_px.saturating_sub(paper.x));
    let max_dy = paper.height.min(dst.height_px.saturating_sub(paper.y));
    let rw = rendered.width_px.max(1);
    let rh = rendered.height_px.max(1);
    for dy in 0..max_dy {
        for dx in 0..max_dx {
            let px = paper.x + dx;
            let py = paper.y + dy;
            let Some(scene_pt) = mapping.pixel_to_scene_pt(f64::from(px) + 0.5, f64::from(py) + 0.5) else {
                dst.blend_pixel(px, py, bg_fallback);
                continue;
            };
            let u = scene_pt.x / mapping.scene_width_pt.max(1.0);
            let v = scene_pt.y / mapping.scene_height_pt.max(1.0);
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                dst.blend_pixel(px, py, bg_fallback);
                continue;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let sx = ((u * f64::from(rw)).floor() as u32).min(rw - 1);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let sy = ((v * f64::from(rh)).floor() as u32).min(rh - 1);
            let idx = ((sy as usize) * (rendered.width_px as usize) + (sx as usize)) * 4;
            if let Some(&[r, g, b, a]) = rendered.rgba.get(idx..idx + 4) {
                dst.blend_pixel(px, py, Rgba8::rgba(r, g, b, a));
            }
        }
    }
}

fn draw_brushed_point_highlights(
    img: &mut RasterImage,
    paper: WorkspacePixelRect,
    scene: &ChartScene,
    app: &UiApp,
    mapping: &ScenePaperMapping,
    pal: &FramePalette,
) {
    if app.brushed_cell.is_none() && app.brushed_points.is_empty() {
        return;
    }
    for meta in &scene.plotted_points {
        let is_primary = app.brushed_cell.as_ref() == Some(&meta.source_cell);
        let is_multi = app.brushed_points.iter().any(|p| p == &meta.source_cell);
        if !is_primary && !is_multi {
            continue;
        }
        let Some([fx, fy]) = mapping.scene_pt_to_pixel(meta.center_pt) else {
            continue;
        };
        if !fx.is_finite() || !fy.is_finite() || fx < 6.0 || fy < 6.0 {
            continue;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (cx, cy) = (fx.round() as u32, fy.round() as u32);
        if !paper.contains(cx, cy) {
            continue;
        }
        let outer = WorkspacePixelRect::new(cx.saturating_sub(5), cy.saturating_sub(5), 11, 11);
        let inner = WorkspacePixelRect::new(cx.saturating_sub(3), cy.saturating_sub(3), 7, 7);
        stroke_rect(img, outer, pal.accent);
        stroke_rect(img, inner, Rgba8::WHITE);
    }
}

fn draw_snap_guides(img: &mut RasterImage, paper: WorkspacePixelRect, app: &UiApp, mapping: &ScenePaperMapping, pal: &FramePalette) {
    for guide in &app.canvas_dialogs.canvas.active_snap_guides {
        let guide_color = match guide.source {
            SnapSource::PlotOrigin => Rgba8::rgb(220, 38, 38),
            _ => pal.accent,
        };
        match guide.orientation {
            SnapOrientation::Vertical => {
                if let (Some([px, sy]), Some([_, ey])) = (
                    mapping.scene_pt_to_pixel(PointPt::new(guide.position_pt, guide.span_start_pt)),
                    mapping.scene_pt_to_pixel(PointPt::new(guide.position_pt, guide.span_end_pt)),
                ) {
                    let x = clamp_to_u32(px, paper.x, paper.right().saturating_sub(1));
                    let y0 = clamp_to_u32(sy.min(ey), paper.y, paper.bottom());
                    let y1 = clamp_to_u32(sy.max(ey), paper.y, paper.bottom());
                    fill_rect(img, WorkspacePixelRect::new(x, y0, 1, y1.saturating_sub(y0)), guide_color);
                }
            }
            SnapOrientation::Horizontal => {
                if let (Some([sx, py]), Some([ex, _])) = (
                    mapping.scene_pt_to_pixel(PointPt::new(guide.span_start_pt, guide.position_pt)),
                    mapping.scene_pt_to_pixel(PointPt::new(guide.span_end_pt, guide.position_pt)),
                ) {
                    let y = clamp_to_u32(py, paper.y, paper.bottom().saturating_sub(1));
                    let x0 = clamp_to_u32(sx.min(ex), paper.x, paper.right());
                    let x1 = clamp_to_u32(sx.max(ex), paper.x, paper.right());
                    fill_rect(img, WorkspacePixelRect::new(x0, y, x1.saturating_sub(x0), 1), guide_color);
                }
            }
        }
    }
}

fn draw_live_canvas_drag_and_handles(
    img: &mut RasterImage,
    paper: WorkspacePixelRect,
    app: &UiApp,
    sheet_id: &SheetId,
    mapping: &ScenePaperMapping,
    pal: &FramePalette,
) {
    let preview_bounds = match &app.interaction.canvas_drag {
        Some(CanvasDragTarget::LayoutItem { sheet_id: drag_sheet, current_bounds_pt, preview_rect_pt, .. }) if drag_sheet == sheet_id => {
            Some(preview_rect_pt.unwrap_or(*current_bounds_pt))
        }
        Some(CanvasDragTarget::GraphAnnotation { sheet_id: drag_sheet, current_bounds_pt, .. }) if drag_sheet == sheet_id => Some(*current_bounds_pt),
        _ => None,
    };

    if let Some(bounds_pt) = preview_bounds
        && let Some(rect_px) = scene_rect_to_paper_px(bounds_pt, paper, mapping)
    {
        stroke_rect(img, rect_px, pal.accent);
        draw_resize_handle_grips(img, rect_px, pal.accent);
        return;
    }

    if let Some(ls) = app.document().layout_sheet(sheet_id)
        && let Some(
            CanvasHitTarget::LayoutPanel { index } | CanvasHitTarget::PanelResizeHandle { index, .. } | CanvasHitTarget::EmbeddedTable { index },
        ) = app.canvas_dialogs.canvas.hover_target
        && let Some(item) = ls.spec.items.get(index)
    {
        let bounds_pt = match item {
            LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
                *bounds_pt
            }
        };
        if let Some(rect_px) = scene_rect_to_paper_px(bounds_pt, paper, mapping) {
            stroke_rect(img, rect_px, pal.accent);
            draw_resize_handle_grips(img, rect_px, pal.accent);
        }
    }
}

fn scene_rect_to_paper_px(bounds: lumencraft_doc::RectPt, paper: WorkspacePixelRect, mapping: &ScenePaperMapping) -> Option<WorkspacePixelRect> {
    let [x0, y0] = mapping.scene_pt_to_pixel(PointPt::new(bounds.x, bounds.y))?;
    let [x1, y1] = mapping.scene_pt_to_pixel(PointPt::new(bounds.x + bounds.width, bounds.y + bounds.height))?;
    let px = clamp_to_u32(x0.min(x1), paper.x, paper.right().saturating_sub(1));
    let py = clamp_to_u32(y0.min(y1), paper.y, paper.bottom().saturating_sub(1));
    let pr = clamp_to_u32(x0.max(x1), px.saturating_add(2), paper.right());
    let pb = clamp_to_u32(y0.max(y1), py.saturating_add(2), paper.bottom());
    Some(WorkspacePixelRect::new(px, py, pr.saturating_sub(px), pb.saturating_sub(py)))
}

fn draw_resize_handle_grips(img: &mut RasterImage, rect: WorkspacePixelRect, accent: Rgba8) {
    let mid_x = rect.x.saturating_add(rect.width / 2);
    let mid_y = rect.y.saturating_add(rect.height / 2);
    let points = [
        (rect.x, rect.y),
        (mid_x, rect.y),
        (rect.right(), rect.y),
        (rect.right(), mid_y),
        (rect.right(), rect.bottom()),
        (mid_x, rect.bottom()),
        (rect.x, rect.bottom()),
        (rect.x, mid_y),
    ];
    for (hx, hy) in points {
        let grip = WorkspacePixelRect::new(hx.saturating_sub(2), hy.saturating_sub(2), 5, 5);
        fill_rect(img, grip, Rgba8::WHITE);
        stroke_rect(img, grip, accent);
    }
}

fn clamp_to_u32(val: f64, lo: u32, hi: u32) -> u32 {
    if !val.is_finite() {
        return lo;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rounded = clamp_f64_safe(val.round(), f64::from(lo), f64::from(hi.max(lo))) as u32;
    rounded
}

fn draw_graph_summary_chips(img: &mut RasterImage, paper: WorkspacePixelRect, gs: &GraphSheet, pal: &FramePalette) {
    let mut sx = paper.x + 10;
    for (idx, _s) in gs.spec.series.iter().take(4).enumerate() {
        let chip = WorkspacePixelRect::new(sx, paper.y + 8, 12, 8);
        fill_rect(img, chip, gs.spec.palette.color_at(idx));
        stroke_rect(img, chip, Rgba8::BLACK);
        sx = sx.saturating_add(16);
    }
    if !gs.initialized && paper.width > 120 && paper.height > 40 {
        let banner = WorkspacePixelRect::new(paper.x + 10, paper.bottom().saturating_sub(24), paper.width.saturating_sub(20).min(180), 16);
        fill_rect(img, banner, pal.accent);
        draw_text_clipped(img, banner.x + 6, banner.y + 4, "SELECT GRAPH TYPE", Rgba8::WHITE, banner.width.saturating_sub(12));
    }
}

fn draw_layout_summary_badges(img: &mut RasterImage, paper: WorkspacePixelRect, ls: &LayoutSheet, mapping: &ScenePaperMapping, pal: &FramePalette) {
    for (idx, item) in ls.spec.items.iter().enumerate() {
        let bounds = match item {
            LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
                *bounds_pt
            }
        };
        let (Some([x0, y0]), Some([x1, y1])) = (
            mapping.scene_pt_to_pixel(PointPt::new(bounds.x, bounds.y)),
            mapping.scene_pt_to_pixel(PointPt::new(bounds.x + bounds.width, bounds.y + bounds.height)),
        ) else {
            continue;
        };
        let px = clamp_to_u32(x0, paper.x, paper.right());
        let py = clamp_to_u32(y0, paper.y, paper.bottom());
        let pw = clamp_to_u32(x1, px, paper.right()).saturating_sub(px).max(4).min(paper.right().saturating_sub(px));
        let ph = clamp_to_u32(y1, py, paper.bottom()).saturating_sub(py).max(4).min(paper.bottom().saturating_sub(py));
        if pw >= 4 && ph >= 4 {
            let box_rect = WorkspacePixelRect::new(px, py, pw, ph);
            stroke_rect(img, box_rect, pal.accent);
            if let Some(badge) = ls.spec.panel_badges.badge_for(idx) {
                fill_rect(img, WorkspacePixelRect::new(px + 2, py + 2, 14, 12), pal.accent);
                draw_text_clipped(img, px + 5, py + 4, &badge, Rgba8::WHITE, 10);
            }
        }
    }
}

fn draw_data_grid(img: &mut RasterImage, rect: WorkspacePixelRect, ds: &DataSheet, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    let geom = compute_grid_geometry(rect, ds, app, is_rtl);
    let sheet_rect = geom.sheet_rect;
    fill_rect(img, sheet_rect, pal.surface);
    stroke_rect(img, sheet_rect, pal.border);

    let title = format!("[{}] {}", ds.kind().label(), ds.title);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: sheet_rect.x + 8,
            y: sheet_rect.y + 6,
            text: &title,
            color: pal.text,
            max_width_px: sheet_rect.width.saturating_sub(16),
            rtl: is_rtl,
        },
    );

    let frame = app.grid.tool.format_visible_window(ds);
    fill_rect(img, WorkspacePixelRect::new(sheet_rect.x + 1, geom.hdr_y, sheet_rect.width.saturating_sub(2), geom.row_h), pal.header);
    let hash_x = sheet_rect.x.saturating_add(crate::i18n::mirror_x_for_rtl(0, geom.gutter_w, sheet_rect.width, is_rtl));
    draw_text_5x7(img, hash_x + 8, geom.hdr_y + 5, "#", pal.muted);

    for subcol in 0..geom.x_subcols {
        let Some(hdr) = geom.x_header_rect(subcol) else {
            continue;
        };
        stroke_rect(img, hdr, pal.border);
        let x_label = match &ds.archetype {
            TableArchetype::Xy(xy) => match subcol {
                0 if xy.x_title.is_empty() => "X",
                0 => xy.x_title.as_str(),
                1 => "X+/-",
                _ => "XN",
            },
            TableArchetype::Survival(_) => "Time",
            _ => "X",
        };
        draw_text_clipped_dir(
            img,
            DrawTextClipped { x: hdr.x + 4, y: hdr.y + 5, text: x_label, color: pal.text, max_width_px: hdr.width.saturating_sub(6), rtl: is_rtl },
        );
    }

    for (vis_idx, (g_idx, _)) in geom.visible_cols.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let hdr = geom.column_header_rect(vis_idx as u32);
        stroke_rect(img, hdr, pal.border);
        if let TableArchetype::MultipleVariables(mv) = &ds.archetype
            && let Some(var) = mv.variables.get(*g_idx)
        {
            fill_rect(
                img,
                WorkspacePixelRect::new(hdr.x + 2, hdr.y + 2, 16, geom.row_h.saturating_sub(4)),
                variable_role_color(var.role, &app.theme),
            );
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: hdr.x + 21,
                    y: hdr.y + 5,
                    text: &var.title,
                    color: pal.text,
                    max_width_px: hdr.width.saturating_sub(22),
                    rtl: is_rtl,
                },
            );
        } else {
            let label = data_column_title(ds, *g_idx).unwrap_or("Col");
            draw_text_clipped_dir(
                img,
                DrawTextClipped { x: hdr.x + 4, y: hdr.y + 5, text: label, color: pal.text, max_width_px: hdr.width.saturating_sub(6), rtl: is_rtl },
            );
        }
    }

    draw_visible_grid_cells(img, ds, &geom, &frame.cells, app, pal);
}

fn draw_visible_grid_cells(
    img: &mut RasterImage,
    ds: &DataSheet,
    geom: &GridHitGeometry,
    cells: &[lumencraft_tools::grid::VisibleCellView],
    app: &UiApp,
    pal: &FramePalette,
) {
    let sheet_rect = geom.sheet_rect;
    for r_off in 0..geom.end_row.saturating_sub(geom.start_row) {
        #[allow(clippy::cast_possible_truncation)]
        let r_off_u32 = r_off as u32;
        let gutter = geom.row_gutter_rect(r_off_u32);
        let row_num = geom.start_row + r_off + 1;
        draw_text_5x7(img, gutter.x + 6, gutter.y + 5, &format!("{row_num}"), pal.muted);
        fill_rect(img, WorkspacePixelRect::new(sheet_rect.x + 1, gutter.y + geom.row_h - 1, sheet_rect.width.saturating_sub(2), 1), pal.border);
    }

    let is_generated_x = matches!(&ds.archetype, TableArchetype::Xy(xy) if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }));
    for row in geom.start_row..geom.end_row {
        for subcol in 0..geom.x_subcols {
            let Some(cell_rect) = geom.x_cell_rect(row, subcol) else {
                continue;
            };
            let is_active = app.grid.active_x_cell == Some((row, subcol));
            let is_inline_edited = matches!(
                (&app.inline_editor.active, &app.inline_editor.target),
                (true, Some(InlineEditorTarget::GridXCell { sheet_id, row: r, subcol: s }))
                    if sheet_id == &geom.sheet_id && *r == row && *s == subcol
            );
            if is_active || is_inline_edited {
                fill_rect(img, cell_rect, shift_luma(pal.surface, -10));
                stroke_rect(img, cell_rect, pal.accent);
            }
            let (raw_text, fg) = if is_inline_edited {
                (app.inline_editor.buffer.clone(), pal.text)
            } else if is_generated_x {
                let dec = app.locale_preset.decimal_separator;
                let text = match &ds.archetype {
                    TableArchetype::Xy(xy) => xy.x_value_at(row).map_or_else(String::new, |v| format_localized_compact_f64(v, dec)),
                    _ => String::new(),
                };
                (text, pal.muted)
            } else if let Some(cell) = ds.x_or_time_cell(row, subcol) {
                (format_cell_display(cell, &app.locale_preset), if cell.excluded { pal.excluded } else { pal.text })
            } else {
                (String::new(), pal.text)
            };
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: cell_rect.x + 4,
                    y: cell_rect.y + 5,
                    text: &raw_text,
                    color: fg,
                    max_width_px: cell_rect.width.saturating_sub(6),
                    rtl: geom.is_rtl,
                },
            );
        }
    }

    for cell in cells {
        let Some(cell_rect) = geom.cell_rect(cell.coord.row, cell.flat_col) else {
            continue;
        };
        let is_brushed = app
            .brushed_cell
            .as_ref()
            .is_some_and(|b| b.sheet_id == geom.sheet_id && b.row == cell.coord.row && b.col == cell.coord.col && b.subcol == cell.coord.subcol)
            || app
                .brushed_points
                .iter()
                .any(|b| b.sheet_id == geom.sheet_id && b.row == cell.coord.row && b.col == cell.coord.col && b.subcol == cell.coord.subcol);
        let is_inline_edited = matches!(
            (&app.inline_editor.active, &app.inline_editor.target),
            (true, Some(InlineEditorTarget::GridCell { sheet_id, coord })) if sheet_id == &geom.sheet_id && *coord == cell.coord
        );

        if is_brushed {
            fill_rect(img, cell_rect, shift_luma(pal.surface, -16));
        } else if cell.is_selected {
            fill_rect(img, cell_rect, shift_luma(pal.surface, -10));
        }
        if is_inline_edited || (cell.is_active && app.grid.active_x_cell.is_none()) {
            stroke_rect(img, cell_rect, pal.accent);
        } else if is_brushed {
            stroke_rect(img, cell_rect, pal.link_blue);
        } else if let Some(link) = cell.link_border {
            let link_color = match link {
                LinkBorderColor::Blue => pal.link_blue,
                LinkBorderColor::Green => pal.link_green,
                LinkBorderColor::Red => pal.link_red,
                LinkBorderColor::Charcoal => pal.muted,
            };
            stroke_rect(img, cell_rect, link_color);
        }
        let text = if is_inline_edited { app.inline_editor.buffer.as_str() } else { cell.display_text.as_str() };
        let fg = if cell.excluded { pal.excluded } else { pal.text };
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: cell_rect.x + 4,
                y: cell_rect.y + 5,
                text,
                color: fg,
                max_width_px: cell_rect.width.saturating_sub(6),
                rtl: geom.is_rtl,
            },
        );
    }
}

fn draw_result_view(
    img: &mut RasterImage,
    rect: WorkspacePixelRect,
    rs: &ResultSheet,
    view: &ResultSheetViewState,
    pal: &FramePalette,
    is_rtl: bool,
) {
    let box_rect = WorkspacePixelRect::new(rect.x + 8, rect.y + 8, rect.width.saturating_sub(16), rect.height.saturating_sub(16));
    fill_rect(img, box_rect, pal.surface);
    stroke_rect(img, box_rect, pal.border);

    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: box_rect.x + 10,
            y: box_rect.y + 8,
            text: rs.title(),
            color: pal.text,
            max_width_px: box_rect.width.saturating_sub(20),
            rtl: is_rtl,
        },
    );

    let mut tx = box_rect.x + 10;
    let tab_y = box_rect.y + 24;
    for (idx, tab) in rs.tabs.iter().take(4).enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let tw = ((tab.title.len() as u32) * 6 + 14).min(110);
        if tx + tw >= box_rect.right() {
            break;
        }
        let active = idx == view.active_tab_index;
        let trect = WorkspacePixelRect::new(tx, tab_y, tw, 18);
        fill_rect(img, trect, if active { pal.accent } else { pal.header });
        stroke_rect(img, trect, pal.border);
        draw_text_clipped(img, trect.x + 5, trect.y + 5, &tab.title, if active { Rgba8::WHITE } else { pal.text }, tw.saturating_sub(10));
        tx = tx.saturating_add(tw + 4);
    }

    let mut row_y = tab_y + 28;
    if let Some(banner) = AssumptionDiagnosticBanner::from_result(rs)
        && let Some(banner_rect) = compute_result_banner_rect(rect, rs)
    {
        let bg = match banner.severity {
            ChecklistStatus::Violated => pal.link_red,
            ChecklistStatus::Warn => Rgba8::rgb(0xD5, 0x5E, 0x00),
            ChecklistStatus::Info | ChecklistStatus::Pass => pal.link_blue,
        };
        fill_rect(img, banner_rect, bg);
        let msg = banner.format_banner_line();
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: banner_rect.x + 6,
                y: banner_rect.y + 5,
                text: &msg,
                color: Rgba8::WHITE,
                max_width_px: banner_rect.width.saturating_sub(12),
                rtl: is_rtl,
            },
        );
        row_y = banner_rect.bottom().saturating_add(6);
    }

    if let Some(tab) = view.active_tab(rs) {
        for (r_idx, row) in tab.rows.iter().take(6).enumerate() {
            if row_y + 16 >= box_rect.bottom().saturating_sub(56) {
                break;
            }
            let r_label = tab.row_labels.get(r_idx).map_or("", String::as_str);
            let first_val = row.first().map(|c| ResultSheetViewState::format_result_cell(c, rs.analysis.p_value_format)).unwrap_or_default();
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: box_rect.x + 12,
                    y: row_y,
                    text: &format!("{r_label}: {first_val}"),
                    color: pal.text,
                    max_width_px: box_rect.width.saturating_sub(24),
                    rtl: is_rtl,
                },
            );
            row_y = row_y.saturating_add(16);
        }
    }

    let chk_y = box_rect.bottom().saturating_sub(54).max(row_y + 6);
    fill_rect(img, WorkspacePixelRect::new(box_rect.x + 8, chk_y, box_rect.width.saturating_sub(16), 1), pal.border);
    draw_text_5x7(img, box_rect.x + 10, chk_y + 6, "ASSUMPTION CHECKLIST", pal.muted);
    let mut cy = chk_y + 18;
    for item in view.visible_checklist(rs).into_iter().take(2) {
        if cy + 12 >= box_rect.bottom() {
            break;
        }
        let status_color = match item.status {
            ChecklistStatus::Pass => pal.link_green,
            ChecklistStatus::Warn => Rgba8::rgb(0xD5, 0x5E, 0x00),
            ChecklistStatus::Violated => pal.link_red,
            ChecklistStatus::Info => pal.link_blue,
        };
        fill_rect(img, WorkspacePixelRect::new(box_rect.x + 10, cy + 1, 8, 8), status_color);
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: box_rect.x + 24,
                y: cy,
                text: &item.assumption,
                color: pal.text,
                max_width_px: box_rect.width.saturating_sub(32),
                rtl: is_rtl,
            },
        );
        cy = cy.saturating_add(14);
    }
}

fn draw_info_view(img: &mut RasterImage, rect: WorkspacePixelRect, info: &InfoSheet, app: &UiApp, pal: &FramePalette, is_rtl: bool) {
    let box_rect = WorkspacePixelRect::new(rect.x + 8, rect.y + 8, rect.width.saturating_sub(16), rect.height.saturating_sub(16));
    fill_rect(img, box_rect, pal.surface);
    stroke_rect(img, box_rect, pal.border);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: box_rect.x + 10,
            y: box_rect.y + 8,
            text: &info.title,
            color: pal.text,
            max_width_px: box_rect.width.saturating_sub(20),
            rtl: is_rtl,
        },
    );

    let split_x = box_rect.x + (box_rect.width / 2);
    fill_rect(img, WorkspacePixelRect::new(split_x, box_rect.y + 24, 1, box_rect.height.saturating_sub(28)), pal.border);
    let geom = compute_info_geometry(rect, info, is_rtl);
    for row in &geom.rows {
        let editing_row = matches!(
            (&app.inline_editor.active, &app.inline_editor.target),
            (true, Some(InlineEditorTarget::InfoConstant { sheet_id, name })) if sheet_id == &info.id && name == &row.name
        );
        let line = if editing_row {
            stroke_rect(img, row.rect, pal.accent);
            format!("{}: {}", row.name, app.inline_editor.buffer)
        } else {
            format!("{}: {}", row.name, row.value_text)
        };
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: row.rect.x + 2,
                y: row.rect.y + 3,
                text: &line,
                color: pal.text,
                max_width_px: row.rect.width.saturating_sub(4),
                rtl: is_rtl,
            },
        );
    }

    let editing_notes = matches!(
        (&app.inline_editor.active, &app.inline_editor.target),
        (true, Some(InlineEditorTarget::InfoNotes { sheet_id })) if sheet_id == &info.id
    );
    let notes_text = if editing_notes {
        stroke_rect(img, geom.notes_rect, pal.accent);
        app.inline_editor.buffer.as_str()
    } else {
        info.notes_markdown.as_str()
    };
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: geom.notes_rect.x + 2,
            y: geom.notes_rect.y + 4,
            text: notes_text,
            color: if editing_notes { pal.text } else { pal.muted },
            max_width_px: geom.notes_rect.width.saturating_sub(4),
            rtl: is_rtl,
        },
    );
}

fn draw_inspector_pane(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    fill_rect(img, rect, pal.panel);
    let border_x = if is_rtl { rect.right().saturating_sub(1) } else { rect.x };
    fill_rect(img, WorkspacePixelRect::new(border_x, rect.y, 1, rect.height), pal.border);
    let text_w = rect.width.saturating_sub(18);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 10,
            y: rect.y + 8,
            text: app.canvas_dialogs.active_inspector.label(),
            color: pal.text,
            max_width_px: text_w,
            rtl: is_rtl,
        },
    );
    fill_rect(img, WorkspacePixelRect::new(rect.x + 8, rect.y + 22, rect.width.saturating_sub(16), 1), pal.border);

    match app.canvas_dialogs.active_inspector {
        ActiveInspector::FormatGraph => draw_inspector_format_graph(img, rect, app, pal, is_rtl),
        ActiveInspector::FormatAxes => {
            let draft = &app.canvas_dialogs.format_axes.axis_draft;
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 30,
                    text: &format!("Segments: {}", draft.segments.len()),
                    color: pal.text,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
            for (idx, seg) in draft.segments.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                let y = rect.y + 46 + (idx as u32) * 16;
                draw_text_clipped_dir(
                    img,
                    DrawTextClipped {
                        x: rect.x + 10,
                        y,
                        text: &format!("S{}: {:?}", idx + 1, seg.scale),
                        color: pal.muted,
                        max_width_px: text_w,
                        rtl: is_rtl,
                    },
                );
            }
        }
        ActiveInspector::PairwiseComparisons => {
            let count = app.canvas_dialogs.pairwise.brackets.len();
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 30,
                    text: &format!("Brackets: {count}"),
                    color: pal.text,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
        }
        ActiveInspector::BubblePlot => {
            let bp = &app.canvas_dialogs.bubble_plot;
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 30,
                    text: &format!("X col {} / Y col {}", bp.x_column, bp.y_column),
                    color: pal.text,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 46,
                    text: if bp.show_convex_hulls { "Hulls: On" } else { "Hulls: Off" },
                    color: pal.muted,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
        }
        ActiveInspector::PageLayout => {
            let pl = &app.canvas_dialogs.page_layout;
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 30,
                    text: &format!("Grid: {}x{}", pl.rows, pl.cols),
                    color: pal.text,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
            draw_text_clipped_dir(
                img,
                DrawTextClipped {
                    x: rect.x + 10,
                    y: rect.y + 46,
                    text: &format!("Items: {}", pl.items.len()),
                    color: pal.muted,
                    max_width_px: text_w,
                    rtl: is_rtl,
                },
            );
        }
        ActiveInspector::ResultDetails | ActiveInspector::None => {
            draw_text_clipped_dir(
                img,
                DrawTextClipped { x: rect.x + 10, y: rect.y + 30, text: "Ready", color: pal.muted, max_width_px: text_w, rtl: is_rtl },
            );
        }
    }
}

fn draw_inspector_format_graph(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette, is_rtl: bool) {
    let text_w = rect.width.saturating_sub(18);
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 10,
            y: rect.y + 30,
            text: app.canvas_dialogs.format_graph.palette.label(),
            color: pal.muted,
            max_width_px: text_w,
            rtl: is_rtl,
        },
    );
    let mut local_x = 10u32;
    for i in 0..6 {
        if local_x + 18 >= rect.width {
            break;
        }
        let sx = rect.x.saturating_add(crate::i18n::mirror_x_for_rtl(local_x, 16, rect.width, is_rtl));
        let sw = WorkspacePixelRect::new(sx, rect.y + 44, 16, 14);
        fill_rect(img, sw, app.canvas_dialogs.format_graph.preview_swatch(i));
        stroke_rect(img, sw, pal.border);
        local_x = local_x.saturating_add(20);
    }
    let frame_lbl = format!("Frame: {:?}", app.canvas_dialogs.format_graph.frame_preset);
    draw_text_clipped_dir(
        img,
        DrawTextClipped { x: rect.x + 10, y: rect.y + 68, text: &frame_lbl, color: pal.text, max_width_px: text_w, rtl: is_rtl },
    );
}

fn draw_status_bar(img: &mut RasterImage, rect: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    fill_rect(img, rect, pal.header);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.y, rect.width, 1), pal.border);

    let cursor_str = format!("Cursor: {:?}", app.active_cursor());
    let stats_str = app
        .grid
        .quick_stats
        .filter(|s| s.numeric_count > 0)
        .and_then(|s| s.mean.map(|m| format!("n={} Mean={}", s.numeric_count, format_localized_compact_f64(m, app.locale_preset.decimal_separator))))
        .unwrap_or_default();
    let banner = app.canvas_dialogs.status_banner.as_deref().unwrap_or("");
    let left_text = [cursor_str, stats_str, String::from(banner)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<String>>().join(" | ");
    draw_text_clipped_dir(
        img,
        DrawTextClipped {
            x: rect.x + 10,
            y: rect.y + (rect.height.saturating_sub(7)) / 2,
            text: &left_text,
            color: pal.text,
            max_width_px: rect.width.saturating_sub(20),
            rtl: is_rtl,
        },
    );
}

fn format_localized_compact_f64(value: f64, decimal_separator: char) -> String {
    let text = format_compact_f64(value);
    if decimal_separator != '.' {
        let mut buf = [0u8; 4];
        text.replace('.', decimal_separator.encode_utf8(&mut buf))
    } else {
        text
    }
}

fn draw_modal_overlay(img: &mut RasterImage, backdrop: WorkspacePixelRect, modal: WorkspacePixelRect, app: &UiApp, pal: &FramePalette) {
    let is_rtl = crate::i18n::is_rtl_locale(&app.locale);
    dim_rect(img, backdrop);
    fill_rect(img, modal, pal.surface);
    stroke_rect(img, modal, pal.accent);

    let header = WorkspacePixelRect::new(modal.x, modal.y, modal.width, 24u32.min(modal.height));
    fill_rect(img, header, pal.accent);
    if header.height >= 12 && header.width > 20 {
        let title = modal_title(&app.canvas_dialogs.active_dialog);
        draw_text_clipped_dir(
            img,
            DrawTextClipped {
                x: header.x + 10,
                y: header.y + 8,
                text: title,
                color: Rgba8::WHITE,
                max_width_px: header.width.saturating_sub(20),
                rtl: is_rtl,
            },
        );
    }

    let body_y = header.bottom().saturating_add(8);
    let lines = modal_preview_lines(app);
    for (idx, line) in lines.into_iter().take(6).enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let ly = body_y + (idx as u32) * 16;
        if ly + 10 >= modal.bottom() {
            break;
        }
        draw_text_clipped_dir(
            img,
            DrawTextClipped { x: modal.x + 12, y: ly, text: &line, color: pal.text, max_width_px: modal.width.saturating_sub(24), rtl: is_rtl },
        );
    }

    if app.canvas_dialogs.active_dialog == ActiveDialog::PowerAndSampleSize
        && app.canvas_dialogs.power_wizard.stage == crate::power_wizard::PowerWizardStage::Results
    {
        draw_power_wizard_curve_preview(img, modal, body_y.saturating_add(72), app, pal);
    }
}

fn draw_power_wizard_curve_preview(img: &mut RasterImage, modal: WorkspacePixelRect, top_y: u32, app: &UiApp, pal: &FramePalette) {
    let Some(res) = app.canvas_dialogs.power_wizard.result.as_ref() else {
        return;
    };
    if res.power_curve.len() < 2 || modal.width <= 34 || top_y + 36 >= modal.bottom() {
        return;
    }
    let plot_h = (modal.bottom().saturating_sub(top_y + 12)).min(84);
    let plot_w = modal.width.saturating_sub(28);
    let plot = WorkspacePixelRect::new(modal.x + 14, top_y, plot_w, plot_h);
    fill_rect(img, plot, pal.surface);
    stroke_rect(img, plot, pal.border);

    let n_lo = res.power_curve.first().map_or(2.0, |p| p.n);
    let n_hi = res.power_curve.last().map_or(n_lo + 1.0, |p| p.n).max(n_lo + 1e-6);
    for pt in &res.power_curve {
        let fx = clamp_f64_safe((pt.n - n_lo) / (n_hi - n_lo), 0.0, 1.0);
        let fy = clamp_f64_safe(1.0 - clamp_f64_safe(pt.power, 0.0, 1.0), 0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let px = plot.x + (fx * f64::from(plot.width.saturating_sub(6))).round() as u32 + 2;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let py = plot.y + (fy * f64::from(plot.height.saturating_sub(6))).round() as u32 + 2;
        fill_rect(img, WorkspacePixelRect::new(px.saturating_sub(1), py.saturating_sub(1), 3, 3), pal.accent);
    }
    if let (Some(hn), Some(hp)) = (app.canvas_dialogs.power_wizard.hover_n, app.canvas_dialogs.power_wizard.hover_power) {
        let fx = clamp_f64_safe((hn - n_lo) / (n_hi - n_lo), 0.0, 1.0);
        let fy = clamp_f64_safe(1.0 - clamp_f64_safe(hp, 0.0, 1.0), 0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let px = plot.x + (fx * f64::from(plot.width.saturating_sub(6))).round() as u32 + 2;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let py = plot.y + (fy * f64::from(plot.height.saturating_sub(6))).round() as u32 + 2;
        fill_rect(img, WorkspacePixelRect::new(px.saturating_sub(3), py.saturating_sub(3), 7, 7), pal.link_red);
    }
}

fn modal_title(dialog: &ActiveDialog) -> &'static str {
    match dialog {
        ActiveDialog::None => "",
        ActiveDialog::AnalyzeSheet { .. } => "Analyze...",
        ActiveDialog::DuplicateAnalysisPrompt { .. } => "Analysis Already Exists",
        ActiveDialog::EditAnalysisParameters { .. } => "Edit Analysis Parameters (Cmd+T)",
        ActiveDialog::ChangeDataAnalyzed { .. } => "Change Data Analyzed",
        ActiveDialog::ResultViewer { .. } => "Result Sheet Viewer",
        ActiveDialog::ConstantHookPicker { .. } => "Insert Hooked Constant",
        ActiveDialog::ChangeGraphType { .. } => "Change Graph Type",
        ActiveDialog::FormatGraph { .. } => "Format Graph",
        ActiveDialog::FormatAxes { .. } => "Format Axes",
        ActiveDialog::PairwiseComparisons { .. } => "Pairwise Comparisons",
        ActiveDialog::BubblePlotInspector { .. } => "Bubble Plot Channels",
        ActiveDialog::ApplyStyleFrom { .. } => "Apply Style From...",
        ActiveDialog::PageLayoutComposer { .. } => "Page Layout Composer",
        ActiveDialog::ExportFigure { .. } => "Export Publication Figure",
        ActiveDialog::ConvertArchetype { .. } => "Convert Table Archetype...",
        ActiveDialog::ImportReshape => "Import Reshape (Plate Map / Tidy Long)",
        ActiveDialog::ExcludeWithReason { .. } => "Exclude Selection with Reason",
        ActiveDialog::FormatSelectedPoints { .. } => "Format Selected Points...",
        ActiveDialog::PowerAndSampleSize => "Power and Sample Size",
        ActiveDialog::Print { .. } => "Print...",
        ActiveDialog::RandomizeGroups => "Randomize Subjects to Groups...",
        ActiveDialog::RandomSampleRows { .. } => "Random Sample Rows...",
        ActiveDialog::SimulateDistributions => "Simulate Distributions...",
        ActiveDialog::InsertModelEquation { .. } => "Insert Model Equation...",
        ActiveDialog::CommandPalette => "Command Palette (Cmd+K)",
        ActiveDialog::About => "About LumenCraft",
    }
}

fn modal_preview_lines(app: &UiApp) -> Vec<String> {
    let state = &app.canvas_dialogs;
    match &state.active_dialog {
        ActiveDialog::PowerAndSampleSize => {
            let mut lines = vec![format!("Stage: {}", state.power_wizard.stage.label())];
            lines.extend(state.power_wizard.summary_lines());
            lines
        }
        ActiveDialog::FormatSelectedPoints { .. } => {
            let fp = &state.format_points;
            vec![
                format!("Points selected: {}", fp.selected_points.len()),
                format!("Color: {} | Symbol: {}", fp.color.as_deref().unwrap_or("keep"), fp.symbol.as_deref().unwrap_or("keep")),
                format!("Show row labels: {} | Clear: {}", fp.show_row_labels, fp.clear_overrides),
            ]
        }
        ActiveDialog::AnalyzeSheet { .. } => {
            let mut rows = vec![format!("Search: {}", state.analyze.search_query)];
            for item in state.analyze.compatible_entries().into_iter().take(5) {
                rows.push(format!("- {}", item.label));
            }
            rows
        }
        ActiveDialog::CommandPalette => state.command_palette.preview_rows_for_locale(Some(app.session()), &app.locale, 5),
        ActiveDialog::ChangeGraphType { .. } => {
            state.change_graph_type.recommended_options().into_iter().take(5).map(|o| format!("[*] {}", o.label)).collect()
        }
        ActiveDialog::ApplyStyleFrom { .. } => {
            state.apply_style.donor_candidates.iter().take(5).map(|(id, title)| format!("Donor: {title} ({})", id.as_str())).collect()
        }
        ActiveDialog::ConvertArchetype { .. } => {
            let mut lines = vec![format!("Target: {}", state.convert_archetype.target_kind.label())];
            if state.convert_archetype.preview_ready {
                lines.push(format!(
                    "Preserved cells: {}, dropped: {}, lossy: {}",
                    state.convert_archetype.preserved_cells, state.convert_archetype.dropped_cells, state.convert_archetype.preview_lossy
                ));
                for w in state.convert_archetype.preview_warnings.iter().take(2) {
                    lines.push(format!("Warning: {w}"));
                }
            }
            lines
        }
        ActiveDialog::ImportReshape => {
            vec![
                format!("Mode: {:?}", app.grid.import_reshape.mode),
                format!("Rows: {} / Wells: {}", app.grid.import_reshape.tidy_csv.lines().count(), app.grid.import_reshape.plate_values.len()),
            ]
        }
        ActiveDialog::ExcludeWithReason { .. } => {
            vec![
                format!("Reason: {}", state.exclude_with_reason.reason_tag),
                format!("Note: {}", state.exclude_with_reason.note),
                format!("Cells selected: {}", state.exclude_with_reason.coords.len()),
            ]
        }
        ActiveDialog::Print { sheet_id } => {
            let p = &state.print;
            let printer_label = p.selected_printer.as_deref().unwrap_or("System Default");
            let sheet_label = sheet_id.as_ref().map(SheetId::as_str).or_else(|| p.job.sheet_ids.first().map(SheetId::as_str)).unwrap_or("None");
            let mut lines = vec![
                format!("Sheet: {sheet_label} | Copies: {} | Scale: {:.0}%", p.copies, p.job.scale_percent),
                format!("Printer: {printer_label} ({} queue(s) found)", p.discovered_printers.len()),
                format!("Crop marks: {} | Spool bytes: {}", p.job.crop_marks, p.last_spool_bytes),
            ];
            if let Some(status) = &p.last_status {
                lines.push(format!("Status: {status}"));
            }
            lines
        }
        ActiveDialog::RandomizeGroups => {
            let rg = &state.randomize_groups;
            vec![format!("Groups: {} | Subjects: {}", rg.groups.len(), rg.subject_count), format!("Scheme: {:?} | Seed: {}", rg.scheme, rg.seed)]
        }
        ActiveDialog::RandomSampleRows { source_sheet_id } => {
            let rs = &state.random_sample_rows;
            vec![
                format!("Source: {} | Sample N: {}", source_sheet_id.as_ref().map_or("None", SheetId::as_str), rs.sample_size),
                format!("Seed: {} | Stratify: {:?}", rs.seed, rs.stratify_column),
            ]
        }
        ActiveDialog::SimulateDistributions => {
            let sd = &state.simulate_distributions;
            vec![
                format!("Columns: {} | Rows: {}", sd.columns.len(), sd.n_rows),
                format!("Seed: {} | MV table: {}", sd.seed, sd.use_multiple_variables),
            ]
        }
        ActiveDialog::InsertModelEquation { graph_sheet_id } => {
            let eq = &state.insert_model_equation;
            vec![
                format!("Graph: {} | Result: {}", graph_sheet_id.as_str(), eq.result_sheet_id.as_ref().map_or("None", SheetId::as_str)),
                format!("Mode: {:?} | Dataset: {}", eq.mode, eq.data_set_index),
            ]
        }
        _ => vec![String::from("Active scientific dialog ready")],
    }
}

// ============================================================================
// Low-Level Safe Software Raster Primitives & Compact 5x7 ASCII Font
// ============================================================================

fn fill_rect(img: &mut RasterImage, rect: WorkspacePixelRect, color: Rgba8) {
    let x_end = rect.right().min(img.width_px);
    let y_end = rect.bottom().min(img.height_px);
    for py in rect.y..y_end {
        for px in rect.x..x_end {
            img.blend_pixel(px, py, color);
        }
    }
}

fn stroke_rect(img: &mut RasterImage, rect: WorkspacePixelRect, color: Rgba8) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.y, rect.width, 1), color);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.bottom().saturating_sub(1), rect.width, 1), color);
    fill_rect(img, WorkspacePixelRect::new(rect.x, rect.y, 1, rect.height), color);
    fill_rect(img, WorkspacePixelRect::new(rect.right().saturating_sub(1), rect.y, 1, rect.height), color);
}

fn dim_rect(img: &mut RasterImage, rect: WorkspacePixelRect) {
    fill_rect(img, rect, Rgba8::rgba(16, 20, 28, 110));
}

fn blit_image(dst: &mut RasterImage, dst_x: u32, dst_y: u32, src: &RasterImage) {
    let w = src.width_px.min(dst.width_px.saturating_sub(dst_x));
    let h = src.height_px.min(dst.height_px.saturating_sub(dst_y));
    for sy in 0..h {
        for sx in 0..w {
            let idx = ((sy as usize) * (src.width_px as usize) + (sx as usize)) * 4;
            if let Some(&[r, g, b, a]) = src.rgba.get(idx..idx + 4) {
                dst.blend_pixel(dst_x + sx, dst_y + sy, Rgba8::rgba(r, g, b, a));
            }
        }
    }
}

struct DrawTextClipped<'a> {
    x: u32,
    y: u32,
    text: &'a str,
    color: Rgba8,
    max_width_px: u32,
    rtl: bool,
}

fn draw_text_clipped(img: &mut RasterImage, x: u32, y: u32, text: &str, color: Rgba8, max_width_px: u32) {
    draw_text_clipped_dir(img, DrawTextClipped { x, y, text, color, max_width_px, rtl: false });
}

fn draw_text_clipped_dir(img: &mut RasterImage, spec: DrawTextClipped<'_>) {
    let reordered;
    let display = if spec.rtl {
        reordered = reorder_visual_bidi(spec.text, TextDirection::Rtl);
        reordered.as_str()
    } else {
        spec.text
    };
    let max_chars = (spec.max_width_px / 6) as usize;
    let count = display.chars().count().min(max_chars);
    #[allow(clippy::cast_possible_truncation)]
    let draw_w = (count as u32).saturating_mul(6).saturating_sub(1);
    let mut cx = if spec.rtl { spec.x.saturating_add(spec.max_width_px.saturating_sub(draw_w)) } else { spec.x };
    for ch in display.chars().take(max_chars) {
        draw_glyph_5x7(img, cx, spec.y, ch, spec.color);
        cx = cx.saturating_add(6);
    }
}

fn draw_text_5x7(img: &mut RasterImage, x: u32, y: u32, text: &str, color: Rgba8) {
    draw_text_5x7_dir(img, x, y, text, color, false);
}

fn draw_text_5x7_dir(img: &mut RasterImage, x: u32, y: u32, text: &str, color: Rgba8, rtl: bool) {
    let reordered;
    let display = if rtl {
        reordered = reorder_visual_bidi(text, TextDirection::Rtl);
        reordered.as_str()
    } else {
        text
    };
    let mut cx = x;
    for ch in display.chars() {
        draw_glyph_5x7(img, cx, y, ch, color);
        cx = cx.saturating_add(6);
    }
}

fn draw_glyph_5x7(img: &mut RasterImage, x: u32, y: u32, ch: char, color: Rgba8) {
    let rows = glyph_5x7(ch.to_ascii_uppercase());
    for (dy, bits) in rows.into_iter().enumerate() {
        for dx in 0..5u32 {
            if (bits & (1 << (4 - dx))) != 0 {
                #[allow(clippy::cast_possible_truncation)]
                img.blend_pixel(x.saturating_add(dx), y.saturating_add(dy as u32), color);
            }
        }
    }
}

const FONT_5X7: &[(u8, [u8; 7])] = &[
    (b' ', [0, 0, 0, 0, 0, 0, 0]),
    (b'!', [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100]),
    (b'#', [0b01010, 0b11111, 0b01010, 0b01010, 0b11111, 0b01010, 0]),
    (b'%', [0b11001, 0b11010, 0b00100, 0b01000, 0b10110, 0b10011, 0]),
    (b'+', [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0]),
    (b',', [0, 0, 0, 0, 0, 0b00100, 0b01000]),
    (b'-', [0, 0, 0, 0b11111, 0, 0, 0]),
    (b'.', [0, 0, 0, 0, 0, 0b01100, 0b01100]),
    (b'/', [0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0, 0]),
    (b':', [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0]),
    (b'=', [0, 0, 0b11111, 0, 0b11111, 0, 0]),
    (b'>', [0b10000, 0b01000, 0b00100, 0b01000, 0b10000, 0, 0]),
    (b'(', [0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010]),
    (b')', [0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000]),
    (b'[', [0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110]),
    (b']', [0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110]),
    (b'|', [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    (b'*', [0b00100, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0b00100]),
    (b'0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    (b'1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    (b'2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    (b'3', [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110]),
    (b'4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    (b'5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    (b'6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    (b'7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    (b'8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    (b'9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    (b'A', [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    (b'B', [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110]),
    (b'C', [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
    (b'D', [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100]),
    (b'E', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
    (b'F', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000]),
    (b'G', [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111]),
    (b'H', [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    (b'I', [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    (b'J', [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100]),
    (b'K', [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001]),
    (b'L', [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111]),
    (b'M', [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001]),
    (b'N', [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001]),
    (b'O', [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    (b'P', [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
    (b'Q', [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101]),
    (b'R', [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
    (b'S', [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
    (b'T', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    (b'U', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    (b'V', [0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b01010, 0b00100]),
    (b'W', [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001]),
    (b'X', [0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b01010, 0b10001]),
    (b'Y', [0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    (b'Z', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111]),
];

fn glyph_5x7(ch: char) -> [u8; 7] {
    if !ch.is_ascii() {
        return [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111];
    }
    let byte = ch as u8;
    FONT_5X7.iter().find(|(k, _)| *k == byte).map_or([0b00000, 0b01110, 0b01010, 0b01010, 0b01110, 0b00000, 0b00000], |(_, rows)| *rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_viewport_with_open_dialog_does_not_panic() {
        assert_eq!(clamp_u32_safe(38, 180, 40), 40);
        assert_eq!(clamp_u32_safe(0, 180, 0), 0);

        let mut app = UiApp::default();
        app.canvas_dialogs.active_dialog = ActiveDialog::CommandPalette;

        let degenerate = compute_workspace_layout(&app, 0, 0);
        assert_eq!(degenerate.modal_overlay, Some(WorkspacePixelRect::new(0, 0, 0, 0)));

        for (w, h) in [(10, 10), (64, 64), (160, 100), (200, 400), (400, 140), (204, 144)] {
            let layout = compute_workspace_layout(&app, w, h);
            let modal = layout.modal_overlay.expect("dialog overlay must exist");
            assert!(modal.right() <= w);
            assert!(modal.bottom() <= h);

            let (png, cw, ch) =
                render_workspace_region_png(&app, RenderTargetSpec { width_px: w, height_px: h, dpi: 96 }, ScreenshotRegionTarget::Dialog)
                    .expect("small viewport screenshot should succeed");
            assert!(!png.is_empty());
            assert!(cw <= w && ch <= h && cw > 0 && ch > 0);
        }

        let mut ctrl_app = UiApp::new();
        let _ = crate::control_io::handle_control_line(&mut ctrl_app, r#"{"id":1,"op":"ui.palette.open"}"#);
        for (w, h) in [(64u32, 64u32), (200, 400), (400, 140), (204, 144)] {
            let req = format!(r#"{{"id":2,"op":"ui.screenshot","params":{{"width_px":{w},"height_px":{h}}}}}"#);
            let resp = crate::control_io::handle_control_line(&mut ctrl_app, &req);
            assert!(resp.contains("\"ok\":true"), "expected ok response for {w}x{h}: {resp}");
        }
    }
}
