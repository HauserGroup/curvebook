//! Project lifecycle, file I/O, template, inspection, recalculation, and checkpoint commands.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::AtomicBool;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook_doc::{
    DocError, ErrorCode, MAX_PROJECT_IMAGE_ASSETS, MAX_SHEETS_PER_PROJECT, MAX_TITLE_BYTES, NamedCheckpoint, ProjectDocument, SheetId, SheetOrigin,
    validate_project_cells, validate_title,
};
use curvebook_format::registry::DESC_PRISM;
use curvebook_format::{
    ArchiveExportOptions, ArchiveKind, MAX_ARCHIVE_FILE_BYTES, export_prism_bytes_with_options, export_pzfx_bytes, instantiate_template_bytes,
    migrate_document_json, open_archive_bytes_with_warnings, open_project_bytes, open_project_file, open_project_unpacked_dir,
    open_project_unpacked_dir_with_warnings, sanitize_circular_hooks, save_project_bytes, save_project_unpacked_dir, save_template_bytes,
    strip_project_data_for_template, validate_project_document,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cmd::helpers::{
    always_enabled, bad_input, bad_input_ptr, decode_base64_capped, encode_base64, parse_sheet_id, prune_out_of_bounds_point_overrides,
    sync_info_linked_data_sheets,
};
use crate::cmd::params::{command_schema, empty_object_schema, parse_params};
use crate::cmd::{CommandGroup, CommandSpec, Ctx, Effects, command_specs};
use crate::services::{PZFX_EXPORT_DATA_ONLY_NOTICE, PZFX_EXPORT_STRUCTURE_NOTICE, format_import_or_archive_warning, format_pzfx_export_warning};
use crate::{CommandOutcome, EngineError, ExternalPathPolicy, dag};

/// Summary returned by project link/recalculation helpers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RecalcSummary {
    pub recalculated_sheets: usize,
    pub refreshed_links: usize,
    pub failed_result_sheets: Vec<dag::FailedResultSheet>,
}

impl RecalcSummary {
    pub fn failed_json(&self) -> Vec<Value> {
        self.failed_result_sheets.iter().map(dag::FailedResultSheet::to_json).collect()
    }

    pub fn failure_note(&self) -> Option<String> {
        if self.failed_result_sheets.is_empty() {
            return None;
        }
        let ids: Vec<&str> = self.failed_result_sheets.iter().map(|f| f.sheet_id.as_str()).collect();
        Some(format!(" with {} failed result sheet(s): {}", self.failed_result_sheets.len(), ids.join(", ")))
    }
}

/// Refresh internal/external linked regions and run full DAG recalculation under an explicit policy.
pub(super) fn recalculate_with_links_under_policy(
    doc: &mut ProjectDocument,
    include_external_links: bool,
    policy: &ExternalPathPolicy,
    cancelled: Option<&AtomicBool>,
) -> Result<RecalcSummary, EngineError> {
    let external_policy = include_external_links.then_some(policy);
    let (report, refreshed_links) = dag::recalculate_project_with_links_report(doc, external_policy, cancelled)?;
    prune_out_of_bounds_point_overrides(doc);
    let failed_result_sheets = dag::collect_failed_result_sheets_for_ids(doc, &report.failed_result_sheets);
    Ok(RecalcSummary { recalculated_sheets: report.total_updated(), refreshed_links, failed_result_sheets })
}

/// Incrementally recalculate the dirty transitive closure of `dirty_seeds` (plus any pending
/// `ResultStatus::Dirty` sheets) and refresh internal links without recomputing unrelated branches.
///
/// External links are refreshed only when an explicit [`ExternalPathPolicy`] is provided (`AUD-02-18`).
pub(super) fn recalculate_dirty_with_links(
    doc: &mut ProjectDocument,
    dirty_seeds: &[SheetId],
    external_policy: Option<&ExternalPathPolicy>,
    cancelled: Option<&AtomicBool>,
) -> Result<RecalcSummary, EngineError> {
    let (report, refreshed_links) = dag::recalculate_dirty_with_links_report(doc, dirty_seeds, external_policy, cancelled)?;
    prune_out_of_bounds_point_overrides(doc);
    let failed_result_sheets = dag::collect_failed_result_sheets_for_ids(doc, &report.failed_result_sheets);
    Ok(RecalcSummary { recalculated_sheets: report.total_updated(), refreshed_links, failed_result_sheets })
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

/// Validate an optional caller-supplied merge timestamp (`unix:<seconds>` or ISO-8601 UTC),
/// defaulting to [`current_utc_timestamp`] when omitted.
pub(super) fn resolve_optional_merged_at(raw: Option<&str>) -> Result<String, EngineError> {
    match raw {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err(bad_input_ptr("/merged_at", "`merged_at` cannot be empty"));
            }
            validate_title(trimmed)?;
            if !is_valid_iso8601_utc_timestamp(trimmed) {
                return Err(bad_input_ptr(
                    "/merged_at",
                    format!("invalid `merged_at` `{trimmed}`; expected ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) or `unix:<seconds>`"),
                ));
            }
            Ok(trimmed.to_owned())
        }
        None => Ok(current_utc_timestamp()),
    }
}

fn truncate_utf8_max_bytes(raw: &str, max_bytes: usize) -> &str {
    if raw.len() <= max_bytes {
        return raw;
    }
    let mut end = max_bytes;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw.get(..end).unwrap_or("")
}

/// Extract a sanitized basename-only `source_label` from an optional path (stripping both `/` and `\`
/// directories and control characters, never leaking full paths, and capping to [`MAX_TITLE_BYTES`]).
#[must_use]
pub(super) fn extract_source_label(path: Option<&str>, fallback: &str) -> String {
    let candidate = path
        .map(|raw| {
            let trimmed = raw.trim().trim_end_matches(['/', '\\']);
            let leaf = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
            let cleaned: String = leaf.chars().filter(|ch| !ch.is_control() && *ch != '/' && *ch != '\\').collect();
            cleaned.trim().to_owned()
        })
        .filter(|s| !s.is_empty() && s != "." && s != "..");
    let chosen = candidate.unwrap_or_else(|| fallback.to_owned());
    let clipped = truncate_utf8_max_bytes(&chosen, MAX_TITLE_BYTES).trim().to_owned();
    if clipped.is_empty() { fallback.to_owned() } else { clipped }
}

