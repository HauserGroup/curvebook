//! Full clean-room `.pzfx` XML project importer (§4.1).
//!
//! Capabilities:
//! - Automatic encoding detection (`UTF-8` with/without BOM, `UTF-16LE`/`UTF-16BE` with/without
//!   BOM, and declared `ISO-8859-1`, `Windows-1252`, `MacRoman`) via [`parse_xml_bytes_with_options`].
//! - Stop-after-root parsing and streaming opaque `<Template>` skipping up to 128 MiB raw input
//!   (reporting skipped opaque and/or trailing bytes in a single [`ImportWarningKind::IgnoredSection`]
//!   warning).
//! - Unwrapping XSLT-wrapped `.pzfx` payloads (`<xsl:stylesheet>` direct child root) and local-name
//!   matching across default and prefixed XML namespaces.
//! - `<Table>`, `<HugeTable>`, and `<Table1024>` plus `ExtTableType="Nested"` and
//!   `ExtTableType="MultipleVariables"` (including `YColumn@MVType="Categorical"|"Continues"`).
//! - Native v9 2-subcolumn summary modes (`SD` -> [`SubcolumnMode::MeanSd`], `SE`/`SEM` ->
//!   [`SubcolumnMode::MeanSem`]) on both Y columns and `XFormat="error"` X columns, while keeping
//!   2-subcolumn `%CV` lossless in [`SubcolumnMode::MeanCvN`] with empty `N` and a typed
//!   [`ImportWarningKind::MissingMetadata`] warning.
//! - Rich-text conversion (`<b>`, `<i>`, `<sup>`, `<sub>`, `<br/>`, `<Font Face="Symbol">` via
//!   `SYMBOL.TXT`), strict ISO-8601 `ConstantValue::Date`, `<SubColumnTitles>`, `Decimals` (`0..=15`),
//!   `<TableSequence>` / `<InfoSequence>` ordering & `Selected="1"` selection, per-table embedded
//!   `<Table><Info ID="...">` links, and colour tags (`Table@Highlighted`, `FloatingNote@Color`).

use std::collections::{HashMap, HashSet};

use curvebook_doc::error::MAX_SHEET_ID_BYTES;
use curvebook_doc::{
    Cell, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSetColumn, DataSheet, FloatingNote, GroupedTable,
    InfoConstant, InfoSheet, MAX_DISPLAY_DECIMALS, MAX_TITLE_BYTES, MultipleVariablesTable, NestedGroupColumn, NestedTable, PartsOfWholeTable,
    ProjectDocument, RectPt, SequenceProgression, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, VariableRole, XColumnMode,
    XyTable, default_column_label, validate_data_sets, validate_rows, validate_subcolumns,
};

use super::export::PZFX_ROOT_ELEMENT;
use super::richtext::{
    attr_ci, extract_pzfx_color_tag, extract_pzfx_table_color_tag, parse_pzfx_constant_date, pzfx_element_to_markup, pzfx_element_to_plain_text,
};
use crate::archive::{validate_archive_save_limits, validate_project_document};
use crate::error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
use crate::xml::{MAX_XML_OPAQUE_BYTES, ParsedXml, XmlElement, XmlLimits, XmlParseOptions, parse_xml_bytes_with_options, parse_xml_with_options};

/// Maximum total dense `Cell` entries allocated across all tables of a single `.pzfx` import.
pub const MAX_IMPORT_CELLS: usize = crate::csv::MAX_IMPORT_CELLS;

/// Leading ASCII magic bytes identifying legacy binary `.pzf` files.
pub const BINARY_PZF_MAGIC: &[u8; 4] = b"PCFF";

/// Canonical refusal message for unsupported legacy binary `.pzf` inputs.
pub const BINARY_PZF_UNSUPPORTED_MESSAGE: &str =
    "This file uses the older binary `.pzf` format, which CurveBook doesn't read. Re-save it as `.pzfx` or `.prism` and open it again.";

/// Return `true` when `bytes` start with the legacy binary `.pzf` (`PCFF`) header.
#[must_use]
pub fn is_binary_pzf_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(BINARY_PZF_MAGIC)
}

/// Construct the canonical [`FormatError::UnsupportedFormat`] for legacy binary `.pzf` files.
#[must_use]
pub fn binary_pzf_unsupported_error() -> FormatError {
    FormatError::UnsupportedFormat { reason: String::from(BINARY_PZF_UNSUPPORTED_MESSAGE) }
}

/// Standard XML parse options for `.pzfx`: stop after root, skip opaque `<Template>` bodies, and
/// allow raw input up to 128 MiB while bounding the parsed DOM by default node/depth caps.
#[must_use]
fn pzfx_xml_parse_options() -> XmlParseOptions {
    XmlParseOptions::default()
        .with_limits(XmlLimits { max_bytes: MAX_XML_OPAQUE_BYTES, ..XmlLimits::default() })
        .with_stop_after_root(true)
        .with_opaque_elements(["Template"])
}

/// Import a `.pzfx` XML byte buffer into a [`ProjectDocument`] and compatibility warnings.
pub fn import_pzfx_bytes(bytes: &[u8]) -> Result<ImportReport, FormatError> {
    let mut used_cells = 0_usize;
    import_pzfx_bytes_with_used_cells(bytes, &mut used_cells)
}

/// Import a `.pzfx` XML string into a [`ProjectDocument`] and compatibility warnings.
pub fn import_pzfx_str(xml: &str) -> Result<ImportReport, FormatError> {
    let mut used_cells = 0_usize;
    import_pzfx_str_with_used_cells(xml, &mut used_cells)
}

/// Import `.pzfx` bytes while charging allocated cells against an accumulated project total.
pub(crate) fn import_pzfx_bytes_with_used_cells(bytes: &[u8], used_cells: &mut usize) -> Result<ImportReport, FormatError> {
    if is_binary_pzf_bytes(bytes) {
        return Err(binary_pzf_unsupported_error());
    }
    let parsed = parse_xml_bytes_with_options(bytes, &pzfx_xml_parse_options())?;
    import_parsed_pzfx(parsed, used_cells)
}

pub(crate) fn import_pzfx_str_with_used_cells(xml: &str, used_cells: &mut usize) -> Result<ImportReport, FormatError> {
    if is_binary_pzf_bytes(xml.as_bytes()) {
        return Err(binary_pzf_unsupported_error());
    }
    let parsed = parse_xml_with_options(xml, &pzfx_xml_parse_options())?;
    import_parsed_pzfx(parsed, used_cells)
}

fn is_pzfx_root_local_name(local: &str) -> bool {
    local == "PrismFile" || local == PZFX_ROOT_ELEMENT || local.ends_with("PrismFile")
}

fn unwrap_pzfx_root(root: &XmlElement) -> Result<&XmlElement, FormatError> {
    let local = root.local_name();
    if is_pzfx_root_local_name(local) {
        return Ok(root);
    }
    if local.eq_ignore_ascii_case("stylesheet") {
        if let Some(inner) = root.child_elements().find(|child| is_pzfx_root_local_name(child.local_name())) {
            return Ok(inner);
        }
        for child in root.child_elements() {
            if child.local_name().eq_ignore_ascii_case("template")
                && let Some(inner) = child.child_elements().find(|grand| is_pzfx_root_local_name(grand.local_name()))
            {
                return Ok(inner);
            }
        }
    }
    Err(FormatError::UnsupportedFormat { reason: format!("unexpected `.pzfx` root element `<{local}>`") })
}

fn import_parsed_pzfx(parsed: ParsedXml, used_cells: &mut usize) -> Result<ImportReport, FormatError> {
    let root = unwrap_pzfx_root(&parsed.root)?;
    let mut warnings = Vec::new();
    check_schema_version(root, &mut warnings);
    emit_opaque_and_trailing_warning(&parsed, &mut warnings);

    let title = extract_element_title(root).unwrap_or_else(|| String::from("Imported Project"));
    let clamped_title = clamp_title(&title, "project", &mut warnings);
    let mut doc = ProjectDocument::new(clamped_title)?;
    let mut used_ids = HashSet::new();
    let doc_decimal =
        DocDecimalContext { root_decimal_comma: detect_decimal_separator_attr(root), doc_evidence: collect_document_decimal_evidence(root) };

    let ordered_tables = collect_ordered_table_elements(root);
    let info_summary = import_info_sections(root, &ordered_tables.entries, &mut doc, &mut used_ids, &mut warnings, doc_decimal)?;
    let table_summary = import_table_sections(&ordered_tables, &info_summary, &mut doc, &mut used_ids, &mut warnings, used_cells, doc_decimal)?;
    attach_root_notes(root, &mut doc, &mut used_ids, &mut warnings)?;
    apply_selection_state(&mut doc, &info_summary, &table_summary);
    warn_ignored_sections(root, &mut warnings);

    validate_project_document(&doc)?;
    validate_archive_save_limits(&doc)?;
    Ok(ImportReport::new(doc, warnings))
}

fn emit_opaque_and_trailing_warning(parsed: &ParsedXml, warnings: &mut Vec<ImportWarning>) {
    let opaque_count = parsed.skipped_opaque.len();
    let opaque_bytes = parsed.total_opaque_bytes();
    let trailing = parsed.trailing_bytes;
    if opaque_count == 0 && trailing == 0 {
        return;
    }
    let (source, message) = match (opaque_count > 0, trailing > 0) {
        (true, true) => {
            let first_name = parsed.skipped_opaque.first().map_or("Template", |entry| entry.local_name());
            (
                first_name,
                format!(
                    "skipped {opaque_count} opaque `<{first_name}>` section(s) ({opaque_bytes} byte(s)) and ignored {trailing} trailing byte(s) after root element"
                ),
            )
        }
        (true, false) => {
            let first_name = parsed.skipped_opaque.first().map_or("Template", |entry| entry.local_name());
            (first_name, format!("skipped {opaque_count} opaque `<{first_name}>` section(s) ({opaque_bytes} byte(s)) during `.pzfx` import"))
        }
        (false, true) => ("trailing", format!("ignored {trailing} trailing byte(s) after `.pzfx` root element closed")),
        (false, false) => return,
    };
    warnings.push(ImportWarning::new(ImportWarningKind::IgnoredSection, source, message));
}

fn check_schema_version(root: &XmlElement, warnings: &mut Vec<ImportWarning>) {
    let Some((attr_name, raw)) = root
        .attr("PrismXMLVersion")
        .map(|v| ("PrismXMLVersion", v))
        .or_else(|| root.attr("SchemaVersion").map(|v| ("SchemaVersion", v)))
        .or_else(|| root.attr("Version").map(|v| ("Version", v)))
    else {
        return;
    };
    let trimmed = raw.trim();
    let major = trimmed.split('.').next().and_then(|head| head.parse::<u32>().ok());
    match major {
        Some(1..=11) => {}
        _ => warnings.push(ImportWarning::new(
            ImportWarningKind::MissingMetadata,
            attr_name,
            format!("unrecognized `.pzfx` {attr_name} `{trimmed}`; imported using compatibility fallback"),
        )),
    }
}

fn is_truthy_attr(raw: Option<&str>) -> bool {
    raw.is_some_and(|v| {
        let t = v.trim();
        t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
    })
}

fn collect_element_notes_text(el: &XmlElement) -> Option<String> {
    let mut parts = Vec::new();
    for tag in ["Notes", "Note", "Description"] {
        for child in el.children_named(tag) {
            let text = pzfx_element_to_markup(child);
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_owned());
            }
        }
    }
    if parts.is_empty() { None } else { Some(parts.join("\n\n")) }
}

fn extract_floating_notes(el: &XmlElement, used_note_ids: &mut HashSet<String>) -> Vec<FloatingNote> {
    let mut notes = Vec::new();
    for (idx, note_el) in el.children_named("FloatingNote").enumerate() {
        let body = pzfx_element_to_markup(note_el);
        let trimmed = body.trim();
        if trimmed.is_empty() {
            continue;
        }
        let raw_id = note_el.attr("ID").map(str::trim).filter(|s| !s.is_empty());
        let base_id = raw_id.map_or_else(|| format!("note-{}", idx + 1), str::to_owned);
        let note_id = reserve_unique_note_id(&base_id, used_note_ids);
        let bounds = parse_floating_note_bounds(note_el);
        let mut note = FloatingNote::new(note_id, bounds, trimmed.to_owned());
        if let Some(fill) = extract_pzfx_color_tag(note_el) {
            note.fill_color = fill;
        }
        notes.push(note);
    }
    notes
}

fn reserve_unique_note_id(base: &str, used: &mut HashSet<String>) -> String {
    let bounded = truncate_utf8_to_bytes(base, MAX_SHEET_ID_BYTES);
    let clean = if bounded.trim().is_empty() { "note" } else { bounded };
    if used.insert(clean.to_owned()) {
        return clean.to_owned();
    }
    let mut suffix = 1_usize;
    loop {
        let tag = format!("-{suffix}");
        let budget = MAX_SHEET_ID_BYTES.saturating_sub(tag.len());
        let candidate = format!("{}{tag}", truncate_utf8_to_bytes(clean, budget));
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn parse_floating_note_bounds(note_el: &XmlElement) -> RectPt {
    let parse_f64 = |key: &str, default: f64| -> f64 {
        attr_ci(note_el, key).and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(default)
    };
    let x = parse_f64("left", 16.0);
    let y = parse_f64("top", 16.0);
    let width = parse_f64("width", 220.0).max(0.0);
    let height = parse_f64("height", 64.0).max(0.0);
    RectPt::new(x, y, width, height)
}

fn attach_root_notes(
    root: &XmlElement,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    let Some(root_notes) = collect_element_notes_text(root) else {
        return Ok(());
    };
    if let Some(info) = doc.info_sheets.first_mut() {
        if info.notes_markdown.trim().is_empty() {
            info.notes_markdown = root_notes;
        } else {
            info.notes_markdown.push_str("\n\n");
            info.notes_markdown.push_str(&root_notes);
        }
    } else if let Some(ds) = doc.data_sheets.first_mut() {
        let mut used_notes: HashSet<String> = ds.floating_notes.iter().map(|n| n.id.as_str().to_owned()).collect();
        let note_id = reserve_unique_note_id("note-project", &mut used_notes);
        ds.floating_notes.push(FloatingNote::new(note_id, RectPt::new(16.0, 16.0, 240.0, 72.0), root_notes));
    } else {
        let id = allocate_valid_unique_sheet_id(Some("info-notes"), "info", 1, used_ids, warnings);
        let mut sheet = InfoSheet::new_preseeded(id.as_str().to_owned(), "Project Notes")?;
        sheet.constants.clear();
        sheet.notes_markdown = root_notes;
        doc.add_info_sheet(sheet)?;
    }
    Ok(())
}

struct SequenceRefSpec<'a> {
    id: &'a str,
    selected: bool,
}

fn collect_sequence_refs<'a>(root: &'a XmlElement, seq_tag: &'static str) -> (bool, Vec<SequenceRefSpec<'a>>) {
    let mut seq_selected = false;
    let mut refs = Vec::new();
    for seq in root.children_named(seq_tag) {
        if is_truthy_attr(seq.attr("Selected")) {
            seq_selected = true;
        }
        for ref_el in seq.children_named("Ref") {
            if let Some(id) = ref_el.attr("ID").map(str::trim).filter(|s| !s.is_empty()) {
                refs.push(SequenceRefSpec { id, selected: is_truthy_attr(ref_el.attr("Selected")) });
            }
        }
    }
    (seq_selected, refs)
}

struct OrderedTableEntry<'a> {
    el: &'a XmlElement,
    is_huge: bool,
    selected_from_ref: bool,
}

struct OrderedTables<'a> {
    sequence_selected: bool,
    entries: Vec<OrderedTableEntry<'a>>,
}

fn is_table_element_local_name(local: &str) -> Option<bool> {
    match local {
        "Table" => Some(false),
        "HugeTable" | "Table1024" => Some(true),
        _ => None,
    }
}

fn collect_ordered_table_elements(root: &XmlElement) -> OrderedTables<'_> {
    let (sequence_selected, refs) = collect_sequence_refs(root, "TableSequence");
    let mut raw_tables: Vec<(&XmlElement, bool)> = Vec::new();
    for seq in root.children_named("TableSequence") {
        for child in seq.child_elements() {
            if let Some(is_huge) = is_table_element_local_name(child.local_name()) {
                raw_tables.push((child, is_huge));
            }
        }
    }
    for seq in root.children_named("HugeTableSequence") {
        for child in seq.child_elements() {
            if is_table_element_local_name(child.local_name()).is_some() {
                raw_tables.push((child, true));
            }
        }
    }
    for child in root.child_elements() {
        if let Some(is_huge) = is_table_element_local_name(child.local_name()) {
            raw_tables.push((child, is_huge));
        }
    }

    let mut consumed = vec![false; raw_tables.len()];
    let mut entries = Vec::with_capacity(raw_tables.len());
    for seq_ref in &refs {
        if let Some((idx, &(el, is_huge))) = raw_tables
            .iter()
            .enumerate()
            .find(|(idx, (el, _))| !consumed.get(*idx).copied().unwrap_or(true) && el.attr("ID").map(str::trim) == Some(seq_ref.id))
        {
            if let Some(slot) = consumed.get_mut(idx) {
                *slot = true;
            }
            entries.push(OrderedTableEntry { el, is_huge, selected_from_ref: seq_ref.selected });
        }
    }
    for (idx, (el, is_huge)) in raw_tables.into_iter().enumerate() {
        if !consumed.get(idx).copied().unwrap_or(true) {
            entries.push(OrderedTableEntry { el, is_huge, selected_from_ref: is_truthy_attr(el.attr("Selected")) });
        }
    }
    OrderedTables { sequence_selected, entries }
}

