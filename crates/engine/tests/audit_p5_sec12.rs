#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::error::Error;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec,
    ErrorCode, MAX_PROJECT_CELLS, MultipleVariablesTable, MultivariatePlotMode, NestedTable, ProjectDocument, ResultCell, ResultSheet, ResultStatus,
    ResultTab, ResultTabKind, SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype, VariableRole, XColumnMode, XyTable,
};
use curvebook_engine::{EngineError, Session, cmd::table};
use curvebook_format::{MAX_CSV_FILE_BYTES, MAX_XML_BYTES};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

fn ok_cmd(session: &mut Session, cmd: &str, params: Value) -> Result<Value, Box<dyn Error>> {
    Ok(session.execute(cmd, params)?.payload)
}

fn err_cmd(session: &mut Session, cmd: &str, params: Value) -> Result<EngineError, Box<dyn Error>> {
    match session.execute(cmd, params) {
        Ok(out) => Err(format!("expected `{cmd}` to fail, got payload: {:?}", out.payload).into()),
        Err(err) => Ok(err),
    }
}

fn seed_xy_sheet(id: &str, title: &str, data_sets: usize, y_sub: SubcolumnMode, rows: usize) -> Result<DataSheet, Box<dyn Error>> {
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, y_sub, data_sets, rows)?;
    let subs = y_sub.subcolumn_count();
    for r in 0..rows {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        for c in 0..data_sets {
            for s in 0..subs {
                xy.data_sets[c].cells[r][s] = Cell::number(((c + 1) * 10 + r) as f64 + s as f64 * 0.1);
            }
        }
    }
    Ok(DataSheet::new(id, title, TableArchetype::Xy(xy))?)
}

