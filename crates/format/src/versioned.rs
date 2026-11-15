//! Shared versioned-file migration framework, preferences persistence, and batch schema validation.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::FormatError;

/// Current persisted version of user preferences files.
pub const CURRENT_PREFERENCES_VERSION: u32 = 1;

/// Maximum byte length permitted when reading a user preferences JSON file from disk.
pub const MAX_PREFERENCES_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Expected `schema` identifier for v1 headless batch manifests.
pub const BATCH_SCHEMA_V1: &str = "lumencraft.batch.v1";

/// Single forward migration step transforming raw JSON from `from_version` to `from_version + 1`.
#[derive(Debug, Clone, Copy)]
pub struct VersionStep {
    /// Source schema version matched by this step.
    pub from_version: u32,
    /// In-place transformation applied to the raw JSON payload.
    pub apply: fn(&mut serde_json::Value) -> Result<(), FormatError>,
}

/// Upgrade a raw `serde_json::Value` payload step-by-step from `from_version` to `current_version`.
///
/// Rejects versions strictly below `first_supported` or strictly above `current_version` with
/// [`FormatError::UnsupportedSchemaVersion`], as well as any gap in `steps`.
pub fn upgrade(
    mut value: serde_json::Value,
    from_version: u32,
    first_supported: u32,
    current_version: u32,
    steps: &[VersionStep],
) -> Result<serde_json::Value, FormatError> {
    if from_version < first_supported || from_version > current_version {
        return Err(FormatError::UnsupportedSchemaVersion { version: from_version, max_supported: current_version });
    }

    for step_ver in from_version..current_version {
        let Some(step) = steps.iter().find(|candidate| candidate.from_version == step_ver) else {
            return Err(FormatError::UnsupportedSchemaVersion { version: step_ver, max_supported: current_version });
        };
        (step.apply)(&mut value)?;
    }

    Ok(value)
}

const PREFERENCES_MIGRATION_STEPS: &[VersionStep] = &[VersionStep { from_version: 0, apply: migrate_preferences_v0_to_v1 }];

const _: () = assert!(PREFERENCES_MIGRATION_STEPS.len() == CURRENT_PREFERENCES_VERSION as usize);

fn default_preferences_version() -> u32 {
    CURRENT_PREFERENCES_VERSION
}

/// Persisted user preferences payload (`preferences.json`) with forward-safe schema versioning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreferencesFile {
    /// Monotonically increasing preferences format version.
    #[serde(default = "default_preferences_version")]
    pub version: u32,
    /// Preferred BCP-47 locale tag, if explicitly overridden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Preferred UI theme name, if explicitly overridden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// User-customized command shortcut bindings keyed by action identifier.
    #[serde(default)]
    pub shortcuts: BTreeMap<String, String>,
    /// Preserved forward-compatible extension properties not modeled in the typed fields above.
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for PreferencesFile {
    fn default() -> Self {
        Self { version: CURRENT_PREFERENCES_VERSION, locale: None, theme: None, shortcuts: BTreeMap::new(), extra: serde_json::Map::new() }
    }
}

fn migrate_preferences_v0_to_v1(value: &mut serde_json::Value) -> Result<(), FormatError> {
    let Some(obj) = value.as_object_mut() else {
        return Err(FormatError::Json(String::from("preferences root payload must be a JSON object")));
    };
    obj.insert(String::from("version"), serde_json::Value::from(CURRENT_PREFERENCES_VERSION));
    Ok(())
}

fn extract_preferences_version(obj: &serde_json::Map<String, serde_json::Value>) -> Result<u32, FormatError> {
    let Some(ver_val) = obj.get("version") else {
        return Ok(0);
    };
    let Some(ver_u64) = ver_val.as_u64() else {
        return Err(FormatError::Json(String::from("preferences `version` must be a non-negative integer")));
    };
    Ok(u32::try_from(ver_u64).unwrap_or(u32::MAX))
}

