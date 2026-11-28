//! Experimental `.prism` structural archive writer (§6.3).
//!
//! External `.prism` archives in the corpus unanimously require undocumented 32-hex `.dt`
//! dirty-tracking tokens (`data/tables/<uid>/data.dt`) and opaque binary `PCFFGRA4` blobs
//! (`graphs/<uid>/data.bin`, `layouts/<uid>/data.bin`, `misc/used_fonts.bin`). Per §6.3, `.prism`
//! export therefore remains **gated behind [`ArchiveExportOptions::experimental`]** (default
//! `false`), while `.pzfx` serves as the supported interchange export format for Beta.
//!
//! When `experimental: true` is opted in explicitly, this writer produces a deterministic,
//! clean-room ID-indexed ZIP container matching the open UTF-8 JSON/CSV schema of external
//! `.prism` archives (schema `$id` paths, uppercase deterministic UUIDs, `fenID`,
//! `DriverReplicate` descriptors, object-typed `uiSettings.currentSheets`, and neutral
//! deterministic metadata with zero vendor branding or user PII) and round-tripping cleanly
//! through CurveBook's own `.prism` importer.

use std::collections::BTreeMap;

use curvebook_doc::{
    Cell, CellValue, ConstantValue, DataSetColumn, DataSheet, InfoSheet, ProjectDocument, SequenceProgression, SheetId, SubcolumnMode,
    TableArchetype, XColumnMode,
};
use curvebook_palette::Rgba8;
use serde_json::{Value, json};

use crate::archive::validate_project_document;
use crate::csv::{CsvOptions, format_cell_token, write_delimited_records};
use crate::error::{FormatError, ImportWarning, ImportWarningKind};
use crate::zip::ZipWriter;

const SCHEMA_DOCUMENT: &str = "document.schema.json";
const SCHEMA_DATA_SHEET: &str = "sheets/data_sheet/data_sheet.schema.json";
const SCHEMA_DATA_TABLE: &str = "sheets/data_sheet/data_table/data_table.schema.json";
const SCHEMA_DATA_STORAGE: &str = "sheets/data_sheet/data_table/data_table_storage.schema.json";
const SCHEMA_DATA_SET: &str = "sheets/data_sheet/data_table/data_set.schema.json";
const SCHEMA_INFO_SHEET: &str = "sheets/info_sheet/info_sheet.schema.json";
const DETERMINISTIC_TIMESTAMP: &str = "1970-01-01T00:00:00Z";
const MIN_FORMAT_APP_VERSION_KEY: &str = "minPrismVersion";

/// Options controlling experimental `.prism` archive export (§6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchiveExportOptions {
    /// Must be set to `true` to write an experimental `.prism` container. Defaults to `false`
    /// (import-only policy for Beta per §6.3).
    pub experimental: bool,
}

/// Result of exporting a CurveBook project to an experimental `.prism` container.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveExportReport {
    pub bytes: Vec<u8>,
    pub warnings: Vec<ImportWarning>,
}

/// Export `doc` to a `.prism` archive honoring [`ArchiveExportOptions`].
///
/// Refuses with [`FormatError::UnsupportedFormat`] unless `options.experimental == true`.
pub fn export_prism_bytes_with_options(doc: &ProjectDocument, options: ArchiveExportOptions) -> Result<ArchiveExportReport, FormatError> {
    if !options.experimental {
        return Err(FormatError::UnsupportedFormat {
            reason: String::from(
                "`.prism` export is experimental and disabled by default (`.prism` is import-only for Beta because external archives require undocumented `.dt` digest tokens and binary `.bin` blobs); export `.pzfx` for interchange or enable `experimental` explicitly",
            ),
        });
    }
    export_prism_bytes_experimental(doc)
}

#[derive(Debug, Default)]
struct ArchiveIdAlloc {
    next_uuid: u64,
    next_fen: u64,
}

impl ArchiveIdAlloc {
    fn next_uid(&mut self) -> String {
        self.next_uuid += 1;
        format!("A1000000-0000-4000-8000-{:012X}", self.next_uuid)
    }

    fn next_fen_id(&mut self) -> u64 {
        self.next_fen += 1;
        self.next_fen
    }
}

struct ExportCtx<'a> {
    writer: &'a mut ZipWriter,
    alloc: ArchiveIdAlloc,
    sheet_attrs: serde_json::Map<String, Value>,
    warnings: &'a mut Vec<ImportWarning>,
}

