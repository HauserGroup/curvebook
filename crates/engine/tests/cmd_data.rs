//! Integration tests for all document, sheet, table, import/link, variable, and info commands in `cmd_data`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ConstantValue, FramePreset, GraphSheet, HookSource, JournalWidthPreset, LayoutItem,
    LayoutSheet, LayoutSpec, NonlinearFitSpec, ParameterRule, ParameterScope, ProjectDocument, RectPt, ResultSheet, ResultStatus, SheetId,
    TableArchetype, TableKind, VariableRole,
};
use curvebook_engine::{Caller, Session, cmd};
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

fn unique_temp_path(stem: &str, ext: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("curvebook_engine_test_{stem}_{}_{seq}.{ext}", std::process::id()))
}

fn data_cell(session: &Session, sheet_id: &str, row: usize, col: usize, subcol: usize) -> Option<Cell> {
    let id = SheetId::new(sheet_id);
    session.document().data_sheet(&id).and_then(|d| d.cell(CellCoord::new(row, col, subcol))).cloned()
}

fn cell_num(cell: &Cell) -> Option<f64> {
    match cell.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

#[test]
fn sheet_create_all_eight_archetypes_auto_family_and_inspect() {
    let mut session = Session::new();

    // Default create without auto_* creates ONLY the data sheet (UI navigator contract).
    let plain = session.execute("sheet.createData", json!({ "sheet_id": "data-plain", "title": "Assay" })).expect("create plain data sheet");
    assert!(plain.mutated);
    assert_eq!(session.document().total_sheet_count(), 1);
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("data-plain"));

    // auto_family: true provisions Data + paired Info + uninitialized Graph while keeping active_sheet_id on Data.
    let xy = session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "xy-1",
                "title": "Dose Response",
                "archetype": "xy",
                "replicates": 3,
                "cols": 2,
                "rows": 8,
                "x_title": "Log Conc (M)",
                "auto_family": true,
            }),
        )
        .expect("create xy auto family");
    assert_eq!(xy.payload.get("kind").and_then(Value::as_str), Some("Xy"));
    let xy_info = xy.payload.get("info_sheet_id").and_then(Value::as_str).expect("auto info id");
    let xy_graph = xy.payload.get("graph_sheet_id").and_then(Value::as_str).expect("auto graph id");
    assert!(session.document().info_sheet(&SheetId::new(xy_info)).is_some());
    assert!(session.document().graph_sheet(&SheetId::new(xy_graph)).is_some());
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("xy-1"));

    // auto_family: true with auto_graph: false suppresses GraphSheet while keeping paired InfoSheet.
    let col = session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "col-1",
                "title": "Treatments",
                "kind": "column",
                "design_hint": "paired",
                "cols": 3,
                "rows": 6,
                "auto_family": true,
                "auto_graph": false,
            }),
        )
        .expect("create column");
    assert!(col.payload.get("info_sheet_id").and_then(Value::as_str).is_some());
    assert!(col.payload.get("graph_sheet_id").and_then(Value::as_str).is_none());
    assert_eq!(session.document().graph_sheets.len(), 1);

    // Remaining 6 TableArchetypes: Grouped, Contingency, Survival, PartsOfWhole, MultipleVariables, Nested.
    session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "grp-1",
                "archetype": "grouped",
                "replicates": 3,
                "cols": 2,
                "rows": 4,
                "row_factor_title": "Genotype",
                "column_factor_title": "Treatment",
            }),
        )
        .expect("create grouped");

    session
        .execute("sheet.createData", json!({ "sheet_id": "cont-1", "archetype": "contingency", "cols": 2, "rows": 3 }))
        .expect("create contingency");

    session
        .execute("sheet.createData", json!({ "sheet_id": "surv-1", "archetype": "survival", "cols": 2, "rows": 10, "time_title": "Days" }))
        .expect("create survival");

    session
        .execute("sheet.createData", json!({ "sheet_id": "pow-1", "archetype": "parts_of_whole", "cols": 1, "rows": 5 }))
        .expect("create parts of whole");

    session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "mv-1",
                "archetype": "multiple_variables",
                "cols": 3,
                "rows": 5,
                "variables": [
                    { "title": "SampleID", "role": "Identifier" },
                    { "title": "Group", "role": "Categorical" },
                    { "title": "Signal", "role": "Continuous" }
                ]
            }),
        )
        .expect("create multiple variables");

    session
        .execute("sheet.createData", json!({ "sheet_id": "nest-1", "archetype": "nested", "cols": 2, "biological_units": 4, "rows": 5 }))
        .expect("create nested");

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

    // project.inspect (read-only) returns all sections plus per-sheet focus details.
    let inspect = session.execute("project.inspect", json!({ "sheet_id": xy_graph })).expect("inspect");
    assert!(!inspect.mutated);
    assert_eq!(inspect.payload.get("data_sheets").and_then(Value::as_array).map(Vec::len), Some(9));
    assert_eq!(inspect.payload.get("focus").and_then(|f| f.get("section")).and_then(Value::as_str), Some("Graphs"));
}

