#![forbid(unsafe_code)]

use std::error::Error;

use curvebook_doc::{
    AxisSpec, Cell, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, ErrorCode, GraphAnnotation, GraphSheet, InfoSheet, OutlierSpec,
    ProjectDocument, RectPt, SheetId, SurvivalTable, TableArchetype,
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

fn seed_three_group_column_session() -> Result<Session, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("RV82")?;
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4)?;
    col.data_sets[0].title = String::from("Vehicle");
    col.data_sets[1].title = String::from("Low Dose");
    col.data_sets[2].title = String::from("High Dose");
    for (r, (v0, v1, v2)) in [(10.0, 20.0, 40.0), (11.0, 21.0, 42.0), (9.5, 19.5, 41.0), (10.5, 20.5, 43.0)].into_iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v0);
        col.data_sets[1].cells[r][0] = Cell::number(v1);
        col.data_sets[2].cells[r][0] = Cell::number(v2);
    }
    let ds = DataSheet::new("col-1", "Groups", TableArchetype::Column(col))?;
    doc.add_data_sheet(ds)?;
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "anova-1", "source_sheet": "col-1", "kind": "one_way_anova", "multiplicity": "tukey" }))?;
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-1", "kind": "column" }))?;
    Ok(session)
}

#[test]
fn rv82_comparison_brackets_reject_partial_endpoints_and_dedup_upserts() -> TestResult {
    let mut session = seed_three_group_column_session()?;

    // 1. Single endpoint without partner must fail with BadInput (never fall back to add-all).
    let err_a_only =
        err_cmd(&mut session, "graph.addComparisonBrackets", json!({ "sheet_id": "g-col", "result_sheet_id": "anova-1", "group_a": "Vehicle" }))?;
    assert_eq!(err_a_only.code, ErrorCode::BadInput);
    let g = session.document().graph_sheet(&SheetId::new("g-col")).ok_or("missing g-col")?;
    assert!(g.spec.comparisons.is_empty());

    let err_b_only =
        err_cmd(&mut session, "graph.addComparisonBrackets", json!({ "sheet_id": "g-col", "result_sheet_id": "anova-1", "group_b": "High Dose" }))?;
    assert_eq!(err_b_only.code, ErrorCode::BadInput);
    let g = session.document().graph_sheet(&SheetId::new("g-col")).ok_or("missing g-col")?;
    assert!(g.spec.comparisons.is_empty());

    // Same group for both ends is rejected.
    let err_same = err_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({ "sheet_id": "g-col", "result_sheet_id": "anova-1", "group_a": "Vehicle", "group_b": "Vehicle" }),
    )?;
    assert_eq!(err_same.code, ErrorCode::BadInput);

    // 2. Repeated additions for the same pair without `clear_existing: true` upsert in place instead of stacking duplicates.
    let p1 = ok_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({
            "sheet_id": "g-col",
            "result_sheet_id": "anova-1",
            "group_a": "Vehicle",
            "group_b": "High Dose",
            "style": "flat_line",
            "label_mode": "stars"
        }),
    )?;
    assert_eq!(p1["added"], json!(1));
    assert_eq!(p1["updated"], json!(0));
    assert_eq!(p1["total_brackets"], json!(1));

    let p2 = ok_cmd(
        &mut session,
        "graph.addComparisonBrackets",
        json!({
            "sheet_id": "g-col",
            "result_sheet_id": "anova-1",
            "group_a": "High Dose",
            "group_b": "Vehicle",
            "style": "curly",
            "label_mode": "numeric"
        }),
    )?;
    assert_eq!(p2["added"], json!(0));
    assert_eq!(p2["updated"], json!(1));
    assert_eq!(p2["total_brackets"], json!(1));

    // Add-all twice updates existing bracket and adds missing pairs without duplicates.
    let p3 =
        ok_cmd(&mut session, "graph.addComparisonBrackets", json!({ "sheet_id": "g-col", "result_sheet_id": "anova-1", "significant_only": false }))?;
    assert_eq!(p3["total_brackets"], json!(3));
    let p4 =
        ok_cmd(&mut session, "graph.addComparisonBrackets", json!({ "sheet_id": "g-col", "result_sheet_id": "anova-1", "significant_only": false }))?;
    assert_eq!(p4["added"], json!(0));
    assert_eq!(p4["updated"], json!(3));
    assert_eq!(p4["total_brackets"], json!(3));
    Ok(())
}

