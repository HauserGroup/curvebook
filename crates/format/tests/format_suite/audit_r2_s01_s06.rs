use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use crate::common::{TempDirGuard, build_zip_entries, sample_audit_project};
use curvebook_doc::{
    CURRENT_SCHEMA_VERSION, Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, DataSheet, DocError, InfoConstant,
    InfoSheet, MAX_TITLE_BYTES, ProjectDocument, SheetId, SheetSection, TableArchetype,
};
use curvebook_format::ZipArchive;
use curvebook_format::recovery::{DirectoryJournalBackend, JournalBackend, JournalLockLease, LOCKFILE_NAME};
use curvebook_format::{
    ArchiveManifest, ExportOptions, ExportSource, FormatError, FormatKind, FormatRegistry, MAX_ZIP_ENTRY_BYTES, SheetManifestEntry, ZipLimits,
    batch_v1_schema, instantiate_template_bytes, journal_record_schema, open_project_file, open_project_unpacked_dir, open_template_bytes,
    preferences_schema, save_project_bytes, save_project_file, save_project_unpacked_dir, save_template_bytes, save_template_file,
    strip_project_data_for_template, validate_batch_manifest_version, validate_project_document,
};
use curvebook_palette::CanvasPreviewFilter;
use curvebook_platform::{PathResolver, PlatformError, ProcessCommand, WorkdirGuard, write_atomic};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[cfg(unix)]
#[test]
fn r2_s01_01_process_runner_handles_stalled_stdin_and_descendant_pipes() -> TestResult {
    let dir = TempDirGuard::new("proc")?;
    let pid_file = dir.child("child.pid");

    let start = Instant::now();
    let stdin_err = ProcessCommand::new("sh")
        .args(["-c", "(sleep 8) & exec 1>&- 2>&-"])
        .stdin_bytes(vec![b'a'; 2 * 1024 * 1024])
        .wall_timeout(Duration::from_millis(120))
        .idle_timeout(Duration::from_millis(120))
        .run()
        .unwrap_err();
    assert!(matches!(stdin_err, PlatformError::TimedOut { .. }) && start.elapsed() < Duration::from_secs(2));

    let pg_err = ProcessCommand::new("sh")
        .args(["-c", "sleep 10 & printf '%s' \"$!\" > \"$CB_PID_FILE\"; exit 0"])
        .env("CB_PID_FILE", pid_file.to_string_lossy().as_ref())
        .wall_timeout(Duration::from_millis(450))
        .idle_timeout(Duration::from_secs(5))
        .run()
        .unwrap_err();
    assert!(matches!(pg_err, PlatformError::TimedOut { .. }));

    let pid = fs::read_to_string(&pid_file)?.trim().to_owned();
    let deadline = Instant::now() + Duration::from_secs(1);
    while std::process::Command::new("kill")
        .args(["-0", &pid])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        assert!(Instant::now() < deadline, "descendant process {pid} survived process-group kill");
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[test]
fn r2_s01_02_path_guard_preserves_plate_template_and_guards_template_path() -> TestResult {
    let dir = TempDirGuard::new("pathguard")?;
    let guard = WorkdirGuard::new(dir.path())?;
    let canon = fs::canonicalize(dir.path())?;
    let inline_tpl = json!({ "name": "Plate ../96", "geometry": "96", "wells": [] });
    let inline_tpl_str = r#"{"format":"curvebook-archive","kind":"template","note":"../keep"}"#;

    for resolver in [PathResolver::Unconstrained { base_dir: dir.path().to_path_buf() }, PathResolver::Sandboxed(guard.clone())] {
        let mut params = json!({ "template": inline_tpl.clone(), "template_path": "tpl/plate.cbtemplate" });
        resolver.sanitize_params(&mut params)?;
        assert_eq!(params["template"], inline_tpl);
        let base = if matches!(resolver, PathResolver::Sandboxed(_)) { &canon } else { dir.path() };
        assert_eq!(params["template_path"], json!(base.join("tpl").join("plate.cbtemplate").to_string_lossy()));

        let mut str_params = json!({ "template": inline_tpl_str });
        resolver.sanitize_params(&mut str_params)?;
        assert_eq!(str_params["template"], json!(inline_tpl_str));
    }

    let mut bad = json!({ "template": inline_tpl, "templatePath": "../escape.cbtemplate" });
    assert!(matches!(PathResolver::Sandboxed(guard).sanitize_params(&mut bad), Err(PlatformError::PathEscape { .. })));
    Ok(())
}

#[cfg(unix)]
#[test]
fn r2_s01_03_atomic_writes_and_project_saves_preserve_mode_and_symlinks() -> TestResult {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let dir = TempDirGuard::new("perms")?;
    let (real_file, link_file) = (dir.child("real.curvebook"), dir.child("link.curvebook"));
    let mut doc = sample_audit_project();
    save_project_file(&real_file, &doc)?;
    fs::set_permissions(&real_file, fs::Permissions::from_mode(0o600))?;
    symlink(&real_file, &link_file)?;

    doc.title = String::from("Updated Via Symlink");
    save_project_file(&link_file, &doc)?;
    assert!(fs::symlink_metadata(&link_file)?.file_type().is_symlink());
    assert_eq!(
        (fs::metadata(&real_file)?.permissions().mode() & 0o777, open_project_file(&real_file)?.title.as_str()),
        (0o600, "Updated Via Symlink")
    );

    let cfg = dir.child("prefs.json");
    fs::write(&cfg, b"{}")?;
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o640))?;
    write_atomic(&cfg, b"{\"theme\":\"dark\"}")?;
    assert_eq!(fs::metadata(&cfg)?.permissions().mode() & 0o777, 0o640);
    Ok(())
}