#[test]
fn project_new_save_open_and_template_round_trip() {
    let proj_path = unique_temp_path("proj", "curvebook");
    let tpl_path = unique_temp_path("tpl", "cbtemplate");

    let mut session = Session::new();
    session.execute("project.new", json!({ "title": "Ligand Panel" })).expect("project.new");
    assert_eq!(session.document().title, "Ligand Panel");

    session
        .execute(
            "sheet.createData",
            json!({ "sheet_id": "data-1", "title": "Plate 1", "archetype": "xy", "replicates": 2, "cols": 1, "rows": 4, "info_sheet_id": "info-1" }),
        )
        .expect("create data");
    session.execute("table.setXCells", json!({ "sheet_id": "data-1", "values": [1.0, 2.0, 4.0] })).expect("set x");
    session
        .execute(
            "table.setCells",
            json!({
                "sheet_id": "data-1",
                "edits": [
                    { "row": 0, "col": 0, "subcol": 0, "value": 10.5 },
                    { "row": 1, "col": 0, "subcol": 0, "value": 20.5 }
                ]
            }),
        )
        .expect("set y");
    session.execute("info.setNote", json!({ "sheet_id": "info-1", "note": "Protocol v2" })).expect("set note");
    session.execute("info.setConstant", json!({ "sheet_id": "info-1", "name": "TempC", "value": 37.0, "units": "C" })).expect("set const");

    let save_res = session.execute("project.save", json!({ "path": proj_path.to_str() })).expect("project.save");
    assert!(!save_res.mutated);

    let tpl_res = session.execute("project.saveTemplate", json!({ "path": tpl_path.to_str(), "return_bytes": true })).expect("project.saveTemplate");
    assert!(!tpl_res.mutated);
    let tpl_b64 = tpl_res.payload.get("bytes_base64").and_then(Value::as_str).expect("template b64");

    // Reopen saved archive via file.new -> project.open.
    let mut reopened = Session::new();
    reopened.execute("file.new", json!({ "title": "Scratch" })).expect("file.new");
    reopened.execute("project.open", json!({ "path": proj_path.to_str() })).expect("project.open");
    assert_eq!(reopened.document().title, "Ligand Panel");
    assert_eq!(data_cell(&reopened, "data-1", 0, 0, 0).and_then(|c| cell_num(&c)), Some(10.5));
    assert_eq!(reopened.document().info_sheet(&SheetId::new("info-1")).map(|i| i.notes_markdown.as_str()), Some("Protocol v2"));

    // Instantiate template (both path and base64) preserves structure + constants while clearing assay measurements.
    let mut from_tpl = Session::new();
    from_tpl.execute("project.instantiateTemplate", json!({ "path": tpl_path.to_str(), "title": "Plate 2 Run" })).expect("instantiate template path");
    assert_eq!(from_tpl.document().title, "Plate 2 Run");
    assert!(from_tpl.document().data_sheet(&SheetId::new("data-1")).is_some());
    assert!(from_tpl.document().info_sheet(&SheetId::new("info-1")).is_some());
    assert_eq!(data_cell(&from_tpl, "data-1", 0, 0, 0).and_then(|c| cell_num(&c)), None);

    let mut from_b64 = Session::new();
    from_b64.execute("project.instantiateTemplate", json!({ "bytes_base64": tpl_b64, "title": "Plate 3 Run" })).expect("instantiate template b64");
    assert_eq!(from_b64.document().title, "Plate 3 Run");

    let _ = std::fs::remove_file(proj_path);
    let _ = std::fs::remove_file(tpl_path);
}

#[test]
fn sheet_rename_delete_duplicate_reorder_and_select() {
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "d1", "title": "First", "info_sheet_id": "i1", "graph_sheet_id": "g1" })).expect("d1");
    session.execute("sheet.createData", json!({ "sheet_id": "d2", "title": "Second" })).expect("d2");

    // Rename across Data, Info, Graph.
    session.execute("sheet.rename", json!({ "sheet_id": "d1", "title": "Primary Plate" })).expect("rename d1");
    session.execute("sheet.rename", json!({ "sheet_id": "i1", "title": "Primary Notes" })).expect("rename i1");
    session.execute("sheet.rename", json!({ "sheet_id": "g1", "title": "Primary Plot" })).expect("rename g1");
    assert_eq!(session.document().data_sheet(&SheetId::new("d1")).map(|s| s.title.as_str()), Some("Primary Plate"));
    assert_eq!(session.document().info_sheet(&SheetId::new("i1")).map(|s| s.title.as_str()), Some("Primary Notes"));
    assert_eq!(session.document().graph_sheet(&SheetId::new("g1")).map(|s| s.title.as_str()), Some("Primary Plot"));

    // Select in family pane preserves family anchor while switching active sheet.
    session.execute("sheet.select", json!({ "sheet_id": "d1", "mode": "navigator" })).expect("nav select");
    assert_eq!(session.document().family_anchor_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));
    session.execute("sheet.select", json!({ "sheet_id": "g1", "mode": "family" })).expect("family select");
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("g1"));
    assert_eq!(session.document().family_anchor_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));

    // Reorder both by index and explicit section order.
    session.execute("sheet.reorder", json!({ "sheet_id": "d2", "index": 0 })).expect("move d2 first");
    assert_eq!(session.document().data_sheets.first().map(|s| s.id.as_str()), Some("d2"));
    session.execute("sheet.reorder", json!({ "section": "Data", "order": ["d1", "d2"] })).expect("explicit order");
    assert_eq!(session.document().data_sheets.first().map(|s| s.id.as_str()), Some("d1"));

    // Duplicate sheet preserves data and clears freeze.
    session.execute("table.setCell", json!({ "sheet_id": "d2", "row": 0, "col": 0, "value": 99.0 })).expect("set d2");
    session.execute("sheet.duplicate", json!({ "sheet_id": "d2", "new_sheet_id": "d2-copy", "title": "Second Copy" })).expect("dup d2");
    assert_eq!(data_cell(&session, "d2-copy", 0, 0, 0).and_then(|c| cell_num(&c)), Some(99.0));

    // Frozen sheet cannot be renamed or deleted (error rolls back cleanly).
    session.execute("sheet.freeze", json!({ "sheet_id": "d2-copy" })).expect("freeze copy");
    assert!(session.execute("sheet.rename", json!({ "sheet_id": "d2-copy", "title": "Blocked" })).is_err());
    assert!(session.execute("sheet.delete", json!({ "sheet_id": "d2-copy" })).is_err());
    session.execute("sheet.unfreeze", json!({ "sheet_id": "d2-copy" })).expect("unfreeze copy");
    session.execute("sheet.delete", json!({ "sheet_id": "d2-copy" })).expect("delete unfrozen copy");
    assert!(session.document().data_sheet(&SheetId::new("d2-copy")).is_none());

    // Delete cascade removes paired info/graph when d1 is deleted.
    session.execute("sheet.delete", json!({ "sheet_id": "d1", "cascade": true })).expect("cascade delete d1");
    assert!(session.document().data_sheet(&SheetId::new("d1")).is_none());
    assert!(session.document().info_sheet(&SheetId::new("i1")).is_none());
    assert!(session.document().graph_sheet(&SheetId::new("g1")).is_none());
}

