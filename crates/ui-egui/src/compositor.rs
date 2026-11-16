//! Retained software frame compositor (`RasterImage` + PNG for `ui.screenshot`).
//!
//! Computes region geometry via [`crate::views::compute_frame_geometry`], projects unified
//! [`crate::layout::LayoutTree`] frames via [`crate::layout::paint::paint_layout_tree`] using
//! proportional/tabular coverage-mask text (`crate::text_bridge`, retiring `FONT_5X7` per `R10.9`),
//! and preserves [`WorkspaceLayoutMap`] for physical/RTL interaction and accessibility tools.

use lumencraft_palette::Rgba8;
use lumencraft_render::{RasterImage, RenderError, RenderTargetSpec, encode_png};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::{ViewportMetrics, WorkspaceLayoutMap};
use crate::layout::paint::paint_layout_tree;
use crate::text_bridge::DesktopTextMeasurer;

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

/// Active theme-derived color tokens passed to view builders (`WCAG 2.1 AA` verified).
#[derive(Debug, Clone, Copy)]
pub struct FramePalette {
    pub panel: Rgba8,
    pub surface: Rgba8,
    pub text: Rgba8,
    pub muted: Rgba8,
    pub accent: Rgba8,
    pub header: Rgba8,
    pub border: Rgba8,
    pub canvas_mat: Rgba8,
    pub excluded: Rgba8,
    pub link_blue: Rgba8,
    pub link_green: Rgba8,
    pub link_red: Rgba8,
}

impl FramePalette {
    #[must_use]
    pub fn from_theme(theme: &crate::theme::UiThemeSpec) -> Self {
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

/// Compute deterministic UI region bounds for a frame of `(width_px, height_px)`.
#[must_use]
pub fn compute_workspace_layout(app: &UiApp, width_px: u32, height_px: u32) -> WorkspaceFrameLayout {
    crate::views::compute_frame_geometry(app, width_px, height_px)
}

/// Compute the full physical-pixel layout & hit-testing map for `app` under `metrics` (`G6`, `M7`).
#[must_use]
pub fn compute_workspace_layout_map(app: &UiApp, metrics: ViewportMetrics) -> WorkspaceLayoutMap {
    app.layout_tree_cow(metrics).to_workspace_layout_map()
}

/// Render the full application frame deterministically into an RGBA [`RasterImage`] via [`crate::layout::LayoutTree`].
pub fn render_workspace_rgba(app: &UiApp, target: RenderTargetSpec) -> Result<RasterImage, RenderError> {
    let mut img = RasterImage::new(target.width_px, target.height_px, target.dpi, app.theme.surface_bg)?;
    let scale = (f64::from(target.dpi.max(48)) / 96.0).clamp(0.75, 4.0);
    let metrics = ViewportMetrics::new((f64::from(target.width_px) / scale).max(1.0), (f64::from(target.height_px) / scale).max(1.0), scale);
    let tree = app.layout_tree_cow(metrics);
    paint_layout_tree(&tree, DesktopTextMeasurer::global(), &mut img);
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
    let mut sub = RasterImage::new(w, h, src.dpi, Rgba8::TRANSPARENT)?;
    for row in 0..h {
        let src_start = (((y0 + row) as usize) * (src.width_px as usize) + (x0 as usize)) * 4;
        let dst_start = (row as usize) * (w as usize) * 4;
        let bytes = (w as usize) * 4;
        if let (Some(src_slice), Some(dst_slice)) = (src.rgba.get(src_start..src_start + bytes), sub.rgba.get_mut(dst_start..dst_start + bytes)) {
            dst_slice.copy_from_slice(src_slice);
        }
    }
    Ok(sub)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs_canvas::ActiveDialog;

    #[test]
    fn small_viewport_with_open_dialog_does_not_panic() {
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
