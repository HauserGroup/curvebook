//! Experimental `.prism` structural archive writer (§6.3).
//!
//! External `.prism` archives in the corpus unanimously require undocumented 32-hex `.dt`
//! dirty-tracking tokens (`data/tables/<uid>/data.dt`) and opaque binary `PCFFGRA4` blobs
//! (`graphs/<uid>/data.bin`, `layouts/<uid>/data.bin`, `misc/used_fonts.bin`). Per §6.3, `.prism`
//! export therefore remains **gated behind [`ArchiveExportOptions::experimental`]** (default
//! `false`), while `.pzfx` serves as the supported interchange export format for Beta.
//!
//! When `experimental: true` is opted in explicitly, this writer produces a deterministic,
//! clean-room ID-indexed ZIP container matching the open UTF-8 JSON/CSV structure of external
//! `.prism` archives (omitting vendor/user/timestamp fields on the §6.3 exclusion list) and
//! round-tripping cleanly through CurveBook's own `.prism` importer.

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
    let mut data_uids = Vec::with_capacity(doc.data_sheets.len());
    let mut info_uids = Vec::with_capacity(doc.info_sheets.len());
    let mut sheet_id_to_uid: BTreeMap<SheetId, String> = BTreeMap::new();
    let mut sheet_attrs = serde_json::Map::new();

    for (idx, ds) in doc.data_sheets.iter().enumerate() {
        let uid = format!("data-{}", idx + 1);
        sheet_id_to_uid.insert(ds.id.clone(), uid.clone());
        data_uids.push(uid.clone());
        write_data_sheet_entries(&mut writer, ds, &uid, idx + 1, &mut sheet_attrs, &mut warnings)?;
    }

    for (idx, info) in doc.info_sheets.iter().enumerate() {
        let uid = format!("info-{}", idx + 1);
        sheet_id_to_uid.insert(info.id.clone(), uid.clone());
        info_uids.push(uid.clone());
        write_info_sheet_entry(&mut writer, info, &uid, &sheet_id_to_uid, &mut sheet_attrs, &mut warnings)?;
    }

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

    let current_sheets: Vec<Value> =
        doc.active_sheet_id.as_ref().and_then(|id| sheet_id_to_uid.get(id)).map(|uid| vec![Value::String(uid.clone())]).unwrap_or_default();

    // Intentionally omit vendor/user/timestamp keys per §6.3 exclusion list.
    let doc_json = json!({
        "$id": "document-root",
        "@class": "Document",
        "formatVersion": "1-0-0",
        "title": doc.title,
        "sheets": {
            "data": data_uids,
            "info": info_uids,
            "analyses": [],
            "graphs": [],
            "layouts": []
        },
        "sheetAttributesMap": Value::Object(sheet_attrs),
        "uiSettings": {
            "currentSheets": current_sheets
        }
    });
    writer.add_file("document.json", serde_json::to_vec_pretty(&doc_json)?)?;

    let bytes = writer.finish()?;
    Ok(ArchiveExportReport { bytes, warnings })
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

