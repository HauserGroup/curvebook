//! Regression tests for Stage 4A S2 engine validation, external link shrink/grow,
//! graph/layout cycle guards, linked-region write protection, and title limits
//! (`R1-S07-03`, `R1-S07-04`, `R1-S07-07`, `R1-S07-09`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, DataSheet, DescriptiveSpec, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, TableArchetype,
    XyTable,
};
use curvebook_engine::{EngineError, ErrorCode, Session, dag};
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb-s4a-eng-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exec(session: &mut Session, cmd: &str, params: Value) -> Result<Value, String> {
    session.execute(cmd, params).map(|o| o.payload).map_err(|e| format!("{cmd} failed: {e}"))
}

fn exec_err(session: &mut Session, cmd: &str, params: Value) -> Result<EngineError, String> {
    match session.execute(cmd, params) {
        Ok(o) => Err(format!("expected `{cmd}` to fail, succeeded with: {}", o.payload)),
        Err(err) => Ok(err),
    }
}

fn save_roundtrip(session: &mut Session, path: &Path) -> Result<ProjectDocument, String> {
    let path_str = path.to_string_lossy().into_owned();
    let _ = exec(session, "project.save", json!({ "path": path_str }))?;
    let mut reopened = Session::new();
    let _ = exec(&mut reopened, "project.open", json!({ "path": path_str }))?;
    Ok(reopened.document().clone())
}

