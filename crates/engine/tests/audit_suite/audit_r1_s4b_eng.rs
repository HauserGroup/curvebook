//! Stage 4B S3 audit regressions: `R1-S07-12`, `R1-S07-13`, `R1-S07-14`, `R1-X2-09`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, BracketStyle, ColumnDesignHint, ConstantValue, DataSheet, GraphAnnotation, GraphSheet, HookSource,
    InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, LinearRegressionSpec, NonlinearFitSpec, NormalizeAnchor, PairwiseBracketSpec, ParameterRule,
    ParameterScope, ProjectDocument, RectPt, RegressionOverlaySpec, ResultScalarEntry, ResultSheet, ResultStatus, SheetId, TransformAnalysisSpec,
};
use curvebook_engine::{Session, cmd, cmd::Effects, dag};
use serde_json::{Value, json};

use super::common::{cell_num, cleanroom_pzfx_xml, col_sheet_hint, ok_cmd as ok, rs, run_cmds, scalar as doc_scalar, xy_sheet};

fn make_linear_xy(id: &str, title: &str, slope: f64, intercept: f64) -> DataSheet {
    let ys: Vec<f64> = (1..=5).map(|i| slope * (i as f64) + intercept).collect();
    xy_sheet(id, title, "X", &[1.0, 2.0, 3.0, 4.0, 5.0], &[("10.0", &ys)])
}

fn plant_sentinels(session: &Session, ids: &[&str]) -> Session {
    let mut doc = session.document().clone();
    for rid in ids {
        let r = doc.result_sheet_mut(&SheetId::new(*rid)).unwrap();
        r.scalars.retain(|s| s.key != "__untouched__");
        r.scalars.push(ResultScalarEntry { key: String::from("__untouched__"), data_set_index: None, value: 98765.0, units: None });
    }
    Session::from_document(doc)
}

fn has_sentinel(session: &Session, res_id: &str) -> bool {
    rs(session, res_id).scalars.iter().any(|s| s.key == "__untouched__")
}

fn hook_rule(name: &str, h: HookSource) -> ParameterRule {
    ParameterRule { name: name.to_owned(), scope: ParameterScope::HookedConstant(h), initial_override: None, lower_bound: None, upper_bound: None }
}

