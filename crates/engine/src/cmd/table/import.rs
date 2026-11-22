//! Data-import commands: the import-link pipeline shared by `table.pasteLink` and
//! `table.importCsv`, plus `table.importPrism`, `table.importPzfx`, `table.importPlateMap`, and
//! `table.importTidyLong`. Command specs stay in the parent `table` module.

use std::collections::{HashMap, HashSet};

use curvebook_doc::{
    CellCoord, DataSheet, DocError, ImportLinkSpec, InfoSheet, InternalLinkKind, LinkSource, MAX_SHEETS_PER_PROJECT, PlacementMode, ProjectDocument,
    ResultSheet, SheetCellRange, SheetId, SheetSection, SubcolumnMode, TableArchetype, VariableSource, validate_project_cells, validate_title,
};
use curvebook_format::{
    CsvOptions, MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES, MAX_XML_BYTES, import_csv_to_datasheet, import_prism_bytes, import_pzfx_bytes,
    import_pzfx_str, parse_delimited_records,
};
use curvebook_transform::{
    ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, PlateGeometry, PlateMapTemplate, PlateWellRole, TargetArchetypeKind,
    TidyColumnMapping, apply_import_link_pipeline, map_plate_to_datasheet, pivot_tidy_long_to_datasheet,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CommandOutcome, EngineError, ErrorCode, ExternalPathPolicy, dag};

use super::{
    BaselineInput, CellRangeInput, PlacementFields, anchor_schema, baseline_schema, cell_range_schema, col_ref_schema, decimal_sep_schema,
    delimiter_schema, ensure_mv_edit_preserves_calculated, ensure_range_outside_linked_regions, import_filter_schema, parse_import_filter_val,
    parse_table_kind, parse_target_archetype_kind, resolve_baseline_adjustment, resolve_decimal_char, resolve_delimiter, resolve_subcolumn_mode,
    validate_project_cell_budget, validate_project_cell_budget_after_change,
};
use crate::cmd::Ctx;
use crate::cmd::helpers::{
    column_subcolumn_count, decode_base64, ensure_sheet_not_frozen, locate_target_and_info_indices, next_unique_sheet_id,
    next_unique_sheet_id_excluding, parse_sheet_id, remap_analysis_hooks, remap_data_sheet_links, remap_graph_spec_references,
    remap_layout_spec_references, remap_sheet_id, require_data_sheet, select_result_tab, sync_info_linked_data_sheets,
};
use crate::cmd::params::{
    any_json_schema, array_schema, bool_schema, deserialize_strict, int_schema, non_empty_str_schema, nullable_schema, num_schema,
    obj_schema_with_aliases, parse_params, record_deprecated_object_aliases, str_schema,
};
use crate::cmd::project::recalculate_dirty_with_links;

fn check_inline_payload_len(len: usize, max_bytes: usize, label: &str) -> Result<(), EngineError> {
    if len > max_bytes {
        return Err(EngineError::with_code(
            ErrorCode::ResourceLimit,
            format!("{label} payload size ({len} bytes) exceeds maximum ({max_bytes} bytes)"),
        ));
    }
    Ok(())
}

pub(super) enum OwnedSourcePayload {
    Data(Box<DataSheet>),
    Result(Box<ResultSheet>),
    Delimited { text: String, delimiter: char, decimal_separator: char },
}

pub(super) fn resolve_initial_link_source_payload(
    doc: &ProjectDocument,
    source: &LinkSource,
    policy: &ExternalPathPolicy,
) -> Result<Option<OwnedSourcePayload>, EngineError> {
    match source {
        LinkSource::Clipboard => Ok(None),
        LinkSource::InternalSheet { sheet_id, kind, .. } => match kind {
            InternalLinkKind::DataSheet => Ok(doc.data_sheet(sheet_id).cloned().map(Box::new).map(OwnedSourcePayload::Data)),
            InternalLinkKind::AnalyzableResultSheet | InternalLinkKind::TerminalResultSheet => {
                Ok(doc.result_sheet(sheet_id).cloned().map(Box::new).map(OwnedSourcePayload::Result))
            }
        },
        LinkSource::ExternalDelimitedFile { path, delimiter, decimal_separator } => {
            let text = policy.read_utf8_file(path, "linked file", MAX_CSV_FILE_BYTES)?;
            Ok(Some(OwnedSourcePayload::Delimited { text, delimiter: *delimiter, decimal_separator: *decimal_separator }))
        }
    }
}

pub(super) fn run_import_pipeline(
    target_ds: &mut DataSheet,
    spec: &ImportLinkSpec,
    payload: &OwnedSourcePayload,
    opts: &ImportPipelineOptions,
    info_sheet: Option<&mut InfoSheet>,
) -> Result<ImportPipelineReport, EngineError> {
    let report = match payload {
        OwnedSourcePayload::Data(src) => apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalData(src), opts, info_sheet)?,
        OwnedSourcePayload::Result(src) => apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalResult(src), opts, info_sheet)?,
        OwnedSourcePayload::Delimited { text, delimiter, decimal_separator } => {
            let src = ImportSourcePayload::DelimitedText { text, delimiter: *delimiter, decimal_separator: *decimal_separator };
            apply_import_link_pipeline(target_ds, spec, src, opts, info_sheet)?
        }
    };
    target_ds.prune_stale_exclusion_notes();
    Ok(report)
}