struct CandidateInfo<'a> {
    el: &'a XmlElement,
    owner_table_idx: Option<usize>,
}

struct InfoImportSummary {
    sequence_selected: bool,
    selected_info_id: Option<SheetId>,
    primary_top_level_info: Option<SheetId>,
    by_raw_id: HashMap<String, SheetId>,
    by_owner_table_idx: HashMap<usize, SheetId>,
}

fn import_info_sections(
    root: &XmlElement,
    ordered_tables: &[OrderedTableEntry<'_>],
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    doc_decimal: DocDecimalContext,
) -> Result<InfoImportSummary, FormatError> {
    let (sequence_selected, refs) = collect_sequence_refs(root, "InfoSequence");
    let mut candidates: Vec<CandidateInfo<'_>> = Vec::new();
    for seq in root.children_named("InfoSequence") {
        for info_el in seq.children_named("Info") {
            candidates.push(CandidateInfo { el: info_el, owner_table_idx: None });
        }
    }
    for info_el in root.children_named("Info") {
        candidates.push(CandidateInfo { el: info_el, owner_table_idx: None });
    }
    for (table_idx, entry) in ordered_tables.iter().enumerate() {
        for info_el in entry.el.children_named("Info") {
            candidates.push(CandidateInfo { el: info_el, owner_table_idx: Some(table_idx) });
        }
    }

    let mut consumed = vec![false; candidates.len()];
    let mut ordered: Vec<(usize, bool)> = Vec::with_capacity(candidates.len());
    for seq_ref in &refs {
        if let Some((idx, _)) = candidates
            .iter()
            .enumerate()
            .find(|(idx, cand)| !consumed.get(*idx).copied().unwrap_or(true) && cand.el.attr("ID").map(str::trim) == Some(seq_ref.id))
        {
            if let Some(slot) = consumed.get_mut(idx) {
                *slot = true;
            }
            ordered.push((idx, seq_ref.selected));
        }
    }
    for (idx, cand) in candidates.iter().enumerate() {
        if !consumed.get(idx).copied().unwrap_or(true) {
            ordered.push((idx, is_truthy_attr(cand.el.attr("Selected"))));
        }
    }

    let mut selected_info_id = None;
    let mut primary_top_level_info = None;
    let mut by_raw_id = HashMap::new();
    let mut by_owner_table_idx = HashMap::new();

    for (out_idx, &(cand_idx, is_selected)) in ordered.iter().enumerate() {
        let Some(cand) = candidates.get(cand_idx) else {
            continue;
        };
        let sheet = parse_info_element(cand.el, out_idx + 1, used_ids, warnings, doc_decimal)?;
        let sheet_id = sheet.id.clone();
        if let Some(raw_id) = cand.el.attr("ID").map(str::trim).filter(|s| !s.is_empty()) {
            by_raw_id.entry(raw_id.to_owned()).or_insert_with(|| sheet_id.clone());
        }
        if let Some(table_idx) = cand.owner_table_idx {
            by_owner_table_idx.entry(table_idx).or_insert_with(|| sheet_id.clone());
        } else if primary_top_level_info.is_none() {
            primary_top_level_info = Some(sheet_id.clone());
        }
        if is_selected && selected_info_id.is_none() {
            selected_info_id = Some(sheet_id.clone());
        }
        doc.add_info_sheet(sheet)?;
    }

    Ok(InfoImportSummary { sequence_selected, selected_info_id, primary_top_level_info, by_raw_id, by_owner_table_idx })
}

fn parse_info_element(
    info_el: &XmlElement,
    one_based_idx: usize,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    doc_decimal: DocDecimalContext,
) -> Result<InfoSheet, FormatError> {
    let info_evidence = collect_info_decimal_evidence(info_el);
    let id = allocate_valid_unique_sheet_id(info_el.attr("ID"), "info", one_based_idx, used_ids, warnings);
    let raw_title = extract_element_title(info_el).unwrap_or_else(|| format!("Info {one_based_idx}"));
    let title = clamp_title(&raw_title, id.as_str(), warnings);

    let mut sheet = InfoSheet::new_preseeded(id.as_str().to_owned(), title)?;
    sheet.constants.clear();
    sheet.color_tag = extract_pzfx_table_color_tag(info_el);
    let decimal_comma = detect_decimal_separator_attr(info_el).or(doc_decimal.root_decimal_comma);
    for c_el in info_el.children_named("Constant") {
        let name_raw = c_el
            .first_child("Name")
            .map(|el| pzfx_element_to_plain_text(el).trim().to_owned())
            .or_else(|| c_el.attr("Name").map(|s| s.trim().to_owned()))
            .unwrap_or_default();
        if name_raw.is_empty() {
            continue;
        }
        let name = clamp_title(&name_raw, "constant", warnings);
        let (val_plain, val_markup) = match c_el.first_child("Value") {
            Some(el) => (pzfx_element_to_plain_text(el), pzfx_element_to_markup(el)),
            None => {
                let attr_val = c_el.attr("Value").map(str::to_owned).unwrap_or_default();
                (attr_val.clone(), attr_val)
            }
        };
        let val_trimmed = val_plain.trim();
        let value = if val_trimmed.is_empty() {
            ConstantValue::Empty
        } else if let Some(date_val) = parse_pzfx_constant_date(val_trimmed) {
            date_val
        } else if let Some(num) = parse_pzfx_scalar_f64(val_trimmed, info_evidence, doc_decimal.doc_evidence, decimal_comma) {
            ConstantValue::Number(num)
        } else {
            ConstantValue::Text(val_markup.trim().to_owned())
        };
        let units = c_el
            .first_child("Units")
            .map(|el| clamp_title(pzfx_element_to_plain_text(el).trim(), "units", warnings))
            .or_else(|| c_el.attr("Units").map(|s| clamp_title(s.trim(), "units", warnings)))
            .filter(|u| !u.is_empty());
        sheet.constants.push(InfoConstant::new(name, value, units)?);
    }
    if let Some(notes_text) = collect_element_notes_text(info_el) {
        sheet.notes_markdown = notes_text;
    }
    let mut used_note_ids = HashSet::new();
    sheet.floating_notes = extract_floating_notes(info_el, &mut used_note_ids);
    Ok(sheet)
}

struct TableImportSummary {
    sequence_selected: bool,
    selected_table_id: Option<SheetId>,
}

fn import_table_sections(
    ordered: &OrderedTables<'_>,
    info_summary: &InfoImportSummary,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    total_cells: &mut usize,
    doc_decimal: DocDecimalContext,
) -> Result<TableImportSummary, FormatError> {
    let mut selected_table_id = None;

    for (idx, entry) in ordered.entries.iter().enumerate() {
        let mut ds = parse_table_element(entry.el, idx + 1, entry.is_huge, used_ids, warnings, *total_cells, doc_decimal)?;
        let actual_cells = ds.archetype.allocated_cell_slots();
        let next_total = total_cells.checked_add(actual_cells).ok_or_else(|| FormatError::XmlParse {
            offset: 0,
            reason: format!("table `{}` cell allocation overflows budget cap ({MAX_IMPORT_CELLS})", ds.id),
        })?;
        if next_total > MAX_IMPORT_CELLS {
            let remaining = MAX_IMPORT_CELLS.saturating_sub(*total_cells);
            return Err(FormatError::XmlParse {
                offset: 0,
                reason: format!(
                    "table `{}` cell allocation ({actual_cells}) exceeds remaining import budget ({remaining}, cap {MAX_IMPORT_CELLS})",
                    ds.id
                ),
            });
        }
        *total_cells = next_total;

        let explicit_info_id = info_summary
            .by_owner_table_idx
            .get(&idx)
            .cloned()
            .or_else(|| entry.el.attr("InfoID").map(str::trim).and_then(|raw| info_summary.by_raw_id.get(raw)).cloned());
        if let Some(linked_info_id) = explicit_info_id {
            ds.info_sheet_id = Some(linked_info_id.clone());
            if let Some(info_sheet) = doc.info_sheet_mut(&linked_info_id)
                && !info_sheet.linked_data_sheets.contains(&ds.id)
            {
                info_sheet.linked_data_sheets.push(ds.id.clone());
            }
        } else if ds.info_sheet_id.is_none() {
            ds.info_sheet_id = info_summary.primary_top_level_info.clone().or_else(|| doc.info_sheets.first().map(|s| s.id.clone()));
        }

        if entry.selected_from_ref && selected_table_id.is_none() {
            selected_table_id = Some(ds.id.clone());
        }
        doc.add_data_sheet(ds)?;
    }

    Ok(TableImportSummary { sequence_selected: ordered.sequence_selected, selected_table_id })
}

fn apply_selection_state(doc: &mut ProjectDocument, info: &InfoImportSummary, tables: &TableImportSummary) {
    let chosen = if info.sequence_selected && info.selected_info_id.is_some() {
        info.selected_info_id.clone()
    } else if tables.selected_table_id.is_some() {
        tables.selected_table_id.clone()
    } else if info.selected_info_id.is_some() {
        info.selected_info_id.clone()
    } else {
        doc.active_sheet_id.clone().or_else(|| doc.info_sheets.first().map(|s| s.id.clone()))
    };
    let _ = tables.sequence_selected;
    if let Some(id) = chosen
        && doc.sheet_section(&id).is_some()
    {
        doc.active_sheet_id = Some(id.clone());
        doc.family_anchor_sheet_id = Some(id);
    }
}

struct ParsedSubColumnTitles {
    own_set: bool,
    flat: Vec<String>,
}

fn parse_subcolumn_titles_element(table_el: &XmlElement, table_id: &str, warnings: &mut Vec<ImportWarning>) -> Option<ParsedSubColumnTitles> {
    let sct_el = table_el.first_child("SubColumnTitles")?;
    let own_set = is_truthy_attr(sct_el.attr("OwnSet"));
    let mut flat = Vec::new();
    for sub_el in sct_el.children_named("Subcolumn") {
        let mut saw_d = false;
        for d_el in sub_el.children_named("d") {
            saw_d = true;
            let raw = pzfx_element_to_markup(d_el);
            flat.push(clamp_title(raw.trim(), table_id, warnings));
        }
        if !saw_d {
            flat.push(String::new());
        }
    }
    Some(ParsedSubColumnTitles { own_set, flat })
}

fn parse_mv_type_attr(y_el: &XmlElement) -> Option<VariableRole> {
    let raw = y_el.attr("MVType")?.trim();
    if raw.eq_ignore_ascii_case("categorical") {
        Some(VariableRole::Categorical)
    } else if raw.eq_ignore_ascii_case("continues") || raw.eq_ignore_ascii_case("continuous") {
        Some(VariableRole::Continuous)
    } else if raw.eq_ignore_ascii_case("label") {
        Some(VariableRole::Label)
    } else {
        None
    }
}

fn parse_display_decimals_u8(col_el: &XmlElement) -> Option<u8> {
    let raw = parse_decimals_attr(col_el)?;
    let val = u8::try_from(raw).ok()?;
    (val <= MAX_DISPLAY_DECIMALS).then_some(val)
}

fn parse_table_element(
    table_el: &XmlElement,
    one_based_idx: usize,
    is_huge: bool,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    used_cells: usize,
    doc_decimal: DocDecimalContext,
) -> Result<DataSheet, FormatError> {
    let id = allocate_valid_unique_sheet_id(table_el.attr("ID"), "data", one_based_idx, used_ids, warnings);
    let raw_title = extract_element_title(table_el).unwrap_or_else(|| format!("Data {one_based_idx}"));
    let title = clamp_title(&raw_title, id.as_str(), warnings);

    let kind = resolve_table_kind(table_el.attr("TableType"), table_el.attr("ExtTableType"), is_huge, id.as_str(), warnings);
    let explicit_decimal_comma = detect_decimal_separator_attr(table_el).or(doc_decimal.root_decimal_comma);
    let table_evidence = collect_table_decimal_evidence(table_el, kind);
    let num_ctx = NumericParseScope { table_ev: table_evidence, doc_ev: doc_decimal.doc_evidence, explicit_decimal_comma };
    let reps = parse_replicates_attr(table_el.attr("Replicates"));
    if let Some(raw_rep) = reps.malformed {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            id.as_str(),
            format!("invalid Replicates=\"{raw_rep}\" in table `{title}`, defaulting to 1"),
        ));
    }
    if reps.count > 0 {
        validate_subcolumns(reps.count)?;
    }

    let ev_role = classify_ev_format(table_el.attr("EVFormat"));
    let y_format = table_el
        .attr("YFormat")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (ev_role == EvFormatRole::SubcolumnModeFallback).then(|| table_el.attr("EVFormat")).flatten());
    let cell_opts = CellParseOptions { ev_role, column_excluded: false };

    let row_titles_el = table_el.first_child("RowTitlesColumn");
    let row_factor_title = row_titles_el.and_then(extract_element_title).map(|raw| clamp_title(&raw, id.as_str(), warnings));
    let row_titles_declared_subs = row_titles_el.map(|el| parse_subcolumns_attr(el, id.as_str(), "RowTitlesColumn", warnings)).transpose()?.flatten();
    let row_titles_cols = row_titles_el.map(extract_raw_subcolumn_strings).unwrap_or_default();
    warn_discarded_subcolumns(
        warnings,
        (id.as_str(), &title),
        "RowTitlesColumn",
        row_titles_cols.len(),
        row_titles_declared_subs,
        1,
        "row-title column",
    );
    let row_titles_raw = row_titles_cols.into_iter().next().unwrap_or_default();
    let row_titles: Vec<String> = row_titles_raw.iter().map(|raw| clamp_title(raw, &format!("table `{title}` row title"), warnings)).collect();

    let parsed_x = parse_table_x_column(table_el, kind, id.as_str(), cell_opts, num_ctx, warnings)?;
    let mut parsed_y = parse_table_y_columns(table_el, id.as_str(), cell_opts, num_ctx, warnings)?;
    let subcol_titles_spec = parse_subcolumn_titles_element(table_el, id.as_str(), warnings);

    let y_resolution = resolve_subcolumn_mode(kind, y_format, reps, &parsed_y, id.as_str(), warnings)?;
    let subcolumn_mode = widen_y_subcolumn_mode(kind, y_resolution.mode, &parsed_y)?;
    let y_err_meta = ErrorSubcolumnMeta { mode: subcolumn_mode, is_plus_minus: y_resolution.is_plus_minus, is_no_n: y_resolution.is_no_n };
    for col in &mut parsed_y {
        normalize_error_subcolumns(&mut col.subcolumns, &col.sub_titles, y_err_meta, (id.as_str(), &col.title), warnings);
    }

    let max_y_rows = parsed_y.iter().flat_map(|col| col.subcolumns.iter().map(Vec::len)).max().unwrap_or(0);
    let max_x_rows = parsed_x.cols.iter().map(Vec::len).max().unwrap_or(0);
    let row_count = row_titles.len().max(max_x_rows).max(max_y_rows);
    validate_rows(row_count)?;

    let mut padded_row_titles = row_titles;
    padded_row_titles.resize(row_count, String::new());
    let has_row_titles = padded_row_titles.iter().any(|s| !s.is_empty());

    warn_discarded_x_subcolumns(
        warnings,
        (id.as_str(), &title),
        kind,
        &parsed_x.mode,
        &parsed_x.title,
        &parsed_x.cols,
        parsed_x.declared_subs,
        has_row_titles,
    );
    if kind != TableKind::Nested {
        let allowed_y_subs = allowed_y_subcolumns_for_kind(kind, subcolumn_mode);
        let mode_desc = format!("{kind:?} mode {subcolumn_mode:?}");
        for col in &parsed_y {
            warn_discarded_subcolumns(
                warnings,
                (id.as_str(), &title),
                &col.title,
                col.subcolumns.len(),
                col.declared_subcolumns,
                allowed_y_subs,
                &mode_desc,
            );
        }
    }
    emit_table_numeric_cell_warnings(
        warnings,
        (id.as_str(), &title),
        &TableNumericStatsView { kind, x_mode: &parsed_x.mode, subcolumn_mode, x_sub_stats: &parsed_x.sub_stats, parsed_y: &parsed_y },
    );

    let remaining_cells = MAX_IMPORT_CELLS.saturating_sub(used_cells);
    let table_cells = estimate_table_cells(kind, subcolumn_mode, &parsed_x.mode, &parsed_y, row_count)?;
    if table_cells > remaining_cells {
        return Err(FormatError::XmlParse {
            offset: 0,
            reason: format!(
                "table `{id}` cell allocation ({table_cells}) exceeds remaining import budget ({remaining_cells}, cap {MAX_IMPORT_CELLS})"
            ),
        });
    }

    let (effective_row_titles, effective_has_row_titles) =
        resolve_effective_row_titles(kind, parsed_x.is_text_x, parsed_x.source_el, padded_row_titles, has_row_titles, row_count, &title, warnings);
    let mut arch = build_table_archetype(
        kind,
        subcolumn_mode,
        &parsed_x,
        &parsed_y,
        effective_row_titles,
        effective_has_row_titles,
        row_factor_title.as_deref(),
        row_count,
    )?;
    apply_table_subcolumn_titles(&mut arch, &parsed_y, subcol_titles_spec.as_ref());
    warn_domain_and_non_numeric_anomalies(id.as_str(), kind, &parsed_x.cols, &parsed_y, warnings);

    let mut sheet = DataSheet::new(id, title, arch)?;
    sheet.color_tag = extract_pzfx_table_color_tag(table_el);
    let mut used_note_ids = HashSet::new();
    sheet.floating_notes = extract_floating_notes(table_el, &mut used_note_ids);
    if let Some(table_notes) = collect_element_notes_text(table_el) {
        let note_id = reserve_unique_note_id("note-1", &mut used_note_ids);
        sheet.floating_notes.push(FloatingNote::new(note_id, RectPt::new(16.0, 16.0, 220.0, 64.0), table_notes));
    }
    Ok(sheet)
}

