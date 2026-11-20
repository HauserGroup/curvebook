//! Shared versioned-file migration framework, preferences persistence, and batch schema validation.

use std::collections::BTreeMap;
use std::path::Path;

use curvebook_doc::param_spellings::{
    canonical_batch_command_key, canonical_batch_export_key, canonical_batch_input_key, canonical_command_param_alias,
};
use serde::{Deserialize, Serialize};

use crate::error::FormatError;

/// Current persisted version of user preferences files.
pub const CURRENT_PREFERENCES_VERSION: u32 = 1;

/// Maximum byte length permitted when reading a user preferences JSON file from disk.
pub const MAX_PREFERENCES_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Expected `schema` identifier for v1 headless batch manifests.
pub const BATCH_SCHEMA_V1: &str = "curvebook.batch.v1";

const LEGACY_BATCH_SCHEMA_PREFIX: &str = "lumencraft.batch.v";

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

fn deserialize_null_default_shortcuts<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<BTreeMap<String, String>>::deserialize(deserializer)?.unwrap_or_default())
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
    #[serde(default, deserialize_with = "deserialize_null_default_shortcuts")]
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
    let raw = curvebook_platform::fs::read_regular_file_utf8(path, MAX_PREFERENCES_FILE_BYTES)?;
    parse_preferences_json(&raw)
}