/// Stamp `origin` onto every sheet across all five sections of `doc`.
pub(super) fn stamp_document_origin(doc: &mut ProjectDocument, origin: &SheetOrigin) {
    for ds in &mut doc.data_sheets {
        ds.origin = Some(origin.clone());
    }
    for info in &mut doc.info_sheets {
        info.origin = Some(origin.clone());
    }
    for rs in &mut doc.result_sheets {
        rs.origin = Some(origin.clone());
    }
    for gs in &mut doc.graph_sheets {
        gs.origin = Some(origin.clone());
    }
    for ls in &mut doc.layout_sheets {
        ls.origin = Some(origin.clone());
    }
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

fn plan_imported_id_remap(doc: &ProjectDocument, imported: &ProjectDocument) -> Result<HashMap<SheetId, SheetId>, EngineError> {
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

/// Merge all sheets and referenced image assets from `imported` into `doc` using the single
/// canonical cap-checked, collision-renumbering, DAG-cycle-checked path (§10.1, §10.2).
///
/// Leaves behind `imported.checkpoints`, project-level settings/title, `active_sheet_id`, and
/// `family_anchor_sheet_id`, and preserves `ResultStatus::Imported` / `ImportedStale`.
pub(super) fn merge_imported_document(doc: &mut ProjectDocument, mut imported: ProjectDocument) -> Result<(), EngineError> {
    let total_sheets = doc.total_sheet_count().saturating_add(imported.total_sheet_count());
    if total_sheets > MAX_SHEETS_PER_PROJECT {
        return Err(EngineError::from(DocError::TooManySheets { count: total_sheets, max: MAX_SHEETS_PER_PROJECT }));
    }
    let existing_cells = doc.total_allocated_cells()?;
    let added_cells = imported.total_allocated_cells()?;
    validate_project_cells(existing_cells, added_cells)?;
    let existing_asset_ids: HashSet<&str> = doc.image_assets.iter().map(|a| a.asset_id.as_str()).collect();
    let mut added_asset_ids: HashSet<&str> = HashSet::new();
    for asset in &imported.image_assets {
        if !existing_asset_ids.contains(asset.asset_id.as_str()) {
            added_asset_ids.insert(asset.asset_id.as_str());
        }
    }
    let total_assets = doc.image_assets.len().saturating_add(added_asset_ids.len());
    if total_assets > MAX_PROJECT_IMAGE_ASSETS {
        return Err(EngineError::from(DocError::TooManyImageAssets { count: total_assets, max: MAX_PROJECT_IMAGE_ASSETS }));
    }

    let id_map = plan_imported_id_remap(doc, &imported)?;
    if !id_map.is_empty() {
        let btree_map: BTreeMap<SheetId, SheetId> = id_map.into_iter().collect();
        curvebook_doc::remap_document_sheet_ids(&mut imported, &btree_map);
    }

    for asset in imported.image_assets {
        doc.upsert_image_asset(asset)?;
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NewProjectParams {
    /// Title for the new project (defaults to `Untitled Project`).
    title: Option<String>,
}

fn run_file_new(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    run_new_project_with_id(ctx, "file.new", params)
}

fn run_project_new(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    run_new_project_with_id(ctx, "project.new", params)
}

fn run_new_project_with_id(ctx: &mut Ctx<'_>, command_id: &str, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: NewProjectParams = parse_params(&command_schema::<NewProjectParams>(), params)?;
    let title = parsed.title.unwrap_or_else(|| String::from("Untitled Project"));
    let doc = ProjectDocument::new(title)?;
    let result_title = doc.title.clone();
    ctx.session.doc = doc;
    ctx.session.set_file_path(None, false);
    Ok(CommandOutcome::mutated(command_id, "created new project", json!({ "title": result_title })))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenProjectParams {
    /// Filesystem path to `.curvebook` archive or `.curvebook.d/` directory.
    path: Option<String>,
    /// Base64-encoded `.curvebook` zip archive bytes.
    bytes_base64: Option<String>,
    /// Treat `path` as an unpacked `.curvebook.d/` project directory.
    unpacked: Option<bool>,
}

fn run_project_open(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: OpenProjectParams = parse_params(&command_schema::<OpenProjectParams>(), params)?;
    let explicit_unpacked = parsed.unpacked.unwrap_or(false);
    let (loaded, used_unpacked, opened_path) = if let Some(ref path) = parsed.path {
        if path.trim().is_empty() {
            return Err(bad_input("project.open `path` must not be empty"));
        }
        let (resolved, use_unpacked) = ctx.session.path_policy.resolve_project_open_target(path, explicit_unpacked)?;
        let opened = if use_unpacked { open_project_unpacked_dir(&resolved)? } else { open_project_file(&resolved)? };
        (opened, use_unpacked, Some(resolved))
    } else if explicit_unpacked {
        return Err(bad_input("project.open with `unpacked: true` requires `path`")
            .with_remediation("Provide a directory path in `path` when opening an unpacked project."));
    } else if let Some(ref b64) = parsed.bytes_base64 {
        let bytes = decode_base64_capped(b64, MAX_ARCHIVE_FILE_BYTES, "/bytes_base64")?;
        (open_project_bytes(&bytes)?, false, None)
    } else {
        return Err(bad_input("project.open requires `path` or `bytes_base64`").with_remediation("Pass either `path` or `bytes_base64`."));
    };

    let opened_from_path = opened_path.is_some();
    ctx.session.doc = loaded;
    ctx.session.set_file_path(opened_path, opened_from_path);
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MergeProjectParams {
    /// Filesystem path to `.curvebook` archive, `.curvebook.d/` directory, or `.cbtemplate` template.
    path: Option<String>,
    /// Base64-encoded `.curvebook` or `.cbtemplate` archive bytes.
    bytes_base64: Option<String>,
    /// Treat `path` as an unpacked `.curvebook.d/` project directory.
    unpacked: Option<bool>,
    /// Optional caller-supplied UTC timestamp (`unix:<seconds>` or ISO-8601 UTC) recorded in merged sheet origins.
    merged_at: Option<String>,
    /// Optional filename-only source label override (no path separators) when merging inline `bytes_base64`.
    source_label: Option<String>,
}

fn load_merge_archive_bytes(bytes: &[u8]) -> Result<(ProjectDocument, &'static str, &'static str, Vec<String>), EngineError> {
    let (kind, raw_doc, warnings) = open_archive_bytes_with_warnings(bytes)?;
    match kind {
        ArchiveKind::Project => Ok((raw_doc, "curvebook", "inline.curvebook", warnings)),
        ArchiveKind::Template => {
            let templated = strip_project_data_for_template(&raw_doc)?;
            Ok((templated, "cbtemplate", "inline.cbtemplate", warnings))
        }
    }
}

fn run_project_merge(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: MergeProjectParams = parse_params(&command_schema::<MergeProjectParams>(), params)?;
    let explicit_unpacked = parsed.unpacked.unwrap_or(false);
    let merged_at = resolve_optional_merged_at(parsed.merged_at.as_deref())?;

    let (mut loaded, source_format, fallback_label, used_unpacked, load_warnings) = if let Some(ref path) = parsed.path {
        if path.trim().is_empty() {
            return Err(bad_input_ptr("/path", "project.merge `path` must not be empty"));
        }
        let (resolved, use_unpacked) = ctx.session.path_policy.resolve_project_open_target(path, explicit_unpacked)?;
        if use_unpacked {
            let (doc, warnings) = open_project_unpacked_dir_with_warnings(&resolved)?;
            (doc, "curvebook.d", "inline.curvebook.d", true, warnings)
        } else {
            let bytes = ctx.session.path_policy.read_binary_file(path, "project file", MAX_ARCHIVE_FILE_BYTES)?;
            let (doc, fmt, fallback, warnings) = load_merge_archive_bytes(&bytes)?;
            (doc, fmt, fallback, false, warnings)
        }
    } else if explicit_unpacked {
        return Err(bad_input("project.merge with `unpacked: true` requires `path`")
            .with_remediation("Provide a directory path in `path` when merging an unpacked project."));
    } else if let Some(ref b64) = parsed.bytes_base64 {
        let bytes = decode_base64_capped(b64, MAX_ARCHIVE_FILE_BYTES, "/bytes_base64")?;
        let (doc, fmt, fallback, warnings) = load_merge_archive_bytes(&bytes)?;
        (doc, fmt, fallback, false, warnings)
    } else {
        return Err(bad_input("project.merge requires `path` or `bytes_base64`").with_remediation("Pass either `path` or `bytes_base64`."));
    };

    let source_label = match parsed.source_label.as_deref() {
        Some(raw) => raw.trim().to_owned(),
        None => extract_source_label(parsed.path.as_deref(), fallback_label),
    };
    let origin = SheetOrigin::new(source_label, source_format, merged_at)?;
    stamp_document_origin(&mut loaded, &origin);

    let prev_active = ctx.session.doc.active_sheet_id.clone();
    let prev_anchor = ctx.session.doc.family_anchor_sheet_id.clone();
    let before_data = ctx.session.doc.data_sheets.len();
    let before_info = ctx.session.doc.info_sheets.len();
    let before_results = ctx.session.doc.result_sheets.len();
    let before_graphs = ctx.session.doc.graph_sheets.len();
    let before_layouts = ctx.session.doc.layout_sheets.len();
    let added_sheets = loaded.total_sheet_count();

    merge_imported_document(&mut ctx.session.doc, loaded)?;
    sync_info_linked_data_sheets(&mut ctx.session.doc, &HashSet::new());

    if prev_active.is_some() {
        ctx.session.doc.active_sheet_id = prev_active;
    }
    if prev_anchor.is_some() {
        ctx.session.doc.family_anchor_sheet_id = prev_anchor;
    }

    let merged_sheet_ids: Vec<String> = ctx
        .session
        .doc
        .data_sheets
        .iter()
        .skip(before_data)
        .map(|s| s.id.as_str().to_owned())
        .chain(ctx.session.doc.info_sheets.iter().skip(before_info).map(|s| s.id.as_str().to_owned()))
        .chain(ctx.session.doc.result_sheets.iter().skip(before_results).map(|s| s.id.as_str().to_owned()))
        .chain(ctx.session.doc.graph_sheets.iter().skip(before_graphs).map(|s| s.id.as_str().to_owned()))
        .chain(ctx.session.doc.layout_sheets.iter().skip(before_layouts).map(|s| s.id.as_str().to_owned()))
        .collect();

    let mut outcome = CommandOutcome::mutated(
        "project.merge",
        format!("merged {added_sheets} sheet(s) from `{}`", origin.source_label),
        json!({
            "title": ctx.session.doc.title,
            "source_label": origin.source_label,
            "source_format": origin.source_format,
            "merged_at": origin.merged_at,
            "added_sheets": added_sheets,
            "merged_sheet_ids": merged_sheet_ids,
            "sheet_count": ctx.session.doc.total_sheet_count(),
            "unpacked": used_unpacked,
        }),
    );
    outcome.warnings = load_warnings;
    Ok(outcome)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveProjectParams {
    /// Destination `.curvebook` archive path or `.curvebook.d/` directory.
    path: Option<String>,
    /// Include base64-encoded `.curvebook` bytes in response (defaults to true when `path` is omitted).
    return_bytes: Option<bool>,
    /// Save as an unpacked `.curvebook.d/` project directory.
    unpacked: Option<bool>,
}

fn run_project_save(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: SaveProjectParams = parse_params(&command_schema::<SaveProjectParams>(), params)?;
    let explicit_unpacked = parsed.unpacked.unwrap_or(false);
    if explicit_unpacked && parsed.path.is_none() {
        return Err(bad_input("project.save with `unpacked: true` requires `path`")
            .with_remediation("Provide a target directory `path` when saving unpacked format."));
    }
    let return_bytes = parsed.return_bytes.unwrap_or(parsed.path.is_none());

    let (used_unpacked, archive_bytes, saved_path) = if let Some(ref path) = parsed.path {
        if path.trim().is_empty() {
            return Err(bad_input("project.save `path` must not be empty"));
        }
        let (resolved, use_unpacked) = ctx.session.path_policy.resolve_project_save_target(path, explicit_unpacked)?;
        if use_unpacked {
            save_project_unpacked_dir(&ctx.session.doc, &resolved)?;
            let bytes = if return_bytes { save_project_bytes(&ctx.session.doc)? } else { Vec::new() };
            (true, bytes, Some(resolved))
        } else {
            let bytes = save_project_bytes(&ctx.session.doc)?;
            ctx.session.path_policy.write_atomic_archive_bytes(path, &bytes)?;
            (false, bytes, Some(resolved))
        }
    } else {
        let bytes = save_project_bytes(&ctx.session.doc)?;
        (false, bytes, None)
    };

    let reported_path = saved_path.as_ref().map(|p| p.to_string_lossy().into_owned()).or(parsed.path);
    if let Some(resolved_path) = saved_path {
        ctx.session.mark_saved_clean(resolved_path);
    }

    let bytes_len = archive_bytes.len();
    let encoded = if return_bytes { Some(encode_base64(&archive_bytes)) } else { None };
    Ok(CommandOutcome::read_only(
        "project.save",
        "saved project",
        json!({
            "title": ctx.session.doc.title,
            "path": reported_path,
            "unpacked": used_unpacked,
            "bytes_len": bytes_len,
            "bytes_base64": encoded,
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveTemplateParams {
    /// Destination `.cbtemplate` file path.
    path: Option<String>,
    /// Include base64-encoded template bytes in response (defaults to true when `path` is omitted).
    return_bytes: Option<bool>,
}

fn run_project_save_template(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: SaveTemplateParams = parse_params(&command_schema::<SaveTemplateParams>(), params)?;
    let return_bytes = parsed.return_bytes.unwrap_or(parsed.path.is_none());
    let bytes = save_template_bytes(&ctx.session.doc)?;
    if let Some(ref path) = parsed.path {
        ctx.session.path_policy.write_atomic_archive_bytes(path, &bytes)?;
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportPzfxParams {
    /// Optional destination `.pzfx` file path written atomically without changing the session's active `file_path` or unsaved state.
    path: Option<String>,
    /// Include base64-encoded `.pzfx` XML bytes in response (defaults to true when `path` is omitted).
    return_bytes: Option<bool>,
}

fn run_project_export_pzfx(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: ExportPzfxParams = parse_params(&command_schema::<ExportPzfxParams>(), params)?;
    if let Some(ref path) = parsed.path
        && path.trim().is_empty()
    {
        return Err(bad_input_ptr("/path", "project.exportPzfx `path` must not be empty"));
    }
    let return_bytes = parsed.return_bytes.unwrap_or(parsed.path.is_none());
    let report = export_pzfx_bytes(&ctx.session.doc)?;
    if let Some(ref path) = parsed.path {
        ctx.session.path_policy.write_atomic_bytes(path, &report.bytes)?;
    }
    let encoded = if return_bytes { Some(encode_base64(&report.bytes)) } else { None };
    let warning_lines: Vec<String> = report.warnings.iter().map(format_pzfx_export_warning).collect();
    let mut outcome = CommandOutcome::read_only(
        "project.exportPzfx",
        "exported .pzfx data file",
        json!({
            "format": "pzfx",
            "title": ctx.session.doc.title,
            "path": parsed.path,
            "bytes_len": report.bytes.len(),
            "bytes_base64": encoded,
            "data_sheet_count": report.preflight.data_sheet_count,
            "info_sheet_count": report.preflight.info_sheet_count,
            "total_dropped_count": report.preflight.total_dropped_count(),
            "preflight": report.preflight,
            "warnings": report.warnings,
            "data_only_notice": PZFX_EXPORT_DATA_ONLY_NOTICE,
            "notice": PZFX_EXPORT_STRUCTURE_NOTICE,
        }),
    );
    outcome.warnings = warning_lines;
    Ok(outcome)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportPrismParams {
    /// Optional destination `.prism` file path written atomically without changing the session's active `file_path` or unsaved state.
    path: Option<String>,
    /// Include base64-encoded `.prism` archive bytes in response (defaults to true when `path` is omitted).
    return_bytes: Option<bool>,
    /// Explicit opt-in required to write experimental `.prism` archives (defaults to false; `.prism` is import-only at Beta per §6.3).
    experimental: Option<bool>,
}

fn run_project_export_prism(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: ExportPrismParams = parse_params(&command_schema::<ExportPrismParams>(), params)?;
    if let Some(ref path) = parsed.path
        && path.trim().is_empty()
    {
        return Err(bad_input_ptr("/path", "project.exportPrism `path` must not be empty"));
    }
    let experimental = parsed.experimental.unwrap_or(false);
    let report = export_prism_bytes_with_options(&ctx.session.doc, ArchiveExportOptions { experimental })?;
    let return_bytes = parsed.return_bytes.unwrap_or(parsed.path.is_none());
    if let Some(ref path) = parsed.path {
        ctx.session.path_policy.write_atomic_bytes(path, &report.bytes)?;
    }
    let encoded = if return_bytes { Some(encode_base64(&report.bytes)) } else { None };
    let warning_lines: Vec<String> = report.warnings.iter().map(format_import_or_archive_warning).collect();
    let mut outcome = CommandOutcome::read_only(
        "project.exportPrism",
        "exported experimental .prism archive",
        json!({
            "format": DESC_PRISM.id,
            "experimental": true,
            "title": ctx.session.doc.title,
            "path": parsed.path,
            "bytes_len": report.bytes.len(),
            "bytes_base64": encoded,
            "warnings": report.warnings,
        }),
    );
    outcome.warnings = warning_lines;
    Ok(outcome)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InstantiateTemplateParams {
    /// Path to `.cbtemplate` archive file.
    path: Option<String>,
    /// Base64-encoded `.cbtemplate` archive bytes.
    bytes_base64: Option<String>,
    /// Optional title override for the instantiated project.
    title: Option<String>,
}

fn run_project_instantiate_template(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: InstantiateTemplateParams = parse_params(&command_schema::<InstantiateTemplateParams>(), params)?;
    let bytes = if let Some(ref path) = parsed.path {
        ctx.session.path_policy.read_binary_file(path, "template file", MAX_ARCHIVE_FILE_BYTES)?
    } else if let Some(ref b64) = parsed.bytes_base64 {
        decode_base64_capped(b64, MAX_ARCHIVE_FILE_BYTES, "/bytes_base64")?
    } else {
        return Err(bad_input("project.instantiateTemplate requires `path` or `bytes_base64`")
            .with_remediation("Pass either `path` or `bytes_base64` pointing to a template archive."));
    };

    let instantiated = instantiate_template_bytes(&bytes, parsed.title.as_deref())?;
    ctx.session.doc = instantiated;
    ctx.session.set_file_path(None, false);
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InspectProjectParams {
    /// Optional sheet ID to focus dependency and section inspection on.
    sheet_id: Option<String>,
}

fn run_project_inspect(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: InspectProjectParams = parse_params(&command_schema::<InspectProjectParams>(), params)?;
    let focus_id = parsed.sheet_id.as_deref().map(parse_sheet_id).transpose()?;
    let summary = crate::services::inspect_project_summary(&ctx.session.doc, ctx.session.is_dirty(), ctx.session.file_path(), focus_id.as_ref())?;
    Ok(CommandOutcome::read_only("project.inspect", "inspected project", summary.to_project_inspect_json()))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecalculateParams {
    /// Refresh external delimited-file links before recalculating downstream analyses (default true).
    include_external_links: Option<bool>,
}

fn run_project_recalculate(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: RecalculateParams = parse_params(&command_schema::<RecalculateParams>(), params)?;
    let include_external = parsed.include_external_links.unwrap_or(true);
    let recalc_policy = ctx.session.path_policy.clone();
    let summary = recalculate_with_links_under_policy(&mut ctx.session.doc, include_external, &recalc_policy, Some(ctx.cancelled))?;
    let message = match summary.failure_note() {
        Some(note) => format!("recalculated project{note}"),
        None => String::from("recalculated project"),
    };
    Ok(CommandOutcome::mutated(
        "project.recalculate",
        message,
        json!({
            "recalculated_sheets": summary.recalculated_sheets,
            "refreshed_links": summary.refreshed_links,
            "failed_count": summary.failed_result_sheets.len(),
            "failed_result_sheets": summary.failed_json(),
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateCheckpointParams {
    /// Human-readable checkpoint name.
    #[schemars(length(min = 1))]
    name: String,
    /// Optional unique checkpoint ID (auto-generated if omitted).
    #[schemars(length(min = 1))]
    id: Option<String>,
    /// Optional checkpoint description or note.
    note: Option<String>,
    /// Optional explicit ISO-8601 UTC timestamp.
    timestamp_utc: Option<String>,
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

fn is_valid_iso8601_utc_timestamp(raw: &str) -> bool {
    if let Some(secs) = raw.strip_prefix("unix:") {
        return !secs.is_empty() && secs.bytes().all(|b| b.is_ascii_digit());
    }
    let bytes = raw.as_bytes();
    if bytes.len() < 20 {
        return false;
    }
    // Core calendar + time prefix: YYYY-MM-DDTHH:MM:SS (or 't')
    let [_, _, _, _, b'-', _, _, b'-', _, _, b'T' | b't', _, _, b':', _, _, b':', ..] = bytes else {
        return false;
    };
    let parse_2 = |i: usize| -> Option<u32> {
        let s = raw.get(i..i + 2)?;
        if s.bytes().all(|b| b.is_ascii_digit()) { s.parse().ok() } else { None }
    };
    let Some(year_str) = raw.get(0..4).filter(|s| s.bytes().all(|b| b.is_ascii_digit())) else {
        return false;
    };
    let Ok(year) = year_str.parse::<u32>() else {
        return false;
    };
    let (Some(month), Some(day), Some(hour), Some(min), Some(sec)) = (parse_2(5), parse_2(8), parse_2(11), parse_2(14), parse_2(17)) else {
        return false;
    };
    let is_leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    if !(1..=max_day).contains(&day) || hour > 23 || min > 59 || sec > 60 {
        return false;
    }
    let Some(mut rest) = raw.get(19..) else {
        return false;
    };
    if let Some(after_dot) = rest.strip_prefix('.') {
        let frac_len = after_dot.bytes().take_while(u8::is_ascii_digit).count();
        if frac_len == 0 || frac_len > 9 {
            return false;
        }
        let Some(after_frac) = after_dot.get(frac_len..) else {
            return false;
        };
        rest = after_frac;
    }
    if matches!(rest, "Z" | "z") {
        return true;
    }
    if let Some(off) = rest.strip_prefix('+').or_else(|| rest.strip_prefix('-'))
        && let Some((hh_str, mm_str)) = off.split_once(':')
        && hh_str.len() == 2
        && mm_str.len() == 2
    {
        let oh = hh_str.bytes().all(|b| b.is_ascii_digit()).then(|| hh_str.parse::<u32>().ok()).flatten();
        let om = mm_str.bytes().all(|b| b.is_ascii_digit()).then(|| mm_str.parse::<u32>().ok()).flatten();
        return matches!((oh, om), (Some(h), Some(m)) if h <= 23 && m <= 59);
    }
    false
}

const MAX_CHECKPOINTS_PER_PROJECT: usize = 32;
const MAX_TOTAL_CHECKPOINT_BYTES: usize = MAX_ARCHIVE_FILE_BYTES;

fn run_project_create_checkpoint(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: CreateCheckpointParams = parse_params(&command_schema::<CreateCheckpointParams>(), params)?;
    let name = parsed.name.trim().to_owned();
    if name.is_empty() {
        return Err(bad_input("checkpoint `name` must not be empty"));
    }
    validate_title(&name)?;

    let id = match parsed.id {
        Some(raw) => {
            let trimmed = raw.trim().to_owned();
            if trimmed.is_empty() {
                return Err(bad_input("checkpoint `id` must not be empty"));
            }
            validate_title(&trimmed)?;
            trimmed
        }
        None => next_checkpoint_id(&ctx.session.doc),
    };

    if ctx.session.doc.checkpoints.len() >= MAX_CHECKPOINTS_PER_PROJECT {
        return Err(EngineError::with_code(
            ErrorCode::ResourceLimit,
            format!("checkpoint limit reached (maximum {MAX_CHECKPOINTS_PER_PROJECT} checkpoints per project)"),
        )
        .with_remediation("Delete an older checkpoint with `project.deleteCheckpoint` before creating another."));
    }
    if ctx.session.doc.checkpoints.iter().any(|cp| cp.id == id) {
        return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("checkpoint `{id}` already exists"))
            .with_remediation("Choose a distinct `id` or omit it to auto-assign."));
    }

    let note = parsed.note.unwrap_or_default();
    validate_title(&note)?;
    let created_utc = match parsed.timestamp_utc {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(bad_input("`timestamp_utc` cannot be empty"));
            }
            validate_title(trimmed)?;
            if !is_valid_iso8601_utc_timestamp(trimmed) {
                return Err(bad_input_ptr(
                    "/timestamp_utc",
                    format!("invalid `timestamp_utc` `{trimmed}`; expected ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) or `unix:<seconds>`"),
                ));
            }
            trimmed.to_owned()
        }
        None => current_utc_timestamp(),
    };

    let mut snapshot_doc = ctx.session.doc.clone();
    snapshot_doc.checkpoints.clear();
    let snapshot_json = serde_json::to_string(&snapshot_doc)
        .map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed serializing checkpoint snapshot: {err}")))?;

    let retained_bytes: usize = ctx.session.doc.checkpoints.iter().fold(0usize, |acc, cp| acc.saturating_add(cp.snapshot_json.len()));
    if retained_bytes.saturating_add(snapshot_json.len()) > MAX_TOTAL_CHECKPOINT_BYTES {
        return Err(EngineError::with_code(
            ErrorCode::ResourceLimit,
            format!("checkpoint storage exceeds maximum {MAX_TOTAL_CHECKPOINT_BYTES} byte budget"),
        )
        .with_remediation("Delete older checkpoints with `project.deleteCheckpoint` to free space."));
    }

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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CheckpointRefParams {
    /// Checkpoint ID to locate.
    id: Option<String>,
    /// Checkpoint name to locate.
    name: Option<String>,
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
    Err(bad_input(format!("`{command_id}` requires `id` or `name`")).with_remediation("Provide a non-empty checkpoint `id` or `name`."))
}

fn run_project_restore_checkpoint(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: CheckpointRefParams = parse_params(&command_schema::<CheckpointRefParams>(), params)?;
    let idx = find_checkpoint_index(&ctx.session.doc, "project.restoreCheckpoint", &parsed)?;
    let cp =
        ctx.session.doc.checkpoints.get(idx).cloned().ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, "checkpoint index out of bounds"))?;
    if cp.snapshot_json.len() > MAX_ARCHIVE_FILE_BYTES {
        return Err(EngineError::with_code(
            ErrorCode::ResourceLimit,
            format!("checkpoint `{}` snapshot exceeds maximum allowed archive size", cp.id),
        ));
    }
    let raw_val: Value = serde_json::from_str(&cp.snapshot_json)
        .map_err(|err| EngineError::with_code(ErrorCode::CorruptData, format!("corrupt checkpoint `{}` snapshot: {err}", cp.id)))?;
    let from_version = match raw_val.get("schema_version") {
        Some(v) => u32::try_from(
            v.as_u64()
                .ok_or_else(|| EngineError::with_code(ErrorCode::CorruptData, format!("checkpoint `{}` has invalid `schema_version`", cp.id)))?,
        )
        .map_err(|_| EngineError::with_code(ErrorCode::UnsupportedVersion, format!("checkpoint `{}` `schema_version` overflow", cp.id)))?,
        None => 0,
    };
    let migrated_val = migrate_document_json(raw_val, from_version)?;
    let mut restored: ProjectDocument = serde_json::from_value(migrated_val)
        .map_err(|err| EngineError::with_code(ErrorCode::CorruptData, format!("corrupt checkpoint `{}` snapshot: {err}", cp.id)))?;
    restored.checkpoints.clear();
    let sanitized_cycles = sanitize_circular_hooks(&mut restored);
    restored.checkpoints.clone_from(&ctx.session.doc.checkpoints);
    validate_project_document(&restored)?;
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
            "sanitized_cycles": sanitized_cycles.len(),
        }),
    ))
}

fn run_project_delete_checkpoint(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: CheckpointRefParams = parse_params(&command_schema::<CheckpointRefParams>(), params)?;
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

static SPECS: &[CommandSpec] = &command_specs!(
    CommandGroup::Project;
    {
        id: "file.new",
        label: "New Project",
        menu_path: "File/New Project",
        shortcut: "Cmd+N",
        summary: "Create a fresh empty CurveBook project.",
        beginner: "Replaces the active workspace with an empty project document (preserving undo history in this session).",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<NewProjectParams>,
        enabled: always_enabled,
        run: run_file_new,
    },
    {
        id: "project.new",
        label: "New Project",
        menu_path: "File/New Project",
        summary: "Create a fresh empty CurveBook project.",
        beginner: "Creates a clean empty project document with an optional title; alias for `file.new`.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<NewProjectParams>,
        enabled: always_enabled,
        run: run_project_new,
    },
    {
        id: "project.open",
        label: "Open Project",
        menu_path: "File/Open",
        shortcut: "Cmd+O",
        summary: "Load a `.curvebook` archive or unpacked `.curvebook.d/` directory.",
        beginner: "Opens a project from a filesystem path or base64 archive payload and replaces the current session document.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<OpenProjectParams>,
        enabled: always_enabled,
        run: run_project_open,
    },
    {
        id: "project.merge",
        label: "Merge Project",
        menu_path: "File/Merge Project...",
        summary: "Merge sheets and referenced image assets from another `.curvebook`, `.curvebook.d/`, or `.cbtemplate` project into the active project.",
        beginner: "Brings data tables, info sheets, results, graphs, layouts, and embedded images from another project into this project in one undoable step without changing the open project's file path.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: command_schema::<MergeProjectParams>,
        enabled: always_enabled,
        run: run_project_merge,
    },
    {
        id: "project.save",
        label: "Save Project",
        menu_path: "File/Save",
        shortcut: "Cmd+S",
        summary: "Save the active project atomically to `.curvebook`, unpacked directory, or bytes.",
        beginner: "Serializes the project document using atomic filesystem writes when a path is provided, or returns base64 bytes.",
        effects: Effects::WRITES_FILESYSTEM,
        input_schema: command_schema::<SaveProjectParams>,
        enabled: always_enabled,
        run: run_project_save,
    },
    {
        id: "project.saveTemplate",
        label: "Save Template",
        menu_path: "File/Save Template",
        summary: "Export the project structure as a data-stripped `.cbtemplate` file.",
        beginner: "Strips raw data values while preserving table schemas, analyses, graphs, and layouts, then writes atomically to disk or base64.",
        effects: Effects::WRITES_FILESYSTEM,
        input_schema: command_schema::<SaveTemplateParams>,
        enabled: always_enabled,
        run: run_project_save_template,
    },
    {
        id: "project.exportPzfx",
        label: "Export .pzfx File",
        menu_path: "File/Export/Export .pzfx File...",
        summary: "Export data tables and info sheets as a deterministic `.pzfx` XML file.",
        beginner: "Saves data tables and info sheets only (analyses, graphs, and layouts stay in your `.curvebook` project; written to match the `.pzfx` structure of real files and not tested in other applications) without changing the active file path or unsaved state.",
        effects: Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<ExportPzfxParams>,
        enabled: always_enabled,
        run: run_project_export_pzfx,
    },
    {
        id: "project.exportPrism",
        label: "Export .prism File (Experimental)",
        menu_path: "File/Export/Export .prism File (Experimental)...",
        summary: "Experimental `.prism` archive export (disabled by default; requires `experimental: true`).",
        beginner: "Requires explicit `experimental: true` opt-in because `.prism` is import-only at Beta; use `project.exportPzfx` for standard data interchange. Never changes the active file path or unsaved state.",
        effects: Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<ExportPrismParams>,
        enabled: always_enabled,
        run: run_project_export_prism,
    },
    {
        id: "project.instantiateTemplate",
        label: "New Project From Template",
        menu_path: "File/New From Template",
        summary: "Create a new project from a `.cbtemplate` archive.",
        beginner: "Loads a saved template with its analysis pipelines, graphs, and layouts ready for new experimental data.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<InstantiateTemplateParams>,
        enabled: always_enabled,
        run: run_project_instantiate_template,
    },
    {
        id: "project.inspect",
        label: "Inspect Project",
        menu_path: "View/Inspect Project",
        summary: "Inspect sheet structure, metadata, and upstream dependency edges.",
        beginner: "Returns a structured read-only summary of all sheets, constants, checkpoints, and optional single-sheet dependencies.",
        effects: Effects::IDEMPOTENT,
        input_schema: command_schema::<InspectProjectParams>,
        enabled: always_enabled,
        run: run_project_inspect,
    },
    {
        id: "project.recalculate",
        label: "Recalculate All",
        menu_path: "Analyze/Recalculate All",
        summary: "Refresh live links and recalculate all downstream analyses and graphs.",
        beginner: "Re-evaluates linked regions, calculated variables, statistical analyses, curve fits, and derived tables in dependency order.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::IDEMPOTENT),
        input_schema: command_schema::<RecalculateParams>,
        enabled: always_enabled,
        run: run_project_recalculate,
    },
    {
        id: "project.createCheckpoint",
        label: "Create Checkpoint",
        menu_path: "File/Checkpoints/Create Checkpoint",
        summary: "Save a named project snapshot checkpoint inside the document.",
        beginner: "Captures a named milestone copy of all sheets so you can restore or branch back to it later.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<CreateCheckpointParams>,
        enabled: always_enabled,
        run: run_project_create_checkpoint,
    },
    {
        id: "project.listCheckpoints",
        label: "List Checkpoints",
        menu_path: "File/Checkpoints/List Checkpoints",
        summary: "List named checkpoints stored in the project.",
        beginner: "Lists each checkpoint's ID, name, note, and UTC timestamp without dumping full snapshot payloads.",
        effects: Effects::IDEMPOTENT,
        input_schema: empty_object_schema,
        enabled: always_enabled,
        run: run_project_list_checkpoints,
    },
    {
        id: "project.restoreCheckpoint",
        label: "Restore Checkpoint",
        menu_path: "File/Checkpoints/Restore Checkpoint",
        summary: "Restore the document to a previously saved named checkpoint.",
        beginner: "Replaces the current sheets with the chosen checkpoint snapshot while keeping existing checkpoints intact.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<CheckpointRefParams>,
        enabled: always_enabled,
        run: run_project_restore_checkpoint,
    },
    {
        id: "project.deleteCheckpoint",
        label: "Delete Checkpoint",
        menu_path: "File/Checkpoints/Delete Checkpoint",
        summary: "Remove a named checkpoint from the project.",
        beginner: "Deletes a stored checkpoint by ID or name without altering active data sheets.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<CheckpointRefParams>,
        enabled: always_enabled,
        run: run_project_delete_checkpoint,
    },
);

/// Registered command specifications for the `project` group.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use curvebook_doc::{DataSheet, InfoSheet, TableArchetype, XyTable};
    use serde_json::json;

    use crate::{Session, cmd::Caller};

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

        let r1 = exec_cmd(&mut session, "file.new", Caller::Ui, json!({ "title": "Created Via File" })).unwrap();
        assert!(r1.mutated);
        assert_eq!(session.document().title, "Created Via File");

        let r2 = exec_cmd(&mut session, "project.new", Caller::Mcp, json!({ "title": "Study Alpha" })).unwrap();
        assert!(r2.mutated);
        assert_eq!(session.document().title, "Study Alpha");

        let info = InfoSheet::new_preseeded("info-1", "Info").unwrap();
        session.doc.add_info_sheet(info).unwrap();
        let xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 3).unwrap();
        let mut ds = DataSheet::new("data-1", "Dose", TableArchetype::Xy(xy)).unwrap();
        ds.info_sheet_id = Some(SheetId::new("info-1"));
        session.doc.add_data_sheet(ds).unwrap();

        let inspect = exec_cmd(&mut session, "project.inspect", Caller::Mcp, json!({ "sheet_id": "data-1" })).unwrap();
        assert!(!inspect.mutated);
        assert_eq!(inspect.payload["total_sheet_count"], json!(2));
        assert_eq!(inspect.payload["focus"]["section"], json!("Data"));

        let recalc = exec_cmd(&mut session, "project.recalculate", Caller::Mcp, json!({ "include_external_links": false })).unwrap();
        assert!(recalc.mutated);

        let dir = std::env::temp_dir().join(format!("cb-proj-cmd-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_path = dir.join("study.curvebook");
        let tpl_path = dir.join("nested").join("study.cbtemplate");

        let saved =
            exec_cmd(&mut session, "project.save", Caller::Cli, json!({ "path": save_path.display().to_string(), "return_bytes": true })).unwrap();
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
        let xy = XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 2).unwrap();
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

    fn build_future_archive_bytes(kind: &str, schema_version: u64) -> Vec<u8> {
        let mut w = curvebook_format::zip::ZipWriter::new();
        let doc_json = format!(r#"{{"schema_version":{schema_version},"title":"Future Study"}}"#).into_bytes();
        let manifest_json = format!(
            r#"{{"format":"curvebook-archive","schema_version":{schema_version},"kind":"{kind}","title":"Future Study","document_crc32":0,"sheets":[]}}"#
        )
        .into_bytes();
        w.add_file("manifest.json", manifest_json).unwrap();
        w.add_file("document.json", doc_json).unwrap();
        w.finish().unwrap()
    }

    #[test]
    fn forward_version_save_commands_refuse_to_overwrite_existing_targets() {
        let dir = std::env::temp_dir().join(format!("cb_proj_forward_guard_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let future_ver = u64::from(curvebook_doc::CURRENT_SCHEMA_VERSION) + 1;
        let archive_path = dir.join("future.curvebook");
        let tpl_path = dir.join("future.cbtemplate");
        let archive_bytes = build_future_archive_bytes("project", future_ver);
        let tpl_bytes = build_future_archive_bytes("template", future_ver);
        std::fs::write(&archive_path, &archive_bytes).unwrap();
        std::fs::write(&tpl_path, &tpl_bytes).unwrap();

        let overflow_path = dir.join("overflow.curvebook");
        let overflow_bytes = build_future_archive_bytes("project", 9_999_999_999);
        std::fs::write(&overflow_path, &overflow_bytes).unwrap();

        let unpacked_d = dir.join("future.curvebook.d");
        std::fs::create_dir_all(&unpacked_d).unwrap();
        let manifest_vnext = format!(
            r#"{{"format":"curvebook-archive","schema_version":{future_ver},"kind":"project","title":"Future","document_crc32":0,"sheets":[]}}"#
        );
        let doc_vnext = format!(r#"{{"schema_version":{future_ver},"title":"Future"}}"#);
        std::fs::write(unpacked_d.join("manifest.json"), &manifest_vnext).unwrap();
        std::fs::write(unpacked_d.join("document.json"), &doc_vnext).unwrap();

        let explicit_dir = dir.join("future_unpacked_plain");
        std::fs::create_dir_all(&explicit_dir).unwrap();
        std::fs::write(explicit_dir.join("manifest.json"), &manifest_vnext).unwrap();
        std::fs::write(explicit_dir.join("document.json"), &doc_vnext).unwrap();

        let mut session = new_session("Current Writer");

        let save_err = exec_cmd(&mut session, "project.save", Caller::Cli, json!({ "path": archive_path.to_string_lossy() })).unwrap_err();
        assert_eq!(save_err.code, ErrorCode::UnsupportedVersion);
        assert_eq!(std::fs::read(&archive_path).unwrap(), archive_bytes);

        let overflow_err = exec_cmd(&mut session, "project.save", Caller::Cli, json!({ "path": overflow_path.to_string_lossy() })).unwrap_err();
        assert_eq!(overflow_err.code, ErrorCode::UnsupportedVersion);
        assert_eq!(std::fs::read(&overflow_path).unwrap(), overflow_bytes);

        let tpl_err = exec_cmd(&mut session, "project.saveTemplate", Caller::Cli, json!({ "path": tpl_path.to_string_lossy() })).unwrap_err();
        assert_eq!(tpl_err.code, ErrorCode::UnsupportedVersion);
        assert_eq!(std::fs::read(&tpl_path).unwrap(), tpl_bytes);

        let dir_err = exec_cmd(&mut session, "project.save", Caller::Cli, json!({ "path": unpacked_d.to_string_lossy() })).unwrap_err();
        assert_eq!(dir_err.code, ErrorCode::UnsupportedVersion);
        assert_eq!(std::fs::read_to_string(unpacked_d.join("manifest.json")).unwrap(), manifest_vnext);
        assert_eq!(std::fs::read_to_string(unpacked_d.join("document.json")).unwrap(), doc_vnext);

        let explicit_err =
            exec_cmd(&mut session, "project.save", Caller::Cli, json!({ "path": explicit_dir.to_string_lossy(), "unpacked": true })).unwrap_err();
        assert_eq!(explicit_err.code, ErrorCode::UnsupportedVersion);
        assert_eq!(std::fs::read_to_string(explicit_dir.join("manifest.json")).unwrap(), manifest_vnext);

        let implicit_err = exec_cmd(&mut session, "project.save", Caller::Cli, json!({ "path": explicit_dir.to_string_lossy() })).unwrap_err();
        assert_eq!(implicit_err.code, ErrorCode::UnsupportedVersion);
        assert_eq!(std::fs::read_to_string(explicit_dir.join("manifest.json")).unwrap(), manifest_vnext);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
