//! Deterministic CPU rasterizer and compositor for CurveBook graphs and multi-figure publication
//! layouts.
//!
//! Layer L3 rendering crate. Governing spec: `docs/implementation/graphing-and-layouts.md` (§1, §6).
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

pub mod emf;
pub mod png;
pub mod rasterizer;
pub mod tiff;

use serde::{Deserialize, Serialize};

pub use emf::{MAX_EMF_OUTPUT_BYTES, MAX_EMF_PRIMITIVES, encode_emf, encode_emf_with_raster, encode_raster_emf};
pub use png::encode_png;
pub use rasterizer::{
    DEFAULT_CAPPED_RASTER_HEIGHT_PX, DEFAULT_CAPPED_RASTER_WIDTH_PX, MAX_CAPPED_RASTER_DIM_PX, MAX_RASTER_DIM_PX, MAX_RASTER_PIXELS,
    MIN_CAPPED_RASTER_DIM_PX, RasterImage, RenderError, RenderOptions, RgbaPixmap, apply_canvas_preview_filter, render_scene,
    render_scene_with_options,
};
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
    use curvebook_chart::{ChartScene, Path2D, PointPt, ScenePrimitive, StrokePaint};
    use curvebook_palette::Rgba8;

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

    #[test]
    fn transparent_bg_raster_png_and_tiff_emit_zero_alpha_corners_and_preserve_foreground() {
        use curvebook_chart::{FillPaint, RectPt};

        use super::{RenderOptions, render_scene_with_options};

        let mut scene = ChartScene::new("graph-trans", "Transparent", 100.0, 80.0, Some(Rgba8::WHITE));
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(RectPt::new(30.0, 20.0, 40.0, 40.0)),
            fill: Some(FillPaint::solid(Rgba8::rgb(20, 40, 200))),
            stroke: None,
            clip: None,
        });
        let target = RenderTargetSpec { width_px: 100, height_px: 80, dpi: 72 };

        // Opaque default: corner (0,0) is opaque white.
        let opaque = render_scene_with_options(&scene, target, RenderOptions { transparent_bg: false }).expect("opaque render");
        assert_eq!(opaque.rgba.get(0..4), Some(&[255, 255, 255, 255][..]));

        // Transparent mode: corner (0,0) has alpha=0, foreground center (50,40) is opaque blue.
        let trans = render_scene_with_options(&scene, target, RenderOptions { transparent_bg: true }).expect("transparent render");
        assert_eq!(trans.rgba.get(0..4), Some(&[0, 0, 0, 0][..]), "top-left corner must be fully transparent");
        let center_idx = (40 * 100 + 50) * 4;
        assert_eq!(trans.rgba.get(center_idx..center_idx + 4), Some(&[20, 40, 200, 255][..]), "foreground rect must remain opaque");

        // Decoded PNG preserves alpha=0 corner and alpha=255 foreground.
        let png_bytes = encode_png(&trans).expect("encode transparent png");
        let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = decoder.read_info().expect("read png info");
        let mut decoded = vec![0u8; reader.output_buffer_size().expect("png output buffer size")];
        let info = reader.next_frame(&mut decoded).expect("decode png frame");
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(decoded.get(0..4), Some(&[0, 0, 0, 0][..]), "decoded PNG corner alpha must be 0");
        assert_eq!(decoded.get(center_idx..center_idx + 4), Some(&[20, 40, 200, 255][..]), "decoded PNG foreground must be preserved");

        // Uncompressed TIFF preserves alpha=0 corner and alpha=255 foreground in its strip.
        let tiff_bytes = encode_tiff(&trans, TiffCompression::None).expect("encode transparent tiff");
        assert!(tiff_bytes.starts_with(b"II*\0"));
        let strip_offset = 32usize + 2 + 13 * 12 + 4;
        assert_eq!(tiff_bytes.get(strip_offset..strip_offset + 4), Some(&[0, 0, 0, 0][..]), "TIFF corner pixel alpha must be 0");
        assert_eq!(
            tiff_bytes.get(strip_offset + center_idx..strip_offset + center_idx + 4),
            Some(&[20, 40, 200, 255][..]),
            "TIFF foreground pixel must be preserved"
        );
    }
}