#[test]
fn sheet_clone_pipeline_and_duplicate_family_for_fresh_assay_run() {
    // Construct a complete Data -> Info + Analysis + Styled Graph + Layout pipeline.
    let mut doc = ProjectDocument::new("Cloning Assay").expect("doc");
    let mut xy = curvebook_doc::XyTable::new(curvebook_doc::SubcolumnMode::Single, 1, 5).expect("xy");
    xy.data_sets[0].title = String::from("Inhibitor A");
    let mut data = curvebook_doc::DataSheet::new(SheetId::new("data-base"), "Run 1", TableArchetype::Xy(xy)).expect("ds");
    data.info_sheet_id = Some(SheetId::new("info-base"));
    let x_vals = [-8.0, -7.0, -6.0, -5.0, -4.0];
    let y_vals = [95.0, 82.0, 50.0, 18.0, 5.0];
    for i in 0..5 {
        data.set_x_or_time_cell(i, 0, Cell::number(x_vals[i])).expect("set x");
        data.set_cell(CellCoord::new(i, 0, 0), Cell::number(y_vals[i])).expect("set y");
    }
    doc.add_data_sheet(data).expect("add data");

    let mut info = curvebook_doc::InfoSheet::new_preseeded(SheetId::new("info-base"), "Run 1 Info").expect("info");
    info.set_constant("PlateBatch", ConstantValue::Text(String::from("B1")), None).expect("const");
    info.set_constant("MaxResp", ConstantValue::Number(100.0), None).expect("maxresp");
    doc.add_info_sheet(info).expect("add info");

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
    res.scalars.push(curvebook_doc::ResultScalarEntry { key: String::from("LogEC50"), data_set_index: Some(0), value: -6.0, units: None });
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
    let layout = LayoutSheet::new(SheetId::new("layout-base"), "Figure 1", layout_spec).expect("layout");
    doc.add_layout_sheet(layout).expect("add layout");

    let mut session = Session::from_document(doc);

    // 1) duplicateFamily keeps all Y measurements intact across cloned sheets.
    let dup = session
        .execute("sheet.duplicateFamily", json!({ "sheet_id": "data-base", "new_sheet_id": "data-dup", "title": "Run 1 Exact Copy" }))
        .expect("duplicate family");
    assert_eq!(data_cell(&session, "data-dup", 2, 0, 0).and_then(|c| cell_num(&c)), Some(50.0));
    assert_eq!(session.document().data_sheet(&SheetId::new("data-dup")).and_then(|d| d.x_or_time_cell(2, 0)).and_then(cell_num), Some(-6.0));
    assert_eq!(dup.payload.get("graph_sheet_ids").and_then(Value::as_array).map(Vec::len), Some(1));
    assert_eq!(dup.payload.get("layout_sheet_ids").and_then(Value::as_array).map(Vec::len), Some(1));

    // 2) clonePipeline clears Y values while keeping X concentrations, GDO styling, Result hooks, and Layout links.
    let clone = session
        .execute(
            "sheet.clonePipeline",
            json!({
                "sheet_id": "data-base",
                "new_sheet_id": "data-run2",
                "title": "Run 2 Fresh Assay",
                "keepXValues": true,
                "clearYValues": true
            }),
        )
        .expect("clonePipeline");
    let cloned_info = clone.payload.get("info_sheet_id").and_then(Value::as_str).expect("cloned info");
    let cloned_res =
        clone.payload.get("result_sheet_ids").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).expect("cloned res");
    let cloned_graph =
        clone.payload.get("graph_sheet_ids").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).expect("cloned graph");
    let cloned_layout =
        clone.payload.get("layout_sheet_ids").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).expect("cloned layout");

    // Y cleared, X preserved.
    assert_eq!(data_cell(&session, "data-run2", 2, 0, 0).and_then(|c| cell_num(&c)), None);
    assert_eq!(session.document().data_sheet(&SheetId::new("data-run2")).and_then(|d| d.x_or_time_cell(2, 0)).and_then(cell_num), Some(-6.0));
    // Cloned ResultSheet points to cloned data + cloned info and clears stale scalars to Dirty.
    let res_obj = session.document().result_sheet(&SheetId::new(cloned_res)).expect("res obj");
    assert_eq!(res_obj.analysis.source_sheet_id.as_str(), "data-run2");
    assert!(res_obj.scalars.is_empty());
    if let AnalysisKind::NonlinearRegression(nl_cloned) = &res_obj.analysis.kind {
        assert_eq!(
            nl_cloned.parameter_rules.first().map(|r| &r.scope),
            Some(&ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new(cloned_info)),
                constant_name: String::from("MaxResp"),
            }))
        );
    } else {
        panic!("expected NonlinearRegression");
    }
    // Cloned GraphSheet retains PlainBoxed styling while remapping primary/series IDs.
    let graph_obj = session.document().graph_sheet(&SheetId::new(cloned_graph)).expect("graph obj");
    assert_eq!(graph_obj.spec.frame_preset, FramePreset::PlainBoxed);
    assert_eq!(graph_obj.spec.primary_source_sheet_id.as_str(), "data-run2");
    assert_eq!(graph_obj.spec.series[0].source_sheet_id.as_str(), "data-run2");
    assert_eq!(graph_obj.spec.series[1].source_sheet_id.as_str(), cloned_res);
    // Cloned LayoutSheet remaps both LiveGraph and LiveTable items.
    let layout_obj = session.document().layout_sheet(&SheetId::new(cloned_layout)).expect("layout obj");
    assert!(matches!(&layout_obj.spec.items[0], LayoutItem::LiveGraph { graph_sheet_id, .. } if graph_sheet_id.as_str() == cloned_graph));
    assert!(matches!(&layout_obj.spec.items[1], LayoutItem::LiveTable { source_sheet_id, .. } if source_sheet_id.as_str() == cloned_res));

    // Populate fresh assay Y data on cloned run and recalculate.
    session
        .execute(
            "table.setCells",
            json!({
                "sheet_id": "data-run2",
                "edits": [
                    { "row": 0, "col": 0, "value": 98.0 },
                    { "row": 1, "col": 0, "value": 85.0 },
                    { "row": 2, "col": 0, "value": 52.0 },
                    { "row": 3, "col": 0, "value": 20.0 },
                    { "row": 4, "col": 0, "value": 4.0 }
                ]
            }),
        )
        .expect("fresh run data");
    session.execute("project.recalculate", json!({})).expect("recalculate cloned pipeline");
    let res_after = session.document().result_sheet(&SheetId::new(cloned_res)).expect("recomputed res");
    assert!(matches!(res_after.status, ResultStatus::UpToDate));
    assert_ne!(data_cell(&session, "data-base", 2, 0, 0).and_then(|c| cell_num(&c)), Some(52.0));
}

