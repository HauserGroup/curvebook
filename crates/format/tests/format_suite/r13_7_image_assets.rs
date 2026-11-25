#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use crate::common::{TempDirGuard, build_archive_zip_from_value, col_sheet, encode_rgb_jpeg, encode_rgba_png, encode_rgba_tiff};
use curvebook_doc::{
    CURRENT_SCHEMA_VERSION, CropInsetsPt, GraphAnnotation, GraphSheet, ImageBorderSpec, ImageMimeKind, JournalWidthPreset, LayoutItem, LayoutSheet,
    LayoutSpec, MAX_IMAGE_ASSET_BYTES, ProjectDocument, ProjectImageAsset, RectPt, SheetId,
};
use curvebook_format::{
    ArchiveManifest, FormatError, ZipArchive, ZipWriter, ingest_image_asset, instantiate_template_bytes, open_archive_bytes, open_project_bytes,
    open_project_unpacked_dir, open_template_bytes, save_project_bytes, save_project_unpacked_dir, save_template_bytes, validate_image_asset,
};
use curvebook_palette::{LineDashStyle, Rgba8};
use serde_json::{Value, json};

fn base_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Image Asset Test").unwrap();
    let ds = col_sheet("data-1", 2, 3);
    let gs = GraphSheet::new_uninitialized_for_data("graph-1", &ds).unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs).unwrap();
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm)).unwrap()).unwrap();
    doc
}

