//! Shared sheet lookup, table geometry, color, base64, and export helpers for engine commands.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use curvebook_chart::ChartScene;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, DataSetColumn, DataSheet, GraphAnnotation, GraphKind, GraphSheet, GraphSpec, HookSource, InfoSheet, LayoutItem,
    LayoutSheet, LayoutSpec, LinkSource, NormalizeAnchor, ParameterScope, ProjectDocument, ResultCell, ResultSheet, ResultTab, SheetId,
    TableArchetype, TransformAnalysisSpec, VariableRole, validate_title,
};
use curvebook_palette::Rgba8;
use serde_json::json;

use crate::{EngineError, ErrorCode};

/// Parse and validate a non-empty [`SheetId`].
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if `raw` is not a valid sheet identifier.
pub fn parse_sheet_id(raw: &str) -> Result<SheetId, EngineError> {
    SheetId::try_new(raw).map_err(EngineError::from)
}

/// Return `true` if `id` is used by any sheet in `doc`.
#[must_use]
pub fn sheet_id_exists(doc: &ProjectDocument, id: &SheetId) -> bool {
    doc.data_sheet(id).is_some()
        || doc.info_sheet(id).is_some()
        || doc.result_sheet(id).is_some()
        || doc.graph_sheet(id).is_some()
        || doc.layout_sheet(id).is_some()
}

fn sanitize_id_prefix(raw: &str) -> String {
    let cleaned: String = raw.trim().chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' { ch } else { '_' }).collect();
    if cleaned.trim_matches('_').is_empty() { String::from("sheet") } else { cleaned }
}

/// Generate a unique [`SheetId`] with `prefix` (e.g. `data_1`, `data_2`).
///
/// # Errors
/// Returns [`EngineError`] if no unique identifier could be allocated within bounds.
pub fn next_unique_sheet_id(doc: &ProjectDocument, prefix: &str) -> Result<SheetId, EngineError> {
    let base = sanitize_id_prefix(prefix);
    for idx in 1..=100_000_usize {
        let candidate = SheetId::try_new(format!("{base}_{idx}"))?;
        if !sheet_id_exists(doc, &candidate) {
            return Ok(candidate);
        }
    }
    Err(EngineError::with_code(ErrorCode::ResourceLimit, format!("exhausted unique sheet IDs for prefix `{prefix}`")))
}

/// Generate a unique [`SheetId`] based on `candidate` that does not collide with `doc` or `exclude`.
///
/// # Errors
/// Returns [`EngineError`] if no unique identifier could be allocated within bounds.
pub fn next_unique_sheet_id_excluding(doc: &ProjectDocument, candidate: &str, exclude: &[&SheetId]) -> Result<SheetId, EngineError> {
    let base = sanitize_id_prefix(candidate);
    let direct = SheetId::try_new(&base)?;
    if !sheet_id_exists(doc, &direct) && !exclude.contains(&&direct) {
        return Ok(direct);
    }
    for idx in 2..=100_000_usize {
        let trial = SheetId::try_new(format!("{base}_{idx}"))?;
        if !sheet_id_exists(doc, &trial) && !exclude.contains(&&trial) {
            return Ok(trial);
        }
    }
    Err(EngineError::with_code(ErrorCode::ResourceLimit, format!("exhausted unique sheet IDs for `{candidate}`")))
}

fn not_found_sheet_err(kind: &str, id: &SheetId) -> EngineError {
    EngineError::with_code(ErrorCode::NotFound, format!("{kind} `{id}` not found"))
        .with_details(json!({ "sheet_id": id.as_str(), "sheet_kind": kind }))
}

/// Look up an immutable [`DataSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no data sheet has `id`.
pub fn require_data_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a DataSheet, EngineError> {
    doc.data_sheet(id).ok_or_else(|| not_found_sheet_err("data sheet", id))
}

/// Borrow an analyzable [`DataSheet`] from either `doc.data_sheets` or a [`ResultSheet`]'s
/// `analyzable_data` secondary table (`Transform`, `Normalize`, `Transpose`, etc.).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if no sheet exists with `id`, or
/// [`ErrorCode::Conflict`] if `id` names a terminal [`ResultSheet`] with no analyzable table.
pub fn require_analyzable_data_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a DataSheet, EngineError> {
    if let Some(ds) = doc.data_sheet(id) {
        return Ok(ds);
    }
    if let Some(res) = doc.result_sheet(id) {
        return res.analyzable_data.as_ref().ok_or_else(|| {
            EngineError::with_code(
                ErrorCode::Conflict,
                format!("result sheet `{id}` has no primary analyzable table; extract a tabular result tab first with `sheet.extractResultTab`"),
            )
            .with_remediation("Use `sheet.extractResultTab` to copy a ResultTab into a reactive DataSheet, or choose an analyzable ResultSheet.")
            .with_details(json!({ "sheet_id": id.as_str() }))
        });
    }
    Err(not_found_sheet_err("data or analyzable result sheet", id))
}

