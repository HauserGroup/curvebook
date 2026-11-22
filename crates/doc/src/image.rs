//! Embedded raster image asset model, pure-Rust SHA-256 content addressing, crop insets, and
//! border styling (`docs/implementation/graphing-and-layouts.md`, `R13.7`).
//!
//! Raster decoders (`PNG`, `JPEG`, and `TIFF`) live in downstream L2/L3 crates; SVG image import
//! is intentionally omitted under CurveBook's permissive-only dependency policy (`docs/licences.md`).

use std::fmt::Write as _;

use curvebook_palette::{LineDashStyle, Rgba8};

use crate::TableKind;
use crate::error::DocError;
use crate::ids::RectPt;

/// Hard ceiling on raw encoded image asset bytes (`32 MiB`).
pub const MAX_IMAGE_ASSET_BYTES: usize = 32 * 1024 * 1024;

/// Hard ceiling on decoded image width or height in pixels (`16,384 px`).
pub const MAX_IMAGE_DIM_PX: u32 = 16_384;

/// Hard ceiling on decoded pixel count per image asset and per compiled layout (`8192 × 8192`).
pub const MAX_IMAGE_PIXELS: u64 = 67_108_864;

/// Maximum number of deduplicated content-addressed image assets retained per project.
pub const MAX_PROJECT_IMAGE_ASSETS: usize = 256;

/// Supported permissive-policy embedded raster image formats (`PNG`, `JPEG`, `TIFF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ImageMimeKind {
    /// Portable Network Graphics (`image/png`).
    Png,
    /// Joint Photographic Experts Group (`image/jpeg`).
    Jpeg,
    /// Tagged Image File Format (`image/tiff`).
    Tiff,
}

impl ImageMimeKind {
    /// Canonical lowercase archive file extension (`png`, `jpg`, or `tif`).
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Tiff => "tif",
        }
    }

    /// Standard IANA MIME media type string.
    #[must_use]
    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Tiff => "image/tiff",
        }
    }

    /// Parse an extension or MIME token (case-insensitive).
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        let lower = raw.trim().to_ascii_lowercase();
        match lower.as_str() {
            "png" | "image/png" => Some(Self::Png),
            "jpg" | "jpeg" | "image/jpeg" | "image/jpg" => Some(Self::Jpeg),
            "tif" | "tiff" | "image/tiff" => Some(Self::Tiff),
            _ => None,
        }
    }

    /// Detect [`ImageMimeKind`] strictly from checked leading magic-byte signatures.
    #[must_use]
    pub fn sniff_header(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(&[b'I', b'I', 0x2A, 0x00]) || bytes.starts_with(&[b'M', b'M', 0x00, 0x2A]) {
            Some(Self::Tiff)
        } else {
            None
        }
    }
}

/// Four-edge inward crop insets measured in typographic points relative to `bounds_pt`.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct CropInsetsPt {
    /// Inset clipped from the left edge (`>= 0.0`, finite).
    #[serde(default)]
    pub left_pt: f64,
    /// Inset clipped from the right edge (`>= 0.0`, finite).
    #[serde(default)]
    pub right_pt: f64,
    /// Inset clipped from the top edge (`>= 0.0`, finite).
    #[serde(default)]
    pub top_pt: f64,
    /// Inset clipped from the bottom edge (`>= 0.0`, finite).
    #[serde(default)]
    pub bottom_pt: f64,
}

impl CropInsetsPt {
    /// Return `true` when all four crop insets are finite and non-negative.
    #[must_use]
    pub fn is_finite_non_negative(self) -> bool {
        self.left_pt.is_finite()
            && self.right_pt.is_finite()
            && self.top_pt.is_finite()
            && self.bottom_pt.is_finite()
            && self.left_pt >= 0.0
            && self.right_pt >= 0.0
            && self.top_pt >= 0.0
            && self.bottom_pt >= 0.0
    }

