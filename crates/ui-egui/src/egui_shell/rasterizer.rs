//! Pure-Rust (`#![forbid(unsafe_code)]`) CPU software rasterizer over `egui::Context::tessellate(...)`
//! (`ClippedPrimitive` triangles, scissor rects, premultiplied alpha blending, `TexturesDelta`).
//!
//! Powers `ui.screenshot` (`full_window`, `canvas`, `dialog` crops) on GPU-less CI runners and
//! `egui` backend snapshots without platform GPU dependencies.

use std::collections::BTreeMap;

use curvebook_render::{RasterImage, RenderError, RenderTargetSpec, encode_png};

use crate::inspect::ScreenshotRegionTarget;

/// Standard logical display DPI (`1.0x` scale factor = `96` DPI).
pub const BASE_SCREENSHOT_DPI: f32 = 96.0;

#[derive(Debug, Clone)]
struct TextureSurface {
    width: usize,
    height: usize,
    /// Premultiplied sRGBA pixels `[r, g, b, a]` matching `egui::Color32`.
    pixels: Vec<[u8; 4]>,
    options: egui::TextureOptions,
}

impl TextureSurface {
    fn from_image_data(image: &egui::ImageData, options: egui::TextureOptions) -> Self {
        match image {
            egui::ImageData::Color(color_img) => {
                let [width, height] = color_img.size;
                let pixels = color_img.pixels.iter().map(|c| c.to_array()).collect();
                Self { width, height, pixels, options }
            }
        }
    }

    fn blit_subrect(&mut self, pos: [usize; 2], patch: &Self) {
        self.options = patch.options;
        if patch.width == 0 || patch.height == 0 || self.width == 0 || self.height == 0 {
            return;
        }
        let max_y = patch.height.min(self.height.saturating_sub(pos[1]));
        let max_x = patch.width.min(self.width.saturating_sub(pos[0]));
        for row in 0..max_y {
            let dst_y = pos[1] + row;
            let src_start = row * patch.width;
            let dst_start = dst_y * self.width + pos[0];
            if let (Some(src_slice), Some(dst_slice)) =
                (patch.pixels.get(src_start..src_start + max_x), self.pixels.get_mut(dst_start..dst_start + max_x))
            {
                dst_slice.copy_from_slice(src_slice);
            }
        }
    }

    fn texel(&self, ix: usize, iy: usize) -> [f32; 4] {
        let idx = iy * self.width + ix;
        let [r, g, b, a] = self.pixels.get(idx).copied().unwrap_or([255, 255, 255, 255]);
        [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0]
    }

    fn wrap_coord(coord: f32, mode: egui::TextureWrapMode) -> f32 {
        match mode {
            egui::TextureWrapMode::ClampToEdge => coord.clamp(0.0, 1.0),
            egui::TextureWrapMode::Repeat => coord.rem_euclid(1.0),
            egui::TextureWrapMode::MirroredRepeat => {
                let m = coord.rem_euclid(2.0);
                if m > 1.0 { 2.0 - m } else { m }
            }
        }
    }

    fn wrap_index(coord: isize, len: usize, mode: egui::TextureWrapMode) -> usize {
        if len <= 1 {
            return 0;
        }
        let len_i = isize::try_from(len).unwrap_or(1);
        match mode {
            egui::TextureWrapMode::ClampToEdge => usize::try_from(coord.clamp(0, len_i - 1)).unwrap_or(0),
            egui::TextureWrapMode::Repeat => usize::try_from(coord.rem_euclid(len_i)).unwrap_or(0),
            egui::TextureWrapMode::MirroredRepeat => {
                let period = len_i * 2;
                let m = coord.rem_euclid(period);
                let reflected = if m >= len_i { period - 1 - m } else { m };
                usize::try_from(reflected.clamp(0, len_i - 1)).unwrap_or(0)
            }
        }
    }

    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 || self.pixels.is_empty() {
            return [1.0, 1.0, 1.0, 1.0];
        }
        let wrap = self.options.wrap_mode;
        let uw = Self::wrap_coord(if u.is_finite() { u } else { 0.0 }, wrap);
        let vw = Self::wrap_coord(if v.is_finite() { v } else { 0.0 }, wrap);

        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let (fx, fy) = (uw * self.width as f32, vw * self.height as f32);

