//! Crash-recovery write-ahead journal, atomic snapshots, and lease locking (`G2`).
//!
//! Provides two parity backends behind [`JournalBackend`]:
//! - [`MemoryJournalBackend`]: zero-OS-call in-memory model with JSON export/import
//!   (`to_web_storage_json` / `from_web_storage_json`) matching browser `localStorage` + Web Locks
//!   so `wasm32` targets share identical recovery behavior.
//! - [`DirectoryJournalBackend`] (`#[cfg(not(target_arch = "wasm32"))]`): native directory storage
//!   using exclusive lockfile creation (`curvebook.lock`), atomic temp-file + rename snapshots
//!   (`snapshot.curvebook`), and append-only NDJSON command records (`ops.journal`).

use serde::{Deserialize, Serialize};

use crate::FormatError;

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, OpenOptions};
#[cfg(not(target_arch = "wasm32"))]
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Current persisted format version of [`JournalOpRecord`] entries in `ops.journal`.
pub const CURRENT_JOURNAL_VERSION: u32 = 2;
/// Current persisted format version of [`JournalLockLease`] files (`curvebook.lock`).
pub const CURRENT_LOCK_VERSION: u32 = 1;

/// Check whether `lease` may be reclaimed because its TTL expired or its same-host local PID died.
#[must_use]
pub fn is_lease_reclaimable(lease: &JournalLockLease, now_epoch_ms: u64) -> bool {
    lease.is_stale(now_epoch_ms) || is_dead_same_host_owner(&lease.owner_id, lease.host_id.as_deref())
}

/// Return `true` only when `lease_host_id` is present, matches the current machine's
/// [`curvebook_platform::local_host_fingerprint`], and `owner_id` encodes a dead local
/// `curvebook-pid-<N>` process.
///
/// Whenever `lease_host_id` is `None` (unknown/legacy lockfile) or differs from the current host
/// (such as a shared network directory), returns `false` so callers rely strictly on TTL expiry.
#[must_use]
pub fn is_dead_same_host_owner(owner_id: &str, lease_host_id: Option<&str>) -> bool {
    let Some(lease_host) = lease_host_id.map(str::trim).filter(|h| !h.is_empty()) else {
        return false;
    };
    let Some(current_host) = curvebook_platform::local_host_fingerprint() else {
        return false;
    };
    if lease_host != current_host {
        return false;
    }
    is_dead_local_owner(owner_id)
}

