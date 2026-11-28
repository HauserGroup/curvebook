//! Integration tests for all document, sheet, table, import/link, variable, and info commands in `cmd_data`.

use std::sync::atomic::AtomicBool;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ConstantValue, ErrorCode, FramePreset, GraphSheet, HookSource, InfoSheet,
    JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, MultipleVariablesTable, NonlinearFitSpec, ParameterRule, ParameterScope,
    ProjectDocument, RectPt, ResultScalarEntry, ResultSheet, ResultStatus, SheetCellRange, SheetId, TableArchetype, TableKind, VariableRole,
};
use curvebook_engine::{Caller, Session, cmd, dag};
use serde_json::{Value, json};

use super::common::{
    TempDir, assert_bad_input, assert_bad_inputs, assert_err_code, cell_num, cell_val, cleanroom_pzfx_xml, col_sheet, ds, err_cmd, gs, info, ls,
    ok_cmd, rs, run_cmds, session_with, x_num, xy_sheet,
};

#[test]
fn sheet_create_all_eight_archetypes_auto_family_and_inspect() {
    let mut session = Session::new();

    // Default create without auto_* creates ONLY the data sheet (UI navigator contract).
    let plain = ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "data-plain", "title": "Assay" }));
    assert!(plain.mutated);
    assert_eq!(session.document().total_sheet_count(), 1);
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("data-plain"));

    // auto_family: true provisions Data + paired Info + uninitialized Graph while keeping active_sheet_id on Data.
    let xy = ok_cmd(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "xy-1", "title": "Dose Response", "archetype": "xy", "replicates": 3, "cols": 2, "rows": 8, "x_title": "Log Conc (M)", "auto_family": true }),
    );
    assert_eq!(xy.payload["kind"], "Xy");
    let xy_info = xy.payload["info_sheet_id"].as_str().expect("auto info id");
    let xy_graph = xy.payload["graph_sheet_id"].as_str().expect("auto graph id");
    assert!(session.document().info_sheet(&SheetId::new(xy_info)).is_some());
    assert!(session.document().graph_sheet(&SheetId::new(xy_graph)).is_some());
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("xy-1"));

    // auto_family: true with auto_graph: false suppresses GraphSheet while keeping paired InfoSheet.
    let col = ok_cmd(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "col-1", "title": "Treatments", "kind": "column", "design_hint": "paired", "cols": 3, "rows": 6, "auto_family": true, "auto_graph": false }),
    );
    assert!(col.payload.get("info_sheet_id").and_then(Value::as_str).is_some());
    assert!(col.payload.get("graph_sheet_id").and_then(Value::as_str).is_none());
    assert_eq!(session.document().graph_sheets.len(), 1);

    // Remaining 6 TableArchetypes: Grouped, Contingency, Survival, PartsOfWhole, MultipleVariables, Nested.
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "grp-1", "archetype": "grouped", "replicates": 3, "cols": 2, "rows": 4, "row_factor_title": "Genotype", "column_factor_title": "Treatment" }),
            ),
            ("sheet.createData", json!({ "sheet_id": "cont-1", "archetype": "contingency", "cols": 2, "rows": 3 })),
            ("sheet.createData", json!({ "sheet_id": "surv-1", "archetype": "survival", "cols": 2, "rows": 10, "time_title": "Days" })),
            ("sheet.createData", json!({ "sheet_id": "pow-1", "archetype": "parts_of_whole", "cols": 1, "rows": 5 })),
            (
                "sheet.createData",
                json!({ "sheet_id": "mv-1", "archetype": "multiple_variables", "cols": 3, "rows": 5, "variables": [{ "title": "SampleID", "role": "Identifier" }, { "title": "Group", "role": "Categorical" }, { "title": "Signal", "role": "Continuous" }] }),
            ),
            ("sheet.createData", json!({ "sheet_id": "nest-1", "kind": "nested", "cols": 2, "replicates": 4, "rows": 5 })),
        ],
    );

    let kinds: Vec<TableKind> = session.document().data_sheets.iter().map(|s| s.archetype.kind()).collect();
    for expected in [
        TableKind::Xy,
        TableKind::Column,
        TableKind::Grouped,
        TableKind::Contingency,
        TableKind::Survival,
        TableKind::PartsOfWhole,
        TableKind::MultipleVariables,
        TableKind::Nested,
    ] {
        assert!(kinds.contains(&expected), "missing archetype {expected:?}");
    }

    let inspect = ok_cmd(&mut session, "project.inspect", json!({ "sheet_id": xy_graph }));
    assert!(!inspect.mutated);
    assert_eq!(inspect.payload["data_sheets"].as_array().map(Vec::len), Some(9));
    assert_eq!(inspect.payload["focus"]["section"], "Graphs");
}

#[test]
fn project_new_save_open_and_template_round_trip() {
    let dir = TempDir::new("cmd-data-proj");
    let proj_path = dir.child("proj.curvebook");
    let tpl_path = dir.child("tpl.cbtemplate");

    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("project.new", json!({ "title": "Ligand Panel" })),
            (
                "sheet.createData",
                json!({ "sheet_id": "data-1", "title": "Plate 1", "archetype": "xy", "replicates": 2, "cols": 1, "rows": 4, "info_sheet_id": "info-1" }),
            ),
            ("table.setXCells", json!({ "sheet_id": "data-1", "values": [1.0, 2.0, 4.0] })),
            (
                "table.setCells",
                json!({ "sheet_id": "data-1", "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 10.5 }, { "row": 1, "col": 0, "subcol": 0, "value": 20.5 }] }),
            ),
            ("info.setNote", json!({ "sheet_id": "info-1", "text": "Protocol v2" })),
            ("info.setConstant", json!({ "sheet_id": "info-1", "name": "TempC", "value": 37.0, "units": "C" })),
        ],
    );
    assert_eq!(session.document().title, "Ligand Panel");

    let save_res = ok_cmd(&mut session, "project.save", json!({ "path": proj_path.to_str() }));
    assert!(!save_res.mutated);
    let tpl_res = ok_cmd(&mut session, "project.saveTemplate", json!({ "path": tpl_path.to_str(), "return_bytes": true }));
    assert!(!tpl_res.mutated);
    let tpl_b64 = tpl_res.payload["bytes_base64"].as_str().expect("template b64");

    // Reopen saved archive via file.new -> project.open.
    let mut reopened = Session::new();
    run_cmds(&mut reopened, &[("file.new", json!({ "title": "Scratch" })), ("project.open", json!({ "path": proj_path.to_str() }))]);
    assert_eq!(reopened.document().title, "Ligand Panel");
    assert_eq!(cell_num(&reopened, "data-1", 0, 0, 0), Some(10.5));
    assert_eq!(info(&reopened, "info-1").notes_markdown, "Protocol v2");

    // Instantiate template (both path and base64) preserves structure + constants while clearing assay measurements.
    let mut from_tpl = Session::new();
    ok_cmd(&mut from_tpl, "project.instantiateTemplate", json!({ "path": tpl_path.to_str(), "title": "Plate 2 Run" }));
    assert_eq!(from_tpl.document().title, "Plate 2 Run");
    assert!(from_tpl.document().data_sheet(&SheetId::new("data-1")).is_some());
    assert!(from_tpl.document().info_sheet(&SheetId::new("info-1")).is_some());
    assert_eq!(cell_num(&from_tpl, "data-1", 0, 0, 0), None);

    let mut from_b64 = Session::new();
    ok_cmd(&mut from_b64, "project.instantiateTemplate", json!({ "bytes_base64": tpl_b64, "title": "Plate 3 Run" }));
    assert_eq!(from_b64.document().title, "Plate 3 Run");
}

