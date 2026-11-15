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
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// Current persisted format version of [`JournalOpRecord`] entries in `ops.journal`.
pub const CURRENT_JOURNAL_VERSION: u32 = 1;
/// Current persisted format version of [`JournalLockLease`] files (`lumencraft.lock`).
pub const CURRENT_LOCK_VERSION: u32 = 1;

/// Check whether `lease` may be reclaimed because its TTL expired or its local PID died.
#[must_use]
pub fn is_lease_reclaimable(lease: &JournalLockLease, now_epoch_ms: u64) -> bool {
    lease.is_stale(now_epoch_ms) || is_dead_local_owner(&lease.owner_id)
}

/// Return `true` only when `owner_id` encodes a local `lumencraft-pid-<N>` owner whose OS
/// process is verified absent (`ESRCH` / missing `/proc/<pid>`), falling back to `false` (TTL)
/// whenever uncertain or on unsupported platforms.
#[must_use]
pub fn is_dead_local_owner(owner_id: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(pid) = parse_local_lumencraft_pid(owner_id) else {
            return false;
        };
        if pid == std::process::id() {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            if Path::new("/proc/self").exists() {
                return !Path::new(&format!("/proc/{pid}")).exists();
            }
        }
        #[cfg(unix)]
        {
            is_unix_pid_absent(pid)
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            false
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = owner_id;
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_local_lumencraft_pid(owner_id: &str) -> Option<u32> {
    let rest = owner_id.strip_prefix("lumencraft-pid-")?;
    let pid_part = rest.split('-').next()?;
    let pid: u32 = pid_part.parse().ok()?;
    if pid == 0 || i32::try_from(pid).is_err() {
        return None;
    }
    Some(pid)
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn is_unix_pid_absent(pid: u32) -> bool {
    let pid_arg = pid.to_string();
    let Ok(output) = std::process::Command::new("/bin/kill").args(["-0", &pid_arg]).output() else {
        return false;
    };
    if output.status.success() {
        return false;
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    stderr.contains("no such process") || stderr.contains("esrch")
}

/// Filename of the exclusive writer lease file on native backends.
pub const LOCKFILE_NAME: &str = "lumencraft.lock";
/// Filename of the baseline atomic project snapshot on native backends.
pub const SNAPSHOT_FILE_NAME: &str = "snapshot.lumencraft";
/// Filename of the append-only NDJSON recovery journal on native backends.
pub const JOURNAL_FILE_NAME: &str = "ops.journal";

const MAX_WEB_STORAGE_JSON_BYTES: usize = 64 * 1024 * 1024;

fn default_v1() -> u32 {
    1
}

/// Exclusive writer lease stored in `lumencraft.lock` or serialized web-storage state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalLockLease {
    /// Persisted schema version of this lock lease (`1`).
    #[serde(default = "default_v1")]
    pub format_version: u32,
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
        Ok(Self { format_version: CURRENT_LOCK_VERSION, owner_id, acquired_epoch_ms, ttl_ms })
    }

    /// Reject leases with unsupported `format_version` (`0` or `> CURRENT_LOCK_VERSION`).
    pub fn validate_version(&self) -> Result<(), FormatError> {
        if self.format_version == 0 || self.format_version > CURRENT_LOCK_VERSION {
            return Err(FormatError::UnsupportedSchemaVersion { version: self.format_version, max_supported: CURRENT_LOCK_VERSION });
        }
        Ok(())
    }

    /// Deserialize a lock lease from JSON bytes and validate its `format_version`.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, FormatError> {
        let lease: Self = serde_json::from_slice(bytes)?;
        lease.validate_version()?;
        Ok(lease)
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

/// Single NDJSON line record in `ops.journal`: `{ "format_version", "seq", "crc32", "command_json" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalOpRecord {
    /// Persisted schema version of this journal record (`1`).
    #[serde(default = "default_v1")]
    pub format_version: u32,
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
        Ok(Self { format_version: CURRENT_JOURNAL_VERSION, seq, crc32, command_json })
    }

    /// Reject journal records with unsupported `format_version` (`0` or `> CURRENT_JOURNAL_VERSION`).
    pub fn validate_version(&self) -> Result<(), FormatError> {
        if self.format_version == 0 || self.format_version > CURRENT_JOURNAL_VERSION {
            return Err(FormatError::UnsupportedSchemaVersion { version: self.format_version, max_supported: CURRENT_JOURNAL_VERSION });
        }
        Ok(())
    }

    /// Parse a single JSON journal record string and validate its `format_version`.
    pub fn from_json_str(text: &str) -> Result<Self, FormatError> {
        let record: Self = serde_json::from_str(text)?;
        record.validate_version()?;
        Ok(record)
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

/// Filename of the snapshot coverage metadata sidecar on native backends.
#[cfg(not(target_arch = "wasm32"))]
const SNAPSHOT_META_FILE_NAME: &str = "snapshot.meta.json";
/// Staging filename written before replacing `snapshot.lumencraft` so crashes during snapshot
/// replacement still keep the covered journal sequence paired with the snapshot checksum.
#[cfg(not(target_arch = "wasm32"))]
const SNAPSHOT_PENDING_META_FILE_NAME: &str = "snapshot.pending.meta.json";
#[cfg(not(target_arch = "wasm32"))]
const CURRENT_SNAPSHOT_META_VERSION: u32 = 1;
/// Sentinel key embedded in legacy-backend fallback checkpoint markers inside `ops.journal`.
const SNAPSHOT_MARKER_KEY: &str = "__lumencraft_snapshot_marker__";

/// Structured warning emitted when crash recovery quarantines a damaged journal entry without
/// aborting recovery of intact records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecoveryWarning {
    /// 1-based non-empty line index in `ops.journal` (`0` if journal-wide).
    pub line: usize,
    /// Classification tag (`"corrupt_tail_line"`, `"corrupt_interior_line"`, etc.).
    pub kind: String,
    /// Human-readable description of the recovered defect.
    pub message: String,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotMeta {
    #[serde(default = "default_v1")]
    format_version: u32,
    covered_seq: u64,
    snapshot_crc32: u32,
    snapshot_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SnapshotMarkerPayload {
    covered_seq: u64,
    snapshot_crc32: u32,
    snapshot_len: u64,
}

/// Diagnostic and replayed payload returned by [`CrashRecoveryJournal::open_or_recover`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    /// Baseline snapshot payload read from `snapshot.lumencraft`, if present.
    pub snapshot: Option<Vec<u8>>,
    /// Highest journal sequence number covered by `snapshot`, when recorded atomically at checkpoint.
    #[serde(default)]
    pub snapshot_covered_seq: Option<u64>,
    /// Validated journal command payloads in strict replay sequence order (`seq > snapshot_covered_seq`).
    pub recovered_commands: Vec<serde_json::Value>,
    /// Sequence numbers matching `recovered_commands` 1:1.
    pub recovered_seqs: Vec<u64>,
    /// Number of incomplete/torn tail lines dropped cleanly from `ops.journal`.
    pub discarded_corrupt_tail_lines: usize,
    /// Number of damaged interior lines quarantined cleanly from `ops.journal`.
    #[serde(default)]
    pub discarded_corrupt_interior_lines: usize,
    /// Structured warnings emitted for quarantined/repaired journal entries during recovery.
    #[serde(default)]
    pub warnings: Vec<JournalRecoveryWarning>,
    /// Stale lease that was evicted during recovery, if any.
    pub recovered_from_stale_lock: Option<JournalLockLease>,
    /// Active lease held by the reopened [`CrashRecoveryJournal`].
    pub active_lease: JournalLockLease,
}

impl RecoveryReport {
    /// Total corrupt/truncated lines quarantined across tail and interior positions.
    pub fn total_discarded_corrupt_lines(&self) -> usize {
        self.discarded_corrupt_tail_lines.saturating_add(self.discarded_corrupt_interior_lines)
    }
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

    /// Atomically replace `snapshot.lumencraft` and bind `covered_seq` to the new snapshot bytes.
    fn write_snapshot_with_seq(&mut self, bytes: &[u8], covered_seq: u64) -> Result<(), FormatError> {
        let _ = covered_seq;
        self.write_snapshot_atomic(bytes)
    }

    /// Read baseline `snapshot.lumencraft` bytes if present.
    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError>;

    /// Read the highest journal sequence covered by the persisted snapshot, if known.
    fn read_snapshot_covered_seq(&self) -> Result<Option<u64>, FormatError> {
        Ok(None)
    }

    /// Clear persisted snapshot-covered sequence metadata once the journal has been cleared cleanly.
    fn clear_snapshot_covered_seq(&mut self) -> Result<(), FormatError> {
        Ok(())
    }

    /// Append a single NDJSON journal entry line to `ops.journal`.
    fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError>;

    /// Read raw bytes from `ops.journal` (empty vector if absent).
    fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError>;

    /// Atomically replace `ops.journal` bytes (for example after stripping damaged lines).
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
    snapshot_covered_seq: Option<u64>,
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
        let backend: Self = serde_json::from_str(json)?;
        if let Some(lease) = &backend.lease {
            lease.validate_version()?;
        }
        Ok(backend)
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
        lease.validate_version()?;
        let evicted = match &self.lease {
            Some(existing) => {
                existing.validate_version()?;
                if !is_lease_reclaimable(existing, now_epoch_ms) {
                    return Err(FormatError::Io(format!(
                        "journal lock `{LOCKFILE_NAME}` is actively held by `{}` until {}ms",
                        existing.owner_id,
                        existing.expires_epoch_ms()
                    )));
                }
                Some(existing.clone())
            }
            None => None,
        };
        self.lease = Some(lease.clone());
        Ok(evicted)
    }

    fn renew_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<(), FormatError> {
        lease.validate_version()?;
        let Some(existing) = &self.lease else {
            return Err(FormatError::Io(String::from("cannot renew journal lock: no active lease present")));
        };
        existing.validate_version()?;
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
        if let Some(existing) = &self.lease {
            existing.validate_version()?;
        }
        Ok(self.lease.clone())
    }

    fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.snapshot = Some(bytes.to_vec());
        self.snapshot_covered_seq = None;
        Ok(())
    }

    fn write_snapshot_with_seq(&mut self, bytes: &[u8], covered_seq: u64) -> Result<(), FormatError> {
        self.snapshot = Some(bytes.to_vec());
        self.snapshot_covered_seq = Some(covered_seq);
        Ok(())
    }

    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
        Ok(self.snapshot.clone())
    }

    fn read_snapshot_covered_seq(&self) -> Result<Option<u64>, FormatError> {
        Ok(self.snapshot_covered_seq)
    }

    fn clear_snapshot_covered_seq(&mut self) -> Result<(), FormatError> {
        self.snapshot_covered_seq = None;
        Ok(())
    }

    fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError> {
        if !self.journal_bytes.is_empty() && !self.journal_bytes.ends_with(b"\n") {
            self.journal_bytes.push(b'\n');
        }
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

    fn snapshot_meta_path(&self) -> PathBuf {
        self.dir.join(SNAPSHOT_META_FILE_NAME)
    }

    fn pending_snapshot_meta_path(&self) -> PathBuf {
        self.dir.join(SNAPSHOT_PENDING_META_FILE_NAME)
    }

    fn read_matching_snapshot_meta(&self, meta_path: &Path, actual_crc: u32, actual_len: u64) -> Result<Option<u64>, FormatError> {
        let meta_bytes = match std::fs::read(meta_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(FormatError::from(err)),
        };
        let Ok(meta) = serde_json::from_slice::<SnapshotMeta>(&meta_bytes) else {
            return Ok(None);
        };
        if meta.format_version == CURRENT_SNAPSHOT_META_VERSION && meta.snapshot_crc32 == actual_crc && meta.snapshot_len == actual_len {
            return Ok(Some(meta.covered_seq));
        }
        Ok(None)
    }

    fn try_create_lock_exclusively(&self, lease_bytes: &[u8]) -> Result<bool, FormatError> {
        let lock_path = self.lock_path();
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

#[cfg(not(target_arch = "wasm32"))]
fn sync_directory_best_effort(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl JournalBackend for DirectoryJournalBackend {
    fn acquire_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<Option<JournalLockLease>, FormatError> {
        lease.validate_version()?;
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

        if !is_lease_reclaimable(&existing, now_epoch_ms) {
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
        lease.validate_version()?;
        let Some(existing) = self.read_lease()? else {
            return Err(FormatError::Io(String::from("cannot renew journal lock: lockfile missing")));
        };
        if existing.owner_id != lease.owner_id {
            return Err(FormatError::Io(format!("cannot renew journal lock owned by `{}` from `{}`", existing.owner_id, lease.owner_id)));
        }
        let mut refreshed = lease.clone();
        refreshed.acquired_epoch_ms = now_epoch_ms;
        let bytes = serde_json::to_vec(&refreshed)?;
        lumencraft_platform::fs::write_atomic(&self.lock_path(), &bytes)?;
        Ok(())
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
            Ok(bytes) => Ok(Some(JournalLockLease::from_json_bytes(&bytes)?)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FormatError::from(err)),
        }
    }

    fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.clear_snapshot_covered_seq()?;
        lumencraft_platform::fs::write_atomic(&self.snapshot_path(), bytes)?;
        Ok(())
    }

    fn write_snapshot_with_seq(&mut self, bytes: &[u8], covered_seq: u64) -> Result<(), FormatError> {
        let snapshot_len = u64::try_from(bytes.len()).map_err(|_| FormatError::Io(String::from("snapshot length overflow")))?;
        let meta = SnapshotMeta { format_version: CURRENT_SNAPSHOT_META_VERSION, covered_seq, snapshot_crc32: crc32fast::hash(bytes), snapshot_len };
        let meta_bytes = serde_json::to_vec(&meta)?;
        let pending_path = self.pending_snapshot_meta_path();
        let meta_path = self.snapshot_meta_path();
        lumencraft_platform::fs::write_atomic(&pending_path, &meta_bytes)?;
        lumencraft_platform::fs::write_atomic(&self.snapshot_path(), bytes)?;
        std::fs::rename(&pending_path, &meta_path)?;
        sync_directory_best_effort(&self.dir);
        Ok(())
    }

    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
        match std::fs::read(self.snapshot_path()) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FormatError::from(err)),
        }
    }

    fn read_snapshot_covered_seq(&self) -> Result<Option<u64>, FormatError> {
        let Some(snapshot_bytes) = self.read_snapshot()? else {
            return Ok(None);
        };
        let actual_crc = crc32fast::hash(&snapshot_bytes);
        let actual_len = u64::try_from(snapshot_bytes.len()).map_err(|_| FormatError::Io(String::from("snapshot length overflow")))?;
        if let Some(seq) = self.read_matching_snapshot_meta(&self.snapshot_meta_path(), actual_crc, actual_len)? {
            return Ok(Some(seq));
        }
        self.read_matching_snapshot_meta(&self.pending_snapshot_meta_path(), actual_crc, actual_len)
    }

    fn clear_snapshot_covered_seq(&mut self) -> Result<(), FormatError> {
        for path in [self.snapshot_meta_path(), self.pending_snapshot_meta_path()] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => return Err(FormatError::from(err)),
            }
        }
        sync_directory_best_effort(&self.dir);
        Ok(())
    }

    fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError> {
        let mut file = OpenOptions::new().create(true).read(true).append(true).open(self.journal_path())?;
        let pre_len = file.metadata()?.len();
        let needs_leading_newline = if pre_len > 0 {
            file.seek(SeekFrom::Start(pre_len - 1))?;
            let mut last = [0u8; 1];
            file.read_exact(&mut last)?;
            last[0] != b'\n'
        } else {
            false
        };
        file.seek(SeekFrom::End(0))?;
        let write_result = (|| -> std::io::Result<()> {
            if needs_leading_newline {
                file.write_all(b"\n")?;
            }
            file.write_all(ndjson_line.as_bytes())?;
            if !ndjson_line.ends_with('\n') {
                file.write_all(b"\n")?;
            }
            file.sync_data()?;
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = file.set_len(pre_len);
            return Err(FormatError::from(err));
        }
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
        lumencraft_platform::fs::write_atomic(&self.journal_path(), bytes)?;
        Ok(())
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
            snapshot_covered_seq: recovered_state.snapshot_covered_seq,
            recovered_commands: recovered_state.commands,
            recovered_seqs: recovered_state.seqs,
            discarded_corrupt_tail_lines: recovered_state.discarded_corrupt_tail_lines,
            discarded_corrupt_interior_lines: recovered_state.discarded_corrupt_interior_lines,
            warnings: recovered_state.warnings,
            recovered_from_stale_lock,
            active_lease: lease.clone(),
        };
        Ok((Self { backend, lease, next_seq }, report))
    }

    fn ensure_lease_ownership(&self, action: &str) -> Result<(), FormatError> {
        let Some(existing) = self.backend.read_lease()? else {
            return Err(FormatError::Io(format!("cannot {action}: journal lock `{LOCKFILE_NAME}` is missing")));
        };
        if existing.owner_id != self.lease.owner_id {
            return Err(FormatError::Io(format!(
                "cannot {action}: journal lock `{LOCKFILE_NAME}` is owned by `{}` instead of `{}`",
                existing.owner_id, self.lease.owner_id
            )));
        }
        Ok(())
    }

    /// Append a new command JSON payload to `ops.journal` with monotonic sequence and CRC32.
    pub fn record_command(&mut self, command_json: serde_json::Value) -> Result<u64, FormatError> {
        self.ensure_lease_ownership("record journal command")?;
        let seq = self.next_seq;
        let record = JournalOpRecord::new(seq, command_json)?;
        let line = record.to_ndjson_line()?;
        self.backend.append_journal_line(&line)?;
        self.next_seq = seq.checked_add(1).ok_or_else(|| FormatError::Io(String::from("journal sequence number overflow")))?;
        Ok(seq)
    }

    /// Persist an atomic snapshot checkpoint binding the covered journal sequence BEFORE
    /// truncating `ops.journal`.
    pub fn checkpoint(&mut self, snapshot_bytes: &[u8]) -> Result<(), FormatError> {
        self.ensure_lease_ownership("checkpoint journal")?;
        let covered_seq = self.next_seq.saturating_sub(1);
        self.backend.write_snapshot_with_seq(snapshot_bytes, covered_seq)?;
        let backend_stored_seq = self.backend.read_snapshot_covered_seq()? == Some(covered_seq);
        if covered_seq > 0 && !backend_stored_seq {
            let marker_seq = self.next_seq;
            let marker = build_snapshot_marker_record(marker_seq, covered_seq, snapshot_bytes)?;
            let line = marker.to_ndjson_line()?;
            self.backend.append_journal_line(&line)?;
            if let Err(err) = self.backend.clear_journal() {
                self.next_seq = marker_seq.checked_add(1).ok_or_else(|| FormatError::Io(String::from("journal sequence number overflow")))?;
                return Err(err);
            }
            return Ok(());
        }
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
        self.ensure_lease_ownership("close journal cleanly")?;
        let covered_seq = self.next_seq.saturating_sub(1);
        self.backend.write_snapshot_with_seq(final_snapshot_bytes, covered_seq)?;
        let backend_stored_seq = self.backend.read_snapshot_covered_seq()? == Some(covered_seq);
        if covered_seq > 0 && !backend_stored_seq {
            let marker_seq = self.next_seq;
            let marker = build_snapshot_marker_record(marker_seq, covered_seq, final_snapshot_bytes)?;
            let line = marker.to_ndjson_line()?;
            self.backend.append_journal_line(&line)?;
        }
        self.backend.clear_journal()?;
        self.backend.clear_snapshot_covered_seq()?;
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
    snapshot_covered_seq: Option<u64>,
    commands: Vec<serde_json::Value>,
    seqs: Vec<u64>,
    discarded_corrupt_tail_lines: usize,
    discarded_corrupt_interior_lines: usize,
    warnings: Vec<JournalRecoveryWarning>,
    last_seq: u64,
}