/// Look up a mutable [`DataSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no data sheet has `id`.
pub fn require_data_sheet_mut<'a>(doc: &'a mut ProjectDocument, id: &SheetId) -> Result<&'a mut DataSheet, EngineError> {
    doc.data_sheet_mut(id).ok_or_else(|| not_found_sheet_err("data sheet", id))
}

/// Fail with [`ErrorCode::FrozenSheet`] if `sheet` is frozen.
///
/// # Errors
/// Returns [`EngineError`] when `sheet.frozen` is `true`.
pub fn ensure_sheet_not_frozen(sheet: &DataSheet) -> Result<(), EngineError> {
    if sheet.frozen {
        return Err(EngineError::with_code(ErrorCode::FrozenSheet, format!("data sheet `{}` is frozen", sheet.id))
            .with_details(json!({ "sheet_id": sheet.id.as_str() }))
            .with_remediation("Unfreeze the sheet before editing its contents or links."));
    }
    Ok(())
}

/// Look up a mutable [`DataSheet`] and fail if it does not exist or is frozen.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] or [`ErrorCode::FrozenSheet`].
pub fn require_unfrozen_data_sheet_mut<'a>(doc: &'a mut ProjectDocument, id: &SheetId) -> Result<&'a mut DataSheet, EngineError> {
    let sheet = require_data_sheet_mut(doc, id)?;
    ensure_sheet_not_frozen(sheet)?;
    Ok(sheet)
}

/// Look up an immutable [`InfoSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no info sheet has `id`.
pub fn require_info_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a InfoSheet, EngineError> {
    doc.info_sheet(id).ok_or_else(|| not_found_sheet_err("info sheet", id))
}

/// Look up a mutable [`InfoSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no info sheet has `id`.
pub fn require_info_sheet_mut<'a>(doc: &'a mut ProjectDocument, id: &SheetId) -> Result<&'a mut InfoSheet, EngineError> {
    doc.info_sheet_mut(id).ok_or_else(|| not_found_sheet_err("info sheet", id))
}

/// Resolve either an [`InfoSheet`] ID directly or a [`DataSheet`] ID whose paired `info_sheet_id` exists.
///
/// # Errors
/// Returns [`EngineError`] if `raw_id` is empty/invalid or no matching info sheet exists.
pub fn resolve_target_info_sheet_id(doc: &ProjectDocument, raw_id: &str) -> Result<SheetId, EngineError> {
    let id = parse_sheet_id(raw_id)?;
    if doc.info_sheet(&id).is_some() {
        return Ok(id);
    }
    if let Some(ds) = doc.data_sheet(&id)
        && let Some(paired) = &ds.info_sheet_id
        && doc.info_sheet(paired).is_some()
    {
        return Ok(paired.clone());
    }
    Err(not_found_sheet_err("info sheet", &id))
}

/// Locate vector indices of a target [`DataSheet`] and optional paired [`InfoSheet`] for split borrows.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if the data sheet is missing.
pub fn locate_target_and_info_indices(
    doc: &ProjectDocument,
    target_id: &SheetId,
    info_id: Option<&SheetId>,
) -> Result<(usize, Option<usize>), EngineError> {
    let target_idx = doc.data_sheets.iter().position(|d| &d.id == target_id).ok_or_else(|| not_found_sheet_err("data sheet", target_id))?;
    let info_idx = info_id.and_then(|iid| doc.info_sheets.iter().position(|i| &i.id == iid));
    Ok((target_idx, info_idx))
}

/// Look up an immutable [`ResultSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no result sheet has `id`.
pub fn require_result_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a ResultSheet, EngineError> {
    doc.result_sheet(id).ok_or_else(|| not_found_sheet_err("result sheet", id))
}

/// Look up a mutable [`ResultSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no result sheet has `id`.
pub fn require_result_sheet_mut<'a>(doc: &'a mut ProjectDocument, id: &SheetId) -> Result<&'a mut ResultSheet, EngineError> {
    doc.result_sheet_mut(id).ok_or_else(|| not_found_sheet_err("result sheet", id))
}

/// Select a tab by ID or title from a [`ResultSheet`], defaulting to the first tab.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if the requested tab is absent or sheet has no tabs.
pub fn select_result_tab<'a>(res: &'a ResultSheet, tab_id: Option<&str>) -> Result<&'a ResultTab, EngineError> {
    if let Some(tid) = tab_id {
        res.tabs.iter().find(|t| t.id == tid || t.title == tid).ok_or_else(|| {
            EngineError::with_code(ErrorCode::NotFound, format!("result tab `{tid}` not found"))
                .with_details(json!({ "sheet_id": res.id.as_str(), "tab_id": tid }))
        })
    } else {
        res.tabs.first().ok_or_else(|| {
            EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{}` has no tabs", res.id))
                .with_details(json!({ "sheet_id": res.id.as_str() }))
        })
    }
}

/// Look up an immutable [`GraphSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no graph sheet has `id`.
pub fn require_graph_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a GraphSheet, EngineError> {
    doc.graph_sheet(id).ok_or_else(|| not_found_sheet_err("graph sheet", id))
}

/// Look up a mutable [`GraphSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no graph sheet has `id`.
pub fn require_graph_sheet_mut<'a>(doc: &'a mut ProjectDocument, id: &SheetId) -> Result<&'a mut GraphSheet, EngineError> {
    doc.graph_sheet_mut(id).ok_or_else(|| not_found_sheet_err("graph sheet", id))
}

/// Look up an immutable [`LayoutSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no layout sheet has `id`.
pub fn require_layout_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Result<&'a LayoutSheet, EngineError> {
    doc.layout_sheet(id).ok_or_else(|| not_found_sheet_err("layout sheet", id))
}

