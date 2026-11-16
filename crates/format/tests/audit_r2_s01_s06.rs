use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CURRENT_SCHEMA_VERSION, Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, DataSheet,
    DescriptiveSpec, DocError, InfoConstant, InfoSheet, MAX_TITLE_BYTES, NamedCheckpoint, ProjectDocument, ResultScalarEntry, ResultSheet,
    ResultStatus, SheetId, SheetSection, TableArchetype,
};
use curvebook_format::ZipArchive;
use curvebook_format::recovery::{DirectoryJournalBackend, JournalBackend, JournalLockLease, LOCKFILE_NAME};
use curvebook_format::{
    ArchiveManifest, ExportOptions, ExportSource, FormatError, FormatKind, FormatRegistry, MAX_ZIP_ENTRY_BYTES, SheetManifestEntry, ZipLimits,
    ZipWriter, batch_v1_schema, instantiate_template_bytes, journal_record_schema, open_project_file, open_project_unpacked_dir, open_template_bytes,
    preferences_schema, save_project_bytes, save_project_file, save_project_unpacked_dir, save_template_bytes, save_template_file,
    strip_project_data_for_template, validate_batch_manifest_version, validate_project_document,
};
use curvebook_palette::CanvasPreviewFilter;
use curvebook_platform::{PathResolver, PlatformError, ProcessCommand, WorkdirGuard, write_atomic};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
    #[cfg(unix)]
    restore_writable: Vec<PathBuf>,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, std::io::Error> {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb_audit_r2_s01s06_{label}_{}_{seq}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            #[cfg(unix)]
            restore_writable: Vec::new(),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(unix)]
    fn track_chmod_restore(&mut self, dir: PathBuf) {
        self.restore_writable.push(dir);
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for dir in &self.restore_writable {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn build_sample_project() -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("R2 Audit Study")?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2)?;
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Treated");
    col.data_sets[0].cells[0] = vec![Cell::number(1.25)];
    col.data_sets[0].cells[1] = vec![Cell::number(2.5)];
    col.data_sets[1].cells[0] = vec![Cell::number(9.75)];
    col.data_sets[1].cells[1] = vec![Cell::number(10.5)];
    let mut ds = DataSheet::new("sheet-a", "Cohort A", TableArchetype::Column(col))?;
    ds.set_excluded_with_reason(CellCoord::new(1, 1, 0), "hemolyzed", "Patient sample #4 hemolyzed", "2026-11-15T10:00:00Z")?;
    doc.add_data_sheet(ds)?;

    let spec = AnalysisSpec::new("res-1", "sheet-a", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?;
    let mut rs = ResultSheet::new(spec)?;
    rs.status = ResultStatus::UpToDate;
    rs.scalars.push(ResultScalarEntry { key: String::from("Mean"), data_set_index: Some(0), value: 1.875, units: Some(String::from("nM")) });
    doc.add_result_sheet(rs)?;

    let mut snap = doc.clone();
    snap.checkpoints.clear();
    let snapshot_json = serde_json::to_string(&snap)?;
    doc.checkpoints.push(NamedCheckpoint {
        id: String::from("cp-1"),
        name: String::from("Pre-QC"),
        note: String::from("Private cohort snapshot"),
        created_utc: String::from("2026-11-15T10:05:00Z"),
        snapshot_json,
    });
    Ok(doc)
}

#[cfg(unix)]
#[test]
fn r2_s01_01_process_runner_handles_stalled_stdin_and_descendant_pipes() -> TestResult {
    let dir = TempDirGuard::new("proc")?;
    let pid_file = dir.path().join("child.pid");
    let huge_stdin = vec![b'a'; 2 * 1024 * 1024];

    let start = Instant::now();
    let stdin_err = ProcessCommand::new("sh")
        .args(["-c", "(sleep 8) & exec 1>&- 2>&-"])
        .stdin_bytes(huge_stdin)
        .wall_timeout(Duration::from_millis(120))
        .idle_timeout(Duration::from_millis(120))
        .run()
        .err()
        .ok_or("expected stalled stdin process to time out")?;
    assert!(matches!(stdin_err, PlatformError::TimedOut { .. }));
    assert!(start.elapsed() < Duration::from_secs(2));

    let pg_err = ProcessCommand::new("sh")
        .args(["-c", "sleep 10 & printf '%s' \"$!\" > \"$CB_PID_FILE\"; exit 0"])
        .env("CB_PID_FILE", pid_file.to_string_lossy().as_ref())
        .wall_timeout(Duration::from_millis(450))
        .idle_timeout(Duration::from_secs(5))
        .run()
        .err()
        .ok_or("expected leaked stdout descendant to time out")?;
    assert!(matches!(pg_err, PlatformError::TimedOut { .. }));

    let pid = std::fs::read_to_string(&pid_file)?.trim().to_owned();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !alive {
            break;
        }
        assert!(Instant::now() < deadline, "descendant process {pid} survived process-group kill");
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[test]
fn r2_s01_02_path_guard_preserves_plate_template_and_guards_template_path() -> TestResult {
    let dir = TempDirGuard::new("pathguard")?;
    let guard = WorkdirGuard::new(dir.path())?;
    let canon = std::fs::canonicalize(dir.path())?;

    let inline_tpl = json!({ "name": "Plate ../96", "geometry": "96", "wells": [] });
    let inline_tpl_str = r#"{"format":"curvebook-archive","kind":"template","note":"../keep"}"#;

    for resolver in [PathResolver::Unconstrained { base_dir: dir.path().to_path_buf() }, PathResolver::Sandboxed(guard.clone())] {
        let mut params = json!({
            "template": inline_tpl.clone(),
            "template_path": "tpl/plate.cbtemplate",
        });
        resolver.sanitize_params(&mut params)?;
        assert_eq!(params["template"], inline_tpl);
        let expected = match &resolver {
            PathResolver::Unconstrained { .. } => dir.path().join("tpl").join("plate.cbtemplate"),
            PathResolver::Sandboxed(_) => canon.join("tpl").join("plate.cbtemplate"),
        };
        assert_eq!(params["template_path"], json!(expected.to_string_lossy()));

        let mut str_params = json!({ "template": inline_tpl_str });
        resolver.sanitize_params(&mut str_params)?;
        assert_eq!(str_params["template"], json!(inline_tpl_str));
    }

    let sandbox = PathResolver::Sandboxed(guard);
    let mut bad = json!({ "template": inline_tpl, "templatePath": "../escape.cbtemplate" });
    assert!(matches!(sandbox.sanitize_params(&mut bad), Err(PlatformError::PathEscape { .. })));
    Ok(())
}

#[cfg(unix)]
#[test]
fn r2_s01_03_atomic_writes_and_project_saves_preserve_mode_and_symlinks() -> TestResult {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let dir = TempDirGuard::new("perms")?;
    let real_file = dir.path().join("real.curvebook");
    let link_file = dir.path().join("link.curvebook");

    let doc_v1 = build_sample_project()?;
    save_project_file(&real_file, &doc_v1)?;
    std::fs::set_permissions(&real_file, std::fs::Permissions::from_mode(0o600))?;
    symlink(&real_file, &link_file)?;

    let mut doc_v2 = doc_v1.clone();
    doc_v2.title = String::from("Updated Via Symlink");
    save_project_file(&link_file, &doc_v2)?;

    assert!(std::fs::symlink_metadata(&link_file)?.file_type().is_symlink());
    assert_eq!(std::fs::metadata(&real_file)?.permissions().mode() & 0o777, 0o600);
    assert_eq!(open_project_file(&real_file)?.title, "Updated Via Symlink");

    let cfg = dir.path().join("prefs.json");
    std::fs::write(&cfg, b"{}")?;
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o640))?;
    write_atomic(&cfg, b"{\"theme\":\"dark\"}")?;
    assert_eq!(std::fs::metadata(&cfg)?.permissions().mode() & 0o777, 0o640);
    Ok(())
}

