//! Publication RGBA TIFF 6.0 encoder supporting uncompressed, Deflate, and LZW strip payloads.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};

use crate::rasterizer::{MAX_RASTER_DIM_PX, RasterImage, RenderError};

const CLEAR_CODE: u16 = 256;
const EOI_CODE: u16 = 257;
const FIRST_DICT_CODE: u16 = 258;
const MAX_DICT_CODE: u16 = 4094;
const HASH_SIZE: usize = 8192;
const HASH_MASK: usize = HASH_SIZE - 1;

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
    if image.width_px == 0 || image.height_px == 0 || image.dpi == 0 {
        return Err(RenderError::InvalidTarget { message: String::from("TIFF dimensions and DPI must be positive") });
    }
    if image.width_px > MAX_RASTER_DIM_PX || image.height_px > MAX_RASTER_DIM_PX {
        return Err(RenderError::InvalidTarget {
            message: format!("TIFF dimensions {}x{} exceed maximum {MAX_RASTER_DIM_PX}x{MAX_RASTER_DIM_PX}", image.width_px, image.height_px),
        });
    }
    let expected = image.expected_byte_len().ok_or_else(|| RenderError::InvalidTarget { message: String::from("image byte length overflow") })?;
    if image.rgba.len() != expected {
        return Err(RenderError::BufferSizeMismatch { expected, actual: image.rgba.len() });
    }

    let (compression_tag, payload) = match compression {
        TiffCompression::None => (1u16, image.rgba.clone()),
        TiffCompression::Deflate => {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&image.rgba).map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
            let bytes = encoder.finish().map_err(|err| RenderError::EncodingFailed { message: err.to_string() })?;
            (8u16, bytes)
        }
        TiffCompression::Lzw => (5u16, encode_tiff_lzw(&image.rgba)),
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

struct LzwEncoderState {
    out: Vec<u8>,
    bit_buf: u32,
    bit_count: u8,
    hash_keys: Vec<u32>,
    hash_codes: Vec<u16>,
    parent_of: Vec<u16>,
    byte_of: Vec<u8>,
    next_code: u16,
    code_bits: u8,
}

impl LzwEncoderState {
    fn new(capacity: usize) -> Self {
        Self {
            out: Vec::with_capacity(capacity),
            bit_buf: 0,
            bit_count: 0,
            hash_keys: vec![u32::MAX; HASH_SIZE],
            hash_codes: vec![0; HASH_SIZE],
            parent_of: vec![0; 4096],
            byte_of: vec![0; 4096],
            next_code: FIRST_DICT_CODE,
            code_bits: 9,
        }
    }

    fn push_code(&mut self, code: u16) {
        self.bit_buf = (self.bit_buf << u32::from(self.code_bits)) | u32::from(code);
        self.bit_count += self.code_bits;
        while self.bit_count >= 8 {
            self.bit_count -= 8;
            self.out.push(((self.bit_buf >> self.bit_count) & 0xFF) as u8);
        }
    }

    fn reset_table(&mut self) {
        self.hash_keys.fill(u32::MAX);
        self.next_code = FIRST_DICT_CODE;
        self.code_bits = 9;
    }

    fn lookup_or_insert(&mut self, parent: u16, byte: u8) -> Result<u16, ()> {
        let key = (u32::from(parent) << 8) | u32::from(byte);
        let mut slot = ((key.wrapping_mul(2_654_435_761)) as usize) & HASH_MASK;
        for _ in 0..HASH_SIZE {
            let existing = self.hash_keys.get(slot).copied().unwrap_or(u32::MAX);
            if existing == key {
                return Ok(self.hash_codes.get(slot).copied().unwrap_or(0));
            }
            if existing == u32::MAX {
                if let Some(k_slot) = self.hash_keys.get_mut(slot) {
                    *k_slot = key;
                }
                if let Some(c_slot) = self.hash_codes.get_mut(slot) {
                    *c_slot = self.next_code;
                }
                if let Some(p_slot) = self.parent_of.get_mut(usize::from(self.next_code)) {
                    *p_slot = parent;
                }
                if let Some(b_slot) = self.byte_of.get_mut(usize::from(self.next_code)) {
                    *b_slot = byte;
                }
                return Err(());
            }
            slot = (slot + 1) & HASH_MASK;
        }
        Err(())
    }

    fn decode_entry(&self, mut code: u16) -> Vec<u8> {
        let mut rev = Vec::new();
        let mut guard = 0usize;
        while code >= FIRST_DICT_CODE && guard < 4096 {
            let idx = usize::from(code);
            rev.push(self.byte_of.get(idx).copied().unwrap_or(0));
            code = self.parent_of.get(idx).copied().unwrap_or(0);
            guard += 1;
        }
        if let Ok(b) = u8::try_from(code) {
            rev.push(b);
        }
        rev.reverse();
        rev
    }

    fn flush_tail(&mut self, last_code: u16) {
        // Guard exact early-change bit-width boundary (`next_code` at 511/1023/2047) so encoder
        // and every compliant TIFF 6.0 decoder agree on the bit width of the terminating EOI code.
        if matches!(self.next_code, 511 | 1023 | 2047) {
            let tail = self.decode_entry(last_code);
            self.push_code(CLEAR_CODE);
            self.reset_table();
            for (idx, &byte) in tail.iter().enumerate() {
                if idx > 0 && idx % 200 == 0 {
                    self.push_code(CLEAR_CODE);
                }
                self.push_code(u16::from(byte));
            }
        } else {
            self.push_code(last_code);
        }
        self.push_code(EOI_CODE);
        if self.bit_count > 0 {
            self.out.push(((self.bit_buf << (8 - self.bit_count)) & 0xFF) as u8);
        }
    }
}

/// Clean-room TIFF 6.0 MSB-first variable-bit-width (9..=12 bit) LZW compressor with Clear (`256`)
/// and EOI (`257`) codes and TIFF early-change bit-width transitions.
fn encode_tiff_lzw(raw: &[u8]) -> Vec<u8> {
    let mut state = LzwEncoderState::new(raw.len() / 2 + 16);
    state.push_code(CLEAR_CODE);

    let mut omega: Option<u16> = None;
    for &byte in raw {
        let Some(parent) = omega else {
            omega = Some(u16::from(byte));
            continue;
        };
        if let Ok(child) = state.lookup_or_insert(parent, byte) {
            omega = Some(child);
            continue;
        }
        state.push_code(parent);
        state.next_code += 1;
        if state.next_code == 512 {
            state.code_bits = 10;
        } else if state.next_code == 1024 {
            state.code_bits = 11;
        } else if state.next_code == 2048 {
            state.code_bits = 12;
        }
        if state.next_code >= MAX_DICT_CODE {
            state.push_code(CLEAR_CODE);
            state.reset_table();
        }
        omega = Some(u16::from(byte));
    }

    if let Some(last_code) = omega {
        state.flush_tail(last_code);
    } else {
        state.push_code(EOI_CODE);
        if state.bit_count > 0 {
            state.out.push(((state.bit_buf << (8 - state.bit_count)) & 0xFF) as u8);
        }
    }
    state.out
}
