//! Wave 3 (`L5` Engine Wiring & Parity) integration tests covering `sheet.reorder`,
//! `graph.setAdditionalTicks` -> `ChartScene`, `#114` biostatistics/imputation/fitting parity,
//! `curvebook_format::sanitize_circular_hooks` integration, and `CrashRecoveryJournal` WAL replay.

use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive};
use curvebook_doc::{
    AdditionalTick, AnalysisKind, AnalysisSpec, Cell, ConstantValue, ContingencyMethod, DataSheet, GraphSheet, HookSource, InfoSheet, LayoutSheet,
    LayoutSpec, ModelComparisonSpec, MultiplicityMethod, MultivariateSpec, NonlinearFitSpec, NormalizeAnchor, ParameterRule, ParameterScope,
    ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTabKind, SheetFolderKind, SheetId, SurvivalAnalysisSpec, TransformAnalysisSpec,
    VariableRole,
};
use curvebook_engine::{Session, cmd, cmd::helpers::compile_sheet_scene, dag};
use curvebook_format::{JOURNAL_FILE_NAME, MemoryJournalBackend};
use curvebook_palette::LineDashStyle;
use serde_json::json;

use super::common::{
    TempDir, cell_num, col_sheet, contingency_sheet, err_cmd, grouped_rep_sheet, gs, mv_sheet, ok_cmd, rs, run_cmds, session_with, xy_rep_sheet,
    xy_sheet,
};

fn plain_scene_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

fn count_dotted_gridlines(scene: &ChartScene) -> usize {
    scene
        .primitives
        .iter()
        .filter(|p| matches!(p, ScenePrimitive::Path { path, stroke: Some(st), .. } if st.dash == LineDashStyle::Dotted && path.commands.len() == 2))
        .count()
}

fn three_group_col(id: &str) -> DataSheet {
    col_sheet(
        id,
        "ANOVA Groups",
        &[("Vehicle", &[10.2, 10.9, 9.8, 10.5, 10.1]), ("Low", &[14.5, 15.1, 13.9, 14.8, 15.2]), ("High", &[22.4, 23.0, 21.9, 22.8, 23.1])],
    )
}

fn clinical_mv(id: &str, with_missing: bool) -> DataSheet {
    let rows = [
        (12.0, 35.0, 1.0, "Ctrl"),
        (14.5, 42.0, 0.0, "Ctrl"),
        (13.2, 38.0, 1.0, "Ctrl"),
        (15.1, 45.0, 0.0, "Ctrl"),
        (11.8, 33.0, 1.0, "Ctrl"),
        (22.4, 52.0, 1.0, "Treat"),
        (25.0, 58.0, 1.0, "Treat"),
        (24.1, 55.0, 0.0, "Treat"),
        (26.8, 61.0, 1.0, "Treat"),
        (23.5, 50.0, 0.0, "Treat"),
    ];
    let rts: Vec<String> = (1..=10).map(|i| format!("P{i}")).collect();
    let rt_refs: Vec<&str> = rts.iter().map(String::as_str).collect();
    let resp: Vec<Cell> = rows.iter().enumerate().map(|(r, &(v, ..))| if with_missing && r == 2 { Cell::empty() } else { Cell::number(v) }).collect();
    let age: Vec<Cell> =
        rows.iter().enumerate().map(|(r, &(_, v, ..))| if with_missing && r == 7 { Cell::empty() } else { Cell::number(v) }).collect();
    let status: Vec<Cell> = rows.iter().map(|&(_, _, s, _)| Cell::number(s)).collect();
    let grp: Vec<Cell> = rows.iter().map(|&(_, _, _, g)| Cell::text(g)).collect();
    mv_sheet(
        id,
        "Clinical Cohort",
        &rt_refs,
        &[("Response", None, &resp), ("Age", None, &age), ("Status", None, &status), ("Cohort", Some(VariableRole::Categorical), &grp)],
    )
}

fn dual_dose_xy(id: &str) -> DataSheet {
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5];
    let r0: Vec<[f64; 2]> = xs
        .iter()
        .map(|&x| {
            let y = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.2));
            [y - 0.5, y + 0.5]
        })
        .collect();
    let r1: Vec<[f64; 2]> = xs
        .iter()
        .map(|&x| {
            let y = 20.0 + 80.0 / (1.0 + 10.0_f64.powf((-7.0 - x) * 1.1));
            [y - 0.6, y + 0.6]
        })
        .collect();
    let s0: Vec<&[f64]> = r0.iter().map(|a| a.as_slice()).collect();
    let s1: Vec<&[f64]> = r1.iter().map(|a| a.as_slice()).collect();
    xy_rep_sheet(id, "Dual Dose Response", "Log10 [Agonist] (M)", &xs, &[("10.0", &s0), ("20.0", &s1)])
}