struct ParsedXColumn<'a> {
    source_el: Option<&'a XmlElement>,
    title: String,
    mode: XColumnMode,
    display_decimals: Option<u8>,
    declared_subs: Option<usize>,
    sub_titles: Vec<Option<String>>,
    cols: Vec<Vec<Cell>>,
    sub_stats: Vec<SubcolumnNumericStats>,
    is_text_x: bool,
}

fn parse_table_x_column<'a>(
    table_el: &'a XmlElement,
    kind: TableKind,
    table_id: &str,
    cell_opts: CellParseOptions,
    num_ctx: NumericParseScope,
    warnings: &mut Vec<ImportWarning>,
) -> Result<ParsedXColumn<'a>, FormatError> {
    let x_el = table_el.first_child("XColumn");
    let x_adv_el = table_el.first_child("XAdvancedColumn");
    let primary_el = x_el.or(x_adv_el);

    let mut chosen_el = primary_el;
    let mut x_sub_titles = Vec::new();
    let mut x_sub_stats = Vec::new();
    let mut x_cols = if let Some(el) = primary_el {
        let x_opts = CellParseOptions { column_excluded: element_is_excluded(el), ..cell_opts };
        let (subs_t, cols, stats) = extract_labeled_subcolumns(el, table_id, warnings, x_opts, num_ctx);
        x_sub_titles = subs_t;
        x_sub_stats = stats;
        cols
    } else {
        Vec::new()
    };

    if x_cols.iter().all(Vec::is_empty)
        && let Some(adv_el) = x_adv_el
        && x_el.is_some()
    {
        let adv_opts = CellParseOptions { column_excluded: element_is_excluded(adv_el), ..cell_opts };
        let (subs_t, cols, stats) = extract_labeled_subcolumns(adv_el, table_id, warnings, adv_opts, num_ctx);
        if cols.iter().any(|c| !c.is_empty()) {
            chosen_el = Some(adv_el);
            x_sub_titles = subs_t;
            x_sub_stats = stats;
            x_cols = cols;
        }
    }

    let x_declared_subs = chosen_el.map(|el| parse_subcolumns_attr(el, table_id, "XColumn", warnings)).transpose()?.flatten().or_else(|| {
        if chosen_el.is_some() {
            None
        } else {
            x_adv_el.and_then(|el| parse_subcolumns_attr(el, table_id, "XAdvancedColumn", warnings).ok().flatten())
        }
    });
    let explicit_x_title =
        x_el.and_then(extract_element_title).or_else(|| x_adv_el.and_then(extract_element_title)).map(|raw| clamp_title(&raw, table_id, warnings));
    let display_decimals = x_el.and_then(parse_display_decimals_u8).or_else(|| x_adv_el.and_then(parse_display_decimals_u8));

    let x_resolution =
        resolve_x_mode(kind, table_el, x_el.or(x_adv_el), explicit_x_title.as_deref(), &x_cols, x_declared_subs, num_ctx, warnings, table_id)?;
    let x_mode = x_resolution.mode;
    let x_title = explicit_x_title.unwrap_or_else(|| match &x_mode {
        XColumnMode::DateOrElapsedTime { unit_label } => unit_label.clone(),
        _ => String::from("X"),
    });
    let x_err_meta = ErrorSubcolumnMeta {
        mode: match x_mode {
            XColumnMode::Numbers { subcolumn_mode } => subcolumn_mode,
            _ => SubcolumnMode::Single,
        },
        is_plus_minus: x_resolution.is_plus_minus,
        is_no_n: x_resolution.is_no_n,
    };
    normalize_error_subcolumns(&mut x_cols, &x_sub_titles, x_err_meta, (table_id, &x_title), warnings);

    Ok(ParsedXColumn {
        source_el: chosen_el,
        title: x_title,
        mode: x_mode,
        display_decimals,
        declared_subs: x_declared_subs,
        sub_titles: x_sub_titles,
        cols: x_cols,
        sub_stats: x_sub_stats,
        is_text_x: x_resolution.is_text_x,
    })
}

fn parse_table_y_columns(
    table_el: &XmlElement,
    table_id: &str,
    cell_opts: CellParseOptions,
    num_ctx: NumericParseScope,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Vec<ParsedYColumn>, FormatError> {
    let y_elements: Vec<&XmlElement> =
        table_el.children_named("YColumn").chain(table_el.children_named("Column")).chain(table_el.children_named("VariableColumn")).collect();
    validate_data_sets(y_elements.len())?;

    let mut parsed_y: Vec<ParsedYColumn> = Vec::with_capacity(y_elements.len());
    for (c_idx, y_el) in y_elements.iter().enumerate() {
        let col_title = extract_element_title(y_el).map(|raw| clamp_title(&raw, table_id, warnings)).unwrap_or_else(|| default_column_label(c_idx));
        let declared_subcolumns = parse_subcolumns_attr(y_el, table_id, &col_title, warnings)?;
        let display_decimals = parse_display_decimals_u8(y_el);
        let mv_type = parse_mv_type_attr(y_el);
        let y_opts = CellParseOptions { column_excluded: element_is_excluded(y_el), ..cell_opts };
        let (sub_titles, subcolumns, sub_stats) = extract_labeled_subcolumns(y_el, table_id, warnings, y_opts, num_ctx);
        parsed_y.push(ParsedYColumn { title: col_title, declared_subcolumns, display_decimals, mv_type, sub_titles, subcolumns, sub_stats });
    }
    Ok(parsed_y)
}

#[allow(clippy::too_many_arguments)]
fn resolve_effective_row_titles(
    kind: TableKind,
    is_text_x: bool,
    x_el: Option<&XmlElement>,
    padded_row_titles: Vec<String>,
    has_row_titles: bool,
    row_count: usize,
    title: &str,
    warnings: &mut Vec<ImportWarning>,
) -> (Vec<String>, bool) {
    if kind == TableKind::PartsOfWhole
        && !has_row_titles
        && let Some(first_x_raw) = x_el.and_then(|el| extract_raw_subcolumn_strings(el).into_iter().next())
    {
        let mut labels: Vec<String> = first_x_raw.iter().map(|raw| clamp_title(raw, &format!("table `{title}` slice label"), warnings)).collect();
        labels.resize(row_count, String::new());
        (labels, true)
    } else if kind == TableKind::Xy
        && is_text_x
        && !has_row_titles
        && let Some(first_x_raw) = x_el.and_then(|el| extract_raw_subcolumn_strings(el).into_iter().next())
    {
        let mut labels: Vec<String> = first_x_raw.iter().map(|raw| clamp_title(raw, &format!("table `{title}` row title"), warnings)).collect();
        labels.resize(row_count, String::new());
        let any_non_empty = labels.iter().any(|s| !s.is_empty());
        (labels, any_non_empty)
    } else {
        (padded_row_titles, has_row_titles)
    }
}

struct ParsedYColumn {
    title: String,
    declared_subcolumns: Option<usize>,
    display_decimals: Option<u8>,
    mv_type: Option<VariableRole>,
    sub_titles: Vec<Option<String>>,
    subcolumns: Vec<Vec<Cell>>,
    sub_stats: Vec<SubcolumnNumericStats>,
}

fn parse_subcolumns_attr(el: &XmlElement, table_id: &str, col_label: &str, warnings: &mut Vec<ImportWarning>) -> Result<Option<usize>, FormatError> {
    let Some(raw) = el.attr("Subcolumns").map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let Ok(count) = raw.parse::<usize>() else {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            table_id,
            format!("invalid Subcolumns=\"{raw}\" on `{col_label}` in table `{table_id}`; ignoring attribute"),
        ));
        return Ok(None);
    };
    if count == 0 {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            table_id,
            format!("Subcolumns=\"0\" on `{col_label}` in table `{table_id}` is invalid; ignoring attribute"),
        ));
        return Ok(None);
    }
    validate_subcolumns(count)?;
    Ok(Some(count))
}

fn widen_y_subcolumn_mode(kind: TableKind, resolved: SubcolumnMode, parsed_y: &[ParsedYColumn]) -> Result<SubcolumnMode, FormatError> {
    let max_requested = parsed_y.iter().map(|col| col.subcolumns.len().max(col.declared_subcolumns.unwrap_or(0))).max().unwrap_or(0);
    if matches!(kind, TableKind::Xy | TableKind::Grouped) && !resolved.is_pre_summarized() {
        let widened = resolved.subcolumn_count().max(max_requested).max(usize::from(kind == TableKind::Grouped));
        if widened > 1 || matches!(resolved, SubcolumnMode::RawReplicates(_)) {
            validate_subcolumns(widened)?;
            return Ok(SubcolumnMode::RawReplicates(widened));
        }
    }
    Ok(resolved)
}

fn warn_discarded_subcolumns(
    warnings: &mut Vec<ImportWarning>,
    table: (&str, &str),
    col_label: &str,
    actual_subs: usize,
    declared_subs: Option<usize>,
    allowed_subs: usize,
    mode_desc: &str,
) {
    let (table_id, table_title) = table;
    let requested = actual_subs.max(declared_subs.unwrap_or(0));
    if requested <= allowed_subs {
        return;
    }
    let discarded = actual_subs.saturating_sub(allowed_subs);
    let declared_note = declared_subs.map_or_else(String::new, |k| format!(", Subcolumns=\"{k}\""));
    let action = if discarded > 0 {
        format!("discarded {discarded} subcolumn(s) (kept {allowed_subs} of {actual_subs})")
    } else {
        let excess = requested.saturating_sub(allowed_subs);
        format!("ignored {excess} declared subcolumn slot(s) beyond capacity ({allowed_subs})")
    };
    warnings.push(ImportWarning::new(
        ImportWarningKind::CoercedValue,
        table_id,
        format!(
            "table `{table_id}` (`{table_title}`) column `{col_label}` has {actual_subs} subcolumn(s){declared_note} exceeding {mode_desc} capacity ({allowed_subs}); {action}"
        ),
    ));
}

#[allow(clippy::too_many_arguments)]
fn warn_discarded_x_subcolumns(
    warnings: &mut Vec<ImportWarning>,
    table: (&str, &str),
    kind: TableKind,
    x_mode: &XColumnMode,
    x_title: &str,
    x_cols: &[Vec<Cell>],
    x_declared_subs: Option<usize>,
    has_row_titles: bool,
) {
    let col_label = format!("XColumn ({x_title})");
    match kind {
        TableKind::Xy => {
            warn_discarded_subcolumns(
                warnings,
                table,
                &col_label,
                x_cols.len(),
                x_declared_subs,
                x_mode.subcolumn_count(),
                &format!("XY X-column mode {x_mode:?}"),
            );
        }
        TableKind::Survival => {
            warn_discarded_subcolumns(warnings, table, &col_label, x_cols.len(), x_declared_subs, 1, "Survival time column");
        }
        TableKind::PartsOfWhole => {
            if !has_row_titles {
                warn_discarded_subcolumns(warnings, table, &col_label, x_cols.len(), x_declared_subs, 1, "PartsOfWhole slice-label column");
            } else if x_cols.iter().any(|s| !s.is_empty()) || x_declared_subs.is_some() {
                warn_discarded_subcolumns(warnings, table, &col_label, x_cols.len(), x_declared_subs, 0, "PartsOfWhole table with RowTitlesColumn");
            }
        }
        TableKind::Column | TableKind::Grouped | TableKind::Contingency | TableKind::MultipleVariables | TableKind::Nested => {
            if x_cols.iter().any(|s| !s.is_empty()) || x_declared_subs.is_some() {
                warn_discarded_subcolumns(warnings, table, &col_label, x_cols.len(), x_declared_subs, 0, &format!("{kind:?} table (no X column)"));
            }
        }
    }
}

