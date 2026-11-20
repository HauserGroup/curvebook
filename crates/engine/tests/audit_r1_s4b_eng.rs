//! Stage 4B S3 audit regressions: `R1-S07-12`, `R1-S07-13`, `R1-S07-14`, `R1-X2-09`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, BracketStyle, Cell, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphAnnotation,
    GraphSheet, HookSource, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, LinearRegressionSpec, NonlinearFitSpec, NormalizeAnchor,
    PairwiseBracketSpec, ParameterRule, ParameterScope, ProjectDocument, RectPt, RegressionOverlaySpec, ResultScalarEntry, ResultSheet, ResultStatus,
    SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XyTable,
};
use curvebook_engine::{CommandOutcome, Session, cmd, cmd::Effects, dag};
use serde_json::{Value, json};

fn ok(session: &mut Session, cmd_id: &str, params: Value) -> CommandOutcome {
    session.execute(cmd_id, params).unwrap_or_else(|e| panic!("command `{cmd_id}` failed: {e}"))
}

fn make_linear_xy(id: &str, title: &str, slope: f64, intercept: f64) -> DataSheet {
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(1), 1, 5).unwrap();
    xy.data_sets[0].title = String::from("10.0");
    for r in 0..5 {
        let x = (r + 1) as f64;
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(slope * x + intercept);
    }
    DataSheet::new(id, title, TableArchetype::Xy(xy)).unwrap()
}

fn scalar(doc: &ProjectDocument, res_id: &str, key: &str) -> Option<f64> {
    doc.result_sheet(&SheetId::new(res_id)).and_then(|r| r.scalars.iter().find(|s| s.key == key)).map(|s| s.value)
}

fn set_sentinel(doc: &mut ProjectDocument, res_id: &str, tag: &str) {
    let r = doc.result_sheet_mut(&SheetId::new(res_id)).unwrap();
    r.scalars.retain(|s| s.key != tag);
    r.scalars.push(ResultScalarEntry { key: String::from(tag), data_set_index: None, value: 98765.0, units: None });
}

fn has_sentinel(doc: &ProjectDocument, res_id: &str, tag: &str) -> bool {
    scalar(doc, res_id, tag).is_some()
}

fn cleanroom_pzfx_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile Version="2.0" Title="Cleanroom Assay Batch">
  <InfoSequence>
    <Info ID="info-study">
      <Title>Imported PZFX Info</Title>
      <Constant><Name>Protocol</Name><Value>ELISA-v3</Value></Constant>
      <Notes>Cleanroom generated XML project fixture.</Notes>
    </Info>
  </InfoSequence>
  <TableSequence>
    <Table ID="tbl-col" TableType="OneWay" Replicates="1">
      <Title>Imported PZFX Cohort</Title>
      <YColumn Width="80" Decimals="2" Subcolumns="1">
        <Title>Vehicle</Title>
        <Subcolumn><d>12.0</d><d>13.4</d><d>11.8</d><d>12.9</d></Subcolumn>
      </YColumn>
      <YColumn Width="80" Decimals="2" Subcolumns="1">
        <Title>Agonist</Title>
        <Subcolumn><d>24.5</d><d>26.1</d><d>25.2</d><d>27.0</d></Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#
}

