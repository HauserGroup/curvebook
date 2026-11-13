//! Clean-room `.prism` ZIP container importer.
//!
//! Reads bounded ZIP containers carrying CSV data tables, optional JSON sheet/table/info
//! metadata, or project JSON manifests, emitting non-fatal [`ImportWarning`] diagnostics for
//! unrecognized binary or script chunks instead of panicking.

use std::collections::{BTreeMap, HashSet};

use lumencraft_doc::{ConstantValue, DataSheet, InfoConstant, InfoSheet, ProjectDocument, SheetId, SubcolumnMode, TableKind};

use crate::archive::{migrate_document_json, validate_project_document};
use crate::csv::{CsvOptions, import_csv_to_datasheet};
use crate::error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
use crate::zip::{ZipArchive, ZipLimits};

/// Optional metadata sidecar describing a tabular CSV sheet inside a `.prism` ZIP archive.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PrismTableMeta {
    id: Option<String>,
    title: Option<String>,
    #[serde(alias = "table_type", alias = "archetype")]
    kind: Option<String>,
    subcolumn_mode: Option<String>,
    replicates: Option<usize>,
    delimiter: Option<char>,
    decimal_separator: Option<char>,
    has_row_titles: Option<bool>,
    csv_path: Option<String>,
}

/// Optional top-level `.prism` manifest JSON.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PrismManifest {
    title: Option<String>,
    sheets: Option<Vec<PrismTableMeta>>,
}

/// Optional info metadata JSON inside a `.prism` ZIP archive.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PrismInfoMeta {
    id: Option<String>,
    title: Option<String>,
    notes: Option<String>,
    constants: Option<BTreeMap<String, serde_json::Value>>,
}

/// Import a `.prism` ZIP container from raw bytes using [`ZipLimits::default`].
pub fn import_prism_bytes(bytes: &[u8]) -> Result<ImportReport, FormatError> {
    import_prism_bytes_with_limits(bytes, ZipLimits::default())
}

/// Import a `.prism` ZIP container enforcing explicit [`ZipLimits`].
pub fn import_prism_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<ImportReport, FormatError> {
    let archive = ZipArchive::from_bytes_with_limits(bytes, limits)?;
    let mut warnings = Vec::new();

    if let Some(doc_report) = try_load_embedded_project_json(&archive, &mut warnings)? {
        return Ok(doc_report);
    }

    let mut manifest_meta: Option<PrismManifest> = None;
    let mut handled_paths: HashSet<String> = HashSet::new();

    if let Some(manifest_bytes) = archive.get("manifest.json") {
        handled_paths.insert(String::from("manifest.json"));
        match serde_json::from_slice::<PrismManifest>(manifest_bytes) {
            Ok(parsed) => manifest_meta = Some(parsed),
            Err(err) => warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                "manifest.json",
                format!("could not parse manifest.json: {err}"),
            )),
        }
    }

    let proj_title = manifest_meta
        .as_ref()
        .and_then(|m| m.title.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| String::from("Imported Prism Project"));
    let mut doc = ProjectDocument::new(proj_title)?;
    let mut used_ids: HashSet<String> = HashSet::new();

    let mut meta_by_stem: BTreeMap<String, PrismTableMeta> = BTreeMap::new();
    let mut meta_by_csv: BTreeMap<String, PrismTableMeta> = BTreeMap::new();

    if let Some(manifest) = &manifest_meta
        && let Some(sheets) = &manifest.sheets
    {
        for item in sheets {
            if let Some(csv_path) = &item.csv_path {
                meta_by_csv.insert(csv_path.clone(), item.clone());
            }
        }
    }

    // First pass: parse JSON metadata / info sheets.
    for (path, payload) in archive.iter() {
        if handled_paths.contains(path) || !path.ends_with(".json") {
            continue;
        }
        if is_info_json_path(path) {
            handled_paths.insert(path.to_owned());
            import_info_json(path, payload, &mut doc, &mut used_ids, &mut warnings)?;
            continue;
        }
        if let Ok(meta) = serde_json::from_slice::<PrismTableMeta>(payload)
            && (meta.kind.is_some() || meta.title.is_some() || meta.csv_path.is_some() || meta.subcolumn_mode.is_some())
        {
            handled_paths.insert(path.to_owned());
            if let Some(csv_p) = &meta.csv_path {
                meta_by_csv.insert(csv_p.clone(), meta.clone());
            }
            meta_by_stem.insert(stem_without_ext(path).to_owned(), meta);
        }
    }

    // Second pass: parse CSV / TSV data tables.
    let mut data_idx = 1_usize;
    for (path, payload) in archive.iter() {
        if handled_paths.contains(path) || !(path.ends_with(".csv") || path.ends_with(".tsv")) {
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
        let meta = meta_by_csv.get(path).or_else(|| meta_by_stem.get(stem_without_ext(path))).cloned().unwrap_or_default();
        let ds = import_csv_entry(path, text, &meta, data_idx, &mut used_ids, &mut warnings)?;
        doc.add_data_sheet(ds)?;
        data_idx += 1;
    }

    // Third pass: warn on any unrecognized binary/script/other chunks without panicking.
    for (path, _) in archive.iter() {
        if !handled_paths.contains(path) {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedChunk,
                path,
                format!("unsupported or binary `.prism` chunk `{path}` was skipped"),
            ));
        }
    }

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

