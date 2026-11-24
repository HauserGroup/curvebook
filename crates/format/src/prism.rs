//! Clean-room `.prism` ZIP container importer.
//!
//! Reads bounded ZIP containers carrying CSV data tables, optional JSON sheet/table/info
//! metadata, or project JSON manifests, emitting non-fatal [`ImportWarning`] diagnostics for
//! unrecognized binary or script chunks instead of panicking.

use std::collections::{BTreeMap, HashSet};

use curvebook_doc::{
    AnalysisKind, ConstantValue, DataSheet, FloatingNote, GraphAnnotation, HookSource, InfoConstant, InfoSheet, LayoutItem, LinkSource,
    MAX_IMAGE_ASSET_BYTES, MAX_PROJECT_IMAGE_ASSETS, NormalizeAnchor, ParameterScope, ProjectDocument, SheetId, SubcolumnMode, TableKind,
    TransformAnalysisSpec, extract_template_hooks, is_valid_asset_id,
};

use crate::archive::{
    BoundedJsonOutcome, StreamedDocumentOutcome, clamp_peek_schema_version, finalize_migrated_document, migrate_document_json,
    parse_bounded_json_outcome, parse_streamed_document_outcome, validate_project_document,
};
use crate::csv::{CsvOptions, import_csv_to_datasheet_with_budget, parse_delimited_records};
use crate::error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
use crate::image_asset::validate_image_asset_parts;
use crate::pzfx::{allocate_valid_unique_sheet_id, clamp_title, import_pzfx_bytes_with_used_cells};
use crate::zip::{ZipArchive, ZipLimits};

/// Maximum cumulative dense `Cell` entries allocated across all tables of a `.prism` import.
pub const MAX_PROJECT_CELLS: usize = crate::csv::MAX_IMPORT_CELLS;

/// Optional metadata sidecar describing a tabular CSV sheet inside a `.prism` ZIP archive.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PrismTableMeta {
    #[serde(alias = "sheet_id", alias = "sheetId")]
    id: Option<String>,
    #[serde(alias = "name")]
    title: Option<String>,
    #[serde(alias = "table_type", alias = "tableType", alias = "archetype")]
    kind: Option<String>,
    #[serde(alias = "subcolumnMode", alias = "y_format", alias = "yFormat", alias = "error_format")]
    subcolumn_mode: Option<String>,
    #[serde(alias = "replicate_count", alias = "replicateCount")]
    replicates: Option<usize>,
    delimiter: Option<char>,
    #[serde(alias = "decimalSeparator")]
    decimal_separator: Option<char>,
    #[serde(alias = "hasRowTitles")]
    has_row_titles: Option<bool>,
    #[serde(alias = "csvPath", alias = "path", alias = "data_path", alias = "dataPath", alias = "file")]
    csv_path: Option<String>,
}

/// Optional top-level `.prism` manifest JSON or external `document.json`.
#[derive(Debug, Clone, Default)]
struct PrismManifest {
    title: Option<String>,
    sheets: Vec<(String, PrismTableMeta)>,
    info_sheets: Vec<PrismInfoMeta>,
    unsupported_sections: Vec<(ImportWarningKind, String, String)>,
}

/// Optional info metadata JSON inside a `.prism` ZIP archive.
#[derive(Debug, Clone, Default)]
struct PrismInfoMeta {
    id: Option<String>,
    title: Option<String>,
    notes: Option<String>,
    constants: Vec<ParsedInfoConstantEntry>,
}

#[derive(Debug, Clone)]
struct ParsedInfoConstantEntry {
    name: String,
    value: ConstantValue,
    units: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReferencedCsvPath {
    lookup_key: String,
    raw_display: String,
}

/// Import a `.prism` ZIP container from raw bytes using [`ZipLimits::default`].
pub fn import_prism_bytes(bytes: &[u8]) -> Result<ImportReport, FormatError> {
    import_prism_bytes_with_limits(bytes, ZipLimits::default())
}

enum ProjectJsonLoadOutcome {
    Absent,
    Embedded(Box<ImportReport>),
    ExternalMeta { path: String, value: serde_json::Value },
    InvalidJsonChunk(String),
}

struct TableMetaIndex {
    by_csv: BTreeMap<String, PrismTableMeta>,
    by_stem: BTreeMap<String, PrismTableMeta>,
    by_dir: BTreeMap<String, PrismTableMeta>,
    expected_csv_refs: Vec<ReferencedCsvPath>,
}

impl TableMetaIndex {
    fn new() -> Self {
        Self { by_csv: BTreeMap::new(), by_stem: BTreeMap::new(), by_dir: BTreeMap::new(), expected_csv_refs: Vec::new() }
    }
}

/// Import a `.prism` ZIP container enforcing explicit [`ZipLimits`].
pub fn import_prism_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<ImportReport, FormatError> {
    let archive = ZipArchive::from_bytes_with_limits(bytes, limits)?;
    let root_prefix = detect_archive_root_prefix(&archive);
    let manifest_entry = find_archive_entry_ci(&archive, "manifest.json", &root_prefix);
    let manifest_state = inspect_manifest_bounded(manifest_entry)?;
    let mut warnings = Vec::new();
    let mut handled_paths: HashSet<String> = HashSet::new();

    let mut parsed_manifest = match try_load_project_json(&archive, &root_prefix, &mut warnings)? {
        ProjectJsonLoadOutcome::Embedded(report) => return Ok(*report),
        ProjectJsonLoadOutcome::ExternalMeta { path, value } => {
            handled_paths.insert(path.clone());
            Some(parse_external_project_meta(&value, &path, &mut warnings))
        }
        ProjectJsonLoadOutcome::InvalidJsonChunk(path) => {
            handled_paths.insert(path);
            None
        }
        ProjectJsonLoadOutcome::Absent => None,
    };

    if let Some(manifest_from_file) = apply_manifest_parse_state(manifest_state, &mut handled_paths, &mut warnings) {
        match parsed_manifest {
            Some(doc_fallback) => {
                let mut merged = manifest_from_file;
                merge_manifest_fallback(&mut merged, doc_fallback);
                parsed_manifest = Some(merged);
            }
            None => parsed_manifest = Some(manifest_from_file),
        }
    }

    let raw_proj_title =
        parsed_manifest.as_ref().and_then(|m| m.title.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| String::from("Imported Project"));
    let proj_title = clamp_title(&raw_proj_title, "project", &mut warnings);
    let mut doc = ProjectDocument::new(proj_title)?;
    let mut used_ids: HashSet<String> = HashSet::new();
    let mut meta_index = TableMetaIndex::new();

    if let Some(manifest) = &parsed_manifest {
        register_manifest_entries(manifest, &mut doc, &mut used_ids, &mut warnings, &mut meta_index)?;
    }

    import_json_pass(&archive, &root_prefix, &mut handled_paths, &mut doc, &mut used_ids, &mut warnings, &mut meta_index)?;
    let mut used_cells = import_csv_pass(&archive, &root_prefix, &mut handled_paths, &mut doc, &mut used_ids, &mut warnings, &meta_index)?;
    import_pzfx_and_tail_passes(&archive, &mut handled_paths, &mut doc, &mut used_ids, &mut warnings, &mut used_cells)?;

    if let Some(primary_info) = doc.info_sheets.first().map(|s| s.id.clone()) {
        for ds in &mut doc.data_sheets {
            if ds.info_sheet_id.is_none() {
                ds.info_sheet_id = Some(primary_info.clone());
            }
        }
    }

    validate_project_document(&doc)?;
    Ok(ImportReport { document: doc, warnings })
}

fn apply_manifest_parse_state(
    manifest_state: ManifestParseState,
    handled_paths: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Option<PrismManifest> {
    match manifest_state {
        ManifestParseState::Absent => None,
        ManifestParseState::ValidValue(entry_path, val) => {
            handled_paths.insert(entry_path.clone());
            Some(parse_external_project_meta(&val, &entry_path, warnings))
        }
        ManifestParseState::InvalidSyntax(entry_path, err) => {
            handled_paths.insert(entry_path);
            warnings.push(ImportWarning::new(ImportWarningKind::MissingMetadata, "manifest.json", format!("could not parse manifest.json: {err}")));
            None
        }
    }
}

fn register_manifest_entries(
    manifest: &PrismManifest,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    meta_index: &mut TableMetaIndex,
) -> Result<(), FormatError> {
    for (source_path, item) in &manifest.sheets {
        if let Some(csv_path) = &item.csv_path {
            let norm = normalize_rel_csv_key(csv_path, "");
            if !norm.is_empty() {
                meta_index.by_csv.insert(norm.clone(), item.clone());
                meta_index.expected_csv_refs.push(ReferencedCsvPath { lookup_key: norm, raw_display: csv_path.clone() });
            }
        } else if let Some(id_or_title) = item.id.as_deref().or(item.title.as_deref()) {
            let norm = stem_without_ext(id_or_title).to_ascii_lowercase();
            if !norm.is_empty() {
                meta_index.by_stem.insert(norm, item.clone());
            }
            let _ = source_path;
        }
    }
    for info_meta in &manifest.info_sheets {
        import_info_meta(info_meta, doc, used_ids, warnings)?;
    }
    for (kind, source, msg) in &manifest.unsupported_sections {
        warnings.push(ImportWarning::new(*kind, source, msg));
    }
    Ok(())
}

fn import_json_pass(
    archive: &ZipArchive,
    root_prefix: &str,
    handled_paths: &mut HashSet<String>,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    meta_index: &mut TableMetaIndex,
) -> Result<(), FormatError> {
    for (path, payload) in archive.iter() {
        if handled_paths.contains(path) || !has_ext_ci(path, ".json") {
            continue;
        }
        let rel = strip_root_prefix(path, root_prefix);
        handled_paths.insert(path.to_owned());

        if is_info_json_path(rel) {
            import_info_json(path, payload, doc, used_ids, warnings)?;
            continue;
        }
        if let Some((kind, desc)) = classify_unsupported_json_dir(rel) {
            warnings.push(ImportWarning::new(
                kind,
                path,
                format!("{desc} entry `{path}` is not imported automatically; data tables and info constants were preserved"),
            ));
            continue;
        }

        let val = match parse_bounded_json_outcome(payload) {
            BoundedJsonOutcome::Value(v) => v,
            BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
            BoundedJsonOutcome::SyntaxOrType(err) => {
                warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, format!("could not parse JSON entry `{path}`: {err}")));
                continue;
            }
        };

        classify_and_register_json_value(
            rel,
            path,
            &val,
            doc,
            used_ids,
            warnings,
            &mut meta_index.by_csv,
            &mut meta_index.by_stem,
            &mut meta_index.by_dir,
            &mut meta_index.expected_csv_refs,
        )?;
    }
    Ok(())
}