#[test]
fn rv83_info_set_constant_preserves_units_when_omitted_and_clears_explicitly() -> TestResult {
    let mut doc = ProjectDocument::new("RV83")?;
    let info = InfoSheet::new_preseeded("info-1", "Assay Info")?;
    doc.add_info_sheet(info)?;
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-1", "name": "Kd", "value": 12.5, "units": "nM" }))?;
    assert_eq!(
        session.document().info_sheet(&SheetId::new("info-1")).and_then(|i| i.get_constant("Kd")).and_then(|c| c.units.as_deref()),
        Some("nM")
    );

    // Omitting `units` on update must preserve existing "nM".
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-1", "name": "Kd", "value": 13.0 }))?;
    let c = session.document().info_sheet(&SheetId::new("info-1")).and_then(|i| i.get_constant("Kd")).ok_or("missing Kd")?;
    assert_eq!(c.value, ConstantValue::Number(13.0));
    assert_eq!(c.units.as_deref(), Some("nM"));

    // Explicit `null` clears `units`.
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-1", "name": "Kd", "value": 14.0, "units": null }))?;
    assert_eq!(session.document().info_sheet(&SheetId::new("info-1")).and_then(|i| i.get_constant("Kd")).and_then(|c| c.units.as_deref()), None);

    // Explicit empty string clears `units` as well.
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-1", "name": "Kd", "value": 15.0, "units": "uM" }))?;
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-1", "name": "Kd", "value": 16.0, "units": "  " }))?;
    assert_eq!(session.document().info_sheet(&SheetId::new("info-1")).and_then(|i| i.get_constant("Kd")).and_then(|c| c.units.as_deref()), None);
    Ok(())
}

#[test]
fn rv84_layout_set_spec_validates_geometry_and_matches_shorthand() -> TestResult {
    let mut session = Session::new();
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1", "grid_template": [1, 2] }))?;

    for bad_spec in [
        json!({ "width": 0.0 }),
        json!({ "height": -5.0 }),
        json!({ "width": 0.005 }),
        json!({ "grid_template": [0, 2] }),
        json!({
            "items": [{
                "TextCallout": {
                    "id": "t1",
                    "markdown": "Hello",
                    "bounds_pt": { "x": 0.0, "y": 0.0, "width": 0.0, "height": 20.0 },
                    "font_size_pt": 10.0
                }
            }]
        }),
        json!({
            "items": [{
                "TextCallout": {
                    "id": "t1",
                    "markdown": "Hello",
                    "bounds_pt": { "x": 0.0, "y": 0.0, "width": 80.0, "height": 20.0 },
                    "font_size_pt": 0.1
                }
            }]
        }),
    ] {
        let err = err_cmd(&mut session, "layout.setSpec", json!({ "sheet_id": "lay-1", "spec": bad_spec }))?;
        assert_eq!(err.code, ErrorCode::BadInput);
    }

    // Valid spec update and preset application work cleanly.
    ok_cmd(&mut session, "layout.setSpec", json!({ "sheet_id": "lay-1", "spec": { "width": 120.0, "height": 90.0 } }))?;
    ok_cmd(&mut session, "layout.applyCanvasPreset", json!({ "sheet_id": "lay-1", "preset": "SingleColumn88mm" }))?;
    Ok(())
}

