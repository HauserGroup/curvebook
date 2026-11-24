use std::fs;

use crate::common::TempDirGuard;
use curvebook_doc::{CURRENT_SCHEMA_VERSION, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, TableArchetype};
use curvebook_format::recovery::{is_dead_same_host_owner, is_lease_reclaimable};
use curvebook_format::{
    CrashRecoveryJournal, DirectoryJournalBackend, FormatError, JOURNAL_FILE_NAME, JournalBackend, JournalLockLease, LOCKFILE_NAME,
    MAX_ARCHIVE_FILE_BYTES, MAX_ZIP_ENTRIES, ZipWriter, save_project_bytes, save_project_file, save_template_file,
};
use curvebook_platform::local_host_fingerprint;

fn sample_doc(title: &str) -> Result<ProjectDocument, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new(title)?;
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    doc.add_data_sheet(DataSheet::new("data-1", "Cohort A", TableArchetype::Column(col))?)?;
    Ok(doc)
}

/// Synthesize a minimal ZIP64-sentinel archive (EOCD `total_entries = 0xFFFF` / `cd_offset = 0xFFFFFFFF`)
/// mimicking a >4 GiB or >65535-entry archive produced by a newer CurveBook release.
fn synthesize_zip64_sentinel_bytes() -> Vec<u8> {
    let mut bytes =
        Vec::from(b"PK\x03\x04\x2d\x00\x00\x00\x00\x00\x00\x00\x21\x00\x00\x00\x00\x00\xff\xff\xff\xff\xff\xff\xff\xff\x0d\x00\x00\x00manifest.json");
    // ZIP64 EOCD locator + EOCD with 0xFFFF / 0xFFFFFFFF sentinels
    bytes.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_u16.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_u16.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    bytes.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes
}

