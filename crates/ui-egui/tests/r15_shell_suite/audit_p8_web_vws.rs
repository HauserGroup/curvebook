#![forbid(unsafe_code)]

use curvebook_doc::{CURRENT_SCHEMA_VERSION, InfoSheet, SheetId};
use curvebook_format::{ZipArchive, ZipLimits, ingest_image_asset, save_project_bytes};
use curvebook_ui_egui::serde_json::{self, Value, json};
use curvebook_ui_egui::{UiApp, VirtualWorkspaceFolder};

fn folder_from_archive(doc: &curvebook_doc::ProjectDocument, name: &str) -> Result<VirtualWorkspaceFolder, Box<dyn std::error::Error>> {
    let bytes = save_project_bytes(doc)?;
    let archive = ZipArchive::from_bytes(&bytes)?;
    let mut folder = VirtualWorkspaceFolder::new(name);
    for (entry, content) in archive.iter() {
        folder.write_file(entry, content.to_vec())?;
    }
    Ok(folder)
}

fn assert_manifest_mutation_rejected(folder: &VirtualWorkspaceFolder, mutate: impl FnOnce(&mut Value)) -> Result<(), Box<dyn std::error::Error>> {
    let mut cloned = folder.clone();
    let mut val: Value = serde_json::from_slice(cloned.read_file("manifest.json").ok_or("missing manifest")?)?;
    mutate(&mut val);
    cloned.write_file("manifest.json", serde_json::to_vec(&val)?)?;
    assert!(cloned.open_unpacked_snapshot().is_err(), "tampered manifest must be rejected");
    Ok(())
}

#[test]
fn vws_enforces_entry_byte_and_count_caps() {
    let limits = ZipLimits::default();
    let max_entry = usize::try_from(limits.max_entry_uncompressed_bytes).unwrap_or(usize::MAX);
    let mut folder = VirtualWorkspaceFolder::new("caps.curvebook.d");

    let err = folder.write_file("big.bin", vec![b'a'; max_entry + 1]).expect_err("entry > 64 MiB must fail");
    assert!(err.contains("per-entry byte limit"), "unexpected error: {err}");

    let chunk = vec![b'x'; 32 * 1024 * 1024];
    for idx in 0..4 {
        folder.write_file(&format!("part_{idx}.bin"), chunk.clone()).expect("4 x 32 MiB fits within 128 MiB");
    }
    let total_err = folder.write_file("part_4.bin", vec![b'z']).expect_err("> 128 MiB total must fail");
    assert!(total_err.contains("total byte budget"), "unexpected error: {total_err}");

    folder.write_file("part_3.bin", vec![b'y'; 16]).expect("shrinking overwrite succeeds");
    folder.write_file("extra.txt", b"ok".to_vec()).expect("small write after shrink succeeds");

    let mut too_many = VirtualWorkspaceFolder::new("many.curvebook.d");
    for idx in 0..=limits.max_entries {
        too_many.files.insert(format!("items/f_{idx}.txt"), b"x".to_vec());
    }
    assert!(too_many.open_unpacked_snapshot().expect_err("> 4096 entries must fail").contains("maximum entry count"));
}

#[test]
fn vws_validates_manifest_crc_schema_and_declared_sheets() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "kind": "xy", "title": "XY Data" }))?;
    let mut doc = app.document().clone();
    doc.title = String::from("VWS Valid");
    let mut folder = VirtualWorkspaceFolder::new("valid.curvebook.d");
    folder.save_unpacked_snapshot(&doc)?;
    assert_eq!(folder.open_unpacked_snapshot()?.title, "VWS Valid");

    // 1. Missing or malformed manifest rejected
    let mut missing = folder.clone();
    missing.files.remove("manifest.json");
    assert!(missing.open_unpacked_snapshot().expect_err("missing manifest").contains("manifest.json"));
    for bad in [&b"not-json"[..], &b"[]"[..]] {
        let mut f = folder.clone();
        f.write_file("manifest.json", bad.to_vec())?;
        assert!(f.open_unpacked_snapshot().is_err());
    }

    // 2. Archive manifest loads cleanly and rejects tampered document.json or sheet CSV CRC
    let crc_folder = folder_from_archive(&doc, "crc.curvebook.d")?;
    assert_eq!(crc_folder.open_unpacked_snapshot()?.title, "VWS Valid");

    let mut tampered_doc = crc_folder.clone();
    let doc_text = String::from_utf8(tampered_doc.read_file("document.json").ok_or("missing doc")?.to_vec())?;
    tampered_doc.write_file("document.json", format!("{doc_text} ").into_bytes())?;
    assert!(tampered_doc.open_unpacked_snapshot().expect_err("tampered doc").contains("CRC-32 mismatch"));

    let sheet_key = crc_folder.list_files().into_iter().find(|p| p.starts_with("sheets/")).ok_or("missing sheet csv")?;
    let mut tampered_csv = crc_folder.clone();
    tampered_csv.write_file(&sheet_key, b"X,Y\n999,888\n".to_vec())?;
    assert!(tampered_csv.open_unpacked_snapshot().expect_err("tampered csv").contains("CRC-32 mismatch"));

    // 3. Schema version, legacy magic, forged sheets/title, and missing declared sheet CSV rejected
    assert_manifest_mutation_rejected(&folder, |m| m["schema_version"] = json!(CURRENT_SCHEMA_VERSION + 1))?;
    assert_manifest_mutation_rejected(&crc_folder, |m| m["format"] = json!("legacy-v0-magic"))?;
    assert_manifest_mutation_rejected(&folder, |m| m["sheets"] = json!([]))?;
    assert_manifest_mutation_rejected(&folder, |m| m["title"] = json!("Forged Title"))?;

    let mut missing_csv = folder;
    missing_csv.files.remove(&sheet_key);
    assert!(missing_csv.open_unpacked_snapshot().is_err());
    Ok(())
}

