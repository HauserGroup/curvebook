//! Regression tests for ledger items RV-56, RV-57, and RV-61 (`crates/doc` & `crates/engine/src/cmd/{table,sheet}.rs`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, BracketStyle, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, CompactLetterDisplaySpec,
    DataSheet, EquationDisplayMode, ErrorCode, GraphAnnotation, GraphSheet, GraphSpec, LevelOrderKind, LevelOrderSpec, MultipleVariablesTable,
    MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec, OneWayAnovaSpec, PairwiseBracketSpec, PairwiseDisplayMode, ProjectDocument,
    RectPt, RegressionOverlaySpec, ResultSheet, SheetId, SizeChannelMapping, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::{Session, dag};
use curvebook_palette::PaletteId;
use serde_json::json;

#[test]
fn rv56_recode_levels_updates_variable_explicit_order_and_graph_overrides_with_dedup() {
    let mut doc = ProjectDocument::new("RV-56 Recode Order").expect("doc");

    let mut mv = MultipleVariablesTable::new(3, 4).expect("mv");
    mv.variables[0].title = String::from("Arm");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[0].cells = vec![Cell::text("ctrl"), Cell::text("drugA"), Cell::text("veh"), Cell::text("drugB")];
    mv.variables[0].level_order = LevelOrderSpec {
        order: LevelOrderKind::Explicit(vec![String::from("ctrl"), String::from("drugA"), String::from("veh"), String::from("drugB")]),
        reversed: false,
    };

    mv.variables[1].title = String::from("Response");
    mv.variables[1].cells = vec![Cell::number(10.0), Cell::number(25.0), Cell::number(11.0), Cell::number(30.0)];

    mv.variables[2].title = String::from("Cohort");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[2].cells = vec![Cell::text("c1"), Cell::text("c2"), Cell::text("c1"), Cell::text("c2")];

    let ds = DataSheet::new("mv-1", "Study", TableArchetype::MultipleVariables(mv)).expect("ds");
    let mut gspec = GraphSpec::default_for_data_sheet(&ds);
    gspec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Categorical {
            column: 2,
            palette: PaletteId::OkabeIto,
            level_order_override: Some(LevelOrderSpec {
                order: LevelOrderKind::Explicit(vec![String::from("c2"), String::from("c1")]),
                reversed: false,
            }),
        },
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: Some(LevelOrderSpec {
            order: LevelOrderKind::Explicit(vec![String::from("veh"), String::from("drugB"), String::from("ctrl"), String::from("drugA")]),
            reversed: false,
        }),
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });
    doc.add_data_sheet(ds).expect("add mv-1");
    doc.add_graph_sheet(GraphSheet {
        id: SheetId::new("g-mv"),
        title: String::from("MV Plot"),
        initialized: true,
        spec: gspec,
        floating_notes: Vec::new(),
        color_tag: None,
    })
    .expect("add g-mv");

    // Unrelated MV sheet + graph whose override must remain untouched when mv-1 is recoded.
    let mut mv2 = MultipleVariablesTable::new(2, 2).expect("mv2");
    mv2.variables[0].lock_role(VariableRole::Categorical);
    mv2.variables[0].cells = vec![Cell::text("ctrl"), Cell::text("drugA")];
    mv2.variables[1].cells = vec![Cell::number(1.0), Cell::number(2.0)];
    let ds2 = DataSheet::new("mv-2", "Other Study", TableArchetype::MultipleVariables(mv2)).expect("ds2");
    let mut gspec2 = GraphSpec::default_for_data_sheet(&ds2);
    if let Some(enc2) = &mut gspec2.multivariate {
        enc2.x_level_order_override =
            Some(LevelOrderSpec { order: LevelOrderKind::Explicit(vec![String::from("ctrl"), String::from("drugA")]), reversed: false });
    }
    doc.add_data_sheet(ds2).expect("add mv-2");
    doc.add_graph_sheet(GraphSheet {
        id: SheetId::new("g-other"),
        title: String::from("Other Plot"),
        initialized: true,
        spec: gspec2,
        floating_notes: Vec::new(),
        color_tag: None,
    })
    .expect("add g-other");

    let mut session = Session::from_document(doc);

    // Recode both `ctrl` and `veh` -> `control` on mv-1 column 0 (`Arm`).
    session
        .execute(
            "table.recodeLevels",
            json!({
                "sheet_id": "mv-1",
                "col": 0,
                "mappings": [
                    { "from": "ctrl", "to": "control" },
                    { "from": "veh", "to": "control" }
                ]
            }),
        )
        .expect("recode Arm");

    let TableArchetype::MultipleVariables(mv_after) = &session.document().data_sheet(&SheetId::new("mv-1")).expect("mv-1").archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(
        mv_after.variables[0].level_order.order,
        LevelOrderKind::Explicit(vec![String::from("control"), String::from("drugA"), String::from("drugB")])
    );
    let ordered_names: Vec<String> = mv_after.variables[0].ordered_levels().into_iter().map(|s| s.level).collect();
    assert_eq!(ordered_names, vec!["control", "drugA", "drugB"]);

    let g_mv = session.document().graph_sheet(&SheetId::new("g-mv")).expect("g-mv");
    let enc = g_mv.spec.multivariate.as_ref().expect("multivariate");
    assert_eq!(
        enc.x_level_order_override.as_ref().expect("x override").order,
        LevelOrderKind::Explicit(vec![String::from("control"), String::from("drugB"), String::from("drugA")])
    );

    // Recode `c2` -> `CohortTwo` on mv-1 column 2 (`Cohort`) and verify colour-channel override updates too.
    session
        .execute(
            "table.recodeLevels",
            json!({
                "sheet_id": "mv-1",
                "col": 2,
                "mappings": [{ "from": "c2", "to": "CohortTwo" }]
            }),
        )
        .expect("recode Cohort");

    let g_mv2 = session.document().graph_sheet(&SheetId::new("g-mv")).expect("g-mv");
    let ColorChannelMapping::Categorical { level_order_override: Some(color_ov), .. } =
        &g_mv2.spec.multivariate.as_ref().expect("multivariate").color
    else {
        panic!("expected Categorical color channel with override");
    };
    assert_eq!(color_ov.order, LevelOrderKind::Explicit(vec![String::from("CohortTwo"), String::from("c1")]));

    // Unrelated graph on mv-2 is untouched.
    let g_other = session.document().graph_sheet(&SheetId::new("g-other")).expect("g-other");
    assert_eq!(
        g_other.spec.multivariate.as_ref().expect("mv2").x_level_order_override.as_ref().expect("ov2").order,
        LevelOrderKind::Explicit(vec![String::from("ctrl"), String::from("drugA")])
    );
}

