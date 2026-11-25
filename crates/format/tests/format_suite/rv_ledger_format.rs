use std::fs;

use crate::common::{RawZipEntry, TempDirGuard, build_raw_zip, create_sparse_file, open_dir_journal, sample_column_project};
use curvebook_doc::CURRENT_SCHEMA_VERSION;
use curvebook_format::recovery::{is_dead_same_host_owner, is_lease_reclaimable};
use curvebook_format::{
    CrashRecoveryJournal, DirectoryJournalBackend, FormatError, JOURNAL_FILE_NAME, JournalBackend, JournalLockLease, LOCKFILE_NAME,
    MAX_ARCHIVE_FILE_BYTES, MAX_ZIP_ENTRIES, ZipLimits, ZipWriter, save_project_bytes, save_project_file, save_template_file,
};
use curvebook_platform::local_host_fingerprint;

fn synthesize_zip64_sentinel_bytes() -> Vec<u8> {
    let mut bytes =
        Vec::from(b"PK\x03\x04\x2d\x00\x00\x00\x00\x00\x00\x00\x21\x00\x00\x00\x00\x00\xff\xff\xff\xff\xff\xff\xff\xff\x0d\x00\x00\x00manifest.json");
    bytes.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_u16.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_u16.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes
}

#[test]
fn rv_07_refuse_newer_never_clobbers_zip64_oversize_or_unparseable_existing_archives() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("rv07")?;
    let doc = sample_column_project("Current Release Doc");

    // 1. Overwriting a 0-byte empty placeholder file remains allowed.
    let (empty_proj, empty_tpl) = (dir.child("empty.curvebook"), dir.child("empty.cbtemplate"));
    fs::write(&empty_proj, b"")?;
    fs::write(&empty_tpl, b"")?;
    save_project_file(&empty_proj, &doc)?;
    save_template_file(&empty_tpl, &doc)?;

    // 2. ZIP64 sentinel archive from a newer release must NOT be clobbered.
    let zip64_bytes = synthesize_zip64_sentinel_bytes();
    let (zip64_proj, zip64_tpl) = (dir.child("newer_zip64.curvebook"), dir.child("newer_zip64.cbtemplate"));
    fs::write(&zip64_proj, &zip64_bytes)?;
    fs::write(&zip64_tpl, &zip64_bytes)?;
    assert!(save_project_file(&zip64_proj, &doc).is_err() && save_template_file(&zip64_tpl, &doc).is_err());
    assert_eq!((fs::read(&zip64_proj)?, fs::read(&zip64_tpl)?), (zip64_bytes.clone(), zip64_bytes));

    // 3. Future compression codec (Zstd = 93) must NOT be clobbered.
    let mut zstd_entry = RawZipEntry::stored(b"manifest.json", br#"{"format":"curvebook-archive","schema_version":99}"#);
    zstd_entry.method = 93;
    let future_codec = build_raw_zip(&[zstd_entry], b"");
    let codec_proj = dir.child("future_zstd.curvebook");
    fs::write(&codec_proj, &future_codec)?;
    assert!(matches!(save_project_file(&codec_proj, &doc), Err(FormatError::UnsupportedCompression { method: 93, .. })));
    assert_eq!(fs::read(&codec_proj)?, future_codec);

    // 4. Archive exceeding MAX_ZIP_ENTRIES with schema_version = CURRENT + 3 reports UnsupportedSchemaVersion & stays intact.
    let future_ver = CURRENT_SCHEMA_VERSION + 3;
    let wide_limits = ZipLimits {
        max_entries: MAX_ZIP_ENTRIES + 8,
        max_total_uncompressed_bytes: 64 * 1024 * 1024,
        max_entry_uncompressed_bytes: 16 * 1024 * 1024,
    };
    let mut writer = ZipWriter::with_limits(wide_limits);
    writer.add_file("manifest.json", format!(r#"{{"format":"curvebook-archive","schema_version":{future_ver}}}"#).into_bytes())?;
    for idx in 0..=MAX_ZIP_ENTRIES {
        writer.add_file(&format!("sheets/s_{idx}.csv"), b"1\n".to_vec())?;
    }
    let many_bytes = writer.finish()?;
    let many_proj = dir.child("many_entries_vnext.curvebook");
    fs::write(&many_proj, &many_bytes)?;
    assert!(matches!(save_project_file(&many_proj, &doc), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver));
    assert_eq!(fs::read(&many_proj)?, many_bytes);

    // 5. Oversize (> MAX_ARCHIVE_FILE_BYTES) sparse file on disk must NOT be clobbered.
    let huge_proj = dir.child("huge_future.curvebook");
    let huge_len = (MAX_ARCHIVE_FILE_BYTES as u64) + 1024;
    create_sparse_file(&huge_proj, huge_len)?;
    assert!(matches!(save_project_file(&huge_proj, &doc), Err(FormatError::FileTooLarge { .. })));
    assert_eq!(fs::metadata(&huge_proj)?.len(), huge_len);
    Ok(())
}

struct OrderTrackingBackend {
    inner: DirectoryJournalBackend,
    events: Vec<&'static str>,
    fail_snapshot_sync: bool,
}

impl JournalBackend for OrderTrackingBackend {
    fn acquire_lease(&mut self, l: &JournalLockLease, t: u64) -> Result<Option<JournalLockLease>, FormatError> {
        self.inner.acquire_lease(l, t)
    }
    fn renew_lease(&mut self, l: &JournalLockLease, t: u64) -> Result<(), FormatError> {
        self.inner.renew_lease(l, t)
    }
    fn release_lease(&mut self, o: &str) -> Result<(), FormatError> {
        self.events.push("release_lease");
        self.inner.release_lease(o)
    }
    fn read_lease(&self) -> Result<Option<JournalLockLease>, FormatError> {
        self.inner.read_lease()
    }
    fn write_snapshot_atomic(&mut self, b: &[u8]) -> Result<(), FormatError> {
        self.events.push("write_snapshot_atomic");
        self.inner.write_snapshot_atomic(b)
    }
    fn write_snapshot_with_seq(&mut self, b: &[u8], seq: u64) -> Result<(), FormatError> {
        self.events.push("write_snapshot_and_sync_dir");
        if self.fail_snapshot_sync {
            return Err(FormatError::Io(String::from("simulated EIO syncing parent directory")));
        }
        self.inner.write_snapshot_with_seq(b, seq)
    }
    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
        self.inner.read_snapshot()
    }
    fn read_snapshot_covered_seq(&self) -> Result<Option<u64>, FormatError> {
        self.inner.read_snapshot_covered_seq()
    }
    fn clear_snapshot_covered_seq(&mut self) -> Result<(), FormatError> {
        self.events.push("clear_snapshot_covered_seq");
        self.inner.clear_snapshot_covered_seq()
    }
    fn append_journal_line(&mut self, s: &str) -> Result<(), FormatError> {
        self.events.push("append_journal_line");
        self.inner.append_journal_line(s)
    }
    fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError> {
        self.inner.read_journal_bytes()
    }
    fn replace_journal_bytes_atomic(&mut self, b: &[u8]) -> Result<(), FormatError> {
        self.inner.replace_journal_bytes_atomic(b)
    }
    fn clear_journal(&mut self) -> Result<(), FormatError> {
        self.events.push("clear_journal");
        self.inner.clear_journal()
    }
}

#[test]
fn rv_08_checkpoint_syncs_parent_dir_before_clear_journal_and_aborts_on_sync_failure() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("rv08")?;
    let snap_bytes = save_project_bytes(&sample_column_project("Checkpoint Durability"))?;

    let backend = OrderTrackingBackend { inner: DirectoryJournalBackend::open_dir(dir.path())?, events: Vec::new(), fail_snapshot_sync: true };
    let (mut journal, _) = CrashRecoveryJournal::open_or_recover(backend, "rv08-writer", 30_000, 1_000)?;
    journal.record_command(serde_json::json!({ "command_id": "project.new", "params": { "title": "After" } }))?;
    journal.backend_mut().events.clear();

    assert!(journal.checkpoint(&snap_bytes).unwrap_err().to_string().contains("simulated EIO"));
    assert_eq!(journal.backend().events, vec!["write_snapshot_and_sync_dir"]);
    assert!(dir.child(JOURNAL_FILE_NAME).exists());

    journal.backend_mut().fail_snapshot_sync = false;
    journal.backend_mut().events.clear();
    journal.checkpoint(&snap_bytes)?;
    assert_eq!(journal.backend().events, vec!["write_snapshot_and_sync_dir", "clear_journal"]);
    assert!(!dir.child(JOURNAL_FILE_NAME).exists());

    let _ = journal.close_cleanly(&snap_bytes)?;
    assert!(!dir.child(LOCKFILE_NAME).exists());
    Ok(())
}

