//! Pure-Rust OOXML workbook (`.xlsx`) importer backed by `crate::zip` and `quick_xml`.
//!
//! Enforces strict ZIP/XML size/depth bounds, path-traversal sanitization, cell-allocation caps,
//! and XXE (`<!DOCTYPE>`) rejection without any third-party XLSX or calendar dependencies.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use curvebook_doc::error::MAX_SHEET_ID_BYTES;
use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, ContingencyTable, DataSetColumn, DataSheet, DocError, GroupedTable, MAX_DATA_SETS, MAX_PROJECT_CELLS,
    MAX_ROWS, MAX_SHEETS_PER_PROJECT, MAX_SUBCOLUMNS, MAX_TITLE_BYTES, MultipleVariablesTable, NestedGroupColumn, NestedTable, PartsOfWholeTable,
    ProjectDocument, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, XyTable, validate_data_sets, validate_rows,
    validate_subcolumns, validate_title,
};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::archive::{MAX_ARCHIVE_FILE_BYTES, validate_project_document};
use crate::csv::{MAX_IMPORT_CELLS, collapse_merged_header_rows};
use crate::error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
use crate::xml::{MAX_XML_BYTES, MAX_XML_DEPTH, XmlLimits, decode_xml_entities, parse_xml_with_limits};
use crate::zip::{ZipArchive, ZipLimits, sanitize_zip_path};

const DEFAULT_WORKBOOK_PATH: &str = "xl/workbook.xml";
const DEFAULT_SHARED_STRINGS_PATH: &str = "xl/sharedStrings.xml";
const MAX_SHARED_STRINGS: usize = 2_000_000;
const MAX_MERGED_RANGES_PER_SHEET: usize = 65_536;
const MAX_SHEET_ID_SUFFIX_HEADROOM: usize = 16;

/// Metadata describing one worksheet entry inside an `.xlsx` workbook.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct XlsxSheetInfo {
    pub index: usize,
    pub name: String,
    pub sheet_id: String,
    pub state: Option<String>,
    pub path: String,
}

/// Workbook sheet-selection and parsing options for `.xlsx` import.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct XlsxImportOptions {
    pub sheet_names: Option<Vec<String>>,
    pub sheet_indices: Option<Vec<usize>>,
    pub has_header: bool,
    pub header_rows: usize,
    pub has_row_titles: bool,
    pub table_kind: Option<TableKind>,
    pub subcolumn_mode: SubcolumnMode,
    pub include_hidden: bool,
    pub title: Option<String>,
}

impl Default for XlsxImportOptions {
    fn default() -> Self {
        Self {
            sheet_names: None,
            sheet_indices: None,
            has_header: true,
            header_rows: 1,
            has_row_titles: false,
            table_kind: None,
            subcolumn_mode: SubcolumnMode::Single,
            include_hidden: false,
            title: None,
        }
    }
}

#[derive(Debug, Clone)]
struct RelationshipTarget {
    rel_type: String,
    target: String,
    external: bool,
}

#[derive(Debug, Clone)]
struct WorkbookManifest {
    sheets: Vec<XlsxSheetInfo>,
    shared_strings_path: Option<String>,
    warnings: Vec<ImportWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MergeRange {
    r1: usize,
    c1: usize,
    r2: usize,
    c2: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XlsxCellType {
    Numeric,
    SharedString,
    InlineString,
    FormulaString,
    Boolean,
    Error,
    IsoDate,
}

impl XlsxCellType {
    fn from_attr(raw: Option<&str>) -> Self {
        match raw {
            Some("s") => Self::SharedString,
            Some("inlineStr") => Self::InlineString,
            Some("str") => Self::FormulaString,
            Some("b") => Self::Boolean,
            Some("e") => Self::Error,
            Some("d") => Self::IsoDate,
            _ => Self::Numeric,
        }
    }
}

#[derive(Debug, Default)]
struct ParsedWorksheet {
    cells: BTreeMap<(usize, usize), Cell>,
    max_row: Option<usize>,
    max_col: Option<usize>,
    merges: Vec<MergeRange>,
    formula_cells: usize,
    error_cells: Vec<(String, String)>,
    coerced_numeric_cells: Vec<String>,
}

/// Enumerate worksheet tabs inside raw `.xlsx` workbook bytes without materializing cell grids.
pub fn list_xlsx_sheets(bytes: &[u8]) -> Result<Vec<XlsxSheetInfo>, FormatError> {
    if bytes.len() > MAX_ARCHIVE_FILE_BYTES {
        return Err(FormatError::FileTooLarge {
            path: String::from(".xlsx"),
            actual_bytes: Some(bytes.len() as u64),
            limit_bytes: MAX_ARCHIVE_FILE_BYTES,
        });
    }
    let archive = ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default())?;
    let manifest = parse_workbook_manifest(&archive)?;
    Ok(manifest.sheets)
}

/// Import all visible worksheets from raw `.xlsx` workbook bytes using default [`XlsxImportOptions`].
pub fn import_xlsx_bytes(bytes: &[u8]) -> Result<ImportReport, FormatError> {
    import_xlsx_bytes_with_options(bytes, &XlsxImportOptions::default())
}

/// Import selected or default worksheets from raw `.xlsx` bytes with explicit [`XlsxImportOptions`].
pub fn import_xlsx_bytes_with_options(bytes: &[u8], opts: &XlsxImportOptions) -> Result<ImportReport, FormatError> {
    if bytes.len() > MAX_ARCHIVE_FILE_BYTES {
        return Err(FormatError::FileTooLarge {
            path: String::from(".xlsx"),
            actual_bytes: Some(bytes.len() as u64),
            limit_bytes: MAX_ARCHIVE_FILE_BYTES,
        });
    }
    opts.subcolumn_mode.validate()?;
    if let Some(kind) = opts.table_kind
        && !kind.allows_subcolumn_mode(opts.subcolumn_mode)
    {
        return Err(FormatError::Doc(DocError::IncompatibleSubcolumnMode { archetype: kind, mode: opts.subcolumn_mode }));
    }

    let archive = ZipArchive::from_bytes_with_limits(bytes, ZipLimits::default())?;
    let mut manifest = parse_workbook_manifest(&archive)?;
    let mut warnings = std::mem::take(&mut manifest.warnings);

    if manifest.sheets.is_empty() {
        return Err(FormatError::InvalidPipeline { reason: String::from("workbook contains no worksheets") });
    }

    let selected = select_target_sheets(&manifest.sheets, opts, &mut warnings)?;
    if selected.len() > MAX_SHEETS_PER_PROJECT {
        return Err(FormatError::Doc(DocError::TooManySheets { count: selected.len(), max: MAX_SHEETS_PER_PROJECT }));
    }

    let shared_strings = load_shared_strings(&archive, manifest.shared_strings_path.as_deref())?;
    let project_title = opts
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| clamp_title(s, "project", &mut warnings))
        .unwrap_or_else(|| String::from("Imported Workbook"));
    let mut doc = ProjectDocument::new(project_title)?;
    let mut used_ids: HashSet<String> = HashSet::new();
    let mut cumulative_cells = 0_usize;

    let effective_header_rows = if opts.has_header { opts.header_rows.max(1) } else { 0 };
    for sheet_info in selected {
        let Some(ws_bytes) = find_zip_entry(&archive, &sheet_info.path) else {
            return Err(FormatError::MissingArchiveEntry { path: sheet_info.path.clone() });
        };
        let ws_xml = decode_xml_entry(ws_bytes, &sheet_info.path)?;
        let parsed = parse_worksheet_xml(&ws_xml, &shared_strings)?;
        emit_worksheet_warnings(&sheet_info.name, &parsed, effective_header_rows, &mut warnings);

        let sheet_id = make_unique_sheet_id(&sheet_info.name, sheet_info.index, &mut used_ids);
        let sheet_title = clamp_title(if sheet_info.name.trim().is_empty() { "Sheet" } else { sheet_info.name.trim() }, "worksheet", &mut warnings);
        let ds = build_datasheet_from_worksheet(sheet_id, sheet_title, &parsed, opts, effective_header_rows)?;
        cumulative_cells = cumulative_cells
            .checked_add(ds.archetype.allocated_cell_slots())
            .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("workbook cell count overflow") })?;
        if cumulative_cells > MAX_PROJECT_CELLS {
            return Err(FormatError::Doc(DocError::TooManyCells { count: cumulative_cells, max: MAX_PROJECT_CELLS }));
        }
        doc.add_data_sheet(ds)?;
    }

    doc.active_sheet_id = doc.data_sheets.first().map(|s| s.id.clone());
    validate_project_document(&doc)?;
    Ok(ImportReport { document: doc, warnings })
}

