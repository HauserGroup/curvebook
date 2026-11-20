//! Clean-room `.prism` ZIP container importer.
//!
//! Reads bounded ZIP containers carrying CSV data tables, optional JSON sheet/table/info
//! metadata, or project JSON manifests, emitting non-fatal [`ImportWarning`] diagnostics for
//! unrecognized binary or script chunks instead of panicking.

use std::collections::{BTreeMap, HashSet};

use curvebook_doc::{
    AnalysisKind, ConstantValue, DataSheet, GraphAnnotation, InfoConstant, InfoSheet, LayoutItem, LinkSource, ProjectDocument, SheetId,
    SubcolumnMode, TableKind,
};

use crate::archive::{BoundedJsonOutcome, migrate_document_json, parse_bounded_json_outcome, validate_project_document};
use crate::csv::{CsvOptions, import_csv_to_datasheet_with_budget, parse_delimited_records};
use crate::error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
use crate::pzfx::{allocate_valid_unique_sheet_id, import_pzfx_bytes_with_used_cells};
use crate::zip::{ZipArchive, ZipLimits};

/// Maximum cumulative dense `Cell` entries allocated across all tables of a `.prism` import.
pub const MAX_PROJECT_CELLS: usize = crate::csv::MAX_IMPORT_CELLS;

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
    info_sheets: Option<Vec<PrismInfoMeta>>,
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
    let manifest_state = inspect_manifest_bounded(&archive)?;
    let mut warnings = Vec::new();

    if let Some(doc_report) = try_load_embedded_project_json(&archive, &mut warnings)? {
        return Ok(doc_report);
    }

    let mut manifest_meta: Option<PrismManifest> = None;
    let mut handled_paths: HashSet<String> = HashSet::new();

    match manifest_state {
        ManifestParseState::Absent => {}
        ManifestParseState::ValidValue(val) => {
            handled_paths.insert(String::from("manifest.json"));
            match serde_json::from_value::<PrismManifest>(val) {
                Ok(parsed) => manifest_meta = Some(parsed),
                Err(err) => warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    "manifest.json",
                    format!("could not parse manifest.json: {err}"),
                )),
            }
        }
        ManifestParseState::InvalidSyntax(err) => {
            handled_paths.insert(String::from("manifest.json"));
            warnings.push(ImportWarning::new(ImportWarningKind::MissingMetadata, "manifest.json", format!("could not parse manifest.json: {err}")));
        }
    }

    let proj_title =
        manifest_meta.as_ref().and_then(|m| m.title.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| String::from("Imported Project"));
    let mut doc = ProjectDocument::new(proj_title)?;
    let mut used_ids: HashSet<String> = HashSet::new();

    let mut meta_by_stem: BTreeMap<String, PrismTableMeta> = BTreeMap::new();
    let mut meta_by_csv: BTreeMap<String, PrismTableMeta> = BTreeMap::new();

    if let Some(manifest) = &manifest_meta {
        if let Some(sheets) = &manifest.sheets {
            for item in sheets {
                if let Some(csv_path) = &item.csv_path {
                    meta_by_csv.insert(csv_path.clone(), item.clone());
                }
            }
        }
        if let Some(infos) = &manifest.info_sheets {
            for meta in infos {
                import_info_meta(meta, &mut doc, &mut used_ids, &mut warnings)?;
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
        if let Some(meta) = parse_table_meta_bounded(payload)?
            && (meta.kind.is_some() || meta.title.is_some() || meta.csv_path.is_some() || meta.subcolumn_mode.is_some())
        {
            handled_paths.insert(path.to_owned());
            if let Some(csv_p) = &meta.csv_path {
                meta_by_csv.insert(csv_p.clone(), meta.clone());
            }
            meta_by_stem.insert(stem_without_ext(path).to_owned(), meta);
        }
    }

    // Second pass: parse CSV / TSV data tables while enforcing cumulative project cell budget.
    let mut used_cells: usize = 0;
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
        let remaining_cells = MAX_PROJECT_CELLS.saturating_sub(used_cells);
        let ds = import_csv_entry(path, text, &meta, data_idx, &mut used_ids, &mut warnings, remaining_cells)?;
        used_cells = accumulate_project_cells(used_cells, &ds)?;
        doc.add_data_sheet(ds)?;
        data_idx += 1;
    }

    // Embedded `.pzfx` payloads inside a `.prism` ZIP container (if any).
    for (path, payload) in archive.iter() {
        if handled_paths.contains(path) || !path.ends_with(".pzfx") {
            continue;
        }
        handled_paths.insert(path.to_owned());
        merge_embedded_pzfx(payload, &mut doc, &mut used_ids, &mut warnings, &mut used_cells)?;
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

enum ManifestParseState {
    Absent,
    ValidValue(serde_json::Value),
    InvalidSyntax(FormatError),
}

fn inspect_manifest_bounded(archive: &ZipArchive) -> Result<ManifestParseState, FormatError> {
    let Some(manifest_bytes) = archive.get("manifest.json") else {
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
            Ok(ManifestParseState::ValidValue(manifest_val))
        }
        BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
        BoundedJsonOutcome::SyntaxOrType(err) => Ok(ManifestParseState::InvalidSyntax(err)),
    }
}

fn parse_table_meta_bounded(payload: &[u8]) -> Result<Option<PrismTableMeta>, FormatError> {
    match parse_bounded_json_outcome(payload) {
        BoundedJsonOutcome::Value(val) => Ok(serde_json::from_value(val).ok()),
        BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
        BoundedJsonOutcome::SyntaxOrType(_) => Ok(None),
    }
}

fn try_load_embedded_project_json(archive: &ZipArchive, warnings: &mut Vec<ImportWarning>) -> Result<Option<ImportReport>, FormatError> {
    let Some(doc_bytes) = archive.get("document.json") else {
        return Ok(None);
    };
    let val = match parse_bounded_json_outcome(doc_bytes) {
        BoundedJsonOutcome::Value(val) => val,
        BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
        BoundedJsonOutcome::SyntaxOrType(_) => return Ok(None),
    };
    if val.get("data_sheets").is_none() {
        return Ok(None);
    }
    let version = match val.get("schema_version") {
        None | Some(serde_json::Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(curvebook_doc::CURRENT_SCHEMA_VERSION),
    };
    let migrated = migrate_document_json(val, version)?;
    let mut doc: ProjectDocument = serde_json::from_value(migrated)?;
    for asset in &mut doc.image_assets {
        if asset.bytes.is_empty() {
            let expected_path = format!("assets/{}.{}", asset.asset_id, asset.media_type.extension());
            if let Some(bytes) = archive.get(&expected_path) {
                asset.bytes = bytes.to_vec();
            }
        }
    }
    doc.data_sheets.iter().try_fold(0_usize, accumulate_project_cells)?;
    sanitize_invalid_document_sheet_ids(&mut doc, warnings);
    validate_project_document(&doc)?;

    for (path, _) in archive.iter() {
        if path != "document.json" && path != "manifest.json" && !path.starts_with("sheets/") && !path.starts_with("assets/") {
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
    let meta: PrismInfoMeta = match serde_json::from_value(val) {
        Ok(m) => m,
        Err(err) => {
            warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, format!("could not parse info JSON `{path}`: {err}")));
            return Ok(());
        }
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
    let title = meta.title.as_deref().map(str::trim).filter(|s| !s.is_empty()).map_or_else(|| format!("Info {idx}"), str::to_owned);
    let mut sheet = InfoSheet::new_preseeded(id.as_str().to_owned(), title)?;
    sheet.constants.clear();
    if let Some(map) = &meta.constants {
        for (k, v) in map {
            if k.trim().is_empty() {
                continue;
            }
            let (cval, units) = parse_prism_constant_value(v);
            sheet.constants.push(InfoConstant::new(k.clone(), cval, units)?);
        }
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
    text: &str,
    meta: &PrismTableMeta,
    idx: usize,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    remaining_cells: usize,
) -> Result<DataSheet, FormatError> {
    let fallback_title = path.rsplit('/').next().map(stem_without_ext).filter(|s| !s.is_empty()).unwrap_or("Table");
    let id = allocate_valid_unique_sheet_id(meta.id.as_deref(), "data", idx, used_ids, warnings);
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
    } else if csv_has_explicit_row_title_header(text, kind, &opts) {
        opts.has_row_titles = true;
    }

    import_csv_to_datasheet_with_budget(id, title, text, kind, subcolumn_mode, &opts, remaining_cells)
}

fn csv_has_explicit_row_title_header(text: &str, kind: TableKind, opts: &CsvOptions) -> bool {
    let prefix: String = text.lines().take(2).collect::<Vec<_>>().join("\n");
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
    first_hdr.eq_ignore_ascii_case("Row Title")
        || (kind == TableKind::Contingency && first_hdr.eq_ignore_ascii_case("Outcome / Exposure"))
        || (kind == TableKind::Grouped && first_hdr.eq_ignore_ascii_case("Row Factor"))
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

fn remap_document_sheet_references(doc: &mut ProjectDocument, remap: &BTreeMap<SheetId, SheetId>) {
    apply_opt_sheet_remap(&mut doc.active_sheet_id, remap);
    apply_opt_sheet_remap(&mut doc.family_anchor_sheet_id, remap);
    for info in &mut doc.info_sheets {
        for linked in &mut info.linked_data_sheets {
            apply_sheet_remap(linked, remap);
        }
    }
    for ds in &mut doc.data_sheets {
        remap_datasheet_references(ds, remap);
    }
    for rs in &mut doc.result_sheets {
        apply_sheet_remap(&mut rs.analysis.id, remap);
        apply_sheet_remap(&mut rs.analysis.source_sheet_id, remap);
        if let AnalysisKind::PowerAnalysis(ref mut power) = rs.analysis.kind {
            apply_opt_sheet_remap(&mut power.info_sheet_id, remap);
        }
        if let Some(ref mut derived) = rs.analyzable_data {
            remap_datasheet_references(derived, remap);
        }
    }
    for gs in &mut doc.graph_sheets {
        remap_graph_references(gs, remap);
    }
    for ls in &mut doc.layout_sheets {
        for item in &mut ls.spec.items {
            match item {
                LayoutItem::LiveGraph { graph_sheet_id, .. } => apply_sheet_remap(graph_sheet_id, remap),
                LayoutItem::LiveTable { source_sheet_id, .. } => apply_sheet_remap(source_sheet_id, remap),
                LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => {}
            }
        }
    }
}

fn remap_datasheet_references(ds: &mut DataSheet, remap: &BTreeMap<SheetId, SheetId>) {
    apply_opt_sheet_remap(&mut ds.info_sheet_id, remap);
    for region in &mut ds.linked_regions {
        if let LinkSource::InternalSheet { ref mut sheet_id, .. } = region.spec.source {
            apply_sheet_remap(sheet_id, remap);
        }
    }
}

fn remap_graph_references(gs: &mut curvebook_doc::GraphSheet, remap: &BTreeMap<SheetId, SheetId>) {
    apply_sheet_remap(&mut gs.spec.primary_source_sheet_id, remap);
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
        if let GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } = ann {
            apply_sheet_remap(result_sheet_id, remap);
        }
    }
}
