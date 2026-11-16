//! Wave 3 (`L5` Engine Wiring & Parity) integration tests covering `sheet.reorder`,
//! `graph.setAdditionalTicks` -> `ChartScene`, `#114` biostatistics/imputation/fitting parity,
//! `curvebook_format::sanitize_circular_hooks` integration, and `CrashRecoveryJournal` WAL replay.

use curvebook_chart::{ChartScene, ScenePrimitive};
use curvebook_doc::{
    AdditionalTick, AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyMethod, ContingencyTable, DataSheet,
    GraphSheet, GroupedTable, HookSource, InfoSheet, LayoutSheet, LayoutSpec, ModelComparisonSpec, MultipleVariablesTable, MultiplicityMethod,
    NonlinearFitSpec, NormalizeAnchor, ParameterScope, ProjectDocument, ResultSheet, ResultStatus, ResultTabKind, SheetFolderKind, SheetId,
    SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec, VariableRole, XyTable,
};
use curvebook_engine::{CommandOutcome, Session, cmd, cmd::helpers::compile_sheet_scene, dag};
use curvebook_format::{JOURNAL_FILE_NAME, MemoryJournalBackend};
use curvebook_palette::LineDashStyle;
use serde_json::{Value, json};

type TestRes<T = ()> = Result<T, String>;

fn exec(session: &mut Session, command_id: &str, params: Value) -> TestRes<CommandOutcome> {
    session.execute(command_id, params).map_err(|err| format!("command `{command_id}` failed: {err}"))
}

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

fn build_column_pair_sheet(id: &str, title_a: &str, a: &[f64], title_b: &str, b: &[f64]) -> TestRes<DataSheet> {
    let rows = a.len().max(b.len());
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, rows).map_err(|e| e.to_string())?;
    col.data_sets[0].title = title_a.to_owned();
    col.data_sets[1].title = title_b.to_owned();
    for (r, &v) in a.iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v);
    }
    for (r, &v) in b.iter().enumerate() {
        col.data_sets[1].cells[r][0] = Cell::number(v);
    }
    DataSheet::new(SheetId::new(id), "Comparison Data", TableArchetype::Column(col)).map_err(|e| e.to_string())
}

fn build_three_group_column_sheet(id: &str) -> TestRes<DataSheet> {
    let groups = [("Vehicle", [10.2, 10.9, 9.8, 10.5, 10.1]), ("Low", [14.5, 15.1, 13.9, 14.8, 15.2]), ("High", [22.4, 23.0, 21.9, 22.8, 23.1])];
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).map_err(|e| e.to_string())?;
    for (c, (name, vals)) in groups.iter().enumerate() {
        col.data_sets[c].title = (*name).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), "ANOVA Groups", TableArchetype::Column(col)).map_err(|e| e.to_string())
}

fn build_grouped_multiple_row_sheet(id: &str) -> TestRes<DataSheet> {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 4).map_err(|e| e.to_string())?;
    grp.row_titles = vec![String::from("Gene A"), String::from("Gene B"), String::from("Gene C"), String::from("Gene D")];
    grp.data_sets[0].title = String::from("Control");
    grp.data_sets[1].title = String::from("Treated");
    let ctrl = [[10.0, 10.5, 9.8], [20.1, 19.9, 20.4], [5.0, 5.2, 4.9], [15.0, 15.2, 14.8]];
    let trt = [[18.5, 19.2, 18.9], [20.2, 20.0, 20.3], [12.1, 12.6, 12.4], [15.1, 14.9, 15.3]];
    for r in 0..4 {
        for s in 0..3 {
            grp.data_sets[0].cells[r][s] = Cell::number(ctrl[r][s]);
            grp.data_sets[1].cells[r][s] = Cell::number(trt[r][s]);
        }
    }
    DataSheet::new(SheetId::new(id), "Multiple Row Expression", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())
}