#[test]
fn sheet_reorder_all_folders_and_undo_redo() {
    let mut doc = ProjectDocument::new("Reorder Parity").unwrap();
    let (ds1, ds2, ds3) = (three_group_col("d1"), three_group_col("d2"), three_group_col("d3"));
    doc.add_data_sheet(ds1.clone()).unwrap();
    doc.add_data_sheet(ds2.clone()).unwrap();
    doc.add_data_sheet(ds3).unwrap();
    for (id, title) in [("i1", "Info 1"), ("i2", "Info 2")] {
        doc.add_info_sheet(InfoSheet::new_preseeded(id, title).unwrap()).unwrap();
    }
    for (rid, did) in [("r1", "d1"), ("r2", "d2")] {
        doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new(rid, did, AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap())
            .unwrap();
    }
    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("g1", &ds1).unwrap()).unwrap();
    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("g2", &ds2).unwrap()).unwrap();
    for (id, title) in [("l1", "L1"), ("l2", "L2")] {
        doc.add_layout_sheet(LayoutSheet::new(id, title, LayoutSpec::default()).unwrap()).unwrap();
    }

    let mut session = Session::from_document(doc);
    fn data_ids(s: &Session) -> Vec<&str> {
        s.document().data_sheets.iter().map(|d| d.id.as_str()).collect()
    }

    ok_cmd(&mut session, "sheet.reorder", json!({ "folder": "data", "from_index": 0, "to_index": 2 }));
    assert_eq!(data_ids(&session), vec!["d2", "d3", "d1"]);
    ok_cmd(&mut session, "edit.undo", json!({}));
    assert_eq!(data_ids(&session), vec!["d1", "d2", "d3"]);
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(data_ids(&session), vec!["d2", "d3", "d1"]);

    run_cmds(
        &mut session,
        &[
            ("sheet.reorder", json!({ "folder": SheetFolderKind::Info, "from_index": 1, "to_index": 0 })),
            ("sheet.reorder", json!({ "folder": "results", "from_index": 0, "to_index": 1 })),
            ("sheet.reorder", json!({ "folder": "graphs", "from_index": 1, "to_index": 0 })),
            ("sheet.reorder", json!({ "folder": "layouts", "from_index": 0, "to_index": 1 })),
        ],
    );
    assert_eq!(session.document().info_sheets[0].id.as_str(), "i2");
    assert_eq!(session.document().result_sheets[0].id.as_str(), "r2");
    assert_eq!(session.document().graph_sheets[0].id.as_str(), "g2");
    assert_eq!(session.document().layout_sheets[0].id.as_str(), "l2");
    let _ = err_cmd(&mut session, "sheet.reorder", json!({ "folder": "data", "from_index": 9, "to_index": 0 }));
}

#[test]
fn graph_set_additional_ticks_renders_to_chart_scene_with_undo_redo() {
    assert!(cmd::registry().get("graph.setAdditionalTicks").is_some());
    let mut session = session_with("Additional Ticks Parity", vec![dual_dose_xy("xy-1")]);
    let x_ticks = vec![
        AdditionalTick { value: -6.5, label: Some(String::from("EC50-A")), draw_gridline: true },
        AdditionalTick { value: -5.0, label: None, draw_gridline: false },
    ];
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "xy-1", "kind": "xy" })),
            ("graph.setAxes", json!({ "sheet_id": "gr-1", "x_axis": { "min": -9.5, "max": -4.0 }, "y_axis": { "min": 0.0, "max": 120.0 } })),
            ("graph.setAdditionalTicks", json!({ "sheet_id": "gr-1", "axis": "x", "ticks": x_ticks })),
            (
                "graph.setAdditionalTicks",
                json!({ "sheet_id": "gr-1", "axis": "y", "ticks": [{ "value": 55.0, "label": "HalfMax", "draw_gridline": true }] }),
            ),
            (
                "graph.setAdditionalTicks",
                json!({ "sheet_id": "gr-1", "axis": "y2", "ticks": [{ "value": 80.0, "label": "TopRef", "draw_gridline": false }] }),
            ),
        ],
    );
    let g = gs(&session, "gr-1");
    assert_eq!(
        (g.spec.x_axis.additional_ticks.len(), g.spec.y_axis.additional_ticks.len(), g.spec.y2_axis.as_ref().map(|a| a.additional_ticks.len())),
        (2, 1, Some(1))
    );

    let scene = compile_sheet_scene(session.document(), &SheetId::new("gr-1")).unwrap();
    let texts = plain_scene_texts(&scene);
    assert!(["EC50-A", "HalfMax", "TopRef"].iter().all(|lbl| texts.iter().any(|t| t == lbl)) && count_dotted_gridlines(&scene) >= 2);

    ok_cmd(&mut session, "edit.undo", json!({}));
    assert!(gs(&session, "gr-1").spec.y2_axis.is_none());
    ok_cmd(&mut session, "edit.redo", json!({}));
    assert_eq!(gs(&session, "gr-1").spec.y2_axis.as_ref().map(|a| a.additional_ticks.len()), Some(1));
}