fn estimate_table_cells(
    kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    x_mode: &XColumnMode,
    parsed_y: &[ParsedYColumn],
    row_count: usize,
) -> Result<usize, FormatError> {
    let overflow = || FormatError::XmlParse { offset: 0, reason: format!("table cell count overflows budget cap of {MAX_IMPORT_CELLS}") };
    match kind {
        TableKind::Xy => {
            validate_subcolumns(subcolumn_mode.subcolumn_count())?;
            let x_subs = x_mode.subcolumn_count();
            let y_cols = parsed_y.len().checked_mul(subcolumn_mode.subcolumn_count()).ok_or_else(overflow)?;
            let total_cols = x_subs.checked_add(y_cols).ok_or_else(overflow)?;
            total_cols.checked_mul(row_count).ok_or_else(overflow)
        }
        TableKind::Column | TableKind::Grouped => {
            validate_subcolumns(subcolumn_mode.subcolumn_count())?;
            let total_cols = parsed_y.len().checked_mul(subcolumn_mode.subcolumn_count()).ok_or_else(overflow)?;
            total_cols.checked_mul(row_count).ok_or_else(overflow)
        }
        TableKind::Contingency | TableKind::PartsOfWhole | TableKind::MultipleVariables => parsed_y.len().checked_mul(row_count).ok_or_else(overflow),
        TableKind::Survival => {
            let total_cols = parsed_y.len().checked_add(1).ok_or_else(overflow)?;
            total_cols.checked_mul(row_count).ok_or_else(overflow)
        }
        TableKind::Nested => {
            let max_units = parsed_y.iter().map(|col| col.subcolumns.len().max(col.declared_subcolumns.unwrap_or(0))).max().unwrap_or(1).max(1);
            validate_subcolumns(max_units)?;
            let mut total: usize = 0;
            for col in parsed_y {
                let units = col.subcolumns.len().max(col.declared_subcolumns.unwrap_or(0)).max(1);
                let group_cells = units.checked_mul(row_count).ok_or_else(overflow)?;
                total = total.checked_add(group_cells).ok_or_else(overflow)?;
            }
            Ok(total)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_table_archetype(
    kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    parsed_x: &ParsedXColumn<'_>,
    parsed_y: &[ParsedYColumn],
    row_titles: Vec<String>,
    has_row_titles: bool,
    row_factor_title: Option<&str>,
    row_count: usize,
) -> Result<TableArchetype, FormatError> {
    match kind {
        TableKind::Xy => {
            let mut xy = XyTable::new_with_x_mode(parsed_x.mode.clone(), subcolumn_mode, parsed_y.len(), row_count)?;
            xy.x_title = parsed_x.title.clone();
            xy.x_display_decimals = parsed_x.display_decimals;
            xy.x_subcolumn_titles = build_optional_subcolumn_titles(&parsed_x.sub_titles, xy.x_mode.subcolumn_count());
            xy.row_titles = row_titles;
            xy.row_titles_expanded = has_row_titles;
            for (s, sub_cells) in parsed_x.cols.iter().enumerate() {
                if s < xy.x_mode.subcolumn_count() {
                    for (r, cell) in sub_cells.iter().enumerate() {
                        xy.set_x_subcell(r, s, cell.clone())?;
                    }
                }
            }
            fill_datasets(&mut xy.data_sets, parsed_y);
            Ok(TableArchetype::Xy(xy))
        }
        TableKind::Column => {
            let hint = if subcolumn_mode.is_pre_summarized() { ColumnDesignHint::PrecomputedError } else { ColumnDesignHint::UnpairedReplicates };
            let mut col = ColumnTable::new_with_subcolumns(hint, subcolumn_mode, parsed_y.len(), row_count)?;
            col.row_titles = row_titles;
            col.row_titles_expanded = has_row_titles;
            fill_datasets(&mut col.data_sets, parsed_y);
            Ok(TableArchetype::Column(col))
        }
        TableKind::Grouped => {
            let mut grp = GroupedTable::new(subcolumn_mode, parsed_y.len(), row_count)?;
            if let Some(rf) = row_factor_title {
                grp.row_factor_title = rf.to_owned();
            }
            grp.row_titles = row_titles;
            fill_datasets(&mut grp.data_sets, parsed_y);
            Ok(TableArchetype::Grouped(grp))
        }
        TableKind::Contingency => {
            let mut cont = ContingencyTable::new(parsed_y.len(), row_count)?;
            if let Some(rf) = row_factor_title {
                cont.row_factor_title = rf.to_owned();
            }
            cont.row_titles = row_titles;
            fill_datasets(&mut cont.data_sets, parsed_y);
            Ok(TableArchetype::Contingency(cont))
        }
        TableKind::Survival => {
            let mut surv = SurvivalTable::new(parsed_y.len(), row_count)?;
            surv.time_title = parsed_x.title.clone();
            surv.row_titles = row_titles;
            surv.row_titles_expanded = has_row_titles;
            if let Some(first_x) = parsed_x.cols.first() {
                for (r, cell) in first_x.iter().enumerate() {
                    if let Some(slot) = surv.times.get_mut(r) {
                        *slot = cell.clone();
                    }
                }
            }
            fill_datasets(&mut surv.cohorts, parsed_y);
            Ok(TableArchetype::Survival(surv))
        }
        TableKind::PartsOfWhole => {
            let mut pow = PartsOfWholeTable::new(parsed_y.len(), row_count)?;
            pow.slice_labels = if has_row_titles { row_titles } else { vec![String::new(); row_count] };
            fill_datasets(&mut pow.data_sets, parsed_y);
            Ok(TableArchetype::PartsOfWhole(pow))
        }
        TableKind::MultipleVariables => {
            let mut mv = MultipleVariablesTable::new(parsed_y.len(), row_count)?;
            mv.row_titles = row_titles;
            mv.row_titles_expanded = has_row_titles;
            for (c_idx, col) in parsed_y.iter().enumerate() {
                let Some(var) = mv.variables.get_mut(c_idx) else {
                    continue;
                };
                var.title = col.title.clone();
                var.display_decimals = col.display_decimals;
                if let Some(first_sub) = col.subcolumns.first() {
                    for (r, cell) in first_sub.iter().enumerate() {
                        if let Some(slot) = var.cells.get_mut(r) {
                            *slot = cell.clone();
                        }
                    }
                }
                if let Some(explicit_role) = col.mv_type {
                    var.lock_role(explicit_role);
                } else {
                    var.refresh_inferred_role();
                }
            }
            Ok(TableArchetype::MultipleVariables(mv))
        }
        TableKind::Nested => {
            let max_units = parsed_y.iter().map(|col| col.subcolumns.len().max(col.declared_subcolumns.unwrap_or(0))).max().unwrap_or(1).max(1);
            validate_subcolumns(max_units)?;
            let mut nested = NestedTable::new(parsed_y.len(), 1, 0)?;
            nested.row_titles = row_titles;
            nested.row_titles_expanded = has_row_titles;
            for (g_idx, col) in parsed_y.iter().enumerate() {
                let units = col.subcolumns.len().max(col.declared_subcolumns.unwrap_or(0)).max(1);
                let mut grp = NestedGroupColumn::new(col.title.clone(), units, row_count)?;
                for (s, sub_cells) in col.subcolumns.iter().enumerate() {
                    for (r, cell) in sub_cells.iter().enumerate() {
                        if let Some(slot) = grp.cells.get_mut(r).and_then(|row| row.get_mut(s)) {
                            *slot = cell.clone();
                        }
                    }
                }
                if let Some(slot) = nested.groups.get_mut(g_idx) {
                    *slot = grp;
                }
            }
            Ok(TableArchetype::Nested(nested))
        }
    }
}

fn build_optional_subcolumn_titles(sub_titles: &[Option<String>], expected_subcols: usize) -> Vec<String> {
    if expected_subcols == 0 {
        return Vec::new();
    }
    let any_non_empty = sub_titles.iter().take(expected_subcols).any(|t| t.as_deref().is_some_and(|s| !s.is_empty()));
    if !any_non_empty {
        return Vec::new();
    }
    (0..expected_subcols).map(|idx| sub_titles.get(idx).and_then(|o| o.clone()).unwrap_or_default()).collect()
}

fn apply_table_subcolumn_titles(arch: &mut TableArchetype, parsed_y: &[ParsedYColumn], sct: Option<&ParsedSubColumnTitles>) {
    match arch {
        TableArchetype::Xy(t) => apply_dataset_subcolumn_titles(&mut t.data_sets, t.y_subcolumn_mode.subcolumn_count(), parsed_y, sct),
        TableArchetype::Column(t) => apply_dataset_subcolumn_titles(&mut t.data_sets, t.subcolumn_mode.subcolumn_count(), parsed_y, sct),
        TableArchetype::Grouped(t) => apply_dataset_subcolumn_titles(&mut t.data_sets, t.subcolumn_mode.subcolumn_count(), parsed_y, sct),
        TableArchetype::Contingency(t) => apply_dataset_subcolumn_titles(&mut t.data_sets, 1, parsed_y, sct),
        TableArchetype::Survival(t) => apply_dataset_subcolumn_titles(&mut t.cohorts, 1, parsed_y, sct),
        TableArchetype::PartsOfWhole(t) => apply_dataset_subcolumn_titles(&mut t.data_sets, 1, parsed_y, sct),
        TableArchetype::MultipleVariables(_) => {}
        TableArchetype::Nested(nested) => apply_nested_subcolumn_titles(nested, parsed_y, sct),
    }
}

fn apply_dataset_subcolumn_titles(
    data_sets: &mut [DataSetColumn],
    expected_subcols: usize,
    parsed_y: &[ParsedYColumn],
    sct: Option<&ParsedSubColumnTitles>,
) {
    if expected_subcols == 0 {
        return;
    }
    for (c_idx, ds) in data_sets.iter_mut().enumerate() {
        let mut resolved = vec![String::new(); expected_subcols];
        if let Some(spec) = sct {
            let offset = if spec.own_set { c_idx.saturating_mul(expected_subcols) } else { 0 };
            for (s_idx, slot) in resolved.iter_mut().enumerate() {
                if let Some(title) = spec.flat.get(offset + s_idx)
                    && !title.is_empty()
                {
                    *slot = title.clone();
                }
            }
        }
        if let Some(col) = parsed_y.get(c_idx) {
            for (s_idx, slot) in resolved.iter_mut().enumerate() {
                if slot.is_empty()
                    && let Some(Some(sub_title)) = col.sub_titles.get(s_idx)
                    && !sub_title.is_empty()
                {
                    *slot = sub_title.clone();
                }
            }
        }
        if resolved.iter().any(|t| !t.is_empty()) {
            ds.subcolumn_titles = resolved;
        }
    }
}

fn apply_nested_subcolumn_titles(nested: &mut NestedTable, parsed_y: &[ParsedYColumn], sct: Option<&ParsedSubColumnTitles>) {
    let mut flat_cursor = 0_usize;
    for (g_idx, grp) in nested.groups.iter_mut().enumerate() {
        let units = grp.subcolumn_titles.len();
        if let Some(spec) = sct {
            let offset = if spec.own_set { flat_cursor } else { 0 };
            for (s_idx, slot) in grp.subcolumn_titles.iter_mut().enumerate() {
                if let Some(title) = spec.flat.get(offset + s_idx)
                    && !title.is_empty()
                {
                    *slot = title.clone();
                }
            }
            flat_cursor = flat_cursor.saturating_add(units);
        }
        if let Some(col) = parsed_y.get(g_idx) {
            for (s_idx, maybe_title) in col.sub_titles.iter().enumerate() {
                if let Some(sub_title) = maybe_title
                    && !sub_title.is_empty()
                    && let Some(slot) = grp.subcolumn_titles.get_mut(s_idx)
                {
                    *slot = sub_title.clone();
                }
            }
        }
    }
}

struct XModeResolution {
    mode: XColumnMode,
    is_plus_minus: bool,
    is_text_x: bool,
    is_no_n: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryFormatInfo {
    /// Native v9 2-subcolumn no-N summary format (`SD` -> `MeanSd`, `SE`/`SEM` -> `MeanSem`),
    /// upgrading to `three_sub_mode` only when a file contradicts the format with 3 subcolumns.
    NativeTwoSubcolumn { two_sub_mode: SubcolumnMode, three_sub_mode: SubcolumnMode },
    /// No-N 2-subcolumn `%CV` format (no `MeanCv` mode per §3.5; stored in 3-subcolumn `MeanCvN`
    /// with empty N + `MissingMetadata` warning).
    PaddedNoNCv,
    /// Symmetric 2-subcolumn `PlusMinus` error format, expanded into `MeanLowHigh`.
    PlusMinus,
    /// Standard 3-subcolumn summary format (`SDN`, `SEN`/`SEMN`, `CVN`, `low-high`, `upper-lower-limits`, `median-iqr`).
    ThreeSubcolumn(SubcolumnMode),
}

impl SummaryFormatInfo {
    const fn expected_subcolumns(self) -> usize {
        match self {
            Self::NativeTwoSubcolumn { .. } | Self::PaddedNoNCv | Self::PlusMinus => 2,
            Self::ThreeSubcolumn(_) => 3,
        }
    }
}

fn classify_summary_format(fmt_lower: &str) -> Option<SummaryFormatInfo> {
    match fmt_lower {
        "sd" => Some(SummaryFormatInfo::NativeTwoSubcolumn { two_sub_mode: SubcolumnMode::MeanSd, three_sub_mode: SubcolumnMode::MeanSdN }),
        "se" | "sem" => Some(SummaryFormatInfo::NativeTwoSubcolumn { two_sub_mode: SubcolumnMode::MeanSem, three_sub_mode: SubcolumnMode::MeanSemN }),
        "cv" => Some(SummaryFormatInfo::PaddedNoNCv),
        "plus-minus" | "plusminus" | "meanplusminus" => Some(SummaryFormatInfo::PlusMinus),
        "sdn" | "meansdn" => Some(SummaryFormatInfo::ThreeSubcolumn(SubcolumnMode::MeanSdN)),
        "sen" | "semn" | "meansemn" => Some(SummaryFormatInfo::ThreeSubcolumn(SubcolumnMode::MeanSemN)),
        "cvn" | "meancvn" => Some(SummaryFormatInfo::ThreeSubcolumn(SubcolumnMode::MeanCvN)),
        "low-high" | "lowhigh" | "meanlowhigh" => Some(SummaryFormatInfo::ThreeSubcolumn(SubcolumnMode::MeanLowHigh)),
        "upper-lower" | "upper-lower-limits" | "upperlowerlimits" | "upperlower" | "limits" => {
            Some(SummaryFormatInfo::ThreeSubcolumn(SubcolumnMode::UpperLowerLimits))
        }
        "median-iqr" | "medianiqr" | "iqr" => Some(SummaryFormatInfo::ThreeSubcolumn(SubcolumnMode::MedianIqr)),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_x_mode(
    kind: TableKind,
    table_el: &XmlElement,
    x_el: Option<&XmlElement>,
    explicit_x_title: Option<&str>,
    x_cols: &[Vec<Cell>],
    x_declared_subs: Option<usize>,
    scope: NumericParseScope,
    warnings: &mut Vec<ImportWarning>,
    table_id: &str,
) -> Result<XModeResolution, FormatError> {
    let x_fmt = table_el.attr("XFormat").unwrap_or("").trim();
    if x_fmt.eq_ignore_ascii_case("text") || x_fmt.eq_ignore_ascii_case("labels") || x_fmt.eq_ignore_ascii_case("categories") {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            table_id,
            format!(
                "table `{table_id}` uses categorical XFormat=\"{x_fmt}\"; CurveBook XY tables store numeric X values, so text X cells and row labels were preserved as-is"
            ),
        ));
        return Ok(XModeResolution {
            mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
            is_plus_minus: false,
            is_text_x: true,
            is_no_n: false,
        });
    }
    if x_fmt.eq_ignore_ascii_case("date") || x_fmt.eq_ignore_ascii_case("time") || x_fmt.eq_ignore_ascii_case("elapsed-time") {
        let unit_label = explicit_x_title
            .map(str::to_owned)
            .or_else(|| x_el.and_then(extract_element_title).map(|raw| clamp_title(&raw, table_id, warnings)))
            .unwrap_or_else(|| String::from("Time"));
        if x_cols.first().is_some_and(|col| col.iter().any(|c| matches!(c.value, CellValue::Text(_)))) {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                table_id,
                format!(
                    "table `{table_id}` XFormat=\"{x_fmt}\" contains string date/time entries preserved as text without automatic calendar conversion"
                ),
            ));
        }
        return Ok(XModeResolution { mode: XColumnMode::DateOrElapsedTime { unit_label }, is_plus_minus: false, is_text_x: false, is_no_n: false });
    }
    if x_fmt.eq_ignore_ascii_case("start-step") || x_fmt.eq_ignore_ascii_case("sequence") {
        let start_raw = table_attr_or_x_prop(table_el, x_el, &["X0", "Start"]);
        let step_raw = table_attr_or_x_prop(table_el, x_el, &["DeltaX", "Step"]);
        let start = parse_sequence_param(start_raw.as_deref(), 0.0, "X0", scope, warnings, table_id);
        let step = parse_sequence_param(step_raw.as_deref(), 1.0, "DeltaX", scope, warnings, table_id);
        return Ok(XModeResolution {
            mode: XColumnMode::GeneratedSequence { start, step, progression: SequenceProgression::Arithmetic },
            is_plus_minus: false,
            is_text_x: false,
            is_no_n: false,
        });
    }
    if x_fmt.eq_ignore_ascii_case("replicates") {
        let count = x_cols.len().max(x_declared_subs.unwrap_or(0)).max(1);
        validate_subcolumns(count)?;
        return Ok(XModeResolution {
            mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(count) },
            is_plus_minus: false,
            is_text_x: false,
            is_no_n: false,
        });
    }
    if kind != TableKind::Xy {
        return Ok(XModeResolution {
            mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
            is_plus_minus: false,
            is_text_x: false,
            is_no_n: false,
        });
    }
    let x_ev = table_el.attr("XEVFormat").map(str::trim).filter(|s| !s.is_empty());
    let has_err_x = x_fmt.eq_ignore_ascii_case("err")
        || x_fmt.eq_ignore_ascii_case("error")
        || x_ev.is_some_and(|ev| !ev.is_empty() && (x_cols.len() >= 2 || x_declared_subs.is_some_and(|n| n >= 2)))
        || x_cols.len() == 3;
    if has_err_x {
        return Ok(resolve_err_x_mode(x_fmt, x_ev, x_cols.len(), x_declared_subs, table_id, warnings));
    }
    Ok(XModeResolution {
        mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
        is_plus_minus: false,
        is_text_x: false,
        is_no_n: false,
    })
}

fn resolve_err_x_mode(
    x_fmt: &str,
    x_ev: Option<&str>,
    actual_subs: usize,
    declared_subs: Option<usize>,
    table_id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> XModeResolution {
    let Some(raw_ev) = x_ev else {
        return resolve_unspecified_err_x_mode(x_fmt, actual_subs, declared_subs, table_id, warnings);
    };
    let ev_lower = raw_ev.to_ascii_lowercase();
    let Some(info) = classify_summary_format(&ev_lower) else {
        let is_two = declared_subs == Some(2) || (declared_subs.is_none() && actual_subs == 2);
        let fallback_mode = if is_two { SubcolumnMode::MeanSd } else { SubcolumnMode::MeanSdN };
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            table_id,
            format!("table `{table_id}` has unrecognized XEVFormat=\"{raw_ev}\", defaulting to {fallback_mode:?}"),
        ));
        return XModeResolution {
            mode: XColumnMode::Numbers { subcolumn_mode: fallback_mode },
            is_plus_minus: false,
            is_text_x: false,
            is_no_n: false,
        };
    };

    let expected = info.expected_subcolumns();
    if let Some(decl) = declared_subs
        && decl != expected
    {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            table_id,
            format!(
                "table `{table_id}` XColumn declares Subcolumns=\"{decl}\" which contradicts XEVFormat=\"{raw_ev}\" (expected {expected} subcolumns)"
            ),
        ));
        if decl == 1 && actual_subs <= 1 {
            return XModeResolution {
                mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
                is_plus_minus: false,
                is_text_x: false,
                is_no_n: false,
            };
        }
    }

    let effective_subs = declared_subs.unwrap_or(actual_subs);
    match info {
        SummaryFormatInfo::NativeTwoSubcolumn { two_sub_mode, three_sub_mode } => {
            if effective_subs >= 3 {
                if declared_subs.is_none() {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        table_id,
                        format!("table `{table_id}` XColumn has {actual_subs} subcolumns contradicting 2-subcolumn XEVFormat=\"{raw_ev}\"; stored as {three_sub_mode:?}"),
                    ));
                }
                XModeResolution {
                    mode: XColumnMode::Numbers { subcolumn_mode: three_sub_mode },
                    is_plus_minus: false,
                    is_text_x: false,
                    is_no_n: false,
                }
            } else {
                XModeResolution {
                    mode: XColumnMode::Numbers { subcolumn_mode: two_sub_mode },
                    is_plus_minus: false,
                    is_text_x: false,
                    is_no_n: false,
                }
            }
        }
        SummaryFormatInfo::PaddedNoNCv => {
            if declared_subs.is_none() && actual_subs >= 3 {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::CoercedValue,
                    table_id,
                    format!("table `{table_id}` XColumn has {actual_subs} subcolumns contradicting no-N XEVFormat=\"{raw_ev}\" (expected 2); stored as MeanCvN"),
                ));
            }
            if effective_subs <= 2 {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    table_id,
                    format!("table `{table_id}` XColumn uses no-N `%CV` summary XEVFormat=\"{raw_ev}\" (2 subcolumns without N); stored in MeanCvN with empty N slot"),
                ));
                return XModeResolution {
                    mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanCvN },
                    is_plus_minus: false,
                    is_text_x: false,
                    is_no_n: true,
                };
            }
            XModeResolution {
                mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanCvN },
                is_plus_minus: false,
                is_text_x: false,
                is_no_n: false,
            }
        }
        SummaryFormatInfo::PlusMinus => {
            if effective_subs == 3 {
                if declared_subs.is_none() {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        table_id,
                        format!(
                            "table `{table_id}` XColumn has 3 subcolumns contradicting 2-subcolumn XEVFormat=\"{raw_ev}\"; stored as MeanLowHigh"
                        ),
                    ));
                }
                return XModeResolution {
                    mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanLowHigh },
                    is_plus_minus: false,
                    is_text_x: false,
                    is_no_n: false,
                };
            }
            XModeResolution {
                mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanLowHigh },
                is_plus_minus: true,
                is_text_x: false,
                is_no_n: false,
            }
        }
        SummaryFormatInfo::ThreeSubcolumn(base_mode) => {
            if declared_subs == Some(2) && actual_subs < 2 {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    table_id,
                    format!(
                        "table `{table_id}` XColumn declares 2 subcolumns for 3-subcolumn mode {base_mode:?}; third subcolumn (N/limit) left empty"
                    ),
                ));
            }
            XModeResolution { mode: XColumnMode::Numbers { subcolumn_mode: base_mode }, is_plus_minus: false, is_text_x: false, is_no_n: false }
        }
    }
}

