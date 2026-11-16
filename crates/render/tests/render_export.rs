//! End-to-end tests for deterministic CPU rasterization, PNG `IHDR`/`pHYs`/`IDAT` encoding,
//! and baseline TIFF 6.0 (`None`, `Deflate`, `Lzw`) export across `72..=1200` DPI.

use std::io::Read;

use crc32fast::Hasher;
use flate2::read::ZlibDecoder;
use lumencraft_chart::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor,
    compile_graph, compile_layout,
};
use lumencraft_doc::{
    DataSheet, GraphSheet, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, RectPt as DocRectPt, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use lumencraft_palette::{LineDashStyle, OKABE_ITO_BLUE, OKABE_ITO_VERMILION, Rgba8};
use lumencraft_render::{RasterImage, RenderError, RenderTargetSpec, TiffCompression, encode_png, encode_tiff, render_scene};
use lumencraft_text::parse_markup;

struct DecodedPng {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    phys_x_ppm: u32,
    phys_y_ppm: u32,
    phys_unit: u8,
    rgba: Vec<u8>,
}

fn read_u32_be(bytes: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

fn decode_and_verify_png(bytes: &[u8]) -> DecodedPng {
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "missing PNG signature");
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut phys_x_ppm = 0u32;
    let mut phys_y_ppm = 0u32;
    let mut phys_unit = 0u8;
    let mut saw_ihdr = false;
    let mut saw_phys = false;
    let mut saw_iend = false;
    let mut idat = Vec::new();

    while pos < bytes.len() {
        let len = read_u32_be(bytes, pos) as usize;
        let chunk_type: [u8; 4] = [bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]];
        let data_start = pos + 8;
        let data_end = data_start + len;
        let data = &bytes[data_start..data_end];
        let expected_crc = read_u32_be(bytes, data_end);

        let mut hasher = Hasher::new();
        hasher.update(&chunk_type);
        hasher.update(data);
        assert_eq!(hasher.finalize(), expected_crc, "CRC mismatch on PNG chunk {:?}", std::str::from_utf8(&chunk_type));

        match &chunk_type {
            b"IHDR" => {
                assert_eq!(len, 13);
                width = read_u32_be(data, 0);
                height = read_u32_be(data, 4);
                bit_depth = data[8];
                color_type = data[9];
                assert_eq!(data[10], 0, "compression method must be 0");
                assert_eq!(data[11], 0, "filter method must be 0");
                assert_eq!(data[12], 0, "interlace method must be 0");
                saw_ihdr = true;
            }
            b"pHYs" => {
                assert_eq!(len, 9);
                phys_x_ppm = read_u32_be(data, 0);
                phys_y_ppm = read_u32_be(data, 4);
                phys_unit = data[8];
                saw_phys = true;
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => {
                assert_eq!(len, 0);
                saw_iend = true;
            }
            _ => {}
        }
        pos = data_end + 4;
    }

    assert!(saw_ihdr && saw_phys && saw_iend, "missing required PNG chunks");
    assert_eq!(pos, bytes.len());

    let mut decoder = ZlibDecoder::new(idat.as_slice());
    let mut raw = Vec::new();
    assert!(decoder.read_to_end(&mut raw).is_ok(), "zlib decode IDAT failed");

    let stride = (width as usize) * 4;
    assert_eq!(raw.len(), (stride + 1) * (height as usize));
    let mut rgba = vec![0u8; stride * (height as usize)];

    for y in 0..(height as usize) {
        let src_row = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let filter = src_row[0];
        let filt_bytes = &src_row[1..];
        let (prev_rows, curr_rows) = rgba.split_at_mut(y * stride);
        let curr = &mut curr_rows[..stride];
        let prev = if y > 0 { &prev_rows[(y - 1) * stride..] } else { &[] };

        assert!(filter <= 4, "unexpected PNG filter byte {filter}");
        for x in 0..stride {
            let a = if x >= 4 { curr[x - 4] } else { 0 };
            let b = if prev.is_empty() { 0 } else { prev[x] };
            let c = if prev.is_empty() || x < 4 { 0 } else { prev[x - 4] };
            let recon = match filter {
                0 => filt_bytes[x],
                1 => filt_bytes[x].wrapping_add(a),
                2 => filt_bytes[x].wrapping_add(b),
                3 => filt_bytes[x].wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                _ => filt_bytes[x].wrapping_add(paeth_predictor(a, b, c)),
            };
            curr[x] = recon;
        }
    }

    DecodedPng { width, height, bit_depth, color_type, phys_x_ppm, phys_y_ppm, phys_unit, rgba }
}

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let ia = i32::from(a);
    let ib = i32::from(b);
    let ic = i32::from(c);
    let p = ia + ib - ic;
    let pa = (p - ia).abs();
    let pb = (p - ib).abs();
    let pc = (p - ic).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

struct DecodedTiff {
    width: u32,
    height: u32,
    compression: u16,
    x_res_num: u32,
    x_res_den: u32,
    y_res_num: u32,
    y_res_den: u32,
    res_unit: u16,
    extra_samples: u16,
    rgba: Vec<u8>,
}

fn read_u16_le(bytes: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([bytes[off], bytes[off + 1]])
}

fn read_u32_le(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

fn decode_and_verify_tiff(bytes: &[u8]) -> DecodedTiff {
    assert_eq!(&bytes[0..2], b"II", "little-endian TIFF header expected");
    assert_eq!(read_u16_le(bytes, 2), 42, "TIFF magic 42 expected");
    let ifd_off = read_u32_le(bytes, 4) as usize;
    let count = read_u16_le(bytes, ifd_off) as usize;
    assert!(count >= 13, "expected baseline IFD tags");

    let mut prev_tag = 0u16;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut compression = 0u16;
    let mut strip_off = 0usize;
    let mut strip_bytes = 0usize;
    let mut x_res = (0u32, 0u32);
    let mut y_res = (0u32, 0u32);
    let mut res_unit = 0u16;
    let mut extra_samples = 0u16;
    let mut samples_per_pixel = 0u16;
    let mut photometric = 0u16;
    let mut bps_off = 0usize;

    for i in 0..count {
        let entry = ifd_off + 2 + i * 12;
        let tag = read_u16_le(bytes, entry);
        assert!(tag > prev_tag, "IFD tags must be strictly sorted ascending: {prev_tag} -> {tag}");
        prev_tag = tag;
        let ty = read_u16_le(bytes, entry + 2);
        let cnt = read_u32_le(bytes, entry + 4);
        let val_u32 = read_u32_le(bytes, entry + 8);
        let val_u16 = read_u16_le(bytes, entry + 8);

        match tag {
            256 => width = val_u32,
            257 => height = val_u32,
            258 => {
                assert_eq!(ty, 3);
                assert_eq!(cnt, 4);
                bps_off = val_u32 as usize;
            }
            259 => compression = val_u16,
            262 => photometric = val_u16,
            273 => strip_off = val_u32 as usize,
            277 => samples_per_pixel = val_u16,
            278 => assert_eq!(val_u32, height),
            279 => strip_bytes = val_u32 as usize,
            282 => {
                assert_eq!(ty, 5);
                let off = val_u32 as usize;
                x_res = (read_u32_le(bytes, off), read_u32_le(bytes, off + 4));
            }
            283 => {
                assert_eq!(ty, 5);
                let off = val_u32 as usize;
                y_res = (read_u32_le(bytes, off), read_u32_le(bytes, off + 4));
            }
            296 => res_unit = val_u16,
            338 => extra_samples = val_u16,
            _ => {}
        }
    }

    assert_eq!(read_u32_le(bytes, ifd_off + 2 + count * 12), 0, "single IFD expected");
    assert_eq!(photometric, 2);
    assert_eq!(samples_per_pixel, 4);
    assert_eq!(res_unit, 2);
    assert_eq!(extra_samples, 2);
    for ch in 0..4 {
        assert_eq!(read_u16_le(bytes, bps_off + ch * 2), 8);
    }

    let strip = &bytes[strip_off..strip_off + strip_bytes];
    assert!(matches!(compression, 1 | 5 | 8), "unexpected TIFF compression code {compression}");
    let rgba = match compression {
        1 => strip.to_vec(),
        8 => {
            let mut dec = ZlibDecoder::new(strip);
            let mut out = Vec::new();
            assert!(dec.read_to_end(&mut out).is_ok(), "decode TIFF deflate strip failed");
            out
        }
        _ => decode_tiff_lzw(strip),
    };

    DecodedTiff {
        width,
        height,
        compression,
        x_res_num: x_res.0,
        x_res_den: x_res.1,
        y_res_num: y_res.0,
        y_res_den: y_res.1,
        res_unit,
        extra_samples,
        rgba,
    }
}

/// Clean-room TIFF 6.0 MSB-first LZW decoder implementing TIFF early-change code widths (9..=12 bits).
fn decode_tiff_lzw(encoded: &[u8]) -> Vec<u8> {
    struct BitReader<'a> {
        data: &'a [u8],
        byte_pos: usize,
        bit_buf: u32,
        bit_count: u8,
    }
    impl<'a> BitReader<'a> {
        fn next_code(&mut self, bits: u8) -> Option<u16> {
            while self.bit_count < bits {
                let &b = self.data.get(self.byte_pos)?;
                self.byte_pos += 1;
                self.bit_buf = (self.bit_buf << 8) | u32::from(b);
                self.bit_count += 8;
            }
            self.bit_count -= bits;
            let mask = (1u32 << bits) - 1;
            Some(((self.bit_buf >> self.bit_count) & mask) as u16)
        }
    }

    let mut reader = BitReader { data: encoded, byte_pos: 0, bit_buf: 0, bit_count: 0 };
    let mut table: Vec<Vec<u8>> = (0..=255u16).map(|b| vec![b as u8]).collect();
    table.push(Vec::new()); // 256 Clear
    table.push(Vec::new()); // 257 EOI
    let mut bits = 9u8;
    let mut old_code: Option<u16> = None;
    let mut out = Vec::new();

    while let Some(code) = reader.next_code(bits) {
        if code == 257 {
            break;
        }
        if code == 256 {
            table.truncate(258);
            bits = 9;
            old_code = None;
            continue;
        }
        if let Some(prev) = old_code {
            let entry = if usize::from(code) < table.len() {
                let s = table[usize::from(code)].clone();
                let mut combined = table[usize::from(prev)].clone();
                combined.push(s[0]);
                table.push(combined);
                s
            } else {
                assert_eq!(usize::from(code), table.len(), "corrupt LZW code {code}");
                let mut combined = table[usize::from(prev)].clone();
                combined.push(combined[0]);
                table.push(combined.clone());
                combined
            };
            out.extend_from_slice(&entry);
            match table.len() {
                511 => bits = 10,
                1023 => bits = 11,
                2047 => bits = 12,
                _ => {}
            }
        } else {
            let s = table[usize::from(code)].clone();
            out.extend_from_slice(&s);
        }
        old_code = Some(code);
    }

    out
}

fn pixel_at(img: &RasterImage, x: u32, y: u32) -> Rgba8 {
    let idx = ((y as usize) * (img.width_px as usize) + (x as usize)) * 4;
    Rgba8::rgba(img.rgba[idx], img.rgba[idx + 1], img.rgba[idx + 2], img.rgba[idx + 3])
}

#[test]
fn validates_bounds_and_prevents_overflow_or_oom() {
    let scene = ChartScene::new("s1", "Bounds", 100.0, 100.0, Some(Rgba8::WHITE));
    for spec in [
        RenderTargetSpec { width_px: 0, height_px: 100, dpi: 72 },
        RenderTargetSpec { width_px: 100, height_px: 0, dpi: 72 },
        RenderTargetSpec { width_px: 100, height_px: 100, dpi: 0 },
        RenderTargetSpec { width_px: 16_385, height_px: 16, dpi: 300 },
        RenderTargetSpec { width_px: 16, height_px: u32::MAX, dpi: 300 },
    ] {
        let err = render_scene(&scene, spec).unwrap_err();
        assert!(matches!(err, RenderError::InvalidTarget { .. }), "expected InvalidTarget, got {err:?}");
    }

    let mut bad_scene = scene.clone();
    bad_scene.width_pt = f64::NAN;
    assert!(matches!(render_scene(&bad_scene, RenderTargetSpec { width_px: 100, height_px: 100, dpi: 72 }), Err(RenderError::InvalidTarget { .. })));

    let mismatch_img = RasterImage { width_px: 4, height_px: 4, dpi: 72, rgba: vec![0; 12] };
    assert!(matches!(encode_png(&mismatch_img), Err(RenderError::BufferSizeMismatch { expected: 64, actual: 12 })));
    assert!(matches!(encode_tiff(&mismatch_img, TiffCompression::Lzw), Err(RenderError::BufferSizeMismatch { expected: 64, actual: 12 })));
}

#[test]
fn png_and_tiff_roundtrip_across_72_to_1200_dpi() {
    let mut scene = ChartScene::new("s-dpi", "DPI Sweep", 72.0, 54.0, Some(Rgba8::WHITE));
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(8.0, 8.0, 24.0, 18.0)),
        fill: Some(FillPaint::solid(OKABE_ITO_BLUE)),
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.5)),
        clip: None,
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(10.0, 44.0)),
                PathCmd::CubicTo { c1: PointPt::new(26.0, 12.0), c2: PointPt::new(46.0, 48.0), to: PointPt::new(64.0, 16.0) },
            ],
        },
        fill: None,
        stroke: Some(StrokePaint {
            color: OKABE_ITO_VERMILION,
            width_pt: 2.0,
            dash: LineDashStyle::Dashed,
            cap: LineCap::Round,
            join: LineJoin::Round,
        }),
        clip: None,
    });

    for dpi in [72u16, 150, 300, 600, 1200] {
        // Keep physical size proportional at 0.5x0.375 inch so 1200 DPI renders rapidly in unit tests (600x450 px).
        let scale_in = 0.5;
        let w_px = (f64::from(dpi) * scale_in) as u32;
        let h_px = (f64::from(dpi) * 0.375) as u32;
        let img = render_scene(&scene, RenderTargetSpec { width_px: w_px, height_px: h_px, dpi }).expect("render");

        let png = encode_png(&img).expect("png");
        let decoded_png = decode_and_verify_png(&png);
        assert_eq!(decoded_png.width, w_px);
        assert_eq!(decoded_png.height, h_px);
        assert_eq!(decoded_png.bit_depth, 8);
        assert_eq!(decoded_png.color_type, 6);
        assert_eq!(decoded_png.phys_unit, 1);
        let expected_ppm = (f64::from(dpi) / 0.0254).round() as u32;
        assert_eq!(decoded_png.phys_x_ppm, expected_ppm);
        assert_eq!(decoded_png.phys_y_ppm, expected_ppm);
        assert_eq!(decoded_png.rgba, img.rgba, "PNG pixel round-trip mismatch at {dpi} DPI");

        let tiff_none = encode_tiff(&img, TiffCompression::None).expect("tiff none");
        let tiff_deflate = encode_tiff(&img, TiffCompression::Deflate).expect("tiff deflate");
        let tiff_lzw = encode_tiff(&img, TiffCompression::Lzw).expect("tiff lzw");

        assert!(tiff_lzw.len() < tiff_none.len(), "LZW should compress figure payload at {dpi} DPI");
        assert!(tiff_deflate.len() < tiff_none.len(), "Deflate should compress figure payload at {dpi} DPI");

        for (mode, bytes, expected_code) in [("none", &tiff_none, 1u16), ("deflate", &tiff_deflate, 8u16), ("lzw", &tiff_lzw, 5u16)] {
            let dec = decode_and_verify_tiff(bytes);
            assert_eq!(dec.width, w_px);
            assert_eq!(dec.height, h_px);
            assert_eq!(dec.compression, expected_code);
            assert_eq!(dec.x_res_num, u32::from(dpi));
            assert_eq!(dec.x_res_den, 1);
            assert_eq!(dec.y_res_num, u32::from(dpi));
            assert_eq!(dec.y_res_den, 1);
            assert_eq!(dec.res_unit, 2);
            assert_eq!(dec.extra_samples, 2);
            assert_eq!(dec.rgba, img.rgba, "TIFF ({mode}) pixel mismatch at {dpi} DPI");
        }
    }
}