#[test]
fn rv85_point_overrides_range_check_and_prune_on_series_and_row_shorten() -> TestResult {
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d1", "kind": "xy", "rows": 4, "cols": 2 }))?;
    ok_cmd(&mut session, "table.setXCells", json!({ "sheet_id": "d1", "values": [1.0, 2.0, 3.0, 4.0] }))?;
    ok_cmd(&mut session, "table.setCells", json!({ "sheet_id": "d1", "rows": [[10.0, 20.0], [11.0, 21.0], [12.0, 22.0], [13.0, 23.0]] }))?;
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d2", "kind": "xy", "rows": 2, "cols": 1 }))?;
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "d1", "kind": "xy" }))?;

    // Out-of-bounds series_index, point_index, and subcol must fail with BadInput.
    let err_series = err_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g1", "points": [{ "series_index": 99, "point_index": 0 }], "color": "#FF0000" }),
    )?;
    assert_eq!(err_series.code, ErrorCode::BadInput);

    let err_point = err_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g1", "points": [{ "series_index": 0, "point_index": 99 }], "color": "#FF0000" }),
    )?;
    assert_eq!(err_point.code, ErrorCode::BadInput);

    let err_subcol = err_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g1", "points": [{ "series_index": 0, "point_index": 0, "subcol": 3 }], "color": "#FF0000" }),
    )?;
    assert_eq!(err_subcol.code, ErrorCode::BadInput);

    // Also validate point_overrides supplied via graph.setSpec.
    let err_spec = err_cmd(
        &mut session,
        "graph.setSpec",
        json!({ "sheet_id": "g1", "spec": { "point_overrides": [{ "series_index": 42, "point_index": 0, "hidden": true }] } }),
    )?;
    assert_eq!(err_spec.code, ErrorCode::BadInput);

    // Format points across series 0 and series 1.
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({
            "sheet_id": "g1",
            "points": [
                { "series_index": 0, "point_index": 1 },
                { "series_index": 0, "point_index": 3 },
                { "series_index": 1, "point_index": 2 }
            ],
            "color": "#0072B2"
        }),
    )?;
    assert_eq!(session.document().graph_sheet(&SheetId::new("g1")).map_or(0, |g| g.spec.point_overrides.len()), 3);

    // Removing series 1 via graph.setSeries (`mode: "replace"`) prunes override for series 1 immediately.
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g1", "mode": "replace", "series": [{ "data_set_index": 0 }] }))?;
    let overrides = &session.document().graph_sheet(&SheetId::new("g1")).ok_or("missing g1")?.spec.point_overrides;
    assert_eq!(overrides.len(), 2);
    assert!(overrides.iter().all(|ov| ov.series_index == 0));

    // Clearing trailing row 3 prunes point_index == 3 override while preserving point_index == 1.
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "d1", "row": 3, "col": 0 }))?;
    let overrides = &session.document().graph_sheet(&SheetId::new("g1")).ok_or("missing g1")?.spec.point_overrides;
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].point_index, 1);

    // Replacing series source sheet from d1 -> d2 prunes stale overrides targeting d1.
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g1", "series": [{ "source_sheet_id": "d2", "data_set_index": 0 }] }))?;
    assert!(
        session.document().graph_sheet(&SheetId::new("g1")).ok_or("missing g1")?.spec.point_overrides.is_empty(),
        "switching series source sheet must prune overrides referencing the previous table"
    );

    // Structural row-count shrink via importCsv { replace: true } prunes out-of-range overrides.
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g1", "points": [{ "series_index": 0, "point_index": 1 }], "hidden": true }),
    )?;
    ok_cmd(&mut session, "table.importCsv", json!({ "sheet_id": "d2", "replace": true, "archetype": "xy", "csv": "X,Y\n1,5\n" }))?;
    assert!(
        session.document().graph_sheet(&SheetId::new("g1")).ok_or("missing g1")?.spec.point_overrides.is_empty(),
        "structural row shortening via importCsv replace must prune out-of-bounds overrides"
    );
    Ok(())
}

