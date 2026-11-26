//! Project and sheet inspection summaries, bounded CSV/result previews, and JSON projections.

use std::path::Path;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ColumnTable, ContingencyTable, DataSetColumn, DataSheet, ErrorCode, GroupedTable, InfoConstant,
    MultipleVariablesTable, NestedGroupColumn, NestedTable, PartsOfWholeTable, ProjectDocument, ResultTab, SheetId, SheetSection, SurvivalTable,
    TableArchetype, VariableColumn, XyTable,
};
use curvebook_format::{CsvOptions, export_datasheet_to_csv};
use serde::Serialize;
use serde_json::{Value, json};

use crate::EngineError;

/// Maximum number of rows included in bounded CSV sheet previews.
pub const MAX_CSV_PREVIEW_ROWS: usize = 100;
/// Maximum UTF-8 byte length for bounded CSV sheet previews.
pub const MAX_CSV_PREVIEW_BYTES: usize = 65_536;
/// Maximum number of result tabs included in bounded result previews.
pub const MAX_RESULT_PREVIEW_TABS: usize = 32;
/// Maximum number of rows per result tab in bounded result previews.
pub const MAX_RESULT_PREVIEW_ROWS: usize = 100;
/// Maximum number of columns per result tab in bounded result previews.
pub const MAX_RESULT_PREVIEW_COLS: usize = 64;
/// Maximum total result cells across all previewed tabs.
pub const MAX_RESULT_PREVIEW_TOTAL_CELLS: usize = 4_096;
/// Maximum footnotes per result tab in bounded result previews.
pub const MAX_RESULT_PREVIEW_FOOTNOTES: usize = 64;
/// Maximum diagnostic checklist entries in bounded result previews.
pub const MAX_RESULT_PREVIEW_CHECKLIST: usize = 128;
/// Maximum scalar hook entries in bounded result previews.
pub const MAX_RESULT_PREVIEW_SCALARS: usize = 2_048;

/// Focused section and upstream dependency metadata for a single requested sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SheetFocusSummary {
    /// Focused sheet identifier.
    pub sheet_id: String,
    /// Debug name of the [`SheetSection`].
    pub section: String,
    /// Upstream sheet identifiers in dependency order.
    pub upstream_dependencies: Vec<String>,
}

/// Summary metadata for a data sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Human-readable sheet title.
    pub title: String,
    /// Debug representation of the table archetype kind (e.g. `"Xy"`).
    pub kind_debug: String,
    /// Display label of the table archetype kind (e.g. `"XY"`).
    pub kind_label: String,
    /// Whether edits to this sheet are frozen.
    pub frozen: bool,
    /// Number of rows.
    pub rows: usize,
    /// Number of columns.
    pub cols: usize,
    /// Total allocated cell slots.
    pub allocated_cells: usize,
    /// Optional linked info sheet identifier.
    pub info_sheet_id: Option<String>,
    /// Number of linked import regions.
    pub linked_regions: usize,
    /// Upstream sheet identifiers.
    pub upstream_dependencies: Vec<String>,
}

/// Summary metadata for an info sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InfoSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Human-readable sheet title.
    pub title: String,
    /// Byte length of the markdown notes body.
    pub notes_len: usize,
    /// Linked data sheet identifiers.
    pub linked_data_sheets: Vec<String>,
    /// Named constants stored on the info sheet.
    pub constants: Vec<InfoConstant>,
}

/// Summary metadata for a result sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResultSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Upstream source sheet identifier.
    pub source_sheet_id: String,
    /// Effective analysis title.
    pub title: String,
    /// Debug representation of [`curvebook_doc::ResultStatus`].
    pub status: String,
    /// Number of configured hook references.
    pub hook_count: usize,
    /// Number of output tables/tabs.
    pub tabs: usize,
    /// Number of emitted scalar hooks.
    pub scalars: usize,
    /// Number of diagnostic checklist entries.
    pub checklist: usize,
    /// Whether this result sheet exposes downstream-analyzable data.
    pub analyzable: bool,
}