#[test]
fn tiff_lzw_handles_dictionary_boundaries_and_clear_resets() {
    // Exercise every small byte length across the 9->10 bit transition (`next_code` 510/511/512)
    // plus larger sizes hitting 11->12 bit transitions and Clear resets at 4094.
    let mut lengths: Vec<usize> = (4..=1200).step_by(4).collect();
    lengths.extend([2_048, 4_096, 8_192, 16_384, 32_768]);
    for size in lengths {
        let mut rgba = Vec::with_capacity(size);
        for i in 0..size {
            let v = ((i.wrapping_mul(37)) ^ (i >> 3)) as u8;
            rgba.push(v);
        }
        let w = (size / 4) as u32;
        let img = RasterImage { width_px: w, height_px: 1, dpi: 300, rgba: rgba.clone() };
        let tiff = encode_tiff(&img, TiffCompression::Lzw).expect("lzw encode");
        let dec = decode_and_verify_tiff(&tiff);
        assert_eq!(dec.rgba, rgba, "LZW mismatch for byte length {size}");
    }
}

#[test]
fn rasterizes_caps_joins_quad_curves_aa_and_no_join_double_blend() {
    // Verify LineCap geometry (Butt vs Square vs Round).
    let mut cap_scene = ChartScene::new("caps", "Caps", 80.0, 80.0, None);
    for (y, cap) in [(20.0, LineCap::Butt), (40.0, LineCap::Square), (60.0, LineCap::Round)] {
        cap_scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(20.0, y), PointPt::new(50.0, y)),
            fill: None,
            stroke: Some(StrokePaint { color: Rgba8::BLACK, width_pt: 10.0, dash: LineDashStyle::Solid, cap, join: LineJoin::Miter }),
            clip: None,
        });
    }
    let cap_img = render_scene(&cap_scene, RenderTargetSpec { width_px: 80, height_px: 80, dpi: 72 }).expect("render caps");

    // Butt stops at x=20.
    assert_eq!(pixel_at(&cap_img, 17, 20), Rgba8::TRANSPARENT);
    assert_eq!(pixel_at(&cap_img, 22, 20), Rgba8::BLACK);
    // Square extends to [15..20] across full height [35..45].
    assert_eq!(pixel_at(&cap_img, 17, 40), Rgba8::BLACK);
    assert_eq!(pixel_at(&cap_img, 16, 36), Rgba8::BLACK);
    assert_eq!(pixel_at(&cap_img, 14, 40), Rgba8::TRANSPARENT);
    // Round extends to x=17 at centerline, anti-aliases at (16, 56), and is clear at (15, 55) outside radius 5.
    assert_eq!(pixel_at(&cap_img, 17, 60), Rgba8::BLACK);
    let round_aa = pixel_at(&cap_img, 16, 56).a;
    assert!(round_aa > 0 && round_aa < 255, "expected partial SSAA coverage at round cap corner, got {round_aa}");
    assert_eq!(pixel_at(&cap_img, 15, 55), Rgba8::TRANSPARENT);

    // Verify LineJoin (Miter vs Bevel vs Round) and zero double-blending on semi-transparent join.
    for (join, expect_outer_54_36, expect_mid_53_37) in
        [(LineJoin::Miter, true, true), (LineJoin::Round, false, true), (LineJoin::Bevel, false, false)]
    {
        let mut join_scene = ChartScene::new("join", "Join", 80.0, 80.0, None);
        join_scene.primitives.push(ScenePrimitive::Path {
            path: Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(20.0, 40.0)),
                    PathCmd::LineTo(PointPt::new(50.0, 40.0)),
                    PathCmd::LineTo(PointPt::new(50.0, 70.0)),
                ],
            },
            fill: None,
            stroke: Some(StrokePaint { color: Rgba8::rgba(200, 50, 0, 128), width_pt: 10.0, dash: LineDashStyle::Solid, cap: LineCap::Butt, join }),
            clip: None,
        });
        let img = render_scene(&join_scene, RenderTargetSpec { width_px: 80, height_px: 80, dpi: 72 }).expect("render join");
        assert_eq!(pixel_at(&img, 50, 40), Rgba8::rgba(200, 50, 0, 128), "join {join:?} must not double-blend");
        assert_eq!(pixel_at(&img, 54, 36).a > 0, expect_outer_54_36, "outer corner pixel check failed for {join:?}");
        assert_eq!(pixel_at(&img, 53, 37).a > 0, expect_mid_53_37, "mid corner pixel check failed for {join:?}");
    }

    // Verify QuadTo curve + primitive clip + 4x4 SSAA anti-aliasing.
    let mut quad_scene = ChartScene::new("quad", "Quad", 64.0, 64.0, Some(Rgba8::WHITE));
    quad_scene.primitives.push(ScenePrimitive::Path {
        path: Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(4.0, 56.0)),
                PathCmd::QuadTo { control: PointPt::new(32.0, 4.0), to: PointPt::new(60.0, 56.0) },
            ],
        },
        fill: None,
        stroke: Some(StrokePaint { color: Rgba8::BLACK, width_pt: 2.0, dash: LineDashStyle::DashDot, cap: LineCap::Round, join: LineJoin::Round }),
        clip: Some(RectPt::new(8.0, 8.0, 48.0, 48.0)),
    });
    let quad_img = render_scene(&quad_scene, RenderTargetSpec { width_px: 64, height_px: 64, dpi: 72 }).expect("render quad");
    // Outside primitive clip must stay pure white.
    assert_eq!(pixel_at(&quad_img, 5, 55), Rgba8::WHITE);
    // Curve inside clip must exhibit anti-aliased intermediate gray pixels (0 < r < 255).
    let aa_pixels = quad_img.rgba.as_chunks::<4>().0.iter().filter(|px| px[0] > 0 && px[0] < 255).count();
    assert!(aa_pixels > 10, "expected 4x4 SSAA intermediate edge pixels on QuadTo curve, got {aa_pixels}");
}