fn setup_s07_12_session() -> Session {
    let mut doc = ProjectDocument::new("DAG Incremental").unwrap();
    let mut info_c = InfoSheet::new_preseeded("info-c", "Info C").unwrap();
    info_c.set_constant("Scale", ConstantValue::Number(100.0), None).unwrap();
    info_c.set_constant("Multiplier", ConstantValue::Number(2.0), None).unwrap();
    info_c.linked_data_sheets.push(SheetId::new("data-c"));
    doc.add_info_sheet(info_c).unwrap();

    doc.add_data_sheet(make_linear_xy("data-a", "Table A", 2.0, 1.0)).unwrap();
    doc.add_data_sheet(make_linear_xy("data-b", "Table B", 3.0, 0.0)).unwrap();
    let mut ds_c = make_linear_xy("data-c", "Table C", 1.5, 0.5);
    ds_c.info_sheet_id = Some(SheetId::new("info-c"));
    doc.add_data_sheet(ds_c).unwrap();
    doc.add_data_sheet(make_linear_xy("data-d", "Table D", 0.0, 0.0)).unwrap();

    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("res-a", "data-a", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap()).unwrap(),
    )
    .unwrap();
    doc.add_result_sheet(
        ResultSheet::new(
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
        .unwrap(),
    )
    .unwrap();

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.parameter_rules.push(hook_rule("Slope", HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Multiplier") }));
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-c2", "res-c", AnalysisKind::NonlinearRegression(nl)).unwrap()).unwrap()).unwrap();

    let mut nl_title = NonlinearFitSpec::new("straight_line");
    nl_title.parameter_rules.push(hook_rule("Slope", HookSource::ColumnTitleValue { data_sheet_id: SheetId::new("data-c"), data_set_index: 0 }));
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-title", "data-c", AnalysisKind::NonlinearRegression(nl_title)).unwrap()).unwrap())
        .unwrap();

    dag::recalculate_project(&mut doc).unwrap();
    let mut session = Session::from_document(doc);
    run_cmds(
        &mut session,
        &[
            ("table.pasteLink", json!({ "source_sheet_id": "data-c", "target_sheet_id": "data-d" })),
            ("analysis.add", json!({ "sheet_id": "res-d", "source_sheet_id": "data-d", "kind": "linear_regression" })),
        ],
    );
    session
}

#[test]
fn r1_s07_12_dirty_incremental_recalc_closure_and_isolation() {
    let all = ["res-a", "res-c", "res-c2", "res-title", "res-d"];
    let mut session = plant_sentinels(&setup_s07_12_session(), &all);

    let edit_b = ok(&mut session, "table.setCell", json!({ "sheet_id": "data-b", "row": 0, "col": 0, "value": 99.0 }));
    assert_eq!((edit_b.payload["recalculated_sheets"].as_u64(), edit_b.payload["refreshed_links"].as_u64()), (Some(0), Some(0)));
    assert!(all.iter().all(|r| has_sentinel(&session, r)));

    let edit_a = ok(&mut session, "table.setCell", json!({ "sheet_id": "data-a", "row": 4, "col": 0, "value": 20.0 }));
    assert_eq!(edit_a.payload["recalculated_sheets"], json!(1));
    assert!(!has_sentinel(&session, "res-a") && all[1..].iter().all(|r| has_sentinel(&session, r)));

    let mut session = plant_sentinels(&session, &["res-a"]);
    let edit_c = ok(&mut session, "table.setCell", json!({ "sheet_id": "data-c", "row": 4, "col": 0, "value": 18.0 }));
    assert_eq!((edit_c.payload["recalculated_sheets"].as_u64(), edit_c.payload["refreshed_links"].as_u64()), (Some(5), Some(1)));
    assert!(has_sentinel(&session, "res-a") && all[1..].iter().all(|r| !has_sentinel(&session, r)));
}

#[test]
fn r1_s07_12_info_title_freeze_and_explicit_force_recalc() {
    let mut doc = setup_s07_12_session().document().clone();
    let mut info_extra = InfoSheet::new_preseeded("info-standalone", "Standalone Info").unwrap();
    info_extra.set_constant("SlopeOverride", ConstantValue::Number(4.0), None).unwrap();
    let mut nl_extra = NonlinearFitSpec::new("straight_line");
    nl_extra.parameter_rules.push(hook_rule(
        "Slope",
        HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-standalone")), constant_name: String::from("SlopeOverride") },
    ));
    doc.add_info_sheet(info_extra).unwrap();
    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("res-standalone", "data-b", AnalysisKind::NonlinearRegression(nl_extra)).unwrap()).unwrap(),
    )
    .unwrap();
    dag::recalculate_project(&mut doc).unwrap();

    let base_ids = ["res-a", "res-c", "res-c2", "res-title", "res-d"];
    let mut session = plant_sentinels(&Session::from_document(doc), &base_ids);
    assert_eq!(
        ok(&mut session, "info.setConstant", json!({ "sheet_id": "info-standalone", "name": "SlopeOverride", "value": 7.0 })).payload["recalculated_sheets"],
        json!(1)
    );
    assert!(base_ids.iter().all(|r| has_sentinel(&session, r)));

    let mut session = plant_sentinels(&session, &["res-a", "res-standalone"]);
    assert_eq!(
        ok(&mut session, "info.setConstant", json!({ "sheet_id": "info-c", "name": "Scale", "value": 50.0 })).payload["recalculated_sheets"],
        json!(5)
    );
    assert!(has_sentinel(&session, "res-a") && has_sentinel(&session, "res-standalone"));

    assert!(
        ok(&mut session, "table.setTitles", json!({ "sheet_id": "data-c", "titles": ["25.0"] })).payload["recalculated_sheets"].as_u64().unwrap()
            >= 1
    );
    assert!(has_sentinel(&session, "res-a") && (doc_scalar(&session, "res-title", "Slope") - 25.0).abs() < 1e-6);

    ok(&mut session, "sheet.freeze", json!({ "sheet_id": "data-d" }));
    let slope_frozen = doc_scalar(&session, "res-d", "Slope");
    assert_eq!(
        ok(&mut session, "table.setCell", json!({ "sheet_id": "data-c", "row": 4, "col": 0, "value": 42.0 })).payload["refreshed_links"],
        json!(0)
    );
    assert!((doc_scalar(&session, "res-d", "Slope") - slope_frozen).abs() < 1e-12);

    assert!(ok(&mut session, "sheet.unfreeze", json!({ "sheet_id": "data-d" })).payload["recalculated_sheets"].as_u64().unwrap() >= 1);
    assert!((doc_scalar(&session, "res-d", "Slope") - slope_frozen).abs() > 1.0);

    let all_six = ["res-a", "res-c", "res-c2", "res-title", "res-d", "res-standalone"];
    let mut session = plant_sentinels(&session, &all_six);
    assert!(ok(&mut session, "project.recalculate", json!({})).payload["recalculated_sheets"].as_u64().unwrap() >= 6);
    assert!(all_six.iter().all(|r| !has_sentinel(&session, r)));
}

