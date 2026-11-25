//! Regression tests covering all 12 confirmed `SEC-14` findings (`FA-SEC14-01..05`, `FA-SEC14-07..13`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
use curvebook_ui_egui::dialogs_canvas::dialog_patch::{patch_analyze, patch_edit_params, patch_format_axes, patch_import_reshape};
use curvebook_ui_egui::dialogs_canvas::{
    AnalyzeDialogState, AxisTarget, BubblePlotInspectorState, CommandPaletteState, EditAnalysisParametersState, FormatAxesState,
    PairwiseComparisonsState,
};
use curvebook_ui_egui::dialogs_r9::InsertModelEquationDialogState;
use curvebook_ui_egui::grid_view::{GridViewState, ImportReshapeState, MoveDirection};
use curvebook_ui_egui::i18n::LocalePreset;
use curvebook_ui_egui::menus::{UI_COMMANDS, command_tooltip};
use curvebook_ui_egui::power_wizard::PowerWizardState;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

use crate::common::{result_scalar, seed_column_doc};
use crate::helpers::patch_map;

fn pw_rec(a: usize, b: usize, diff: f64, p: f64, sig: bool) -> PairwiseComparisonRecord {
    PairwiseComparisonRecord {
        group_a: a,
        group_b: b,
        label_a: format!("G{a}"),
        label_b: format!("G{b}"),
        estimate_diff: Some(diff),
        raw_p_or_q: p,
        is_fdr_q: false,
        significant: sig,
    }
}

fn cell_f64(cell: &ResultCell) -> f64 {
    match *cell {
        ResultCell::Number { value, .. } => value,
        ref other => panic!("expected ResultCell::Number, got {other:?}"),
    }
}

#[test]
fn fa_sec14_01_pairwise_clear_all_multi_bracket_and_style_edits_persist() {
    let (mut doc, _) = seed_column_doc(
        "SEC14-01 Pairwise",
        "ds-pw",
        "Groups",
        ColumnDesignHint::UnpairedReplicates,
        &[("G0", &[10.0, 11.0, 12.0, 13.0]), ("G1", &[20.0, 21.0, 22.0, 23.0]), ("G2", &[35.0, 36.0, 37.0, 38.0])],
    )
    .unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-pw", doc.data_sheet(&"ds-pw".into()).unwrap()).unwrap();
    doc.add_graph_sheet(gs.clone()).unwrap();

    let mut res_spec = AnalysisSpec::new("res-pw", "ds-pw", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
    res_spec.selected_data_sets = vec![0, 1, 2];
    let mut rs = ResultSheet::new(res_spec).unwrap();
    rs.pairwise_comparisons = vec![pw_rec(0, 1, -10.0, 0.01, true), pw_rec(0, 2, -25.0, 0.001, true), pw_rec(1, 2, -15.0, 0.25, false)];
    doc.add_result_sheet(rs).unwrap();

    // 1. Fresh single-bracket auto-add uses `graph.addComparisonBrackets`.
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
    assert_eq!(pw_single.commit_intents()[0].command_id, "graph.addComparisonBrackets");

    // 2. Multi-bracket + style/label/hide edit emits `graph.setSpec` retaining all brackets.
    let mut sess = Session::from_document(doc);
    for group_b in [1, 2] {
        sess.execute("graph.addComparisonBrackets", json!({ "sheet_id": "g-pw", "result_sheet_id": "res-pw", "group_a": 0, "group_b": group_b }))
            .unwrap();
    }
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
    assert!(
        updated_gs
            .spec
            .comparisons
            .iter()
            .all(|b| { b.style == BracketStyle::CurlyBrace && b.label_mode == BracketLabelMode::NumericPOrQ && b.hide_if_non_significant })
    );

    // 3. Clearing ALL brackets emits `graph.setSpec` with empty `comparisons`.
    pw.sync_from_graph(updated_gs, sess.document());
    pw.brackets.clear();
    pw.brackets_dirty = true;
    let clear = pw.commit_intents();
    assert_eq!((clear.len(), clear[0].command_id.as_str()), (1, "graph.setSpec"));
    sess.execute(&clear[0].command_id, clear[0].params.clone()).unwrap();
    assert!(sess.document().graph_sheet(&"g-pw".into()).unwrap().spec.comparisons.is_empty());
}

#[test]
fn fa_sec14_02_bubble_plot_preserves_x_level_order_override() {
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    mv.variables[0].role = VariableRole::Categorical;
    for (r, lvl) in ["High", "Mid", "Low", "High"].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(lvl);
        mv.variables[1].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[2].cells[r] = Cell::number((r + 2) as f64);
    }
    let ds = DataSheet::new("ds-mv", "MV", TableArchetype::MultipleVariables(mv)).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-mv", &ds).unwrap();
    let override_order = LevelOrderSpec { order: LevelOrderKind::Explicit(vec!["High".into(), "Mid".into(), "Low".into()]), reversed: true };
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
    assert!(saved.show_convex_hulls && saved.x_level_order_override == Some(override_order));
}