#[test]
fn r2_s01_04_canvas_preview_filter_serde_matches_canonical_names_and_pascal_case() -> TestResult {
    assert_eq!(serde_json::to_string(&CanvasPreviewFilter::GrayscalePrint)?, "\"grayscale\"");
    assert_eq!(serde_json::to_string(&CanvasPreviewFilter::Normal)?, "\"normal\"");

    for (raw, expected) in [
        ("\"grayscale\"", CanvasPreviewFilter::GrayscalePrint),
        ("\"GrayscalePrint\"", CanvasPreviewFilter::GrayscalePrint),
        ("\"deuteranopia\"", CanvasPreviewFilter::Deuteranopia),
        ("\"Deuteranopia\"", CanvasPreviewFilter::Deuteranopia),
        ("\"protanopia\"", CanvasPreviewFilter::Protanopia),
        ("\"tritanopia\"", CanvasPreviewFilter::Tritanopia),
        ("\"Normal\"", CanvasPreviewFilter::Normal),
    ] {
        let parsed: CanvasPreviewFilter = serde_json::from_str(raw)?;
        assert_eq!(parsed, expected);
    }
    assert!(serde_json::from_str::<CanvasPreviewFilter>("\"invalid-mode\"").is_err());
    Ok(())
}

#[test]
fn r2_s05_02_templates_strip_checkpoints_and_notes_and_archive_validates_both() -> TestResult {
    let doc = build_sample_project()?;
    let stripped = strip_project_data_for_template(&doc)?;
    assert!(stripped.checkpoints.is_empty());
    assert!(stripped.data_sheets[0].exclusion_notes.is_empty());

    let tpl_bytes = save_template_bytes(&doc)?;
    let tpl_zip = ZipArchive::from_bytes(&tpl_bytes)?;
    let tpl_doc_json = tpl_zip.get_utf8("document.json")?.ok_or("missing document.json")?;
    assert!(!tpl_doc_json.contains("Private cohort snapshot"), "raw checkpoint data leaked into .cbtemplate");
    assert!(!tpl_doc_json.contains("Patient sample #4 hemolyzed"), "exclusion note leaked into .cbtemplate");
    assert!(open_template_bytes(&tpl_bytes)?.checkpoints.is_empty());
    assert!(instantiate_template_bytes(&tpl_bytes, Some("Fresh"))?.checkpoints.is_empty());

    // 1. Empty checkpoint ID
    let mut bad_empty_id = doc.clone();
    bad_empty_id.checkpoints[0].id = String::from("   ");
    assert!(matches!(validate_project_document(&bad_empty_id), Err(FormatError::Doc(DocError::EmptyIdentifier))));

    // 2. Duplicate checkpoint ID
    let mut bad_dup_cp = doc.clone();
    bad_dup_cp.checkpoints.push(bad_dup_cp.checkpoints[0].clone());
    assert!(validate_project_document(&bad_dup_cp).is_err());

    // 3. Oversized checkpoint title/note
    let mut bad_long_cp = doc.clone();
    bad_long_cp.checkpoints[0].name = "x".repeat(MAX_TITLE_BYTES + 1);
    assert!(matches!(validate_project_document(&bad_long_cp), Err(FormatError::Doc(DocError::TitleTooLong { .. }))));

    // 4. Nested checkpoint inside snapshot_json
    let mut bad_nested = doc.clone();
    bad_nested.checkpoints[0].snapshot_json = serde_json::to_string(&doc)?;
    assert!(validate_project_document(&bad_nested).is_err());

    // 5. Corrupt/NaN payload inside checkpoint snapshot_json
    let mut nan_snap = doc.clone();
    nan_snap.checkpoints.clear();
    if let TableArchetype::Column(ref mut c) = nan_snap.data_sheets[0].archetype {
        c.data_sets[0].cells[0][0] = Cell::number(1.0);
    }
    let nan_json = serde_json::to_string(&nan_snap)?.replace("1.0", "1e99999");
    let mut bad_snap_num = doc.clone();
    bad_snap_num.checkpoints[0].snapshot_json = nan_json;
    assert!(validate_project_document(&bad_snap_num).is_err());

    // 6. Exclusion note out-of-bounds
    let mut bad_oob_note = doc.clone();
    bad_oob_note.data_sheets[0].exclusion_notes.push(CellExclusionNote {
        coord: CellCoord::new(99, 0, 0),
        reason_tag: String::from("outlier"),
        note: String::from("oob"),
        timestamp_utc: String::from("2026-11-15T10:00:00Z"),
    });
    assert!(matches!(validate_project_document(&bad_oob_note), Err(FormatError::Doc(DocError::CellOutOfBounds { .. }))));

    // 7. Exclusion note pointing to unexcluded cell
    let mut bad_unexcluded = doc.clone();
    bad_unexcluded.data_sheets[0].exclusion_notes[0].coord = CellCoord::new(0, 0, 0);
    assert!(validate_project_document(&bad_unexcluded).is_err());

    // 8. Oversized exclusion note text
    let mut bad_long_note = doc;
    bad_long_note.data_sheets[0].exclusion_notes[0].note = "n".repeat(MAX_TITLE_BYTES + 1);
    assert!(matches!(validate_project_document(&bad_long_note), Err(FormatError::Doc(DocError::TitleTooLong { .. }))));
    Ok(())
}

