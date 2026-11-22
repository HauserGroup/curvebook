//! Regression tests covering all 12 confirmed `SEC-14` findings (`FA-SEC14-01..05`, `FA-SEC14-07..13`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BracketLabelMode, BracketStyle, Cell, ColorChannelMapping, ColumnDesignHint, ColumnTable, ContingencySpec,
    ContingencyTable, DataSheet, EquationDisplayMode, FramePreset, GraphAnnotation, GraphAxisScale, GraphKind, GraphSheet, LevelOrderKind,
    LevelOrderSpec, MAX_SHEETS_PER_PROJECT, MultipleVariablesTable, MultivariateGraphEncoding, MultivariatePlotMode, MultivariateSpec,
    OneWayAnovaSpec, PairwiseBracketSpec, PairwiseComparisonRecord, ProjectDocument, RectPt, ResultCell, ResultSheet, SizeChannelMapping,
    SubcolumnMode, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_math::DenseMatrix;
use curvebook_palette::PaletteId;
use curvebook_stats::multivariate::pca_analysis;
use curvebook_stats::run_analysis;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::{
    AnalyzeDialogState, AxisTarget, BubblePlotInspectorState, CommandPaletteState, EditAnalysisParametersState, FormatAxesState,
    PairwiseComparisonsState,
};
use curvebook_ui_egui::dialogs_r9::InsertModelEquationDialogState;
use curvebook_ui_egui::grid_view::{GridViewState, ImportReshapeState, MoveDirection};
use curvebook_ui_egui::menus::{UI_COMMANDS, command_tooltip};
use curvebook_ui_egui::power_wizard::PowerWizardState;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Map, Value, json};

#[test]
fn fa_sec14_01_pairwise_clear_all_multi_bracket_and_style_edits_persist() {
    let mut doc = ProjectDocument::new("SEC14-01 Pairwise").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).unwrap();
    for (c, base) in [10.0, 20.0, 35.0].into_iter().enumerate() {
        col.data_sets[c].title = format!("G{c}");
        for r in 0..4 {
            col.data_sets[c].cells[r][0] = Cell::number(base + r as f64);
        }
    }
    let ds = DataSheet::new("ds-pw", "Groups", TableArchetype::Column(col)).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-pw", &ds).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    doc.add_graph_sheet(gs.clone()).unwrap();

    let mut res_spec = AnalysisSpec::new("res-pw", "ds-pw", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
    res_spec.selected_data_sets = vec![0, 1, 2];
    let mut rs = ResultSheet::new(res_spec).unwrap();
    rs.pairwise_comparisons = vec![
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("G0"),
            label_b: String::from("G1"),
            estimate_diff: Some(-10.0),
            raw_p_or_q: 0.01,
            is_fdr_q: false,
            significant: true,
        },
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 2,
            label_a: String::from("G0"),
            label_b: String::from("G2"),
            estimate_diff: Some(-25.0),
            raw_p_or_q: 0.001,
            is_fdr_q: false,
            significant: true,
        },
        PairwiseComparisonRecord {
            group_a: 1,
            group_b: 2,
            label_a: String::from("G1"),
            label_b: String::from("G2"),
            estimate_diff: Some(-15.0),
            raw_p_or_q: 0.25,
            is_fdr_q: false,
            significant: false,
        },
    ];
    doc.add_result_sheet(rs).unwrap();

    // 1. Fresh single-bracket auto-add still uses `graph.addComparisonBrackets`.
    let mut pw_single = PairwiseComparisonsState::default();
    pw_single.sync_from_graph(&gs, &doc);
    pw_single.selected_result_sheet_id = Some("res-pw".into());
    pw_single.brackets.push(PairwiseBracketSpec {
        result_sheet_id: "res-pw".into(),
        group_a: 0,
        group_b: 1,
        style: BracketStyle::Square,
        label_mode: BracketLabelMode::SummaryStars,
        hide_if_non_significant: false,
        stroke_width_pt: 0.9,
        font_size_pt: 9.0,
        bracket_step_pt: 12.0,
    });
    pw_single.brackets_dirty = true;
    let intents_single = pw_single.commit_intents();
    assert_eq!(intents_single[0].command_id, "graph.addComparisonBrackets");

    // 2. Multi-bracket + style/label/hide edit emits `graph.setSpec` retaining all brackets.
    let mut sess = Session::from_document(doc);
    sess.execute("graph.addComparisonBrackets", json!({ "sheet_id": "g-pw", "result_sheet_id": "res-pw", "group_a": 0, "group_b": 1 })).unwrap();
    sess.execute("graph.addComparisonBrackets", json!({ "sheet_id": "g-pw", "result_sheet_id": "res-pw", "group_a": 0, "group_b": 2 })).unwrap();
    gs = sess.document().graph_sheet(&"g-pw".into()).unwrap().clone();
    assert_eq!(gs.spec.comparisons.len(), 2);

    let mut pw = PairwiseComparisonsState::default();
    pw.sync_from_graph(&gs, sess.document());
    assert!(pw.had_brackets_on_graph);
    pw.bracket_style = BracketStyle::CurlyBrace;
    pw.label_mode = BracketLabelMode::NumericPOrQ;
    pw.hide_non_significant = true;
    pw.brackets_dirty = true;
    for intent in pw.commit_intents() {
        sess.execute(&intent.command_id, intent.params).unwrap();
    }
    let updated_gs = sess.document().graph_sheet(&"g-pw".into()).unwrap();
    assert_eq!(updated_gs.spec.comparisons.len(), 2);
    assert!(updated_gs.spec.comparisons.iter().all(|b| b.style == BracketStyle::CurlyBrace));
    assert!(updated_gs.spec.comparisons.iter().all(|b| b.label_mode == BracketLabelMode::NumericPOrQ));
    assert!(updated_gs.spec.comparisons.iter().all(|b| b.hide_if_non_significant));

    // 3. Clearing ALL brackets emits `graph.setSpec` with empty `comparisons` and clears graph.
    pw.sync_from_graph(updated_gs, sess.document());
    pw.brackets.clear();
    pw.brackets_dirty = true;
    let clear_intents = pw.commit_intents();
    assert_eq!(clear_intents.len(), 1);
    assert_eq!(clear_intents[0].command_id, "graph.setSpec");
    sess.execute(&clear_intents[0].command_id, clear_intents[0].params.clone()).unwrap();
    assert!(sess.document().graph_sheet(&"g-pw".into()).unwrap().spec.comparisons.is_empty());
}

