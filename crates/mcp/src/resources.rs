//! MCP `resources/list` and `resources/read` handlers exposing active `Session` sheets and results.

use lumencraft_doc::{ProjectDocument, SheetId, SheetSection};
use lumencraft_format::{CsvOptions, export_datasheet_to_csv, export_result_sheet_json};
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

fn read_sheet_resource(doc: &ProjectDocument, uri: &str, id_str: &str) -> Result<Vec<ResourceContent>, ResourceError> {
    let target = SheetId::new(id_str);
    let deps = doc.upstream_dependencies(&target);
    let Some(section) = doc.sheet_section(&target) else {
        return Err(ResourceError::NotFound(format!("resource not found: {uri}")));
    };

    let payload: Option<Value> = match section {
        SheetSection::Data => doc.data_sheet(&target).map(|ds| {
            let csv_preview = export_datasheet_to_csv(ds, &CsvOptions::csv()).unwrap_or_default();
            json!({
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
                "linked_regions": ds.linked_regions,
                "upstream_dependencies": deps,
            })
        }),
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
            let mut payload = json!({
                "uri": uri,
                "section": "analyses",
                "id": rs.id,
                "title": rs.title(),
                "source_sheet_id": rs.analysis.source_sheet_id,
                "selected_data_sets": rs.analysis.selected_data_sets,
                "status": rs.status,
                "has_analyzable_data": rs.analyzable_data.is_some(),
                "tabs": rs.tabs,
                "scalars": rs.scalars,
                "pairwise_comparisons": rs.pairwise_comparisons,
                "fitted_curves": rs.fitted_curves,
                "checklist": rs.checklist,
                "upstream_dependencies": deps,
            });
            if matches!(&rs.analysis.kind, lumencraft_doc::AnalysisKind::NonlinearRegression(nl) if nl.method == lumencraft_doc::FitMethod::RobustCauchy)
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
