#![forbid(unsafe_code)]

use std::error::Error;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, GraphAnnotation, HookSource, InfoSheet, LayoutItem,
    LinkSource, MultipleVariablesTable, NonlinearFitSpec, NormalizeAnchor, ParameterRule, ParameterScope, PowerAnalysisSpec, ProjectDocument,
    ResultCell, ResultSheet, ResultStatus, SheetId, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec, VariableRole,
    XyTable,
};
use curvebook_engine::{EngineError, Session};
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

fn find_tab_metric(session: &Session, res_id: &str, metric_label: &str) -> Option<f64> {
    let rs = session.document().result_sheet(&SheetId::new(res_id))?;
    let tab = rs.tabs.first()?;
    let row_idx = tab.row_labels.iter().position(|lbl| lbl.eq_ignore_ascii_case(metric_label))?;
    tab.rows.get(row_idx)?.iter().find_map(|cell| match cell {
        ResultCell::Number { value, .. } => Some(*value),
        _ => None,
    })
}

#[test]
fn audit_57_import_plate_map_rejects_ragged_and_wrong_row_count_grids() -> TestResult {
    let mut session = Session::new();
    let roles96: Vec<Value> = (0..96)
        .map(|idx| {
            if idx < 4 {
                json!("Blank")
            } else {
                json!({ "role": "treatment", "group_name": format!("G{}", idx / 12), "dilution_x": (idx % 12 + 1) as f64, "replicate": 1 })
            }
        })
        .collect();

    // Ragged 8-row grid that still sums to 96 elements (row 3 has 11, row 7 has 13) -> BadInput.
    let mut ragged: Vec<Vec<Option<f64>>> = vec![vec![Some(1.0); 12]; 8];
    ragged[3].pop();
    ragged[7].push(Some(2.0));
    let err_ragged = err_cmd(&mut session, "table.importPlateMap", json!({ "geometry": 96, "roles": roles96, "values": ragged }))?;
    assert_eq!(err_ragged.code, ErrorCode::BadInput);
    assert!(err_ragged.message().contains("row 3"));

    // Wrong row count (6 rows * 16 cols = 96 elements != 8x12) -> BadInput.
    let wrong_rows: Vec<Vec<Option<f64>>> = vec![vec![Some(1.0); 16]; 6];
    let err_rows = err_cmd(&mut session, "table.importPlateMap", json!({ "geometry": 96, "roles": roles96, "values": wrong_rows }))?;
    assert_eq!(err_rows.code, ErrorCode::BadInput);

    // Wrong flat length (95 != 96) -> BadInput.
    let err_flat = err_cmd(&mut session, "table.importPlateMap", json!({ "geometry": 96, "roles": roles96, "values": vec![Some(1.0); 95] }))?;
    assert_eq!(err_flat.code, ErrorCode::BadInput);

    // Rectangular 8x12 succeeds cleanly.
    let valid_grid: Vec<Vec<Option<f64>>> = vec![vec![Some(5.0); 12]; 8];
    let ok = ok_cmd(&mut session, "table.importPlateMap", json!({ "geometry": 96, "roles": roles96, "values": valid_grid }))?;
    assert!(ok.get("sheet_id").and_then(Value::as_str).is_some());
    Ok(())
}

#[test]
fn audit_71_recalculate_and_import_csv_surface_failed_result_sheets() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 71")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    for (r, (x, y)) in [(1.0, 2.0), (2.0, 4.0), (3.0, 6.0), (4.0, 8.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("d1", "Dose", TableArchetype::Xy(xy))?)?;
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "fit_1", "source_sheet_id": "d1", "kind": "linear", "title": "Linear Fit" }))?;

    // Replacing Y values with blanks makes fit_1 fail, and table.importCsv surfaces the failure cleanly.
    let imp = session.execute("table.importCsv", json!({ "sheet_id": "d1", "archetype": "xy", "replace": true, "csv": "X,Y\n1,\n2,\n3,\n4,\n" }))?;
    assert_eq!(imp.payload["failed_count"], json!(1));
    assert_eq!(imp.payload["failed_result_sheets"][0]["sheet_id"], json!("fit_1"));
    assert_eq!(imp.payload["failed_result_sheets"][0]["title"], json!("Linear Fit"));
    assert!(imp.message.contains("fit_1"));

    // project.recalculate surfaces failed_count and failed_result_sheets as well.
    let recalc = session.execute("project.recalculate", json!({}))?;
    assert_eq!(recalc.payload["failed_count"], json!(1));
    assert_eq!(recalc.payload["failed_result_sheets"][0]["sheet_id"], json!("fit_1"));
    assert!(recalc.message.contains("fit_1"));

    // Session report helpers also surface failed result sheets accurately without breaking recalculate_all().
    let report = session.recalculate_all_with_report()?;
    assert_eq!(report.failed_result_sheets, vec![SheetId::new("fit_1")]);
    let failed = session.failed_result_sheets();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].sheet_id, SheetId::new("fit_1"));
    session.recalculate_all()?;
    Ok(())
}