#[test]
fn fa_sec12_01_convert_archetype_remaps_point_overrides_and_propagates_multivariate_errors() -> TestResult {
    // 1. 1-Y XY -> Column: Y0 shifts 0 -> 1, so point override must track series_index 0 -> 1 instead of pruning.
    let mut doc = ProjectDocument::new("SEC12-01")?;
    doc.add_data_sheet(seed_xy_sheet("xy1", "Single Y", 1, SubcolumnMode::Single, 4)?)?;
    // 2. 2-Y XY -> Column: Y0 shifts 0 -> 1, Y1 shifts 1 -> 2; overrides on both must remap cleanly (Y1 must NOT mis-target Y0).
    doc.add_data_sheet(seed_xy_sheet("xy2", "Dual Y", 2, SubcolumnMode::Single, 4)?)?;
    // 3. XY RawReplicates(2) -> Column: Y0 expands to columns 1 and 2; subcol override remaps to expanded column.
    doc.add_data_sheet(seed_xy_sheet("xy_rep", "Rep Y", 1, SubcolumnMode::RawReplicates(2), 3)?)?;
    // 4. Column (2 cols) -> XY: col 0 moves to X (MovedToX), col 1 -> Y0 (col 0).
    let mut col2 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    for r in 0..3 {
        col2.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        col2.data_sets[1].cells[r][0] = Cell::number((r + 1) as f64 * 5.0);
    }
    doc.add_data_sheet(DataSheet::new("col2", "Two Cols", TableArchetype::Column(col2))?)?;
    // 5. MV (Categorical + 2 Numeric) -> Grouped: Categorical collapses columns -> Unmappable for spec.multivariate.
    let mut mv = MultipleVariablesTable::new(3, 3)?;
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[0].title = String::from("Group");
    mv.variables[0].cells = vec![Cell::text("A"), Cell::text("B"), Cell::text("A")];
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("Dose");
    mv.variables[1].cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)];
    mv.variables[2].role = VariableRole::Continuous;
    mv.variables[2].title = String::from("Resp");
    mv.variables[2].cells = vec![Cell::number(10.0), Cell::number(20.0), Cell::number(30.0)];
    doc.add_data_sheet(DataSheet::new("mv1", "MV Sheet", TableArchetype::MultipleVariables(mv))?)?;

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy1", "kind": "xy" }))?;
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g1", "points": [{ "series_index": 0, "point_index": 2 }], "hidden": true }),
    )?;

    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g2", "data_sheet_id": "xy2", "kind": "xy" }))?;
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g2", "points": [{ "series_index": 0, "point_index": 1 }], "color": "#ff0000" }),
    )?;
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g2", "points": [{ "series_index": 1, "point_index": 2 }], "hidden": true }),
    )?;

    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "grep", "data_sheet_id": "xy_rep", "kind": "xy" }))?;
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "grep", "points": [{ "series_index": 0, "point_index": 1, "subcol": 1 }], "hidden": true }),
    )?;

    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "gcol", "data_sheet_id": "col2", "kind": "column" }))?;
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "gcol", "points": [{ "series_index": 0, "point_index": 0 }], "hidden": true }),
    )?;
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "gcol", "points": [{ "series_index": 1, "point_index": 2 }], "color": "#00ff00" }),
    )?;

    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "gmv", "data_sheet_id": "mv1", "kind": "multiple_variables" }))?;
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "gmv",
            "spec": {
                "multivariate": {
                    "plot_mode": MultivariatePlotMode::BubbleScatter,
                    "x_column": 1,
                    "y_column": 2,
                    "color": ColorChannelMapping::Categorical {
                        column: 0,
                        palette: curvebook_palette::PaletteId::OkabeIto,
                        level_order_override: None,
                    },
                    "size": SizeChannelMapping::Variable { column: 2, min_radius_pt: 3.0, max_radius_pt: 12.0 },
                    "label_column": 0,
                    "x_level_order_override": null,
                    "show_convex_hulls": false,
                    "show_confidence_ellipses": false
                }
            }
        }),
    )?;

    ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "xy1", "target_kind": "Column", "allow_lossy": true }))?;
    ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "xy2", "target_kind": "Column", "allow_lossy": true }))?;
    ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "xy_rep", "target_kind": "Column", "allow_lossy": true }))?;
    ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "col2", "target_kind": "XY", "allow_lossy": true }))?;
    ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "mv1", "target_kind": "Grouped", "allow_lossy": true }))?;

    let g1 = session.document().graph_sheet(&SheetId::new("g1")).ok_or("missing g1")?;
    assert_eq!(g1.spec.series.iter().map(|s| s.data_set_index).collect::<Vec<_>>(), vec![1]);
    assert_eq!(g1.spec.point_overrides.len(), 1);
    assert_eq!(g1.spec.point_overrides[0].series_index, 1);
    assert_eq!(g1.spec.point_overrides[0].point_index, 2);
    assert!(g1.spec.point_overrides[0].hidden);

    let g2 = session.document().graph_sheet(&SheetId::new("g2")).ok_or("missing g2")?;
    assert_eq!(g2.spec.series.iter().map(|s| s.data_set_index).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(g2.spec.point_overrides.len(), 2);
    assert_eq!(g2.spec.point_overrides[0].series_index, 1);
    assert_eq!(g2.spec.point_overrides[0].point_index, 1);
    assert_eq!(g2.spec.point_overrides[1].series_index, 2);
    assert_eq!(g2.spec.point_overrides[1].point_index, 2);
    assert!(g2.spec.point_overrides[1].hidden);

    let grep = session.document().graph_sheet(&SheetId::new("grep")).ok_or("missing grep")?;
    assert_eq!(grep.spec.series.iter().map(|s| s.data_set_index).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(grep.spec.point_overrides.len(), 1);
    assert_eq!(grep.spec.point_overrides[0].series_index, 2);
    assert_eq!(grep.spec.point_overrides[0].point_index, 1);
    assert_eq!(grep.spec.point_overrides[0].subcol, None);

    let gcol = session.document().graph_sheet(&SheetId::new("gcol")).ok_or("missing gcol")?;
    assert_eq!(gcol.spec.series.iter().map(|s| s.data_set_index).collect::<Vec<_>>(), vec![0]);
    assert_eq!(gcol.spec.point_overrides.len(), 1);
    assert_eq!(gcol.spec.point_overrides[0].series_index, 0);
    assert_eq!(gcol.spec.point_overrides[0].point_index, 2);
    assert!(!gcol.spec.point_overrides[0].hidden);

    let gmv = session.document().graph_sheet(&SheetId::new("gmv")).ok_or("missing gmv")?;
    assert!(gmv.spec.multivariate.is_none(), "unmappable multivariate channel indices must be cleared instead of kept out-of-bounds");
    Ok(())
}

#[test]
fn fa_sec12_02_project_cell_budget_enforced_across_import_replace_and_generators() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-02")?;
    doc.add_data_sheet(DataSheet::new(
        "big-1",
        "Big 1",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 25, 1_000_000)?),
    )?)?;
    doc.add_data_sheet(DataSheet::new(
        "big-2",
        "Big 2",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 24, 1_000_000)?),
    )?)?;
    doc.add_data_sheet(DataSheet::new(
        "big-3",
        "Big 3",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 100, 9_999)?),
    )?)?;
    doc.add_data_sheet(DataSheet::new("mv-small", "MV Small", TableArchetype::MultipleVariables(MultipleVariablesTable::new(1, 20)?))?)?;

    let mut small = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 70)?;
    for r in 0..70 {
        small.data_sets[0].cells[r][0] = Cell::number(r as f64);
    }
    doc.add_data_sheet(DataSheet::new("small", "Small", TableArchetype::Column(small))?)?;
    assert_eq!(doc.total_allocated_cells()?, MAX_PROJECT_CELLS - 10);

    let mut session = Session::from_document(doc);
    let csv_20_rows: String = (0..20).map(|i| format!("{i},{}\n", i * 2)).collect();
    let csv_45_rows: String = (0..45).map(|i| format!("{i},{}\n", i * 2)).collect();
    let roles96: Vec<Value> = (0..96)
        .map(|idx| {
            if idx < 4 {
                json!("Blank")
            } else {
                json!({ "role": "treatment", "group_name": "G1", "dilution_x": (idx % 12 + 1) as f64, "replicate": 1 })
            }
        })
        .collect();
    let plate_vals: Vec<f64> = (0..96).map(|i| i as f64).collect();
    let tidy_rows: Vec<Value> = (0..20).map(|i| json!(["G1", i as f64, 100.0 + i as f64])).collect();

    let e1 = err_cmd(&mut session, "table.importCsv", json!({ "sheet_id": "new-csv", "csv": csv_20_rows, "archetype": "column" }))?;
    assert_eq!(e1.code, ErrorCode::ResourceLimit);

    let e2 = err_cmd(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "small", "csv": csv_45_rows, "archetype": "column", "has_header": false, "replace": true }),
    )?;
    assert_eq!(e2.code, ErrorCode::ResourceLimit);

    let e3 = err_cmd(&mut session, "table.importPlateMap", json!({ "geometry": 96, "values": plate_vals, "roles": roles96 }))?;
    assert_eq!(e3.code, ErrorCode::ResourceLimit);

    let e4 = err_cmd(
        &mut session,
        "table.importTidyLong",
        json!({ "headers": ["Group", "X", "Val"], "rows": tidy_rows, "group_col": "Group", "value_col": "Val" }),
    )?;
    assert_eq!(e4.code, ErrorCode::ResourceLimit);

    let e5 = err_cmd(&mut session, "table.simulateXy", json!({ "row_count": 10, "replicates": 2 }))?;
    assert_eq!(e5.code, ErrorCode::ResourceLimit);
    let e5_overwrite = err_cmd(&mut session, "table.simulateXy", json!({ "sheet_id": "small", "row_count": 30, "replicates": 3 }))?;
    assert_eq!(e5_overwrite.code, ErrorCode::ResourceLimit);

    let info_before = session.document().info_sheets.len();
    let e6 = err_cmd(&mut session, "table.randomizeGroups", json!({ "groups": ["Ctrl", "Drug"], "subject_count": 12 }))?;
    assert_eq!(e6.code, ErrorCode::ResourceLimit);
    assert_eq!(session.document().info_sheets.len(), info_before, "randomizeGroups failure must not leave orphan InfoSheet");

    let e7 = err_cmd(&mut session, "table.randomSampleRows", json!({ "source_sheet_id": "small", "sample_size": 20 }))?;
    assert_eq!(e7.code, ErrorCode::ResourceLimit);
    assert_eq!(session.document().info_sheets.len(), info_before, "randomSampleRows failure must not leave orphan InfoSheet");

    let e8 = err_cmd(
        &mut session,
        "table.simulateDistributions",
        json!({
            "n_rows": 10,
            "columns": [
                { "name": "A", "distribution": "gaussian", "mean": 0.0, "sd": 1.0 },
                { "name": "B", "distribution": "gaussian", "mean": 1.0, "sd": 1.0 }
            ]
        }),
    )?;
    assert_eq!(e8.code, ErrorCode::ResourceLimit);

    let e9 = err_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-small", "title": "CalcOver", "formula": "2" }))?;
    assert_eq!(e9.code, ErrorCode::ResourceLimit);

    let e10 = err_cmd(&mut session, "table.paste", json!({ "sheet_id": "small", "mode": "append_rows", "text": csv_20_rows }))?;
    assert_eq!(e10.code, ErrorCode::ResourceLimit);

    let e11 = err_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "small", "source_sheet_id": "mv-small", "mode": "append_rows" }))?;
    assert_eq!(e11.code, ErrorCode::ResourceLimit);

    assert_eq!(session.document().total_allocated_cells()?, MAX_PROJECT_CELLS - 10);
    Ok(())
}

