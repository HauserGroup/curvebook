//! Regression tests for AUD-11 findings in `crates/render`:
//! `AUD-11-01`, `AUD-11-09`, `AUD-11-10`, `AUD-11-11`, `AUD-11-12`, `AUD-11-17`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_chart::{ChartScene, FillPaint, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor};
use curvebook_palette::Rgba8;
use curvebook_render::{RasterImage, RenderTargetSpec, encode_emf, encode_raster_emf, render_scene};
use curvebook_text::{RichSpan, RichText};

const EMR_POLYBEZIERTO: u32 = 5;
const EMR_STRETCHDIBITS: u32 = 81;
const EMR_EXTCREATEFONTINDIRECTW: u32 = 82;
const EMR_EXTTEXTOUTW: u32 = 84;

fn read_u32_le(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(bytes[off..off + 4].try_into().expect("u32"))
}

fn read_i32_le(bytes: &[u8], off: usize) -> i32 {
    i32::from_le_bytes(bytes[off..off + 4].try_into().expect("i32"))
}

fn iter_emf_records(emf: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= emf.len() {
        let i_type = read_u32_le(emf, off);
        let n_size = read_u32_le(emf, off + 4) as usize;
        assert!(n_size >= 8 && off + n_size <= emf.len(), "malformed EMF record at {off}");
        out.push((i_type, &emf[off..off + n_size]));
        off += n_size;
    }
    assert_eq!(off, emf.len(), "EMF trailing bytes");
    out
}

fn small_diamond(cx: f64, cy: f64, r: f64) -> Path2D {
    let mut p = Path2D::new();
    p.commands.push(PathCmd::MoveTo(PointPt::new(cx, cy - r)));
    p.commands.push(PathCmd::LineTo(PointPt::new(cx + r, cy)));
    p.commands.push(PathCmd::LineTo(PointPt::new(cx, cy + r)));
    p.commands.push(PathCmd::LineTo(PointPt::new(cx - r, cy)));
    p.commands.push(PathCmd::Close);
    p
}

#[test]
fn aud11_01_offscreen_small_poly_stamp_neither_panics_nor_wraps_onto_canvas() {
    let target = RenderTargetSpec { width_px: 120, height_px: 120, dpi: 72 };
    let mut scene = ChartScene::new("aud11-01", "Offscreen Small Poly", 120.0, 120.0, Some(Rgba8::WHITE));
    // 1. i32::MAX finite coordinate (previously panicked in debug/test with `anchor_x + max_dx` overflow).
    scene.primitives.push(ScenePrimitive::Path {
        path: small_diamond(f64::from(i32::MAX), 50.0, 4.0),
        fill: Some(FillPaint::solid(Rgba8::rgb(220, 0, 0))),
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    });
    // 2. 2^32 + 50.0 finite coordinate (previously truncated mod 2^32 to x=50 and drew a phantom marker at (50, 50)).
    let wrap_x = 4_294_967_296.0_f64 + 50.0;
    scene.primitives.push(ScenePrimitive::Path {
        path: small_diamond(wrap_x, 50.0, 4.0),
        fill: Some(FillPaint::solid(Rgba8::rgb(0, 0, 220))),
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(wrap_x, 50.0, 6.0, 6.0)),
        fill: Some(FillPaint::solid(Rgba8::rgb(0, 200, 0))),
        stroke: None,
        clip: None,
    });

    let img = render_scene(&scene, target).expect("render must not panic");
    // Every pixel on the visible canvas must remain solid white (zero phantom markers).
    assert!(img.rgba.as_chunks::<4>().0.iter().all(|px| *px == [255, 255, 255, 255]), "phantom off-screen marker leaked onto visible canvas");
}

