//! CurveBook in-app brand mark: the seven-pane logo (`assets/app-icon/`, see its README).
//!
//! Decoded once per `egui::Context` from the 128 px hicolor PNG into a mipmapped texture so it
//! stays crisp from the Tier 1 application toolbar mark (`20 pt`) to the About dialog (`44 pt`).

use egui::{Color32, ColorImage, Context, Id, Rect, TextureHandle, TextureOptions, Ui, pos2};

/// 128 px hicolor PNG: sharp at 44 pt on 2× Retina displays, with linear mipmaps for 20 pt toolbar.
const ICON_PNG: &[u8] = include_bytes!("../../../../assets/app-icon/hicolor/128x128/apps/org.curvebook.CurveBook.png");

/// Decode the embedded brand mark PNG into an unmultiplied RGBA [`ColorImage`].
///
/// Falls back to a 1×1 transparent pixel on decode failure (ruled out by unit tests).
#[must_use]
pub fn decode_brand_mark() -> ColorImage {
    let decode = || -> Option<ColorImage> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(ICON_PNG));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let buf_len = reader.output_buffer_size()?;
        let mut buf = vec![0u8; buf_len];
        let info = reader.next_frame(&mut buf).ok()?;
        let bytes = buf.get(..info.buffer_size())?;
        let size = [info.width as usize, info.height as usize];
        match info.color_type {
            png::ColorType::Rgba => Some(ColorImage::from_rgba_unmultiplied(size, bytes)),
            png::ColorType::Rgb => Some(ColorImage::from_rgb(size, bytes)),
            _ => None,
        }
    };
    decode().unwrap_or_else(|| ColorImage::filled([1, 1], Color32::TRANSPARENT))
}

/// Upload the brand mark texture on first use and cache it in the `egui::Context`.
fn texture(ctx: &Context) -> TextureHandle {
    let id = Id::unique("curvebook-brand-mark");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    let tex = ctx.load_texture("curvebook-brand-mark", decode_brand_mark(), options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Paint the CurveBook brand mark into square `rect`.
pub fn paint_mark(ui: &Ui, rect: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture(ui.ctx()).id(), rect, uv, Color32::WHITE);
}

/// Expose the cached texture id for unit and integration tests.
#[cfg(test)]
#[must_use]
pub fn texture_id(ctx: &Context) -> egui::TextureId {
    texture(ctx).id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_mark_decodes_128px_transparent_rgba() {
        let img = decode_brand_mark();
        assert_eq!(img.size, [128, 128]);
        // Outer corners are transparent; interior artwork contains opaque pixels.
        assert_eq!(img.pixels[0].a(), 0);
        assert!(img.pixels.iter().any(|p| p.a() == 255), "brand mark must contain opaque artwork pixels");
    }

    #[test]
    fn brand_mark_uploads_once_and_reuses_cached_texture() {
        let ctx = Context::default();
        let run_frame = || {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                paint_mark(ui, rect);
            });
            let uploads = out.textures_delta.set.values().flat_map(|d| d.iter()).filter(|d| d.image.size() == [128, 128]).count();
            out.textures_delta.clear();
            let tex_id = texture_id(&ctx);
            let painted = out.shapes.iter().filter(|clipped| matches!(&clipped.shape, egui::Shape::Mesh(m) if m.texture_id == tex_id)).count();
            (uploads, painted)
        };

        assert_eq!(run_frame(), (1, 1));
        assert_eq!(run_frame(), (0, 1));
    }
}
