//! Regression tests for Wave 1 SEC-04 S1 findings (`FA-SEC04-01`, `FA-SEC04-02`, `FA-SEC04-03`).
#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::sync::mpsc;
#[cfg(unix)]
use std::time::Duration;

use curvebook_doc::{CURRENT_SCHEMA_VERSION, DocError, MAX_IMAGE_ASSET_BYTES, MAX_PROJECT_IMAGE_ASSETS, ProjectDocument, content_addressed_asset_id};
use curvebook_format::recovery::{
    CrashRecoveryJournal, DirectoryJournalBackend, JOURNAL_FILE_NAME, JournalBackend, JournalLockLease, LOCKFILE_NAME, MAX_JOURNAL_BYTES,
    SNAPSHOT_FILE_NAME,
};
use curvebook_format::{FormatError, MAX_ARCHIVE_FILE_BYTES, ZipWriter, import_prism_bytes, ingest_image_asset};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

const EVICTION_LOCK_FILE_NAME: &str = ".curvebook.lock.evict";
const SNAPSHOT_META_FILE_NAME: &str = "snapshot.meta.json";
const LOCKFILE_CAP_BYTES: u64 = 64 * 1024;

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, std::io::Error> {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-w1-sec04-{label}-{}-{seq}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn encode_small_png(w: u32, h: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let img = RgbaImage::from_fn(w, h, |x, y| Rgba([(x * 37) as u8, (y * 53) as u8, 180, 255]));
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), ImageFormat::Png)?;
    Ok(out)
}

fn create_sparse_file(path: &Path, byte_len: u64) -> TestResult {
    let file = File::create(path)?;
    file.set_len(byte_len)?;
    Ok(())
}

fn current_wall_epoch_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn assert_no_evict_or_temp_leftovers(dir: &Path) -> TestResult {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            !name.ends_with(".tmp") && !name.contains(".tmp.") && !name.ends_with(".evict"),
            "unexpected leftover lock/evict/temp file `{name}` in `{}`",
            dir.display()
        );
    }
    Ok(())
}

