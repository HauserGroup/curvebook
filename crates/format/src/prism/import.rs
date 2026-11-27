//! Lazy `.prism` ZIP container importer routing across CurveBook-native embedded projects,
//! external ID-indexed `.prism` archives (§5.1–§5.3), and legacy generic `manifest.json`/CSV
//! containers.

use std::collections::{BTreeMap, HashSet};

use curvebook_doc::{
    Cell, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSetColumn, DataSheet, GroupedTable, InfoConstant, InfoSheet,
    LinkSource, MAX_DATA_SETS, MAX_IMAGE_ASSET_BYTES, MAX_PROJECT_IMAGE_ASSETS, MAX_ROWS, MAX_SHEET_CELLS, MAX_SUBCOLUMNS, MultipleVariablesTable,
    NestedGroupColumn, NestedTable, PartsOfWholeTable, ProjectDocument, SequenceProgression, SheetId, SubcolumnMode, SurvivalTable, TableArchetype,
    TableKind, VariableColumn, XColumnMode, XyTable, is_valid_asset_id, remap_document_sheet_references,
};

use crate::archive::{
    BoundedJsonOutcome, MAX_ARCHIVE_FILE_BYTES, StreamedDocumentOutcome, clamp_peek_schema_version, finalize_migrated_document,
    migrate_document_json, parse_bounded_json_outcome, parse_streamed_document_outcome, validate_archive_save_limits, validate_project_document,
};
use crate::csv::{CsvOptions, import_csv_to_datasheet_with_budget, parse_cell_token, parse_delimited_records};
use crate::error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
use crate::image_asset::validate_image_asset_parts;
use crate::prism::analyses::{AnalysisImportContext, ExternalDataSetPositions, import_external_analyses};
use crate::prism::graphs::{GraphImportContext, import_external_graphs};
use crate::prism::layouts::import_external_layouts;
use crate::prism::schema::{
    ArchiveIdIndex, ExternalDataSetJson, ExternalDataSheetJson, ExternalDocumentIndex, ExternalInfoSheetJson, ExternalSubColumnTitlesJson, IdLookup,
    has_ext_ci, is_external_indexed_document, looks_like_external_indexed_document_bytes, parse_strict_iso8601_date,
};
use crate::pzfx::{allocate_valid_unique_sheet_id, clamp_title, import_pzfx_bytes_with_used_cells};
use crate::zip::{MAX_ZIP_ENTRY_BYTES, MAX_ZIP_UNCOMPRESSED_BYTES, ZipEntryClass, ZipIndex, ZipLimits, ZipReadOptions};

/// Maximum cumulative dense `Cell` entries allocated across all tables of a `.prism` import.
pub const MAX_PROJECT_CELLS: usize = crate::csv::MAX_IMPORT_CELLS;

/// Maximum lazy ZIP central-directory entries allowed for external `.prism` archive imports.
///
/// External ID-indexed `.prism` archives store each table, replicate set, analysis, graph, and
/// preview in its own ZIP entry, so valid multi-study archives can exceed 10,000 entries. Because
/// [`ZipIndex`] reads entries lazily and enforces strict cumulative *decompressed* byte caps
/// ([`MAX_ZIP_UNCOMPRESSED_BYTES`] / [`MAX_ZIP_ENTRY_BYTES`]) plus per-project cell/sheet/dataset
/// limits, raising only the lazy entry-count cap for `.prism` preserves ZIP-bomb protection while
/// keeping native `.curvebook`/`.cbtemplate` [`ZipLimits::default`] (`4_096` entries) unchanged.
pub const MAX_ARCHIVE_ZIP_ENTRIES: usize = 32_768;

/// Default [`ZipLimits`] used by [`import_prism_bytes`] for lazy external `.prism` archive imports.
#[must_use]
pub const fn external_archive_zip_limits() -> ZipLimits {
    ZipLimits {
        max_entries: MAX_ARCHIVE_ZIP_ENTRIES,
        max_total_uncompressed_bytes: MAX_ZIP_UNCOMPRESSED_BYTES,
        max_entry_uncompressed_bytes: MAX_ZIP_ENTRY_BYTES,
    }
}

/// Import a `.prism` ZIP container from raw bytes using [`external_archive_zip_limits`].
pub fn import_prism_bytes(bytes: &[u8]) -> Result<ImportReport, FormatError> {
    import_prism_bytes_with_limits(bytes, external_archive_zip_limits())
}

/// Import a `.prism` ZIP container enforcing explicit [`ZipLimits`].
pub fn import_prism_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<ImportReport, FormatError> {
    if bytes.len() > MAX_ARCHIVE_FILE_BYTES {
        return Err(FormatError::FileTooLarge {
            path: String::from(".prism"),
            actual_bytes: Some(bytes.len() as u64),
            limit_bytes: MAX_ARCHIVE_FILE_BYTES,
        });
    }
    if crate::pzfx::is_binary_pzf_bytes(bytes) {
        return Err(crate::pzfx::binary_pzf_unsupported_error());
    }
    let mut zip_index = ZipIndex::from_bytes_with_options(bytes, limits, ZipReadOptions::external_interop())?;
    let root_prefix = detect_archive_root_prefix(&zip_index);
    let mut warnings = Vec::new();

    let report = match route_document_json(&mut zip_index, &root_prefix, &mut warnings)? {
        DocumentRoute::NativeEmbedded(report) => *report,
        DocumentRoute::ExternalIndexed { doc_val } => import_external_indexed_archive(&mut zip_index, &doc_val, warnings)?,
        DocumentRoute::Generic { doc_meta, handled_doc_path } => {
            import_generic_archive(&mut zip_index, &root_prefix, doc_meta, handled_doc_path, warnings)?
        }
    };
    validate_archive_save_limits(&report.document)?;
    Ok(report)
}

enum DocumentRoute {
    NativeEmbedded(Box<ImportReport>),
    ExternalIndexed { doc_val: serde_json::Value },
    Generic { doc_meta: Option<(String, serde_json::Value)>, handled_doc_path: Option<String> },
}

fn route_document_json(zip_index: &mut ZipIndex<'_>, root_prefix: &str, warnings: &mut Vec<ImportWarning>) -> Result<DocumentRoute, FormatError> {
    let Some(doc_path) = find_document_json_path(zip_index, root_prefix) else {
        return Ok(DocumentRoute::Generic { doc_meta: None, handled_doc_path: None });
    };
    let Some(raw_doc_bytes) = zip_index.read_entry(&doc_path)? else {
        return Ok(DocumentRoute::Generic { doc_meta: None, handled_doc_path: None });
    };
    let doc_bytes = raw_doc_bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&raw_doc_bytes);

    let streamed = match parse_streamed_document_outcome(doc_bytes) {
        StreamedDocumentOutcome::Parsed(doc) => *doc,
        StreamedDocumentOutcome::BudgetExceeded(err) | StreamedDocumentOutcome::DataError(err) => return Err(err),
        StreamedDocumentOutcome::NonObject | StreamedDocumentOutcome::SyntaxOrType(_) => {
            return match parse_bounded_json_outcome(doc_bytes) {
                BoundedJsonOutcome::Value(value) => {
                    if is_external_indexed_document(&value) {
                        Ok(DocumentRoute::ExternalIndexed { doc_val: value })
                    } else {
                        Ok(DocumentRoute::Generic { doc_meta: Some((doc_path.clone(), value)), handled_doc_path: Some(doc_path) })
                    }
                }
                BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
                BoundedJsonOutcome::SyntaxOrType(err) => {
                    if looks_like_external_indexed_document_bytes(doc_bytes, zip_index) {
                        return Err(FormatError::Json(format!("malformed external `.prism` index `{doc_path}`: {err}")));
                    }
                    warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, &doc_path, format!("could not parse `{doc_path}`: {err}")));
                    Ok(DocumentRoute::Generic { doc_meta: None, handled_doc_path: Some(doc_path) })
                }
            };
        }
    };

    if !streamed.has_data_sheets_key {
        let value = match parse_bounded_json_outcome(doc_bytes) {
            BoundedJsonOutcome::Value(v) => v,
            BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
            BoundedJsonOutcome::SyntaxOrType(err) => {
                if looks_like_external_indexed_document_bytes(doc_bytes, zip_index) {
                    return Err(FormatError::Json(format!("malformed external `.prism` index `{doc_path}`: {err}")));
                }
                serde_json::Value::Object(serde_json::Map::new())
            }
        };
        if is_external_indexed_document(&value) {
            return Ok(DocumentRoute::ExternalIndexed { doc_val: value });
        }
        return Ok(DocumentRoute::Generic { doc_meta: Some((doc_path.clone(), value)), handled_doc_path: Some(doc_path) });
    }

    // Native CurveBook embedded `document.json`: check legacy LumenCraft manifest first.
    if let Some(manifest_path) = find_archive_path_ci(zip_index, "manifest.json", root_prefix)
        && let Some(manifest_bytes) = zip_index.read_entry(&manifest_path)?
    {
        let _ = inspect_manifest_bounded(Some((manifest_path, manifest_bytes)))?;
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
    hydrate_embedded_image_assets(zip_index, root_prefix, &mut doc)?;
    sanitize_invalid_document_sheet_ids(&mut doc, warnings);
    validate_project_document(&doc)?;

    let all_names: Vec<String> = zip_index.entry_names().map(str::to_owned).collect();
    for path in all_names {
        let rel = strip_root_prefix(&path, root_prefix);
        if !rel.eq_ignore_ascii_case("document.json")
            && !rel.eq_ignore_ascii_case("manifest.json")
            && !starts_with_dir_ci(rel, &["sheets/", "assets/"])
        {
            let msg = if is_sqlite_path(&path) {
                format!("embedded SQLite database `{path}` was ignored because `document.json` provided full project state")
            } else {
                format!("auxiliary `.prism` chunk `{path}` was ignored because `document.json` provided full project state")
            };
            warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, msg));
        }
    }
    Ok(DocumentRoute::NativeEmbedded(Box::new(ImportReport::new(doc, std::mem::take(warnings)))))
}