#[test]
fn r2_s01_04_canvas_preview_filter_serde_matches_canonical_names_and_pascal_case() -> TestResult {
    assert_eq!(
        (serde_json::to_string(&CanvasPreviewFilter::GrayscalePrint)?, serde_json::to_string(&CanvasPreviewFilter::Normal)?),
        (String::from("\"grayscale\""), String::from("\"normal\""))
    );
    for (raw, expected) in [
        ("\"grayscale\"", CanvasPreviewFilter::GrayscalePrint),
        ("\"GrayscalePrint\"", CanvasPreviewFilter::GrayscalePrint),
        ("\"deuteranopia\"", CanvasPreviewFilter::Deuteranopia),
        ("\"Deuteranopia\"", CanvasPreviewFilter::Deuteranopia),
        ("\"protanopia\"", CanvasPreviewFilter::Protanopia),
        ("\"tritanopia\"", CanvasPreviewFilter::Tritanopia),
        ("\"Normal\"", CanvasPreviewFilter::Normal),
    ] {
        assert_eq!(serde_json::from_str::<CanvasPreviewFilter>(raw)?, expected);
    }
    assert!(serde_json::from_str::<CanvasPreviewFilter>("\"invalid-mode\"").is_err());
    Ok(())
}

#[test]
fn r2_s05_02_templates_strip_checkpoints_and_notes_and_archive_validates_both() -> TestResult {
    let doc = sample_audit_project();
    let stripped = strip_project_data_for_template(&doc)?;
    assert!(stripped.checkpoints.is_empty() && stripped.data_sheets[0].exclusion_notes.is_empty());

    let tpl_bytes = save_template_bytes(&doc)?;
    let tpl_zip = ZipArchive::from_bytes(&tpl_bytes)?;
    let tpl_doc_json = tpl_zip.get_utf8("document.json")?.ok_or("missing document.json")?;
    assert!(!tpl_doc_json.contains("Private cohort snapshot") && !tpl_doc_json.contains("Patient sample #4 hemolyzed"));
    assert!(open_template_bytes(&tpl_bytes)?.checkpoints.is_empty() && instantiate_template_bytes(&tpl_bytes, Some("Fresh"))?.checkpoints.is_empty());

    let mut nan_snap = doc.clone();
    nan_snap.checkpoints.clear();
    if let TableArchetype::Column(ref mut c) = nan_snap.data_sheets[0].archetype {
        c.data_sets[0].cells[0][0] = Cell::number(1.0);
    }
    let nan_json = serde_json::to_string(&nan_snap)?.replace("1.0", "1e99999");
    let nested_json = serde_json::to_string(&doc)?;

    type MutFn = Box<dyn Fn(&mut ProjectDocument)>;
    type CheckFn = fn(&Result<(), FormatError>) -> bool;
    let invalid_cases: Vec<(&str, MutFn, CheckFn)> = vec![
        ("empty_cp_id", Box::new(|d| d.checkpoints[0].id = String::from("   ")), |r| matches!(r, Err(FormatError::Doc(DocError::EmptyIdentifier)))),
        (
            "dup_cp_id",
            Box::new(|d| {
                let c = d.checkpoints[0].clone();
                d.checkpoints.push(c);
            }),
            Result::is_err,
        ),
        ("long_cp_name", Box::new(|d| d.checkpoints[0].name = "x".repeat(MAX_TITLE_BYTES + 1)), |r| {
            matches!(r, Err(FormatError::Doc(DocError::TitleTooLong { .. })))
        }),
        ("nested_cp", Box::new(move |d| d.checkpoints[0].snapshot_json = nested_json.clone()), Result::is_err),
        ("corrupt_snap_num", Box::new(move |d| d.checkpoints[0].snapshot_json = nan_json.clone()), Result::is_err),
        (
            "oob_note",
            Box::new(|d| {
                d.data_sheets[0].exclusion_notes.push(CellExclusionNote {
                    coord: CellCoord::new(99, 0, 0),
                    reason_tag: String::from("outlier"),
                    note: String::from("oob"),
                    timestamp_utc: String::from("2026-11-15T10:00:00Z"),
                })
            }),
            |r| matches!(r, Err(FormatError::Doc(DocError::CellOutOfBounds { .. }))),
        ),
        ("unexcluded_note", Box::new(|d| d.data_sheets[0].exclusion_notes[0].coord = CellCoord::new(0, 0, 0)), Result::is_err),
        ("long_note", Box::new(|d| d.data_sheets[0].exclusion_notes[0].note = "n".repeat(MAX_TITLE_BYTES + 1)), |r| {
            matches!(r, Err(FormatError::Doc(DocError::TitleTooLong { .. })))
        }),
    ];
    for (label, mutate, check) in invalid_cases {
        let mut bad = doc.clone();
        mutate(&mut bad);
        assert!(check(&validate_project_document(&bad)), "{label}");
    }
    Ok(())
}

