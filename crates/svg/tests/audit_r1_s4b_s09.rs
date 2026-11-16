//! Regression tests for Stage 4B S09 typography & layout fixes in `curvebook-svg`.

use curvebook_chart::{ChartScene, PointPt, PositionedText, ScenePrimitive, TextAnchor};
use curvebook_palette::Rgba8;
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use curvebook_text::{RichText, parse_markup};

fn path_x_bounds(svg: &str) -> Option<(f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut any = false;
    for chunk in svg.split(" d=\"").skip(1) {
        let Some((d, _)) = chunk.split_once('"') else {
            continue;
        };
        let mut coord_idx = 0usize;
        for cmd in d.split(' ') {
            if cmd.is_empty() || cmd == "Z" {
                continue;
            }
            if cmd.starts_with(['M', 'L', 'Q', 'C']) {
                coord_idx = 0;
            }
            let raw = cmd.trim_start_matches(['M', 'L', 'Q', 'C']);
            if let Ok(val) = raw.parse::<f64>() {
                if coord_idx.is_multiple_of(2) {
                    any = true;
                    min_x = min_x.min(val);
                    max_x = max_x.max(val);
                }
                coord_idx += 1;
            }
        }
    }
    any.then_some((min_x, max_x))
}

#[test]
fn r1_s09_06_svg_multiline_editable_and_outlined_split_lines_accurately() {
    let mut scene = ChartScene::new("g", "MultilineSvg", 300.0, 200.0, Some(Rgba8::WHITE));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>Line one</b>\r\nLine two\n\nLine four"),
        anchor_pt: PointPt::new(150.0, 60.0),
        font_size_pt: 10.0,
        anchor: TextAnchor::Middle,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let editable = render_svg(&scene, &SvgExportOptions { outline_text: false, ..SvgExportOptions::default() }).expect("render editable svg");
    validate_svg_structure(&editable).expect("valid editable svg");
    assert_eq!(editable.matches("<tspan").count(), 3);
    assert!(editable.contains("x=\"150.000\""), "line breaks must reset x anchor: {editable}");
    assert!(editable.contains("dy=\"12.000\""), "1.20 * 10pt line leading expected: {editable}");
    assert!(editable.contains("dy=\"24.000\""), "blank middle line must preserve 2.40 * 10pt jump: {editable}");
    assert!(!editable.contains("Line one\n"), "raw newline must not stay inside <tspan>");

    // Outlined SVG aligns each line independently: line 0 width must equal "MMMM" alone even with long line 1.
    let mut m_only = ChartScene::new("g", "M", 300.0, 200.0, Some(Rgba8::WHITE));
    m_only.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("MMMM"),
        anchor_pt: PointPt::new(150.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Middle,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let mut multi = ChartScene::new("g", "Multi", 300.0, 200.0, Some(Rgba8::WHITE));
    multi.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("MMMM\nI"),
        anchor_pt: PointPt::new(150.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Middle,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let svg_m = render_svg(&m_only, &SvgExportOptions { outline_text: true, ..SvgExportOptions::default() }).expect("outlined M");
    let svg_multi = render_svg(&multi, &SvgExportOptions { outline_text: true, ..SvgExportOptions::default() }).expect("outlined multi");
    validate_svg_structure(&svg_multi).expect("valid outlined multi svg");

    let (m_min, m_max) = path_x_bounds(&svg_m).expect("M path bounds");
    let (multi_min, multi_max) = path_x_bounds(&svg_multi).expect("multi path bounds");
    // Line 1 ('I') sits strictly inside line 0 ('MMMM') horizontally when per-line middle-anchored.
    assert!((multi_min - m_min).abs() < 0.05 && (multi_max - m_max).abs() < 0.05);
}

#[test]
fn r1_s09_08_svg_outlined_text_applies_bidi_order() {
    let mut bidi_scene = ChartScene::new("g", "SvgBiDi", 300.0, 120.0, Some(Rgba8::WHITE));
    bidi_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("שלום 2026"),
        anchor_pt: PointPt::new(50.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let mut digits_scene = ChartScene::new("g", "SvgDigits", 300.0, 120.0, Some(Rgba8::WHITE));
    digits_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("2026"),
        anchor_pt: PointPt::new(50.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let opts = SvgExportOptions { outline_text: true, ..SvgExportOptions::default() };
    let bidi_svg = render_svg(&bidi_scene, &opts).expect("render bidi svg");
    let digits_svg = render_svg(&digits_scene, &opts).expect("render digits svg");
    validate_svg_structure(&bidi_svg).expect("valid bidi svg");

    let first_digit_path = digits_svg.lines().find(|l| l.trim_start().starts_with("<path ")).expect("digit path");
    assert!(bidi_svg.contains(first_digit_path.trim()), "visual LTR digit path must appear identically at start of outlined RTL string");
}
