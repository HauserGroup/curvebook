#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Cursor;

use crate::common::TempDirGuard;

use curvebook_doc::{
    CURRENT_SCHEMA_VERSION, ColumnDesignHint, ColumnTable, CropInsetsPt, DataSheet, GraphAnnotation, GraphSheet, ImageBorderSpec, ImageMimeKind,
    JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MAX_IMAGE_ASSET_BYTES, ProjectDocument, ProjectImageAsset, RectPt, SheetId,
    TableArchetype, TableKind,
};
use curvebook_format::{
    ArchiveManifest, FormatError, ZipArchive, ZipWriter, ingest_image_asset, instantiate_template_bytes, open_archive_bytes, open_project_bytes,
    open_project_unpacked_dir, open_template_bytes, save_project_bytes, save_project_unpacked_dir, save_template_bytes, validate_image_asset,
};
use curvebook_palette::{LineDashStyle, Rgba8};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::Value;

fn encode_rgba_png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| {
        let a = if (x + y).is_multiple_of(2) { 255 } else { 128 };
        Rgba([(x * 40) as u8, (y * 55) as u8, 200, a])
    });
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Png).expect("encode png");
    out
}

fn encode_rgb_jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(w, h, |x, y| Rgb([((x * 60) % 255) as u8, 90, ((y * 70) % 255) as u8]));
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg).expect("encode jpeg");
    out
}

fn encode_rgba_tiff(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(w, h, |x, y| Rgba([30, (x * 50) as u8, (y * 60) as u8, 210]));
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Tiff).expect("encode tiff");
    out
}

fn base_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Image Asset Test").unwrap();
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
    let ds = DataSheet::new("data-1", "Cohort", TableArchetype::Column(col)).unwrap();
    let gs = GraphSheet::new_uninitialized_for_data("graph-1", &ds).unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs).unwrap();
    let lspec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).unwrap()).unwrap();
    doc
}