#[test]
fn audit_74_convert_archetype_remaps_dependents_and_handles_links() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 74")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    xy.x_title = String::from("Conc");
    xy.data_sets[0].title = String::from("10");
    for (r, (x, y)) in [(1.0, 100.0), (2.0, 110.0), (3.0, 120.0), (4.0, 130.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("xy", "XY Source", TableArchetype::Xy(xy))?)?;

    let mut upstream = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 4)?;
    for (r, y) in [100.0, 110.0, 120.0, 130.0].into_iter().enumerate() {
        upstream.data_sets[0].cells[r][0] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("up", "Upstream", TableArchetype::Column(upstream))?)?;
    doc.add_data_sheet(DataSheet::new(
        "copy",
        "Linked Copy",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 4)?),
    )?)?;

    // ColumnTitleValue hook referencing xy col 0 ("10").
    let mut other_xy = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    for (r, x) in [1.0, 2.0, 3.0, 4.0].into_iter().enumerate() {
        other_xy.x_cells[r][0] = Cell::number(x);
        other_xy.data_sets[0].cells[r][0] = Cell::number(20.0 * x);
    }
    doc.add_data_sheet(DataSheet::new("xy2", "Downstream XY", TableArchetype::Xy(other_xy))?)?;
    let mut nl_hook = NonlinearFitSpec::new("custom");
    nl_hook.custom_dsl = Some(String::from("Y = K * A * X"));
    nl_hook.parameter_rules = vec![
        ParameterRule { name: String::from("K"), scope: ParameterScope::Local, initial_override: Some(1.0), lower_bound: None, upper_bound: None },
        ParameterRule {
            name: String::from("A"),
            scope: ParameterScope::HookedConstant(HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("xy"), data_set_index: 0 }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-hook", "xy2", AnalysisKind::NonlinearRegression(nl_hook))?)?)?;

    let mut session = Session::from_document(doc);
    // Outgoing link from xy col 0 -> copy col 0; incoming link from up col 0 -> xy col 0.
    ok_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "xy", "target_sheet_id": "copy", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 3, "col": 0 } }, "start_row": 0, "start_col": 0 }),
    )?;
    ok_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "up", "target_sheet_id": "xy", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 3, "col": 0 } }, "start_row": 0, "start_col": 0, "link_id": "in-up" }),
    )?;
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "desc-explicit", "source_sheet_id": "xy", "kind": "descriptive_stats", "selected_data_sets": [0] }),
    )?;
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "desc-all", "source_sheet_id": "xy", "kind": "descriptive_stats" }))?;
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy", "kind": "xy" }))?;

    // Convert xy -> Column without allow_lossy (incoming link is mappable and preserved!).
    ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "xy", "target_kind": "Column" }))?;

    let conv_ds = session.document().data_sheet(&SheetId::new("xy")).ok_or("missing xy")?;
    assert_eq!(conv_ds.linked_regions.len(), 1);
    assert_eq!(conv_ds.linked_regions[0].target_range.start.col, 1);

    for rid in ["desc-explicit", "desc-all"] {
        let rs = session.document().result_sheet(&SheetId::new(rid)).ok_or("missing desc")?;
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(rs.analysis.selected_data_sets, vec![1]);
        let med = find_tab_metric(&session, rid, "Median").ok_or("missing Median")?;
        assert!((med - 115.0).abs() < 1e-9, "{rid} expected Median 115 (Y), got {med}");
    }

    let graph = session.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
    assert_eq!(graph.spec.series[0].data_set_index, 1);

    let hook_rs = session.document().result_sheet(&SheetId::new("res-hook")).ok_or("missing res-hook")?;
    assert_eq!(hook_rs.status, ResultStatus::UpToDate);
    assert!((hook_rs.lookup_scalar("K", Some(0)).ok_or("missing K")? - 2.0).abs() < 1e-6);

    // Mutate `up` -> propagates through preserved incoming link at col 1 -> outgoing link to `copy`!
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "up", "row": 0, "col": 0, "value": 500.0 }))?;
    let copy_ds = session.document().data_sheet(&SheetId::new("copy")).ok_or("missing copy")?;
    assert_eq!(copy_ds.archetype.data_cell(curvebook_doc::CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(500.0));

    // Now preview & commit converting a 2-col Column table whose analysis explicitly targets col 0 & incoming link targets col 0 -> XY:
    let mut col2 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    for r in 0..3 {
        col2.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        col2.data_sets[1].cells[r][0] = Cell::number((r + 10) as f64);
    }
    let mut col_up = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3)?;
    for r in 0..3 {
        col_up.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let mut doc2 = ProjectDocument::new("Unmap")?;
    doc2.add_data_sheet(DataSheet::new("up2", "Up2", TableArchetype::Column(col_up))?)?;
    doc2.add_data_sheet(DataSheet::new("c2", "Two Cols", TableArchetype::Column(col2))?)?;
    let mut s2 = Session::from_document(doc2);
    ok_cmd(
        &mut s2,
        "table.pasteLink",
        json!({ "source_sheet_id": "up2", "target_sheet_id": "c2", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 2, "col": 0 } }, "start_row": 0, "start_col": 0, "link_id": "in-col0" }),
    )?;
    ok_cmd(&mut s2, "stats.run", json!({ "sheet_id": "d0", "source_sheet_id": "c2", "kind": "descriptive_stats", "selected_data_sets": [0] }))?;
    let preview_unmap = ok_cmd(&mut s2, "table.convertArchetype", json!({ "sheet_id": "c2", "target_kind": "XY", "preview_only": true }))?;
    assert_eq!(preview_unmap["preview"]["lossy"], json!(true));
    let warnings = preview_unmap["preview"]["warnings"].as_array().ok_or("expected preview warnings")?;
    assert!(warnings.len() >= 2, "expected warnings for detached incoming link and failed analysis: {warnings:?}");
    let refused = err_cmd(&mut s2, "table.convertArchetype", json!({ "sheet_id": "c2", "target_kind": "XY" }))?;
    assert_eq!(refused.code, ErrorCode::BadInput);
    ok_cmd(&mut s2, "table.convertArchetype", json!({ "sheet_id": "c2", "target_kind": "XY", "allow_lossy": true }))?;
    let c2_after = s2.document().data_sheet(&SheetId::new("c2")).ok_or("missing c2")?;
    assert!(c2_after.linked_regions.is_empty(), "unmappable col-0 incoming link must be detached");
    let d0_after = s2.document().result_sheet(&SheetId::new("d0")).ok_or("missing d0")?;
    assert!(matches!(d0_after.status, ResultStatus::Failed { .. }), "moved-to-X selection must fail, got {:?}", d0_after.status);
    Ok(())
}

