//! Regression tests for ledger items RV-56, RV-57, and RV-61 (`crates/doc` & `crates/engine/src/cmd/{table,sheet}.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, BracketStyle, Cell, ColorChannelMapping, CompactLetterDisplaySpec, EquationDisplayMode, ErrorCode,
    GraphAnnotation, GraphSheet, GraphSpec, LevelOrderKind, LevelOrderSpec, MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec,
    OneWayAnovaSpec, PairwiseBracketSpec, PairwiseDisplayMode, ProjectDocument, RectPt, RegressionOverlaySpec, ResultSheet, SheetId,
    SizeChannelMapping, TableArchetype, VariableRole,
};
use curvebook_engine::{Session, dag};
use curvebook_palette::PaletteId;
use serde_json::json;

use super::common::{assert_err_code, col_sheet, ds, gs, mv_sheet, ok_cmd, run_cmds, xy_sheet};

#[test]
fn rv56_recode_levels_updates_variable_explicit_order_and_graph_overrides_with_dedup() {
    let mut doc = ProjectDocument::new("RV-56 Recode Order").unwrap();
    let mut d1 = mv_sheet(
        "mv-1",
        "Study",
        &[],
        &[
            ("Arm", Some(VariableRole::Categorical), &[Cell::text("ctrl"), Cell::text("drugA"), Cell::text("veh"), Cell::text("drugB")]),
            ("Response", None, &[Cell::number(10.0), Cell::number(25.0), Cell::number(11.0), Cell::number(30.0)]),
            ("Cohort", Some(VariableRole::Categorical), &[Cell::text("c1"), Cell::text("c2"), Cell::text("c1"), Cell::text("c2")]),
        ],
    );
    if let TableArchetype::MultipleVariables(ref mut mv) = d1.archetype {
        mv.variables[0].level_order =
            LevelOrderSpec { order: LevelOrderKind::Explicit(vec!["ctrl".into(), "drugA".into(), "veh".into(), "drugB".into()]), reversed: false };
    }
    let mut gspec = GraphSpec::default_for_data_sheet(&d1);
    gspec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Categorical {
            column: 2,
            palette: PaletteId::OkabeIto,
            level_order_override: Some(LevelOrderSpec { order: LevelOrderKind::Explicit(vec!["c2".into(), "c1".into()]), reversed: false }),
        },
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: Some(LevelOrderSpec {
            order: LevelOrderKind::Explicit(vec!["veh".into(), "drugB".into(), "ctrl".into(), "drugA".into()]),
            reversed: false,
        }),
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    doc.add_data_sheet(d1).unwrap();
    doc.add_graph_sheet(GraphSheet {
        id: SheetId::new("g-mv"),
        title: "MV Plot".into(),
        initialized: true,
        spec: gspec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    })
    .unwrap();

    let d2 = mv_sheet(
        "mv-2",
        "Other Study",
        &[],
        &[("A", Some(VariableRole::Categorical), &[Cell::text("ctrl"), Cell::text("drugA")]), ("B", None, &[Cell::number(1.0), Cell::number(2.0)])],
    );
    let mut gspec2 = GraphSpec::default_for_data_sheet(&d2);
    if let Some(enc2) = &mut gspec2.multivariate {
        enc2.x_level_order_override = Some(LevelOrderSpec { order: LevelOrderKind::Explicit(vec!["ctrl".into(), "drugA".into()]), reversed: false });
    }
    doc.add_data_sheet(d2).unwrap();
    doc.add_graph_sheet(GraphSheet {
        id: SheetId::new("g-other"),
        title: "Other Plot".into(),
        initialized: true,
        spec: gspec2,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    })
    .unwrap();

    let mut session = Session::from_document(doc);
    ok_cmd(
        &mut session,
        "table.recodeLevels",
        json!({ "sheet_id": "mv-1", "col": 0, "mappings": [{ "from": "ctrl", "to": "control" }, { "from": "veh", "to": "control" }] }),
    );

    let TableArchetype::MultipleVariables(mv_after) = &ds(&session, "mv-1").archetype else { panic!("expected MV") };
    assert_eq!(mv_after.variables[0].level_order.order, LevelOrderKind::Explicit(vec!["control".into(), "drugA".into(), "drugB".into()]));
    assert_eq!(mv_after.variables[0].ordered_levels().into_iter().map(|s| s.level).collect::<Vec<_>>(), vec!["control", "drugA", "drugB"]);
    assert_eq!(
        gs(&session, "g-mv").spec.multivariate.as_ref().unwrap().x_level_order_override.as_ref().unwrap().order,
        LevelOrderKind::Explicit(vec!["control".into(), "drugB".into(), "drugA".into()])
    );

    ok_cmd(&mut session, "table.recodeLevels", json!({ "sheet_id": "mv-1", "col": 2, "mappings": [{ "from": "c2", "to": "CohortTwo" }] }));
    let ColorChannelMapping::Categorical { level_order_override: Some(color_ov), .. } =
        &gs(&session, "g-mv").spec.multivariate.as_ref().unwrap().color
    else {
        panic!("expected categorical override")
    };
    assert_eq!(color_ov.order, LevelOrderKind::Explicit(vec!["CohortTwo".into(), "c1".into()]));
    assert_eq!(
        gs(&session, "g-other").spec.multivariate.as_ref().unwrap().x_level_order_override.as_ref().unwrap().order,
        LevelOrderKind::Explicit(vec!["ctrl".into(), "drugA".into()])
    );
}