fn resolve_unspecified_err_x_mode(
    x_fmt: &str,
    actual_subs: usize,
    declared_subs: Option<usize>,
    table_id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> XModeResolution {
    if let Some(decl) = declared_subs {
        if decl == 1 && actual_subs <= 1 {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                table_id,
                format!("table `{table_id}` XColumn declares Subcolumns=\"1\" which contradicts XFormat=\"{x_fmt}\"; using Single"),
            ));
            return XModeResolution {
                mode: XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
                is_plus_minus: false,
                is_text_x: false,
                is_no_n: false,
            };
        }
        if decl > 3 {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                table_id,
                format!("table `{table_id}` XColumn declares Subcolumns=\"{decl}\" which contradicts XFormat=\"{x_fmt}\" (max 3 summary subcolumns)"),
            ));
        }
    }
    let is_two_sub = declared_subs == Some(2) || (declared_subs.is_none() && actual_subs == 2);
    let sub_mode = if is_two_sub { SubcolumnMode::MeanSd } else { SubcolumnMode::MeanSdN };
    XModeResolution { mode: XColumnMode::Numbers { subcolumn_mode: sub_mode }, is_plus_minus: false, is_text_x: false, is_no_n: false }
}

fn table_attr_or_x_prop(table_el: &XmlElement, x_el: Option<&XmlElement>, names: &[&str]) -> Option<String> {
    for name in names {
        if let Some(v) = table_el.attr(name).map(str::trim).filter(|s| !s.is_empty()) {
            return Some(v.to_owned());
        }
        if let Some(el) = x_el {
            if let Some(v) = el.attr(name).map(str::trim).filter(|s| !s.is_empty()) {
                return Some(v.to_owned());
            }
            if let Some(child) = el.first_child(name) {
                let text = pzfx_element_to_plain_text(child);
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
        }
    }
    None
}

fn parse_sequence_param(
    raw: Option<&str>,
    default: f64,
    label: &str,
    scope: NumericParseScope,
    warnings: &mut Vec<ImportWarning>,
    table_id: &str,
) -> f64 {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return default;
    };
    match parse_pzfx_scalar_f64(text, scope.table_ev, scope.doc_ev, scope.explicit_decimal_comma) {
        Some(val) => val,
        None => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                table_id,
                format!("non-finite or invalid sequence attribute {label}=\"{text}\" in table `{table_id}`; defaulted to {default}"),
            ));
            default
        }
    }
}