#[test]
fn table_cells_titles_exclude_clear_undo_and_err_rollback() {
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "xy-1", "archetype": "xy", "replicates": 2, "cols": 2, "rows": 4 })).expect("create xy");
    session
        .execute("sheet.createData", json!({ "sheet_id": "nest-1", "archetype": "nested", "cols": 2, "biological_units": 3, "rows": 3 }))
        .expect("create nested");

    session.execute("table.setRowTitles", json!({ "sheet_id": "xy-1", "titles": ["Baseline", "Low", "Mid", "High"] })).expect("row titles");
    session
        .execute("table.setGroupTitles", json!({ "sheet_id": "xy-1", "x_title": "Time (min)", "titles": ["Vehicle", "Drug"] }))
        .expect("group titles");
    assert!(session.execute("table.setSubcolumnTitles", json!({ "sheet_id": "xy-1", "col": 1, "titles": ["Rep A", "Rep B"] })).is_err());
    session
        .execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-1", "col": 0, "subcol": 2, "title": "Mouse 3" }))
        .expect("subcol title nested");
    if let TableArchetype::Nested(nt) = &session.document().data_sheet(&SheetId::new("nest-1")).expect("nest").archetype {
        assert_eq!(nt.groups[0].subcolumn_titles.get(2).map(String::as_str), Some("Mouse 3"));
    } else {
        panic!("expected Nested archetype");
    }

    session.execute("table.setXCells", json!({ "sheet_id": "xy-1", "values": [0.0, 5.0, 15.0, 30.0] })).expect("set x");
    session
        .execute(
            "table.setCells",
            json!({
                "sheet_id": "xy-1",
                "rows": [[1.0, 2.0], [3.0, 4.0]],
                "start_row": 0,
                "start_col": 0
            }),
        )
        .expect("set rows");
    session
        .execute(
            "table.excludeCells",
            json!({
                "sheet_id": "xy-1",
                "range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 1, "col": 0, "subcol": 0 } },
                "excluded": true
            }),
        )
        .expect("exclude range");
    assert!(data_cell(&session, "xy-1", 0, 0, 0).is_some_and(|c| c.excluded));
    assert!(data_cell(&session, "xy-1", 1, 0, 0).is_some_and(|c| c.excluded));

    // Undo restores non-excluded state, redo re-applies.
    assert!(session.undo().expect("undo"));
    assert!(!data_cell(&session, "xy-1", 1, 0, 0).is_some_and(|c| c.excluded));
    assert!(session.redo().expect("redo"));
    assert!(data_cell(&session, "xy-1", 1, 0, 0).is_some_and(|c| c.excluded));

    // Atomic rollback when mid-batch setCells hits an out-of-bounds subcolumn or missing sheet.
    let err = session.execute(
        "table.setCells",
        json!({
            "sheet_id": "xy-1",
            "edits": [
                { "row": 0, "col": 1, "subcol": 0, "value": 777.0 },
                { "row": 0, "col": 99, "subcol": 0, "value": 888.0 }
            ]
        }),
    );
    assert!(err.is_err());
    assert_ne!(data_cell(&session, "xy-1", 0, 1, 0).and_then(|c| cell_num(&c)), Some(777.0));

    // Clear single cell vs clear all including X.
    session.execute("table.clearCells", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "subcol": 0 })).expect("clear one");
    assert_eq!(data_cell(&session, "xy-1", 0, 0, 0).and_then(|c| cell_num(&c)), None);
    session.execute("table.clearCells", json!({ "sheet_id": "xy-1", "all": true, "clear_x": true })).expect("clear all");
    assert_eq!(data_cell(&session, "xy-1", 1, 0, 0).and_then(|c| cell_num(&c)), None);
    assert_eq!(session.document().data_sheet(&SheetId::new("xy-1")).and_then(|d| d.x_or_time_cell(1, 0)).and_then(cell_num), None);
}

