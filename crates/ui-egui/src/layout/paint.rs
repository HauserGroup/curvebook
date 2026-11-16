//! Z-ordered, clip-enforced CPU rasterizer for [`super::LayoutTree`] (`PaintOp` -> `RasterImage`).
//!
//! Renders every glyph through [`crate::text_bridge::SoftwareTextPainter`] without any bitmap
//! fallback tables (`FONT_5X7` retired cleanly per `R10.9`).

use std::sync::Arc;

use curvebook_palette::Rgba8;
use curvebook_render::RasterImage;

use super::{ClipId, LayoutRectPx, LayoutTree, WidgetId, ZLayer};
use crate::text_bridge::{DesktopTextMeasurer, PaintClipPx, ShapedTextLine, SoftwareTextPainter, TextPaintStyle, UiTextStyle};

/// Geometric or typographic primitive emitted by retained view builders.
#[derive(Debug, Clone, PartialEq)]
pub enum PaintOpKind {
    FillRect { rect: LayoutRectPx, color: Rgba8 },
    StrokeRect { rect: LayoutRectPx, color: Rgba8, thickness: u32 },
    DimRect { rect: LayoutRectPx, color: Rgba8 },
    LineSegment { p0: [i32; 2], p1: [i32; 2], color: Rgba8, thickness: u32 },
    CircleIndicator { center: [i32; 2], radius: u32, fill: Rgba8, stroke: Option<Rgba8> },
    TextRun { text: String, style: UiTextStyle, locale: String, origin_px: [f64; 2], color: Rgba8, shaped: ShapedTextLine },
    BlitImage { dst: LayoutRectPx, image: Arc<RasterImage> },
}

/// Z-layered, clip-bounded paint instruction owned by an optional [`WidgetId`].
#[derive(Debug, Clone, PartialEq)]
pub struct PaintOp {
    pub z_layer: ZLayer,
    pub paint_order: u32,
    pub clip: Option<ClipId>,
    pub owner_widget_id: Option<WidgetId>,
    pub kind: PaintOpKind,
}

impl PaintOp {
    /// Whether this instruction actually deposits visible pixels/glyphs within `clips`.
    #[must_use]
    pub fn emits_visible_geometry(&self, clips: &[LayoutRectPx], viewport: LayoutRectPx) -> bool {
        let clip_rect = resolve_clip_rect(self.clip, clips, viewport);
        if clip_rect.width == 0 || clip_rect.height == 0 {
            return false;
        }
        let target = match &self.kind {
            PaintOpKind::FillRect { rect, color } | PaintOpKind::StrokeRect { rect, color, .. } | PaintOpKind::DimRect { rect, color } => {
                if color.a == 0 {
                    return false;
                }
                *rect
            }
            PaintOpKind::LineSegment { p0, p1, color, .. } => {
                if color.a == 0 {
                    return false;
                }
                let min_x = p0[0].min(p1[0]);
                let min_y = p0[1].min(p1[1]);
                let w = u32::try_from((p0[0].max(p1[0]) - min_x).saturating_add(1)).unwrap_or(1);
                let h = u32::try_from((p0[1].max(p1[1]) - min_y).saturating_add(1)).unwrap_or(1);
                LayoutRectPx::new(min_x, min_y, w, h)
            }
            PaintOpKind::CircleIndicator { center, radius, fill, stroke } => {
                if fill.a == 0 && stroke.is_none_or(|c| c.a == 0) {
                    return false;
                }
                let r = i32::try_from(*radius).unwrap_or(0);
                let d = radius.saturating_mul(2).max(1);
                LayoutRectPx::new(center[0] - r, center[1] - r, d, d)
            }
            PaintOpKind::TextRun { origin_px, shaped, color, .. } => {
                if color.a == 0 {
                    return false;
                }
                #[allow(clippy::cast_possible_truncation)]
                let rx = origin_px[0].floor() as i32;
                #[allow(clippy::cast_possible_truncation)]
                let ry = (origin_px[1] - shaped.metrics.ascent_px).floor() as i32;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let rw = shaped.metrics.width_px.ceil().max(1.0) as u32;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let rh = shaped.metrics.line_height_px.ceil().max(1.0) as u32;
                LayoutRectPx::new(rx, ry, rw, rh)
            }
            PaintOpKind::BlitImage { dst, .. } => *dst,
        };
        let inter = target.intersect(clip_rect);
        inter.width > 0 && inter.height > 0
    }
}

