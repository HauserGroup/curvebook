//! Document, sheet, spreadsheet cell, import/link, variable, info, and recalculation command dispatchers.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, Cell, CellCoord, CellValue, CloneDataSheetOptions, ColumnDesignHint, ColumnTable, ConstantValue,
    ContingencyTable, DataSetColumn, DataSheet, GraphAnnotation, GraphSheet, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec,
    ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutItem, LayoutSheet, LevelOrderKind, LevelOrderSpec, LinkSource, MultipleVariablesTable,
    NestedTable, ParameterScope, PartsOfWholeTable, PlacementMode, ProjectDocument, ResultSheet, ResultStatus, SheetCellRange, SheetId, SheetSection,
    SimulationNoise, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, VariableColumn, VariableRole, VariableSource, XColumnMode, XyTable,
    validate_rows, validate_title,
};
use lumencraft_format::{
    CsvOptions, export_datasheet_to_csv, export_result_sheet_tidy_csv, import_csv_to_datasheet, import_prism_bytes, import_pzfx_bytes,
    import_pzfx_str, instantiate_template_bytes, open_project_bytes, open_project_file, save_project_bytes, save_project_file, save_template_bytes,
    save_template_file,
};
use lumencraft_transform::{
    ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, apply_import_link_pipeline, evaluate_data_sheet_calculated_variables,
    simulate_xy_sheet,
};
use serde_json::{Value, json};

use crate::dag;
use crate::{CommandOutcome, EngineError};

/// Try dispatching data/document commands; returns `Ok(None)` when `command_id` belongs elsewhere.
pub fn dispatch_data_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    if let Some(outcome) = dispatch_project_command(doc, command_id, params)? {
        return Ok(Some(outcome));
    }
    if let Some(outcome) = dispatch_sheet_command(doc, command_id, params)? {
        return Ok(Some(outcome));
    }
    if let Some(outcome) = dispatch_table_command(doc, command_id, params)? {
        return Ok(Some(outcome));
    }
    if let Some(outcome) = dispatch_info_command(doc, command_id, params)? {
        return Ok(Some(outcome));
    }
    Ok(None)
}

fn dispatch_project_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    match command_id {
        "file.new" | "project.new" => cmd_project_new(doc, command_id, params).map(Some),
        "project.open" => cmd_project_open(doc, command_id, params).map(Some),
        "project.save" => cmd_project_save(doc, command_id, params).map(Some),
        "project.saveTemplate" => cmd_project_save_template(doc, command_id, params).map(Some),
        "project.instantiateTemplate" => cmd_project_instantiate_template(doc, command_id, params).map(Some),
        "project.inspect" => cmd_project_inspect(doc, command_id, params).map(Some),
        "project.recalculate" => cmd_project_recalculate(doc, command_id).map(Some),
        _ => Ok(None),
    }
}

fn dispatch_sheet_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    match command_id {
        "sheet.createData" => cmd_sheet_create_data(doc, command_id, params).map(Some),
        "sheet.rename" => cmd_sheet_rename(doc, command_id, params).map(Some),
        "sheet.delete" => cmd_sheet_delete(doc, command_id, params).map(Some),
        "sheet.duplicate" => cmd_sheet_duplicate(doc, command_id, params).map(Some),
        "sheet.duplicateFamily" => cmd_sheet_duplicate_family(doc, command_id, params).map(Some),
        "sheet.clonePipeline" => cmd_sheet_clone_pipeline(doc, command_id, params).map(Some),
        "sheet.freeze" => cmd_sheet_set_frozen(doc, command_id, params, true).map(Some),
        "sheet.unfreeze" => cmd_sheet_set_frozen(doc, command_id, params, false).map(Some),
        "sheet.reorder" => cmd_sheet_reorder(doc, command_id, params).map(Some),
        "sheet.select" => cmd_sheet_select(doc, command_id, params).map(Some),
        _ => Ok(None),
    }
}

fn dispatch_table_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    match command_id {
        "table.setCell" => cmd_table_set_cell(doc, command_id, params).map(Some),
        "table.setCells" => cmd_table_set_cells(doc, command_id, params).map(Some),
        "table.setXCells" => cmd_table_set_x_cells(doc, command_id, params).map(Some),
        "table.setRowTitles" => cmd_table_set_row_titles(doc, command_id, params).map(Some),
        "table.setGroupTitles" | "table.setTitles" => cmd_table_set_group_titles(doc, command_id, params).map(Some),
        "table.setSubcolumnTitles" => cmd_table_set_subcolumn_titles(doc, command_id, params).map(Some),
        "table.excludeCells" => cmd_table_exclude_cells(doc, command_id, params).map(Some),
        "table.clearCells" => cmd_table_clear_cells(doc, command_id, params).map(Some),
        "table.paste" => cmd_table_paste(doc, command_id, params).map(Some),
        "table.pasteLink" => cmd_table_paste_link(doc, command_id, params).map(Some),
        "table.importCsv" => cmd_table_import_csv(doc, command_id, params).map(Some),
        "table.exportCsv" => cmd_table_export_csv(doc, command_id, params).map(Some),
        "table.importPrism" => cmd_table_import_prism(doc, command_id, params).map(Some),
        "table.importPzfx" => cmd_table_import_pzfx(doc, command_id, params).map(Some),
        "table.setVariableRole" => cmd_table_set_variable_role(doc, command_id, params).map(Some),
        "table.recodeLevels" => cmd_table_recode_levels(doc, command_id, params).map(Some),
        "table.setCategoricalOrder" => cmd_table_set_categorical_order(doc, command_id, params).map(Some),
        "table.setCalculatedVariable" => cmd_table_set_calculated_variable(doc, command_id, params).map(Some),
        "table.simulateXy" => cmd_table_simulate_xy(doc, command_id, params).map(Some),
        _ => Ok(None),
    }
}

fn dispatch_info_command(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<Option<CommandOutcome>, EngineError> {
    match command_id {
        "info.setNote" => cmd_info_set_note(doc, command_id, params).map(Some),
        "info.setConstant" => cmd_info_set_constant(doc, command_id, params).map(Some),
        "info.removeConstant" => cmd_info_remove_constant(doc, command_id, params).map(Some),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Reactive DAG & LinkedDataRegion Recalculation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct RecalcSummary {
    recalculated_sheets: usize,
    refreshed_links: usize,
}

fn recalculate_with_links(doc: &mut ProjectDocument, include_external_links: bool) -> Result<RecalcSummary, EngineError> {
    let mut refreshed = refresh_linked_regions(doc, include_external_links)?;
    let mut recalculated = dag::recalculate_project(doc)?;
    let max_hops = doc.data_sheets.len().saturating_add(1);
    for _ in 0..max_hops {
        let hop_refreshed = refresh_linked_regions(doc, include_external_links)?;
        if hop_refreshed == 0 {
            break;
        }
        refreshed = refreshed.saturating_add(hop_refreshed);
        recalculated = recalculated.saturating_add(dag::recalculate_project(doc)?);
    }
    Ok(RecalcSummary { recalculated_sheets: recalculated, refreshed_links: refreshed })
}

enum OwnedSourcePayload {
    Data(Box<DataSheet>),
    Result(Box<ResultSheet>),
    Delimited { text: String, delimiter: char, decimal_separator: char },
}

fn refresh_linked_regions(doc: &mut ProjectDocument, include_external_links: bool) -> Result<usize, EngineError> {
    let order = dag::topological_order(doc)?;
    let mut updated_count = 0usize;

    for target_id in order {
        let Some(target_idx) = doc.data_sheets.iter().position(|s| s.id == target_id) else {
            continue;
        };
        let is_frozen = doc.data_sheets.get(target_idx).is_none_or(|ds| ds.frozen);
        if is_frozen {
            continue;
        }
        let regions = doc.data_sheets.get(target_idx).map(|ds| ds.linked_regions.clone()).unwrap_or_default();
        if regions.is_empty() {
            continue;
        }

        let mut target_changed = false;
        for region in regions {
            let Some(source_payload) = resolve_link_source_payload(doc, &region.spec.source, include_external_links)? else {
                continue;
            };
            let Some(target_ds) = doc.data_sheets.get_mut(target_idx) else {
                continue;
            };
            let changed = apply_refresh_to_target(target_ds, &region.spec, &source_payload)?;
            if changed {
                target_changed = true;
                updated_count = updated_count.saturating_add(1);
            }
        }
        if target_changed {
            let info_snapshot = doc.data_sheets.get(target_idx).and_then(|ds| ds.info_sheet_id.as_ref()).and_then(|iid| doc.info_sheet(iid)).cloned();
            if let Some(target_ds) = doc.data_sheets.get_mut(target_idx) {
                evaluate_data_sheet_calculated_variables(target_ds, info_snapshot.as_ref())?;
            }
        }
    }
    Ok(updated_count)
}

fn resolve_link_source_payload(
    doc: &ProjectDocument,
    source: &LinkSource,
    include_external_links: bool,
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
            if !include_external_links {
                return Ok(None);
            }
            let text = std::fs::read_to_string(path).map_err(|e| EngineError::new(format!("failed reading linked file `{path}`: {e}")))?;
            Ok(Some(OwnedSourcePayload::Delimited { text, delimiter: *delimiter, decimal_separator: *decimal_separator }))
        }
    }
}

fn apply_refresh_to_target(target_ds: &mut DataSheet, spec: &ImportLinkSpec, payload: &OwnedSourcePayload) -> Result<bool, EngineError> {
    let before = target_ds.clone();
    let existing_links = target_ds.linked_regions.len();
    let mut refresh_spec = spec.clone();
    refresh_spec.placement.mode = PlacementMode::Overwrite;

    let opts = ImportPipelineOptions::default();
    run_import_pipeline(target_ds, &refresh_spec, payload, &opts, None)?;
    propagate_internal_source_titles(target_ds, &refresh_spec, payload);
    target_ds.linked_regions.truncate(existing_links);
    Ok(target_ds != &before)
}

fn propagate_internal_source_titles(target_ds: &mut DataSheet, spec: &ImportLinkSpec, payload: &OwnedSourcePayload) {
    if !spec.placement.propagate_title_header {
        return;
    }
    let LinkSource::InternalSheet { range, .. } = &spec.source else {
        return;
    };
    let OwnedSourcePayload::Data(src_ds) = payload else {
        return;
    };
    let base_col = spec.placement.anchor.col;
    if spec.placement.transpose {
        for (offset, row) in (range.start.row..=range.end.row).enumerate() {
            if let Some(title) = src_row_title(&src_ds.archetype, row)
                && !title.is_empty()
                && base_col.saturating_add(offset) < target_ds.archetype.column_count()
            {
                let _ = set_group_title(&mut target_ds.archetype, base_col.saturating_add(offset), title);
            }
        }
    } else {
        for (offset, col) in (range.start.col..=range.end.col).enumerate() {
            if let Some(title) = src_group_title(&src_ds.archetype, col)
                && !title.is_empty()
                && base_col.saturating_add(offset) < target_ds.archetype.column_count()
            {
                let _ = set_group_title(&mut target_ds.archetype, base_col.saturating_add(offset), title);
            }
        }
    }
}

fn src_row_title(arch: &TableArchetype, row: usize) -> Option<&str> {
    let slice = match arch {
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    };
    slice.get(row).map(String::as_str)
}

fn src_group_title(arch: &TableArchetype, col: usize) -> Option<&str> {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(col).map(|d| d.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col).map(|v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(col).map(|g| g.title.as_str()),
    }
}

fn run_import_pipeline(
    target_ds: &mut DataSheet,
    spec: &ImportLinkSpec,
    payload: &OwnedSourcePayload,
    opts: &ImportPipelineOptions,
    info_sheet: Option<&mut InfoSheet>,
) -> Result<ImportPipelineReport, EngineError> {
    match payload {
        OwnedSourcePayload::Data(src) => Ok(apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalData(src), opts, info_sheet)?),
        OwnedSourcePayload::Result(src) => {
            Ok(apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalResult(src), opts, info_sheet)?)
        }
        OwnedSourcePayload::Delimited { text, delimiter, decimal_separator } => {
            let src = ImportSourcePayload::DelimitedText { text, delimiter: *delimiter, decimal_separator: *decimal_separator };
            Ok(apply_import_link_pipeline(target_ds, spec, src, opts, info_sheet)?)
        }
    }
}

// ---------------------------------------------------------------------------
// Project Commands
// ---------------------------------------------------------------------------

fn cmd_project_new(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let title = opt_str(params, &["title", "name"]).unwrap_or("Untitled Project");
    *doc = ProjectDocument::new(title)?;
    Ok(CommandOutcome::mutated(command_id, "created new project", json!({ "title": doc.title })))
}

fn cmd_project_open(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let loaded = if let Some(path) = opt_str(params, &["path", "file_path", "filePath"]) {
        open_project_file(Path::new(path))?
    } else if let Some(b64) = opt_str(params, &["bytes_base64", "bytesBase64", "archive_base64"]) {
        let bytes = decode_base64(b64)?;
        open_project_bytes(&bytes)?
    } else if let Some(byte_arr) = opt_array(params, &["bytes"]) {
        let bytes = parse_byte_array(byte_arr)?;
        open_project_bytes(&bytes)?
    } else {
        return Err(EngineError::new("project.open requires `path` or `bytes_base64`"));
    };

    *doc = loaded;
    Ok(CommandOutcome::mutated(
        command_id,
        "opened project",
        json!({
            "title": doc.title,
            "sheet_count": doc.total_sheet_count(),
            "active_sheet_id": doc.active_sheet_id.as_ref().map(SheetId::as_str),
        }),
    ))
}

fn cmd_project_save(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let path = opt_str(params, &["path", "file_path", "filePath"]);
    let return_bytes = opt_bool(params, &["return_bytes", "returnBytes", "include_bytes", "includeBytes"]).unwrap_or(path.is_none());

    if let Some(p) = path {
        save_project_file(Path::new(p), doc)?;
    }
    let (bytes_len, encoded) = if return_bytes || path.is_none() {
        let bytes = save_project_bytes(doc)?;
        let len = bytes.len();
        let b64 = if return_bytes { Some(encode_base64(&bytes)) } else { None };
        (Some(len), b64)
    } else {
        (None, None)
    };

    Ok(CommandOutcome::read_only(
        command_id,
        "saved project",
        json!({ "title": doc.title, "path": path, "bytes_len": bytes_len, "bytes_base64": encoded }),
    ))
}

fn cmd_project_save_template(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let path = opt_str(params, &["path", "file_path", "filePath"]);
    let return_bytes = opt_bool(params, &["return_bytes", "returnBytes", "include_bytes", "includeBytes"]).unwrap_or(path.is_none());

    if let Some(p) = path {
        save_template_file(Path::new(p), doc)?;
    }
    let (bytes_len, encoded) = if return_bytes || path.is_none() {
        let bytes = save_template_bytes(doc)?;
        let len = bytes.len();
        let b64 = if return_bytes { Some(encode_base64(&bytes)) } else { None };
        (Some(len), b64)
    } else {
        (None, None)
    };

    Ok(CommandOutcome::read_only(
        command_id,
        "saved project template",
        json!({ "title": doc.title, "path": path, "bytes_len": bytes_len, "bytes_base64": encoded }),
    ))
}

fn cmd_project_instantiate_template(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let new_title = opt_str(params, &["title", "project_title", "projectTitle"]);
    let bytes = if let Some(path) = opt_str(params, &["path", "file_path", "filePath", "template_path", "templatePath"]) {
        std::fs::read(Path::new(path)).map_err(|e| EngineError::new(format!("failed reading template `{path}`: {e}")))?
    } else if let Some(b64) = opt_str(params, &["bytes_base64", "bytesBase64", "archive_base64"]) {
        decode_base64(b64)?
    } else if let Some(byte_arr) = opt_array(params, &["bytes"]) {
        parse_byte_array(byte_arr)?
    } else {
        return Err(EngineError::new("project.instantiateTemplate requires `path` or `bytes_base64`"));
    };

    *doc = instantiate_template_bytes(&bytes, new_title)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "instantiated project template",
        json!({
            "title": doc.title,
            "sheet_count": doc.total_sheet_count(),
            "active_sheet_id": doc.active_sheet_id.as_ref().map(SheetId::as_str),
        }),
    ))
}

