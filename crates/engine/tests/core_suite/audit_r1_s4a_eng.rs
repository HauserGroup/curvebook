//! Regression tests for Stage 4A S2 engine validation, external link shrink/grow,
//! graph/layout cycle guards, linked-region write protection, and title limits
//! (`R1-S07-03`, `R1-S07-04`, `R1-S07-07`, `R1-S07-09`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, DataSheet, DescriptiveSpec, GraphSheet, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::{ErrorCode, Session, dag};
use serde_json::json;

use super::common::{TempDir, assert_err_cases, assert_err_code, cell_num, ds, err_cmd, gs, ok_cmd, run_cmds, save_and_reopen};

#[test]
fn r1_s07_03_external_csv_and_tsv_shrink_grow_and_empty_refresh() -> Result<(), String> {
    let dir = TempDir::try_new("s4a-eng")?;
    let (csv_path, tsv_path, offset_csv, save_path) =
        (dir.child("external.csv"), dir.child("external.tsv"), dir.child("offset.csv"), dir.child("project.curvebook"));
    std::fs::write(&csv_path, "1,10,100\n2,20,200\n3,30,300\n4,40,400\n").map_err(|e| e.to_string())?;
    std::fs::write(&tsv_path, "10\t1.5\t2.5\t3.5\n20\t4.5\t5.5\t6.5\n30\t7.5\t8.5\t9.5\n").map_err(|e| e.to_string())?;
    std::fs::write(&offset_csv, "1.25,2.5\n3.75,4.5\n").map_err(|e| e.to_string())?;

    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "d-csv", "kind": "column", "cols": 3, "rows": 4 })),
            ("sheet.createData", json!({ "sheet_id": "d-tsv", "kind": "column", "cols": 4, "rows": 3 })),
            ("table.pasteLink", json!({ "target_sheet_id": "d-csv", "source_path": csv_path.to_string_lossy(), "delimiter": "," })),
            ("table.pasteLink", json!({ "target_sheet_id": "d-tsv", "source_path": tsv_path.to_string_lossy(), "delimiter": "\t" })),
        ],
    );
    assert_eq!(
        (cell_num(&session, "d-csv", 3, 0, 0), cell_num(&session, "d-csv", 3, 2, 0), cell_num(&session, "d-tsv", 2, 3, 0)),
        (Some(4.0), Some(400.0), Some(9.5))
    );

    std::fs::write(&csv_path, "1,11,111\n2,22,222\n").map_err(|e| e.to_string())?;
    std::fs::write(&tsv_path, "99\t8.25\n").map_err(|e| e.to_string())?;
    ok_cmd(&mut session, "project.recalculate", json!({ "include_external_links": true }));
    assert_eq!(
        (
            cell_num(&session, "d-csv", 0, 1, 0),
            cell_num(&session, "d-csv", 1, 2, 0),
            cell_num(&session, "d-csv", 2, 0, 0),
            cell_num(&session, "d-csv", 3, 2, 0)
        ),
        (Some(11.0), Some(222.0), None, None)
    );
    assert_eq!(
        (
            cell_num(&session, "d-tsv", 0, 0, 0),
            cell_num(&session, "d-tsv", 0, 1, 0),
            cell_num(&session, "d-tsv", 0, 2, 0),
            cell_num(&session, "d-tsv", 0, 3, 0),
            cell_num(&session, "d-tsv", 2, 3, 0)
        ),
        (Some(99.0), Some(8.25), None, None, None)
    );

    // Shrink to empty (0 rows) then grow back.
    std::fs::write(&csv_path, "").map_err(|e| e.to_string())?;
    ok_cmd(&mut session, "project.recalculate", json!({ "include_external_links": true }));
    assert_eq!((cell_num(&session, "d-csv", 0, 0, 0), cell_num(&session, "d-csv", 1, 2, 0)), (None, None));
    std::fs::write(&csv_path, "5,55,555\n6,66,666\n7,77,777\n").map_err(|e| e.to_string())?;
    ok_cmd(&mut session, "project.recalculate", json!({ "include_external_links": true }));
    assert_eq!(cell_num(&session, "d-csv", 2, 2, 0), Some(777.0));

    // Offset placement via importCsv { link: true, replace: false }.
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "d-offset", "kind": "column", "cols": 4, "rows": 5 })),
            (
                "table.importCsv",
                json!({ "sheet_id": "d-offset", "path": offset_csv.to_string_lossy(), "delimiter": ",", "replace": false, "link": true, "has_header": false, "start_row": 1, "start_col": 1 }),
            ),
        ],
    );
    assert_eq!((cell_num(&session, "d-offset", 1, 1, 0), cell_num(&session, "d-offset", 2, 2, 0)), (Some(1.25), Some(4.5)));
    std::fs::write(&offset_csv, "9.5,8.5\n").map_err(|e| e.to_string())?;
    ok_cmd(&mut session, "project.recalculate", json!({ "include_external_links": true }));
    assert_eq!((cell_num(&session, "d-offset", 1, 1, 0), cell_num(&session, "d-offset", 2, 2, 0)), (Some(9.5), None));
    assert_eq!(save_and_reopen(&mut session, &save_path).document().data_sheets.len(), 3);
    Ok(())
}