/// Summary metadata for a graph sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Human-readable sheet title.
    pub title: String,
    /// Whether the graph has been initialized with a plot type.
    pub initialized: bool,
    /// Debug representation of [`curvebook_doc::GraphKind`].
    pub kind_debug: String,
    /// Primary source sheet identifier.
    pub primary_source_sheet_id: String,
    /// Number of plotted series.
    pub series_count: usize,
    /// Canvas width in typographic points.
    pub width_pt: f64,
    /// Canvas height in typographic points.
    pub height_pt: f64,
}

/// Summary metadata for a layout sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayoutSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Human-readable sheet title.
    pub title: String,
    /// Number of placed layout items.
    pub item_count: usize,
    /// Page width in typographic points.
    pub width_pt: f64,
    /// Page height in typographic points.
    pub height_pt: f64,
}

/// Complete project overview produced by [`inspect_project_summary`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectInspectSummary {
    /// Document archive schema version.
    pub schema_version: u32,
    /// Project title.
    pub title: String,
    /// Whether the owning session has unsaved modifications.
    pub dirty: bool,
    /// Associated archive path, if any.
    pub file_path: Option<String>,
    /// Total count of sheets across all folders.
    pub total_sheet_count: usize,
    /// Active sheet identifier, if any.
    pub active_sheet_id: Option<String>,
    /// Active family anchor sheet identifier, if any.
    pub family_anchor_sheet_id: Option<String>,
    /// Number of saved named checkpoints.
    pub checkpoints: usize,
    /// Data sheet summaries.
    pub data_sheets: Vec<DataSheetSummary>,
    /// Info sheet summaries.
    pub info_sheets: Vec<InfoSheetSummary>,
    /// Result sheet summaries.
    pub result_sheets: Vec<ResultSheetSummary>,
    /// Graph sheet summaries.
    pub graph_sheets: Vec<GraphSheetSummary>,
    /// Layout sheet summaries.
    pub layout_sheets: Vec<LayoutSheetSummary>,
    /// Optional focused sheet section and upstream dependency breakdown.
    pub focus: Option<SheetFocusSummary>,
}