fn cmd_project_inspect(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let data_sheets: Vec<Value> = doc
        .data_sheets
        .iter()
        .map(|ds| {
            let vars = match &ds.archetype {
                TableArchetype::MultipleVariables(mv) => Some(
                    mv.variables
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let formula = match &v.source {
                                VariableSource::Entered => None,
                                VariableSource::Calculated { formula } => Some(formula.as_str()),
                            };
                            json!({
                                "index": i,
                                "title": v.title,
                                "role": format!("{:?}", v.role),
                                "role_inferred": v.role_inferred,
                                "calculated_formula": formula,
                            })
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            };
            json!({
                "id": ds.id.as_str(),
                "title": ds.title,
                "kind": format!("{:?}", ds.archetype.kind()),
                "rows": ds.archetype.row_count(),
                "cols": ds.archetype.column_count(),
                "frozen": ds.frozen,
                "info_sheet_id": ds.info_sheet_id.as_ref().map(SheetId::as_str),
                "linked_regions": ds.linked_regions.len(),
                "variables": vars,
            })
        })
        .collect();

    let info_sheets: Vec<Value> = doc
        .info_sheets
        .iter()
        .map(|info| {
            json!({
                "id": info.id.as_str(),
                "title": info.title,
                "notes_markdown": info.notes_markdown,
                "constants": info.constants.iter().map(|c| json!({ "name": c.name, "value": &c.value, "units": c.units })).collect::<Vec<_>>(),
            })
        })
        .collect();

    let result_sheets: Vec<Value> = doc
        .result_sheets
        .iter()
        .map(|res| {
            json!({
                "id": res.id.as_str(),
                "title": res.title(),
                "source_sheet_id": res.analysis.source_sheet_id.as_str(),
                "status": format!("{:?}", res.status),
                "tab_count": res.tabs.len(),
                "scalar_count": res.scalars.len(),
                "hook_count": count_analysis_hooks(&res.analysis),
            })
        })
        .collect();

    let graph_sheets: Vec<Value> = doc
        .graph_sheets
        .iter()
        .map(|gs| {
            json!({
                "id": gs.id.as_str(),
                "title": gs.title,
                "primary_source_sheet_id": gs.spec.primary_source_sheet_id.as_str(),
                "initialized": gs.initialized,
                "graph_kind": format!("{:?}", gs.spec.kind),
                "series_count": gs.spec.series.len(),
                "comparison_count": gs.spec.comparisons.len(),
                "annotation_count": gs.spec.annotations.len(),
            })
        })
        .collect();

    let layout_sheets: Vec<Value> =
        doc.layout_sheets.iter().map(|ls| json!({ "id": ls.id.as_str(), "title": ls.title, "item_count": ls.spec.items.len() })).collect();

    let focus_info = if let Some(focus_str) = opt_str(params, &["sheet_id", "sheetId"]) {
        let focus_id = SheetId::try_new(focus_str)?;
        let section = doc.sheet_section(&focus_id).ok_or_else(|| EngineError::new(format!("sheet `{focus_id}` not found")))?;
        let upstream_strs: Vec<String> = doc.upstream_dependencies(&focus_id).into_iter().map(|s| s.as_str().to_owned()).collect();
        Some(json!({ "sheet_id": focus_id.as_str(), "section": format!("{section:?}"), "upstream_dependencies": upstream_strs }))
    } else {
        None
    };

    Ok(CommandOutcome::read_only(
        command_id,
        "inspected project",
        json!({
            "title": doc.title,
            "total_sheet_count": doc.total_sheet_count(),
            "active_sheet_id": doc.active_sheet_id.as_ref().map(SheetId::as_str),
            "family_anchor_sheet_id": doc.family_anchor_sheet_id.as_ref().map(SheetId::as_str),
            "data_sheets": data_sheets,
            "info_sheets": info_sheets,
            "result_sheets": result_sheets,
            "graph_sheets": graph_sheets,
            "layout_sheets": layout_sheets,
            "focus": focus_info,
        }),
    ))
}

fn count_analysis_hooks(spec: &AnalysisSpec) -> usize {
    match &spec.kind {
        AnalysisKind::NonlinearRegression(nl) => nl.parameter_rules.iter().filter(|r| matches!(r.scope, ParameterScope::HookedConstant(_))).count(),
        _ => 0,
    }
}

fn cmd_project_recalculate(doc: &mut ProjectDocument, command_id: &str) -> Result<CommandOutcome, EngineError> {
    let summary = recalculate_with_links(doc, true)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "recalculated project",
        json!({ "recalculated_sheets": summary.recalculated_sheets, "refreshed_links": summary.refreshed_links }),
    ))
}

// ---------------------------------------------------------------------------
// Sheet Commands
// ---------------------------------------------------------------------------

fn cmd_sheet_create_data(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = match opt_str(params, &["sheet_id", "sheetId", "id"]) {
        Some(raw) => SheetId::try_new(raw)?,
        None => next_unique_sheet_id(doc, "data")?,
    };
    let default_title = format!("Data {}", doc.data_sheets.len().saturating_add(1));
    let title = opt_str(params, &["title", "name"]).unwrap_or(&default_title);
    let archetype = build_table_archetype(params)?;
    let kind = archetype.kind();

    let mut data = DataSheet::new(id.clone(), title, archetype)?;

    let auto_family = opt_bool(params, &["auto_family", "autoFamily"]).unwrap_or(false);
    let has_info_id = opt_str(params, &["info_sheet_id", "infoSheetId"]).is_some();
    let auto_info = opt_bool(params, &["auto_info", "autoInfo"]).unwrap_or(auto_family || has_info_id);
    let info_id = if auto_info {
        let iid = match opt_str(params, &["info_sheet_id", "infoSheetId"]) {
            Some(raw) => SheetId::try_new(raw)?,
            None => next_unique_sheet_id_excluding(doc, &format!("info-{}", id.as_str()), &[&id])?,
        };
        data.info_sheet_id = Some(iid.clone());
        Some(iid)
    } else {
        None
    };

    doc.add_data_sheet(data)?;

    if let Some(iid) = &info_id {
        let info = InfoSheet::new_preseeded(iid.clone(), format!("{title} Info"))?;
        doc.add_info_sheet(info)?;
    }

    let has_graph_id = opt_str(params, &["graph_sheet_id", "graphSheetId"]).is_some();
    let auto_graph = opt_bool(params, &["auto_graph", "autoGraph"]).unwrap_or(auto_family || has_graph_id);
    let graph_id = if auto_graph {
        let gid = match opt_str(params, &["graph_sheet_id", "graphSheetId"]) {
            Some(raw) => SheetId::try_new(raw)?,
            None => next_unique_sheet_id(doc, "graph")?,
        };
        if let Some(ds) = doc.data_sheet(&id).cloned() {
            let graph = GraphSheet::new_uninitialized_for_data(gid.clone(), &ds)?;
            doc.add_graph_sheet(graph)?;
            Some(gid)
        } else {
            None
        }
    } else {
        None
    };

    doc.select_in_navigator(&id)?;

    Ok(CommandOutcome::mutated(
        command_id,
        "created data sheet",
        json!({
            "sheet_id": id.as_str(),
            "title": title,
            "kind": format!("{kind:?}"),
            "info_sheet_id": info_id.as_ref().map(SheetId::as_str),
            "graph_sheet_id": graph_id.as_ref().map(SheetId::as_str),
        }),
    ))
}

fn build_table_archetype(params: &Value) -> Result<TableArchetype, EngineError> {
    let kind = parse_table_kind(opt_str(params, &["archetype", "kind", "table_kind", "tableKind", "type"]).unwrap_or("xy"))?;
    let rows = opt_usize(params, &["rows", "row_count", "rowCount"]).unwrap_or(12);
    let cols = opt_usize(params, &["cols", "columns", "data_sets", "dataSets", "column_count", "columnCount", "groups", "cohorts"]);
    let replicates = opt_usize(params, &["replicates", "replicate_k", "k", "subcolumns", "biological_units", "biologicalUnits"]).unwrap_or(1);
    let subcolumn_mode = parse_subcolumn_mode(params, replicates)?;

    match kind {
        TableKind::Xy => {
            let y_sets = cols.unwrap_or(2);
            let x_mode = parse_x_mode(params, replicates)?;
            let mut table = XyTable::new_with_x_mode(x_mode, subcolumn_mode, y_sets, rows)?;
            if let Some(xt) = opt_str(params, &["x_title", "xTitle"]) {
                table.x_title = xt.to_owned();
            }
            Ok(TableArchetype::Xy(table))
        }
        TableKind::Column => {
            let design = parse_column_design_hint(opt_str(params, &["design_hint", "designHint"]).unwrap_or("unpaired"))?;
            let sets = cols.unwrap_or(3);
            Ok(TableArchetype::Column(ColumnTable::new_with_subcolumns(design, subcolumn_mode, sets, rows)?))
        }
        TableKind::Grouped => {
            let sets = cols.unwrap_or(2);
            let mut table = GroupedTable::new(subcolumn_mode, sets, rows)?;
            if let Some(rf) = opt_str(params, &["row_factor_title", "rowFactorTitle"]) {
                table.row_factor_title = rf.to_owned();
            }
            if let Some(cf) = opt_str(params, &["column_factor_title", "columnFactorTitle"]) {
                table.column_factor_title = cf.to_owned();
            }
            Ok(TableArchetype::Grouped(table))
        }
        TableKind::Contingency => {
            let sets = cols.unwrap_or(2);
            let contingency_rows = opt_usize(params, &["rows", "row_count", "rowCount"]).unwrap_or(2);
            let mut table = ContingencyTable::new(sets, contingency_rows)?;
            if let Some(rf) = opt_str(params, &["row_factor_title", "rowFactorTitle"]) {
                table.row_factor_title = rf.to_owned();
            }
            if let Some(cf) = opt_str(params, &["column_factor_title", "columnFactorTitle"]) {
                table.column_factor_title = cf.to_owned();
            }
            Ok(TableArchetype::Contingency(table))
        }
        TableKind::Survival => {
            let cohorts = cols.unwrap_or(2);
            let mut table = SurvivalTable::new(cohorts, rows)?;
            if let Some(tt) = opt_str(params, &["time_title", "timeTitle"]) {
                table.time_title = tt.to_owned();
            }
            Ok(TableArchetype::Survival(table))
        }
        TableKind::PartsOfWhole => {
            let series = cols.unwrap_or(1);
            let slice_rows = opt_usize(params, &["rows", "row_count", "rowCount", "slices"]).unwrap_or(6);
            Ok(TableArchetype::PartsOfWhole(PartsOfWholeTable::new(series, slice_rows)?))
        }
        TableKind::MultipleVariables => {
            let vars = cols.unwrap_or(4);
            let mut table = MultipleVariablesTable::new(vars, rows)?;
            if let Some(var_specs) = opt_array(params, &["variables"]) {
                for (idx, item) in var_specs.iter().enumerate() {
                    let Some(var) = table.variables.get_mut(idx) else {
                        break;
                    };
                    if let Some(t) = opt_str(item, &["title", "name"]) {
                        var.title = t.to_owned();
                    }
                    if let Some(r) = opt_str(item, &["role"]) {
                        var.lock_role(parse_variable_role(r)?);
                    }
                }
            }
            Ok(TableArchetype::MultipleVariables(table))
        }
        TableKind::Nested => {
            let groups = cols.unwrap_or(2);
            let bio_units = opt_usize(params, &["replicates", "replicate_k", "k", "biological_units", "biologicalUnits", "subcolumns"]).unwrap_or(3);
            Ok(TableArchetype::Nested(NestedTable::new(groups, bio_units, rows)?))
        }
    }
}

