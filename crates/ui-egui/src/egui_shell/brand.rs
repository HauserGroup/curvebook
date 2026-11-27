//! CurveBook in-app brand mark: the seven-pane logo (`assets/app-icon/`, see its README).
//!
//! Trims the outer transparent letterbox from the 512 px hicolor asset to its natural ~1.43:1
//! bounds, CPU-downsamples in premultiplied linear RGBA to exact physical pixel dimensions per DPI
//! and surface size (avoiding `egui-wgpu`'s missing mipmap filtering), and caches one texture
//! handle per `(phys_w, phys_h)` in the `egui::Context`.

use std::sync::OnceLock;

use egui::{Color32, ColorImage, Context, Id, Rect, TextureHandle, TextureOptions, Ui, Vec2, pos2};

/// 512 px hicolor PNG: strictly downsamples at all supported UI sizes and DPI factors (1x–3x+).
const ICON_PNG: &[u8] = include_bytes!("../../../../assets/app-icon/hicolor/512x512/apps/org.curvebook.CurveBook.png");

/// Minimum alpha value treated as part of the visible artwork when trimming transparent padding.
const ALPHA_TRIM_THRESHOLD: u8 = 4;

/// Maximum cached per-size textures retained in `egui::Context` temporary storage.
const MAX_CACHED_BRAND_TEXTURES: usize = 16;

/// Natural trimmed aspect ratio (`width / height`) of the seven-pane brand mark (`380 / 266 ≈ 1.43:1`).
pub const BRAND_ASPECT_RATIO: f32 = 380.0 / 266.0;

/// Logical size of the brand mark inside the unified top toolbar's `toolbar.home` button (`~1.43:1`).
pub const TOOLBAR_MARK_SIZE: Vec2 = Vec2::new(33.0, 23.0);

/// Logical size of the brand mark in the Welcome Hub header (`46 × 32 pt`, `~1.43:1`).
pub const WELCOME_MARK_SIZE: Vec2 = Vec2::new(46.0, 32.0);

/// Logical size of the brand mark in the About CurveBook panel (`63 × 44 pt`, `~1.43:1`).
pub const ABOUT_MARK_SIZE: Vec2 = Vec2::new(63.0, 44.0);

#[derive(Clone)]
struct MasterBrandRgba {
    width: usize,
    height: usize,
    rgba_unmult: Vec<u8>,
}

static MASTER_BRAND: OnceLock<MasterBrandRgba> = OnceLock::new();

fn decode_trimmed_master() -> MasterBrandRgba {
    let decode = || -> Option<MasterBrandRgba> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(ICON_PNG));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let buf_len = reader.output_buffer_size()?;
        let mut buf = vec![0u8; buf_len];
        let info = reader.next_frame(&mut buf).ok()?;
        let raw = buf.get(..info.buffer_size())?;
        let src_w = info.width as usize;
        let src_h = info.height as usize;
        let rgba: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => raw.to_vec(),
            png::ColorType::Rgb => raw.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
            _ => return None,
        };
        Some(trim_transparent_margins(src_w, src_h, &rgba, ALPHA_TRIM_THRESHOLD))
    };
    decode().unwrap_or_else(|| MasterBrandRgba { width: 1, height: 1, rgba_unmult: vec![0, 0, 0, 0] })
}