#[test]
fn audit_75_undo_open_restores_matching_file_path() -> TestResult {
    let dir = std::env::temp_dir().join(format!("cb-audit-75-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let path_a = dir.join("study_a.curvebook");
    let path_b = dir.join("study_b.curvebook");

    let mut sess_a = Session::from_document(ProjectDocument::new("Project A")?);
    ok_cmd(&mut sess_a, "sheet.createData", json!({ "sheet_id": "sheet-a", "title": "Only A", "kind": "Column" }))?;
    ok_cmd(&mut sess_a, "project.save", json!({ "path": path_a.to_string_lossy() }))?;

    let mut sess_b = Session::from_document(ProjectDocument::new("Project B")?);
    ok_cmd(&mut sess_b, "sheet.createData", json!({ "sheet_id": "sheet-b", "title": "Only B", "kind": "Column" }))?;
    ok_cmd(&mut sess_b, "project.save", json!({ "path": path_b.to_string_lossy() }))?;
    let bytes_b_before = std::fs::read(&path_b)?;

    let mut session = Session::new();
    ok_cmd(&mut session, "project.open", json!({ "path": path_a.to_string_lossy() }))?;
    assert_eq!(session.file_path().map(|p| p.to_path_buf()), Some(path_a.clone()));

    ok_cmd(&mut session, "project.open", json!({ "path": path_b.to_string_lossy() }))?;
    assert_eq!(session.document().title, "Project B");
    assert_eq!(session.file_path().map(|p| p.to_path_buf()), Some(path_b.clone()));

    // Undoing project.open B restores A's document AND A's file path.
    assert!(session.undo()?);
    assert_eq!(session.document().title, "Project A");
    assert_eq!(session.file_path().map(|p| p.to_path_buf()), Some(path_a.clone()));

    // Saving to session.file_path() writes to A, leaving B untouched.
    let target_save = session.file_path().ok_or("expected restored file_path")?.to_path_buf();
    ok_cmd(&mut session, "project.save", json!({ "path": target_save.to_string_lossy() }))?;
    assert_eq!(std::fs::read(&path_b)?, bytes_b_before);

    // Redoing restores B's document and B's path again.
    assert!(session.redo()?);
    assert_eq!(session.document().title, "Project B");
    assert_eq!(session.file_path().map(|p| p.to_path_buf()), Some(path_b.clone()));

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn audit_76_duplicate_family_remaps_overlays_cld_hooks_anchors_and_power_info() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 76")?;
    let mut info = InfoSheet::new_preseeded("info-1", "Cohort Info")?;
    info.set_constant("NormAnchor", curvebook_doc::ConstantValue::Number(200.0), None)?;
    info.set_constant("Alpha", curvebook_doc::ConstantValue::Number(0.05), None)?;
    info.linked_data_sheets.push(SheetId::new("xy-1"));
    doc.add_info_sheet(info)?;

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    for (r, (x, y)) in [(1.0, 100.0), (2.0, 200.0), (3.0, 300.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let mut ds = DataSheet::new("xy-1", "Curve", TableArchetype::Xy(xy))?;
    ds.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(ds)?;

    let mut norm_spec = AnalysisSpec::new(
        "norm-1",
        "xy-1",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ColumnMin,
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: String::from("NormAnchor"),
            }),
            output_percent: true,
            per_data_set: false,
        }),
    )?;
    norm_spec.custom_title = Some(String::from("Norm"));
    doc.add_result_sheet(ResultSheet::new(norm_spec)?)?;

    let mut pow_spec = AnalysisSpec::new(
        "pow-1",
        "xy-1",
        AnalysisKind::PowerAnalysis(PowerAnalysisSpec {
            info_sheet_id: Some(SheetId::new("info-1")),
            target_power: Some(0.8),
            ..PowerAnalysisSpec::default()
        }),
    )?;
    pow_spec.custom_title = Some(String::from("Power"));
    doc.add_result_sheet(ResultSheet::new(pow_spec)?)?;

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "fit-1", "source_sheet_id": "xy-1", "kind": "linear" }))?;
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "desc-1", "source_sheet_id": "xy-1", "kind": "descriptive_stats" }))?;
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "kind": "xy" }))?;
    ok_cmd(&mut session, "graph.setRegressionOverlay", json!({ "sheet_id": "g-1", "result_sheet_id": "fit-1", "series_index": 0 }))?;
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g-1",
            "spec": {
                "title": "Anchor={{info:info-1:NormAnchor}}",
                "compact_letter_display": { "enabled": true, "result_sheet_id": "desc-1" }
            }
        }),
    )?;
    ok_cmd(
        &mut session,
        "graph.addAnnotation",
        json!({ "sheet_id": "g-1", "kind": "text", "text": "Slope={{scalar:fit-1:Slope:0}}", "x_pt": 10.0, "y_pt": 10.0 }),
    )?;
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay-1" }))?;
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-1", "bounds_pt": [10.0, 10.0, 100.0, 80.0] }),
    )?;
    ok_cmd(
        &mut session,
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "text", "markdown": "Val={{info:info-1:NormAnchor}}", "bounds_pt": [10.0, 95.0, 100.0, 20.0] }),
    )?;

    let cloned = ok_cmd(&mut session, "sheet.duplicateFamily", json!({ "sheet_id": "xy-1", "new_sheet_id": "xy-2" }))?;
    let cloned_info_id = cloned["info_sheet_id"].as_str().ok_or("missing cloned info")?;
    let cloned_graph_id = cloned["graph_sheet_ids"][0].as_str().ok_or("missing cloned graph")?;
    let cloned_layout_id = cloned["layout_sheet_ids"][0].as_str().ok_or("missing cloned layout")?;

    // Mutate cloned InfoSheet's NormAnchor from 200 -> 400 and verify cloned normalization updates while original stays unchanged.
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": cloned_info_id, "name": "NormAnchor", "value": 400.0 }))?;

    let cloned_norm = session
        .document()
        .result_sheets
        .iter()
        .find(|r| r.analysis.source_sheet_id == SheetId::new("xy-2") && matches!(r.analysis.kind, AnalysisKind::Transform(_)))
        .ok_or("missing cloned norm")?;
    let cloned_last_val = cloned_norm
        .analyzable_data
        .as_ref()
        .and_then(|d| d.archetype.data_cell(curvebook_doc::CellCoord::new(2, 0, 0)))
        .and_then(|c| c.active_number())
        .ok_or("expected numeric norm output")?;
    // With 0-anchor=100 (row 0) and 100-anchor=400, row 2 (300) normalizes to 200/300 * 100 = 66.6667% (instead of 200%).
    assert!((cloned_last_val - (200.0 / 300.0 * 100.0)).abs() < 1e-4, "got {cloned_last_val}");

    let cloned_pow = session
        .document()
        .result_sheets
        .iter()
        .find(|r| r.analysis.source_sheet_id == SheetId::new("xy-2") && matches!(r.analysis.kind, AnalysisKind::PowerAnalysis(_)))
        .ok_or("missing cloned pow")?;
    let AnalysisKind::PowerAnalysis(ref pow_spec_after) = cloned_pow.analysis.kind else {
        return Err("expected PowerAnalysis".into());
    };
    assert_eq!(pow_spec_after.info_sheet_id.as_ref().map(SheetId::as_str), Some(cloned_info_id));

    let cloned_gs = session.document().graph_sheet(&SheetId::new(cloned_graph_id)).ok_or("missing cloned graph")?;
    let ov_id = cloned_gs.spec.series[0].regression_overlay.as_ref().ok_or("missing cloned overlay")?.result_sheet_id.clone();
    assert_ne!(ov_id, SheetId::new("fit-1"));
    let cld_id = cloned_gs.spec.compact_letter_display.as_ref().ok_or("missing cloned CLD")?.result_sheet_id.clone();
    assert_ne!(cld_id, SheetId::new("desc-1"));
    assert!(cloned_gs.spec.title.contains(cloned_info_id), "title token must remap: {}", cloned_gs.spec.title);
    assert!(
        matches!(&cloned_gs.spec.annotations[0], GraphAnnotation::TextBox { markdown, .. } if markdown.contains(ov_id.as_str())),
        "textbox scalar token must remap"
    );

    let cloned_lay = session.document().layout_sheet(&SheetId::new(cloned_layout_id)).ok_or("missing cloned layout")?;
    assert!(
        cloned_lay.spec.items.iter().any(|it| matches!(it, LayoutItem::TextCallout { markdown, .. } if markdown.contains(cloned_info_id))),
        "layout TextCallout hook must remap to cloned InfoSheet"
    );
    Ok(())
}

