//! Publication PDF 1.4+ document writer emitting compressed vector geometry, Core-14
//! selectable scientific typography (WinAnsi + Adobe Symbol), optional glyph outlines,
//! alpha transparency states, sRGB/CMYK/Grayscale colour spaces, and multi-page layouts.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as FmtWrite;
use std::io::Write as IoWrite;

use curvebook_chart::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, SceneRasterImage, StrokePaint,
    TextAnchor,
};
use curvebook_doc::{MAX_IMAGE_DIM_PX, MAX_IMAGE_PIXELS};
use curvebook_palette::Rgba8;
use curvebook_text::glyph_paths::outline_visual_rich_text_rotated_with_fonts_and_ttf;
use curvebook_text::metrics::{core14_advance_em, is_combining_mark, reorder_rich_text_bidi, rich_contains_rtl, split_rich_text_lines};
use curvebook_text::{
    BUNDLED_FONTS, GlyphOutlineCmd, PositionedGlyphOutline, RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, SystemFontSet, TrueTypeFont,
    TrueTypeFontInfo,
};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};

pub use crate::{PdfColorSpace, PdfVersion};

/// Export options controlling PDF specification version, colour space, optional TrueType font
/// embedding, text outline mode, trim/bleed boxes, stream compression, and document metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfExportOptions {
    pub color_space: PdfColorSpace,
    pub compress_streams: bool,
    pub title: Option<String>,
    pub creator: String,
    pub outline_text: bool,
    pub trim_pt: Option<RectPt>,
    pub bleed_pt: Option<RectPt>,
    #[serde(default)]
    pub version: PdfVersion,
    #[serde(default)]
    pub embedded_truetype_font: Option<Vec<u8>>,
}

impl Default for PdfExportOptions {
    fn default() -> Self {
        Self {
            color_space: PdfColorSpace::Srgb,
            compress_streams: true,
            title: None,
            creator: String::from("CurveBook"),
            outline_text: false,
            trim_pt: None,
            bleed_pt: None,
            version: PdfVersion::Pdf17,
            embedded_truetype_font: None,
        }
    }
}

/// Hard ceiling on pages encoded into a single multi-page PDF document.
pub const MAX_PDF_PAGES: usize = 64;

/// Hard ceiling on total scene primitives encoded across all pages of a PDF document.
pub const MAX_PDF_TOTAL_PRIMITIVES: usize = 500_000;

/// Hard ceiling on emitted PDF byte length (32 MiB).
pub const MAX_PDF_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

/// Errors produced while encoding a [`ChartScene`] into PDF bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PdfError {
    EmptyDocument,
    InvalidScene { message: String },
    ResourceLimit { message: String },
    CompressionFailed { message: String },
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyDocument => f.write_str("cannot write an empty PDF document without pages"),
            Self::InvalidScene { message } => write!(f, "invalid scene for PDF export: {message}"),
            Self::ResourceLimit { message } => write!(f, "PDF export resource limit exceeded: {message}"),
            Self::CompressionFailed { message } => write!(f, "PDF stream compression failed: {message}"),
        }
    }
}

impl std::error::Error for PdfError {}

/// Render a single [`ChartScene`] to a standalone single-page PDF byte stream.
///
/// # Errors
/// Returns [`PdfError`] if scene dimensions or geometry are invalid, limits are exceeded, or stream compression fails.
pub fn render_pdf(scene: &ChartScene, options: &PdfExportOptions) -> Result<Vec<u8>, PdfError> {
    render_pdf_page_refs(&[scene], options)
}

/// Render one or more [`ChartScene`] pages into a multi-page PDF byte stream.
///
/// # Errors
/// Returns [`PdfError::EmptyDocument`] when `pages` is empty, [`PdfError::ResourceLimit`] when page,
/// primitive, or output byte budgets are exceeded, [`PdfError::InvalidScene`] when any page or option
/// box has invalid geometry, or [`PdfError::CompressionFailed`] if zlib fails.
pub fn render_pdf_pages(pages: &[ChartScene], options: &PdfExportOptions) -> Result<Vec<u8>, PdfError> {
    let refs: Vec<&ChartScene> = pages.iter().collect();
    render_pdf_page_refs(&refs, options)
}

/// Render one or more referenced [`ChartScene`] pages into a multi-page PDF byte stream without
/// duplicating scene storage for repeated pages.
///
/// # Errors
/// Returns [`PdfError`] if `pages` is empty, exceeds [`MAX_PDF_PAGES`], [`MAX_PDF_TOTAL_PRIMITIVES`],
/// or [`MAX_PDF_OUTPUT_BYTES`], contains invalid geometry, or fails stream compression.
pub fn render_pdf_page_refs(pages: &[&ChartScene], options: &PdfExportOptions) -> Result<Vec<u8>, PdfError> {
    if pages.is_empty() {
        return Err(PdfError::EmptyDocument);
    }
    if pages.len() > MAX_PDF_PAGES {
        return Err(PdfError::ResourceLimit { message: format!("PDF page count {} exceeds maximum {MAX_PDF_PAGES}", pages.len()) });
    }

    let mut total_primitives = 0_usize;
    for scene in pages {
        total_primitives = total_primitives
            .checked_add(scene.primitives.len())
            .ok_or_else(|| PdfError::ResourceLimit { message: format!("PDF scene primitive count overflows budget {MAX_PDF_TOTAL_PRIMITIVES}") })?;
        if total_primitives > MAX_PDF_TOTAL_PRIMITIVES {
            return Err(PdfError::ResourceLimit {
                message: format!("PDF scene primitive count {total_primitives} exceeds maximum {MAX_PDF_TOTAL_PRIMITIVES}"),
            });
        }
    }

    validate_box_opt(options.trim_pt, "trim_pt")?;
    validate_box_opt(options.bleed_pt, "bleed_pt")?;

    let embedded_ttf = resolve_embedded_truetype(options, !options.outline_text)?;
    let embedded_info = embedded_ttf.as_ref().map(|ttf| &ttf.info);
    let parsed_ttf = embedded_ttf.as_ref().map(|ttf| &ttf.parsed);
    let mut cumulative_stream_bytes = 0_usize;
    let mut doc_ttf_charged = false;

    let mut encoded_pages = Vec::with_capacity(pages.len());
    let mut all_gs_both = BTreeSet::new();
    let mut all_gs_stroke = BTreeSet::new();
    let mut image_registry = PdfImageRegistry::default();

    for scene in pages {
        validate_scene_bounds(scene)?;
        let (raw_stream, used) = encode_page_stream(scene, options, embedded_info, parsed_ttf, &mut image_registry, &mut cumulative_stream_bytes)?;
        if !options.outline_text && used.ttf && !doc_ttf_charged {
            let font_bytes = embedded_ttf.as_ref().map_or(0_usize, |t| t.compressed_bytes.len());
            cumulative_stream_bytes = cumulative_stream_bytes
                .checked_add(font_bytes)
                .ok_or_else(|| PdfError::ResourceLimit { message: format!("embedded PDF font exceeds output byte limit {MAX_PDF_OUTPUT_BYTES}") })?;
            if cumulative_stream_bytes > MAX_PDF_OUTPUT_BYTES {
                return Err(PdfError::ResourceLimit { message: format!("embedded PDF font exceeds output byte limit {MAX_PDF_OUTPUT_BYTES}") });
            }
            doc_ttf_charged = true;
        }
        all_gs_both.extend(used.gs_both.iter().copied());
        all_gs_stroke.extend(used.gs_stroke.iter().copied());
        let stream_bytes = compress_if_requested(raw_stream.into_bytes(), options.compress_streams)?;
        cumulative_stream_bytes = cumulative_stream_bytes
            .checked_add(stream_bytes.len())
            .ok_or_else(|| PdfError::ResourceLimit { message: format!("PDF stream byte count overflows limit {MAX_PDF_OUTPUT_BYTES}") })?;
        if cumulative_stream_bytes > MAX_PDF_OUTPUT_BYTES {
            return Err(PdfError::ResourceLimit { message: format!("PDF encoded content size exceeds maximum {MAX_PDF_OUTPUT_BYTES} bytes") });
        }
        encoded_pages.push(EncodedPage { stream_bytes, used });
    }

    let doc_embedded_ttf = if doc_ttf_charged { embedded_ttf.as_ref() } else { None };
    let doc_resources =
        PdfDocResources { all_gs_both: &all_gs_both, all_gs_stroke: &all_gs_stroke, embedded_ttf: doc_embedded_ttf, images: &image_registry };
    assemble_pdf_document(pages, &encoded_pages, &doc_resources, options)
}

struct EmbeddedTrueTypeFont<'a> {
    raw_len: usize,
    compressed_bytes: Vec<u8>,
    parsed: TrueTypeFont<'a>,
    info: TrueTypeFontInfo,
    ps_name: String,
}

struct ExtGStateIds {
    both: BTreeMap<u8, usize>,
    stroke: BTreeMap<u8, usize>,
}

fn is_truetype_sfnt(bytes: &[u8]) -> bool {
    bytes.get(0..4).is_some_and(|tag| tag == [0x00, 0x01, 0x00, 0x00] || tag == b"true")
}

fn sanitize_pdf_font_name(primary: &str, fallback: &str) -> String {
    let pick = |raw: &str| -> String {
        raw.chars().filter(|&c| c.is_ascii_graphic() && !matches!(c, '(' | ')' | '<' | '>' | '[' | ']' | '{' | '}' | '/' | '%' | '#')).collect()
    };
    let clean = pick(primary);
    if !clean.is_empty() {
        return clean;
    }
    let fb = pick(fallback);
    if !fb.is_empty() { fb } else { String::from("CurveBookEmbedded") }
}

fn parse_embedded_sfnt(bytes: &[u8]) -> Option<(&[u8], TrueTypeFont<'_>, TrueTypeFontInfo)> {
    if !is_truetype_sfnt(bytes) {
        return None;
    }
    let parsed = TrueTypeFont::parse_checked(bytes).ok()?;
    let info = parsed.inspect_info().ok()?;
    Some((bytes, parsed, info))
}