#[test]
fn rv57_recode_levels_refreshes_inferred_continuous_role_to_categorical() {
    let mut session = Session::new();
    session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "mv-num",
                "archetype": "multiple_variables",
                "cols": 2,
                "rows": 3,
                "auto_graph": false
            }),
        )
        .expect("create mv-num");

    for (r, code) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        session.execute("table.setCell", json!({ "sheet_id": "mv-num", "row": r, "col": 0, "value": code })).expect("set numeric code");
        session.execute("table.setCell", json!({ "sheet_id": "mv-num", "row": r, "col": 1, "value": code })).expect("set locked col");
    }
    session.execute("table.setVariableRole", json!({ "sheet_id": "mv-num", "col": 1, "role": "label" })).expect("lock col 1 as Label");

    {
        let TableArchetype::MultipleVariables(mv_before) = &session.document().data_sheet(&SheetId::new("mv-num")).expect("mv-num").archetype else {
            panic!("expected MultipleVariables");
        };
        assert_eq!(mv_before.variables[0].role, VariableRole::Continuous);
        assert!(mv_before.variables[0].role_inferred);
        assert_eq!(mv_before.variables[1].role, VariableRole::Label);
        assert!(!mv_before.variables[1].role_inferred);
    }

    session
        .execute(
            "table.recodeLevels",
            json!({
                "sheet_id": "mv-num",
                "col": 0,
                "mappings": [
                    { "from": "1", "to": "Placebo" },
                    { "from": "2", "to": "LowDose" },
                    { "from": "3", "to": "HighDose" }
                ]
            }),
        )
        .expect("recode col 0");
    session
        .execute(
            "table.recodeLevels",
            json!({
                "sheet_id": "mv-num",
                "col": 1,
                "mappings": [{ "from": "1", "to": "Subject-1" }]
            }),
        )
        .expect("recode locked col 1");

    let TableArchetype::MultipleVariables(mv_after) = &session.document().data_sheet(&SheetId::new("mv-num")).expect("mv-num").archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv_after.variables[0].role, VariableRole::Categorical);
    assert!(mv_after.variables[0].role_inferred);
    let levels: Vec<String> = mv_after.variables[0].ordered_levels().into_iter().map(|s| s.level).collect();
    assert_eq!(levels, vec!["Placebo", "LowDose", "HighDose"]);

    // Explicitly locked role (`role_inferred == false`) stays locked.
    assert_eq!(mv_after.variables[1].role, VariableRole::Label);
    assert!(!mv_after.variables[1].role_inferred);
}