fn build_snapshot_marker_record(seq: u64, covered_seq: u64, snapshot_bytes: &[u8]) -> Result<JournalOpRecord, FormatError> {
    let snapshot_len = u64::try_from(snapshot_bytes.len()).map_err(|_| FormatError::Io(String::from("snapshot length overflow")))?;
    let payload = SnapshotMarkerPayload { covered_seq, snapshot_crc32: crc32fast::hash(snapshot_bytes), snapshot_len };
    let mut map = serde_json::Map::new();
    map.insert(String::from(SNAPSHOT_MARKER_KEY), serde_json::to_value(payload)?);
    JournalOpRecord::new(seq, serde_json::Value::Object(map))
}

fn is_snapshot_marker(command_json: &serde_json::Value) -> bool {
    command_json.as_object().is_some_and(|obj| obj.contains_key(SNAPSHOT_MARKER_KEY))
}

fn extract_matching_snapshot_marker(command_json: &serde_json::Value, snapshot: Option<&[u8]>) -> Option<SnapshotMarkerPayload> {
    let obj = command_json.as_object()?;
    let raw_marker = obj.get(SNAPSHOT_MARKER_KEY)?;
    let payload: SnapshotMarkerPayload = serde_json::from_value(raw_marker.clone()).ok()?;
    let snap = snapshot?;
    let actual_len = u64::try_from(snap.len()).ok()?;
    if payload.snapshot_len == actual_len && payload.snapshot_crc32 == crc32fast::hash(snap) { Some(payload) } else { None }
}