#[test]
fn audit_95_randomization_and_row_sample_use_dedicated_info_sheets() -> TestResult {
    let mut session = Session::new();
    let r1 = ok_cmd(
        &mut session,
        "table.randomizeGroups",
        json!({ "sheet_id": "rand-1", "subject_count": 4, "groups": [{ "label": "Ctrl", "weight": 1 }, { "label": "Trt", "weight": 1 }], "seed": 111 }),
    )?;
    let r2 = ok_cmd(
        &mut session,
        "table.randomizeGroups",
        json!({ "sheet_id": "rand-2", "subject_count": 6, "groups": [{ "label": "Ctrl", "weight": 1 }, { "label": "Trt", "weight": 2 }], "seed": 222 }),
    )?;
    let info1_id = r1["info_sheet_id"].as_str().ok_or("missing info1")?;
    let info2_id = r2["info_sheet_id"].as_str().ok_or("missing info2")?;
    assert_ne!(info1_id, info2_id);
    let info1 = session.document().info_sheet(&SheetId::new(info1_id)).ok_or("info1")?;
    let info2 = session.document().info_sheet(&SheetId::new(info2_id)).ok_or("info2")?;
    assert_eq!(info1.get_constant("randomization.seed").map(|c| &c.value), Some(&curvebook_doc::ConstantValue::Text(String::from("111"))));
    assert_eq!(info2.get_constant("randomization.seed").map(|c| &c.value), Some(&curvebook_doc::ConstantValue::Text(String::from("222"))));

    // Create source MV table without paired info and take two row samples -> each gets its own dedicated Info sheet.
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "src-mv", "kind": "multiple_variables", "rows": 8, "cols": 1 }))?;
    for r in 0..8 {
        ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "src-mv", "row": r, "col": 0, "value": (r + 1) as f64 }))?;
    }
    let s1 = ok_cmd(&mut session, "table.randomSampleRows", json!({ "source_sheet_id": "src-mv", "sample_size": 3, "seed": 10 }))?;
    let s2 = ok_cmd(&mut session, "table.randomSampleRows", json!({ "source_sheet_id": "src-mv", "sample_size": 5, "seed": 20 }))?;
    let si1 = s1["info_sheet_id"].as_str().ok_or("missing sample info 1")?;
    let si2 = s2["info_sheet_id"].as_str().ok_or("missing sample info 2")?;
    assert_ne!(si1, si2);
    assert_ne!(si1, info1_id);
    let si1_sheet = session.document().info_sheet(&SheetId::new(si1)).ok_or("si1")?;
    let si2_sheet = session.document().info_sheet(&SheetId::new(si2)).ok_or("si2")?;
    assert_eq!(si1_sheet.get_constant("sample.seed").map(|c| &c.value), Some(&curvebook_doc::ConstantValue::Text(String::from("10"))));
    assert_eq!(si2_sheet.get_constant("sample.seed").map(|c| &c.value), Some(&curvebook_doc::ConstantValue::Text(String::from("20"))));

    // Two row samples from a source WITH a paired Info sheet: second sample must create a dedicated Info sheet instead of overwriting first sample's provenance.
    ok_cmd(
        &mut session,
        "sheet.createData",
        json!({ "sheet_id": "src-paired", "kind": "multiple_variables", "rows": 8, "cols": 1, "auto_info": true }),
    )?;
    for r in 0..8 {
        ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "src-paired", "row": r, "col": 0, "value": (r + 1) as f64 }))?;
    }
    let p1 = ok_cmd(&mut session, "table.randomSampleRows", json!({ "source_sheet_id": "src-paired", "sample_size": 3, "seed": 101 }))?;
    let p2 = ok_cmd(&mut session, "table.randomSampleRows", json!({ "source_sheet_id": "src-paired", "sample_size": 4, "seed": 202 }))?;
    let pi1 = p1["info_sheet_id"].as_str().ok_or("missing pi1")?;
    let pi2 = p2["info_sheet_id"].as_str().ok_or("missing pi2")?;
    assert_ne!(pi1, pi2);
    assert_eq!(
        session.document().info_sheet(&SheetId::new(pi1)).ok_or("pi1")?.get_constant("sample.seed").map(|c| &c.value),
        Some(&curvebook_doc::ConstantValue::Text(String::from("101")))
    );
    assert_eq!(
        session.document().info_sheet(&SheetId::new(pi2)).ok_or("pi2")?.get_constant("sample.seed").map(|c| &c.value),
        Some(&curvebook_doc::ConstantValue::Text(String::from("202")))
    );
    Ok(())
}

#[test]
fn audit_96_recode_levels_remaps_glm_and_cox_reference_levels() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 96")?;
    let mut mv = MultipleVariablesTable::new(4, 10)?;
    mv.variables[0].title = String::from("Outcome");
    mv.variables[1].title = String::from("Arm");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Time");
    mv.variables[3].title = String::from("Status");
    let rows = [
        (2.0, "ctrl", 5.0, 1.0),
        (2.2, "ctrl", 7.0, 0.0),
        (1.8, "ctrl", 9.0, 1.0),
        (6.0, "drugA", 4.0, 1.0),
        (6.1, "drugA", 6.0, 1.0),
        (5.9, "drugA", 8.0, 0.0),
        (10.0, "drugB", 2.0, 1.0),
        (9.9, "drugB", 3.0, 1.0),
        (10.1, "drugB", 5.0, 1.0),
        (2.0, "ctrl", 11.0, 0.0),
    ];
    for (r, (y, arm, t, st)) in rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(y);
        mv.variables[1].cells[r] = Cell::text(arm);
        mv.variables[2].cells[r] = Cell::number(t);
        mv.variables[3].cells[r] = Cell::number(st);
    }
    doc.add_data_sheet(DataSheet::new("mv", "Cohort", TableArchetype::MultipleVariables(mv))?)?;
    let mut session = Session::from_document(doc);

    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "glm-1",
            "source_sheet_id": "mv",
            "kind": "glm",
            "family": "linear",
            "response_column": 0,
            "predictor_columns": [1],
            "reference_levels": [[1, "ctrl"]]
        }),
    )?;
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "cox-1",
            "source_sheet_id": "mv",
            "kind": "cox_ph",
            "time_column": 2,
            "status_column": 3,
            "predictor_columns": [1],
            "reference_levels": [[1, "ctrl"]]
        }),
    )?;

    let intercept_before = find_tab_metric(&session, "glm-1", "Intercept").ok_or("missing intercept")?;

    // Recode `"ctrl"` -> `"Control"` (note alphabetically `"Control"` vs `"drugA"`, plus saved ref must update).
    let recode_out =
        ok_cmd(&mut session, "table.recodeLevels", json!({ "sheet_id": "mv", "col": 1, "mappings": [{ "from": "ctrl", "to": "Control" }] }))?;
    assert_eq!(recode_out["updated_reference_levels"], json!(2));

    let glm_rs = session.document().result_sheet(&SheetId::new("glm-1")).ok_or("missing glm-1")?;
    assert_eq!(glm_rs.status, ResultStatus::UpToDate);
    let AnalysisKind::GeneralizedLinearModel(ref glm_spec) = glm_rs.analysis.kind else {
        return Err("expected Glm".into());
    };
    assert_eq!(glm_spec.reference_levels, vec![(1_usize, String::from("Control"))]);
    let intercept_after = find_tab_metric(&session, "glm-1", "Intercept").ok_or("missing intercept after")?;
    assert!((intercept_before - intercept_after).abs() < 1e-12);

    let cox_rs = session.document().result_sheet(&SheetId::new("cox-1")).ok_or("missing cox-1")?;
    assert_eq!(cox_rs.status, ResultStatus::UpToDate);
    let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { ref reference_levels, .. }) = cox_rs.analysis.kind else {
        return Err("expected Cox".into());
    };
    assert_eq!(reference_levels, &vec![(1_usize, String::from("Control"))]);
    Ok(())
}