fn write_data_sheet_entries(
    writer: &mut ZipWriter,
    ds: &DataSheet,
    sheet_uid: &str,
    ordinal: usize,
    sheet_attrs: &mut serde_json::Map<String, Value>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    if !ds.floating_notes.is_empty() {
        warnings.push(ImportWarning::new(
            ImportWarningKind::IgnoredSection,
            ds.id.as_str(),
            format!("dropped {} floating note(s) on sheet `{}`", ds.floating_notes.len(), ds.title),
        ));
    }
    sheet_attrs.insert(sheet_uid.to_owned(), build_sheet_attr_obj(&ds.title, ds.color_tag, ds.frozen));

    let table_uid = format!("tbl-{ordinal}");
    let row_count = ds.archetype.row_count();
    let row_titles = extract_row_titles(&ds.archetype);
    let has_row_titles = row_titles.iter().any(|t| !t.trim().is_empty());
    let rt_uid = if has_row_titles {
        let uid = format!("set-{ordinal}-rt");
        let rt_json = json!({
            "$id": uid,
            "@class": "DataSet",
            "uid": uid,
            "format": "text",
            "title": "",
            "replicates": [{ "valueType": "string" }]
        });
        writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&rt_json)?)?;
        Some(uid)
    } else {
        None
    };

    let (table_format, table_class, data_format, reps_count, x_info, y_export_cols, subcol_titles_uid) =
        prepare_archetype_export(writer, ds, ordinal, warnings)?;

    // Build headerless CSV rows: [rowTitles?][X? (0 cols if series)][Y datasets...]
    let mut csv_rows: Vec<Vec<String>> = vec![Vec::new(); row_count];
    if has_row_titles {
        for (r, row) in csv_rows.iter_mut().enumerate() {
            row.push(row_titles.get(r).cloned().unwrap_or_default());
        }
    }
    if let Some(ref x) = x_info {
        for (r, row) in csv_rows.iter_mut().enumerate() {
            if let Some(x_row) = x.csv_rows.get(r) {
                row.extend(x_row.iter().cloned());
            }
        }
    }
    for y_col in &y_export_cols {
        for (r, row) in csv_rows.iter_mut().enumerate() {
            if let Some(y_row) = y_col.csv_rows.get(r) {
                row.extend(y_row.iter().cloned());
            }
        }
    }

    let ncols = csv_rows.first().map_or(0, Vec::len);
    let csv_opts = CsvOptions { header_rows: 0, parse_asterisk_exclusions: false, ..CsvOptions::csv() };
    let csv_text = write_delimited_records(&csv_rows, &csv_opts);
    writer.add_file(&format!("data/tables/{table_uid}/data.csv"), csv_text.into_bytes())?;

    let content_json = json!({
        "$id": format!("content-{ordinal}"),
        "numberOfColumns": ncols,
        "numberOfRows": row_count,
        "version": 1
    });
    writer.add_file(&format!("data/tables/{table_uid}/content.json"), serde_json::to_vec_pretty(&content_json)?)?;

    let y_uids: Vec<String> = y_export_cols.iter().map(|c| c.uid.clone()).collect();
    let mut table_obj = serde_json::Map::new();
    table_obj.insert(String::from("$id"), Value::String(format!("table-{ordinal}")));
    table_obj.insert(String::from("@class"), Value::String(table_class.to_owned()));
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
    sheet_obj.insert(String::from("$id"), Value::String(sheet_uid.to_owned()));
    sheet_obj.insert(String::from("@class"), Value::String(String::from("DataSheet")));
    sheet_obj.insert(String::from("uid"), Value::String(sheet_uid.to_owned()));
    sheet_obj.insert(String::from("title"), Value::String(ds.title.clone()));
    sheet_obj.insert(String::from("flags"), json!([]));
    if let Some(c) = ds.color_tag {
        sheet_obj.insert(String::from("highlightColor"), Value::String(format_rgba_hex(c)));
    }
    sheet_obj.insert(String::from("table"), Value::Object(table_obj));
    writer.add_file(&format!("data/sheets/{sheet_uid}/sheet.json"), serde_json::to_vec_pretty(&Value::Object(sheet_obj))?)?;
    Ok(())
}

struct ExportedXColumn {
    uid: String,
    format: &'static str,
    csv_rows: Vec<Vec<String>>,
}

struct ExportedYColumn {
    uid: String,
    csv_rows: Vec<Vec<String>>,
}

