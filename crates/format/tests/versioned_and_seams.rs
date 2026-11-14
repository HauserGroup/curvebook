//! Integration tests for C3 (atomic writes), C4 (versioned files), C5 (format registry seam),
//! and B5 (persisted Draft 2020-12 JSON Schemas).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lumencraft_doc::{Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, TableArchetype};
use lumencraft_format::{
    BATCH_SCHEMA_V1, CURRENT_JOURNAL_VERSION, CURRENT_LOCK_VERSION, CURRENT_PREFERENCES_VERSION, CrashRecoveryJournal, DirectoryJournalBackend,
    ExportOptions, ExportSource, Exporter, FormatDescriptor, FormatError, FormatKind, FormatRegistry, ImportOptions, ImportPayload, JournalLockLease,
    JournalOpRecord, PreferencesFile, VersionStep, all_persisted_schemas, load_preferences_file, parse_preferences_json, save_preferences_file,
    save_project_file, save_project_unpacked_dir, save_template_file, upgrade, validate_batch_manifest_version,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new() -> std::io::Result<Self> {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("lc-w8b-format-test-{}-{seq}", std::process::id()));
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn sample_project() -> Result<ProjectDocument, FormatError> {
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    let mut sheet = DataSheet::new("data-1", "Cohort Table", TableArchetype::Column(col))?;
    sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(1.5))?;
    sheet.set_cell(CellCoord::new(1, 0, 0), Cell::number(2.5))?;
    Ok(ProjectDocument { title: String::from("Versioned Seams Study"), data_sheets: vec![sheet], ..Default::default() })
}

fn step_1_to_2(val: &mut serde_json::Value) -> Result<(), FormatError> {
    let obj = val.as_object_mut().ok_or_else(|| FormatError::Json(String::from("expected object")))?;
    obj.insert(String::from("v"), serde_json::json!(2));
    obj.insert(String::from("migrated_from_1"), serde_json::json!(true));
    Ok(())
}

fn step_2_to_3(val: &mut serde_json::Value) -> Result<(), FormatError> {
    let obj = val.as_object_mut().ok_or_else(|| FormatError::Json(String::from("expected object")))?;
    obj.insert(String::from("v"), serde_json::json!(3));
    obj.insert(String::from("migrated_from_2"), serde_json::json!(true));
    Ok(())
}

#[test]
fn versioned_upgrade_steps_and_bounds() {
    let steps = [VersionStep { from_version: 1, apply: step_1_to_2 }, VersionStep { from_version: 2, apply: step_2_to_3 }];

    let input = serde_json::json!({ "v": 1, "name": "alpha" });
    let upgraded = upgrade(input, 1, 1, 3, &steps).unwrap();
    assert_eq!(upgraded["v"], 3);
    assert_eq!(upgraded["migrated_from_1"], true);
    assert_eq!(upgraded["migrated_from_2"], true);
    assert_eq!(upgraded["name"], "alpha");

    let already_current = upgrade(serde_json::json!({ "v": 3 }), 3, 1, 3, &steps).unwrap();
    assert_eq!(already_current["v"], 3);

    let err_too_new = upgrade(serde_json::json!({ "v": 4 }), 4, 1, 3, &steps).unwrap_err();
    assert_eq!(err_too_new, FormatError::UnsupportedSchemaVersion { version: 4, max_supported: 3 });

    let err_too_old = upgrade(serde_json::json!({ "v": 0 }), 0, 1, 3, &steps).unwrap_err();
    assert_eq!(err_too_old, FormatError::UnsupportedSchemaVersion { version: 0, max_supported: 3 });

    let err_gap = upgrade(serde_json::json!({ "v": 1 }), 1, 1, 3, &steps[1..]).unwrap_err();
    assert_eq!(err_gap, FormatError::UnsupportedSchemaVersion { version: 1, max_supported: 3 });
}

