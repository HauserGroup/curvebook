#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{ErrorCode, GraphAnnotation, LayoutItem, SheetId};
use curvebook_engine::{CommandOutcome, ExternalPathPolicy, Session};
use curvebook_pdf::{PdfColorSpace, PdfExportOptions, render_pdf};
use curvebook_render::{RenderTargetSpec, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::{Value, json};

static SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    root: PathBuf,
    workdir: PathBuf,
    outside: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-r13-7-eng-{label}-{}-{id}", std::process::id()));
        let workdir = root.join("workdir");
        let outside = root.join("outside");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&workdir).expect("create workdir");
        fs::create_dir_all(&outside).expect("create outside");
        Self { root, workdir, outside }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(*chunk.first().unwrap_or(&0));
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        let encode = |shift: usize| TABLE.get(((n >> shift) & 0x3F) as usize).copied().unwrap_or(b'A') as char;
        out.push(encode(18));
        out.push(encode(12));
        out.push(if chunk.len() > 1 { encode(6) } else { '=' });
        out.push(if chunk.len() > 2 { encode(0) } else { '=' });
    }
    out
}

fn decode_base64(input: &str) -> Vec<u8> {
    fn val(b: u8) -> u8 {
        match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::new();
    for chunk in clean.as_chunks::<4>().0 {
        let v0 = u32::from(val(chunk[0]));
        let v1 = u32::from(val(chunk[1]));
        if chunk[2] == b'=' {
            out.push((((v0 << 18) | (v1 << 12)) >> 16) as u8);
        } else if chunk[3] == b'=' {
            let v2 = u32::from(val(chunk[2]));
            let t = (v0 << 18) | (v1 << 12) | (v2 << 6);
            out.push((t >> 16) as u8);
            out.push((t >> 8) as u8);
        } else {
            let v2 = u32::from(val(chunk[2]));
            let v3 = u32::from(val(chunk[3]));
            let t = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
            out.push((t >> 16) as u8);
            out.push((t >> 8) as u8);
            out.push(t as u8);
        }
    }
    out
}

fn encode_quadrant_png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| {
        let right = x >= w / 2;
        let bottom = y >= h / 2;
        match (right, bottom) {
            (false, false) => Rgba([220, 30, 30, 140]), // semi-transparent red top-left
            (true, false) => Rgba([30, 180, 40, 255]),  // opaque green top-right
            (false, true) => Rgba([30, 60, 220, 255]),  // opaque blue bottom-left
            (true, true) => Rgba([230, 190, 20, 180]),  // semi-transparent yellow bottom-right
        }
    });
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Png).expect("encode png");
    out
}

fn encode_rgb_jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(w, h, |x, y| Rgb([(x * 3) as u8, 120, (y * 4) as u8]));
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg).expect("encode jpeg");
    out
}

fn encode_rgba_tiff(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| Rgba([40, (x * 5) as u8, (y * 7) as u8, 220]));
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Tiff).expect("encode tiff");
    out
}

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

fn seed_session_with_graphs_and_layout() -> Session {
    let mut s = Session::new();
    ok_cmd(&mut s, "sheet.createData", json!({ "sheet_id": "xy-1", "archetype": "xy", "cols": 1, "rows": 3 }));
    ok_cmd(&mut s, "table.setXCells", json!({ "sheet_id": "xy-1", "values": [1.0, 2.0, 3.0] }));
    ok_cmd(
        &mut s,
        "table.setCells",
        json!({
            "sheet_id": "xy-1",
            "cells": [
                { "row": 0, "col": 0, "value": 10.0 },
                { "row": 1, "col": 0, "value": 25.0 },
                { "row": 2, "col": 0, "value": 40.0 }
            ]
        }),
    );
    ok_cmd(&mut s, "graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "kind": "xy" }));
    ok_cmd(&mut s, "graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "xy-1", "kind": "xy" }));
    ok_cmd(&mut s, "layout.create", json!({ "sheet_id": "lay-1", "title": "Composite Fig" }));
    ok_cmd(
        &mut s,
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "panel_id": "panel-graph", "kind": "graph", "graph_sheet_id": "g-2", "bounds_pt": [20.0, 20.0, 180.0, 130.0] }),
    );
    s
}

