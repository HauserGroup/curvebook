#![forbid(unsafe_code)]

use curvebook_doc::{CURRENT_SCHEMA_VERSION, InfoSheet, SheetId};
use curvebook_format::{ZipArchive, ZipLimits, ingest_image_asset, save_project_bytes};
use curvebook_ui_egui::serde_json::{self, Value, json};
use curvebook_ui_egui::{UiApp, VirtualWorkspaceFolder};

#[test]
fn vws_enforces_entry_byte_and_count_caps() {
    let limits = ZipLimits::default();
    let max_entry = usize::try_from(limits.max_entry_uncompressed_bytes).unwrap_or(usize::MAX);
    let mut folder = VirtualWorkspaceFolder::new("caps.curvebook.d");

    let oversized = vec![b'a'; max_entry + 1];
    let err = folder.write_file("big.bin", oversized).expect_err("single entry exceeding 64 MiB must fail");
    assert!(err.contains("per-entry byte limit"), "unexpected error: {err}");

    let chunk = vec![b'x'; 32 * 1024 * 1024];
    for idx in 0..4 {
        folder.write_file(&format!("part_{idx}.bin"), chunk.clone()).expect("4 x 32 MiB fits within 128 MiB cap");
    }
    let total_err = folder.write_file("part_4.bin", vec![b'z']).expect_err("exceeding 128 MiB total must fail");
    assert!(total_err.contains("total byte budget"), "unexpected error: {total_err}");

    // Overwriting an existing entry replaces its previous byte length cleanly.
    folder.write_file("part_3.bin", vec![b'y'; 16]).expect("shrinking overwrite must succeed");
    folder.write_file("extra.txt", b"ok".to_vec()).expect("subsequent small write after shrink must succeed");

    // Deserialized or directly constructed folders with > 4096 entries also fail open/validate.
    let mut too_many = VirtualWorkspaceFolder::new("many.curvebook.d");
    for idx in 0..=limits.max_entries {
        too_many.files.insert(format!("items/f_{idx}.txt"), b"x".to_vec());
    }
    let count_err = too_many.open_unpacked_snapshot().expect_err("> 4096 entries must be rejected");
    assert!(count_err.contains("maximum entry count"), "unexpected count error: {count_err}");
}