#[test]
fn sheet_rename_delete_duplicate_reorder_and_select() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "d1", "title": "First", "info_sheet_id": "i1", "graph_sheet_id": "g1" })),
            ("sheet.createData", json!({ "sheet_id": "d2", "title": "Second" })),
            ("sheet.rename", json!({ "sheet_id": "d1", "title": "Primary Plate" })),
            ("sheet.rename", json!({ "sheet_id": "i1", "title": "Primary Notes" })),
            ("sheet.rename", json!({ "sheet_id": "g1", "title": "Primary Plot" })),
        ],
    );
    assert_eq!(ds(&session, "d1").title, "Primary Plate");
    assert_eq!(info(&session, "i1").title, "Primary Notes");
    assert_eq!(gs(&session, "g1").title, "Primary Plot");

    // Select in family pane preserves family anchor while switching active sheet.
    ok_cmd(&mut session, "sheet.select", json!({ "sheet_id": "d1", "mode": "navigator" }));
    assert_eq!(session.document().family_anchor_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));
    ok_cmd(&mut session, "sheet.select", json!({ "sheet_id": "g1", "mode": "family" }));
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("g1"));
    assert_eq!(session.document().family_anchor_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));

    // Reorder both by index and explicit section order.
    ok_cmd(&mut session, "sheet.reorder", json!({ "sheet_id": "d2", "index": 0 }));
    assert_eq!(session.document().data_sheets.first().map(|s| s.id.as_str()), Some("d2"));
    ok_cmd(&mut session, "sheet.reorder", json!({ "section": "Data", "order": ["d1", "d2"] }));
    assert_eq!(session.document().data_sheets.first().map(|s| s.id.as_str()), Some("d1"));

    // Duplicate sheet preserves data; frozen sheet blocks rename/delete until unfrozen.
    run_cmds(
        &mut session,
        &[
            ("table.setCell", json!({ "sheet_id": "d2", "row": 0, "col": 0, "value": 99.0 })),
            ("sheet.duplicate", json!({ "sheet_id": "d2", "new_sheet_id": "d2-copy", "title": "Second Copy" })),
        ],
    );
    assert_eq!(cell_num(&session, "d2-copy", 0, 0, 0), Some(99.0));

    ok_cmd(&mut session, "sheet.freeze", json!({ "sheet_id": "d2-copy" }));
    let _ = err_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "d2-copy", "title": "Blocked" }));
    let _ = err_cmd(&mut session, "sheet.delete", json!({ "sheet_id": "d2-copy" }));
    run_cmds(&mut session, &[("sheet.unfreeze", json!({ "sheet_id": "d2-copy" })), ("sheet.delete", json!({ "sheet_id": "d2-copy" }))]);
    assert!(session.document().data_sheet(&SheetId::new("d2-copy")).is_none());

    // Delete cascade removes paired info/graph when d1 is deleted.
    ok_cmd(&mut session, "sheet.delete", json!({ "sheet_id": "d1", "cascade": true }));
    assert!(session.document().data_sheet(&SheetId::new("d1")).is_none());
    assert!(session.document().info_sheet(&SheetId::new("i1")).is_none());
    assert!(session.document().graph_sheet(&SheetId::new("g1")).is_none());
}

#[test]
fn sheet_clone_pipeline_and_duplicate_family_for_fresh_assay_run() {
    let mut doc = ProjectDocument::new("Cloning Assay").expect("doc");
    let mut data = xy_sheet("data-base", "Run 1", "X", &[-8.0, -7.0, -6.0, -5.0, -4.0], &[("Inhibitor A", &[95.0, 82.0, 50.0, 18.0, 5.0])]);
    data.info_sheet_id = Some(SheetId::new("info-base"));
    doc.add_data_sheet(data).expect("add data");

    let mut info_sheet = InfoSheet::new_preseeded(SheetId::new("info-base"), "Run 1 Info").expect("info");
    info_sheet.set_constant("PlateBatch", ConstantValue::Text(String::from("B1")), None).expect("const");
    info_sheet.set_constant("MaxResp", ConstantValue::Number(100.0), None).expect("maxresp");
    doc.add_info_sheet(info_sheet).expect("add info");

    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.parameter_rules.push(ParameterRule {
        name: String::from("Top"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-base")),
            constant_name: String::from("MaxResp"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let ana = AnalysisSpec::new(SheetId::new("res-base"), SheetId::new("data-base"), AnalysisKind::NonlinearRegression(nl)).expect("ana");
    let mut res = ResultSheet::new(ana).expect("res");
    res.scalars.push(ResultScalarEntry { key: String::from("LogEC50"), data_set_index: Some(0), value: -6.0, units: None });
    doc.add_result_sheet(res).expect("add res");

    let mut graph =
        GraphSheet::new_uninitialized_for_data(SheetId::new("graph-base"), doc.data_sheet(&SheetId::new("data-base")).expect("ds")).expect("graph");
    graph.initialized = true;
    graph.spec.frame_preset = FramePreset::PlainBoxed;
    let mut res_series = graph.spec.series[0].clone();
    res_series.source_sheet_id = SheetId::new("res-base");
    graph.spec.series.push(res_series);
    doc.add_graph_sheet(graph).expect("add graph");

    let mut layout_spec = LayoutSpec::from_preset(JournalWidthPreset::LetterLandscape);
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-1"),
        graph_sheet_id: SheetId::new("graph-base"),
        bounds_pt: RectPt::new(10.0, 10.0, 200.0, 150.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    layout_spec.items.push(LayoutItem::LiveTable {
        id: String::from("tbl-1"),
        source_sheet_id: SheetId::new("res-base"),
        tab_id: None,
        bounds_pt: RectPt::new(220.0, 10.0, 200.0, 150.0),
        selected_rows: None,
        selected_cols: None,
        style: curvebook_doc::EmbeddedTableStyle::default(),
    });
    doc.add_layout_sheet(LayoutSheet::new(SheetId::new("layout-base"), "Figure 1", layout_spec).expect("layout")).expect("add layout");

    let mut session = Session::from_document(doc);

    // 1) duplicateFamily keeps all Y measurements intact across cloned sheets.
    let dup =
        ok_cmd(&mut session, "sheet.duplicateFamily", json!({ "sheet_id": "data-base", "new_sheet_id": "data-dup", "title": "Run 1 Exact Copy" }));
    assert_eq!(cell_num(&session, "data-dup", 2, 0, 0), Some(50.0));
    assert_eq!(x_num(&session, "data-dup", 2), Some(-6.0));
    assert_eq!(dup.payload["graph_sheet_ids"].as_array().map(Vec::len), Some(1));
    assert_eq!(dup.payload["layout_sheet_ids"].as_array().map(Vec::len), Some(1));

    // 2) clonePipeline clears Y values while keeping X concentrations, GDO styling, Result hooks, and Layout links.
    let clone = ok_cmd(
        &mut session,
        "sheet.clonePipeline",
        json!({ "sheet_id": "data-base", "new_sheet_id": "data-run2", "title": "Run 2 Fresh Assay", "keep_x_values": true, "clear_y_values": true }),
    );
    let cloned_info = clone.payload["info_sheet_id"].as_str().expect("cloned info");
    let cloned_res = clone.payload["result_sheet_ids"][0].as_str().expect("cloned res");
    let cloned_graph = clone.payload["graph_sheet_ids"][0].as_str().expect("cloned graph");
    let cloned_layout = clone.payload["layout_sheet_ids"][0].as_str().expect("cloned layout");

    assert_eq!(cell_num(&session, "data-run2", 2, 0, 0), None);
    assert_eq!(x_num(&session, "data-run2", 2), Some(-6.0));
    let res_obj = rs(&session, cloned_res);
    assert_eq!(res_obj.analysis.source_sheet_id.as_ref().map(SheetId::as_str), Some("data-run2"));
    assert!(res_obj.scalars.is_empty());
    let AnalysisKind::NonlinearRegression(nl_cloned) = &res_obj.analysis.kind else {
        panic!("expected NonlinearRegression");
    };
    assert_eq!(
        nl_cloned.parameter_rules.first().map(|r| &r.scope),
        Some(&ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new(cloned_info)),
            constant_name: String::from("MaxResp"),
        }))
    );

    let graph_obj = gs(&session, cloned_graph);
    assert_eq!(graph_obj.spec.frame_preset, FramePreset::PlainBoxed);
    assert_eq!(graph_obj.spec.primary_source_sheet_id.as_str(), "data-run2");
    assert_eq!(graph_obj.spec.series[0].source_sheet_id.as_str(), "data-run2");
    assert_eq!(graph_obj.spec.series[1].source_sheet_id.as_str(), cloned_res);

    let layout_obj = ls(&session, cloned_layout);
    assert!(matches!(&layout_obj.spec.items[0], LayoutItem::LiveGraph { graph_sheet_id, .. } if graph_sheet_id.as_str() == cloned_graph));
    assert!(matches!(&layout_obj.spec.items[1], LayoutItem::LiveTable { source_sheet_id, .. } if source_sheet_id.as_str() == cloned_res));

    // Populate fresh assay Y data on cloned run and recalculate.
    run_cmds(
        &mut session,
        &[
            (
                "table.setCells",
                json!({ "sheet_id": "data-run2", "edits": [{ "row": 0, "col": 0, "value": 98.0 }, { "row": 1, "col": 0, "value": 85.0 }, { "row": 2, "col": 0, "value": 52.0 }, { "row": 3, "col": 0, "value": 20.0 }, { "row": 4, "col": 0, "value": 4.0 }] }),
            ),
            ("project.recalculate", json!({})),
        ],
    );
    assert!(matches!(rs(&session, cloned_res).status, ResultStatus::UpToDate));
    assert_ne!(cell_num(&session, "data-base", 2, 0, 0), Some(52.0));
}

#[test]
fn table_cells_titles_exclude_clear_undo_and_err_rollback() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "xy-1", "archetype": "xy", "replicates": 2, "cols": 2, "rows": 4 })),
            ("sheet.createData", json!({ "sheet_id": "nest-1", "kind": "nested", "cols": 2, "replicates": 3, "rows": 3 })),
            ("table.setRowTitles", json!({ "sheet_id": "xy-1", "titles": ["Baseline", "Low", "Mid", "High"] })),
            ("table.setGroupTitles", json!({ "sheet_id": "xy-1", "x_title": "Time (min)", "titles": ["Vehicle", "Drug"] })),
        ],
    );
    let _ = err_cmd(&mut session, "table.setSubcolumnTitles", json!({ "sheet_id": "xy-1", "col": 1, "titles": ["Rep A", "Rep B"] }));
    ok_cmd(&mut session, "table.setSubcolumnTitles", json!({ "sheet_id": "nest-1", "col": 0, "subcol": 2, "title": "Mouse 3" }));
    let TableArchetype::Nested(nt) = &ds(&session, "nest-1").archetype else { panic!("expected Nested archetype") };
    assert_eq!(nt.groups[0].subcolumn_titles.get(2).map(String::as_str), Some("Mouse 3"));

    run_cmds(
        &mut session,
        &[
            ("table.setXCells", json!({ "sheet_id": "xy-1", "values": [0.0, 5.0, 15.0, 30.0] })),
            ("table.setCells", json!({ "sheet_id": "xy-1", "rows": [[1.0, 2.0], [3.0, 4.0]], "start_row": 0, "start_col": 0 })),
            (
                "table.excludeCells",
                json!({ "sheet_id": "xy-1", "range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 1, "col": 0, "subcol": 0 } }, "excluded": true }),
            ),
        ],
    );
    assert!(cell_val(&session, "xy-1", 0, 0, 0).is_some_and(|c| c.excluded));
    assert!(cell_val(&session, "xy-1", 1, 0, 0).is_some_and(|c| c.excluded));

    // Undo restores non-excluded state, redo re-applies.
    assert!(session.undo().expect("undo"));
    assert!(!cell_val(&session, "xy-1", 1, 0, 0).is_some_and(|c| c.excluded));
    assert!(session.redo().expect("redo"));
    assert!(cell_val(&session, "xy-1", 1, 0, 0).is_some_and(|c| c.excluded));

    // Atomic rollback when mid-batch setCells hits an out-of-bounds subcolumn.
    let _ = err_cmd(
        &mut session,
        "table.setCells",
        json!({ "sheet_id": "xy-1", "edits": [{ "row": 0, "col": 1, "subcol": 0, "value": 777.0 }, { "row": 0, "col": 99, "subcol": 0, "value": 888.0 }] }),
    );
    assert_ne!(cell_num(&session, "xy-1", 0, 1, 0), Some(777.0));

    // Clear single cell vs clear all including X.
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "subcol": 0 }));
    assert_eq!(cell_num(&session, "xy-1", 0, 0, 0), None);
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "xy-1", "all": true, "clear_x": true }));
    assert_eq!(cell_num(&session, "xy-1", 1, 0, 0), None);
    assert_eq!(x_num(&session, "xy-1", 1), None);
}