#[test]
fn table_paste_decimal_normalization_slice_and_transpose() {
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "col-1", "archetype": "column", "cols": 3, "rows": 4 })).expect("create column");

    // European semicolon CSV with decimal commas, asterisk exclusion, row slice (rows 1..=2), and transpose.
    session
        .execute(
            "table.paste",
            json!({
                "sheet_id": "col-1",
                "text": "9,9;8,8\n1,5;2,75*\n3,25;4,5\n7,7;6,6",
                "delimiter": ";",
                "decimal_separator": ",",
                "slice": { "row_start": 1, "row_end": 2, "col_start": 0, "col_end": 1 },
                "transpose": true,
                "anchor": { "row": 0, "col": 0, "subcol": 0 }
            }),
        )
        .expect("paste european sliced transposed");

    // Transposed 2x2 slice: row 0 = [1.5, 3.25], row 1 = [2.75*, 4.5].
    assert_eq!(data_cell(&session, "col-1", 0, 0, 0).and_then(|c| cell_num(&c)), Some(1.5));
    assert_eq!(data_cell(&session, "col-1", 0, 1, 0).and_then(|c| cell_num(&c)), Some(3.25));
    let excl = data_cell(&session, "col-1", 1, 0, 0).expect("transposed excluded");
    assert_eq!(cell_num(&excl), Some(2.75));
    assert!(excl.excluded);
    assert_eq!(data_cell(&session, "col-1", 1, 1, 0).and_then(|c| cell_num(&c)), Some(4.5));
}