fn find_zip_entry<'a>(archive: &'a ZipArchive, target: &str) -> Option<&'a [u8]> {
    archive.get(target).or_else(|| archive.iter().find(|(path, _)| path.eq_ignore_ascii_case(target)).map(|(_, data)| data))
}

fn decode_xml_entry(bytes: &[u8], path: &str) -> Result<String, FormatError> {
    if bytes.len() > MAX_XML_BYTES {
        return Err(FormatError::FileTooLarge { path: path.to_owned(), actual_bytes: Some(bytes.len() as u64), limit_bytes: MAX_XML_BYTES });
    }
    let text =
        std::str::from_utf8(bytes).map_err(|err| FormatError::XmlParse { offset: 0, reason: format!("entry `{path}` is not valid UTF-8: {err}") })?;
    Ok(text.strip_prefix('\u{FEFF}').unwrap_or(text).to_owned())
}

fn parse_workbook_manifest(archive: &ZipArchive) -> Result<WorkbookManifest, FormatError> {
    let mut warnings = Vec::new();
    let workbook_path = resolve_root_workbook_path(archive)?;
    let Some(wb_bytes) = find_zip_entry(archive, &workbook_path) else {
        return Err(FormatError::MissingArchiveEntry { path: workbook_path });
    };
    let wb_xml = decode_xml_entry(wb_bytes, &workbook_path)?;
    let wb_root = parse_xml_with_limits(&wb_xml, XmlLimits::default())?;
    if wb_root.local_name() != "workbook" {
        return Err(FormatError::XmlParse {
            offset: 0,
            reason: format!("expected `<workbook>` root in `{workbook_path}`, found `<{}>`", wb_root.local_name()),
        });
    }

    let wb_dir = workbook_dir(&workbook_path);
    let rels_path = workbook_rels_path(&workbook_path);
    let rels = parse_relationships(archive, &rels_path, wb_dir)?;

    let shared_strings_path = rels
        .values()
        .find(|r| !r.external && r.rel_type.ends_with("/sharedStrings"))
        .map(|r| r.target.clone())
        .or_else(|| find_zip_entry(archive, DEFAULT_SHARED_STRINGS_PATH).map(|_| String::from(DEFAULT_SHARED_STRINGS_PATH)));

    let mut sheets = Vec::new();
    if let Some(sheets_el) = wb_root.first_child("sheets") {
        for sheet_el in sheets_el.children_named("sheet") {
            let name = sheet_el.attr("name").unwrap_or("").to_owned();
            let sheet_id = sheet_el.attr("sheetId").unwrap_or("").to_owned();
            let state = sheet_el.attr("state").map(str::to_owned);
            let rel_id = sheet_el.attr("r:id").or_else(|| sheet_el.attr("id"));

            let resolved = match rel_id {
                Some(id) => match rels.get(id) {
                    Some(rel) if rel.external => {
                        warnings.push(ImportWarning::new(
                            ImportWarningKind::IgnoredSection,
                            name.clone(),
                            format!("skipped external worksheet relationship `{id}`"),
                        ));
                        continue;
                    }
                    Some(rel) if !rel.rel_type.is_empty() && !rel.rel_type.ends_with("/worksheet") => {
                        warnings.push(ImportWarning::new(
                            ImportWarningKind::UnsupportedTableType,
                            name.clone(),
                            format!("skipped non-worksheet tab (`{}`)", rel.rel_type),
                        ));
                        continue;
                    }
                    Some(rel) => rel.target.clone(),
                    None => {
                        return Err(FormatError::MissingArchiveEntry { path: format!("relationship `{id}` referenced by sheet `{name}`") });
                    }
                },
                None => {
                    let guessed = format!("{wb_dir}{}worksheets/sheet{}.xml", if wb_dir.is_empty() { "" } else { "/" }, sheets.len() + 1);
                    sanitize_zip_path(&guessed)?
                }
            };

            sheets.push(XlsxSheetInfo { index: sheets.len(), name, sheet_id, state, path: resolved });
        }
    }

    Ok(WorkbookManifest { sheets, shared_strings_path, warnings })
}

fn resolve_root_workbook_path(archive: &ZipArchive) -> Result<String, FormatError> {
    if let Some(root_rels_bytes) = find_zip_entry(archive, "_rels/.rels") {
        let root_rels_xml = decode_xml_entry(root_rels_bytes, "_rels/.rels")?;
        let rels = parse_rels_xml(&root_rels_xml, "")?;
        if let Some(doc_rel) = rels.values().find(|r| !r.external && r.rel_type.ends_with("/officeDocument")) {
            return Ok(doc_rel.target.clone());
        }
    }
    Ok(String::from(DEFAULT_WORKBOOK_PATH))
}

fn workbook_dir(workbook_path: &str) -> &str {
    workbook_path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn workbook_rels_path(workbook_path: &str) -> String {
    match workbook_path.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{workbook_path}.rels"),
    }
}

fn parse_relationships(archive: &ZipArchive, rels_path: &str, base_dir: &str) -> Result<HashMap<String, RelationshipTarget>, FormatError> {
    let Some(bytes) = find_zip_entry(archive, rels_path) else {
        return Err(FormatError::MissingArchiveEntry { path: rels_path.to_owned() });
    };
    let xml = decode_xml_entry(bytes, rels_path)?;
    parse_rels_xml(&xml, base_dir)
}

fn parse_rels_xml(xml: &str, base_dir: &str) -> Result<HashMap<String, RelationshipTarget>, FormatError> {
    let root = parse_xml_with_limits(xml, XmlLimits::default())?;
    let mut out = HashMap::new();
    for rel_el in root.children_named("Relationship") {
        let Some(id) = rel_el.attr("Id") else {
            continue;
        };
        let rel_type = rel_el.attr("Type").unwrap_or("").to_owned();
        let target_raw = rel_el.attr("Target").unwrap_or("");
        let external = rel_el.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
        let target = if external { target_raw.to_owned() } else { resolve_ooxml_target(base_dir, target_raw)? };
        out.insert(id.to_owned(), RelationshipTarget { rel_type, target, external });
    }
    Ok(out)
}

fn resolve_ooxml_target(base_dir: &str, raw_target: &str) -> Result<String, FormatError> {
    if raw_target.is_empty() || raw_target.contains('\0') || raw_target.contains('\\') {
        return Err(FormatError::UnsafePath { path: raw_target.to_owned() });
    }
    let mut stack: Vec<&str> = Vec::new();
    let is_root_relative = raw_target.starts_with('/');
    if !is_root_relative && !base_dir.is_empty() {
        for seg in base_dir.split('/').filter(|s| !s.is_empty()) {
            stack.push(seg);
        }
    }
    let trimmed = raw_target.trim_start_matches('/');
    for seg in trimmed.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.pop().is_none() {
                    return Err(FormatError::UnsafePath { path: raw_target.to_owned() });
                }
            }
            part => {
                if part.contains(':') {
                    return Err(FormatError::UnsafePath { path: raw_target.to_owned() });
                }
                stack.push(part);
            }
        }
    }
    let joined = stack.join("/");
    sanitize_zip_path(&joined)
}

