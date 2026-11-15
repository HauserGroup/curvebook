use lumencraft_format::FormatError;
use lumencraft_format::recovery::{
    CURRENT_JOURNAL_VERSION, CrashRecoveryJournal, DirectoryJournalBackend, JOURNAL_FILE_NAME, JournalBackend, JournalLimits, JournalLockLease,
    JournalOpRecord, LOCKFILE_NAME, MAX_JOURNAL_BYTES, MAX_WEB_STORAGE_JSON_BYTES, MemoryJournalBackend,
};
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Result<Self, std::io::Error> {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("lc_audit_s4b_rec_{prefix}_{pid}_{seq}"));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn assert_no_lock_temp_or_evict_leftovers(dir: &Path) -> TestResult {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            !name.ends_with(".tmp") && !name.contains(".tmp.") && !name.ends_with(".evict"),
            "unexpected leftover lock/temp file `{name}` in `{}`",
            dir.display()
        );
    }
    Ok(())
}

#[test]
fn r1_s06_07_concurrent_stale_lock_eviction_has_single_winner() -> TestResult {
    let dir = TempDirGuard::new("stale_race")?;
    let expired_lease = JournalLockLease::new("crashed-worker", 1_000, 500)?;
    std::fs::write(dir.path().join(LOCKFILE_NAME), serde_json::to_vec(&expired_lease)?)?;

    let thread_count = 8usize;
    let barrier = Arc::new(Barrier::new(thread_count));
    let mut handles = Vec::with_capacity(thread_count);

    for idx in 0..thread_count {
        let dir_path = dir.path().to_path_buf();
        let gate = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || -> Result<Option<(String, Option<String>)>, String> {
            let backend = DirectoryJournalBackend::open_dir(dir_path).map_err(|e| e.to_string())?;
            let owner = format!("racer-{idx}");
            gate.wait();
            match CrashRecoveryJournal::open_or_recover(backend, owner.clone(), 60_000, 2_000) {
                Ok((journal, report)) => {
                    let on_disk = journal.backend().read_lease().map_err(|e| e.to_string())?.ok_or_else(|| String::from("missing lease on disk"))?;
                    if on_disk.owner_id != owner {
                        return Err(format!("winner `{owner}` saw `{}` on disk immediately after acquire", on_disk.owner_id));
                    }
                    let evicted_owner = report.recovered_from_stale_lock.map(|lease| lease.owner_id);
                    std::mem::drop(journal);
                    Ok(Some((owner, evicted_owner)))
                }
                Err(_) => Ok(None),
            }
        }));
    }

    let mut winners = Vec::new();
    for handle in handles {
        let outcome = handle.join().map_err(|_| "worker thread panicked during stale-lock race")??;
        if let Some(winner) = outcome {
            winners.push(winner);
        }
    }

    assert_eq!(winners.len(), 1, "expected exactly 1 winner out of {thread_count} racers, got {winners:?}");
    let (winner_id, evicted_owner) = &winners[0];
    assert_eq!(evicted_owner.as_deref(), Some("crashed-worker"), "winner must report evicted stale lease owner");

    let on_disk = DirectoryJournalBackend::open_dir(dir.path())?.read_lease()?.ok_or("winning lease missing on disk")?;
    assert_eq!(&on_disk.owner_id, winner_id);
    assert_no_lock_temp_or_evict_leftovers(dir.path())?;
    Ok(())
}

