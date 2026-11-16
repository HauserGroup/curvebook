//! Wave 22b end-to-end strict `--gui-only` integration tests covering:
//! - `R9.7` (`QOL-7` diagnostics-only banner, same-sheet tab switch, `applyRemediation` rejection)
//! - `R9.12` (`QOL-12` statistical options & pre-summarized `MeanSdN` Column table modal guards)
//! - `R9.14` (`QOL-14` `RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions` dialogs)
//! - `R9.16` (`QOL-16` `InsertModelEquation` + `FormatGraph` `Symbolic`/`Substituted` toggle, no CLD)
//! - `WF-44..WF-48` GUI parity controls (`col.raincloud`, BubblePlot, Pairwise, Layout, Export)

#![forbid(unsafe_code)]

use curvebook_doc::{
    AnalysisKind, BaselineOperation, CellCoord, CellValue, ChecklistStatus, ContingencyMethod, DiagnosticPlotKind, EquationDisplayMode, GlmFamily,
    GraphAnnotation, GraphKind, MultiplicityMethod, MultivariatePlotMode, MultivariateSpec, SheetId, SurvivalAnalysisSpec, TableArchetype, TableKind,
    TransformAnalysisSpec, TwoGroupMethod, VariableRole,
};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AssumptionDiagnosticBanner};
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn strict_gui_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    if !app.is_gui_only() {
        return Err(String::from("expected strict gui_only"));
    }
    Ok(app)
}

fn act(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed [{}]: {}", err.code, err.message()))
}

fn create_data_table(app: &mut UiApp, params: Value) -> Result<String, String> {
    let before: Vec<String> = app.document().data_sheets.iter().map(|s| s.id.as_str().to_owned()).collect();
    let res = act(app, "ui.invoke_menu", json!({ "command_id": "sheet.createData", "params": params }))?;
    if let Some(id) = res.pointer("/payload/sheet_id").and_then(Value::as_str) {
        return Ok(id.to_owned());
    }
    app.document()
        .data_sheets
        .iter()
        .map(|s| s.id.as_str().to_owned())
        .find(|id| !before.contains(id))
        .ok_or_else(|| String::from("sheet.createData did not create a DataSheet"))
}

fn paste_grid(app: &mut UiApp, row: usize, col: usize, subcol: usize, tsv: &str) -> TestRes {
    let wid = format!("grid.cell.{row}.{col}.{subcol}");
    act(app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": wid }))?;
    act(app, "ui.type_text", json!({ "widget_id": wid, "text": tsv, "clear_existing": false, "commit_enter": false }))?;
    Ok(())
}

fn paste_x(app: &mut UiApp, row: usize, subcol: usize, text: &str) -> TestRes {
    let wid = format!("grid.xcell.{row}.{subcol}");
    act(app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": wid }))?;
    act(app, "ui.type_text", json!({ "widget_id": wid, "text": text, "clear_existing": true, "commit_enter": true }))?;
    Ok(())
}

fn select_sheet(app: &mut UiApp, sheet_id: &str) -> TestRes {
    act(app, "ui.switch_tab", json!({ "sheet_id": sheet_id }))?;
    Ok(())
}

fn find_widget<'a>(snap: &'a Value, widget_id: &str) -> Option<&'a Value> {
    fn walk<'a>(v: &'a Value, target: &str) -> Option<&'a Value> {
        match v {
            Value::Object(map) => {
                if map.get("widget_id").and_then(Value::as_str) == Some(target) {
                    return Some(v);
                }
                for child in map.values() {
                    if let Some(found) = walk(child, target) {
                        return Some(found);
                    }
                }
                None
            }
            Value::Array(arr) => arr.iter().find_map(|item| walk(item, target)),
            _ => None,
        }
    }
    walk(snap, widget_id)
}

#[test]
fn r9_7_diagnostics_only_banner_inspect_and_policy_rejection() -> TestRes {
    let mut app = strict_gui_app()?;
    let col_id =
        create_data_table(&mut app, json!({ "title": "Heteroscedastic Variance", "kind": "column", "cols": 3, "rows": 5, "auto_family": true }))?;
    // Wide variance difference triggers equal-variance check violation without remediating automatically.
    paste_grid(&mut app, 0, 0, 0, "10.0\t20.0\t50.0\n10.1\t35.0\t120.0\n10.2\t12.0\t30.0\n10.0\t48.0\t160.0\n10.1\t22.0\t95.0")?;

    let before_res: Vec<String> = app.document().result_sheets.iter().map(|r| r.id.as_str().to_owned()).collect();
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.one_way_anova" } }))?;
    let res_id = app
        .document()
        .result_sheets
        .iter()
        .map(|r| r.id.as_str().to_owned())
        .find(|id| !before_res.contains(id))
        .ok_or_else(|| String::from("expected ANOVA ResultSheet"))?;
    select_sheet(&mut app, &res_id)?;

    let snap = act(&mut app, "ui.inspect", json!({ "scope": "all", "include_bounds": true }))?;
    let banner_json = snap.get("result_diagnostic_banner").ok_or_else(|| format!("missing result_diagnostic_banner: {snap}"))?;
    assert!(banner_json.is_object(), "expected structured result_diagnostic_banner object: {banner_json}");
    let banner_node = find_widget(&snap, "result.diagnostic_banner").ok_or_else(|| String::from("missing canvas banner widget"))?;
    assert_eq!(banner_node["command_id"], "ui.result.inspectDiagnostics");

    let rs = app.document().result_sheet(&SheetId::try_new(&res_id).map_err(|e| e.to_string())?).unwrap();
    let banner = AssumptionDiagnosticBanner::from_result(rs).ok_or_else(|| String::from("expected banner on heteroscedastic ANOVA"))?;
    assert!(matches!(banner.severity, ChecklistStatus::Violated | ChecklistStatus::Warn));
    assert!(!banner.detail_tab_ids.is_empty(), "expected detail_tab_ids populated from same ResultSheet");

    let undo_before = app.session().undo_depth();
    let count_before = app.document().result_sheets.len();
    let click_res = act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "result.diagnostic_banner" }))?;
    assert_eq!(click_res["mutated"], false);
    assert_eq!(app.session().undo_depth(), undo_before);
    assert_eq!(app.document().result_sheets.len(), count_before);

    // Clicking a specific detail tab link must switch active tab with zero document mutation.
    let first_tab = &banner.detail_tab_ids[0];
    let tab_wid = format!("result.diagnostic_banner.tab.{first_tab}");
    let tab_res = act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": tab_wid }))?;
    assert_eq!(tab_res["tab_id"], *first_tab);
    assert_eq!(tab_res["mutated"], false);

    // Calling deprecated `ui.result.applyRemediation` must fail cleanly without mutating the document.
    let err = dispatch_gui_action(
        &mut app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.result.applyRemediation", "params": { "result_sheet_id": res_id } }),
    )
    .expect_err("applyRemediation must fail under QOL-7 diagnostics-only policy");
    assert!(err.message().contains("diagnostics_only_policy"), "unexpected error: {err}");
    assert_eq!(app.document().result_sheets.len(), count_before);
    let _ = col_id;
    Ok(())
}

