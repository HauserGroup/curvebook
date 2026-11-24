//! Integration tests for C3 (atomic writes), C4 (versioned files), C5 (format registry seam),
//! and B5 (persisted Draft 2020-12 JSON Schemas).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{CURRENT_SCHEMA_VERSION, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, TableArchetype};
use curvebook_format::{
    BATCH_SCHEMA_V1, CURRENT_JOURNAL_VERSION, CURRENT_LOCK_VERSION, CURRENT_PREFERENCES_VERSION, CrashRecoveryJournal, CsvOptions,
    DirectoryJournalBackend, ExportOptions, ExportSource, Exporter, FormatDescriptor, FormatError, FormatKind, FormatRegistry, ImportOptions,
    ImportPayload, JournalLockLease, JournalOpRecord, PreferencesFile, VersionStep, ZipWriter, all_persisted_schemas, export_datasheet_to_csv,
    import_prism_bytes, import_pzfx_bytes, instantiate_template_bytes, is_unpacked_project_dir, load_preferences_file, open_archive_bytes,
    open_project_bytes, open_project_unpacked_dir, open_template_bytes, parse_preferences_json, recovery::is_dead_local_owner, save_preferences_file,
    save_project_file, save_project_unpacked_dir, save_template_file, upgrade, validate_batch_manifest_version,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new() -> std::io::Result<Self> {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-w8b-format-test-{}-{seq}", std::process::id()));
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
        validate_batch_manifest_version(&serde_json::json!({ "schema": "curvebook.batch.v2" })).unwrap_err(),
        FormatError::UnsupportedSchemaVersion { version: 2, max_supported: 1 }
    );
    assert!(matches!(
        validate_batch_manifest_version(&serde_json::json!({ "schema": "lumencraft.batch.v1" })),
        Err(FormatError::UnsupportedFormat { .. })
    ));
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
    assert_eq!(JournalOpRecord::from_json_str(&legacy_record_json).unwrap().format_version, 1);

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
    std::fs::write(journal_dir.join(curvebook_format::JOURNAL_FILE_NAME), format!("{future_record_json}\n")).unwrap();
    let open_err = CrashRecoveryJournal::open_or_recover(backend, "writer-2", 5_000, 12_000).unwrap_err();
    assert_eq!(open_err, FormatError::UnsupportedSchemaVersion { version: 99, max_supported: CURRENT_JOURNAL_VERSION });

    // Future-version lockfile also rejects clean open.
    let lock_dir = dir.path().join("lock_recovery");
    let lock_backend = DirectoryJournalBackend::open_dir(&lock_dir).unwrap();
    std::fs::write(lock_dir.join(curvebook_format::LOCKFILE_NAME), &future_lease_bytes).unwrap();
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
    save_project_file(&artifacts_dir.join("out.curvebook"), &doc).unwrap();
    save_template_file(&artifacts_dir.join("out.cbtemplate"), &doc).unwrap();
    save_project_unpacked_dir(&doc, &artifacts_dir.join("out.curvebook.d")).unwrap();
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
    assert_eq!(import_ids, vec!["curvebook", "curvebook.d", "cbtemplate", "prism", "pzfx", "xlsx", "csv", "tsv"]);

    let export_ids: Vec<&str> = reg.supported_export_formats().into_iter().map(|d| d.id).collect();
    assert_eq!(export_ids, vec!["curvebook", "curvebook.d", "cbtemplate", "csv", "tsv", "json", "svg", "pdf", "png", "tiff", "emf"]);

    assert_eq!(reg.importer_by_format("curvebook").unwrap().descriptor().kind, FormatKind::Archive);
    assert_eq!(reg.importer_by_format(".CURVEBOOK.D").unwrap().descriptor().id, "curvebook.d");
    assert_eq!(reg.importer_for_path(Path::new("nested/study.curvebook.d")).unwrap().descriptor().id, "curvebook.d");
    assert_eq!(reg.importer_for_path(Path::new("STUDY.PZFX")).unwrap().descriptor().id, "pzfx");
    assert_eq!(reg.importer_for_path(Path::new("legacy.prism")).unwrap().descriptor().id, "prism");
    assert_eq!(reg.importer_for_path(Path::new("workbook.XLSX")).unwrap().descriptor().id, "xlsx");
    assert_eq!(reg.exporter_by_format("tif").unwrap().descriptor().id, "tiff");
    assert_eq!(reg.exporter_for_path(Path::new("figure.TIFF")).unwrap().descriptor().id, "tiff");
    assert_eq!(reg.exporter_for_path(Path::new("out.curvebook.d")).unwrap().descriptor().kind, FormatKind::ProjectDir);

    let dir = TempDir::new().unwrap();
    let doc = sample_project().unwrap();

    // Unpacked dir export + import via FormatRegistry seam (both `.curvebook.d` and content detection).
    let unpacked_path = dir.path().join("bundle.curvebook.d");
    reg.exporter_for_path(&unpacked_path).unwrap().export_path(ExportSource::Document(&doc), &unpacked_path, &ExportOptions::default()).unwrap();
    let imported_dir_doc =
        reg.importer_for_path(&unpacked_path).unwrap().import_path(&unpacked_path, &ImportOptions::default()).unwrap().into_document().unwrap();
    assert_eq!(imported_dir_doc.title, doc.title);

    // Archive byte round-trip via registry.
    let archive_bytes = reg.exporter_by_format("curvebook").unwrap().export_bytes(ExportSource::Document(&doc), &ExportOptions::default()).unwrap();
    let archive_doc =
        reg.importer_by_format("curvebook").unwrap().import_bytes(&archive_bytes, &ImportOptions::default()).unwrap().into_document().unwrap();
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

fn synthesize_legacy_archive_zip(doc: &ProjectDocument, kind: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let ds0 = doc.data_sheets.first().ok_or("missing primary data sheet")?;
    let doc_bytes = serde_json::to_vec_pretty(doc)?;
    let csv_bytes = export_datasheet_to_csv(ds0, &CsvOptions::csv())?.into_bytes();
    let manifest = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": kind,
        "title": doc.title,
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": [{
            "id": ds0.id,
            "section": "Data",
            "title": ds0.title,
            "csv_path": "sheets/data-1.csv",
            "crc32": crc32fast::hash(&csv_bytes),
        }]
    });
    let mut writer = ZipWriter::new();
    writer.add_file("document.json", doc_bytes)?;
    writer.add_file("sheets/data-1.csv", csv_bytes)?;
    writer.add_file("manifest.json", serde_json::to_vec_pretty(&manifest)?)?;
    Ok(writer.finish()?)
}