// ============================================================================
// External ID-Indexed `.prism` Importer (§5.1, §5.2, §5.3)
// ============================================================================

fn import_external_indexed_archive(
    zip_index: &mut ZipIndex<'_>,
    doc_val: &serde_json::Value,
    mut warnings: Vec<ImportWarning>,
) -> Result<ImportReport, FormatError> {
    let id_index = ArchiveIdIndex::from_zip_index(zip_index);
    let doc_index = ExternalDocumentIndex::from_json(doc_val);

    let raw_proj_title = doc_index.title.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| String::from("Imported Project"));
    let proj_title = clamp_title(&raw_proj_title, "project", &mut warnings);
    let mut doc = ProjectDocument::new(proj_title)?;
    let mut used_ids: HashSet<String> = HashSet::new();
    let mut used_cells: usize = 0;
    let mut data_uid_to_sheet_id: BTreeMap<String, SheetId> = BTreeMap::new();
    let mut data_set_positions = ExternalDataSetPositions::new();
    let mut uid_to_sheet_id: BTreeMap<String, SheetId> = BTreeMap::new();

    // 1. Data sheets (listed in `sheets.data`, skipping `view` / `ANALYSIS_VIEW` sheets).
    for (idx, sheet_uid) in doc_index.data_sheet_ids.iter().enumerate() {
        let Some((ds, data_set_uids)) =
            import_external_data_sheet(zip_index, &id_index, &doc_index, sheet_uid, idx, &mut used_ids, &mut used_cells, &mut warnings)?
        else {
            continue;
        };
        for (pos, ds_uid) in data_set_uids.into_iter().enumerate() {
            data_set_positions.insert(ds_uid, (sheet_uid.clone(), pos));
        }
        data_uid_to_sheet_id.insert(sheet_uid.clone(), ds.id.clone());
        uid_to_sheet_id.insert(sheet_uid.clone(), ds.id.clone());
        doc.add_data_sheet(ds)?;
    }

    // 2. Info sheets (`info/<uid>/sheet.json`).
    import_external_info_sheets(
        zip_index,
        &id_index,
        &doc_index,
        &data_uid_to_sheet_id,
        &mut uid_to_sheet_id,
        &mut doc,
        &mut used_ids,
        &mut warnings,
    )?;

    // 3. Analyses (`analyses/<uid>/` via 3-hop `preferredResultSheet` chain).
    let mut promoted_analyses = Vec::new();
    let mut analysis_ctx = AnalysisImportContext {
        id_index: &id_index,
        doc_index: &doc_index,
        data_uid_to_sheet_id: &data_uid_to_sheet_id,
        data_set_positions: &data_set_positions,
        uid_to_sheet_id: &mut uid_to_sheet_id,
        used_ids: &mut used_ids,
        warnings: &mut warnings,
        promoted_analyses: &mut promoted_analyses,
    };
    import_external_analyses(zip_index, &mut doc, &mut analysis_ctx)?;

    // 4. Graphs & layouts (basic editable graph reconstruction from `inputDataSets` +
    // `floatingNotes` text annotations, raster preview/media ingestion, and opaque binary warnings).
    let mut graph_ctx = GraphImportContext {
        id_index: &id_index,
        doc_index: &doc_index,
        data_uid_to_sheet_id: &data_uid_to_sheet_id,
        data_set_positions: &data_set_positions,
        uid_to_sheet_id: &mut uid_to_sheet_id,
        used_ids: &mut used_ids,
        warnings: &mut warnings,
    };
    import_external_graphs(zip_index, &mut doc, &mut graph_ctx)?;
    import_external_layouts(zip_index, &id_index, &doc_index, &mut doc, &mut uid_to_sheet_id, &mut used_ids, &mut warnings)?;

    // 5. Restore navigator selection if `uiSettings.currentSheets` resolves to an imported sheet,
    // otherwise fall back to the first imported sheet across data/info/result/graph/layout folders.
    if let Some(ref selected_uid) = doc_index.selected_sheet_uid
        && let Some(mapped_id) = uid_to_sheet_id.get(selected_uid)
    {
        let _ = doc.select_in_navigator(mapped_id);
    }
    if doc.active_sheet_id.is_none() {
        let fallback_id = doc
            .data_sheets
            .first()
            .map(|s| s.id.clone())
            .or_else(|| doc.info_sheets.first().map(|s| s.id.clone()))
            .or_else(|| doc.result_sheets.first().map(|s| s.id.clone()))
            .or_else(|| doc.graph_sheets.first().map(|s| s.id.clone()))
            .or_else(|| doc.layout_sheets.first().map(|s| s.id.clone()));
        if let Some(id) = fallback_id {
            let _ = doc.select_in_navigator(&id);
        }
    }

    // 6. Report unread/opaque entries (e.g. `.dt`, `.bin`, `.cache`, `misc/*`) via `ZipIndex`
    // without decompressing them.
    report_unread_opaque_entries(zip_index, &mut warnings);

    let remap = sanitize_invalid_document_sheet_ids(&mut doc, &mut warnings);
    if !remap.is_empty() {
        for pref in &mut promoted_analyses {
            if let Some(new_id) = remap.get(&pref.sheet_id) {
                pref.sheet_id = new_id.clone();
            }
        }
    }
    validate_project_document(&doc)?;
    Ok(ImportReport { document: doc, warnings, promoted_analyses })
}