fn try_load_embedded_project_json(archive: &ZipArchive, warnings: &mut Vec<ImportWarning>) -> Result<Option<ImportReport>, FormatError> {
    let Some(doc_bytes) = archive.get("document.json") else {
        return Ok(None);
    };
    let Ok(val) = serde_json::from_slice::<serde_json::Value>(doc_bytes) else {
        return Ok(None);
    };
    if val.get("data_sheets").is_none() {
        return Ok(None);
    }
    let version = val
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(lumencraft_doc::CURRENT_SCHEMA_VERSION);
    let migrated = migrate_document_json(val, version)?;
    let doc: ProjectDocument = serde_json::from_value(migrated)?;
    validate_project_document(&doc)?;

    for (path, _) in archive.iter() {
        if path != "document.json" && path != "manifest.json" && !path.starts_with("sheets/") {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedChunk,
                path,
                format!("auxiliary `.prism` chunk `{path}` was ignored because `document.json` provided full project state"),
            ));
        }
    }
    Ok(Some(ImportReport { document: doc, warnings: std::mem::take(warnings) }))
}

fn is_info_json_path(path: &str) -> bool {
    path.starts_with("info/") || path.ends_with("/info.json") || path == "info.json"
}

fn stem_without_ext(path: &str) -> &str {
    path.rsplit_once('.').map_or(path, |(stem, _)| stem)
}

fn import_info_json(
    path: &str,
    payload: &[u8],
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    let meta: PrismInfoMeta = match serde_json::from_slice(payload) {
        Ok(m) => m,
        Err(err) => {
            warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, format!("could not parse info JSON `{path}`: {err}")));
            return Ok(());
        }
    };
    let idx = doc.info_sheets.len() + 1;
    let id = allocate_unique_id(meta.id.as_deref(), "info", idx, used_ids);
    let title = meta.title.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| format!("Info {idx}"));
    let mut sheet = InfoSheet::new_preseeded(id, title)?;
    sheet.constants.clear();
    if let Some(map) = meta.constants {
        for (k, v) in map {
            if k.trim().is_empty() {
                continue;
            }
            let cval = match v {
                serde_json::Value::Null => ConstantValue::Empty,
                serde_json::Value::Number(n) => match n.as_f64().filter(|x| x.is_finite()) {
                    Some(f) => ConstantValue::Number(f),
                    None => ConstantValue::Text(n.to_string()),
                },
                serde_json::Value::String(s) => ConstantValue::Text(s),
                serde_json::Value::Bool(b) => ConstantValue::Text(b.to_string()),
                other => ConstantValue::Text(other.to_string()),
            };
            sheet.constants.push(InfoConstant::new(k, cval, None)?);
        }
    }
    if let Some(notes) = meta.notes {
        sheet.notes_markdown = notes;
    }
    doc.add_info_sheet(sheet)?;
    Ok(())
}

