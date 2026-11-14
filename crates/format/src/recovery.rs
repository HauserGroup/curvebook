//! Crash-recovery write-ahead journal, atomic snapshots, and lease locking (`G2`).
//!
//! Provides two parity backends behind [`JournalBackend`]:
//! - [`MemoryJournalBackend`]: zero-OS-call in-memory model with JSON export/import
//!   (`to_web_storage_json` / `from_web_storage_json`) matching browser `localStorage` + Web Locks
//!   so `wasm32` targets share identical recovery behavior.
//! - [`DirectoryJournalBackend`] (`#[cfg(not(target_arch = "wasm32"))]`): native directory storage
//!   using exclusive lockfile creation (`lumencraft.lock`), atomic temp-file + rename snapshots
//!   (`snapshot.lumencraft`), and append-only NDJSON command records (`ops.journal`).

use serde::{Deserialize, Serialize};

use crate::FormatError;

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{ErrorKind, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};

/// Filename of the exclusive writer lease file on native backends.
pub const LOCKFILE_NAME: &str = "lumencraft.lock";
/// Filename of the baseline atomic project snapshot on native backends.
pub const SNAPSHOT_FILE_NAME: &str = "snapshot.lumencraft";
/// Filename of the append-only NDJSON recovery journal on native backends.
pub const JOURNAL_FILE_NAME: &str = "ops.journal";

const MAX_WEB_STORAGE_JSON_BYTES: usize = 64 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Exclusive writer lease stored in `lumencraft.lock` or serialized web-storage state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalLockLease {
    /// Identifier of the session/process/tab holding the lock lease.
    pub owner_id: String,
    /// Epoch timestamp (milliseconds) when the lease was acquired or last renewed.
    pub acquired_epoch_ms: u64,
    /// Lease validity duration in milliseconds after `acquired_epoch_ms`.
    pub ttl_ms: u64,
}

impl JournalLockLease {
    /// Construct a validated lease for `owner_id`.
    pub fn new(owner_id: impl Into<String>, acquired_epoch_ms: u64, ttl_ms: u64) -> Result<Self, FormatError> {
        let owner_id = owner_id.into();
        if owner_id.trim().is_empty() {
            return Err(FormatError::Io(String::from("journal lock owner_id must not be empty")));
        }
        Ok(Self { owner_id, acquired_epoch_ms, ttl_ms })
    }

    /// Epoch timestamp (milliseconds, saturating) at which this lease becomes stale.
    pub fn expires_epoch_ms(&self) -> u64 {
        self.acquired_epoch_ms.saturating_add(self.ttl_ms)
    }

    /// Return `true` when `now_epoch_ms` has reached or exceeded the lease expiry timestamp.
    pub fn is_stale(&self, now_epoch_ms: u64) -> bool {
        now_epoch_ms >= self.expires_epoch_ms()
    }

    /// Alias indicating whether a new writer is permitted to evict and recover this lease.
    pub fn can_recover(&self, now_epoch_ms: u64) -> bool {
        self.is_stale(now_epoch_ms)
    }
}

/// Single NDJSON line record in `ops.journal`: `{ "seq", "crc32", "command_json" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalOpRecord {
    /// Monotonically increasing 1-based command sequence number.
    pub seq: u64,
    /// IEEE CRC32 checksum over `seq` and canonical JSON representation of `command_json`.
    pub crc32: u32,
    /// Serialized command payload.
    pub command_json: serde_json::Value,
}

impl JournalOpRecord {
    /// Create a checksummed journal entry for `seq` and `command_json`.
    pub fn new(seq: u64, command_json: serde_json::Value) -> Result<Self, FormatError> {
        let crc32 = compute_record_crc32(seq, &command_json)?;
        Ok(Self { seq, crc32, command_json })
    }

    /// Compute and validate the record's CRC32 checksum, returning the computed checksum on mismatch.
    pub fn verify_crc(&self) -> Result<(), FormatError> {
        let actual = compute_record_crc32(self.seq, &self.command_json)?;
        if self.crc32 != actual {
            return Err(FormatError::CorruptCrc { entry: format!("{JOURNAL_FILE_NAME}#seq_{}", self.seq), expected: self.crc32, actual });
        }
        Ok(())
    }