#[test]
fn r9_12_presummarized_guards_and_extended_statistical_options() -> TestRes {
    let mut app = strict_gui_app()?;

    // 1. Pre-summarized Column table (`MeanSdN`): guards forbid nonparametric / paired switching.
    let pre_col = create_data_table(
        &mut app,
        json!({ "title": "Summary Stats", "kind": "column", "cols": 2, "rows": 1, "subcolumn_mode": "mean_sd_n", "auto_family": false }),
    )?;
    select_sheet(&mut app, &pre_col)?;
    paste_grid(&mut app, 0, 0, 0, "10.0\t1.5\t8\t14.0\t2.0\t8")?;
    let before_res: Vec<String> = app.document().result_sheets.iter().map(|r| r.id.as_str().to_owned()).collect();
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    let an_snap = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_eq!(find_widget(&an_snap, "analyze.assume_gaussian").unwrap()["enabled"], false);
    assert_eq!(find_widget(&an_snap, "analyze.paired_or_matched").unwrap()["enabled"], false);
    let nonparam_err = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "two_group_method": "mann_whitney" } }))
        .expect_err("MeanSdN table must block nonparametric two-group method");
    assert!(nonparam_err.message().to_ascii_lowercase().contains("nonparametric") || nonparam_err.message().contains("pre-summarized"));

    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.two_group", "two_group_method": "unpaired_t" } }))?;
    let pre_res_id = app
        .document()
        .result_sheets
        .iter()
        .map(|r| r.id.as_str().to_owned())
        .find(|id| !before_res.contains(id))
        .ok_or_else(|| String::from("expected ResultSheet for pre-summarized two-group"))?;
    select_sheet(&mut app, &pre_res_id)?;
    act(&mut app, "ui.press_key", json!({ "key": "T", "modifiers": { "meta": true } }))?;
    let ep_snap = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_eq!(find_widget(&ep_snap, "edit.assume_gaussian").unwrap()["enabled"], false);
    assert_eq!(find_widget(&ep_snap, "edit.paired_or_matched").unwrap()["enabled"], false);
    assert!(dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "fields": { "assume_gaussian": false } })).is_err());
    act(&mut app, "ui.answer_dialog", json!({ "action": "cancel" }))?;

    // 2. Paired Sign test via AnalyzeDialog on raw paired Column table.
    let raw_col = create_data_table(
        &mut app,
        json!({ "title": "Paired Sign", "kind": "column", "cols": 2, "rows": 6, "design_hint": "paired", "auto_family": false }),
    )?;
    select_sheet(&mut app, &raw_col)?;
    paste_grid(&mut app, 0, 0, 0, "10\t12\n14\t19\n11\t15\n9\t13\n16\t21\n8\t11")?;
    let res_before_len = app.document().result_sheets.len();
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(
        &mut app,
        "ui.answer_dialog",
        json!({ "action": "ok", "fields": { "entry_id": "stats.two_group", "two_group_method": "paired_sign_test" } }),
    )?;
    assert_eq!(app.document().result_sheets.len(), res_before_len + 1);
    assert!(matches!(
        app.document().result_sheets.last().unwrap().analysis.kind,
        AnalysisKind::TwoGroupComparison(ref s) if s.method == TwoGroupMethod::PairedSignTest
    ));

    // 3. Contingency Kappa weighting + Exact Binomial null_proportion via EditAnalysisParameters.
    let cont_id = create_data_table(&mut app, json!({ "title": "Rater 2x2", "kind": "contingency", "cols": 2, "rows": 2, "auto_family": false }))?;
    select_sheet(&mut app, &cont_id)?;
    paste_grid(&mut app, 0, 0, 0, "20\t3\n4\t18")?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.contingency", "kappa_weighting": "linear" } }))?;
    let cont_res_id = app.document().result_sheets.last().unwrap().id.as_str().to_owned();
    assert!(matches!(
        app.document().result_sheets.last().unwrap().analysis.kind,
        AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::CohenKappa { weighting: curvebook_doc::KappaWeighting::Linear }
    ));
    select_sheet(&mut app, &cont_res_id)?;
    act(&mut app, "ui.press_key", json!({ "key": "T", "modifiers": { "meta": true } }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "contingency_method": "barnard_exact" } }))?;
    assert!(matches!(
        app.document().result_sheet(&SheetId::try_new(&cont_res_id).unwrap()).unwrap().analysis.kind,
        AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::BarnardExact
    ));

    let binom_id =
        create_data_table(&mut app, json!({ "title": "Binomial 1x2", "kind": "contingency", "cols": 2, "rows": 1, "auto_family": false }))?;
    select_sheet(&mut app, &binom_id)?;
    paste_grid(&mut app, 0, 0, 0, "15\t5")?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(
        &mut app,
        "ui.answer_dialog",
        json!({ "action": "ok", "fields": { "entry_id": "stats.contingency", "contingency_method": "exact_binomial", "null_proportion": 0.25 } }),
    )?;
    assert!(matches!(
        app.document().result_sheets.last().unwrap().analysis.kind,
        AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::ExactBinomial && c.null_proportion.is_some_and(|p| (p - 0.25).abs() < 1e-12)
    ));

    // 4. MultipleVariables: Poisson GLM + Clustering via AnalyzeDialog, plus Cox via Cmd+T / Analyze.
    let mv_id = create_data_table(
        &mut app,
        json!({
            "title": "Cohort MV",
            "kind": "multiple_variables",
            "cols": 4,
            "rows": 10,
            "variables": [
                { "title": "Time", "role": "continuous" },
                { "title": "Status", "role": "continuous" },
                { "title": "Dose", "role": "continuous" },
                { "title": "CountY", "role": "continuous" }
            ],
            "auto_family": false
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid(
        &mut app,
        0,
        0,
        0,
        "5\t1\t0.1\t2\n8\t0\t0.2\t3\n12\t1\t0.4\t4\n15\t1\t0.6\t6\n19\t0\t0.8\t7\n21\t1\t1.0\t9\n24\t1\t1.2\t11\n28\t0\t1.4\t12\n31\t1\t1.6\t14\n35\t1\t1.8\t16",
    )?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "entry_id": "stats.glm",
                "glm_family": "poisson",
                "response_column": 3,
                "selected_data_sets": [2],
                "compute_vif": true
            }
        }),
    )?;
    assert!(matches!(
        app.document().result_sheets.last().unwrap().analysis.kind,
        AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Poisson && g.compute_vif
    ));

    select_sheet(&mut app, &mv_id)?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "entry_id": "stats.clustering",
                "selected_data_sets": [0, 2, 3],
                "cluster_distance": "manhattan",
                "cluster_linkage": "ward",
                "k_means_k": 2
            }
        }),
    )?;
    assert!(matches!(
        app.document().result_sheets.last().unwrap().analysis.kind,
        AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: Some(2), .. })
    ));

    select_sheet(&mut app, &mv_id)?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "entry_id": "stats.survival_cox",
                "cox_time_column": 0,
                "cox_status_column": 1,
                "cox_predictor_columns": [2],
                "tie_method": "efron"
            }
        }),
    )?;
    assert!(matches!(
        app.document().result_sheets.last().unwrap().analysis.kind,
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: curvebook_doc::CoxTieMethod::Efron, .. })
    ));

    // 5. RandomSample transform through AnalyzeDialog.
    select_sheet(&mut app, &mv_id)?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "entry_id": "transform.random_sample",
                "sample_size": 5,
                "seed": 33,
                "proportional_allocation": false
            }
        }),
    )?;
    assert!(matches!(
        app.document().result_sheets.last().unwrap().analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed: 33, sample_size: 5, .. })
    ));
    Ok(())
}