fn build_multivariate_clinical_sheet(id: &str, with_missing: bool) -> TestRes<DataSheet> {
    let mut mv = MultipleVariablesTable::new(4, 10).map_err(|e| e.to_string())?;
    mv.variables[0].title = String::from("Response");
    mv.variables[1].title = String::from("Age");
    mv.variables[2].title = String::from("Status");
    mv.variables[3].title = String::from("Cohort");
    mv.variables[3].role = VariableRole::Categorical;
    mv.variables[3].role_inferred = false;

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
    for (r, &(resp, age, status, grp)) in rows.iter().enumerate() {
        mv.row_titles[r] = format!("P{}", r + 1);
        mv.variables[0].cells[r] = if with_missing && r == 2 { Cell::empty() } else { Cell::number(resp) };
        mv.variables[1].cells[r] = if with_missing && r == 7 { Cell::empty() } else { Cell::number(age) };
        mv.variables[2].cells[r] = Cell::number(status);
        mv.variables[3].cells[r] = Cell::text(grp);
    }
    DataSheet::new(SheetId::new(id), "Clinical Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())
}

fn build_dose_response_two_dataset_xy(id: &str) -> TestRes<DataSheet> {
    let xs = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5];
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, xs.len()).map_err(|e| e.to_string())?;
    xy.x_title = String::from("Log10 [Agonist] (M)");
    xy.data_sets[0].title = String::from("10.0");
    xy.data_sets[1].title = String::from("20.0");

    for (r, &x) in xs.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        let y0 = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.2));
        let y1 = 20.0 + 80.0 / (1.0 + 10.0_f64.powf((-7.0 - x) * 1.1));
        xy.data_sets[0].cells[r][0] = Cell::number(y0 - 0.5);
        xy.data_sets[0].cells[r][1] = Cell::number(y0 + 0.5);
        xy.data_sets[1].cells[r][0] = Cell::number(y1 - 0.6);
        xy.data_sets[1].cells[r][1] = Cell::number(y1 + 0.6);
    }
    DataSheet::new(SheetId::new(id), "Dual Dose Response", TableArchetype::Xy(xy)).map_err(|e| e.to_string())
}

fn result_sheet_by_id<'a>(session: &'a Session, id: &str) -> TestRes<&'a ResultSheet> {
    session.document().result_sheet(&SheetId::new(id)).ok_or_else(|| format!("missing result sheet `{id}`"))
}

