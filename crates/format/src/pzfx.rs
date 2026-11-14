//! Clean-room `.pzfx` XML project importer.
//!
//! Parses scientific XML tables, replicate subcolumns, row/column titles (preserving nested
//! formatting text), excluded-cell masks (`Excluded="1"`), and Info constants into a
//! [`ProjectDocument`] alongside non-fatal [`ImportWarning`] diagnostics.

use std::collections::HashSet;

use lumencraft_doc::{
    Cell, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSetColumn, DataSheet, GroupedTable, InfoConstant, InfoSheet,
    MAX_TITLE_BYTES, MultipleVariablesTable, NestedGroupColumn, NestedTable, PartsOfWholeTable, ProjectDocument, SequenceProgression, SheetId,
    SubcolumnMode, SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable, default_column_label, validate_data_sets, validate_rows,
    validate_subcolumns,
};

use crate::archive::validate_project_document;
use crate::csv::{CsvOptions, parse_cell_token};
use crate::error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
use crate::xml::{XmlElement, parse_xml};

/// Maximum total dense `Cell` entries allocated across all tables of a single PZFX import.
pub const MAX_IMPORT_CELLS: usize = crate::csv::MAX_IMPORT_CELLS;

/// Import a `.pzfx` XML byte buffer into a [`ProjectDocument`] and compatibility warnings.
pub fn import_pzfx_bytes(bytes: &[u8]) -> Result<ImportReport, FormatError> {
    let text = std::str::from_utf8(bytes).map_err(|err| FormatError::XmlParse { offset: 0, reason: format!("invalid UTF-8 in XML: {err}") })?;
    import_pzfx_str(text)
}