/// Export `doc` to a deterministic experimental `.prism` ID-indexed ZIP archive (§6.3).
pub fn export_prism_bytes_experimental(doc: &ProjectDocument) -> Result<ArchiveExportReport, FormatError> {
    validate_project_document(doc)?;

    let mut warnings = vec![ImportWarning::new(
        ImportWarningKind::UnsupportedChunk,
        "archive",
        String::from(
            "experimental `.prism` export omits undocumented `.dt` digest tokens and opaque binary `.bin` blobs; not verified to open in external applications",
        ),
    )];

    let mut writer = ZipWriter::new();
    let mut ctx = ExportCtx { writer: &mut writer, alloc: ArchiveIdAlloc::default(), sheet_attrs: serde_json::Map::new(), warnings: &mut warnings };

    let mut data_uids = Vec::with_capacity(doc.data_sheets.len());
    let mut info_uids = Vec::with_capacity(doc.info_sheets.len());
    let mut sheet_id_to_uid: BTreeMap<SheetId, String> = BTreeMap::new();
    let mut info_uid_set: BTreeMap<SheetId, String> = BTreeMap::new();

    if !doc.data_sheets.is_empty() {
        ctx.writer.add_dir("data/")?;
        ctx.writer.add_dir("data/sets/")?;
        ctx.writer.add_dir("data/sheets/")?;
        ctx.writer.add_dir("data/tables/")?;
    }
    for ds in &doc.data_sheets {
        let uid = ctx.alloc.next_uid();
        sheet_id_to_uid.insert(ds.id.clone(), uid.clone());
        data_uids.push(uid.clone());
        write_data_sheet_entries(&mut ctx, ds, &uid)?;
    }

    if !doc.info_sheets.is_empty() {
        ctx.writer.add_dir("info/")?;
    }
    for info in &doc.info_sheets {
        let uid = ctx.alloc.next_uid();
        sheet_id_to_uid.insert(info.id.clone(), uid.clone());
        info_uid_set.insert(info.id.clone(), uid.clone());
        info_uids.push(uid.clone());
        write_info_sheet_entry(&mut ctx, info, &uid, &sheet_id_to_uid)?;
    }

    push_dropped_sheet_warnings(doc, ctx.warnings);

    let doc_json = build_document_json(doc, &data_uids, &info_uids, &sheet_id_to_uid, &info_uid_set, ctx.sheet_attrs);
    ctx.writer.add_file("document.json", serde_json::to_vec_pretty(&doc_json)?)?;

    let bytes = writer.finish()?;
    Ok(ArchiveExportReport { bytes, warnings })
}

fn push_dropped_sheet_warnings(doc: &ProjectDocument, warnings: &mut Vec<ImportWarning>) {
    if !doc.result_sheets.is_empty() {
        warnings.push(ImportWarning::new(
            ImportWarningKind::UnsupportedAnalysis,
            "analyses",
            format!("dropped {} analysis/result sheet(s) during experimental `.prism` export", doc.result_sheets.len()),
        ));
    }
    if !doc.graph_sheets.is_empty() {
        warnings.push(ImportWarning::new(
            ImportWarningKind::UnsupportedGraphOrLayout,
            "graphs",
            format!("dropped {} graph sheet(s) during experimental `.prism` export (graph visuals require binary blobs)", doc.graph_sheets.len()),
        ));
    }
    if !doc.layout_sheets.is_empty() {
        warnings.push(ImportWarning::new(
            ImportWarningKind::UnsupportedGraphOrLayout,
            "layouts",
            format!(
                "dropped {} layout sheet(s) during experimental `.prism` export (layout geometry requires binary blobs)",
                doc.layout_sheets.len()
            ),
        ));
    }
}

fn build_document_json(
    doc: &ProjectDocument,
    data_uids: &[String],
    info_uids: &[String],
    sheet_id_to_uid: &BTreeMap<SheetId, String>,
    info_uid_set: &BTreeMap<SheetId, String>,
    sheet_attrs: serde_json::Map<String, Value>,
) -> Value {
    let mut sheets_obj = serde_json::Map::new();
    sheets_obj.insert(String::from("data"), json!(data_uids));
    if !info_uids.is_empty() {
        sheets_obj.insert(String::from("info"), json!(info_uids));
    }

    let active_is_info = doc.active_sheet_id.as_ref().is_some_and(|id| info_uid_set.contains_key(id));
    let current_sheet_type = if active_is_info { "Info" } else { "Data" };
    let mut current_sheets = serde_json::Map::new();
    if let Some(active_id) = &doc.active_sheet_id
        && let Some(uid) = sheet_id_to_uid.get(active_id)
    {
        let folder = if active_is_info { "info" } else { "data" };
        current_sheets.insert(String::from(folder), Value::String(uid.clone()));
    }
    if !current_sheets.contains_key("data")
        && let Some(first_data) = data_uids.first()
    {
        current_sheets.insert(String::from("data"), Value::String(first_data.clone()));
    }
    if !current_sheets.contains_key("info")
        && let Some(first_info) = info_uids.first()
    {
        current_sheets.insert(String::from("info"), Value::String(first_info.clone()));
    }

    let creator = json!({
        "name": "CurveBook",
        "user": "",
        "version": env!("CARGO_PKG_VERSION"),
        "platform": "Win"
    });

    let mut root = serde_json::Map::new();
    root.insert(String::from("@class"), Value::String(String::from("Document")));
    root.insert(String::from("$id"), Value::String(String::from(SCHEMA_DOCUMENT)));
    root.insert(String::from("creationDate"), Value::String(String::from(DETERMINISTIC_TIMESTAMP)));
    root.insert(String::from("modificationDate"), Value::String(String::from(DETERMINISTIC_TIMESTAMP)));
    root.insert(String::from("createdBy"), creator.clone());
    root.insert(String::from("modifiedBy"), creator);
    root.insert(String::from("formatVersion"), Value::String(String::from("1-0-0")));
    root.insert(String::from("minFormatVersion"), Value::String(String::from("1-0-0")));
    root.insert(String::from(MIN_FORMAT_APP_VERSION_KEY), Value::String(String::from("10.0.0")));
    root.insert(String::from("sheets"), Value::Object(sheets_obj));
    root.insert(
        String::from("uiSettings"),
        json!({
            "currentSheetType": current_sheet_type,
            "viewMode": "sheet",
            "nextDataSheetNumber": doc.data_sheets.len() + 1,
            "printer": {
                "defaultPaperSize": { "width": 8.5, "height": 11.0 },
                "deviceName": "",
                "driverVersion": 0,
                "specVersion": 0,
                "driverExtra": 0
            },
            "navigator": {
                "width": 200,
                "collapsedFolders": {
                    "data": false,
                    "info": false,
                    "analyses": false,
                    "graphs": false,
                    "layouts": false,
                    "dataAndAnalyses": false,
                    "search": false,
                    "family": false
                }
            },
            "currentSheets": Value::Object(current_sheets)
        }),
    );
    root.insert(String::from("sheetAttributesMap"), Value::Object(sheet_attrs));
    Value::Object(root)
}