#[test]
fn fa_sec12_03_inline_payload_byte_ceilings_enforced() -> TestResult {
    let mut session = Session::new();
    let big_csv = "a".repeat(MAX_CSV_FILE_BYTES + 1);
    let e_csv = err_cmd(&mut session, "table.importCsv", json!({ "csv": big_csv }))?;
    assert_eq!(e_csv.code, ErrorCode::ResourceLimit);

    let big_tidy = "a".repeat(MAX_CSV_FILE_BYTES + 1);
    let e_tidy = err_cmd(&mut session, "table.importTidyLong", json!({ "csv": big_tidy, "group_col": 0, "value_col": 1 }))?;
    assert_eq!(e_tidy.code, ErrorCode::ResourceLimit);

    let big_xml = "a".repeat(MAX_XML_BYTES + 1);
    let e_xml = err_cmd(&mut session, "table.importPzfx", json!({ "xml": big_xml }))?;
    assert_eq!(e_xml.code, ErrorCode::ResourceLimit);
    Ok(())
}

#[test]
fn fa_sec12_04_set_calculated_variable_rejects_linked_region_overlap() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-04")?;
    let mut src_mv = MultipleVariablesTable::new(2, 3)?;
    src_mv.variables[0].title = String::from("X");
    src_mv.variables[1].title = String::from("LinkedSrc");
    src_mv.variables[1].cells = vec![Cell::number(10.0), Cell::number(20.0), Cell::number(30.0)];
    doc.add_data_sheet(DataSheet::new("src", "Source", TableArchetype::MultipleVariables(src_mv))?)?;

    let mut dst_mv = MultipleVariablesTable::new(2, 3)?;
    dst_mv.variables[0].title = String::from("VarA");
    dst_mv.variables[0].cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)];
    dst_mv.variables[1].title = String::from("LinkedCol");
    doc.add_data_sheet(DataSheet::new("dst", "Dest", TableArchetype::MultipleVariables(dst_mv))?)?;

    let mut session = Session::from_document(doc);
    ok_cmd(
        &mut session,
        "table.pasteLink",
        json!({
            "source_sheet_id": "src",
            "target_sheet_id": "dst",
            "source_range": { "start": { "row": 0, "col": 1 }, "end": { "row": 2, "col": 1 } },
            "start_row": 0,
            "start_col": 1
        }),
    )?;

    let by_col = err_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "dst", "col": 1, "formula": "[VarA] * 3" }))?;
    assert_eq!(by_col.code, ErrorCode::Conflict);

    let by_title = err_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "dst", "title": "LinkedCol", "formula": "[VarA] * 3" }))?;
    assert_eq!(by_title.code, ErrorCode::Conflict);

    // Creating a new unlinked column succeeds.
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "dst", "title": "UnlinkedCalc", "formula": "[VarA] * 2" }))?;
    let dst = session.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?;
    assert_eq!(dst.archetype.data_cell(CellCoord::new(0, 1, 0)).map(|c| &c.value), Some(&CellValue::Number(10.0)));
    Ok(())
}

