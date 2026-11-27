#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::error::Error;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BaselineOperation, Cell, CellCoord, ConstantValue, DataSheet, ErrorBarMetric, ErrorCode, GraphAnnotation, HookSource,
    InfoSheet, LayoutItem, LinkSource, NonlinearFitSpec, NormalizeAnchor, ParameterRule, ParameterScope, PowerAnalysisSpec, ProjectDocument,
    ResultCell, ResultSheet, ResultStatus, SheetId, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec, VariableRole,
    XColumnMode, XyTable,
};
use curvebook_engine::Session;
use serde_json::{Value, json};

use super::common::{
    TempDir, assert_bad_input, assert_bad_inputs, assert_err_code, col_sheet, ds, grouped_rep_sheet, gs, info, ls, mv_sheet, ok_payload as ok_cmd,
    rs, run_cmds, session_with, try_err_cmd as err_cmd, xy_sheet,
};

type TestResult = Result<(), Box<dyn Error>>;

fn find_tab_metric(session: &Session, res_id: &str, metric_label: &str) -> Option<f64> {
    let tab = rs(session, res_id).tabs.first()?;
    let idx = tab.row_labels.iter().position(|lbl| lbl.eq_ignore_ascii_case(metric_label))?;
    tab.rows.get(idx)?.iter().find_map(|c| match c {
        ResultCell::Number { value, .. } => Some(*value),
        _ => None,
    })
}

#[test]
fn audit_57_import_plate_map_rejects_ragged_and_wrong_row_count_grids() -> TestResult {
    let mut session = Session::new();
    let roles96: Vec<Value> = (0..96)
        .map(|i| {
            if i < 4 {
                json!("Blank")
            } else {
                json!({ "role": "treatment", "group_name": format!("G{}", i / 12), "dilution_x": (i % 12 + 1) as f64, "replicate": 1 })
            }
        })
        .collect();

    let mut ragged: Vec<Vec<Option<f64>>> = vec![vec![Some(1.0); 12]; 8];
    ragged[3].pop();
    ragged[7].push(Some(2.0));
    let err_ragged = assert_bad_input(&mut session, "table.importPlateMap", json!({ "geometry": 96, "roles": roles96, "values": ragged }));
    assert!(err_ragged.message().contains("row 3"));

    assert_bad_inputs(
        &mut session,
        &[
            ("table.importPlateMap", json!({ "geometry": 96, "roles": roles96, "values": vec![vec![Some(1.0); 16]; 6] })),
            ("table.importPlateMap", json!({ "geometry": 96, "roles": roles96, "values": vec![Some(1.0); 95] })),
        ],
    );
    let ok = ok_cmd(&mut session, "table.importPlateMap", json!({ "geometry": 96, "roles": roles96, "values": vec![vec![Some(5.0); 12]; 8] }))?;
    assert!(ok.get("sheet_id").and_then(Value::as_str).is_some());
    Ok(())
}

#[test]
fn audit_71_recalculate_and_import_csv_surface_failed_result_sheets() -> TestResult {
    let mut session = session_with("Audit 71", vec![xy_sheet("d1", "Dose", "X", &[1.0, 2.0, 3.0, 4.0], &[("Y", &[2.0, 4.0, 6.0, 8.0])])]);
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "fit_1", "source_sheet_id": "d1", "kind": "linear", "title": "Linear Fit" }))?;

    let imp = session.execute("table.importCsv", json!({ "sheet_id": "d1", "archetype": "xy", "replace": true, "csv": "X,Y\n1,\n2,\n3,\n4,\n" }))?;
    assert_eq!(
        (
            imp.payload["failed_count"].as_u64(),
            imp.payload["failed_result_sheets"][0]["sheet_id"].as_str(),
            imp.payload["failed_result_sheets"][0]["title"].as_str()
        ),
        (Some(1), Some("fit_1"), Some("Linear Fit"))
    );
    assert!(imp.message.contains("fit_1"));

    let recalc = session.execute("project.recalculate", json!({}))?;
    assert_eq!((recalc.payload["failed_count"].as_u64(), recalc.payload["failed_result_sheets"][0]["sheet_id"].as_str()), (Some(1), Some("fit_1")));
    assert!(recalc.message.contains("fit_1"));

    assert_eq!(session.recalculate_all_with_report()?.failed_result_sheets, vec![SheetId::new("fit_1")]);
    let failed = session.failed_result_sheets();
    assert_eq!((failed.len(), &failed[0].sheet_id), (1, &SheetId::new("fit_1")));
    session.recalculate_all()?;
    Ok(())
}