fn import_csv_pass(
    archive: &ZipArchive,
    root_prefix: &str,
    handled_paths: &mut HashSet<String>,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    meta_index: &TableMetaIndex,
) -> Result<usize, FormatError> {
    let mut used_cells: usize = 0;
    let mut data_idx = 1_usize;
    let mut matched_csv_keys: HashSet<String> = HashSet::new();
    for (path, payload) in archive.iter() {
        if handled_paths.contains(path) || !(has_ext_ci(path, ".csv") || has_ext_ci(path, ".tsv")) {
            continue;
        }
        handled_paths.insert(path.to_owned());
        let text = match std::str::from_utf8(payload) {
            Ok(s) => s,
            Err(err) => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::UnsupportedChunk,
                    path,
                    format!("CSV/TSV entry is not valid UTF-8 and was skipped: {err}"),
                ));
                continue;
            }
        };
        let rel = strip_root_prefix(path, root_prefix);
        let rel_lower = rel.to_ascii_lowercase();
        let rel_stem = stem_without_ext(rel).to_ascii_lowercase();
        let rel_dir = parent_dir(rel).to_ascii_lowercase();
        let base_stem = rel.rsplit('/').next().map(stem_without_ext).unwrap_or("").to_ascii_lowercase();

        let meta = if let Some(m) = meta_index.by_csv.get(&rel_lower) {
            matched_csv_keys.insert(rel_lower);
            m.clone()
        } else if let Some(m) = meta_index.by_dir.get(&rel_dir) {
            m.clone()
        } else if let Some(m) = meta_index.by_stem.get(&rel_stem).or_else(|| meta_index.by_stem.get(&base_stem)) {
            m.clone()
        } else {
            PrismTableMeta::default()
        };

        let remaining_cells = MAX_PROJECT_CELLS.saturating_sub(used_cells);
        let ds = import_csv_entry(path, text, &meta, data_idx, used_ids, warnings, remaining_cells)?;
        used_cells = accumulate_project_cells(used_cells, &ds)?;
        doc.add_data_sheet(ds)?;
        data_idx += 1;
    }

    let mut reported_missing: HashSet<String> = HashSet::new();
    for expected in &meta_index.expected_csv_refs {
        if !matched_csv_keys.contains(&expected.lookup_key) && reported_missing.insert(expected.lookup_key.clone()) {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                expected.raw_display.clone(),
                format!("referenced table CSV/TSV `{}` was not found in the `.prism` archive", expected.raw_display),
            ));
        }
    }
    Ok(used_cells)
}

fn import_pzfx_and_tail_passes(
    archive: &ZipArchive,
    handled_paths: &mut HashSet<String>,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    used_cells: &mut usize,
) -> Result<(), FormatError> {
    for (path, payload) in archive.iter() {
        if handled_paths.contains(path) || !has_ext_ci(path, ".pzfx") {
            continue;
        }
        handled_paths.insert(path.to_owned());
        merge_embedded_pzfx(payload, doc, used_ids, warnings, used_cells)?;
    }

    for (path, payload) in archive.iter() {
        if !handled_paths.contains(path) {
            let msg = if is_sqlite_chunk(path, payload) {
                format!("embedded SQLite database `{path}` cannot be imported natively without tabular CSV/JSON/PZFX chunks")
            } else {
                format!("unsupported or binary `.prism` chunk `{path}` was skipped")
            };
            warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, msg));
        }
    }
    Ok(())
}

enum ManifestParseState {
    Absent,
    ValidValue(String, serde_json::Value),
    InvalidSyntax(String, FormatError),
}

fn inspect_manifest_bounded(entry: Option<(&str, &[u8])>) -> Result<ManifestParseState, FormatError> {
    let Some((path, manifest_bytes)) = entry else {
        return Ok(ManifestParseState::Absent);
    };
    match parse_bounded_json_outcome(manifest_bytes) {
        BoundedJsonOutcome::Value(manifest_val) => {
            if let Some(format) = manifest_val.get("format").and_then(serde_json::Value::as_str)
                && (format == crate::archive::LEGACY_FORMAT_MAGIC || format == "lumencraft")
            {
                return Err(FormatError::UnsupportedFormat {
                    reason: format!("legacy LumenCraft archive (`{format}`) cannot be imported as a `.prism` project"),
                });
            }
            Ok(ManifestParseState::ValidValue(path.to_owned(), manifest_val))
        }
        BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
        BoundedJsonOutcome::SyntaxOrType(err) => Ok(ManifestParseState::InvalidSyntax(path.to_owned(), err)),
    }
}

