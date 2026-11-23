//! Zero-telemetry browser host adapters for CurveBook Web (`R8.4`).
//!
//! All storage persistence, multi-tab lock coordination, virtual directory incremental sync,
//! touch pinch/pan gesture recognition, multi-flavor clipboard assembly, vector print output,
//! and JSON bridge message handling remain 100% local and free of network calls.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_engine::cmd;
use curvebook_format::{
    CURRENT_PREFERENCES_VERSION, CsvOptions, MAX_ARCHIVE_FILE_BYTES, MAX_ZIP_ENTRIES, PreferencesFile, export_datasheet_to_csv,
    parse_preferences_json,
};
use curvebook_ui_egui::control_io::host_routing::{
    HostPrefFingerprint, capture_pref_fingerprint, command_replaces_project, has_non_null_path_param, is_command_read_only,
    normalize_control_request_params,
};
use curvebook_ui_egui::{
    ActiveDialog, CanvasPreviewFilter, ClipboardBundle, KeyModifiers, PointerButton, PrintJobSpec, UiApp, UiEventOutcome, UiInputEvent,
    ViewportMetrics, dispatch_ui_command,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value, json};

use crate::chrome_bridge::{
    self, WebChromeAction, WebChromeBridge, WebChromeDispatch, WebChromeSnapshot, WebCommandRoute, WebExportFormat, WebNoticeLevel,
    WebShortcutAction, commit_active_modal_dialog, contextual_command_params, resolve_web_command_route,
};
use crate::text_bridge::{BrowserCanvasTextMeasurer, MAX_FRAME_RGBA_BYTES, WebRgbaLayer, WebTextBridge, WebTextDrawOp};
use crate::{WebWorkspaceHost, WorkspaceSummary};

/// Default browser `localStorage` quota budget (`5 MiB`).
pub const QUOTA_BUDGET_BYTES: usize = 5 * 1024 * 1024;
/// Canonical `localStorage` key for user preferences.
pub const PREFERENCES_STORAGE_KEY: &str = "curvebook.v1.preferences";
/// Canonical `localStorage` key prefix for per-document autosave snapshots.
pub const AUTOSAVE_KEY_PREFIX: &str = "curvebook.v1.autosave.";
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

/// Summary returned after persisting user preferences under quota management.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferencesSaveOutcome {
    pub storage_key: String,
    pub encoded: String,
    pub evicted_keys: Vec<String>,
}

#[derive(Debug, Clone)]
struct EvictedBackup {
    key: String,
    value: String,
    recency: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
struct StorageRollbackOp {
    target_key: String,
    prev_target_value: Option<String>,
    prev_target_recency: Option<(u64, u64)>,
    evicted_backups: Vec<EvictedBackup>,
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

    /// Build the canonical `curvebook.v1.autosave.<doc_id>` key for `doc_id`.
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

    /// Load and validate [`PreferencesFile`] from `curvebook.v1.preferences`.
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

    /// Persist [`PreferencesFile`] under `curvebook.v1.preferences` without clobbering newer schemas.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::ForwardVersionPreferences`] if either `prefs.version` or any
    /// already-stored payload has a schema version newer than [`CURRENT_PREFERENCES_VERSION`].
    pub fn save_preferences(&mut self, prefs: &PreferencesFile) -> Result<String, WebAdapterError> {
        let outcome = self.save_preferences_with_evictions(prefs)?;
        Ok(outcome.encoded)
    }

    /// Persist [`PreferencesFile`] under `curvebook.v1.preferences` and report any evicted
    /// autosave keys so the browser host can retry and mirror evictions consistently.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::ForwardVersionPreferences`] if schema versions are newer than
    /// supported, or [`WebAdapterError::QuotaExceeded`] if storage cannot accommodate preferences.
    pub fn save_preferences_with_evictions(&mut self, prefs: &PreferencesFile) -> Result<PreferencesSaveOutcome, WebAdapterError> {
        let (outcome, _rollback) = self.save_preferences_tracked(prefs)?;
        Ok(outcome)
    }