#[test]
fn audit_74_convert_archetype_remaps_dependents_and_handles_links() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 74")?;
    doc.add_data_sheet(xy_sheet("xy", "XY Source", "Conc", &[1.0, 2.0, 3.0, 4.0], &[("10", &[100.0, 110.0, 120.0, 130.0])]))?;
    doc.add_data_sheet(col_sheet("up", "Upstream", &[("U", &[100.0, 110.0, 120.0, 130.0])]))?;
    doc.add_data_sheet(col_sheet("copy", "Linked Copy", &[("C", &[])]))?;
    doc.add_data_sheet(xy_sheet("xy2", "Downstream XY", "X", &[1.0, 2.0, 3.0, 4.0], &[("Y", &[20.0, 40.0, 60.0, 80.0])]))?;

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
    run_cmds(
        &mut session,
        &[
            (
                "table.pasteLink",
                json!({ "source_sheet_id": "xy", "target_sheet_id": "copy", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 3, "col": 0 } }, "start_row": 0, "start_col": 0 }),
            ),
            (
                "table.pasteLink",
                json!({ "source_sheet_id": "up", "target_sheet_id": "xy", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 3, "col": 0 } }, "start_row": 0, "start_col": 0, "link_id": "in-up" }),
            ),
            ("stats.run", json!({ "sheet_id": "desc-explicit", "source_sheet_id": "xy", "kind": "descriptive_stats", "selected_data_sets": [0] })),
            ("stats.run", json!({ "sheet_id": "desc-all", "source_sheet_id": "xy", "kind": "descriptive_stats" })),
            ("graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy", "kind": "xy" })),
            ("table.convertArchetype", json!({ "sheet_id": "xy", "target_kind": "Column" })),
        ],
    );

    let conv_ds = ds(&session, "xy");
    assert_eq!((conv_ds.linked_regions.len(), conv_ds.linked_regions[0].target_range.start.col), (1, 1));
    for rid in ["desc-explicit", "desc-all"] {
        assert_eq!((&rs(&session, rid).status, rs(&session, rid).analysis.selected_data_sets.as_slice()), (&ResultStatus::UpToDate, [1].as_slice()));
        assert!((find_tab_metric(&session, rid, "Median").ok_or("missing Median")? - 115.0).abs() < 1e-9);
    }
    assert_eq!(gs(&session, "g-xy").spec.series[0].data_set_index, 1);
    let hook_rs = rs(&session, "res-hook");
    assert_eq!(hook_rs.status, ResultStatus::UpToDate);
    assert!((hook_rs.lookup_scalar("K", Some(0)).ok_or("missing K")? - 2.0).abs() < 1e-6);

    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "up", "row": 0, "col": 0, "value": 500.0 }))?;
    assert_eq!(ds(&session, "copy").archetype.data_cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(500.0));

    // Preview & commit lossy conversion of 2-col Column -> XY (col 0 moves to X).
    let mut s2 = session_with(
        "Unmap",
        vec![
            col_sheet("up2", "Up2", &[("U", &[1.0, 2.0, 3.0])]),
            col_sheet("c2", "Two Cols", &[("A", &[1.0, 2.0, 3.0]), ("B", &[10.0, 11.0, 12.0])]),
        ],
    );
    run_cmds(
        &mut s2,
        &[
            (
                "table.pasteLink",
                json!({ "source_sheet_id": "up2", "target_sheet_id": "c2", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 2, "col": 0 } }, "start_row": 0, "start_col": 0, "link_id": "in-col0" }),
            ),
            ("stats.run", json!({ "sheet_id": "d0", "source_sheet_id": "c2", "kind": "descriptive_stats", "selected_data_sets": [0] })),
        ],
    );
    let preview = ok_cmd(&mut s2, "table.convertArchetype", json!({ "sheet_id": "c2", "target_kind": "XY", "preview_only": true }))?;
    assert_eq!(preview["preview"]["lossy"], json!(true));
    assert!(preview["preview"]["warnings"].as_array().ok_or("warnings")?.len() >= 2);
    assert_bad_input(&mut s2, "table.convertArchetype", json!({ "sheet_id": "c2", "target_kind": "XY" }));
    ok_cmd(&mut s2, "table.convertArchetype", json!({ "sheet_id": "c2", "target_kind": "XY", "allow_lossy": true }))?;
    assert!(ds(&s2, "c2").linked_regions.is_empty() && matches!(rs(&s2, "d0").status, ResultStatus::Failed { .. }));
    Ok(())
}

