//! Versioned `.cbplate` microplate layout persistence (`R17.7`).
//!
//! Uses a dedicated single extension (`.cbplate`) distinct from CurveBook ZIP project templates
//! (`.cbtemplate`) and JSON result exports (`.json`), with an explicit `format_version`,
//! `VersionStep` migration support, forward-version refusal (`UnsupportedSchemaVersion`), and
//! atomic writes via `curvebook-platform`.

use std::fmt;
use std::path::Path;

use curvebook_doc::{DocError, MAX_PLATE_WELLS, PlateGeometry, PlateWellRole, validate_title};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::error::FormatError;
use crate::versioned::{VersionStep, upgrade};

/// Canonical single file extension for versioned CurveBook microplate layout files.
pub const PLATE_MAP_EXTENSION: &str = "cbplate";

/// Current schema version of `.cbplate` microplate layout files.
pub const CURRENT_PLATE_MAP_VERSION: u32 = 1;

/// Maximum byte length permitted when reading a `.cbplate` file from disk or inline text.
pub const MAX_PLATE_MAP_FILE_BYTES: usize = 4 * 1024 * 1024;

const PLATE_MAP_MIGRATION_STEPS: &[VersionStep] = &[VersionStep { from_version: 0, apply: migrate_plate_map_v0_to_v1 }];

const _: () = assert!(PLATE_MAP_MIGRATION_STEPS.len() == CURRENT_PLATE_MAP_VERSION as usize);

const fn default_plate_map_version() -> u32 {
    CURRENT_PLATE_MAP_VERSION
}

fn deserialize_bounded_plate_wells<'de, D>(deserializer: D) -> Result<Vec<PlateWellRole>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct BoundedPlateWellsVisitor;
    impl<'de> Visitor<'de> for BoundedPlateWellsVisitor {
        type Value = Vec<PlateWellRole>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a sequence of at most {MAX_PLATE_WELLS} plate well roles")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let cap = seq.size_hint().unwrap_or(0).min(MAX_PLATE_WELLS);
            let mut out = Vec::with_capacity(cap);
            while let Some(role) = seq.next_element::<PlateWellRole>()? {
                if out.len() >= MAX_PLATE_WELLS {
                    return Err(de::Error::custom(format!("plate layout `wells` exceeds maximum length {MAX_PLATE_WELLS}")));
                }
                out.push(role);
            }
            Ok(out)
        }
    }
    deserializer.deserialize_seq(BoundedPlateWellsVisitor)
}

/// Versioned `.cbplate` microplate layout document (`format_version = 1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlateMapFile {
    /// Monotonically increasing `.cbplate` schema version.
    #[serde(default = "default_plate_map_version")]
    pub format_version: u32,
    /// Human-readable plate layout name.
    pub name: String,
    /// Physical microplate geometry (`Wells96` or `Wells384`).
    pub geometry: PlateGeometry,
    /// Independent *X* / concentration column title for mapped `Xy` / `Grouped` outputs.
    pub x_title: String,
    /// Row-major (`A01`, `A02`, ...) well role assignments (`len() == geometry.well_count()`).
    #[serde(deserialize_with = "deserialize_bounded_plate_wells")]
    pub wells: Vec<PlateWellRole>,
}

impl PlateMapFile {
    /// Construct a validated v1 [`PlateMapFile`].
    pub fn new(name: impl Into<String>, geometry: PlateGeometry, x_title: impl Into<String>, wells: Vec<PlateWellRole>) -> Result<Self, FormatError> {
        let file = Self { format_version: CURRENT_PLATE_MAP_VERSION, name: name.into(), geometry, x_title: x_title.into(), wells };
        file.validate()?;
        Ok(file)
    }

    /// Validate version bounds, titles, well count, and finite numeric treatment parameters.
    pub fn validate(&self) -> Result<(), FormatError> {
        if self.format_version == 0 || self.format_version > CURRENT_PLATE_MAP_VERSION {
            return Err(FormatError::UnsupportedSchemaVersion { version: self.format_version, max_supported: CURRENT_PLATE_MAP_VERSION });
        }
        if self.name.trim().is_empty() {
            return Err(FormatError::Doc(DocError::EmptyIdentifier));
        }
        validate_title(&self.name)?;
        validate_title(&self.x_title)?;
        let expected = self.geometry.well_count();
        if self.wells.len() != expected {
            return Err(FormatError::InvalidPipeline {
                reason: format!("plate layout has {} well(s), expected {expected} for {:?}", self.wells.len(), self.geometry),
            });
        }
        for (idx, role) in self.wells.iter().enumerate() {
            if let PlateWellRole::Treatment { group_name, dilution_x, .. } = role {
                let label = self.geometry.label_for_index(idx).unwrap_or_else(|_| format!("#{idx}"));
                if group_name.trim().is_empty() {
                    return Err(FormatError::InvalidPipeline { reason: format!("treatment group_name at well {label} cannot be empty") });
                }
                validate_title(group_name)?;
                if !dilution_x.is_finite() {
                    return Err(FormatError::NonFiniteFloat { location: format!("wells[{label}].dilution_x") });
                }
            }
        }
        Ok(())
    }
}