#[test]
fn aud11_09_encode_raster_emf_composites_transparent_pixels_over_white() {
    let mut img = RasterImage::new(2, 1, 96, Rgba8::TRANSPARENT).expect("2x1 raster");
    // Pixel 0: fully transparent [0, 0, 0, 0] -> must composite to white [255, 255, 255, 255], NOT black.
    // Pixel 1: semi-transparent red [200, 0, 0, 128] over white -> RGB ~(227, 127, 127), opaque.
    img.rgba = vec![0, 0, 0, 0, 200, 0, 0, 128];

    let emf = encode_raster_emf(&img).expect("encode transparent raster EMF");
    let dib_rec = iter_emf_records(&emf).into_iter().find(|(t, _)| *t == EMR_STRETCHDIBITS).expect("EMR_STRETCHDIBITS").1;
    let off_bits = read_u32_le(dib_rec, 56) as usize;
    let pixels = &dib_rec[off_bits..off_bits + 8];
    assert_eq!(&pixels[0..4], &[255, 255, 255, 255], "transparent pixel must composite to white, not solid black");
    // Semi-transparent red over white in BGRX: B=127, G=127, R=227, X=255.
    assert!(
        pixels[2] > 200 && pixels[1] > 100 && pixels[0] > 100 && pixels[3] == 255,
        "semi-transparent pixel must blend over white: {:?}",
        &pixels[4..8]
    );
    assert!(pixels[6] > 200 && pixels[5] > 100 && pixels[4] > 100 && pixels[7] == 255, "semi-transparent red over white BGRX: {:?}", &pixels[4..8]);
}

#[test]
fn aud11_10_extreme_finite_bezier_coordinates_do_not_amplify_subdivision() {
    let mut scene = ChartScene::new("aud11-10", "Extreme Beziers", 100.0, 100.0, Some(Rgba8::WHITE));
    let mut path = Path2D::new();
    path.commands.push(PathCmd::MoveTo(PointPt::new(1e160, 1e160)));
    for i in 0..5_000u32 {
        let off = f64::from(i) * 1e155;
        path.commands.push(PathCmd::CubicTo {
            c1: PointPt::new(1e160 + off, 1e160 + 1e158),
            c2: PointPt::new(1e160 + off + 1e158, 1e160),
            to: PointPt::new(1e160 + off + 2e158, 1e160),
        });
    }
    // Also include a curve straddling the viewport with huge finite control points (1e308) where squares overflow.
    path.commands.push(PathCmd::MoveTo(PointPt::new(-1e160, 50.0)));
    path.commands.push(PathCmd::QuadTo { control: PointPt::new(0.0, 1e160), to: PointPt::new(1e160, 50.0) });
    scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)), clip: None });

    let t0 = Instant::now();
    let target = RenderTargetSpec { width_px: 100, height_px: 100, dpi: 72 };
    let _img = render_scene(&scene, target).expect("render extreme beziers");
    let elapsed = t0.elapsed();
    assert!(elapsed.as_secs_f64() < 2.0, "extreme-coord bezier flattening must stay bounded, took {elapsed:?}");
}

#[test]
fn aud11_11_emf_chunks_long_polybezier_sequences() {
    let mut scene = ChartScene::new("aud11-11", "Chunked Beziers", 200.0, 100.0, Some(Rgba8::WHITE));
    let mut path = Path2D::new();
    path.commands.push(PathCmd::MoveTo(PointPt::new(5.0, 50.0)));
    for i in 0..2_500u32 {
        let x = 5.0 + f64::from(i) * 0.05;
        path.commands.push(PathCmd::CubicTo { c1: PointPt::new(x + 0.01, 40.0), c2: PointPt::new(x + 0.03, 60.0), to: PointPt::new(x + 0.05, 50.0) });
    }
    scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)), clip: None });
    let emf = encode_emf(&scene).expect("encode chunked beziers");
    let poly_counts: Vec<u32> =
        iter_emf_records(&emf).into_iter().filter(|(t, _)| *t == EMR_POLYBEZIERTO).map(|(_, rec)| read_u32_le(rec, 24)).collect();
    assert!(poly_counts.len() >= 3, "expected multiple bounded EMR_POLYBEZIERTO chunks, got {poly_counts:?}");
    assert!(poly_counts.iter().all(|&c| c <= 3 * 1024 && c.is_multiple_of(3)));
    assert_eq!(poly_counts.iter().sum::<u32>(), 2_500 * 3);
}

