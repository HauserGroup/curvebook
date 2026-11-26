#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::error::Error;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ColorChannelMapping, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec,
    ErrorCode, MAX_PROJECT_CELLS, MultipleVariablesTable, MultivariatePlotMode, NestedTable, ProjectDocument, ResultCell, ResultSheet, ResultStatus,
    ResultTab, ResultTabKind, SizeChannelMapping, SubcolumnMode, TableArchetype, VariableRole, XColumnMode, XyTable,
};
use curvebook_engine::{Session, cmd::table};
use curvebook_format::{MAX_CSV_FILE_BYTES, MAX_XML_BYTES};
use serde_json::{Value, json};

use super::common::{
    assert_bad_input, assert_bad_inputs, assert_err_cases, assert_err_code, col_sheet, ds, gs, mv_sheet, ok_payload as ok_cmd, run_cmds,
    session_with, try_err_cmd as err_cmd,
};

type TestResult = Result<(), Box<dyn Error>>;

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
    let mv1 = mv_sheet(
        "mv1",
        "MV Sheet",
        &[],
        &[
            ("Group", Some(VariableRole::Categorical), &[Cell::text("A"), Cell::text("B"), Cell::text("A")]),
            ("Dose", Some(VariableRole::Continuous), &[Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)]),
            ("Resp", Some(VariableRole::Continuous), &[Cell::number(10.0), Cell::number(20.0), Cell::number(30.0)]),
        ],
    );
    let mut session = session_with(
        "SEC12-01",
        vec![
            seed_xy_sheet("xy1", "Single Y", 1, SubcolumnMode::Single, 4)?,
            seed_xy_sheet("xy2", "Dual Y", 2, SubcolumnMode::Single, 4)?,
            seed_xy_sheet("xy_rep", "Rep Y", 1, SubcolumnMode::RawReplicates(2), 3)?,
            col_sheet("col2", "Two Cols", &[("A", &[1.0, 2.0, 3.0]), ("B", &[5.0, 10.0, 15.0])]),
            mv1,
        ],
    );
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy1", "kind": "xy" })),
            ("graph.formatSelectedPoints", json!({ "sheet_id": "g1", "points": [{ "series_index": 0, "point_index": 2 }], "hidden": true })),
            ("graph.create", json!({ "sheet_id": "g2", "data_sheet_id": "xy2", "kind": "xy" })),
            ("graph.formatSelectedPoints", json!({ "sheet_id": "g2", "points": [{ "series_index": 0, "point_index": 1 }], "color": "#ff0000" })),
            ("graph.formatSelectedPoints", json!({ "sheet_id": "g2", "points": [{ "series_index": 1, "point_index": 2 }], "hidden": true })),
            ("graph.create", json!({ "sheet_id": "grep", "data_sheet_id": "xy_rep", "kind": "xy" })),
            (
                "graph.formatSelectedPoints",
                json!({ "sheet_id": "grep", "points": [{ "series_index": 0, "point_index": 1, "subcol": 1 }], "hidden": true }),
            ),
            ("graph.create", json!({ "sheet_id": "gcol", "data_sheet_id": "col2", "kind": "column" })),
            ("graph.formatSelectedPoints", json!({ "sheet_id": "gcol", "points": [{ "series_index": 0, "point_index": 0 }], "hidden": true })),
            ("graph.formatSelectedPoints", json!({ "sheet_id": "gcol", "points": [{ "series_index": 1, "point_index": 2 }], "color": "#00ff00" })),
            ("graph.create", json!({ "sheet_id": "gmv", "data_sheet_id": "mv1", "kind": "multiple_variables" })),
            (
                "graph.setSpec",
                json!({
                    "sheet_id": "gmv",
                    "spec": { "multivariate": {
                        "plot_mode": MultivariatePlotMode::BubbleScatter, "x_column": 1, "y_column": 2,
                        "color": ColorChannelMapping::Categorical { column: 0, palette: curvebook_palette::PaletteId::OkabeIto, level_order_override: None },
                        "size": SizeChannelMapping::Variable { column: 2, min_radius_pt: 3.0, max_radius_pt: 12.0 },
                        "label_column": 0, "x_level_order_override": null, "show_convex_hulls": false, "show_confidence_ellipses": false
                    }}
                }),
            ),
            ("table.convertArchetype", json!({ "sheet_id": "xy1", "target_kind": "Column", "allow_lossy": true })),
            ("table.convertArchetype", json!({ "sheet_id": "xy2", "target_kind": "Column", "allow_lossy": true })),
            ("table.convertArchetype", json!({ "sheet_id": "xy_rep", "target_kind": "Column", "allow_lossy": true })),
            ("table.convertArchetype", json!({ "sheet_id": "col2", "target_kind": "XY", "allow_lossy": true })),
            ("table.convertArchetype", json!({ "sheet_id": "mv1", "target_kind": "Grouped", "allow_lossy": true })),
        ],
    );

    let g1 = gs(&session, "g1");
    assert_eq!(
        (
            g1.spec.series.iter().map(|s| s.data_set_index).collect::<Vec<_>>(),
            g1.spec.point_overrides.len(),
            g1.spec.point_overrides[0].series_index,
            g1.spec.point_overrides[0].point_index,
            g1.spec.point_overrides[0].hidden
        ),
        (vec![1], 1, 0, 2, true)
    );
    let g2 = gs(&session, "g2");
    assert_eq!((g2.spec.series.iter().map(|s| s.data_set_index).collect::<Vec<_>>(), g2.spec.point_overrides.len()), (vec![1, 2], 2));
    assert_eq!(
        (
            g2.spec.point_overrides[0].series_index,
            g2.spec.point_overrides[0].point_index,
            g2.spec.point_overrides[1].series_index,
            g2.spec.point_overrides[1].point_index,
            g2.spec.point_overrides[1].hidden
        ),
        (0, 1, 1, 2, true)
    );
    let grep = gs(&session, "grep");
    assert_eq!(
        (
            grep.spec.series.iter().map(|s| s.data_set_index).collect::<Vec<_>>(),
            grep.spec.point_overrides.len(),
            grep.spec.point_overrides[0].series_index,
            grep.spec.point_overrides[0].point_index,
            grep.spec.point_overrides[0].subcol
        ),
        (vec![1, 2], 1, 1, 1, None)
    );
    let gcol = gs(&session, "gcol");
    assert_eq!(
        (
            gcol.spec.series.iter().map(|s| s.data_set_index).collect::<Vec<_>>(),
            gcol.spec.point_overrides.len(),
            gcol.spec.point_overrides[0].series_index,
            gcol.spec.point_overrides[0].point_index,
            gcol.spec.point_overrides[0].hidden
        ),
        (vec![0], 1, 0, 2, false)
    );
    assert!(gs(&session, "gmv").spec.multivariate.is_none());
    Ok(())
}