    /// Return a static non-finite float location label if any crop inset is non-finite.
    #[must_use]
    pub fn first_non_finite_float(self, prefix: &'static str) -> Option<&'static str> {
        if self.left_pt.is_finite() && self.right_pt.is_finite() && self.top_pt.is_finite() && self.bottom_pt.is_finite() {
            None
        } else {
            Some(prefix)
        }
    }

    /// Validate that all four crop insets are finite and non-negative.
    pub fn validate(self, archetype: TableKind) -> Result<(), DocError> {
        if !self.is_finite_non_negative() {
            return Err(DocError::InvalidCellValue { archetype, row: 0, col: 0, reason: "embedded image crop insets must be finite and >= 0" });
        }
        Ok(())
    }

    /// Compute the unrotated visible sub-rectangle inside `bounds` after applying these crop insets.
    ///
    /// Returns `None` when `bounds` is non-finite, crop insets are invalid, or the cropped width
    /// or height collapses to `<= 0.0`.
    #[must_use]
    pub fn visible_rect(self, bounds: RectPt) -> Option<RectPt> {
        if !bounds.x.is_finite() || !bounds.y.is_finite() || !bounds.width.is_finite() || !bounds.height.is_finite() || !self.is_finite_non_negative()
        {
            return None;
        }
        let x = bounds.x + self.left_pt;
        let y = bounds.y + self.top_pt;
        let width = bounds.width - self.left_pt - self.right_pt;
        let height = bounds.height - self.top_pt - self.bottom_pt;
        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 { Some(RectPt::new(x, y, width, height)) } else { None }
    }
}

/// Optional outline frame rendered around an embedded image's cropped region.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImageBorderSpec {
    /// Stroke width in typographic points (`>= 0.0`, finite).
    pub width_pt: f64,
    /// Border stroke sRGB color.
    pub color: Rgba8,
    /// Solid, dashed, or dotted stroke pattern.
    #[serde(default)]
    pub dash: LineDashStyle,
}

impl Default for ImageBorderSpec {
    fn default() -> Self {
        Self { width_pt: 0.75, color: Rgba8::BLACK, dash: LineDashStyle::Solid }
    }
}

impl ImageBorderSpec {
    /// Validate border width finiteness and non-negativity.
    pub fn validate(self, archetype: TableKind) -> Result<(), DocError> {
        if !self.width_pt.is_finite() || self.width_pt < 0.0 {
            return Err(DocError::InvalidCellValue { archetype, row: 0, col: 0, reason: "embedded image border width_pt must be finite and >= 0" });
        }
        Ok(())
    }
}