#[test]
fn r1_s07_03_external_csv_and_tsv_shrink_grow_and_empty_refresh() -> Result<(), String> {
    let dir = TempDir::new()?;
    let csv_path = dir.path().join("external.csv");
    let tsv_path = dir.path().join("external.tsv");
    let save_path = dir.path().join("project.curvebook");

    std::fs::write(&csv_path, "1,10,100\n2,20,200\n3,30,300\n4,40,400\n").map_err(|e| e.to_string())?;
    std::fs::write(&tsv_path, "10\t1.5\t2.5\t3.5\n20\t4.5\t5.5\t6.5\n30\t7.5\t8.5\t9.5\n").map_err(|e| e.to_string())?;

    let mut session = Session::new();

    // Create target sheets and attach external CSV and TSV live links via table.pasteLink.
    let _ = exec(&mut session, "sheet.createData", json!({ "sheet_id": "d-csv", "kind": "column", "cols": 3, "rows": 4 }))?;
    let _ = exec(&mut session, "sheet.createData", json!({ "sheet_id": "d-tsv", "kind": "column", "cols": 4, "rows": 3 }))?;
    let _ = exec(
        &mut session,
        "table.pasteLink",
        json!({
            "target_sheet_id": "d-csv",
            "source_path": csv_path.to_string_lossy(),
            "delimiter": ",",
        }),
    )?;
    let _ = exec(
        &mut session,
        "table.pasteLink",
        json!({
            "target_sheet_id": "d-tsv",
            "source_path": tsv_path.to_string_lossy(),
            "delimiter": "\t",
        }),
    )?;

    // Verify initial data imported.
    let ds_csv = session.document().data_sheet(&SheetId::new("d-csv")).ok_or("missing d-csv")?;
    assert_eq!(ds_csv.cell(CellCoord::new(3, 0, 0)).and_then(Cell::active_number), Some(4.0));
    assert_eq!(ds_csv.cell(CellCoord::new(3, 2, 0)).and_then(Cell::active_number), Some(400.0));

    let ds_tsv = session.document().data_sheet(&SheetId::new("d-tsv")).ok_or("missing d-tsv")?;
    assert_eq!(ds_tsv.cell(CellCoord::new(2, 3, 0)).and_then(Cell::active_number), Some(9.5));

    // Shrink CSV in rows (4 -> 2) and shrink TSV in both rows (3 -> 1) and columns (4 -> 2).
    std::fs::write(&csv_path, "1,11,111\n2,22,222\n").map_err(|e| e.to_string())?;
    std::fs::write(&tsv_path, "99\t8.25\n").map_err(|e| e.to_string())?;

    let _ = exec(&mut session, "project.recalculate", json!({ "include_external_links": true }))?;

    let ds_csv = session.document().data_sheet(&SheetId::new("d-csv")).ok_or("missing d-csv")?;
    assert_eq!(ds_csv.cell(CellCoord::new(0, 1, 0)).and_then(Cell::active_number), Some(11.0));
    assert_eq!(ds_csv.cell(CellCoord::new(1, 2, 0)).and_then(Cell::active_number), Some(222.0));
    // Trailing rows from prior 4-row import must be cleared, not retained as stale data.
    assert!(ds_csv.cell(CellCoord::new(2, 0, 0)).is_none_or(Cell::is_empty));
    assert!(ds_csv.cell(CellCoord::new(3, 2, 0)).is_none_or(Cell::is_empty));

    let ds_tsv = session.document().data_sheet(&SheetId::new("d-tsv")).ok_or("missing d-tsv")?;
    assert_eq!(ds_tsv.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(99.0));
    assert_eq!(ds_tsv.cell(CellCoord::new(0, 1, 0)).and_then(Cell::active_number), Some(8.25));
    // Shrunk trailing columns & rows must all be cleared.
    assert!(ds_tsv.cell(CellCoord::new(0, 2, 0)).is_none_or(Cell::is_empty));
    assert!(ds_tsv.cell(CellCoord::new(0, 3, 0)).is_none_or(Cell::is_empty));
    assert!(ds_tsv.cell(CellCoord::new(2, 3, 0)).is_none_or(Cell::is_empty));

    // Shrink CSV to empty (0 rows) -> all old data rows cleared cleanly, then grow back.
    std::fs::write(&csv_path, "").map_err(|e| e.to_string())?;
    let _ = exec(&mut session, "project.recalculate", json!({ "include_external_links": true }))?;
    let ds_csv = session.document().data_sheet(&SheetId::new("d-csv")).ok_or("missing d-csv")?;
    assert!(ds_csv.cell(CellCoord::new(0, 0, 0)).is_none_or(Cell::is_empty));
    assert!(ds_csv.cell(CellCoord::new(1, 2, 0)).is_none_or(Cell::is_empty));

    std::fs::write(&csv_path, "5,55,555\n6,66,666\n7,77,777\n").map_err(|e| e.to_string())?;
    let _ = exec(&mut session, "project.recalculate", json!({ "include_external_links": true }))?;
    let ds_csv = session.document().data_sheet(&SheetId::new("d-csv")).ok_or("missing d-csv")?;
    assert_eq!(ds_csv.cell(CellCoord::new(2, 2, 0)).and_then(Cell::active_number), Some(777.0));

    // Target placement offset into an existing sheet via importCsv { link: true, replace: false }.
    let _ = exec(&mut session, "sheet.createData", json!({ "sheet_id": "d-offset", "kind": "column", "cols": 4, "rows": 5 }))?;
    let offset_csv = dir.path().join("offset.csv");
    std::fs::write(&offset_csv, "1.25,2.5\n3.75,4.5\n").map_err(|e| e.to_string())?;
    let _ = exec(
        &mut session,
        "table.importCsv",
        json!({
            "sheet_id": "d-offset",
            "path": offset_csv.to_string_lossy(),
            "delimiter": ",",
            "replace": false,
            "link": true,
            "has_header": false,
            "start_row": 1,
            "start_col": 1,
        }),
    )?;
    let ds_offset = session.document().data_sheet(&SheetId::new("d-offset")).ok_or("missing d-offset")?;
    assert_eq!(ds_offset.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(1.25));
    assert_eq!(ds_offset.cell(CellCoord::new(2, 2, 0)).and_then(Cell::active_number), Some(4.5));
    // Shrink offset CSV and ensure only linked rows are updated while offset stays intact.
    std::fs::write(&offset_csv, "9.5,8.5\n").map_err(|e| e.to_string())?;
    let _ = exec(&mut session, "project.recalculate", json!({ "include_external_links": true }))?;
    let ds_offset = session.document().data_sheet(&SheetId::new("d-offset")).ok_or("missing d-offset")?;
    assert_eq!(ds_offset.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(9.5));
    assert!(ds_offset.cell(CellCoord::new(2, 2, 0)).is_none_or(Cell::is_empty));

    let reloaded = save_roundtrip(&mut session, &save_path)?;
    assert_eq!(reloaded.data_sheets.len(), 3);
    Ok(())
}