fn format_rgba_hex(c: Rgba8) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

fn build_sheet_attr_obj(title: &str, color_tag: Option<Rgba8>, frozen: bool) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(String::from("title"), Value::String(title.to_owned()));
    if let Some(c) = color_tag {
        map.insert(String::from("highlightColor"), Value::String(format_rgba_hex(c)));
    }
    if frozen {
        map.insert(String::from("isFrozen"), Value::Bool(true));
    }
    Value::Object(map)
}

#[derive(Debug, Clone, Copy)]
enum ExportSubcolTransform {
    Direct,
    /// CurveBook `MeanLowHigh` `[Mean, -Err, +Err]` -> `.prism` `y_high_low` `[Y, +Err, -Err]`.
    SwapMeanLowHigh,
    /// CurveBook `MeanCvN` `[Mean, %CV, N]` -> `.prism` `y_cv_n` `[Mean, SD, N]`.
    ConvertCvPercentToSd,
    /// CurveBook `[Center, Lower, Upper]` -> `.prism` `y_high_low` `[Center, Upper - Center, Center - Lower]`.
    ConvertLimitsToOffsets,
}

fn resolve_export_data_format(
    mode: SubcolumnMode,
    sheet_id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> (&'static str, Option<usize>, ExportSubcolTransform) {
    match mode {
        SubcolumnMode::Single => ("y_single", None, ExportSubcolTransform::Direct),
        SubcolumnMode::RawReplicates(n) => ("y_replicates", Some(n), ExportSubcolTransform::Direct),
        SubcolumnMode::MeanSdN => ("y_sd_n", None, ExportSubcolTransform::Direct),
        SubcolumnMode::MeanSemN => ("y_se_n", None, ExportSubcolTransform::Direct),
        SubcolumnMode::MeanCvN => ("y_cv_n", None, ExportSubcolTransform::ConvertCvPercentToSd),
        SubcolumnMode::MeanSd => ("y_sd", None, ExportSubcolTransform::Direct),
        SubcolumnMode::MeanSem => ("y_se", None, ExportSubcolTransform::Direct),
        SubcolumnMode::MeanLowHigh => ("y_high_low", None, ExportSubcolTransform::SwapMeanLowHigh),
        SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                sheet_id,
                format!("subcolumn mode {mode:?} on `{sheet_id}` converted to `y_high_low` offsets for `.prism` export"),
            ));
            ("y_high_low", None, ExportSubcolTransform::ConvertLimitsToOffsets)
        }
    }
}