#[test]
fn zip_and_unpacked_roundtrip_png_jpeg_tiff_externalize_bytes_and_prune_stale() {
    let (png_bytes, jpg_bytes, tif_bytes) = (encode_rgba_png(6, 4), encode_rgb_jpeg(8, 8), encode_rgba_tiff(5, 3));
    let (a_png, a_jpg, a_tif) =
        (ingest_image_asset(&png_bytes).unwrap(), ingest_image_asset(&jpg_bytes).unwrap(), ingest_image_asset(&tif_bytes).unwrap());
    assert_eq!((a_png.media_type, a_png.pixel_size), (ImageMimeKind::Png, (6, 4)));
    assert_eq!((a_jpg.media_type, a_jpg.pixel_size), (ImageMimeKind::Jpeg, (8, 8)));
    assert_eq!((a_tif.media_type, a_tif.pixel_size), (ImageMimeKind::Tiff, (5, 3)));

    let mut doc = base_project();
    let (id_png, id_jpg, id_tif) =
        (doc.upsert_image_asset(a_png).unwrap(), doc.upsert_image_asset(a_jpg).unwrap(), doc.upsert_image_asset(a_tif).unwrap());

    doc.graph_sheet_mut(&SheetId::new("graph-1")).unwrap().spec.annotations.extend([
        GraphAnnotation::EmbeddedImage {
            asset_id: id_png.clone(),
            bounds_pt: RectPt::new(10.0, 12.0, 60.0, 40.0),
            rotation_deg: 15.0,
            lock_aspect_ratio: true,
            crop: CropInsetsPt { left_pt: 2.0, right_pt: 2.0, top_pt: 0.0, bottom_pt: 0.0 },
            border: Some(ImageBorderSpec { width_pt: 1.0, color: Rgba8::BLACK, dash: LineDashStyle::Solid }),
        },
        GraphAnnotation::EmbeddedImage {
            asset_id: id_jpg.clone(),
            bounds_pt: RectPt::new(80.0, 20.0, 50.0, 50.0),
            rotation_deg: 0.0,
            lock_aspect_ratio: true,
            crop: CropInsetsPt::default(),
            border: None,
        },
    ]);
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

    let zip1 = save_project_bytes(&doc).unwrap();
    assert_eq!(zip1, save_project_bytes(&doc).unwrap());

    let raw_zip = ZipArchive::from_bytes(&zip1).unwrap();
    for (path, expected) in
        [(format!("assets/{id_png}.png"), &png_bytes), (format!("assets/{id_jpg}.jpg"), &jpg_bytes), (format!("assets/{id_tif}.tif"), &tif_bytes)]
    {
        assert_eq!(raw_zip.get(&path), Some(expected.as_slice()));
    }

    let doc_json: Value = serde_json::from_str(raw_zip.get_utf8("document.json").unwrap().unwrap()).unwrap();
    assert_eq!(doc_json["schema_version"].as_u64(), Some(u64::from(CURRENT_SCHEMA_VERSION)));
    let stored_assets = doc_json["image_assets"].as_array().unwrap();
    assert_eq!(stored_assets.len(), 3);
    assert!(stored_assets.iter().all(|a| a.get("bytes").and_then(Value::as_array).is_none_or(Vec::is_empty)));

    let manifest: ArchiveManifest = serde_json::from_str(raw_zip.get_utf8("manifest.json").unwrap().unwrap()).unwrap();
    assert_eq!((manifest.schema_version, manifest.assets.len()), (CURRENT_SCHEMA_VERSION, 3));
    for entry in &manifest.assets {
        assert_eq!(entry.crc32, crc32fast::hash(raw_zip.get(&entry.path).unwrap()));
    }
    assert_eq!(open_project_bytes(&zip1).unwrap(), doc);

    // Unpacked `.curvebook.d/` roundtrip + stale asset pruning + unrelated file preservation.
    let dir = TempDirGuard::new("unpacked").unwrap();
    let proj_dir = dir.child("study.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    fs::write(proj_dir.join("README.notes"), "user notes kept").unwrap();
    assert_eq!(fs::read(proj_dir.join(format!("assets/{id_png}.png"))).unwrap(), png_bytes);
    assert_eq!(fs::read(proj_dir.join(format!("assets/{id_tif}.tif"))).unwrap(), tif_bytes);
    let unpacked_doc_json: Value = serde_json::from_str(&fs::read_to_string(proj_dir.join("document.json")).unwrap()).unwrap();
    assert!(unpacked_doc_json["image_assets"].as_array().unwrap().iter().all(|a| a.get("bytes").and_then(Value::as_array).is_none_or(Vec::is_empty)));
    assert_eq!(open_project_unpacked_dir(&proj_dir).unwrap(), doc);

    let mut trimmed = doc;
    trimmed.layout_sheet_mut(&SheetId::new("layout-1")).unwrap().spec.items.clear();
    trimmed.image_assets.retain(|a| a.asset_id != id_tif);
    save_project_unpacked_dir(&trimmed, &proj_dir).unwrap();
    assert!(!proj_dir.join(format!("assets/{id_tif}.tif")).exists());
    assert!(proj_dir.join(format!("assets/{id_png}.png")).exists() && proj_dir.join("README.notes").exists());
    assert_eq!(open_project_unpacked_dir(&proj_dir).unwrap(), trimmed);
}

#[test]
fn cbtemplate_keeps_referenced_assets_and_drops_unreferenced_orphans() {
    let (png_used, tiff_used, jpg_orphan) = (encode_rgba_png(4, 4), encode_rgba_tiff(4, 4), encode_rgb_jpeg(6, 6));
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

    let tpl_bytes = save_template_bytes(&doc).unwrap();
    let raw_tpl = ZipArchive::from_bytes(&tpl_bytes).unwrap();
    assert!(
        raw_tpl.get(&format!("assets/{id_png}.png")).is_some()
            && raw_tpl.get(&format!("assets/{id_tif}.tif")).is_some()
            && raw_tpl.get(&format!("assets/{id_orphan}.jpg")).is_none()
    );

    let (_, tpl_doc) = open_archive_bytes(&tpl_bytes).unwrap();
    assert!(
        tpl_doc.image_assets.len() == 2
            && tpl_doc.image_asset(&id_png).is_some()
            && tpl_doc.image_asset(&id_tif).is_some()
            && tpl_doc.image_asset(&id_orphan).is_none()
    );

    let inst = instantiate_template_bytes(&tpl_bytes, Some("Instantiated Study")).unwrap();
    assert_eq!(
        (inst.image_assets.len(), inst.image_asset(&id_png).unwrap().bytes.as_slice(), inst.image_asset(&id_tif).unwrap().bytes.as_slice()),
        (2, png_used.as_slice(), tiff_used.as_slice())
    );
    assert!(open_template_bytes(&tpl_bytes).is_ok());
}

#[test]
fn schema_v6_migrates_to_v7_and_v8_is_rejected() {
    let base = ProjectDocument::new("Migration Target").unwrap();
    let mut v6 = serde_json::to_value(&base).unwrap();
    v6["schema_version"] = json!(6);
    v6.as_object_mut().unwrap().remove("image_assets");
    let upgraded = open_project_bytes(&build_archive_zip_from_value(&v6, 6)).unwrap();
    assert_eq!(upgraded.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(upgraded.image_assets.is_empty());

    let mut v8 = serde_json::to_value(&base).unwrap();
    v8["schema_version"] = json!(CURRENT_SCHEMA_VERSION + 1);
    let err = open_project_bytes(&build_archive_zip_from_value(&v8, CURRENT_SCHEMA_VERSION + 1)).unwrap_err();
    assert!(
        matches!(err, FormatError::UnsupportedSchemaVersion { version, max_supported } if version == CURRENT_SCHEMA_VERSION + 1 && max_supported == CURRENT_SCHEMA_VERSION)
    );
}

#[test]
fn rejects_svg_policy_corrupt_assets_and_tampered_checksums() {
    for svg in
        [b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".as_slice(), b"\xEF\xBB\xBF  <?xml version=\"1.0\"?><svg/>", b"<!DOCTYPE svg><svg/>"]
    {
        assert!(ingest_image_asset(svg).unwrap_err().to_string().to_ascii_lowercase().contains("permissive-only"));
    }
    for bad in [&[][..], b"BM\x00\x00\x00\x00", b"\x89PNG\r\n\x1a\ncorrupt-payload"] {
        assert!(matches!(ingest_image_asset(bad), Err(FormatError::UnsupportedFormat { .. })));
    }
    assert!(matches!(ingest_image_asset(&vec![0u8; MAX_IMAGE_ASSET_BYTES + 1]), Err(FormatError::FileTooLarge { .. })));

    let (valid_png, other_png) = (encode_rgba_png(3, 3), encode_rgba_png(4, 4));
    let mut tampered = ingest_image_asset(&valid_png).unwrap();
    tampered.bytes = other_png.clone();
    assert!(validate_image_asset(&tampered).is_err());

    let mut doc = base_project();
    let asset_id = doc.upsert_image_asset(ingest_image_asset(&valid_png).unwrap()).unwrap();
    let orig_zip = ZipArchive::from_bytes(&save_project_bytes(&doc).unwrap()).unwrap();
    let asset_entry_path = format!("assets/{asset_id}.png");

    let mut missing_writer = ZipWriter::new();
    for (k, v) in orig_zip.iter().filter(|(k, _)| *k != asset_entry_path) {
        missing_writer.add_file(k, v.to_vec()).unwrap();
    }
    assert!(matches!(open_project_bytes(&missing_writer.finish().unwrap()), Err(FormatError::MissingArchiveEntry { .. })));

    let dir = TempDirGuard::new("corrupt-dir").unwrap();
    let proj_dir = dir.child("broken.curvebook.d");
    save_project_unpacked_dir(&doc, &proj_dir).unwrap();
    let mut same_len_corrupt = valid_png.clone();
    *same_len_corrupt.last_mut().unwrap() ^= 0x01;
    fs::write(proj_dir.join(&asset_entry_path), same_len_corrupt).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::CorruptCrc { .. })));

    fs::write(proj_dir.join(&asset_entry_path), other_png).unwrap();
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::ZipBomb { .. })));

    let bogus = ProjectImageAsset { asset_id: String::from("not-hex"), media_type: ImageMimeKind::Png, pixel_size: (3, 3), bytes: valid_png };
    assert!(validate_image_asset(&bogus).is_err());
}