#[test]
fn sheet_reorder_all_folders_and_undo_redo() -> TestRes {
    let mut doc = ProjectDocument::new("Reorder Parity").map_err(|e| e.to_string())?;
    let ds1 = build_three_group_column_sheet("d1")?;
    let ds2 = build_three_group_column_sheet("d2")?;
    let ds3 = build_three_group_column_sheet("d3")?;
    doc.add_data_sheet(ds1.clone()).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds2.clone()).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds3).map_err(|e| e.to_string())?;

    doc.add_info_sheet(InfoSheet::new_preseeded("i1", "Info 1").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    doc.add_info_sheet(InfoSheet::new_preseeded("i2", "Info 2").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let r1 = ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::DescriptiveStats(Default::default())).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let r2 = ResultSheet::new(AnalysisSpec::new("r2", "d2", AnalysisKind::DescriptiveStats(Default::default())).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    doc.add_result_sheet(r1).map_err(|e| e.to_string())?;
    doc.add_result_sheet(r2).map_err(|e| e.to_string())?;

    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("g1", &ds1).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("g2", &ds2).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    doc.add_layout_sheet(LayoutSheet::new("l1", "L1", LayoutSpec::default()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(LayoutSheet::new("l2", "L2", LayoutSpec::default()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);

    exec(&mut session, "sheet.reorder", json!({ "folder": "data", "from_index": 0, "to_index": 2 }))?;
    let data_ids: Vec<&str> = session.document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(data_ids, vec!["d2", "d3", "d1"]);

    exec(&mut session, "edit.undo", json!({}))?;
    let reverted: Vec<&str> = session.document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(reverted, vec!["d1", "d2", "d3"]);

    exec(&mut session, "edit.redo", json!({}))?;
    let redone: Vec<&str> = session.document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(redone, vec!["d2", "d3", "d1"]);

    exec(&mut session, "sheet.reorder", json!({ "folder": SheetFolderKind::Info, "from_index": 1, "to_index": 0 }))?;
    exec(&mut session, "sheet.reorder", json!({ "folder": "results", "from_index": 0, "to_index": 1 }))?;
    exec(&mut session, "sheet.reorder", json!({ "folder": "graphs", "from_index": 1, "to_index": 0 }))?;
    exec(&mut session, "sheet.reorder", json!({ "folder": "layouts", "from_index": 0, "to_index": 1 }))?;

    assert_eq!(session.document().info_sheets[0].id.as_str(), "i2");
    assert_eq!(session.document().result_sheets[0].id.as_str(), "r2");
    assert_eq!(session.document().graph_sheets[0].id.as_str(), "g2");
    assert_eq!(session.document().layout_sheets[0].id.as_str(), "l2");

    assert!(session.execute("sheet.reorder", json!({ "folder": "data", "from_index": 9, "to_index": 0 })).is_err());
    Ok(())
}

#[test]
fn graph_set_additional_ticks_renders_to_chart_scene_with_undo_redo() -> TestRes {
    assert!(cmd::registry().get("graph.setAdditionalTicks").is_some());
    let mut doc = ProjectDocument::new("Additional Ticks Parity").map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_dose_response_two_dataset_xy("xy-1")?).map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);
    exec(&mut session, "graph.create", json!({ "sheet_id": "gr-1", "data_sheet_id": "xy-1", "kind": "xy" }))?;
    exec(
        &mut session,
        "graph.setAxes",
        json!({ "sheet_id": "gr-1", "x_axis": { "min": -9.5, "max": -4.0 }, "y_axis": { "min": 0.0, "max": 120.0 } }),
    )?;

    let x_ticks = vec![
        AdditionalTick { value: -6.5, label: Some(String::from("EC50-A")), draw_gridline: true },
        AdditionalTick { value: -5.0, label: None, draw_gridline: false },
    ];
    exec(&mut session, "graph.setAdditionalTicks", json!({ "sheet_id": "gr-1", "axis": "x", "ticks": x_ticks }))?;
    exec(
        &mut session,
        "graph.setAdditionalTicks",
        json!({ "sheet_id": "gr-1", "axis": "y", "ticks": [{ "value": 55.0, "label": "HalfMax", "draw_gridline": true }] }),
    )?;
    exec(
        &mut session,
        "graph.setAdditionalTicks",
        json!({ "sheet_id": "gr-1", "axis": "y2", "ticks": [{ "value": 80.0, "label": "TopRef", "draw_gridline": false }] }),
    )?;

    let graph = session.document().graph_sheet(&SheetId::new("gr-1")).ok_or_else(|| String::from("missing graph"))?;
    assert_eq!(graph.spec.x_axis.additional_ticks.len(), 2);
    assert_eq!(graph.spec.y_axis.additional_ticks.len(), 1);
    assert_eq!(graph.spec.y2_axis.as_ref().map(|a| a.additional_ticks.len()), Some(1));

    let scene = compile_sheet_scene(session.document(), &SheetId::new("gr-1")).map_err(|e| e.to_string())?;
    let texts = plain_scene_texts(&scene);
    assert!(texts.iter().any(|t| t == "EC50-A"), "expected X additional tick label in ChartScene: {texts:?}");
    assert!(texts.iter().any(|t| t == "HalfMax"), "expected Y additional tick label in ChartScene: {texts:?}");
    assert!(texts.iter().any(|t| t == "TopRef"), "expected Y2 additional tick label in ChartScene: {texts:?}");
    assert!(count_dotted_gridlines(&scene) >= 2, "expected dotted additional tick gridlines");

    // Undo removes Y2 additional ticks; redo restores them cleanly.
    exec(&mut session, "edit.undo", json!({}))?;
    let after_undo = session.document().graph_sheet(&SheetId::new("gr-1")).ok_or_else(|| String::from("missing graph"))?;
    assert!(after_undo.spec.y2_axis.is_none());

    exec(&mut session, "edit.redo", json!({}))?;
    let after_redo = session.document().graph_sheet(&SheetId::new("gr-1")).ok_or_else(|| String::from("missing graph"))?;
    assert_eq!(after_redo.spec.y2_axis.as_ref().map(|a| a.additional_ticks.len()), Some(1));
    Ok(())
}

#[test]
fn analysis_add_new_biostatistics_and_multiple_imputation_kinds() -> TestRes {
    assert!(cmd::registry().get("analysis.add").is_some());
    let mut doc = ProjectDocument::new("New Analyses Parity").map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_grouped_multiple_row_sheet("grp-mr")?).map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_column_pair_sheet(
        "col-pair",
        "Assay A",
        &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0],
        "Assay B",
        &[10.8, 19.6, 30.5, 39.2, 51.0, 59.4],
    )?)
    .map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_multivariate_clinical_sheet("mv-miss", true)?).map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);

    exec(
        &mut session,
        "analysis.add",
        json!({
            "sheet_id": "res-mr",
            "source_sheet": "grp-mr",
            "kind": "stats.multiple_row_t_tests",
            "paired": false,
            "multiplicity": "benjamini_hochberg"
        }),
    )?;
    let mr = result_sheet_by_id(&session, "res-mr")?;
    assert_eq!(mr.status, ResultStatus::UpToDate);
    assert!(matches!(mr.analysis.kind, AnalysisKind::MultipleRowTTests(_)));
    assert!(!mr.pairwise_comparisons.is_empty());

    exec(
        &mut session,
        "analysis.add",
        json!({
            "sheet_id": "res-ba",
            "source_sheet": "col-pair",
            "kind": "stats.bland_altman",
            "column_a": 0,
            "column_b": 1,
            "percent_difference": false
        }),
    )?;
    let ba = result_sheet_by_id(&session, "res-ba")?;
    assert_eq!(ba.status, ResultStatus::UpToDate);
    assert!(matches!(ba.analysis.kind, AnalysisKind::BlandAltman(_)));
    assert!(!ba.scalars.is_empty());

    exec(
        &mut session,
        "analysis.add",
        json!({
            "sheet_id": "res-pb",
            "source_sheet": "col-pair",
            "kind": "stats.passing_bablok",
            "x_column": 0,
            "y_column": 1
        }),
    )?;
    let pb = result_sheet_by_id(&session, "res-pb")?;
    assert_eq!(pb.status, ResultStatus::UpToDate);
    assert!(matches!(pb.analysis.kind, AnalysisKind::PassingBablok(_)));
    assert!(!pb.fitted_curves.is_empty());

    exec(
        &mut session,
        "analysis.add",
        json!({
            "sheet_id": "res-mi",
            "source_sheet": "mv-miss",
            "kind": "stats.multiple_imputation",
            "imputations_m": 3,
            "max_iters": 5,
            "seed": 77,
            "response_column": 0,
            "predictor_columns": [1, 2]
        }),
    )?;
    let mi = result_sheet_by_id(&session, "res-mi")?;
    assert_eq!(mi.status, ResultStatus::UpToDate);
    assert!(matches!(
        &mi.analysis.kind,
        AnalysisKind::Multivariate(curvebook_doc::MultivariateSpec::MultipleImputation {
            imputations_m: 3,
            max_iters: 5,
            seed: 77,
            response_column: Some(0),
            ..
        })
    ));
    Ok(())
}

