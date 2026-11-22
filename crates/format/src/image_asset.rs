//! Permissive-policy raster image asset ingestion and validation (`PNG`, `JPEG`, `TIFF`).
//!
//! SVG image import is intentionally omitted under CurveBook's permissive-only dependency policy
//! (`docs/licences.md`).

use std::io::Cursor;

use curvebook_doc::{
    ImageMimeKind, MAX_IMAGE_ASSET_BYTES, MAX_IMAGE_DIM_PX, MAX_IMAGE_PIXELS, ProjectImageAsset, content_addressed_asset_id, is_valid_asset_id,
};
use image::{ImageFormat, ImageReader, Limits};

use crate::error::FormatError;

const fn image_format_for_mime(kind: ImageMimeKind) -> ImageFormat {
    match kind {
        ImageMimeKind::Png => ImageFormat::Png,
        ImageMimeKind::Jpeg => ImageFormat::Jpeg,
        ImageMimeKind::Tiff => ImageFormat::Tiff,
    }
}

fn bounded_decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIM_PX);
    limits.max_image_height = Some(MAX_IMAGE_DIM_PX);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS.saturating_mul(16));
    limits
}

fn bounded_reader(bytes: &[u8], kind: ImageMimeKind) -> ImageReader<Cursor<&[u8]>> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format_for_mime(kind));
    reader.limits(bounded_decode_limits());
    reader
}

fn has_svg_or_xml_signature(bytes: &[u8]) -> bool {
    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let mut idx = 0_usize;
    while idx < stripped.len() && stripped[idx].is_ascii_whitespace() {
        idx += 1;
    }
    let head = &stripped[idx..stripped.len().min(idx + 16)];
    let lower: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    lower.starts_with(b"<svg") || lower.starts_with(b"<?xml") || lower.starts_with(b"<!doctype svg")
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), FormatError> {
    if width == 0 || height == 0 || width > MAX_IMAGE_DIM_PX || height > MAX_IMAGE_DIM_PX {
        return Err(FormatError::UnsupportedFormat {
            reason: format!("image dimensions {width}x{height} out of supported range 1..={MAX_IMAGE_DIM_PX}"),
        });
    }
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels > MAX_IMAGE_PIXELS {
        return Err(FormatError::UnsupportedFormat { reason: format!("image pixel count {pixels} exceeds limit {MAX_IMAGE_PIXELS}") });
    }
    Ok(())
}

/// Sniff, validate, bounded-decode, and content-address raw PNG/JPEG/TIFF image bytes.
///
/// Rejects empty inputs, inputs exceeding [`MAX_IMAGE_ASSET_BYTES`], SVG/XML documents (explicitly
/// omitted under CurveBook's permissive-only dependency policy), unrecognized magic headers,
/// out-of-bounds dimensions, and corrupt/truncated streams.
pub fn ingest_image_asset(bytes: &[u8]) -> Result<ProjectImageAsset, FormatError> {
    if bytes.is_empty() {
        return Err(FormatError::UnsupportedFormat { reason: String::from("image payload must not be empty") });
    }
    if bytes.len() > MAX_IMAGE_ASSET_BYTES {
        return Err(FormatError::FileTooLarge {
            path: String::from("<image>"),
            actual_bytes: Some(bytes.len() as u64),
            limit_bytes: MAX_IMAGE_ASSET_BYTES,
        });
    }
    if has_svg_or_xml_signature(bytes) {
        return Err(FormatError::UnsupportedFormat {
            reason: String::from(
                "SVG image import is intentionally omitted under CurveBook's permissive-only dependency policy; supported formats are PNG, JPEG, and TIFF",
            ),
        });
    }
    let media_type = ImageMimeKind::sniff_header(bytes).ok_or_else(|| FormatError::UnsupportedFormat {
        reason: String::from("unrecognized image signature; supported formats are PNG, JPEG, and TIFF"),
    })?;

    let (width, height) = bounded_reader(bytes, media_type)
        .into_dimensions()
        .map_err(|err| FormatError::UnsupportedFormat { reason: format!("failed to read {} header dimensions: {err}", media_type.mime_type()) })?;
    validate_dimensions(width, height)?;

    // Decode upfront to reject truncated or corrupt image bitstreams before storing them in a project.
    let decoded = bounded_reader(bytes, media_type)
        .decode()
        .map_err(|err| FormatError::UnsupportedFormat { reason: format!("corrupt {} image payload: {err}", media_type.mime_type()) })?;
    if decoded.width() != width || decoded.height() != height {
        return Err(FormatError::UnsupportedFormat {
            reason: format!(
                "{} header dimensions {width}x{height} disagree with decoded {}x{}",
                media_type.mime_type(),
                decoded.width(),
                decoded.height()
            ),
        });
    }

    Ok(ProjectImageAsset { asset_id: content_addressed_asset_id(bytes), media_type, pixel_size: (width, height), bytes: bytes.to_vec() })
}

/// Verify an existing [`ProjectImageAsset`]'s ID syntax, size limit, SHA-256 content address,
/// container header signature, and intrinsic pixel dimensions.
pub fn validate_image_asset(asset: &ProjectImageAsset) -> Result<(), FormatError> {
    validate_image_asset_parts(&asset.asset_id, asset.media_type, asset.pixel_size, &asset.bytes)
}

/// Validate image asset metadata and payload bytes directly from a borrowed slice without
/// allocating or cloning `bytes` upfront.
pub(crate) fn validate_image_asset_parts(asset_id: &str, media_type: ImageMimeKind, pixel_size: (u32, u32), bytes: &[u8]) -> Result<(), FormatError> {
    if !is_valid_asset_id(asset_id) {
        return Err(FormatError::UnsupportedFormat { reason: format!("invalid image asset identifier `{asset_id}`") });
    }
    if bytes.is_empty() {
        return Err(FormatError::MissingArchiveEntry { path: format!("assets/{asset_id}.{}", media_type.extension()) });
    }
    if bytes.len() > MAX_IMAGE_ASSET_BYTES {
        return Err(FormatError::FileTooLarge {
            path: format!("assets/{asset_id}.{}", media_type.extension()),
            actual_bytes: Some(bytes.len() as u64),
            limit_bytes: MAX_IMAGE_ASSET_BYTES,
        });
    }
    let actual_id = content_addressed_asset_id(bytes);
    if actual_id != asset_id {
        return Err(FormatError::UnsupportedFormat { reason: format!("image asset `{asset_id}` SHA-256 digest mismatch (computed `{actual_id}`)") });
    }
    let Some(sniffed) = ImageMimeKind::sniff_header(bytes) else {
        return Err(FormatError::UnsupportedFormat { reason: format!("image asset `{asset_id}` has unrecognized magic bytes") });
    };
    if sniffed != media_type {
        return Err(FormatError::UnsupportedFormat {
            reason: format!("image asset `{asset_id}` declares {media_type:?} but magic bytes are {sniffed:?}"),
        });
    }
    let (width, height) = bounded_reader(bytes, media_type)
        .into_dimensions()
        .map_err(|err| FormatError::UnsupportedFormat { reason: format!("image asset `{asset_id}` header could not be decoded: {err}") })?;
    validate_dimensions(width, height)?;
    if (width, height) != pixel_size {
        return Err(FormatError::UnsupportedFormat {
            reason: format!(
                "image asset `{asset_id}` declares pixel_size {}x{} but image header reports {width}x{height}",
                pixel_size.0, pixel_size.1
            ),
        });
    }
    Ok(())
}