/// Synthesize a single-entry ZIP archive declaring an unsupported future compression method (e.g. Zstd = 93).
fn synthesize_future_compression_zip_bytes() -> Vec<u8> {
    let name = b"manifest.json";
    let payload = br#"{"format":"curvebook-archive","schema_version":99}"#;
    let crc = crc32fast::hash(payload);
    let method: u16 = 93;
    let mut out = Vec::new();
    // Local file header
    out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
    out.extend_from_slice(&20_u16.to_le_bytes());
    out.extend_from_slice(&0x0800_u16.to_le_bytes());
    out.extend_from_slice(&method.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0x0021_u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(payload);

    let cd_offset = out.len() as u32;
    out.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0x0314_u16.to_le_bytes());
    out.extend_from_slice(&20_u16.to_le_bytes());
    out.extend_from_slice(&0x0800_u16.to_le_bytes());
    out.extend_from_slice(&method.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0x0021_u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(name);
    let cd_size = (out.len() as u32) - cd_offset;

    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out
}

#[test]
fn rv_07_refuse_newer_never_clobbers_zip64_oversize_or_unparseable_existing_archives() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("rv07")?;
    let doc = sample_doc("Current Release Doc")?;

    // 1. Overwriting a 0-byte empty placeholder file remains allowed.
    let empty_proj = dir.path().join("empty.curvebook");
    let empty_tpl = dir.path().join("empty.cbtemplate");
    fs::write(&empty_proj, b"")?;
    fs::write(&empty_tpl, b"")?;
    save_project_file(&empty_proj, &doc)?;
    save_template_file(&empty_tpl, &doc)?;

    // 2. ZIP64 sentinel archive from a newer release must NOT be clobbered.
    let zip64_bytes = synthesize_zip64_sentinel_bytes();
    let zip64_proj = dir.path().join("newer_zip64.curvebook");
    let zip64_tpl = dir.path().join("newer_zip64.cbtemplate");
    fs::write(&zip64_proj, &zip64_bytes)?;
    fs::write(&zip64_tpl, &zip64_bytes)?;
    assert!(save_project_file(&zip64_proj, &doc).is_err(), "must refuse to overwrite ZIP64 project archive");
    assert!(save_template_file(&zip64_tpl, &doc).is_err(), "must refuse to overwrite ZIP64 template archive");
    assert_eq!(fs::read(&zip64_proj)?, zip64_bytes);
    assert_eq!(fs::read(&zip64_tpl)?, zip64_bytes);

    // 3. Future compression codec archive must NOT be clobbered.
    let future_codec = synthesize_future_compression_zip_bytes();
    let codec_proj = dir.path().join("future_zstd.curvebook");
    fs::write(&codec_proj, &future_codec)?;
    assert!(matches!(save_project_file(&codec_proj, &doc), Err(FormatError::UnsupportedCompression { method: 93, .. })));
    assert_eq!(fs::read(&codec_proj)?, future_codec);

    // 4. Archive exceeding MAX_ZIP_ENTRIES whose manifest.json declares schema_version = CURRENT + 3
    //    reports UnsupportedSchemaVersion directly and leaves the file intact.
    let future_ver = CURRENT_SCHEMA_VERSION + 3;
    let wide_limits = curvebook_format::ZipLimits {
        max_entries: MAX_ZIP_ENTRIES + 8,
        max_total_uncompressed_bytes: 64 * 1024 * 1024,
        max_entry_uncompressed_bytes: 16 * 1024 * 1024,
    };
    let mut writer = ZipWriter::with_limits(wide_limits);
    writer.add_file("manifest.json", format!(r#"{{"format":"curvebook-archive","schema_version":{future_ver}}}"#).into_bytes())?;
    for idx in 0..=MAX_ZIP_ENTRIES {
        writer.add_file(&format!("sheets/s_{idx}.csv"), b"1\n".to_vec())?;
    }
    let many_entries_bytes = writer.finish()?;
    let many_proj = dir.path().join("many_entries_vnext.curvebook");
    fs::write(&many_proj, &many_entries_bytes)?;
    assert!(
        matches!(save_project_file(&many_proj, &doc), Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver),
        "expected UnsupportedSchemaVersion on high-entry-count future archive"
    );
    assert_eq!(fs::read(&many_proj)?, many_entries_bytes);

    // 5. Oversize (> MAX_ARCHIVE_FILE_BYTES) sparse file on disk must NOT be clobbered.
    let huge_proj = dir.path().join("huge_future.curvebook");
    let f = fs::File::create(&huge_proj)?;
    f.set_len((MAX_ARCHIVE_FILE_BYTES as u64) + 1024)?;
    drop(f);
    assert!(matches!(save_project_file(&huge_proj, &doc), Err(FormatError::FileTooLarge { .. })));
    assert_eq!(fs::metadata(&huge_proj)?.len(), (MAX_ARCHIVE_FILE_BYTES as u64) + 1024);

    Ok(())
}

struct OrderTrackingBackend {
    inner: DirectoryJournalBackend,
    events: Vec<&'static str>,
    fail_snapshot_sync: bool,
}

impl JournalBackend for OrderTrackingBackend {
    fn acquire_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<Option<JournalLockLease>, FormatError> {
        self.inner.acquire_lease(lease, now_epoch_ms)
    }

    fn renew_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<(), FormatError> {
        self.inner.renew_lease(lease, now_epoch_ms)
    }

    fn release_lease(&mut self, owner_id: &str) -> Result<(), FormatError> {
        self.events.push("release_lease");
        self.inner.release_lease(owner_id)
    }

    fn read_lease(&self) -> Result<Option<JournalLockLease>, FormatError> {
        self.inner.read_lease()
    }

    fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.events.push("write_snapshot_atomic");
        self.inner.write_snapshot_atomic(bytes)
    }

    fn write_snapshot_with_seq(&mut self, bytes: &[u8], covered_seq: u64) -> Result<(), FormatError> {
        self.events.push("write_snapshot_and_sync_dir");
        if self.fail_snapshot_sync {
            return Err(FormatError::Io(String::from("simulated EIO syncing parent directory")));
        }
        self.inner.write_snapshot_with_seq(bytes, covered_seq)
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

    fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError> {
        self.events.push("append_journal_line");
        self.inner.append_journal_line(ndjson_line)
    }

    fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError> {
        self.inner.read_journal_bytes()
    }

    fn replace_journal_bytes_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.inner.replace_journal_bytes_atomic(bytes)
    }

    fn clear_journal(&mut self) -> Result<(), FormatError> {
        self.events.push("clear_journal");
        self.inner.clear_journal()
    }
}

#[test]
fn rv_08_checkpoint_syncs_parent_dir_before_clear_journal_and_aborts_on_sync_failure() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("rv08")?;
    let snap_bytes = save_project_bytes(&sample_doc("Checkpoint Durability")?)?;

    let backend = OrderTrackingBackend { inner: DirectoryJournalBackend::open_dir(dir.path())?, events: Vec::new(), fail_snapshot_sync: true };
    let (mut journal, _) = CrashRecoveryJournal::open_or_recover(backend, "rv08-writer", 30_000, 1_000)?;
    journal.record_command(serde_json::json!({ "command_id": "project.new", "params": { "title": "After" } }))?;
    journal.backend_mut().events.clear();

    // Simulated directory sync failure during snapshot persistence must abort BEFORE clear_journal.
    let err = journal.checkpoint(&snap_bytes).expect_err("snapshot/dir sync failure must propagate");
    assert!(err.to_string().contains("simulated EIO"));
    assert_eq!(journal.backend().events, vec!["write_snapshot_and_sync_dir"]);
    assert!(dir.path().join(JOURNAL_FILE_NAME).exists(), "ops.journal must remain intact when snapshot dir sync fails");

    // Once storage recovers, write_snapshot_and_sync_dir runs strictly BEFORE clear_journal.
    journal.backend_mut().fail_snapshot_sync = false;
    journal.backend_mut().events.clear();
    journal.checkpoint(&snap_bytes)?;
    assert_eq!(journal.backend().events, vec!["write_snapshot_and_sync_dir", "clear_journal"]);
    assert!(!dir.path().join(JOURNAL_FILE_NAME).exists());

    let _ = journal.close_cleanly(&snap_bytes)?;
    assert!(!dir.path().join(LOCKFILE_NAME).exists());
    Ok(())
}