#[test]
fn fa_sec04_01_prism_zip_rejects_excess_and_duplicate_image_stubs_before_cloning() -> TestResult {
    let png_bytes = encode_small_png(4, 4)?;
    let asset = ingest_image_asset(&png_bytes)?;
    let asset_path = format!("assets/{}.png", asset.asset_id);
    let base_doc = ProjectDocument::new("Embedded Image Stubs")?;

    // 1. 10,000 stubs referencing the same archive image asset must fail with TooManyImageAssets.
    let mut doc_many = serde_json::to_value(&base_doc)?;
    let stub = json!({
        "asset_id": asset.asset_id,
        "media_type": "Png",
        "pixel_size": [4, 4]
    });
    doc_many["image_assets"] = serde_json::Value::Array(vec![stub.clone(); 10_000]);

    let mut zip_many = ZipWriter::new();
    zip_many.add_file("document.json", serde_json::to_vec(&doc_many)?)?;
    zip_many.add_file(&asset_path, png_bytes.clone())?;
    let err_many = import_prism_bytes(&zip_many.finish()?).err().ok_or("expected >MAX_PROJECT_IMAGE_ASSETS stubs to fail")?;
    assert!(
        matches!(err_many, FormatError::Doc(DocError::TooManyImageAssets { count: 10_000, max }) if max == MAX_PROJECT_IMAGE_ASSETS),
        "unexpected error for 10,000 image stubs: {err_many:?}"
    );

    // 2. Boundary count MAX_PROJECT_IMAGE_ASSETS + 1 (257) also fails immediately.
    let mut doc_boundary = serde_json::to_value(&base_doc)?;
    doc_boundary["image_assets"] = serde_json::Value::Array(vec![stub.clone(); MAX_PROJECT_IMAGE_ASSETS + 1]);
    let mut zip_boundary = ZipWriter::new();
    zip_boundary.add_file("document.json", serde_json::to_vec(&doc_boundary)?)?;
    zip_boundary.add_file(&asset_path, png_bytes.clone())?;
    let err_boundary = import_prism_bytes(&zip_boundary.finish()?).err().ok_or("expected 257 stubs to fail")?;
    assert!(
        matches!(
            err_boundary,
            FormatError::Doc(DocError::TooManyImageAssets { count, max })
            if count == MAX_PROJECT_IMAGE_ASSETS + 1 && max == MAX_PROJECT_IMAGE_ASSETS
        ),
        "unexpected error for 257 stubs: {err_boundary:?}"
    );

    // 3. Duplicate asset_id within MAX_PROJECT_IMAGE_ASSETS is rejected with DuplicateEntry.
    let mut doc_dup = serde_json::to_value(&base_doc)?;
    doc_dup["image_assets"] = serde_json::Value::Array(vec![stub.clone(), stub]);
    let mut zip_dup = ZipWriter::new();
    zip_dup.add_file("document.json", serde_json::to_vec(&doc_dup)?)?;
    zip_dup.add_file(&asset_path, png_bytes.clone())?;
    let err_dup = import_prism_bytes(&zip_dup.finish()?).err().ok_or("expected duplicate asset_id to fail")?;
    assert!(
        matches!(err_dup, FormatError::DuplicateEntry { ref path } if path == &asset.asset_id),
        "unexpected error for duplicate asset_id: {err_dup:?}"
    );

    // 4. Positive control: single valid externalized asset hydrates cleanly.
    let mut doc_ok = base_doc;
    doc_ok.upsert_image_asset(asset.clone())?;
    let mut ok_val = serde_json::to_value(&doc_ok)?;
    if let Some(arr) = ok_val.get_mut("image_assets").and_then(serde_json::Value::as_array_mut)
        && let Some(first) = arr.first_mut().and_then(serde_json::Value::as_object_mut)
    {
        first.remove("bytes");
    }
    let mut zip_ok = ZipWriter::new();
    zip_ok.add_file("document.json", serde_json::to_vec(&ok_val)?)?;
    zip_ok.add_file(&asset_path, png_bytes.clone())?;
    let imported = import_prism_bytes(&zip_ok.finish()?)?;
    assert_eq!(imported.document.image_assets.len(), 1);
    assert_eq!(imported.document.image_assets[0].asset_id, asset.asset_id);
    assert_eq!(imported.document.image_assets[0].bytes, png_bytes);
    Ok(())
}