#[test]
fn fa_sec12_02_project_cell_budget_enforced_across_import_replace_and_generators() -> TestResult {
    let mut doc = ProjectDocument::new("SEC12-02")?;
    for (id, cols, rows) in [("big-1", 25, 1_000_000), ("big-2", 24, 1_000_000), ("big-3", 100, 9_999)] {
        doc.add_data_sheet(DataSheet::new(id, id, TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows)?))?)?;
    }
    doc.add_data_sheet(DataSheet::new("mv-small", "MV Small", TableArchetype::MultipleVariables(MultipleVariablesTable::new(1, 20)?))?)?;
    let small_vals: Vec<f64> = (0..70).map(|r| r as f64).collect();
    doc.add_data_sheet(col_sheet("small", "Small", &[("S", &small_vals)]))?;
    assert_eq!(doc.total_allocated_cells()?, MAX_PROJECT_CELLS - 10);

    let mut session = Session::from_document(doc);
    let csv_20: String = (0..20).map(|i| format!("{i},{}\n", i * 2)).collect();
    let csv_45: String = (0..45).map(|i| format!("{i},{}\n", i * 2)).collect();
    let roles96: Vec<Value> = (0..96)
        .map(|i| {
            if i < 4 { json!("Blank") } else { json!({ "role": "treatment", "group_name": "G1", "dilution_x": (i % 12 + 1) as f64, "replicate": 1 }) }
        })
        .collect();
    let plate_vals: Vec<f64> = (0..96).map(|i| i as f64).collect();
    let tidy_rows: Vec<Value> = (0..20).map(|i| json!(["G1", i as f64, 100.0 + i as f64])).collect();
    let info_before = session.document().info_sheets.len();

    assert_err_cases(
        &mut session,
        &[
            ("table.importCsv", json!({ "sheet_id": "new-csv", "csv": csv_20, "archetype": "column" }), ErrorCode::ResourceLimit),
            (
                "table.importCsv",
                json!({ "sheet_id": "small", "csv": csv_45, "archetype": "column", "has_header": false, "replace": true }),
                ErrorCode::ResourceLimit,
            ),
            ("table.importPlateMap", json!({ "geometry": 96, "values": plate_vals, "roles": roles96 }), ErrorCode::ResourceLimit),
            (
                "table.importTidyLong",
                json!({ "headers": ["Group", "X", "Val"], "rows": tidy_rows, "group_col": "Group", "value_col": "Val" }),
                ErrorCode::ResourceLimit,
            ),
            ("table.simulateXy", json!({ "row_count": 10, "replicates": 2 }), ErrorCode::ResourceLimit),
            ("table.simulateXy", json!({ "sheet_id": "small", "row_count": 30, "replicates": 3 }), ErrorCode::ResourceLimit),
            ("table.randomizeGroups", json!({ "groups": ["Ctrl", "Drug"], "subject_count": 12 }), ErrorCode::ResourceLimit),
            ("table.randomSampleRows", json!({ "source_sheet_id": "small", "sample_size": 20 }), ErrorCode::ResourceLimit),
            (
                "table.simulateDistributions",
                json!({ "n_rows": 10, "columns": [{ "name": "A", "distribution": "gaussian", "mean": 0.0, "sd": 1.0 }, { "name": "B", "distribution": "gaussian", "mean": 1.0, "sd": 1.0 }] }),
                ErrorCode::ResourceLimit,
            ),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-small", "title": "CalcOver", "formula": "2" }), ErrorCode::ResourceLimit),
            ("table.paste", json!({ "sheet_id": "small", "mode": "append_rows", "text": csv_20 }), ErrorCode::ResourceLimit),
            (
                "table.pasteLink",
                json!({ "target_sheet_id": "small", "source_sheet_id": "mv-small", "mode": "append_rows" }),
                ErrorCode::ResourceLimit,
            ),
        ],
    );
    assert_eq!((session.document().info_sheets.len(), session.document().total_allocated_cells()?), (info_before, MAX_PROJECT_CELLS - 10));
    Ok(())
}