#[test]
fn table_paste_decimal_normalization_slice_and_transpose() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "col-1", "archetype": "column", "cols": 3, "rows": 4 })),
            (
                "table.paste",
                json!({
                    "sheet_id": "col-1",
                    "text": "9,9;8,8\n1,5;2,75*\n3,25;4,5\n7,7;6,6",
                    "delimiter": ";",
                    "decimal_separator": ",",
                    "filter": { "row_start": 1, "row_end": 2, "col_start": 0, "col_end": 1 },
                    "transpose": true,
                    "anchor": { "row": 0, "col": 0, "subcol": 0 }
                }),
            ),
        ],
    );

    // Transposed 2x2 slice: row 0 = [1.5, 3.25], row 1 = [2.75*, 4.5].
    assert_eq!(cell_num(&session, "col-1", 0, 0, 0), Some(1.5));
    assert_eq!(cell_num(&session, "col-1", 0, 1, 0), Some(3.25));
    let excl = cell_val(&session, "col-1", 1, 0, 0).expect("transposed excluded");
    assert_eq!(excl.value, CellValue::Number(2.75));
    assert!(excl.excluded);
    assert_eq!(cell_num(&session, "col-1", 1, 1, 0), Some(4.5));
}

#[test]
fn table_paste_link_reactive_transpose_freeze_unfreeze_and_cycle_guard() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "grp-1", "archetype": "grouped", "replicates": 3, "cols": 1, "rows": 2 })),
            ("sheet.createData", json!({ "sheet_id": "col-1", "archetype": "column", "cols": 2, "rows": 4, "auto_info": true })),
            ("table.setRowTitles", json!({ "sheet_id": "grp-1", "row": 0, "title": "Stimulated" })),
            (
                "table.setCells",
                json!({ "sheet_id": "grp-1", "edits": [{ "row": 0, "col": 0, "subcol": 0, "value": 10.0 }, { "row": 0, "col": 0, "subcol": 1, "value": 12.0 }, { "row": 0, "col": 0, "subcol": 2, "value": 14.0 }] }),
            ),
            (
                "table.pasteLink",
                json!({
                    "target_sheet_id": "col-1",
                    "source_sheet_id": "grp-1",
                    "source_range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 0, "col": 0, "subcol": 2 } },
                    "anchor": { "row": 0, "col": 0, "subcol": 0 },
                    "transpose": true
                }),
            ),
        ],
    );

    assert_eq!(cell_num(&session, "col-1", 0, 0, 0), Some(10.0));
    assert_eq!(cell_num(&session, "col-1", 1, 0, 0), Some(12.0));
    assert_eq!(cell_num(&session, "col-1", 2, 0, 0), Some(14.0));
    let TableArchetype::Column(t) = &ds(&session, "col-1").archetype else { panic!("expected Column archetype") };
    assert_eq!(t.data_sets[0].title, "Stimulated");

    // Upstream mutation reactively updates linked target cell; freeze blocks direct edits & defers updates until unfreeze.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "grp-1", "row": 0, "col": 0, "subcol": 1, "value": 25.0 }));
    assert_eq!(cell_num(&session, "col-1", 1, 0, 0), Some(25.0));

    ok_cmd(&mut session, "sheet.freeze", json!({ "sheet_id": "col-1" }));
    let _ = err_cmd(&mut session, "table.setCell", json!({ "sheet_id": "col-1", "row": 0, "col": 1, "value": 5.0 }));
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "grp-1", "row": 0, "col": 0, "subcol": 2, "value": 99.0 }));
    assert_eq!(cell_num(&session, "col-1", 2, 0, 0), Some(14.0));

    ok_cmd(&mut session, "sheet.unfreeze", json!({ "sheet_id": "col-1" }));
    assert_eq!(cell_num(&session, "col-1", 2, 0, 0), Some(99.0));

    // Backwards pasteLink creating cycle col-1 -> grp-1 must fail cleanly without corrupting grp-1.
    let cycle_params = json!({
        "target_sheet_id": "grp-1",
        "source_sheet_id": "col-1",
        "range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 1, "col": 0, "subcol": 0 } },
        "anchor": { "row": 1, "col": 0, "subcol": 0 }
    });
    let _ = err_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "targetSheetId": "grp-1", "source_sheet_id": "col-1", "range": cycle_params["range"], "anchor": cycle_params["anchor"] }),
    );
    assert!(ds(&session, "grp-1").linked_regions.is_empty());

    // Direct dispatch cycle check also cleans up outside Session rollback wrapper.
    let mut raw_session = Session::from_document(session.document().clone());
    let spec = cmd::registry().get("table.pasteLink").expect("table.pasteLink registered");
    let cancelled = AtomicBool::new(false);
    let mut ctx = cmd::Ctx { session: &mut raw_session, caller: Caller::Ui, cancelled: &cancelled };
    assert!((spec.run)(&mut ctx, cycle_params).is_err());
    assert!(ds(&raw_session, "grp-1").linked_regions.is_empty());
}