#[test]
fn audit_75_undo_open_restores_matching_file_path() -> TestResult {
    let dir = TempDir::try_new("audit-75")?;
    let (path_a, path_b) = (dir.child("study_a.curvebook"), dir.child("study_b.curvebook"));

    for (title, sid, path) in [("Project A", "sheet-a", &path_a), ("Project B", "sheet-b", &path_b)] {
        let mut s = Session::from_document(ProjectDocument::new(title)?);
        assert!(!s.is_dirty());
        ok_cmd(&mut s, "sheet.createData", json!({ "sheet_id": sid, "title": title, "kind": "Column" }))?;
        assert!(s.is_dirty());
        ok_cmd(&mut s, "project.save", json!({ "path": path.to_string_lossy() }))?;
        assert!(!s.is_dirty());
    }
    let bytes_b_before = std::fs::read(&path_b)?;

    let mut session = Session::new();
    ok_cmd(&mut session, "project.open", json!({ "path": path_a.to_string_lossy() }))?;
    assert!(!session.is_dirty() && session.file_path() == Some(path_a.as_path()));

    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "temp-dirty", "title": "Temp", "kind": "Column" }))?;
    assert!(session.is_dirty());
    assert_eq!(err_cmd(&mut session, "project.open", json!({ "path": dir.child("missing.curvebook").to_string_lossy() }))?.code, ErrorCode::IoError);
    assert!(session.is_dirty());

    ok_cmd(&mut session, "project.open", json!({ "path": path_b.to_string_lossy() }))?;
    assert!(!session.is_dirty() && session.document().title == "Project B" && session.file_path() == Some(path_b.as_path()));

    assert!(session.undo()? && session.is_dirty() && session.document().title == "Project A" && session.file_path() == Some(path_a.as_path()));
    let target_save = session.file_path().ok_or("restored file_path")?.to_path_buf();
    ok_cmd(&mut session, "project.save", json!({ "path": target_save.to_string_lossy() }))?;
    assert!(!session.is_dirty() && std::fs::read(&path_b)? == bytes_b_before);

    assert!(session.redo()? && session.is_dirty() && session.document().title == "Project B" && session.file_path() == Some(path_b.as_path()));
    ok_cmd(&mut session, "project.save", json!({ "path": path_b.to_string_lossy() }))?;
    assert!(!session.is_dirty());
    Ok(())
}

#[test]
fn audit_76_duplicate_family_remaps_overlays_cld_hooks_anchors_and_power_info() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 76")?;
    let mut info1 = InfoSheet::new_preseeded("info-1", "Cohort Info")?;
    info1.set_constant("NormAnchor", ConstantValue::Number(200.0), None)?;
    info1.set_constant("Alpha", ConstantValue::Number(0.05), None)?;
    info1.linked_data_sheets.push(SheetId::new("xy-1"));
    doc.add_info_sheet(info1)?;

    let mut d = xy_sheet("xy-1", "Curve", "X", &[1.0, 2.0, 3.0], &[("Y", &[100.0, 200.0, 300.0])]);
    d.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(d)?;

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
    run_cmds(
        &mut session,
        &[
            ("fit.run", json!({ "sheet_id": "fit-1", "source_sheet_id": "xy-1", "kind": "linear" })),
            ("stats.run", json!({ "sheet_id": "desc-1", "source_sheet_id": "xy-1", "kind": "descriptive_stats" })),
            ("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "kind": "xy" })),
            ("graph.setRegressionOverlay", json!({ "sheet_id": "g-1", "result_sheet_id": "fit-1", "series_index": 0 })),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g-1", "spec": { "title": "Anchor={{info:info-1:NormAnchor}}", "compact_letter_display": { "enabled": true, "result_sheet_id": "desc-1" } } }),
            ),
            (
                "graph.addAnnotation",
                json!({ "sheet_id": "g-1", "kind": "text", "text": "Slope={{scalar:fit-1:Slope:0}}", "x_pt": 10.0, "y_pt": 10.0 }),
            ),
            ("layout.create", json!({ "sheet_id": "lay-1" })),
            ("layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-1", "bounds_pt": [10.0, 10.0, 100.0, 80.0] })),
            (
                "layout.addPanel",
                json!({ "sheet_id": "lay-1", "kind": "text", "markdown": "Val={{info:info-1:NormAnchor}}", "bounds_pt": [10.0, 95.0, 100.0, 20.0] }),
            ),
        ],
    );

    let cloned = ok_cmd(&mut session, "sheet.duplicateFamily", json!({ "sheet_id": "xy-1", "new_sheet_id": "xy-2" }))?;
    let (c_info, c_graph, c_layout) = (
        cloned["info_sheet_id"].as_str().ok_or("info")?,
        cloned["graph_sheet_ids"][0].as_str().ok_or("graph")?,
        cloned["layout_sheet_ids"][0].as_str().ok_or("layout")?,
    );
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": c_info, "name": "NormAnchor", "value": 400.0 }))?;

    let c_norm = session
        .document()
        .result_sheets
        .iter()
        .find(|r| r.analysis.source_sheet_id.as_ref() == Some(&SheetId::new("xy-2")) && matches!(r.analysis.kind, AnalysisKind::Transform(_)))
        .ok_or("cloned norm")?;
    let last_val = c_norm
        .analyzable_data
        .as_ref()
        .and_then(|d| d.archetype.data_cell(CellCoord::new(2, 0, 0)))
        .and_then(Cell::active_number)
        .ok_or("norm val")?;
    assert!((last_val - (200.0 / 300.0 * 100.0)).abs() < 1e-4);

    let c_pow = session
        .document()
        .result_sheets
        .iter()
        .find(|r| r.analysis.source_sheet_id.as_ref() == Some(&SheetId::new("xy-2")) && matches!(r.analysis.kind, AnalysisKind::PowerAnalysis(_)))
        .ok_or("cloned pow")?;
    assert!(matches!(&c_pow.analysis.kind, AnalysisKind::PowerAnalysis(p) if p.info_sheet_id.as_ref().map(SheetId::as_str) == Some(c_info)));

    let c_gs = gs(&session, c_graph);
    let ov_id = &c_gs.spec.series[0].regression_overlay.as_ref().ok_or("ov")?.result_sheet_id;
    let cld_id = &c_gs.spec.compact_letter_display.as_ref().ok_or("cld")?.result_sheet_id;
    assert!(ov_id.as_str() != "fit-1" && cld_id.as_str() != "desc-1" && c_gs.spec.title.contains(c_info));
    assert!(matches!(&c_gs.spec.annotations[0], GraphAnnotation::TextBox { markdown, .. } if markdown.contains(ov_id.as_str())));
    assert!(ls(&session, c_layout).spec.items.iter().any(|it| matches!(it, LayoutItem::TextCallout { markdown, .. } if markdown.contains(c_info))));
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
    let (i1, i2) = (r1["info_sheet_id"].as_str().ok_or("i1")?, r2["info_sheet_id"].as_str().ok_or("i2")?);
    assert_ne!(i1, i2);
    assert_eq!(info(&session, i1).get_constant("randomization.seed").map(|c| &c.value), Some(&ConstantValue::Text(String::from("111"))));
    assert_eq!(info(&session, i2).get_constant("randomization.seed").map(|c| &c.value), Some(&ConstantValue::Text(String::from("222"))));

    for (src_id, auto_info, seed_a, seed_b) in [("src-mv", false, 10, 20), ("src-paired", true, 101, 202)] {
        ok_cmd(
            &mut session,
            "sheet.createData",
            json!({ "sheet_id": src_id, "kind": "multiple_variables", "rows": 8, "cols": 1, "auto_info": auto_info }),
        )?;
        for r in 0..8 {
            ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": src_id, "row": r, "col": 0, "value": (r + 1) as f64 }))?;
        }
        let s1 = ok_cmd(&mut session, "table.randomSampleRows", json!({ "source_sheet_id": src_id, "sample_size": 3, "seed": seed_a }))?;
        let s2 = ok_cmd(&mut session, "table.randomSampleRows", json!({ "source_sheet_id": src_id, "sample_size": 4, "seed": seed_b }))?;
        let (si1, si2) = (s1["info_sheet_id"].as_str().ok_or("si1")?, s2["info_sheet_id"].as_str().ok_or("si2")?);
        assert!(si1 != si2 && si1 != i1);
        assert_eq!(info(&session, si1).get_constant("sample.seed").map(|c| &c.value), Some(&ConstantValue::Text(seed_a.to_string())));
        assert_eq!(info(&session, si2).get_constant("sample.seed").map(|c| &c.value), Some(&ConstantValue::Text(seed_b.to_string())));
    }
    Ok(())
}