#[test]
fn r1_s07_13_sheet_select_non_mutating_preserves_dirty_undo_and_redo() {
    let spec = cmd::registry().get("sheet.select").expect("sheet.select");
    assert!(!spec.effects.contains(Effects::MUTATES_DOCUMENT) && spec.effects.contains(Effects::CHANGES_UI_STATE | Effects::IDEMPOTENT));

    let mut doc = ProjectDocument::new("Select Test").unwrap();
    doc.add_data_sheet(make_linear_xy("data-1", "D1", 1.0, 0.0)).unwrap();
    doc.add_data_sheet(make_linear_xy("data-2", "D2", 2.0, 0.0)).unwrap();
    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("graph-1", &doc.data_sheets[0]).unwrap()).unwrap();
    doc.active_sheet_id = Some(SheetId::new("data-1"));
    doc.family_anchor_sheet_id = Some(SheetId::new("data-1"));

    let mut session = Session::from_document(doc);
    let sel_nav = ok(&mut session, "sheet.select", json!({ "sheet_id": "data-2", "mode": "navigator" }));
    assert!(!sel_nav.mutated && !session.is_dirty() && session.undo_depth() == 0);
    assert_eq!(
        (&session.document().active_sheet_id, &session.document().family_anchor_sheet_id),
        (&Some(SheetId::new("data-2")), &Some(SheetId::new("data-2")))
    );

    let sel_fam = ok(&mut session, "sheet.select", json!({ "sheet_id": "graph-1", "mode": "family" }));
    assert!(!sel_fam.mutated && !session.is_dirty() && session.undo_depth() == 0);
    assert_eq!(
        (&session.document().active_sheet_id, &session.document().family_anchor_sheet_id),
        (&Some(SheetId::new("graph-1")), &Some(SheetId::new("data-2")))
    );

    ok(&mut session, "table.setCell", json!({ "sheet_id": "data-1", "row": 0, "col": 0, "value": 77.5 }));
    ok(&mut session, "sheet.select", json!({ "sheet_id": "data-2" }));
    assert!(session.is_dirty() && session.undo_depth() == 1);

    assert!(session.undo().unwrap() && session.undo_depth() == 0 && session.redo_depth() == 1 && cell_num(&session, "data-1", 0, 0, 0) == Some(1.0));
    assert!(!ok(&mut session, "sheet.select", json!({ "sheet_id": "data-1", "mode": "navigator" })).mutated && session.redo_depth() == 1);
    assert!(session.redo().unwrap() && cell_num(&session, "data-1", 0, 0, 0) == Some(77.5));
}

