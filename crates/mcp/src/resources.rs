//! MCP `resources/list`, `resources/templates/list`, and `resources/read` handlers exposing active `Session` sheets and results.

use curvebook_doc::{DataSheet, ProjectDocument, SheetId, SheetSection};
pub(crate) use curvebook_engine::services::{
    MAX_CSV_PREVIEW_ROWS, MAX_RESULT_PREVIEW_CHECKLIST, MAX_RESULT_PREVIEW_ROWS, MAX_RESULT_PREVIEW_SCALARS, slice_preview_vec,
    slice_result_tabs_preview_with_opts,
};
use curvebook_format::export_result_sheet_json;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Resource entry returned inside MCP `resources/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDescriptor {
    pub uri: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

/// Resource template entry returned inside MCP `resources/templates/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTemplateDescriptor {
    #[serde(rename = "uriTemplate")]
    pub uri_template: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

/// Enumerate canonical MCP resource URI templates supported by `resources/read`.
#[must_use]
pub fn list_resource_templates() -> Vec<ResourceTemplateDescriptor> {
    vec![
        ResourceTemplateDescriptor {
            uri_template: String::from("sheet://{id}"),
            name: String::from("Sheet summary"),
            description: String::from("Active project sheet summary/preview (supports optional `?max_rows=N&tab_id=<id>&detail=summary|full`)."),
            mime_type: String::from("application/json"),
        },
        ResourceTemplateDescriptor {
            uri_template: String::from("result://{id}"),
            name: String::from("Result sheet JSON"),
            description: String::from(
                "Canonical computed ResultSheet JSON export (supports `?include_curves=false` or `?detail=summary` to omit dense curve sample arrays).",
            ),
            mime_type: String::from("application/json"),
        },
    ]
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

impl std::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUri(msg) | Self::NotFound(msg) | Self::Internal(msg) => f.write_str(msg),
        }
    }
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
    if let Some(sheet_raw) = trimmed.strip_prefix("sheet://") {
        let (id_str, query) = split_resource_query(sheet_raw)?;
        if id_str.trim().is_empty() {
            return Err(ResourceError::InvalidUri(String::from("sheet:// URI requires a non-empty sheet id")));
        }
        return read_sheet_resource(doc, trimmed, id_str, &query);
    }
    if let Some(result_raw) = trimmed.strip_prefix("result://") {
        let (id_str, query) = split_resource_query(result_raw)?;
        if id_str.trim().is_empty() {
            return Err(ResourceError::InvalidUri(String::from("result:// URI requires a non-empty result sheet id")));
        }
        return read_result_resource(doc, trimmed, id_str, &query);
    }
    Err(ResourceError::InvalidUri(format!("unsupported resource URI scheme '{trimmed}'; expected sheet://<id> or result://<id>")))
}

#[derive(Default)]
pub(crate) struct ResourceQuery {
    pub(crate) include_curves: Option<bool>,
    pub(crate) detail_summary: bool,
    pub(crate) max_rows: Option<usize>,
    pub(crate) tab_id: Option<String>,
}

pub(crate) fn split_resource_query(raw: &str) -> Result<(&str, ResourceQuery), ResourceError> {
    let Some((id, query_str)) = raw.split_once('?') else {
        return Ok((raw, ResourceQuery::default()));
    };
    let mut query = ResourceQuery::default();
    for pair in query_str.split('&').filter(|s| !s.is_empty()) {
        let (key, val) = pair
            .split_once('=')
            .ok_or_else(|| ResourceError::InvalidUri(format!("invalid query parameter '{pair}' in resource URI; expected key=value")))?;
        match key {
            "include_curves" => match val.to_ascii_lowercase().as_str() {
                "true" | "1" => query.include_curves = Some(true),
                "false" | "0" => query.include_curves = Some(false),
                _ => {
                    return Err(ResourceError::InvalidUri(format!("invalid `include_curves` value '{val}'; expected true or false")));
                }
            },
            "detail" => match val.to_ascii_lowercase().as_str() {
                "summary" | "compact" => query.detail_summary = true,
                "full" => query.detail_summary = false,
                _ => {
                    return Err(ResourceError::InvalidUri(format!("invalid `detail` value '{val}'; expected summary, compact, or full")));
                }
            },
            "max_rows" => {
                let parsed = val
                    .parse::<usize>()
                    .map_err(|_| ResourceError::InvalidUri(format!("invalid `max_rows` value '{val}'; expected positive integer")))?;
                if parsed == 0 || parsed > MAX_CSV_PREVIEW_ROWS {
                    return Err(ResourceError::InvalidUri(format!("`max_rows` must be in 1..={MAX_CSV_PREVIEW_ROWS}, got {parsed}")));
                }
                query.max_rows = Some(parsed);
            }
            "tab_id" => {
                if val.trim().is_empty() {
                    return Err(ResourceError::InvalidUri(String::from("`tab_id` query parameter must not be empty")));
                }
                query.tab_id = Some(val.to_owned());
            }
            other => {
                return Err(ResourceError::InvalidUri(format!(
                    "unsupported resource query parameter '{other}'; supported: detail, include_curves, max_rows, tab_id"
                )));
            }
        }
    }
    Ok((id, query))
}

