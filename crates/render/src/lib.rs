//! Deterministic CPU rasterizer and compositor for LumenCraft graphs and multi-figure publication
//! layouts.
//!
//! Layer L3 rendering crate. Governing spec: `docs/graphing-and-layouts.md` (§1, §6).
#![forbid(unsafe_code)]

pub mod png;
pub mod rasterizer;
pub mod tiff;

use serde::{Deserialize, Serialize};

pub use png::encode_png;
pub use rasterizer::{RasterImage, RenderError, render_scene};
pub use tiff::{TiffCompression, encode_tiff};

/// Output pixel dimensions and dots-per-inch metadata for rendered scientific figures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderTargetSpec {
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
}

#[cfg(test)]
mod tests {
    use lumencraft_chart::{ChartScene, Path2D, PointPt, ScenePrimitive, StrokePaint};
    use lumencraft_palette::Rgba8;

    use super::{RenderTargetSpec, TiffCompression, encode_png, encode_tiff, render_scene};

    #[test]
    fn publication_dpi_target_constructs() {
        let spec = RenderTargetSpec { width_px: 2400, height_px: 1800, dpi: 600 };
        assert_eq!(spec.dpi, 600);
    }

    #[test]
    fn rasterize_png_and_tiff_smoke() {
        let mut scene = ChartScene::new("graph-1", "Preview", 120.0, 90.0, Some(Rgba8::WHITE));
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(10.0, 10.0), PointPt::new(100.0, 80.0)),
            fill: None,
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
            clip: None,
        });
        let img = render_scene(&scene, RenderTargetSpec { width_px: 120, height_px: 90, dpi: 300 }).expect("rasterize");
        let png = encode_png(&img).expect("png");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        let tiff = encode_tiff(&img, TiffCompression::Deflate).expect("tiff");
        assert!(tiff.starts_with(b"II*\0"));
    }
}