fn select_target_sheets<'a>(
    sheets: &'a [XlsxSheetInfo],
    opts: &XlsxImportOptions,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Vec<&'a XlsxSheetInfo>, FormatError> {
    if opts.sheet_names.is_none() && opts.sheet_indices.is_none() {
        let mut selected = Vec::new();
        for sheet in sheets {
            let is_hidden = sheet.state.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("hidden") || s.eq_ignore_ascii_case("veryHidden"));
            if is_hidden && !opts.include_hidden {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::IgnoredSection,
                    sheet.name.clone(),
                    format!("skipped hidden worksheet `{}`", sheet.name),
                ));
                continue;
            }
            selected.push(sheet);
        }
        if selected.is_empty() {
            return Ok(sheets.iter().collect());
        }
        return Ok(selected);
    }

    let mut chosen_indices = BTreeSet::new();
    if let Some(ref names) = opts.sheet_names {
        if names.is_empty() {
            return Err(FormatError::InvalidPipeline { reason: String::from("no worksheets selected in `sheet_names`") });
        }
        for req in names {
            let found = sheets.iter().find(|s| s.name == *req).or_else(|| sheets.iter().find(|s| s.name.eq_ignore_ascii_case(req.trim())));
            let Some(sheet) = found else {
                let available: Vec<&str> = sheets.iter().map(|s| s.name.as_str()).collect();
                return Err(FormatError::InvalidPipeline { reason: format!("worksheet `{req}` not found (available: {})", available.join(", ")) });
            };
            chosen_indices.insert(sheet.index);
        }
    }
    if let Some(ref indices) = opts.sheet_indices {
        if indices.is_empty() {
            return Err(FormatError::InvalidPipeline { reason: String::from("no worksheets selected in `sheet_indices`") });
        }
        for &idx in indices {
            if idx >= sheets.len() {
                return Err(FormatError::InvalidPipeline {
                    reason: format!("worksheet index {idx} out of range (workbook has {} sheet(s))", sheets.len()),
                });
            }
            chosen_indices.insert(idx);
        }
    }

    let mut out = Vec::with_capacity(chosen_indices.len());
    for idx in chosen_indices {
        if let Some(sheet) = sheets.get(idx) {
            out.push(sheet);
        }
    }
    Ok(out)
}

fn load_shared_strings(archive: &ZipArchive, shared_strings_path: Option<&str>) -> Result<Vec<String>, FormatError> {
    let Some(path) = shared_strings_path else {
        return Ok(Vec::new());
    };
    let Some(bytes) = find_zip_entry(archive, path) else {
        return Ok(Vec::new());
    };
    let xml = decode_xml_entry(bytes, path)?;
    parse_shared_strings_xml(&xml)
}