fn write_data_sheet_entries(ctx: &mut ExportCtx<'_>, ds: &DataSheet, sheet_uid: &str) -> Result<(), FormatError> {
    if !ds.floating_notes.is_empty() {
        ctx.warnings.push(ImportWarning::new(
            ImportWarningKind::IgnoredSection,
            ds.id.as_str(),
            format!("dropped {} floating note(s) on sheet `{}`", ds.floating_notes.len(), ds.title),
        ));
    }
    ctx.sheet_attrs.insert(sheet_uid.to_owned(), build_sheet_attr_obj(&ds.title, ds.color_tag, ds.frozen));

    let sheet_fen = ctx.alloc.next_fen_id();
    let table_uid = ctx.alloc.next_uid();
    let row_count = ds.archetype.row_count();
    let row_titles = extract_row_titles(&ds.archetype);
    let has_row_titles = row_titles.iter().any(|t| !t.trim().is_empty());
    let rt_uid = if has_row_titles {
        let uid = ctx.alloc.next_uid();
        let fen = ctx.alloc.next_fen_id();
        let rt_json = json!({
            "@class": "DataSet",
            "$id": SCHEMA_DATA_SET,
            "uid": uid,
            "fenID": fen,
            "format": "text",
            "attributes": ["DS_ATTR_RT"],
            "title": "",
            "replicates": [build_replicate_json("string", None, row_count, &[], &[])]
        });
        ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&rt_json)?)?;
        Some(uid)
    } else {
        None
    };

    let (table_format, table_class, data_format, reps_count, x_info, y_export_cols, subcol_titles_uid) = prepare_archetype_export(ctx, ds)?;

    // Build headerless CSV rows: [rowTitles?][X? (0 cols if series)][Y datasets...]
    let mut csv_rows: Vec<Vec<String>> = vec![Vec::new(); row_count];
    let mut expected_cols = usize::from(has_row_titles);
    if has_row_titles {
        for (r, row) in csv_rows.iter_mut().enumerate() {
            row.push(row_titles.get(r).cloned().unwrap_or_default());
        }
    }
    if let Some(ref x) = x_info {
        expected_cols += x.csv_width;
        for (r, row) in csv_rows.iter_mut().enumerate() {
            if let Some(x_row) = x.csv_rows.get(r) {
                row.extend(x_row.iter().cloned());
            }
        }
    }
    for y_col in &y_export_cols {
        expected_cols += y_col.csv_width;
        for (r, row) in csv_rows.iter_mut().enumerate() {
            if let Some(y_row) = y_col.csv_rows.get(r) {
                row.extend(y_row.iter().cloned());
            }
        }
    }

    let ncols = csv_rows.first().map_or(expected_cols, Vec::len);
    let csv_opts = CsvOptions { header_rows: 0, parse_asterisk_exclusions: false, ..CsvOptions::csv() };
    let csv_text = write_delimited_records(&csv_rows, &csv_opts);

    ctx.writer.add_dir(&format!("data/tables/{table_uid}/"))?;
    ctx.writer.add_file(&format!("data/tables/{table_uid}/data.csv"), csv_text.into_bytes())?;

    let content_json = json!({
        "$id": SCHEMA_DATA_STORAGE,
        "numberOfColumns": ncols,
        "numberOfRows": row_count,
        "version": 1
    });
    ctx.writer.add_file(&format!("data/tables/{table_uid}/content.json"), serde_json::to_vec_pretty(&content_json)?)?;

    let y_uids: Vec<String> = y_export_cols.iter().map(|c| c.uid.clone()).collect();
    let mut table_obj = serde_json::Map::new();
    table_obj.insert(String::from("@class"), Value::String(table_class.to_owned()));
    table_obj.insert(String::from("$id"), Value::String(String::from(SCHEMA_DATA_TABLE)));
    table_obj.insert(String::from("uid"), Value::String(table_uid));
    table_obj.insert(String::from("format"), Value::String(table_format.to_owned()));
    table_obj.insert(String::from("dataFormat"), Value::String(data_format.to_owned()));
    table_obj.insert(String::from("dataSets"), json!(y_uids));
    if let Some(rc) = reps_count {
        table_obj.insert(String::from("replicatesCount"), json!(rc));
    }
    if let Some(uid) = rt_uid {
        table_obj.insert(String::from("rowTitlesDataSet"), Value::String(uid));
    }
    if let Some(ref x) = x_info {
        table_obj.insert(String::from("xDataSet"), Value::String(x.uid.clone()));
        table_obj.insert(String::from("xDataSetFormat"), Value::String(x.format.to_owned()));
    }
    if let Some(st_uid) = subcol_titles_uid {
        table_obj.insert(String::from("subcolumnTitlesDataSet"), Value::String(st_uid));
    }

    let mut sheet_obj = serde_json::Map::new();
    sheet_obj.insert(String::from("@class"), Value::String(String::from("DataSheet")));
    sheet_obj.insert(String::from("$id"), Value::String(String::from(SCHEMA_DATA_SHEET)));
    sheet_obj.insert(String::from("uid"), Value::String(sheet_uid.to_owned()));
    sheet_obj.insert(String::from("fenID"), json!(sheet_fen));
    sheet_obj.insert(String::from("title"), Value::String(ds.title.clone()));
    if let Some(c) = ds.color_tag {
        sheet_obj.insert(String::from("highlightColor"), Value::String(format_rgba_hex(c)));
    }
    sheet_obj.insert(String::from("table"), Value::Object(table_obj));
    sheet_obj.insert(String::from("font"), json!({ "family": "Arial", "size": 10.0 }));
    sheet_obj.insert(String::from("titleWidth"), json!(70));
    sheet_obj.insert(String::from("xSubColWidth"), json!(70));
    sheet_obj.insert(String::from("maxDataSets"), json!(y_export_cols.len()));
    sheet_obj.insert(String::from("maxRow"), json!(row_count));
    sheet_obj.insert(
        String::from("selectionRange"),
        json!({
            "startCell": { "row": 0, "column": 0, "subcolumn": 0 },
            "endCell": { "row": 0, "column": 0, "subcolumn": 0 }
        }),
    );
    sheet_obj.insert(String::from("anchorRow"), json!(0));
    sheet_obj.insert(String::from("anchorColumn"), json!(0));
    sheet_obj.insert(String::from("firstSubColumn"), json!(0));
    sheet_obj.insert(String::from("xRowWidth"), json!(70));
    sheet_obj.insert(String::from("rowTitlesWidth"), json!(70));
    sheet_obj.insert(String::from("subColumnTitleType"), Value::String(String::from("default")));
    sheet_obj.insert(String::from("dateTimeConversion"), json!({ "units": "Minutes", "zeroDate": "" }));

    ctx.writer.add_dir(&format!("data/sheets/{sheet_uid}/"))?;
    ctx.writer.add_file(&format!("data/sheets/{sheet_uid}/sheet.json"), serde_json::to_vec_pretty(&Value::Object(sheet_obj))?)?;
    Ok(())
}

struct ExportedXColumn {
    uid: String,
    format: &'static str,
    csv_width: usize,
    csv_rows: Vec<Vec<String>>,
}

struct ExportedYColumn {
    uid: String,
    csv_width: usize,
    csv_rows: Vec<Vec<String>>,
}