fn try_load_project_json(archive: &ZipArchive, root_prefix: &str, warnings: &mut Vec<ImportWarning>) -> Result<ProjectJsonLoadOutcome, FormatError> {
    let Some((doc_path, doc_bytes)) = find_archive_entry_ci(archive, "document.json", root_prefix) else {
        return Ok(ProjectJsonLoadOutcome::Absent);
    };
    let streamed = match parse_streamed_document_outcome(doc_bytes) {
        StreamedDocumentOutcome::Parsed(doc) => *doc,
        StreamedDocumentOutcome::BudgetExceeded(err) | StreamedDocumentOutcome::DataError(err) => return Err(err),
        StreamedDocumentOutcome::NonObject | StreamedDocumentOutcome::SyntaxOrType(_) => match parse_bounded_json_outcome(doc_bytes) {
            BoundedJsonOutcome::Value(value) => return Ok(ProjectJsonLoadOutcome::ExternalMeta { path: doc_path.to_owned(), value }),
            BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
            BoundedJsonOutcome::SyntaxOrType(err) => {
                warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, doc_path, format!("could not parse `{doc_path}`: {err}")));
                return Ok(ProjectJsonLoadOutcome::InvalidJsonChunk(doc_path.to_owned()));
            }
        },
    };
    if !streamed.has_data_sheets_key {
        let value = match parse_bounded_json_outcome(doc_bytes) {
            BoundedJsonOutcome::Value(v) => v,
            BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
            BoundedJsonOutcome::SyntaxOrType(_) => serde_json::Value::Object(serde_json::Map::new()),
        };
        return Ok(ProjectJsonLoadOutcome::ExternalMeta { path: doc_path.to_owned(), value });
    }
    let version = match streamed.meta_doc.get("schema_version") {
        None | Some(serde_json::Value::Null) => 0,
        Some(v) => match v.as_u64() {
            Some(n) => clamp_peek_schema_version(n),
            None => {
                return Err(FormatError::Json(String::from("invalid `schema_version` in embedded `document.json`: expected unsigned integer")));
            }
        },
    };
    let migrated = migrate_document_json(streamed.meta_doc, version)?;
    precheck_migrated_image_assets(&migrated)?;
    let mut doc = finalize_migrated_document(migrated, streamed.heavy)?;
    doc.data_sheets.iter().try_fold(0_usize, accumulate_project_cells)?;
    hydrate_embedded_image_assets(archive, root_prefix, &mut doc)?;
    sanitize_invalid_document_sheet_ids(&mut doc, warnings);
    validate_project_document(&doc)?;

    for (path, payload) in archive.iter() {
        let rel = strip_root_prefix(path, root_prefix);
        if !rel.eq_ignore_ascii_case("document.json")
            && !rel.eq_ignore_ascii_case("manifest.json")
            && !starts_with_dir_ci(rel, &["sheets/", "assets/"])
        {
            let msg = if is_sqlite_chunk(path, payload) {
                format!("embedded SQLite database `{path}` was ignored because `document.json` provided full project state")
            } else {
                format!("auxiliary `.prism` chunk `{path}` was ignored because `document.json` provided full project state")
            };
            warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, msg));
        }
    }
    Ok(ProjectJsonLoadOutcome::Embedded(Box::new(ImportReport { document: doc, warnings: std::mem::take(warnings) })))
}

fn precheck_migrated_image_assets(migrated: &serde_json::Value) -> Result<(), FormatError> {
    let Some(raw_assets) = migrated.get("image_assets").and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    if raw_assets.len() > MAX_PROJECT_IMAGE_ASSETS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyImageAssets { count: raw_assets.len(), max: MAX_PROJECT_IMAGE_ASSETS }));
    }
    let mut seen_ids = HashSet::with_capacity(raw_assets.len());
    for entry in raw_assets {
        if let Some(asset_id) = entry.get("asset_id").and_then(serde_json::Value::as_str)
            && !seen_ids.insert(asset_id)
        {
            return Err(FormatError::DuplicateEntry { path: asset_id.to_owned() });
        }
        if let Some(encoded) = entry.get("bytes").and_then(serde_json::Value::as_str) {
            let clean = encoded.trim();
            if !clean.is_empty() && clean.len().is_multiple_of(4) {
                let pad = if clean.ends_with("==") {
                    2
                } else if clean.ends_with('=') {
                    1
                } else {
                    0
                };
                let decoded_len = (clean.len() / 4).saturating_mul(3).saturating_sub(pad);
                if decoded_len > MAX_IMAGE_ASSET_BYTES {
                    let asset_id = entry.get("asset_id").and_then(serde_json::Value::as_str).unwrap_or("<image>");
                    return Err(FormatError::FileTooLarge {
                        path: format!("assets/{asset_id}"),
                        actual_bytes: Some(decoded_len as u64),
                        limit_bytes: MAX_IMAGE_ASSET_BYTES,
                    });
                }
            }
        }
    }
    Ok(())
}

fn hydrate_embedded_image_assets(archive: &ZipArchive, root_prefix: &str, doc: &mut ProjectDocument) -> Result<(), FormatError> {
    if doc.image_assets.len() > MAX_PROJECT_IMAGE_ASSETS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyImageAssets { count: doc.image_assets.len(), max: MAX_PROJECT_IMAGE_ASSETS }));
    }
    let mut seen_ids = HashSet::with_capacity(doc.image_assets.len());
    for asset in &mut doc.image_assets {
        if !is_valid_asset_id(&asset.asset_id) {
            return Err(FormatError::UnsupportedFormat { reason: format!("invalid image asset identifier `{}`", asset.asset_id) });
        }
        if !seen_ids.insert(asset.asset_id.clone()) {
            return Err(FormatError::DuplicateEntry { path: asset.asset_id.clone() });
        }
        let expected_path = format!("assets/{}.{}", asset.asset_id, asset.media_type.extension());
        let entry = find_archive_entry_ci(archive, &expected_path, root_prefix);
        if asset.bytes.is_empty() {
            let Some((_, bytes)) = entry else {
                return Err(FormatError::MissingArchiveEntry { path: expected_path });
            };
            if bytes.len() > MAX_IMAGE_ASSET_BYTES {
                return Err(FormatError::FileTooLarge {
                    path: expected_path,
                    actual_bytes: Some(bytes.len() as u64),
                    limit_bytes: MAX_IMAGE_ASSET_BYTES,
                });
            }
            validate_image_asset_parts(&asset.asset_id, asset.media_type, asset.pixel_size, bytes)?;
            asset.bytes = bytes.to_vec();
        } else {
            validate_image_asset_parts(&asset.asset_id, asset.media_type, asset.pixel_size, &asset.bytes)?;
            if let Some((_, archived)) = entry
                && archived != asset.bytes.as_slice()
            {
                return Err(FormatError::Json(format!(
                    "image asset `{}` inline bytes in document.json disagree with archive payload",
                    asset.asset_id
                )));
            }
        }
    }
    Ok(())
}

fn detect_archive_root_prefix(archive: &ZipArchive) -> String {
    if find_archive_entry_exact_or_ci_top(archive, "manifest.json").is_some()
        || find_archive_entry_exact_or_ci_top(archive, "document.json").is_some()
    {
        return String::new();
    }
    let mut common_dir: Option<String> = None;
    for (path, _) in archive.iter() {
        if path.starts_with("__MACOSX/") {
            continue;
        }
        let Some((first_seg, _)) = path.split_once('/') else {
            return String::new();
        };
        let candidate = format!("{first_seg}/");
        match &common_dir {
            None => common_dir = Some(candidate),
            Some(prev) if prev.eq_ignore_ascii_case(&candidate) => {}
            Some(_) => return String::new(),
        }
    }
    let Some(dir) = common_dir else {
        return String::new();
    };
    let lower = dir.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "sheets/" | "info/" | "assets/" | "scripts/" | "analyses/" | "analysis/" | "graphs/" | "layouts/" | "equations/" | "tables/" | "data/"
    ) {
        String::new()
    } else {
        dir
    }
}

fn find_archive_entry_exact_or_ci_top<'a>(archive: &'a ZipArchive, target: &str) -> Option<(&'a str, &'a [u8])> {
    archive.iter().find(|(p, _)| *p == target).or_else(|| archive.iter().find(|(p, _)| p.eq_ignore_ascii_case(target)))
}

