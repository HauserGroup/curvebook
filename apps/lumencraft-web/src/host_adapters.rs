//! Zero-telemetry browser host adapters for LumenCraft Web (`R8.4`).
//!
//! All storage persistence, multi-tab lock coordination, virtual directory incremental sync,
//! touch pinch/pan gesture recognition, multi-flavor clipboard assembly, vector print output,
//! and JSON bridge message handling remain 100% local and free of network calls.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt;

use lumencraft_engine::cmd;
use lumencraft_format::{CURRENT_PREFERENCES_VERSION, PreferencesFile, parse_preferences_json};
use lumencraft_ui_egui::serde_json::{self, Map, Value, json};
use lumencraft_ui_egui::{CanvasPreviewFilter, ClipboardBundle, PrintJobSpec, UiEventOutcome, UiInputEvent, ViewportMetrics};

use crate::{WebWorkspaceHost, WorkspaceSummary};

/// Default browser `localStorage` quota budget (`5 MiB`).
pub const QUOTA_BUDGET_BYTES: usize = 5 * 1024 * 1024;
/// Canonical `localStorage` key for user preferences.
pub const PREFERENCES_STORAGE_KEY: &str = "lumencraft.v1.preferences";
/// Canonical `localStorage` key prefix for per-document autosave snapshots.
pub const AUTOSAVE_KEY_PREFIX: &str = "lumencraft.v1.autosave.";
/// Default multi-tab lease duration before a stalled tab lock may be reclaimed (in milliseconds).
pub const DEFAULT_LOCK_LEASE_TTL_MS: u64 = 15_000;
/// Maximum allowed clipboard/raster DPI matching [`ClipboardBundle`].
const MAX_CLIPBOARD_DPI: u16 = 1200;
/// Minimum allowed clipboard/raster DPI matching [`ClipboardBundle`].
const MIN_CLIPBOARD_DPI: u16 = 72;

/// Typed errors returned by browser host adapters and the web bridge session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebAdapterError {
    /// Stored or supplied preferences carry a newer schema version than this client supports.
    ForwardVersionPreferences { found: u32, supported: u32 },
    /// Persistence payload exceeds the bounded browser storage quota.
    QuotaExceeded { required_bytes: usize, budget_bytes: usize },
    /// A write operation was rejected because another browser tab holds the document lock.
    ReadOnlyContended { doc_id: String, holder_tab_id: String, operation: String },
    /// JSON, base64, or preferences payload could not be serialized or decoded.
    Serialization(String),
    /// Caller-supplied key, identifier, path, or numeric parameter failed validation.
    Validation(String),
    /// Workspace or engine execution failed.
    Workspace(String),
    /// Multi-flavor clipboard assembly failed.
    Clipboard(String),
    /// Print spooling or SVG print preparation failed.
    Print(String),
}

impl WebAdapterError {
    /// Machine-readable error code string for JSON bridge replies.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ForwardVersionPreferences { .. } => "forward_version_preferences",
            Self::QuotaExceeded { .. } => "quota_exceeded",
            Self::ReadOnlyContended { .. } => "read_only_contended",
            Self::Serialization(_) => "serialization_error",
            Self::Validation(_) => "validation_error",
            Self::Workspace(_) => "workspace_error",
            Self::Clipboard(_) => "clipboard_error",
            Self::Print(_) => "print_error",
        }
    }

    /// Convert the error into a structured JSON object.
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        match self {
            Self::ForwardVersionPreferences { found, supported } => json!({
                "code": self.code(),
                "found": found,
                "supported": supported,
                "message": self.to_string(),
            }),
            Self::QuotaExceeded { required_bytes, budget_bytes } => json!({
                "code": self.code(),
                "required_bytes": required_bytes,
                "budget_bytes": budget_bytes,
                "message": self.to_string(),
            }),
            Self::ReadOnlyContended { doc_id, holder_tab_id, operation } => json!({
                "code": self.code(),
                "doc_id": doc_id,
                "holder_tab_id": holder_tab_id,
                "operation": operation,
                "message": self.to_string(),
            }),
            Self::Serialization(msg) | Self::Validation(msg) | Self::Workspace(msg) | Self::Clipboard(msg) | Self::Print(msg) => json!({
                "code": self.code(),
                "message": msg,
            }),
        }
    }
}

impl fmt::Display for WebAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForwardVersionPreferences { found, supported } => {
                write!(f, "preferences schema version {found} is newer than supported version {supported}")
            }
            Self::QuotaExceeded { required_bytes, budget_bytes } => {
                write!(f, "storage quota exceeded: required {required_bytes} bytes, budget is {budget_bytes} bytes")
            }
            Self::ReadOnlyContended { doc_id, holder_tab_id, operation } => {
                write!(f, "document `{doc_id}` is locked read-only by tab `{holder_tab_id}`; rejected `{operation}`")
            }
            Self::Serialization(msg) => write!(f, "serialization error: {msg}"),
            Self::Validation(msg) => write!(f, "validation error: {msg}"),
            Self::Workspace(msg) => write!(f, "workspace error: {msg}"),
            Self::Clipboard(msg) => write!(f, "clipboard error: {msg}"),
            Self::Print(msg) => write!(f, "print error: {msg}"),
        }
    }
}

impl Error for WebAdapterError {}

/// Abstract key-value browser storage backend (`window.localStorage` or test memory mock).
pub trait WebStorageBackend {
    /// Read the stored value for `key`, if present.
    fn get_item(&self, key: &str) -> Option<String>;
    /// Write `value` under `key`, failing if the backend's capacity limit would be exceeded.
    fn set_item(&mut self, key: &str, value: &str) -> Result<(), WebAdapterError>;
    /// Delete `key` and return its previous value, if any.
    fn remove_item(&mut self, key: &str) -> Option<String>;
    /// Return all stored keys in deterministic sorted order.
    fn keys(&self) -> Vec<String>;
    /// Total serialized byte usage (`key.len() + value.len()`) currently held in storage.
    fn used_bytes(&self) -> usize;
}

/// Pure-Rust in-memory [`WebStorageBackend`] enforcing a bounded byte budget.
#[derive(Debug, Clone)]
pub struct MemoryWebStorage {
    entries: BTreeMap<String, String>,
    quota_budget_bytes: usize,
}

impl Default for MemoryWebStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryWebStorage {
    /// Create a new memory storage backend with the default `5 MiB` quota budget.
    #[must_use]
    pub fn new() -> Self {
        Self::with_budget(QUOTA_BUDGET_BYTES)
    }

    /// Create a new memory storage backend with a custom quota budget in bytes.
    #[must_use]
    pub fn with_budget(quota_budget_bytes: usize) -> Self {
        Self { entries: BTreeMap::new(), quota_budget_bytes }
    }

    /// Configured quota budget in bytes.
    #[must_use]
    pub const fn quota_budget_bytes(&self) -> usize {
        self.quota_budget_bytes
    }

    fn entry_size(key: &str, value: &str) -> usize {
        key.len().saturating_add(value.len())
    }
}

impl WebStorageBackend for MemoryWebStorage {
    fn get_item(&self, key: &str) -> Option<String> {
        self.entries.get(key).cloned()
    }

    fn set_item(&mut self, key: &str, value: &str) -> Result<(), WebAdapterError> {
        let prev_size = self.entries.get(key).map_or(0, |old| Self::entry_size(key, old));
        let next_size = Self::entry_size(key, value);
        let projected = self.used_bytes().saturating_sub(prev_size).saturating_add(next_size);
        if projected > self.quota_budget_bytes {
            return Err(WebAdapterError::QuotaExceeded { required_bytes: projected, budget_bytes: self.quota_budget_bytes });
        }
        self.entries.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn remove_item(&mut self, key: &str) -> Option<String> {
        self.entries.remove(key)
    }

    fn keys(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    fn used_bytes(&self) -> usize {
        self.entries.iter().map(|(k, v)| Self::entry_size(k, v)).sum()
    }
}

/// Summary returned after persisting an autosave snapshot under quota management.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutosaveSaveOutcome {
    pub storage_key: String,
    pub doc_id: String,
    pub entry_bytes: usize,
    pub total_used_bytes: usize,
    pub evicted_keys: Vec<String>,
}

/// High-level storage manager handling key prefixing, schema checks, and 5 MiB LRU eviction.
#[derive(Debug, Clone)]
pub struct WebStorageManager<B: WebStorageBackend = MemoryWebStorage> {
    backend: B,
    quota_budget_bytes: usize,
    access_seq: u64,
    autosave_recency: BTreeMap<String, (u64, u64)>,
}

impl Default for WebStorageManager<MemoryWebStorage> {
    fn default() -> Self {
        Self::memory()
    }
}

impl WebStorageManager<MemoryWebStorage> {
    /// Construct a manager backed by an empty `5 MiB` [`MemoryWebStorage`].
    #[must_use]
    pub fn memory() -> Self {
        Self::new(MemoryWebStorage::new())
    }
}

impl<B: WebStorageBackend> WebStorageManager<B> {
    /// Construct a manager over `backend` using the standard `5 MiB` (`QUOTA_BUDGET_BYTES`) cap.
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self::with_budget(backend, QUOTA_BUDGET_BYTES)
    }

    /// Construct a manager over `backend` with an explicit byte quota budget.
    #[must_use]
    pub fn with_budget(backend: B, quota_budget_bytes: usize) -> Self {
        let mut mgr = Self { backend, quota_budget_bytes, access_seq: 0, autosave_recency: BTreeMap::new() };
        for key in mgr.backend.keys() {
            if key.starts_with(AUTOSAVE_KEY_PREFIX) {
                mgr.access_seq = mgr.access_seq.saturating_add(1);
                mgr.autosave_recency.insert(key, (0, mgr.access_seq));
            }
        }
        mgr
    }

    /// Borrow the underlying storage backend.
    #[must_use]
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// Mutably borrow the underlying storage backend.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Total bytes currently held across preferences and autosaves.
    #[must_use]
    pub fn used_bytes(&self) -> usize {
        self.backend.used_bytes()
    }

    /// Configured quota budget in bytes.
    #[must_use]
    pub const fn quota_budget_bytes(&self) -> usize {
        self.quota_budget_bytes
    }