#[test]
fn rv_09_lease_pid_reclaim_requires_matching_host_fingerprint() -> Result<(), Box<dyn std::error::Error>> {
    let local_fp = local_host_fingerprint().ok_or("expected native host fingerprint")?;
    let dead_pid_owner = "curvebook-pid-999999999";

    // 1. Remote host lease with dead-locally PID and active TTL MUST NOT be treated as reclaimable.
    let remote_lease = JournalLockLease::new(dead_pid_owner, 10_000, 60_000)?.with_host_id(Some(String::from("linux:x86_64:other-shared-nfs-host")));
    assert!(!is_dead_same_host_owner(&remote_lease.owner_id, remote_lease.host_id.as_deref()));
    assert!(!is_lease_reclaimable(&remote_lease, 20_000), "remote-host lease must not be stolen before TTL");
    assert!(is_lease_reclaimable(&remote_lease, 70_000), "remote-host lease reclaims once TTL expires");

    // 2. Unknown-host / legacy lockfile (`host_id: None`) MUST rely on TTL expiry, never local PID probe.
    let legacy_lease = JournalLockLease::new(dead_pid_owner, 10_000, 60_000)?.with_host_id(None);
    assert!(!is_dead_same_host_owner(&legacy_lease.owner_id, legacy_lease.host_id.as_deref()));
    assert!(!is_lease_reclaimable(&legacy_lease, 20_000), "unknown-host lease must rely on TTL");
    assert!(is_lease_reclaimable(&legacy_lease, 70_000));

    // Verify end-to-end on DirectoryJournalBackend: remote lockfile blocks until TTL, then recovers.
    let dir = TempDirGuard::new("rv09")?;
    let _ = DirectoryJournalBackend::open_dir(dir.path())?;
    fs::write(dir.path().join(LOCKFILE_NAME), serde_json::to_vec(&remote_lease)?)?;

    let blocked = CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(dir.path())?, "local-writer", 30_000, 20_000)
        .expect_err("remote active lease must block local writer before TTL");
    assert!(blocked.to_string().contains("actively held"));

    let (reclaimed_after_ttl, ttl_report) =
        CrashRecoveryJournal::open_or_recover(DirectoryJournalBackend::open_dir(dir.path())?, "local-writer", 30_000, 70_001)?;
    assert_eq!(ttl_report.recovered_from_stale_lock, Some(remote_lease));
    assert_eq!(reclaimed_after_ttl.lease().host_id.as_deref(), Some(local_fp.as_str()));
    let _ = reclaimed_after_ttl.close_cleanly(b"SNAP")?;

    // 3. Same-host lease with dead PID IS reclaimed immediately before TTL expires.
    #[cfg(unix)]
    {
        let same_host_dead = JournalLockLease::new(dead_pid_owner, 10_000, 60_000)?;
        assert_eq!(same_host_dead.host_id.as_deref(), Some(local_fp.as_str()));
        assert!(is_dead_same_host_owner(&same_host_dead.owner_id, same_host_dead.host_id.as_deref()));
        assert!(is_lease_reclaimable(&same_host_dead, 20_000));
    }

    Ok(())
}