fn find_archive_entry_ci<'a>(archive: &'a ZipArchive, rel_target: &str, root_prefix: &str) -> Option<(&'a str, &'a [u8])> {
    if let Some(hit) = archive.iter().find(|(p, _)| *p == rel_target) {
        return Some(hit);
    }
    if !root_prefix.is_empty() {
        let prefixed = format!("{root_prefix}{rel_target}");
        if let Some(hit) = archive.iter().find(|(p, _)| *p == prefixed) {
            return Some(hit);
        }
        if let Some(hit) = archive.iter().find(|(p, _)| p.eq_ignore_ascii_case(&prefixed)) {
            return Some(hit);
        }
    }
    archive.iter().find(|(p, _)| strip_root_prefix(p, root_prefix).eq_ignore_ascii_case(rel_target))
}

fn strip_root_prefix<'a>(path: &'a str, root_prefix: &str) -> &'a str {
    if root_prefix.is_empty() {
        return path;
    }
    if path.len() >= root_prefix.len() && path.get(..root_prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(root_prefix)) {
        path.get(root_prefix.len()..).unwrap_or(path)
    } else {
        path
    }
}

fn has_ext_ci(path: &str, ext_with_dot: &str) -> bool {
    path.len() >= ext_with_dot.len()
        && path.get(path.len().saturating_sub(ext_with_dot.len())..).is_some_and(|tail| tail.eq_ignore_ascii_case(ext_with_dot))
}

fn starts_with_dir_ci(rel_path: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| rel_path.len() >= prefix.len() && rel_path.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix)))
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn normalize_rel_csv_key(raw: &str, base_dir: &str) -> String {
    let mut trimmed = raw.trim().replace('\\', "/");
    while let Some(rest) = trimmed.strip_prefix("./") {
        trimmed = rest.to_owned();
    }
    let trimmed = trimmed.trim_start_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if !trimmed.contains('/') && !base_dir.is_empty() { format!("{}/{trimmed}", base_dir.to_ascii_lowercase()) } else { trimmed.to_ascii_lowercase() }
}

fn is_sqlite_chunk(path: &str, payload: &[u8]) -> bool {
    has_ext_ci(path, ".sqlite") || has_ext_ci(path, ".sqlite3") || has_ext_ci(path, ".db") || payload.starts_with(b"SQLite format 3\0")
}

fn is_info_json_path(rel_path: &str) -> bool {
    let lower = rel_path.to_ascii_lowercase();
    let basename = lower.rsplit('/').next().unwrap_or(&lower);
    lower.starts_with("info/") || lower == "info.json" || lower.ends_with("/info.json") || basename.starts_with("info_")
}

fn classify_unsupported_json_dir(rel_path: &str) -> Option<(ImportWarningKind, &'static str)> {
    let lower = rel_path.to_ascii_lowercase();
    let basename = lower.rsplit('/').next().unwrap_or(&lower);
    if starts_with_dir_ci(&lower, &["analyses/", "analysis/", "results/"]) || matches!(basename, "analyses.json" | "analysis.json" | "results.json") {
        Some((ImportWarningKind::UnsupportedAnalysis, "analysis definition"))
    } else if starts_with_dir_ci(&lower, &["graphs/", "graph/", "layouts/", "layout/"]) || matches!(basename, "graphs.json" | "layouts.json") {
        Some((ImportWarningKind::UnsupportedGraphOrLayout, "graph/layout definition"))
    } else if starts_with_dir_ci(&lower, &["equations/", "equation/"]) || matches!(basename, "equations.json" | "user_equations.json") {
        Some((ImportWarningKind::UnsupportedEquation, "user equation definition"))
    } else if starts_with_dir_ci(&lower, &["scripts/", "script/"]) || matches!(basename, "scripts.json" | "script.json") {
        Some((ImportWarningKind::UnsupportedScript, "script definition"))
    } else {
        None
    }
}