fn report_unread_opaque_entries(zip_index: &ZipIndex<'_>, warnings: &mut Vec<ImportWarning>) {
    let mut counts: BTreeMap<ZipEntryClass, usize> = BTreeMap::new();
    for path in zip_index.unread_entry_names() {
        if path.starts_with("__MACOSX/") {
            continue;
        }
        let class = crate::zip::classify_zip_entry_name(path);
        *counts.entry(class).or_insert(0) += 1;
    }
    for (class, count) in counts {
        if count == 0 {
            continue;
        }
        let label = match class {
            ZipEntryClass::Preview => "preview",
            ZipEntryClass::Cache => "cache",
            ZipEntryClass::Database => "database",
            ZipEntryClass::Other => "opaque/auxiliary",
        };
        warnings.push(ImportWarning::new(
            ImportWarningKind::UnsupportedChunk,
            format!("archive/{label}"),
            format!("skipped {count} unread {label} `.prism` archive entr{} without decompressing", if count == 1 { "y" } else { "ies" }),
        ));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OffsetSubcolMapping {
    Direct,
    /// `.prism` `[col0 = Y, col1 = +Err, col2 = -Err]` -> CurveBook `[Mean, -Err, +Err]`.
    SwapHighLowOffsets,
    /// `.prism` `y_plus_minus`: per-dataset check — 3 replicates swap `[Y, +Err, -Err]`,
    /// otherwise 2 replicates duplicate `[Y, +-Err]`.
    PlusMinusPerDataset,
}

#[derive(Debug, Clone)]
struct ResolvedFormatPlan {
    kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    csv_stride: usize,
    offset_mapping: OffsetSubcolMapping,
}

#[allow(clippy::too_many_arguments)]
fn import_external_data_sheet(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    doc_index: &ExternalDocumentIndex,
    sheet_uid: &str,
    idx: usize,
    used_ids: &mut HashSet<String>,
    used_cells: &mut usize,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Option<(DataSheet, Vec<String>)>, FormatError> {
    let sheet_path = match id_index.find_data_sheet_json(sheet_uid) {
        IdLookup::Found(p) => p,
        IdLookup::Missing => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                sheet_uid,
                format!("data sheet `{sheet_uid}` has no matching `sheet.json`; skipping table"),
            ));
            return Ok(None);
        }
        IdLookup::Ambiguous(candidates) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                sheet_uid,
                format!("ambiguous `sheet.json` paths for data sheet `{sheet_uid}` ({} candidates); skipping table", candidates.len()),
            ));
            return Ok(None);
        }
    };

    let Some(sheet_bytes) = zip_index.read_entry(&sheet_path)? else {
        return Ok(None);
    };
    let sheet_val = match parse_bounded_json_outcome(&sheet_bytes) {
        BoundedJsonOutcome::Value(v) => v,
        BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
        BoundedJsonOutcome::SyntaxOrType(err) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                sheet_path,
                format!("could not parse data `sheet.json` for `{sheet_uid}`: {err}"),
            ));
            return Ok(None);
        }
    };
    let Some(sheet_meta) = ExternalDataSheetJson::from_json(&sheet_val) else {
        return Ok(None);
    };
    if sheet_meta.is_analysis_view() {
        return Ok(None);
    }

    let Some(table_uid) = sheet_meta.table.uid.as_deref().filter(|s| !s.trim().is_empty()) else {
        warnings.push(ImportWarning::new(
            ImportWarningKind::MissingMetadata,
            sheet_uid,
            format!("data sheet `{sheet_uid}` is missing `table.uid`; skipping table"),
        ));
        return Ok(None);
    };

    let csv_path = match id_index.find_table_csv(table_uid) {
        IdLookup::Found(p) => p,
        IdLookup::Missing => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                table_uid,
                format!("data table `{table_uid}` for sheet `{sheet_uid}` has no matching `data.csv`; skipping table"),
            ));
            return Ok(None);
        }
        IdLookup::Ambiguous(candidates) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                table_uid,
                format!("ambiguous `data.csv` paths for table `{table_uid}` ({} candidates); skipping table", candidates.len()),
            ));
            return Ok(None);
        }
    };

    // Resolve row titles, X dataset, Y datasets, and optional subcolumn titles by whole-stem ID.
    let row_titles_set = match sheet_meta.table.row_titles_data_set.as_deref() {
        Some(uid) => match load_required_dataset(zip_index, id_index, uid, sheet_uid, warnings)? {
            Some(ds) => Some(ds),
            None => return Ok(None),
        },
        None => None,
    };
    let x_set = match sheet_meta.table.x_data_set.as_deref() {
        Some(uid) => match load_required_dataset(zip_index, id_index, uid, sheet_uid, warnings)? {
            Some(ds) => Some(ds),
            None => return Ok(None),
        },
        None => None,
    };
    let mut y_sets = Vec::with_capacity(sheet_meta.table.data_sets.len());
    for ds_uid in &sheet_meta.table.data_sets {
        match load_required_dataset(zip_index, id_index, ds_uid, sheet_uid, warnings)? {
            Some(ds) => y_sets.push(ds),
            None => return Ok(None),
        }
    }
    let subcol_titles = match sheet_meta.table.subcolumn_titles_data_set.as_deref() {
        Some(uid) => load_optional_subcol_titles(zip_index, id_index, uid, sheet_uid, warnings)?,
        None => None,
    };

    let Some(plan) = resolve_external_table_plan(&sheet_meta, &y_sets, sheet_uid, warnings) else {
        return Ok(None);
    };

    let Some(csv_bytes) = zip_index.read_entry(&csv_path)? else {
        return Ok(None);
    };
    let csv_text = std::str::from_utf8(&csv_bytes)
        .map_err(|err| FormatError::InvalidZip { reason: format!("table CSV `{csv_path}` is not valid UTF-8: {err}") })?;
    let csv_opts = CsvOptions { header_rows: 0, has_row_titles: false, parse_asterisk_exclusions: true, ..CsvOptions::csv() };
    let records = parse_delimited_records(csv_text, &csv_opts)?;

    let row_count = records.len().max(1);
    if row_count > MAX_ROWS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyRows { count: row_count, max: MAX_ROWS }));
    }
    let ds_count = y_sets.len().max(1);
    if ds_count > MAX_DATA_SETS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyDataSets { count: ds_count, max: MAX_DATA_SETS }));
    }
    let y_subs = plan.subcolumn_mode.subcolumn_count();
    if y_subs > MAX_SUBCOLUMNS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManySubcolumns { count: y_subs, max: MAX_SUBCOLUMNS }));
    }

    let (x_csv_cols, x_mode) = resolve_x_layout(&sheet_meta, x_set.as_ref(), sheet_uid, warnings);
    let projected_cells = estimate_external_sheet_cells(plan.kind, row_count, x_mode.subcolumn_count(), ds_count, y_subs).ok_or_else(|| {
        FormatError::InvalidPipeline { reason: format!("sheet `{sheet_uid}` cell allocation overflows project budget cap ({MAX_PROJECT_CELLS})") }
    })?;
    if projected_cells > MAX_SHEET_CELLS {
        return Err(FormatError::Doc(curvebook_doc::DocError::TooManyCells { count: projected_cells, max: MAX_SHEET_CELLS }));
    }
    let remaining_budget = MAX_PROJECT_CELLS.saturating_sub(*used_cells);
    if projected_cells > remaining_budget {
        return Err(FormatError::InvalidPipeline {
            reason: format!(
                "sheet `{sheet_uid}` cell allocation ({projected_cells}) exceeds remaining project import budget ({remaining_budget}, cap {MAX_PROJECT_CELLS})"
            ),
        });
    }

    let attr = doc_index.sheet_attrs.get(sheet_uid);
    let raw_title = attr
        .and_then(|a| a.title.clone())
        .or_else(|| sheet_meta.title.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("Table {}", idx + 1));
    let sid = allocate_valid_unique_sheet_id(Some(sheet_uid), "data", idx + 1, used_ids, warnings);
    let title = clamp_title(&raw_title, sid.as_str(), warnings);

    let archetype = build_external_archetype(
        &ExternalBuildInput {
            sheet_meta: &sheet_meta,
            plan: &plan,
            records: &records,
            csv_opts: &csv_opts,
            x_csv_cols,
            x_mode,
            row_titles_set: row_titles_set.as_ref(),
            x_set: x_set.as_ref(),
            y_sets: &y_sets,
            subcol_titles: subcol_titles.as_ref(),
            sheet_uid,
        },
        warnings,
    )?;

    let mut ds = DataSheet::new(sid, title, archetype)?;
    ds.frozen = attr.is_some_and(|a| a.is_frozen);
    ds.color_tag = attr.and_then(|a| a.highlight_color).or(sheet_meta.highlight_color);

    *used_cells = accumulate_project_cells(*used_cells, &ds)?;
    Ok(Some((ds, sheet_meta.table.data_sets)))
}

fn load_required_dataset(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    set_uid: &str,
    sheet_uid: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Option<ExternalDataSetJson>, FormatError> {
    let path = match id_index.find_dataset_json(set_uid) {
        IdLookup::Found(p) => p,
        IdLookup::Missing => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                set_uid,
                format!("dataset `{set_uid}` referenced by sheet `{sheet_uid}` was not found; skipping table"),
            ));
            return Ok(None);
        }
        IdLookup::Ambiguous(candidates) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                set_uid,
                format!(
                    "ambiguous dataset paths for `{set_uid}` referenced by sheet `{sheet_uid}` ({} candidates); skipping table",
                    candidates.len()
                ),
            ));
            return Ok(None);
        }
    };
    let Some(bytes) = zip_index.read_entry(&path)? else {
        return Ok(None);
    };
    match parse_bounded_json_outcome(&bytes) {
        BoundedJsonOutcome::Value(val) => Ok(ExternalDataSetJson::from_json(&val)),
        BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
        BoundedJsonOutcome::SyntaxOrType(err) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                path,
                format!("could not parse dataset `{set_uid}` for sheet `{sheet_uid}`: {err}"),
            ));
            Ok(None)
        }
    }
}

