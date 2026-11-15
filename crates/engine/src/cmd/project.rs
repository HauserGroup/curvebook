//! Project lifecycle, file I/O, template, inspection, recalculation, and checkpoint commands.

use std::collections::HashSet;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, DataSetColumn, DataSheet, ErrorCode, ImportLinkSpec, InternalLinkKind, LinkSource, NamedCheckpoint, ParameterScope,
    ProjectDocument, ResultSheet, SheetId, TableArchetype,
};
use lumencraft_format::{
    instantiate_template_bytes, open_project_bytes, open_project_file, open_project_unpacked_dir, save_project_bytes, save_project_unpacked_dir,
    save_template_bytes,
};
use lumencraft_transform::{ImportPipelineOptions, ImportSourcePayload, apply_import_link_pipeline, evaluate_data_sheet_calculated_variables};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cmd::helpers::{decode_base64, encode_base64, locate_target_and_info_indices, parse_sheet_id};
use crate::cmd::params::{
    bool_schema, empty_object_schema, non_empty_str_schema, nullable_schema, obj_schema_with_aliases, parse_params, str_schema,
};
use crate::cmd::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::{CommandOutcome, EngineError, ExternalPathPolicy, Session, dag};

/// Summary returned by [`recalculate_with_links`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RecalcSummary {
    pub recalculated_sheets: usize,
    pub refreshed_links: usize,
}

enum OwnedSourcePayload {
    Data(Box<DataSheet>),
    Result(Box<ResultSheet>),
    Delimited { text: String, delimiter: char, decimal_separator: char },
}

fn always_enabled(_session: &Session) -> bool {
    true
}

/// Refresh internal/external linked regions and run full DAG recalculation under an explicit policy.
pub(super) fn recalculate_with_links_under_policy(
    doc: &mut ProjectDocument,
    include_external_links: bool,
    policy: &ExternalPathPolicy,
) -> Result<RecalcSummary, EngineError> {
    let mut refreshed = refresh_linked_regions(doc, include_external_links, policy)?;
    let mut recalculated = dag::recalculate_project(doc)?;
    let extra_refreshed = refresh_linked_regions(doc, false, policy)?;
    refreshed = refreshed.saturating_add(extra_refreshed);
    if extra_refreshed > 0 {
        recalculated = recalculated.saturating_add(dag::recalculate_project(doc)?);
    }
    Ok(RecalcSummary { recalculated_sheets: recalculated, refreshed_links: refreshed })
}

/// Refresh internal linked regions and run full DAG recalculation (no external file refreshes).
pub(super) fn recalculate_with_links(doc: &mut ProjectDocument, include_external_links: bool) -> Result<RecalcSummary, EngineError> {
    recalculate_with_links_under_policy(doc, include_external_links, &ExternalPathPolicy::Unconstrained)
}

fn refresh_linked_regions(doc: &mut ProjectDocument, include_external_links: bool, policy: &ExternalPathPolicy) -> Result<usize, EngineError> {
    let target_ids: Vec<SheetId> = doc.data_sheets.iter().filter(|ds| !ds.frozen && !ds.linked_regions.is_empty()).map(|ds| ds.id.clone()).collect();
    let mut refreshed = 0usize;

    for target_id in target_ids {
        refreshed = refreshed.saturating_add(refresh_single_sheet_links(doc, &target_id, include_external_links, policy)?);
    }

    Ok(refreshed)
}

fn refresh_single_sheet_links(
    doc: &mut ProjectDocument,
    target_id: &SheetId,
    include_external_links: bool,
    policy: &ExternalPathPolicy,
) -> Result<usize, EngineError> {
    let regions = doc.data_sheet(target_id).map(|ds| ds.linked_regions.clone()).unwrap_or_default();
    let mut count = 0usize;

    for region in regions {
        let Some(payload) = load_link_source_payload(doc, &region.spec.source, include_external_links, policy)? else {
            continue;
        };
        let opts = ImportPipelineOptions {
            link_id: Some(region.id.clone()),
            provenance_summary: Some(region.provenance_summary.clone()),
            ..ImportPipelineOptions::default()
        };
        let info_id = doc.data_sheet(target_id).and_then(|ds| ds.info_sheet_id.clone());
        let (target_idx, info_idx) = locate_target_and_info_indices(doc, target_id, info_id.as_ref())?;
        let info_snapshot = info_idx.and_then(|idx| doc.info_sheets.get(idx)).cloned();
        apply_owned_link_refresh(doc, target_idx, &region.spec, &payload, &opts)?;
        if let Some(target_ds) = doc.data_sheets.get_mut(target_idx) {
            evaluate_data_sheet_calculated_variables(target_ds, info_snapshot.as_ref())?;
        }
        count = count.saturating_add(1);
    }

    Ok(count)
}

fn load_link_source_payload(
    doc: &ProjectDocument,
    source: &LinkSource,
    include_external_links: bool,
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
            if !include_external_links {
                return Ok(None);
            }
            let text = policy.read_utf8_file(path, "linked file")?;
            Ok(Some(OwnedSourcePayload::Delimited { text, delimiter: *delimiter, decimal_separator: *decimal_separator }))
        }
    }
}

