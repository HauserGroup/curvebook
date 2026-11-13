//! `lumencraft-cli info <file>` archive & document inspection.

use std::fs;
use std::path::Path;

use lumencraft_doc::{ProjectDocument, ResultCell, ResultSheet, ResultStatus, TableArchetype};
use serde_json::{Map, Value, json};

use crate::cmd_convert::{LoadedProject, load_project_from_bytes};
use crate::error::CliError;

struct InfoArgs {
    file: String,
    json: bool,
}

/// Execute `lumencraft-cli info <file> [--json]`.
pub fn run_info(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_info_args(args)?;
    let path = Path::new(&parsed.file);
    let bytes = fs::read(path).map_err(|err| CliError::Runtime(format!("failed to read `{}`: {err}", path.display())))?;
    let loaded = load_project_from_bytes(path, &bytes, false)?;

    if parsed.json {
        let report = build_info_json(&parsed.file, &loaded);
        let text = serde_json::to_string_pretty(&report).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
        println!("{text}");
    } else {
        print_info_human(&parsed.file, &loaded);
    }
    Ok(())
}

/// Build the structured JSON summary for a [`ProjectDocument`].
pub fn build_info_json(file_label: &str, loaded: &LoadedProject) -> Value {
    let doc = &loaded.doc;
    let tables: Vec<Value> = doc.data_sheets.iter().map(summarize_table_json).collect();
    let info_sheets: Vec<Value> = doc.info_sheets.iter().map(summarize_info_sheet_json).collect();
    let analyses: Vec<Value> = doc.result_sheets.iter().map(summarize_result_sheet_json).collect();
    let graphs: Vec<Value> = doc.graph_sheets.iter().map(summarize_graph_sheet_json).collect();
    let layouts: Vec<Value> = doc.layout_sheets.iter().map(summarize_layout_sheet_json).collect();

    json!({
        "file": file_label,
        "format": loaded.format_tag,
        "schema_version": doc.schema_version,
        "title": doc.title,
        "counts": {
            "tables": tables.len(),
            "info_sheets": info_sheets.len(),
            "analyses": analyses.len(),
            "graphs": graphs.len(),
            "layouts": layouts.len(),
            "total_sheets": doc.total_sheet_count(),
        },
        "tables": tables,
        "info_sheets": info_sheets,
        "analyses": analyses,
        "graphs": graphs,
        "layouts": layouts,
        "warnings": loaded.warnings,
    })
}

fn summarize_table_json(ds: &lumencraft_doc::DataSheet) -> Value {
    let subcolumn_mode = match &ds.archetype {
        TableArchetype::Xy(xy) => format!("{:?}", xy.y_subcolumn_mode),
        TableArchetype::Grouped(g) => format!("{:?}", g.subcolumn_mode),
        _ => String::from("None"),
    };
    json!({
        "id": ds.id.as_str(),
        "title": ds.title,
        "kind": ds.kind().label(),
        "rows": ds.archetype.row_count(),
        "cols": ds.archetype.column_count(),
        "subcolumn_mode": subcolumn_mode,
        "frozen": ds.frozen,
        "info_sheet_id": ds.info_sheet_id.as_ref().map(lumencraft_doc::SheetId::as_str),
    })
}

fn summarize_info_sheet_json(info: &lumencraft_doc::InfoSheet) -> Value {
    let constants: Vec<Value> = info
        .constants
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "value": c.value.display_text(),
                "numeric_value": c.value.as_finite_f64(),
                "units": c.units,
            })
        })
        .collect();
    json!({
        "id": info.id.as_str(),
        "title": info.title,
        "constants_count": info.constants.len(),
        "constants": constants,
        "linked_data_sheets": info.linked_data_sheets.iter().map(lumencraft_doc::SheetId::as_str).collect::<Vec<_>>(),
        "notes_bytes": info.notes_markdown.len(),
    })
}

fn summarize_result_sheet_json(res: &ResultSheet) -> Value {
    let mut scalar_map = Map::new();
    let scalar_entries: Vec<Value> = res
        .scalars
        .iter()
        .map(|entry| {
            if entry.value.is_finite() {
                scalar_map.insert(entry.key.clone(), json!(entry.value));
            }
            json!({
                "key": entry.key,
                "data_set_index": entry.data_set_index,
                "value": entry.value,
                "units": entry.units,
            })
        })
        .collect();

    let tabs: Vec<Value> = res
        .tabs
        .iter()
        .map(|tab| {
            let rows: Vec<Vec<Value>> = tab.rows.iter().map(|row| row.iter().map(result_cell_json).collect()).collect();
            json!({
                "id": tab.id,
                "title": tab.title,
                "kind": format!("{:?}", tab.kind),
                "row_count": tab.rows.len(),
                "col_count": tab.column_headers.len(),
                "column_headers": tab.column_headers,
                "row_labels": tab.row_labels,
                "rows": rows,
            })
        })
        .collect();

    let fitted_curves: Vec<Value> = res
        .fitted_curves
        .iter()
        .map(|fc| {
            json!({
                "data_set_index": fc.data_set_index,
                "equation_id": fc.equation_id,
                "converged": fc.converged,
                "ambiguous": fc.ambiguous,
                "point_count": fc.points.len(),
            })
        })
        .collect();

    json!({
        "id": res.id.as_str(),
        "title": res.title(),
        "source_sheet_id": res.analysis.source_sheet_id.as_str(),
        "status": format_result_status(&res.status),
        "scalars": scalar_map,
        "scalar_entries": scalar_entries,
        "tabs": tabs,
        "fitted_curve_count": res.fitted_curves.len(),
        "fitted_curves": fitted_curves,
    })
}