/// Compute the NIST FIPS 180-4 SHA-256 digest of `data`.
#[must_use]
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let mut state: [u32; 8] = [0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19];

    let mut compress = |block: &[u8; 64]| {
        let mut w = [0u32; 64];
        for i in 0..16 {
            let base = i * 4;
            w[i] = u32::from_be_bytes([block[base], block[base + 1], block[base + 2], block[base + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        let updated = [a, b, c, d, e, f, g, h];
        for (slot, delta) in state.iter_mut().zip(updated) {
            *slot = slot.wrapping_add(delta);
        }
    };

    let (blocks, rem) = data.as_chunks::<64>();
    for block in blocks {
        compress(block);
    }
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let total_tail = if rem.len() < 56 { 64 } else { 128 };
    tail[total_tail - 8..total_tail].copy_from_slice(&bit_len.to_be_bytes());

    let mut b0 = [0u8; 64];
    b0.copy_from_slice(&tail[..64]);
    compress(&b0);
    if total_tail == 128 {
        let mut b1 = [0u8; 64];
        b1.copy_from_slice(&tail[64..128]);
        compress(&b1);
    }

    let mut digest = [0u8; 32];
    for (idx, word) in state.into_iter().enumerate() {
        digest[idx * 4..idx * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// Derive a deterministic content-addressed identifier (`img-<16 lowercase hex chars>`) from `bytes`.
#[must_use]
pub fn content_addressed_asset_id(bytes: &[u8]) -> String {
    let digest = sha256_bytes(bytes);
    let mut out = String::with_capacity(4 + 16);
    out.push_str("img-");
    for byte in &digest[..8] {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Return `true` when `id` follows the content-addressed `img-<16 lowercase hex chars>` format.
#[must_use]
pub fn is_valid_asset_id(id: &str) -> bool {
    id.strip_prefix("img-").is_some_and(|hex| hex.len() == 16 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

mod serde_base64_bytes {
    use serde::{Deserializer, Serializer};

    use super::MAX_IMAGE_ASSET_BYTES;

    const ENC_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b0 = chunk.first().copied().unwrap_or(0);
            let b1 = chunk.get(1).copied().unwrap_or(0);
            let b2 = chunk.get(2).copied().unwrap_or(0);
            let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
            out.push(char::from(ENC_TABLE[((triple >> 18) & 0x3F) as usize]));
            out.push(char::from(ENC_TABLE[((triple >> 12) & 0x3F) as usize]));
            out.push(if chunk.len() > 1 { char::from(ENC_TABLE[((triple >> 6) & 0x3F) as usize]) } else { '=' });
            out.push(if chunk.len() > 2 { char::from(ENC_TABLE[(triple & 0x3F) as usize]) } else { '=' });
        }
        serializer.serialize_str(&out)
    }

    /// Decode a base64 JSON string under a hard decoded-byte cap (`MAX_IMAGE_ASSET_BYTES`).
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        struct Base64BytesVisitor;

        impl serde::de::Visitor<'_> for Base64BytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a base64-encoded image byte string within MAX_IMAGE_ASSET_BYTES")
            }

            fn visit_str<E: serde::de::Error>(self, encoded: &str) -> Result<Self::Value, E> {
                let max_encoded_len = MAX_IMAGE_ASSET_BYTES.div_ceil(3).saturating_mul(4);
                let trimmed = encoded.trim();
                if trimmed.len() > max_encoded_len {
                    return Err(E::custom("image asset payload exceeds MAX_IMAGE_ASSET_BYTES"));
                }
                decode_base64(trimmed).map_err(E::custom)
            }

            fn visit_string<E: serde::de::Error>(self, encoded: String) -> Result<Self::Value, E> {
                self.visit_str(&encoded)
            }
        }

        deserializer.deserialize_str(Base64BytesVisitor)
    }

    const fn decode_val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    fn decode_base64(input: &str) -> Result<Vec<u8>, &'static str> {
        let clean = input.trim();
        if clean.is_empty() {
            return Ok(Vec::new());
        }
        let raw = clean.as_bytes();
        if !raw.len().is_multiple_of(4) {
            return Err("invalid base64 length");
        }
        let max_encoded_len = MAX_IMAGE_ASSET_BYTES.div_ceil(3).saturating_mul(4);
        if raw.len() > max_encoded_len {
            return Err("image asset payload exceeds MAX_IMAGE_ASSET_BYTES");
        }
        let mut out = Vec::with_capacity((raw.len() / 4).saturating_mul(3));
        let chunk_count = raw.len() / 4;
        for (idx, quad) in raw.as_chunks::<4>().0.iter().enumerate() {
            let is_last = idx + 1 == chunk_count;
            let (q0, q1, q2, q3) = (quad[0], quad[1], quad[2], quad[3]);
            let v0 = u32::from(decode_val(q0).ok_or("invalid base64 character")?);
            let v1 = u32::from(decode_val(q1).ok_or("invalid base64 character")?);
            if q2 == b'=' {
                if !is_last || q3 != b'=' {
                    return Err("invalid base64 padding");
                }
                let triple = (v0 << 18) | (v1 << 12);
                out.push(((triple >> 16) & 0xFF) as u8);
            } else if q3 == b'=' {
                if !is_last {
                    return Err("invalid base64 padding");
                }
                let v2 = u32::from(decode_val(q2).ok_or("invalid base64 character")?);
                let triple = (v0 << 18) | (v1 << 12) | (v2 << 6);
                out.push(((triple >> 16) & 0xFF) as u8);
                out.push(((triple >> 8) & 0xFF) as u8);
            } else {
                let v2 = u32::from(decode_val(q2).ok_or("invalid base64 character")?);
                let v3 = u32::from(decode_val(q3).ok_or("invalid base64 character")?);
                let triple = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
                out.push(((triple >> 16) & 0xFF) as u8);
                out.push(((triple >> 8) & 0xFF) as u8);
                out.push((triple & 0xFF) as u8);
            }
        }
        if out.len() > MAX_IMAGE_ASSET_BYTES {
            return Err("image asset payload exceeds MAX_IMAGE_ASSET_BYTES");
        }
        Ok(out)
    }
}

/// Content-addressed raster image payload stored alongside a [`crate::ProjectDocument`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProjectImageAsset {
    /// Content-addressed identifier (`img-<sha256-prefix16>`).
    pub asset_id: String,
    /// Verified image container media type (`Png`, `Jpeg`, or `Tiff`).
    pub media_type: ImageMimeKind,
    /// Intrinsic pixel dimensions `(width_px, height_px)`.
    pub pixel_size: (u32, u32),
    /// Encoded image payload bytes (`PNG`, `JPEG`, or `TIFF`), omitted in `document.json` when
    /// persisted as `assets/<asset_id>.<ext>` inside native archives.
    #[serde(default, skip_serializing_if = "Vec::is_empty", with = "serde_base64_bytes")]
    pub bytes: Vec<u8>,
}

impl ProjectImageAsset {
    /// Validate identifier syntax, non-zero pixel dimensions, and byte budget on this asset.
    pub fn validate(&self) -> Result<(), crate::error::DocError> {
        if !is_valid_asset_id(&self.asset_id) {
            return Err(crate::error::DocError::InvalidIdentifier { reason: "asset_id must match `img-<16 lowercase hex>`" });
        }
        if self.pixel_size.0 == 0 || self.pixel_size.1 == 0 || self.pixel_size.0 > MAX_IMAGE_DIM_PX || self.pixel_size.1 > MAX_IMAGE_DIM_PX {
            return Err(crate::error::DocError::InvalidIdentifier {
                reason: "image asset pixel_size dimensions must lie in 1..=MAX_IMAGE_DIM_PX (16,384)",
            });
        }
        if u64::from(self.pixel_size.0).saturating_mul(u64::from(self.pixel_size.1)) > MAX_IMAGE_PIXELS {
            return Err(crate::error::DocError::InvalidIdentifier { reason: "image asset total pixel count exceeds MAX_IMAGE_PIXELS (67,108,864)" });
        }
        if self.bytes.len() > MAX_IMAGE_ASSET_BYTES {
            return Err(crate::error::DocError::InvalidIdentifier { reason: "image asset byte payload exceeds MAX_IMAGE_ASSET_BYTES" });
        }
        Ok(())
    }
}

impl<'de> serde::Deserialize<'de> for ProjectImageAsset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct RawProjectImageAsset {
            asset_id: String,
            media_type: ImageMimeKind,
            pixel_size: (u32, u32),
            #[serde(default, with = "serde_base64_bytes")]
            bytes: Vec<u8>,
        }

        let raw = RawProjectImageAsset::deserialize(deserializer)?;
        let asset = Self { asset_id: raw.asset_id, media_type: raw.media_type, pixel_size: raw.pixel_size, bytes: raw.bytes };
        asset.validate().map_err(serde::de::Error::custom)?;
        Ok(asset)
    }
}

#[cfg(test)]
mod tests {
    use super::{CropInsetsPt, ImageMimeKind, ProjectImageAsset, content_addressed_asset_id, is_valid_asset_id, sha256_bytes};
    use crate::ids::RectPt;

    fn to_hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    #[test]
    fn sha256_matches_nist_fips_180_4_vectors() {
        assert_eq!(to_hex(&sha256_bytes(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(to_hex(&sha256_bytes(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            to_hex(&sha256_bytes(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let id = content_addressed_asset_id(b"abc");
        assert_eq!(id, "img-ba7816bf8f01cfea");
        assert!(is_valid_asset_id(&id));
        assert!(!is_valid_asset_id("img-BA7816BF8F01CFEA"));
        assert!(!is_valid_asset_id("img-abc"));
    }

    #[test]
    fn sniff_header_identifies_png_jpeg_tiff_and_rejects_svg() {
        assert_eq!(ImageMimeKind::sniff_header(b"\x89PNG\r\n\x1a\n\x00\x00"), Some(ImageMimeKind::Png));
        assert_eq!(ImageMimeKind::sniff_header(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageMimeKind::Jpeg));
        assert_eq!(ImageMimeKind::sniff_header(&[b'I', b'I', 0x2A, 0x00]), Some(ImageMimeKind::Tiff));
        assert_eq!(ImageMimeKind::sniff_header(&[b'M', b'M', 0x00, 0x2A]), Some(ImageMimeKind::Tiff));
        assert_eq!(ImageMimeKind::sniff_header(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(ImageMimeKind::sniff_header(b""), None);
    }

    #[test]
    fn crop_insets_visible_rect_and_asset_serde_roundtrip() {
        let bounds = RectPt::new(10.0, 20.0, 100.0, 80.0);
        let crop = CropInsetsPt { left_pt: 5.0, right_pt: 15.0, top_pt: 10.0, bottom_pt: 20.0 };
        assert_eq!(crop.visible_rect(bounds), Some(RectPt::new(15.0, 30.0, 80.0, 50.0)));
        assert_eq!(CropInsetsPt { left_pt: 60.0, right_pt: 40.0, top_pt: 0.0, bottom_pt: 0.0 }.visible_rect(bounds), None);

        let full = ProjectImageAsset {
            asset_id: content_addressed_asset_id(b"\x89PNG\r\n\x1a\ntest"),
            media_type: ImageMimeKind::Png,
            pixel_size: (4, 3),
            bytes: b"\x89PNG\r\n\x1a\ntest".to_vec(),
        };
        let v = serde_json::to_value(&full).unwrap();
        let decoded: ProjectImageAsset = serde_json::from_value(v).unwrap();
        assert_eq!(decoded, full);

        let dehydrated = serde_json::json!({
            "asset_id": full.asset_id,
            "media_type": "Png",
            "pixel_size": [4, 3],
        });
        let dec_empty: ProjectImageAsset = serde_json::from_value(dehydrated).unwrap();
        assert!(dec_empty.bytes.is_empty());
    }
}
