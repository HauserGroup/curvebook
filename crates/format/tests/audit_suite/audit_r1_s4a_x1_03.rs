//! Permanent regression tests for `R1-X1-03` across `curvebook-format`:
//! - `open_project_file`, `open_template_file`, `load_preferences_file`, and `FormatRegistry::import_path`
//!   enforce regular-file checks (no FIFO blocking, no directory/device reading) and per-format byte caps
//!   before reading or allocating.
use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use crate::common::TempDirGuard;
use curvebook_format::{
    FormatError, FormatRegistry, ImportOptions, MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES, MAX_PREFERENCES_FILE_BYTES, MAX_XML_BYTES,
    load_preferences_file, open_project_file, open_template_file,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn create_sparse_file(path: &Path, byte_len: u64) -> TestResult {
    let f = File::create(path)?;
    f.set_len(byte_len)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn r1_x1_03_format_readers_reject_fifo_symlink_and_devices_without_blocking() -> TestResult {
    let dir = TempDirGuard::new("fifo")?;
    let fifo = dir.child("pipe.dat");
    assert!(Command::new("mkfifo").arg(&fifo).status()?.success());
    let link = dir.child("pipe_link.dat");
    std::os::unix::fs::symlink(&fifo, &link)?;

    let check = |target: &Path| -> Result<(), String> {
        let owned = target.to_path_buf();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut results = Vec::new();
            results.push(open_project_file(&owned).is_err());
            results.push(open_template_file(&owned).is_err());
            results.push(load_preferences_file(&owned).is_err());
            let reg = FormatRegistry::default_registry();
            for fmt_id in ["curvebook", "cbtemplate", "prism", "pzfx", "csv", "tsv"] {
                if let Some(imp) = reg.importer_by_format(fmt_id) {
                    results.push(imp.import_path(&owned, &ImportOptions::default()).is_err());
                }
            }
            let _ = tx.send(results);
        });
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(all) => {
                if all.iter().all(|ok| *ok) {
                    Ok(())
                } else {
                    Err(String::from("at least one format reader accepted non-regular file"))
                }
            }
            Err(err) => {
                if let Ok(f) = fs::OpenOptions::new().write(true).open(&fifo) {
                    drop(f);
                }
                Err(format!("format reader hung on FIFO: {err}"))
            }
        }
    };

    check(&fifo)?;
    check(&link)?;

    let subdir = dir.child("folder");
    fs::create_dir_all(&subdir)?;
    assert!(matches!(open_project_file(&subdir), Err(FormatError::Io(_))));
    assert!(matches!(open_template_file(&subdir), Err(FormatError::Io(_))));
    assert!(matches!(load_preferences_file(&subdir), Err(FormatError::Io(_))));

    let dev_zero = Path::new("/dev/zero");
    if dev_zero.exists() {
        assert!(matches!(open_project_file(dev_zero), Err(FormatError::Io(_))));
        assert!(matches!(load_preferences_file(dev_zero), Err(FormatError::Io(_))));
    }

    Ok(())
}

#[test]
fn r1_x1_03_format_readers_enforce_per_format_caps_before_allocation() -> TestResult {
    let dir = TempDirGuard::new("oversized")?;
    let reg = FormatRegistry::default_registry();

    let big_proj = dir.child("big.curvebook");
    create_sparse_file(&big_proj, (MAX_ARCHIVE_FILE_BYTES as u64).saturating_add(1))?;
    assert!(matches!(open_project_file(&big_proj), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_ARCHIVE_FILE_BYTES));

    let big_tpl = dir.child("big.cbtemplate");
    create_sparse_file(&big_tpl, (MAX_ARCHIVE_FILE_BYTES as u64).saturating_add(1))?;
    assert!(matches!(open_template_file(&big_tpl), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_ARCHIVE_FILE_BYTES));

    let big_prefs = dir.child("prefs.json");
    create_sparse_file(&big_prefs, (MAX_PREFERENCES_FILE_BYTES as u64).saturating_add(1))?;
    assert!(
        matches!(load_preferences_file(&big_prefs), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_PREFERENCES_FILE_BYTES)
    );

    let cases: [(&str, &str, usize); 6] = [
        ("curvebook", "a.curvebook", MAX_ARCHIVE_FILE_BYTES),
        ("cbtemplate", "b.cbtemplate", MAX_ARCHIVE_FILE_BYTES),
        ("prism", "c.prism", MAX_ARCHIVE_FILE_BYTES),
        ("pzfx", "d.pzfx", MAX_XML_BYTES),
        ("csv", "e.csv", MAX_CSV_FILE_BYTES),
        ("tsv", "f.tsv", MAX_CSV_FILE_BYTES),
    ];

    for (fmt_id, name, expected_cap) in cases {
        let p = dir.child(name);
        create_sparse_file(&p, (expected_cap as u64).saturating_add(1))?;
        let importer = reg.importer_by_format(fmt_id).ok_or_else(|| format!("missing importer {fmt_id}"))?;
        let res = importer.import_path(&p, &ImportOptions::default());
        assert!(
            matches!(res, Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == expected_cap),
            "importer {fmt_id} did not enforce FileTooLarge({expected_cap}): {res:?}"
        );
    }

    Ok(())
}