#[test]
fn audit_96_recode_levels_remaps_glm_and_cox_reference_levels() -> TestResult {
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
    let c0: Vec<Cell> = rows.iter().map(|r| Cell::number(r.0)).collect();
    let c1: Vec<Cell> = rows.iter().map(|r| Cell::text(r.1)).collect();
    let c2: Vec<Cell> = rows.iter().map(|r| Cell::number(r.2)).collect();
    let c3: Vec<Cell> = rows.iter().map(|r| Cell::number(r.3)).collect();
    let mut session = session_with(
        "Audit 96",
        vec![mv_sheet(
            "mv",
            "Cohort",
            &[],
            &[("Outcome", None, &c0), ("Arm", Some(VariableRole::Categorical), &c1), ("Time", None, &c2), ("Status", None, &c3)],
        )],
    );

    run_cmds(
        &mut session,
        &[
            (
                "stats.run",
                json!({ "sheet_id": "glm-1", "source_sheet_id": "mv", "kind": "glm", "family": "linear", "response_column": 0, "predictor_columns": [1], "reference_levels": [[1, "ctrl"]] }),
            ),
            (
                "stats.run",
                json!({ "sheet_id": "cox-1", "source_sheet_id": "mv", "kind": "cox_ph", "time_column": 2, "status_column": 3, "predictor_columns": [1], "reference_levels": [[1, "ctrl"]] }),
            ),
        ],
    );
    let intercept_before = find_tab_metric(&session, "glm-1", "Intercept").ok_or("intercept")?;
    let recode_out =
        ok_cmd(&mut session, "table.recodeLevels", json!({ "sheet_id": "mv", "col": 1, "mappings": [{ "from": "ctrl", "to": "Control" }] }))?;
    assert_eq!(recode_out["updated_reference_levels"], json!(2));

    let glm_rs = rs(&session, "glm-1");
    assert!(
        glm_rs.status == ResultStatus::UpToDate
            && matches!(&glm_rs.analysis.kind, AnalysisKind::GeneralizedLinearModel(g) if g.reference_levels == vec![(1, String::from("Control"))])
    );
    assert!((intercept_before - find_tab_metric(&session, "glm-1", "Intercept").ok_or("after")?).abs() < 1e-12);
    let cox_rs = rs(&session, "cox-1");
    assert!(
        cox_rs.status == ResultStatus::UpToDate
            && matches!(&cox_rs.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { reference_levels, .. }) if reference_levels == &vec![(1, String::from("Control"))])
    );
    Ok(())
}