#[test]
fn r1_s07_04_graph_and_layout_reject_self_refs_wrong_sections_and_cycles() -> Result<(), String> {
    let dir = TempDir::new()?;
    let save_path = dir.path().join("valid.curvebook");

    let mut session = Session::new();
    let _ = exec(&mut session, "sheet.createData", json!({ "sheet_id": "d", "kind": "column", "cols": 2, "rows": 3, "auto_info": true }))?;
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "d", "row": 0, "col": 0, "value": 10.0 }))?;
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "d", "row": 1, "col": 0, "value": 12.0 }))?;
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "d", "row": 0, "col": 1, "value": 20.0 }))?;
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "d", "row": 1, "col": 1, "value": 24.0 }))?;
    let _ = exec(&mut session, "graph.create", json!({ "sheet_id": "g", "data_sheet_id": "d", "title": "Main Graph" }))?;
    let _ = exec(&mut session, "graph.create", json!({ "sheet_id": "g2", "data_sheet_id": "d", "title": "Other Graph" }))?;
    let _ = exec(&mut session, "layout.create", json!({ "sheet_id": "l", "title": "Main Layout" }))?;

    // Run an analysis so we have a valid ResultSheet too.
    let _ =
        exec(&mut session, "stats.run", json!({ "sheet_id": "res", "source_sheet_id": "d", "kind": "descriptive", "selected_data_sets": [0, 1] }))?;

    let before_graph = session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?.clone();

    // 1. Self-referential or missing primary_source_sheet_id on graph.setSpec (both top-level and nested `spec`) -> CycleDetected / NotFound.
    let err = exec_err(&mut session, "graph.setSpec", json!({ "sheet_id": "g", "primary_source_sheet_id": "g" }))?;
    assert_eq!(err.code, ErrorCode::CycleDetected);
    let err = exec_err(&mut session, "graph.setSpec", json!({ "sheet_id": "g", "spec": { "primary_source_sheet_id": "g" } }))?;
    assert_eq!(err.code, ErrorCode::CycleDetected);
    let err = exec_err(&mut session, "graph.setSpec", json!({ "sheet_id": "g", "spec": { "primary_source_sheet_id": "nope" } }))?;
    assert_eq!(err.code, ErrorCode::NotFound);

    // 2. Self-referential series source via graph.setSpec / graph.setSeries -> CycleDetected.
    let err = exec_err(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g",
            "series": [{ "source_sheet_id": "g", "data_set_index": 0, "legend_label": null, "bind_to_right_y2": false, "gdo": before_graph.spec.series[0].gdo }]
        }),
    )?;
    assert_eq!(err.code, ErrorCode::CycleDetected);
    let err = exec_err(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series_index": 0, "source_sheet_id": "g" }))?;
    assert_eq!(err.code, ErrorCode::CycleDetected);
    let err = exec_err(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series": [{ "source_sheet_id": "g", "data_set_index": 0 }] }))?;
    assert_eq!(err.code, ErrorCode::CycleDetected);
    let err = exec_err(
        &mut session,
        "graph.setSeries",
        json!({
            "sheet_id": "g",
            "series_index": 0,
            "regression_overlay": { "result_sheet_id": "g", "sample_segments": 256, "show_confidence_band": false, "show_prediction_band": false, "shaded_ribbon": false }
        }),
    )?;
    assert_eq!(err.code, ErrorCode::CycleDetected);

    // 3. Self-referential regression_overlay, comparisons, and annotations via graph.setSpec -> CycleDetected.
    let err = exec_err(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g",
            "series": [{
                "source_sheet_id": "d",
                "data_set_index": 0,
                "legend_label": null,
                "bind_to_right_y2": false,
                "gdo": before_graph.spec.series[0].gdo,
                "regression_overlay": { "result_sheet_id": "g", "sample_segments": 256, "show_confidence_band": false, "show_prediction_band": false, "shaded_ribbon": false }
            }]
        }),
    )?;
    assert_eq!(err.code, ErrorCode::CycleDetected);

    let err = exec_err(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g",
            "comparisons": [{
                "result_sheet_id": "g",
                "group_a": 0,
                "group_b": 1,
                "style": "Square",
                "label_mode": "SummaryStars",
                "hide_if_non_significant": false,
                "stroke_width_pt": 1.0,
                "font_size_pt": 9.0,
                "bracket_step_pt": 10.0
            }]
        }),
    )?;
    assert_eq!(err.code, ErrorCode::CycleDetected);

    let err = exec_err(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g",
            "annotations": [{ "EmbeddedResultTable": { "result_sheet_id": "g", "tab_id": null, "bounds_pt": { "x": 10.0, "y": 10.0, "width": 100.0, "height": 50.0 } } }]
        }),
    )?;
    assert_eq!(err.code, ErrorCode::CycleDetected);

    // 4. Self-referential inline hooks in graph spec title, axis title, and annotation -> CycleDetected.
    let err = exec_err(&mut session, "graph.setSpec", json!({ "sheet_id": "g", "spec": { "title": "Fit {{result:g:mean}}" } }))?;
    assert_eq!(err.code, ErrorCode::CycleDetected);
    let err = exec_err(&mut session, "graph.setAxes", json!({ "sheet_id": "g", "axis": "y", "title": "Y {{info:g:dose}}" }))?;
    assert_eq!(err.code, ErrorCode::CycleDetected);
    let err = exec_err(
        &mut session,
        "graph.addAnnotation",
        json!({
            "sheet_id": "g",
            "annotation": { "TextBox": { "bounds_pt": { "x": 5.0, "y": 5.0, "width": 80.0, "height": 20.0 }, "markdown": "{{result:g:EC50}}", "bordered": false } }
        }),
    )?;
    assert_eq!(err.code, ErrorCode::CycleDetected);

    // 5. Wrong-section and missing sheet references rejected upfront.
    let err = exec_err(&mut session, "graph.setSpec", json!({ "sheet_id": "g", "primary_source_sheet_id": "g2" }))?;
    assert_eq!(err.code, ErrorCode::BadInput);
    let err = exec_err(&mut session, "graph.setSpec", json!({ "sheet_id": "g", "primary_source_sheet_id": "missing-sheet" }))?;
    assert_eq!(err.code, ErrorCode::NotFound);
    let err = exec_err(&mut session, "graph.setRegressionOverlay", json!({ "sheet_id": "g", "series_index": 0, "result_sheet_id": "d" }))?;
    assert_ne!(err.code, ErrorCode::CycleDetected);

    // 6. Layout self-reference and wrong-section references rejected upfront.
    let err = exec_err(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "l", "kind": "table", "source_sheet_id": "l", "bounds_pt": [10.0, 10.0, 100.0, 80.0] }),
    )?;
    assert_eq!(err.code, ErrorCode::CycleDetected);
    let err = exec_err(
        &mut session,
        "layout.setSpec",
        json!({
            "sheet_id": "l",
            "items": [{ "LiveTable": { "id": "bad", "source_sheet_id": "g", "tab_id": null, "bounds_pt": { "x": 10.0, "y": 10.0, "width": 100.0, "height": 80.0 } } }]
        }),
    )?;
    assert_eq!(err.code, ErrorCode::BadInput);
    let err = exec_err(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "l", "kind": "text", "markdown": "Bad {{info:l:key}}", "bounds_pt": [10.0, 10.0, 100.0, 40.0] }),
    )?;
    assert_eq!(err.code, ErrorCode::CycleDetected);

    // After all rejected calls, graph & layout retain valid specs and save cleanly.
    assert_eq!(session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?.spec, before_graph.spec);
    let _ = save_roundtrip(&mut session, &save_path)?;

    // 7. Multi-hop cycle detection in dag::topological_order & dag::recalculate_project.
    let mut cycle_doc = ProjectDocument::new("Cycle").map_err(|e| e.to_string())?;
    let ds = DataSheet::new("d", "D", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).map_err(|e| e.to_string())?))
        .map_err(|e| e.to_string())?;
    cycle_doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    let mut g = curvebook_doc::GraphSheet::new_uninitialized_for_data("g", &cycle_doc.data_sheets[0]).map_err(|e| e.to_string())?;
    g.spec.primary_source_sheet_id = SheetId::new("res-cycle");
    cycle_doc.add_graph_sheet(g).map_err(|e| e.to_string())?;
    let analysis_spec = AnalysisSpec::new("res-cycle", "g", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    let mut rs = ResultSheet::new(analysis_spec).map_err(|e| e.to_string())?;
    rs.analysis.source_sheet_id = SheetId::new("g");
    cycle_doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let topo_err = dag::topological_order(&cycle_doc).err().ok_or("expected cycle error in topological_order")?;
    assert_eq!(topo_err.code, ErrorCode::CycleDetected);
    let recalc_err = dag::recalculate_project(&mut cycle_doc).err().ok_or("expected cycle error in recalculate_project")?;
    assert_eq!(recalc_err.code, ErrorCode::CycleDetected);

    Ok(())
}