#[test]
fn r9_14_randomization_sampling_and_simulation_dialogs_via_menus() -> TestRes {
    let mut app = strict_gui_app()?;

    // 1. Randomize Groups dialog -> creates schedule sheet & records ARRIVE Info constant.
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.table.randomizeGroups" }))?;
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::RandomizeGroups);
    let rand_out = act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "groups": "Control, Treatment",
                "subject_count": 8,
                "scheme": "permuted_block",
                "block_sizes": [4],
                "seed": 123,
                "title": "ARRIVE Allocation"
            }
        }),
    )?;
    let rand_sid = rand_out["payload"]["sheet_id"].as_str().ok_or_else(|| format!("missing sheet_id: {rand_out}"))?.to_owned();
    let rand_ds = app.document().data_sheet(&SheetId::try_new(&rand_sid).unwrap()).unwrap();
    assert_eq!(rand_ds.archetype.row_count(), 8);

    // 2. Simulate Distributions dialog -> creates MultipleVariables table with Gaussian + Poisson columns.
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.table.simulateDistributions" }))?;
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::SimulateDistributions);
    let sim_out = act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "n_rows": 12,
                "seed": 55,
                "use_multiple_variables": true,
                "title": "Synthetic Cohort",
                "columns": [
                    { "name": "Biomarker", "distribution": "gaussian", "mean": 100.0, "sd": 15.0 },
                    { "name": "Cohort", "distribution": "binomial", "trials": 1, "p": 0.5 }
                ]
            }
        }),
    )?;
    let sim_sid = sim_out["payload"]["sheet_id"].as_str().ok_or_else(|| format!("missing sim sheet_id: {sim_out}"))?.to_owned();
    assert_eq!(app.document().data_sheet(&SheetId::try_new(&sim_sid).unwrap()).unwrap().archetype.row_count(), 12);

    // 3. Random Sample Rows dialog on the simulated table.
    select_sheet(&mut app, &sim_sid)?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.table.randomSampleRows" }))?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::RandomSampleRows { .. }));
    let samp_out = act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "sample_size": 6,
                "seed": 9,
                "stratify_column": 1,
                "proportional_allocation": true,
                "title": "Subsample 6"
            }
        }),
    )?;
    let samp_sid = samp_out["payload"]["sheet_id"].as_str().ok_or_else(|| format!("missing sample sheet_id: {samp_out}"))?.to_owned();
    assert_eq!(app.document().data_sheet(&SheetId::try_new(&samp_sid).unwrap()).unwrap().archetype.row_count(), 6);
    Ok(())
}