#[test]
fn aud11_12_emf_expands_tabs_to_spaces_and_normalizes_escapement() {
    let mut scene = ChartScene::new("aud11-12", "Tabs and Angle", 200.0, 100.0, Some(Rgba8::WHITE));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Group A\t(n=12)"),
        anchor_pt: PointPt::new(20.0, 40.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        color: Rgba8::BLACK,
        rotation_deg: 90.0,
    }));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("HugeAngle"),
        anchor_pt: PointPt::new(20.0, 70.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        color: Rgba8::BLACK,
        rotation_deg: 1e12,
    }));

    let emf = encode_emf(&scene).expect("encode emf");
    let mut texts = Vec::new();
    let mut escapements = Vec::new();
    for (i_type, rec) in iter_emf_records(&emf) {
        if i_type == EMR_EXTTEXTOUTW {
            let n_chars = read_u32_le(rec, 44) as usize;
            let off_str = read_u32_le(rec, 48) as usize;
            let units: Vec<u16> = rec[off_str..off_str + n_chars * 2].as_chunks::<2>().0.iter().map(|&p| u16::from_le_bytes(p)).collect();
            assert!(!units.contains(&0x0009), "raw tab U+0009 must never appear in EMR_EXTTEXTOUTW");
            texts.push(String::from_utf16(&units).expect("valid UTF-16LE"));
        } else if i_type == EMR_EXTCREATEFONTINDIRECTW {
            // LOGFONTW starts at offset 12 (8 header + 4 ihFont); lfEscapement is at offset 12 + 8 = 20.
            escapements.push(read_i32_le(rec, 20));
        }
    }
    assert!(texts.contains(&String::from("Group A    (n=12)")), "expected 4-space tab expansion, got {texts:?}");
    assert_eq!(escapements.len(), 2);
    assert_eq!(escapements[0], 2700, "90 deg CW must normalize to 2700 tenths of a degree");
    assert!((0..3600).contains(&escapements[1]), "1e12 deg must normalize into [0, 3600), got {}", escapements[1]);
}

#[test]
fn aud11_17_emf_push_utf16_fixed_does_not_split_surrogate_pair() {
    let mut scene = ChartScene::new("aud11-17", "Surrogate Font", 200.0, 100.0, Some(Rgba8::WHITE));
    // 30 ASCII chars + 1 non-BMP musical symbol (2 UTF-16 code units) lands right across lfFaceName[32] boundary.
    let face_name = format!("{}\u{1D11E}", "A".repeat(30));
    let mut span = RichSpan::plain("Sample");
    span.font_family = Some(face_name);
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![span]),
        anchor_pt: PointPt::new(20.0, 40.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        color: Rgba8::BLACK,
        rotation_deg: 0.0,
    }));

    let emf = encode_emf(&scene).expect("encode emf with non-BMP font family");
    let font_rec = iter_emf_records(&emf).into_iter().find(|(t, _)| *t == EMR_EXTCREATEFONTINDIRECTW).expect("font record").1;
    // lfFaceName[32] occupies bytes 40..104 (32 UTF-16LE units).
    let units: Vec<u16> = font_rec[40..104].as_chunks::<2>().0.iter().map(|&p| u16::from_le_bytes(p)).take_while(|&u| u != 0).collect();
    let decoded = String::from_utf16(&units).expect("lfFaceName must be valid UTF-16LE without lone surrogate");
    assert_eq!(decoded, "A".repeat(30));
    // Verify NUL termination at indices 30 and 31.
    assert_eq!(u16::from_le_bytes([font_rec[100], font_rec[101]]), 0);
    assert_eq!(u16::from_le_bytes([font_rec[102], font_rec[103]]), 0);
}