#[test]
fn audit_97_extract_result_tab_refreshes_labels_and_grows_columns() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 97")?;
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4)?;
    col.data_sets[0].title = String::from("A");
    col.data_sets[1].title = String::from("B");
    col.data_sets[2].title = String::from("C");
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        col.data_sets[1].cells[r][0] = Cell::number(20.0 + r as f64);
        col.data_sets[2].cells[r][0] = Cell::number(40.0 + r as f64);
    }
    doc.add_data_sheet(DataSheet::new("src", "Source", TableArchetype::Column(col))?)?;
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "anova", "source_sheet_id": "src", "kind": "one_way_anova", "multiplicity": "tukey" }))?;
    ok_cmd(&mut session, "sheet.extractResultTab", json!({ "result_sheet_id": "anova", "tab_id": "multiple-comparisons", "sheet_id": "mc-ext" }))?;
    // Verify open column bound at extraction.
    let ext_ds = session.document().data_sheet(&SheetId::new("mc-ext")).ok_or("missing mc-ext")?;
    let LinkSource::InternalSheet { range, .. } = ext_ds.linked_regions[0].spec.source else {
        return Err("expected internal sheet source".into());
    };
    assert_eq!(range.end.col, 1_000_000);

    // Rename groups on `src`: refreshed extracted tab MUST update row_labels and shrink stale rows when fewer groups remain.
    ok_cmd(&mut session, "table.setTitles", json!({ "sheet_id": "src", "titles": ["Vehicle", "Low", "High"] }))?;
    let ext_after = session.document().data_sheet(&SheetId::new("mc-ext")).ok_or("missing mc-ext after rename")?;
    let TableArchetype::MultipleVariables(ref ext_mv) = ext_after.archetype else {
        return Err("expected MultipleVariables extracted sheet".into());
    };
    assert!(ext_mv.row_titles.iter().any(|lbl| lbl.contains("Vehicle") && lbl.contains("Low")), "got {:?}", ext_mv.row_titles);
    assert!(!ext_mv.row_titles.iter().any(|lbl| lbl == "A vs. B"));

    // Shrinking upstream to 2 groups reduces pairwise comparisons from 3 to 1; trailing stale row labels must clear.
    ok_cmd(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "src", "archetype": "column", "replace": true, "csv": "Vehicle,Low\n10,20\n11,21\n12,22\n" }),
    )?;
    let ext_shrunk = session.document().data_sheet(&SheetId::new("mc-ext")).ok_or("missing mc-ext after shrink")?;
    let TableArchetype::MultipleVariables(ref shrunk_mv) = ext_shrunk.archetype else {
        return Err("expected MultipleVariables extracted sheet".into());
    };
    assert_eq!(shrunk_mv.row_titles[0], "Vehicle vs. Low");
    assert_eq!(shrunk_mv.row_titles[1], "", "shrunk trailing row label must clear");

    // Extract DescriptiveStats tab (starts with 2 columns) -> open column bound allows growing to 3 columns on refresh!
    let mut col2 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    col2.data_sets[0].title = String::from("G1");
    col2.data_sets[1].title = String::from("G2");
    for r in 0..3 {
        col2.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        col2.data_sets[1].cells[r][0] = Cell::number((r + 10) as f64);
    }
    let mut doc2 = ProjectDocument::new("Tab Width")?;
    doc2.add_data_sheet(DataSheet::new("d2", "Data2", TableArchetype::Column(col2))?)?;
    let mut s2 = Session::from_document(doc2);
    ok_cmd(&mut s2, "stats.run", json!({ "sheet_id": "desc-dyn", "source_sheet_id": "d2", "kind": "descriptive_stats" }))?;
    ok_cmd(&mut s2, "sheet.extractResultTab", json!({ "result_sheet_id": "desc-dyn", "sheet_id": "ext-dyn", "archetype": "column" }))?;
    ok_cmd(
        &mut s2,
        "table.importCsv",
        json!({ "sheet_id": "d2", "archetype": "column", "replace": true, "csv": "New1,New2,New3\n10,20,30\n11,21,31\n12,22,32\n" }),
    )?;
    let ext2 = s2.document().data_sheet(&SheetId::new("ext-dyn")).ok_or("missing ext-dyn")?;
    let TableArchetype::Column(ref ext2_col) = ext2.archetype else {
        return Err("expected Column".into());
    };
    assert_eq!(ext2_col.data_sets.len(), 3, "open column bound must grow to 3 columns");
    assert_eq!(ext2_col.data_sets[0].title, "New1");
    assert_eq!(ext2_col.data_sets[2].title, "New3");
    assert_eq!(find_tab_metric(&s2, "desc-dyn", "Mean"), Some(11.0));
    assert!(ext2_col.data_sets[2].cells.iter().any(|row| row[0].active_number() == Some(31.0)));
    Ok(())
}