#[test]
fn rv57_recode_levels_refreshes_inferred_continuous_role_to_categorical() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "mv-num", "archetype": "multiple_variables", "cols": 2, "rows": 3, "auto_graph": false })),
            ("table.setCells", json!({ "sheet_id": "mv-num", "rows": [[1.0, 1.0], [2.0, 2.0], [3.0, 3.0]] })),
            ("table.setVariableRole", json!({ "sheet_id": "mv-num", "col": 1, "role": "label" })),
        ],
    );
    let mv_roles = |s: &Session| match &ds(s, "mv-num").archetype {
        TableArchetype::MultipleVariables(mv) => {
            ((mv.variables[0].role, mv.variables[0].role_inferred), (mv.variables[1].role, mv.variables[1].role_inferred))
        }
        _ => panic!("expected MV"),
    };
    assert_eq!(mv_roles(&session), ((VariableRole::Continuous, true), (VariableRole::Label, false)));

    run_cmds(
        &mut session,
        &[
            (
                "table.recodeLevels",
                json!({ "sheet_id": "mv-num", "col": 0, "mappings": [{ "from": "1", "to": "Placebo" }, { "from": "2", "to": "LowDose" }, { "from": "3", "to": "HighDose" }] }),
            ),
            ("table.recodeLevels", json!({ "sheet_id": "mv-num", "col": 1, "mappings": [{ "from": "1", "to": "Subject-1" }] })),
        ],
    );
    assert_eq!(mv_roles(&session), ((VariableRole::Categorical, true), (VariableRole::Label, false)));
    let TableArchetype::MultipleVariables(mv_after) = &ds(&session, "mv-num").archetype else { panic!("expected MV") };
    assert_eq!(mv_after.variables[0].ordered_levels().into_iter().map(|s| s.level).collect::<Vec<_>>(), vec!["Placebo", "LowDose", "HighDose"]);
}

