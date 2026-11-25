#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Cursor;

use curvebook_doc::{ErrorCode, GraphAnnotation, LayoutItem, SheetId};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_pdf::{PdfColorSpace, PdfExportOptions, render_pdf};
use curvebook_render::{RenderTargetSpec, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg, validate_svg_structure};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::json;

use super::common::{
    TempWorkspace, assert_bad_input, assert_err_code, decode_base64, encode_base64, err_cmd, gs, ls, ok_cmd, run_cmds, session_with, xy_sheet,
};

fn encode_quadrant_png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| match (x >= w / 2, y >= h / 2) {
        (false, false) => Rgba([220, 30, 30, 140]),
        (true, false) => Rgba([30, 180, 40, 255]),
        (false, true) => Rgba([30, 60, 220, 255]),
        (true, true) => Rgba([230, 190, 20, 180]),
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

fn seed_session_with_graphs_and_layout() -> Session {
    let mut s = session_with("Images", vec![xy_sheet("xy-1", "XY", "X", &[1.0, 2.0, 3.0], &[("Y", &[10.0, 25.0, 40.0])])]);
    run_cmds(
        &mut s,
        &[
            ("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "kind": "xy" })),
            ("graph.create", json!({ "sheet_id": "g-2", "data_sheet_id": "xy-1", "kind": "xy" })),
            ("layout.create", json!({ "sheet_id": "lay-1", "title": "Composite Fig" })),
            (
                "layout.addPanel",
                json!({ "sheet_id": "lay-1", "panel_id": "panel-graph", "kind": "graph", "graph_sheet_id": "g-2", "bounds_pt": [20.0, 20.0, 180.0, 130.0] }),
            ),
        ],
    );
    s
}

#[test]
fn five_image_commands_sources_dedup_aspect_crop_border_zorder_and_undo() {
    let mut s = seed_session_with_graphs_and_layout();
    let dir = TempWorkspace::new("cmds");
    let png_b64 = encode_base64(&encode_quadrant_png(80, 60));
    let jpg_path = dir.workdir.join("blot.jpg");
    fs::write(&jpg_path, encode_rgb_jpeg(64, 48)).unwrap();
    let tif_b64 = encode_base64(&encode_rgba_tiff(32, 32));

    // 1. graph.insertImage from base64 (alpha PNG, 80x60) & file path (JPEG)
    let r1 = ok_cmd(
        &mut s,
        "graph.insertImage",
        json!({ "sheet_id": "g-1", "bytes_base64": png_b64, "bounds_pt": [20.0, 20.0, 80.0, 60.0], "border": { "width_pt": 1.5, "color": "#112233", "dash": "dashed" } }),
    );
    let asset_png_id = r1.payload["asset_id"].as_str().unwrap().to_owned();
    assert_eq!(r1.payload["media_type"], "image/png");
    assert_eq!(r1.payload["pixel_size"], json!([80, 60]));
    assert_eq!(s.document().image_assets.len(), 1);

    let r2 =
        ok_cmd(&mut s, "graph.insertImage", json!({ "sheet_id": "g-1", "path": jpg_path.to_string_lossy(), "bounds_pt": [120.0, 25.0, 64.0, 48.0] }));
    assert_eq!(r2.payload["media_type"], "image/jpeg");
    assert_eq!(s.document().image_assets.len(), 2);

    // 2. graph.updateImage with lock_aspect_ratio=true then false, crop, rotation, and border:null
    let ann_idx = r1.payload["annotation_index"].as_u64().unwrap();
    ok_cmd(&mut s, "graph.updateImage", json!({ "sheet_id": "g-1", "annotation_index": ann_idx, "width_pt": 40.0 }));
    let GraphAnnotation::EmbeddedImage { bounds_pt, .. } = &gs(&s, "g-1").spec.annotations[ann_idx as usize] else {
        panic!("expected EmbeddedImage")
    };
    assert!((bounds_pt.width - 40.0).abs() < 1e-9 && (bounds_pt.height - 30.0).abs() < 1e-9);

    ok_cmd(
        &mut s,
        "graph.updateImage",
        json!({ "sheet_id": "g-1", "annotation_index": ann_idx, "lock_aspect_ratio": false, "width_pt": 100.0, "rotation_deg": 22.5, "crop": { "left_pt": 4.0, "right_pt": 6.0, "top_pt": 2.0, "bottom_pt": 4.0 }, "border": null }),
    );
    let GraphAnnotation::EmbeddedImage { bounds_pt, rotation_deg, lock_aspect_ratio, crop, border, .. } =
        &gs(&s, "g-1").spec.annotations[ann_idx as usize]
    else {
        panic!("expected EmbeddedImage");
    };
    assert!(!*lock_aspect_ratio && (bounds_pt.width - 100.0).abs() < 1e-9 && (bounds_pt.height - 30.0).abs() < 1e-9);
    assert!((*rotation_deg - 22.5).abs() < 1e-9 && (crop.left_pt - 4.0).abs() < 1e-9 && (crop.bottom_pt - 4.0).abs() < 1e-9 && border.is_none());

    // 3. layout.insertImage reusing asset_id deduplicates; edit.pasteImage onto both Graph & Layout
    let l1 = ok_cmd(
        &mut s,
        "layout.insertImage",
        json!({ "sheet_id": "lay-1", "panel_id": "img-reused", "asset_id": asset_png_id, "bounds_pt": [15.0, 160.0, 60.0, 45.0], "badge_override": "B" }),
    );
    assert_eq!(l1.payload["panel_id"], "img-reused");
    assert_eq!(s.document().image_assets.len(), 2);

    let p_g = ok_cmd(&mut s, "edit.pasteImage", json!({ "target_sheet_id": "g-1", "bytes_base64": tif_b64, "bounds_pt": [30.0, 90.0, 32.0, 32.0] }));
    assert_eq!(p_g.payload["media_type"], "image/tiff");
    assert_eq!(s.document().image_assets.len(), 3);

    let p_l = ok_cmd(
        &mut s,
        "edit.pasteImage",
        json!({ "target_sheet_id": "lay-1", "panel_id": "img-pasted", "asset_id": asset_png_id, "bounds_pt": [90.0, 160.0, 40.0, 30.0], "badge_override": "C" }),
    );
    assert_eq!(p_l.payload["panel_id"], "img-pasted");

    // 4. layout.updateImage & z-order via layout.reorderPanel + undo/redo
    ok_cmd(
        &mut s,
        "layout.updateImage",
        json!({ "sheet_id": "lay-1", "panel_id": "img-pasted", "height_pt": 60.0, "rotation_deg": -15.0, "crop": { "left_pt": 2.0, "top_pt": 3.0 }, "border": { "width_pt": 2.0, "color": "#AA1100", "dash": "dotted" }, "badge_override": null }),
    );
    let pasted = ls(&s, "lay-1").spec.items.iter().find(|it| it.id() == "img-pasted").unwrap();
    let LayoutItem::EmbeddedImage { bounds_pt: lb, badge_override, border: lborder, rotation_deg: lrot, .. } = pasted else {
        panic!("expected EmbeddedImage")
    };
    assert!(
        (lb.height - 60.0).abs() < 1e-9
            && (lb.width - 80.0).abs() < 1e-9
            && (*lrot + 15.0).abs() < 1e-9
            && badge_override.is_none()
            && lborder.is_some()
    );

    assert_eq!(ls(&s, "lay-1").spec.items.iter().position(|it| it.id() == "img-pasted"), Some(2));
    ok_cmd(&mut s, "layout.reorderPanel", json!({ "sheet_id": "lay-1", "panel_index": 2, "action": "send_to_back" }));
    assert_eq!(ls(&s, "lay-1").spec.items[0].id(), "img-pasted");
    ok_cmd(&mut s, "edit.undo", json!({}));
    assert_eq!(ls(&s, "lay-1").spec.items[2].id(), "img-pasted");
    ok_cmd(&mut s, "edit.redo", json!({}));
    assert_eq!(ls(&s, "lay-1").spec.items[0].id(), "img-pasted");
}

#[test]
fn image_commands_reject_urls_svg_unknown_fields_and_workdir_escapes() {
    let ws = TempWorkspace::new("policy");
    let png_bytes = encode_quadrant_png(16, 12);
    let (inside_png, inside_svg, outside_png) = (ws.workdir.join("sub").join("ok.png"), ws.workdir.join("bad.svg"), ws.outside.join("secret.png"));
    fs::create_dir_all(inside_png.parent().unwrap()).unwrap();
    fs::write(&inside_png, &png_bytes).unwrap();
    fs::write(&inside_svg, "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").unwrap();
    fs::write(&outside_png, &png_bytes).unwrap();

    let mut s = seed_session_with_graphs_and_layout().with_path_policy(ExternalPathPolicy::workdir(&ws.workdir).unwrap());
    let mut s_plain = seed_session_with_graphs_and_layout();

    for cmd in ["graph.insertImage", "layout.insertImage", "edit.pasteImage"] {
        let (key, target) = if cmd == "edit.pasteImage" {
            ("target_sheet_id", "g-1")
        } else if cmd == "layout.insertImage" {
            ("sheet_id", "lay-1")
        } else {
            ("sheet_id", "g-1")
        };
        for bad_url in ["https://example.com/x.png", "http://127.0.0.1/x.png", "file:///tmp/x.png", "data:image/png;base64,AA=="] {
            let _ = assert_bad_input(&mut s_plain, cmd, json!({ (key): target, "path": bad_url }));
            let err_wd = err_cmd(&mut s, cmd, json!({ (key): target, "path": bad_url }));
            assert!(
                matches!(err_wd.code, ErrorCode::BadInput | ErrorCode::IoError | ErrorCode::PathEscape),
                "{cmd} `{bad_url}` -> {:?}",
                err_wd.code
            );
        }
    }

    // Reject SVG/XML via base64 and local file path
    let err_b64 = assert_bad_input(
        &mut s,
        "graph.insertImage",
        json!({ "sheet_id": "g-1", "bytes_base64": encode_base64(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>") }),
    );
    assert!(err_b64.to_string().to_ascii_lowercase().contains("permissive-only"));
    let err_path = assert_bad_input(&mut s, "layout.insertImage", json!({ "sheet_id": "lay-1", "path": "bad.svg" }));
    assert!(err_path.to_string().to_ascii_lowercase().contains("permissive-only"));

    // Reject unknown fields, mutually exclusive sources, and collapsed crop
    let _ = assert_bad_input(&mut s, "graph.insertImage", json!({ "sheet_id": "g-1", "bytes_base64": encode_base64(&png_bytes), "extra_bogus": 1 }));
    let _ = assert_bad_input(
        &mut s,
        "edit.pasteImage",
        json!({ "target_sheet_id": "lay-1", "path": "sub/ok.png", "bytes_base64": encode_base64(&png_bytes) }),
    );
    ok_cmd(&mut s, "graph.insertImage", json!({ "sheet_id": "g-1", "path": "sub/ok.png", "bounds_pt": [10.0, 10.0, 20.0, 15.0] }));
    let _ = assert_bad_input(
        &mut s,
        "graph.updateImage",
        json!({ "sheet_id": "g-1", "annotation_index": 0, "crop": { "left_pt": 12.0, "right_pt": 10.0 } }),
    );

    // Reject --workdir escapes (relative .., absolute outside, and Unix symlink escape)
    let mut escapes = vec![String::from("../outside/secret.png"), outside_png.to_string_lossy().into_owned()];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside_png, ws.workdir.join("link_escape.png")).unwrap();
        escapes.push(String::from("link_escape.png"));
    }
    for esc in &escapes {
        for (cmd, key, target) in
            [("graph.insertImage", "sheet_id", "g-1"), ("layout.insertImage", "sheet_id", "lay-1"), ("edit.pasteImage", "target_sheet_id", "g-1")]
        {
            let _ = assert_err_code(&mut s, cmd, json!({ (key): target, "path": esc }), ErrorCode::PathEscape);
        }
    }
}

fn find_emf_record(emf: &[u8], expected_record_type: u32) -> bool {
    let mut offset = 0usize;
    while let Some(header) = emf.get(offset..offset.saturating_add(8)) {
        let rec_type = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let rec_size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if rec_size < 8 || offset.checked_add(rec_size).is_none_or(|end| end > emf.len()) {
            return false;
        }
        if rec_type == expected_record_type {
            return true;
        }
        if rec_type == 14 {
            break;
        }
        offset += rec_size;
    }
    false
}

#[test]
fn embedded_images_render_deterministically_across_rgba_svg_pdf_and_emf() {
    let mut s = seed_session_with_graphs_and_layout();
    let (png_b64, jpg_b64, tif_b64) =
        (encode_base64(&encode_quadrant_png(64, 48)), encode_base64(&encode_rgb_jpeg(48, 36)), encode_base64(&encode_rgba_tiff(40, 40)));
    run_cmds(
        &mut s,
        &[
            (
                "graph.insertImage",
                json!({ "sheet_id": "g-1", "bytes_base64": png_b64, "bounds_pt": [40.0, 40.0, 80.0, 60.0], "rotation_deg": 0.0, "border": { "width_pt": 1.0, "color": "#000000", "dash": "solid" } }),
            ),
            ("graph.insertImage", json!({ "sheet_id": "g-1", "bytes_base64": jpg_b64, "bounds_pt": [140.0, 40.0, 48.0, 36.0] })),
            (
                "layout.insertImage",
                json!({ "sheet_id": "lay-1", "panel_id": "lay-png", "bytes_base64": png_b64, "bounds_pt": [220.0, 20.0, 80.0, 60.0], "rotation_deg": 20.0, "crop": { "left_pt": 8.0, "top_pt": 6.0 } }),
            ),
            (
                "layout.insertImage",
                json!({ "sheet_id": "lay-1", "panel_id": "lay-tif", "bytes_base64": tif_b64, "bounds_pt": [220.0, 90.0, 50.0, 50.0] }),
            ),
        ],
    );

    // 1. RGBA repeatability + alpha blending + crop/rotation pixel diffs
    let p1 = decode_base64(
        ok_cmd(&mut s, "export.png", json!({ "sheet_id": "g-1", "width_px": 300, "height_px": 220, "dpi": 72 })).payload["base64"].as_str().unwrap(),
    );
    let p2 = decode_base64(
        ok_cmd(&mut s, "export.png", json!({ "sheet_id": "g-1", "width_px": 300, "height_px": 220, "dpi": 72 })).payload["base64"].as_str().unwrap(),
    );
    assert_eq!(p1, p2);

    let spec = RenderTargetSpec { width_px: 360, height_px: 260, dpi: 72 };
    let scene_base = s.render_sheet_scene(&SheetId::new("g-1")).unwrap();
    let raster_base = render_scene(&scene_base, spec).unwrap();
    let (tl_x, tl_y) = ((60.0 / scene_base.width_pt * 360.0) as u32, (52.0 / scene_base.height_pt * 260.0) as u32);
    let px = |img: &curvebook_render::RasterImage| -> [u8; 4] {
        let idx = ((tl_y * img.width_px + tl_x) * 4) as usize;
        [img.rgba[idx], img.rgba[idx + 1], img.rgba[idx + 2], img.rgba[idx + 3]]
    };
    let p_tl = px(&raster_base);
    assert!(p_tl[0] > p_tl[1] + 40 && p_tl[1] > 50, "alpha blend failed: {p_tl:?}");

    ok_cmd(&mut s, "graph.updateImage", json!({ "sheet_id": "g-1", "annotation_index": 0, "crop": { "left_pt": 35.0, "top_pt": 25.0 } }));
    let raster_cropped = render_scene(&s.render_sheet_scene(&SheetId::new("g-1")).unwrap(), spec).unwrap();
    assert_ne!(raster_cropped.rgba, raster_base.rgba);
    let p_tl_c = px(&raster_cropped);
    assert!(p_tl_c[0] > 230 && p_tl_c[1] > 230, "cropped region not paper: {p_tl_c:?}");

    ok_cmd(
        &mut s,
        "graph.updateImage",
        json!({ "sheet_id": "g-1", "annotation_index": 0, "crop": { "left_pt": 0.0, "top_pt": 0.0 }, "rotation_deg": 90.0 }),
    );
    assert_ne!(render_scene(&s.render_sheet_scene(&SheetId::new("g-1")).unwrap(), spec).unwrap().rgba, raster_base.rgba);

    // 2. SVG determinism & structure
    for target in ["g-1", "lay-1"] {
        let s1 = ok_cmd(&mut s, "export.svg", json!({ "sheet_id": target }));
        let s2 = ok_cmd(&mut s, "export.svg", json!({ "sheet_id": target }));
        let svg_txt = s1.payload["svg"].as_str().unwrap();
        assert_eq!(svg_txt, s2.payload["svg"].as_str().unwrap());
        validate_svg_structure(svg_txt).unwrap();
        assert!(svg_txt.contains("<image ") && svg_txt.contains("data:image/png;base64,") && svg_txt.contains("rotate("));
    }
    let scene_lay = s.render_sheet_scene(&SheetId::new("lay-1")).unwrap();
    assert!(render_svg(&scene_lay, &SvgExportOptions::default()).unwrap().contains("<clipPath"));

    // 3. PDF export across RGB/Gray/CMYK
    for target in ["g-1", "lay-1"] {
        let b1 = decode_base64(ok_cmd(&mut s, "export.pdf", json!({ "sheet_id": target })).payload["base64"].as_str().unwrap());
        let b2 = decode_base64(ok_cmd(&mut s, "export.pdf", json!({ "sheet_id": target })).payload["base64"].as_str().unwrap());
        assert_eq!(b1, b2);
        let text = String::from_utf8_lossy(&b1);
        assert!(
            text.starts_with("%PDF-1.7\n")
                && text.contains("/XObject")
                && text.contains("/Subtype /Image")
                && text.contains("/SMask")
                && text.contains("/FlateDecode")
        );
    }
    for (cs, marker) in [(PdfColorSpace::Grayscale, "/DeviceGray"), (PdfColorSpace::Cmyk, "/DeviceCMYK")] {
        let opts = PdfExportOptions { color_space: cs, ..PdfExportOptions::default() };
        let b1 = render_pdf(&scene_lay, &opts).unwrap();
        assert_eq!(b1, render_pdf(&scene_lay, &opts).unwrap());
        assert!(String::from_utf8_lossy(&b1).contains(marker));
    }

    // 4. EMF export (`graph.exportEmf`, `layout.exportEmf`) + EMR_STRETCHDIBITS (`81`)
    for (cmd_id, sheet_id) in [("graph.exportEmf", "g-1"), ("layout.exportEmf", "lay-1")] {
        let e1 = decode_base64(ok_cmd(&mut s, cmd_id, json!({ "sheet_id": sheet_id })).payload["base64"].as_str().unwrap());
        let e2 = decode_base64(ok_cmd(&mut s, cmd_id, json!({ "sheet_id": sheet_id })).payload["base64"].as_str().unwrap());
        assert_eq!(e1, e2);
        assert!(e1.len() >= 88 && &e1[40..44] == b" EMF" && find_emf_record(&e1, 81));
    }
}