#[test]
fn r1_s07_07_linked_region_rejects_edits_exclusions_clears_pastes_and_recodes() -> Result<(), String> {
    let dir = TempDir::new()?;
    let save_path = dir.path().join("linked.curvebook");
    let csv_path = dir.path().join("mv.csv");
    std::fs::write(&csv_path, "A,10\nB,20\n").map_err(|e| e.to_string())?;

    let mut session = Session::new();

    // Source XY table (1 Y col, 3 rows).
    let _ = exec(&mut session, "sheet.createData", json!({ "sheet_id": "src", "kind": "xy", "cols": 1, "rows": 3 }))?;
    for (row, val) in [(0, 10.0), (1, 20.0), (2, 30.0)] {
        let _ = exec(&mut session, "table.setXCells", json!({ "sheet_id": "src", "row": row, "value": (row + 1) as f64 }))?;
        let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "src", "row": row, "col": 0, "subcol": 0, "value": val }))?;
    }

    // Target XY table with 2 cols and 2 replicate subcolumns per col:
    // link ONLY into (row 0..=2, col 0, subcol 0). Leave (col 0, subcol 1), col 1, and X unlinked!
    let _ = exec(&mut session, "sheet.createData", json!({ "sheet_id": "dst", "kind": "xy", "cols": 2, "replicates": 2, "rows": 4 }))?;
    let _ = exec(
        &mut session,
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst",
            "source_sheet_id": "src",
            "source_range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 2, "col": 0, "subcol": 0 } },
            "start_row": 0,
            "start_col": 0,
            "start_subcol": 0,
        }),
    )?;

    // Reject single-cell edit inside linked region.
    let err = exec_err(&mut session, "table.setCell", json!({ "sheet_id": "dst", "row": 1, "col": 0, "subcol": 0, "value": 999.0 }))?;
    assert_eq!(err.code, ErrorCode::Conflict);

    // Reject batch setCells if ANY cell overlaps linked region, atomically leaving outside cells untouched.
    let err = exec_err(
        &mut session,
        "table.setCells",
        json!({
            "sheet_id": "dst",
            "edits": [
                { "row": 0, "col": 0, "subcol": 1, "value": 77.0 },
                { "row": 2, "col": 0, "subcol": 0, "value": 888.0 }
            ]
        }),
    )?;
    assert_eq!(err.code, ErrorCode::Conflict);
    let ds_dst = session.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?;
    assert!(ds_dst.cell(CellCoord::new(0, 0, 1)).is_none_or(Cell::is_empty));
    assert_eq!(ds_dst.cell(CellCoord::new(2, 0, 0)).and_then(Cell::active_number), Some(30.0));

    // Reject excludeCells (single, cells array, range) inside linked region.
    let err = exec_err(&mut session, "table.excludeCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "subcol": 0, "excluded": true }))?;
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = exec_err(
        &mut session,
        "table.excludeCells",
        json!({ "sheet_id": "dst", "cells": [{ "row": 0, "col": 0, "subcol": 1 }, { "row": 1, "col": 0, "subcol": 0 }], "excluded": true }),
    )?;
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = exec_err(
        &mut session,
        "table.excludeCells",
        json!({ "sheet_id": "dst", "row_start": 0, "row_end": 1, "col_start": 0, "col_end": 0, "subcol_start": 0, "subcol_end": 1, "excluded": true }),
    )?;
    assert_eq!(err.code, ErrorCode::Conflict);

    // Reject excludeCellsWithReason inside linked region.
    let err = exec_err(
        &mut session,
        "table.excludeCellsWithReason",
        json!({ "sheet_id": "dst", "row": 1, "col": 0, "subcol": 0, "reason_tag": "outlier" }),
    )?;
    assert_eq!(err.code, ErrorCode::Conflict);

    // Reject clearCells (single, cells, range, all without unlink: true) inside linked region.
    let err = exec_err(&mut session, "table.clearCells", json!({ "sheet_id": "dst", "row": 1, "col": 0, "subcol": 0 }))?;
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = exec_err(&mut session, "table.clearCells", json!({ "sheet_id": "dst", "cells": [{ "row": 1, "col": 0, "subcol": 0 }] }))?;
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = exec_err(
        &mut session,
        "table.clearCells",
        json!({ "sheet_id": "dst", "row_start": 1, "row_end": 2, "col_start": 0, "col_end": 0, "subcol_start": 0, "subcol_end": 0 }),
    )?;
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = exec_err(&mut session, "table.clearCells", json!({ "sheet_id": "dst", "all": true }))?;
    assert_eq!(err.code, ErrorCode::Conflict);

    // Reject unlinked table.paste (Overwrite and InsertRows) overlapping linked region.
    let err = exec_err(
        &mut session,
        "table.paste",
        json!({ "sheet_id": "dst", "text": "55\n66\n", "start_row": 1, "start_col": 0, "start_subcol": 0, "mode": "overwrite" }),
    )?;
    assert_eq!(err.code, ErrorCode::Conflict);
    let err = exec_err(
        &mut session,
        "table.paste",
        json!({ "sheet_id": "dst", "text": "99\n", "start_row": 0, "start_col": 0, "start_subcol": 0, "mode": "insert_rows" }),
    )?;
    assert_eq!(err.code, ErrorCode::Conflict);

    // Reject unlinked importCsv (link: false, replace: false) overlapping existing linked region.
    let err = exec_err(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "dst", "csv": "91\n92\n", "has_header": false, "replace": false, "link": false, "start_row": 0, "start_col": 0, "start_subcol": 0 }),
    )?;
    assert_eq!(err.code, ErrorCode::Conflict);

    // Adjacent unlinked subcolumn, unlinked column, unlinked row 3, and X column edits MUST succeed!
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "dst", "row": 0, "col": 0, "subcol": 1, "value": 42.0 }))?;
    let _ = exec(&mut session, "table.excludeCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "subcol": 1, "excluded": true }))?;
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "dst", "row": 3, "col": 0, "subcol": 0, "value": 99.0 }))?;
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "dst", "row": 0, "col": 1, "subcol": 0, "value": 5.0 }))?;
    let _ = exec(&mut session, "table.setXCells", json!({ "sheet_id": "dst", "row": 0, "value": 1.0 }))?;
    let _ = exec(&mut session, "table.excludeCells", json!({ "sheet_id": "dst", "target": "x", "row": 0, "excluded": true }))?;

    // MultipleVariables linked column must reject table.recodeLevels.
    let _ = exec(&mut session, "sheet.createData", json!({ "sheet_id": "mv-linked", "kind": "multiple_variables", "cols": 2, "rows": 2 }))?;
    let _ = exec(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "mv-linked", "path": csv_path.to_string_lossy(), "delimiter": ",", "replace": false, "has_header": false, "link": true }),
    )?;
    let err = exec_err(&mut session, "table.recodeLevels", json!({ "sheet_id": "mv-linked", "col": 0, "mappings": { "A": "Alpha" } }))?;
    assert_eq!(err.code, ErrorCode::Conflict);

    // Save/reload round-trip preserves linked data intact.
    let reloaded = save_roundtrip(&mut session, &save_path)?;
    let ds_reloaded = reloaded.data_sheet(&SheetId::new("dst")).ok_or("missing dst after reload")?;
    assert_eq!(ds_reloaded.linked_regions.len(), 1);
    assert_eq!(ds_reloaded.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(20.0));

    // Intentional unlink + clear via table.clearCells { all: true, unlink: true } detaches link and allows edits.
    let _ = exec(&mut session, "table.clearCells", json!({ "sheet_id": "dst", "all": true, "unlink": true }))?;
    assert!(session.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?.linked_regions.is_empty());
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "dst", "row": 1, "col": 0, "subcol": 0, "value": 123.0 }))?;
    assert_eq!(
        session.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number),
        Some(123.0)
    );

    Ok(())
}