#[test]
fn fa_sec12_05_graph_patchers_reject_omitted_axis_unknown_keys_and_mistyped_values() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-05")?;
    doc.add_data_sheet(seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 4)?)?;
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy1", "kind": "xy" }))?;

    for bad in [
        json!({ "sheet_id": "g1", "title": "Orphan Title" }),
        json!({ "sheet_id": "g1", "x_axis": { "bogus": 1 } }),
        json!({ "sheet_id": "g1", "x_axis": { "stroke_width_pt": "thick" } }),
        json!({ "sheet_id": "g1", "x_axis": { "minor_ticks_per_interval": "two" } }),
        json!({ "sheet_id": "g1", "x_axis": { "segments": "bad" } }),
        json!({ "sheet_id": "g1", "x_axis": { "segments": [{ "scale": "linear", "bogus": 1 }] } }),
        json!({ "sheet_id": "g1", "x_axis": { "thresholds": [{ "value": 1.0, "bogus": 1 }] } }),
        json!({ "sheet_id": "g1", "x_axis": { "additional_ticks": [{ "value": 1.0, "bogus": 1 }] } }),
    ] {
        let err = err_cmd(&mut session, "graph.setAxes", bad)?;
        assert_eq!(err.code, ErrorCode::BadInput);
    }

    for bad in [
        json!({ "sheet_id": "g1", "bogus": 1 }),
        json!({ "sheet_id": "g1", "series": [{ "series_index": 0, "bogus": 1 }] }),
        json!({ "sheet_id": "g1", "series_index": 0, "bind_to_right_y2": "yes" }),
        json!({ "sheet_id": "g1", "series_index": 0, "marker": 5 }),
        json!({ "sheet_id": "g1", "series_index": 0, "marker": { "bogus": 1 } }),
        json!({ "sheet_id": "g1", "series_index": 0, "marker": { "size_pt": "big" } }),
        json!({ "sheet_id": "g1", "series_index": 0, "connecting_line": 5 }),
        json!({ "sheet_id": "g1", "series_index": 0, "connecting_line": { "bogus": 1 } }),
        json!({ "sheet_id": "g1", "series_index": 0, "connecting_line": { "stroke_width_pt": "wide" } }),
        json!({ "sheet_id": "g1", "series_index": 0, "connecting_line": { "stroke_width_pt": null, "width_pt": 2.0 } }),
        json!({ "sheet_id": "g1", "series_index": 0, "y_error_bars": 5 }),
        json!({ "sheet_id": "g1", "series_index": 0, "y_error_bars": { "bogus": 1 } }),
        json!({ "sheet_id": "g1", "series_index": 0, "y_error_bars": { "stroke_width_pt": "wide" } }),
        json!({ "sheet_id": "g1", "series_index": 0, "offset": { "x_pt": 1.0, "bogus": 1 } }),
    ] {
        let err = err_cmd(&mut session, "graph.setSeries", bad)?;
        assert_eq!(err.code, ErrorCode::BadInput);
    }

    let mv_err = err_cmd(&mut session, "graph.setSpec", json!({ "sheet_id": "g1", "multivariate": { "x_column": 0, "y_column": 0, "bogus": 1 } }))?;
    assert_eq!(mv_err.code, ErrorCode::BadInput);

    let hm_err = err_cmd(&mut session, "graph.setSpec", json!({ "sheet_id": "g1", "heatmap_labels": { "annotate_cells": true, "bogus": 1 } }))?;
    assert_eq!(hm_err.code, ErrorCode::BadInput);
    Ok(())
}

