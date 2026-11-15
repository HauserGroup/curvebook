//! Native `.lumencraft` and `.lctemplate` ZIP archive serialization, deserialization,
//! schema migration, dimension/float validation, and circular dependency guards.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use lumencraft_doc::{
    AnalysisKind, CURRENT_SCHEMA_VERSION, Cell, CellValue, CiBound, CloneDataSheetOptions, ConstantValue, DataSetColumn, DataSheet, GraphAnnotation,
    GraphSheet, HookSource, InfoSheet, LayoutItem, LayoutSheet, MAX_DATA_SETS, MAX_ROWS, MAX_SHEETS_PER_PROJECT, MAX_SUBCOLUMNS, NormalizeAnchor,
    ParameterScope, ProjectDocument, ResultCell, ResultSheet, ResultStatus, SheetId, SheetSection, TableArchetype, TransformAnalysisSpec,
    extract_template_hooks, validate_data_sets, validate_rows, validate_subcolumns, validate_title,
};

use crate::csv::{CsvOptions, export_datasheet_to_csv};
use crate::error::FormatError;
use crate::versioned::{VersionStep, upgrade};
use crate::zip::{ZipArchive, ZipLimits, ZipWriter, sanitize_zip_path};

const MANIFEST_PATH: &str = "manifest.json";
const DOCUMENT_PATH: &str = "document.json";
const SHEETS_DIR: &str = "sheets";
const FORMAT_MAGIC: &str = "lumencraft-archive";

/// Maximum byte length permitted when reading a `.lumencraft`, `.lctemplate`, or `.prism` archive file.
pub const MAX_ARCHIVE_FILE_BYTES: usize = 128 * 1024 * 1024;

/// Kind of native LumenCraft ZIP archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ArchiveKind {
    Project,
    Template,
}

impl ArchiveKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Template => "template",
        }
    }
}

/// Index entry inside [`ArchiveManifest`] pointing to per-sheet human-inspectable payloads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SheetManifestEntry {
    pub id: SheetId,
    pub section: SheetSection,
    pub title: String,
    pub csv_path: Option<String>,
    pub crc32: Option<u32>,
}

/// Top-level `manifest.json` stored inside `.lumencraft` and `.lctemplate` archives.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArchiveManifest {
    pub format: String,
    pub schema_version: u32,
    pub kind: ArchiveKind,
    pub title: String,
    pub document_crc32: u32,
    pub sheets: Vec<SheetManifestEntry>,
}

/// Serialize a [`ProjectDocument`] into deterministic `.lumencraft` archive bytes.
pub fn save_project_bytes(doc: &ProjectDocument) -> Result<Vec<u8>, FormatError> {
    save_archive_bytes(doc, ArchiveKind::Project)
}

/// Serialize a [`ProjectDocument`] into deterministic `.lctemplate` archive bytes, stripping
/// project-specific raw data cell values and cached results while preserving schemas, specs,
/// graphs, layouts, and info constants.
pub fn save_template_bytes(doc: &ProjectDocument) -> Result<Vec<u8>, FormatError> {
    let templated = strip_project_data_for_template(doc)?;
    save_archive_bytes(&templated, ArchiveKind::Template)
}

/// Load and validate a `.lumencraft` project archive from raw bytes.
pub fn open_project_bytes(bytes: &[u8]) -> Result<ProjectDocument, FormatError> {
    let (kind, doc) = open_archive_bytes(bytes)?;
    if kind != ArchiveKind::Project {
        return Err(FormatError::InvalidArchiveKind { expected: ArchiveKind::Project.as_str(), found: kind.as_str() });
    }
    Ok(doc)
}

/// Load and validate a `.lctemplate` archive from raw bytes.
pub fn open_template_bytes(bytes: &[u8]) -> Result<ProjectDocument, FormatError> {
    let (kind, doc) = open_archive_bytes(bytes)?;
    if kind != ArchiveKind::Template {
        return Err(FormatError::InvalidArchiveKind { expected: ArchiveKind::Template.as_str(), found: kind.as_str() });
    }
    Ok(doc)
}

/// Load a `.lctemplate` (or `.lumencraft`) archive and instantiate a clean [`ProjectDocument`]
/// ready for new experimental data, optionally assigning a new project title.
pub fn instantiate_template_bytes(bytes: &[u8], new_title: Option<&str>) -> Result<ProjectDocument, FormatError> {
    let (_, raw_doc) = open_archive_bytes(bytes)?;
    let mut doc = strip_project_data_for_template(&raw_doc)?;
    if let Some(t) = new_title {
        validate_title(t)?;
        doc.title = t.to_owned();
    }
    Ok(doc)
}

/// Load any native LumenCraft archive (`.lumencraft` or `.lctemplate`), returning its [`ArchiveKind`]
/// and validated, schema-migrated [`ProjectDocument`].
pub fn open_archive_bytes(bytes: &[u8]) -> Result<(ArchiveKind, ProjectDocument), FormatError> {
    open_archive_bytes_with_limits(bytes, ZipLimits::default())
}

