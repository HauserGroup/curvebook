//! `curvebook-cli info <file>` archive & document inspection.

use std::path::Path;

use curvebook_doc::{ProjectDocument, ResultCell, ResultSheet, ResultStatus, TableArchetype};
use curvebook_engine::services::{
    DataSheetSummary, GraphSheetSummary, InfoSheetSummary, LayoutSheetSummary, ProjectInspectSummary, inspect_project_summary,
};
use serde_json::{Map, Value, json};

use crate::cmd_convert::{LoadedProject, load_project_from_path};
use crate::error::CliError;

struct InfoArgs {
    file: String,
    list_sheets: bool,
    json: bool,
}

/// Execute `curvebook-cli info <file> [--list-sheets] [--json]`.
pub fn run_info(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_info_args(args)?;
    let path = Path::new(&parsed.file);
    if parsed.list_sheets {
        let bytes = curvebook_platform::fs::read_regular_file_bytes(path, curvebook_format::MAX_ARCHIVE_FILE_BYTES)
            .map_err(|err| CliError::Runtime(format!("failed to read `{}`: {err}", path.display())))?;
        let sheets = curvebook_format::list_xlsx_sheets(&bytes)?;
        if parsed.json {
            let payload = json!({
                "file": parsed.file,
                "format": "xlsx",
                "sheet_count": sheets.len(),
                "sheets": sheets,
            });
            let text = serde_json::to_string_pretty(&payload).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
            println!("{text}");
        } else {
            println!("File: {} (xlsx, {} sheet(s))", sanitize_terminal_text(&parsed.file), sheets.len());
            for s in &sheets {
                let state_note = s.state.as_deref().map_or(String::new(), |st| format!(" [{st}]"));
                println!("  [{}] `{}`{}", s.index, sanitize_terminal_text(&s.name), state_note);
            }
        }
        return Ok(());
    }
    let (loaded, _bytes) = load_project_from_path(path, false)?;

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
    let Ok(summary) = inspect_project_summary(doc, false, None, None) else {
        return json!({});
    };
    let tables: Vec<Value> = summary.data_sheets.iter().zip(&doc.data_sheets).map(|(s, ds)| summarize_table_json(s, ds)).collect();
    let info_sheets: Vec<Value> = summary.info_sheets.iter().map(summarize_info_sheet_json).collect();
    let analyses: Vec<Value> = doc.result_sheets.iter().map(summarize_result_sheet_json).collect();
    let graphs: Vec<Value> = summary.graph_sheets.iter().zip(&doc.graph_sheets).map(|(s, g)| summarize_graph_sheet_json(s, g)).collect();
    let layouts: Vec<Value> = summary.layout_sheets.iter().map(summarize_layout_sheet_json).collect();

    json!({
        "file": file_label,
        "format": loaded.format_tag,
        "schema_version": summary.schema_version,
        "title": summary.title,
        "counts": {
            "tables": tables.len(),
            "info_sheets": info_sheets.len(),
            "analyses": analyses.len(),
            "graphs": graphs.len(),
            "layouts": layouts.len(),
            "total_sheets": summary.total_sheet_count,
        },
        "tables": tables,
        "info_sheets": info_sheets,
        "analyses": analyses,
        "graphs": graphs,
        "layouts": layouts,
        "warnings": loaded.warnings,
    })
}

fn summarize_table_json(summary: &DataSheetSummary, ds: &curvebook_doc::DataSheet) -> Value {
    let subcolumn_mode = match &ds.archetype {
        TableArchetype::Xy(xy) => format!("{:?}", xy.y_subcolumn_mode),
        TableArchetype::Column(c) => format!("{:?}", c.subcolumn_mode),
        TableArchetype::Grouped(g) => format!("{:?}", g.subcolumn_mode),
        _ => String::from("None"),
    };
    json!({
        "id": summary.id,
        "title": summary.title,
        "kind": summary.kind_label,
        "rows": summary.rows,
        "cols": summary.cols,
        "subcolumn_mode": subcolumn_mode,
        "frozen": summary.frozen,
        "info_sheet_id": summary.info_sheet_id,
    })
}

fn summarize_info_sheet_json(info: &InfoSheetSummary) -> Value {
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
        "id": info.id,
        "title": info.title,
        "constants_count": info.constants.len(),
        "constants": constants,
        "linked_data_sheets": info.linked_data_sheets,
        "notes_bytes": info.notes_len,
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

    let mut payload = json!({
        "id": res.id.as_str(),
        "title": res.title(),
        "source_sheet_id": res.analysis.source_sheet_id.as_ref().map(curvebook_doc::SheetId::as_str),
        "status": format_result_status(&res.status),
        "provenance": &res.provenance,
        "scalars": scalar_map,
        "scalar_entries": scalar_entries,
        "tabs": tabs,
        "fitted_curve_count": res.fitted_curves.len(),
        "fitted_curves": fitted_curves,
    });
    if matches!(&res.analysis.kind, curvebook_doc::AnalysisKind::NonlinearRegression(nl) if nl.method == curvebook_doc::FitMethod::RobustCauchy)
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert(String::from("inference_availability"), Value::String(String::from("NotApplicableInRobustMode")));
    }
    payload
}

