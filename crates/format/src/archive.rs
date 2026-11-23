//! Native `.curvebook` and `.cbtemplate` ZIP archive serialization, deserialization,
//! schema migration, dimension/float validation, and circular dependency guards.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{OpenOptions, Permissions};
use std::io::Write;
#[cfg(all(unix, not(target_arch = "wasm32")))]
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use curvebook_doc::{
    AnalysisKind, CURRENT_SCHEMA_VERSION, Cell, CellExclusionNote, CellValue, CiBound, CloneDataSheetOptions, ConstantValue, DataSetColumn,
    DataSheet, DocError, FittedCurveOverlay, FloatingNote, GraphAnnotation, GraphSheet, HookSource, ImageMimeKind, InfoSheet, LayoutItem,
    LayoutSheet, LinkedDataRegion, MAX_DATA_SETS, MAX_IMAGE_ASSET_BYTES, MAX_PROJECT_IMAGE_ASSETS, MAX_ROWS, MAX_SHEET_CELLS, MAX_SHEETS_PER_PROJECT,
    MAX_SUBCOLUMNS, NormalizeAnchor, ParameterScope, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, SheetId, SheetSection,
    SourceCellId, TableArchetype, TransformAnalysisSpec, extract_template_hooks, is_valid_asset_id, validate_data_sets, validate_project_cells,
    validate_rows, validate_subcolumns, validate_title,
};

use crate::csv::{CsvOptions, export_datasheet_to_csv};
use crate::error::FormatError;
use crate::image_asset::validate_image_asset;
use crate::versioned::{VersionStep, upgrade};
use crate::zip::{ZipArchive, ZipLimits, ZipWriter, sanitize_zip_path};

const MANIFEST_PATH: &str = "manifest.json";
const DOCUMENT_PATH: &str = "document.json";
const SHEETS_DIR: &str = "sheets";
const ASSETS_DIR: &str = "assets";
const UNPACKED_TX_DIR: &str = ".curvebook.tx";
const UNPACKED_TX_NEXT_DIR: &str = "next";
const UNPACKED_TX_PREV_DIR: &str = "prev";
const UNPACKED_TX_COMMIT_MARKER: &str = "COMMITTING";
const UNPACKED_TX_FORMAT: &str = "curvebook-unpacked-tx";
pub(crate) const FORMAT_MAGIC: &str = "curvebook-archive";
pub(crate) const LEGACY_FORMAT_MAGIC: &str = "lumencraft-archive";
const MAX_CHECKPOINTS_PER_PROJECT: usize = 64;
const MAX_SHEET_CSV_STEM_LEN: usize = 64;
const MAX_MANIFEST_CSV_STEM_LEN: usize = 128;

/// Maximum byte length permitted when reading a `.curvebook`, `.cbtemplate`, or `.prism` archive file.
pub const MAX_ARCHIVE_FILE_BYTES: usize = 128 * 1024 * 1024;

/// Kind of native CurveBook ZIP archive.
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

/// Index entry inside [`ArchiveManifest`] pointing to content-addressed raster image assets under `assets/`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssetManifestEntry {
    pub asset_id: String,
    pub media_type: ImageMimeKind,
    pub path: String,
    pub byte_len: u64,
    pub crc32: u32,
}

/// Top-level `manifest.json` stored inside `.curvebook` and `.cbtemplate` archives.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArchiveManifest {
    pub format: String,
    pub schema_version: u32,
    pub kind: ArchiveKind,
    pub title: String,
    pub document_crc32: u32,
    pub sheets: Vec<SheetManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<AssetManifestEntry>,
}

/// Serialize a [`ProjectDocument`] into deterministic `.curvebook` archive bytes.
pub fn save_project_bytes(doc: &ProjectDocument) -> Result<Vec<u8>, FormatError> {
    save_archive_bytes(doc, ArchiveKind::Project)
}

/// Serialize a [`ProjectDocument`] into deterministic `.cbtemplate` archive bytes, stripping
/// project-specific raw data cell values and cached results while preserving schemas, specs,
/// graphs, layouts, and info constants.
pub fn save_template_bytes(doc: &ProjectDocument) -> Result<Vec<u8>, FormatError> {
    let templated = strip_project_data_for_template(doc)?;
    save_archive_bytes(&templated, ArchiveKind::Template)
}

/// Load and validate a `.curvebook` project archive from raw bytes.
pub fn open_project_bytes(bytes: &[u8]) -> Result<ProjectDocument, FormatError> {
    let (kind, doc) = open_archive_bytes(bytes)?;
    if kind != ArchiveKind::Project {
        return Err(FormatError::InvalidArchiveKind { expected: ArchiveKind::Project.as_str(), found: kind.as_str() });
    }
    Ok(doc)
}

/// Load and validate a `.cbtemplate` archive from raw bytes.
pub fn open_template_bytes(bytes: &[u8]) -> Result<ProjectDocument, FormatError> {
    let (kind, doc) = open_archive_bytes(bytes)?;
    if kind != ArchiveKind::Template {
        return Err(FormatError::InvalidArchiveKind { expected: ArchiveKind::Template.as_str(), found: kind.as_str() });
    }
    Ok(doc)
}

/// Load a `.cbtemplate` (or `.curvebook`) archive and instantiate a clean [`ProjectDocument`]
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