#[test]
fn table_csv_prism_pzfx_imports_and_export() {
    let dir = TempDir::new("cmd-data-io");
    let csv_path = dir.child("export.csv");
    let prism_path = dir.child("imp.prism");
    let mut session = Session::new();

    ok_cmd(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "csv-1", "title": "Calib Curve", "archetype": "xy", "csv": "X,Std\n0.1,5.2\n1.0,25.4\n10.0,88.1\n" }),
    );
    assert_eq!(cell_num(&session, "csv-1", 1, 0, 0), Some(25.4));

    let export = ok_cmd(&mut session, "table.exportCsv", json!({ "sheet_id": "csv-1", "path": csv_path.to_str() }));
    assert!(!export.mutated);
    assert!(export.payload["csv"].as_str().expect("csv string").contains("25.4"));

    // Save archive bytes and import back via table.importPrism.
    ok_cmd(&mut session, "project.save", json!({ "path": prism_path.to_str() }));
    let mut prism_sess = Session::new();
    let archive_out = ok_cmd(&mut prism_sess, "table.importPrism", json!({ "path": prism_path.to_str() }));
    assert_eq!(archive_out.message, "imported .prism file");
    assert!(prism_sess.document().data_sheet(&SheetId::new("csv-1")).is_some());

    // Import clean-room XML .pzfx string via table.importPzfx.
    let mut pzfx_sess = Session::new();
    let pzfx_out = ok_cmd(&mut pzfx_sess, "table.importPzfx", json!({ "xml": cleanroom_pzfx_xml() }));
    assert_eq!(pzfx_out.message, "imported .pzfx file");
    assert!(!pzfx_sess.document().data_sheets.is_empty());

    let archive_spec = curvebook_engine::cmd::registry().get("table.importPrism").expect("table.importPrism spec");
    let pzfx_spec = curvebook_engine::cmd::registry().get("table.importPzfx").expect("table.importPzfx spec");
    assert_eq!((archive_spec.label, archive_spec.menu_path), ("Import .prism files", "File/Import/.prism files"));
    assert_eq!((pzfx_spec.label, pzfx_spec.menu_path), ("Import .pzfx files", "File/Import/.pzfx files"));
    assert!(archive_spec.summary.contains(".prism files") && pzfx_spec.summary.contains(".pzfx files"));
}

#[test]
fn table_multiple_variables_and_simulate_xy_commands() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "mv-1", "archetype": "multiple_variables", "cols": 2, "rows": 3, "variables": [{ "title": "Arm", "role": "Categorical" }, { "title": "Dose", "role": "Continuous" }] }),
            ),
            (
                "table.setCells",
                json!({ "sheet_id": "mv-1", "edits": [{ "row": 0, "col": 0, "value": "veh" }, { "row": 1, "col": 0, "value": "low" }, { "row": 2, "col": 0, "value": "high" }, { "row": 0, "col": 1, "value": 1.0 }, { "row": 1, "col": 1, "value": 10.0 }, { "row": 2, "col": 1, "value": 100.0 }] }),
            ),
        ],
    );

    let recode =
        ok_cmd(&mut session, "table.recodeLevels", json!({ "sheet_id": "mv-1", "col": "Arm", "mappings": { "veh": "Vehicle", "high": "High" } }));
    assert_eq!(recode.payload["recoded_count"].as_u64(), Some(2));
    assert_eq!(cell_val(&session, "mv-1", 0, 0, 0).map(|c| c.value), Some(CellValue::Text(String::from("Vehicle"))));

    run_cmds(
        &mut session,
        &[
            (
                "table.setCategoricalOrder",
                json!({ "sheet_id": "mv-1", "col": 0, "order": "explicit", "levels": ["Vehicle", "low", "High"], "reversed": true }),
            ),
            ("table.setVariableRole", json!({ "sheet_id": "mv-1", "col": "Dose", "role": "Identifier" })),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "title": "LogDose", "formula": "= log10([Dose])" })),
        ],
    );
    let TableArchetype::MultipleVariables(mv) = &ds(&session, "mv-1").archetype else { panic!("expected MultipleVariables") };
    assert!(mv.variables[0].level_order.reversed);
    assert_eq!(mv.variables[1].role, VariableRole::Label);
    assert!(!mv.variables[1].role_inferred);
    let log_val = cell_num(&session, "mv-1", 2, 2, 0).expect("computed log10(100)");
    assert!((log_val - 2.0).abs() < 1e-9);

    ok_cmd(
        &mut session,
        "table.simulateXy",
        json!({ "sheet_id": "sim-1", "title": "Simulated Curve", "seed": 42, "row_count": 6, "replicates": 2, "x_min": -9.0, "x_max": -4.0, "equation_id": "hill_4pl", "parameters": [0.0, 100.0, -6.5, 1.0], "noise_kind": "gaussian", "noise_sd": 0.5 }),
    );
    assert_eq!(ds(&session, "sim-1").archetype.row_count(), 6);
    assert!(cell_num(&session, "sim-1", 0, 0, 1).is_some());
}

#[test]
fn info_notes_constants_and_hooked_recalculation() {
    let mut doc = ProjectDocument::new("Info Hooks").expect("doc");
    doc.add_info_sheet(InfoSheet::new_preseeded(SheetId::new("info-1"), "Experiment Info").expect("info")).expect("add info");

    let mut mv = MultipleVariablesTable::new(2, 2).expect("mv");
    mv.variables[0].title = String::from("Raw");
    mv.variables[1].title = String::from("Scaled");
    mv.variables[1].set_calculated_formula("= [Raw] * Info(\"ScaleFactor\")");
    let mut data = curvebook_doc::DataSheet::new(SheetId::new("mv-1"), "Assay", TableArchetype::MultipleVariables(mv)).expect("ds");
    data.info_sheet_id = Some(SheetId::new("info-1"));
    data.set_cell(CellCoord::new(0, 0, 0), Cell::number(5.0)).expect("set 5");
    doc.add_data_sheet(data).expect("add ds");

    let mut session = Session::from_document(doc);
    run_cmds(
        &mut session,
        &[
            ("info.setNote", json!({ "sheet_id": "info-1", "text": "Line 1" })),
            ("info.setNote", json!({ "sheet_id": "mv-1", "text": "Line 2", "append": true })),
            ("info.setConstant", json!({ "sheet_id": "info-1", "name": "ScaleFactor", "value": 3.0, "units": "x" })),
        ],
    );
    assert_eq!(info(&session, "info-1").notes_markdown, "Line 1\nLine 2");
    assert_eq!(cell_num(&session, "mv-1", 0, 1, 0), Some(15.0));

    let _ = err_cmd(&mut session, "info.removeConstant", json!({ "sheet_id": "info-1", "name": "Nope" }));
    run_cmds(
        &mut session,
        &[
            ("info.removeConstant", json!({ "sheet_id": "info-1", "name": "Nope", "if_exists": true })),
            ("info.setConstant", json!({ "sheet_id": "mv-1", "name": "ScaleFactor", "value": 4.0 })),
        ],
    );
    assert_eq!(cell_num(&session, "mv-1", 0, 1, 0), Some(20.0));
}