#[test]
fn audit_98_paste_link_rejects_overlap_and_duplicate_link_ids() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 98")?;
    for (sid, v) in [("src1", 10.0), ("src2", 20.0)] {
        let mut c = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
        c.data_sets[0].cells[0][0] = Cell::number(v);
        c.data_sets[0].cells[1][0] = Cell::number(v + 1.0);
        doc.add_data_sheet(DataSheet::new(sid, sid, TableArchetype::Column(c))?)?;
    }
    doc.add_data_sheet(DataSheet::new("dst", "Dest", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2)?))?)?;
    let mut session = Session::from_document(doc);

    ok_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "src1", "target_sheet_id": "dst", "start_row": 0, "start_col": 0, "link_id": "link-1" }),
    )?;

    // Overlapping target fails with Conflict and leaves dst intact.
    let err_overlap = err_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "src2", "target_sheet_id": "dst", "start_row": 0, "start_col": 0, "link_id": "link-2" }),
    )?;
    assert_eq!(err_overlap.code, ErrorCode::Conflict);

    // Duplicate link_id at a non-overlapping column also fails with Conflict.
    let err_dup = err_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "src2", "target_sheet_id": "dst", "start_row": 0, "start_col": 1, "link_id": "link-1" }),
    )?;
    assert_eq!(err_dup.code, ErrorCode::Conflict);

    // Non-overlapping with unique ID succeeds.
    ok_cmd(&mut session, "table.pasteLink", json!({ "source_sheet_id": "src2", "target_sheet_id": "dst", "start_row": 0, "start_col": 1 }))?;
    assert_eq!(session.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?.linked_regions.len(), 2);

    // InsertRows mode above a live link in the same column must also reject with Conflict.
    let mut dst_low = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5)?;
    dst_low.data_sets[0].cells[3][0] = Cell::number(99.0);
    let mut doc3 = ProjectDocument::new("InsertShift")?;
    let mut c_s = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    c_s.data_sets[0].cells[0][0] = Cell::number(1.0);
    c_s.data_sets[0].cells[1][0] = Cell::number(2.0);
    doc3.add_data_sheet(DataSheet::new("s1", "S1", TableArchetype::Column(c_s.clone()))?)?;
    doc3.add_data_sheet(DataSheet::new("s2", "S2", TableArchetype::Column(c_s))?)?;
    doc3.add_data_sheet(DataSheet::new("dl", "DestLow", TableArchetype::Column(dst_low))?)?;
    let mut s3 = Session::from_document(doc3);
    ok_cmd(&mut s3, "table.pasteLink", json!({ "source_sheet_id": "s1", "target_sheet_id": "dl", "start_row": 3, "start_col": 0 }))?;
    let err_shift = err_cmd(
        &mut s3,
        "table.pasteLink",
        json!({ "source_sheet_id": "s2", "target_sheet_id": "dl", "start_row": 0, "start_col": 0, "mode": "insert_rows" }),
    )?;
    assert_eq!(err_shift.code, ErrorCode::Conflict);
    Ok(())
}

#[test]
fn audit_99_standalone_power_analysis_ignores_source_info_sheet() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 99")?;
    let mut info = InfoSheet::new_preseeded("info-1", "Study Info")?;
    info.set_constant("Alpha", curvebook_doc::ConstantValue::Number(0.01), None)?;
    doc.add_info_sheet(info)?;
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    col.data_sets[0].cells[0][0] = Cell::number(1.0);
    let mut ds = DataSheet::new("col-1", "Data", TableArchetype::Column(col))?;
    ds.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(ds)?;

    let mut session = Session::from_document(doc);
    // Standalone (no info_sheet_id): alpha = 0.05, power = 0.8, effect = 0.5 -> N per group = 64.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "pow-standalone", "source_sheet_id": "col-1", "kind": "power_analysis", "family": "unpaired_t", "alpha": 0.05, "target_power": 0.8, "effect_size": 0.5 }),
    )?;
    // Explicitly linked to info-1 (alpha = 0.01) -> N per group = 96.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "pow-linked", "source_sheet_id": "col-1", "kind": "power_analysis", "info_sheet_id": "info-1", "family": "unpaired_t", "alpha": 0.05, "target_power": 0.8, "effect_size": 0.5 }),
    )?;
    ok_cmd(&mut session, "project.recalculate", json!({}))?;

    let n_standalone = session
        .document()
        .result_sheet(&SheetId::new("pow-standalone"))
        .and_then(|r| r.lookup_scalar("NPerGroup", None))
        .ok_or("missing standalone sample size")?;
    let n_linked = session
        .document()
        .result_sheet(&SheetId::new("pow-linked"))
        .and_then(|r| r.lookup_scalar("NPerGroup", None))
        .ok_or("missing linked sample size")?;
    assert!((n_standalone - 64.0).abs() < 1.0, "standalone power must stay at alpha=0.05 (N=64), got {n_standalone}");
    assert!((n_linked - 96.0).abs() < 1.0, "linked power must read Info alpha=0.01 (N=96), got {n_linked}");
    Ok(())
}

#[test]
fn audit_122_graph_set_series_patches_in_place_by_default() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 122")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 4, 3)?;
    for r in 0..3 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        for c in 0..4 {
            xy.data_sets[c].cells[r][0] = Cell::number(((c + 1) * (r + 1)) as f64);
        }
    }
    doc.add_data_sheet(DataSheet::new("xy", "Multi", TableArchetype::Xy(xy))?)?;
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g", "data_sheet_id": "xy", "kind": "xy" }))?;

    let orig_eb1 = session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?.spec.series[1].y_error_bars;

    // Short 1-element array without indices patches series 0 and leaves series 1..3 intact.
    let res = ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series": [{ "y_error_bars": { "metric": "Sem" } }] }))?;
    assert_eq!(res["series_count"], json!(4));
    let gs = session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?;
    assert_eq!(gs.spec.series.len(), 4);
    assert_eq!(gs.spec.series[0].y_error_bars.map(|eb| eb.metric), Some(curvebook_doc::ErrorBarMetric::Sem));
    assert_eq!(gs.spec.series[1].y_error_bars, orig_eb1);

    // Embedded `series_index` patches only slot 2.
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series": [{ "series_index": 2, "legend_label": "Target Two" }] }))?;
    let gs2 = session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?;
    assert_eq!(gs2.spec.series[2].legend_label.as_deref(), Some("Target Two"));
    assert_eq!(gs2.spec.series[0].legend_label, None);

    // Embedded `data_set_index` without `series_index` matches series 3.
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series": [{ "data_set_index": 3, "legend_label": "Target Three" }] }))?;
    assert_eq!(session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?.spec.series[3].legend_label.as_deref(), Some("Target Three"));

    // Over-long patch array rejected with BadInput.
    let err_long = err_cmd(
        &mut session,
        "graph.setSeries",
        json!({ "sheet_id": "g", "series": [{ "color": "#111111" }, { "color": "#222222" }, { "color": "#333333" }, { "color": "#444444" }, { "color": "#555555" }] }),
    )?;
    assert_eq!(err_long.code, ErrorCode::BadInput);

    // Out-of-bounds embedded series_index rejected with BadInput.
    let err_oob = err_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series": [{ "series_index": 9, "color": "#111111" }] }))?;
    assert_eq!(err_oob.code, ErrorCode::BadInput);

    // GUI full-length array updates all 4 series cleanly.
    ok_cmd(
        &mut session,
        "graph.setSeries",
        json!({
            "sheet_id": "g",
            "series": [
                { "legend_label": "S0" },
                { "legend_label": "S1" },
                { "legend_label": "S2" },
                { "legend_label": "S3" }
            ]
        }),
    )?;
    assert_eq!(session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?.spec.series.len(), 4);

    // all_series: true applies single patch across every series.
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "all_series": true, "bind_to_right_y2": true }))?;
    assert!(session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?.spec.series.iter().all(|s| s.bind_to_right_y2));

    // Explicit mode: "replace" shrinks series cleanly.
    let rep = ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "mode": "replace", "series": [{ "data_set_index": 0 }] }))?;
    assert_eq!(rep["series_count"], json!(1));
    assert_eq!(session.document().graph_sheet(&SheetId::new("g")).ok_or("missing g")?.spec.series.len(), 1);
    Ok(())
}

