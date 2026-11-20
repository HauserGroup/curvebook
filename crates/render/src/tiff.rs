//! Publication RGBA TIFF 6.0 encoder supporting uncompressed, Deflate, and LZW strip payloads.

use std::borrow::Cow;
use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};
use weezl::BitOrder;
use weezl::encode::Encoder as LzwEncoder;

use crate::rasterizer::{RasterImage, RenderError};

/// Supported TIFF strip compression modes for journal artwork export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TiffCompression {
    None,
    #[default]
    Deflate,
    Lzw,
}

/// Encode a [`RasterImage`] into a little-endian RGBA TIFF 6.0 buffer with DPI resolution tags.
///
/// # Errors
/// Returns [`RenderError`] on invalid target geometry, buffer length mismatch, or compression failure.
pub fn encode_tiff(image: &RasterImage, compression: TiffCompression) -> Result<Vec<u8>, RenderError> {
    let rgba = image.validated_rgba("TIFF")?;

    let (compression_tag, payload): (u16, Cow<'_, [u8]>) = match compression {
        TiffCompression::None => (1u16, Cow::Borrowed(rgba)),
        TiffCompression::Deflate => {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(rgba).map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
            let bytes = encoder.finish().map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
            (8u16, Cow::Owned(bytes))
        }
        TiffCompression::Lzw => (5u16, Cow::Owned(encode_tiff_lzw(rgba)?)),
    };

    // Layout: 8-byte header | extra values (BitsPerSample, XRes, YRes) | IFD (13 entries) | image strip
    let header_len = 8u32;
    let bps_offset = header_len; // 4 x u16 = 8 bytes
    let xres_offset = bps_offset + 8; // 2 x u32 = 8 bytes
    let yres_offset = xres_offset + 8; // 2 x u32 = 8 bytes
    let ifd_offset = yres_offset + 8; // offset 32
    let tag_count = 13u16;
    let ifd_len = 2 + u32::from(tag_count) * 12 + 4; // 162 bytes
    let strip_offset = ifd_offset + ifd_len;
    let strip_len = u32::try_from(payload.len())
        .map_err(|_| RenderError::EncodingFailed { message: String::from("TIFF strip exceeds 4 GiB baseline container limit") })?;
    if strip_offset.checked_add(strip_len).is_none() {
        return Err(RenderError::EncodingFailed { message: String::from("TIFF total file size exceeds 4 GiB baseline container limit") });
    }

    let mut out = Vec::with_capacity((strip_offset as usize).saturating_add(payload.len()));
    // Little-endian TIFF header + IFD offset
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&ifd_offset.to_le_bytes());

    // BitsPerSample = [8, 8, 8, 8]
    for _ in 0..4 {
        out.extend_from_slice(&8u16.to_le_bytes());
    }
    // XResolution & YResolution rationals (dpi / 1)
    out.extend_from_slice(&u32::from(image.dpi).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&u32::from(image.dpi).to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());

    // IFD sorted ascending by tag number
    out.extend_from_slice(&tag_count.to_le_bytes());
    write_ifd_long(&mut out, 256, 4, 1, image.width_px); // ImageWidth
    write_ifd_long(&mut out, 257, 4, 1, image.height_px); // ImageLength
    write_ifd_long(&mut out, 258, 3, 4, bps_offset); // BitsPerSample
    write_ifd_short(&mut out, 259, compression_tag); // Compression
    write_ifd_short(&mut out, 262, 2); // PhotometricInterpretation = RGB
    write_ifd_long(&mut out, 273, 4, 1, strip_offset); // StripOffsets
    write_ifd_short(&mut out, 277, 4); // SamplesPerPixel = 4
    write_ifd_long(&mut out, 278, 4, 1, image.height_px); // RowsPerStrip
    write_ifd_long(&mut out, 279, 4, 1, strip_len); // StripByteCounts
    write_ifd_long(&mut out, 282, 5, 1, xres_offset); // XResolution
    write_ifd_long(&mut out, 283, 5, 1, yres_offset); // YResolution
    write_ifd_short(&mut out, 296, 2); // ResolutionUnit = Inch
    write_ifd_short(&mut out, 338, 2); // ExtraSamples = Unassociated alpha
    out.extend_from_slice(&0u32.to_le_bytes()); // Next IFD offset

    out.extend_from_slice(&payload);
    Ok(out)
}

fn write_ifd_short(out: &mut Vec<u8>, tag: u16, value: u16) {
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&value.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
}

fn write_ifd_long(out: &mut Vec<u8>, tag: u16, ty: u16, count: u32, value: u32) {
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&value.to_le_bytes());
}

/// TIFF 6.0 MSB-first variable-bit-width (9..=12 bit) LZW compressor with TIFF early-change
/// bit-width transitions.
fn encode_tiff_lzw(raw: &[u8]) -> Result<Vec<u8>, RenderError> {
    LzwEncoder::with_tiff_size_switch(BitOrder::Msb, 8).encode(raw).map_err(|err| RenderError::EncodingFailed { message: err.to_string() })
}
