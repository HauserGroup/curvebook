//! MCP `resources/list` and `resources/read` handlers exposing active `Session` sheets and results.

use curvebook_doc::{
    ColumnTable, ContingencyTable, DataSetColumn, DataSheet, GroupedTable, MultipleVariablesTable, NestedGroupColumn, NestedTable, PartsOfWholeTable,
    ProjectDocument, ResultTab, SheetId, SheetSection, SurvivalTable, TableArchetype, VariableColumn, XyTable,
};
use curvebook_format::{CsvOptions, export_datasheet_to_csv, export_result_sheet_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const MAX_CSV_PREVIEW_ROWS: usize = 100;
pub(crate) const MAX_CSV_PREVIEW_BYTES: usize = 65_536;
pub(crate) const MAX_RESULT_PREVIEW_TABS: usize = 32;
pub(crate) const MAX_RESULT_PREVIEW_ROWS: usize = 100;
pub(crate) const MAX_RESULT_PREVIEW_COLS: usize = 64;
pub(crate) const MAX_RESULT_PREVIEW_TOTAL_CELLS: usize = 4_096;
pub(crate) const MAX_RESULT_PREVIEW_FOOTNOTES: usize = 64;
pub(crate) const MAX_RESULT_PREVIEW_CHECKLIST: usize = 128;
pub(crate) const MAX_RESULT_PREVIEW_SCALARS: usize = 2_048;

/// Resource entry returned inside MCP `resources/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDescriptor {
    pub uri: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

/// Content entry returned inside MCP `resources/read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub text: String,
}

/// Failure modes for MCP resource discovery and lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceError {
    /// URI or request parameter syntax was invalid (`-32602`).
    InvalidUri(String),
    /// Requested sheet or result resource does not exist (`-32002`).
    NotFound(String),
    /// Serialization error while formatting resource contents (`-32603`).
    Internal(String),
}

/// Enumerate all `sheet://<id>` and `result://<id>` resources in `doc`.
#[must_use]
pub fn list_resources(doc: &ProjectDocument) -> Vec<ResourceDescriptor> {
    let mut out = Vec::with_capacity(doc.total_sheet_count() + doc.result_sheets.len());

    for ds in &doc.data_sheets {
        out.push(ResourceDescriptor {
            uri: format!("sheet://{}", ds.id.as_str()),
            name: ds.title.clone(),
            description: format!("{} data sheet ({} rows x {} cols)", ds.kind().label(), ds.archetype.row_count(), ds.archetype.column_count()),
            mime_type: String::from("application/json"),
        });
    }
    for info in &doc.info_sheets {
        out.push(ResourceDescriptor {
            uri: format!("sheet://{}", info.id.as_str()),
            name: info.title.clone(),
            description: format!("Info sheet ({} constants)", info.constants.len()),
            mime_type: String::from("application/json"),
        });
    }
    for rs in &doc.result_sheets {
        out.push(ResourceDescriptor {
            uri: format!("sheet://{}", rs.id.as_str()),
            name: rs.title().to_owned(),
            description: format!("Result sheet summary ({} tabs, {} scalars)", rs.tabs.len(), rs.scalars.len()),
            mime_type: String::from("application/json"),
        });
        out.push(ResourceDescriptor {
            uri: format!("result://{}", rs.id.as_str()),
            name: format!("{} (Result)", rs.title()),
            description: format!("Canonical statistical/regression JSON output for '{}'", rs.id.as_str()),
            mime_type: String::from("application/json"),
        });
    }
    for gs in &doc.graph_sheets {
        out.push(ResourceDescriptor {
            uri: format!("sheet://{}", gs.id.as_str()),
            name: gs.title.clone(),
            description: format!("Graph sheet ({} series)", gs.spec.series.len()),
            mime_type: String::from("application/json"),
        });
    }
    for ls in &doc.layout_sheets {
        out.push(ResourceDescriptor {
            uri: format!("sheet://{}", ls.id.as_str()),
            name: ls.title.clone(),
            description: format!("Layout sheet ({} items)", ls.spec.items.len()),
            mime_type: String::from("application/json"),
        });
    }

    out
}

