//! Regression tests for audit P4 (`FA-SEC10-07`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use curvebook_chart::{ChartScene, PointPt, PositionedText, ScenePrimitive, TextAnchor};
use curvebook_palette::Rgba8;
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use curvebook_text::{RichSpan, RichText};

#[test]
fn sec10_07_svg_text_font_family_escaping_and_valid_xml_structure() {
    let mut scene = ChartScene::new("svg-sec10-07", "Title with ]]> & <tag>", 300.0, 120.0, Some(Rgba8::WHITE));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![RichSpan { font_family: Some(String::from("Serif]]>\"'<&")), ..RichSpan::plain("Sample Label") }]),
        anchor_pt: PointPt::new(20.0, 60.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    for embed_fonts in [false, true] {
        let svg =
            render_svg(&scene, &SvgExportOptions { outline_text: false, embed_fonts, ..SvgExportOptions::default() }).expect("render editable svg");
        validate_svg_structure(&svg).expect("editable svg must remain well-formed XML");
        assert!(svg.contains("<text"), "expected <text element");
        assert!(svg.contains("Sample Label"), "expected text content");
        assert!(!svg.contains("Serif]]>"), "raw ]]> must never appear unescaped");
    }
}