#[test]
fn r2_s05_04_preferences_and_journal_schemas_match_migrator_and_serde_contracts() -> TestResult {
    let prefs = preferences_schema();
    assert!(prefs.get("required").is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)), "preferences schema must allow unversioned v0 objects");
    assert_eq!(prefs["properties"]["version"]["minimum"], 0);
    assert_eq!(prefs["properties"]["version"]["maximum"], 1);
    assert_eq!(prefs["properties"]["shortcuts"]["type"], json!(["object", "null"]));

    let journal = journal_record_schema();
    let op_def = &journal["$defs"]["JournalOpRecord"];
    let op_req: BTreeSet<&str> = op_def["required"].as_array().ok_or("op required")?.iter().filter_map(Value::as_str).collect();
    assert!(!op_req.contains("format_version"));
    assert_eq!(op_def["properties"]["seq"]["maximum"], Value::from(u64::MAX));

    let lock_def = &journal["$defs"]["JournalLockLease"];
    let lock_req: BTreeSet<&str> = lock_def["required"].as_array().ok_or("lock required")?.iter().filter_map(Value::as_str).collect();
    assert!(!lock_req.contains("format_version"));
    assert_eq!(lock_def["properties"]["acquired_epoch_ms"]["maximum"], Value::from(u64::MAX));
    assert_eq!(lock_def["properties"]["ttl_ms"]["maximum"], Value::from(u64::MAX));
    Ok(())
}