/// Read a single `sheet://<id>` or `result://<id>` resource from `doc`.
pub fn read_resource(doc: &ProjectDocument, uri: &str) -> Result<Vec<ResourceContent>, ResourceError> {
    let trimmed = uri.trim();
    if let Some(sheet_id_raw) = trimmed.strip_prefix("sheet://") {
        if sheet_id_raw.trim().is_empty() {
            return Err(ResourceError::InvalidUri(String::from("sheet:// URI requires a non-empty sheet id")));
        }
        return read_sheet_resource(doc, trimmed, sheet_id_raw);
    }
    if let Some(result_id_raw) = trimmed.strip_prefix("result://") {
        if result_id_raw.trim().is_empty() {
            return Err(ResourceError::InvalidUri(String::from("result:// URI requires a non-empty result sheet id")));
        }
        return read_result_resource(doc, trimmed, result_id_raw);
    }
    Err(ResourceError::InvalidUri(format!("unsupported resource URI scheme '{trimmed}'; expected sheet://<id> or result://<id>")))
}

fn slice_dataset_columns(cols: &[DataSetColumn], max_rows: usize) -> Vec<DataSetColumn> {
    cols.iter().map(|c| DataSetColumn { title: c.title.clone(), cells: c.cells.iter().take(max_rows).cloned().collect() }).collect()
}

fn slice_archetype_rows(archetype: &TableArchetype, max_rows: usize) -> TableArchetype {
    match archetype {
        TableArchetype::Xy(t) => TableArchetype::Xy(XyTable {
            x_title: t.x_title.clone(),
            x_mode: t.x_mode.clone(),
            x_cells: t.x_cells.iter().take(max_rows).cloned().collect(),
            row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
            row_titles_expanded: t.row_titles_expanded,
            y_subcolumn_mode: t.y_subcolumn_mode,
            data_sets: slice_dataset_columns(&t.data_sets, max_rows),
        }),
        TableArchetype::Column(t) => TableArchetype::Column(ColumnTable {
            design_hint: t.design_hint,
            subcolumn_mode: t.subcolumn_mode,
            row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
            row_titles_expanded: t.row_titles_expanded,
            data_sets: slice_dataset_columns(&t.data_sets, max_rows),
        }),
        TableArchetype::Grouped(t) => TableArchetype::Grouped(GroupedTable {
            row_factor_title: t.row_factor_title.clone(),
            column_factor_title: t.column_factor_title.clone(),
            row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
            row_numeric_spacing: t.row_numeric_spacing.as_ref().map(|v| v.iter().take(max_rows).copied().collect()),
            subcolumn_mode: t.subcolumn_mode,
            replicate_stacking: t.replicate_stacking,
            three_way_grouping: t.three_way_grouping.clone(),
            data_sets: slice_dataset_columns(&t.data_sets, max_rows),
        }),
        TableArchetype::Contingency(t) => TableArchetype::Contingency(ContingencyTable {
            row_factor_title: t.row_factor_title.clone(),
            column_factor_title: t.column_factor_title.clone(),
            row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
            data_sets: slice_dataset_columns(&t.data_sets, max_rows),
        }),
        TableArchetype::Survival(t) => TableArchetype::Survival(SurvivalTable {
            time_title: t.time_title.clone(),
            times: t.times.iter().take(max_rows).cloned().collect(),
            event_code: t.event_code,
            censored_code: t.censored_code,
            row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
            row_titles_expanded: t.row_titles_expanded,
            cohorts: slice_dataset_columns(&t.cohorts, max_rows),
        }),
        TableArchetype::PartsOfWhole(t) => TableArchetype::PartsOfWhole(PartsOfWholeTable {
            slice_labels: t.slice_labels.iter().take(max_rows).cloned().collect(),
            data_sets: slice_dataset_columns(&t.data_sets, max_rows),
        }),
        TableArchetype::MultipleVariables(t) => TableArchetype::MultipleVariables(MultipleVariablesTable {
            row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
            row_titles_expanded: t.row_titles_expanded,
            variables: t
                .variables
                .iter()
                .map(|v| VariableColumn {
                    title: v.title.clone(),
                    role: v.role,
                    role_inferred: v.role_inferred,
                    source: v.source.clone(),
                    level_order: v.level_order.clone(),
                    cells: v.cells.iter().take(max_rows).cloned().collect(),
                })
                .collect(),
        }),
        TableArchetype::Nested(t) => TableArchetype::Nested(NestedTable {
            row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
            row_titles_expanded: t.row_titles_expanded,
            groups: t
                .groups
                .iter()
                .map(|g| NestedGroupColumn {
                    title: g.title.clone(),
                    subcolumn_titles: g.subcolumn_titles.clone(),
                    cells: g.cells.iter().take(max_rows).cloned().collect(),
                })
                .collect(),
        }),
    }
}