#[test]
fn vws_validates_manifest_crc_schema_and_declared_sheets() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let _ = app.execute_command("sheet.createData", json!({ "kind": "xy", "title": "XY Data" }))?;
    let mut doc = app.document().clone();
    doc.title = String::from("VWS Valid");
    let mut folder = VirtualWorkspaceFolder::new("valid.curvebook.d");
    let _total = folder.save_unpacked_snapshot(&doc)?;

    let reopened = folder.open_unpacked_snapshot()?;
    assert_eq!(reopened.title, "VWS Valid");

    // 1. Missing manifest.json rejected
    let mut missing_manifest = folder.clone();
    missing_manifest.files.remove("manifest.json");
    let err_missing = missing_manifest.open_unpacked_snapshot().expect_err("missing manifest must fail");
    assert!(err_missing.contains("manifest.json"), "unexpected err: {err_missing}");

    // 2. Corrupt JSON / non-object manifest rejected
    let mut bad_manifest = folder.clone();
    bad_manifest.write_file("manifest.json", b"not-json".to_vec())?;
    assert!(bad_manifest.open_unpacked_snapshot().is_err());
    bad_manifest.write_file("manifest.json", b"[]".to_vec())?;
    assert!(bad_manifest.open_unpacked_snapshot().is_err());

    // 3. Archive-style manifest from saved bytes loads when valid and rejects tampered document/sheet CRC
    let archive_bytes = curvebook_format::save_project_bytes(&doc)?;
    let archive = curvebook_format::ZipArchive::from_bytes(&archive_bytes)?;
    let mut crc_folder = VirtualWorkspaceFolder::new("crc.curvebook.d");
    for (name, content) in archive.iter() {
        crc_folder.write_file(name, content.to_vec())?;
    }
    assert_eq!(crc_folder.open_unpacked_snapshot()?.title, "VWS Valid");

    let mut tampered_doc = crc_folder.clone();
    let doc_text = String::from_utf8(tampered_doc.read_file("document.json").ok_or("missing doc")?.to_vec())?;
    tampered_doc.write_file("document.json", format!("{doc_text} ").into_bytes())?;
    let crc_err = tampered_doc.open_unpacked_snapshot().expect_err("tampered document.json must fail CRC");
    assert!(crc_err.contains("CRC-32 mismatch"), "unexpected crc err: {crc_err}");

    // 4. Tampered sheet CSV detected via sheet crc32
    let mut tampered_csv = crc_folder.clone();
    let sheet_key = tampered_csv.list_files().into_iter().find(|p| p.starts_with("sheets/")).ok_or("missing sheet csv")?;
    tampered_csv.write_file(&sheet_key, b"X,Y\n999,888\n".to_vec())?;
    let csv_err = tampered_csv.open_unpacked_snapshot().expect_err("tampered sheet CSV must fail CRC");
    assert!(csv_err.contains("CRC-32 mismatch"), "unexpected csv crc err: {csv_err}");

    // 5. Mismatched schema_version or legacy archive format rejected
    let mut bad_ver = folder.clone();
    let mut manifest_val: Value = serde_json::from_slice(bad_ver.read_file("manifest.json").ok_or("missing manifest")?)?;
    manifest_val["schema_version"] = json!(CURRENT_SCHEMA_VERSION + 1);
    bad_ver.write_file("manifest.json", serde_json::to_vec(&manifest_val)?)?;
    assert!(bad_ver.open_unpacked_snapshot().is_err());

    let mut legacy_magic = crc_folder.clone();
    let mut archive_val: Value = serde_json::from_slice(legacy_magic.read_file("manifest.json").ok_or("missing manifest")?)?;
    archive_val["format"] = json!("legacy-v0-magic");
    legacy_magic.write_file("manifest.json", serde_json::to_vec(&archive_val)?)?;
    assert!(legacy_magic.open_unpacked_snapshot().is_err());

    // 6. Forged manifest omitting declared sheets or matching title rejected
    let mut wrong_sheets = folder.clone();
    let mut ws_val: Value = serde_json::from_slice(wrong_sheets.read_file("manifest.json").ok_or("missing manifest")?)?;
    ws_val["sheets"] = json!([]);
    wrong_sheets.write_file("manifest.json", serde_json::to_vec(&ws_val)?)?;
    assert!(wrong_sheets.open_unpacked_snapshot().is_err());

    let mut wrong_title = folder.clone();
    let mut wt_val: Value = serde_json::from_slice(wrong_title.read_file("manifest.json").ok_or("missing manifest")?)?;
    wt_val["title"] = json!("Forged Title");
    wrong_title.write_file("manifest.json", serde_json::to_vec(&wt_val)?)?;
    assert!(wrong_title.open_unpacked_snapshot().is_err());

    // 7. Missing declared sheet CSV file rejected
    let mut missing_csv = folder.clone();
    missing_csv.files.remove(&sheet_key);
    assert!(missing_csv.open_unpacked_snapshot().is_err());

    Ok(())
}

