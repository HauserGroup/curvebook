//! Regression tests for AUD-11 findings in `crates/pdf`:
//! `AUD-11-06`, `AUD-11-08`, `AUD-11-15`, `AUD-11-16`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{
    ChartScene, FillPaint, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, SceneRasterImage, StrokePaint, TextAnchor,
};
use curvebook_doc::{CropInsetsPt, ImageMimeKind};
use curvebook_palette::Rgba8;
use curvebook_pdf::{MAX_PDF_RAW_STREAM_BYTES, MAX_PDF_TOTAL_PATH_COMMANDS, PdfError, PdfExportOptions, render_pdf};
use curvebook_text::RichText;

#[test]
fn aud11_06_pdf_enforces_path_command_cap_and_rejects_oversized_embedded_ttf() {
    // 1. Single path exceeding MAX_PDF_TOTAL_PATH_COMMANDS fails fast with ResourceLimit.
    let mut huge_path = Path2D::new();
    huge_path.commands.reserve(MAX_PDF_TOTAL_PATH_COMMANDS + 1);
    huge_path.commands.push(PathCmd::MoveTo(PointPt::new(0.0, 0.0)));
    for _ in 0..MAX_PDF_TOTAL_PATH_COMMANDS {
        huge_path.commands.push(PathCmd::LineTo(PointPt::new(1.0, 1.0)));
    }
    let mut cmd_scene = ChartScene::new("pdf-cmds", "Huge Path", 100.0, 100.0, None);
    cmd_scene.primitives.push(ScenePrimitive::Path { path: huge_path, fill: None, stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)), clip: None });
    assert!(matches!(render_pdf(&cmd_scene, &PdfExportOptions::default()), Err(PdfError::ResourceLimit { .. })));

    // 2. Oversized embedded_truetype_font buffer fails fast with ResourceLimit before cloning/compressing.
    let empty_scene = ChartScene::new("pdf-big-ttf", "Big TTF", 100.0, 100.0, Some(Rgba8::WHITE));
    let mut oversized_ttf = vec![0u8; MAX_PDF_RAW_STREAM_BYTES + 1];
    oversized_ttf[0..4].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
    let big_opts = PdfExportOptions { embedded_truetype_font: Some(oversized_ttf), ..PdfExportOptions::default() };
    assert!(matches!(render_pdf(&empty_scene, &big_opts), Err(PdfError::ResourceLimit { .. })));
}

#[test]
fn aud11_08_extreme_finite_coordinates_never_emit_inf_or_nan_tokens_in_pdf() {
    let opts = PdfExportOptions { compress_streams: false, ..PdfExportOptions::default() };

    // 1. Overflowing TrimBox (`rect.x + rect.width` overflows `f64`) must return InvalidScene, never `inf`.
    let base_scene = ChartScene::new("pdf-trim-inf", "Trim Inf", 200.0, 100.0, Some(Rgba8::WHITE));
    let trim_opts = PdfExportOptions { trim_pt: Some(RectPt::new(1e308, 0.0, 1e308, 50.0)), ..opts.clone() };
    assert!(matches!(render_pdf(&base_scene, &trim_opts), Err(PdfError::InvalidScene { .. })));

    // 2. Overflowing BleedBox (`page_h - (rect.y + rect.height)` overflows `f64`) must return InvalidScene.
    let bleed_opts = PdfExportOptions { bleed_pt: Some(RectPt::new(0.0, 1e308, 50.0, 1e308)), ..opts.clone() };
    assert!(matches!(render_pdf(&base_scene, &bleed_opts), Err(PdfError::InvalidScene { .. })));

    // 3. QuadTo with ±1e308 previously overflowed `control.x - cur.x` to `inf` in degree elevation.
    let mut quad_scene = ChartScene::new("pdf-quad", "Quad", 200.0, 100.0, None);
    let mut quad_path = Path2D::new();
    quad_path.commands.push(PathCmd::MoveTo(PointPt::new(-1e308, 0.0)));
    quad_path.commands.push(PathCmd::QuadTo { control: PointPt::new(1e308, 0.0), to: PointPt::new(0.0, 0.0) });
    quad_scene.primitives.push(ScenePrimitive::Path { path: quad_path, fill: None, stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)), clip: None });
    match render_pdf(&quad_scene, &opts) {
        Ok(pdf) => {
            let text = String::from_utf8_lossy(&pdf);
            assert!(!text.contains("inf") && !text.contains("NaN"), "PDF stream must never contain inf/NaN tokens");
        }
        Err(PdfError::InvalidScene { .. }) => {}
        Err(other) => panic!("unexpected error: {other}"),
    }

    // 4. Text at extreme anchor coordinate where `anchor_pt.x + pen_u` or `page_h - y` overflows.
    let mut text_scene = ChartScene::new("pdf-text-inf", "Text Inf", 200.0, 100.0, None);
    text_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Overflow"),
        anchor_pt: PointPt::new(1e308, -1e308),
        font_size_pt: 1e308,
        anchor: TextAnchor::Middle,
        color: Rgba8::BLACK,
        rotation_deg: 0.0,
    }));
    assert!(matches!(render_pdf(&text_scene, &opts), Err(PdfError::InvalidScene { .. })));
    let outlined_opts = PdfExportOptions { outline_text: true, ..opts };
    match render_pdf(&text_scene, &outlined_opts) {
        Ok(pdf) => {
            let text = String::from_utf8_lossy(&pdf);
            assert!(!text.contains("inf") && !text.contains("NaN"), "outlined PDF stream must never contain inf/NaN tokens");
        }
        Err(PdfError::InvalidScene { .. }) => {}
        Err(other) => panic!("unexpected error: {other}"),
    }
}