    /// Build the canonical `lumencraft.v1.autosave.<doc_id>` key for `doc_id`.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `doc_id` is empty or contains path separators.
    pub fn autosave_key(doc_id: &str) -> Result<String, WebAdapterError> {
        let trimmed = doc_id.trim();
        if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") || trimmed.chars().any(char::is_control) {
            return Err(WebAdapterError::Validation(format!("invalid autosave document id `{doc_id}`")));
        }
        Ok(format!("{AUTOSAVE_KEY_PREFIX}{trimmed}"))
    }

    /// Load and validate [`PreferencesFile`] from `lumencraft.v1.preferences`.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::ForwardVersionPreferences`] if the stored schema exceeds
    /// [`CURRENT_PREFERENCES_VERSION`], or [`WebAdapterError::Serialization`] on invalid JSON.
    pub fn load_preferences(&self) -> Result<Option<PreferencesFile>, WebAdapterError> {
        let Some(raw) = self.backend.get_item(PREFERENCES_STORAGE_KEY) else {
            return Ok(None);
        };
        check_raw_preferences_schema(&raw)?;
        parse_preferences_json(&raw).map(Some).map_err(|err| WebAdapterError::Serialization(err.to_string()))
    }

    /// Persist [`PreferencesFile`] under `lumencraft.v1.preferences` without clobbering newer schemas.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::ForwardVersionPreferences`] if either `prefs.version` or any
    /// already-stored payload has a schema version newer than [`CURRENT_PREFERENCES_VERSION`].
    pub fn save_preferences(&mut self, prefs: &PreferencesFile) -> Result<String, WebAdapterError> {
        if prefs.version > CURRENT_PREFERENCES_VERSION {
            return Err(WebAdapterError::ForwardVersionPreferences { found: prefs.version, supported: CURRENT_PREFERENCES_VERSION });
        }
        self.ensure_stored_preferences_not_forward()?;
        let encoded = serde_json::to_string_pretty(prefs).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let needed_delta = self.projected_delta_for_key(PREFERENCES_STORAGE_KEY, &encoded);
        let _evicted = self.evict_autosaves_to_fit(PREFERENCES_STORAGE_KEY, needed_delta)?;
        self.backend.set_item(PREFERENCES_STORAGE_KEY, &encoded)?;
        Ok(encoded)
    }

    /// Validate raw JSON preferences text and persist it without clobbering newer schemas.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::ForwardVersionPreferences`] if either the incoming JSON or
    /// stored preferences exceed [`CURRENT_PREFERENCES_VERSION`].
    pub fn save_preferences_raw_json(&mut self, raw_json: &str) -> Result<PreferencesFile, WebAdapterError> {
        check_raw_preferences_schema(raw_json)?;
        let prefs = parse_preferences_json(raw_json).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let _ = self.save_preferences(&prefs)?;
        Ok(prefs)
    }

    /// Save a document autosave snapshot, evicting oldest/LRU autosave entries to stay within quota.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] for invalid `doc_id` or [`WebAdapterError::QuotaExceeded`]
    /// if the snapshot cannot fit even after evicting all other autosave entries.
    pub fn save_autosave_snapshot(&mut self, doc_id: &str, snapshot: &str, timestamp_ms: u64) -> Result<AutosaveSaveOutcome, WebAdapterError> {
        let key = Self::autosave_key(doc_id)?;
        let entry_bytes = key.len().saturating_add(snapshot.len());
        let non_autosave_bytes = self.non_autosave_used_bytes();
        if non_autosave_bytes.saturating_add(entry_bytes) > self.quota_budget_bytes {
            return Err(WebAdapterError::QuotaExceeded {
                required_bytes: non_autosave_bytes.saturating_add(entry_bytes),
                budget_bytes: self.quota_budget_bytes,
            });
        }

        let delta = self.projected_delta_for_key(&key, snapshot);
        let evicted_keys = self.evict_autosaves_to_fit(&key, delta)?;
        self.backend.set_item(&key, snapshot)?;
        self.touch_autosave_recency(&key, timestamp_ms);

        Ok(AutosaveSaveOutcome {
            storage_key: key,
            doc_id: doc_id.trim().to_owned(),
            entry_bytes,
            total_used_bytes: self.backend.used_bytes(),
            evicted_keys,
        })
    }

    /// Load an autosave snapshot for `doc_id`, refreshing its LRU recency timestamp if present.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `doc_id` is invalid.
    pub fn load_autosave_snapshot(&mut self, doc_id: &str, now_ms: u64) -> Result<Option<String>, WebAdapterError> {
        let key = Self::autosave_key(doc_id)?;
        let found = self.backend.get_item(&key);
        if found.is_some() {
            self.touch_autosave_recency(&key, now_ms);
        }
        Ok(found)
    }

    /// Remove the stored autosave snapshot for `doc_id`, returning whether an entry existed.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `doc_id` is invalid.
    pub fn clear_autosave_snapshot(&mut self, doc_id: &str) -> Result<bool, WebAdapterError> {
        let key = Self::autosave_key(doc_id)?;
        self.autosave_recency.remove(&key);
        Ok(self.backend.remove_item(&key).is_some())
    }

    /// List currently stored autosave document identifiers in sorted key order.
    #[must_use]
    pub fn list_autosave_doc_ids(&self) -> Vec<String> {
        self.backend.keys().into_iter().filter_map(|key| key.strip_prefix(AUTOSAVE_KEY_PREFIX).map(str::to_owned)).collect()
    }

    fn ensure_stored_preferences_not_forward(&self) -> Result<(), WebAdapterError> {
        if let Some(existing) = self.backend.get_item(PREFERENCES_STORAGE_KEY) {
            check_raw_preferences_schema(&existing)?;
        }
        Ok(())
    }

    fn non_autosave_used_bytes(&self) -> usize {
        self.backend
            .keys()
            .into_iter()
            .filter(|k| !k.starts_with(AUTOSAVE_KEY_PREFIX))
            .filter_map(|k| self.backend.get_item(&k).map(|v| k.len().saturating_add(v.len())))
            .sum()
    }

    fn projected_delta_for_key(&self, key: &str, next_value: &str) -> isize {
        let prev = self.backend.get_item(key).map_or(0usize, |v| key.len().saturating_add(v.len()));
        let next = key.len().saturating_add(next_value.len());
        let prev_i = isize::try_from(prev).unwrap_or(isize::MAX);
        let next_i = isize::try_from(next).unwrap_or(isize::MAX);
        next_i.saturating_sub(prev_i)
    }

    fn touch_autosave_recency(&mut self, key: &str, now_ms: u64) {
        self.access_seq = self.access_seq.saturating_add(1);
        self.autosave_recency.insert(key.to_owned(), (now_ms, self.access_seq));
    }

    fn evict_autosaves_to_fit(&mut self, protected_key: &str, additional_delta: isize) -> Result<Vec<String>, WebAdapterError> {
        let mut evicted = Vec::new();
        loop {
            let current = isize::try_from(self.backend.used_bytes()).unwrap_or(isize::MAX);
            let projected = current.saturating_add(additional_delta);
            let budget = isize::try_from(self.quota_budget_bytes).unwrap_or(isize::MAX);
            if projected <= budget {
                return Ok(evicted);
            }

            let Some(victim_key) = self.oldest_autosave_key(protected_key) else {
                let req = usize::try_from(projected.max(0)).unwrap_or(usize::MAX);
                return Err(WebAdapterError::QuotaExceeded { required_bytes: req, budget_bytes: self.quota_budget_bytes });
            };
            let _ = self.backend.remove_item(&victim_key);
            self.autosave_recency.remove(&victim_key);
            evicted.push(victim_key);
        }
    }

    fn oldest_autosave_key(&self, protected_key: &str) -> Option<String> {
        self.backend.keys().into_iter().filter(|k| k.starts_with(AUTOSAVE_KEY_PREFIX) && k != protected_key).min_by_key(|k| {
            let (ts, seq) = self.autosave_recency.get(k).copied().unwrap_or((0, 0));
            (ts, seq, k.clone())
        })
    }
}

fn check_raw_preferences_schema(raw: &str) -> Result<(), WebAdapterError> {
    let value: Value = serde_json::from_str(raw).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
    let obj = value.as_object().ok_or_else(|| WebAdapterError::Serialization(String::from("preferences root must be a JSON object")))?;
    let mut detected_version: Option<u32> = None;
    for field in ["schema_version", "version"] {
        if let Some(val) = obj.get(field) {
            let ver_u64 = val.as_u64().ok_or_else(|| WebAdapterError::Serialization(format!("preferences `{field}` must be an unsigned integer")))?;
            let ver = u32::try_from(ver_u64).unwrap_or(u32::MAX);
            detected_version = Some(detected_version.map_or(ver, |prev| prev.max(ver)));
        }
    }
    if let Some(found) = detected_version
        && found > CURRENT_PREFERENCES_VERSION
    {
        return Err(WebAdapterError::ForwardVersionPreferences { found, supported: CURRENT_PREFERENCES_VERSION });
    }
    Ok(())
}

/// Access grant mode for a browser tab viewing/editing a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabAccessMode {
    /// Tab owns the exclusive document lock and may perform document mutations.
    ExclusiveReadWrite,
    /// Another tab holds the active lock; only viewport navigation, previewing, and exports are allowed.
    ReadOnlyContended { holder_tab_id: String },
}

impl TabAccessMode {
    /// Returns `true` when mutations must be rejected due to contention.
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        matches!(self, Self::ReadOnlyContended { .. })
    }

    /// Returns the locking tab identifier when contended.
    #[must_use]
    pub fn holder_tab_id(&self) -> Option<&str> {
        match self {
            Self::ExclusiveReadWrite => None,
            Self::ReadOnlyContended { holder_tab_id } => Some(holder_tab_id.as_str()),
        }
    }
}

/// Result of attempting to acquire a document lock in [`WebLockCoordinator`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockAcquisition {
    pub doc_id: String,
    pub tab_id: String,
    pub mode: TabAccessMode,
    pub stale_takeover: bool,
}

/// Result of releasing a document lock (and optionally promoting the next waiting tab).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockReleaseOutcome {
    pub doc_id: String,
    pub released_tab_id: String,
    pub was_holder: bool,
    pub promoted_tab_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DocumentLockRecord {
    holder_tab_id: String,
    acquired_at_ms: u64,
    last_heartbeat_ms: u64,
    contenders: VecDeque<String>,
}

impl DocumentLockRecord {
    fn is_stale(&self, now_ms: u64, lease_ttl_ms: u64) -> bool {
        now_ms >= self.last_heartbeat_ms.saturating_add(lease_ttl_ms)
    }
}

/// Multi-tab `navigator.locks` coordinator with heartbeat renewal, stale takeover, and promotion.
#[derive(Debug, Clone)]
pub struct WebLockCoordinator {
    lease_ttl_ms: u64,
    records: BTreeMap<String, DocumentLockRecord>,
}

impl Default for WebLockCoordinator {
    fn default() -> Self {
        Self::new(DEFAULT_LOCK_LEASE_TTL_MS)
    }
}

impl WebLockCoordinator {
    /// Create a coordinator with explicit stale-lease timeout in milliseconds.
    #[must_use]
    pub fn new(lease_ttl_ms: u64) -> Self {
        Self { lease_ttl_ms: lease_ttl_ms.max(1), records: BTreeMap::new() }
    }

    /// Configured lease timeout in milliseconds.
    #[must_use]
    pub const fn lease_ttl_ms(&self) -> u64 {
        self.lease_ttl_ms
    }

    /// Attempt to acquire an exclusive lock on `doc_id` for `tab_id` at `now_ms`.
    ///
    /// Grants [`TabAccessMode::ExclusiveReadWrite`] when unheld, already owned by `tab_id`, or
    /// when the existing holder's lease has expired; otherwise registers `tab_id` as a contender
    /// and returns [`TabAccessMode::ReadOnlyContended`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `doc_id` or `tab_id` is empty.
    pub fn acquire_document_lock(&mut self, doc_id: &str, tab_id: &str, now_ms: u64) -> Result<LockAcquisition, WebAdapterError> {
        let doc = require_non_empty(doc_id, "doc_id")?;
        let tab = require_non_empty(tab_id, "tab_id")?;

        let Some(record) = self.records.get_mut(doc) else {
            self.records.insert(
                doc.to_owned(),
                DocumentLockRecord { holder_tab_id: tab.to_owned(), acquired_at_ms: now_ms, last_heartbeat_ms: now_ms, contenders: VecDeque::new() },
            );
            return Ok(LockAcquisition {
                doc_id: doc.to_owned(),
                tab_id: tab.to_owned(),
                mode: TabAccessMode::ExclusiveReadWrite,
                stale_takeover: false,
            });
        };

        if record.holder_tab_id == tab {
            record.last_heartbeat_ms = record.last_heartbeat_ms.max(now_ms);
            return Ok(LockAcquisition {
                doc_id: doc.to_owned(),
                tab_id: tab.to_owned(),
                mode: TabAccessMode::ExclusiveReadWrite,
                stale_takeover: false,
            });
        }

        if record.is_stale(now_ms, self.lease_ttl_ms) {
            record.holder_tab_id = tab.to_owned();
            record.acquired_at_ms = now_ms;
            record.last_heartbeat_ms = now_ms;
            record.contenders.retain(|c| c != tab);
            return Ok(LockAcquisition {
                doc_id: doc.to_owned(),
                tab_id: tab.to_owned(),
                mode: TabAccessMode::ExclusiveReadWrite,
                stale_takeover: true,
            });
        }

        if !record.contenders.iter().any(|c| c == tab) {
            record.contenders.push_back(tab.to_owned());
        }
        Ok(LockAcquisition {
            doc_id: doc.to_owned(),
            tab_id: tab.to_owned(),
            mode: TabAccessMode::ReadOnlyContended { holder_tab_id: record.holder_tab_id.clone() },
            stale_takeover: false,
        })
    }

    /// Renew `tab_id`'s active lock lease heartbeat at `now_ms`.
    ///
    /// Returns `true` when `tab_id` still holds a live lock on `doc_id`, or `false` if another
    /// tab holds it or no lock exists.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `doc_id` or `tab_id` is empty.
    pub fn renew_heartbeat(&mut self, doc_id: &str, tab_id: &str, now_ms: u64) -> Result<bool, WebAdapterError> {
        let doc = require_non_empty(doc_id, "doc_id")?;
        let tab = require_non_empty(tab_id, "tab_id")?;
        let Some(record) = self.records.get_mut(doc) else {
            return Ok(false);
        };
        if record.holder_tab_id != tab {
            return Ok(false);
        }
        if record.is_stale(now_ms, self.lease_ttl_ms) && !record.contenders.is_empty() {
            return Ok(false);
        }
        record.last_heartbeat_ms = record.last_heartbeat_ms.max(now_ms);
        Ok(true)
    }