fn fill_datasets(target: &mut [DataSetColumn], parsed_y: &[ParsedYColumn]) {
    for (c_idx, col) in parsed_y.iter().enumerate() {
        let Some(ds) = target.get_mut(c_idx) else {
            continue;
        };
        ds.title = col.title.clone();
        ds.display_decimals = col.display_decimals;
        let allowed_subs = ds.subcolumn_count();
        for (s, sub_cells) in col.subcolumns.iter().enumerate() {
            if s >= allowed_subs {
                break;
            }
            for (r, cell) in sub_cells.iter().enumerate() {
                if let Some(slot) = ds.cell_mut(r, s) {
                    *slot = cell.clone();
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvFormatRole {
    AsteriskAfter,
    AsteriskBefore,
    AsteriskEither,
    NonNumericOrBlank,
    SubcolumnModeFallback,
    Unspecified,
}

#[derive(Debug, Clone, Copy)]
struct CellParseOptions {
    ev_role: EvFormatRole,
    column_excluded: bool,
}

impl Default for CellParseOptions {
    fn default() -> Self {
        Self { ev_role: EvFormatRole::Unspecified, column_excluded: false }
    }
}

fn classify_ev_format(raw: Option<&str>) -> EvFormatRole {
    let trimmed = raw.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return EvFormatRole::Unspecified;
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "asteriskafternumber" | "asteriskafter" | "asterisk-after" => EvFormatRole::AsteriskAfter,
        "asteriskbeforenumber" | "asteriskbefore" | "asterisk-before" => EvFormatRole::AsteriskBefore,
        "asterisk" | "asteriskbeforeorafter" => EvFormatRole::AsteriskEither,
        "blank" | "workingwithexcluded" | "workingwith" | "exclude" | "empty" => EvFormatRole::NonNumericOrBlank,
        _ => EvFormatRole::SubcolumnModeFallback,
    }
}

fn element_is_excluded(el: &XmlElement) -> bool {
    is_truthy_attr(el.attr("Excluded"))
}

fn strip_asterisk_for_ev(trimmed: &str, ev_role: EvFormatRole, attr_excluded: bool) -> (&str, bool) {
    match ev_role {
        EvFormatRole::AsteriskAfter => {
            if let Some(core) = trimmed.strip_suffix('*') {
                return (core.trim(), true);
            }
        }
        EvFormatRole::AsteriskBefore => {
            if let Some(core) = trimmed.strip_prefix('*') {
                return (core.trim(), true);
            }
        }
        EvFormatRole::AsteriskEither => {
            if let Some(core) = trimmed.strip_suffix('*').or_else(|| trimmed.strip_prefix('*')) {
                return (core.trim(), true);
            }
        }
        EvFormatRole::Unspecified | EvFormatRole::SubcolumnModeFallback | EvFormatRole::NonNumericOrBlank => {
            if attr_excluded && let Some(core) = trimmed.strip_suffix('*').or_else(|| trimmed.strip_prefix('*')) {
                return (core.trim(), true);
            }
        }
    }
    (trimmed, false)
}

#[derive(Debug, Clone, Copy, Default)]
struct DocDecimalContext {
    root_decimal_comma: Option<bool>,
    doc_evidence: DecimalEvidence,
}

#[derive(Debug, Clone, Copy, Default)]
struct DecimalEvidence {
    dot: usize,
    comma: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvidenceBias {
    None,
    Dot,
    Comma,
    Conflicting,
}

impl DecimalEvidence {
    fn record(&mut self, class: PzfxTokenClass) {
        match class {
            PzfxTokenClass::DotNumber(_) => self.dot = self.dot.saturating_add(1),
            PzfxTokenClass::CommaNumber(_) => self.comma = self.comma.saturating_add(1),
            PzfxTokenClass::Empty
            | PzfxTokenClass::PlainText
            | PzfxTokenClass::NeutralNumber(_)
            | PzfxTokenClass::AmbiguousThreeDigitComma { .. }
            | PzfxTokenClass::MalformedNumeric => {}
        }
    }

    fn merge(&mut self, other: Self) {
        self.dot = self.dot.saturating_add(other.dot);
        self.comma = self.comma.saturating_add(other.comma);
    }

    const fn bias(self) -> EvidenceBias {
        match (self.dot > 0, self.comma > 0) {
            (false, false) => EvidenceBias::None,
            (true, false) => EvidenceBias::Dot,
            (false, true) => EvidenceBias::Comma,
            (true, true) => EvidenceBias::Conflicting,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct NumericParseScope {
    table_ev: DecimalEvidence,
    doc_ev: DecimalEvidence,
    explicit_decimal_comma: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ColumnParseContext {
    col_ev: DecimalEvidence,
    table_ev: DecimalEvidence,
    doc_ev: DecimalEvidence,
    col_decimals: Option<usize>,
    explicit_decimal_comma: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default)]
struct SubcolumnNumericStats {
    numeric_looking_kept_as_text: usize,
    ambiguous_via_decimals_attr: usize,
    ambiguous_zero_context_thousands: usize,
}

impl SubcolumnNumericStats {
    fn merge(&mut self, other: Self) {
        self.numeric_looking_kept_as_text = self.numeric_looking_kept_as_text.saturating_add(other.numeric_looking_kept_as_text);
        self.ambiguous_via_decimals_attr = self.ambiguous_via_decimals_attr.saturating_add(other.ambiguous_via_decimals_attr);
        self.ambiguous_zero_context_thousands = self.ambiguous_zero_context_thousands.saturating_add(other.ambiguous_zero_context_thousands);
    }
}

#[derive(Debug, Clone, Copy)]
enum PzfxTokenClass {
    Empty,
    PlainText,
    NeutralNumber(f64),
    DotNumber(f64),
    CommaNumber(f64),
    AmbiguousThreeDigitComma { as_decimal: f64, as_thousands: f64 },
    MalformedNumeric,
}

#[derive(Debug, Clone, Copy)]
enum AmbiguousCommaResolution {
    Decimal { warned_via_decimals_attr: bool },
    Thousands { warned_zero_context: bool },
    ConflictingKeepText,
}

fn split_sign_mantissa_exponent(trimmed: &str) -> Option<(&str, &str, &str)> {
    let (sign, after_sign) = if let Some(rest) = trimmed.strip_prefix('+') {
        ("+", rest)
    } else if let Some(rest) = trimmed.strip_prefix('-') {
        ("-", rest)
    } else {
        ("", trimmed)
    };
    let (mantissa, exp_suffix) = if let Some(pos) = after_sign.find(['e', 'E']) {
        let mant = after_sign.get(..pos)?;
        let exp_full = after_sign.get(pos..)?;
        let exp_body = exp_full.get(1..)?;
        let exp_digits = exp_body.strip_prefix(['+', '-']).unwrap_or(exp_body);
        if exp_digits.is_empty() || !exp_digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        (mant, exp_full)
    } else {
        (after_sign, "")
    };
    if mantissa.is_empty() || !mantissa.bytes().any(|b| b.is_ascii_digit()) || !mantissa.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b',')
    {
        return None;
    }
    Some((sign, mantissa, exp_suffix))
}

fn is_grouped_thousands(part: &str, sep: char, min_groups_after_first: usize) -> bool {
    let mut groups = part.split(sep);
    let Some(first) = groups.next() else {
        return false;
    };
    if !(1..=3).contains(&first.len()) || first.starts_with('0') || !first.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let mut tail_count = 0_usize;
    for group in groups {
        if group.len() != 3 || !group.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        tail_count += 1;
    }
    tail_count >= min_groups_after_first
}

fn parse_finite_assembled(sign: &str, int_digits: &str, frac_digits: Option<&str>, exp_suffix: &str) -> Option<f64> {
    let cap = sign
        .len()
        .saturating_add(int_digits.len())
        .saturating_add(frac_digits.map_or(0, |f| f.len().saturating_add(1)))
        .saturating_add(exp_suffix.len());
    let mut buf = String::with_capacity(cap);
    buf.push_str(sign);
    buf.push_str(int_digits);
    if let Some(frac) = frac_digits {
        buf.push('.');
        buf.push_str(frac);
    }
    buf.push_str(exp_suffix);
    buf.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn classify_mixed_dot_and_comma(sign: &str, mantissa: &str, exp_suffix: &str, dots: usize, commas: usize) -> PzfxTokenClass {
    if dots == 1
        && let Some((int_part, frac_part)) = mantissa.split_once('.')
        && !frac_part.is_empty()
        && frac_part.bytes().all(|b| b.is_ascii_digit())
        && is_grouped_thousands(int_part, ',', 1)
    {
        let int_digits: String = int_part.bytes().filter(|&b| b != b',').map(char::from).collect();
        return parse_finite_assembled(sign, &int_digits, Some(frac_part), exp_suffix)
            .map_or(PzfxTokenClass::MalformedNumeric, PzfxTokenClass::DotNumber);
    }
    if commas == 1
        && let Some((int_part, frac_part)) = mantissa.split_once(',')
        && !frac_part.is_empty()
        && frac_part.bytes().all(|b| b.is_ascii_digit())
        && is_grouped_thousands(int_part, '.', 1)
    {
        let int_digits: String = int_part.bytes().filter(|&b| b != b'.').map(char::from).collect();
        return parse_finite_assembled(sign, &int_digits, Some(frac_part), exp_suffix)
            .map_or(PzfxTokenClass::MalformedNumeric, PzfxTokenClass::CommaNumber);
    }
    PzfxTokenClass::MalformedNumeric
}

fn classify_single_comma(sign: &str, mantissa: &str, exp_suffix: &str) -> PzfxTokenClass {
    let Some((int_part, frac_part)) = mantissa.split_once(',') else {
        return PzfxTokenClass::MalformedNumeric;
    };
    if int_part.is_empty() || frac_part.is_empty() {
        return PzfxTokenClass::MalformedNumeric;
    }
    if exp_suffix.is_empty() && is_grouped_thousands(mantissa, ',', 1) {
        let joined = format!("{int_part}{frac_part}");
        if let (Some(as_decimal), Some(as_thousands)) =
            (parse_finite_assembled(sign, int_part, Some(frac_part), ""), parse_finite_assembled(sign, &joined, None, ""))
        {
            return PzfxTokenClass::AmbiguousThreeDigitComma { as_decimal, as_thousands };
        }
        return PzfxTokenClass::MalformedNumeric;
    }
    parse_finite_assembled(sign, int_part, Some(frac_part), exp_suffix).map_or(PzfxTokenClass::MalformedNumeric, PzfxTokenClass::CommaNumber)
}

fn classify_pzfx_numeric_token(raw: &str) -> PzfxTokenClass {
    let trimmed_raw = raw.trim();
    if trimmed_raw.is_empty() {
        return PzfxTokenClass::Empty;
    }
    let normalized_minus;
    let trimmed = if trimmed_raw.contains('\u{2212}') {
        normalized_minus = trimmed_raw.replace('\u{2212}', "-");
        normalized_minus.as_str()
    } else {
        trimmed_raw
    };
    let Some((sign, mantissa, exp_suffix)) = split_sign_mantissa_exponent(trimmed) else {
        return PzfxTokenClass::PlainText;
    };
    let dots = mantissa.bytes().filter(|&b| b == b'.').count();
    let commas = mantissa.bytes().filter(|&b| b == b',').count();
    match (commas, dots) {
        (0, 0) => trimmed.parse::<f64>().ok().filter(|v| v.is_finite()).map_or(PzfxTokenClass::MalformedNumeric, PzfxTokenClass::NeutralNumber),
        (0, 1) => trimmed.parse::<f64>().ok().filter(|v| v.is_finite()).map_or(PzfxTokenClass::MalformedNumeric, PzfxTokenClass::DotNumber),
        (0, _) => {
            if exp_suffix.is_empty() && is_grouped_thousands(mantissa, '.', 2) {
                let digits: String = mantissa.bytes().filter(|&b| b != b'.').map(char::from).collect();
                parse_finite_assembled(sign, &digits, None, "").map_or(PzfxTokenClass::MalformedNumeric, PzfxTokenClass::CommaNumber)
            } else {
                PzfxTokenClass::MalformedNumeric
            }
        }
        (1, 0) => classify_single_comma(sign, mantissa, exp_suffix),
        (_, 0) => {
            if exp_suffix.is_empty() && is_grouped_thousands(mantissa, ',', 2) {
                let digits: String = mantissa.bytes().filter(|&b| b != b',').map(char::from).collect();
                parse_finite_assembled(sign, &digits, None, "").map_or(PzfxTokenClass::MalformedNumeric, PzfxTokenClass::DotNumber)
            } else {
                PzfxTokenClass::MalformedNumeric
            }
        }
        _ => classify_mixed_dot_and_comma(sign, mantissa, exp_suffix, dots, commas),
    }
}

fn resolve_ambiguous_three_digit_comma(ctx: ColumnParseContext) -> AmbiguousCommaResolution {
    if let Some(is_comma) = ctx.explicit_decimal_comma {
        return if is_comma {
            AmbiguousCommaResolution::Decimal { warned_via_decimals_attr: false }
        } else {
            AmbiguousCommaResolution::Thousands { warned_zero_context: false }
        };
    }
    for scope_ev in [ctx.col_ev, ctx.table_ev] {
        match scope_ev.bias() {
            EvidenceBias::Comma => return AmbiguousCommaResolution::Decimal { warned_via_decimals_attr: false },
            EvidenceBias::Dot => return AmbiguousCommaResolution::Thousands { warned_zero_context: false },
            EvidenceBias::Conflicting => return AmbiguousCommaResolution::ConflictingKeepText,
            EvidenceBias::None => {}
        }
    }
    match ctx.doc_ev.bias() {
        EvidenceBias::Comma => return AmbiguousCommaResolution::Decimal { warned_via_decimals_attr: false },
        EvidenceBias::Dot => return AmbiguousCommaResolution::Thousands { warned_zero_context: false },
        EvidenceBias::None | EvidenceBias::Conflicting => {}
    }
    match ctx.col_decimals {
        Some(3) => AmbiguousCommaResolution::Decimal { warned_via_decimals_attr: true },
        Some(_) => AmbiguousCommaResolution::Thousands { warned_zero_context: false },
        None if ctx.doc_ev.bias() == EvidenceBias::Conflicting => AmbiguousCommaResolution::ConflictingKeepText,
        None => AmbiguousCommaResolution::Thousands { warned_zero_context: true },
    }
}

fn parse_pzfx_scalar_f64(raw: &str, local_ev: DecimalEvidence, doc_ev: DecimalEvidence, explicit_decimal_comma: Option<bool>) -> Option<f64> {
    match classify_pzfx_numeric_token(raw) {
        PzfxTokenClass::NeutralNumber(v) | PzfxTokenClass::DotNumber(v) | PzfxTokenClass::CommaNumber(v) => Some(v),
        PzfxTokenClass::AmbiguousThreeDigitComma { as_decimal, as_thousands } => {
            match resolve_ambiguous_three_digit_comma(ColumnParseContext {
                col_ev: local_ev,
                table_ev: local_ev,
                doc_ev,
                col_decimals: None,
                explicit_decimal_comma,
            }) {
                AmbiguousCommaResolution::Decimal { .. } => Some(as_decimal),
                AmbiguousCommaResolution::Thousands { .. } => Some(as_thousands),
                AmbiguousCommaResolution::ConflictingKeepText => None,
            }
        }
        PzfxTokenClass::Empty | PzfxTokenClass::PlainText | PzfxTokenClass::MalformedNumeric => None,
    }
}

fn parse_decimals_attr(col_el: &XmlElement) -> Option<usize> {
    col_el.attr("Decimals").map(str::trim).filter(|s| !s.is_empty()).and_then(|s| s.parse::<usize>().ok())
}

fn collect_column_decimal_evidence(col_el: &XmlElement) -> DecimalEvidence {
    let mut ev = DecimalEvidence::default();
    for sub_el in col_el.children_named("Subcolumn") {
        for d_el in sub_el.children_named("d") {
            let text = pzfx_element_to_plain_text(d_el);
            ev.record(classify_pzfx_numeric_token(text.trim().trim_matches('*')));
        }
    }
    for d_el in col_el.children_named("d") {
        let text = pzfx_element_to_plain_text(d_el);
        ev.record(classify_pzfx_numeric_token(text.trim().trim_matches('*')));
    }
    ev
}

fn collect_info_decimal_evidence(info_el: &XmlElement) -> DecimalEvidence {
    let mut ev = DecimalEvidence::default();
    for c_el in info_el.children_named("Constant") {
        if let Some(val_raw) = c_el.first_child("Value").map(pzfx_element_to_plain_text).or_else(|| c_el.attr("Value").map(str::to_owned)) {
            ev.record(classify_pzfx_numeric_token(&val_raw));
        }
    }
    ev
}

fn table_x_is_numeric_source(table_el: &XmlElement, kind: TableKind) -> bool {
    if !matches!(kind, TableKind::Xy | TableKind::Survival) {
        return false;
    }
    let x_fmt = table_el.attr("XFormat").unwrap_or("").trim();
    !(x_fmt.eq_ignore_ascii_case("date")
        || x_fmt.eq_ignore_ascii_case("time")
        || x_fmt.eq_ignore_ascii_case("elapsed-time")
        || x_fmt.eq_ignore_ascii_case("start-step")
        || x_fmt.eq_ignore_ascii_case("sequence"))
}

fn collect_table_decimal_evidence(table_el: &XmlElement, kind: TableKind) -> DecimalEvidence {
    let mut ev = DecimalEvidence::default();
    let x_fmt = table_el.attr("XFormat").unwrap_or("").trim();
    if x_fmt.eq_ignore_ascii_case("start-step") || x_fmt.eq_ignore_ascii_case("sequence") {
        let x_el = table_el.first_child("XColumn");
        for names in [&["X0", "Start"][..], &["DeltaX", "Step"][..]] {
            if let Some(val) = table_attr_or_x_prop(table_el, x_el, names) {
                ev.record(classify_pzfx_numeric_token(&val));
            }
        }
    }
    if table_x_is_numeric_source(table_el, kind)
        && let Some(x_el) = table_el.first_child("XColumn").or_else(|| table_el.first_child("XAdvancedColumn"))
    {
        ev.merge(collect_column_decimal_evidence(x_el));
    }
    for y_el in table_el.children_named("YColumn").chain(table_el.children_named("Column")).chain(table_el.children_named("VariableColumn")) {
        ev.merge(collect_column_decimal_evidence(y_el));
    }
    ev
}

fn peek_table_kind_quiet(table_type: Option<&str>, ext_table_type: Option<&str>) -> TableKind {
    if let Some(ext) = ext_table_type.map(str::trim).filter(|s| !s.is_empty()) {
        if ext.eq_ignore_ascii_case("nested") {
            return TableKind::Nested;
        }
        if ext.eq_ignore_ascii_case("multiplevariables") || ext.eq_ignore_ascii_case("multiple-variables") {
            return TableKind::MultipleVariables;
        }
    }
    match table_type.map(str::trim).unwrap_or("") {
        "XY" | "xy" => TableKind::Xy,
        "TwoWay" | "Grouped" | "grouped" => TableKind::Grouped,
        "Contingency" | "contingency" => TableKind::Contingency,
        "Survival" | "survival" => TableKind::Survival,
        "PartsOfWhole" | "parts-of-whole" => TableKind::PartsOfWhole,
        "MultipleVariables" | "multiple-variables" => TableKind::MultipleVariables,
        "Nested" | "nested" => TableKind::Nested,
        _ => TableKind::Column,
    }
}

fn collect_document_decimal_evidence(root: &XmlElement) -> DecimalEvidence {
    let mut ev = DecimalEvidence::default();
    for info_el in root.children_named("InfoSequence").flat_map(|s| s.children_named("Info")).chain(root.children_named("Info")) {
        ev.merge(collect_info_decimal_evidence(info_el));
    }
    let ordered_tables = collect_ordered_table_elements(root);
    for entry in &ordered_tables.entries {
        for info_el in entry.el.children_named("Info") {
            ev.merge(collect_info_decimal_evidence(info_el));
        }
        let kind = peek_table_kind_quiet(entry.el.attr("TableType"), entry.el.attr("ExtTableType"));
        ev.merge(collect_table_decimal_evidence(entry.el, kind));
    }
    ev
}

fn parse_pzfx_cell_with_context(d_el: &XmlElement, opts: CellParseOptions, ctx: ColumnParseContext, stats: &mut SubcolumnNumericStats) -> Cell {
    let attr_excluded = opts.column_excluded || element_is_excluded(d_el);
    let plain = pzfx_element_to_plain_text(d_el);
    let trimmed = plain.trim();
    if trimmed.is_empty() {
        return Cell { value: CellValue::Empty, excluded: attr_excluded };
    }
    let (candidate, asterisk_excluded) = strip_asterisk_for_ev(trimmed, opts.ev_role, attr_excluded);
    let excluded = attr_excluded || asterisk_excluded;
    match classify_pzfx_numeric_token(candidate) {
        PzfxTokenClass::Empty => Cell { value: CellValue::Empty, excluded },
        PzfxTokenClass::NeutralNumber(num) | PzfxTokenClass::DotNumber(num) | PzfxTokenClass::CommaNumber(num) => {
            Cell { value: CellValue::Number(num), excluded }
        }
        PzfxTokenClass::AmbiguousThreeDigitComma { as_decimal, as_thousands } => match resolve_ambiguous_three_digit_comma(ctx) {
            AmbiguousCommaResolution::Decimal { warned_via_decimals_attr } => {
                if warned_via_decimals_attr {
                    stats.ambiguous_via_decimals_attr = stats.ambiguous_via_decimals_attr.saturating_add(1);
                }
                Cell { value: CellValue::Number(as_decimal), excluded }
            }
            AmbiguousCommaResolution::Thousands { warned_zero_context } => {
                if warned_zero_context {
                    stats.ambiguous_zero_context_thousands = stats.ambiguous_zero_context_thousands.saturating_add(1);
                }
                Cell { value: CellValue::Number(as_thousands), excluded }
            }
            AmbiguousCommaResolution::ConflictingKeepText => {
                stats.numeric_looking_kept_as_text = stats.numeric_looking_kept_as_text.saturating_add(1);
                Cell { value: CellValue::Text(pzfx_element_to_markup(d_el)), excluded }
            }
        },
        PzfxTokenClass::MalformedNumeric => {
            stats.numeric_looking_kept_as_text = stats.numeric_looking_kept_as_text.saturating_add(1);
            Cell { value: CellValue::Text(pzfx_element_to_markup(d_el)), excluded }
        }
        PzfxTokenClass::PlainText => Cell { value: CellValue::Text(pzfx_element_to_markup(d_el)), excluded },
    }
}

fn extract_subcolumn_cells_with_stats(sub_el: &XmlElement, opts: CellParseOptions, ctx: ColumnParseContext) -> (Vec<Cell>, SubcolumnNumericStats) {
    let sub_opts = CellParseOptions { column_excluded: opts.column_excluded || element_is_excluded(sub_el), ..opts };
    let mut stats = SubcolumnNumericStats::default();
    let cells = sub_el.children_named("d").map(|d_el| parse_pzfx_cell_with_context(d_el, sub_opts, ctx, &mut stats)).collect();
    (cells, stats)
}

fn extract_raw_subcolumn_strings(col_el: &XmlElement) -> Vec<Vec<String>> {
    let subs: Vec<Vec<String>> =
        col_el.children_named("Subcolumn").map(|sub_el| sub_el.children_named("d").map(pzfx_element_to_markup).collect()).collect();
    if !subs.is_empty() {
        return subs;
    }
    let direct: Vec<String> = col_el.children_named("d").map(pzfx_element_to_markup).collect();
    if direct.is_empty() { Vec::new() } else { vec![direct] }
}

fn build_column_parse_context(col_el: &XmlElement, scope: NumericParseScope) -> ColumnParseContext {
    ColumnParseContext {
        col_ev: collect_column_decimal_evidence(col_el),
        table_ev: scope.table_ev,
        doc_ev: scope.doc_ev,
        col_decimals: parse_decimals_attr(col_el),
        explicit_decimal_comma: scope.explicit_decimal_comma,
    }
}

fn extract_labeled_subcolumns(
    col_el: &XmlElement,
    source_id: &str,
    warnings: &mut Vec<ImportWarning>,
    opts: CellParseOptions,
    scope: NumericParseScope,
) -> (Vec<Option<String>>, Vec<Vec<Cell>>, Vec<SubcolumnNumericStats>) {
    let ctx = build_column_parse_context(col_el, scope);
    let mut titles = Vec::new();
    let mut subcolumns = Vec::new();
    let mut sub_stats = Vec::new();
    for sub_el in col_el.children_named("Subcolumn") {
        let sub_title = extract_element_title(sub_el).map(|raw| clamp_title(&raw, source_id, warnings));
        titles.push(sub_title);
        let (cells, stats) = extract_subcolumn_cells_with_stats(sub_el, opts, ctx);
        subcolumns.push(cells);
        sub_stats.push(stats);
    }
    if subcolumns.is_empty() {
        let mut stats = SubcolumnNumericStats::default();
        let direct: Vec<Cell> = col_el.children_named("d").map(|d_el| parse_pzfx_cell_with_context(d_el, opts, ctx, &mut stats)).collect();
        if !direct.is_empty() {
            titles.push(None);
            subcolumns.push(direct);
            sub_stats.push(stats);
        }
    }
    (titles, subcolumns, sub_stats)
}

#[cfg(test)]
fn extract_column_subcolumns(col_el: &XmlElement) -> Vec<Vec<Cell>> {
    let opts = CellParseOptions { column_excluded: element_is_excluded(col_el), ..CellParseOptions::default() };
    let ctx = build_column_parse_context(col_el, NumericParseScope::default());
    let subs: Vec<Vec<Cell>> = col_el.children_named("Subcolumn").map(|sub_el| extract_subcolumn_cells_with_stats(sub_el, opts, ctx).0).collect();
    if !subs.is_empty() {
        return subs;
    }
    let mut stats = SubcolumnNumericStats::default();
    let direct: Vec<Cell> = col_el.children_named("d").map(|d_el| parse_pzfx_cell_with_context(d_el, opts, ctx, &mut stats)).collect();
    if direct.is_empty() { Vec::new() } else { vec![direct] }
}

fn allowed_y_subcolumns_for_kind(kind: TableKind, subcolumn_mode: SubcolumnMode) -> usize {
    match kind {
        TableKind::Nested => usize::MAX,
        TableKind::Contingency | TableKind::Survival | TableKind::PartsOfWhole | TableKind::MultipleVariables => 1,
        TableKind::Xy | TableKind::Column | TableKind::Grouped => subcolumn_mode.subcolumn_count(),
    }
}

struct TableNumericStatsView<'a> {
    kind: TableKind,
    x_mode: &'a XColumnMode,
    subcolumn_mode: SubcolumnMode,
    x_sub_stats: &'a [SubcolumnNumericStats],
    parsed_y: &'a [ParsedYColumn],
}

fn emit_table_numeric_cell_warnings(warnings: &mut Vec<ImportWarning>, table: (&str, &str), view: &TableNumericStatsView<'_>) {
    let (table_id, table_title) = table;
    let mut total = SubcolumnNumericStats::default();
    let retained_x_subs = match view.kind {
        TableKind::Xy if matches!(view.x_mode, XColumnMode::Numbers { .. }) => view.x_mode.subcolumn_count(),
        TableKind::Survival => 1,
        _ => 0,
    };
    for stat in view.x_sub_stats.iter().take(retained_x_subs) {
        total.merge(*stat);
    }
    let retained_y_subs = allowed_y_subcolumns_for_kind(view.kind, view.subcolumn_mode);
    for col in view.parsed_y {
        for stat in col.sub_stats.iter().take(retained_y_subs) {
            total.merge(*stat);
        }
    }
    if total.numeric_looking_kept_as_text > 0 {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            table_id,
            format!(
                "table `{table_id}` (`{table_title}`) kept {} numeric-looking cell(s) as text because decimal or thousands separators were malformed, overflowing, or conflicted within the sheet",
                total.numeric_looking_kept_as_text
            ),
        ));
    }
    if total.ambiguous_via_decimals_attr > 0 {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            table_id,
            format!(
                "table `{table_id}` (`{table_title}`) interpreted {} `ddd,ddd` cell(s) as decimal-comma numbers using column `Decimals=\"3\"` metadata",
                total.ambiguous_via_decimals_attr
            ),
        ));
    }
    if total.ambiguous_zero_context_thousands > 0 {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            table_id,
            format!(
                "table `{table_id}` (`{table_title}`) interpreted {} ambiguous `ddd,ddd` cell(s) as thousands-grouped integers",
                total.ambiguous_zero_context_thousands
            ),
        ));
    }
}

fn extract_element_title(el: &XmlElement) -> Option<String> {
    el.first_child("Title")
        .map(pzfx_element_to_markup)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| el.attr("Title").map(str::to_owned).filter(|s| !s.trim().is_empty()))
}

#[derive(Debug, Clone, Copy)]
struct ParsedReplicatesAttr<'a> {
    count: usize,
    explicit: bool,
    malformed: Option<&'a str>,
}

fn parse_replicates_attr(raw: Option<&str>) -> ParsedReplicatesAttr<'_> {
    let Some(val) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return ParsedReplicatesAttr { count: 1, explicit: false, malformed: None };
    };
    match val.parse::<usize>() {
        Ok(n) if n >= 1 => ParsedReplicatesAttr { count: n, explicit: true, malformed: None },
        _ => ParsedReplicatesAttr { count: 1, explicit: false, malformed: Some(val) },
    }
}

fn detect_decimal_separator_attr(el: &XmlElement) -> Option<bool> {
    match el.attr("DecimalSeparator").map(str::trim) {
        Some(",") | Some("comma") => Some(true),
        Some(".") | Some("dot") | Some("period") => Some(false),
        _ => None,
    }
}

