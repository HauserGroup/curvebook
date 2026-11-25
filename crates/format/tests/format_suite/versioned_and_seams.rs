//! Integration tests for C3 (atomic writes), C4 (versioned files), C5 (format registry seam),
//! and B5 (persisted Draft 2020-12 JSON Schemas).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::common::{TempDirGuard, assert_no_lock_temp_or_evict_leftovers, build_zip_entries, open_dir_journal, sample_column_project};
use curvebook_doc::{CURRENT_SCHEMA_VERSION, ProjectDocument};
use curvebook_format::{
    BATCH_SCHEMA_V1, CURRENT_JOURNAL_VERSION, CURRENT_LOCK_VERSION, CURRENT_PREFERENCES_VERSION, CrashRecoveryJournal, CsvOptions, ExportOptions,
    ExportSource, Exporter, FormatDescriptor, FormatError, FormatKind, FormatRegistry, ImportOptions, ImportPayload, JOURNAL_FILE_NAME,
    JournalBackend, JournalLockLease, JournalOpRecord, LOCKFILE_NAME, MemoryJournalBackend, PreferencesFile, VersionStep, all_persisted_schemas,
    compute_record_crc32, export_datasheet_to_csv, import_prism_bytes, import_pzfx_bytes, instantiate_template_bytes, is_unpacked_project_dir,
    load_preferences_file, normalize_batch_manifest_v1, normalize_batch_manifest_v1_with_warnings, open_archive_bytes, open_project_bytes,
    open_project_unpacked_dir, open_template_bytes, parse_preferences_json, recovery::is_dead_local_owner, save_preferences_file, save_project_file,
    save_project_unpacked_dir, save_template_file, upgrade, validate_batch_manifest_version,
};
use serde_json::{Value, json};

fn step_1_to_2(val: &mut Value) -> Result<(), FormatError> {
    let obj = val.as_object_mut().ok_or_else(|| FormatError::Json(String::from("expected object")))?;
    obj.insert(String::from("v"), json!(2));
    obj.insert(String::from("migrated_from_1"), json!(true));
    Ok(())
}

fn step_2_to_3(val: &mut Value) -> Result<(), FormatError> {
    let obj = val.as_object_mut().ok_or_else(|| FormatError::Json(String::from("expected object")))?;
    obj.insert(String::from("v"), json!(3));
    obj.insert(String::from("migrated_from_2"), json!(true));
    Ok(())
}

#[test]
fn versioned_upgrade_steps_and_bounds() {
    let steps = [VersionStep { from_version: 1, apply: step_1_to_2 }, VersionStep { from_version: 2, apply: step_2_to_3 }];
    let upgraded = upgrade(json!({ "v": 1, "name": "alpha" }), 1, 1, 3, &steps).unwrap();
    assert_eq!(
        (&upgraded["v"], &upgraded["migrated_from_1"], &upgraded["migrated_from_2"], &upgraded["name"]),
        (&json!(3), &json!(true), &json!(true), &json!("alpha"))
    );
    assert_eq!(upgrade(json!({ "v": 3 }), 3, 1, 3, &steps).unwrap()["v"], 3);
    for (ver, slice) in [(4_u32, &steps[..]), (0, &steps[..]), (1, &steps[1..])] {
        assert_eq!(
            upgrade(json!({ "v": ver }), ver, 1, 3, slice).unwrap_err(),
            FormatError::UnsupportedSchemaVersion { version: ver, max_supported: 3 }
        );
    }
}