fn load_optional_subcol_titles(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    set_uid: &str,
    sheet_uid: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Option<ExternalSubColumnTitlesJson>, FormatError> {
    let path = match id_index.find_dataset_json(set_uid) {
        IdLookup::Found(p) => p,
        IdLookup::Missing => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                set_uid,
                format!("subcolumn titles dataset `{set_uid}` referenced by sheet `{sheet_uid}` was not found"),
            ));
            return Ok(None);
        }
        IdLookup::Ambiguous(candidates) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                set_uid,
                format!("ambiguous subcolumn titles dataset `{set_uid}` ({} candidates); ignoring", candidates.len()),
            ));
            return Ok(None);
        }
    };
    let Some(bytes) = zip_index.read_entry(&path)? else {
        return Ok(None);
    };
    match parse_bounded_json_outcome(&bytes) {
        BoundedJsonOutcome::Value(val) => Ok(ExternalSubColumnTitlesJson::from_json(&val)),
        BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
        BoundedJsonOutcome::SyntaxOrType(err) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                path,
                format!("could not parse subcolumn titles `{set_uid}`: {err}"),
            ));
            Ok(None)
        }
    }
}

fn resolve_external_table_plan(
    sheet_meta: &ExternalDataSheetJson,
    y_sets: &[ExternalDataSetJson],
    sheet_uid: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Option<ResolvedFormatPlan> {
    let mut kind = match sheet_meta.table.format.as_deref().map(str::trim) {
        Some(f) if f.eq_ignore_ascii_case("xy") => TableKind::Xy,
        Some(f) if f.eq_ignore_ascii_case("column") => TableKind::Column,
        Some(f) if f.eq_ignore_ascii_case("grouped") => TableKind::Grouped,
        Some(f) if f.eq_ignore_ascii_case("survival") => TableKind::Survival,
        Some(f) if f.eq_ignore_ascii_case("contingency") => TableKind::Contingency,
        Some(f) if f.eq_ignore_ascii_case("multivariable") => TableKind::MultipleVariables,
        Some(f) if f.eq_ignore_ascii_case("nested") => TableKind::Nested,
        Some(f) if f.eq_ignore_ascii_case("parts_of_whole") => TableKind::PartsOfWhole,
        None | Some("") => {
            if sheet_meta.table.x_data_set.is_some() {
                TableKind::Xy
            } else {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    sheet_uid,
                    format!("table `{sheet_uid}` omitted `table.format`; assuming Column table"),
                ));
                TableKind::Column
            }
        }
        Some(other) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedTableType,
                sheet_uid,
                format!("unrecognized `.prism` table.format `{other}` on `{sheet_uid}`; skipping table"),
            ));
            return None;
        }
    };

    let raw_fmt = sheet_meta.table.data_format.as_deref().map(str::trim).unwrap_or("y_single");
    let max_ds_reps = y_sets.iter().map(|ds| ds.replicates.len()).max().unwrap_or(0);

    let (mut mode, csv_stride, offset_mapping) = match raw_fmt.to_ascii_lowercase().as_str() {
        "y_single" | "text" => {
            let m = if matches!(kind, TableKind::Grouped | TableKind::Nested) { SubcolumnMode::RawReplicates(1) } else { SubcolumnMode::Single };
            (m, 1, OffsetSubcolMapping::Direct)
        }
        "y_replicates" | "text_replicates" => {
            let reps = match sheet_meta.table.replicates_count {
                Some(0) => 0,
                Some(n) => n.max(max_ds_reps),
                None => max_ds_reps,
            };
            if reps == 0 {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    sheet_uid,
                    format!("table `{sheet_uid}` specifies `{raw_fmt}` with missing or zero replicatesCount; skipping table"),
                ));
                return None;
            }
            let m = if reps == 1
                && !matches!(kind, TableKind::Grouped | TableKind::Nested)
                && !kind.allows_subcolumn_mode(SubcolumnMode::RawReplicates(1))
            {
                SubcolumnMode::Single
            } else {
                SubcolumnMode::RawReplicates(reps)
            };
            (m, reps, OffsetSubcolMapping::Direct)
        }
        "y_sd_n" => (SubcolumnMode::MeanSdN, 3, OffsetSubcolMapping::Direct),
        "y_se_n" => (SubcolumnMode::MeanSemN, 3, OffsetSubcolMapping::Direct),
        // `.prism` CV stores SD directly (§5.1): map losslessly to MeanSdN / MeanSd.
        "y_cv_n" => (SubcolumnMode::MeanSdN, 3, OffsetSubcolMapping::Direct),
        "y_sd" | "y_cv" => (SubcolumnMode::MeanSd, 2, OffsetSubcolMapping::Direct),
        "y_se" => (SubcolumnMode::MeanSem, 2, OffsetSubcolMapping::Direct),
        "y_high_low" => (SubcolumnMode::MeanLowHigh, 3, OffsetSubcolMapping::SwapHighLowOffsets),
        "y_plus_minus" => (SubcolumnMode::MeanLowHigh, 2, OffsetSubcolMapping::PlusMinusPerDataset),
        other => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedTableType,
                sheet_uid,
                format!("unsupported `.prism` dataFormat `{other}` on `{sheet_uid}`; skipping table"),
            ));
            return None;
        }
    };

    if !kind.allows_subcolumn_mode(mode) {
        if kind == TableKind::Column && TableKind::Grouped.allows_subcolumn_mode(mode) {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                sheet_uid,
                format!("Column table `{sheet_uid}` uses subcolumn mode {mode:?}; promoted to Grouped to preserve all subcolumns"),
            ));
            kind = TableKind::Grouped;
        } else if matches!(kind, TableKind::Grouped | TableKind::Nested) && mode == SubcolumnMode::Single {
            mode = SubcolumnMode::RawReplicates(1);
        } else {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedTableType,
                sheet_uid,
                format!("table `{sheet_uid}` archetype {kind:?} does not support subcolumn mode {mode:?}; skipping table"),
            ));
            return None;
        }
    }

    Some(ResolvedFormatPlan { kind, subcolumn_mode: mode, csv_stride, offset_mapping })
}

struct ExternalBuildInput<'a> {
    sheet_meta: &'a ExternalDataSheetJson,
    plan: &'a ResolvedFormatPlan,
    records: &'a [Vec<String>],
    csv_opts: &'a CsvOptions,
    x_csv_cols: usize,
    x_mode: XColumnMode,
    row_titles_set: Option<&'a ExternalDataSetJson>,
    x_set: Option<&'a ExternalDataSetJson>,
    y_sets: &'a [ExternalDataSetJson],
    subcol_titles: Option<&'a ExternalSubColumnTitlesJson>,
    sheet_uid: &'a str,
}

fn estimate_external_sheet_cells(kind: TableKind, row_count: usize, x_subs: usize, ds_count: usize, y_subs: usize) -> Option<usize> {
    match kind {
        TableKind::Xy => {
            let x_cells = row_count.checked_mul(x_subs)?;
            let y_cells = row_count.checked_mul(ds_count)?.checked_mul(y_subs)?;
            x_cells.checked_add(y_cells)
        }
        TableKind::Survival => row_count.checked_add(row_count.checked_mul(ds_count)?),
        TableKind::Column | TableKind::Grouped | TableKind::Contingency | TableKind::PartsOfWhole => {
            row_count.checked_mul(ds_count)?.checked_mul(y_subs)
        }
        TableKind::MultipleVariables => row_count.checked_mul(ds_count),
        TableKind::Nested => row_count.checked_mul(ds_count)?.checked_mul(y_subs.max(1)),
    }
}