#[test]
fn rv61_sheet_delete_checks_and_prunes_result_backed_graph_overlays_and_annotations() {
    let mut doc = ProjectDocument::new("RV-61 Graph Result Deps").expect("doc");

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4).expect("xy");
    for (i, (x, y)) in [(1.0, 2.1), (2.0, 3.9), (3.0, 6.2), (4.0, 8.0)].into_iter().enumerate() {
        xy.set_x_cell(i, Cell::number(x)).expect("x");
        xy.data_sets[0].cells[i][0] = Cell::number(y);
    }
    let ds_xy = DataSheet::new("d-xy", "XY Data", TableArchetype::Xy(xy)).expect("ds_xy");

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 3).expect("col");
    for (c, vals) in [[10.0, 11.0, 12.0], [20.0, 21.0, 22.0], [30.0, 31.0, 32.0]].into_iter().enumerate() {
        for (r, v) in vals.into_iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let ds_col = DataSheet::new("d-col", "Col Data", TableArchetype::Column(col)).expect("ds_col");

    let fit_spec = AnalysisSpec::new("res-fit", "d-xy", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).expect("fit spec");
    let anova_spec = AnalysisSpec::new("res-anova", "d-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("anova spec");

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
    gspec.annotations.push(GraphAnnotation::EmbeddedResultTable {
        result_sheet_id: SheetId::new("res-anova"),
        tab_id: None,
        bounds_pt: RectPt::new(10.0, 10.0, 120.0, 60.0),
        selected_rows: None,
        selected_cols: None,
        style: curvebook_doc::EmbeddedTableStyle::default(),
    });
    gspec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: SheetId::new("res-fit"),
        data_set_index: 0,
        mode: EquationDisplayMode::Substituted,
        bounds_pt: RectPt::new(20.0, 20.0, 120.0, 40.0),
        font_size_pt: 10.0,
        bordered: true,
    });
    gspec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(5.0, 5.0, 80.0, 20.0),
        markdown: String::from("Keep me"),
        bordered: false,
    });

    doc.add_data_sheet(ds_xy).expect("add d-xy");
    doc.add_data_sheet(ds_col).expect("add d-col");
    doc.add_result_sheet(ResultSheet::new(fit_spec).expect("res-fit")).expect("add res-fit");
    doc.add_result_sheet(ResultSheet::new(anova_spec).expect("res-anova")).expect("add res-anova");
    doc.add_graph_sheet(GraphSheet {
        id: SheetId::new("g-1"),
        title: String::from("Combined Plot"),
        initialized: true,
        spec: gspec,
        floating_notes: Vec::new(),
        color_tag: None,
    })
    .expect("add g-1");
    dag::recalculate_project(&mut doc).expect("recalc");

    assert_eq!(doc.upstream_dependencies(&SheetId::new("g-1")), vec![SheetId::new("d-col"), SheetId::new("res-anova"), SheetId::new("res-fit")]);

    let mut session = Session::from_document(doc);

    // 1. `cascade: false` refuses to delete either result sheet while `g-1` references it.
    let err_fit = session.execute("sheet.delete", json!({ "sheet_id": "res-fit", "cascade": false })).expect_err("must conflict on res-fit");
    assert_eq!(err_fit.code, ErrorCode::Conflict);
    assert!(err_fit.message().contains("g-1"));

    let err_anova = session.execute("sheet.delete", json!({ "sheet_id": "res-anova", "cascade": false })).expect_err("must conflict on res-anova");
    assert_eq!(err_anova.code, ErrorCode::Conflict);
    assert!(err_anova.message().contains("g-1"));

    // 2. `cascade: true` on `res-fit` keeps `g-1` alive (its primary data sheet `d-col` still exists),
    //    pruning `regression_overlay` and `ModelEquation` while preserving `res-anova` bindings and `TextBox`.
    session.execute("sheet.delete", json!({ "sheet_id": "res-fit", "cascade": true })).expect("delete res-fit");
    let g_after_fit = session.document().graph_sheet(&SheetId::new("g-1")).expect("g-1 survives");
    assert!(g_after_fit.spec.series[0].regression_overlay.is_none());
    assert_eq!(g_after_fit.spec.comparisons.len(), 1);
    assert!(g_after_fit.spec.compact_letter_display.is_some());
    assert_eq!(g_after_fit.spec.annotations.len(), 2);
    assert_eq!(session.document().upstream_dependencies(&SheetId::new("g-1")), vec![SheetId::new("d-col"), SheetId::new("res-anova")]);

    // 3. `cascade: true` on `res-anova` keeps `g-1` alive, pruning `comparisons`, `compact_letter_display`,
    //    and `EmbeddedResultTable`, resetting `pairwise_display_mode`, and leaving zero dangling `result_sheet_id`s.
    session.execute("sheet.delete", json!({ "sheet_id": "res-anova", "cascade": true })).expect("delete res-anova");
    let g_after_anova = session.document().graph_sheet(&SheetId::new("g-1")).expect("g-1 still survives");
    assert!(g_after_anova.spec.comparisons.is_empty());
    assert!(g_after_anova.spec.compact_letter_display.is_none());
    assert_eq!(g_after_anova.spec.pairwise_display_mode, PairwiseDisplayMode::BracketsOnly);
    assert_eq!(g_after_anova.spec.annotations.len(), 1);
    assert!(matches!(&g_after_anova.spec.annotations[0], GraphAnnotation::TextBox { markdown, .. } if markdown == "Keep me"));
    assert_eq!(session.document().upstream_dependencies(&SheetId::new("g-1")), vec![SheetId::new("d-col")]);

    // 4. Deleting the graph's primary data sheet `d-col` with `cascade: true` deletes `g-1` too.
    session.execute("sheet.delete", json!({ "sheet_id": "d-col", "cascade": true })).expect("delete d-col");
    assert!(session.document().graph_sheet(&SheetId::new("g-1")).is_none());
}
