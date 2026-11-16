//! Aspect-ratio centered canvas & layout viewport mapping, zoom-to-fit / zoom-% selector,
//! pan scrollbars, and hover/selected-only decoration visibility guards (`R10.15`).

use curvebook_doc::SheetId;

use super::LayoutRectPx;
use super::scroll::{compute_scrollbar_thumb, scrollbar_thickness_px};
use crate::UiApp;
use crate::interaction::ScenePaperMapping;

/// Supported zoom presets exposed in the canvas viewport Zoom-% selector.
pub const CANVAS_ZOOM_PRESETS: &[f64] = &[0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// Resolved geometry for rendering a physical paper/layout sheet inside the canvas mat.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasViewportGeometry {
    pub outer_pane_px: LayoutRectPx,
    pub mat_rect_px: LayoutRectPx,
    pub zoom_bar_rect_px: LayoutRectPx,
    pub zoom_fit_btn_px: LayoutRectPx,
    pub zoom_pct_btn_px: LayoutRectPx,
    pub base_paper_rect_px: LayoutRectPx,
    pub paper_rect_px: LayoutRectPx,
    pub mapping: ScenePaperMapping,
    pub h_scrollbar: Option<(LayoutRectPx, LayoutRectPx)>,
    pub v_scrollbar: Option<(LayoutRectPx, LayoutRectPx)>,
    pub max_pan_pt: [f64; 2],
}