    /// Release `tab_id`'s lock (or contender reservation) on `doc_id`, promoting the oldest
    /// waiting contending tab when the active holder releases.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `doc_id` or `tab_id` is empty.
    pub fn release_document_lock(&mut self, doc_id: &str, tab_id: &str, now_ms: u64) -> Result<LockReleaseOutcome, WebAdapterError> {
        let doc = require_non_empty(doc_id, "doc_id")?;
        let tab = require_non_empty(tab_id, "tab_id")?;
        let Some(record) = self.records.get_mut(doc) else {
            return Ok(LockReleaseOutcome { doc_id: doc.to_owned(), released_tab_id: tab.to_owned(), was_holder: false, promoted_tab_id: None });
        };

        if record.holder_tab_id != tab {
            record.contenders.retain(|c| c != tab);
            return Ok(LockReleaseOutcome { doc_id: doc.to_owned(), released_tab_id: tab.to_owned(), was_holder: false, promoted_tab_id: None });
        }

        if let Some(next_holder) = record.contenders.pop_front() {
            record.holder_tab_id = next_holder.clone();
            record.acquired_at_ms = now_ms;
            record.last_heartbeat_ms = now_ms;
            return Ok(LockReleaseOutcome {
                doc_id: doc.to_owned(),
                released_tab_id: tab.to_owned(),
                was_holder: true,
                promoted_tab_id: Some(next_holder),
            });
        }

        self.records.remove(doc);
        Ok(LockReleaseOutcome { doc_id: doc.to_owned(), released_tab_id: tab.to_owned(), was_holder: true, promoted_tab_id: None })
    }

    /// Record an externally observed `navigator.locks` lock holder for `doc_id`.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `doc_id` or `holder_tab_id` is empty.
    pub fn observe_external_holder(&mut self, doc_id: &str, holder_tab_id: &str, now_ms: u64) -> Result<(), WebAdapterError> {
        let doc = require_non_empty(doc_id, "doc_id")?;
        let holder = require_non_empty(holder_tab_id, "holder_tab_id")?;
        if let Some(record) = self.records.get_mut(doc) {
            record.holder_tab_id = holder.to_owned();
            record.acquired_at_ms = now_ms;
            record.last_heartbeat_ms = now_ms;
            record.contenders.retain(|c| c != holder);
        } else {
            self.records.insert(
                doc.to_owned(),
                DocumentLockRecord {
                    holder_tab_id: holder.to_owned(),
                    acquired_at_ms: now_ms,
                    last_heartbeat_ms: now_ms,
                    contenders: VecDeque::new(),
                },
            );
        }
        Ok(())
    }

    /// Compute the effective [`TabAccessMode`] for `tab_id` viewing `doc_id` at `now_ms`.
    #[must_use]
    pub fn access_mode(&self, doc_id: &str, tab_id: &str, now_ms: u64) -> TabAccessMode {
        let Some(record) = self.records.get(doc_id.trim()) else {
            return TabAccessMode::ExclusiveReadWrite;
        };
        if record.holder_tab_id == tab_id.trim() || record.is_stale(now_ms, self.lease_ttl_ms) {
            TabAccessMode::ExclusiveReadWrite
        } else {
            TabAccessMode::ReadOnlyContended { holder_tab_id: record.holder_tab_id.clone() }
        }
    }

    /// Return the active non-stale holder tab identifier for `doc_id`, if any.
    #[must_use]
    pub fn active_holder(&self, doc_id: &str, now_ms: u64) -> Option<String> {
        self.records.get(doc_id.trim()).filter(|rec| !rec.is_stale(now_ms, self.lease_ttl_ms)).map(|rec| rec.holder_tab_id.clone())
    }

    /// Ordered queue of tabs currently waiting as read-only contenders on `doc_id`.
    #[must_use]
    pub fn contenders(&self, doc_id: &str) -> Vec<String> {
        self.records.get(doc_id.trim()).map(|rec| rec.contenders.iter().cloned().collect()).unwrap_or_default()
    }
}

fn require_non_empty<'a>(raw: &'a str, label: &str) -> Result<&'a str, WebAdapterError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(WebAdapterError::Validation(format!("{label} must not be empty")));
    }
    Ok(trimmed)
}

/// Deterministic incremental write/removal diff for `.lumencraft.d/` directory synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirectorySyncPlan {
    pub writes: Vec<(String, Vec<u8>)>,
    pub removals: Vec<String>,
    pub unchanged_count: usize,
}

impl DirectorySyncPlan {
    /// Returns `true` when no files need to be written or removed.
    #[must_use]
    pub fn is_empty_changes(&self) -> bool {
        self.writes.is_empty() && self.removals.is_empty()
    }

    /// Ordered relative paths scheduled to be written.
    #[must_use]
    pub fn written_paths(&self) -> Vec<&str> {
        self.writes.iter().map(|(p, _)| p.as_str()).collect()
    }
}

/// Pure-Rust File System Access & HTML5 drag-and-drop bridge for archives and `.lumencraft.d/`.
#[derive(Debug, Clone, Default)]
pub struct FileSystemAccessBridge {
    last_synced_root: Option<String>,
    last_synced_tree: BTreeMap<String, Vec<u8>>,
}

impl FileSystemAccessBridge {
    /// Construct an empty file-system bridge with no previously synced directory tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Root folder name captured during the last `.lumencraft.d/` import or sync.
    #[must_use]
    pub fn last_synced_root(&self) -> Option<&str> {
        self.last_synced_root.as_deref()
    }

    /// Snapshot of relative file contents recorded during the last `.lumencraft.d/` import or sync.
    #[must_use]
    pub const fn last_synced_tree(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.last_synced_tree
    }

    /// Import a single picked or dropped file (`lumencraft`, `lctemplate`, `pzfx`, `prism`, `csv`, `tsv`).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if decoding fails.
    pub fn import_file_bytes(&mut self, host: &mut WebWorkspaceHost, file_name: &str, bytes: &[u8]) -> Result<WorkspaceSummary, WebAdapterError> {
        let summary = host.open_bytes_for_name(file_name, bytes).map_err(WebAdapterError::Workspace)?;
        self.last_synced_root = None;
        self.last_synced_tree.clear();
        Ok(summary)
    }

    /// Import a `.lumencraft.d/` virtual directory tree and seed incremental sync tracking.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] for empty/unsafe paths or [`WebAdapterError::Workspace`]
    /// if the unpacked `document.json` cannot be loaded.
    pub fn import_directory_tree(
        &mut self,
        host: &mut WebWorkspaceHost,
        root_name: &str,
        entries: BTreeMap<String, Vec<u8>>,
    ) -> Result<WorkspaceSummary, WebAdapterError> {
        let root = require_non_empty(root_name, "root_name")?;
        if entries.is_empty() {
            return Err(WebAdapterError::Validation(String::from("directory tree must contain at least document.json")));
        }
        let normalized = normalize_directory_entries(root, entries)?;
        let summary =
            host.load_virtual_folder_entries(root, normalized.iter().map(|(k, v)| (k.clone(), v.clone()))).map_err(WebAdapterError::Workspace)?;
        self.last_synced_root = Some(root.to_owned());
        self.last_synced_tree = normalized;
        Ok(summary)
    }

    /// Import HTML5 drag-and-drop entries (either a single file or an unpacked `.lumencraft.d/` folder).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `files` is empty, or [`WebAdapterError::Workspace`]
    /// if the dropped payload cannot be decoded.
    pub fn import_dropped_files(&mut self, host: &mut WebWorkspaceHost, files: Vec<(String, Vec<u8>)>) -> Result<WorkspaceSummary, WebAdapterError> {
        if files.is_empty() {
            return Err(WebAdapterError::Validation(String::from("no dropped files supplied")));
        }
        if files.len() == 1 && !looks_like_directory_entry(&files[0].0) {
            let (name, bytes) = &files[0];
            return self.import_file_bytes(host, name, bytes);
        }

        let detected_root = detect_dropped_directory_root(&files);
        let mut tree = BTreeMap::new();
        for (raw_path, bytes) in files {
            let rel = normalize_relative_entry_path(&raw_path, detected_root.as_deref())?;
            tree.insert(rel, bytes);
        }
        if !tree.contains_key("document.json") && !tree.contains_key("manifest.json") {
            return Err(WebAdapterError::Validation(String::from(
                "dropped multi-file batch must contain a .lumencraft.d document.json or manifest.json",
            )));
        }
        let root_name = detected_root.unwrap_or_else(|| String::from("dropped.lumencraft.d"));
        self.import_directory_tree(host, &root_name, tree)
    }

    /// Export the active workspace as a `.lumencraft` archive byte buffer.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if archive serialization fails.
    pub fn export_lumencraft_bytes(&self, host: &mut WebWorkspaceHost) -> Result<Vec<u8>, WebAdapterError> {
        host.export_lumencraft_bytes().map_err(WebAdapterError::Workspace)
    }

    /// Materialize the current workspace as a deterministic `.lumencraft.d/` relative file tree.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if exporting unpacked entries fails.
    pub fn export_directory_tree(&self, host: &mut WebWorkspaceHost) -> Result<BTreeMap<String, Vec<u8>>, WebAdapterError> {
        let entries = host.export_virtual_entries().map_err(WebAdapterError::Workspace)?;
        Ok(entries.into_iter().collect())
    }

    /// Compute deterministic incremental write/removal operations from `previous` to `next`.
    #[must_use]
    pub fn plan_directory_sync(previous: &BTreeMap<String, Vec<u8>>, next: &BTreeMap<String, Vec<u8>>) -> DirectorySyncPlan {
        let mut writes = Vec::new();
        let mut removals = Vec::new();
        let mut unchanged_count = 0usize;

        for (path, next_bytes) in next {
            match previous.get(path) {
                Some(prev_bytes) if prev_bytes == next_bytes => {
                    unchanged_count = unchanged_count.saturating_add(1);
                }
                _ => {
                    writes.push((path.clone(), next_bytes.clone()));
                }
            }
        }
        for prev_path in previous.keys() {
            if !next.contains_key(prev_path) {
                removals.push(prev_path.clone());
            }
        }

        DirectorySyncPlan { writes, removals, unchanged_count }
    }

    /// Export the active workspace to `.lumencraft.d/` entries, diff against the last synced tree,
    /// and advance the tracked snapshot so only changed files are emitted.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if virtual snapshot serialization fails.
    pub fn sync_directory_from_host(&mut self, host: &mut WebWorkspaceHost, root_name: Option<&str>) -> Result<DirectorySyncPlan, WebAdapterError> {
        let resolved_root = if let Some(explicit) = root_name.map(str::trim).filter(|s| !s.is_empty()) {
            host.virtual_workspace_mut().root_name = explicit.to_owned();
            explicit.to_owned()
        } else if let Some(prev) = &self.last_synced_root {
            host.virtual_workspace_mut().root_name = prev.clone();
            prev.clone()
        } else {
            host.virtual_workspace().root_name.clone()
        };

        let next = self.export_directory_tree(host)?;
        let plan = Self::plan_directory_sync(&self.last_synced_tree, &next);
        self.last_synced_root = Some(resolved_root);
        self.last_synced_tree = next;
        Ok(plan)
    }
}

fn looks_like_directory_entry(path: &str) -> bool {
    let trimmed = path.trim().replace('\\', "/");
    trimmed.contains('/') || trimmed == "document.json" || trimmed == "manifest.json"
}

fn detect_dropped_directory_root(files: &[(String, Vec<u8>)]) -> Option<String> {
    let mut candidate_roots = Vec::new();
    for (raw, _) in files {
        let normalized = raw.trim().replace('\\', "/");
        let trimmed = normalized.trim_start_matches("./").trim_start_matches('/');
        if let Some((first, _rest)) = trimmed.split_once('/')
            && !first.is_empty()
        {
            candidate_roots.push(first.to_owned());
        }
    }
    let first = candidate_roots.first()?.clone();
    if candidate_roots.len() == files.len() && candidate_roots.iter().all(|root| root == &first) { Some(first) } else { None }
}

fn normalize_directory_entries(root_name: &str, entries: BTreeMap<String, Vec<u8>>) -> Result<BTreeMap<String, Vec<u8>>, WebAdapterError> {
    let mut normalized = BTreeMap::new();
    for (path, bytes) in entries {
        let rel = normalize_relative_entry_path(&path, Some(root_name))?;
        normalized.insert(rel, bytes);
    }
    Ok(normalized)
}

fn normalize_relative_entry_path(raw: &str, root_prefix: Option<&str>) -> Result<String, WebAdapterError> {
    let slash_normalized = raw.trim().replace('\\', "/");
    let mut rel = slash_normalized.trim_start_matches("./").trim_start_matches('/');
    if let Some(root) = root_prefix {
        let prefix = format!("{root}/");
        if let Some(stripped) = rel.strip_prefix(&prefix) {
            rel = stripped;
        }
    }
    if rel.is_empty() || rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return Err(WebAdapterError::Validation(format!("unsafe virtual directory entry path `{raw}`")));
    }
    Ok(rel.to_owned())
}