#[test]
fn zip_and_unpacked_roundtrip_png_jpeg_tiff_externalize_bytes_and_prune_stale() {
    let png_bytes = encode_rgba_png(6, 4);
    let jpg_bytes = encode_rgb_jpeg(8, 8);
    let tif_bytes = encode_rgba_tiff(5, 3);

    let a_png = ingest_image_asset(&png_bytes).expect("ingest png");
    let a_jpg = ingest_image_asset(&jpg_bytes).expect("ingest jpg");
    let a_tif = ingest_image_asset(&tif_bytes).expect("ingest tif");
    assert_eq!(a_png.media_type, ImageMimeKind::Png);
    assert_eq!(a_png.pixel_size, (6, 4));
    assert_eq!(a_jpg.media_type, ImageMimeKind::Jpeg);
    assert_eq!(a_jpg.pixel_size, (8, 8));
    assert_eq!(a_tif.media_type, ImageMimeKind::Tiff);
    assert_eq!(a_tif.pixel_size, (5, 3));

    let mut doc = base_project();
    let id_png = doc.upsert_image_asset(a_png.clone()).unwrap();
    let id_jpg = doc.upsert_image_asset(a_jpg.clone()).unwrap();
    let id_tif = doc.upsert_image_asset(a_tif.clone()).unwrap();

    doc.graph_sheet_mut(&SheetId::new("graph-1")).unwrap().spec.annotations.push(GraphAnnotation::EmbeddedImage {
        asset_id: id_png.clone(),
        bounds_pt: RectPt::new(10.0, 12.0, 60.0, 40.0),
        rotation_deg: 15.0,
        lock_aspect_ratio: true,
        crop: CropInsetsPt { left_pt: 2.0, right_pt: 2.0, top_pt: 0.0, bottom_pt: 0.0 },
        border: Some(ImageBorderSpec { width_pt: 1.0, color: Rgba8::BLACK, dash: LineDashStyle::Solid }),
    });
    doc.graph_sheet_mut(&SheetId::new("graph-1")).unwrap().spec.annotations.push(GraphAnnotation::EmbeddedImage {
        asset_id: id_jpg.clone(),
        bounds_pt: RectPt::new(80.0, 20.0, 50.0, 50.0),
        rotation_deg: 0.0,
        lock_aspect_ratio: true,
        crop: CropInsetsPt::default(),
        border: None,
    });
    doc.layout_sheet_mut(&SheetId::new("layout-1"))
        .unwrap()
        .spec
        .add_item(LayoutItem::EmbeddedImage {
            id: String::from("panel-img"),
            asset_id: id_tif.clone(),
            bounds_pt: RectPt::new(14.0, 14.0, 90.0, 54.0),
            rotation_deg: -5.0,
            lock_aspect_ratio: false,
            crop: CropInsetsPt::default(),
            border: None,
            badge_override: Some(String::from("A")),
        })
        .unwrap();

    // Deterministic ZIP output
    let zip1 = save_project_bytes(&doc).expect("save zip 1");
    let zip2 = save_project_bytes(&doc).expect("save zip 2");
    assert_eq!(zip1, zip2);

    // Inspect ZIP entries: binary payloads under `assets/<id>.<ext>` and empty inline bytes in `document.json`.
    let raw_zip = ZipArchive::from_bytes(&zip1).unwrap();
    assert_eq!(raw_zip.get(&format!("assets/{id_png}.png")), Some(png_bytes.as_slice()));
    assert_eq!(raw_zip.get(&format!("assets/{id_jpg}.jpg")), Some(jpg_bytes.as_slice()));
    assert_eq!(raw_zip.get(&format!("assets/{id_tif}.tif")), Some(tif_bytes.as_slice()));

    let doc_json: Value = serde_json::from_str(raw_zip.get_utf8("document.json").unwrap().unwrap()).unwrap();
    assert_eq!(doc_json["schema_version"].as_u64(), Some(u64::from(CURRENT_SCHEMA_VERSION)));
    let stored_assets = doc_json["image_assets"].as_array().unwrap();
    assert_eq!(stored_assets.len(), 3);
    for a in stored_assets {
        let is_empty_bytes = a.get("bytes").and_then(Value::as_array).is_none_or(Vec::is_empty);
        assert!(is_empty_bytes, "document.json must strip inline asset bytes: {a}");
    }

    let manifest: ArchiveManifest = serde_json::from_str(raw_zip.get_utf8("manifest.json").unwrap().unwrap()).unwrap();
    assert_eq!(manifest.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(manifest.assets.len(), 3);
    for entry in &manifest.assets {
        let expected_bytes = raw_zip.get(&entry.path).expect("manifest asset path in zip");
        assert_eq!(entry.crc32, crc32fast::hash(expected_bytes));
    }

    let reopened = open_project_bytes(&zip1).expect("reopen zip");
    assert_eq!(reopened, doc);

    // Unpacked `.curvebook.d/` roundtrip + stale asset pruning + unrelated file preservation.
    let dir = TempDirGuard::new("unpacked").expect("create temp dir");
    let proj_dir = dir.path().join("study.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_dir).expect("save unpacked dir");
    std::fs::write(proj_dir.join("README.notes"), "user notes kept").unwrap();

    assert_eq!(std::fs::read(proj_dir.join(format!("assets/{id_png}.png"))).unwrap(), png_bytes);
    assert_eq!(std::fs::read(proj_dir.join(format!("assets/{id_tif}.tif"))).unwrap(), tif_bytes);
    let unpacked_doc_json: Value = serde_json::from_str(&std::fs::read_to_string(proj_dir.join("document.json")).unwrap()).unwrap();
    for a in unpacked_doc_json["image_assets"].as_array().unwrap() {
        assert!(a.get("bytes").and_then(Value::as_array).is_none_or(Vec::is_empty));
    }
    assert_eq!(open_project_unpacked_dir(&proj_dir).expect("open unpacked dir"), doc);

    // Remove TIFF from layout + project, re-save, verify `assets/<id_tif>.tif` was pruned while README stays.
    let mut trimmed = doc.clone();
    trimmed.layout_sheet_mut(&SheetId::new("layout-1")).unwrap().spec.items.clear();
    trimmed.image_assets.retain(|a| a.asset_id != id_tif);
    save_project_unpacked_dir(&trimmed, &proj_dir).expect("resave unpacked dir");
    assert!(!proj_dir.join(format!("assets/{id_tif}.tif")).exists(), "stale asset file must be pruned");
    assert!(proj_dir.join(format!("assets/{id_png}.png")).exists());
    assert!(proj_dir.join("README.notes").exists(), "non-archive repo file must be kept");
    assert_eq!(open_project_unpacked_dir(&proj_dir).unwrap(), trimmed);
}

#[test]
fn cbtemplate_keeps_referenced_assets_and_drops_unreferenced_orphans() {
    let png_used = encode_rgba_png(4, 4);
    let tiff_used = encode_rgba_tiff(4, 4);
    let jpg_orphan = encode_rgb_jpeg(6, 6);

    let mut doc = base_project();
    let id_png = doc.upsert_image_asset(ingest_image_asset(&png_used).unwrap()).unwrap();
    let id_tif = doc.upsert_image_asset(ingest_image_asset(&tiff_used).unwrap()).unwrap();
    let id_orphan = doc.upsert_image_asset(ingest_image_asset(&jpg_orphan).unwrap()).unwrap();

    doc.graph_sheet_mut(&SheetId::new("graph-1")).unwrap().spec.annotations.push(GraphAnnotation::EmbeddedImage {
        asset_id: id_png.clone(),
        bounds_pt: RectPt::new(5.0, 5.0, 30.0, 30.0),
        rotation_deg: 0.0,
        lock_aspect_ratio: true,
        crop: CropInsetsPt::default(),
        border: None,
    });
    doc.layout_sheet_mut(&SheetId::new("layout-1"))
        .unwrap()
        .spec
        .add_item(LayoutItem::EmbeddedImage {
            id: String::from("logo"),
            asset_id: id_tif.clone(),
            bounds_pt: RectPt::new(10.0, 10.0, 30.0, 30.0),
            rotation_deg: 0.0,
            lock_aspect_ratio: true,
            crop: CropInsetsPt::default(),
            border: None,
            badge_override: None,
        })
        .unwrap();

    let tpl_bytes = save_template_bytes(&doc).expect("save template");
    let raw_tpl = ZipArchive::from_bytes(&tpl_bytes).unwrap();
    assert!(raw_tpl.get(&format!("assets/{id_png}.png")).is_some());
    assert!(raw_tpl.get(&format!("assets/{id_tif}.tif")).is_some());
    assert!(raw_tpl.get(&format!("assets/{id_orphan}.jpg")).is_none(), "unreferenced asset must be dropped from template");

    let (_, tpl_doc) = open_archive_bytes(&tpl_bytes).unwrap();
    assert_eq!(tpl_doc.image_assets.len(), 2);
    assert!(tpl_doc.image_asset(&id_png).is_some());
    assert!(tpl_doc.image_asset(&id_tif).is_some());
    assert!(tpl_doc.image_asset(&id_orphan).is_none());

    let inst = instantiate_template_bytes(&tpl_bytes, Some("Instantiated Study")).unwrap();
    assert_eq!(inst.image_assets.len(), 2);
    assert_eq!(inst.image_asset(&id_png).unwrap().bytes, png_used);
    assert_eq!(inst.image_asset(&id_tif).unwrap().bytes, tiff_used);
    assert!(open_template_bytes(&tpl_bytes).is_ok());
}

#[test]
fn schema_v6_migrates_to_v7_and_v8_is_rejected() {
    let base = ProjectDocument::new("Migration Target").unwrap();
    let mut val = serde_json::to_value(&base).unwrap();
    val["schema_version"] = serde_json::json!(6);
    val.as_object_mut().unwrap().remove("image_assets");
    let doc_json = serde_json::to_string_pretty(&val).unwrap();
    let doc_crc = crc32fast::hash(doc_json.as_bytes());
    let manifest = serde_json::json!({
        "format": "curvebook-archive",
        "schema_version": 6,
        "kind": "Project",
        "title": "Migration Target",
        "document_crc32": doc_crc,
        "sheets": [],
        "assets": []
    });
    let mut writer = ZipWriter::new();
    writer.add_file("manifest.json", serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    writer.add_file("document.json", doc_json.into_bytes()).unwrap();
    let v6_zip = writer.finish().unwrap();

    let upgraded = open_project_bytes(&v6_zip).expect("v6 should upgrade to v7");
    assert_eq!(upgraded.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(upgraded.image_assets.is_empty());

    // v8 forward refusal
    let mut v8_val = serde_json::to_value(&base).unwrap();
    v8_val["schema_version"] = serde_json::json!(CURRENT_SCHEMA_VERSION + 1);
    let v8_json = serde_json::to_string_pretty(&v8_val).unwrap();
    let v8_crc = crc32fast::hash(v8_json.as_bytes());
    let v8_manifest = serde_json::json!({
        "format": "curvebook-archive",
        "schema_version": CURRENT_SCHEMA_VERSION + 1,
        "kind": "Project",
        "title": "Future Version",
        "document_crc32": v8_crc,
        "sheets": [],
        "assets": []
    });
    let mut v8_writer = ZipWriter::new();
    v8_writer.add_file("manifest.json", serde_json::to_vec_pretty(&v8_manifest).unwrap()).unwrap();
    v8_writer.add_file("document.json", v8_json.into_bytes()).unwrap();
    let v8_zip = v8_writer.finish().unwrap();

    let err = open_project_bytes(&v8_zip).expect_err("v8 must be rejected");
    assert!(matches!(
        err,
        FormatError::UnsupportedSchemaVersion { version, max_supported }
        if version == CURRENT_SCHEMA_VERSION + 1 && max_supported == CURRENT_SCHEMA_VERSION
    ));
}

#[test]
fn rejects_svg_policy_corrupt_assets_and_tampered_checksums() {
    for svg_payload in [
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".as_slice(),
        b"\xEF\xBB\xBF  <?xml version=\"1.0\"?><svg/>".as_slice(),
        b"<!DOCTYPE svg><svg/>".as_slice(),
    ] {
        let err = ingest_image_asset(svg_payload).expect_err("SVG/XML must be rejected");
        let msg = err.to_string().to_ascii_lowercase();
        assert!(msg.contains("permissive-only"), "expected explicit permissive-policy message, got: {msg}");
    }

    assert!(matches!(ingest_image_asset(&[]), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(ingest_image_asset(b"BM\x00\x00\x00\x00"), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(ingest_image_asset(b"\x89PNG\r\n\x1a\ncorrupt-payload"), Err(FormatError::UnsupportedFormat { .. })));

    let big = vec![0u8; MAX_IMAGE_ASSET_BYTES + 1];
    assert!(matches!(ingest_image_asset(&big), Err(FormatError::FileTooLarge { .. })));

    // Tampered SHA-256 or dimensions rejected by `validate_image_asset`.
    let valid_png = encode_rgba_png(3, 3);
    let other_png = encode_rgba_png(4, 4);
    let mut tampered = ingest_image_asset(&valid_png).unwrap();
    tampered.bytes = other_png.clone();
    assert!(validate_image_asset(&tampered).is_err());

    // Missing `assets/<id>.png` entry in ZIP and corrupt CRC in `.curvebook.d/` rejected on open.
    let mut doc = base_project();
    let asset = ingest_image_asset(&valid_png).unwrap();
    let asset_id = doc.upsert_image_asset(asset).unwrap();
    let zip_bytes = save_project_bytes(&doc).unwrap();
    let orig_zip = ZipArchive::from_bytes(&zip_bytes).unwrap();
    let asset_entry_path = format!("assets/{asset_id}.png");

    // 1) Missing asset entry
    let mut missing_writer = ZipWriter::new();
    for (k, v) in orig_zip.iter() {
        if k != asset_entry_path {
            missing_writer.add_file(k, v.to_vec()).unwrap();
        }
    }
    let missing_zip = missing_writer.finish().unwrap();
    assert!(matches!(open_project_bytes(&missing_zip), Err(FormatError::MissingArchiveEntry { .. })));

    // 2) Corrupt same-length asset bytes on disk in `.curvebook.d/` -> CRC-32 mismatch
    let dir = TempDirGuard::new("corrupt-dir").expect("create temp dir");
    let proj_dir = dir.path().join("broken.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    let mut same_len_corrupt = valid_png.clone();
    let last = same_len_corrupt.len() - 1;
    same_len_corrupt[last] ^= 0x01;
    std::fs::write(proj_dir.join(&asset_entry_path), same_len_corrupt).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::CorruptCrc { .. })));

    // 3) Length-mismatched asset replacement on disk -> ZipBomb size mismatch
    std::fs::write(proj_dir.join(&asset_entry_path), other_png).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::ZipBomb { .. })));

    // Sanity check invalid Asset helper
    let bogus = ProjectImageAsset { asset_id: String::from("not-hex"), media_type: ImageMimeKind::Png, pixel_size: (3, 3), bytes: valid_png };
    assert!(validate_image_asset(&bogus).is_err());
    let _ = TableKind::Column;
}