type PreparedArchetype = (&'static str, &'static str, &'static str, Option<usize>, Option<ExportedXColumn>, Vec<ExportedYColumn>, Option<String>);

fn prepare_archetype_export(
    writer: &mut ZipWriter,
    ds: &DataSheet,
    ordinal: usize,
    warnings: &mut Vec<ImportWarning>,
) -> Result<PreparedArchetype, FormatError> {
    let csv_opts = CsvOptions { header_rows: 0, parse_asterisk_exclusions: false, ..CsvOptions::csv() };
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            let (data_fmt, reps_cnt, xform) = resolve_export_data_format(xy.y_subcolumn_mode, ds.id.as_str(), warnings);
            let x_col = write_xy_x_dataset(writer, xy, ordinal, ds.id.as_str(), &csv_opts, warnings)?;
            let y_cols = write_dataset_columns_json(writer, &xy.data_sets, ordinal, data_fmt, xform, &csv_opts, ds.id.as_str(), warnings)?;
            let st_uid = write_subcol_titles_if_any(writer, &xy.data_sets, ordinal)?;
            Ok(("xy", "XYDataTable", data_fmt, reps_cnt, Some(x_col), y_cols, st_uid))
        }
        TableArchetype::Column(col) => {
            let (data_fmt, reps_cnt, xform) = resolve_export_data_format(col.subcolumn_mode, ds.id.as_str(), warnings);
            let y_cols = write_dataset_columns_json(writer, &col.data_sets, ordinal, data_fmt, xform, &csv_opts, ds.id.as_str(), warnings)?;
            let st_uid = write_subcol_titles_if_any(writer, &col.data_sets, ordinal)?;
            Ok(("column", "DataTable", data_fmt, reps_cnt, None, y_cols, st_uid))
        }
        TableArchetype::Grouped(grp) => {
            let (data_fmt, reps_cnt, xform) = resolve_export_data_format(grp.subcolumn_mode, ds.id.as_str(), warnings);
            let y_cols = write_dataset_columns_json(writer, &grp.data_sets, ordinal, data_fmt, xform, &csv_opts, ds.id.as_str(), warnings)?;
            let st_uid = write_subcol_titles_if_any(writer, &grp.data_sets, ordinal)?;
            Ok(("grouped", "DataTable", data_fmt, reps_cnt, None, y_cols, st_uid))
        }
        TableArchetype::Contingency(cont) => {
            let y_cols = write_dataset_columns_json(
                writer,
                &cont.data_sets,
                ordinal,
                "y_single",
                ExportSubcolTransform::Direct,
                &csv_opts,
                ds.id.as_str(),
                warnings,
            )?;
            Ok(("contingency", "DataTable", "y_single", None, None, y_cols, None))
        }
        TableArchetype::Survival(surv) => {
            let x_uid = format!("set-{ordinal}-x");
            let x_excl = collect_excluded_rows(surv.times.iter());
            let x_json = json!({
                "$id": x_uid,
                "@class": "DataSet",
                "uid": x_uid,
                "format": "y_single",
                "title": surv.time_title,
                "replicates": [build_replicate_json("real", None, &x_excl, &[])]
            });
            writer.add_file(&format!("data/sets/{x_uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            let x_rows = surv.times.iter().map(|c| vec![format_cell_value_unmasked(c, &csv_opts)]).collect();
            let x_col = ExportedXColumn { uid: x_uid, format: "y_single", csv_rows: x_rows };
            let y_cols = write_dataset_columns_json(
                writer,
                &surv.cohorts,
                ordinal,
                "y_single",
                ExportSubcolTransform::Direct,
                &csv_opts,
                ds.id.as_str(),
                warnings,
            )?;
            Ok(("survival", "XYDataTable", "y_single", None, Some(x_col), y_cols, None))
        }
        TableArchetype::PartsOfWhole(pow) => {
            let y_cols = write_dataset_columns_json(
                writer,
                &pow.data_sets,
                ordinal,
                "y_single",
                ExportSubcolTransform::Direct,
                &csv_opts,
                ds.id.as_str(),
                warnings,
            )?;
            Ok(("parts_of_whole", "DataTable", "y_single", None, None, y_cols, None))
        }
        TableArchetype::MultipleVariables(mv) => {
            let mut y_cols = Vec::with_capacity(mv.variables.len());
            for (c_idx, var) in mv.variables.iter().enumerate() {
                let uid = format!("set-{ordinal}-y{c_idx}");
                let excl = collect_excluded_rows(var.cells.iter());
                let is_text = var.cells.iter().any(|c| matches!(c.value, CellValue::Text(_)));
                let ds_json = json!({
                    "$id": uid,
                    "@class": "DataSet",
                    "uid": uid,
                    "format": "y_single",
                    "title": var.title,
                    "replicates": [build_replicate_json(if is_text { "string" } else { "real" }, var.display_decimals, &excl, &[])]
                });
                writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&ds_json)?)?;
                let csv_rows = var.cells.iter().map(|c| vec![format_cell_value_unmasked(c, &csv_opts)]).collect();
                y_cols.push(ExportedYColumn { uid, csv_rows });
            }
            Ok(("multivariable", "DataTable", "y_single", None, None, y_cols, None))
        }
        TableArchetype::Nested(nested) => {
            let units = nested.groups.first().map_or(1, |g| g.biological_unit_count().max(1));
            let mut y_cols = Vec::with_capacity(nested.groups.len());
            let mut st_entries = Vec::new();
            for (g_idx, grp) in nested.groups.iter().enumerate() {
                let uid = format!("set-{ordinal}-y{g_idx}");
                let mut reps_json = Vec::with_capacity(units);
                for s in 0..units {
                    let excl: Vec<usize> =
                        grp.cells.iter().enumerate().filter_map(|(r, row)| row.get(s).is_some_and(|c| c.excluded).then_some(r)).collect();
                    reps_json.push(build_replicate_json("real", None, &excl, &[]));
                }
                let ds_json = json!({
                    "$id": uid,
                    "@class": "DataSet",
                    "uid": uid,
                    "format": "y_replicates",
                    "title": grp.title,
                    "replicates": reps_json
                });
                writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&ds_json)?)?;
                let csv_rows = grp
                    .cells
                    .iter()
                    .map(|row| (0..units).map(|s| row.get(s).map_or_else(String::new, |c| format_cell_value_unmasked(c, &csv_opts))).collect())
                    .collect();
                y_cols.push(ExportedYColumn { uid, csv_rows });

                let rep_titles: Vec<Value> =
                    grp.subcolumn_titles.iter().enumerate().map(|(s, name)| json!({ "replicate": s, "name": name })).collect();
                st_entries.push(json!({ "column": g_idx, "replicates": rep_titles }));
            }
            let st_uid = if st_entries.is_empty() {
                None
            } else {
                let uid = format!("set-{ordinal}-subtitles");
                let st_json = json!({
                    "$id": uid,
                    "@class": "SubColumnTitlesDataSet",
                    "uid": uid,
                    "replicate count": units,
                    "titles": st_entries
                });
                writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&st_json)?)?;
                Some(uid)
            };
            Ok(("nested", "DataTable", "y_replicates", Some(units), None, y_cols, st_uid))
        }
    }
}