/// Active browser touch contact point in CSS logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchPoint {
    pub id: i64,
    pub x: f64,
    pub y: f64,
}

/// Pure-Rust two-finger touch tracker converting browser touch points into [`UiInputEvent`]s.
#[derive(Debug, Clone, Default)]
pub struct TouchGestureRecognizer {
    active: BTreeMap<i64, (f64, f64)>,
    last_midpoint: Option<(f64, f64)>,
    last_distance: Option<f64>,
    two_finger_active: bool,
    moved_two_finger: bool,
}

impl TouchGestureRecognizer {
    /// Construct an idle touch recognizer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of currently tracked touch contacts.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Whether a two-finger pinch/pan gesture is currently active.
    #[must_use]
    pub const fn is_two_finger_gesture(&self) -> bool {
        self.two_finger_active
    }

    /// Process browser `touchstart` contacts.
    pub fn touch_start(&mut self, points: &[TouchPoint], _dt_sec: f64) -> Vec<UiInputEvent> {
        let prev_count = self.active.len();
        for pt in points {
            if pt.x.is_finite() && pt.y.is_finite() {
                self.active.insert(pt.id, (pt.x, pt.y));
            }
        }
        if self.active.len() >= 2 {
            self.two_finger_active = true;
            if let Some((mid, dist)) = self.two_finger_geometry() {
                self.last_midpoint = Some(mid);
                self.last_distance = Some(dist);
            }
            return Vec::new();
        }
        if prev_count == 0
            && self.active.len() == 1
            && let Some(&(x, y)) = self.active.values().next()
        {
            return vec![UiInputEvent::PointerDown {
                x,
                y,
                button: lumencraft_ui_egui::PointerButton::Primary,
                modifiers: lumencraft_ui_egui::KeyModifiers::default(),
            }];
        }
        Vec::new()
    }

    /// Process browser `touchmove` contacts into [`UiInputEvent::PinchZoom`] and [`UiInputEvent::InertialPan`].
    pub fn touch_move(&mut self, points: &[TouchPoint], dt_sec: f64) -> Vec<UiInputEvent> {
        let clamped_dt = clamp_gesture_dt(dt_sec);
        for pt in points {
            if pt.x.is_finite() && pt.y.is_finite() && self.active.contains_key(&pt.id) {
                self.active.insert(pt.id, (pt.x, pt.y));
            }
        }
        if self.active.len() >= 2 {
            self.two_finger_active = true;
            return self.process_two_finger_move(clamped_dt);
        }
        if !self.two_finger_active
            && self.active.len() == 1
            && let Some(&(x, y)) = self.active.values().next()
        {
            return vec![UiInputEvent::PointerMove { x, y, modifiers: lumencraft_ui_egui::KeyModifiers::default() }];
        }
        Vec::new()
    }

    /// Process browser `touchend` contact releases, emitting inertial coast steps after two-finger pans.
    pub fn touch_end(&mut self, released_ids: &[i64], dt_sec: f64) -> Vec<UiInputEvent> {
        let clamped_dt = clamp_gesture_dt(dt_sec);
        let was_two_finger = self.two_finger_active;
        let last_single_pos = self.active.values().next().copied();
        for id in released_ids {
            self.active.remove(id);
        }

        let mut events = Vec::new();
        if self.active.len() >= 2 {
            if let Some((mid, dist)) = self.two_finger_geometry() {
                self.last_midpoint = Some(mid);
                self.last_distance = Some(dist);
            }
            return events;
        }

        self.last_midpoint = None;
        self.last_distance = None;
        if was_two_finger && self.moved_two_finger {
            events.push(UiInputEvent::InertialPan { delta_x: 0.0, delta_y: 0.0, dt_sec: clamped_dt, step_coast: true });
            self.moved_two_finger = false;
        }
        if self.active.is_empty() {
            if !was_two_finger && let Some((x, y)) = last_single_pos {
                events.push(UiInputEvent::PointerUp {
                    x,
                    y,
                    button: lumencraft_ui_egui::PointerButton::Primary,
                    modifiers: lumencraft_ui_egui::KeyModifiers::default(),
                });
            }
            self.two_finger_active = false;
            self.moved_two_finger = false;
        }
        events
    }

    /// Cancel all tracked touches and reset gesture state.
    pub fn touch_cancel(&mut self) -> Vec<UiInputEvent> {
        self.active.clear();
        self.last_midpoint = None;
        self.last_distance = None;
        self.two_finger_active = false;
        self.moved_two_finger = false;
        vec![UiInputEvent::CancelInlineEdit]
    }

    /// Produce an animation-frame coasting step [`UiInputEvent::InertialPan`].
    #[must_use]
    pub fn step_inertia(&self, dt_sec: f64) -> UiInputEvent {
        UiInputEvent::InertialPan { delta_x: 0.0, delta_y: 0.0, dt_sec: clamp_gesture_dt(dt_sec), step_coast: true }
    }

    fn two_finger_geometry(&self) -> Option<((f64, f64), f64)> {
        let mut iter = self.active.values();
        let p0 = *iter.next()?;
        let p1 = *iter.next()?;
        let mid = (0.5 * (p0.0 + p1.0), 0.5 * (p0.1 + p1.1));
        let dist = (p1.0 - p0.0).hypot(p1.1 - p0.1);
        Some((mid, dist))
    }

    fn process_two_finger_move(&mut self, dt_sec: f64) -> Vec<UiInputEvent> {
        let Some((mid, dist)) = self.two_finger_geometry() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(2);
        if let Some(prev_dist) = self.last_distance
            && prev_dist > 1e-6
            && dist > 1e-6
        {
            let scale_delta = (dist / prev_dist).clamp(0.1, 10.0);
            if (scale_delta - 1.0).abs() > 1e-6 {
                out.push(UiInputEvent::PinchZoom { x: mid.0, y: mid.1, scale_delta });
            }
        }
        if let Some(prev_mid) = self.last_midpoint {
            let delta_x = mid.0 - prev_mid.0;
            let delta_y = mid.1 - prev_mid.1;
            if delta_x.abs() > 1e-6 || delta_y.abs() > 1e-6 {
                self.moved_two_finger = true;
                out.push(UiInputEvent::InertialPan { delta_x, delta_y, dt_sec, step_coast: false });
            }
        }
        self.last_midpoint = Some(mid);
        self.last_distance = Some(dist);
        out
    }
}

fn clamp_gesture_dt(dt_sec: f64) -> f64 {
    if dt_sec.is_finite() && dt_sec > 0.0 { dt_sec.clamp(1.0 / 240.0, 0.25) } else { 1.0 / 60.0 }
}

/// Multi-flavor web clipboard bundle (`PDF`, `SVG`, `PNG`, `HTML`, `TSV`).
#[derive(Debug, Clone, PartialEq)]
pub struct WebClipboardPayload {
    pub pdf_bytes: Vec<u8>,
    pub svg: String,
    pub html: String,
    pub png_bytes: Vec<u8>,
    pub tsv: Option<String>,
    pub plain_text: Option<String>,
    pub width_pt: f64,
    pub height_pt: f64,
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
    pub flavors: Vec<String>,
}

impl WebClipboardPayload {
    /// Serialize this clipboard payload to a JSON [`Value`].
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        json!({
            "pdf_base64": encode_base64(&self.pdf_bytes),
            "svg": self.svg,
            "html": self.html,
            "png_base64": encode_base64(&self.png_bytes),
            "tsv": self.tsv,
            "plain_text": self.plain_text,
            "width_pt": self.width_pt,
            "height_pt": self.height_pt,
            "width_px": self.width_px,
            "height_px": self.height_px,
            "dpi": self.dpi,
            "flavors": self.flavors,
        })
    }

    /// Reconstruct a [`WebClipboardPayload`] from its JSON representation.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] if required fields are missing or malformed.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let obj = require_object(value, "clipboard payload")?;
        let pdf_bytes = decode_base64_field(obj, "pdf_base64", "pdf_bytes")?;
        let png_bytes = decode_base64_field(obj, "png_base64", "png_bytes")?;
        let svg = optional_str_field(obj, "svg").unwrap_or_default();
        let html = optional_str_field(obj, "html").unwrap_or_default();
        let tsv = optional_str_field(obj, "tsv");
        let plain_text = optional_str_field(obj, "plain_text");
        let width_pt = require_f64_or_default(obj, "width_pt", 0.0)?;
        let height_pt = require_f64_or_default(obj, "height_pt", 0.0)?;
        let width_px = u32::try_from(obj.get("width_px").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0);
        let height_px = u32::try_from(obj.get("height_px").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0);
        let dpi = u16::try_from(obj.get("dpi").and_then(Value::as_u64).unwrap_or(300)).unwrap_or(300);
        let flavors = obj
            .get("flavors")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();

        Ok(Self { pdf_bytes, svg, html, png_bytes, tsv, plain_text, width_pt, height_pt, width_px, height_px, dpi, flavors })
    }
}

/// Incoming event sent from browser DOM/JS host hooks into [`WebBridgeSession`].
#[derive(Debug, Clone, PartialEq)]
pub enum WebBridgeEvent {
    ResizeViewport { logical_width: f64, logical_height: f64, scale_factor: f64 },
    TouchStart { points: Vec<TouchPoint>, dt_sec: f64 },
    TouchMove { points: Vec<TouchPoint>, dt_sec: f64 },
    TouchEnd { released_ids: Vec<i64>, dt_sec: f64 },
    TouchCancel,
    CoastStep { dt_sec: f64 },
    UiInput { event: UiInputEvent },
    ExecuteCommand { command_id: String, params: Value },
    DispatchControlLine { line: String },
    RenderFrame { dpi: u16 },
    CopyClipboard { sheet_id: Option<String>, dpi: u16 },
    PrintCurrentSheet { sheet_id: Option<String>, scale_percent: f64 },
    OpenFileBytes { file_name: String, bytes: Vec<u8> },
    ImportDroppedFiles { files: Vec<(String, Vec<u8>)> },
    ImportDirectoryEntries { root_name: String, entries: BTreeMap<String, Vec<u8>> },
    SyncDirectory { root_name: Option<String> },
    SavePreferences { preferences: PreferencesFile },
    SaveAutosave { now_ms: u64 },
    AcquireLock { doc_id: String, tab_id: String, now_ms: u64 },
    ObserveExternalLock { doc_id: String, holder_tab_id: String, now_ms: u64 },
    RenewHeartbeat { now_ms: u64 },
    ReleaseLock { now_ms: u64 },
    SetPreviewFilter { filter: CanvasPreviewFilter },
    SetPresentationMode { enabled: bool },
}

impl WebBridgeEvent {
    /// Encode this bridge event into a JSON [`Value`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] if embedded `UiInputEvent` or `PreferencesFile` serialization fails.
    pub fn to_json_value(&self) -> Result<Value, WebAdapterError> {
        let value = match self {
            Self::ResizeViewport { logical_width, logical_height, scale_factor } => json!({
                "kind": "resize_viewport",
                "logical_width": logical_width,
                "logical_height": logical_height,
                "scale_factor": scale_factor,
            }),
            Self::TouchStart { points, dt_sec } => json!({
                "kind": "touch_start",
                "points": touch_points_to_json(points),
                "dt_sec": dt_sec,
            }),
            Self::TouchMove { points, dt_sec } => json!({
                "kind": "touch_move",
                "points": touch_points_to_json(points),
                "dt_sec": dt_sec,
            }),
            Self::TouchEnd { released_ids, dt_sec } => json!({
                "kind": "touch_end",
                "released_ids": released_ids,
                "dt_sec": dt_sec,
            }),
            Self::TouchCancel => json!({ "kind": "touch_cancel" }),
            Self::CoastStep { dt_sec } => json!({ "kind": "coast_step", "dt_sec": dt_sec }),
            Self::UiInput { event } => {
                let encoded = serde_json::to_value(event).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
                json!({ "kind": "ui_input", "event": encoded })
            }
            Self::ExecuteCommand { command_id, params } => json!({
                "kind": "execute_command",
                "command_id": command_id,
                "params": params,
            }),
            Self::DispatchControlLine { line } => json!({ "kind": "dispatch_control_line", "line": line }),
            Self::RenderFrame { dpi } => json!({ "kind": "render_frame", "dpi": dpi }),
            Self::CopyClipboard { sheet_id, dpi } => json!({
                "kind": "copy_clipboard",
                "sheet_id": sheet_id,
                "dpi": dpi,
            }),
            Self::PrintCurrentSheet { sheet_id, scale_percent } => json!({
                "kind": "print_current_sheet",
                "sheet_id": sheet_id,
                "scale_percent": scale_percent,
            }),
            Self::OpenFileBytes { file_name, bytes } => json!({
                "kind": "open_file_bytes",
                "file_name": file_name,
                "bytes_base64": encode_base64(bytes),
            }),
            Self::ImportDroppedFiles { files } => json!({
                "kind": "import_dropped_files",
                "files": files_to_json_array(files),
            }),
            Self::ImportDirectoryEntries { root_name, entries } => json!({
                "kind": "import_directory_entries",
                "root_name": root_name,
                "entries": map_to_json_array(entries),
            }),
            Self::SyncDirectory { root_name } => json!({ "kind": "sync_directory", "root_name": root_name }),
            Self::SavePreferences { preferences } => {
                let encoded = serde_json::to_value(preferences).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
                json!({ "kind": "save_preferences", "preferences": encoded })
            }
            Self::SaveAutosave { now_ms } => json!({ "kind": "save_autosave", "now_ms": now_ms }),
            Self::AcquireLock { doc_id, tab_id, now_ms } => json!({
                "kind": "acquire_lock",
                "doc_id": doc_id,
                "tab_id": tab_id,
                "now_ms": now_ms,
            }),
            Self::ObserveExternalLock { doc_id, holder_tab_id, now_ms } => json!({
                "kind": "observe_external_lock",
                "doc_id": doc_id,
                "holder_tab_id": holder_tab_id,
                "now_ms": now_ms,
            }),
            Self::RenewHeartbeat { now_ms } => json!({ "kind": "renew_heartbeat", "now_ms": now_ms }),
            Self::ReleaseLock { now_ms } => json!({ "kind": "release_lock", "now_ms": now_ms }),
            Self::SetPreviewFilter { filter } => {
                let encoded = serde_json::to_value(filter).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
                json!({ "kind": "set_preview_filter", "filter": encoded })
            }
            Self::SetPresentationMode { enabled } => json!({ "kind": "set_presentation_mode", "enabled": enabled }),
        };
        Ok(value)
    }