#[test]
fn analysis_add_new_biostatistics_and_multiple_imputation_kinds() {
    assert!(cmd::registry().get("analysis.add").is_some());
    let grp = grouped_rep_sheet(
        "grp-mr",
        "Multiple Row Expression",
        &["Gene A", "Gene B", "Gene C", "Gene D"],
        &[
            ("Control", &[&[10.0, 10.5, 9.8], &[20.1, 19.9, 20.4], &[5.0, 5.2, 4.9], &[15.0, 15.2, 14.8]]),
            ("Treated", &[&[18.5, 19.2, 18.9], &[20.2, 20.0, 20.3], &[12.1, 12.6, 12.4], &[15.1, 14.9, 15.3]]),
        ],
    );
    let pair = col_sheet(
        "col-pair",
        "Comparison Data",
        &[("Assay A", &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0]), ("Assay B", &[10.8, 19.6, 30.5, 39.2, 51.0, 59.4])],
    );
    let mut session = session_with("New Analyses Parity", vec![grp, pair, clinical_mv("mv-miss", true)]);
    run_cmds(
        &mut session,
        &[
            (
                "analysis.add",
                json!({ "sheet_id": "res-mr", "source_sheet_id": "grp-mr", "kind": "stats.multiple_row_t_tests", "paired": false, "multiplicity": "benjamini_hochberg" }),
            ),
            (
                "analysis.add",
                json!({ "sheet_id": "res-ba", "source_sheet_id": "col-pair", "kind": "stats.bland_altman", "column_a": 0, "column_b": 1, "percent_difference": false }),
            ),
            (
                "analysis.add",
                json!({ "sheet_id": "res-pb", "source_sheet_id": "col-pair", "kind": "stats.passing_bablok", "x_column": 0, "y_column": 1 }),
            ),
            (
                "analysis.add",
                json!({ "sheet_id": "res-mi", "source_sheet_id": "mv-miss", "kind": "stats.multiple_imputation", "imputations_m": 3, "max_iters": 5, "seed": 77, "response_column": 0, "predictor_columns": [1, 2] }),
            ),
        ],
    );

    let (mr, ba, pb, mi) = (rs(&session, "res-mr"), rs(&session, "res-ba"), rs(&session, "res-pb"), rs(&session, "res-mi"));
    assert!(
        mr.status == ResultStatus::UpToDate && matches!(mr.analysis.kind, AnalysisKind::MultipleRowTTests(_)) && !mr.pairwise_comparisons.is_empty()
    );
    assert!(ba.status == ResultStatus::UpToDate && matches!(ba.analysis.kind, AnalysisKind::BlandAltman(_)) && !ba.scalars.is_empty());
    assert!(pb.status == ResultStatus::UpToDate && matches!(pb.analysis.kind, AnalysisKind::PassingBablok(_)) && !pb.fitted_curves.is_empty());
    assert!(
        mi.status == ResultStatus::UpToDate
            && matches!(
                &mi.analysis.kind,
                AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                    imputations_m: 3,
                    max_iters: 5,
                    seed: 77,
                    response_column: Some(0),
                    ..
                })
            )
    );
}