#[test]
fn multiplicity_barnard_and_multivariate_two_group_km_parity() -> TestRes {
    let mut doc = ProjectDocument::new("Stats Overrides Parity").map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_three_group_column_sheet("col-anova")?).map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_multivariate_clinical_sheet("mv-surv", false)?).map_err(|e| e.to_string())?;

    let mut ctab = ContingencyTable::new(2, 2).map_err(|e| e.to_string())?;
    ctab.data_sets[0].title = String::from("Drug");
    ctab.data_sets[1].title = String::from("Placebo");
    ctab.row_titles = vec![String::from("Responder"), String::from("Non-responder")];
    ctab.data_sets[0].cells[0][0] = Cell::number(9.0);
    ctab.data_sets[0].cells[1][0] = Cell::number(3.0);
    ctab.data_sets[1].cells[0][0] = Cell::number(2.0);
    ctab.data_sets[1].cells[1][0] = Cell::number(10.0);
    doc.add_data_sheet(DataSheet::new(SheetId::new("ct-1"), "2x2 Contingency", TableArchetype::Contingency(ctab)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);

    exec(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "res-snk", "source_sheet": "col-anova", "kind": "one_way_anova", "multiplicity": "newman_keuls" }),
    )?;
    exec(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "res-lsd", "source_sheet": "col-anova", "kind": "one_way_anova", "multiplicity": "fisher_lsd" }),
    )?;
    exec(&mut session, "stats.run", json!({ "sheet_id": "res-barnard", "source_sheet": "ct-1", "kind": "stats.barnard_exact" }))?;
    exec(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "res-mv-t",
            "source_sheet": "mv-surv",
            "kind": "unpaired_t",
            "mv_group_column": 3,
            "mv_response_column": 0
        }),
    )?;
    exec(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "res-mv-km",
            "source_sheet": "mv-surv",
            "kind": "kaplan_meier",
            "mv_time_column": 1,
            "mv_status_column": 2,
            "mv_group_column": 3,
            "log_rank_mantel_cox": true
        }),
    )?;

    let snk = result_sheet_by_id(&session, "res-snk")?;
    let lsd = result_sheet_by_id(&session, "res-lsd")?;
    let barnard = result_sheet_by_id(&session, "res-barnard")?;
    let mv_t = result_sheet_by_id(&session, "res-mv-t")?;
    let mv_km = result_sheet_by_id(&session, "res-mv-km")?;

    assert!(matches!(snk.analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.multiplicity.method == MultiplicityMethod::NewmanKeuls));
    assert!(matches!(lsd.analysis.kind, AnalysisKind::OneWayAnova(ref s) if s.multiplicity.method == MultiplicityMethod::FisherLsd));
    assert!(matches!(barnard.analysis.kind, AnalysisKind::Contingency(ref s) if s.method == ContingencyMethod::BarnardExact));
    assert!(matches!(
        mv_t.analysis.kind,
        AnalysisKind::TwoGroupComparison(ref s) if s.mv_group_column == Some(3) && s.mv_response_column == Some(0)
    ));
    assert!(matches!(
        mv_km.analysis.kind,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            mv_time_column: Some(1),
            mv_status_column: Some(2),
            mv_group_column: Some(3),
            ..
        })
    ));
    for sheet in [snk, lsd, barnard, mv_t, mv_km] {
        assert_eq!(sheet.status, ResultStatus::UpToDate);
    }
    Ok(())
}

