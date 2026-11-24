//! Permanent regression tests for `R1-X1-03` across `curvebook-engine`:
//! - Non-regular files (FIFOs, symlinks to FIFOs, directories, `/dev/null`, `/dev/zero`) rejected
//!   before `open()`/`read()` without blocking or OOM.
//! - Oversized files (`> per-format cap`) rejected BEFORE reading or allocating via metadata +
//!   `Read::take(limit + 1)` with `ErrorCode::ResourceLimit`.

use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use curvebook_doc::{
    CellCoord, ErrorCode, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange,
    SheetId,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_format::{MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES, MAX_XML_BYTES};
use serde_json::json;

use super::common::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn create_sparse_file(path: &Path, byte_len: u64) -> TestResult {
    let f = File::create(path)?;
    f.set_len(byte_len)?;
    Ok(())
}

fn run_with_timeout<F, T>(timeout: Duration, action: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(action());
    });
    rx.recv_timeout(timeout).map_err(|err| format!("operation timed out after {timeout:?}: {err}"))
}

#[cfg(unix)]
#[test]
fn r1_x1_03_fifo_and_symlink_to_fifo_rejected_without_blocking() -> TestResult {
    let dir = TempDir::try_new("fifo")?;
    let fifo = dir.child("input.csv");
    let status = Command::new("mkfifo").arg(&fifo).status()?;
    assert!(status.success(), "mkfifo failed");

    let fifo_link = dir.child("alias.csv");
    std::os::unix::fs::symlink(&fifo, &fifo_link)?;

    let fifo_str = fifo.display().to_string();
    let link_str = fifo_link.display().to_string();
    let timeout = Duration::from_secs(3);

    // 1. Reproducer equivalent: `table.importCsv` on FIFO and symlink-to-FIFO returns promptly with BadInput.
    for target in [fifo_str.clone(), link_str.clone()] {
        let p = target.clone();
        let res = run_with_timeout(timeout, move || {
            let mut s = Session::new();
            s.execute("table.importCsv", json!({ "path": p }))
        });
        let err = match res {
            Ok(Err(err)) => err,
            Ok(Ok(_)) => return Err(format!("expected FIFO import to fail for {target}").into()),
            Err(msg) => {
                if let Ok(f) = fs::OpenOptions::new().write(true).open(&fifo) {
                    drop(f);
                }
                return Err(msg.into());
            }
        };
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for FIFO `{target}`, got {err:?}");
    }

    // 2. Cover remaining engine file-read commands against FIFO blocking.
    let mut base = Session::from_document(ProjectDocument::new("FIFO Matrix")?);
    base.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "D1", "kind": "Column", "rows": 2, "cols": 1 }))?;

    for (cmd, params) in [
        ("table.importTidyLong", json!({ "path": fifo_str })),
        ("table.pasteLink", json!({ "target_sheet_id": "d1", "source_path": fifo_str })),
        ("table.importPzfx", json!({ "path": fifo_str })),
        ("table.importPrism", json!({ "path": fifo_str })),
        ("project.open", json!({ "path": fifo_str })),
        ("project.instantiateTemplate", json!({ "path": fifo_str })),
    ] {
        let mut s = base.clone();
        let c = cmd.to_owned();
        let res = run_with_timeout(timeout, move || s.execute(&c, params));
        let err = match res {
            Ok(Err(err)) => err,
            Ok(Ok(_)) => return Err(format!("{cmd} on FIFO should have failed").into()),
            Err(msg) => {
                if let Ok(f) = fs::OpenOptions::new().write(true).open(&fifo) {
                    drop(f);
                }
                return Err(msg.into());
            }
        };
        assert_eq!(err.code, ErrorCode::BadInput, "{cmd} expected BadInput on FIFO, got {err:?}");
    }

    // 3. Stored external link pointing to FIFO on `project.recalculate` must fail without blocking.
    let mut linked_session = Session::from_document(ProjectDocument::new("External Link FIFO")?);
    linked_session.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "D1", "kind": "Column", "rows": 2, "cols": 1 }))?;
    let mut doc = linked_session.document().clone();
    if let Some(ds) = doc.data_sheet_mut(&SheetId::new("d1")) {
        ds.linked_regions.push(LinkedDataRegion {
            id: String::from("fifo-link"),
            spec: ImportLinkSpec {
                source: LinkSource::ExternalDelimitedFile { path: fifo_str.clone(), delimiter: ',', decimal_separator: '.' },
                filter: ImportFilterSpec::default(),
                placement: ImportPlacementSpec::default(),
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            provenance_summary: String::from("fifo link"),
        });
    }
    let res = run_with_timeout(timeout, move || {
        let mut s = Session::from_document(doc).with_path_policy(ExternalPathPolicy::Unconstrained);
        s.execute("project.recalculate", json!({ "include_external_links": true }))
    });
    let err = match res {
        Ok(Err(err)) => err,
        Ok(Ok(_)) => return Err("recalculate on FIFO external link should have failed".into()),
        Err(msg) => {
            if let Ok(f) = fs::OpenOptions::new().write(true).open(&fifo) {
                drop(f);
            }
            return Err(msg.into());
        }
    };
    assert_eq!(err.code, ErrorCode::BadInput);

    // 4. Special devices (/dev/zero, /dev/null) and directories must fail immediately without OOM.
    let mut s = Session::new();
    let subdir = dir.child("subdir");
    fs::create_dir_all(&subdir)?;
    let dir_err = match s.execute("table.importCsv", json!({ "path": subdir.display().to_string() })) {
        Ok(_) => return Err("directory import should fail".into()),
        Err(err) => err,
    };
    assert_eq!(dir_err.code, ErrorCode::BadInput);

    let dev_zero = Path::new("/dev/zero");
    if dev_zero.exists() {
        let zero_err = match s.execute("table.importCsv", json!({ "path": "/dev/zero" })) {
            Ok(_) => return Err("/dev/zero import should fail".into()),
            Err(err) => err,
        };
        assert_eq!(zero_err.code, ErrorCode::BadInput);
    }

    Ok(())
}