/// Look up a mutable [`LayoutSheet`] or return [`ErrorCode::NotFound`].
///
/// # Errors
/// Returns [`EngineError`] if no layout sheet has `id`.
pub fn require_layout_sheet_mut<'a>(doc: &'a mut ProjectDocument, id: &SheetId) -> Result<&'a mut LayoutSheet, EngineError> {
    doc.layout_sheet_mut(id).ok_or_else(|| not_found_sheet_err("layout sheet", id))
}

/// Number of X subcolumns supported by `arch`.
#[must_use]
pub fn x_subcolumn_count(arch: &TableArchetype) -> usize {
    match arch {
        TableArchetype::Xy(t) => t.x_cells.first().map_or(1, Vec::len).max(1),
        TableArchetype::Survival(_) => 1,
        _ => 0,
    }
}

/// Number of subcolumns in Y column `col` of `arch`.
#[must_use]
pub fn column_subcolumn_count(arch: &TableArchetype, col: usize) -> usize {
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

/// Compile a graph or layout [`ChartScene`] from `doc` for rendering/export.
///
/// # Errors
/// Returns [`EngineError`] if the sheet is missing, invalid, or fails compilation.
pub fn compile_sheet_scene(doc: &ProjectDocument, sheet_id: &SheetId) -> Result<ChartScene, EngineError> {
    crate::guard::run_guarded("compile_sheet_scene", || {
        if let Some(graph) = doc.graph_sheet(sheet_id) {
            return Ok(curvebook_chart::compile_graph(graph, doc)?);
        }
        if let Some(layout) = doc.layout_sheet(sheet_id) {
            return Ok(curvebook_chart::compile_layout(layout, doc)?);
        }
        Err(EngineError::with_code(ErrorCode::NotFound, format!("sheet `{sheet_id}` is not a graph or layout sheet"))
            .with_details(json!({ "sheet_id": sheet_id.as_str() })))
    })
}

/// Write exported bytes to a filesystem path, mapping I/O errors to structured [`EngineError`].
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] for empty paths or [`ErrorCode::IoError`] on failure.
pub fn write_output_file(path: &str, bytes: &[u8]) -> Result<(), EngineError> {
    if path.trim().is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "output path must not be empty"));
    }
    let out = Path::new(path);
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|err| {
            EngineError::with_code(ErrorCode::IoError, format!("failed to create output directory `{}`: {err}", parent.display()))
                .with_retryable(true)
                .with_details(json!({ "path": path }))
        })?;
    }
    std::fs::write(out, bytes).map_err(|err| {
        EngineError::with_code(ErrorCode::IoError, format!("failed to write output file `{path}`: {err}"))
            .with_retryable(true)
            .with_details(json!({ "path": path }))
    })
}

/// Encode binary bytes into standard RFC 4648 base64 without line wrapping.
#[must_use]
pub fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3).saturating_mul(4));
    for chunk in input.chunks(3) {
        let b0 = usize::from(chunk[0]);
        let b1 = usize::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = usize::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[(triple >> 18) & 0x3f]));
        out.push(char::from(TABLE[(triple >> 12) & 0x3f]));
        if chunk.len() > 1 {
            out.push(char::from(TABLE[(triple >> 6) & 0x3f]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(TABLE[triple & 0x3f]));
        } else {
            out.push('=');
        }
    }
    out
}

fn decode_b64_sextet(byte: u8) -> Result<u8, EngineError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid base64 character `{}`", char::from(byte)))),
    }
}

/// Decode standard RFC 4648 base64 string (ignoring ASCII whitespace).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if length or characters are invalid.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, EngineError> {
    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !cleaned.len().is_multiple_of(4) {
        return Err(EngineError::with_code(ErrorCode::BadInput, "base64 payload length must be a multiple of 4"));
    }
    let (chunks, _) = cleaned.as_chunks::<4>();
    let mut out = Vec::with_capacity(chunks.len().saturating_mul(3));
    for (idx, chunk) in chunks.iter().enumerate() {
        let is_last = idx.saturating_add(1) == chunks.len();
        if (chunk[0] == b'=' || chunk[1] == b'=') || (!is_last && (chunk[2] == b'=' || chunk[3] == b'=')) {
            return Err(EngineError::with_code(ErrorCode::BadInput, "misplaced base64 `=` padding"));
        }
        if chunk[2] == b'=' && chunk[3] != b'=' {
            return Err(EngineError::with_code(ErrorCode::BadInput, "malformed base64 trailing padding"));
        }
        let s0 = u32::from(decode_b64_sextet(chunk[0])?);
        let s1 = u32::from(decode_b64_sextet(chunk[1])?);
        let pad_len = usize::from(chunk[2] == b'=') + usize::from(chunk[3] == b'=');
        let s2 = if chunk[2] == b'=' { 0 } else { u32::from(decode_b64_sextet(chunk[2])?) };
        let s3 = if chunk[3] == b'=' { 0 } else { u32::from(decode_b64_sextet(chunk[3])?) };
        let word = (s0 << 18) | (s1 << 12) | (s2 << 6) | s3;
        out.push(((word >> 16) & 0xff) as u8);
        if pad_len < 2 {
            out.push(((word >> 8) & 0xff) as u8);
        }
        if pad_len == 0 {
            out.push((word & 0xff) as u8);
        }
    }
    Ok(out)
}