#[test]
fn fa_sec12_06_set_spec_rejects_shorthand_conflict_and_unknown_nested_keys() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-06")?;
    doc.add_data_sheet(seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 4)?)?;
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "desc1", "source_sheet": "xy1", "kind": "descriptive_stats" }))?;
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy1", "kind": "xy" }))?;
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "l1" }))?;

    let l_conflict = err_cmd(&mut session, "layout.setSpec", json!({ "sheet_id": "l1", "spec": { "width": 180.0 }, "height": 120.0 }))?;
    assert_eq!(l_conflict.code, ErrorCode::BadInput);

    let l_unknown = err_cmd(&mut session, "layout.setSpec", json!({ "sheet_id": "l1", "spec": { "bogus": 1 } }))?;
    assert_eq!(l_unknown.code, ErrorCode::BadInput);

    let l_bad_type = err_cmd(&mut session, "layout.setSpec", json!({ "sheet_id": "l1", "spec": { "width": "bad" } }))?;
    assert_eq!(l_bad_type.code, ErrorCode::BadInput);

    let g_conflict = err_cmd(&mut session, "graph.setSpec", json!({ "sheet_id": "g1", "spec": { "title": "X" }, "orientation": "horizontal" }))?;
    assert_eq!(g_conflict.code, ErrorCode::BadInput);

    let g_unknown = err_cmd(&mut session, "graph.setSpec", json!({ "sheet_id": "g1", "spec": { "bogus": 1 } }))?;
    assert_eq!(g_unknown.code, ErrorCode::BadInput);

    let g_nested_unknown = err_cmd(&mut session, "graph.setSpec", json!({ "sheet_id": "g1", "spec": { "x_axis": { "bogus": 1 } } }))?;
    assert_eq!(g_nested_unknown.code, ErrorCode::BadInput);

    // CompactLetterDisplay enabled flag compat remains functional without accepting arbitrary bogus keys.
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g1",
            "spec": { "title": "CLD Plot", "compact_letter_display": { "enabled": true, "result_sheet_id": "desc1" } }
        }),
    )?;
    let g_cld_bad = err_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g1",
            "spec": { "compact_letter_display": { "enabled": true, "result_sheet_id": "desc1", "bogus": 1 } }
        }),
    )?;
    assert_eq!(g_cld_bad.code, ErrorCode::BadInput);
    Ok(())
}