/// Load any native CurveBook archive (`.curvebook` or `.cbtemplate`), returning its [`ArchiveKind`]
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

    let mut hydrated_assets = HashMap::with_capacity(manifest.assets.len());
    for entry in &manifest.assets {
        let asset_bytes = archive.get(&entry.path).ok_or_else(|| FormatError::MissingArchiveEntry { path: entry.path.clone() })?;
        if asset_bytes.len() as u64 != entry.byte_len {
            return Err(FormatError::ZipBomb {
                reason: format!("asset `{}` byte length {} disagrees with manifest {}", entry.path, asset_bytes.len(), entry.byte_len),
            });
        }
        let actual_crc = crc32fast::hash(asset_bytes);
        if actual_crc != entry.crc32 {
            return Err(FormatError::CorruptCrc { entry: entry.path.clone(), expected: entry.crc32, actual: actual_crc });
        }
        hydrated_assets.insert(entry.asset_id.clone(), (entry.media_type, asset_bytes.to_vec()));
    }

    let mut doc = parse_and_migrate_document(doc_bytes, manifest.schema_version)?;
    hydrate_archive_image_assets(&mut doc, &manifest.assets, hydrated_assets)?;
    validate_project_document(&doc)?;
    Ok((manifest.kind, doc))
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FormatError> {
    curvebook_platform::fs::write_atomic(path, bytes)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn platform_write_atomic(_path: &Path, _bytes: &[u8]) -> Result<(), FormatError> {
    Err(FormatError::Io(String::from("atomic file writes are not supported on wasm32")))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn platform_read_regular_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, FormatError> {
    Ok(curvebook_platform::fs::read_regular_file_bytes(path, max_bytes)?)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn platform_read_regular_file(_path: &Path, _max_bytes: usize) -> Result<Vec<u8>, FormatError> {
    Err(FormatError::Io(String::from("filesystem reads are not supported on wasm32")))
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_sync_dir(dir: &Path) -> Result<(), FormatError> {
    curvebook_platform::fs::sync_dir(dir)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn platform_sync_dir(_dir: &Path) -> Result<(), FormatError> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn stage_unpacked_entry_file(
    parent: &Path,
    target: &Path,
    rel: &str,
    bytes: &[u8],
    existing_perms: Option<Permissions>,
) -> Result<(PathBuf, String), FormatError> {
    let staged = curvebook_platform::fs::write_staged_sibling(parent, target, "stage", bytes, existing_perms)?;
    let stage_rel = stage_rel_for_entry(rel, staged.path())?;
    Ok((staged.into_path(), stage_rel))
}

#[cfg(target_arch = "wasm32")]
fn stage_unpacked_entry_file(
    _parent: &Path,
    _target: &Path,
    _rel: &str,
    _bytes: &[u8],
    _existing_perms: Option<Permissions>,
) -> Result<(PathBuf, String), FormatError> {
    Err(FormatError::Io(String::from("unpacked directory staging is not supported on wasm32")))
}

pub(crate) fn clamp_peek_schema_version(raw: u64) -> u32 {
    u32::try_from(raw).unwrap_or(u32::MAX)
}

fn peek_json_schema_version(bytes: &[u8]) -> Option<u32> {
    serde_json::from_slice::<DocumentSchemaPeek>(bytes).ok()?.schema_version.map(clamp_peek_schema_version)
}

fn ensure_schema_version_supported(version: u32) -> Result<(), FormatError> {
    if version > CURRENT_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version, max_supported: CURRENT_SCHEMA_VERSION });
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn inspect_archive_entries_not_newer<'a, I>(entries: I) -> Result<(), FormatError>
where
    I: IntoIterator<Item = (&'a str, Option<&'a [u8]>)>,
{
    let mut saw_metadata_entry = false;
    for (entry_path, maybe_bytes) in entries {
        let Some(entry_bytes) = maybe_bytes else {
            continue;
        };
        saw_metadata_entry = true;
        let peek: DocumentSchemaPeek = serde_json::from_slice(entry_bytes)
            .map_err(|err| FormatError::Json(format!("existing archive `{entry_path}` could not be parsed while checking schema version: {err}")))?;
        if let Some(version) = peek.schema_version.map(clamp_peek_schema_version) {
            ensure_schema_version_supported(version)?;
        }
    }
    if !saw_metadata_entry {
        return Err(FormatError::MissingArchiveEntry { path: String::from(MANIFEST_PATH) });
    }
    Ok(())
}

/// Inspect any existing `.curvebook` or `.cbtemplate` archive at `path` and refuse if its
/// `schema_version` exceeds [`CURRENT_SCHEMA_VERSION`].
pub fn ensure_existing_archive_not_newer(path: &Path) -> Result<(), FormatError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(bytes) = curvebook_platform::fs::read_regular_file_bytes_if_exists(path, MAX_ARCHIVE_FILE_BYTES)? else {
            return Ok(());
        };
        if bytes.is_empty() {
            return Ok(());
        }
        match ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()) {
            Ok(archive) => {
                inspect_archive_entries_not_newer([(MANIFEST_PATH, archive.get(MANIFEST_PATH)), (DOCUMENT_PATH, archive.get(DOCUMENT_PATH))])
            }
            Err(zip_err) => {
                if let Ok(peeked) = ZipArchive::peek_named_entries_bounded(&bytes, &[MANIFEST_PATH, DOCUMENT_PATH], crate::zip::MAX_ZIP_ENTRY_BYTES) {
                    for entry_path in [MANIFEST_PATH, DOCUMENT_PATH] {
                        if let Some(entry_bytes) = peeked.get(entry_path)
                            && let Some(version) = peek_json_schema_version(entry_bytes)
                        {
                            ensure_schema_version_supported(version)?;
                        }
                    }
                }
                Err(zip_err)
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Ok(())
    }
}

/// Inspect any existing unpacked `.curvebook.d/` directory at `dir` (and any pending crash
/// recovery snapshots under `.curvebook.tx/`) and refuse if any `schema_version` exceeds
/// [`CURRENT_SCHEMA_VERSION`].
pub fn ensure_existing_unpacked_dir_not_newer(dir: &Path) -> Result<(), FormatError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match std::fs::metadata(dir) {
            Ok(meta) if meta.is_dir() => {
                let canon_root = std::fs::canonicalize(dir)?;
                ensure_existing_unpacked_not_newer(&canon_root)
            }
            Ok(_) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(FormatError::Io(err.to_string())),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = dir;
        Ok(())
    }
}

/// Write a `.curvebook` archive to disk.
pub fn save_project_file(path: &Path, doc: &ProjectDocument) -> Result<(), FormatError> {
    let bytes = save_project_bytes(doc)?;
    ensure_existing_archive_not_newer(path)?;
    platform_write_atomic(path, &bytes)?;
    Ok(())
}

/// Read a `.curvebook` archive from disk.
pub fn open_project_file(path: &Path) -> Result<ProjectDocument, FormatError> {
    let bytes = platform_read_regular_file(path, MAX_ARCHIVE_FILE_BYTES)?;
    open_project_bytes(&bytes)
}

/// Write a `.cbtemplate` archive to disk.
pub fn save_template_file(path: &Path, doc: &ProjectDocument) -> Result<(), FormatError> {
    let bytes = save_template_bytes(doc)?;
    ensure_existing_archive_not_newer(path)?;
    platform_write_atomic(path, &bytes)?;
    Ok(())
}

/// Read a `.cbtemplate` archive from disk.
pub fn open_template_file(path: &Path) -> Result<ProjectDocument, FormatError> {
    let bytes = platform_read_regular_file(path, MAX_ARCHIVE_FILE_BYTES)?;
    open_template_bytes(&bytes)
}

/// Return `true` when `path` is an existing directory containing a valid unpacked `.curvebook.d/`
/// project manifest and document payload (rejecting template archives and escaping symlinks).
#[must_use]
pub fn is_unpacked_project_dir(path: &Path) -> bool {
    check_unpacked_project_dir(path).unwrap_or(false)
}

/// Write `doc` to an unpacked, Git-friendly `.curvebook.d/` project directory using the exact
/// deterministic `.curvebook` layout (`manifest.json`, canonical 2-space `document.json`, and
/// RFC-4180 `sheets/<id>.csv` CSVs), pruning stale tracked sheet files without disturbing `.git`
/// or user files, enforcing `ZipLimits::default()`, refusing future-schema overwrites, and
/// committing transactionally across staged temp files.
pub fn save_project_unpacked_dir(doc: &ProjectDocument, dir: &Path) -> Result<(), FormatError> {
    save_project_unpacked_dir_with_hook(doc, dir, |_| Ok(()))
}

enum MidCommitDirective {
    Proceed,
    FailRollback(std::io::Error),
    #[cfg(test)]
    SimulateCrash,
}

fn save_project_unpacked_dir_with_hook<F>(doc: &ProjectDocument, dir: &Path, mut before_commit: F) -> Result<(), FormatError>
where
    F: FnMut(&str) -> Result<(), std::io::Error>,
{
    save_project_unpacked_dir_driven(doc, dir, |rel| match before_commit(rel) {
        Ok(()) => MidCommitDirective::Proceed,
        Err(err) => MidCommitDirective::FailRollback(err),
    })
}

fn save_project_unpacked_dir_driven<F>(doc: &ProjectDocument, dir: &Path, on_commit_entry: F) -> Result<(), FormatError>
where
    F: FnMut(&str) -> MidCommitDirective,
{
    save_project_unpacked_dir_driven_with_rollback(doc, dir, on_commit_entry, platform_write_atomic)
}

fn save_project_unpacked_dir_driven_with_rollback<F, R>(
    doc: &ProjectDocument,
    dir: &Path,
    mut on_commit_entry: F,
    restore_file: R,
) -> Result<(), FormatError>
where
    F: FnMut(&str) -> MidCommitDirective,
    R: FnMut(&Path, &[u8]) -> Result<(), FormatError>,
{
    let entries = build_archive_entries(doc, ArchiveKind::Project)?;
    ensure_dir_exists(dir)?;
    let canon_root = std::fs::canonicalize(dir)?;
    preflight_unpacked_save(&canon_root, &entries)?;
    reconcile_pending_unpacked_tx(&canon_root)?;

    let prior_paths = collect_prior_manifest_csv_paths(&canon_root)?;
    let expected_paths: HashSet<String> = entries.iter().map(|(rel, _)| rel.clone()).collect();
    if !unpacked_changes_needed(&canon_root, &entries, &prior_paths, &expected_paths)? {
        return Ok(());
    }

    let prev_entries = capture_prior_snapshot_entries(&canon_root, ZipLimits::default());
    let mut tx_guard = begin_unpacked_tx(&canon_root, &entries, prev_entries.as_deref())?;

    let sheets_dir = canon_root.join(SHEETS_DIR);
    let sheets_existed_before = std::fs::symlink_metadata(&sheets_dir).is_ok();
    let assets_dir = canon_root.join(ASSETS_DIR);
    let assets_existed_before = std::fs::symlink_metadata(&assets_dir).is_ok();
    let cleanup_new_dirs = || {
        if !sheets_existed_before {
            let _ = std::fs::remove_dir(&sheets_dir);
        }
        if !assets_existed_before {
            let _ = std::fs::remove_dir(&assets_dir);
        }
    };

    let (staged, mut stage_guard) = match stage_changed_unpacked_entries(&canon_root, &entries) {
        Ok(res) => res,
        Err(err) => {
            cleanup_new_dirs();
            return Err(err);
        }
    };

    if let Err(err) = write_unpacked_tx_commit_marker(&canon_root, &staged, &prior_paths, &entries) {
        drop(stage_guard);
        cleanup_new_dirs();
        return Err(err);
    }

    if let Err(err) = commit_staged_unpacked_entries_with_rollback(&staged, &mut stage_guard, &mut tx_guard, &mut on_commit_entry, restore_file) {
        drop(stage_guard);
        cleanup_new_dirs();
        return Err(err);
    }

    prune_stale_unpacked_files(&canon_root, &prior_paths, &expected_paths)?;
    sync_unpacked_target_dirs(&canon_root)?;
    tx_guard.finish()?;
    platform_sync_dir(&canon_root)
}

/// Read and validate an unpacked `.curvebook.d/` project directory, enforcing strict relative-path
/// containment, symlink boundaries, CRC-32 verification, size caps, and schema migration.
pub fn open_project_unpacked_dir(dir: &Path) -> Result<ProjectDocument, FormatError> {
    if !std::fs::metadata(dir)?.is_dir() {
        return Err(FormatError::Io(format!("`{}` is not a directory", dir.display())));
    }
    let canon_root = std::fs::canonicalize(dir)?;
    let limits = ZipLimits::default();
    match inspect_pending_tx_state(&canon_root)? {
        PendingUnpackedTx::None => open_validated_unpacked_snapshot(&canon_root, limits),
        PendingUnpackedTx::Committing { next_dir, prev_dir } => match open_validated_unpacked_snapshot(&next_dir, limits) {
            Ok(doc) => Ok(doc),
            Err(err @ (FormatError::UnsupportedSchemaVersion { .. } | FormatError::InvalidArchiveKind { .. } | FormatError::UnsafePath { .. })) => {
                Err(err)
            }
            Err(next_err) => {
                if let Some(prev) = prev_dir
                    && let Ok(doc) = open_validated_unpacked_snapshot(&prev, limits)
                {
                    return Ok(doc);
                }
                Err(next_err)
            }
        },
    }
}

fn check_unpacked_project_dir(path: &Path) -> Result<bool, FormatError> {
    if !std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
        return Ok(false);
    }
    let canon_root = std::fs::canonicalize(path)?;
    match inspect_pending_tx_state(&canon_root)? {
        PendingUnpackedTx::None => check_snapshot_project_dir(&canon_root, false),
        PendingUnpackedTx::Committing { next_dir, prev_dir } => {
            if check_snapshot_project_dir(&next_dir, true).unwrap_or(false) {
                return Ok(true);
            }
            if let Some(prev) = prev_dir {
                return check_snapshot_project_dir(&prev, true);
            }
            Ok(false)
        }
    }
}

fn validate_sheet_csv_path(raw: &str) -> Result<(), FormatError> {
    let clean = sanitize_zip_path(raw)?;
    let Some(file_name) = clean.strip_prefix("sheets/") else {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    };
    let Some(stem) = file_name.strip_suffix(".csv") else {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    };
    if file_name.contains('/')
        || stem.is_empty()
        || stem.len() > MAX_MANIFEST_CSV_STEM_LEN
        || !stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    Ok(())
}

fn validate_asset_path(raw: &str, asset_id: &str, media_type: ImageMimeKind) -> Result<(), FormatError> {
    if !is_valid_asset_id(asset_id) {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    let clean = sanitize_zip_path(raw)?;
    let expected = format!("{ASSETS_DIR}/{asset_id}.{}", media_type.extension());
    if clean != expected {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    Ok(())
}

fn validate_asset_rel_path(raw: &str) -> Result<(), FormatError> {
    let clean = sanitize_zip_path(raw)?;
    let Some(file_name) = clean.strip_prefix("assets/") else {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    };
    if file_name.contains('/') {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    let Some((stem, ext)) = file_name.rsplit_once('.') else {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    };
    if !is_valid_asset_id(stem) || !matches!(ext, "png" | "jpg" | "tif") {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    Ok(())
}

fn validate_tracked_subpath(raw: &str) -> bool {
    validate_sheet_csv_path(raw).is_ok() || validate_asset_rel_path(raw).is_ok()
}

#[derive(serde::Deserialize)]
struct ManifestHeaderPeek {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    schema_version: Option<u32>,
}

fn reject_legacy_archive_magic(format: &str) -> Result<(), FormatError> {
    if format == LEGACY_FORMAT_MAGIC {
        return Err(FormatError::UnsupportedFormat {
            reason: format!("unsupported legacy archive format `{LEGACY_FORMAT_MAGIC}`; CurveBook does not open or convert LumenCraft files"),
        });
    }
    Ok(())
}

fn parse_and_validate_manifest(manifest_text: &str) -> Result<ArchiveManifest, FormatError> {
    if let Ok(peek) = serde_json::from_str::<ManifestHeaderPeek>(manifest_text) {
        if let Some(format) = peek.format.as_deref() {
            reject_legacy_archive_magic(format)?;
        }
        if let Some(version) = peek.schema_version {
            ensure_schema_version_supported(version)?;
        }
    }
    let manifest: ArchiveManifest = serde_json::from_str(manifest_text)?;
    reject_legacy_archive_magic(&manifest.format)?;
    if manifest.format != FORMAT_MAGIC {
        return Err(FormatError::UnsupportedFormat { reason: format!("unexpected archive format `{}`", manifest.format) });
    }
    ensure_schema_version_supported(manifest.schema_version)?;
    if manifest.sheets.len() > crate::zip::MAX_ZIP_ENTRIES {
        return Err(FormatError::ZipBomb {
            reason: format!("manifest sheet count {} exceeds limit {}", manifest.sheets.len(), crate::zip::MAX_ZIP_ENTRIES),
        });
    }
    for entry in &manifest.sheets {
        if let Some(csv_path) = &entry.csv_path {
            validate_sheet_csv_path(csv_path)?;
        }
    }
    if manifest.assets.len() > MAX_PROJECT_IMAGE_ASSETS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyImageAssets { count: manifest.assets.len(), max: MAX_PROJECT_IMAGE_ASSETS }));
    }
    let mut seen_asset_ids = HashSet::with_capacity(manifest.assets.len());
    let mut seen_asset_paths = HashSet::with_capacity(manifest.assets.len());
    for entry in &manifest.assets {
        validate_asset_path(&entry.path, &entry.asset_id, entry.media_type)?;
        if entry.byte_len > MAX_IMAGE_ASSET_BYTES as u64 {
            return Err(FormatError::FileTooLarge {
                path: entry.path.clone(),
                actual_bytes: Some(entry.byte_len),
                limit_bytes: MAX_IMAGE_ASSET_BYTES,
            });
        }
        if !seen_asset_ids.insert(entry.asset_id.as_str()) {
            return Err(FormatError::DuplicateEntry { path: entry.asset_id.clone() });
        }
        if !seen_asset_paths.insert(entry.path.as_str()) {
            return Err(FormatError::DuplicateEntry { path: entry.path.clone() });
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
    schema_version: Option<u64>,
}

struct JsonDomBudget {
    remaining_nodes: usize,
    remaining_bytes: usize,
    budget_exceeded: bool,
}

impl JsonDomBudget {
    fn charge(&mut self, nodes: usize, bytes: usize) -> Result<(), String> {
        if nodes > self.remaining_nodes {
            self.budget_exceeded = true;
            return Err(format!("JSON DOM node count exceeds migration limit {MAX_MIGRATION_JSON_NODES}"));
        }
        if bytes > self.remaining_bytes {
            self.budget_exceeded = true;
            return Err(String::from("JSON DOM allocation exceeds migration memory budget"));
        }
        self.remaining_nodes -= nodes;
        self.remaining_bytes -= bytes;
        Ok(())
    }
}

pub(crate) enum BoundedJsonOutcome {
    Value(serde_json::Value),
    BudgetExceeded(FormatError),
    SyntaxOrType(FormatError),
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

pub(crate) fn parse_bounded_json_outcome(doc_bytes: &[u8]) -> BoundedJsonOutcome {
    let max_bytes = doc_bytes.len().saturating_mul(6).clamp(MIN_MIGRATION_JSON_BYTES, MAX_MIGRATION_JSON_BYTES);
    let mut budget = JsonDomBudget { remaining_nodes: MAX_MIGRATION_JSON_NODES, remaining_bytes: max_bytes, budget_exceeded: false };
    let mut de = serde_json::Deserializer::from_slice(doc_bytes);
    match serde::de::DeserializeSeed::deserialize(BoundedValueSeed(&mut budget), &mut de) {
        Ok(val) => match de.end() {
            Ok(()) => BoundedJsonOutcome::Value(val),
            Err(err) => BoundedJsonOutcome::SyntaxOrType(FormatError::from(err)),
        },
        Err(err) => {
            let formatted = FormatError::from(err);
            if budget.budget_exceeded { BoundedJsonOutcome::BudgetExceeded(formatted) } else { BoundedJsonOutcome::SyntaxOrType(formatted) }
        }
    }
}

#[derive(Default)]
pub(crate) struct StreamedResultHeavyParts {
    pub(crate) analyzable_data: Option<DataSheet>,
    pub(crate) has_tabs: bool,
    pub(crate) tabs: Vec<ResultTab>,
    pub(crate) has_fitted_curves: bool,
    pub(crate) fitted_curves: Vec<FittedCurveOverlay>,
    pub(crate) has_eliminated_outliers: bool,
    pub(crate) eliminated_outliers: Vec<SourceCellId>,
}

pub(crate) struct StreamedHeavyParts {
    pub(crate) data_sheets: Vec<DataSheet>,
    pub(crate) result_heavy_parts: Vec<StreamedResultHeavyParts>,
}

pub(crate) struct StreamedProjectDocument {
    pub(crate) meta_doc: serde_json::Value,
    pub(crate) has_data_sheets_key: bool,
    pub(crate) heavy: StreamedHeavyParts,
}

pub(crate) enum StreamedDocumentOutcome {
    Parsed(Box<StreamedProjectDocument>),
    NonObject,
    BudgetExceeded(FormatError),
    DataError(FormatError),
    SyntaxOrType(FormatError),
}

struct StreamedDocContext {
    dom_budget: JsonDomBudget,
    seq_budget_exceeded: bool,
    saw_data_sheets_key: bool,
    used_project_cells: usize,
}

struct SeqBudget {
    max_single_seq: usize,
    remaining_total: usize,
    exceeded: bool,
}

impl SeqBudget {
    fn new(max_single_seq: usize, remaining_total: usize) -> Self {
        Self { max_single_seq, remaining_total, exceeded: false }
    }

    fn charge_item<E: serde::de::Error>(&mut self, seq_count: usize) -> Result<(), E> {
        if seq_count > self.max_single_seq {
            self.exceeded = true;
            return Err(serde::de::Error::custom(format!("JSON array length exceeds limit {}", self.max_single_seq)));
        }
        if self.remaining_total == 0 {
            self.exceeded = true;
            return Err(serde::de::Error::custom("JSON table element count exceeds sheet migration limit"));
        }
        self.remaining_total -= 1;
        Ok(())
    }
}

struct BoundedSeqDeserializer<'a, D> {
    inner: D,
    budget: &'a mut SeqBudget,
}

struct BoundedSeqVisitor<'a, V> {
    inner: V,
    budget: &'a mut SeqBudget,
}

struct BoundedSeqAccess<'a, A> {
    inner: A,
    budget: &'a mut SeqBudget,
    count: usize,
}

struct BoundedMapAccess<'a, A> {
    inner: A,
    budget: &'a mut SeqBudget,
}

struct BoundedSeed<'a, S> {
    inner: S,
    budget: &'a mut SeqBudget,
}

impl<'de, S> serde::de::DeserializeSeed<'de> for BoundedSeed<'_, S>
where
    S: serde::de::DeserializeSeed<'de>,
{
    type Value = S::Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        self.inner.deserialize(BoundedSeqDeserializer { inner: deserializer, budget: self.budget })
    }
}

impl<'de, A> serde::de::SeqAccess<'de> for BoundedSeqAccess<'_, A>
where
    A: serde::de::SeqAccess<'de>,
{
    type Error = A::Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: serde::de::DeserializeSeed<'de>,
    {
        let next = self.inner.next_element_seed(BoundedSeed { inner: seed, budget: self.budget })?;
        if next.is_some() {
            self.count = self.count.saturating_add(1);
            self.budget.charge_item(self.count)?;
        }
        Ok(next)
    }

    fn size_hint(&self) -> Option<usize> {
        self.inner.size_hint()
    }
}