#[test]
fn preferences_migration_and_refuse_overwrite_newer_version() {
    let legacy_raw = r#"{ "theme": "paper", "shortcuts": { "save": "Cmd+S" }, "recent_limit": 12 }"#;
    let migrated = parse_preferences_json(legacy_raw).unwrap();
    assert_eq!(migrated.version, CURRENT_PREFERENCES_VERSION);
    assert_eq!(migrated.theme.as_deref(), Some("paper"));
    assert_eq!(migrated.shortcuts.get("save").map(String::as_str), Some("Cmd+S"));
    assert_eq!(migrated.extra.get("recent_limit"), Some(&serde_json::json!(12)));

    let future_err = parse_preferences_json(r#"{ "version": 99, "theme": "dark" }"#).unwrap_err();
    assert_eq!(future_err, FormatError::UnsupportedSchemaVersion { version: 99, max_supported: CURRENT_PREFERENCES_VERSION });

    let dir = TempDir::new().unwrap();
    let prefs_path = dir.path().join("nested").join("preferences.json");

    let mut prefs = PreferencesFile {
        locale: Some(String::from("en-US")),
        theme: Some(String::from("light")),
        shortcuts: BTreeMap::from([(String::from("export"), String::from("Cmd+E"))]),
        ..Default::default()
    };
    prefs.extra.insert(String::from("telemetry_opt_in"), serde_json::json!(false));

    save_preferences_file(&prefs_path, &prefs).unwrap();
    let loaded = load_preferences_file(&prefs_path).unwrap();
    assert_eq!(loaded, prefs);

    let newer_disk = r#"{"version":99,"future_feature":{"enabled":true}}"#;
    std::fs::write(&prefs_path, newer_disk).unwrap();

    let overwrite_err = save_preferences_file(&prefs_path, &prefs).unwrap_err();
    assert_eq!(overwrite_err, FormatError::UnsupportedSchemaVersion { version: 99, max_supported: CURRENT_PREFERENCES_VERSION });
    assert_eq!(std::fs::read_to_string(&prefs_path).unwrap(), newer_disk);

    let newer_mem = PreferencesFile { version: 2, ..Default::default() };
    let mem_err = save_preferences_file(&dir.path().join("other.json"), &newer_mem).unwrap_err();
    assert_eq!(mem_err, FormatError::UnsupportedSchemaVersion { version: 2, max_supported: CURRENT_PREFERENCES_VERSION });
}

#[test]
fn batch_manifest_version_validation() {
    assert!(validate_batch_manifest_version(&serde_json::json!({ "schema": BATCH_SCHEMA_V1 })).is_ok());
    assert_eq!(
        validate_batch_manifest_version(&serde_json::json!({ "schema": "lumencraft.batch.v2" })).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 2, max_supported: 1 }
    );
    assert!(matches!(validate_batch_manifest_version(&serde_json::json!({})), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(
        validate_batch_manifest_version(&serde_json::json!({ "schema": "unknown.format" })),
        Err(FormatError::UnsupportedFormat { .. })
    ));
}

#[test]
fn journal_and_lock_version_validation_and_atomic_cleanup() {
    let record = JournalOpRecord::new(1, serde_json::json!({ "op": "rename_sheet" })).unwrap();
    assert_eq!(record.format_version, CURRENT_JOURNAL_VERSION);
    let line = record.to_ndjson_line().unwrap();
    assert_eq!(JournalOpRecord::from_json_str(&line).unwrap(), record);

    let legacy_record_json = serde_json::json!({
        "seq": record.seq,
        "crc32": record.crc32,
        "command_json": record.command_json,
    })
    .to_string();
    assert_eq!(JournalOpRecord::from_json_str(&legacy_record_json).unwrap().format_version, CURRENT_JOURNAL_VERSION);

    let future_record_json = serde_json::json!({
        "format_version": 99,
        "seq": 1,
        "crc32": record.crc32,
        "command_json": { "op": "rename_sheet" },
    })
    .to_string();
    assert_eq!(
        JournalOpRecord::from_json_str(&future_record_json).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 99, max_supported: CURRENT_JOURNAL_VERSION }
    );

    let lease = JournalLockLease::new("session-1", 10_000, 5_000).unwrap();
    assert_eq!(lease.format_version, CURRENT_LOCK_VERSION);
    let future_lease_bytes = serde_json::to_vec(&serde_json::json!({
        "format_version": 5,
        "owner_id": "future-writer",
        "acquired_epoch_ms": 10_000,
        "ttl_ms": 5_000,
    }))
    .unwrap();
    assert_eq!(
        JournalLockLease::from_json_bytes(&future_lease_bytes).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 5, max_supported: CURRENT_LOCK_VERSION }
    );

    // A complete future-version record on the last journal line must fail rather than be discarded as a torn tail.
    let dir = TempDir::new().unwrap();
    let journal_dir = dir.path().join("recovery");
    let backend = DirectoryJournalBackend::open_dir(&journal_dir).unwrap();
    std::fs::write(journal_dir.join(lumencraft_format::JOURNAL_FILE_NAME), format!("{future_record_json}\n")).unwrap();
    let open_err = CrashRecoveryJournal::open_or_recover(backend, "writer-2", 5_000, 12_000).unwrap_err();
    assert_eq!(open_err, FormatError::UnsupportedSchemaVersion { version: 99, max_supported: CURRENT_JOURNAL_VERSION });

    // Future-version lockfile also rejects clean open.
    let lock_dir = dir.path().join("lock_recovery");
    let lock_backend = DirectoryJournalBackend::open_dir(&lock_dir).unwrap();
    std::fs::write(lock_dir.join(lumencraft_format::LOCKFILE_NAME), &future_lease_bytes).unwrap();
    let lock_err = CrashRecoveryJournal::open_or_recover(lock_backend, "writer-3", 5_000, 12_000).unwrap_err();
    assert_eq!(lock_err, FormatError::UnsupportedSchemaVersion { version: 5, max_supported: CURRENT_LOCK_VERSION });

    // Valid open -> renew_lease -> checkpoint atomic writes leave no lingering temp files.
    let clean_dir = dir.path().join("clean_recovery");
    let clean_backend = DirectoryJournalBackend::open_dir(&clean_dir).unwrap();
    let (mut session, _) = CrashRecoveryJournal::open_or_recover(clean_backend, "writer-4", 5_000, 12_000).unwrap();
    session.renew_lease(13_000).unwrap();
    session.checkpoint(b"snap-bytes").unwrap();
    session.close_cleanly(b"snap-bytes").unwrap();

    // Atomic project/template/unpacked writes leave no temp files behind.
    let artifacts_dir = dir.path().join("artifacts");
    std::fs::create_dir_all(&artifacts_dir).unwrap();
    let doc = sample_project().unwrap();
    save_project_file(&artifacts_dir.join("out.lumencraft"), &doc).unwrap();
    save_template_file(&artifacts_dir.join("out.lctemplate"), &doc).unwrap();
    save_project_unpacked_dir(&doc, &artifacts_dir.join("out.lumencraft.d")).unwrap();
    for check_dir in [&artifacts_dir, &clean_dir] {
        let leftovers: Vec<String> = std::fs::read_dir(check_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "unexpected leftover temp files in {}: {leftovers:?}", check_dir.display());
    }
}