#[test]
fn r9_16_insert_and_toggle_model_equation_strictly_equations_only() -> TestRes {
    let mut app = strict_gui_app()?;
    let xy_id = create_data_table(&mut app, json!({ "title": "Linear Fit Data", "kind": "xy", "cols": 1, "rows": 5, "auto_family": true }))?;
    select_sheet(&mut app, &xy_id)?;
    for (r, x) in ["1", "2", "3", "4", "5"].into_iter().enumerate() {
        paste_x(&mut app, r, 0, x)?;
    }
    paste_grid(&mut app, 0, 0, 0, "2.1\n4.0\n6.2\n7.9\n10.1")?;

    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "fit.linear" } }))?;
    let res_id = app.document().result_sheets.last().unwrap().id.as_str().to_owned();
    let graph_id = app
        .document()
        .graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id.as_str() == xy_id)
        .map(|g| g.id.as_str().to_owned())
        .ok_or_else(|| String::from("expected linked graph"))?;
    select_sheet(&mut app, &graph_id)?;

    // Open Insert Model Equation dialog -> insert substituted mode equation.
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.insertModelEquation" }))?;
    assert!(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::InsertModelEquation { .. }));
    let dlg_snap = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_eq!(find_widget(&dlg_snap, "equation.mode").unwrap()["value"], "substituted");
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "result_sheet_id": res_id, "mode": "substituted" } }))?;
    let gs = app.document().graph_sheet(&SheetId::try_new(&graph_id).unwrap()).unwrap();
    assert!(
        matches!(gs.spec.annotations.first(), Some(GraphAnnotation::ModelEquation { mode: EquationDisplayMode::Substituted, .. })),
        "expected Substituted ModelEquation: {:?}",
        gs.spec.annotations
    );
    assert_eq!(gs.spec.annotations.len(), 1, "strictly ModelEquation only (no extra fit-stat or CLD callouts)");

    // Reopen Insert Model Equation -> click `equation.toggle_mode` -> commit Symbolic.
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.insertModelEquation" }))?;
    let toggle1 = act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "equation.toggle_mode" }))?;
    assert_eq!(toggle1["mode"], "symbolic");
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    let gs_sym = app.document().graph_sheet(&SheetId::try_new(&graph_id).unwrap()).unwrap();
    assert!(matches!(gs_sym.spec.annotations.first(), Some(GraphAnnotation::ModelEquation { mode: EquationDisplayMode::Symbolic, .. })));

    // Toggle back to Substituted via Format Graph (`format_graph.toggle_model_equation_mode`).
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.format.open" }))?;
    let toggle2 = act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "format_graph.toggle_model_equation_mode" }))?;
    assert_eq!(toggle2["mode"], "substituted");
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    let gs_back = app.document().graph_sheet(&SheetId::try_new(&graph_id).unwrap()).unwrap();
    assert!(matches!(gs_back.spec.annotations.first(), Some(GraphAnnotation::ModelEquation { mode: EquationDisplayMode::Substituted, .. })));
    Ok(())
}

#[test]
fn wf44_to_48_raincloud_bubble_pairwise_layout_and_export_controls() -> TestRes {
    let mut app = strict_gui_app()?;
    let col_id = create_data_table(&mut app, json!({ "title": "Raincloud Col", "kind": "column", "cols": 2, "rows": 5, "auto_family": true }))?;
    select_sheet(&mut app, &col_id)?;
    paste_grid(&mut app, 0, 0, 0, "10\t20\n12\t25\n14\t30\n11\t22\n15\t28")?;

    let an_res = {
        act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
        act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.one_way_anova" } }))?;
        app.document().result_sheets.last().unwrap().id.as_str().to_owned()
    };
    let graph_id = app.document().graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id.as_str() == col_id).unwrap().id.as_str().to_owned();
    select_sheet(&mut app, &graph_id)?;

    // Raincloud graph option + Format Graph half-violin toggle.
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "selected_option_id": "col.raincloud" } }))?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.format.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "half_violin_with_points": true, "palette": "okabe_ito" } }))?;

    // Pairwise Comparison Brackets dialog + auto_add.
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.pairwise.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "fields": { "result_sheet_id": an_res, "hide_non_significant": false } }))?;
    let added = act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "pairwise.auto_add" }))?;
    assert!(added["added"].as_u64().unwrap_or(0) >= 1, "expected auto_add brackets: {added}");
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok" }))?;
    assert!(!app.document().graph_sheet(&SheetId::try_new(&graph_id).unwrap()).unwrap().spec.comparisons.is_empty());

    // Bubble Plot 5-channel inspector on MultipleVariables graph.
    let mv_id = create_data_table(
        &mut app,
        json!({
            "title": "Bubble Data",
            "kind": "multiple_variables",
            "cols": 4,
            "rows": 4,
            "variables": [
                { "title": "X", "role": "continuous" },
                { "title": "Y", "role": "continuous" },
                { "title": "Size", "role": "continuous" },
                { "title": "Group", "role": "categorical" }
            ],
            "auto_family": true
        }),
    )?;
    let mv_graph = app.document().graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id.as_str() == mv_id).unwrap().id.as_str().to_owned();
    select_sheet(&mut app, &mv_graph)?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open" }))?;
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "plot_mode": "bubble_scatter",
                "x_column": 0,
                "y_column": 1,
                "size_column": 2,
                "color_mode": "categorical",
                "color_column": 3,
                "show_convex_hulls": true,
                "show_confidence_ellipses": true
            }
        }),
    )?;
    let mv_gs = app.document().graph_sheet(&SheetId::try_new(&mv_graph).unwrap()).unwrap();
    let mv_enc = mv_gs.spec.multivariate.as_ref().expect("multivariate encoding");
    assert_eq!(mv_enc.plot_mode, MultivariatePlotMode::BubbleScatter);
    assert!(mv_enc.show_convex_hulls && mv_enc.show_confidence_ellipses);

    // Export figure dialog (`svg`).
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.export.figure.open" }))?;
    let svg_out = act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "format": "svg" } }))?;
    assert!(svg_out["payload"]["svg"].as_str().unwrap_or_default().contains("<svg"));

    // Page Layout Composer dialog on a fresh layout sheet.
    let lay_out = act(&mut app, "ui.invoke_menu", json!({ "command_id": "layout.create", "params": { "title": "Fig Layout" } }))?;
    let lay_id = lay_out["payload"]["sheet_id"].as_str().unwrap().to_owned();
    select_sheet(&mut app, &lay_id)?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.layout.composer.open" }))?;
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "preset": "double_column_174mm",
                "rows": 1,
                "cols": 2,
                "arrange_graph_ids": [graph_id, mv_graph],
                "align_plot_origins": true
            }
        }),
    )?;
    let ls = app.document().layout_sheet(&SheetId::try_new(&lay_id).unwrap()).unwrap();
    assert_eq!(ls.spec.items.len(), 2);
    Ok(())
}