#[test]
fn audit_97_extract_result_tab_refreshes_labels_and_grows_columns() -> TestResult {
    let mut session = session_with(
        "Audit 97",
        vec![col_sheet("src", "Source", &[("A", &[10.0, 11.0, 12.0, 13.0]), ("B", &[20.0, 21.0, 22.0, 23.0]), ("C", &[40.0, 41.0, 42.0, 43.0])])],
    );
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "anova", "source_sheet_id": "src", "kind": "one_way_anova", "multiplicity": "tukey" })),
            ("sheet.extractResultTab", json!({ "result_sheet_id": "anova", "tab_id": "multiple-comparisons", "sheet_id": "mc-ext" })),
        ],
    );
    assert!(matches!(ds(&session, "mc-ext").linked_regions[0].spec.source, LinkSource::InternalSheet { range, .. } if range.end.col == 1_000_000));

    ok_cmd(&mut session, "table.setTitles", json!({ "sheet_id": "src", "titles": ["Vehicle", "Low", "High"] }))?;
    let TableArchetype::MultipleVariables(ext_mv) = &ds(&session, "mc-ext").archetype else { return Err("expected MV".into()) };
    assert!(ext_mv.row_titles.iter().any(|l| l.contains("Vehicle") && l.contains("Low")) && !ext_mv.row_titles.iter().any(|l| l == "A vs. B"));

    ok_cmd(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "src", "archetype": "column", "replace": true, "csv": "Vehicle,Low\n10,20\n11,21\n12,22\n" }),
    )?;
    let TableArchetype::MultipleVariables(shrunk_mv) = &ds(&session, "mc-ext").archetype else { return Err("expected MV".into()) };
    assert_eq!((shrunk_mv.row_titles[0].as_str(), shrunk_mv.row_titles[1].as_str()), ("Vehicle vs. Low", ""));

    let mut s2 = session_with("Tab Width", vec![col_sheet("d2", "Data2", &[("G1", &[1.0, 2.0, 3.0]), ("G2", &[10.0, 11.0, 12.0])])]);
    run_cmds(
        &mut s2,
        &[
            ("stats.run", json!({ "sheet_id": "desc-dyn", "source_sheet_id": "d2", "kind": "descriptive_stats" })),
            ("sheet.extractResultTab", json!({ "result_sheet_id": "desc-dyn", "sheet_id": "ext-dyn", "archetype": "column" })),
            (
                "table.importCsv",
                json!({ "sheet_id": "d2", "archetype": "column", "replace": true, "csv": "New1,New2,New3\n10,20,30\n11,21,31\n12,22,32\n" }),
            ),
        ],
    );
    let TableArchetype::Column(ext2_col) = &ds(&s2, "ext-dyn").archetype else { return Err("expected Column".into()) };
    assert_eq!((ext2_col.data_sets.len(), ext2_col.data_sets[0].title.as_str(), ext2_col.data_sets[2].title.as_str()), (3, "New1", "New3"));
    assert!(find_tab_metric(&s2, "desc-dyn", "Mean") == Some(11.0) && ext2_col.data_sets[2].cells.iter().any(|r| r[0].active_number() == Some(31.0)));
    Ok(())
}

#[test]
fn audit_98_paste_link_rejects_overlap_and_duplicate_link_ids() -> TestResult {
    let mut session = session_with(
        "Audit 98",
        vec![
            col_sheet("src1", "src1", &[("A", &[10.0, 11.0])]),
            col_sheet("src2", "src2", &[("A", &[20.0, 21.0])]),
            col_sheet("dst", "Dest", &[("A", &[]), ("B", &[])]),
        ],
    );
    ok_cmd(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "src1", "target_sheet_id": "dst", "start_row": 0, "start_col": 0, "link_id": "link-1" }),
    )?;
    assert_err_code(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "src2", "target_sheet_id": "dst", "start_row": 0, "start_col": 0, "link_id": "link-2" }),
        ErrorCode::Conflict,
    );
    assert_err_code(
        &mut session,
        "table.pasteLink",
        json!({ "source_sheet_id": "src2", "target_sheet_id": "dst", "start_row": 0, "start_col": 1, "link_id": "link-1" }),
        ErrorCode::Conflict,
    );
    ok_cmd(&mut session, "table.pasteLink", json!({ "source_sheet_id": "src2", "target_sheet_id": "dst", "start_row": 0, "start_col": 1 }))?;
    assert_eq!(ds(&session, "dst").linked_regions.len(), 2);

    let mut s3 = session_with(
        "InsertShift",
        vec![
            col_sheet("s1", "S1", &[("A", &[1.0, 2.0])]),
            col_sheet("s2", "S2", &[("A", &[1.0, 2.0])]),
            col_sheet("dl", "DestLow", &[("A", &[0.0, 0.0, 0.0, 99.0, 0.0])]),
        ],
    );
    ok_cmd(&mut s3, "table.pasteLink", json!({ "source_sheet_id": "s1", "target_sheet_id": "dl", "start_row": 3, "start_col": 0 }))?;
    assert_err_code(
        &mut s3,
        "table.pasteLink",
        json!({ "source_sheet_id": "s2", "target_sheet_id": "dl", "start_row": 0, "start_col": 0, "mode": "insert_rows" }),
        ErrorCode::Conflict,
    );
    Ok(())
}