fn truncate_csv_bytes(mut csv: String, max_bytes: usize) -> (String, bool) {
    if csv.len() <= max_bytes {
        return (csv, false);
    }
    let mut end = max_bytes;
    while end > 0 && !csv.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(prefix) = csv.get(..end)
        && let Some(nl) = prefix.rfind('\n')
        && nl > 0
    {
        end = nl + 1;
    }
    csv.truncate(end);
    (csv, true)
}

pub(crate) fn build_csv_preview(ds: &DataSheet) -> Result<(String, bool), ResourceError> {
    ds.archetype.validate().map_err(|err| ResourceError::Internal(format!("CSV preview export failed: {err}")))?;
    let row_count = ds.archetype.row_count();
    let row_truncated = row_count > MAX_CSV_PREVIEW_ROWS;
    let raw_csv = if row_truncated {
        let sliced = slice_archetype_rows(&ds.archetype, MAX_CSV_PREVIEW_ROWS);
        let preview_ds = DataSheet::new(ds.id.clone(), ds.title.clone(), sliced)
            .map_err(|err| ResourceError::Internal(format!("CSV preview export failed: {err}")))?;
        export_datasheet_to_csv(&preview_ds, &CsvOptions::csv())
    } else {
        export_datasheet_to_csv(ds, &CsvOptions::csv())
    }
    .map_err(|err| ResourceError::Internal(format!("CSV preview export failed: {err}")))?;
    let (capped, byte_truncated) = truncate_csv_bytes(raw_csv, MAX_CSV_PREVIEW_BYTES);
    Ok((capped, row_truncated || byte_truncated))
}

pub(crate) fn slice_preview_vec<T: Clone>(items: &[T], limit: usize) -> (Vec<T>, bool) {
    let truncated = items.len() > limit;
    (items.iter().take(limit).cloned().collect(), truncated)
}

pub(crate) fn slice_result_tabs_preview(tabs: &[ResultTab]) -> (Vec<ResultTab>, bool) {
    let mut truncated = tabs.len() > MAX_RESULT_PREVIEW_TABS;
    let mut remaining_cells = MAX_RESULT_PREVIEW_TOTAL_CELLS;
    let mut out = Vec::with_capacity(tabs.len().min(MAX_RESULT_PREVIEW_TABS));

    for tab in tabs.iter().take(MAX_RESULT_PREVIEW_TABS) {
        if tab.column_headers.len() > MAX_RESULT_PREVIEW_COLS || tab.rows.len() > MAX_RESULT_PREVIEW_ROWS {
            truncated = true;
        }
        let column_headers: Vec<String> = tab.column_headers.iter().take(MAX_RESULT_PREVIEW_COLS).cloned().collect();
        let mut out_rows = Vec::new();
        let mut out_row_labels = Vec::new();
        for (r_idx, row) in tab.rows.iter().take(MAX_RESULT_PREVIEW_ROWS).enumerate() {
            if row.len() > MAX_RESULT_PREVIEW_COLS {
                truncated = true;
            }
            let take_cols = row.len().min(MAX_RESULT_PREVIEW_COLS);
            if take_cols > remaining_cells {
                truncated = true;
                break;
            }
            remaining_cells -= take_cols;
            if let Some(row_slice) = row.get(..take_cols) {
                out_rows.push(row_slice.to_vec());
            }
            if let Some(lbl) = tab.row_labels.get(r_idx) {
                out_row_labels.push(lbl.clone());
            }
        }
        if tab.row_labels.len() > out_rows.len() || tab.footnotes.len() > MAX_RESULT_PREVIEW_FOOTNOTES {
            truncated = true;
        }
        let footnotes = tab.footnotes.iter().take(MAX_RESULT_PREVIEW_FOOTNOTES).cloned().collect();
        out.push(ResultTab {
            id: tab.id.clone(),
            title: tab.title.clone(),
            kind: tab.kind,
            column_headers,
            row_labels: out_row_labels,
            rows: out_rows,
            footnotes,
        });
    }

    (out, truncated)
}