    /// Serialize to a single newline-terminated NDJSON line.
    pub fn to_ndjson_line(&self) -> Result<String, FormatError> {
        let mut line = serde_json::to_string(self)?;
        line.push('\n');
        Ok(line)
    }
}

/// Compute the deterministic IEEE CRC32 over `(seq, command_json)`.
pub fn compute_record_crc32(seq: u64, command_json: &serde_json::Value) -> Result<u32, FormatError> {
    let canonical_cmd = serde_json::to_vec(command_json)?;
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&seq.to_le_bytes());
    hasher.update(&canonical_cmd);
    Ok(hasher.finalize())
}

/// Diagnostic and replayed payload returned by [`CrashRecoveryJournal::open_or_recover`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    /// Baseline snapshot payload read from `snapshot.lumencraft`, if present.
    pub snapshot: Option<Vec<u8>>,
    /// Validated journal command payloads in strict replay sequence order.
    pub recovered_commands: Vec<serde_json::Value>,
    /// Sequence numbers matching `recovered_commands` 1:1.
    pub recovered_seqs: Vec<u64>,
    /// Number of incomplete/torn tail lines dropped cleanly from `ops.journal`.
    pub discarded_corrupt_tail_lines: usize,
    /// Stale lease that was evicted during recovery, if any.
    pub recovered_from_stale_lock: Option<JournalLockLease>,
    /// Active lease held by the reopened [`CrashRecoveryJournal`].
    pub active_lease: JournalLockLease,
}

/// Storage backend abstraction for [`CrashRecoveryJournal`].
pub trait JournalBackend {
    /// Atomically acquire an exclusive lease, evicting a pre-existing lease only if it is stale.
    fn acquire_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<Option<JournalLockLease>, FormatError>;

    /// Refresh an active lease already held by `lease.owner_id`.
    fn renew_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<(), FormatError>;

    /// Release the lock lease if currently owned by `owner_id`.
    fn release_lease(&mut self, owner_id: &str) -> Result<(), FormatError>;

    /// Read the currently persisted lease, if any.
    fn read_lease(&self) -> Result<Option<JournalLockLease>, FormatError>;

    /// Write `snapshot.lumencraft` atomically so readers never observe partial snapshot bytes.
    fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError>;

    /// Read baseline `snapshot.lumencraft` bytes if present.
    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError>;

    /// Append a single NDJSON journal entry line to `ops.journal`.
    fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError>;

    /// Read raw bytes from `ops.journal` (empty vector if absent).
    fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError>;

    /// Atomically replace `ops.journal` bytes (for example after stripping a torn tail line).
    fn replace_journal_bytes_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError>;

    /// Remove or empty `ops.journal` on checkpoint or clean close.
    fn clear_journal(&mut self) -> Result<(), FormatError>;
}

/// Pure in-memory journal backend with zero OS calls, plus web-storage JSON import/export.
///
/// Models browser `localStorage` + Web Locks lease semantics identically on `wasm32` and native.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryJournalBackend {
    #[serde(default)]
    lease: Option<JournalLockLease>,
    #[serde(default)]
    snapshot: Option<Vec<u8>>,
    #[serde(default)]
    journal_bytes: Vec<u8>,
}

impl MemoryJournalBackend {
    /// Create an empty in-memory journal backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// Export the entire backend state (lock lease, baseline snapshot, and journal) as JSON
    /// suitable for persisting into browser `localStorage`.
    pub fn to_web_storage_json(&self) -> Result<String, FormatError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Restore a [`MemoryJournalBackend`] from a JSON string previously created by
    /// [`Self::to_web_storage_json`].
    pub fn from_web_storage_json(json: &str) -> Result<Self, FormatError> {
        if json.len() > MAX_WEB_STORAGE_JSON_BYTES {
            return Err(FormatError::Io(format!("web-storage journal payload exceeds budget of {MAX_WEB_STORAGE_JSON_BYTES} bytes")));
        }
        Ok(serde_json::from_str(json)?)
    }

    /// Inspect the stored snapshot bytes, if present.
    pub fn snapshot_bytes(&self) -> Option<&[u8]> {
        self.snapshot.as_deref()
    }

    /// Inspect the raw journal bytes currently stored.
    pub fn journal_bytes(&self) -> &[u8] {
        &self.journal_bytes
    }