#[test]
fn safety_guards_oversized_inputs_bad_base64_and_catalog_aliases() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "d1", "archetype": "column", "cols": 2, "rows": 3 })),
            ("sheet.createData", json!({ "sheet_id": "d2", "archetype": "column", "cols": 2, "rows": 4 })),
            ("table.setCells", json!({ "sheet_id": "d1", "edits": [{ "row": 0, "col": 0, "value": 11.0 }, { "row": 1, "col": 0, "value": 22.0 }] })),
        ],
    );

    // 1-2. Oversized row indices and bounds must fail cleanly without OOM/panic.
    for (id, params) in [
        ("table.setRowTitles", json!({ "sheet_id": "d1", "row": u64::MAX, "title": "Huge" })),
        ("table.setRowTitles", json!({ "sheet_id": "d1", "start_row": u64::MAX, "titles": ["A", "B"] })),
        ("table.setRowTitles", json!({ "sheet_id": "d1", "start_row": 2, "titles": ["Ok", "PastEnd"] })),
        ("table.excludeCells", json!({ "sheet_id": "d1", "range": { "start": { "row": 0, "col": 0 }, "end": { "row": u64::MAX, "col": 0 } } })),
        ("table.clearCells", json!({ "sheet_id": "d1", "range": { "start": { "row": 0, "col": 0 }, "end": { "row": 100, "col": 0 } } })),
    ] {
        let _ = err_cmd(&mut session, id, params);
    }

    // 3. Malformed base64 padding sequences must return Err across archive open commands.
    for bad_b64 in ["AA=A", "====", "AA==AAAA"] {
        let _ = err_cmd(&mut session, "project.open", json!({ "bytes_base64": bad_b64 }));
        let _ = err_cmd(&mut session, "project.instantiateTemplate", json!({ "bytes_base64": bad_b64 }));
    }

    // 4. Canonical parameter coverage: return_bytes, start_row/start_col, prefix + clear_data, top-level clear box, mappings.
    assert!(ok_cmd(&mut session, "project.save", json!({ "return_bytes": true })).payload["bytes_base64"].is_string());
    assert!(ok_cmd(&mut session, "project.saveTemplate", json!({ "return_bytes": true })).payload["bytes_base64"].is_string());

    ok_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "target_sheet_id": "d2", "source_sheet_id": "d1", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 1, "col": 0 } }, "start_row": 2, "start_col": 1 }),
    );
    assert_eq!(cell_num(&session, "d2", 2, 1, 0), Some(11.0));
    assert_eq!(cell_num(&session, "d2", 3, 1, 0), Some(22.0));

    let dup = ok_cmd(&mut session, "sheet.duplicateFamily", json!({ "sheet_id": "d1", "prefix": "fam", "clear_data": true }));
    let fam_id = dup.payload["sheet_id"].as_str().expect("fam sheet id");
    assert!(fam_id.starts_with("fam"));
    assert_eq!(cell_num(&session, fam_id, 0, 0, 0), None);

    run_cmds(
        &mut session,
        &[
            ("table.clearCells", json!({ "sheet_id": "d1", "row_start": 0, "row_end": 1, "col_start": 0, "col_end": 0 })),
            (
                "sheet.createData",
                json!({ "sheet_id": "mv-alias", "kind": "multiple_variables", "cols": 1, "rows": 2, "variables": [{ "title": "Grp", "role": "Categorical" }] }),
            ),
            ("table.setCells", json!({ "sheet_id": "mv-alias", "edits": [{ "row": 0, "col": 0, "value": "old" }] })),
            ("table.recodeLevels", json!({ "sheet_id": "mv-alias", "col": 0, "mappings": { "old": "new" } })),
        ],
    );
    assert_eq!(cell_num(&session, "d1", 0, 0, 0), None);
    assert_eq!(cell_num(&session, "d1", 1, 0, 0), None);
    assert_eq!(cell_val(&session, "mv-alias", 0, 0, 0).map(|c| c.value), Some(CellValue::Text(String::from("new"))));
}

