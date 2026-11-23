//! Regression tests for audit P4 (`FA-SEC10-01`, `FA-SEC10-02`, `FA-X1-03` PDF side).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;

use curvebook_chart::{ChartScene, Path2D, PointPt, PositionedText, RectPt, ScenePrimitive, SceneRasterImage, StrokePaint, TextAnchor};
use curvebook_doc::{CropInsetsPt, ImageMimeKind, MAX_IMAGE_DIM_PX};
use curvebook_palette::Rgba8;
use curvebook_pdf::{PdfColorSpace, PdfError, PdfExportOptions, render_pdf, render_pdf_pages};
use curvebook_text::{RichSpan, RichText, SpanSlope, SpanWeight, build_minimal_truetype_font};
use flate2::read::ZlibDecoder;

fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn put_bytes(buf: &mut [u8], off: usize, bytes: &[u8]) {
    if let Some(dst) = buf.get_mut(off..off + bytes.len()) {
        dst.copy_from_slice(bytes);
    }
}

/// Build an uncompressed Regular SFNT TrueType font mapping only ASCII `'A'..='Z'` (`0x0041..=0x005A`).
/// Deliberately omits `'€'` (Win-1252-only) and `'±'` (Win-1252 + Symbol) to verify `/F6` fallback.
fn build_ascii_caps_only_ttf() -> Vec<u8> {
    let num_glyphs: u16 = 27; // 0 = .notdef, 1..=26 = A..Z

    let mut head = vec![0u8; 54];
    put_bytes(&mut head, 0, &0x0001_0000u32.to_be_bytes());
    put_bytes(&mut head, 12, &0x5F0F_3CF5u32.to_be_bytes());
    put_bytes(&mut head, 18, &1000u16.to_be_bytes());
    put_bytes(&mut head, 36, &0i16.to_be_bytes());
    put_bytes(&mut head, 38, &(-200i16).to_be_bytes());
    put_bytes(&mut head, 40, &700i16.to_be_bytes());
    put_bytes(&mut head, 42, &800i16.to_be_bytes());

    let mut hhea = vec![0u8; 36];
    put_bytes(&mut hhea, 0, &0x0001_0000u32.to_be_bytes());
    put_bytes(&mut hhea, 4, &800i16.to_be_bytes());
    put_bytes(&mut hhea, 6, &(-200i16).to_be_bytes());
    put_bytes(&mut hhea, 8, &90i16.to_be_bytes());
    put_bytes(&mut hhea, 34, &num_glyphs.to_be_bytes());

    let mut maxp = vec![0u8; 6];
    put_bytes(&mut maxp, 0, &0x0001_0000u32.to_be_bytes());
    put_bytes(&mut maxp, 4, &num_glyphs.to_be_bytes());

    let mut hmtx = Vec::with_capacity(usize::from(num_glyphs) * 4);
    for _ in 0..num_glyphs {
        push_u16(&mut hmtx, 667);
        push_i16(&mut hmtx, 40);
    }

    // cmap format 4 mapping `65..=90` -> `gid 1..=26` (`delta = -64`).
    let mut cmap = Vec::new();
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 1);
    push_u16(&mut cmap, 3);
    push_u16(&mut cmap, 1);
    push_u32(&mut cmap, 12);
    push_u16(&mut cmap, 4);
    push_u16(&mut cmap, 32);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 4);
    push_u16(&mut cmap, 4);
    push_u16(&mut cmap, 1);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 90);
    push_u16(&mut cmap, 0xFFFF);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 65);
    push_u16(&mut cmap, 0xFFFF);
    push_i16(&mut cmap, -64);
    push_i16(&mut cmap, 1);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 0);

    let ps = "CapsOnlySubset";
    let mut str_buf = Vec::new();
    for unit in ps.encode_utf16() {
        push_u16(&mut str_buf, unit);
    }
    let mut name = Vec::new();
    push_u16(&mut name, 0);
    push_u16(&mut name, 2);
    push_u16(&mut name, 30);
    for name_id in [1u16, 6u16] {
        push_u16(&mut name, 3);
        push_u16(&mut name, 1);
        push_u16(&mut name, 0x0409);
        push_u16(&mut name, name_id);
        push_u16(&mut name, u16::try_from(str_buf.len()).unwrap());
        push_u16(&mut name, 0);
    }
    name.extend_from_slice(&str_buf);

    let mut post = vec![0u8; 32];
    put_bytes(&mut post, 0, &0x0003_0000u32.to_be_bytes());

    let mut os2 = vec![0u8; 96];
    put_bytes(&mut os2, 0, &4u16.to_be_bytes());
    put_bytes(&mut os2, 4, &400u16.to_be_bytes());
    put_bytes(&mut os2, 62, &0x0040u16.to_be_bytes());
    put_bytes(&mut os2, 68, &800i16.to_be_bytes());
    put_bytes(&mut os2, 70, &(-200i16).to_be_bytes());
    put_bytes(&mut os2, 88, &700i16.to_be_bytes());

    let tables = [
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::new();
    push_u32(&mut font, 0x0001_0000);
    push_u16(&mut font, u16::try_from(tables.len()).unwrap());
    push_u16(&mut font, 128);
    push_u16(&mut font, 3);
    push_u16(&mut font, 0);

    let mut offset = dir_size;
    for (tag, data) in &tables {
        font.extend_from_slice(tag);
        push_u32(&mut font, 0);
        push_u32(&mut font, u32::try_from(offset).unwrap());
        push_u32(&mut font, u32::try_from(data.len()).unwrap());
        offset += (data.len() + 3) & !3;
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        while !font.len().is_multiple_of(4) {
            font.push(0);
        }
    }
    font
}