#[test]
fn preferences_migration_and_refuse_overwrite_newer_version() {
    let migrated = parse_preferences_json(r#"{ "theme": "paper", "shortcuts": { "save": "Cmd+S" }, "recent_limit": 12 }"#).unwrap();
    assert_eq!(
        (migrated.version, migrated.theme.as_deref(), migrated.shortcuts.get("save").map(String::as_str), migrated.extra.get("recent_limit")),
        (CURRENT_PREFERENCES_VERSION, Some("paper"), Some("Cmd+S"), Some(&json!(12)))
    );
    assert_eq!(
        parse_preferences_json(r#"{ "version": 99, "theme": "dark" }"#).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 99, max_supported: CURRENT_PREFERENCES_VERSION }
    );

    let dir = TempDirGuard::new("w8b-format-test").unwrap();
    let prefs_path = dir.child("nested").join("preferences.json");
    let mut prefs = PreferencesFile {
        locale: Some(String::from("en-US")),
        theme: Some(String::from("light")),
        shortcuts: BTreeMap::from([(String::from("export"), String::from("Cmd+E"))]),
        ..Default::default()
    };
    prefs.extra.insert(String::from("telemetry_opt_in"), json!(false));

    save_preferences_file(&prefs_path, &prefs).unwrap();
    assert_eq!(load_preferences_file(&prefs_path).unwrap(), prefs);

    let newer_disk = r#"{"version":99,"future_feature":{"enabled":true}}"#;
    fs::write(&prefs_path, newer_disk).unwrap();
    assert_eq!(
        save_preferences_file(&prefs_path, &prefs).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 99, max_supported: CURRENT_PREFERENCES_VERSION }
    );
    assert_eq!(fs::read_to_string(&prefs_path).unwrap(), newer_disk);

    let newer_mem = PreferencesFile { version: 2, ..Default::default() };
    assert_eq!(
        save_preferences_file(&dir.child("other.json"), &newer_mem).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 2, max_supported: CURRENT_PREFERENCES_VERSION }
    );
}

#[test]
fn batch_manifest_version_validation() {
    assert!(validate_batch_manifest_version(&json!({ "schema": BATCH_SCHEMA_V1 })).is_ok());
    assert_eq!(
        validate_batch_manifest_version(&json!({ "schema": "curvebook.batch.v2" })).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 2, max_supported: 1 }
    );
    for bad in [json!({ "schema": "lumencraft.batch.v1" }), json!({}), json!({ "schema": "unknown.format" })] {
        assert!(matches!(validate_batch_manifest_version(&bad), Err(FormatError::UnsupportedFormat { .. })), "{bad}");
    }
}

#[test]
fn journal_and_lock_version_validation_and_atomic_cleanup() {
    let record = JournalOpRecord::new(1, json!({ "op": "rename_sheet" })).unwrap();
    assert_eq!(
        (record.format_version, JournalOpRecord::from_json_str(&record.to_ndjson_line().unwrap()).unwrap()),
        (CURRENT_JOURNAL_VERSION, record.clone())
    );

    let legacy_rec = json!({ "seq": record.seq, "crc32": record.crc32, "command_json": record.command_json }).to_string();
    assert_eq!(JournalOpRecord::from_json_str(&legacy_rec).unwrap().format_version, 1);

    let future_rec = json!({ "format_version": 99, "seq": 1, "crc32": record.crc32, "command_json": { "op": "rename_sheet" } }).to_string();
    assert_eq!(
        JournalOpRecord::from_json_str(&future_rec).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 99, max_supported: CURRENT_JOURNAL_VERSION }
    );

    assert_eq!(JournalLockLease::new("session-1", 10_000, 5_000).unwrap().format_version, CURRENT_LOCK_VERSION);
    let future_lease =
        serde_json::to_vec(&json!({ "format_version": 5, "owner_id": "future-writer", "acquired_epoch_ms": 10_000, "ttl_ms": 5_000 })).unwrap();
    assert_eq!(
        JournalLockLease::from_json_bytes(&future_lease).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 5, max_supported: CURRENT_LOCK_VERSION }
    );

    let dir = TempDirGuard::new("w8b-format-test").unwrap();
    for (sub, fname, bytes, expected_ver, max_ver) in [
        ("recovery", JOURNAL_FILE_NAME, format!("{future_rec}\n").into_bytes(), 99, CURRENT_JOURNAL_VERSION),
        ("lock_recovery", LOCKFILE_NAME, future_lease, 5, CURRENT_LOCK_VERSION),
    ] {
        let d = dir.child(sub);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(fname), bytes).unwrap();
        assert_eq!(
            open_dir_journal(&d, "writer", 5_000, 12_000).unwrap_err(),
            FormatError::UnsupportedSchemaVersion { version: expected_ver, max_supported: max_ver }
        );
    }

    let clean_dir = dir.child("clean_recovery");
    let (mut session, _) = open_dir_journal(&clean_dir, "writer-4", 5_000, 12_000).unwrap();
    session.renew_lease(13_000).unwrap();
    session.checkpoint(b"snap-bytes").unwrap();
    session.close_cleanly(b"snap-bytes").unwrap();

    let artifacts_dir = dir.child("artifacts");
    fs::create_dir_all(&artifacts_dir).unwrap();
    let doc = sample_column_project("Versioned Seams Study");
    save_project_file(&artifacts_dir.join("out.curvebook"), &doc).unwrap();
    save_template_file(&artifacts_dir.join("out.cbtemplate"), &doc).unwrap();
    save_project_unpacked_dir(&doc, &artifacts_dir.join("out.curvebook.d")).unwrap();
    assert_no_lock_temp_or_evict_leftovers(&artifacts_dir);
    assert_no_lock_temp_or_evict_leftovers(&clean_dir);
}