#[test]
fn fa_sec12_07_all_28_table_schemas_closed_and_flatten_commands_reject_unknown_keys() -> TestResult {
    let specs = table::specs();
    assert_eq!(specs.len(), 28);
    let mut ids = BTreeSet::new();
    for spec in specs {
        ids.insert(spec.id);
        let schema = (spec.input_schema)();
        assert_eq!(schema.get("additionalProperties"), Some(&Value::Bool(false)), "schema `{}` must set additionalProperties: false", spec.id);
    }
    assert_eq!(ids.len(), 28);

    let mut doc = ProjectDocument::new("SEC12-07")?;
    doc.add_data_sheet(seed_xy_sheet("xy1", "XY", 2, SubcolumnMode::Single, 3)?)?;
    let mut mv = MultipleVariablesTable::new(2, 2)?;
    mv.variables[0].title = String::from("Group");
    mv.variables[0].cells = vec![Cell::text("A"), Cell::text("B")];
    mv.variables[1].title = String::from("Score");
    mv.variables[1].cells = vec![Cell::number(1.0), Cell::number(2.0)];
    doc.add_data_sheet(DataSheet::new("mv1", "MV", TableArchetype::MultipleVariables(mv))?)?;
    doc.add_data_sheet(DataSheet::new("col1", "Col", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?))?)?;
    doc.add_data_sheet(DataSheet::new("nest1", "Nested", TableArchetype::Nested(NestedTable::new(1, 2, 2)?))?)?;
    let mut session = Session::from_document(doc);

    let probes: [(&str, Value); 28] = [
        ("table.setCell", json!({ "sheet_id": "col1", "row": 0, "col": 0, "value": 1.0, "bogus": 1 })),
        ("table.setCells", json!({ "sheet_id": "col1", "edits": [{ "row": 0, "col": 0, "value": 1.0 }], "bogus": 1 })),
        ("table.setXCells", json!({ "sheet_id": "xy1", "row": 0, "value": 1.0, "bogus": 1 })),
        ("table.setRowTitles", json!({ "sheet_id": "col1", "row": 0, "title": "R0", "bogus": 1 })),
        ("table.setGroupTitles", json!({ "sheet_id": "col1", "col": 0, "title": "G0", "bogus": 1 })),
        ("table.setTitles", json!({ "sheet_id": "col1", "col": 0, "title": "G0", "bogus": 1 })),
        ("table.setSubcolumnTitles", json!({ "sheet_id": "nest1", "subcol": 0, "title": "S0", "bogus": 1 })),
        ("table.excludeCells", json!({ "sheet_id": "col1", "row": 0, "col": 0, "bogus": 1 })),
        ("table.excludeCellsWithReason", json!({ "sheet_id": "col1", "row": 0, "col": 0, "reason_tag": "Outlier", "bogus": 1 })),
        ("table.exclusionAuditReport", json!({ "sheet_id": "col1", "bogus": 1 })),
        ("table.clearCells", json!({ "sheet_id": "col1", "all": true, "bogus": 1 })),
        ("table.paste", json!({ "sheet_id": "col1", "text": "1\n2\n", "bogus": 1 })),
        ("table.pasteLink", json!({ "target_sheet_id": "col1", "source_sheet_id": "xy1", "bogus": 1 })),
        ("table.importCsv", json!({ "csv": "1,2\n3,4\n", "bogus": 1 })),
        ("table.exportCsv", json!({ "sheet_id": "col1", "bogus": 1 })),
        ("table.importPrism", json!({ "bytes_base64": "QQ==", "bogus": 1 })),
        ("table.importPzfx", json!({ "xml": "<xml/>", "bogus": 1 })),
        ("table.setVariableRole", json!({ "sheet_id": "mv1", "col": 1, "role": "continuous", "bogus": 1 })),
        ("table.recodeLevels", json!({ "sheet_id": "mv1", "col": 0, "mappings": { "A": "Alpha" }, "bogus": 1 })),
        ("table.setCategoricalOrder", json!({ "sheet_id": "mv1", "col": 0, "order": "alphabetical", "bogus": 1 })),
        ("table.setCalculatedVariable", json!({ "sheet_id": "mv1", "title": "Calc", "formula": "2", "bogus": 1 })),
        ("table.simulateXy", json!({ "row_count": 5, "bogus": 1 })),
        ("table.convertArchetype", json!({ "sheet_id": "xy1", "target_kind": "Column", "bogus": 1 })),
        ("table.importPlateMap", json!({ "geometry": 96, "values": vec![1.0; 96], "bogus": 1 })),
        ("table.importTidyLong", json!({ "headers": ["G", "V"], "rows": [["A", 1.0]], "group_col": "G", "value_col": "V", "bogus": 1 })),
        ("table.randomizeGroups", json!({ "groups": ["A", "B"], "subject_count": 4, "bogus": 1 })),
        ("table.randomSampleRows", json!({ "source_sheet_id": "col1", "sample_size": 2, "bogus": 1 })),
        (
            "table.simulateDistributions",
            json!({ "n_rows": 3, "columns": [{ "name": "C", "distribution": "gaussian", "mean": 0.0, "sd": 1.0 }], "bogus": 1 }),
        ),
    ];
    let mut probed = BTreeSet::new();
    for (cmd_id, payload) in probes {
        probed.insert(cmd_id);
        let err = err_cmd(&mut session, cmd_id, payload)?;
        assert_eq!(err.code, ErrorCode::BadInput, "`{cmd_id}` must reject unknown field");
        assert!(err.to_string().contains("unknown field"), "`{cmd_id}` err: {err}");
    }
    assert_eq!(probed, ids);

    // Nested strictness for flattened item/filter/baseline/dist/cell objects:
    for (cmd_id, payload) in [
        ("table.setCells", json!({ "sheet_id": "col1", "edits": [{ "row": 0, "col": 0, "value": 1.0, "bogus": 1 }] })),
        ("table.setCells", json!({ "sheet_id": "col1", "rows": [[{ "value": "1.5", "bogus": 1 }]] })),
        ("table.setXCells", json!({ "sheet_id": "xy1", "edits": [{ "row": 0, "value": 1.0, "bogus": 1 }] })),
        ("table.setCell", json!({ "sheet_id": "col1", "coord": { "coord": { "row": 0, "col": 0, "bogus": 1 } }, "value": 1.0 })),
        ("table.paste", json!({ "sheet_id": "col1", "text": "1\n", "filter": { "row_start": 0, "bogus": 1 } })),
        ("table.pasteLink", json!({ "target_sheet_id": "col1", "source_sheet_id": "xy1", "baseline": { "col": 0, "op": "difference", "bogus": 1 } })),
        (
            "table.importTidyLong",
            json!({ "headers": ["G", "V"], "rows": [["A", 1.0]], "mapping": { "group_col": "G", "value_col": "V", "bogus": 1 } }),
        ),
        (
            "table.simulateDistributions",
            json!({ "n_rows": 3, "columns": [{ "name": "C", "distribution": "gaussian", "mean": 0.0, "sd": 1.0, "bogus": 1 }] }),
        ),
    ] {
        let err = err_cmd(&mut session, cmd_id, payload)?;
        assert_eq!(err.code, ErrorCode::BadInput, "`{cmd_id}` nested unknown key failed: {err}");
    }
    Ok(())
}