pub(super) fn apply_pipeline_with_split_info(
    doc: &mut ProjectDocument,
    target_idx: usize,
    info_idx: Option<usize>,
    spec: &ImportLinkSpec,
    payload: &OwnedSourcePayload,
    opts: &ImportPipelineOptions,
) -> Result<ImportPipelineReport, EngineError> {
    let ProjectDocument { data_sheets, info_sheets, .. } = doc;
    let target_ds =
        data_sheets.get_mut(target_idx).ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "target data sheet index out of bounds"))?;
    let target_info = info_idx.and_then(|idx| info_sheets.get_mut(idx));
    let report = run_import_pipeline(target_ds, spec, payload, opts, target_info)?;
    if let OwnedSourcePayload::Data(src) = payload {
        dag::propagate_internal_source_titles(target_ds, spec, report.target_range.start.col, src);
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Shared Deserialization & Parsing Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct PasteLinkParams {
    pub(super) target_sheet_id: String,
    pub(super) link_spec: Option<Value>,
    pub(super) source_sheet_id: Option<String>,
    pub(super) source_kind: Option<String>,
    pub(super) tab_id: Option<String>,
    pub(super) source_range: Option<CellRangeInput>,
    pub(super) source_path: Option<String>,
    pub(super) delimiter: Option<String>,
    pub(super) decimal_separator: Option<String>,
    pub(super) normalize_decimal: Option<bool>,
    pub(super) filter: Option<Value>,
    pub(super) baseline: Option<BaselineInput>,
    pub(super) exclude_row_predicate: Option<String>,
    pub(super) link_id: Option<String>,
    pub(super) provenance: Option<String>,
    #[serde(flatten)]
    pub(super) placement: PlacementFields,
}

pub(super) fn schema_paste_link() -> Value {
    obj_schema_with_aliases(
        vec![
            ("target_sheet_id", non_empty_str_schema("Destination data sheet identifier")),
            ("link_spec", nullable_schema(json!({ "type": "object", "description": "Direct ImportLinkSpec payload" }))),
            ("source_sheet_id", nullable_schema(non_empty_str_schema("Internal source Data or Result sheet ID"))),
            ("source_kind", nullable_schema(str_schema("Explicit source kind: data, result, analyzable_result, terminal_result"))),
            ("tab_id", nullable_schema(str_schema("Result sheet tab ID or title"))),
            ("source_range", nullable_schema(cell_range_schema())),
            ("source_path", nullable_schema(non_empty_str_schema("External delimited file path"))),
            ("delimiter", delimiter_schema()),
            ("decimal_separator", decimal_sep_schema()),
            ("normalize_decimal", bool_schema("Infer European decimal separator when ambiguous")),
            ("filter", nullable_schema(import_filter_schema())),
            ("baseline", nullable_schema(baseline_schema())),
            ("exclude_row_predicate", nullable_schema(str_schema("Optional predicate excluding matching linked rows"))),
            ("link_id", nullable_schema(str_schema("Custom link identifier"))),
            ("provenance", nullable_schema(str_schema("Custom provenance note recorded in linked InfoSheet"))),
            ("anchor", anchor_schema()),
            ("start_row", int_schema("Target anchor row", Some(0), None)),
            ("start_col", int_schema("Target anchor column", Some(0), None)),
            ("start_subcol", int_schema("Target anchor subcolumn", Some(0), None)),
            ("mode", str_schema("Placement mode: overwrite, insert_rows, append_rows, append_columns")),
            ("propagate_title_header", bool_schema("Propagate source titles to target headers")),
            ("skip_contiguous_blanks", bool_schema("Skip contiguous blanks")),
            ("transpose", bool_schema("Transpose source matrix")),
        ],
        &["target_sheet_id"],
        vec![
            ("target_sheet_id", &["targetSheetId", "sheet_id", "sheetId"]),
            ("link_spec", &["linkSpec"]),
            ("source_sheet_id", &["sourceSheetId"]),
            ("source_kind", &["sourceKind"]),
            ("tab_id", &["tabId", "source_tab_id", "sourceTabId"]),
            ("source_range", &["sourceRange", "range"]),
            ("source_path", &["sourcePath", "path", "file_path", "filePath"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("normalize_decimal", &["normalizeDecimal"]),
            ("filter", &["slice"]),
            ("exclude_row_predicate", &["excludeRowPredicate"]),
            ("link_id", &["linkId"]),
            ("provenance", &["provenance_summary"]),
            ("anchor", &["start"]),
            ("start_row", &["startRow", "target_row", "targetRow", "row"]),
            ("start_col", &["startCol", "target_col", "targetCol", "col", "data_set", "dataSet"]),
            ("start_subcol", &["startSubcol", "target_subcol", "targetSubcol", "subcol", "subcolumn"]),
            ("mode", &["placement_mode", "placementMode"]),
            ("propagate_title_header", &["propagateTitleHeader"]),
            ("skip_contiguous_blanks", &["skipContiguousBlanks"]),
        ],
        false,
    )
}

pub(super) fn summarize_link_source(source: &LinkSource) -> String {
    match source {
        LinkSource::Clipboard => String::from("Clipboard paste"),
        LinkSource::InternalSheet { sheet_id, kind, .. } => format!("Link from {kind:?} `{sheet_id}`"),
        LinkSource::ExternalDelimitedFile { path, .. } => format!("Link from `{path}`"),
    }
}

pub(super) fn resolve_internal_link_kind(doc: &ProjectDocument, src_id: &SheetId, explicit: Option<&str>) -> Result<InternalLinkKind, EngineError> {
    if let Some(raw) = explicit {
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        return match norm.as_str() {
            "data" | "datasheet" => Ok(InternalLinkKind::DataSheet),
            "analyzableresult" | "analyzableresultsheet" => Ok(InternalLinkKind::AnalyzableResultSheet),
            "terminalresult" | "terminalresultsheet" => Ok(InternalLinkKind::TerminalResultSheet),
            "result" | "resultsheet" => {
                let res = doc
                    .result_sheet(src_id)
                    .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{src_id}` not found")))?;
                Ok(if res.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet })
            }
            _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown link source kind `{raw}`"))),
        };
    }
    match doc.sheet_section(src_id) {
        Some(SheetSection::Data) => Ok(InternalLinkKind::DataSheet),
        Some(SheetSection::Analyses) => {
            let res =
                doc.result_sheet(src_id).ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{src_id}` not found")))?;
            Ok(if res.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet })
        }
        Some(other) => Err(EngineError::with_code(ErrorCode::BadInput, format!("sheet `{src_id}` in section {other:?} cannot be link source"))),
        None => Err(EngineError::with_code(ErrorCode::NotFound, format!("source sheet `{src_id}` not found"))),
    }
}

pub(super) fn default_full_source_range(
    doc: &ProjectDocument,
    src_id: &SheetId,
    kind: InternalLinkKind,
    tab_id: Option<&str>,
) -> Result<SheetCellRange, EngineError> {
    match kind {
        InternalLinkKind::DataSheet => {
            let ds = require_data_sheet(doc, src_id)?;
            let max_r = ds.archetype.row_count().saturating_sub(1);
            let max_c = ds.archetype.column_count().saturating_sub(1);
            let max_s = (0..ds.archetype.column_count()).map(|c| column_subcolumn_count(&ds.archetype, c)).max().unwrap_or(1).saturating_sub(1);
            Ok(SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(max_r, max_c, max_s)))
        }
        InternalLinkKind::AnalyzableResultSheet | InternalLinkKind::TerminalResultSheet => {
            let res = doc
                .result_sheet(src_id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("source result sheet `{src_id}` not found")))?;
            let tab = select_result_tab(res, tab_id)?;
            let max_r = tab.rows.len().saturating_sub(1);
            let max_c = tab.column_headers.len().max(tab.rows.first().map_or(0, Vec::len)).saturating_sub(1);
            Ok(SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(max_r, max_c, 0)))
        }
    }
}

pub(super) fn build_paste_link_spec(
    doc: &ProjectDocument,
    target_id: &SheetId,
    p: &PasteLinkParams,
    policy: &ExternalPathPolicy,
) -> Result<ImportLinkSpec, EngineError> {
    let resolved_baseline = resolve_baseline_adjustment(p.baseline.clone())?;
    if let Some(raw_spec) = &p.link_spec {
        let mut parsed: ImportLinkSpec = deserialize_strict(raw_spec, "/link_spec").map_err(|mut err| {
            if err.remediation.is_none() {
                err.remediation = Some(String::from("Provide a valid ImportLinkSpec object or omit `link_spec` to use shorthand parameters."));
            }
            err
        })?;
        policy.normalize_stored_link_source(&mut parsed.source)?;
        if p.placement.anchor.is_some() {
            parsed.placement.anchor = p.placement.resolve_anchor();
        }
        if let Some(t) = p.placement.transpose {
            parsed.placement.transpose = t;
        }
        if let Some(prop) = p.placement.propagate_title_header {
            parsed.placement.propagate_title_header = prop;
        }
        if let Some(baseline) = resolved_baseline {
            parsed.filter.baseline = Some(baseline);
        }
        if let Some(ref pred) = p.exclude_row_predicate {
            parsed.filter.exclude_row_predicate = Some(pred.clone());
        }
        return Ok(parsed);
    }

    let anchor = p.placement.resolve_anchor();
    let transpose = p.placement.transpose.unwrap_or(false);
    let default_prop = transpose && anchor.row == 0;
    let placement = p.placement.to_placement_spec(default_prop)?;
    let mut filter = parse_import_filter_val(p.filter.as_ref())?;
    if let Some(baseline) = resolved_baseline {
        filter.baseline = Some(baseline);
    }
    if let Some(ref pred) = p.exclude_row_predicate {
        filter.exclude_row_predicate = Some(pred.clone());
    }

    let source = if let Some(src_str) = p.source_sheet_id.as_deref() {
        let src_id = parse_sheet_id(src_str)?;
        if &src_id == target_id {
            return Err(EngineError::with_code(ErrorCode::CycleDetected, format!("self-link on `{target_id}` would create a DAG cycle"))
                .with_remediation("Link from a different upstream data or result sheet."));
        }
        let kind = resolve_internal_link_kind(doc, &src_id, p.source_kind.as_deref())?;
        let tab_id = p.tab_id.clone();
        let range = match &p.source_range {
            Some(r) => r.resolve()?,
            None => default_full_source_range(doc, &src_id, kind, tab_id.as_deref())?,
        };
        LinkSource::InternalSheet { sheet_id: src_id, kind, tab_id, range }
    } else if let Some(path) = p.source_path.as_deref() {
        let preview_text = policy.read_utf8_file(path, "external link file", MAX_CSV_FILE_BYTES)?;
        let delimiter = resolve_delimiter(p.delimiter.as_deref(), &preview_text)?;
        let decimal_separator = resolve_decimal_char(p.decimal_separator.as_deref(), p.normalize_decimal.unwrap_or(false), &preview_text, delimiter)?;
        let mut src = LinkSource::ExternalDelimitedFile { path: path.to_owned(), delimiter, decimal_separator };
        policy.normalize_stored_link_source(&mut src)?;
        src
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.pasteLink requires `link_spec`, `source_sheet_id`, or `path`")
            .with_remediation("Specify an internal `source_sheet_id` or an external `path`."));
    };

    Ok(ImportLinkSpec { source, filter, placement })
}