/// Import a `.pzfx` XML string into a [`ProjectDocument`] and compatibility warnings.
pub fn import_pzfx_str(xml: &str) -> Result<ImportReport, FormatError> {
    let root = parse_xml(xml)?;
    let local = root.local_name();
    if !(local == "PrismFile" || local.ends_with("PrismFile")) {
        return Err(FormatError::UnsupportedFormat { reason: format!("unexpected `.pzfx` root element `<{local}>`") });
    }

    let mut warnings = Vec::new();
    let title = root
        .first_child("Title")
        .map(|el| el.plain_text().trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| String::from("Imported Project"));
    let clamped_title = clamp_title(&title, "project", &mut warnings);
    let mut doc = ProjectDocument::new(clamped_title)?;
    let mut used_ids = HashSet::new();

    import_info_sections(&root, &mut doc, &mut used_ids, &mut warnings)?;
    import_table_sections(&root, &mut doc, &mut used_ids, &mut warnings)?;
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
        let id = allocate_unique_id(info_el.attr("ID"), "info", idx + 1, used_ids);
        let raw_title = info_el
            .first_child("Title")
            .map(|el| el.plain_text().trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Info {}", idx + 1));
        let title = clamp_title(&raw_title, &id, warnings);

        let mut sheet = InfoSheet::new_preseeded(id, title)?;
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
) -> Result<(), FormatError> {
    let mut table_nodes: Vec<&XmlElement> = Vec::new();
    for seq in root.children_named("TableSequence") {
        table_nodes.extend(seq.children_named("Table"));
    }
    table_nodes.extend(root.children_named("Table"));

    let primary_info = doc.info_sheets.first().map(|s| s.id.clone());
    let mut remaining_cells = MAX_IMPORT_CELLS;
    for (idx, table_el) in table_nodes.iter().enumerate() {
        let mut ds = parse_table_element(table_el, idx + 1, used_ids, warnings, &mut remaining_cells)?;
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
    remaining_cells: &mut usize,
) -> Result<DataSheet, FormatError> {
    let id = allocate_unique_id(table_el.attr("ID"), "data", one_based_idx, used_ids);
    let raw_title = table_el
        .first_child("Title")
        .map(|el| el.plain_text().trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Data {one_based_idx}"));
    let title = clamp_title(&raw_title, &id, warnings);

    let kind = resolve_table_kind(table_el.attr("TableType"), &id, warnings);
    let replicates = table_el.attr("Replicates").and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(1);
    if replicates > 0 {
        validate_subcolumns(replicates)?;
    }
    let y_format = table_el.attr("YFormat").or_else(|| table_el.attr("EVFormat"));
    let subcolumn_mode = resolve_subcolumn_mode(kind, y_format, replicates, &id, warnings)?;

    let row_titles_raw =
        table_el.first_child("RowTitlesColumn").map(|el| extract_column_subcolumns(el).into_iter().next().unwrap_or_default()).unwrap_or_default();
    let row_titles: Vec<String> = row_titles_raw
        .into_iter()
        .map(|c| match c.value {
            CellValue::Empty => String::new(),
            CellValue::Number(v) => format!("{v}"),
            CellValue::Text(s) => s,
        })
        .collect();

    let x_el = table_el.first_child("XColumn");
    let x_title = x_el
        .and_then(|el| el.first_child("Title"))
        .map(|el| clamp_title(el.plain_text().trim(), &id, warnings))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| String::from("X"));
    let x_cols = x_el.map(extract_column_subcolumns).unwrap_or_default();

    let y_elements: Vec<&XmlElement> =
        table_el.children_named("YColumn").chain(table_el.children_named("Column")).chain(table_el.children_named("VariableColumn")).collect();
    validate_data_sets(y_elements.len())?;

    let mut parsed_y: Vec<(String, Vec<Vec<Cell>>)> = Vec::with_capacity(y_elements.len());
    let mut nested_sub_titles: Vec<Vec<Option<String>>> = Vec::with_capacity(y_elements.len());
    for (c_idx, y_el) in y_elements.iter().enumerate() {
        let col_title = y_el
            .first_child("Title")
            .map(|el| clamp_title(el.plain_text().trim(), &id, warnings))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_column_label(c_idx));
        let (sub_titles, subs) = extract_labeled_subcolumns(y_el, &id, warnings);
        parsed_y.push((col_title, subs));
        nested_sub_titles.push(sub_titles);
    }

    let max_y_rows = parsed_y.iter().flat_map(|(_, subs)| subs.iter().map(Vec::len)).max().unwrap_or(0);
    let max_x_rows = x_cols.iter().map(Vec::len).max().unwrap_or(0);
    let row_count = row_titles.len().max(max_x_rows).max(max_y_rows);
    validate_rows(row_count)?;

    let table_cells = estimate_table_cells(kind, subcolumn_mode, table_el, &x_cols, &parsed_y, row_count)?;
    if table_cells > *remaining_cells {
        return Err(FormatError::XmlParse {
            offset: 0,
            reason: format!(
                "table `{id}` cell allocation ({table_cells}) exceeds remaining import budget ({remaining_cells}, cap {MAX_IMPORT_CELLS})"
            ),
        });
    }
    *remaining_cells -= table_cells;

    let mut padded_row_titles = row_titles;
    padded_row_titles.resize(row_count, String::new());
    let has_row_titles = padded_row_titles.iter().any(|s| !s.is_empty());

    let mut arch = build_table_archetype(kind, subcolumn_mode, table_el, &x_title, &x_cols, &parsed_y, padded_row_titles, has_row_titles, row_count)?;
    if let TableArchetype::Nested(ref mut nested) = arch {
        apply_nested_subcolumn_titles(nested, &nested_sub_titles);
    }

    Ok(DataSheet::new(SheetId::new(id), title, arch)?)
}

fn estimate_table_cells(
    kind: TableKind,
    subcolumn_mode: SubcolumnMode,
    table_el: &XmlElement,
    x_cols: &[Vec<Cell>],
    parsed_y: &[(String, Vec<Vec<Cell>>)],
    row_count: usize,
) -> Result<usize, FormatError> {
    let overflow = || FormatError::XmlParse { offset: 0, reason: format!("table cell count overflows budget cap of {MAX_IMPORT_CELLS}") };
    match kind {
        TableKind::Xy => {
            validate_subcolumns(subcolumn_mode.subcolumn_count())?;
            let x_subs = resolve_x_mode(table_el, x_cols).subcolumn_count();
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
            let max_units = parsed_y.iter().map(|(_, subs)| subs.len()).max().unwrap_or(1).max(1);
            validate_subcolumns(max_units)?;
            let mut total: usize = 0;
            for (_, subs) in parsed_y {
                let units = subs.len().max(1);
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
    table_el: &XmlElement,
    x_title: &str,
    x_cols: &[Vec<Cell>],
    parsed_y: &[(String, Vec<Vec<Cell>>)],
    row_titles: Vec<String>,
    has_row_titles: bool,
    row_count: usize,
) -> Result<TableArchetype, FormatError> {
    match kind {
        TableKind::Xy => {
            let x_mode = resolve_x_mode(table_el, x_cols);
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
                    surv.times[r] = cell.clone();
                }
            }
            fill_datasets(&mut surv.cohorts, parsed_y);
            Ok(TableArchetype::Survival(surv))
        }
        TableKind::PartsOfWhole => {
            let mut pow = PartsOfWholeTable::new(parsed_y.len(), row_count)?;
            pow.slice_labels = if has_row_titles {
                row_titles
            } else if let Some(first_x) = x_cols.first() {
                let mut labels: Vec<String> = first_x
                    .iter()
                    .map(|c| match &c.value {
                        CellValue::Empty => String::new(),
                        CellValue::Number(v) => format!("{v}"),
                        CellValue::Text(s) => s.clone(),
                    })
                    .collect();
                labels.resize(row_count, String::new());
                labels
            } else {
                vec![String::new(); row_count]
            };
            fill_datasets(&mut pow.data_sets, parsed_y);
            Ok(TableArchetype::PartsOfWhole(pow))
        }
        TableKind::MultipleVariables => {
            let mut mv = MultipleVariablesTable::new(parsed_y.len(), row_count)?;
            mv.row_titles = row_titles;
            mv.row_titles_expanded = has_row_titles;
            for (c_idx, (col_title, subs)) in parsed_y.iter().enumerate() {
                let var = &mut mv.variables[c_idx];
                var.title = col_title.clone();
                if let Some(first_sub) = subs.first() {
                    for (r, cell) in first_sub.iter().enumerate() {
                        var.cells[r] = cell.clone();
                    }
                }
                var.refresh_inferred_role();
            }
            Ok(TableArchetype::MultipleVariables(mv))
        }
        TableKind::Nested => {
            let max_units = parsed_y.iter().map(|(_, subs)| subs.len()).max().unwrap_or(1).max(1);
            validate_subcolumns(max_units)?;
            let mut nested = NestedTable::new(parsed_y.len(), 1, 0)?;
            nested.row_titles = row_titles;
            nested.row_titles_expanded = has_row_titles;
            for (g_idx, (col_title, subs)) in parsed_y.iter().enumerate() {
                let units = subs.len().max(1);
                let mut grp = NestedGroupColumn::new(col_title.clone(), units, row_count)?;
                for (s, sub_cells) in subs.iter().enumerate() {
                    for (r, cell) in sub_cells.iter().enumerate() {
                        grp.cells[r][s] = cell.clone();
                    }
                }
                nested.groups[g_idx] = grp;
            }
            Ok(TableArchetype::Nested(nested))
        }
    }
}

fn resolve_x_mode(table_el: &XmlElement, x_cols: &[Vec<Cell>]) -> XColumnMode {
    let x_fmt = table_el.attr("XFormat").unwrap_or("").trim();
    if x_fmt.eq_ignore_ascii_case("date") || x_fmt.eq_ignore_ascii_case("time") || x_fmt.eq_ignore_ascii_case("elapsed-time") {
        let unit_label = table_el
            .first_child("XColumn")
            .and_then(|el| el.first_child("Title"))
            .map(|el| el.plain_text().trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("Time"));
        return XColumnMode::DateOrElapsedTime { unit_label };
    }
    if x_fmt.eq_ignore_ascii_case("start-step") || x_fmt.eq_ignore_ascii_case("sequence") {
        let start = table_el.attr("X0").and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0);
        let step = table_el.attr("DeltaX").and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(1.0);
        return XColumnMode::GeneratedSequence { start, step, progression: SequenceProgression::Arithmetic };
    }
    if x_cols.len() == 3 {
        let sub_mode = match table_el.attr("XEVFormat").unwrap_or("").trim() {
            "SEM" => SubcolumnMode::MeanSemN,
            "CV" => SubcolumnMode::MeanCvN,
            _ => SubcolumnMode::MeanSdN,
        };
        return XColumnMode::Numbers { subcolumn_mode: sub_mode };
    }
    XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }
}