type PreparedArchetype = (&'static str, &'static str, &'static str, Option<usize>, Option<ExportedXColumn>, Vec<ExportedYColumn>, Option<String>);

fn prepare_archetype_export(ctx: &mut ExportCtx<'_>, ds: &DataSheet) -> Result<PreparedArchetype, FormatError> {
    let csv_opts = CsvOptions { header_rows: 0, parse_asterisk_exclusions: false, ..CsvOptions::csv() };
    let sheet_id = ds.id.as_str();
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            let (data_fmt, reps_cnt, xform) = resolve_export_data_format(xy.y_subcolumn_mode, sheet_id, ctx.warnings);
            let x_col = write_xy_x_dataset(ctx, xy, sheet_id, &csv_opts)?;
            let y_cols = write_dataset_columns_json(ctx, &xy.data_sets, data_fmt, xform, &csv_opts, sheet_id)?;
            let st_uid = write_subcol_titles_if_any(ctx, &xy.data_sets)?;
            Ok(("xy", "XYDataTable", data_fmt, reps_cnt, Some(x_col), y_cols, st_uid))
        }
        TableArchetype::Column(col) => {
            let (data_fmt, reps_cnt, xform) = resolve_export_data_format(col.subcolumn_mode, sheet_id, ctx.warnings);
            let y_cols = write_dataset_columns_json(ctx, &col.data_sets, data_fmt, xform, &csv_opts, sheet_id)?;
            let st_uid = write_subcol_titles_if_any(ctx, &col.data_sets)?;
            Ok(("column", "DataTable", data_fmt, reps_cnt, None, y_cols, st_uid))
        }
        TableArchetype::Grouped(grp) => {
            let (data_fmt, reps_cnt, xform) = resolve_export_data_format(grp.subcolumn_mode, sheet_id, ctx.warnings);
            let y_cols = write_dataset_columns_json(ctx, &grp.data_sets, data_fmt, xform, &csv_opts, sheet_id)?;
            let st_uid = write_subcol_titles_if_any(ctx, &grp.data_sets)?;
            Ok(("grouped", "DataTable", data_fmt, reps_cnt, None, y_cols, st_uid))
        }
        TableArchetype::Contingency(cont) => {
            let y_cols = write_dataset_columns_json(ctx, &cont.data_sets, "y_single", ExportSubcolTransform::Direct, &csv_opts, sheet_id)?;
            Ok(("contingency", "DataTable", "y_single", None, None, y_cols, None))
        }
        TableArchetype::Survival(surv) => {
            let x_uid = ctx.alloc.next_uid();
            let x_fen = ctx.alloc.next_fen_id();
            let x_excl = collect_excluded_rows(surv.times.iter());
            let x_json = json!({
                "@class": "DataSet",
                "$id": SCHEMA_DATA_SET,
                "uid": x_uid,
                "fenID": x_fen,
                "format": "y_single",
                "attributes": ["DS_ATTR_X"],
                "title": surv.time_title,
                "replicates": [build_replicate_json("real", None, surv.times.len(), &x_excl, &[])]
            });
            ctx.writer.add_file(&format!("data/sets/{x_uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            let x_rows = surv.times.iter().map(|c| vec![format_cell_value_unmasked(c, &csv_opts)]).collect();
            let x_col = ExportedXColumn { uid: x_uid, format: "y_single", csv_width: 1, csv_rows: x_rows };
            let y_cols = write_dataset_columns_json(ctx, &surv.cohorts, "y_single", ExportSubcolTransform::Direct, &csv_opts, sheet_id)?;
            Ok(("survival", "XYDataTable", "y_single", None, Some(x_col), y_cols, None))
        }
        TableArchetype::PartsOfWhole(pow) => {
            let y_cols = write_dataset_columns_json(ctx, &pow.data_sets, "y_single", ExportSubcolTransform::Direct, &csv_opts, sheet_id)?;
            Ok(("parts_of_whole", "DataTable", "y_single", None, None, y_cols, None))
        }
        TableArchetype::MultipleVariables(mv) => {
            let mut y_cols = Vec::with_capacity(mv.variables.len());
            for var in &mv.variables {
                let uid = ctx.alloc.next_uid();
                let fen = ctx.alloc.next_fen_id();
                let excl = collect_excluded_rows(var.cells.iter());
                let is_text = var.cells.iter().any(|c| matches!(c.value, CellValue::Text(_)));
                let ds_json = json!({
                    "@class": "DataSet",
                    "$id": SCHEMA_DATA_SET,
                    "uid": uid,
                    "fenID": fen,
                    "format": "y_single",
                    "attributes": ["DS_ATTR_Y"],
                    "title": var.title,
                    "replicates": [build_replicate_json(if is_text { "string" } else { "real" }, var.display_decimals, var.cells.len(), &excl, &[])]
                });
                ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&ds_json)?)?;
                let csv_rows = var.cells.iter().map(|c| vec![format_cell_value_unmasked(c, &csv_opts)]).collect();
                y_cols.push(ExportedYColumn { uid, csv_width: 1, csv_rows });
            }
            Ok(("multivariable", "DataTable", "y_single", None, None, y_cols, None))
        }
        TableArchetype::Nested(nested) => {
            let units = nested.groups.first().map_or(1, |g| g.biological_unit_count().max(1));
            let mut y_cols = Vec::with_capacity(nested.groups.len());
            let mut st_entries = Vec::new();
            for (g_idx, grp) in nested.groups.iter().enumerate() {
                let uid = ctx.alloc.next_uid();
                let fen = ctx.alloc.next_fen_id();
                let mut reps_json = Vec::with_capacity(units);
                for s in 0..units {
                    let excl: Vec<usize> =
                        grp.cells.iter().enumerate().filter_map(|(r, row)| row.get(s).is_some_and(|c| c.excluded).then_some(r)).collect();
                    reps_json.push(build_replicate_json("real", None, grp.cells.len(), &excl, &[]));
                }
                let ds_json = json!({
                    "@class": "DataSet",
                    "$id": SCHEMA_DATA_SET,
                    "uid": uid,
                    "fenID": fen,
                    "format": "y_replicates",
                    "attributes": ["DS_ATTR_Y"],
                    "title": grp.title,
                    "replicates": reps_json
                });
                ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&ds_json)?)?;
                let csv_rows = grp
                    .cells
                    .iter()
                    .map(|row| (0..units).map(|s| row.get(s).map_or_else(String::new, |c| format_cell_value_unmasked(c, &csv_opts))).collect())
                    .collect();
                y_cols.push(ExportedYColumn { uid, csv_width: units, csv_rows });

                let rep_titles: Vec<Value> =
                    grp.subcolumn_titles.iter().enumerate().map(|(s, name)| json!({ "replicate": s, "name": name })).collect();
                st_entries.push(json!({ "column": g_idx, "replicates": rep_titles }));
            }
            let st_uid = if st_entries.is_empty() {
                None
            } else {
                let uid = ctx.alloc.next_uid();
                let fen = ctx.alloc.next_fen_id();
                let st_json = json!({
                    "@class": "SubColumnTitlesDataSet",
                    "$id": SCHEMA_DATA_SET,
                    "uid": uid,
                    "fenID": fen,
                    "format": "text",
                    "replicate count": units,
                    "titles": st_entries
                });
                ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&st_json)?)?;
                Some(uid)
            };
            Ok(("nested", "DataTable", "y_replicates", Some(units), None, y_cols, st_uid))
        }
    }
}