/// Compute aspect-ratio-preserving paper geometry centered on the neutral canvas mat (`R10.15`).
#[must_use]
pub fn compute_aspect_ratio_paper_mapping(
    sheet_id: SheetId,
    outer_pane_px: LayoutRectPx,
    scene_width_pt: f64,
    scene_height_pt: f64,
    app: &UiApp,
    scale_factor: f64,
) -> CanvasViewportGeometry {
    let safe_w = if scene_width_pt.is_finite() && scene_width_pt > 1.0 { scene_width_pt } else { 420.0 };
    let safe_h = if scene_height_pt.is_finite() && scene_height_pt > 1.0 { scene_height_pt } else { 297.0 };

    let show_controls = outer_pane_px.width >= 160 && outer_pane_px.height >= 120;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bar_h = if show_controls { (22.0 * scale_factor.clamp(0.75, 3.0)).round() as u32 } else { 0 };
    let zoom_bar_rect_px = LayoutRectPx::new(
        outer_pane_px.x,
        outer_pane_px.y + i32::try_from(outer_pane_px.height.saturating_sub(bar_h)).unwrap_or(0),
        outer_pane_px.width,
        bar_h,
    );

    let avail_h = outer_pane_px.height.saturating_sub(bar_h);
    let sb_thick = scrollbar_thickness_px(scale_factor);
    let (eff_zoom, eff_pan) = app.canvas_dialogs.effective_zoom_and_pan();
    let zoom = if eff_zoom.is_finite() && eff_zoom > 0.0 { eff_zoom.clamp(0.25, 8.0) } else { 1.0 };
    let needs_scrollbars = show_controls && zoom > 1.02 && outer_pane_px.width > sb_thick + 40 && avail_h > sb_thick + 40;
    let mat_w = if needs_scrollbars { outer_pane_px.width.saturating_sub(sb_thick) } else { outer_pane_px.width };
    let mat_h = if needs_scrollbars { avail_h.saturating_sub(sb_thick) } else { avail_h };
    let mat_rect_px = LayoutRectPx::new(outer_pane_px.x, outer_pane_px.y, mat_w, mat_h);

    let pad_x = (mat_w / 24).clamp(2, 18);
    let pad_y = (mat_h / 24).clamp(2, 18);
    let inner_w = f64::from(mat_w.saturating_sub(pad_x.saturating_mul(2)).max(1));
    let inner_h = f64::from(mat_h.saturating_sub(pad_y.saturating_mul(2)).max(1));

    let fit_scale = (inner_w / safe_w).min(inner_h / safe_h).max(1e-4);
    let base_w_px = (safe_w * fit_scale).max(1.0);
    let base_h_px = (safe_h * fit_scale).max(1.0);

    #[allow(clippy::cast_possible_truncation)]
    let base_x = (f64::from(mat_rect_px.x) + (f64::from(mat_rect_px.width) - base_w_px) * 0.5).round() as i32;
    #[allow(clippy::cast_possible_truncation)]
    let base_y = (f64::from(mat_rect_px.y) + (f64::from(mat_rect_px.height) - base_h_px) * 0.5).round() as i32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let base_w_u = base_w_px.round().max(1.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let base_h_u = base_h_px.round().max(1.0) as u32;
    let base_paper_rect_px = LayoutRectPx::new(base_x, base_y, base_w_u, base_h_u);

    let scaled_w_px = (base_w_px * zoom).max(1.0);
    let scaled_h_px = (base_h_px * zoom).max(1.0);
    let paper_x_f = f64::from(base_x) + eff_pan[0] * fit_scale;
    let paper_y_f = f64::from(base_y) + eff_pan[1] * fit_scale;

    #[allow(clippy::cast_possible_truncation)]
    let raw_px_x = paper_x_f.round() as i32;
    #[allow(clippy::cast_possible_truncation)]
    let raw_px_y = paper_y_f.round() as i32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let paper_w_u = scaled_w_px.round().max(1.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let paper_h_u = scaled_h_px.round().max(1.0) as u32;
    let paper_rect_px = LayoutRectPx::new(raw_px_x, raw_px_y, paper_w_u, paper_h_u);

    let mapping = ScenePaperMapping {
        sheet_id,
        paper: base_paper_rect_px.to_workspace_rect(),
        scene_width_pt: safe_w,
        scene_height_pt: safe_h,
        zoom,
        pan_pt: eff_pan,
    };

    let content_w_u = paper_w_u.saturating_add(pad_x.saturating_mul(2));
    let content_h_u = paper_h_u.saturating_add(pad_y.saturating_mul(2));
    let max_pan_x = (f64::from(content_w_u.saturating_sub(mat_w)) * 0.5 / fit_scale).max(0.0);
    let max_pan_y = (f64::from(content_h_u.saturating_sub(mat_h)) * 0.5 / fit_scale).max(0.0);

    let h_scrollbar = if needs_scrollbars && content_w_u > mat_w {
        let track = LayoutRectPx::new(mat_rect_px.x, mat_rect_px.y + i32::try_from(mat_h).unwrap_or(0), mat_w, sb_thick);
        let offset = (max_pan_x - eff_pan[0]) * fit_scale;
        compute_scrollbar_thumb(track, mat_w, content_w_u, offset, false)
    } else {
        None
    };

    let v_scrollbar = if needs_scrollbars && content_h_u > mat_h {
        let track = LayoutRectPx::new(mat_rect_px.x + i32::try_from(mat_w).unwrap_or(0), mat_rect_px.y, sb_thick, mat_h);
        let offset = (max_pan_y - eff_pan[1]) * fit_scale;
        compute_scrollbar_thumb(track, mat_h, content_h_u, offset, true)
    } else {
        None
    };

    let btn_w = (zoom_bar_rect_px.width / 4).clamp(44, 110);
    let zoom_fit_btn_px = if show_controls {
        LayoutRectPx::new(zoom_bar_rect_px.x + 6, zoom_bar_rect_px.y + 2, btn_w, bar_h.saturating_sub(4).max(12))
    } else {
        LayoutRectPx::default()
    };
    let zoom_pct_btn_px = if show_controls {
        LayoutRectPx::new(
            zoom_fit_btn_px.x + i32::try_from(zoom_fit_btn_px.width).unwrap_or(0) + 6,
            zoom_bar_rect_px.y + 2,
            btn_w,
            bar_h.saturating_sub(4).max(12),
        )
    } else {
        LayoutRectPx::default()
    };

    CanvasViewportGeometry {
        outer_pane_px,
        mat_rect_px,
        zoom_bar_rect_px,
        zoom_fit_btn_px,
        zoom_pct_btn_px,
        base_paper_rect_px,
        paper_rect_px,
        mapping,
        h_scrollbar,
        v_scrollbar,
        max_pan_pt: [max_pan_x, max_pan_y],
    }
}

/// True only when a graph series chip should be visibly rendered (hovered or brushed/selected) (`R10.15`).
#[must_use]
pub fn should_show_graph_series_chip(app: &UiApp, series_index: usize) -> bool {
    let hover_prefix = format!("canvas.series.{series_index}");
    if app.interaction.hovered_widget_id.as_deref().is_some_and(|id| id == hover_prefix || id.starts_with("canvas.point.")) {
        return true;
    }
    if let Some(ref cell) = app.brushed_cell
        && cell.col == series_index + 1
    {
        return true;
    }
    !app.brushed_points.is_empty() && app.brushed_points.iter().any(|c| c.col == series_index + 1)
}

/// True only when layout-panel selection chrome/badges should be drawn (hovered or selected) (`R10.15`).
#[must_use]
pub fn should_show_layout_panel_chrome(app: &UiApp, panel_id: &str) -> bool {
    let selected = app.canvas_dialogs.canvas.selected_object_id.as_deref() == Some(panel_id);
    let hover_id = format!("canvas.layout_panel.{panel_id}");
    let hovered = app.interaction.hovered_widget_id.as_deref() == Some(hover_id.as_str());
    selected || hovered
}

/// Cycle to the next standard zoom percentage in [`CANVAS_ZOOM_PRESETS`].
#[must_use]
pub fn next_zoom_preset(current_zoom: f64) -> f64 {
    for &preset in CANVAS_ZOOM_PRESETS {
        if preset > current_zoom + 0.05 {
            return preset;
        }
    }
    CANVAS_ZOOM_PRESETS[0]
}