#[test]
fn r1_s06_07_concurrent_corrupt_lock_reclamation_has_single_winner() -> TestResult {
    let dir = TempDirGuard::new("corrupt_race")?;
    std::fs::write(dir.path().join(LOCKFILE_NAME), b"{corrupt_lock\x00\xff")?;

    let thread_count = 8usize;
    let barrier = Arc::new(Barrier::new(thread_count));
    let mut handles = Vec::with_capacity(thread_count);

    for idx in 0..thread_count {
        let dir_path = dir.path().to_path_buf();
        let gate = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || -> Result<Option<(String, usize)>, String> {
            let backend = DirectoryJournalBackend::open_dir(dir_path).map_err(|e| e.to_string())?;
            let owner = format!("reclaimer-{idx}");
            gate.wait();
            match CrashRecoveryJournal::open_or_recover(backend, owner.clone(), 60_000, 5_000) {
                Ok((_journal, report)) => {
                    if report.recovered_from_stale_lock.is_some() {
                        return Err(String::from("corrupt unparsable lockfile should yield recovered_from_stale_lock == None"));
                    }
                    let corrupt_warnings = report.warnings.iter().filter(|w| w.kind == "corrupt_lockfile").count();
                    Ok(Some((owner, corrupt_warnings)))
                }
                Err(_) => Ok(None),
            }
        }));
    }

    let mut winners = Vec::new();
    for handle in handles {
        let outcome = handle.join().map_err(|_| "worker thread panicked during corrupt-lock race")??;
        if let Some(winner) = outcome {
            winners.push(winner);
        }
    }

    assert_eq!(winners.len(), 1, "expected exactly 1 winner out of {thread_count} reclaimers, got {winners:?}");
    assert_eq!(winners[0].1, 1, "expected exactly 1 corrupt_lockfile warning on the winner");

    let on_disk = DirectoryJournalBackend::open_dir(dir.path())?.read_lease()?.ok_or("winning lease missing on disk")?;
    assert_eq!(on_disk.owner_id, winners[0].0);
    assert_no_lock_temp_or_evict_leftovers(dir.path())?;
    Ok(())
}

#[test]
fn r1_s06_07_renew_and_release_respect_lease_ownership() -> TestResult {
    let dir = TempDirGuard::new("lease_ops")?;
    let backend = DirectoryJournalBackend::open_dir(dir.path())?;
    let (mut journal, _report) = CrashRecoveryJournal::open_or_recover(backend, "owner-a", 1_000, 10_000)?;
    journal.renew_lease(10_800)?;

    let intruder = DirectoryJournalBackend::open_dir(dir.path())?;
    assert!(
        CrashRecoveryJournal::open_or_recover(intruder, "owner-b", 1_000, 11_200).is_err(),
        "renewed lease should block intruder until renewed expiry"
    );

    let mut releaser = DirectoryJournalBackend::open_dir(dir.path())?;
    assert!(releaser.release_lease("owner-b").is_err(), "wrong owner release must be rejected");
    let bogus_lease = JournalLockLease::new("owner-b", 11_500, 1_000)?;
    assert!(releaser.renew_lease(&bogus_lease, 11_500).is_err(), "wrong owner renew must be rejected");
    let still_held = releaser.read_lease()?.ok_or("wrong owner release removed active lease")?;
    assert_eq!(still_held.owner_id, "owner-a");
    assert_eq!(still_held.acquired_epoch_ms, 10_800);

    let _ = journal.close_cleanly(b"done")?;
    assert!(!dir.path().join(LOCKFILE_NAME).exists());
    assert_no_lock_temp_or_evict_leftovers(dir.path())?;
    Ok(())
}