#[test]
fn nonlinear_fit_overrides_parameter_scopes_and_lrt_parity() -> TestRes {
    let mut doc = ProjectDocument::new("Nonlinear Fit Parity").map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_dose_response_two_dataset_xy("xy-dose")?).map_err(|e| e.to_string())?;
    let empty_xy = XyTable::new(SubcolumnMode::Single, 1, 2).map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new(SheetId::new("xy-empty"), "Function Curve", TableArchetype::Xy(empty_xy)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut session = Session::from_document(doc);

    exec(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-parity",
            "source_sheet": "xy-dose",
            "equation_id": "log_agonist_vs_response_4pl",
            "parameter_rules": [
                { "name": "Top", "scope": "per_dataset_constant", "values": [[0, 100.0], [1, 100.0]] },
                { "name": "Bottom", "scope": "column_title_constant" }
            ],
            "model_comparison": { "method": "likelihood_ratio_test", "alternative_equation_id": "log_agonist_vs_response_3pl" },
            "x_range_filter": [-9.5, -4.0],
            "y_range_filter": [0.0, 120.0],
            "plot_x_min": -10.0,
            "plot_x_max": -3.5,
            "emit_residuals_tab": true,
            "emit_covariance_tab": true,
            "homoscedasticity_test": true,
            "normality_tests": true,
            "band_log_spacing": false
        }),
    )?;

    let res = result_sheet_by_id(&session, "fit-parity")?;
    assert_eq!(res.status, ResultStatus::UpToDate);
    let AnalysisKind::NonlinearRegression(ref nl) = res.analysis.kind else {
        return Err(String::from("expected NonlinearRegression kind"));
    };
    assert_eq!(nl.parameter_rules.len(), 2);
    assert!(matches!(nl.parameter_rules[0].scope, ParameterScope::PerDatasetConstant(ref v) if v == &vec![(0, 100.0), (1, 100.0)]));
    assert_eq!(nl.parameter_rules[1].scope, ParameterScope::ColumnTitleConstant);
    assert_eq!(
        nl.comparison,
        Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: String::from("log_agonist_vs_response_3pl") })
    );
    assert_eq!(nl.x_range_filter, Some((Some(-9.5), Some(-4.0))));
    assert_eq!(nl.y_range_filter, Some((Some(0.0), Some(120.0))));
    assert_eq!(nl.plot_x_min, Some(-10.0));
    assert_eq!(nl.plot_x_max, Some(-3.5));
    assert!(nl.emit_residuals_tab);
    assert!(nl.emit_covariance_tab);
    assert!(nl.homoscedasticity_test);
    assert!(nl.normality_of_residuals);
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals));
    assert!(res.tabs.iter().any(|t| t.kind == ResultTabKind::CovarianceMatrix));

    // Plot-function-only mode on an empty XY sheet generates synthetic curves cleanly.
    exec(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-fn-only",
            "source_sheet": "xy-empty",
            "equation_id": "log_agonist_vs_response_4pl",
            "plot_function_only": true,
            "plot_x_min": -9.0,
            "plot_x_max": -4.0,
            "parameter_rules": [
                { "name": "Bottom", "scope": "constant", "value": 5.0 },
                { "name": "Top", "scope": "constant", "value": 95.0 },
                { "name": "LogEC50", "scope": "constant", "value": -6.5 },
                { "name": "HillSlope", "scope": "constant", "value": 1.0 }
            ]
        }),
    )?;
    let fn_only = result_sheet_by_id(&session, "fit-fn-only")?;
    assert_eq!(fn_only.status, ResultStatus::UpToDate);
    assert!(!fn_only.fitted_curves.is_empty());
    Ok(())
}