#[test]
fn fa_sec14_03_pcr_end_to_end_distinct_from_pca_with_leakage_guard_and_methods_report() {
    let mut mv = MultipleVariablesTable::new(5, 8).unwrap();
    for (c, t) in ["X1", "X2", "X3", "ResponseY", "GroupCat"].into_iter().enumerate() {
        mv.variables[c].title = t.into();
    }
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

    // 1. PCA vs PCR titles, R^2, back-transformed coefficients, and direct PCA eigen oracle.
    let run_pca = |id: &str, outcome: Option<usize>| {
        let mut spec = AnalysisSpec::new(
            id,
            "mv-pcr",
            AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: None, pcr_outcome_column: outcome }),
        )
        .unwrap();
        spec.selected_data_sets = vec![0, 1, 2];
        let title = spec.effective_title().to_owned();
        (title, run_analysis(&ds, &spec).unwrap())
    };
    let (pca_title, pca_res) = run_pca("res-pca", None);
    assert_eq!(pca_title, "Principal component analysis");
    assert!(result_scalar(&pca_res, "PCR R squared").is_err());

    let (pcr_title, pcr_res) = run_pca("res-pcr", Some(3));
    assert_eq!(pcr_title, "Principal component regression");
    let r2 = result_scalar(&pcr_res, "PCR R squared").unwrap();
    assert!(r2 > 0.99 && r2 <= 1.0 + 1e-12, "unexpected PCR R^2: {r2}");

    let pred_tab = pcr_res.tabs.iter().find(|t| t.id == "pcr-predictors").unwrap();
    let diag_tab = pcr_res.tabs.iter().find(|t| t.id == "pcr-diagnostics").unwrap();
    let coefs: [f64; 4] = std::array::from_fn(|i| cell_f64(&pred_tab.rows[i][0]));
    for (r, row) in data.iter().enumerate() {
        let yhat = coefs[0] + coefs[1] * row[0] + coefs[2] * row[1] + coefs[3] * row[2];
        assert!((yhat - cell_f64(&diag_tab.rows[r][1])).abs() < 1e-9);
    }
    let flat_x: Vec<f64> = data.iter().flat_map(|row| row[..3].iter().copied()).collect();
    let direct_pca = pca_analysis(&DenseMatrix::from_row_major(8, 3, flat_x).unwrap(), &["X1".into(), "X2".into(), "X3".into()], true, None).unwrap();
    assert!((result_scalar(&pcr_res, "PC1 eigenvalue").unwrap() - direct_pca.eigenvalues[0]).abs() < 1e-10);

    // 2. AnalyzeDialogState `toggle_variable`, patch hoisting, leakage guard, and non-PCR pass-through.
    let preset = LocalePreset::default();
    let mut az = AnalyzeDialogState::default();
    az.open_for_data_sheet(&ds);
    az.select_catalog_entry("stats.pcr");
    az.toggle_variable(0);
    az.toggle_variable(1);
    assert_eq!((&az.pca_predictors, &az.selected_data_sets), (&vec![0, 1], &vec![0, 1]));
    az.select_pcr_outcome(3).unwrap();
    az.toggle_variable(3);
    assert_eq!((&az.pca_predictors, az.pcr_outcome_column), (&vec![0, 1, 3], None));
    az.toggle_variable(3);

    for bad in [json!({ "pcr_outcome_column": 1, "selected_data_sets": [0, 1, 2] }), json!({ "pcr_outcome_column": "invalid" })] {
        assert!(patch_analyze(&mut az, &patch_map(bad), &preset).is_err());
    }
    let mut reg_az = AnalyzeDialogState::default();
    reg_az.open_for_data_sheet(&ds);
    patch_analyze(
        &mut reg_az,
        &patch_map(json!({ "entry_id": "stats.multiple_regression", "selected_data_sets": [0, 1, 2], "response_column": 2 })),
        &preset,
    )
    .unwrap();
    assert_eq!(reg_az.pcr_outcome_column, None);

    patch_analyze(&mut az, &patch_map(json!({ "selected_data_sets": [0, 1, 2], "pcr_outcome_column": 3 })), &preset).unwrap();
    let built_kind = az.build_analysis_kind().unwrap();
    assert!(matches!(built_kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column: Some(3), .. })));

    // 3. Engine run, methods report, EditAnalysisParametersState reopen/clear roundtrip.
    let mut doc = ProjectDocument::new("SEC14-03 Engine").unwrap();
    doc.add_data_sheet(ds).unwrap();
    let mut sess = Session::from_document(doc);
    let run_out = sess.execute("analysis.run", json!({ "source_sheet_id": "mv-pcr", "sheet_id": "res-pcr-1", "kind": built_kind })).unwrap();
    assert_eq!(run_out.payload["sheet_id"], json!("res-pcr-1"));

    let rep = sess.execute("result.generateReport", json!({ "result_sheet_id": "res-pcr-1", "format": "plain" })).unwrap();
    let rep_text = rep.payload["report"].as_str().unwrap();
    assert!(rep_text.contains("Principal component regression") && rep_text.contains("ResponseY"));

    let rs_saved = sess.document().result_sheet(&"res-pcr-1".into()).unwrap().clone();
    let mut ep = EditAnalysisParametersState::default();
    ep.open_for_result(&rs_saved);
    assert_eq!(ep.pcr_outcome_column, Some(3));
    patch_edit_params(&mut ep, &patch_map(json!({ "pcr_outcome_column": Value::Null })), &preset).unwrap();
    assert_eq!(ep.pcr_outcome_column, None);
    let upd = ep.commit_intent().unwrap();
    assert_eq!(upd.params["pcr_outcome_column"], Value::Null);
    sess.execute(&upd.command_id, upd.params).unwrap();
    let updated_rs = sess.document().result_sheet(&"res-pcr-1".into()).unwrap();
    assert!(matches!(updated_rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column: None, .. })));
    assert!(result_scalar(updated_rs, "PCR R squared").is_err());
}