    /// Serialize this bridge event into compact JSON text.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] if serialization fails.
    pub fn to_json_string(&self) -> Result<String, WebAdapterError> {
        let value = self.to_json_value()?;
        serde_json::to_string(&value).map_err(|err| WebAdapterError::Serialization(err.to_string()))
    }

    /// Decode a [`WebBridgeEvent`] from a JSON [`Value`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when the payload does not match any supported variant.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let obj = require_object(value, "bridge event")?;
        let kind = require_str_field(obj, "kind")?;
        match kind {
            "resize_viewport" => Ok(Self::ResizeViewport {
                logical_width: require_f64_field(obj, "logical_width")?,
                logical_height: require_f64_field(obj, "logical_height")?,
                scale_factor: require_f64_or_default(obj, "scale_factor", 1.0)?,
            }),
            "touch_start" => {
                Ok(Self::TouchStart { points: parse_touch_points_field(obj, "points")?, dt_sec: require_f64_or_default(obj, "dt_sec", 1.0 / 60.0)? })
            }
            "touch_move" => {
                Ok(Self::TouchMove { points: parse_touch_points_field(obj, "points")?, dt_sec: require_f64_or_default(obj, "dt_sec", 1.0 / 60.0)? })
            }
            "touch_end" => {
                Ok(Self::TouchEnd { released_ids: parse_i64_array(obj, "released_ids")?, dt_sec: require_f64_or_default(obj, "dt_sec", 1.0 / 60.0)? })
            }
            "touch_cancel" => Ok(Self::TouchCancel),
            "coast_step" => Ok(Self::CoastStep { dt_sec: require_f64_or_default(obj, "dt_sec", 1.0 / 60.0)? }),
            "ui_input" => {
                let raw = obj.get("event").ok_or_else(|| WebAdapterError::Serialization(String::from("missing `event` field")))?;
                let event: UiInputEvent = serde_json::from_value(raw.clone()).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
                Ok(Self::UiInput { event })
            }
            "execute_command" => Ok(Self::ExecuteCommand {
                command_id: require_str_field(obj, "command_id")?.to_owned(),
                params: obj.get("params").cloned().unwrap_or_else(|| Value::Object(Map::new())),
            }),
            "dispatch_control_line" => Ok(Self::DispatchControlLine { line: require_str_field(obj, "line")?.to_owned() }),
            "render_frame" => Ok(Self::RenderFrame { dpi: require_u16_or_default(obj, "dpi", 96) }),
            "copy_clipboard" => {
                Ok(Self::CopyClipboard { sheet_id: optional_str_field(obj, "sheet_id"), dpi: require_u16_or_default(obj, "dpi", 300) })
            }
            "print_current_sheet" => Ok(Self::PrintCurrentSheet {
                sheet_id: optional_str_field(obj, "sheet_id"),
                scale_percent: require_f64_or_default(obj, "scale_percent", 100.0)?,
            }),
            "open_file_bytes" => Ok(Self::OpenFileBytes {
                file_name: require_str_field(obj, "file_name")?.to_owned(),
                bytes: decode_base64_field(obj, "bytes_base64", "bytes")?,
            }),
            "import_dropped_files" => Ok(Self::ImportDroppedFiles { files: parse_named_bytes_vec(obj, "files")? }),
            "import_directory_entries" => Ok(Self::ImportDirectoryEntries {
                root_name: require_str_field(obj, "root_name")?.to_owned(),
                entries: parse_named_bytes_vec(obj, "entries")?.into_iter().collect(),
            }),
            "sync_directory" => Ok(Self::SyncDirectory { root_name: optional_str_field(obj, "root_name") }),
            "save_preferences" => {
                let raw = obj.get("preferences").ok_or_else(|| WebAdapterError::Serialization(String::from("missing `preferences` field")))?;
                let raw_text = serde_json::to_string(raw).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
                check_raw_preferences_schema(&raw_text)?;
                let preferences = parse_preferences_json(&raw_text).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
                Ok(Self::SavePreferences { preferences })
            }
            "save_autosave" => Ok(Self::SaveAutosave { now_ms: require_u64_or_default(obj, "now_ms", 0) }),
            "acquire_lock" => Ok(Self::AcquireLock {
                doc_id: require_str_field(obj, "doc_id")?.to_owned(),
                tab_id: require_str_field(obj, "tab_id")?.to_owned(),
                now_ms: require_u64_or_default(obj, "now_ms", 0),
            }),
            "observe_external_lock" => Ok(Self::ObserveExternalLock {
                doc_id: require_str_field(obj, "doc_id")?.to_owned(),
                holder_tab_id: require_str_field(obj, "holder_tab_id")?.to_owned(),
                now_ms: require_u64_or_default(obj, "now_ms", 0),
            }),
            "renew_heartbeat" => Ok(Self::RenewHeartbeat { now_ms: require_u64_or_default(obj, "now_ms", 0) }),
            "release_lock" => Ok(Self::ReleaseLock { now_ms: require_u64_or_default(obj, "now_ms", 0) }),
            "set_preview_filter" => {
                let raw = obj.get("filter").ok_or_else(|| WebAdapterError::Serialization(String::from("missing `filter` field")))?;
                let filter: CanvasPreviewFilter =
                    serde_json::from_value(raw.clone()).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
                Ok(Self::SetPreviewFilter { filter })
            }
            "set_presentation_mode" => Ok(Self::SetPresentationMode { enabled: obj.get("enabled").and_then(Value::as_bool).unwrap_or(false) }),
            other => Err(WebAdapterError::Serialization(format!("unknown bridge event kind `{other}`"))),
        }
    }

    /// Decode a [`WebBridgeEvent`] from raw JSON text.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] on parse failure.
    pub fn from_json_str(raw: &str) -> Result<Self, WebAdapterError> {
        let value: Value = serde_json::from_str(raw).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        Self::from_json_value(&value)
    }
}

/// Outgoing host command emitted by [`WebBridgeSession`] for the browser DOM host to execute.
#[derive(Debug, Clone, PartialEq)]
pub enum WebBridgeCommand {
    RenderRgbaFrame {
        width_px: u32,
        height_px: u32,
        dpi: u16,
        rgba_bytes: Vec<u8>,
        zoom: f64,
        pan_pt: [f64; 2],
        action: Option<String>,
        command_result: Option<Value>,
    },
    WriteClipboardItems {
        payload: WebClipboardPayload,
    },
    TriggerPrintHtml {
        title: String,
        svg_pages: Vec<String>,
        html: String,
        pdf_bytes: Vec<u8>,
        scale_percent: f64,
        page_sizes_pt: Vec<(f64, f64)>,
    },
    ApplyDirectorySync {
        root_name: String,
        plan: DirectorySyncPlan,
    },
    PersistLocalStorage {
        writes: Vec<(String, String)>,
        removals: Vec<String>,
    },
    SetReadOnlyBanner {
        doc_id: String,
        read_only: bool,
        holder_tab_id: Option<String>,
        message: Option<String>,
    },
}

impl WebBridgeCommand {
    /// Encode this command into a JSON [`Value`].
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        match self {
            Self::RenderRgbaFrame { width_px, height_px, dpi, rgba_bytes, zoom, pan_pt, action, command_result } => json!({
                "kind": "render_rgba_frame",
                "width_px": width_px,
                "height_px": height_px,
                "dpi": dpi,
                "rgba_base64": encode_base64(rgba_bytes),
                "zoom": zoom,
                "pan_pt": pan_pt,
                "action": action,
                "command_result": command_result,
            }),
            Self::WriteClipboardItems { payload } => json!({
                "kind": "write_clipboard_items",
                "payload": payload.to_json_value(),
            }),
            Self::TriggerPrintHtml { title, svg_pages, html, pdf_bytes, scale_percent, page_sizes_pt } => json!({
                "kind": "trigger_print_html",
                "title": title,
                "svg_pages": svg_pages,
                "html": html,
                "pdf_base64": encode_base64(pdf_bytes),
                "scale_percent": scale_percent,
                "page_sizes_pt": page_sizes_pt,
            }),
            Self::ApplyDirectorySync { root_name, plan } => json!({
                "kind": "apply_directory_sync",
                "root_name": root_name,
                "writes": files_to_json_array(&plan.writes),
                "removals": plan.removals,
                "unchanged_count": plan.unchanged_count,
            }),
            Self::PersistLocalStorage { writes, removals } => json!({
                "kind": "persist_local_storage",
                "writes": writes.iter().map(|(k, v)| json!({ "key": k, "value": v })).collect::<Vec<_>>(),
                "removals": removals,
            }),
            Self::SetReadOnlyBanner { doc_id, read_only, holder_tab_id, message } => json!({
                "kind": "set_read_only_banner",
                "doc_id": doc_id,
                "read_only": read_only,
                "holder_tab_id": holder_tab_id,
                "message": message,
            }),
        }
    }

    /// Reconstruct a [`WebBridgeCommand`] from a JSON [`Value`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when required fields are missing or malformed.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let obj = require_object(value, "bridge command")?;
        let kind = require_str_field(obj, "kind")?;
        match kind {
            "render_rgba_frame" => {
                let width_px = u32::try_from(obj.get("width_px").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0);
                let height_px = u32::try_from(obj.get("height_px").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0);
                let dpi = require_u16_or_default(obj, "dpi", 96);
                let rgba_bytes = decode_base64_field(obj, "rgba_base64", "rgba_bytes")?;
                let zoom = require_f64_or_default(obj, "zoom", 1.0)?;
                let pan_pt = parse_pan_pt(obj.get("pan_pt"))?;
                let action = optional_str_field(obj, "action");
                let command_result = obj.get("command_result").filter(|v| !v.is_null()).cloned();
                Ok(Self::RenderRgbaFrame { width_px, height_px, dpi, rgba_bytes, zoom, pan_pt, action, command_result })
            }
            "write_clipboard_items" => {
                let payload_val = obj.get("payload").ok_or_else(|| WebAdapterError::Serialization(String::from("missing `payload`")))?;
                Ok(Self::WriteClipboardItems { payload: WebClipboardPayload::from_json_value(payload_val)? })
            }
            "trigger_print_html" => {
                let title = require_str_field(obj, "title")?.to_owned();
                let svg_pages = obj
                    .get("svg_pages")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
                let html = optional_str_field(obj, "html").unwrap_or_default();
                let pdf_bytes = decode_base64_field(obj, "pdf_base64", "pdf_bytes")?;
                let scale_percent = require_f64_or_default(obj, "scale_percent", 100.0)?;
                let page_sizes_pt = parse_page_sizes(obj.get("page_sizes_pt"))?;
                Ok(Self::TriggerPrintHtml { title, svg_pages, html, pdf_bytes, scale_percent, page_sizes_pt })
            }
            "apply_directory_sync" => {
                let root_name = require_str_field(obj, "root_name")?.to_owned();
                let writes = parse_named_bytes_vec(obj, "writes")?;
                let removals = obj
                    .get("removals")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
                let unchanged_count = usize::try_from(obj.get("unchanged_count").and_then(Value::as_u64).unwrap_or(0)).unwrap_or(0);
                Ok(Self::ApplyDirectorySync { root_name, plan: DirectorySyncPlan { writes, removals, unchanged_count } })
            }
            "persist_local_storage" => {
                let writes = parse_string_pairs(obj, "writes")?;
                let removals = obj
                    .get("removals")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
                Ok(Self::PersistLocalStorage { writes, removals })
            }
            "set_read_only_banner" => Ok(Self::SetReadOnlyBanner {
                doc_id: require_str_field(obj, "doc_id")?.to_owned(),
                read_only: obj.get("read_only").and_then(Value::as_bool).unwrap_or(false),
                holder_tab_id: optional_str_field(obj, "holder_tab_id"),
                message: optional_str_field(obj, "message"),
            }),
            other => Err(WebAdapterError::Serialization(format!("unknown bridge command kind `{other}`"))),
        }
    }
}