    /// Return the currently stored lock lease, if present.
    pub fn lock_lease(&self) -> Option<&JournalLockLease> {
        self.lease.as_ref()
    }

    /// Overwrite the stored lock lease directly (useful for modeling pre-crash browser storage state).
    pub fn set_lock_lease(&mut self, lease: Option<JournalLockLease>) {
        self.lease = lease;
    }

    /// Append raw bytes directly to the in-memory journal buffer (e.g. simulating a torn tail write).
    pub fn append_raw_journal_bytes(&mut self, bytes: &[u8]) {
        self.journal_bytes.extend_from_slice(bytes);
    }

    /// Replace the raw in-memory journal buffer directly.
    pub fn set_raw_journal_bytes(&mut self, bytes: Vec<u8>) {
        self.journal_bytes = bytes;
    }
}

impl JournalBackend for MemoryJournalBackend {
    fn acquire_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<Option<JournalLockLease>, FormatError> {
        let evicted = match &self.lease {
            Some(existing) if !existing.is_stale(now_epoch_ms) => {
                return Err(FormatError::Io(format!(
                    "journal lock `{LOCKFILE_NAME}` is actively held by `{}` until {}ms",
                    existing.owner_id,
                    existing.expires_epoch_ms()
                )));
            }
            Some(stale) => Some(stale.clone()),
            None => None,
        };
        self.lease = Some(lease.clone());
        Ok(evicted)
    }

    fn renew_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<(), FormatError> {
        let Some(existing) = &self.lease else {
            return Err(FormatError::Io(String::from("cannot renew journal lock: no active lease present")));
        };
        if existing.owner_id != lease.owner_id {
            return Err(FormatError::Io(format!("cannot renew journal lock owned by `{}` from `{}`", existing.owner_id, lease.owner_id)));
        }
        let mut refreshed = lease.clone();
        refreshed.acquired_epoch_ms = now_epoch_ms;
        self.lease = Some(refreshed);
        Ok(())
    }

    fn release_lease(&mut self, owner_id: &str) -> Result<(), FormatError> {
        match &self.lease {
            None => Ok(()),
            Some(existing) if existing.owner_id == owner_id => {
                self.lease = None;
                Ok(())
            }
            Some(existing) => Err(FormatError::Io(format!("cannot release journal lock owned by `{}` from `{owner_id}`", existing.owner_id))),
        }
    }

    fn read_lease(&self) -> Result<Option<JournalLockLease>, FormatError> {
        Ok(self.lease.clone())
    }

    fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.snapshot = Some(bytes.to_vec());
        Ok(())
    }

    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
        Ok(self.snapshot.clone())
    }

    fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError> {
        self.journal_bytes.extend_from_slice(ndjson_line.as_bytes());
        if !ndjson_line.ends_with('\n') {
            self.journal_bytes.push(b'\n');
        }
        Ok(())
    }

    fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError> {
        Ok(self.journal_bytes.clone())
    }

    fn replace_journal_bytes_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.journal_bytes = bytes.to_vec();
        Ok(())
    }

    fn clear_journal(&mut self) -> Result<(), FormatError> {
        self.journal_bytes.clear();
        Ok(())
    }
}