fn recover_journal_contents<B: JournalBackend>(backend: &mut B) -> Result<RecoveredJournalState, FormatError> {
    let snapshot = backend.read_snapshot()?;
    let backend_covered_seq = backend.read_snapshot_covered_seq()?;
    let raw_journal = backend.read_journal_bytes()?;
    let lines = collect_non_empty_lines(&raw_journal);

    let mut parsed_records: Vec<(Vec<u8>, JournalOpRecord)> = Vec::with_capacity(lines.len());
    let mut cleaned_bytes = Vec::with_capacity(raw_journal.len());
    let mut discarded_corrupt_tail_lines = 0usize;
    let mut discarded_corrupt_interior_lines = 0usize;
    let mut warnings = Vec::new();
    let mut prev_seq = 0u64;
    let mut marker_covered_seq: Option<u64> = None;

    let last_idx = lines.len().saturating_sub(1);
    for (idx, &(line_no, raw_line)) in lines.iter().enumerate() {
        let is_tail = idx == last_idx;
        let record = match parse_journal_line(line_no, raw_line) {
            Ok(record) => record,
            Err(err @ FormatError::UnsupportedSchemaVersion { .. }) => return Err(err),
            Err(err) => {
                let (kind, bucket) = if is_tail {
                    discarded_corrupt_tail_lines = discarded_corrupt_tail_lines.saturating_add(1);
                    ("corrupt_tail_line", "tail")
                } else {
                    discarded_corrupt_interior_lines = discarded_corrupt_interior_lines.saturating_add(1);
                    ("corrupt_interior_line", "interior")
                };
                warnings.push(JournalRecoveryWarning {
                    line: line_no,
                    kind: String::from(kind),
                    message: format!("quarantined malformed {bucket} journal entry at {JOURNAL_FILE_NAME} line {line_no}: {err}"),
                });
                continue;
            }
        };

        record.verify_crc()?;
        if record.seq == 0 || record.seq <= prev_seq {
            return Err(FormatError::InvalidPipeline {
                reason: format!("non-monotonic journal sequence {} after {prev_seq} at line {line_no}", record.seq),
            });
        }
        prev_seq = record.seq;
        if let Some(marker) = extract_matching_snapshot_marker(&record.command_json, snapshot.as_deref()) {
            marker_covered_seq = Some(marker_covered_seq.map_or(marker.covered_seq, |existing| existing.max(marker.covered_seq)));
        }
        parsed_records.push((raw_line.to_vec(), record));
    }

    let effective_covered_seq = match (backend_covered_seq, marker_covered_seq) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    let mut commands = Vec::with_capacity(parsed_records.len());
    let mut seqs = Vec::with_capacity(parsed_records.len());
    let mut stripped_covered_records = false;
    let covered_floor = effective_covered_seq.unwrap_or(0);

    for (raw_line, record) in parsed_records {
        if is_snapshot_marker(&record.command_json) || record.seq <= covered_floor {
            stripped_covered_records = true;
            continue;
        }
        commands.push(record.command_json);
        seqs.push(record.seq);
        cleaned_bytes.extend_from_slice(&raw_line);
        cleaned_bytes.push(b'\n');
    }

    let total_discarded = discarded_corrupt_tail_lines.saturating_add(discarded_corrupt_interior_lines);
    if total_discarded > 0 || stripped_covered_records {
        backend.replace_journal_bytes_atomic(&cleaned_bytes)?;
    }

    let last_seq = prev_seq.max(covered_floor);
    Ok(RecoveredJournalState {
        snapshot,
        snapshot_covered_seq: effective_covered_seq,
        commands,
        seqs,
        discarded_corrupt_tail_lines,
        discarded_corrupt_interior_lines,
        warnings,
        last_seq,
    })
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

fn parse_journal_line(line_no: usize, raw_line: &[u8]) -> Result<JournalOpRecord, FormatError> {
    let text =
        std::str::from_utf8(raw_line).map_err(|err| FormatError::Json(format!("invalid UTF-8 in {JOURNAL_FILE_NAME} line {line_no}: {err}")))?;

    match serde_json::from_str::<JournalOpRecord>(text) {
        Ok(record) => {
            record.validate_version()?;
            Ok(record)
        }
        Err(err) => {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
                && let Some(raw_ver) = value.get("format_version").and_then(serde_json::Value::as_u64)
            {
                let version = u32::try_from(raw_ver).unwrap_or(u32::MAX);
                if version == 0 || version > CURRENT_JOURNAL_VERSION {
                    return Err(FormatError::UnsupportedSchemaVersion { version, max_supported: CURRENT_JOURNAL_VERSION });
                }
            }
            Err(FormatError::Json(format!("malformed entry in {JOURNAL_FILE_NAME} line {line_no}: {err}")))
        }
    }
}