#[test]
fn fa_sec12_08_paste_link_rejects_malformed_link_spec_instead_of_fallback() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-08")?;
    doc.add_data_sheet(seed_xy_sheet("src", "Source", 1, SubcolumnMode::Single, 3)?)?;
    doc.add_data_sheet(DataSheet::new("dst", "Dest", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3)?))?)?;
    let mut session = Session::from_document(doc);

    let err = err_cmd(
        &mut session,
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst",
            "source_sheet_id": "src",
            "link_spec": { "bogus": 1 }
        }),
    )?;
    assert_eq!(err.code, ErrorCode::BadInput);
    assert!(session.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?.linked_regions.is_empty());
    Ok(())
}

#[test]
fn fa_sec12_09_explicit_decimal_separator_rejects_auto_and_invalid_values() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-09")?;
    doc.add_data_sheet(seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 3)?)?;
    doc.add_data_sheet(DataSheet::new("col1", "Col", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3)?))?)?;
    let mut session = Session::from_document(doc);

    for bad_sep in ["auto", "semicolon"] {
        for (cmd_id, payload) in [
            ("table.setCell", json!({ "sheet_id": "col1", "row": 0, "col": 0, "value": "1,5", "decimal_separator": bad_sep })),
            ("table.setCells", json!({ "sheet_id": "col1", "edits": [{ "row": 0, "col": 0, "value": "1,5", "decimal_separator": bad_sep }] })),
            ("table.setCells", json!({ "sheet_id": "col1", "rows": [[{ "value": "1,5", "decimal_separator": bad_sep }]] })),
            ("table.setXCells", json!({ "sheet_id": "xy1", "row": 0, "value": "1,5", "decimal_separator": bad_sep })),
            ("table.setXCells", json!({ "sheet_id": "xy1", "edits": [{ "row": 0, "value": "1,5", "decimal_separator": bad_sep }] })),
            ("table.paste", json!({ "sheet_id": "col1", "rows": [["1,5"]], "decimal_separator": bad_sep })),
            ("table.exportCsv", json!({ "sheet_id": "col1", "decimal_separator": bad_sep })),
        ] {
            let err = err_cmd(&mut session, cmd_id, payload)?;
            assert_eq!(err.code, ErrorCode::BadInput, "`{cmd_id}` must reject `{bad_sep}`");
        }
    }

    // Text paste with `"auto"` remains supported.
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "col1", "text": "1,5\n2,5\n", "decimal_separator": "auto" }))?;
    Ok(())
}