#[test]
fn rv_09_lease_pid_reclaim_requires_matching_host_fingerprint() -> Result<(), Box<dyn std::error::Error>> {
    let local_fp = local_host_fingerprint().ok_or("expected native host fingerprint")?;
    let dead_pid_owner = "curvebook-pid-999999999";

    let remote_lease = JournalLockLease::new(dead_pid_owner, 10_000, 60_000)?.with_host_id(Some(String::from("linux:x86_64:other-shared-nfs-host")));
    let legacy_lease = JournalLockLease::new(dead_pid_owner, 10_000, 60_000)?.with_host_id(None);
    for (label, lease) in [("remote", &remote_lease), ("legacy", &legacy_lease)] {
        assert!(!is_dead_same_host_owner(&lease.owner_id, lease.host_id.as_deref()), "{label}");
        assert!(!is_lease_reclaimable(lease, 20_000), "{label} must not reclaim before TTL");
        assert!(is_lease_reclaimable(lease, 70_000), "{label} must reclaim after TTL");
    }

    let dir = TempDirGuard::new("rv09")?;
    let _ = DirectoryJournalBackend::open_dir(dir.path())?;
    fs::write(dir.child(LOCKFILE_NAME), serde_json::to_vec(&remote_lease)?)?;
    assert!(open_dir_journal(dir.path(), "local-writer", 30_000, 20_000).unwrap_err().to_string().contains("actively held"));

    let (reclaimed, ttl_report) = open_dir_journal(dir.path(), "local-writer", 30_000, 70_001)?;
    assert_eq!((ttl_report.recovered_from_stale_lock, reclaimed.lease().host_id.as_deref()), (Some(remote_lease), Some(local_fp.as_str())));
    let _ = reclaimed.close_cleanly(b"SNAP")?;

    #[cfg(unix)]
    {
        let same_host_dead = JournalLockLease::new(dead_pid_owner, 10_000, 60_000)?;
        assert_eq!(same_host_dead.host_id.as_deref(), Some(local_fp.as_str()));
        assert!(is_dead_same_host_owner(&same_host_dead.owner_id, same_host_dead.host_id.as_deref()));
        assert!(is_lease_reclaimable(&same_host_dead, 20_000));
    }
    Ok(())
}