impl<'de, A> serde::de::MapAccess<'de> for BoundedMapAccess<'_, A>
where
    A: serde::de::MapAccess<'de>,
{
    type Error = A::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: serde::de::DeserializeSeed<'de>,
    {
        self.inner.next_key_seed(BoundedSeed { inner: seed, budget: self.budget })
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::DeserializeSeed<'de>,
    {
        self.inner.next_value_seed(BoundedSeed { inner: seed, budget: self.budget })
    }

    fn next_entry_seed<K, V>(&mut self, kseed: K, vseed: V) -> Result<Option<(K::Value, V::Value)>, Self::Error>
    where
        K: serde::de::DeserializeSeed<'de>,
        V: serde::de::DeserializeSeed<'de>,
    {
        match self.next_key_seed(kseed)? {
            Some(key) => Ok(Some((key, self.next_value_seed(vseed)?))),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        self.inner.size_hint()
    }
}

impl<'de, 'a, V> serde::de::Visitor<'de> for BoundedSeqVisitor<'a, V>
where
    V: serde::de::Visitor<'de>,
{
    type Value = V::Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.expecting(formatter)
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
        self.inner.visit_bool(v)
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
        self.inner.visit_i64(v)
    }

    fn visit_i128<E: serde::de::Error>(self, v: i128) -> Result<Self::Value, E> {
        self.inner.visit_i128(v)
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
        self.inner.visit_u64(v)
    }

    fn visit_u128<E: serde::de::Error>(self, v: u128) -> Result<Self::Value, E> {
        self.inner.visit_u128(v)
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
        self.inner.visit_f64(v)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        self.inner.visit_str(v)
    }

    fn visit_borrowed_str<E: serde::de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
        self.inner.visit_borrowed_str(v)
    }

    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
        self.inner.visit_string(v)
    }

    fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
        self.inner.visit_bytes(v)
    }

    fn visit_borrowed_bytes<E: serde::de::Error>(self, v: &'de [u8]) -> Result<Self::Value, E> {
        self.inner.visit_borrowed_bytes(v)
    }

    fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
        self.inner.visit_byte_buf(v)
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        self.inner.visit_none()
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        self.inner.visit_some(BoundedSeqDeserializer { inner: deserializer, budget: self.budget })
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        self.inner.visit_unit()
    }

    fn visit_newtype_struct<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        self.inner.visit_newtype_struct(BoundedSeqDeserializer { inner: deserializer, budget: self.budget })
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        self.inner.visit_seq(BoundedSeqAccess { inner: seq, budget: self.budget, count: 0 })
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        self.inner.visit_map(BoundedMapAccess { inner: map, budget: self.budget })
    }
}

struct BoundedEnumAccessRef<'a, E> {
    inner: E,
    budget: &'a mut SeqBudget,
}

struct BoundedVariantAccessRef<'a, V> {
    inner: V,
    budget: &'a mut SeqBudget,
}

impl<'de, 'a, E> serde::de::EnumAccess<'de> for BoundedEnumAccessRef<'a, E>
where
    E: serde::de::EnumAccess<'de>,
{
    type Error = E::Error;
    type Variant = BoundedVariantAccessRef<'a, E::Variant>;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
    where
        V: serde::de::DeserializeSeed<'de>,
    {
        let (val, var) = self.inner.variant_seed(seed)?;
        Ok((val, BoundedVariantAccessRef { inner: var, budget: self.budget }))
    }
}

impl<'de, 'a, V> serde::de::VariantAccess<'de> for BoundedVariantAccessRef<'a, V>
where
    V: serde::de::VariantAccess<'de>,
{
    type Error = V::Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        self.inner.unit_variant()
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
    where
        T: serde::de::DeserializeSeed<'de>,
    {
        self.inner.newtype_variant_seed(BoundedSeed { inner: seed, budget: self.budget })
    }

    fn tuple_variant<W>(self, len: usize, visitor: W) -> Result<W::Value, Self::Error>
    where
        W: serde::de::Visitor<'de>,
    {
        self.inner.tuple_variant(len, BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn struct_variant<W>(self, fields: &'static [&'static str], visitor: W) -> Result<W::Value, Self::Error>
    where
        W: serde::de::Visitor<'de>,
    {
        self.inner.struct_variant(fields, BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }
}

impl<'de, 'a, D> serde::Deserializer<'de> for BoundedSeqDeserializer<'a, D>
where
    D: serde::Deserializer<'de>,
{
    type Error = D::Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_any(BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_bool<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_bool(visitor)
    }

    fn deserialize_i8<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_i8(visitor)
    }

    fn deserialize_i16<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_i16(visitor)
    }

    fn deserialize_i32<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_i32(visitor)
    }

    fn deserialize_i64<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_i64(visitor)
    }

    fn deserialize_u8<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_u8(visitor)
    }

    fn deserialize_u16<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_u16(visitor)
    }

    fn deserialize_u32<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_u32(visitor)
    }

    fn deserialize_u64<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_u64(visitor)
    }

    fn deserialize_f32<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_f32(visitor)
    }

    fn deserialize_f64<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_f64(visitor)
    }

    fn deserialize_char<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_char(visitor)
    }

    fn deserialize_str<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_str(visitor)
    }

    fn deserialize_string<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_string(visitor)
    }

    fn deserialize_bytes<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_bytes(visitor)
    }

    fn deserialize_byte_buf<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_byte_buf(visitor)
    }

    fn deserialize_option<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_option(BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_unit<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_unit(visitor)
    }

    fn deserialize_unit_struct<V: serde::de::Visitor<'de>>(self, name: &'static str, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_unit_struct(name, visitor)
    }

    fn deserialize_newtype_struct<V: serde::de::Visitor<'de>>(self, name: &'static str, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_newtype_struct(name, BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_seq<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_seq(BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_tuple<V: serde::de::Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_tuple(len, BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_tuple_struct<V: serde::de::Visitor<'de>>(self, name: &'static str, len: usize, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_tuple_struct(name, len, BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_map<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_map(BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_struct(name, fields, BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_enum<V: serde::de::Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        struct EnumWrapperVisitor<'a, V> {
            inner: V,
            budget: &'a mut SeqBudget,
        }
        impl<'de, 'a, V> serde::de::Visitor<'de> for EnumWrapperVisitor<'a, V>
        where
            V: serde::de::Visitor<'de>,
        {
            type Value = V::Value;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.inner.expecting(formatter)
            }

            fn visit_enum<A: serde::de::EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
                self.inner.visit_enum(BoundedEnumAccessRef { inner: data, budget: self.budget })
            }
        }
        self.inner.deserialize_enum(name, variants, EnumWrapperVisitor { inner: visitor, budget: self.budget })
    }

    fn deserialize_identifier<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_identifier(visitor)
    }

    fn deserialize_ignored_any<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_ignored_any(BoundedSeqVisitor { inner: visitor, budget: self.budget })
    }
}

struct BoundedTypedSeed<'a, T> {
    ctx: &'a mut StreamedDocContext,
    max_single_seq: usize,
    max_total_elems: usize,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T> BoundedTypedSeed<'a, T> {
    fn new(ctx: &'a mut StreamedDocContext, max_single_seq: usize, max_total_elems: usize) -> Self {
        Self { ctx, max_single_seq, max_total_elems, _marker: std::marker::PhantomData }
    }
}

impl<'de, T> serde::de::DeserializeSeed<'de> for BoundedTypedSeed<'_, T>
where
    T: serde::Deserialize<'de>,
{
    type Value = T;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut seq_budget = SeqBudget::new(self.max_single_seq, self.max_total_elems);
        let res = T::deserialize(BoundedSeqDeserializer { inner: deserializer, budget: &mut seq_budget });
        if seq_budget.exceeded {
            self.ctx.seq_budget_exceeded = true;
        }
        res
    }
}

struct LegacyDataSheetSeed<'a>(&'a mut StreamedDocContext);

impl<'de> serde::de::DeserializeSeed<'de> for LegacyDataSheetSeed<'_> {
    type Value = DataSheet;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct DataSheetVisitor<'a>(&'a mut StreamedDocContext);

        impl<'de> serde::de::Visitor<'de> for DataSheetVisitor<'_> {
            type Value = DataSheet;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a DataSheet JSON object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut id: Option<SheetId> = None;
                let mut title: Option<String> = None;
                let mut archetype: Option<TableArchetype> = None;
                let mut frozen: Option<bool> = None;
                let mut info_sheet_id: Option<Option<SheetId>> = None;
                let mut linked_regions: Option<Vec<LinkedDataRegion>> = None;
                let mut floating_notes: Option<Vec<FloatingNote>> = None;
                let mut color_tag: Option<Option<curvebook_palette::Rgba8>> = None;
                let mut exclusion_notes: Option<Vec<CellExclusionNote>> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "id" => id = Some(map.next_value()?),
                        "title" => title = Some(map.next_value()?),
                        "archetype" => {
                            let max_total = MAX_SHEET_CELLS.saturating_mul(4).saturating_add(MAX_ROWS.saturating_mul(4));
                            archetype = Some(map.next_value_seed(BoundedTypedSeed::<TableArchetype>::new(self.0, MAX_ROWS, max_total))?);
                        }
                        "frozen" => frozen = Some(map.next_value()?),
                        "info_sheet_id" => info_sheet_id = Some(map.next_value()?),
                        "linked_regions" => {
                            linked_regions = Some(map.next_value_seed(BoundedTypedSeed::<Vec<LinkedDataRegion>>::new(self.0, 10_000, 100_000))?);
                        }
                        "floating_notes" => {
                            floating_notes = Some(map.next_value_seed(BoundedTypedSeed::<Vec<FloatingNote>>::new(self.0, 10_000, 100_000))?);
                        }
                        "color_tag" => color_tag = Some(map.next_value()?),
                        "exclusion_notes" => {
                            let max_total = MAX_SHEET_CELLS.saturating_mul(2);
                            exclusion_notes =
                                Some(map.next_value_seed(BoundedTypedSeed::<Vec<CellExclusionNote>>::new(self.0, MAX_SHEET_CELLS, max_total))?);
                        }
                        _ => {
                            self.0.dom_budget.charge(0, key.len().saturating_add(64)).map_err(serde::de::Error::custom)?;
                            let _ = map.next_value_seed(BoundedValueSeed(&mut self.0.dom_budget))?;
                        }
                    }
                }

                let sheet = DataSheet {
                    id: id.ok_or_else(|| serde::de::Error::missing_field("id"))?,
                    title: title.ok_or_else(|| serde::de::Error::missing_field("title"))?,
                    archetype: archetype.ok_or_else(|| serde::de::Error::missing_field("archetype"))?,
                    frozen: frozen.unwrap_or(false),
                    info_sheet_id: info_sheet_id.unwrap_or(None),
                    linked_regions: linked_regions.unwrap_or_default(),
                    floating_notes: floating_notes.unwrap_or_default(),
                    color_tag: color_tag.unwrap_or(None),
                    exclusion_notes: exclusion_notes.unwrap_or_default(),
                };
                sheet.validate().map_err(serde::de::Error::custom)?;
                sheet.validate_finite_metadata().map_err(serde::de::Error::custom)?;
                self.0.used_project_cells =
                    validate_project_cells(self.0.used_project_cells, sheet.archetype.allocated_cell_slots()).map_err(serde::de::Error::custom)?;
                Ok(sheet)
            }
        }

        deserializer.deserialize_map(DataSheetVisitor(self.0))
    }
}

struct OptionLegacyDataSheetSeed<'a>(&'a mut StreamedDocContext);

impl<'de> serde::de::DeserializeSeed<'de> for OptionLegacyDataSheetSeed<'_> {
    type Value = Option<DataSheet>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct OptVisitor<'a>(&'a mut StreamedDocContext);
        impl<'de> serde::de::Visitor<'de> for OptVisitor<'_> {
            type Value = Option<DataSheet>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("null or a DataSheet JSON object")
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }

            fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
                serde::de::DeserializeSeed::deserialize(LegacyDataSheetSeed(self.0), deserializer).map(Some)
            }
        }
        deserializer.deserialize_option(OptVisitor(self.0))
    }
}

struct DataSheetsSeqSeed<'a>(&'a mut StreamedDocContext);

impl<'de> serde::de::DeserializeSeed<'de> for DataSheetsSeqSeed<'_> {
    type Value = Vec<DataSheet>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SeqVisitor<'a>(&'a mut StreamedDocContext);
        impl<'de> serde::de::Visitor<'de> for SeqVisitor<'_> {
            type Value = Vec<DataSheet>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a sequence of DataSheet objects")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut sheets = Vec::new();
                loop {
                    if sheets.len() >= MAX_SHEETS_PER_PROJECT {
                        // Probe one more element ONLY to see if sequence ends cleanly; reject if > MAX_SHEETS_PER_PROJECT.
                        let next_exists = seq.next_element::<serde::de::IgnoredAny>()?.is_some();
                        if next_exists {
                            return Err(serde::de::Error::custom(DocError::TooManySheets {
                                count: MAX_SHEETS_PER_PROJECT + 1,
                                max: MAX_SHEETS_PER_PROJECT,
                            }));
                        }
                        break;
                    }
                    let Some(sheet) = seq.next_element_seed(LegacyDataSheetSeed(self.0))? else {
                        break;
                    };
                    sheets.push(sheet);
                }
                Ok(sheets)
            }
        }
        deserializer.deserialize_seq(SeqVisitor(self.0))
    }
}

struct ResultSheetMigrationSeed<'a>(&'a mut StreamedDocContext);

impl<'de> serde::de::DeserializeSeed<'de> for ResultSheetMigrationSeed<'_> {
    type Value = (serde_json::Value, StreamedResultHeavyParts);

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ResultVisitor<'a>(&'a mut StreamedDocContext);

        impl<'de> serde::de::Visitor<'de> for ResultVisitor<'_> {
            type Value = (serde_json::Value, StreamedResultHeavyParts);

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a ResultSheet JSON object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                self.0.dom_budget.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
                let mut meta_obj = serde_json::Map::new();
                let mut heavy = StreamedResultHeavyParts::default();
                let max_tab_total = MAX_SHEET_CELLS.saturating_mul(4).saturating_add(MAX_ROWS.saturating_mul(4));

                while let Some(key) = map.next_key::<String>()? {
                    self.0.dom_budget.charge(0, key.len().saturating_add(64)).map_err(serde::de::Error::custom)?;
                    match key.as_str() {
                        "analyzable_data" => {
                            heavy.analyzable_data = map.next_value_seed(OptionLegacyDataSheetSeed(self.0))?;
                            meta_obj.insert(key, serde_json::Value::Null);
                        }
                        "tabs" => {
                            heavy.tabs = map.next_value_seed(BoundedTypedSeed::<Vec<ResultTab>>::new(self.0, MAX_SHEET_CELLS, max_tab_total))?;
                            heavy.has_tabs = true;
                            meta_obj.insert(key, serde_json::Value::Array(Vec::new()));
                        }
                        "fitted_curves" => {
                            heavy.fitted_curves =
                                map.next_value_seed(BoundedTypedSeed::<Vec<FittedCurveOverlay>>::new(self.0, MAX_SHEET_CELLS, max_tab_total))?;
                            heavy.has_fitted_curves = true;
                            meta_obj.insert(key, serde_json::Value::Array(Vec::new()));
                        }
                        "eliminated_outliers" => {
                            heavy.eliminated_outliers =
                                map.next_value_seed(BoundedTypedSeed::<Vec<SourceCellId>>::new(self.0, MAX_SHEET_CELLS, max_tab_total))?;
                            heavy.has_eliminated_outliers = true;
                            meta_obj.insert(key, serde_json::Value::Array(Vec::new()));
                        }
                        _ => {
                            let val = map.next_value_seed(BoundedValueSeed(&mut self.0.dom_budget))?;
                            meta_obj.insert(key, val);
                        }
                    }
                }
                Ok((serde_json::Value::Object(meta_obj), heavy))
            }

            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_bool(v)?, StreamedResultHeavyParts::default()))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_i64(v)?, StreamedResultHeavyParts::default()))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_u64(v)?, StreamedResultHeavyParts::default()))
            }

            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_f64(v)?, StreamedResultHeavyParts::default()))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_str(v)?, StreamedResultHeavyParts::default()))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_string(v)?, StreamedResultHeavyParts::default()))
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_none()?, StreamedResultHeavyParts::default()))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_unit()?, StreamedResultHeavyParts::default()))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
                Ok((BoundedValueVisitor(&mut self.0.dom_budget).visit_seq(seq)?, StreamedResultHeavyParts::default()))
            }
        }

        deserializer.deserialize_any(ResultVisitor(self.0))
    }
}