#[test]
fn aud_02_01_02_03_07_11_16_20_data_bounds_frozen_links_simulate_and_rename() {
    // AUD-02-01: table.excludeCells target="x" rejects nonzero col and counts accurately.
    let mut s_xy = session_with("XY", vec![xy_sheet("xy-1", "Dose", "X", &[1.0, 2.0, 3.0, 4.0], &[("Drug A", &[10.0, 20.0, 30.0, 40.0])])]);
    assert_bad_input(
        &mut s_xy,
        "table.excludeCells",
        json!({ "sheet_id": "xy-1", "target": "x", "row_start": 0, "row_end": 2, "col_start": 0, "col_end": 1_000_000_000_usize }),
    );
    let ex = ok_cmd(
        &mut s_xy,
        "table.excludeCells",
        json!({ "sheet_id": "xy-1", "target": "x", "row_start": 0, "row_end": 2, "col_start": 0, "col_end": 0 }),
    );
    assert_eq!(ex.payload["updated_count"], 3);
    let d_xy = ds(&s_xy, "xy-1");
    assert!(
        d_xy.x_or_time_cell(0, 0).is_some_and(|c| c.excluded)
            && d_xy.x_or_time_cell(2, 0).is_some_and(|c| c.excluded)
            && d_xy.x_or_time_cell(3, 0).is_some_and(|c| !c.excluded)
    );

    // AUD-02-16: transform.run simulate rejects zero row_count/replicates and preserves row_count=1.
    assert_bad_inputs(
        &mut s_xy,
        &[
            ("transform.run", json!({ "source_sheet_id": "xy-1", "kind": "simulate", "row_count": 0, "replicates": 2 })),
            ("transform.run", json!({ "source_sheet_id": "xy-1", "kind": "simulate", "row_count": 5, "replicates": 0 })),
        ],
    );
    ok_cmd(
        &mut s_xy,
        "transform.run",
        json!({ "sheet_id": "sim-one", "source_sheet_id": "xy-1", "kind": "simulate", "row_count": 1, "replicates": 1 }),
    );
    assert_eq!(rs(&s_xy, "sim-one").analyzable_data.as_ref().map(|d| d.archetype.row_count()), Some(1));

    // AUD-02-02: pasteLink & clear_cell_range clamp unbounded ranges; AUD-02-20: sheet.rename rejects whitespace-only titles.
    let mut s_col = session_with("Col", vec![col_sheet("col-1", "Groups", &[("Ctrl", &[1.0, 2.0]), ("Drug", &[3.0, 4.0])])]);
    run_cmds(
        &mut s_col,
        &[
            ("sheet.createData", json!({ "sheet_id": "col-dst", "kind": "column", "cols": 2, "rows": 2 })),
            ("stats.run", json!({ "sheet_id": "res-1", "source_sheet_id": "col-1", "kind": "descriptive" })),
        ],
    );
    let pl = ok_cmd(
        &mut s_col,
        "table.pasteLink",
        json!({ "target_sheet_id": "col-dst", "source_sheet_id": "col-1", "source_range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 1_000_000_000_usize, "col": 1_000_000_000_usize, "subcol": 0 } }, "propagate_title_header": true }),
    );
    assert_eq!((&pl.payload["rows_placed"], &pl.payload["cols_placed"]), (&json!(2), &json!(2)));
    let mut doc = s_col.document().clone();
    if let Some(dst) = doc.data_sheet_mut(&SheetId::new("col-dst")) {
        dst.linked_regions[0].target_range = SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(usize::MAX, usize::MAX, 0));
    }
    assert!(!dag::recalculate_project_with_report(&mut doc).expect("recalc").updated_data_sheets.is_empty());
    assert_bad_inputs(
        &mut s_col,
        &[("sheet.rename", json!({ "sheet_id": "col-1", "title": "   \t  " })), ("sheet.rename", json!({ "sheet_id": "res-1", "title": "   " }))],
    );
    assert_eq!(ds(&s_col, "col-1").title, "Groups");
    assert!(rs(&s_col, "res-1").analysis.custom_title.is_none());

    // AUD-02-03: randomizeGroups rejects huge subject_count without OOM.
    let mut s = Session::new();
    assert_bad_inputs(
        &mut s,
        &[
            ("table.randomizeGroups", json!({ "groups": ["Vehicle", "Drug"], "subject_count": 100_000_000_usize })),
            ("table.randomizeGroups", json!({ "groups": ["Vehicle", "Drug"], "subject_count": 1_000_001_usize })),
        ],
    );
    assert_eq!(
        ok_cmd(&mut s, "table.randomizeGroups", json!({ "groups": ["Vehicle", "Drug"], "subject_count": 6, "seed": 7 })).payload["subject_count"],
        6
    );

    // AUD-02-07: frozen DataSheet protected from paired InfoSheet delete and upstream XY convertArchetype.
    ok_cmd(&mut s, "sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "cols": 1, "rows": 3, "auto_info": true }));
    let info_id = ds(&s, "d1").info_sheet_id.clone().expect("info_sheet_id");
    ok_cmd(&mut s, "sheet.freeze", json!({ "sheet_id": "d1" }));
    assert_err_code(&mut s, "sheet.delete", json!({ "sheet_id": info_id.as_str(), "cascade": true }), ErrorCode::FrozenSheet);
    assert_eq!(ds(&s, "d1").info_sheet_id.as_ref(), Some(&info_id));
    run_cmds(
        &mut s,
        &[
            ("sheet.createData", json!({ "sheet_id": "src-xy", "kind": "xy", "cols": 1, "rows": 3 })),
            ("table.setXCells", json!({ "sheet_id": "src-xy", "values": [1.0, 2.0, 3.0] })),
            ("table.setCells", json!({ "sheet_id": "src-xy", "rows": [[10.0], [20.0], [30.0]] })),
            ("sheet.createData", json!({ "sheet_id": "dst-frozen", "kind": "column", "cols": 1, "rows": 3 })),
            ("table.pasteLink", json!({ "target_sheet_id": "dst-frozen", "source_sheet_id": "src-xy" })),
            ("sheet.freeze", json!({ "sheet_id": "dst-frozen" })),
        ],
    );
    assert_err_code(
        &mut s,
        "table.convertArchetype",
        json!({ "sheet_id": "src-xy", "target_kind": "MultipleVariables", "allow_lossy": true }),
        ErrorCode::FrozenSheet,
    );
    assert_eq!(ds(&s, "dst-frozen").linked_regions.len(), 1);

    // AUD-02-11: table.importCsv link=true rejects overlapping anchor with Conflict and mints unique IDs for disjoint links.
    let tmp = TempDir::new("aud0211");
    let (csv_a, csv_b) = (tmp.child("a.csv"), tmp.child("b.csv"));
    std::fs::write(&csv_a, "1,10\n2,20\n").expect("write a");
    std::fs::write(&csv_b, "3,30\n4,40\n").expect("write b");
    ok_cmd(&mut s, "sheet.createData", json!({ "sheet_id": "lnk", "kind": "column", "cols": 5, "rows": 4 }));
    ok_cmd(
        &mut s,
        "table.importCsv",
        json!({ "sheet_id": "lnk", "path": csv_a.to_string_lossy(), "link": true, "has_header": false, "anchor": { "row": 0, "col": 0 } }),
    );
    assert_err_code(
        &mut s,
        "table.importCsv",
        json!({ "sheet_id": "lnk", "path": csv_b.to_string_lossy(), "link": true, "has_header": false, "anchor": { "row": 0, "col": 0 } }),
        ErrorCode::Conflict,
    );
    ok_cmd(
        &mut s,
        "table.importCsv",
        json!({ "sheet_id": "lnk", "path": csv_b.to_string_lossy(), "link": true, "has_header": false, "anchor": { "row": 0, "col": 3 } }),
    );
    let regions = &ds(&s, "lnk").linked_regions;
    assert!(regions.len() == 2 && regions[0].id != regions[1].id);
}

#[test]
fn table_paste_transpose_swaps_grid_rejects_transpose_field_and_honors_guards() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "col-1", "archetype": "column", "cols": 3, "rows": 4 })),
            ("sheet.createData", json!({ "sheet_id": "src-1", "archetype": "column", "cols": 1, "rows": 2 })),
            ("table.setCells", json!({ "sheet_id": "src-1", "edits": [{ "row": 0, "col": 0, "value": 7.0 }, { "row": 1, "col": 0, "value": 8.0 }] })),
        ],
    );

    let out = ok_cmd(
        &mut session,
        "table.pasteTranspose",
        json!({
            "sheet_id": "col-1",
            "text": "1,5;2,75*\n3,25;4,5",
            "delimiter": ";",
            "decimal_separator": ",",
            "anchor": { "row": 0, "col": 0, "subcol": 0 }
        }),
    );
    assert!(out.mutated);
    assert_eq!(out.payload["transposed"], json!(true));
    assert_eq!(out.payload["rows_placed"], json!(2));
    assert_eq!(out.payload["cols_placed"], json!(2));
    assert_eq!(cell_num(&session, "col-1", 0, 0, 0), Some(1.5));
    assert_eq!(cell_num(&session, "col-1", 0, 1, 0), Some(3.25));
    let excl = cell_val(&session, "col-1", 1, 0, 0).expect("transposed excluded");
    assert_eq!(excl.value, CellValue::Number(2.75));
    assert!(excl.excluded);
    assert_eq!(cell_num(&session, "col-1", 1, 1, 0), Some(4.5));

    // Explicit `transpose` parameter is not accepted on `table.pasteTranspose`.
    assert_bad_input(&mut session, "table.pasteTranspose", json!({ "sheet_id": "col-1", "rows": [[1.0, 2.0]], "transpose": false }));

    // Undo / redo round-trip.
    assert!(session.undo().expect("undo pasteTranspose"));
    assert_eq!(cell_num(&session, "col-1", 0, 0, 0), None);
    assert!(session.redo().expect("redo pasteTranspose"));
    assert_eq!(cell_num(&session, "col-1", 0, 0, 0), Some(1.5));

    // Overlapping live linked region returns Conflict; frozen sheet returns FrozenSheet.
    ok_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "col-1", "source_sheet_id": "src-1", "anchor": { "row": 2, "col": 0 } }));
    assert_err_code(
        &mut session,
        "table.pasteTranspose",
        json!({ "sheet_id": "col-1", "rows": [[9.0, 10.0]], "start_row": 2, "start_col": 0 }),
        ErrorCode::Conflict,
    );
    ok_cmd(&mut session, "sheet.freeze", json!({ "sheet_id": "col-1" }));
    assert_err_code(&mut session, "table.pasteTranspose", json!({ "sheet_id": "col-1", "rows": [[1.0, 2.0]] }), ErrorCode::FrozenSheet);
}

#[test]
fn table_sort_rows_column_ties_exclusions_overrides_and_partial_range() {
    use curvebook_doc::PlateCellOrigin;

    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 2, "rows": 5 })),
            ("table.setRowTitles", json!({ "sheet_id": "col-1", "titles": ["T0", "T1", "T2", "T3", "T4"] })),
            (
                "table.setCells",
                json!({
                    "sheet_id": "col-1",
                    "edits": [
                        { "row": 0, "col": 0, "value": 20.0 },
                        { "row": 1, "col": 0, "value": null },
                        { "row": 2, "col": 0, "value": 10.0 },
                        { "row": 3, "col": 0, "value": 20.0 },
                        { "row": 4, "col": 0, "value": 5.0 },
                        { "row": 0, "col": 1, "value": 100.0 },
                        { "row": 1, "col": 1, "value": 101.0 },
                        { "row": 2, "col": 1, "value": 102.0 },
                        { "row": 3, "col": 1, "value": 103.0 },
                        { "row": 4, "col": 1, "value": 104.0 },
                    ]
                }),
            ),
            (
                "table.excludeCellsWithReason",
                json!({ "sheet_id": "col-1", "coords": [{ "row": 4, "col": 0 }], "reason_tag": "outlier", "note": "low 5", "timestamp_utc": "unix:10" }),
            ),
            ("graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "col-1", "kind": "column" })),
            (
                "graph.formatSelectedPoints",
                json!({ "sheet_id": "gr-1", "points": [{ "series_index": 0, "point_index": 2 }], "color": "#D55E00", "symbol": "diamond" }),
            ),
            ("stats.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "col-1", "kind": "descriptive_stats" })),
        ],
    );
    let mut doc = session.document().clone();
    doc.data_sheet_mut(&SheetId::new("col-1")).expect("col-1").plate_well_origins = vec![
        PlateCellOrigin { coord: CellCoord::new(2, 0, 0), well: String::from("B02") },
        PlateCellOrigin { coord: CellCoord::new(4, 0, 0), well: String::from("E05") },
    ];
    let mut session = Session::from_document(doc);

    let sorted = ok_cmd(&mut session, "table.sortRows", json!({ "sheet_id": "col-1", "col": 0 }));
    assert!(sorted.mutated && sorted.payload["rows_sorted"] == 5 && sorted.payload["changed"] == true && sorted.payload["direction"] == "asc");
    assert!(sorted.payload["recalculated_sheets"].as_u64().unwrap_or(0) >= 1);

    let TableArchetype::Column(ct) = &ds(&session, "col-1").archetype else { panic!("expected Column") };
    assert_eq!(ct.row_titles, vec!["T4", "T2", "T0", "T3", "T1"]);
    assert_eq!(cell_val(&session, "col-1", 0, 0, 0), Some(Cell::excluded_number(5.0)));
    assert_eq!((cell_num(&session, "col-1", 2, 1, 0), cell_num(&session, "col-1", 3, 1, 0)), (Some(100.0), Some(103.0)));
    assert!(cell_val(&session, "col-1", 4, 0, 0).is_some_and(|c| c.is_empty()));

    let d_col = ds(&session, "col-1");
    assert_eq!(d_col.exclusion_notes.len(), 1);
    assert_eq!((d_col.exclusion_notes[0].coord, d_col.exclusion_notes[0].note.as_str()), (CellCoord::new(0, 0, 0), "low 5"));
    assert_eq!(
        d_col.plate_well_origins,
        vec![
            PlateCellOrigin { coord: CellCoord::new(0, 0, 0), well: String::from("E05") },
            PlateCellOrigin { coord: CellCoord::new(1, 0, 0), well: String::from("B02") },
        ]
    );
    assert_eq!(gs(&session, "gr-1").spec.point_overrides[0].point_index, 1);

    let noop = ok_cmd(&mut session, "table.sortRows", json!({ "sheet_id": "col-1", "col": 0 }));
    assert!(!noop.mutated && noop.payload["changed"] == false);

    ok_cmd(&mut session, "table.sortRows", json!({ "sheet_id": "col-1", "col": 0, "direction": "desc", "empty_placement": "first" }));
    let TableArchetype::Column(ct_desc) = &ds(&session, "col-1").archetype else { panic!("expected Column") };
    assert_eq!(ct_desc.row_titles, vec!["T1", "T0", "T3", "T2", "T4"]);
    assert_eq!(gs(&session, "gr-1").spec.point_overrides[0].point_index, 3);

    assert!(session.undo().expect("undo desc sort"));
    assert_eq!(gs(&session, "gr-1").spec.point_overrides[0].point_index, 1);

    ok_cmd(&mut session, "table.sortRows", json!({ "sheet_id": "col-1", "by": "row_title", "direction": "desc", "row_start": 1, "row_end": 3 }));
    let TableArchetype::Column(ct_partial) = &ds(&session, "col-1").archetype else { panic!("expected Column") };
    assert_eq!(ct_partial.row_titles, vec!["T4", "T3", "T2", "T0", "T1"]);
}