#[test]
fn format_registry_lookup_and_roundtrips() {
    let mut reg = FormatRegistry::default_registry();

    let import_ids: Vec<&str> = reg.supported_import_formats().into_iter().map(|d| d.id).collect();
    assert_eq!(import_ids, vec!["lumencraft", "lumencraft.d", "lctemplate", "prism", "pzfx", "csv", "tsv"]);

    let export_ids: Vec<&str> = reg.supported_export_formats().into_iter().map(|d| d.id).collect();
    assert_eq!(export_ids, vec!["lumencraft", "lumencraft.d", "lctemplate", "csv", "tsv", "json", "svg", "pdf", "png", "tiff", "emf"]);

    assert_eq!(reg.importer_by_format("lumencraft").unwrap().descriptor().kind, FormatKind::Archive);
    assert_eq!(reg.importer_by_format(".LUMENCRAFT.D").unwrap().descriptor().id, "lumencraft.d");
    assert_eq!(reg.importer_for_path(Path::new("/tmp/study.lumencraft.d")).unwrap().descriptor().id, "lumencraft.d");
    assert_eq!(reg.importer_for_path(Path::new("STUDY.PZFX")).unwrap().descriptor().id, "pzfx");
    assert_eq!(reg.importer_for_path(Path::new("legacy.prism")).unwrap().descriptor().id, "prism");
    assert_eq!(reg.exporter_by_format("tif").unwrap().descriptor().id, "tiff");
    assert_eq!(reg.exporter_for_path(Path::new("figure.TIFF")).unwrap().descriptor().id, "tiff");
    assert_eq!(reg.exporter_for_path(Path::new("out.lumencraft.d")).unwrap().descriptor().kind, FormatKind::ProjectDir);

    let dir = TempDir::new().unwrap();
    let doc = sample_project().unwrap();

    // Unpacked dir export + import via FormatRegistry seam (both `.lumencraft.d` and content detection).
    let unpacked_path = dir.path().join("bundle.lumencraft.d");
    reg.exporter_for_path(&unpacked_path).unwrap().export_path(ExportSource::Document(&doc), &unpacked_path, &ExportOptions::default()).unwrap();
    let imported_dir_doc =
        reg.importer_for_path(&unpacked_path).unwrap().import_path(&unpacked_path, &ImportOptions::default()).unwrap().into_document().unwrap();
    assert_eq!(imported_dir_doc.title, doc.title);

    // Archive byte round-trip via registry.
    let archive_bytes = reg.exporter_by_format("lumencraft").unwrap().export_bytes(ExportSource::Document(&doc), &ExportOptions::default()).unwrap();
    let archive_doc =
        reg.importer_by_format("lumencraft").unwrap().import_bytes(&archive_bytes, &ImportOptions::default()).unwrap().into_document().unwrap();
    assert_eq!(archive_doc.data_sheets.len(), 1);

    // CSV round-trip via registry.
    let csv_bytes =
        reg.exporter_by_format("csv").unwrap().export_bytes(ExportSource::DataSheet(&doc.data_sheets[0]), &ExportOptions::default()).unwrap();
    let csv_payload = reg.importer_by_format("csv").unwrap().import_bytes(&csv_bytes, &ImportOptions::default()).unwrap();
    assert!(matches!(csv_payload, ImportPayload::DataSheet(_)));

    // Default figure exporter returns UnsupportedFormat; overriding via register_exporter works cleanly.
    let svg_err = reg.exporter_by_format("svg").unwrap().export_bytes(ExportSource::Document(&doc), &ExportOptions::default()).unwrap_err();
    assert!(matches!(svg_err, FormatError::UnsupportedFormat { .. }));

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
    let svg_bytes =
        reg.exporter_for_path(Path::new("plot.svg")).unwrap().export_bytes(ExportSource::Document(&doc), &ExportOptions::default()).unwrap();
    assert_eq!(svg_bytes, b"<svg/>");
}