#[test]
fn rasterizes_alpha_ribbons_curves_dashes_winding_holes_and_clips() {
    // Transparent background (`None`) + semi-transparent ribbon + polygon hole + clip stack + dashes.
    let mut scene = ChartScene::new("alpha-clip", "Alpha & Clip", 100.0, 100.0, None);

    // 1. Semi-transparent ribbon over transparent canvas must keep alpha == 128 and pure RGB!
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(10.0, 10.0, 30.0, 30.0)),
        fill: Some(FillPaint::solid(Rgba8::rgba(0, 120, 200, 128))),
        stroke: None,
        clip: None,
    });

    // 2. Concentric outer CCW + inner CW hole: center pixel must remain transparent.
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D {
            commands: vec![
                // Outer square [50..90, 10..50]
                PathCmd::MoveTo(PointPt::new(50.0, 10.0)),
                PathCmd::LineTo(PointPt::new(50.0, 50.0)),
                PathCmd::LineTo(PointPt::new(90.0, 50.0)),
                PathCmd::LineTo(PointPt::new(90.0, 10.0)),
                PathCmd::Close,
                // Inner hole [60..80, 20..40] opposite winding
                PathCmd::MoveTo(PointPt::new(60.0, 20.0)),
                PathCmd::LineTo(PointPt::new(80.0, 20.0)),
                PathCmd::LineTo(PointPt::new(80.0, 40.0)),
                PathCmd::LineTo(PointPt::new(60.0, 40.0)),
                PathCmd::Close,
            ],
        },
        fill: Some(FillPaint::solid(Rgba8::rgb(200, 40, 40))),
        stroke: None,
        clip: None,
    });

    // 3. Clipped stroke via PushClip/PopClip + primitive clip.
    scene.primitives.push(ScenePrimitive::PushClip { rect: RectPt::new(10.0, 60.0, 30.0, 20.0) });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(0.0, 50.0, 100.0, 50.0)),
        fill: Some(FillPaint::solid(Rgba8::rgb(0, 160, 80))),
        stroke: None,
        clip: None,
    });
    scene.primitives.push(ScenePrimitive::PopClip);

    // 4. Dashed polyline below clip.
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(10.0, 90.0), PointPt::new(90.0, 90.0)),
        fill: None,
        stroke: Some(StrokePaint { color: Rgba8::BLACK, width_pt: 2.0, dash: LineDashStyle::Dashed, cap: LineCap::Butt, join: LineJoin::Miter }),
        clip: None,
    });

    let img = render_scene(&scene, RenderTargetSpec { width_px: 100, height_px: 100, dpi: 72 }).expect("render");

    // Outside any primitive: transparent.
    assert_eq!(pixel_at(&img, 2, 2), Rgba8::TRANSPARENT);
    // Inside translucent ribbon over transparent canvas: straight-alpha preserved!
    assert_eq!(pixel_at(&img, 25, 25), Rgba8::rgba(0, 120, 200, 128));
    // Donut ring vs transparent hole center.
    assert_eq!(pixel_at(&img, 55, 30), Rgba8::rgb(200, 40, 40));
    assert_eq!(pixel_at(&img, 70, 30), Rgba8::TRANSPARENT);
    // Clip region inside vs clipped outside.
    assert_eq!(pixel_at(&img, 20, 70), Rgba8::rgb(0, 160, 80));
    assert_eq!(pixel_at(&img, 50, 70), Rgba8::TRANSPARENT);
    assert_eq!(pixel_at(&img, 20, 55), Rgba8::TRANSPARENT);

    // Dashed line at y=90 must have both opaque black pixels (dashes) and transparent gaps.
    let mut black_cols = 0usize;
    let mut gap_cols = 0usize;
    for x in 12..88 {
        let p = pixel_at(&img, x, 90);
        if p == Rgba8::BLACK {
            black_cols += 1;
        } else if p == Rgba8::TRANSPARENT {
            gap_cols += 1;
        }
    }
    assert!(black_cols > 15 && gap_cols > 10, "expected visible dash/gap cadence, got black={black_cols}, gap={gap_cols}");
}