fn setup_s07_12_session() -> Session {
    let mut doc = ProjectDocument::new("DAG Incremental").unwrap();

    // Paired Info for data-c.
    let mut info_c = InfoSheet::new_preseeded("info-c", "Info C").unwrap();
    info_c.set_constant("Scale", ConstantValue::Number(100.0), None).unwrap();
    info_c.set_constant("Multiplier", ConstantValue::Number(2.0), None).unwrap();
    info_c.linked_data_sheets.push(SheetId::new("data-c"));
    doc.add_info_sheet(info_c).unwrap();

    // data-a with linear fit res-a; data-b completely unrelated; data-c paired with info-c;
    // data-d linked downstream from data-c; data-title hooked by ColumnTitleValue on data-c.
    doc.add_data_sheet(make_linear_xy("data-a", "Table A", 2.0, 1.0)).unwrap();
    doc.add_data_sheet(make_linear_xy("data-b", "Table B", 3.0, 0.0)).unwrap();
    let mut ds_c = make_linear_xy("data-c", "Table C", 1.5, 0.5);
    ds_c.info_sheet_id = Some(SheetId::new("info-c"));
    doc.add_data_sheet(ds_c).unwrap();
    doc.add_data_sheet(make_linear_xy("data-d", "Table D", 0.0, 0.0)).unwrap();

    let res_a =
        ResultSheet::new(AnalysisSpec::new("res-a", "data-a", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap()).unwrap();
    doc.add_result_sheet(res_a).unwrap();

    // Unpinned InfoConstant { info_sheet_id: None } on res-c, plus chained res-c2 reading res-c.
    let res_c = ResultSheet::new(
        AnalysisSpec::new(
            "res-c",
            "data-c",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::FixedValue(0.0),
                hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Scale") }),
                output_percent: true,
                per_data_set: false,
            }),
        )
        .unwrap(),
    )
    .unwrap();
    doc.add_result_sheet(res_c).unwrap();

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.parameter_rules.push(ParameterRule {
        name: String::from("Slope"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Multiplier") }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-c2", "res-c", AnalysisKind::NonlinearRegression(nl)).unwrap()).unwrap()).unwrap();

    // res-title uses ColumnTitleValue from data-c column 0.
    let mut nl_title = NonlinearFitSpec::new("straight_line");
    nl_title.parameter_rules.push(ParameterRule {
        name: String::from("Slope"),
        scope: ParameterScope::HookedConstant(HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("data-c"), data_set_index: 0 }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-title", "data-c", AnalysisKind::NonlinearRegression(nl_title)).unwrap()).unwrap())
        .unwrap();

    // Initial full recalc, then add a live link data-c -> data-d + res-d via Session.
    dag::recalculate_project(&mut doc).unwrap();
    let mut session = Session::from_document(doc);
    ok(&mut session, "table.pasteLink", json!({ "source_sheet_id": "data-c", "target_sheet_id": "data-d" }));
    ok(&mut session, "analysis.add", json!({ "sheet_id": "res-d", "source_sheet": "data-d", "kind": "linear_regression" }));
    session
}

#[test]
fn r1_s07_12_dirty_incremental_recalc_closure_and_isolation() {
    let session = setup_s07_12_session();

    // Plant sentinels on every result sheet so any unnecessary recomputation wipes them out cleanly.
    let mut doc = session.document().clone();
    for rid in ["res-a", "res-c", "res-c2", "res-title", "res-d"] {
        set_sentinel(&mut doc, rid, "__untouched__");
    }
    let mut session = Session::from_document(doc);

    // 1. Edit unrelated table `data-b` -> ZERO sheets recalculated, every sentinel preserved!
    let edit_b = ok(&mut session, "table.setCell", json!({ "sheet_id": "data-b", "row": 0, "col": 0, "value": 99.0 }));
    assert_eq!(edit_b.payload["recalculated_sheets"], json!(0));
    assert_eq!(edit_b.payload["refreshed_links"], json!(0));
    for rid in ["res-a", "res-c", "res-c2", "res-title", "res-d"] {
        assert!(has_sentinel(session.document(), rid, "__untouched__"), "{rid} must NOT recompute when editing data-b");
    }

    // 2. Edit `data-a` -> ONLY `res-a` recomputes (recalculated_sheets == 1), all C/D sheets keep sentinels!
    let edit_a = ok(&mut session, "table.setCell", json!({ "sheet_id": "data-a", "row": 4, "col": 0, "value": 20.0 }));
    assert_eq!(edit_a.payload["recalculated_sheets"], json!(1));
    assert!(!has_sentinel(session.document(), "res-a", "__untouched__"));
    for rid in ["res-c", "res-c2", "res-title", "res-d"] {
        assert!(has_sentinel(session.document(), rid, "__untouched__"), "{rid} must NOT recompute when editing data-a");
    }

    // 3. Re-plant sentinel on `res-a`, edit `data-c` -> C chain + D link/result recalculate, `res-a` stays untouched!
    let mut doc = session.document().clone();
    set_sentinel(&mut doc, "res-a", "__untouched__");
    let mut session = Session::from_document(doc);

    let edit_c = ok(&mut session, "table.setCell", json!({ "sheet_id": "data-c", "row": 4, "col": 0, "value": 18.0 }));
    assert_eq!(edit_c.payload["recalculated_sheets"], json!(5), "expected data-d + res-c, res-c2, res-title, res-d");
    assert_eq!(edit_c.payload["refreshed_links"], json!(1), "expected data-d link refresh");
    assert!(has_sentinel(session.document(), "res-a", "__untouched__"), "res-a must NOT recompute when editing data-c");
    for rid in ["res-c", "res-c2", "res-title", "res-d"] {
        assert!(!has_sentinel(session.document(), rid, "__untouched__"), "{rid} should have recomputed");
    }
}

#[test]
fn r1_s07_12_info_title_freeze_and_explicit_force_recalc() {
    let session = setup_s07_12_session();

    // 4a. Unpaired InfoSheet (`info-standalone`) hooked ONLY by `res-standalone` -> recalculates ONLY 1 sheet!
    let mut info_extra = InfoSheet::new_preseeded("info-standalone", "Standalone Info").unwrap();
    info_extra.set_constant("SlopeOverride", ConstantValue::Number(4.0), None).unwrap();
    let mut nl_extra = NonlinearFitSpec::new("straight_line");
    nl_extra.parameter_rules.push(ParameterRule {
        name: String::from("Slope"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-standalone")),
            constant_name: String::from("SlopeOverride"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let res_extra = ResultSheet::new(AnalysisSpec::new("res-standalone", "data-b", AnalysisKind::NonlinearRegression(nl_extra)).unwrap()).unwrap();
    let mut doc = session.document().clone();
    doc.add_info_sheet(info_extra).unwrap();
    doc.add_result_sheet(res_extra).unwrap();
    dag::recalculate_project(&mut doc).unwrap();
    for rid in ["res-a", "res-c", "res-c2", "res-title", "res-d"] {
        set_sentinel(&mut doc, rid, "__untouched__");
    }
    let mut session = Session::from_document(doc);
    let edit_extra_info = ok(&mut session, "info.setConstant", json!({ "sheet_id": "info-standalone", "name": "SlopeOverride", "value": 7.0 }));
    assert_eq!(edit_extra_info.payload["recalculated_sheets"], json!(1), "only res-standalone should recompute");
    for rid in ["res-a", "res-c", "res-c2", "res-title", "res-d"] {
        assert!(has_sentinel(session.document(), rid, "__untouched__"), "{rid} must NOT recompute on info-standalone change");
    }

    // 4b. Edit paired `info-c` constant `Scale` -> propagates through paired `data-c` (`data-d` + C analyses = 5), keeping `res-a` untouched!
    let mut doc = session.document().clone();
    for rid in ["res-a", "res-standalone"] {
        set_sentinel(&mut doc, rid, "__untouched__");
    }
    let mut session = Session::from_document(doc);
    let edit_info = ok(&mut session, "info.setConstant", json!({ "sheet_id": "info-c", "name": "Scale", "value": 50.0 }));
    assert_eq!(edit_info.payload["recalculated_sheets"], json!(5), "paired info-c propagates to data-c's dependents");
    assert!(has_sentinel(session.document(), "res-a", "__untouched__"), "res-a must NOT recompute on info-c change");
    assert!(has_sentinel(session.document(), "res-standalone", "__untouched__"), "res-standalone must NOT recompute on info-c change");

    // 5. Update `data-c` column title -> `res-title` (plus any link/analysis on data-c). Specifically verify `res-a` is untouched!
    let edit_title = ok(&mut session, "table.setTitles", json!({ "sheet_id": "data-c", "titles": ["25.0"] }));
    assert!(edit_title.payload["recalculated_sheets"].as_u64().unwrap() >= 1);
    assert!(has_sentinel(session.document(), "res-a", "__untouched__"));
    assert!((scalar(session.document(), "res-title", "Slope").unwrap() - 25.0).abs() < 1e-6);

    // 6. Freezing `data-d` pauses link refresh & `res-d`; editing `data-c` still updates `res-c` while skipping `data-d`,
    //    and unfreezing catches `data-d` + `res-d` back up!
    ok(&mut session, "sheet.freeze", json!({ "sheet_id": "data-d" }));
    let slope_d_frozen = scalar(session.document(), "res-d", "Slope").unwrap();
    let edit_c_while_frozen = ok(&mut session, "table.setCell", json!({ "sheet_id": "data-c", "row": 4, "col": 0, "value": 42.0 }));
    assert_eq!(edit_c_while_frozen.payload["refreshed_links"], json!(0), "frozen data-d link must not refresh");
    assert!((scalar(session.document(), "res-d", "Slope").unwrap() - slope_d_frozen).abs() < 1e-12);

    let unfreeze = ok(&mut session, "sheet.unfreeze", json!({ "sheet_id": "data-d" }));
    assert!(unfreeze.payload["recalculated_sheets"].as_u64().unwrap() >= 1);
    assert!(
        (scalar(session.document(), "res-d", "Slope").unwrap() - slope_d_frozen).abs() > 1.0,
        "res-d should reflect updated data-c after unfreezing data-d"
    );

    // 7. Explicit `project.recalculate` runs a full force pass over all result sheets even with sentinels present.
    let mut doc = session.document().clone();
    for rid in ["res-a", "res-c", "res-c2", "res-title", "res-d", "res-standalone"] {
        set_sentinel(&mut doc, rid, "__untouched__");
    }
    let mut session = Session::from_document(doc);
    let full = ok(&mut session, "project.recalculate", json!({}));
    assert!(full.payload["recalculated_sheets"].as_u64().unwrap() >= 6);
    for rid in ["res-a", "res-c", "res-c2", "res-title", "res-d", "res-standalone"] {
        assert!(!has_sentinel(session.document(), rid, "__untouched__"), "project.recalculate must force-recompute {rid}");
    }
}

#[test]
fn r1_s07_13_sheet_select_non_mutating_preserves_dirty_undo_and_redo() {
    let spec = cmd::registry().get("sheet.select").expect("sheet.select registered");
    assert!(!spec.effects.contains(Effects::MUTATES_DOCUMENT), "sheet.select must not have MUTATES_DOCUMENT");
    assert!(spec.effects.contains(Effects::CHANGES_UI_STATE));
    assert!(spec.effects.contains(Effects::IDEMPOTENT));

    let mut doc = ProjectDocument::new("Select Test").unwrap();
    doc.add_data_sheet(make_linear_xy("data-1", "D1", 1.0, 0.0)).unwrap();
    doc.add_data_sheet(make_linear_xy("data-2", "D2", 2.0, 0.0)).unwrap();
    let g1 = GraphSheet::new_uninitialized_for_data("graph-1", &doc.data_sheets[0]).unwrap();
    doc.add_graph_sheet(g1).unwrap();
    doc.active_sheet_id = Some(SheetId::new("data-1"));
    doc.family_anchor_sheet_id = Some(SheetId::new("data-1"));

    let mut session = Session::from_document(doc);
    assert!(!session.is_dirty());
    assert_eq!(session.undo_depth(), 0);
    assert_eq!(session.redo_depth(), 0);

    // 1. Selecting in navigator mode on a clean session must not mark dirty or push undo.
    let sel_nav = ok(&mut session, "sheet.select", json!({ "sheet_id": "data-2", "mode": "navigator" }));
    assert!(!sel_nav.mutated);
    assert!(!session.is_dirty(), "sheet.select must keep a clean session clean");
    assert_eq!(session.undo_depth(), 0);
    assert_eq!(session.document().active_sheet_id, Some(SheetId::new("data-2")));
    assert_eq!(session.document().family_anchor_sheet_id, Some(SheetId::new("data-2")));

    // 2. Selecting in family mode keeps family_anchor_sheet_id anchored while remaining clean/non-mutating.
    let sel_fam = ok(&mut session, "sheet.select", json!({ "sheet_id": "graph-1", "mode": "family" }));
    assert!(!sel_fam.mutated);
    assert!(!session.is_dirty());
    assert_eq!(session.undo_depth(), 0);
    assert_eq!(session.document().active_sheet_id, Some(SheetId::new("graph-1")));
    assert_eq!(session.document().family_anchor_sheet_id, Some(SheetId::new("data-2")));

    // 3. Edit cell -> undo -> sheet.select -> redo stack is preserved and redo restores edit!
    ok(&mut session, "table.setCell", json!({ "sheet_id": "data-1", "row": 0, "col": 0, "value": 77.5 }));
    assert_eq!(session.undo_depth(), 1);
    assert!(session.is_dirty());

    ok(&mut session, "sheet.select", json!({ "sheet_id": "data-2" }));
    assert_eq!(session.undo_depth(), 1, "sheet.select must not append an undo entry");

    assert!(session.undo().unwrap());
    assert_eq!(session.undo_depth(), 0);
    assert_eq!(session.redo_depth(), 1);
    assert_eq!(
        session.document().data_sheet(&SheetId::new("data-1")).unwrap().cell(curvebook_doc::CellCoord::new(0, 0, 0)).unwrap().active_number(),
        Some(1.0)
    );

    let sel_after_undo = ok(&mut session, "sheet.select", json!({ "sheet_id": "data-1", "mode": "navigator" }));
    assert!(!sel_after_undo.mutated);
    assert_eq!(session.redo_depth(), 1, "sheet.select must NOT clear the redo stack");

    assert!(session.redo().unwrap());
    assert_eq!(
        session.document().data_sheet(&SheetId::new("data-1")).unwrap().cell(curvebook_doc::CellCoord::new(0, 0, 0)).unwrap().active_number(),
        Some(77.5)
    );
}

#[test]
fn r1_s07_14_pzfx_double_merge_allocates_unique_ids() {
    let mut dest_ds = make_linear_xy("data-1", "Dest Table One", 1.0, 0.0);
    dest_ds.info_sheet_id = Some(SheetId::new("info-1"));
    let dest_info = InfoSheet::new_preseeded("info-1", "Dest Info One").unwrap();
    let mut dest_doc = ProjectDocument::new("Dest PZFX").unwrap();
    dest_doc.add_data_sheet(dest_ds).unwrap();
    dest_doc.add_info_sheet(dest_info).unwrap();
    let mut pzfx_session = Session::from_document(dest_doc);

    ok(&mut pzfx_session, "table.importPzfx", json!({ "xml": cleanroom_pzfx_xml(), "merge": true }));
    ok(&mut pzfx_session, "table.importPzfx", json!({ "xml": cleanroom_pzfx_xml(), "merge": true }));
    assert_eq!(pzfx_session.document().data_sheets.len(), 3);
    assert_eq!(pzfx_session.document().info_sheets.len(), 3);
    assert_eq!(pzfx_session.document().data_sheet(&SheetId::new("data-1")).unwrap().title, "Dest Table One");
    assert_eq!(pzfx_session.document().info_sheet(&SheetId::new("info-1")).unwrap().title, "Dest Info One");
    assert!(curvebook_format::save_project_bytes(pzfx_session.document()).is_ok());
}

fn build_colliding_source_archive_b64() -> String {
    let mut src_doc = ProjectDocument::new("Imported Study").unwrap();
    let mut src_info = InfoSheet::new_preseeded("info-1", "Imported Info").unwrap();
    src_info.set_constant("Dose", ConstantValue::Number(25.0), Some(String::from("uM"))).unwrap();
    src_info.set_constant("Top", ConstantValue::Number(100.0), None).unwrap();
    src_info.linked_data_sheets.push(SheetId::new("data-1"));
    src_doc.add_info_sheet(src_info).unwrap();

    let mut src_d1 = make_linear_xy("data-1", "Imported Data 1", 3.0, 2.0);
    src_d1.info_sheet_id = Some(SheetId::new("info-1"));
    src_doc.add_data_sheet(src_d1).unwrap();
    src_doc.add_data_sheet(make_linear_xy("data-2", "Imported Data 2", 0.0, 0.0)).unwrap();

    let mut src_sess = Session::from_document(src_doc);
    ok(&mut src_sess, "table.pasteLink", json!({ "source_sheet_id": "data-1", "target_sheet_id": "data-2" }));
    ok(&mut src_sess, "analysis.add", json!({ "sheet_id": "res-1", "source_sheet": "data-1", "kind": "linear_regression" }));

    let mut src_doc = src_sess.document().clone();
    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.parameter_rules.push(ParameterRule {
        name: String::from("Top"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-1")),
            constant_name: String::from("Top"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    nl.parameter_rules.push(ParameterRule {
        name: String::from("Slope"),
        scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("res-1"),
            parameter_key: String::from("Slope"),
            data_set_index: Some(0),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    src_doc
        .add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-2", "data-1", AnalysisKind::NonlinearRegression(nl)).unwrap()).unwrap())
        .unwrap();

    let mut src_graph = GraphSheet::new_uninitialized_for_data("graph-1", &src_doc.data_sheets[0]).unwrap();
    src_graph.spec.title = String::from("Imported Graph");
    src_graph.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-1"),
        sample_segments: 256,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    src_graph.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res-2"),
        group_a: 0,
        group_b: 1,
        style: BracketStyle::FlatLine,
        label_mode: BracketLabelMode::SummaryStars,
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 10.0,
        bracket_step_pt: 12.0,
    });
    src_graph.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 100.0, 30.0),
        markdown: String::from("Slope={{result:res-1:Slope}} Dose={{info:info-1:Dose}}"),
        bordered: false,
    });
    src_doc.add_graph_sheet(src_graph).unwrap();

    let mut src_layout = LayoutSpec::default();
    src_layout.items.push(LayoutItem::LiveGraph {
        id: String::from("lg-1"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: RectPt::new(0.0, 0.0, 200.0, 150.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    src_layout.items.push(LayoutItem::LiveTable {
        id: String::from("lt-1"),
        source_sheet_id: SheetId::new("res-1"),
        tab_id: None,
        bounds_pt: RectPt::new(210.0, 0.0, 200.0, 150.0),
        selected_rows: None,
        selected_cols: None,
        style: curvebook_doc::EmbeddedTableStyle::default(),
    });
    src_layout.items.push(LayoutItem::TextCallout {
        id: String::from("tc-1"),
        markdown: String::from("Top={{info:info-1:Top}} Slp={{result:res-1:0:Slope}}"),
        bounds_pt: RectPt::new(0.0, 160.0, 200.0, 40.0),
        font_size_pt: 10.0,
    });
    src_doc.add_layout_sheet(LayoutSheet::new("layout-1", "Imported Layout", src_layout).unwrap()).unwrap();

    let mut src_export_sess = Session::from_document(src_doc);
    let saved = ok(&mut src_export_sess, "project.save", json!({}));
    saved.payload["bytes_base64"].as_str().unwrap().to_owned()
}

#[test]
fn r1_s07_14_prism_merge_remaps_colliding_ids_and_all_refs() {
    let src_b64 = build_colliding_source_archive_b64();

    // Destination document using EVERY colliding ID (`info-1`, `data-1`, `data-2`, `res-1`, `res-2`, `graph-1`, `layout-1`).
    let mut dest_doc = ProjectDocument::new("Dest Study").unwrap();
    dest_doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Dest Info").unwrap()).unwrap();
    dest_doc.add_data_sheet(make_linear_xy("data-1", "Dest Data 1", 9.0, 0.0)).unwrap();
    dest_doc.add_data_sheet(make_linear_xy("data-2", "Dest Data 2", 8.0, 0.0)).unwrap();
    dest_doc
        .add_result_sheet(
            ResultSheet::new(AnalysisSpec::new("res-1", "data-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap()).unwrap(),
        )
        .unwrap();
    dest_doc
        .add_result_sheet(
            ResultSheet::new(AnalysisSpec::new("res-2", "data-2", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap()).unwrap(),
        )
        .unwrap();
    let mut dest_graph = GraphSheet::new_uninitialized_for_data("graph-1", &dest_doc.data_sheets[0]).unwrap();
    dest_graph.spec.title = String::from("Dest Graph");
    dest_doc.add_graph_sheet(dest_graph).unwrap();
    dest_doc.add_layout_sheet(LayoutSheet::new("layout-1", "Dest Layout", LayoutSpec::default()).unwrap()).unwrap();

    let mut dest_sess = Session::from_document(dest_doc);
    ok(&mut dest_sess, "table.importPrism", json!({ "bytes_base64": src_b64, "merge": true }));

    let merged = dest_sess.document();
    // Original destination sheets kept intact at original IDs.
    assert_eq!(merged.info_sheet(&SheetId::new("info-1")).unwrap().title, "Dest Info");
    assert_eq!(merged.data_sheet(&SheetId::new("data-1")).unwrap().title, "Dest Data 1");
    assert_eq!(merged.data_sheet(&SheetId::new("data-2")).unwrap().title, "Dest Data 2");
    assert_eq!(merged.graph_sheet(&SheetId::new("graph-1")).unwrap().spec.title, "Dest Graph");
    assert_eq!(merged.layout_sheet(&SheetId::new("layout-1")).unwrap().title, "Dest Layout");

    assert_remapped_archive_refs(merged);

    assert!(dag::topological_order(merged).is_ok());
    assert!(curvebook_format::save_project_bytes(merged).is_ok());
    let _ = ok(&mut dest_sess, "project.recalculate", json!({}));
    assert_eq!(dest_sess.document().result_sheet(&SheetId::new("res-1-2")).unwrap().status, ResultStatus::UpToDate);
}

fn assert_remapped_archive_refs(merged: &ProjectDocument) {
    // Remapped imported sheets exist at `-2` IDs with EVERY internal reference rewritten to `-2`.
    let imp_info = merged.info_sheet(&SheetId::new("info-1-2")).expect("remapped info-1-2");
    assert_eq!(imp_info.linked_data_sheets, vec![SheetId::new("data-1-2")]);

    let imp_d1 = merged.data_sheet(&SheetId::new("data-1-2")).expect("remapped data-1-2");
    assert_eq!(imp_d1.info_sheet_id, Some(SheetId::new("info-1-2")));

    let imp_d2 = merged.data_sheet(&SheetId::new("data-2-2")).expect("remapped data-2-2");
    assert_eq!(imp_d2.linked_regions[0].spec.source.upstream_sheet_id(), Some(&SheetId::new("data-1-2")));

    let imp_r1 = merged.result_sheet(&SheetId::new("res-1-2")).expect("remapped res-1-2");
    assert_eq!(imp_r1.analysis.id, SheetId::new("res-1-2"));
    assert_eq!(imp_r1.analysis.source_sheet_id, SheetId::new("data-1-2"));

    let imp_r2 = merged.result_sheet(&SheetId::new("res-2-2")).expect("remapped res-2-2");
    assert_eq!(imp_r2.analysis.source_sheet_id, SheetId::new("data-1-2"));
    let hooks = imp_r2.analysis.kind.hooked_sources();
    assert!(hooks.iter().any(|h| matches!(h, HookSource::InfoConstant { info_sheet_id: Some(id), .. } if id == &SheetId::new("info-1-2"))));
    assert!(hooks.iter().any(|h| matches!(h, HookSource::ResultScalar { result_sheet_id, .. } if result_sheet_id == &SheetId::new("res-1-2"))));

    let imp_g1 = merged.graph_sheet(&SheetId::new("graph-1-2")).expect("remapped graph-1-2");
    assert_eq!(imp_g1.spec.primary_source_sheet_id, SheetId::new("data-1-2"));
    assert_eq!(imp_g1.spec.series[0].source_sheet_id, SheetId::new("data-1-2"));
    assert_eq!(imp_g1.spec.series[0].regression_overlay.as_ref().unwrap().result_sheet_id, SheetId::new("res-1-2"));
    assert_eq!(imp_g1.spec.comparisons[0].result_sheet_id, SheetId::new("res-2-2"));
    match &imp_g1.spec.annotations[0] {
        GraphAnnotation::TextBox { markdown, .. } => {
            assert!(markdown.contains("{{result:res-1-2:Slope}}"), "markdown={markdown}");
            assert!(markdown.contains("{{info:info-1-2:Dose}}"), "markdown={markdown}");
        }
        other => panic!("unexpected annotation: {other:?}"),
    }

    let imp_l1 = merged.layout_sheet(&SheetId::new("layout-1-2")).expect("remapped layout-1-2");
    assert!(matches!(&imp_l1.spec.items[0], LayoutItem::LiveGraph { graph_sheet_id, .. } if graph_sheet_id == &SheetId::new("graph-1-2")));
    assert!(matches!(&imp_l1.spec.items[1], LayoutItem::LiveTable { source_sheet_id, .. } if source_sheet_id == &SheetId::new("res-1-2")));
    match &imp_l1.spec.items[2] {
        LayoutItem::TextCallout { markdown, .. } => {
            assert!(markdown.contains("{{info:info-1-2:Top}}"), "markdown={markdown}");
            assert!(markdown.contains("{{result:res-1-2:0:Slope}}"), "markdown={markdown}");
        }
        other => panic!("unexpected layout item: {other:?}"),
    }
}

#[test]
fn r1_x2_09_overflowing_bland_altman_fails_cleanly_and_never_blocks_save() {
    // Case 0: Differences around 1e300 stay finite after `bland_altman` SD scaling and must save cleanly.
    let mut scaled_doc = ProjectDocument::new("Scaled Extreme Check").unwrap();
    let mut scaled_tbl = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 3).unwrap();
    scaled_tbl.data_sets[0].title = String::from("Method A");
    scaled_tbl.data_sets[1].title = String::from("Method B");
    let col_a_scaled = [1e300_f64, 1.1e300, 1.2e300];
    let col_b_scaled = [-1e300_f64, 1e300, 1.5e300];
    for r in 0..3 {
        scaled_tbl.data_sets[0].cells[r][0] = Cell::number(col_a_scaled[r]);
        scaled_tbl.data_sets[1].cells[r][0] = Cell::number(col_b_scaled[r]);
    }
    scaled_doc.add_data_sheet(DataSheet::new("col-ba", "Scaled Extreme Pair", TableArchetype::Column(scaled_tbl)).unwrap()).unwrap();
    let mut scaled_session = Session::from_document(scaled_doc);
    let scaled_res = scaled_session.execute("analysis.add", json!({ "sheet_id": "res-ba", "source_sheet": "col-ba", "kind": "stats.bland_altman" }));
    assert!(scaled_res.is_ok(), "scaled 1e300 Bland-Altman should succeed finitely: {scaled_res:?}");
    assert_eq!(scaled_session.document().result_sheet(&SheetId::new("res-ba")).unwrap().status, ResultStatus::UpToDate);
    assert!(curvebook_format::save_project_bytes(scaled_session.document()).is_ok());

    // Case 1: Adding Bland-Altman directly onto difference-overflowing 1e308 data (`1e308 - (-1e308) = +inf`)
    // must fail cleanly rather than committing non-finite floats that poison `project.save`.
    let mut overflow_doc = ProjectDocument::new("Overflow Add Check").unwrap();
    let mut tbl = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 3).unwrap();
    tbl.data_sets[0].title = String::from("Method A");
    tbl.data_sets[1].title = String::from("Method B");
    let col_a = [1e308_f64, 1.1e308, 1.2e308];
    let col_b = [-1e308_f64, 1e308, 1.5e308];
    for r in 0..3 {
        tbl.data_sets[0].cells[r][0] = Cell::number(col_a[r]);
        tbl.data_sets[1].cells[r][0] = Cell::number(col_b[r]);
    }
    overflow_doc.add_data_sheet(DataSheet::new("col-ba", "Overflowing Pair", TableArchetype::Column(tbl)).unwrap()).unwrap();
    let mut add_session = Session::from_document(overflow_doc);
    let add_res = add_session.execute("analysis.add", json!({ "sheet_id": "res-ba", "source_sheet": "col-ba", "kind": "stats.bland_altman" }));
    assert!(add_res.is_err(), "analysis.add must reject non-finite Bland-Altman result");
    assert!(add_session.document().result_sheet(&SheetId::new("res-ba")).is_none(), "rolled-back session must stay clean");
    assert!(curvebook_format::save_project_bytes(add_session.document()).is_ok());
    let _ = ok(&mut add_session, "project.save", json!({}));

    // Case 2: Initially healthy Bland-Altman becomes overflowing on downstream cell edits ->
    // `recompute_result_sheet` clears stale outputs, marks ResultStatus::Failed, and save still succeeds!
    let mut doc = ProjectDocument::new("Overflow Downstream Check").unwrap();
    let mut tbl2 = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 3).unwrap();
    tbl2.data_sets[0].title = String::from("Method A");
    tbl2.data_sets[1].title = String::from("Method B");
    for r in 0..3 {
        tbl2.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 10.0);
        tbl2.data_sets[1].cells[r][0] = Cell::number((r + 1) as f64 * 10.0 + 1.5);
    }
    doc.add_data_sheet(DataSheet::new("col-ba", "Healthy Pair", TableArchetype::Column(tbl2)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);
    ok(&mut session, "analysis.add", json!({ "sheet_id": "res-ba", "source_sheet": "col-ba", "kind": "stats.bland_altman" }));
    assert_eq!(session.document().result_sheet(&SheetId::new("res-ba")).unwrap().status, ResultStatus::UpToDate);

    for (r, (&a, &b)) in col_a.iter().zip(col_b.iter()).enumerate() {
        ok(&mut session, "table.setCell", json!({ "sheet_id": "col-ba", "row": r, "col": 0, "value": a }));
        ok(&mut session, "table.setCell", json!({ "sheet_id": "col-ba", "row": r, "col": 1, "value": b }));
    }

    let res = session.document().result_sheet(&SheetId::new("res-ba")).expect("res-ba present");
    assert!(matches!(res.status, ResultStatus::Failed { .. }), "expected overflowing analysis after edit to be marked Failed, got {:?}", res.status);
    assert!(res.fitted_curves.is_empty());
    assert!(res.scalars.is_empty());
    assert!(res.tabs.is_empty());
    assert!(res.pairwise_comparisons.is_empty());
    assert!(res.analyzable_data.is_none());

    assert!(curvebook_format::save_project_bytes(session.document()).is_ok());
    let _saved = ok(&mut session, "project.save", json!({}));
}