#[test]
fn format_registry_lookup_and_roundtrips() {
    let mut reg = FormatRegistry::default_registry();
    assert_eq!(
        reg.supported_import_formats().into_iter().map(|d| d.id).collect::<Vec<_>>(),
        vec!["curvebook", "curvebook.d", "cbtemplate", "prism", "pzfx", "xlsx", "csv", "tsv"]
    );
    assert_eq!(
        reg.supported_export_formats().into_iter().map(|d| d.id).collect::<Vec<_>>(),
        vec!["curvebook", "curvebook.d", "cbtemplate", "csv", "tsv", "json", "svg", "pdf", "png", "tiff", "emf"]
    );

    assert_eq!(
        (
            reg.importer_by_format("curvebook").unwrap().descriptor().kind,
            reg.importer_by_format(".CURVEBOOK.D").unwrap().descriptor().id,
            reg.exporter_by_format("tif").unwrap().descriptor().id
        ),
        (FormatKind::Archive, "curvebook.d", "tiff")
    );
    for (p, id) in [("nested/study.curvebook.d", "curvebook.d"), ("STUDY.PZFX", "pzfx"), ("legacy.prism", "prism"), ("workbook.XLSX", "xlsx")] {
        assert_eq!(reg.importer_for_path(Path::new(p)).unwrap().descriptor().id, id, "{p}");
    }
    assert_eq!(
        (
            reg.exporter_for_path(Path::new("figure.TIFF")).unwrap().descriptor().id,
            reg.exporter_for_path(Path::new("out.curvebook.d")).unwrap().descriptor().kind
        ),
        ("tiff", FormatKind::ProjectDir)
    );

    let dir = TempDirGuard::new("w8b-format-test").unwrap();
    let doc = sample_column_project("Versioned Seams Study");
    let unpacked = dir.child("bundle.curvebook.d");
    reg.exporter_for_path(&unpacked).unwrap().export_path(ExportSource::Document(&doc), &unpacked, &ExportOptions::default()).unwrap();
    assert_eq!(
        reg.importer_for_path(&unpacked).unwrap().import_path(&unpacked, &ImportOptions::default()).unwrap().into_document().unwrap().title,
        doc.title
    );

    let archive_bytes = reg.exporter_by_format("curvebook").unwrap().export_bytes(ExportSource::Document(&doc), &ExportOptions::default()).unwrap();
    assert_eq!(
        reg.importer_by_format("curvebook")
            .unwrap()
            .import_bytes(&archive_bytes, &ImportOptions::default())
            .unwrap()
            .into_document()
            .unwrap()
            .data_sheets
            .len(),
        1
    );

    let csv_bytes =
        reg.exporter_by_format("csv").unwrap().export_bytes(ExportSource::DataSheet(&doc.data_sheets[0]), &ExportOptions::default()).unwrap();
    assert!(matches!(
        reg.importer_by_format("csv").unwrap().import_bytes(&csv_bytes, &ImportOptions::default()).unwrap(),
        ImportPayload::DataSheet(_)
    ));
    assert!(matches!(
        reg.exporter_by_format("svg").unwrap().export_bytes(ExportSource::Document(&doc), &ExportOptions::default()),
        Err(FormatError::UnsupportedFormat { .. })
    ));

    struct CustomSvgExporter;
    impl Exporter for CustomSvgExporter {
        fn descriptor(&self) -> FormatDescriptor {
            FormatDescriptor { id: "svg", label: "Custom SVG", extensions: &["svg"], kind: FormatKind::FigureVector }
        }
        fn export_bytes(&self, _source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
            Ok(b"<svg/>".to_vec())
        }
    }
    reg.register_exporter(Arc::new(CustomSvgExporter));
    assert_eq!(
        reg.exporter_for_path(Path::new("plot.svg")).unwrap().export_bytes(ExportSource::Document(&doc), &ExportOptions::default()).unwrap(),
        b"<svg/>"
    );
}