/// Host filesystem preferences loading is unsupported on `wasm32` targets.
#[cfg(target_arch = "wasm32")]
pub fn load_preferences_file(_path: &Path) -> Result<PreferencesFile, FormatError> {
    Err(FormatError::Io(String::from("preferences file I/O is not supported on wasm32")))
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_existing_preferences_not_newer(path: &Path) -> Result<(), FormatError> {
    let Some(bytes) = curvebook_platform::fs::read_regular_file_bytes_if_exists(path, MAX_PREFERENCES_FILE_BYTES)? else {
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
    curvebook_platform::fs::write_atomic(path, &bytes)?;
    Ok(())
}

/// Host filesystem preferences persistence is unsupported on `wasm32` targets.
#[cfg(target_arch = "wasm32")]
pub fn save_preferences_file(_path: &Path, _prefs: &PreferencesFile) -> Result<(), FormatError> {
    Err(FormatError::Io(String::from("preferences file I/O is not supported on wasm32")))
}

/// Validate that a batch manifest JSON object declares `"schema": "curvebook.batch.v1"`.
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
    if schema_str.starts_with(LEGACY_BATCH_SCHEMA_PREFIX) {
        return Err(FormatError::UnsupportedFormat {
            reason: format!("legacy LumenCraft batch schema `{schema_str}` is not supported; expected `{BATCH_SCHEMA_V1}`"),
        });
    }
    if let Some(ver_suffix) = schema_str.strip_prefix("curvebook.batch.v")
        && !ver_suffix.is_empty()
        && ver_suffix.bytes().all(|b| b.is_ascii_digit())
        && (ver_suffix == "0" || !ver_suffix.starts_with('0'))
        && let Ok(version) = ver_suffix.parse::<u32>()
        && version != 1
    {
        return Err(FormatError::UnsupportedSchemaVersion { version, max_supported: 1 });
    }
    Err(FormatError::UnsupportedFormat { reason: format!("unsupported batch manifest schema `{schema_str}` (expected `{BATCH_SCHEMA_V1}`)") })
}

const JOURNAL_MIGRATION_STEPS: &[VersionStep] = &[VersionStep { from_version: 1, apply: migrate_journal_command_v1_to_v2 }];

fn extract_object_kind(obj: &serde_json::Map<String, serde_json::Value>) -> Option<&str> {
    obj.get("kind").and_then(serde_json::Value::as_str)
}

fn rename_key_if_present(obj: &mut serde_json::Map<String, serde_json::Value>, old_key: &str, canonical_key: &str) -> Option<(String, String)> {
    if old_key == canonical_key || !obj.contains_key(old_key) {
        return None;
    }
    let moved = obj.remove(old_key);
    if !obj.contains_key(canonical_key)
        && let Some(val) = moved
    {
        obj.insert(canonical_key.to_owned(), val);
    }
    Some((old_key.to_owned(), canonical_key.to_owned()))
}

fn normalize_command_params_recursive(
    command_id: &str,
    path: &mut Vec<String>,
    kind: Option<&str>,
    val: &mut serde_json::Value,
    hits: &mut Vec<(String, String)>,
) {
    match val {
        serde_json::Value::Object(obj) => {
            let path_refs: Vec<&str> = path.iter().map(String::as_str).collect();
            // Upgrade any retired `"kind"` discriminant alias (e.g. `graph.addAnnotation`
            // `"type" -> "kind"`) before reading `local_kind`, so sibling kind-dependent aliases
            // (such as `"color" -> "stroke_color"` on `shape` annotations) resolve in one pass.
            let pre_keys: Vec<String> = obj.keys().cloned().collect();
            for key in pre_keys {
                if canonical_command_param_alias(command_id, &path_refs, kind, &key) == Some("kind")
                    && let Some(pair) = rename_key_if_present(obj, &key, "kind")
                {
                    hits.push(pair);
                }
            }
            let local_kind = extract_object_kind(obj).map(str::to_owned);
            let eff_kind = local_kind.as_deref().or(kind);
            let keys: Vec<String> = obj.keys().cloned().collect();
            for key in keys {
                if let Some(canonical) = canonical_command_param_alias(command_id, &path_refs, eff_kind, &key)
                    && let Some(pair) = rename_key_if_present(obj, &key, canonical)
                {
                    hits.push(pair);
                }
            }
            let next_keys: Vec<String> = obj.keys().cloned().collect();
            for child_key in next_keys {
                if let Some(child_val) = obj.get_mut(&child_key) {
                    path.push(child_key);
                    normalize_command_params_recursive(command_id, path, eff_kind, child_val, hits);
                    path.pop();
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_command_params_recursive(command_id, path, kind, item, hits);
            }
        }
        _ => {}
    }
}

/// Normalize deprecated parameter spellings in a command's `params` JSON object in place,
/// returning `(deprecated_key, canonical_key)` replacements applied.
pub fn normalize_command_params_in_place(command_id: &str, params: &mut serde_json::Value) -> Vec<(String, String)> {
    let mut hits = Vec::new();
    let mut path = Vec::new();
    normalize_command_params_recursive(command_id, &mut path, None, params, &mut hits);
    hits
}

fn migrate_journal_command_v1_to_v2(command_json: &mut serde_json::Value) -> Result<(), FormatError> {
    let Some(obj) = command_json.as_object_mut() else {
        return Ok(());
    };
    // Only upgrade CurveBook command envelopes (`{ "command_id" | "id" | "command": "<area>.<cmd>", "params": ... }`);
    // arbitrary non-command payloads stored by recovery unit tests stay untouched.
    let cmd_str = obj
        .get("command_id")
        .or_else(|| obj.get("command"))
        .or_else(|| obj.get("id"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| s.contains('.'))
        .map(str::to_owned);
    let Some(command_id) = cmd_str else {
        return Ok(());
    };
    if !obj.contains_key("command_id") {
        if obj.get("command").and_then(serde_json::Value::as_str).is_some() {
            let _ = rename_key_if_present(obj, "command", "command_id");
        } else if obj.get("id").and_then(serde_json::Value::as_str).is_some() {
            let _ = rename_key_if_present(obj, "id", "command_id");
        }
    }
    if let Some(params) = obj.get_mut("params") {
        let _ = normalize_command_params_in_place(&command_id, params);
    }
    Ok(())
}

/// Upgrade a CRC-verified `ops.journal` `command_json` payload from `from_version` to `to_version`.
pub(crate) fn upgrade_journal_command_json(value: serde_json::Value, from_version: u32, to_version: u32) -> Result<serde_json::Value, FormatError> {
    upgrade(value, from_version, 1, to_version, JOURNAL_MIGRATION_STEPS)
}

fn normalize_manifest_section(
    arr: Option<&mut serde_json::Value>,
    section_name: &str,
    resolver: fn(&str) -> Option<&'static str>,
    warnings: &mut Vec<String>,
) {
    let Some(items) = arr.and_then(serde_json::Value::as_array_mut) else {
        return;
    };
    for (idx, item) in items.iter_mut().enumerate() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let keys: Vec<String> = obj.keys().cloned().collect();
        for key in keys {
            if let Some(canonical) = resolver(&key)
                && let Some((old, canon)) = rename_key_if_present(obj, &key, canonical)
            {
                warnings.push(format!("batch manifest `{section_name}[{idx}].{old}` is deprecated; use `{canon}` instead"));
            }
        }
    }
}

/// Upgrade deprecated section keys and command parameter aliases inside a `curvebook.batch.v1`
/// manifest in place, returning human-readable deprecation notices for any upgraded fields.
pub fn normalize_batch_manifest_v1_with_warnings(manifest: &mut serde_json::Value) -> Result<Vec<String>, FormatError> {
    validate_batch_manifest_version(manifest)?;
    let Some(root) = manifest.as_object_mut() else {
        return Err(FormatError::Json(String::from("batch manifest root payload must be a JSON object")));
    };

    let mut warnings = Vec::new();
    normalize_manifest_section(root.get_mut("inputs"), "inputs", canonical_batch_input_key, &mut warnings);
    normalize_manifest_section(root.get_mut("exports"), "exports", canonical_batch_export_key, &mut warnings);
    normalize_manifest_section(root.get_mut("commands"), "commands", canonical_batch_command_key, &mut warnings);

    if let Some(commands) = root.get_mut("commands").and_then(serde_json::Value::as_array_mut) {
        for (idx, entry) in commands.iter_mut().enumerate() {
            let Some(obj) = entry.as_object_mut() else {
                continue;
            };
            let Some(cmd_id) = obj.get("cmd").and_then(serde_json::Value::as_str).map(str::to_owned) else {
                continue;
            };
            if let Some(params) = obj.get_mut("params") {
                for (old, canon) in normalize_command_params_in_place(&cmd_id, params) {
                    warnings.push(format!("batch manifest `commands[{idx}]` (`{cmd_id}`) parameter `{old}` is deprecated; use `{canon}` instead"));
                }
            }
        }
    }

    Ok(warnings)
}

/// Upgrade deprecated section keys and command parameter aliases inside a `curvebook.batch.v1`
/// manifest in place without returning deprecation notices.
pub fn normalize_batch_manifest_v1(manifest: &mut serde_json::Value) -> Result<(), FormatError> {
    normalize_batch_manifest_v1_with_warnings(manifest).map(|_| ())
}