#[test]
fn table_sort_rows_replicates_and_presummarized_modes() {
    let mut s_rep = Session::new();
    run_cmds(
        &mut s_rep,
        &[
            ("sheet.createData", json!({ "sheet_id": "grp-rep", "kind": "grouped", "cols": 1, "rows": 3, "replicates": 2 })),
            ("table.setRowTitles", json!({ "sheet_id": "grp-rep", "titles": ["HighMean", "LowMean", "MidMean"] })),
            (
                "table.setCells",
                json!({
                    "sheet_id": "grp-rep",
                    "edits": [
                        { "row": 0, "col": 0, "subcol": 0, "value": 10.0 }, { "row": 0, "col": 0, "subcol": 1, "value": 30.0 },
                        { "row": 1, "col": 0, "subcol": 0, "value": 12.0 }, { "row": 1, "col": 0, "subcol": 1, "value": 2.0 },
                        { "row": 2, "col": 0, "subcol": 0, "value": 15.0 }, { "row": 2, "col": 0, "subcol": 1, "value": 15.0 },
                    ]
                }),
            ),
            ("sheet.createData", json!({ "sheet_id": "grp-sum", "kind": "grouped", "cols": 1, "rows": 2, "subcolumn_mode": "mean_sd_n" })),
            ("table.setRowTitles", json!({ "sheet_id": "grp-sum", "titles": ["BigMean", "SmallMean"] })),
            (
                "table.setCells",
                json!({
                    "sheet_id": "grp-sum",
                    "edits": [
                        { "row": 0, "col": 0, "subcol": 0, "value": 50.0 }, { "row": 0, "col": 0, "subcol": 1, "value": 1.0 }, { "row": 0, "col": 0, "subcol": 2, "value": 3.0 },
                        { "row": 1, "col": 0, "subcol": 0, "value": 10.0 }, { "row": 1, "col": 0, "subcol": 1, "value": 99.0 }, { "row": 1, "col": 0, "subcol": 2, "value": 3.0 },
                    ]
                }),
            ),
        ],
    );
    ok_cmd(&mut s_rep, "table.sortRows", json!({ "sheet_id": "grp-rep", "col": 0 }));
    let TableArchetype::Grouped(gt) = &ds(&s_rep, "grp-rep").archetype else { panic!("expected Grouped") };
    assert_eq!(gt.row_titles, vec!["LowMean", "MidMean", "HighMean"]);

    ok_cmd(&mut s_rep, "table.sortRows", json!({ "sheet_id": "grp-rep", "col": 0, "subcol": 0 }));
    let TableArchetype::Grouped(gt2) = &ds(&s_rep, "grp-rep").archetype else { panic!("expected Grouped") };
    assert_eq!(gt2.row_titles, vec!["HighMean", "LowMean", "MidMean"]);

    ok_cmd(&mut s_rep, "table.sortRows", json!({ "sheet_id": "grp-sum", "col": 0 }));
    let TableArchetype::Grouped(gs_t) = &ds(&s_rep, "grp-sum").archetype else { panic!("expected Grouped") };
    assert_eq!(gs_t.row_titles, vec!["SmallMean", "BigMean"]);
}

#[test]
fn table_sort_rows_generated_x_calculated_mv_parts_and_link_guards() {
    use curvebook_doc::{SequenceProgression, SubcolumnMode, XColumnMode, XyTable};

    let mut gen_xy = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 10.0, step: 5.0, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        1,
        3,
    )
    .expect("gen xy");
    gen_xy.data_sets[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)], vec![Cell::number(3.0)]];
    let mut gen_doc = ProjectDocument::new("Sort Archetypes").expect("doc");
    gen_doc.add_data_sheet(curvebook_doc::DataSheet::new("xy-gen", "Gen XY", TableArchetype::Xy(gen_xy)).expect("ds")).expect("add");
    let mut s_arch = Session::from_document(gen_doc);

    let gen_asc = ok_cmd(&mut s_arch, "table.sortRows", json!({ "sheet_id": "xy-gen", "by": "x", "direction": "asc" }));
    assert_eq!(gen_asc.payload["changed"], json!(false));
    assert!(matches!(&ds(&s_arch, "xy-gen").archetype, TableArchetype::Xy(t) if matches!(t.x_mode, XColumnMode::GeneratedSequence { .. })));

    let gen_desc = ok_cmd(&mut s_arch, "table.sortRows", json!({ "sheet_id": "xy-gen", "by": "x", "direction": "desc" }));
    assert_eq!(gen_desc.payload["changed"], json!(true));
    let TableArchetype::Xy(xy_after) = &ds(&s_arch, "xy-gen").archetype else { panic!("expected Xy") };
    assert_eq!(xy_after.x_mode, XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single });
    assert_eq!((xy_after.x_value_at(0), cell_num(&s_arch, "xy-gen", 0, 0, 0)), (Some(20.0), Some(3.0)));
    assert_eq!((xy_after.x_value_at(2), cell_num(&s_arch, "xy-gen", 2, 0, 0)), (Some(10.0), Some(1.0)));

    run_cmds(
        &mut s_arch,
        &[
            ("sheet.createData", json!({ "sheet_id": "mv-1", "kind": "multiple_variables", "cols": 2, "rows": 3, "variables": ["Val", "Doubled"] })),
            (
                "table.setCells",
                json!({ "sheet_id": "mv-1", "edits": [{ "row": 0, "col": 0, "value": 30.0 }, { "row": 1, "col": 0, "value": 10.0 }, { "row": 2, "col": 0, "value": 20.0 }] }),
            ),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "col": 1, "formula": "= [Val] * 2" })),
            ("table.sortRows", json!({ "sheet_id": "mv-1", "col": 0 })),
            ("sheet.createData", json!({ "sheet_id": "pow-1", "kind": "parts_of_whole", "cols": 1, "rows": 3 })),
            ("table.setRowTitles", json!({ "sheet_id": "pow-1", "titles": ["Item 10", "Item 2", "Item 1"] })),
            ("table.sortRows", json!({ "sheet_id": "pow-1", "by": "row_title" })),
            ("sheet.createData", json!({ "sheet_id": "src-lnk", "kind": "column", "cols": 1, "rows": 2 })),
            (
                "table.setCells",
                json!({ "sheet_id": "src-lnk", "edits": [{ "row": 0, "col": 0, "value": 9.0 }, { "row": 1, "col": 0, "value": 1.0 }] }),
            ),
            ("sheet.createData", json!({ "sheet_id": "dst-lnk", "kind": "column", "cols": 1, "rows": 4 })),
            ("table.pasteLink", json!({ "target_sheet_id": "dst-lnk", "source_sheet_id": "src-lnk", "anchor": { "row": 0, "col": 0 } })),
            (
                "table.setCells",
                json!({ "sheet_id": "dst-lnk", "edits": [{ "row": 2, "col": 0, "value": 40.0 }, { "row": 3, "col": 0, "value": 10.0 }] }),
            ),
        ],
    );
    assert_eq!((cell_num(&s_arch, "mv-1", 0, 1, 0), cell_num(&s_arch, "mv-1", 2, 1, 0)), (Some(20.0), Some(60.0)));
    let TableArchetype::PartsOfWhole(pow_t) = &ds(&s_arch, "pow-1").archetype else { panic!("expected PartsOfWhole") };
    assert_eq!(pow_t.slice_labels, vec!["Item 1", "Item 2", "Item 10"]);

    assert_err_code(&mut s_arch, "table.sortRows", json!({ "sheet_id": "dst-lnk" }), ErrorCode::Conflict);
    ok_cmd(&mut s_arch, "table.sortRows", json!({ "sheet_id": "dst-lnk", "row_start": 2, "row_end": 3 }));
    assert_eq!((cell_num(&s_arch, "dst-lnk", 2, 0, 0), cell_num(&s_arch, "dst-lnk", 3, 0, 0)), (Some(10.0), Some(40.0)));
    ok_cmd(&mut s_arch, "sheet.freeze", json!({ "sheet_id": "dst-lnk" }));
    assert_err_code(&mut s_arch, "table.sortRows", json!({ "sheet_id": "dst-lnk", "row_start": 2, "row_end": 3 }), ErrorCode::FrozenSheet);
}

