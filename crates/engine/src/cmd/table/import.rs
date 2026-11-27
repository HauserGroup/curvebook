//! Data-import commands: the import-link pipeline shared by `table.pasteLink` and
//! `table.importCsv`, plus `table.importPrism`, `table.importPzfx`, `table.importPlateMap`, and
//! `table.importTidyLong`. Command specs stay in the parent `table` module.

use std::collections::HashSet;

use curvebook_doc::{
    AnalysisKind, CellCoord, DataSheet, DocError, FloatingNote, ImportLinkSpec, InfoSheet, InternalLinkKind, LinkSource, MAX_SHEETS_PER_PROJECT,
    PlacementMode, ProjectDocument, RectPt, ResultSheet, SheetCellRange, SheetId, SheetOrigin, SheetSection, SubcolumnMode, TableArchetype,
    VariableSource, validate_project_cells, validate_title,
};
use curvebook_format::registry::FormatRegistry;
use curvebook_format::{
    CsvOptions, MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES, MAX_XML_BYTES, XlsxImportOptions, import_csv_sheets_with_filter, import_csv_to_datasheet,
    import_prism_bytes, import_pzfx_bytes, import_pzfx_str, import_xlsx_bytes_with_options, list_xlsx_sheets, parse_cbplate_json,
    parse_delimited_records,
};
use curvebook_transform::{
    ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, PlateGeometry, PlateGridBuildOptions, PlateKineticReducer, PlateMapTemplate,
    PlateSourceExtraction, PlateWellMatchMode, PlateWellRole, TargetArchetypeKind, TidyColumnMapping, apply_import_link_pipeline,
    build_plate_template_from_grids, extract_plate_grid_from_datasheet, extract_plate_values_from_analyzable_sheet,
    extract_plate_x_grid_from_datasheet, map_plate_to_datasheet_with_origins, pivot_tidy_long_to_datasheet,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{CommandOutcome, EngineError, ErrorCode, ExternalPathPolicy, dag};

use super::{
    BaselineInput, CellRangeInput, ColumnEqualsFilterSchema, ImportFilterSchema, PlacementFields, ensure_mv_edit_preserves_calculated,
    ensure_range_outside_linked_regions, parse_import_filter_val, parse_table_kind, parse_target_archetype_kind, resolve_baseline_adjustment,
    resolve_decimal_char, resolve_delimiter, resolve_subcolumn_mode, validate_project_cell_budget, validate_project_cell_budget_after_change,
};
use crate::cmd::Ctx;
use crate::cmd::helpers::{
    column_subcolumn_count, decode_base64, ensure_sheet_not_frozen, locate_target_and_info_indices, next_unique_sheet_id,
    next_unique_sheet_id_excluding, parse_sheet_id, prune_shortened_series_overrides, require_analyzable_data_sheet, require_data_sheet,
    select_result_tab, shift_point_overrides_on_insert_rows, snapshot_active_series_lengths, sync_info_linked_data_sheets,
};
use crate::cmd::params::{command_schema, deserialize_strict, parse_params};
use crate::cmd::project::{
    collect_all_sheet_ids, extract_source_label, merge_imported_document, next_collision_free_sheet_id, recalculate_dirty_with_links,
    resolve_optional_merged_at, stamp_document_origin,
};

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
    Data { sheet: Box<DataSheet>, propagate_titles: bool },
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
        LinkSource::InternalSheet { sheet_id, kind, tab_id, .. } => Ok(dag::resolve_internal_link_source(doc, sheet_id, *kind, tab_id.as_deref())
            .map(|resolved| match resolved {
                dag::ResolvedInternalLinkSource::Data { sheet, propagate_titles } => {
                    OwnedSourcePayload::Data { sheet: Box::new(sheet.clone()), propagate_titles }
                }
                dag::ResolvedInternalLinkSource::Result(rs) => OwnedSourcePayload::Result(Box::new(rs.clone())),
            })),
        LinkSource::ExternalDelimitedFile { path, delimiter, decimal_separator } => {
            let text = policy.read_delimited_text_file(path, "linked file", MAX_CSV_FILE_BYTES)?;
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
        OwnedSourcePayload::Data { sheet: src, .. } => {
            apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalData(src), opts, info_sheet)?
        }
        OwnedSourcePayload::Result(src) => apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalResult(src), opts, info_sheet)?,
        OwnedSourcePayload::Delimited { text, delimiter, decimal_separator } => {
            let src = ImportSourcePayload::DelimitedText { text, delimiter: *delimiter, decimal_separator: *decimal_separator };
            apply_import_link_pipeline(target_ds, spec, src, opts, info_sheet)?
        }
    };
    target_ds.prune_stale_exclusion_notes();
    target_ds.prune_stale_plate_well_origins();
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
    if let OwnedSourcePayload::Data { sheet: src, propagate_titles: true } = payload {
        dag::propagate_internal_source_titles(target_ds, spec, report.target_range.start.col, src);
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Shared Deserialization & Parsing Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct PasteLinkParams {
    /// Destination data sheet identifier.
    #[schemars(length(min = 1))]
    pub(super) target_sheet_id: String,
    /// Direct ImportLinkSpec payload.
    #[schemars(with = "Option<Map<String, Value>>")]
    pub(super) link_spec: Option<Value>,
    /// Internal source Data or Result sheet ID.
    #[schemars(length(min = 1))]
    pub(super) source_sheet_id: Option<String>,
    /// Explicit source kind: data, result, analyzable_result, terminal_result.
    pub(super) source_kind: Option<String>,
    /// Result sheet tab ID or title.
    pub(super) tab_id: Option<String>,
    /// Optional source range.
    pub(super) source_range: Option<CellRangeInput>,
    /// External delimited file path.
    #[schemars(length(min = 1))]
    pub(super) source_path: Option<String>,
    /// Column delimiter (`auto`, `,`, `\\t`/`tab`, `;`, or single character).
    pub(super) delimiter: Option<String>,
    /// Decimal separator (`.`, `,`, `dot`, `comma`, `period`, or `auto`).
    pub(super) decimal_separator: Option<String>,
    /// Infer European decimal separator when ambiguous.
    pub(super) normalize_decimal: Option<bool>,
    /// Optional import filter specification.
    #[schemars(with = "Option<ImportFilterSchema>")]
    pub(super) filter: Option<Value>,
    /// Optional baseline adjustment.
    pub(super) baseline: Option<BaselineInput>,
    /// Optional predicate excluding matching linked rows.
    pub(super) exclude_row_predicate: Option<String>,
    /// Custom link identifier.
    pub(super) link_id: Option<String>,
    /// Custom provenance note recorded in linked InfoSheet.
    pub(super) provenance: Option<String>,
    #[serde(flatten)]
    pub(super) placement: PlacementFields,
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
    let resolved = dag::resolve_internal_link_source(doc, src_id, kind, tab_id).ok_or_else(|| {
        let label = match kind {
            InternalLinkKind::DataSheet => "source data sheet",
            InternalLinkKind::AnalyzableResultSheet | InternalLinkKind::TerminalResultSheet => "source result sheet",
        };
        EngineError::with_code(ErrorCode::NotFound, format!("{label} `{src_id}` not found"))
    })?;
    match resolved {
        dag::ResolvedInternalLinkSource::Data { sheet: ds, .. } => {
            let max_r = ds.archetype.row_count().saturating_sub(1);
            let max_c = ds.archetype.column_count().saturating_sub(1);
            let max_s = (0..ds.archetype.column_count()).map(|c| column_subcolumn_count(&ds.archetype, c)).max().unwrap_or(1).saturating_sub(1);
            Ok(SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(max_r, max_c, max_s)))
        }
        dag::ResolvedInternalLinkSource::Result(res) => {
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
        let preview_text = policy.read_delimited_text_file(path, "external link file", MAX_CSV_FILE_BYTES)?;
        let delimiter = resolve_delimiter(p.delimiter.as_deref(), &preview_text)?;
        let decimal_separator = resolve_decimal_char(p.decimal_separator.as_deref(), p.normalize_decimal.unwrap_or(false), &preview_text, delimiter)?;
        let mut src = LinkSource::ExternalDelimitedFile { path: path.to_owned(), delimiter, decimal_separator };
        policy.normalize_stored_link_source(&mut src)?;
        src
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.pasteLink requires `link_spec`, `source_sheet_id`, or `source_path`")
            .with_remediation("Specify an internal `source_sheet_id` or an external `source_path`."));
    };

    Ok(ImportLinkSpec { source, filter, placement })
}