#[test]
fn rasterizes_rich_and_rotated_text_and_compiled_chart_scenes() {
    // Test rotated & centered text outlines (works identically with and without bundled-fonts).
    let mut text_scene = ChartScene::new("text-rot", "Text Rotation", 160.0, 160.0, Some(Rgba8::WHITE));
    text_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("IC<sub>50</sub> = 2.4 ± 0.3 µM"),
        anchor_pt: PointPt::new(80.0, 36.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    text_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("10<sup>-6</sup>"),
        anchor_pt: PointPt::new(28.0, 110.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: -90.0,
        color: OKABE_ITO_BLUE,
    }));

    let img = render_scene(&text_scene, RenderTargetSpec { width_px: 320, height_px: 320, dpi: 144 }).expect("render text");

    // Measure horizontal vs vertical bounding extents of the two labels.
    let mut horiz_xs = (u32::MAX, 0u32);
    let mut horiz_ys = (u32::MAX, 0u32);
    let mut vert_xs = (u32::MAX, 0u32);
    let mut vert_ys = (u32::MAX, 0u32);

    for y in 0..img.height_px {
        for x in 0..img.width_px {
            let p = pixel_at(&img, x, y);
            if p != Rgba8::WHITE {
                if y < 120 {
                    horiz_xs = (horiz_xs.0.min(x), horiz_xs.1.max(x));
                    horiz_ys = (horiz_ys.0.min(y), horiz_ys.1.max(y));
                } else if x < 100 {
                    vert_xs = (vert_xs.0.min(x), vert_xs.1.max(x));
                    vert_ys = (vert_ys.0.min(y), vert_ys.1.max(y));
                }
            }
        }
    }

    assert!(horiz_xs.1 > horiz_xs.0 && horiz_ys.1 > horiz_ys.0, "horizontal rich label produced no pixels");
    assert!((horiz_xs.1 - horiz_xs.0) > (horiz_ys.1 - horiz_ys.0), "horizontal label should be wider than tall");
    assert!(vert_ys.1 > vert_ys.0 && vert_xs.1 > vert_xs.0, "rotated -90° label produced no pixels");
    assert!((vert_ys.1 - vert_ys.0) > (vert_xs.1 - vert_xs.0), "rotated -90° label should be taller than wide");

    // Compile real ChartScenes via `lumencraft_chart` and verify non-blank deterministic exports.
    let mut doc = ProjectDocument::default();
    let archetype = TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 8).expect("xy table"));
    let data = DataSheet::new(SheetId::new("data-1"), "Dose Response", archetype).expect("datasheet");
    let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-1"), &data).expect("graph");
    graph.spec.title = String::from("<b>Dose</b> Curve");
    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: DocRectPt::new(16.0, 16.0, 220.0, 160.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    layout_spec.items.push(LayoutItem::TextCallout {
        id: String::from("caption-1"),
        markdown: String::from("<b>Fig. 1</b> Dose Curve"),
        bounds_pt: DocRectPt::new(250.0, 24.0, 140.0, 60.0),
        font_size_pt: 11.0,
    });
    let layout = LayoutSheet::new(SheetId::new("layout-1"), "Figure 1", layout_spec).expect("layout");
    doc.add_data_sheet(data).expect("add data");
    doc.add_graph_sheet(graph.clone()).expect("add graph");
    doc.add_layout_sheet(layout.clone()).expect("add layout");

    let g_scene = compile_graph(&graph, &doc).expect("compile graph");
    let l_scene = compile_layout(&layout, &doc).expect("compile layout");

    for compiled in [&g_scene, &l_scene] {
        let r1 = render_scene(compiled, RenderTargetSpec { width_px: 360, height_px: 260, dpi: 300 }).expect("render compiled");
        let r2 = render_scene(compiled, RenderTargetSpec { width_px: 360, height_px: 260, dpi: 300 }).expect("render deterministic");
        assert_eq!(r1, r2, "rendering must be byte-deterministic");
        let non_white = r1.rgba.as_chunks::<4>().0.iter().filter(|px| *px != &[255, 255, 255, 255]).count();
        assert!(non_white > 50, "compiled scene '{}' rendered nearly blank ({non_white} pixels)", compiled.title);
    }
}