#[test]
fn five_image_commands_sources_dedup_aspect_crop_border_zorder_and_undo() {
    let mut s = seed_session_with_graphs_and_layout();
    let dir = TempDirGuard::new("cmds");
    let png_bytes = encode_quadrant_png(80, 60);
    let png_b64 = encode_base64(&png_bytes);
    let jpg_bytes = encode_rgb_jpeg(64, 48);
    let jpg_path = dir.workdir.join("blot.jpg");
    fs::write(&jpg_path, &jpg_bytes).unwrap();
    let tif_bytes = encode_rgba_tiff(32, 32);
    let tif_b64 = encode_base64(&tif_bytes);

    // 1. graph.insertImage from base64 (alpha PNG, 80x60)
    let r1 = ok_cmd(
        &mut s,
        "graph.insertImage",
        json!({
            "sheet_id": "g-1",
            "bytes_base64": png_b64,
            "bounds_pt": [20.0, 20.0, 80.0, 60.0],
            "border": { "width_pt": 1.5, "color": "#112233", "dash": "dashed" }
        }),
    );
    let asset_png_id = r1.payload["asset_id"].as_str().unwrap().to_string();
    assert_eq!(r1.payload["media_type"], "image/png");
    assert_eq!(r1.payload["pixel_size"], json!([80, 60]));
    assert_eq!(s.document().image_assets.len(), 1);

    // 2. graph.insertImage from regular file path (JPEG)
    let r2 = ok_cmd(
        &mut s,
        "graph.insertImage",
        json!({
            "sheet_id": "g-1",
            "path": jpg_path.to_string_lossy(),
            "bounds_pt": [120.0, 25.0, 64.0, 48.0]
        }),
    );
    assert_eq!(r2.payload["media_type"], "image/jpeg");
    assert_eq!(s.document().image_assets.len(), 2);

    // 3. graph.updateImage with lock_aspect_ratio=true then false, crop, rotation, and border:null
    let ann_idx = r1.payload["annotation_index"].as_u64().unwrap();
    ok_cmd(&mut s, "graph.updateImage", json!({ "sheet_id": "g-1", "annotation_index": ann_idx, "width_pt": 40.0 }));
    let g1 = s.document().graph_sheet(&SheetId::new("g-1")).unwrap();
    let GraphAnnotation::EmbeddedImage { bounds_pt, .. } = &g1.spec.annotations[ann_idx as usize] else {
        panic!("expected EmbeddedImage");
    };
    assert!((bounds_pt.width - 40.0).abs() < 1e-9);
    assert!((bounds_pt.height - 30.0).abs() < 1e-9, "aspect ratio must lock 80x60 -> 40x30");

    ok_cmd(
        &mut s,
        "graph.updateImage",
        json!({
            "sheet_id": "g-1",
            "annotation_index": ann_idx,
            "lock_aspect_ratio": false,
            "width_pt": 100.0,
            "rotation_deg": 22.5,
            "crop": { "left_pt": 4.0, "right_pt": 6.0, "top_pt": 2.0, "bottom_pt": 4.0 },
            "border": null
        }),
    );
    let g1_after = s.document().graph_sheet(&SheetId::new("g-1")).unwrap();
    let GraphAnnotation::EmbeddedImage { bounds_pt, rotation_deg, lock_aspect_ratio, crop, border, .. } =
        &g1_after.spec.annotations[ann_idx as usize]
    else {
        panic!("expected EmbeddedImage");
    };
    assert!(!*lock_aspect_ratio);
    assert!((bounds_pt.width - 100.0).abs() < 1e-9);
    assert!((bounds_pt.height - 30.0).abs() < 1e-9, "unlocked aspect preserves height");
    assert!((*rotation_deg - 22.5).abs() < 1e-9);
    assert!((crop.left_pt - 4.0).abs() < 1e-9 && (crop.bottom_pt - 4.0).abs() < 1e-9);
    assert!(border.is_none(), "border: null must clear existing border");

    // 4. layout.insertImage reusing existing asset_id (deduplicates instead of creating duplicate asset)
    let l1 = ok_cmd(
        &mut s,
        "layout.insertImage",
        json!({
            "sheet_id": "lay-1",
            "panel_id": "img-reused",
            "asset_id": asset_png_id,
            "bounds_pt": [15.0, 160.0, 60.0, 45.0],
            "badge_override": "B"
        }),
    );
    assert_eq!(l1.payload["panel_id"], "img-reused");
    assert_eq!(s.document().image_assets.len(), 2, "asset_id reuse must deduplicate project store");

    // 5. edit.pasteImage onto both GraphSheet and LayoutSheet (using TIFF + base64 PNG)
    let p_g = ok_cmd(&mut s, "edit.pasteImage", json!({ "target_sheet_id": "g-1", "bytes_base64": tif_b64, "bounds_pt": [30.0, 90.0, 32.0, 32.0] }));
    assert_eq!(p_g.payload["media_type"], "image/tiff");
    assert_eq!(s.document().image_assets.len(), 3);

    let p_l = ok_cmd(
        &mut s,
        "edit.pasteImage",
        json!({
            "target_sheet_id": "lay-1",
            "panel_id": "img-pasted",
            "asset_id": asset_png_id,
            "bounds_pt": [90.0, 160.0, 40.0, 30.0],
            "badge_override": "C"
        }),
    );
    assert_eq!(p_l.payload["panel_id"], "img-pasted");

    // 6. layout.updateImage & z-order via layout.reorderPanel + undo/redo
    ok_cmd(
        &mut s,
        "layout.updateImage",
        json!({
            "sheet_id": "lay-1",
            "panel_id": "img-pasted",
            "height_pt": 60.0,
            "rotation_deg": -15.0,
            "crop": { "left_pt": 2.0, "top_pt": 3.0 },
            "border": { "width_pt": 2.0, "color": "#AA1100", "dash": "dotted" },
            "badge_override": null
        }),
    );
    let lay = s.document().layout_sheet(&SheetId::new("lay-1")).unwrap();
    let pasted = lay.spec.items.iter().find(|it| it.id() == "img-pasted").unwrap();
    let LayoutItem::EmbeddedImage { bounds_pt: l_bounds, badge_override, border: l_border, rotation_deg: l_rot, .. } = pasted else {
        panic!("expected LayoutItem::EmbeddedImage");
    };
    assert!((l_bounds.height - 60.0).abs() < 1e-9);
    assert!((l_bounds.width - 80.0).abs() < 1e-9, "locked 80x60 aspect scales height 60 -> width 80");
    assert!((*l_rot + 15.0).abs() < 1e-9);
    assert!(badge_override.is_none());
    assert!(l_border.is_some());

    let idx_before = s.document().layout_sheet(&SheetId::new("lay-1")).unwrap().spec.items.iter().position(|it| it.id() == "img-pasted").unwrap();
    assert_eq!(idx_before, 2);
    ok_cmd(&mut s, "layout.reorderPanel", json!({ "sheet_id": "lay-1", "panel_index": 2, "action": "send_to_back" }));
    assert_eq!(s.document().layout_sheet(&SheetId::new("lay-1")).unwrap().spec.items[0].id(), "img-pasted");

    ok_cmd(&mut s, "edit.undo", json!({}));
    assert_eq!(s.document().layout_sheet(&SheetId::new("lay-1")).unwrap().spec.items[2].id(), "img-pasted");
    ok_cmd(&mut s, "edit.redo", json!({}));
    assert_eq!(s.document().layout_sheet(&SheetId::new("lay-1")).unwrap().spec.items[0].id(), "img-pasted");
}

