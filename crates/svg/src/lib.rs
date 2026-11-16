//! Publication-quality SVG figure and multi-panel layout export for CurveBook.
//!
//! Layer L3 crate. Governing spec: `docs/graphing-and-layouts.md` (§6) and `docs/file-format-parity.md`.
#![forbid(unsafe_code)]

pub mod writer;

pub use writer::{SvgError, SvgExportOptions, render_svg, validate_svg_structure};

/// Default SVG XML namespace URI.
pub const SVG_XMLNS: &str = "http://www.w3.org/2000/svg";

#[cfg(test)]
mod tests {
    use curvebook_chart::{ChartScene, Path2D, PointPt, ScenePrimitive, StrokePaint};
    use curvebook_palette::Rgba8;

    use super::{SVG_XMLNS, SvgExportOptions, render_svg, validate_svg_structure};

    #[test]
    fn svg_namespace_is_standard() {
        assert!(SVG_XMLNS.starts_with("http://"));
    }

    #[test]
    fn render_and_validate_svg_roundtrip() {
        let mut scene = ChartScene::new("graph-1", "Dose Response", 240.0, 180.0, Some(Rgba8::WHITE));
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(10.0, 10.0), PointPt::new(100.0, 90.0)),
            fill: None,
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
            clip: None,
        });
        let svg = render_svg(&scene, &SvgExportOptions::default()).expect("render svg");
        validate_svg_structure(&svg).expect("valid svg");
        assert!(svg.contains("<path"));
    }
}