fn apply_owned_link_refresh(
    doc: &mut ProjectDocument,
    target_idx: usize,
    spec: &ImportLinkSpec,
    payload: &OwnedSourcePayload,
    opts: &ImportPipelineOptions,
) -> Result<(), EngineError> {
    let target_ds =
        doc.data_sheets.get_mut(target_idx).ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, "target data sheet index out of bounds"))?;
    match payload {
        OwnedSourcePayload::Data(src) => {
            let _ = apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalData(src), opts, None)?;
        }
        OwnedSourcePayload::Result(src) => {
            let _ = apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalResult(src), opts, None)?;
        }
        OwnedSourcePayload::Delimited { text, delimiter, decimal_separator } => {
            let src = ImportSourcePayload::DelimitedText { text, delimiter: *delimiter, decimal_separator: *decimal_separator };
            let _ = apply_import_link_pipeline(target_ds, spec, src, opts, None)?;
        }
    }
    propagate_internal_source_titles(target_ds, spec, payload);
    Ok(())
}

fn propagate_internal_source_titles(target_ds: &mut DataSheet, spec: &ImportLinkSpec, payload: &OwnedSourcePayload) {
    if !spec.placement.propagate_title_header {
        return;
    }
    let OwnedSourcePayload::Data(src) = payload else {
        return;
    };
    let base_col = spec.placement.anchor.col;
    let LinkSource::InternalSheet { range, .. } = &spec.source else {
        return;
    };

    let titles: Vec<String> = if spec.placement.transpose {
        let r0 = spec.filter.row_window.map_or(range.start.row, |(lo, _)| lo.max(range.start.row));
        let r1 = spec.filter.row_window.map_or(range.end.row, |(_, hi)| hi.min(range.end.row));
        (r0..=r1).filter_map(|r| source_row_title(&src.archetype, r)).collect()
    } else {
        let c0 = spec.filter.col_window.map_or(range.start.col, |(lo, _)| lo.max(range.start.col));
        let c1 = spec.filter.col_window.map_or(range.end.col, |(_, hi)| hi.min(range.end.col));
        (c0..=c1).filter_map(|c| source_col_title(&src.archetype, c)).collect()
    };

    for (offset, title) in titles.into_iter().enumerate() {
        if !title.is_empty() {
            set_target_group_title(&mut target_ds.archetype, base_col.saturating_add(offset), &title);
        }
    }
}

fn source_col_title(arch: &TableArchetype, col: usize) -> Option<String> {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Column(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map(|d| d.title.clone()),
        TableArchetype::Survival(t) => t.cohorts.get(col).map(|d| d.title.clone()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col).map(|v| v.title.clone()),
        TableArchetype::Nested(t) => t.groups.get(col).map(|g| g.title.clone()),
    }
}

fn source_row_title(arch: &TableArchetype, row: usize) -> Option<String> {
    let titles = match arch {
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    };
    titles.get(row).cloned()
}

fn set_target_group_title(arch: &mut TableArchetype, col: usize, title: &str) {
    match arch {
        TableArchetype::Xy(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Column(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Grouped(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Contingency(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::PartsOfWhole(t) => assign_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Survival(t) => assign_ds_title(&mut t.cohorts, col, title),
        TableArchetype::MultipleVariables(t) => {
            if let Some(var) = t.variables.get_mut(col) {
                var.title = title.to_owned();
            }
        }
        TableArchetype::Nested(t) => {
            if let Some(grp) = t.groups.get_mut(col) {
                grp.title = title.to_owned();
            }
        }
    }
}

fn assign_ds_title(sets: &mut [DataSetColumn], col: usize, title: &str) {
    if let Some(ds) = sets.get_mut(col) {
        ds.title = title.to_owned();
    }
}

/// Format current system wall-clock time as `"unix:<seconds>"`.
#[must_use]
pub(super) fn current_utc_timestamp() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        String::from("unix:0")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0_u64, |dur| dur.as_secs());
        format!("unix:{secs}")
    }
}

#[derive(Debug, Deserialize)]
struct NewProjectParams {
    #[serde(alias = "name")]
    title: Option<String>,
}

fn new_project_schema() -> Value {
    obj_schema_with_aliases(
        vec![("title", nullable_schema(str_schema("Title for the new project (defaults to `Untitled Project`)")))],
        &[],
        vec![("title", &["name"])],
        true,
    )
}

fn run_file_new(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    run_new_project_with_id(ctx, "file.new", params)
}

fn run_project_new(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    run_new_project_with_id(ctx, "project.new", params)
}

fn run_new_project_with_id(ctx: &mut Ctx<'_>, command_id: &str, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: NewProjectParams = parse_params(&new_project_schema(), params)?;
    let title = parsed.title.unwrap_or_else(|| String::from("Untitled Project"));
    let doc = ProjectDocument::new(title)?;
    let result_title = doc.title.clone();
    ctx.session.doc = doc;
    Ok(CommandOutcome::mutated(command_id, "created new project", json!({ "title": result_title })))
}

#[derive(Debug, Deserialize)]
struct OpenProjectParams {
    #[serde(alias = "file_path", alias = "filePath")]
    path: Option<String>,
    #[serde(alias = "bytesBase64", alias = "archive_base64")]
    bytes_base64: Option<String>,
    #[serde(alias = "unpacked_dir", alias = "unpackedDir")]
    unpacked: Option<bool>,
}

fn open_project_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("path", nullable_schema(str_schema("Filesystem path to `.lumencraft` archive or `.lumencraft.d/` directory"))),
            ("bytes_base64", nullable_schema(str_schema("Base64-encoded `.lumencraft` zip archive bytes"))),
            ("unpacked", nullable_schema(bool_schema("Treat `path` as an unpacked `.lumencraft.d/` project directory"))),
        ],
        &[],
        vec![
            ("path", &["file_path", "filePath"]),
            ("bytes_base64", &["bytesBase64", "archive_base64"]),
            ("unpacked", &["unpacked_dir", "unpackedDir"]),
        ],
        true,
    )
}