/// Return `true` only when `owner_id` encodes a local `curvebook-pid-<N>` owner whose OS
/// process is verified absent (`ESRCH` / missing `/proc/<pid>`), falling back to `false` (TTL)
/// whenever uncertain or on unsupported platforms.
#[must_use]
pub fn is_dead_local_owner(owner_id: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(pid) = parse_local_curvebook_pid(owner_id) else {
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
fn parse_local_curvebook_pid(owner_id: &str) -> Option<u32> {
    let rest = owner_id.strip_prefix("curvebook-pid-")?;
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
    let Ok(output) = curvebook_platform::ProcessCommand::new("/bin/kill").args(["-0", &pid_arg]).env("LC_ALL", "C").run() else {
        return false;
    };
    if output.success {
        return false;
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    stderr.contains("no such process") || stderr.contains("esrch")
}

/// Filename of the exclusive writer lease file on native backends.
pub const LOCKFILE_NAME: &str = "curvebook.lock";
/// Filename of the baseline atomic project snapshot on native backends.
pub const SNAPSHOT_FILE_NAME: &str = "snapshot.curvebook";
/// Filename of the append-only NDJSON recovery journal on native backends.
pub const JOURNAL_FILE_NAME: &str = "ops.journal";

/// Maximum serialized web-storage JSON payload size accepted by [`MemoryJournalBackend`] (64 MiB).
pub const MAX_WEB_STORAGE_JSON_BYTES: usize = 64 * 1024 * 1024;
/// Default maximum `ops.journal` size in bytes allowed during recording and recovery replay (32 MiB).
pub const MAX_JOURNAL_BYTES: usize = 32 * 1024 * 1024;
/// Default maximum single NDJSON record line length in bytes in `ops.journal` (16 MiB).
pub const MAX_JOURNAL_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Default maximum number of non-empty records in `ops.journal` allowed during recording and replay.
pub const MAX_JOURNAL_RECORDS: usize = 65_536;

/// Resource limits enforced on WAL journal recording and crash-recovery replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalLimits {
    /// Maximum cumulative byte size of `ops.journal`.
    pub max_bytes: usize,
    /// Maximum byte length of a single line in `ops.journal`.
    pub max_line_bytes: usize,
    /// Maximum number of non-empty lines/records in `ops.journal`.
    pub max_records: usize,
}

impl Default for JournalLimits {
    fn default() -> Self {
        Self { max_bytes: MAX_JOURNAL_BYTES, max_line_bytes: MAX_JOURNAL_LINE_BYTES, max_records: MAX_JOURNAL_RECORDS }
    }
}

#[cfg(not(target_arch = "wasm32"))]
const EVICTION_LOCK_FILE_NAME: &str = ".curvebook.lock.evict";
#[cfg(not(target_arch = "wasm32"))]
const EVICTION_LOCK_TTL_MS: u64 = 5_000;
#[cfg(not(target_arch = "wasm32"))]
const MAX_LOCK_ATTEMPTS: usize = 32;
#[cfg(not(target_arch = "wasm32"))]
const LOCK_CONTENTION_BACKOFF_MS: u64 = 2;
#[cfg(not(target_arch = "wasm32"))]
const MAX_CORRUPT_GRACE_ATTEMPTS: usize = 12;
#[cfg(not(target_arch = "wasm32"))]
const CORRUPT_GRACE_BACKOFF_MS: u64 = 5;
#[cfg(not(target_arch = "wasm32"))]
const FRESH_CORRUPT_LOCK_AGE_MS: u64 = 500;
#[cfg(not(target_arch = "wasm32"))]
const MAX_LOCKFILE_BYTES: usize = 64 * 1024;
#[cfg(not(target_arch = "wasm32"))]
static LOCK_TMP_SEQ: AtomicU64 = AtomicU64::new(1);

fn default_v1() -> u32 {
    1
}

/// Exclusive writer lease stored in `curvebook.lock` or serialized web-storage state.
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
    /// Optional host/machine fingerprint recorded when the lease was created, restricting local
    /// PID dead-owner reclamation to leases created on the same host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
}

impl JournalLockLease {
    /// Construct a validated lease for `owner_id`, tagging `host_id` with the local host fingerprint when available.
    pub fn new(owner_id: impl Into<String>, acquired_epoch_ms: u64, ttl_ms: u64) -> Result<Self, FormatError> {
        let owner_id = owner_id.into();
        if owner_id.trim().is_empty() {
            return Err(FormatError::Io(String::from("journal lock owner_id must not be empty")));
        }
        Ok(Self { format_version: CURRENT_LOCK_VERSION, owner_id, acquired_epoch_ms, ttl_ms, host_id: curvebook_platform::local_host_fingerprint() })
    }

    /// Override or clear the optional `host_id` machine fingerprint on this lease.
    #[must_use]
    pub fn with_host_id(mut self, host_id: Option<String>) -> Self {
        self.host_id = host_id.and_then(|raw| {
            let trimmed = raw.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        });
        self
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
        match serde_json::from_slice::<Self>(bytes) {
            Ok(mut lease) => {
                lease.validate_version()?;
                if lease.owner_id.trim().is_empty() {
                    return Err(FormatError::Json(String::from("journal lock owner_id must not be empty")));
                }
                if lease.host_id.as_deref().is_some_and(|h| h.trim().is_empty()) {
                    lease.host_id = None;
                }
                Ok(lease)
            }
            Err(err) => {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
                    && let Some(raw_ver) = value.get("format_version").and_then(serde_json::Value::as_u64)
                {
                    let version = u32::try_from(raw_ver).unwrap_or(u32::MAX);
                    if version == 0 || version > CURRENT_LOCK_VERSION {
                        return Err(FormatError::UnsupportedSchemaVersion { version, max_supported: CURRENT_LOCK_VERSION });
                    }
                }
                Err(FormatError::from(err))
            }
        }
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
    /// Persisted schema version of this journal record (`1` for legacy entries, `2` for canonical).
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

    /// Verify raw record CRC first, then migrate older journal records (`format_version == 1`)
    /// to [`CURRENT_JOURNAL_VERSION`] in memory by normalizing retired parameter spellings and
    /// recomputing `crc32` over `(seq, upgraded_command_json)`.
    ///
    /// Returns `Ok(true)` if an upgrade occurred and `Ok(false)` when already at the current version.
    pub fn upgrade_to_current(&mut self) -> Result<bool, FormatError> {
        self.validate_version()?;
        self.verify_crc()?;
        if self.format_version == CURRENT_JOURNAL_VERSION {
            return Ok(false);
        }
        self.upgrade_verified_to_current()?;
        Ok(true)
    }

    fn upgrade_verified_to_current(&mut self) -> Result<(), FormatError> {
        let raw_cmd = std::mem::replace(&mut self.command_json, serde_json::Value::Null);
        let upgraded = crate::versioned::upgrade_journal_command_json(raw_cmd, self.format_version, CURRENT_JOURNAL_VERSION)?;
        self.crc32 = compute_record_crc32(self.seq, &upgraded)?;
        self.command_json = upgraded;
        self.format_version = CURRENT_JOURNAL_VERSION;
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
/// Staging filename written before replacing `snapshot.curvebook` so crashes during snapshot
/// replacement still keep the covered journal sequence paired with the snapshot checksum.
#[cfg(not(target_arch = "wasm32"))]
const SNAPSHOT_PENDING_META_FILE_NAME: &str = "snapshot.pending.meta.json";
#[cfg(not(target_arch = "wasm32"))]
const CURRENT_SNAPSHOT_META_VERSION: u32 = 1;
/// Sentinel key embedded in legacy-backend fallback checkpoint markers inside `ops.journal`.
const SNAPSHOT_MARKER_KEY: &str = "__curvebook_snapshot_marker__";

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
    /// Baseline snapshot payload read from `snapshot.curvebook`, if present.
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

    /// Write `snapshot.curvebook` atomically so readers never observe partial snapshot bytes.
    fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError>;

    /// Atomically replace `snapshot.curvebook` and bind `covered_seq` to the new snapshot bytes.
    fn write_snapshot_with_seq(&mut self, bytes: &[u8], covered_seq: u64) -> Result<(), FormatError> {
        let _ = covered_seq;
        self.write_snapshot_atomic(bytes)
    }

    /// Read baseline `snapshot.curvebook` bytes if present.
    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError>;

    /// Read the highest journal sequence covered by the persisted snapshot, if known.
    fn read_snapshot_covered_seq(&self) -> Result<Option<u64>, FormatError> {
        Ok(None)
    }

    /// Read the highest journal sequence covered by `snapshot_bytes` without re-reading the
    /// persisted snapshot file when supported by the backend.
    ///
    /// Existing [`JournalBackend`] implementations fall back to [`Self::read_snapshot_covered_seq`].
    fn read_snapshot_covered_seq_for(&self, snapshot_bytes: &[u8]) -> Result<Option<u64>, FormatError> {
        let _ = snapshot_bytes;
        self.read_snapshot_covered_seq()
    }

    /// Clear persisted snapshot-covered sequence metadata once the journal has been cleared cleanly.
    fn clear_snapshot_covered_seq(&mut self) -> Result<(), FormatError> {
        Ok(())
    }

    /// Append a single NDJSON journal entry line to `ops.journal`.
    fn append_journal_line(&mut self, ndjson_line: &str) -> Result<(), FormatError>;

    /// Read raw bytes from `ops.journal` (empty vector if absent).
    fn read_journal_bytes(&self) -> Result<Vec<u8>, FormatError>;

    /// Open a bounded streaming reader over `ops.journal` when supported by the backend.
    ///
    /// Returning `Ok(None)` falls back to [`Self::read_journal_bytes`].
    fn open_journal_stream(&self, max_bytes: usize) -> Result<Option<Box<dyn std::io::BufRead + '_>>, FormatError> {
        let _ = max_bytes;
        Ok(None)
    }

    /// Drain non-fatal recovery warnings recorded during backend operations such as reclaiming a
    /// corrupt lockfile.
    fn take_recovery_warnings(&mut self) -> Vec<JournalRecoveryWarning> {
        Vec::new()
    }

    /// Atomically replace `ops.journal` bytes (for example after stripping damaged lines).
    fn replace_journal_bytes_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError>;

    /// Remove or empty `ops.journal` on checkpoint or clean close.
    fn clear_journal(&mut self) -> Result<(), FormatError>;
}

const BASE64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encoded_len(raw_len: usize) -> usize {
    raw_len.div_ceil(3).saturating_mul(4)
}

fn encode_base64(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(base64_encoded_len(bytes.len()));
    let (chunks, remainder) = bytes.as_chunks::<3>();
    for &[b0, b1, b2] in chunks {
        let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        push_base64_char(&mut out, (triple >> 18) & 0x3F);
        push_base64_char(&mut out, (triple >> 12) & 0x3F);
        push_base64_char(&mut out, (triple >> 6) & 0x3F);
        push_base64_char(&mut out, triple & 0x3F);
    }
    match remainder {
        [b0] => {
            let v = u32::from(*b0) << 16;
            push_base64_char(&mut out, (v >> 18) & 0x3F);
            push_base64_char(&mut out, (v >> 12) & 0x3F);
            out.push(b'=');
            out.push(b'=');
        }
        [b0, b1] => {
            let v = (u32::from(*b0) << 16) | (u32::from(*b1) << 8);
            push_base64_char(&mut out, (v >> 18) & 0x3F);
            push_base64_char(&mut out, (v >> 12) & 0x3F);
            push_base64_char(&mut out, (v >> 6) & 0x3F);
            out.push(b'=');
        }
        _ => {}
    }
    String::from_utf8(out).unwrap_or_default()
}

fn push_base64_char(out: &mut Vec<u8>, idx: u32) {
    if let Some(&ch) = BASE64_ALPHABET.get(idx as usize) {
        out.push(ch);
    }
}

fn decode_base64_char(byte: u8) -> Result<u8, &'static str> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("invalid base64 character"),
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, &'static str> {
    let raw = input.as_bytes();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    if !raw.len().is_multiple_of(4) {
        return Err("base64 length must be a multiple of 4");
    }
    let pad = if raw.ends_with(b"==") {
        2usize
    } else if raw.ends_with(b"=") {
        1usize
    } else {
        0usize
    };
    let num_quads = raw.len() / 4;
    let mut out = Vec::with_capacity(num_quads.saturating_mul(3).saturating_sub(pad));
    let (quads, _) = raw.as_chunks::<4>();
    for (quad_idx, &[q0, q1, q2, q3]) in quads.iter().enumerate() {
        let is_last = quad_idx + 1 == num_quads;
        let a = decode_base64_char(q0)?;
        let b = decode_base64_char(q1)?;
        if q2 == b'=' {
            if !is_last || q3 != b'=' || (b & 0x0F) != 0 {
                return Err("invalid base64 padding");
            }
            out.push((a << 2) | (b >> 4));
        } else if q3 == b'=' {
            if !is_last {
                return Err("unexpected base64 padding");
            }
            let c = decode_base64_char(q2)?;
            if (c & 0x03) != 0 {
                return Err("non-canonical trailing base64 bits");
            }
            out.push((a << 2) | (b >> 4));
            out.push((b << 4) | (c >> 2));
        } else {
            let c = decode_base64_char(q2)?;
            let d = decode_base64_char(q3)?;
            out.push((a << 2) | (b >> 4));
            out.push((b << 4) | (c >> 2));
            out.push((c << 6) | d);
        }
    }
    Ok(out)
}

mod serde_compact_bytes {
    use std::fmt;

    use serde::de::{SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};

    use super::{MAX_WEB_STORAGE_JSON_BYTES, decode_base64, encode_base64};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode_base64(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        struct CompactBytesVisitor;

        impl<'de> Visitor<'de> for CompactBytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a base64 string or array of byte integers")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                decode_base64(value).map_err(serde::de::Error::custom)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let cap = seq.size_hint().unwrap_or(0).min(MAX_WEB_STORAGE_JSON_BYTES);
                let mut out = Vec::with_capacity(cap);
                while let Some(byte) = seq.next_element::<u8>()? {
                    out.push(byte);
                }
                Ok(out)
            }
        }

        deserializer.deserialize_any(CompactBytesVisitor)
    }
}