fn fill_datasets(target: &mut [DataSetColumn], parsed_y: &[(String, Vec<Vec<Cell>>)]) {
    for (c_idx, (col_title, subs)) in parsed_y.iter().enumerate() {
        let Some(ds) = target.get_mut(c_idx) else {
            continue;
        };
        ds.title = col_title.clone();
        let allowed_subs = ds.subcolumn_count();
        for (s, sub_cells) in subs.iter().enumerate() {
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

fn extract_subcolumn_cells(sub_el: &XmlElement, csv_opts: &CsvOptions) -> Vec<Cell> {
    let mut cells = Vec::new();
    for d_el in sub_el.children_named("d") {
        let excluded_attr = d_el.attr("Excluded").is_some_and(|v| v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"));
        let raw = d_el.plain_text();
        let mut cell = parse_cell_token(&raw, csv_opts);
        if excluded_attr {
            cell.excluded = true;
        }
        cells.push(cell);
    }
    cells
}

fn extract_labeled_subcolumns(col_el: &XmlElement, source_id: &str, warnings: &mut Vec<ImportWarning>) -> (Vec<Option<String>>, Vec<Vec<Cell>>) {
    let csv_opts = CsvOptions::csv();
    let mut titles = Vec::new();
    let mut subcolumns = Vec::new();
    for sub_el in col_el.children_named("Subcolumn") {
        let sub_title = sub_el
            .first_child("Title")
            .map(|el| clamp_title(el.plain_text().trim(), source_id, warnings))
            .or_else(|| sub_el.attr("Title").map(|s| clamp_title(s.trim(), source_id, warnings)))
            .filter(|s| !s.is_empty());
        titles.push(sub_title);
        subcolumns.push(extract_subcolumn_cells(sub_el, &csv_opts));
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
    let csv_opts = CsvOptions::csv();
    col_el.children_named("Subcolumn").map(|sub_el| extract_subcolumn_cells(sub_el, &csv_opts)).collect()
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

fn allocate_unique_id(raw: Option<&str>, prefix: &str, idx: usize, used: &mut HashSet<String>) -> String {
    let base = raw.map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| format!("{prefix}-{idx}"));
    let mut candidate = base.clone();
    let mut suffix = 1_usize;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    candidate
}

fn clamp_title(raw: &str, source: &str, warnings: &mut Vec<ImportWarning>) -> String {
    if raw.len() <= MAX_TITLE_BYTES {
        return raw.to_owned();
    }
    let mut end = MAX_TITLE_BYTES;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    warnings.push(ImportWarning::new(ImportWarningKind::TruncatedTitle, source, format!("title exceeded {MAX_TITLE_BYTES} bytes and was truncated")));
    raw[..end].to_owned()
}