#[test]
fn w22c_formula_bar_cycle_guard_and_calculated_cell_readonly() -> TestRes {
    let mut app = strict_gui_app()?;
    let mv_id = create_data_table(
        &mut app,
        json!({
            "title": "Formula Cohort",
            "kind": "multiple_variables",
            "cols": 3,
            "rows": 3,
            "variables": [
                { "title": "A", "role": "continuous" },
                { "title": "B", "role": "continuous" },
                { "title": "C", "role": "continuous" }
            ],
            "auto_family": false
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid(&mut app, 0, 0, 0, "1\t10\t0\n2\t20\t0\n3\t30\t0")?;

    let grid_snap = act(&mut app, "ui.inspect", json!({ "scope": "grid" }))?;
    for wid in [
        "formula_bar.open_col.0",
        "formula_bar.open_col.1",
        "formula_bar.open_col.2",
        "formula_bar.input",
        "formula_bar.commit",
        "formula_bar.cancel",
    ] {
        if find_widget(&grid_snap, wid).is_none() {
            return Err(format!("missing formula bar widget `{wid}` in grid inspect"));
        }
    }
    let commit_initial = find_widget(&grid_snap, "formula_bar.commit").ok_or_else(|| String::from("missing commit"))?;
    if commit_initial.get("enabled").and_then(Value::as_bool) != Some(false) {
        return Err(format!("expected inactive formula_bar.commit to be disabled: {commit_initial}"));
    }

    // Focus column C via pointer click on `formula_bar.open_col.2`, type self-cycle, check live parse_error & disabled commit.
    act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "formula_bar.open_col.2" }))?;
    let cyclic = act(&mut app, "ui.type_text", json!({ "widget_id": "formula_bar.input", "text": "= [C] + 1" }))?;
    if cyclic.get("is_valid").and_then(Value::as_bool) != Some(false)
        || cyclic.get("parse_error").and_then(Value::as_str).unwrap_or_default().is_empty()
    {
        return Err(format!("expected self-cycle to fail live validation: {cyclic}"));
    }
    let cyc_snap = act(&mut app, "ui.inspect", json!({ "scope": "grid" }))?;
    let cyc_commit = find_widget(&cyc_snap, "formula_bar.commit").ok_or_else(|| String::from("missing commit"))?;
    if cyc_commit.get("enabled").and_then(Value::as_bool) != Some(false) {
        return Err(format!("expected cyclic formula_bar.commit to be disabled: {cyc_commit}"));
    }
    if dispatch_gui_action(&mut app, "ui.press_key", json!({ "key": "Enter" })).is_ok() {
        return Err(String::from("expected Enter on cyclic formula to return an error"));
    }
    if dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "formula_bar.commit" })).is_ok() {
        return Err(String::from("expected clicking disabled formula_bar.commit to return an error"));
    }

    // Escape closes the formula bar without committing.
    act(&mut app, "ui.press_key", json!({ "key": "Escape" }))?;
    if app.grid.formula_bar.active {
        return Err(String::from("expected Escape to cancel/close formula bar"));
    }

    // Reopen via ui.table.openFormulaBar with column + title, type valid formula, commit via Enter.
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.table.openFormulaBar", "params": { "column": 1, "title": "DoubleA" } }))?;
    act(&mut app, "ui.type_text", json!({ "widget_id": "formula_bar.input", "text": "= [A] * 2" }))?;
    act(&mut app, "ui.press_key", json!({ "key": "Enter" }))?;

    // Open col 2 via pointer click, type valid chained formula referencing DoubleA, click formula_bar.commit.
    act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "formula_bar.open_col.2" }))?;
    let valid_snap_in = act(&mut app, "ui.type_text", json!({ "widget_id": "formula_bar.input", "text": "= [DoubleA] + 10" }))?;
    if valid_snap_in.get("is_valid").and_then(Value::as_bool) != Some(true) {
        return Err(format!("expected valid formula before click commit: {valid_snap_in}"));
    }
    act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "formula_bar.commit" }))?;

    let ds = app.document().data_sheet(&SheetId::try_new(&mv_id).map_err(|e| e.to_string())?).ok_or_else(|| String::from("missing mv sheet"))?;
    let c0 = ds.archetype.data_cell(CellCoord::new(0, 1, 0)).map(|c| &c.value);
    let c2 = ds.archetype.data_cell(CellCoord::new(0, 2, 0)).map(|c| &c.value);
    if !matches!(c0, Some(CellValue::Number(v)) if (*v - 2.0).abs() < 1e-9) || !matches!(c2, Some(CellValue::Number(v)) if (*v - 12.0).abs() < 1e-9) {
        return Err(format!("unexpected calculated values DoubleA[0]={c0:?}, C[0]={c2:?}"));
    }

    // Calculated MV cells must report enabled=false with disabled_reason and reject direct writes.
    let after_snap = act(&mut app, "ui.inspect", json!({ "scope": "grid" }))?;
    let calc_cell = find_widget(&after_snap, "grid.cell.0.1.0").ok_or_else(|| String::from("missing calculated grid.cell.0.1.0"))?;
    if calc_cell.get("enabled").and_then(Value::as_bool) != Some(false)
        || calc_cell.get("disabled_reason").and_then(Value::as_str) != Some("Calculated variable cells are formula-derived (read-only).")
    {
        return Err(format!("unexpected calculated cell inspect state: {calc_cell}"));
    }
    if dispatch_gui_action(&mut app, "ui.type_text", json!({ "widget_id": "grid.cell.0.1.0", "text": "999" })).is_ok() {
        return Err(String::from("expected typing into calculated variable cell to be rejected"));
    }
    if dispatch_gui_action(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.0.1.0" })).is_ok() {
        return Err(String::from("expected clicking disabled calculated variable cell to be rejected"));
    }
    Ok(())
}