        if matches!(self.options.magnification, egui::TextureFilter::Nearest) {
            #[allow(clippy::cast_possible_truncation)]
            let ix = Self::wrap_index(fx.floor() as isize, self.width, wrap);
            #[allow(clippy::cast_possible_truncation)]
            let iy = Self::wrap_index(fy.floor() as isize, self.height, wrap);
            return self.texel(ix, iy);
        }

        let x_shifted = fx - 0.5;
        let y_shifted = fy - 0.5;
        #[allow(clippy::cast_possible_truncation)]
        let x0_i = x_shifted.floor() as isize;
        #[allow(clippy::cast_possible_truncation)]
        let y0_i = y_shifted.floor() as isize;
        #[allow(clippy::cast_precision_loss)]
        let tx = (x_shifted - x0_i as f32).clamp(0.0, 1.0);
        #[allow(clippy::cast_precision_loss)]
        let ty = (y_shifted - y0_i as f32).clamp(0.0, 1.0);

        let x0 = Self::wrap_index(x0_i, self.width, wrap);
        let x1 = Self::wrap_index(x0_i + 1, self.width, wrap);
        let y0 = Self::wrap_index(y0_i, self.height, wrap);
        let y1 = Self::wrap_index(y0_i + 1, self.height, wrap);

        let c00 = self.texel(x0, y0);
        let c10 = self.texel(x1, y0);
        let c01 = self.texel(x0, y1);
        let c11 = self.texel(x1, y1);

        let bilerp = |k00: f32, k10: f32, k01: f32, k11: f32| {
            let top = k00 + (k10 - k00) * tx;
            let bot = k01 + (k11 - k01) * tx;
            top + (bot - top) * ty
        };
        [
            bilerp(c00[0], c10[0], c01[0], c11[0]),
            bilerp(c00[1], c10[1], c01[1], c11[1]),
            bilerp(c00[2], c10[2], c01[2], c11[2]),
            bilerp(c00[3], c10[3], c01[3], c11[3]),
        ]
    }
}

/// CPU software rasterizer for `egui` tessellated meshes and `TexturesDelta` updates.
#[derive(Debug, Clone, Default)]
pub struct EguiSoftwareRasterizer {
    textures: BTreeMap<egui::TextureId, TextureSurface>,
}

/// Convert a screenshot DPI integer (`96` = `1.0x`, `144` = `1.5x`, `192` = `2.0x`) into a clamped
/// `egui` `pixels_per_point` scale factor.
#[must_use]
pub fn pixels_per_point_for_dpi(dpi: u16) -> f32 {
    if dpi == 0 {
        return 1.0;
    }
    (f32::from(dpi.max(48)) / BASE_SCREENSHOT_DPI).clamp(0.5, 8.0)
}

#[derive(Debug, Clone, Copy)]
struct ScissorBoxPx {
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
}