#[test]
fn fa_sec12_03_inline_payload_byte_ceilings_enforced() -> TestResult {
    let mut session = Session::new();
    let big_csv = "a".repeat(MAX_CSV_FILE_BYTES + 1);
    let big_xml = "a".repeat(MAX_XML_BYTES + 1);
    assert_err_cases(
        &mut session,
        &[
            ("table.importCsv", json!({ "csv": big_csv }), ErrorCode::ResourceLimit),
            ("table.importTidyLong", json!({ "csv": big_csv, "group_col": 0, "value_col": 1 }), ErrorCode::ResourceLimit),
            ("table.importPzfx", json!({ "xml": big_xml }), ErrorCode::ResourceLimit),
        ],
    );
    Ok(())
}

#[test]
fn fa_sec12_04_set_calculated_variable_rejects_linked_region_overlap() -> TestResult {
    let src = mv_sheet(
        "src",
        "Source",
        &[],
        &[("X", None, &vec![Cell::empty(); 3]), ("LinkedSrc", None, &[Cell::number(10.0), Cell::number(20.0), Cell::number(30.0)])],
    );
    let dst = mv_sheet(
        "dst",
        "Dest",
        &[],
        &[("VarA", None, &[Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)]), ("LinkedCol", None, &vec![Cell::empty(); 3])],
    );
    let mut session = session_with("SEC12-04", vec![src, dst]);
    ok_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "src", "target_sheet_id": "dst", "source_range": { "start": { "row": 0, "col": 1 }, "end": { "row": 2, "col": 1 } }, "start_row": 0, "start_col": 1 }),
    )?;
    assert_err_code(
        &mut session,
        "table.setCalculatedVariable",
        json!({ "sheet_id": "dst", "col": 1, "formula": "[VarA] * 3" }),
        ErrorCode::Conflict,
    );
    assert_err_code(
        &mut session,
        "table.setCalculatedVariable",
        json!({ "sheet_id": "dst", "title": "LinkedCol", "formula": "[VarA] * 3" }),
        ErrorCode::Conflict,
    );
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "dst", "title": "UnlinkedCalc", "formula": "[VarA] * 2" }))?;
    assert_eq!(ds(&session, "dst").archetype.data_cell(CellCoord::new(0, 1, 0)).map(|c| &c.value), Some(&CellValue::Number(10.0)));
    Ok(())
}