#[test]
fn rv61_sheet_delete_checks_and_prunes_result_backed_graph_overlays_and_annotations() {
    let mut doc = ProjectDocument::new("RV-61 Graph Result Deps").unwrap();
    let ds_xy = xy_sheet("d-xy", "XY Data", "X", &[1.0, 2.0, 3.0, 4.0], &[("Y", &[2.1, 3.9, 6.2, 8.0])]);
    let ds_col = col_sheet("d-col", "Col Data", &[("A", &[10.0, 11.0, 12.0]), ("B", &[20.0, 21.0, 22.0]), ("C", &[30.0, 31.0, 32.0])]);

    let mut gspec = GraphSpec::default_for_data_sheet(&ds_col);
    gspec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("res-fit"),
        sample_segments: 128,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    gspec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res-anova"),
        group_a: 0,
        group_b: 1,
        style: BracketStyle::Square,
        label_mode: BracketLabelMode::SummaryStars,
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 9.0,
        bracket_step_pt: 14.0,
    });
    gspec.compact_letter_display = Some(CompactLetterDisplaySpec::new("res-anova"));
    gspec.pairwise_display_mode = PairwiseDisplayMode::BracketsAndCompactLetterDisplay;
    gspec.annotations = vec![
        GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("res-anova"),
            tab_id: None,
            bounds_pt: RectPt::new(10.0, 10.0, 120.0, 60.0),
            selected_rows: None,
            selected_cols: None,
            style: Default::default(),
        },
        GraphAnnotation::ModelEquation {
            result_sheet_id: SheetId::new("res-fit"),
            data_set_index: 0,
            mode: EquationDisplayMode::Substituted,
            bounds_pt: RectPt::new(20.0, 20.0, 120.0, 40.0),
            font_size_pt: 10.0,
            bordered: true,
        },
        GraphAnnotation::TextBox { bounds_pt: RectPt::new(5.0, 5.0, 80.0, 20.0), markdown: "Keep me".into(), bordered: false },
    ];

    doc.add_data_sheet(ds_xy).unwrap();
    doc.add_data_sheet(ds_col).unwrap();
    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("res-fit", "d-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap())
            .unwrap(),
    )
    .unwrap();
    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("res-anova", "d-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap(),
    )
    .unwrap();
    doc.add_graph_sheet(GraphSheet {
        id: SheetId::new("g-1"),
        title: "Combined Plot".into(),
        initialized: true,
        spec: gspec,
        floating_notes: Vec::new(),
        color_tag: None,
        origin: None,
    })
    .unwrap();
    dag::recalculate_project(&mut doc).unwrap();
    assert_eq!(doc.upstream_dependencies(&SheetId::new("g-1")), vec![SheetId::new("d-col"), SheetId::new("res-anova"), SheetId::new("res-fit")]);

    let mut session = Session::from_document(doc);
    for rid in ["res-fit", "res-anova"] {
        assert!(
            assert_err_code(&mut session, "sheet.delete", json!({ "sheet_id": rid, "cascade": false }), ErrorCode::Conflict)
                .message()
                .contains("g-1")
        );
    }

    ok_cmd(&mut session, "sheet.delete", json!({ "sheet_id": "res-fit", "cascade": true }));
    let g1 = gs(&session, "g-1");
    assert!(
        g1.spec.series[0].regression_overlay.is_none()
            && g1.spec.comparisons.len() == 1
            && g1.spec.compact_letter_display.is_some()
            && g1.spec.annotations.len() == 2
    );
    assert_eq!(session.document().upstream_dependencies(&SheetId::new("g-1")), vec![SheetId::new("d-col"), SheetId::new("res-anova")]);

    ok_cmd(&mut session, "sheet.delete", json!({ "sheet_id": "res-anova", "cascade": true }));
    let g2 = gs(&session, "g-1");
    assert!(
        g2.spec.comparisons.is_empty()
            && g2.spec.compact_letter_display.is_none()
            && g2.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly
            && g2.spec.annotations.len() == 1
    );
    assert!(matches!(&g2.spec.annotations[0], GraphAnnotation::TextBox { markdown, .. } if markdown == "Keep me"));
    assert_eq!(session.document().upstream_dependencies(&SheetId::new("g-1")), vec![SheetId::new("d-col")]);

    ok_cmd(&mut session, "sheet.delete", json!({ "sheet_id": "d-col", "cascade": true }));
    assert!(session.document().graph_sheet(&SheetId::new("g-1")).is_none());
}
