//! Regression tests for audit P4 (`FA-SEC10-03`, `FA-SEC10-06`, `FA-SEC10-08`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ChartScene, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor};
use curvebook_palette::Rgba8;
use curvebook_render::{
    MAX_CAPPED_RASTER_DIM_PX, MAX_EMF_PRIMITIVES, MAX_RASTER_PIXELS, RasterImage, RenderError, RenderTargetSpec, encode_emf, encode_raster_emf,
    render_scene,
};
use curvebook_text::{RichSpan, RichText, SpanWeight};

const EMR_EXTTEXTOUTW: u32 = 84;

fn read_u32_le(bytes: &[u8], off: usize) -> u32 {
    let arr: Option<[u8; 4]> = bytes.get(off..off + 4).and_then(|s| s.try_into().ok());
    assert!(arr.is_some(), "u32_le offset {off} out of bounds");
    u32::from_le_bytes(arr.unwrap_or([0; 4]))
}

fn decode_emf_texts(emf: &[u8]) -> Vec<String> {
    let mut texts = Vec::new();
    let mut off = 0usize;
    while off + 8 <= emf.len() {
        let rec_type = read_u32_le(emf, off);
        let rec_size = read_u32_le(emf, off + 4) as usize;
        if rec_size < 8 || off + rec_size > emf.len() {
            break;
        }
        if rec_type == EMR_EXTTEXTOUTW && rec_size >= 76 {
            let n_chars = read_u32_le(emf, off + 44) as usize;
            let off_str = read_u32_le(emf, off + 48) as usize;
            if off_str + n_chars * 2 <= rec_size
                && let Some(utf16_bytes) = emf.get(off + off_str..off + off_str + n_chars * 2)
            {
                let units: Vec<u16> = utf16_bytes.as_chunks::<2>().0.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
                texts.push(String::from_utf16(&units).expect("UTF-16LE in EMR_EXTTEXTOUTW"));
            }
        }
        off += rec_size;
    }
    texts
}

fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> Path2D {
    let k = 0.552_284_749_830_793_6;
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(cx + rx, cy)),
            PathCmd::CubicTo { c1: PointPt::new(cx + rx, cy + k * ry), c2: PointPt::new(cx + k * rx, cy + ry), to: PointPt::new(cx, cy + ry) },
            PathCmd::CubicTo { c1: PointPt::new(cx - k * rx, cy + ry), c2: PointPt::new(cx - rx, cy + k * ry), to: PointPt::new(cx - rx, cy) },
            PathCmd::CubicTo { c1: PointPt::new(cx - rx, cy - k * ry), c2: PointPt::new(cx - k * rx, cy - ry), to: PointPt::new(cx, cy - ry) },
            PathCmd::CubicTo { c1: PointPt::new(cx + k * rx, cy - ry), c2: PointPt::new(cx + rx, cy - k * ry), to: PointPt::new(cx + rx, cy) },
            PathCmd::Close,
        ],
    }
}