#[test]
fn fa_sec14_04_edit_params_contingency_reopen_sync_and_paired_graph_differences() {
    let preset = LocalePreset::default();
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    for (r, c, v) in [(0, 0, 20.0), (0, 1, 5.0), (1, 0, 10.0), (1, 1, 25.0)] {
        ct.data_sets[c].cells[r][0] = Cell::number(v);
    }
    let ds_ct = DataSheet::new("ct-1", "2x2", TableArchetype::Contingency(ct)).unwrap();
    let rs_ct = ResultSheet::new(
        AnalysisSpec::new("res-ct", "ct-1", AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() }))
            .unwrap(),
    )
    .unwrap();

    let mut ep = EditAnalysisParametersState::default();
    ep.open_for_result(&rs_ct);
    assert!(ep.contingency_odds_ratio_checked && ep.contingency_relative_risk_checked && ep.contingency_attributable_risk_checked);
    assert!(patch_edit_params(&mut ep, &patch_map(json!({ "edit.contingency.ci.baptista_pike": "BogusCI" })), &preset).is_err());

    patch_edit_params(&mut ep, &patch_map(json!({ "edit.contingency.odds_ratio": false })), &preset).unwrap();
    assert!(!ep.contingency_attributable_risk_checked);
    patch_edit_params(&mut ep, &patch_map(json!({ "edit.contingency.attributable_risk": true })), &preset).unwrap();
    assert!(matches!(ep.draft.as_ref().unwrap().kind, AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, .. })));

    let (mut doc, _) = seed_column_doc(
        "SEC14-04 TwoGroup",
        "ds-tg",
        "Paired",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[("A", &[10.0, 11.0, 12.0, 13.0]), ("B", &[14.0, 16.5, 19.0, 21.5])],
    )
    .unwrap();
    doc.add_data_sheet(ds_ct).unwrap();
    let make_tg = |id: &str, method: TwoGroupMethod| {
        ResultSheet::new(AnalysisSpec::new(id, "ds-tg", AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() })).unwrap())
            .unwrap()
    };
    ep.open_for_result(&make_tg("res-mw", TwoGroupMethod::MannWhitneyU));
    assert!(ep.hodges_lehmann_ci);

    let rs_paired = make_tg("res-p", TwoGroupMethod::PairedT);
    doc.add_result_sheet(rs_paired.clone()).unwrap();
    ep.open_for_result(&rs_paired);
    patch_edit_params(&mut ep, &patch_map(json!({ "edit.two_group.graph_differences": true })), &preset).unwrap();
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

    patch_edit_params(&mut ep, &patch_map(json!({ "two_group_method": "unpaired_t" })), &preset).unwrap();
    assert!(!ep.graph_differences);
}