fn cmd_sheet_rename(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId", "id"])?)?;
    let title = req_str(params, &["title", "new_title", "newTitle", "name"])?.trim();
    if title.is_empty() {
        return Err(EngineError::new("sheet title cannot be empty"));
    }

    let section = doc.sheet_section(&id).ok_or_else(|| EngineError::new(format!("sheet `{id}` not found")))?;
    match section {
        SheetSection::Data => {
            let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
            ds.ensure_mutable()?;
            ds.set_title(title)?;
            let _ = recalculate_with_links(doc, false)?;
        }
        SheetSection::Info => {
            let info = doc.info_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("info sheet `{id}` not found")))?;
            info.title = title.to_owned();
        }
        SheetSection::Analyses => {
            let res = doc.result_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` not found")))?;
            res.analysis.custom_title = Some(title.to_owned());
        }
        SheetSection::Graphs => {
            let gs = doc.graph_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?;
            gs.title = title.to_owned();
        }
        SheetSection::Layouts => {
            let ls = doc.layout_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("layout sheet `{id}` not found")))?;
            ls.title = title.to_owned();
        }
    }

    Ok(CommandOutcome::mutated(command_id, "renamed sheet", json!({ "sheet_id": id.as_str(), "title": title, "section": format!("{section:?}") })))
}

fn cmd_sheet_delete(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId", "id"])?)?;
    let cascade = opt_bool(params, &["cascade"]);
    let section = doc.sheet_section(&id).ok_or_else(|| EngineError::new(format!("sheet `{id}` not found")))?;

    if section == SheetSection::Data
        && let Some(ds) = doc.data_sheet(&id)
    {
        ds.ensure_mutable()?;
    }

    let mut to_delete: HashSet<SheetId> = HashSet::new();
    to_delete.insert(id.clone());

    if cascade == Some(false) {
        ensure_no_live_dependents(doc, &id, section)?;
    } else if section == SheetSection::Data {
        collect_dependent_family_for_delete(doc, &id, cascade == Some(true), &mut to_delete);
    } else if section == SheetSection::Analyses {
        collect_downstream_results_and_graphs(doc, &mut to_delete);
    }

    for ds in &doc.data_sheets {
        if !to_delete.contains(&ds.id) && ds.frozen && ds_links_to_any(ds, &to_delete) {
            return Err(EngineError::new(format!("cannot delete `{id}`: frozen sheet `{}` holds a link to it", ds.id)));
        }
    }

    doc.data_sheets.retain(|s| !to_delete.contains(&s.id));
    doc.info_sheets.retain(|s| !to_delete.contains(&s.id));
    doc.result_sheets.retain(|s| !to_delete.contains(&s.id));
    doc.graph_sheets.retain(|s| !to_delete.contains(&s.id));
    doc.layout_sheets.retain(|s| !to_delete.contains(&s.id));

    for ds in &mut doc.data_sheets {
        if let Some(info_id) = &ds.info_sheet_id
            && to_delete.contains(info_id)
        {
            ds.info_sheet_id = None;
        }
        if !ds.frozen {
            ds.linked_regions.retain(|r| match &r.spec.source {
                LinkSource::InternalSheet { sheet_id, .. } => !to_delete.contains(sheet_id),
                LinkSource::Clipboard | LinkSource::ExternalDelimitedFile { .. } => true,
            });
        }
    }

    // Reassign primary_source_sheet_id on surviving multi-source graphs if primary was deleted,
    // or remove the graph if no series sources survive.
    doc.graph_sheets.retain_mut(|gs| {
        gs.spec.series.retain(|s| !to_delete.contains(&s.source_sheet_id));
        if to_delete.contains(&gs.spec.primary_source_sheet_id) {
            if let Some(first) = gs.spec.series.first() {
                gs.spec.primary_source_sheet_id = first.source_sheet_id.clone();
                true
            } else {
                false
            }
        } else {
            true
        }
    });

    for ls in &mut doc.layout_sheets {
        ls.spec.items.retain(|item| match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => !to_delete.contains(graph_sheet_id),
            LayoutItem::LiveTable { source_sheet_id, .. } => !to_delete.contains(source_sheet_id),
            LayoutItem::TextCallout { .. } => true,
        });
    }

    normalize_active_sheet_selection(doc);

    let mut deleted_ids: Vec<String> = to_delete.into_iter().map(|s| s.as_str().to_owned()).collect();
    deleted_ids.sort();
    Ok(CommandOutcome::mutated(command_id, "deleted sheet", json!({ "sheet_id": id.as_str(), "deleted_ids": deleted_ids })))
}

fn ds_links_to_any(ds: &DataSheet, set: &HashSet<SheetId>) -> bool {
    ds.linked_regions.iter().any(|r| matches!(&r.spec.source, LinkSource::InternalSheet { sheet_id, .. } if set.contains(sheet_id)))
}

fn ensure_no_live_dependents(doc: &ProjectDocument, id: &SheetId, section: SheetSection) -> Result<(), EngineError> {
    for res in &doc.result_sheets {
        if &res.analysis.source_sheet_id == id || analysis_hooks_mention_sheet(&res.analysis, id) {
            return Err(EngineError::new(format!("cannot delete `{id}`: referenced by analysis `{}`", res.id)));
        }
    }
    for gs in &doc.graph_sheets {
        let is_auto_uninit = section == SheetSection::Data && !gs.initialized && &gs.spec.primary_source_sheet_id == id;
        if !is_auto_uninit && (&gs.spec.primary_source_sheet_id == id || gs.spec.series.iter().any(|s| &s.source_sheet_id == id)) {
            return Err(EngineError::new(format!("cannot delete `{id}`: referenced by graph `{}`", gs.id)));
        }
    }
    for ds in &doc.data_sheets {
        if &ds.id != id && ds.linked_regions.iter().any(|r| matches!(&r.spec.source, LinkSource::InternalSheet { sheet_id, .. } if sheet_id == id)) {
            return Err(EngineError::new(format!("cannot delete `{id}`: referenced by linked region on `{}`", ds.id)));
        }
    }
    Ok(())
}

fn collect_dependent_family_for_delete(doc: &ProjectDocument, data_id: &SheetId, aggressive_info: bool, to_delete: &mut HashSet<SheetId>) {
    collect_downstream_results_and_graphs(doc, to_delete);
    if let Some(ds) = doc.data_sheet(data_id)
        && let Some(info_id) = &ds.info_sheet_id
    {
        let used_elsewhere = doc.data_sheets.iter().any(|other| &other.id != data_id && other.info_sheet_id.as_ref() == Some(info_id));
        if !used_elsewhere {
            let empty_info = doc.info_sheet(info_id).is_some_and(|i| i.constants.is_empty() && i.notes_markdown.trim().is_empty());
            if aggressive_info || empty_info {
                to_delete.insert(info_id.clone());
            }
        }
    }
}

fn collect_downstream_results_and_graphs(doc: &ProjectDocument, to_delete: &mut HashSet<SheetId>) {
    loop {
        let before = to_delete.len();
        for res in &doc.result_sheets {
            if to_delete.contains(&res.analysis.source_sheet_id) || analysis_hooks_mention_any(&res.analysis, to_delete) {
                to_delete.insert(res.id.clone());
            }
        }
        for gs in &doc.graph_sheets {
            let primary_gone = to_delete.contains(&gs.spec.primary_source_sheet_id);
            let all_series_gone = !gs.spec.series.is_empty() && gs.spec.series.iter().all(|s| to_delete.contains(&s.source_sheet_id));
            if primary_gone || all_series_gone {
                to_delete.insert(gs.id.clone());
            }
        }
        if to_delete.len() == before {
            break;
        }
    }
}

fn normalize_active_sheet_selection(doc: &mut ProjectDocument) {
    let valid_active = doc.active_sheet_id.as_ref().and_then(|id| doc.sheet_section(id)).is_some();
    if !valid_active {
        doc.active_sheet_id = first_available_sheet_id(doc);
    }
    let valid_anchor = doc.family_anchor_sheet_id.as_ref().and_then(|id| doc.sheet_section(id)).is_some();
    if !valid_anchor {
        doc.family_anchor_sheet_id = doc.data_sheets.first().map(|s| s.id.clone()).or_else(|| doc.active_sheet_id.clone());
    }
}

fn first_available_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    doc.data_sheets
        .first()
        .map(|s| s.id.clone())
        .or_else(|| doc.info_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.result_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.graph_sheets.first().map(|s| s.id.clone()))
        .or_else(|| doc.layout_sheets.first().map(|s| s.id.clone()))
}

fn cmd_sheet_duplicate(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId", "id"])?)?;
    let section = doc.sheet_section(&id).ok_or_else(|| EngineError::new(format!("sheet `{id}` not found")))?;
    let explicit_new_id = opt_str(params, &["new_sheet_id", "newSheetId", "target_sheet_id", "targetSheetId", "new_id"]);
    let explicit_title = opt_str(params, &["title", "new_title", "newTitle"]);

    let new_id = match explicit_new_id {
        Some(raw) => SheetId::try_new(raw)?,
        None => next_unique_sheet_id(doc, id.as_str())?,
    };
    if doc.sheet_section(&new_id).is_some() {
        return Err(EngineError::new(format!("sheet `{new_id}` already exists")));
    }

    match section {
        SheetSection::Data => {
            let mut copy = doc.data_sheet(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?.clone();
            copy.id = new_id.clone();
            let title = explicit_title.map_or_else(|| format!("{} Copy", copy.title), str::to_owned);
            copy.set_title(title)?;
            copy.frozen = opt_bool(params, &["frozen"]).unwrap_or(false);
            doc.add_data_sheet(copy)?;
            let _ = recalculate_with_links(doc, false)?;
        }
        SheetSection::Info => {
            let mut copy = doc.info_sheet(&id).ok_or_else(|| EngineError::new(format!("info sheet `{id}` not found")))?.clone();
            copy.id = new_id.clone();
            copy.title = explicit_title.map_or_else(|| format!("{} Copy", copy.title), str::to_owned);
            doc.add_info_sheet(copy)?;
        }
        SheetSection::Analyses => {
            let mut copy = doc.result_sheet(&id).ok_or_else(|| EngineError::new(format!("result sheet `{id}` not found")))?.clone();
            let orig_title = copy.title().to_owned();
            copy.id = new_id.clone();
            copy.analysis.id = new_id.clone();
            copy.analysis.custom_title = Some(explicit_title.map_or_else(|| format!("{orig_title} Copy"), str::to_owned));
            doc.add_result_sheet(copy)?;
        }
        SheetSection::Graphs => {
            let mut copy = doc.graph_sheet(&id).ok_or_else(|| EngineError::new(format!("graph sheet `{id}` not found")))?.clone();
            copy.id = new_id.clone();
            copy.title = explicit_title.map_or_else(|| format!("{} Copy", copy.title), str::to_owned);
            doc.add_graph_sheet(copy)?;
        }
        SheetSection::Layouts => {
            let mut copy = doc.layout_sheet(&id).ok_or_else(|| EngineError::new(format!("layout sheet `{id}` not found")))?.clone();
            copy.id = new_id.clone();
            copy.title = explicit_title.map_or_else(|| format!("{} Copy", copy.title), str::to_owned);
            doc.add_layout_sheet(copy)?;
        }
    }

    doc.select_in_navigator(&new_id)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "duplicated sheet",
        json!({ "source_sheet_id": id.as_str(), "sheet_id": new_id.as_str(), "section": format!("{section:?}") }),
    ))
}

fn cmd_sheet_duplicate_family(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let clear_data = opt_bool(params, &["clear_data", "clearData"]).unwrap_or(false);
    let clone_opts = CloneFamilyParams {
        keep_x_values: true,
        clear_y_values: clear_data,
        clear_column_titles: false,
        clear_row_titles: false,
        override_replicates: None,
        clear_results: clear_data,
    };
    clone_or_duplicate_family(doc, command_id, params, clone_opts, "duplicated sheet family")
}

fn cmd_sheet_clone_pipeline(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let clear_y = opt_bool(params, &["clear_y_values", "clearYValues"]).unwrap_or(true);
    let clone_opts = CloneFamilyParams {
        keep_x_values: opt_bool(params, &["keep_x_values", "keepXValues"]).unwrap_or(true),
        clear_y_values: clear_y,
        clear_column_titles: opt_bool(params, &["clear_column_titles", "clearColumnTitles"]).unwrap_or(false),
        clear_row_titles: opt_bool(params, &["clear_row_titles", "clearRowTitles"]).unwrap_or(false),
        override_replicates: opt_usize(params, &["override_replicates", "overrideReplicates", "replicates"]),
        clear_results: clear_y,
    };
    clone_or_duplicate_family(doc, command_id, params, clone_opts, "cloned pipeline family")
}

#[derive(Debug, Clone, Copy)]
struct CloneFamilyParams {
    keep_x_values: bool,
    clear_y_values: bool,
    clear_column_titles: bool,
    clear_row_titles: bool,
    override_replicates: Option<usize>,
    clear_results: bool,
}

struct FamilyMembers {
    info_id: Option<SheetId>,
    result_ids: Vec<SheetId>,
    graph_ids: Vec<SheetId>,
    layout_ids: Vec<SheetId>,
}

fn clone_or_duplicate_family(
    doc: &mut ProjectDocument,
    command_id: &str,
    params: &Value,
    opts: CloneFamilyParams,
    summary_msg: &str,
) -> Result<CommandOutcome, EngineError> {
    let raw_id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId", "id"])?)?;
    let root_data_id = resolve_family_root_data_sheet(doc, &raw_id)?;
    let src_ds = doc.data_sheet(&root_data_id).ok_or_else(|| EngineError::new(format!("data sheet `{root_data_id}` not found")))?.clone();

    let id_prefix = opt_str(params, &["prefix"]).unwrap_or(root_data_id.as_str());
    let new_data_id = match opt_str(params, &["new_sheet_id", "newSheetId", "target_sheet_id", "targetSheetId"]) {
        Some(raw) => SheetId::try_new(raw)?,
        None => next_unique_sheet_id(doc, id_prefix)?,
    };
    if doc.sheet_section(&new_data_id).is_some() {
        return Err(EngineError::new(format!("sheet `{new_data_id}` already exists")));
    }

    let suffix_label = if opts.clear_y_values { "Clone" } else { "Copy" };
    let new_title = opt_str(params, &["title", "new_title", "newTitle"]).map_or_else(|| format!("{} ({suffix_label})", src_ds.title), str::to_owned);

    let family = collect_family_members(doc, &root_data_id);
    let id_map = allocate_family_id_map(doc, &root_data_id, &new_data_id, &family)?;

    let clone_spec = CloneDataSheetOptions {
        title: new_title.clone(),
        keep_x_values: opts.keep_x_values,
        clear_y_values: opts.clear_y_values,
        clear_column_titles: opts.clear_column_titles,
        clear_row_titles: opts.clear_row_titles,
        override_replicates: opts.override_replicates,
    };
    let mut cloned_data = src_ds.clone_structure(new_data_id.clone(), &clone_spec)?;
    cloned_data.frozen = false;
    cloned_data.info_sheet_id = family.info_id.as_ref().and_then(|iid| id_map.get(iid).cloned());
    doc.add_data_sheet(cloned_data)?;

    let cloned_info_id = clone_family_info(doc, &family, &id_map, &new_title)?;
    let cloned_result_ids = clone_family_results(doc, &family, &id_map, opts.clear_results, suffix_label)?;
    let cloned_graph_ids = clone_family_graphs(doc, &family, &id_map, suffix_label)?;
    let cloned_layout_ids = clone_family_layouts(doc, &family, &id_map, suffix_label)?;

    if !opts.clear_results {
        let _ = recalculate_with_links(doc, false)?;
    }
    doc.select_in_navigator(&new_data_id)?;

    let mapped_pairs: serde_json::Map<String, Value> =
        id_map.iter().map(|(k, v)| (k.as_str().to_owned(), Value::String(v.as_str().to_owned()))).collect();

    Ok(CommandOutcome::mutated(
        command_id,
        summary_msg,
        json!({
            "source_sheet_id": root_data_id.as_str(),
            "sheet_id": new_data_id.as_str(),
            "title": new_title,
            "info_sheet_id": cloned_info_id.as_ref().map(SheetId::as_str),
            "result_sheet_ids": cloned_result_ids.iter().map(SheetId::as_str).collect::<Vec<_>>(),
            "graph_sheet_ids": cloned_graph_ids.iter().map(SheetId::as_str).collect::<Vec<_>>(),
            "layout_sheet_ids": cloned_layout_ids.iter().map(SheetId::as_str).collect::<Vec<_>>(),
            "id_map": Value::Object(mapped_pairs),
        }),
    ))
}

fn allocate_family_id_map(
    doc: &ProjectDocument,
    root_data_id: &SheetId,
    new_data_id: &SheetId,
    family: &FamilyMembers,
) -> Result<HashMap<SheetId, SheetId>, EngineError> {
    let mut id_map = HashMap::new();
    let mut reserved = HashSet::new();
    reserved.insert(new_data_id.as_str().to_owned());
    id_map.insert(root_data_id.clone(), new_data_id.clone());

    let all_ids = family.info_id.iter().chain(family.result_ids.iter()).chain(family.graph_ids.iter()).chain(family.layout_ids.iter());
    for orig_id in all_ids {
        let next = next_unique_id_with_reserved(doc, orig_id.as_str(), &reserved)?;
        reserved.insert(next.as_str().to_owned());
        id_map.insert(orig_id.clone(), next);
    }
    Ok(id_map)
}

fn clone_family_info(
    doc: &mut ProjectDocument,
    family: &FamilyMembers,
    id_map: &HashMap<SheetId, SheetId>,
    new_title: &str,
) -> Result<Option<SheetId>, EngineError> {
    let Some(src_info_id) = &family.info_id else {
        return Ok(None);
    };
    let Some(new_info_id) = id_map.get(src_info_id).cloned() else {
        return Ok(None);
    };
    let Some(mut new_info) = doc.info_sheet(src_info_id).cloned() else {
        return Ok(None);
    };
    new_info.id = new_info_id.clone();
    new_info.title = format!("{new_title} Info");
    doc.add_info_sheet(new_info)?;
    Ok(Some(new_info_id))
}

fn clone_family_results(
    doc: &mut ProjectDocument,
    family: &FamilyMembers,
    id_map: &HashMap<SheetId, SheetId>,
    clear_results: bool,
    suffix_label: &str,
) -> Result<Vec<SheetId>, EngineError> {
    let mut out = Vec::new();
    for src_id in &family.result_ids {
        if let Some(src_res) = doc.result_sheet(src_id).cloned()
            && let Some(new_id) = id_map.get(src_id).cloned()
        {
            let remapped = remap_result_sheet(src_res, new_id.clone(), id_map, clear_results, suffix_label);
            doc.add_result_sheet(remapped)?;
            out.push(new_id);
        }
    }
    Ok(out)
}

fn clone_family_graphs(
    doc: &mut ProjectDocument,
    family: &FamilyMembers,
    id_map: &HashMap<SheetId, SheetId>,
    suffix_label: &str,
) -> Result<Vec<SheetId>, EngineError> {
    let mut out = Vec::new();
    for src_id in &family.graph_ids {
        if let Some(src_gs) = doc.graph_sheet(src_id).cloned()
            && let Some(new_id) = id_map.get(src_id).cloned()
        {
            let remapped = remap_graph_sheet(src_gs, new_id.clone(), id_map, suffix_label);
            doc.add_graph_sheet(remapped)?;
            out.push(new_id);
        }
    }
    Ok(out)
}

fn clone_family_layouts(
    doc: &mut ProjectDocument,
    family: &FamilyMembers,
    id_map: &HashMap<SheetId, SheetId>,
    suffix_label: &str,
) -> Result<Vec<SheetId>, EngineError> {
    let mut out = Vec::new();
    for src_id in &family.layout_ids {
        if let Some(src_ls) = doc.layout_sheet(src_id).cloned()
            && let Some(new_id) = id_map.get(src_id).cloned()
        {
            let remapped = remap_layout_sheet(src_ls, new_id.clone(), id_map, suffix_label);
            doc.add_layout_sheet(remapped)?;
            out.push(new_id);
        }
    }
    Ok(out)
}

fn resolve_family_root_data_sheet(doc: &ProjectDocument, sheet_id: &SheetId) -> Result<SheetId, EngineError> {
    if doc.data_sheet(sheet_id).is_some() {
        return Ok(sheet_id.clone());
    }
    if let Some(res) = doc.result_sheet(sheet_id) {
        return resolve_family_root_data_sheet(doc, &res.analysis.source_sheet_id);
    }
    if let Some(gs) = doc.graph_sheet(sheet_id) {
        return resolve_family_root_data_sheet(doc, &gs.spec.primary_source_sheet_id);
    }
    if let Some(ds) = doc.data_sheets.iter().find(|d| d.info_sheet_id.as_ref() == Some(sheet_id)) {
        return Ok(ds.id.clone());
    }
    Err(EngineError::new(format!("cannot resolve data sheet family root for `{sheet_id}`")))
}

fn collect_family_members(doc: &ProjectDocument, root_data_id: &SheetId) -> FamilyMembers {
    let info_id = doc.data_sheet(root_data_id).and_then(|d| d.info_sheet_id.clone());
    let mut member_set: HashSet<SheetId> = HashSet::new();
    member_set.insert(root_data_id.clone());
    if let Some(iid) = &info_id {
        member_set.insert(iid.clone());
    }

    let mut result_ids = Vec::new();
    loop {
        let before = member_set.len();
        for res in &doc.result_sheets {
            if !member_set.contains(&res.id) && member_set.contains(&res.analysis.source_sheet_id) {
                member_set.insert(res.id.clone());
                result_ids.push(res.id.clone());
            }
        }
        if member_set.len() == before {
            break;
        }
    }

    let mut graph_ids = Vec::new();
    for gs in &doc.graph_sheets {
        let primary_match = member_set.contains(&gs.spec.primary_source_sheet_id);
        let series_match = gs.spec.series.iter().any(|s| member_set.contains(&s.source_sheet_id));
        if primary_match || series_match {
            member_set.insert(gs.id.clone());
            graph_ids.push(gs.id.clone());
        }
    }

    let mut layout_ids = Vec::new();
    for ls in &doc.layout_sheets {
        let uses_member = ls.spec.items.iter().any(|item| match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => member_set.contains(graph_sheet_id),
            LayoutItem::LiveTable { source_sheet_id, .. } => member_set.contains(source_sheet_id),
            LayoutItem::TextCallout { .. } => false,
        });
        if uses_member {
            layout_ids.push(ls.id.clone());
        }
    }

    FamilyMembers { info_id, result_ids, graph_ids, layout_ids }
}

fn remap_result_sheet(
    mut res: ResultSheet,
    new_id: SheetId,
    id_map: &HashMap<SheetId, SheetId>,
    clear_results: bool,
    suffix_label: &str,
) -> ResultSheet {
    let orig_title = res.title().to_owned();
    res.id = new_id.clone();
    res.analysis.id = new_id;
    if let Some(mapped) = id_map.get(&res.analysis.source_sheet_id) {
        res.analysis.source_sheet_id = mapped.clone();
    }
    if let Some(custom) = res.analysis.custom_title.take() {
        res.analysis.custom_title = Some(format!("{custom} ({suffix_label})"));
    } else {
        res.analysis.custom_title = Some(format!("{orig_title} ({suffix_label})"));
    }
    remap_analysis_hooks(&mut res.analysis, id_map);
    if clear_results {
        res.status = ResultStatus::Dirty;
        res.analyzable_data = None;
        res.tabs.clear();
        res.scalars.clear();
        res.pairwise_comparisons.clear();
        res.fitted_curves.clear();
        res.checklist.clear();
    } else if let Some(analyzable) = &mut res.analyzable_data {
        analyzable.id = res.id.clone();
    }
    res
}

fn remap_analysis_hooks(spec: &mut AnalysisSpec, id_map: &HashMap<SheetId, SheetId>) {
    if let AnalysisKind::NonlinearRegression(nl) = &mut spec.kind {
        for rule in &mut nl.parameter_rules {
            if let ParameterScope::HookedConstant(source) = &mut rule.scope {
                remap_hook_source(source, id_map);
            }
        }
    }
}

fn remap_hook_source(source: &mut HookSource, id_map: &HashMap<SheetId, SheetId>) {
    match source {
        HookSource::InfoConstant { info_sheet_id: Some(iid), .. } => {
            if let Some(mapped) = id_map.get(iid) {
                *iid = mapped.clone();
            }
        }
        HookSource::ResultScalar { result_sheet_id, .. } => {
            if let Some(mapped) = id_map.get(result_sheet_id) {
                *result_sheet_id = mapped.clone();
            }
        }
        HookSource::ColumnTitleValue { data_sheet_id, .. } => {
            if let Some(mapped) = id_map.get(data_sheet_id) {
                *data_sheet_id = mapped.clone();
            }
        }
        HookSource::InfoConstant { info_sheet_id: None, .. } => {}
    }
}

fn remap_graph_sheet(mut gs: GraphSheet, new_id: SheetId, id_map: &HashMap<SheetId, SheetId>, suffix_label: &str) -> GraphSheet {
    gs.id = new_id;
    gs.title = format!("{} ({suffix_label})", gs.title);
    if let Some(mapped) = id_map.get(&gs.spec.primary_source_sheet_id) {
        gs.spec.primary_source_sheet_id = mapped.clone();
    }
    for series in &mut gs.spec.series {
        if let Some(mapped) = id_map.get(&series.source_sheet_id) {
            series.source_sheet_id = mapped.clone();
        }
    }
    for cmp in &mut gs.spec.comparisons {
        if let Some(mapped) = id_map.get(&cmp.result_sheet_id) {
            cmp.result_sheet_id = mapped.clone();
        }
    }
    for ann in &mut gs.spec.annotations {
        if let GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } = ann
            && let Some(mapped) = id_map.get(result_sheet_id)
        {
            *result_sheet_id = mapped.clone();
        }
    }
    gs
}

fn remap_layout_sheet(mut ls: LayoutSheet, new_id: SheetId, id_map: &HashMap<SheetId, SheetId>, suffix_label: &str) -> LayoutSheet {
    ls.id = new_id;
    ls.title = format!("{} ({suffix_label})", ls.title);
    for item in &mut ls.spec.items {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => {
                if let Some(mapped) = id_map.get(graph_sheet_id) {
                    *graph_sheet_id = mapped.clone();
                }
            }
            LayoutItem::LiveTable { source_sheet_id, .. } => {
                if let Some(mapped) = id_map.get(source_sheet_id) {
                    *source_sheet_id = mapped.clone();
                }
            }
            LayoutItem::TextCallout { .. } => {}
        }
    }
    ls
}

fn cmd_sheet_set_frozen(doc: &mut ProjectDocument, command_id: &str, params: &Value, frozen: bool) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId", "id"])?)?;
    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.frozen = frozen;

    let recalc = if frozen { RecalcSummary::default() } else { recalculate_with_links(doc, false)? };
    let msg = if frozen { "froze data sheet" } else { "unfroze data sheet" };
    Ok(CommandOutcome::mutated(
        command_id,
        msg,
        json!({ "sheet_id": id.as_str(), "frozen": frozen, "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn cmd_sheet_reorder(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    if let Some(order_arr) = opt_array(params, &["order", "sheet_ids", "sheetIds"]) {
        let section_str = req_str(params, &["section"])?;
        let section = parse_sheet_section(section_str)?;
        let order: Vec<SheetId> = order_arr
            .iter()
            .map(|v| {
                let s = v.as_str().ok_or_else(|| EngineError::new("`order` entries must be sheet ID strings"))?;
                Ok(SheetId::try_new(s)?)
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        apply_section_order(doc, section, &order)?;
        return Ok(CommandOutcome::mutated(
            command_id,
            "reordered section sheets",
            json!({ "section": format!("{section:?}"), "order": order.iter().map(SheetId::as_str).collect::<Vec<_>>() }),
        ));
    }

    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId", "id"])?)?;
    let section = doc.sheet_section(&id).ok_or_else(|| EngineError::new(format!("sheet `{id}` not found")))?;
    let target_index = req_usize(params, &["index", "to_index", "toIndex", "position"])?;
    move_sheet_in_section(doc, section, &id, target_index)?;

    Ok(CommandOutcome::mutated(
        command_id,
        "reordered sheet",
        json!({ "sheet_id": id.as_str(), "section": format!("{section:?}"), "index": target_index }),
    ))
}

fn apply_section_order(doc: &mut ProjectDocument, section: SheetSection, order: &[SheetId]) -> Result<(), EngineError> {
    match section {
        SheetSection::Data => reorder_vec_by_id(&mut doc.data_sheets, order, |s| &s.id),
        SheetSection::Info => reorder_vec_by_id(&mut doc.info_sheets, order, |s| &s.id),
        SheetSection::Analyses => reorder_vec_by_id(&mut doc.result_sheets, order, |s| &s.id),
        SheetSection::Graphs => reorder_vec_by_id(&mut doc.graph_sheets, order, |s| &s.id),
        SheetSection::Layouts => reorder_vec_by_id(&mut doc.layout_sheets, order, |s| &s.id),
    }
}

fn reorder_vec_by_id<T>(items: &mut Vec<T>, order: &[SheetId], get_id: fn(&T) -> &SheetId) -> Result<(), EngineError> {
    if items.len() != order.len() {
        return Err(EngineError::new(format!("expected {} sheet IDs for reorder, got {}", items.len(), order.len())));
    }
    let unique: HashSet<&SheetId> = order.iter().collect();
    if unique.len() != order.len() {
        return Err(EngineError::new("duplicate sheet ID in reorder list"));
    }
    let mut map: HashMap<SheetId, T> = items.drain(..).map(|item| (get_id(&item).clone(), item)).collect();
    for id in order {
        let item = map.remove(id).ok_or_else(|| EngineError::new(format!("sheet `{id}` does not belong to target section")))?;
        items.push(item);
    }
    Ok(())
}

fn move_sheet_in_section(doc: &mut ProjectDocument, section: SheetSection, id: &SheetId, target_index: usize) -> Result<(), EngineError> {
    match section {
        SheetSection::Data => move_vec_item(&mut doc.data_sheets, id, target_index, |s| &s.id),
        SheetSection::Info => move_vec_item(&mut doc.info_sheets, id, target_index, |s| &s.id),
        SheetSection::Analyses => move_vec_item(&mut doc.result_sheets, id, target_index, |s| &s.id),
        SheetSection::Graphs => move_vec_item(&mut doc.graph_sheets, id, target_index, |s| &s.id),
        SheetSection::Layouts => move_vec_item(&mut doc.layout_sheets, id, target_index, |s| &s.id),
    }
}

fn move_vec_item<T>(items: &mut Vec<T>, id: &SheetId, target_index: usize, get_id: fn(&T) -> &SheetId) -> Result<(), EngineError> {
    let from = items.iter().position(|item| get_id(item) == id).ok_or_else(|| EngineError::new(format!("sheet `{id}` not found in section")))?;
    if target_index >= items.len() {
        return Err(EngineError::new(format!("reorder index {target_index} out of bounds for len {}", items.len())));
    }
    let item = items.remove(from);
    items.insert(target_index, item);
    Ok(())
}

fn cmd_sheet_select(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId", "id"])?)?;
    let mode = opt_str(params, &["mode", "pane"]).unwrap_or("navigator");
    if mode.eq_ignore_ascii_case("family") {
        doc.select_in_family_pane(&id)?;
    } else {
        doc.select_in_navigator(&id)?;
    }
    Ok(CommandOutcome::mutated(
        command_id,
        "selected sheet",
        json!({
            "sheet_id": id.as_str(),
            "mode": mode,
            "active_sheet_id": doc.active_sheet_id.as_ref().map(SheetId::as_str),
            "family_anchor_sheet_id": doc.family_anchor_sheet_id.as_ref().map(SheetId::as_str),
        }),
    ))
}

// ---------------------------------------------------------------------------
// Table Cell & Title Commands
// ---------------------------------------------------------------------------

fn cmd_table_set_cell(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let coord = parse_cell_coord(params)?;
    let cell = parse_cell_from_params(params)?;
    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;
    ds.set_cell(coord, cell)?;
    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated cell",
        json!({
            "sheet_id": id.as_str(),
            "row": coord.row,
            "col": coord.col,
            "subcol": coord.subcol,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

fn cmd_table_set_cells(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let mut edits: Vec<(CellCoord, Cell)> = Vec::new();

    if let Some(arr) = opt_array(params, &["edits", "cells"]) {
        for item in arr {
            let coord = parse_cell_coord(item)?;
            let cell = parse_cell_from_params(item)?;
            edits.push((coord, cell));
        }
    } else if let Some(rows_arr) = opt_array(params, &["rows", "grid"]) {
        let start_row = opt_usize(params, &["start_row", "startRow"]).unwrap_or(0);
        let start_col = opt_usize(params, &["start_col", "startCol"]).unwrap_or(0);
        let start_subcol = opt_usize(params, &["start_subcol", "startSubcol"]).unwrap_or(0);
        for (r_off, row_val) in rows_arr.iter().enumerate() {
            let row_cells = row_val.as_array().ok_or_else(|| EngineError::new("each entry in `rows` must be an array"))?;
            for (c_off, cell_val) in row_cells.iter().enumerate() {
                let coord = CellCoord::new(start_row.saturating_add(r_off), start_col.saturating_add(c_off), start_subcol);
                edits.push((coord, parse_cell_from_value(cell_val)?));
            }
        }
    } else {
        return Err(EngineError::new("table.setCells requires `edits`/`cells` or `rows`"));
    }

    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;
    for (coord, cell) in &edits {
        ds.set_cell(*coord, cell.clone())?;
    }
    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated cells",
        json!({ "sheet_id": id.as_str(), "edited_count": edits.len(), "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn cmd_table_set_x_cells(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let mut updates: Vec<(usize, usize, Cell)> = Vec::new();

    if let Some(arr) = opt_array(params, &["edits", "cells"]) {
        for item in arr {
            let row = req_usize(item, &["row"])?;
            let subcol = opt_usize(item, &["subcol", "subcolumn", "replicate"]).unwrap_or(0);
            updates.push((row, subcol, parse_cell_from_params(item)?));
        }
    } else if let Some(vals) = opt_array(params, &["values", "x"]) {
        let start_row = opt_usize(params, &["start_row", "startRow"]).unwrap_or(0);
        let subcol = opt_usize(params, &["subcol", "subcolumn", "replicate"]).unwrap_or(0);
        for (idx, val) in vals.iter().enumerate() {
            updates.push((start_row.saturating_add(idx), subcol, parse_cell_from_value(val)?));
        }
    } else {
        let row = req_usize(params, &["row"])?;
        let subcol = opt_usize(params, &["subcol", "subcolumn", "replicate"]).unwrap_or(0);
        updates.push((row, subcol, parse_cell_from_params(params)?));
    }

    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;
    for (row, subcol, cell) in &updates {
        ds.set_x_or_time_cell(*row, *subcol, cell.clone())?;
    }
    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated X/time cells",
        json!({ "sheet_id": id.as_str(), "edited_count": updates.len(), "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn cmd_table_set_row_titles(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let mut updated = 0usize;
    if let Some(arr) = opt_array(params, &["titles", "row_titles", "rowTitles"]) {
        let start_row = opt_usize(params, &["start_row", "startRow"]).unwrap_or(0);
        let titles = parse_validated_row_title_batch(&ds.archetype, start_row, arr)?;
        for (offset, title) in titles.into_iter().enumerate() {
            set_row_title(&mut ds.archetype, start_row.saturating_add(offset), title)?;
            updated = updated.saturating_add(1);
        }
    } else {
        let row = req_usize(params, &["row"])?;
        let title = opt_str(params, &["title"]).map(str::to_owned);
        set_row_title(&mut ds.archetype, row, title)?;
        updated = 1;
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated row titles",
        json!({ "sheet_id": id.as_str(), "updated_count": updated, "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn parse_validated_row_title_batch(arch: &TableArchetype, start_row: usize, arr: &[Value]) -> Result<Vec<Option<String>>, EngineError> {
    let row_count = arch.row_count();
    if !arr.is_empty() {
        let end_exclusive = start_row.checked_add(arr.len()).ok_or_else(|| EngineError::new("row title range overflow"))?;
        if end_exclusive > row_count {
            return Err(EngineError::new(format!("row title range {start_row}..{end_exclusive} exceeds row count {row_count}")));
        }
    }
    let mut out = Vec::with_capacity(arr.len());
    for val in arr {
        let title = match val {
            Value::Null => None,
            Value::String(s) => {
                validate_title(s)?;
                Some(s.clone())
            }
            _ => return Err(EngineError::new("row title entry must be a string or null")),
        };
        out.push(title);
    }
    Ok(out)
}

fn cmd_table_set_group_titles(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let mut updated = 0usize;
    if let Some(xt) = opt_str(params, &["x_title", "xTitle", "time_title", "timeTitle"]) {
        set_x_or_factor_title(&mut ds.archetype, Some(xt), None, None)?;
        updated = updated.saturating_add(1);
    }
    if opt_str(params, &["row_factor_title", "rowFactorTitle"]).is_some() || opt_str(params, &["column_factor_title", "columnFactorTitle"]).is_some()
    {
        set_x_or_factor_title(
            &mut ds.archetype,
            None,
            opt_str(params, &["row_factor_title", "rowFactorTitle"]),
            opt_str(params, &["column_factor_title", "columnFactorTitle"]),
        )?;
        updated = updated.saturating_add(1);
    }

    if let Some(arr) =
        opt_array(params, &["titles", "group_titles", "groupTitles", "column_titles", "columnTitles", "data_set_titles", "dataSetTitles"])
    {
        let start_col = opt_usize(params, &["start_col", "startCol"]).unwrap_or(0);
        for (offset, val) in arr.iter().enumerate() {
            let title = val.as_str().ok_or_else(|| EngineError::new("group title must be a string"))?;
            set_group_title(&mut ds.archetype, start_col.saturating_add(offset), title)?;
            updated = updated.saturating_add(1);
        }
    } else if let Some(title) = opt_str(params, &["title"]) {
        let col = req_usize(params, &["col", "group", "data_set", "dataSet", "column", "index"])?;
        set_group_title(&mut ds.archetype, col, title)?;
        updated = updated.saturating_add(1);
    }

    if let Some(row_titles) = opt_array(params, &["row_titles", "rowTitles"]) {
        let validated = parse_validated_row_title_batch(&ds.archetype, 0, row_titles)?;
        for (idx, title) in validated.into_iter().enumerate() {
            set_row_title(&mut ds.archetype, idx, title)?;
            updated = updated.saturating_add(1);
        }
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated group titles",
        json!({ "sheet_id": id.as_str(), "updated_count": updated, "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn cmd_table_set_subcolumn_titles(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let TableArchetype::Nested(nested) = &mut ds.archetype else {
        return Err(EngineError::new(format!("sheet `{id}` does not store explicit subcolumn titles (only Nested tables store subcolumn_titles)")));
    };

    let col_opt = opt_usize(params, &["col", "group", "data_set", "dataSet", "column"]);
    let mut updated = 0usize;
    if let Some(arr) = opt_array(params, &["titles", "subcolumn_titles", "subcolumnTitles"]) {
        let start_subcol = opt_usize(params, &["start_subcol", "startSubcol"]).unwrap_or(0);
        for (offset, val) in arr.iter().enumerate() {
            let title = val.as_str().ok_or_else(|| EngineError::new("subcolumn title must be a string"))?;
            set_nested_subcol_title(nested, col_opt, start_subcol.saturating_add(offset), title)?;
            updated = updated.saturating_add(1);
        }
    } else {
        let subcol = req_usize(params, &["subcol", "subcolumn", "replicate", "index"])?;
        let title = req_str(params, &["title"])?;
        set_nested_subcol_title(nested, col_opt, subcol, title)?;
        updated = 1;
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated subcolumn titles",
        json!({ "sheet_id": id.as_str(), "updated_count": updated, "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn cmd_table_exclude_cells(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let excluded = opt_bool(params, &["excluded"]).unwrap_or(true);
    let target_x = opt_str(params, &["target"]).is_some_and(|t| t.eq_ignore_ascii_case("x"));

    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let mut updated = 0usize;
    if let Some(range) = parse_optional_cell_range(params)? {
        validate_cell_range_bounds(ds, range, target_x)?;
        for r in range.start.row..=range.end.row {
            for c in range.start.col..=range.end.col {
                for s in range.start.subcol..=range.end.subcol {
                    if target_x {
                        ds.set_x_or_time_excluded(r, s, excluded)?;
                    } else {
                        ds.set_excluded(CellCoord::new(r, c, s), excluded)?;
                    }
                    updated = updated.saturating_add(1);
                }
            }
        }
    } else if let Some(coords) = opt_array(params, &["cells", "coords"]) {
        for item in coords {
            if target_x {
                let row = req_usize(item, &["row"])?;
                let subcol = opt_usize(item, &["subcol", "subcolumn", "replicate"]).unwrap_or(0);
                ds.set_x_or_time_excluded(row, subcol, excluded)?;
            } else {
                ds.set_excluded(parse_cell_coord(item)?, excluded)?;
            }
            updated = updated.saturating_add(1);
        }
    } else if target_x {
        let row = req_usize(params, &["row"])?;
        let subcol = opt_usize(params, &["subcol", "subcolumn", "replicate"]).unwrap_or(0);
        ds.set_x_or_time_excluded(row, subcol, excluded)?;
        updated = 1;
    } else {
        ds.set_excluded(parse_cell_coord(params)?, excluded)?;
        updated = 1;
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated exclusion mask",
        json!({ "sheet_id": id.as_str(), "excluded": excluded, "updated_count": updated, "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn cmd_table_clear_cells(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let mut cleared = 0usize;
    if opt_bool(params, &["all"]).unwrap_or(false) {
        let rows = ds.archetype.row_count();
        let cols = ds.archetype.column_count();
        let clear_x = opt_bool(params, &["clear_x", "clearX", "include_x", "includeX"]).unwrap_or(false);
        for r in 0..rows {
            if clear_x && matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_)) {
                let x_subcols = x_subcolumn_count(&ds.archetype);
                for s in 0..x_subcols {
                    ds.set_x_or_time_cell(r, s, Cell::empty())?;
                    cleared = cleared.saturating_add(1);
                }
            }
            for c in 0..cols {
                let subcols = column_subcolumn_count(&ds.archetype, c);
                for s in 0..subcols {
                    ds.set_cell(CellCoord::new(r, c, s), Cell::empty())?;
                    cleared = cleared.saturating_add(1);
                }
            }
        }
    } else if let Some(range) = parse_optional_cell_range(params)? {
        validate_cell_range_bounds(ds, range, false)?;
        for r in range.start.row..=range.end.row {
            for c in range.start.col..=range.end.col {
                for s in range.start.subcol..=range.end.subcol {
                    ds.set_cell(CellCoord::new(r, c, s), Cell::empty())?;
                    cleared = cleared.saturating_add(1);
                }
            }
        }
    } else if let Some(coords) = opt_array(params, &["cells", "coords"]) {
        for item in coords {
            ds.set_cell(parse_cell_coord(item)?, Cell::empty())?;
            cleared = cleared.saturating_add(1);
        }
    } else {
        ds.set_cell(parse_cell_coord(params)?, Cell::empty())?;
        cleared = 1;
    }

    if opt_bool(params, &["unlink", "break_links", "breakLinks"]).unwrap_or(false) {
        ds.linked_regions.clear();
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "cleared cells",
        json!({ "sheet_id": id.as_str(), "cleared_count": cleared, "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn parse_optional_cell_range(params: &Value) -> Result<Option<SheetCellRange>, EngineError> {
    if let Some(range_val) = opt_val(params, &["range", "selection"]) {
        return parse_cell_range(range_val).map(Some);
    }
    let has_bounds = ["row_start", "rowStart", "row_end", "rowEnd", "col_start", "colStart", "col_end", "colEnd"]
        .iter()
        .any(|k| opt_usize(params, &[*k]).is_some());
    if !has_bounds {
        return Ok(None);
    }
    let r0 = opt_usize(params, &["row_start", "rowStart", "row"]).unwrap_or(0);
    let r1 = opt_usize(params, &["row_end", "rowEnd"]).unwrap_or(r0);
    let c0 = opt_usize(params, &["col_start", "colStart", "col"]).unwrap_or(0);
    let c1 = opt_usize(params, &["col_end", "colEnd"]).unwrap_or(c0);
    let s0 = opt_usize(params, &["subcol_start", "subcolStart", "subcol"]).unwrap_or(0);
    let s1 = opt_usize(params, &["subcol_end", "subcolEnd"]).unwrap_or(s0);
    Ok(Some(SheetCellRange::new(CellCoord::new(r0.min(r1), c0.min(c1), s0.min(s1)), CellCoord::new(r0.max(r1), c0.max(c1), s0.max(s1)))))
}

fn validate_cell_range_bounds(ds: &DataSheet, range: SheetCellRange, target_x: bool) -> Result<(), EngineError> {
    let rows = ds.archetype.row_count();
    if range.end.row >= rows {
        return Err(EngineError::new(format!("range end row {} out of range (row count {rows})", range.end.row)));
    }
    if target_x {
        if !matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_)) {
            return Err(EngineError::new("target=`x` requires an XY or Survival sheet"));
        }
        let x_subcols = x_subcolumn_count(&ds.archetype);
        if range.end.subcol >= x_subcols {
            return Err(EngineError::new(format!("range end X subcolumn {} out of range (width {x_subcols})", range.end.subcol)));
        }
        return Ok(());
    }
    let cols = ds.archetype.column_count();
    if range.end.col >= cols {
        return Err(EngineError::new(format!("range end column {} out of range (column count {cols})", range.end.col)));
    }
    for col in range.start.col..=range.end.col {
        let subcols = column_subcolumn_count(&ds.archetype, col);
        if range.end.subcol >= subcols {
            return Err(EngineError::new(format!("range end subcolumn {} out of range on column {col} (width {subcols})", range.end.subcol)));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Paste, Paste Link, CSV & Archive Imports/Exports
// ---------------------------------------------------------------------------

fn cmd_table_paste(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId", "target_sheet_id", "targetSheetId"])?)?;
    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let anchor = parse_anchor_coord(params)?;
    let transpose = opt_bool(params, &["transpose"]).unwrap_or(false);
    let propagate_title_header = opt_bool(params, &["propagate_title_header", "propagateTitleHeader"]).unwrap_or(false);
    let filter = parse_import_filter(opt_val(params, &["filter", "slice"]))?;
    let placement = ImportPlacementSpec {
        mode: parse_placement_mode(opt_str(params, &["mode", "placement_mode", "placementMode"]).unwrap_or("overwrite"))?,
        anchor,
        propagate_title_header,
        skip_contiguous_blanks: opt_bool(params, &["skip_contiguous_blanks", "skipContiguousBlanks"]).unwrap_or(false),
        transpose,
    };
    let spec = ImportLinkSpec { source: LinkSource::Clipboard, filter, placement };

    let existing_links = ds.linked_regions.len();
    let opts = ImportPipelineOptions::default();

    let report = if let Some(text) = opt_str(params, &["text", "tsv", "csv", "clipboard"]) {
        let delimiter = parse_delimiter(params, text)?;
        let decimal_separator = parse_decimal_char(params, text, delimiter)?;
        let payload = ImportSourcePayload::DelimitedText { text, delimiter, decimal_separator };
        apply_import_link_pipeline(ds, &spec, payload, &opts, None)?
    } else if let Some(rows_arr) = opt_array(params, &["rows", "grid"]) {
        let dec = parse_decimal_char_opt(params).unwrap_or('.');
        let tsv = serialize_2d_rows_as_tsv(rows_arr, dec)?;
        let payload = ImportSourcePayload::DelimitedText { text: &tsv, delimiter: '\t', decimal_separator: '.' };
        apply_import_link_pipeline(ds, &spec, payload, &opts, None)?
    } else {
        return Err(EngineError::new("table.paste requires `text`/`tsv`/`csv` or `rows`/`grid`"));
    };

    ds.linked_regions.truncate(existing_links);
    let recalc = recalculate_with_links(doc, false)?;

    Ok(CommandOutcome::mutated(
        command_id,
        "pasted table data",
        json!({
            "sheet_id": id.as_str(),
            "rows_placed": report.rows_placed,
            "cols_placed": report.cols_placed,
            "target_range": &report.target_range,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

fn serialize_2d_rows_as_tsv(rows_arr: &[Value], decimal_separator: char) -> Result<String, EngineError> {
    let mut lines = Vec::with_capacity(rows_arr.len());
    for row_val in rows_arr {
        let row_cells = row_val.as_array().ok_or_else(|| EngineError::new("each row in `rows` must be an array"))?;
        let mut tokens = Vec::with_capacity(row_cells.len());
        for cell_val in row_cells {
            let cell = parse_cell_with_options(cell_val, decimal_separator, true)?;
            tokens.push(format_cell_as_tsv_token(&cell));
        }
        lines.push(tokens.join("\t"));
    }
    Ok(lines.join("\n"))
}

fn format_cell_as_tsv_token(cell: &Cell) -> String {
    let base = match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(v) => v.to_string(),
        CellValue::Text(s) => s.replace(['\t', '\n', '\r'], " "),
    };
    if cell.excluded && !base.is_empty() { format!("{base}*") } else { base }
}

fn cmd_table_paste_link(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let target_id = SheetId::try_new(req_str(params, &["target_sheet_id", "targetSheetId", "sheet_id", "sheetId"])?)?;
    {
        let target_ds = doc.data_sheet(&target_id).ok_or_else(|| EngineError::new(format!("data sheet `{target_id}` not found")))?;
        target_ds.ensure_mutable()?;
    }

    let spec = parse_paste_link_spec(doc, &target_id, params)?;
    let info_id = doc.data_sheet(&target_id).and_then(|d| d.info_sheet_id.clone());
    let payload = resolve_link_source_payload(doc, &spec.source, true)?
        .ok_or_else(|| EngineError::new("table.pasteLink requires an internal sheet or external delimited file source"))?;

    let (target_idx, info_idx) = locate_target_and_info_indices(doc, &target_id, info_id.as_ref())?;
    let prev_ds = doc.data_sheets.get(target_idx).cloned();
    let prev_info = info_idx.and_then(|idx| doc.info_sheets.get(idx).cloned());

    let link_seq = prev_ds.as_ref().map_or(1, |d| d.linked_regions.len().saturating_add(1));
    let link_id = opt_str(params, &["link_id", "linkId"]).map_or_else(|| format!("link-{}-{link_seq}", target_id.as_str()), str::to_owned);
    let provenance_summary =
        opt_str(params, &["provenance", "provenance_summary"]).map_or_else(|| summarize_link_source(&spec.source), str::to_owned);
    let opts = ImportPipelineOptions {
        baseline: parse_baseline_adjustment(params)?,
        exclude_coords: Vec::new(),
        exclude_row_predicate: opt_str(params, &["exclude_row_predicate", "excludeRowPredicate"]).map(str::to_owned),
        link_id: Some(link_id),
        provenance_summary: Some(provenance_summary),
    };

    let outcome = (|| -> Result<ImportPipelineReport, EngineError> {
        let report = apply_pipeline_with_split_info(doc, target_idx, info_idx, &spec, &payload, &opts)?;
        let _ = dag::topological_order(doc)?;
        Ok(report)
    })();

    let report = match outcome {
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

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
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

fn summarize_link_source(source: &LinkSource) -> String {
    match source {
        LinkSource::Clipboard => String::from("Clipboard paste"),
        LinkSource::InternalSheet { sheet_id, kind, .. } => format!("Link from {kind:?} `{sheet_id}`"),
        LinkSource::ExternalDelimitedFile { path, .. } => format!("Link from `{path}`"),
    }
}

fn parse_paste_link_spec(doc: &ProjectDocument, target_id: &SheetId, params: &Value) -> Result<ImportLinkSpec, EngineError> {
    if let Some(raw_spec) = opt_val(params, &["link_spec", "linkSpec"])
        && let Ok(mut parsed) = serde_json::from_value::<ImportLinkSpec>(raw_spec.clone())
    {
        if let Some(anchor_val) = opt_val(params, &["anchor"]) {
            parsed.placement.anchor = parse_cell_coord_flexible(anchor_val)?;
        }
        if let Some(transpose) = opt_bool(params, &["transpose"]) {
            parsed.placement.transpose = transpose;
        }
        if let Some(prop) = opt_bool(params, &["propagate_title_header", "propagateTitleHeader"]) {
            parsed.placement.propagate_title_header = prop;
        }
        return Ok(parsed);
    }

    let anchor = parse_anchor_coord(params)?;
    let transpose = opt_bool(params, &["transpose"]).unwrap_or(false);
    let propagate_title_header = opt_bool(params, &["propagate_title_header", "propagateTitleHeader"]).unwrap_or(transpose && anchor.row == 0);
    let filter = parse_import_filter(opt_val(params, &["filter", "slice"]))?;
    let placement = ImportPlacementSpec {
        mode: parse_placement_mode(opt_str(params, &["mode", "placement_mode", "placementMode"]).unwrap_or("overwrite"))?,
        anchor,
        propagate_title_header,
        skip_contiguous_blanks: opt_bool(params, &["skip_contiguous_blanks", "skipContiguousBlanks"]).unwrap_or(false),
        transpose,
    };

    let source = if let Some(src_str) = opt_str(params, &["source_sheet_id", "sourceSheetId"]) {
        let src_id = SheetId::try_new(src_str)?;
        if &src_id == target_id {
            return Err(EngineError::new(format!("self-link on `{target_id}` would create a DAG cycle")));
        }
        let kind = resolve_internal_link_kind(doc, &src_id, opt_str(params, &["source_kind", "sourceKind"]))?;
        let tab_id = opt_str(params, &["tab_id", "tabId", "source_tab_id", "sourceTabId"]).map(str::to_owned);
        let range = match opt_val(params, &["range", "source_range", "sourceRange"]) {
            Some(rv) => parse_cell_range(rv)?,
            None => default_full_source_range(doc, &src_id, kind, tab_id.as_deref())?,
        };
        LinkSource::InternalSheet { sheet_id: src_id, kind, tab_id, range }
    } else if let Some(path) = opt_str(params, &["source_path", "sourcePath", "path", "file_path", "filePath"]) {
        let preview_text = std::fs::read_to_string(path).unwrap_or_default();
        let delimiter = parse_delimiter(params, &preview_text)?;
        let decimal_separator = parse_decimal_char(params, &preview_text, delimiter)?;
        LinkSource::ExternalDelimitedFile { path: path.to_owned(), delimiter, decimal_separator }
    } else {
        return Err(EngineError::new("table.pasteLink requires `link_spec`, `source_sheet_id`, or `path`"));
    };

    Ok(ImportLinkSpec { source, filter, placement })
}

fn resolve_internal_link_kind(doc: &ProjectDocument, src_id: &SheetId, explicit: Option<&str>) -> Result<InternalLinkKind, EngineError> {
    if let Some(raw) = explicit {
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        return match norm.as_str() {
            "data" | "datasheet" => Ok(InternalLinkKind::DataSheet),
            "analyzableresult" | "analyzableresultsheet" => Ok(InternalLinkKind::AnalyzableResultSheet),
            "terminalresult" | "terminalresultsheet" => Ok(InternalLinkKind::TerminalResultSheet),
            "result" | "resultsheet" => {
                let res = doc.result_sheet(src_id).ok_or_else(|| EngineError::new(format!("result sheet `{src_id}` not found")))?;
                Ok(if res.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet })
            }
            _ => Err(EngineError::new(format!("unknown link source kind `{raw}`"))),
        };
    }
    match doc.sheet_section(src_id) {
        Some(SheetSection::Data) => Ok(InternalLinkKind::DataSheet),
        Some(SheetSection::Analyses) => {
            let res = doc.result_sheet(src_id).ok_or_else(|| EngineError::new(format!("result sheet `{src_id}` not found")))?;
            Ok(if res.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet })
        }
        Some(other) => Err(EngineError::new(format!("sheet `{src_id}` in section {other:?} cannot be link source"))),
        None => Err(EngineError::new(format!("source sheet `{src_id}` not found"))),
    }
}

fn default_full_source_range(
    doc: &ProjectDocument,
    src_id: &SheetId,
    kind: InternalLinkKind,
    tab_id: Option<&str>,
) -> Result<SheetCellRange, EngineError> {
    match kind {
        InternalLinkKind::DataSheet => {
            let ds = doc.data_sheet(src_id).ok_or_else(|| EngineError::new(format!("source data sheet `{src_id}` not found")))?;
            let max_r = ds.archetype.row_count().saturating_sub(1);
            let max_c = ds.archetype.column_count().saturating_sub(1);
            let max_s = (0..ds.archetype.column_count()).map(|c| column_subcolumn_count(&ds.archetype, c)).max().unwrap_or(1).saturating_sub(1);
            Ok(SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(max_r, max_c, max_s)))
        }
        InternalLinkKind::AnalyzableResultSheet | InternalLinkKind::TerminalResultSheet => {
            let res = doc.result_sheet(src_id).ok_or_else(|| EngineError::new(format!("source result sheet `{src_id}` not found")))?;
            let tab = select_result_tab(res, tab_id)?;
            let max_r = tab.rows.len().saturating_sub(1);
            let max_c = tab.column_headers.len().max(tab.rows.first().map_or(0, Vec::len)).saturating_sub(1);
            Ok(SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(max_r, max_c, 0)))
        }
    }
}

fn cmd_table_import_csv(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = match opt_str(params, &["sheet_id", "sheetId"]) {
        Some(raw) => SheetId::try_new(raw)?,
        None => next_unique_sheet_id(doc, "data")?,
    };
    let text = if let Some(inline) = opt_str(params, &["csv", "tsv", "text", "content"]) {
        inline.to_owned()
    } else if let Some(path) = opt_str(params, &["path", "file_path", "filePath"]) {
        std::fs::read_to_string(Path::new(path)).map_err(|e| EngineError::new(format!("failed reading CSV `{path}`: {e}")))?
    } else {
        return Err(EngineError::new("table.importCsv requires `path` or `csv`/`text`"));
    };

    let delimiter = parse_delimiter(params, &text)?;
    let decimal_separator = parse_decimal_char(params, &text, delimiter)?;
    let mut csv_opts = build_csv_options(delimiter, decimal_separator);
    if let Some(first_row_titles) = opt_bool(params, &["has_header", "hasHeader", "first_row_titles", "firstRowTitles"]) {
        csv_opts.header_rows = usize::from(first_row_titles);
    }
    if let Some(has_row_titles) = opt_bool(params, &["has_row_titles", "hasRowTitles"]) {
        csv_opts.has_row_titles = has_row_titles;
    }

    let subcolumn_mode = parse_subcolumn_mode(params, opt_usize(params, &["replicates", "k"]).unwrap_or(1))?;

    if let Some(ds) = doc.data_sheet(&id) {
        ds.ensure_mutable()?;
        let info_id = ds.info_sheet_id.clone();

        if opt_bool(params, &["replace"]).unwrap_or(false) {
            let title = opt_str(params, &["title"]).unwrap_or(&ds.title).to_owned();
            let kind = match opt_str(params, &["archetype", "kind", "table_kind", "tableKind"]) {
                Some(k) => parse_table_kind(k)?,
                None => ds.archetype.kind(),
            };
            let mut imported = import_csv_to_datasheet(id.clone(), title, &text, kind, subcolumn_mode, &csv_opts)?;
            imported.info_sheet_id = info_id;
            if let Some(slot) = doc.data_sheet_mut(&id) {
                *slot = imported;
            }
        } else {
            let anchor = parse_anchor_coord(params)?;
            let filter = parse_import_filter(opt_val(params, &["filter", "slice"]))?;
            let placement = ImportPlacementSpec {
                mode: parse_placement_mode(opt_str(params, &["mode", "placement_mode", "placementMode"]).unwrap_or("overwrite"))?,
                anchor,
                propagate_title_header: opt_bool(params, &["propagate_title_header", "propagateTitleHeader"]).unwrap_or(false),
                skip_contiguous_blanks: opt_bool(params, &["skip_contiguous_blanks", "skipContiguousBlanks"]).unwrap_or(false),
                transpose: opt_bool(params, &["transpose"]).unwrap_or(false),
            };
            let keep_link = opt_bool(params, &["link", "live_link", "liveLink"]).unwrap_or(false);
            let source = match opt_str(params, &["path", "file_path", "filePath"]) {
                Some(p) if keep_link => LinkSource::ExternalDelimitedFile { path: p.to_owned(), delimiter, decimal_separator },
                _ => LinkSource::Clipboard,
            };
            let link_spec = ImportLinkSpec { source, filter, placement };
            let (target_idx, info_idx) = locate_target_and_info_indices(doc, &id, info_id.as_ref())?;
            let opts = ImportPipelineOptions {
                link_id: if keep_link { Some(format!("csv-link-{}", id.as_str())) } else { None },
                provenance_summary: if keep_link { Some(String::from("Imported CSV link")) } else { None },
                ..ImportPipelineOptions::default()
            };
            let link_before = doc.data_sheets.get(target_idx).map_or(0, |d| d.linked_regions.len());
            let payload = OwnedSourcePayload::Delimited { text, delimiter, decimal_separator };
            apply_pipeline_with_split_info(doc, target_idx, info_idx, &link_spec, &payload, &opts)?;
            if !keep_link && let Some(target_ds) = doc.data_sheets.get_mut(target_idx) {
                target_ds.linked_regions.truncate(link_before);
            }
        }
    } else {
        let title = opt_str(params, &["title"]).unwrap_or("Imported CSV");
        let kind = parse_table_kind(opt_str(params, &["archetype", "kind", "table_kind", "tableKind"]).unwrap_or("xy"))?;
        let imported = import_csv_to_datasheet(id.clone(), title, &text, kind, subcolumn_mode, &csv_opts)?;
        doc.add_data_sheet(imported)?;
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "imported CSV into data sheet",
        json!({ "sheet_id": id.as_str(), "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn build_csv_options(delimiter: char, decimal_separator: char) -> CsvOptions {
    let mut opts = match (delimiter, decimal_separator) {
        ('\t', _) => CsvOptions::tsv(),
        (';', ',') => CsvOptions::european_csv(),
        _ => CsvOptions::csv(),
    };
    opts.delimiter = delimiter;
    opts.decimal_separator = decimal_separator;
    opts
}

fn cmd_table_export_csv(doc: &ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let mut csv_opts = CsvOptions::csv();
    if let Some(delim) = opt_str(params, &["delimiter"]) {
        csv_opts.delimiter = parse_delimiter_char(delim)?;
    }
    if let Some(dec) = parse_decimal_char_opt(params) {
        csv_opts.decimal_separator = dec;
    }

    let csv_text = if let Some(ds) = doc.data_sheet(&id) {
        export_datasheet_to_csv(ds, &csv_opts)?
    } else if let Some(res) = doc.result_sheet(&id) {
        export_result_sheet_tidy_csv(res, &csv_opts)
    } else {
        return Err(EngineError::new(format!("data or result sheet `{id}` not found")));
    };

    let path = opt_str(params, &["path", "file_path", "filePath"]);
    if let Some(p) = path {
        std::fs::write(Path::new(p), csv_text.as_bytes()).map_err(|e| EngineError::new(format!("failed writing CSV `{p}`: {e}")))?;
    }

    Ok(CommandOutcome::read_only(command_id, "exported CSV", json!({ "sheet_id": id.as_str(), "path": path, "csv": csv_text })))
}

fn cmd_table_import_prism(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let bytes = if let Some(path) = opt_str(params, &["path", "file_path", "filePath"]) {
        std::fs::read(Path::new(path)).map_err(|e| EngineError::new(format!("failed reading archive `{path}`: {e}")))?
    } else if let Some(b64) = opt_str(params, &["bytes_base64", "bytesBase64", "archive_base64"]) {
        decode_base64(b64)?
    } else if let Some(byte_arr) = opt_array(params, &["bytes"]) {
        parse_byte_array(byte_arr)?
    } else {
        return Err(EngineError::new("table.importPrism requires `path` or `bytes_base64`"));
    };

    let report = import_prism_bytes(&bytes)?;
    let merge = opt_bool(params, &["merge"]).unwrap_or(false);
    if merge {
        merge_imported_document(doc, report.document)?;
    } else {
        *doc = report.document;
    }

    Ok(CommandOutcome::mutated(
        command_id,
        "imported external archive",
        json!({ "title": doc.title, "sheet_count": doc.total_sheet_count(), "warnings": report.warnings }),
    ))
}

fn cmd_table_import_pzfx(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let report = if let Some(xml) = opt_str(params, &["xml", "text", "content"]) {
        import_pzfx_str(xml)?
    } else if let Some(path) = opt_str(params, &["path", "file_path", "filePath"]) {
        let bytes = std::fs::read(Path::new(path)).map_err(|e| EngineError::new(format!("failed reading XML file `{path}`: {e}")))?;
        import_pzfx_bytes(&bytes)?
    } else if let Some(b64) = opt_str(params, &["bytes_base64", "bytesBase64"]) {
        let bytes = decode_base64(b64)?;
        import_pzfx_bytes(&bytes)?
    } else {
        return Err(EngineError::new("table.importPzfx requires `path`, `xml`/`text`, or `bytes_base64`"));
    };

    let merge = opt_bool(params, &["merge"]).unwrap_or(false);
    if merge {
        merge_imported_document(doc, report.document)?;
    } else {
        *doc = report.document;
    }

    Ok(CommandOutcome::mutated(
        command_id,
        "imported XML project",
        json!({ "title": doc.title, "sheet_count": doc.total_sheet_count(), "warnings": report.warnings }),
    ))
}

fn merge_imported_document(doc: &mut ProjectDocument, imported: ProjectDocument) -> Result<(), EngineError> {
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
    Ok(())
}

// ---------------------------------------------------------------------------
// MultipleVariables & Simulation Commands
// ---------------------------------------------------------------------------

fn cmd_table_set_variable_role(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let role_str = req_str(params, &["role"])?;

    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let TableArchetype::MultipleVariables(mv) = &mut ds.archetype else {
        return Err(EngineError::new(format!("sheet `{id}` is not a MultipleVariables table")));
    };
    let col_idx = resolve_variable_index(mv, params)?;
    let var = mv.variables.get_mut(col_idx).ok_or_else(|| EngineError::new(format!("variable column {col_idx} out of range")))?;

    if role_str.eq_ignore_ascii_case("auto") || role_str.eq_ignore_ascii_case("inferred") {
        var.refresh_inferred_role();
    } else {
        var.lock_role(parse_variable_role(role_str)?);
    }
    let final_role = var.role;
    let role_inferred = var.role_inferred;

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated variable role",
        json!({
            "sheet_id": id.as_str(),
            "column": col_idx,
            "role": format!("{final_role:?}"),
            "role_inferred": role_inferred,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

fn cmd_table_recode_levels(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let pairs = parse_recode_mappings(params)?;

    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let TableArchetype::MultipleVariables(mv) = &mut ds.archetype else {
        return Err(EngineError::new(format!("sheet `{id}` is not a MultipleVariables table")));
    };
    let col_idx = resolve_variable_index(mv, params)?;
    let var = mv.variables.get_mut(col_idx).ok_or_else(|| EngineError::new(format!("variable column {col_idx} out of range")))?;
    let recoded_count = var.recode_levels(&pairs);

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "recoded variable levels",
        json!({ "sheet_id": id.as_str(), "column": col_idx, "recoded_count": recoded_count, "recalculated_sheets": recalc.recalculated_sheets }),
    ))
}

fn cmd_table_set_categorical_order(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let spec = parse_level_order_spec(params)?;

    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let TableArchetype::MultipleVariables(mv) = &mut ds.archetype else {
        return Err(EngineError::new(format!("sheet `{id}` is not a MultipleVariables table")));
    };
    let col_idx = resolve_variable_index(mv, params)?;
    let var = mv.variables.get_mut(col_idx).ok_or_else(|| EngineError::new(format!("variable column {col_idx} out of range")))?;
    var.level_order = spec;

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated categorical level order",
        json!({ "sheet_id": id.as_str(), "column": col_idx, "recalculated_sheets": recalc.recalculated_sheets }),
    ))
}

fn cmd_table_set_calculated_variable(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = SheetId::try_new(req_str(params, &["sheet_id", "sheetId"])?)?;
    let formula = req_str(params, &["formula", "expression"])?;
    let opt_title = opt_str(params, &["title", "name"]);

    let info_snapshot = doc.data_sheet(&id).and_then(|ds| ds.info_sheet_id.as_ref()).and_then(|iid| doc.info_sheet(iid)).cloned();
    let ds = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
    ds.ensure_mutable()?;

    let TableArchetype::MultipleVariables(mv) = &mut ds.archetype else {
        return Err(EngineError::new(format!("sheet `{id}` is not a MultipleVariables table")));
    };
    let col_idx = if opt_val(params, &["col", "column", "column_index", "columnIndex", "index", "variable"]).is_some() {
        resolve_variable_index(mv, params)?
    } else if let Some(title) = opt_title {
        if let Some(existing) = mv.find_variable(title) {
            existing
        } else {
            let row_count = mv.row_count();
            let col = VariableColumn::new(title, row_count)?;
            mv.variables.push(col);
            mv.variables.len().saturating_sub(1)
        }
    } else {
        return Err(EngineError::new("table.setCalculatedVariable requires `column`/`col` or `title`"));
    };

    let var = mv.variables.get_mut(col_idx).ok_or_else(|| EngineError::new(format!("variable column {col_idx} out of range")))?;
    if let Some(title) = opt_title {
        var.title = title.to_owned();
    }
    var.set_calculated_formula(formula);

    evaluate_data_sheet_calculated_variables(ds, info_snapshot.as_ref())?;
    let recalc = recalculate_with_links(doc, false)?;

    Ok(CommandOutcome::mutated(
        command_id,
        "updated calculated variable",
        json!({ "sheet_id": id.as_str(), "column": col_idx, "formula": formula, "recalculated_sheets": recalc.recalculated_sheets }),
    ))
}

fn cmd_table_simulate_xy(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let id = match opt_str(params, &["sheet_id", "sheetId"]) {
        Some(raw) => SheetId::try_new(raw)?,
        None => next_unique_sheet_id(doc, "data")?,
    };
    let title = opt_str(params, &["title", "name"]).unwrap_or("Simulated XY");
    let seed = opt_u64(params, &["seed"]).unwrap_or(1);
    let row_count = opt_usize(params, &["rows", "row_count", "rowCount"]).unwrap_or(12);
    let replicates = opt_usize(params, &["replicates", "k", "subcolumns"]).unwrap_or(3);
    let x_min = opt_f64(params, &["x_min", "xMin"]).unwrap_or(0.0);
    let x_max = opt_f64(params, &["x_max", "xMax"]).unwrap_or(10.0);
    let equation_id = opt_str(params, &["equation_id", "equationId", "equation"]).unwrap_or("hill_4pl");
    let sim_params = parse_simulation_parameters(params)?;
    let noise = parse_simulation_noise(params)?;

    let mut simulated = simulate_xy_sheet(id.as_str(), title, seed, row_count, replicates, x_min, x_max, equation_id, &sim_params, noise)?;

    if doc.data_sheet(&id).is_some() {
        let existing_info = doc.data_sheet(&id).and_then(|d| d.info_sheet_id.clone());
        let slot = doc.data_sheet_mut(&id).ok_or_else(|| EngineError::new(format!("data sheet `{id}` not found")))?;
        slot.ensure_mutable()?;
        if let Some(info_id) = existing_info {
            simulated.info_sheet_id = Some(info_id);
        }
        *slot = simulated;
    } else {
        doc.add_data_sheet(simulated)?;
        if opt_bool(params, &["auto_info", "autoInfo"]).unwrap_or(false) {
            let iid = next_unique_sheet_id(doc, "info")?;
            if let Some(ds) = doc.data_sheet_mut(&id) {
                ds.info_sheet_id = Some(iid.clone());
            }
            doc.add_info_sheet(InfoSheet::new_preseeded(iid, format!("{title} Info"))?)?;
        }
        if let Some(graph_id) = opt_str(params, &["graph_sheet_id", "graphSheetId"])
            && let Some(ds) = doc.data_sheet(&id).cloned()
        {
            doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data(SheetId::try_new(graph_id)?, &ds)?)?;
        }
        doc.select_in_navigator(&id)?;
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "simulated XY data sheet",
        json!({ "sheet_id": id.as_str(), "equation_id": equation_id, "row_count": row_count, "replicates": replicates, "recalculated_sheets": recalc.recalculated_sheets }),
    ))
}

// ---------------------------------------------------------------------------
// Info Sheet Commands
// ---------------------------------------------------------------------------

fn cmd_info_set_note(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let info_id = resolve_target_info_sheet_id(doc, params)?;
    let note = req_str(params, &["note", "notes", "notes_markdown", "notesMarkdown", "markdown", "text"])?;
    let append = opt_bool(params, &["append"]).unwrap_or(false);

    let info = doc.info_sheet_mut(&info_id).ok_or_else(|| EngineError::new(format!("info sheet `{info_id}` not found")))?;
    if append && !info.notes_markdown.is_empty() {
        info.notes_markdown.push('\n');
        info.notes_markdown.push_str(note);
    } else {
        info.notes_markdown = note.to_owned();
    }

    Ok(CommandOutcome::mutated(command_id, "updated info note", json!({ "sheet_id": info_id.as_str(), "notes_len": info.notes_markdown.len() })))
}

fn cmd_info_set_constant(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let info_id = resolve_target_info_sheet_id(doc, params)?;
    let name = req_str(params, &["name", "key", "constant"])?;
    let units = opt_str(params, &["units", "unit"]).map(str::to_owned);
    let val = parse_constant_value(params)?;

    let info = doc.info_sheet_mut(&info_id).ok_or_else(|| EngineError::new(format!("info sheet `{info_id}` not found")))?;
    info.set_constant(name, val, units)?;

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated info constant",
        json!({ "sheet_id": info_id.as_str(), "name": name, "recalculated_sheets": recalc.recalculated_sheets, "refreshed_links": recalc.refreshed_links }),
    ))
}

fn cmd_info_remove_constant(doc: &mut ProjectDocument, command_id: &str, params: &Value) -> Result<CommandOutcome, EngineError> {
    let info_id = resolve_target_info_sheet_id(doc, params)?;
    let name = req_str(params, &["name", "key", "constant"])?;
    let if_exists = opt_bool(params, &["if_exists", "ifExists"]).unwrap_or(false);

    let info = doc.info_sheet_mut(&info_id).ok_or_else(|| EngineError::new(format!("info sheet `{info_id}` not found")))?;
    let removed = info.remove_constant(name);
    if !removed && !if_exists {
        return Err(EngineError::new(format!("constant `{name}` not found in info sheet `{info_id}`")));
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        command_id,
        "removed info constant",
        json!({ "sheet_id": info_id.as_str(), "name": name, "removed": removed, "recalculated_sheets": recalc.recalculated_sheets }),
    ))
}

// ---------------------------------------------------------------------------
// Archetype Mutation & Inspection Helpers
// ---------------------------------------------------------------------------

fn set_row_title(arch: &mut TableArchetype, row: usize, title: Option<String>) -> Result<(), EngineError> {
    let row_count = arch.row_count();
    let val = title.unwrap_or_default();
    match arch {
        TableArchetype::Xy(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Column(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Grouped(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Contingency(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Survival(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::PartsOfWhole(t) => set_row_title_vec(&mut t.slice_labels, row_count, row, val),
        TableArchetype::MultipleVariables(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Nested(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
    }
}

fn set_row_title_vec(titles: &mut Vec<String>, row_count: usize, row: usize, val: String) -> Result<(), EngineError> {
    if row >= row_count {
        return Err(EngineError::new(format!("row {row} out of range (row count {row_count})")));
    }
    let needed = row.saturating_add(1);
    validate_rows(needed)?;
    validate_title(&val)?;
    if titles.len() < needed {
        titles.resize(needed, String::new());
    }
    if let Some(slot) = titles.get_mut(row) {
        *slot = val;
    }
    Ok(())
}

fn set_group_title(arch: &mut TableArchetype, col: usize, title: &str) -> Result<(), EngineError> {
    match arch {
        TableArchetype::Xy(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Column(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Grouped(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Contingency(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::PartsOfWhole(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Survival(t) => set_ds_title(&mut t.cohorts, col, title),
        TableArchetype::MultipleVariables(t) => {
            let var = t.variables.get_mut(col).ok_or_else(|| EngineError::new(format!("variable {col} out of range")))?;
            var.title = title.to_owned();
            Ok(())
        }
        TableArchetype::Nested(t) => {
            let grp = t.groups.get_mut(col).ok_or_else(|| EngineError::new(format!("group {col} out of range")))?;
            grp.title = title.to_owned();
            Ok(())
        }
    }
}

fn set_ds_title(sets: &mut [DataSetColumn], col: usize, title: &str) -> Result<(), EngineError> {
    let ds = sets.get_mut(col).ok_or_else(|| EngineError::new(format!("column {col} out of range")))?;
    ds.title = title.to_owned();
    Ok(())
}

fn set_nested_subcol_title(nested: &mut NestedTable, col_opt: Option<usize>, subcol: usize, title: &str) -> Result<(), EngineError> {
    if let Some(col) = col_opt {
        let grp = nested.groups.get_mut(col).ok_or_else(|| EngineError::new(format!("group {col} out of range")))?;
        assign_subcol_title(&mut grp.subcolumn_titles, grp.cells.len(), subcol, title)?;
    } else {
        for grp in &mut nested.groups {
            assign_subcol_title(&mut grp.subcolumn_titles, grp.cells.len(), subcol, title)?;
        }
    }
    Ok(())
}

fn assign_subcol_title(titles: &mut Vec<String>, width: usize, subcol: usize, title: &str) -> Result<(), EngineError> {
    if subcol >= width {
        return Err(EngineError::new(format!("subcolumn {subcol} out of range (width {width})")));
    }
    if titles.len() < width {
        titles.resize(width, String::new());
    }
    if let Some(slot) = titles.get_mut(subcol) {
        *slot = title.to_owned();
    }
    Ok(())
}

fn set_x_or_factor_title(
    arch: &mut TableArchetype,
    x_title: Option<&str>,
    row_factor: Option<&str>,
    col_factor: Option<&str>,
) -> Result<(), EngineError> {
    if let Some(xt) = x_title {
        match arch {
            TableArchetype::Xy(t) => t.x_title = xt.to_owned(),
            TableArchetype::Survival(t) => t.time_title = xt.to_owned(),
            _ => return Err(EngineError::new("x_title/time_title applies only to XY or Survival tables")),
        }
    }
    if row_factor.is_some() || col_factor.is_some() {
        match arch {
            TableArchetype::Grouped(t) => {
                if let Some(rf) = row_factor {
                    t.row_factor_title = rf.to_owned();
                }
                if let Some(cf) = col_factor {
                    t.column_factor_title = cf.to_owned();
                }
            }
            TableArchetype::Contingency(t) => {
                if let Some(rf) = row_factor {
                    t.row_factor_title = rf.to_owned();
                }
                if let Some(cf) = col_factor {
                    t.column_factor_title = cf.to_owned();
                }
            }
            _ => return Err(EngineError::new("factor titles apply only to Grouped or Contingency tables")),
        }
    }
    Ok(())
}

fn x_subcolumn_count(arch: &TableArchetype) -> usize {
    match arch {
        TableArchetype::Xy(t) => t.x_cells.first().map_or(1, Vec::len).max(1),
        TableArchetype::Survival(_) => 1,
        _ => 0,
    }
}

fn column_subcolumn_count(arch: &TableArchetype, col: usize) -> usize {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.get(col).map_or(1, DataSetColumn::subcolumn_count).max(1),
        TableArchetype::Column(t) => t.data_sets.get(col).map_or(1, DataSetColumn::subcolumn_count).max(1),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map_or(1, DataSetColumn::subcolumn_count).max(1),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map_or(1, DataSetColumn::subcolumn_count).max(1),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map_or(1, DataSetColumn::subcolumn_count).max(1),
        TableArchetype::Survival(t) => t.cohorts.get(col).map_or(1, DataSetColumn::subcolumn_count).max(1),
        TableArchetype::Nested(t) => t.groups.get(col).map_or(1, |g| g.cells.len()).max(1),
        TableArchetype::MultipleVariables(_) => 1,
    }
}

// ---------------------------------------------------------------------------
// Split Borrow & Resolution Helpers
// ---------------------------------------------------------------------------

fn locate_target_and_info_indices(
    doc: &ProjectDocument,
    target_id: &SheetId,
    info_id: Option<&SheetId>,
) -> Result<(usize, Option<usize>), EngineError> {
    let target_idx =
        doc.data_sheets.iter().position(|d| &d.id == target_id).ok_or_else(|| EngineError::new(format!("data sheet `{target_id}` not found")))?;
    let info_idx = info_id.and_then(|iid| doc.info_sheets.iter().position(|i| &i.id == iid));
    Ok((target_idx, info_idx))
}

fn apply_pipeline_with_split_info(
    doc: &mut ProjectDocument,
    target_idx: usize,
    info_idx: Option<usize>,
    spec: &ImportLinkSpec,
    payload: &OwnedSourcePayload,
    opts: &ImportPipelineOptions,
) -> Result<ImportPipelineReport, EngineError> {
    let ProjectDocument { data_sheets, info_sheets, .. } = doc;
    let target_ds = data_sheets.get_mut(target_idx).ok_or_else(|| EngineError::new("target data sheet index out of bounds"))?;
    let target_info = info_idx.and_then(|idx| info_sheets.get_mut(idx));
    let report = run_import_pipeline(target_ds, spec, payload, opts, target_info)?;
    propagate_internal_source_titles(target_ds, spec, payload);
    Ok(report)
}

fn resolve_target_info_sheet_id(doc: &ProjectDocument, params: &Value) -> Result<SheetId, EngineError> {
    let raw = req_str(params, &["sheet_id", "sheetId", "info_sheet_id", "infoSheetId", "id"])?;
    let id = SheetId::try_new(raw)?;
    if doc.info_sheet(&id).is_some() {
        return Ok(id);
    }
    if let Some(ds) = doc.data_sheet(&id)
        && let Some(paired) = &ds.info_sheet_id
        && doc.info_sheet(paired).is_some()
    {
        return Ok(paired.clone());
    }
    Err(EngineError::new(format!("info sheet `{id}` not found")))
}

fn resolve_variable_index(mv: &MultipleVariablesTable, params: &Value) -> Result<usize, EngineError> {
    if let Some(idx) = opt_usize(params, &["col", "column", "column_index", "columnIndex", "index"]) {
        return Ok(idx);
    }
    if let Some(title) = opt_str(params, &["col", "column", "variable", "title"]) {
        return mv.find_variable(title).ok_or_else(|| EngineError::new(format!("variable `{title}` not found")));
    }
    Err(EngineError::new("missing required `column`/`col` variable index or title"))
}

fn select_result_tab<'a>(res: &'a ResultSheet, tab_id: Option<&str>) -> Result<&'a lumencraft_doc::ResultTab, EngineError> {
    if let Some(tid) = tab_id {
        res.tabs.iter().find(|t| t.id == tid || t.title == tid).ok_or_else(|| EngineError::new(format!("result tab `{tid}` not found")))
    } else {
        res.tabs.first().ok_or_else(|| EngineError::new(format!("result sheet `{}` has no tabs", res.id)))
    }
}

// ---------------------------------------------------------------------------
// Sheet ID & Parameter Parsing Helpers
// ---------------------------------------------------------------------------

fn next_unique_sheet_id(doc: &ProjectDocument, prefix: &str) -> Result<SheetId, EngineError> {
    next_unique_id_with_reserved(doc, prefix, &HashSet::new())
}

fn next_unique_sheet_id_excluding(doc: &ProjectDocument, candidate: &str, exclude: &[&SheetId]) -> Result<SheetId, EngineError> {
    let mut reserved = HashSet::new();
    for id in exclude {
        reserved.insert(id.as_str().to_owned());
    }
    if let Ok(cand_id) = SheetId::try_new(candidate)
        && doc.sheet_section(&cand_id).is_none()
        && !reserved.contains(cand_id.as_str())
    {
        return Ok(cand_id);
    }
    next_unique_id_with_reserved(doc, candidate, &reserved)
}

fn next_unique_id_with_reserved(doc: &ProjectDocument, base: &str, reserved: &HashSet<String>) -> Result<SheetId, EngineError> {
    let clean_base = sanitize_id_prefix(base);
    for n in 1usize..10_000 {
        let candidate = format!("{clean_base}-{n}");
        if reserved.contains(&candidate) {
            continue;
        }
        if let Ok(id) = SheetId::try_new(&candidate)
            && doc.sheet_section(&id).is_none()
        {
            return Ok(id);
        }
    }
    Err(EngineError::new("exhausted generated sheet ID range"))
}

fn sanitize_id_prefix(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::from("sheet");
    }
    let out: String = trimmed.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
    let trimmed_dashes = out.trim_matches('-');
    if trimmed_dashes.is_empty() { String::from("sheet") } else { trimmed_dashes.chars().take(40).collect() }
}

fn opt_val<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    for key in keys {
        if let Some(v) = params.get(*key)
            && !v.is_null()
        {
            return Some(v);
        }
    }
    None
}

fn opt_str<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(s) = params.get(*key).and_then(Value::as_str) {
            return Some(s);
        }
    }
    None
}

fn req_str<'a>(params: &'a Value, keys: &[&str]) -> Result<&'a str, EngineError> {
    opt_str(params, keys).ok_or_else(|| EngineError::new(format!("missing required string parameter `{}`", keys.first().copied().unwrap_or("param"))))
}

fn opt_usize(params: &Value, keys: &[&str]) -> Option<usize> {
    for key in keys {
        if let Some(n) = params.get(*key).and_then(Value::as_u64) {
            return usize::try_from(n).ok();
        }
    }
    None
}

fn req_usize(params: &Value, keys: &[&str]) -> Result<usize, EngineError> {
    opt_usize(params, keys)
        .ok_or_else(|| EngineError::new(format!("missing required integer parameter `{}`", keys.first().copied().unwrap_or("param"))))
}

fn opt_u64(params: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(n) = params.get(*key).and_then(Value::as_u64) {
            return Some(n);
        }
    }
    None
}

fn opt_f64(params: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(f) = params.get(*key).and_then(Value::as_f64).filter(|x| x.is_finite()) {
            return Some(f);
        }
    }
    None
}

fn opt_bool(params: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(b) = params.get(*key).and_then(Value::as_bool) {
            return Some(b);
        }
    }
    None
}

fn opt_array<'a>(params: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    for key in keys {
        if let Some(arr) = params.get(*key).and_then(Value::as_array) {
            return Some(arr);
        }
    }
    None
}

fn parse_cell_coord(params: &Value) -> Result<CellCoord, EngineError> {
    if let Some(coord_obj) = opt_val(params, &["coord", "coordinate"]) {
        return parse_cell_coord(coord_obj);
    }
    let row = req_usize(params, &["row"])?;
    let col = req_usize(params, &["col", "data_set", "dataSet", "column"])?;
    let subcol = opt_usize(params, &["subcol", "subcolumn", "replicate"]).unwrap_or(0);
    Ok(CellCoord::new(row, col, subcol))
}

fn parse_cell_coord_flexible(val: &Value) -> Result<CellCoord, EngineError> {
    let row = opt_usize(val, &["row", "target_row", "targetRow"]).unwrap_or(0);
    let col = opt_usize(val, &["col", "target_col", "targetCol", "data_set", "dataSet", "column"]).unwrap_or(0);
    let subcol = opt_usize(val, &["subcol", "target_subcol", "targetSubcol", "subcolumn", "replicate"]).unwrap_or(0);
    Ok(CellCoord::new(row, col, subcol))
}

fn parse_anchor_coord(params: &Value) -> Result<CellCoord, EngineError> {
    if let Some(anchor) = opt_val(params, &["anchor", "start"]) {
        return parse_cell_coord_flexible(anchor);
    }
    let row = opt_usize(params, &["start_row", "startRow", "target_row", "targetRow", "row"]).unwrap_or(0);
    let col = opt_usize(params, &["start_col", "startCol", "target_col", "targetCol", "col", "data_set", "dataSet"]).unwrap_or(0);
    let subcol = opt_usize(params, &["start_subcol", "startSubcol", "target_subcol", "targetSubcol", "subcol", "subcolumn"]).unwrap_or(0);
    Ok(CellCoord::new(row, col, subcol))
}

fn parse_cell_range(val: &Value) -> Result<SheetCellRange, EngineError> {
    let start_val = opt_val(val, &["start", "from"]).ok_or_else(|| EngineError::new("range requires `start`"))?;
    let end_val = opt_val(val, &["end", "to"]).unwrap_or(start_val);
    let a = parse_cell_coord(start_val)?;
    let b = parse_cell_coord(end_val)?;
    let start = CellCoord::new(a.row.min(b.row), a.col.min(b.col), a.subcol.min(b.subcol));
    let end = CellCoord::new(a.row.max(b.row), a.col.max(b.col), a.subcol.max(b.subcol));
    Ok(SheetCellRange::new(start, end))
}

fn parse_cell_from_params(params: &Value) -> Result<Cell, EngineError> {
    let val = params.get("value").unwrap_or(&Value::Null);
    let dec = parse_decimal_char_opt(params).unwrap_or('.');
    let parse_asterisk = opt_bool(params, &["parse_asterisk_exclusion", "parseAsteriskExclusion"]).unwrap_or(true);
    let mut cell = parse_cell_with_options(val, dec, parse_asterisk)?;
    if let Some(excluded) = opt_bool(params, &["excluded"]) {
        cell.excluded = excluded;
    }
    Ok(cell)
}

fn parse_cell_from_value(val: &Value) -> Result<Cell, EngineError> {
    if val.is_object() {
        return parse_cell_from_params(val);
    }
    parse_cell_with_options(val, '.', true)
}

fn parse_cell_with_options(val: &Value, dec: char, parse_asterisk: bool) -> Result<Cell, EngineError> {
    match val {
        Value::Null => Ok(Cell::empty()),
        Value::Bool(b) => Ok(Cell::text(if *b { "true" } else { "false" })),
        Value::Number(n) => {
            let f = n.as_f64().filter(|x| x.is_finite()).ok_or_else(|| EngineError::new("cell numeric value must be finite"))?;
            Ok(Cell::number(f))
        }
        Value::String(s) => Ok(Cell::parse_import_token(s, dec, parse_asterisk)),
        Value::Object(_) => parse_cell_from_params(val),
        Value::Array(_) => Err(EngineError::new("cell value cannot be a JSON array")),
    }
}

fn parse_import_filter(val: Option<&Value>) -> Result<ImportFilterSpec, EngineError> {
    let Some(v) = val else {
        return Ok(ImportFilterSpec::default());
    };
    if let Ok(parsed) = serde_json::from_value::<ImportFilterSpec>(v.clone()) {
        return Ok(parsed);
    }
    let mut filter = ImportFilterSpec::default();
    if opt_val(v, &["row_start", "rowStart", "row_end", "rowEnd"]).is_some() {
        let start = opt_usize(v, &["row_start", "rowStart"]).unwrap_or(0);
        let end = opt_usize(v, &["row_end", "rowEnd"]).unwrap_or(usize::MAX);
        filter.row_window = Some((start, end));
    }
    if opt_val(v, &["col_start", "colStart", "col_end", "colEnd"]).is_some() {
        let start = opt_usize(v, &["col_start", "colStart"]).unwrap_or(0);
        let end = opt_usize(v, &["col_end", "colEnd"]).unwrap_or(usize::MAX);
        filter.col_window = Some((start, end));
    }
    if let Some(k) = opt_usize(v, &["decimate_every_k", "decimateEveryK", "every_nth_row", "everyNthRow"]) {
        filter.decimate_every_k = Some(k.max(1));
    }
    if let Some(u) = opt_usize(v, &["unstack_group_column", "unstackGroupColumn"]) {
        filter.unstack_group_column = Some(u);
    }
    filter.parse_asterisk_exclusions =
        opt_bool(v, &["parse_asterisk_exclusions", "parseAsteriskExclusions", "parse_asterisk_exclusion", "parseAsteriskExclusion"]).unwrap_or(true);
    Ok(filter)
}

fn parse_baseline_adjustment(params: &Value) -> Result<Option<(usize, BaselineOperation)>, EngineError> {
    let Some(b) = opt_val(params, &["baseline"]) else {
        return Ok(None);
    };
    let col = opt_usize(b, &["col", "column", "reference_col", "referenceCol"]).unwrap_or(0);
    let op_str = req_str(b, &["op", "operation", "mode"])?;
    let norm: String = op_str.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let op = match norm.as_str() {
        "subtract" | "difference" => BaselineOperation::Difference,
        "divide" | "ratio" => BaselineOperation::Ratio,
        "percentchange" | "pctchange" => BaselineOperation::PercentChange,
        "fractionalchange" => BaselineOperation::FractionalChange,
        _ => return Err(EngineError::new(format!("unknown baseline operation `{op_str}`"))),
    };
    Ok(Some((col, op)))
}

fn parse_table_kind(raw: &str) -> Result<TableKind, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "xy" => Ok(TableKind::Xy),
        "column" | "col" => Ok(TableKind::Column),
        "grouped" | "twoway" => Ok(TableKind::Grouped),
        "contingency" => Ok(TableKind::Contingency),
        "survival" => Ok(TableKind::Survival),
        "partsofwhole" | "pie" => Ok(TableKind::PartsOfWhole),
        "multiplevariables" | "multivar" => Ok(TableKind::MultipleVariables),
        "nested" => Ok(TableKind::Nested),
        _ => Err(EngineError::new(format!("unknown table archetype `{raw}`"))),
    }
}

fn parse_subcolumn_mode(params: &Value, replicates: usize) -> Result<SubcolumnMode, EngineError> {
    if let Some(raw_val) = opt_val(params, &["subcolumn_mode", "subcolumnMode"]) {
        if let Some(s) = raw_val.as_str() {
            return parse_subcolumn_mode_str(s, replicates);
        }
        if let Ok(mode) = serde_json::from_value::<SubcolumnMode>(raw_val.clone()) {
            return Ok(mode);
        }
    }
    if replicates > 1 { Ok(SubcolumnMode::RawReplicates(replicates)) } else { Ok(SubcolumnMode::Single) }
}

fn parse_subcolumn_mode_str(s: &str, replicates: usize) -> Result<SubcolumnMode, EngineError> {
    let norm: String = s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "single" => Ok(SubcolumnMode::Single),
        "replicate" | "replicates" | "rawreplicates" => Ok(SubcolumnMode::RawReplicates(replicates.max(1))),
        "meansdn" => Ok(SubcolumnMode::MeanSdN),
        "meansemn" => Ok(SubcolumnMode::MeanSemN),
        "meancvn" => Ok(SubcolumnMode::MeanCvN),
        "meanlowhigh" => Ok(SubcolumnMode::MeanLowHigh),
        "upperlowerlimits" | "meanupperlower" => Ok(SubcolumnMode::UpperLowerLimits),
        "medianiqr" => Ok(SubcolumnMode::MedianIqr),
        _ => Err(EngineError::new(format!("unknown subcolumn mode `{s}`"))),
    }
}

fn parse_x_mode(params: &Value, default_replicates: usize) -> Result<XColumnMode, EngineError> {
    if let Some(raw_val) = opt_val(params, &["x_mode", "xMode"]) {
        if let Ok(mode) = serde_json::from_value::<XColumnMode>(raw_val.clone()) {
            return Ok(mode);
        }
        if let Some(s) = raw_val.as_str() {
            let norm: String = s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
            return match norm.as_str() {
                "single" | "numbers" => Ok(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }),
                "date" | "dateorelapsedtime" | "time" => {
                    Ok(XColumnMode::DateOrElapsedTime { unit_label: opt_str(params, &["x_unit", "xUnit"]).unwrap_or("days").to_owned() })
                }
                _ => Ok(XColumnMode::Numbers {
                    subcolumn_mode: parse_subcolumn_mode_str(s, opt_usize(params, &["x_replicates", "xReplicates"]).unwrap_or(default_replicates))?,
                }),
            };
        }
    }
    Ok(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single })
}

fn parse_column_design_hint(raw: &str) -> Result<ColumnDesignHint, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "unpaired" | "unpairedreplicates" => Ok(ColumnDesignHint::UnpairedReplicates),
        "paired" | "repeatedmeasures" | "pairedrepeatedmeasures" | "rm" => Ok(ColumnDesignHint::PairedRepeatedMeasures),
        "precomputederror" | "precomputed" => Ok(ColumnDesignHint::PrecomputedError),
        _ => Err(EngineError::new(format!("unknown column design hint `{raw}`"))),
    }
}

fn parse_sheet_section(raw: &str) -> Result<SheetSection, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "data" | "datatables" => Ok(SheetSection::Data),
        "info" => Ok(SheetSection::Info),
        "analyses" | "results" => Ok(SheetSection::Analyses),
        "graphs" => Ok(SheetSection::Graphs),
        "layouts" => Ok(SheetSection::Layouts),
        _ => Err(EngineError::new(format!("unknown sheet section `{raw}`"))),
    }
}

fn parse_placement_mode(raw: &str) -> Result<PlacementMode, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "overwrite" => Ok(PlacementMode::Overwrite),
        "insert" | "insertrows" => Ok(PlacementMode::InsertRows),
        "append" | "appendrows" => Ok(PlacementMode::AppendRows),
        "appendcolumns" | "appendcols" => Ok(PlacementMode::AppendColumns),
        _ => Err(EngineError::new(format!("unknown placement mode `{raw}`"))),
    }
}

fn parse_delimiter_char(raw: &str) -> Result<char, EngineError> {
    match raw {
        "," | "comma" => Ok(','),
        "\t" | "\\t" | "tab" | "tsv" => Ok('\t'),
        ";" | "semicolon" => Ok(';'),
        _ if raw.chars().count() == 1 => raw.chars().next().ok_or_else(|| EngineError::new("invalid delimiter")),
        _ => Err(EngineError::new(format!("unsupported delimiter `{raw}`"))),
    }
}

fn parse_delimiter(params: &Value, sample_text: &str) -> Result<char, EngineError> {
    if let Some(raw) = opt_str(params, &["delimiter"])
        && !raw.eq_ignore_ascii_case("auto")
    {
        return parse_delimiter_char(raw);
    }
    if sample_text.contains('\t') {
        return Ok('\t');
    }
    if sample_text.contains(';') {
        return Ok(';');
    }
    Ok(',')
}

fn parse_decimal_char_opt(params: &Value) -> Option<char> {
    let raw = opt_str(params, &["decimal_separator", "decimalSeparator", "decimal"])?;
    match raw.to_ascii_lowercase().as_str() {
        "." | "dot" | "period" => Some('.'),
        "," | "comma" => Some(','),
        _ => None,
    }
}

fn parse_decimal_char(params: &Value, sample_text: &str, delimiter: char) -> Result<char, EngineError> {
    if let Some(raw) = opt_str(params, &["decimal_separator", "decimalSeparator", "decimal"]) {
        if raw.eq_ignore_ascii_case("auto") {
            return Ok(infer_decimal_char(sample_text, delimiter));
        }
        return match raw.to_ascii_lowercase().as_str() {
            "." | "dot" | "period" => Ok('.'),
            "," | "comma" => Ok(','),
            _ => Err(EngineError::new(format!("unknown decimal separator `{raw}`"))),
        };
    }
    if opt_bool(params, &["normalize_decimal", "normalizeDecimal"]).unwrap_or(false) || delimiter != ',' {
        return Ok(infer_decimal_char(sample_text, delimiter));
    }
    Ok('.')
}

fn infer_decimal_char(sample_text: &str, delimiter: char) -> char {
    if delimiter == ',' {
        return '.';
    }
    let mut comma_nums = 0usize;
    let mut dot_nums = 0usize;
    for line in sample_text.lines().take(32) {
        for tok in line.split(delimiter) {
            let t = tok.trim().trim_matches('"').trim_matches('*').trim_start_matches(['+', '-']);
            if t.is_empty() {
                continue;
            }
            if t.contains(',') && !t.contains('.') && !looks_like_us_thousands(t) && t.replace(',', ".").parse::<f64>().is_ok() {
                comma_nums = comma_nums.saturating_add(1);
            } else if t.contains('.') && t.parse::<f64>().is_ok() {
                dot_nums = dot_nums.saturating_add(1);
            }
        }
    }
    if delimiter == ';' && dot_nums == 0 {
        return ',';
    }
    if comma_nums > dot_nums { ',' } else { '.' }
}

fn looks_like_us_thousands(token: &str) -> bool {
    let mut parts = token.split(',');
    let Some(first) = parts.next() else {
        return false;
    };
    let rest: Vec<&str> = parts.collect();
    !rest.is_empty()
        && (1..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_digit())
        && rest.iter().all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
}

fn parse_variable_role(raw: &str) -> Result<VariableRole, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "continuous" | "numeric" => Ok(VariableRole::Continuous),
        "categorical" | "category" | "factor" => Ok(VariableRole::Categorical),
        "label" | "identifier" | "id" | "text" => Ok(VariableRole::Label),
        "calculated" | "formula" => Ok(VariableRole::Calculated),
        _ => Err(EngineError::new(format!("unknown variable role `{raw}`"))),
    }
}

fn parse_recode_mappings(params: &Value) -> Result<Vec<(String, String)>, EngineError> {
    let raw = opt_val(params, &["mappings", "mapping", "map", "recode"])
        .ok_or_else(|| EngineError::new("table.recodeLevels requires `mappings` / `mapping`"))?;
    match raw {
        Value::Array(arr) => {
            let mut pairs = Vec::with_capacity(arr.len());
            for item in arr {
                let from = req_str(item, &["from", "old", "source"])?.to_owned();
                let to = req_str(item, &["to", "new", "target"])?.to_owned();
                pairs.push((from, to));
            }
            Ok(pairs)
        }
        Value::Object(map) => {
            let mut pairs = Vec::with_capacity(map.len());
            for (k, v) in map {
                let to = v.as_str().ok_or_else(|| EngineError::new("recode mapping target must be a string"))?.to_owned();
                pairs.push((k.clone(), to));
            }
            Ok(pairs)
        }
        _ => Err(EngineError::new("`mappings` must be an array of {from, to} or an object map")),
    }
}

fn parse_level_order_spec(params: &Value) -> Result<LevelOrderSpec, EngineError> {
    if let Some(spec_val) = opt_val(params, &["order_spec", "orderSpec"]) {
        if let Ok(parsed) = serde_json::from_value::<LevelOrderSpec>(spec_val.clone()) {
            return Ok(parsed);
        }
        return parse_level_order_spec(spec_val);
    }

    let reversed = opt_bool(params, &["reversed"]).unwrap_or(false);
    if let Some(levels_arr) = opt_array(params, &["levels", "explicit", "order"]) {
        return Ok(LevelOrderSpec { order: LevelOrderKind::Explicit(parse_string_array(levels_arr)?), reversed });
    }

    let kind_str = req_str(params, &["order", "kind", "mode"])?;
    let norm: String = kind_str.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let order = match norm.as_str() {
        "entryorder" | "entry" => LevelOrderKind::EntryOrder,
        "alphabetical" | "alpha" => LevelOrderKind::Alphabetical,
        "frequencydescending" | "freqdesc" => LevelOrderKind::FrequencyDescending,
        "frequencyascending" | "freqasc" => LevelOrderKind::FrequencyAscending,
        "explicit" => {
            let arr = opt_array(params, &["levels"]).ok_or_else(|| EngineError::new("explicit categorical order requires `levels` array"))?;
            LevelOrderKind::Explicit(parse_string_array(arr)?)
        }
        _ => return Err(EngineError::new(format!("unknown level order `{kind_str}`"))),
    };
    Ok(LevelOrderSpec { order, reversed })
}

fn parse_string_array(arr: &[Value]) -> Result<Vec<String>, EngineError> {
    arr.iter().map(|v| v.as_str().map(str::to_owned).ok_or_else(|| EngineError::new("expected array of strings"))).collect()
}

fn parse_simulation_parameters(params: &Value) -> Result<Vec<(String, f64)>, EngineError> {
    let Some(raw) = opt_val(params, &["parameters", "params"]) else {
        return Ok(Vec::new());
    };
    match raw {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let f = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| EngineError::new("simulation parameter value must be finite"))?;
                Ok((k.clone(), f))
            })
            .collect(),
        Value::Array(arr) => {
            let default_names = ["Bottom", "Top", "LogEC50", "HillSlope", "S", "K", "Span", "Rate", "Slope", "Intercept"];
            let mut out = Vec::with_capacity(arr.len());
            for (idx, item) in arr.iter().enumerate() {
                if let Some(num) = item.as_f64().filter(|x| x.is_finite()) {
                    let name = default_names.get(idx).copied().unwrap_or("Param").to_owned();
                    out.push((name, num));
                } else if let Some(pair) = item.as_array()
                    && pair.len() == 2
                {
                    let name = pair.first().and_then(Value::as_str).ok_or_else(|| EngineError::new("parameter tuple name must be string"))?;
                    let num = pair
                        .get(1)
                        .and_then(Value::as_f64)
                        .filter(|x| x.is_finite())
                        .ok_or_else(|| EngineError::new("parameter tuple value must be finite"))?;
                    out.push((name.to_owned(), num));
                } else if item.is_object() {
                    let name = req_str(item, &["name", "key"])?;
                    let num = opt_f64(item, &["value"]).ok_or_else(|| EngineError::new("parameter object requires finite `value`"))?;
                    out.push((name.to_owned(), num));
                } else {
                    return Err(EngineError::new("invalid simulation parameter entry"));
                }
            }
            Ok(out)
        }
        _ => Err(EngineError::new("simulation `parameters` must be an object map or array")),
    }
}

fn parse_simulation_noise(params: &Value) -> Result<SimulationNoise, EngineError> {
    let kind = opt_str(params, &["noise_kind", "noiseKind", "noise"]).unwrap_or("gaussian");
    let norm: String = kind.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "none" | "zero" => Ok(SimulationNoise::None),
        "gaussian" | "normal" => Ok(SimulationNoise::Gaussian { sd: opt_f64(params, &["noise_sd", "noiseSd", "sd"]).unwrap_or(0.0) }),
        "relative" | "relativepercent" | "cv" => {
            Ok(SimulationNoise::RelativePercent { cv_percent: opt_f64(params, &["cv_percent", "cvPercent", "noise_sd", "noiseSd"]).unwrap_or(0.0) })
        }
        "poisson" => Ok(SimulationNoise::Poisson),
        _ => Err(EngineError::new(format!("unknown simulation noise model `{kind}`"))),
    }
}

fn parse_constant_value(params: &Value) -> Result<ConstantValue, EngineError> {
    let val = params.get("value").unwrap_or(&Value::Null);
    if opt_str(params, &["kind", "type"]).is_some_and(|k| k.eq_ignore_ascii_case("date"))
        && let Some(s) = val.as_str()
    {
        return Ok(ConstantValue::Date(s.to_owned()));
    }
    match val {
        Value::Null => Ok(ConstantValue::Empty),
        Value::Number(n) => {
            let f = n.as_f64().filter(|x| x.is_finite()).ok_or_else(|| EngineError::new("constant number must be finite"))?;
            Ok(ConstantValue::Number(f))
        }
        Value::Bool(b) => Ok(ConstantValue::Text(if *b { String::from("true") } else { String::from("false") })),
        Value::String(s) => Ok(ConstantValue::Text(s.clone())),
        Value::Object(_) | Value::Array(_) => Err(EngineError::new("constant `value` must be null, number, or string")),
    }
}

fn analysis_hooks_mention_sheet(spec: &AnalysisSpec, target: &SheetId) -> bool {
    match &spec.kind {
        AnalysisKind::NonlinearRegression(nl) => nl
            .parameter_rules
            .iter()
            .filter_map(|r| match &r.scope {
                ParameterScope::HookedConstant(s) => Some(s),
                _ => None,
            })
            .any(|s| hook_mentions_sheet(s, target)),
        _ => false,
    }
}

fn analysis_hooks_mention_any(spec: &AnalysisSpec, set: &HashSet<SheetId>) -> bool {
    match &spec.kind {
        AnalysisKind::NonlinearRegression(nl) => nl
            .parameter_rules
            .iter()
            .filter_map(|r| match &r.scope {
                ParameterScope::HookedConstant(s) => Some(s),
                _ => None,
            })
            .any(|s| hook_references_any(s, set)),
        _ => false,
    }
}

fn hook_mentions_sheet(source: &HookSource, target: &SheetId) -> bool {
    match source {
        HookSource::InfoConstant { info_sheet_id: Some(iid), .. } => iid == target,
        HookSource::ResultScalar { result_sheet_id, .. } => result_sheet_id == target,
        HookSource::ColumnTitleValue { data_sheet_id, .. } => data_sheet_id == target,
        HookSource::InfoConstant { info_sheet_id: None, .. } => false,
    }
}

fn hook_references_any(source: &HookSource, set: &HashSet<SheetId>) -> bool {
    match source {
        HookSource::InfoConstant { info_sheet_id: Some(iid), .. } => set.contains(iid),
        HookSource::ResultScalar { result_sheet_id, .. } => set.contains(result_sheet_id),
        HookSource::ColumnTitleValue { data_sheet_id, .. } => set.contains(data_sheet_id),
        HookSource::InfoConstant { info_sheet_id: None, .. } => false,
    }
}

// ---------------------------------------------------------------------------
// Zero-Dependency Base64 Codec (for headless/WASM in-memory archives)
// ---------------------------------------------------------------------------

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3).saturating_mul(4));
    for chunk in input.chunks(3) {
        let b0 = usize::from(*chunk.first().unwrap_or(&0));
        let b1 = usize::from(*chunk.get(1).unwrap_or(&0));
        let b2 = usize::from(*chunk.get(2).unwrap_or(&0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(B64_ALPHABET.get((n >> 18) & 0x3f).copied().unwrap_or(b'A')));
        out.push(char::from(B64_ALPHABET.get((n >> 12) & 0x3f).copied().unwrap_or(b'A')));
        if chunk.len() > 1 {
            out.push(char::from(B64_ALPHABET.get((n >> 6) & 0x3f).copied().unwrap_or(b'=')));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(B64_ALPHABET.get(n & 0x3f).copied().unwrap_or(b'=')));
        } else {
            out.push('=');
        }
    }
    out
}

fn decode_base64(input: &str) -> Result<Vec<u8>, EngineError> {
    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !cleaned.len().is_multiple_of(4) {
        return Err(EngineError::new("invalid base64 length"));
    }
    let total_quads = cleaned.len() / 4;
    let mut out = Vec::with_capacity(total_quads * 3);
    for (quad_idx, quad) in cleaned.as_chunks::<4>().0.iter().enumerate() {
        let [c0, c1, c2, c3] = *quad;
        let v0 = b64_val(c0)?;
        let v1 = b64_val(c1)?;
        let pad2 = c2 == b'=';
        let pad3 = c3 == b'=';
        if pad2 && !pad3 {
            return Err(EngineError::new("invalid base64 padding sequence"));
        }
        if (pad2 || pad3) && quad_idx + 1 != total_quads {
            return Err(EngineError::new("invalid base64 padding before final quantum"));
        }
        let v2 = if pad2 { 0 } else { b64_val(c2)? };
        let v3 = if pad3 { 0 } else { b64_val(c3)? };
        let n = (u32::from(v0) << 18) | (u32::from(v1) << 12) | (u32::from(v2) << 6) | u32::from(v3);
        out.push(u8::try_from((n >> 16) & 0xff).unwrap_or(0));
        if !pad2 {
            out.push(u8::try_from((n >> 8) & 0xff).unwrap_or(0));
        }
        if !pad3 {
            out.push(u8::try_from(n & 0xff).unwrap_or(0));
        }
    }
    Ok(out)
}

fn b64_val(b: u8) -> Result<u8, EngineError> {
    match b {
        b'A'..=b'Z' => Ok(b - b'A'),
        b'a'..=b'z' => Ok(b - b'a' + 26),
        b'0'..=b'9' => Ok(b - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(EngineError::new("invalid base64 character")),
    }
}

fn parse_byte_array(arr: &[Value]) -> Result<Vec<u8>, EngineError> {
    arr.iter()
        .map(|v| {
            let n = v.as_u64().ok_or_else(|| EngineError::new("byte array entries must be integers"))?;
            u8::try_from(n).map_err(|_| EngineError::new("byte array entry out of 0..=255 range"))
        })
        .collect()
}