#[test]
fn multiplicity_barnard_and_multivariate_two_group_km_parity() {
    let ct = contingency_sheet("ct-1", "2x2 Contingency", &["Responder", "Non-responder"], &["Drug", "Placebo"], &[&[9.0, 2.0], &[3.0, 10.0]]);
    let mut session = session_with("Stats Overrides Parity", vec![three_group_col("col-anova"), clinical_mv("mv-surv", false), ct]);
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "res-snk", "source_sheet_id": "col-anova", "kind": "one_way_anova", "multiplicity": "newman_keuls" })),
            ("stats.run", json!({ "sheet_id": "res-lsd", "source_sheet_id": "col-anova", "kind": "one_way_anova", "multiplicity": "fisher_lsd" })),
            ("stats.run", json!({ "sheet_id": "res-barnard", "source_sheet_id": "ct-1", "kind": "stats.barnard_exact" })),
            (
                "stats.run",
                json!({ "sheet_id": "res-mv-t", "source_sheet_id": "mv-surv", "kind": "unpaired_t", "mv_group_column": 3, "mv_response_column": 0 }),
            ),
            (
                "stats.run",
                json!({ "sheet_id": "res-mv-km", "source_sheet_id": "mv-surv", "kind": "kaplan_meier", "mv_time_column": 1, "mv_status_column": 2, "mv_group_column": 3, "log_rank_mantel_cox": true }),
            ),
        ],
    );
    assert!(
        matches!(rs(&session, "res-snk").analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.multiplicity.method == MultiplicityMethod::NewmanKeuls)
    );
    assert!(
        matches!(rs(&session, "res-lsd").analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.multiplicity.method == MultiplicityMethod::FisherLsd)
    );
    assert!(matches!(rs(&session, "res-barnard").analysis.kind, AnalysisKind::Contingency(ref s) if s.method == ContingencyMethod::BarnardExact));
    assert!(
        matches!(rs(&session, "res-mv-t").analysis.kind, AnalysisKind::TwoGroupComparison(ref s) if s.mv_group_column == Some(3) && s.mv_response_column == Some(0))
    );
    assert!(matches!(
        rs(&session, "res-mv-km").analysis.kind,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            mv_time_column: Some(1),
            mv_status_column: Some(2),
            mv_group_column: Some(3),
            ..
        })
    ));
    for id in ["res-snk", "res-lsd", "res-barnard", "res-mv-t", "res-mv-km"] {
        assert_eq!(rs(&session, id).status, ResultStatus::UpToDate);
    }
}

#[test]
fn nonlinear_fit_overrides_parameter_scopes_and_lrt_parity() {
    let mut session =
        session_with("Nonlinear Fit Parity", vec![dual_dose_xy("xy-dose"), xy_sheet("xy-empty", "Function Curve", "X", &[], &[("Y", &[])])]);
    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-parity", "source_sheet_id": "xy-dose", "equation_id": "log_agonist_vs_response_4pl",
            "parameter_rules": [{ "name": "Top", "scope": "per_dataset_constant", "values": [[0, 100.0], [1, 100.0]] }, { "name": "Bottom", "scope": "column_title_constant" }],
            "comparison": { "method": "likelihood_ratio_test", "alternative_equation_id": "log_agonist_vs_response_3pl" },
            "x_range_filter": [-9.5, -4.0], "y_range_filter": [0.0, 120.0], "plot_x_min": -10.0, "plot_x_max": -3.5,
            "emit_residuals_tab": true, "emit_covariance_tab": true, "homoscedasticity_test": true, "normality_of_residuals": true, "band_log_spacing": false
        }),
    );

    let res = rs(&session, "fit-parity");
    assert_eq!(res.status, ResultStatus::UpToDate);
    let AnalysisKind::NonlinearRegression(ref nl) = res.analysis.kind else { panic!("expected NonlinearRegression") };
    assert!(
        nl.parameter_rules.len() == 2
            && matches!(nl.parameter_rules[0].scope, ParameterScope::PerDatasetConstant(ref v) if v == &vec![(0, 100.0), (1, 100.0)])
            && nl.parameter_rules[1].scope == ParameterScope::ColumnTitleConstant
    );
    assert_eq!(
        nl.comparison,
        Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: String::from("log_agonist_vs_response_3pl") })
    );
    assert_eq!(
        (nl.x_range_filter, nl.y_range_filter, nl.plot_x_min, nl.plot_x_max),
        (Some((Some(-9.5), Some(-4.0))), Some((Some(0.0), Some(120.0))), Some(-10.0), Some(-3.5))
    );
    assert!(nl.emit_residuals_tab && nl.emit_covariance_tab && nl.homoscedasticity_test && nl.normality_of_residuals);
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals) && res.tabs.iter().any(|t| t.kind == ResultTabKind::CovarianceMatrix));

    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-fn-only", "source_sheet_id": "xy-empty", "equation_id": "log_agonist_vs_response_4pl",
            "plot_function_only": true, "plot_x_min": -9.0, "plot_x_max": -4.0,
            "parameter_rules": [{ "name": "Bottom", "scope": "constant", "value": 5.0 }, { "name": "Top", "scope": "constant", "value": 95.0 }, { "name": "LogEC50", "scope": "constant", "value": -6.5 }, { "name": "HillSlope", "scope": "constant", "value": 1.0 }]
        }),
    );
    assert!(rs(&session, "fit-fn-only").status == ResultStatus::UpToDate && !rs(&session, "fit-fn-only").fitted_curves.is_empty());
}