fn assert_exec_err_code(session: &mut Session, cmd: &str, params: Value, expected: ErrorCode) -> Result<(), String> {
    let err = exec_err(session, cmd, params)?;
    assert_eq!(err.code, expected, "expected {expected:?} for `{cmd}`, got `{err}`");
    Ok(())
}

fn verify_sheet_and_table_title_limits(session: &mut Session, oversized_ascii: &str, oversized_utf8: &str) -> Result<(), String> {
    for sid in ["d-xy", "i-1", "r-1", "g-1", "l-1"] {
        assert!(session.document().sheet_section(&SheetId::new(sid)).is_some(), "missing test sheet {sid}");
        assert_exec_err_code(session, "sheet.rename", json!({ "sheet_id": sid, "title": oversized_ascii }), ErrorCode::ResourceLimit)?;
        assert_exec_err_code(session, "sheet.rename", json!({ "sheet_id": sid, "title": oversized_utf8 }), ErrorCode::ResourceLimit)?;
    }

    assert_exec_err_code(
        session,
        "table.setGroupTitles",
        json!({ "sheet_id": "d-xy", "titles": ["GoodFirstCol", oversized_ascii] }),
        ErrorCode::ResourceLimit,
    )?;
    let first_col_title = match &session.document().data_sheet(&SheetId::new("d-xy")).ok_or("missing d-xy")?.archetype {
        TableArchetype::Xy(xy) => xy.data_sets[0].title.as_str(),
        _ => return Err(String::from("expected Xy table")),
    };
    assert_ne!(first_col_title, "GoodFirstCol");

    assert_exec_err_code(session, "table.setTitles", json!({ "sheet_id": "d-xy", "col": 0, "title": oversized_utf8 }), ErrorCode::ResourceLimit)?;
    assert_exec_err_code(session, "table.setGroupTitles", json!({ "sheet_id": "d-xy", "x_title": oversized_ascii }), ErrorCode::ResourceLimit)?;
    assert_exec_err_code(
        session,
        "table.setGroupTitles",
        json!({ "sheet_id": "d-grp", "row_factor_title": oversized_ascii }),
        ErrorCode::ResourceLimit,
    )?;
    assert_exec_err_code(
        session,
        "table.setGroupTitles",
        json!({ "sheet_id": "d-grp", "column_factor_title": oversized_utf8 }),
        ErrorCode::ResourceLimit,
    )?;
    assert_exec_err_code(
        session,
        "table.setSubcolumnTitles",
        json!({ "sheet_id": "d-nest", "col": 0, "titles": ["FirstSubOk", oversized_ascii] }),
        ErrorCode::ResourceLimit,
    )?;
    let sub0_title = match &session.document().data_sheet(&SheetId::new("d-nest")).ok_or("missing d-nest")?.archetype {
        TableArchetype::Nested(nest) => nest.groups[0].subcolumn_titles[0].as_str(),
        _ => return Err(String::from("expected Nested table")),
    };
    assert_ne!(sub0_title, "FirstSubOk");
    assert_exec_err_code(session, "table.setRowTitles", json!({ "sheet_id": "d-xy", "row": 0, "title": oversized_ascii }), ErrorCode::ResourceLimit)?;
    assert_exec_err_code(
        session,
        "table.setCalculatedVariable",
        json!({ "sheet_id": "d-mv", "title": oversized_ascii, "formula": "1 + 1" }),
        ErrorCode::ResourceLimit,
    )
}