impl ProjectInspectSummary {
    /// Project this summary into the canonical `project.inspect` command JSON payload shape.
    #[must_use]
    pub fn to_project_inspect_json(&self) -> Value {
        let data_sheets: Vec<Value> = self
            .data_sheets
            .iter()
            .map(|ds| {
                json!({
                    "id": ds.id,
                    "title": ds.title,
                    "kind": ds.kind_debug,
                    "frozen": ds.frozen,
                    "rows": ds.rows,
                    "cols": ds.cols,
                    "allocated_cells": ds.allocated_cells,
                    "info_sheet_id": ds.info_sheet_id,
                    "linked_regions": ds.linked_regions,
                })
            })
            .collect();
        let info_sheets: Vec<Value> = self
            .info_sheets
            .iter()
            .map(|info| {
                json!({
                    "id": info.id,
                    "title": info.title,
                    "notes_len": info.notes_len,
                    "constants": info.constants.iter().map(|c| json!({ "name": c.name, "value": &c.value, "units": c.units })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let result_sheets: Vec<Value> = self
            .result_sheets
            .iter()
            .map(|rs| {
                json!({
                    "id": rs.id,
                    "source_sheet_id": rs.source_sheet_id,
                    "title": rs.title,
                    "status": rs.status,
                    "hook_count": rs.hook_count,
                    "tabs": rs.tabs,
                    "scalars": rs.scalars,
                    "analyzable": rs.analyzable,
                })
            })
            .collect();
        let graph_sheets: Vec<Value> = self
            .graph_sheets
            .iter()
            .map(|gs| {
                json!({
                    "id": gs.id,
                    "title": gs.title,
                    "initialized": gs.initialized,
                    "kind": gs.kind_debug,
                    "primary_source_sheet_id": gs.primary_source_sheet_id,
                    "series_count": gs.series_count,
                })
            })
            .collect();
        let layout_sheets: Vec<Value> =
            self.layout_sheets.iter().map(|ls| json!({ "id": ls.id, "title": ls.title, "item_count": ls.item_count })).collect();
        let focus = self.focus.as_ref().map(|f| {
            json!({
                "sheet_id": f.sheet_id,
                "section": f.section,
                "upstream_dependencies": f.upstream_dependencies,
            })
        });
        json!({
            "schema_version": self.schema_version,
            "title": self.title,
            "total_sheet_count": self.total_sheet_count,
            "active_sheet_id": self.active_sheet_id,
            "family_anchor_sheet_id": self.family_anchor_sheet_id,
            "checkpoints": self.checkpoints,
            "data_sheets": data_sheets,
            "info_sheets": info_sheets,
            "result_sheets": result_sheets,
            "graph_sheets": graph_sheets,
            "layout_sheets": layout_sheets,
            "focus": focus,
        })
    }

    /// Project this summary into the canonical live-control `doc.inspect` JSON payload shape.
    #[must_use]
    pub fn to_doc_inspect_json(&self) -> Value {
        let data_sheets: Vec<Value> = self
            .data_sheets
            .iter()
            .map(|s| json!({ "id": s.id, "title": s.title, "kind": s.kind_label, "rows": s.rows, "cols": s.cols, "frozen": s.frozen }))
            .collect();
        let info_sheets: Vec<Value> =
            self.info_sheets.iter().map(|s| json!({ "id": s.id, "title": s.title, "constants": s.constants.len() })).collect();
        let result_sheets: Vec<Value> = self
            .result_sheets
            .iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "title": s.title,
                    "status": s.status,
                    "tabs": s.tabs,
                    "scalars": s.scalars,
                    "checklist": s.checklist,
                    "has_analyzable_data": s.analyzable,
                })
            })
            .collect();
        let graph_sheets: Vec<Value> = self
            .graph_sheets
            .iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "title": s.title,
                    "initialized": s.initialized,
                    "kind": s.kind_debug,
                    "width_pt": s.width_pt,
                    "height_pt": s.height_pt,
                })
            })
            .collect();
        let layout_sheets: Vec<Value> = self
            .layout_sheets
            .iter()
            .map(|s| json!({ "id": s.id, "title": s.title, "items": s.item_count, "width_pt": s.width_pt, "height_pt": s.height_pt }))
            .collect();

        json!({
            "title": self.title,
            "schema_version": self.schema_version,
            "dirty": self.dirty,
            "file_path": self.file_path,
            "active_sheet_id": self.active_sheet_id,
            "family_anchor_sheet_id": self.family_anchor_sheet_id,
            "counts": {
                "total": self.total_sheet_count,
                "data": data_sheets.len(),
                "info": info_sheets.len(),
                "results": result_sheets.len(),
                "graphs": graph_sheets.len(),
                "layouts": layout_sheets.len(),
            },
            "data_sheets": data_sheets,
            "info_sheets": info_sheets,
            "result_sheets": result_sheets,
            "graph_sheets": graph_sheets,
            "layout_sheets": layout_sheets,
        })
    }
}

fn count_analysis_hooks(spec: &AnalysisSpec) -> usize {
    let generic_hooks = spec.kind.hooked_sources().len();
    let power_info_hook = usize::from(matches!(&spec.kind, AnalysisKind::PowerAnalysis(p) if p.info_sheet_id.is_some()));
    generic_hooks.saturating_add(power_info_hook)
}