#[test]
fn rv86_cascade_delete_preserves_info_used_by_surviving_power_graph_or_layout() -> TestResult {
    // 1. Graph annotation hook referencing paired info sheet blocks info deletion when cascading d1.
    let mut sess_graph = Session::new();
    ok_cmd(&mut sess_graph, "sheet.createData", json!({ "sheet_id": "d1", "kind": "column", "auto_info": true }))?;
    ok_cmd(&mut sess_graph, "sheet.createData", json!({ "sheet_id": "d2", "kind": "column" }))?;
    ok_cmd(&mut sess_graph, "graph.create", json!({ "sheet_id": "g2", "data_sheet_id": "d2", "kind": "column" }))?;
    let info1_id = sess_graph
        .document()
        .data_sheet(&SheetId::new("d1"))
        .and_then(|d| d.info_sheet_id.as_ref())
        .map(|s| s.as_str().to_owned())
        .ok_or("missing info1")?;
    ok_cmd(
        &mut sess_graph,
        "graph.addAnnotation",
        json!({ "sheet_id": "g2", "kind": "text", "text": format!("Exp {{{{info:{info1_id}:Experiment ID}}}}"), "x_pt": 20.0, "y_pt": 20.0 }),
    )?;
    ok_cmd(&mut sess_graph, "sheet.delete", json!({ "sheet_id": "d1", "cascade": true }))?;
    assert!(sess_graph.document().data_sheet(&SheetId::new("d1")).is_none());
    assert!(sess_graph.document().info_sheet(&SheetId::new(&info1_id)).is_some(), "surviving graph hook must protect info sheet");

    // 2. Surviving Layout text callout protects paired info sheet on cascade delete.
    let mut sess_layout = Session::new();
    ok_cmd(&mut sess_layout, "sheet.createData", json!({ "sheet_id": "d1", "kind": "column", "auto_info": true }))?;
    let info_l_id = sess_layout
        .document()
        .data_sheet(&SheetId::new("d1"))
        .and_then(|d| d.info_sheet_id.as_ref())
        .map(|s| s.as_str().to_owned())
        .ok_or("missing info_l")?;
    ok_cmd(&mut sess_layout, "layout.create", json!({ "sheet_id": "lay-keep" }))?;
    ok_cmd(
        &mut sess_layout,
        "layout.addPanel",
        json!({ "sheet_id": "lay-keep", "kind": "text", "markdown": format!("Proto={{{{info:{info_l_id}:Protocol}}}}") }),
    )?;
    ok_cmd(&mut sess_layout, "sheet.delete", json!({ "sheet_id": "d1", "cascade": true }))?;
    assert!(sess_layout.document().info_sheet(&SheetId::new(&info_l_id)).is_some(), "surviving layout callout must protect info sheet");

    // 3. Surviving PowerAnalysis info_sheet_id protects paired info sheet on cascade delete.
    let mut sess_power = Session::new();
    ok_cmd(&mut sess_power, "sheet.createData", json!({ "sheet_id": "d1", "kind": "column", "auto_info": true }))?;
    ok_cmd(&mut sess_power, "sheet.createData", json!({ "sheet_id": "d2", "kind": "column" }))?;
    let info_p_id = sess_power
        .document()
        .data_sheet(&SheetId::new("d1"))
        .and_then(|d| d.info_sheet_id.as_ref())
        .map(|s| s.as_str().to_owned())
        .ok_or("missing info_p")?;
    ok_cmd(
        &mut sess_power,
        "stats.run",
        json!({
            "sheet_id": "pwr-1",
            "source_sheet": "d2",
            "kind": "power",
            "alpha": 0.05,
            "target_power": 0.8,
            "effect_size": 0.5,
            "info_sheet_id": info_p_id
        }),
    )?;
    ok_cmd(&mut sess_power, "sheet.delete", json!({ "sheet_id": "d1", "cascade": true }))?;
    assert!(sess_power.document().info_sheet(&SheetId::new(&info_p_id)).is_some(), "surviving Power analysis must protect info sheet");
    Ok(())
}