#[test]
fn r2_s06_01_unpacked_dir_preserves_untracked_files_and_rejects_untrusted_csv_paths() -> TestResult {
    let dir = TempDirGuard::new("prune")?;
    let proj_dir = dir.path().join("study.curvebook.d");
    let mut doc = build_sample_project()?;
    save_project_unpacked_dir(&doc, &proj_dir)?;

    std::fs::create_dir_all(proj_dir.join(".git"))?;
    std::fs::write(proj_dir.join(".git/HEAD"), b"ref: refs/heads/main\n")?;
    std::fs::write(proj_dir.join("README.md"), b"keep readme\n")?;
    std::fs::write(proj_dir.join("sheets/notes.txt"), b"keep researcher notes\n")?;

    // Poison manifest.json with untrusted `csv_path` targeting `.git/HEAD` and `README.md`.
    let mut manifest: ArchiveManifest = serde_json::from_slice(&std::fs::read(proj_dir.join("manifest.json"))?)?;
    manifest.sheets.push(SheetManifestEntry {
        id: SheetId::new("poison-git"),
        section: SheetSection::Data,
        title: String::from("Poison"),
        csv_path: Some(String::from(".git/HEAD")),
        crc32: Some(0),
    });
    manifest.sheets.push(SheetManifestEntry {
        id: SheetId::new("poison-readme"),
        section: SheetSection::Data,
        title: String::from("Poison2"),
        csv_path: Some(String::from("README.md")),
        crc32: Some(0),
    });
    std::fs::write(proj_dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;

    // Open must reject non-`sheets/<stem>.csv` paths.
    assert!(matches!(open_project_unpacked_dir(&proj_dir), Err(FormatError::UnsafePath { .. })));

    // Re-save after removing `sheet-a` must NOT touch `.git/HEAD`, `README.md`, or `sheets/notes.txt`,
    // while keeping `sheets/` because `sheets/notes.txt` still exists there.
    doc.result_sheets.clear();
    doc.data_sheets.clear();
    doc.checkpoints.clear();
    save_project_unpacked_dir(&doc, &proj_dir)?;

    assert_eq!(std::fs::read(proj_dir.join(".git/HEAD"))?, b"ref: refs/heads/main\n");
    assert_eq!(std::fs::read(proj_dir.join("README.md"))?, b"keep readme\n");
    assert_eq!(std::fs::read(proj_dir.join("sheets/notes.txt"))?, b"keep researcher notes\n");
    assert!(!proj_dir.join("sheets/sheet-a.csv").exists());
    Ok(())
}

#[test]
fn r2_s06_02_registry_distinguishes_unpacked_project_dirs_from_arbitrary_dirs() -> TestResult {
    let dir = TempDirGuard::new("registry-dir")?;
    let reg = FormatRegistry::default_registry();

    let arbitrary_dir = dir.path().join("workspace");
    let dot_non_project_dir = dir.path().join("not_a_project.curvebook");
    let json_dir = dir.path().join("export.json");
    let empty_d_dir = dir.path().join("fresh.curvebook.d");
    std::fs::create_dir_all(&arbitrary_dir)?;
    std::fs::create_dir_all(&dot_non_project_dir)?;
    std::fs::create_dir_all(&json_dir)?;
    std::fs::create_dir_all(&empty_d_dir)?;

    assert!(reg.importer_for_path(&arbitrary_dir).is_none());
    assert!(reg.exporter_for_path(&arbitrary_dir).is_none());
    assert!(reg.importer_for_path(&dot_non_project_dir).is_none());
    assert!(reg.exporter_for_path(&dot_non_project_dir).is_none());
    assert!(reg.importer_for_path(&json_dir).is_none());
    assert!(reg.exporter_for_path(&json_dir).is_none());
    assert_eq!(reg.exporter_for_path(&empty_d_dir).ok_or("missing .curvebook.d exporter")?.descriptor().kind, FormatKind::ProjectDir);

    // Valid unpacked project directory whose folder name ends in `.curvebook` must still resolve to `curvebook.d`.
    let valid_dot_dir = dir.path().join("study.curvebook");
    let doc = build_sample_project()?;
    save_project_unpacked_dir(&doc, &valid_dot_dir)?;
    assert_eq!(reg.importer_for_path(&valid_dot_dir).ok_or("importer")?.descriptor().kind, FormatKind::ProjectDir);
    assert_eq!(reg.exporter_for_path(&valid_dot_dir).ok_or("exporter")?.descriptor().kind, FormatKind::ProjectDir);
    let imported = reg.importer_for_path(&valid_dot_dir).ok_or("importer")?.import_path(&valid_dot_dir, &Default::default())?.into_document();
    assert_eq!(imported.as_ref().map(|d| d.title.as_str()), Some("R2 Audit Study"));
    Ok(())
}

#[test]
fn r2_s06_03_save_paths_reject_newer_schema_versions_on_doc_and_disk() -> TestResult {
    let dir = TempDirGuard::new("schema-guard")?;
    let mut doc = build_sample_project()?;
    let reg = FormatRegistry::default_registry();

    // In-memory future doc fails on all save paths.
    let mut future_doc = doc.clone();
    future_doc.schema_version = CURRENT_SCHEMA_VERSION + 1;
    assert!(matches!(save_project_bytes(&future_doc), Err(FormatError::UnsupportedSchemaVersion { .. })));
    assert!(matches!(save_template_bytes(&future_doc), Err(FormatError::UnsupportedSchemaVersion { .. })));
    assert!(matches!(save_project_file(&dir.path().join("fut.curvebook"), &future_doc), Err(FormatError::UnsupportedSchemaVersion { .. })));
    assert!(matches!(save_project_unpacked_dir(&future_doc, &dir.path().join("fut.curvebook.d")), Err(FormatError::UnsupportedSchemaVersion { .. })));

    // Build on-disk v+1 `.curvebook` and `.cbtemplate` ZIPs manually.
    let future_ver = CURRENT_SCHEMA_VERSION + 2;
    let future_manifest = serde_json::json!({
        "format": "curvebook-archive",
        "schema_version": future_ver,
        "kind": "Project",
        "title": "Future",
        "document_crc32": 0,
        "sheets": [],
        "future_required_field": true
    });
    let mut zip_w = ZipWriter::new();
    zip_w.add_file("manifest.json", serde_json::to_vec(&future_manifest)?)?;
    zip_w.add_file("document.json", format!(r#"{{"schema_version":{future_ver},"title":"Future"}}"#).into_bytes())?;
    let future_zip_bytes = zip_w.finish()?;

    let proj_path = dir.path().join("vnext.curvebook");
    let tpl_path = dir.path().join("vnext.cbtemplate");
    std::fs::write(&proj_path, &future_zip_bytes)?;
    std::fs::write(&tpl_path, &future_zip_bytes)?;

    doc.schema_version = CURRENT_SCHEMA_VERSION;
    assert!(matches!(save_project_file(&proj_path, &doc), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver));
    assert!(matches!(save_template_file(&tpl_path, &doc), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver));
    assert!(matches!(
        reg.exporter_by_format("curvebook").ok_or("archive exporter")?.export_path(
            ExportSource::Document(&doc),
            &proj_path,
            &ExportOptions::default()
        ),
        Err(FormatError::UnsupportedSchemaVersion { .. })
    ));
    assert!(matches!(
        reg.exporter_by_format("cbtemplate").ok_or("template exporter")?.export_path(
            ExportSource::Document(&doc),
            &tpl_path,
            &ExportOptions::default()
        ),
        Err(FormatError::UnsupportedSchemaVersion { .. })
    ));
    assert_eq!(std::fs::read(&proj_path)?, future_zip_bytes);
    assert_eq!(std::fs::read(&tpl_path)?, future_zip_bytes);

    // Unpacked `.curvebook.d/` with future manifest OR future document.json.
    let unpacked_vnext = dir.path().join("vnext.curvebook.d");
    std::fs::create_dir_all(&unpacked_vnext)?;
    std::fs::write(unpacked_vnext.join("manifest.json"), serde_json::to_vec(&future_manifest)?)?;
    assert!(
        matches!(save_project_unpacked_dir(&doc, &unpacked_vnext), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver)
    );

    let unpacked_doc_vnext = dir.path().join("docvnext.curvebook.d");
    std::fs::create_dir_all(&unpacked_doc_vnext)?;
    std::fs::write(unpacked_doc_vnext.join("document.json"), format!(r#"{{"schema_version":{future_ver},"title":"FutureDoc"}}"#))?;
    assert!(
        matches!(save_project_unpacked_dir(&doc, &unpacked_doc_vnext), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver)
    );
    Ok(())
}

#[test]
fn r2_s06_04_unpacked_save_rejects_dir_conflicts_and_preserves_crc_consistency() -> TestResult {
    let mut dir = TempDirGuard::new("unpacked-tx")?;
    let proj_dir = dir.path().join("tx.curvebook.d");
    let doc_v1 = build_sample_project()?;
    save_project_unpacked_dir(&doc_v1, &proj_dir)?;

    // Conflict: directory at target `sheets/sheet-b.csv/` when saving `doc_v2`.
    let mut doc_v2 = doc_v1.clone();
    doc_v2.title = String::from("V2 Should Abort Cleanly");
    let mut col2 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?;
    col2.data_sets[0].cells[0] = vec![Cell::number(42.0)];
    doc_v2.add_data_sheet(DataSheet::new("sheet-b", "Cohort B", TableArchetype::Column(col2))?)?;
    std::fs::create_dir_all(proj_dir.join("sheets/sheet-b.csv"))?;

    assert!(save_project_unpacked_dir(&doc_v2, &proj_dir).is_err());
    assert_eq!(open_project_unpacked_dir(&proj_dir)?, doc_v1);
    std::fs::remove_dir_all(proj_dir.join("sheets/sheet-b.csv"))?;

    // Stage failure on read-only `sheets/` dir leaves live `document.json` and `manifest.json` untouched.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let sheets_dir = proj_dir.join("sheets");
        dir.track_chmod_restore(sheets_dir.clone());
        std::fs::set_permissions(&sheets_dir, std::fs::Permissions::from_mode(0o555))?;
        let res = save_project_unpacked_dir(&doc_v2, &proj_dir);
        let _ = std::fs::set_permissions(&sheets_dir, std::fs::Permissions::from_mode(0o755));
        // Root UID 0 ignores 0555 DAC bits, so only assert failure when not running as root.
        if res.is_err() {
            assert_eq!(open_project_unpacked_dir(&proj_dir)?, doc_v1);
        }
    }
    Ok(())
}

#[test]
fn r2_s06_05_unpacked_save_enforces_zip_limits_matching_reopen() -> TestResult {
    let dir = TempDirGuard::new("ziplimits")?;
    let proj_dir = dir.path().join("limits.curvebook.d");
    let base_doc = build_sample_project()?;
    save_project_unpacked_dir(&base_doc, &proj_dir)?;

    // Exceed 64 MiB single-entry limit (`document.json`) via valid 4 KiB constants inside InfoSheets.
    let mut big_doc = ProjectDocument::new("Oversized")?;
    let huge_name = "x".repeat(MAX_TITLE_BYTES);
    let mut info = InfoSheet::new_preseeded("info-big", "Big Constants")?;
    for i in 0..17_000 {
        info.constants.push(InfoConstant { name: huge_name.clone(), value: curvebook_doc::ConstantValue::Number(i as f64), units: None });
    }
    big_doc.add_info_sheet(info)?;

    let err = save_project_unpacked_dir(&big_doc, &proj_dir).err().ok_or("expected ZipLimits overflow to fail")?;
    assert!(matches!(err, FormatError::ZipBomb { .. }), "unexpected error: {err:?}");
    assert!(serde_json::to_vec_pretty(&big_doc)?.len() as u64 > MAX_ZIP_ENTRY_BYTES);
    assert_eq!(open_project_unpacked_dir(&proj_dir)?, base_doc);
    assert_eq!(ZipLimits::default().max_entry_uncompressed_bytes, MAX_ZIP_ENTRY_BYTES);
    Ok(())
}

#[test]
fn r2_s06_06_unique_sheet_csv_path_handles_case_collisions_long_ids_and_reserved_names() -> TestResult {
    let dir = TempDirGuard::new("csv-names")?;
    let proj_dir = dir.path().join("portable.curvebook.d");
    let mut doc = ProjectDocument::new("Portable Names")?;

    let long_id = "L".repeat(300);
    let ids = ["WT", "wt", "Wt", "CON", "con", "prn", "Aux", "nul", "COM1", "lpt9", long_id.as_str()];
    for (idx, raw_id) in ids.iter().enumerate() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?;
        col.data_sets[0].cells[0] = vec![Cell::number((idx + 1) as f64)];
        doc.add_data_sheet(DataSheet::new(*raw_id, format!("Sheet {idx}"), TableArchetype::Column(col))?)?;
    }

    save_project_unpacked_dir(&doc, &proj_dir)?;
    let manifest: ArchiveManifest = serde_json::from_slice(&std::fs::read(proj_dir.join("manifest.json"))?)?;
    let mut lower_paths = BTreeSet::new();
    for entry in &manifest.sheets {
        if let Some(csv_path) = &entry.csv_path {
            assert!(lower_paths.insert(csv_path.to_ascii_lowercase()), "case collision in {csv_path}");
            let fname = Path::new(csv_path).file_name().and_then(|s| s.to_str()).ok_or("filename")?;
            assert!(fname.len() <= 96, "filename too long: {fname}");
            let stem = fname.strip_suffix(".csv").ok_or(".csv suffix")?;
            assert!(
                !matches!(
                    stem.to_ascii_uppercase().as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                ),
                "reserved Windows device stem emitted: {stem}"
            );
        }
    }
    assert_eq!(open_project_unpacked_dir(&proj_dir)?, doc);
    Ok(())
}

#[test]
fn r2_s06_07_journal_lock_waits_for_fresh_empty_in_flight_lockfile() -> TestResult {
    let dir = TempDirGuard::new("lock-race")?;
    let lock_path = dir.path().join(LOCKFILE_NAME);
    let lease_a = JournalLockLease::new("owner-a", 10_000, 60_000)?;
    let lease_b = JournalLockLease::new("owner-b", 10_000, 60_000)?;
    let lease_a_bytes = serde_json::to_vec(&lease_a)?;

    // Create 0-byte in-flight lockfile, signal challenger, wait 25ms, then flush owner-a lease.
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&lock_path)?;
    let barrier = Arc::new(Barrier::new(2));
    let barrier_writer = Arc::clone(&barrier);
    let writer = std::thread::spawn(move || -> Result<(), std::io::Error> {
        use std::io::Write as _;
        barrier_writer.wait();
        std::thread::sleep(Duration::from_millis(25));
        file.write_all(&lease_a_bytes)?;
        file.sync_all()?;
        Ok(())
    });

    barrier.wait();
    let mut backend_b = DirectoryJournalBackend::open_dir(dir.path())?;
    let err = backend_b.acquire_lease(&lease_b, 10_030).err().ok_or("expected lock contention error, not eviction")?;
    writer.join().map_err(|_| "writer thread panicked")??;

    let msg = err.to_string();
    assert!(msg.contains("actively held by `owner-a`"), "unexpected contention error: {msg}");
    assert_eq!(backend_b.read_lease()?.map(|l| l.owner_id), Some(String::from("owner-a")));
    Ok(())
}

#[test]
fn r2_s06_08_json_adapter_validates_floats_exports_result_sheet_and_returns_sheet_not_found() -> TestResult {
    let reg = FormatRegistry::default_registry();
    let json_exp = reg.exporter_by_format("json").ok_or("missing json exporter")?;
    let csv_exp = reg.exporter_by_format("csv").ok_or("missing csv exporter")?;
    let tsv_exp = reg.exporter_by_format("tsv").ok_or("missing tsv exporter")?;

    let mut doc = build_sample_project()?;

    // 1. Default ExportSource::Document exports first ResultSheet (ResultTable), not full ProjectDocument.
    let default_json: Value = serde_json::from_slice(&json_exp.export_bytes(ExportSource::Document(&doc), &ExportOptions::default())?)?;
    assert_eq!(default_json["id"], "res-1");
    assert!(default_json.get("data_sheets").is_none());

    // Document without ResultSheets fails cleanly on default JSON export.
    let mut no_res = doc.clone();
    no_res.result_sheets.clear();
    assert!(matches!(json_exp.export_bytes(ExportSource::Document(&no_res), &ExportOptions::default()), Err(FormatError::UnsupportedFormat { .. })));

    // 2. Missing sheet_id returns DocError::SheetNotFound across JSON, CSV, and TSV.
    let missing_opts = ExportOptions { sheet_id: Some(String::from("does-not-exist")), ..ExportOptions::default() };
    for exp in [json_exp, csv_exp, tsv_exp] {
        let err = exp.export_bytes(ExportSource::Document(&doc), &missing_opts).err().ok_or("expected SheetNotFound")?;
        assert!(matches!(err, FormatError::Doc(DocError::SheetNotFound { .. })), "expected SheetNotFound, got {err:?}");
    }

    // 3. Non-finite float inside DataSheet must fail JSON export instead of turning into `null`.
    if let TableArchetype::Column(ref mut c) = doc.data_sheets[0].archetype {
        c.data_sets[0].cells[0][0] = Cell { value: CellValue::Number(f64::NAN), excluded: false };
    }
    assert!(matches!(
        json_exp.export_bytes(ExportSource::DataSheet(&doc.data_sheets[0]), &ExportOptions::default()),
        Err(FormatError::NonFiniteFloat { .. })
    ));
    let ds_opts = ExportOptions { sheet_id: Some(String::from("sheet-a")), ..ExportOptions::default() };
    assert!(matches!(json_exp.export_bytes(ExportSource::Document(&doc), &ds_opts), Err(FormatError::NonFiniteFloat { .. })));
    Ok(())
}

#[test]
fn r2_s06_09_batch_schema_and_version_validator_match_cli_contract() -> TestResult {
    let schema = batch_v1_schema();

    assert_eq!(schema["not"]["properties"]["template"]["type"], "string");
    assert_eq!(schema["not"]["properties"]["project"]["type"], "string");

    let input_props = schema["$defs"]["BatchInput"]["properties"].as_object().ok_or("BatchInput properties")?;
    for expected_key in ["sheet_id", "sheetId", "table_kind", "tableKind", "subcolumnMode", "decimalSeparator"] {
        assert!(input_props.contains_key(expected_key), "BatchInput missing `{expected_key}`");
    }
    assert!(input_props["delimiter"].get("maxLength").is_none(), "delimiter must allow multi-char tokens like `tab`/`\\t`");
    assert_eq!(schema["$defs"]["BatchInput"]["anyOf"].as_array().map(Vec::len), Some(4));

    let cmd_def = &schema["$defs"]["BatchCommand"];
    assert_eq!(cmd_def["properties"]["params"]["type"], json!(["object", "null"]));
    assert_eq!(cmd_def["anyOf"].as_array().map(Vec::len), Some(3));

    let exp_props = schema["$defs"]["BatchExport"]["properties"].as_object().ok_or("BatchExport properties")?;
    for expected_key in ["sheet_id", "sheetId", "tabIndex", "pdfVersion", "ttfFont"] {
        assert!(exp_props.contains_key(expected_key), "BatchExport missing `{expected_key}`");
    }
    let formats: BTreeSet<&str> = exp_props["format"]["enum"].as_array().ok_or("format enum")?.iter().filter_map(Value::as_str).collect();
    for required_fmt in ["tif", "curvebook-dir", "unpacked", "dir", "emf"] {
        assert!(formats.contains(required_fmt), "BatchExport.format missing `{required_fmt}`");
    }
    assert!(!formats.contains("cbtemplate"), "BatchExport.format must not advertise unsupported `cbtemplate`");
    assert_eq!(exp_props["dpi"]["type"], json!(["integer", "null"]));
    assert_eq!(exp_props["dpi"]["minimum"], 1);
    assert_eq!(exp_props["dpi"]["maximum"], 65_535);

    assert!(validate_batch_manifest_version(&json!({ "schema": "curvebook.batch.v1" })).is_ok());
    assert!(matches!(
        validate_batch_manifest_version(&json!({ "schema": "curvebook.batch.v2" })),
        Err(FormatError::UnsupportedSchemaVersion { version: 2, max_supported: 1 })
    ));
    assert!(matches!(
        validate_batch_manifest_version(&json!({ "schema": "curvebook.batch.v0" })),
        Err(FormatError::UnsupportedSchemaVersion { version: 0, max_supported: 1 })
    ));
    for malformed in ["curvebook.batch.v01", "curvebook.batch.v+1", "curvebook.batch.v-1", "lumencraft.batch.v1", "lumencraft.batch.v2"] {
        assert!(
            matches!(validate_batch_manifest_version(&json!({ "schema": malformed })), Err(FormatError::UnsupportedFormat { .. })),
            "`{malformed}` should be rejected as UnsupportedFormat"
        );
    }
    Ok(())
}