pub(super) fn run_paste_link(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: PasteLinkParams = parse_params(&command_schema::<PasteLinkParams>(), raw)?;
    let target_id = parse_sheet_id(&p.target_sheet_id)?;
    let policy = ctx.session.path_policy.clone();
    let doc = &mut ctx.session.doc;

    let (info_id, existing_rows_before) = {
        let target_ds = require_data_sheet(doc, &target_id)?;
        ensure_sheet_not_frozen(target_ds)?;
        (target_ds.info_sheet_id.clone(), target_ds.archetype.row_count())
    };
    let before_active = snapshot_active_series_lengths(doc, &target_id);

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

    if spec.placement.mode == PlacementMode::InsertRows && spec.placement.anchor.row < existing_rows_before && report.rows_placed > 0 {
        shift_point_overrides_on_insert_rows(doc, &target_id, spec.placement.anchor.row, report.rows_placed);
    }

    let mut dirty_seeds = vec![target_id.clone()];
    if info_idx.is_some()
        && let Some(ref iid) = info_id
    {
        dirty_seeds.push(iid.clone());
    }
    let recalc = recalculate_dirty_with_links(doc, &dirty_seeds, None, Some(ctx.cancelled))?;
    prune_shortened_series_overrides(doc, &target_id, &before_active);
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

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ImportCsvParams {
    /// Optional target sheet ID (auto-generated if omitted).
    #[schemars(length(min = 1))]
    pub(super) sheet_id: Option<String>,
    /// Inline CSV/TSV content.
    pub(super) csv: Option<String>,
    /// Filesystem path to CSV/TSV file.
    #[schemars(length(min = 1))]
    pub(super) path: Option<String>,
    /// Column delimiter (`auto`, `,`, `\\t`/`tab`, `;`, or single character).
    pub(super) delimiter: Option<String>,
    /// Decimal separator (`.`, `,`, `dot`, `comma`, `period`, or `auto`).
    pub(super) decimal_separator: Option<String>,
    /// Infer European decimal separator when ambiguous.
    pub(super) normalize_decimal: Option<bool>,
    /// Treat first row as column headers.
    pub(super) has_header: Option<bool>,
    /// Number of header rows to combine into column titles (default 1 when `has_header` is true).
    #[schemars(range(min = 0, max = 16))]
    pub(super) header_rows: Option<usize>,
    /// Treat first column as row titles.
    pub(super) has_row_titles: Option<bool>,
    /// Replicate count per group (default 1).
    #[schemars(range(min = 1))]
    pub(super) replicates: Option<usize>,
    /// Subcolumn mode name or object.
    pub(super) subcolumn_mode: Option<Value>,
    /// Replace existing target sheet instead of placing into region.
    pub(super) replace: Option<bool>,
    /// Title for imported sheet.
    pub(super) title: Option<String>,
    /// Target archetype (XY, Column, Grouped, Contingency, Survival, PartsOfWhole, MultipleVariables, Nested).
    pub(super) archetype: Option<String>,
    /// Optional import filter specification.
    #[schemars(with = "Option<ImportFilterSchema>")]
    pub(super) filter: Option<Value>,
    /// When importing from `path` into an existing sheet, keep a live file link.
    pub(super) link: Option<bool>,
    /// Split data rows into one sheet per distinct trimmed value in this zero-based column (new non-linked imports only; mutually exclusive with `filter.row_match`).
    pub(super) split_by_column: Option<usize>,
    /// Save skipped leading preamble `Key: Value` rows and plain-text notes into one shared linked InfoSheet (default false).
    pub(super) extract_preamble_info: Option<bool>,
    #[serde(flatten)]
    pub(super) placement: PlacementFields,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn import_into_existing_sheet(
    doc: &mut ProjectDocument,
    id: &SheetId,
    p: &ImportCsvParams,
    filter: &curvebook_doc::ImportFilterSpec,
    text: String,
    csv_opts: &CsvOptions,
    subcolumn_mode: SubcolumnMode,
    policy: &ExternalPathPolicy,
) -> Result<(Option<SheetId>, Vec<String>), EngineError> {
    let delimiter = csv_opts.delimiter;
    let decimal_separator = csv_opts.decimal_separator;
    let (info_id, cur_title, cur_kind, existing_rows_before) = {
        let ds = require_data_sheet(doc, id)?;
        ensure_sheet_not_frozen(ds)?;
        (ds.info_sheet_id.clone(), ds.title.clone(), ds.archetype.kind(), ds.archetype.row_count())
    };
    if p.replace.unwrap_or(false) {
        let title = p.title.as_deref().unwrap_or(&cur_title).to_owned();
        let kind = match p.archetype.as_deref() {
            Some(k) => parse_table_kind(k)?,
            None => cur_kind,
        };
        let extract_preamble = p.extract_preamble_info.unwrap_or(false);
        let (mut imported, created_info, warnings) = if p.filter.is_some() || extract_preamble {
            let mut multi = import_csv_sheets_with_filter(id.clone(), title, &text, kind, subcolumn_mode, csv_opts, filter, None, extract_preamble)?;
            let ds = multi.sheets.pop().ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "CSV import returned no sheets"))?;
            let warns: Vec<String> = multi.warnings.iter().map(format_import_warning_text).collect();
            (ds, multi.info_sheet, warns)
        } else {
            let ds = import_csv_to_datasheet(id.clone(), title, &text, kind, subcolumn_mode, csv_opts)?;
            (ds, None, Vec::new())
        };
        validate_project_cell_budget_after_change(doc, Some(id), imported.archetype.allocated_cell_slots())?;
        let new_info_id = if let Some(mut info) = created_info {
            let next_count = doc.total_sheet_count().saturating_add(1);
            if next_count > MAX_SHEETS_PER_PROJECT {
                return Err(EngineError::from(DocError::TooManySheets { count: next_count, max: MAX_SHEETS_PER_PROJECT }));
            }
            let mut reserved = collect_all_sheet_ids(doc);
            if reserved.contains(info.id.as_str()) {
                info.id = next_collision_free_sheet_id(&info.id, &mut reserved)?;
            }
            let added_info_id = info.id.clone();
            info.linked_data_sheets = vec![id.clone()];
            imported.info_sheet_id = Some(added_info_id.clone());
            doc.add_info_sheet(info)?;
            Some(added_info_id)
        } else {
            imported.info_sheet_id = info_id;
            None
        };
        if let Some(slot) = doc.data_sheet_mut(id) {
            *slot = imported;
        }
        if new_info_id.is_some() {
            sync_info_linked_data_sheets(doc, &HashSet::new());
        }
        return Ok((new_info_id, warnings));
    }
    let placement = p.placement.to_placement_spec(false)?;
    let keep_link = p.link.unwrap_or(false);
    let mut source = match p.path.as_deref() {
        Some(path) if keep_link => LinkSource::ExternalDelimitedFile { path: path.to_owned(), delimiter, decimal_separator },
        _ => LinkSource::Clipboard,
    };
    policy.normalize_stored_link_source(&mut source)?;
    let link_spec = ImportLinkSpec { source, filter: filter.clone(), placement };
    let (target_idx, info_idx) = locate_target_and_info_indices(doc, id, info_id.as_ref())?;
    let existing_link_ids: HashSet<String> =
        doc.data_sheets.get(target_idx).map(|d| d.linked_regions.iter().map(|r| r.id.clone()).collect()).unwrap_or_default();
    let csv_link_id = keep_link.then(|| {
        let base = format!("csv-link-{}", id.as_str());
        if !existing_link_ids.contains(&base) {
            base
        } else {
            let mut seq = 2usize;
            loop {
                let candidate = format!("csv-link-{}-{seq}", id.as_str());
                if !existing_link_ids.contains(&candidate) {
                    break candidate;
                }
                seq = seq.saturating_add(1);
            }
        }
    });
    let opts = ImportPipelineOptions {
        link_id: csv_link_id,
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
        if link_before > 0 && report.rows_placed > 0 && report.cols_placed > 0 {
            let link_conflict = ensure_range_outside_linked_regions(prev_ds, report.target_range, "import into").and_then(|()| {
                if matches!(link_spec.placement.mode, PlacementMode::InsertRows) {
                    let shift_span = SheetCellRange::new(
                        CellCoord::new(link_spec.placement.anchor.row, report.target_range.start.col, report.target_range.start.subcol),
                        CellCoord::new(usize::MAX, report.target_range.end.col, report.target_range.end.subcol),
                    );
                    ensure_range_outside_linked_regions(prev_ds, shift_span, "insert rows into")
                } else {
                    Ok(())
                }
            });
            if let Err(err) = link_conflict {
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
    }
    if !keep_link && let Some(target_ds) = doc.data_sheets.get_mut(target_idx) {
        target_ds.linked_regions.truncate(link_before);
    }
    if link_spec.placement.mode == PlacementMode::InsertRows && link_spec.placement.anchor.row < existing_rows_before && report.rows_placed > 0 {
        shift_point_overrides_on_insert_rows(doc, id, link_spec.placement.anchor.row, report.rows_placed);
    }
    Ok((info_idx.and(info_id), Vec::new()))
}

pub(super) fn run_import_csv(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportCsvParams = parse_params(&command_schema::<ImportCsvParams>(), raw)?;
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    let filter = parse_import_filter_val(p.filter.as_ref())?;
    let policy = ctx.session.path_policy.clone();
    let doc = &mut ctx.session.doc;
    let id = match p.sheet_id.as_deref() {
        Some(raw_id) => parse_sheet_id(raw_id)?,
        None => next_unique_sheet_id(doc, "data")?,
    };
    let targets_existing = doc.data_sheet(&id).is_some();
    if p.split_by_column.is_some() {
        if targets_existing || p.link == Some(true) {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                "`split_by_column` cannot be used when targeting an existing sheet or with `link: true`",
            )
            .with_remediation("Use `filter.row_match` to import one matching block into an existing or linked sheet, or target a new unlinked sheet to split all blocks."));
        }
        if filter.row_match.is_some() {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                "cannot combine `split_by_column` and `filter.row_match` in the same `table.importCsv` call",
            )
            .with_remediation("Use `split_by_column` to split into one sheet per block, or `filter.row_match` to keep a single block."));
        }
    }

    let text = if let Some(inline) = &p.csv {
        check_inline_payload_len(inline.len(), MAX_CSV_FILE_BYTES, "CSV")?;
        inline.clone()
    } else if let Some(path) = &p.path {
        policy.read_delimited_text_file(path, "CSV", MAX_CSV_FILE_BYTES)?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importCsv requires `path` or `csv`")
            .with_remediation("Provide a CSV/TSV file `path` or inline `csv` text."));
    };

    let delimiter = resolve_delimiter(p.delimiter.as_deref(), &text)?;
    let decimal_separator = resolve_decimal_char(p.decimal_separator.as_deref(), p.normalize_decimal.unwrap_or(false), &text, delimiter)?;
    let mut csv_opts = build_csv_options(delimiter, decimal_separator);
    let detected = CsvOptions::detect(&text);
    if detected.decimal_separator == decimal_separator {
        csv_opts.thousands_separator = detected.thousands_separator;
    }
    if let Some(header_rows) = p.header_rows {
        csv_opts.header_rows = header_rows;
    } else if let Some(first_row_titles) = p.has_header {
        csv_opts.header_rows = usize::from(first_row_titles);
    }
    if let Some(has_row_titles) = p.has_row_titles {
        csv_opts.has_row_titles = has_row_titles;
    }
    let subcolumn_mode = resolve_subcolumn_mode(p.subcolumn_mode.as_ref(), p.replicates.unwrap_or(1))?;

    let mut dirty_seeds: Vec<SheetId> = Vec::new();
    let mut imported_sheet_ids: Vec<String> = Vec::new();
    let mut created_info_sheet_id: Option<SheetId> = None;
    let mut import_warnings: Vec<String> = Vec::new();
    let (primary_id, primary_title, before_active) = if targets_existing {
        let before = snapshot_active_series_lengths(doc, &id);
        dirty_seeds.push(id.clone());
        let (updated_info, warns) = import_into_existing_sheet(doc, &id, &p, &filter, text, &csv_opts, subcolumn_mode, &policy)?;
        import_warnings.extend(warns);
        if let Some(info_id) = updated_info {
            dirty_seeds.push(info_id.clone());
            if p.replace.unwrap_or(false) {
                created_info_sheet_id = Some(info_id);
            }
        }
        let title = doc.data_sheet(&id).map(|d| d.title.clone()).unwrap_or_default();
        imported_sheet_ids.push(id.as_str().to_owned());
        (id.clone(), title, Some(before))
    } else {
        let title = p.title.as_deref().unwrap_or("Imported CSV");
        let kind = parse_table_kind(p.archetype.as_deref().unwrap_or("xy"))?;
        let mut multi = import_csv_sheets_with_filter(
            id.clone(),
            title,
            &text,
            kind,
            subcolumn_mode,
            &csv_opts,
            &filter,
            p.split_by_column,
            p.extract_preamble_info.unwrap_or(false),
        )?;
        let added_sheet_count = multi.sheets.len().saturating_add(usize::from(multi.info_sheet.is_some()));
        let total_sheets = doc.total_sheet_count().saturating_add(added_sheet_count);
        if total_sheets > MAX_SHEETS_PER_PROJECT {
            return Err(EngineError::from(DocError::TooManySheets { count: total_sheets, max: MAX_SHEETS_PER_PROJECT }));
        }
        let existing_cells = doc.total_allocated_cells()?;
        let added_cells = multi.sheets.iter().try_fold(0_usize, |acc, ds| {
            let slots = ds.archetype.allocated_cell_slots();
            validate_project_cells(acc, slots)?;
            Ok::<usize, EngineError>(acc.saturating_add(slots))
        })?;
        validate_project_cells(existing_cells, added_cells)?;

        let mut reserved = collect_all_sheet_ids(doc);
        let preserve_explicit_single_id = p.split_by_column.is_none() && p.sheet_id.is_some();
        for ds in &mut multi.sheets {
            if !preserve_explicit_single_id {
                if reserved.contains(ds.id.as_str()) {
                    ds.id = next_collision_free_sheet_id(&ds.id, &mut reserved)?;
                } else {
                    reserved.insert(ds.id.as_str().to_owned());
                }
            } else {
                reserved.insert(ds.id.as_str().to_owned());
            }
        }
        let final_sheet_ids: Vec<SheetId> = multi.sheets.iter().map(|s| s.id.clone()).collect();
        if let Some(ref mut info) = multi.info_sheet {
            if reserved.contains(info.id.as_str()) {
                info.id = next_collision_free_sheet_id(&info.id, &mut reserved)?;
            } else {
                reserved.insert(info.id.as_str().to_owned());
            }
            info.linked_data_sheets = final_sheet_ids.clone();
            for ds in &mut multi.sheets {
                ds.info_sheet_id = Some(info.id.clone());
            }
            created_info_sheet_id = Some(info.id.clone());
        }
        let primary = multi
            .sheets
            .first()
            .map(|s| (s.id.clone(), s.title.clone()))
            .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "CSV import produced no data sheets"))?;
        for ds in multi.sheets {
            dirty_seeds.push(ds.id.clone());
            imported_sheet_ids.push(ds.id.as_str().to_owned());
            doc.add_data_sheet(ds)?;
        }
        if let Some(info) = multi.info_sheet {
            dirty_seeds.push(info.id.clone());
            doc.add_info_sheet(info)?;
            sync_info_linked_data_sheets(doc, &HashSet::new());
        }
        import_warnings.extend(multi.warnings.iter().map(format_import_warning_text));
        (primary.0, primary.1, None)
    };

    let recalc = recalculate_dirty_with_links(doc, &dirty_seeds, None, Some(ctx.cancelled))?;
    if let Some(before) = before_active.as_deref() {
        prune_shortened_series_overrides(doc, &primary_id, before);
    }
    let message = match recalc.failure_note() {
        Some(note) => format!("imported CSV into data sheet{note}"),
        None => String::from("imported CSV into data sheet"),
    };
    let mut payload = json!({
        "sheet_id": primary_id.as_str(),
        "title": primary_title,
        "imported_sheet_ids": imported_sheet_ids,
        "recalculated_sheets": recalc.recalculated_sheets,
        "refreshed_links": recalc.refreshed_links,
        "failed_count": recalc.failed_result_sheets.len(),
        "failed_result_sheets": recalc.failed_json(),
    });
    if let Some(ref info_id) = created_info_sheet_id
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert(String::from("info_sheet_id"), json!(info_id.as_str()));
    }
    let mut outcome = CommandOutcome::mutated("table.importCsv", message, payload);
    outcome.warnings = import_warnings;
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// 15. table.exportCsv
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportPrismParams {
    /// Filesystem path to a .prism file.
    #[schemars(length(min = 1))]
    pub(super) path: Option<String>,
    /// Base64-encoded .prism file bytes.
    #[schemars(length(min = 1))]
    pub(super) bytes_base64: Option<String>,
    /// Raw byte array of .prism file contents.
    pub(super) bytes: Option<Vec<u8>>,
    /// Merge imported sheets into existing document instead of replacing (default false).
    pub(super) merge: Option<bool>,
    /// Optional caller-supplied UTC timestamp (`unix:<seconds>` or ISO-8601 UTC) recorded on merged sheet origins.
    pub(super) merged_at: Option<String>,
}