fn verify_graph_layout_and_csv_title_limits(session: &mut Session, oversized_ascii: &str, oversized_utf8: &str) -> Result<(), String> {
    assert_exec_err_code(
        session,
        "graph.create",
        json!({ "sheet_id": "g-bad", "data_sheet_id": "d-xy", "title": oversized_ascii }),
        ErrorCode::ResourceLimit,
    )?;
    assert!(session.document().graph_sheet(&SheetId::new("g-bad")).is_none());

    assert_exec_err_code(session, "graph.initialize", json!({ "sheet_id": "g-1", "title": oversized_utf8 }), ErrorCode::ResourceLimit)?;
    assert_exec_err_code(session, "graph.setSpec", json!({ "sheet_id": "g-1", "title": oversized_ascii }), ErrorCode::ResourceLimit)?;
    assert_exec_err_code(session, "graph.setSpec", json!({ "sheet_id": "g-1", "spec": { "title": oversized_utf8 } }), ErrorCode::ResourceLimit)?;
    assert_exec_err_code(session, "graph.setAxes", json!({ "sheet_id": "g-1", "axis": "x", "title": oversized_ascii }), ErrorCode::ResourceLimit)?;
    assert_exec_err_code(session, "layout.create", json!({ "sheet_id": "l-bad", "title": oversized_ascii }), ErrorCode::ResourceLimit)?;
    assert_exec_err_code(session, "layout.setSpec", json!({ "sheet_id": "l-1", "title": oversized_utf8 }), ErrorCode::ResourceLimit)?;

    assert_exec_err_code(
        session,
        "table.importCsv",
        json!({ "sheet_id": "d-csv-bad", "csv": "A,B\n1,2\n", "title": oversized_ascii }),
        ErrorCode::ResourceLimit,
    )?;
    let bad_header_csv = format!("{oversized_ascii},B\n1,2\n");
    assert_exec_err_code(
        session,
        "table.importCsv",
        json!({ "sheet_id": "d-csv-bad-hdr", "csv": bad_header_csv, "has_header": true }),
        ErrorCode::ResourceLimit,
    )?;
    assert!(session.document().data_sheet(&SheetId::new("d-csv-bad-hdr")).is_none());
    Ok(())
}

