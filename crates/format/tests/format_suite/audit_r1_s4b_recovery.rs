use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Barrier};

use crate::common::{TempDirGuard, assert_no_lock_temp_or_evict_leftovers, create_sparse_file, open_dir_journal, write_stale_lock};
use curvebook_format::FormatError;
use curvebook_format::recovery::{
    CURRENT_JOURNAL_VERSION, CrashRecoveryJournal, DirectoryJournalBackend, JOURNAL_FILE_NAME, JournalBackend, JournalLimits, JournalLockLease,
    JournalOpRecord, LOCKFILE_NAME, MAX_JOURNAL_BYTES, MAX_WEB_STORAGE_JSON_BYTES, MemoryJournalBackend, RecoveryReport,
};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn run_lock_race<T, F>(dir: &std::path::Path, prefix: &'static str, now_ms: u64, on_win: F) -> Result<Vec<(String, T)>, Box<dyn std::error::Error>>
where
    T: Send + 'static,
    F: Fn(&CrashRecoveryJournal<DirectoryJournalBackend>, RecoveryReport, &str) -> Result<T, String> + Send + Sync + 'static,
{
    let thread_count = 8usize;
    let barrier = Arc::new(Barrier::new(thread_count));
    let on_win = Arc::new(on_win);
    let mut handles = Vec::with_capacity(thread_count);
    for idx in 0..thread_count {
        let dir_path = dir.to_path_buf();
        let gate = Arc::clone(&barrier);
        let check = Arc::clone(&on_win);
        handles.push(std::thread::spawn(move || -> Result<Option<(String, T)>, String> {
            let backend = DirectoryJournalBackend::open_dir(dir_path).map_err(|e| e.to_string())?;
            let owner = format!("{prefix}-{idx}");
            gate.wait();
            match CrashRecoveryJournal::open_or_recover(backend, owner.clone(), 60_000, now_ms) {
                Ok((journal, report)) => {
                    let val = check(&journal, report, &owner)?;
                    drop(journal);
                    Ok(Some((owner, val)))
                }
                Err(_) => Ok(None),
            }
        }));
    }
    let mut winners = Vec::new();
    for handle in handles {
        if let Some(winner) = handle.join().map_err(|_| "worker thread panicked during lock race")?? {
            winners.push(winner);
        }
    }
    Ok(winners)
}

#[test]
fn r1_s06_07_concurrent_stale_lock_eviction_has_single_winner() -> TestResult {
    let dir = TempDirGuard::new("stale_race")?;
    write_stale_lock(dir.path(), "crashed-worker", 1_000, 500);

    let winners = run_lock_race(dir.path(), "racer", 2_000, |journal, report, owner| {
        let on_disk = journal.backend().read_lease().map_err(|e| e.to_string())?.ok_or_else(|| String::from("missing lease on disk"))?;
        if on_disk.owner_id != owner {
            return Err(format!("winner `{owner}` saw `{}` on disk immediately after acquire", on_disk.owner_id));
        }
        Ok(report.recovered_from_stale_lock.map(|lease| lease.owner_id))
    })?;

    assert_eq!(winners.len(), 1, "expected exactly 1 winner, got {winners:?}");
    assert_eq!(winners[0].1.as_deref(), Some("crashed-worker"));
    let on_disk = DirectoryJournalBackend::open_dir(dir.path())?.read_lease()?.ok_or("winning lease missing on disk")?;
    assert_eq!(on_disk.owner_id, winners[0].0);
    assert_no_lock_temp_or_evict_leftovers(dir.path());
    Ok(())
}

#[test]
fn r1_s06_07_concurrent_corrupt_lock_reclamation_has_single_winner() -> TestResult {
    let dir = TempDirGuard::new("corrupt_race")?;
    fs::write(dir.child(LOCKFILE_NAME), b"{corrupt_lock\x00\xff")?;

    let winners = run_lock_race(dir.path(), "reclaimer", 5_000, |_journal, report, _owner| {
        if report.recovered_from_stale_lock.is_some() {
            return Err(String::from("corrupt unparsable lockfile should yield recovered_from_stale_lock == None"));
        }
        Ok(report.warnings.iter().filter(|w| w.kind == "corrupt_lockfile").count())
    })?;

    assert_eq!(winners.len(), 1, "expected exactly 1 winner, got {winners:?}");
    assert_eq!(winners[0].1, 1, "expected exactly 1 corrupt_lockfile warning on the winner");
    let on_disk = DirectoryJournalBackend::open_dir(dir.path())?.read_lease()?.ok_or("winning lease missing on disk")?;
    assert_eq!(on_disk.owner_id, winners[0].0);
    assert_no_lock_temp_or_evict_leftovers(dir.path());
    Ok(())
}

#[test]
fn r1_s06_07_renew_and_release_respect_lease_ownership() -> TestResult {
    let dir = TempDirGuard::new("lease_ops")?;
    let (mut journal, _) = open_dir_journal(dir.path(), "owner-a", 1_000, 10_000)?;
    journal.renew_lease(10_800)?;

    assert!(open_dir_journal(dir.path(), "owner-b", 1_000, 11_200).is_err(), "renewed lease should block intruder until renewed expiry");

    let mut releaser = DirectoryJournalBackend::open_dir(dir.path())?;
    assert!(releaser.release_lease("owner-b").is_err(), "wrong owner release must be rejected");
    let bogus_lease = JournalLockLease::new("owner-b", 11_500, 1_000)?;
    assert!(releaser.renew_lease(&bogus_lease, 11_500).is_err(), "wrong owner renew must be rejected");
    let still_held = releaser.read_lease()?.ok_or("wrong owner release removed active lease")?;
    assert_eq!((still_held.owner_id.as_str(), still_held.acquired_epoch_ms), ("owner-a", 10_800));

    let _ = journal.close_cleanly(b"done")?;
    assert!(!dir.child(LOCKFILE_NAME).exists());
    assert_no_lock_temp_or_evict_leftovers(dir.path());
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
        fs::write(dir.child(JOURNAL_FILE_NAME), format!("{}\n", prior_rec.to_ndjson_line()?))?;
        fs::write(dir.child(LOCKFILE_NAME), &payload)?;

        let (journal, report) = open_dir_journal(dir.path(), "rescuer", 60_000, 50_000)?;
        assert!(report.recovered_from_stale_lock.is_none(), "{label}: expected None prior lease");
        assert!(report.warnings.iter().any(|w| w.line == 0 && w.kind == "corrupt_lockfile"), "{label}: missing warning");
        assert_eq!(report.recovered_seqs, vec![1], "{label}: should still replay WAL records");
        assert_eq!(journal.backend().read_lease()?.ok_or("missing lock")?.owner_id, "rescuer");
        let _ = journal.close_cleanly(b"snap")?;
        assert_no_lock_temp_or_evict_leftovers(dir.path());
    }

    // Active valid lock must NOT be reclaimed.
    let active_dir = TempDirGuard::new("active_valid")?;
    write_stale_lock(active_dir.path(), "active-writer", 1_000, 60_000);
    assert!(matches!(open_dir_journal(active_dir.path(), "challenger", 60_000, 2_000), Err(FormatError::Io(_))));
    assert_eq!(DirectoryJournalBackend::open_dir(active_dir.path())?.read_lease()?.ok_or("active lock vanished")?.owner_id, "active-writer");

    // Future schema version lockfile fails with UnsupportedSchemaVersion and is preserved.
    let fut_dir = TempDirGuard::new("future_lock")?;
    let future_ver = CURRENT_JOURNAL_VERSION + 7;
    fs::write(fut_dir.child(LOCKFILE_NAME), format!(r#"{{"format_version":{future_ver},"future_only":true}}"#))?;
    assert!(matches!(
        open_dir_journal(fut_dir.path(), "older-client", 60_000, 2_000),
        Err(FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver
    ));
    assert!(fut_dir.child(LOCKFILE_NAME).exists(), "future lockfile must not be deleted");

    // MemoryJournalBackend empty-owner parity.
    let bad_lease_json = r#"{"format_version":1,"lease":{"format_version":1,"owner_id":" ","acquired_epoch_ms":1,"ttl_ms":1000},"snapshot":null,"snapshot_covered_seq":null,"journal_bytes":""}"#;
    assert!(matches!(MemoryJournalBackend::from_web_storage_json(bad_lease_json), Err(FormatError::Json(_))));
    let mut mem = MemoryJournalBackend::new();
    assert!(mem.acquire_lease(&JournalLockLease::new("seed-owner", 1, 1000)?, 1)?.is_none());
    assert!(mem.acquire_lease(&JournalLockLease::new("new-owner", 5_000, 1000)?, 5_000)?.is_some());
    Ok(())
}

#[test]
fn r1_s06_09_web_storage_json_enforces_real_serialized_size_and_supports_20mb_snapshot() -> TestResult {
    let twenty_mb = vec![0xA5u8; 20 * 1024 * 1024];
    let (mut journal, _) = CrashRecoveryJournal::open_or_recover(MemoryJournalBackend::new(), "web-tab-1", 60_000, 1_000)?;
    let _ = journal.record_command(json!({"cmd": "draw", "stroke": 42}))?;
    journal.checkpoint(&twenty_mb)?;
    let _ = journal.record_command(json!({"cmd": "draw", "stroke": 43}))?;

    let serialized = journal.abandon().to_web_storage_json()?;
    assert!(serialized.len() <= MAX_WEB_STORAGE_JSON_BYTES);

    let restored = MemoryJournalBackend::from_web_storage_json(&serialized)?;
    assert_eq!(restored.read_snapshot()?.as_deref(), Some(twenty_mb.as_slice()));
    let (_reopened, report) = CrashRecoveryJournal::open_or_recover(restored, "web-tab-2", 60_000, 120_000)?;
    assert_eq!(report.snapshot.as_deref(), Some(twenty_mb.as_slice()));
    assert_eq!(report.recovered_commands, vec![json!({"cmd": "draw", "stroke": 43})]);

    // Real serialized JSON exceeding 64 MiB rejected on export & import.
    let mut oversized_backend = MemoryJournalBackend::new();
    oversized_backend.write_snapshot_atomic(&vec![0x5Au8; 49 * 1024 * 1024])?;
    assert!(matches!(oversized_backend.to_web_storage_json(), Err(FormatError::Io(_))));

    let raw_oversized_json = format!(
        r#"{{"format_version":1,"lease":null,"snapshot":[0],"snapshot_covered_seq":null,"journal_bytes":"{}"}}"#,
        "A".repeat(MAX_WEB_STORAGE_JSON_BYTES)
    );
    assert!(raw_oversized_json.len() > MAX_WEB_STORAGE_JSON_BYTES);
    assert!(matches!(MemoryJournalBackend::from_web_storage_json(&raw_oversized_json), Err(FormatError::Io(_))));

    // Legacy integer-array byte encoding remains readable.
    let legacy_json = r#"{"format_version":1,"lease":null,"snapshot":[1,2,3,4],"snapshot_covered_seq":null,"journal_bytes":[]}"#;
    assert_eq!(MemoryJournalBackend::from_web_storage_json(legacy_json)?.read_snapshot()?.as_deref(), Some(&[1u8, 2, 3, 4][..]));
    Ok(())
}

#[test]
fn r1_s06_14_bounded_replay_and_append_enforce_bytes_line_and_record_limits() -> TestResult {
    // 1. Default 32 MiB limit rejects sparse >32 MiB ops.journal and cleans up acquired lock.
    let sparse_dir = TempDirGuard::new("sparse_huge_wal")?;
    create_sparse_file(&sparse_dir.child(JOURNAL_FILE_NAME), (MAX_JOURNAL_BYTES as u64).saturating_add(1))?;
    assert!(matches!(open_dir_journal(sparse_dir.path(), "reader", 60_000, 1_000), Err(FormatError::Io(_))));
    assert!(!sparse_dir.child(LOCKFILE_NAME).exists(), "failed replay must clean up newly acquired lock");

    // 2-5. Bounded replay rejections across line size, valid count, corrupt flood, and blank-line flood.
    let oversized_line = format!("{}\n", JournalOpRecord::new(1, json!({"payload": "X".repeat(600)}))?.to_ndjson_line()?);
    let mut six_valid = String::new();
    for seq in 1..=6u64 {
        six_valid.push_str(&JournalOpRecord::new(seq, json!({"i": seq}))?.to_ndjson_line()?);
        six_valid.push('\n');
    }
    let replay_cases = [
        ("oversized_line", oversized_line, JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 256, max_records: 100 }),
        ("too_many_records", six_valid, JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 4096, max_records: 5 }),
        ("corrupt_line_flood", "not-json\n".repeat(6), JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 4096, max_records: 5 }),
        ("blank_line_flood", "\n".repeat(2 * 4 + 1025), JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 4096, max_records: 2 }),
    ];
    for (label, content, limits) in replay_cases {
        let dir = TempDirGuard::new(label)?;
        let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(dir.child(JOURNAL_FILE_NAME))?;
        f.write_all(content.as_bytes())?;
        drop(f);
        let backend = DirectoryJournalBackend::open_dir(dir.path())?;
        assert!(
            matches!(CrashRecoveryJournal::open_or_recover_with_limits(backend, "reader", 60_000, 1_000, limits), Err(FormatError::Io(_))),
            "{label}: expected FormatError::Io"
        );
        assert!(!dir.child(LOCKFILE_NAME).exists(), "{label}: lock should be released on failure");
    }

    // 6. Live session record_command enforces limits and resets cleanly after checkpoint().
    let live_dir = TempDirGuard::new("live_limits")?;
    let limits = JournalLimits { max_bytes: 4096, max_line_bytes: 256, max_records: 3 };
    let (mut journal, _) =
        CrashRecoveryJournal::open_or_recover_with_limits(DirectoryJournalBackend::open_dir(live_dir.path())?, "writer", 60_000, 1_000, limits)?;
    assert_eq!(journal.limits(), limits);
    assert!(journal.record_command(json!({"big": "Z".repeat(300)})).is_err());
    for seq in 1..=3u64 {
        assert_eq!(journal.record_command(json!({"n": seq}))?, seq);
    }
    assert!(journal.record_command(json!({"n": 4})).is_err(), "4th command must hit max_records=3");
    journal.checkpoint(b"checkpoint-bytes")?;
    assert_eq!(journal.record_command(json!({"n": 4}))?, 4);
    let _ = journal.close_cleanly(b"final-snap")?;

    // 7. MemoryJournalBackend enforces bounded replay identically.
    let mut mem = MemoryJournalBackend::new();
    let lines =
        format!("{}\n{}\n", JournalOpRecord::new(1, json!({"k": 1}))?.to_ndjson_line()?, JournalOpRecord::new(2, json!({"k": 2}))?.to_ndjson_line()?);
    mem.replace_journal_bytes_atomic(lines.as_bytes())?;
    let tight = JournalLimits { max_bytes: 1024 * 1024, max_line_bytes: 4096, max_records: 1 };
    assert!(matches!(CrashRecoveryJournal::open_or_recover_with_limits(mem, "mem-reader", 60_000, 1_000, tight), Err(FormatError::Io(_))));
    Ok(())
}