fn inflate_first_flate_page_or_raw(pdf: &[u8]) -> String {
    // Check raw content streams (`compress_streams: false`) directly when unambiguous,
    // otherwise decompress non-`/Length1` Flate streams and concatenate.
    let text = String::from_utf8_lossy(pdf);
    if !text.contains(" /Filter /FlateDecode >>\nstream\n") {
        return text.into_owned();
    }
    let mut out = String::new();
    let mut idx = 0usize;
    while let Some(tail) = pdf.get(idx..)
        && let Some(rel) = tail.windows(b"\nstream\n".len()).position(|w| w == b"\nstream\n")
    {
        let header_slice = tail.get(..rel).unwrap_or(&[]);
        let header = String::from_utf8_lossy(header_slice);
        let stream_start = idx + rel + b"\nstream\n".len();
        let after_start = pdf.get(stream_start..).unwrap_or(&[]);
        let end_rel = after_start.windows(b"\nendstream".len()).position(|w| w == b"\nendstream").expect("endstream");
        let payload = after_start.get(..end_rel).unwrap_or(&[]);
        if header.contains("/FlateDecode") && !header.contains("/Length1 ") {
            let mut dec = ZlibDecoder::new(payload);
            let mut s = String::new();
            if dec.read_to_string(&mut s).is_ok() {
                out.push_str(&s);
                out.push('\n');
            }
        }
        idx = stream_start + end_rel;
    }
    out
}

