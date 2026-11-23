//! Clean-room `.pzfx` XML project importer.
//!
//! Parses scientific XML tables, replicate subcolumns, row/column titles (preserving nested
//! formatting text), excluded-cell masks (`Excluded="1"`), and Info constants into a
//! [`ProjectDocument`] alongside non-fatal [`ImportWarning`] diagnostics.

use std::collections::HashSet;

use curvebook_doc::error::MAX_SHEET_ID_BYTES;
use curvebook_doc::{
    Cell, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSetColumn, DataSheet, GroupedTable, InfoConstant, InfoSheet,
    MAX_TITLE_BYTES, MultipleVariablesTable, NestedGroupColumn, NestedTable, PartsOfWholeTable, ProjectDocument, SequenceProgression, SheetId,
    SubcolumnMode, SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable, default_column_label, validate_data_sets, validate_rows,
    validate_subcolumns,
};

use crate::archive::validate_project_document;
use crate::error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
use crate::xml::{XmlElement, parse_xml};

/// Maximum total dense `Cell` entries allocated across all tables of a single PZFX import.
pub const MAX_IMPORT_CELLS: usize = crate::csv::MAX_IMPORT_CELLS;

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
    let text = std::str::from_utf8(bytes).map_err(|err| FormatError::XmlParse { offset: 0, reason: format!("invalid UTF-8 in XML: {err}") })?;
    import_pzfx_str_with_used_cells(text, used_cells)
}

pub(crate) fn import_pzfx_str_with_used_cells(xml: &str, used_cells: &mut usize) -> Result<ImportReport, FormatError> {
    let root = parse_xml(xml)?;
    let local = root.local_name();
    if !(local == "PrismFile" || local.ends_with("PrismFile")) {
        return Err(FormatError::UnsupportedFormat { reason: format!("unexpected `.pzfx` root element `<{local}>`") });
    }

    let mut warnings = Vec::new();
    let title =
        root.first_child("Title").map(XmlElement::plain_text).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| String::from("Imported Project"));
    let clamped_title = clamp_title(&title, "project", &mut warnings);
    let mut doc = ProjectDocument::new(clamped_title)?;
    let mut used_ids = HashSet::new();

    import_info_sections(&root, &mut doc, &mut used_ids, &mut warnings)?;
    import_table_sections(&root, &mut doc, &mut used_ids, &mut warnings, used_cells)?;
    warn_ignored_sections(&root, &mut warnings);

    validate_project_document(&doc)?;
    Ok(ImportReport { document: doc, warnings })
}

fn import_info_sections(
    root: &XmlElement,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    let mut info_nodes: Vec<&XmlElement> = Vec::new();
    for seq in root.children_named("InfoSequence") {
        info_nodes.extend(seq.children_named("Info"));
    }
    info_nodes.extend(root.children_named("Info"));

    for (idx, info_el) in info_nodes.iter().enumerate() {
        let id = allocate_valid_unique_sheet_id(info_el.attr("ID"), "info", idx + 1, used_ids, warnings);
        let raw_title =
            info_el.first_child("Title").map(XmlElement::plain_text).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| format!("Info {}", idx + 1));
        let title = clamp_title(&raw_title, id.as_str(), warnings);

        let mut sheet = InfoSheet::new_preseeded(id.as_str().to_owned(), title)?;
        sheet.constants.clear();
        for c_el in info_el.children_named("Constant") {
            let name_raw = c_el
                .first_child("Name")
                .map(|el| el.plain_text().trim().to_owned())
                .or_else(|| c_el.attr("Name").map(|s| s.trim().to_owned()))
                .unwrap_or_default();
            if name_raw.is_empty() {
                continue;
            }
            let name = clamp_title(&name_raw, "constant", warnings);
            let val_raw = c_el.first_child("Value").map(|el| el.plain_text()).or_else(|| c_el.attr("Value").map(str::to_owned)).unwrap_or_default();
            let val_trimmed = val_raw.trim();
            let value = if val_trimmed.is_empty() {
                ConstantValue::Empty
            } else if let Ok(num) = val_trimmed.parse::<f64>()
                && num.is_finite()
            {
                ConstantValue::Number(num)
            } else {
                ConstantValue::Text(val_trimmed.to_owned())
            };
            let units = c_el
                .first_child("Units")
                .map(|el| clamp_title(el.plain_text().trim(), "units", warnings))
                .or_else(|| c_el.attr("Units").map(|s| clamp_title(s.trim(), "units", warnings)))
                .filter(|u| !u.is_empty());
            sheet.constants.push(InfoConstant::new(name, value, units)?);
        }
        if let Some(notes_el) = info_el.first_child("Notes") {
            sheet.notes_markdown = notes_el.plain_text().trim().to_owned();
        }
        doc.add_info_sheet(sheet)?;
    }
    Ok(())
}