#[test]
fn sheet_floating_notes_crud_across_all_five_sections_and_presets() {
    use curvebook_doc::FloatingNote;
    use curvebook_palette::Rgba8;

    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            (
                "sheet.createData",
                json!({ "sheet_id": "d1", "kind": "column", "cols": 2, "rows": 3, "auto_info": true, "info_sheet_id": "i1", "auto_graph": true, "graph_sheet_id": "g1" }),
            ),
            ("table.setCells", json!({ "sheet_id": "d1", "edits": [{ "row": 0, "col": 0, "value": 10.0 }, { "row": 1, "col": 0, "value": 20.0 }] })),
            ("stats.run", json!({ "sheet_id": "r1", "source_sheet_id": "d1", "kind": "descriptive_stats" })),
            ("layout.create", json!({ "sheet_id": "l1", "title": "Figure 1" })),
        ],
    );

    for (sid, color_in, expected_fill) in [
        ("d1", "yellow", FloatingNote::DEFAULT_FILL),
        ("i1", "blue", FloatingNote::BLUE_FILL),
        ("r1", "green", FloatingNote::GREEN_FILL),
        ("g1", "red", FloatingNote::RED_FILL),
        ("l1", "gray", FloatingNote::GRAY_FILL),
    ] {
        let added = ok_cmd(
            &mut session,
            "sheet.addFloatingNote",
            json!({ "sheet_id": sid, "body_markdown": format!("Note on {sid}"), "fill_color": color_in }),
        );
        assert!(added.mutated && added.payload["note_id"] == "note-1");
        assert_eq!(added.payload["bounds_pt"], json!({ "x": 24.0, "y": 24.0, "width": 240.0, "height": 120.0 }));
        assert_eq!(serde_json::from_value::<Rgba8>(added.payload["fill_color"].clone()).expect("rgba8"), expected_fill);
    }

    let n2 = ok_cmd(
        &mut session,
        "sheet.addFloatingNote",
        json!({ "sheet_id": "d1", "body_markdown": "Second note", "fill_color": "#112233", "minimized": true }),
    );
    assert_eq!(n2.payload["note_id"], json!("note-2"));
    assert_eq!(n2.payload["bounds_pt"], json!({ "x": 44.0, "y": 44.0, "width": 240.0, "height": 120.0 }));
    assert_eq!(ds(&session, "d1").floating_notes[1].fill_color, Rgba8::rgb(0x11, 0x22, 0x33));

    assert_err_code(
        &mut session,
        "sheet.addFloatingNote",
        json!({ "sheet_id": "d1", "note_id": "note-1", "body_markdown": "dup" }),
        ErrorCode::AlreadyExists,
    );
    assert_bad_input(&mut session, "sheet.updateFloatingNote", json!({ "sheet_id": "d1", "note_id": "note-1" }));
    assert_err_code(
        &mut session,
        "sheet.updateFloatingNote",
        json!({ "sheet_id": "d1", "note_id": "missing", "minimized": true }),
        ErrorCode::NotFound,
    );
    ok_cmd(
        &mut session,
        "sheet.updateFloatingNote",
        json!({
            "sheet_id": "d1",
            "note_id": "note-1",
            "body_markdown": "Updated body",
            "bounds_pt": { "x": 100.0, "y": 80.0, "width": 300.0, "height": 150.0 },
            "fill_color": { "r": 220, "g": 252, "b": 231, "a": 255 },
            "minimized": true
        }),
    );
    let updated = &ds(&session, "d1").floating_notes[0];
    assert_eq!(
        (updated.body_markdown.as_str(), updated.bounds_pt, updated.fill_color, updated.minimized),
        ("Updated body", RectPt::new(100.0, 80.0, 300.0, 150.0), FloatingNote::GREEN_FILL, true)
    );

    assert_err_code(&mut session, "sheet.deleteFloatingNote", json!({ "sheet_id": "d1", "note_id": "missing" }), ErrorCode::NotFound);
    let del = ok_cmd(&mut session, "sheet.deleteFloatingNote", json!({ "sheet_id": "d1", "note_id": "note-2" }));
    assert_eq!(del.payload["remaining_notes"], json!(1));
    assert!(session.undo().expect("undo delete"));
    assert_eq!(ds(&session, "d1").floating_notes.len(), 2);
}

#[test]
fn sheet_floating_notes_frozen_and_resource_limits() {
    use curvebook_doc::ids::{MAX_FLOATING_NOTE_BODY_BYTES, MAX_FLOATING_NOTES_PER_SHEET};

    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "d1", "kind": "column", "cols": 1, "rows": 2, "auto_info": true, "info_sheet_id": "i1" })),
            ("sheet.addFloatingNote", json!({ "sheet_id": "d1", "body_markdown": "initial" })),
            ("sheet.freeze", json!({ "sheet_id": "d1" })),
        ],
    );
    assert_err_code(&mut session, "sheet.addFloatingNote", json!({ "sheet_id": "d1", "body_markdown": "blocked" }), ErrorCode::FrozenSheet);
    assert_err_code(
        &mut session,
        "sheet.updateFloatingNote",
        json!({ "sheet_id": "d1", "note_id": "note-1", "minimized": true }),
        ErrorCode::FrozenSheet,
    );
    assert_err_code(&mut session, "sheet.deleteFloatingNote", json!({ "sheet_id": "d1", "note_id": "note-1" }), ErrorCode::FrozenSheet);

    assert_err_code(
        &mut session,
        "sheet.addFloatingNote",
        json!({ "sheet_id": "i1", "body_markdown": "x".repeat(MAX_FLOATING_NOTE_BODY_BYTES + 1) }),
        ErrorCode::ResourceLimit,
    );
    ok_cmd(&mut session, "sheet.addFloatingNote", json!({ "sheet_id": "i1", "body_markdown": "x".repeat(MAX_FLOATING_NOTE_BODY_BYTES) }));
    assert_err_code(
        &mut session,
        "sheet.updateFloatingNote",
        json!({ "sheet_id": "i1", "note_id": "note-1", "body_markdown": "x".repeat(MAX_FLOATING_NOTE_BODY_BYTES + 1) }),
        ErrorCode::ResourceLimit,
    );
    for _ in 1..MAX_FLOATING_NOTES_PER_SHEET {
        ok_cmd(&mut session, "sheet.addFloatingNote", json!({ "sheet_id": "i1" }));
    }
    assert_eq!(info(&session, "i1").floating_notes.len(), MAX_FLOATING_NOTES_PER_SHEET);
    assert_err_code(&mut session, "sheet.addFloatingNote", json!({ "sheet_id": "i1" }), ErrorCode::ResourceLimit);
}