#[test]
fn fa_sec14_02_bubble_plot_preserves_x_level_order_override() {
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[2].role = VariableRole::Continuous;
    for (r, lvl) in ["High", "Mid", "Low", "High"].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(lvl);
        mv.variables[1].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[2].cells[r] = Cell::number((r + 2) as f64);
    }
    let ds = DataSheet::new("ds-mv", "MV", TableArchetype::MultipleVariables(mv)).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-mv", &ds).unwrap();
    let override_order =
        LevelOrderSpec { order: LevelOrderKind::Explicit(vec![String::from("High"), String::from("Mid"), String::from("Low")]), reversed: true };
    gs.spec.kind = GraphKind::MultipleVariables;
    gs.spec.multivariate = Some(MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column: 0,
        y_column: 1,
        color: ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0)),
        size: SizeChannelMapping::Constant { radius_pt: 5.0 },
        label_column: None,
        x_level_order_override: Some(override_order.clone()),
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    });

    let mut doc = ProjectDocument::new("SEC14-02 Bubble").unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs.clone()).unwrap();

    let mut bp = BubblePlotInspectorState::default();
    bp.sync_from_graph(&gs, &doc);
    assert_eq!(bp.x_level_order_override, Some(override_order.clone()));
    bp.show_convex_hulls = true;

    let intent = bp.commit_intent().unwrap();
    let mut sess = Session::from_document(doc);
    sess.execute(&intent.command_id, intent.params).unwrap();
    let saved = sess.document().graph_sheet(&"g-mv".into()).unwrap().spec.multivariate.as_ref().unwrap();
    assert!(saved.show_convex_hulls);
    assert_eq!(saved.x_level_order_override, Some(override_order));
}