/// Rasterize every ordered [`PaintOp`] inside `tree` into `img`.
pub fn paint_layout_tree(tree: &LayoutTree, measurer: &DesktopTextMeasurer, img: &mut RasterImage) {
    let viewport = LayoutRectPx::new(0, 0, img.width_px, img.height_px);
    for op in &tree.paint_ops {
        let clip_rect = resolve_clip_rect(op.clip, &tree.clips, viewport);
        if clip_rect.width == 0 || clip_rect.height == 0 {
            continue;
        }
        match &op.kind {
            PaintOpKind::FillRect { rect, color } => fill_clipped_rect(img, *rect, *color, clip_rect),
            PaintOpKind::StrokeRect { rect, color, thickness } => stroke_clipped_rect(img, *rect, *color, *thickness, clip_rect),
            PaintOpKind::DimRect { rect, color } => fill_clipped_rect(img, *rect, *color, clip_rect),
            PaintOpKind::LineSegment { p0, p1, color, thickness } => draw_clipped_line(img, *p0, *p1, *color, *thickness, clip_rect),
            PaintOpKind::CircleIndicator { center, radius, fill, stroke } => {
                draw_clipped_circle(img, *center, *radius, *fill, *stroke, clip_rect);
            }
            PaintOpKind::TextRun { style, locale, origin_px, color, shaped, .. } => {
                let clip = PaintClipPx::new(clip_rect.x, clip_rect.y, clip_rect.width, clip_rect.height);
                SoftwareTextPainter::paint_shaped_line(measurer, img, shaped, &TextPaintStyle { style, locale, color: *color, clip }, *origin_px);
            }
            PaintOpKind::BlitImage { dst, image } => blit_clipped_image(img, *dst, image, clip_rect),
        }
    }
}

fn resolve_clip_rect(clip: Option<ClipId>, clips: &[LayoutRectPx], viewport: LayoutRectPx) -> LayoutRectPx {
    clip.and_then(|c| clips.get(c.0).copied()).map_or(viewport, |r| r.intersect(viewport))
}

fn fill_clipped_rect(img: &mut RasterImage, rect: LayoutRectPx, color: Rgba8, clip: LayoutRectPx) {
    if color.a == 0 {
        return;
    }
    let vis = rect.intersect(clip);
    if vis.width == 0 || vis.height == 0 {
        return;
    }
    let x0 = vis.x as usize;
    let y0 = vis.y as usize;
    let w = vis.width as usize;
    let h = vis.height as usize;
    let img_w = img.width_px as usize;
    let row_bytes = w * 4;

    if color.a == u8::MAX {
        let px_bytes = [color.r, color.g, color.b, 255u8];
        let first_start = (y0 * img_w + x0) * 4;
        if let Some(first_row) = img.rgba.get_mut(first_start..first_start + row_bytes) {
            for chunk in first_row.as_chunks_mut::<4>().0 {
                *chunk = px_bytes;
            }
        } else {
            return;
        }
        for py in (y0 + 1)..(y0 + h) {
            let dst_start = (py * img_w + x0) * 4;
            if dst_start + row_bytes <= img.rgba.len() {
                img.rgba.copy_within(first_start..first_start + row_bytes, dst_start);
            }
        }
    } else {
        let sa = u32::from(color.a);
        let inv = 255 - sa;
        let sr = u32::from(color.r) * sa;
        let sg = u32::from(color.g) * sa;
        let sb = u32::from(color.b) * sa;
        for py in y0..y0 + h {
            let row_start = (py * img_w + x0) * 4;
            if let Some(row) = img.rgba.get_mut(row_start..row_start + row_bytes) {
                for dst in row.as_chunks_mut::<4>().0 {
                    dst[0] = ((sr + u32::from(dst[0]) * inv + 127) / 255) as u8;
                    dst[1] = ((sg + u32::from(dst[1]) * inv + 127) / 255) as u8;
                    dst[2] = ((sb + u32::from(dst[2]) * inv + 127) / 255) as u8;
                    dst[3] = (sa + (u32::from(dst[3]) * inv + 127) / 255).min(255) as u8;
                }
            }
        }
    }
}

fn stroke_clipped_rect(img: &mut RasterImage, rect: LayoutRectPx, color: Rgba8, thickness: u32, clip: LayoutRectPx) {
    if rect.width == 0 || rect.height == 0 || thickness == 0 || color.a == 0 {
        return;
    }
    let t = thickness.min(rect.width).min(rect.height).max(1);
    let ti = i32::try_from(t).unwrap_or(1);
    fill_clipped_rect(img, LayoutRectPx::new(rect.x, rect.y, rect.width, t), color, clip);
    fill_clipped_rect(img, LayoutRectPx::new(rect.x, rect.bottom() - ti, rect.width, t), color, clip);
    fill_clipped_rect(img, LayoutRectPx::new(rect.x, rect.y, t, rect.height), color, clip);
    fill_clipped_rect(img, LayoutRectPx::new(rect.right() - ti, rect.y, t, rect.height), color, clip);
}