fn verify_duplicate_title_overflows(session: &mut Session, oversized_ascii: &str, borderline_4095: &str) -> Result<(), String> {
    assert_exec_err_code(
        session,
        "sheet.createData",
        json!({ "sheet_id": "d-4095-info", "title": borderline_4095, "kind": "column", "auto_info": true }),
        ErrorCode::ResourceLimit,
    )?;
    assert!(session.document().data_sheet(&SheetId::new("d-4095-info")).is_none());

    for (sid, copy_id) in [("d-xy", "d-copy"), ("i-1", "i-copy"), ("r-1", "r-copy"), ("g-1", "g-copy"), ("l-1", "l-copy")] {
        assert_exec_err_code(
            session,
            "sheet.duplicate",
            json!({ "sheet_id": sid, "new_sheet_id": copy_id, "title": oversized_ascii }),
            ErrorCode::ResourceLimit,
        )?;
        let _ = exec(session, "sheet.rename", json!({ "sheet_id": sid, "title": borderline_4095 }))?;
        assert_exec_err_code(session, "sheet.duplicate", json!({ "sheet_id": sid, "new_sheet_id": copy_id }), ErrorCode::ResourceLimit)?;
        assert!(session.document().sheet_section(&SheetId::new(copy_id)).is_none());
    }

    let sheet_count_before = session.document().total_sheet_count();
    assert_exec_err_code(session, "sheet.duplicateFamily", json!({ "sheet_id": "d-xy", "new_sheet_id": "d-fam" }), ErrorCode::ResourceLimit)?;
    assert_eq!(session.document().total_sheet_count(), sheet_count_before);
    Ok(())
}