#[test]
fn fa_sec14_03_pcr_end_to_end_distinct_from_pca_with_leakage_guard_and_methods_report() {
    let mut mv = MultipleVariablesTable::new(5, 8).unwrap();
    mv.variables[0].title = String::from("X1");
    mv.variables[1].title = String::from("X2");
    mv.variables[2].title = String::from("X3");
    mv.variables[3].title = String::from("ResponseY");
    mv.variables[4].title = String::from("GroupCat");
    mv.variables[4].lock_role(VariableRole::Categorical);
    let data = [
        [1.0, 2.1, 0.5, 9.2],
        [2.0, 4.0, 1.1, 15.1],
        [3.0, 6.2, 1.4, 21.8],
        [4.0, 7.9, 2.2, 27.6],
        [5.0, 10.1, 2.5, 34.4],
        [6.0, 12.0, 3.1, 40.3],
        [7.0, 13.8, 3.8, 46.7],
        [8.0, 16.2, 4.0, 53.1],
    ];
    for (r, row) in data.iter().enumerate() {
        for c in 0..4 {
            mv.variables[c].cells[r] = Cell::number(row[c]);
        }
        mv.variables[4].cells[r] = Cell::text(if r % 2 == 0 { "A" } else { "B" });
    }
    let ds = DataSheet::new("mv-pcr", "Cohort", TableArchetype::MultipleVariables(mv)).unwrap();

    // 1. Auto titles & stats differ strictly between PCA (`None`) and PCR (`Some(3)`).
    let mut pca_spec = AnalysisSpec::new(
        "res-pca",
        "mv-pcr",
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: None, pcr_outcome_column: None }),
    )
    .unwrap();
    pca_spec.selected_data_sets = vec![0, 1, 2];
    assert_eq!(pca_spec.effective_title(), "Principal component analysis");
    let pca_res = run_analysis(&ds, &pca_spec).unwrap();
    assert!(!pca_res.scalars.iter().any(|s| s.key == "PCR R squared"));

    let mut pcr_spec = AnalysisSpec::new(
        "res-pcr",
        "mv-pcr",
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: None, pcr_outcome_column: Some(3) }),
    )
    .unwrap();
    pcr_spec.selected_data_sets = vec![0, 1, 2];
    assert_eq!(pcr_spec.effective_title(), "Principal component regression");
    let pcr_res = run_analysis(&ds, &pcr_spec).unwrap();
    let r2 = pcr_res.scalars.iter().find(|s| s.key == "PCR R squared").unwrap().value;
    assert!(r2 > 0.99 && r2 <= 1.0 + 1e-12, "unexpected PCR R^2: {r2}");

    // Verify back-transformed predictor coefficients exactly reproduce predicted Y.
    let pred_tab = pcr_res.tabs.iter().find(|t| t.id == "pcr-predictors").unwrap();
    let diag_tab = pcr_res.tabs.iter().find(|t| t.id == "pcr-diagnostics").unwrap();
    let b0 = match pred_tab.rows[0][0] {
        ResultCell::Number { value, .. } => value,
        _ => panic!("expected Intercept number"),
    };
    let b1 = match pred_tab.rows[1][0] {
        ResultCell::Number { value, .. } => value,
        _ => panic!("expected X1 coef"),
    };
    let b2 = match pred_tab.rows[2][0] {
        ResultCell::Number { value, .. } => value,
        _ => panic!("expected X2 coef"),
    };
    let b3 = match pred_tab.rows[3][0] {
        ResultCell::Number { value, .. } => value,
        _ => panic!("expected X3 coef"),
    };
    for (r, row) in data.iter().enumerate() {
        let yhat_orig = b0 + b1 * row[0] + b2 * row[1] + b3 * row[2];
        let yhat_tab = match diag_tab.rows[r][1] {
            ResultCell::Number { value, .. } => value,
            _ => panic!("expected Predicted Y number"),
        };
        assert!((yhat_orig - yhat_tab).abs() < 1e-9, "back-transformed yhat {yhat_orig} != {yhat_tab}");
    }
    // Cross-check PCA eigen/scores consistency against `pca_analysis` oracle.
    let flat_x: Vec<f64> = data.iter().flat_map(|row| row[..3].iter().copied()).collect();
    let mat = DenseMatrix::from_row_major(8, 3, flat_x).unwrap();
    let direct_pca = pca_analysis(&mat, &["X1".into(), "X2".into(), "X3".into()], true, None).unwrap();
    let pca_ev1 = pcr_res.scalars.iter().find(|s| s.key == "PC1 eigenvalue").unwrap().value;
    assert!((pca_ev1 - direct_pca.eigenvalues[0]).abs() < 1e-10);

    // 2. AnalyzeDialogState hoists predictors BEFORE outcome regardless of key order and rejects leakage & non-usize.
    let mut az = AnalyzeDialogState::default();
    az.open_for_data_sheet(&ds);
    az.select_catalog_entry("stats.pcr");
    let mut leak_patch = Map::new();
    leak_patch.insert(String::from("pcr_outcome_column"), json!(1));
    leak_patch.insert(String::from("selected_data_sets"), json!([0, 1, 2]));
    assert!(az.apply_field_patch(&leak_patch).is_err(), "reverse BTreeMap order must still catch target leakage");

    let mut bad_type = Map::new();
    bad_type.insert(String::from("pcr_outcome_column"), json!("invalid"));
    assert!(az.apply_field_patch(&bad_type).is_err());

    // `response_column` on non-PCR entry (`stats.multiple_regression`) must NOT trigger PCA leakage guard.
    let mut reg_az = AnalyzeDialogState::default();
    reg_az.open_for_data_sheet(&ds);
    let mut reg_patch = Map::new();
    reg_patch.insert(String::from("entry_id"), json!("stats.multiple_regression"));
    reg_patch.insert(String::from("selected_data_sets"), json!([0, 1, 2]));
    reg_patch.insert(String::from("response_column"), json!(2));
    reg_az.apply_field_patch(&reg_patch).unwrap();
    assert_eq!(reg_az.pcr_outcome_column, None);

    let mut ok_patch = Map::new();
    ok_patch.insert(String::from("selected_data_sets"), json!([0, 1, 2]));
    ok_patch.insert(String::from("pcr_outcome_column"), json!(3));
    az.apply_field_patch(&ok_patch).unwrap();
    let built_kind = az.build_analysis_kind().unwrap();
    assert!(matches!(built_kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column: Some(3), .. })));

    // 3. Engine run, methods report, EditAnalysisParametersState reopen/clear roundtrip.
    let mut doc = ProjectDocument::new("SEC14-03 Engine").unwrap();
    doc.add_data_sheet(ds).unwrap();
    let mut sess = Session::from_document(doc);
    let run_out = sess.execute("analysis.run", json!({ "source_sheet_id": "mv-pcr", "sheet_id": "res-pcr-1", "kind": built_kind })).unwrap();
    assert_eq!(run_out.payload["sheet_id"], json!("res-pcr-1"));

    let rep = sess.execute("result.generateReport", json!({ "sheet_id": "res-pcr-1", "format": "plain" })).unwrap();
    let rep_text = rep.payload["report"].as_str().unwrap();
    assert!(rep_text.contains("Principal component regression"), "methods report missing PCR prose: {rep_text}");
    assert!(rep_text.contains("ResponseY"), "methods report missing outcome column title: {rep_text}");

    let rs_saved = sess.document().result_sheet(&"res-pcr-1".into()).unwrap().clone();
    let mut ep = EditAnalysisParametersState::default();
    ep.open_for_result(&rs_saved);
    assert_eq!(ep.pcr_outcome_column, Some(3));
    let mut clear_patch = Map::new();
    clear_patch.insert(String::from("pcr_outcome_column"), Value::Null);
    ep.apply_field_patch(&clear_patch).unwrap();
    assert_eq!(ep.pcr_outcome_column, None);
    let upd_intent = ep.commit_intent().unwrap();
    assert_eq!(upd_intent.params["pcr_outcome_column"], Value::Null);
    sess.execute(&upd_intent.command_id, upd_intent.params).unwrap();
    let updated_rs = sess.document().result_sheet(&"res-pcr-1".into()).unwrap();
    assert!(matches!(updated_rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column: None, .. })));
    assert!(!updated_rs.scalars.iter().any(|s| s.key == "PCR R squared"));
}