fn dark_pixel_bbox_size(img: &RasterImage) -> (u32, u32) {
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (img.width_px, 0u32, img.height_px, 0u32);
    for y in 0..img.height_px {
        for x in 0..img.width_px {
            let idx = ((y * img.width_px + x) * 4) as usize;
            if img.rgba.get(idx).is_some_and(|&r| r < 200) {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    (max_x.saturating_sub(min_x), max_y.saturating_sub(min_y))
}

#[test]
fn sec10_03_from_scene_capped_preserves_aspect_ratio_and_effective_dpi() {
    // Wide panoramic scene exceeds MAX_CAPPED_RASTER_DIM_PX on width; height and dpi must scale uniformly.
    let wide_scene = ChartScene::new("wide", "Wide", 2000.0, 200.0, None);
    let wide = RenderTargetSpec::from_scene_capped(&wide_scene, None, None, 600);
    assert_eq!(wide.width_px, MAX_CAPPED_RASTER_DIM_PX);
    assert_eq!(wide.height_px, 819);
    let wide_ratio = f64::from(wide.width_px) / f64::from(wide.height_px);
    assert!((wide_ratio - 10.0).abs() < 0.05, "aspect ratio distorted: {wide_ratio}");
    let expected_wide_dpi = ((f64::from(wide.width_px) * 72.0) / 2000.0).round() as u16;
    assert_eq!(wide.dpi, expected_wide_dpi);

    // Tall skinny scene exceeds MAX_CAPPED_RASTER_DIM_PX on height.
    let tall_scene = ChartScene::new("tall", "Tall", 100.0, 3000.0, None);
    let tall = RenderTargetSpec::from_scene_capped(&tall_scene, None, None, 600);
    assert_eq!(tall.height_px, MAX_CAPPED_RASTER_DIM_PX);
    assert_eq!(tall.width_px, 273);
    let expected_tall_dpi = ((f64::from(tall.height_px) * 72.0) / 3000.0).round() as u16;
    assert_eq!(tall.dpi, expected_tall_dpi);

    // Extreme aspect ratio does not panic on clamp bounds and remains proportional.
    let strip_scene = ChartScene::new("strip", "Strip", 2000.0, 10.0, None);
    let strip = RenderTargetSpec::from_scene_capped(&strip_scene, None, None, 600);
    assert_eq!(strip.width_px, MAX_CAPPED_RASTER_DIM_PX);
    assert_eq!(strip.height_px, 41);

    // Square scene obeys pixel area cap while remaining 1:1.
    let sq_scene = ChartScene::new("sq", "Square", 3000.0, 3000.0, None);
    let square = RenderTargetSpec::from_scene_capped(&sq_scene, None, None, 600);
    assert!(u64::from(square.width_px) * u64::from(square.height_px) <= MAX_RASTER_PIXELS);
    assert_eq!(square.width_px, square.height_px);

    // Single-axis overrides maintain uniform scaling and compute effective DPI accurately.
    let rect_scene = ChartScene::new("rect", "Rect", 400.0, 200.0, None);
    let ov_w = RenderTargetSpec::from_scene_capped(&rect_scene, Some(1200), None, 300);
    assert_eq!((ov_w.width_px, ov_w.height_px), (1200, 600));
    assert_eq!(ov_w.dpi, 216);

    let ov_h = RenderTargetSpec::from_scene_capped(&rect_scene, None, Some(500), 300);
    assert_eq!((ov_h.width_px, ov_h.height_px), (1000, 500));
    assert_eq!(ov_h.dpi, 180);
}

#[test]
fn sec10_06_emf_rejects_zero_dpi_enforces_primitive_budget_and_reorders_rtl_spans() {
    // 1. Zero DPI rejected with InvalidTarget.
    let img = RasterImage { width_px: 2, height_px: 2, dpi: 0, rgba: vec![255; 16] };
    assert!(matches!(encode_raster_emf(&img), Err(RenderError::InvalidTarget { .. })));

    // 2. Primitive budget enforced before unbounded allocation/encoding.
    let mut giant = ChartScene::new("emf-budget", "Giant", 100.0, 100.0, None);
    let tiny_line = ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(1.0, 1.0), PointPt::new(2.0, 2.0)),
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    };
    giant.primitives = vec![tiny_line; MAX_EMF_PRIMITIVES + 1];
    assert!(matches!(encode_emf(&giant), Err(RenderError::EncodingFailed { .. })));

    // 3. Multi-span BiDi RTL reordering in EMF emits visual LTR order (`2026` before `שלום`).
    let mut rtl_scene = ChartScene::new("emf-rtl", "RTL MultiSpan", 300.0, 120.0, Some(Rgba8::WHITE));
    rtl_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![RichSpan { weight: SpanWeight::Bold, ..RichSpan::plain("שלום") }, RichSpan::plain(" 2026")]),
        anchor_pt: PointPt::new(20.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let emf = encode_emf(&rtl_scene).expect("encode rtl emf");
    let texts = decode_emf_texts(&emf);
    let joined = texts.join("|");
    let pos_digits = joined.find("2026").expect("missing digits in EMF output");
    // Reordering reverses Hebrew characters into visual LTR order ("םולש") after "2026 ".
    let pos_hebrew = joined.find("םולש").expect("missing visual Hebrew run in EMF output");
    assert!(pos_digits < pos_hebrew, "expected visual digits before visual Hebrew in EMF, got `{joined}`");
}

#[test]
fn sec10_08_stroked_near_circle_ellipse_preserves_anisotropy() {
    let mut scene = ChartScene::new("ellipse", "Near Circle", 200.0, 200.0, Some(Rgba8::WHITE));
    // 4% eccentricity (previously collapsed into an isotropic circle).
    scene.primitives.push(ScenePrimitive::Path {
        path: ellipse_path(100.0, 100.0, 80.0, 76.8),
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 2.0)),
        clip: None,
    });

    // At 288 DPI (4x pt->px), diameter difference is 2 * (80 - 76.8) * 4 = 25.6 px.
    let spec = RenderTargetSpec::from_scene_dpi(&scene, 288).expect("spec");
    let img = render_scene(&scene, spec).expect("render near-circle ellipse");
    let (bbox_w, bbox_h) = dark_pixel_bbox_size(&img);
    assert!(bbox_w > bbox_h + 15, "4%-eccentric stroked ellipse must not flatten into a circle: bbox_w={bbox_w}, bbox_h={bbox_h}");

    // Exact circle (rx == ry) still renders symmetrically.
    let mut circle_scene = ChartScene::new("circle", "Exact Circle", 200.0, 200.0, Some(Rgba8::WHITE));
    circle_scene.primitives.push(ScenePrimitive::Path {
        path: ellipse_path(100.0, 100.0, 80.0, 80.0),
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 2.0)),
        clip: Some(RectPt::new(0.0, 0.0, 200.0, 200.0)),
    });
    let circle_img = render_scene(&circle_scene, spec).expect("render circle");
    let (cw, ch) = dark_pixel_bbox_size(&circle_img);
    assert!(cw.abs_diff(ch) <= 2);
}