pub(super) fn run_import_prism(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportPrismParams = parse_params(&command_schema::<ImportPrismParams>(), raw)?;
    import_prism_with_params(ctx, p)
}

fn format_import_warning_text(warning: &curvebook_format::ImportWarning) -> String {
    if warning.source.trim().is_empty() {
        format!("[{:?}] {}", warning.kind, warning.message)
    } else {
        format!("[{:?}] {}: {}", warning.kind, warning.source, warning.message)
    }
}

fn finish_project_import_report(
    ctx: &mut Ctx<'_>,
    command_id: &str,
    summary: &str,
    mut report: curvebook_format::ImportReport,
    merge: bool,
    origin: Option<SheetOrigin>,
) -> Result<CommandOutcome, EngineError> {
    let doc = &mut ctx.session.doc;
    if merge {
        if let Some(ref orig) = origin {
            stamp_document_origin(&mut report.document, orig);
        }
        merge_imported_document(doc, report.document)?;
    } else {
        validate_project_cells(0, report.document.total_allocated_cells()?)?;
        *doc = report.document;
    }
    sync_info_linked_data_sheets(doc, &HashSet::new());
    let mut outcome = CommandOutcome::mutated(
        command_id,
        summary,
        json!({
            "title": doc.title,
            "sheet_count": doc.total_sheet_count(),
            "warnings": report.warnings,
        }),
    );
    outcome.warnings = report.warnings.iter().map(format_import_warning_text).collect();
    Ok(outcome)
}