fn parse_shared_strings_xml(xml: &str) -> Result<Vec<String>, FormatError> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0_usize;
    let mut in_si = false;
    let mut phonetic_depth = 0_usize;
    let mut in_text = false;
    let mut current = String::new();
    let mut strings = Vec::new();
    let mut total_text_bytes = 0_usize;

    loop {
        let offset = reader_offset(&reader);
        let event = reader.read_event().map_err(|err| FormatError::XmlParse { offset, reason: format!("malformed sharedStrings XML: {err}") })?;
        match event {
            Event::Start(start) => {
                depth = enter_depth(depth)?;
                let name = start.local_name();
                match name.as_ref() {
                    "si" if depth == 2 => {
                        if strings.len() >= MAX_SHARED_STRINGS {
                            return Err(FormatError::ZipBomb { reason: format!("sharedStrings count exceeds maximum of {MAX_SHARED_STRINGS}") });
                        }
                        in_si = true;
                        current.clear();
                    }
                    "rPh" if in_si => {
                        phonetic_depth = phonetic_depth.saturating_add(1);
                    }
                    "t" if in_si && phonetic_depth == 0 => {
                        in_text = true;
                    }
                    _ => {}
                }
            }
            Event::Empty(start) => {
                let _ = enter_depth(depth)?;
                if start.local_name().as_ref() == "si" && depth + 1 == 2 {
                    if strings.len() >= MAX_SHARED_STRINGS {
                        return Err(FormatError::ZipBomb { reason: format!("sharedStrings count exceeds maximum of {MAX_SHARED_STRINGS}") });
                    }
                    strings.push(String::new());
                }
            }
            Event::End(end) => {
                let name = end.local_name();
                match name.as_ref() {
                    "t" if in_text => {
                        in_text = false;
                    }
                    "rPh" if phonetic_depth > 0 => {
                        phonetic_depth -= 1;
                    }
                    "si" if in_si && depth == 2 => {
                        let decoded = decode_ooxml_escapes(&current);
                        total_text_bytes = total_text_bytes.saturating_add(decoded.len());
                        if total_text_bytes > MAX_XML_BYTES {
                            return Err(FormatError::ZipBomb { reason: format!("sharedStrings decoded text exceeds {MAX_XML_BYTES} byte limit") });
                        }
                        strings.push(decoded);
                        in_si = false;
                        current.clear();
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(text) if in_text => {
                current.push_str(text.as_ref());
            }
            Event::CData(cdata) if in_text => {
                current.push_str(cdata.as_ref());
            }
            Event::GeneralRef(entity) if in_text => {
                append_entity(&mut current, entity.as_ref(), offset)?;
            }
            Event::DocType(_) => {
                return Err(FormatError::XmlParse { offset, reason: String::from("DOCTYPE declarations are forbidden in XLSX XML") });
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(strings)
}

fn parse_worksheet_xml(xml: &str, shared_strings: &[String]) -> Result<ParsedWorksheet, FormatError> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0_usize;
    let mut ws = ParsedWorksheet::default();

    let mut current_row_idx: Option<usize> = None;
    let mut next_row_guess: usize = 0;
    let mut next_col_in_row: usize = 0;

    let mut active_cell: Option<ActiveCellState> = None;
    let mut capture_tag = CaptureTag::None;
    let mut phonetic_depth = 0_usize;

    loop {
        let offset = reader_offset(&reader);
        let event = reader.read_event().map_err(|err| FormatError::XmlParse { offset, reason: format!("malformed worksheet XML: {err}") })?;
        match event {
            Event::Start(start) => {
                depth = enter_depth(depth)?;
                handle_ws_start(
                    &start,
                    depth,
                    offset,
                    &mut current_row_idx,
                    &mut next_row_guess,
                    &mut next_col_in_row,
                    &mut active_cell,
                    &mut capture_tag,
                    &mut phonetic_depth,
                    &mut ws,
                )?;
            }
            Event::Empty(start) => {
                let _ = enter_depth(depth)?;
                handle_ws_empty(&start, offset, current_row_idx, &mut next_row_guess, &mut next_col_in_row, &mut active_cell, &mut ws)?;
            }
            Event::End(end) => {
                let name = end.local_name();
                match name.as_ref() {
                    "v" | "t" => {
                        capture_tag = CaptureTag::None;
                    }
                    "rPh" if phonetic_depth > 0 => {
                        phonetic_depth -= 1;
                    }
                    "c" => {
                        capture_tag = CaptureTag::None;
                        phonetic_depth = 0;
                        if let Some(cell_state) = active_cell.take() {
                            commit_cell(cell_state, shared_strings, offset, &mut ws)?;
                        }
                    }
                    "row" => {
                        current_row_idx = None;
                        next_col_in_row = 0;
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(text) => {
                if let Some(ref mut cell) = active_cell {
                    append_captured_text(cell, capture_tag, phonetic_depth, text.as_ref());
                }
            }
            Event::CData(cdata) => {
                if let Some(ref mut cell) = active_cell {
                    append_captured_text(cell, capture_tag, phonetic_depth, cdata.as_ref());
                }
            }
            Event::GeneralRef(entity) => {
                if let Some(ref mut cell) = active_cell {
                    append_captured_entity(cell, capture_tag, phonetic_depth, entity.as_ref(), offset)?;
                }
            }
            Event::DocType(_) => {
                return Err(FormatError::XmlParse { offset, reason: String::from("DOCTYPE declarations are forbidden in XLSX XML") });
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(ws)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureTag {
    None,
    Value,
    InlineText,
}

#[derive(Debug)]
struct ActiveCellState {
    row: usize,
    col: usize,
    coord_label: String,
    cell_type: XlsxCellType,
    has_formula: bool,
    value_buf: String,
    inline_buf: String,
    saw_value: bool,
    saw_inline: bool,
}

#[allow(clippy::too_many_arguments)]
fn handle_ws_start(
    start: &BytesStart<'_>,
    depth: usize,
    offset: usize,
    current_row_idx: &mut Option<usize>,
    next_row_guess: &mut usize,
    next_col_in_row: &mut usize,
    active_cell: &mut Option<ActiveCellState>,
    capture_tag: &mut CaptureTag,
    phonetic_depth: &mut usize,
    ws: &mut ParsedWorksheet,
) -> Result<(), FormatError> {
    match start.local_name().as_ref() {
        "row" => {
            let row_idx = parse_row_attr(start, *next_row_guess, offset)?;
            *current_row_idx = Some(row_idx);
            *next_row_guess = row_idx.saturating_add(1);
            *next_col_in_row = 0;
        }
        "c" => {
            let row_fallback = current_row_idx.unwrap_or(*next_row_guess);
            let cell = begin_cell(start, row_fallback, next_col_in_row, offset)?;
            *active_cell = Some(cell);
            *capture_tag = CaptureTag::None;
            *phonetic_depth = 0;
        }
        "f" if active_cell.is_some() => {
            if let Some(cell) = active_cell.as_mut() {
                cell.has_formula = true;
            }
        }
        "v" if active_cell.is_some() => {
            if let Some(cell) = active_cell.as_mut() {
                cell.saw_value = true;
            }
            *capture_tag = CaptureTag::Value;
        }
        "rPh" if active_cell.is_some() => {
            *phonetic_depth = phonetic_depth.saturating_add(1);
        }
        "t" if active_cell.is_some() && *phonetic_depth == 0 => {
            if let Some(cell) = active_cell.as_mut() {
                cell.saw_inline = true;
            }
            *capture_tag = CaptureTag::InlineText;
        }
        "mergeCell" if depth <= MAX_XML_DEPTH => {
            record_merge_cell(start, offset, ws)?;
        }
        _ => {}
    }
    Ok(())
}

fn handle_ws_empty(
    start: &BytesStart<'_>,
    offset: usize,
    current_row_idx: Option<usize>,
    next_row_guess: &mut usize,
    next_col_in_row: &mut usize,
    active_cell: &mut Option<ActiveCellState>,
    ws: &mut ParsedWorksheet,
) -> Result<(), FormatError> {
    match start.local_name().as_ref() {
        "row" => {
            let row_idx = parse_row_attr(start, *next_row_guess, offset)?;
            *next_row_guess = row_idx.saturating_add(1);
        }
        "c" => {
            let row_fallback = current_row_idx.unwrap_or(*next_row_guess);
            let _ = begin_cell(start, row_fallback, next_col_in_row, offset)?;
        }
        "f" if active_cell.is_some() => {
            if let Some(cell) = active_cell.as_mut() {
                cell.has_formula = true;
            }
        }
        "mergeCell" => {
            record_merge_cell(start, offset, ws)?;
        }
        _ => {}
    }
    Ok(())
}

fn parse_row_attr(start: &BytesStart<'_>, guess: usize, offset: usize) -> Result<usize, FormatError> {
    if let Some(raw_r) = find_attr(start, "r", offset)? {
        let r_one_based: usize =
            raw_r.trim().parse().map_err(|_| FormatError::XmlParse { offset, reason: format!("invalid worksheet `<row r=\"{raw_r}\">`") })?;
        if r_one_based == 0 {
            return Err(FormatError::XmlParse { offset, reason: String::from("worksheet `<row r=\"0\">` must be >= 1") });
        }
        return Ok(r_one_based - 1);
    }
    Ok(guess)
}

fn begin_cell(start: &BytesStart<'_>, row_fallback: usize, next_col_in_row: &mut usize, offset: usize) -> Result<ActiveCellState, FormatError> {
    let raw_ref = find_attr(start, "r", offset)?;
    let raw_type = find_attr(start, "t", offset)?;
    let (row, col, coord_label) = match raw_ref {
        Some(r_str) => {
            let (col_idx, opt_row) = parse_cell_ref(&r_str, offset)?;
            let row_idx = opt_row.unwrap_or(row_fallback);
            *next_col_in_row = col_idx.saturating_add(1);
            (row_idx, col_idx, r_str)
        }
        None => {
            let col_idx = *next_col_in_row;
            *next_col_in_row = col_idx.saturating_add(1);
            (row_fallback, col_idx, format!("R{}C{}", row_fallback + 1, col_idx + 1))
        }
    };

    Ok(ActiveCellState {
        row,
        col,
        coord_label,
        cell_type: XlsxCellType::from_attr(raw_type.as_deref()),
        has_formula: false,
        value_buf: String::new(),
        inline_buf: String::new(),
        saw_value: false,
        saw_inline: false,
    })
}

fn record_merge_cell(start: &BytesStart<'_>, offset: usize, ws: &mut ParsedWorksheet) -> Result<(), FormatError> {
    if ws.merges.len() >= MAX_MERGED_RANGES_PER_SHEET {
        return Err(FormatError::ZipBomb { reason: format!("worksheet mergedCells count exceeds {MAX_MERGED_RANGES_PER_SHEET}") });
    }
    let Some(ref_attr) = find_attr(start, "ref", offset)? else {
        return Ok(());
    };
    let (start_ref, end_ref) = ref_attr.split_once(':').unwrap_or((&ref_attr, &ref_attr));
    let (c1, Some(r1)) = parse_cell_ref(start_ref.trim(), offset)? else {
        return Err(FormatError::XmlParse { offset, reason: format!("invalid mergeCell ref `{ref_attr}`") });
    };
    let (c2, Some(r2)) = parse_cell_ref(end_ref.trim(), offset)? else {
        return Err(FormatError::XmlParse { offset, reason: format!("invalid mergeCell ref `{ref_attr}`") });
    };
    ws.merges.push(MergeRange { r1: r1.min(r2), c1: c1.min(c2), r2: r1.max(r2), c2: c1.max(c2) });
    Ok(())
}

fn commit_cell(state: ActiveCellState, shared_strings: &[String], offset: usize, ws: &mut ParsedWorksheet) -> Result<(), FormatError> {
    if state.has_formula {
        ws.formula_cells = ws.formula_cells.saturating_add(1);
    }
    if !state.saw_value && !state.saw_inline {
        return Ok(());
    }

    let cell = match state.cell_type {
        XlsxCellType::SharedString => {
            let idx_raw = state.value_buf.trim();
            if idx_raw.is_empty() {
                Cell::empty()
            } else {
                let idx: usize = idx_raw.parse().map_err(|_| FormatError::XmlParse {
                    offset,
                    reason: format!("invalid shared string index `{idx_raw}` in cell {}", state.coord_label),
                })?;
                let text = shared_strings.get(idx).ok_or_else(|| FormatError::XmlParse {
                    offset,
                    reason: format!("shared string index {idx} out of bounds (table len {}) in cell {}", shared_strings.len(), state.coord_label),
                })?;
                Cell::text(text.clone())
            }
        }
        XlsxCellType::InlineString => {
            if !state.saw_inline {
                Cell::empty()
            } else {
                Cell::text(decode_ooxml_escapes(&state.inline_buf))
            }
        }
        XlsxCellType::FormulaString => {
            if !state.saw_value {
                Cell::empty()
            } else {
                Cell::text(decode_ooxml_escapes(&state.value_buf))
            }
        }
        XlsxCellType::Boolean => {
            let trimmed = state.value_buf.trim();
            if trimmed.is_empty() {
                Cell::empty()
            } else if trimmed == "1" || trimmed.eq_ignore_ascii_case("true") {
                Cell::text("TRUE")
            } else if trimmed == "0" || trimmed.eq_ignore_ascii_case("false") {
                Cell::text("FALSE")
            } else {
                ws.coerced_numeric_cells.push(state.coord_label.clone());
                Cell::text(trimmed.to_owned())
            }
        }
        XlsxCellType::Error => {
            let err_txt = state.value_buf.trim();
            if err_txt.is_empty() {
                Cell::empty()
            } else {
                ws.error_cells.push((state.coord_label.clone(), err_txt.to_owned()));
                Cell::text(err_txt.to_owned())
            }
        }
        XlsxCellType::IsoDate => {
            let txt = state.value_buf.trim();
            if txt.is_empty() { Cell::empty() } else { Cell::text(txt.to_owned()) }
        }
        XlsxCellType::Numeric => {
            let raw = state.value_buf.trim();
            if raw.is_empty() {
                Cell::empty()
            } else if let Ok(v) = raw.parse::<f64>()
                && v.is_finite()
            {
                Cell::number(v)
            } else {
                ws.coerced_numeric_cells.push(state.coord_label.clone());
                Cell::text(raw.to_owned())
            }
        }
    };

    if cell.is_empty() {
        return Ok(());
    }

    if state.row >= MAX_ROWS {
        return Err(FormatError::Doc(DocError::TooManyRows { count: state.row + 1, max: MAX_ROWS }));
    }
    let max_cols = MAX_DATA_SETS.saturating_mul(MAX_SUBCOLUMNS).saturating_add(4);
    if state.col >= max_cols {
        return Err(FormatError::Doc(DocError::TooManyDataSets { count: state.col + 1, max: MAX_DATA_SETS }));
    }

    ws.max_row = Some(ws.max_row.map_or(state.row, |cur| cur.max(state.row)));
    ws.max_col = Some(ws.max_col.map_or(state.col, |cur| cur.max(state.col)));
    ws.cells.insert((state.row, state.col), cell);
    Ok(())
}

/// Parse an Excel A1-style reference like `"C5"` into `(0-based col, Some(0-based row))`.
fn parse_cell_ref(raw: &str, offset: usize) -> Result<(usize, Option<usize>), FormatError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FormatError::XmlParse { offset, reason: String::from("empty cell reference") });
    }
    let mut col = 0_usize;
    let mut saw_col = false;
    let mut split_idx = trimmed.len();
    for (idx, b) in trimmed.bytes().enumerate() {
        if b.is_ascii_alphabetic() {
            saw_col = true;
            let val = usize::from(b.to_ascii_uppercase() - b'A') + 1;
            col = col
                .checked_mul(26)
                .and_then(|c| c.checked_add(val))
                .ok_or_else(|| FormatError::XmlParse { offset, reason: format!("column overflow in `{raw}`") })?;
        } else {
            split_idx = idx;
            break;
        }
    }
    if !saw_col || col == 0 {
        return Err(FormatError::XmlParse { offset, reason: format!("missing column letters in cell reference `{raw}`") });
    }
    let row_part = trimmed.get(split_idx..).unwrap_or("");
    let row = if row_part.is_empty() {
        None
    } else {
        let r_one_based: usize =
            row_part.parse().map_err(|_| FormatError::XmlParse { offset, reason: format!("invalid row index in cell reference `{raw}`") })?;
        if r_one_based == 0 {
            return Err(FormatError::XmlParse { offset, reason: format!("row index 0 in cell reference `{raw}` must be >= 1") });
        }
        Some(r_one_based - 1)
    };
    Ok((col - 1, row))
}

fn append_captured_text(cell: &mut ActiveCellState, capture_tag: CaptureTag, phonetic_depth: usize, text: &str) {
    match capture_tag {
        CaptureTag::None => {}
        CaptureTag::Value => cell.value_buf.push_str(text),
        CaptureTag::InlineText if phonetic_depth == 0 => cell.inline_buf.push_str(text),
        CaptureTag::InlineText => {}
    }
}

fn append_captured_entity(
    cell: &mut ActiveCellState,
    capture_tag: CaptureTag,
    phonetic_depth: usize,
    entity_name: &str,
    offset: usize,
) -> Result<(), FormatError> {
    match capture_tag {
        CaptureTag::None => Ok(()),
        CaptureTag::Value => append_entity(&mut cell.value_buf, entity_name, offset),
        CaptureTag::InlineText if phonetic_depth == 0 => append_entity(&mut cell.inline_buf, entity_name, offset),
        CaptureTag::InlineText => Ok(()),
    }
}

fn find_attr(start: &BytesStart<'_>, target_local: &str, offset: usize) -> Result<Option<String>, FormatError> {
    for attr_res in start.attributes().with_checks(false) {
        let attr = attr_res.map_err(|err| FormatError::XmlParse { offset, reason: format!("malformed XML attribute: {err}") })?;
        if attr.key.local_name().as_ref() == target_local {
            return decode_xml_entities(attr.value.as_ref(), offset).map(Some);
        }
    }
    Ok(None)
}

fn append_entity(dst: &mut String, entity_name: &str, offset: usize) -> Result<(), FormatError> {
    let wrapped = format!("&{entity_name};");
    let decoded = decode_xml_entities(&wrapped, offset)?;
    dst.push_str(&decoded);
    Ok(())
}

fn enter_depth(current: usize) -> Result<usize, FormatError> {
    let next = current.saturating_add(1);
    if next > MAX_XML_DEPTH {
        return Err(FormatError::XmlDepthExceeded { depth: next, max: MAX_XML_DEPTH });
    }
    Ok(next)
}

fn reader_offset(reader: &Reader<&[u8]>) -> usize {
    usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
}

fn decode_ooxml_escapes(raw: &str) -> String {
    if !raw.contains("_x") {
        return raw.to_owned();
    }
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut idx = 0_usize;
    while idx < bytes.len() {
        if bytes.get(idx..idx + 7).is_some_and(|win| {
            win.first() == Some(&b'_')
                && win.get(1) == Some(&b'x')
                && win.get(6) == Some(&b'_')
                && win.get(2..6).is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
        }) && let Some(hex_str) = raw.get(idx + 2..idx + 6)
            && let Ok(code) = u32::from_str_radix(hex_str, 16)
            && let Some(ch) = char::from_u32(code)
            && (ch == '\r' || ch == '\n' || ch == '\t' || ch == '_' || !ch.is_control())
        {
            out.push(ch);
            idx += 7;
            continue;
        }
        if let Some(rest) = raw.get(idx..)
            && let Some(ch) = rest.chars().next()
        {
            out.push(ch);
            idx += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn emit_worksheet_warnings(sheet_name: &str, ws: &ParsedWorksheet, header_rows: usize, warnings: &mut Vec<ImportWarning>) {
    if ws.formula_cells > 0 {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            sheet_name,
            format!("worksheet `{sheet_name}` contained {} formula cell(s); imported cached values", ws.formula_cells),
        ));
    }
    if !ws.error_cells.is_empty() {
        let sample = ws.error_cells.first().map(|(coord, code)| format!("{coord}={code}")).unwrap_or_default();
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            sheet_name,
            format!("worksheet `{sheet_name}` contained {} error cell(s) (first: {sample})", ws.error_cells.len()),
        ));
    }
    if !ws.coerced_numeric_cells.is_empty() {
        let sample = ws.coerced_numeric_cells.first().cloned().unwrap_or_default();
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            sheet_name,
            format!("worksheet `{sheet_name}` had {} non-finite/non-numeric value(s) kept as text (first: {sample})", ws.coerced_numeric_cells.len()),
        ));
    }
    let body_merges = ws.merges.iter().filter(|m| m.r2 >= header_rows).count();
    if body_merges > 0 {
        warnings.push(ImportWarning::new(
            ImportWarningKind::IgnoredSection,
            sheet_name,
            format!("worksheet `{sheet_name}` had {body_merges} merged range(s) in data rows; kept top-left cell values"),
        ));
    }
}

fn build_datasheet_from_worksheet(
    sheet_id: SheetId,
    title: String,
    ws: &ParsedWorksheet,
    opts: &XlsxImportOptions,
    effective_header_rows: usize,
) -> Result<DataSheet, FormatError> {
    let Some(max_row) = ws.max_row else {
        let kind = opts.table_kind.unwrap_or(TableKind::Column);
        let sub_mode = effective_subcolumn_mode(kind, opts.subcolumn_mode);
        let ds_count = usize::from(!matches!(kind, TableKind::Xy | TableKind::Survival));
        return create_empty_datasheet(sheet_id, title, kind, sub_mode, ds_count);
    };
    let Some(max_col) = ws.max_col else {
        return create_empty_datasheet(sheet_id, title, opts.table_kind.unwrap_or(TableKind::Column), opts.subcolumn_mode, 1);
    };

    let total_rows = max_row + 1;
    let total_cols = max_col + 1;
    let total_grid_cells =
        total_rows.checked_mul(total_cols).ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("worksheet dimensions overflow") })?;
    if total_grid_cells > MAX_IMPORT_CELLS {
        return Err(FormatError::Doc(DocError::TooManyCells { count: total_grid_cells, max: MAX_IMPORT_CELLS }));
    }

    let header_count = effective_header_rows.min(total_rows);
    let headers = extract_xlsx_headers(ws, header_count, total_cols);
    let data_row_count = total_rows.saturating_sub(header_count);
    validate_rows(data_row_count)?;

    let mut data_rows: Vec<Vec<Cell>> = Vec::with_capacity(data_row_count);
    for r in header_count..total_rows {
        let mut row_cells = Vec::with_capacity(total_cols);
        for c in 0..total_cols {
            row_cells.push(ws.cells.get(&(r, c)).cloned().unwrap_or_else(Cell::empty));
        }
        data_rows.push(row_cells);
    }

    let target_kind = opts.table_kind.unwrap_or_else(|| infer_xlsx_table_kind(&data_rows, total_cols, opts));
    let subcolumn_mode = effective_subcolumn_mode(target_kind, opts.subcolumn_mode);
    subcolumn_mode.validate()?;
    if !target_kind.allows_subcolumn_mode(subcolumn_mode) {
        return Err(FormatError::Doc(DocError::IncompatibleSubcolumnMode { archetype: target_kind, mode: subcolumn_mode }));
    }

    assemble_datasheet(sheet_id, title, target_kind, subcolumn_mode, opts.has_row_titles, &headers, &data_rows, total_cols)
}

fn create_empty_datasheet(
    sheet_id: SheetId,
    title: String,
    kind: TableKind,
    sub_mode: SubcolumnMode,
    ds_count: usize,
) -> Result<DataSheet, FormatError> {
    assemble_datasheet(sheet_id, title, kind, sub_mode, false, &[], &[], ds_count)
}

fn extract_xlsx_headers(ws: &ParsedWorksheet, header_count: usize, total_cols: usize) -> Vec<String> {
    if header_count == 0 {
        return Vec::new();
    }
    let mut raw_rows = vec![vec![String::new(); total_cols]; header_count];
    for r in 0..header_count {
        for c in 0..total_cols {
            if let Some(cell) = ws.cells.get(&(r, c))
                && let Some(row_vec) = raw_rows.get_mut(r)
                && let Some(slot) = row_vec.get_mut(c)
            {
                *slot = cell_as_header_text(cell);
            }
        }
    }
    for merge in &ws.merges {
        if merge.r1 >= header_count {
            continue;
        }
        let anchor_val = ws.cells.get(&(merge.r1, merge.c1)).map(cell_as_header_text).unwrap_or_default();
        if anchor_val.trim().is_empty() {
            continue;
        }
        let r_end = merge.r2.min(header_count.saturating_sub(1));
        let c_end = merge.c2.min(total_cols.saturating_sub(1));
        for r in merge.r1..=r_end {
            for c in merge.c1..=c_end {
                if let Some(row_vec) = raw_rows.get_mut(r)
                    && let Some(slot) = row_vec.get_mut(c)
                    && slot.trim().is_empty()
                {
                    *slot = anchor_val.clone();
                }
            }
        }
    }
    collapse_merged_header_rows(&raw_rows)
}

fn cell_as_header_text(cell: &Cell) -> String {
    match &cell.value {
        curvebook_doc::CellValue::Empty => String::new(),
        curvebook_doc::CellValue::Text(s) => s.clone(),
        curvebook_doc::CellValue::Number(v) => v.to_string(),
    }
}

fn cell_as_label(cell: &Cell) -> String {
    cell_as_header_text(cell).trim().to_owned()
}

fn infer_xlsx_table_kind(data_rows: &[Vec<Cell>], total_cols: usize, opts: &XlsxImportOptions) -> TableKind {
    let col_offset = usize::from(opts.has_row_titles && total_cols > 0);
    let active_cols = total_cols.saturating_sub(col_offset);
    if active_cols <= 1 || opts.subcolumn_mode != SubcolumnMode::Single {
        return TableKind::Column;
    }

    let has_text_col0 = data_rows.iter().any(|row| row.get(col_offset).is_some_and(|cell| cell.active_text().is_some()));
    let has_text_later =
        data_rows.iter().any(|row| (col_offset + 1..total_cols).any(|c| row.get(c).is_some_and(|cell| cell.active_text().is_some())));

    if has_text_col0 || has_text_later {
        return TableKind::MultipleVariables;
    }
    TableKind::Xy
}

fn effective_subcolumn_mode(kind: TableKind, requested: SubcolumnMode) -> SubcolumnMode {
    if matches!(kind, TableKind::Grouped) && requested == SubcolumnMode::Single {
        SubcolumnMode::RawReplicates(1)
    } else if matches!(kind, TableKind::Contingency | TableKind::Survival | TableKind::PartsOfWhole | TableKind::MultipleVariables) {
        SubcolumnMode::Single
    } else {
        requested
    }
}

#[allow(clippy::too_many_arguments)]
fn assemble_datasheet(
    sheet_id: SheetId,
    title: String,
    target_kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    has_row_titles_flag: bool,
    headers: &[String],
    data_rows: &[Vec<Cell>],
    raw_col_count: usize,
) -> Result<DataSheet, FormatError> {
    let row_count = data_rows.len();
    let has_row_titles = has_row_titles_flag && raw_col_count > 0;
    let mut col_offset = usize::from(has_row_titles);

    let arch = match target_kind {
        TableKind::Xy => {
            let x_title = header_at(headers, col_offset).unwrap_or_else(|| String::from("X"));
            let x_col_idx = col_offset;
            if raw_col_count > col_offset {
                col_offset += 1;
            }
            let subs = subcolumn_mode.subcolumn_count();
            let ds_count = raw_col_count.saturating_sub(col_offset).div_ceil(subs);
            validate_data_sets(ds_count)?;
            let mut xy = XyTable::new(subcolumn_mode, ds_count, row_count)?;
            xy.x_title = clamp_header_title(&x_title)?;
            assign_xlsx_row_titles(&mut xy.row_titles, data_rows, has_row_titles);
            xy.row_titles_expanded = has_row_titles;
            for (r, rec) in data_rows.iter().enumerate() {
                if let Some(cell) = rec.get(x_col_idx) {
                    xy.set_x_cell(r, cell.clone())?;
                }
            }
            populate_xlsx_datasets(&mut xy.data_sets, data_rows, headers, col_offset, subs)?;
            TableArchetype::Xy(xy)
        }
        TableKind::Column => {
            let subs = subcolumn_mode.subcolumn_count();
            let ds_count = raw_col_count.saturating_sub(col_offset).div_ceil(subs);
            validate_data_sets(ds_count)?;
            let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, subcolumn_mode, ds_count, row_count)?;
            assign_xlsx_row_titles(&mut col.row_titles, data_rows, has_row_titles);
            col.row_titles_expanded = has_row_titles;
            populate_xlsx_datasets(&mut col.data_sets, data_rows, headers, col_offset, subs)?;
            TableArchetype::Column(col)
        }
        TableKind::Grouped => {
            let subs = subcolumn_mode.subcolumn_count();
            let ds_count = raw_col_count.saturating_sub(col_offset).div_ceil(subs);
            validate_data_sets(ds_count)?;
            let mut grp = GroupedTable::new(subcolumn_mode, ds_count, row_count)?;
            assign_xlsx_row_titles(&mut grp.row_titles, data_rows, has_row_titles);
            populate_xlsx_datasets(&mut grp.data_sets, data_rows, headers, col_offset, subs)?;
            TableArchetype::Grouped(grp)
        }
        TableKind::Contingency => {
            let ds_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(ds_count)?;
            let mut cont = ContingencyTable::new(ds_count, row_count)?;
            assign_xlsx_row_titles(&mut cont.row_titles, data_rows, has_row_titles);
            populate_xlsx_datasets(&mut cont.data_sets, data_rows, headers, col_offset, 1)?;
            TableArchetype::Contingency(cont)
        }
        TableKind::Survival => {
            let time_title = header_at(headers, col_offset).unwrap_or_else(|| String::from("Elapsed Time"));
            let time_col = col_offset;
            if raw_col_count > col_offset {
                col_offset += 1;
            }
            let ds_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(ds_count)?;
            let mut surv = SurvivalTable::new(ds_count, row_count)?;
            surv.time_title = clamp_header_title(&time_title)?;
            assign_xlsx_row_titles(&mut surv.row_titles, data_rows, has_row_titles);
            surv.row_titles_expanded = has_row_titles;
            for (r, rec) in data_rows.iter().enumerate() {
                if let Some(cell) = rec.get(time_col)
                    && let Some(slot) = surv.times.get_mut(r)
                {
                    *slot = cell.clone();
                }
            }
            populate_xlsx_datasets(&mut surv.cohorts, data_rows, headers, col_offset, 1)?;
            TableArchetype::Survival(surv)
        }
        TableKind::PartsOfWhole => {
            let label_col = col_offset;
            if raw_col_count > col_offset && !has_row_titles {
                col_offset += 1;
            }
            let ds_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(ds_count)?;
            let mut pow = PartsOfWholeTable::new(ds_count, row_count)?;
            if has_row_titles {
                assign_xlsx_row_titles(&mut pow.slice_labels, data_rows, true);
            } else if raw_col_count > label_col {
                for (slot, rec) in pow.slice_labels.iter_mut().zip(data_rows.iter()) {
                    if let Some(cell) = rec.get(label_col) {
                        *slot = cell_as_label(cell);
                    }
                }
            }
            populate_xlsx_datasets(&mut pow.data_sets, data_rows, headers, col_offset, 1)?;
            TableArchetype::PartsOfWhole(pow)
        }
        TableKind::MultipleVariables => {
            let var_count = raw_col_count.saturating_sub(col_offset);
            validate_data_sets(var_count)?;
            let mut mv = MultipleVariablesTable::new(var_count, row_count)?;
            assign_xlsx_row_titles(&mut mv.row_titles, data_rows, has_row_titles);
            mv.row_titles_expanded = has_row_titles;
            for (idx, var) in mv.variables.iter_mut().enumerate() {
                let c = col_offset + idx;
                if let Some(h) = header_at(headers, c) {
                    var.title = clamp_header_title(&h)?;
                }
                for (r, rec) in data_rows.iter().enumerate() {
                    if let Some(cell) = rec.get(c)
                        && let Some(slot) = var.cells.get_mut(r)
                    {
                        *slot = cell.clone();
                    }
                }
                var.refresh_inferred_role();
            }
            TableArchetype::MultipleVariables(mv)
        }
        TableKind::Nested => {
            let total_data_cols = raw_col_count.saturating_sub(col_offset);
            let subs = match subcolumn_mode {
                SubcolumnMode::RawReplicates(n) => n.max(1),
                SubcolumnMode::Single
                | SubcolumnMode::MeanSdN
                | SubcolumnMode::MeanSemN
                | SubcolumnMode::MeanCvN
                | SubcolumnMode::MeanLowHigh
                | SubcolumnMode::UpperLowerLimits
                | SubcolumnMode::MedianIqr => 1,
            };
            validate_subcolumns(subs)?;
            let group_count = total_data_cols.div_ceil(subs);
            validate_data_sets(group_count)?;
            let mut groups: Vec<NestedGroupColumn> = Vec::with_capacity(group_count);
            for g in 0..group_count {
                let title_idx = col_offset + g.saturating_mul(subs);
                let title_str = header_at(headers, title_idx)
                    .map(|h| clamp_header_title(&h))
                    .transpose()?
                    .unwrap_or_else(|| format!("Group {}", curvebook_doc::default_column_label(g)));
                let mut subcolumn_titles = Vec::with_capacity(subs);
                for s in 0..subs {
                    let c = col_offset + g.saturating_mul(subs) + s;
                    let sub_title =
                        header_at(headers, c).map(|h| clamp_header_title(&h)).transpose()?.unwrap_or_else(|| format!("Subcolumn {}", s + 1));
                    subcolumn_titles.push(sub_title);
                }
                let mut cells = vec![vec![Cell::empty(); subs]; row_count];
                for (r, rec) in data_rows.iter().enumerate() {
                    if let Some(cell_row) = cells.get_mut(r) {
                        for (s, slot) in cell_row.iter_mut().enumerate() {
                            let c = col_offset + g.saturating_mul(subs) + s;
                            if let Some(cell) = rec.get(c) {
                                *slot = cell.clone();
                            }
                        }
                    }
                }
                groups.push(NestedGroupColumn { title: title_str, subcolumn_titles, cells });
            }
            let mut nested = NestedTable { row_titles: vec![String::new(); row_count], row_titles_expanded: has_row_titles, groups };
            assign_xlsx_row_titles(&mut nested.row_titles, data_rows, has_row_titles);
            TableArchetype::Nested(nested)
        }
    };

    Ok(DataSheet::new(sheet_id, title, arch)?)
}

fn assign_xlsx_row_titles(dst: &mut [String], data_rows: &[Vec<Cell>], has_row_titles: bool) {
    if !has_row_titles {
        return;
    }
    for (slot, rec) in dst.iter_mut().zip(data_rows.iter()) {
        if let Some(cell) = rec.first() {
            *slot = cell_as_label(cell);
        }
    }
}

fn populate_xlsx_datasets(
    data_sets: &mut [DataSetColumn],
    data_rows: &[Vec<Cell>],
    headers: &[String],
    col_offset: usize,
    subs: usize,
) -> Result<(), FormatError> {
    for (ds_idx, ds) in data_sets.iter_mut().enumerate() {
        let header_idx = col_offset + ds_idx * subs;
        if let Some(h) = header_at(headers, header_idx) {
            ds.title = clamp_header_title(&h)?;
        }
        for (r, rec) in data_rows.iter().enumerate() {
            if let Some(cell_row) = ds.cells.get_mut(r) {
                for (s, slot) in cell_row.iter_mut().enumerate() {
                    let c = col_offset + ds_idx * subs + s;
                    if let Some(cell) = rec.get(c) {
                        *slot = cell.clone();
                    }
                }
            }
        }
    }
    Ok(())
}

fn header_at(headers: &[String], col_idx: usize) -> Option<String> {
    headers.get(col_idx).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn clamp_header_title(raw: &str) -> Result<String, FormatError> {
    let trimmed = raw.trim();
    if trimmed.len() <= MAX_TITLE_BYTES {
        validate_title(trimmed)?;
        return Ok(trimmed.to_owned());
    }
    let clipped = truncate_utf8_to_bytes(trimmed, MAX_TITLE_BYTES).trim().to_owned();
    validate_title(&clipped)?;
    Ok(clipped)
}

fn clamp_title(raw: &str, kind: &str, warnings: &mut Vec<ImportWarning>) -> String {
    let trimmed = raw.trim();
    if trimmed.len() <= MAX_TITLE_BYTES && validate_title(trimmed).is_ok() {
        return trimmed.to_owned();
    }
    let clipped = truncate_utf8_to_bytes(trimmed, MAX_TITLE_BYTES.saturating_sub(3));
    warnings.push(ImportWarning::new(
        ImportWarningKind::TruncatedTitle,
        trimmed.chars().take(64).collect::<String>(),
        format!("truncated oversized {kind} title to {MAX_TITLE_BYTES} bytes"),
    ));
    format!("{clipped}...")
}

fn truncate_utf8_to_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = 0_usize;
    for (idx, ch) in s.char_indices() {
        let next = idx + ch.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    s.get(..end).unwrap_or("")
}

fn make_unique_sheet_id(raw_name: &str, index: usize, used: &mut HashSet<String>) -> SheetId {
    let fallback = format!("xlsx-{}", index + 1);
    let mut slug = String::with_capacity(raw_name.len());
    let mut prev_dash = false;
    for ch in raw_name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if matches!(ch, '_' | '-') {
            slug.push(ch);
            prev_dash = ch == '-';
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    let max_len = MAX_SHEET_ID_BYTES.saturating_sub(MAX_SHEET_ID_SUFFIX_HEADROOM);
    let base_raw = truncate_utf8_to_bytes(trimmed, max_len).trim_end_matches('-');
    let base = if base_raw.is_empty() || SheetId::try_new(base_raw).is_err() { fallback.as_str() } else { base_raw };

    if used.insert(base.to_owned())
        && let Ok(id) = SheetId::try_new(base)
    {
        return id;
    }
    let mut n = 2_usize;
    loop {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone())
            && let Ok(id) = SheetId::try_new(&candidate)
        {
            return id;
        }
        n = n.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::ZipWriter;

    fn build_xlsx_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut zw = ZipWriter::new();
        for (path, body) in entries {
            zw.add_file(path, body.as_bytes().to_vec()).expect("add zip entry");
        }
        zw.finish().expect("finish zip")
    }

    #[test]
    fn parses_all_xlsx_cell_types_sparse_coords_and_rich_strings() {
        let rels_root = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
        let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Assay Data" sheetId="1" r:id="rId1"/>
    <sheet name="Summary" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;
        let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="/xl/worksheets/sheet2.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>"#;
        let shared = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">
  <si><t>Dose</t></si>
  <si><r><t xml:space="preserve">Response </t></r><r><rPr><b/></rPr><t>A</t></r><rPh><t>IGNORED</t></rPh></si>
  <si><t>Condition</t></si>
  <si><t>Ctrl</t></si>
</sst>"#;
        let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1" t="s"><v>0</v></c>
      <c r="B1" t="s"><v>1</v></c>
      <c r="C1" t="inlineStr"><is><t>Flag</t></is></c>
      <c r="D1" t="s"><v>2</v></c>
    </row>
    <row r="2">
      <c r="A2"><v>0.125</v></c>
      <c r="B2"><f>A2*10</f><v>1.25</v></c>
      <c r="C2" t="b"><v>1</v></c>
      <c r="D2" t="str"><f>"Tag"&amp;A2</f><v>Tag0.125</v></c>
    </row>
    <row r="5">
      <c r="A5" t="s"><v>3</v></c>
      <c r="B5" t="e"><v>#DIV/0!</v></c>
      <c r="C5"><v>99.5</v></c>
      <c r="D5" t="b"><v>0</v></c>
    </row>
    <row r="1048576">
      <c r="A1048576" s="1"/>
    </row>
  </sheetData>
</worksheet>"#;
        let sheet2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>X</t></is></c><c r="B1" t="inlineStr"><is><t>Y</t></is></c></row>
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>2</v></c></row>
  </sheetData>
</worksheet>"#;

        let bytes = build_xlsx_zip(&[
            ("_rels/.rels", rels_root),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", wb_rels),
            ("xl/sharedStrings.xml", shared),
            ("xl/worksheets/sheet1.xml", sheet1),
            ("xl/worksheets/sheet2.xml", sheet2),
        ]);

        let listed = list_xlsx_sheets(&bytes).expect("list sheets");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed.first().map(|s| s.name.as_str()), Some("Assay Data"));
        assert_eq!(listed.get(1).map(|s| s.name.as_str()), Some("Summary"));

        let report = import_xlsx_bytes(&bytes).expect("import all sheets");
        assert_eq!(report.document.data_sheets.len(), 2);
        assert!(report.warnings.iter().any(|w| w.message.contains("formula cell(s)")));
        assert!(report.warnings.iter().any(|w| w.message.contains("#DIV/0!")));

        let s1 = report.document.data_sheets.first().expect("sheet 1");
        assert_eq!(s1.title, "Assay Data");
        let TableArchetype::MultipleVariables(mv) = &s1.archetype else {
            panic!("expected MultipleVariables due to mixed text/bool columns, got {:?}", s1.archetype);
        };
        assert_eq!(mv.variables.len(), 4);
        assert_eq!(mv.variables.first().map(|v| v.title.as_str()), Some("Dose"));
        assert_eq!(mv.variables.get(1).map(|v| v.title.as_str()), Some("Response A"));
        assert_eq!(mv.variables.get(2).map(|v| v.title.as_str()), Some("Flag"));
        assert_eq!(mv.variables.get(3).map(|v| v.title.as_str()), Some("Condition"));
        // Row 5 in Excel -> index 4 total -> data row 3 (rows 1 and 2 skipped as sparse empty!)
        assert_eq!(mv.variables.first().and_then(|v| v.cells.first()), Some(&Cell::number(0.125)));
        assert_eq!(mv.variables.first().and_then(|v| v.cells.get(1)), Some(&Cell::empty()));
        assert_eq!(mv.variables.first().and_then(|v| v.cells.get(2)), Some(&Cell::empty()));
        assert_eq!(mv.variables.first().and_then(|v| v.cells.get(3)), Some(&Cell::text("Ctrl")));
        assert_eq!(mv.variables.get(1).and_then(|v| v.cells.first()), Some(&Cell::number(1.25)));
        assert_eq!(mv.variables.get(1).and_then(|v| v.cells.get(3)), Some(&Cell::text("#DIV/0!")));
        assert_eq!(mv.variables.get(2).and_then(|v| v.cells.first()), Some(&Cell::text("TRUE")));
        assert_eq!(mv.variables.get(2).and_then(|v| v.cells.get(3)), Some(&Cell::number(99.5)));
        assert_eq!(mv.variables.get(3).and_then(|v| v.cells.first()), Some(&Cell::text("Tag0.125")));
        assert_eq!(mv.variables.get(3).and_then(|v| v.cells.get(3)), Some(&Cell::text("FALSE")));

        // Single-sheet selection by name + by index
        let by_name = import_xlsx_bytes_with_options(
            &bytes,
            &XlsxImportOptions { sheet_names: Some(vec![String::from("Summary")]), ..XlsxImportOptions::default() },
        )
        .expect("import by sheet name");
        assert_eq!(by_name.document.data_sheets.len(), 1);
        assert_eq!(by_name.document.data_sheets.first().map(|s| s.title.as_str()), Some("Summary"));
    }

    #[test]
    fn merged_multi_row_headers_and_security_guards() {
        let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Merged" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;
        let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
        let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Drug A</t></is></c></row>
    <row r="2"><c r="B2" t="inlineStr"><is><t>Rep 1</t></is></c><c r="C2" t="inlineStr"><is><t>Rep 2</t></is></c></row>
    <row r="3"><c r="A3"><v>1</v></c><c r="B3"><v>10</v></c><c r="C3"><v>12</v></c></row>
  </sheetData>
  <mergeCells count="1"><mergeCell ref="B1:C1"/></mergeCells>
</worksheet>"#;

        let bytes = build_xlsx_zip(&[("xl/workbook.xml", workbook), ("xl/_rels/workbook.xml.rels", wb_rels), ("xl/worksheets/sheet1.xml", sheet1)]);
        let opts = XlsxImportOptions {
            header_rows: 2,
            table_kind: Some(TableKind::Xy),
            subcolumn_mode: SubcolumnMode::RawReplicates(2),
            ..XlsxImportOptions::default()
        };
        let report = import_xlsx_bytes_with_options(&bytes, &opts).expect("merged headers");
        let sheet = report.document.data_sheets.first().expect("sheet");
        let TableArchetype::Xy(xy) = &sheet.archetype else {
            panic!("expected XY table");
        };
        assert_eq!(xy.x_title, "Dose");
        assert_eq!(xy.data_sets.first().map(|d| d.title.as_str()), Some("Drug A - Rep 1"));
        assert_eq!(xy.data_sets.first().and_then(|d| d.cells.first()), Some(&vec![Cell::number(10.0), Cell::number(12.0)]));

        // Reject XXE DOCTYPE inside worksheet XML
        let evil_sheet = r#"<?xml version="1.0"?><!DOCTYPE foo [<!ENTITY xxe "boom">]><worksheet><sheetData/></worksheet>"#;
        let evil_bytes =
            build_xlsx_zip(&[("xl/workbook.xml", workbook), ("xl/_rels/workbook.xml.rels", wb_rels), ("xl/worksheets/sheet1.xml", evil_sheet)]);
        assert!(matches!(import_xlsx_bytes(&evil_bytes), Err(FormatError::XmlParse { .. })));

        // Reject path traversal in relationship Target
        let traversal_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="../../etc/passwd"/>
</Relationships>"#;
        let traversal_bytes = build_xlsx_zip(&[("xl/workbook.xml", workbook), ("xl/_rels/workbook.xml.rels", traversal_rels)]);
        assert!(matches!(list_xlsx_sheets(&traversal_bytes), Err(FormatError::UnsafePath { .. })));
    }
}