fn import_table_sections(
    root: &XmlElement,
    doc: &mut ProjectDocument,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    total_cells: &mut usize,
) -> Result<(), FormatError> {
    let mut table_nodes: Vec<&XmlElement> = Vec::new();
    for seq in root.children_named("TableSequence") {
        table_nodes.extend(seq.children_named("Table"));
    }
    table_nodes.extend(root.children_named("Table"));

    let primary_info = doc.info_sheets.first().map(|s| s.id.clone());
    for (idx, table_el) in table_nodes.iter().enumerate() {
        let mut ds = parse_table_element(table_el, idx + 1, used_ids, warnings, *total_cells)?;
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
        if ds.info_sheet_id.is_none() {
            ds.info_sheet_id = primary_info.clone();
        }
        doc.add_data_sheet(ds)?;
    }
    Ok(())
}

fn parse_table_element(
    table_el: &XmlElement,
    one_based_idx: usize,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    used_cells: usize,
) -> Result<DataSheet, FormatError> {
    let id = allocate_valid_unique_sheet_id(table_el.attr("ID"), "data", one_based_idx, used_ids, warnings);
    let raw_title =
        table_el.first_child("Title").map(XmlElement::plain_text).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| format!("Data {one_based_idx}"));
    let title = clamp_title(&raw_title, id.as_str(), warnings);

    let kind = resolve_table_kind(table_el.attr("TableType"), id.as_str(), warnings);
    let explicit_replicates = table_el.attr("Replicates").and_then(|s| s.trim().parse::<usize>().ok());
    let replicates = explicit_replicates.unwrap_or(1);
    if replicates > 0 {
        validate_subcolumns(replicates)?;
    }
    let y_format = table_el.attr("YFormat").or_else(|| table_el.attr("EVFormat"));
    let initial_subcolumn_mode = resolve_subcolumn_mode(kind, y_format, replicates, id.as_str(), warnings)?;

    let row_titles_el = table_el.first_child("RowTitlesColumn");
    let row_titles_declared_subs = row_titles_el.map(parse_subcolumns_attr).transpose()?.flatten();
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

    let x_el = table_el.first_child("XColumn");
    let x_declared_subs = x_el.map(parse_subcolumns_attr).transpose()?.flatten();
    let x_title = x_el
        .and_then(|el| el.first_child("Title"))
        .map(XmlElement::plain_text)
        .filter(|s| !s.trim().is_empty())
        .map(|raw| clamp_title(&raw, id.as_str(), warnings))
        .unwrap_or_else(|| String::from("X"));
    let x_cols = x_el.map(extract_column_subcolumns).unwrap_or_default();
    let x_mode = resolve_x_mode(table_el, &x_cols, x_declared_subs)?;

    let y_elements: Vec<&XmlElement> =
        table_el.children_named("YColumn").chain(table_el.children_named("Column")).chain(table_el.children_named("VariableColumn")).collect();
    validate_data_sets(y_elements.len())?;

    let mut parsed_y: Vec<ParsedYColumn> = Vec::with_capacity(y_elements.len());
    for (c_idx, y_el) in y_elements.iter().enumerate() {
        let col_title = y_el
            .first_child("Title")
            .map(XmlElement::plain_text)
            .filter(|s| !s.trim().is_empty())
            .map(|raw| clamp_title(&raw, id.as_str(), warnings))
            .unwrap_or_else(|| default_column_label(c_idx));
        let declared_subcolumns = parse_subcolumns_attr(y_el)?;
        let (sub_titles, subcolumns) = extract_labeled_subcolumns(y_el, id.as_str(), warnings);
        parsed_y.push(ParsedYColumn { title: col_title, declared_subcolumns, sub_titles, subcolumns });
    }

    let subcolumn_mode = widen_y_subcolumn_mode(kind, initial_subcolumn_mode, &parsed_y)?;

    let max_y_rows = parsed_y.iter().flat_map(|col| col.subcolumns.iter().map(Vec::len)).max().unwrap_or(0);
    let max_x_rows = x_cols.iter().map(Vec::len).max().unwrap_or(0);
    let row_count = row_titles.len().max(max_x_rows).max(max_y_rows);
    validate_rows(row_count)?;

    let mut padded_row_titles = row_titles;
    padded_row_titles.resize(row_count, String::new());
    let has_row_titles = padded_row_titles.iter().any(|s| !s.is_empty());

    warn_discarded_x_subcolumns(warnings, (id.as_str(), &title), kind, &x_mode, &x_title, &x_cols, x_declared_subs, has_row_titles);
    if kind != TableKind::Nested {
        let allowed_y_subs = match kind {
            TableKind::Contingency | TableKind::Survival | TableKind::PartsOfWhole | TableKind::MultipleVariables => 1,
            _ => subcolumn_mode.subcolumn_count(),
        };
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

    let remaining_cells = MAX_IMPORT_CELLS.saturating_sub(used_cells);
    let table_cells = estimate_table_cells(kind, subcolumn_mode, &x_mode, &parsed_y, row_count)?;
    if table_cells > remaining_cells {
        return Err(FormatError::XmlParse {
            offset: 0,
            reason: format!(
                "table `{id}` cell allocation ({table_cells}) exceeds remaining import budget ({remaining_cells}, cap {MAX_IMPORT_CELLS})"
            ),
        });
    }

    let (effective_row_titles, effective_has_row_titles) = if kind == TableKind::PartsOfWhole
        && !has_row_titles
        && let Some(first_x_raw) = x_el.and_then(|el| extract_raw_subcolumn_strings(el).into_iter().next())
    {
        let mut labels: Vec<String> = first_x_raw.iter().map(|raw| clamp_title(raw, &format!("table `{title}` slice label"), warnings)).collect();
        labels.resize(row_count, String::new());
        (labels, true)
    } else {
        (padded_row_titles, has_row_titles)
    };
    let mut arch =
        build_table_archetype(kind, subcolumn_mode, x_mode, &x_title, &x_cols, &parsed_y, effective_row_titles, effective_has_row_titles, row_count)?;
    if let TableArchetype::Nested(ref mut nested) = arch {
        let nested_sub_titles: Vec<Vec<Option<String>>> = parsed_y.iter().map(|col| col.sub_titles.clone()).collect();
        apply_nested_subcolumn_titles(nested, &nested_sub_titles);
    }

    Ok(DataSheet::new(id, title, arch)?)
}