fn write_xy_x_dataset(
    writer: &mut ZipWriter,
    xy: &curvebook_doc::XyTable,
    ordinal: usize,
    sheet_id: &str,
    csv_opts: &CsvOptions,
    warnings: &mut Vec<ImportWarning>,
) -> Result<ExportedXColumn, FormatError> {
    let uid = format!("set-{ordinal}-x");
    let row_count = xy.row_count();
    match &xy.x_mode {
        XColumnMode::GeneratedSequence { start, step, progression: SequenceProgression::Arithmetic } => {
            let mut rep = serde_json::Map::new();
            rep.insert(String::from("valueType"), Value::String(String::from("real")));
            rep.insert(String::from("startValue"), json!(start));
            rep.insert(String::from("interval"), json!(step));
            if let Some(d) = xy.x_display_decimals {
                rep.insert(String::from("decimalsLength"), json!(d));
            }
            let x_json = json!({
                "$id": uid,
                "@class": "DataSet",
                "uid": uid,
                "format": "series",
                "title": xy.x_title,
                "replicates": [Value::Object(rep)]
            });
            writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            // Series X occupies 0 columns in `data.csv`!
            Ok(ExportedXColumn { uid, format: "series", csv_rows: vec![Vec::new(); row_count] })
        }
        XColumnMode::GeneratedSequence { progression: SequenceProgression::Geometric, .. } => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                sheet_id,
                format!("geometric X sequence on `{sheet_id}` materialized as explicit numeric X values in `.prism` export"),
            ));
            let x_json = json!({
                "$id": uid,
                "@class": "DataSet",
                "uid": uid,
                "format": "y_single",
                "title": xy.x_title,
                "replicates": [build_replicate_json("real", xy.x_display_decimals, &[], &[])]
            });
            writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            let csv_rows = (0..row_count).map(|r| vec![format_cell_value_unmasked(&xy.materialized_x_cell_at(r), csv_opts)]).collect();
            Ok(ExportedXColumn { uid, format: "y_single", csv_rows })
        }
        XColumnMode::DateOrElapsedTime { unit_label } => {
            let excl = collect_excluded_rows(xy.x_cells.iter().filter_map(|r| r.first()));
            let x_json = json!({
                "$id": uid,
                "@class": "DataSet",
                "uid": uid,
                "format": "timeSpan",
                "title": xy.x_title,
                "dateTimeConversion": { "units": unit_label },
                "replicates": [build_replicate_json("timespan", xy.x_display_decimals, &excl, &[])]
            });
            writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            let csv_rows = xy.x_cells.iter().map(|r| vec![r.first().map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts))]).collect();
            Ok(ExportedXColumn { uid, format: "timeSpan", csv_rows })
        }
        XColumnMode::Numbers { subcolumn_mode } => {
            let x_subs = subcolumn_mode.subcolumn_count();
            let (fmt, emit_subs) = if x_subs >= 2 { ("y_errorBar", 2) } else { ("y_single", 1) };
            if x_subs > 2 {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::CoercedValue,
                    sheet_id,
                    format!("X subcolumn mode {subcolumn_mode:?} truncated to 2 subcolumns (`y_errorBar`) in `.prism` export"),
                ));
            }
            let mut reps = Vec::with_capacity(emit_subs);
            for s in 0..emit_subs {
                let excl: Vec<usize> =
                    xy.x_cells.iter().enumerate().filter_map(|(r, row)| row.get(s).is_some_and(|c| c.excluded).then_some(r)).collect();
                reps.push(build_replicate_json("real", xy.x_display_decimals, &excl, &[]));
            }
            let x_json = json!({
                "$id": uid,
                "@class": "DataSet",
                "uid": uid,
                "format": fmt,
                "title": xy.x_title,
                "replicates": reps
            });
            writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&x_json)?)?;
            let csv_rows = xy
                .x_cells
                .iter()
                .map(|row| (0..emit_subs).map(|s| row.get(s).map_or_else(String::new, |c| format_cell_value_unmasked(c, csv_opts))).collect())
                .collect();
            Ok(ExportedXColumn { uid, format: fmt, csv_rows })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_dataset_columns_json(
    writer: &mut ZipWriter,
    columns: &[DataSetColumn],
    ordinal: usize,
    data_format: &str,
    transform: ExportSubcolTransform,
    csv_opts: &CsvOptions,
    sheet_id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Vec<ExportedYColumn>, FormatError> {
    let mut out = Vec::with_capacity(columns.len());
    for (c_idx, col) in columns.iter().enumerate() {
        let uid = format!("set-{ordinal}-y{c_idx}");
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
            reps_json.push(build_replicate_json("real", col.display_decimals, &excl, &[]));
        }
        let ds_json = json!({
            "$id": uid,
            "@class": "DataSet",
            "uid": uid,
            "format": data_format,
            "title": col.title,
            "replicates": reps_json
        });
        writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&ds_json)?)?;

        let mut csv_rows = Vec::with_capacity(col.cells.len());
        let mut warned_zero_cv = false;
        for row in &col.cells {
            let formatted = match transform {
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
                                warned_zero_cv = true;
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
            };
            csv_rows.push(formatted);
        }
        if warned_zero_cv {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                sheet_id,
                format!("MeanCvN row with Mean = 0 on `{sheet_id}` converted to SD = 0 in `.prism` `y_cv_n` export"),
            ));
        }
        out.push(ExportedYColumn { uid, csv_rows });
    }
    Ok(out)
}