#[test]
fn fa_sec12_10_simulate_transform_rejects_non_object_parameters_without_breaking_tuple_updates() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-10")?;
    doc.add_data_sheet(seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 5)?)?;
    let mut session = Session::from_document(doc);

    let base_kind = json!({
        "Transform": {
            "Simulate": {
                "seed": 1,
                "row_count": 5,
                "replicates": 1,
                "x_min": 0.0,
                "x_max": 4.0,
                "equation_id": "straight_line",
                "parameters": [],
                "noise": "None"
            }
        }
    });

    let err_arr = err_cmd(
        &mut session,
        "transform.run",
        json!({
            "source_sheet_id": "xy1",
            "kind": base_kind,
            "parameters": [1.0, 2.0]
        }),
    )?;
    assert_eq!(err_arr.code, ErrorCode::BadInput);

    let err_str = err_cmd(
        &mut session,
        "transform.run",
        json!({
            "source_sheet_id": "xy1",
            "kind": base_kind,
            "params": "bad"
        }),
    )?;
    assert_eq!(err_str.code, ErrorCode::BadInput);

    // Valid object parameter override on transform.run works, and tuple array update in analysis.updateParams keeps working.
    let res = ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "sim-1",
            "source_sheet_id": "xy1",
            "kind": base_kind,
            "parameters": { "Slope": 2.0, "Intercept": 1.0 }
        }),
    )?;
    assert_eq!(res["sheet_id"], json!("sim-1"));

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "sim-1", "params": { "parameters": [["Slope", 3.0], ["Intercept", 0.5]] } }))?;
    Ok(())
}

#[test]
fn fa_sec12_11_set_titles_rejects_noop_and_ambiguous_title_overrides() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-11")?;
    doc.add_data_sheet(DataSheet::new("col1", "Col", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?))?)?;
    let mut session = Session::from_document(doc);

    for cmd_id in ["table.setTitles", "table.setGroupTitles"] {
        for bad in [
            json!({ "sheet_id": "col1" }),
            json!({ "sheet_id": "col1", "titles": [] }),
            json!({ "sheet_id": "col1", "row_titles": [] }),
            json!({ "sheet_id": "col1", "titles": ["A", "B"], "title": "Single", "col": 0 }),
        ] {
            let err = err_cmd(&mut session, cmd_id, bad)?;
            assert_eq!(err.code, ErrorCode::BadInput, "`{cmd_id}` should reject {err}");
        }
    }

    ok_cmd(&mut session, "table.setTitles", json!({ "sheet_id": "col1", "titles": ["Control", "Treated"] }))?;
    ok_cmd(&mut session, "table.setGroupTitles", json!({ "sheet_id": "col1", "col": 0, "title": "Vehicle" }))?;
    Ok(())
}

#[test]
fn fa_sec04_04_failed_result_sheet_csv_export_returns_error() -> TestResult {
    let mut doc = ProjectDocument::new("SEC04-04")?;
    doc.add_data_sheet(seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 3)?)?;

    let tab = ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric")],
        row_labels: vec![String::from("Mean")],
        rows: vec![vec![ResultCell::Empty]],
        footnotes: Vec::new(),
    };
    let mut failed_sheet = ResultSheet::new(AnalysisSpec::new("res-failed", "xy1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    failed_sheet.tabs = vec![tab];
    failed_sheet.status = ResultStatus::Failed { code: ErrorCode::InsufficientData, message: String::from("not enough data") };
    doc.add_result_sheet(failed_sheet)?;

    let mut session = Session::from_document(doc);

    let err_table = err_cmd(&mut session, "table.exportCsv", json!({ "sheet_id": "res-failed" }))?;
    assert_eq!(err_table.code, ErrorCode::BadInput);

    let err_tidy = err_cmd(&mut session, "analysis.exportCsv", json!({ "sheet_id": "res-failed" }))?;
    assert_eq!(err_tidy.code, ErrorCode::BadInput);

    let err_tab = err_cmd(&mut session, "analysis.exportCsv", json!({ "sheet_id": "res-failed", "tab_id": "summary" }))?;
    assert_eq!(err_tab.code, ErrorCode::BadInput);
    Ok(())
}