fn resolve_embedded_truetype(options: &PdfExportOptions, compress_for_embedding: bool) -> Result<Option<EmbeddedTrueTypeFont<'_>>, PdfError> {
    let candidate = if let Some(bytes) = options.embedded_truetype_font.as_deref() {
        parse_embedded_sfnt(bytes)
    } else {
        BUNDLED_FONTS.iter().find_map(|&(_, bytes)| parse_embedded_sfnt(bytes))
    };

    let Some((raw_bytes, parsed, info)) = candidate else {
        return Ok(None);
    };
    let ps_name = sanitize_pdf_font_name(&info.postscript_name, &info.family_name);
    let compressed_bytes = if compress_for_embedding { compress_if_requested(raw_bytes.to_vec(), true)? } else { Vec::new() };
    Ok(Some(EmbeddedTrueTypeFont { raw_len: raw_bytes.len(), compressed_bytes, parsed, info, ps_name }))
}

#[derive(Default)]
struct PageResourcesUsed {
    fonts: BTreeSet<PdfFontId>,
    gs_both: BTreeSet<u8>,
    gs_stroke: BTreeSet<u8>,
    images: BTreeSet<usize>,
    ttf: bool,
}

struct EncodedPage {
    stream_bytes: Vec<u8>,
    used: PageResourcesUsed,
}

struct PdfImageEntry {
    width: u32,
    height: u32,
    color_space: PdfColorSpace,
    color_stream: Vec<u8>,
    smask_stream: Option<Vec<u8>>,
}

#[derive(Default)]
struct PdfImageRegistry {
    index_by_key: BTreeMap<(String, u32, u32, u64), usize>,
    entries: Vec<PdfImageEntry>,
}