#[test]
fn fa_sec14_04_edit_params_contingency_reopen_sync_and_paired_graph_differences() {
    // 1. Contingency reopen restores `contingency_*_checked` from persisted `compute_odds_and_risk_ratios`.
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    ct.data_sets[0].cells[0][0] = Cell::number(20.0);
    ct.data_sets[1].cells[0][0] = Cell::number(5.0);
    ct.data_sets[0].cells[1][0] = Cell::number(10.0);
    ct.data_sets[1].cells[1][0] = Cell::number(25.0);
    let ds_ct = DataSheet::new("ct-1", "2x2", TableArchetype::Contingency(ct)).unwrap();
    let rs_ct = ResultSheet::new(
        AnalysisSpec::new(
            "res-ct",
            "ct-1",
            AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, ..ContingencySpec::default() }),
        )
        .unwrap(),
    )
    .unwrap();

    let mut ep = EditAnalysisParametersState::default();
    ep.open_for_result(&rs_ct);
    assert!(ep.contingency_odds_ratio_checked);
    assert!(ep.contingency_relative_risk_checked);
    assert!(ep.contingency_attributable_risk_checked);

    // Invalid CI method rejected; unchecking attributable risk while leaving OR checked works cleanly.
    let mut bad_ci = Map::new();
    bad_ci.insert(String::from("edit.contingency.ci.baptista_pike"), json!("BogusCI"));
    assert!(ep.apply_field_patch(&bad_ci).is_err());

    let mut check_ar = Map::new();
    check_ar.insert(String::from("edit.contingency.odds_ratio"), json!(false));
    ep.apply_field_patch(&check_ar).unwrap();
    assert!(!ep.contingency_attributable_risk_checked);
    let mut recheck_ar = Map::new();
    recheck_ar.insert(String::from("edit.contingency.attributable_risk"), json!(true));
    ep.apply_field_patch(&recheck_ar).unwrap();
    assert!(matches!(ep.draft.as_ref().unwrap().kind, AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, .. })));

    // 2. Paired TwoGroupComparison `graph_differences` creates a Before-After graph on commit and resets on Unpaired switch.
    let mut doc = ProjectDocument::new("SEC14-04 TwoGroup").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 4).unwrap();
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        col.data_sets[1].cells[r][0] = Cell::number(14.0 + (r as f64) * 2.5);
    }
    let ds_tg = DataSheet::new("ds-tg", "Paired", TableArchetype::Column(col)).unwrap();
    doc.add_data_sheet(ds_ct).unwrap();
    doc.add_data_sheet(ds_tg).unwrap();
    let rs_mw = ResultSheet::new(
        AnalysisSpec::new(
            "res-mw",
            "ds-tg",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..TwoGroupSpec::default() }),
        )
        .unwrap(),
    )
    .unwrap();
    ep.open_for_result(&rs_mw);
    assert!(ep.hodges_lehmann_ci, "MannWhitneyU reopen should reflect Hodges-Lehmann CI");

    let rs_paired = ResultSheet::new(
        AnalysisSpec::new(
            "res-p",
            "ds-tg",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }),
        )
        .unwrap(),
    )
    .unwrap();
    doc.add_result_sheet(rs_paired.clone()).unwrap();
    ep.open_for_result(&rs_paired);
    let mut gd_patch = Map::new();
    gd_patch.insert(String::from("edit.two_group.graph_differences"), json!(true));
    ep.apply_field_patch(&gd_patch).unwrap();
    let intents = ep.commit_intents(Some(&doc));
    assert!(intents.iter().any(|i| i.command_id == "graph.create"));
    let mut sess = Session::from_document(doc);
    for i in intents {
        sess.execute(&i.command_id, i.params).unwrap();
    }
    assert!(
        sess.document().graph_sheets.iter().any(|g| g
            .spec
            .series
            .iter()
            .any(|s| matches!(s.gdo, curvebook_doc::GraphicalDataObject::BeforeAfterLines { .. })))
    );

    // Switching back to Unpaired resets incompatible `graph_differences` flag cleanly.
    let mut unpaired = Map::new();
    unpaired.insert(String::from("two_group_method"), json!("unpaired_t"));
    ep.apply_field_patch(&unpaired).unwrap();
    assert!(!ep.graph_differences);
}