#[test]
fn r1_s07_09_title_validation_enforced_across_all_sheets_tables_graphs_and_layouts() -> Result<(), String> {
    let dir = TempDir::new()?;
    let save_path = dir.path().join("titles.curvebook");

    let oversized_ascii = "T".repeat(4097);
    let oversized_utf8 = "é".repeat(2049); // 2049 chars, 4098 UTF-8 bytes > MAX_TITLE_BYTES (4096)
    let borderline_4095 = "B".repeat(4095);

    let mut session = Session::new();
    let _ = exec(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "d-xy", "title": "XY Data", "kind": "xy", "cols": 2, "replicates": 2, "rows": 3, "auto_info": true, "info_sheet_id": "i-1", "auto_graph": true, "graph_sheet_id": "g-1" }),
    )?;
    let _ = exec(&mut session, "table.setXCells", json!({ "sheet_id": "d-xy", "row": 0, "value": 1.0 }))?;
    let _ = exec(&mut session, "table.setXCells", json!({ "sheet_id": "d-xy", "row": 1, "value": 2.0 }))?;
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "d-xy", "row": 0, "col": 0, "subcol": 0, "value": 10.0 }))?;
    let _ = exec(&mut session, "table.setCell", json!({ "sheet_id": "d-xy", "row": 1, "col": 0, "subcol": 0, "value": 20.0 }))?;
    let _ =
        exec(&mut session, "stats.run", json!({ "sheet_id": "r-1", "source_sheet_id": "d-xy", "kind": "descriptive", "selected_data_sets": [0] }))?;
    let _ = exec(&mut session, "layout.create", json!({ "sheet_id": "l-1", "title": "Layout Title" }))?;
    let _ = exec(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "d-grp", "title": "Grouped", "kind": "grouped", "cols": 2, "replicates": 2, "rows": 2 }),
    )?;
    let _ = exec(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "d-nest", "title": "Nested", "kind": "nested", "cols": 2, "replicates": 2, "rows": 2 }),
    )?;
    let _ = exec(&mut session, "sheet.createData", json!({ "sheet_id": "d-mv", "title": "MV", "kind": "multiple_variables", "cols": 2, "rows": 2 }))?;

    verify_sheet_and_table_title_limits(&mut session, &oversized_ascii, &oversized_utf8)?;
    verify_graph_layout_and_csv_title_limits(&mut session, &oversized_ascii, &oversized_utf8)?;
    verify_duplicate_title_overflows(&mut session, &oversized_ascii, &borderline_4095)?;

    let _ = save_roundtrip(&mut session, &save_path)?;
    Ok(())
}