/// Complete browser adapter session binding workspace, storage, locks, filesystem sync, and touch input.
#[derive(Debug, Clone)]
pub struct WebBridgeSession {
    pub host: WebWorkspaceHost,
    pub storage: WebStorageManager<MemoryWebStorage>,
    pub locks: WebLockCoordinator,
    pub fs: FileSystemAccessBridge,
    pub touch: TouchGestureRecognizer,
    doc_id: String,
    tab_id: String,
    access_mode: TabAccessMode,
    render_dpi: u16,
}

impl Default for WebBridgeSession {
    fn default() -> Self {
        Self::new()
    }
}

impl WebBridgeSession {
    /// Construct a bootstrap session on document `"default"` and tab `"tab-1"` with exclusive access.
    #[must_use]
    pub fn new() -> Self {
        Self::with_ids("default", "tab-1")
    }

    /// Construct a session for explicit `doc_id` and `tab_id` identifiers.
    #[must_use]
    pub fn with_ids(doc_id: &str, tab_id: &str) -> Self {
        let doc = if doc_id.trim().is_empty() { String::from("default") } else { doc_id.trim().to_owned() };
        let tab = if tab_id.trim().is_empty() { String::from("tab-1") } else { tab_id.trim().to_owned() };
        Self {
            host: WebWorkspaceHost::new(),
            storage: WebStorageManager::memory(),
            locks: WebLockCoordinator::default(),
            fs: FileSystemAccessBridge::new(),
            touch: TouchGestureRecognizer::new(),
            doc_id: doc,
            tab_id: tab,
            access_mode: TabAccessMode::ExclusiveReadWrite,
            render_dpi: 96,
        }
    }

    /// Active document identifier tracked for multi-tab locking and autosave keys.
    #[must_use]
    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    /// Browser tab identifier tracked for multi-tab locking.
    #[must_use]
    pub fn tab_id(&self) -> &str {
        &self.tab_id
    }

    /// Current multi-tab access grant (`ExclusiveReadWrite` or `ReadOnlyContended`).
    #[must_use]
    pub const fn access_mode(&self) -> &TabAccessMode {
        &self.access_mode
    }

    /// Acquire the document lock for `(doc_id, tab_id)` against a shared [`WebLockCoordinator`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if identifiers are empty.
    pub fn acquire_lock_with(
        &mut self,
        coordinator: &mut WebLockCoordinator,
        now_ms: u64,
    ) -> Result<(LockAcquisition, Vec<WebBridgeCommand>), WebAdapterError> {
        let acquisition = coordinator.acquire_document_lock(&self.doc_id, &self.tab_id, now_ms)?;
        let banner = self.apply_access_mode(acquisition.mode.clone());
        Ok((acquisition, vec![banner]))
    }

    /// Renew lease heartbeat against a shared [`WebLockCoordinator`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if identifiers are empty.
    pub fn renew_heartbeat_with(&self, coordinator: &mut WebLockCoordinator, now_ms: u64) -> Result<bool, WebAdapterError> {
        coordinator.renew_heartbeat(&self.doc_id, &self.tab_id, now_ms)
    }

    /// Release this tab's lock in a shared [`WebLockCoordinator`], promoting any contending tab.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if identifiers are empty.
    pub fn release_lock_with(
        &mut self,
        coordinator: &mut WebLockCoordinator,
        now_ms: u64,
    ) -> Result<(LockReleaseOutcome, Vec<WebBridgeCommand>), WebAdapterError> {
        let outcome = coordinator.release_document_lock(&self.doc_id, &self.tab_id, now_ms)?;
        let next_mode = coordinator.access_mode(&self.doc_id, &self.tab_id, now_ms);
        let banner = self.apply_access_mode(next_mode);
        Ok((outcome, vec![banner]))
    }

    /// Refresh this session's local access mode from a shared [`WebLockCoordinator`].
    pub fn sync_lock_from(&mut self, coordinator: &WebLockCoordinator, now_ms: u64) -> (TabAccessMode, Vec<WebBridgeCommand>) {
        let mode = coordinator.access_mode(&self.doc_id, &self.tab_id, now_ms);
        let banner = self.apply_access_mode(mode.clone());
        (mode, vec![banner])
    }

    /// Export a `.lumencraft` archive copy (explicitly allowed even when contended read-only).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if archive export fails.
    pub fn export_lumencraft_bytes(&mut self) -> Result<Vec<u8>, WebAdapterError> {
        self.fs.export_lumencraft_bytes(&mut self.host)
    }

    /// Export a `.lumencraft.d/` file tree copy (explicitly allowed even when contended read-only).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if virtual tree export fails.
    pub fn export_directory_tree(&mut self) -> Result<BTreeMap<String, Vec<u8>>, WebAdapterError> {
        self.fs.export_directory_tree(&mut self.host)
    }

    /// Build a multi-flavor clipboard payload (`PDF`/`SVG`/`PNG`/`HTML`/`TSV`), allowed in read-only mode.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Clipboard`] if `dpi` is invalid or no sheet can be exported.
    pub fn build_clipboard_payload(&mut self, sheet_id: Option<&str>, dpi: u16) -> Result<WebClipboardPayload, WebAdapterError> {
        if !(MIN_CLIPBOARD_DPI..=MAX_CLIPBOARD_DPI).contains(&dpi) {
            return Err(WebAdapterError::Clipboard(format!("clipboard DPI {dpi} out of supported range {MIN_CLIPBOARD_DPI}..={MAX_CLIPBOARD_DPI}")));
        }

        let decimal_sep = if self.host.app().locale.starts_with("de") || self.host.app().locale.starts_with("fr") { ',' } else { '.' };
        let session = self.host.app_mut().session_mut();
        let doc = session.document();

        let render_sheet_id = sheet_id
            .and_then(|raw| {
                doc.graph_sheets
                    .iter()
                    .find(|s| s.id.as_str() == raw)
                    .map(|s| s.id.clone())
                    .or_else(|| doc.layout_sheets.iter().find(|s| s.id.as_str() == raw).map(|s| s.id.clone()))
            })
            .or_else(|| doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()).cloned())
            .or_else(|| doc.graph_sheets.first().map(|s| s.id.clone()))
            .or_else(|| doc.layout_sheets.first().map(|s| s.id.clone()));