#[test]
fn aud11_15_pdf_raster_dedup_preserves_distinct_and_deduplicates_identical_images() {
    let img_a = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255];
    let img_b = vec![10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255];
    let img_a_copy = img_a.clone();

    let mut scene = ChartScene::new("pdf-dedup", "Dedup", 200.0, 100.0, Some(Rgba8::WHITE));
    for (x, rgba) in [(10.0, img_a), (80.0, img_b), (140.0, img_a_copy)] {
        scene.primitives.push(ScenePrimitive::RasterImage(SceneRasterImage {
            asset_id: String::new(),
            media_type: ImageMimeKind::Png,
            pixel_size: (2, 2),
            rgba,
            bounds_pt: RectPt::new(x, 10.0, 40.0, 40.0),
            crop: CropInsetsPt::default(),
            rotation_deg: 0.0,
            clip: None,
        }));
    }

    let opts = PdfExportOptions { compress_streams: false, ..PdfExportOptions::default() };
    let pdf = render_pdf(&scene, &opts).expect("render PDF images");
    let text = String::from_utf8_lossy(&pdf);
    assert_eq!(text.matches("/Im0 Do").count(), 2, "identical image copies must deduplicate to /Im0");
    assert_eq!(text.matches("/Im1 Do").count(), 1, "distinct image must emit separate /Im1");
    assert!(!text.contains("/Im2 Do"), "no third image XObject expected");
}

#[test]
fn aud11_16_pdf_validates_stroke_width_inactive_paths_and_negative_clips_like_svg() {
    let opts = PdfExportOptions::default();

    // 1. NaN, +Inf, and negative stroke widths must fail with InvalidScene (not silently drop the path).
    for bad_width in [f64::NAN, f64::INFINITY, -2.0] {
        let mut scene = ChartScene::new("pdf-bad-stroke", "Bad Stroke", 100.0, 100.0, None);
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(10.0, 10.0), PointPt::new(90.0, 90.0)),
            fill: None,
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, bad_width)),
            clip: None,
        });
        assert!(matches!(render_pdf(&scene, &opts), Err(PdfError::InvalidScene { .. })), "PDF must reject invalid stroke width {bad_width}");
    }

    // 2. Zero-alpha path with NaN coordinate must fail validation (not be skipped silently).
    let mut nan_inactive = ChartScene::new("pdf-nan-inactive", "NaN Inactive", 100.0, 100.0, None);
    nan_inactive.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(f64::NAN, 10.0), PointPt::new(50.0, 50.0)),
        fill: Some(FillPaint::solid(Rgba8::TRANSPARENT)),
        stroke: None,
        clip: None,
    });
    assert!(matches!(render_pdf(&nan_inactive, &opts), Err(PdfError::InvalidScene { .. })));

    // 3. Zero-alpha path with negative clip dimension must fail validation.
    let mut neg_path_clip = ChartScene::new("pdf-neg-path-clip", "Neg Clip", 100.0, 100.0, None);
    neg_path_clip.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(10.0, 10.0), PointPt::new(50.0, 50.0)),
        fill: Some(FillPaint::solid(Rgba8::TRANSPARENT)),
        stroke: None,
        clip: Some(RectPt { x: 0.0, y: 0.0, width: -5.0, height: 10.0 }),
    });
    assert!(matches!(render_pdf(&neg_path_clip, &opts), Err(PdfError::InvalidScene { .. })));

    // 4. PushClip with negative width or height must fail validation (matching SVG).
    let mut neg_push_clip = ChartScene::new("pdf-neg-push-clip", "Neg PushClip", 100.0, 100.0, None);
    neg_push_clip.primitives.push(ScenePrimitive::PushClip { rect: RectPt { x: 0.0, y: 0.0, width: 10.0, height: -1.0 } });
    assert!(matches!(render_pdf(&neg_push_clip, &opts), Err(PdfError::InvalidScene { .. })));
}