fn write_subcol_titles_if_any(writer: &mut ZipWriter, columns: &[DataSetColumn], ordinal: usize) -> Result<Option<String>, FormatError> {
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
    let uid = format!("set-{ordinal}-subtitles");
    let st_json = json!({
        "$id": uid,
        "@class": "SubColumnTitlesDataSet",
        "uid": uid,
        "replicate count": max_subs,
        "titles": entries
    });
    writer.add_file(&format!("data/sets/{uid}.json"), serde_json::to_vec_pretty(&st_json)?)?;
    Ok(Some(uid))
}

fn write_info_sheet_entry(
    writer: &mut ZipWriter,
    info: &InfoSheet,
    info_uid: &str,
    sheet_id_to_uid: &BTreeMap<SheetId, String>,
    sheet_attrs: &mut serde_json::Map<String, Value>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    if !info.floating_notes.is_empty() {
        warnings.push(ImportWarning::new(
            ImportWarningKind::IgnoredSection,
            info.id.as_str(),
            format!("dropped {} floating note(s) on info sheet `{}`", info.floating_notes.len(), info.title),
        ));
    }
    sheet_attrs.insert(info_uid.to_owned(), build_sheet_attr_obj(&info.title, info.color_tag, false));

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
    obj.insert(String::from("$id"), Value::String(info_uid.to_owned()));
    obj.insert(String::from("@class"), Value::String(String::from("InfoSheet")));
    obj.insert(String::from("uid"), Value::String(info_uid.to_owned()));
    obj.insert(String::from("title"), Value::String(info.title.clone()));
    obj.insert(String::from("experimentDate"), Value::String(experiment_date));
    obj.insert(String::from("notesText"), Value::String(info.notes_markdown.clone()));
    obj.insert(String::from("constants"), Value::Array(constants_arr));
    if let Some(first_linked) = info.linked_data_sheets.first().and_then(|id| sheet_id_to_uid.get(id)) {
        obj.insert(String::from("linkedDataSheetID"), Value::String(first_linked.clone()));
    }
    if let Some(c) = info.color_tag {
        obj.insert(String::from("highlightColor"), Value::String(format_rgba_hex(c)));
    }
    writer.add_file(&format!("info/{info_uid}/sheet.json"), serde_json::to_vec_pretty(&Value::Object(obj))?)?;
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

fn build_replicate_json(value_type: &str, decimals: Option<u8>, excluded_rows: &[usize], censored_rows: &[usize]) -> Value {
    let mut map = serde_json::Map::new();
    map.insert(String::from("valueType"), Value::String(value_type.to_owned()));
    if let Some(d) = decimals {
        map.insert(String::from("decimalsLength"), json!(d));
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