/// Native filesystem crash-recovery backend storing `lumencraft.lock`, `snapshot.lumencraft`, and
/// `ops.journal` inside a dedicated recovery directory.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct DirectoryJournalBackend {
    dir: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl DirectoryJournalBackend {
    /// Open or create a filesystem journal directory at `dir`.
    pub fn open_dir(dir: impl AsRef<Path>) -> Result<Self, FormatError> {
        let dir_path = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir_path)?;
        Ok(Self { dir: dir_path })
    }

    /// Root directory backing this journal.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Full path to `lumencraft.lock`.
    pub fn lock_path(&self) -> PathBuf {
        self.dir.join(LOCKFILE_NAME)
    }

    /// Full path to `snapshot.lumencraft`.
    pub fn snapshot_path(&self) -> PathBuf {
        self.dir.join(SNAPSHOT_FILE_NAME)
    }

    /// Full path to `ops.journal`.
    pub fn journal_path(&self) -> PathBuf {
        self.dir.join(JOURNAL_FILE_NAME)
    }

    fn next_temp_path(&self, target_name: &str) -> PathBuf {
        let id = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.dir.join(format!(".{target_name}.tmp.{id}"))
    }

    fn write_temp_and_sync(&self, target_name: &str, bytes: &[u8]) -> Result<PathBuf, FormatError> {
        let tmp_path = self.next_temp_path(target_name);
        let write_result = (|| -> Result<(), std::io::Error> {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp_path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(FormatError::from(err));
        }
        Ok(tmp_path)
    }

    fn atomic_write_file(&self, target_name: &str, bytes: &[u8]) -> Result<(), FormatError> {
        let target_path = self.dir.join(target_name);
        let tmp_path = self.write_temp_and_sync(target_name, bytes)?;
        if let Err(err) = std::fs::rename(&tmp_path, &target_path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(FormatError::from(err));
        }
        sync_directory_best_effort(&self.dir);
        Ok(())
    }

    fn try_create_lock_exclusively(&self, lease_bytes: &[u8]) -> Result<bool, FormatError> {
        let lock_path = self.lock_path();
        let tmp_path = self.write_temp_and_sync(LOCKFILE_NAME, lease_bytes)?;
        match std::fs::hard_link(&tmp_path, &lock_path) {
            Ok(()) => {
                let _ = std::fs::remove_file(&tmp_path);
                sync_directory_best_effort(&self.dir);
                Ok(true)
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&tmp_path);
                Ok(false)
            }
            Err(_) => {
                // Fallback for filesystems without hard-link support: exclusive file creation.
                let _ = std::fs::remove_file(&tmp_path);
                match OpenOptions::new().write(true).create_new(true).open(&lock_path) {
                    Ok(mut file) => {
                        if let Err(write_err) = file.write_all(lease_bytes).and_then(|()| file.sync_all()) {
                            let _ = std::fs::remove_file(&lock_path);
                            return Err(FormatError::from(write_err));
                        }
                        sync_directory_best_effort(&self.dir);
                        Ok(true)
                    }
                    Err(create_err) if create_err.kind() == ErrorKind::AlreadyExists => Ok(false),
                    Err(create_err) => Err(FormatError::from(create_err)),
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_directory_best_effort(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl JournalBackend for DirectoryJournalBackend {
    fn acquire_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<Option<JournalLockLease>, FormatError> {
        let lease_bytes = serde_json::to_vec(lease)?;
        if self.try_create_lock_exclusively(&lease_bytes)? {
            return Ok(None);
        }

        let existing = match self.read_lease() {
            Ok(Some(found)) => found,
            Ok(None) => {
                // Lock disappeared between exclusive create check and read; attempt once more.
                if self.try_create_lock_exclusively(&lease_bytes)? {
                    return Ok(None);
                }
                return Err(FormatError::Io(String::from("concurrent journal lock contention")));
            }
            Err(err) => return Err(err),
        };

        if !existing.is_stale(now_epoch_ms) {
            return Err(FormatError::Io(format!(
                "journal lock `{LOCKFILE_NAME}` is actively held by `{}` until {}ms",
                existing.owner_id,
                existing.expires_epoch_ms()
            )));
        }

        // Stale lock recovery: unlink stale file and win exclusive creation atomically.
        match std::fs::remove_file(self.lock_path()) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(FormatError::from(err)),
        }
        if self.try_create_lock_exclusively(&lease_bytes)? {
            return Ok(Some(existing));
        }

        Err(FormatError::Io(String::from("stale journal lock was claimed concurrently by another writer")))
    }

    fn renew_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<(), FormatError> {
        let Some(existing) = self.read_lease()? else {
            return Err(FormatError::Io(String::from("cannot renew journal lock: lockfile missing")));
        };
        if existing.owner_id != lease.owner_id {
            return Err(FormatError::Io(format!("cannot renew journal lock owned by `{}` from `{}`", existing.owner_id, lease.owner_id)));
        }
        let mut refreshed = lease.clone();
        refreshed.acquired_epoch_ms = now_epoch_ms;
        let bytes = serde_json::to_vec(&refreshed)?;
        self.atomic_write_file(LOCKFILE_NAME, &bytes)
    }

    fn release_lease(&mut self, owner_id: &str) -> Result<(), FormatError> {
        let Some(existing) = self.read_lease()? else {
            return Ok(());
        };
        if existing.owner_id != owner_id {
            return Err(FormatError::Io(format!("cannot release journal lock owned by `{}` from `{owner_id}`", existing.owner_id)));
        }
        match std::fs::remove_file(self.lock_path()) {
            Ok(()) => {
                sync_directory_best_effort(&self.dir);
                Ok(())
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(FormatError::from(err)),
        }
    }

    fn read_lease(&self) -> Result<Option<JournalLockLease>, FormatError> {
        match std::fs::read(self.lock_path()) {
            Ok(bytes) => {
                let parsed: JournalLockLease = serde_json::from_slice(&bytes)?;
                Ok(Some(parsed))
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FormatError::from(err)),
        }
    }

    fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.atomic_write_file(SNAPSHOT_FILE_NAME, bytes)
    }

    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
        match std::fs::read(self.snapshot_path()) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FormatError::from(err)),
        }
    }

    fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError> {
        let mut file = OpenOptions::new().create(true).append(true).open(self.journal_path())?;
        file.write_all(ndjson_line.as_bytes())?;
        if !ndjson_line.ends_with('\n') {
            file.write_all(b"\n")?;
        }
        file.sync_data()?;
        Ok(())
    }

    fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError> {
        match std::fs::read(self.journal_path()) {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(FormatError::from(err)),
        }
    }

    fn replace_journal_bytes_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.atomic_write_file(JOURNAL_FILE_NAME, bytes)
    }

    fn clear_journal(&mut self) -> Result<(), FormatError> {
        match std::fs::remove_file(self.journal_path()) {
            Ok(()) => {
                sync_directory_best_effort(&self.dir);
                Ok(())
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(FormatError::from(err)),
        }
    }
}

/// Crash-recovery journal session managing exclusive lock acquisition, NDJSON command logging,
/// atomic snapshot checkpointing, torn-tail truncation, and clean shutdown.
#[derive(Debug)]
pub struct CrashRecoveryJournal<B: JournalBackend> {
    backend: B,
    lease: JournalLockLease,
    next_seq: u64,
}

impl<B: JournalBackend> CrashRecoveryJournal<B> {
    /// Open a journal session, acquiring or recovering a stale lock lease, loading any baseline
    /// snapshot, validating CRC32 checksums of `ops.journal`, and safely truncating torn tail
    /// lines without crashing.
    pub fn open_or_recover(
        mut backend: B,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
    ) -> Result<(Self, RecoveryReport), FormatError> {
        let lease = JournalLockLease::new(owner_id, now_epoch_ms, ttl_ms)?;
        let recovered_from_stale_lock = backend.acquire_lease(&lease, now_epoch_ms)?;

        let recovered_state = match recover_journal_contents(&mut backend) {
            Ok(state) => state,
            Err(err) => {
                let _ = backend.release_lease(&lease.owner_id);
                return Err(err);
            }
        };

        let next_seq = recovered_state.last_seq.saturating_add(1).max(1);
        let report = RecoveryReport {
            snapshot: recovered_state.snapshot,
            recovered_commands: recovered_state.commands,
            recovered_seqs: recovered_state.seqs,
            discarded_corrupt_tail_lines: recovered_state.discarded_corrupt_tail_lines,
            recovered_from_stale_lock,
            active_lease: lease.clone(),
        };
        Ok((Self { backend, lease, next_seq }, report))
    }

    /// Append a new command JSON payload to `ops.journal` with monotonic sequence and CRC32.
    pub fn record_command(&mut self, command_json: serde_json::Value) -> Result<u64, FormatError> {
        let seq = self.next_seq;
        let record = JournalOpRecord::new(seq, command_json)?;
        let line = record.to_ndjson_line()?;
        self.backend.append_journal_line(&line)?;
        self.next_seq = seq.checked_add(1).ok_or_else(|| FormatError::Io(String::from("journal sequence number overflow")))?;
        Ok(seq)
    }

    /// Persist an atomic snapshot checkpoint and clear `ops.journal`.
    pub fn checkpoint(&mut self, snapshot_bytes: &[u8]) -> Result<(), FormatError> {
        self.backend.write_snapshot_atomic(snapshot_bytes)?;
        self.backend.clear_journal()?;
        Ok(())
    }

    /// Refresh the active writer lease timestamp to `now_epoch_ms`.
    pub fn renew_lease(&mut self, now_epoch_ms: u64) -> Result<(), FormatError> {
        self.backend.renew_lease(&self.lease, now_epoch_ms)?;
        self.lease.acquired_epoch_ms = now_epoch_ms;
        Ok(())
    }

    /// Write the final atomic snapshot, clear `ops.journal`, release `lumencraft.lock`, and return
    /// the underlying backend.
    pub fn close_cleanly(mut self, final_snapshot_bytes: &[u8]) -> Result<B, FormatError> {
        self.backend.write_snapshot_atomic(final_snapshot_bytes)?;
        self.backend.clear_journal()?;
        self.backend.release_lease(&self.lease.owner_id)?;
        Ok(self.backend)
    }

    /// Consume the journal handle WITHOUT clearing logs or releasing the lock (crash simulation helper).
    pub fn abandon(self) -> B {
        self.backend
    }

    /// Borrow the active lock lease.
    pub fn lease(&self) -> &JournalLockLease {
        &self.lease
    }

    /// Next sequence number to be assigned on [`Self::record_command`].
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Borrow the underlying backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Mutably borrow the underlying backend.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}

struct RecoveredJournalState {
    snapshot: Option<Vec<u8>>,
    commands: Vec<serde_json::Value>,
    seqs: Vec<u64>,
    discarded_corrupt_tail_lines: usize,
    last_seq: u64,
}

fn recover_journal_contents<B: JournalBackend>(backend: &mut B) -> Result<RecoveredJournalState, FormatError> {
    let snapshot = backend.read_snapshot()?;
    let raw_journal = backend.read_journal_bytes()?;
    let lines = collect_non_empty_lines(&raw_journal);

    let mut commands = Vec::with_capacity(lines.len());
    let mut seqs = Vec::with_capacity(lines.len());
    let mut cleaned_bytes = Vec::with_capacity(raw_journal.len());
    let mut discarded_corrupt_tail_lines = 0usize;
    let mut prev_seq = 0u64;

    let last_idx = lines.len().saturating_sub(1);
    for (idx, &(line_no, raw_line)) in lines.iter().enumerate() {
        let is_tail = idx == last_idx;
        let Some(record) = parse_journal_line(line_no, raw_line, is_tail)? else {
            discarded_corrupt_tail_lines = discarded_corrupt_tail_lines.saturating_add(1);
            break;
        };

        record.verify_crc()?;
        if record.seq == 0 || record.seq <= prev_seq {
            return Err(FormatError::InvalidPipeline {
                reason: format!("non-monotonic journal sequence {} after {prev_seq} at line {line_no}", record.seq),
            });
        }
        prev_seq = record.seq;
        commands.push(record.command_json);
        seqs.push(record.seq);
        cleaned_bytes.extend_from_slice(raw_line);
        cleaned_bytes.push(b'\n');
    }

    if discarded_corrupt_tail_lines > 0 {
        backend.replace_journal_bytes_atomic(&cleaned_bytes)?;
    }

    Ok(RecoveredJournalState { snapshot, commands, seqs, discarded_corrupt_tail_lines, last_seq: prev_seq })
}

fn collect_non_empty_lines(raw: &[u8]) -> Vec<(usize, &[u8])> {
    let mut out = Vec::new();
    for (zero_idx, chunk) in raw.split(|&b| b == b'\n').enumerate() {
        let trimmed = chunk.strip_suffix(b"\r").unwrap_or(chunk);
        if !trimmed.is_empty() && !trimmed.iter().all(u8::is_ascii_whitespace) {
            out.push((zero_idx.saturating_add(1), trimmed));
        }
    }
    out
}

fn parse_journal_line(line_no: usize, raw_line: &[u8], is_tail: bool) -> Result<Option<JournalOpRecord>, FormatError> {
    let text = match std::str::from_utf8(raw_line) {
        Ok(s) => s,
        Err(err) if is_tail => {
            let _ = err;
            return Ok(None);
        }
        Err(err) => {
            return Err(FormatError::Json(format!("invalid UTF-8 in {JOURNAL_FILE_NAME} line {line_no}: {err}")));
        }
    };

    match serde_json::from_str::<JournalOpRecord>(text) {
        Ok(record) => Ok(Some(record)),
        Err(_) if is_tail => Ok(None),
        Err(err) => Err(FormatError::Json(format!("malformed entry in {JOURNAL_FILE_NAME} line {line_no}: {err}"))),
    }
}