#[test]
fn audit_99_standalone_power_analysis_ignores_source_info_sheet() -> TestResult {
    let mut doc = ProjectDocument::new("Audit 99")?;
    let mut info1 = InfoSheet::new_preseeded("info-1", "Study Info")?;
    info1.set_constant("Alpha", ConstantValue::Number(0.01), None)?;
    doc.add_info_sheet(info1)?;
    let mut d = col_sheet("col-1", "Data", &[("A", &[1.0, 2.0])]);
    d.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(d)?;

    let mut session = Session::from_document(doc);
    run_cmds(
        &mut session,
        &[
            (
                "stats.run",
                json!({ "sheet_id": "pow-standalone", "source_sheet_id": "col-1", "kind": "power_analysis", "family": "unpaired_t", "alpha": 0.05, "target_power": 0.8, "effect_size": 0.5 }),
            ),
            (
                "stats.run",
                json!({ "sheet_id": "pow-linked", "source_sheet_id": "col-1", "kind": "power_analysis", "info_sheet_id": "info-1", "family": "unpaired_t", "alpha": 0.05, "target_power": 0.8, "effect_size": 0.5 }),
            ),
            ("project.recalculate", json!({})),
        ],
    );
    assert!((rs(&session, "pow-standalone").lookup_scalar("NPerGroup", None).ok_or("n1")? - 64.0).abs() < 1.0);
    assert!((rs(&session, "pow-linked").lookup_scalar("NPerGroup", None).ok_or("n2")? - 96.0).abs() < 1.0);
    Ok(())
}

#[test]
fn audit_122_graph_set_series_patches_in_place_by_default() -> TestResult {
    let mut session = session_with(
        "Audit 122",
        vec![xy_sheet(
            "xy",
            "Multi",
            "X",
            &[1.0, 2.0, 3.0],
            &[("A", &[1.0, 2.0, 3.0]), ("B", &[2.0, 4.0, 6.0]), ("C", &[3.0, 6.0, 9.0]), ("D", &[4.0, 8.0, 12.0])],
        )],
    );
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g", "data_sheet_id": "xy", "kind": "xy" }))?;
    let orig_eb1 = gs(&session, "g").spec.series[1].y_error_bars;

    let res = ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series": [{ "y_error_bars": { "metric": "Sem" } }] }))?;
    assert_eq!(
        (
            res["series_count"].as_u64(),
            gs(&session, "g").spec.series.len(),
            gs(&session, "g").spec.series[0].y_error_bars.map(|e| e.metric),
            gs(&session, "g").spec.series[1].y_error_bars
        ),
        (Some(4), 4, Some(ErrorBarMetric::Sem), orig_eb1)
    );

    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series": [{ "series_index": 2, "legend_label": "Target Two" }] }))?;
    assert_eq!(
        (gs(&session, "g").spec.series[2].legend_label.as_deref(), gs(&session, "g").spec.series[0].legend_label.as_deref()),
        (Some("Target Two"), None)
    );
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "series": [{ "data_set_index": 3, "legend_label": "Target Three" }] }))?;
    assert_eq!(gs(&session, "g").spec.series[3].legend_label.as_deref(), Some("Target Three"));

    assert_bad_inputs(
        &mut session,
        &[
            (
                "graph.setSeries",
                json!({ "sheet_id": "g", "series": [{ "color": "#111111" }, { "color": "#222222" }, { "color": "#333333" }, { "color": "#444444" }, { "color": "#555555" }] }),
            ),
            ("graph.setSeries", json!({ "sheet_id": "g", "series": [{ "series_index": 9, "color": "#111111" }] })),
        ],
    );

    ok_cmd(
        &mut session,
        "graph.setSeries",
        json!({ "sheet_id": "g", "series": [{ "legend_label": "S0" }, { "legend_label": "S1" }, { "legend_label": "S2" }, { "legend_label": "S3" }] }),
    )?;
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "all_series": true, "bind_to_right_y2": true }))?;
    assert!(gs(&session, "g").spec.series.len() == 4 && gs(&session, "g").spec.series.iter().all(|s| s.bind_to_right_y2));

    let rep = ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g", "mode": "replace", "series": [{ "data_set_index": 0 }] }))?;
    assert_eq!((rep["series_count"].as_u64(), gs(&session, "g").spec.series.len()), (Some(1), 1));
    Ok(())
}