#[test]
fn persisted_schemas_cover_all_five_draft_2020_12_formats() {
    let schemas = all_persisted_schemas();
    assert_eq!(
        schemas.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        vec!["archive-manifest.schema.json", "document.schema.json", "batch-v1.schema.json", "preferences.schema.json", "journal-record.schema.json"]
    );
    for (name, schema) in &schemas {
        let obj = schema.as_object().unwrap();
        assert!(
            !obj.is_empty()
                && obj.get("$schema").and_then(Value::as_str) == Some("https://json-schema.org/draft/2020-12/schema")
                && obj.get("$id").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
                && obj.get("title").and_then(Value::as_str).is_some_and(|s| !s.is_empty()),
            "{name}"
        );
    }
    let find = |k: &str| &schemas.iter().find(|(n, _)| *n == k).unwrap().1;
    assert!(find("batch-v1.schema.json")["properties"]["schema"]["const"] == BATCH_SCHEMA_V1 && find("batch-v1.schema.json").get("not").is_some());
    assert_eq!(find("preferences.schema.json")["additionalProperties"], json!(true));
    let journal = find("journal-record.schema.json");
    assert!(
        journal["oneOf"].as_array().map(Vec::len) == Some(2)
            && journal["$defs"].get("JournalOpRecord").is_some()
            && journal["$defs"].get("JournalLockLease").is_some()
    );
}

fn synthesize_legacy_archive_zip(doc: &ProjectDocument, kind: &str) -> Vec<u8> {
    let ds0 = &doc.data_sheets[0];
    let doc_bytes = serde_json::to_vec_pretty(doc).unwrap();
    let csv_bytes = export_datasheet_to_csv(ds0, &CsvOptions::csv()).unwrap().into_bytes();
    let manifest = json!({
        "format": "lumencraft-archive", "schema_version": CURRENT_SCHEMA_VERSION, "kind": kind, "title": doc.title,
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": [{ "id": ds0.id, "section": "Data", "title": ds0.title, "csv_path": "sheets/data-1.csv", "crc32": crc32fast::hash(&csv_bytes) }]
    });
    build_zip_entries(&[
        ("document.json", &doc_bytes),
        ("sheets/data-1.csv", &csv_bytes),
        ("manifest.json", &serde_json::to_vec_pretty(&manifest).unwrap()),
    ])
}