/// Load a native archive with explicit [`ZipLimits`] for security testing.
pub fn open_archive_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<(ArchiveKind, ProjectDocument), FormatError> {
    let archive = ZipArchive::from_bytes_with_limits(bytes, limits)?;
    let manifest_text = archive.get_utf8(MANIFEST_PATH)?.ok_or_else(|| FormatError::MissingArchiveEntry { path: String::from(MANIFEST_PATH) })?;
    let manifest = parse_and_validate_manifest(manifest_text)?;

    let doc_bytes = archive.get(DOCUMENT_PATH).ok_or_else(|| FormatError::MissingArchiveEntry { path: String::from(DOCUMENT_PATH) })?;
    let actual_doc_crc = crc32fast::hash(doc_bytes);
    if actual_doc_crc != manifest.document_crc32 {
        return Err(FormatError::CorruptCrc { entry: String::from(DOCUMENT_PATH), expected: manifest.document_crc32, actual: actual_doc_crc });
    }

    for entry in &manifest.sheets {
        if let (Some(csv_path), Some(expected_crc)) = (&entry.csv_path, entry.crc32) {
            let csv_bytes = archive.get(csv_path).ok_or_else(|| FormatError::MissingArchiveEntry { path: csv_path.clone() })?;
            let actual_crc = crc32fast::hash(csv_bytes);
            if actual_crc != expected_crc {
                return Err(FormatError::CorruptCrc { entry: csv_path.clone(), expected: expected_crc, actual: actual_crc });
            }
        }
    }

    let doc = decode_and_validate_document(doc_bytes, manifest.schema_version)?;
    Ok((manifest.kind, doc))
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FormatError> {
    lumencraft_platform::fs::write_atomic(path, bytes)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn platform_write_atomic(_path: &Path, _bytes: &[u8]) -> Result<(), FormatError> {
    Err(FormatError::Io(String::from("atomic file writes are not supported on wasm32")))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn platform_read_regular_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, FormatError> {
    Ok(lumencraft_platform::fs::read_regular_file_bytes(path, max_bytes)?)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn platform_read_regular_file(_path: &Path, _max_bytes: usize) -> Result<Vec<u8>, FormatError> {
    Err(FormatError::Io(String::from("filesystem reads are not supported on wasm32")))
}

/// Write a `.lumencraft` archive to disk.
pub fn save_project_file(path: &Path, doc: &ProjectDocument) -> Result<(), FormatError> {
    let bytes = save_project_bytes(doc)?;
    platform_write_atomic(path, &bytes)?;
    Ok(())
}

/// Read a `.lumencraft` archive from disk.
pub fn open_project_file(path: &Path) -> Result<ProjectDocument, FormatError> {
    let bytes = platform_read_regular_file(path, MAX_ARCHIVE_FILE_BYTES)?;
    open_project_bytes(&bytes)
}

/// Write a `.lctemplate` archive to disk.
pub fn save_template_file(path: &Path, doc: &ProjectDocument) -> Result<(), FormatError> {
    let bytes = save_template_bytes(doc)?;
    platform_write_atomic(path, &bytes)?;
    Ok(())
}

/// Read a `.lctemplate` archive from disk.
pub fn open_template_file(path: &Path) -> Result<ProjectDocument, FormatError> {
    let bytes = platform_read_regular_file(path, MAX_ARCHIVE_FILE_BYTES)?;
    open_template_bytes(&bytes)
}

/// Return `true` when `path` is an existing directory containing a valid unpacked `.lumencraft.d/`
/// project manifest and document payload (rejecting template archives and escaping symlinks).
#[must_use]
pub fn is_unpacked_project_dir(path: &Path) -> bool {
    check_unpacked_project_dir(path).unwrap_or(false)
}

/// Write `doc` to an unpacked, Git-friendly `.lumencraft.d/` project directory using the exact
/// deterministic `.lumencraft` layout (`manifest.json`, canonical 2-space `document.json`, and
/// RFC-4180 `sheets/<id>.csv` CSVs), pruning stale sheet files without disturbing `.git` or other
/// user files and rejecting any relative-path escapes or symlinks resolving outside `dir`.
pub fn save_project_unpacked_dir(doc: &ProjectDocument, dir: &Path) -> Result<(), FormatError> {
    let entries = build_archive_entries(doc, ArchiveKind::Project)?;
    ensure_dir_exists(dir)?;
    let canon_root = std::fs::canonicalize(dir)?;
    let prior_paths = preflight_unpacked_save(&canon_root, &entries)?;

    let mut expected_paths = HashSet::with_capacity(entries.len());
    for (rel, bytes) in &entries {
        expected_paths.insert(rel.clone());
        write_file_atomically(&canon_root, rel, bytes)?;
    }

    prune_stale_unpacked_files(&canon_root, &prior_paths, &expected_paths)
}

/// Read and validate an unpacked `.lumencraft.d/` project directory, enforcing strict relative-path
/// containment, symlink boundaries, CRC-32 verification, size caps, and schema migration.
pub fn open_project_unpacked_dir(dir: &Path) -> Result<ProjectDocument, FormatError> {
    if !std::fs::metadata(dir)?.is_dir() {
        return Err(FormatError::Io(format!("`{}` is not a directory", dir.display())));
    }
    let canon_root = std::fs::canonicalize(dir)?;
    let limits = ZipLimits::default();
    let mut remaining_budget = limits.max_total_uncompressed_bytes;

    let manifest_bytes = read_contained_file(&canon_root, MANIFEST_PATH, limits.max_entry_uncompressed_bytes, &mut remaining_budget)?;
    let manifest_text = std::str::from_utf8(&manifest_bytes)
        .map_err(|err| FormatError::InvalidZip { reason: format!("entry `{MANIFEST_PATH}` is not valid UTF-8: {err}") })?;
    let manifest = parse_and_validate_manifest(manifest_text)?;
    if manifest.kind != ArchiveKind::Project {
        return Err(FormatError::InvalidArchiveKind { expected: ArchiveKind::Project.as_str(), found: manifest.kind.as_str() });
    }

    let doc_bytes = read_contained_file(&canon_root, DOCUMENT_PATH, limits.max_entry_uncompressed_bytes, &mut remaining_budget)?;
    let actual_doc_crc = crc32fast::hash(&doc_bytes);
    if actual_doc_crc != manifest.document_crc32 {
        return Err(FormatError::CorruptCrc { entry: String::from(DOCUMENT_PATH), expected: manifest.document_crc32, actual: actual_doc_crc });
    }

    verify_unpacked_sheet_entries(&canon_root, &manifest, limits, &mut remaining_budget)?;
    decode_and_validate_document(&doc_bytes, manifest.schema_version)
}

fn check_unpacked_project_dir(path: &Path) -> Result<bool, FormatError> {
    if !std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
        return Ok(false);
    }
    let canon_root = std::fs::canonicalize(path)?;
    let mut remaining_budget = crate::zip::MAX_ZIP_UNCOMPRESSED_BYTES;
    let manifest_bytes = read_contained_file(&canon_root, MANIFEST_PATH, crate::zip::MAX_ZIP_ENTRY_BYTES, &mut remaining_budget)?;
    let manifest_text = std::str::from_utf8(&manifest_bytes)
        .map_err(|err| FormatError::InvalidZip { reason: format!("entry `{MANIFEST_PATH}` is not valid UTF-8: {err}") })?;
    let manifest = parse_and_validate_manifest(manifest_text)?;
    if manifest.kind != ArchiveKind::Project {
        return Ok(false);
    }
    let doc_path = resolve_contained_path(&canon_root, DOCUMENT_PATH, true)?;
    let doc_meta = std::fs::metadata(doc_path)?;
    Ok(doc_meta.is_file() && doc_meta.len() <= crate::zip::MAX_ZIP_ENTRY_BYTES)
}

fn parse_and_validate_manifest(manifest_text: &str) -> Result<ArchiveManifest, FormatError> {
    let manifest: ArchiveManifest = serde_json::from_str(manifest_text)?;
    if manifest.format != FORMAT_MAGIC {
        return Err(FormatError::UnsupportedFormat { reason: format!("unexpected archive format `{}`", manifest.format) });
    }
    if manifest.schema_version > CURRENT_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: manifest.schema_version, max_supported: CURRENT_SCHEMA_VERSION });
    }
    if manifest.sheets.len() > crate::zip::MAX_ZIP_ENTRIES {
        return Err(FormatError::ZipBomb {
            reason: format!("manifest sheet count {} exceeds limit {}", manifest.sheets.len(), crate::zip::MAX_ZIP_ENTRIES),
        });
    }
    for entry in &manifest.sheets {
        if let Some(csv_path) = &entry.csv_path {
            validate_unpacked_relative_path(csv_path)?;
        }
    }
    Ok(manifest)
}

const MAX_MIGRATION_JSON_NODES: usize = 1_000_000;
const MIN_MIGRATION_JSON_BYTES: usize = 1_048_576;
const MAX_MIGRATION_JSON_BYTES: usize = 64 * 1024 * 1024;

#[derive(serde::Deserialize)]
struct DocumentSchemaPeek {
    #[serde(default)]
    schema_version: Option<u32>,
}

struct JsonDomBudget {
    remaining_nodes: usize,
    remaining_bytes: usize,
}

impl JsonDomBudget {
    fn charge(&mut self, nodes: usize, bytes: usize) -> Result<(), String> {
        if nodes > self.remaining_nodes {
            return Err(format!("JSON DOM node count exceeds migration limit {MAX_MIGRATION_JSON_NODES}"));
        }
        if bytes > self.remaining_bytes {
            return Err(String::from("JSON DOM allocation exceeds migration memory budget"));
        }
        self.remaining_nodes -= nodes;
        self.remaining_bytes -= bytes;
        Ok(())
    }
}

struct BoundedValueSeed<'a>(&'a mut JsonDomBudget);

impl<'de> serde::de::DeserializeSeed<'de> for BoundedValueSeed<'_> {
    type Value = serde_json::Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(BoundedValueVisitor(self.0))
    }
}

struct BoundedValueVisitor<'a>(&'a mut JsonDomBudget);

impl<'de> serde::de::Visitor<'de> for BoundedValueVisitor<'_> {
    type Value = serde_json::Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("any valid bounded JSON value")
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
        Ok(serde_json::Value::Bool(v))
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
        Ok(serde_json::Value::from(v))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
        Ok(serde_json::Value::from(v))
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
        serde_json::Number::from_f64(v).map(serde_json::Value::Number).ok_or_else(|| serde::de::Error::custom("non-finite JSON number"))
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>().saturating_add(v.len())).map_err(serde::de::Error::custom)?;
        Ok(serde_json::Value::String(v.to_owned()))
    }

    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>().saturating_add(v.len())).map_err(serde::de::Error::custom)?;
        Ok(serde_json::Value::String(v))
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
        Ok(serde_json::Value::Null)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
        Ok(serde_json::Value::Null)
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        serde::de::DeserializeSeed::deserialize(BoundedValueSeed(self.0), deserializer)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
        let mut vec = Vec::new();
        while let Some(elem) = seq.next_element_seed(BoundedValueSeed(self.0))? {
            vec.push(elem);
        }
        Ok(serde_json::Value::Array(vec))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        self.0.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
        let mut obj = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            self.0.charge(0, key.len().saturating_add(64)).map_err(serde::de::Error::custom)?;
            let val = map.next_value_seed(BoundedValueSeed(self.0))?;
            obj.insert(key, val);
        }
        Ok(serde_json::Value::Object(obj))
    }
}

fn parse_bounded_json_value(doc_bytes: &[u8]) -> Result<serde_json::Value, FormatError> {
    let max_bytes = doc_bytes.len().saturating_mul(6).clamp(MIN_MIGRATION_JSON_BYTES, MAX_MIGRATION_JSON_BYTES);
    let mut budget = JsonDomBudget { remaining_nodes: MAX_MIGRATION_JSON_NODES, remaining_bytes: max_bytes };
    let mut de = serde_json::Deserializer::from_slice(doc_bytes);
    let val = serde::de::DeserializeSeed::deserialize(BoundedValueSeed(&mut budget), &mut de)?;
    de.end()?;
    Ok(val)
}

fn decode_and_validate_document(doc_bytes: &[u8], manifest_schema_version: u32) -> Result<ProjectDocument, FormatError> {
    let peek: DocumentSchemaPeek = serde_json::from_slice(doc_bytes)?;
    let doc_version = peek.schema_version.unwrap_or(manifest_schema_version);
    if doc_version > CURRENT_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: doc_version, max_supported: CURRENT_SCHEMA_VERSION });
    }
    let doc: ProjectDocument = if doc_version == CURRENT_SCHEMA_VERSION && peek.schema_version.is_some() {
        let mut parsed: ProjectDocument = serde_json::from_slice(doc_bytes)?;
        parsed.schema_version = CURRENT_SCHEMA_VERSION;
        parsed
    } else {
        let raw_value = parse_bounded_json_value(doc_bytes)?;
        let migrated_value = migrate_document_json(raw_value, doc_version)?;
        serde_json::from_value(migrated_value)?
    };
    validate_project_document(&doc)?;
    Ok(doc)
}

fn save_archive_bytes(doc: &ProjectDocument, kind: ArchiveKind) -> Result<Vec<u8>, FormatError> {
    let entries = build_archive_entries(doc, kind)?;
    let mut writer = ZipWriter::new();
    for (path, bytes) in entries {
        writer.add_file(&path, bytes)?;
    }
    writer.finish()
}