fn import_prism_with_params(ctx: &mut Ctx<'_>, p: ImportPrismParams) -> Result<CommandOutcome, EngineError> {
    let merged_at = resolve_optional_merged_at(p.merged_at.as_deref())?;
    let merge = p.merge.unwrap_or(false);
    let source_path = p.path.clone();
    let bytes = if let Some(path) = p.path.as_deref() {
        ctx.session.path_policy.read_binary_file(path, ".prism file", MAX_ARCHIVE_FILE_BYTES)?
    } else if let Some(b64) = p.bytes_base64.as_deref() {
        decode_base64(b64)?
    } else if let Some(raw_bytes) = p.bytes {
        check_inline_payload_len(raw_bytes.len(), MAX_ARCHIVE_FILE_BYTES, ".prism file")?;
        raw_bytes
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPrism requires `path`, `bytes_base64`, or `bytes`")
            .with_remediation("Supply a .prism `path`, `bytes_base64`, or `bytes`."));
    };

    let report = import_prism_bytes(&bytes)?;
    let origin = if merge {
        let format_id = FormatRegistry::default_registry().importer_by_format("prism").map_or(".prism", |imp| imp.descriptor().id);
        Some(SheetOrigin::new(extract_source_label(source_path.as_deref(), "inline.prism"), format_id, merged_at)?)
    } else {
        None
    };
    let before_result_len = if merge { ctx.session.doc.result_sheets.len() } else { 0 };
    let mut outcome = finish_project_import_report(ctx, "table.importPrism", "imported .prism file", report, merge, origin)?;

    // §6.1: If the imported archive produced any live-translated (non-ImportedSnapshot) analyses,
    // recompute them inside the same single journaled undoable step while preserving untouched
    // `ResultStatus::Imported` snapshots.
    let live_result_ids: Vec<SheetId> = ctx
        .session
        .doc
        .result_sheets
        .iter()
        .skip(before_result_len)
        .filter(|rs| !matches!(rs.analysis.kind, AnalysisKind::ImportedSnapshot { .. }) && !rs.is_imported())
        .map(|rs| rs.id.clone())
        .collect();
    if !live_result_ids.is_empty() {
        let recalc = recalculate_dirty_with_links(&mut ctx.session.doc, &live_result_ids, None, Some(ctx.cancelled))?;
        if let Some(note) = recalc.failure_note() {
            outcome.message = format!("{}{note}", outcome.message);
        }
        if let Some(obj) = outcome.payload.as_object_mut() {
            obj.insert(String::from("recalculated_sheets"), json!(recalc.recalculated_sheets));
            obj.insert(String::from("refreshed_links"), json!(recalc.refreshed_links));
            obj.insert(String::from("failed_count"), json!(recalc.failed_result_sheets.len()));
            obj.insert(String::from("failed_result_sheets"), json!(recalc.failed_json()));
        }
    }
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// 17. table.importPzfx
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportPzfxParams {
    /// Inline .pzfx XML string.
    pub(super) xml: Option<String>,
    /// Filesystem path to a .pzfx file.
    #[schemars(length(min = 1))]
    pub(super) path: Option<String>,
    /// Base64-encoded .pzfx file bytes.
    #[schemars(length(min = 1))]
    pub(super) bytes_base64: Option<String>,
    /// Merge imported sheets into existing document instead of replacing (default false).
    pub(super) merge: Option<bool>,
    /// Optional caller-supplied UTC timestamp (`unix:<seconds>` or ISO-8601 UTC) recorded on merged sheet origins.
    pub(super) merged_at: Option<String>,
}

