//! Regression tests for AUD-11 findings in `crates/svg`: `AUD-11-07` and `AUD-11-15`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_chart::{ChartScene, Path2D, PathCmd, PointPt, RectPt, ScenePrimitive, SceneRasterImage, StrokePaint};
use curvebook_doc::{CropInsetsPt, ImageMimeKind, MAX_IMAGE_DIM_PX};
use curvebook_palette::Rgba8;
use curvebook_svg::{MAX_SVG_PATH_COMMANDS, MAX_SVG_PRIMITIVES, SvgError, SvgExportOptions, render_svg, validate_svg_structure};

/// Modify the last 4 bytes of `buf` so `crc32fast::hash(buf) == target_crc` (exact IEEE CRC32 patch).
fn forge_trailing_4_bytes_for_crc32(buf: &mut [u8], target_crc: u32) {
    let n = buf.len();
    assert!(n >= 4);
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = u32::try_from(i).expect("u8");
        for _ in 0..8 {
            c = if (c & 1) != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
        *slot = c;
    }
    let mut s = target_crc ^ 0xFFFF_FFFF;
    let mut idxs = [0u8; 4];
    for i in (0..4).rev() {
        let top = s >> 24;
        let idx = table.iter().position(|&v| (v >> 24) == top).expect("unique top byte in CRC32 table");
        idxs[i] = u8::try_from(idx).expect("u8");
        s = (s ^ table[idx]) << 8;
    }
    let mut c = crc32fast::hash(&buf[..n - 4]) ^ 0xFFFF_FFFF;
    for i in 0..4 {
        let idx = idxs[i];
        buf[n - 4 + i] = ((c & 0xFF) as u8) ^ idx;
        c = (c >> 8) ^ table[usize::from(idx)];
    }
}

#[test]
fn aud11_07_svg_enforces_primitive_command_image_caps_and_deduplicates_stamps() {
    // 1. Oversized primitive count fails cleanly.
    let mut many_prims = ChartScene::new("svg-many", "Many", 100.0, 100.0, None);
    many_prims.primitives.resize(MAX_SVG_PRIMITIVES + 1, ScenePrimitive::PopClip);
    assert!(matches!(render_svg(&many_prims, &SvgExportOptions::default()), Err(SvgError::InvalidScene { .. })));

    // 2. Oversized single-path command count fails cleanly before allocating huge path data.
    let mut huge_path = Path2D::new();
    huge_path.commands.reserve(MAX_SVG_PATH_COMMANDS + 1);
    huge_path.commands.push(PathCmd::MoveTo(PointPt::new(0.0, 0.0)));
    for _ in 0..MAX_SVG_PATH_COMMANDS {
        huge_path.commands.push(PathCmd::LineTo(PointPt::new(1.0, 1.0)));
    }
    let mut cmd_scene = ChartScene::new("svg-cmds", "Cmds", 100.0, 100.0, None);
    cmd_scene.primitives.push(ScenePrimitive::Path { path: huge_path, fill: None, stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)), clip: None });
    assert!(matches!(render_svg(&cmd_scene, &SvgExportOptions::default()), Err(SvgError::InvalidScene { .. })));

    // 3. Oversized raster dimension fails cleanly before PNG encoding.
    let mut dim_scene = ChartScene::new("svg-dim", "Dim", 100.0, 100.0, None);
    dim_scene.primitives.push(ScenePrimitive::RasterImage(SceneRasterImage {
        asset_id: String::from("huge"),
        media_type: ImageMimeKind::Png,
        pixel_size: (MAX_IMAGE_DIM_PX + 1, 1),
        rgba: Vec::new(),
        bounds_pt: RectPt::new(0.0, 0.0, 10.0, 10.0),
        crop: CropInsetsPt::default(),
        rotation_deg: 0.0,
        clip: None,
    }));
    assert!(matches!(render_svg(&dim_scene, &SvgExportOptions::default()), Err(SvgError::InvalidScene { .. })));

    // 4. 40 repeated stamps of a 32x32 image emit the Base64 PNG ONCE in <defs> and reference via <use>.
    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32u8 {
        for x in 0..32u8 {
            rgba.extend_from_slice(&[x.wrapping_mul(7), y.wrapping_mul(9), x ^ y, 255]);
        }
    }
    let mut stamp_scene = ChartScene::new("svg-stamps", "Stamps", 400.0, 400.0, Some(Rgba8::WHITE));
    for i in 0..40u32 {
        stamp_scene.primitives.push(ScenePrimitive::RasterImage(SceneRasterImage {
            asset_id: String::from("tile"),
            media_type: ImageMimeKind::Png,
            pixel_size: (32, 32),
            rgba: rgba.clone(),
            bounds_pt: RectPt::new(f64::from(i) * 8.0, 10.0, 20.0, 20.0),
            crop: CropInsetsPt::default(),
            rotation_deg: 0.0,
            clip: None,
        }));
    }
    let svg = render_svg(&stamp_scene, &SvgExportOptions::default()).expect("render 40 stamps");
    validate_svg_structure(&svg).expect("valid SVG with <defs>/<symbol>/<use>");
    assert_eq!(svg.matches("<symbol id=\"img-").count(), 1, "repeated image must be defined once in <defs>");
    assert_eq!(svg.matches("<image ").count(), 1, "only one <image> definition expected");
    assert_eq!(svg.matches("<use ").count(), 40, "expected 40 compact <use> stamps");
    assert_eq!(
        svg.matches("data:image/png;base64,").count(),
        2,
        "Base64 URI appears once for xlink:href and once for href on the single <image> def"
    );
}

#[test]
fn aud11_15_svg_raster_dedup_verifies_byte_equality_on_crc32_collision() {
    let img_a: Vec<u8> = vec![
        255, 0, 0, 255, // red
        0, 255, 0, 255, // green
        0, 0, 255, 255, // blue
        255, 255, 0, 255, // yellow
    ];
    let target_crc = crc32fast::hash(&img_a);
    let mut img_b: Vec<u8> = vec![
        10, 20, 30, 255, // distinct pixels
        40, 50, 60, 255, 70, 80, 90, 255, 0, 0, 0, 0, // overwritten by forge
    ];
    forge_trailing_4_bytes_for_crc32(&mut img_b, target_crc);
    assert_ne!(img_a, img_b, "precondition: buffers must differ");
    assert_eq!(crc32fast::hash(&img_a), crc32fast::hash(&img_b), "precondition: exact CRC32 collision");

    let mut scene = ChartScene::new("svg-crc-collide", "CRC Collision", 200.0, 100.0, Some(Rgba8::WHITE));
    for (x, rgba) in [(10.0, img_a), (80.0, img_b)] {
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

    let svg = render_svg(&scene, &SvgExportOptions::default()).expect("render colliding SVG images");
    validate_svg_structure(&svg).expect("valid svg");
    assert_eq!(svg.matches("<symbol id=\"img-").count(), 2, "distinct images with identical CRC32 must NOT collide in cache");
    assert!(svg.contains("xlink:href=\"#img-0\"") && svg.contains("xlink:href=\"#img-1\""), "both distinct images must be referenced");
}