#[test]
fn explicit_rejection_of_legacy_lumencraft_archives_and_renamed_interchange_files() {
    let reg = FormatRegistry::default_registry();
    for id in ["lumencraft", ".lumencraft", "lumencraft.d", ".lumencraft.d", "lctemplate", ".lctemplate"] {
        assert!(reg.importer_by_format(id).is_none() && reg.exporter_by_format(id).is_none(), "{id}");
    }
    for p in ["legacy.lumencraft", "legacy.lctemplate", "legacy.lumencraft.d"] {
        assert!(reg.importer_for_path(Path::new(p)).is_none() && reg.exporter_for_path(Path::new(p)).is_none(), "{p}");
    }

    let doc = sample_column_project("Versioned Seams Study");
    let (legacy_proj, legacy_tpl) = (synthesize_legacy_archive_zip(&doc, "Project"), synthesize_legacy_archive_zip(&doc, "Template"));
    assert!(matches!(open_project_bytes(&legacy_proj), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(open_archive_bytes(&legacy_proj), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(open_template_bytes(&legacy_tpl), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(instantiate_template_bytes(&legacy_tpl, Some("Renamed")), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(import_prism_bytes(&legacy_proj), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(import_prism_bytes(&legacy_tpl), Err(FormatError::UnsupportedFormat { .. })));
    assert!(import_pzfx_bytes(&legacy_proj).is_err());

    let dir = TempDirGuard::new("w8b-format-test").unwrap();
    for (suffix, bytes) in [
        ("renamed.curvebook", &legacy_proj),
        ("renamed.cbtemplate", &legacy_tpl),
        ("renamed.prism", &legacy_proj),
        ("renamed_tpl.prism", &legacy_tpl),
        ("renamed.pzfx", &legacy_proj),
        ("renamed.csv", &legacy_proj),
        ("renamed.tsv", &legacy_proj),
    ] {
        let path = dir.child(suffix);
        fs::write(&path, bytes).unwrap();
        assert!(reg.importer_for_path(&path).unwrap().import_path(&path, &ImportOptions::default()).is_err(), "{suffix}");
    }
}

#[test]
fn explicit_rejection_of_legacy_unpacked_dirs_batch_manifests_and_owner_pids() {
    let reg = FormatRegistry::default_registry();
    let doc = sample_column_project("Versioned Seams Study");
    let dir = TempDirGuard::new("w8b-format-test").unwrap();

    let doc_bytes = serde_json::to_vec_pretty(&doc).unwrap();
    let legacy_manifest = serde_json::to_vec_pretty(&json!({ "format": "lumencraft-archive", "schema_version": CURRENT_SCHEMA_VERSION, "kind": "Project", "title": doc.title, "document_crc32": crc32fast::hash(&doc_bytes), "sheets": [] })).unwrap();
    for dir_name in ["old.lumencraft.d", "renamed.curvebook.d"] {
        let unpacked = dir.child(dir_name);
        fs::create_dir_all(&unpacked).unwrap();
        fs::write(unpacked.join("document.json"), &doc_bytes).unwrap();
        fs::write(unpacked.join("manifest.json"), &legacy_manifest).unwrap();
        assert!(!is_unpacked_project_dir(&unpacked) && matches!(open_project_unpacked_dir(&unpacked), Err(FormatError::UnsupportedFormat { .. })));
    }
    assert!(reg.importer_for_path(&dir.child("old.lumencraft.d")).is_none());
    let renamed = dir.child("renamed.curvebook.d");
    assert!(matches!(
        reg.importer_for_path(&renamed).unwrap().import_path(&renamed, &ImportOptions::default()),
        Err(FormatError::UnsupportedFormat { .. })
    ));

    let legacy_batch = serde_json::to_vec(&json!({ "schema": "lumencraft.batch.v1", "commands": [] })).unwrap();
    for s in ["lumencraft.batch.v1", "lumencraft.batch.v2"] {
        assert!(matches!(validate_batch_manifest_version(&json!({ "schema": s })), Err(FormatError::UnsupportedFormat { .. })));
    }
    for suffix in ["batch.curvebook", "batch.cbtemplate", "batch.prism", "batch.pzfx"] {
        let path = dir.child(suffix);
        fs::write(&path, &legacy_batch).unwrap();
        assert!(reg.importer_for_path(&path).unwrap().import_path(&path, &ImportOptions::default()).is_err(), "{suffix}");
    }
    assert!(!is_dead_local_owner("lumencraft-pid-999999999"));
}

#[test]
fn journal_v1_crc_verified_before_v2_upgrade_and_batch_manifest_normalizes() {
    let v1_cmd = json!({ "command_id": "sheet.rename", "params": { "id": "data_1", "new_title": "Normalized Title" } });
    let v1_crc = compute_record_crc32(1, &v1_cmd).unwrap();
    let v1_line = json!({ "format_version": 1, "seq": 1, "crc32": v1_crc, "command_json": v1_cmd }).to_string();

    let mut mem = MemoryJournalBackend::new();
    mem.append_journal_line(&v1_line).unwrap();
    let (mut journal, report) = CrashRecoveryJournal::open_or_recover(mem, "migrator", 10_000, 1_000).unwrap();
    assert_eq!(
        report.recovered_commands,
        vec![json!({ "command_id": "sheet.rename", "params": { "sheet_id": "data_1", "title": "Normalized Title" } })]
    );

    journal.record_command(json!({ "command_id": "sheet.freeze", "params": { "sheet_id": "data_1" } })).unwrap();
    let tail = JournalOpRecord::from_json_str(std::str::from_utf8(journal.backend().journal_bytes()).unwrap().lines().last().unwrap()).unwrap();
    assert_eq!(tail.format_version, 2);

    let mut bad_rec = JournalOpRecord { format_version: 1, seq: 1, crc32: v1_crc ^ 0xFFFF, command_json: v1_cmd.clone() };
    assert!(matches!(bad_rec.upgrade_to_current(), Err(FormatError::CorruptCrc { .. })));

    let mut tampered = MemoryJournalBackend::new();
    tampered.append_journal_line(&json!({ "format_version": 1, "seq": 1, "crc32": v1_crc ^ 1, "command_json": v1_cmd }).to_string()).unwrap();
    tampered.append_journal_line(&tail.to_ndjson_line().unwrap()).unwrap();
    assert!(matches!(CrashRecoveryJournal::open_or_recover(tampered, "migrator-bad", 10_000, 1_000), Err(FormatError::CorruptCrc { .. })));

    let mut manifest = json!({
        "schema": BATCH_SCHEMA_V1,
        "inputs": [{ "csv": "in.csv", "sheetId": "s1", "tableKind": "xy", "subcolumnMode": "replicate", "decimalSeparator": "." }],
        "commands": [{ "command": "graph.create", "params": { "id": "g1", "source_sheet_id": "s1" } }, { "method": "sheet.rename", "params": { "id": "s1", "new_title": "Dose" } }],
        "exports": [{ "out": "out.pdf", "sheetId": "g1", "tabIndex": 0, "pdfVersion": "1.7", "ttfFont": "font.ttf" }]
    });
    assert!(normalize_batch_manifest_v1_with_warnings(&mut manifest).unwrap().len() >= 8);
    assert_eq!(
        manifest,
        json!({
            "schema": BATCH_SCHEMA_V1,
            "inputs": [{ "csv": "in.csv", "sheet": "s1", "kind": "xy", "subcolumn_mode": "replicate", "decimal_separator": "." }],
            "commands": [{ "cmd": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "s1" } }, { "cmd": "sheet.rename", "params": { "sheet_id": "s1", "title": "Dose" } }],
            "exports": [{ "out": "out.pdf", "sheet": "g1", "tab_index": 0, "pdf_version": "1.7", "ttf_font": "font.ttf" }]
        })
    );
    assert!(
        normalize_batch_manifest_v1(&mut manifest).is_ok() && normalize_batch_manifest_v1(&mut json!({ "schema": "curvebook.batch.v2" })).is_err()
    );

    let mut ann = json!({ "schema": BATCH_SCHEMA_V1, "commands": [{ "cmd": "graph.addAnnotation", "params": { "id": "g1", "type": "shape", "color": "#ff0000", "rect_pt": { "x": 10.0, "y": 10.0, "width": 50.0, "height": 20.0 } } }] });
    assert_eq!(normalize_batch_manifest_v1_with_warnings(&mut ann).unwrap().len(), 4);
    assert_eq!(
        ann["commands"][0]["params"],
        json!({ "sheet_id": "g1", "kind": "shape", "stroke_color": "#ff0000", "bounds_pt": { "x": 10.0, "y": 10.0, "width": 50.0, "height": 20.0 } })
    );
}