#[test]
fn cld_compiled_graph_rasterizes_deterministically_into_rgba_annotation_band() {
    use lumencraft_doc::{
        AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, CompactLetterDisplaySpec, OneWayAnovaSpec, PairwiseComparisonRecord,
        PairwiseDisplayMode, ResultSheet, ResultStatus,
    };

    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("column table");
    table.data_sets[0].title = String::from("Ctrl");
    table.data_sets[1].title = String::from("Mid");
    table.data_sets[2].title = String::from("High");
    for (c, base) in [(0, 10.0), (1, 14.0), (2, 22.0)] {
        table.data_sets[c].cells =
            vec![vec![Cell::number(base - 1.0)], vec![Cell::number(base)], vec![Cell::number(base + 1.0)], vec![Cell::number(base)]];
    }
    let ds = DataSheet::new(SheetId::new("col-cld"), "Treatment", TableArchetype::Column(table)).expect("ds");
    let spec =
        AnalysisSpec::new(SheetId::new("res-cld"), SheetId::new("col-cld"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("spec");
    let mut res = ResultSheet::new(spec).expect("res");
    res.status = ResultStatus::UpToDate;
    res.pairwise_comparisons = vec![
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("Ctrl"),
            label_b: String::from("Mid"),
            estimate_diff: Some(-4.0),
            raw_p_or_q: 0.14,
            is_fdr_q: false,
            significant: false,
        },
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 2,
            label_a: String::from("Ctrl"),
            label_b: String::from("High"),
            estimate_diff: Some(-12.0),
            raw_p_or_q: 0.001,
            is_fdr_q: false,
            significant: true,
        },
        PairwiseComparisonRecord {
            group_a: 1,
            group_b: 2,
            label_a: String::from("Mid"),
            label_b: String::from("High"),
            estimate_diff: Some(-8.0),
            raw_p_or_q: 0.12,
            is_fdr_q: false,
            significant: false,
        },
    ];

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-cld"), &ds).expect("gs");
    gs.spec.title = String::from("CLD RGBA Figure");
    gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res-cld")));
    gs.spec.pairwise_display_mode = PairwiseDisplayMode::CompactLetterDisplayOnly;

    let mut gs_ctl = gs.clone();
    gs_ctl.spec.pairwise_display_mode = PairwiseDisplayMode::BracketsOnly;
    gs_ctl.spec.compact_letter_display = None;

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds).expect("add ds");
    doc.add_result_sheet(res).expect("add res");
    doc.add_graph_sheet(gs.clone()).expect("add gs");

    let scene_cld = compile_graph(&gs, &doc).expect("compile cld scene");
    let scene_ctl = compile_graph(&gs_ctl, &doc).expect("compile control scene");

    let target = RenderTargetSpec { width_px: 480, height_px: 340, dpi: 150 };
    let img_1 = render_scene(&scene_cld, target).expect("render cld 1");
    let img_2 = render_scene(&scene_cld, target).expect("render cld 2");
    assert_eq!(img_1, img_2, "CLD RGBA render must be byte-deterministic");
    assert_ne!(img_1, render_scene(&scene_ctl, target).expect("render control"), "CLD render must differ from non-CLD control");

    let png = encode_png(&img_1).expect("encode cld png");
    let decoded = decode_and_verify_png(&png);
    assert_eq!(decoded.width, target.width_px);
    assert_eq!(decoded.height, target.height_px);
    assert_eq!(decoded.rgba, img_1.rgba);

    let sx = f64::from(target.width_px) / scene_cld.width_pt;
    let sy = f64::from(target.height_px) / scene_cld.height_pt;
    for label in ["b", "ab", "a"] {
        let pos = scene_cld
            .primitives
            .iter()
            .find_map(|p| match p {
                ScenePrimitive::Text(t) if t.rich.plain_text() == label => Some(t.anchor_pt),
                _ => None,
            })
            .expect("CLD text primitive");
        let x0 = ((pos.x - 10.0) * sx).floor().max(0.0) as u32;
        let x1 = ((pos.x + 10.0) * sx).ceil().min(f64::from(target.width_px - 1)) as u32;
        let y0 = ((pos.y - 12.0) * sy).floor().max(0.0) as u32;
        let y1 = ((pos.y + 3.0) * sy).ceil().min(f64::from(target.height_px - 1)) as u32;
        let mut ink_pixels = 0usize;
        for py in y0..=y1 {
            for px in x0..=x1 {
                if pixel_at(&img_1, px, py) != Rgba8::WHITE {
                    ink_pixels += 1;
                }
            }
        }
        assert!(ink_pixels >= 4, "expected rasterized glyph ink around CLD label `{label}` in [{x0}..={x1}, {y0}..={y1}], got {ink_pixels}");
    }
}