fn classify_limit_sign_hint(title: Option<&str>) -> Option<bool> {
    let raw = title?.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    let is_plus = raw.contains('+') || raw.contains("plus") || raw.contains("upper") || raw.contains("high") || raw.contains("max");
    let is_minus =
        raw.contains('-') || raw.contains('\u{2212}') || raw.contains("minus") || raw.contains("lower") || raw.contains("low") || raw.contains("min");
    match (is_plus, is_minus) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct ErrorSubcolumnMeta {
    mode: SubcolumnMode,
    is_plus_minus: bool,
    is_no_n: bool,
}

fn normalize_error_subcolumns(
    subcolumns: &mut Vec<Vec<Cell>>,
    sub_titles: &[Option<String>],
    meta: ErrorSubcolumnMeta,
    location: (&str, &str),
    warnings: &mut Vec<ImportWarning>,
) {
    let (table_id, col_label) = location;
    if !meta.mode.is_pre_summarized() {
        return;
    }
    let non_empty_count = subcolumns.len();
    if meta.is_plus_minus && non_empty_count == 2 {
        if let Some(err_col) = subcolumns.get(1).cloned() {
            subcolumns.push(err_col);
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                table_id,
                format!(
                    "table `{table_id}` column `{col_label}` used 2-subcolumn PlusMinus format; duplicated ±Error into both -Error and +Error slots"
                ),
            ));
        }
        return;
    }
    if non_empty_count == 2 && meta.mode.subcolumn_count() == 3 {
        if !meta.is_no_n {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                table_id,
                format!(
                    "table `{table_id}` column `{col_label}` has 2 summary subcolumns for mode {:?}; third subcolumn (N/limit) left empty",
                    meta.mode
                ),
            ));
        }
        return;
    }
    if matches!(meta.mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) && subcolumns.len() >= 3 {
        let first_hint = sub_titles.get(1).and_then(|o| classify_limit_sign_hint(o.as_deref()));
        let second_hint = sub_titles.get(2).and_then(|o| classify_limit_sign_hint(o.as_deref()));
        if first_hint == Some(true) && second_hint == Some(false) {
            subcolumns.swap(1, 2);
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                table_id,
                format!("reordered asymmetric error subcolumns in table `{table_id}` column `{col_label}` to place Lower/-Error before Upper/+Error"),
            ));
        }
    }
}

fn warn_domain_and_non_numeric_anomalies(
    table_id: &str,
    kind: TableKind,
    x_cols: &[Vec<Cell>],
    parsed_y: &[ParsedYColumn],
    warnings: &mut Vec<ImportWarning>,
) {
    match kind {
        TableKind::Contingency => {
            for col in parsed_y {
                if let Some(sub) = col.subcolumns.first() {
                    let invalid = sub.iter().any(|c| match &c.value {
                        CellValue::Number(v) => *v < 0.0 || v.fract().abs() > 1e-9,
                        CellValue::Text(_) => true,
                        CellValue::Empty => false,
                    });
                    if invalid {
                        warnings.push(ImportWarning::new(
                            ImportWarningKind::CoercedValue,
                            table_id,
                            format!(
                                "contingency table `{table_id}` column `{}` contains negative, fractional, or non-numeric entries preserved verbatim",
                                col.title
                            ),
                        ));
                    }
                }
            }
        }
        TableKind::Survival => {
            if let Some(times) = x_cols.first()
                && times.iter().any(|c| match &c.value {
                    CellValue::Number(t) => *t < 0.0,
                    CellValue::Text(_) => true,
                    CellValue::Empty => false,
                })
            {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::CoercedValue,
                    table_id,
                    format!("survival table `{table_id}` time column contains negative or non-numeric values preserved verbatim"),
                ));
            }
            for col in parsed_y {
                if let Some(sub) = col.subcolumns.first()
                    && sub.iter().any(|c| match &c.value {
                        CellValue::Number(s) => (*s - 0.0).abs() > 1e-9 && (*s - 1.0).abs() > 1e-9,
                        CellValue::Text(_) => true,
                        CellValue::Empty => false,
                    })
                {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        table_id,
                        format!("survival table `{table_id}` cohort `{}` contains status codes other than 0/1 preserved verbatim", col.title),
                    ));
                }
            }
        }
        TableKind::PartsOfWhole => {
            for col in parsed_y {
                if let Some(sub) = col.subcolumns.first()
                    && sub.iter().any(|c| matches!(c.value, CellValue::Number(v) if v < 0.0) || matches!(c.value, CellValue::Text(_)))
                {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        table_id,
                        format!(
                            "parts-of-whole table `{table_id}` column `{}` contains negative or non-numeric entries preserved verbatim",
                            col.title
                        ),
                    ));
                }
            }
        }
        TableKind::Xy | TableKind::Column | TableKind::Grouped | TableKind::Nested => {
            let x_text = x_cols.iter().flat_map(|col| col.iter()).any(|c| matches!(c.value, CellValue::Text(_)));
            let y_text =
                parsed_y.iter().flat_map(|col| col.subcolumns.iter().flat_map(|sub| sub.iter())).any(|c| matches!(c.value, CellValue::Text(_)));
            if x_text || y_text {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::CoercedValue,
                    table_id,
                    format!("numeric table `{table_id}` contains non-numeric, non-finite, or placeholder cell entries preserved verbatim as text"),
                ));
            }
        }
        TableKind::MultipleVariables => {}
    }
}

fn resolve_table_kind(raw: Option<&str>, ext_raw: Option<&str>, is_huge: bool, id: &str, warnings: &mut Vec<ImportWarning>) -> TableKind {
    if let Some(ext) = ext_raw.map(str::trim).filter(|s| !s.is_empty()) {
        if ext.eq_ignore_ascii_case("nested") {
            return TableKind::Nested;
        }
        if ext.eq_ignore_ascii_case("multiplevariables") || ext.eq_ignore_ascii_case("multiple-variables") {
            return TableKind::MultipleVariables;
        }
    }
    let trimmed = raw.map(str::trim).unwrap_or("");
    if is_huge && trimmed.is_empty() {
        warnings.push(ImportWarning::new(
            ImportWarningKind::UnsupportedTableType,
            id,
            format!("`<HugeTable>` `{id}` imported as MultipleVariables table"),
        ));
        return TableKind::MultipleVariables;
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "xy" => TableKind::Xy,
        "oneway" | "column" | "" => TableKind::Column,
        "twoway" | "grouped" => TableKind::Grouped,
        "contingency" => TableKind::Contingency,
        "survival" => TableKind::Survival,
        "partsofwhole" | "parts-of-whole" => TableKind::PartsOfWhole,
        "multiplevariables" | "multiple-variables" | "huge" => {
            if trimmed.eq_ignore_ascii_case("huge") {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::UnsupportedTableType,
                    id,
                    format!("TableType `Huge` on `{id}` mapped to MultipleVariables"),
                ));
            }
            TableKind::MultipleVariables
        }
        "nested" => TableKind::Nested,
        _ => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedTableType,
                id,
                format!("unrecognized TableType `{trimmed}`, defaulting to Column"),
            ));
            TableKind::Column
        }
    }
}

struct YSubcolumnModeResolution {
    mode: SubcolumnMode,
    is_plus_minus: bool,
    is_no_n: bool,
}

const fn single_subcolumn_mode_for_kind(kind: TableKind) -> SubcolumnMode {
    if matches!(kind, TableKind::Grouped | TableKind::Nested) { SubcolumnMode::RawReplicates(1) } else { SubcolumnMode::Single }
}

fn warn_declared_y_contradictions(parsed_y: &[ParsedYColumn], expected: usize, attr_desc: &str, id: &str, warnings: &mut Vec<ImportWarning>) {
    for col in parsed_y {
        if let Some(decl) = col.declared_subcolumns
            && decl != expected
        {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                id,
                format!(
                    "table `{id}` column `{}` declares Subcolumns=\"{decl}\" which contradicts {attr_desc} (expected {expected} subcolumns)",
                    col.title
                ),
            ));
        }
    }
}

fn resolve_summary_y_mode(
    kind: TableKind,
    fmt: &str,
    info: SummaryFormatInfo,
    reps: ParsedReplicatesAttr<'_>,
    parsed_y: &[ParsedYColumn],
    id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<YSubcolumnModeResolution, FormatError> {
    let expected = info.expected_subcolumns();
    warn_declared_y_contradictions(parsed_y, expected, &format!("YFormat=\"{fmt}\""), id, warnings);

    let any_declared = parsed_y.iter().any(|c| c.declared_subcolumns.is_some());
    let all_declared = !parsed_y.is_empty() && parsed_y.iter().all(|c| c.declared_subcolumns.is_some());
    let max_declared = parsed_y.iter().filter_map(|c| c.declared_subcolumns).max();
    let max_actual = parsed_y.iter().map(|c| c.subcolumns.len()).max().unwrap_or(0);
    let effective = max_declared.unwrap_or(max_actual);

    if all_declared && max_declared == Some(1) && max_actual <= 1 {
        let mode = single_subcolumn_mode_for_kind(kind);
        mode.validate()?;
        return Ok(YSubcolumnModeResolution { mode, is_plus_minus: false, is_no_n: false });
    }

    match info {
        SummaryFormatInfo::NativeTwoSubcolumn { two_sub_mode, three_sub_mode } => {
            let target_mode = if effective >= 3 {
                if !any_declared {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        id,
                        format!("table `{id}` has {max_actual} subcolumns contradicting 2-subcolumn YFormat=\"{fmt}\"; stored as {three_sub_mode:?}"),
                    ));
                }
                three_sub_mode
            } else {
                two_sub_mode
            };
            if !kind.allows_subcolumn_mode(target_mode) {
                return Ok(coerce_disallowed_subcolumn_mode(kind, target_mode, reps.count, id, warnings));
            }
            Ok(YSubcolumnModeResolution { mode: target_mode, is_plus_minus: false, is_no_n: false })
        }
        SummaryFormatInfo::PaddedNoNCv => {
            if !kind.allows_subcolumn_mode(SubcolumnMode::MeanCvN) {
                return Ok(coerce_disallowed_subcolumn_mode(kind, SubcolumnMode::MeanCvN, reps.count, id, warnings));
            }
            if !any_declared && max_actual >= 3 {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::CoercedValue,
                    id,
                    format!("table `{id}` has {max_actual} subcolumns contradicting no-N YFormat=\"{fmt}\" (expected 2); stored as MeanCvN"),
                ));
            }
            if effective <= 2 {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    id,
                    format!("table `{id}` uses no-N `%CV` summary YFormat=\"{fmt}\" (2 subcolumns without N); stored in MeanCvN with empty N slot"),
                ));
                return Ok(YSubcolumnModeResolution { mode: SubcolumnMode::MeanCvN, is_plus_minus: false, is_no_n: true });
            }
            Ok(YSubcolumnModeResolution { mode: SubcolumnMode::MeanCvN, is_plus_minus: false, is_no_n: false })
        }
        SummaryFormatInfo::PlusMinus => {
            if !kind.allows_subcolumn_mode(SubcolumnMode::MeanLowHigh) {
                return Ok(coerce_disallowed_subcolumn_mode(kind, SubcolumnMode::MeanLowHigh, reps.count, id, warnings));
            }
            if effective == 3 {
                if !any_declared {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        id,
                        format!("table `{id}` has 3 subcolumns contradicting 2-subcolumn YFormat=\"{fmt}\"; stored as MeanLowHigh"),
                    ));
                }
                return Ok(YSubcolumnModeResolution { mode: SubcolumnMode::MeanLowHigh, is_plus_minus: false, is_no_n: false });
            }
            Ok(YSubcolumnModeResolution { mode: SubcolumnMode::MeanLowHigh, is_plus_minus: true, is_no_n: false })
        }
        SummaryFormatInfo::ThreeSubcolumn(base_mode) => {
            if !kind.allows_subcolumn_mode(base_mode) {
                return Ok(coerce_disallowed_subcolumn_mode(kind, base_mode, reps.count, id, warnings));
            }
            if max_declared == Some(2) && max_actual < 2 {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    id,
                    format!("table `{id}` declares 2 subcolumns for 3-subcolumn mode {base_mode:?}; third subcolumn (N/limit) left empty"),
                ));
            }
            Ok(YSubcolumnModeResolution { mode: base_mode, is_plus_minus: false, is_no_n: false })
        }
    }
}

fn coerce_disallowed_subcolumn_mode(
    kind: TableKind,
    candidate: SubcolumnMode,
    replicates: usize,
    id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> YSubcolumnModeResolution {
    if matches!(kind, TableKind::Grouped | TableKind::Nested) {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            id,
            format!("subcolumn mode {candidate:?} is not valid for {kind:?}, using RawReplicates"),
        ));
        YSubcolumnModeResolution { mode: SubcolumnMode::RawReplicates(replicates.max(1)), is_plus_minus: false, is_no_n: false }
    } else {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            id,
            format!("subcolumn mode {candidate:?} is not valid for {kind:?}, using Single"),
        ));
        YSubcolumnModeResolution { mode: SubcolumnMode::Single, is_plus_minus: false, is_no_n: false }
    }
}

fn resolve_subcolumn_mode(
    kind: TableKind,
    raw_fmt: Option<&str>,
    reps: ParsedReplicatesAttr<'_>,
    parsed_y: &[ParsedYColumn],
    id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<YSubcolumnModeResolution, FormatError> {
    let fmt = raw_fmt.map(str::trim).unwrap_or("");
    let fmt_lower = fmt.to_ascii_lowercase();
    if let Some(info) = classify_summary_format(&fmt_lower) {
        return resolve_summary_y_mode(kind, fmt, info, reps, parsed_y, id, warnings);
    }

    let candidate = match fmt_lower.as_str() {
        "replicates" | "raw" | "raw-replicates" => {
            if reps.explicit {
                warn_declared_y_contradictions(parsed_y, reps.count, &format!("Replicates=\"{}\" on YFormat=\"{fmt}\"", reps.count), id, warnings);
            }
            let all_declared = !parsed_y.is_empty() && parsed_y.iter().all(|c| c.declared_subcolumns.is_some());
            let max_declared = parsed_y.iter().filter_map(|c| c.declared_subcolumns).max();
            let max_actual = parsed_y.iter().map(|c| c.subcolumns.len()).max().unwrap_or(0);
            let target_count = if all_declared && let Some(decl_max) = max_declared { decl_max.max(max_actual).max(1) } else { reps.count.max(1) };
            SubcolumnMode::RawReplicates(target_count)
        }
        "" | "none" | "single" => {
            if !fmt.is_empty() && matches!(kind, TableKind::Xy | TableKind::Grouped) {
                warn_declared_y_contradictions(parsed_y, 1, &format!("YFormat=\"{fmt}\""), id, warnings);
            }
            if matches!(kind, TableKind::Grouped | TableKind::Nested) {
                SubcolumnMode::RawReplicates(reps.count.max(1))
            } else if kind == TableKind::Xy && reps.count > 1 {
                SubcolumnMode::RawReplicates(reps.count)
            } else {
                if reps.count > 1 {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        id,
                        format!("Replicates=\"{}\" is not valid for {kind:?} without a summary YFormat, using Single", reps.count),
                    ));
                }
                SubcolumnMode::Single
            }
        }
        _ => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                id,
                format!("unrecognized YFormat `{fmt}`, falling back to default subcolumn mode"),
            ));
            if matches!(kind, TableKind::Grouped | TableKind::Nested) {
                SubcolumnMode::RawReplicates(reps.count.max(1))
            } else {
                SubcolumnMode::Single
            }
        }
    };

    if kind.allows_subcolumn_mode(candidate) {
        candidate.validate()?;
        Ok(YSubcolumnModeResolution { mode: candidate, is_plus_minus: false, is_no_n: false })
    } else {
        Ok(coerce_disallowed_subcolumn_mode(kind, candidate, reps.count, id, warnings))
    }
}

fn warn_ignored_sections(root: &XmlElement, warnings: &mut Vec<ImportWarning>) {
    for tag in ["AnalysisSequence", "GraphSequence", "LayoutSequence", "ScriptSequence"] {
        if let Some(el) = root.first_child(tag)
            && el.child_elements().next().is_some()
        {
            warnings.push(ImportWarning::new(
                ImportWarningKind::IgnoredSection,
                tag,
                format!("section `<{tag}>` is not imported automatically; data tables and info constants were preserved"),
            ));
        }
    }
    for (tag, kind, desc) in [
        ("EquationSequence", ImportWarningKind::UnsupportedEquation, "custom equations"),
        ("UserEquations", ImportWarningKind::UnsupportedEquation, "user-defined equations"),
        ("Equations", ImportWarningKind::UnsupportedEquation, "equations"),
        ("Equation", ImportWarningKind::UnsupportedEquation, "equation definitions"),
        ("ResultsSequence", ImportWarningKind::UnsupportedAnalysis, "cached analysis results"),
        ("AnalysisResults", ImportWarningKind::UnsupportedAnalysis, "cached analysis results"),
        ("Analysis", ImportWarningKind::UnsupportedAnalysis, "top-level analysis definitions"),
        ("Result", ImportWarningKind::UnsupportedAnalysis, "top-level result definitions"),
        ("Graph", ImportWarningKind::UnsupportedGraphOrLayout, "top-level graph definitions"),
        ("Layout", ImportWarningKind::UnsupportedGraphOrLayout, "top-level layout definitions"),
        ("Script", ImportWarningKind::UnsupportedScript, "top-level embedded scripts"),
    ] {
        for el in root.children_named(tag) {
            if el.child_elements().next().is_some() || !el.plain_text().trim().is_empty() || !el.attrs.is_empty() {
                warnings.push(ImportWarning::new(
                    kind,
                    tag,
                    format!("section `<{tag}>` ({desc}) is not imported automatically; data tables and info constants were preserved"),
                ));
                break;
            }
        }
    }
}

const SHEET_ID_SANITIZE_HEADROOM_BYTES: usize = 24;

