//! Regression tests for Wave 1 SEC-04 S1 findings (`FA-SEC04-01`, `FA-SEC04-02`, `FA-SEC04-03`).

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use crate::common::run_with_fifo_watchdog;
use crate::common::{
    TempDirGuard, assert_no_lock_temp_or_evict_leftovers, build_zip_entries, create_sparse_file, encode_rgba_png, open_dir_journal, write_stale_lock,
};
use curvebook_doc::{CURRENT_SCHEMA_VERSION, DocError, MAX_IMAGE_ASSET_BYTES, MAX_PROJECT_IMAGE_ASSETS, ProjectDocument, content_addressed_asset_id};
use curvebook_format::recovery::{
    CrashRecoveryJournal, DirectoryJournalBackend, JOURNAL_FILE_NAME, JournalBackend, LOCKFILE_NAME, MAX_JOURNAL_BYTES, SNAPSHOT_FILE_NAME,
};
use curvebook_format::{FormatError, MAX_ARCHIVE_FILE_BYTES, import_prism_bytes, ingest_image_asset};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const EVICTION_LOCK_FILE_NAME: &str = ".curvebook.lock.evict";
const SNAPSHOT_META_FILE_NAME: &str = "snapshot.meta.json";
const LOCKFILE_CAP_BYTES: u64 = 64 * 1024;

#[test]
fn fa_sec04_01_prism_zip_rejects_excess_and_duplicate_image_stubs_before_cloning() -> TestResult {
    let png_bytes = encode_rgba_png(4, 4);
    let asset = ingest_image_asset(&png_bytes)?;
    let asset_path = format!("assets/{}.png", asset.asset_id);
    let base_doc = ProjectDocument::new("Embedded Image Stubs")?;
    let stub = json!({ "asset_id": asset.asset_id, "media_type": "Png", "pixel_size": [4, 4] });
    let zip_with = |stubs: Vec<Value>| {
        let mut v = serde_json::to_value(&base_doc).unwrap();
        v["image_assets"] = Value::Array(stubs);
        build_zip_entries(&[("document.json", &serde_json::to_vec(&v).unwrap()), (&asset_path, &png_bytes)])
    };

    for count in [10_000, MAX_PROJECT_IMAGE_ASSETS + 1] {
        assert!(
            matches!(import_prism_bytes(&zip_with(vec![stub.clone(); count])), Err(FormatError::Doc(DocError::TooManyImageAssets { count: c, max })) if c == count && max == MAX_PROJECT_IMAGE_ASSETS)
        );
    }
    assert!(
        matches!(import_prism_bytes(&zip_with(vec![stub.clone(), stub])), Err(FormatError::DuplicateEntry { ref path }) if path == &asset.asset_id)
    );

    let mut doc_ok = base_doc;
    doc_ok.upsert_image_asset(asset.clone())?;
    let mut ok_val = serde_json::to_value(&doc_ok)?;
    ok_val["image_assets"][0].as_object_mut().unwrap().remove("bytes");
    let imported = import_prism_bytes(&build_zip_entries(&[("document.json", &serde_json::to_vec(&ok_val)?), (&asset_path, &png_bytes)]))?.document;
    assert_eq!(
        (imported.image_assets.len(), imported.image_assets[0].asset_id.as_str(), imported.image_assets[0].bytes.as_slice()),
        (1, asset.asset_id.as_str(), png_bytes.as_slice())
    );
    Ok(())
}