struct ParsedYColumn {
    title: String,
    declared_subcolumns: Option<usize>,
    sub_titles: Vec<Option<String>>,
    subcolumns: Vec<Vec<Cell>>,
}

fn parse_subcolumns_attr(el: &XmlElement) -> Result<Option<usize>, FormatError> {
    let Some(raw) = el.attr("Subcolumns").map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let Ok(count) = raw.parse::<usize>() else {
        return Ok(None);
    };
    if count == 0 {
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
    x_mode: XColumnMode,
    x_title: &str,
    x_cols: &[Vec<Cell>],
    parsed_y: &[ParsedYColumn],
    row_titles: Vec<String>,
    has_row_titles: bool,
    row_count: usize,
) -> Result<TableArchetype, FormatError> {
    match kind {
        TableKind::Xy => {
            let mut xy = XyTable::new_with_x_mode(x_mode, subcolumn_mode, parsed_y.len(), row_count)?;
            xy.x_title = x_title.to_owned();
            xy.row_titles = row_titles;
            xy.row_titles_expanded = has_row_titles;
            for (s, sub_cells) in x_cols.iter().enumerate() {
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
            grp.row_titles = row_titles;
            fill_datasets(&mut grp.data_sets, parsed_y);
            Ok(TableArchetype::Grouped(grp))
        }
        TableKind::Contingency => {
            let mut cont = ContingencyTable::new(parsed_y.len(), row_count)?;
            cont.row_titles = row_titles;
            fill_datasets(&mut cont.data_sets, parsed_y);
            Ok(TableArchetype::Contingency(cont))
        }
        TableKind::Survival => {
            let mut surv = SurvivalTable::new(parsed_y.len(), row_count)?;
            surv.time_title = x_title.to_owned();
            surv.row_titles = row_titles;
            surv.row_titles_expanded = has_row_titles;
            if let Some(first_x) = x_cols.first() {
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
                if let Some(first_sub) = col.subcolumns.first() {
                    for (r, cell) in first_sub.iter().enumerate() {
                        if let Some(slot) = var.cells.get_mut(r) {
                            *slot = cell.clone();
                        }
                    }
                }
                var.refresh_inferred_role();
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

fn resolve_x_mode(table_el: &XmlElement, x_cols: &[Vec<Cell>], x_declared_subs: Option<usize>) -> Result<XColumnMode, FormatError> {
    let x_fmt = table_el.attr("XFormat").unwrap_or("").trim();
    if x_fmt.eq_ignore_ascii_case("date") || x_fmt.eq_ignore_ascii_case("time") || x_fmt.eq_ignore_ascii_case("elapsed-time") {
        let unit_label = table_el
            .first_child("XColumn")
            .and_then(|el| el.first_child("Title"))
            .map(XmlElement::plain_text)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| String::from("Time"));
        return Ok(XColumnMode::DateOrElapsedTime { unit_label });
    }
    if x_fmt.eq_ignore_ascii_case("start-step") || x_fmt.eq_ignore_ascii_case("sequence") {
        let start = table_el.attr("X0").and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0);
        let step = table_el.attr("DeltaX").and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(1.0);
        return Ok(XColumnMode::GeneratedSequence { start, step, progression: SequenceProgression::Arithmetic });
    }
    if x_fmt.eq_ignore_ascii_case("replicates") {
        let count = x_cols.len().max(x_declared_subs.unwrap_or(0)).max(1);
        validate_subcolumns(count)?;
        return Ok(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(count) });
    }
    if x_cols.len() == 3 || (x_fmt.eq_ignore_ascii_case("err") && x_cols.len() >= 3) {
        let sub_mode = match table_el.attr("XEVFormat").unwrap_or("").trim() {
            "SEM" => SubcolumnMode::MeanSemN,
            "CV" => SubcolumnMode::MeanCvN,
            _ => SubcolumnMode::MeanSdN,
        };
        return Ok(XColumnMode::Numbers { subcolumn_mode: sub_mode });
    }
    Ok(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single })
}

fn fill_datasets(target: &mut [DataSetColumn], parsed_y: &[ParsedYColumn]) {
    for (c_idx, col) in parsed_y.iter().enumerate() {
        let Some(ds) = target.get_mut(c_idx) else {
            continue;
        };
        ds.title = col.title.clone();
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

fn parse_pzfx_cell(d_el: &XmlElement) -> Cell {
    let excluded = d_el.attr("Excluded").is_some_and(|v| v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"));
    let raw = d_el.plain_text();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Cell { value: CellValue::Empty, excluded };
    }
    if let Ok(num) = trimmed.parse::<f64>()
        && num.is_finite()
    {
        return Cell { value: CellValue::Number(num), excluded };
    }
    Cell { value: CellValue::Text(raw), excluded }
}

fn extract_subcolumn_cells(sub_el: &XmlElement) -> Vec<Cell> {
    sub_el.children_named("d").map(parse_pzfx_cell).collect()
}

fn extract_raw_subcolumn_strings(col_el: &XmlElement) -> Vec<Vec<String>> {
    col_el.children_named("Subcolumn").map(|sub_el| sub_el.children_named("d").map(XmlElement::plain_text).collect()).collect()
}

fn extract_labeled_subcolumns(col_el: &XmlElement, source_id: &str, warnings: &mut Vec<ImportWarning>) -> (Vec<Option<String>>, Vec<Vec<Cell>>) {
    let mut titles = Vec::new();
    let mut subcolumns = Vec::new();
    for sub_el in col_el.children_named("Subcolumn") {
        let sub_title = sub_el
            .first_child("Title")
            .map(XmlElement::plain_text)
            .or_else(|| sub_el.attr("Title").map(str::to_owned))
            .filter(|s| !s.trim().is_empty())
            .map(|raw| clamp_title(&raw, source_id, warnings));
        titles.push(sub_title);
        subcolumns.push(extract_subcolumn_cells(sub_el));
    }
    (titles, subcolumns)
}

fn apply_nested_subcolumn_titles(nested: &mut NestedTable, nested_sub_titles: &[Vec<Option<String>>]) {
    for (g_idx, grp) in nested.groups.iter_mut().enumerate() {
        let Some(titles) = nested_sub_titles.get(g_idx) else {
            continue;
        };
        for (s_idx, maybe_title) in titles.iter().enumerate() {
            if let Some(sub_title) = maybe_title
                && let Some(slot) = grp.subcolumn_titles.get_mut(s_idx)
            {
                *slot = sub_title.clone();
            }
        }
    }
}

fn extract_column_subcolumns(col_el: &XmlElement) -> Vec<Vec<Cell>> {
    col_el.children_named("Subcolumn").map(extract_subcolumn_cells).collect()
}

fn resolve_table_kind(raw: Option<&str>, id: &str, warnings: &mut Vec<ImportWarning>) -> TableKind {
    match raw.map(str::trim).unwrap_or("") {
        "XY" | "xy" => TableKind::Xy,
        "OneWay" | "Column" | "column" | "" => TableKind::Column,
        "TwoWay" | "Grouped" | "grouped" => TableKind::Grouped,
        "Contingency" | "contingency" => TableKind::Contingency,
        "Survival" | "survival" => TableKind::Survival,
        "PartsOfWhole" | "parts-of-whole" => TableKind::PartsOfWhole,
        "MultipleVariables" | "multiple-variables" => TableKind::MultipleVariables,
        "Nested" | "nested" => TableKind::Nested,
        other => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedTableType,
                id,
                format!("unrecognized TableType `{other}`, defaulting to Column"),
            ));
            TableKind::Column
        }
    }
}

fn resolve_subcolumn_mode(
    kind: TableKind,
    raw_fmt: Option<&str>,
    replicates: usize,
    id: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<SubcolumnMode, FormatError> {
    let fmt = raw_fmt.map(str::trim).unwrap_or("");
    let candidate = match fmt {
        "SD" | "MeanSDN" => SubcolumnMode::MeanSdN,
        "SEM" | "MeanSemN" => SubcolumnMode::MeanSemN,
        "CV" | "MeanCvN" => SubcolumnMode::MeanCvN,
        "SDN" | "low-high" | "MeanLowHigh" => SubcolumnMode::MeanLowHigh,
        "upper-lower" | "UpperLowerLimits" => SubcolumnMode::UpperLowerLimits,
        "median-iqr" | "MedianIqr" | "IQR" => SubcolumnMode::MedianIqr,
        "replicates" => SubcolumnMode::RawReplicates(replicates.max(1)),
        "" | "none" => {
            if matches!(kind, TableKind::Grouped | TableKind::Nested) {
                SubcolumnMode::RawReplicates(replicates.max(1))
            } else if kind == TableKind::Xy && replicates > 1 {
                SubcolumnMode::RawReplicates(replicates)
            } else {
                if replicates > 1 {
                    warnings.push(ImportWarning::new(
                        ImportWarningKind::CoercedValue,
                        id,
                        format!("Replicates=\"{replicates}\" is not valid for {kind:?} without a summary YFormat, using Single"),
                    ));
                }
                SubcolumnMode::Single
            }
        }
        other => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::CoercedValue,
                id,
                format!("unrecognized YFormat `{other}`, falling back to default subcolumn mode"),
            ));
            if matches!(kind, TableKind::Grouped | TableKind::Nested) {
                SubcolumnMode::RawReplicates(replicates.max(1))
            } else {
                SubcolumnMode::Single
            }
        }
    };

    if kind.allows_subcolumn_mode(candidate) {
        candidate.validate()?;
        Ok(candidate)
    } else if matches!(kind, TableKind::Grouped | TableKind::Nested) {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            id,
            format!("subcolumn mode {candidate:?} is not valid for {kind:?}, using RawReplicates"),
        ));
        Ok(SubcolumnMode::RawReplicates(replicates.max(1)))
    } else {
        warnings.push(ImportWarning::new(
            ImportWarningKind::CoercedValue,
            id,
            format!("subcolumn mode {candidate:?} is not valid for {kind:?}, using Single"),
        ));
        Ok(SubcolumnMode::Single)
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

fn clamp_title(raw: &str, source: &str, warnings: &mut Vec<ImportWarning>) -> String {
    if raw.len() <= MAX_TITLE_BYTES {
        return raw.to_owned();
    }
    warnings.push(ImportWarning::new(ImportWarningKind::TruncatedTitle, source, format!("title exceeded {MAX_TITLE_BYTES} bytes and was truncated")));
    truncate_utf8_to_bytes(raw, MAX_TITLE_BYTES).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let kind = resolve_table_kind(table_el.attr("TableType"), "t", &mut Vec::new());
            let reps = table_el.attr("Replicates").and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(1);
            let mode = resolve_subcolumn_mode(kind, table_el.attr("YFormat"), reps, "t", &mut Vec::new()).unwrap();
            let row_titles = table_el
                .first_child("RowTitlesColumn")
                .map(|el| extract_column_subcolumns(el).into_iter().next().unwrap_or_default())
                .unwrap_or_default();
            let x_cols = table_el.first_child("XColumn").map(extract_column_subcolumns).unwrap_or_default();
            let x_mode = resolve_x_mode(table_el, &x_cols, None).unwrap();
            let parsed_y: Vec<ParsedYColumn> = table_el
                .children_named("YColumn")
                .enumerate()
                .map(|(i, y_el)| ParsedYColumn {
                    title: default_column_label(i),
                    declared_subcolumns: None,
                    sub_titles: Vec::new(),
                    subcolumns: extract_column_subcolumns(y_el),
                })
                .collect();
            let max_y = parsed_y.iter().flat_map(|c| c.subcolumns.iter().map(Vec::len)).max().unwrap_or(0);
            let max_x = x_cols.iter().map(Vec::len).max().unwrap_or(0);
            let rows = row_titles.len().max(max_x).max(max_y);
            let est = estimate_table_cells(kind, mode, &x_mode, &parsed_y, rows).unwrap();
            let arch = build_table_archetype(kind, mode, x_mode, "X", &x_cols, &parsed_y, vec![String::new(); rows], false, rows).unwrap();
            assert_eq!(est, arch.allocated_cell_slots(), "estimate mismatch for {kind:?}");
            total_slots += arch.allocated_cell_slots();
        }
        // xy-sd: 3*(3+4)=21, xy-seq: 4*(0+1)=4, surv: 3*(1+2)=9, col: 1*3=3, grp: 2*3=6, cont: 2*2=4, pow: 2*1=2, mv: 3*2=6, nest: 2*(2+1)=6
        assert_eq!(total_slots, 61);
    }

    #[test]
    fn estimate_table_cells_rejects_usize_overflow_cleanly() {
        let parsed_y: Vec<ParsedYColumn> = (0..1_024)
            .map(|_| ParsedYColumn { title: String::from("A"), declared_subcolumns: None, sub_titles: Vec::new(), subcolumns: vec![vec![]] })
            .collect();
        let x_mode = XColumnMode::default();
        // On 64-bit, `usize::MAX` rows overflows checked_mul cleanly without panic.
        let err = estimate_table_cells(TableKind::Xy, SubcolumnMode::RawReplicates(512), &x_mode, &parsed_y, usize::MAX).unwrap_err();
        assert!(matches!(err, FormatError::XmlParse { offset: 0, .. }));

        let surv_err = estimate_table_cells(TableKind::Survival, SubcolumnMode::Single, &x_mode, &parsed_y, usize::MAX).unwrap_err();
        assert!(matches!(surv_err, FormatError::XmlParse { offset: 0, .. }));
    }

    #[test]
    fn r1_s06_04_widens_compatible_tables_and_warns_when_subcolumns_discarded() {
        // 1. OneWay (Column) cannot hold RawReplicates(2): keeps subcolumn 0 and emits CoercedValue naming table, column, and dropped count.
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

        // 2. XY without YFormat / Replicates widens cleanly to RawReplicates(k) from Subcolumns="3" / 3 <Subcolumn>s.
        let xy_xml = r#"<?xml version="1.0"?><PrismFile><TableSequence><Table ID="t-xy" TableType="XY"><Title>Curve</Title>
<XColumn><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>
<YColumn Subcolumns="3"><Title>Resp</Title><Subcolumn><d>10</d><d>20</d></Subcolumn><Subcolumn><d>11</d><d>21</d></Subcolumn><Subcolumn><d>12</d><d>22</d></Subcolumn></YColumn></Table></TableSequence></PrismFile>"#;
        let xy_rep = import_pzfx_str(xy_xml).unwrap();
        assert!(xy_rep.warnings.is_empty(), "expected lossless widening without warning, got {:?}", xy_rep.warnings);
        let TableArchetype::Xy(xy) = &xy_rep.document.data_sheets[0].archetype else { panic!("expected XY") };
        assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(3));
        assert_eq!(xy.data_sets[0].cell(1, 2).unwrap().active_number(), Some(22.0));

        // 3. Grouped (TwoWay) without Replicates widens cleanly to RawReplicates(k).
        let grp_xml = r#"<?xml version="1.0"?><PrismFile><TableSequence><Table ID="t-grp" TableType="TwoWay"><Title>Grouped</Title>