#[test]
fn fa_sec04_01_prism_zip_rejects_oversized_corrupt_and_missing_image_assets_before_cloning() -> TestResult {
    let base_doc = ProjectDocument::new("Embedded Image Asset Bounds")?;

    // 1. Oversized archive asset (> MAX_IMAGE_ASSET_BYTES, while < MAX_ZIP_ENTRY_BYTES) fails with FileTooLarge.
    let oversized_payload = vec![0u8; MAX_IMAGE_ASSET_BYTES + 1];
    let oversized_id = content_addressed_asset_id(&oversized_payload);
    let oversized_path = format!("assets/{oversized_id}.png");
    let mut doc_big = serde_json::to_value(&base_doc)?;
    doc_big["image_assets"] = json!([{
        "asset_id": oversized_id,
        "media_type": "Png",
        "pixel_size": [4, 4]
    }]);
    let mut zip_big = ZipWriter::new();
    zip_big.add_file("document.json", serde_json::to_vec(&doc_big)?)?;
    zip_big.add_file(&oversized_path, oversized_payload)?;
    let err_big = import_prism_bytes(&zip_big.finish()?).err().ok_or("expected >MAX_IMAGE_ASSET_BYTES asset to fail")?;
    assert!(
        matches!(err_big, FormatError::FileTooLarge { limit_bytes, .. } if limit_bytes == MAX_IMAGE_ASSET_BYTES),
        "unexpected error for oversized archive asset: {err_big:?}"
    );

    // 2. Oversized inline base64 payload in document.json fails with FileTooLarge before base64 Vec allocation.
    let over_b64_quads = (MAX_IMAGE_ASSET_BYTES + 1).div_ceil(3);
    let mut doc_inline_big = serde_json::to_value(&base_doc)?;
    doc_inline_big["schema_version"] = json!(CURRENT_SCHEMA_VERSION);
    doc_inline_big["image_assets"] = json!([{
        "asset_id": "img-0123456789abcdef",
        "media_type": "Png",
        "pixel_size": [4, 4],
        "bytes": "A".repeat(over_b64_quads * 4)
    }]);
    let mut zip_inline_big = ZipWriter::new();
    zip_inline_big.add_file("document.json", serde_json::to_vec(&doc_inline_big)?)?;
    let err_inline_big = import_prism_bytes(&zip_inline_big.finish()?).err().ok_or("expected oversized inline base64 to fail")?;
    assert!(
        matches!(err_inline_big, FormatError::FileTooLarge { limit_bytes, .. } if limit_bytes == MAX_IMAGE_ASSET_BYTES),
        "unexpected error for oversized inline base64 asset: {err_inline_big:?}"
    );

    // 3. Digest mismatch / corrupt payload fails before cloning.
    let valid_png = encode_small_png(4, 4)?;
    let other_png = encode_small_png(5, 5)?;
    let valid_id = content_addressed_asset_id(&valid_png);
    let mut doc_tampered = serde_json::to_value(&base_doc)?;
    doc_tampered["image_assets"] = json!([{
        "asset_id": valid_id,
        "media_type": "Png",
        "pixel_size": [4, 4]
    }]);
    let mut zip_tampered = ZipWriter::new();
    zip_tampered.add_file("document.json", serde_json::to_vec(&doc_tampered)?)?;
    zip_tampered.add_file(&format!("assets/{valid_id}.png"), other_png)?;
    let err_tampered = import_prism_bytes(&zip_tampered.finish()?).err().ok_or("expected digest mismatch to fail")?;
    assert!(matches!(err_tampered, FormatError::UnsupportedFormat { .. }), "unexpected error for tampered asset: {err_tampered:?}");

    // 4. Missing asset entry fails with MissingArchiveEntry.
    let mut zip_missing = ZipWriter::new();
    zip_missing.add_file("document.json", serde_json::to_vec(&doc_tampered)?)?;
    let err_missing = import_prism_bytes(&zip_missing.finish()?).err().ok_or("expected missing archive asset to fail")?;
    assert!(matches!(err_missing, FormatError::MissingArchiveEntry { .. }), "unexpected error for missing asset: {err_missing:?}");

    Ok(())
}