#[test]
fn r1_s07_04_graph_and_layout_reject_self_refs_wrong_sections_and_cycles() -> Result<(), String> {
    let dir = TempDir::try_new("s4a-eng")?;
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "d", "kind": "column", "cols": 2, "rows": 3, "auto_info": true })),
            ("table.setCells", json!({ "sheet_id": "d", "rows": [[10.0, 20.0], [12.0, 24.0]] })),
            ("graph.create", json!({ "sheet_id": "g", "data_sheet_id": "d", "title": "Main Graph" })),
            ("graph.create", json!({ "sheet_id": "g2", "data_sheet_id": "d", "title": "Other Graph" })),
            ("layout.create", json!({ "sheet_id": "l", "title": "Main Layout" })),
            ("stats.run", json!({ "sheet_id": "res", "source_sheet_id": "d", "kind": "descriptive", "selected_data_sets": [0, 1] })),
        ],
    );
    let before_graph = gs(&session, "g").clone();
    let gdo = &before_graph.spec.series[0].gdo;

    assert_err_cases(
        &mut session,
        &[
            ("graph.setSpec", json!({ "sheet_id": "g", "primary_source_sheet_id": "g" }), ErrorCode::CycleDetected),
            ("graph.setSpec", json!({ "sheet_id": "g", "spec": { "primary_source_sheet_id": "g" } }), ErrorCode::CycleDetected),
            ("graph.setSpec", json!({ "sheet_id": "g", "spec": { "primary_source_sheet_id": "nope" } }), ErrorCode::NotFound),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g", "series": [{ "source_sheet_id": "g", "data_set_index": 0, "legend_label": null, "bind_to_right_y2": false, "gdo": gdo }] }),
                ErrorCode::CycleDetected,
            ),
            ("graph.setSeries", json!({ "sheet_id": "g", "series_index": 0, "source_sheet_id": "g" }), ErrorCode::CycleDetected),
            ("graph.setSeries", json!({ "sheet_id": "g", "series": [{ "source_sheet_id": "g", "data_set_index": 0 }] }), ErrorCode::CycleDetected),
            (
                "graph.setSeries",
                json!({ "sheet_id": "g", "series_index": 0, "regression_overlay": { "result_sheet_id": "g", "sample_segments": 256, "show_confidence_band": false, "show_prediction_band": false, "shaded_ribbon": false } }),
                ErrorCode::CycleDetected,
            ),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g", "series": [{ "source_sheet_id": "d", "data_set_index": 0, "legend_label": null, "bind_to_right_y2": false, "gdo": gdo, "regression_overlay": { "result_sheet_id": "g", "sample_segments": 256, "show_confidence_band": false, "show_prediction_band": false, "shaded_ribbon": false } }] }),
                ErrorCode::CycleDetected,
            ),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g", "comparisons": [{ "result_sheet_id": "g", "group_a": 0, "group_b": 1, "style": "Square", "label_mode": "SummaryStars", "hide_if_non_significant": false, "stroke_width_pt": 1.0, "font_size_pt": 9.0, "bracket_step_pt": 10.0 }] }),
                ErrorCode::CycleDetected,
            ),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g", "annotations": [{ "EmbeddedResultTable": { "result_sheet_id": "g", "tab_id": null, "bounds_pt": { "x": 10.0, "y": 10.0, "width": 100.0, "height": 50.0 } } }] }),
                ErrorCode::CycleDetected,
            ),
            ("graph.setSpec", json!({ "sheet_id": "g", "spec": { "title": "Fit {{result:g:mean}}" } }), ErrorCode::CycleDetected),
            ("graph.setAxes", json!({ "sheet_id": "g", "axis": "y", "title": "Y {{info:g:dose}}" }), ErrorCode::CycleDetected),
            (
                "graph.addAnnotation",
                json!({ "sheet_id": "g", "annotation": { "TextBox": { "bounds_pt": { "x": 5.0, "y": 5.0, "width": 80.0, "height": 20.0 }, "markdown": "{{result:g:EC50}}", "bordered": false } } }),
                ErrorCode::CycleDetected,
            ),
            ("graph.setSpec", json!({ "sheet_id": "g", "primary_source_sheet_id": "g2" }), ErrorCode::BadInput),
            ("graph.setSpec", json!({ "sheet_id": "g", "primary_source_sheet_id": "missing-sheet" }), ErrorCode::NotFound),
            (
                "layout.addPanel",
                json!({ "sheet_id": "l", "kind": "table", "source_sheet_id": "l", "bounds_pt": [10.0, 10.0, 100.0, 80.0] }),
                ErrorCode::CycleDetected,
            ),
            (
                "layout.setSpec",
                json!({ "sheet_id": "l", "items": [{ "LiveTable": { "id": "bad", "source_sheet_id": "g", "tab_id": null, "bounds_pt": { "x": 10.0, "y": 10.0, "width": 100.0, "height": 80.0 } } }] }),
                ErrorCode::BadInput,
            ),
            (
                "layout.addPanel",
                json!({ "sheet_id": "l", "kind": "text", "markdown": "Bad {{info:l:key}}", "bounds_pt": [10.0, 10.0, 100.0, 40.0] }),
                ErrorCode::CycleDetected,
            ),
        ],
    );
    assert_ne!(
        err_cmd(&mut session, "graph.setRegressionOverlay", json!({ "sheet_id": "g", "series_index": 0, "result_sheet_id": "d" })).code,
        ErrorCode::CycleDetected
    );
    assert_eq!(gs(&session, "g").spec, before_graph.spec);
    let _ = save_and_reopen(&mut session, &dir.child("valid.curvebook"));

    // Multi-hop cycle detection in dag::topological_order & dag::recalculate_project.
    let mut cycle_doc = ProjectDocument::new("Cycle").map_err(|e| e.to_string())?;
    cycle_doc
        .add_data_sheet(
            DataSheet::new("d", "D", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2).map_err(|e| e.to_string())?))
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    let mut g = GraphSheet::new_uninitialized_for_data("g", &cycle_doc.data_sheets[0]).map_err(|e| e.to_string())?;
    g.spec.primary_source_sheet_id = SheetId::new("res-cycle");
    cycle_doc.add_graph_sheet(g).map_err(|e| e.to_string())?;
    cycle_doc
        .add_result_sheet(
            ResultSheet::new(
                AnalysisSpec::new("res-cycle", "g", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    assert_eq!(dag::topological_order(&cycle_doc).unwrap_err().code, ErrorCode::CycleDetected);
    assert_eq!(dag::recalculate_project(&mut cycle_doc).unwrap_err().code, ErrorCode::CycleDetected);
    Ok(())
}

#[test]
fn r1_s07_07_linked_region_rejects_edits_exclusions_clears_pastes_and_recodes() -> Result<(), String> {
    let dir = TempDir::try_new("s4a-eng")?;
    let csv_path = dir.child("mv.csv");
    std::fs::write(&csv_path, "A,10\nB,20\n").map_err(|e| e.to_string())?;

    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "src", "kind": "xy", "cols": 1, "rows": 3 })),
            ("table.setXCells", json!({ "sheet_id": "src", "values": [1.0, 2.0, 3.0] })),
            ("table.setCells", json!({ "sheet_id": "src", "rows": [[10.0], [20.0], [30.0]] })),
            ("sheet.createData", json!({ "sheet_id": "dst", "kind": "xy", "cols": 2, "replicates": 2, "rows": 4 })),
            (
                "table.pasteLink",
                json!({ "target_sheet_id": "dst", "source_sheet_id": "src", "source_range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 2, "col": 0, "subcol": 0 } }, "start_row": 0, "start_col": 0, "start_subcol": 0 }),
            ),
            ("sheet.createData", json!({ "sheet_id": "mv-linked", "kind": "multiple_variables", "cols": 2, "rows": 2 })),
            (
                "table.importCsv",
                json!({ "sheet_id": "mv-linked", "path": csv_path.to_string_lossy(), "delimiter": ",", "replace": false, "has_header": false, "link": true }),
            ),
        ],
    );

    assert_err_cases(
        &mut session,
        &[
            ("table.setCell", json!({ "sheet_id": "dst", "row": 1, "col": 0, "subcol": 0, "value": 999.0 }), ErrorCode::Conflict),
            (
                "table.setCells",
                json!({ "sheet_id": "dst", "edits": [{ "row": 0, "col": 0, "subcol": 1, "value": 77.0 }, { "row": 2, "col": 0, "subcol": 0, "value": 888.0 }] }),
                ErrorCode::Conflict,
            ),
            ("table.excludeCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "subcol": 0, "excluded": true }), ErrorCode::Conflict),
            (
                "table.excludeCells",
                json!({ "sheet_id": "dst", "cells": [{ "row": 0, "col": 0, "subcol": 1 }, { "row": 1, "col": 0, "subcol": 0 }], "excluded": true }),
                ErrorCode::Conflict,
            ),
            (
                "table.excludeCells",
                json!({ "sheet_id": "dst", "row_start": 0, "row_end": 1, "col_start": 0, "col_end": 0, "subcol_start": 0, "subcol_end": 1, "excluded": true }),
                ErrorCode::Conflict,
            ),
            (
                "table.excludeCellsWithReason",
                json!({ "sheet_id": "dst", "row": 1, "col": 0, "subcol": 0, "reason_tag": "outlier" }),
                ErrorCode::Conflict,
            ),
            ("table.clearCells", json!({ "sheet_id": "dst", "row": 1, "col": 0, "subcol": 0 }), ErrorCode::Conflict),
            ("table.clearCells", json!({ "sheet_id": "dst", "cells": [{ "row": 1, "col": 0, "subcol": 0 }] }), ErrorCode::Conflict),
            (
                "table.clearCells",
                json!({ "sheet_id": "dst", "row_start": 1, "row_end": 2, "col_start": 0, "col_end": 0, "subcol_start": 0, "subcol_end": 0 }),
                ErrorCode::Conflict,
            ),
            ("table.clearCells", json!({ "sheet_id": "dst", "all": true }), ErrorCode::Conflict),
            (
                "table.paste",
                json!({ "sheet_id": "dst", "text": "55\n66\n", "start_row": 1, "start_col": 0, "start_subcol": 0, "mode": "overwrite" }),
                ErrorCode::Conflict,
            ),
            (
                "table.paste",
                json!({ "sheet_id": "dst", "text": "99\n", "start_row": 0, "start_col": 0, "start_subcol": 0, "mode": "insert_rows" }),
                ErrorCode::Conflict,
            ),
            (
                "table.importCsv",
                json!({ "sheet_id": "dst", "csv": "91\n92\n", "has_header": false, "replace": false, "link": false, "start_row": 0, "start_col": 0, "start_subcol": 0 }),
                ErrorCode::Conflict,
            ),
            ("table.recodeLevels", json!({ "sheet_id": "mv-linked", "col": 0, "mappings": { "A": "Alpha" } }), ErrorCode::Conflict),
        ],
    );
    assert_eq!((cell_num(&session, "dst", 0, 0, 1), cell_num(&session, "dst", 2, 0, 0)), (None, Some(30.0)));

    // Adjacent unlinked edits succeed, round-trip preserves link, and unlink+clear detaches it cleanly.
    run_cmds(
        &mut session,
        &[
            ("table.setCell", json!({ "sheet_id": "dst", "row": 0, "col": 0, "subcol": 1, "value": 42.0 })),
            ("table.excludeCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "subcol": 1, "excluded": true })),
            ("table.setCell", json!({ "sheet_id": "dst", "row": 3, "col": 0, "subcol": 0, "value": 99.0 })),
            ("table.setCell", json!({ "sheet_id": "dst", "row": 0, "col": 1, "subcol": 0, "value": 5.0 })),
            ("table.setXCells", json!({ "sheet_id": "dst", "row": 0, "value": 1.0 })),
            ("table.excludeCells", json!({ "sheet_id": "dst", "target": "x", "row": 0, "excluded": true })),
        ],
    );
    let reloaded = save_and_reopen(&mut session, &dir.child("linked.curvebook"));
    assert_eq!((ds(&reloaded, "dst").linked_regions.len(), cell_num(&reloaded, "dst", 1, 0, 0)), (1, Some(20.0)));

    run_cmds(
        &mut session,
        &[
            ("table.clearCells", json!({ "sheet_id": "dst", "all": true, "unlink": true })),
            ("table.setCell", json!({ "sheet_id": "dst", "row": 1, "col": 0, "subcol": 0, "value": 123.0 })),
        ],
    );
    assert!(ds(&session, "dst").linked_regions.is_empty() && cell_num(&session, "dst", 1, 0, 0) == Some(123.0));
    Ok(())
}

#[test]
fn r1_s07_09_title_validation_enforced_across_all_sheets_tables_graphs_and_layouts() -> Result<(), String> {
    let dir = TempDir::try_new("s4a-eng")?;
    let (over_ascii, over_utf8, border_4095) = ("T".repeat(4097), "é".repeat(2049), "B".repeat(4095));

    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "d-xy", "title": "XY Data", "kind": "xy", "cols": 2, "replicates": 2, "rows": 3, "auto_info": true, "info_sheet_id": "i-1", "auto_graph": true, "graph_sheet_id": "g-1" }),
            ),
            ("table.setXCells", json!({ "sheet_id": "d-xy", "values": [1.0, 2.0] })),
            (
                "table.setCells",
                json!({ "sheet_id": "d-xy", "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 10.0 }, { "row": 1, "col": 0, "subcol": 0, "value": 20.0 }] }),
            ),
            ("stats.run", json!({ "sheet_id": "r-1", "source_sheet_id": "d-xy", "kind": "descriptive", "selected_data_sets": [0] })),
            ("layout.create", json!({ "sheet_id": "l-1", "title": "Layout Title" })),
            ("sheet.createData", json!({ "sheet_id": "d-grp", "title": "Grouped", "kind": "grouped", "cols": 2, "replicates": 2, "rows": 2 })),
            ("sheet.createData", json!({ "sheet_id": "d-nest", "title": "Nested", "kind": "nested", "cols": 2, "replicates": 2, "rows": 2 })),
            ("sheet.createData", json!({ "sheet_id": "d-mv", "title": "MV", "kind": "multiple_variables", "cols": 2, "rows": 2 })),
        ],
    );

    for sid in ["d-xy", "i-1", "r-1", "g-1", "l-1"] {
        assert_err_code(&mut session, "sheet.rename", json!({ "sheet_id": sid, "title": over_ascii }), ErrorCode::ResourceLimit);
        assert_err_code(&mut session, "sheet.rename", json!({ "sheet_id": sid, "title": over_utf8 }), ErrorCode::ResourceLimit);
    }

    assert_err_cases(
        &mut session,
        &[
            ("table.setGroupTitles", json!({ "sheet_id": "d-xy", "titles": ["GoodFirstCol", over_ascii] }), ErrorCode::ResourceLimit),
            ("table.setTitles", json!({ "sheet_id": "d-xy", "col": 0, "title": over_utf8 }), ErrorCode::ResourceLimit),
            ("table.setGroupTitles", json!({ "sheet_id": "d-xy", "x_title": over_ascii }), ErrorCode::ResourceLimit),
            ("table.setGroupTitles", json!({ "sheet_id": "d-grp", "row_factor_title": over_ascii }), ErrorCode::ResourceLimit),
            ("table.setGroupTitles", json!({ "sheet_id": "d-grp", "column_factor_title": over_utf8 }), ErrorCode::ResourceLimit),
            ("table.setSubcolumnTitles", json!({ "sheet_id": "d-nest", "col": 0, "titles": ["FirstSubOk", over_ascii] }), ErrorCode::ResourceLimit),
            ("table.setRowTitles", json!({ "sheet_id": "d-xy", "row": 0, "title": over_ascii }), ErrorCode::ResourceLimit),
            ("table.setCalculatedVariable", json!({ "sheet_id": "d-mv", "title": over_ascii, "formula": "1 + 1" }), ErrorCode::ResourceLimit),
            ("graph.create", json!({ "sheet_id": "g-bad", "data_sheet_id": "d-xy", "title": over_ascii }), ErrorCode::ResourceLimit),
            ("graph.initialize", json!({ "sheet_id": "g-1", "title": over_utf8 }), ErrorCode::ResourceLimit),
            ("graph.setSpec", json!({ "sheet_id": "g-1", "title": over_ascii }), ErrorCode::ResourceLimit),
            ("graph.setSpec", json!({ "sheet_id": "g-1", "spec": { "title": over_utf8 } }), ErrorCode::ResourceLimit),
            ("graph.setAxes", json!({ "sheet_id": "g-1", "axis": "x", "title": over_ascii }), ErrorCode::ResourceLimit),
            ("layout.create", json!({ "sheet_id": "l-bad", "title": over_ascii }), ErrorCode::ResourceLimit),
            ("layout.setSpec", json!({ "sheet_id": "l-1", "title": over_utf8 }), ErrorCode::ResourceLimit),
            ("table.importCsv", json!({ "sheet_id": "d-csv-bad", "csv": "A,B\n1,2\n", "title": over_ascii }), ErrorCode::ResourceLimit),
            (
                "table.importCsv",
                json!({ "sheet_id": "d-csv-bad-hdr", "csv": format!("{over_ascii},B\n1,2\n"), "has_header": true }),
                ErrorCode::ResourceLimit,
            ),
            (
                "sheet.createData",
                json!({ "sheet_id": "d-4095-info", "title": border_4095, "kind": "column", "auto_info": true }),
                ErrorCode::ResourceLimit,
            ),
        ],
    );
    assert!(matches!(&ds(&session, "d-xy").archetype, TableArchetype::Xy(xy) if xy.data_sets[0].title != "GoodFirstCol"));
    assert!(matches!(&ds(&session, "d-nest").archetype, TableArchetype::Nested(nt) if nt.groups[0].subcolumn_titles[0] != "FirstSubOk"));
    assert!(
        session.document().graph_sheet(&SheetId::new("g-bad")).is_none()
            && session.document().data_sheet(&SheetId::new("d-csv-bad-hdr")).is_none()
            && session.document().data_sheet(&SheetId::new("d-4095-info")).is_none()
    );

    for (sid, copy_id) in [("d-xy", "d-copy"), ("i-1", "i-copy"), ("r-1", "r-copy"), ("g-1", "g-copy"), ("l-1", "l-copy")] {
        assert_err_code(
            &mut session,
            "sheet.duplicate",
            json!({ "sheet_id": sid, "new_sheet_id": copy_id, "title": over_ascii }),
            ErrorCode::ResourceLimit,
        );
        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": sid, "title": border_4095 }));
        assert_err_code(&mut session, "sheet.duplicate", json!({ "sheet_id": sid, "new_sheet_id": copy_id }), ErrorCode::ResourceLimit);
        assert!(session.document().sheet_section(&SheetId::new(copy_id)).is_none());
    }
    let count_before = session.document().total_sheet_count();
    assert_err_code(&mut session, "sheet.duplicateFamily", json!({ "sheet_id": "d-xy", "new_sheet_id": "d-fam" }), ErrorCode::ResourceLimit);
    assert_eq!(session.document().total_sheet_count(), count_before);
    let _ = save_and_reopen(&mut session, &dir.child("titles.curvebook"));
    Ok(())
}