fn write_xy_x_dataset(
    ctx: &mut ExportCtx<'_>,
    xy: &curvebook_doc::XyTable,
    sheet_id: &str,
    csv_opts: &CsvOptions,
) -> Result<ExportedXColumn, FormatError> {
    let uid = ctx.alloc.next_uid();
    let fen = ctx.alloc.next_fen_id();
    let row_count = xy.row_count();
    match &xy.x_mode {
        XColumnMode::GeneratedSequence { start, step, progression: SequenceProgression::Arithmetic } => {
            let mut rep = serde_json::Map::new();
            rep.insert(String::from("@class"), Value::String(String::from("DriverReplicate")));
            rep.insert(String::from("valueType"), Value::String(String::from("real")));
            rep.insert(String::from("commonAttributes"), json!(["NUMERIC"]));
            rep.insert(String::from("decimalsLength"), json!(xy.x_display_decimals.unwrap_or(15)));
            rep.insert(String::from("significantDigitsCount"), json!(7));
            rep.insert(String::from("startValue"), json!(start));
            rep.insert(String::from("interval"), json!(step));
            let x_json = json!({
                "@class": "DataSet",
                "$id": SCHEMA_DATA_SET,
                "uid": uid,
                "fenID": fen,
                "format": "series",
                "attributes": ["DS_ATTR_X"],
                "title": xy.x_title,
                "replicates": [Value::Object(rep)]
            });
            ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            // Series X occupies 0 columns in `data.csv`!
            Ok(ExportedXColumn { uid, format: "series", csv_width: 0, csv_rows: vec![Vec::new(); row_count] })
        }
        XColumnMode::GeneratedSequence { progression: SequenceProgression::Geometric, .. } => {
            ctx.warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                sheet_id,
                format!("geometric X sequence on `{sheet_id}` materialized as explicit numeric X values in `.prism` export"),
            ));
            let x_json = json!({
                "@class": "DataSet",
                "$id": SCHEMA_DATA_SET,
                "uid": uid,
                "fenID": fen,
                "format": "y_single",
                "attributes": ["DS_ATTR_X"],
                "title": xy.x_title,
                "replicates": [build_replicate_json("real", xy.x_display_decimals, row_count, &[], &[])]
            });
            ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            let csv_rows = (0..row_count).map(|r| vec![format_cell_value_unmasked(&xy.materialized_x_cell_at(r), csv_opts)]).collect();
            Ok(ExportedXColumn { uid, format: "y_single", csv_width: 1, csv_rows })
        }
        XColumnMode::DateOrElapsedTime { unit_label } => {
            let excl = collect_excluded_rows(xy.x_cells.iter().filter_map(|r| r.first()));
            let x_json = json!({
                "@class": "DataSet",
                "$id": SCHEMA_DATA_SET,
                "uid": uid,
                "fenID": fen,
                "format": "timeSpan",
                "attributes": ["DS_ATTR_X"],
                "title": xy.x_title,
                "dateTimeConversion": { "units": unit_label },
                "replicates": [build_replicate_json("timespan", xy.x_display_decimals, row_count, &excl, &[])]
            });
            ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            let csv_rows = xy.x_cells.iter().map(|r| vec![r.first().map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts))]).collect();
            Ok(ExportedXColumn { uid, format: "timeSpan", csv_width: 1, csv_rows })
        }
        XColumnMode::Numbers { subcolumn_mode } => {
            let x_subs = subcolumn_mode.subcolumn_count();
            let (fmt, emit_subs) = if x_subs >= 2 { ("y_errorBar", 2) } else { ("y_single", 1) };
            if x_subs > 2 {
                ctx.warnings.push(ImportWarning::new(
                    ImportWarningKind::CoercedValue,
                    sheet_id,
                    format!("X subcolumn mode {subcolumn_mode:?} truncated to 2 subcolumns (`y_errorBar`) in `.prism` export"),
                ));
            }
            let mut reps = Vec::with_capacity(emit_subs);
            for s in 0..emit_subs {
                let excl: Vec<usize> =
                    xy.x_cells.iter().enumerate().filter_map(|(r, row)| row.get(s).is_some_and(|c| c.excluded).then_some(r)).collect();
                reps.push(build_replicate_json("real", xy.x_display_decimals, row_count, &excl, &[]));
            }
            let x_json = json!({
                "@class": "DataSet",
                "$id": SCHEMA_DATA_SET,
                "uid": uid,
                "fenID": fen,
                "format": fmt,
                "attributes": ["DS_ATTR_X"],
                "title": xy.x_title,
                "replicates": reps
            });
            ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            let csv_rows = xy
                .x_cells
                .iter()
                .map(|row| (0..emit_subs).map(|s| row.get(s).map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts))).collect())
                .collect();
            Ok(ExportedXColumn { uid, format: fmt, csv_width: emit_subs, csv_rows })
        }
    }
}