#[test]
fn fa_sec12_05_graph_patchers_reject_omitted_axis_unknown_keys_and_mistyped_values() -> TestResult {
    let mut session = session_with("SEC12-05", vec![seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 4)?]);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy1", "kind": "xy" }))?;
    assert_bad_inputs(
        &mut session,
        &[
            ("graph.setAxes", json!({ "sheet_id": "g1", "title": "Orphan Title" })),
            ("graph.setAxes", json!({ "sheet_id": "g1", "x_axis": { "bogus": 1 } })),
            ("graph.setAxes", json!({ "sheet_id": "g1", "x_axis": { "stroke_width_pt": "thick" } })),
            ("graph.setAxes", json!({ "sheet_id": "g1", "x_axis": { "minor_ticks_per_interval": "two" } })),
            ("graph.setAxes", json!({ "sheet_id": "g1", "x_axis": { "segments": "bad" } })),
            ("graph.setAxes", json!({ "sheet_id": "g1", "x_axis": { "segments": [{ "scale": "linear", "bogus": 1 }] } })),
            ("graph.setAxes", json!({ "sheet_id": "g1", "x_axis": { "thresholds": [{ "value": 1.0, "bogus": 1 }] } })),
            ("graph.setAxes", json!({ "sheet_id": "g1", "x_axis": { "additional_ticks": [{ "value": 1.0, "bogus": 1 }] } })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "bogus": 1 })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series": [{ "series_index": 0, "bogus": 1 }] })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "bind_to_right_y2": "yes" })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "marker": 5 })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "marker": { "bogus": 1 } })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "marker": { "size_pt": "big" } })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "connecting_line": 5 })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "connecting_line": { "bogus": 1 } })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "connecting_line": { "stroke_width_pt": "wide" } })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "connecting_line": { "stroke_width_pt": null, "width_pt": 2.0 } })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "y_error_bars": 5 })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "y_error_bars": { "bogus": 1 } })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "y_error_bars": { "stroke_width_pt": "wide" } })),
            ("graph.setSeries", json!({ "sheet_id": "g1", "series_index": 0, "offset": { "x_pt": 1.0, "bogus": 1 } })),
            ("graph.setSpec", json!({ "sheet_id": "g1", "multivariate": { "x_column": 0, "y_column": 0, "bogus": 1 } })),
            ("graph.setSpec", json!({ "sheet_id": "g1", "heatmap_labels": { "annotate_cells": true, "bogus": 1 } })),
        ],
    );
    Ok(())
}

#[test]
fn fa_sec12_06_set_spec_rejects_shorthand_conflict_and_unknown_nested_keys() -> TestResult {
    let mut session = session_with("SEC12-06", vec![seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 4)?]);
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "desc1", "source_sheet_id": "xy1", "kind": "descriptive_stats" })),
            ("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy1", "kind": "xy" })),
            ("layout.create", json!({ "sheet_id": "l1" })),
        ],
    );
    assert_bad_inputs(
        &mut session,
        &[
            ("layout.setSpec", json!({ "sheet_id": "l1", "spec": { "width": 180.0 }, "height": 120.0 })),
            ("layout.setSpec", json!({ "sheet_id": "l1", "spec": { "bogus": 1 } })),
            ("layout.setSpec", json!({ "sheet_id": "l1", "spec": { "width": "bad" } })),
            ("graph.setSpec", json!({ "sheet_id": "g1", "spec": { "title": "X" }, "orientation": "horizontal" })),
            ("graph.setSpec", json!({ "sheet_id": "g1", "spec": { "bogus": 1 } })),
            ("graph.setSpec", json!({ "sheet_id": "g1", "spec": { "x_axis": { "bogus": 1 } } })),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g1", "spec": { "compact_letter_display": { "enabled": true, "result_sheet_id": "desc1", "bogus": 1 } } }),
            ),
        ],
    );
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({ "sheet_id": "g1", "spec": { "title": "CLD Plot", "compact_letter_display": { "enabled": true, "result_sheet_id": "desc1" } } }),
    )?;
    Ok(())
}