        let data_sheet = sheet_id
            .and_then(|raw| doc.data_sheets.iter().find(|s| s.id.as_str() == raw))
            .or_else(|| doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id)))
            .or_else(|| doc.data_sheets.first());

        let tsv = match data_sheet {
            Some(ds) => Some(ClipboardBundle::tsv_from_datasheet(ds, decimal_sep).map_err(WebAdapterError::Clipboard)?),
            None => None,
        };

        if let Some(target_id) = render_sheet_id {
            let bundle = ClipboardBundle::from_sheet(session, &target_id, dpi).map_err(WebAdapterError::Clipboard)?;
            let mut flavors: Vec<String> = bundle.available_flavors().into_iter().filter(|f| *f != "emf").map(str::to_owned).collect();
            if tsv.as_ref().is_some_and(|s| !s.is_empty()) {
                flavors.push(String::from("tsv"));
            }
            return Ok(WebClipboardPayload {
                pdf_bytes: bundle.pdf_bytes,
                svg: bundle.svg,
                html: bundle.html,
                png_bytes: bundle.png_bytes,
                plain_text: tsv.clone(),
                tsv,
                width_pt: bundle.width_pt,
                height_pt: bundle.height_pt,
                width_px: bundle.width_px,
                height_px: bundle.height_px,
                dpi: bundle.dpi,
                flavors,
            });
        }

        if let Some(tsv_text) = tsv {
            let escaped = escape_html_text(&tsv_text);
            let html = format!("<!DOCTYPE html><html><body><pre>{escaped}</pre></body></html>");
            let mut flavors = vec![String::from("html")];
            if !tsv_text.is_empty() {
                flavors.push(String::from("tsv"));
            }
            return Ok(WebClipboardPayload {
                pdf_bytes: Vec::new(),
                svg: String::new(),
                html,
                png_bytes: Vec::new(),
                plain_text: Some(tsv_text.clone()),
                tsv: Some(tsv_text),
                width_pt: 0.0,
                height_pt: 0.0,
                width_px: 0,
                height_px: 0,
                dpi,
                flavors,
            });
        }

        Err(WebAdapterError::Clipboard(String::from("no printable graph/layout or data sheet available for clipboard export")))
    }

    /// Build a 100%-scale publication print command with vector `<svg>` pages + `@media print` HTML.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Print`] if no printable sheet exists or spooling fails.
    pub fn build_print_command(&mut self, sheet_id: Option<&str>, scale_percent: f64) -> Result<WebBridgeCommand, WebAdapterError> {
        let mut spec = PrintJobSpec::publication_100_percent();
        spec.scale_percent = scale_percent;

        let target_ids: Vec<String> = {
            let doc = self.host.app().document();
            if let Some(raw) = sheet_id {
                let found = doc
                    .graph_sheets
                    .iter()
                    .find(|s| s.id.as_str() == raw)
                    .map(|s| s.id.clone())
                    .or_else(|| doc.layout_sheets.iter().find(|s| s.id.as_str() == raw).map(|s| s.id.clone()))
                    .ok_or_else(|| WebAdapterError::Print(format!("printable graph or layout sheet `{raw}` not found")))?;
                spec.sheet_ids.push(found.clone());
                vec![found.as_str().to_owned()]
            } else {
                doc.graph_sheets.iter().map(|s| s.id.as_str().to_owned()).chain(doc.layout_sheets.iter().map(|s| s.id.as_str().to_owned())).collect()
            }
        };

        let title = self.host.app().document().title.clone();
        let spool = spec.spool_pdf(self.host.app_mut().session_mut()).map_err(WebAdapterError::Print)?;

        let mut svg_pages = Vec::with_capacity(target_ids.len());
        for id in &target_ids {
            let req = json!({ "id": "print", "method": "render.svg", "params": { "sheet_id": id } }).to_string();
            let reply_raw = self.host.dispatch_control_line(&req);
            let reply: Value = serde_json::from_str(&reply_raw).map_err(|err| WebAdapterError::Print(err.to_string()))?;
            if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                let err_msg = reply.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("SVG print render failed");
                return Err(WebAdapterError::Print(err_msg.to_owned()));
            }
            let svg = reply
                .get("result")
                .and_then(|r| r.get("svg"))
                .and_then(Value::as_str)
                .ok_or_else(|| WebAdapterError::Print(String::from("render.svg reply missing `svg`")))?;
            svg_pages.push(svg.to_owned());
        }

        let html = build_print_html_document(&title, &svg_pages);
        Ok(WebBridgeCommand::TriggerPrintHtml {
            title,
            svg_pages,
            html,
            pdf_bytes: spool.pdf_bytes,
            scale_percent: spool.scale_percent,
            page_sizes_pt: spool.page_sizes_pt,
        })
    }

    /// Dispatch a typed [`WebBridgeEvent`] into the workspace and return host DOM commands.
    ///
    /// Rejects mutating operations when [`Self::access_mode`] is [`TabAccessMode::ReadOnlyContended`]
    /// while permitting viewport pan/zoom, filters, frame rendering, clipboard copy, and print export.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::ReadOnlyContended`] when contended or another typed adapter error.
    pub fn handle_event(&mut self, event: WebBridgeEvent) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        match event {
            WebBridgeEvent::ResizeViewport { logical_width, logical_height, scale_factor } => {
                let metrics = ViewportMetrics::new(logical_width, logical_height, scale_factor);
                self.host.set_viewport(metrics);
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("viewport.resize")), None)?])
            }
            WebBridgeEvent::TouchStart { points, dt_sec } => {
                let events = self.touch.touch_start(&points, dt_sec);
                self.dispatch_touch_events(events)
            }
            WebBridgeEvent::TouchMove { points, dt_sec } => {
                let events = self.touch.touch_move(&points, dt_sec);
                self.dispatch_touch_events(events)
            }
            WebBridgeEvent::TouchEnd { released_ids, dt_sec } => {
                let events = self.touch.touch_end(&released_ids, dt_sec);
                self.dispatch_touch_events(events)
            }
            WebBridgeEvent::TouchCancel => {
                let events = self.touch.touch_cancel();
                self.dispatch_touch_events(events)
            }
            WebBridgeEvent::CoastStep { dt_sec } => {
                let coast = self.touch.step_inertia(dt_sec);
                self.dispatch_single_ui_input(coast)
            }
            WebBridgeEvent::UiInput { event: input } => {
                if ui_input_requires_write_lock(&input) {
                    self.require_write_lock(ui_input_label(&input))?;
                }
                self.dispatch_single_ui_input(input)
            }
            WebBridgeEvent::ExecuteCommand { command_id, params } => self.handle_execute_command(&command_id, params),
            WebBridgeEvent::DispatchControlLine { line } => self.handle_control_line(&line),
            WebBridgeEvent::RenderFrame { dpi } => {
                self.render_dpi = dpi;
                Ok(vec![self.render_frame_command(dpi, Some(String::from("render.frame")), None)?])
            }
            WebBridgeEvent::CopyClipboard { sheet_id, dpi } => {
                let payload = self.build_clipboard_payload(sheet_id.as_deref(), dpi)?;
                Ok(vec![WebBridgeCommand::WriteClipboardItems { payload }])
            }
            WebBridgeEvent::PrintCurrentSheet { sheet_id, scale_percent } => Ok(vec![self.build_print_command(sheet_id.as_deref(), scale_percent)?]),
            WebBridgeEvent::OpenFileBytes { file_name, bytes } => {
                self.require_write_lock("open_file_bytes")?;
                let summary = self.fs.import_file_bytes(&mut self.host, &file_name, &bytes)?;
                Ok(vec![self.summary_render_command("open_file_bytes", &summary)?])
            }
            WebBridgeEvent::ImportDroppedFiles { files } => {
                self.require_write_lock("import_dropped_files")?;
                let summary = self.fs.import_dropped_files(&mut self.host, files)?;
                Ok(vec![self.summary_render_command("import_dropped_files", &summary)?])
            }
            WebBridgeEvent::ImportDirectoryEntries { root_name, entries } => {
                self.require_write_lock("import_directory_entries")?;
                let summary = self.fs.import_directory_tree(&mut self.host, &root_name, entries)?;
                Ok(vec![self.summary_render_command("import_directory_entries", &summary)?])
            }
            WebBridgeEvent::SyncDirectory { root_name } => {
                self.require_write_lock("sync_directory")?;
                let plan = self.fs.sync_directory_from_host(&mut self.host, root_name.as_deref())?;
                let resolved_root = self.fs.last_synced_root().unwrap_or("workspace.lumencraft.d").to_owned();
                Ok(vec![WebBridgeCommand::ApplyDirectorySync { root_name: resolved_root, plan }])
            }
            WebBridgeEvent::SavePreferences { preferences } => {
                let encoded = self.storage.save_preferences(&preferences)?;
                Ok(vec![WebBridgeCommand::PersistLocalStorage {
                    writes: vec![(String::from(PREFERENCES_STORAGE_KEY), encoded)],
                    removals: Vec::new(),
                }])
            }
            WebBridgeEvent::SaveAutosave { now_ms } => self.handle_save_autosave(now_ms),
            WebBridgeEvent::AcquireLock { doc_id, tab_id, now_ms } => {
                self.doc_id = require_non_empty(&doc_id, "doc_id")?.to_owned();
                self.tab_id = require_non_empty(&tab_id, "tab_id")?.to_owned();
                let acq = self.locks.acquire_document_lock(&self.doc_id, &self.tab_id, now_ms)?;
                let banner = self.apply_access_mode(acq.mode);
                Ok(vec![banner])
            }
            WebBridgeEvent::ObserveExternalLock { doc_id, holder_tab_id, now_ms } => {
                self.doc_id = require_non_empty(&doc_id, "doc_id")?.to_owned();
                self.locks.observe_external_holder(&self.doc_id, &holder_tab_id, now_ms)?;
                let mode = self.locks.access_mode(&self.doc_id, &self.tab_id, now_ms);
                let banner = self.apply_access_mode(mode);
                Ok(vec![banner])
            }
            WebBridgeEvent::RenewHeartbeat { now_ms } => {
                let _renewed = self.locks.renew_heartbeat(&self.doc_id, &self.tab_id, now_ms)?;
                Ok(Vec::new())
            }
            WebBridgeEvent::ReleaseLock { now_ms } => {
                let _outcome = self.locks.release_document_lock(&self.doc_id, &self.tab_id, now_ms)?;
                let mode = self.locks.access_mode(&self.doc_id, &self.tab_id, now_ms);
                let banner = self.apply_access_mode(mode);
                Ok(vec![banner])
            }
            WebBridgeEvent::SetPreviewFilter { filter } => {
                self.host.set_preview_filter(filter);
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("ui.previewFilter.set")), None)?])
            }
            WebBridgeEvent::SetPresentationMode { enabled } => {
                self.host.set_presentation_mode(enabled);
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("ui.presentation.set")), None)?])
            }
        }
    }

    /// Parse raw JSON [`WebBridgeEvent`], execute it, and return serialized JSON command array.
    ///
    /// # Errors
    /// Returns [`WebAdapterError`] if parsing fails or dispatch returns an error.
    pub fn handle_event_json(&mut self, raw_event_json: &str) -> Result<String, WebAdapterError> {
        let event = WebBridgeEvent::from_json_str(raw_event_json)?;
        let commands = self.handle_event(event)?;
        Self::commands_to_json_string(&commands)
    }

    /// Serialize outgoing bridge commands to JSON text.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] if JSON formatting fails.
    pub fn commands_to_json_string(commands: &[WebBridgeCommand]) -> Result<String, WebAdapterError> {
        let array: Vec<Value> = commands.iter().map(WebBridgeCommand::to_json_value).collect();
        serde_json::to_string(&Value::Array(array)).map_err(|err| WebAdapterError::Serialization(err.to_string()))
    }

    /// Deserialize a JSON command array produced by [`Self::commands_to_json_string`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] on invalid JSON or command shape.
    pub fn commands_from_json_str(raw: &str) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let value: Value = serde_json::from_str(raw).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let arr = value.as_array().ok_or_else(|| WebAdapterError::Serialization(String::from("expected JSON array of bridge commands")))?;
        arr.iter().map(WebBridgeCommand::from_json_value).collect()
    }

    fn require_write_lock(&self, operation: &str) -> Result<(), WebAdapterError> {
        if let TabAccessMode::ReadOnlyContended { holder_tab_id } = &self.access_mode {
            return Err(WebAdapterError::ReadOnlyContended {
                doc_id: self.doc_id.clone(),
                holder_tab_id: holder_tab_id.clone(),
                operation: operation.to_owned(),
            });
        }
        Ok(())
    }

    fn apply_access_mode(&mut self, mode: TabAccessMode) -> WebBridgeCommand {
        self.access_mode = mode.clone();
        match mode {
            TabAccessMode::ExclusiveReadWrite => {
                WebBridgeCommand::SetReadOnlyBanner { doc_id: self.doc_id.clone(), read_only: false, holder_tab_id: None, message: None }
            }
            TabAccessMode::ReadOnlyContended { holder_tab_id } => WebBridgeCommand::SetReadOnlyBanner {
                doc_id: self.doc_id.clone(),
                read_only: true,
                message: Some(format!(
                    "Read-only mode: document `{}` is currently open for editing in tab `{holder_tab_id}`. Pan, zoom, and exports remain enabled.",
                    self.doc_id
                )),
                holder_tab_id: Some(holder_tab_id),
            },
        }
    }

    fn dispatch_touch_events(&mut self, events: Vec<UiInputEvent>) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let mut last_outcome: Option<UiEventOutcome> = None;
        for ev in events {
            if self.access_mode.is_read_only() && ui_input_requires_write_lock(&ev) {
                continue;
            }
            let outcome = self.host.dispatch_event(ev).map_err(WebAdapterError::Workspace)?;
            if outcome.handled {
                last_outcome = Some(outcome);
            }
        }
        if let Some(outcome) = last_outcome {
            Ok(vec![self.render_frame_command(self.render_dpi, Some(outcome.action), outcome.command_result)?])
        } else {
            Ok(Vec::new())
        }
    }

    fn dispatch_single_ui_input(&mut self, input: UiInputEvent) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let outcome = self.host.dispatch_event(input).map_err(WebAdapterError::Workspace)?;
        if outcome.handled {
            Ok(vec![self.render_frame_command(self.render_dpi, Some(outcome.action), outcome.command_result)?])
        } else {
            Ok(Vec::new())
        }
    }

    fn handle_execute_command(&mut self, command_id: &str, params: Value) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        if self.access_mode.is_read_only() && !is_command_read_only(command_id, Some(&params)) {
            return Err(self.read_only_err(command_id));
        }

        if cmd::registry().get(command_id).is_some() {
            let result = self.host.execute_command(command_id, params).map_err(WebAdapterError::Workspace)?;
            return Ok(vec![self.render_frame_command(self.render_dpi, Some(command_id.to_owned()), Some(result))?]);
        }

        let line = json!({ "id": "bridge", "method": command_id, "params": params }).to_string();
        let reply_raw = self.host.dispatch_control_line(&line);
        let reply: Value = serde_json::from_str(&reply_raw).map_err(|err| WebAdapterError::Workspace(err.to_string()))?;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let err_msg = reply.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("control command failed");
            return Err(WebAdapterError::Workspace(err_msg.to_owned()));
        }
        Ok(vec![self.render_frame_command(self.render_dpi, Some(command_id.to_owned()), reply.get("result").cloned())?])
    }

    fn handle_control_line(&mut self, line: &str) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let req: Value = serde_json::from_str(line).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let method = req
            .get("method")
            .or_else(|| req.get("op"))
            .and_then(Value::as_str)
            .ok_or_else(|| WebAdapterError::Serialization(String::from("control request missing `method` or `op`")))?;
        let params = req.get("params");

        if self.access_mode.is_read_only() && !is_command_read_only(method, params) {
            return Err(self.read_only_err(method));
        }

        let reply_raw = self.host.dispatch_control_line(line);
        let reply: Value = serde_json::from_str(&reply_raw).map_err(|err| WebAdapterError::Workspace(err.to_string()))?;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let err_msg = reply.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("control line failed");
            return Err(WebAdapterError::Workspace(err_msg.to_owned()));
        }
        Ok(vec![self.render_frame_command(self.render_dpi, Some(method.to_owned()), Some(reply))?])
    }

    fn handle_save_autosave(&mut self, now_ms: u64) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        self.require_write_lock("save_autosave")?;
        let snapshot = serde_json::to_string(self.host.app().document()).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let outcome = self.storage.save_autosave_snapshot(&self.doc_id, &snapshot, now_ms)?;
        Ok(vec![WebBridgeCommand::PersistLocalStorage { writes: vec![(outcome.storage_key, snapshot)], removals: outcome.evicted_keys }])
    }

    fn summary_render_command(&mut self, action: &str, summary: &WorkspaceSummary) -> Result<WebBridgeCommand, WebAdapterError> {
        let summary_json = json!({
            "title": summary.title,
            "format_tag": summary.format_tag,
            "sheet_count": summary.sheet_count,
            "active_sheet_id": summary.active_sheet_id,
            "dirty": summary.dirty,
        });
        self.render_frame_command(self.render_dpi, Some(action.to_owned()), Some(summary_json))
    }

    fn render_frame_command(&mut self, dpi: u16, action: Option<String>, command_result: Option<Value>) -> Result<WebBridgeCommand, WebAdapterError> {
        let frame = self.host.render_rgba(dpi).map_err(|err| WebAdapterError::Workspace(err.to_string()))?;
        let pan_zoom = &self.host.app().canvas_dialogs.pan_zoom;
        Ok(WebBridgeCommand::RenderRgbaFrame {
            width_px: frame.width_px,
            height_px: frame.height_px,
            dpi,
            rgba_bytes: frame.rgba,
            zoom: pan_zoom.zoom,
            pan_pt: pan_zoom.pan_pt,
            action,
            command_result,
        })
    }

    fn read_only_err(&self, operation: &str) -> WebAdapterError {
        let holder = self.access_mode.holder_tab_id().unwrap_or("unknown").to_owned();
        WebAdapterError::ReadOnlyContended { doc_id: self.doc_id.clone(), holder_tab_id: holder, operation: operation.to_owned() }
    }
}