fn resolve_x_layout(
    sheet_meta: &ExternalDataSheetJson,
    x_set: Option<&ExternalDataSetJson>,
    sheet_uid: &str,
    warnings: &mut Vec<ImportWarning>,
) -> (usize, XColumnMode) {
    if sheet_meta.table.x_data_set.is_none() {
        return (0, XColumnMode::default());
    }
    let fmt = sheet_meta.table.x_data_set_format.as_deref().or_else(|| x_set.and_then(|s| s.format.as_deref())).unwrap_or("y_single").trim();
    match fmt.to_ascii_lowercase().as_str() {
        "series" => {
            let rep = x_set.and_then(|s| s.replicates.first());
            let start = rep.and_then(|r| r.start_value).unwrap_or_else(|| {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    sheet_uid,
                    format!("series X on `{sheet_uid}` missing startValue; defaulted to 0.0"),
                ));
                0.0
            });
            let step = rep.and_then(|r| r.interval).unwrap_or_else(|| {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    sheet_uid,
                    format!("series X on `{sheet_uid}` missing interval; defaulted to 1.0"),
                ));
                1.0
            });
            (0, XColumnMode::GeneratedSequence { start, step, progression: SequenceProgression::Arithmetic })
        }
        "timespan" => {
            let unit_label =
                x_set.and_then(|s| s.date_time_units.clone()).or_else(|| sheet_meta.date_time_units.clone()).unwrap_or_else(|| String::from("Time"));
            (1, XColumnMode::DateOrElapsedTime { unit_label })
        }
        "y_errorbar" => (2, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSd }),
        "date_se" => (2, XColumnMode::DateOrElapsedTime { unit_label: String::from("Date") }),
        _ => (1, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }),
    }
}

fn build_external_archetype(inp: &ExternalBuildInput<'_>, warnings: &mut Vec<ImportWarning>) -> Result<TableArchetype, FormatError> {
    let row_count = inp.records.len().max(1);
    let has_rt = inp.sheet_meta.table.row_titles_data_set.is_some();
    let rt_cols = usize::from(has_rt);
    let x_csv_cols = inp.x_csv_cols;
    let x_mode = &inp.x_mode;
    let y_start = rt_cols + x_csv_cols;

    let mut row_titles = vec![String::new(); row_count];
    let mut any_row_title = false;
    if has_rt {
        for (r, rec) in inp.records.iter().enumerate() {
            if let Some(tok) = rec.first() {
                let clamped = clamp_title(tok.trim(), inp.sheet_uid, warnings);
                if !clamped.is_empty() {
                    any_row_title = true;
                }
                if let Some(slot) = row_titles.get_mut(r) {
                    *slot = clamped;
                }
            }
        }
    }
    let hide_rt = inp.sheet_meta.flags.iter().any(|f| f.eq_ignore_ascii_case("HIDE_ROW_TITLES"));
    let row_titles_expanded = any_row_title && !hide_rt;

    let ds_count = inp.y_sets.len().max(1);

    match inp.plan.kind {
        TableKind::Xy => {
            let mut xy = XyTable::new_with_x_mode(x_mode.clone(), inp.plan.subcolumn_mode, ds_count, row_count)?;
            if let Some(x_title) = inp.x_set.and_then(|s| s.title_markup.as_deref()) {
                xy.x_title = clamp_title(x_title, inp.sheet_uid, warnings);
            }
            xy.x_display_decimals = inp.x_set.and_then(|s| s.replicates.first()).and_then(|r| r.decimals_length);
            xy.row_titles = row_titles;
            xy.row_titles_expanded = row_titles_expanded;
            populate_xy_x_cells(&mut xy, inp, rt_cols, x_csv_cols, warnings);
            populate_dataset_columns(&mut xy.data_sets, inp, y_start, warnings);
            Ok(TableArchetype::Xy(xy))
        }
        TableKind::Column => {
            let design_hint =
                if inp.plan.subcolumn_mode.is_pre_summarized() { ColumnDesignHint::PrecomputedError } else { ColumnDesignHint::UnpairedReplicates };
            let mut col = ColumnTable::new_with_subcolumns(design_hint, inp.plan.subcolumn_mode, ds_count, row_count)?;
            col.row_titles = row_titles;
            col.row_titles_expanded = row_titles_expanded;
            populate_dataset_columns(&mut col.data_sets, inp, y_start, warnings);
            Ok(TableArchetype::Column(col))
        }
        TableKind::Grouped => {
            let mut grp = GroupedTable::new(inp.plan.subcolumn_mode, ds_count, row_count)?;
            if let Some(rt_title) = inp.row_titles_set.and_then(|s| s.title_markup.as_deref()) {
                grp.row_factor_title = clamp_title(rt_title, inp.sheet_uid, warnings);
            }
            grp.row_titles = row_titles;
            populate_dataset_columns(&mut grp.data_sets, inp, y_start, warnings);
            Ok(TableArchetype::Grouped(grp))
        }
        TableKind::Contingency => {
            let mut cont = ContingencyTable::new(ds_count, row_count)?;
            if let Some(rt_title) = inp.row_titles_set.and_then(|s| s.title_markup.as_deref()) {
                cont.row_factor_title = clamp_title(rt_title, inp.sheet_uid, warnings);
            }
            cont.row_titles = row_titles;
            populate_dataset_columns(&mut cont.data_sets, inp, y_start, warnings);
            Ok(TableArchetype::Contingency(cont))
        }
        TableKind::Survival => {
            let mut surv = SurvivalTable::new(ds_count, row_count)?;
            if let Some(x_title) = inp.x_set.and_then(|s| s.title_markup.as_deref()) {
                surv.time_title = clamp_title(x_title, inp.sheet_uid, warnings);
            }
            surv.row_titles = row_titles;
            surv.row_titles_expanded = row_titles_expanded;
            populate_survival_times(&mut surv, inp, rt_cols, x_csv_cols, x_mode);
            populate_dataset_columns(&mut surv.cohorts, inp, y_start, warnings);
            apply_survival_censored_flags(&mut surv, inp.y_sets);
            Ok(TableArchetype::Survival(surv))
        }
        TableKind::PartsOfWhole => {
            let mut pow = PartsOfWholeTable::new(ds_count, row_count)?;
            pow.slice_labels = row_titles;
            populate_dataset_columns(&mut pow.data_sets, inp, y_start, warnings);
            Ok(TableArchetype::PartsOfWhole(pow))
        }
        TableKind::MultipleVariables => {
            let mut mv = MultipleVariablesTable::new(ds_count, row_count)?;
            mv.row_titles = row_titles;
            mv.row_titles_expanded = row_titles_expanded;
            populate_mv_variables(&mut mv.variables, inp, y_start, warnings);
            Ok(TableArchetype::MultipleVariables(mv))
        }
        TableKind::Nested => {
            let units = inp.plan.subcolumn_mode.subcolumn_count().max(1);
            let mut nested = NestedTable::new(ds_count, units, row_count)?;
            nested.row_titles = row_titles;
            nested.row_titles_expanded = row_titles_expanded;
            populate_nested_groups(&mut nested.groups, inp, y_start, warnings);
            Ok(TableArchetype::Nested(nested))
        }
    }
}

fn populate_xy_x_cells(xy: &mut XyTable, inp: &ExternalBuildInput<'_>, rt_cols: usize, x_csv_cols: usize, warnings: &mut Vec<ImportWarning>) {
    if x_csv_cols == 0 {
        return;
    }
    let expected_x_subs = xy.x_mode.subcolumn_count();
    let mut dropped_date_se = false;
    for (r, rec) in inp.records.iter().enumerate() {
        for s in 0..expected_x_subs {
            let raw = rec.get(rt_cols + s).map(String::as_str).unwrap_or("");
            let mut cell = parse_cell_token(raw, inp.csv_opts);
            if inp.x_set.and_then(|xs| xs.replicates.get(s)).is_some_and(|rep| rep.excluded_rows.contains(&r)) {
                cell.excluded = true;
            }
            if let Some(slot) = xy.x_cells.get_mut(r).and_then(|row| row.get_mut(s)) {
                *slot = cell;
            }
        }
        if x_csv_cols > expected_x_subs && rec.get(rt_cols + expected_x_subs).is_some_and(|tok| !tok.trim().is_empty()) {
            dropped_date_se = true;
        }
    }
    if dropped_date_se {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            inp.sheet_uid,
            format!("secondary date_se error column on `{}` is not represented in DateOrElapsedTime mode", inp.sheet_uid),
        ));
    }
}