struct ResultSheetsSeqSeed<'a>(&'a mut StreamedDocContext);

impl<'de> serde::de::DeserializeSeed<'de> for ResultSheetsSeqSeed<'_> {
    type Value = (Vec<serde_json::Value>, Vec<StreamedResultHeavyParts>);

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SeqVisitor<'a>(&'a mut StreamedDocContext);
        impl<'de> serde::de::Visitor<'de> for SeqVisitor<'_> {
            type Value = (Vec<serde_json::Value>, Vec<StreamedResultHeavyParts>);

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a sequence of ResultSheet objects")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                self.0.dom_budget.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
                let mut metas = Vec::new();
                let mut heavies = Vec::new();
                while let Some((meta, heavy)) = seq.next_element_seed(ResultSheetMigrationSeed(self.0))? {
                    metas.push(meta);
                    heavies.push(heavy);
                    if metas.len() > MAX_SHEETS_PER_PROJECT {
                        return Err(serde::de::Error::custom(DocError::TooManySheets { count: metas.len(), max: MAX_SHEETS_PER_PROJECT }));
                    }
                }
                Ok((metas, heavies))
            }
        }
        deserializer.deserialize_seq(SeqVisitor(self.0))
    }
}

struct HybridDocumentSeed<'a>(&'a mut StreamedDocContext);

impl<'de> serde::de::DeserializeSeed<'de> for HybridDocumentSeed<'_> {
    type Value = Option<StreamedProjectDocument>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct RootVisitor<'a>(&'a mut StreamedDocContext);

        impl<'de> serde::de::Visitor<'de> for RootVisitor<'_> {
            type Value = Option<StreamedProjectDocument>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a ProjectDocument JSON object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                self.0.dom_budget.charge(1, std::mem::size_of::<serde_json::Value>()).map_err(serde::de::Error::custom)?;
                let mut meta_obj = serde_json::Map::new();
                let mut data_sheets = Vec::new();
                let mut result_heavy_parts = Vec::new();
                let mut has_data_sheets_key = false;

                while let Some(key) = map.next_key::<String>()? {
                    self.0.dom_budget.charge(0, key.len().saturating_add(64)).map_err(serde::de::Error::custom)?;
                    match key.as_str() {
                        "data_sheets" => {
                            has_data_sheets_key = true;
                            self.0.saw_data_sheets_key = true;
                            data_sheets = map.next_value_seed(DataSheetsSeqSeed(self.0))?;
                            meta_obj.insert(key, serde_json::Value::Array(Vec::new()));
                        }
                        "result_sheets" => {
                            let (metas, heavies) = map.next_value_seed(ResultSheetsSeqSeed(self.0))?;
                            result_heavy_parts = heavies;
                            meta_obj.insert(key, serde_json::Value::Array(metas));
                        }
                        _ => {
                            let val = map.next_value_seed(BoundedValueSeed(&mut self.0.dom_budget))?;
                            meta_obj.insert(key, val);
                        }
                    }
                }

                Ok(Some(StreamedProjectDocument {
                    meta_doc: serde_json::Value::Object(meta_obj),
                    has_data_sheets_key,
                    heavy: StreamedHeavyParts { data_sheets, result_heavy_parts },
                }))
            }

            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_bool(v)?;
                Ok(None)
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_i64(v)?;
                Ok(None)
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_u64(v)?;
                Ok(None)
            }

            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_f64(v)?;
                Ok(None)
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_str(v)?;
                Ok(None)
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_string(v)?;
                Ok(None)
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_none()?;
                Ok(None)
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_unit()?;
                Ok(None)
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
                let _ = BoundedValueVisitor(&mut self.0.dom_budget).visit_seq(seq)?;
                Ok(None)
            }
        }

        deserializer.deserialize_any(RootVisitor(self.0))
    }
}

pub(crate) fn parse_streamed_document_outcome(doc_bytes: &[u8]) -> StreamedDocumentOutcome {
    let max_bytes = doc_bytes.len().saturating_mul(6).clamp(MIN_MIGRATION_JSON_BYTES, MAX_MIGRATION_JSON_BYTES);
    let mut ctx = StreamedDocContext {
        dom_budget: JsonDomBudget { remaining_nodes: MAX_MIGRATION_JSON_NODES, remaining_bytes: max_bytes, budget_exceeded: false },
        seq_budget_exceeded: false,
        saw_data_sheets_key: false,
        used_project_cells: 0,
    };
    let mut de = serde_json::Deserializer::from_slice(doc_bytes);
    match serde::de::DeserializeSeed::deserialize(HybridDocumentSeed(&mut ctx), &mut de) {
        Ok(Some(doc)) => match de.end() {
            Ok(()) => StreamedDocumentOutcome::Parsed(Box::new(doc)),
            Err(err) => StreamedDocumentOutcome::SyntaxOrType(FormatError::from(err)),
        },
        Ok(None) => match de.end() {
            Ok(()) => StreamedDocumentOutcome::NonObject,
            Err(err) => StreamedDocumentOutcome::SyntaxOrType(FormatError::from(err)),
        },
        Err(err) => {
            let is_data_err = err.is_data();
            let formatted = FormatError::from(err);
            if ctx.dom_budget.budget_exceeded || ctx.seq_budget_exceeded {
                StreamedDocumentOutcome::BudgetExceeded(formatted)
            } else if ctx.saw_data_sheets_key && is_data_err {
                StreamedDocumentOutcome::DataError(formatted)
            } else {
                StreamedDocumentOutcome::SyntaxOrType(formatted)
            }
        }
    }
}

pub(crate) fn finalize_migrated_document(migrated_meta: serde_json::Value, heavy: StreamedHeavyParts) -> Result<ProjectDocument, FormatError> {
    let mut doc: ProjectDocument = serde_json::from_value(migrated_meta)?;
    doc.data_sheets = heavy.data_sheets;
    for (rs, parts) in doc.result_sheets.iter_mut().zip(heavy.result_heavy_parts) {
        rs.analyzable_data = parts.analyzable_data;
        if parts.has_tabs {
            rs.tabs = parts.tabs;
        }
        if parts.has_fitted_curves {
            rs.fitted_curves = parts.fitted_curves;
        }
        if parts.has_eliminated_outliers {
            rs.eliminated_outliers = parts.eliminated_outliers;
        }
        rs.validate()?;
    }
    doc.validate_structural_limits()?;
    Ok(doc)
}

fn parse_and_migrate_document(doc_bytes: &[u8], manifest_schema_version: u32) -> Result<ProjectDocument, FormatError> {
    let peek: DocumentSchemaPeek = serde_json::from_slice(doc_bytes)?;
    let doc_version = peek.schema_version.map(clamp_peek_schema_version).unwrap_or(manifest_schema_version);
    if doc_version > CURRENT_SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchemaVersion { version: doc_version, max_supported: CURRENT_SCHEMA_VERSION });
    }
    if doc_version == CURRENT_SCHEMA_VERSION && peek.schema_version.is_some() {
        let mut parsed: ProjectDocument = serde_json::from_slice(doc_bytes)?;
        parsed.schema_version = CURRENT_SCHEMA_VERSION;
        Ok(parsed)
    } else {
        let streamed = match parse_streamed_document_outcome(doc_bytes) {
            StreamedDocumentOutcome::Parsed(doc) => *doc,
            StreamedDocumentOutcome::NonObject => {
                return Err(FormatError::Json(String::from("expected ProjectDocument JSON object")));
            }
            StreamedDocumentOutcome::BudgetExceeded(err) | StreamedDocumentOutcome::DataError(err) | StreamedDocumentOutcome::SyntaxOrType(err) => {
                return Err(err);
            }
        };
        let migrated_value = migrate_document_json(streamed.meta_doc, doc_version)?;
        finalize_migrated_document(migrated_value, streamed.heavy)
    }
}

fn hydrate_archive_image_assets(
    doc: &mut ProjectDocument,
    manifest_assets: &[AssetManifestEntry],
    mut loaded_assets: HashMap<String, (ImageMimeKind, Vec<u8>)>,
) -> Result<(), FormatError> {
    if doc.image_assets.len() > MAX_PROJECT_IMAGE_ASSETS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyImageAssets { count: doc.image_assets.len(), max: MAX_PROJECT_IMAGE_ASSETS }));
    }
    let mut seen_ids = HashSet::with_capacity(doc.image_assets.len());
    for asset in &mut doc.image_assets {
        if !seen_ids.insert(asset.asset_id.clone()) {
            return Err(FormatError::DuplicateEntry { path: asset.asset_id.clone() });
        }
        let expected_path = manifest_assets
            .iter()
            .find(|e| e.asset_id == asset.asset_id)
            .map_or_else(|| format!("{ASSETS_DIR}/{}.{}", asset.asset_id, asset.media_type.extension()), |e| e.path.clone());
        let Some((loaded_mime, loaded_bytes)) = loaded_assets.remove(&asset.asset_id) else {
            return Err(FormatError::MissingArchiveEntry { path: expected_path });
        };
        if loaded_mime != asset.media_type {
            return Err(FormatError::Json(format!(
                "image asset `{}` media_type {:?} in document.json disagrees with {:?} in manifest.json",
                asset.asset_id, asset.media_type, loaded_mime
            )));
        }
        if !asset.bytes.is_empty() && asset.bytes != loaded_bytes {
            return Err(FormatError::Json(format!("image asset `{}` inline bytes in document.json disagree with archive payload", asset.asset_id)));
        }
        asset.bytes = loaded_bytes;
    }
    if let Some(orphan) = manifest_assets.iter().find(|entry| loaded_assets.contains_key(&entry.asset_id)) {
        return Err(FormatError::Json(format!(
            "manifest.json declares image asset `{}` (`{}`) missing from document.json",
            orphan.asset_id, orphan.path
        )));
    }
    Ok(())
}