#[test]
fn fa_sec12_07_all_29_table_schemas_closed_and_flatten_commands_reject_unknown_keys() -> TestResult {
    let specs = table::specs();
    assert_eq!(specs.len(), 29);
    let ids: BTreeSet<&str> = specs
        .iter()
        .map(|s| {
            assert_eq!((s.input_schema)().get("additionalProperties"), Some(&Value::Bool(false)), "`{}`", s.id);
            s.id
        })
        .collect();
    assert_eq!(ids.len(), 29);

    let mut session = session_with(
        "SEC12-07",
        vec![
            seed_xy_sheet("xy1", "XY", 2, SubcolumnMode::Single, 3)?,
            mv_sheet(
                "mv1",
                "MV",
                &[],
                &[("Group", None, &[Cell::text("A"), Cell::text("B")]), ("Score", None, &[Cell::number(1.0), Cell::number(2.0)])],
            ),
            col_sheet("col1", "Col", &[("A", &[]), ("B", &[])]),
            DataSheet::new("nest1", "Nested", TableArchetype::Nested(NestedTable::new(1, 2, 2)?))?,
        ],
    );
    let probes = [
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
        ("table.importXlsx", json!({ "bytes_base64": "QQ==", "bogus": 1 })),
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
        assert!(err.code == ErrorCode::BadInput && err.to_string().contains("unknown field"), "`{cmd_id}`: {err}");
    }
    assert_eq!(probed, ids);

    assert_bad_inputs(
        &mut session,
        &[
            ("table.setCells", json!({ "sheet_id": "col1", "edits": [{ "row": 0, "col": 0, "value": 1.0, "bogus": 1 }] })),
            ("table.setCells", json!({ "sheet_id": "col1", "rows": [[{ "value": "1.5", "bogus": 1 }]] })),
            ("table.setXCells", json!({ "sheet_id": "xy1", "edits": [{ "row": 0, "value": 1.0, "bogus": 1 }] })),
            ("table.setCell", json!({ "sheet_id": "col1", "coord": { "coord": { "row": 0, "col": 0, "bogus": 1 } }, "value": 1.0 })),
            ("table.paste", json!({ "sheet_id": "col1", "text": "1\n", "filter": { "row_start": 0, "bogus": 1 } })),
            (
                "table.pasteLink",
                json!({ "target_sheet_id": "col1", "source_sheet_id": "xy1", "baseline": { "col": 0, "op": "difference", "bogus": 1 } }),
            ),
            (
                "table.importTidyLong",
                json!({ "headers": ["G", "V"], "rows": [["A", 1.0]], "mapping": { "group_col": "G", "value_col": "V", "bogus": 1 } }),
            ),
            (
                "table.simulateDistributions",
                json!({ "n_rows": 3, "columns": [{ "name": "C", "distribution": "gaussian", "mean": 0.0, "sd": 1.0, "bogus": 1 }] }),
            ),
        ],
    );
    Ok(())
}

#[test]
fn fa_sec12_08_paste_link_rejects_malformed_link_spec_instead_of_fallback() -> TestResult {
    let mut session =
        session_with("SEC12-08", vec![seed_xy_sheet("src", "Source", 1, SubcolumnMode::Single, 3)?, col_sheet("dst", "Dest", &[("A", &[])])]);
    assert_bad_input(&mut session, "table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src", "link_spec": { "bogus": 1 } }));
    assert!(ds(&session, "dst").linked_regions.is_empty());
    Ok(())
}