pub(super) fn run_import_pzfx(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportPzfxParams = parse_params(&command_schema::<ImportPzfxParams>(), raw)?;
    let merged_at = resolve_optional_merged_at(p.merged_at.as_deref())?;
    let merge = p.merge.unwrap_or(false);
    let source_path = p.path.clone();
    let report = if let Some(xml) = p.xml.as_deref() {
        check_inline_payload_len(xml.len(), MAX_XML_BYTES, ".pzfx XML")?;
        import_pzfx_str(xml)?
    } else if let Some(path) = p.path.as_deref() {
        let bytes = ctx.session.path_policy.read_binary_file(path, ".pzfx file", MAX_XML_BYTES)?;
        import_pzfx_bytes(&bytes)?
    } else if let Some(b64) = p.bytes_base64.as_deref() {
        let bytes = decode_base64(b64)?;
        import_pzfx_bytes(&bytes)?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPzfx requires `path`, `xml`, or `bytes_base64`")
            .with_remediation("Provide a .pzfx `path`, inline `xml` string, or `bytes_base64`."));
    };

    let origin = if merge { Some(SheetOrigin::new(extract_source_label(source_path.as_deref(), "inline.pzfx"), "pzfx", merged_at)?) } else { None };
    finish_project_import_report(ctx, "table.importPzfx", "imported .pzfx file", report, merge, origin)
}

// ---------------------------------------------------------------------------
// 17b. table.importXlsx
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportXlsxParams {
    /// Filesystem path to `.xlsx` workbook.
    #[schemars(length(min = 1))]
    pub(super) path: Option<String>,
    /// Base64-encoded `.xlsx` workbook bytes.
    #[schemars(length(min = 1))]
    pub(super) bytes_base64: Option<String>,
    /// Raw byte array of `.xlsx` workbook.
    pub(super) bytes: Option<Vec<u8>>,
    /// Worksheet tab names to import (imports all visible sheets if omitted).
    pub(super) sheets: Option<Vec<String>>,
    /// Zero-based worksheet indices to import.
    pub(super) sheet_indices: Option<Vec<usize>>,
    /// When true, enumerate workbook sheet names/indices without mutating the document.
    pub(super) list_sheets: Option<bool>,
    /// Treat first row(s) of each sheet as column headers (default true).
    pub(super) has_header: Option<bool>,
    /// Number of top header rows to merge into column titles (default 1 when `has_header` is true).
    #[schemars(range(min = 0, max = 16))]
    pub(super) header_rows: Option<usize>,
    /// Treat first column of each sheet as row titles (default false).
    pub(super) has_row_titles: Option<bool>,
    /// Optional target archetype (XY, Column, Grouped, Contingency, Survival, PartsOfWhole, MultipleVariables, Nested); inferred per sheet when omitted.
    pub(super) archetype: Option<String>,
    /// Subcolumn mode name or object.
    pub(super) subcolumn_mode: Option<Value>,
    /// Include hidden worksheets when no explicit `sheets`/`sheet_indices` are supplied.
    pub(super) include_hidden: Option<bool>,
    /// Merge imported worksheets into the current project instead of replacing it (default false).
    pub(super) merge: Option<bool>,
    /// Optional project title override (defaults to the workbook filename stem when omitted).
    pub(super) title: Option<String>,
    /// Optional A1 cell rectangle (e.g. `"B4:M11"`). Requires selecting a single worksheet.
    #[schemars(length(min = 1))]
    pub(super) cell_range: Option<String>,
    /// Leading rows skipped before `header_rows` (relative to `cell_range` when set; default 0).
    pub(super) skip_rows: Option<usize>,
    /// Expand exact OOXML `<mergeCell>` top-left values across body rows before cropping/slicing (default false).
    pub(super) fill_merged_cells: Option<bool>,
    /// Split data rows into one sheet per distinct trimmed value in this zero-based column (mutually exclusive with `row_match`).
    pub(super) split_by_column: Option<usize>,
    /// Retain only data rows matching one trimmed value in a zero-based discriminator column (mutually exclusive with `split_by_column`).
    pub(super) row_match: Option<ColumnEqualsFilterSchema>,
    /// When true and `skip_rows > 0`, store skipped preamble `Key: Value` rows and notes in one shared linked InfoSheet (default false).
    pub(super) extract_preamble_info: Option<bool>,
    /// Optional caller-supplied UTC timestamp (`unix:<seconds>` or ISO-8601 UTC) recorded on merged sheet origins.
    pub(super) merged_at: Option<String>,
}

pub(super) fn run_import_xlsx(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportXlsxParams = parse_params(&command_schema::<ImportXlsxParams>(), raw)?;
    let merged_at = resolve_optional_merged_at(p.merged_at.as_deref())?;
    let source_path = p.path.clone();
    if let Some(ref title) = p.title {
        validate_title(title)?;
        if title.trim().is_empty() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "table.importXlsx `title` must not be empty"));
        }
    }
    let bytes = if let Some(path) = p.path.as_deref() {
        ctx.session.path_policy.read_binary_file(path, "workbook", MAX_ARCHIVE_FILE_BYTES)?
    } else if let Some(b64) = p.bytes_base64.as_deref() {
        decode_base64(b64)?
    } else if let Some(raw_bytes) = p.bytes {
        check_inline_payload_len(raw_bytes.len(), MAX_ARCHIVE_FILE_BYTES, "workbook")?;
        raw_bytes
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importXlsx requires `path`, `bytes_base64`, or `bytes`")
            .with_remediation("Supply a `.xlsx` file `path`, `bytes_base64`, or `bytes`."));
    };

    if p.list_sheets.unwrap_or(false) {
        let sheets = list_xlsx_sheets(&bytes)?;
        return Ok(CommandOutcome::read_only(
            "table.importXlsx",
            format!("listed {} worksheet(s) from .xlsx workbook", sheets.len()),
            json!({
                "sheet_count": sheets.len(),
                "sheets": sheets,
            }),
        ));
    }

    let has_header = p.has_header.unwrap_or(true);
    let header_rows = match p.header_rows {
        Some(n) => n,
        None => usize::from(has_header),
    };
    let table_kind = p.archetype.as_deref().map(parse_table_kind).transpose()?;
    let subcolumn_mode = resolve_subcolumn_mode(p.subcolumn_mode.as_ref(), 1)?;
    let title = p.title.map(|t| t.trim().to_owned()).or_else(|| {
        p.path.as_deref().and_then(|raw_path| {
            std::path::Path::new(raw_path).file_stem().and_then(|s| s.to_str()).filter(|s| !s.trim().is_empty()).map(str::to_owned)
        })
    });

    let opts = XlsxImportOptions {
        sheet_names: p.sheets,
        sheet_indices: p.sheet_indices,
        has_header,
        header_rows,
        has_row_titles: p.has_row_titles.unwrap_or(false),
        table_kind,
        subcolumn_mode,
        include_hidden: p.include_hidden.unwrap_or(false),
        title,
        cell_range: p.cell_range,
        skip_rows: p.skip_rows.unwrap_or(0),
        fill_merged_cells: p.fill_merged_cells.unwrap_or(false),
        split_by_column: p.split_by_column,
        row_match: p.row_match.map(ColumnEqualsFilterSchema::into_doc),
        extract_preamble_info: p.extract_preamble_info.unwrap_or(false),
    };

    let mut report = import_xlsx_bytes_with_options(&bytes, &opts)?;
    let doc = &mut ctx.session.doc;
    let (imported_ids, info_sheet_id): (Vec<String>, Option<String>) = if p.merge.unwrap_or(false) {
        let origin = SheetOrigin::new(extract_source_label(source_path.as_deref(), "inline.xlsx"), "xlsx", merged_at)?;
        stamp_document_origin(&mut report.document, &origin);
        let before_count = doc.data_sheets.len();
        let before_info_count = doc.info_sheets.len();
        merge_imported_document(doc, report.document)?;
        let data_ids = doc.data_sheets.iter().skip(before_count).map(|s| s.id.as_str().to_owned()).collect();
        let info_id = doc.info_sheets.get(before_info_count).map(|s| s.id.as_str().to_owned());
        (data_ids, info_id)
    } else {
        validate_project_cells(0, report.document.total_allocated_cells()?)?;
        let data_ids = report.document.data_sheets.iter().map(|s| s.id.as_str().to_owned()).collect();
        let info_id = report.document.info_sheets.first().map(|s| s.id.as_str().to_owned());
        *doc = report.document;
        (data_ids, info_id)
    };
    sync_info_linked_data_sheets(doc, &HashSet::new());
    let mut payload = json!({
        "title": doc.title,
        "sheet_count": doc.total_sheet_count(),
        "imported_sheet_ids": imported_ids,
        "warnings": report.warnings,
    });
    if let Some(ref info_id) = info_sheet_id
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert(String::from("info_sheet_id"), json!(info_id));
    }
    let mut outcome =
        CommandOutcome::mutated("table.importXlsx", format!("imported {} worksheet(s) from .xlsx workbook", imported_ids.len()), payload);
    outcome.warnings = report.warnings.iter().map(format_import_warning_text).collect();
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// 18-21. MultipleVariables Commands
// ---------------------------------------------------------------------------