impl ScissorBoxPx {
    fn from_clip_rect(clip_rect: egui::Rect, ppp: f32, fb_w: i32, fb_h: i32) -> Option<Self> {
        if !clip_rect.is_positive() {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        let min_x = (clip_rect.min.x * ppp).floor().max(0.0) as i32;
        #[allow(clippy::cast_possible_truncation)]
        let min_y = (clip_rect.min.y * ppp).floor().max(0.0) as i32;
        #[allow(clippy::cast_possible_truncation)]
        let max_x = (clip_rect.max.x * ppp).ceil().min(fb_w as f32) as i32;
        #[allow(clippy::cast_possible_truncation)]
        let max_y = (clip_rect.max.y * ppp).ceil().min(fb_h as f32) as i32;
        if min_x >= max_x || min_y >= max_y {
            return None;
        }
        Some(Self { min_x, min_y, max_x, max_y })
    }
}

#[derive(Debug, Clone, Copy)]
struct TriangleVertex {
    pos_px: [f32; 2],
    uv: [f32; 2],
    color_pm: [f32; 4],
}

impl TriangleVertex {
    fn from_epaint(v: &egui::epaint::Vertex, ppp: f32) -> Self {
        let [r, g, b, a] = v.color.to_array();
        Self {
            pos_px: [v.pos.x * ppp, v.pos.y * ppp],
            uv: [v.uv.x, v.uv.y],
            color_pm: [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0],
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct EdgeRasterContext<'a> {
    fb_w: usize,
    scissor: ScissorBoxPx,
    texture: Option<&'a TextureSurface>,
}

fn color32_to_pm_f32(color: egui::Color32) -> [f32; 4] {
    let [r, g, b, a] = color.to_array();
    [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0]
}

fn edge_fn(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn rasterize_triangle(buf: &mut [[f32; 4]], ctx: EdgeRasterContext<'_>, tri: [TriangleVertex; 3]) {
    let [tv0, tv1, tv2] = tri;
    let area = edge_fn(tv0.pos_px, tv1.pos_px, tv2.pos_px);
    if area.abs() < 1e-6 || !area.is_finite() {
        return;
    }
    let inv_area = 1.0 / area;

    let min_fx = tv0.pos_px[0].min(tv1.pos_px[0]).min(tv2.pos_px[0]).floor();
    let max_fx = tv0.pos_px[0].max(tv1.pos_px[0]).max(tv2.pos_px[0]).ceil();
    let min_fy = tv0.pos_px[1].min(tv1.pos_px[1]).min(tv2.pos_px[1]).floor();
    let max_fy = tv0.pos_px[1].max(tv1.pos_px[1]).max(tv2.pos_px[1]).ceil();

    #[allow(clippy::cast_possible_truncation)]
    let x0 = (min_fx as i32).max(ctx.scissor.min_x);
    #[allow(clippy::cast_possible_truncation)]
    let x1 = (max_fx as i32).min(ctx.scissor.max_x);
    #[allow(clippy::cast_possible_truncation)]
    let y0 = (min_fy as i32).max(ctx.scissor.min_y);
    #[allow(clippy::cast_possible_truncation)]
    let y1 = (max_fy as i32).min(ctx.scissor.max_y);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    for py in y0..y1 {
        #[allow(clippy::cast_precision_loss)]
        let sample_y = py as f32 + 0.5;
        let Ok(uy) = usize::try_from(py) else {
            continue;
        };
        let row_offset = uy * ctx.fb_w;
        for px in x0..x1 {
            #[allow(clippy::cast_precision_loss)]
            let sample_x = px as f32 + 0.5;
            let p = [sample_x, sample_y];

            // Dividing signed edge values by signed `area` normalizes both CW and CCW winding.
            let w0 = edge_fn(tv1.pos_px, tv2.pos_px, p) * inv_area;
            let w1 = edge_fn(tv2.pos_px, tv0.pos_px, p) * inv_area;
            let w2 = edge_fn(tv0.pos_px, tv1.pos_px, p) * inv_area;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let sum = (w0.max(0.0) + w1.max(0.0) + w2.max(0.0)).max(1e-6);
            let b0 = w0.max(0.0) / sum;
            let b1 = w1.max(0.0) / sum;
            let b2 = w2.max(0.0) / sum;

            let u = b0 * tv0.uv[0] + b1 * tv1.uv[0] + b2 * tv2.uv[0];
            let v = b0 * tv0.uv[1] + b1 * tv1.uv[1] + b2 * tv2.uv[1];
            let tex_pm = ctx.texture.map_or([1.0, 1.0, 1.0, 1.0], |tex| tex.sample(u, v));

            let vert_r = b0 * tv0.color_pm[0] + b1 * tv1.color_pm[0] + b2 * tv2.color_pm[0];
            let vert_g = b0 * tv0.color_pm[1] + b1 * tv1.color_pm[1] + b2 * tv2.color_pm[1];
            let vert_b = b0 * tv0.color_pm[2] + b1 * tv1.color_pm[2] + b2 * tv2.color_pm[2];
            let vert_a = b0 * tv0.color_pm[3] + b1 * tv1.color_pm[3] + b2 * tv2.color_pm[3];
            let src = [
                (vert_r * tex_pm[0]).clamp(0.0, 1.0),
                (vert_g * tex_pm[1]).clamp(0.0, 1.0),
                (vert_b * tex_pm[2]).clamp(0.0, 1.0),
                (vert_a * tex_pm[3]).clamp(0.0, 1.0),
            ];
            let inv_src_a = (1.0 - src[3]).clamp(0.0, 1.0);
            let Ok(ux) = usize::try_from(px) else {
                continue;
            };
            let idx = row_offset + ux;
            if let Some(dst) = buf.get_mut(idx) {
                dst[0] = (src[0] + dst[0] * inv_src_a).clamp(0.0, 1.0);
                dst[1] = (src[1] + dst[1] * inv_src_a).clamp(0.0, 1.0);
                dst[2] = (src[2] + dst[2] * inv_src_a).clamp(0.0, 1.0);
                dst[3] = (src[3] + dst[3] * inv_src_a).clamp(0.0, 1.0);
            }
        }
    }
}

fn premultiplied_buf_to_straight_rgba(buf: &[[f32; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(buf.len() * 4);
    for &[pr, pg, pb, pa] in buf {
        let a_clamped = pa.clamp(0.0, 1.0);
        if a_clamped <= 1e-6 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let r = ((pr / a_clamped).clamp(0.0, 1.0) * 255.0).round() as u8;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let g = ((pg / a_clamped).clamp(0.0, 1.0) * 255.0).round() as u8;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let b = ((pb / a_clamped).clamp(0.0, 1.0) * 255.0).round() as u8;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let a = (a_clamped * 255.0).round() as u8;
            out.extend_from_slice(&[r, g, b, a]);
        }
    }
    out
}

fn logical_rect_to_pixel_crop(rect_pt: egui::Rect, ppp: f32, img_w: u32, img_h: u32) -> Result<(u32, u32, u32, u32), RenderError> {
    if !rect_pt.is_positive() {
        return Err(RenderError::InvalidTarget { message: String::from("screenshot region rect has zero area") });
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let x0 = ((rect_pt.min.x * ppp).floor().max(0.0) as u32).min(img_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let y0 = ((rect_pt.min.y * ppp).floor().max(0.0) as u32).min(img_h);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let x1 = ((rect_pt.max.x * ppp).ceil().max(0.0) as u32).min(img_w);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let y1 = ((rect_pt.max.y * ppp).ceil().max(0.0) as u32).min(img_h);
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        return Err(RenderError::InvalidTarget { message: String::from("cropped screenshot region lies outside rendered framebuffer") });
    }
    Ok((x0, y0, w, h))
}

/// Crop a straight-RGBA [`RasterImage`] to an explicit sub-rectangle in physical pixels.
///
/// # Errors
/// Returns [`RenderError::InvalidTarget`] when the crop rectangle is empty or out of bounds.
pub fn crop_raster_rgba(image: &RasterImage, x: u32, y: u32, w: u32, h: u32) -> Result<RasterImage, RenderError> {
    if w == 0 || h == 0 || x.saturating_add(w) > image.width_px || y.saturating_add(h) > image.height_px {
        return Err(RenderError::InvalidTarget {
            message: format!("crop rectangle ({x}, {y}, {w}x{h}) exceeds image {}x{}", image.width_px, image.height_px),
        });
    }
    let src_w =
        usize::try_from(image.width_px).map_err(|_| RenderError::InvalidTarget { message: String::from("source image width exceeds usize") })?;
    let src_h =
        usize::try_from(image.height_px).map_err(|_| RenderError::InvalidTarget { message: String::from("source image height exceeds usize") })?;
    let expected_src_bytes = src_w
        .checked_mul(src_h)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| RenderError::InvalidTarget { message: String::from("source image byte size overflows usize") })?;
    if image.rgba.len() < expected_src_bytes {
        return Err(RenderError::InvalidTarget {
            message: format!(
                "source RGBA buffer truncated ({} bytes < expected {expected_src_bytes} bytes for {}x{})",
                image.rgba.len(),
                image.width_px,
                image.height_px
            ),
        });
    }
    let src_stride = src_w.checked_mul(4).ok_or_else(|| RenderError::InvalidTarget { message: String::from("source stride overflows usize") })?;
    let dst_w = usize::try_from(w).map_err(|_| RenderError::InvalidTarget { message: String::from("crop width exceeds usize") })?;
    let dst_h = usize::try_from(h).map_err(|_| RenderError::InvalidTarget { message: String::from("crop height exceeds usize") })?;
    let row_bytes = dst_w.checked_mul(4).ok_or_else(|| RenderError::InvalidTarget { message: String::from("crop row bytes overflow usize") })?;
    let dst_bytes = dst_h
        .checked_mul(row_bytes)
        .ok_or_else(|| RenderError::InvalidTarget { message: String::from("crop output buffer size overflows usize") })?;
    let mut rgba = Vec::with_capacity(dst_bytes);
    let x_off = usize::try_from(x)
        .ok()
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| RenderError::InvalidTarget { message: String::from("crop x offset overflows usize") })?;
    let y_base = usize::try_from(y).map_err(|_| RenderError::InvalidTarget { message: String::from("crop y offset exceeds usize") })?;
    for row in 0..dst_h {
        let start = y_base
            .checked_add(row)
            .and_then(|sy| sy.checked_mul(src_stride))
            .and_then(|off| off.checked_add(x_off))
            .ok_or_else(|| RenderError::InvalidTarget { message: String::from("crop row offset overflows usize") })?;
        let end = start.checked_add(row_bytes).ok_or_else(|| RenderError::InvalidTarget { message: String::from("crop row end overflows usize") })?;
        let row_slice = image
            .rgba
            .get(start..end)
            .ok_or_else(|| RenderError::InvalidTarget { message: String::from("crop rectangle row exceeds source buffer") })?;
        rgba.extend_from_slice(row_slice);
    }
    Ok(RasterImage { width_px: w, height_px: h, dpi: image.dpi, rgba })
}

impl EguiSoftwareRasterizer {
    /// Construct an empty software rasterizer with no uploaded textures.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of active uploaded `egui` textures tracked by this rasterizer.
    #[must_use]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Upload and patch all entries from `textures_delta.set`. Call prior to rasterizing frame primitives.
    pub fn apply_textures_set(&mut self, delta: &egui::TexturesDelta) {
        for (id, deltas) in &delta.set {
            for image_delta in deltas {
                let patch = TextureSurface::from_image_data(&image_delta.image, image_delta.options);
                if let Some(pos) = image_delta.pos
                    && let Some(existing) = self.textures.get_mut(id)
                {
                    existing.blit_subrect(pos, &patch);
                } else {
                    self.textures.insert(*id, patch);
                }
            }
        }
    }

    /// Release all texture entries scheduled in `textures_delta.free` after rasterizing a frame.
    pub fn free_textures(&mut self, delta: &egui::TexturesDelta) {
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    /// Rasterize `clipped_primitives` into a straight-RGBA [`RasterImage`] of exact dimensions `target`.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidTarget`] when `target` dimensions or `pixels_per_point` are invalid.
    pub fn rasterize_tessellated(
        &self,
        clipped_primitives: &[egui::ClippedPrimitive],
        target: RenderTargetSpec,
        pixels_per_point: f32,
        clear_color: egui::Color32,
    ) -> Result<RasterImage, RenderError> {
        if target.width_px == 0 || target.height_px == 0 {
            return Err(RenderError::InvalidTarget { message: String::from("raster target width and height must be positive") });
        }
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return Err(RenderError::InvalidTarget { message: String::from("pixels_per_point must be finite and positive") });
        }
        let fb_w = usize::try_from(target.width_px).map_err(|_| RenderError::InvalidTarget { message: String::from("width overflow") })?;
        let fb_h = usize::try_from(target.height_px).map_err(|_| RenderError::InvalidTarget { message: String::from("height overflow") })?;
        let fb_w_i32 = i32::try_from(target.width_px).map_err(|_| RenderError::InvalidTarget { message: String::from("width overflow") })?;
        let fb_h_i32 = i32::try_from(target.height_px).map_err(|_| RenderError::InvalidTarget { message: String::from("height overflow") })?;

        let total_pixels = fb_w.checked_mul(fb_h).ok_or_else(|| RenderError::InvalidTarget { message: String::from("framebuffer size overflow") })?;
        let mut buf = vec![color32_to_pm_f32(clear_color); total_pixels];

        for clipped in clipped_primitives {
            let Some(scissor) = ScissorBoxPx::from_clip_rect(clipped.clip_rect, pixels_per_point, fb_w_i32, fb_h_i32) else {
                continue;
            };
            let egui::epaint::Primitive::Mesh(mesh) = &clipped.primitive else {
                continue;
            };
            let texture = self.textures.get(&mesh.texture_id);
            let ctx = EdgeRasterContext { fb_w, scissor, texture };
            for &[i0, i1, i2] in mesh.indices.as_chunks::<3>().0 {
                let Some(v0) = mesh.vertices.get(usize::try_from(i0).unwrap_or(usize::MAX)) else {
                    continue;
                };
                let Some(v1) = mesh.vertices.get(usize::try_from(i1).unwrap_or(usize::MAX)) else {
                    continue;
                };
                let Some(v2) = mesh.vertices.get(usize::try_from(i2).unwrap_or(usize::MAX)) else {
                    continue;
                };
                let tri = [
                    TriangleVertex::from_epaint(v0, pixels_per_point),
                    TriangleVertex::from_epaint(v1, pixels_per_point),
                    TriangleVertex::from_epaint(v2, pixels_per_point),
                ];
                rasterize_triangle(&mut buf, ctx, tri);
            }
        }

        let rgba = premultiplied_buf_to_straight_rgba(&buf);
        Ok(RasterImage { width_px: target.width_px, height_px: target.height_px, dpi: target.dpi, rgba })
    }

    /// Upload and patch all entries from `textures_delta.set` and drain them so `TexturesDelta` can drop cleanly.
    pub fn consume_textures_set(&mut self, delta: &mut egui::TexturesDelta) {
        self.apply_textures_set(delta);
        delta.set.clear();
    }

    /// Release all texture entries scheduled in `textures_delta.free` and clear `free`.
    pub fn consume_textures_free(&mut self, delta: &mut egui::TexturesDelta) {
        self.free_textures(delta);
        delta.free.clear();
    }

    /// Apply `full_output.textures_delta` updates, tessellate `full_output.shapes`, rasterize at
    /// `target`, and crop to `region` returning a straight-RGBA [`RasterImage`].
    ///
    /// # Errors
    /// Returns [`RenderError`] if `target` is invalid or `region == Dialog` without an active dialog
    /// rect.
    pub fn render_egui_screenshot_rgba(
        &mut self,
        ctx: &egui::Context,
        full_output: &mut egui::FullOutput,
        target: RenderTargetSpec,
        region: ScreenshotRegionTarget,
        region_rects: EguiScreenshotRegions,
        clear_color: egui::Color32,
    ) -> Result<RasterImage, RenderError> {
        self.consume_textures_set(&mut full_output.textures_delta);
        let ppp = if full_output.pixels_per_point.is_finite() && full_output.pixels_per_point > 0.0 {
            full_output.pixels_per_point
        } else {
            pixels_per_point_for_dpi(target.dpi)
        };
        let clipped = ctx.tessellate(std::mem::take(&mut full_output.shapes), ppp);
        let full_img = self.rasterize_tessellated(&clipped, target, ppp, clear_color);
        self.consume_textures_free(&mut full_output.textures_delta);
        let full_img = full_img?;

        match region {
            ScreenshotRegionTarget::FullWindow => Ok(full_img),
            ScreenshotRegionTarget::Canvas => {
                let rect = region_rects
                    .canvas_rect_pt
                    .ok_or_else(|| RenderError::InvalidTarget { message: String::from("canvas screenshot region unavailable on current frame") })?;
                let (cx, cy, cw, ch) = logical_rect_to_pixel_crop(rect, ppp, full_img.width_px, full_img.height_px)?;
                crop_raster_rgba(&full_img, cx, cy, cw, ch)
            }
            ScreenshotRegionTarget::Dialog => {
                let rect = region_rects.dialog_rect_pt.ok_or_else(|| RenderError::InvalidTarget {
                    message: String::from("cannot capture `dialog` screenshot when no modal dialog is open"),
                })?;
                let (dx, dy, dw, dh) = logical_rect_to_pixel_crop(rect, ppp, full_img.width_px, full_img.height_px)?;
                crop_raster_rgba(&full_img, dx, dy, dw, dh)
            }
        }
    }

    /// Apply `full_output.textures_delta` updates, tessellate `full_output.shapes`, rasterize at
    /// `target`, crop to `region`, and encode an RGBA PNG (`(png_bytes, out_width_px, out_height_px)`).
    ///
    /// # Errors
    /// Returns [`RenderError`] if `target` is invalid, `region == Dialog` without an active dialog
    /// rect, or PNG encoding fails.
    pub fn render_egui_screenshot_png(
        &mut self,
        ctx: &egui::Context,
        full_output: &mut egui::FullOutput,
        target: RenderTargetSpec,
        region: ScreenshotRegionTarget,
        region_rects: EguiScreenshotRegions,
        clear_color: egui::Color32,
    ) -> Result<(Vec<u8>, u32, u32), RenderError> {
        let cropped = self.render_egui_screenshot_rgba(ctx, full_output, target, region, region_rects, clear_color)?;
        let out_w = cropped.width_px;
        let out_h = cropped.height_px;
        let png = encode_png(&cropped)?;
        Ok((png, out_w, out_h))
    }
}

/// Optional logical-point crop rectangles discovered during the settled `egui` layout pass.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EguiScreenshotRegions {
    pub canvas_rect_pt: Option<egui::Rect>,
    pub dialog_rect_pt: Option<egui::Rect>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizer_renders_solid_and_blended_rects_with_scissor_and_dpi() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 120.0))),
            viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(1.5), ..Default::default() })]
                .into_iter()
                .collect(),
            ..Default::default()
        };

        let mut modal_rect = None;
        let mut out = ctx.run_ui(raw, |ui| {
            let painter = ui.painter();
            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(60.0, 40.0)), 0.0, egui::Color32::from_rgb(200, 40, 40));
            let mrect = egui::Rect::from_min_size(egui::pos2(30.0, 20.0), egui::vec2(80.0, 50.0));
            painter.rect_filled(mrect, 0.0, egui::Color32::from_rgba_unmultiplied(20, 40, 220, 128));
            modal_rect = Some(mrect);
            ui.label("CurveBook Rasterizer");
        });
        let saved_shapes = out.shapes.clone();

        let mut rast = EguiSoftwareRasterizer::new();
        let target = RenderTargetSpec { width_px: 300, height_px: 180, dpi: 144 };
        let (png, w, h) = rast
            .render_egui_screenshot_png(
                &ctx,
                &mut out,
                target,
                ScreenshotRegionTarget::FullWindow,
                EguiScreenshotRegions { canvas_rect_pt: None, dialog_rect_pt: modal_rect },
                egui::Color32::WHITE,
            )
            .expect("full_window screenshot should encode PNG");

        assert_eq!((w, h), (300, 180));
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        out.shapes = saved_shapes;
        let (dlg_png, dw, dh) = rast
            .render_egui_screenshot_png(
                &ctx,
                &mut out,
                target,
                ScreenshotRegionTarget::Dialog,
                EguiScreenshotRegions { canvas_rect_pt: None, dialog_rect_pt: modal_rect },
                egui::Color32::WHITE,
            )
            .expect("dialog crop should succeed");
        assert_eq!((dw, dh), (120, 75));
        assert!(dlg_png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn rasterizer_handles_color_texture_subrect_delta_and_dialog_error() {
        let mut rast = EguiSoftwareRasterizer::new();
        let tex_id = egui::TextureId::User(7);
        let base_img = egui::ColorImage::filled([4, 4], egui::Color32::from_rgb(10, 20, 30));
        let mut set_delta = egui::TexturesDelta::default();
        set_delta.set.insert(tex_id, [egui::epaint::ImageDelta::full(base_img, egui::TextureOptions::NEAREST)].into_iter().collect());
        rast.consume_textures_set(&mut set_delta);
        assert_eq!(rast.texture_count(), 1);

        let patch_img = egui::ColorImage::filled([2, 2], egui::Color32::from_rgb(200, 210, 220));
        let mut patch_delta = egui::TexturesDelta::default();
        patch_delta.set.insert(tex_id, [egui::epaint::ImageDelta::partial([1, 1], patch_img, egui::TextureOptions::NEAREST)].into_iter().collect());
        rast.consume_textures_set(&mut patch_delta);

        let mut mesh = egui::epaint::Mesh::with_texture(tex_id);
        mesh.add_rect_with_uv(
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(4.0, 4.0)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        let clipped = [egui::ClippedPrimitive { clip_rect: egui::Rect::EVERYTHING, primitive: egui::epaint::Primitive::Mesh(mesh) }];
        let img = rast
            .rasterize_tessellated(&clipped, RenderTargetSpec { width_px: 4, height_px: 4, dpi: 96 }, 1.0, egui::Color32::BLACK)
            .expect("mesh rasterization");
        // Top-left pixel `(0,0)` stays `[10, 20, 30, 255]`, patched pixel `(1,1)` is `[200, 210, 220, 255]`.
        assert_eq!(&img.rgba[0..4], &[10, 20, 30, 255]);
        let idx11 = (4 + 1) * 4;
        assert_eq!(&img.rgba[idx11..idx11 + 4], &[200, 210, 220, 255]);

        let mut free_delta = egui::TexturesDelta::default();
        free_delta.free.insert(tex_id);
        rast.consume_textures_free(&mut free_delta);
        assert_eq!(rast.texture_count(), 0);
    }
}