/// Parse a named color (`black`, `white`, `transparent`, `red`, `blue`, `green`, `orange`, `purple`)
/// or hex color (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if `raw` is malformed.
pub fn parse_color_string(raw: &str) -> Result<Rgba8, EngineError> {
    let trimmed = raw.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "black" => return Ok(Rgba8::BLACK),
        "white" => return Ok(Rgba8::WHITE),
        "transparent" => return Ok(Rgba8::TRANSPARENT),
        "red" => return Ok(Rgba8::rgb(214, 39, 40)),
        "blue" => return Ok(Rgba8::rgb(31, 119, 180)),
        "green" => return Ok(Rgba8::rgb(44, 160, 44)),
        "orange" => return Ok(Rgba8::rgb(255, 127, 14)),
        "purple" => return Ok(Rgba8::rgb(148, 103, 189)),
        _ => {}
    }
    let hex = trimmed.strip_prefix('#').ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{raw}`")))?;
    if !hex.is_ascii() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{raw}`: non-ASCII characters")));
    }
    match hex.len() {
        3 | 4 => parse_short_hex(hex, raw),
        6 | 8 => parse_long_hex(hex, raw),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{raw}`: expected 3, 4, 6, or 8 hex digits"))),
    }
}

fn parse_short_hex(hex: &str, orig: &str) -> Result<Rgba8, EngineError> {
    let bytes = hex.as_bytes();
    let nibble = |idx: usize| -> Result<u8, EngineError> {
        let digit =
            char::from(bytes[idx]).to_digit(16).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{orig}`")))?;
        let v = digit as u8;
        Ok((v << 4) | v)
    };
    let r = nibble(0)?;
    let g = nibble(1)?;
    let b = nibble(2)?;
    let a = if bytes.len() == 4 { nibble(3)? } else { 255 };
    Ok(Rgba8::rgba(r, g, b, a))
}