#[test]
fn image_commands_reject_urls_svg_unknown_fields_and_workdir_escapes() {
    let ws = TempDirGuard::new("policy");
    let png_bytes = encode_quadrant_png(16, 12);
    let inside_png = ws.workdir.join("sub").join("ok.png");
    let inside_svg = ws.workdir.join("bad.svg");
    let outside_png = ws.outside.join("secret.png");
    fs::create_dir_all(inside_png.parent().unwrap()).unwrap();
    fs::write(&inside_png, &png_bytes).unwrap();
    fs::write(&inside_svg, "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").unwrap();
    fs::write(&outside_png, &png_bytes).unwrap();

    let policy = ExternalPathPolicy::workdir(&ws.workdir).unwrap();
    let mut s = seed_session_with_graphs_and_layout().with_path_policy(policy);

    // Reject URL schemes (both on unrestricted session -> BadInput, and under --workdir -> rejected cleanly)
    let mut s_plain = seed_session_with_graphs_and_layout();
    for cmd in ["graph.insertImage", "layout.insertImage", "edit.pasteImage"] {
        for bad_url in ["https://example.com/x.png", "http://127.0.0.1/x.png", "file:///tmp/x.png", "data:image/png;base64,AA=="] {
            let key = if cmd == "edit.pasteImage" { "target_sheet_id" } else { "sheet_id" };
            let target = if cmd == "layout.insertImage" { "lay-1" } else { "g-1" };
            let err_plain =
                s_plain.execute(cmd, json!({ (key): target, "path": bad_url })).expect_err("URL scheme must be rejected on plain session");
            assert_eq!(err_plain.code, ErrorCode::BadInput, "{cmd} `{bad_url}` on plain session got {:?}", err_plain.code);
            let err_wd = s.execute(cmd, json!({ (key): target, "path": bad_url })).expect_err("URL scheme must be rejected under workdir");
            assert!(
                matches!(err_wd.code, ErrorCode::BadInput | ErrorCode::IoError | ErrorCode::PathEscape),
                "{cmd} `{bad_url}` under workdir got unexpected {:?}",
                err_wd.code
            );
        }
    }

    // Reject SVG/XML via base64 and local file path
    let svg_b64 = encode_base64(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
    let err_b64 = s.execute("graph.insertImage", json!({ "sheet_id": "g-1", "bytes_base64": svg_b64 })).expect_err("SVG b64 must fail");
    assert_eq!(err_b64.code, ErrorCode::BadInput);
    assert!(err_b64.to_string().to_ascii_lowercase().contains("permissive-only"));

    let err_path = s.execute("layout.insertImage", json!({ "sheet_id": "lay-1", "path": "bad.svg" })).expect_err("SVG file path must fail");
    assert_eq!(err_path.code, ErrorCode::BadInput);
    assert!(err_path.to_string().to_ascii_lowercase().contains("permissive-only"));

    // Reject unknown fields & mutually exclusive sources
    let err_unk = s
        .execute("graph.insertImage", json!({ "sheet_id": "g-1", "bytes_base64": encode_base64(&png_bytes), "extra_bogus": 1 }))
        .expect_err("deny_unknown_fields must reject extra field");
    assert_eq!(err_unk.code, ErrorCode::BadInput);
    let err_both = s
        .execute("edit.pasteImage", json!({ "target_sheet_id": "lay-1", "path": "sub/ok.png", "bytes_base64": encode_base64(&png_bytes) }))
        .expect_err("mutually exclusive sources must fail");
    assert_eq!(err_both.code, ErrorCode::BadInput);

    // Reject crop exceeding bounds
    ok_cmd(&mut s, "graph.insertImage", json!({ "sheet_id": "g-1", "path": "sub/ok.png", "bounds_pt": [10.0, 10.0, 20.0, 15.0] }));
    let err_crop = s
        .execute("graph.updateImage", json!({ "sheet_id": "g-1", "annotation_index": 0, "crop": { "left_pt": 12.0, "right_pt": 10.0 } }))
        .expect_err("collapsed crop must fail");
    assert_eq!(err_crop.code, ErrorCode::BadInput);

    // Reject --workdir escapes (relative .., absolute outside, and Unix symlink escape)
    let mut escapes = vec![String::from("../outside/secret.png"), outside_png.to_string_lossy().into_owned()];
    #[cfg(unix)]
    {
        let sym = ws.workdir.join("link_escape.png");
        std::os::unix::fs::symlink(&outside_png, &sym).unwrap();
        escapes.push(String::from("link_escape.png"));
    }
    for esc in &escapes {
        for (cmd, key, target) in
            [("graph.insertImage", "sheet_id", "g-1"), ("layout.insertImage", "sheet_id", "lay-1"), ("edit.pasteImage", "target_sheet_id", "g-1")]
        {
            let err = s.execute(cmd, json!({ (key): target, "path": esc })).expect_err("escape outside workdir must fail");
            assert_eq!(err.code, ErrorCode::PathEscape, "{cmd} with `{esc}` got {:?}", err.code);
        }
    }
}

fn find_emf_record(emf: &[u8], expected_record_type: u32) -> bool {
    let mut offset = 0usize;
    while let Some(header) = emf.get(offset..offset.saturating_add(8)) {
        let Some((type_bytes, size_bytes)) = header.split_at_checked(4) else {
            return false;
        };
        let Ok(type_arr) = <[u8; 4]>::try_from(type_bytes) else {
            return false;
        };
        let Ok(size_arr) = <[u8; 4]>::try_from(size_bytes) else {
            return false;
        };
        let rec_type = u32::from_le_bytes(type_arr);
        let rec_size = u32::from_le_bytes(size_arr) as usize;
        if rec_size < 8 || offset.checked_add(rec_size).is_none_or(|end| end > emf.len()) {
            return false;
        }
        if rec_type == expected_record_type {
            return true;
        }
        if rec_type == 14 {
            // EMR_EOF
            break;
        }
        offset += rec_size;
    }
    false
}

#[test]
fn embedded_images_render_deterministically_across_rgba_svg_pdf_and_emf() {
    let mut s = seed_session_with_graphs_and_layout();
    let png_b64 = encode_base64(&encode_quadrant_png(64, 48));
    let jpg_b64 = encode_base64(&encode_rgb_jpeg(48, 36));
    let tif_b64 = encode_base64(&encode_rgba_tiff(40, 40));

    ok_cmd(
        &mut s,
        "graph.insertImage",
        json!({
            "sheet_id": "g-1",
            "bytes_base64": png_b64,
            "bounds_pt": [40.0, 40.0, 80.0, 60.0],
            "rotation_deg": 0.0,
            "border": { "width_pt": 1.0, "color": "#000000", "dash": "solid" }
        }),
    );
    ok_cmd(
        &mut s,
        "graph.insertImage",
        json!({
            "sheet_id": "g-1",
            "bytes_base64": jpg_b64,
            "bounds_pt": [140.0, 40.0, 48.0, 36.0]
        }),
    );
    ok_cmd(
        &mut s,
        "layout.insertImage",
        json!({
            "sheet_id": "lay-1",
            "panel_id": "lay-png",
            "bytes_base64": png_b64,
            "bounds_pt": [220.0, 20.0, 80.0, 60.0],
            "rotation_deg": 20.0,
            "crop": { "left_pt": 8.0, "top_pt": 6.0 }
        }),
    );
    ok_cmd(
        &mut s,
        "layout.insertImage",
        json!({
            "sheet_id": "lay-1",
            "panel_id": "lay-tif",
            "bytes_base64": tif_b64,
            "bounds_pt": [220.0, 90.0, 50.0, 50.0]
        }),
    );

    // 1. RGBA export repeatability + measurable crop and rotation pixel differences + alpha blending
    let png_r1 = ok_cmd(&mut s, "export.png", json!({ "sheet_id": "g-1", "width_px": 300, "height_px": 220, "dpi": 72 }));
    let png_r2 = ok_cmd(&mut s, "export.png", json!({ "sheet_id": "g-1", "width_px": 300, "height_px": 220, "dpi": 72 }));
    let png_raw1 = decode_base64(png_r1.payload["base64"].as_str().unwrap());
    let png_raw2 = decode_base64(png_r2.payload["base64"].as_str().unwrap());
    assert_eq!(png_raw1, png_raw2, "RGBA PNG export must be deterministic");

    let scene_base = s.render_sheet_scene(&SheetId::new("g-1")).unwrap();
    let raster_base = render_scene(&scene_base, RenderTargetSpec { width_px: 360, height_px: 260, dpi: 72 }).unwrap();

    // Sample pixel inside top-left semi-transparent red quadrant: blended over white paper -> high R, medium G/B
    let tl_x = (60.0 / scene_base.width_pt * 360.0) as u32;
    let tl_y = (52.0 / scene_base.height_pt * 260.0) as u32;
    let px = |img: &curvebook_render::RasterImage, x: u32, y: u32| -> [u8; 4] {
        let idx = ((y * img.width_px + x) * 4) as usize;
        [img.rgba[idx], img.rgba[idx + 1], img.rgba[idx + 2], img.rgba[idx + 3]]
    };
    let p_tl = px(&raster_base, tl_x, tl_y);
    assert!(p_tl[0] > p_tl[1] + 40 && p_tl[1] > 50, "semi-transparent red must alpha-blend over paper: {p_tl:?}");

    // Apply heavy left+top crop -> top-left corner becomes paper white instead of red
    ok_cmd(&mut s, "graph.updateImage", json!({ "sheet_id": "g-1", "annotation_index": 0, "crop": { "left_pt": 35.0, "top_pt": 25.0 } }));
    let scene_cropped = s.render_sheet_scene(&SheetId::new("g-1")).unwrap();
    let raster_cropped = render_scene(&scene_cropped, RenderTargetSpec { width_px: 360, height_px: 260, dpi: 72 }).unwrap();
    assert_ne!(raster_cropped.rgba, raster_base.rgba, "crop must alter rendered RGBA output");
    let p_tl_cropped = px(&raster_cropped, tl_x, tl_y);
    assert!(p_tl_cropped[0] > 230 && p_tl_cropped[1] > 230, "cropped region must reveal paper background: {p_tl_cropped:?}");

    // Reset crop and rotate 90 degrees clockwise -> quadrant colors swap
    ok_cmd(
        &mut s,
        "graph.updateImage",
        json!({ "sheet_id": "g-1", "annotation_index": 0, "crop": { "left_pt": 0.0, "top_pt": 0.0 }, "rotation_deg": 90.0 }),
    );
    let scene_rot = s.render_sheet_scene(&SheetId::new("g-1")).unwrap();
    let raster_rot = render_scene(&scene_rot, RenderTargetSpec { width_px: 360, height_px: 260, dpi: 72 }).unwrap();
    assert_ne!(raster_rot.rgba, raster_base.rgba, "rotation_deg=90 must alter rendered pixels");

    // 2. SVG export determinism + structure (`<image`, `data:image/png;base64,`, `rotate(`, `<clipPath`)
    for target in ["g-1", "lay-1"] {
        let s1 = ok_cmd(&mut s, "export.svg", json!({ "sheet_id": target }));
        let s2 = ok_cmd(&mut s, "export.svg", json!({ "sheet_id": target }));
        let svg_txt = s1.payload["svg"].as_str().unwrap();
        assert_eq!(svg_txt, s2.payload["svg"].as_str().unwrap());
        validate_svg_structure(svg_txt).unwrap();
        assert!(svg_txt.contains("<image "), "{target} SVG missing <image");
        assert!(svg_txt.contains("data:image/png;base64,"), "{target} SVG missing png data URI");
        assert!(svg_txt.contains("rotate("), "{target} SVG missing rotation transform");
    }
    let scene_lay = s.render_sheet_scene(&SheetId::new("lay-1")).unwrap();
    let lay_svg = render_svg(&scene_lay, &SvgExportOptions::default()).unwrap();
    assert!(lay_svg.contains("<clipPath"), "cropped layout image SVG must emit <clipPath>");

    // 3. PDF export (`%PDF-1.7`, `/XObject`, `/Subtype /Image`, `/SMask`, `/FlateDecode`, `/DeviceGray`, `/DeviceCMYK`)
    for target in ["g-1", "lay-1"] {
        let p1 = decode_base64(ok_cmd(&mut s, "export.pdf", json!({ "sheet_id": target })).payload["base64"].as_str().unwrap());
        let p2 = decode_base64(ok_cmd(&mut s, "export.pdf", json!({ "sheet_id": target })).payload["base64"].as_str().unwrap());
        assert_eq!(p1, p2, "{target} PDF export must be byte-identical");
        let text = String::from_utf8_lossy(&p1);
        assert!(text.starts_with("%PDF-1.7\n"));
        assert!(text.contains("/XObject"), "{target} PDF missing /XObject");
        assert!(text.contains("/Subtype /Image"), "{target} PDF missing /Subtype /Image");
        assert!(text.contains("/SMask"), "{target} PDF missing /SMask for semi-transparent PNG");
        assert!(text.contains("/FlateDecode"), "{target} PDF missing /FlateDecode");
    }
    let gray_pdf = render_pdf(&scene_lay, &PdfExportOptions { color_space: PdfColorSpace::Grayscale, ..PdfExportOptions::default() }).unwrap();
    let gray_pdf_2 = render_pdf(&scene_lay, &PdfExportOptions { color_space: PdfColorSpace::Grayscale, ..PdfExportOptions::default() }).unwrap();
    assert_eq!(gray_pdf, gray_pdf_2);
    assert!(String::from_utf8_lossy(&gray_pdf).contains("/DeviceGray"));

    let cmyk_pdf = render_pdf(&scene_lay, &PdfExportOptions { color_space: PdfColorSpace::Cmyk, ..PdfExportOptions::default() }).unwrap();
    let cmyk_pdf_2 = render_pdf(&scene_lay, &PdfExportOptions { color_space: PdfColorSpace::Cmyk, ..PdfExportOptions::default() }).unwrap();
    assert_eq!(cmyk_pdf, cmyk_pdf_2);
    assert!(String::from_utf8_lossy(&cmyk_pdf).contains("/DeviceCMYK"));

    // 4. EMF export (`graph.exportEmf`, `layout.exportEmf`) + EMR_STRETCHDIBITS (`81`)
    for (cmd_id, sheet_id) in [("graph.exportEmf", "g-1"), ("layout.exportEmf", "lay-1")] {
        let e1 = decode_base64(ok_cmd(&mut s, cmd_id, json!({ "sheet_id": sheet_id })).payload["base64"].as_str().unwrap());
        let e2 = decode_base64(ok_cmd(&mut s, cmd_id, json!({ "sheet_id": sheet_id })).payload["base64"].as_str().unwrap());
        assert_eq!(e1, e2, "{cmd_id} must be deterministic");
        assert!(e1.len() >= 88 && &e1[40..44] == b" EMF", "{cmd_id} must produce valid EMF header");
        assert!(find_emf_record(&e1, 81), "{cmd_id} EMF must contain EMR_STRETCHDIBITS (81) for embedded image");
    }
    let _ = Path::new("");
}