#[allow(clippy::cast_precision_loss)]
fn populate_survival_times(surv: &mut SurvivalTable, inp: &ExternalBuildInput<'_>, rt_cols: usize, x_csv_cols: usize, x_mode: &XColumnMode) {
    if let XColumnMode::GeneratedSequence { start, step, .. } = x_mode {
        for (r, slot) in surv.times.iter_mut().enumerate() {
            let val = start + (r as f64) * step;
            if val.is_finite() {
                *slot = Cell::number(val);
            }
        }
        return;
    }
    if x_csv_cols == 0 {
        return;
    }
    for (r, rec) in inp.records.iter().enumerate() {
        let raw = rec.get(rt_cols).map(String::as_str).unwrap_or("");
        let mut cell = parse_cell_token(raw, inp.csv_opts);
        if inp.x_set.and_then(|xs| xs.replicates.first()).is_some_and(|rep| rep.excluded_rows.contains(&r)) {
            cell.excluded = true;
        }
        if let Some(slot) = surv.times.get_mut(r) {
            *slot = cell;
        }
    }
}

fn apply_survival_censored_flags(surv: &mut SurvivalTable, y_sets: &[ExternalDataSetJson]) {
    for (col_idx, ds_meta) in y_sets.iter().enumerate() {
        let Some(rep) = ds_meta.replicates.first() else {
            continue;
        };
        if rep.censored_rows.is_empty() {
            continue;
        }
        let Some(cohort) = surv.cohorts.get_mut(col_idx) else {
            continue;
        };
        for &row in &rep.censored_rows {
            if let Some(cell) = cohort.cell_mut(row, 0)
                && cell.is_empty()
            {
                cell.value = CellValue::Number(surv.censored_code as f64);
            }
        }
    }
}

fn populate_dataset_columns(columns: &mut [DataSetColumn], inp: &ExternalBuildInput<'_>, y_start: usize, warnings: &mut Vec<ImportWarning>) {
    let target_subs = inp.plan.subcolumn_mode.subcolumn_count();
    let stride = inp.plan.csv_stride;
    let is_text_mode =
        inp.sheet_meta.table.data_format.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("text") || f.eq_ignore_ascii_case("text_replicates"));

    let mut csv_col_base = y_start;
    for (col_idx, ds_col) in columns.iter_mut().enumerate() {
        let ds_meta = inp.y_sets.get(col_idx);
        if let Some(title) = ds_meta.and_then(|m| m.title_markup.as_deref()) {
            ds_col.title = clamp_title(title, inp.sheet_uid, warnings);
        }
        ds_col.display_decimals = ds_meta.and_then(|m| m.replicates.first()).and_then(|r| r.decimals_length);
        if let Some(raw_sub_titles) = inp.subcol_titles.and_then(|st| st.titles_for_column(col_idx, target_subs)) {
            ds_col.subcolumn_titles = raw_sub_titles.into_iter().map(|t| clamp_title(&t, inp.sheet_uid, warnings)).collect();
        }

        let ds_reps = ds_meta.map_or(0, |m| m.replicates.len());
        let ds_width = if ds_reps > 0 { ds_reps } else { stride };
        let use_swap_3 = match inp.plan.offset_mapping {
            OffsetSubcolMapping::SwapHighLowOffsets => true,
            OffsetSubcolMapping::PlusMinusPerDataset => ds_reps >= 3,
            OffsetSubcolMapping::Direct => false,
        };
        let use_dup_2 = matches!(inp.plan.offset_mapping, OffsetSubcolMapping::PlusMinusPerDataset) && !use_swap_3;

        for (r, rec) in inp.records.iter().enumerate() {
            if use_swap_3 {
                // `.prism` `[0: Y, 1: +Err, 2: -Err]` -> CurveBook `[0: Mean, 1: -Err, 2: +Err]`
                let c_mean = parse_dataset_cell(rec, csv_col_base, r, 0, ds_meta, false, inp.csv_opts);
                let c_plus = parse_dataset_cell(rec, csv_col_base + 1, r, 1, ds_meta, false, inp.csv_opts);
                let c_minus = parse_dataset_cell(rec, csv_col_base + 2, r, 2, ds_meta, false, inp.csv_opts);
                if let Some(slot) = ds_col.cell_mut(r, 0) {
                    *slot = c_mean;
                }
                if let Some(slot) = ds_col.cell_mut(r, 1) {
                    *slot = c_minus;
                }
                if let Some(slot) = ds_col.cell_mut(r, 2) {
                    *slot = c_plus;
                }
            } else if use_dup_2 {
                let c_mean = parse_dataset_cell(rec, csv_col_base, r, 0, ds_meta, false, inp.csv_opts);
                let c_err = parse_dataset_cell(rec, csv_col_base + 1, r, 1, ds_meta, false, inp.csv_opts);
                if let Some(slot) = ds_col.cell_mut(r, 0) {
                    *slot = c_mean;
                }
                if let Some(slot) = ds_col.cell_mut(r, 1) {
                    *slot = c_err.clone();
                }
                if let Some(slot) = ds_col.cell_mut(r, 2) {
                    *slot = c_err;
                }
            } else {
                for s in 0..ds_width.min(target_subs) {
                    let cell = parse_dataset_cell(rec, csv_col_base + s, r, s, ds_meta, is_text_mode, inp.csv_opts);
                    if let Some(slot) = ds_col.cell_mut(r, s) {
                        *slot = cell;
                    }
                }
            }
        }
        csv_col_base = csv_col_base.saturating_add(ds_width);
    }
}

fn populate_mv_variables(variables: &mut [VariableColumn], inp: &ExternalBuildInput<'_>, y_start: usize, warnings: &mut Vec<ImportWarning>) {
    let mut csv_col_base = y_start;
    for (col_idx, var) in variables.iter_mut().enumerate() {
        let ds_meta = inp.y_sets.get(col_idx);
        if let Some(title) = ds_meta.and_then(|m| m.title_markup.as_deref()) {
            var.title = clamp_title(title, inp.sheet_uid, warnings);
        }
        var.display_decimals = ds_meta.and_then(|m| m.replicates.first()).and_then(|r| r.decimals_length);
        let is_string_col =
            ds_meta.and_then(|m| m.replicates.first()).and_then(|r| r.value_type.as_deref()).is_some_and(|vt| vt.eq_ignore_ascii_case("string"));
        for (r, rec) in inp.records.iter().enumerate() {
            let cell = parse_dataset_cell(rec, csv_col_base, r, 0, ds_meta, is_string_col, inp.csv_opts);
            if let Some(slot) = var.cells.get_mut(r) {
                *slot = cell;
            }
        }
        var.refresh_inferred_role();
        let ds_width = ds_meta.map_or(1, |m| m.replicates.len().max(1));
        csv_col_base = csv_col_base.saturating_add(ds_width);
    }
}

fn populate_nested_groups(groups: &mut [NestedGroupColumn], inp: &ExternalBuildInput<'_>, y_start: usize, warnings: &mut Vec<ImportWarning>) {
    let units = inp.plan.subcolumn_mode.subcolumn_count().max(1);
    let mut csv_col_base = y_start;
    for (col_idx, grp) in groups.iter_mut().enumerate() {
        let ds_meta = inp.y_sets.get(col_idx);
        if let Some(title) = ds_meta.and_then(|m| m.title_markup.as_deref()) {
            grp.title = clamp_title(title, inp.sheet_uid, warnings);
        }
        if let Some(raw_sub_titles) = inp.subcol_titles.and_then(|st| st.titles_for_column(col_idx, units)) {
            grp.subcolumn_titles = raw_sub_titles
                .into_iter()
                .enumerate()
                .map(|(u, t)| if t.trim().is_empty() { format!("{}:{}", grp.title, u + 1) } else { clamp_title(&t, inp.sheet_uid, warnings) })
                .collect();
        }
        let ds_width = ds_meta.map_or(units, |m| if m.replicates.is_empty() { units } else { m.replicates.len() });
        for (r, rec) in inp.records.iter().enumerate() {
            for s in 0..ds_width.min(units) {
                let cell = parse_dataset_cell(rec, csv_col_base + s, r, s, ds_meta, false, inp.csv_opts);
                if let Some(slot) = grp.cells.get_mut(r).and_then(|row| row.get_mut(s)) {
                    *slot = cell;
                }
            }
        }
        csv_col_base = csv_col_base.saturating_add(ds_width);
    }
}

fn parse_dataset_cell(
    rec: &[String],
    csv_col: usize,
    row_idx: usize,
    rep_idx: usize,
    ds_meta: Option<&ExternalDataSetJson>,
    force_text: bool,
    csv_opts: &CsvOptions,
) -> Cell {
    let raw = rec.get(csv_col).map(String::as_str).unwrap_or("");
    let mut cell = if force_text {
        let trimmed = raw.trim();
        if trimmed.is_empty() { Cell::empty() } else { Cell::text(trimmed) }
    } else {
        parse_cell_token(raw, csv_opts)
    };
    if ds_meta.and_then(|m| m.replicates.get(rep_idx)).is_some_and(|rep| rep.excluded_rows.contains(&row_idx)) {
        cell.excluded = true;
    }
    cell
}