fn parse_external_project_meta(val: &serde_json::Value, source_path: &str, warnings: &mut Vec<ImportWarning>) -> PrismManifest {
    let Some(obj) = val.as_object() else {
        warnings.push(ImportWarning::new(ImportWarningKind::MissingMetadata, source_path, format!("expected JSON object in `{source_path}`")));
        return PrismManifest::default();
    };
    let mut out = PrismManifest {
        title: obj
            .get("title")
            .or_else(|| obj.get("name"))
            .or_else(|| obj.get("project_title"))
            .or_else(|| obj.get("projectTitle"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        ..PrismManifest::default()
    };

    for key in ["sheets", "tables", "data_tables", "dataTables", "data_sheets", "data"] {
        if let Some(raw_sheets) = obj.get(key) {
            parse_manifest_sheets_value(raw_sheets, source_path, &mut out.sheets, warnings);
        }
    }
    for key in ["info_sheets", "infoSheets", "info_tables", "infoTables", "info", "infos"] {
        if let Some(raw_infos) = obj.get(key) {
            parse_manifest_infos_value(raw_infos, source_path, &mut out.info_sheets, warnings);
        }
    }

    for (key, kind, desc) in [
        ("analyses", ImportWarningKind::UnsupportedAnalysis, "analyses"),
        ("results", ImportWarningKind::UnsupportedAnalysis, "cached analysis results"),
        ("graphs", ImportWarningKind::UnsupportedGraphOrLayout, "graphs"),
        ("layouts", ImportWarningKind::UnsupportedGraphOrLayout, "layouts"),
        ("equations", ImportWarningKind::UnsupportedEquation, "equations"),
        ("user_equations", ImportWarningKind::UnsupportedEquation, "user equations"),
        ("scripts", ImportWarningKind::UnsupportedScript, "scripts"),
    ] {
        if let Some(section) = obj.get(key) {
            let non_empty = match section {
                serde_json::Value::Array(arr) => !arr.is_empty(),
                serde_json::Value::Object(map) => !map.is_empty(),
                serde_json::Value::Bool(b) => *b,
                serde_json::Value::String(s) => !s.trim().is_empty(),
                serde_json::Value::Number(_) => true,
                serde_json::Value::Null => false,
            };
            if non_empty {
                out.unsupported_sections.push((
                    kind,
                    source_path.to_owned(),
                    format!("`{source_path}` section `{key}` ({desc}) is not imported automatically; tabular data was preserved"),
                ));
            }
        }
    }
    out
}

fn merge_manifest_fallback(target: &mut PrismManifest, fallback: PrismManifest) {
    if target.title.as_deref().map(str::trim).unwrap_or("").is_empty() {
        target.title = fallback.title;
    }
    target.sheets.extend(fallback.sheets);
    target.info_sheets.extend(fallback.info_sheets);
    target.unsupported_sections.extend(fallback.unsupported_sections);
}

fn parse_manifest_sheets_value(
    raw: &serde_json::Value,
    source_path: &str,
    out: &mut Vec<(String, PrismTableMeta)>,
    warnings: &mut Vec<ImportWarning>,
) {
    match raw {
        serde_json::Value::Array(items) => {
            for item in items {
                match serde_json::from_value::<PrismTableMeta>(item.clone()) {
                    Ok(meta) => out.push((source_path.to_owned(), meta)),
                    Err(err) => warnings.push(ImportWarning::new(
                        ImportWarningKind::MissingMetadata,
                        source_path,
                        format!("could not parse sheet entry in `{source_path}`: {err}"),
                    )),
                }
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                match serde_json::from_value::<PrismTableMeta>(item.clone()) {
                    Ok(mut meta) => {
                        if meta.id.as_deref().map(str::trim).unwrap_or("").is_empty() {
                            meta.id = Some(key.clone());
                        }
                        out.push((source_path.to_owned(), meta));
                    }
                    Err(err) => warnings.push(ImportWarning::new(
                        ImportWarningKind::MissingMetadata,
                        source_path,
                        format!("could not parse sheet entry `{key}` in `{source_path}`: {err}"),
                    )),
                }
            }
        }
        serde_json::Value::Null => {}
        _ => warnings.push(ImportWarning::new(
            ImportWarningKind::MissingMetadata,
            source_path,
            format!("unexpected sheet list type in `{source_path}`"),
        )),
    }
}

fn parse_manifest_infos_value(raw: &serde_json::Value, source_path: &str, out: &mut Vec<PrismInfoMeta>, warnings: &mut Vec<ImportWarning>) {
    match raw {
        serde_json::Value::Array(items) => {
            for item in items {
                match parse_info_meta_value(item) {
                    Some(meta) => out.push(meta),
                    None => warnings.push(ImportWarning::new(
                        ImportWarningKind::MissingMetadata,
                        source_path,
                        format!("could not parse info entry in `{source_path}`"),
                    )),
                }
            }
        }
        serde_json::Value::Object(map) => {
            if map.contains_key("constants") || map.contains_key("notes") || map.contains_key("notes_markdown") || map.contains_key("title") {
                if let Some(meta) = parse_info_meta_value(raw) {
                    out.push(meta);
                }
            } else {
                for (key, item) in map {
                    if let Some(mut meta) = parse_info_meta_value(item) {
                        if meta.id.as_deref().map(str::trim).unwrap_or("").is_empty() {
                            meta.id = Some(key.clone());
                        }
                        out.push(meta);
                    }
                }
            }
        }
        serde_json::Value::Null => {}
        _ => {}
    }
}

fn parse_info_meta_value(val: &serde_json::Value) -> Option<PrismInfoMeta> {
    let obj = val.as_object()?;
    let id = obj.get("id").or_else(|| obj.get("sheet_id")).and_then(serde_json::Value::as_str).map(str::to_owned);
    let title = obj.get("title").or_else(|| obj.get("name")).and_then(serde_json::Value::as_str).map(str::to_owned);
    let notes = obj
        .get("notes")
        .or_else(|| obj.get("notes_markdown"))
        .or_else(|| obj.get("description"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let mut constants = Vec::new();
    if let Some(raw_consts) = obj.get("constants") {
        match raw_consts {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    let trimmed = k.trim();
                    if !trimmed.is_empty() {
                        let (cval, units) = parse_prism_constant_value(v);
                        constants.push(ParsedInfoConstantEntry { name: trimmed.to_owned(), value: cval, units });
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    if let Some(entry_obj) = item.as_object()
                        && let Some(name) = entry_obj.get("name").or_else(|| entry_obj.get("key")).and_then(serde_json::Value::as_str)
                    {
                        let trimmed = name.trim();
                        if !trimmed.is_empty() {
                            let val_node = entry_obj.get("value").unwrap_or(&serde_json::Value::Null);
                            let units = entry_obj
                                .get("units")
                                .and_then(serde_json::Value::as_str)
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned);
                            constants.push(ParsedInfoConstantEntry { name: trimmed.to_owned(), value: scalar_json_to_constant(val_node), units });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(PrismInfoMeta { id, title, notes, constants })
}

#[allow(clippy::too_many_arguments)]
fn classify_and_register_json_value(
    rel_path: &str,
    full_path: &str,
    val: &serde_json::Value,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    meta_by_csv: &mut BTreeMap<String, PrismTableMeta>,
    meta_by_stem: &mut BTreeMap<String, PrismTableMeta>,
    meta_by_dir: &mut BTreeMap<String, PrismTableMeta>,
    expected_csv_refs: &mut Vec<ReferencedCsvPath>,
) -> Result<(), FormatError> {
    let Some(obj) = val.as_object() else {
        warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, full_path, format!("non-object JSON chunk `{full_path}` was skipped")));
        return Ok(());
    };

    let has_info_shape = obj.contains_key("constants")
        || (obj.contains_key("notes_markdown") && !obj.contains_key("csv_path") && !obj.contains_key("csvPath") && !obj.contains_key("kind"));
    if has_info_shape && let Some(info_meta) = parse_info_meta_value(val) {
        return import_info_meta(&info_meta, doc, used_ids, warnings);
    }

    if let Some((kind, desc)) = detect_unsupported_json_payload_keys(obj) {
        warnings.push(ImportWarning::new(
            kind,
            full_path,
            format!("{desc} JSON `{full_path}` is not imported automatically; data tables and info constants were preserved"),
        ));
        return Ok(());
    }

    let is_table_sidecar_dir = starts_with_dir_ci(rel_path, &["sheets/", "tables/", "data/"]);
    let has_table_keys = obj.contains_key("csv_path")
        || obj.contains_key("csvPath")
        || obj.contains_key("data_path")
        || obj.contains_key("dataPath")
        || obj.contains_key("kind")
        || obj.contains_key("table_type")
        || obj.contains_key("tableType")
        || obj.contains_key("archetype")
        || obj.contains_key("subcolumn_mode")
        || obj.contains_key("subcolumnMode")
        || obj.contains_key("replicates")
        || obj.contains_key("delimiter")
        || obj.contains_key("decimal_separator")
        || obj.contains_key("decimalSeparator")
        || obj.contains_key("has_row_titles")
        || obj.contains_key("hasRowTitles")
        || (is_table_sidecar_dir && obj.contains_key("title"));

    if has_table_keys && let Ok(meta) = serde_json::from_value::<PrismTableMeta>(val.clone()) {
        let json_dir = parent_dir(rel_path);
        if let Some(csv_p) = &meta.csv_path {
            let root_key = normalize_rel_csv_key(csv_p, "");
            let rel_key = normalize_rel_csv_key(csv_p, json_dir);
            meta_by_csv.insert(root_key, meta.clone());
            if !rel_key.is_empty() {
                meta_by_csv.insert(rel_key.clone(), meta.clone());
                expected_csv_refs.push(ReferencedCsvPath { lookup_key: rel_key, raw_display: csv_p.clone() });
            }
        }
        let stem_lower = stem_without_ext(rel_path).to_ascii_lowercase();
        let file_stem = rel_path.rsplit('/').next().map(stem_without_ext).unwrap_or("").to_ascii_lowercase();
        if matches!(file_stem.as_str(), "meta" | "sheet" | "table") && !json_dir.is_empty() {
            meta_by_dir.insert(json_dir.to_ascii_lowercase(), meta);
        } else {
            meta_by_stem.insert(stem_lower, meta);
        }
        return Ok(());
    }

    warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, full_path, format!("unrecognized JSON chunk `{full_path}` was skipped")));
    Ok(())
}

fn detect_unsupported_json_payload_keys(obj: &serde_json::Map<String, serde_json::Value>) -> Option<(ImportWarningKind, &'static str)> {
    if obj.contains_key("equation") || obj.contains_key("formula") || obj.contains_key("user_equation") {
        Some((ImportWarningKind::UnsupportedEquation, "equation"))
    } else if obj.contains_key("analysis")
        || obj.contains_key("analysis_spec")
        || obj.contains_key("fit_results")
        || obj.contains_key("fitted_curves")
    {
        Some((ImportWarningKind::UnsupportedAnalysis, "analysis"))
    } else if obj.contains_key("axes") || obj.contains_key("series") || obj.contains_key("layout_items") || obj.contains_key("graph_spec") {
        Some((ImportWarningKind::UnsupportedGraphOrLayout, "graph/layout"))
    } else if obj.contains_key("script") || obj.contains_key("code") {
        Some((ImportWarningKind::UnsupportedScript, "script"))
    } else {
        None
    }
}

fn stem_without_ext(path: &str) -> &str {
    path.rsplit_once('.').map_or(path, |(stem, _)| stem)
}

fn accumulate_project_cells(used_cells: usize, ds: &DataSheet) -> Result<usize, FormatError> {
    let actual_cells = ds.archetype.allocated_cell_slots();
    let next_total = used_cells.checked_add(actual_cells).ok_or_else(|| FormatError::InvalidPipeline {
        reason: format!("sheet `{}` cell allocation overflows project budget cap ({MAX_PROJECT_CELLS})", ds.id),
    })?;
    if next_total > MAX_PROJECT_CELLS {
        let remaining = MAX_PROJECT_CELLS.saturating_sub(used_cells);
        return Err(FormatError::InvalidPipeline {
            reason: format!(
                "sheet `{}` cell allocation ({actual_cells}) exceeds remaining project import budget ({remaining}, cap {MAX_PROJECT_CELLS})",
                ds.id
            ),
        });
    }
    Ok(next_total)
}

fn merge_embedded_pzfx(
    payload: &[u8],
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    used_cells: &mut usize,
) -> Result<(), FormatError> {
    let sub = import_pzfx_bytes_with_used_cells(payload, used_cells)?;
    warnings.extend(sub.warnings);

    let mut info_id_remap: BTreeMap<SheetId, SheetId> = BTreeMap::new();
    let mut infos_to_add = Vec::with_capacity(sub.document.info_sheets.len());
    for mut info in sub.document.info_sheets {
        let old_id = info.id.clone();
        let idx = doc.info_sheets.len() + infos_to_add.len() + 1;
        let new_id = allocate_valid_unique_sheet_id(Some(old_id.as_str()), "info", idx, used_ids, warnings);
        if new_id != old_id {
            info_id_remap.insert(old_id, new_id.clone());
        }
        info.id = new_id;
        infos_to_add.push(info);
    }

    let mut data_id_remap: BTreeMap<SheetId, SheetId> = BTreeMap::new();
    let mut datas_to_add = Vec::with_capacity(sub.document.data_sheets.len());
    for mut ds in sub.document.data_sheets {
        let old_id = ds.id.clone();
        let idx = doc.data_sheets.len() + datas_to_add.len() + 1;
        let new_id = allocate_valid_unique_sheet_id(Some(old_id.as_str()), "data", idx, used_ids, warnings);
        if new_id != old_id {
            data_id_remap.insert(old_id, new_id.clone());
        }
        ds.id = new_id;
        if let Some(ref mut info_id) = ds.info_sheet_id
            && let Some(remapped) = info_id_remap.get(info_id)
        {
            *info_id = remapped.clone();
        }
        datas_to_add.push(ds);
    }

    for mut info in infos_to_add {
        for linked in &mut info.linked_data_sheets {
            if let Some(remapped) = data_id_remap.get(linked) {
                *linked = remapped.clone();
            }
        }
        doc.add_info_sheet(info)?;
    }
    for mut ds in datas_to_add {
        for region in &mut ds.linked_regions {
            if let LinkSource::InternalSheet { ref mut sheet_id, .. } = region.spec.source
                && let Some(remapped) = data_id_remap.get(sheet_id)
            {
                *sheet_id = remapped.clone();
            }
        }
        doc.add_data_sheet(ds)?;
    }
    Ok(())
}

fn import_info_json(
    path: &str,
    payload: &[u8],
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    let val = match parse_bounded_json_outcome(payload) {
        BoundedJsonOutcome::Value(v) => v,
        BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
        BoundedJsonOutcome::SyntaxOrType(err) => {
            warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, format!("could not parse info JSON `{path}`: {err}")));
            return Ok(());
        }
    };
    let Some(meta) = parse_info_meta_value(&val) else {
        warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, format!("could not parse info JSON `{path}`")));
        return Ok(());
    };
    import_info_meta(&meta, doc, used_ids, warnings)
}