fn draw_clipped_line(img: &mut RasterImage, p0: [i32; 2], p1: [i32; 2], color: Rgba8, thickness: u32, clip: LayoutRectPx) {
    if color.a == 0 {
        return;
    }
    let r = i32::try_from(thickness / 2).unwrap_or(0);
    let stamp = thickness.max(1);
    if p0[0] == p1[0] || p0[1] == p1[1] {
        let min_x = p0[0].min(p1[0]);
        let max_x = p0[0].max(p1[0]);
        let min_y = p0[1].min(p1[1]);
        let max_y = p0[1].max(p1[1]);
        let w = u32::try_from(max_x.saturating_sub(min_x)).unwrap_or(0).saturating_add(stamp);
        let h = u32::try_from(max_y.saturating_sub(min_y)).unwrap_or(0).saturating_add(stamp);
        fill_clipped_rect(img, LayoutRectPx::new(min_x - r, min_y - r, w, h), color, clip);
        return;
    }
    let dx = (p1[0] - p0[0]).abs();
    let dy = (p1[1] - p0[1]).abs();
    let steps = dx.max(dy).max(1);
    for i in 0..=steps {
        let fx = p0[0] + (p1[0] - p0[0]) * i / steps;
        let fy = p0[1] + (p1[1] - p0[1]) * i / steps;
        fill_clipped_rect(img, LayoutRectPx::new(fx - r, fy - r, stamp, stamp), color, clip);
    }
}

fn draw_clipped_circle(img: &mut RasterImage, center: [i32; 2], radius: u32, fill: Rgba8, stroke: Option<Rgba8>, clip: LayoutRectPx) {
    let r = i32::try_from(radius).unwrap_or(0);
    if r <= 0 {
        return;
    }
    let r_sq = r * r;
    let inner_sq = (r - 1).max(0) * (r - 1).max(0);
    for dy in -r..=r {
        for dx in -r..=r {
            let px = center[0] + dx;
            let py = center[1] + dy;
            if !clip.contains_px(px, py) {
                continue;
            }
            let d2 = dx * dx + dy * dy;
            if d2 <= r_sq {
                let c = if let Some(sc) = stroke
                    && d2 >= inner_sq
                {
                    sc
                } else {
                    fill
                };
                if c.a > 0 {
                    img.blend_pixel(px as u32, py as u32, c);
                }
            }
        }
    }
}

fn blit_clipped_image(dst_img: &mut RasterImage, dst_rect: LayoutRectPx, src_img: &RasterImage, clip: LayoutRectPx) {
    let vis = dst_rect.intersect(clip);
    if vis.width == 0 || vis.height == 0 || src_img.width_px == 0 || src_img.height_px == 0 {
        return;
    }
    let dw = dst_rect.width.max(1) as usize;
    let dh = dst_rect.height.max(1) as usize;
    let sw = src_img.width_px as usize;
    let sh = src_img.height_px as usize;
    let out_w = dst_img.width_px as usize;

    for py in vis.y..vis.bottom() {
        let local_y = (py - dst_rect.y) as usize;
        let sy = ((local_y * sh) / dh).min(sh - 1);
        let dst_row = (py as usize) * out_w;
        let src_row = sy * sw;
        for px in vis.x..vis.right() {
            let local_x = (px - dst_rect.x) as usize;
            let sx = ((local_x * sw) / dw).min(sw - 1);
            let sidx = (src_row + sx) * 4;
            let didx = (dst_row + (px as usize)) * 4;
            let Some(src) = src_img.rgba.get(sidx..sidx + 4) else {
                continue;
            };
            let Some(dst) = dst_img.rgba.get_mut(didx..didx + 4) else {
                continue;
            };
            if src[3] == 255 {
                dst.copy_from_slice(src);
            } else if src[3] > 0 {
                let a = u32::from(src[3]);
                let inv = 255 - a;
                dst[0] = ((u32::from(src[0]) * a + u32::from(dst[0]) * inv + 127) / 255) as u8;
                dst[1] = ((u32::from(src[1]) * a + u32::from(dst[1]) * inv + 127) / 255) as u8;
                dst[2] = ((u32::from(src[2]) * a + u32::from(dst[2]) * inv + 127) / 255) as u8;
                dst[3] = 255;
            }
        }
    }
}