/// Inspect project-wide metadata, sheet summaries, and optional focused sheet dependencies.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `focus_sheet_id` does not exist in `doc`.
pub fn inspect_project_summary(
    doc: &ProjectDocument,
    dirty: bool,
    file_path: Option<&Path>,
    focus_sheet_id: Option<&SheetId>,
) -> Result<ProjectInspectSummary, EngineError> {
    let focus = if let Some(focus_id) = focus_sheet_id {
        let section = doc
            .sheet_section(focus_id)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", focus_id.as_str())))?;
        let upstream = doc.upstream_dependencies(focus_id).into_iter().map(|id| id.as_str().to_owned()).collect();
        Some(SheetFocusSummary { sheet_id: focus_id.as_str().to_owned(), section: format!("{section:?}"), upstream_dependencies: upstream })
    } else {
        None
    };

    let data_sheets = doc
        .data_sheets
        .iter()
        .map(|ds| DataSheetSummary {
            id: ds.id.as_str().to_owned(),
            title: ds.title.clone(),
            kind_debug: format!("{:?}", ds.kind()),
            kind_label: ds.kind().label().to_owned(),
            frozen: ds.frozen,
            rows: ds.archetype.row_count(),
            cols: ds.archetype.column_count(),
            allocated_cells: ds.archetype.allocated_cell_slots(),
            info_sheet_id: ds.info_sheet_id.as_ref().map(|id| id.as_str().to_owned()),
            linked_regions: ds.linked_regions.len(),
            upstream_dependencies: doc.upstream_dependencies(&ds.id).into_iter().map(|id| id.as_str().to_owned()).collect(),
        })
        .collect();

    let info_sheets = doc
        .info_sheets
        .iter()
        .map(|info| InfoSheetSummary {
            id: info.id.as_str().to_owned(),
            title: info.title.clone(),
            notes_len: info.notes_markdown.len(),
            linked_data_sheets: info.linked_data_sheets.iter().map(|id| id.as_str().to_owned()).collect(),
            constants: info.constants.clone(),
        })
        .collect();

    let result_sheets = doc
        .result_sheets
        .iter()
        .map(|rs| ResultSheetSummary {
            id: rs.id.as_str().to_owned(),
            source_sheet_id: rs.analysis.source_sheet_id.as_str().to_owned(),
            title: rs.analysis.effective_title().to_owned(),
            status: format!("{:?}", rs.status),
            hook_count: count_analysis_hooks(&rs.analysis),
            tabs: rs.tabs.len(),
            scalars: rs.scalars.len(),
            checklist: rs.checklist.len(),
            analyzable: rs.analyzable_data.is_some(),
        })
        .collect();

    let graph_sheets = doc
        .graph_sheets
        .iter()
        .map(|gs| GraphSheetSummary {
            id: gs.id.as_str().to_owned(),
            title: gs.title.clone(),
            initialized: gs.initialized,
            kind_debug: format!("{:?}", gs.spec.kind),
            primary_source_sheet_id: gs.spec.primary_source_sheet_id.as_str().to_owned(),
            series_count: gs.spec.series.len(),
            width_pt: gs.spec.canvas_size_pt.width,
            height_pt: gs.spec.canvas_size_pt.height,
        })
        .collect();

    let layout_sheets = doc
        .layout_sheets
        .iter()
        .map(|ls| {
            let size = ls.spec.page_size_pt();
            LayoutSheetSummary {
                id: ls.id.as_str().to_owned(),
                title: ls.title.clone(),
                item_count: ls.spec.items.len(),
                width_pt: size.width,
                height_pt: size.height,
            }
        })
        .collect();

    Ok(ProjectInspectSummary {
        schema_version: doc.schema_version,
        title: doc.title.clone(),
        dirty,
        file_path: file_path.map(|p| p.display().to_string()),
        total_sheet_count: doc.total_sheet_count(),
        active_sheet_id: doc.active_sheet_id.as_ref().map(|id| id.as_str().to_owned()),
        family_anchor_sheet_id: doc.family_anchor_sheet_id.as_ref().map(|id| id.as_str().to_owned()),
        checkpoints: doc.checkpoints.len(),
        data_sheets,
        info_sheets,
        result_sheets,
        graph_sheets,
        layout_sheets,
        focus,
    })
}