#[test]
fn rv87_bare_info_hook_tracking_and_rendering_paired_first() -> TestResult {
    let mut doc = ProjectDocument::new("RV87")?;

    let mut info1 = InfoSheet::new_preseeded("info-1", "First Info")?;
    info1.set_constant("Tag", ConstantValue::Text(String::from("FIRST")), None)?;
    let mut info2 = InfoSheet::new_preseeded("info-2", "Paired Info")?;
    info2.set_constant("Tag", ConstantValue::Text(String::from("SECOND")), None)?;
    doc.add_info_sheet(info1)?;
    doc.add_info_sheet(info2)?;

    let mut d1 = DataSheet::new("d1", "Table 1", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?))?;
    d1.info_sheet_id = Some(SheetId::new("info-1"));
    doc.add_data_sheet(d1)?;

    let mut surv = SurvivalTable::new(1, 2)?;
    surv.times[0] = Cell::number(5.0);
    surv.times[1] = Cell::number(10.0);
    surv.cohorts[0].cells[0][0] = Cell::number(1.0);
    surv.cohorts[0].cells[1][0] = Cell::number(1.0);
    let mut d2 = DataSheet::new("d2", "Survival Paired", TableArchetype::Survival(surv))?;
    d2.info_sheet_id = Some(SheetId::new("info-2"));
    doc.add_data_sheet(d2.clone())?;

    let mut gs = GraphSheet::new_uninitialized_for_data("g-surv", &d2)?;
    gs.initialized = true;
    gs.spec.title = String::from("Cohort {{info:Tag}}");
    gs.spec.x_axis = AxisSpec::linear("Time ({{info:Tag}})");
    gs.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 100.0, 24.0),
        markdown: String::from("Note={{info:Tag}}"),
        bordered: false,
    });
    doc.add_graph_sheet(gs)?;

    // 1. Dependency tracking uses paired `info-2`, not `info-1`.
    let deps = doc.upstream_dependencies(&SheetId::new("g-surv"));
    assert!(deps.contains(&SheetId::new("info-2")), "expected info-2 in deps: {deps:?}");
    assert!(!deps.contains(&SheetId::new("info-1")), "unexpected info-1 in deps: {deps:?}");

    // 2. Rendering across title, survival axis title, and annotation expands `SECOND` from `info-2`.
    let session = Session::from_document(doc);
    let scene = session.render_sheet_scene(&SheetId::new("g-surv"))?;
    let rendered_texts: Vec<String> = scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            curvebook_chart::ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect();
    assert!(rendered_texts.iter().any(|t| t == "Cohort SECOND"), "title texts: {rendered_texts:?}");
    assert!(rendered_texts.iter().any(|t| t == "Time (SECOND)"), "axis texts: {rendered_texts:?}");
    assert!(rendered_texts.iter().any(|t| t == "Note=SECOND"), "annotation texts: {rendered_texts:?}");
    Ok(())
}