#[test]
fn r1_s07_14_pzfx_double_merge_allocates_unique_ids() {
    let mut dest_doc = ProjectDocument::new("Dest PZFX").unwrap();
    let mut dest_ds = make_linear_xy("data-1", "Dest Table One", 1.0, 0.0);
    dest_ds.info_sheet_id = Some(SheetId::new("info-1"));
    dest_doc.add_data_sheet(dest_ds).unwrap();
    dest_doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Dest Info One").unwrap()).unwrap();
    let mut s = Session::from_document(dest_doc);

    ok(&mut s, "table.importPzfx", json!({ "xml": cleanroom_pzfx_xml(), "merge": true }));
    ok(&mut s, "table.importPzfx", json!({ "xml": cleanroom_pzfx_xml(), "merge": true }));
    let doc = s.document();
    assert_eq!((doc.data_sheets.len(), doc.info_sheets.len()), (3, 3));
    assert_eq!(
        (doc.data_sheet(&SheetId::new("data-1")).unwrap().title.as_str(), doc.info_sheet(&SheetId::new("info-1")).unwrap().title.as_str()),
        ("Dest Table One", "Dest Info One")
    );
    assert!(curvebook_format::save_project_bytes(doc).is_ok());
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
    run_cmds(
        &mut src_sess,
        &[
            ("table.pasteLink", json!({ "source_sheet_id": "data-1", "target_sheet_id": "data-2" })),
            ("analysis.add", json!({ "sheet_id": "res-1", "source_sheet_id": "data-1", "kind": "linear_regression" })),
        ],
    );
    let mut src_doc = src_sess.document().clone();

    let mut nl = NonlinearFitSpec::new("straight_line");
    nl.parameter_rules = vec![
        hook_rule("Top", HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-1")), constant_name: String::from("Top") }),
        hook_rule(
            "Slope",
            HookSource::ResultScalar { result_sheet_id: SheetId::new("res-1"), parameter_key: String::from("Slope"), data_set_index: Some(0) },
        ),
    ];
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

    let src_layout = LayoutSpec {
        items: vec![
            LayoutItem::LiveGraph {
                id: String::from("lg-1"),
                graph_sheet_id: SheetId::new("graph-1"),
                bounds_pt: RectPt::new(0.0, 0.0, 200.0, 150.0),
                scale_percent: 100.0,
                badge_override: None,
            },
            LayoutItem::LiveTable {
                id: String::from("lt-1"),
                source_sheet_id: SheetId::new("res-1"),
                tab_id: None,
                bounds_pt: RectPt::new(210.0, 0.0, 200.0, 150.0),
                selected_rows: None,
                selected_cols: None,
                style: Default::default(),
            },
            LayoutItem::TextCallout {
                id: String::from("tc-1"),
                markdown: String::from("Top={{info:info-1:Top}} Slp={{result:res-1:0:Slope}}"),
                bounds_pt: RectPt::new(0.0, 160.0, 200.0, 40.0),
                font_size_pt: 10.0,
            },
        ],
        ..Default::default()
    };
    src_doc.add_layout_sheet(LayoutSheet::new("layout-1", "Imported Layout", src_layout).unwrap()).unwrap();
    ok(&mut Session::from_document(src_doc), "project.save", json!({})).payload.get("bytes_base64").and_then(Value::as_str).unwrap().to_owned()
}

#[test]
fn r1_s07_14_prism_merge_remaps_colliding_ids_and_all_refs() {
    let src_b64 = build_colliding_source_archive_b64();
    let mut dest_doc = ProjectDocument::new("Dest Study").unwrap();
    dest_doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Dest Info").unwrap()).unwrap();
    dest_doc.add_data_sheet(make_linear_xy("data-1", "Dest Data 1", 9.0, 0.0)).unwrap();
    dest_doc.add_data_sheet(make_linear_xy("data-2", "Dest Data 2", 8.0, 0.0)).unwrap();
    for (rid, src) in [("res-1", "data-1"), ("res-2", "data-2")] {
        dest_doc
            .add_result_sheet(
                ResultSheet::new(AnalysisSpec::new(rid, src, AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap()).unwrap(),
            )
            .unwrap();
    }
    let mut dest_graph = GraphSheet::new_uninitialized_for_data("graph-1", &dest_doc.data_sheets[0]).unwrap();
    dest_graph.spec.title = String::from("Dest Graph");
    dest_doc.add_graph_sheet(dest_graph).unwrap();
    dest_doc.add_layout_sheet(LayoutSheet::new("layout-1", "Dest Layout", LayoutSpec::default()).unwrap()).unwrap();

    let mut dest_sess = Session::from_document(dest_doc);
    ok(&mut dest_sess, "table.importPrism", json!({ "bytes_base64": src_b64, "merge": true }));
    let merged = dest_sess.document();

    assert_eq!(
        (
            merged.info_sheet(&SheetId::new("info-1")).unwrap().title.as_str(),
            merged.data_sheet(&SheetId::new("data-1")).unwrap().title.as_str(),
            merged.data_sheet(&SheetId::new("data-2")).unwrap().title.as_str(),
            merged.graph_sheet(&SheetId::new("graph-1")).unwrap().spec.title.as_str(),
            merged.layout_sheet(&SheetId::new("layout-1")).unwrap().title.as_str()
        ),
        ("Dest Info", "Dest Data 1", "Dest Data 2", "Dest Graph", "Dest Layout")
    );

    assert_eq!(merged.info_sheet(&SheetId::new("info-1-2")).unwrap().linked_data_sheets, vec![SheetId::new("data-1-2")]);
    assert_eq!(merged.data_sheet(&SheetId::new("data-1-2")).unwrap().info_sheet_id, Some(SheetId::new("info-1-2")));
    assert_eq!(
        merged.data_sheet(&SheetId::new("data-2-2")).unwrap().linked_regions[0].spec.source.upstream_sheet_id(),
        Some(&SheetId::new("data-1-2"))
    );
    let imp_r1 = merged.result_sheet(&SheetId::new("res-1-2")).unwrap();
    assert_eq!((imp_r1.analysis.id.as_str(), imp_r1.analysis.source_sheet_id.as_str()), ("res-1-2", "data-1-2"));
    let imp_r2 = merged.result_sheet(&SheetId::new("res-2-2")).unwrap();
    let hooks = imp_r2.analysis.kind.hooked_sources();
    assert_eq!(imp_r2.analysis.source_sheet_id.as_str(), "data-1-2");
    assert!(hooks.iter().any(|h| matches!(h, HookSource::InfoConstant { info_sheet_id: Some(id), .. } if id.as_str() == "info-1-2")));
    assert!(hooks.iter().any(|h| matches!(h, HookSource::ResultScalar { result_sheet_id, .. } if result_sheet_id.as_str() == "res-1-2")));

    let imp_g1 = merged.graph_sheet(&SheetId::new("graph-1-2")).unwrap();
    assert_eq!(
        (
            imp_g1.spec.primary_source_sheet_id.as_str(),
            imp_g1.spec.series[0].source_sheet_id.as_str(),
            imp_g1.spec.series[0].regression_overlay.as_ref().unwrap().result_sheet_id.as_str(),
            imp_g1.spec.comparisons[0].result_sheet_id.as_str()
        ),
        ("data-1-2", "data-1-2", "res-1-2", "res-2-2")
    );
    assert!(
        matches!(&imp_g1.spec.annotations[0], GraphAnnotation::TextBox { markdown, .. } if markdown.contains("{{result:res-1-2:Slope}}") && markdown.contains("{{info:info-1-2:Dose}}"))
    );

    let items = &merged.layout_sheet(&SheetId::new("layout-1-2")).unwrap().spec.items;
    assert_eq!(items.len(), 3);
    assert!(matches!(&items[0], LayoutItem::LiveGraph { graph_sheet_id, .. } if graph_sheet_id.as_str() == "graph-1-2"));
    assert!(matches!(&items[1], LayoutItem::LiveTable { source_sheet_id, .. } if source_sheet_id.as_str() == "res-1-2"));
    assert!(
        matches!(&items[2], LayoutItem::TextCallout { markdown, .. } if markdown.contains("{{info:info-1-2:Top}}") && markdown.contains("{{result:res-1-2:0:Slope}}"))
    );

    assert!(dag::topological_order(merged).is_ok() && curvebook_format::save_project_bytes(merged).is_ok());
    ok(&mut dest_sess, "project.recalculate", json!({}));
    assert_eq!(rs(&dest_sess, "res-1-2").status, ResultStatus::UpToDate);
}

#[test]
fn r1_x2_09_overflowing_bland_altman_fails_cleanly_and_never_blocks_save() {
    let pair = |title: &str, a: &[f64], b: &[f64]| {
        let mut doc = ProjectDocument::new(title).unwrap();
        doc.add_data_sheet(col_sheet_hint("col-ba", title, ColumnDesignHint::PairedRepeatedMeasures, &[("Method A", a), ("Method B", b)])).unwrap();
        Session::from_document(doc)
    };

    // Case 0: 1e300 stays finite after SD scaling and saves cleanly.
    let mut s0 = pair("Scaled Extreme", &[1e300, 1.1e300, 1.2e300], &[-1e300, 1e300, 1.5e300]);
    assert!(s0.execute("analysis.add", json!({ "sheet_id": "res-ba", "source_sheet_id": "col-ba", "kind": "stats.bland_altman" })).is_ok());
    assert_eq!(rs(&s0, "res-ba").status, ResultStatus::UpToDate);
    assert!(curvebook_format::save_project_bytes(s0.document()).is_ok());

    // Case 1: Direct add on overflowing 1e308 rejects cleanly without poisoning save.
    let (ovf_a, ovf_b) = ([1e308, 1.1e308, 1.2e308], [-1e308, 1e308, 1.5e308]);
    let mut s1 = pair("Overflow Add", &ovf_a, &ovf_b);
    assert!(s1.execute("analysis.add", json!({ "sheet_id": "res-ba", "source_sheet_id": "col-ba", "kind": "stats.bland_altman" })).is_err());
    assert!(s1.document().result_sheet(&SheetId::new("res-ba")).is_none() && curvebook_format::save_project_bytes(s1.document()).is_ok());
    ok(&mut s1, "project.save", json!({}));

    // Case 2: Downstream cell edits causing overflow mark Failed, clear outputs, and allow save.
    let mut s2 = pair("Overflow Downstream", &[10.0, 20.0, 30.0], &[11.5, 21.5, 31.5]);
    ok(&mut s2, "analysis.add", json!({ "sheet_id": "res-ba", "source_sheet_id": "col-ba", "kind": "stats.bland_altman" }));
    assert_eq!(rs(&s2, "res-ba").status, ResultStatus::UpToDate);
    for (r, (&a, &b)) in ovf_a.iter().zip(ovf_b.iter()).enumerate() {
        ok(&mut s2, "table.setCell", json!({ "sheet_id": "col-ba", "row": r, "col": 0, "value": a }));
        ok(&mut s2, "table.setCell", json!({ "sheet_id": "col-ba", "row": r, "col": 1, "value": b }));
    }
    let res = rs(&s2, "res-ba");
    assert!(
        matches!(res.status, ResultStatus::Failed { .. })
            && res.fitted_curves.is_empty()
            && res.scalars.is_empty()
            && res.tabs.is_empty()
            && res.pairwise_comparisons.is_empty()
            && res.analyzable_data.is_none()
    );
    assert!(curvebook_format::save_project_bytes(s2.document()).is_ok());
    ok(&mut s2, "project.save", json!({}));
}