fn migrate_plate_map_v0_to_v1(value: &mut serde_json::Value) -> Result<(), FormatError> {
    let Some(obj) = value.as_object_mut() else {
        return Err(FormatError::Json(String::from("plate layout root payload must be a JSON object")));
    };
    obj.insert(String::from("format_version"), serde_json::Value::from(CURRENT_PLATE_MAP_VERSION));
    Ok(())
}

fn extract_plate_map_version(obj: &serde_json::Map<String, serde_json::Value>) -> Result<u32, FormatError> {
    let Some(ver_val) = obj.get("format_version") else {
        return Ok(0);
    };
    let Some(ver_u64) = ver_val.as_u64() else {
        return Err(FormatError::Json(String::from("plate layout `format_version` must be a non-negative integer")));
    };
    Ok(u32::try_from(ver_u64).unwrap_or(u32::MAX))
}

/// Parse, migrate (`v0 -> v1`), and validate a `.cbplate` UTF-8 JSON string.
///
/// Legacy unversioned plate JSON objects (`format_version` omitted) migrate cleanly to `v1`;
/// payloads declaring `format_version > CURRENT_PLATE_MAP_VERSION` fail with
/// [`FormatError::UnsupportedSchemaVersion`].
pub fn parse_cbplate_json(raw: &str) -> Result<PlateMapFile, FormatError> {
    if raw.len() > MAX_PLATE_MAP_FILE_BYTES {
        return Err(FormatError::FileTooLarge {
            path: String::from(".cbplate"),
            actual_bytes: Some(raw.len() as u64),
            limit_bytes: MAX_PLATE_MAP_FILE_BYTES,
        });
    }
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let Some(obj) = value.as_object() else {
        return Err(FormatError::Json(String::from("plate layout root payload must be a JSON object")));
    };
    let from_version = extract_plate_map_version(obj)?;
    let upgraded = upgrade(value, from_version, 0, CURRENT_PLATE_MAP_VERSION, PLATE_MAP_MIGRATION_STEPS)?;
    let parsed: PlateMapFile = serde_json::from_value(upgraded)?;
    parsed.validate()?;
    Ok(parsed)
}

/// Serialize a validated [`PlateMapFile`] as versioned `.cbplate` UTF-8 JSON.
pub fn serialize_cbplate_json(file: &PlateMapFile, pretty: bool) -> Result<String, FormatError> {
    file.validate()?;
    let json = if pretty { serde_json::to_string_pretty(file)? } else { serde_json::to_string(file)? };
    Ok(json)
}

/// Load, migrate, and validate a `.cbplate` file from disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_cbplate_file(path: &Path) -> Result<PlateMapFile, FormatError> {
    let raw = curvebook_platform::fs::read_regular_file_utf8(path, MAX_PLATE_MAP_FILE_BYTES)?;
    parse_cbplate_json(&raw)
}

/// Host filesystem `.cbplate` loading is unsupported on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub fn load_cbplate_file(_path: &Path) -> Result<PlateMapFile, FormatError> {
    Err(FormatError::Io(String::from("plate layout file I/O is not supported on wasm32")))
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_existing_cbplate_not_newer(path: &Path) -> Result<(), FormatError> {
    let Some(bytes) = curvebook_platform::fs::read_regular_file_bytes_if_exists(path, MAX_PLATE_MAP_FILE_BYTES)? else {
        return Ok(());
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(());
    };
    let Some(obj) = value.as_object() else {
        return Ok(());
    };
    let existing = extract_plate_map_version(obj)?;
    if existing > CURRENT_PLATE_MAP_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: existing, max_supported: CURRENT_PLATE_MAP_VERSION });
    }
    Ok(())
}

/// Atomically persist a validated [`PlateMapFile`] to disk as `.cbplate`.
///
/// Refuses to save an unsupported future [`PlateMapFile`] or overwrite an existing file whose
/// `format_version` exceeds [`CURRENT_PLATE_MAP_VERSION`], returning
/// [`FormatError::UnsupportedSchemaVersion`].
#[cfg(not(target_arch = "wasm32"))]
pub fn save_cbplate_file(path: &Path, file: &PlateMapFile, pretty: bool) -> Result<(), FormatError> {
    if file.format_version > CURRENT_PLATE_MAP_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: file.format_version, max_supported: CURRENT_PLATE_MAP_VERSION });
    }
    ensure_existing_cbplate_not_newer(path)?;
    let mut normalized = file.clone();
    if normalized.format_version == 0 {
        normalized.format_version = CURRENT_PLATE_MAP_VERSION;
    }
    let json = serialize_cbplate_json(&normalized, pretty)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    curvebook_platform::fs::write_atomic(path, json.as_bytes())?;
    Ok(())
}

/// Host filesystem `.cbplate` saving is unsupported on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub fn save_cbplate_file(_path: &Path, _file: &PlateMapFile, _pretty: bool) -> Result<(), FormatError> {
    Err(FormatError::Io(String::from("plate layout file I/O is not supported on wasm32")))
}