#[test]
fn fa_sec14_05_format_axes_preserves_multi_axis_edits_and_y2_across_tab_switches() {
    let xy = XyTable::new(SubcolumnMode::Single, 1, 4).unwrap();
    let ds = DataSheet::new("d-ax", "Assay", TableArchetype::Xy(xy)).unwrap();
    let gs = GraphSheet::new_uninitialized_for_data("g-ax", &ds).unwrap();
    let mut doc = ProjectDocument::new("SEC14-05 Axes").unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs.clone()).unwrap();

    let mut fa = FormatAxesState::default();
    fa.sync_from_graph(&gs, AxisTarget::X);

    // Edit X axis: title + Log10 scale + bounds.
    let mut x_patch = Map::new();
    x_patch.insert(String::from("axes.title"), json!("Dose (nM)"));
    x_patch.insert(String::from("axes.scale"), json!("log10"));
    x_patch.insert(String::from("axes.min"), json!(1.0));
    x_patch.insert(String::from("axes.max"), json!(1000.0));
    fa.apply_field_patch(&x_patch, Some(&doc)).unwrap();

    // Switch to Y axis, edit Y title & tick interval & enable Y2.
    let mut y_patch = Map::new();
    y_patch.insert(String::from("axes.target_axis"), json!("Y"));
    y_patch.insert(String::from("axes.title"), json!("Response (%)"));
    y_patch.insert(String::from("axes.major_tick_interval"), json!(25.0));
    y_patch.insert(String::from("axes.enable_y2"), json!(true));
    fa.apply_field_patch(&y_patch, Some(&doc)).unwrap();

    // Switch to Y2, edit Y2 title & threshold, then switch back to X.
    let mut y2_patch = Map::new();
    y2_patch.insert(String::from("axes.target_axis"), json!("Y2"));
    y2_patch.insert(String::from("axes.title"), json!("Normalized Y2"));
    fa.apply_field_patch(&y2_patch, Some(&doc)).unwrap();
    assert!(fa.add_threshold(42.0, None, Some(String::from("EC50")), curvebook_palette::Rgba8::rgba(200, 0, 0, 255)));

    let mut back_to_x = Map::new();
    back_to_x.insert(String::from("axes.target_axis"), json!("X"));
    fa.apply_field_patch(&back_to_x, Some(&doc)).unwrap();
    assert_eq!(fa.axis_draft.title, "Dose (nM)");
    assert_eq!(fa.axis_draft.segments[0].scale, GraphAxisScale::Log10);
    assert!(fa.enable_y2);

    // Commit while X tab is active — X, Y, Y2 edits + DualY preset must all commit!
    let intent = fa.commit_intent().unwrap();
    let mut sess = Session::from_document(doc);
    sess.execute(&intent.command_id, intent.params).unwrap();
    let saved = sess.document().graph_sheet(&"g-ax".into()).unwrap();
    assert_eq!(saved.spec.x_axis.title, "Dose (nM)");
    assert_eq!(saved.spec.x_axis.segments[0].scale, GraphAxisScale::Log10);
    assert_eq!(saved.spec.y_axis.title, "Response (%)");
    assert_eq!(saved.spec.y_axis.segments[0].major_tick_interval, Some(25.0));
    assert_eq!(saved.spec.frame_preset, FramePreset::DualYSecondaryRight);
    let y2 = saved.spec.y2_axis.as_ref().unwrap();
    assert_eq!(y2.title, "Normalized Y2");
    assert_eq!(y2.thresholds.len(), 1);

    // Disabling Y2 clears `y2_axis` cleanly even when X tab is active.
    fa.sync_from_graph(saved, AxisTarget::X);
    let mut disable_y2 = Map::new();
    disable_y2.insert(String::from("axes.enable_y2"), json!(false));
    fa.apply_field_patch(&disable_y2, Some(sess.document())).unwrap();
    let clear_intent = fa.commit_intent().unwrap();
    sess.execute(&clear_intent.command_id, clear_intent.params).unwrap();
    assert!(sess.document().graph_sheet(&"g-ax".into()).unwrap().spec.y2_axis.is_none());
}