fn plate_geometry_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "anyOf": [
            { "type": "integer", "enum": [96, 384] },
            { "type": "string", "minLength": 1 }
        ]
    })
}

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

#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PlateXWindowMeanInput {
    /// Inclusive lower X bound (finite, <= `x_max`).
    pub(super) x_min: f64,
    /// Inclusive upper X bound (finite, >= `x_min`).
    pub(super) x_max: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum PlateReducerInput {
    /// Minimum active numeric reading across each well column.
    Min,
    /// Maximum active numeric reading across each well column.
    Max,
    /// Active numeric reading at zero-based row index.
    RowIndex(usize),
    /// Arithmetic mean of active numeric readings whose X lies in `[x_min, x_max]` (inclusive).
    XWindowMean(PlateXWindowMeanInput),
}

impl PlateReducerInput {
    fn into_transform(self) -> (PlateKineticReducer, String) {
        match self {
            Self::Min => (PlateKineticReducer::Min, String::from("min")),
            Self::Max => (PlateKineticReducer::Max, String::from("max")),
            Self::RowIndex(idx) => (PlateKineticReducer::RowIndex(idx), format!("row_index({idx})")),
            Self::XWindowMean(win) => {
                (PlateKineticReducer::XWindowMean { x_min: win.x_min, x_max: win.x_max }, format!("x_window_mean([{}, {}])", win.x_min, win.x_max))
            }
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PlateSourceInput {
    /// Upstream DataSheet or UpToDate analyzable ResultSheet ID supplying per-well values.
    #[schemars(length(min = 1))]
    pub(super) sheet_id: String,
    /// Column header supplying per-well values when each row is a well (e.g. `"Net Area"`). Mutually exclusive with `reducer`.
    #[schemars(length(min = 1))]
    pub(super) value_column: Option<String>,
    /// Kinetic reducer collapsing each well column into one reading (`"min"`, `"max"`, `{"row_index": n}`, `{"x_window_mean": {"x_min": a, "x_max": b}}`). Mutually exclusive with `value_column`.
    pub(super) reducer: Option<PlateReducerInput>,
    /// Well matching strategy: `"by_well_id"` (default) or `"positional"`.
    pub(super) match_wells: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportPlateMapParams {
    /// Optional target sheet ID.
    #[schemars(length(min = 1))]
    pub(super) sheet_id: Option<String>,
    /// Imported sheet title (default `Plate Data`).
    pub(super) title: Option<String>,
    /// Plate geometry: 96, 384, `Wells96`, or `Wells384`.
    #[serde(default)]
    #[schemars(schema_with = "plate_geometry_schema")]
    pub(super) geometry: Option<PlateGeometryInput>,
    /// Flat (96/384) or 2D (8x12 / 16x24) plate readings array of finite numbers or nulls. Mutually exclusive with `source`.
    pub(super) values: Option<PlateValuesInput>,
    /// Upstream analyzable DataSheet or UpToDate ResultSheet source supplying per-well values. Mutually exclusive with `values`.
    pub(super) source: Option<PlateSourceInput>,
    /// PlateMapTemplate object or `.cbplate` v1 / legacy JSON string. Mutually exclusive with `label_grid` / `layout_sheet_id`.
    pub(super) template: Option<Value>,
    /// Array of per-well role definitions. Mutually exclusive with `label_grid` / `layout_sheet_id`.
    pub(super) roles: Option<Vec<Value>>,
    /// Strict 8x12 or 16x24 literal text label grid (null/empty = Excluded). Mutually exclusive with `template`, `roles`, and `layout_sheet_id`.
    pub(super) label_grid: Option<Vec<Vec<Option<String>>>>,
    /// DataSheet (or UpToDate analyzable ResultSheet) whose data cells form a strict 8x12 or 16x24 label grid. Mutually exclusive with `template`, `roles`, and `label_grid`.
    #[schemars(length(min = 1))]
    pub(super) layout_sheet_id: Option<String>,
    /// Optional strict 8x12 or 16x24 numeric X / concentration grid paired with `label_grid` or `layout_sheet_id`. Mutually exclusive with `x_grid_sheet_id`.
    pub(super) x_grid: Option<Vec<Vec<Option<f64>>>>,
    /// Optional DataSheet (or UpToDate analyzable ResultSheet) whose data cells form a strict 8x12 or 16x24 numeric X grid. Mutually exclusive with `x_grid`.
    #[schemars(length(min = 1))]
    pub(super) x_grid_sheet_id: Option<String>,
    /// Case-insensitive trimmed labels treated as Blank wells when building from `label_grid` / `layout_sheet_id`.
    pub(super) blank_labels: Option<Vec<String>>,
    /// Case-insensitive trimmed labels treated as PositiveControl wells when building from `label_grid` / `layout_sheet_id`.
    pub(super) positive_control_labels: Option<Vec<String>>,
    /// Template name when building from `roles`, `label_grid`, or `layout_sheet_id`.
    pub(super) template_name: Option<String>,
    /// X/concentration axis title (default `Concentration`).
    pub(super) x_title: Option<String>,
    /// Target archetype (Grouped, XY, Column, MultipleVariables).
    pub(super) target_kind: Option<String>,
    /// Subtract mean of Blank wells from Treatment readings.
    pub(super) subtract_blank_mean: Option<bool>,
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
    #[serde(deny_unknown_fields)]
    struct RoleObj {
        role: Option<String>,
        group_name: Option<String>,
        dilution_x: Option<f64>,
        replicate: Option<usize>,
    }
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

fn parse_cbplate_value_to_template(raw: &str) -> Result<PlateMapTemplate, EngineError> {
    let file = parse_cbplate_json(raw)?;
    PlateMapTemplate::from_wells(file.name, file.geometry, file.x_title, file.wells).map_err(EngineError::from)
}

pub(super) fn parse_plate_template_obj(val: &Value, default_name: &str) -> Result<PlateMapTemplate, EngineError> {
    if val.as_object().is_some_and(|obj| obj.contains_key("format_version")) {
        let raw = serde_json::to_string(val)
            .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid plate `template` object: {err}")))?;
        return parse_cbplate_value_to_template(&raw);
    }
    if let Ok(tpl) = serde_json::from_value::<PlateMapTemplate>(val.clone()) {
        tpl.validate()?;
        return Ok(tpl);
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TplObj {
        geometry: PlateGeometryInput,
        name: Option<String>,
        x_title: Option<String>,
        wells: Vec<Value>,
    }
    let obj: TplObj = serde_json::from_value(val.clone())
        .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid plate `template` object: {err}")))?;
    let geometry = obj.geometry.resolve()?;
    let wells: Vec<PlateWellRole> = obj.wells.iter().map(parse_plate_well_role).collect::<Result<_, _>>()?;
    PlateMapTemplate::from_wells(obj.name.as_deref().unwrap_or(default_name), geometry, obj.x_title.as_deref().unwrap_or("Concentration"), wells)
        .map_err(EngineError::from)
}

fn require_fresh_analyzable_data_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a DataSheet, EngineError> {
    if let Some(res) = doc.result_sheet(id)
        && !res.is_computed_up_to_date()
    {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("source result sheet `{id}` has status {:?}; recalculate so it is UpToDate before mapping plate values", res.status),
        )
        .with_remediation("Run `project.recalculate` before invoking `table.importPlateMap` on a ResultSheet.")
        .with_details(json!({ "sheet_id": id.as_str(), "status": format!("{:?}", res.status) })));
    }
    require_analyzable_data_sheet(doc, id)
}

fn parse_plate_match_mode(raw: Option<&str>) -> Result<PlateWellMatchMode, EngineError> {
    let Some(s) = raw else {
        return Ok(PlateWellMatchMode::ByWellId);
    };
    let norm: String = s.trim().chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "bywellid" | "wellid" => Ok(PlateWellMatchMode::ByWellId),
        "positional" | "position" | "sequential" => Ok(PlateWellMatchMode::Positional),
        _ => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unknown `source.match_wells` mode `{s}` (expected `by_well_id` or `positional`)"),
        )),
    }
}

pub(super) fn resolve_plate_template_with_doc(
    doc: &ProjectDocument,
    p: &ImportPlateMapParams,
    default_name: &str,
    target_kind: TargetArchetypeKind,
) -> Result<(PlateGeometry, PlateMapTemplate, Vec<String>), EngineError> {
    let has_legacy = p.template.is_some() || p.roles.is_some();
    let has_grid = p.label_grid.is_some() || p.layout_sheet_id.is_some();
    if has_legacy && has_grid {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "cannot combine `template`/`roles` with `label_grid`/`layout_sheet_id` in `table.importPlateMap`",
        ));
    }
    if p.label_grid.is_some() && p.layout_sheet_id.is_some() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "cannot specify both `label_grid` and `layout_sheet_id`"));
    }
    if p.x_grid.is_some() && p.x_grid_sheet_id.is_some() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "cannot specify both `x_grid` and `x_grid_sheet_id`"));
    }
    let has_grid_aux = p.x_grid.is_some() || p.x_grid_sheet_id.is_some() || p.blank_labels.is_some() || p.positive_control_labels.is_some();
    if has_legacy && has_grid_aux {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "`x_grid`, `x_grid_sheet_id`, `blank_labels`, and `positive_control_labels` can only be used with `label_grid` or `layout_sheet_id`",
        ));
    }

    let geom_override = p.geometry.as_ref().map(PlateGeometryInput::resolve).transpose()?;
    if has_grid {
        let label_grid: Vec<Vec<String>> = if let Some(raw_grid) = &p.label_grid {
            raw_grid.iter().map(|row| row.iter().map(|cell| cell.clone().unwrap_or_default()).collect()).collect()
        } else if let Some(layout_id_raw) = p.layout_sheet_id.as_deref() {
            let layout_id = parse_sheet_id(layout_id_raw)?;
            let layout_sheet = require_fresh_analyzable_data_sheet(doc, &layout_id)?;
            let (_, extracted) = extract_plate_grid_from_datasheet(layout_sheet)?;
            extracted
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "missing `label_grid` or `layout_sheet_id`"));
        };

        let x_grid: Option<Vec<Vec<Option<f64>>>> = if let Some(raw_xg) = &p.x_grid {
            Some(raw_xg.clone())
        } else if let Some(x_id_raw) = p.x_grid_sheet_id.as_deref() {
            let x_id = parse_sheet_id(x_id_raw)?;
            let x_sheet = require_fresh_analyzable_data_sheet(doc, &x_id)?;
            let (_, extracted) = extract_plate_x_grid_from_datasheet(x_sheet)?;
            Some(extracted)
        } else {
            None
        };

        let opts = PlateGridBuildOptions {
            name: p.template_name.as_deref().unwrap_or(default_name).to_owned(),
            x_title: p.x_title.as_deref().unwrap_or("Concentration").to_owned(),
            blank_labels: p.blank_labels.clone().unwrap_or_default(),
            positive_control_labels: p.positive_control_labels.clone().unwrap_or_default(),
            target_kind,
        };
        let (tpl, warnings) = build_plate_template_from_grids(&label_grid, x_grid.as_deref(), &opts)?;
        if let Some(explicit_geom) = geom_override
            && explicit_geom != tpl.geometry
        {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("explicit `geometry` ({explicit_geom:?}) does not match label grid geometry ({:?})", tpl.geometry),
            ));
        }
        return Ok((tpl.geometry, tpl, warnings));
    }

    if let Some(tpl_val) = &p.template {
        let tpl = if let Some(json_str) = tpl_val.as_str() {
            parse_cbplate_value_to_template(json_str)?
        } else {
            parse_plate_template_obj(tpl_val, default_name)?
        };
        let geometry = geom_override.unwrap_or(tpl.geometry);
        return Ok((geometry, tpl, Vec::new()));
    }
    if let Some(roles_arr) = &p.roles {
        let geometry = geom_override
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap requires `geometry` when `template` is omitted"))?;
        let wells: Vec<PlateWellRole> = roles_arr.iter().map(parse_plate_well_role).collect::<Result<_, _>>()?;
        let tpl_name = p.template_name.as_deref().unwrap_or(default_name);
        let x_title = p.x_title.as_deref().unwrap_or("Concentration");
        let tpl = PlateMapTemplate::from_wells(tpl_name, geometry, x_title, wells)?;
        return Ok((geometry, tpl, Vec::new()));
    }

    Err(EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap requires one of `template`, `roles`, `label_grid`, or `layout_sheet_id`"))
}