#[test]
fn fa_sec14_05_format_axes_preserves_multi_axis_edits_and_y2_across_tab_switches() {
    let preset = LocalePreset::default();
    let ds = DataSheet::new("d-ax", "Assay", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 4).unwrap())).unwrap();
    let gs = GraphSheet::new_uninitialized_for_data("g-ax", &ds).unwrap();
    let mut doc = ProjectDocument::new("SEC14-05 Axes").unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs.clone()).unwrap();

    let mut fa = FormatAxesState::default();
    fa.sync_from_graph(&gs, AxisTarget::X);
    for p in [
        json!({ "axes.title": "Dose (nM)", "axes.scale": "log10", "axes.min": 1.0, "axes.max": 1000.0 }),
        json!({ "axes.target_axis": "Y", "axes.title": "Response (%)", "axes.major_tick_interval": 25.0, "axes.enable_y2": true }),
        json!({ "axes.target_axis": "Y2", "axes.title": "Normalized Y2" }),
    ] {
        patch_format_axes(&mut fa, &patch_map(p), Some(&doc), &preset).unwrap();
    }
    assert!(fa.add_threshold(42.0, None, Some("EC50".into()), curvebook_palette::Rgba8::rgba(200, 0, 0, 255)));
    patch_format_axes(&mut fa, &patch_map(json!({ "axes.target_axis": "X" })), Some(&doc), &preset).unwrap();
    assert_eq!((fa.axis_draft.title.as_str(), fa.axis_draft.segments[0].scale, fa.enable_y2), ("Dose (nM)", GraphAxisScale::Log10, true));

    let intent = fa.commit_intent().unwrap();
    let mut sess = Session::from_document(doc);
    sess.execute(&intent.command_id, intent.params).unwrap();
    let saved = sess.document().graph_sheet(&"g-ax".into()).unwrap();
    assert_eq!((saved.spec.x_axis.title.as_str(), saved.spec.x_axis.segments[0].scale), ("Dose (nM)", GraphAxisScale::Log10));
    assert_eq!((saved.spec.y_axis.title.as_str(), saved.spec.y_axis.segments[0].major_tick_interval), ("Response (%)", Some(25.0)));
    assert_eq!(saved.spec.frame_preset, FramePreset::DualYSecondaryRight);
    let y2 = saved.spec.y2_axis.as_ref().unwrap();
    assert_eq!((y2.title.as_str(), y2.thresholds.len()), ("Normalized Y2", 1));

    fa.sync_from_graph(saved, AxisTarget::X);
    patch_format_axes(&mut fa, &patch_map(json!({ "axes.enable_y2": false })), Some(sess.document()), &preset).unwrap();
    let clear = fa.commit_intent().unwrap();
    sess.execute(&clear.command_id, clear.params).unwrap();
    assert!(sess.document().graph_sheet(&"g-ax".into()).unwrap().spec.y2_axis.is_none());
}

#[test]
fn fa_sec14_07_grid_cmd_deselect_single_cell_and_row_and_navigate_clears_stale_discontiguous() {
    let (doc, sid) = seed_column_doc(
        "SEC14-07 Grid",
        "ds-sel",
        "Sel",
        ColumnDesignHint::UnpairedReplicates,
        &[("A", &[0.0, 10.0, 20.0, 30.0]), ("B", &[1.0, 11.0, 21.0, 31.0]), ("C", &[2.0, 12.0, 22.0, 32.0])],
    )
    .unwrap();
    let ds = doc.data_sheet(&sid).unwrap();
    let (c00, c10, c21) = (GridCellCoord::new(0, 0, 0), GridCellCoord::new(1, 0, 0), GridCellCoord::new(2, 1, 0));

    let mut grid = GridViewState::default();
    grid.select_single_sync(c00);
    grid.toggle_cmd_selection(ds, c21);
    assert!(grid.is_cell_selected(ds, c00) && grid.is_cell_selected(ds, c21) && !grid.is_cell_selected(ds, c10));

    grid.toggle_cmd_selection(ds, c21);
    assert!(grid.is_cell_selected(ds, c00) && !grid.is_cell_selected(ds, c21) && !grid.is_cell_selected(ds, c10) && grid.selected_coords.is_empty());

    grid.toggle_cmd_selection(ds, c21);
    grid.refresh_selection_stats(&doc);
    assert_eq!(grid.quick_stats.map(|q| q.selected_cells), Some(2));
    grid.navigate(ds, MoveDirection::Down, false, false);
    assert!(grid.selected_coords.is_empty() && grid.quick_stats.map(|q| q.selected_cells) == Some(1));

    grid.select_row_click(ds, 0, false, false);
    grid.select_row_click(ds, 2, false, true);
    assert_eq!(grid.selected_rows, vec![0, 2]);
    grid.select_row_click(ds, 2, false, true);
    assert_eq!(grid.selected_rows, vec![0]);
    assert!(grid.is_cell_selected(ds, GridCellCoord::new(0, 1, 0)) && !grid.is_cell_selected(ds, c21));
}