#[test]
fn audit_74_identity_conversion_preserves_single_link_copy() -> TestResult {
    let mut up = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3)?;
    let mut dst = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3)?;
    for r in 0..3 {
        up.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    dst.data_sets[0].cells[0][0] = Cell::number(0.0);
    let mut doc_id = ProjectDocument::new("Identity Link")?;
    doc_id.add_data_sheet(DataSheet::new("up", "Up", TableArchetype::Column(up))?)?;
    doc_id.add_data_sheet(DataSheet::new("dst", "Dst", TableArchetype::Column(dst))?)?;
    let mut sess_id = Session::from_document(doc_id);
    ok_cmd(
        &mut sess_id,
        "table.pasteLink",
        json!({ "source_sheet_id": "up", "target_sheet_id": "dst", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 2, "col": 0 } }, "start_row": 0, "start_col": 0, "link_id": "id-link" }),
    )?;
    let id_preview = ok_cmd(&mut sess_id, "table.convertArchetype", json!({ "sheet_id": "dst", "target_kind": "Column", "preview_only": true }))?;
    assert_eq!(id_preview["preview"]["lossy"], json!(false));
    ok_cmd(&mut sess_id, "table.convertArchetype", json!({ "sheet_id": "dst", "target_kind": "Column" }))?;
    let dst_after = sess_id.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?;
    assert_eq!(dst_after.linked_regions.len(), 1, "identity conversion duplicated linked_regions: {:?}", dst_after.linked_regions);
    assert_eq!(dst_after.linked_regions[0].id, "id-link");
    Ok(())
}

#[test]
fn audit_74_grouped_replicates_expands_by_rows_and_detaches_transposed_link() -> TestResult {
    let mut grp = curvebook_doc::GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?;
    grp.row_titles = vec![String::from("R0"), String::from("R1"), String::from("R2")];
    for r in 0..3 {
        for s in 0..2 {
            grp.data_sets[0].cells[r][s] = Cell::number((r * 2 + s + 1) as f64);
            grp.data_sets[1].cells[r][s] = Cell::number((100 + r * 2 + s) as f64);
        }
    }
    let mut doc_grp = ProjectDocument::new("Grouped Expand")?;
    doc_grp.add_data_sheet(DataSheet::new("grp", "Grouped", TableArchetype::Grouped(grp))?)?;
    doc_grp.add_data_sheet(DataSheet::new(
        "grp_copy",
        "Copy",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?),
    )?)?;
    let mut sess_grp = Session::from_document(doc_grp);
    ok_cmd(
        &mut sess_grp,
        "table.pasteLink",
        json!({ "source_sheet_id": "grp", "target_sheet_id": "grp_copy", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 2, "col": 1 } }, "start_row": 0, "start_col": 0, "link_id": "grp-out" }),
    )?;
    ok_cmd(
        &mut sess_grp,
        "stats.run",
        json!({ "sheet_id": "grp_desc", "source_sheet_id": "grp", "kind": "descriptive_stats", "selected_data_sets": [1] }),
    )?;
    ok_cmd(&mut sess_grp, "table.convertArchetype", json!({ "sheet_id": "grp", "target_kind": "Column", "allow_lossy": true }))?;
    let grp_conv = sess_grp.document().data_sheet(&SheetId::new("grp")).ok_or("missing grp")?;
    assert_eq!(grp_conv.archetype.column_count(), 6);
    let grp_desc = sess_grp.document().result_sheet(&SheetId::new("grp_desc")).ok_or("missing grp_desc")?;
    assert_eq!(grp_desc.status, ResultStatus::UpToDate);
    assert_eq!(grp_desc.analysis.selected_data_sets, vec![3, 4, 5]);
    assert!(sess_grp.document().data_sheet(&SheetId::new("grp_copy")).ok_or("missing grp_copy")?.linked_regions.is_empty());
    Ok(())
}

#[test]
fn audit_74_xy_nonmatching_parametric_keeps_y_column_indices() -> TestResult {
    let mut xy_param =
        XyTable::new_with_x_mode(curvebook_doc::XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSdN, 2, 3)?;
    for r in 0..3 {
        xy_param.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy_param.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        xy_param.data_sets[0].cells[r][1] = Cell::number(1.0);
        xy_param.data_sets[0].cells[r][2] = Cell::number(3.0);
        xy_param.data_sets[1].cells[r][0] = Cell::number(50.0 + r as f64);
        xy_param.data_sets[1].cells[r][1] = Cell::number(2.0);
        xy_param.data_sets[1].cells[r][2] = Cell::number(3.0);
    }
    let mut doc_xy = ProjectDocument::new("XY Param")?;
    doc_xy.add_data_sheet(DataSheet::new("xyp", "XY Param", TableArchetype::Xy(xy_param))?)?;
    doc_xy.add_data_sheet(DataSheet::new(
        "xyp_copy",
        "Copy",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3)?),
    )?)?;
    let mut sess_xy = Session::from_document(doc_xy);
    ok_cmd(
        &mut sess_xy,
        "table.pasteLink",
        json!({ "source_sheet_id": "xyp", "target_sheet_id": "xyp_copy", "source_range": { "start": { "row": 0, "col": 1 }, "end": { "row": 2, "col": 1 } }, "start_row": 0, "start_col": 0, "link_id": "xyp-out" }),
    )?;
    ok_cmd(
        &mut sess_xy,
        "stats.run",
        json!({ "sheet_id": "xyp_desc", "source_sheet_id": "xyp", "kind": "descriptive_stats", "selected_data_sets": [1] }),
    )?;
    ok_cmd(&mut sess_xy, "graph.create", json!({ "sheet_id": "g_xyp", "data_sheet_id": "xyp", "kind": "xy" }))?;
    ok_cmd(&mut sess_xy, "table.convertArchetype", json!({ "sheet_id": "xyp", "target_kind": "Column", "allow_lossy": true }))?;
    assert_eq!(sess_xy.document().data_sheet(&SheetId::new("xyp")).ok_or("missing xyp")?.archetype.column_count(), 2);
    let xyp_desc = sess_xy.document().result_sheet(&SheetId::new("xyp_desc")).ok_or("missing xyp_desc")?;
    assert_eq!(xyp_desc.status, ResultStatus::UpToDate);
    assert_eq!(xyp_desc.analysis.selected_data_sets, vec![1]);
    let g_xyp = sess_xy.document().graph_sheet(&SheetId::new("g_xyp")).ok_or("missing g_xyp")?;
    assert_eq!(g_xyp.spec.series[0].data_set_index, 0);
    assert_eq!(g_xyp.spec.series[1].data_set_index, 1);
    let xyp_link = &sess_xy.document().data_sheet(&SheetId::new("xyp_copy")).ok_or("missing xyp_copy")?.linked_regions[0];
    assert!(matches!(xyp_link.spec.source, LinkSource::InternalSheet { range, .. } if range.start.col == 1 && range.end.col == 1));
    Ok(())
}