fn run_project_open(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: OpenProjectParams = parse_params(&open_project_schema(), params)?;
    let explicit_unpacked = parsed.unpacked.unwrap_or(false);
    let (loaded, used_unpacked) = if let Some(ref path) = parsed.path {
        if path.trim().is_empty() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "project.open `path` must not be empty"));
        }
        let (resolved, use_unpacked) = ctx.session.path_policy.resolve_project_open_target(path, explicit_unpacked)?;
        let opened = if use_unpacked { open_project_unpacked_dir(&resolved)? } else { open_project_file(&resolved)? };
        (opened, use_unpacked)
    } else if explicit_unpacked {
        return Err(EngineError::with_code(ErrorCode::BadInput, "project.open with `unpacked: true` requires `path`")
            .with_remediation("Provide a directory path in `path` when opening an unpacked project."));
    } else if let Some(ref b64) = parsed.bytes_base64 {
        let bytes = decode_base64(b64)?;
        (open_project_bytes(&bytes)?, false)
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "project.open requires `path` or `bytes_base64`")
            .with_remediation("Pass either `path` or `bytes_base64`."));
    };

    ctx.session.doc = loaded;
    Ok(CommandOutcome::mutated(
        "project.open",
        "opened project",
        json!({
            "title": ctx.session.doc.title,
            "schema_version": ctx.session.doc.schema_version,
            "sheet_count": ctx.session.doc.total_sheet_count(),
            "checkpoint_count": ctx.session.doc.checkpoints.len(),
            "unpacked": used_unpacked,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct SaveProjectParams {
    #[serde(alias = "file_path", alias = "filePath")]
    path: Option<String>,
    #[serde(alias = "returnBytes", alias = "include_bytes", alias = "includeBytes")]
    return_bytes: Option<bool>,
    #[serde(alias = "unpacked_dir", alias = "unpackedDir")]
    unpacked: Option<bool>,
}

fn save_project_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("path", nullable_schema(str_schema("Destination `.lumencraft` archive path or `.lumencraft.d/` directory"))),
            (
                "return_bytes",
                nullable_schema(bool_schema("Include base64-encoded `.lumencraft` bytes in response (defaults to true when `path` is omitted)")),
            ),
            ("unpacked", nullable_schema(bool_schema("Save as an unpacked `.lumencraft.d/` project directory"))),
        ],
        &[],
        vec![
            ("path", &["file_path", "filePath"]),
            ("return_bytes", &["returnBytes", "include_bytes", "includeBytes"]),
            ("unpacked", &["unpacked_dir", "unpackedDir"]),
        ],
        true,
    )
}

fn run_project_save(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: SaveProjectParams = parse_params(&save_project_schema(), params)?;
    let explicit_unpacked = parsed.unpacked.unwrap_or(false);
    if explicit_unpacked && parsed.path.is_none() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "project.save with `unpacked: true` requires `path`")
            .with_remediation("Provide a target directory `path` when saving unpacked format."));
    }
    let return_bytes = parsed.return_bytes.unwrap_or(parsed.path.is_none());

    let (used_unpacked, archive_bytes) = if let Some(ref path) = parsed.path {
        if path.trim().is_empty() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "project.save `path` must not be empty"));
        }
        let (resolved, use_unpacked) = ctx.session.path_policy.resolve_project_save_target(path, explicit_unpacked)?;
        if use_unpacked {
            save_project_unpacked_dir(&ctx.session.doc, &resolved)?;
            let bytes = if return_bytes { save_project_bytes(&ctx.session.doc)? } else { Vec::new() };
            (true, bytes)
        } else {
            let bytes = save_project_bytes(&ctx.session.doc)?;
            ctx.session.path_policy.write_atomic_bytes(path, &bytes)?;
            (false, bytes)
        }
    } else {
        let bytes = save_project_bytes(&ctx.session.doc)?;
        (false, bytes)
    };

    let bytes_len = archive_bytes.len();
    let encoded = if return_bytes { Some(encode_base64(&archive_bytes)) } else { None };
    Ok(CommandOutcome::read_only(
        "project.save",
        "saved project",
        json!({
            "title": ctx.session.doc.title,
            "path": parsed.path,
            "unpacked": used_unpacked,
            "bytes_len": bytes_len,
            "bytes_base64": encoded,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct SaveTemplateParams {
    #[serde(alias = "file_path", alias = "filePath")]
    path: Option<String>,
    #[serde(alias = "returnBytes", alias = "include_bytes", alias = "includeBytes")]
    return_bytes: Option<bool>,
}

fn save_template_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("path", nullable_schema(str_schema("Destination `.lctemplate` file path"))),
            (
                "return_bytes",
                nullable_schema(bool_schema("Include base64-encoded template bytes in response (defaults to true when `path` is omitted)")),
            ),
        ],
        &[],
        vec![("path", &["file_path", "filePath"]), ("return_bytes", &["returnBytes", "include_bytes", "includeBytes"])],
        true,
    )
}

fn run_project_save_template(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: SaveTemplateParams = parse_params(&save_template_schema(), params)?;
    let return_bytes = parsed.return_bytes.unwrap_or(parsed.path.is_none());
    let bytes = save_template_bytes(&ctx.session.doc)?;
    if let Some(ref path) = parsed.path {
        ctx.session.path_policy.write_atomic_bytes(path, &bytes)?;
    }
    let encoded = if return_bytes { Some(encode_base64(&bytes)) } else { None };
    Ok(CommandOutcome::read_only(
        "project.saveTemplate",
        "saved project template",
        json!({
            "title": ctx.session.doc.title,
            "path": parsed.path,
            "bytes_len": bytes.len(),
            "bytes_base64": encoded,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct InstantiateTemplateParams {
    #[serde(alias = "file_path", alias = "filePath", alias = "template_path", alias = "templatePath")]
    path: Option<String>,
    #[serde(alias = "bytesBase64", alias = "archive_base64")]
    bytes_base64: Option<String>,
    #[serde(alias = "project_title", alias = "projectTitle")]
    title: Option<String>,
}

fn instantiate_template_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("path", nullable_schema(str_schema("Path to `.lctemplate` archive file"))),
            ("bytes_base64", nullable_schema(str_schema("Base64-encoded `.lctemplate` archive bytes"))),
            ("title", nullable_schema(str_schema("Optional title override for the instantiated project"))),
        ],
        &[],
        vec![
            ("path", &["file_path", "filePath", "template_path", "templatePath"]),
            ("bytes_base64", &["bytesBase64", "archive_base64"]),
            ("title", &["project_title", "projectTitle"]),
        ],
        true,
    )
}

fn run_project_instantiate_template(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: InstantiateTemplateParams = parse_params(&instantiate_template_schema(), params)?;
    let bytes = if let Some(ref path) = parsed.path {
        ctx.session.path_policy.read_binary_file(path, "template file")?
    } else if let Some(ref b64) = parsed.bytes_base64 {
        decode_base64(b64)?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "project.instantiateTemplate requires `path` or `bytes_base64`")
            .with_remediation("Pass either `path` or `bytes_base64` pointing to a template archive."));
    };

    let instantiated = instantiate_template_bytes(&bytes, parsed.title.as_deref())?;
    ctx.session.doc = instantiated;
    Ok(CommandOutcome::mutated(
        "project.instantiateTemplate",
        "instantiated project from template",
        json!({
            "title": ctx.session.doc.title,
            "schema_version": ctx.session.doc.schema_version,
            "sheet_count": ctx.session.doc.total_sheet_count(),
            "data_sheets": ctx.session.doc.data_sheets.len(),
            "result_sheets": ctx.session.doc.result_sheets.len(),
            "graph_sheets": ctx.session.doc.graph_sheets.len(),
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct InspectProjectParams {
    #[serde(alias = "sheetId", alias = "id")]
    sheet_id: Option<String>,
}

fn inspect_project_schema() -> Value {
    obj_schema_with_aliases(
        vec![("sheet_id", nullable_schema(str_schema("Optional sheet ID to focus dependency and section inspection on")))],
        &[],
        vec![("sheet_id", &["sheetId", "id"])],
        true,
    )
}

fn run_project_inspect(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: InspectProjectParams = parse_params(&inspect_project_schema(), params)?;
    let doc = &ctx.session.doc;

    let focus_info = if let Some(focus_str) = parsed.sheet_id {
        let focus_id = parse_sheet_id(&focus_str)?;
        let section = doc
            .sheet_section(&focus_id)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", focus_id.as_str())))?;
        let upstream = doc.upstream_dependencies(&focus_id);
        let upstream_strs: Vec<&str> = upstream.iter().map(SheetId::as_str).collect();
        Some(json!({
            "sheet_id": focus_id.as_str(),
            "section": format!("{section:?}"),
            "upstream_dependencies": upstream_strs,
        }))
    } else {
        None
    };

    let data_sheets: Vec<Value> = doc
        .data_sheets
        .iter()
        .map(|ds| {
            json!({
                "id": ds.id.as_str(),
                "title": ds.title,
                "kind": format!("{:?}", ds.kind()),
                "frozen": ds.frozen,
                "rows": ds.archetype.row_count(),
                "cols": ds.archetype.column_count(),
                "allocated_cells": ds.archetype.allocated_cell_slots(),
                "info_sheet_id": ds.info_sheet_id.as_ref().map(SheetId::as_str),
                "linked_regions": ds.linked_regions.len(),
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
                "notes_len": info.notes_markdown.len(),
                "constants": info.constants.iter().map(|c| json!({ "name": c.name, "value": &c.value, "units": c.units })).collect::<Vec<_>>(),
            })
        })
        .collect();

    let result_sheets: Vec<Value> = doc
        .result_sheets
        .iter()
        .map(|rs| {
            json!({
                "id": rs.id.as_str(),
                "source_sheet_id": rs.analysis.source_sheet_id.as_str(),
                "title": rs.analysis.effective_title(),
                "status": format!("{:?}", rs.status),
                "hook_count": count_analysis_hooks(&rs.analysis),
                "tabs": rs.tabs.len(),
                "scalars": rs.scalars.len(),
                "analyzable": rs.analyzable_data.is_some(),
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
                "initialized": gs.initialized,
                "kind": format!("{:?}", gs.spec.kind),
                "primary_source_sheet_id": gs.spec.primary_source_sheet_id.as_str(),
                "series_count": gs.spec.series.len(),
            })
        })
        .collect();

    let layout_sheets: Vec<Value> =
        doc.layout_sheets.iter().map(|ls| json!({ "id": ls.id.as_str(), "title": ls.title, "item_count": ls.spec.items.len() })).collect();

    Ok(CommandOutcome::read_only(
        "project.inspect",
        "inspected project",
        json!({
            "schema_version": doc.schema_version,
            "title": doc.title,
            "total_sheet_count": doc.total_sheet_count(),
            "active_sheet_id": doc.active_sheet_id.as_ref().map(SheetId::as_str),
            "family_anchor_sheet_id": doc.family_anchor_sheet_id.as_ref().map(SheetId::as_str),
            "checkpoints": doc.checkpoints.len(),
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

#[derive(Debug, Deserialize)]
struct RecalculateParams {
    #[serde(alias = "includeExternalLinks")]
    include_external_links: Option<bool>,
}

fn recalculate_schema() -> Value {
    obj_schema_with_aliases(
        vec![(
            "include_external_links",
            nullable_schema(bool_schema("Refresh external delimited-file links before recalculating downstream analyses (default true)")),
        )],
        &[],
        vec![("include_external_links", &["includeExternalLinks"])],
        true,
    )
}

fn run_project_recalculate(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: RecalculateParams = parse_params(&recalculate_schema(), params)?;
    let include_external = parsed.include_external_links.unwrap_or(true);
    let recalc_policy = ctx.session.path_policy.clone();
    let summary = recalculate_with_links_under_policy(&mut ctx.session.doc, include_external, &recalc_policy)?;
    Ok(CommandOutcome::mutated(
        "project.recalculate",
        "recalculated project",
        json!({
            "recalculated_sheets": summary.recalculated_sheets,
            "refreshed_links": summary.refreshed_links,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct CreateCheckpointParams {
    #[serde(alias = "label", alias = "title")]
    name: String,
    #[serde(alias = "checkpoint_id", alias = "checkpointId")]
    id: Option<String>,
    #[serde(alias = "description")]
    note: Option<String>,
    #[serde(alias = "timestampUtc", alias = "created_utc", alias = "createdUtc")]
    timestamp_utc: Option<String>,
}

fn create_checkpoint_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("name", non_empty_str_schema("Human-readable checkpoint name")),
            ("id", nullable_schema(non_empty_str_schema("Optional unique checkpoint ID (auto-generated if omitted)"))),
            ("note", nullable_schema(str_schema("Optional checkpoint description or note"))),
            ("timestamp_utc", nullable_schema(str_schema("Optional explicit ISO-8601 UTC timestamp"))),
        ],
        &["name"],
        vec![
            ("name", &["label", "title"]),
            ("id", &["checkpoint_id", "checkpointId"]),
            ("note", &["description"]),
            ("timestamp_utc", &["timestampUtc", "created_utc", "createdUtc"]),
        ],
        true,
    )
}

fn next_checkpoint_id(doc: &ProjectDocument) -> String {
    let existing: HashSet<&str> = doc.checkpoints.iter().map(|cp| cp.id.as_str()).collect();
    let mut seq = doc.checkpoints.len().saturating_add(1).max(1);
    loop {
        let candidate = format!("cp-{seq}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
        seq = seq.saturating_add(1);
    }
}

fn run_project_create_checkpoint(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: CreateCheckpointParams = parse_params(&create_checkpoint_schema(), params)?;
    let name = parsed.name.trim().to_owned();
    if name.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "checkpoint `name` must not be empty"));
    }

    let id = match parsed.id {
        Some(raw) => {
            let trimmed = raw.trim().to_owned();
            if trimmed.is_empty() {
                return Err(EngineError::with_code(ErrorCode::BadInput, "checkpoint `id` must not be empty"));
            }
            trimmed
        }
        None => next_checkpoint_id(&ctx.session.doc),
    };

    if ctx.session.doc.checkpoints.iter().any(|cp| cp.id == id) {
        return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("checkpoint `{id}` already exists"))
            .with_remediation("Choose a distinct `id` or omit it to auto-assign."));
    }

    let note = parsed.note.unwrap_or_default();
    let created_utc = match parsed.timestamp_utc {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`timestamp_utc` cannot be empty"));
            }
            trimmed.to_owned()
        }
        None => current_utc_timestamp(),
    };

    let mut snapshot_doc = ctx.session.doc.clone();
    snapshot_doc.checkpoints.clear();
    let snapshot_json = serde_json::to_string(&snapshot_doc)
        .map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed serializing checkpoint snapshot: {err}")))?;

    ctx.session.doc.checkpoints.push(NamedCheckpoint {
        id: id.clone(),
        name: name.clone(),
        note: note.clone(),
        created_utc: created_utc.clone(),
        snapshot_json,
    });

    Ok(CommandOutcome::mutated(
        "project.createCheckpoint",
        "created checkpoint",
        json!({
            "id": id,
            "name": name,
            "note": note,
            "created_utc": created_utc,
            "checkpoint_count": ctx.session.doc.checkpoints.len(),
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct ListCheckpointsParams {}

fn run_project_list_checkpoints(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let _parsed: ListCheckpointsParams = parse_params(&empty_object_schema(), params)?;
    let entries: Vec<Value> = ctx
        .session
        .doc
        .checkpoints
        .iter()
        .map(|cp| {
            json!({
                "id": cp.id,
                "name": cp.name,
                "note": cp.note,
                "created_utc": cp.created_utc,
            })
        })
        .collect();
    Ok(CommandOutcome::read_only(
        "project.listCheckpoints",
        "listed checkpoints",
        json!({
            "count": entries.len(),
            "checkpoints": entries,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct CheckpointRefParams {
    #[serde(alias = "checkpoint_id", alias = "checkpointId")]
    id: Option<String>,
    #[serde(alias = "checkpoint_name", alias = "checkpointName", alias = "title")]
    name: Option<String>,
}

fn checkpoint_ref_schema() -> Value {
    obj_schema_with_aliases(
        vec![("id", nullable_schema(str_schema("Checkpoint ID to locate"))), ("name", nullable_schema(str_schema("Checkpoint name to locate")))],
        &[],
        vec![("id", &["checkpoint_id", "checkpointId"]), ("name", &["checkpoint_name", "checkpointName", "title"])],
        true,
    )
}

fn find_checkpoint_index(doc: &ProjectDocument, command_id: &str, params: &CheckpointRefParams) -> Result<usize, EngineError> {
    if let Some(id) = params.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return doc
            .checkpoints
            .iter()
            .position(|cp| cp.id == id)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("checkpoint id `{id}` not found")));
    }
    if let Some(name) = params.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return doc
            .checkpoints
            .iter()
            .position(|cp| cp.name == name)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("checkpoint `{name}` not found")));
    }
    Err(EngineError::with_code(ErrorCode::BadInput, format!("`{command_id}` requires `id` or `name`"))
        .with_remediation("Provide a non-empty checkpoint `id` or `name`."))
}

fn run_project_restore_checkpoint(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: CheckpointRefParams = parse_params(&checkpoint_ref_schema(), params)?;
    let idx = find_checkpoint_index(&ctx.session.doc, "project.restoreCheckpoint", &parsed)?;
    let cp = ctx.session.doc.checkpoints[idx].clone();
    let mut restored: ProjectDocument = serde_json::from_str(&cp.snapshot_json)
        .map_err(|err| EngineError::with_code(ErrorCode::CorruptData, format!("corrupt checkpoint `{}` snapshot: {err}", cp.id)))?;
    restored.checkpoints.clone_from(&ctx.session.doc.checkpoints);
    ctx.session.doc = restored;

    Ok(CommandOutcome::mutated(
        "project.restoreCheckpoint",
        "restored checkpoint",
        json!({
            "id": cp.id,
            "name": cp.name,
            "created_utc": cp.created_utc,
            "title": ctx.session.doc.title,
            "sheet_count": ctx.session.doc.total_sheet_count(),
            "checkpoint_count": ctx.session.doc.checkpoints.len(),
        }),
    ))
}

fn run_project_delete_checkpoint(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: CheckpointRefParams = parse_params(&checkpoint_ref_schema(), params)?;
    let idx = find_checkpoint_index(&ctx.session.doc, "project.deleteCheckpoint", &parsed)?;
    let removed = ctx.session.doc.checkpoints.remove(idx);

    Ok(CommandOutcome::mutated(
        "project.deleteCheckpoint",
        "deleted checkpoint",
        json!({
            "id": removed.id,
            "name": removed.name,
            "remaining": ctx.session.doc.checkpoints.len(),
        }),
    ))
}

static SPECS: [CommandSpec; 12] = [
    CommandSpec {
        id: "file.new",
        label: "New Project",
        menu_path: "File/New Project",
        shortcut: "Cmd+N",
        summary: "Create a fresh empty LumenCraft project.",
        beginner: "Replaces the active workspace with an empty project document (preserving undo history in this session).",
        group: CommandGroup::Project,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: new_project_schema,
        enabled: always_enabled,
        run: run_file_new,
    },
    CommandSpec {
        id: "project.new",
        label: "New Project",
        menu_path: "File/New Project",
        shortcut: "",
        summary: "Create a fresh empty LumenCraft project.",
        beginner: "Creates a clean empty project document with an optional title; alias for `file.new`.",
        group: CommandGroup::Project,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: new_project_schema,
        enabled: always_enabled,
        run: run_project_new,
    },
    CommandSpec {
        id: "project.open",
        label: "Open Project",
        menu_path: "File/Open...",
        shortcut: "Cmd+O",
        summary: "Load a `.lumencraft` archive or unpacked `.lumencraft.d/` directory.",
        beginner: "Opens a project from a filesystem path or base64 archive payload and replaces the current session document.",
        group: CommandGroup::Project,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: open_project_schema,
        enabled: always_enabled,
        run: run_project_open,
    },
    CommandSpec {
        id: "project.save",
        label: "Save Project",
        menu_path: "File/Save",
        shortcut: "Cmd+S",
        summary: "Save the active project atomically to `.lumencraft`, unpacked directory, or bytes.",
        beginner: "Serializes the project document using atomic filesystem writes when a path is provided, or returns base64 bytes.",
        group: CommandGroup::Project,
        effects: Effects::WRITES_FILESYSTEM,
        input_schema: save_project_schema,
        enabled: always_enabled,
        run: run_project_save,
    },
    CommandSpec {
        id: "project.saveTemplate",
        label: "Save Template",
        menu_path: "File/Save Template...",
        shortcut: "",
        summary: "Export the project structure as a data-stripped `.lctemplate` file.",
        beginner: "Strips raw data values while preserving table schemas, analyses, graphs, and layouts, then writes atomically to disk or base64.",
        group: CommandGroup::Project,
        effects: Effects::WRITES_FILESYSTEM,
        input_schema: save_template_schema,
        enabled: always_enabled,
        run: run_project_save_template,
    },
    CommandSpec {
        id: "project.instantiateTemplate",
        label: "New Project From Template",
        menu_path: "File/New From Template...",
        shortcut: "",
        summary: "Create a new project from a `.lctemplate` archive.",
        beginner: "Loads a saved template with its analysis pipelines, graphs, and layouts ready for new experimental data.",
        group: CommandGroup::Project,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: instantiate_template_schema,
        enabled: always_enabled,
        run: run_project_instantiate_template,
    },
    CommandSpec {
        id: "project.inspect",
        label: "Inspect Project",
        menu_path: "View/Inspect Project",
        shortcut: "",
        summary: "Inspect sheet structure, metadata, and upstream dependency edges.",
        beginner: "Returns a structured read-only summary of all sheets, constants, checkpoints, and optional single-sheet dependencies.",
        group: CommandGroup::Project,
        effects: Effects::IDEMPOTENT,
        input_schema: inspect_project_schema,
        enabled: always_enabled,
        run: run_project_inspect,
    },
    CommandSpec {
        id: "project.recalculate",
        label: "Recalculate All",
        menu_path: "Analyze/Recalculate All",
        shortcut: "",
        summary: "Refresh live links and recalculate all downstream analyses and graphs.",
        beginner: "Re-evaluates linked regions, calculated variables, statistical analyses, curve fits, and derived tables in dependency order.",
        group: CommandGroup::Project,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::IDEMPOTENT),
        input_schema: recalculate_schema,
        enabled: always_enabled,
        run: run_project_recalculate,
    },
    CommandSpec {
        id: "project.createCheckpoint",
        label: "Create Checkpoint",
        menu_path: "File/Checkpoints/Create Checkpoint...",
        shortcut: "",
        summary: "Save a named project snapshot checkpoint inside the document.",
        beginner: "Captures a named milestone copy of all sheets so you can restore or branch back to it later.",
        group: CommandGroup::Project,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: create_checkpoint_schema,
        enabled: always_enabled,
        run: run_project_create_checkpoint,
    },
    CommandSpec {
        id: "project.listCheckpoints",
        label: "List Checkpoints",
        menu_path: "File/Checkpoints/List Checkpoints",
        shortcut: "",
        summary: "List named checkpoints stored in the project.",
        beginner: "Lists each checkpoint's ID, name, note, and UTC timestamp without dumping full snapshot payloads.",
        group: CommandGroup::Project,
        effects: Effects::IDEMPOTENT,
        input_schema: empty_object_schema,
        enabled: always_enabled,
        run: run_project_list_checkpoints,
    },
    CommandSpec {
        id: "project.restoreCheckpoint",
        label: "Restore Checkpoint",
        menu_path: "File/Checkpoints/Restore Checkpoint...",
        shortcut: "",
        summary: "Restore the document to a previously saved named checkpoint.",
        beginner: "Replaces the current sheets with the chosen checkpoint snapshot while keeping existing checkpoints intact.",
        group: CommandGroup::Project,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: checkpoint_ref_schema,
        enabled: always_enabled,
        run: run_project_restore_checkpoint,
    },
    CommandSpec {
        id: "project.deleteCheckpoint",
        label: "Delete Checkpoint",
        menu_path: "File/Checkpoints/Delete Checkpoint...",
        shortcut: "",
        summary: "Remove a named checkpoint from the project.",
        beginner: "Deletes a stored checkpoint by ID or name without altering active data sheets.",
        group: CommandGroup::Project,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: checkpoint_ref_schema,
        enabled: always_enabled,
        run: run_project_delete_checkpoint,
    },
];

/// Registered command specifications for the `project` group.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    &SPECS
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use lumencraft_doc::{DataSheet, InfoSheet, TableArchetype, XyTable};
    use serde_json::json;

    use crate::cmd::Caller;

    use super::*;

    fn find_spec(id: &str) -> &'static CommandSpec {
        specs().iter().find(|s| s.id == id).unwrap()
    }

    fn new_session(title: &str) -> Session {
        Session::from_document(ProjectDocument::new(title).unwrap())
    }

    fn exec_cmd(session: &mut Session, id: &str, caller: Caller, params: Value) -> Result<CommandOutcome, EngineError> {
        let cancel = AtomicBool::new(false);
        session.execute_registered_spec(find_spec(id), params, caller, &cancel)
    }

    #[test]
    fn new_inspect_save_open_template_and_recalculate_round_trip() {
        let mut session = new_session("Initial");

        let r1 = exec_cmd(&mut session, "file.new", Caller::Ui, json!({ "name": "Created Via File" })).unwrap();
        assert!(r1.mutated);
        assert_eq!(session.document().title, "Created Via File");

        let r2 = exec_cmd(&mut session, "project.new", Caller::Mcp, json!({ "title": "Study Alpha" })).unwrap();
        assert!(r2.mutated);
        assert_eq!(session.document().title, "Study Alpha");

        let info = InfoSheet::new_preseeded("info-1", "Info").unwrap();
        session.doc.add_info_sheet(info).unwrap();
        let xy = XyTable::new(lumencraft_doc::SubcolumnMode::Single, 1, 3).unwrap();
        let mut ds = DataSheet::new("data-1", "Dose", TableArchetype::Xy(xy)).unwrap();
        ds.info_sheet_id = Some(SheetId::new("info-1"));
        session.doc.add_data_sheet(ds).unwrap();

        let inspect = exec_cmd(&mut session, "project.inspect", Caller::Mcp, json!({ "sheet_id": "data-1" })).unwrap();
        assert!(!inspect.mutated);
        assert_eq!(inspect.payload["total_sheet_count"], json!(2));
        assert_eq!(inspect.payload["focus"]["section"], json!("Data"));

        let recalc = exec_cmd(&mut session, "project.recalculate", Caller::Mcp, json!({ "include_external_links": false })).unwrap();
        assert!(recalc.mutated);

        let dir = std::env::temp_dir().join(format!("lc-proj-cmd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_path = dir.join("study.lumencraft");
        let tpl_path = dir.join("nested").join("study.lctemplate");

        let saved =
            exec_cmd(&mut session, "project.save", Caller::Cli, json!({ "path": save_path.display().to_string(), "include_bytes": true })).unwrap();
        assert!(!saved.mutated);
        assert!(save_path.exists());
        let b64 = saved.payload["bytes_base64"].as_str().unwrap().to_owned();

        let tpl_saved = exec_cmd(&mut session, "project.saveTemplate", Caller::Cli, json!({ "path": tpl_path.display().to_string() })).unwrap();
        assert!(!tpl_saved.mutated);
        assert!(tpl_path.exists());

        let mut reopened = new_session("Other");
        let opened = exec_cmd(&mut reopened, "project.open", Caller::Mcp, json!({ "bytes_base64": b64 })).unwrap();
        assert!(opened.mutated);
        assert_eq!(reopened.document().title, "Study Alpha");
        assert_eq!(reopened.document().total_sheet_count(), 2);

        let mut from_tpl = new_session("Other");
        let inst = exec_cmd(
            &mut from_tpl,
            "project.instantiateTemplate",
            Caller::Mcp,
            json!({ "path": tpl_path.display().to_string(), "title": "Run Two" }),
        )
        .unwrap();
        assert!(inst.mutated);
        assert_eq!(from_tpl.document().title, "Run Two");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoints_lifecycle_and_error_codes() {
        let mut session = new_session("Checkpoints");
        let cp1 = exec_cmd(
            &mut session,
            "project.createCheckpoint",
            Caller::Ui,
            json!({ "name": "Baseline", "note": "Before edit", "timestamp_utc": "2026-01-01T00:00:00Z" }),
        )
        .unwrap();
        let cp1_id = cp1.payload["id"].as_str().unwrap().to_owned();
        assert_eq!(cp1.payload["created_utc"], json!("2026-01-01T00:00:00Z"));

        let dup_err = exec_cmd(&mut session, "project.createCheckpoint", Caller::Mcp, json!({ "name": "Dup", "id": cp1_id })).unwrap_err();
        assert_eq!(dup_err.code, ErrorCode::AlreadyExists);

        exec_cmd(&mut session, "project.new", Caller::Ui, json!({ "title": "Modified" })).unwrap();
        session.undo().unwrap();
        let xy = XyTable::new(lumencraft_doc::SubcolumnMode::Single, 1, 2).unwrap();
        session.doc.add_data_sheet(DataSheet::new("d-added", "Added", TableArchetype::Xy(xy)).unwrap()).unwrap();

        let listed = exec_cmd(&mut session, "project.listCheckpoints", Caller::Mcp, json!({})).unwrap();
        assert!(!listed.mutated);
        assert_eq!(listed.payload["count"], json!(1));
        assert!(listed.payload["checkpoints"][0].get("snapshot_json").is_none());

        let restored = exec_cmd(&mut session, "project.restoreCheckpoint", Caller::Ui, json!({ "name": "Baseline" })).unwrap();
        assert!(restored.mutated);
        assert!(session.document().data_sheet(&SheetId::new("d-added")).is_none());
        assert_eq!(session.document().checkpoints.len(), 1);

        let deleted = exec_cmd(&mut session, "project.deleteCheckpoint", Caller::Ui, json!({ "id": cp1_id })).unwrap();
        assert!(deleted.mutated);
        assert_eq!(session.document().checkpoints.len(), 0);

        let missing_err = exec_cmd(&mut session, "project.deleteCheckpoint", Caller::Mcp, json!({ "name": "Missing" })).unwrap_err();
        assert_eq!(missing_err.code, ErrorCode::NotFound);
    }
}