#[test]
fn rv88_shorthand_analysis_params_reject_wrong_json_types() -> TestResult {
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d-col", "kind": "column", "rows": 6, "cols": 1 }))?;
    for (r, v) in [10.0, 11.0, 10.5, 11.2, 10.8, 99.0].into_iter().enumerate() {
        ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "d-col", "row": r, "col": 0, "value": v }))?;
    }
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d-xy", "kind": "xy", "rows": 6, "cols": 1 }))?;
    ok_cmd(&mut session, "table.setXCells", json!({ "sheet_id": "d-xy", "values": [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] }))?;
    for (r, v) in [2.0, 4.0, 6.0, 8.0, 10.0, 50.0].into_iter().enumerate() {
        ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "d-xy", "row": r, "col": 0, "value": v }))?;
    }

    for (cmd, payload) in [
        ("stats.run", json!({ "source_sheet": "d-col", "kind": "grubbs", "alpha": "0.01" })),
        ("stats.run", json!({ "source_sheet": "d-col", "kind": "grubbs", "iterative": "true" })),
        ("stats.run", json!({ "source_sheet": "d-col", "kind": "rout", "q_percent": "5" })),
        ("stats.run", json!({ "source_sheet": "d-col", "kind": "one_sample_t", "hypothetical": "10" })),
        ("stats.run", json!({ "source_sheet": "d-col", "kind": "power", "alpha": "0.05", "target_power": 0.8, "effect_size": 0.5 })),
        ("transform.run", json!({ "source_sheet": "d-xy", "kind": "prune_rows", "every_k_rows": "2" })),
        ("transform.run", json!({ "source_sheet": "d-xy", "kind": "prune_rows", "average_window": "yes" })),
        ("transform.run", json!({ "source_sheet": "d-xy", "kind": "simulate", "x_min": "-9" })),
    ] {
        let err = err_cmd(&mut session, cmd, payload.clone())?;
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for {payload}");
    }

    // Valid numeric types take effect rather than defaulting.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "g-ok", "source_sheet": "d-col", "kind": "grubbs", "alpha": 0.01 }))?;
    let res = session.document().result_sheet(&SheetId::new("g-ok")).ok_or("missing g-ok")?;
    assert!(matches!(
        res.analysis.kind,
        curvebook_doc::AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, .. }) if (alpha - 0.01).abs() < 1e-12
    ));

    // `analysis.updateParams` also rejects wrong-type shorthand params in `params`.
    let err_upd = err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "g-ok", "params": { "alpha": "0.05" } }))?;
    assert_eq!(err_upd.code, ErrorCode::BadInput);
    Ok(())
}

#[test]
fn rv89_sheet_id_rejects_hook_delimiters_whitespace_control_and_overlong() -> TestResult {
    let mut session = Session::new();
    let long_id = "s".repeat(257);
    for bad_id in ["fit:1", "bad}}id", "bad{{id", "has space", "ctrl\nchar", long_id.as_str()] {
        let err = err_cmd(&mut session, "sheet.createData", json!({ "sheet_id": bad_id, "kind": "column" }))?;
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for sheet_id `{bad_id}`");
    }
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "valid_sheet-1.ok", "kind": "column" }))?;
    assert!(session.document().data_sheet(&SheetId::new("valid_sheet-1.ok")).is_some());
    Ok(())
}

#[test]
fn rv90_info_linked_data_sheets_maintained_across_create_duplicate_and_delete() -> TestResult {
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d1", "kind": "column", "auto_info": true }))?;
    let info_id = session.document().data_sheet(&SheetId::new("d1")).and_then(|d| d.info_sheet_id.clone()).ok_or("missing info_id")?;
    assert_eq!(session.document().info_sheet(&info_id).map(|i| i.linked_data_sheets.as_slice()), Some([SheetId::new("d1")].as_slice()));

    // Sharing info sheet on second created table adds d2 without duplicating d1.
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d2", "kind": "column", "info_sheet_id": info_id.as_str() }))?;
    // Duplicating d1 records d1-copy on the shared info sheet.
    ok_cmd(&mut session, "sheet.duplicate", json!({ "sheet_id": "d1", "new_sheet_id": "d1-copy" }))?;
    let linked = &session.document().info_sheet(&info_id).ok_or("missing info")?.linked_data_sheets;
    assert_eq!(linked, &vec![SheetId::new("d1"), SheetId::new("d2"), SheetId::new("d1-copy")]);

    // Duplicating standalone InfoSheet starts with empty linked_data_sheets.
    ok_cmd(&mut session, "sheet.duplicate", json!({ "sheet_id": info_id.as_str(), "new_sheet_id": "info-standalone" }))?;
    assert!(session.document().info_sheet(&SheetId::new("info-standalone")).ok_or("missing info-standalone")?.linked_data_sheets.is_empty());

    // Duplicating family creates new cloned info linked only to the cloned data sheet.
    let fam_out = ok_cmd(&mut session, "sheet.duplicateFamily", json!({ "sheet_id": "d1", "prefix": "fam2" }))?;
    let fam_data = fam_out["sheet_id"].as_str().ok_or("missing fam2 data")?;
    let fam_info = fam_out["info_sheet_id"].as_str().ok_or("missing fam2 info")?;
    assert_eq!(
        session.document().info_sheet(&SheetId::new(fam_info)).map(|i| i.linked_data_sheets.as_slice()),
        Some([SheetId::new(fam_data)].as_slice())
    );

    // Deleting d1-copy removes it from `linked_data_sheets` cleanly.
    ok_cmd(&mut session, "sheet.delete", json!({ "sheet_id": "d1-copy" }))?;
    let after_del = &session.document().info_sheet(&info_id).ok_or("missing info")?.linked_data_sheets;
    assert_eq!(after_del, &vec![SheetId::new("d1"), SheetId::new("d2")]);
    Ok(())
}