#[test]
fn fa_sec12_09_explicit_decimal_separator_rejects_auto_and_invalid_values() -> TestResult {
    let mut session =
        session_with("SEC12-09", vec![seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 3)?, col_sheet("col1", "Col", &[("A", &[])])]);
    for bad_sep in ["auto", "semicolon"] {
        assert_bad_inputs(
            &mut session,
            &[
                ("table.setCell", json!({ "sheet_id": "col1", "row": 0, "col": 0, "value": "1,5", "decimal_separator": bad_sep })),
                ("table.setCells", json!({ "sheet_id": "col1", "edits": [{ "row": 0, "col": 0, "value": "1,5", "decimal_separator": bad_sep }] })),
                ("table.setCells", json!({ "sheet_id": "col1", "rows": [[{ "value": "1,5", "decimal_separator": bad_sep }]] })),
                ("table.setXCells", json!({ "sheet_id": "xy1", "row": 0, "value": "1,5", "decimal_separator": bad_sep })),
                ("table.setXCells", json!({ "sheet_id": "xy1", "edits": [{ "row": 0, "value": "1,5", "decimal_separator": bad_sep }] })),
                ("table.paste", json!({ "sheet_id": "col1", "rows": [["1,5"]], "decimal_separator": bad_sep })),
                ("table.exportCsv", json!({ "sheet_id": "col1", "decimal_separator": bad_sep })),
            ],
        );
    }
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "col1", "text": "1,5\n2,5\n", "decimal_separator": "auto" }))?;
    Ok(())
}

#[test]
fn fa_sec12_10_simulate_transform_rejects_non_object_parameters_without_breaking_tuple_updates() -> TestResult {
    let mut session = session_with("SEC12-10", vec![seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 5)?]);
    let base_kind = json!({ "Transform": { "Simulate": { "seed": 1, "row_count": 5, "replicates": 1, "x_min": 0.0, "x_max": 4.0, "equation_id": "straight_line", "parameters": [], "noise": "None" } } });
    assert_bad_inputs(
        &mut session,
        &[
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": base_kind, "parameters": [1.0, 2.0] })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": base_kind, "parameters": "bad" })),
        ],
    );
    let res = ok_cmd(
        &mut session,
        "transform.run",
        json!({ "sheet_id": "sim-1", "source_sheet_id": "xy1", "kind": base_kind, "parameters": { "Slope": 2.0, "Intercept": 1.0 } }),
    )?;
    assert_eq!(res["sheet_id"], json!("sim-1"));
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "sim-1", "params": { "parameters": [["Slope", 3.0], ["Intercept", 0.5]] } }))?;
    Ok(())
}

#[test]
fn fa_sec12_11_set_titles_rejects_noop_and_ambiguous_title_overrides() -> TestResult {
    let mut session = session_with("SEC12-11", vec![col_sheet("col1", "Col", &[("A", &[]), ("B", &[])])]);
    for cmd_id in ["table.setTitles", "table.setGroupTitles"] {
        assert_bad_inputs(
            &mut session,
            &[
                (cmd_id, json!({ "sheet_id": "col1" })),
                (cmd_id, json!({ "sheet_id": "col1", "titles": [] })),
                (cmd_id, json!({ "sheet_id": "col1", "row_titles": [] })),
                (cmd_id, json!({ "sheet_id": "col1", "titles": ["A", "B"], "title": "Single", "col": 0 })),
            ],
        );
    }
    ok_cmd(&mut session, "table.setTitles", json!({ "sheet_id": "col1", "titles": ["Control", "Treated"] }))?;
    ok_cmd(&mut session, "table.setGroupTitles", json!({ "sheet_id": "col1", "col": 0, "title": "Vehicle" }))?;
    Ok(())
}

#[test]
fn fa_sec04_04_failed_result_sheet_csv_export_returns_error() -> TestResult {
    let mut doc = ProjectDocument::new("SEC04-04")?;
    doc.add_data_sheet(seed_xy_sheet("xy1", "XY", 1, SubcolumnMode::Single, 3)?)?;
    let mut failed_sheet = ResultSheet::new(AnalysisSpec::new("res-failed", "xy1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    failed_sheet.tabs = vec![ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric")],
        row_labels: vec![String::from("Mean")],
        rows: vec![vec![ResultCell::Empty]],
        footnotes: Vec::new(),
    }];
    failed_sheet.status = ResultStatus::Failed { code: ErrorCode::InsufficientData, message: String::from("not enough data") };
    doc.add_result_sheet(failed_sheet)?;
    let mut session = Session::from_document(doc);
    assert_bad_inputs(
        &mut session,
        &[
            ("table.exportCsv", json!({ "sheet_id": "res-failed" })),
            ("analysis.exportCsv", json!({ "sheet_id": "res-failed" })),
            ("analysis.exportCsv", json!({ "sheet_id": "res-failed", "tab_id": "summary" })),
        ],
    );
    Ok(())
}