#[test]
fn r2_s05_04_preferences_and_journal_schemas_match_migrator_and_serde_contracts() -> TestResult {
    let prefs = preferences_schema();
    assert!(prefs.get("required").is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)));
    assert_eq!(
        (&prefs["properties"]["version"]["minimum"], &prefs["properties"]["version"]["maximum"], &prefs["properties"]["shortcuts"]["type"]),
        (&json!(0), &json!(1), &json!(["object", "null"]))
    );

    let journal = journal_record_schema();
    for (def_name, max_fields) in [("JournalOpRecord", &["seq"][..]), ("JournalLockLease", &["acquired_epoch_ms", "ttl_ms"][..])] {
        let def = &journal["$defs"][def_name];
        let req: BTreeSet<&str> = def["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(!req.contains("format_version"));
        for f in max_fields {
            assert_eq!(def["properties"][f]["maximum"], Value::from(u64::MAX), "{def_name}.{f}");
        }
    }
    Ok(())
}

#[test]
fn r2_s06_01_unpacked_dir_preserves_untracked_files_and_rejects_untrusted_csv_paths() -> TestResult {
    let dir = TempDirGuard::new("prune")?;
    let proj_dir = dir.child("study.curvebook.d");
    let mut doc = sample_audit_project();
    save_project_unpacked_dir(&doc, &proj_dir)?;

    fs::create_dir_all(proj_dir.join(".git"))?;
    fs::write(proj_dir.join(".git/HEAD"), b"ref: refs/heads/main\n")?;
    fs::write(proj_dir.join("README.md"), b"keep readme\n")?;
    fs::write(proj_dir.join("sheets/notes.txt"), b"keep researcher notes\n")?;

    let mut manifest: ArchiveManifest = serde_json::from_slice(&fs::read(proj_dir.join("manifest.json"))?)?;
    for (id, path) in [("poison-git", ".git/HEAD"), ("poison-readme", "README.md")] {
        manifest.sheets.push(SheetManifestEntry {
            id: SheetId::new(id),
            section: SheetSection::Data,
            title: String::from(id),
            csv_path: Some(String::from(path)),
            crc32: Some(0),
        });
    }
    fs::write(proj_dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::UnsafePath { .. })));

    doc.result_sheets.clear();
    doc.data_sheets.clear();
    doc.checkpoints.clear();
    save_project_unpacked_dir(&doc, &proj_dir)?;
    assert_eq!(
        (
            fs::read(proj_dir.join(".git/HEAD"))?.as_slice(),
            fs::read(proj_dir.join("README.md"))?.as_slice(),
            fs::read(proj_dir.join("sheets/notes.txt"))?.as_slice()
        ),
        (b"ref: refs/heads/main\n".as_slice(), b"keep readme\n".as_slice(), b"keep researcher notes\n".as_slice())
    );
    assert!(!proj_dir.join("sheets/sheet-a.csv").exists());
    Ok(())
}