#[test]
fn fa_sec14_07_grid_cmd_deselect_single_cell_and_row_and_navigate_clears_stale_discontiguous() {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).unwrap();
    for r in 0..4 {
        for c in 0..3 {
            col.data_sets[c].cells[r][0] = Cell::number((r * 10 + c) as f64);
        }
    }
    let ds = DataSheet::new("ds-sel", "Sel", TableArchetype::Column(col)).unwrap();
    let mut doc = ProjectDocument::new("SEC14-07 Grid").unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    doc.active_sheet_id = Some("ds-sel".into());

    let mut grid = GridViewState::default();
    grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    grid.toggle_cmd_selection(&ds, GridCellCoord::new(2, 1, 0));
    assert!(grid.is_cell_selected(&ds, GridCellCoord::new(0, 0, 0)));
    assert!(grid.is_cell_selected(&ds, GridCellCoord::new(2, 1, 0)));
    assert!(!grid.is_cell_selected(&ds, GridCellCoord::new(1, 0, 0)));

    // Deselecting (2, 1, 0) collapses cleanly onto single remaining cell (0, 0, 0).
    grid.toggle_cmd_selection(&ds, GridCellCoord::new(2, 1, 0));
    assert!(grid.is_cell_selected(&ds, GridCellCoord::new(0, 0, 0)));
    assert!(!grid.is_cell_selected(&ds, GridCellCoord::new(2, 1, 0)));
    assert!(!grid.is_cell_selected(&ds, GridCellCoord::new(1, 0, 0)));
    assert!(grid.selected_coords.is_empty());

    // Keyboard navigation clears stale discontiguous coordinates and refreshes quick_stats.
    grid.toggle_cmd_selection(&ds, GridCellCoord::new(2, 1, 0));
    grid.refresh_selection_stats(&doc);
    assert_eq!(grid.quick_stats.map(|q| q.selected_cells), Some(2));
    grid.navigate(&ds, MoveDirection::Down, false, false);
    assert!(grid.selected_coords.is_empty());
    assert_eq!(grid.quick_stats.map(|q| q.selected_cells), Some(1));

    // Row Cmd+Click toggle off to 1 remaining row collapses cleanly without phantom bounding box.
    grid.select_row_click(&ds, 0, false, false);
    grid.select_row_click(&ds, 2, false, true);
    assert_eq!(grid.selected_rows, vec![0, 2]);
    grid.select_row_click(&ds, 2, false, true);
    assert_eq!(grid.selected_rows, vec![0]);
    assert!(grid.is_cell_selected(&ds, GridCellCoord::new(0, 1, 0)));
    assert!(!grid.is_cell_selected(&ds, GridCellCoord::new(2, 1, 0)));
}

#[test]
fn fa_sec14_08_command_palette_ui_metadata_enablement_schema_and_activation() {
    let empty_sess = Session::new();
    let palette_axes = CommandPaletteState { query: String::from("graph.setAxes"), ..CommandPaletteState::default() };
    let engine_hits = palette_axes.matches_for_session(&empty_sess);
    let eng = engine_hits.iter().find(|e| e.id == "graph.setAxes").unwrap();
    assert!(eng.params_doc.contains("sheet_id"), "Engine params_doc must reflect schema fields, got {}", eng.params_doc);

    let palette_ui = CommandPaletteState { query: String::from("ui.graph.axes.open"), ..CommandPaletteState::default() };
    let ui_hits = palette_ui.matches_for_session(&empty_sess);
    let ui_entry = ui_hits.iter().find(|e| e.id == "ui.graph.axes.open").unwrap();
    assert_eq!(ui_entry.menu_path, "Format/Format Axes...");
    assert!(!ui_entry.enabled, "ui.graph.axes.open must be disabled on empty Session");
    assert!(palette_ui.activate_selected_for_session(Some(&empty_sess)).is_none(), "disabled command must not activate");

    // Populated session enables UI command and auto-populates sheet_id on activation.
    let mut doc = ProjectDocument::new("SEC14-08 Palette").unwrap();
    let xy = XyTable::new(SubcolumnMode::Single, 1, 4).unwrap();
    let ds = DataSheet::new("d-pal", "Data", TableArchetype::Xy(xy)).unwrap();
    let gs = GraphSheet::new_uninitialized_for_data("g-pal", &ds).unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs).unwrap();
    doc.active_sheet_id = Some("g-pal".into());
    let pop_sess = Session::from_document(doc);

    let active_hits = palette_ui.matches_for_session(&pop_sess);
    assert!(active_hits.iter().find(|e| e.id == "ui.graph.axes.open").unwrap().enabled);
    let intent = palette_ui.activate_selected_for_session(Some(&pop_sess)).unwrap();
    assert_eq!(intent.command_id, "ui.graph.axes.open");
    assert_eq!(intent.params["sheet_id"], json!("g-pal"));
}