mod serde_opt_compact_bytes {
    use std::fmt;

    use serde::Serializer;
    use serde::de::{Deserializer, Visitor};

    use super::serde_compact_bytes;

    pub(super) fn serialize<S: Serializer>(value: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => serde_compact_bytes::serialize(bytes, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error> {
        struct OptCompactBytesVisitor;

        impl<'de> Visitor<'de> for OptCompactBytesVisitor {
            type Value = Option<Vec<u8>>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("null, a base64 string, or an array of byte integers")
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_some<D2: Deserializer<'de>>(self, deserializer: D2) -> Result<Self::Value, D2::Error> {
                serde_compact_bytes::deserialize(deserializer).map(Some)
            }
        }

        deserializer.deserialize_option(OptCompactBytesVisitor)
    }
}

/// Pure in-memory journal backend with zero OS calls, plus web-storage JSON import/export.
///
/// Models browser `localStorage` + Web Locks lease semantics identically on `wasm32` and native.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryJournalBackend {
    #[serde(default)]
    lease: Option<JournalLockLease>,
    #[serde(default, with = "serde_opt_compact_bytes")]
    snapshot: Option<Vec<u8>>,
    #[serde(default)]
    snapshot_covered_seq: Option<u64>,
    #[serde(default, with = "serde_compact_bytes")]
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
        let snapshot_b64_len = self.snapshot.as_ref().map_or(0, |bytes| base64_encoded_len(bytes.len()));
        let journal_b64_len = base64_encoded_len(self.journal_bytes.len());
        if snapshot_b64_len.saturating_add(journal_b64_len) > MAX_WEB_STORAGE_JSON_BYTES {
            return Err(FormatError::Io(format!("web-storage journal payload exceeds budget of {MAX_WEB_STORAGE_JSON_BYTES} bytes")));
        }
        let json = serde_json::to_string(self)?;
        if json.len() > MAX_WEB_STORAGE_JSON_BYTES {
            return Err(FormatError::Io(format!(
                "web-storage journal payload ({} bytes) exceeds budget of {MAX_WEB_STORAGE_JSON_BYTES} bytes",
                json.len()
            )));
        }
        Ok(json)
    }

    /// Restore a [`MemoryJournalBackend`] from a JSON string previously created by
    /// [`Self::to_web_storage_json`].
    pub fn from_web_storage_json(json: &str) -> Result<Self, FormatError> {
        if json.len() > MAX_WEB_STORAGE_JSON_BYTES {
            return Err(FormatError::Io(format!(
                "web-storage journal payload ({} bytes) exceeds budget of {MAX_WEB_STORAGE_JSON_BYTES} bytes",
                json.len()
            )));
        }
        let backend: Self = serde_json::from_str(json)?;
        if let Some(lease) = &backend.lease {
            lease.validate_version()?;
            if lease.owner_id.trim().is_empty() {
                return Err(FormatError::Json(String::from("journal lock owner_id must not be empty")));
            }
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
        if lease.owner_id.trim().is_empty() {
            return Err(FormatError::Io(String::from("journal lock owner_id must not be empty")));
        }
        let evicted = match &self.lease {
            Some(existing) => {
                existing.validate_version()?;
                if existing.owner_id.trim().is_empty() {
                    None
                } else if !is_lease_reclaimable(existing, now_epoch_ms) {
                    return Err(FormatError::Io(format!(
                        "journal lock `{LOCKFILE_NAME}` is actively held by `{}` until {}ms",
                        existing.owner_id,
                        existing.expires_epoch_ms()
                    )));
                } else {
                    Some(existing.clone())
                }
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

    fn open_journal_stream(&self, max_bytes: usize) -> Result<Option<Box<dyn std::io::BufRead + '_>>, FormatError> {
        if self.journal_bytes.len() > max_bytes {
            return Err(FormatError::InvalidPipeline {
                reason: format!("journal size {} bytes exceeds maximum replay limit of {max_bytes} bytes", self.journal_bytes.len()),
            });
        }
        Ok(Some(Box::new(std::io::Cursor::new(self.journal_bytes.as_slice()))))
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

/// Native filesystem crash-recovery backend storing `curvebook.lock`, `snapshot.curvebook`, and
/// `ops.journal` inside a dedicated recovery directory.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct DirectoryJournalBackend {
    dir: PathBuf,
    pending_warnings: Vec<JournalRecoveryWarning>,
}

#[cfg(not(target_arch = "wasm32"))]
struct TempFileUnlinkGuard {
    path: PathBuf,
    armed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl TempFileUnlinkGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempFileUnlinkGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EvictionLockRecord {
    owner_id: String,
    acquired_wall_ms: u64,
    ttl_ms: u64,
    nonce: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_id: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
struct EvictionLockGuard {
    path: PathBuf,
    expected_bytes: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for EvictionLockGuard {
    fn drop(&mut self) {
        if let Ok(current) = std::fs::read(&self.path)
            && current == self.expected_bytes
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum LockfileState {
    Missing,
    Active(JournalLockLease),
    FutureVersion { version: u32, max_supported: u32 },
    Corrupt(String),
}

#[cfg(not(target_arch = "wasm32"))]
enum BoundedLockRead {
    Missing,
    Oversized(u64),
    Bytes(Vec<u8>),
}

#[cfg(not(target_arch = "wasm32"))]
fn current_wall_epoch_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bounded_lock_bytes(path: &Path, max_bytes: usize) -> Result<BoundedLockRead, FormatError> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(BoundedLockRead::Missing),
        Err(err) => return Err(FormatError::from(err)),
    };
    let file_len = file.metadata()?.len();
    let max_bytes_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if file_len > max_bytes_u64 {
        return Ok(BoundedLockRead::Oversized(file_len));
    }
    let cap = usize::try_from(file_len).unwrap_or(0).min(max_bytes);
    let mut out = Vec::with_capacity(cap);
    file.take(max_bytes_u64.saturating_add(1)).read_to_end(&mut out)?;
    if out.len() > max_bytes {
        return Ok(BoundedLockRead::Oversized(u64::try_from(out.len()).unwrap_or(u64::MAX)));
    }
    Ok(BoundedLockRead::Bytes(out))
}

#[cfg(not(target_arch = "wasm32"))]
impl DirectoryJournalBackend {
    /// Open or create a filesystem journal directory at `dir`.
    pub fn open_dir(dir: impl AsRef<Path>) -> Result<Self, FormatError> {
        let dir_path = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir_path)?;
        Ok(Self { dir: dir_path, pending_warnings: Vec::new() })
    }

    /// Root directory backing this journal.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Full path to `curvebook.lock`.
    pub fn lock_path(&self) -> PathBuf {
        self.dir.join(LOCKFILE_NAME)
    }

    /// Full path to `snapshot.curvebook`.
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

    fn eviction_lock_path(&self) -> PathBuf {
        self.dir.join(EVICTION_LOCK_FILE_NAME)
    }

    fn next_unique_sibling_path(&self, stem: &str) -> PathBuf {
        let seq = LOCK_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        self.dir.join(format!(".{stem}.tmp.{}.{seq}", std::process::id()))
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

    fn try_create_file_exclusively(&self, target_path: &Path, stem: &str, bytes: &[u8], durable: bool) -> Result<bool, FormatError> {
        let tmp_path = self.next_unique_sibling_path(stem);
        let tmp_guard = TempFileUnlinkGuard::new(tmp_path.clone());
        {
            let mut tmp_file = OpenOptions::new().write(true).create_new(true).open(&tmp_path)?;
            tmp_file.write_all(bytes)?;
            if durable {
                tmp_file.sync_all()?;
            }
        }

        match std::fs::hard_link(&tmp_path, target_path) {
            Ok(()) => {
                drop(tmp_guard);
                if durable {
                    curvebook_platform::fs::sync_dir(&self.dir)?;
                }
                Ok(true)
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                drop(tmp_guard);
                Ok(false)
            }
            Err(err) if matches!(err.kind(), ErrorKind::Unsupported | ErrorKind::PermissionDenied) => {
                // Fallback for filesystems without hard-link support (e.g. exFAT/FAT32):
                // clean up the staging temp file and fall back to exclusive file creation.
                drop(tmp_guard);
                match OpenOptions::new().write(true).create_new(true).open(target_path) {
                    Ok(mut file) => {
                        let write_res = if durable { file.write_all(bytes).and_then(|()| file.sync_all()) } else { file.write_all(bytes) };
                        drop(file);
                        if let Err(write_err) = write_res {
                            let _ = std::fs::remove_file(target_path);
                            return Err(FormatError::from(write_err));
                        }
                        if durable {
                            curvebook_platform::fs::sync_dir(&self.dir)?;
                        }
                        match read_bounded_lock_bytes(target_path, MAX_LOCKFILE_BYTES)? {
                            BoundedLockRead::Bytes(actual) if actual == bytes => Ok(true),
                            _ => Ok(false),
                        }
                    }
                    Err(create_err) if create_err.kind() == ErrorKind::AlreadyExists => Ok(false),
                    Err(create_err) => Err(FormatError::from(create_err)),
                }
            }
            Err(err) => {
                drop(tmp_guard);
                Err(FormatError::from(err))
            }
        }
    }

    fn inspect_lockfile(&self) -> Result<LockfileState, FormatError> {
        let bytes = match read_bounded_lock_bytes(&self.lock_path(), MAX_LOCKFILE_BYTES)? {
            BoundedLockRead::Missing => return Ok(LockfileState::Missing),
            BoundedLockRead::Oversized(len) => {
                return Ok(LockfileState::Corrupt(format!("lockfile size {len} bytes exceeds maximum {MAX_LOCKFILE_BYTES} bytes")));
            }
            BoundedLockRead::Bytes(bytes) => bytes,
        };
        match JournalLockLease::from_json_bytes(&bytes) {
            Ok(lease) => Ok(LockfileState::Active(lease)),
            Err(FormatError::UnsupportedSchemaVersion { version, max_supported }) => Ok(LockfileState::FutureVersion { version, max_supported }),
            Err(FormatError::Json(msg)) => Ok(LockfileState::Corrupt(msg)),
            Err(err) => Err(err),
        }
    }

    fn reclaim_stale_eviction_lock(&self, evict_path: &Path, observed_bytes: &[u8]) -> Result<(), FormatError> {
        let reclaim_path = self.next_unique_sibling_path("curvebook.lock.evict.reclaim");
        match std::fs::rename(evict_path, &reclaim_path) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(FormatError::from(err)),
        }
        let mut reclaim_guard = TempFileUnlinkGuard::new(reclaim_path.clone());
        let actual_bytes = match std::fs::read(&reclaim_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(FormatError::from(err)),
        };
        if actual_bytes != observed_bytes {
            // Another contender installed a fresh guard just before rename; restore best-effort without clobbering.
            let _ = std::fs::hard_link(&reclaim_path, evict_path);
        }
        let _ = std::fs::remove_file(&reclaim_path);
        reclaim_guard.disarm();
        Ok(())
    }

    fn try_acquire_eviction_guard(&self, owner_id: &str) -> Result<Option<EvictionLockGuard>, FormatError> {
        let evict_path = self.eviction_lock_path();
        let nonce = LOCK_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let record = EvictionLockRecord {
            owner_id: owner_id.to_owned(),
            acquired_wall_ms: current_wall_epoch_ms(),
            ttl_ms: EVICTION_LOCK_TTL_MS,
            nonce,
            host_id: curvebook_platform::local_host_fingerprint(),
        };
        let guard_bytes = serde_json::to_vec(&record)?;

        if self.try_create_file_exclusively(&evict_path, "curvebook.lock.evict", &guard_bytes, false)? {
            return Ok(Some(EvictionLockGuard { path: evict_path, expected_bytes: guard_bytes }));
        }

        let observed_bytes = match read_bounded_lock_bytes(&evict_path, MAX_LOCKFILE_BYTES)? {
            BoundedLockRead::Missing => return Ok(None),
            BoundedLockRead::Oversized(_) => {
                let raw = Vec::new();
                self.reclaim_stale_eviction_lock(&evict_path, &raw)?;
                return Ok(None);
            }
            BoundedLockRead::Bytes(bytes) => bytes,
        };

        let is_reclaimable_guard = match serde_json::from_slice::<EvictionLockRecord>(&observed_bytes) {
            Ok(existing) => {
                is_dead_same_host_owner(&existing.owner_id, existing.host_id.as_deref())
                    || current_wall_epoch_ms() >= existing.acquired_wall_ms.saturating_add(existing.ttl_ms)
            }
            Err(_) => true,
        };
        if is_reclaimable_guard {
            self.reclaim_stale_eviction_lock(&evict_path, &observed_bytes)?;
        }
        Ok(None)
    }

    fn acquire_eviction_guard_with_retry(&self, owner_id: &str, timeout_msg: &str) -> Result<EvictionLockGuard, FormatError> {
        for attempt in 0..MAX_LOCK_ATTEMPTS {
            if let Some(guard) = self.try_acquire_eviction_guard(owner_id)? {
                return Ok(guard);
            }
            if attempt + 1 < MAX_LOCK_ATTEMPTS {
                std::thread::sleep(Duration::from_millis(LOCK_CONTENTION_BACKOFF_MS));
            }
        }
        Err(FormatError::Io(String::from(timeout_msg)))
    }

    fn evict_lock_inside_guard(&mut self, lease_bytes: &[u8], now_epoch_ms: u64) -> Result<Option<Option<JournalLockLease>>, FormatError> {
        match self.inspect_lockfile()? {
            LockfileState::Missing => {
                if self.try_create_file_exclusively(&self.lock_path(), LOCKFILE_NAME, lease_bytes, true)? {
                    return Ok(Some(None));
                }
                Ok(None)
            }
            LockfileState::FutureVersion { version, max_supported } => Err(FormatError::UnsupportedSchemaVersion { version, max_supported }),
            LockfileState::Active(current) => {
                if !is_lease_reclaimable(&current, now_epoch_ms) {
                    return Err(FormatError::Io(format!(
                        "journal lock `{LOCKFILE_NAME}` is actively held by `{}` until {}ms",
                        current.owner_id,
                        current.expires_epoch_ms()
                    )));
                }
                curvebook_platform::fs::write_atomic(&self.lock_path(), lease_bytes)?;
                Ok(Some(Some(current)))
            }
            LockfileState::Corrupt(err_msg) => {
                curvebook_platform::fs::write_atomic(&self.lock_path(), lease_bytes)?;
                self.pending_warnings.push(JournalRecoveryWarning {
                    line: 0,
                    kind: String::from("corrupt_lockfile"),
                    message: format!("reclaimed corrupt journal lock `{LOCKFILE_NAME}`: {err_msg}"),
                });
                Ok(Some(None))
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn is_fresh_corrupt_lockfile(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    SystemTime::now().duration_since(modified).is_ok_and(|age| age < Duration::from_millis(FRESH_CORRUPT_LOCK_AGE_MS))
}

#[cfg(not(target_arch = "wasm32"))]
impl JournalBackend for DirectoryJournalBackend {
    fn acquire_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<Option<JournalLockLease>, FormatError> {
        lease.validate_version()?;
        if lease.owner_id.trim().is_empty() {
            return Err(FormatError::Io(String::from("journal lock owner_id must not be empty")));
        }
        self.pending_warnings.clear();
        let lease_bytes = serde_json::to_vec(lease)?;
        let mut corrupt_grace_attempts = 0usize;

        for attempt in 0..MAX_LOCK_ATTEMPTS {
            if self.try_create_file_exclusively(&self.lock_path(), LOCKFILE_NAME, &lease_bytes, true)? {
                return Ok(None);
            }

            match self.inspect_lockfile()? {
                LockfileState::Missing => continue,
                LockfileState::FutureVersion { version, max_supported } => {
                    return Err(FormatError::UnsupportedSchemaVersion { version, max_supported });
                }
                LockfileState::Active(existing) if !is_lease_reclaimable(&existing, now_epoch_ms) => {
                    return Err(FormatError::Io(format!(
                        "journal lock `{LOCKFILE_NAME}` is actively held by `{}` until {}ms",
                        existing.owner_id,
                        existing.expires_epoch_ms()
                    )));
                }
                LockfileState::Corrupt(_) if corrupt_grace_attempts < MAX_CORRUPT_GRACE_ATTEMPTS && is_fresh_corrupt_lockfile(&self.lock_path()) => {
                    corrupt_grace_attempts += 1;
                    std::thread::sleep(Duration::from_millis(CORRUPT_GRACE_BACKOFF_MS));
                    continue;
                }
                LockfileState::Active(_) | LockfileState::Corrupt(_) => {
                    let Some(guard) = self.try_acquire_eviction_guard(&lease.owner_id)? else {
                        if attempt + 1 < MAX_LOCK_ATTEMPTS {
                            std::thread::sleep(Duration::from_millis(LOCK_CONTENTION_BACKOFF_MS));
                            continue;
                        }
                        return Err(FormatError::Io(String::from("stale journal lock was claimed concurrently by another writer")));
                    };
                    let outcome = self.evict_lock_inside_guard(&lease_bytes, now_epoch_ms);
                    drop(guard);
                    if let Some(evicted) = outcome? {
                        return Ok(evicted);
                    }
                }
            }
        }

        Err(FormatError::Io(String::from("concurrent journal lock contention")))
    }

    fn renew_lease(&mut self, lease: &JournalLockLease, now_epoch_ms: u64) -> Result<(), FormatError> {
        lease.validate_version()?;
        let _guard = self.acquire_eviction_guard_with_retry(&lease.owner_id, "concurrent journal lock contention during renewal")?;
        let Some(existing) = self.read_lease()? else {
            return Err(FormatError::Io(String::from("cannot renew journal lock: lockfile missing")));
        };
        if existing.owner_id != lease.owner_id {
            return Err(FormatError::Io(format!("cannot renew journal lock owned by `{}` from `{}`", existing.owner_id, lease.owner_id)));
        }
        let mut refreshed = lease.clone();
        refreshed.acquired_epoch_ms = now_epoch_ms;
        let bytes = serde_json::to_vec(&refreshed)?;
        curvebook_platform::fs::write_atomic(&self.lock_path(), &bytes)?;
        Ok(())
    }

    fn release_lease(&mut self, owner_id: &str) -> Result<(), FormatError> {
        let _guard = self.acquire_eviction_guard_with_retry(owner_id, "concurrent journal lock contention during release")?;
        let Some(existing) = self.read_lease()? else {
            return Ok(());
        };
        if existing.owner_id != owner_id {
            return Err(FormatError::Io(format!("cannot release journal lock owned by `{}` from `{owner_id}`", existing.owner_id)));
        }
        match std::fs::remove_file(self.lock_path()) {
            Ok(()) => {
                curvebook_platform::fs::sync_dir(&self.dir)?;
                Ok(())
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(FormatError::from(err)),
        }
    }

    fn read_lease(&self) -> Result<Option<JournalLockLease>, FormatError> {
        match read_bounded_lock_bytes(&self.lock_path(), MAX_LOCKFILE_BYTES)? {
            BoundedLockRead::Missing => Ok(None),
            BoundedLockRead::Oversized(len) => {
                Err(FormatError::Json(format!("journal lock `{LOCKFILE_NAME}` size {len} bytes exceeds maximum {MAX_LOCKFILE_BYTES} bytes")))
            }
            BoundedLockRead::Bytes(bytes) => Ok(Some(JournalLockLease::from_json_bytes(&bytes)?)),
        }
    }

    fn write_snapshot_atomic(&mut self, bytes: &[u8]) -> Result<(), FormatError> {
        self.clear_snapshot_covered_seq()?;
        curvebook_platform::fs::write_atomic(&self.snapshot_path(), bytes)?;
        Ok(())
    }

    fn write_snapshot_with_seq(&mut self, bytes: &[u8], covered_seq: u64) -> Result<(), FormatError> {
        let snapshot_len = u64::try_from(bytes.len()).map_err(|_| FormatError::Io(String::from("snapshot length overflow")))?;
        let meta = SnapshotMeta { format_version: CURRENT_SNAPSHOT_META_VERSION, covered_seq, snapshot_crc32: crc32fast::hash(bytes), snapshot_len };
        let meta_bytes = serde_json::to_vec(&meta)?;
        let pending_path = self.pending_snapshot_meta_path();
        let meta_path = self.snapshot_meta_path();
        curvebook_platform::fs::write_atomic(&pending_path, &meta_bytes)?;
        curvebook_platform::fs::write_atomic(&self.snapshot_path(), bytes)?;
        std::fs::rename(&pending_path, &meta_path)?;
        curvebook_platform::fs::sync_dir(&self.dir)?;
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
        self.read_snapshot_covered_seq_for(&snapshot_bytes)
    }

    fn read_snapshot_covered_seq_for(&self, snapshot_bytes: &[u8]) -> Result<Option<u64>, FormatError> {
        let actual_crc = crc32fast::hash(snapshot_bytes);
        let actual_len = u64::try_from(snapshot_bytes.len()).map_err(|_| FormatError::Io(String::from("snapshot length overflow")))?;
        if let Some(seq) = self.read_matching_snapshot_meta(&self.snapshot_meta_path(), actual_crc, actual_len)? {
            return Ok(Some(seq));
        }
        self.read_matching_snapshot_meta(&self.pending_snapshot_meta_path(), actual_crc, actual_len)
    }

    fn clear_snapshot_covered_seq(&mut self) -> Result<(), FormatError> {
        let mut removed_any = false;
        for path in [self.snapshot_meta_path(), self.pending_snapshot_meta_path()] {
            match std::fs::remove_file(path) {
                Ok(()) => removed_any = true,
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => return Err(FormatError::from(err)),
            }
        }
        if removed_any {
            curvebook_platform::fs::sync_dir(&self.dir)?;
        }
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
        curvebook_platform::fs::write_atomic(&self.journal_path(), bytes)?;
        Ok(())
    }

    fn clear_journal(&mut self) -> Result<(), FormatError> {
        match std::fs::remove_file(self.journal_path()) {
            Ok(()) => {
                curvebook_platform::fs::sync_dir(&self.dir)?;
                Ok(())
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(FormatError::from(err)),
        }
    }

    fn take_recovery_warnings(&mut self) -> Vec<JournalRecoveryWarning> {
        std::mem::take(&mut self.pending_warnings)
    }

    fn open_journal_stream(&self, max_bytes: usize) -> Result<Option<Box<dyn std::io::BufRead + '_>>, FormatError> {
        let file = match std::fs::File::open(self.journal_path()) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Some(Box::new(std::io::empty()))),
            Err(err) => return Err(FormatError::from(err)),
        };
        let file_len = file.metadata()?.len();
        let max_bytes_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        if file_len > max_bytes_u64 {
            return Err(FormatError::Io(format!(
                "journal `{JOURNAL_FILE_NAME}` size ({file_len} bytes) exceeds maximum replay limit ({max_bytes} bytes)"
            )));
        }
        let limited = std::io::Read::take(file, max_bytes_u64.saturating_add(1));
        Ok(Some(Box::new(std::io::BufReader::new(limited))))
    }
}

/// Crash-recovery journal session managing exclusive lock acquisition, NDJSON command logging,
/// atomic snapshot checkpointing, torn-tail truncation, and clean shutdown.
#[derive(Debug)]
pub struct CrashRecoveryJournal<B: JournalBackend> {
    backend: B,
    lease: JournalLockLease,
    next_seq: u64,
    limits: JournalLimits,
    pending_record_count: usize,
    pending_journal_bytes: usize,
}

impl<B: JournalBackend> CrashRecoveryJournal<B> {
    /// Open a journal session, acquiring or recovering a stale lock lease, loading any baseline
    /// snapshot, validating CRC32 checksums of `ops.journal`, and safely truncating torn tail
    /// lines without crashing using [`JournalLimits::default`].
    pub fn open_or_recover(backend: B, owner_id: impl Into<String>, ttl_ms: u64, now_epoch_ms: u64) -> Result<(Self, RecoveryReport), FormatError> {
        Self::open_or_recover_with_limits(backend, owner_id, ttl_ms, now_epoch_ms, JournalLimits::default())
    }

    /// Open a journal session using explicit replay/size [`JournalLimits`].
    pub fn open_or_recover_with_limits(
        mut backend: B,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
        limits: JournalLimits,
    ) -> Result<(Self, RecoveryReport), FormatError> {
        let lease = JournalLockLease::new(owner_id, now_epoch_ms, ttl_ms)?;
        let recovered_from_stale_lock = backend.acquire_lease(&lease, now_epoch_ms)?;
        let lock_warnings = backend.take_recovery_warnings();

        let recovered_state = match recover_journal_contents(&mut backend, limits) {
            Ok(state) => state,
            Err(err) => {
                let _ = backend.release_lease(&lease.owner_id);
                return Err(err);
            }
        };

        let mut warnings = lock_warnings;
        warnings.extend(recovered_state.warnings);

        let next_seq = recovered_state.last_seq.saturating_add(1).max(1);
        let pending_record_count = recovered_state.remaining_record_count;
        let pending_journal_bytes = recovered_state.remaining_journal_bytes;
        let report = RecoveryReport {
            snapshot: recovered_state.snapshot,
            snapshot_covered_seq: recovered_state.snapshot_covered_seq,
            recovered_commands: recovered_state.commands,
            recovered_seqs: recovered_state.seqs,
            discarded_corrupt_tail_lines: recovered_state.discarded_corrupt_tail_lines,
            discarded_corrupt_interior_lines: recovered_state.discarded_corrupt_interior_lines,
            warnings,
            recovered_from_stale_lock,
            active_lease: lease.clone(),
        };
        Ok((Self { backend, lease, next_seq, limits, pending_record_count, pending_journal_bytes }, report))
    }

    /// Return the active [`JournalLimits`] enforced for replay and command appends.
    pub fn limits(&self) -> JournalLimits {
        self.limits
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
        if line.len() > self.limits.max_line_bytes {
            return Err(FormatError::Io(format!(
                "journal `{JOURNAL_FILE_NAME}` record size ({} bytes) exceeds maximum line limit ({} bytes)",
                line.len(),
                self.limits.max_line_bytes
            )));
        }
        let next_record_count = self.pending_record_count.checked_add(1).filter(|&count| count <= self.limits.max_records).ok_or_else(|| {
            FormatError::Io(format!("journal `{JOURNAL_FILE_NAME}` record count exceeds maximum limit ({} records)", self.limits.max_records))
        })?;
        let appended_bytes = if line.ends_with('\n') { line.len() } else { line.len().saturating_add(1) };
        let next_journal_bytes =
            self.pending_journal_bytes.checked_add(appended_bytes).filter(|&total| total <= self.limits.max_bytes).ok_or_else(|| {
                FormatError::Io(format!("journal `{JOURNAL_FILE_NAME}` size exceeds maximum replay limit ({} bytes)", self.limits.max_bytes))
            })?;
        self.backend.append_journal_line(&line)?;
        self.pending_record_count = next_record_count;
        self.pending_journal_bytes = next_journal_bytes;
        self.next_seq = seq.checked_add(1).ok_or_else(|| FormatError::Io(String::from("journal sequence number overflow")))?;
        Ok(seq)
    }

    fn publish_snapshot_and_optional_marker(&mut self, snapshot_bytes: &[u8]) -> Result<Option<PublishedSnapshotMarker>, FormatError> {
        let covered_seq = self.next_seq.saturating_sub(1);
        self.backend.write_snapshot_with_seq(snapshot_bytes, covered_seq)?;
        let backend_stored_seq = self.backend.read_snapshot_covered_seq_for(snapshot_bytes)? == Some(covered_seq);
        if covered_seq > 0 && !backend_stored_seq {
            let marker_seq = self.next_seq;
            let marker = build_snapshot_marker_record(marker_seq, covered_seq, snapshot_bytes)?;
            let line = marker.to_ndjson_line()?;
            self.backend.append_journal_line(&line)?;
            let line_bytes = if line.ends_with('\n') { line.len() } else { line.len().saturating_add(1) };
            return Ok(Some(PublishedSnapshotMarker { seq: marker_seq, line_bytes }));
        }
        Ok(None)
    }

    /// Persist an atomic snapshot checkpoint binding the covered journal sequence BEFORE
    /// truncating `ops.journal`.
    pub fn checkpoint(&mut self, snapshot_bytes: &[u8]) -> Result<(), FormatError> {
        self.ensure_lease_ownership("checkpoint journal")?;
        let appended_marker = self.publish_snapshot_and_optional_marker(snapshot_bytes)?;
        if let Err(err) = self.backend.clear_journal() {
            if let Some(marker) = appended_marker {
                self.next_seq = marker.seq.checked_add(1).ok_or_else(|| FormatError::Io(String::from("journal sequence number overflow")))?;
                self.pending_record_count = self.pending_record_count.saturating_add(1);
                self.pending_journal_bytes = self.pending_journal_bytes.saturating_add(marker.line_bytes);
            }
            return Err(err);
        }
        self.pending_record_count = 0;
        self.pending_journal_bytes = 0;
        Ok(())
    }

    /// Refresh the active writer lease timestamp to `now_epoch_ms`.
    pub fn renew_lease(&mut self, now_epoch_ms: u64) -> Result<(), FormatError> {
        self.backend.renew_lease(&self.lease, now_epoch_ms)?;
        self.lease.acquired_epoch_ms = now_epoch_ms;
        Ok(())
    }

    /// Release the active writer lease without altering `snapshot.curvebook` or `ops.journal`.
    pub fn release_lease(&mut self) -> Result<(), FormatError> {
        self.backend.release_lease(&self.lease.owner_id)
    }

    /// Write the final atomic snapshot, clear `ops.journal`, release `curvebook.lock`, and return
    /// the underlying backend.
    pub fn close_cleanly(mut self, final_snapshot_bytes: &[u8]) -> Result<B, FormatError> {
        self.ensure_lease_ownership("close journal cleanly")?;
        let _ = self.publish_snapshot_and_optional_marker(final_snapshot_bytes)?;
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

struct PublishedSnapshotMarker {
    seq: u64,
    line_bytes: usize,
}

struct LazySnapshotDigest<'a> {
    bytes: &'a [u8],
    digest: Option<(u64, u32)>,
}

impl<'a> LazySnapshotDigest<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, digest: None }
    }

    fn len_and_crc(&mut self) -> Option<(u64, u32)> {
        if let Some(cached) = self.digest {
            return Some(cached);
        }
        let len = u64::try_from(self.bytes.len()).ok()?;
        let crc = crc32fast::hash(self.bytes);
        self.digest = Some((len, crc));
        Some((len, crc))
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
    remaining_record_count: usize,
    remaining_journal_bytes: usize,
}

struct ParsedRecordSummary {
    seq: u64,
    crc32: u32,
    command_json: serde_json::Value,
}

struct StreamedJournalParse {
    records: Vec<ParsedRecordSummary>,
    discarded_corrupt_tail_lines: usize,
    discarded_corrupt_interior_lines: usize,
    warnings: Vec<JournalRecoveryWarning>,
    prev_seq: u64,
    marker_covered_seq: Option<u64>,
    total_bytes: usize,
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

fn extract_matching_snapshot_marker(
    command_json: &serde_json::Value,
    snapshot_digest: &mut Option<LazySnapshotDigest<'_>>,
) -> Option<SnapshotMarkerPayload> {
    let obj = command_json.as_object()?;
    let raw_marker = obj.get(SNAPSHOT_MARKER_KEY)?;
    let payload: SnapshotMarkerPayload = serde_json::from_value(raw_marker.clone()).ok()?;
    let (actual_len, actual_crc) = snapshot_digest.as_mut()?.len_and_crc()?;
    if payload.snapshot_len == actual_len && payload.snapshot_crc32 == actual_crc { Some(payload) } else { None }
}

fn flush_pending_bad_line(pending_bad: &mut Option<(usize, String)>, is_tail: bool, state: &mut StreamedJournalParse) {
    let Some((line_no, err_msg)) = pending_bad.take() else {
        return;
    };
    let (kind, bucket) = if is_tail {
        state.discarded_corrupt_tail_lines = state.discarded_corrupt_tail_lines.saturating_add(1);
        ("corrupt_tail_line", "tail")
    } else {
        state.discarded_corrupt_interior_lines = state.discarded_corrupt_interior_lines.saturating_add(1);
        ("corrupt_interior_line", "interior")
    };
    state.warnings.push(JournalRecoveryWarning {
        line: line_no,
        kind: String::from(kind),
        message: format!("quarantined malformed {bucket} journal entry at {JOURNAL_FILE_NAME} line {line_no}: {err_msg}"),
    });
}

fn read_line_bytes_bounded(
    reader: &mut dyn std::io::BufRead,
    line_buf: &mut Vec<u8>,
    max_line_bytes: usize,
    total_bytes: &mut usize,
    max_total_bytes: usize,
) -> Result<bool, FormatError> {
    line_buf.clear();
    let raw_line_cap = max_line_bytes.saturating_add(2);
    loop {
        let available = match reader.fill_buf() {
            Ok(buf) => buf,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(FormatError::from(err)),
        };
        if available.is_empty() {
            if line_buf.is_empty() {
                return Ok(false);
            }
            break;
        }
        let (consume_len, found_newline) = match available.iter().position(|&b| b == b'\n') {
            Some(idx) => (idx.saturating_add(1), true),
            None => (available.len(), false),
        };
        *total_bytes = total_bytes.checked_add(consume_len).ok_or_else(|| FormatError::Io(String::from("journal replay byte count overflow")))?;
        if *total_bytes > max_total_bytes {
            return Err(FormatError::Io(format!("journal `{JOURNAL_FILE_NAME}` exceeds maximum replay limit ({max_total_bytes} bytes)")));
        }
        if line_buf.len().saturating_add(consume_len) > raw_line_cap {
            return Err(FormatError::Io(format!("journal `{JOURNAL_FILE_NAME}` line exceeds maximum line limit ({max_line_bytes} bytes)")));
        }
        line_buf.extend_from_slice(&available[..consume_len]);
        reader.consume(consume_len);
        if found_newline {
            break;
        }
    }
    if line_buf.ends_with(b"\n") {
        line_buf.pop();
    }
    if line_buf.ends_with(b"\r") {
        line_buf.pop();
    }
    if line_buf.len() > max_line_bytes {
        return Err(FormatError::Io(format!("journal `{JOURNAL_FILE_NAME}` line exceeds maximum line limit ({max_line_bytes} bytes)")));
    }
    Ok(true)
}

fn stream_and_parse_journal(
    reader: &mut dyn std::io::BufRead,
    limits: JournalLimits,
    snapshot: Option<&[u8]>,
) -> Result<StreamedJournalParse, FormatError> {
    let mut state = StreamedJournalParse {
        records: Vec::new(),
        discarded_corrupt_tail_lines: 0,
        discarded_corrupt_interior_lines: 0,
        warnings: Vec::new(),
        prev_seq: 0,
        marker_covered_seq: None,
        total_bytes: 0,
    };
    let max_physical_lines = limits.max_records.saturating_mul(4).saturating_add(1024);
    let mut line_buf = Vec::new();
    let mut physical_line_no = 0usize;
    let mut non_empty_lines = 0usize;
    let mut pending_bad: Option<(usize, String)> = None;
    let mut snapshot_digest = snapshot.map(LazySnapshotDigest::new);

    while read_line_bytes_bounded(reader, &mut line_buf, limits.max_line_bytes, &mut state.total_bytes, limits.max_bytes)? {
        physical_line_no = physical_line_no.saturating_add(1);
        if physical_line_no > max_physical_lines {
            return Err(FormatError::Io(format!(
                "journal `{JOURNAL_FILE_NAME}` line count exceeds maximum replay limit ({max_physical_lines} lines)"
            )));
        }
        if line_buf.is_empty() || line_buf.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        non_empty_lines = non_empty_lines.saturating_add(1);
        if non_empty_lines > limits.max_records {
            return Err(FormatError::Io(format!(
                "journal `{JOURNAL_FILE_NAME}` record count exceeds maximum replay limit ({} records)",
                limits.max_records
            )));
        }
        flush_pending_bad_line(&mut pending_bad, false, &mut state);

        let mut record = match parse_journal_line(physical_line_no, &line_buf) {
            Ok(record) => record,
            Err(err @ FormatError::UnsupportedSchemaVersion { .. }) => return Err(err),
            Err(err) => {
                pending_bad = Some((physical_line_no, err.to_string()));
                continue;
            }
        };

        // CRITICAL CRC ORDERING: verify CRC32 against the raw persisted v1 payload FIRST,
        // then upgrade v1 aliases to canonical v2 spellings and recompute in-memory CRC32.
        record.verify_crc()?;
        if record.format_version < CURRENT_JOURNAL_VERSION {
            record.upgrade_verified_to_current()?;
        }
        if record.seq == 0 || record.seq <= state.prev_seq {
            return Err(FormatError::InvalidPipeline {
                reason: format!("non-monotonic journal sequence {} after {} at line {physical_line_no}", record.seq, state.prev_seq),
            });
        }
        state.prev_seq = record.seq;
        if let Some(marker) = extract_matching_snapshot_marker(&record.command_json, &mut snapshot_digest) {
            state.marker_covered_seq = Some(state.marker_covered_seq.map_or(marker.covered_seq, |existing| existing.max(marker.covered_seq)));
        }
        state.records.push(ParsedRecordSummary { seq: record.seq, crc32: record.crc32, command_json: record.command_json });
    }

    flush_pending_bad_line(&mut pending_bad, true, &mut state);
    Ok(state)
}

fn build_cleaned_journal_bytes(seqs: &[u64], crcs: &[u32], commands: &[serde_json::Value]) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    for ((&seq, &crc32), command_json) in seqs.iter().zip(crcs.iter()).zip(commands.iter()) {
        let record = JournalOpRecord { format_version: CURRENT_JOURNAL_VERSION, seq, crc32, command_json: command_json.clone() };
        let line = record.to_ndjson_line()?;
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

fn recover_journal_contents<B: JournalBackend>(backend: &mut B, limits: JournalLimits) -> Result<RecoveredJournalState, FormatError> {
    let snapshot = backend.read_snapshot()?;
    let backend_covered_seq = match snapshot.as_deref() {
        Some(bytes) => backend.read_snapshot_covered_seq_for(bytes)?,
        None => None,
    };
    let mut reader: Box<dyn std::io::BufRead + '_> = match backend.open_journal_stream(limits.max_bytes)? {
        Some(stream) => stream,
        None => {
            let raw_journal = backend.read_journal_bytes()?;
            if raw_journal.len() > limits.max_bytes {
                return Err(FormatError::Io(format!(
                    "journal `{JOURNAL_FILE_NAME}` size ({} bytes) exceeds maximum replay limit ({} bytes)",
                    raw_journal.len(),
                    limits.max_bytes
                )));
            }
            Box::new(std::io::Cursor::new(raw_journal))
        }
    };

    let parsed = stream_and_parse_journal(&mut *reader, limits, snapshot.as_deref())?;
    drop(reader);

    let effective_covered_seq = match (backend_covered_seq, parsed.marker_covered_seq) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    let mut commands = Vec::with_capacity(parsed.records.len());
    let mut seqs = Vec::with_capacity(parsed.records.len());
    let mut kept_crcs = Vec::with_capacity(parsed.records.len());
    let mut stripped_covered_records = false;
    let covered_floor = effective_covered_seq.unwrap_or(0);

    for record in parsed.records {
        if is_snapshot_marker(&record.command_json) || record.seq <= covered_floor {
            stripped_covered_records = true;
            continue;
        }
        commands.push(record.command_json);
        seqs.push(record.seq);
        kept_crcs.push(record.crc32);
    }

    let total_discarded = parsed.discarded_corrupt_tail_lines.saturating_add(parsed.discarded_corrupt_interior_lines);
    let remaining_journal_bytes = if total_discarded > 0 || stripped_covered_records {
        let cleaned_bytes = build_cleaned_journal_bytes(&seqs, &kept_crcs, &commands)?;
        let len = cleaned_bytes.len();
        backend.replace_journal_bytes_atomic(&cleaned_bytes)?;
        len
    } else {
        parsed.total_bytes
    };

    let last_seq = parsed.prev_seq.max(covered_floor);
    let remaining_record_count = commands.len();
    Ok(RecoveredJournalState {
        snapshot,
        snapshot_covered_seq: effective_covered_seq,
        commands,
        seqs,
        discarded_corrupt_tail_lines: parsed.discarded_corrupt_tail_lines,
        discarded_corrupt_interior_lines: parsed.discarded_corrupt_interior_lines,
        warnings: parsed.warnings,
        last_seq,
        remaining_record_count,
        remaining_journal_bytes,
    })
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

#[cfg(test)]
mod tests {
    use super::{
        CrashRecoveryJournal, DirectoryJournalBackend, JournalBackend, JournalLockLease, LazySnapshotDigest, MemoryJournalBackend,
        build_snapshot_marker_record, extract_matching_snapshot_marker,
    };
    use crate::error::FormatError;

    #[test]
    fn lazy_snapshot_digest_caches_length_and_crc_across_markers() {
        let snap = b"SNAPSHOT-BYTES-FOR-LAZY-DIGEST";
        let m1 = build_snapshot_marker_record(2, 1, snap).unwrap();
        let m2 = build_snapshot_marker_record(4, 3, snap).unwrap();
        let mut digest = Some(LazySnapshotDigest::new(snap));
        assert!(digest.as_ref().unwrap().digest.is_none());

        let p1 = extract_matching_snapshot_marker(&m1.command_json, &mut digest).unwrap();
        assert_eq!(p1.covered_seq, 1);
        let cached = digest.as_ref().unwrap().digest.unwrap();
        assert_eq!(cached, (snap.len() as u64, crc32fast::hash(snap)));

        let p2 = extract_matching_snapshot_marker(&m2.command_json, &mut digest).unwrap();
        assert_eq!(p2.covered_seq, 3);
        assert_eq!(digest.as_ref().unwrap().digest, Some(cached));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn directory_backend_read_snapshot_covered_seq_for_uses_caller_bytes_without_rereading_snapshot() {
        let dir = std::env::temp_dir().join(format!("cb-recovery-snap-for-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut backend = DirectoryJournalBackend::open_dir(&dir).unwrap();
        let snap = b"SNAPSHOT-PAYLOAD-V1";
        backend.write_snapshot_with_seq(snap, 7).unwrap();

        // Remove snapshot file on disk: `read_snapshot_covered_seq_for` must still match metadata
        // against caller-supplied bytes without trying to re-read `snapshot.curvebook`.
        std::fs::remove_file(backend.snapshot_path()).unwrap();
        assert_eq!(backend.read_snapshot_covered_seq_for(snap).unwrap(), Some(7));
        assert_eq!(backend.read_snapshot_covered_seq_for(b"DIFFERENT-PAYLOAD").unwrap(), None);
        assert_eq!(backend.read_snapshot_covered_seq().unwrap(), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_close_and_checkpoint_share_legacy_marker_publication_order() {
        struct LegacyTraceBackend {
            inner: MemoryJournalBackend,
            events: Vec<&'static str>,
            fail_clear: bool,
        }

        impl JournalBackend for LegacyTraceBackend {
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
            fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
                self.inner.read_snapshot()
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
                if self.fail_clear {
                    return Err(FormatError::Io(String::from("simulated clear failure")));
                }
                self.inner.clear_journal()
            }
        }

        let backend = LegacyTraceBackend { inner: MemoryJournalBackend::new(), events: Vec::new(), fail_clear: true };
        let (mut journal, _) = CrashRecoveryJournal::open_or_recover(backend, "w1", 5_000, 1_000).unwrap();
        assert_eq!(journal.record_command(serde_json::json!({"op": "a"})).unwrap(), 1);
        journal.backend_mut().events.clear();

        // Failed checkpoint on legacy backend appends marker before clear_journal and advances next_seq.
        assert!(journal.checkpoint(b"SNAP-1").is_err());
        assert_eq!(journal.backend().events, vec!["write_snapshot_atomic", "append_journal_line", "clear_journal"]);
        assert_eq!(journal.next_seq(), 3);

        // Clean close on legacy backend publishes snapshot -> marker -> clear_journal -> release_lease.
        journal.backend_mut().fail_clear = false;
        journal.backend_mut().events.clear();
        let closed = journal.close_cleanly(b"SNAP-2").unwrap();
        assert_eq!(closed.events, vec!["write_snapshot_atomic", "append_journal_line", "clear_journal", "release_lease"]);
    }
}