#[test]
fn vws_opens_archive_manifest_with_all_five_sections_and_hydrates_image_assets() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let _ = app.execute_command("sheet.createData", json!({ "kind": "xy", "title": "Assay XY" }))?;
    let ds_id = app.document().data_sheets.first().map(|s| s.id.clone()).ok_or("missing data sheet")?;
    for (row, x, y) in [(0usize, 1.0_f64, 2.0_f64), (1, 2.0, 4.0), (2, 3.0, 6.0)] {
        let _ = app.execute_command("table.setCell", json!({ "sheet_id": ds_id.as_str(), "row": row, "col": 0, "subcol": 0, "value": x }))?;
        let _ = app.execute_command("table.setCell", json!({ "sheet_id": ds_id.as_str(), "row": row, "col": 1, "subcol": 0, "value": y }))?;
    }

    // Add InfoSheet, ResultSheet, GraphSheet, and LayoutSheet
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &ds_id));
    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("stats.descriptive"));
    let intent = app.canvas_dialogs.analyze.build_create_new_intent().ok_or("missing analysis intent")?;
    let _ = app.execute_command(&intent.command_id, intent.params)?;
    let _ = app.execute_command("graph.create", json!({ "data_sheet_id": ds_id.as_str(), "title": "Assay Graph" }))?;
    let _ = app.execute_command("layout.create", json!({ "title": "Publication Layout" }))?;

    let mut doc = app.document().clone();
    doc.title = String::from("Full 5-Section Project");
    if doc.info_sheets.is_empty() {
        let mut info = InfoSheet::new_preseeded(SheetId::new("info-1"), "Study Protocol")?;
        info.linked_data_sheets.push(ds_id);
        doc.info_sheets.push(info);
    }

    // Encode valid 2x2 RGBA PNG asset and register it on ProjectDocument
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255])?;
    }
    let asset = ingest_image_asset(&png_bytes)?;
    let _asset_id = doc.upsert_image_asset(asset)?;

    assert!(!doc.data_sheets.is_empty());
    assert!(!doc.info_sheets.is_empty());
    assert!(!doc.result_sheets.is_empty());
    assert!(!doc.graph_sheets.is_empty());
    assert!(!doc.layout_sheets.is_empty());

    let archive_bytes = save_project_bytes(&doc)?;
    let archive = ZipArchive::from_bytes(&archive_bytes)?;
    let mut folder = VirtualWorkspaceFolder::new("full.curvebook.d");
    for (name, content) in archive.iter() {
        folder.write_file(name, content.to_vec())?;
    }

    // Opens cleanly with all 5 sections (where non-Data sections have csv_path: null) and hydrated image asset bytes!
    let reopened = folder.open_unpacked_snapshot()?;
    assert_eq!(reopened.title, "Full 5-Section Project");
    assert_eq!(reopened.data_sheets.len(), doc.data_sheets.len());
    assert_eq!(reopened.info_sheets.len(), doc.info_sheets.len());
    assert_eq!(reopened.result_sheets.len(), doc.result_sheets.len());
    assert_eq!(reopened.graph_sheets.len(), doc.graph_sheets.len());
    assert_eq!(reopened.layout_sheets.len(), doc.layout_sheets.len());
    assert_eq!(reopened.image_assets.len(), 1);
    assert_eq!(reopened.image_assets[0].bytes, png_bytes);

    // Reject missing asset file
    let mut missing_asset = folder.clone();
    let asset_path = folder.list_files().into_iter().find(|p| p.starts_with("assets/")).ok_or("missing assets/ entry")?;
    missing_asset.files.remove(&asset_path);
    assert!(missing_asset.open_unpacked_snapshot().is_err());

    // Reject tampered asset byte_len in manifest.json
    let mut bad_asset_len = folder.clone();
    let mut mval: Value = serde_json::from_slice(bad_asset_len.read_file("manifest.json").ok_or("missing manifest")?)?;
    mval["assets"][0]["byte_len"] = json!(999_999);
    bad_asset_len.write_file("manifest.json", serde_json::to_vec(&mval)?)?;
    assert!(bad_asset_len.open_unpacked_snapshot().is_err());

    // Reject template archive kind in unpacked project open
    let mut wrong_kind = folder.clone();
    let mut kind_val: Value = serde_json::from_slice(wrong_kind.read_file("manifest.json").ok_or("missing manifest")?)?;
    kind_val["kind"] = json!("Template");
    wrong_kind.write_file("manifest.json", serde_json::to_vec(&kind_val)?)?;
    assert!(wrong_kind.open_unpacked_snapshot().is_err());

    // Reject missing metadata section entry from manifest.json
    let mut dropped_meta = folder.clone();
    let mut meta_val: Value = serde_json::from_slice(dropped_meta.read_file("manifest.json").ok_or("missing manifest")?)?;
    if let Some(arr) = meta_val.get_mut("sheets").and_then(Value::as_array_mut) {
        arr.retain(|s| s.get("section").and_then(Value::as_str) != Some("Layouts"));
    }
    dropped_meta.write_file("manifest.json", serde_json::to_vec(&meta_val)?)?;
    assert!(dropped_meta.open_unpacked_snapshot().is_err());

    // Reject SheetSection::Data missing csv_path
    let mut missing_data_csv = folder;
    let mut data_val: Value = serde_json::from_slice(missing_data_csv.read_file("manifest.json").ok_or("missing manifest")?)?;
    if let Some(arr) = data_val.get_mut("sheets").and_then(Value::as_array_mut) {
        for item in arr {
            if item.get("section").and_then(Value::as_str) == Some("Data")
                && let Some(obj) = item.as_object_mut()
            {
                obj.insert(String::from("csv_path"), Value::Null);
            }
        }
    }
    missing_data_csv.write_file("manifest.json", serde_json::to_vec(&data_val)?)?;
    assert!(missing_data_csv.open_unpacked_snapshot().is_err());

    Ok(())
}