#[test]
fn r1_s06_08_corrupt_and_oversized_lockfiles_are_reclaimed_while_valid_and_future_locks_are_preserved() -> TestResult {
    let corrupt_cases: [(&str, Vec<u8>); 5] = [
        ("invalid_json", b"{not valid json".to_vec()),
        ("empty_file", Vec::new()),
        ("invalid_utf8", vec![0xFF, 0xFE, 0x00, 0x80]),
        ("empty_owner", br#"{"format_version":1,"owner_id":"   ","acquired_epoch_ms":1,"ttl_ms":5000}"#.to_vec()),
        ("oversized_garbage", vec![b' '; 128 * 1024]),
    ];

    for (label, payload) in corrupt_cases {
        let dir = TempDirGuard::new(label)?;
        let prior_rec = JournalOpRecord::new(1, json!({"op": "keep-me"}))?;
        std::fs::write(dir.path().join(JOURNAL_FILE_NAME), format!("{}\n", prior_rec.to_ndjson_line()?))?;
        std::fs::write(dir.path().join(LOCKFILE_NAME), &payload)?;

        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let (journal, report) = CrashRecoveryJournal::open_or_recover(backend, "rescuer", 60_000, 50_000)?;
        assert!(report.recovered_from_stale_lock.is_none(), "case `{label}` has no parsable prior lease");
        assert!(
            report.warnings.iter().any(|w| w.line == 0 && w.kind == "corrupt_lockfile"),
            "case `{label}` missing corrupt_lockfile warning: {:?}",
            report.warnings
        );
        assert_eq!(report.recovered_seqs, vec![1], "case `{label}` should still replay WAL records");
        let on_disk = journal.backend().read_lease()?.ok_or("lockfile missing after reclamation")?;
        assert_eq!(on_disk.owner_id, "rescuer");
        let _ = journal.close_cleanly(b"snap")?;
        assert_no_lock_temp_or_evict_leftovers(dir.path())?;
    }

    // Active valid lock must NOT be reclaimed.
    {
        let dir = TempDirGuard::new("active_valid")?;
        let active = JournalLockLease::new("active-writer", 1_000, 60_000)?;
        std::fs::write(dir.path().join(LOCKFILE_NAME), serde_json::to_vec(&active)?)?;
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = CrashRecoveryJournal::open_or_recover(backend, "challenger", 60_000, 2_000)
            .err()
            .ok_or("expected active valid lock to block open_or_recover")?;
        assert!(matches!(err, FormatError::Io(_)), "expected Io lock contention error, got {err:?}");
        let untouched = DirectoryJournalBackend::open_dir(dir.path())?.read_lease()?.ok_or("active lock vanished")?;
        assert_eq!(untouched.owner_id, "active-writer");
    }

    // Future schema version lockfile must fail with UnsupportedSchemaVersion instead of silent eviction.
    {
        let dir = TempDirGuard::new("future_lock")?;
        let future_ver = CURRENT_JOURNAL_VERSION + 7;
        let future_json = format!(r#"{{"format_version":{future_ver},"future_only":true}}"#);
        std::fs::write(dir.path().join(LOCKFILE_NAME), future_json.as_bytes())?;
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err =
            CrashRecoveryJournal::open_or_recover(backend, "older-client", 60_000, 2_000).err().ok_or("expected future schema lockfile to fail")?;
        assert!(
            matches!(err, FormatError::UnsupportedSchemaVersion { version, .. } if version == future_ver),
            "expected UnsupportedSchemaVersion({future_ver}), got {err:?}"
        );
        assert!(dir.path().join(LOCKFILE_NAME).exists(), "future lockfile must not be deleted");
    }

    // MemoryJournalBackend empty-owner parity.
    {
        let mut mem = MemoryJournalBackend::new();
        let bad_lease_json = r#"{"format_version":1,"lease":{"format_version":1,"owner_id":" ","acquired_epoch_ms":1,"ttl_ms":1000},"snapshot":null,"snapshot_covered_seq":null,"journal_bytes":""}"#;
        assert!(
            matches!(MemoryJournalBackend::from_web_storage_json(bad_lease_json), Err(FormatError::Json(_))),
            "expected empty lease owner in web storage JSON to return FormatError::Json"
        );
        mem.write_journal_record_for_test()?;
        drop(mem);
    }

    Ok(())
}

trait TestMemoryExt {
    fn write_journal_record_for_test(&mut self) -> TestResult;
}

impl TestMemoryExt for MemoryJournalBackend {
    fn write_journal_record_for_test(&mut self) -> TestResult {
        let lease = JournalLockLease::new("seed-owner", 1, 1000)?;
        assert!(self.acquire_lease(&lease, 1)?.is_none());
        let next = JournalLockLease::new("new-owner", 5_000, 1000)?;
        assert!(self.acquire_lease(&next, 5_000)?.is_some());
        Ok(())
    }
}

#[test]
fn r1_s06_09_web_storage_json_enforces_real_serialized_size_and_supports_20mb_snapshot() -> TestResult {
    // 20 MiB snapshot easily round-trips under the 64 MiB serialized JSON budget via compact base64.
    let twenty_mb = vec![0xA5u8; 20 * 1024 * 1024];
    let mem = MemoryJournalBackend::new();
    let (mut journal, _report) = CrashRecoveryJournal::open_or_recover(mem, "web-tab-1", 60_000, 1_000)?;
    let _ = journal.record_command(json!({"cmd": "draw", "stroke": 42}))?;
    journal.checkpoint(&twenty_mb)?;
    let _ = journal.record_command(json!({"cmd": "draw", "stroke": 43}))?;

    let backend = journal.abandon();
    let serialized = backend.to_web_storage_json()?;
    assert!(
        serialized.len() <= MAX_WEB_STORAGE_JSON_BYTES,
        "20 MiB serialized web JSON ({} bytes) exceeded budget ({} bytes)",
        serialized.len(),
        MAX_WEB_STORAGE_JSON_BYTES
    );

    let restored = MemoryJournalBackend::from_web_storage_json(&serialized)?;
    assert_eq!(restored.read_snapshot()?.as_deref(), Some(twenty_mb.as_slice()));

    let (_reopened, report) = CrashRecoveryJournal::open_or_recover(restored, "web-tab-2", 60_000, 120_000)?;
    assert_eq!(report.snapshot.as_deref(), Some(twenty_mb.as_slice()));
    assert_eq!(report.recovered_commands, vec![json!({"cmd": "draw", "stroke": 43})]);

    // Real serialized JSON exceeding 64 MiB must be rejected by to_web_storage_json just like from_web_storage_json.
    let forty_nine_mb = vec![0x5Au8; 49 * 1024 * 1024];
    let mut oversized_backend = MemoryJournalBackend::new();
    oversized_backend.write_snapshot_atomic(&forty_nine_mb)?;
    let to_err = oversized_backend.to_web_storage_json().err().ok_or("expected >64 MiB serialized web-storage JSON to be rejected on export")?;
    assert!(matches!(to_err, FormatError::Io(_)), "expected FormatError::Io, got {to_err:?}");

    let raw_oversized_json = format!(
        r#"{{"format_version":1,"lease":null,"snapshot":[0],"snapshot_covered_seq":null,"journal_bytes":"{}"}}"#,
        "A".repeat(MAX_WEB_STORAGE_JSON_BYTES)
    );
    assert!(raw_oversized_json.len() > MAX_WEB_STORAGE_JSON_BYTES);
    let from_err = MemoryJournalBackend::from_web_storage_json(&raw_oversized_json)
        .err()
        .ok_or("expected >64 MiB JSON string to be rejected by from_web_storage_json")?;
    assert!(matches!(from_err, FormatError::Io(_)), "expected FormatError::Io, got {from_err:?}");

    // Legacy integer-array byte encoding remains readable for backwards compatibility.
    let legacy_json = r#"{"format_version":1,"lease":null,"snapshot":[1,2,3,4],"snapshot_covered_seq":null,"journal_bytes":[]}"#;
    let legacy_backend = MemoryJournalBackend::from_web_storage_json(legacy_json)?;
    assert_eq!(legacy_backend.read_snapshot()?.as_deref(), Some(&[1u8, 2, 3, 4][..]));

    Ok(())
}

#[test]
fn r1_s06_14_bounded_replay_and_append_enforce_bytes_line_and_record_limits() -> TestResult {
    // 1. Default 32 MiB limit rejects a sparse >32 MiB ops.journal without reading/allocating the whole file.
    {
        let dir = TempDirGuard::new("sparse_huge_wal")?;
        let journal_path = dir.path().join(JOURNAL_FILE_NAME);
        let file = File::create(&journal_path)?;
        file.set_len((MAX_JOURNAL_BYTES as u64).saturating_add(1))?;
        drop(file);

        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = CrashRecoveryJournal::open_or_recover(backend, "reader", 60_000, 1_000)
            .err()
            .ok_or("expected sparse >MAX_JOURNAL_BYTES ops.journal to fail")?;
        assert!(matches!(err, FormatError::Io(_)), "expected FormatError::Io, got {err:?}");
        assert!(!dir.path().join(LOCKFILE_NAME).exists(), "failed replay must clean up newly acquired lock");
    }

    // 2. Single oversized line exceeding max_line_bytes is rejected cleanly without allocating beyond limit.
    {
        let dir = TempDirGuard::new("oversized_line")?;
        let big_cmd = json!({"payload": "X".repeat(600)});
        let rec = JournalOpRecord::new(1, big_cmd)?;
        std::fs::write(dir.path().join(JOURNAL_FILE_NAME), format!("{}\n", rec.to_ndjson_line()?))?;

        let limits = JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 256, max_records: 100 };
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = CrashRecoveryJournal::open_or_recover_with_limits(backend, "reader", 60_000, 1_000, limits)
            .err()
            .ok_or("expected line > max_line_bytes to fail")?;
        assert!(matches!(err, FormatError::Io(_)), "expected FormatError::Io, got {err:?}");
        assert!(!dir.path().join(LOCKFILE_NAME).exists());
    }

    // 3. Valid record count exceeding max_records is rejected.
    {
        let dir = TempDirGuard::new("too_many_records")?;
        let mut file = OpenOptions::new().create(true).write(true).truncate(true).open(dir.path().join(JOURNAL_FILE_NAME))?;
        for seq in 1..=6u64 {
            let line = JournalOpRecord::new(seq, json!({"i": seq}))?.to_ndjson_line()?;
            writeln!(file, "{line}")?;
        }
        drop(file);

        let limits = JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 4096, max_records: 5 };
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = CrashRecoveryJournal::open_or_recover_with_limits(backend, "reader", 60_000, 1_000, limits)
            .err()
            .ok_or("expected record count > max_records to fail")?;
        assert!(matches!(err, FormatError::Io(_)), "expected FormatError::Io, got {err:?}");
    }

    // 4. Corrupt non-empty line flood exceeding max_records cannot bypass cap.
    {
        let dir = TempDirGuard::new("corrupt_line_flood")?;
        let flood = "not-json\n".repeat(6);
        std::fs::write(dir.path().join(JOURNAL_FILE_NAME), flood.as_bytes())?;

        let limits = JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 4096, max_records: 5 };
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = CrashRecoveryJournal::open_or_recover_with_limits(backend, "reader", 60_000, 1_000, limits)
            .err()
            .ok_or("expected corrupt lines > max_records to fail")?;
        assert!(matches!(err, FormatError::Io(_)), "expected FormatError::Io, got {err:?}");
    }

    // 5. Blank-line flood within max_bytes is bounded too.
    {
        let dir = TempDirGuard::new("blank_line_flood")?;
        let flood = "\n".repeat(2 * 4 + 1025);
        std::fs::write(dir.path().join(JOURNAL_FILE_NAME), flood.as_bytes())?;

        let limits = JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 4096, max_records: 2 };
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let err = CrashRecoveryJournal::open_or_recover_with_limits(backend, "reader", 60_000, 1_000, limits)
            .err()
            .ok_or("expected blank-line flood to fail")?;
        assert!(matches!(err, FormatError::Io(_)), "expected FormatError::Io, got {err:?}");
    }

    // 6. Live session record_command enforces limits and resets cleanly after checkpoint().
    {
        let dir = TempDirGuard::new("live_limits")?;
        let limits = JournalLimits { max_bytes: 4096, max_line_bytes: 256, max_records: 3 };
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        let (mut journal, _report) = CrashRecoveryJournal::open_or_recover_with_limits(backend, "writer", 60_000, 1_000, limits)?;
        assert_eq!(journal.limits(), limits);

        assert!(journal.record_command(json!({"big": "Z".repeat(300)})).is_err(), "oversized command line must be rejected before append");
        assert_eq!(journal.record_command(json!({"n": 1}))?, 1);
        assert_eq!(journal.record_command(json!({"n": 2}))?, 2);
        assert_eq!(journal.record_command(json!({"n": 3}))?, 3);
        assert!(journal.record_command(json!({"n": 4})).is_err(), "4th command must hit max_records=3");

        journal.checkpoint(b"checkpoint-bytes")?;
        assert_eq!(journal.record_command(json!({"n": 4}))?, 4, "checkpoint clears pending count and allows seq 4");
        let _ = journal.close_cleanly(b"final-snap")?;
    }

    // 7. MemoryJournalBackend enforces bounded replay identically.
    {
        let mut mem = MemoryJournalBackend::new();
        let line1 = JournalOpRecord::new(1, json!({"k": 1}))?.to_ndjson_line()?;
        let line2 = JournalOpRecord::new(2, json!({"k": 2}))?.to_ndjson_line()?;
        mem.replace_journal_bytes_atomic(format!("{line1}\n{line2}\n").as_bytes())?;
        let tight = JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 4096, max_records: 1 };
        let err = CrashRecoveryJournal::open_or_recover_with_limits(mem, "mem-reader", 60_000, 1_000, tight)
            .err()
            .ok_or("expected MemoryJournalBackend bounded replay to fail when records > max_records")?;
        assert!(matches!(err, FormatError::Io(_)), "expected FormatError::Io, got {err:?}");
    }

    Ok(())
}