fn write_dataset_columns_json(
    ctx: &mut ExportCtx<'_>,
    columns: &[DataSetColumn],
    data_format: &str,
    transform: ExportSubcolTransform,
    csv_opts: &CsvOptions,
    sheet_id: &str,
) -> Result<Vec<ExportedYColumn>, FormatError> {
    let mut out = Vec::with_capacity(columns.len());
    for col in columns {
        let uid = ctx.alloc.next_uid();
        let fen = ctx.alloc.next_fen_id();
        let subs = col.subcolumn_count().max(1);
        let mut reps_json = Vec::with_capacity(subs);
        for s in 0..subs {
            let source_sub = match transform {
                ExportSubcolTransform::SwapMeanLowHigh | ExportSubcolTransform::ConvertLimitsToOffsets if s == 1 => 2,
                ExportSubcolTransform::SwapMeanLowHigh | ExportSubcolTransform::ConvertLimitsToOffsets if s == 2 => 1,
                _ => s,
            };
            let excl: Vec<usize> =
                col.cells.iter().enumerate().filter_map(|(r, row)| row.get(source_sub).is_some_and(|c| c.excluded).then_some(r)).collect();
            reps_json.push(build_replicate_json("real", col.display_decimals, col.cells.len(), &excl, &[]));
        }
        let ds_json = json!({
            "@class": "DataSet",
            "$id": SCHEMA_DATA_SET,
            "uid": uid,
            "fenID": fen,
            "format": data_format,
            "attributes": ["DS_ATTR_Y"],
            "title": col.title,
            "replicates": reps_json
        });
        ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&ds_json)?)?;

        let mut csv_rows = Vec::with_capacity(col.cells.len());
        let mut warned_zero_cv = false;
        for row in &col.cells {
            let formatted = format_y_row(row, subs, transform, csv_opts, &mut warned_zero_cv);
            csv_rows.push(formatted);
        }
        if warned_zero_cv {
            ctx.warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                sheet_id,
                format!("MeanCvN row with Mean = 0 on `{sheet_id}` converted to SD = 0 in `.prism` `y_cv_n` export"),
            ));
        }
        out.push(ExportedYColumn { uid, csv_width: subs, csv_rows });
    }
    Ok(out)
}

fn format_y_row(row: &[Cell], subs: usize, transform: ExportSubcolTransform, csv_opts: &CsvOptions, warned_zero_cv: &mut bool) -> Vec<String> {
    match transform {
        ExportSubcolTransform::Direct => {
            (0..subs).map(|s| row.get(s).map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts))).collect()
        }
        ExportSubcolTransform::SwapMeanLowHigh => {
            // CurveBook `[0: Mean, 1: -Err, 2: +Err]` -> `.prism` `[0: Y, 1: +Err, 2: -Err]`
            let c0 = row.first().map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts));
            let c_plus = row.get(2).map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts));
            let c_minus = row.get(1).map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts));
            vec![c0, c_plus, c_minus]
        }
        ExportSubcolTransform::ConvertCvPercentToSd => {
            let mean_opt = row.first().and_then(|c| c.value.finite_f64());
            let cv_opt = row.get(1).and_then(|c| c.value.finite_f64());
            let c0 = row.first().map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts));
            let c1 = match (mean_opt, cv_opt) {
                (Some(m), Some(cv)) => {
                    if m == 0.0 && cv > 0.0 {
                        *warned_zero_cv = true;
                    }
                    let sd = m.abs() * cv / 100.0;
                    format_cell_token(&Cell::number(sd), csv_opts)
                }
                _ => row.get(1).map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts)),
            };
            let c2 = row.get(2).map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts));
            vec![c0, c1, c2]
        }
        ExportSubcolTransform::ConvertLimitsToOffsets => {
            let center = row.first().and_then(|c| c.value.finite_f64());
            let lower = row.get(1).and_then(|c| c.value.finite_f64());
            let upper = row.get(2).and_then(|c| c.value.finite_f64());
            let c0 = row.first().map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts));
            let c_plus = match (center, upper) {
                (Some(m), Some(u)) => format_cell_token(&Cell::number((u - m).max(0.0)), csv_opts),
                _ => String::new(),
            };
            let c_minus = match (center, lower) {
                (Some(m), Some(l)) => format_cell_token(&Cell::number((m - l).max(0.0)), csv_opts),
                _ => String::new(),
            };
            vec![c0, c_plus, c_minus]
        }
    }
}

fn write_subcol_titles_if_any(ctx: &mut ExportCtx<'_>, columns: &[DataSetColumn]) -> Result<Option<String>, FormatError> {
    let mut entries = Vec::new();
    let mut max_subs = 0_usize;
    for (c_idx, col) in columns.iter().enumerate() {
        if col.subcolumn_titles.iter().any(|t| !t.trim().is_empty()) {
            max_subs = max_subs.max(col.subcolumn_titles.len());
            let reps: Vec<Value> = col.subcolumn_titles.iter().enumerate().map(|(s, name)| json!({ "replicate": s, "name": name })).collect();
            entries.push(json!({ "column": c_idx, "replicates": reps }));
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }
    let uid = ctx.alloc.next_uid();
    let fen = ctx.alloc.next_fen_id();
    let st_json = json!({
        "@class": "SubColumnTitlesDataSet",
        "$id": SCHEMA_DATA_SET,
        "uid": uid,
        "fenID": fen,
        "format": "text",
        "replicate count": max_subs,
        "titles": entries
    });
    ctx.writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&st_json)?)?;
    Ok(Some(uid))
}