struct PdfDocResources<'a> {
    all_gs_both: &'a BTreeSet<u8>,
    all_gs_stroke: &'a BTreeSet<u8>,
    embedded_ttf: Option<&'a EmbeddedTrueTypeFont<'a>>,
    images: &'a PdfImageRegistry,
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn convert_raster_chunk(pixels: &[[u8; 4]], color_space: PdfColorSpace, color_out: &mut Vec<u8>, mut alpha_out: Option<&mut Vec<u8>>) {
    for &[r, g, b, a] in pixels {
        match color_space {
            PdfColorSpace::Srgb => {
                color_out.extend_from_slice(&[r, g, b]);
            }
            PdfColorSpace::Grayscale => {
                let luma = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
                color_out.push(luma.round().clamp(0.0, 255.0) as u8);
            }
            PdfColorSpace::Cmyk => {
                let cmyk = Rgba8::rgb(r, g, b).to_cmyk();
                color_out.push((cmyk.c * 255.0).round().clamp(0.0, 255.0) as u8);
                color_out.push((cmyk.m * 255.0).round().clamp(0.0, 255.0) as u8);
                color_out.push((cmyk.y * 255.0).round().clamp(0.0, 255.0) as u8);
                color_out.push((cmyk.k * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
        if let Some(alpha) = alpha_out.as_deref_mut() {
            alpha.push(a);
        }
    }
}

fn encode_raster_streams(
    rgba: &[u8],
    color_space: PdfColorSpace,
    compress: bool,
    raw_color_bytes: usize,
    raw_alpha_bytes: usize,
    has_alpha: bool,
    cumulative_stream_bytes: usize,
) -> Result<(Vec<u8>, Option<Vec<u8>>), PdfError> {
    let pixels = rgba.as_chunks::<4>().0;
    if !compress {
        let mut color_raw = Vec::with_capacity(raw_color_bytes);
        let mut alpha_raw = if has_alpha { Vec::with_capacity(raw_alpha_bytes) } else { Vec::new() };
        convert_raster_chunk(pixels, color_space, &mut color_raw, has_alpha.then_some(&mut alpha_raw));
        return Ok((color_raw, has_alpha.then_some(alpha_raw)));
    }

    const CHUNK_PIXELS: usize = 65_536;
    let color_channels = match color_space {
        PdfColorSpace::Srgb => 3_usize,
        PdfColorSpace::Grayscale => 1_usize,
        PdfColorSpace::Cmyk => 4_usize,
    };
    let mut color_encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    let mut alpha_encoder = has_alpha.then(|| ZlibEncoder::new(Vec::new(), Compression::default()));
    let mut color_chunk = Vec::with_capacity(CHUNK_PIXELS * color_channels);
    let mut alpha_chunk = if has_alpha { Vec::with_capacity(CHUNK_PIXELS) } else { Vec::new() };

    for batch in pixels.chunks(CHUNK_PIXELS) {
        color_chunk.clear();
        alpha_chunk.clear();
        convert_raster_chunk(batch, color_space, &mut color_chunk, has_alpha.then_some(&mut alpha_chunk));
        color_encoder.write_all(&color_chunk).map_err(|err| PdfError::CompressionFailed { message: err.to_string() })?;
        if let Some(enc) = alpha_encoder.as_mut() {
            enc.write_all(&alpha_chunk).map_err(|err| PdfError::CompressionFailed { message: err.to_string() })?;
        }
        let partial = color_encoder.get_ref().len().saturating_add(alpha_encoder.as_ref().map_or(0, |e| e.get_ref().len()));
        if cumulative_stream_bytes.saturating_add(partial) > MAX_PDF_OUTPUT_BYTES {
            return Err(PdfError::ResourceLimit { message: format!("PDF embedded image size exceeds maximum {MAX_PDF_OUTPUT_BYTES} bytes") });
        }
    }

    let color_stream = color_encoder.finish().map_err(|err| PdfError::CompressionFailed { message: err.to_string() })?;
    let smask_stream = alpha_encoder.map(|enc| enc.finish().map_err(|err| PdfError::CompressionFailed { message: err.to_string() })).transpose()?;
    Ok((color_stream, smask_stream))
}

impl PdfImageRegistry {
    fn register(&mut self, raster: &SceneRasterImage, options: &PdfExportOptions, cumulative_stream_bytes: &mut usize) -> Result<usize, PdfError> {
        let (w, h) = raster.pixel_size;
        if w == 0 || h == 0 {
            return Err(PdfError::InvalidScene { message: String::from("invalid raster image buffer for PDF XObject") });
        }
        if w > MAX_IMAGE_DIM_PX || h > MAX_IMAGE_DIM_PX {
            return Err(PdfError::ResourceLimit { message: format!("PDF raster image dimension {w}x{h} exceeds maximum {MAX_IMAGE_DIM_PX}px") });
        }
        let px_count_u64 = u64::from(w) * u64::from(h);
        if px_count_u64 > MAX_IMAGE_PIXELS {
            return Err(PdfError::ResourceLimit {
                message: format!("PDF raster image pixel count {px_count_u64} exceeds maximum {MAX_IMAGE_PIXELS}"),
            });
        }
        let px_count = (w as usize)
            .checked_mul(h as usize)
            .ok_or_else(|| PdfError::InvalidScene { message: String::from("PDF image dimensions overflow byte length") })?;
        let expected_len =
            px_count.checked_mul(4).ok_or_else(|| PdfError::InvalidScene { message: String::from("PDF image dimensions overflow byte length") })?;
        if raster.rgba.len() != expected_len {
            return Err(PdfError::InvalidScene { message: String::from("invalid raster image buffer for PDF XObject") });
        }
        let key = (raster.asset_id.clone(), w, h, fnv1a_64(&raster.rgba));
        if let Some(&idx) = self.index_by_key.get(&key) {
            return Ok(idx);
        }

        let has_alpha = raster.rgba.as_chunks::<4>().0.iter().any(|&[_, _, _, a]| a < 255);
        let color_channels = match options.color_space {
            PdfColorSpace::Srgb => 3_usize,
            PdfColorSpace::Grayscale => 1_usize,
            PdfColorSpace::Cmyk => 4_usize,
        };
        let raw_color_bytes = px_count
            .checked_mul(color_channels)
            .ok_or_else(|| PdfError::ResourceLimit { message: format!("PDF image color buffer overflows limit {MAX_PDF_OUTPUT_BYTES}") })?;
        let raw_alpha_bytes = if has_alpha { px_count } else { 0_usize };
        let raw_total_bytes = raw_color_bytes
            .checked_add(raw_alpha_bytes)
            .ok_or_else(|| PdfError::ResourceLimit { message: format!("PDF image byte count overflows limit {MAX_PDF_OUTPUT_BYTES}") })?;
        if !options.compress_streams {
            let projected = cumulative_stream_bytes
                .checked_add(raw_total_bytes)
                .ok_or_else(|| PdfError::ResourceLimit { message: format!("PDF image stream byte count overflows limit {MAX_PDF_OUTPUT_BYTES}") })?;
            if projected > MAX_PDF_OUTPUT_BYTES {
                return Err(PdfError::ResourceLimit {
                    message: format!("PDF uncompressed image stream size exceeds maximum {MAX_PDF_OUTPUT_BYTES} bytes"),
                });
            }
        }

        let (color_stream, smask_stream) = encode_raster_streams(
            &raster.rgba,
            options.color_space,
            options.compress_streams,
            raw_color_bytes,
            raw_alpha_bytes,
            has_alpha,
            *cumulative_stream_bytes,
        )?;
        let added = color_stream.len().saturating_add(smask_stream.as_ref().map_or(0, Vec::len));
        *cumulative_stream_bytes = cumulative_stream_bytes
            .checked_add(added)
            .ok_or_else(|| PdfError::ResourceLimit { message: format!("PDF image stream byte count overflows limit {MAX_PDF_OUTPUT_BYTES}") })?;
        if *cumulative_stream_bytes > MAX_PDF_OUTPUT_BYTES {
            return Err(PdfError::ResourceLimit { message: format!("PDF embedded image size exceeds maximum {MAX_PDF_OUTPUT_BYTES} bytes") });
        }

        let idx = self.entries.len();
        self.entries.push(PdfImageEntry { width: w, height: h, color_space: options.color_space, color_stream, smask_stream });
        self.index_by_key.insert(key, idx);
        Ok(idx)
    }
}

fn validate_scene_bounds(scene: &ChartScene) -> Result<(), PdfError> {
    if !scene.width_pt.is_finite() || !scene.height_pt.is_finite() || scene.width_pt <= 0.0 || scene.height_pt <= 0.0 {
        return Err(PdfError::InvalidScene { message: String::from("PDF page dimensions must be finite and positive") });
    }
    Ok(())
}

fn validate_box_opt(box_opt: Option<RectPt>, label: &str) -> Result<(), PdfError> {
    if let Some(r) = box_opt
        && (!r.x.is_finite() || !r.y.is_finite() || !r.width.is_finite() || !r.height.is_finite() || r.width <= 0.0 || r.height <= 0.0)
    {
        return Err(PdfError::InvalidScene { message: format!("PDF {label} box must have finite positive dimensions") });
    }
    Ok(())
}

fn compress_if_requested(raw: Vec<u8>, compress: bool) -> Result<Vec<u8>, PdfError> {
    if !compress {
        return Ok(raw);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw).map_err(|err| PdfError::CompressionFailed { message: err.to_string() })?;
    encoder.finish().map_err(|err| PdfError::CompressionFailed { message: err.to_string() })
}

fn assemble_pdf_document(
    pages: &[&ChartScene],
    encoded_pages: &[EncodedPage],
    res: &PdfDocResources<'_>,
    options: &PdfExportOptions,
) -> Result<Vec<u8>, PdfError> {
    // Fixed objects:
    // 1: Catalog, 2: Pages, 3: /F1 Helvetica, 4: /F2 Helvetica-Bold,
    // 5: /F3 Helvetica-Oblique, 6: /F4 Symbol, 7: /F5 Helvetica-BoldOblique, 8: Info
    let fixed_objs = 8usize;
    let mut next_id = fixed_objs + 1;

    let ttf_ids = res.embedded_ttf.map(|ttf| {
        let font_id = next_id;
        let desc_id = next_id + 1;
        let stream_id = next_id + 2;
        next_id += 3;
        (ttf, font_id, desc_id, stream_id)
    });

    let mut gs_both_ids = BTreeMap::new();
    for &alpha in res.all_gs_both {
        gs_both_ids.insert(alpha, next_id);
        next_id += 1;
    }
    let mut gs_stroke_ids = BTreeMap::new();
    for &alpha in res.all_gs_stroke {
        gs_stroke_ids.insert(alpha, next_id);
        next_id += 1;
    }
    let gs_ids = ExtGStateIds { both: gs_both_ids, stroke: gs_stroke_ids };

    let mut image_obj_ids = Vec::with_capacity(res.images.entries.len());
    for entry in &res.images.entries {
        let im_id = next_id;
        next_id += 1;
        let smask_id = entry.smask_stream.as_ref().map(|_| {
            let sid = next_id;
            next_id += 1;
            sid
        });
        image_obj_ids.push((im_id, smask_id));
    }

    let pages_base_id = next_id;
    let total_objs = pages_base_id - 1 + pages.len() * 2;
    let mut offsets = vec![0usize; total_objs + 1];

    let mut pdf = Vec::with_capacity(8192 + res.embedded_ttf.map_or(0, |t| t.compressed_bytes.len()));
    let header: &[u8] = match options.version {
        PdfVersion::Pdf14 => b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n",
        PdfVersion::Pdf17 => b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n",
    };
    pdf.extend_from_slice(header);

    let mut page_refs = String::new();
    for idx in 0..pages.len() {
        let page_obj_id = pages_base_id + idx * 2 + 1;
        let _ = write!(page_refs, "{page_obj_id} 0 R ");
    }

    write_pdf_obj(&mut pdf, &mut offsets, 1, b"<< /Type /Catalog /Pages 2 0 R >>\n");
    let pages_dict = format!("<< /Type /Pages /Count {} /Kids [ {}] >>\n", pages.len(), page_refs);
    write_pdf_obj(&mut pdf, &mut offsets, 2, pages_dict.as_bytes());

    write_pdf_obj(&mut pdf, &mut offsets, 3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\n");
    write_pdf_obj(&mut pdf, &mut offsets, 4, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>\n");
    write_pdf_obj(&mut pdf, &mut offsets, 5, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique /Encoding /WinAnsiEncoding >>\n");
    write_pdf_obj(&mut pdf, &mut offsets, 6, b"<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>\n");
    write_pdf_obj(&mut pdf, &mut offsets, 7, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-BoldOblique /Encoding /WinAnsiEncoding >>\n");

    let info_dict = build_info_dict(pages, options);
    write_pdf_obj(&mut pdf, &mut offsets, 8, info_dict.as_bytes());

    if let Some((ttf, font_id, desc_id, stream_id)) = ttf_ids {
        write_truetype_font_objects(&mut pdf, &mut offsets, ttf, font_id, desc_id, stream_id);
    }

    for (&alpha, &obj_id) in &gs_ids.both {
        let a = norm_zero(f64::from(alpha) / 255.0);
        let gs = format!("<< /Type /ExtGState /ca {a:.3} /CA {a:.3} >>\n");
        write_pdf_obj(&mut pdf, &mut offsets, obj_id, gs.as_bytes());
    }
    for (&alpha, &obj_id) in &gs_ids.stroke {
        let a = norm_zero(f64::from(alpha) / 255.0);
        let gs = format!("<< /Type /ExtGState /CA {a:.3} >>\n");
        write_pdf_obj(&mut pdf, &mut offsets, obj_id, gs.as_bytes());
    }

    let filter_tag = if options.compress_streams { " /Filter /FlateDecode" } else { "" };
    for (entry, &(im_id, smask_id)) in res.images.entries.iter().zip(image_obj_ids.iter()) {
        write_pdf_image_objects(&mut pdf, &mut offsets, entry, im_id, smask_id, filter_tag);
    }

    let ttf_font_id = ttf_ids.map(|(_, font_id, _, _)| font_id);
    for (idx, (&scene, encoded)) in pages.iter().zip(encoded_pages.iter()).enumerate() {
        let content_obj_id = pages_base_id + idx * 2;
        let page_obj_id = content_obj_id + 1;

        let mut content_obj = Vec::with_capacity(encoded.stream_bytes.len() + 96);
        let header = format!("<< /Length {}{} >>\nstream\n", encoded.stream_bytes.len(), filter_tag);
        content_obj.extend_from_slice(header.as_bytes());
        content_obj.extend_from_slice(&encoded.stream_bytes);
        content_obj.extend_from_slice(b"\nendstream\n");
        write_pdf_obj(&mut pdf, &mut offsets, content_obj_id, &content_obj);

        let page_dict = build_page_dict(scene, options, encoded, content_obj_id, ttf_font_id, &gs_ids, &image_obj_ids);
        write_pdf_obj(&mut pdf, &mut offsets, page_obj_id, page_dict.as_bytes());
    }

    write_xref_and_trailer(&mut pdf, &offsets, total_objs);
    if pdf.len() > MAX_PDF_OUTPUT_BYTES {
        return Err(PdfError::ResourceLimit {
            message: format!("PDF output length {} bytes exceeds maximum {MAX_PDF_OUTPUT_BYTES} bytes", pdf.len()),
        });
    }
    Ok(pdf)
}

fn write_truetype_font_objects(
    pdf: &mut Vec<u8>,
    offsets: &mut [usize],
    ttf: &EmbeddedTrueTypeFont<'_>,
    font_id: usize,
    desc_id: usize,
    stream_id: usize,
) {
    let mut widths = String::with_capacity(224 * 5);
    for idx in 0..224usize {
        if idx > 0 {
            widths.push(' ');
        }
        let w = ttf.info.widths_1000.get(idx).copied().unwrap_or(556);
        let _ = write!(widths, "{w}");
    }
    let font_dict = format!(
        "<< /Type /Font /Subtype /TrueType /BaseFont /{} /FirstChar 32 /LastChar 255 /Widths [{}] /FontDescriptor {} 0 R /Encoding /WinAnsiEncoding >>\n",
        ttf.ps_name, widths, desc_id
    );
    write_pdf_obj(pdf, offsets, font_id, font_dict.as_bytes());

    let flags = if ttf.info.flags == 0 { 32 } else { ttf.info.flags };
    let [b0, b1, b2, b3] = ttf.info.bbox_1000;
    let italic_str = if ttf.info.italic_angle.is_finite() && ttf.info.italic_angle.abs() >= 0.05 {
        format!("{:.2}", ttf.info.italic_angle)
    } else {
        String::from("0")
    };
    let desc_dict = format!(
        "<< /Type /FontDescriptor /FontName /{} /Flags {} /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV {} /FontFile2 {} 0 R >>\n",
        ttf.ps_name,
        flags,
        b0,
        b1,
        b2,
        b3,
        italic_str,
        ttf.info.ascender_1000,
        ttf.info.descender_1000,
        ttf.info.cap_height_1000,
        ttf.info.stem_v,
        stream_id
    );
    write_pdf_obj(pdf, offsets, desc_id, desc_dict.as_bytes());

    let mut stream_obj = Vec::with_capacity(ttf.compressed_bytes.len() + 128);
    let header = format!("<< /Length {} /Length1 {} /Filter /FlateDecode >>\nstream\n", ttf.compressed_bytes.len(), ttf.raw_len);
    stream_obj.extend_from_slice(header.as_bytes());
    stream_obj.extend_from_slice(&ttf.compressed_bytes);
    stream_obj.extend_from_slice(b"\nendstream\n");
    write_pdf_obj(pdf, offsets, stream_id, &stream_obj);
}

fn build_info_dict(pages: &[&ChartScene], options: &PdfExportOptions) -> String {
    let title = options
        .title
        .as_deref()
        .filter(|t| !t.is_empty())
        .or_else(|| (pages.len() == 1).then(|| pages.first().map(|p| p.title.as_str()).unwrap_or("")).filter(|t| !t.is_empty()));
    let creator = if options.creator.is_empty() { "CurveBook" } else { options.creator.as_str() };
    let mut dict = String::from("<< ");
    if let Some(t) = title {
        let _ = write!(dict, "/Title {} ", format_pdf_info_string(t));
    }
    let _ = writeln!(dict, "/Creator {} /Producer (CurveBook) >>", format_pdf_info_string(creator));
    dict
}

fn write_pdf_image_objects(pdf: &mut Vec<u8>, offsets: &mut [usize], entry: &PdfImageEntry, im_id: usize, smask_id: Option<usize>, filter_tag: &str) {
    let cs_name = match entry.color_space {
        PdfColorSpace::Srgb => "DeviceRGB",
        PdfColorSpace::Grayscale => "DeviceGray",
        PdfColorSpace::Cmyk => "DeviceCMYK",
    };
    let smask_tag = match smask_id {
        Some(sid) => format!(" /SMask {sid} 0 R"),
        None => String::new(),
    };
    let mut img_obj = Vec::with_capacity(entry.color_stream.len() + 192);
    let header = format!(
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{cs_name} /BitsPerComponent 8{filter_tag}{smask_tag} /Length {} >>\nstream\n",
        entry.width,
        entry.height,
        entry.color_stream.len()
    );
    img_obj.extend_from_slice(header.as_bytes());
    img_obj.extend_from_slice(&entry.color_stream);
    img_obj.extend_from_slice(b"\nendstream\n");
    write_pdf_obj(pdf, offsets, im_id, &img_obj);

    if let (Some(sid), Some(smask_bytes)) = (smask_id, entry.smask_stream.as_ref()) {
        let mut smask_obj = Vec::with_capacity(smask_bytes.len() + 160);
        let sheader = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8{filter_tag} /Length {} >>\nstream\n",
            entry.width,
            entry.height,
            smask_bytes.len()
        );
        smask_obj.extend_from_slice(sheader.as_bytes());
        smask_obj.extend_from_slice(smask_bytes);
        smask_obj.extend_from_slice(b"\nendstream\n");
        write_pdf_obj(pdf, offsets, sid, &smask_obj);
    }
}

fn build_page_dict(
    scene: &ChartScene,
    options: &PdfExportOptions,
    encoded: &EncodedPage,
    content_obj_id: usize,
    ttf_font_id: Option<usize>,
    gs_ids: &ExtGStateIds,
    image_obj_ids: &[(usize, Option<usize>)],
) -> String {
    let mut dict = String::with_capacity(280);
    let _ = write!(dict, "<< /Type /Page /Parent 2 0 R /MediaBox [0.000 0.000 {:.3} {:.3}]", norm_zero(scene.width_pt), norm_zero(scene.height_pt));
    if let Some(trim) = options.trim_pt {
        let [llx, lly, urx, ury] = rect_to_pdf_box(trim, scene.height_pt);
        let _ = write!(dict, " /TrimBox [{llx:.3} {lly:.3} {urx:.3} {ury:.3}]");
    }
    if let Some(bleed) = options.bleed_pt {
        let [llx, lly, urx, ury] = rect_to_pdf_box(bleed, scene.height_pt);
        let _ = write!(dict, " /BleedBox [{llx:.3} {lly:.3} {urx:.3} {ury:.3}]");
    }
    dict.push_str(" /Resources <<");
    if !encoded.used.fonts.is_empty() {
        dict.push_str(" /Font <<");
        for &font in &encoded.used.fonts {
            let obj_id = match font {
                PdfFontId::Helvetica => Some(3),
                PdfFontId::HelveticaBold => Some(4),
                PdfFontId::HelveticaOblique => Some(5),
                PdfFontId::Symbol => Some(6),
                PdfFontId::HelveticaBoldOblique => Some(7),
                PdfFontId::EmbeddedTrueType => ttf_font_id,
            };
            if let Some(id) = obj_id {
                let _ = write!(dict, " /{} {id} 0 R", font.tag());
            }
        }
        dict.push_str(" >>");
    }
    if !encoded.used.gs_both.is_empty() || !encoded.used.gs_stroke.is_empty() {
        dict.push_str(" /ExtGState <<");
        for &alpha in &encoded.used.gs_both {
            if let Some(&obj_id) = gs_ids.both.get(&alpha) {
                let _ = write!(dict, " /GSa{alpha:03} {obj_id} 0 R");
            }
        }
        for &alpha in &encoded.used.gs_stroke {
            if let Some(&obj_id) = gs_ids.stroke.get(&alpha) {
                let _ = write!(dict, " /GSs{alpha:03} {obj_id} 0 R");
            }
        }
        dict.push_str(" >>");
    }
    if !encoded.used.images.is_empty() {
        dict.push_str(" /XObject <<");
        for &im_idx in &encoded.used.images {
            if let Some(&(obj_id, _)) = image_obj_ids.get(im_idx) {
                let _ = write!(dict, " /Im{im_idx} {obj_id} 0 R");
            }
        }
        dict.push_str(" >>");
    }
    let _ = writeln!(dict, " >> /Contents {content_obj_id} 0 R >>");
    dict
}

fn write_xref_and_trailer(pdf: &mut Vec<u8>, offsets: &[usize], total_objs: usize) {
    let xref_offset = pdf.len();
    let _ = writeln!(pdf_writer(pdf), "xref");
    let _ = writeln!(pdf_writer(pdf), "0 {}", total_objs + 1);
    let _ = writeln!(pdf_writer(pdf), "0000000000 65535 f ");
    for offset in offsets.iter().skip(1) {
        let _ = writeln!(pdf_writer(pdf), "{offset:010} 00000 n ");
    }
    let _ = writeln!(pdf_writer(pdf), "trailer");
    let _ = writeln!(pdf_writer(pdf), "<< /Size {} /Root 1 0 R /Info 8 0 R >>", total_objs + 1);
    let _ = writeln!(pdf_writer(pdf), "startxref");
    let _ = writeln!(pdf_writer(pdf), "{xref_offset}");
    pdf.extend_from_slice(b"%%EOF\n");
}

fn write_pdf_obj(pdf: &mut Vec<u8>, offsets: &mut [usize], obj_id: usize, body: &[u8]) {
    if let Some(slot) = offsets.get_mut(obj_id) {
        *slot = pdf.len();
    }
    let header = format!("{obj_id} 0 obj\n");
    pdf.extend_from_slice(header.as_bytes());
    pdf.extend_from_slice(body);
    pdf.extend_from_slice(b"endobj\n");
}

fn encode_page_stream<'a>(
    scene: &ChartScene,
    options: &'a PdfExportOptions,
    embedded_info: Option<&'a TrueTypeFontInfo>,
    embedded_ttf: Option<&'a TrueTypeFont<'a>>,
    images: &'a mut PdfImageRegistry,
    cumulative_stream_bytes: &'a mut usize,
) -> Result<(String, PageResourcesUsed), PdfError> {
    let mut ctx = StreamCtx {
        buf: String::with_capacity(1024),
        used: PageResourcesUsed::default(),
        page_h: scene.height_pt,
        color_space: options.color_space,
        outline_text: options.outline_text,
        options,
        embedded_info,
        embedded_ttf,
        images,
        cumulative_stream_bytes,
    };
    let mut clip_depth = 0usize;

    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Path { path, fill, stroke, clip } => {
                ctx.emit_path(path, *fill, *stroke, *clip)?;
            }
            ScenePrimitive::Text(text) => {
                ctx.emit_text(text)?;
            }
            ScenePrimitive::RasterImage(raster) => {
                ctx.emit_raster_image(raster)?;
            }
            ScenePrimitive::PushClip { rect } => {
                ctx.push_clip(*rect)?;
                clip_depth += 1;
            }
            ScenePrimitive::PopClip => {
                if clip_depth > 0 {
                    ctx.buf.push_str("Q\n");
                    clip_depth -= 1;
                }
            }
        }
    }

    while clip_depth > 0 {
        ctx.buf.push_str("Q\n");
        clip_depth -= 1;
    }

    Ok((ctx.buf, ctx.used))
}

struct StreamCtx<'a> {
    buf: String,
    used: PageResourcesUsed,
    page_h: f64,
    color_space: PdfColorSpace,
    outline_text: bool,
    options: &'a PdfExportOptions,
    embedded_info: Option<&'a TrueTypeFontInfo>,
    embedded_ttf: Option<&'a TrueTypeFont<'a>>,
    images: &'a mut PdfImageRegistry,
    cumulative_stream_bytes: &'a mut usize,
}

impl StreamCtx<'_> {
    fn y_pdf(&self, y_scene: f64) -> f64 {
        norm_zero(self.page_h - y_scene)
    }

    fn emit_raster_image(&mut self, raster: &SceneRasterImage) -> Result<(), PdfError> {
        let b = raster.bounds_pt;
        check_finite_rect(b, "raster image bounds")?;
        if b.width <= 0.0 || b.height <= 0.0 || !raster.rotation_deg.is_finite() {
            return Err(PdfError::InvalidScene { message: String::from("PDF raster image dimensions and rotation must be valid") });
        }
        let Some(vis_doc) = raster.crop.visible_rect(curvebook_doc::RectPt::from(b)) else {
            return Ok(());
        };
        let vis = RectPt::from(vis_doc);
        let im_idx = self.images.register(raster, self.options, self.cumulative_stream_bytes)?;
        self.used.images.insert(im_idx);

        self.buf.push_str("q\n");
        if let Some(c) = raster.clip {
            check_finite_rect(c, "raster image clip")?;
            let [llx, lly, _, _] = rect_to_pdf_box(c, self.page_h);
            let _ = writeln!(self.buf, "{llx:.3} {lly:.3} {:.3} {:.3} re W n", norm_zero(c.width.max(0.0)), norm_zero(c.height.max(0.0)));
        }

        if raster.rotation_deg.abs() > 1e-9 {
            let cx = norm_zero(b.x + 0.5 * b.width);
            let cy_pdf = self.y_pdf(b.y + 0.5 * b.height);
            let (sin_t, cos_t) = raster.rotation_deg.to_radians().sin_cos();
            let _ = writeln!(self.buf, "1 0 0 1 {cx:.3} {cy_pdf:.3} cm");
            let _ = writeln!(self.buf, "{cos_t:.6} {:.6} {sin_t:.6} {cos_t:.6} 0 0 cm", norm_tiny(-sin_t));
            let _ = writeln!(self.buf, "1 0 0 1 {:.3} {:.3} cm", norm_zero(-cx), norm_zero(-cy_pdf));
        }

        let is_cropped = raster.crop.left_pt > 0.0 || raster.crop.right_pt > 0.0 || raster.crop.top_pt > 0.0 || raster.crop.bottom_pt > 0.0;
        if is_cropped {
            let [vx, vy, _, _] = rect_to_pdf_box(vis, self.page_h);
            let _ = writeln!(self.buf, "{vx:.3} {vy:.3} {:.3} {:.3} re W n", norm_zero(vis.width), norm_zero(vis.height));
        }

        let [bx, by, _, _] = rect_to_pdf_box(b, self.page_h);
        let _ = writeln!(self.buf, "{:.3} 0 0 {:.3} {bx:.3} {by:.3} cm", norm_zero(b.width), norm_zero(b.height));
        let _ = writeln!(self.buf, "/Im{im_idx} Do");
        self.buf.push_str("Q\n");
        Ok(())
    }

    fn push_clip(&mut self, rect: RectPt) -> Result<(), PdfError> {
        check_finite_rect(rect, "clip rect")?;
        self.buf.push_str("q\n");
        let [llx, lly, _, _] = rect_to_pdf_box(rect, self.page_h);
        let w = norm_zero(rect.width.max(0.0));
        let h = norm_zero(rect.height.max(0.0));
        let _ = writeln!(self.buf, "{llx:.3} {lly:.3} {w:.3} {h:.3} re W n");
        Ok(())
    }

    fn emit_path(&mut self, path: &Path2D, fill: Option<FillPaint>, stroke: Option<StrokePaint>, clip: Option<RectPt>) -> Result<(), PdfError> {
        if path.is_empty() {
            return Ok(());
        }
        let active_fill = fill.filter(|f| f.color.a > 0);
        let active_stroke = stroke.filter(|s| s.color.a > 0 && s.width_pt.is_finite() && s.width_pt > 0.0);
        if active_fill.is_none() && active_stroke.is_none() {
            return Ok(());
        }

        self.buf.push_str("q\n");
        if let Some(r) = clip {
            check_finite_rect(r, "path clip rect")?;
            let [llx, lly, _, _] = rect_to_pdf_box(r, self.page_h);
            let w = norm_zero(r.width.max(0.0));
            let h = norm_zero(r.height.max(0.0));
            let _ = writeln!(self.buf, "{llx:.3} {lly:.3} {w:.3} {h:.3} re W n");
        }

        self.emit_path_alpha(active_fill, active_stroke);
        if let Some(f) = active_fill {
            let _ = writeln!(self.buf, "{}", format_fill_color(f.color, self.color_space));
        }
        if let Some(st) = active_stroke {
            self.emit_stroke_style(st);
        }
        self.emit_path_commands(path)?;

        match (active_fill.is_some(), active_stroke.is_some()) {
            (true, true) => self.buf.push_str("B\n"),
            (true, false) => self.buf.push_str("f\n"),
            (false, true) => self.buf.push_str("S\n"),
            (false, false) => {}
        }
        self.buf.push_str("Q\n");
        Ok(())
    }

    fn emit_path_alpha(&mut self, fill: Option<FillPaint>, stroke: Option<StrokePaint>) {
        match (fill, stroke) {
            (Some(f), Some(st)) => {
                if f.color.a < 255 {
                    self.used.gs_both.insert(f.color.a);
                    let _ = writeln!(self.buf, "/GSa{:03} gs", f.color.a);
                }
                if st.color.a != f.color.a {
                    self.used.gs_stroke.insert(st.color.a);
                    let _ = writeln!(self.buf, "/GSs{:03} gs", st.color.a);
                }
            }
            (Some(f), None) if f.color.a < 255 => {
                self.used.gs_both.insert(f.color.a);
                let _ = writeln!(self.buf, "/GSa{:03} gs", f.color.a);
            }
            (None, Some(st)) if st.color.a < 255 => {
                self.used.gs_both.insert(st.color.a);
                let _ = writeln!(self.buf, "/GSa{:03} gs", st.color.a);
            }
            _ => {}
        }
    }

    fn emit_stroke_style(&mut self, st: StrokePaint) {
        let cap_code = match st.cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::Square => 2,
        };
        let join_code = match st.join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        let _ =
            writeln!(self.buf, "{} {:.3} w {} J {} j", format_stroke_color(st.color, self.color_space), norm_zero(st.width_pt), cap_code, join_code);
        let mults = st.dash.dash_multipliers();
        if !mults.is_empty() {
            self.buf.push('[');
            for (idx, &m) in mults.iter().enumerate() {
                if idx > 0 {
                    self.buf.push(' ');
                }
                let dash_len = norm_zero((m * st.width_pt).max(0.1));
                let _ = write!(self.buf, "{dash_len:.3}");
            }
            self.buf.push_str("] 0 d\n");
        }
    }

    fn emit_path_commands(&mut self, path: &Path2D) -> Result<(), PdfError> {
        let mut cur = PointPt::new(0.0, 0.0);
        let mut start = cur;
        for cmd in &path.commands {
            match *cmd {
                PathCmd::MoveTo(pt) => {
                    check_finite_pt(pt, "MoveTo")?;
                    let _ = writeln!(self.buf, "{:.3} {:.3} m", norm_zero(pt.x), self.y_pdf(pt.y));
                    cur = pt;
                    start = pt;
                }
                PathCmd::LineTo(pt) => {
                    check_finite_pt(pt, "LineTo")?;
                    let _ = writeln!(self.buf, "{:.3} {:.3} l", norm_zero(pt.x), self.y_pdf(pt.y));
                    cur = pt;
                }
                PathCmd::QuadTo { control, to } => {
                    check_finite_pt(control, "QuadTo.control")?;
                    check_finite_pt(to, "QuadTo.to")?;
                    // Exact quadratic-to-cubic degree elevation:
                    // C1 = P0 + 2/3 (P1 - P0), C2 = P2 + 2/3 (P1 - P2).
                    let c1 = PointPt::new(cur.x + (2.0 / 3.0) * (control.x - cur.x), cur.y + (2.0 / 3.0) * (control.y - cur.y));
                    let c2 = PointPt::new(to.x + (2.0 / 3.0) * (control.x - to.x), to.y + (2.0 / 3.0) * (control.y - to.y));
                    let _ = writeln!(
                        self.buf,
                        "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c",
                        norm_zero(c1.x),
                        self.y_pdf(c1.y),
                        norm_zero(c2.x),
                        self.y_pdf(c2.y),
                        norm_zero(to.x),
                        self.y_pdf(to.y),
                    );
                    cur = to;
                }
                PathCmd::CubicTo { c1, c2, to } => {
                    check_finite_pt(c1, "CubicTo.c1")?;
                    check_finite_pt(c2, "CubicTo.c2")?;
                    check_finite_pt(to, "CubicTo.to")?;
                    let _ = writeln!(
                        self.buf,
                        "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c",
                        norm_zero(c1.x),
                        self.y_pdf(c1.y),
                        norm_zero(c2.x),
                        self.y_pdf(c2.y),
                        norm_zero(to.x),
                        self.y_pdf(to.y),
                    );
                    cur = to;
                }
                PathCmd::Close => {
                    self.buf.push_str("h\n");
                    cur = start;
                }
            }
        }
        Ok(())
    }

    fn emit_text(&mut self, text: &PositionedText) -> Result<(), PdfError> {
        check_finite_pt(text.anchor_pt, "Text.anchor_pt")?;
        if !text.font_size_pt.is_finite() || !text.rotation_deg.is_finite() {
            return Err(PdfError::InvalidScene { message: String::from("text font size and rotation must be finite") });
        }
        if text.font_size_pt <= 0.0 || text.rich.is_empty() {
            return Ok(());
        }
        let has_visible = text.rich.spans.iter().any(|s| !s.text.is_empty() && s.color.unwrap_or(text.color).a > 0);
        if !has_visible {
            return Ok(());
        }
        if self.outline_text { self.emit_outlined_text(text) } else { self.emit_selectable_text(text) }
    }

    fn emit_selectable_text(&mut self, text: &PositionedText) -> Result<(), PdfError> {
        let lines = split_rich_text_lines(&text.rich);
        let line_step = text.font_size_pt * 1.20;
        let rad = text.rotation_deg.to_radians();
        let (raw_sin, raw_cos) = rad.sin_cos();
        let mut state = SelectableTextState {
            anchor_pt: text.anchor_pt,
            base_color: text.color,
            font_size_pt: text.font_size_pt,
            cos_a: norm_tiny(raw_cos),
            sin_a: norm_tiny(raw_sin),
            active_alpha: if text.color.a > 0 { text.color.a } else { 255 },
            active_color: text.color,
            active_font: None,
            active_rise: 0.0,
            active_bold_stroke: false,
            active_bold_width: 0.0,
            active_stroke_color: None,
            active_italic_slant: false,
            pen_u: 0.0,
            line_v: 0.0,
            in_text_block: false,
        };

        for (line_idx, line) in lines.iter().enumerate() {
            let line_visible = line.spans.iter().any(|s| !s.text.is_empty() && s.color.unwrap_or(text.color).a > 0);
            if !line_visible {
                continue;
            }
            let visual_line: Cow<'_, RichText> = if rich_contains_rtl(line) { Cow::Owned(reorder_rich_text_bidi(line)) } else { Cow::Borrowed(line) };
            let line_width = measure_selectable_text_width_pt(&visual_line, text.font_size_pt, self.embedded_info, self.embedded_ttf);
            state.pen_u = anchor_offset_x(text.anchor, line_width);
            state.line_v = (line_idx as f64) * line_step;

            for span in &visual_line.spans {
                self.emit_selectable_span(span, &mut state)?;
            }

            self.close_selectable_block(&mut state);
        }
        Ok(())
    }

    fn open_selectable_block(&mut self, state: &mut SelectableTextState, color: Rgba8, italic_slant: bool) {
        if state.in_text_block {
            return;
        }
        // Emit page-level graphics state alpha outside `BT ... ET` per PDF 1.4 operator rules.
        self.buf.push_str("q\n");
        state.active_alpha = if color.a > 0 { color.a } else { 255 };
        if state.active_alpha < 255 {
            self.used.gs_both.insert(state.active_alpha);
            let _ = writeln!(self.buf, "/GSa{:03} gs", state.active_alpha);
        }
        let _ = writeln!(self.buf, "{}", format_fill_color(color, self.color_space));
        state.active_color = color;
        state.active_font = None;
        state.active_rise = 0.0;
        state.active_bold_stroke = false;
        state.active_bold_width = 0.0;
        state.active_stroke_color = None;
        state.active_italic_slant = italic_slant;
        self.buf.push_str("BT\n");
        self.emit_text_matrix(state.anchor_pt, state.pen_u, state.line_v, state.cos_a, state.sin_a, italic_slant);
        state.in_text_block = true;
    }

    fn close_selectable_block(&mut self, state: &mut SelectableTextState) {
        if !state.in_text_block {
            return;
        }
        if state.active_bold_stroke {
            self.buf.push_str("0 Tr\n");
            state.active_bold_stroke = false;
            state.active_bold_width = 0.0;
            state.active_stroke_color = None;
        }
        if state.active_rise.abs() > 1e-4 {
            self.buf.push_str("0.000 Ts\n");
            state.active_rise = 0.0;
        }
        self.buf.push_str("ET\nQ\n");
        state.in_text_block = false;
        state.active_font = None;
        state.active_italic_slant = false;
    }

    fn emit_selectable_span(&mut self, span: &RichSpan, state: &mut SelectableTextState) -> Result<(), PdfError> {
        if span.text.is_empty() {
            return Ok(());
        }
        let span_color = span.color.unwrap_or(state.base_color);
        if span_color.a == 0 {
            state.pen_u += measure_span_width_pt(span, state.font_size_pt, self.embedded_info, self.embedded_ttf);
            return Ok(());
        }

        let span_size = effective_span_font_size(state.font_size_pt, span);
        let span_rise = match span.script {
            SpanScript::Baseline => 0.0,
            SpanScript::Superscript => state.font_size_pt * 0.35,
            SpanScript::Subscript => -state.font_size_pt * 0.20,
        };
        let ttf_is_bold = self.embedded_info.is_some_and(|info| info.stem_v >= 118);
        let ttf_is_italic = self.embedded_info.is_some_and(|info| (info.flags & 64) != 0 || info.italic_angle.abs() >= 0.05);
        let want_bold_stroke = span.weight == SpanWeight::Bold && !ttf_is_bold;
        let want_italic_slant = span.slope == SpanSlope::Italic && !ttf_is_italic;

        for seg in split_span_into_pdf_segments(span, state.font_size_pt, self.embedded_info) {
            match seg {
                SpanPdfSegment::PdfText { font, bytes, width_pt } => {
                    let is_ttf = font == PdfFontId::EmbeddedTrueType;
                    let style = SpanPdfStyle {
                        color: span_color,
                        size_pt: span_size,
                        rise_pt: span_rise,
                        bold_stroke: is_ttf && want_bold_stroke,
                        italic_slant: is_ttf && want_italic_slant,
                    };
                    self.emit_pdf_text_segment(font, &bytes, style, state);
                    state.pen_u += width_pt;
                }
                SpanPdfSegment::Outlined(slice) => {
                    self.emit_outlined_segment(&slice, span, span_size, state)?;
                }
            }
        }
        Ok(())
    }

    fn emit_pdf_text_segment(&mut self, font: PdfFontId, bytes: &[u8], style: SpanPdfStyle, state: &mut SelectableTextState) {
        if bytes.is_empty() {
            return;
        }
        self.used.fonts.insert(font);
        if font == PdfFontId::EmbeddedTrueType {
            self.used.ttf = true;
        }
        if state.in_text_block && style.color.a != state.active_alpha {
            self.close_selectable_block(state);
        }
        if !state.in_text_block {
            self.open_selectable_block(state, style.color, style.italic_slant);
        } else {
            if style.color != state.active_color {
                let _ = writeln!(self.buf, "{}", format_fill_color(style.color, self.color_space));
                state.active_color = style.color;
            }
            if style.italic_slant != state.active_italic_slant {
                self.emit_text_matrix(state.anchor_pt, state.pen_u, state.line_v, state.cos_a, state.sin_a, style.italic_slant);
                state.active_italic_slant = style.italic_slant;
            }
        }
        if style.bold_stroke {
            let stroke_w = norm_zero((style.size_pt * 0.032).max(0.15));
            if !state.active_bold_stroke {
                self.buf.push_str("2 Tr\n");
                state.active_bold_stroke = true;
            }
            if state.active_stroke_color != Some(style.color) {
                let _ = writeln!(self.buf, "{}", format_stroke_color(style.color, self.color_space));
                state.active_stroke_color = Some(style.color);
            }
            if (stroke_w - state.active_bold_width).abs() > 1e-4 {
                let _ = writeln!(self.buf, "{stroke_w:.3} w 1 J 1 j");
                state.active_bold_width = stroke_w;
            }
        } else if state.active_bold_stroke {
            self.buf.push_str("0 Tr\n");
            state.active_bold_stroke = false;
        }
        if (style.rise_pt - state.active_rise).abs() > 1e-4 {
            let _ = writeln!(self.buf, "{:.3} Ts", norm_zero(style.rise_pt));
            state.active_rise = style.rise_pt;
        }
        let size_key = (style.size_pt * 100.0).round();
        let need_tf = state.active_font.is_none_or(|(pf, ps)| pf != font || (ps - size_key).abs() > 0.5);
        if need_tf {
            let _ = writeln!(self.buf, "/{} {:.2} Tf", font.tag(), style.size_pt);
            state.active_font = Some((font, size_key));
        }
        write_pdf_bytes_literal(&mut self.buf, bytes);
        self.buf.push_str(" Tj\n");
    }

    fn emit_outlined_segment(&mut self, slice: &str, span: &RichSpan, span_size: f64, state: &mut SelectableTextState) -> Result<(), PdfError> {
        // Close any active `BT ... ET` and outer alpha `q ... Q` before emitting vector glyph outlines.
        self.close_selectable_block(state);
        let (glyphs, width_pt) = build_outlined_fallback_glyphs(slice, span, state.font_size_pt, self.embedded_ttf);
        let xform = TextOutlineTransform {
            ax: state.anchor_pt.x,
            ay: state.anchor_pt.y,
            u_start: state.pen_u,
            v_start: state.line_v,
            cos_a: state.cos_a,
            sin_a: state.sin_a,
            page_h: self.page_h,
        };
        for glyph in &glyphs {
            self.emit_single_outlined_glyph(glyph, state.base_color, span_size, &xform)?;
        }
        state.pen_u += width_pt;
        Ok(())
    }

    fn emit_text_matrix(&mut self, anchor_pt: PointPt, pen_u: f64, line_v: f64, cos_a: f64, sin_a: f64, italic_slant: bool) {
        let tx = norm_zero(anchor_pt.x + pen_u * cos_a - line_v * sin_a);
        let ty = norm_zero(self.y_pdf(anchor_pt.y + pen_u * sin_a + line_v * cos_a));
        let slant = if italic_slant { 0.2125 } else { 0.0 };
        let c = norm_tiny(sin_a + slant * cos_a);
        let d = norm_tiny(cos_a - slant * sin_a);
        let _ = writeln!(self.buf, "{cos_a:.6} {:.6} {c:.6} {d:.6} {tx:.3} {ty:.3} Tm", norm_tiny(-sin_a));
    }

    fn emit_outlined_text(&mut self, text: &PositionedText) -> Result<(), PdfError> {
        let lines = split_rich_text_lines(&text.rich);
        let line_step = text.font_size_pt * 1.20;
        let rad = text.rotation_deg.to_radians();
        let (sin_a, cos_a) = rad.sin_cos();

        for (line_idx, line) in lines.iter().enumerate() {
            let visual_line: Cow<'_, RichText> = if rich_contains_rtl(line) { Cow::Owned(reorder_rich_text_bidi(line)) } else { Cow::Borrowed(line) };
            let outlines = outline_visual_rich_text_rotated_with_fonts_and_ttf(
                &visual_line,
                0.0,
                0.0,
                text.font_size_pt,
                0.0,
                SystemFontSet::global(),
                self.embedded_ttf,
            );
            if outlines.is_empty() {
                continue;
            }
            let line_width: f64 = outlines.iter().map(|g| if g.advance_x_pt.is_finite() { g.advance_x_pt } else { 0.0 }).sum();
            let u_start = anchor_offset_x(text.anchor, line_width);
            let v_start = (line_idx as f64) * line_step;
            let xform = TextOutlineTransform { ax: text.anchor_pt.x, ay: text.anchor_pt.y, u_start, v_start, cos_a, sin_a, page_h: self.page_h };

            for glyph in &outlines {
                self.emit_single_outlined_glyph(glyph, text.color, text.font_size_pt, &xform)?;
            }
        }
        Ok(())
    }

    fn emit_single_outlined_glyph(
        &mut self,
        glyph: &PositionedGlyphOutline,
        default_color: Rgba8,
        font_size_pt: f64,
        xform: &TextOutlineTransform,
    ) -> Result<(), PdfError> {
        if glyph.commands.is_empty() {
            return Ok(());
        }
        let color = glyph.color.unwrap_or(default_color);
        if color.a == 0 {
            return Ok(());
        }

        let has_close = glyph.commands.iter().any(|c| matches!(c, GlyphOutlineCmd::Close));
        self.buf.push_str("q\n");
        if color.a < 255 {
            self.used.gs_both.insert(color.a);
            let _ = writeln!(self.buf, "/GSa{:03} gs", color.a);
        }
        if has_close {
            let _ = writeln!(self.buf, "{}", format_fill_color(color, self.color_space));
        } else {
            let stroke_w = norm_zero((font_size_pt * 0.08).max(0.4));
            let _ = writeln!(self.buf, "{} {stroke_w:.3} w 1 J 1 j", format_stroke_color(color, self.color_space));
        }

        let mut cur_scene = PointPt::new(0.0, 0.0);
        let mut start_scene = cur_scene;
        for cmd in &glyph.commands {
            match *cmd {
                GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                    let p = xform.to_pdf(x_pt, y_pt)?;
                    let _ = writeln!(self.buf, "{:.3} {:.3} m", norm_zero(p.x), norm_zero(p.y));
                    cur_scene = PointPt::new(x_pt, y_pt);
                    start_scene = cur_scene;
                }
                GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                    let p = xform.to_pdf(x_pt, y_pt)?;
                    let _ = writeln!(self.buf, "{:.3} {:.3} l", norm_zero(p.x), norm_zero(p.y));
                    cur_scene = PointPt::new(x_pt, y_pt);
                }
                GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                    let c1x = cur_scene.x + (2.0 / 3.0) * (cx_pt - cur_scene.x);
                    let c1y = cur_scene.y + (2.0 / 3.0) * (cy_pt - cur_scene.y);
                    let c2x = x_pt + (2.0 / 3.0) * (cx_pt - x_pt);
                    let c2y = y_pt + (2.0 / 3.0) * (cy_pt - y_pt);
                    let p1 = xform.to_pdf(c1x, c1y)?;
                    let p2 = xform.to_pdf(c2x, c2y)?;
                    let p3 = xform.to_pdf(x_pt, y_pt)?;
                    let _ = writeln!(
                        self.buf,
                        "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c",
                        norm_zero(p1.x),
                        norm_zero(p1.y),
                        norm_zero(p2.x),
                        norm_zero(p2.y),
                        norm_zero(p3.x),
                        norm_zero(p3.y),
                    );
                    cur_scene = PointPt::new(x_pt, y_pt);
                }
                GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                    let p1 = xform.to_pdf(c1x_pt, c1y_pt)?;
                    let p2 = xform.to_pdf(c2x_pt, c2y_pt)?;
                    let p3 = xform.to_pdf(x_pt, y_pt)?;
                    let _ = writeln!(
                        self.buf,
                        "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c",
                        norm_zero(p1.x),
                        norm_zero(p1.y),
                        norm_zero(p2.x),
                        norm_zero(p2.y),
                        norm_zero(p3.x),
                        norm_zero(p3.y),
                    );
                    cur_scene = PointPt::new(x_pt, y_pt);
                }
                GlyphOutlineCmd::Close => {
                    self.buf.push_str("h\n");
                    cur_scene = start_scene;
                }
            }
        }

        self.buf.push_str(if has_close { "f\nQ\n" } else { "S\nQ\n" });
        Ok(())
    }
}