fn append_plate_source_floating_note(sheet: &mut DataSheet, provenance_line: &str) {
    if let Some(note) = sheet.floating_notes.first_mut() {
        if !note.body_markdown.is_empty() {
            note.body_markdown.push('\n');
        }
        note.body_markdown.push_str(provenance_line);
    } else {
        sheet.floating_notes.push(FloatingNote::new(
            format!("{}-plate-source", sheet.id.as_str()),
            RectPt::new(12.0, 12.0, 260.0, 72.0),
            provenance_line.to_owned(),
        ));
    }
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
    let p: ImportPlateMapParams = parse_params(&command_schema::<ImportPlateMapParams>(), raw)?;
    let title = p.title.as_deref().unwrap_or("Plate Data");
    validate_title(title)?;
    let target_kind = match p.target_kind.as_deref() {
        Some(raw_kind) => parse_target_archetype_kind(raw_kind)?,
        None => TargetArchetypeKind::Grouped,
    };
    if p.values.is_some() && p.source.is_some() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap cannot specify both `values` and `source`"));
    }
    if p.values.is_none() && p.source.is_none() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap requires either `values` or `source`"));
    }

    let (geometry, template, grid_warnings) = resolve_plate_template_with_doc(&ctx.session.doc, &p, title, target_kind)?;
    let (values, source_warnings, source_provenance): (Vec<Option<f64>>, Vec<String>, Option<String>) = if let Some(raw_vals) = p.values {
        (raw_vals.into_flat_checked(geometry)?, Vec::new(), None)
    } else if let Some(src) = p.source {
        if src.value_column.is_some() && src.reducer.is_some() {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                "table.importPlateMap `source` cannot specify both `value_column` and `reducer`",
            ));
        }
        let (extraction, extractor_desc) = match (src.value_column, src.reducer) {
            (Some(col), None) => (PlateSourceExtraction::WellPerRow { value_column: col.clone() }, format!("value_column `{col}`")),
            (None, Some(red)) => {
                let (reducer, desc) = red.into_transform();
                (PlateSourceExtraction::WellPerColumn { reducer }, format!("reducer `{desc}`"))
            }
            _ => {
                return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap `source` requires either `value_column` or `reducer`"));
            }
        };
        let match_mode = parse_plate_match_mode(src.match_wells.as_deref())?;
        let source_id = parse_sheet_id(&src.sheet_id)?;
        let source_sheet = require_fresh_analyzable_data_sheet(&ctx.session.doc, &source_id)?;
        let extracted = extract_plate_values_from_analyzable_sheet(source_sheet, geometry, &extraction, match_mode)?;
        let provenance = format!("Static plate snapshot from `{}` ({extractor_desc})", source_id.as_str());
        (extracted.values, extracted.warnings, Some(provenance))
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap requires either `values` or `source`"));
    };

    let subtract_blank_mean = p.subtract_blank_mean.unwrap_or(false);
    let mut mapped = map_plate_to_datasheet_with_origins(title, geometry, &values, &template, target_kind, subtract_blank_mean)?;
    if let Some(ref provenance) = source_provenance {
        append_plate_source_floating_note(&mut mapped.sheet, provenance);
    }

    let mut all_warnings = grid_warnings;
    all_warnings.extend(source_warnings);
    all_warnings.extend(mapped.warnings);
    let well_cell_map: Vec<Value> = mapped
        .well_cell_map
        .iter()
        .map(|entry| {
            json!({
                "well": entry.well,
                "row": entry.coord.row,
                "col": entry.coord.col,
                "subcol": entry.coord.subcol,
            })
        })
        .collect();

    let doc = &mut ctx.session.doc;
    let (sheet_id, sheet_title, rows, cols) = assign_and_add_imported_sheet(doc, mapped.sheet, p.sheet_id.as_deref())?;

    let mut outcome = CommandOutcome::mutated(
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
            "well_cell_map": well_cell_map,
            "warnings": all_warnings,
        }),
    );
    outcome.warnings = all_warnings;
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// 25. table.importTidyLong
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(super) enum TidyColRef {
    Index(usize),
    Name(#[schemars(length(min = 1))] String),
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(super) struct TidyMappingInput {
    /// Zero-based header index or header name for X values.
    pub(super) x_col: Option<TidyColRef>,
    /// Zero-based header index or header name for group/series.
    pub(super) group_col: Option<TidyColRef>,
    /// Zero-based header index or header name for replicate.
    pub(super) replicate_col: Option<TidyColRef>,
    /// Zero-based header index or header name for Y value.
    pub(super) value_col: Option<TidyColRef>,
    /// Zero-based header index or header name for exclusion flag.
    pub(super) excluded_col: Option<TidyColRef>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ImportTidyLongParams {
    /// Optional target sheet ID.
    #[schemars(length(min = 1))]
    pub(super) sheet_id: Option<String>,
    /// Imported sheet title (default `Tidy Import`).
    pub(super) title: Option<String>,
    /// Target archetype (Grouped, Column, XY, MultipleVariables).
    pub(super) target_kind: Option<String>,
    /// Column header names when using `rows`.
    pub(super) headers: Option<Vec<String>>,
    /// 2D tidy record rows.
    pub(super) rows: Option<Vec<Vec<Value>>>,
    /// Inline delimited tidy table text.
    pub(super) csv: Option<String>,
    /// Filesystem path to delimited tidy file.
    #[schemars(length(min = 1))]
    pub(super) path: Option<String>,
    /// Column delimiter (`auto`, `,`, `\\t`/`tab`, `;`, or single character).
    pub(super) delimiter: Option<String>,
    /// Decimal separator (`.`, `,`, `dot`, `comma`, `period`, or `auto`).
    pub(super) decimal_sep: Option<String>,
    /// Infer European decimal separator when ambiguous.
    pub(super) normalize_decimal: Option<bool>,
    /// Column role mapping object.
    pub(super) mapping: Option<TidyMappingInput>,
    #[serde(flatten)]
    pub(super) flat_mapping: TidyMappingInput,
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
    match s.trim().to_ascii_lowercase().as_str() {
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
        policy.read_delimited_text_file(path, "tidy file", MAX_CSV_FILE_BYTES)?
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
    let has_flat = p.flat_mapping.x_col.is_some()
        || p.flat_mapping.group_col.is_some()
        || p.flat_mapping.replicate_col.is_some()
        || p.flat_mapping.value_col.is_some()
        || p.flat_mapping.excluded_col.is_some();
    if p.mapping.is_some() && has_flat {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "table.importTidyLong cannot combine nested `mapping` with top-level column mapping fields",
        )
        .with_remediation("Supply either `mapping: { ... }` or top-level column mapping fields, not both."));
    }
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
    let p: ImportTidyLongParams = parse_params(&command_schema::<ImportTidyLongParams>(), raw)?;
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
        let params =
            ImportPrismParams { path: None, bytes_base64: None, bytes: Some(vec![0u8; MAX_ARCHIVE_FILE_BYTES + 1]), merge: None, merged_at: None };
        let res = import_prism_with_params(&mut ctx, params);
        assert!(matches!(res, Err(err) if err.code == ErrorCode::ResourceLimit));
    }

    #[test]
    fn aud_02_05_merge_imported_document_preserves_images_and_remaps_outliers() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, ColumnDesignHint, ColumnTable, DescriptiveSpec, ImageMimeKind, LayoutItem, LayoutSheet, LayoutSpec,
            ProjectImageAsset, RectPt, SourceCellId,
        };

        let mut dest = ProjectDocument::new("Dest").unwrap();
        let d_dest =
            DataSheet::new("data-1", "Dest Data", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap()))
                .unwrap();
        dest.add_data_sheet(d_dest).unwrap();

        let mut imported = ProjectDocument::new("Imported").unwrap();
        let d_imp =
            DataSheet::new("data-1", "Imp Data", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap()))
                .unwrap();
        imported.add_data_sheet(d_imp).unwrap();

        let asset = ProjectImageAsset {
            asset_id: String::from("img-0123456789abcdef"),
            media_type: ImageMimeKind::Png,
            pixel_size: (2, 2),
            bytes: vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0],
        };
        imported.upsert_image_asset(asset.clone()).unwrap();

        let mut layout_spec = LayoutSpec::default();
        layout_spec
            .add_item(LayoutItem::EmbeddedImage {
                id: String::from("img-panel"),
                asset_id: asset.asset_id.clone(),
                bounds_pt: RectPt::new(10.0, 10.0, 40.0, 40.0),
                rotation_deg: 0.0,
                lock_aspect_ratio: true,
                crop: curvebook_doc::CropInsetsPt::default(),
                border: None,
                badge_override: None,
            })
            .unwrap();
        imported.add_layout_sheet(LayoutSheet::new("layout-1", "Imp Layout", layout_spec).unwrap()).unwrap();

        let spec =
            AnalysisSpec::new(SheetId::new("res-1"), SheetId::new("data-1"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
        let mut res = ResultSheet::new(spec).unwrap();
        res.eliminated_outliers.push(SourceCellId { sheet_id: SheetId::new("data-1"), row: 0, col: 0, subcol: 0 });
        imported.add_result_sheet(res).unwrap();

        merge_imported_document(&mut dest, imported).unwrap();

        assert!(dest.image_asset(&asset.asset_id).is_some(), "merged document must retain imported image assets");
        let merged_res = dest.result_sheet(&SheetId::new("res-1")).expect("merged result sheet");
        assert_eq!(merged_res.analysis.source_sheet_id.as_ref().map(SheetId::as_str), Some("data-1-2"));
        assert_eq!(merged_res.eliminated_outliers.len(), 1);
        assert_eq!(
            merged_res.eliminated_outliers[0].sheet_id.as_str(),
            "data-1-2",
            "eliminated_outliers.sheet_id must be remapped on sheet ID collision"
        );
    }
}