#[test]
fn fa_sec04_02_oversized_and_corrupt_evict_locks_are_reclaimed_without_self_restore() -> TestResult {
    // 1. 70 KiB oversized .curvebook.lock.evict alongside stale curvebook.lock is deleted and unblocked.
    {
        let dir = TempDirGuard::new("evict-70k")?;
        let stale = JournalLockLease::new("crashed-owner", 1_000, 500)?;
        fs::write(dir.child(LOCKFILE_NAME), serde_json::to_vec(&stale)?)?;
        fs::write(dir.child(EVICTION_LOCK_FILE_NAME), vec![b'X'; 70_000])?;

        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let (journal, report) = CrashRecoveryJournal::open_or_recover(backend, "rescuer-70k", 60_000, 10_000)?;
        assert_eq!(report.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str()), Some("crashed-owner"));
        assert_eq!(journal.lease().owner_id, "rescuer-70k");
        assert!(!dir.child(EVICTION_LOCK_FILE_NAME).exists(), "oversized evict lock must be removed, not restored");
        let _ = journal.close_cleanly(b"ok")?;
        assert_no_evict_or_temp_leftovers(dir.path())?;
    }

    // 2. Multi-GiB sparse .curvebook.lock.evict is deleted in O(1) without unbounded read or OOM.
    {
        let dir = TempDirGuard::new("evict-4g")?;
        let stale = JournalLockLease::new("crashed-sparse", 1_000, 500)?;
        fs::write(dir.child(LOCKFILE_NAME), serde_json::to_vec(&stale)?)?;
        create_sparse_file(&dir.child(EVICTION_LOCK_FILE_NAME), 4_u64 * 1024 * 1024 * 1024)?;

        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let (journal, report) = CrashRecoveryJournal::open_or_recover(backend, "rescuer-4g", 60_000, 10_000)?;
        assert_eq!(report.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str()), Some("crashed-sparse"));
        assert!(!dir.child(EVICTION_LOCK_FILE_NAME).exists());
        let _ = journal.close_cleanly(b"ok")?;
        assert_no_evict_or_temp_leftovers(dir.path())?;
    }

    // 3. Corrupt small .curvebook.lock.evict alongside stale curvebook.lock is reclaimed cleanly.
    {
        let dir = TempDirGuard::new("evict-corrupt")?;
        let stale = JournalLockLease::new("crashed-corrupt-evict", 1_000, 500)?;
        fs::write(dir.child(LOCKFILE_NAME), serde_json::to_vec(&stale)?)?;
        fs::write(dir.child(EVICTION_LOCK_FILE_NAME), b"{malformed-evict-guard")?;

        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let (journal, report) = CrashRecoveryJournal::open_or_recover(backend, "rescuer-corrupt", 60_000, 10_000)?;
        assert_eq!(report.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str()), Some("crashed-corrupt-evict"));
        assert!(!dir.child(EVICTION_LOCK_FILE_NAME).exists());
        let _ = journal.close_cleanly(b"ok")?;
        assert_no_evict_or_temp_leftovers(dir.path())?;
    }

    // 4. Fresh, non-expired EvictionLockRecord from another live host still blocks concurrent eviction.
    {
        let dir = TempDirGuard::new("evict-live")?;
        let stale = JournalLockLease::new("crashed-wait", 1_000, 500)?;
        fs::write(dir.child(LOCKFILE_NAME), serde_json::to_vec(&stale)?)?;
        let live_guard = json!({
            "owner_id": "remote-active-evictor",
            "acquired_wall_ms": current_wall_epoch_ms().saturating_add(5_000),
            "ttl_ms": 60_000_u64,
            "nonce": 42_u64,
            "host_id": "remote-host-fingerprint-xyz"
        });
        fs::write(dir.child(EVICTION_LOCK_FILE_NAME), serde_json::to_vec(&live_guard)?)?;

        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = CrashRecoveryJournal::open_or_recover(backend, "challenger", 60_000, 10_000)
            .err()
            .ok_or("expected active live eviction guard to block stale lock eviction")?;
        assert!(matches!(err, FormatError::Io(_)), "expected FormatError::Io, got {err:?}");
        assert!(dir.child(EVICTION_LOCK_FILE_NAME).exists(), "live eviction guard must not be deleted");
    }

    Ok(())
}

#[test]
fn fa_sec04_03_directory_backend_rejects_oversized_snapshot_and_snapshot_meta() -> TestResult {
    // 1. Oversized sparse snapshot (> MAX_ARCHIVE_FILE_BYTES) is rejected before reading/allocating.
    {
        let dir = TempDirGuard::new("oversized-snap")?;
        create_sparse_file(&dir.child(SNAPSHOT_FILE_NAME), (MAX_ARCHIVE_FILE_BYTES as u64).saturating_add(1))?;

        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let direct_err = backend.read_snapshot().err().ok_or("expected oversized snapshot read to fail")?;
        assert!(
            matches!(direct_err, FormatError::FileTooLarge { limit_bytes, .. } if limit_bytes == MAX_ARCHIVE_FILE_BYTES),
            "unexpected read_snapshot error: {direct_err:?}"
        );

        let open_err = CrashRecoveryJournal::open_or_recover(backend, "reader", 60_000, 1_000)
            .err()
            .ok_or("expected open_or_recover on oversized snapshot to fail")?;
        assert!(
            matches!(open_err, FormatError::FileTooLarge { limit_bytes, .. } if limit_bytes == MAX_ARCHIVE_FILE_BYTES),
            "unexpected open_or_recover error: {open_err:?}"
        );
        assert!(!dir.child(LOCKFILE_NAME).exists(), "failed recovery must release newly acquired lock");
    }

    // 2. Oversized snapshot.meta.json (> 64 KiB) is rejected before allocation.
    {
        let dir = TempDirGuard::new("oversized-snap-meta")?;
        create_sparse_file(&dir.child(SNAPSHOT_META_FILE_NAME), LOCKFILE_CAP_BYTES.saturating_add(1))?;
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = backend.read_snapshot_covered_seq_for(b"snap-bytes").err().ok_or("expected oversized snapshot.meta.json to fail")?;
        assert!(
            matches!(err, FormatError::FileTooLarge { limit_bytes, .. } if limit_bytes == LOCKFILE_CAP_BYTES as usize),
            "unexpected snapshot_meta error: {err:?}"
        );
    }

    // 3. Oversized ops.journal (> MAX_JOURNAL_BYTES) via direct read_journal_bytes is rejected before allocation.
    {
        let dir = TempDirGuard::new("oversized-journal-direct")?;
        create_sparse_file(&dir.child(JOURNAL_FILE_NAME), (MAX_JOURNAL_BYTES as u64).saturating_add(1))?;
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = backend.read_journal_bytes().err().ok_or("expected oversized ops.journal direct read to fail")?;
        assert!(
            matches!(err, FormatError::FileTooLarge { limit_bytes, .. } if limit_bytes == MAX_JOURNAL_BYTES),
            "unexpected read_journal_bytes error: {err:?}"
        );
    }

    Ok(())
}