#[test]
fn w22c_transform_baseline_select_archetype_and_km_modals() -> TestRes {
    let mut app = strict_gui_app()?;
    let xy_id = create_data_table(&mut app, json!({ "title": "Baseline XY", "kind": "xy", "cols": 2, "rows": 4, "auto_family": false }))?;
    select_sheet(&mut app, &xy_id)?;
    paste_x(&mut app, 0, 0, "0")?;
    paste_x(&mut app, 1, 0, "1")?;
    paste_x(&mut app, 2, 0, "2")?;
    paste_x(&mut app, 3, 0, "3")?;
    paste_grid(&mut app, 0, 0, 0, "10\t20\n12\t22\n14\t24\n16\t26")?;

    // RemoveBaseline in Analyze + EditAnalysisParameters
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "transform.baseline" } }))?;
    let base_dlg = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    for wid in ["analyze.transform.baseline_columns", "analyze.transform.baseline_operation", "analyze.transform.selected_data_sets"] {
        if find_widget(&base_dlg, wid).is_none() {
            return Err(format!("missing RemoveBaseline analyze control `{wid}`"));
        }
    }
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": { "baseline_columns": [0], "baseline_operation": "Difference", "selected_data_sets": [1] }
        }),
    )?;
    let base_rs_id = app.document().result_sheets.last().ok_or_else(|| String::from("missing baseline result"))?.id.clone();
    let base_rs = app.document().result_sheet(&base_rs_id).ok_or_else(|| String::from("missing base rs"))?;
    if !matches!(
        base_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { ref baseline_columns, operation: BaselineOperation::Difference })
            if baseline_columns == &[0]
    ) || base_rs.analysis.selected_data_sets != vec![1]
    {
        return Err(format!("unexpected initial RemoveBaseline spec: {:?}", base_rs.analysis));
    }

    select_sheet(&mut app, base_rs_id.as_str())?;
    act(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    let base_edit_dlg = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    for wid in ["edit.transform.baseline_columns", "edit.transform.baseline_operation", "edit.transform.selected_data_sets"] {
        if find_widget(&base_edit_dlg, wid).is_none() {
            return Err(format!("missing RemoveBaseline edit control `{wid}`"));
        }
    }
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": { "edit.transform.baseline_operation": "PercentChange", "edit.transform.selected_data_sets": [0, 1] }
        }),
    )?;
    let base_rs_after = app.document().result_sheet(&base_rs_id).ok_or_else(|| String::from("missing edited base rs"))?;
    if !matches!(
        base_rs_after.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { operation: BaselineOperation::PercentChange, .. })
    ) || base_rs_after.analysis.selected_data_sets != vec![0, 1]
    {
        return Err(format!("unexpected edited RemoveBaseline spec: {:?}", base_rs_after.analysis));
    }

    // MultipleVariables table for SelectAndTransform (target_archetype != Column), ConvertArchetype(Contingency), and Initial KM.
    let mv_id = create_data_table(
        &mut app,
        json!({
            "title": "Select & KM MV",
            "kind": "multiple_variables",
            "cols": 3,
            "rows": 6,
            "variables": [
                { "title": "Time", "role": "continuous" },
                { "title": "Event", "role": "continuous" },
                { "title": "Arm", "role": "categorical" }
            ],
            "auto_family": false
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid(&mut app, 0, 0, 0, "5\t1\tA\n10\t1\tA\n15\t0\tA\n7\t1\tB\n14\t1\tB\n20\t1\tB")?;

    // SelectAndTransform with non-Column target_archetype + filter_formula + selected_data_sets
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "transform.select" } }))?;
    let sel_dlg = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    for wid in ["analyze.transform.target_archetype", "analyze.transform.filter_formula", "analyze.transform.selected_data_sets"] {
        if find_widget(&sel_dlg, wid).is_none() {
            return Err(format!("missing SelectAndTransform analyze control `{wid}`"));
        }
    }
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": { "target_archetype": "Grouped", "filter_formula": "[Time] >= 7", "selected_data_sets": [0, 1] }
        }),
    )?;
    let sel_rs_id = app.document().result_sheets.last().ok_or_else(|| String::from("missing select result"))?.id.clone();
    let sel_rs = app.document().result_sheet(&sel_rs_id).ok_or_else(|| String::from("missing select rs"))?;
    if !matches!(
        sel_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { ref filter_formula, target_archetype: TableKind::Grouped })
            if filter_formula.as_deref() == Some("[Time] >= 7")
    ) || sel_rs.analysis.selected_data_sets != vec![0, 1]
    {
        return Err(format!("unexpected initial SelectAndTransform spec: {:?}", sel_rs.analysis));
    }

    select_sheet(&mut app, sel_rs_id.as_str())?;
    act(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    let sel_edit_dlg = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    for wid in ["edit.transform.target_archetype", "edit.transform.filter_formula", "edit.transform.selected_data_sets"] {
        if find_widget(&sel_edit_dlg, wid).is_none() {
            return Err(format!("missing SelectAndTransform edit control `{wid}`"));
        }
    }
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "edit.transform.target_archetype": "Xy",
                "edit.transform.filter_formula": "[Time] >= 10",
                "edit.transform.selected_data_sets": [0]
            }
        }),
    )?;
    let sel_rs_after = app.document().result_sheet(&sel_rs_id).ok_or_else(|| String::from("missing edited select rs"))?;
    if !matches!(
        sel_rs_after.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { ref filter_formula, target_archetype: TableKind::Xy })
            if filter_formula.as_deref() == Some("[Time] >= 10")
    ) || sel_rs_after.analysis.selected_data_sets != vec![0]
    {
        return Err(format!("unexpected edited SelectAndTransform spec: {:?}", sel_rs_after.analysis));
    }

    // Initial Kaplan-Meier Analyze controls & patcher
    select_sheet(&mut app, &mv_id)?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.survival_km" } }))?;
    let km_dlg = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    for wid in [
        "analyze.km.mv_time_column",
        "analyze.km.mv_status_column",
        "analyze.km.mv_group_column",
        "analyze.km.log_rank_mantel_cox",
        "analyze.km.gehan_breslow_wilcoxon",
        "analyze.km.log_rank_for_trend",
        "analyze.km.multiplicity_method",
    ] {
        if find_widget(&km_dlg, wid).is_none() {
            return Err(format!("missing initial KM control `{wid}`"));
        }
    }
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "mv_time_column": 0,
                "mv_status_column": 1,
                "mv_group_column": 2,
                "log_rank_mantel_cox": true,
                "gehan_breslow_wilcoxon": true,
                "log_rank_for_trend": true,
                "multiplicity_method": "HolmSidak"
            }
        }),
    )?;
    let km_rs_id = app.document().result_sheets.last().ok_or_else(|| String::from("missing km result"))?.id.clone();
    let km_rs = app.document().result_sheet(&km_rs_id).ok_or_else(|| String::from("missing km rs"))?;
    if !matches!(
        km_rs.analysis.kind,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: true,
            trend_test: true,
            mv_time_column: Some(0),
            mv_status_column: Some(1),
            mv_group_column: Some(2),
            ref multiplicity,
            ..
        }) if multiplicity.method == MultiplicityMethod::HolmSidak
    ) {
        return Err(format!("unexpected initial KM spec: {:?}", km_rs.analysis));
    }

    select_sheet(&mut app, km_rs_id.as_str())?;
    act(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": { "meta": true } }))?;
    let km_edit_dlg = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    for wid in [
        "edit.km.mv_time_column",
        "edit.km.mv_status_column",
        "edit.km.mv_group_column",
        "edit.km.log_rank_mantel_cox",
        "edit.km.gehan_breslow_wilcoxon",
        "edit.km.trend_test",
        "edit.km.log_rank_for_trend",
        "edit.km.multiplicity_method",
    ] {
        if find_widget(&km_edit_dlg, wid).is_none() {
            return Err(format!("missing KM edit control `{wid}`"));
        }
    }
    act(&mut app, "ui.answer_dialog", json!({ "action": "cancel" }))?;

    // ConvertArchetype -> Contingency target
    let src_id = create_data_table(&mut app, json!({ "title": "Convert Src", "kind": "column", "cols": 2, "rows": 2, "auto_family": false }))?;
    select_sheet(&mut app, &src_id)?;
    paste_grid(&mut app, 0, 0, 0, "12\t18\n25\t15")?;
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.table.convertArchetype.open", "params": { "sheet_id": src_id } }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "target_kind": "Contingency" } }))?;
    let converted = app.document().data_sheet(&SheetId::try_new(&src_id).map_err(|e| e.to_string())?).ok_or_else(|| String::from("missing src"))?;
    if converted.kind() != TableKind::Contingency {
        return Err(format!("expected converted TableKind::Contingency, got {:?}", converted.kind()));
    }
    Ok(())
}