fn build_archive_entries(doc: &ProjectDocument, kind: ArchiveKind) -> Result<Vec<(String, Vec<u8>)>, FormatError> {
    validate_project_document(doc)?;

    let mut normalized = doc.clone();
    normalized.schema_version = CURRENT_SCHEMA_VERSION;
    let doc_json = serde_json::to_vec_pretty(&normalized)?;
    let doc_crc = crc32fast::hash(&doc_json);

    let mut entries = Vec::with_capacity(normalized.data_sheets.len() + 2);
    entries.push((String::from(DOCUMENT_PATH), doc_json));

    let mut manifest_sheets = Vec::with_capacity(normalized.total_sheet_count());
    let csv_opts = CsvOptions::csv();
    let mut used_paths = HashSet::new();

    for (idx, ds) in normalized.data_sheets.iter().enumerate() {
        let csv_path = unique_sheet_csv_path(idx, &ds.id, &mut used_paths);
        let csv_bytes = export_datasheet_to_csv(ds, &csv_opts)?.into_bytes();
        let crc = crc32fast::hash(&csv_bytes);
        entries.push((csv_path.clone(), csv_bytes));
        manifest_sheets.push(SheetManifestEntry {
            id: ds.id.clone(),
            section: SheetSection::Data,
            title: ds.title.clone(),
            csv_path: Some(csv_path),
            crc32: Some(crc),
        });
    }

    push_metadata_entries(&mut manifest_sheets, &normalized);

    let manifest = ArchiveManifest {
        format: String::from(FORMAT_MAGIC),
        schema_version: CURRENT_SCHEMA_VERSION,
        kind,
        title: normalized.title.clone(),
        document_crc32: doc_crc,
        sheets: manifest_sheets,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    entries.push((String::from(MANIFEST_PATH), manifest_json));
    Ok(entries)
}

fn ensure_dir_exists(dir: &Path) -> Result<(), FormatError> {
    match std::fs::symlink_metadata(dir) {
        Ok(_) => {
            if !std::fs::metadata(dir)?.is_dir() {
                return Err(FormatError::Io(format!("`{}` exists and is not a directory", dir.display())));
            }
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir)?;
            Ok(())
        }
        Err(err) => Err(FormatError::Io(err.to_string())),
    }
}

fn validate_unpacked_relative_path(raw: &str) -> Result<(), FormatError> {
    let clean = sanitize_zip_path(raw)?;
    if clean.ends_with('/') {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    Ok(())
}

fn check_symlink_contained(cur: &Path, canon_root: &Path, rel: &str) -> Result<(), FormatError> {
    let Ok(resolved) = std::fs::canonicalize(cur) else {
        return Err(FormatError::UnsafePath { path: rel.to_owned() });
    };
    if !resolved.starts_with(canon_root) {
        return Err(FormatError::UnsafePath { path: rel.to_owned() });
    }
    Ok(())
}

fn resolve_contained_path(canon_root: &Path, rel: &str, must_exist: bool) -> Result<PathBuf, FormatError> {
    validate_unpacked_relative_path(rel)?;
    let mut cur = canon_root.to_path_buf();
    let mut parts = rel.split('/').peekable();
    while let Some(seg) = parts.next() {
        let is_last = parts.peek().is_none();
        cur.push(seg);
        match std::fs::symlink_metadata(&cur) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    check_symlink_contained(&cur, canon_root, rel)?;
                }
                if !is_last && !std::fs::metadata(&cur).map(|m| m.is_dir()).unwrap_or(false) {
                    return Err(if must_exist {
                        FormatError::MissingArchiveEntry { path: rel.to_owned() }
                    } else {
                        FormatError::Io(format!("path component `{}` for `{rel}` is not a directory", cur.display()))
                    });
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if must_exist {
                    return Err(FormatError::MissingArchiveEntry { path: rel.to_owned() });
                }
                for rest in parts {
                    cur.push(rest);
                }
                return Ok(cur);
            }
            Err(err) => return Err(FormatError::Io(err.to_string())),
        }
    }
    if let Ok(resolved) = std::fs::canonicalize(&cur)
        && !resolved.starts_with(canon_root)
    {
        return Err(FormatError::UnsafePath { path: rel.to_owned() });
    }
    Ok(cur)
}

fn read_contained_file(canon_root: &Path, rel: &str, max_entry_bytes: u64, remaining_budget: &mut u64) -> Result<Vec<u8>, FormatError> {
    let resolved = resolve_contained_path(canon_root, rel, true)?;
    let entry_cap = max_entry_bytes.min(*remaining_budget);
    let cap_usize = usize::try_from(entry_cap).unwrap_or(usize::MAX);
    let buf = match platform_read_regular_file(&resolved, cap_usize) {
        Ok(bytes) => bytes,
        Err(FormatError::FileTooLarge { actual_bytes: Some(actual), .. }) => {
            return Err(FormatError::ZipBomb { reason: format!("entry `{rel}` size {actual} exceeds security limit {entry_cap}") });
        }
        Err(FormatError::FileTooLarge { actual_bytes: None, .. }) => {
            return Err(FormatError::ZipBomb { reason: format!("entry `{rel}` payload exceeds security limit") });
        }
        Err(err) => return Err(err),
    };
    *remaining_budget = remaining_budget.saturating_sub(buf.len() as u64);
    Ok(buf)
}

fn verify_unpacked_sheet_entries(
    canon_root: &Path,
    manifest: &ArchiveManifest,
    limits: ZipLimits,
    remaining_budget: &mut u64,
) -> Result<(), FormatError> {
    let mut seen_csvs = HashSet::new();
    let mut entry_count = 2_usize;
    for entry in &manifest.sheets {
        let Some(csv_path) = &entry.csv_path else {
            continue;
        };
        if !seen_csvs.insert(csv_path.as_str()) {
            return Err(FormatError::DuplicateEntry { path: csv_path.clone() });
        }
        entry_count += 1;
        if entry_count > limits.max_entries {
            return Err(FormatError::ZipBomb { reason: format!("unpacked entry count exceeds limit {}", limits.max_entries) });
        }
        let csv_bytes = read_contained_file(canon_root, csv_path, limits.max_entry_uncompressed_bytes, remaining_budget)?;
        if let Some(expected_crc) = entry.crc32 {
            let actual_crc = crc32fast::hash(&csv_bytes);
            if actual_crc != expected_crc {
                return Err(FormatError::CorruptCrc { entry: csv_path.clone(), expected: expected_crc, actual: actual_crc });
            }
        }
    }
    Ok(())
}

fn preflight_unpacked_save(canon_root: &Path, entries: &[(String, Vec<u8>)]) -> Result<Vec<String>, FormatError> {
    for (rel, _) in entries {
        resolve_contained_path(canon_root, rel, false)?;
    }
    let sheets_path = canon_root.join(SHEETS_DIR);
    if let Ok(meta) = std::fs::symlink_metadata(&sheets_path) {
        if meta.file_type().is_symlink() {
            check_symlink_contained(&sheets_path, canon_root, SHEETS_DIR)?;
        } else if meta.is_dir() {
            for entry in std::fs::read_dir(&sheets_path)? {
                let entry = entry?;
                if entry.file_type()?.is_symlink() {
                    let rel = format!("{SHEETS_DIR}/{}", entry.file_name().to_string_lossy());
                    check_symlink_contained(&entry.path(), canon_root, &rel)?;
                }
            }
        }
    }
    collect_prior_manifest_csv_paths(canon_root)
}

fn collect_prior_manifest_csv_paths(canon_root: &Path) -> Result<Vec<String>, FormatError> {
    let Ok(manifest_path) = resolve_contained_path(canon_root, MANIFEST_PATH, true) else {
        return Ok(Vec::new());
    };
    let cap = usize::try_from(crate::zip::MAX_ZIP_ENTRY_BYTES).unwrap_or(usize::MAX);
    let Ok(bytes) = platform_read_regular_file(&manifest_path, cap) else {
        return Ok(Vec::new());
    };
    let Ok(manifest) = serde_json::from_slice::<ArchiveManifest>(&bytes) else {
        return Ok(Vec::new());
    };
    Ok(manifest.sheets.into_iter().filter_map(|e| e.csv_path).collect())
}

fn write_file_atomically(canon_root: &Path, rel: &str, bytes: &[u8]) -> Result<(), FormatError> {
    let target = resolve_contained_path(canon_root, rel, false)?;
    let parent = target.parent().ok_or_else(|| FormatError::UnsafePath { path: rel.to_owned() })?;
    std::fs::create_dir_all(parent)?;
    let canon_parent = std::fs::canonicalize(parent)?;
    if !canon_parent.starts_with(canon_root) {
        return Err(FormatError::UnsafePath { path: rel.to_owned() });
    }
    if let Ok(meta) = std::fs::symlink_metadata(&target)
        && !meta.file_type().is_symlink()
        && meta.is_file()
        && meta.len() == bytes.len() as u64
        && let Ok(existing) = platform_read_regular_file(&target, bytes.len())
        && existing == bytes
    {
        return Ok(());
    }

    platform_write_atomic(&target, bytes)
}