#[allow(clippy::too_many_arguments)]
fn import_external_info_sheets(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    doc_index: &ExternalDocumentIndex,
    data_uid_to_sheet_id: &BTreeMap<String, SheetId>,
    uid_to_sheet_id: &mut BTreeMap<String, SheetId>,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    for (idx, info_uid) in doc_index.info_sheet_ids.iter().enumerate() {
        let path = match id_index.find_info_sheet_json(info_uid) {
            IdLookup::Found(p) => p,
            IdLookup::Missing => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    info_uid,
                    format!("info sheet `{info_uid}` has no matching `sheet.json`; skipping"),
                ));
                continue;
            }
            IdLookup::Ambiguous(candidates) => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    info_uid,
                    format!("ambiguous info `sheet.json` for `{info_uid}` ({} candidates); skipping", candidates.len()),
                ));
                continue;
            }
        };
        let Some(bytes) = zip_index.read_entry(&path)? else {
            continue;
        };
        let val = match parse_bounded_json_outcome(&bytes) {
            BoundedJsonOutcome::Value(v) => v,
            BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
            BoundedJsonOutcome::SyntaxOrType(err) => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    path,
                    format!("could not parse info sheet `{info_uid}`: {err}"),
                ));
                continue;
            }
        };
        let Some(info_meta) = ExternalInfoSheetJson::from_json(&val) else {
            continue;
        };

        let attr = doc_index.sheet_attrs.get(info_uid);
        let raw_title =
            attr.and_then(|a| a.title.clone()).or(info_meta.title).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| format!("Info {}", idx + 1));
        let sid = allocate_valid_unique_sheet_id(Some(info_uid), "info", idx + 1, used_ids, warnings);
        let title = clamp_title(&raw_title, sid.as_str(), warnings);

        let mut sheet = InfoSheet::new_preseeded(sid.clone(), title)?;
        sheet.constants.clear();
        sheet.color_tag = attr.and_then(|a| a.highlight_color).or(info_meta.highlight_color);

        let has_exp_date_const = info_meta.constants.iter().any(|(k, _)| k.eq_ignore_ascii_case("Experiment Date"));
        if !has_exp_date_const && let Some(raw_date) = info_meta.experiment_date {
            let cval = match parse_strict_iso8601_date(&raw_date) {
                Some(iso) => ConstantValue::Date(iso),
                None => ConstantValue::Text(raw_date),
            };
            sheet.constants.push(InfoConstant::new("Experiment Date", cval, None)?);
        }

        for (raw_name, raw_val) in info_meta.constants {
            let cname = clamp_title(&raw_name, sid.as_str(), warnings);
            let cval = parse_info_string_value(&raw_val);
            sheet.constants.push(InfoConstant::new(cname, cval, None)?);
        }

        if let Some(notes) = info_meta.notes_text {
            sheet.notes_markdown = notes;
        }

        if let Some(ref linked_uid) = info_meta.linked_data_sheet_id
            && let Some(mapped_ds_id) = data_uid_to_sheet_id.get(linked_uid)
        {
            sheet.linked_data_sheets.push(mapped_ds_id.clone());
            if let Some(ds) = doc.data_sheets.iter_mut().find(|d| d.id == *mapped_ds_id)
                && ds.info_sheet_id.is_none()
            {
                ds.info_sheet_id = Some(sheet.id.clone());
            }
        }

        uid_to_sheet_id.insert(info_uid.clone(), sheet.id.clone());
        doc.add_info_sheet(sheet)?;
    }
    Ok(())
}

fn parse_info_string_value(raw: &str) -> ConstantValue {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ConstantValue::Empty;
    }
    if let Some(iso) = parse_strict_iso8601_date(trimmed) {
        return ConstantValue::Date(iso);
    }
    if let Ok(num) = trimmed.parse::<f64>()
        && num.is_finite()
    {
        return ConstantValue::Number(num);
    }
    ConstantValue::Text(raw.to_owned())
}

// ============================================================================
// Legacy Generic `manifest.json` / Plain CSV / Embedded `.pzfx` Path
// ============================================================================

/// Optional metadata sidecar describing a tabular CSV sheet inside a generic `.prism` ZIP archive.
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

/// Optional top-level generic `.prism` manifest JSON.
#[derive(Debug, Clone, Default)]
struct PrismManifest {
    title: Option<String>,
    sheets: Vec<(String, PrismTableMeta)>,
    info_sheets: Vec<PrismInfoMeta>,
    unsupported_sections: Vec<(ImportWarningKind, String, String)>,
}

/// Optional info metadata JSON inside a generic `.prism` ZIP archive.
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

fn import_generic_archive(
    zip_index: &mut ZipIndex<'_>,
    root_prefix: &str,
    doc_meta: Option<(String, serde_json::Value)>,
    handled_doc_path: Option<String>,
    mut warnings: Vec<ImportWarning>,
) -> Result<ImportReport, FormatError> {
    let manifest_entry = match find_archive_path_ci(zip_index, "manifest.json", root_prefix) {
        Some(p) => zip_index.read_entry(&p)?.map(|b| (p, b)),
        None => None,
    };
    let manifest_state = inspect_manifest_bounded(manifest_entry)?;
    let mut handled_paths: HashSet<String> = HashSet::new();
    if let Some(p) = handled_doc_path {
        handled_paths.insert(p);
    }

    let mut parsed_manifest = doc_meta.map(|(path, value)| parse_external_project_meta(&value, &path, &mut warnings));
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

    import_json_pass(zip_index, root_prefix, &mut handled_paths, &mut doc, &mut used_ids, &mut warnings, &mut meta_index)?;
    let mut used_cells = import_csv_pass(zip_index, root_prefix, &mut handled_paths, &mut doc, &mut used_ids, &mut warnings, &meta_index)?;
    import_pzfx_and_tail_passes(zip_index, &mut handled_paths, &mut doc, &mut used_ids, &mut warnings, &mut used_cells)?;

    if let Some(primary_info) = doc.info_sheets.first().map(|s| s.id.clone()) {
        for ds in &mut doc.data_sheets {
            if ds.info_sheet_id.is_none() {
                ds.info_sheet_id = Some(primary_info.clone());
            }
        }
    }

    validate_project_document(&doc)?;
    Ok(ImportReport::new(doc, warnings))
}

enum ManifestParseState {
    Absent,
    ValidValue(String, serde_json::Value),
    InvalidSyntax(String, FormatError),
}

fn inspect_manifest_bounded(entry: Option<(String, Vec<u8>)>) -> Result<ManifestParseState, FormatError> {
    let Some((path, manifest_bytes)) = entry else {
        return Ok(ManifestParseState::Absent);
    };
    match parse_bounded_json_outcome(&manifest_bytes) {
        BoundedJsonOutcome::Value(manifest_val) => {
            if let Some(format) = manifest_val.get("format").and_then(serde_json::Value::as_str)
                && (format == crate::archive::LEGACY_FORMAT_MAGIC || format == "lumencraft")
            {
                return Err(FormatError::UnsupportedFormat {
                    reason: format!("legacy LumenCraft archive (`{format}`) cannot be imported as a `.prism` project"),
                });
            }
            Ok(ManifestParseState::ValidValue(path, manifest_val))
        }
        BoundedJsonOutcome::BudgetExceeded(err) => Err(err),
        BoundedJsonOutcome::SyntaxOrType(err) => Ok(ManifestParseState::InvalidSyntax(path, err)),
    }
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
    for (_, item) in &manifest.sheets {
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
    zip_index: &mut ZipIndex<'_>,
    root_prefix: &str,
    handled_paths: &mut HashSet<String>,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    meta_index: &mut TableMetaIndex,
) -> Result<(), FormatError> {
    let json_paths: Vec<String> =
        zip_index.entry_names().filter(|p| !handled_paths.contains(*p) && has_ext_ci(p, ".json")).map(str::to_owned).collect();

    for path in json_paths {
        let rel = strip_root_prefix(&path, root_prefix).to_owned();
        handled_paths.insert(path.clone());

        if let Some((kind, desc)) = classify_unsupported_json_dir(&rel) {
            warnings.push(ImportWarning::new(
                kind,
                &path,
                format!("{desc} entry `{path}` is not imported automatically; data tables and info constants were preserved"),
            ));
            continue;
        }
        let Some(payload) = zip_index.read_entry(&path)? else {
            continue;
        };
        if is_info_json_path(&rel) {
            import_info_json(&path, &payload, doc, used_ids, warnings)?;
            continue;
        }

        let val = match parse_bounded_json_outcome(&payload) {
            BoundedJsonOutcome::Value(v) => v,
            BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
            BoundedJsonOutcome::SyntaxOrType(err) => {
                warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, &path, format!("could not parse JSON entry `{path}`: {err}")));
                continue;
            }
        };

        classify_and_register_json_value(&rel, &path, &val, doc, used_ids, warnings, meta_index)?;
    }
    Ok(())
}