#[test]
fn w22c_diagnostic_graph_thresholds_and_bubble_inspect() -> TestRes {
    let mut app = strict_gui_app()?;
    let mv_id = create_data_table(
        &mut app,
        json!({
            "title": "Bubble & Diag MV",
            "kind": "multiple_variables",
            "cols": 4,
            "rows": 6,
            "variables": [
                { "title": "F1", "role": "continuous" },
                { "title": "F2", "role": "continuous" },
                { "title": "F3", "role": "continuous" },
                { "title": "Arm", "role": "categorical" }
            ],
            "auto_family": true
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid(
        &mut app,
        0,
        0,
        0,
        "1.0\t1.0\t2.0\tC1\n1.1\t0.9\t2.1\tC1\n0.9\t1.1\t1.9\tC1\n10.0\t10.0\t8.0\tC2\n10.2\t9.8\t8.2\tC2\n9.9\t10.1\t7.9\tC2",
    )?;
    let mv_sid = SheetId::try_new(&mv_id).map_err(|e| e.to_string())?;
    let mv_graph = app
        .document()
        .graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id == mv_sid)
        .ok_or_else(|| String::from("missing MV graph"))?
        .id
        .as_str()
        .to_owned();
    select_sheet(&mut app, &mv_graph)?;

    // BubblePlotInspector controls in dialog inspect scope
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.graph.bubblePlot.open" }))?;
    let bubble_dlg = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    for wid in [
        "bubble.plot_mode",
        "bubble.x_column",
        "bubble.y_column",
        "bubble.color_channel",
        "bubble.size_channel",
        "bubble.label_column",
        "bubble.color_mode",
        "bubble.color_column",
        "bubble.size_column",
        "bubble.available_columns",
    ] {
        if find_widget(&bubble_dlg, wid).is_none() {
            return Err(format!("missing BubblePlotInspector control `{wid}`"));
        }
    }
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "color_mode": "categorical", "color_column": 3, "size_column": 2 } }))?;

    // FormatAxes opened via `toolbar.axes` toolbar click + y_thresholds/x_thresholds inspection & commit.
    act(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "toolbar.axes" }))?;
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }) {
        return Err(format!("expected toolbar.axes click to open FormatAxes, got {:?}", app.canvas_dialogs.active_dialog));
    }
    let axes_dlg = act(&mut app, "ui.inspect", json!({ "scope": "dialog" }))?;
    for wid in ["axes.thresholds", "axes.x_thresholds", "axes.y_thresholds"] {
        if find_widget(&axes_dlg, wid).is_none() {
            return Err(format!("missing FormatAxes control `{wid}`"));
        }
    }
    act(
        &mut app,
        "ui.answer_dialog",
        json!({
            "action": "ok",
            "fields": {
                "y_thresholds": [{ "value": 5.0, "label": "Mid", "dash_style": "dashed" }],
                "x_thresholds": [{ "value": 2.5, "shade_to": 7.5, "label": "Band" }]
            }
        }),
    )?;
    let gs = app.document().graph_sheet(&SheetId::try_new(&mv_graph).map_err(|e| e.to_string())?).ok_or_else(|| String::from("missing graph"))?;
    if gs.spec.y_axis.thresholds.len() != 1
        || (gs.spec.y_axis.thresholds[0].value - 5.0).abs() > 1e-9
        || gs.spec.x_axis.thresholds.len() != 1
        || gs.spec.x_axis.thresholds[0].shade_to != Some(7.5)
    {
        return Err(format!("unexpected axis thresholds persisted: x={:?}, y={:?}", gs.spec.x_axis.thresholds, gs.spec.y_axis.thresholds));
    }

    // Diagnostic graph type switches (`diag.residual` and `diag.normal_qq`) via `graph.setKind`.
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "selected_option_id": "diag.residual" } }))?;
    let gs_res = app.document().graph_sheet(&SheetId::try_new(&mv_graph).map_err(|e| e.to_string())?).ok_or_else(|| String::from("missing graph"))?;
    if gs_res.spec.kind != GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot) {
        return Err(format!("expected DiagnosticPlotKind::ResidualPlot, got {:?}", gs_res.spec.kind));
    }
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "selected_option_id": "diag.normal_qq" } }))?;
    let gs_qq = app.document().graph_sheet(&SheetId::try_new(&mv_graph).map_err(|e| e.to_string())?).ok_or_else(|| String::from("missing graph"))?;
    if gs_qq.spec.kind != GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot) {
        return Err(format!("expected DiagnosticPlotKind::NormalQqPlot, got {:?}", gs_qq.spec.kind));
    }
    Ok(())
}