#[test]
fn audit_74_identity_conversion_preserves_single_link_copy() -> TestResult {
    let mut s =
        session_with("Identity Link", vec![col_sheet("up", "Up", &[("U", &[1.0, 2.0, 3.0])]), col_sheet("dst", "Dst", &[("D", &[0.0, 0.0, 0.0])])]);
    ok_cmd(
        &mut s,
        "table.pasteLink",
        json!({ "source_sheet_id": "up", "target_sheet_id": "dst", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 2, "col": 0 } }, "start_row": 0, "start_col": 0, "link_id": "id-link" }),
    )?;
    assert_eq!(
        ok_cmd(&mut s, "table.convertArchetype", json!({ "sheet_id": "dst", "target_kind": "Column", "preview_only": true }))?["preview"]["lossy"],
        json!(false)
    );
    ok_cmd(&mut s, "table.convertArchetype", json!({ "sheet_id": "dst", "target_kind": "Column" }))?;
    assert_eq!((ds(&s, "dst").linked_regions.len(), ds(&s, "dst").linked_regions[0].id.as_str()), (1, "id-link"));
    Ok(())
}

#[test]
fn audit_74_grouped_replicates_expands_by_rows_and_detaches_transposed_link() -> TestResult {
    let grp = grouped_rep_sheet(
        "grp",
        "Grouped",
        &["R0", "R1", "R2"],
        &[("A", &[&[1.0, 2.0], &[3.0, 4.0], &[5.0, 6.0]]), ("B", &[&[100.0, 101.0], &[102.0, 103.0], &[104.0, 105.0]])],
    );
    let mut s = session_with("Grouped Expand", vec![grp, col_sheet("grp_copy", "Copy", &[("A", &[]), ("B", &[])])]);
    run_cmds(
        &mut s,
        &[
            (
                "table.pasteLink",
                json!({ "source_sheet_id": "grp", "target_sheet_id": "grp_copy", "source_range": { "start": { "row": 0, "col": 0 }, "end": { "row": 2, "col": 1 } }, "start_row": 0, "start_col": 0, "link_id": "grp-out" }),
            ),
            ("stats.run", json!({ "sheet_id": "grp_desc", "source_sheet_id": "grp", "kind": "descriptive_stats", "selected_data_sets": [1] })),
            ("table.convertArchetype", json!({ "sheet_id": "grp", "target_kind": "Column", "allow_lossy": true })),
        ],
    );
    assert_eq!(
        (ds(&s, "grp").archetype.column_count(), &rs(&s, "grp_desc").status, rs(&s, "grp_desc").analysis.selected_data_sets.as_slice()),
        (6, &ResultStatus::UpToDate, [3, 4, 5].as_slice())
    );
    assert!(ds(&s, "grp_copy").linked_regions.is_empty());
    Ok(())
}

#[test]
fn audit_74_xy_nonmatching_parametric_keeps_y_column_indices() -> TestResult {
    let mut xyp = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSdN, 2, 3)?;
    for r in 0..3 {
        xyp.x_cells[r][0] = Cell::number((r + 1) as f64);
        xyp.data_sets[0].cells[r] = vec![Cell::number(10.0 + r as f64), Cell::number(1.0), Cell::number(3.0)];
        xyp.data_sets[1].cells[r] = vec![Cell::number(50.0 + r as f64), Cell::number(2.0), Cell::number(3.0)];
    }
    let mut s =
        session_with("XY Param", vec![DataSheet::new("xyp", "XY Param", TableArchetype::Xy(xyp))?, col_sheet("xyp_copy", "Copy", &[("C", &[])])]);
    run_cmds(
        &mut s,
        &[
            (
                "table.pasteLink",
                json!({ "source_sheet_id": "xyp", "target_sheet_id": "xyp_copy", "source_range": { "start": { "row": 0, "col": 1 }, "end": { "row": 2, "col": 1 } }, "start_row": 0, "start_col": 0, "link_id": "xyp-out" }),
            ),
            ("stats.run", json!({ "sheet_id": "xyp_desc", "source_sheet_id": "xyp", "kind": "descriptive_stats", "selected_data_sets": [1] })),
            ("graph.create", json!({ "sheet_id": "g_xyp", "data_sheet_id": "xyp", "kind": "xy" })),
            ("table.convertArchetype", json!({ "sheet_id": "xyp", "target_kind": "Column", "allow_lossy": true })),
        ],
    );
    assert_eq!(
        (ds(&s, "xyp").archetype.column_count(), &rs(&s, "xyp_desc").status, rs(&s, "xyp_desc").analysis.selected_data_sets.as_slice()),
        (2, &ResultStatus::UpToDate, [1].as_slice())
    );
    assert_eq!((gs(&s, "g_xyp").spec.series[0].data_set_index, gs(&s, "g_xyp").spec.series[1].data_set_index), (0, 1));
    assert!(
        matches!(ds(&s, "xyp_copy").linked_regions[0].spec.source, LinkSource::InternalSheet { range, .. } if range.start.col == 1 && range.end.col == 1)
    );
    Ok(())
}