#[test]
fn r2_s06_02_registry_distinguishes_unpacked_project_dirs_from_arbitrary_dirs() -> TestResult {
    let dir = TempDirGuard::new("registry-dir")?;
    let reg = FormatRegistry::default_registry();
    for name in ["workspace", "not_a_project.curvebook", "export.json"] {
        let p = dir.child(name);
        fs::create_dir_all(&p)?;
        assert!(reg.importer_for_path(&p).is_none() && reg.exporter_for_path(&p).is_none(), "{name}");
    }
    let empty_d = dir.child("fresh.curvebook.d");
    fs::create_dir_all(&empty_d)?;
    assert_eq!(reg.exporter_for_path(&empty_d).unwrap().descriptor().kind, FormatKind::ProjectDir);

    let valid_dot_dir = dir.child("study.curvebook");
    save_project_unpacked_dir(&sample_audit_project(), &valid_dot_dir)?;
    assert_eq!(
        (reg.importer_for_path(&valid_dot_dir).unwrap().descriptor().kind, reg.exporter_for_path(&valid_dot_dir).unwrap().descriptor().kind),
        (FormatKind::ProjectDir, FormatKind::ProjectDir)
    );
    let imported = reg.importer_for_path(&valid_dot_dir).unwrap().import_path(&valid_dot_dir, &Default::default())?.into_document();
    assert_eq!(imported.as_ref().map(|d| d.title.as_str()), Some("R2 Audit Study"));
    Ok(())
}