struct SelectableTextState {
    anchor_pt: PointPt,
    base_color: Rgba8,
    font_size_pt: f64,
    cos_a: f64,
    sin_a: f64,
    active_alpha: u8,
    active_color: Rgba8,
    active_font: Option<(PdfFontId, f64)>,
    active_rise: f64,
    active_bold_stroke: bool,
    active_bold_width: f64,
    active_stroke_color: Option<Rgba8>,
    active_italic_slant: bool,
    pen_u: f64,
    line_v: f64,
    in_text_block: bool,
}

#[derive(Clone, Copy)]
struct SpanPdfStyle {
    color: Rgba8,
    size_pt: f64,
    rise_pt: f64,
    bold_stroke: bool,
    italic_slant: bool,
}

struct TextOutlineTransform {
    ax: f64,
    ay: f64,
    u_start: f64,
    v_start: f64,
    cos_a: f64,
    sin_a: f64,
    page_h: f64,
}

impl TextOutlineTransform {
    fn to_pdf(&self, local_x: f64, local_y: f64) -> Result<PointPt, PdfError> {
        if !local_x.is_finite() || !local_y.is_finite() {
            return Err(PdfError::InvalidScene { message: String::from("non-finite glyph outline coordinate") });
        }
        let u = self.u_start + local_x;
        let v = self.v_start + local_y;
        let x_scene = self.ax + u * self.cos_a - v * self.sin_a;
        let y_scene = self.ay + u * self.sin_a + v * self.cos_a;
        Ok(PointPt::new(x_scene, self.page_h - y_scene))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PdfFontId {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    Symbol,
    HelveticaBoldOblique,
    EmbeddedTrueType,
}

impl PdfFontId {
    const fn tag(self) -> &'static str {
        match self {
            Self::Helvetica => "F1",
            Self::HelveticaBold => "F2",
            Self::HelveticaOblique => "F3",
            Self::Symbol => "F4",
            Self::HelveticaBoldOblique => "F5",
            Self::EmbeddedTrueType => "F6",
        }
    }

    const fn from_style(weight: SpanWeight, slope: SpanSlope) -> Self {
        match (weight, slope) {
            (SpanWeight::Regular, SpanSlope::Upright) => Self::Helvetica,
            (SpanWeight::Bold, SpanSlope::Upright) => Self::HelveticaBold,
            (SpanWeight::Regular, SpanSlope::Italic) => Self::HelveticaOblique,
            (SpanWeight::Bold, SpanSlope::Italic) => Self::HelveticaBoldOblique,
        }
    }
}

enum SpanPdfSegment {
    PdfText { font: PdfFontId, bytes: Vec<u8>, width_pt: f64 },
    Outlined(String),
}

fn push_outlined_text(segments: &mut Vec<SpanPdfSegment>, text: &str) {
    if let Some(SpanPdfSegment::Outlined(existing)) = segments.last_mut() {
        existing.push_str(text);
    } else {
        segments.push(SpanPdfSegment::Outlined(text.to_owned()));
    }
}

fn split_span_into_pdf_segments(span: &RichSpan, base_font_size_pt: f64, embedded_info: Option<&TrueTypeFontInfo>) -> Vec<SpanPdfSegment> {
    let size_pt = effective_span_font_size(base_font_size_pt, span);
    let core14_font = PdfFontId::from_style(span.weight, span.slope);
    let mut segments: Vec<SpanPdfSegment> = Vec::new();
    let mut chars = span.text.chars().peekable();

    while let Some(ch) = chars.next() {
        if matches!(ch, '\n' | '\r') {
            continue;
        }
        if ch == '\t' {
            let space_font = if embedded_info.is_some_and(|info| info.glyph_id(' ') != 0) { PdfFontId::EmbeddedTrueType } else { core14_font };
            let w = winansi_char_width_pt(' ', span.weight, span.slope, space_font, size_pt, embedded_info);
            for _ in 0..4 {
                push_pdf_segment_byte(&mut segments, space_font, b' ', w);
            }
            continue;
        }
        if chars.peek().is_some_and(|&next| is_combining_mark(next)) {
            let mut cluster = String::new();
            cluster.push(ch);
            while let Some(&next) = chars.peek() {
                if is_combining_mark(next) {
                    cluster.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            push_outlined_text(&mut segments, &cluster);
            continue;
        }

        if let Some(win_byte) = try_winansi_font_byte(ch)
            && embedded_info.is_some_and(|info| info.glyph_id(ch) != 0)
        {
            let w = winansi_char_width_pt(ch, span.weight, span.slope, PdfFontId::EmbeddedTrueType, size_pt, embedded_info);
            push_pdf_segment_byte(&mut segments, PdfFontId::EmbeddedTrueType, win_byte, w);
        } else if let Some(sym_byte) = symbol_font_byte(ch) {
            let w = size_pt * core14_advance_em(ch, span.weight, span.slope);
            push_pdf_segment_byte(&mut segments, PdfFontId::Symbol, sym_byte, w);
        } else if let Some(win_byte) = try_winansi_font_byte(ch) {
            let w = winansi_char_width_pt(ch, span.weight, span.slope, core14_font, size_pt, embedded_info);
            push_pdf_segment_byte(&mut segments, core14_font, win_byte, w);
        } else {
            let mut utf8 = [0_u8; 4];
            push_outlined_text(&mut segments, ch.encode_utf8(&mut utf8));
        }
    }
    segments
}

fn push_pdf_segment_byte(segments: &mut Vec<SpanPdfSegment>, font: PdfFontId, byte: u8, char_width_pt: f64) {
    if let Some(SpanPdfSegment::PdfText { font: prev_font, bytes, width_pt }) = segments.last_mut()
        && *prev_font == font
    {
        bytes.push(byte);
        *width_pt += char_width_pt;
    } else {
        segments.push(SpanPdfSegment::PdfText { font, bytes: vec![byte], width_pt: char_width_pt });
    }
}

fn winansi_char_width_pt(
    ch: char,
    weight: SpanWeight,
    slope: SpanSlope,
    base_font: PdfFontId,
    size_pt: f64,
    embedded_info: Option<&TrueTypeFontInfo>,
) -> f64 {
    if base_font == PdfFontId::EmbeddedTrueType
        && let Some(info) = embedded_info
    {
        info.width_pt(ch, size_pt)
    } else {
        size_pt * core14_advance_em(ch, weight, slope)
    }
}

fn build_outlined_fallback_glyphs(
    slice: &str,
    span: &RichSpan,
    base_font_size_pt: f64,
    embedded_ttf: Option<&TrueTypeFont<'_>>,
) -> (Vec<PositionedGlyphOutline>, f64) {
    let single_span = RichText::new(vec![RichSpan { text: slice.to_owned(), ..span.clone() }]);
    let glyphs =
        outline_visual_rich_text_rotated_with_fonts_and_ttf(&single_span, 0.0, 0.0, base_font_size_pt, 0.0, SystemFontSet::global(), embedded_ttf);
    let pen_x = glyphs.iter().map(|g| if g.advance_x_pt.is_finite() { g.advance_x_pt } else { 0.0 }).sum();
    (glyphs, pen_x)
}

fn outlined_slice_width_pt(slice: &str, span: &RichSpan, base_font_size_pt: f64, embedded_ttf: Option<&TrueTypeFont<'_>>) -> f64 {
    build_outlined_fallback_glyphs(slice, span, base_font_size_pt, embedded_ttf).1
}

/// Adobe Type 1 Symbol font encoding map (PDF Reference Appendix D.4).
/// Covers full uppercase/lowercase Greek (`Α..Ω`, `α..ω`, variants `ϑ`, `ϒ`, `ϕ`, `ϖ`, `ς`) and
/// scientific mathematical operators (`±`, `≤`, `≥`, `×`, `÷`, `≈`, `≠`, `≡`, `∝`, `∞`, `µ`, `−`, `°`,
/// `⋅`, `√`, `∑`, `∏`, `∫`, `∂`, `∇`, arrows, and set/logic symbols).
fn symbol_font_byte(ch: char) -> Option<u8> {
    let code = match ch {
        // Uppercase Greek
        'Α' => 0x41,
        'Β' => 0x42,
        'Γ' => 0x47,
        'Δ' | '\u{2206}' => 0x44,
        'Ε' => 0x45,
        'Ζ' => 0x5A,
        'Η' => 0x48,
        'Θ' => 0x51,
        'Ι' => 0x49,
        'Κ' => 0x4B,
        'Λ' => 0x4C,
        'Μ' => 0x4D,
        'Ν' => 0x4E,
        'Ξ' => 0x58,
        'Ο' => 0x4F,
        'Π' => 0x50,
        'Ρ' => 0x52,
        'Σ' => 0x53,
        'Τ' => 0x54,
        'Υ' => 0x55,
        'ϒ' => 0xA1,
        'Φ' => 0x46,
        'Χ' => 0x43,
        'Ψ' => 0x59,
        'Ω' | '\u{2126}' => 0x57,
        // Lowercase Greek & micro sign
        'α' => 0x61,
        'β' => 0x62,
        'γ' => 0x67,
        'δ' => 0x64,
        'ε' | 'ϵ' => 0x65,
        'ζ' => 0x7A,
        'η' => 0x68,
        'θ' => 0x71,
        'ϑ' => 0x4A,
        'ι' => 0x69,
        'κ' => 0x6B,
        'λ' => 0x6C,
        'μ' | 'µ' => 0x6D,
        'ν' => 0x6E,
        'ξ' => 0x78,
        'ο' => 0x6F,
        'π' => 0x70,
        'ρ' => 0x72,
        'ς' => 0x56,
        'σ' => 0x73,
        'τ' => 0x74,
        'υ' => 0x75,
        'φ' => 0x66,
        'ϕ' => 0x6A,
        'χ' => 0x63,
        'ψ' => 0x79,
        'ω' => 0x77,
        'ϖ' => 0x76,
        // Scientific math & relational operators
        '∀' => 0x22,
        '∃' => 0x24,
        '∋' => 0x27,
        '∗' => 0x2A,
        '−' => 0x2D,
        '≅' => 0x40,
        '∴' => 0x5C,
        '⊥' => 0x5E,
        '∼' => 0x7E,
        '′' => 0xA2,
        '≤' => 0xA3,
        '⁄' => 0xA4,
        '∞' => 0xA5,
        '↔' => 0xAB,
        '←' => 0xAC,
        '↑' => 0xAD,
        '→' => 0xAE,
        '↓' => 0xAF,
        '°' => 0xB0,
        '±' => 0xB1,
        '″' => 0xB2,
        '≥' => 0xB3,
        '×' => 0xB4,
        '∝' => 0xB5,
        '∂' => 0xB6,
        '÷' => 0xB8,
        '≠' => 0xB9,
        '≡' => 0xBA,
        '≈' => 0xBB,
        '⊗' => 0xC4,
        '⊕' => 0xC5,
        '∅' => 0xC6,
        '∩' => 0xC7,
        '∪' => 0xC8,
        '⊃' => 0xC9,
        '⊇' => 0xCA,
        '⊄' => 0xCB,
        '⊂' => 0xCC,
        '⊆' => 0xCD,
        '∈' => 0xCE,
        '∉' => 0xCF,
        '∠' => 0xD0,
        '∇' => 0xD1,
        '∏' => 0xD5,
        '√' => 0xD6,
        '⋅' => 0xD7,
        '¬' => 0xD8,
        '∧' => 0xD9,
        '∨' => 0xDA,
        '⇔' => 0xDB,
        '⇐' => 0xDC,
        '⇑' => 0xDD,
        '⇒' => 0xDE,
        '⇓' => 0xDF,
        '◊' => 0xE0,
        '∑' => 0xE5,
        '∫' => 0xF2,
        _ => return None,
    };
    Some(code)
}

/// WinAnsiEncoding (`Windows-1252`) single-byte character encoder (PDF Appendix D.2).
/// Returns `None` for characters outside Windows-1252 so non-WinAnsi glyphs render via vector outlines.
fn try_winansi_font_byte(ch: char) -> Option<u8> {
    let byte = match ch {
        c if (' '..='~').contains(&c) => c as u8,
        '€' => 0x80,
        '‚' => 0x82,
        'ƒ' => 0x83,
        '„' => 0x84,
        '…' => 0x85,
        '†' => 0x86,
        '‡' => 0x87,
        'ˆ' => 0x88,
        '‰' => 0x89,
        'Š' => 0x8A,
        '‹' => 0x8B,
        'Œ' => 0x8C,
        'Ž' => 0x8E,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '˜' => 0x98,
        '™' => 0x99,
        'š' => 0x9A,
        '›' => 0x9B,
        'œ' => 0x9C,
        'ž' => 0x9E,
        'Ÿ' => 0x9F,
        c if ('\u{00A0}'..='\u{00FF}').contains(&c) => c as u8,
        _ => return None,
    };
    Some(byte)
}

fn write_pdf_bytes_literal(out: &mut String, bytes: &[u8]) {
    out.push('(');
    for &b in bytes {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'(' => out.push_str("\\("),
            b')' => out.push_str("\\)"),
            0x20..=0x7E => out.push(b as char),
            other => {
                let _ = write!(out, "\\{other:03o}");
            }
        }
    }
    out.push(')');
}

fn format_pdf_info_string(raw: &str) -> String {
    if raw.chars().all(|c| (' '..='~').contains(&c)) {
        let mut out = String::with_capacity(raw.len() + 2);
        write_pdf_bytes_literal(&mut out, raw.as_bytes());
        out
    } else {
        // UTF-16BE BOM hex string per PDF 1.4 §3.8.1 for Unicode metadata.
        let mut out = String::from("<FEFF");
        for unit in raw.encode_utf16() {
            let _ = write!(out, "{unit:04X}");
        }
        out.push('>');
        out
    }
}

fn effective_span_font_size(base_font_size_pt: f64, span: &RichSpan) -> f64 {
    let raw_scale = if span.scale.is_finite() && span.scale > 0.0 { span.scale } else { 1.0 };
    let script_scale = match span.script {
        SpanScript::Baseline => raw_scale,
        SpanScript::Superscript | SpanScript::Subscript => {
            if (raw_scale - 1.0).abs() < 1e-6 {
                0.70
            } else {
                raw_scale
            }
        }
    };
    (base_font_size_pt * script_scale).max(0.5)
}

fn anchor_offset_x(anchor: TextAnchor, total_width: f64) -> f64 {
    match anchor {
        TextAnchor::Start => 0.0,
        TextAnchor::Middle => -0.5 * total_width,
        TextAnchor::End => -total_width,
    }
}

fn measure_span_width_pt(
    span: &RichSpan,
    base_font_size_pt: f64,
    embedded_info: Option<&TrueTypeFontInfo>,
    embedded_ttf: Option<&TrueTypeFont<'_>>,
) -> f64 {
    let mut total = 0.0;
    for seg in split_span_into_pdf_segments(span, base_font_size_pt, embedded_info) {
        match seg {
            SpanPdfSegment::PdfText { width_pt, .. } => total += width_pt,
            SpanPdfSegment::Outlined(slice) => {
                total += outlined_slice_width_pt(&slice, span, base_font_size_pt, embedded_ttf);
            }
        }
    }
    total
}

fn measure_selectable_text_width_pt(
    rich: &RichText,
    base_font_size_pt: f64,
    embedded_info: Option<&TrueTypeFontInfo>,
    embedded_ttf: Option<&TrueTypeFont<'_>>,
) -> f64 {
    rich.spans.iter().map(|span| measure_span_width_pt(span, base_font_size_pt, embedded_info, embedded_ttf)).sum()
}

fn format_fill_color(color: Rgba8, space: PdfColorSpace) -> String {
    match space {
        PdfColorSpace::Srgb => {
            let r = norm_zero(f64::from(color.r) / 255.0);
            let g = norm_zero(f64::from(color.g) / 255.0);
            let b = norm_zero(f64::from(color.b) / 255.0);
            format!("{r:.3} {g:.3} {b:.3} rg")
        }
        PdfColorSpace::Grayscale => {
            let r = f64::from(color.r) / 255.0;
            let g = f64::from(color.g) / 255.0;
            let b = f64::from(color.b) / 255.0;
            let luma = norm_zero((0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 1.0));
            format!("{luma:.3} g")
        }
        PdfColorSpace::Cmyk => {
            let cmyk = color.to_cmyk();
            format!("{:.3} {:.3} {:.3} {:.3} k", norm_zero(cmyk.c), norm_zero(cmyk.m), norm_zero(cmyk.y), norm_zero(cmyk.k))
        }
    }
}

fn format_stroke_color(color: Rgba8, space: PdfColorSpace) -> String {
    let fill_cmd = format_fill_color(color, space);
    if let Some(prefix) = fill_cmd.strip_suffix(" rg") {
        format!("{prefix} RG")
    } else if let Some(prefix) = fill_cmd.strip_suffix(" g") {
        format!("{prefix} G")
    } else if let Some(prefix) = fill_cmd.strip_suffix(" k") {
        format!("{prefix} K")
    } else {
        fill_cmd
    }
}

fn rect_to_pdf_box(rect: RectPt, page_h: f64) -> [f64; 4] {
    let llx = norm_zero(rect.x);
    let lly = norm_zero(page_h - (rect.y + rect.height));
    let urx = norm_zero(rect.x + rect.width);
    let ury = norm_zero(page_h - rect.y);
    [llx, lly, urx, ury]
}

fn check_finite_pt(pt: PointPt, label: &str) -> Result<(), PdfError> {
    if !pt.x.is_finite() || !pt.y.is_finite() {
        return Err(PdfError::InvalidScene { message: format!("non-finite coordinate in {label}") });
    }
    Ok(())
}

fn check_finite_rect(rect: RectPt, label: &str) -> Result<(), PdfError> {
    if !rect.x.is_finite() || !rect.y.is_finite() || !rect.width.is_finite() || !rect.height.is_finite() {
        return Err(PdfError::InvalidScene { message: format!("non-finite rectangle in {label}") });
    }
    Ok(())
}

fn norm_zero(v: f64) -> f64 {
    if v.abs() < 0.0005 { 0.0 } else { v }
}

fn norm_tiny(v: f64) -> f64 {
    if v.abs() < 5e-7 { 0.0 } else { v }
}

struct VecAsciiWriter<'a>(&'a mut Vec<u8>);

fn pdf_writer(vec: &mut Vec<u8>) -> VecAsciiWriter<'_> {
    VecAsciiWriter(vec)
}

impl std::fmt::Write for VecAsciiWriter<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}