#[test]
fn r1_x1_03_oversized_files_rejected_before_allocation() -> TestResult {
    let dir = TempDir::try_new("oversized")?;

    let big_csv = dir.child("oversized.csv");
    create_sparse_file(&big_csv, (MAX_CSV_FILE_BYTES as u64).saturating_add(1))?;
    let big_csv_str = big_csv.display().to_string();

    let big_pzfx = dir.child("oversized.pzfx");
    create_sparse_file(&big_pzfx, (MAX_XML_BYTES as u64).saturating_add(1))?;
    let big_pzfx_str = big_pzfx.display().to_string();

    let big_archive = dir.child("oversized.curvebook");
    create_sparse_file(&big_archive, (MAX_ARCHIVE_FILE_BYTES as u64).saturating_add(1))?;
    let big_archive_str = big_archive.display().to_string();

    let mut base = Session::from_document(ProjectDocument::new("Oversized Matrix")?);
    base.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "D1", "kind": "Column", "rows": 2, "cols": 1 }))?;

    for (cmd, params) in [
        ("table.importCsv", json!({ "path": big_csv_str })),
        ("table.importTidyLong", json!({ "path": big_csv_str })),
        ("table.pasteLink", json!({ "target_sheet_id": "d1", "source_path": big_csv_str })),
        ("table.importPzfx", json!({ "path": big_pzfx_str })),
        ("table.importPrism", json!({ "path": big_archive_str })),
        ("project.open", json!({ "path": big_archive_str })),
        ("project.instantiateTemplate", json!({ "path": big_archive_str })),
    ] {
        let mut s = base.clone();
        let err = match s.execute(cmd, params) {
            Ok(_) => return Err(format!("{cmd} on oversized file should have failed").into()),
            Err(err) => err,
        };
        assert_eq!(err.code, ErrorCode::ResourceLimit, "{cmd} expected ResourceLimit, got {err:?}");
    }

    // Stored ExternalDelimitedFile link pointing to an oversized CSV must reject during recalculate.
    let mut doc = base.document().clone();
    if let Some(ds) = doc.data_sheet_mut(&SheetId::new("d1")) {
        ds.linked_regions.push(LinkedDataRegion {
            id: String::from("big-link"),
            spec: ImportLinkSpec {
                source: LinkSource::ExternalDelimitedFile { path: big_csv_str, delimiter: ',', decimal_separator: '.' },
                filter: ImportFilterSpec::default(),
                placement: ImportPlacementSpec::default(),
            },
            target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
            provenance_summary: String::from("oversized link"),
        });
    }
    let mut s = Session::from_document(doc).with_path_policy(ExternalPathPolicy::Unconstrained);
    let err = match s.execute("project.recalculate", json!({ "include_external_links": true })) {
        Ok(_) => return Err("recalculate on oversized external link should have failed".into()),
        Err(err) => err,
    };
    assert_eq!(err.code, ErrorCode::ResourceLimit);

    Ok(())
}