#[test]
fn circular_hook_sanitization_prevents_dag_stall() {
    let mut doc = ProjectDocument::new("Circular Hook Guard").unwrap();
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Constants").unwrap();
    info.set_constant("ValidBase", ConstantValue::Number(5.0), None).unwrap();
    doc.add_info_sheet(info).unwrap();
    doc.add_data_sheet(dual_dose_xy("xy-hook")).unwrap();

    let scalar_rule = |name: &str, rid: &str| ParameterRule {
        name: name.to_owned(),
        scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
            result_sheet_id: SheetId::new(rid),
            parameter_key: String::from("Top"),
            data_set_index: Some(0),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    };
    let mut fit1 = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit1.parameter_rules = vec![
        scalar_rule("Top", "r2"),
        ParameterRule {
            name: String::from("Bottom"),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-1")),
                constant_name: String::from("ValidBase"),
            }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
    ];
    let mut fit2 = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit2.parameter_rules = vec![scalar_rule("Top", "r1")];
    let norm_cycle = TransformAnalysisSpec::Normalize {
        zero_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("r1"),
            parameter_key: String::from("Top"),
            data_set_index: Some(0),
        }),
        hundred_anchor: NormalizeAnchor::ColumnMax,
        output_percent: true,
        per_data_set: false,
    };

    for (id, kind) in [
        ("r1", AnalysisKind::NonlinearRegression(fit1)),
        ("r2", AnalysisKind::NonlinearRegression(fit2)),
        ("r-norm", AnalysisKind::Transform(norm_cycle)),
    ] {
        doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new(id, "xy-hook", kind).unwrap()).unwrap()).unwrap();
    }

    assert!(dag::recalculate_project(&mut doc.clone()).is_err());

    let mut san = doc.clone();
    let (_count, diagnostics) = dag::recalculate_project_sanitized(&mut san).unwrap();
    assert!(!diagnostics.is_empty());
    let AnalysisKind::NonlinearRegression(ref cleaned_fit) = san.result_sheet(&SheetId::new("r1")).unwrap().analysis.kind else {
        panic!("expected NL")
    };
    assert!(matches!(cleaned_fit.parameter_rules[1].scope, ParameterScope::HookedConstant(_)));

    let mut session = Session::from_document(doc);
    session.recalculate_all().unwrap();
    assert_eq!((rs(&session, "r1").status.clone(), rs(&session, "r2").status.clone()), (ResultStatus::UpToDate, ResultStatus::UpToDate));
}