fn format_result_status(status: &ResultStatus) -> Value {
    match status {
        ResultStatus::Dirty => json!({ "state": "dirty" }),
        ResultStatus::UpToDate => json!({ "state": "up_to_date" }),
        ResultStatus::Failed { code, message } => json!({ "state": "failed", "code": code, "message": message }),
    }
}

fn result_cell_json(cell: &ResultCell) -> Value {
    match cell {
        ResultCell::Empty => Value::Null,
        ResultCell::Number { value, prefix, units } => json!({
            "type": "number",
            "value": value,
            "prefix": prefix,
            "units": units,
        }),
        ResultCell::ConfidenceInterval { lower, upper } => json!({
            "type": "ci",
            "lower": format_ci_bound(*lower),
            "upper": format_ci_bound(*upper),
        }),
        ResultCell::PValue { raw, formatted, stars } => json!({
            "type": "p_value",
            "raw": raw,
            "formatted": formatted,
            "stars": stars,
        }),
        ResultCell::Text(s) => json!(s),
        ResultCell::Boolean(b) => json!(b),
    }
}

fn format_ci_bound(bound: lumencraft_doc::CiBound) -> Value {
    match bound {
        lumencraft_doc::CiBound::Finite(v) => json!(v),
        lumencraft_doc::CiBound::Open => Value::Null,
    }
}

fn summarize_graph_sheet_json(g: &lumencraft_doc::GraphSheet) -> Value {
    json!({
        "id": g.id.as_str(),
        "title": g.title,
        "initialized": g.initialized,
        "kind": format!("{:?}", g.spec.kind),
        "primary_source_sheet_id": g.spec.primary_source_sheet_id.as_str(),
        "series_count": g.spec.series.len(),
        "x_title": g.spec.x_axis.title,
        "y_title": g.spec.y_axis.title,
        "canvas_width_pt": g.spec.canvas_size_pt.width,
        "canvas_height_pt": g.spec.canvas_size_pt.height,
    })
}

fn summarize_layout_sheet_json(l: &lumencraft_doc::LayoutSheet) -> Value {
    let page = l.spec.page_size_pt();
    json!({
        "id": l.id.as_str(),
        "title": l.title,
        "items_count": l.spec.items.len(),
        "page_width_pt": page.width,
        "page_height_pt": page.height,
    })
}

fn print_info_human(file_label: &str, loaded: &LoadedProject) {
    let doc = &loaded.doc;
    println!("File: {file_label} ({}, schema v{})", loaded.format_tag, doc.schema_version);
    println!("Title: {}", doc.title);
    println!(
        "Sheets ({} total): {} tables, {} info sheets, {} analyses, {} graphs, {} layouts",
        doc.total_sheet_count(),
        doc.data_sheets.len(),
        doc.info_sheets.len(),
        doc.result_sheets.len(),
        doc.graph_sheets.len(),
        doc.layout_sheets.len()
    );
    print_human_sections(doc);
    if !loaded.warnings.is_empty() {
        println!("Warnings ({}):", loaded.warnings.len());
        for w in &loaded.warnings {
            println!("  - {w}");
        }
    }
}

fn print_human_sections(doc: &ProjectDocument) {
    if !doc.data_sheets.is_empty() {
        println!("Tables:");
        for ds in &doc.data_sheets {
            println!("  - {} [{}] `{}` ({}x{})", ds.id, ds.kind().label(), ds.title, ds.archetype.row_count(), ds.archetype.column_count());
        }
    }
    if !doc.info_sheets.is_empty() {
        println!("Info Sheets:");
        for info in &doc.info_sheets {
            println!("  - {} `{}` ({} constants)", info.id, info.title, info.constants.len());
        }
    }
    if !doc.result_sheets.is_empty() {
        println!("Analyses:");
        for res in &doc.result_sheets {
            println!(
                "  - {} `{}` (source: {}, tabs: {}, scalars: {}, curves: {})",
                res.id,
                res.title(),
                res.analysis.source_sheet_id,
                res.tabs.len(),
                res.scalars.len(),
                res.fitted_curves.len()
            );
        }
    }
    if !doc.graph_sheets.is_empty() {
        println!("Graphs:");
        for g in &doc.graph_sheets {
            println!("  - {} [{:?}] `{}` (initialized: {}, series: {})", g.id, g.spec.kind, g.title, g.initialized, g.spec.series.len());
        }
    }
    if !doc.layout_sheets.is_empty() {
        println!("Layouts:");
        for l in &doc.layout_sheets {
            println!("  - {} `{}` ({} items)", l.id, l.title, l.spec.items.len());
        }
    }
}

fn parse_info_args(args: &[String]) -> Result<InfoArgs, CliError> {
    let mut file = None;
    let mut json = false;
    for arg in args {
        if arg == "--json" {
            json = true;
        } else if arg.starts_with('-') {
            return Err(CliError::Usage(format!("unknown flag `{arg}` for `info`")));
        } else if file.is_none() {
            file = Some(arg.clone());
        } else {
            return Err(CliError::Usage(String::from("usage: lumencraft-cli info <file> [--json]")));
        }
    }
    let Some(file) = file else {
        return Err(CliError::Usage(String::from("usage: lumencraft-cli info <file> [--json]")));
    };
    Ok(InfoArgs { file, json })
}