fn decode_and_validate_document_inner(
    doc_bytes: &[u8],
    manifest_schema_version: u32,
    validate_checkpoint_entries: bool,
) -> Result<ProjectDocument, FormatError> {
    let doc = parse_and_migrate_document(doc_bytes, manifest_schema_version)?;
    validate_project_document_inner(&doc, validate_checkpoint_entries)?;
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

fn validate_archive_entry_limits(entries: &[(String, Vec<u8>)], limits: ZipLimits) -> Result<(), FormatError> {
    if entries.len() > limits.max_entries {
        return Err(FormatError::ZipBomb { reason: format!("archive entry count {} exceeds limit {}", entries.len(), limits.max_entries) });
    }
    let mut total_bytes = 0_u64;
    for (rel, bytes) in entries {
        let len = bytes.len() as u64;
        if len > limits.max_entry_uncompressed_bytes {
            return Err(FormatError::ZipBomb {
                reason: format!("entry `{rel}` size {len} exceeds security limit {}", limits.max_entry_uncompressed_bytes),
            });
        }
        total_bytes =
            total_bytes.checked_add(len).ok_or_else(|| FormatError::ZipBomb { reason: String::from("archive uncompressed size overflow") })?;
        if total_bytes > limits.max_total_uncompressed_bytes {
            return Err(FormatError::ZipBomb {
                reason: format!("archive total size {total_bytes} exceeds security limit {}", limits.max_total_uncompressed_bytes),
            });
        }
    }
    Ok(())
}

fn build_archive_entries(doc: &ProjectDocument, kind: ArchiveKind) -> Result<Vec<(String, Vec<u8>)>, FormatError> {
    ensure_schema_version_supported(doc.schema_version)?;
    validate_project_document(doc)?;

    let mut asset_entries = Vec::with_capacity(doc.image_assets.len());
    let mut manifest_assets = Vec::with_capacity(doc.image_assets.len());
    let normalized: Cow<'_, ProjectDocument> = if doc.schema_version == CURRENT_SCHEMA_VERSION && doc.image_assets.is_empty() {
        Cow::Borrowed(doc)
    } else {
        let mut owned = doc.clone();
        owned.schema_version = CURRENT_SCHEMA_VERSION;
        for asset in &mut owned.image_assets {
            let path = format!("{ASSETS_DIR}/{}.{}", asset.asset_id, asset.media_type.extension());
            let crc = crc32fast::hash(&asset.bytes);
            let byte_len = asset.bytes.len() as u64;
            asset_entries.push((path.clone(), std::mem::take(&mut asset.bytes)));
            manifest_assets.push(AssetManifestEntry { asset_id: asset.asset_id.clone(), media_type: asset.media_type, path, byte_len, crc32: crc });
        }
        Cow::Owned(owned)
    };
    let doc_json = serde_json::to_vec_pretty(normalized.as_ref())?;
    let doc_crc = crc32fast::hash(&doc_json);

    let mut entries = Vec::with_capacity(normalized.data_sheets.len() + asset_entries.len() + 2);
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

    entries.extend(asset_entries);
    push_metadata_entries(&mut manifest_sheets, &normalized);

    let manifest = ArchiveManifest {
        format: String::from(FORMAT_MAGIC),
        schema_version: CURRENT_SCHEMA_VERSION,
        kind,
        title: normalized.title.clone(),
        document_crc32: doc_crc,
        sheets: manifest_sheets,
        assets: manifest_assets,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    entries.push((String::from(MANIFEST_PATH), manifest_json));
    validate_archive_entry_limits(&entries, ZipLimits::default())?;
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
                if is_last && !must_exist && std::fs::metadata(&cur).is_ok_and(|m| !m.is_file()) {
                    return Err(FormatError::Io(format!("target path `{rel}` exists and is not a regular file")));
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

fn check_subdir_symlinks_contained(canon_root: &Path, subdir: &str) -> Result<(), FormatError> {
    let dir_path = canon_root.join(subdir);
    if let Ok(meta) = std::fs::symlink_metadata(&dir_path) {
        if meta.file_type().is_symlink() {
            check_symlink_contained(&dir_path, canon_root, subdir)?;
        } else if meta.is_dir() {
            for entry in std::fs::read_dir(&dir_path)? {
                let entry = entry?;
                if entry.file_type()?.is_symlink() {
                    let rel = format!("{subdir}/{}", entry.file_name().to_string_lossy());
                    check_symlink_contained(&entry.path(), canon_root, &rel)?;
                }
            }
        }
    }
    Ok(())
}

fn preflight_unpacked_save(canon_root: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), FormatError> {
    ensure_existing_unpacked_not_newer(canon_root)?;
    for (rel, _) in entries {
        resolve_contained_path(canon_root, rel, false)?;
    }
    check_subdir_symlinks_contained(canon_root, SHEETS_DIR)?;
    check_subdir_symlinks_contained(canon_root, ASSETS_DIR)?;
    Ok(())
}

fn ensure_non_symlink_subpath(parent: &Path, child_name: &str, rel_label: &str) -> Result<Option<PathBuf>, FormatError> {
    let path = parent.join(child_name);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(FormatError::UnsafePath { path: rel_label.to_owned() });
            }
            Ok(Some(path))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(FormatError::Io(err.to_string())),
    }
}

fn ensure_unpacked_metadata_files_not_newer(dir_root: &Path, allow_corrupt_json: bool) -> Result<(), FormatError> {
    let cap = usize::try_from(crate::zip::MAX_ZIP_ENTRY_BYTES).unwrap_or(usize::MAX);
    for rel in [MANIFEST_PATH, DOCUMENT_PATH] {
        let resolved = match resolve_contained_path(dir_root, rel, true) {
            Ok(path) => path,
            Err(FormatError::MissingArchiveEntry { .. }) => continue,
            Err(err) => return Err(err),
        };
        let bytes = platform_read_regular_file(&resolved, cap)?;
        if bytes.is_empty() {
            continue;
        }
        match serde_json::from_slice::<DocumentSchemaPeek>(&bytes) {
            Ok(peek) => {
                if let Some(version) = peek.schema_version.map(clamp_peek_schema_version) {
                    ensure_schema_version_supported(version)?;
                }
            }
            Err(_) if allow_corrupt_json => continue,
            Err(err) => {
                return Err(FormatError::Json(format!("existing unpacked `{rel}` could not be parsed while checking schema version: {err}")));
            }
        }
    }
    Ok(())
}

fn ensure_existing_unpacked_not_newer(canon_root: &Path) -> Result<(), FormatError> {
    let tx_dir = ensure_non_symlink_subpath(canon_root, UNPACKED_TX_DIR, UNPACKED_TX_DIR)?;
    let tx_commit_active = match tx_dir.as_ref() {
        Some(dir) if std::fs::metadata(dir).map(|m| m.is_dir()).unwrap_or(false) => {
            let marker_rel = format!("{UNPACKED_TX_DIR}/{UNPACKED_TX_COMMIT_MARKER}");
            ensure_non_symlink_subpath(dir, UNPACKED_TX_COMMIT_MARKER, &marker_rel)?.is_some()
        }
        _ => false,
    };
    ensure_unpacked_metadata_files_not_newer(canon_root, tx_commit_active)?;
    let Some(tx_dir) = tx_dir else {
        return Ok(());
    };
    if !std::fs::metadata(&tx_dir).map(|m| m.is_dir()).unwrap_or(false) {
        return Ok(());
    }
    for sub in [UNPACKED_TX_NEXT_DIR, UNPACKED_TX_PREV_DIR] {
        let rel = format!("{UNPACKED_TX_DIR}/{sub}");
        let Some(sub_path) = ensure_non_symlink_subpath(&tx_dir, sub, &rel)? else {
            continue;
        };
        if std::fs::metadata(&sub_path).map(|m| m.is_dir()).unwrap_or(false) {
            let canon_sub = std::fs::canonicalize(&sub_path)?;
            if !canon_sub.starts_with(canon_root) {
                return Err(FormatError::UnsafePath { path: rel });
            }
            ensure_unpacked_metadata_files_not_newer(&canon_sub, true)?;
        }
    }
    Ok(())
}

enum PendingUnpackedTx {
    None,
    Committing { next_dir: PathBuf, prev_dir: Option<PathBuf> },
}

fn validated_tx_subdir(canon_root: &Path, tx_dir: &Path, name: &str) -> Result<Option<PathBuf>, FormatError> {
    let rel = format!("{UNPACKED_TX_DIR}/{name}");
    let Some(sub) = ensure_non_symlink_subpath(tx_dir, name, &rel)? else {
        return Ok(None);
    };
    if !std::fs::metadata(&sub).map(|m| m.is_dir()).unwrap_or(false) {
        return Ok(None);
    }
    let canon_sub = std::fs::canonicalize(&sub)?;
    if !canon_sub.starts_with(canon_root) {
        return Err(FormatError::UnsafePath { path: rel });
    }
    Ok(Some(canon_sub))
}

fn inspect_pending_tx_state(canon_root: &Path) -> Result<PendingUnpackedTx, FormatError> {
    let Some(tx_dir) = ensure_non_symlink_subpath(canon_root, UNPACKED_TX_DIR, UNPACKED_TX_DIR)? else {
        return Ok(PendingUnpackedTx::None);
    };
    if !std::fs::metadata(&tx_dir).map(|m| m.is_dir()).unwrap_or(false) {
        return Ok(PendingUnpackedTx::None);
    }
    let marker_rel = format!("{UNPACKED_TX_DIR}/{UNPACKED_TX_COMMIT_MARKER}");
    let Some(marker) = ensure_non_symlink_subpath(&tx_dir, UNPACKED_TX_COMMIT_MARKER, &marker_rel)? else {
        return Ok(PendingUnpackedTx::None);
    };
    if !std::fs::metadata(&marker).map(|m| m.is_file()).unwrap_or(false) {
        return Ok(PendingUnpackedTx::None);
    }
    let next_dir = validated_tx_subdir(canon_root, &tx_dir, UNPACKED_TX_NEXT_DIR)?
        .ok_or_else(|| FormatError::MissingArchiveEntry { path: format!("{UNPACKED_TX_DIR}/{UNPACKED_TX_NEXT_DIR}/{MANIFEST_PATH}") })?;
    let prev_dir = validated_tx_subdir(canon_root, &tx_dir, UNPACKED_TX_PREV_DIR)?;
    Ok(PendingUnpackedTx::Committing { next_dir, prev_dir })
}

struct VerifiedUnpackedSnapshot {
    manifest: ArchiveManifest,
    entries: Vec<(String, Vec<u8>)>,
    doc: ProjectDocument,
}

fn verify_and_read_snapshot(canon_snap: &Path, limits: ZipLimits, collect_entries: bool) -> Result<VerifiedUnpackedSnapshot, FormatError> {
    let mut remaining_budget = limits.max_total_uncompressed_bytes;
    let manifest_bytes = read_contained_file(canon_snap, MANIFEST_PATH, limits.max_entry_uncompressed_bytes, &mut remaining_budget)?;
    let manifest_text = std::str::from_utf8(&manifest_bytes)
        .map_err(|err| FormatError::InvalidZip { reason: format!("entry `{MANIFEST_PATH}` is not valid UTF-8: {err}") })?;
    let manifest = parse_and_validate_manifest(manifest_text)?;
    if manifest.kind != ArchiveKind::Project {
        return Err(FormatError::InvalidArchiveKind { expected: ArchiveKind::Project.as_str(), found: manifest.kind.as_str() });
    }

    let doc_bytes = read_contained_file(canon_snap, DOCUMENT_PATH, limits.max_entry_uncompressed_bytes, &mut remaining_budget)?;
    let actual_doc_crc = crc32fast::hash(&doc_bytes);
    if actual_doc_crc != manifest.document_crc32 {
        return Err(FormatError::CorruptCrc { entry: String::from(DOCUMENT_PATH), expected: manifest.document_crc32, actual: actual_doc_crc });
    }

    let mut entries = Vec::new();
    if collect_entries {
        entries.push((String::from(MANIFEST_PATH), manifest_bytes));
        entries.push((String::from(DOCUMENT_PATH), doc_bytes.clone()));
    }

    let mut seen_paths = HashSet::new();
    let mut entry_count = 2_usize;
    for entry in &manifest.sheets {
        let Some(csv_path) = &entry.csv_path else {
            continue;
        };
        if !seen_paths.insert(csv_path.as_str()) {
            return Err(FormatError::DuplicateEntry { path: csv_path.clone() });
        }
        entry_count += 1;
        if entry_count > limits.max_entries {
            return Err(FormatError::ZipBomb { reason: format!("unpacked entry count exceeds limit {}", limits.max_entries) });
        }
        let csv_bytes = read_contained_file(canon_snap, csv_path, limits.max_entry_uncompressed_bytes, &mut remaining_budget)?;
        if let Some(expected_crc) = entry.crc32 {
            let actual_crc = crc32fast::hash(&csv_bytes);
            if actual_crc != expected_crc {
                return Err(FormatError::CorruptCrc { entry: csv_path.clone(), expected: expected_crc, actual: actual_crc });
            }
        }
        if collect_entries {
            entries.push((csv_path.clone(), csv_bytes));
        }
    }

    let mut hydrated_assets = HashMap::with_capacity(manifest.assets.len());
    for entry in &manifest.assets {
        if !seen_paths.insert(entry.path.as_str()) {
            return Err(FormatError::DuplicateEntry { path: entry.path.clone() });
        }
        entry_count += 1;
        if entry_count > limits.max_entries {
            return Err(FormatError::ZipBomb { reason: format!("unpacked entry count exceeds limit {}", limits.max_entries) });
        }
        let asset_bytes = read_contained_file(canon_snap, &entry.path, limits.max_entry_uncompressed_bytes, &mut remaining_budget)?;
        if asset_bytes.len() as u64 != entry.byte_len {
            return Err(FormatError::ZipBomb {
                reason: format!("asset `{}` byte length {} disagrees with manifest {}", entry.path, asset_bytes.len(), entry.byte_len),
            });
        }
        let actual_crc = crc32fast::hash(&asset_bytes);
        if actual_crc != entry.crc32 {
            return Err(FormatError::CorruptCrc { entry: entry.path.clone(), expected: entry.crc32, actual: actual_crc });
        }
        if collect_entries {
            entries.push((entry.path.clone(), asset_bytes.clone()));
        }
        hydrated_assets.insert(entry.asset_id.clone(), (entry.media_type, asset_bytes));
    }

    let mut doc = parse_and_migrate_document(&doc_bytes, manifest.schema_version)?;
    hydrate_archive_image_assets(&mut doc, &manifest.assets, hydrated_assets)?;
    validate_project_document(&doc)?;
    Ok(VerifiedUnpackedSnapshot { manifest, entries, doc })
}

fn open_validated_unpacked_snapshot(canon_snap: &Path, limits: ZipLimits) -> Result<ProjectDocument, FormatError> {
    verify_and_read_snapshot(canon_snap, limits, false).map(|snap| snap.doc)
}

fn check_snapshot_project_dir(canon_snap: &Path, verify_doc_crc: bool) -> Result<bool, FormatError> {
    let mut remaining_budget = crate::zip::MAX_ZIP_UNCOMPRESSED_BYTES;
    let manifest_bytes = read_contained_file(canon_snap, MANIFEST_PATH, crate::zip::MAX_ZIP_ENTRY_BYTES, &mut remaining_budget)?;
    let manifest_text = std::str::from_utf8(&manifest_bytes)
        .map_err(|err| FormatError::InvalidZip { reason: format!("entry `{MANIFEST_PATH}` is not valid UTF-8: {err}") })?;
    let manifest = parse_and_validate_manifest(manifest_text)?;
    if manifest.kind != ArchiveKind::Project {
        return Ok(false);
    }
    if verify_doc_crc {
        let doc_bytes = read_contained_file(canon_snap, DOCUMENT_PATH, crate::zip::MAX_ZIP_ENTRY_BYTES, &mut remaining_budget)?;
        return Ok(crc32fast::hash(&doc_bytes) == manifest.document_crc32);
    }
    let doc_path = resolve_contained_path(canon_snap, DOCUMENT_PATH, true)?;
    let doc_meta = std::fs::metadata(doc_path)?;
    Ok(doc_meta.is_file() && doc_meta.len() <= crate::zip::MAX_ZIP_ENTRY_BYTES)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct UnpackedTxCommitRecord {
    format: String,
    version: u32,
    stage_paths: Vec<String>,
    prior_csv_paths: Vec<String>,
    new_csv_paths: Vec<String>,
}

struct TxDirGuard {
    root_dir: PathBuf,
    tx_dir: PathBuf,
    active: bool,
}

impl TxDirGuard {
    fn new(root_dir: &Path, tx_dir: PathBuf) -> Self {
        Self { root_dir: root_dir.to_path_buf(), tx_dir, active: true }
    }

    fn preserve_for_recovery(&mut self) {
        self.active = false;
    }

    fn finish(mut self) -> Result<(), FormatError> {
        self.active = false;
        let marker = self.tx_dir.join(UNPACKED_TX_COMMIT_MARKER);
        if std::fs::symlink_metadata(&marker).is_ok() {
            std::fs::remove_file(&marker)?;
        }
        std::fs::remove_dir_all(&self.tx_dir)?;
        let _ = platform_sync_dir(&self.root_dir);
        Ok(())
    }
}

impl Drop for TxDirGuard {
    fn drop(&mut self) {
        if self.active {
            let marker = self.tx_dir.join(UNPACKED_TX_COMMIT_MARKER);
            let _ = std::fs::remove_file(&marker);
            let _ = std::fs::remove_dir_all(&self.tx_dir);
        }
    }
}

fn write_snapshot_dir(snap_dir: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), FormatError> {
    std::fs::create_dir_all(snap_dir)?;
    for (rel, bytes) in entries {
        validate_unpacked_relative_path(rel)?;
        let target = snap_dir.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(all(unix, not(target_arch = "wasm32")))]
        opts.mode(0o644);
        let mut file = opts.open(&target)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    for subdir in [SHEETS_DIR, ASSETS_DIR] {
        let sub_path = snap_dir.join(subdir);
        if std::fs::symlink_metadata(&sub_path).is_ok_and(|m| m.is_dir()) {
            platform_sync_dir(&sub_path)?;
        }
    }
    platform_sync_dir(snap_dir)
}

fn capture_prior_snapshot_entries(canon_root: &Path, limits: ZipLimits) -> Option<Vec<(String, Vec<u8>)>> {
    verify_and_read_snapshot(canon_root, limits, true).ok().map(|snap| snap.entries)
}

fn begin_unpacked_tx(
    canon_root: &Path,
    entries: &[(String, Vec<u8>)],
    prev_entries: Option<&[(String, Vec<u8>)]>,
) -> Result<TxDirGuard, FormatError> {
    let tx_dir = canon_root.join(UNPACKED_TX_DIR);
    if std::fs::symlink_metadata(&tx_dir).is_ok() {
        std::fs::remove_dir_all(&tx_dir)?;
    }
    std::fs::create_dir_all(&tx_dir)?;
    let guard = TxDirGuard::new(canon_root, tx_dir.clone());
    write_snapshot_dir(&tx_dir.join(UNPACKED_TX_NEXT_DIR), entries)?;
    if let Some(prev) = prev_entries {
        write_snapshot_dir(&tx_dir.join(UNPACKED_TX_PREV_DIR), prev)?;
    }
    platform_sync_dir(&tx_dir)?;
    Ok(guard)
}

fn collect_csv_paths(entries: &[(String, Vec<u8>)]) -> Vec<String> {
    entries.iter().map(|(rel, _)| rel).filter(|rel| validate_tracked_subpath(rel)).cloned().collect()
}

fn write_unpacked_tx_commit_marker(
    canon_root: &Path,
    staged: &[StagedWrite],
    prior_paths: &[String],
    entries: &[(String, Vec<u8>)],
) -> Result<(), FormatError> {
    let tx_dir = canon_root.join(UNPACKED_TX_DIR);
    let record = UnpackedTxCommitRecord {
        format: String::from(UNPACKED_TX_FORMAT),
        version: 1,
        stage_paths: staged.iter().map(|s| s.stage_rel.clone()).collect(),
        prior_csv_paths: prior_paths.to_vec(),
        new_csv_paths: collect_csv_paths(entries),
    };
    let bytes = serde_json::to_vec(&record).map_err(|err| FormatError::Json(format!("serialize unpacked tx marker failed: {err}")))?;
    platform_write_atomic(&tx_dir.join(UNPACKED_TX_COMMIT_MARKER), &bytes)?;
    platform_sync_dir(&tx_dir)
}

fn remove_stage_rel_files(canon_root: &Path, stage_rels: &[String]) -> Result<(), FormatError> {
    for rel in stage_rels {
        validate_unpacked_relative_path(rel)?;
        let path = resolve_contained_path(canon_root, rel, false)?;
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && !meta.file_type().is_symlink()
            && meta.is_file()
        {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

fn install_recovered_snapshot(
    canon_root: &Path,
    entries: &[(String, Vec<u8>)],
    stale_candidates: &[String],
    keep_csv_paths: &[String],
) -> Result<(), FormatError> {
    for (rel, bytes) in entries {
        let target = resolve_contained_path(canon_root, rel, false)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        platform_write_atomic(&target, bytes)?;
    }
    let keep_set: HashSet<String> = keep_csv_paths.iter().cloned().collect();
    prune_stale_unpacked_files(canon_root, stale_candidates, &keep_set)?;
    sync_unpacked_target_dirs(canon_root)
}

fn reconcile_pending_unpacked_tx(canon_root: &Path) -> Result<(), FormatError> {
    let Some(tx_dir) = ensure_non_symlink_subpath(canon_root, UNPACKED_TX_DIR, UNPACKED_TX_DIR)? else {
        return Ok(());
    };
    if !std::fs::metadata(&tx_dir).map(|m| m.is_dir()).unwrap_or(false) {
        return Err(FormatError::Io(format!("`{}` exists and is not a directory", tx_dir.display())));
    }
    let marker_rel = format!("{UNPACKED_TX_DIR}/{UNPACKED_TX_COMMIT_MARKER}");
    let Some(marker) = ensure_non_symlink_subpath(&tx_dir, UNPACKED_TX_COMMIT_MARKER, &marker_rel)? else {
        std::fs::remove_dir_all(&tx_dir)?;
        return platform_sync_dir(canon_root);
    };
    let cap = usize::try_from(crate::zip::MAX_ZIP_ENTRY_BYTES).unwrap_or(usize::MAX);
    let marker_bytes = platform_read_regular_file(&marker, cap)?;
    let record: UnpackedTxCommitRecord =
        serde_json::from_slice(&marker_bytes).map_err(|err| FormatError::Json(format!("unpacked transaction marker could not be parsed: {err}")))?;
    if record.format != UNPACKED_TX_FORMAT || record.version != 1 {
        return Err(FormatError::Io(String::from("unsupported unpacked transaction marker format")));
    }

    remove_stage_rel_files(canon_root, &record.stage_paths)?;
    let next_dir = validated_tx_subdir(canon_root, &tx_dir, UNPACKED_TX_NEXT_DIR)?;
    let prev_dir = validated_tx_subdir(canon_root, &tx_dir, UNPACKED_TX_PREV_DIR)?;

    let mut primary_err: Option<FormatError> = None;
    let recovered = match next_dir {
        Some(ref next) => match verify_and_read_snapshot(next, ZipLimits::default(), true) {
            Ok(snap) => {
                install_recovered_snapshot(canon_root, &snap.entries, &record.prior_csv_paths, &record.new_csv_paths)?;
                true
            }
            Err(err @ (FormatError::UnsupportedSchemaVersion { .. } | FormatError::InvalidArchiveKind { .. } | FormatError::UnsafePath { .. })) => {
                return Err(err);
            }
            Err(err) => {
                primary_err = Some(err);
                false
            }
        },
        None => false,
    };

    if !recovered {
        if let Some(prev) = prev_dir {
            let prev_snap = verify_and_read_snapshot(&prev, ZipLimits::default(), true)?;
            let mut prev_tracked: Vec<String> = prev_snap.manifest.sheets.into_iter().filter_map(|e| e.csv_path).collect();
            prev_tracked.extend(prev_snap.manifest.assets.into_iter().map(|e| e.path));
            install_recovered_snapshot(canon_root, &prev_snap.entries, &record.new_csv_paths, &prev_tracked)?;
        } else if let Some(err) = primary_err {
            return Err(err);
        } else {
            return Err(FormatError::MissingArchiveEntry { path: format!("{UNPACKED_TX_DIR}/{UNPACKED_TX_NEXT_DIR}/{MANIFEST_PATH}") });
        }
    }

    std::fs::remove_file(&marker)?;
    std::fs::remove_dir_all(&tx_dir)?;
    platform_sync_dir(canon_root)
}

fn unpacked_changes_needed(
    canon_root: &Path,
    entries: &[(String, Vec<u8>)],
    prior_paths: &[String],
    expected_paths: &HashSet<String>,
) -> Result<bool, FormatError> {
    if prior_paths.iter().any(|rel| !expected_paths.contains(rel) && validate_tracked_subpath(rel)) {
        return Ok(true);
    }
    for (rel, bytes) in entries {
        let target = resolve_contained_path(canon_root, rel, false)?;
        if !matches!(inspect_existing_unpacked_entry(&target, bytes)?, ExistingEntryState::Unchanged) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn sync_unpacked_target_dirs(canon_root: &Path) -> Result<(), FormatError> {
    for subdir in [SHEETS_DIR, ASSETS_DIR] {
        let sub_dir = canon_root.join(subdir);
        if std::fs::symlink_metadata(&sub_dir).is_ok_and(|m| !m.file_type().is_symlink() && m.is_dir()) {
            platform_sync_dir(&sub_dir)?;
        }
    }
    platform_sync_dir(canon_root)
}

fn collect_prior_manifest_csv_paths(canon_root: &Path) -> Result<Vec<String>, FormatError> {
    let manifest_path = match resolve_contained_path(canon_root, MANIFEST_PATH, true) {
        Ok(path) => path,
        Err(FormatError::MissingArchiveEntry { .. }) => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let cap = usize::try_from(crate::zip::MAX_ZIP_ENTRY_BYTES).unwrap_or(usize::MAX);
    let Ok(bytes) = platform_read_regular_file(&manifest_path, cap) else {
        return Ok(Vec::new());
    };
    if let Some(version) = peek_json_schema_version(&bytes) {
        ensure_schema_version_supported(version)?;
    }
    let Ok(manifest) = serde_json::from_slice::<ArchiveManifest>(&bytes) else {
        return Ok(Vec::new());
    };
    if manifest.format != FORMAT_MAGIC || manifest.kind != ArchiveKind::Project {
        return Ok(Vec::new());
    }
    ensure_schema_version_supported(manifest.schema_version)?;
    let mut tracked: Vec<String> =
        manifest.sheets.into_iter().filter_map(|e| e.csv_path).filter(|rel| validate_sheet_csv_path(rel).is_ok()).collect();
    tracked.extend(manifest.assets.into_iter().map(|e| e.path).filter(|rel| validate_asset_rel_path(rel).is_ok()));
    Ok(tracked)
}

enum PriorSnapshot {
    Existed(Vec<u8>),
    DidNotExist,
}

struct StagedWrite {
    rel: String,
    stage_rel: String,
    target_path: PathBuf,
    stage_path: PathBuf,
    prior: PriorSnapshot,
}

struct UnpackedStageGuard {
    stage_paths: Vec<Option<PathBuf>>,
    active: bool,
}

impl UnpackedStageGuard {
    fn new(len: usize) -> Self {
        Self { stage_paths: Vec::with_capacity(len), active: true }
    }

    fn push(&mut self, path: PathBuf) {
        self.stage_paths.push(Some(path));
    }

    fn take(&mut self, idx: usize) {
        if let Some(slot) = self.stage_paths.get_mut(idx) {
            slot.take();
        }
    }

    #[cfg(test)]
    fn preserve_for_recovery(&mut self) {
        self.active = false;
    }
}

impl Drop for UnpackedStageGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        for path in self.stage_paths.iter().flatten() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn stage_rel_for_entry(entry_rel: &str, stage_path: &Path) -> Result<String, FormatError> {
    let file_name = stage_path.file_name().and_then(|s| s.to_str()).ok_or_else(|| FormatError::UnsafePath { path: entry_rel.to_owned() })?;
    let stage_rel = match entry_rel.rsplit_once('/') {
        Some((dir_prefix, _)) => format!("{dir_prefix}/{file_name}"),
        None => file_name.to_owned(),
    };
    validate_unpacked_relative_path(&stage_rel)?;
    Ok(stage_rel)
}

fn stage_changed_unpacked_entries(canon_root: &Path, entries: &[(String, Vec<u8>)]) -> Result<(Vec<StagedWrite>, UnpackedStageGuard), FormatError> {
    let mut staged = Vec::new();
    let mut guard = UnpackedStageGuard::new(entries.len());

    for (rel, bytes) in entries {
        let target = resolve_contained_path(canon_root, rel, false)?;
        let parent = target.parent().ok_or_else(|| FormatError::UnsafePath { path: rel.to_owned() })?;
        std::fs::create_dir_all(parent)?;
        let canon_parent = std::fs::canonicalize(parent)?;
        if !canon_parent.starts_with(canon_root) {
            return Err(FormatError::UnsafePath { path: rel.to_owned() });
        }

        let (prior, existing_perms) = match inspect_existing_unpacked_entry(&target, bytes)? {
            ExistingEntryState::Unchanged => continue,
            ExistingEntryState::NeedsWrite { prior, perms } => (prior, perms),
        };

        let (stage_path, stage_rel) = stage_unpacked_entry_file(parent, &target, rel, bytes, existing_perms)?;
        guard.push(stage_path.clone());
        staged.push(StagedWrite { rel: rel.clone(), stage_rel, target_path: target, stage_path, prior });
    }

    Ok((staged, guard))
}

enum ExistingEntryState {
    Unchanged,
    NeedsWrite { prior: PriorSnapshot, perms: Option<Permissions> },
}

fn inspect_existing_unpacked_entry(target: &Path, new_bytes: &[u8]) -> Result<ExistingEntryState, FormatError> {
    match std::fs::symlink_metadata(target) {
        Ok(meta) => {
            if !meta.file_type().is_symlink()
                && meta.is_file()
                && meta.len() == new_bytes.len() as u64
                && new_bytes.len() <= MAX_ARCHIVE_FILE_BYTES
                && let Ok(existing) = platform_read_regular_file(target, new_bytes.len())
            {
                if existing == new_bytes {
                    return Ok(ExistingEntryState::Unchanged);
                }
                let perms = std::fs::metadata(target).ok().map(|m| m.permissions());
                return Ok(ExistingEntryState::NeedsWrite { prior: PriorSnapshot::Existed(existing), perms });
            }
            let existing = platform_read_regular_file(target, MAX_ARCHIVE_FILE_BYTES)?;
            let perms = std::fs::metadata(target).ok().map(|m| m.permissions());
            Ok(ExistingEntryState::NeedsWrite { prior: PriorSnapshot::Existed(existing), perms })
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(ExistingEntryState::NeedsWrite { prior: PriorSnapshot::DidNotExist, perms: None })
        }
        Err(err) => Err(FormatError::Io(err.to_string())),
    }
}

fn commit_staged_unpacked_entries_with_rollback<F, R>(
    staged: &[StagedWrite],
    guard: &mut UnpackedStageGuard,
    tx_guard: &mut TxDirGuard,
    on_commit_entry: &mut F,
    mut restore_file: R,
) -> Result<(), FormatError>
where
    F: FnMut(&str) -> MidCommitDirective,
    R: FnMut(&Path, &[u8]) -> Result<(), FormatError>,
{
    for (idx, item) in staged.iter().enumerate() {
        let rename_res = match on_commit_entry(&item.rel) {
            MidCommitDirective::Proceed => std::fs::rename(&item.stage_path, &item.target_path),
            MidCommitDirective::FailRollback(err) => Err(err),
            #[cfg(test)]
            MidCommitDirective::SimulateCrash => {
                guard.preserve_for_recovery();
                tx_guard.preserve_for_recovery();
                return Err(FormatError::Io(String::from("simulated abrupt crash mid-commit")));
            }
        };
        if let Err(err) = rename_res {
            let committed = staged.get(..idx).unwrap_or(staged);
            if let Err(rollback_failures) = rollback_unpacked_commits(committed, &mut restore_file) {
                tx_guard.preserve_for_recovery();
                return Err(FormatError::Io(format!(
                    "unpacked commit failed at `{}` ({err}); rollback also failed ({}); preserved `{UNPACKED_TX_DIR}` recovery snapshot",
                    item.rel,
                    rollback_failures.join("; ")
                )));
            }
            return Err(FormatError::Io(err.to_string()));
        }
        guard.take(idx);
    }
    Ok(())
}

fn rollback_unpacked_commits<R>(committed: &[StagedWrite], restore_file: &mut R) -> Result<(), Vec<String>>
where
    R: FnMut(&Path, &[u8]) -> Result<(), FormatError>,
{
    let mut failures = Vec::new();
    for item in committed.iter().rev() {
        match &item.prior {
            PriorSnapshot::Existed(bytes) => {
                if let Err(err) = restore_file(&item.target_path, bytes) {
                    failures.push(format!("{}: {err}", item.rel));
                }
            }
            PriorSnapshot::DidNotExist => {
                if let Err(err) = std::fs::remove_file(&item.target_path)
                    && err.kind() != std::io::ErrorKind::NotFound
                {
                    failures.push(format!("{}: {err}", item.rel));
                }
            }
        }
    }
    if failures.is_empty() { Ok(()) } else { Err(failures) }
}

fn prune_stale_unpacked_files(canon_root: &Path, prior_paths: &[String], expected_paths: &HashSet<String>) -> Result<(), FormatError> {
    for rel in prior_paths {
        if expected_paths.contains(rel) || !validate_tracked_subpath(rel) {
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

    for subdir in [SHEETS_DIR, ASSETS_DIR] {
        let sub_dir = canon_root.join(subdir);
        let has_outputs = expected_paths.iter().any(|p| p.starts_with(&format!("{subdir}/")));
        if !has_outputs && std::fs::symlink_metadata(&sub_dir).is_ok_and(|m| !m.file_type().is_symlink() && m.is_dir()) {
            // `remove_dir` succeeds only when empty, preserving any user-managed files inside the subdirectory.
            let _ = std::fs::remove_dir(&sub_dir);
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

fn is_windows_reserved_stem(stem: &str) -> bool {
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn unique_sheet_csv_path(idx: usize, id: &SheetId, used_lower: &mut HashSet<String>) -> String {
    let sanitized: String = id.as_str().chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
    let mut stem = if sanitized.is_empty() { format!("sheet_{idx}") } else { sanitized };
    if is_windows_reserved_stem(&stem) {
        stem = format!("sheet_{stem}");
    }
    stem.truncate(MAX_SHEET_CSV_STEM_LEN);
    let mut candidate = format!("{SHEETS_DIR}/{stem}.csv");
    let mut suffix = 1_usize;
    while !used_lower.insert(candidate.to_ascii_lowercase()) {
        candidate = format!("{SHEETS_DIR}/{stem}_{suffix}.csv");
        suffix += 1;
    }
    candidate
}

/// Strip raw cell values and cached result tables to produce a clean reusable `.cbtemplate`.
pub fn strip_project_data_for_template(doc: &ProjectDocument) -> Result<ProjectDocument, FormatError> {
    ensure_schema_version_supported(doc.schema_version)?;
    validate_project_document(doc)?;
    let mut out = doc.clone();
    out.schema_version = CURRENT_SCHEMA_VERSION;
    out.checkpoints.clear();
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
        clean.exclusion_notes.clear();
        *ds = clean;
    }
    for res in &mut out.result_sheets {
        res.status = ResultStatus::Dirty;
        res.analyzable_data = None;
        res.tabs.clear();
        res.scalars.clear();
        res.pairwise_comparisons.clear();
        res.fitted_curves.clear();
        res.eliminated_outliers.clear();
        res.checklist.clear();
    }
    let mut referenced_assets: HashSet<String> = HashSet::new();
    for graph in &out.graph_sheets {
        for ann in &graph.spec.annotations {
            if let GraphAnnotation::EmbeddedImage { asset_id, .. } = ann {
                referenced_assets.insert(asset_id.clone());
            }
        }
    }
    for layout in &out.layout_sheets {
        for item in &layout.spec.items {
            if let LayoutItem::EmbeddedImage { asset_id, .. } = item {
                referenced_assets.insert(asset_id.clone());
            }
        }
    }
    out.image_assets.retain(|asset| referenced_assets.contains(&asset.asset_id));
    Ok(out)
}

const MIGRATION_STEPS: &[VersionStep] = &[
    VersionStep { from_version: 0, apply: migrate_v0_to_v1 },
    VersionStep { from_version: 1, apply: migrate_v1_to_v2 },
    VersionStep { from_version: 2, apply: migrate_v2_to_v3 },
    VersionStep { from_version: 3, apply: migrate_v3_to_v4 },
    VersionStep { from_version: 4, apply: migrate_v4_to_v5 },
    VersionStep { from_version: 5, apply: migrate_v5_to_v6 },
    VersionStep { from_version: 6, apply: migrate_v6_to_v7 },
];

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

fn migrate_result_sheet_step(value: &mut serde_json::Value, schema_version: u32) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(String::from("schema_version"), serde_json::Value::from(schema_version));
        backfill_top_level_fields(obj);
        if let Some(result_sheets) = obj.get_mut("result_sheets").and_then(serde_json::Value::as_array_mut) {
            for res in result_sheets {
                backfill_result_sheet_defaults(res);
            }
        }
    }
}

fn migrate_v1_to_v2(value: &mut serde_json::Value) -> Result<(), FormatError> {
    migrate_result_sheet_step(value, 2);
    Ok(())
}

fn migrate_v2_to_v3(value: &mut serde_json::Value) -> Result<(), FormatError> {
    migrate_result_sheet_step(value, 3);
    Ok(())
}

fn migrate_v3_to_v4(value: &mut serde_json::Value) -> Result<(), FormatError> {
    migrate_result_sheet_step(value, 4);
    Ok(())
}

fn migrate_v4_to_v5(value: &mut serde_json::Value) -> Result<(), FormatError> {
    migrate_result_sheet_step(value, 5);
    Ok(())
}

fn migrate_v5_to_v6(value: &mut serde_json::Value) -> Result<(), FormatError> {
    migrate_result_sheet_step(value, 6);
    Ok(())
}

fn migrate_v6_to_v7(value: &mut serde_json::Value) -> Result<(), FormatError> {
    migrate_result_sheet_step(value, 7);
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
    obj.entry(String::from("image_assets")).or_insert_with(|| serde_json::Value::Array(Vec::new()));
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
    for arr_key in ["tabs", "scalars", "pairwise_comparisons", "fitted_curves", "eliminated_outliers", "checklist", "floating_notes"] {
        res_obj.entry(String::from(arr_key)).or_insert_with(|| serde_json::Value::Array(Vec::new()));
    }
    res_obj.entry(String::from("color_tag")).or_insert(serde_json::Value::Null);
    let Some(analysis) = res_obj.get_mut("analysis").and_then(serde_json::Value::as_object_mut) else {
        return;
    };
    let Some(kind) = analysis.get_mut("kind").and_then(serde_json::Value::as_object_mut) else {
        return;
    };
    if let Some(fit) = kind.get_mut("LinearRegression").and_then(serde_json::Value::as_object_mut) {
        for key in ["x_range_filter", "plot_x_min", "plot_x_max", "plot_segments"] {
            fit.entry(String::from(key)).or_insert(serde_json::Value::Null);
        }
    }
    if let Some(fit) = kind.get_mut("NonlinearRegression").and_then(serde_json::Value::as_object_mut) {
        for opt_key in ["x_range_filter", "y_range_filter", "plot_x_min", "plot_x_max", "mv_y_column", "flag_r_squared_below"] {
            fit.entry(String::from(opt_key)).or_insert(serde_json::Value::Null);
        }
        for vec_key in ["mv_group_columns", "mv_x_columns"] {
            fit.entry(String::from(vec_key)).or_insert_with(|| serde_json::Value::Array(Vec::new()));
        }
        for bool_key in ["plot_function_only", "homoscedasticity_test", "emit_residuals_tab", "emit_covariance_tab", "identify_unstable_parameters"] {
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
    if let Some(one_way) = kind.get_mut("OneWayAnova").and_then(serde_json::Value::as_object_mut) {
        one_way.entry(String::from("mixed_effects_reml")).or_insert(serde_json::Value::Bool(false));
        one_way.entry(String::from("df_method")).or_insert_with(|| serde_json::Value::String(String::from("KenwardRoger")));
        one_way.entry(String::from("residual_covariance")).or_insert_with(|| serde_json::Value::String(String::from("CompoundSymmetry")));
    }
    if let Some(fac) = kind.get_mut("FactorialAnova").and_then(serde_json::Value::as_object_mut) {
        fac.entry(String::from("mixed_effects_reml")).or_insert(serde_json::Value::Bool(false));
        fac.entry(String::from("df_method")).or_insert_with(|| serde_json::Value::String(String::from("KenwardRoger")));
        fac.entry(String::from("residual_covariance")).or_insert_with(|| serde_json::Value::String(String::from("CompoundSymmetry")));
    }
    if let Some(nested) = kind.get_mut("NestedComparison").and_then(serde_json::Value::as_object_mut) {
        nested.entry(String::from("df_method")).or_insert_with(|| serde_json::Value::String(String::from("KenwardRoger")));
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

/// Enforce all `curvebook-doc` structural bounds, finite-float invariants, and DAG cycle absence.
pub fn validate_project_document(doc: &ProjectDocument) -> Result<(), FormatError> {
    validate_project_document_inner(doc, true)
}

fn validate_project_document_inner(doc: &ProjectDocument, validate_checkpoint_entries: bool) -> Result<(), FormatError> {
    validate_title(&doc.title)?;
    let total = doc.total_sheet_count();
    if total > MAX_SHEETS_PER_PROJECT {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManySheets { count: total, max: MAX_SHEETS_PER_PROJECT }));
    }

    let mut seen_ids = HashSet::with_capacity(total);
    for ds in &doc.data_sheets {
        register_unique_sheet_id(&mut seen_ids, &ds.id)?;
        validate_data_sheet_floats(ds)?;
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
        if let Some(field) = res.analysis.first_non_finite_float() {
            return Err(FormatError::NonFiniteFloat { location: format!("{}.{field}", res.id.as_str()) });
        }
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

    if doc.image_assets.len() > MAX_PROJECT_IMAGE_ASSETS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyImageAssets { count: doc.image_assets.len(), max: MAX_PROJECT_IMAGE_ASSETS }));
    }
    let mut seen_asset_ids = HashSet::with_capacity(doc.image_assets.len());
    for asset in &doc.image_assets {
        if !seen_asset_ids.insert(asset.asset_id.as_str()) {
            return Err(FormatError::DuplicateEntry { path: asset.asset_id.clone() });
        }
        validate_image_asset(asset)?;
    }

    if validate_checkpoint_entries {
        validate_checkpoints(doc)?;
    }
    detect_dependency_cycles(doc)
}

pub(crate) fn validate_data_sheet_floats(ds: &DataSheet) -> Result<(), FormatError> {
    validate_data_sheet(ds)?;
    validate_archetype_extra_floats(&ds.archetype, ds.id.as_str())?;
    validate_floating_notes(&ds.floating_notes, ds.id.as_str())
}

fn validate_checkpoints(doc: &ProjectDocument) -> Result<(), FormatError> {
    if doc.checkpoints.len() > MAX_CHECKPOINTS_PER_PROJECT {
        return Err(FormatError::Json(format!("checkpoint count {} exceeds maximum {MAX_CHECKPOINTS_PER_PROJECT}", doc.checkpoints.len())));
    }
    let max_total_bytes = usize::try_from(crate::zip::MAX_ZIP_ENTRY_BYTES).unwrap_or(usize::MAX);
    let mut total_snapshot_bytes = 0_usize;
    let mut seen_ids = HashSet::with_capacity(doc.checkpoints.len());
    for cp in &doc.checkpoints {
        if cp.id.trim().is_empty() {
            return Err(FormatError::Doc(curvebook_doc::DocError::EmptyIdentifier));
        }
        validate_title(&cp.id)?;
        validate_title(&cp.name)?;
        validate_title(&cp.note)?;
        validate_title(&cp.created_utc)?;
        if !seen_ids.insert(cp.id.as_str()) {
            return Err(FormatError::Json(format!("duplicate checkpoint id `{}`", cp.id)));
        }
        total_snapshot_bytes = total_snapshot_bytes.saturating_add(cp.snapshot_json.len());
        if total_snapshot_bytes > max_total_bytes {
            return Err(FormatError::ZipBomb { reason: format!("checkpoint snapshot_json total size exceeds limit {max_total_bytes}") });
        }
        let snapshot_doc = decode_and_validate_document_inner(cp.snapshot_json.as_bytes(), CURRENT_SCHEMA_VERSION, false)?;
        if !snapshot_doc.checkpoints.is_empty() {
            return Err(FormatError::Json(format!("checkpoint `{}` snapshot_json must not contain nested checkpoints", cp.id)));
        }
    }
    Ok(())
}

fn check_finite(value: f64, location: impl FnOnce() -> String) -> Result<(), FormatError> {
    if value.is_finite() { Ok(()) } else { Err(FormatError::NonFiniteFloat { location: location() }) }
}

fn check_opt_finite(value: Option<f64>, location: impl FnOnce() -> String) -> Result<(), FormatError> {
    if let Some(v) = value { check_finite(v, location) } else { Ok(()) }
}

fn validate_floating_notes(notes: &[curvebook_doc::FloatingNote], sheet_id: &str) -> Result<(), FormatError> {
    for (idx, note) in notes.iter().enumerate() {
        let b = note.bounds_pt;
        if !b.x.is_finite() || !b.y.is_finite() || !b.width.is_finite() || !b.height.is_finite() {
            return Err(FormatError::NonFiniteFloat { location: format!("{sheet_id}.floating_notes[{idx}].bounds_pt") });
        }
    }
    Ok(())
}

fn validate_archetype_extra_floats(arch: &curvebook_doc::TableArchetype, ctx: &str) -> Result<(), FormatError> {
    if let curvebook_doc::TableArchetype::Grouped(t) = arch
        && let Some(spacing) = &t.row_numeric_spacing
    {
        for (row, maybe_v) in spacing.iter().enumerate() {
            check_opt_finite(*maybe_v, || format!("{ctx}.row_numeric_spacing[{row}]"))?;
        }
    }
    Ok(())
}

fn register_unique_sheet_id(seen: &mut HashSet<SheetId>, id: &SheetId) -> Result<(), FormatError> {
    if id.as_str().trim().is_empty() {
        return Err(FormatError::Doc(curvebook_doc::DocError::EmptyIdentifier));
    }
    if !seen.insert(id.clone()) {
        return Err(FormatError::Doc(curvebook_doc::DocError::DuplicateSheetId { sheet_id: id.clone() }));
    }
    Ok(())
}

fn validate_data_sheet(ds: &DataSheet) -> Result<(), FormatError> {
    ds.validate()?;
    let ctx = ds.id.as_str();
    match &ds.archetype {
        TableArchetype::Xy(t) => {
            if let curvebook_doc::XColumnMode::GeneratedSequence { start, step, .. } = t.x_mode
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
    validate_exclusion_notes(ds)
}

fn validate_exclusion_notes(ds: &DataSheet) -> Result<(), FormatError> {
    let ctx = ds.id.as_str();
    let max_notes = ds.archetype.allocated_cell_slots();
    if ds.exclusion_notes.len() > max_notes {
        return Err(FormatError::Json(format!("{ctx}.exclusion_notes count {} exceeds allocated cell slots {max_notes}", ds.exclusion_notes.len())));
    }
    let mut seen_coords = HashSet::with_capacity(ds.exclusion_notes.len());
    for entry in &ds.exclusion_notes {
        validate_title(&entry.reason_tag)?;
        validate_title(&entry.note)?;
        validate_title(&entry.timestamp_utc)?;
        if !seen_coords.insert(entry.coord) {
            return Err(FormatError::Json(format!(
                "{ctx}.exclusion_notes contains duplicate entry for coordinate ({}, {}, {})",
                entry.coord.row, entry.coord.col, entry.coord.subcol
            )));
        }
        let Some(cell) = ds.archetype.data_cell(entry.coord) else {
            return Err(FormatError::Doc(curvebook_doc::DocError::CellOutOfBounds {
                row: entry.coord.row,
                col: entry.coord.col,
                subcol: entry.coord.subcol,
            }));
        };
        if !cell.excluded {
            return Err(FormatError::Json(format!(
                "{ctx}.exclusion_notes references unexcluded cell at ({}, {}, {})",
                entry.coord.row, entry.coord.col, entry.coord.subcol
            )));
        }
    }
    Ok(())
}

fn validate_titles(titles: &[String]) -> Result<(), FormatError> {
    if titles.len() > MAX_ROWS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyRows { count: titles.len(), max: MAX_ROWS }));
    }
    for title in titles {
        validate_title(title)?;
    }
    Ok(())
}

fn validate_dataset_columns(cols: &[DataSetColumn], ctx: &str) -> Result<(), FormatError> {
    if cols.len() > MAX_DATA_SETS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyDataSets { count: cols.len(), max: MAX_DATA_SETS }));
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
            return Err(FormatError::Doc(curvebook_doc::DocError::TooManySubcolumns { count: row.len(), max: max_subs }));
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
        if let AnalysisKind::PowerAnalysis(p) = &res.analysis.kind
            && let Some(info_id) = &p.info_sheet_id
        {
            deps.push(info_id.clone());
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
    if let Some(cld) = &gs.spec.compact_letter_display {
        deps.push(cld.result_sheet_id.clone());
    }
    for ann in &gs.spec.annotations {
        match ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                deps.push(result_sheet_id.clone());
            }
            GraphAnnotation::TextBox { .. }
            | GraphAnnotation::LineOrArrow { .. }
            | GraphAnnotation::Shape { .. }
            | GraphAnnotation::EmbeddedImage { .. } => {}
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
            LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => {}
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
                    let mut cycle = stack.get(start..).unwrap_or_default().to_vec();
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

    while let Some((before, after_open)) = rest.split_once("{{") {
        out.push_str(before);
        let Some((inner, after_close)) = after_open.split_once("}}") else {
            out.push_str("{{");
            out.push_str(after_open);
            rest = "";
            break;
        };
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
        rest = after_close;
    }
    out.push_str(rest);
    if removed > 0 {
        *text = out;
    }
    removed
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use curvebook_doc::{CURRENT_SCHEMA_VERSION, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, TableArchetype};

    use super::{
        DOCUMENT_PATH, MANIFEST_PATH, MidCommitDirective, UNPACKED_TX_COMMIT_MARKER, UNPACKED_TX_DIR, UNPACKED_TX_NEXT_DIR, UNPACKED_TX_PREV_DIR,
        is_unpacked_project_dir, open_project_unpacked_dir, save_project_unpacked_dir, save_project_unpacked_dir_driven,
        save_project_unpacked_dir_driven_with_rollback, save_project_unpacked_dir_with_hook,
    };

    fn test_col_sheet(id: &str, title: &str, val: f64) -> DataSheet {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
        col.data_sets[0].cells[0] = vec![curvebook_doc::Cell::number(val)];
        DataSheet::new(id, title, TableArchetype::Column(col)).unwrap()
    }

    #[test]
    fn unpacked_save_rolls_back_committed_files_on_mid_commit_failure() {
        let dir = std::env::temp_dir().join(format!("cb-unpacked-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut doc_v1 = ProjectDocument::new("Version One").unwrap();
        doc_v1.add_data_sheet(test_col_sheet("sheet-a", "A", 10.0)).unwrap();
        save_project_unpacked_dir(&doc_v1, &dir).unwrap();

        let mut doc_v2 = doc_v1.clone();
        doc_v2.title = String::from("Version Two");
        doc_v2.add_data_sheet(test_col_sheet("sheet-b", "B", 20.0)).unwrap();

        // Simulate failure right before committing `manifest.json` (after `document.json` and new CSV committed).
        let err = save_project_unpacked_dir_with_hook(&doc_v2, &dir, |rel| {
            if rel == MANIFEST_PATH { Err(std::io::Error::other("simulated disk failure")) } else { Ok(()) }
        })
        .unwrap_err();
        assert!(matches!(err, crate::error::FormatError::Io(_)));

        // Re-opening must recover intact `doc_v1` with zero CRC mismatch, zero leaked `sheet-b.csv`, and zero stage temps.
        let reloaded = open_project_unpacked_dir(&dir).unwrap();
        assert_eq!(reloaded, doc_v1);
        assert!(!dir.join("sheets/sheet-b.csv").exists());
        assert!(!dir.join(UNPACKED_TX_DIR).exists());
        for entry in std::fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(!name.contains(".stage."), "leaked staging temp file: {name}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpacked_save_crash_between_document_and_manifest_recovers_on_reopen_and_resave() {
        let dir = std::env::temp_dir().join(format!("cb-unpacked-crash-tx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut doc_v1 = ProjectDocument::new("Version One").unwrap();
        doc_v1.add_data_sheet(test_col_sheet("sheet-a", "A", 10.0)).unwrap();
        doc_v1.add_data_sheet(test_col_sheet("sheet-c", "C", 30.0)).unwrap();
        save_project_unpacked_dir(&doc_v1, &dir).unwrap();

        let mut doc_v2 = ProjectDocument::new("Version Two").unwrap();
        doc_v2.add_data_sheet(test_col_sheet("sheet-a", "A", 10.0)).unwrap();
        doc_v2.add_data_sheet(test_col_sheet("sheet-b", "B", 20.0)).unwrap();

        // Abruptly crash right after `document.json` and `sheets/sheet-b.csv` rename, BEFORE `manifest.json` rename & pruning.
        let err = save_project_unpacked_dir_driven(&doc_v2, &dir, |rel| {
            if rel == MANIFEST_PATH { MidCommitDirective::SimulateCrash } else { MidCommitDirective::Proceed }
        })
        .unwrap_err();
        assert!(matches!(err, crate::error::FormatError::Io(_)));

        // Verify the torn on-disk state matches the bug window: new document.json vs old manifest.json.
        let disk_doc = std::fs::read_to_string(dir.join(DOCUMENT_PATH)).unwrap();
        let disk_manifest = std::fs::read_to_string(dir.join(MANIFEST_PATH)).unwrap();
        assert!(disk_doc.contains("Version Two"));
        assert!(disk_manifest.contains("Version One"));
        assert!(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_COMMIT_MARKER).exists());

        // Reopen must recover a complete valid generation (`doc_v2`) repeatedly without CorruptCrc.
        assert!(is_unpacked_project_dir(&dir));
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v2);
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v2);

        // Subsequent save reconciles the interrupted transaction, prunes stale `sheet-c.csv`, and cleans all tx/stage artifacts.
        let mut doc_v3 = doc_v2.clone();
        doc_v3.title = String::from("Version Three");
        save_project_unpacked_dir(&doc_v3, &dir).unwrap();

        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v3);
        assert!(!dir.join(UNPACKED_TX_DIR).exists());
        assert!(!dir.join("sheets/sheet-c.csv").exists());
        for entry in std::fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(!name.contains(".stage."), "leaked staging temp file after recovery: {name}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpacked_save_crash_falls_back_to_prev_and_recovers_initial_commit() {
        let dir = std::env::temp_dir().join(format!("cb-unpacked-crash-fallback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // Part A: crash on initial commit (no prior `prev/` snapshot exists) still recovers Gen 1 from `next/`.
        let mut doc_v1 = ProjectDocument::new("Initial Gen").unwrap();
        doc_v1.add_data_sheet(test_col_sheet("sheet-a", "A", 10.0)).unwrap();
        let _ = save_project_unpacked_dir_driven(&doc_v1, &dir, |rel| {
            if rel == MANIFEST_PATH { MidCommitDirective::SimulateCrash } else { MidCommitDirective::Proceed }
        })
        .unwrap_err();
        assert!(is_unpacked_project_dir(&dir));
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v1);
        save_project_unpacked_dir(&doc_v1, &dir).unwrap();
        assert!(!dir.join(UNPACKED_TX_DIR).exists());

        // Part B: crash updating to `doc_v2`, then corrupt `tx/next/document.json` -> must fall back cleanly to `tx/prev/` (`doc_v1`).
        let mut doc_v2 = doc_v1.clone();
        doc_v2.title = String::from("Second Gen");
        doc_v2.add_data_sheet(test_col_sheet("sheet-b", "B", 25.0)).unwrap();
        let _ = save_project_unpacked_dir_driven(&doc_v2, &dir, |rel| {
            if rel == MANIFEST_PATH { MidCommitDirective::SimulateCrash } else { MidCommitDirective::Proceed }
        })
        .unwrap_err();
        std::fs::write(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_NEXT_DIR).join(DOCUMENT_PATH), b"{corrupt").unwrap();
        assert!(is_unpacked_project_dir(&dir));
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v1);

        // Re-saving reconciles cleanly to `doc_v1` and removes orphaned `sheet-b.csv`.
        save_project_unpacked_dir(&doc_v1, &dir).unwrap();
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v1);
        assert!(!dir.join("sheets/sheet-b.csv").exists());
        assert!(!dir.join(UNPACKED_TX_DIR).exists());

        // Part C: pre-commit `.curvebook.tx/next` with future schema version must block overwrite & preserve tx state.
        let future_dir = dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_NEXT_DIR);
        std::fs::create_dir_all(&future_dir).unwrap();
        let future_ver = CURRENT_SCHEMA_VERSION + 1;
        std::fs::write(
            future_dir.join(MANIFEST_PATH),
            format!(
                r#"{{"format":"curvebook-archive","schema_version":{future_ver},"kind":"project","title":"Future","document_crc32":0,"sheets":[]}}"#
            ),
        )
        .unwrap();
        assert!(matches!(
            save_project_unpacked_dir(&doc_v1, &dir),
            Err(crate::error::FormatError::UnsupportedSchemaVersion { version, .. }) if version == future_ver
        ));

        // Removing the pending future tx leaves intact `doc_v1` (with only a benign non-COMMITTING folder ignored if removed).
        std::fs::remove_dir_all(dir.join(UNPACKED_TX_DIR)).unwrap();
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unpacked_save_preserves_tx_snapshots_and_reports_error_when_rollback_fails() {
        let dir = std::env::temp_dir().join(format!("cb-unpacked-rollback-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut doc_v1 = ProjectDocument::new("Version One").unwrap();
        doc_v1.add_data_sheet(test_col_sheet("sheet-a", "A", 10.0)).unwrap();
        save_project_unpacked_dir(&doc_v1, &dir).unwrap();

        let mut doc_v2 = doc_v1.clone();
        doc_v2.title = String::from("Version Two");
        doc_v2.add_data_sheet(test_col_sheet("sheet-b", "B", 20.0)).unwrap();

        // Commit succeeds for `document.json`, then fails at `manifest.json`, and rollback fails when rewriting `document.json`.
        let err = save_project_unpacked_dir_driven_with_rollback(
            &doc_v2,
            &dir,
            |rel| {
                if rel == MANIFEST_PATH {
                    MidCommitDirective::FailRollback(std::io::Error::other("simulated manifest rename failure"))
                } else {
                    MidCommitDirective::Proceed
                }
            },
            |path, _bytes| Err(crate::error::FormatError::Io(format!("simulated read-only rollback failure on {}", path.display()))),
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("simulated manifest rename failure"), "error must report commit failure: {msg}");
        assert!(msg.contains("rollback also failed"), "error must report rollback failure: {msg}");
        assert!(msg.contains("simulated read-only rollback failure"), "error must include rollback cause: {msg}");

        // Commit marker and snapshots MUST remain preserved on disk so reopen recovers a complete generation cleanly.
        assert!(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_COMMIT_MARKER).is_file());
        assert!(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_NEXT_DIR).join(MANIFEST_PATH).is_file());
        assert!(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_PREV_DIR).join(MANIFEST_PATH).is_file());
        assert!(is_unpacked_project_dir(&dir));
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v2);

        // Even if `next/` were later unreadable, `prev/` recovery still yields complete `doc_v1`.
        let orig_next_doc = std::fs::read(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_NEXT_DIR).join(DOCUMENT_PATH)).unwrap();
        std::fs::write(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_NEXT_DIR).join(DOCUMENT_PATH), b"{corrupt").unwrap();
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v1);
        std::fs::write(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_NEXT_DIR).join(DOCUMENT_PATH), orig_next_doc).unwrap();

        // Subsequent save reconciles the pending transaction and cleans `.curvebook.tx/`.
        let mut doc_v3 = doc_v2.clone();
        doc_v3.title = String::from("Version Three");
        save_project_unpacked_dir(&doc_v3, &dir).unwrap();
        assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v3);
        assert!(!dir.join(UNPACKED_TX_DIR).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn unpacked_save_preserves_tx_when_root_becomes_readonly_after_document_publish() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = std::env::temp_dir().join(format!("cb-unpacked-ro-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut doc_v1 = ProjectDocument::new("Gen 1").unwrap();
        doc_v1.add_data_sheet(test_col_sheet("sheet-a", "A", 1.0)).unwrap();
        save_project_unpacked_dir(&doc_v1, &dir).unwrap();

        let mut doc_v2 = doc_v1.clone();
        doc_v2.title = String::from("Gen 2");
        if let TableArchetype::Column(ref mut c) = doc_v2.data_sheets[0].archetype {
            c.data_sets[0].cells[0] = vec![curvebook_doc::Cell::number(2.0)];
        }

        let dir_clone = dir.clone();
        let res = save_project_unpacked_dir_driven(&doc_v2, &dir, move |rel| {
            if rel == MANIFEST_PATH {
                let _ = std::fs::set_permissions(&dir_clone, std::fs::Permissions::from_mode(0o555));
            }
            MidCommitDirective::Proceed
        });

        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
        if let Err(err) = res {
            // Non-root UID: `manifest.json` rename AND `document.json` rollback both failed with PermissionDenied.
            let msg = err.to_string();
            assert!(msg.contains("rollback also failed"), "expected rollback failure in error, got: {msg}");
            assert!(dir.join(UNPACKED_TX_DIR).join(UNPACKED_TX_COMMIT_MARKER).is_file());
            assert!(is_unpacked_project_dir(&dir));
            assert_eq!(open_project_unpacked_dir(&dir).unwrap(), doc_v2);
            save_project_unpacked_dir(&doc_v2, &dir).unwrap();
            assert!(!dir.join(UNPACKED_TX_DIR).exists());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_archive_entries_normalizes_older_version_identically_to_current() {
        use super::{ArchiveKind, build_archive_entries};

        let mut current = ProjectDocument::new("Borrow Path").unwrap();
        current.add_data_sheet(test_col_sheet("sheet-a", "A", 42.0)).unwrap();
        assert_eq!(current.schema_version, CURRENT_SCHEMA_VERSION);

        let mut legacy = current.clone();
        legacy.schema_version = 0;

        let current_entries = build_archive_entries(&current, ArchiveKind::Project).unwrap();
        let legacy_entries = build_archive_entries(&legacy, ArchiveKind::Project).unwrap();
        assert_eq!(current_entries, legacy_entries);
    }

    #[test]
    fn inspect_existing_unpacked_entry_reuses_same_size_buffer_and_handles_missing() {
        use super::{ExistingEntryState, PriorSnapshot, inspect_existing_unpacked_entry};

        let dir = std::env::temp_dir().join(format!("cb-inspect-entry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("entry.json");

        assert!(matches!(
            inspect_existing_unpacked_entry(&path, b"abcd").unwrap(),
            ExistingEntryState::NeedsWrite { prior: PriorSnapshot::DidNotExist, perms: None }
        ));

        std::fs::write(&path, b"abcd").unwrap();
        assert!(matches!(inspect_existing_unpacked_entry(&path, b"abcd").unwrap(), ExistingEntryState::Unchanged));

        match inspect_existing_unpacked_entry(&path, b"wxyz").unwrap() {
            ExistingEntryState::NeedsWrite { prior: PriorSnapshot::Existed(bytes), perms } => {
                assert_eq!(bytes, b"abcd");
                assert!(perms.is_some());
            }
            _ => panic!("expected NeedsWrite with prior bytes on same-size change"),
        }

        match inspect_existing_unpacked_entry(&path, b"longer").unwrap() {
            ExistingEntryState::NeedsWrite { prior: PriorSnapshot::Existed(bytes), .. } => {
                assert_eq!(bytes, b"abcd");
            }
            _ => panic!("expected NeedsWrite with prior bytes on size change"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