#[test]
fn persisted_schemas_cover_all_five_draft_2020_12_formats() {
    let schemas = all_persisted_schemas();
    let names: Vec<&str> = schemas.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names,
        vec!["archive-manifest.schema.json", "document.schema.json", "batch-v1.schema.json", "preferences.schema.json", "journal-record.schema.json",]
    );

    for (name, schema) in &schemas {
        let obj = schema.as_object().unwrap_or_else(|| panic!("{name} schema must be an object"));
        assert!(!obj.is_empty(), "{name} schema must not be empty");
        assert_eq!(
            obj.get("$schema").and_then(serde_json::Value::as_str),
            Some("https://json-schema.org/draft/2020-12/schema"),
            "{name} must declare Draft 2020-12 $schema"
        );
        assert!(obj.get("$id").and_then(serde_json::Value::as_str).is_some_and(|s| !s.is_empty()), "{name} missing $id");
        assert!(obj.get("title").and_then(serde_json::Value::as_str).is_some_and(|s| !s.is_empty()), "{name} missing title");
    }

    let (_, batch) = schemas.iter().find(|(n, _)| *n == "batch-v1.schema.json").unwrap();
    assert_eq!(batch["properties"]["schema"]["const"], BATCH_SCHEMA_V1);
    assert!(batch.get("not").is_some());

    let (_, prefs) = schemas.iter().find(|(n, _)| *n == "preferences.schema.json").unwrap();
    assert_eq!(prefs["additionalProperties"], serde_json::json!(true));

    let (_, journal) = schemas.iter().find(|(n, _)| *n == "journal-record.schema.json").unwrap();
    assert_eq!(journal["oneOf"].as_array().map(Vec::len), Some(2));
    assert!(journal["$defs"].get("JournalOpRecord").is_some());
    assert!(journal["$defs"].get("JournalLockLease").is_some());
}