#[test]
fn table_paste_link_reactive_transpose_freeze_unfreeze_and_cycle_guard() {
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "grp-1", "archetype": "grouped", "replicates": 3, "cols": 1, "rows": 2 })).expect("grp");
    session.execute("sheet.createData", json!({ "sheet_id": "col-1", "archetype": "column", "cols": 2, "rows": 4, "auto_info": true })).expect("col");

    session.execute("table.setRowTitles", json!({ "sheet_id": "grp-1", "row": 0, "title": "Stimulated" })).expect("row title");
    session
        .execute(
            "table.setCells",
            json!({
                "sheet_id": "grp-1",
                "edits": [
                    { "row": 0, "col": 0, "subcol": 0, "value": 10.0 },
                    { "row": 0, "col": 0, "subcol": 1, "value": 12.0 },
                    { "row": 0, "col": 0, "subcol": 2, "value": 14.0 }
                ]
            }),
        )
        .expect("grp cells");

    // Link row 0's (1x3) replicates transposed into col-1 vertical column 0; propagates "Stimulated" header.
    session
        .execute(
            "table.pasteLink",
            json!({
                "targetSheetId": "col-1",
                "source_sheet_id": "grp-1",
                "range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 0, "col": 0, "subcol": 2 } },
                "anchor": { "row": 0, "col": 0, "subcol": 0 },
                "transpose": true
            }),
        )
        .expect("pasteLink");

    assert_eq!(data_cell(&session, "col-1", 0, 0, 0).and_then(|c| cell_num(&c)), Some(10.0));
    assert_eq!(data_cell(&session, "col-1", 1, 0, 0).and_then(|c| cell_num(&c)), Some(12.0));
    assert_eq!(data_cell(&session, "col-1", 2, 0, 0).and_then(|c| cell_num(&c)), Some(14.0));
    if let TableArchetype::Column(t) = &session.document().data_sheet(&SheetId::new("col-1")).expect("col").archetype {
        assert_eq!(t.data_sets[0].title, "Stimulated");
    } else {
        panic!("expected Column archetype");
    }

    // Upstream mutation reactively updates linked target cell.
    session.execute("table.setCell", json!({ "sheet_id": "grp-1", "row": 0, "col": 0, "subcol": 1, "value": 25.0 })).expect("edit upstream");
    assert_eq!(data_cell(&session, "col-1", 1, 0, 0).and_then(|c| cell_num(&c)), Some(25.0));

    // Freezing col-1 blocks direct edits AND preserves frozen snapshot when upstream changes; unfreeze catches up.
    session.execute("sheet.freeze", json!({ "sheet_id": "col-1" })).expect("freeze col");
    assert!(session.execute("table.setCell", json!({ "sheet_id": "col-1", "row": 0, "col": 1, "value": 5.0 })).is_err());
    session.execute("table.setCell", json!({ "sheet_id": "grp-1", "row": 0, "col": 0, "subcol": 2, "value": 99.0 })).expect("upstream while frozen");
    assert_eq!(data_cell(&session, "col-1", 2, 0, 0).and_then(|c| cell_num(&c)), Some(14.0));

    session.execute("sheet.unfreeze", json!({ "sheet_id": "col-1" })).expect("unfreeze col");
    assert_eq!(data_cell(&session, "col-1", 2, 0, 0).and_then(|c| cell_num(&c)), Some(99.0));

    // Backwards pasteLink creating cycle col-1 -> grp-1 must fail cleanly without corrupting grp-1.
    let cycle_err = session.execute(
        "table.pasteLink",
        json!({
            "targetSheetId": "grp-1",
            "source_sheet_id": "col-1",
            "range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 1, "col": 0, "subcol": 0 } },
            "anchor": { "row": 1, "col": 0, "subcol": 0 }
        }),
    );
    assert!(cycle_err.is_err());
    assert!(session.document().data_sheet(&SheetId::new("grp-1")).expect("grp").linked_regions.is_empty());

    // Direct dispatch cycle check also cleans up outside Session rollback wrapper.
    let mut raw_session = Session::from_document(session.document().clone());
    let spec = cmd::registry().get("table.pasteLink").expect("table.pasteLink registered");
    let cancelled = AtomicBool::new(false);
    let mut ctx = cmd::Ctx { session: &mut raw_session, caller: Caller::Ui, cancelled: &cancelled };
    let direct_err = (spec.run)(
        &mut ctx,
        json!({
            "target_sheet_id": "grp-1",
            "source_sheet_id": "col-1",
            "range": { "start": { "row": 0, "col": 0, "subcol": 0 }, "end": { "row": 1, "col": 0, "subcol": 0 } },
            "anchor": { "row": 1, "col": 0, "subcol": 0 }
        }),
    );
    assert!(direct_err.is_err());
    assert!(raw_session.document().data_sheet(&SheetId::new("grp-1")).expect("grp").linked_regions.is_empty());
}

#[test]
fn table_csv_prism_pzfx_imports_and_export() {
    let csv_path = unique_temp_path("export", "csv");
    let prism_path = unique_temp_path("imp", "prism");
    let mut session = Session::new();

    session
        .execute(
            "table.importCsv",
            json!({
                "sheet_id": "csv-1",
                "title": "Calib Curve",
                "archetype": "xy",
                "csv": "X,Std\n0.1,5.2\n1.0,25.4\n10.0,88.1\n"
            }),
        )
        .expect("importCsv");
    assert_eq!(data_cell(&session, "csv-1", 1, 0, 0).and_then(|c| cell_num(&c)), Some(25.4));

    let export = session.execute("table.exportCsv", json!({ "sheet_id": "csv-1", "path": csv_path.to_str() })).expect("exportCsv");
    assert!(!export.mutated);
    let csv_out = export.payload.get("csv").and_then(Value::as_str).expect("csv string");
    assert!(csv_out.contains("25.4"));

    // Save archive bytes and import back via table.importPrism.
    session.execute("project.save", json!({ "path": prism_path.to_str() })).expect("save prism bytes");
    let mut prism_sess = Session::new();
    prism_sess.execute("table.importPrism", json!({ "path": prism_path.to_str() })).expect("importPrism");
    assert!(prism_sess.document().data_sheet(&SheetId::new("csv-1")).is_some());

    // Import clean-room XML .pzfx string via table.importPzfx.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <Info Sequence="info-xml">
    <Title>Metadata</Title>
    <Constant><Name>Conc_uM</Name><Value>2.5</Value></Constant>
    <Notes>Clean XML fixture</Notes>
  </Info>
  <Table Sequence="tbl-xml" TableType="OneWay">
    <Title>Cytokine</Title>
    <YColumn Width="80" Subcolumns="1"><Title>Ctrl</Title><Subcolumn><d>11.0</d><d>13.0</d></Subcolumn></YColumn>
    <YColumn Width="80" Subcolumns="1"><Title>Treated</Title><Subcolumn><d>21.0</d><d>24.0</d></Subcolumn></YColumn>
  </Table>
</PrismFile>"#;

    let mut pzfx_sess = Session::new();
    pzfx_sess.execute("table.importPzfx", json!({ "xml": xml })).expect("importPzfx");
    assert!(!pzfx_sess.document().data_sheets.is_empty());

    let _ = std::fs::remove_file(csv_path);
    let _ = std::fs::remove_file(prism_path);
}