<YColumn Subcolumns="2"><Title>Drug</Title><Subcolumn><d>5</d></Subcolumn><Subcolumn><d>6</d></Subcolumn></YColumn></Table></TableSequence></PrismFile>"#;
        let grp_rep = import_pzfx_str(grp_xml).unwrap();
        assert!(grp_rep.warnings.is_empty(), "expected lossless widening without warning, got {:?}", grp_rep.warnings);
        let TableArchetype::Grouped(grp) = &grp_rep.document.data_sheets[0].archetype else { panic!("expected Grouped") };
        assert_eq!(grp.subcolumn_mode, SubcolumnMode::RawReplicates(2));
        assert_eq!(grp.data_sets[0].cell(0, 1).unwrap().active_number(), Some(6.0));

        // 4. XY with two X subcolumns (no XFormat) emits CoercedValue naming XColumn and discarded subcolumn;
        //    MultipleVariables with >1 subcolumn emits CoercedValue naming the variable column.
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

        // 5. OneWay without Subcolumns attr still warns when >1 <Subcolumn> exists; XY without Subcolumns attr still widens;
        //    pre-summarized YFormat="SD" with 4 subcolumns warns on the 4th; RowTitlesColumn with 2 subcolumns warns.
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

        // 6. Subcolumns exceeding MAX_SUBCOLUMNS rejected cleanly, and widened grid exceeding MAX_IMPORT_CELLS rejected before allocation.
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