#[test]
fn sec10_01_pdf_ttf_cmap_fallback_bold_italic_and_combining_context() {
    // 1. Subset TTF lacking `€` and `±`: `'A'` uses `/F6`, `€` falls back to `/F1`, `±` uses `/F4`.
    let mut subset_scene = ChartScene::new("ttf-cmap", "Cmap Fallback", 320.0, 140.0, Some(Rgba8::WHITE));
    subset_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("A € ±"),
        anchor_pt: PointPt::new(20.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let subset_opts = PdfExportOptions {
        compress_streams: false,
        outline_text: false,
        embedded_truetype_font: Some(build_ascii_caps_only_ttf()),
        ..PdfExportOptions::default()
    };
    let subset_pdf = render_pdf(&subset_scene, &subset_opts).expect("render subset pdf");
    let subset_str = String::from_utf8_lossy(&subset_pdf);
    assert!(subset_str.contains("/F6 12.00 Tf\n(A) Tj"), "mapped 'A' should use /F6");
    assert!(subset_str.contains("/F1 12.00 Tf\n( \\200 ) Tj"), "unmapped space and WinAnsi '€' must fall back to /F1");
    assert!(subset_str.contains("/F4 12.00 Tf\n(\\261) Tj"), "unmapped symbol '±' must use /F4");

    // 2. Bold + Italic + Combining-mark context on embedded Regular TTF.
    let mut styled_scene = ChartScene::new("ttf-styles", "Styles", 360.0, 140.0, Some(Rgba8::WHITE));
    styled_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![
            RichSpan { weight: SpanWeight::Bold, ..RichSpan::plain("Bold ") },
            RichSpan { slope: SpanSlope::Italic, ..RichSpan::plain("Ital ") },
            RichSpan::plain("Cafe\u{0301} ok"),
        ]),
        anchor_pt: PointPt::new(20.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let full_opts = PdfExportOptions {
        compress_streams: false,
        outline_text: false,
        embedded_truetype_font: Some(build_minimal_truetype_font()),
        ..PdfExportOptions::default()
    };
    let styled_pdf = render_pdf(&styled_scene, &full_opts).expect("render styled TTF pdf");
    let styled_str = String::from_utf8_lossy(&styled_pdf);
    assert!(styled_str.contains("2 Tr\n"), "Bold on Regular TTF must emit fill+stroke text rendering mode 2 Tr");
    assert!(styled_str.contains("0 Tr\n"), "Must reset 0 Tr after leaving Bold");
    assert!(styled_str.contains("0.212500"), "Italic on upright TTF must apply oblique shear Tm");
    assert!(!styled_str.contains("/F2 12.00 Tf"), "Mapped Bold TTF span must stay in /F6 instead of Core-14 Helvetica-Bold");
    assert!(!styled_str.contains("/F3 12.00 Tf"), "Mapped Italic TTF span must stay in /F6 instead of Core-14 Helvetica-Oblique");
    assert!(styled_str.contains("/F6 12.00 Tf\n(Bold ) Tj"), "Bold TTF text should emit under /F6");
    assert!(styled_str.contains("(Ital ) Tj"), "Italic TTF text should emit under /F6");
    // `'e\u{0301}'` cluster is grouped cleanly into outlined vector fallback alongside selectable `(Caf) Tj` and `( ok) Tj`.
    assert!(styled_str.contains("(Caf) Tj"), "base ASCII prefix stays selectable");
    assert!(styled_str.contains("( ok) Tj"), "trailing ASCII remains selectable");
    assert!(styled_str.contains(" m\n") && styled_str.matches("\nf\nQ\n").count() >= 2, "expected both base 'e' and combining acute outlined paths");
}

#[test]
fn sec10_02_pdf_skips_unused_ttf_when_outlined_or_unreferenced() {
    let mut text_scene = ChartScene::new("txt", "Text", 300.0, 120.0, Some(Rgba8::WHITE));
    text_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Dose 2026"),
        anchor_pt: PointPt::new(20.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    // Even when `embedded_truetype_font` is provided, `outline_text: true` must not embed `/FontFile2` or `/F6`.
    let out_opts = PdfExportOptions {
        compress_streams: false,
        outline_text: true,
        embedded_truetype_font: Some(build_minimal_truetype_font()),
        ..PdfExportOptions::default()
    };
    let out_pdf = render_pdf(&text_scene, &out_opts).expect("outlined pdf");
    let out_str = String::from_utf8_lossy(&out_pdf);
    assert!(!out_str.contains("/FontFile2"), "outline_text=true must not embed /FontFile2");
    assert!(!out_str.contains("/FontDescriptor"), "outline_text=true must not embed /FontDescriptor");
    assert!(!out_str.contains("/F6 "), "outline_text=true must not advertise /F6 in page resources");

    // Geometry-only scene with `outline_text: false` also omits unused `/F6` and `/FontFile2`.
    let mut path_scene = ChartScene::new("geom", "Geom", 200.0, 120.0, Some(Rgba8::WHITE));
    path_scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(10.0, 10.0), PointPt::new(100.0, 80.0)),
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    });
    let sel_opts = PdfExportOptions {
        compress_streams: true,
        outline_text: false,
        embedded_truetype_font: Some(build_minimal_truetype_font()),
        ..PdfExportOptions::default()
    };
    let geom_pdf = render_pdf(&path_scene, &sel_opts).expect("geometry pdf");
    let geom_str = String::from_utf8_lossy(&geom_pdf);
    assert!(!geom_str.contains("/FontFile2"), "unused TTF must not bloat geometry-only PDF");
    assert!(!geom_str.contains("/F6 "), "unused /F6 must not appear in page resources");

    // Multi-page: page 0 (geom only) omits `/F6`, page 1 (text) includes `/F6` and embeds `/FontFile2` once.
    let multi = render_pdf_pages(&[path_scene, text_scene], &sel_opts).expect("multi-page pdf");
    let multi_str = String::from_utf8_lossy(&multi);
    assert!(multi_str.contains("/FontFile2"));
    assert_eq!(multi_str.matches("/F6 ").count(), 1, "only page 2 should reference /F6 in Resources: {multi_str}");
    assert!(!inflate_first_flate_page_or_raw(&multi).is_empty());
}