#[test]
fn circular_hook_sanitization_prevents_dag_stall() -> TestRes {
    let mut doc = ProjectDocument::new("Circular Hook Guard").map_err(|e| e.to_string())?;
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Constants").map_err(|e| e.to_string())?;
    info.set_constant("ValidBase", ConstantValue::Number(5.0), None).map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_dose_response_two_dataset_xy("xy-hook")?).map_err(|e| e.to_string())?;

    // Mutual HookSource cycle (r1 <-> r2) + valid acyclic InfoConstant hook on Bottom.
    let mut fit1 = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit1.parameter_rules = vec![
        curvebook_doc::ParameterRule {
            name: String::from("Top"),
            scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
                result_sheet_id: SheetId::new("r2"),
                parameter_key: String::from("Top"),
                data_set_index: Some(0),
            }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        },
        curvebook_doc::ParameterRule {
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
    fit2.parameter_rules = vec![curvebook_doc::ParameterRule {
        name: String::from("Top"),
        scope: ParameterScope::HookedConstant(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("r1"),
            parameter_key: String::from("Top"),
            data_set_index: Some(0),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }];
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

    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("r1", "xy-hook", AnalysisKind::NonlinearRegression(fit1)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("r2", "xy-hook", AnalysisKind::NonlinearRegression(fit2)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    doc.add_result_sheet(
        ResultSheet::new(AnalysisSpec::new("r-norm", "xy-hook", AnalysisKind::Transform(norm_cycle)).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    // Unsanitized direct DAG rejects the mutual HookSource cycle cleanly.
    let mut raw = doc.clone();
    assert!(dag::recalculate_project(&mut raw).is_err());

    // Sanitized DAG + Session::from_document break circular hooks while preserving ValidBase hook.
    let mut san = doc.clone();
    let (_count, diagnostics) = dag::recalculate_project_sanitized(&mut san).map_err(|e| e.to_string())?;
    assert!(!diagnostics.is_empty(), "expected circular hook diagnostics");
    let r1_san = san.result_sheet(&SheetId::new("r1")).ok_or_else(|| String::from("missing r1"))?;
    let AnalysisKind::NonlinearRegression(ref cleaned_fit) = r1_san.analysis.kind else {
        return Err(String::from("expected NonlinearRegression"));
    };
    assert!(matches!(cleaned_fit.parameter_rules[1].scope, ParameterScope::HookedConstant(_)));

    let mut session = Session::from_document(doc);
    session.recalculate_all().map_err(|e| e.to_string())?;
    assert_eq!(result_sheet_by_id(&session, "r1")?.status, ResultStatus::UpToDate);
    assert_eq!(result_sheet_by_id(&session, "r2")?.status, ResultStatus::UpToDate);
    Ok(())
}

#[test]
fn crash_recovery_journal_torn_tail_replay_into_session() -> TestRes {
    let backend = MemoryJournalBackend::new();
    let (mut s1, mut j1, init_report) = Session::open_with_recovery(backend, "client-1", 5_000, 1_000).map_err(|e| e.to_string())?;
    assert!(init_report.recovered_commands.is_empty());

    s1.execute_and_record(&mut j1, "sheet.createData", json!({ "sheet_id": "d1", "title": "Base", "archetype": "column", "cols": 2, "rows": 2 }))
        .map_err(|e| e.to_string())?;
    s1.checkpoint_journal(&mut j1).map_err(|e| e.to_string())?;

    s1.execute_and_record(
        &mut j1,
        "table.setCells",
        json!({ "sheet_id": "d1", "cells": [{ "row": 0, "col": 0, "value": 42.5 }, { "row": 0, "col": 1, "value": 84.0 }] }),
    )
    .map_err(|e| e.to_string())?;
    s1.execute_and_record(&mut j1, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "d1", "kind": "column" }))
        .map_err(|e| e.to_string())?;
    s1.execute_and_record(
        &mut j1,
        "graph.setAdditionalTicks",
        json!({ "sheet_id": "g1", "axis": "y", "ticks": [{ "value": 50.0, "label": "Threshold50", "draw_gridline": true }] }),
    )
    .map_err(|e| e.to_string())?;

    // Simulate sudden process crash leaving active lock and truncated torn WAL tail line.
    let mut crashed_backend = j1.abandon();
    crashed_backend.append_raw_journal_bytes(b"{\"seq\":99,\"corrupt_tail");
    let web_json = crashed_backend.to_web_storage_json().map_err(|e| e.to_string())?;
    let restored_backend = MemoryJournalBackend::from_web_storage_json(&web_json).map_err(|e| e.to_string())?;

    let (mut recovered, j2, rec_report) = Session::open_with_recovery(restored_backend, "client-2", 5_000, 7_000).map_err(|e| e.to_string())?;
    assert_eq!(rec_report.discarded_corrupt_tail_lines, 1);
    assert_eq!(rec_report.recovered_commands.len(), 3);
    assert_eq!(rec_report.recovered_from_stale_lock.as_ref().map(|l| l.owner_id.as_str()), Some("client-1"));

    let ds = recovered.document().data_sheet(&SheetId::new("d1")).ok_or_else(|| String::from("missing recovered d1"))?;
    let TableArchetype::Column(ref col) = ds.archetype else {
        return Err(String::from("expected Column table"));
    };
    assert_eq!(col.data_sets[0].cells[0][0].active_number(), Some(42.5));
    let scene = compile_sheet_scene(recovered.document(), &SheetId::new("g1")).map_err(|e| e.to_string())?;
    assert!(plain_scene_texts(&scene).iter().any(|t| t == "Threshold50"));

    let finalized_backend = recovered.close_journal_cleanly(j2).map_err(|e| e.to_string())?;
    assert!(finalized_backend.lock_lease().is_none());
    assert!(finalized_backend.journal_bytes().is_empty());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_directory_journal_torn_tail_replay_into_session() -> TestRes {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DIR_SEQ: AtomicU64 = AtomicU64::new(1);

    struct GuardDir(std::path::PathBuf);
    impl Drop for GuardDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let id = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cb-engine-recovery-{}-{id}", std::process::id()));
    let _guard = GuardDir(path.clone());

    let (mut s1, mut j1, _) = Session::open_dir_with_recovery(&path, "native-1", 2_000, 10_000).map_err(|e| e.to_string())?;
    s1.execute_and_record(
        &mut j1,
        "sheet.createData",
        json!({ "sheet_id": "d-nat", "title": "Native", "archetype": "column", "cols": 1, "rows": 2 }),
    )
    .map_err(|e| e.to_string())?;
    s1.checkpoint_journal(&mut j1).map_err(|e| e.to_string())?;
    s1.execute_and_record(&mut j1, "table.setCell", json!({ "sheet_id": "d-nat", "row": 1, "col": 0, "value": 99.5 })).map_err(|e| e.to_string())?;
    drop(j1.abandon());

    let mut wal = OpenOptions::new().append(true).open(path.join(JOURNAL_FILE_NAME)).map_err(|e| e.to_string())?;
    wal.write_all(b"{\"seq\":2,\"torn_line").map_err(|e| e.to_string())?;
    wal.flush().map_err(|e| e.to_string())?;

    let (mut s2, j2, report) = Session::open_dir_with_recovery(&path, "native-2", 2_000, 13_000).map_err(|e| e.to_string())?;
    assert_eq!(report.discarded_corrupt_tail_lines, 1);
    assert_eq!(report.recovered_commands.len(), 1);
    let ds = s2.document().data_sheet(&SheetId::new("d-nat")).ok_or_else(|| String::from("missing d-nat"))?;
    let TableArchetype::Column(ref col) = ds.archetype else {
        return Err(String::from("expected Column table"));
    };
    assert_eq!(col.data_sets[0].cells[1][0].active_number(), Some(99.5));

    let _ = s2.close_journal_cleanly(j2).map_err(|e| e.to_string())?;
    Ok(())
}