#[test]
fn explicit_rejection_of_legacy_lumencraft_archives_and_renamed_interchange_files() {
    let reg = FormatRegistry::default_registry();

    // 1. Legacy format IDs and extensions are absent from FormatRegistry lookup & path dispatch.
    for legacy_id in ["lumencraft", ".lumencraft", "lumencraft.d", ".lumencraft.d", "lctemplate", ".lctemplate"] {
        assert!(reg.importer_by_format(legacy_id).is_none(), "unexpected importer for `{legacy_id}`");
        assert!(reg.exporter_by_format(legacy_id).is_none(), "unexpected exporter for `{legacy_id}`");
    }
    for legacy_path in ["legacy.lumencraft", "legacy.lctemplate", "legacy.lumencraft.d"] {
        assert!(reg.importer_for_path(Path::new(legacy_path)).is_none(), "unexpected path importer for `{legacy_path}`");
        assert!(reg.exporter_for_path(Path::new(legacy_path)).is_none(), "unexpected path exporter for `{legacy_path}`");
    }

    // 2. Synthesized legacy `lumencraft-archive` project & template ZIPs fail explicitly everywhere,
    //    including when renamed to `.curvebook`, `.cbtemplate`, `.prism`, `.pzfx`, `.csv`, or `.tsv`.
    let doc = sample_project().expect("sample project");
    let legacy_project_zip = synthesize_legacy_archive_zip(&doc, "Project").expect("build legacy project zip");
    let legacy_template_zip = synthesize_legacy_archive_zip(&doc, "Template").expect("build legacy template zip");

    assert!(matches!(open_project_bytes(&legacy_project_zip), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(open_archive_bytes(&legacy_project_zip), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(open_template_bytes(&legacy_template_zip), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(instantiate_template_bytes(&legacy_template_zip, Some("Renamed")), Err(FormatError::UnsupportedFormat { .. })));

    assert!(matches!(import_prism_bytes(&legacy_project_zip), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(import_prism_bytes(&legacy_template_zip), Err(FormatError::UnsupportedFormat { .. })));
    assert!(import_pzfx_bytes(&legacy_project_zip).is_err());

    let dir = TempDir::new().expect("temp dir");
    for (suffix, bytes) in [
        ("renamed.curvebook", &legacy_project_zip),
        ("renamed.cbtemplate", &legacy_template_zip),
        ("renamed.prism", &legacy_project_zip),
        ("renamed_tpl.prism", &legacy_template_zip),
        ("renamed.pzfx", &legacy_project_zip),
        ("renamed.csv", &legacy_project_zip),
        ("renamed.tsv", &legacy_project_zip),
    ] {
        let path = dir.path().join(suffix);
        std::fs::write(&path, bytes).expect("write renamed legacy archive");
        let importer = reg.importer_for_path(&path).unwrap_or_else(|| panic!("expected importer dispatch for `{suffix}`"));
        assert!(
            importer.import_path(&path, &ImportOptions::default()).is_err(),
            "renamed legacy archive `{suffix}` must be rejected and never misidentified"
        );
    }
}

#[test]
fn explicit_rejection_of_legacy_unpacked_dirs_batch_manifests_and_owner_pids() {
    let reg = FormatRegistry::default_registry();
    let doc = sample_project().expect("sample project");
    let dir = TempDir::new().expect("temp dir");

    // 1. Unpacked legacy directory (`lumencraft-archive` manifest.json) is rejected both under
    //    `.lumencraft.d` and after renaming its suffix to `.curvebook.d`.
    let doc_bytes = serde_json::to_vec_pretty(&doc).expect("serialize doc");
    let legacy_manifest = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": doc.title,
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": []
    });
    let legacy_manifest_bytes = serde_json::to_vec_pretty(&legacy_manifest).expect("serialize legacy manifest");
    for dir_name in ["old.lumencraft.d", "renamed.curvebook.d"] {
        let unpacked = dir.path().join(dir_name);
        std::fs::create_dir_all(&unpacked).expect("create unpacked legacy dir");
        std::fs::write(unpacked.join("document.json"), &doc_bytes).expect("write legacy document.json");
        std::fs::write(unpacked.join("manifest.json"), &legacy_manifest_bytes).expect("write legacy manifest.json");
        assert!(!is_unpacked_project_dir(&unpacked));
        assert!(matches!(open_project_unpacked_dir(&unpacked), Err(FormatError::UnsupportedFormat { .. })));
    }
    assert!(reg.importer_for_path(&dir.path().join("old.lumencraft.d")).is_none());
    let renamed_dir = dir.path().join("renamed.curvebook.d");
    let dir_importer = reg.importer_for_path(&renamed_dir).expect("unpacked dir importer");
    assert!(matches!(dir_importer.import_path(&renamed_dir, &ImportOptions::default()), Err(FormatError::UnsupportedFormat { .. })));

    // 2. Legacy batch manifests (`lumencraft.batch.v1` / `v2`) are rejected both directly and when
    //    renamed to native/interchange extensions.
    let legacy_batch = serde_json::to_vec(&serde_json::json!({ "schema": "lumencraft.batch.v1", "commands": [] })).expect("serialize legacy batch");
    assert!(matches!(
        validate_batch_manifest_version(&serde_json::json!({ "schema": "lumencraft.batch.v1" })),
        Err(FormatError::UnsupportedFormat { .. })
    ));
    assert!(matches!(
        validate_batch_manifest_version(&serde_json::json!({ "schema": "lumencraft.batch.v2" })),
        Err(FormatError::UnsupportedFormat { .. })
    ));
    for suffix in ["batch.curvebook", "batch.cbtemplate", "batch.prism", "batch.pzfx"] {
        let path = dir.path().join(suffix);
        std::fs::write(&path, &legacy_batch).expect("write legacy batch");
        let importer = reg.importer_for_path(&path).expect("importer dispatch");
        assert!(importer.import_path(&path, &ImportOptions::default()).is_err());
    }

    // 3. Legacy PID owner prefix does not match CurveBook local PID reclamation.
    assert!(!is_dead_local_owner("lumencraft-pid-999999999"));
}

#[test]
fn journal_v1_crc_verified_before_v2_upgrade_and_batch_manifest_normalizes() {
    use curvebook_format::{
        JournalBackend, MemoryJournalBackend, compute_record_crc32, normalize_batch_manifest_v1, normalize_batch_manifest_v1_with_warnings,
    };

    // 1. Valid v1 record carrying retired aliases verifies raw v1 CRC first, then upgrades to v2.
    let v1_cmd = serde_json::json!({
        "command_id": "sheet.rename",
        "params": { "id": "data_1", "new_title": "Normalized Title" }
    });
    let v1_crc = compute_record_crc32(1, &v1_cmd).unwrap();
    let v1_line = serde_json::json!({
        "format_version": 1,
        "seq": 1,
        "crc32": v1_crc,
        "command_json": v1_cmd,
    })
    .to_string();

    let mut mem = MemoryJournalBackend::new();
    mem.append_journal_line(&v1_line).unwrap();
    let (mut journal, report) = CrashRecoveryJournal::open_or_recover(mem, "migrator", 10_000, 1_000).unwrap();
    assert_eq!(report.recovered_commands.len(), 1);
    assert_eq!(
        report.recovered_commands[0],
        serde_json::json!({
            "command_id": "sheet.rename",
            "params": { "sheet_id": "data_1", "title": "Normalized Title" }
        })
    );

    // Subsequent writes use CURRENT_JOURNAL_VERSION (2).
    journal.record_command(serde_json::json!({ "command_id": "sheet.freeze", "params": { "sheet_id": "data_1" } })).unwrap();
    let journal_text = std::str::from_utf8(journal.backend().journal_bytes()).unwrap();
    let parsed_tail = JournalOpRecord::from_json_str(journal_text.lines().last().unwrap()).unwrap();
    assert_eq!(parsed_tail.format_version, 2);

    // 2. Tampered v1 record with invalid raw CRC fails CorruptCrc even if command_json would upgrade.
    let mut bad_rec = JournalOpRecord { format_version: 1, seq: 1, crc32: v1_crc ^ 0xFFFF, command_json: v1_cmd.clone() };
    assert!(matches!(bad_rec.upgrade_to_current(), Err(FormatError::CorruptCrc { .. })));

    let mut tampered_mem = MemoryJournalBackend::new();
    tampered_mem
        .append_journal_line(
            &serde_json::json!({
                "format_version": 1,
                "seq": 1,
                "crc32": v1_crc ^ 1,
                "command_json": v1_cmd,
            })
            .to_string(),
        )
        .unwrap();
    tampered_mem.append_journal_line(&parsed_tail.to_ndjson_line().unwrap()).unwrap();
    assert!(matches!(CrashRecoveryJournal::open_or_recover(tampered_mem, "migrator-bad", 10_000, 1_000), Err(FormatError::CorruptCrc { .. })));

    // 3. Batch manifest v1 normalization upgrades inputs, commands, and exports while catching versions.
    let mut manifest = serde_json::json!({
        "schema": BATCH_SCHEMA_V1,
        "inputs": [{ "csv": "in.csv", "sheetId": "s1", "tableKind": "xy", "subcolumnMode": "replicate", "decimalSeparator": "." }],
        "commands": [
            { "command": "graph.create", "params": { "id": "g1", "source_sheet_id": "s1" } },
            { "method": "sheet.rename", "params": { "id": "s1", "new_title": "Dose" } }
        ],
        "exports": [{ "out": "out.pdf", "sheetId": "g1", "tabIndex": 0, "pdfVersion": "1.7", "ttfFont": "font.ttf" }]
    });
    let warnings = normalize_batch_manifest_v1_with_warnings(&mut manifest).unwrap();
    assert!(warnings.len() >= 8, "expected warnings for all deprecated batch keys, got {warnings:?}");
    assert_eq!(
        manifest,
        serde_json::json!({
            "schema": BATCH_SCHEMA_V1,
            "inputs": [{ "csv": "in.csv", "sheet": "s1", "kind": "xy", "subcolumn_mode": "replicate", "decimal_separator": "." }],
            "commands": [
                { "cmd": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "s1" } },
                { "cmd": "sheet.rename", "params": { "sheet_id": "s1", "title": "Dose" } }
            ],
            "exports": [{ "out": "out.pdf", "sheet": "g1", "tab_index": 0, "pdf_version": "1.7", "ttf_font": "font.ttf" }]
        })
    );
    assert!(normalize_batch_manifest_v1(&mut manifest).is_ok());
    assert!(normalize_batch_manifest_v1(&mut serde_json::json!({ "schema": "curvebook.batch.v2" })).is_err());

    // 4. `graph.addAnnotation` combining `"type": "shape"` and `"color"` upgrades `"type" -> "kind"`
    // before reading `"kind"`, so `"color"` also resolves to `"stroke_color"` in one pass.
    let mut ann_manifest = serde_json::json!({
        "schema": BATCH_SCHEMA_V1,
        "commands": [{
            "cmd": "graph.addAnnotation",
            "params": { "id": "g1", "type": "shape", "color": "#ff0000", "rect_pt": { "x": 10.0, "y": 10.0, "width": 50.0, "height": 20.0 } }
        }]
    });
    let ann_warnings = normalize_batch_manifest_v1_with_warnings(&mut ann_manifest).unwrap();
    assert_eq!(ann_warnings.len(), 4, "expected 4 warnings, got {ann_warnings:?}");
    assert_eq!(
        ann_manifest["commands"][0]["params"],
        serde_json::json!({
            "sheet_id": "g1",
            "kind": "shape",
            "stroke_color": "#ff0000",
            "bounds_pt": { "x": 10.0, "y": 10.0, "width": 50.0, "height": 20.0 }
        })
    );
}