fn trim_transparent_margins(src_w: usize, src_h: usize, rgba: &[u8], min_alpha: u8) -> MasterBrandRgba {
    if src_w == 0 || src_h == 0 || rgba.len() < src_w * src_h * 4 {
        return MasterBrandRgba { width: 1, height: 1, rgba_unmult: vec![0, 0, 0, 0] };
    }
    let find_bbox = |thresh: u8| -> Option<(usize, usize, usize, usize)> {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (src_w, src_h, 0usize, 0usize);
        let mut any = false;
        for y in 0..src_h {
            for x in 0..src_w {
                let a = rgba[(y * src_w + x) * 4 + 3];
                if a >= thresh {
                    any = true;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        any.then_some((min_x, min_y, max_x, max_y))
    };
    let Some((x0, y0, x1, y1)) = find_bbox(min_alpha).or_else(|| find_bbox(1)) else {
        return MasterBrandRgba { width: 1, height: 1, rgba_unmult: vec![0, 0, 0, 0] };
    };
    let dst_w = x1 - x0 + 1;
    let dst_h = y1 - y0 + 1;
    let mut out = Vec::with_capacity(dst_w * dst_h * 4);
    for y in y0..=y1 {
        let start = (y * src_w + x0) * 4;
        let end = start + dst_w * 4;
        out.extend_from_slice(&rgba[start..end]);
    }
    MasterBrandRgba { width: dst_w, height: dst_h, rgba_unmult: out }
}

fn master_brand() -> &'static MasterBrandRgba {
    MASTER_BRAND.get_or_init(decode_trimmed_master)
}

#[inline]
fn srgb_u8_to_linear(v: u8) -> f32 {
    let c = f32::from(v) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

#[inline]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn linear_to_srgb_u8(v: f32) -> u8 {
    let clamped = v.clamp(0.0, 1.0);
    let srgb = if clamped <= 0.003_130_8 { clamped * 12.92 } else { 1.055 * clamped.powf(1.0 / 2.4) - 0.055 };
    (srgb * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Resample unmultiplied RGBA image bytes to `(dst_w, dst_h)` using separable premultiplied-linear
/// exact box-area downsampling so transparent edges never pick up dark alpha-fringe halos.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
pub fn resample_rgba_premultiplied(src_w: usize, src_h: usize, src_rgba_unmult: &[u8], dst_w: usize, dst_h: usize) -> ColorImage {
    let dst_w = dst_w.max(1);
    let dst_h = dst_h.max(1);
    if src_w == 0 || src_h == 0 || src_rgba_unmult.len() < src_w * src_h * 4 {
        return ColorImage::filled([dst_w, dst_h], Color32::TRANSPARENT);
    }
    if src_w == dst_w && src_h == dst_h {
        return ColorImage::from_rgba_unmultiplied([dst_w, dst_h], &src_rgba_unmult[..src_w * src_h * 4]);
    }

    // Horizontal pass: `src_w × src_h` -> `dst_w × src_h` in premultiplied linear RGBA.
    let mut horiz = vec![[0.0_f32; 4]; dst_w * src_h];
    if dst_w == src_w {
        for (dst_px, px) in horiz.iter_mut().zip(src_rgba_unmult.as_chunks::<4>().0) {
            let a = f32::from(px[3]) / 255.0;
            *dst_px = [srgb_u8_to_linear(px[0]) * a, srgb_u8_to_linear(px[1]) * a, srgb_u8_to_linear(px[2]) * a, a];
        }
    } else if dst_w < src_w {
        let scale_x = src_w as f32 / dst_w as f32;
        for ox in 0..dst_w {
            let x0 = ox as f32 * scale_x;
            let x1 = (ox + 1) as f32 * scale_x;
            let ix0 = (x0.floor() as usize).min(src_w - 1);
            let ix1 = ((x1.ceil() as usize).max(ix0 + 1)).min(src_w);
            let inv_span = 1.0 / (x1 - x0);
            for y in 0..src_h {
                let mut acc = [0.0_f32; 4];
                for ix in ix0..ix1 {
                    let w = (x1.min((ix + 1) as f32) - x0.max(ix as f32)).max(0.0);
                    if w <= 0.0 {
                        continue;
                    }
                    let base = (y * src_w + ix) * 4;
                    let a = f32::from(src_rgba_unmult[base + 3]) / 255.0;
                    let wa = w * a;
                    if wa > 0.0 {
                        acc[0] += srgb_u8_to_linear(src_rgba_unmult[base]) * wa;
                        acc[1] += srgb_u8_to_linear(src_rgba_unmult[base + 1]) * wa;
                        acc[2] += srgb_u8_to_linear(src_rgba_unmult[base + 2]) * wa;
                        acc[3] += wa;
                    }
                }
                horiz[y * dst_w + ox] = [acc[0] * inv_span, acc[1] * inv_span, acc[2] * inv_span, acc[3] * inv_span];
            }
        }
    } else {
        let scale_x = src_w as f32 / dst_w as f32;
        for ox in 0..dst_w {
            let sx = ((ox as f32 + 0.5) * scale_x - 0.5).clamp(0.0, (src_w - 1) as f32);
            let ix0 = sx.floor() as usize;
            let ix1 = (ix0 + 1).min(src_w - 1);
            let t = sx - ix0 as f32;
            for y in 0..src_h {
                let b0 = (y * src_w + ix0) * 4;
                let b1 = (y * src_w + ix1) * 4;
                let a0 = f32::from(src_rgba_unmult[b0 + 3]) / 255.0;
                let a1 = f32::from(src_rgba_unmult[b1 + 3]) / 255.0;
                let p0 = [
                    srgb_u8_to_linear(src_rgba_unmult[b0]) * a0,
                    srgb_u8_to_linear(src_rgba_unmult[b0 + 1]) * a0,
                    srgb_u8_to_linear(src_rgba_unmult[b0 + 2]) * a0,
                    a0,
                ];
                let p1 = [
                    srgb_u8_to_linear(src_rgba_unmult[b1]) * a1,
                    srgb_u8_to_linear(src_rgba_unmult[b1 + 1]) * a1,
                    srgb_u8_to_linear(src_rgba_unmult[b1 + 2]) * a1,
                    a1,
                ];
                horiz[y * dst_w + ox] =
                    [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t, p0[2] + (p1[2] - p0[2]) * t, p0[3] + (p1[3] - p0[3]) * t];
            }
        }
    }

    // Vertical pass: `dst_w × src_h` -> `dst_w × dst_h`, unpremultiplying cleanly to sRGBA bytes.
    let mut out_rgba = vec![0u8; dst_w * dst_h * 4];
    let write_pixel = |out: &mut [u8], idx: usize, p: [f32; 4]| {
        let a = p[3].clamp(0.0, 1.0);
        let a_u8 = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        let base = idx * 4;
        if a_u8 == 0 || a <= 1e-6 {
            out[base..base + 4].copy_from_slice(&[0, 0, 0, 0]);
        } else {
            out[base] = linear_to_srgb_u8(p[0] / a);
            out[base + 1] = linear_to_srgb_u8(p[1] / a);
            out[base + 2] = linear_to_srgb_u8(p[2] / a);
            out[base + 3] = a_u8;
        }
    };

    if dst_h == src_h {
        for (idx, &p) in horiz.iter().enumerate() {
            write_pixel(&mut out_rgba, idx, p);
        }
    } else if dst_h < src_h {
        let scale_y = src_h as f32 / dst_h as f32;
        for oy in 0..dst_h {
            let y0 = oy as f32 * scale_y;
            let y1 = (oy + 1) as f32 * scale_y;
            let iy0 = (y0.floor() as usize).min(src_h - 1);
            let iy1 = ((y1.ceil() as usize).max(iy0 + 1)).min(src_h);
            let inv_span = 1.0 / (y1 - y0);
            for ox in 0..dst_w {
                let mut acc = [0.0_f32; 4];
                for iy in iy0..iy1 {
                    let w = (y1.min((iy + 1) as f32) - y0.max(iy as f32)).max(0.0);
                    if w <= 0.0 {
                        continue;
                    }
                    let p = horiz[iy * dst_w + ox];
                    acc[0] += p[0] * w;
                    acc[1] += p[1] * w;
                    acc[2] += p[2] * w;
                    acc[3] += p[3] * w;
                }
                write_pixel(&mut out_rgba, oy * dst_w + ox, [acc[0] * inv_span, acc[1] * inv_span, acc[2] * inv_span, acc[3] * inv_span]);
            }
        }
    } else {
        let scale_y = src_h as f32 / dst_h as f32;
        for oy in 0..dst_h {
            let sy = ((oy as f32 + 0.5) * scale_y - 0.5).clamp(0.0, (src_h - 1) as f32);
            let iy0 = sy.floor() as usize;
            let iy1 = (iy0 + 1).min(src_h - 1);
            let t = sy - iy0 as f32;
            for ox in 0..dst_w {
                let p0 = horiz[iy0 * dst_w + ox];
                let p1 = horiz[iy1 * dst_w + ox];
                write_pixel(
                    &mut out_rgba,
                    oy * dst_w + ox,
                    [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t, p0[2] + (p1[2] - p0[2]) * t, p0[3] + (p1[3] - p0[3]) * t],
                );
            }
        }
    }

    ColorImage::from_rgba_unmultiplied([dst_w, dst_h], &out_rgba)
}

/// Decode the trimmed CurveBook brand mark into an unmultiplied RGBA [`ColorImage`].
///
/// Falls back to a 1×1 transparent pixel on decode failure (ruled out by unit tests).
#[must_use]
pub fn decode_brand_mark() -> ColorImage {
    let master = master_brand();
    ColorImage::from_rgba_unmultiplied([master.width, master.height], &master.rgba_unmult)
}

#[derive(Clone, Default)]
struct BrandTextureCache {
    entries: Vec<((usize, usize), TextureHandle)>,
}

/// Upload (once per physical pixel size `(phys_w, phys_h)`) and cache the premultiplied-alpha
/// downsampled brand mark texture in `egui::Context`.
fn texture_for_size(ctx: &Context, phys_w: usize, phys_h: usize) -> TextureHandle {
    let key = (phys_w.max(1), phys_h.max(1));
    let cache_id = Id::unique("curvebook-brand-mark-cache");
    if let Some(tex) =
        ctx.data(|d| d.get_temp::<BrandTextureCache>(cache_id).and_then(|c| c.entries.iter().find(|(k, _)| *k == key).map(|(_, t)| t.clone())))
    {
        return tex;
    }

    let master = master_brand();
    let img = resample_rgba_premultiplied(master.width, master.height, &master.rgba_unmult, key.0, key.1);
    let options = TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: None,
    };
    let tex = ctx.load_texture(format!("curvebook-brand-mark-{}x{}", key.0, key.1), img, options);
    ctx.data_mut(|d| {
        let mut cache = d.get_temp::<BrandTextureCache>(cache_id).unwrap_or_default();
        if let Some(pos) = cache.entries.iter().position(|(k, _)| *k == key) {
            let _ = cache.entries.remove(pos);
        } else if cache.entries.len() >= MAX_CACHED_BRAND_TEXTURES {
            let _ = cache.entries.remove(0);
        }
        cache.entries.push((key, tex.clone()));
        d.insert_temp(cache_id, cache);
    });
    tex
}

/// Paint the CurveBook brand mark into `rect` using an exact physical-pixel premultiplied texture.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn paint_mark(ui: &Ui, rect: Rect) {
    let ppp = ui.ctx().pixels_per_point();
    let safe_ppp = if ppp.is_finite() { ppp.clamp(0.5, 8.0) } else { 1.0 };
    let phys_w = (rect.width().max(1.0) * safe_ppp).round().max(1.0) as usize;
    let phys_h = (rect.height().max(1.0) * safe_ppp).round().max(1.0) as usize;
    let tex = texture_for_size(ui.ctx(), phys_w, phys_h);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
}

/// Expose the cached texture id for a given physical size in unit and integration tests.
#[cfg(test)]
#[must_use]
pub fn texture_id_for_size(ctx: &Context, phys_w: usize, phys_h: usize) -> egui::TextureId {
    texture_for_size(ctx, phys_w, phys_h).id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_mark_trims_letterbox_to_natural_aspect_ratio_and_high_res() {
        let img = decode_brand_mark();
        let [w, h] = img.size;
        assert!(w >= 300 && h >= 200, "expected high-res trimmed brand mark, got {w}x{h}");
        let ratio = w as f32 / h as f32;
        assert!((ratio - 1.43).abs() <= 0.03, "expected ~1.43:1 natural ratio, got {ratio:.4} ({w}x{h})");
        // All four trimmed borders contain artwork pixels (double letterboxing removed).
        assert!((0..w).any(|x| img.pixels[x].a() >= ALPHA_TRIM_THRESHOLD), "top border empty");
        assert!((0..w).any(|x| img.pixels[(h - 1) * w + x].a() >= ALPHA_TRIM_THRESHOLD), "bottom border empty");
        assert!((0..h).any(|y| img.pixels[y * w].a() >= ALPHA_TRIM_THRESHOLD), "left border empty");
        assert!((0..h).any(|y| img.pixels[y * w + (w - 1)].a() >= ALPHA_TRIM_THRESHOLD), "right border empty");
        assert!(img.pixels.iter().any(|p| p.a() == 255), "brand mark must contain opaque artwork pixels");
    }

    #[test]
    fn premultiplied_area_downsample_avoids_dark_alpha_fringe_halos() {
        // 4×4 image: left half solid bright coral-red, right half fully transparent black.
        let mut rgba = Vec::with_capacity(4 * 4 * 4);
        for _y in 0..4 {
            for x in 0..4 {
                if x < 2 {
                    rgba.extend_from_slice(&[240, 80, 60, 255]);
                } else {
                    rgba.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
        // Downsample 4×4 -> 1×1: should yield ~50% alpha and retain coral hue (not dark RGB).
        let img = resample_rgba_premultiplied(4, 4, &rgba, 1, 1);
        assert_eq!(img.size, [1, 1]);
        let px = img.pixels[0];
        assert!((120..=135).contains(&px.a()), "expected ~50% alpha, got {}", px.a());
        // `Color32` stores premultiplied sRGBA: red channel should dominate green/blue cleanly without black crush.
        assert!(px.r() >= 100 && px.r() > px.g() * 2 && px.r() > px.b() * 2, "unexpected dark fringe / hue shift: {px:?}");

        // Pure transparent region stays fully transparent.
        let half = resample_rgba_premultiplied(4, 4, &rgba, 2, 2);
        assert_eq!(half.pixels[1], Color32::TRANSPARENT);
        assert_eq!(half.pixels[3], Color32::TRANSPARENT);
    }

    #[test]
    fn brand_mark_uploads_once_per_physical_size_and_reuses_across_frames_and_dpi() {
        let ctx = Context::default();
        let run_at_dpi = |ppp: f32, size_pt: Vec2| {
            let mut raw = egui::RawInput::default();
            if let Some(vp) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
                vp.native_pixels_per_point = Some(ppp);
            }
            let mut out = ctx.run_ui(raw, |ui| {
                let (rect, _) = ui.allocate_exact_size(size_pt, egui::Sense::hover());
                paint_mark(ui, rect);
            });
            let phys_w = (size_pt.x * ppp).round() as usize;
            let phys_h = (size_pt.y * ppp).round() as usize;
            let uploads: Vec<[usize; 2]> = out
                .textures_delta
                .set
                .iter()
                .filter(|(id, _)| **id != egui::TextureId::Managed(0))
                .flat_map(|(_, deltas)| deltas.iter())
                .map(|d| d.image.size())
                .collect();
            out.textures_delta.clear();
            let tex_id = texture_id_for_size(&ctx, phys_w, phys_h);
            let painted = out.shapes.iter().filter(|clipped| matches!(&clipped.shape, egui::Shape::Mesh(m) if m.texture_id == tex_id)).count();
            (uploads, painted, [phys_w, phys_h])
        };

        // 1.0x Toolbar (33x23): uploads once, reuses on frame 2.
        let (u1, p1, s1) = run_at_dpi(1.0, TOOLBAR_MARK_SIZE);
        assert_eq!((u1, p1), (vec![s1], 1));
        let (u2, p2, _) = run_at_dpi(1.0, TOOLBAR_MARK_SIZE);
        assert_eq!((u2, p2), (Vec::new(), 1));

        // 1.5x Toolbar (50x35): uploads dedicated 1:1 physical texture once, reuses on frame 2.
        let (u3, p3, s3) = run_at_dpi(1.5, TOOLBAR_MARK_SIZE);
        assert_eq!((u3, p3, s3), (vec![[50, 35]], 1, [50, 35]));
        let (u4, p4, _) = run_at_dpi(1.5, TOOLBAR_MARK_SIZE);
        assert_eq!((u4, p4), (Vec::new(), 1));

        // 2.0x About (126x88): uploads exact 126x88 texture once, reuses on frame 2.
        let (u5, p5, s5) = run_at_dpi(2.0, ABOUT_MARK_SIZE);
        assert_eq!((u5, p5, s5), (vec![[126, 88]], 1, [126, 88]));
        let (u6, p6, _) = run_at_dpi(2.0, ABOUT_MARK_SIZE);
        assert_eq!((u6, p6), (Vec::new(), 1));
    }
}