#[test]
fn audit_74_mv_contingency_non_crosstab_maps_sequential_data_indices() -> TestResult {
    let mv1 = mv_sheet(
        "mv",
        "MV",
        &[],
        &[
            ("Strain", Some(VariableRole::Label), &[Cell::text("WT"), Cell::text("KO")]),
            ("Dose", Some(VariableRole::Categorical), &[Cell::number(1.0), Cell::number(2.0)]),
            ("Count", Some(VariableRole::Continuous), &[Cell::number(15.0), Cell::number(25.0)]),
        ],
    );
    let mv2 = mv_sheet(
        "mv2",
        "MV2",
        &[],
        &[
            ("L0", Some(VariableRole::Label), &[Cell::empty(), Cell::empty()]),
            ("L1", Some(VariableRole::Label), &[Cell::empty(), Cell::empty()]),
            ("Cat", Some(VariableRole::Categorical), &[Cell::number(1.0), Cell::number(2.0)]),
            ("Val", Some(VariableRole::Continuous), &[Cell::number(10.0), Cell::number(20.0)]),
        ],
    );
    let mut doc = ProjectDocument::new("MV Contingency")?;
    doc.add_data_sheet(mv1)?;
    doc.add_data_sheet(mv2)?;
    let norm = || {
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::FixedValue(100.0),
            output_percent: true,
            per_data_set: false,
        })
    };
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec { selected_data_sets: vec![1, 2], ..AnalysisSpec::new("mv_norm", "mv", norm())? })?)?;
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec { selected_data_sets: vec![2, 3], ..AnalysisSpec::new("mv2_norm", "mv2", norm())? })?)?;

    let mut s = Session::from_document(doc);
    ok_cmd(&mut s, "table.convertArchetype", json!({ "sheet_id": "mv", "target_kind": "Contingency", "allow_lossy": true }))?;
    ok_cmd(&mut s, "table.convertArchetype", json!({ "sheet_id": "mv2", "target_kind": "Contingency", "allow_lossy": true }))?;
    assert_eq!((&rs(&s, "mv_norm").status, rs(&s, "mv_norm").analysis.selected_data_sets.as_slice()), (&ResultStatus::UpToDate, [0, 1].as_slice()));
    assert_eq!((&rs(&s, "mv2_norm").status, rs(&s, "mv2_norm").analysis.selected_data_sets.as_slice()), (&ResultStatus::UpToDate, [2, 3].as_slice()));
    Ok(())
}

#[test]
fn audit_74_broken_dependents_and_cross_sheet_hooks_stay_failed_on_recalc() -> TestResult {
    let mut doc = ProjectDocument::new("Stay Failed")?;
    doc.add_data_sheet(col_sheet("ds_a", "A", &[("10", &[1.0, 2.0, 3.0]), ("20", &[20.0, 21.0, 22.0]), ("30", &[30.0, 31.0, 32.0])]))?;
    doc.add_data_sheet(col_sheet("ds_b", "B", &[("B0", &[5.0, 6.0, 7.0]), ("B1", &[10.0, 11.0, 12.0])]))?;
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec {
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
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new(
        "ctrl_a",
        "ds_a",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::ControlColumnMean { column: 0 },
            hundred_anchor: NormalizeAnchor::ColumnMax,
            output_percent: true,
            per_data_set: false,
        }),
    )?)?)?;
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new(
        "base_a",
        "ds_a",
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Difference }),
    )?)?)?;

    let mut s = Session::from_document(doc);
    run_cmds(
        &mut s,
        &[("project.recalculate", json!({})), ("table.convertArchetype", json!({ "sheet_id": "ds_a", "target_kind": "XY", "allow_lossy": true }))],
    );
    assert_eq!(rs(&s, "hook_b").analysis.selected_data_sets, vec![0, 1]);
    for rid in ["hook_b", "ctrl_a", "base_a"] {
        assert!(matches!(rs(&s, rid).status, ResultStatus::Failed { .. }));
    }
    assert_eq!(ok_cmd(&mut s, "project.recalculate", json!({}))?["failed_count"], json!(3));
    for rid in ["hook_b", "ctrl_a", "base_a"] {
        assert!(matches!(rs(&s, rid).status, ResultStatus::Failed { .. }));
    }
    Ok(())
}