#[test]
fn r2_s06_03_save_paths_reject_newer_schema_versions_on_doc_and_disk() -> TestResult {
    let dir = TempDirGuard::new("schema-guard")?;
    let mut doc = sample_audit_project();
    let reg = FormatRegistry::default_registry();

    let mut future_doc = doc.clone();
    future_doc.schema_version = CURRENT_SCHEMA_VERSION + 1;
    assert!(matches!(save_project_bytes(&future_doc), Err(FormatError::UnsupportedSchemaVersion { .. })));
    assert!(matches!(save_template_bytes(&future_doc), Err(FormatError::UnsupportedSchemaVersion { .. })));
    assert!(matches!(save_project_file(&dir.child("fut.curvebook"), &future_doc), Err(FormatError::UnsupportedSchemaVersion { .. })));
    assert!(matches!(save_project_unpacked_dir(&future_doc, &dir.child("fut.curvebook.d")), Err(FormatError::UnsupportedSchemaVersion { .. })));

    let future_ver = CURRENT_SCHEMA_VERSION + 2;
    let future_manifest = json!({ "format": "curvebook-archive", "schema_version": future_ver, "kind": "Project", "title": "Future", "document_crc32": 0, "sheets": [], "future_required_field": true });
    let future_doc_bytes = format!(r#"{{"schema_version":{future_ver},"title":"Future"}}"#).into_bytes();
    let future_zip = build_zip_entries(&[("manifest.json", &serde_json::to_vec(&future_manifest)?), ("document.json", &future_doc_bytes)]);

    let (proj_path, tpl_path) = (dir.child("vnext.curvebook"), dir.child("vnext.cbtemplate"));
    fs::write(&proj_path, &future_zip)?;
    fs::write(&tpl_path, &future_zip)?;
    doc.schema_version = CURRENT_SCHEMA_VERSION;
    assert!(matches!(save_project_file(&proj_path, &doc), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver));
    assert!(matches!(save_template_file(&tpl_path, &doc), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver));
    for (fmt, path) in [("curvebook", &proj_path), ("cbtemplate", &tpl_path)] {
        assert!(matches!(
            reg.exporter_by_format(fmt).unwrap().export_path(ExportSource::Document(&doc), path, &ExportOptions::default()),
            Err(FormatError::UnsupportedSchemaVersion { .. })
        ));
        assert_eq!(fs::read(path)?, future_zip, "{fmt}");
    }
    for (dname, fname, payload) in
        [("vnext.curvebook.d", "manifest.json", serde_json::to_vec(&future_manifest)?), ("docvnext.curvebook.d", "document.json", future_doc_bytes)]
    {
        let p = dir.child(dname);
        fs::create_dir_all(&p)?;
        fs::write(p.join(fname), payload)?;
        assert!(
            matches!(save_project_unpacked_dir(&doc, &p), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver),
            "{dname}"
        );
    }
    Ok(())
}

#[test]
fn r2_s06_04_unpacked_save_rejects_dir_conflicts_and_preserves_crc_consistency() -> TestResult {
    let mut dir = TempDirGuard::new("unpacked-tx")?;
    let proj_dir = dir.child("tx.curvebook.d");
    let doc_v1 = sample_audit_project();
    save_project_unpacked_dir(&doc_v1, &proj_dir)?;

    let mut doc_v2 = doc_v1.clone();
    doc_v2.title = String::from("V2 Should Abort Cleanly");
    let mut col2 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?;
    col2.data_sets[0].cells[0] = vec![Cell::number(42.0)];
    doc_v2.add_data_sheet(DataSheet::new("sheet-b", "Cohort B", TableArchetype::Column(col2))?)?;
    fs::create_dir_all(proj_dir.join("sheets/sheet-b.csv"))?;
    assert!(save_project_unpacked_dir(&doc_v2, &proj_dir).is_err() && open_project_unpacked_dir(&proj_dir)? == doc_v1);
    fs::remove_dir_all(proj_dir.join("sheets/sheet-b.csv"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let sheets_dir = proj_dir.join("sheets");
        dir.track_chmod_restore(sheets_dir.clone());
        fs::set_permissions(&sheets_dir, fs::Permissions::from_mode(0o555))?;
        let res = save_project_unpacked_dir(&doc_v2, &proj_dir);
        let _ = fs::set_permissions(&sheets_dir, fs::Permissions::from_mode(0o755));
        if res.is_err() {
            assert_eq!(open_project_unpacked_dir(&proj_dir)?, doc_v1);
        }
    }
    Ok(())
}

#[test]
fn r2_s06_05_unpacked_save_enforces_zip_limits_matching_reopen() -> TestResult {
    let dir = TempDirGuard::new("ziplimits")?;
    let proj_dir = dir.child("limits.curvebook.d");
    let base_doc = sample_audit_project();
    save_project_unpacked_dir(&base_doc, &proj_dir)?;

    let mut big_doc = ProjectDocument::new("Oversized")?;
    let huge_name = "x".repeat(MAX_TITLE_BYTES);
    let mut info = InfoSheet::new_preseeded("info-big", "Big Constants")?;
    info.constants.extend((0..17_000).map(|i| InfoConstant {
        name: huge_name.clone(),
        value: curvebook_doc::ConstantValue::Number(i as f64),
        units: None,
    }));
    big_doc.add_info_sheet(info)?;

    assert!(matches!(save_project_unpacked_dir(&big_doc, &proj_dir), Err(FormatError::ZipBomb { .. })));
    assert!(serde_json::to_vec_pretty(&big_doc)?.len() as u64 > MAX_ZIP_ENTRY_BYTES);
    assert_eq!((open_project_unpacked_dir(&proj_dir)?, ZipLimits::default().max_entry_uncompressed_bytes), (base_doc, MAX_ZIP_ENTRY_BYTES));
    Ok(())
}

#[test]
fn r2_s06_06_unique_sheet_csv_path_handles_case_collisions_long_ids_and_reserved_names() -> TestResult {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
        "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let dir = TempDirGuard::new("csv-names")?;
    let proj_dir = dir.child("portable.curvebook.d");
    let mut doc = ProjectDocument::new("Portable Names")?;

    let long_id = "L".repeat(300);
    for (idx, raw_id) in ["WT", "wt", "Wt", "CON", "con", "prn", "Aux", "nul", "COM1", "lpt9", long_id.as_str()].iter().enumerate() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?;
        col.data_sets[0].cells[0] = vec![Cell::number((idx + 1) as f64)];
        doc.add_data_sheet(DataSheet::new(*raw_id, format!("Sheet {idx}"), TableArchetype::Column(col))?)?;
    }

    save_project_unpacked_dir(&doc, &proj_dir)?;
    let manifest: ArchiveManifest = serde_json::from_slice(&fs::read(proj_dir.join("manifest.json"))?)?;
    let mut lower_paths = BTreeSet::new();
    for csv_path in manifest.sheets.iter().filter_map(|e| e.csv_path.as_deref()) {
        assert!(lower_paths.insert(csv_path.to_ascii_lowercase()), "case collision in {csv_path}");
        let fname = Path::new(csv_path).file_name().and_then(|s| s.to_str()).unwrap();
        let stem = fname.strip_suffix(".csv").unwrap();
        assert!(fname.len() <= 96 && !RESERVED.contains(&stem.to_ascii_uppercase().as_str()), "bad name: {fname}");
    }
    assert_eq!(open_project_unpacked_dir(&proj_dir)?, doc);
    Ok(())
}

#[test]
fn r2_s06_07_journal_lock_waits_for_fresh_empty_in_flight_lockfile() -> TestResult {
    let dir = TempDirGuard::new("lock-race")?;
    let lease_a_bytes = serde_json::to_vec(&JournalLockLease::new("owner-a", 10_000, 60_000)?)?;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(dir.child(LOCKFILE_NAME))?;
    let barrier = Arc::new(Barrier::new(2));
    let gate = Arc::clone(&barrier);
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        use std::io::Write as _;
        gate.wait();
        std::thread::sleep(Duration::from_millis(25));
        file.write_all(&lease_a_bytes)?;
        file.sync_all()
    });
    barrier.wait();
    let mut backend_b = DirectoryJournalBackend::open_dir(dir.path())?;
    let err = backend_b.acquire_lease(&JournalLockLease::new("owner-b", 10_000, 60_000)?, 10_030).unwrap_err();
    writer.join().map_err(|_| "writer panicked")??;
    assert!(err.to_string().contains("actively held by `owner-a`"));
    assert_eq!(backend_b.read_lease()?.map(|l| l.owner_id), Some(String::from("owner-a")));
    Ok(())
}