#[test]
fn audit_74_mv_contingency_non_crosstab_maps_sequential_data_indices() -> TestResult {
    let mut mv = MultipleVariablesTable::new(3, 2)?;
    mv.variables[0].role = VariableRole::Label;
    mv.variables[0].title = String::from("Strain");
    mv.variables[0].cells = vec![Cell::text("WT"), Cell::text("KO")];
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[1].title = String::from("Dose");
    mv.variables[1].cells = vec![Cell::number(1.0), Cell::number(2.0)];
    mv.variables[2].role = VariableRole::Continuous;
    mv.variables[2].title = String::from("Count");
    mv.variables[2].cells = vec![Cell::number(15.0), Cell::number(25.0)];
    let mut mv_non_crosstab = MultipleVariablesTable::new(4, 2)?;
    mv_non_crosstab.variables[0].role = VariableRole::Label;
    mv_non_crosstab.variables[1].role = VariableRole::Label;
    mv_non_crosstab.variables[2].role = VariableRole::Categorical;
    mv_non_crosstab.variables[2].cells = vec![Cell::number(1.0), Cell::number(2.0)];
    mv_non_crosstab.variables[3].role = VariableRole::Continuous;
    mv_non_crosstab.variables[3].cells = vec![Cell::number(10.0), Cell::number(20.0)];
    let mut doc_mv = ProjectDocument::new("MV Contingency")?;
    doc_mv.add_data_sheet(DataSheet::new("mv", "MV", TableArchetype::MultipleVariables(mv))?)?;
    doc_mv.add_data_sheet(DataSheet::new("mv2", "MV2", TableArchetype::MultipleVariables(mv_non_crosstab))?)?;
    doc_mv.add_result_sheet(ResultSheet::new(AnalysisSpec {
        selected_data_sets: vec![1, 2],
        ..AnalysisSpec::new(
            "mv_norm",
            "mv",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::FixedValue(0.0),
                hundred_anchor: NormalizeAnchor::FixedValue(100.0),
                output_percent: true,
                per_data_set: false,
            }),
        )?
    })?)?;
    doc_mv.add_result_sheet(ResultSheet::new(AnalysisSpec {
        selected_data_sets: vec![2, 3],
        ..AnalysisSpec::new(
            "mv2_norm",
            "mv2",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::FixedValue(0.0),
                hundred_anchor: NormalizeAnchor::FixedValue(100.0),
                output_percent: true,
                per_data_set: false,
            }),
        )?
    })?)?;
    let mut sess_mv = Session::from_document(doc_mv);
    ok_cmd(&mut sess_mv, "table.convertArchetype", json!({ "sheet_id": "mv", "target_kind": "Contingency", "allow_lossy": true }))?;
    ok_cmd(&mut sess_mv, "table.convertArchetype", json!({ "sheet_id": "mv2", "target_kind": "Contingency", "allow_lossy": true }))?;
    let mv_norm = sess_mv.document().result_sheet(&SheetId::new("mv_norm")).ok_or("missing mv_norm")?;
    assert_eq!(mv_norm.status, ResultStatus::UpToDate);
    assert_eq!(mv_norm.analysis.selected_data_sets, vec![0, 1]);
    let mv2_norm = sess_mv.document().result_sheet(&SheetId::new("mv2_norm")).ok_or("missing mv2_norm")?;
    assert_eq!(mv2_norm.status, ResultStatus::UpToDate);
    assert_eq!(mv2_norm.analysis.selected_data_sets, vec![2, 3]);
    Ok(())
}

#[test]
fn audit_74_broken_dependents_and_cross_sheet_hooks_stay_failed_on_recalc() -> TestResult {
    let mut ds_a = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 3)?;
    ds_a.data_sets[0].title = String::from("10");
    ds_a.data_sets[1].title = String::from("20");
    ds_a.data_sets[2].title = String::from("30");
    let mut ds_b = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    for r in 0..3 {
        ds_a.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        ds_a.data_sets[1].cells[r][0] = Cell::number(20.0 + r as f64);
        ds_a.data_sets[2].cells[r][0] = Cell::number(30.0 + r as f64);
        ds_b.data_sets[0].cells[r][0] = Cell::number(5.0 + r as f64);
        ds_b.data_sets[1].cells[r][0] = Cell::number(10.0 + r as f64);
    }
    let mut doc_fail = ProjectDocument::new("Stay Failed")?;
    doc_fail.add_data_sheet(DataSheet::new("ds_a", "A", TableArchetype::Column(ds_a))?)?;
    doc_fail.add_data_sheet(DataSheet::new("ds_b", "B", TableArchetype::Column(ds_b))?)?;
    doc_fail.add_result_sheet(ResultSheet::new(AnalysisSpec {
        selected_data_sets: vec![0, 1],
        ..AnalysisSpec::new(
            "hook_b",
            "ds_b",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::Hooked(HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("ds_a"), data_set_index: 0 }),
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: false,
            }),
        )?
    })?)?;
    doc_fail.add_result_sheet(ResultSheet::new(AnalysisSpec::new(
        "ctrl_a",
        "ds_a",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ControlColumnMean { column: 0 },
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: false,
        }),
    )?)?)?;
    doc_fail.add_result_sheet(ResultSheet::new(AnalysisSpec::new(
        "base_a",
        "ds_a",
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline {
            baseline_columns: vec![0],
            operation: curvebook_doc::BaselineOperation::Difference,
        }),
    )?)?)?;
    let mut sess_fail = Session::from_document(doc_fail);
    ok_cmd(&mut sess_fail, "project.recalculate", json!({}))?;
    ok_cmd(&mut sess_fail, "table.convertArchetype", json!({ "sheet_id": "ds_a", "target_kind": "XY", "allow_lossy": true }))?;

    let hook_b = sess_fail.document().result_sheet(&SheetId::new("hook_b")).ok_or("missing hook_b")?;
    assert!(matches!(hook_b.status, ResultStatus::Failed { .. }));
    assert_eq!(hook_b.analysis.selected_data_sets, vec![0, 1], "cross-sheet hook failure corrupted ds_b selected_data_sets");
    for rid in ["hook_b", "ctrl_a", "base_a"] {
        assert!(matches!(sess_fail.document().result_sheet(&SheetId::new(rid)).ok_or("missing")?.status, ResultStatus::Failed { .. }));
    }

    let recalc = ok_cmd(&mut sess_fail, "project.recalculate", json!({}))?;
    assert_eq!(recalc["failed_count"], json!(3));
    for rid in ["hook_b", "ctrl_a", "base_a"] {
        assert!(
            matches!(sess_fail.document().result_sheet(&SheetId::new(rid)).ok_or("missing")?.status, ResultStatus::Failed { .. }),
            "{rid} silently recovered with bogus column mapping on project.recalculate"
        );
    }
    Ok(())
}