fn import_info_meta(
    meta: &PrismInfoMeta,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    let idx = doc.info_sheets.len() + 1;
    let id = allocate_valid_unique_sheet_id(meta.id.as_deref(), "info", idx, used_ids, warnings);
    let raw_title = meta.title.as_deref().map(str::trim).filter(|s| !s.is_empty()).map_or_else(|| format!("Info {idx}"), str::to_owned);
    let title = clamp_title(&raw_title, id.as_str(), warnings);
    let mut sheet = InfoSheet::new_preseeded(id.as_str().to_owned(), title)?;
    sheet.constants.clear();
    for entry in &meta.constants {
        let name = clamp_title(&entry.name, id.as_str(), warnings);
        let units = entry.units.as_deref().map(|u| clamp_title(u, "units", warnings)).filter(|u| !u.is_empty());
        sheet.constants.push(InfoConstant::new(name, entry.value.clone(), units)?);
    }
    if let Some(notes) = &meta.notes {
        sheet.notes_markdown = notes.clone();
    }
    doc.add_info_sheet(sheet)?;
    Ok(())
}

fn parse_prism_constant_value(v: &serde_json::Value) -> (ConstantValue, Option<String>) {
    if let Some(obj) = v.as_object()
        && let Some(inner) = obj.get("value")
    {
        let units = obj.get("units").and_then(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        return (scalar_json_to_constant(inner), units);
    }
    (scalar_json_to_constant(v), None)
}

fn scalar_json_to_constant(v: &serde_json::Value) -> ConstantValue {
    match v {
        serde_json::Value::Null => ConstantValue::Empty,
        serde_json::Value::Number(n) => match n.as_f64().filter(|x| x.is_finite()) {
            Some(f) => ConstantValue::Number(f),
            None => ConstantValue::Text(n.to_string()),
        },
        serde_json::Value::String(s) => ConstantValue::Text(s.clone()),
        serde_json::Value::Bool(b) => ConstantValue::Text(b.to_string()),
        other => ConstantValue::Text(other.to_string()),
    }
}

fn import_csv_entry(
    path: &str,
    raw_text: &str,
    meta: &PrismTableMeta,
    idx: usize,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    remaining_cells: usize,
) -> Result<DataSheet, FormatError> {
    let text = raw_text.strip_prefix('\u{FEFF}').unwrap_or(raw_text);
    let fallback_title = path.rsplit('/').next().map(stem_without_ext).filter(|s| !s.is_empty()).unwrap_or("Table");
    let id = allocate_valid_unique_sheet_id(meta.id.as_deref(), "data", idx, used_ids, warnings);
    let raw_title = meta.title.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| fallback_title.to_owned());
    let title = clamp_title(&raw_title, id.as_str(), warnings);

    let kind = parse_table_kind(meta.kind.as_deref(), path, warnings);
    let replicates = match meta.replicates {
        Some(0) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                path,
                format!("`.prism` table `{path}` specified replicates=0; defaulted to 1"),
            ));
            1
        }
        Some(n) => n,
        None => 1,
    };
    let subcolumn_mode = parse_subcolumn_mode(kind, meta.subcolumn_mode.as_deref(), replicates, path, warnings);

    let mut opts = if has_ext_ci(path, ".tsv") { CsvOptions::tsv() } else { CsvOptions::detect(text) };
    if let Some(d) = meta.delimiter {
        opts.delimiter = d;
    }
    if let Some(dec) = meta.decimal_separator {
        opts.decimal_separator = dec;
    }
    if let Some(has_rt) = meta.has_row_titles {
        opts.has_row_titles = has_rt;
    } else if csv_has_explicit_row_title_header(text, kind, &opts) {
        opts.has_row_titles = true;
    }

    import_csv_to_datasheet_with_budget(id, title, text, kind, subcolumn_mode, &opts, remaining_cells)
}

fn csv_has_explicit_row_title_header(text: &str, kind: TableKind, opts: &CsvOptions) -> bool {
    let prefix: String = text.strip_prefix('\u{FEFF}').unwrap_or(text).lines().take(16).collect::<Vec<_>>().join("\n");
    let Ok(records) = parse_delimited_records(&prefix, opts) else {
        return false;
    };
    let Some(first_row) = records.first() else {
        return false;
    };
    let Some(first_hdr) = first_row.first().map(|h| h.trim()) else {
        return false;
    };
    if first_row.len() < 2 {
        return false;
    }
    let first_line = first_hdr.split('\n').next().map(str::trim).unwrap_or("");
    let header_matches =
        |expected: &str| first_hdr.eq_ignore_ascii_case(expected) || (first_hdr.contains('\n') && first_line.eq_ignore_ascii_case(expected));
    header_matches("Row Title")
        || (kind == TableKind::Contingency && header_matches("Outcome / Exposure"))
        || (kind == TableKind::Grouped && header_matches("Row Factor"))
}