#[test]
fn fa_sec04_01_prism_zip_rejects_oversized_corrupt_and_missing_image_assets_before_cloning() -> TestResult {
    let base_doc = ProjectDocument::new("Embedded Image Asset Bounds")?;

    let oversized = vec![0u8; MAX_IMAGE_ASSET_BYTES + 1];
    let oversized_id = content_addressed_asset_id(&oversized);
    let mut doc_big = serde_json::to_value(&base_doc)?;
    doc_big["image_assets"] = json!([{ "asset_id": oversized_id, "media_type": "Png", "pixel_size": [4, 4] }]);
    let zip_big = build_zip_entries(&[("document.json", &serde_json::to_vec(&doc_big)?), (&format!("assets/{oversized_id}.png"), &oversized)]);
    assert!(matches!(import_prism_bytes(&zip_big), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_IMAGE_ASSET_BYTES));

    let mut doc_inline = serde_json::to_value(&base_doc)?;
    doc_inline["schema_version"] = json!(CURRENT_SCHEMA_VERSION);
    doc_inline["image_assets"] = json!([{ "asset_id": "img-0123456789abcdef", "media_type": "Png", "pixel_size": [4, 4], "bytes": "A".repeat((MAX_IMAGE_ASSET_BYTES + 1).div_ceil(3) * 4) }]);
    assert!(
        matches!(import_prism_bytes(&build_zip_entries(&[("document.json", &serde_json::to_vec(&doc_inline)?)])), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_IMAGE_ASSET_BYTES)
    );

    let (valid_png, other_png) = (encode_rgba_png(4, 4), encode_rgba_png(5, 5));
    let valid_id = content_addressed_asset_id(&valid_png);
    let mut doc_tampered = serde_json::to_value(&base_doc)?;
    doc_tampered["image_assets"] = json!([{ "asset_id": valid_id, "media_type": "Png", "pixel_size": [4, 4] }]);
    let doc_bytes = serde_json::to_vec(&doc_tampered)?;
    assert!(matches!(
        import_prism_bytes(&build_zip_entries(&[("document.json", &doc_bytes), (&format!("assets/{valid_id}.png"), &other_png)])),
        Err(FormatError::UnsupportedFormat { .. })
    ));
    assert!(matches!(import_prism_bytes(&build_zip_entries(&[("document.json", &doc_bytes)])), Err(FormatError::MissingArchiveEntry { .. })));
    Ok(())
}

#[test]
fn fa_sec04_02_oversized_and_corrupt_evict_locks_are_reclaimed_without_self_restore() -> TestResult {
    enum Payload {
        Bytes(Vec<u8>),
        Sparse(u64),
    }
    for (label, stale_owner, rescuer, payload) in [
        ("evict-70k", "crashed-owner", "rescuer-70k", Payload::Bytes(vec![b'X'; 70_000])),
        ("evict-4g", "crashed-sparse", "rescuer-4g", Payload::Sparse(4_u64 * 1024 * 1024 * 1024)),
        ("evict-corrupt", "crashed-corrupt-evict", "rescuer-corrupt", Payload::Bytes(b"{malformed-evict-guard".to_vec())),
    ] {
        let dir = TempDirGuard::new(label)?;
        write_stale_lock(dir.path(), stale_owner, 1_000, 500);
        match payload {
            Payload::Bytes(b) => fs::write(dir.child(EVICTION_LOCK_FILE_NAME), b)?,
            Payload::Sparse(sz) => create_sparse_file(&dir.child(EVICTION_LOCK_FILE_NAME), sz)?,
        }
        let (journal, report) = open_dir_journal(dir.path(), rescuer, 60_000, 10_000)?;
        assert_eq!(
            (report.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str()), journal.lease().owner_id.as_str()),
            (Some(stale_owner), rescuer),
            "{label}"
        );
        assert!(!dir.child(EVICTION_LOCK_FILE_NAME).exists(), "{label}");
        let _ = journal.close_cleanly(b"ok")?;
        assert_no_lock_temp_or_evict_leftovers(dir.path());
    }

    let dir = TempDirGuard::new("evict-live")?;
    write_stale_lock(dir.path(), "crashed-wait", 1_000, 500);
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let live_guard = json!({ "owner_id": "remote-active-evictor", "acquired_wall_ms": now_ms.saturating_add(5_000), "ttl_ms": 60_000_u64, "nonce": 42_u64, "host_id": "remote-host-fingerprint-xyz" });
    fs::write(dir.child(EVICTION_LOCK_FILE_NAME), serde_json::to_vec(&live_guard)?)?;
    assert!(matches!(open_dir_journal(dir.path(), "challenger", 60_000, 10_000), Err(FormatError::Io(_))));
    assert!(dir.child(EVICTION_LOCK_FILE_NAME).exists());
    Ok(())
}