#[test]
fn table_multiple_variables_and_simulate_xy_commands() {
    let mut session = Session::new();
    session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "mv-1",
                "archetype": "multiple_variables",
                "cols": 2,
                "rows": 3,
                "variables": [
                    { "title": "Arm", "role": "Categorical" },
                    { "title": "Dose", "role": "Continuous" }
                ]
            }),
        )
        .expect("create mv");

    session
        .execute(
            "table.setCells",
            json!({
                "sheet_id": "mv-1",
                "edits": [
                    { "row": 0, "col": 0, "value": "veh" },
                    { "row": 1, "col": 0, "value": "low" },
                    { "row": 2, "col": 0, "value": "high" },
                    { "row": 0, "col": 1, "value": 1.0 },
                    { "row": 1, "col": 1, "value": 10.0 },
                    { "row": 2, "col": 1, "value": 100.0 }
                ]
            }),
        )
        .expect("populate mv");

    let recode = session
        .execute("table.recodeLevels", json!({ "sheet_id": "mv-1", "column": "Arm", "mappings": { "veh": "Vehicle", "high": "High" } }))
        .expect("recode");
    assert_eq!(recode.payload.get("recoded_count").and_then(Value::as_u64), Some(2));
    assert_eq!(data_cell(&session, "mv-1", 0, 0, 0).map(|c| c.value), Some(CellValue::Text(String::from("Vehicle"))));

    session
        .execute(
            "table.setCategoricalOrder",
            json!({ "sheet_id": "mv-1", "column": 0, "order": "explicit", "levels": ["Vehicle", "low", "High"], "reversed": true }),
        )
        .expect("cat order");
    if let TableArchetype::MultipleVariables(mv) = &session.document().data_sheet(&SheetId::new("mv-1")).expect("mv").archetype {
        assert!(mv.variables[0].level_order.reversed);
    } else {
        panic!("expected MultipleVariables");
    }

    session.execute("table.setVariableRole", json!({ "sheet_id": "mv-1", "column": "Dose", "role": "Identifier" })).expect("lock role");
    if let TableArchetype::MultipleVariables(mv) = &session.document().data_sheet(&SheetId::new("mv-1")).expect("mv").archetype {
        assert_eq!(mv.variables[1].role, VariableRole::Label);
        assert!(!mv.variables[1].role_inferred);
    } else {
        panic!("expected MultipleVariables");
    }

    session
        .execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "title": "LogDose", "formula": "= log10([Dose])" }))
        .expect("calculated variable");
    let log_val = data_cell(&session, "mv-1", 2, 2, 0).and_then(|c| cell_num(&c)).expect("computed log10(100)");
    assert!((log_val - 2.0).abs() < 1e-9);

    // table.simulateXy deterministically populates simulated replicates.
    session
        .execute(
            "table.simulateXy",
            json!({
                "sheet_id": "sim-1",
                "title": "Simulated Curve",
                "seed": 42,
                "rows": 6,
                "replicates": 2,
                "x_min": -9.0,
                "x_max": -4.0,
                "equation_id": "hill_4pl",
                "parameters": [0.0, 100.0, -6.5, 1.0],
                "noise_kind": "gaussian",
                "noise_sd": 0.5
            }),
        )
        .expect("simulateXy");
    let sim = session.document().data_sheet(&SheetId::new("sim-1")).expect("sim sheet");
    assert_eq!(sim.archetype.row_count(), 6);
    assert!(sim.cell(CellCoord::new(0, 0, 1)).and_then(cell_num).is_some());
}

#[test]
fn info_notes_constants_and_hooked_recalculation() {
    let mut doc = ProjectDocument::new("Info Hooks").expect("doc");
    let info = curvebook_doc::InfoSheet::new_preseeded(SheetId::new("info-1"), "Experiment Info").expect("info");
    doc.add_info_sheet(info).expect("add info");

    let mut mv = curvebook_doc::MultipleVariablesTable::new(2, 2).expect("mv");
    mv.variables[0].title = String::from("Raw");
    mv.variables[1].title = String::from("Scaled");
    mv.variables[1].set_calculated_formula("= [Raw] * Info(\"ScaleFactor\")");
    let mut ds = curvebook_doc::DataSheet::new(SheetId::new("mv-1"), "Assay", TableArchetype::MultipleVariables(mv)).expect("ds");
    ds.info_sheet_id = Some(SheetId::new("info-1"));
    ds.set_cell(CellCoord::new(0, 0, 0), Cell::number(5.0)).expect("set 5");
    doc.add_data_sheet(ds).expect("add ds");

    let mut session = Session::from_document(doc);

    session.execute("info.setNote", json!({ "sheet_id": "info-1", "note": "Line 1" })).expect("note 1");
    session.execute("info.setNote", json!({ "sheet_id": "mv-1", "note": "Line 2", "append": true })).expect("note append via paired data id");
    assert_eq!(session.document().info_sheet(&SheetId::new("info-1")).map(|i| i.notes_markdown.as_str()), Some("Line 1\nLine 2"));

    session.execute("info.setConstant", json!({ "sheet_id": "info-1", "name": "ScaleFactor", "value": 3.0, "units": "x" })).expect("set constant");
    assert_eq!(data_cell(&session, "mv-1", 0, 1, 0).and_then(|c| cell_num(&c)), Some(15.0));

    // Removing missing constant fails unless if_exists: true; updating triggers recalculation.
    assert!(session.execute("info.removeConstant", json!({ "sheet_id": "info-1", "name": "Nope" })).is_err());
    session.execute("info.removeConstant", json!({ "sheet_id": "info-1", "name": "Nope", "if_exists": true })).expect("if_exists remove");
    session
        .execute("info.setConstant", json!({ "sheet_id": "mv-1", "name": "ScaleFactor", "value": 4.0 }))
        .expect("update constant via paired data id");
    assert_eq!(data_cell(&session, "mv-1", 0, 1, 0).and_then(|c| cell_num(&c)), Some(20.0));
}