fn parse_table_kind(raw: Option<&str>, source: &str, warnings: &mut Vec<ImportWarning>) -> TableKind {
    let trimmed = raw.map(str::trim).unwrap_or("");
    match trimmed.to_ascii_lowercase().as_str() {
        "" | "column" | "oneway" => TableKind::Column,
        "xy" => TableKind::Xy,
        "grouped" | "twoway" => TableKind::Grouped,
        "contingency" => TableKind::Contingency,
        "survival" => TableKind::Survival,
        "partsofwhole" | "parts_of_whole" | "parts-of-whole" => TableKind::PartsOfWhole,
        "multiplevariables" | "multiple_variables" | "multiple-variables" | "huge" => {
            if trimmed.eq_ignore_ascii_case("huge") {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::UnsupportedTableType,
                    source,
                    format!("`.prism` table kind `Huge` on `{source}` mapped to MultipleVariables"),
                ));
            }
            TableKind::MultipleVariables
        }
        "nested" => TableKind::Nested,
        _ => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedTableType,
                source,
                format!("unrecognized `.prism` table kind `{trimmed}`, defaulting to Column"),
            ));
            TableKind::Column
        }
    }
}

fn parse_subcolumn_mode(kind: TableKind, raw: Option<&str>, replicates: usize, source: &str, warnings: &mut Vec<ImportWarning>) -> SubcolumnMode {
    let trimmed = raw.map(str::trim).unwrap_or("");
    let candidate = match trimmed.to_ascii_lowercase().as_str() {
        "" | "single" | "none" => {
            if matches!(kind, TableKind::Grouped | TableKind::Nested) {
                SubcolumnMode::RawReplicates(replicates.max(1))
            } else {
                if replicates > 1
                    && matches!(
                        kind,
                        TableKind::Column | TableKind::Contingency | TableKind::Survival | TableKind::PartsOfWhole | TableKind::MultipleVariables
                    )
                {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        source,
                        format!("replicates={replicates} is not supported for `{kind:?}` without a summary mode; using Single"),
                    ));
                } else if replicates > 1 && kind == TableKind::Xy {
                    return SubcolumnMode::RawReplicates(replicates);
                }
                SubcolumnMode::Single
            }
        }
        "rawreplicates" | "replicates" | "raw" | "raw-replicates" => SubcolumnMode::RawReplicates(replicates.max(1)),
        "meansdn" | "sd" | "sdn" => SubcolumnMode::MeanSdN,
        "meansemn" | "sem" | "semn" | "sen" | "se" => SubcolumnMode::MeanSemN,
        "meancvn" | "cv" | "cvn" => SubcolumnMode::MeanCvN,
        "meanlowhigh" | "low-high" | "lowhigh" | "plus-minus" | "plusminus" => SubcolumnMode::MeanLowHigh,
        "upperlowerlimits" | "upper-lower" | "upper-lower-limits" | "upperlower" | "limits" => SubcolumnMode::UpperLowerLimits,
        "medianiqr" | "median-iqr" | "iqr" => SubcolumnMode::MedianIqr,
        _ => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                source,
                format!("unrecognized `.prism` subcolumn mode `{trimmed}`, using default"),
            ));
            if matches!(kind, TableKind::Grouped | TableKind::Nested) {
                SubcolumnMode::RawReplicates(replicates.max(1))
            } else {
                SubcolumnMode::Single
            }
        }
    };
    if kind.allows_subcolumn_mode(candidate) {
        candidate
    } else if matches!(kind, TableKind::Grouped | TableKind::Nested) {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            source,
            format!("subcolumn mode {candidate:?} is not valid for {kind:?} on `{source}`; using RawReplicates"),
        ));
        SubcolumnMode::RawReplicates(replicates.max(1))
    } else {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            source,
            format!("subcolumn mode {candidate:?} is not valid for {kind:?} on `{source}`; using Single"),
        ));
        SubcolumnMode::Single
    }
}

fn sanitize_invalid_document_sheet_ids(doc: &mut ProjectDocument, warnings: &mut Vec<ImportWarning>) {
    let mut used_ids = collect_valid_sheet_ids(doc);
    let mut remap: BTreeMap<SheetId, SheetId> = BTreeMap::new();

    for (idx, info) in doc.info_sheets.iter_mut().enumerate() {
        maybe_sanitize_sheet_id(&mut info.id, "info", idx + 1, &mut used_ids, &mut remap, warnings);
    }
    for (idx, ds) in doc.data_sheets.iter_mut().enumerate() {
        maybe_sanitize_sheet_id(&mut ds.id, "data", idx + 1, &mut used_ids, &mut remap, warnings);
    }
    for (idx, rs) in doc.result_sheets.iter_mut().enumerate() {
        let old_id = rs.id.clone();
        if maybe_sanitize_sheet_id(&mut rs.id, "result", idx + 1, &mut used_ids, &mut remap, warnings) {
            rs.analysis.id = rs.id.clone();
            if let Some(ref mut derived) = rs.analyzable_data
                && derived.id == old_id
            {
                derived.id = rs.id.clone();
            }
        }
    }
    for (idx, gs) in doc.graph_sheets.iter_mut().enumerate() {
        maybe_sanitize_sheet_id(&mut gs.id, "graph", idx + 1, &mut used_ids, &mut remap, warnings);
    }
    for (idx, ls) in doc.layout_sheets.iter_mut().enumerate() {
        maybe_sanitize_sheet_id(&mut ls.id, "layout", idx + 1, &mut used_ids, &mut remap, warnings);
    }

    if !remap.is_empty() {
        remap_document_sheet_references(doc, &remap);
    }
}

fn collect_valid_sheet_ids(doc: &ProjectDocument) -> HashSet<String> {
    let all_ids = doc
        .info_sheets
        .iter()
        .map(|s| s.id.as_str())
        .chain(doc.data_sheets.iter().map(|s| s.id.as_str()))
        .chain(doc.result_sheets.iter().map(|s| s.id.as_str()))
        .chain(doc.graph_sheets.iter().map(|s| s.id.as_str()))
        .chain(doc.layout_sheets.iter().map(|s| s.id.as_str()));
    all_ids.filter(|id| SheetId::try_new(*id).is_ok()).map(str::to_owned).collect()
}

fn maybe_sanitize_sheet_id(
    slot: &mut SheetId,
    prefix: &str,
    idx: usize,
    used_ids: &mut HashSet<String>,
    remap: &mut BTreeMap<SheetId, SheetId>,
    warnings: &mut Vec<ImportWarning>,
) -> bool {
    if SheetId::try_new(slot.as_str()).is_ok() {
        return false;
    }
    let old_id = slot.clone();
    let new_id = allocate_valid_unique_sheet_id(Some(old_id.as_str()), prefix, idx, used_ids, warnings);
    remap.insert(old_id, new_id.clone());
    *slot = new_id;
    true
}

fn apply_sheet_remap(slot: &mut SheetId, remap: &BTreeMap<SheetId, SheetId>) {
    if let Some(new_id) = remap.get(slot) {
        *slot = new_id.clone();
    }
}

fn apply_opt_sheet_remap(slot: &mut Option<SheetId>, remap: &BTreeMap<SheetId, SheetId>) {
    if let Some(id) = slot {
        apply_sheet_remap(id, remap);
    }
}

fn remap_hook_source(hook: &mut HookSource, remap: &BTreeMap<SheetId, SheetId>) {
    match hook {
        HookSource::InfoConstant { info_sheet_id, .. } => apply_opt_sheet_remap(info_sheet_id, remap),
        HookSource::ResultScalar { result_sheet_id, .. } => apply_sheet_remap(result_sheet_id, remap),
        HookSource::ColumnTitleValue { data_sheet_id, .. } => apply_sheet_remap(data_sheet_id, remap),
    }
}