pub(crate) fn allocate_valid_unique_sheet_id(
    raw: Option<&str>,
    prefix: &str,
    idx: usize,
    used: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> SheetId {
    let fallback = || format!("{prefix}-{idx}");
    let base = match raw {
        None | Some("") => fallback(),
        Some(raw_id) if SheetId::try_new(raw_id).is_ok() => raw_id.to_owned(),
        Some(raw_id) => {
            let sanitized = sanitize_sheet_id_candidate(raw_id, &fallback());
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                raw_id,
                format!("imported sheet ID `{raw_id}` is invalid and was sanitized to `{sanitized}`"),
            ));
            sanitized
        }
    };
    let unique = reserve_unique_sheet_id(&base, used);
    SheetId::try_new(&unique).unwrap_or_else(|_| SheetId::new(reserve_unique_sheet_id(&fallback(), used)))
}

fn sanitize_sheet_id_candidate(raw: &str, fallback: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    let mut last_dash = false;
    for ch in raw.chars() {
        if ch.is_whitespace() || ch.is_control() || matches!(ch, ':' | '{' | '}') {
            if !last_dash && !cleaned.is_empty() {
                cleaned.push('-');
                last_dash = true;
            }
        } else {
            cleaned.push(ch);
            last_dash = ch == '-';
        }
    }
    let trimmed = cleaned.trim_matches('-');
    let max_base_bytes = MAX_SHEET_ID_BYTES.saturating_sub(SHEET_ID_SANITIZE_HEADROOM_BYTES);
    let truncated = truncate_utf8_to_bytes(trimmed, max_base_bytes).trim_end_matches('-');
    if truncated.is_empty() || SheetId::try_new(truncated).is_err() {
        truncate_utf8_to_bytes(fallback, max_base_bytes).to_owned()
    } else {
        truncated.to_owned()
    }
}

fn reserve_unique_sheet_id(base: &str, used: &mut HashSet<String>) -> String {
    let initial = truncate_utf8_to_bytes(base, MAX_SHEET_ID_BYTES).to_owned();
    if used.insert(initial.clone()) {
        return initial;
    }
    let mut suffix = 1_usize;
    loop {
        let tag = format!("-{suffix}");
        let budget = MAX_SHEET_ID_BYTES.saturating_sub(tag.len());
        let stem = truncate_utf8_to_bytes(base, budget);
        let candidate = format!("{stem}{tag}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn truncate_utf8_to_bytes(raw: &str, max_bytes: usize) -> &str {
    if raw.len() <= max_bytes {
        return raw;
    }
    let mut end = max_bytes;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw.get(..end).unwrap_or("")
}

pub(crate) fn clamp_title(raw: &str, source: &str, warnings: &mut Vec<ImportWarning>) -> String {
    if raw.len() <= MAX_TITLE_BYTES {
        return raw.to_owned();
    }
    warnings.push(ImportWarning::new(ImportWarningKind::TruncatedTitle, source, format!("title exceeded {MAX_TITLE_BYTES} bytes and was truncated")));
    truncate_utf8_to_bytes(raw, MAX_TITLE_BYTES).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::parse_xml;

    #[test]
    fn estimate_table_cells_matches_allocated_cell_slots_across_all_archetypes() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="xy-sd" TableType="XY" XEVFormat="SD" YFormat="replicates" Replicates="2">
      <XColumn><Subcolumn><d>1</d><d>2</d><d>3</d></Subcolumn><Subcolumn><d>0.1</d></Subcolumn><Subcolumn><d>3</d></Subcolumn></XColumn>
      <YColumn><Subcolumn><d>10</d></Subcolumn><Subcolumn><d>11</d></Subcolumn></YColumn>
      <YColumn><Subcolumn><d>20</d></Subcolumn><Subcolumn><d>21</d></Subcolumn></YColumn>
    </Table>
    <Table ID="xy-seq" TableType="XY" XFormat="sequence" X0="0" DeltaX="5">
      <RowTitlesColumn><Subcolumn><d>r1</d><d>r2</d><d>r3</d><d>r4</d></Subcolumn></RowTitlesColumn>
      <YColumn><Subcolumn><d>1</d><d>2</d></Subcolumn></YColumn>
    </Table>
    <Table ID="surv" TableType="Survival">
      <XColumn><Subcolumn><d>5</d><d>10</d><d>15</d></Subcolumn></XColumn>
      <YColumn><Subcolumn><d>1</d><d>0</d><d>1</d></Subcolumn></YColumn>
      <YColumn><Subcolumn><d>0</d><d>1</d><d>0</d></Subcolumn></YColumn>
    </Table>
    <Table ID="col" TableType="OneWay" YFormat="SD">
      <YColumn><Subcolumn><d>1</d></Subcolumn><Subcolumn><d>0.2</d></Subcolumn><Subcolumn><d>5</d></Subcolumn></YColumn>
    </Table>
    <Table ID="grp" TableType="TwoWay" Replicates="3">
      <YColumn><Subcolumn><d>1</d><d>2</d></Subcolumn><Subcolumn><d>3</d><d>4</d></Subcolumn><Subcolumn><d>5</d><d>6</d></Subcolumn></YColumn>
    </Table>
    <Table ID="cont" TableType="Contingency">
      <YColumn><Subcolumn><d>10</d><d>20</d></Subcolumn></YColumn>
      <YColumn><Subcolumn><d>5</d><d>15</d></Subcolumn></YColumn>
    </Table>
    <Table ID="pow" TableType="PartsOfWhole">
      <YColumn><Subcolumn><d>25</d><d>75</d></Subcolumn></YColumn>
    </Table>
    <Table ID="mv" TableType="MultipleVariables">
      <YColumn><Subcolumn><d>A</d><d>B</d><d>C</d></Subcolumn></YColumn>
      <YColumn><Subcolumn><d>1</d><d>2</d><d>3</d></Subcolumn></YColumn>
    </Table>
    <Table ID="nest" TableType="Nested">
      <YColumn><Subcolumn><d>1</d><d>2</d></Subcolumn><Subcolumn><d>3</d><d>4</d></Subcolumn></YColumn>
      <YColumn><Subcolumn><d>5</d><d>6</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;

        let root = parse_xml(xml).unwrap();
        let seq = root.first_child("TableSequence").unwrap();
        let mut total_slots = 0_usize;
        for table_el in seq.children_named("Table") {
            let kind = resolve_table_kind(table_el.attr("TableType"), table_el.attr("ExtTableType"), false, "t", &mut Vec::new());
            let reps = parse_replicates_attr(table_el.attr("Replicates"));
            let row_titles = table_el
                .first_child("RowTitlesColumn")
                .map(|el| extract_column_subcolumns(el).into_iter().next().unwrap_or_default())
                .unwrap_or_default();
            let x_cols = table_el.first_child("XColumn").map(extract_column_subcolumns).unwrap_or_default();
            let x_res = resolve_x_mode(
                kind,
                table_el,
                table_el.first_child("XColumn"),
                None,
                &x_cols,
                None,
                NumericParseScope::default(),
                &mut Vec::new(),
                "t",
            )
            .unwrap();
            let parsed_x = ParsedXColumn {
                source_el: table_el.first_child("XColumn"),
                title: String::from("X"),
                mode: x_res.mode,
                display_decimals: None,
                declared_subs: None,
                sub_titles: Vec::new(),
                cols: x_cols,
                sub_stats: Vec::new(),
                is_text_x: x_res.is_text_x,
            };
            let parsed_y: Vec<ParsedYColumn> = table_el
                .children_named("YColumn")
                .enumerate()
                .map(|(i, y_el)| ParsedYColumn {
                    title: default_column_label(i),
                    declared_subcolumns: None,
                    display_decimals: None,
                    mv_type: None,
                    sub_titles: Vec::new(),
                    subcolumns: extract_column_subcolumns(y_el),
                    sub_stats: Vec::new(),
                })
                .collect();
            let y_res = resolve_subcolumn_mode(kind, table_el.attr("YFormat"), reps, &parsed_y, "t", &mut Vec::new()).unwrap();
            let mode = y_res.mode;
            let max_y = parsed_y.iter().flat_map(|c| c.subcolumns.iter().map(Vec::len)).max().unwrap_or(0);
            let max_x = parsed_x.cols.iter().map(Vec::len).max().unwrap_or(0);
            let rows = row_titles.len().max(max_x).max(max_y);
            let est = estimate_table_cells(kind, mode, &parsed_x.mode, &parsed_y, rows).unwrap();
            let arch = build_table_archetype(kind, mode, &parsed_x, &parsed_y, vec![String::new(); rows], false, None, rows).unwrap();
            assert_eq!(est, arch.allocated_cell_slots(), "estimate mismatch for {kind:?}");
            total_slots += arch.allocated_cell_slots();
        }
        // xy-sd: 3*(3+4)=21, xy-seq: 4*(0+1)=4, surv: 3*(1+2)=9, col: 1*3=3, grp: 2*3=6, cont: 2*2=4, pow: 2*1=2, mv: 3*2=6, nest: 2*(2+1)=6
        assert_eq!(total_slots, 61);
    }

    #[test]
    fn estimate_table_cells_rejects_usize_overflow_cleanly() {
        let parsed_y: Vec<ParsedYColumn> = (0..1_024)
            .map(|_| ParsedYColumn {
                title: String::from("A"),
                declared_subcolumns: None,
                display_decimals: None,
                mv_type: None,
                sub_titles: Vec::new(),
                subcolumns: vec![vec![]],
                sub_stats: Vec::new(),
            })
            .collect();
        let x_mode = XColumnMode::default();
        let err = estimate_table_cells(TableKind::Xy, SubcolumnMode::RawReplicates(512), &x_mode, &parsed_y, usize::MAX).unwrap_err();
        assert!(matches!(err, FormatError::XmlParse { offset: 0, .. }));

        let surv_err = estimate_table_cells(TableKind::Survival, SubcolumnMode::Single, &x_mode, &parsed_y, usize::MAX).unwrap_err();
        assert!(matches!(surv_err, FormatError::XmlParse { offset: 0, .. }));
    }

    #[test]
    fn r1_s06_04_widens_compatible_tables_and_warns_when_subcolumns_discarded() {
        let oneway_xml = r#"<?xml version="1.0"?><PrismFile><TableSequence><Table ID="t-col" TableType="OneWay"><Title>Treatment</Title>
<YColumn Subcolumns="2"><Title>DoseA</Title><Subcolumn><d>1</d><d>2</d></Subcolumn><Subcolumn><d>30</d><d>40</d></Subcolumn></YColumn></Table></TableSequence></PrismFile>"#;
        let rep = import_pzfx_str(oneway_xml).unwrap();
        assert!(
            rep.warnings.iter().any(|w| {
                w.kind == ImportWarningKind::CoercedValue
                    && w.source == "t-col"
                    && w.message.contains("Treatment")
                    && w.message.contains("DoseA")
                    && w.message.contains("discarded 1 subcolumn")
            }),
            "unexpected warnings: {:?}",
            rep.warnings
        );

        let xy_xml = r#"<?xml version="1.0"?><PrismFile><TableSequence><Table ID="t-xy" TableType="XY"><Title>Curve</Title>
<XColumn><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>
<YColumn Subcolumns="3"><Title>Resp</Title><Subcolumn><d>10</d><d>20</d></Subcolumn><Subcolumn><d>11</d><d>21</d></Subcolumn><Subcolumn><d>12</d><d>22</d></Subcolumn></YColumn></Table></TableSequence></PrismFile>"#;
        let xy_rep = import_pzfx_str(xy_xml).unwrap();
        assert!(xy_rep.warnings.is_empty(), "expected lossless widening without warning, got {:?}", xy_rep.warnings);
        let TableArchetype::Xy(xy) = &xy_rep.document.data_sheets[0].archetype else { panic!("expected XY") };
        assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(3));
        assert_eq!(xy.data_sets[0].cell(1, 2).unwrap().active_number(), Some(22.0));

        let grp_xml = r#"<?xml version="1.0"?><PrismFile><TableSequence><Table ID="t-grp" TableType="TwoWay"><Title>Grouped</Title>
<YColumn Subcolumns="2"><Title>Drug</Title><Subcolumn><d>5</d></Subcolumn><Subcolumn><d>6</d></Subcolumn></YColumn></Table></TableSequence></PrismFile>"#;
        let grp_rep = import_pzfx_str(grp_xml).unwrap();
        assert!(grp_rep.warnings.is_empty(), "expected lossless widening without warning, got {:?}", grp_rep.warnings);
        let TableArchetype::Grouped(grp) = &grp_rep.document.data_sheets[0].archetype else { panic!("expected Grouped") };
        assert_eq!(grp.subcolumn_mode, SubcolumnMode::RawReplicates(2));
        assert_eq!(grp.data_sets[0].cell(0, 1).unwrap().active_number(), Some(6.0));

        let multi_xml = r#"<?xml version="1.0"?><PrismFile><TableSequence>
<Table ID="t-xy2" TableType="XY"><Title>XY-X2</Title>
<XColumn><Title>Conc</Title><Subcolumn><d>1</d></Subcolumn><Subcolumn><d>0.2</d></Subcolumn></XColumn>
<YColumn><Title>Y</Title><Subcolumn><d>5</d></Subcolumn></YColumn></Table>
<Table ID="t-mv2" TableType="MultipleVariables"><Title>MV2</Title>
<YColumn><Title>Age</Title><Subcolumn><d>42</d></Subcolumn><Subcolumn><d>43</d></Subcolumn></YColumn></Table>
<Table ID="t-surv2" TableType="Survival"><Title>Surv2</Title>
<XColumn><Title>Days</Title><Subcolumn><d>10</d></Subcolumn><Subcolumn><d>11</d></Subcolumn></XColumn>
<YColumn><Title>Arm</Title><Subcolumn><d>1</d></Subcolumn></YColumn></Table>
</TableSequence></PrismFile>"#;
        let m_rep = import_pzfx_str(multi_xml).unwrap();
        assert!(m_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "t-xy2" && w.message.contains("Conc")));
        assert!(m_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "t-mv2" && w.message.contains("Age")));
        assert!(m_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "t-surv2" && w.message.contains("Days")));

        let implicit_xml = r#"<?xml version="1.0"?><PrismFile><TableSequence>
<Table ID="t" TableType="OneWay"><Title>T</Title>
<RowTitlesColumn><Subcolumn><d>r1</d><d>r2</d></Subcolumn><Subcolumn><d>extra</d></Subcolumn></RowTitlesColumn>
<YColumn><Title>A</Title><Subcolumn><d>1</d><d>2</d></Subcolumn><Subcolumn><d>30</d><d>40</d></Subcolumn></YColumn></Table>
<Table ID="xy-imp" TableType="XY"><Title>XYImp</Title>
<XColumn><Subcolumn><d>1</d></Subcolumn></XColumn>
<YColumn><Title>B</Title><Subcolumn><d>5</d></Subcolumn><Subcolumn><d>6</d></Subcolumn></YColumn></Table>
<Table ID="xy-sd-over" TableType="XY" YFormat="SD"><Title>XYSD</Title>
<YColumn><Title>C</Title><Subcolumn><d>1</d></Subcolumn><Subcolumn><d>0.1</d></Subcolumn><Subcolumn><d>3</d></Subcolumn><Subcolumn><d>99</d></Subcolumn></YColumn></Table>
</TableSequence></PrismFile>"#;
        let imp_rep = import_pzfx_str(implicit_xml).unwrap();
        assert!(
            imp_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "t" && w.message.contains("RowTitlesColumn"))
        );
        assert!(imp_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "t" && w.message.contains("A")));
        assert!(!imp_rep.warnings.iter().any(|w| w.source == "xy-imp"), "xy-imp warnings: {:?}", imp_rep.warnings);
        let TableArchetype::Xy(xy_imp) = &imp_rep.document.data_sheets[1].archetype else { panic!("expected XY") };
        assert_eq!(xy_imp.y_subcolumn_mode, SubcolumnMode::RawReplicates(2));
        assert_eq!(xy_imp.data_sets[0].cell(0, 1).unwrap().active_number(), Some(6.0));
        assert!(imp_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::CoercedValue && w.source == "xy-sd-over" && w.message.contains("C")));

        let too_many_subs = format!(
            "<PrismFile><Table ID=\"big\" TableType=\"XY\"><YColumn Subcolumns=\"{}\"/></Table></PrismFile>",
            curvebook_doc::MAX_SUBCOLUMNS + 1
        );
        assert!(matches!(import_pzfx_str(&too_many_subs), Err(FormatError::Doc(curvebook_doc::DocError::TooManySubcolumns { .. }))));

        let mut rows_xml = String::from("<XColumn><Subcolumn>");
        for r in 0..250 {
            rows_xml.push_str(&format!("<d>{r}</d>"));
        }
        rows_xml.push_str("</Subcolumn></XColumn>");
        for c in 0..100 {
            rows_xml.push_str(&format!("<YColumn Subcolumns=\"500\"><Title>Y{c}</Title></YColumn>"));
        }
        let budget_xml = format!("<PrismFile><Table ID=\"budget-widen\" TableType=\"XY\">{rows_xml}</Table></PrismFile>");
        assert!(matches!(import_pzfx_str(&budget_xml), Err(FormatError::XmlParse { offset: 0, .. })));
    }
}