#[test]
fn safety_guards_oversized_inputs_bad_base64_and_catalog_aliases() {
    let mut session = Session::new();
    session.execute("sheet.createData", json!({ "sheet_id": "d1", "archetype": "column", "cols": 2, "rows": 3 })).expect("create d1");
    session.execute("sheet.createData", json!({ "sheet_id": "d2", "archetype": "column", "cols": 2, "rows": 4 })).expect("create d2");
    session
        .execute(
            "table.setCells",
            json!({ "sheet_id": "d1", "cells": [{ "row": 0, "col": 0, "value": 11.0 }, { "row": 1, "col": 0, "value": 22.0 }] }),
        )
        .expect("cells");

    // 1. Oversized row in table.setRowTitles must fail cleanly without OOM/panic.
    assert!(session.execute("table.setRowTitles", json!({ "sheet_id": "d1", "row": u64::MAX, "title": "Huge" })).is_err());
    assert!(session.execute("table.setRowTitles", json!({ "sheet_id": "d1", "start_row": u64::MAX, "titles": ["A", "B"] })).is_err());
    assert!(session.execute("table.setRowTitles", json!({ "sheet_id": "d1", "start_row": 2, "titles": ["Ok", "PastEnd"] })).is_err());

    // 2. Oversized range in excludeCells & clearCells must reject bounds before iterating.
    assert!(
        session
            .execute(
                "table.excludeCells",
                json!({ "sheet_id": "d1", "range": { "start": { "row": 0, "col": 0 }, "end": { "row": u64::MAX, "col": 0 } } }),
            )
            .is_err()
    );
    assert!(
        session
            .execute("table.clearCells", json!({ "sheet_id": "d1", "range": { "start": { "row": 0, "col": 0 }, "end": { "row": 100, "col": 0 } } }))
            .is_err()
    );

    // 3. Malformed base64 padding sequences must return Err across archive open commands.
    for bad_b64 in ["AA=A", "====", "AA==AAAA"] {
        assert!(session.execute("project.open", json!({ "bytes_base64": bad_b64 })).is_err(), "accepted bad base64 `{bad_b64}`");
        assert!(
            session.execute("project.instantiateTemplate", json!({ "bytes_base64": bad_b64 })).is_err(),
            "accepted bad template base64 `{bad_b64}`"
        );
    }

    // 4. Catalog params_doc alias coverage: include_bytes, target_row/target_col, prefix + clear_data, top-level clear box, mapping.
    let saved = session.execute("project.save", json!({ "include_bytes": true })).expect("save include_bytes");
    assert!(saved.payload.get("bytes_base64").and_then(Value::as_str).is_some());
    let tpl_saved = session.execute("project.saveTemplate", json!({ "include_bytes": true })).expect("saveTemplate include_bytes");
    assert!(tpl_saved.payload.get("bytes_base64").and_then(Value::as_str).is_some());

    session
        .execute(
            "table.pasteLink",
            json!({
                "target_sheet_id": "d2",
                "source_sheet_id": "d1",
                "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 1, "col": 0 } },
                "target_row": 2,
                "target_col": 1
            }),
        )
        .expect("pasteLink target_row/target_col");
    assert_eq!(data_cell(&session, "d2", 2, 1, 0).and_then(|c| cell_num(&c)), Some(11.0));
    assert_eq!(data_cell(&session, "d2", 3, 1, 0).and_then(|c| cell_num(&c)), Some(22.0));

    let dup = session
        .execute("sheet.duplicateFamily", json!({ "sheet_id": "d1", "prefix": "fam", "clear_data": true }))
        .expect("dup family prefix+clear_data");
    let fam_id = dup.payload.get("sheet_id").and_then(Value::as_str).expect("fam sheet id");
    assert!(fam_id.starts_with("fam"));
    assert_eq!(data_cell(&session, fam_id, 0, 0, 0).and_then(|c| cell_num(&c)), None);

    session
        .execute("table.clearCells", json!({ "sheet_id": "d1", "row_start": 0, "row_end": 1, "col_start": 0, "col_end": 0 }))
        .expect("clearCells row_start/col_start");
    assert_eq!(data_cell(&session, "d1", 0, 0, 0).and_then(|c| cell_num(&c)), None);
    assert_eq!(data_cell(&session, "d1", 1, 0, 0).and_then(|c| cell_num(&c)), None);

    session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "mv-alias",
                "archetype": "multiple_variables",
                "cols": 1,
                "rows": 2,
                "variables": [{ "title": "Grp", "role": "Categorical" }]
            }),
        )
        .expect("create mv-alias");
    session.execute("table.setCells", json!({ "sheet_id": "mv-alias", "cells": [{ "row": 0, "col": 0, "value": "old" }] })).expect("mv cells");
    session.execute("table.recodeLevels", json!({ "sheet_id": "mv-alias", "col": 0, "mapping": { "old": "new" } })).expect("recode mapping alias");
    assert_eq!(data_cell(&session, "mv-alias", 0, 0, 0).map(|c| c.value), Some(CellValue::Text(String::from("new"))));
}
