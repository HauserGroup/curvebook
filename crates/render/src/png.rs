//! Baseline RGBA PNG encoder with physical DPI `pHYs` chunk metadata.

use std::io::Write;

use png::{BitDepth, ColorType, Encoder, PixelDimensions, Unit};

use crate::rasterizer::{RasterImage, RenderError};

/// Encode a [`RasterImage`] into an RGBA PNG byte buffer including DPI (`pHYs`) metadata.
///
/// # Errors
/// Returns [`RenderError`] on invalid target dimensions/DPI, pixel buffer mismatch, or PNG
/// compression failure.
pub fn encode_png(image: &RasterImage) -> Result<Vec<u8>, RenderError> {
    let rgba = image.validated_rgba("PNG")?;
    let ppm = encode_pixels_per_meter(image.dpi);
    let row_bytes = (image.width_px as usize) * 4;

    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, image.width_px, image.height_px);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_pixel_dims(Some(PixelDimensions { xppu: ppm, yppu: ppm, unit: Unit::Meter }));

        let mut writer = encoder.write_header().map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
        {
            let mut stream = writer.stream_writer().map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
            for row in rgba.chunks_exact(row_bytes) {
                stream.write_all(row).map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
            }
            stream.finish().map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
        }
        writer.finish().map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
    }
    Ok(out)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn encode_pixels_per_meter(dpi: u16) -> u32 {
    (f64::from(dpi) / 0.0254).round().clamp(1.0, f64::from(u32::MAX)) as u32
}