/// Parse and migrate a JSON string containing a [`PreferencesFile`].
///
/// Unversioned objects (`"version"` omitted) are treated as v0 and migrated forward to v1.
/// Payloads declaring `version > CURRENT_PREFERENCES_VERSION` are rejected with
/// [`FormatError::UnsupportedSchemaVersion`].
pub fn parse_preferences_json(raw: &str) -> Result<PreferencesFile, FormatError> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let Some(obj) = value.as_object() else {
        return Err(FormatError::Json(String::from("preferences root payload must be a JSON object")));
    };
    let from_version = extract_preferences_version(obj)?;
    let upgraded = upgrade(value, from_version, 0, CURRENT_PREFERENCES_VERSION, PREFERENCES_MIGRATION_STEPS)?;
    let prefs: PreferencesFile = serde_json::from_value(upgraded)?;
    if prefs.version > CURRENT_PREFERENCES_VERSION || prefs.version == 0 {
        return Err(FormatError::UnsupportedSchemaVersion { version: prefs.version, max_supported: CURRENT_PREFERENCES_VERSION });
    }
    Ok(prefs)
}

/// Load and migrate a [`PreferencesFile`] from disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_preferences_file(path: &Path) -> Result<PreferencesFile, FormatError> {
    let raw = lumencraft_platform::fs::read_regular_file_utf8(path, MAX_PREFERENCES_FILE_BYTES)?;
    parse_preferences_json(&raw)
}

/// Host filesystem preferences loading is unsupported on `wasm32` targets.
#[cfg(target_arch = "wasm32")]
pub fn load_preferences_file(_path: &Path) -> Result<PreferencesFile, FormatError> {
    Err(FormatError::Io(String::from("preferences file I/O is not supported on wasm32")))
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_existing_preferences_not_newer(path: &Path) -> Result<(), FormatError> {
    let Some(bytes) = lumencraft_platform::fs::read_regular_file_bytes_if_exists(path, MAX_PREFERENCES_FILE_BYTES)? else {
        return Ok(());
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(());
    };
    let Some(obj) = value.as_object() else {
        return Ok(());
    };
    let existing_version = extract_preferences_version(obj)?;
    if existing_version > CURRENT_PREFERENCES_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: existing_version, max_supported: CURRENT_PREFERENCES_VERSION });
    }
    Ok(())
}

/// Atomically persist a [`PreferencesFile`] to disk.
///
/// Refuses to overwrite an existing on-disk preferences file whose `version` exceeds
/// [`CURRENT_PREFERENCES_VERSION`], returning [`FormatError::UnsupportedSchemaVersion`].
#[cfg(not(target_arch = "wasm32"))]
pub fn save_preferences_file(path: &Path, prefs: &PreferencesFile) -> Result<(), FormatError> {
    if prefs.version > CURRENT_PREFERENCES_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: prefs.version, max_supported: CURRENT_PREFERENCES_VERSION });
    }
    ensure_existing_preferences_not_newer(path)?;

    let mut normalized = prefs.clone();
    if normalized.version == 0 {
        normalized.version = CURRENT_PREFERENCES_VERSION;
    }
    let bytes = serde_json::to_vec_pretty(&normalized)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    lumencraft_platform::fs::write_atomic(path, &bytes)?;
    Ok(())
}

/// Host filesystem preferences persistence is unsupported on `wasm32` targets.
#[cfg(target_arch = "wasm32")]
pub fn save_preferences_file(_path: &Path, _prefs: &PreferencesFile) -> Result<(), FormatError> {
    Err(FormatError::Io(String::from("preferences file I/O is not supported on wasm32")))
}

/// Validate that a batch manifest JSON object declares `"schema": "lumencraft.batch.v1"`.
pub fn validate_batch_manifest_version(value: &serde_json::Value) -> Result<(), FormatError> {
    let Some(obj) = value.as_object() else {
        return Err(FormatError::Json(String::from("batch manifest root payload must be a JSON object")));
    };
    let Some(schema_val) = obj.get("schema") else {
        return Err(FormatError::UnsupportedFormat { reason: format!("missing required batch `schema` (expected `{BATCH_SCHEMA_V1}`)") });
    };
    let Some(schema_str) = schema_val.as_str() else {
        return Err(FormatError::UnsupportedFormat { reason: format!("batch `schema` must be string `{BATCH_SCHEMA_V1}`") });
    };
    if schema_str == BATCH_SCHEMA_V1 {
        return Ok(());
    }
    if let Some(ver_suffix) = schema_str.strip_prefix("lumencraft.batch.v")
        && let Ok(version) = ver_suffix.parse::<u32>()
    {
        return Err(FormatError::UnsupportedSchemaVersion { version, max_supported: 1 });
    }
    Err(FormatError::UnsupportedFormat { reason: format!("unsupported batch manifest schema `{schema_str}` (expected `{BATCH_SCHEMA_V1}`)") })
}