/// Inspect either a single sheet detail payload or the live-control `doc.inspect` project summary.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `sheet_id` does not exist in `doc`, or
/// [`ErrorCode::BadInput`] if CSV preview generation fails for a requested data sheet.
pub fn inspect_document_control_json(
    doc: &ProjectDocument,
    dirty: bool,
    file_path: Option<&Path>,
    sheet_id: Option<&SheetId>,
    include_cells: bool,
) -> Result<Value, EngineError> {
    if let Some(id) = sheet_id {
        return inspect_sheet_detail(doc, id, include_cells);
    }
    Ok(inspect_project_summary(doc, dirty, file_path, None)?.to_doc_inspect_json())
}

/// Inspect a single sheet in detail (including optional CSV preview, tabs, checklist, or specs).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `sheet_id` does not exist in `doc`.
pub fn inspect_sheet_detail(doc: &ProjectDocument, sheet_id: &SheetId, include_cells: bool) -> Result<Value, EngineError> {
    let Some(section) = doc.sheet_section(sheet_id) else {
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", sheet_id.as_str()))
            .with_details(json!({ "sheet_id": sheet_id.as_str() })));
    };
    let deps: Vec<String> = doc.upstream_dependencies(sheet_id).into_iter().map(|id| id.as_str().to_owned()).collect();
    let detail = match section {
        SheetSection::Data => match doc.data_sheet(sheet_id) {
            Some(ds) => {
                let mut val = json!({
                    "section": "Data",
                    "id": ds.id.as_str(),
                    "title": ds.title,
                    "kind": ds.kind().label(),
                    "rows": ds.archetype.row_count(),
                    "cols": ds.archetype.column_count(),
                    "columns": ds.archetype.column_count(),
                    "frozen": ds.frozen,
                    "info_sheet_id": ds.info_sheet_id.as_ref().map(SheetId::as_str),
                    "linked_regions": ds.linked_regions.len(),
                    "upstream_dependencies": deps,
                });
                if include_cells && let Some(obj) = val.as_object_mut() {
                    let (csv_preview, csv_preview_truncated) = build_csv_preview(ds)?;
                    obj.insert(String::from("csv_preview"), Value::String(csv_preview));
                    obj.insert(String::from("csv_preview_truncated"), Value::Bool(csv_preview_truncated));
                }
                Some(val)
            }
            None => None,
        },
        SheetSection::Info => doc.info_sheet(sheet_id).map(|info| {
            let mut val = json!({
                "section": "Info",
                "id": info.id.as_str(),
                "title": info.title,
                "constants": info.constants.len(),
                "linked_data_sheets": info.linked_data_sheets.iter().map(SheetId::as_str).collect::<Vec<_>>(),
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                obj.insert(String::from("constants_entries"), serde_json::to_value(&info.constants).unwrap_or(Value::Null));
                obj.insert(String::from("notes_markdown"), Value::String(info.notes_markdown.clone()));
            }
            val
        }),
        SheetSection::Analyses => doc.result_sheet(sheet_id).map(|rs| {
            let mut val = json!({
                "section": "Results",
                "id": rs.id.as_str(),
                "title": rs.analysis.effective_title(),
                "source_sheet_id": rs.analysis.source_sheet_id.as_str(),
                "status": format!("{:?}", rs.status),
                "tabs": rs.tabs.len(),
                "scalars": rs.scalars.len(),
                "checklist": rs.checklist.len(),
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                let (tabs_preview, tabs_truncated) = slice_result_tabs_preview(&rs.tabs);
                let (scalars_preview, scalars_truncated) = slice_preview_vec(&rs.scalars, MAX_RESULT_PREVIEW_SCALARS);
                let (checklist_preview, checklist_truncated) = slice_preview_vec(&rs.checklist, MAX_RESULT_PREVIEW_CHECKLIST);
                obj.insert(String::from("tabs_data"), serde_json::to_value(&tabs_preview).unwrap_or(Value::Null));
                obj.insert(String::from("tabs_truncated"), Value::Bool(tabs_truncated));
                obj.insert(String::from("scalars_data"), serde_json::to_value(&scalars_preview).unwrap_or(Value::Null));
                obj.insert(String::from("scalars_truncated"), Value::Bool(scalars_truncated));
                obj.insert(String::from("checklist_entries"), serde_json::to_value(&checklist_preview).unwrap_or(Value::Null));
                obj.insert(String::from("checklist_truncated"), Value::Bool(checklist_truncated));
            }
            val
        }),
        SheetSection::Graphs => doc.graph_sheet(sheet_id).map(|gs| {
            let mut val = json!({
                "section": "Graphs",
                "id": gs.id.as_str(),
                "title": gs.title,
                "initialized": gs.initialized,
                "kind": format!("{:?}", gs.spec.kind),
                "width_pt": gs.spec.canvas_size_pt.width,
                "height_pt": gs.spec.canvas_size_pt.height,
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                obj.insert(String::from("spec"), serde_json::to_value(&gs.spec).unwrap_or(Value::Null));
            }
            val
        }),
        SheetSection::Layouts => doc.layout_sheet(sheet_id).map(|ls| {
            let size = ls.spec.page_size_pt();
            let mut val = json!({
                "section": "Layouts",
                "id": ls.id.as_str(),
                "title": ls.title,
                "items": ls.spec.items.len(),
                "width_pt": size.width,
                "height_pt": size.height,
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                obj.insert(String::from("spec"), serde_json::to_value(&ls.spec).unwrap_or(Value::Null));
            }
            val
        }),
    };
    detail.ok_or_else(|| {
        EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", sheet_id.as_str()))
            .with_details(json!({ "sheet_id": sheet_id.as_str() }))
    })
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
    if let Some(nl) = csv.get(..end).and_then(|s| s.rfind('\n'))
        && nl > 0
    {
        end = nl + 1;
    }
    csv.truncate(end);
    (csv, true)
}

fn map_preview_err(err: impl std::fmt::Display) -> EngineError {
    EngineError::with_code(ErrorCode::BadInput, format!("CSV preview export failed: {err}"))
}

/// Render a bounded CSV preview (capped at [`MAX_CSV_PREVIEW_ROWS`] rows and
/// [`MAX_CSV_PREVIEW_BYTES`] bytes) along with a truncation flag.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if table validation or CSV rendering fails.
pub fn build_csv_preview(ds: &DataSheet) -> Result<(String, bool), EngineError> {
    build_csv_preview_with_limit(ds, MAX_CSV_PREVIEW_ROWS)
}

/// Render a bounded CSV preview capped at `max_rows` (clamped to `1..=MAX_CSV_PREVIEW_ROWS`) and
/// [`MAX_CSV_PREVIEW_BYTES`] bytes along with a truncation flag.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if table validation or CSV rendering fails.
pub fn build_csv_preview_with_limit(ds: &DataSheet, max_rows: usize) -> Result<(String, bool), EngineError> {
    let capped_rows = max_rows.clamp(1, MAX_CSV_PREVIEW_ROWS);
    ds.archetype.validate().map_err(map_preview_err)?;
    let row_count = ds.archetype.row_count();
    let row_truncated = row_count > capped_rows;
    let raw_csv = if row_truncated {
        let sliced = slice_archetype_rows(&ds.archetype, capped_rows);
        let preview_ds = DataSheet::new(ds.id.clone(), ds.title.clone(), sliced).map_err(map_preview_err)?;
        export_datasheet_to_csv(&preview_ds, &CsvOptions::csv())
    } else {
        export_datasheet_to_csv(ds, &CsvOptions::csv())
    }
    .map_err(map_preview_err)?;
    let (capped, byte_truncated) = truncate_csv_bytes(raw_csv, MAX_CSV_PREVIEW_BYTES);
    Ok((capped, row_truncated || byte_truncated))
}

/// Slice up to `limit` cloned items from `items`, returning `(sliced, was_truncated)`.
#[must_use]
pub fn slice_preview_vec<T: Clone>(items: &[T], limit: usize) -> (Vec<T>, bool) {
    let truncated = items.len() > limit;
    (items.iter().take(limit).cloned().collect(), truncated)
}

/// Slice result tabs to preview bounds (`MAX_RESULT_PREVIEW_*`), returning `(tabs, was_truncated)`.
#[must_use]
pub fn slice_result_tabs_preview(tabs: &[ResultTab]) -> (Vec<ResultTab>, bool) {
    slice_result_tabs_preview_with_opts(tabs, MAX_RESULT_PREVIEW_ROWS, None)
}

/// Slice result tabs to preview bounds with custom row cap (clamped to `1..=MAX_RESULT_PREVIEW_ROWS`)
/// and optional `tab_id` filter, returning `(tabs, was_truncated)`.
#[must_use]
pub fn slice_result_tabs_preview_with_opts(tabs: &[ResultTab], max_rows: usize, tab_id: Option<&str>) -> (Vec<ResultTab>, bool) {
    let row_cap = max_rows.clamp(1, MAX_RESULT_PREVIEW_ROWS);
    let filtered: Vec<&ResultTab> = match tab_id {
        Some(target) => tabs.iter().filter(|t| t.id == target).collect(),
        None => tabs.iter().collect(),
    };
    let mut truncated = filtered.len() > MAX_RESULT_PREVIEW_TABS;
    let mut remaining_cells = MAX_RESULT_PREVIEW_TOTAL_CELLS;
    let mut out = Vec::with_capacity(filtered.len().min(MAX_RESULT_PREVIEW_TABS));

    for tab in filtered.into_iter().take(MAX_RESULT_PREVIEW_TABS) {
        if tab.column_headers.len() > MAX_RESULT_PREVIEW_COLS || tab.rows.len() > row_cap {
            truncated = true;
        }
        let column_headers: Vec<String> = tab.column_headers.iter().take(MAX_RESULT_PREVIEW_COLS).cloned().collect();
        let mut out_rows = Vec::new();
        let mut out_row_labels = Vec::new();
        for (r_idx, row) in tab.rows.iter().take(row_cap).enumerate() {
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

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, DataSheet, GraphSheet, LayoutSheet, LayoutSpec, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
    use serde_json::json;

    use super::inspect_document_control_json;

    #[test]
    fn control_doc_inspect_projection_includes_counts_and_sheet_summaries() -> Result<(), Box<dyn std::error::Error>> {
        let mut doc = ProjectDocument::new("Control Inspect")?;
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 1)?;
        xy.x_cells[0][0] = Cell::number(1.0);
        xy.data_sets[0].cells[0][0] = Cell::number(2.0);
        let ds = DataSheet::new("d1", "Data", TableArchetype::Xy(xy))?;
        let mut gs = GraphSheet::new_uninitialized_for_data("g1", &ds)?;
        gs.initialized = true;
        doc.add_data_sheet(ds)?;
        doc.add_graph_sheet(gs)?;
        doc.add_layout_sheet(LayoutSheet::new("l1", "Layout", LayoutSpec::default())?)?;

        let summary = inspect_document_control_json(&doc, true, None, None, false)?;
        assert_eq!(summary["title"], json!("Control Inspect"));
        assert_eq!(summary["dirty"], json!(true));
        assert_eq!(summary["counts"], json!({ "total": 3, "data": 1, "info": 0, "results": 0, "graphs": 1, "layouts": 1 }));
        assert_eq!(summary["data_sheets"][0]["kind"], json!("XY"));
        assert_eq!(summary["graph_sheets"][0]["initialized"], json!(true));
        assert_eq!(summary["layout_sheets"][0]["items"], json!(0));

        let detail = inspect_document_control_json(&doc, true, None, Some(&doc.data_sheets[0].id), true)?;
        assert_eq!(detail["section"], json!("Data"));
        assert!(detail["csv_preview"].as_str().is_some_and(|s| s.contains("2")));
        Ok(())
    }
}