fn read_sheet_resource(doc: &ProjectDocument, uri: &str, id_str: &str) -> Result<Vec<ResourceContent>, ResourceError> {
    let target = SheetId::new(id_str);
    let deps = doc.upstream_dependencies(&target);
    let Some(section) = doc.sheet_section(&target) else {
        return Err(ResourceError::NotFound(format!("resource not found: {uri}")));
    };

    let payload: Option<Value> = match section {
        SheetSection::Data => match doc.data_sheet(&target) {
            Some(ds) => {
                let (csv_preview, csv_preview_truncated) = build_csv_preview(ds)?;
                Some(json!({
                    "uri": uri,
                    "section": "data",
                    "id": ds.id,
                    "title": ds.title,
                    "kind": ds.kind().label(),
                    "frozen": ds.frozen,
                    "info_sheet_id": ds.info_sheet_id,
                    "rows": ds.archetype.row_count(),
                    "columns": ds.archetype.column_count(),
                    "csv_preview": csv_preview,
                    "csv_preview_truncated": csv_preview_truncated,
                    "linked_regions": ds.linked_regions,
                    "upstream_dependencies": deps,
                }))
            }
            None => None,
        },
        SheetSection::Info => doc.info_sheet(&target).map(|info| {
            json!({
                "uri": uri,
                "section": "info",
                "id": info.id,
                "title": info.title,
                "linked_data_sheets": info.linked_data_sheets,
                "constants": info.constants,
                "notes_markdown": info.notes_markdown,
                "upstream_dependencies": deps,
            })
        }),
        SheetSection::Analyses => doc.result_sheet(&target).map(|rs| {
            let (tabs_preview, tabs_truncated) = slice_result_tabs_preview(&rs.tabs);
            let (scalars_preview, scalars_truncated) = slice_preview_vec(&rs.scalars, MAX_RESULT_PREVIEW_SCALARS);
            let (checklist_preview, checklist_truncated) = slice_preview_vec(&rs.checklist, MAX_RESULT_PREVIEW_CHECKLIST);
            let mut payload = json!({
                "uri": uri,
                "section": "analyses",
                "id": rs.id,
                "title": rs.title(),
                "source_sheet_id": rs.analysis.source_sheet_id,
                "selected_data_sets": rs.analysis.selected_data_sets,
                "status": rs.status,
                "has_analyzable_data": rs.analyzable_data.is_some(),
                "tabs": tabs_preview,
                "tabs_truncated": tabs_truncated,
                "scalars": scalars_preview,
                "scalars_truncated": scalars_truncated,
                "pairwise_comparisons": rs.pairwise_comparisons,
                "fitted_curves": rs.fitted_curves,
                "checklist": checklist_preview,
                "checklist_truncated": checklist_truncated,
                "upstream_dependencies": deps,
            });
            if matches!(&rs.analysis.kind, curvebook_doc::AnalysisKind::NonlinearRegression(nl) if nl.method == curvebook_doc::FitMethod::RobustCauchy)
                && let Some(obj) = payload.as_object_mut()
            {
                obj.insert(String::from("inference_availability"), Value::String(String::from("NotApplicableInRobustMode")));
            }
            payload
        }),
        SheetSection::Graphs => doc.graph_sheet(&target).map(|gs| {
            json!({
                "uri": uri,
                "section": "graphs",
                "id": gs.id,
                "title": gs.title,
                "initialized": gs.initialized,
                "spec": gs.spec,
                "upstream_dependencies": deps,
            })
        }),
        SheetSection::Layouts => doc.layout_sheet(&target).map(|ls| {
            json!({
                "uri": uri,
                "section": "layouts",
                "id": ls.id,
                "title": ls.title,
                "spec": ls.spec,
                "upstream_dependencies": deps,
            })
        }),
    };

    let Some(val) = payload else {
        return Err(ResourceError::NotFound(format!("resource not found: {uri}")));
    };
    let text = serde_json::to_string_pretty(&val).map_err(|err| ResourceError::Internal(format!("JSON serialization failed: {err}")))?;
    Ok(vec![ResourceContent { uri: uri.to_owned(), mime_type: String::from("application/json"), text }])
}

fn read_result_resource(doc: &ProjectDocument, uri: &str, id_str: &str) -> Result<Vec<ResourceContent>, ResourceError> {
    let target = SheetId::new(id_str);
    let Some(rs) = doc.result_sheet(&target) else {
        return Err(ResourceError::NotFound(format!("resource not found: {uri}")));
    };
    let text = export_result_sheet_json(rs, true).map_err(|err| ResourceError::Internal(format!("result JSON export failed: {err}")))?;
    Ok(vec![ResourceContent { uri: uri.to_owned(), mime_type: String::from("application/json"), text }])
}