#[cfg(unix)]
#[test]
fn fa_sec04_03_directory_backend_rejects_symlinks_and_directories() -> TestResult {
    let root = TempDirGuard::new("symlinks-dirs")?;
    let outside_file = root.child("outside_secret.txt");
    fs::write(&outside_file, b"SECRET_OUTSIDE_CONTENT")?;

    assert_symlink_snapshot_rejected(root.path(), &outside_file)?;
    assert_symlink_journal_and_lock_rejected(root.path(), &outside_file)?;
    assert_journal_subdirectories_rejected(root.path())?;
    Ok(())
}

#[cfg(unix)]
fn assert_symlink_snapshot_rejected(root: &Path, outside_file: &Path) -> TestResult {
    use std::os::unix::fs::symlink;

    let jdir = root.join("sym_snap");
    fs::create_dir_all(&jdir)?;
    symlink(outside_file, jdir.join(SNAPSHOT_FILE_NAME))?;
    let mut backend = DirectoryJournalBackend::open_dir(&jdir)?;
    assert!(matches!(backend.read_snapshot(), Err(FormatError::Io(_))));
    assert!(matches!(backend.write_snapshot_atomic(b"NEW"), Err(FormatError::Io(_))));
    assert_eq!(fs::read(outside_file)?, b"SECRET_OUTSIDE_CONTENT");
    assert!(matches!(CrashRecoveryJournal::open_or_recover(backend, "w1", 60_000, 1_000), Err(FormatError::Io(_))));
    assert!(!jdir.join(LOCKFILE_NAME).exists());

    let jdir_dangling = root.join("sym_snap_dangling");
    fs::create_dir_all(&jdir_dangling)?;
    symlink(root.join("does_not_exist.dat"), jdir_dangling.join(SNAPSHOT_FILE_NAME))?;
    let backend_dangling = DirectoryJournalBackend::open_dir(&jdir_dangling)?;
    assert!(matches!(backend_dangling.read_snapshot(), Err(FormatError::Io(_))));

    if Path::new("/dev/zero").exists() {
        let jdir_zero = root.join("sym_snap_zero");
        fs::create_dir_all(&jdir_zero)?;
        symlink("/dev/zero", jdir_zero.join(SNAPSHOT_FILE_NAME))?;
        let backend_zero = DirectoryJournalBackend::open_dir(&jdir_zero)?;
        assert!(matches!(backend_zero.read_snapshot(), Err(FormatError::Io(_))));
    }
    Ok(())
}