#[test]
fn fa_sec14_08_command_palette_ui_metadata_enablement_schema_and_activation() {
    let empty_sess = Session::new();
    let eng_hits = CommandPaletteState { query: "graph.setAxes".into(), ..Default::default() }.matches_for_session(&empty_sess);
    assert!(eng_hits.iter().find(|e| e.id == "graph.setAxes").unwrap().params_doc.contains("sheet_id"));

    let pal_ui = CommandPaletteState { query: "ui.graph.axes.open".into(), ..Default::default() };
    let ui_entry = pal_ui.matches_for_session(&empty_sess).into_iter().find(|e| e.id == "ui.graph.axes.open").unwrap();
    assert_eq!((ui_entry.menu_path, ui_entry.enabled), ("Format/Format Axes...", false));
    assert!(pal_ui.activate_selected_for_session(Some(&empty_sess)).is_none());

    let mut doc = ProjectDocument::new("SEC14-08 Palette").unwrap();
    let ds = DataSheet::new("d-pal", "Data", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 4).unwrap())).unwrap();
    let gs = GraphSheet::new_uninitialized_for_data("g-pal", &ds).unwrap();
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs).unwrap();
    doc.active_sheet_id = Some("g-pal".into());
    let pop_sess = Session::from_document(doc);

    assert!(pal_ui.matches_for_session(&pop_sess).iter().find(|e| e.id == "ui.graph.axes.open").unwrap().enabled);
    let intent = pal_ui.activate_selected_for_session(Some(&pop_sess)).unwrap();
    assert_eq!((intent.command_id.as_str(), &intent.params["sheet_id"]), ("ui.graph.axes.open", &json!("g-pal")));
}

#[test]
fn fa_sec14_09_import_reshape_decimal_sep_hoisted_before_data() {
    let preset = LocalePreset::default();
    let mut reshape = ImportReshapeState::default();
    // `"data"` precedes `"decimal_sep"` alphabetically in serde_json::Map (BTreeMap).
    patch_import_reshape(&mut reshape, &patch_map(json!({ "data": "1,5\t2,25\n3,75\t4,5", "decimal_sep": "," })), &preset).unwrap();
    assert_eq!((reshape.decimal_separator, &reshape.plate_values), (',', &vec![Some(1.5), Some(2.25), Some(3.75), Some(4.5)]));
    assert!(patch_import_reshape(&mut reshape, &patch_map(json!({ "decimal_sep": ";" })), &preset).is_err());
}

#[test]
fn fa_sec14_10_grid_large_sheet_empty_selection_trailing_empty_rows_and_sparse_tail() {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 6_000).unwrap();
    col.data_sets[0].cells[5_500][0] = Cell::number(99.5);
    col.data_sets[1].cells[5_500][0] = Cell::number(42.5);
    let ds = DataSheet::new("ds-6k", "Large", TableArchetype::Column(col)).unwrap();

    let mut grid = GridViewState::default();
    grid.select_single_sync(GridCellCoord::new(5_000, 0, 0));
    grid.extend_shift_range(&ds, GridCellCoord::new(5_003, 1, 0));
    assert_eq!(grid.all_selected_coords(&ds).len(), 8);

    grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    grid.extend_shift_range(&ds, GridCellCoord::new(5_999, 1, 0));
    let sparse = grid.all_selected_coords(&ds);
    assert!(sparse.contains(&GridCellCoord::new(5_500, 0, 0)) && sparse.contains(&GridCellCoord::new(5_500, 1, 0)));

    let mut col_sm = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 6).unwrap();
    col_sm.data_sets[0].cells[0][0] = Cell::number(1.0);
    col_sm.data_sets[0].cells[1][0] = Cell::number(2.0);
    let ds_sm = DataSheet::new("ds-sm", "Small", TableArchetype::Column(col_sm)).unwrap();
    let mut g_sm = GridViewState::default();
    g_sm.select_row_click(&ds_sm, 0, false, false);
    g_sm.select_row_click(&ds_sm, 3, true, false);
    assert_eq!(g_sm.selected_rows, vec![0, 1, 2, 3]);
}