fn format_result_status(status: &ResultStatus) -> Value {
    match status {
        ResultStatus::Dirty => json!({ "state": "dirty" }),
        ResultStatus::UpToDate => json!({ "state": "up_to_date" }),
        ResultStatus::Imported => json!({ "state": "imported" }),
        ResultStatus::ImportedStale => json!({ "state": "imported_stale" }),
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

fn format_ci_bound(bound: curvebook_doc::CiBound) -> Value {
    match bound {
        curvebook_doc::CiBound::Finite(v) => json!(v),
        curvebook_doc::CiBound::Open | curvebook_doc::CiBound::Unavailable => Value::Null,
    }
}

fn summarize_graph_sheet_json(summary: &GraphSheetSummary, g: &curvebook_doc::GraphSheet) -> Value {
    json!({
        "id": summary.id,
        "title": summary.title,
        "initialized": summary.initialized,
        "kind": summary.kind_debug,
        "primary_source_sheet_id": summary.primary_source_sheet_id,
        "series_count": summary.series_count,
        "x_title": g.spec.x_axis.title,
        "y_title": g.spec.y_axis.title,
        "canvas_width_pt": summary.width_pt,
        "canvas_height_pt": summary.height_pt,
    })
}

fn summarize_layout_sheet_json(summary: &LayoutSheetSummary) -> Value {
    json!({
        "id": summary.id,
        "title": summary.title,
        "items_count": summary.item_count,
        "page_width_pt": summary.width_pt,
        "page_height_pt": summary.height_pt,
    })
}

fn sanitize_terminal_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_control() {
            for esc in ch.escape_default() {
                out.push(esc);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn print_info_human(file_label: &str, loaded: &LoadedProject) {
    let doc = &loaded.doc;
    let Ok(summary) = inspect_project_summary(doc, false, None, None) else {
        return;
    };
    println!("File: {} ({}, schema v{})", sanitize_terminal_text(file_label), loaded.format_tag, summary.schema_version);
    println!("Title: {}", sanitize_terminal_text(&summary.title));
    println!(
        "Sheets ({} total): {} tables, {} info sheets, {} analyses, {} graphs, {} layouts",
        summary.total_sheet_count,
        summary.data_sheets.len(),
        summary.info_sheets.len(),
        summary.result_sheets.len(),
        summary.graph_sheets.len(),
        summary.layout_sheets.len()
    );
    print_human_sections(&summary, doc);
    if !loaded.warnings.is_empty() {
        println!("Warnings ({}):", loaded.warnings.len());
        for w in &loaded.warnings {
            println!("  - {}", sanitize_terminal_text(w));
        }
    }
}

fn print_human_sections(summary: &ProjectInspectSummary, doc: &ProjectDocument) {
    if !summary.data_sheets.is_empty() {
        println!("Tables:");
        for ds in &summary.data_sheets {
            println!("  - {} [{}] `{}` ({}x{})", sanitize_terminal_text(&ds.id), ds.kind_label, sanitize_terminal_text(&ds.title), ds.rows, ds.cols);
        }
    }
    if !summary.info_sheets.is_empty() {
        println!("Info Sheets:");
        for info in &summary.info_sheets {
            println!("  - {} `{}` ({} constants)", sanitize_terminal_text(&info.id), sanitize_terminal_text(&info.title), info.constants.len());
        }
    }
    if !doc.result_sheets.is_empty() {
        println!("Analyses:");
        for (res_sum, res) in summary.result_sheets.iter().zip(&doc.result_sheets) {
            let inference_note = if matches!(
                &res.analysis.kind,
                curvebook_doc::AnalysisKind::NonlinearRegression(nl) if nl.method == curvebook_doc::FitMethod::RobustCauchy
            ) {
                ", inference: not applicable (Robust mode)"
            } else {
                ""
            };
            println!(
                "  - {} `{}` (source: {}, tabs: {}, scalars: {}, curves: {}{})",
                sanitize_terminal_text(&res_sum.id),
                sanitize_terminal_text(&res_sum.title),
                sanitize_terminal_text(&res_sum.source_sheet_id),
                res_sum.tabs,
                res_sum.scalars,
                res.fitted_curves.len(),
                inference_note
            );
        }
    }
    if !summary.graph_sheets.is_empty() {
        println!("Graphs:");
        for g in &summary.graph_sheets {
            println!(
                "  - {} [{}] `{}` (initialized: {}, series: {})",
                sanitize_terminal_text(&g.id),
                g.kind_debug,
                sanitize_terminal_text(&g.title),
                g.initialized,
                g.series_count
            );
        }
    }
    if !summary.layout_sheets.is_empty() {
        println!("Layouts:");
        for l in &summary.layout_sheets {
            println!("  - {} `{}` ({} items)", sanitize_terminal_text(&l.id), sanitize_terminal_text(&l.title), l.item_count);
        }
    }
}

fn parse_info_args(args: &[String]) -> Result<InfoArgs, CliError> {
    let mut file = None;
    let mut list_sheets = false;
    let mut json = false;
    for arg in args {
        if arg == "--json" {
            json = true;
        } else if arg == "--list-sheets" {
            list_sheets = true;
        } else if arg.starts_with('-') {
            return Err(CliError::Usage(format!("unknown flag `{arg}` for `info`")));
        } else if file.is_none() {
            file = Some(arg.clone());
        } else {
            return Err(CliError::Usage(String::from("usage: curvebook-cli info <file> [--list-sheets] [--json]")));
        }
    }
    let Some(file) = file else {
        return Err(CliError::Usage(String::from("usage: curvebook-cli info <file> [--list-sheets] [--json]")));
    };
    Ok(InfoArgs { file, list_sheets, json })
}