fn parse_long_hex(hex: &str, orig: &str) -> Result<Rgba8, EngineError> {
    let pair = |start: usize| -> Result<u8, EngineError> {
        let slice = hex.get(start..start + 2).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{orig}`")))?;
        u8::from_str_radix(slice, 16).map_err(|_| EngineError::with_code(ErrorCode::BadInput, format!("invalid hex color `{orig}`")))
    };
    let r = pair(0)?;
    let g = pair(2)?;
    let b = pair(4)?;
    let a = if hex.len() == 8 { pair(6)? } else { 255 };
    Ok(Rgba8::rgba(r, g, b, a))
}

/// Replace `id` in place if `id_map` contains a mapping for it.
pub(super) fn remap_sheet_id(id: &mut SheetId, id_map: &HashMap<SheetId, SheetId>) {
    if let Some(mapped) = id_map.get(id) {
        *id = mapped.clone();
    }
}

/// Remap explicit upstream `SheetId` targets inside a [`HookSource`].
pub(super) fn remap_hook_source(source: &mut HookSource, id_map: &HashMap<SheetId, SheetId>) {
    match source {
        HookSource::InfoConstant { info_sheet_id: Some(iid), .. } => remap_sheet_id(iid, id_map),
        HookSource::ResultScalar { result_sheet_id, .. } => remap_sheet_id(result_sheet_id, id_map),
        HookSource::ColumnTitleValue { data_sheet_id, .. } => remap_sheet_id(data_sheet_id, id_map),
        HookSource::InfoConstant { info_sheet_id: None, .. } => {}
    }
}

/// Remap embedded [`HookSource`] references inside `spec` (`Normalize` anchors and `NonlinearRegression` rules).
pub(super) fn remap_analysis_hooks(spec: &mut AnalysisSpec, id_map: &HashMap<SheetId, SheetId>) {
    match &mut spec.kind {
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. }) => {
            if let NormalizeAnchor::Hooked(hook) = zero_anchor {
                remap_hook_source(hook, id_map);
            }
            if let NormalizeAnchor::Hooked(hook) = hundred_anchor {
                remap_hook_source(hook, id_map);
            }
        }
        AnalysisKind::NonlinearRegression(nl) => {
            for rule in &mut nl.parameter_rules {
                if let ParameterScope::HookedConstant(source) = &mut rule.scope {
                    remap_hook_source(source, id_map);
                }
            }
        }
        AnalysisKind::PowerAnalysis(p) => {
            if let Some(ref mut info_id) = p.info_sheet_id {
                remap_sheet_id(info_id, id_map);
            }
        }
        _ => {}
    }
}

/// Remap [`LinkSource::InternalSheet`] references on a [`DataSheet`] (`info_sheet_id` and `linked_regions`).
pub(super) fn remap_data_sheet_links(ds: &mut DataSheet, id_map: &HashMap<SheetId, SheetId>) {
    if let Some(ref mut info_id) = ds.info_sheet_id {
        remap_sheet_id(info_id, id_map);
    }
    for region in &mut ds.linked_regions {
        if let LinkSource::InternalSheet { sheet_id, .. } = &mut region.spec.source {
            remap_sheet_id(sheet_id, id_map);
        }
    }
}

/// Rewrite explicit `{{info:<sheet_id>:<name>}}` and `{{result:<sheet_id>:...}}` template tokens using `id_map`.
pub(super) fn remap_inline_hook_tokens(text: &mut String, id_map: &HashMap<SheetId, SheetId>) {
    if id_map.is_empty() || !text.contains("{{") {
        return;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    let mut changed = false;

    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("}}") else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let inner = &after_open[..close];
        if let Some(rewritten) = rewrite_single_hook_token(inner.trim(), id_map) {
            out.push_str("{{");
            out.push_str(&rewritten);
            out.push_str("}}");
            changed = true;
        } else {
            out.push_str(&rest[open..open + 2 + close + 2]);
        }
        rest = &after_open[close + 2..];
    }
    out.push_str(rest);
    if changed {
        *text = out;
    }
}

fn rewrite_single_hook_token(token: &str, id_map: &HashMap<SheetId, SheetId>) -> Option<String> {
    let parts: Vec<&str> = token.split(':').map(str::trim).collect();
    match parts.as_slice() {
        ["info", sheet, name] if !sheet.is_empty() && !name.is_empty() => {
            let mapped = id_map.get(&SheetId::new(*sheet))?;
            Some(format!("info:{}:{}", mapped.as_str(), name))
        }
        ["result", sheet, param] if !sheet.is_empty() && !param.is_empty() => {
            let mapped = id_map.get(&SheetId::new(*sheet))?;
            Some(format!("result:{}:{}", mapped.as_str(), param))
        }
        ["result", sheet, ds_str, param] if !sheet.is_empty() && !param.is_empty() && ds_str.parse::<usize>().is_ok() => {
            let mapped = id_map.get(&SheetId::new(*sheet))?;
            Some(format!("result:{}:{}:{}", mapped.as_str(), ds_str, param))
        }
        ["scalar", sheet, rest @ ..] if !sheet.is_empty() && !rest.is_empty() => {
            let mapped = id_map.get(&SheetId::new(*sheet))?;
            Some(format!("scalar:{}:{}", mapped.as_str(), rest.join(":")))
        }
        _ => None,
    }
}

/// Remap all internal sheet IDs and inline template hook tokens inside a [`GraphSpec`].
pub(super) fn remap_graph_spec_references(spec: &mut GraphSpec, id_map: &HashMap<SheetId, SheetId>) {
    remap_sheet_id(&mut spec.primary_source_sheet_id, id_map);
    for series in &mut spec.series {
        remap_sheet_id(&mut series.source_sheet_id, id_map);
        if let Some(ov) = &mut series.regression_overlay {
            remap_sheet_id(&mut ov.result_sheet_id, id_map);
        }
        if let Some(label) = &mut series.legend_label {
            remap_inline_hook_tokens(label, id_map);
        }
    }
    for cmp in &mut spec.comparisons {
        remap_sheet_id(&mut cmp.result_sheet_id, id_map);
    }
    if let Some(cld) = &mut spec.compact_letter_display {
        remap_sheet_id(&mut cld.result_sheet_id, id_map);
    }
    for ann in &mut spec.annotations {
        match ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                remap_sheet_id(result_sheet_id, id_map);
            }
            GraphAnnotation::TextBox { markdown, .. } => remap_inline_hook_tokens(markdown, id_map),
            GraphAnnotation::LineOrArrow { .. } | GraphAnnotation::Shape { .. } => {}
        }
    }
    remap_inline_hook_tokens(&mut spec.title, id_map);
    remap_inline_hook_tokens(&mut spec.x_axis.title, id_map);
    remap_inline_hook_tokens(&mut spec.y_axis.title, id_map);
    if let Some(y2) = &mut spec.y2_axis {
        remap_inline_hook_tokens(&mut y2.title, id_map);
    }
}

/// Sanitize ASCII control characters into spaces or drop them for table titles and labels.
pub(crate) fn sanitize_title_controls(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '\n' | '\r' | '\t') {
            out.push(' ');
        } else if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// Clamp and sanitize a label/header string into a valid [`DataSetColumn`]/row title.
pub(crate) fn clip_safe_title(raw: &str) -> String {
    let cleaned = sanitize_title_controls(raw);
    if validate_title(&cleaned).is_ok() {
        return cleaned;
    }
    let mut clipped = String::new();
    for ch in cleaned.chars() {
        if clipped.len() + ch.len_utf8() > 180 {
            break;
        }
        clipped.push(ch);
    }
    clipped
}

fn apply_row_labels(row_titles: &mut [String], tab: &ResultTab) {
    for (r, slot) in row_titles.iter_mut().enumerate() {
        let lbl = tab.row_labels.get(r).map(String::as_str).unwrap_or("").trim();
        *slot = if lbl.is_empty() { String::new() } else { clip_safe_title(lbl) };
    }
}

/// Apply column headers, row labels, and inferred text/label variable roles from `tab`
/// onto `sheet`, clearing stale headers/labels if the refreshed tab shrank.
pub(crate) fn apply_result_tab_metadata_to_sheet(sheet: &mut DataSheet, tab: &ResultTab) {
    let has_headers = !tab.column_headers.is_empty();
    let tab_cols = tab.rows.iter().map(Vec::len).max().unwrap_or(0).max(tab.column_headers.len());
    match &mut sheet.archetype {
        TableArchetype::Column(curvebook_doc::ColumnTable { data_sets, row_titles, .. })
        | TableArchetype::Grouped(curvebook_doc::GroupedTable { data_sets, row_titles, .. }) => {
            for (idx, ds) in data_sets.iter_mut().enumerate() {
                if let Some(hdr) = tab.column_headers.get(idx).map(|s| s.trim()) {
                    if !hdr.is_empty() {
                        ds.title = clip_safe_title(hdr);
                    } else if has_headers {
                        ds.title.clear();
                    }
                } else if has_headers || idx >= tab_cols {
                    ds.title.clear();
                }
            }
            apply_row_labels(row_titles, tab);
        }
        TableArchetype::MultipleVariables(mv) => {
            for (idx, var) in mv.variables.iter_mut().enumerate() {
                if let Some(hdr) = tab.column_headers.get(idx).map(|s| s.trim()) {
                    if !hdr.is_empty() {
                        var.title = clip_safe_title(hdr);
                    } else if has_headers {
                        var.title.clear();
                    }
                } else if has_headers || idx >= tab_cols {
                    var.title.clear();
                }
                let col_has_text = tab
                    .rows
                    .iter()
                    .any(|row| matches!(row.get(idx), Some(ResultCell::Text(_) | ResultCell::Boolean(_) | ResultCell::ConfidenceInterval { .. })));
                if col_has_text {
                    var.lock_role(VariableRole::Label);
                }
            }
            apply_row_labels(&mut mv.row_titles, tab);
        }
        _ => {}
    }
}

/// Remap all internal sheet IDs and inline template hook tokens inside a [`LayoutSpec`].
pub(super) fn remap_layout_spec_references(spec: &mut LayoutSpec, id_map: &HashMap<SheetId, SheetId>) {
    for item in &mut spec.items {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => remap_sheet_id(graph_sheet_id, id_map),
            LayoutItem::LiveTable { source_sheet_id, .. } => remap_sheet_id(source_sheet_id, id_map),
            LayoutItem::TextCallout { markdown, .. } => remap_inline_hook_tokens(markdown, id_map),
        }
    }
}

/// Ensure `data_id` is listed in `info_id.linked_data_sheets` without duplicating entries.
pub(super) fn link_data_sheet_to_info(doc: &mut ProjectDocument, info_id: &SheetId, data_id: &SheetId) {
    if let Some(info) = doc.info_sheet_mut(info_id)
        && !info.linked_data_sheets.contains(data_id)
    {
        info.linked_data_sheets.push(data_id.clone());
    }
}

/// Prune `deleted` sheet IDs from all `InfoSheet.linked_data_sheets` lists and ensure every
/// [`DataSheet`] with `info_sheet_id = Some(iid)` is recorded in `iid.linked_data_sheets`.
pub(super) fn sync_info_linked_data_sheets(doc: &mut ProjectDocument, deleted: &HashSet<SheetId>) {
    if !deleted.is_empty() {
        for info in &mut doc.info_sheets {
            info.linked_data_sheets.retain(|id| !deleted.contains(id));
        }
    }
    let pairs: Vec<(SheetId, SheetId)> =
        doc.data_sheets.iter().filter_map(|ds| ds.info_sheet_id.as_ref().map(|iid| (iid.clone(), ds.id.clone()))).collect();
    for (iid, ds_id) in pairs {
        link_data_sheet_to_info(doc, &iid, &ds_id);
    }
}

fn find_backing_data_sheet<'a>(data_sheets: &'a [DataSheet], result_sheets: &'a [ResultSheet], id: &SheetId) -> Option<&'a DataSheet> {
    data_sheets.iter().find(|d| &d.id == id).or_else(|| result_sheets.iter().find(|r| &r.id == id).and_then(|r| r.analyzable_data.as_ref()))
}

fn point_override_bounds_parts(
    data_sheets: &[DataSheet],
    result_sheets: &[ResultSheet],
    spec: &GraphSpec,
    series_index: usize,
) -> Result<(usize, usize), EngineError> {
    let result_row_upper = |res: &ResultSheet| -> usize {
        let mut cursor = &res.analysis.source_sheet_id;
        let mut origin_rows = 0usize;
        let mut seen = Vec::new();
        while !seen.contains(&cursor) {
            seen.push(cursor);
            if let Some(ds) = find_backing_data_sheet(data_sheets, result_sheets, cursor) {
                origin_rows = ds.archetype.row_count();
                break;
            }
            if let Some(upstream) = result_sheets.iter().find(|r| &r.id == cursor) {
                cursor = &upstream.analysis.source_sheet_id;
            } else {
                break;
            }
        }
        let tab_rows = res.tabs.iter().map(|t| t.rows.len()).max().unwrap_or(0);
        origin_rows.max(res.pairwise_comparisons.len()).max(tab_rows)
    };

    let result_col_and_subcols = |res: &ResultSheet, col: usize| -> (usize, usize) {
        let mut cursor = &res.analysis.source_sheet_id;
        let mut seen = Vec::new();
        while !seen.contains(&cursor) {
            seen.push(cursor);
            if let Some(ds) = find_backing_data_sheet(data_sheets, result_sheets, cursor) {
                return (ds.archetype.column_count().max(1), column_subcolumn_count(&ds.archetype, col));
            }
            if let Some(upstream) = result_sheets.iter().find(|r| &r.id == cursor) {
                cursor = &upstream.analysis.source_sheet_id;
            } else {
                break;
            }
        }
        (1, 1)
    };

    if !spec.series.is_empty() {
        let Some(series) = spec.series.iter().find(|s| s.data_set_index == series_index) else {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("`series_index` {series_index} is out of bounds for this graph's plotted series"),
            ));
        };
        if let Some(ds) = find_backing_data_sheet(data_sheets, result_sheets, &series.source_sheet_id) {
            return Ok((ds.archetype.row_count(), column_subcolumn_count(&ds.archetype, series.data_set_index)));
        }
        if let Some(res) = result_sheets.iter().find(|r| r.id == series.source_sheet_id) {
            let (_cols, subcols) = result_col_and_subcols(res, series.data_set_index);
            return Ok((result_row_upper(res), subcols));
        }
        return Err(EngineError::with_code(
            ErrorCode::NotFound,
            format!("source sheet `{}` for series `{series_index}` was not found", series.source_sheet_id),
        ));
    }

    match spec.kind {
        GraphKind::MultipleVariables => {
            if let Some(ds) = find_backing_data_sheet(data_sheets, result_sheets, &spec.primary_source_sheet_id) {
                if series_index >= ds.archetype.column_count() {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("`series_index` {series_index} exceeds column count {}", ds.archetype.column_count()),
                    ));
                }
                return Ok((ds.archetype.row_count(), column_subcolumn_count(&ds.archetype, series_index)));
            }
        }
        GraphKind::Diagnostic(_) => {
            if let Some(ds) = find_backing_data_sheet(data_sheets, result_sheets, &spec.primary_source_sheet_id) {
                if series_index >= ds.archetype.column_count().max(1) {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("`series_index` {series_index} exceeds column count {}", ds.archetype.column_count()),
                    ));
                }
                return Ok((ds.archetype.row_count(), column_subcolumn_count(&ds.archetype, series_index)));
            }
            if let Some(res) = result_sheets.iter().find(|r| r.id == spec.primary_source_sheet_id) {
                let (cols, subcols) = result_col_and_subcols(res, series_index);
                if series_index >= cols {
                    return Err(EngineError::with_code(
                        ErrorCode::BadInput,
                        format!("`series_index` {series_index} exceeds diagnostic source column count {cols}"),
                    ));
                }
                return Ok((result_row_upper(res), subcols));
            }
        }
        _ => {}
    }

    Err(EngineError::with_code(ErrorCode::BadInput, format!("`series_index` {series_index} is out of bounds: graph has no matching plotted series")))
}

/// Validate `(series_index, point_index, subcol)` against the target graph's plotted series and table bounds.
pub(super) fn validate_point_override_in_bounds(
    doc: &ProjectDocument,
    spec: &GraphSpec,
    series_index: usize,
    point_index: usize,
    subcol: Option<usize>,
) -> Result<(), EngineError> {
    let (row_count, subcol_count) = point_override_bounds_parts(&doc.data_sheets, &doc.result_sheets, spec, series_index)?;
    if point_index >= row_count {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("`point_index` {point_index} is out of bounds for series `{series_index}` (row_count={row_count})"),
        ));
    }
    if let Some(sc) = subcol
        && sc >= subcol_count
    {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("`subcol` {sc} is out of bounds for series `{series_index}` (subcol_count={subcol_count})"),
        ));
    }
    Ok(())
}

/// Remove any `PointStyleOverride` entries whose `(series_index, point_index, subcol)` is now out of bounds.
pub(super) fn prune_out_of_bounds_point_overrides(doc: &mut ProjectDocument) {
    let ProjectDocument { data_sheets, result_sheets, graph_sheets, .. } = doc;
    for gs in graph_sheets {
        if gs.spec.point_overrides.is_empty() {
            continue;
        }
        let spec_snapshot = gs.spec.clone();
        gs.spec.point_overrides.retain(|ov| {
            let Ok((row_count, subcol_count)) = point_override_bounds_parts(data_sheets, result_sheets, &spec_snapshot, ov.series_index) else {
                return false;
            };
            ov.point_index < row_count && ov.subcol.is_none_or(|sc| sc < subcol_count)
        });
    }
}

fn active_series_row_len(ds: &DataSheet, col: usize) -> usize {
    let rows = ds.archetype.row_count();
    let subcols = column_subcolumn_count(&ds.archetype, col);
    for r in (0..rows).rev() {
        let has_y = (0..subcols).any(|s| ds.archetype.data_cell(curvebook_doc::CellCoord::new(r, col, s)).is_some_and(|c| !c.is_empty()));
        let has_x = match &ds.archetype {
            TableArchetype::Xy(xy) => match &xy.x_mode {
                curvebook_doc::XColumnMode::GeneratedSequence { .. } => true,
                _ => xy.x_cells.get(r).is_some_and(|row| row.iter().any(|c| !c.is_empty())),
            },
            TableArchetype::Survival(sv) => sv.times.get(r).is_some_and(|c| !c.is_empty()),
            _ => true,
        };
        if has_y && has_x {
            return r + 1;
        }
    }
    0
}

/// Record the active non-empty trailing length of each dataset column in `ds` prior to a cell edit.
pub(super) fn snapshot_active_series_lengths(doc: &ProjectDocument, sheet_id: &SheetId) -> Vec<usize> {
    let Some(ds) = doc.data_sheet(sheet_id) else {
        return Vec::new();
    };
    (0..ds.archetype.column_count()).map(|col| active_series_row_len(ds, col)).collect()
}

/// Prune point overrides targeting trailing rows of `sheet_id` when an edit shortened active data.
pub(super) fn prune_shortened_series_overrides(doc: &mut ProjectDocument, sheet_id: &SheetId, before_active: &[usize]) {
    let Some(ds) = doc.data_sheet(sheet_id) else {
        return;
    };
    let after_active: Vec<usize> = (0..ds.archetype.column_count()).map(|col| active_series_row_len(ds, col)).collect();
    let mut shortened_limits: HashMap<usize, usize> = HashMap::new();
    for (col, &before_len) in before_active.iter().enumerate() {
        let after_len = after_active.get(col).copied().unwrap_or(0);
        if after_len < before_len {
            shortened_limits.insert(col, after_len);
        }
    }
    if shortened_limits.is_empty() {
        return;
    }
    for gs in &mut doc.graph_sheets {
        if gs.spec.point_overrides.is_empty() {
            continue;
        }
        let series_sources: HashMap<usize, &SheetId> = gs.spec.series.iter().map(|s| (s.data_set_index, &s.source_sheet_id)).collect();
        let primary_is_sheet = &gs.spec.primary_source_sheet_id == sheet_id;
        gs.spec.point_overrides.retain(|ov| {
            let targets_sheet = series_sources.get(&ov.series_index).map_or(primary_is_sheet && gs.spec.series.is_empty(), |src| *src == sheet_id);
            if !targets_sheet {
                return true;
            }
            shortened_limits.get(&ov.series_index).is_none_or(|&limit| ov.point_index < limit)
        });
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument};

    use super::*;

    #[test]
    fn lookup_and_frozen_helpers_emit_structured_codes() {
        let mut doc = ProjectDocument::default();
        let id = next_unique_sheet_id(&doc, "data").expect("id should generate");
        assert_eq!(id.as_str(), "data_1");

        let err = require_data_sheet(&doc, &id).expect_err("missing sheet");
        assert_eq!(err.code, ErrorCode::NotFound);

        let table = ColumnTable::new(ColumnDesignHint::default(), 2, 4).expect("column table");
        let mut ds = DataSheet::new(id.clone(), "Sample", TableArchetype::Column(table)).expect("valid data sheet");
        ds.frozen = true;
        doc.add_data_sheet(ds).expect("add data sheet");

        let frozen_err = require_unfrozen_data_sheet_mut(&mut doc, &id).expect_err("frozen sheet");
        assert_eq!(frozen_err.code, ErrorCode::FrozenSheet);
    }

    #[test]
    fn base64_and_color_roundtrip() {
        let payload = b"hello\x00world!";
        let encoded = encode_base64(payload);
        assert_eq!(decode_base64(&encoded).expect("decode"), payload);
        assert_eq!(decode_base64("abc").expect_err("bad len").code, ErrorCode::BadInput);

        assert_eq!(parse_color_string("#0F8").expect("short hex"), Rgba8::rgba(0x00, 0xFF, 0x88, 255));
        assert_eq!(parse_color_string("#11223344").expect("long hex"), Rgba8::rgba(0x11, 0x22, 0x33, 0x44));
        assert!(parse_color_string("not-a-color").expect_err("bad color").to_string().contains("invalid hex"));
    }
}