fn write_info_sheet_entry(
    ctx: &mut ExportCtx<'_>,
    info: &InfoSheet,
    info_uid: &str,
    sheet_id_to_uid: &BTreeMap<SheetId, String>,
) -> Result<(), FormatError> {
    if !info.floating_notes.is_empty() {
        ctx.warnings.push(ImportWarning::new(
            ImportWarningKind::IgnoredSection,
            info.id.as_str(),
            format!("dropped {} floating note(s) on info sheet `{}`", info.floating_notes.len(), info.title),
        ));
    }
    ctx.sheet_attrs.insert(info_uid.to_owned(), build_sheet_attr_obj(&info.title, info.color_tag, false));
    let fen = ctx.alloc.next_fen_id();

    let mut experiment_date = String::new();
    let mut constants_arr = Vec::new();
    for c in &info.constants {
        if c.name.eq_ignore_ascii_case("Experiment Date")
            && let ConstantValue::Date(ref d) = c.value
        {
            experiment_date = d.clone();
            continue;
        }
        constants_arr.push(json!({ "name": c.name, "value": c.value.display_text() }));
    }

    let mut obj = serde_json::Map::new();
    obj.insert(String::from("@class"), Value::String(String::from("InfoSheet")));
    obj.insert(String::from("$id"), Value::String(String::from(SCHEMA_INFO_SHEET)));
    obj.insert(String::from("uid"), Value::String(info_uid.to_owned()));
    obj.insert(String::from("fenID"), json!(fen));
    obj.insert(String::from("title"), Value::String(info.title.clone()));
    obj.insert(String::from("experimentDate"), Value::String(experiment_date));
    obj.insert(String::from("notesText"), Value::String(info.notes_markdown.clone()));
    obj.insert(String::from("constants"), Value::Array(constants_arr));
    obj.insert(String::from("uiState"), json!({ "columnNamesWidth": 150, "columnValuesWidth": 250, "constantPanelProportion": 0.4 }));
    obj.insert(String::from("selectedCellsRange"), json!({ "startCell": { "row": 0, "column": 0 }, "endCell": { "row": 0, "column": 0 } }));
    if let Some(first_linked) = info.linked_data_sheets.first().and_then(|id| sheet_id_to_uid.get(id)) {
        obj.insert(String::from("linkedDataSheetID"), Value::String(first_linked.clone()));
    }
    if let Some(c) = info.color_tag {
        obj.insert(String::from("highlightColor"), Value::String(format_rgba_hex(c)));
    }
    ctx.writer.add_dir(&format!("info/{info_uid}/"))?;
    ctx.writer.add_file(&format!("info/{info_uid}/sheet.json"), serde_json::to_vec_pretty(&Value::Object(obj))?)?;
    Ok(())
}

fn format_cell_value_unmasked(cell: &Cell, opts: &CsvOptions) -> String {
    let unmasked = Cell { value: cell.value.clone(), excluded: false };
    format_cell_token(&unmasked, opts)
}

fn collect_excluded_rows<'a>(cells: impl Iterator<Item = &'a Cell>) -> Vec<usize> {
    cells.enumerate().filter_map(|(r, c)| c.excluded.then_some(r)).collect()
}

fn format_row_ranges(rows: &[usize]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    let mut iter = rows.iter().copied();
    let Some(mut start) = iter.next() else {
        return String::new();
    };
    let mut prev = start;
    for r in iter {
        if r == prev + 1 {
            prev = r;
        } else {
            parts.push(if start == prev { format!("{start}") } else { format!("{start}~{prev}") });
            start = r;
            prev = r;
        }
    }
    parts.push(if start == prev { format!("{start}") } else { format!("{start}~{prev}") });
    parts.join(",")
}

fn build_replicate_json(value_type: &str, decimals: Option<u8>, row_count: usize, excluded_rows: &[usize], censored_rows: &[usize]) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(String::from("@class"), Value::String(String::from("DriverReplicate")));
    map.insert(String::from("valueType"), Value::String(value_type.to_owned()));
    map.insert(String::from("commonAttributes"), json!(["NUMERIC"]));
    map.insert(String::from("decimalsLength"), json!(decimals.unwrap_or(15)));
    map.insert(String::from("significantDigitsCount"), json!(7));
    if row_count > 0 {
        map.insert(String::from("firstRow"), json!(0));
        map.insert(String::from("lastRow"), json!(row_count - 1));
    }
    let mut cell_attrs = Vec::new();
    if !excluded_rows.is_empty() {
        cell_attrs.push(json!({ "rows": format_row_ranges(excluded_rows), "attributes": ["EXCLUDED"] }));
    }
    if !censored_rows.is_empty() {
        cell_attrs.push(json!({ "rows": format_row_ranges(censored_rows), "attributes": ["CENSORED"] }));
    }
    if !cell_attrs.is_empty() {
        map.insert(String::from("cellAttributes"), Value::Array(cell_attrs));
    }
    Value::Object(map)
}

fn extract_row_titles(archetype: &TableArchetype) -> &[String] {
    match archetype {
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    }
}