fn prune_stale_unpacked_files(canon_root: &Path, prior_paths: &[String], expected_paths: &HashSet<String>) -> Result<(), FormatError> {
    for rel in prior_paths {
        if expected_paths.contains(rel) || validate_unpacked_relative_path(rel).is_err() {
            continue;
        }
        if let Ok(path) = resolve_contained_path(canon_root, rel, true)
            && let Ok(meta) = std::fs::symlink_metadata(&path)
            && !meta.file_type().is_symlink()
            && meta.is_file()
        {
            std::fs::remove_file(path)?;
        }
    }

    let sheets_dir = canon_root.join(SHEETS_DIR);
    if let Ok(meta) = std::fs::symlink_metadata(&sheets_dir)
        && !meta.file_type().is_symlink()
        && meta.is_dir()
    {
        for entry in std::fs::read_dir(&sheets_dir)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let rel = format!("{SHEETS_DIR}/{name}");
            if expected_paths.contains(&rel) {
                continue;
            }
            let ft = entry.file_type()?;
            if !ft.is_symlink() && ft.is_file() {
                std::fs::remove_file(entry.path())?;
            }
        }
        let has_sheet_outputs = expected_paths.iter().any(|p| p.starts_with(&format!("{SHEETS_DIR}/")));
        if !has_sheet_outputs {
            let _ = std::fs::remove_dir(&sheets_dir);
        }
    }
    Ok(())
}

fn push_metadata_entries(entries: &mut Vec<SheetManifestEntry>, doc: &ProjectDocument) {
    for info in &doc.info_sheets {
        entries.push(SheetManifestEntry { id: info.id.clone(), section: SheetSection::Info, title: info.title.clone(), csv_path: None, crc32: None });
    }
    for res in &doc.result_sheets {
        entries.push(SheetManifestEntry {
            id: res.id.clone(),
            section: SheetSection::Analyses,
            title: res.title().to_owned(),
            csv_path: None,
            crc32: None,
        });
    }
    for graph in &doc.graph_sheets {
        entries.push(SheetManifestEntry {
            id: graph.id.clone(),
            section: SheetSection::Graphs,
            title: graph.spec.title.clone(),
            csv_path: None,
            crc32: None,
        });
    }
    for layout in &doc.layout_sheets {
        entries.push(SheetManifestEntry {
            id: layout.id.clone(),
            section: SheetSection::Layouts,
            title: layout.title.clone(),
            csv_path: None,
            crc32: None,
        });
    }
}

fn unique_sheet_csv_path(idx: usize, id: &SheetId, used: &mut HashSet<String>) -> String {
    let sanitized: String = id.as_str().chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
    let stem = if sanitized.is_empty() { format!("sheet_{idx}") } else { sanitized };
    let mut candidate = format!("sheets/{stem}.csv");
    let mut suffix = 1_usize;
    while !used.insert(candidate.clone()) {
        candidate = format!("sheets/{stem}_{suffix}.csv");
        suffix += 1;
    }
    candidate
}

/// Strip raw cell values and cached result tables to produce a clean reusable `.lctemplate`.
pub fn strip_project_data_for_template(doc: &ProjectDocument) -> Result<ProjectDocument, FormatError> {
    validate_project_document(doc)?;
    let mut out = doc.clone();
    out.schema_version = CURRENT_SCHEMA_VERSION;
    for ds in &mut out.data_sheets {
        let was_frozen = ds.frozen;
        let notes = ds.floating_notes.clone();
        let linked = ds.linked_regions.clone();
        let opts = CloneDataSheetOptions {
            title: ds.title.clone(),
            keep_x_values: false,
            clear_y_values: true,
            clear_column_titles: false,
            clear_row_titles: false,
            override_replicates: None,
        };
        let mut clean = ds.clone_structure(ds.id.clone(), &opts)?;
        clean.frozen = was_frozen;
        clean.floating_notes = notes;
        clean.linked_regions = linked;
        *ds = clean;
    }
    for res in &mut out.result_sheets {
        res.status = ResultStatus::Dirty;
        res.analyzable_data = None;
        res.tabs.clear();
        res.scalars.clear();
        res.pairwise_comparisons.clear();
        res.fitted_curves.clear();
        res.checklist.clear();
    }
    Ok(out)
}

const MIGRATION_STEPS: &[VersionStep] = &[VersionStep { from_version: 0, apply: migrate_v0_to_v1 }];

const _: () = assert!(MIGRATION_STEPS.len() == CURRENT_SCHEMA_VERSION as usize);

/// Migrate a deserialized JSON [`ProjectDocument`] forward step-by-step from `from_version` to
/// [`CURRENT_SCHEMA_VERSION`], filling backwards-compatible optional fields and rejecting unsupported
/// future major schemas cleanly.
pub fn migrate_document_json(value: serde_json::Value, from_version: u32) -> Result<serde_json::Value, FormatError> {
    let mut upgraded = upgrade(value, from_version, 0, CURRENT_SCHEMA_VERSION, MIGRATION_STEPS)?;
    backfill_compatible_fields(&mut upgraded, from_version < CURRENT_SCHEMA_VERSION);
    Ok(upgraded)
}

fn migrate_v0_to_v1(value: &mut serde_json::Value) -> Result<(), FormatError> {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(String::from("schema_version"), serde_json::Value::from(1_u32));
        backfill_top_level_fields(obj);
    }
    Ok(())
}