fn find_longest_remapped_prefix<'a>(after_kind: &'a str, remap: &'a BTreeMap<SheetId, SheetId>) -> Option<(&'a SheetId, &'a str)> {
    remap
        .iter()
        .filter_map(|(old_id, new_id)| {
            let rest = after_kind.strip_prefix(old_id.as_str())?;
            let after_colon = rest.strip_prefix(':')?;
            Some((old_id.as_str().len(), new_id, after_colon))
        })
        .max_by_key(|(len, _, _)| *len)
        .map(|(_, new_id, rest)| (new_id, rest))
}

fn rewrite_hook_token(raw_inner: &str, remap: &BTreeMap<SheetId, SheetId>) -> Option<String> {
    let wrapped = format!("{{{{{raw_inner}}}}}");
    if let Some(hook) = extract_template_hooks(&wrapped).into_iter().next() {
        return match hook {
            HookSource::InfoConstant { info_sheet_id: Some(ref old_id), ref constant_name } => {
                remap.get(old_id).map(|new_id| format!("{{{{info:{new_id}:{constant_name}}}}}"))
            }
            HookSource::ResultScalar { ref result_sheet_id, ref parameter_key, data_set_index } => remap.get(result_sheet_id).map(|new_id| {
                if let Some(ds) = data_set_index {
                    format!("{{{{result:{new_id}:{ds}:{parameter_key}}}}}")
                } else {
                    format!("{{{{result:{new_id}:{parameter_key}}}}}")
                }
            }),
            _ => None,
        };
    }

    let trimmed = raw_inner.trim();
    if let Some(after_info) = trimmed.strip_prefix("info:") {
        let after_info = after_info.trim_start();
        if let Some((new_id, rest)) = find_longest_remapped_prefix(after_info, remap) {
            let name = rest.trim();
            if !name.is_empty() {
                return Some(format!("{{{{info:{new_id}:{name}}}}}"));
            }
        }
    } else if let Some(after_res) = trimmed.strip_prefix("result:") {
        let after_res = after_res.trim_start();
        if let Some((new_id, rest)) = find_longest_remapped_prefix(after_res, remap) {
            if let Some((ds_str, param)) = rest.split_once(':')
                && let Ok(ds) = ds_str.trim().parse::<usize>()
                && !param.trim().is_empty()
            {
                return Some(format!("{{{{result:{new_id}:{ds}:{}}}}}", param.trim()));
            }
            let param = rest.trim();
            if !param.is_empty() {
                return Some(format!("{{{{result:{new_id}:{param}}}}}"));
            }
        }
    }
    None
}

fn remap_template_hooks_in_text(text: &mut String, remap: &BTreeMap<SheetId, SheetId>) {
    if remap.is_empty() || !text.contains("{{") {
        return;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    let mut changed = false;

    while let Some((before, after_open)) = rest.split_once("{{") {
        out.push_str(before);
        let Some((raw_inner, after_close)) = after_open.split_once("}}") else {
            out.push_str("{{");
            out.push_str(after_open);
            rest = "";
            break;
        };
        if let Some(rewritten) = rewrite_hook_token(raw_inner, remap) {
            out.push_str(&rewritten);
            changed = true;
        } else {
            out.push_str("{{");
            out.push_str(raw_inner);
            out.push_str("}}");
        }
        rest = after_close;
    }
    out.push_str(rest);
    if changed {
        *text = out;
    }
}

fn remap_floating_notes(notes: &mut [FloatingNote], remap: &BTreeMap<SheetId, SheetId>) {
    for note in notes {
        remap_template_hooks_in_text(&mut note.body_markdown, remap);
    }
}

fn remap_document_sheet_references(doc: &mut ProjectDocument, remap: &BTreeMap<SheetId, SheetId>) {
    apply_opt_sheet_remap(&mut doc.active_sheet_id, remap);
    apply_opt_sheet_remap(&mut doc.family_anchor_sheet_id, remap);
    for info in &mut doc.info_sheets {
        for linked in &mut info.linked_data_sheets {
            apply_sheet_remap(linked, remap);
        }
        remap_floating_notes(&mut info.floating_notes, remap);
    }
    for ds in &mut doc.data_sheets {
        remap_datasheet_references(ds, remap);
    }
    for rs in &mut doc.result_sheets {
        apply_sheet_remap(&mut rs.analysis.id, remap);
        apply_sheet_remap(&mut rs.analysis.source_sheet_id, remap);
        match &mut rs.analysis.kind {
            AnalysisKind::PowerAnalysis(power) => {
                apply_opt_sheet_remap(&mut power.info_sheet_id, remap);
            }
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. }) => {
                if let NormalizeAnchor::Hooked(hook) = zero_anchor {
                    remap_hook_source(hook, remap);
                }
                if let NormalizeAnchor::Hooked(hook) = hundred_anchor {
                    remap_hook_source(hook, remap);
                }
            }
            AnalysisKind::NonlinearRegression(fit) => {
                for rule in &mut fit.parameter_rules {
                    if let ParameterScope::HookedConstant(hook) = &mut rule.scope {
                        remap_hook_source(hook, remap);
                    }
                }
            }
            _ => {}
        }
        if let Some(ref mut derived) = rs.analyzable_data {
            remap_datasheet_references(derived, remap);
        }
        for cell_id in &mut rs.eliminated_outliers {
            apply_sheet_remap(&mut cell_id.sheet_id, remap);
        }
        remap_floating_notes(&mut rs.floating_notes, remap);
    }
    for gs in &mut doc.graph_sheets {
        remap_graph_references(gs, remap);
    }
    for ls in &mut doc.layout_sheets {
        for item in &mut ls.spec.items {
            match item {
                LayoutItem::LiveGraph { graph_sheet_id, .. } => apply_sheet_remap(graph_sheet_id, remap),
                LayoutItem::LiveTable { source_sheet_id, .. } => apply_sheet_remap(source_sheet_id, remap),
                LayoutItem::TextCallout { markdown, .. } => remap_template_hooks_in_text(markdown, remap),
                LayoutItem::EmbeddedImage { .. } => {}
            }
        }
        remap_floating_notes(&mut ls.floating_notes, remap);
    }
}

fn remap_datasheet_references(ds: &mut DataSheet, remap: &BTreeMap<SheetId, SheetId>) {
    apply_opt_sheet_remap(&mut ds.info_sheet_id, remap);
    for region in &mut ds.linked_regions {
        if let LinkSource::InternalSheet { ref mut sheet_id, .. } = region.spec.source {
            apply_sheet_remap(sheet_id, remap);
        }
    }
    remap_floating_notes(&mut ds.floating_notes, remap);
}

fn remap_graph_references(gs: &mut curvebook_doc::GraphSheet, remap: &BTreeMap<SheetId, SheetId>) {
    apply_sheet_remap(&mut gs.spec.primary_source_sheet_id, remap);
    remap_template_hooks_in_text(&mut gs.spec.title, remap);
    remap_template_hooks_in_text(&mut gs.spec.x_axis.title, remap);
    remap_template_hooks_in_text(&mut gs.spec.y_axis.title, remap);
    if let Some(ref mut y2) = gs.spec.y2_axis {
        remap_template_hooks_in_text(&mut y2.title, remap);
    }
    for series in &mut gs.spec.series {
        apply_sheet_remap(&mut series.source_sheet_id, remap);
        if let Some(ref mut ov) = series.regression_overlay {
            apply_sheet_remap(&mut ov.result_sheet_id, remap);
        }
    }
    for cmp in &mut gs.spec.comparisons {
        apply_sheet_remap(&mut cmp.result_sheet_id, remap);
    }
    if let Some(ref mut cld) = gs.spec.compact_letter_display {
        apply_sheet_remap(&mut cld.result_sheet_id, remap);
    }
    for ann in &mut gs.spec.annotations {
        match ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                apply_sheet_remap(result_sheet_id, remap);
            }
            GraphAnnotation::TextBox { markdown, .. } => {
                remap_template_hooks_in_text(markdown, remap);
            }
            GraphAnnotation::LineOrArrow { .. } | GraphAnnotation::Shape { .. } | GraphAnnotation::EmbeddedImage { .. } => {}
        }
    }
    remap_floating_notes(&mut gs.floating_notes, remap);
}
