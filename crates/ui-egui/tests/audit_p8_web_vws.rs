#![forbid(unsafe_code)]

use curvebook_doc::CURRENT_SCHEMA_VERSION;
use curvebook_format::ZipLimits;
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