#[test]
fn fa_sec14_09_import_reshape_decimal_sep_hoisted_before_data() {
    let mut reshape = ImportReshapeState::default();
    let mut patch = BTreeMap::new();
    // Notice `"data"` precedes `"decimal_sep"` alphabetically in BTreeMap!
    patch.insert(String::from("data"), json!("1,5\t2,25\n3,75\t4,5"));
    patch.insert(String::from("decimal_sep"), json!(","));
    let json_map: Map<String, Value> = patch.into_iter().collect();
    reshape.apply_field_patch(&json_map).unwrap();
    assert_eq!(reshape.decimal_separator, ',');
    assert_eq!(reshape.plate_values, vec![Some(1.5), Some(2.25), Some(3.75), Some(4.5)]);

    let mut bad_sep = Map::new();
    bad_sep.insert(String::from("decimal_sep"), json!(";"));
    assert!(reshape.apply_field_patch(&bad_sep).is_err());
}

#[test]
fn fa_sec14_10_grid_large_sheet_empty_selection_trailing_empty_rows_and_sparse_tail() {
    // 1. 6,000-row table with sparse data ONLY at row 5,500 (> MAX_MATERIALIZED_SELECTION_COORDS = 4,096).
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 6_000).unwrap();
    col.data_sets[0].cells[5_500][0] = Cell::number(99.5);
    col.data_sets[1].cells[5_500][0] = Cell::number(42.5);
    let ds = DataSheet::new("ds-6k", "Large", TableArchetype::Column(col)).unwrap();
    let mut doc = ProjectDocument::new("SEC14-10 Grid").unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();
    doc.active_sheet_id = Some("ds-6k".into());

    // Small empty selection inside rows 5,000..=5,003 still returns 4 * 2 = 8 coords.
    let mut grid = GridViewState::default();
    grid.select_single_sync(GridCellCoord::new(5_000, 0, 0));
    grid.extend_shift_range(&ds, GridCellCoord::new(5_003, 1, 0));
    let small_empty = grid.all_selected_coords(&ds);
    assert_eq!(small_empty.len(), 8);

    // Full-sheet selection 0..=5,999 finds sparse row 5,500 past the first 4,096 rows!
    grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    grid.extend_shift_range(&ds, GridCellCoord::new(5_999, 1, 0));
    let sparse_coords = grid.all_selected_coords(&ds);
    assert!(sparse_coords.contains(&GridCellCoord::new(5_500, 0, 0)), "sparse row 5500 dropped");
    assert!(sparse_coords.contains(&GridCellCoord::new(5_500, 1, 0)));

    // 2. Small Shift+Click row selection keeps trailing empty rows.
    let mut col_small = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 6).unwrap();
    col_small.data_sets[0].cells[0][0] = Cell::number(1.0);
    col_small.data_sets[0].cells[1][0] = Cell::number(2.0);
    let ds_small = DataSheet::new("ds-sm", "Small", TableArchetype::Column(col_small)).unwrap();
    let mut grid_small = GridViewState::default();
    grid_small.select_row_click(&ds_small, 0, false, false);
    grid_small.select_row_click(&ds_small, 3, true, false);
    assert_eq!(grid_small.selected_rows, vec![0, 1, 2, 3]);
}

#[test]
fn fa_sec14_11_power_wizard_create_is_atomic_on_error_and_exceeding_sheet_limit() {
    // 1. Direct `PowerWizardState::execute_create`: fill up to `MAX_SHEETS_PER_PROJECT - 3` so `sheet.createData` (+3 = MAX) succeeds
    //    and `analysis.run` fails `TooManySheets` (+1 > MAX); entire creation must roll back cleanly!
    let mut doc = ProjectDocument::new("SEC14-11 Direct").unwrap();
    for idx in 0..(MAX_SHEETS_PER_PROJECT - 3) {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
        doc.add_data_sheet(DataSheet::new(format!("s-{idx}"), format!("S{idx}"), TableArchetype::Column(col)).unwrap()).unwrap();
    }
    assert_eq!(doc.total_sheet_count(), MAX_SHEETS_PER_PROJECT - 3);
    let mut sess = Session::from_document(doc.clone());
    let mut wiz = PowerWizardState::default();
    wiz.recalculate().unwrap();
    assert!(wiz.execute_create(&mut sess).is_err());
    assert!(wiz.created_sheets.is_none());
    assert_eq!(sess.document().total_sheet_count(), MAX_SHEETS_PER_PROJECT - 3);
    assert_eq!(sess.undo_depth(), 0);

    // 2. Shell `execute_create_with` path via GUI action `ui.answer_dialog` `"create"`: rolls back Data + Info + Graph too!
    let mut app = UiApp::from_session(Session::from_document(doc));
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.powerWizard.open" })).unwrap();
    app.canvas_dialogs.power_wizard.recalculate().unwrap();
    let res = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "create" }));
    assert!(res.is_err(), "expected TooManySheets failure on Power Wizard create");
    assert_eq!(app.document().total_sheet_count(), MAX_SHEETS_PER_PROJECT - 3, "partial Data+Info+Graph sheets leaked on failure");
    assert!(app.canvas_dialogs.power_wizard.created_sheets.is_none());
}