fn import_csv_pass(
    zip_index: &mut ZipIndex<'_>,
    root_prefix: &str,
    handled_paths: &mut HashSet<String>,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    meta_index: &TableMetaIndex,
) -> Result<usize, FormatError> {
    let csv_paths: Vec<String> = zip_index
        .entry_names()
        .filter(|p| !handled_paths.contains(*p) && (has_ext_ci(p, ".csv") || has_ext_ci(p, ".tsv")))
        .map(str::to_owned)
        .collect();

    let mut used_cells: usize = 0;
    let mut data_idx = 1_usize;
    let mut matched_csv_keys: HashSet<String> = HashSet::new();

    for path in csv_paths {
        handled_paths.insert(path.clone());
        let Some(payload) = zip_index.read_entry(&path)? else {
            continue;
        };
        let text = match std::str::from_utf8(&payload) {
            Ok(s) => s,
            Err(err) => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::UnsupportedChunk,
                    &path,
                    format!("CSV/TSV entry is not valid UTF-8 and was skipped: {err}"),
                ));
                continue;
            }
        };
        let rel = strip_root_prefix(&path, root_prefix);
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
        let ds = import_csv_entry(&path, text, &meta, data_idx, used_ids, warnings, remaining_cells)?;
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
    zip_index: &mut ZipIndex<'_>,
    handled_paths: &mut HashSet<String>,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    used_cells: &mut usize,
) -> Result<(), FormatError> {
    let pzfx_paths: Vec<String> =
        zip_index.entry_names().filter(|p| !handled_paths.contains(*p) && has_ext_ci(p, ".pzfx")).map(str::to_owned).collect();
    for path in pzfx_paths {
        handled_paths.insert(path.clone());
        if let Some(payload) = zip_index.read_entry(&path)? {
            merge_embedded_pzfx(&payload, doc, used_ids, warnings, used_cells)?;
        }
    }

    let remaining_paths: Vec<String> = zip_index.entry_names().filter(|p| !handled_paths.contains(*p)).map(str::to_owned).collect();
    for path in remaining_paths {
        let is_sqlite = is_sqlite_path(&path)
            || zip_index.entry_info(&path).is_some_and(|info| {
                info.uncompressed_size <= 64 * 1024 && zip_index.read_entry(&path).ok().flatten().is_some_and(|b| b.starts_with(b"SQLite format 3\0"))
            });
        let msg = if is_sqlite {
            format!("embedded SQLite database `{path}` cannot be imported natively without tabular CSV/JSON/PZFX chunks")
        } else {
            format!("unsupported or binary `.prism` chunk `{path}` was skipped")
        };
        warnings.push(ImportWarning::new(ImportWarningKind::UnsupportedChunk, path, msg));
    }
    Ok(())
}

// ============================================================================
// Shared Helpers
// ============================================================================

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

fn hydrate_embedded_image_assets(zip_index: &mut ZipIndex<'_>, root_prefix: &str, doc: &mut ProjectDocument) -> Result<(), FormatError> {
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
        let found_path = find_archive_path_ci(zip_index, &expected_path, root_prefix);
        let entry_bytes = match found_path {
            Some(p) => zip_index.read_entry(&p)?,
            None => None,
        };
        if asset.bytes.is_empty() {
            let Some(bytes) = entry_bytes else {
                return Err(FormatError::MissingArchiveEntry { path: expected_path });
            };
            if bytes.len() > MAX_IMAGE_ASSET_BYTES {
                return Err(FormatError::FileTooLarge {
                    path: expected_path,
                    actual_bytes: Some(bytes.len() as u64),
                    limit_bytes: MAX_IMAGE_ASSET_BYTES,
                });
            }
            validate_image_asset_parts(&asset.asset_id, asset.media_type, asset.pixel_size, &bytes)?;
            asset.bytes = bytes;
        } else {
            validate_image_asset_parts(&asset.asset_id, asset.media_type, asset.pixel_size, &asset.bytes)?;
            if let Some(archived) = entry_bytes
                && archived != asset.bytes
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

fn detect_archive_root_prefix(zip_index: &ZipIndex<'_>) -> String {
    if find_archive_path_top_ci(zip_index, "manifest.json").is_some() || find_archive_path_top_ci(zip_index, "document.json").is_some() {
        return String::new();
    }
    let mut common_dir: Option<String> = None;
    for path in zip_index.entry_names() {
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

fn find_archive_path_top_ci(zip_index: &ZipIndex<'_>, target: &str) -> Option<String> {
    if zip_index.contains(target) {
        return Some(target.to_owned());
    }
    zip_index.entry_names().find(|p| p.eq_ignore_ascii_case(target)).map(str::to_owned)
}

fn find_archive_path_ci(zip_index: &ZipIndex<'_>, rel_target: &str, root_prefix: &str) -> Option<String> {
    if zip_index.contains(rel_target) {
        return Some(rel_target.to_owned());
    }
    if !root_prefix.is_empty() {
        let prefixed = format!("{root_prefix}{rel_target}");
        if zip_index.contains(&prefixed) {
            return Some(prefixed);
        }
        if let Some(hit) = zip_index.entry_names().find(|p| p.eq_ignore_ascii_case(&prefixed)) {
            return Some(hit.to_owned());
        }
    }
    zip_index.entry_names().find(|p| strip_root_prefix(p, root_prefix).eq_ignore_ascii_case(rel_target)).map(str::to_owned)
}

fn find_document_json_path(zip_index: &ZipIndex<'_>, root_prefix: &str) -> Option<String> {
    if let Some(hit) = find_archive_path_ci(zip_index, "document.json", root_prefix) {
        return Some(hit);
    }
    zip_index
        .entry_names()
        .find(|p| !p.starts_with("__MACOSX/") && p.rsplit('/').next().is_some_and(|name| name.eq_ignore_ascii_case("document.json")))
        .map(str::to_owned)
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

fn is_sqlite_path(path: &str) -> bool {
    has_ext_ci(path, ".sqlite") || has_ext_ci(path, ".sqlite3") || has_ext_ci(path, ".db")
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
                        let (cval, units) = parse_generic_constant_value(v);
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

fn classify_and_register_json_value(
    rel_path: &str,
    full_path: &str,
    val: &serde_json::Value,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    meta_index: &mut TableMetaIndex,
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
            meta_index.by_csv.insert(root_key, meta.clone());
            if !rel_key.is_empty() {
                meta_index.by_csv.insert(rel_key.clone(), meta.clone());
                meta_index.expected_csv_refs.push(ReferencedCsvPath { lookup_key: rel_key, raw_display: csv_p.clone() });
            }
        }
        let stem_lower = stem_without_ext(rel_path).to_ascii_lowercase();
        let file_stem = rel_path.rsplit('/').next().map(stem_without_ext).unwrap_or("").to_ascii_lowercase();
        if matches!(file_stem.as_str(), "meta" | "sheet" | "table") && !json_dir.is_empty() {
            meta_index.by_dir.insert(json_dir.to_ascii_lowercase(), meta);
        } else {
            meta_index.by_stem.insert(stem_lower, meta);
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

fn parse_generic_constant_value(v: &serde_json::Value) -> (ConstantValue, Option<String>) {
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
        "meansd" => SubcolumnMode::MeanSd,
        "meansem" => SubcolumnMode::MeanSem,
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

fn sanitize_invalid_document_sheet_ids(doc: &mut ProjectDocument, warnings: &mut Vec<ImportWarning>) -> BTreeMap<SheetId, SheetId> {
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
    remap
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