fn backfill_top_level_fields(obj: &mut serde_json::Map<String, serde_json::Value>) {
    obj.entry(String::from("title")).or_insert_with(|| serde_json::Value::String(String::from("Untitled Project")));
    obj.entry(String::from("data_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("info_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("result_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("graph_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("layout_sheets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("active_sheet_id")).or_insert(serde_json::Value::Null);
    obj.entry(String::from("family_anchor_sheet_id")).or_insert(serde_json::Value::Null);
    obj.entry(String::from("checkpoints")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
}

fn backfill_compatible_fields(value: &mut serde_json::Value, bump_schema_version: bool) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if bump_schema_version {
        obj.insert(String::from("schema_version"), serde_json::Value::from(CURRENT_SCHEMA_VERSION));
    } else {
        obj.entry(String::from("schema_version")).or_insert_with(|| serde_json::Value::from(CURRENT_SCHEMA_VERSION));
    }
    backfill_top_level_fields(obj);

    if let Some(data_sheets) = obj.get_mut("data_sheets").and_then(serde_json::Value::as_array_mut) {
        for ds in data_sheets {
            backfill_data_sheet_defaults(ds);
        }
    }
    if let Some(info_sheets) = obj.get_mut("info_sheets").and_then(serde_json::Value::as_array_mut) {
        for info in info_sheets {
            backfill_info_sheet_defaults(info);
        }
    }
    if let Some(result_sheets) = obj.get_mut("result_sheets").and_then(serde_json::Value::as_array_mut) {
        for res in result_sheets {
            backfill_result_sheet_defaults(res);
        }
    }
    if let Some(graph_sheets) = obj.get_mut("graph_sheets").and_then(serde_json::Value::as_array_mut) {
        for gs in graph_sheets {
            backfill_graph_sheet_defaults(gs);
        }
    }
    if let Some(layout_sheets) = obj.get_mut("layout_sheets").and_then(serde_json::Value::as_array_mut) {
        for ls in layout_sheets {
            backfill_layout_sheet_defaults(ls);
        }
    }
}

fn backfill_info_sheet_defaults(info: &mut serde_json::Value) {
    let Some(obj) = info.as_object_mut() else {
        return;
    };
    for arr_key in ["linked_data_sheets", "constants", "floating_notes"] {
        obj.entry(String::from(arr_key)).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    }
    obj.entry(String::from("notes_markdown")).or_insert_with(|| serde_json::Value::String(String::new()));
    obj.entry(String::from("color_tag")).or_insert(serde_json::Value::Null);
}

fn backfill_layout_sheet_defaults(ls: &mut serde_json::Value) {
    let Some(obj) = ls.as_object_mut() else {
        return;
    };
    obj.entry(String::from("floating_notes")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("color_tag")).or_insert(serde_json::Value::Null);
}

fn backfill_data_sheet_defaults(ds: &mut serde_json::Value) {
    let Some(obj) = ds.as_object_mut() else {
        return;
    };
    obj.entry(String::from("frozen")).or_insert(serde_json::Value::Bool(false));
    obj.entry(String::from("info_sheet_id")).or_insert(serde_json::Value::Null);
    obj.entry(String::from("linked_regions")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("floating_notes")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("exclusion_notes")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    obj.entry(String::from("color_tag")).or_insert(serde_json::Value::Null);
}

fn backfill_result_sheet_defaults(res: &mut serde_json::Value) {
    let Some(res_obj) = res.as_object_mut() else {
        return;
    };
    res_obj.entry(String::from("status")).or_insert_with(|| serde_json::Value::String(String::from("UpToDate")));
    res_obj.entry(String::from("analyzable_data")).or_insert(serde_json::Value::Null);
    for arr_key in ["tabs", "scalars", "pairwise_comparisons", "fitted_curves", "checklist", "floating_notes"] {
        res_obj.entry(String::from(arr_key)).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    }
    res_obj.entry(String::from("color_tag")).or_insert(serde_json::Value::Null);
    let Some(analysis) = res_obj.get_mut("analysis").and_then(serde_json::Value::as_object_mut) else {
        return;
    };
    let Some(kind) = analysis.get_mut("kind").and_then(serde_json::Value::as_object_mut) else {
        return;
    };
    if let Some(fit) = kind.get_mut("NonlinearRegression").and_then(serde_json::Value::as_object_mut) {
        for opt_key in ["x_range_filter", "y_range_filter", "plot_x_min", "plot_x_max", "mv_y_column"] {
            fit.entry(String::from(opt_key)).or_insert(serde_json::Value::Null);
        }
        for vec_key in ["mv_group_columns", "mv_x_columns"] {
            fit.entry(String::from(vec_key)).or_insert_with(|| serde_json::Value::Array(Vec::new()));
        }
        for bool_key in ["plot_function_only", "homoscedasticity_test", "emit_residuals_tab", "emit_covariance_tab"] {
            fit.entry(String::from(bool_key)).or_insert(serde_json::Value::Bool(false));
        }
    }
    if let Some(two_grp) = kind.get_mut("TwoGroupComparison").and_then(serde_json::Value::as_object_mut) {
        two_grp.entry(String::from("mv_group_column")).or_insert(serde_json::Value::Null);
        two_grp.entry(String::from("mv_response_column")).or_insert(serde_json::Value::Null);
    }
    if let Some(surv) = kind.get_mut("Survival").and_then(serde_json::Value::as_object_mut)
        && let Some(km) = surv.get_mut("KaplanMeier").and_then(serde_json::Value::as_object_mut)
    {
        km.entry(String::from("mv_time_column")).or_insert(serde_json::Value::Null);
        km.entry(String::from("mv_status_column")).or_insert(serde_json::Value::Null);
        km.entry(String::from("mv_group_column")).or_insert(serde_json::Value::Null);
    }
}

fn backfill_graph_sheet_defaults(gs: &mut serde_json::Value) {
    let Some(gs_obj) = gs.as_object_mut() else {
        return;
    };
    gs_obj.entry(String::from("initialized")).or_insert(serde_json::Value::Bool(true));
    gs_obj.entry(String::from("floating_notes")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    gs_obj.entry(String::from("color_tag")).or_insert(serde_json::Value::Null);
    let Some(spec) = gs_obj.get_mut("spec").and_then(serde_json::Value::as_object_mut) else {
        return;
    };
    for axis_key in ["x_axis", "y_axis", "y2_axis"] {
        if let Some(axis) = spec.get_mut(axis_key).and_then(serde_json::Value::as_object_mut) {
            axis.entry(String::from("additional_ticks")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
        }
    }
}

/// Enforce all `lumencraft-doc` structural bounds, finite-float invariants, and DAG cycle absence.
pub fn validate_project_document(doc: &ProjectDocument) -> Result<(), FormatError> {
    validate_title(&doc.title)?;
    let total = doc.total_sheet_count();
    if total > MAX_SHEETS_PER_PROJECT {
        return Err(FormatError::Doc(lumencraft_doc::DocError::TooManySheets { count: total, max: MAX_SHEETS_PER_PROJECT }));
    }

    let mut seen_ids = HashSet::with_capacity(total);
    for ds in &doc.data_sheets {
        register_unique_sheet_id(&mut seen_ids, &ds.id)?;
        validate_data_sheet(ds)?;
        validate_archetype_extra_floats(&ds.archetype, ds.id.as_str())?;
        validate_floating_notes(&ds.floating_notes, ds.id.as_str())?;
    }
    for info in &doc.info_sheets {
        register_unique_sheet_id(&mut seen_ids, &info.id)?;
        validate_info_sheet(info)?;
        validate_floating_notes(&info.floating_notes, info.id.as_str())?;
    }
    for res in &doc.result_sheets {
        register_unique_sheet_id(&mut seen_ids, &res.id)?;
        validate_result_sheet(res)?;
        if let Some(data) = &res.analyzable_data {
            validate_archetype_extra_floats(&data.archetype, res.id.as_str())?;
            validate_floating_notes(&data.floating_notes, res.id.as_str())?;
        }
        validate_analysis_kind_floats(&res.analysis, res.id.as_str())?;
        validate_floating_notes(&res.floating_notes, res.id.as_str())?;
    }
    for graph in &doc.graph_sheets {
        register_unique_sheet_id(&mut seen_ids, &graph.id)?;
        if let Some(field) = graph.first_non_finite_float() {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.{field}", graph.id) });
        }
        graph.validate()?;
    }
    for layout in &doc.layout_sheets {
        register_unique_sheet_id(&mut seen_ids, &layout.id)?;
        if let Some(field) = layout.first_non_finite_float() {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.{field}", layout.id) });
        }
        layout.validate()?;
    }

    detect_dependency_cycles(doc)
}

fn check_finite(value: f64, location: impl FnOnce() -> String) -> Result<(), FormatError> {
    if value.is_finite() { Ok(()) } else { Err(FormatError::NonFiniteFloat { location: location() }) }
}

fn check_opt_finite(value: Option<f64>, location: impl FnOnce() -> String) -> Result<(), FormatError> {
    if let Some(v) = value { check_finite(v, location) } else { Ok(()) }
}

fn validate_floating_notes(notes: &[lumencraft_doc::FloatingNote], sheet_id: &str) -> Result<(), FormatError> {
    for (idx, note) in notes.iter().enumerate() {
        let b = note.bounds_pt;
        if !b.x.is_finite() || !b.y.is_finite() || !b.width.is_finite() || !b.height.is_finite() {
            return Err(FormatError::NonFiniteFloat { location: format!("{sheet_id}.floating_notes[{idx}].bounds_pt") });
        }
    }
    Ok(())
}

fn validate_archetype_extra_floats(arch: &lumencraft_doc::TableArchetype, ctx: &str) -> Result<(), FormatError> {
    if let lumencraft_doc::TableArchetype::Grouped(t) = arch
        && let Some(spacing) = &t.row_numeric_spacing
    {
        for (row, maybe_v) in spacing.iter().enumerate() {
            check_opt_finite(*maybe_v, || format!("{ctx}.row_numeric_spacing[{row}]"))?;
        }
    }
    Ok(())
}

fn validate_math_transform_preset(preset: &lumencraft_doc::MathTransformPreset, location_prefix: &str) -> Result<(), FormatError> {
    match preset {
        lumencraft_doc::MathTransformPreset::LinearScale { slope, intercept } => {
            check_finite(*slope, || format!("{location_prefix}.slope"))?;
            check_finite(*intercept, || format!("{location_prefix}.intercept"))?;
        }
        lumencraft_doc::MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor } => {
            check_finite(*molar_unit_factor, || format!("{location_prefix}.molar_unit_factor"))?;
        }
        lumencraft_doc::MathTransformPreset::Ln
        | lumencraft_doc::MathTransformPreset::Log10
        | lumencraft_doc::MathTransformPreset::Log2
        | lumencraft_doc::MathTransformPreset::Exp10
        | lumencraft_doc::MathTransformPreset::Exp
        | lumencraft_doc::MathTransformPreset::Reciprocal
        | lumencraft_doc::MathTransformPreset::Sqrt
        | lumencraft_doc::MathTransformPreset::Square
        | lumencraft_doc::MathTransformPreset::Logit
        | lumencraft_doc::MathTransformPreset::Probit
        | lumencraft_doc::MathTransformPreset::ArcsinSqrt => {}
    }
    Ok(())
}

fn validate_normalize_anchor(anchor: &lumencraft_doc::NormalizeAnchor, location: &str) -> Result<(), FormatError> {
    if let lumencraft_doc::NormalizeAnchor::FixedValue(v) = anchor {
        check_finite(*v, || location.to_owned())?;
    }
    Ok(())
}

fn validate_transform_floats(transform: &lumencraft_doc::TransformAnalysisSpec, sheet_id: &str) -> Result<(), FormatError> {
    match transform {
        lumencraft_doc::TransformAnalysisSpec::Math { x_transform, y_transform, .. } => {
            if let Some(p) = x_transform {
                validate_math_transform_preset(p, &format!("{sheet_id}.analysis.x_transform"))?;
            }
            if let Some(p) = y_transform {
                validate_math_transform_preset(p, &format!("{sheet_id}.analysis.y_transform"))?;
            }
        }
        lumencraft_doc::TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. } => {
            validate_normalize_anchor(zero_anchor, &format!("{sheet_id}.analysis.zero_anchor"))?;
            validate_normalize_anchor(hundred_anchor, &format!("{sheet_id}.analysis.hundred_anchor"))?;
        }
        lumencraft_doc::TransformAnalysisSpec::Simulate { x_min, x_max, parameters, noise, .. } => {
            check_finite(*x_min, || format!("{sheet_id}.analysis.simulate.x_min"))?;
            check_finite(*x_max, || format!("{sheet_id}.analysis.simulate.x_max"))?;
            for (idx, (_, val)) in parameters.iter().enumerate() {
                check_finite(*val, || format!("{sheet_id}.analysis.simulate.parameters[{idx}]"))?;
            }
            match noise {
                lumencraft_doc::SimulationNoise::Gaussian { sd } => {
                    check_finite(*sd, || format!("{sheet_id}.analysis.simulate.noise.sd"))?;
                }
                lumencraft_doc::SimulationNoise::RelativePercent { cv_percent } => {
                    check_finite(*cv_percent, || format!("{sheet_id}.analysis.simulate.noise.cv_percent"))?;
                }
                lumencraft_doc::SimulationNoise::None | lumencraft_doc::SimulationNoise::Poisson => {}
            }
        }
        lumencraft_doc::TransformAnalysisSpec::RemoveBaseline { .. }
        | lumencraft_doc::TransformAnalysisSpec::Transpose { .. }
        | lumencraft_doc::TransformAnalysisSpec::PruneRows { .. }
        | lumencraft_doc::TransformAnalysisSpec::SelectAndTransform { .. }
        | lumencraft_doc::TransformAnalysisSpec::RandomSample { .. } => {}
    }
    Ok(())
}

fn validate_nonlinear_fit_floats(fit: &lumencraft_doc::NonlinearFitSpec, sheet_id: &str) -> Result<(), FormatError> {
    if let lumencraft_doc::FitMethod::RoutOutlierElimination { fdr_q_percent } = fit.method {
        check_finite(fdr_q_percent, || format!("{sheet_id}.analysis.fit_method.fdr_q_percent"))?;
    }
    if let lumencraft_doc::WeightingScheme::GeneralPowerY(k) = fit.weighting {
        check_finite(k, || format!("{sheet_id}.analysis.weighting.power"))?;
    }
    for (idx, rule) in fit.parameter_rules.iter().enumerate() {
        check_opt_finite(rule.initial_override, || format!("{sheet_id}.parameter_rules[{idx}].initial_override"))?;
        check_opt_finite(rule.lower_bound, || format!("{sheet_id}.parameter_rules[{idx}].lower_bound"))?;
        check_opt_finite(rule.upper_bound, || format!("{sheet_id}.parameter_rules[{idx}].upper_bound"))?;
        match &rule.scope {
            lumencraft_doc::ParameterScope::Constant(v) => {
                check_finite(*v, || format!("{sheet_id}.parameter_rules[{idx}].constant"))?;
            }
            lumencraft_doc::ParameterScope::PerDatasetConstant(entries) => {
                for (entry_idx, (_, val)) in entries.iter().enumerate() {
                    check_finite(*val, || format!("{sheet_id}.parameter_rules[{idx}].per_dataset[{entry_idx}]"))?;
                }
            }
            lumencraft_doc::ParameterScope::Local
            | lumencraft_doc::ParameterScope::SharedGlobal
            | lumencraft_doc::ParameterScope::HookedConstant(_)
            | lumencraft_doc::ParameterScope::ColumnTitleConstant => {}
        }
    }
    if let Some((lo, hi)) = fit.x_range_filter {
        check_opt_finite(lo, || format!("{sheet_id}.analysis.x_range_filter.min"))?;
        check_opt_finite(hi, || format!("{sheet_id}.analysis.x_range_filter.max"))?;
    }
    if let Some((lo, hi)) = fit.y_range_filter {
        check_opt_finite(lo, || format!("{sheet_id}.analysis.y_range_filter.min"))?;
        check_opt_finite(hi, || format!("{sheet_id}.analysis.y_range_filter.max"))?;
    }
    check_opt_finite(fit.plot_x_min, || format!("{sheet_id}.analysis.plot_x_min"))?;
    check_opt_finite(fit.plot_x_max, || format!("{sheet_id}.analysis.plot_x_max"))
}

fn validate_analysis_kind_floats(spec: &lumencraft_doc::AnalysisSpec, sheet_id: &str) -> Result<(), FormatError> {
    check_finite(spec.confidence_level.percent(), || format!("{sheet_id}.analysis.confidence_level"))?;
    match &spec.kind {
        lumencraft_doc::AnalysisKind::Transform(transform) => validate_transform_floats(transform, sheet_id),
        lumencraft_doc::AnalysisKind::DescriptiveStats(d) => {
            check_opt_finite(d.custom_upper_percentile, || format!("{sheet_id}.analysis.custom_upper_percentile"))?;
            check_opt_finite(d.hypothetical_value, || format!("{sheet_id}.analysis.hypothetical_value"))
        }
        lumencraft_doc::AnalysisKind::OutlierDetection(o) => match o {
            lumencraft_doc::OutlierSpec::Grubbs { alpha, .. } => check_finite(*alpha, || format!("{sheet_id}.analysis.outlier.alpha")),
            lumencraft_doc::OutlierSpec::Rout { fdr_q_percent } => {
                check_finite(*fdr_q_percent, || format!("{sheet_id}.analysis.outlier.fdr_q_percent"))
            }
        },
        lumencraft_doc::AnalysisKind::TwoGroupComparison(t) => match t.method {
            lumencraft_doc::TwoGroupMethod::OneSampleT { hypothetical } | lumencraft_doc::TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
                check_finite(hypothetical, || format!("{sheet_id}.analysis.hypothetical"))
            }
            lumencraft_doc::TwoGroupMethod::UnpairedT
            | lumencraft_doc::TwoGroupMethod::WelchT
            | lumencraft_doc::TwoGroupMethod::PairedT
            | lumencraft_doc::TwoGroupMethod::RatioPairedT
            | lumencraft_doc::TwoGroupMethod::LognormalUnpairedT
            | lumencraft_doc::TwoGroupMethod::MannWhitneyU
            | lumencraft_doc::TwoGroupMethod::WilcoxonSignedRank
            | lumencraft_doc::TwoGroupMethod::KolmogorovSmirnov
            | lumencraft_doc::TwoGroupMethod::PairedSignTest => Ok(()),
        },
        lumencraft_doc::AnalysisKind::MultipleRowTTests(s) => {
            check_finite(s.multiplicity.threshold, || format!("{sheet_id}.analysis.multiplicity.threshold"))
        }
        lumencraft_doc::AnalysisKind::OneWayAnova(s) => {
            check_finite(s.multiplicity.threshold, || format!("{sheet_id}.analysis.multiplicity.threshold"))
        }
        lumencraft_doc::AnalysisKind::FactorialAnova(s) => {
            check_finite(s.multiplicity.threshold, || format!("{sheet_id}.analysis.multiplicity.threshold"))
        }
        lumencraft_doc::AnalysisKind::NestedComparison(s) => {
            check_finite(s.multiplicity.threshold, || format!("{sheet_id}.analysis.multiplicity.threshold"))
        }
        lumencraft_doc::AnalysisKind::Contingency(c) => {
            if let Some(props) = &c.expected_proportions {
                for (idx, p) in props.iter().enumerate() {
                    check_finite(*p, || format!("{sheet_id}.analysis.expected_proportions[{idx}]"))?;
                }
            }
            check_opt_finite(c.null_proportion, || format!("{sheet_id}.analysis.null_proportion"))
        }
        lumencraft_doc::AnalysisKind::Survival(s) => match s {
            lumencraft_doc::SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. } => {
                check_finite(multiplicity.threshold, || format!("{sheet_id}.analysis.multiplicity.threshold"))
            }
            lumencraft_doc::SurvivalAnalysisSpec::CoxProportionalHazards { .. } => Ok(()),
        },
        lumencraft_doc::AnalysisKind::NonlinearRegression(fit) => validate_nonlinear_fit_floats(fit, sheet_id),
        lumencraft_doc::AnalysisKind::BlandAltman(_)
        | lumencraft_doc::AnalysisKind::PassingBablok(_)
        | lumencraft_doc::AnalysisKind::RocCurve(_)
        | lumencraft_doc::AnalysisKind::GeneralizedLinearModel(_)
        | lumencraft_doc::AnalysisKind::Multivariate(_)
        | lumencraft_doc::AnalysisKind::LinearRegression(_) => Ok(()),
    }
}

fn register_unique_sheet_id(seen: &mut HashSet<SheetId>, id: &SheetId) -> Result<(), FormatError> {
    if id.as_str().trim().is_empty() {
        return Err(FormatError::Doc(lumencraft_doc::DocError::EmptyIdentifier));
    }
    if !seen.insert(id.clone()) {
        return Err(FormatError::Doc(lumencraft_doc::DocError::DuplicateSheetId { sheet_id: id.clone() }));
    }
    Ok(())
}

fn validate_data_sheet(ds: &DataSheet) -> Result<(), FormatError> {
    ds.validate()?;
    let ctx = ds.id.as_str();
    match &ds.archetype {
        TableArchetype::Xy(t) => {
            if let lumencraft_doc::XColumnMode::GeneratedSequence { start, step, .. } = t.x_mode
                && (!start.is_finite() || !step.is_finite())
            {
                return Err(FormatError::NonFiniteFloat { location: format!("{ctx}.x_mode") });
            }
            validate_title(&t.x_title)?;
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_subcolumns(t.y_subcolumn_mode.subcolumn_count())?;
            validate_cell_rows(&t.x_cells, MAX_SUBCOLUMNS, &format!("{ctx}.x_cells"))?;
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::Column(t) => {
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_subcolumns(t.subcolumn_mode.subcolumn_count())?;
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::Grouped(t) => {
            validate_title(&t.row_factor_title)?;
            validate_title(&t.column_factor_title)?;
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_subcolumns(t.subcolumn_mode.subcolumn_count())?;
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::Contingency(t) => {
            validate_title(&t.row_factor_title)?;
            validate_title(&t.column_factor_title)?;
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::Survival(t) => {
            validate_title(&t.time_title)?;
            validate_rows(t.row_count())?;
            validate_data_sets(t.cohorts.len())?;
            for (r, c) in t.times.iter().enumerate() {
                validate_cell(c, &format!("{ctx}.times[{r}]"))?;
            }
            validate_titles(&t.row_titles)?;
            validate_dataset_columns(&t.cohorts, ctx)?;
        }
        TableArchetype::PartsOfWhole(t) => {
            validate_rows(t.row_count())?;
            validate_data_sets(t.data_sets.len())?;
            validate_titles(&t.slice_labels)?;
            validate_dataset_columns(&t.data_sets, ctx)?;
        }
        TableArchetype::MultipleVariables(t) => {
            validate_rows(t.row_count())?;
            validate_data_sets(t.variables.len())?;
            validate_titles(&t.row_titles)?;
            for (c, var) in t.variables.iter().enumerate() {
                validate_title(&var.title)?;
                validate_rows(var.cells.len())?;
                for (r, cell) in var.cells.iter().enumerate() {
                    validate_cell(cell, &format!("{ctx}.var[{c}][{r}]"))?;
                }
            }
        }
        TableArchetype::Nested(t) => {
            validate_rows(t.row_count())?;
            validate_data_sets(t.groups.len())?;
            validate_titles(&t.row_titles)?;
            for (g_idx, grp) in t.groups.iter().enumerate() {
                validate_title(&grp.title)?;
                validate_subcolumns(grp.biological_unit_count())?;
                validate_titles(&grp.subcolumn_titles)?;
                validate_cell_rows(&grp.cells, MAX_SUBCOLUMNS, &format!("{ctx}.group[{g_idx}]"))?;
            }
        }
    }
    Ok(())
}

fn validate_titles(titles: &[String]) -> Result<(), FormatError> {
    if titles.len() > MAX_ROWS {
        return Err(FormatError::Doc(lumencraft_doc::DocError::TooManyRows { count: titles.len(), max: MAX_ROWS }));
    }
    for title in titles {
        validate_title(title)?;
    }
    Ok(())
}

fn validate_dataset_columns(cols: &[DataSetColumn], ctx: &str) -> Result<(), FormatError> {
    if cols.len() > MAX_DATA_SETS {
        return Err(FormatError::Doc(lumencraft_doc::DocError::TooManyDataSets { count: cols.len(), max: MAX_DATA_SETS }));
    }
    for (c, col) in cols.iter().enumerate() {
        validate_title(&col.title)?;
        validate_cell_rows(&col.cells, MAX_SUBCOLUMNS, &format!("{ctx}.col[{c}]"))?;
    }
    Ok(())
}

fn validate_cell_rows(rows: &[Vec<Cell>], max_subs: usize, ctx: &str) -> Result<(), FormatError> {
    validate_rows(rows.len())?;
    for (r, row) in rows.iter().enumerate() {
        if row.len() > max_subs {
            return Err(FormatError::Doc(lumencraft_doc::DocError::TooManySubcolumns { count: row.len(), max: max_subs }));
        }
        for (s, cell) in row.iter().enumerate() {
            validate_cell(cell, &format!("{ctx}[{r}][{s}]"))?;
        }
    }
    Ok(())
}

fn validate_cell(cell: &Cell, loc: &str) -> Result<(), FormatError> {
    if let CellValue::Number(v) = cell.value
        && !v.is_finite()
    {
        return Err(FormatError::NonFiniteFloat { location: loc.to_owned() });
    }
    Ok(())
}

fn validate_info_sheet(info: &InfoSheet) -> Result<(), FormatError> {
    validate_title(&info.title)?;
    for (idx, c) in info.constants.iter().enumerate() {
        validate_title(&c.name)?;
        if let Some(u) = &c.units {
            validate_title(u)?;
        }
        if let ConstantValue::Number(v) = c.value
            && !v.is_finite()
        {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.constants[{idx}]", info.id) });
        }
    }
    Ok(())
}

fn validate_result_sheet(res: &ResultSheet) -> Result<(), FormatError> {
    validate_title(res.title())?;
    for (idx, s) in res.scalars.iter().enumerate() {
        if !s.value.is_finite() {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.scalars[{idx}]", res.id) });
        }
    }
    for (tab_idx, tab) in res.tabs.iter().enumerate() {
        for (r, row) in tab.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                validate_result_cell_finite(cell, || format!("{}.tabs[{tab_idx}].rows[{r}][{c}]", res.id))?;
            }
        }
    }
    for (idx, cmp) in res.pairwise_comparisons.iter().enumerate() {
        if cmp.estimate_diff.is_some_and(|v| !v.is_finite()) || !cmp.raw_p_or_q.is_finite() {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.pairwise_comparisons[{idx}]", res.id) });
        }
    }
    for (c_idx, curve) in res.fitted_curves.iter().enumerate() {
        for (p_idx, pt) in curve.points.iter().enumerate() {
            if [Some(pt.x), Some(pt.y), pt.ci_lower, pt.ci_upper, pt.pred_lower, pt.pred_upper].into_iter().flatten().any(|v| !v.is_finite()) {
                return Err(FormatError::NonFiniteFloat { location: format!("{}.fitted_curves[{c_idx}].points[{p_idx}]", res.id) });
            }
        }
    }
    if let Some(data) = &res.analyzable_data {
        validate_data_sheet(data)?;
    }
    validate_analysis_hooks_and_bounds(&res.analysis.kind, &res.id)?;
    Ok(())
}

fn validate_analysis_hooks_and_bounds(kind: &AnalysisKind, sheet_id: &SheetId) -> Result<(), FormatError> {
    if let AnalysisKind::NonlinearRegression(fit) = kind {
        for (idx, rule) in fit.parameter_rules.iter().enumerate() {
            if let ParameterScope::PerDatasetConstant(pairs) = &rule.scope {
                for (ds_idx, val) in pairs {
                    if !val.is_finite() {
                        return Err(FormatError::NonFiniteFloat { location: format!("{sheet_id}.parameter_rules[{idx}].per_dataset[{ds_idx}]") });
                    }
                }
            }
        }
        let bounds = [fit.plot_x_min, fit.plot_x_max]
            .into_iter()
            .chain(fit.x_range_filter.into_iter().flat_map(|(a, b)| [a, b]))
            .chain(fit.y_range_filter.into_iter().flat_map(|(a, b)| [a, b]))
            .flatten();
        if bounds.into_iter().any(|v| !v.is_finite()) {
            return Err(FormatError::NonFiniteFloat { location: format!("{sheet_id}.nonlinear_fit_bounds") });
        }
    }
    Ok(())
}

fn validate_result_cell_finite<F>(cell: &ResultCell, loc: F) -> Result<(), FormatError>
where
    F: FnOnce() -> String,
{
    let is_ok = match cell {
        ResultCell::Number { value, .. } => value.is_finite(),
        ResultCell::PValue { raw, .. } => raw.is_finite(),
        ResultCell::ConfidenceInterval { lower, upper } => ci_bound_finite(*lower) && ci_bound_finite(*upper),
        ResultCell::Empty | ResultCell::Text(_) | ResultCell::Boolean(_) => true,
    };
    if is_ok { Ok(()) } else { Err(FormatError::NonFiniteFloat { location: loc() }) }
}

fn ci_bound_finite(bound: CiBound) -> bool {
    match bound {
        CiBound::Finite(v) => v.is_finite(),
        CiBound::Open => true,
    }
}

/// Detect circular `HookSource`, `ImportLinkSpec`, and transform/analysis DAG dependencies across
/// all sheets in `doc` (including self-referential loops).
pub fn detect_dependency_cycles(doc: &ProjectDocument) -> Result<(), FormatError> {
    let mut adj: BTreeMap<SheetId, Vec<SheetId>> = BTreeMap::new();
    let default_info = doc.info_sheets.first().map(|s| s.id.clone());

    for ds in &doc.data_sheets {
        let mut deps = Vec::new();
        if let Some(info_id) = &ds.info_sheet_id {
            deps.push(info_id.clone());
        }
        for region in &ds.linked_regions {
            if let Some(up) = region.spec.source.upstream_sheet_id() {
                deps.push(up.clone());
            }
        }
        adj.insert(ds.id.clone(), deps);
    }
    for info in &doc.info_sheets {
        adj.insert(info.id.clone(), Vec::new());
    }
    for res in &doc.result_sheets {
        let mut deps = vec![res.analysis.source_sheet_id.clone()];
        let paired_info = doc.data_sheet(&res.analysis.source_sheet_id).and_then(|d| d.info_sheet_id.clone()).or_else(|| default_info.clone());
        for hook in res.analysis.kind.hooked_sources() {
            if let Some(up) = resolve_hook_target(&hook, paired_info.as_ref()) {
                deps.push(up);
            }
        }
        adj.insert(res.id.clone(), deps);
    }
    for gs in &doc.graph_sheets {
        adj.insert(gs.id.clone(), collect_graph_deps(gs, default_info.as_ref()));
    }
    for ls in &doc.layout_sheets {
        adj.insert(ls.id.clone(), collect_layout_deps(ls, default_info.as_ref()));
    }

    run_cycle_dfs(&adj)
}

fn collect_graph_deps(gs: &GraphSheet, default_info: Option<&SheetId>) -> Vec<SheetId> {
    let mut deps = vec![gs.spec.primary_source_sheet_id.clone()];
    for series in &gs.spec.series {
        deps.push(series.source_sheet_id.clone());
        if let Some(ov) = &series.regression_overlay {
            deps.push(ov.result_sheet_id.clone());
        }
    }
    for cmp in &gs.spec.comparisons {
        deps.push(cmp.result_sheet_id.clone());
    }
    for ann in &gs.spec.annotations {
        match ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                deps.push(result_sheet_id.clone());
            }
            GraphAnnotation::TextBox { .. } | GraphAnnotation::LineOrArrow { .. } | GraphAnnotation::Shape { .. } => {}
        }
    }
    for hook in gs.spec.inline_hooks() {
        if let Some(up) = resolve_hook_target(&hook, default_info) {
            deps.push(up);
        }
    }
    deps
}

fn collect_layout_deps(ls: &LayoutSheet, default_info: Option<&SheetId>) -> Vec<SheetId> {
    let mut deps = Vec::new();
    for item in &ls.spec.items {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => deps.push(graph_sheet_id.clone()),
            LayoutItem::LiveTable { source_sheet_id, .. } => deps.push(source_sheet_id.clone()),
            LayoutItem::TextCallout { .. } => {}
        }
    }
    for hook in ls.spec.inline_hooks() {
        if let Some(up) = resolve_hook_target(&hook, default_info) {
            deps.push(up);
        }
    }
    deps
}

fn resolve_hook_target(hook: &HookSource, fallback_info: Option<&SheetId>) -> Option<SheetId> {
    match hook {
        HookSource::InfoConstant { info_sheet_id: Some(id), .. } => Some(id.clone()),
        HookSource::InfoConstant { info_sheet_id: None, .. } => fallback_info.cloned(),
        HookSource::ResultScalar { result_sheet_id, .. } => Some(result_sheet_id.clone()),
        HookSource::ColumnTitleValue { data_sheet_id, .. } => Some(data_sheet_id.clone()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

fn run_cycle_dfs(adj: &BTreeMap<SheetId, Vec<SheetId>>) -> Result<(), FormatError> {
    let mut state: HashMap<SheetId, VisitState> = HashMap::with_capacity(adj.len());
    let mut stack: Vec<SheetId> = Vec::new();
    for node in adj.keys() {
        if !state.contains_key(node) {
            dfs_visit(node, adj, &mut state, &mut stack)?;
        }
    }
    Ok(())
}

fn dfs_visit(
    node: &SheetId,
    adj: &BTreeMap<SheetId, Vec<SheetId>>,
    state: &mut HashMap<SheetId, VisitState>,
    stack: &mut Vec<SheetId>,
) -> Result<(), FormatError> {
    state.insert(node.clone(), VisitState::Visiting);
    stack.push(node.clone());
    if let Some(neighbors) = adj.get(node) {
        for next in neighbors {
            match state.get(next).copied() {
                Some(VisitState::Visiting) => {
                    let start = stack.iter().position(|id| id == next).unwrap_or(0);
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(next.clone());
                    return Err(FormatError::DependencyCycle { cycle });
                }
                Some(VisitState::Done) => {}
                None => {
                    if adj.contains_key(next) {
                        dfs_visit(next, adj, state, stack)?;
                    }
                }
            }
        }
    }
    let _ = stack.pop();
    state.insert(node.clone(), VisitState::Done);
    Ok(())
}

/// Recovery helper that detects circular [`HookSource`] references inside `doc`, breaks the
/// offending hook edges cleanly so the dependency graph becomes acyclic, and returns human-readable
/// diagnostic descriptions of each sanitized hook.
pub fn sanitize_circular_hooks(doc: &mut ProjectDocument) -> Vec<String> {
    let mut diagnostics = Vec::new();
    let max_iterations = doc.total_sheet_count().saturating_mul(8).saturating_add(16);

    for _ in 0..max_iterations {
        let Err(FormatError::DependencyCycle { cycle }) = detect_dependency_cycles(doc) else {
            break;
        };
        if cycle.len() < 2 {
            break;
        }
        let default_info = doc.info_sheets.first().map(|s| s.id.clone());
        let mut broke_edge = false;

        for edge in cycle.windows(2) {
            let [src_id, dst_id] = edge else {
                continue;
            };
            if let Some(msg) = break_hook_edge_in_result_sheets(doc, src_id, dst_id, default_info.as_ref()) {
                diagnostics.push(msg);
                broke_edge = true;
                break;
            }
            if let Some(msg) = break_hook_edge_in_graph_sheets(doc, src_id, dst_id, default_info.as_ref()) {
                diagnostics.push(msg);
                broke_edge = true;
                break;
            }
            if let Some(msg) = break_hook_edge_in_layout_sheets(doc, src_id, dst_id, default_info.as_ref()) {
                diagnostics.push(msg);
                broke_edge = true;
                break;
            }
        }

        if !broke_edge {
            break;
        }
    }

    diagnostics
}

fn paired_info_for_result(doc: &ProjectDocument, source_sheet_id: &SheetId, default_info: Option<&SheetId>) -> Option<SheetId> {
    doc.data_sheet(source_sheet_id).and_then(|d| d.info_sheet_id.clone()).or_else(|| default_info.cloned())
}

fn break_hook_edge_in_result_sheets(doc: &mut ProjectDocument, src_id: &SheetId, dst_id: &SheetId, default_info: Option<&SheetId>) -> Option<String> {
    let paired_info = {
        let res = doc.result_sheets.iter().find(|r| &r.id == src_id)?;
        paired_info_for_result(doc, &res.analysis.source_sheet_id, default_info)
    };
    let res = doc.result_sheets.iter_mut().find(|r| &r.id == src_id)?;
    match &mut res.analysis.kind {
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. }) => {
            if let NormalizeAnchor::Hooked(hook) = zero_anchor
                && resolve_hook_target(hook, paired_info.as_ref()).as_ref() == Some(dst_id)
            {
                *zero_anchor = NormalizeAnchor::FixedValue(0.0);
                return Some(format!("broke circular Normalize zero_anchor HookSource from `{src_id}` to `{dst_id}`"));
            }
            if let NormalizeAnchor::Hooked(hook) = hundred_anchor
                && resolve_hook_target(hook, paired_info.as_ref()).as_ref() == Some(dst_id)
            {
                *hundred_anchor = NormalizeAnchor::FixedValue(100.0);
                return Some(format!("broke circular Normalize hundred_anchor HookSource from `{src_id}` to `{dst_id}`"));
            }
            None
        }
        AnalysisKind::NonlinearRegression(fit) => {
            for rule in &mut fit.parameter_rules {
                if let ParameterScope::HookedConstant(hook) = &rule.scope
                    && resolve_hook_target(hook, paired_info.as_ref()).as_ref() == Some(dst_id)
                {
                    let param_name = rule.name.clone();
                    rule.scope = ParameterScope::Local;
                    return Some(format!("broke circular ParameterScope::HookedConstant(`{param_name}`) HookSource from `{src_id}` to `{dst_id}`"));
                }
            }
            None
        }
        _ => None,
    }
}

fn break_hook_edge_in_graph_sheets(doc: &mut ProjectDocument, src_id: &SheetId, dst_id: &SheetId, default_info: Option<&SheetId>) -> Option<String> {
    let gs = doc.graph_sheets.iter_mut().find(|g| &g.id == src_id)?;
    let mut removed = 0usize;
    removed += strip_template_hooks_targeting(&mut gs.spec.title, dst_id, default_info);
    removed += strip_template_hooks_targeting(&mut gs.spec.x_axis.title, dst_id, default_info);
    removed += strip_template_hooks_targeting(&mut gs.spec.y_axis.title, dst_id, default_info);
    if let Some(y2) = &mut gs.spec.y2_axis {
        removed += strip_template_hooks_targeting(&mut y2.title, dst_id, default_info);
    }
    for ann in &mut gs.spec.annotations {
        if let GraphAnnotation::TextBox { markdown, .. } = ann {
            removed += strip_template_hooks_targeting(markdown, dst_id, default_info);
        }
    }
    if removed > 0 { Some(format!("removed {removed} circular inline HookSource token(s) in graph `{src_id}` targeting `{dst_id}`")) } else { None }
}

fn break_hook_edge_in_layout_sheets(doc: &mut ProjectDocument, src_id: &SheetId, dst_id: &SheetId, default_info: Option<&SheetId>) -> Option<String> {
    let ls = doc.layout_sheets.iter_mut().find(|l| &l.id == src_id)?;
    let mut removed = 0usize;
    for item in &mut ls.spec.items {
        if let LayoutItem::TextCallout { markdown, .. } = item {
            removed += strip_template_hooks_targeting(markdown, dst_id, default_info);
        }
    }
    if removed > 0 { Some(format!("removed {removed} circular inline HookSource token(s) in layout `{src_id}` targeting `{dst_id}`")) } else { None }
}

fn strip_template_hooks_targeting(text: &mut String, target_id: &SheetId, default_info: Option<&SheetId>) -> usize {
    if !text.contains("{{") {
        return 0;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    let mut removed = 0usize;

    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("}}") else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let inner = &after_open[..close];
        let token = format!("{{{{{inner}}}}}");
        let hits_target = extract_template_hooks(&token)
            .first()
            .and_then(|hook| resolve_hook_target(hook, default_info))
            .is_some_and(|resolved| &resolved == target_id);
        if hits_target {
            removed = removed.saturating_add(1);
        } else {
            out.push_str(&token);
        }
        rest = &after_open[close + 2..];
    }
    out.push_str(rest);
    if removed > 0 {
        *text = out;
    }
    removed
}