#[test]
fn r2_s06_08_json_adapter_validates_floats_exports_result_sheet_and_returns_sheet_not_found() -> TestResult {
    let reg = FormatRegistry::default_registry();
    let (json_exp, csv_exp, tsv_exp) =
        (reg.exporter_by_format("json").unwrap(), reg.exporter_by_format("csv").unwrap(), reg.exporter_by_format("tsv").unwrap());
    let mut doc = sample_audit_project();

    let default_json: Value = serde_json::from_slice(&json_exp.export_bytes(ExportSource::Document(&doc), &ExportOptions::default())?)?;
    assert!(default_json["id"] == "res-1" && default_json.get("data_sheets").is_none());

    let mut no_res = doc.clone();
    no_res.result_sheets.clear();
    assert!(matches!(json_exp.export_bytes(ExportSource::Document(&no_res), &ExportOptions::default()), Err(FormatError::UnsupportedFormat { .. })));

    let missing_opts = ExportOptions { sheet_id: Some(String::from("does-not-exist")), ..Default::default() };
    for exp in [json_exp, csv_exp, tsv_exp] {
        assert!(matches!(exp.export_bytes(ExportSource::Document(&doc), &missing_opts), Err(FormatError::Doc(DocError::SheetNotFound { .. }))));
    }

    if let TableArchetype::Column(ref mut c) = doc.data_sheets[0].archetype {
        c.data_sets[0].cells[0][0] = Cell { value: CellValue::Number(f64::NAN), excluded: false };
    }
    assert!(matches!(
        json_exp.export_bytes(ExportSource::DataSheet(&doc.data_sheets[0]), &ExportOptions::default()),
        Err(FormatError::NonFiniteFloat { .. })
    ));
    assert!(matches!(
        json_exp.export_bytes(ExportSource::Document(&doc), &ExportOptions { sheet_id: Some(String::from("sheet-a")), ..Default::default() }),
        Err(FormatError::NonFiniteFloat { .. })
    ));
    Ok(())
}