#[test]
fn fa_sec14_12_menus_tooltips_complete_coverage_data_bucket_and_gating() {
    let mut app = UiApp::new();
    for spec in UI_COMMANDS {
        let tip = command_tooltip(&app, spec.id).unwrap_or_else(|| panic!("missing tooltip for {}", spec.id));
        assert_ne!(tip.menu_path, "UI", "UI command `{}` fell back to generic `UI` menu_path", spec.id);
    }
    for data_cmd in [
        "ui.clonePipeline.open",
        "ui.table.convertArchetype.open",
        "ui.table.randomizeGroups",
        "ui.table.randomSampleRows",
        "ui.table.openFormulaBar",
    ] {
        let tip = command_tooltip(&app, data_cmd).unwrap();
        assert!(tip.menu_path.starts_with("Data/"), "`{data_cmd}` expected `Data/...`, got `{}`", tip.menu_path);
    }
    // Empty project gates brush, fitSheet, rowTitles.
    for gated in ["ui.brush.point", "ui.zoom.fitSheet", "ui.view.rowTitles.toggle"] {
        assert!(!command_tooltip(&app, gated).unwrap().enabled, "`{gated}` must be disabled when project has no data sheets");
    }
    app.execute_command("sheet.createData", json!({ "sheet_id": "d-1", "kind": "column" })).unwrap();
    for gated in ["ui.brush.point", "ui.zoom.fitSheet", "ui.view.rowTitles.toggle"] {
        assert!(command_tooltip(&app, gated).unwrap().enabled, "`{gated}` must enable once data sheet exists");
    }
}

#[test]
fn fa_sec14_13_insert_model_equation_updates_only_matching_annotation() {
    let mut doc = ProjectDocument::new("SEC14-13 Eq").unwrap();
    let xy = XyTable::new(SubcolumnMode::Single, 2, 4).unwrap();
    let ds = DataSheet::new("d-eq", "Dose", TableArchetype::Xy(xy)).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-eq", &ds).unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-1", "d-eq", AnalysisKind::LinearRegression(Default::default())).unwrap()).unwrap())
        .unwrap();
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-2", "d-eq", AnalysisKind::LinearRegression(Default::default())).unwrap()).unwrap())
        .unwrap();

    gs.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: "res-1".into(),
        data_set_index: 0,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: RectPt::new(10.0, 10.0, 120.0, 24.0),
        font_size_pt: 9.0,
        bordered: false,
    });
    gs.spec.annotations.push(GraphAnnotation::ModelEquation {
        result_sheet_id: "res-2".into(),
        data_set_index: 1,
        mode: EquationDisplayMode::Symbolic,
        bounds_pt: RectPt::new(10.0, 40.0, 120.0, 24.0),
        font_size_pt: 9.0,
        bordered: false,
    });
    doc.add_graph_sheet(gs.clone()).unwrap();

    let mut dlg = InsertModelEquationDialogState::default();
    dlg.open_for_graph(&gs, &doc);
    dlg.result_sheet_id = Some("res-2".into());
    dlg.data_set_index = 1;
    dlg.mode = EquationDisplayMode::Substituted;
    dlg.bordered = true;
    dlg.update_existing = true;

    let intent = dlg.commit_intent(Some(&doc)).unwrap();
    assert_eq!(intent.command_id, "graph.setSpec");
    let mut sess = Session::from_document(doc);
    sess.execute(&intent.command_id, intent.params).unwrap();

    let anns = &sess.document().graph_sheet(&"g-eq".into()).unwrap().spec.annotations;
    assert_eq!(anns.len(), 2);
    match &anns[0] {
        GraphAnnotation::ModelEquation { result_sheet_id, mode, bordered, .. } => {
            assert_eq!(result_sheet_id.as_str(), "res-1", "first ModelEquation must remain untouched");
            assert_eq!(*mode, EquationDisplayMode::Symbolic);
            assert!(!bordered);
        }
        other => panic!("unexpected ann[0]: {other:?}"),
    }
    match &anns[1] {
        GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bordered, .. } => {
            assert_eq!(result_sheet_id.as_str(), "res-2");
            assert_eq!(*data_set_index, 1);
            assert_eq!(*mode, EquationDisplayMode::Substituted);
            assert!(*bordered);
        }
        other => panic!("unexpected ann[1]: {other:?}"),
    }
}