#[cfg(unix)]
fn assert_symlink_journal_and_lock_rejected(root: &Path, outside_file: &Path) -> TestResult {
    use std::os::unix::fs::symlink;

    let jdir_wal = root.join("sym_wal");
    fs::create_dir_all(&jdir_wal)?;
    symlink(outside_file, jdir_wal.join(JOURNAL_FILE_NAME))?;
    let mut backend = DirectoryJournalBackend::open_dir(&jdir_wal)?;
    assert!(matches!(backend.read_journal_bytes(), Err(FormatError::Io(_))));
    assert!(matches!(backend.append_journal_line("{}"), Err(FormatError::Io(_))));
    assert_eq!(fs::read(outside_file)?, b"SECRET_OUTSIDE_CONTENT");
    assert!(matches!(CrashRecoveryJournal::open_or_recover(backend, "w2", 60_000, 1_000), Err(FormatError::Io(_))));

    let jdir_lock = root.join("sym_lock");
    fs::create_dir_all(&jdir_lock)?;
    symlink(outside_file, jdir_lock.join(LOCKFILE_NAME))?;
    let backend_lock = DirectoryJournalBackend::open_dir(&jdir_lock)?;
    assert!(matches!(backend_lock.read_lease(), Err(FormatError::Io(_))));
    assert!(matches!(CrashRecoveryJournal::open_or_recover(backend_lock, "w3", 60_000, 1_000), Err(FormatError::Io(_))));

    let jdir_evict = root.join("sym_evict");
    fs::create_dir_all(&jdir_evict)?;
    let stale = JournalLockLease::new("stale", 1_000, 500)?;
    fs::write(jdir_evict.join(LOCKFILE_NAME), serde_json::to_vec(&stale)?)?;
    symlink(outside_file, jdir_evict.join(EVICTION_LOCK_FILE_NAME))?;
    let backend_evict = DirectoryJournalBackend::open_dir(&jdir_evict)?;
    assert!(matches!(CrashRecoveryJournal::open_or_recover(backend_evict, "w4", 60_000, 10_000), Err(FormatError::Io(_))));
    assert!(outside_file.exists(), "symlink target must never be modified or unlinked");

    let jdir_meta = root.join("sym_meta");
    fs::create_dir_all(&jdir_meta)?;
    symlink(outside_file, jdir_meta.join(SNAPSHOT_META_FILE_NAME))?;
    let backend_meta = DirectoryJournalBackend::open_dir(&jdir_meta)?;
    assert!(matches!(backend_meta.read_snapshot_covered_seq_for(b"snap"), Err(FormatError::Io(_))));
    Ok(())
}

#[cfg(unix)]
fn assert_journal_subdirectories_rejected(root: &Path) -> TestResult {
    for name in [SNAPSHOT_FILE_NAME, JOURNAL_FILE_NAME, LOCKFILE_NAME] {
        let jdir = root.join(format!("dir_{name}"));
        fs::create_dir_all(jdir.join(name))?;
        let backend = DirectoryJournalBackend::open_dir(&jdir)?;
        assert!(matches!(CrashRecoveryJournal::open_or_recover(backend, "w5", 60_000, 1_000), Err(FormatError::Io(_))));
    }
    Ok(())
}

#[cfg(unix)]
fn run_with_fifo_watchdog<F>(fifo_path: &Path, label: &str, op: F) -> TestResult
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(op());
    });
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(msg)) => Err(msg.into()),
        Err(err) => {
            if let Ok(writer) = OpenOptions::new().write(true).open(fifo_path) {
                drop(writer);
            }
            Err(format!("DirectoryJournalBackend hung on FIFO `{label}`: {err}").into())
        }
    }
}