#[test]
fn crash_recovery_journal_torn_tail_replay_into_session() {
    let (mut s1, mut j1, init_report) = Session::open_with_recovery(MemoryJournalBackend::new(), "client-1", 5_000, 1_000).unwrap();
    assert!(init_report.recovered_commands.is_empty());

    s1.execute_and_record(&mut j1, "sheet.createData", json!({ "sheet_id": "d1", "title": "Base", "archetype": "column", "cols": 2, "rows": 2 }))
        .unwrap();
    s1.checkpoint_journal(&mut j1).unwrap();
    s1.execute_and_record(
        &mut j1,
        "table.setCells",
        json!({ "sheet_id": "d1", "edits": [{ "row": 0, "col": 0, "value": 42.5 }, { "row": 0, "col": 1, "value": 84.0 }] }),
    )
    .unwrap();
    s1.execute_and_record(&mut j1, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "d1", "kind": "column" })).unwrap();
    s1.execute_and_record(
        &mut j1,
        "graph.setAdditionalTicks",
        json!({ "sheet_id": "g1", "axis": "y", "ticks": [{ "value": 50.0, "label": "Threshold50", "draw_gridline": true }] }),
    )
    .unwrap();

    let mut crashed = j1.abandon();
    crashed.append_raw_journal_bytes(b"{\"seq\":99,\"corrupt_tail");
    let restored = MemoryJournalBackend::from_web_storage_json(&crashed.to_web_storage_json().unwrap()).unwrap();

    let (mut recovered, j2, rec_report) = Session::open_with_recovery(restored, "client-2", 5_000, 7_000).unwrap();
    assert_eq!((rec_report.discarded_corrupt_tail_lines, rec_report.recovered_commands.len()), (1, 3));
    assert_eq!(rec_report.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str()), Some("client-1"));
    assert_eq!(cell_num(&recovered, "d1", 0, 0, 0), Some(42.5));
    assert!(plain_scene_texts(&compile_sheet_scene(recovered.document(), &SheetId::new("g1")).unwrap()).iter().any(|t| t == "Threshold50"));

    let finalized = recovered.close_journal_cleanly(j2).unwrap();
    assert!(finalized.lock_lease().is_none() && finalized.journal_bytes().is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_directory_journal_torn_tail_replay_into_session() {
    use std::fs::OpenOptions;
    use std::io::Write;

    let dir = TempDir::new("native-recovery");
    let (mut s1, mut j1, _) = Session::open_dir_with_recovery(dir.path(), "native-1", 2_000, 10_000).unwrap();
    s1.execute_and_record(
        &mut j1,
        "sheet.createData",
        json!({ "sheet_id": "d-nat", "title": "Native", "archetype": "column", "cols": 1, "rows": 2 }),
    )
    .unwrap();
    s1.checkpoint_journal(&mut j1).unwrap();
    s1.execute_and_record(&mut j1, "table.setCell", json!({ "sheet_id": "d-nat", "row": 1, "col": 0, "value": 99.5 })).unwrap();
    drop(j1.abandon());

    let mut wal = OpenOptions::new().append(true).open(dir.child(JOURNAL_FILE_NAME)).unwrap();
    wal.write_all(b"{\"seq\":2,\"torn_line").unwrap();
    wal.flush().unwrap();

    let (mut s2, j2, report) = Session::open_dir_with_recovery(dir.path(), "native-2", 2_000, 13_000).unwrap();
    assert_eq!((report.discarded_corrupt_tail_lines, report.recovered_commands.len()), (1, 1));
    assert_eq!(cell_num(&s2, "d-nat", 1, 0, 0), Some(99.5));
    let _ = s2.close_journal_cleanly(j2).unwrap();
}

#[test]
fn restricted_linear_fit_uses_the_same_domain_for_analysis_and_overlay() {
    let mut session = session_with(
        "Terminal Phase",
        vec![xy_sheet("xy-terminal", "Terminal Phase", "X", &[0.0, 10.0, 12.0, 14.0, 16.0, 20.0], &[("Y", &[90.0, 24.0, 26.0, 30.0, 36.0, -100.0])])],
    );
    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "terminal", "source_sheet_id": "xy-terminal", "kind": "linear", "x_range_filter": [10, 16], "plot_segments": 3, "confidence_bands": false }),
    );

    let result = rs(&session, "terminal");
    let AnalysisKind::LinearRegression(spec) = &result.analysis.kind else { panic!("expected linear fit") };
    assert_eq!(spec.x_range_filter, Some((Some(10.0), Some(16.0))));
    assert_eq!((result.lookup_scalar("Slope", Some(0)), result.lookup_scalar("R2", Some(0))), (Some(2.0), Some(20.0 / 21.0)));
    let overlay = &result.fitted_curves[0].points;
    assert_eq!(overlay.iter().map(|p| p.x).collect::<Vec<_>>(), [10.0, 12.0, 14.0, 16.0]);
    assert!(overlay.iter().all(|p| p.ci_lower.is_none() && p.pred_lower.is_none()));
    let summary = result.tabs.iter().find(|t| t.id == "linear_summary").unwrap();
    assert_eq!(
        &summary.row_labels[10..],
        ["X-intercept", "95% CI X-intercept", "Observations analyzed", "Excluded by X range", "Residual degrees of freedom"]
    );
    for (row, expected) in summary.rows[12..].iter().zip([4.0, 2.0, 2.0]) {
        assert!(matches!(row.as_slice(), [ResultCell::Number { value, .. }] if *value == expected));
    }

    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "terminal-graph", "data_sheet_id": "xy-terminal", "kind": "xy" })),
            ("graph.setAxes", json!({ "sheet_id": "terminal-graph", "x_axis": { "min": 0, "max": 20 }, "y_axis": { "min": -120, "max": 100 } })),
            ("graph.setRegressionOverlay", json!({ "sheet_id": "terminal-graph", "result_sheet_id": "terminal" })),
        ],
    );
    let scene = compile_sheet_scene(session.document(), &SheetId::new("terminal-graph")).unwrap();
    let plot = scene.plot_rect_pt;
    let endpoints = scene
        .primitives
        .iter()
        .find_map(|p| match p {
            ScenePrimitive::Path { path, stroke: Some(st), clip: Some(clip), .. } if *clip == plot && (st.width_pt - 1.75).abs() < 1e-12 => {
                match (path.commands.first(), path.commands.last()) {
                    (Some(PathCmd::MoveTo(first)), Some(PathCmd::LineTo(last))) => Some((first.x, last.x)),
                    _ => None,
                }
            }
            _ => None,
        })
        .unwrap();
    assert!((endpoints.0 - (plot.x + plot.width * 0.5)).abs() < 1e-8 && (endpoints.1 - (plot.x + plot.width * 0.8)).abs() < 1e-8);

    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "terminal-deming", "source_sheet_id": "xy-terminal", "kind": "deming", "x_range_filter": [10, 16] }),
    );
    let deming = rs(&session, "terminal-deming");
    assert!((deming.lookup_scalar("Slope", Some(0)).unwrap() - (64.0 + 10_496.0_f64.sqrt()) / 80.0).abs() < 1e-12);
    assert_eq!((deming.fitted_curves[0].points.first().map(|p| p.x), deming.fitted_curves[0].points.last().map(|p| p.x)), (Some(10.0), Some(16.0)));

    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "xy-terminal", "row": 0, "col": 0, "value": -500.0 }));
    assert_eq!(rs(&session, "terminal").lookup_scalar("Slope", Some(0)), Some(2.0));

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "terminal", "params": { "plot_x_max": 19.0, "confidence_bands": true } }));
    let ext = rs(&session, "terminal");
    assert_eq!(ext.fitted_curves[0].points.iter().map(|p| p.x).collect::<Vec<_>>(), [10.0, 13.0, 16.0, 19.0]);
    assert!(ext.fitted_curves[0].points.iter().all(|p| p.ci_lower.is_some()) && ext.lookup_scalar("Slope", Some(0)) == Some(2.0));

    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "xy-terminal", "row": 4, "col": 0, "value": 38.0 }));
    assert!((rs(&session, "terminal").lookup_scalar("Slope", Some(0)).unwrap() - 2.3).abs() < 1e-12);
}

#[test]
fn nonlinear_diagnostic_settings_run_and_update_through_commands() {
    let mut session =
        session_with("Diagnostics", vec![xy_sheet("xy", "Observations", "X", &[10.0, 12.0, 14.0, 16.0], &[("Y", &[24.0, 26.0, 30.0, 36.0])])]);
    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "fit", "source_sheet_id": "xy", "kind": "nonlinear", "equation_id": "straight_line", "identify_unstable_parameters": true, "flag_r_squared_below": 0.96 }),
    );
    let r = rs(&session, "fit");
    assert!(
        matches!(&r.analysis.kind, AnalysisKind::NonlinearRegression(spec) if spec.identify_unstable_parameters && spec.flag_r_squared_below == Some(0.96))
    );
    assert!(r.checklist.iter().any(|item| item.assumption == "R squared below threshold"));

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "fit", "params": { "flag_r_squared_below": null } }));
    let r = rs(&session, "fit");
    assert!(matches!(&r.analysis.kind, AnalysisKind::NonlinearRegression(spec) if spec.flag_r_squared_below.is_none()));
    assert!(r.checklist.iter().all(|item| item.assumption != "R squared below threshold"));
}