#[test]
fn vws_opens_archive_manifest_with_all_five_sections_and_hydrates_image_assets() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "kind": "xy", "title": "Assay XY" }))?;
    let ds_id = app.document().data_sheets.first().map(|s| s.id.clone()).ok_or("missing data sheet")?;
    for (row, x, y) in [(0usize, 1.0, 2.0), (1, 2.0, 4.0), (2, 3.0, 6.0)] {
        app.execute_command("table.setCell", json!({ "sheet_id": ds_id.as_str(), "row": row, "col": 0, "subcol": 0, "value": x }))?;
        app.execute_command("table.setCell", json!({ "sheet_id": ds_id.as_str(), "row": row, "col": 1, "subcol": 0, "value": y }))?;
    }
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &ds_id));
    app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("stats.descriptive"));
    let intent = app.canvas_dialogs.analyze.build_create_new_intent().ok_or("missing analysis intent")?;
    app.execute_command(&intent.command_id, intent.params)?;
    app.execute_command("graph.create", json!({ "data_sheet_id": ds_id.as_str(), "title": "Assay Graph" }))?;
    app.execute_command("layout.create", json!({ "title": "Publication Layout" }))?;

    let mut doc = app.document().clone();
    doc.title = String::from("Full 5-Section Project");
    if doc.info_sheets.is_empty() {
        let mut info = InfoSheet::new_preseeded(SheetId::new("info-1"), "Study Protocol")?;
        info.linked_data_sheets.push(ds_id);
        doc.info_sheets.push(info);
    }

    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, 2, 2);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255])?;
    }
    doc.upsert_image_asset(ingest_image_asset(&png_bytes)?)?;
    assert!(
        !doc.data_sheets.is_empty()
            && !doc.info_sheets.is_empty()
            && !doc.result_sheets.is_empty()
            && !doc.graph_sheets.is_empty()
            && !doc.layout_sheets.is_empty()
    );

    let folder = folder_from_archive(&doc, "full.curvebook.d")?;
    let reopened = folder.open_unpacked_snapshot()?;
    assert_eq!(reopened.title, "Full 5-Section Project");
    assert_eq!(
        (
            reopened.data_sheets.len(),
            reopened.info_sheets.len(),
            reopened.result_sheets.len(),
            reopened.graph_sheets.len(),
            reopened.layout_sheets.len()
        ),
        (doc.data_sheets.len(), doc.info_sheets.len(), doc.result_sheets.len(), doc.graph_sheets.len(), doc.layout_sheets.len()),
    );
    assert_eq!((reopened.image_assets.len(), reopened.image_assets[0].bytes.as_slice()), (1, png_bytes.as_slice()));

    let mut missing_asset = folder.clone();
    let asset_path = folder.list_files().into_iter().find(|p| p.starts_with("assets/")).ok_or("missing assets/ entry")?;
    missing_asset.files.remove(&asset_path);
    assert!(missing_asset.open_unpacked_snapshot().is_err());

    assert_manifest_mutation_rejected(&folder, |m| m["assets"][0]["byte_len"] = json!(999_999))?;
    assert_manifest_mutation_rejected(&folder, |m| m["kind"] = json!("Template"))?;
    assert_manifest_mutation_rejected(&folder, |m| {
        if let Some(arr) = m.get_mut("sheets").and_then(Value::as_array_mut) {
            arr.retain(|s| s.get("section").and_then(Value::as_str) != Some("Layouts"));
        }
    })?;
    assert_manifest_mutation_rejected(&folder, |m| {
        if let Some(arr) = m.get_mut("sheets").and_then(Value::as_array_mut) {
            for item in arr {
                if item.get("section").and_then(Value::as_str) == Some("Data")
                    && let Some(obj) = item.as_object_mut()
                {
                    obj.insert(String::from("csv_path"), Value::Null);
                }
            }
        }
    })?;
    Ok(())
}