fn import_csv_entry(
    path: &str,
    text: &str,
    meta: &PrismTableMeta,
    idx: usize,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<DataSheet, FormatError> {
    let fallback_title = path.rsplit('/').next().map(stem_without_ext).filter(|s| !s.is_empty()).unwrap_or("Table");
    let id = allocate_unique_id(meta.id.as_deref(), "data", idx, used_ids);
    let title = meta.title.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| fallback_title.to_owned());

    let kind = parse_table_kind(meta.kind.as_deref(), path, warnings);
    let subcolumn_mode = parse_subcolumn_mode(kind, meta.subcolumn_mode.as_deref(), meta.replicates.unwrap_or(1), path, warnings);

    let mut opts = if path.ends_with(".tsv") { CsvOptions::tsv() } else { CsvOptions::detect(text) };
    if let Some(d) = meta.delimiter {
        opts.delimiter = d;
    }
    if let Some(dec) = meta.decimal_separator {
        opts.decimal_separator = dec;
    }
    if let Some(has_rt) = meta.has_row_titles {
        opts.has_row_titles = has_rt;
    }

    import_csv_to_datasheet(SheetId::new(id), title, text, kind, subcolumn_mode, &opts)
}

fn parse_table_kind(raw: Option<&str>, source: &str, warnings: &mut Vec<ImportWarning>) -> TableKind {
    match raw.map(str::trim).unwrap_or("") {
        "" | "Column" | "column" | "OneWay" => TableKind::Column,
        "XY" | "Xy" | "xy" => TableKind::Xy,
        "Grouped" | "grouped" | "TwoWay" => TableKind::Grouped,
        "Contingency" | "contingency" => TableKind::Contingency,
        "Survival" | "survival" => TableKind::Survival,
        "PartsOfWhole" | "parts_of_whole" | "parts-of-whole" => TableKind::PartsOfWhole,
        "MultipleVariables" | "multiple_variables" | "multiple-variables" => TableKind::MultipleVariables,
        "Nested" | "nested" => TableKind::Nested,
        other => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedTableType,
                source,
                format!("unrecognized `.prism` table kind `{other}`, defaulting to Column"),
            ));
            TableKind::Column
        }
    }
}

fn parse_subcolumn_mode(kind: TableKind, raw: Option<&str>, replicates: usize, source: &str, warnings: &mut Vec<ImportWarning>) -> SubcolumnMode {
    let candidate = match raw.map(str::trim).unwrap_or("") {
        "" | "Single" | "single" => {
            if matches!(kind, TableKind::Grouped | TableKind::Nested) {
                SubcolumnMode::RawReplicates(replicates.max(1))
            } else {
                SubcolumnMode::Single
            }
        }
        "RawReplicates" | "replicates" => SubcolumnMode::RawReplicates(replicates.max(1)),
        "MeanSdN" | "SD" => SubcolumnMode::MeanSdN,
        "MeanSemN" | "SEM" => SubcolumnMode::MeanSemN,
        "MeanCvN" | "CV" => SubcolumnMode::MeanCvN,
        "MeanLowHigh" => SubcolumnMode::MeanLowHigh,
        "UpperLowerLimits" => SubcolumnMode::UpperLowerLimits,
        "MedianIqr" => SubcolumnMode::MedianIqr,
        other => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                source,
                format!("unrecognized `.prism` subcolumn mode `{other}`, using default"),
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
        SubcolumnMode::RawReplicates(replicates.max(1))
    } else {
        SubcolumnMode::Single
    }
}

fn allocate_unique_id(raw: Option<&str>, prefix: &str, idx: usize, used: &mut HashSet<String>) -> String {
    let base = raw.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| format!("{prefix}-{idx}"));
    let mut candidate = base.clone();
    let mut suffix = 1_usize;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    candidate
}