pub(crate) fn build_csv_preview_with_limit(ds: &DataSheet, max_rows: usize) -> Result<(String, bool), ResourceError> {
    curvebook_engine::services::build_csv_preview_with_limit(ds, max_rows).map_err(|err| ResourceError::Internal(err.message().to_owned()))
}

fn read_sheet_resource(doc: &ProjectDocument, uri: &str, id_str: &str, query: &ResourceQuery) -> Result<Vec<ResourceContent>, ResourceError> {
    let target = SheetId::new(id_str);
    let deps = doc.upstream_dependencies(&target);
    let Some(section) = doc.sheet_section(&target) else {
        return Err(ResourceError::NotFound(format!("resource not found: {uri}")));
    };

    let payload: Option<Value> = match section {
        SheetSection::Data => match doc.data_sheet(&target) {
            Some(ds) => {
                let row_cap = query.max_rows.unwrap_or(MAX_CSV_PREVIEW_ROWS);
                let (csv_preview, csv_preview_truncated) = build_csv_preview_with_limit(ds, row_cap)?;
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
            let row_cap = query.max_rows.unwrap_or(MAX_RESULT_PREVIEW_ROWS);
            let (tabs_preview, tabs_truncated) = slice_result_tabs_preview_with_opts(&rs.tabs, row_cap, query.tab_id.as_deref());
            let (scalars_preview, scalars_truncated) = slice_preview_vec(&rs.scalars, MAX_RESULT_PREVIEW_SCALARS);
            let (checklist_preview, checklist_truncated) = slice_preview_vec(&rs.checklist, MAX_RESULT_PREVIEW_CHECKLIST);
            let include_curves = query.include_curves.unwrap_or(!query.detail_summary);
            let curves_val = if include_curves {
                serde_json::to_value(&rs.fitted_curves).unwrap_or_else(|_| json!([]))
            } else {
                Value::Array(rs.fitted_curves.iter().map(summarize_fitted_curve_meta).collect())
            };
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
                "fitted_curves": curves_val,
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
    let text = serde_json::to_string(&val).map_err(|err| ResourceError::Internal(format!("JSON serialization failed: {err}")))?;
    Ok(vec![ResourceContent { uri: uri.to_owned(), mime_type: String::from("application/json"), text }])
}

fn summarize_fitted_curve_meta(curve: &curvebook_doc::FittedCurveOverlay) -> Value {
    json!({
        "data_set_index": curve.data_set_index,
        "equation_id": curve.equation_id,
        "converged": curve.converged,
        "ambiguous": curve.ambiguous,
        "points_count": curve.points.len(),
        "points_omitted": true,
    })
}

#[derive(Serialize)]
struct FilteredResultTabJson<'a> {
    id: &'a str,
    title: &'a str,
    kind: curvebook_doc::ResultTabKind,
    column_headers: &'a [String],
    row_labels: &'a [String],
    rows: &'a [Vec<curvebook_doc::ResultCell>],
    footnotes: &'a [String],
}

#[derive(Serialize)]
struct FittedCurveMetaSummary<'a> {
    data_set_index: usize,
    equation_id: &'a str,
    converged: bool,
    ambiguous: bool,
    points_count: usize,
    points_omitted: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
enum FilteredFittedCurves<'a> {
    Full(&'a [curvebook_doc::FittedCurveOverlay]),
    Summary(Vec<FittedCurveMetaSummary<'a>>),
}

#[derive(Serialize)]
struct FilteredResultSheetJson<'a> {
    id: &'a SheetId,
    title: &'a str,
    status: &'a curvebook_doc::ResultStatus,
    analysis: &'a curvebook_doc::AnalysisSpec,
    scalars: &'a [curvebook_doc::ResultScalarEntry],
    tabs: Vec<FilteredResultTabJson<'a>>,
    pairwise_comparisons: &'a [curvebook_doc::PairwiseComparisonRecord],
    fitted_curves: FilteredFittedCurves<'a>,
    eliminated_outliers: &'a [curvebook_doc::SourceCellId],
    #[serde(skip_serializing_if = "Option::is_none")]
    analyzable_data: Option<&'a DataSheet>,
    checklist: &'a [curvebook_doc::ChecklistItem],
}

fn read_result_resource(doc: &ProjectDocument, uri: &str, id_str: &str, query: &ResourceQuery) -> Result<Vec<ResourceContent>, ResourceError> {
    let target = SheetId::new(id_str);
    let Some(rs) = doc.result_sheet(&target) else {
        return Err(ResourceError::NotFound(format!("resource not found: {uri}")));
    };
    let include_curves = query.include_curves.unwrap_or(!query.detail_summary);
    if include_curves && query.tab_id.is_none() && query.max_rows.is_none() {
        let text = export_result_sheet_json(rs, false).map_err(|err| ResourceError::Internal(format!("result JSON export failed: {err}")))?;
        return Ok(vec![ResourceContent { uri: uri.to_owned(), mime_type: String::from("application/json"), text }]);
    }

    // Validate `tab_id` and serialize directly from borrowed slices without building a full
    // JSON string + `serde_json::Value` DOM round-trip (`AUD-06-16`).
    if let Some(target_tab) = query.tab_id.as_deref()
        && !rs.tabs.iter().any(|t| t.id == target_tab)
    {
        let available: Vec<&str> = rs.tabs.iter().map(|t| t.id.as_str()).collect();
        return Err(ResourceError::NotFound(format!(
            "tab_id '{target_tab}' not found on '{}'; available tab_ids: {}",
            rs.id.as_str(),
            available.join(", ")
        )));
    }

    let tabs: Vec<FilteredResultTabJson<'_>> = rs
        .tabs
        .iter()
        .filter(|t| query.tab_id.as_deref().is_none_or(|want| t.id == want))
        .map(|t| {
            let row_slice = match query.max_rows {
                Some(cap) => t.rows.get(..cap.min(t.rows.len())).unwrap_or(&t.rows),
                None => t.rows.as_slice(),
            };
            let label_slice = match query.max_rows {
                Some(cap) => t.row_labels.get(..cap.min(t.row_labels.len())).unwrap_or(&t.row_labels),
                None => t.row_labels.as_slice(),
            };
            FilteredResultTabJson {
                id: &t.id,
                title: &t.title,
                kind: t.kind,
                column_headers: &t.column_headers,
                row_labels: label_slice,
                rows: row_slice,
                footnotes: &t.footnotes,
            }
        })
        .collect();

    let fitted_curves = if include_curves {
        FilteredFittedCurves::Full(&rs.fitted_curves)
    } else {
        FilteredFittedCurves::Summary(
            rs.fitted_curves
                .iter()
                .map(|c| FittedCurveMetaSummary {
                    data_set_index: c.data_set_index,
                    equation_id: &c.equation_id,
                    converged: c.converged,
                    ambiguous: c.ambiguous,
                    points_count: c.points.len(),
                    points_omitted: true,
                })
                .collect(),
        )
    };

    let view = FilteredResultSheetJson {
        id: &rs.id,
        title: rs.title(),
        status: &rs.status,
        analysis: &rs.analysis,
        scalars: &rs.scalars,
        tabs,
        pairwise_comparisons: &rs.pairwise_comparisons,
        fitted_curves,
        eliminated_outliers: &rs.eliminated_outliers,
        analyzable_data: rs.analyzable_data.as_ref(),
        checklist: &rs.checklist,
    };

    let compact = serde_json::to_string(&view).map_err(|err| ResourceError::Internal(format!("JSON serialization failed: {err}")))?;
    Ok(vec![ResourceContent { uri: uri.to_owned(), mime_type: String::from("application/json"), text: compact }])
}
