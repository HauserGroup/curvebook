//! Regression tests for Stage 4B S09 typography & layout fixes in `curvebook-render`.

use curvebook_chart::{ChartScene, PointPt, PositionedText, ScenePrimitive, TextAnchor};
use curvebook_palette::Rgba8;
use curvebook_render::{RasterImage, RenderTargetSpec, render_scene};
use curvebook_text::RichText;

fn dark_pixel_row_bounds(img: &RasterImage, y_min: u32, y_max: u32) -> Option<(u32, u32)> {
    let mut min_x = u32::MAX;
    let mut max_x = 0u32;
    let mut any = false;
    for y in y_min..y_max.min(img.height_px) {
        for x in 0..img.width_px {
            let idx = ((y * img.width_px + x) * 4) as usize;
            if img.rgba[idx] < 200 {
                any = true;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
        }
    }
    any.then_some((min_x, max_x))
}

#[test]
fn r1_s09_06_rasterizer_multiline_middle_anchor_centers_each_line_independently() {
    let mut single_line = ChartScene::new("g", "Single", 240.0, 120.0, Some(Rgba8::WHITE));
    single_line.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("MMMM"),
        anchor_pt: PointPt::new(120.0, 40.0),
        font_size_pt: 16.0,
        anchor: TextAnchor::Middle,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let mut multi_line = ChartScene::new("g", "Multi", 240.0, 120.0, Some(Rgba8::WHITE));
    multi_line.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("MMMM\nIIIIII"),
        anchor_pt: PointPt::new(120.0, 40.0),
        font_size_pt: 16.0,
        anchor: TextAnchor::Middle,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let spec = RenderTargetSpec { width_px: 240, height_px: 120, dpi: 72 };
    let img_single = render_scene(&single_line, spec).expect("raster single");
    let img_multi = render_scene(&multi_line, spec).expect("raster multi");

    let line0_single = dark_pixel_row_bounds(&img_single, 20, 45).expect("single ink");
    let line0_multi = dark_pixel_row_bounds(&img_multi, 20, 45).expect("line 0 ink");
    let line1_multi = dark_pixel_row_bounds(&img_multi, 45, 70).expect("line 1 ink");

    // Line 0 of multiline must match single-line centered bounds identically (no summed multi-line advance).
    assert_eq!(line0_multi, line0_single);
    let line1_mid = (line1_multi.0 + line1_multi.1) as f64 * 0.5;
    assert!((line1_mid - 120.0).abs() <= 2.5, "second line must center near x=120, got {line1_mid}");
}

#[test]
fn r1_s09_08_rasterizer_renders_rtl_text_in_visual_bidi_order() {
    let mut logical_scene = ChartScene::new("g", "RasterBiDi", 240.0, 80.0, Some(Rgba8::WHITE));
    logical_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("שלום 2026"),
        anchor_pt: PointPt::new(20.0, 50.0),
        font_size_pt: 16.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let mut digits_scene = ChartScene::new("g", "RasterDigits", 240.0, 80.0, Some(Rgba8::WHITE));
    digits_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("2026"),
        anchor_pt: PointPt::new(20.0, 50.0),
        font_size_pt: 16.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    // Logical "שלום 2026" must place visual "2026" in the leftmost glyph slots starting at x=20,
    // with Hebrew glyphs drawn to the right of "2026".
    let spec = RenderTargetSpec { width_px: 240, height_px: 80, dpi: 72 };
    let img_logical = render_scene(&logical_scene, spec).expect("raster logical");
    let img_digits = render_scene(&digits_scene, spec).expect("raster digits");
    let (_, digits_max_x) = dark_pixel_row_bounds(&img_digits, 20, 70).expect("digits ink");

    for y in 0..80 {
        for x in 0..=digits_max_x {
            let idx = ((y * 240 + x) * 4) as usize;
            assert_eq!(img_logical.rgba[idx], img_digits.rgba[idx], "mismatch at ({x}, {y}) inside visual '2026' prefix");
        }
    }
    let after_digits = dark_pixel_row_bounds(&img_logical, 20, 70).expect("logical ink");
    assert!(after_digits.1 > digits_max_x + 10, "Hebrew glyphs must render to the right of '2026'");
}