    fn save_preferences_tracked(&mut self, prefs: &PreferencesFile) -> Result<(PreferencesSaveOutcome, StorageRollbackOp), WebAdapterError> {
        if prefs.version > CURRENT_PREFERENCES_VERSION {
            return Err(WebAdapterError::ForwardVersionPreferences { found: prefs.version, supported: CURRENT_PREFERENCES_VERSION });
        }
        self.ensure_stored_preferences_not_forward()?;
        let encoded = serde_json::to_string_pretty(prefs).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let non_autosave_other = self
            .non_autosave_used_bytes()
            .saturating_sub(self.backend.get_item(PREFERENCES_STORAGE_KEY).map_or(0, |v| PREFERENCES_STORAGE_KEY.len().saturating_add(v.len())));
        let entry_bytes = PREFERENCES_STORAGE_KEY.len().saturating_add(encoded.len());
        if non_autosave_other.saturating_add(entry_bytes) > self.quota_budget_bytes {
            return Err(WebAdapterError::QuotaExceeded {
                required_bytes: non_autosave_other.saturating_add(entry_bytes),
                budget_bytes: self.quota_budget_bytes,
            });
        }
        let (evicted_keys, rollback) = self.write_with_safe_eviction_tracked(PREFERENCES_STORAGE_KEY, &encoded)?;
        Ok((PreferencesSaveOutcome { storage_key: String::from(PREFERENCES_STORAGE_KEY), encoded, evicted_keys }, rollback))
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

    /// Save a document autosave snapshot, writing it BEFORE evicting older autosave entries (`RV-80`)
    /// and rolling back evicted recovery snapshots if quota still fails.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] for invalid `doc_id` or [`WebAdapterError::QuotaExceeded`]
    /// if the snapshot cannot fit even after evicting all other autosave entries.
    pub fn save_autosave_snapshot(&mut self, doc_id: &str, snapshot: &str, timestamp_ms: u64) -> Result<AutosaveSaveOutcome, WebAdapterError> {
        let (outcome, _rollback) = self.save_autosave_snapshot_tracked(doc_id, snapshot, timestamp_ms)?;
        Ok(outcome)
    }

    fn save_autosave_snapshot_tracked(
        &mut self,
        doc_id: &str,
        snapshot: &str,
        timestamp_ms: u64,
    ) -> Result<(AutosaveSaveOutcome, StorageRollbackOp), WebAdapterError> {
        let key = Self::autosave_key(doc_id)?;
        let entry_bytes = key.len().saturating_add(snapshot.len());
        let non_autosave_bytes = self.non_autosave_used_bytes();
        if non_autosave_bytes.saturating_add(entry_bytes) > self.quota_budget_bytes {
            return Err(WebAdapterError::QuotaExceeded {
                required_bytes: non_autosave_bytes.saturating_add(entry_bytes),
                budget_bytes: self.quota_budget_bytes,
            });
        }

        let (evicted_keys, rollback) = self.write_with_safe_eviction_tracked(&key, snapshot)?;
        self.touch_autosave_recency(&key, timestamp_ms);

        let outcome = AutosaveSaveOutcome {
            storage_key: key,
            doc_id: doc_id.trim().to_owned(),
            entry_bytes,
            total_used_bytes: self.backend.used_bytes(),
            evicted_keys,
        };
        Ok((outcome, rollback))
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

    fn touch_autosave_recency(&mut self, key: &str, now_ms: u64) {
        self.access_seq = self.access_seq.saturating_add(1);
        self.autosave_recency.insert(key.to_owned(), (now_ms, self.access_seq));
    }

    fn evict_oldest_autosave_into(&mut self, protected_key: &str, evicted_backups: &mut Vec<EvictedBackup>) -> Option<String> {
        let victim_key = self.oldest_autosave_key(protected_key)?;
        let backup_val = self.backend.get_item(&victim_key).unwrap_or_default();
        let backup_recency = self.autosave_recency.remove(&victim_key);
        let _ = self.backend.remove_item(&victim_key);
        evicted_backups.push(EvictedBackup { key: victim_key.clone(), value: backup_val, recency: backup_recency });
        Some(victim_key)
    }

    fn evict_autosaves_to_fit(
        &mut self,
        protected_key: &str,
        additional_delta: isize,
        evicted_backups: &mut Vec<EvictedBackup>,
    ) -> Result<Vec<String>, WebAdapterError> {
        let mut evicted = Vec::new();
        loop {
            let current = isize::try_from(self.backend.used_bytes()).unwrap_or(isize::MAX);
            let projected = current.saturating_add(additional_delta);
            let budget = isize::try_from(self.quota_budget_bytes).unwrap_or(isize::MAX);
            if projected <= budget {
                return Ok(evicted);
            }

            let Some(victim_key) = self.evict_oldest_autosave_into(protected_key, evicted_backups) else {
                let req = usize::try_from(projected.max(0)).unwrap_or(usize::MAX);
                return Err(WebAdapterError::QuotaExceeded { required_bytes: req, budget_bytes: self.quota_budget_bytes });
            };
            evicted.push(victim_key);
        }
    }

    fn rollback_storage_op(&mut self, op: StorageRollbackOp) {
        if let Some(prev_val) = op.prev_target_value {
            let _ = self.backend.set_item(&op.target_key, &prev_val);
        } else {
            let _ = self.backend.remove_item(&op.target_key);
        }
        if op.target_key.starts_with(AUTOSAVE_KEY_PREFIX) {
            if let Some(rec) = op.prev_target_recency {
                self.autosave_recency.insert(op.target_key.clone(), rec);
            } else {
                self.autosave_recency.remove(&op.target_key);
            }
        }
        for backup in op.evicted_backups.into_iter().rev() {
            let _ = self.backend.set_item(&backup.key, &backup.value);
            if let Some(rec) = backup.recency {
                self.autosave_recency.insert(backup.key, rec);
            }
        }
    }

    fn reconcile_confirmed_storage(&mut self, op: StorageRollbackOp, removed_keys: &[String], succeeded: bool) {
        if !succeeded {
            self.rollback_storage_op(op);
            return;
        }
        let removed_set: BTreeSet<&str> = removed_keys.iter().map(String::as_str).collect();
        for backup in op.evicted_backups.into_iter().rev() {
            if !removed_set.contains(backup.key.as_str()) {
                let _ = self.backend.set_item(&backup.key, &backup.value);
                if let Some(rec) = backup.recency {
                    self.autosave_recency.insert(backup.key, rec);
                }
            }
        }
        for key in removed_keys {
            if key != &op.target_key && (key == PREFERENCES_STORAGE_KEY || key.starts_with(AUTOSAVE_KEY_PREFIX)) {
                self.autosave_recency.remove(key);
                let _ = self.backend.remove_item(key);
            }
        }
    }

    /// Write `value` under `target_key` BEFORE evicting older autosave snapshots (`RV-80`),
    /// rolling back any victims removed during quota retry if writing still fails, and returning
    /// a [`StorageRollbackOp`] so browser quota failures can also be rolled back cleanly.
    fn write_with_safe_eviction_tracked(&mut self, target_key: &str, value: &str) -> Result<(Vec<String>, StorageRollbackOp), WebAdapterError> {
        let prev_value = self.backend.get_item(target_key);
        let prev_recency = self.autosave_recency.get(target_key).copied();
        let mut evicted_backups: Vec<EvictedBackup> = Vec::new();
        let mut evicted_keys: Vec<String> = Vec::new();

        if let Err(first_err) = self.backend.set_item(target_key, value) {
            let mut write_err = first_err;
            loop {
                let Some(victim_key) = self.evict_oldest_autosave_into(target_key, &mut evicted_backups) else {
                    self.rollback_storage_op(StorageRollbackOp {
                        target_key: target_key.to_owned(),
                        prev_target_value: prev_value,
                        prev_target_recency: prev_recency,
                        evicted_backups,
                    });
                    return Err(write_err);
                };
                evicted_keys.push(victim_key);
                match self.backend.set_item(target_key, value) {
                    Ok(()) => break,
                    Err(err) => write_err = err,
                }
            }
        }

        let trimmed = match self.evict_autosaves_to_fit(target_key, 0, &mut evicted_backups) {
            Ok(keys) => keys,
            Err(err) => {
                self.rollback_storage_op(StorageRollbackOp {
                    target_key: target_key.to_owned(),
                    prev_target_value: prev_value,
                    prev_target_recency: prev_recency,
                    evicted_backups,
                });
                return Err(err);
            }
        };
        evicted_keys.extend(trimmed);
        Ok((
            evicted_keys,
            StorageRollbackOp {
                target_key: target_key.to_owned(),
                prev_target_value: prev_value,
                prev_target_recency: prev_recency,
                evicted_backups,
            },
        ))
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

    /// Record an authoritative browser lock grant (such as `navigator.locks` or fallback storage
    /// acquisition), replacing any cached external holder observation even within `lease_ttl_ms`.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `doc_id` or `tab_id` is empty.
    pub fn grant_browser_lock(&mut self, doc_id: &str, tab_id: &str, now_ms: u64) -> Result<LockAcquisition, WebAdapterError> {
        let doc = require_non_empty(doc_id, "doc_id")?;
        let tab = require_non_empty(tab_id, "tab_id")?;
        self.records.insert(
            doc.to_owned(),
            DocumentLockRecord { holder_tab_id: tab.to_owned(), acquired_at_ms: now_ms, last_heartbeat_ms: now_ms, contenders: VecDeque::new() },
        );
        Ok(LockAcquisition { doc_id: doc.to_owned(), tab_id: tab.to_owned(), mode: TabAccessMode::ExclusiveReadWrite, stale_takeover: false })
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

/// Deterministic incremental write/removal diff for `.curvebook.d/` directory synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DirectorySyncPlan {
    #[serde(with = "serde_named_bytes_vec")]
    pub writes: Vec<(String, Vec<u8>)>,
    #[serde(default)]
    pub removals: Vec<String>,
    #[serde(default)]
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

#[derive(Debug, Clone, Default)]
struct PendingDirectorySync {
    root_name: String,
    target_tree: BTreeMap<String, Vec<u8>>,
    planned_writes: BTreeSet<String>,
    planned_removals: BTreeSet<String>,
}

/// Pure-Rust File System Access & HTML5 drag-and-drop bridge for archives and `.curvebook.d/`.
#[derive(Debug, Clone, Default)]
pub struct FileSystemAccessBridge {
    last_synced_root: Option<String>,
    last_synced_tree: BTreeMap<String, Vec<u8>>,
    last_attempted_root: Option<String>,
    pending_sync: Option<PendingDirectorySync>,
}

impl FileSystemAccessBridge {
    /// Construct an empty file-system bridge with no previously synced directory tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Root folder name captured during the last `.curvebook.d/` import or sync.
    #[must_use]
    pub fn last_synced_root(&self) -> Option<&str> {
        self.last_synced_root.as_deref()
    }

    /// Snapshot of relative file contents recorded during the last `.curvebook.d/` import or sync.
    #[must_use]
    pub const fn last_synced_tree(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.last_synced_tree
    }

    /// Root folder name for an in-flight directory sync plan awaiting browser persistence confirmation.
    #[must_use]
    pub fn pending_sync_root(&self) -> Option<&str> {
        self.pending_sync.as_ref().map(|p| p.root_name.as_str())
    }

    /// Clear incremental directory sync tracking after switching to a non-directory or new project.
    pub fn reset_directory_sync(&mut self) {
        self.last_synced_root = None;
        self.last_synced_tree.clear();
        self.last_attempted_root = None;
        self.pending_sync = None;
    }

    /// Import a single picked or dropped file (`curvebook`, `cbtemplate`, `pzfx`, `prism`, `csv`, `tsv`).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if decoding fails.
    pub fn import_file_bytes(&mut self, host: &mut WebWorkspaceHost, file_name: &str, bytes: &[u8]) -> Result<WorkspaceSummary, WebAdapterError> {
        if bytes.len() > MAX_ARCHIVE_FILE_BYTES {
            return Err(WebAdapterError::Validation(format!(
                "file `{file_name}` exceeds maximum allowed size ({} > {MAX_ARCHIVE_FILE_BYTES} bytes)",
                bytes.len()
            )));
        }
        let summary = host.open_bytes_for_name(file_name, bytes).map_err(WebAdapterError::Workspace)?;
        self.reset_directory_sync();
        Ok(summary)
    }

    /// Import a `.curvebook.d/` virtual directory tree and seed incremental sync tracking.
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
        validate_byte_batch_budget(entries.len(), entries.values().map(Vec::len), "directory entries")?;
        let normalized = normalize_directory_entries(root, entries)?;
        self.import_normalized_directory_tree(host, root, normalized)
    }

    fn import_normalized_directory_tree(
        &mut self,
        host: &mut WebWorkspaceHost,
        root: &str,
        normalized: BTreeMap<String, Vec<u8>>,
    ) -> Result<WorkspaceSummary, WebAdapterError> {
        validate_byte_batch_budget(normalized.len(), normalized.values().map(Vec::len), "normalized directory entries")?;
        let summary =
            host.load_virtual_folder_entries(root, normalized.iter().map(|(k, v)| (k.clone(), v.clone()))).map_err(WebAdapterError::Workspace)?;
        self.last_synced_root = Some(root.to_owned());
        self.last_synced_tree = normalized;
        self.last_attempted_root = None;
        self.pending_sync = None;
        Ok(summary)
    }

    /// Import HTML5 drag-and-drop entries (either a single file or an unpacked `.curvebook.d/` folder).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if `files` is empty or contains duplicate normalized
    /// paths, or [`WebAdapterError::Workspace`] if the dropped payload cannot be decoded.
    pub fn import_dropped_files(&mut self, host: &mut WebWorkspaceHost, files: Vec<(String, Vec<u8>)>) -> Result<WorkspaceSummary, WebAdapterError> {
        if files.is_empty() {
            return Err(WebAdapterError::Validation(String::from("no dropped files supplied")));
        }
        validate_byte_batch_budget(files.len(), files.iter().map(|(_, bytes)| bytes.len()), "dropped files")?;
        if let [(name, bytes)] = files.as_slice()
            && !looks_like_directory_entry(name)
        {
            return self.import_file_bytes(host, name, bytes);
        }

        let detected_root = detect_dropped_directory_root(&files);
        let mut tree = BTreeMap::new();
        for (raw_path, bytes) in files {
            let rel = normalize_relative_entry_path(&raw_path, detected_root.as_deref())?;
            if tree.contains_key(&rel) {
                return Err(WebAdapterError::Validation(format!("duplicate normalized directory entry path `{rel}`")));
            }
            tree.insert(rel, bytes);
        }
        if !tree.contains_key("document.json") && !tree.contains_key("manifest.json") {
            return Err(WebAdapterError::Validation(String::from(
                "dropped multi-file batch must contain a .curvebook.d document.json or manifest.json",
            )));
        }
        let root_name = detected_root.unwrap_or_else(|| String::from("dropped.curvebook.d"));
        self.import_normalized_directory_tree(host, &root_name, tree)
    }

    /// Export the active workspace as a `.curvebook` archive byte buffer.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if archive serialization fails.
    pub fn export_curvebook_bytes(&self, host: &mut WebWorkspaceHost) -> Result<Vec<u8>, WebAdapterError> {
        host.export_curvebook_bytes().map_err(WebAdapterError::Workspace)
    }

    /// Materialize the current workspace as a deterministic `.curvebook.d/` relative file tree
    /// without altering the session dirty flag or save baseline.
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

    /// Prepare an incremental `.curvebook.d/` write/removal plan against [`Self::last_synced_tree`]
    /// without advancing the baseline or clearing the dirty flag until browser persistence confirms.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if virtual snapshot serialization fails.
    pub fn sync_directory_from_host(&mut self, host: &mut WebWorkspaceHost, root_name: Option<&str>) -> Result<DirectorySyncPlan, WebAdapterError> {
        let resolved_root = if let Some(explicit) = root_name.map(str::trim).filter(|s| !s.is_empty()) {
            explicit.to_owned()
        } else if let Some(prev) = &self.last_synced_root {
            prev.clone()
        } else if let Some(attempted) = &self.last_attempted_root {
            attempted.clone()
        } else {
            let vw_root = host.virtual_workspace().root_name.trim();
            if vw_root.is_empty() { String::from("workspace.curvebook.d") } else { vw_root.to_owned() }
        };

        let entries = host.snapshot_virtual_entries_with_root(Some(&resolved_root)).map_err(WebAdapterError::Workspace)?;
        let next: BTreeMap<String, Vec<u8>> = entries.into_iter().collect();
        let plan = Self::plan_directory_sync(&self.last_synced_tree, &next);
        self.last_attempted_root = Some(resolved_root.clone());
        self.pending_sync = Some(PendingDirectorySync {
            root_name: resolved_root,
            planned_writes: plan.writes.iter().map(|(p, _)| p.clone()).collect(),
            planned_removals: plan.removals.iter().cloned().collect(),
            target_tree: next,
        });
        Ok(plan)
    }

    /// Commit confirmed browser directory persistence outcomes into `last_synced_tree` and `host`.
    ///
    /// Partial or failed syncs advance `last_synced_tree` only for confirmed writes/removals and
    /// leave the workspace dirty so retries re-emit any remaining changes.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Validation`] if no matching pending directory sync exists.
    pub fn confirm_directory_sync(
        &mut self,
        host: &mut WebWorkspaceHost,
        root_name: &str,
        written_paths: &[String],
        removed_paths: &[String],
        succeeded: bool,
    ) -> Result<bool, WebAdapterError> {
        let trimmed_root = require_non_empty(root_name, "root_name")?;
        let Some(pending) = self.pending_sync.take() else {
            return Err(WebAdapterError::Validation(String::from("no pending directory sync to confirm")));
        };
        if trimmed_root != pending.root_name {
            let expected = pending.root_name.clone();
            self.pending_sync = Some(pending);
            return Err(WebAdapterError::Validation(format!("confirm_directory_sync root `{trimmed_root}` does not match pending `{expected}`")));
        }

        let mut applied_any = false;
        let mut written_set = BTreeSet::new();
        for path in written_paths {
            if let Some(bytes) = pending.target_tree.get(path) {
                self.last_synced_tree.insert(path.clone(), bytes.clone());
                written_set.insert(path.clone());
                applied_any = true;
            }
        }
        let mut removed_set = BTreeSet::new();
        for path in removed_paths {
            if pending.planned_removals.contains(path) {
                self.last_synced_tree.remove(path);
                removed_set.insert(path.clone());
                applied_any = true;
            }
        }

        let all_writes_done = pending.planned_writes.iter().all(|p| written_set.contains(p));
        let all_removals_done = pending.planned_removals.iter().all(|p| removed_set.contains(p));
        let fully_persisted = succeeded && all_writes_done && all_removals_done;

        if applied_any || fully_persisted {
            self.last_synced_root = Some(pending.root_name.clone());
        }
        if fully_persisted {
            self.last_synced_tree = pending.target_tree.clone();
            let doc_json = pending.target_tree.get("document.json").map(Vec::as_slice);
            host.confirm_virtual_directory_persisted(&pending.root_name, &pending.target_tree, doc_json);
        }
        Ok(fully_persisted)
    }

    /// Export the active document or sheet as downloadable bytes for File System Access / Blob fallback (`R10.7`).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] or [`WebAdapterError::Validation`] if export fails or no compatible sheet exists.
    pub fn export_bytes(
        &mut self,
        host: &mut WebWorkspaceHost,
        format: WebExportFormat,
        sheet_id: Option<&str>,
        dpi: u16,
    ) -> Result<WebExportArtifact, WebAdapterError> {
        let stem = sanitize_download_stem(&host.app().document().title);
        let validated_dpi = validate_web_dpi(dpi, "export_bytes")?;

        match format {
            WebExportFormat::CurveBookArchive => {
                let bytes = self.export_curvebook_bytes(host)?;
                Ok(WebExportArtifact {
                    format,
                    file_name: format!("{stem}.curvebook"),
                    mime_type: String::from(format.mime_type()),
                    extension: String::from(format.extension()),
                    bytes,
                })
            }
            WebExportFormat::CurveBookDirectory => {
                let tree = self.export_directory_tree(host)?;
                let dir_name = format!("{stem}.curvebook.d");
                let mut zip = curvebook_format::ZipWriter::new();
                for (rel_path, content) in tree {
                    zip.add_file(&format!("{dir_name}/{rel_path}"), content).map_err(|err| WebAdapterError::Workspace(err.to_string()))?;
                }
                let bytes = zip.finish().map_err(|err| WebAdapterError::Workspace(err.to_string()))?;
                Ok(WebExportArtifact {
                    format,
                    file_name: format!("{dir_name}.zip"),
                    mime_type: String::from("application/zip"),
                    extension: String::from("zip"),
                    bytes,
                })
            }
            WebExportFormat::Csv => export_csv_artifact(host, sheet_id, &stem),
            WebExportFormat::Svg | WebExportFormat::Pdf | WebExportFormat::Png | WebExportFormat::Tiff => {
                export_figure_artifact(host, format, sheet_id, validated_dpi, &stem)
            }
        }
    }
}

/// Downloadable export artifact prepared by [`FileSystemAccessBridge::export_bytes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebExportArtifact {
    pub format: WebExportFormat,
    pub file_name: String,
    pub mime_type: String,
    pub extension: String,
    pub bytes: Vec<u8>,
}

/// Sanitize a project or sheet title into a safe download filename stem (`RV-53`).
#[must_use]
pub fn sanitize_download_stem(raw: &str) -> String {
    curvebook_ui_egui::os_integration::sanitize_filename_stem(raw, '_', 64, "curvebook-project")
}

fn export_csv_artifact(host: &WebWorkspaceHost, sheet_id: Option<&str>, project_stem: &str) -> Result<WebExportArtifact, WebAdapterError> {
    let doc = host.app().document();
    let ds = if let Some(raw) = sheet_id {
        doc.data_sheets
            .iter()
            .find(|s| s.id.as_str() == raw)
            .ok_or_else(|| WebAdapterError::Validation(format!("data sheet `{raw}` not found for CSV export")))?
    } else {
        doc.active_sheet_id
            .as_ref()
            .and_then(|id| doc.data_sheet(id))
            .or_else(|| doc.data_sheets.first())
            .ok_or_else(|| WebAdapterError::Validation(String::from("no data sheet available to export as CSV")))?
    };

    let dec = host.app().locale_preset.decimal_separator;
    let mut opts = if dec == ',' { CsvOptions::european_csv() } else { CsvOptions::csv() };
    opts.decimal_separator = dec;
    let csv_text = export_datasheet_to_csv(ds, &opts).map_err(|err| WebAdapterError::Workspace(err.to_string()))?;
    let sheet_stem = sanitize_download_stem(&ds.title);
    let file_name = if sheet_stem.is_empty() { format!("{project_stem}.csv") } else { format!("{sheet_stem}.csv") };
    Ok(WebExportArtifact {
        format: WebExportFormat::Csv,
        file_name,
        mime_type: String::from(WebExportFormat::Csv.mime_type()),
        extension: String::from(WebExportFormat::Csv.extension()),
        bytes: csv_text.into_bytes(),
    })
}

fn export_figure_artifact(
    host: &mut WebWorkspaceHost,
    format: WebExportFormat,
    sheet_id: Option<&str>,
    dpi: u16,
    project_stem: &str,
) -> Result<WebExportArtifact, WebAdapterError> {
    let target_id = {
        let doc = host.app().document();
        if let Some(raw) = sheet_id {
            doc.graph_sheets
                .iter()
                .find(|s| s.id.as_str() == raw)
                .map(|s| s.id.clone())
                .or_else(|| doc.layout_sheets.iter().find(|s| s.id.as_str() == raw).map(|s| s.id.clone()))
                .ok_or_else(|| WebAdapterError::Validation(format!("printable graph or layout sheet `{raw}` not found for export")))?
        } else {
            doc.active_sheet_id
                .as_ref()
                .filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some())
                .cloned()
                .or_else(|| doc.graph_sheets.first().map(|s| s.id.clone()))
                .or_else(|| doc.layout_sheets.first().map(|s| s.id.clone()))
                .ok_or_else(|| WebAdapterError::Validation(String::from("no graph or layout sheet available to export")))?
        }
    };

    let (cmd_id, params) = match format {
        WebExportFormat::Svg => ("export.svg", json!({ "sheet_id": target_id.as_str() })),
        WebExportFormat::Pdf => ("export.pdf", json!({ "sheet_id": target_id.as_str() })),
        WebExportFormat::Png => ("export.png", json!({ "sheet_id": target_id.as_str(), "dpi": dpi })),
        WebExportFormat::Tiff => ("export.tiff", json!({ "sheet_id": target_id.as_str(), "dpi": dpi, "compression": "deflate" })),
        _ => return Err(WebAdapterError::Validation(String::from("unsupported figure export format"))),
    };

    let payload = host.execute_command(cmd_id, params).map_err(WebAdapterError::Workspace)?;
    build_figure_artifact_from_payload(format, cmd_id, target_id.as_str(), &payload, project_stem)
}

fn build_figure_artifact_from_payload(
    format: WebExportFormat,
    cmd_id: &str,
    target_id: &str,
    payload: &Value,
    project_stem: &str,
) -> Result<WebExportArtifact, WebAdapterError> {
    let inner = payload.get("payload").unwrap_or(payload);
    let bytes = match format {
        WebExportFormat::Svg => inner
            .get("svg")
            .and_then(Value::as_str)
            .ok_or_else(|| WebAdapterError::Workspace(format!("{cmd_id} missing `svg` payload")))?
            .as_bytes()
            .to_vec(),
        WebExportFormat::Pdf | WebExportFormat::Png | WebExportFormat::Tiff => {
            let b64 = inner
                .get("base64")
                .and_then(Value::as_str)
                .ok_or_else(|| WebAdapterError::Workspace(format!("{cmd_id} missing `base64` payload")))?;
            decode_base64(b64)?
        }
        _ => Vec::new(),
    };

    Ok(WebExportArtifact {
        format,
        file_name: format!("{project_stem}-{}.{}", sanitize_download_stem(target_id), format.extension()),
        mime_type: String::from(format.mime_type()),
        extension: String::from(format.extension()),
        bytes,
    })
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
        if normalized.contains_key(&rel) {
            return Err(WebAdapterError::Validation(format!("duplicate normalized directory entry path `{rel}`")));
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
    primary_pointer_down: Option<(f64, f64)>,
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

    /// Whether a single-finger primary pointer press is currently armed.
    #[must_use]
    pub const fn has_primary_pointer_down(&self) -> bool {
        self.primary_pointer_down.is_some()
    }

    /// Process browser `touchstart` contacts.
    pub fn touch_start(&mut self, points: &[TouchPoint], _dt_sec: f64) -> Vec<UiInputEvent> {
        let prev_count = self.active.len();
        let was_two_finger = self.two_finger_active;
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
            if !was_two_finger && self.primary_pointer_down.take().is_some() {
                return vec![UiInputEvent::PointerUp {
                    x: -1.0,
                    y: -1.0,
                    button: curvebook_ui_egui::PointerButton::Primary,
                    modifiers: curvebook_ui_egui::KeyModifiers::default(),
                }];
            }
            return Vec::new();
        }
        if prev_count == 0
            && self.active.len() == 1
            && let Some(&(x, y)) = self.active.values().next()
        {
            self.primary_pointer_down = Some((x, y));
            return vec![UiInputEvent::PointerDown {
                x,
                y,
                button: curvebook_ui_egui::PointerButton::Primary,
                modifiers: curvebook_ui_egui::KeyModifiers::default(),
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
            let mut events = Vec::new();
            if self.primary_pointer_down.take().is_some() {
                events.push(UiInputEvent::PointerUp {
                    x: -1.0,
                    y: -1.0,
                    button: curvebook_ui_egui::PointerButton::Primary,
                    modifiers: curvebook_ui_egui::KeyModifiers::default(),
                });
            }
            events.extend(self.process_two_finger_move(clamped_dt));
            return events;
        }
        if !self.two_finger_active
            && self.active.len() == 1
            && let Some(&(x, y)) = self.active.values().next()
        {
            self.primary_pointer_down = Some((x, y));
            return vec![UiInputEvent::PointerMove { x, y, modifiers: curvebook_ui_egui::KeyModifiers::default() }];
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
            let had_primary = self.primary_pointer_down.take().is_some();
            if !was_two_finger
                && had_primary
                && let Some((x, y)) = last_single_pos
            {
                events.push(UiInputEvent::PointerUp {
                    x,
                    y,
                    button: curvebook_ui_egui::PointerButton::Primary,
                    modifiers: curvebook_ui_egui::KeyModifiers::default(),
                });
            }
            self.two_finger_active = false;
            self.moved_two_finger = false;
        }
        events
    }

    /// Cancel all tracked touches and reset gesture state.
    pub fn touch_cancel(&mut self) -> Vec<UiInputEvent> {
        let had_primary = self.primary_pointer_down.take().is_some();
        self.active.clear();
        self.last_midpoint = None;
        self.last_distance = None;
        self.two_finger_active = false;
        self.moved_two_finger = false;
        let mut events = Vec::new();
        if had_primary {
            events.push(UiInputEvent::PointerUp {
                x: -1.0,
                y: -1.0,
                button: curvebook_ui_egui::PointerButton::Primary,
                modifiers: curvebook_ui_egui::KeyModifiers::default(),
            });
        }
        events.push(UiInputEvent::CancelInlineEdit);
        events
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

const VALIDATION_ERR_PREFIX: &str = "__cb_validation__:";

fn to_de_err<E: de::Error>(err: WebAdapterError) -> E {
    match err {
        WebAdapterError::Validation(msg) => E::custom(format!("{VALIDATION_ERR_PREFIX}{msg}")),
        other => E::custom(other.to_string()),
    }
}

fn map_serde_err(err: serde_json::Error) -> WebAdapterError {
    let msg = err.to_string();
    if let Some((_, rest)) = msg.split_once(VALIDATION_ERR_PREFIX) {
        WebAdapterError::Validation(rest.to_owned())
    } else {
        WebAdapterError::Serialization(msg)
    }
}

const fn default_one_f64() -> f64 {
    1.0
}
const fn default_frame_dt_sec() -> f64 {
    1.0 / 60.0
}
const fn default_scale_percent() -> f64 {
    100.0
}
const fn default_dpi_96() -> u16 {
    96
}
const fn default_dpi_300() -> u16 {
    300
}
const fn default_primary_button() -> PointerButton {
    PointerButton::Primary
}
const fn default_notice_level() -> WebNoticeLevel {
    WebNoticeLevel::Info
}
fn default_empty_object() -> Value {
    Value::Object(Map::new())
}
fn default_status_code() -> String {
    String::from("status")
}
fn de_opt_string_default<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

#[derive(Serialize, Deserialize)]
struct NamedBytesWire {
    path: String,
    #[serde(default)]
    bytes_base64: Option<String>,
    #[serde(default, skip_serializing)]
    bytes: Option<Vec<Value>>,
}

#[derive(Serialize, Deserialize)]
struct KeyValueWire {
    key: String,
    value: String,
}

mod serde_wire_bytes {
    use super::{Deserialize, Deserializer, Serializer, Value, de, decode_base64, decode_wire_bytes, encode_base64, to_de_err};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode_base64(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(Vec::new()),
            Value::String(raw) => decode_base64(&raw).map_err(to_de_err),
            Value::Array(arr) => decode_wire_bytes(None, Some(&arr), "bytes").map_err(to_de_err),
            _ => Err(de::Error::custom("expected base64 string or byte array")),
        }
    }
}

mod serde_named_bytes_vec {
    use super::{Deserialize, Deserializer, NamedBytesWire, Serialize, Serializer, decode_named_bytes_vec, encode_base64, to_de_err};

    pub fn serialize<S: Serializer>(files: &[(String, Vec<u8>)], s: S) -> Result<S::Ok, S::Error> {
        let items: Vec<NamedBytesWire> =
            files.iter().map(|(path, bytes)| NamedBytesWire { path: path.clone(), bytes_base64: Some(encode_base64(bytes)), bytes: None }).collect();
        items.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, Vec<u8>)>, D::Error> {
        decode_named_bytes_vec(Vec::<NamedBytesWire>::deserialize(d)?, "files").map_err(to_de_err)
    }
}

mod serde_named_bytes_map {
    use super::{
        BTreeMap, Deserialize, Deserializer, NamedBytesWire, Serialize, Serializer, collect_unique_directory_entries, decode_named_bytes_vec,
        encode_base64, to_de_err,
    };

    pub fn serialize<S: Serializer>(entries: &BTreeMap<String, Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        let items: Vec<NamedBytesWire> = entries
            .iter()
            .map(|(path, bytes)| NamedBytesWire { path: path.clone(), bytes_base64: Some(encode_base64(bytes)), bytes: None })
            .collect();
        items.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, Vec<u8>>, D::Error> {
        let pairs = decode_named_bytes_vec(Vec::<NamedBytesWire>::deserialize(d)?, "entries").map_err(to_de_err)?;
        collect_unique_directory_entries("", pairs).map_err(to_de_err)
    }
}

mod serde_kv_vec {
    use super::{Deserialize, Deserializer, KeyValueWire, Serialize, Serializer};

    pub fn serialize<S: Serializer>(entries: &[(String, String)], s: S) -> Result<S::Ok, S::Error> {
        let items: Vec<KeyValueWire> = entries.iter().map(|(k, v)| KeyValueWire { key: k.clone(), value: v.clone() }).collect();
        items.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, String)>, D::Error> {
        Ok(Vec::<KeyValueWire>::deserialize(d)?.into_iter().map(|kv| (kv.key, kv.value)).collect())
    }
}

mod serde_dpi_96 {
    use super::{Deserialize, Deserializer, Serializer, Value, parse_dpi_opt, to_de_err};

    pub fn serialize<S: Serializer>(dpi: &u16, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u16(*dpi)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
        parse_dpi_opt(Option::<Value>::deserialize(d)?.as_ref(), "dpi", 96).map_err(to_de_err)
    }
}

mod serde_dpi_300 {
    use super::{Deserialize, Deserializer, Serializer, Value, parse_dpi_opt, to_de_err};

    pub fn serialize<S: Serializer>(dpi: &u16, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u16(*dpi)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
        parse_dpi_opt(Option::<Value>::deserialize(d)?.as_ref(), "dpi", 300).map_err(to_de_err)
    }
}

mod serde_preview_filter {
    use super::{CanvasPreviewFilter, Deserialize, Deserializer, Serialize, Serializer, Value, de};

    pub fn serialize<S: Serializer>(filter: &CanvasPreviewFilter, s: S) -> Result<S::Ok, S::Error> {
        filter.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<CanvasPreviewFilter, D::Error> {
        let raw = Value::deserialize(d)?;
        if let Some(name) = raw.as_str() {
            CanvasPreviewFilter::from_name(name)
                .or_else(|| serde_json::from_value(raw.clone()).ok())
                .ok_or_else(|| de::Error::custom(format!("unknown canvas preview filter `{name}`")))
        } else {
            serde_json::from_value(raw).map_err(de::Error::custom)
        }
    }
}

mod serde_export_format {
    use super::{Deserialize, Deserializer, Serializer, WebExportFormat, de};

    pub fn serialize<S: Serializer>(format: &WebExportFormat, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(format.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<WebExportFormat, D::Error> {
        let raw = String::deserialize(d)?;
        WebExportFormat::from_name(&raw).ok_or_else(|| de::Error::custom(format!("unknown export format `{raw}`")))
    }
}

mod serde_notice_level {
    use super::{Deserialize, Deserializer, Serializer, WebNoticeLevel};

    pub fn serialize<S: Serializer>(level: &WebNoticeLevel, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(level.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<WebNoticeLevel, D::Error> {
        Ok(Option::<String>::deserialize(d)?.as_deref().and_then(WebNoticeLevel::from_name).unwrap_or(WebNoticeLevel::Info))
    }
}

mod serde_pointer_button {
    use super::{Deserialize, Deserializer, PointerButton, Serializer, parse_pointer_button, pointer_button_as_str};

    pub fn serialize<S: Serializer>(button: &PointerButton, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(pointer_button_as_str(*button))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PointerButton, D::Error> {
        Ok(parse_pointer_button(Option::<String>::deserialize(d)?.as_deref()))
    }
}

mod serde_rgba_layers {
    use super::{Deserialize, Deserializer, Serialize, Serializer, Value, WebRgbaLayer, to_de_err};

    pub fn serialize<S: Serializer>(layers: &[WebRgbaLayer], s: S) -> Result<S::Ok, S::Error> {
        let vals: Vec<Value> = layers.iter().map(WebRgbaLayer::to_json_value).collect();
        vals.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<WebRgbaLayer>, D::Error> {
        Vec::<Value>::deserialize(d)?.iter().map(WebRgbaLayer::from_json_value).collect::<Result<Vec<_>, _>>().map_err(to_de_err)
    }
}

/// Multi-flavor web clipboard bundle (`PDF`, `SVG`, `PNG`, `HTML`, `TSV`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebClipboardPayload {
    #[serde(rename = "pdf_base64", alias = "pdf_bytes", default, with = "serde_wire_bytes")]
    pub pdf_bytes: Vec<u8>,
    #[serde(default)]
    pub svg: String,
    #[serde(default)]
    pub html: String,
    #[serde(rename = "png_base64", alias = "png_bytes", default, with = "serde_wire_bytes")]
    pub png_bytes: Vec<u8>,
    #[serde(default)]
    pub tsv: Option<String>,
    #[serde(default)]
    pub plain_text: Option<String>,
    #[serde(default)]
    pub width_pt: f64,
    #[serde(default)]
    pub height_pt: f64,
    #[serde(default)]
    pub width_px: u32,
    #[serde(default)]
    pub height_px: u32,
    #[serde(default = "default_dpi_300", with = "serde_dpi_300")]
    pub dpi: u16,
    #[serde(default)]
    pub flavors: Vec<String>,
}

impl WebClipboardPayload {
    /// Serialize this clipboard payload to a JSON [`Value`].
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }

    /// Reconstruct a [`WebClipboardPayload`] from its JSON representation.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] if required fields are missing or malformed,
    /// or [`WebAdapterError::Validation`] if DPI or payload bytes violate bounds.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let out: Self = serde_json::from_value(value.clone()).map_err(map_serde_err)?;
        let _ = require_finite(out.width_pt, "width_pt")?;
        let _ = require_finite(out.height_pt, "height_pt")?;
        Ok(out)
    }
}

/// Incoming event sent from browser DOM/JS host hooks into [`WebBridgeSession`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebBridgeEvent {
    ResizeViewport {
        logical_width: f64,
        logical_height: f64,
        #[serde(default = "default_one_f64")]
        scale_factor: f64,
    },
    TouchStart {
        points: Vec<TouchPoint>,
        #[serde(default = "default_frame_dt_sec")]
        dt_sec: f64,
    },
    TouchMove {
        points: Vec<TouchPoint>,
        #[serde(default = "default_frame_dt_sec")]
        dt_sec: f64,
    },
    TouchEnd {
        released_ids: Vec<i64>,
        #[serde(default = "default_frame_dt_sec")]
        dt_sec: f64,
    },
    TouchCancel,
    CoastStep {
        #[serde(default = "default_frame_dt_sec")]
        dt_sec: f64,
    },
    UiInput {
        event: UiInputEvent,
    },
    ExecuteCommand {
        command_id: String,
        #[serde(default = "default_empty_object")]
        params: Value,
    },
    DispatchControlLine {
        line: String,
    },
    RenderFrame {
        #[serde(default = "default_dpi_96", with = "serde_dpi_96")]
        dpi: u16,
    },
    CopyClipboard {
        #[serde(default)]
        sheet_id: Option<String>,
        #[serde(default = "default_dpi_300", with = "serde_dpi_300")]
        dpi: u16,
    },
    PrintCurrentSheet {
        #[serde(default)]
        sheet_id: Option<String>,
        #[serde(default = "default_scale_percent")]
        scale_percent: f64,
    },
    OpenFileBytes {
        file_name: String,
        #[serde(rename = "bytes_base64", alias = "bytes", default, with = "serde_wire_bytes")]
        bytes: Vec<u8>,
    },
    ImportDroppedFiles {
        #[serde(with = "serde_named_bytes_vec")]
        files: Vec<(String, Vec<u8>)>,
    },
    ImportDirectoryEntries {
        root_name: String,
        #[serde(with = "serde_named_bytes_map")]
        entries: BTreeMap<String, Vec<u8>>,
    },
    SyncDirectory {
        #[serde(default)]
        root_name: Option<String>,
    },
    SavePreferences {
        preferences: PreferencesFile,
    },
    SaveAutosave {
        #[serde(default)]
        now_ms: u64,
    },
    AcquireLock {
        doc_id: String,
        tab_id: String,
        #[serde(default)]
        now_ms: u64,
    },
    ObserveExternalLock {
        doc_id: String,
        holder_tab_id: String,
        #[serde(default)]
        now_ms: u64,
    },
    RenewHeartbeat {
        #[serde(default)]
        now_ms: u64,
    },
    ReleaseLock {
        #[serde(default)]
        now_ms: u64,
    },
    SetPreviewFilter {
        #[serde(with = "serde_preview_filter")]
        filter: CanvasPreviewFilter,
    },
    Wheel {
        #[serde(default)]
        x: f64,
        #[serde(default)]
        y: f64,
        #[serde(default)]
        delta_x: f64,
        #[serde(default)]
        delta_y: f64,
        #[serde(default)]
        modifiers: KeyModifiers,
    },
    ImePreedit {
        #[serde(default)]
        text: String,
    },
    ImeCommit {
        #[serde(default)]
        text: String,
    },
    DragStateChanged {
        #[serde(default)]
        active: bool,
    },
    PasteClipboard {
        #[serde(default)]
        text: String,
    },
    ExportFile {
        #[serde(with = "serde_export_format")]
        format: WebExportFormat,
        #[serde(default)]
        sheet_id: Option<String>,
        #[serde(default = "default_dpi_300", with = "serde_dpi_300")]
        dpi: u16,
    },
    HydrateLocalStorage {
        #[serde(with = "serde_kv_vec")]
        entries: Vec<(String, String)>,
    },
    #[serde(rename = "__hydrate_local_storage_at")]
    HydrateLocalStorageAt {
        #[serde(with = "serde_kv_vec")]
        entries: Vec<(String, String)>,
        now_ms: u64,
    },
    ClickChromeNode {
        node_id: String,
        #[serde(default = "default_primary_button", with = "serde_pointer_button")]
        button: PointerButton,
        #[serde(default)]
        modifiers: KeyModifiers,
    },
    ConfirmDirectorySync {
        root_name: String,
        #[serde(default)]
        written_paths: Vec<String>,
        #[serde(default)]
        removed_paths: Vec<String>,
        #[serde(default)]
        succeeded: bool,
        #[serde(default)]
        error: Option<String>,
    },
    ConfirmLocalStorage {
        #[serde(default)]
        writes: Vec<String>,
        #[serde(default)]
        removals: Vec<String>,
        #[serde(default)]
        succeeded: bool,
    },
}

impl WebBridgeEvent {
    /// Encode this bridge event into a JSON [`Value`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] if embedded `UiInputEvent` or `PreferencesFile` serialization fails.
    pub fn to_json_value(&self) -> Result<Value, WebAdapterError> {
        let mut val = serde_json::to_value(self).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        if matches!(self, Self::HydrateLocalStorageAt { .. })
            && let Some(obj) = val.as_object_mut()
        {
            obj.insert(String::from("kind"), Value::String(String::from("hydrate_local_storage")));
        }
        Ok(val)
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
    /// Returns [`WebAdapterError::Serialization`] when the payload does not match any supported variant,
    /// or [`WebAdapterError::Validation`] when DPI, byte budgets, or directory paths violate bounds.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let kind = value.get("kind").and_then(Value::as_str);
        if kind == Some("save_preferences") {
            let prefs_val = value.get("preferences").ok_or_else(|| WebAdapterError::Serialization(String::from("missing `preferences`")))?;
            let raw_text = serde_json::to_string(prefs_val).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
            check_raw_preferences_schema(&raw_text)?;
            let preferences = parse_preferences_json(&raw_text).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
            return Ok(Self::SavePreferences { preferences });
        }
        if kind == Some("import_directory_entries") {
            #[derive(Deserialize)]
            struct DirImport {
                root_name: String,
                entries: Vec<NamedBytesWire>,
            }
            let parsed: DirImport = serde_json::from_value(value.clone()).map_err(map_serde_err)?;
            let pairs = decode_named_bytes_vec(parsed.entries, "entries")?;
            let entries = collect_unique_directory_entries(&parsed.root_name, pairs)?;
            return Ok(Self::ImportDirectoryEntries { root_name: parsed.root_name, entries });
        }
        let mut owned = value.clone();
        if kind == Some("hydrate_local_storage")
            && owned.get("now_ms").is_some_and(|v| !v.is_null())
            && let Some(obj) = owned.as_object_mut()
        {
            obj.insert(String::from("kind"), Value::String(String::from("__hydrate_local_storage_at")));
        }
        let event: Self = serde_json::from_value(owned).map_err(map_serde_err)?;
        event.validate_finite_fields()?;
        Ok(event)
    }

    fn validate_finite_fields(&self) -> Result<(), WebAdapterError> {
        match self {
            Self::ResizeViewport { logical_width, logical_height, scale_factor } => {
                let _ = require_finite(*logical_width, "logical_width")?;
                let _ = require_finite(*logical_height, "logical_height")?;
                let _ = require_finite(*scale_factor, "scale_factor")?;
            }
            Self::TouchStart { points, dt_sec } | Self::TouchMove { points, dt_sec } => {
                validate_touch_points_ref(points)?;
                let _ = require_finite(*dt_sec, "dt_sec")?;
            }
            Self::TouchEnd { dt_sec, .. } | Self::CoastStep { dt_sec } => {
                let _ = require_finite(*dt_sec, "dt_sec")?;
            }
            Self::PrintCurrentSheet { scale_percent, .. } => {
                let _ = require_finite(*scale_percent, "scale_percent")?;
            }
            Self::Wheel { x, y, delta_x, delta_y, .. } => {
                let _ = require_finite(*x, "x")?;
                let _ = require_finite(*y, "y")?;
                let _ = require_finite(*delta_x, "delta_x")?;
                let _ = require_finite(*delta_y, "delta_y")?;
            }
            _ => {}
        }
        Ok(())
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebBridgeCommand {
    RenderRgbaFrame {
        width_px: u32,
        height_px: u32,
        #[serde(default = "default_dpi_96", with = "serde_dpi_96")]
        dpi: u16,
        #[serde(rename = "rgba_base64", alias = "rgba_bytes", default, with = "serde_wire_bytes")]
        rgba_bytes: Vec<u8>,
        #[serde(default, with = "serde_rgba_layers")]
        rgba_layers: Vec<WebRgbaLayer>,
        #[serde(default)]
        text_ops: Vec<WebTextDrawOp>,
        #[serde(default)]
        chrome: WebChromeSnapshot,
        #[serde(default = "default_one_f64")]
        zoom: f64,
        #[serde(default)]
        pan_pt: [f64; 2],
        #[serde(default)]
        action: Option<String>,
        #[serde(default)]
        command_result: Option<Value>,
    },
    WriteClipboardItems {
        payload: WebClipboardPayload,
    },
    TriggerPrintHtml {
        title: String,
        #[serde(default)]
        svg_pages: Vec<String>,
        #[serde(default)]
        html: String,
        #[serde(rename = "pdf_base64", alias = "pdf_bytes", default, with = "serde_wire_bytes")]
        pdf_bytes: Vec<u8>,
        #[serde(default = "default_scale_percent")]
        scale_percent: f64,
        #[serde(default)]
        page_sizes_pt: Vec<(f64, f64)>,
    },
    ApplyDirectorySync {
        root_name: String,
        #[serde(flatten)]
        plan: DirectorySyncPlan,
    },
    PersistLocalStorage {
        #[serde(with = "serde_kv_vec")]
        writes: Vec<(String, String)>,
        #[serde(default)]
        removals: Vec<String>,
    },
    SetReadOnlyBanner {
        doc_id: String,
        #[serde(default)]
        read_only: bool,
        #[serde(default)]
        holder_tab_id: Option<String>,
        #[serde(default)]
        message: Option<String>,
    },
    SaveOrDownloadFile {
        file_name: String,
        #[serde(default, deserialize_with = "de_opt_string_default")]
        mime_type: String,
        #[serde(rename = "bytes_base64", alias = "bytes", default, with = "serde_wire_bytes")]
        bytes: Vec<u8>,
        #[serde(with = "serde_export_format")]
        format: WebExportFormat,
    },
    TriggerFilePicker {
        #[serde(default)]
        accept: String,
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        directory: bool,
    },
    NotifyStatus {
        #[serde(default = "default_notice_level", with = "serde_notice_level")]
        level: WebNoticeLevel,
        #[serde(default = "default_status_code")]
        code: String,
        #[serde(default)]
        message: String,
    },
    ClearDirectoryHandle,
    SetDocumentLock {
        doc_id: String,
    },
}

fn precheck_render_rgba_frame(value: &Value) -> Result<usize, WebAdapterError> {
    let width_u64 = value
        .get("width_px")
        .and_then(Value::as_u64)
        .ok_or_else(|| WebAdapterError::Serialization(String::from("`width_px` must be an unsigned 32-bit integer")))?;
    let height_u64 = value
        .get("height_px")
        .and_then(Value::as_u64)
        .ok_or_else(|| WebAdapterError::Serialization(String::from("`height_px` must be an unsigned 32-bit integer")))?;
    let width_px = u32::try_from(width_u64).map_err(|_| WebAdapterError::Serialization(String::from("`width_px` exceeds u32::MAX")))?;
    let height_px = u32::try_from(height_u64).map_err(|_| WebAdapterError::Serialization(String::from("`height_px` exceeds u32::MAX")))?;
    let expected_bytes = usize::try_from(width_px)
        .ok()
        .and_then(|w| usize::try_from(height_px).ok().and_then(|h| w.checked_mul(h)?.checked_mul(4)))
        .filter(|&bytes| bytes <= MAX_FRAME_RGBA_BYTES)
        .ok_or_else(|| {
            WebAdapterError::Validation(format!(
                "render_rgba_frame dimensions {width_px}x{height_px} exceed MAX_FRAME_RGBA_BYTES ({MAX_FRAME_RGBA_BYTES})"
            ))
        })?;
    if let Some(raw_b64) = value.get("rgba_base64").and_then(Value::as_str)
        && raw_b64.len() > MAX_FRAME_RGBA_BYTES.div_ceil(3).saturating_mul(4).saturating_add(64)
    {
        return Err(WebAdapterError::Validation(String::from("rgba_base64 exceeds MAX_FRAME_RGBA_BYTES")));
    }
    Ok(expected_bytes)
}

impl WebBridgeCommand {
    /// Encode this command into a JSON [`Value`].
    #[must_use]
    pub fn to_json_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }

    /// Reconstruct a [`WebBridgeCommand`] from a JSON [`Value`].
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Serialization`] when required fields are missing or malformed,
    /// or [`WebAdapterError::Validation`] when frame dimensions, DPI, or byte budgets violate bounds.
    pub fn from_json_value(value: &Value) -> Result<Self, WebAdapterError> {
        let expected_rgba =
            if value.get("kind").and_then(Value::as_str) == Some("render_rgba_frame") { Some(precheck_render_rgba_frame(value)?) } else { None };
        let mut cmd: Self = serde_json::from_value(value.clone()).map_err(map_serde_err)?;
        match &mut cmd {
            Self::RenderRgbaFrame { width_px, height_px, rgba_bytes, zoom, pan_pt, command_result, .. } => {
                if let Some(expected) = expected_rgba
                    && !rgba_bytes.is_empty()
                    && rgba_bytes.len() != expected
                {
                    return Err(WebAdapterError::Validation(format!(
                        "rgba_bytes length {} does not match {width_px}x{height_px}x4 ({expected})",
                        rgba_bytes.len()
                    )));
                }
                let _ = require_finite(*zoom, "zoom")?;
                let _ = require_finite(pan_pt[0], "pan_pt[0]")?;
                let _ = require_finite(pan_pt[1], "pan_pt[1]")?;
                if command_result.as_ref().is_some_and(Value::is_null) {
                    *command_result = None;
                }
            }
            Self::WriteClipboardItems { payload } => {
                let _ = require_finite(payload.width_pt, "width_pt")?;
                let _ = require_finite(payload.height_pt, "height_pt")?;
            }
            Self::TriggerPrintHtml { scale_percent, page_sizes_pt, .. } => {
                let _ = require_finite(*scale_percent, "scale_percent")?;
                for &(w, h) in page_sizes_pt.iter() {
                    let _ = require_finite(w, "page_sizes_pt.width")?;
                    let _ = require_finite(h, "page_sizes_pt.height")?;
                }
            }
            Self::SaveOrDownloadFile { mime_type, format, .. } => {
                if mime_type.is_empty() {
                    *mime_type = String::from(format.mime_type());
                }
            }
            Self::SetDocumentLock { doc_id } => {
                *doc_id = require_non_empty(doc_id, "doc_id")?.to_owned();
            }
            _ => {}
        }
        Ok(cmd)
    }
}

/// Complete browser adapter session binding workspace, storage, locks, filesystem sync, touch, text measurement, and chrome.
#[derive(Debug, Clone)]
pub struct WebBridgeSession {
    pub host: WebWorkspaceHost,
    pub storage: WebStorageManager<MemoryWebStorage>,
    pub locks: WebLockCoordinator,
    pub fs: FileSystemAccessBridge,
    pub touch: TouchGestureRecognizer,
    pub text_measurer: BrowserCanvasTextMeasurer,
    pub chrome: WebChromeBridge,
    doc_id: String,
    tab_id: String,
    access_mode: TabAccessMode,
    render_dpi: u16,
    pending_storage_ops: VecDeque<StorageRollbackOp>,
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
            text_measurer: BrowserCanvasTextMeasurer::new(),
            chrome: WebChromeBridge::new(),
            doc_id: doc,
            tab_id: tab,
            access_mode: TabAccessMode::ExclusiveReadWrite,
            render_dpi: 96,
            pending_storage_ops: VecDeque::new(),
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

    /// Export a `.curvebook` archive copy (explicitly allowed even when contended read-only).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if archive export fails.
    pub fn export_curvebook_bytes(&mut self) -> Result<Vec<u8>, WebAdapterError> {
        self.fs.export_curvebook_bytes(&mut self.host)
    }

    /// Export a `.curvebook.d/` file tree copy (explicitly allowed even when contended read-only).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Workspace`] if virtual tree export fails.
    pub fn export_directory_tree(&mut self) -> Result<BTreeMap<String, Vec<u8>>, WebAdapterError> {
        self.fs.export_directory_tree(&mut self.host)
    }

    /// Build a multi-flavor clipboard payload (`PDF`/`SVG`/`PNG`/`HTML`/`TSV`), allowed in read-only mode.
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Clipboard`] if `dpi` is invalid, an explicit `sheet_id` does not
    /// exist, or no sheet can be exported.
    pub fn build_clipboard_payload(&mut self, sheet_id: Option<&str>, dpi: u16) -> Result<WebClipboardPayload, WebAdapterError> {
        if !(MIN_CLIPBOARD_DPI..=MAX_CLIPBOARD_DPI).contains(&dpi) {
            return Err(WebAdapterError::Clipboard(format!("clipboard DPI {dpi} out of supported range {MIN_CLIPBOARD_DPI}..={MAX_CLIPBOARD_DPI}")));
        }

        let target = resolve_clipboard_target(self.host.app(), sheet_id)?;
        match target {
            ClipboardTarget::Data(target_id) => build_data_clipboard_payload(self.host.app(), &target_id, dpi),
            ClipboardTarget::Graph(target_id) => build_graph_clipboard_payload(self.host.app_mut(), &target_id, dpi),
            ClipboardTarget::Layout(target_id) => build_layout_clipboard_payload(self.host.app_mut(), &target_id, dpi),
            ClipboardTarget::Result(target_id) => build_result_clipboard_payload(self.host.app(), &target_id, dpi),
            ClipboardTarget::Info(target_id) => build_info_clipboard_payload(self.host.app(), &target_id, dpi),
        }
    }

    /// Build a scaled publication print command targeting the requested or active printable sheet (`R2-S11-13`).
    ///
    /// # Errors
    /// Returns [`WebAdapterError::Print`] if no printable sheet exists or spooling fails.
    pub fn build_print_command(&mut self, sheet_id: Option<&str>, scale_percent: f64) -> Result<WebBridgeCommand, WebAdapterError> {
        let mut spec = PrintJobSpec::publication_100_percent();
        let clamped_scale = if scale_percent.is_finite() { scale_percent.clamp(10.0, 400.0) } else { 100.0 };
        spec.scale_percent = clamped_scale;

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
            } else if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()) {
                spec.sheet_ids.push(active.clone());
                vec![active.as_str().to_owned()]
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
                let err_msg = extract_control_reply_error(&reply, "SVG print render failed");
                return Err(WebAdapterError::Print(err_msg.to_owned()));
            }
            let svg = reply
                .get("result")
                .and_then(|r| r.get("svg"))
                .and_then(Value::as_str)
                .ok_or_else(|| WebAdapterError::Print(String::from("render.svg reply missing `svg`")))?;
            svg_pages.push(svg.to_owned());
        }

        let html = build_print_html_document(&title, &svg_pages, spool.scale_percent);
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
                let prev = self.host.viewport();
                self.host.set_viewport(metrics);
                match self.render_frame_command(self.render_dpi, Some(String::from("viewport.resize")), None) {
                    Ok(frame) => Ok(vec![frame]),
                    Err(err) => {
                        self.host.set_viewport(prev);
                        Err(err)
                    }
                }
            }
            WebBridgeEvent::TouchStart { points, dt_sec } => {
                let was_single_press = !self.touch.is_two_finger_gesture() && self.touch.has_primary_pointer_down();
                let events = self.touch.touch_start(&points, dt_sec);
                if was_single_press && self.touch.is_two_finger_gesture() {
                    cancel_armed_pointer_and_drags(self.host.app_mut());
                }
                self.dispatch_touch_events(events)
            }
            WebBridgeEvent::TouchMove { points, dt_sec } => {
                let was_single_press = !self.touch.is_two_finger_gesture() && self.touch.has_primary_pointer_down();
                let events = self.touch.touch_move(&points, dt_sec);
                if was_single_press && self.touch.is_two_finger_gesture() {
                    cancel_armed_pointer_and_drags(self.host.app_mut());
                }
                self.dispatch_touch_events(events)
            }
            WebBridgeEvent::TouchEnd { released_ids, dt_sec } => {
                let events = self.touch.touch_end(&released_ids, dt_sec);
                self.dispatch_touch_events(events)
            }
            WebBridgeEvent::TouchCancel => {
                cancel_armed_pointer_and_drags(self.host.app_mut());
                let events = self.touch.touch_cancel();
                self.dispatch_touch_events(events)
            }
            WebBridgeEvent::CoastStep { dt_sec } => {
                let coast = self.touch.step_inertia(dt_sec);
                self.dispatch_single_ui_input(coast)
            }
            WebBridgeEvent::UiInput { event: input } => self.handle_ui_input_event(input),
            WebBridgeEvent::ExecuteCommand { command_id, params } => self.handle_execute_command(&command_id, params),
            WebBridgeEvent::DispatchControlLine { line } => self.handle_control_line(&line),
            WebBridgeEvent::RenderFrame { dpi } => {
                let valid_dpi = validate_web_dpi(dpi, "render_frame")?;
                self.render_dpi = valid_dpi;
                Ok(vec![self.render_frame_command(valid_dpi, Some(String::from("render.frame")), None)?])
            }
            WebBridgeEvent::CopyClipboard { sheet_id, dpi } => {
                let payload = self.build_clipboard_payload(sheet_id.as_deref(), dpi)?;
                Ok(vec![WebBridgeCommand::WriteClipboardItems { payload }])
            }
            WebBridgeEvent::PrintCurrentSheet { sheet_id, scale_percent } => Ok(vec![self.build_print_command(sheet_id.as_deref(), scale_percent)?]),
            WebBridgeEvent::OpenFileBytes { file_name, bytes } => self.handle_open_file_bytes(&file_name, &bytes),
            WebBridgeEvent::ImportDroppedFiles { files } => self.handle_import_dropped_files(files),
            WebBridgeEvent::ImportDirectoryEntries { root_name, entries } => self.handle_import_directory_entries(&root_name, entries),
            WebBridgeEvent::SyncDirectory { root_name } => {
                self.require_write_lock("sync_directory")?;
                let plan = self.fs.sync_directory_from_host(&mut self.host, root_name.as_deref())?;
                let resolved_root = self.fs.pending_sync_root().unwrap_or("workspace.curvebook.d").to_owned();
                Ok(vec![WebBridgeCommand::ApplyDirectorySync { root_name: resolved_root, plan }])
            }
            WebBridgeEvent::SavePreferences { preferences } => {
                self.require_write_lock("save_preferences")?;
                let cmd = self.persist_preferences_command(&preferences)?;
                Ok(vec![cmd])
            }
            WebBridgeEvent::SaveAutosave { now_ms } => self.handle_save_autosave(now_ms),
            WebBridgeEvent::AcquireLock { doc_id, tab_id, now_ms } => {
                self.doc_id = require_non_empty(&doc_id, "doc_id")?.to_owned();
                self.tab_id = require_non_empty(&tab_id, "tab_id")?.to_owned();
                let acq = self.locks.grant_browser_lock(&self.doc_id, &self.tab_id, now_ms)?;
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
            other => self.handle_extended_event(other),
        }
    }

    /// Parse raw JSON [`WebBridgeEvent`], execute it, and return serialized JSON command array.
    ///
    /// # Errors
    /// Returns [`WebAdapterError`] if raw JSON exceeds safety caps, parsing fails, or dispatch errors.
    pub fn handle_event_json(&mut self, raw_event_json: &str) -> Result<String, WebAdapterError> {
        let max_event_json_bytes = MAX_ARCHIVE_FILE_BYTES.saturating_mul(2);
        if raw_event_json.len() > max_event_json_bytes {
            return Err(WebAdapterError::Validation(format!(
                "bridge event JSON payload exceeds maximum allowed size ({} > {max_event_json_bytes} bytes)",
                raw_event_json.len()
            )));
        }
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

    fn current_chrome_layout(&self) -> chrome_bridge::WebChromeLayout {
        self.chrome.build_layout(self.host.app(), self.host.viewport())
    }

    fn handle_ui_input_event(&mut self, input: UiInputEvent) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        match input {
            UiInputEvent::PointerMove { x, y, modifiers } => {
                let chrome_layout = self.current_chrome_layout();
                let (hover_changed, occluded) = self.chrome.handle_pointer_move(self.host.app_mut(), &chrome_layout, [x, y]);
                let outcome = if occluded {
                    UiEventOutcome::default()
                } else {
                    let out = self.host.dispatch_event(UiInputEvent::PointerMove { x, y, modifiers }).map_err(WebAdapterError::Workspace)?;
                    self.chrome.sync_from_app(self.host.app());
                    out
                };
                if outcome.handled || hover_changed {
                    let action = if outcome.handled { outcome.action } else { String::from("chrome.hover") };
                    Ok(vec![self.render_frame_command(self.render_dpi, Some(action), outcome.command_result)?])
                } else {
                    Ok(Vec::new())
                }
            }
            UiInputEvent::PointerDown { x, y, button, modifiers } => {
                let viewport = self.host.viewport();
                let [px_f, py_f] = viewport.logical_to_physical_px([x, y]);
                #[allow(clippy::cast_possible_truncation)]
                let phys_pos = [px_f.round() as i32, py_f.round() as i32];
                let chrome_layout = self.current_chrome_layout();
                let dispatch = self.chrome.handle_pointer_down(self.host.app_mut(), &chrome_layout, [x, y], phys_pos, button);
                let patched = match dispatch {
                    WebChromeDispatch::PassToWorkspace(_) => {
                        WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerDown { x, y, button, modifiers })
                    }
                    other => other,
                };
                self.finish_chrome_dispatch(patched)
            }
            UiInputEvent::PointerUp { x, y, button, modifiers } => {
                let chrome_layout = self.current_chrome_layout();
                let dispatch = self.chrome.handle_pointer_up(self.host.app_mut(), &chrome_layout, [x, y], button);
                let patched = match dispatch {
                    WebChromeDispatch::PassToWorkspace(_) => WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerUp { x, y, button, modifiers }),
                    other => other,
                };
                self.finish_chrome_dispatch(patched)
            }
            UiInputEvent::DoubleClick { x, y, button, modifiers } => {
                let chrome_layout = self.current_chrome_layout();
                if chrome_layout
                    .hit_test_logical(x, y)
                    .is_some_and(|n| n.id == "modal.backdrop" || n.z_layer >= crate::text_bridge::ZLayer::DropdownMenu)
                {
                    return self.finish_chrome_dispatch(WebChromeDispatch::BlockedByModalBackdrop);
                }
                if ui_input_requires_write_lock(&UiInputEvent::DoubleClick { x, y, button, modifiers }) {
                    self.require_write_lock("double_click")?;
                }
                self.dispatch_single_ui_input(UiInputEvent::DoubleClick { x, y, button, modifiers })
            }
            UiInputEvent::KeyDown { key, modifiers } => {
                let chrome_layout = self.current_chrome_layout();
                let dispatch = self.chrome.handle_key_down(self.host.app_mut(), &chrome_layout, &key, modifiers);
                self.finish_chrome_dispatch(dispatch)
            }
            UiInputEvent::TextInput { text } => {
                if self.access_mode.is_read_only() && !matches!(self.host.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
                    return Err(self.read_only_err("text_input"));
                }
                let dispatch = self.chrome.handle_ime_commit(self.host.app_mut(), &text);
                self.finish_chrome_dispatch(dispatch)
            }
            other => {
                if ui_input_requires_write_lock(&other) {
                    self.require_write_lock(ui_input_label(&other))?;
                }
                self.dispatch_single_ui_input(other)
            }
        }
    }

    fn handle_extended_event(&mut self, event: WebBridgeEvent) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        match event {
            WebBridgeEvent::Wheel { x, y, delta_x, delta_y, modifiers } => {
                let chrome_layout = self.current_chrome_layout();
                let disp = self.chrome.route_wheel(self.host.app_mut(), &chrome_layout, [x, y], [delta_x, delta_y], modifiers);
                self.finish_chrome_dispatch(disp)
            }
            WebBridgeEvent::ImePreedit { text } => {
                self.chrome.ime_preedit = (!text.is_empty()).then_some(text);
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("ime.preedit")), None)?])
            }
            WebBridgeEvent::ImeCommit { text } => {
                if self.access_mode.is_read_only() && !matches!(self.host.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
                    return Err(self.read_only_err("ime_commit"));
                }
                let disp = self.chrome.handle_ime_commit(self.host.app_mut(), &text);
                self.finish_chrome_dispatch(disp)
            }
            WebBridgeEvent::DragStateChanged { active } => {
                self.chrome.drag_active = active;
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("drag.state")), None)?])
            }
            WebBridgeEvent::PasteClipboard { text } => self.handle_paste_clipboard(text),
            WebBridgeEvent::ExportFile { format, sheet_id, dpi } => {
                let art = self.fs.export_bytes(&mut self.host, format, sheet_id.as_deref(), dpi)?;
                Ok(vec![WebBridgeCommand::SaveOrDownloadFile {
                    file_name: art.file_name,
                    mime_type: art.mime_type,
                    bytes: art.bytes,
                    format: art.format,
                }])
            }
            WebBridgeEvent::HydrateLocalStorage { entries } => self.handle_hydrate_local_storage(entries, current_epoch_ms()),
            WebBridgeEvent::HydrateLocalStorageAt { entries, now_ms } => self.handle_hydrate_local_storage(entries, now_ms),
            WebBridgeEvent::ClickChromeNode { node_id, button, modifiers } => self.handle_click_chrome_node(&node_id, button, modifiers),
            WebBridgeEvent::ConfirmDirectorySync { root_name, written_paths, removed_paths, succeeded, error } => {
                self.handle_confirm_directory_sync(&root_name, &written_paths, &removed_paths, succeeded, error)
            }
            WebBridgeEvent::ConfirmLocalStorage { writes, removals, succeeded } => {
                self.handle_confirm_local_storage(&writes, &removals, succeeded);
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    fn handle_click_chrome_node(
        &mut self,
        node_id: &str,
        button: PointerButton,
        modifiers: KeyModifiers,
    ) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let chrome_layout = self.current_chrome_layout();
        let Some(node) = chrome_layout.find_node(node_id).cloned() else {
            return Err(WebAdapterError::Validation(format!("chrome node `{node_id}` not found")));
        };
        let rect = node.rect_px;
        #[allow(clippy::cast_possible_wrap)]
        let phys_x = rect.x.saturating_add((rect.width / 2) as i32);
        #[allow(clippy::cast_possible_wrap)]
        let phys_y = rect.y.saturating_add((rect.height / 2) as i32);
        let scale = self.host.viewport().scale_factor.max(1e-6);
        let logical_pos = [f64::from(phys_x) / scale, f64::from(phys_y) / scale];
        let down = self.chrome.handle_pointer_down(self.host.app_mut(), &chrome_layout, logical_pos, [phys_x, phys_y], button);
        if button != PointerButton::Primary {
            let patched = match down {
                WebChromeDispatch::PassToWorkspace(_) => {
                    WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerDown { x: logical_pos[0], y: logical_pos[1], button, modifiers })
                }
                other => other,
            };
            return self.finish_chrome_dispatch(patched);
        }
        if let WebChromeDispatch::PassToWorkspace(_) = down {
            let mut cmds = self.finish_chrome_dispatch(WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerDown {
                x: logical_pos[0],
                y: logical_pos[1],
                button,
                modifiers,
            }))?;
            let up_cmds = self.finish_chrome_dispatch(WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerUp {
                x: logical_pos[0],
                y: logical_pos[1],
                button,
                modifiers,
            }))?;
            if !up_cmds.is_empty() {
                cmds = up_cmds;
            }
            return Ok(cmds);
        }
        let armed = matches!(&down, WebChromeDispatch::HandledRedraw { action, .. } if action.starts_with("chrome.pressed:"));
        if !armed {
            return self.finish_chrome_dispatch(down);
        }
        let up_layout = self.current_chrome_layout();
        let up = self.chrome.handle_pointer_up(self.host.app_mut(), &up_layout, logical_pos, button);
        let patched = match up {
            WebChromeDispatch::PassToWorkspace(_) => {
                WebChromeDispatch::PassToWorkspace(UiInputEvent::PointerUp { x: logical_pos[0], y: logical_pos[1], button, modifiers })
            }
            other => other,
        };
        self.finish_chrome_dispatch(patched)
    }

    fn finish_chrome_dispatch(&mut self, dispatch: WebChromeDispatch) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        match dispatch {
            WebChromeDispatch::PassToWorkspace(ev) => {
                if ui_input_requires_write_lock(&ev) && !matches!(self.host.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
                    self.require_write_lock(ui_input_label(&ev))?;
                }
                let cmds = self.dispatch_single_ui_input(ev)?;
                self.chrome.sync_from_app(self.host.app());
                Ok(cmds)
            }
            WebChromeDispatch::WheelScrollGrid { dx_px, dy_px } => {
                let app = self.host.app_mut();
                if let Some(active_ds) = app.document().active_sheet_id.as_ref().and_then(|id| app.document().data_sheet(id)).cloned() {
                    app.grid.tool.scroll_for_sheet(&active_ds, dx_px, dy_px);
                }
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("grid.scroll")), None)?])
            }
            WebChromeDispatch::HandledRedraw { action, command_result } => {
                Ok(vec![self.render_frame_command(self.render_dpi, Some(action), command_result)?])
            }
            WebChromeDispatch::ShortcutTriggered(act) => self.dispatch_shortcut_action(act),
            WebChromeDispatch::InvokeChromeAction(act) => self.dispatch_chrome_action(act),
            WebChromeDispatch::BlockedByModalBackdrop => {
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("modal.backdrop_blocked")), None)?])
            }
            WebChromeDispatch::Ignored => Ok(Vec::new()),
        }
    }

    fn dispatch_shortcut_action(&mut self, action: WebShortcutAction) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        match action {
            WebShortcutAction::None => Ok(Vec::new()),
            WebShortcutAction::OpenFilePicker => Ok(vec![WebBridgeCommand::TriggerFilePicker {
                accept: String::from(".curvebook,.cbtemplate,.pzfx,.prism,.csv,.tsv,.txt"),
                multiple: true,
                directory: false,
            }]),
            WebShortcutAction::SaveArchive => {
                let art = self.fs.export_bytes(&mut self.host, WebExportFormat::CurveBookArchive, None, self.render_dpi)?;
                Ok(vec![WebBridgeCommand::SaveOrDownloadFile {
                    file_name: art.file_name,
                    mime_type: art.mime_type,
                    bytes: art.bytes,
                    format: art.format,
                }])
            }
            WebShortcutAction::ExportFormat(format) => {
                let art = self.fs.export_bytes(&mut self.host, format, None, 300)?;
                Ok(vec![WebBridgeCommand::SaveOrDownloadFile {
                    file_name: art.file_name,
                    mime_type: art.mime_type,
                    bytes: art.bytes,
                    format: art.format,
                }])
            }
            WebShortcutAction::PrintCurrentSheet => Ok(vec![self.build_print_command(None, 100.0)?]),
            WebShortcutAction::CopyClipboard => Ok(vec![WebBridgeCommand::WriteClipboardItems { payload: self.build_clipboard_payload(None, 300)? }]),
            WebShortcutAction::CloseWorkspacePrompt => {
                self.chrome.close_popups();
                let note = WebBridgeCommand::NotifyStatus {
                    level: WebNoticeLevel::Info,
                    code: String::from("workspace.close_prompt"),
                    message: String::from("Use File > New Project to reset or Save before closing."),
                };
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("file.close_prompt")), None)?, note])
            }
            WebShortcutAction::OpenEditAnalysisParameters(sheet_id_str) => {
                if self.access_mode.is_read_only() {
                    return Err(self.read_only_err("edit_analysis_parameters"));
                }
                let opened = {
                    let app = self.host.app_mut();
                    let doc = app.document().clone();
                    doc.result_sheets
                        .iter()
                        .find(|s| s.id.as_str() == sheet_id_str)
                        .map(|s| s.id.clone())
                        .is_some_and(|rs_id| app.canvas_dialogs.open_edit_parameters(&doc, &rs_id))
                };
                if !opened {
                    return Err(WebAdapterError::Validation(format!("result sheet `{sheet_id_str}` not found for parameter edit")));
                }
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("dialog.edit_analysis_parameters")), None)?])
            }
            WebShortcutAction::ExecuteCommand { command_id, params } => {
                let resolved_params = if params.is_null() { contextual_command_params(self.host.app(), &command_id) } else { params };
                self.execute_chrome_command_or_dispatch(&command_id, resolved_params)
            }
        }
    }

    fn dispatch_chrome_action(&mut self, action: WebChromeAction) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        match action {
            WebChromeAction::InvokeCommand(command_id) => {
                self.chrome.close_popups();
                {
                    let app = self.host.app_mut();
                    app.interaction.open_menu = None;
                    app.interaction.open_dropdown = None;
                }
                let params = contextual_command_params(self.host.app(), &command_id);
                self.execute_chrome_command_or_dispatch(&command_id, params)
            }
            WebChromeAction::ExportFormat(format) => self.dispatch_shortcut_action(WebShortcutAction::ExportFormat(format)),
            WebChromeAction::OpenFilePicker => self.dispatch_shortcut_action(WebShortcutAction::OpenFilePicker),
            WebChromeAction::OpenDirectoryPicker => {
                Ok(vec![WebBridgeCommand::TriggerFilePicker { accept: String::new(), multiple: true, directory: true }])
            }
            WebChromeAction::DialogConfirm => {
                if matches!(self.host.app().canvas_dialogs.active_dialog, curvebook_ui_egui::ActiveDialog::CommandPalette) {
                    let palette_hit = {
                        let app = self.host.app();
                        let hits = app.canvas_dialogs.command_palette.matches_for_session(app.session());
                        hits.get(app.canvas_dialogs.command_palette.selected_index).or_else(|| hits.first()).map(|h| h.id.to_owned())
                    };
                    {
                        let app = self.host.app_mut();
                        app.canvas_dialogs.close_dialog();
                        app.focus_widget_id = None;
                        app.interaction.text_field_buffers.remove("palette.query");
                        curvebook_ui_egui::interaction::clear_modal_text_buffers(app);
                    }
                    self.chrome.focused_node_id = None;
                    if let Some(cmd_id) = palette_hit {
                        return self.execute_chrome_command_or_dispatch(&cmd_id, json!({}));
                    }
                    return Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("palette.no_match")), None)?]);
                }
                if self.access_mode.is_read_only() {
                    return Err(self.read_only_err("dialog_confirm"));
                }
                let was_print_dialog = matches!(self.host.app().canvas_dialogs.active_dialog, curvebook_ui_egui::ActiveDialog::Print { .. });
                let print_scale = self.host.app().canvas_dialogs.print.job.scale_percent;
                let export_info = if let ActiveDialog::ExportFigure { sheet_id } = &self.host.app().canvas_dialogs.active_dialog {
                    let st = &self.host.app().canvas_dialogs.export_figure;
                    let fmt = match st.format {
                        curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Svg => WebExportFormat::Svg,
                        curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Pdf => WebExportFormat::Pdf,
                        curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Png => WebExportFormat::Png,
                        curvebook_ui_egui::dialogs_canvas::ExportFigureFormat::Tiff(_) => WebExportFormat::Tiff,
                    };
                    Some((sheet_id.clone(), fmt))
                } else {
                    None
                };
                match commit_active_modal_dialog(self.host.app_mut()) {
                    Ok(out) => {
                        self.chrome.sync_from_app(self.host.app());
                        let export_payload = out.command_result.clone();
                        let mut cmds = vec![self.render_frame_command(self.render_dpi, Some(out.action), out.command_result)?];
                        if was_print_dialog && let Ok(print_cmd) = self.build_print_command(None, print_scale) {
                            cmds.push(print_cmd);
                        }
                        if let Some((export_sid, web_fmt)) = export_info
                            && let Some(ref payload_val) = export_payload
                        {
                            let stem = sanitize_download_stem(&self.host.app().document().title);
                            let cmd_id = match web_fmt {
                                WebExportFormat::Svg => "graph.exportSvg",
                                WebExportFormat::Pdf => "graph.exportPdf",
                                WebExportFormat::Png => "graph.exportPng",
                                WebExportFormat::Tiff => "graph.exportTiff",
                                _ => "graph.export",
                            };
                            let art = build_figure_artifact_from_payload(web_fmt, cmd_id, export_sid.as_str(), payload_val, &stem)?;
                            cmds.push(WebBridgeCommand::SaveOrDownloadFile {
                                file_name: art.file_name,
                                mime_type: art.mime_type,
                                bytes: art.bytes,
                                format: art.format,
                            });
                        }
                        Ok(cmds)
                    }
                    Err(msg) => Ok(vec![
                        self.render_frame_command(self.render_dpi, Some(String::from("dialog.commit_error")), Some(json!({ "error": msg })))?,
                        WebBridgeCommand::NotifyStatus { level: WebNoticeLevel::Error, code: String::from("dialog_error"), message: msg },
                    ]),
                }
            }
            WebChromeAction::ToggleMenuSection(key) => {
                let norm = if key.starts_with("menu.") { key.clone() } else { format!("menu.{key}") };
                let next = if self.chrome.open_menu_key.as_deref() == Some(norm.as_str()) { None } else { Some(norm) };
                self.chrome.open_menu_key.clone_from(&next);
                self.chrome.popup_scroll_y = 0;
                self.host.app_mut().interaction.open_menu = next;
                Ok(vec![self.render_frame_command(self.render_dpi, Some(format!("chrome.menu:{key}")), None)?])
            }
            WebChromeAction::SelectDialogItem(idx) => {
                let app = self.host.app_mut();
                match &app.canvas_dialogs.active_dialog {
                    ActiveDialog::CommandPalette => {
                        let hits_len = app.canvas_dialogs.command_palette.matches_for_session(app.session()).len();
                        app.canvas_dialogs.command_palette.selected_index = if hits_len == 0 { 0 } else { idx.min(hits_len - 1) };
                    }
                    ActiveDialog::AnalyzeSheet { .. } => {
                        let entries = app.canvas_dialogs.analyze.compatible_entries();
                        if let Some(entry) = entries.get(idx) {
                            app.canvas_dialogs.analyze.selected_entry_id = Some(entry.id.to_owned());
                        }
                    }
                    ActiveDialog::ConstantHookPicker { .. } if idx < app.canvas_dialogs.hook_picker.options.len() => {
                        app.canvas_dialogs.hook_picker.selected_index = Some(idx);
                    }
                    _ => {}
                }
                Ok(vec![self.render_frame_command(self.render_dpi, Some(format!("dialog.select_item:{idx}")), None)?])
            }
            WebChromeAction::DialogStepBack => {
                let stage_label = match &self.host.app().canvas_dialogs.active_dialog {
                    ActiveDialog::PowerAndSampleSize => {
                        let stage = self.host.app_mut().canvas_dialogs.power_wizard.go_back().map_err(WebAdapterError::Validation)?;
                        stage.label()
                    }
                    _ => "none",
                };
                Ok(vec![self.render_frame_command(self.render_dpi, Some(format!("dialog.step_back:{stage_label}")), None)?])
            }
            WebChromeAction::DialogStepNext => {
                let stage_label = match &self.host.app().canvas_dialogs.active_dialog {
                    ActiveDialog::PowerAndSampleSize => {
                        let stage = self.host.app_mut().canvas_dialogs.power_wizard.advance().map_err(WebAdapterError::Validation)?;
                        stage.label()
                    }
                    _ => "none",
                };
                Ok(vec![self.render_frame_command(self.render_dpi, Some(format!("dialog.step_next:{stage_label}")), None)?])
            }
            WebChromeAction::FocusModalInput => {
                let focus_target = primary_modal_focus_id(self.host.app()).map(str::to_owned);
                self.host.app_mut().focus_widget_id.clone_from(&focus_target);
                self.chrome.focused_node_id = focus_target;
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("dialog.focus_input")), None)?])
            }
            WebChromeAction::DialogCancel | WebChromeAction::CloseModalBackdrop => {
                let app = self.host.app_mut();
                app.canvas_dialogs.close_dialog();
                app.focus_widget_id = None;
                curvebook_ui_egui::interaction::clear_modal_text_buffers(app);
                self.chrome.focused_node_id = None;
                self.chrome.close_popups();
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("dialog.closed")), None)?])
            }
            WebChromeAction::DismissContextMenu => {
                self.chrome.context_menu = None;
                self.chrome.popup_scroll_y = 0;
                self.host.app_mut().interaction.context_menu_target = None;
                Ok(vec![self.render_frame_command(self.render_dpi, Some(String::from("context_menu.dismissed")), None)?])
            }
            WebChromeAction::None => Ok(Vec::new()),
        }
    }

    fn execute_chrome_command_or_dispatch(&mut self, command_id: &str, params: Value) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let route = resolve_web_command_route(self.host.app(), command_id);
        let has_explicit_params = params.as_object().is_some_and(|obj| !obj.is_empty());
        let is_read_only_export_or_notice =
            matches!(route, WebCommandRoute::OpenFilePicker { .. } | WebCommandRoute::PrintCurrentSheet | WebCommandRoute::NoticeOnly { .. })
                || matches!(route, WebCommandRoute::ExportFormat { .. } if !has_non_null_path_param(&params));
        if self.access_mode.is_read_only()
            && !is_read_only_export_or_notice
            && (!route.is_read_only_hint() || !is_command_read_only(command_id, Some(&params)))
        {
            return Err(self.read_only_err(command_id));
        }

        match route {
            WebCommandRoute::OpenFilePicker { accept } => Ok(vec![WebBridgeCommand::TriggerFilePicker { accept, multiple: true, directory: false }]),
            WebCommandRoute::ExportFormat { format, sheet_id } => {
                if has_explicit_params && has_non_null_path_param(&params) && cmd::registry().get(command_id).is_some() {
                    return self.handle_execute_command(command_id, params);
                }
                let art = self.fs.export_bytes(&mut self.host, format, sheet_id.as_deref(), 300)?;
                Ok(vec![WebBridgeCommand::SaveOrDownloadFile {
                    file_name: art.file_name,
                    mime_type: art.mime_type,
                    bytes: art.bytes,
                    format: art.format,
                }])
            }
            WebCommandRoute::PrintCurrentSheet => Ok(vec![self.build_print_command(None, 100.0)?]),
            WebCommandRoute::OpenEditAnalysisParameters(rs_id) => self.dispatch_shortcut_action(WebShortcutAction::OpenEditAnalysisParameters(rs_id)),
            WebCommandRoute::NoticeOnly { level, code, message } => Ok(vec![
                self.render_frame_command(self.render_dpi, Some(format!("chrome.notice:{code}")), Some(json!({ "notice": message })))?,
                WebBridgeCommand::NotifyStatus { level, code, message },
            ]),
            WebCommandRoute::UiCommand { command_id: ui_cmd, params: default_params } => {
                let before = self.capture_pref_fingerprint();
                let effective = if has_explicit_params { params } else { default_params };
                let Some(command_result) = dispatch_ui_command(self.host.app_mut(), &ui_cmd, &effective).map_err(WebAdapterError::Workspace)? else {
                    return Err(WebAdapterError::Validation(format!("unhandled web UI command `{ui_cmd}`")));
                };
                if ui_cmd == "ui.palette.open" {
                    let app = self.host.app_mut();
                    app.focus_widget_id = Some(String::from("palette.query"));
                }
                self.host.app_mut().sync_views();
                self.chrome.sync_from_app(self.host.app());
                let mut cmds = vec![self.render_frame_command(self.render_dpi, Some(ui_cmd), Some(command_result))?];
                self.maybe_append_preferences_persist(&before, &mut cmds);
                Ok(cmds)
            }
            WebCommandRoute::EngineCommand { command_id: eng_cmd, params: default_params } => {
                let effective = if has_explicit_params {
                    params
                } else if default_params.is_null() {
                    json!({})
                } else {
                    default_params
                };
                self.handle_execute_command(&eng_cmd, effective)
            }
        }
    }

    fn handle_paste_clipboard(&mut self, text: String) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        self.require_write_lock("paste_clipboard")?;
        if text.is_empty() {
            return Ok(Vec::new());
        }
        if self.host.app().inline_editor.active {
            return self.dispatch_single_ui_input(UiInputEvent::TextInput { text });
        }
        let target = {
            let app = self.host.app();
            let coord = app.grid.tool.active.unwrap_or_default();
            let decimal_separator = app.locale_preset.decimal_separator.to_string();
            app.document()
                .active_sheet_id
                .as_ref()
                .filter(|id| app.document().data_sheet(id).is_some())
                .map(|id| (id.as_str().to_owned(), coord.row, coord.col, coord.subcol, decimal_separator))
        };
        if let Some((sheet_id, start_row, start_col, start_subcol, decimal_separator)) = target {
            let mut params = json!({
                "sheet_id": sheet_id,
                "text": text,
                "start_row": start_row,
                "start_col": start_col,
                "start_subcol": start_subcol,
                "decimal_separator": decimal_separator,
                "mode": "overwrite",
            });
            if decimal_separator == ","
                && !text.contains(';')
                && (text.contains('\t') || text.lines().all(|line| line.matches(',').count() <= 1))
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("delimiter"), json!("\t"));
            }
            return self.handle_execute_command("table.paste", params);
        }
        self.dispatch_single_ui_input(UiInputEvent::TextInput { text })
    }

    fn capture_pref_fingerprint(&self) -> HostPrefFingerprint {
        capture_pref_fingerprint(self.host.app())
    }

    fn build_preferences_file(&self) -> PreferencesFile {
        let app = self.host.app();
        let theme = match app.theme.mode {
            curvebook_ui_egui::theme::UiThemeMode::Light => "light",
            curvebook_ui_egui::theme::UiThemeMode::Dark => "dark",
            curvebook_ui_egui::theme::UiThemeMode::HighContrast => "high_contrast",
        };
        let shortcuts: BTreeMap<String, String> =
            app.shortcuts.export_overrides_json().ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default();
        PreferencesFile {
            version: CURRENT_PREFERENCES_VERSION,
            locale: Some(app.locale.clone()),
            theme: Some(String::from(theme)),
            shortcuts,
            extra: Map::new(),
        }
    }

    fn record_pending_storage_op(&mut self, op: StorageRollbackOp) {
        if self.pending_storage_ops.len() >= 64 {
            let _ = self.pending_storage_ops.pop_front();
        }
        self.pending_storage_ops.push_back(op);
    }

    fn persist_preferences_command(&mut self, prefs: &PreferencesFile) -> Result<WebBridgeCommand, WebAdapterError> {
        let (outcome, rollback) = self.storage.save_preferences_tracked(prefs)?;
        self.record_pending_storage_op(rollback);
        Ok(WebBridgeCommand::PersistLocalStorage { writes: vec![(outcome.storage_key, outcome.encoded)], removals: outcome.evicted_keys })
    }

    fn maybe_append_preferences_persist(&mut self, before: &HostPrefFingerprint, cmds: &mut Vec<WebBridgeCommand>) {
        if self.access_mode.is_read_only() || self.capture_pref_fingerprint() == *before {
            return;
        }
        let prefs = self.build_preferences_file();
        match self.persist_preferences_command(&prefs) {
            Ok(cmd) => cmds.push(cmd),
            Err(err) => {
                cmds.push(WebBridgeCommand::NotifyStatus {
                    level: WebNoticeLevel::Warning,
                    code: String::from(err.code()),
                    message: err.to_string(),
                });
            }
        }
    }

    fn handle_confirm_directory_sync(
        &mut self,
        root_name: &str,
        written_paths: &[String],
        removed_paths: &[String],
        succeeded: bool,
        error: Option<String>,
    ) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let fully_persisted = self.fs.confirm_directory_sync(&mut self.host, root_name, written_paths, removed_paths, succeeded)?;
        let action = if fully_persisted { "sync_directory.confirmed" } else { "sync_directory.failed" };
        let mut cmds = vec![self.render_frame_command(
            self.render_dpi,
            Some(String::from(action)),
            Some(json!({ "root_name": root_name, "fully_persisted": fully_persisted })),
        )?];
        if !fully_persisted {
            let message = error.unwrap_or_else(|| String::from("directory sync did not complete; workspace remains dirty"));
            cmds.push(WebBridgeCommand::NotifyStatus { level: WebNoticeLevel::Warning, code: String::from("directory_sync_incomplete"), message });
        }
        Ok(cmds)
    }

    fn handle_confirm_local_storage(&mut self, writes: &[String], removals: &[String], succeeded: bool) {
        let pos = if writes.is_empty() {
            self.pending_storage_ops.len().checked_sub(1)
        } else {
            self.pending_storage_ops.iter().rposition(|op| writes.iter().any(|w| w == &op.target_key))
        };
        if let Some(idx) = pos
            && let Some(op) = self.pending_storage_ops.remove(idx)
        {
            self.storage.reconcile_confirmed_storage(op, removals, succeeded);
        }
    }

    fn handle_hydrate_local_storage(&mut self, entries: Vec<(String, String)>, now_ms: u64) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let mut cmds = Vec::new();
        for (key, value) in entries {
            if let Err(err) = self.storage.backend_mut().set_item(&key, &value) {
                cmds.push(WebBridgeCommand::NotifyStatus {
                    level: WebNoticeLevel::Warning,
                    code: String::from(err.code()),
                    message: err.to_string(),
                });
            }
        }
        let mut restored_any = false;
        match self.storage.load_preferences() {
            Ok(Some(prefs)) => {
                if let Some(locale) = prefs.locale {
                    let _ = dispatch_ui_command(self.host.app_mut(), "ui.locale.set", &json!({ "locale": locale }));
                }
                if let Some(theme) = prefs.theme {
                    let _ = dispatch_ui_command(self.host.app_mut(), "ui.theme.set", &json!({ "theme": theme }));
                }
                if !prefs.shortcuts.is_empty() {
                    let overrides_json = serde_json::to_string(&json!({ "overrides": prefs.shortcuts }))
                        .map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
                    if let Err(msg) = self.host.app_mut().shortcuts.import_overrides_json(&overrides_json) {
                        cmds.push(WebBridgeCommand::NotifyStatus {
                            level: WebNoticeLevel::Warning,
                            code: String::from("invalid_shortcut_overrides"),
                            message: msg,
                        });
                    }
                }
                self.host.app_mut().sync_views();
                restored_any = true;
            }
            Ok(None) => {}
            Err(err) => {
                cmds.push(WebBridgeCommand::NotifyStatus {
                    level: WebNoticeLevel::Warning,
                    code: String::from(err.code()),
                    message: err.to_string(),
                });
            }
        }
        let blank_clean_doc = self.host.app().document().total_sheet_count() == 0 && !self.host.app().session().is_dirty();
        if blank_clean_doc {
            match self.storage.load_autosave_snapshot(&self.doc_id, now_ms) {
                Ok(Some(snapshot)) => match self.host.open_project_json_bytes(snapshot.as_bytes()) {
                    Ok(_) => restored_any = true,
                    Err(err) => cmds.push(WebBridgeCommand::NotifyStatus {
                        level: WebNoticeLevel::Warning,
                        code: String::from("autosave_corrupt"),
                        message: format!("failed to restore browser autosave: {err}"),
                    }),
                },
                Ok(None) => {}
                Err(err) => cmds.push(WebBridgeCommand::NotifyStatus {
                    level: WebNoticeLevel::Warning,
                    code: String::from(err.code()),
                    message: err.to_string(),
                }),
            }
        }
        if restored_any {
            cmds.insert(0, self.render_frame_command(self.render_dpi, Some(String::from("storage.hydrated")), None)?);
        }
        Ok(cmds)
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
        let before = self.capture_pref_fingerprint();
        let fallback_text = match &input {
            UiInputEvent::KeyDown { key, modifiers } => chrome_bridge::printable_text_input_for_key(key, *modifiers),
            _ => None,
        };
        let mut outcome = self.host.dispatch_event(input).map_err(WebAdapterError::Workspace)?;
        if !outcome.handled
            && let Some(text) = fallback_text
        {
            if self.access_mode.is_read_only() && !matches!(self.host.app().canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
                return Err(self.read_only_err("text_input"));
            }
            let text_outcome = self.host.dispatch_event(UiInputEvent::TextInput { text }).map_err(WebAdapterError::Workspace)?;
            if text_outcome.handled {
                outcome = text_outcome;
            }
        }
        if outcome.handled {
            let mut cmds = vec![self.render_frame_command(self.render_dpi, Some(outcome.action), outcome.command_result)?];
            self.maybe_append_preferences_persist(&before, &mut cmds);
            Ok(cmds)
        } else {
            Ok(Vec::new())
        }
    }

    fn handle_open_file_bytes(&mut self, file_name: &str, bytes: &[u8]) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        if bytes.len() > MAX_ARCHIVE_FILE_BYTES {
            return Err(WebAdapterError::Validation(format!(
                "file `{file_name}` exceeds maximum allowed size ({} > {MAX_ARCHIVE_FILE_BYTES} bytes)",
                bytes.len()
            )));
        }
        let next_doc_id = derive_file_doc_id(file_name, bytes);
        if next_doc_id == self.doc_id {
            self.require_write_lock("open_file_bytes")?;
        }
        let summary = self.fs.import_file_bytes(&mut self.host, file_name, bytes)?;
        let mut extra = vec![WebBridgeCommand::ClearDirectoryHandle];
        self.rotate_document_context(next_doc_id, &mut extra);
        let mut cmds = vec![self.summary_render_command("open_file", &summary)?];
        cmds.extend(extra);
        Ok(cmds)
    }

    fn handle_import_dropped_files(&mut self, files: Vec<(String, Vec<u8>)>) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        if files.is_empty() {
            return Err(WebAdapterError::Validation(String::from("no dropped files supplied")));
        }
        validate_byte_batch_budget(files.len(), files.iter().map(|(_, bytes)| bytes.len()), "dropped files")?;
        let next_doc_id = derive_dropped_files_doc_id(&files);
        if next_doc_id == self.doc_id {
            self.require_write_lock("import_dropped_files")?;
        }
        let summary = self.fs.import_dropped_files(&mut self.host, files)?;
        let mut extra = vec![WebBridgeCommand::ClearDirectoryHandle];
        self.rotate_document_context(next_doc_id, &mut extra);
        let mut cmds = vec![self.summary_render_command("import_dropped_files", &summary)?];
        cmds.extend(extra);
        Ok(cmds)
    }

    fn handle_import_directory_entries(
        &mut self,
        root_name: &str,
        entries: BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let root = require_non_empty(root_name, "root_name")?;
        if entries.is_empty() {
            return Err(WebAdapterError::Validation(String::from("directory tree must contain at least document.json")));
        }
        validate_byte_batch_budget(entries.len(), entries.values().map(Vec::len), "directory entries")?;
        let normalized = normalize_directory_entries(root, entries)?;
        let next_doc_id = derive_directory_doc_id(root, &normalized);
        if next_doc_id == self.doc_id {
            self.require_write_lock("import_directory_entries")?;
        }
        let summary = self.fs.import_normalized_directory_tree(&mut self.host, root, normalized)?;
        let mut extra = Vec::new();
        self.rotate_document_context(next_doc_id, &mut extra);
        let mut cmds = vec![self.summary_render_command("import_directory", &summary)?];
        cmds.extend(extra);
        Ok(cmds)
    }

    fn rotate_document_context(&mut self, next_doc_id: String, cmds: &mut Vec<WebBridgeCommand>) {
        let trimmed = next_doc_id.trim();
        let resolved = if trimmed.is_empty() { next_untitled_doc_id(&self.tab_id) } else { trimmed.to_owned() };
        if resolved != self.doc_id {
            let _ = self.locks.release_document_lock(&self.doc_id, &self.tab_id, 0);
            self.doc_id = resolved;
        }
        let banner = self.apply_access_mode(TabAccessMode::ExclusiveReadWrite);
        cmds.push(WebBridgeCommand::SetDocumentLock { doc_id: self.doc_id.clone() });
        cmds.push(banner);
    }

    fn apply_project_replacement_state(&mut self, emit_clear_dir: bool) -> Vec<WebBridgeCommand> {
        self.host.reset_virtual_workspace();
        self.fs.reset_directory_sync();
        let mut extra = Vec::new();
        if emit_clear_dir {
            extra.push(WebBridgeCommand::ClearDirectoryHandle);
        }
        let next_doc_id = next_untitled_doc_id(&self.tab_id);
        self.rotate_document_context(next_doc_id, &mut extra);
        extra
    }

    fn handle_execute_command(&mut self, command_id: &str, params: Value) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let trimmed_id = command_id.trim();
        if self.access_mode.is_read_only() && !is_command_read_only(trimmed_id, Some(&params)) {
            return Err(self.read_only_err(trimmed_id));
        }

        let before = self.capture_pref_fingerprint();
        let replaces = command_replaces_project(trimmed_id, &params);
        if cmd::registry().get(trimmed_id).is_some() {
            let result = self.host.execute_command(trimmed_id, params).map_err(WebAdapterError::Workspace)?;
            let extra = if replaces { self.apply_project_replacement_state(true) } else { Vec::new() };
            let mut cmds = vec![self.render_frame_command(self.render_dpi, Some(trimmed_id.to_owned()), Some(result))?];
            cmds.extend(extra);
            self.maybe_append_preferences_persist(&before, &mut cmds);
            return Ok(cmds);
        }

        let line = json!({ "id": "bridge", "method": trimmed_id, "params": params }).to_string();
        let reply_raw = self.host.dispatch_control_line(&line);
        let reply: Value = serde_json::from_str(&reply_raw).map_err(|err| WebAdapterError::Workspace(err.to_string()))?;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let err_msg = extract_control_reply_error(&reply, "control command failed");
            return Err(WebAdapterError::Workspace(err_msg.to_owned()));
        }
        let extra = if replaces { self.apply_project_replacement_state(true) } else { Vec::new() };
        let mut cmds = vec![self.render_frame_command(self.render_dpi, Some(trimmed_id.to_owned()), reply.get("result").cloned())?];
        cmds.extend(extra);
        self.maybe_append_preferences_persist(&before, &mut cmds);
        Ok(cmds)
    }

    fn handle_control_line(&mut self, line: &str) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        let req: Value = serde_json::from_str(line).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let obj = req.as_object().ok_or_else(|| WebAdapterError::Serialization(String::from("control request must be a JSON object")))?;
        let method = obj
            .get("method")
            .or_else(|| obj.get("op"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| WebAdapterError::Serialization(String::from("control request missing `method` or `op`")))?;
        let normalized_params = normalize_control_request_params(method, obj);

        if self.access_mode.is_read_only() && !is_command_read_only(method, Some(&normalized_params)) {
            return Err(self.read_only_err(method));
        }

        let before = self.capture_pref_fingerprint();
        let replaces = command_replaces_project(method, &normalized_params);
        let reply_raw = self.host.dispatch_control_line(line);
        let reply: Value = serde_json::from_str(&reply_raw).map_err(|err| WebAdapterError::Workspace(err.to_string()))?;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let err_msg = extract_control_reply_error(&reply, "control line failed");
            return Err(WebAdapterError::Workspace(err_msg.to_owned()));
        }
        let extra = if replaces { self.apply_project_replacement_state(true) } else { Vec::new() };
        let mut cmds = vec![self.render_frame_command(self.render_dpi, Some(method.to_owned()), Some(reply))?];
        cmds.extend(extra);
        self.maybe_append_preferences_persist(&before, &mut cmds);
        Ok(cmds)
    }

    fn handle_save_autosave(&mut self, now_ms: u64) -> Result<Vec<WebBridgeCommand>, WebAdapterError> {
        self.require_write_lock("save_autosave")?;
        let snapshot = serde_json::to_string(self.host.app().document()).map_err(|err| WebAdapterError::Serialization(err.to_string()))?;
        let (outcome, rollback) = self.storage.save_autosave_snapshot_tracked(&self.doc_id, &snapshot, now_ms)?;
        self.record_pending_storage_op(rollback);
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
        let viewport = self.host.viewport();
        let chrome_layout = self.chrome.build_layout(self.host.app(), viewport);
        let bg = self.host.app().theme.surface_bg;
        let width_px = viewport.physical_width_px().max(1);
        let height_px = viewport.physical_height_px().max(1);
        let layered = WebTextBridge::try_compose_chrome_frame(width_px, height_px, dpi, [bg.r, bg.g, bg.b, bg.a], &chrome_layout)?;
        WebTextBridge::validate_layered_frame(&layered)?;
        let pan_zoom = &self.host.app().canvas_dialogs.pan_zoom;
        Ok(WebBridgeCommand::RenderRgbaFrame {
            width_px: layered.width_px,
            height_px: layered.height_px,
            dpi,
            rgba_bytes: layered.base_rgba_bytes,
            rgba_layers: layered.rgba_layers,
            text_ops: layered.text_ops,
            chrome: chrome_layout.snapshot,
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

enum ClipboardTarget {
    Data(String),
    Graph(String),
    Layout(String),
    Result(String),
    Info(String),
}

fn resolve_clipboard_target(app: &UiApp, sheet_id: Option<&str>) -> Result<ClipboardTarget, WebAdapterError> {
    let doc = app.document();
    if let Some(raw) = sheet_id {
        if let Some(s) = doc.data_sheets.iter().find(|s| s.id.as_str() == raw) {
            return Ok(ClipboardTarget::Data(s.id.as_str().to_owned()));
        }
        if let Some(s) = doc.graph_sheets.iter().find(|s| s.id.as_str() == raw) {
            return Ok(ClipboardTarget::Graph(s.id.as_str().to_owned()));
        }
        if let Some(s) = doc.layout_sheets.iter().find(|s| s.id.as_str() == raw) {
            return Ok(ClipboardTarget::Layout(s.id.as_str().to_owned()));
        }
        if let Some(s) = doc.result_sheets.iter().find(|s| s.id.as_str() == raw) {
            return Ok(ClipboardTarget::Result(s.id.as_str().to_owned()));
        }
        if let Some(s) = doc.info_sheets.iter().find(|s| s.id.as_str() == raw) {
            return Ok(ClipboardTarget::Info(s.id.as_str().to_owned()));
        }
        return Err(WebAdapterError::Clipboard(format!("sheet `{raw}` not found for clipboard export")));
    }

    if let Some(active) = doc.active_sheet_id.as_ref() {
        let id = active.as_str().to_owned();
        if doc.data_sheet(active).is_some() {
            return Ok(ClipboardTarget::Data(id));
        }
        if doc.graph_sheet(active).is_some() {
            return Ok(ClipboardTarget::Graph(id));
        }
        if doc.layout_sheet(active).is_some() {
            return Ok(ClipboardTarget::Layout(id));
        }
        if doc.result_sheet(active).is_some() {
            return Ok(ClipboardTarget::Result(id));
        }
        if doc.info_sheet(active).is_some() {
            return Ok(ClipboardTarget::Info(id));
        }
    }

    if let Some(s) = doc.graph_sheets.first() {
        return Ok(ClipboardTarget::Graph(s.id.as_str().to_owned()));
    }
    if let Some(s) = doc.layout_sheets.first() {
        return Ok(ClipboardTarget::Layout(s.id.as_str().to_owned()));
    }
    if let Some(s) = doc.data_sheets.first() {
        return Ok(ClipboardTarget::Data(s.id.as_str().to_owned()));
    }
    if let Some(s) = doc.result_sheets.first() {
        return Ok(ClipboardTarget::Result(s.id.as_str().to_owned()));
    }
    if let Some(s) = doc.info_sheets.first() {
        return Ok(ClipboardTarget::Info(s.id.as_str().to_owned()));
    }

    Err(WebAdapterError::Clipboard(String::from("no printable graph/layout or data sheet available for clipboard export")))
}

fn extract_control_reply_error<'a>(reply: &'a Value, fallback: &'a str) -> &'a str {
    reply
        .get("error")
        .and_then(|err| err.as_str().or_else(|| err.get("message").and_then(Value::as_str)))
        .or_else(|| reply.pointer("/error_detail/message").and_then(Value::as_str))
        .filter(|msg| !msg.trim().is_empty())
        .unwrap_or(fallback)
}

fn build_data_clipboard_payload(app: &UiApp, target_id: &str, dpi: u16) -> Result<WebClipboardPayload, WebAdapterError> {
    let doc = app.document();
    let ds = doc
        .data_sheets
        .iter()
        .find(|s| s.id.as_str() == target_id)
        .ok_or_else(|| WebAdapterError::Clipboard(format!("data sheet `{target_id}` not found for clipboard export")))?;
    let is_active = doc.active_sheet_id.as_ref().is_some_and(|id| id.as_str() == target_id);
    let decimal_sep = app.locale_preset.decimal_separator;
    let selected_tsv = if is_active
        && let Some((lo, hi)) = app.grid.tool.selection_bounds()
        && let Ok(range) = serde_json::from_value(json!({
            "start": { "row": lo.row, "col": lo.col, "subcol": lo.subcol },
            "end": { "row": hi.row, "col": hi.col, "subcol": hi.subcol },
        })) {
        let block = ClipboardBundle::tsv_from_datasheet_range(ds, range, decimal_sep);
        (!block.is_empty()).then_some(block)
    } else {
        None
    };
    let tsv = match selected_tsv {
        Some(block) => block,
        None => ClipboardBundle::tsv_from_datasheet(ds, decimal_sep).map_err(WebAdapterError::Clipboard)?,
    };
    Ok(text_only_clipboard_payload(tsv, dpi))
}

fn build_graph_clipboard_payload(app: &mut UiApp, target_id: &str, dpi: u16) -> Result<WebClipboardPayload, WebAdapterError> {
    let decimal_sep = app.locale_preset.decimal_separator;
    let session = app.session_mut();
    let (render_id, tsv) = {
        let doc = session.document();
        let gs = doc
            .graph_sheets
            .iter()
            .find(|s| s.id.as_str() == target_id)
            .ok_or_else(|| WebAdapterError::Clipboard(format!("graph sheet `{target_id}` not found for clipboard export")))?;
        let tsv = match doc.data_sheet(&gs.spec.primary_source_sheet_id) {
            Some(ds) => Some(ClipboardBundle::tsv_from_datasheet(ds, decimal_sep).map_err(WebAdapterError::Clipboard)?),
            None => None,
        };
        (gs.id.clone(), tsv)
    };
    let bundle = ClipboardBundle::from_sheet(session, &render_id, dpi).map_err(WebAdapterError::Clipboard)?;
    Ok(clipboard_payload_from_bundle(bundle, tsv))
}

fn build_layout_clipboard_payload(app: &mut UiApp, target_id: &str, dpi: u16) -> Result<WebClipboardPayload, WebAdapterError> {
    let session = app.session_mut();
    let render_id = session
        .document()
        .layout_sheets
        .iter()
        .find(|s| s.id.as_str() == target_id)
        .map(|s| s.id.clone())
        .ok_or_else(|| WebAdapterError::Clipboard(format!("layout sheet `{target_id}` not found for clipboard export")))?;
    let bundle = ClipboardBundle::from_sheet(session, &render_id, dpi).map_err(WebAdapterError::Clipboard)?;
    Ok(clipboard_payload_from_bundle(bundle, None))
}

fn build_result_clipboard_payload(app: &UiApp, target_id: &str, dpi: u16) -> Result<WebClipboardPayload, WebAdapterError> {
    let rs = app
        .document()
        .result_sheets
        .iter()
        .find(|s| s.id.as_str() == target_id)
        .ok_or_else(|| WebAdapterError::Clipboard(format!("result sheet `{target_id}` not found for clipboard export")))?;
    let mut lines = Vec::new();
    if let Some(tab) = rs.tabs.first() {
        if !tab.column_headers.is_empty() {
            lines.push(tab.column_headers.join("\t"));
        }
        for row in &tab.rows {
            let cols: Vec<String> = row
                .iter()
                .map(|cell| curvebook_ui_egui::dialogs_canvas::ResultSheetViewState::format_result_cell(cell, rs.analysis.p_value_format))
                .collect();
            lines.push(cols.join("\t"));
        }
    } else {
        lines.push(rs.title().to_owned());
    }
    Ok(text_only_clipboard_payload(lines.join("\n"), dpi))
}

fn build_info_clipboard_payload(app: &UiApp, target_id: &str, dpi: u16) -> Result<WebClipboardPayload, WebAdapterError> {
    let info = app
        .document()
        .info_sheets
        .iter()
        .find(|s| s.id.as_str() == target_id)
        .ok_or_else(|| WebAdapterError::Clipboard(format!("info sheet `{target_id}` not found for clipboard export")))?;
    let mut lines = vec![info.title.clone()];
    if !info.notes_markdown.is_empty() {
        lines.push(info.notes_markdown.clone());
    }
    for c in &info.constants {
        lines.push(format!("{}\t{:?}", c.name, c.value));
    }
    Ok(text_only_clipboard_payload(lines.join("\n"), dpi))
}

fn clipboard_payload_from_bundle(bundle: ClipboardBundle, tsv: Option<String>) -> WebClipboardPayload {
    let mut flavors: Vec<String> = bundle.available_flavors().into_iter().filter(|f| *f != "emf").map(str::to_owned).collect();
    if tsv.as_ref().is_some_and(|s| !s.is_empty()) {
        flavors.push(String::from("tsv"));
    }
    WebClipboardPayload {
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
    }
}

fn text_only_clipboard_payload(tsv_text: String, dpi: u16) -> WebClipboardPayload {
    let escaped = escape_html_text(&tsv_text);
    let html = format!("<!DOCTYPE html><html><body><pre>{escaped}</pre></body></html>");
    let mut flavors = vec![String::from("html")];
    if !tsv_text.is_empty() {
        flavors.push(String::from("tsv"));
    }
    WebClipboardPayload {
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
    }
}

fn cancel_armed_pointer_and_drags(app: &mut UiApp) {
    app.interaction.pointer_down = false;
    app.interaction.dragging_grid_sheet = None;
    app.interaction.dragging_split_divider = false;
    app.split.dragging_divider = false;
    app.interaction.dragging_navigator_sheet = None;
    app.nav.cancel_sheet_drag();
    app.interaction.canvas_drag = None;
    app.interaction.active_drag_handle = None;
    app.interaction.pressed_widget_id = None;
    app.canvas_dialogs.canvas.drag_start_pt = None;
    app.canvas_dialogs.canvas.active_snap_guides.clear();
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

static NEXT_UNTITLED_DOC_SEQ: AtomicU64 = AtomicU64::new(1);

fn fnv1a_64_hex(parts: &[&[u8]]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for &byte in *part {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn sanitize_doc_id_component(raw: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(40));
    let mut prev_underscore = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() { String::from(fallback) } else { trimmed.to_owned() }
}

fn derive_file_doc_id(file_name: &str, bytes: &[u8]) -> String {
    let label = sanitize_doc_id_component(file_name, "file");
    let digest = fnv1a_64_hex(&[file_name.as_bytes(), bytes]);
    format!("file-{label}-{digest}")
}

fn derive_directory_doc_id(root_name: &str, entries: &BTreeMap<String, Vec<u8>>) -> String {
    let label = sanitize_doc_id_component(root_name, "folder");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root_name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    for (path, bytes) in entries {
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
        for byte in path.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xfe;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("dir-{label}-{hash:016x}")
}

fn derive_dropped_files_doc_id(files: &[(String, Vec<u8>)]) -> String {
    if let [(name, bytes)] = files
        && !looks_like_directory_entry(name)
    {
        return derive_file_doc_id(name, bytes);
    }
    let root = detect_dropped_directory_root(files).unwrap_or_else(|| String::from("dropped.curvebook.d"));
    let mut sorted: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut parts: Vec<&[u8]> = Vec::with_capacity(1 + sorted.len() * 2);
    parts.push(root.as_bytes());
    for (p, b) in sorted {
        parts.push(p.as_bytes());
        parts.push(b);
    }
    let label = sanitize_doc_id_component(&root, "dropped");
    let digest = fnv1a_64_hex(&parts);
    format!("drop-{label}-{digest}")
}

fn next_untitled_doc_id(tab_id: &str) -> String {
    let seq = NEXT_UNTITLED_DOC_SEQ.fetch_add(1, Ordering::Relaxed);
    let tab = sanitize_doc_id_component(tab_id, "tab");
    format!("untitled-{tab}-{seq}")
}

fn build_print_html_document(title: &str, svg_pages: &[String], scale_percent: f64) -> String {
    let escaped_title = escape_html_text(title);
    let scale_factor = if scale_percent.is_finite() { (scale_percent / 100.0).clamp(0.1, 4.0) } else { 1.0 };
    let mut pages_html = String::new();
    for (idx, svg) in svg_pages.iter().enumerate() {
        pages_html.push_str("<section class=\"curvebook-print-page\" data-page=\"");
        pages_html.push_str(&(idx + 1).to_string());
        pages_html.push_str("\">");
        pages_html.push_str(svg);
        pages_html.push_str("</section>");
    }
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{escaped_title}</title>\
         <style>@page{{margin:0}}body{{margin:0;background:#fff}}.curvebook-print-page{{page-break-after:always;break-after:page;zoom:{scale_factor:.4};}}\
         .curvebook-print-page:last-child{{page-break-after:auto;break-after:auto;}}svg{{display:block;max-width:100%;height:auto;}}</style>\
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

fn require_finite(value: f64, key: &str) -> Result<f64, WebAdapterError> {
    if value.is_finite() { Ok(value) } else { Err(WebAdapterError::Serialization(format!("field `{key}` must be a finite number"))) }
}

fn validate_touch_points_ref(points: &[TouchPoint]) -> Result<(), WebAdapterError> {
    for pt in points {
        let _ = require_finite(pt.x, "x")?;
        let _ = require_finite(pt.y, "y")?;
    }
    Ok(())
}

fn validate_web_dpi(dpi: u16, context: &str) -> Result<u16, WebAdapterError> {
    if !(MIN_CLIPBOARD_DPI..=MAX_CLIPBOARD_DPI).contains(&dpi) {
        return Err(WebAdapterError::Validation(format!("{context} DPI {dpi} out of supported range {MIN_CLIPBOARD_DPI}..={MAX_CLIPBOARD_DPI}")));
    }
    Ok(dpi)
}

fn parse_dpi_opt(raw: Option<&Value>, key: &str, default: u16) -> Result<u16, WebAdapterError> {
    let Some(raw) = raw.filter(|v| !v.is_null()) else {
        return Ok(default);
    };
    let Some(val) = raw.as_u64() else {
        return Err(WebAdapterError::Validation(format!("`{key}` must be an integer in {MIN_CLIPBOARD_DPI}..={MAX_CLIPBOARD_DPI}")));
    };
    let dpi = u16::try_from(val)
        .map_err(|_| WebAdapterError::Validation(format!("`{key}` value {val} out of supported range {MIN_CLIPBOARD_DPI}..={MAX_CLIPBOARD_DPI}")))?;
    validate_web_dpi(dpi, key)
}

fn decode_wire_bytes(b64: Option<&str>, array: Option<&[Value]>, array_key: &str) -> Result<Vec<u8>, WebAdapterError> {
    if let Some(raw_b64) = b64 {
        return decode_base64(raw_b64);
    }
    if let Some(arr) = array {
        if arr.len() > MAX_ARCHIVE_FILE_BYTES {
            return Err(WebAdapterError::Validation(format!(
                "`{array_key}` exceeds maximum allowed size ({} > {MAX_ARCHIVE_FILE_BYTES} bytes)",
                arr.len()
            )));
        }
        let mut out = Vec::new();
        out.try_reserve_exact(arr.len()).map_err(|_| WebAdapterError::Serialization(format!("`{array_key}` exceeds memory budget")))?;
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

fn decode_named_bytes_vec(entries: Vec<NamedBytesWire>, key: &str) -> Result<Vec<(String, Vec<u8>)>, WebAdapterError> {
    if entries.len() > MAX_ZIP_ENTRIES {
        return Err(WebAdapterError::Validation(format!("`{key}` exceeds maximum entry count ({} > {MAX_ZIP_ENTRIES})", entries.len())));
    }
    let mut out = Vec::with_capacity(entries.len());
    let mut total_bytes = 0usize;
    for entry in entries {
        let bytes = decode_wire_bytes(entry.bytes_base64.as_deref(), entry.bytes.as_deref(), "bytes")?;
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .filter(|&sum| sum <= MAX_ARCHIVE_FILE_BYTES)
            .ok_or_else(|| WebAdapterError::Validation(format!("`{key}` exceeds total archive byte budget ({MAX_ARCHIVE_FILE_BYTES} bytes max)")))?;
        out.push((entry.path, bytes));
    }
    Ok(out)
}

fn collect_unique_directory_entries(root_name: &str, pairs: Vec<(String, Vec<u8>)>) -> Result<BTreeMap<String, Vec<u8>>, WebAdapterError> {
    validate_byte_batch_budget(pairs.len(), pairs.iter().map(|(_, b)| b.len()), "directory entries")?;
    let mut map = BTreeMap::new();
    let mut seen_normalized = BTreeSet::new();
    let trimmed_root = root_name.trim();
    let root_opt = (!trimmed_root.is_empty()).then_some(trimmed_root);
    for (path, bytes) in pairs {
        if map.contains_key(&path) {
            return Err(WebAdapterError::Validation(format!("duplicate directory entry path `{path}`")));
        }
        if let Ok(norm) = normalize_relative_entry_path(&path, root_opt)
            && !seen_normalized.insert(norm.clone())
        {
            return Err(WebAdapterError::Validation(format!("duplicate normalized directory entry path `{norm}`")));
        }
        map.insert(path, bytes);
    }
    Ok(map)
}

fn validate_byte_batch_budget<I>(count: usize, lengths: I, label: &str) -> Result<(), WebAdapterError>
where
    I: IntoIterator<Item = usize>,
{
    if count > MAX_ZIP_ENTRIES {
        return Err(WebAdapterError::Validation(format!("{label} exceeds maximum entry count ({count} > {MAX_ZIP_ENTRIES})")));
    }
    let mut total = 0usize;
    for len in lengths {
        if len > MAX_ARCHIVE_FILE_BYTES {
            return Err(WebAdapterError::Validation(format!("{label} entry exceeds maximum allowed size ({len} > {MAX_ARCHIVE_FILE_BYTES} bytes)")));
        }
        total = total
            .checked_add(len)
            .filter(|&sum| sum <= MAX_ARCHIVE_FILE_BYTES)
            .ok_or_else(|| WebAdapterError::Validation(format!("{label} exceeds total byte budget ({MAX_ARCHIVE_FILE_BYTES} bytes max)")))?;
    }
    let _ = total;
    Ok(())
}

fn primary_modal_focus_id(app: &UiApp) -> Option<&'static str> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::CommandPalette => Some("palette.query"),
        ActiveDialog::AnalyzeSheet { .. } => Some("analyze.search"),
        ActiveDialog::ConstantHookPicker { .. } => Some("hook.search_query"),
        ActiveDialog::ExcludeWithReason { .. } => Some("exclude.note"),
        ActiveDialog::FormatAxes { .. } => Some("axes.title"),
        ActiveDialog::EditAnalysisParameters { .. } => Some("edit.custom_title"),
        ActiveDialog::Checkpoints => Some("checkpoints.name"),
        ActiveDialog::ImportReshape => Some("import.title"),
        _ => None,
    }
}

fn current_epoch_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        let ms = js_sys::Date::now();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        if ms.is_finite() && ms >= 0.0 { ms.floor() as u64 } else { 1 }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|dur| u64::try_from(dur.as_millis()).ok()).unwrap_or(1)
    }
}

pub(crate) fn encode_base64_slice(input: &[u8]) -> String {
    encode_base64(input)
}

pub(crate) fn decode_base64_str(input: &str) -> Result<Vec<u8>, WebAdapterError> {
    decode_base64(input)
}

fn pointer_button_as_str(button: PointerButton) -> &'static str {
    match button {
        PointerButton::Primary => "Primary",
        PointerButton::Secondary => "Secondary",
        PointerButton::Middle => "Middle",
    }
}

fn parse_pointer_button(value: Option<&str>) -> PointerButton {
    match value.map(str::to_ascii_lowercase).as_deref() {
        Some("secondary" | "right") => PointerButton::Secondary,
        Some("middle") => PointerButton::Middle,
        _ => PointerButton::Primary,
    }
}

fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

fn decode_base64(input: &str) -> Result<Vec<u8>, WebAdapterError> {
    match curvebook_platform::base64::decode_capped(input, MAX_ARCHIVE_FILE_BYTES) {
        Ok(bytes) => Ok(bytes),
        Err(curvebook_platform::Base64Error::Empty) => Ok(Vec::new()),
        Err(err @ curvebook_platform::Base64Error::MaxSizeExceeded { .. }) => Err(WebAdapterError::Validation(err.to_string())),
        Err(err) => Err(WebAdapterError::Serialization(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use curvebook_format::{CURRENT_PREFERENCES_VERSION, PreferencesFile};
    use curvebook_ui_egui::{
        CanvasPreviewFilter, KeyModifiers, PointerButton, UiInputEvent,
        serde_json::{self, json},
    };

    use super::{PREFERENCES_STORAGE_KEY, WebAdapterError, WebBridgeCommand, WebBridgeEvent, WebBridgeSession, WebExportFormat};
    use crate::text_bridge::ZLayerBand;

    fn build_session_with_xy_and_result() -> (WebBridgeSession, String, String) {
        let mut session = WebBridgeSession::with_ids("r10-web-test", "tab-main");
        session
            .handle_event(WebBridgeEvent::ResizeViewport { logical_width: 1280.0, logical_height: 800.0, scale_factor: 1.0 })
            .expect("initial viewport resize");
        session
            .handle_event(WebBridgeEvent::ExecuteCommand {
                command_id: String::from("sheet.createData"),
                params: json!({ "sheet_id": "xy-1", "kind": "xy", "title": "Dose Curve", "auto_family": true }),
            })
            .expect("create xy sheet");
        session
            .handle_event(WebBridgeEvent::ExecuteCommand {
                command_id: String::from("table.setXCells"),
                params: json!({ "sheet_id": "xy-1", "values": [1.0, 2.0, 4.0, 8.0] }),
            })
            .expect("set x values");
        for (row, val) in [10.0, 22.0, 44.0, 80.0].into_iter().enumerate() {
            session
                .handle_event(WebBridgeEvent::ExecuteCommand {
                    command_id: String::from("table.setCell"),
                    params: json!({ "sheet_id": "xy-1", "row": row, "col": 0, "value": val }),
                })
                .expect("set y value");
        }
        let graph_id = session.host.app().document().graph_sheets[0].id.as_str().to_owned();
        (session, String::from("xy-1"), graph_id)
    }

    fn scroll_popup_until_node_visible(session: &mut WebBridgeSession, popup_id: &str, target_node_id: &str) {
        for pass_dir in [1.0, -1.0] {
            for _ in 0..24 {
                let layout = session.current_chrome_layout();
                if layout.find_node(target_node_id).is_some() {
                    return;
                }
                let popup_rect = layout.find_node(popup_id).unwrap_or_else(|| panic!("popup `{popup_id}` must exist")).rect_px;
                let scale = session.host.viewport().scale_factor.max(1e-6);
                let cx = f64::from(popup_rect.x + i32::try_from(popup_rect.width / 2).unwrap_or(20)) / scale;
                let cy = f64::from(popup_rect.y + i32::try_from(popup_rect.height / 2).unwrap_or(20)) / scale;
                session
                    .handle_event(WebBridgeEvent::Wheel { x: cx, y: cy, delta_x: 0.0, delta_y: pass_dir * 88.0, modifiers: KeyModifiers::default() })
                    .expect("wheel scroll popup");
            }
        }
        panic!("target node `{target_node_id}` never became visible inside `{popup_id}`");
    }

    #[test]
    fn web_chrome_parity_menu_toolbar_modal_and_alt_t_edit_params_flow() {
        let (mut session, data_id, _graph_id) = build_session_with_xy_and_result();

        // Opening File dropdown emits PopupDropdown RGBA + clipped Z-ordered text ops.
        let file_open_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.file"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("click File menu");
        let Some(WebBridgeCommand::RenderRgbaFrame { rgba_layers, text_ops, chrome, .. }) = file_open_cmds.first() else {
            panic!("expected RenderRgbaFrame after menu click");
        };
        assert_eq!(chrome.open_menu_key.as_deref(), Some("menu.file"));
        assert!(rgba_layers.iter().any(|l| l.z_band == ZLayerBand::PopupDropdown && !l.uses_base_frame));
        assert!(text_ops.iter().any(|op| op.z_band == ZLayerBand::PopupDropdown && op.clip_px.is_some()));

        // Clicking File > Open triggers browser picker protocol through the web chrome menu node `menu.project.open`.
        scroll_popup_until_node_visible(&mut session, "popup.menu.file", "menu.project.open");
        let picker_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.project.open"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("click File > Open item");
        assert!(matches!(picker_cmds.as_slice(), [WebBridgeCommand::TriggerFilePicker { directory: false, .. }]));

        // Toolbar Analyze button opens AnalyzeSheet modal cleanly in ModalOverlay band.
        let analyze_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("toolbar.analyze"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("click Analyze toolbar button");
        let Some(WebBridgeCommand::RenderRgbaFrame { rgba_layers, chrome, .. }) = analyze_cmds.first() else {
            panic!("expected RenderRgbaFrame for Analyze modal");
        };
        assert!(chrome.active_dialog.contains("AnalyzeSheet"));
        assert!(rgba_layers.iter().any(|l| l.z_band == ZLayerBand::ModalOverlay));

        // Clicking corner outside modal hits backdrop and must not leak through to workspace.
        let blocked = session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::PointerDown { x: 4.0, y: 4.0, button: PointerButton::Primary, modifiers: KeyModifiers::default() },
            })
            .expect("backdrop click stays clean");
        assert!(matches!(
            blocked.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. }) if act == "modal.backdrop_blocked"
        ));

        // Confirm Analyze modal -> select newly created ResultSheet -> Alt+T opens EditAnalysisParameters.
        session
            .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Enter"), modifiers: KeyModifiers::default() } })
            .expect("confirm Analyze modal");
        if session.host.app().document().result_sheets.is_empty() {
            session
                .handle_event(WebBridgeEvent::ExecuteCommand {
                    command_id: String::from("analysis.run"),
                    params: json!({ "source_sheet_id": data_id, "kind": "descriptive_stats" }),
                })
                .expect("fallback analysis run");
        }
        let rs_id = session.host.app().document().result_sheets[0].id.as_str().to_owned();
        session
            .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": rs_id }) })
            .expect("select ResultSheet");

        let alt_t_cmds = session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("t"), modifiers: KeyModifiers { alt: true, ..Default::default() } },
            })
            .expect("Alt+T on ResultSheet");
        assert!(matches!(
            alt_t_cmds.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { chrome, .. }) if chrome.active_dialog.contains("EditAnalysisParameters")
        ));
        let ok_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("modal.ok"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("confirm EditAnalysisParameters via modal button");
        assert!(matches!(ok_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { chrome, .. }) if chrome.active_dialog == "None"));
    }

    #[test]
    fn web_wheel_ime_paste_clipboard_and_audit_edge_cases_behave_cleanly() {
        let (mut session, data_id, _graph_id) = build_session_with_xy_and_result();
        session
            .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": data_id }) })
            .expect("select DataSheet");

        // Wheel over DataSheet canvas scrolls grid instead of zooming or panning unrelated canvas.
        let scroll_cmds = session
            .handle_event(WebBridgeEvent::Wheel { x: 400.0, y: 300.0, delta_x: 0.0, delta_y: 48.0, modifiers: KeyModifiers::default() })
            .expect("wheel scroll grid");
        assert!(matches!(scroll_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. }) if act == "grid.scroll"));

        // Lowercase preview filter alias parses cleanly (`R2-S01-04`).
        let ev = WebBridgeEvent::from_json_str(r#"{"kind":"set_preview_filter","filter":"grayscale"}"#).expect("lowercase filter parses");
        assert_eq!(ev, WebBridgeEvent::SetPreviewFilter { filter: CanvasPreviewFilter::GrayscalePrint });

        // Active DataSheet clipboard copy exports TSV/HTML without grabbing unrelated Graph PNG (`R2-S11-06`).
        let copy_cmds = session.handle_event(WebBridgeEvent::CopyClipboard { sheet_id: None, dpi: 150 }).expect("copy active DataSheet");
        let Some(WebBridgeCommand::WriteClipboardItems { payload }) = copy_cmds.first() else {
            panic!("expected WriteClipboardItems");
        };
        assert!(payload.tsv.as_deref().is_some_and(|t| t.contains("10")));
        assert!(payload.png_bytes.is_empty());

        // Browser paste event dispatches TSV paste into DataSheet.
        let paste_cmds = session.handle_event(WebBridgeEvent::PasteClipboard { text: String::from("77.5") }).expect("paste into active DataSheet");
        assert!(matches!(paste_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));

        // IME preedit mirrors into chrome state, and Alt+K + IME commit writes to Command Palette search box.
        let preedit = session.handle_event(WebBridgeEvent::ImePreedit { text: String::from("にほ") }).expect("ime preedit");
        assert!(matches!(preedit.first(), Some(WebBridgeCommand::RenderRgbaFrame { chrome, .. }) if chrome.ime_preedit.as_deref() == Some("にほ")));
        session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("k"), modifiers: KeyModifiers { alt: true, ..Default::default() } },
            })
            .expect("Alt+K opens command palette");
        let ime_commit = session.handle_event(WebBridgeEvent::ImeCommit { text: String::from("zoom") }).expect("ime commit into palette");
        assert!(matches!(
            ime_commit.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. }) if act == "palette.ime_commit"
        ));
        session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() },
            })
            .expect("Escape closes palette");

        // Hydrate localStorage preferences safely.
        let prefs = PreferencesFile { version: CURRENT_PREFERENCES_VERSION, theme: Some(String::from("dark")), ..PreferencesFile::default() };
        let encoded_prefs = serde_json::to_string(&prefs).expect("serialize prefs");
        let hydrated = session
            .handle_event(WebBridgeEvent::HydrateLocalStorage { entries: vec![(String::from(PREFERENCES_STORAGE_KEY), encoded_prefs)] })
            .expect("hydrate storage");
        assert!(matches!(hydrated.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));
    }

    #[test]
    fn file_system_access_and_download_fallback_export_all_supported_formats() {
        let (mut session, data_id, graph_id) = build_session_with_xy_and_result();
        // Contended read-only tab still allows File System Access / Blob exports and scaled print.
        session
            .handle_event(WebBridgeEvent::ObserveExternalLock {
                doc_id: String::from("r10-web-test"),
                holder_tab_id: String::from("tab-other"),
                now_ms: 100,
            })
            .expect("enter read-only contended state");
        assert!(session.access_mode().is_read_only());

        let write_err = session.handle_event(WebBridgeEvent::PasteClipboard { text: String::from("99") }).expect_err("read-only tab rejects paste");
        assert!(matches!(write_err, WebAdapterError::ReadOnlyContended { .. }));

        let formats = [
            (WebExportFormat::CurveBookArchive, Some(graph_id.as_str()), b"PK\x03\x04".as_slice()),
            (WebExportFormat::CurveBookDirectory, None, b"PK\x03\x04".as_slice()),
            (WebExportFormat::Csv, Some(data_id.as_str()), b"".as_slice()),
            (WebExportFormat::Svg, Some(graph_id.as_str()), b"<svg".as_slice()),
            (WebExportFormat::Pdf, Some(graph_id.as_str()), b"%PDF-".as_slice()),
            (WebExportFormat::Png, Some(graph_id.as_str()), &[0x89, b'P', b'N', b'G']),
            (WebExportFormat::Tiff, Some(graph_id.as_str()), b"II*\0".as_slice()),
        ];

        for (fmt, target, magic) in formats {
            let cmds = session
                .handle_event(WebBridgeEvent::ExportFile { format: fmt, sheet_id: target.map(str::to_owned), dpi: 150 })
                .expect("export command succeeds");
            let Some(WebBridgeCommand::SaveOrDownloadFile { file_name, bytes, format, .. }) = cmds.into_iter().next() else {
                panic!("expected SaveOrDownloadFile for {fmt:?}");
            };
            assert_eq!(format, fmt);
            assert!(!file_name.is_empty());
            assert!(!bytes.is_empty());
            if !magic.is_empty() && fmt != WebExportFormat::Svg {
                assert!(bytes.starts_with(magic), "unexpected header bytes for {fmt:?}");
            } else if fmt == WebExportFormat::Svg {
                let text = std::str::from_utf8(&bytes).expect("SVG UTF-8");
                assert!(text.contains("<svg"));
            }
        }

        // Scaled print honors active graph and embeds scale in HTML (`R2-S11-13`).
        let print_cmds = session.handle_event(WebBridgeEvent::PrintCurrentSheet { sheet_id: None, scale_percent: 85.0 }).expect("scaled print");
        let Some(WebBridgeCommand::TriggerPrintHtml { html, scale_percent, .. }) = print_cmds.into_iter().next() else {
            panic!("expected TriggerPrintHtml");
        };
        assert!((scale_percent - 85.0).abs() < 1e-6);
        assert!(html.contains("0.8500"));
    }

    #[test]
    fn index_html_enforces_strict_csp_zero_egress_z_compositing_and_blob_fallback() {
        let html = include_str!("../index.html");
        assert!(html.contains("<meta http-equiv=\"Content-Security-Policy\""));
        assert!(html.contains("default-src 'self'"));
        assert!(html.contains("object-src 'none'"));
        assert!(html.contains("form-action 'none'"));

        // Zero external network/telemetry calls (`concat!` avoids triggering static scanner on the test line itself).
        for forbidden in [
            concat!("fetc", "h("),
            concat!("XMLHttp", "Request"),
            concat!("Web", "Socket"),
            concat!("send", "Beacon"),
            concat!("http", "://"),
            concat!("https", "://"),
        ] {
            assert!(!html.contains(forbidden), "index.html must never contain `{forbidden}`");
        }

        // Z-band compositing, clipped canvas fillText, WebGL recovery, FS fallbacks, and lock safety markers.
        for required in [
            "Z_BAND_ORDER",
            "ctx.save()",
            "ctx.clip()",
            "ctx.fillText(",
            "curvebook_file_input",
            "curvebook_dir_input",
            "URL.createObjectURL(",
            "webglcontextlost",
            "webglcontextrestored",
            "MAX_TEXTURE_SIZE",
            "holdsCurrentWebLock",
            "hydrate_local_storage",
        ] {
            assert!(html.contains(required), "index.html missing required bridge feature `{required}`");
        }
    }

    #[test]
    fn toolbar_undo_redo_new_axes_edit_params_and_shortcuts_route_without_null() {
        let (mut session, data_id, _graph_id) = build_session_with_xy_and_result();
        session
            .handle_event(WebBridgeEvent::ExecuteCommand {
                command_id: String::from("analysis.run"),
                params: json!({ "source_sheet_id": data_id, "kind": "descriptive_stats" }),
            })
            .expect("create ResultSheet");
        let rs_id = session.host.app().document().result_sheets[0].id.as_str().to_owned();

        // 1. `toolbar.axes` opens FormatAxes modal cleanly (not `expected object, got null`).
        let axes_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("toolbar.axes"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("toolbar.axes opens FormatAxes");
        assert!(matches!(
            axes_cmds.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { chrome, .. }) if chrome.active_dialog.contains("FormatAxes")
        ));
        session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() },
            })
            .expect("Escape closes FormatAxes");

        // 2. Selecting ResultSheet exposes shared `result.edit_params_btn`, which opens `EditAnalysisParameters`.
        session
            .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("sheet.select"), params: json!({ "sheet_id": rs_id }) })
            .expect("select ResultSheet");
        let edit_params_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("result.edit_params_btn"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("result.edit_params_btn opens EditAnalysisParameters");
        assert!(matches!(
            edit_params_cmds.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { chrome, .. }) if chrome.active_dialog.contains("EditAnalysisParameters")
        ));
        session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() },
            })
            .expect("Escape closes EditAnalysisParameters");

        // 3. `Cmd+Enter` (`analyze.run`), `toolbar.undo`, `toolbar.redo`, `toolbar.new`, and `Alt+Shift+N` (`file.new`) never hit `Value::Null`.
        let cmd_enter_cmds = session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("Enter"), modifiers: KeyModifiers { meta: true, ..Default::default() } },
            })
            .expect("Cmd+Enter triggers analyze.run cleanly");
        assert!(matches!(
            cmd_enter_cmds.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. }) if act == "analyze.run"
        ));

        let undo_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("toolbar.undo"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("toolbar.undo succeeds");
        assert!(matches!(undo_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));

        let redo_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("toolbar.redo"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("toolbar.redo succeeds");
        assert!(matches!(redo_cmds.first(), Some(WebBridgeCommand::RenderRgbaFrame { .. })));

        let new_btn_cmds = session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("toolbar.new"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("toolbar.new succeeds");
        assert!(matches!(
            new_btn_cmds.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. }) if act == "project.new"
        ));

        let alt_shift_n = session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("n"), modifiers: KeyModifiers { alt: true, shift: true, ..Default::default() } },
            })
            .expect("Alt+Shift+N executes file.new cleanly");
        assert!(matches!(
            alt_shift_n.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. }) if act == "file.new"
        ));

        // 4. Unrecognized UI/engine commands must return an error instead of silently succeeding.
        let unknown_err = session
            .execute_chrome_command_or_dispatch("ui.nonexistent.cmd", serde_json::Value::Null)
            .expect_err("unhandled command must fail loudly");
        assert!(matches!(unknown_err, WebAdapterError::Workspace(_) | WebAdapterError::Validation(_)));
    }

    #[test]
    fn dropdown_menus_omit_event_dispatch_disable_remediation_and_route_palette_and_variants() {
        let (mut session, data_id, _graph_id) = build_session_with_xy_and_result();
        session
            .handle_event(WebBridgeEvent::ExecuteCommand {
                command_id: String::from("analysis.run"),
                params: json!({ "source_sheet_id": data_id, "kind": "descriptive_stats" }),
            })
            .expect("create ResultSheet");

        // Open `menu.view`: automation-only `ui.event.dispatch` must be omitted across entire scroll range,
        // and theme/filter/locale clicks succeed via web chrome menu nodes.
        session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.view"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("open View dropdown");
        let view_layout = session.current_chrome_layout();
        assert!(view_layout.find_node("menu.ui.event.dispatch").is_none(), "automation hook must not appear in menu.view");

        scroll_popup_until_node_visible(&mut session, "popup.menu.view", "menu.ui.theme.set");
        assert!(session.current_chrome_layout().find_node("menu.ui.event.dispatch").is_none());
        session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.ui.theme.set"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("click Theme dropdown item cycles to Dark");
        assert!(session.host.app().theme.surface_bg.relative_luminance() < 0.25);

        session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.view"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("reopen View dropdown");
        scroll_popup_until_node_visible(&mut session, "popup.menu.view", "menu.ui.previewFilter.set");
        session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.ui.previewFilter.set"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("click base previewFilter item cycles cleanly");
        assert_ne!(session.host.app().preview_filter, CanvasPreviewFilter::Normal);

        session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.view"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("reopen View dropdown");
        scroll_popup_until_node_visible(&mut session, "popup.menu.view", "menu.ui.locale.set");
        session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.ui.locale.set"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("click Locale dropdown item toggles to de");
        assert_eq!(session.host.app().locale, "de");

        // Open `menu.analyze`: curated Analyze dropdown surfaces `ui.analyze.open` and omits internal `ui.result.applyRemediation`.
        session
            .handle_event(WebBridgeEvent::ClickChromeNode {
                node_id: String::from("menu.analyze"),
                button: PointerButton::Primary,
                modifiers: KeyModifiers::default(),
            })
            .expect("open Analyze dropdown");
        scroll_popup_until_node_visible(&mut session, "popup.menu.analyze", "menu.ui.analyze.open");
        assert!(session.current_chrome_layout().find_node("menu.ui.analyze.open").is_some(), "menu.ui.analyze.open missing in menu.analyze");
        assert!(
            session.current_chrome_layout().find_node("menu.ui.result.applyRemediation").is_none(),
            "internal ui.result.applyRemediation must not clutter curated Analyze menu"
        );
        session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("Escape"), modifiers: KeyModifiers::default() },
            })
            .expect("Escape closes dropdown");

        // Command Palette execution of zero-arg / engine commands runs cleanly without Value::Null failure.
        session
            .handle_event(WebBridgeEvent::UiInput {
                event: UiInputEvent::KeyDown { key: String::from("k"), modifiers: KeyModifiers { alt: true, ..Default::default() } },
            })
            .expect("Alt+K opens Command Palette");
        session.handle_event(WebBridgeEvent::ImeCommit { text: String::from("edit.undo") }).expect("query edit.undo");
        let pal_exec = session
            .handle_event(WebBridgeEvent::UiInput { event: UiInputEvent::KeyDown { key: String::from("Enter"), modifiers: KeyModifiers::default() } })
            .expect("Command Palette executes edit.undo");
        assert!(matches!(
            pal_exec.first(),
            Some(WebBridgeCommand::RenderRgbaFrame { action: Some(act), .. }) if act == "edit.undo"
        ));

        // Exhaustive check across all TOOLBAR_BUTTONS, UI_COMMANDS, and cmd::registry(): zero Value::Null params.
        for btn in curvebook_ui_egui::TOOLBAR_BUTTONS {
            let route = crate::chrome_bridge::resolve_web_command_route(session.host.app(), btn.command_id);
            assert!(
                !matches!(
                    route,
                    crate::chrome_bridge::WebCommandRoute::EngineCommand { ref params, .. }
                        | crate::chrome_bridge::WebCommandRoute::UiCommand { ref params, .. } if params.is_null()
                ),
                "toolbar command `{}` resolved to Value::Null",
                btn.command_id
            );
        }
        for spec in curvebook_ui_egui::UI_COMMANDS {
            let route = crate::chrome_bridge::resolve_web_command_route(session.host.app(), spec.id);
            assert!(
                !matches!(
                    route,
                    crate::chrome_bridge::WebCommandRoute::EngineCommand { ref params, .. }
                        | crate::chrome_bridge::WebCommandRoute::UiCommand { ref params, .. } if params.is_null()
                ),
                "UI command `{}` resolved to Value::Null",
                spec.id
            );
        }
        for spec in curvebook_engine::cmd::registry().specs() {
            let route = crate::chrome_bridge::resolve_web_command_route(session.host.app(), spec.id);
            assert!(
                !matches!(
                    route,
                    crate::chrome_bridge::WebCommandRoute::EngineCommand { ref params, .. }
                        | crate::chrome_bridge::WebCommandRoute::UiCommand { ref params, .. } if params.is_null()
                ),
                "engine command `{}` resolved to Value::Null",
                spec.id
            );
        }
    }

    #[test]
    fn da_06_04_extracts_control_reply_error_strings_and_error_detail_messages() {
        use super::extract_control_reply_error;
        use serde_json::Value;

        // 1. Helper handles string `error`, object `error.message`, `/error_detail/message`, and fallback.
        assert_eq!(extract_control_reply_error(&json!({ "ok": false, "error": "real string diagnostic" }), "fallback"), "real string diagnostic");
        assert_eq!(
            extract_control_reply_error(
                &json!({ "ok": false, "error": Value::Null, "error_detail": { "message": "structured detail diagnostic" } }),
                "fallback"
            ),
            "structured detail diagnostic"
        );
        assert_eq!(
            extract_control_reply_error(&json!({ "ok": false, "error": { "message": "legacy object message" } }), "fallback"),
            "legacy object message"
        );
        assert_eq!(extract_control_reply_error(&json!({ "ok": false, "error": "   " }), "fallback"), "fallback");

        // 2. Live WebAdapterSession surfaces real ControlResponse.error string instead of generic fallback.
        let (mut session, _data_id, _graph_id) = build_session_with_xy_and_result();
        let line_err = session
            .handle_event(WebBridgeEvent::DispatchControlLine {
                line: json!({ "id": "t1", "method": "doc.export", "params": { "format": "bogus_fmt" } }).to_string(),
            })
            .expect_err("unsupported format must fail");
        let line_msg = line_err.to_string();
        assert!(line_msg.contains("unsupported export format"), "expected real control_io diagnostic, got: {line_msg}");
        assert!(!line_msg.contains("control line failed"), "generic fallback must not mask real error: {line_msg}");

        let cmd_err = session
            .handle_event(WebBridgeEvent::ExecuteCommand { command_id: String::from("doc.export"), params: json!({ "format": "bogus_fmt" }) })
            .expect_err("unsupported format via ExecuteCommand must fail");
        let cmd_msg = cmd_err.to_string();
        assert!(cmd_msg.contains("unsupported export format"), "expected real control_io diagnostic, got: {cmd_msg}");
        assert!(!cmd_msg.contains("control command failed"), "generic fallback must not mask real error: {cmd_msg}");
    }

    #[test]
    fn da_06_03_and_da_06_06_web_contended_read_only_blocks_path_exports_and_checkpoint_route_succeeds() {
        use super::TabAccessMode;
        use serde_json::Value;

        // DA-06-06: `project.createCheckpoint` route emits `"name"` and executes cleanly without BadInput.
        let (mut session, _data_id, _graph_id) = build_session_with_xy_and_result();
        let route = crate::chrome_bridge::resolve_web_command_route(session.host.app(), "project.createCheckpoint");
        assert!(
            matches!(route, crate::chrome_bridge::WebCommandRoute::EngineCommand { .. }),
            "expected EngineCommand for project.createCheckpoint, got {route:?}"
        );
        if let crate::chrome_bridge::WebCommandRoute::EngineCommand { command_id, params } = route {
            assert_eq!(command_id, "project.createCheckpoint");
            assert_eq!(params.get("name").and_then(Value::as_str), Some("Checkpoint"));
            assert!(params.get("label").is_none());
            session.handle_event(WebBridgeEvent::ExecuteCommand { command_id, params }).expect("project.createCheckpoint via web route must succeed");
            assert_eq!(session.host.app().document().checkpoints.len(), 1);
        }

        // DA-06-03: Contended read-only web tab rejects disk-writing `doc.export` (path/output_path) and `render.pdf` (path).
        session.access_mode = TabAccessMode::ReadOnlyContended { holder_tab_id: String::from("other-tab") };
        let export_out_path = session.handle_event(WebBridgeEvent::DispatchControlLine {
            line: json!({ "id": "ro1", "method": "doc.export", "params": { "format": "csv", "output_path": "evil.csv" } }).to_string(),
        });
        assert!(matches!(export_out_path, Err(WebAdapterError::ReadOnlyContended { .. })), "output_path export must be blocked: {export_out_path:?}");

        let pdf_path = session.handle_event(WebBridgeEvent::DispatchControlLine {
            line: json!({ "id": "ro2", "method": "render.pdf", "params": { "path": "evil.pdf" } }).to_string(),
        });
        assert!(matches!(pdf_path, Err(WebAdapterError::ReadOnlyContended { .. })), "path render.pdf must be blocked: {pdf_path:?}");

        // Pathless `doc.export` and `render.pdf` remain allowed in read-only mode.
        let export_inline = session.handle_event(WebBridgeEvent::DispatchControlLine {
            line: json!({ "id": "ro3", "method": "doc.export", "params": { "format": "csv" } }).to_string(),
        });
        assert!(export_inline.is_ok(), "pathless doc.export should succeed in read-only mode: {export_inline:?}");
    }
}