#[test]
fn fa_sec04_03_directory_backend_rejects_oversized_snapshot_and_snapshot_meta() -> TestResult {
    let dir_snap = TempDirGuard::new("oversized-snap")?;
    create_sparse_file(&dir_snap.child(SNAPSHOT_FILE_NAME), (MAX_ARCHIVE_FILE_BYTES as u64).saturating_add(1))?;
    let backend = DirectoryJournalBackend::open_dir(dir_snap.path())?;
    assert!(matches!(backend.read_snapshot(), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_ARCHIVE_FILE_BYTES));
    assert!(
        matches!(CrashRecoveryJournal::open_or_recover(backend, "reader", 60_000, 1_000), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_ARCHIVE_FILE_BYTES)
    );
    assert!(!dir_snap.child(LOCKFILE_NAME).exists());

    let dir_meta = TempDirGuard::new("oversized-snap-meta")?;
    create_sparse_file(&dir_meta.child(SNAPSHOT_META_FILE_NAME), LOCKFILE_CAP_BYTES.saturating_add(1))?;
    assert!(
        matches!(DirectoryJournalBackend::open_dir(dir_meta.path())?.read_snapshot_covered_seq_for(b"snap-bytes"), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == LOCKFILE_CAP_BYTES as usize)
    );

    let dir_wal = TempDirGuard::new("oversized-journal-direct")?;
    create_sparse_file(&dir_wal.child(JOURNAL_FILE_NAME), (MAX_JOURNAL_BYTES as u64).saturating_add(1))?;
    assert!(
        matches!(DirectoryJournalBackend::open_dir(dir_wal.path())?.read_journal_bytes(), Err(FormatError::FileTooLarge { limit_bytes, .. }) if limit_bytes == MAX_JOURNAL_BYTES)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn fa_sec04_03_directory_backend_rejects_symlinks_and_directories() -> TestResult {
    use std::os::unix::fs::symlink;

    let root = TempDirGuard::new("symlinks-dirs")?;
    let outside = root.child("outside_secret.txt");
    fs::write(&outside, b"SECRET_OUTSIDE_CONTENT")?;

    let mk = |sub: &str, name: &str, target: &Path| -> std::io::Result<std::path::PathBuf> {
        let d = root.child(sub);
        fs::create_dir_all(&d)?;
        symlink(target, d.join(name))?;
        Ok(d)
    };

    let jdir_snap = mk("sym_snap", SNAPSHOT_FILE_NAME, &outside)?;
    let mut b_snap = DirectoryJournalBackend::open_dir(&jdir_snap)?;
    assert!(matches!(b_snap.read_snapshot(), Err(FormatError::Io(_))) && matches!(b_snap.write_snapshot_atomic(b"NEW"), Err(FormatError::Io(_))));
    assert!(
        matches!(CrashRecoveryJournal::open_or_recover(b_snap, "w1", 60_000, 1_000), Err(FormatError::Io(_)))
            && !jdir_snap.join(LOCKFILE_NAME).exists()
    );

    for (sub, target) in [("sym_snap_dangling", root.child("does_not_exist.dat")), ("sym_snap_zero", Path::new("/dev/zero").to_path_buf())] {
        if sub == "sym_snap_zero" && !target.exists() {
            continue;
        }
        assert!(
            matches!(DirectoryJournalBackend::open_dir(&mk(sub, SNAPSHOT_FILE_NAME, &target)?)?.read_snapshot(), Err(FormatError::Io(_))),
            "{sub}"
        );
    }

    let jdir_wal = mk("sym_wal", JOURNAL_FILE_NAME, &outside)?;
    let mut b_wal = DirectoryJournalBackend::open_dir(&jdir_wal)?;
    assert!(matches!(b_wal.read_journal_bytes(), Err(FormatError::Io(_))) && matches!(b_wal.append_journal_line("{}"), Err(FormatError::Io(_))));
    assert!(matches!(CrashRecoveryJournal::open_or_recover(b_wal, "w2", 60_000, 1_000), Err(FormatError::Io(_))));

    let jdir_lock = mk("sym_lock", LOCKFILE_NAME, &outside)?;
    let b_lock = DirectoryJournalBackend::open_dir(&jdir_lock)?;
    assert!(
        matches!(b_lock.read_lease(), Err(FormatError::Io(_)))
            && matches!(CrashRecoveryJournal::open_or_recover(b_lock, "w3", 60_000, 1_000), Err(FormatError::Io(_)))
    );

    let jdir_evict = mk("sym_evict", EVICTION_LOCK_FILE_NAME, &outside)?;
    write_stale_lock(&jdir_evict, "stale", 1_000, 500);
    assert!(matches!(open_dir_journal(&jdir_evict, "w4", 60_000, 10_000), Err(FormatError::Io(_))));
    assert_eq!(fs::read(&outside)?, b"SECRET_OUTSIDE_CONTENT");

    let jdir_meta = mk("sym_meta", SNAPSHOT_META_FILE_NAME, &outside)?;
    assert!(matches!(DirectoryJournalBackend::open_dir(&jdir_meta)?.read_snapshot_covered_seq_for(b"snap"), Err(FormatError::Io(_))));

    for name in [SNAPSHOT_FILE_NAME, JOURNAL_FILE_NAME, LOCKFILE_NAME] {
        let jdir = root.child(&format!("dir_{name}"));
        fs::create_dir_all(jdir.join(name))?;
        assert!(matches!(open_dir_journal(&jdir, "w5", 60_000, 1_000), Err(FormatError::Io(_))), "{name}");
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn fa_sec04_03_directory_backend_rejects_fifos_without_blocking() -> TestResult {
    type FifoCheck = fn(&Path) -> Result<(), String>;
    let cases: [(&str, &str, bool, FifoCheck); 5] = [
        ("fifo-lock", LOCKFILE_NAME, false, |d| {
            let b = DirectoryJournalBackend::open_dir(d).map_err(|e| e.to_string())?;
            if !matches!(b.read_lease(), Err(FormatError::Io(_)))
                || !matches!(CrashRecoveryJournal::open_or_recover(b, "w", 60_000, 1_000), Err(FormatError::Io(_)))
            {
                return Err(String::from("lock FIFO accepted"));
            }
            Ok(())
        }),
        ("fifo-evict", EVICTION_LOCK_FILE_NAME, true, |d| {
            if !matches!(open_dir_journal(d, "w", 60_000, 10_000), Err(FormatError::Io(_))) {
                return Err(String::from("evict FIFO accepted"));
            }
            Ok(())
        }),
        ("fifo-wal", JOURNAL_FILE_NAME, false, |d| {
            let mut b = DirectoryJournalBackend::open_dir(d).map_err(|e| e.to_string())?;
            if !matches!(b.read_journal_bytes(), Err(FormatError::Io(_)))
                || !matches!(b.append_journal_line("{}"), Err(FormatError::Io(_)))
                || !matches!(CrashRecoveryJournal::open_or_recover(b, "w", 60_000, 1_000), Err(FormatError::Io(_)))
            {
                return Err(String::from("WAL FIFO accepted"));
            }
            Ok(())
        }),
        ("fifo-snap", SNAPSHOT_FILE_NAME, false, |d| {
            let b = DirectoryJournalBackend::open_dir(d).map_err(|e| e.to_string())?;
            if !matches!(b.read_snapshot(), Err(FormatError::Io(_)))
                || !matches!(CrashRecoveryJournal::open_or_recover(b, "w", 60_000, 1_000), Err(FormatError::Io(_)))
            {
                return Err(String::from("snapshot FIFO accepted"));
            }
            Ok(())
        }),
        ("fifo-snap-meta", SNAPSHOT_META_FILE_NAME, false, |d| {
            let b = DirectoryJournalBackend::open_dir(d).map_err(|e| e.to_string())?;
            if !matches!(b.read_snapshot_covered_seq_for(b"snap"), Err(FormatError::Io(_))) {
                return Err(String::from("meta FIFO accepted"));
            }
            Ok(())
        }),
    ];
    for (label, fname, seed_stale, check) in cases {
        let dir = TempDirGuard::new(label)?;
        if seed_stale {
            write_stale_lock(dir.path(), "stale", 1_000, 500);
        }
        let fifo = dir.child(fname);
        assert!(std::process::Command::new("mkfifo").arg(&fifo).status()?.success());
        let dir_path = dir.path().to_path_buf();
        run_with_fifo_watchdog(&fifo, fname, move || check(&dir_path))?;
    }
    Ok(())
}