#[cfg(unix)]
#[test]
fn fa_sec04_03_directory_backend_rejects_fifos_without_blocking() -> TestResult {
    // 1. FIFO at curvebook.lock
    {
        let dir = TempDirGuard::new("fifo-lock")?;
        let fifo = dir.child(LOCKFILE_NAME);
        assert!(Command::new("mkfifo").arg(&fifo).status()?.success());
        let dir_path = dir.path().to_path_buf();
        run_with_fifo_watchdog(&fifo, LOCKFILE_NAME, move || {
            let backend = DirectoryJournalBackend::open_dir(&dir_path).map_err(|e| e.to_string())?;
            if !matches!(backend.read_lease(), Err(FormatError::Io(_))) {
                return Err(String::from("read_lease accepted FIFO"));
            }
            if !matches!(CrashRecoveryJournal::open_or_recover(backend, "w", 60_000, 1_000), Err(FormatError::Io(_))) {
                return Err(String::from("open_or_recover accepted FIFO lockfile"));
            }
            Ok(())
        })?;
    }

    // 2. FIFO at .curvebook.lock.evict alongside stale curvebook.lock
    {
        let dir = TempDirGuard::new("fifo-evict")?;
        let stale = JournalLockLease::new("stale", 1_000, 500)?;
        fs::write(dir.child(LOCKFILE_NAME), serde_json::to_vec(&stale)?)?;
        let fifo = dir.child(EVICTION_LOCK_FILE_NAME);
        assert!(Command::new("mkfifo").arg(&fifo).status()?.success());
        let dir_path = dir.path().to_path_buf();
        run_with_fifo_watchdog(&fifo, EVICTION_LOCK_FILE_NAME, move || {
            let backend = DirectoryJournalBackend::open_dir(&dir_path).map_err(|e| e.to_string())?;
            if !matches!(CrashRecoveryJournal::open_or_recover(backend, "w", 60_000, 10_000), Err(FormatError::Io(_))) {
                return Err(String::from("open_or_recover accepted FIFO eviction lock"));
            }
            Ok(())
        })?;
    }

    // 3. FIFO at ops.journal
    {
        let dir = TempDirGuard::new("fifo-wal")?;
        let fifo = dir.child(JOURNAL_FILE_NAME);
        assert!(Command::new("mkfifo").arg(&fifo).status()?.success());
        let dir_path = dir.path().to_path_buf();
        run_with_fifo_watchdog(&fifo, JOURNAL_FILE_NAME, move || {
            let mut backend = DirectoryJournalBackend::open_dir(&dir_path).map_err(|e| e.to_string())?;
            if !matches!(backend.read_journal_bytes(), Err(FormatError::Io(_))) {
                return Err(String::from("read_journal_bytes accepted FIFO"));
            }
            if !matches!(backend.append_journal_line("{}"), Err(FormatError::Io(_))) {
                return Err(String::from("append_journal_line accepted FIFO"));
            }
            if !matches!(CrashRecoveryJournal::open_or_recover(backend, "w", 60_000, 1_000), Err(FormatError::Io(_))) {
                return Err(String::from("open_or_recover accepted FIFO ops.journal"));
            }
            Ok(())
        })?;
    }

    // 4. FIFO at snapshot.curvebook
    {
        let dir = TempDirGuard::new("fifo-snap")?;
        let fifo = dir.child(SNAPSHOT_FILE_NAME);
        assert!(Command::new("mkfifo").arg(&fifo).status()?.success());
        let dir_path = dir.path().to_path_buf();
        run_with_fifo_watchdog(&fifo, SNAPSHOT_FILE_NAME, move || {
            let backend = DirectoryJournalBackend::open_dir(&dir_path).map_err(|e| e.to_string())?;
            if !matches!(backend.read_snapshot(), Err(FormatError::Io(_))) {
                return Err(String::from("read_snapshot accepted FIFO"));
            }
            if !matches!(CrashRecoveryJournal::open_or_recover(backend, "w", 60_000, 1_000), Err(FormatError::Io(_))) {
                return Err(String::from("open_or_recover accepted FIFO snapshot"));
            }
            Ok(())
        })?;
    }

    // 5. FIFO at snapshot.meta.json
    {
        let dir = TempDirGuard::new("fifo-snap-meta")?;
        let fifo = dir.child(SNAPSHOT_META_FILE_NAME);
        assert!(Command::new("mkfifo").arg(&fifo).status()?.success());
        let dir_path = dir.path().to_path_buf();
        run_with_fifo_watchdog(&fifo, SNAPSHOT_META_FILE_NAME, move || {
            let backend = DirectoryJournalBackend::open_dir(&dir_path).map_err(|e| e.to_string())?;
            if !matches!(backend.read_snapshot_covered_seq_for(b"snap"), Err(FormatError::Io(_))) {
                return Err(String::from("read_snapshot_covered_seq_for accepted FIFO"));
            }
            Ok(())
        })?;
    }

    Ok(())
}