#[test]
fn w22c_cluster_append_column_ui() -> TestRes {
    let mut app = strict_gui_app()?;
    let mv_id = create_data_table(
        &mut app,
        json!({
            "title": "Cluster MV",
            "kind": "multiple_variables",
            "cols": 3,
            "rows": 6,
            "variables": [
                { "title": "F1", "role": "continuous" },
                { "title": "F2", "role": "continuous" },
                { "title": "F3", "role": "continuous" }
            ],
            "auto_family": true
        }),
    )?;
    select_sheet(&mut app, &mv_id)?;
    paste_grid(&mut app, 0, 0, 0, "1.0\t1.0\t2.0\n1.1\t0.9\t2.1\n0.9\t1.1\t1.9\n10.0\t10.0\t8.0\n10.2\t9.8\t8.2\n9.9\t10.1\t7.9")?;

    // K-means Clustering + ui.result.appendClusterColumn
    act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open" }))?;
    act(&mut app, "ui.answer_dialog", json!({ "action": "ok", "fields": { "entry_id": "stats.clustering", "k_means_k": 2 } }))?;
    let cl_rs_id = app.document().result_sheets.last().ok_or_else(|| String::from("missing cluster result"))?.id.clone();
    select_sheet(&mut app, cl_rs_id.as_str())?;

    let append_out = act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.result.appendClusterColumn" }))?;
    if append_out.pointer("/result/updated_existing").and_then(Value::as_bool) != Some(false) {
        return Err(format!("expected initial appendClusterColumn to create new column: {append_out}"));
    }
    let mv_sid = SheetId::try_new(&mv_id).map_err(|e| e.to_string())?;
    let ds = app.document().data_sheet(&mv_sid).ok_or_else(|| String::from("missing mv sheet after cluster append"))?;
    let TableArchetype::MultipleVariables(ref mv_tbl) = ds.archetype else {
        return Err(String::from("expected MV table"));
    };
    if mv_tbl.variables.len() != 4 || mv_tbl.variables[3].title != "Cluster" || mv_tbl.variables[3].role != VariableRole::Categorical {
        return Err(format!("unexpected appended Cluster column: {:?}", mv_tbl.variables.get(3)));
    }
    let first_cluster = ds.archetype.data_cell(CellCoord::new(0, 3, 0)).map(|c| &c.value);
    if !matches!(first_cluster, Some(CellValue::Text(s)) if s.starts_with("Cluster ")) {
        return Err(format!("expected Cluster k assignment string, got {first_cluster:?}"));
    }
    // Re-running updates the existing column in place.
    let update_out = act(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.result.appendClusterColumn" }))?;
    if update_out.pointer("/result/updated_existing").and_then(Value::as_bool) != Some(true) {
        return Err(format!("expected repeat appendClusterColumn to update existing column: {update_out}"));
    }
    Ok(())
}