fn ui_input_requires_write_lock(event: &UiInputEvent) -> bool {
    match event {
        UiInputEvent::PinchZoom { .. }
        | UiInputEvent::InertialPan { .. }
        | UiInputEvent::WheelZoom { .. }
        | UiInputEvent::PointerMove { .. }
        | UiInputEvent::CancelInlineEdit => false,
        UiInputEvent::PointerDown { .. }
        | UiInputEvent::PointerUp { .. }
        | UiInputEvent::DoubleClick { .. }
        | UiInputEvent::KeyDown { .. }
        | UiInputEvent::TextInput { .. }
        | UiInputEvent::CommitInlineEdit => true,
    }
}

fn ui_input_label(event: &UiInputEvent) -> &'static str {
    match event {
        UiInputEvent::PointerDown { .. } => "ui.pointer_down",
        UiInputEvent::PointerMove { .. } => "ui.pointer_move",
        UiInputEvent::PointerUp { .. } => "ui.pointer_up",
        UiInputEvent::DoubleClick { .. } => "ui.double_click",
        UiInputEvent::WheelZoom { .. } => "ui.wheel_zoom",
        UiInputEvent::PinchZoom { .. } => "ui.pinch_zoom",
        UiInputEvent::InertialPan { .. } => "ui.inertial_pan",
        UiInputEvent::KeyDown { .. } => "ui.key_down",
        UiInputEvent::TextInput { .. } => "ui.text_input",
        UiInputEvent::CommitInlineEdit => "ui.commit_inline_edit",
        UiInputEvent::CancelInlineEdit => "ui.cancel_inline_edit",
    }
}

fn is_command_read_only(command_id: &str, params: Option<&Value>) -> bool {
    if let Some(spec) = cmd::registry().get(command_id) {
        return spec.effects.read_only_hint();
    }
    if command_id == "engine.execute"
        && let Some(p) = params
        && let Some(inner) = p.get("command_id").or_else(|| p.get("command")).or_else(|| p.get("op")).and_then(Value::as_str)
    {
        return is_command_read_only(inner, p.get("params"));
    }
    if command_id == "batch.run" {
        return params.and_then(|p| p.get("commands")).and_then(Value::as_array).is_some_and(|cmds| {
            cmds.iter().all(|entry| {
                entry
                    .get("command")
                    .or_else(|| entry.get("command_id"))
                    .and_then(Value::as_str)
                    .is_some_and(|id| is_command_read_only(id, entry.get("params")))
            })
        });
    }
    matches!(
        command_id,
        "render.svg"
            | "render.pdf"
            | "doc.inspect"
            | "ui.state"
            | "state.snapshot"
            | "app.about"
            | "ui.previewFilter.set"
            | "ui.presentation.set"
            | "ui.presentation.toggle"
            | "ui.rulers.toggle"
            | "ui.theme.set"
            | "ui.locale.set"
            | "ui.palette.open"
            | "ui.about.open"
            | "ui.sheet.select"
            | "ui.split.set"
            | "ui.grid.selectRange"
            | "ui.clipboard.copy"
            | "ui.export.figure"
            | "ui.print.open"
            | "file.print"
            | "ui.navigator.toggle"
            | "ui.inspector.toggle"
    )
}

fn build_print_html_document(title: &str, svg_pages: &[String]) -> String {
    let escaped_title = escape_html_text(title);
    let mut pages_html = String::new();
    for (idx, svg) in svg_pages.iter().enumerate() {
        pages_html.push_str("<section class=\"lumencraft-print-page\" data-page=\"");
        pages_html.push_str(&(idx + 1).to_string());
        pages_html.push_str("\">");
        pages_html.push_str(svg);
        pages_html.push_str("</section>");
    }
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{escaped_title}</title>\
         <style>@page{{margin:0}}body{{margin:0;background:#fff}}.lumencraft-print-page{{page-break-after:always;break-after:page;}}\
         .lumencraft-print-page:last-child{{page-break-after:auto;break-after:auto;}}svg{{display:block;max-width:100%;height:auto;}}</style>\
         </head><body>{pages_html}</body></html>"
    )
}

fn escape_html_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn touch_points_to_json(points: &[TouchPoint]) -> Value {
    Value::Array(points.iter().map(|pt| json!({ "id": pt.id, "x": pt.x, "y": pt.y })).collect())
}

fn files_to_json_array(files: &[(String, Vec<u8>)]) -> Value {
    Value::Array(files.iter().map(|(path, bytes)| json!({ "path": path, "bytes_base64": encode_base64(bytes) })).collect())
}

fn map_to_json_array(entries: &BTreeMap<String, Vec<u8>>) -> Value {
    Value::Array(entries.iter().map(|(path, bytes)| json!({ "path": path, "bytes_base64": encode_base64(bytes) })).collect())
}

fn require_object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, WebAdapterError> {
    value.as_object().ok_or_else(|| WebAdapterError::Serialization(format!("{label} must be a JSON object")))
}

fn require_str_field<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a str, WebAdapterError> {
    obj.get(key).and_then(Value::as_str).ok_or_else(|| WebAdapterError::Serialization(format!("missing string field `{key}`")))
}

fn optional_str_field(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn require_f64_field(obj: &Map<String, Value>, key: &str) -> Result<f64, WebAdapterError> {
    obj.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| WebAdapterError::Serialization(format!("missing finite numeric field `{key}`")))
}

fn require_f64_or_default(obj: &Map<String, Value>, key: &str, default: f64) -> Result<f64, WebAdapterError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| WebAdapterError::Serialization(format!("field `{key}` must be a finite number")))
        }
    }
}

fn require_u16_or_default(obj: &Map<String, Value>, key: &str, default: u16) -> u16 {
    obj.get(key).and_then(Value::as_u64).and_then(|v| u16::try_from(v).ok()).unwrap_or(default)
}

fn require_u64_or_default(obj: &Map<String, Value>, key: &str, default: u64) -> u64 {
    obj.get(key).and_then(Value::as_u64).unwrap_or(default)
}

fn parse_touch_points_field(obj: &Map<String, Value>, key: &str) -> Result<Vec<TouchPoint>, WebAdapterError> {
    let arr = obj.get(key).and_then(Value::as_array).ok_or_else(|| WebAdapterError::Serialization(format!("missing array field `{key}`")))?;
    let mut points = Vec::with_capacity(arr.len());
    for item in arr {
        let pt_obj = require_object(item, "touch point")?;
        let id = pt_obj
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| WebAdapterError::Serialization(String::from("touch point missing integer `id`")))?;
        let x = require_f64_field(pt_obj, "x")?;
        let y = require_f64_field(pt_obj, "y")?;
        points.push(TouchPoint { id, x, y });
    }
    Ok(points)
}

fn parse_i64_array(obj: &Map<String, Value>, key: &str) -> Result<Vec<i64>, WebAdapterError> {
    let arr = obj.get(key).and_then(Value::as_array).ok_or_else(|| WebAdapterError::Serialization(format!("missing array field `{key}`")))?;
    arr.iter().map(|v| v.as_i64().ok_or_else(|| WebAdapterError::Serialization(format!("array `{key}` entries must be integers")))).collect()
}

fn parse_named_bytes_vec(obj: &Map<String, Value>, key: &str) -> Result<Vec<(String, Vec<u8>)>, WebAdapterError> {
    let arr = obj.get(key).and_then(Value::as_array).ok_or_else(|| WebAdapterError::Serialization(format!("missing array field `{key}`")))?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let item = require_object(entry, "named byte entry")?;
        let path = require_str_field(item, "path")?.to_owned();
        let bytes = decode_base64_field(item, "bytes_base64", "bytes")?;
        out.push((path, bytes));
    }
    Ok(out)
}

fn parse_string_pairs(obj: &Map<String, Value>, key: &str) -> Result<Vec<(String, String)>, WebAdapterError> {
    let arr = obj.get(key).and_then(Value::as_array).ok_or_else(|| WebAdapterError::Serialization(format!("missing array field `{key}`")))?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let item = require_object(entry, "key/value entry")?;
        let k = require_str_field(item, "key")?.to_owned();
        let v = require_str_field(item, "value")?.to_owned();
        out.push((k, v));
    }
    Ok(out)
}

fn parse_pan_pt(value: Option<&Value>) -> Result<[f64; 2], WebAdapterError> {
    let Some(arr) = value.and_then(Value::as_array) else {
        return Ok([0.0, 0.0]);
    };
    if arr.len() != 2 {
        return Err(WebAdapterError::Serialization(String::from("`pan_pt` must contain exactly 2 elements")));
    }
    let x = arr[0].as_f64().filter(|v| v.is_finite()).ok_or_else(|| WebAdapterError::Serialization(String::from("invalid `pan_pt[0]`")))?;
    let y = arr[1].as_f64().filter(|v| v.is_finite()).ok_or_else(|| WebAdapterError::Serialization(String::from("invalid `pan_pt[1]`")))?;
    Ok([x, y])
}

fn parse_page_sizes(value: Option<&Value>) -> Result<Vec<(f64, f64)>, WebAdapterError> {
    let Some(arr) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut sizes = Vec::with_capacity(arr.len());
    for pair in arr {
        let pair_arr = pair.as_array().ok_or_else(|| WebAdapterError::Serialization(String::from("page size must be [w, h]")))?;
        if pair_arr.len() != 2 {
            return Err(WebAdapterError::Serialization(String::from("page size pair must have length 2")));
        }
        let w = pair_arr[0].as_f64().filter(|v| v.is_finite()).ok_or_else(|| WebAdapterError::Serialization(String::from("invalid page width")))?;
        let h = pair_arr[1].as_f64().filter(|v| v.is_finite()).ok_or_else(|| WebAdapterError::Serialization(String::from("invalid page height")))?;
        sizes.push((w, h));
    }
    Ok(sizes)
}

fn decode_base64_field(obj: &Map<String, Value>, b64_key: &str, array_key: &str) -> Result<Vec<u8>, WebAdapterError> {
    if let Some(raw_b64) = obj.get(b64_key).and_then(Value::as_str) {
        return decode_base64(raw_b64);
    }
    if let Some(arr) = obj.get(array_key).and_then(Value::as_array) {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            let byte = item
                .as_u64()
                .and_then(|v| u8::try_from(v).ok())
                .ok_or_else(|| WebAdapterError::Serialization(format!("`{array_key}` entries must be bytes 0..=255")))?;
            out.push(byte);
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(input: &[u8]) -> String {
    if input.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = usize::from(chunk[0]);
        let b1 = usize::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = usize::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(char::from(B64_ALPHABET[(triple >> 18) & 0x3f]));
        out.push(char::from(B64_ALPHABET[(triple >> 12) & 0x3f]));
        if chunk.len() > 1 {
            out.push(char::from(B64_ALPHABET[(triple >> 6) & 0x3f]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(B64_ALPHABET[triple & 0x3f]));
        } else {
            out.push('=');
        }
    }
    out
}

fn decode_base64(input: &str) -> Result<Vec<u8>, WebAdapterError> {
    let compact: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if compact.is_empty() {
        return Ok(Vec::new());
    }
    if !compact.len().is_multiple_of(4) {
        return Err(WebAdapterError::Serialization(String::from("base64 length must be a multiple of 4")));
    }

    let mut out = Vec::with_capacity((compact.len() / 4) * 3);
    let (chunks, _rem) = compact.as_chunks::<4>();
    for (chunk_idx, chunk) in chunks.iter().enumerate() {
        let is_last = chunk_idx + 1 == compact.len() / 4;
        let v0 = decode_b64_digit(chunk[0])?;
        let v1 = decode_b64_digit(chunk[1])?;
        let pad2 = chunk[2] == b'=';
        let pad3 = chunk[3] == b'=';
        if (pad2 && !pad3) || ((pad2 || pad3) && !is_last) {
            return Err(WebAdapterError::Serialization(String::from("invalid base64 padding")));
        }
        let v2 = if pad2 { 0 } else { decode_b64_digit(chunk[2])? };
        let v3 = if pad3 { 0 } else { decode_b64_digit(chunk[3])? };
        let triple = (u32::from(v0) << 18) | (u32::from(v1) << 12) | (u32::from(v2) << 6) | u32::from(v3);

        out.push(((triple >> 16) & 0xff) as u8);
        if !pad2 {
            out.push(((triple >> 8) & 0xff) as u8);
        }
        if !pad3 {
            out.push((triple & 0xff) as u8);
        }
    }
    Ok(out)
}

fn decode_b64_digit(byte: u8) -> Result<u8, WebAdapterError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(WebAdapterError::Serialization(format!("invalid base64 byte 0x{byte:02x}"))),
    }
}