#[test]
fn x1_03_pdf_image_enforces_pixel_bounds_and_pre_alloc_uncompressed_budget() {
    // 1. Oversized single dimension > MAX_IMAGE_DIM_PX is rejected cleanly.
    let mut dim_scene = ChartScene::new("img-dim", "Too Wide", 100.0, 100.0, None);
    let wide_w = MAX_IMAGE_DIM_PX + 1;
    dim_scene.primitives.push(ScenePrimitive::RasterImage(SceneRasterImage {
        asset_id: String::from("too-wide"),
        media_type: ImageMimeKind::Png,
        pixel_size: (wide_w, 1),
        rgba: vec![255; (wide_w as usize) * 4],
        bounds_pt: RectPt::new(0.0, 0.0, 50.0, 50.0),
        crop: CropInsetsPt::default(),
        rotation_deg: 0.0,
        clip: None,
    }));
    assert!(matches!(render_pdf(&dim_scene, &PdfExportOptions::default()), Err(PdfError::ResourceLimit { .. })));

    // 2. Single valid 4096 x 2800 uniform RGBA image:
    //    - sRGB raw bytes = 11_468_800 * 3 = 34_406_400 > MAX_PDF_OUTPUT_BYTES (33_554_432).
    //    - Uncompressed fails pre-allocation check with ResourceLimit.
    //    - Compressed succeeds smoothly with bounded streaming deflate (< 100 KB PDF output).
    let (w, h) = (4096u32, 2800u32);
    let mut big_scene = ChartScene::new("img-budget", "Big Raster", 200.0, 200.0, None);
    big_scene.primitives.push(ScenePrimitive::RasterImage(SceneRasterImage {
        asset_id: String::from("uniform-huge"),
        media_type: ImageMimeKind::Png,
        pixel_size: (w, h),
        rgba: vec![255u8; (w as usize) * (h as usize) * 4],
        bounds_pt: RectPt::new(10.0, 10.0, 180.0, 180.0),
        crop: CropInsetsPt::default(),
        rotation_deg: 0.0,
        clip: None,
    }));

    let raw_opts = PdfExportOptions { color_space: PdfColorSpace::Srgb, compress_streams: false, ..PdfExportOptions::default() };
    assert!(
        matches!(render_pdf(&big_scene, &raw_opts), Err(PdfError::ResourceLimit { .. })),
        "uncompressed > 32 MiB raster must be rejected before allocating raw color buffer"
    );

    let comp_opts = PdfExportOptions { color_space: PdfColorSpace::Srgb, compress_streams: true, ..PdfExportOptions::default() };
    let compressed_pdf = render_pdf(&big_scene, &comp_opts).expect("uniform huge raster should compress cleanly under streaming budget");
    assert!(compressed_pdf.starts_with(b"%PDF-1.7\n"));
    assert!(compressed_pdf.len() < 256 * 1024);
}
