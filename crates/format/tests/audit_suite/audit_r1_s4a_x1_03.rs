//! Permanent regression tests for `R1-X1-03` across `curvebook-format`:
//! regular-file checks (no FIFO blocking, no directory/device reading) and per-format byte caps.
use std::fs;
use std::path::Path;

#[cfg(unix)]
use crate::common::run_with_fifo_watchdog;
use crate::common::{TempDirGuard, create_sparse_file};
use curvebook_format::{
    FormatError, FormatRegistry, ImportOptions, MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES, MAX_PREFERENCES_FILE_BYTES, MAX_XML_BYTES,
    load_preferences_file, open_project_file, open_template_file,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[cfg(unix)]
#[test]
fn r1_x1_03_format_readers_reject_fifo_symlink_and_devices_without_blocking() -> TestResult {
    let dir = TempDirGuard::new("fifo")?;
    let fifo = dir.child("pipe.dat");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status()?.success());
    let link = dir.child("pipe_link.dat");
    std::os::unix::fs::symlink(&fifo, &link)?;

    for (label, target) in [("fifo", &fifo), ("symlink_to_fifo", &link)] {
        let owned = target.clone();
        run_with_fifo_watchdog(&fifo, label, move || {
            let mut results = vec![open_project_file(&owned).is_err(), open_template_file(&owned).is_err(), load_preferences_file(&owned).is_err()];
            let reg = FormatRegistry::default_registry();
            for fmt_id in ["curvebook", "cbtemplate", "prism", "pzfx", "csv", "tsv"] {
                if let Some(imp) = reg.importer_by_format(fmt_id) {
                    results.push(imp.import_path(&owned, &ImportOptions::default()).is_err());
                }
            }
            if results.iter().all(|ok| *ok) { Ok(()) } else { Err(format!("{label}: at least one format reader accepted non-regular file")) }
        })?;
    }

    let subdir = dir.child("folder");
    fs::create_dir_all(&subdir)?;
    for target in [&subdir, Path::new("/dev/zero")] {
        if !target.exists() {
            continue;
        }
        assert!(matches!(open_project_file(target), Err(FormatError::Io(_))), "open_project_file accepted {}", target.display());
        assert!(matches!(load_preferences_file(target), Err(FormatError::Io(_))), "load_preferences_file accepted {}", target.display());
    }
    assert!(matches!(open_template_file(&subdir), Err(FormatError::Io(_))));
    Ok(())
}

#[test]
fn r1_x1_03_format_readers_enforce_per_format_caps_before_allocation() -> TestResult {
    let dir = TempDirGuard::new("oversized")?;
    let reg = FormatRegistry::default_registry();

    type PathCheck = fn(&Path) -> bool;
    let direct_cases: [(&str, usize, PathCheck); 3] = [
        (
            "big.curvebook",
            MAX_ARCHIVE_FILE_BYTES,
            |p| matches!(open_project_file(p), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_ARCHIVE_FILE_BYTES),
        ),
        (
            "big.cbtemplate",
            MAX_ARCHIVE_FILE_BYTES,
            |p| matches!(open_template_file(p), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_ARCHIVE_FILE_BYTES),
        ),
        (
            "prefs.json",
            MAX_PREFERENCES_FILE_BYTES,
            |p| matches!(load_preferences_file(p), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_PREFERENCES_FILE_BYTES),
        ),
    ];
    for (name, cap, check) in direct_cases {
        let p = dir.child(name);
        create_sparse_file(&p, (cap as u64).saturating_add(1))?;
        assert!(check(&p), "direct reader for {name} did not enforce FileTooLarge({cap})");
    }

    let importer_cases: [(&str, &str, usize); 6] = [
        ("curvebook", "a.curvebook", MAX_ARCHIVE_FILE_BYTES),
        ("cbtemplate", "b.cbtemplate", MAX_ARCHIVE_FILE_BYTES),
        ("prism", "c.prism", MAX_ARCHIVE_FILE_BYTES),
        ("pzfx", "d.pzfx", MAX_XML_BYTES),
        ("csv", "e.csv", MAX_CSV_FILE_BYTES),
        ("tsv", "f.tsv", MAX_CSV_FILE_BYTES),
    ];
    for (fmt_id, name, expected_cap) in importer_cases {
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