#[test]
fn s4_checkpoint_timestamp_and_overlay_and_keep_typography_floor() -> TestResult {
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "d-xy", "kind": "xy", "rows": 4, "cols": 1 }))?;
    ok_cmd(&mut session, "table.setXCells", json!({ "sheet_id": "d-xy", "values": [1.0, 2.0, 3.0, 4.0] }))?;
    ok_cmd(&mut session, "table.setCells", json!({ "sheet_id": "d-xy", "rows": [[2.0], [4.0], [6.0], [8.0]] }))?;
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "fit-lin", "source_sheet": "d-xy", "kind": "linear" }))?;
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "d-xy", "kind": "xy" }))?;

    // Checkpoint timestamp_utc rejects non-ISO / invalid calendar timestamps while accepting UTC ISO-8601 & unix:<secs>.
    for bad_ts in ["not-a-timestamp", "2026-02-30T00:00:00Z", "2026-13-01T00:00:00Z", "2026-11-17T25:00:00Z"] {
        let err = err_cmd(&mut session, "project.createCheckpoint", json!({ "name": "Bad", "timestamp_utc": bad_ts }))?;
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for `{bad_ts}`");
    }
    ok_cmd(&mut session, "project.createCheckpoint", json!({ "name": "ValidISO", "timestamp_utc": "2026-11-17T12:30:00Z" }))?;
    ok_cmd(&mut session, "project.createCheckpoint", json!({ "name": "ValidUnix", "timestamp_utc": "unix:1700000000" }))?;

    // Overlay segment count clamps to [256, 1024] per documented schema behavior.
    ok_cmd(
        &mut session,
        "graph.setRegressionOverlay",
        json!({ "sheet_id": "g-xy", "result_sheet_id": "fit-lin", "series_index": 0, "sample_segments": 32 }),
    )?;
    let ov_segs =
        session.document().graph_sheet(&SheetId::new("g-xy")).and_then(|g| g.spec.series[0].regression_overlay.as_ref()).map(|ov| ov.sample_segments);
    assert_eq!(ov_segs, Some(256));

    // KeepTypographyPt canvas resize enforces intentional 0.25 pt print stroke floor on thin positive strokes.
    ok_cmd(&mut session, "graph.setAxes", json!({ "sheet_id": "g-xy", "axis": "y", "stroke_width_pt": 0.1 }))?;
    ok_cmd(&mut session, "graph.applyCanvasPreset", json!({ "sheet_id": "g-xy", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }))?;
    let y_stroke = session.document().graph_sheet(&SheetId::new("g-xy")).map(|g| g.spec.y_axis.stroke_width_pt);
    assert_eq!(y_stroke, Some(0.25));
    Ok(())
}
