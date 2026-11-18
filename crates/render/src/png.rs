//! Baseline RGBA PNG encoder with physical DPI `pHYs` chunk metadata.

use std::io::Write;

use crc32fast::Hasher;
use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::rasterizer::{RasterImage, RenderError, validate_raster_target_dims};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Encode a [`RasterImage`] into an RGBA PNG byte buffer including DPI (`pHYs`) metadata.
///
/// # Errors
/// Returns [`RenderError`] on invalid target dimensions/DPI, pixel buffer mismatch, or DEFLATE
/// compression failure.
pub fn encode_png(image: &RasterImage) -> Result<Vec<u8>, RenderError> {
    let _ = validate_raster_target_dims(image.width_px, image.height_px, image.dpi, "PNG")?;
    let expected = image.expected_byte_len().ok_or_else(|| RenderError::InvalidTarget { message: String::from("image byte length overflow") })?;
    if image.rgba.len() != expected {
        return Err(RenderError::BufferSizeMismatch { expected, actual: image.rgba.len() });
    }

    let row_bytes = (image.width_px as usize) * 4;
    let raw_len = row_bytes
        .checked_add(1)
        .and_then(|stride| stride.checked_mul(image.height_px as usize))
        .ok_or_else(|| RenderError::InvalidTarget { message: String::from("PNG scanline buffer size overflow") })?;
    let mut raw_scanlines = Vec::with_capacity(raw_len);
    for row in image.rgba.chunks_exact(row_bytes) {
        append_adaptive_scanline(&mut raw_scanlines, row);
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw_scanlines).map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
    let idat = encoder.finish().map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;

    let mut out = Vec::with_capacity(idat.len().saturating_add(128));
    out.extend_from_slice(PNG_SIGNATURE);

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&image.width_px.to_be_bytes());
    ihdr[4..8].copy_from_slice(&image.height_px.to_be_bytes());
    ihdr[8] = 8; // Bit depth
    ihdr[9] = 6; // RGBA color type
    write_png_chunk(&mut out, b"IHDR", &ihdr)?;

    let ppm = encode_pixels_per_meter(image.dpi);
    let mut phys = [0u8; 9];
    phys[0..4].copy_from_slice(&ppm.to_be_bytes());
    phys[4..8].copy_from_slice(&ppm.to_be_bytes());
    phys[8] = 1; // Unit: meter
    write_png_chunk(&mut out, b"pHYs", &phys)?;

    write_png_chunk(&mut out, b"IDAT", &idat)?;
    write_png_chunk(&mut out, b"IEND", &[])?;
    Ok(out)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn encode_pixels_per_meter(dpi: u16) -> u32 {
    (f64::from(dpi) / 0.0254).round().clamp(1.0, f64::from(u32::MAX)) as u32
}

fn append_adaptive_scanline(out: &mut Vec<u8>, row: &[u8]) {
    let mut sad_none = 0usize;
    let mut sad_sub = 0usize;
    for (idx, &byte) in row.iter().enumerate() {
        sad_none += usize::from(byte.cast_signed().unsigned_abs());
        let left = idx.checked_sub(4).and_then(|j| row.get(j)).copied().unwrap_or(0);
        let diff = byte.wrapping_sub(left);
        sad_sub += usize::from(diff.cast_signed().unsigned_abs());
    }

    if sad_sub < sad_none {
        out.push(1); // Filter type 1 (Sub)
        for (idx, &byte) in row.iter().enumerate() {
            let left = idx.checked_sub(4).and_then(|j| row.get(j)).copied().unwrap_or(0);
            out.push(byte.wrapping_sub(left));
        }
    } else {
        out.push(0); // Filter type 0 (None)
        out.extend_from_slice(row);
    }
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) -> Result<(), RenderError> {
    let len = u32::try_from(data.len()).map_err(|_| RenderError::EncodingFailed { message: String::from("PNG chunk exceeds 4 GiB limit") })?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let mut hasher = Hasher::new();
    hasher.update(chunk_type);
    hasher.update(data);
    out.extend_from_slice(&hasher.finalize().to_be_bytes());
    Ok(())
}