#[test]
fn r2_s06_09_batch_schema_and_version_validator_match_cli_contract() -> TestResult {
    let schema = batch_v1_schema();
    assert_eq!(
        (&schema["not"]["properties"]["template"]["type"], &schema["not"]["properties"]["project"]["type"]),
        (&json!("string"), &json!("string"))
    );

    let input_props = schema["$defs"]["BatchInput"]["properties"].as_object().unwrap();
    assert!(["sheet_id", "sheetId", "table_kind", "tableKind", "subcolumnMode", "decimalSeparator"].iter().all(|k| input_props.contains_key(*k)));
    assert!(input_props["delimiter"].get("maxLength").is_none() && schema["$defs"]["BatchInput"]["anyOf"].as_array().map(Vec::len) == Some(4));

    let cmd_def = &schema["$defs"]["BatchCommand"];
    assert_eq!((&cmd_def["properties"]["params"]["type"], cmd_def["anyOf"].as_array().map(Vec::len)), (&json!(["object", "null"]), Some(3)));

    let exp_props = schema["$defs"]["BatchExport"]["properties"].as_object().unwrap();
    assert!(["sheet_id", "sheetId", "tabIndex", "pdfVersion", "ttfFont"].iter().all(|k| exp_props.contains_key(*k)));
    let formats: BTreeSet<&str> = exp_props["format"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(["tif", "curvebook-dir", "unpacked", "dir", "emf"].iter().all(|f| formats.contains(f)) && !formats.contains("cbtemplate"));
    assert_eq!(
        (&exp_props["dpi"]["type"], &exp_props["dpi"]["minimum"], &exp_props["dpi"]["maximum"]),
        (&json!(["integer", "null"]), &json!(1), &json!(65_535))
    );

    assert!(validate_batch_manifest_version(&json!({ "schema": "curvebook.batch.v1" })).is_ok());
    for (s, ver) in [("curvebook.batch.v2", 2), ("curvebook.batch.v0", 0)] {
        assert_eq!(
            validate_batch_manifest_version(&json!({ "schema": s })).unwrap_err(),
            FormatError::UnsupportedSchemaVersion { version: ver, max_supported: 1 }
        );
    }
    for bad in ["curvebook.batch.v01", "curvebook.batch.v+1", "curvebook.batch.v-1", "lumencraft.batch.v1", "lumencraft.batch.v2"] {
        assert!(matches!(validate_batch_manifest_version(&json!({ "schema": bad })), Err(FormatError::UnsupportedFormat { .. })), "{bad}");
    }
    Ok(())
}