#[test]
fn fa_sec14_11_power_wizard_create_is_atomic_on_error_and_exceeding_sheet_limit() {
    let mut doc = ProjectDocument::new("SEC14-11 Direct").unwrap();
    for idx in 0..(MAX_SHEETS_PER_PROJECT - 3) {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
        doc.add_data_sheet(DataSheet::new(format!("s-{idx}"), format!("S{idx}"), TableArchetype::Column(col)).unwrap()).unwrap();
    }
    let mut sess = Session::from_document(doc.clone());
    let mut wiz = PowerWizardState::default();
    wiz.recalculate().unwrap();
    assert!(wiz.execute_create(&mut sess).is_err());
    assert!(wiz.created_sheets.is_none() && sess.document().total_sheet_count() == MAX_SHEETS_PER_PROJECT - 3 && sess.undo_depth() == 0);

    let mut app = UiApp::from_session(Session::from_document(doc));
    dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.powerWizard.open" })).unwrap();
    app.canvas_dialogs.power_wizard.recalculate().unwrap();
    assert!(dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "create" })).is_err());
    assert_eq!(app.document().total_sheet_count(), MAX_SHEETS_PER_PROJECT - 3);
    assert!(app.canvas_dialogs.power_wizard.created_sheets.is_none());
}

#[test]
fn fa_sec14_12_menus_tooltips_complete_coverage_data_bucket_and_gating() {
    let mut app = UiApp::new();
    for spec in UI_COMMANDS {
        let tip = command_tooltip(&app, spec.id).unwrap_or_else(|| panic!("missing tooltip for {}", spec.id));
        assert_ne!(tip.menu_path, "UI", "`{}` fell back to `UI`", spec.id);
    }
    for cmd in [
        "ui.clonePipeline.open",
        "ui.table.convertArchetype.open",
        "ui.table.randomizeGroups",
        "ui.table.randomSampleRows",
        "ui.table.openFormulaBar",
    ] {
        assert!(command_tooltip(&app, cmd).unwrap().menu_path.starts_with("Data/"), "`{cmd}`");
    }
    let gated = ["ui.brush.point", "ui.zoom.fitSheet", "ui.view.rowTitles.toggle"];
    for cmd in gated {
        assert!(!command_tooltip(&app, cmd).unwrap().enabled, "`{cmd}` disabled on empty project");
    }
    app.execute_command("sheet.createData", json!({ "sheet_id": "d-1", "kind": "column" })).unwrap();
    for cmd in gated {
        assert!(command_tooltip(&app, cmd).unwrap().enabled, "`{cmd}` enabled with data sheet");
    }
}

#[test]
fn fa_sec14_13_insert_model_equation_updates_only_matching_annotation() {
    let mut doc = ProjectDocument::new("SEC14-13 Eq").unwrap();
    let ds = DataSheet::new("d-eq", "Dose", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 4).unwrap())).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data("g-eq", &ds).unwrap();
    doc.add_data_sheet(ds).unwrap();
    for (idx, rid) in ["res-1", "res-2"].into_iter().enumerate() {
        doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new(rid, "d-eq", AnalysisKind::LinearRegression(Default::default())).unwrap()).unwrap())
            .unwrap();
        gs.spec.annotations.push(GraphAnnotation::ModelEquation {
            result_sheet_id: rid.into(),
            data_set_index: idx,
            mode: EquationDisplayMode::Symbolic,
            bounds_pt: RectPt::new(10.0, 10.0 + idx as f64 * 30.0, 120.0, 24.0),
            font_size_pt: 9.0,
            bordered: false,
        });
    }
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
    assert!(
        matches!(&anns[0], GraphAnnotation::ModelEquation { result_sheet_id, mode: EquationDisplayMode::Symbolic, bordered: false, .. } if result_sheet_id.as_str() == "res-1")
    );
    assert!(
        matches!(&anns[1], GraphAnnotation::ModelEquation { result_sheet_id, data_set_index: 1, mode: EquationDisplayMode::Substituted, bordered: true, .. } if result_sheet_id.as_str() == "res-2")
    );
}