pub(super) fn run_paste_link(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: PasteLinkParams = parse_params(&schema_paste_link(), raw)?;
    let target_id = parse_sheet_id(&p.target_sheet_id)?;
    let policy = ctx.session.path_policy.clone();
    let doc = &mut ctx.session.doc;

    let info_id = {
        let target_ds = require_data_sheet(doc, &target_id)?;
        ensure_sheet_not_frozen(target_ds)?;
        target_ds.info_sheet_id.clone()
    };

    let spec = build_paste_link_spec(doc, &target_id, &p, &policy)?;
    let payload = resolve_initial_link_source_payload(doc, &spec.source, &policy)?
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "table.pasteLink requires an internal sheet or external delimited file source"))?;
    let (target_idx, info_idx) = locate_target_and_info_indices(doc, &target_id, info_id.as_ref())?;
    let prev_ds = doc.data_sheets.get(target_idx).cloned();
    let prev_info = info_idx.and_then(|idx| doc.info_sheets.get(idx).cloned());

    let existing_link_ids: HashSet<String> = prev_ds.as_ref().map(|d| d.linked_regions.iter().map(|r| r.id.clone()).collect()).unwrap_or_default();
    let link_id = if let Some(raw_id) = p.link_id {
        let trimmed = raw_id.trim();
        if trimmed.is_empty() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`link_id` must not be empty"));
        }
        if existing_link_ids.contains(trimmed) {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("live linked region `{trimmed}` already exists on `{}`", target_id.as_str()),
            ));
        }
        trimmed.to_owned()
    } else {
        let mut seq = prev_ds.as_ref().map_or(1, |d| d.linked_regions.len().saturating_add(1));
        loop {
            let candidate = format!("link-{}-{seq}", target_id.as_str());
            if !existing_link_ids.contains(&candidate) {
                break candidate;
            }
            seq = seq.saturating_add(1);
        }
    };
    let provenance_summary = p.provenance.unwrap_or_else(|| summarize_link_source(&spec.source));
    let opts = ImportPipelineOptions {
        baseline: spec.filter.baseline,
        exclude_coords: Vec::new(),
        exclude_row_predicate: spec.filter.exclude_row_predicate.clone(),
        link_id: Some(link_id),
        provenance_summary: Some(provenance_summary),
    };

    let report = match (|| -> Result<ImportPipelineReport, EngineError> {
        let rep = apply_pipeline_with_split_info(doc, target_idx, info_idx, &spec, &payload, &opts)?;
        if let Some(old_ds) = prev_ds.as_ref()
            && rep.rows_placed > 0
            && rep.cols_placed > 0
        {
            ensure_range_outside_linked_regions(old_ds, rep.target_range, "paste-link over")?;
            if matches!(spec.placement.mode, PlacementMode::InsertRows) {
                let shift_span = SheetCellRange::new(
                    CellCoord::new(spec.placement.anchor.row, rep.target_range.start.col, rep.target_range.start.subcol),
                    CellCoord::new(usize::MAX, rep.target_range.end.col, rep.target_range.end.subcol),
                );
                ensure_range_outside_linked_regions(old_ds, shift_span, "insert linked rows into")?;
            }
        }
        if let OwnedSourcePayload::Result(src_rs) = &payload
            && let Some(target_ds) = doc.data_sheets.get_mut(target_idx)
        {
            dag::backfill_confidence_interval_cells(target_ds, src_rs, &spec, rep.target_range);
        }
        if let (Some(old_ds), Some(new_ds)) = (prev_ds.as_ref(), doc.data_sheets.get(target_idx)) {
            ensure_mv_edit_preserves_calculated(old_ds, new_ds, rep.cols_placed, rep.target_range)?;
        }
        validate_project_cell_budget(doc)?;
        let _ = dag::topological_order(doc)?;
        Ok(rep)
    })() {
        Ok(r) => r,
        Err(e) => {
            if let (Some(slot), Some(old)) = (doc.data_sheets.get_mut(target_idx), prev_ds) {
                *slot = old;
            }
            if let (Some(idx), Some(old_info)) = (info_idx, prev_info)
                && let Some(slot) = doc.info_sheets.get_mut(idx)
            {
                *slot = old_info;
            }
            return Err(e);
        }
    };

    let mut dirty_seeds = vec![target_id.clone()];
    if info_idx.is_some()
        && let Some(ref iid) = info_id
    {
        dirty_seeds.push(iid.clone());
    }
    let recalc = recalculate_dirty_with_links(doc, &dirty_seeds, false)?;
    Ok(CommandOutcome::mutated(
        "table.pasteLink",
        "created live linked region",
        json!({
            "target_sheet_id": target_id.as_str(),
            "rows_placed": report.rows_placed,
            "cols_placed": report.cols_placed,
            "target_range": &report.target_range,
            "linked_regions": doc.data_sheet(&target_id).map_or(0, |d| d.linked_regions.len()),
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 14. table.importCsv
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct ImportCsvParams {
    pub(super) sheet_id: Option<String>,
    pub(super) csv: Option<String>,
    pub(super) path: Option<String>,
    pub(super) delimiter: Option<String>,
    pub(super) decimal_separator: Option<String>,
    pub(super) normalize_decimal: Option<bool>,
    pub(super) has_header: Option<bool>,
    pub(super) has_row_titles: Option<bool>,
    pub(super) replicates: Option<usize>,
    pub(super) subcolumn_mode: Option<Value>,
    pub(super) replace: Option<bool>,
    pub(super) title: Option<String>,
    pub(super) archetype: Option<String>,
    pub(super) filter: Option<Value>,
    pub(super) link: Option<bool>,
    #[serde(flatten)]
    pub(super) placement: PlacementFields,
}

pub(super) fn schema_import_csv() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional target sheet ID (auto-generated if omitted)"))),
            ("csv", nullable_schema(str_schema("Inline CSV/TSV content"))),
            ("path", nullable_schema(non_empty_str_schema("Filesystem path to CSV/TSV file"))),
            ("delimiter", delimiter_schema()),
            ("decimal_separator", decimal_sep_schema()),
            ("normalize_decimal", bool_schema("Infer European decimal separator when ambiguous")),
            ("has_header", bool_schema("Treat first row as column headers")),
            ("has_row_titles", bool_schema("Treat first column as row titles")),
            ("replicates", int_schema("Replicate count per group (default 1)", Some(1), None)),
            ("subcolumn_mode", nullable_schema(any_json_schema("Subcolumn mode name or object"))),
            ("replace", bool_schema("Replace existing target sheet instead of placing into region")),
            ("title", nullable_schema(str_schema("Title for imported sheet"))),
            (
                "archetype",
                nullable_schema(str_schema("Target archetype (XY, Column, Grouped, Contingency, Survival, PartsOfWhole, MultipleVariables, Nested)")),
            ),
            ("filter", nullable_schema(import_filter_schema())),
            ("link", bool_schema("When importing from `path` into an existing sheet, keep a live file link")),
            ("anchor", anchor_schema()),
            ("start_row", int_schema("Target anchor row", Some(0), None)),
            ("start_col", int_schema("Target anchor column", Some(0), None)),
            ("start_subcol", int_schema("Target anchor subcolumn", Some(0), None)),
            ("mode", str_schema("Placement mode: overwrite, insert_rows, append_rows, append_columns")),
            ("propagate_title_header", bool_schema("Propagate header row titles")),
            ("skip_contiguous_blanks", bool_schema("Skip contiguous blanks")),
            ("transpose", bool_schema("Transpose imported table")),
        ],
        &[],
        vec![
            ("sheet_id", &["sheetId"]),
            ("csv", &["tsv", "text", "content"]),
            ("path", &["file_path", "filePath"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("normalize_decimal", &["normalizeDecimal"]),
            ("has_header", &["hasHeader", "first_row_titles", "firstRowTitles"]),
            ("has_row_titles", &["hasRowTitles"]),
            ("replicates", &["k"]),
            ("subcolumn_mode", &["subcolumnMode"]),
            ("archetype", &["kind", "table_kind", "tableKind"]),
            ("filter", &["slice"]),
            ("link", &["live_link", "liveLink"]),
            ("anchor", &["start"]),
            ("start_row", &["startRow", "target_row", "targetRow", "row"]),
            ("start_col", &["startCol", "target_col", "targetCol", "col", "data_set", "dataSet"]),
            ("start_subcol", &["startSubcol", "target_subcol", "targetSubcol", "subcol", "subcolumn"]),
            ("mode", &["placement_mode", "placementMode"]),
            ("propagate_title_header", &["propagateTitleHeader"]),
            ("skip_contiguous_blanks", &["skipContiguousBlanks"]),
        ],
        false,
    )
}

pub(super) fn build_csv_options(delimiter: char, decimal_separator: char) -> CsvOptions {
    let mut opts = match (delimiter, decimal_separator) {
        ('\t', _) => CsvOptions::tsv(),
        (';', ',') => CsvOptions::european_csv(),
        _ => CsvOptions::csv(),
    };
    opts.delimiter = delimiter;
    opts.decimal_separator = decimal_separator;
    opts
}

pub(super) fn import_into_existing_sheet(
    doc: &mut ProjectDocument,
    id: &SheetId,
    p: &ImportCsvParams,
    text: String,
    csv_opts: &CsvOptions,
    subcolumn_mode: SubcolumnMode,
    policy: &ExternalPathPolicy,
) -> Result<Option<SheetId>, EngineError> {
    let delimiter = csv_opts.delimiter;
    let decimal_separator = csv_opts.decimal_separator;
    let (info_id, cur_title, cur_kind) = {
        let ds = require_data_sheet(doc, id)?;
        ensure_sheet_not_frozen(ds)?;
        (ds.info_sheet_id.clone(), ds.title.clone(), ds.archetype.kind())
    };
    if p.replace.unwrap_or(false) {
        let title = p.title.as_deref().unwrap_or(&cur_title).to_owned();
        let kind = match p.archetype.as_deref() {
            Some(k) => parse_table_kind(k)?,
            None => cur_kind,
        };
        let mut imported = import_csv_to_datasheet(id.clone(), title, &text, kind, subcolumn_mode, csv_opts)?;
        validate_project_cell_budget_after_change(doc, Some(id), imported.archetype.allocated_cell_slots())?;
        imported.info_sheet_id = info_id;
        if let Some(slot) = doc.data_sheet_mut(id) {
            *slot = imported;
        }
        return Ok(None);
    }
    let filter = parse_import_filter_val(p.filter.as_ref())?;
    let placement = p.placement.to_placement_spec(false)?;
    let keep_link = p.link.unwrap_or(false);
    let mut source = match p.path.as_deref() {
        Some(path) if keep_link => LinkSource::ExternalDelimitedFile { path: path.to_owned(), delimiter, decimal_separator },
        _ => LinkSource::Clipboard,
    };
    policy.normalize_stored_link_source(&mut source)?;
    let link_spec = ImportLinkSpec { source, filter, placement };
    let (target_idx, info_idx) = locate_target_and_info_indices(doc, id, info_id.as_ref())?;
    let opts = ImportPipelineOptions {
        link_id: if keep_link { Some(format!("csv-link-{}", id.as_str())) } else { None },
        provenance_summary: if keep_link { Some(String::from("Imported CSV link")) } else { None },
        ..ImportPipelineOptions::default()
    };
    let link_before = doc.data_sheets.get(target_idx).map_or(0, |d| d.linked_regions.len());
    let has_calc_mv = doc.data_sheets.get(target_idx).is_some_and(|d| {
        matches!(
            &d.archetype,
            TableArchetype::MultipleVariables(mv) if mv.variables.iter().any(|v| matches!(v.source, VariableSource::Calculated { .. }))
        )
    });
    let pre_import_snapshot = doc.data_sheets.get(target_idx).cloned();
    let pre_info_snapshot = info_idx.and_then(|idx| doc.info_sheets.get(idx).cloned());
    let payload = OwnedSourcePayload::Delimited { text, delimiter, decimal_separator };
    let report = apply_pipeline_with_split_info(doc, target_idx, info_idx, &link_spec, &payload, &opts)?;
    if let Err(err) = validate_project_cell_budget(doc) {
        if let (Some(slot), Some(prev_ds)) = (doc.data_sheets.get_mut(target_idx), pre_import_snapshot) {
            *slot = prev_ds;
        }
        if let (Some(idx), Some(prev_info)) = (info_idx, pre_info_snapshot)
            && let Some(slot) = doc.info_sheets.get_mut(idx)
        {
            *slot = prev_info;
        }
        return Err(err);
    }
    if let (Some(prev_ds), Some(new_ds)) = (pre_import_snapshot.as_ref(), doc.data_sheets.get(target_idx)) {
        if has_calc_mv && let Err(err) = ensure_mv_edit_preserves_calculated(prev_ds, new_ds, report.cols_placed, report.target_range) {
            if let Some(slot) = doc.data_sheets.get_mut(target_idx) {
                *slot = prev_ds.clone();
            }
            if let (Some(idx), Some(prev_info)) = (info_idx, pre_info_snapshot)
                && let Some(slot) = doc.info_sheets.get_mut(idx)
            {
                *slot = prev_info;
            }
            return Err(err);
        }
        if !keep_link
            && link_before > 0
            && report.rows_placed > 0
            && report.cols_placed > 0
            && let Err(err) = ensure_range_outside_linked_regions(prev_ds, report.target_range, "import into")
        {
            if let Some(slot) = doc.data_sheets.get_mut(target_idx) {
                *slot = prev_ds.clone();
            }
            if let (Some(idx), Some(prev_info)) = (info_idx, pre_info_snapshot)
                && let Some(slot) = doc.info_sheets.get_mut(idx)
            {
                *slot = prev_info;
            }
            return Err(err);
        }
    }
    if !keep_link && let Some(target_ds) = doc.data_sheets.get_mut(target_idx) {
        target_ds.linked_regions.truncate(link_before);
    }
    Ok(info_idx.and(info_id))
}

pub(super) fn run_import_csv(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportCsvParams = parse_params(&schema_import_csv(), raw)?;
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    let policy = ctx.session.path_policy.clone();
    let doc = &mut ctx.session.doc;
    let id = match p.sheet_id.as_deref() {
        Some(raw_id) => parse_sheet_id(raw_id)?,
        None => next_unique_sheet_id(doc, "data")?,
    };
    let text = if let Some(inline) = &p.csv {
        check_inline_payload_len(inline.len(), MAX_CSV_FILE_BYTES, "CSV")?;
        inline.clone()
    } else if let Some(path) = &p.path {
        policy.read_utf8_file(path, "CSV", MAX_CSV_FILE_BYTES)?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importCsv requires `path` or `csv`/`text`")
            .with_remediation("Provide a CSV/TSV file `path` or inline `csv` text."));
    };

    let delimiter = resolve_delimiter(p.delimiter.as_deref(), &text)?;
    let decimal_separator = resolve_decimal_char(p.decimal_separator.as_deref(), p.normalize_decimal.unwrap_or(false), &text, delimiter)?;
    let mut csv_opts = build_csv_options(delimiter, decimal_separator);
    if let Some(first_row_titles) = p.has_header {
        csv_opts.header_rows = usize::from(first_row_titles);
    }
    if let Some(has_row_titles) = p.has_row_titles {
        csv_opts.has_row_titles = has_row_titles;
    }
    let subcolumn_mode = resolve_subcolumn_mode(p.subcolumn_mode.as_ref(), p.replicates.unwrap_or(1))?;

    let mut dirty_seeds = vec![id.clone()];
    if doc.data_sheet(&id).is_some() {
        if let Some(updated_info) = import_into_existing_sheet(doc, &id, &p, text, &csv_opts, subcolumn_mode, &policy)? {
            dirty_seeds.push(updated_info);
        }
    } else {
        let title = p.title.as_deref().unwrap_or("Imported CSV");
        let kind = parse_table_kind(p.archetype.as_deref().unwrap_or("xy"))?;
        let imported = import_csv_to_datasheet(id.clone(), title, &text, kind, subcolumn_mode, &csv_opts)?;
        doc.add_data_sheet(imported)?;
    }

    let recalc = recalculate_dirty_with_links(doc, &dirty_seeds, false)?;
    let message = match recalc.failure_note() {
        Some(note) => format!("imported CSV into data sheet{note}"),
        None => String::from("imported CSV into data sheet"),
    };
    Ok(CommandOutcome::mutated(
        "table.importCsv",
        message,
        json!({
            "sheet_id": id.as_str(),
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
            "failed_count": recalc.failed_result_sheets.len(),
            "failed_result_sheets": recalc.failed_json(),
        }),
    ))
}

// ---------------------------------------------------------------------------
// 15. table.exportCsv
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct ImportPrismParams {
    pub(super) path: Option<String>,
    pub(super) bytes_base64: Option<String>,
    pub(super) bytes: Option<Vec<u8>>,
    pub(super) merge: Option<bool>,
}

pub(super) fn schema_import_prism() -> Value {
    obj_schema_with_aliases(
        vec![
            ("path", nullable_schema(non_empty_str_schema("Filesystem path to external archive"))),
            ("bytes_base64", nullable_schema(non_empty_str_schema("Base64-encoded external archive bytes"))),
            ("bytes", array_schema("Raw byte array of archive contents", int_schema("Byte value 0..=255", Some(0), Some(255)), None, None)),
            ("merge", bool_schema("Merge imported sheets into existing document instead of replacing (default false)")),
        ],
        &[],
        vec![("path", &["file_path", "filePath"]), ("bytes_base64", &["bytesBase64", "archive_base64"])],
        false,
    )
}

pub(super) fn collect_all_sheet_ids(doc: &ProjectDocument) -> HashSet<String> {
    let mut ids = HashSet::with_capacity(doc.total_sheet_count());
    for id in doc
        .data_sheets
        .iter()
        .map(|s| &s.id)
        .chain(doc.info_sheets.iter().map(|s| &s.id))
        .chain(doc.result_sheets.iter().map(|s| &s.id))
        .chain(doc.graph_sheets.iter().map(|s| &s.id))
        .chain(doc.layout_sheets.iter().map(|s| &s.id))
    {
        ids.insert(id.as_str().to_owned());
    }
    ids
}

pub(super) fn next_collision_free_sheet_id(original: &SheetId, reserved: &mut HashSet<String>) -> Result<SheetId, EngineError> {
    let raw = original.as_str().trim();
    let base = if raw.is_empty() { "sheet" } else { raw };
    for suffix in 2..=100_000_usize {
        let candidate = SheetId::try_new(format!("{base}-{suffix}"))?;
        if reserved.insert(candidate.as_str().to_owned()) {
            return Ok(candidate);
        }
    }
    Err(EngineError::with_code(ErrorCode::ResourceLimit, format!("exhausted collision-free sheet IDs for `{raw}`")))
}

pub(super) fn plan_imported_id_remap(doc: &ProjectDocument, imported: &ProjectDocument) -> Result<HashMap<SheetId, SheetId>, EngineError> {
    let doc_ids = collect_all_sheet_ids(doc);
    let mut reserved = doc_ids.clone();
    reserved.extend(collect_all_sheet_ids(imported));

    let mut seen_imported: HashSet<String> = HashSet::with_capacity(imported.total_sheet_count());
    let mut id_map: HashMap<SheetId, SheetId> = HashMap::new();

    for orig_id in imported
        .data_sheets
        .iter()
        .map(|s| &s.id)
        .chain(imported.info_sheets.iter().map(|s| &s.id))
        .chain(imported.result_sheets.iter().map(|s| &s.id))
        .chain(imported.graph_sheets.iter().map(|s| &s.id))
        .chain(imported.layout_sheets.iter().map(|s| &s.id))
    {
        let key = orig_id.as_str();
        let needs_remap = doc_ids.contains(key) || !seen_imported.insert(key.to_owned());
        if needs_remap {
            let new_id = next_collision_free_sheet_id(orig_id, &mut reserved)?;
            id_map.entry(orig_id.clone()).or_insert(new_id);
        }
    }
    Ok(id_map)
}

pub(super) fn merge_imported_document(doc: &mut ProjectDocument, mut imported: ProjectDocument) -> Result<(), EngineError> {
    let total_sheets = doc.total_sheet_count().saturating_add(imported.total_sheet_count());
    if total_sheets > MAX_SHEETS_PER_PROJECT {
        return Err(EngineError::from(DocError::TooManySheets { count: total_sheets, max: MAX_SHEETS_PER_PROJECT }));
    }
    let existing_cells = doc.total_allocated_cells()?;
    let added_cells = imported.total_allocated_cells()?;
    validate_project_cells(existing_cells, added_cells)?;

    let id_map = plan_imported_id_remap(doc, &imported)?;
    if !id_map.is_empty() {
        for ds in &mut imported.data_sheets {
            remap_sheet_id(&mut ds.id, &id_map);
            remap_data_sheet_links(ds, &id_map);
        }
        for info in &mut imported.info_sheets {
            remap_sheet_id(&mut info.id, &id_map);
            for linked in &mut info.linked_data_sheets {
                remap_sheet_id(linked, &id_map);
            }
        }
        for res in &mut imported.result_sheets {
            remap_sheet_id(&mut res.id, &id_map);
            res.analysis.id = res.id.clone();
            remap_sheet_id(&mut res.analysis.source_sheet_id, &id_map);
            remap_analysis_hooks(&mut res.analysis, &id_map);
            if let Some(ref mut ad) = res.analyzable_data {
                ad.id = res.id.clone();
                remap_data_sheet_links(ad, &id_map);
            }
        }
        for gs in &mut imported.graph_sheets {
            remap_sheet_id(&mut gs.id, &id_map);
            remap_graph_spec_references(&mut gs.spec, &id_map);
        }
        for ls in &mut imported.layout_sheets {
            remap_sheet_id(&mut ls.id, &id_map);
            remap_layout_spec_references(&mut ls.spec, &id_map);
        }
    }

    for ds in imported.data_sheets {
        doc.add_data_sheet(ds)?;
    }
    for info in imported.info_sheets {
        doc.add_info_sheet(info)?;
    }
    for res in imported.result_sheets {
        doc.add_result_sheet(res)?;
    }
    for gs in imported.graph_sheets {
        doc.add_graph_sheet(gs)?;
    }
    for ls in imported.layout_sheets {
        doc.add_layout_sheet(ls)?;
    }
    let _ = dag::topological_order(doc)?;
    Ok(())
}

pub(super) fn run_import_prism(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportPrismParams = parse_params(&schema_import_prism(), raw)?;
    import_prism_with_params(ctx, p)
}

fn import_prism_with_params(ctx: &mut Ctx<'_>, p: ImportPrismParams) -> Result<CommandOutcome, EngineError> {
    let bytes = if let Some(path) = p.path.as_deref() {
        ctx.session.path_policy.read_binary_file(path, "archive", MAX_ARCHIVE_FILE_BYTES)?
    } else if let Some(b64) = p.bytes_base64.as_deref() {
        decode_base64(b64)?
    } else if let Some(raw_bytes) = p.bytes {
        check_inline_payload_len(raw_bytes.len(), MAX_ARCHIVE_FILE_BYTES, "archive")?;
        raw_bytes
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPrism requires `path`, `bytes_base64`, or `bytes`")
            .with_remediation("Supply `path`, `bytes_base64`, or `bytes`."));
    };

    let report = import_prism_bytes(&bytes)?;
    let doc = &mut ctx.session.doc;
    if p.merge.unwrap_or(false) {
        merge_imported_document(doc, report.document)?;
    } else {
        validate_project_cells(0, report.document.total_allocated_cells()?)?;
        *doc = report.document;
    }
    sync_info_linked_data_sheets(doc, &HashSet::new());
    Ok(CommandOutcome::mutated(
        "table.importPrism",
        "imported external archive",
        json!({
            "title": doc.title,
            "sheet_count": doc.total_sheet_count(),
            "warnings": report.warnings,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 17. table.importPzfx
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct ImportPzfxParams {
    pub(super) xml: Option<String>,
    pub(super) path: Option<String>,
    pub(super) bytes_base64: Option<String>,
    pub(super) merge: Option<bool>,
}

pub(super) fn schema_import_pzfx() -> Value {
    obj_schema_with_aliases(
        vec![
            ("xml", nullable_schema(str_schema("Inline XML project string"))),
            ("path", nullable_schema(non_empty_str_schema("Filesystem path to XML project file"))),
            ("bytes_base64", nullable_schema(non_empty_str_schema("Base64-encoded XML project bytes"))),
            ("merge", bool_schema("Merge imported sheets into existing document instead of replacing (default false)")),
        ],
        &[],
        vec![("xml", &["text", "content"]), ("path", &["file_path", "filePath"]), ("bytes_base64", &["bytesBase64"])],
        false,
    )
}

pub(super) fn run_import_pzfx(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportPzfxParams = parse_params(&schema_import_pzfx(), raw)?;
    let report = if let Some(xml) = p.xml.as_deref() {
        check_inline_payload_len(xml.len(), MAX_XML_BYTES, "XML")?;
        import_pzfx_str(xml)?
    } else if let Some(path) = p.path.as_deref() {
        let bytes = ctx.session.path_policy.read_binary_file(path, "XML file", MAX_XML_BYTES)?;
        import_pzfx_bytes(&bytes)?
    } else if let Some(b64) = p.bytes_base64.as_deref() {
        let bytes = decode_base64(b64)?;
        import_pzfx_bytes(&bytes)?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPzfx requires `path`, `xml`/`text`, or `bytes_base64`")
            .with_remediation("Provide an XML `path`, inline `xml` string, or `bytes_base64`."));
    };

    let doc = &mut ctx.session.doc;
    if p.merge.unwrap_or(false) {
        merge_imported_document(doc, report.document)?;
    } else {
        validate_project_cells(0, report.document.total_allocated_cells()?)?;
        *doc = report.document;
    }
    sync_info_linked_data_sheets(doc, &HashSet::new());
    Ok(CommandOutcome::mutated(
        "table.importPzfx",
        "imported XML project",
        json!({
            "title": doc.title,
            "sheet_count": doc.total_sheet_count(),
            "warnings": report.warnings,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 18-21. MultipleVariables Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum PlateGeometryInput {
    Count(u64),
    Name(String),
}

impl PlateGeometryInput {
    pub(super) fn resolve(&self) -> Result<PlateGeometry, EngineError> {
        match self {
            Self::Count(96) => Ok(PlateGeometry::Wells96),
            Self::Count(384) => Ok(PlateGeometry::Wells384),
            Self::Count(n) => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported plate well count `{n}` (expected 96 or 384)"))),
            Self::Name(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "wells96" | "wells_96" | "96" | "96-well" | "96well" => Ok(PlateGeometry::Wells96),
                "wells384" | "wells_384" | "384" | "384-well" | "384well" => Ok(PlateGeometry::Wells384),
                _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported plate geometry `{raw}`"))),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum PlateValuesInput {
    Flat(Vec<Option<f64>>),
    Grid(Vec<Vec<Option<f64>>>),
}

impl PlateValuesInput {
    pub(super) fn into_flat_checked(self, geometry: PlateGeometry) -> Result<Vec<Option<f64>>, EngineError> {
        let flat = match self {
            Self::Flat(v) => {
                let expected = geometry.well_count();
                if v.len() != expected {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("flat plate `values` length ({}) does not match {expected} wells for {geometry:?}", v.len()),
                    ));
                }
                v
            }
            Self::Grid(rows) => {
                let expected_rows = geometry.rows();
                let expected_cols = geometry.cols();
                if rows.len() != expected_rows {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("2D plate `values` row count ({}) does not match {expected_rows}x{expected_cols} plate geometry", rows.len()),
                    ));
                }
                let mut flat = Vec::with_capacity(expected_rows * expected_cols);
                for (row_idx, row) in rows.into_iter().enumerate() {
                    if row.len() != expected_cols {
                        return Err(EngineError::with_code(
                            ErrorCode::BadInput,
                            format!(
                                "2D plate `values` row {row_idx} has {} column(s), expected {expected_cols} for {expected_rows}x{expected_cols} plate geometry",
                                row.len()
                            ),
                        ));
                    }
                    flat.extend(row);
                }
                flat
            }
        };
        for item in &flat {
            if let Some(num) = item
                && !num.is_finite()
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "plate well values cannot be NaN or infinite"));
            }
        }
        Ok(flat)
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ImportPlateMapParams {
    pub(super) sheet_id: Option<String>,
    pub(super) title: Option<String>,
    pub(super) geometry: Option<PlateGeometryInput>,
    pub(super) values: PlateValuesInput,
    pub(super) template: Option<Value>,
    pub(super) roles: Option<Vec<Value>>,
    pub(super) template_name: Option<String>,
    pub(super) x_title: Option<String>,
    pub(super) target_kind: Option<String>,
    pub(super) subtract_blank_mean: Option<bool>,
}

pub(super) fn schema_import_plate_map() -> Value {
    let geom_schema = nullable_schema(json!({
        "description": "Plate geometry: 96, 384, `Wells96`, or `Wells384`",
        "anyOf": [
            { "type": "integer", "enum": [96, 384] },
            non_empty_str_schema("Plate geometry string")
        ]
    }));
    let values_schema = json!({
        "description": "Flat (96/384) or 2D (8x12 / 16x24) plate readings array of finite numbers or nulls",
        "anyOf": [
            array_schema("1D array of well readings", nullable_schema(num_schema("Well reading", None, None)), None, None),
            array_schema(
                "2D plate row/col readings",
                array_schema("Row of well readings", nullable_schema(num_schema("Well reading", None, None)), None, None),
                None,
                None,
            )
        ]
    });
    obj_schema_with_aliases(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional target sheet ID"))),
            ("title", nullable_schema(str_schema("Imported sheet title (default `Plate Data`)"))),
            ("geometry", geom_schema),
            ("values", values_schema),
            ("template", nullable_schema(any_json_schema("PlateMapTemplate object or `.cbtemplate` JSON string"))),
            ("roles", nullable_schema(array_schema("Array of per-well role definitions", any_json_schema("PlateWellRole"), None, None))),
            ("template_name", nullable_schema(str_schema("Template name when building from `roles`"))),
            ("x_title", nullable_schema(str_schema("X/concentration axis title (default `Concentration`)"))),
            ("target_kind", nullable_schema(str_schema("Target archetype (Grouped, XY, Column, MultipleVariables)"))),
            ("subtract_blank_mean", bool_schema("Subtract mean of Blank wells from Treatment readings")),
        ],
        &["values"],
        vec![
            ("sheet_id", &["sheetId", "id"]),
            ("title", &["name"]),
            ("geometry", &["plate_geometry", "plateGeometry"]),
            ("values", &["readings", "well_values"]),
            ("template", &["plate_template", "plateTemplate"]),
            ("roles", &["wells"]),
            ("template_name", &["templateName"]),
            ("x_title", &["xTitle"]),
            ("target_kind", &["targetKind", "kind"]),
            ("subtract_blank_mean", &["subtractBlankMean"]),
        ],
        false,
    )
}

pub(super) fn parse_plate_well_role(val: &Value) -> Result<PlateWellRole, EngineError> {
    if let Some(tag) = val.as_str() {
        return match tag.trim().to_ascii_lowercase().as_str() {
            "blank" => Ok(PlateWellRole::Blank),
            "positivecontrol" | "positive_control" | "positive-control" | "pos_ctrl" | "positive" => Ok(PlateWellRole::PositiveControl),
            "excluded" | "unused" | "skip" => Ok(PlateWellRole::Excluded),
            _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown plate well role `{tag}`"))),
        };
    }
    if let Ok(role) = serde_json::from_value::<PlateWellRole>(val.clone()) {
        return Ok(role);
    }
    #[derive(Deserialize)]
    struct RoleObj {
        #[serde(alias = "type", alias = "kind")]
        role: Option<String>,
        #[serde(alias = "groupName", alias = "group")]
        group_name: Option<String>,
        #[serde(alias = "dilutionX", alias = "x", alias = "concentration")]
        dilution_x: Option<f64>,
        #[serde(alias = "rep")]
        replicate: Option<usize>,
    }
    record_deprecated_object_aliases(
        val,
        &[
            ("type", "role"),
            ("kind", "role"),
            ("groupName", "group_name"),
            ("group", "group_name"),
            ("dilutionX", "dilution_x"),
            ("x", "dilution_x"),
            ("concentration", "dilution_x"),
            ("rep", "replicate"),
        ],
    );
    let obj: RoleObj =
        serde_json::from_value(val.clone()).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "plate well role must be a string or object"))?;
    if let Some(kind) = obj.role.as_deref() {
        match kind.trim().to_ascii_lowercase().as_str() {
            "blank" => return Ok(PlateWellRole::Blank),
            "positivecontrol" | "positive_control" | "positive-control" | "pos_ctrl" | "positive" => {
                return Ok(PlateWellRole::PositiveControl);
            }
            "excluded" | "unused" | "skip" => return Ok(PlateWellRole::Excluded),
            "treatment" => {}
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown plate well role `{kind}`"))),
        }
    }
    let group_name = obj.group_name.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "treatment well role requires `group_name`"))?;
    let dilution_x = obj
        .dilution_x
        .filter(|x| x.is_finite())
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "treatment well role requires finite `dilution_x`"))?;
    Ok(PlateWellRole::Treatment { group_name, dilution_x, replicate: obj.replicate.unwrap_or(1) })
}

pub(super) fn parse_plate_template_obj(val: &Value, default_name: &str) -> Result<PlateMapTemplate, EngineError> {
    if let Ok(tpl) = serde_json::from_value::<PlateMapTemplate>(val.clone()) {
        tpl.validate()?;
        return Ok(tpl);
    }
    #[derive(Deserialize)]
    struct TplObj {
        #[serde(alias = "plate_geometry", alias = "plateGeometry")]
        geometry: PlateGeometryInput,
        #[serde(alias = "title")]
        name: Option<String>,
        #[serde(alias = "xTitle")]
        x_title: Option<String>,
        #[serde(alias = "roles")]
        wells: Vec<Value>,
    }
    record_deprecated_object_aliases(
        val,
        &[("plate_geometry", "geometry"), ("plateGeometry", "geometry"), ("title", "name"), ("xTitle", "x_title"), ("roles", "wells")],
    );
    let obj: TplObj = serde_json::from_value(val.clone())
        .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid plate `template` object: {err}")))?;
    let geometry = obj.geometry.resolve()?;
    let wells: Vec<PlateWellRole> = obj.wells.iter().map(parse_plate_well_role).collect::<Result<_, _>>()?;
    PlateMapTemplate::from_wells(obj.name.as_deref().unwrap_or(default_name), geometry, obj.x_title.as_deref().unwrap_or("Concentration"), wells)
        .map_err(EngineError::from)
}

pub(super) fn resolve_plate_template(p: &ImportPlateMapParams, default_name: &str) -> Result<(PlateGeometry, PlateMapTemplate), EngineError> {
    let geom_override = p.geometry.as_ref().map(PlateGeometryInput::resolve).transpose()?;
    if let Some(tpl_val) = &p.template {
        let tpl = if let Some(json_str) = tpl_val.as_str() {
            PlateMapTemplate::from_cbtemplate_json(json_str)?
        } else {
            parse_plate_template_obj(tpl_val, default_name)?
        };
        let geometry = geom_override.unwrap_or(tpl.geometry);
        return Ok((geometry, tpl));
    }
    let geometry = geom_override
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap requires `geometry` when `template` is omitted"))?;
    let roles_arr =
        p.roles.as_ref().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap requires `template` or `roles`"))?;
    let wells: Vec<PlateWellRole> = roles_arr.iter().map(parse_plate_well_role).collect::<Result<_, _>>()?;
    let tpl_name = p.template_name.as_deref().unwrap_or(default_name);
    let x_title = p.x_title.as_deref().unwrap_or("Concentration");
    let tpl = PlateMapTemplate::from_wells(tpl_name, geometry, x_title, wells)?;
    Ok((geometry, tpl))
}

pub(super) fn assign_and_add_imported_sheet(
    doc: &mut ProjectDocument,
    mut sheet: DataSheet,
    explicit_sheet_id: Option<&str>,
) -> Result<(SheetId, String, usize, usize), EngineError> {
    if let Some(raw_id) = explicit_sheet_id {
        sheet.id = parse_sheet_id(raw_id)?;
    } else {
        sheet.id = next_unique_sheet_id_excluding(doc, sheet.id.as_str(), &[])?;
    }
    let sheet_id = sheet.id.clone();
    let title = sheet.title.clone();
    let rows = sheet.archetype.row_count();
    let cols = sheet.archetype.column_count();
    validate_project_cell_budget_after_change(doc, None, sheet.archetype.allocated_cell_slots())?;
    doc.add_data_sheet(sheet)?;
    Ok((sheet_id, title, rows, cols))
}

pub(super) fn run_import_plate_map(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportPlateMapParams = parse_params(&schema_import_plate_map(), raw)?;
    let title = p.title.as_deref().unwrap_or("Plate Data");
    validate_title(title)?;
    let (geometry, template) = resolve_plate_template(&p, title)?;
    let values = p.values.into_flat_checked(geometry)?;
    let target_kind = match p.target_kind.as_deref() {
        Some(raw_kind) => parse_target_archetype_kind(raw_kind)?,
        None => TargetArchetypeKind::Grouped,
    };
    let subtract_blank_mean = p.subtract_blank_mean.unwrap_or(false);
    let imported = map_plate_to_datasheet(title, geometry, &values, &template, target_kind, subtract_blank_mean)?;
    let doc = &mut ctx.session.doc;
    let (sheet_id, sheet_title, rows, cols) = assign_and_add_imported_sheet(doc, imported, p.sheet_id.as_deref())?;

    Ok(CommandOutcome::mutated(
        "table.importPlateMap",
        format!("imported plate map into `{}`", sheet_id.as_str()),
        json!({
            "sheet_id": sheet_id.as_str(),
            "title": sheet_title,
            "geometry": geometry,
            "target_kind": target_kind,
            "subtract_blank_mean": subtract_blank_mean,
            "rows": rows,
            "cols": cols,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 25. table.importTidyLong
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum TidyColRef {
    Index(usize),
    Name(String),
}

impl TidyColRef {
    pub(super) fn resolve(&self, headers: &[String], key_label: &str) -> Result<usize, EngineError> {
        match self {
            Self::Index(idx) => Ok(*idx),
            Self::Name(name) => {
                if let Some(pos) = headers.iter().position(|h| h == name) {
                    return Ok(pos);
                }
                let needle = name.trim().to_ascii_lowercase();
                headers
                    .iter()
                    .position(|h| h.trim().to_ascii_lowercase() == needle)
                    .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("column header `{name}` for `{key_label}` not found")))
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct TidyMappingInput {
    #[serde(alias = "xCol")]
    pub(super) x_col: Option<TidyColRef>,
    #[serde(alias = "groupCol")]
    pub(super) group_col: Option<TidyColRef>,
    #[serde(alias = "replicateCol")]
    pub(super) replicate_col: Option<TidyColRef>,
    #[serde(alias = "valueCol")]
    pub(super) value_col: Option<TidyColRef>,
    #[serde(alias = "excludedCol")]
    pub(super) excluded_col: Option<TidyColRef>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ImportTidyLongParams {
    pub(super) sheet_id: Option<String>,
    pub(super) title: Option<String>,
    pub(super) target_kind: Option<String>,
    pub(super) headers: Option<Vec<String>>,
    pub(super) rows: Option<Vec<Vec<Value>>>,
    pub(super) csv: Option<String>,
    pub(super) path: Option<String>,
    pub(super) delimiter: Option<String>,
    pub(super) decimal_sep: Option<String>,
    pub(super) normalize_decimal: Option<bool>,
    pub(super) mapping: Option<TidyMappingInput>,
    #[serde(flatten)]
    pub(super) flat_mapping: TidyMappingInput,
}

pub(super) fn schema_import_tidy_long() -> Value {
    let tidy_col_schema = nullable_schema(col_ref_schema("Zero-based header index or header name"));
    let mapping_schema = obj_schema_with_aliases(
        vec![
            ("x_col", tidy_col_schema.clone()),
            ("group_col", tidy_col_schema.clone()),
            ("replicate_col", tidy_col_schema.clone()),
            ("value_col", tidy_col_schema.clone()),
            ("excluded_col", tidy_col_schema.clone()),
        ],
        &[],
        vec![
            ("x_col", &["xCol"]),
            ("group_col", &["groupCol"]),
            ("replicate_col", &["replicateCol"]),
            ("value_col", &["valueCol"]),
            ("excluded_col", &["excludedCol"]),
        ],
        false,
    );
    obj_schema_with_aliases(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional target sheet ID"))),
            ("title", nullable_schema(str_schema("Imported sheet title (default `Tidy Import`)"))),
            ("target_kind", nullable_schema(str_schema("Target archetype (Grouped, Column, XY, MultipleVariables)"))),
            ("headers", nullable_schema(array_schema("Column header names when using `rows`", str_schema("Header name"), None, None))),
            (
                "rows",
                nullable_schema(array_schema(
                    "2D tidy record rows",
                    array_schema("Record row", any_json_schema("Scalar cell"), None, None),
                    None,
                    None,
                )),
            ),
            ("csv", nullable_schema(str_schema("Inline delimited tidy table text"))),
            ("path", nullable_schema(non_empty_str_schema("Filesystem path to delimited tidy file"))),
            ("delimiter", delimiter_schema()),
            ("decimal_sep", decimal_sep_schema()),
            ("normalize_decimal", bool_schema("Infer European decimal separator when ambiguous")),
            ("mapping", nullable_schema(mapping_schema)),
            ("x_col", tidy_col_schema.clone()),
            ("group_col", tidy_col_schema.clone()),
            ("replicate_col", tidy_col_schema.clone()),
            ("value_col", tidy_col_schema.clone()),
            ("excluded_col", tidy_col_schema),
        ],
        &[],
        vec![
            ("sheet_id", &["sheetId", "id"]),
            ("title", &["name"]),
            ("target_kind", &["targetKind", "kind"]),
            ("csv", &["text", "tsv", "source"]),
            ("path", &["file_path", "filePath"]),
            ("decimal_sep", &["decimalSep", "decimal_separator", "decimalSeparator"]),
            ("normalize_decimal", &["normalizeDecimal"]),
            ("mapping", &["column_mapping", "columnMapping"]),
            ("x_col", &["xCol"]),
            ("group_col", &["groupCol"]),
            ("replicate_col", &["replicateCol"]),
            ("value_col", &["valueCol"]),
            ("excluded_col", &["excludedCol"]),
        ],
        false,
    )
}

pub(super) fn json_scalar_to_string(val: &Value) -> Result<String, EngineError> {
    if let Some(s) = val.as_str() {
        return Ok(s.to_owned());
    }
    if val.is_number() {
        return Ok(val.to_string());
    }
    if let Some(b) = val.as_bool() {
        return Ok(b.to_string());
    }
    if val.is_null() {
        return Ok(String::new());
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "tidy table cell values must be JSON scalars"))
}

pub(super) fn parse_explicit_tidy_decimal(raw: Option<&str>) -> Result<Option<char>, EngineError> {
    let Some(s) = raw else {
        return Ok(None);
    };
    match s.trim() {
        "." | "dot" | "period" => Ok(Some('.')),
        "," | "comma" => Ok(Some(',')),
        "auto" => Ok(None),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported decimal separator `{s}` (expected `.` or `,`)"))),
    }
}

pub(super) struct TidyInputData {
    pub(super) headers: Vec<String>,
    pub(super) rows: Vec<Vec<String>>,
    pub(super) decimal_sep: char,
}

pub(super) fn extract_tidy_data(p: &ImportTidyLongParams, policy: &ExternalPathPolicy) -> Result<TidyInputData, EngineError> {
    let explicit_dec = parse_explicit_tidy_decimal(p.decimal_sep.as_deref())?;
    if let Some(rows_arr) = &p.rows {
        let mut rows = Vec::with_capacity(rows_arr.len());
        for row_val in rows_arr {
            let parsed_row: Vec<String> = row_val.iter().map(json_scalar_to_string).collect::<Result<_, _>>()?;
            rows.push(parsed_row);
        }
        let headers = if let Some(h) = &p.headers {
            h.clone()
        } else if let Some(first_row) = rows.first() {
            (0..first_row.len()).map(|i| format!("col_{i}")).collect()
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "table.importTidyLong requires `headers` when `rows` is empty"));
        };
        return Ok(TidyInputData { headers, rows, decimal_sep: explicit_dec.unwrap_or('.') });
    }

    let text = if let Some(inline) = &p.csv {
        check_inline_payload_len(inline.len(), MAX_CSV_FILE_BYTES, "CSV")?;
        inline.clone()
    } else if let Some(path) = &p.path {
        policy.read_utf8_file(path, "tidy file", MAX_CSV_FILE_BYTES)?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importTidyLong requires `rows` or `csv`")
            .with_remediation("Supply `rows`, inline `csv` text, or a file `path`."));
    };

    let delimiter = resolve_delimiter(p.delimiter.as_deref(), &text)?;
    let decimal_sep = match explicit_dec {
        Some(c) => c,
        None => resolve_decimal_char(p.decimal_sep.as_deref(), p.normalize_decimal.unwrap_or(false), &text, delimiter)?,
    };
    let opts = CsvOptions {
        delimiter,
        quote_char: '"',
        decimal_separator: decimal_sep,
        thousands_separator: None,
        header_rows: 1,
        has_row_titles: false,
        parse_asterisk_exclusions: false,
        missing_sentinels: Vec::new(),
    };
    let mut records = parse_delimited_records(&text, &opts)?;
    if records.len() < 2 {
        return Err(EngineError::with_code(ErrorCode::InsufficientData, "tidy CSV input must contain a header row and at least one data row"));
    }
    let headers = records.remove(0);
    Ok(TidyInputData { headers, rows: records, decimal_sep })
}

pub(super) fn resolve_tidy_mapping(p: &ImportTidyLongParams, headers: &[String]) -> Result<TidyColumnMapping, EngineError> {
    let m = p.mapping.as_ref().unwrap_or(&p.flat_mapping);
    let group_col = m
        .group_col
        .as_ref()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required tidy column `group_col`"))?
        .resolve(headers, "group_col")?;
    let value_col = m
        .value_col
        .as_ref()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required tidy column `value_col`"))?
        .resolve(headers, "value_col")?;
    Ok(TidyColumnMapping {
        x_col: m.x_col.as_ref().map(|c| c.resolve(headers, "x_col")).transpose()?,
        group_col,
        replicate_col: m.replicate_col.as_ref().map(|c| c.resolve(headers, "replicate_col")).transpose()?,
        value_col,
        excluded_col: m.excluded_col.as_ref().map(|c| c.resolve(headers, "excluded_col")).transpose()?,
    })
}

pub(super) fn run_import_tidy_long(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportTidyLongParams = parse_params(&schema_import_tidy_long(), raw)?;
    let title = p.title.as_deref().unwrap_or("Tidy Import");
    validate_title(title)?;
    let target_kind = match p.target_kind.as_deref() {
        Some(raw_kind) => parse_target_archetype_kind(raw_kind)?,
        None => TargetArchetypeKind::Grouped,
    };
    let tidy = extract_tidy_data(&p, &ctx.session.path_policy)?;
    let mapping = resolve_tidy_mapping(&p, &tidy.headers)?;

    let imported = pivot_tidy_long_to_datasheet(title, &tidy.headers, &tidy.rows, &mapping, target_kind, tidy.decimal_sep)?;
    let doc = &mut ctx.session.doc;
    let (sheet_id, sheet_title, sheet_rows, sheet_cols) = assign_and_add_imported_sheet(doc, imported, p.sheet_id.as_deref())?;

    Ok(CommandOutcome::mutated(
        "table.importTidyLong",
        format!("imported tidy long table into `{}`", sheet_id.as_str()),
        json!({
            "sheet_id": sheet_id.as_str(),
            "title": sheet_title,
            "target_kind": target_kind,
            "rows": sheet_rows,
            "cols": sheet_cols,
            "mapping": mapping,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 26–28. table.randomizeGroups, table.randomSampleRows, table.simulateDistributions
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::Session;
    use crate::cmd::Caller;

    #[test]
    fn sec12_03_import_prism_rejects_oversized_inline_bytes() {
        let mut session = Session::new();
        let cancelled = AtomicBool::new(false);
        let mut ctx = Ctx { session: &mut session, caller: Caller::Cli, cancelled: &cancelled };
        let params = ImportPrismParams { path: None, bytes_base64: None, bytes: Some(vec![0u8; MAX_ARCHIVE_FILE_BYTES + 1]), merge: None };
        let res = import_prism_with_params(&mut ctx, params);
        assert!(matches!(res, Err(err) if err.code == ErrorCode::ResourceLimit));
    }
}
