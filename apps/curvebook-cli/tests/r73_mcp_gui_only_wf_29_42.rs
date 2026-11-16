mod r73_common;

use curvebook_svg::validate_svg_structure;
use r73_common::{
    McpChild, TempWorkdir, assert_approx, find_node, read_result_json, result_scalar_f64, result_tab, save_and_verify_headless, tab_cell_f64,
};
use serde_json::{Value, json};

fn mv_cell_f64(grid_snap: &Value, row: usize, col: usize) -> Result<f64, String> {
    let wid = format!("grid.cell.{row}.{col}.0");
    let node = find_node(&grid_snap["grid_cells"], &wid).ok_or_else(|| format!("missing `{wid}` in grid_cells"))?;
    let text = node["value"]["text"].as_str().ok_or_else(|| format!("non-text value at `{wid}`: {node}"))?;
    text.trim().parse::<f64>().map_err(|e| format!("parse float `{text}` at `{wid}`: {e}"))
}

fn set_formula_via_bar(gui: &mut McpChild, col: usize, formula: &str) -> Result<(), String> {
    gui.ui_invoke("ui.table.openFormulaBar", json!({ "column": col }))?;
    let typed = gui.ui_type(Some("formula_bar.input"), formula, true, false)?;
    if typed["is_valid"] != json!(true) {
        return Err(format!("formula `{formula}` marked invalid: {typed}"));
    }
    gui.ui_pointer(json!({ "action": "click", "widget_id": "formula_bar.commit" }))?;
    Ok(())
}

#[test]
fn r73_gui_only_wf29_to_wf32_mv_transforms_formulas_chaining_welch() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf29_32")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-29: Select & Transform -> ConcentrationToNegativeLogM chain -> Row Stats -> Baseline subtraction -> Live edit
    let mv29 = gui.create_table(json!({
        "sheet_id": "wf29-mv",
        "kind": "multiple_variables",
        "rows": 8,
        "auto_graph": true,
        "variables": [
            { "title": "SampleID", "role": "label" },
            { "title": "Batch", "role": "categorical" },
            { "title": "Condition", "role": "categorical" },
            { "title": "Conc_uM", "role": "continuous" },
            { "title": "Signal_T0", "role": "continuous" },
            { "title": "Signal_T1", "role": "continuous" },
            { "title": "Signal_T2", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv29,
        0,
        0,
        0,
        "S1\tA\tControl\t0.01\t100.0\t105.0\t110.0\n\
         S2\tA\tControl\t0.1\t102.0\t108.0\t112.0\n\
         S3\tB\tControl\t1.0\t98.0\t101.0\t104.0\n\
         S4\tB\tControl\t10.0\t100.0\t103.0\t107.0\n\
         S5\tA\tTreated\t0.01\t100.0\t120.0\t150.0\n\
         S6\tA\tTreated\t0.1\t100.0\t140.0\t190.0\n\
         S7\tB\tTreated\t1.0\t100.0\t160.0\t230.0\n\
         S8\tB\tTreated\t10.0\t100.0\t180.0\t270.0",
    )?;
    let sel29 = gui.run_analyze(
        &mv29,
        json!({
            "entry_id": "transform.select",
            "selected_data_sets": [3, 4, 5, 6],
            "target_archetype": "MultipleVariables",
            "filter_formula": "[Condition] == \"Treated\" && [Conc_uM] >= 0.1"
        }),
    )?;
    let math29 = gui.run_analyze(
        &sel29,
        json!({
            "entry_id": "transform.math",
            "selected_data_sets": [0],
            "x_transform": null,
            "y_transform": { "ConcentrationToNegativeLogM": { "molar_unit_factor": 1e-6 } }
        }),
    )?;
    let stats29 = gui.run_analyze(&mv29, json!({ "entry_id": "stats.descriptive", "selected_data_sets": [4, 5, 6] }))?;
    let base29 = gui.run_analyze(
        &mv29,
        json!({
            "entry_id": "transform.baseline",
            "analyze.transform.selected_data_sets": [4, 5, 6],
            "analyze.transform.baseline_columns": [4],
            "analyze.transform.baseline_operation": "Difference"
        }),
    )?;
    let mv29_graph = gui.graph_for_source(&mv29)?;
    gui.ui_switch_sheet(&mv29_graph)?;
    gui.ui_answer_ok(json!({ "selected_option_id": "mv.bubble" }))?;
    let _ = gui.ui_screenshot("canvas", 640, 400)?;
    gui.set_cell(&mv29, 7, 6, 0, "300.0")?;

    // WF-30: Formula Bar Calculated Variables + Syntax/Cycle/Read-Only guards + Live Cascade
    let mv30 = gui.create_table(json!({
        "sheet_id": "wf30-mv",
        "kind": "multiple_variables",
        "rows": 6,
        "variables": [
            { "title": "SampleID", "role": "label" },
            { "title": "Group", "role": "categorical" },
            { "title": "RawSignal", "role": "continuous" },
            { "title": "RefControl", "role": "continuous" },
            { "title": "Delta", "role": "continuous" },
            { "title": "Fold", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv30,
        0,
        0,
        0,
        "C1\tCtrl\t10.0\t10.0\nC2\tCtrl\t12.0\t10.0\nC3\tCtrl\t8.0\t10.0\nD1\tDrug\t6.0\t10.0\nD2\tDrug\t7.0\t10.0\nD3\tDrug\t5.0\t10.0",
    )?;
    set_formula_via_bar(&mut gui, 4, "= [RawSignal] - [RefControl]")?;
    let grid30_a = gui.ui_inspect("grid", false)?;
    assert_approx(mv_cell_f64(&grid30_a, 3, 4)?, -4.0, 1e-6, "WF-30 Delta D1")?;

    gui.ui_pointer(json!({ "action": "click", "widget_id": "formula_bar.open_col.5" }))?;
    let bad_syn = gui.ui_type(Some("formula_bar.input"), "= [RawSignal +", true, false)?;
    if bad_syn["is_valid"] != json!(false) {
        return Err(format!("WF-30 invalid syntax expected is_valid=false: {bad_syn}"));
    }
    let _ = gui.ui_pointer_err(json!({ "action": "click", "widget_id": "formula_bar.commit" }))?;
    let self_cyc = gui.ui_type(Some("formula_bar.input"), "= [Fold] + 1", true, false)?;
    if self_cyc["is_valid"] != json!(false) {
        return Err("WF-30 self-cycle expected is_valid=false".to_owned());
    }
    let _ = gui.ui_press_err("Enter", json!({}))?;
    let good_fold = gui.ui_type(Some("formula_bar.input"), "= IF([Group] == \"Drug\", 2 ^ (-[Delta]), 1.0)", true, false)?;
    if good_fold["is_valid"] != json!(true) {
        return Err(format!("WF-30 valid IF formula rejected: {good_fold}"));
    }
    gui.ui_pointer(json!({ "action": "click", "widget_id": "formula_bar.commit" }))?;
    gui.ui_invoke("ui.table.openFormulaBar", json!({ "column": 4 }))?;
    let multi_cyc = gui.ui_type(Some("formula_bar.input"), "= [Fold] * 2", true, false)?;
    if multi_cyc["is_valid"] != json!(false) {
        return Err("WF-30 multi-hop cycle expected is_valid=false".to_owned());
    }
    gui.ui_press("Escape", json!({}))?;

    let grid30_ro = gui.ui_inspect("grid", false)?;
    if find_node(&grid30_ro["grid_cells"], "grid.cell.0.4.0").and_then(|c| c["enabled"].as_bool()) != Some(false) {
        return Err("WF-30 calculated cell should report enabled=false".to_owned());
    }
    let _ = gui.ui_type_err(Some("grid.cell.0.4.0"), "99")?;
    gui.set_cell(&mv30, 3, 2, 0, "8.0")?;
    let grid30_b = gui.ui_inspect("grid", false)?;
    assert_approx(mv_cell_f64(&grid30_b, 3, 5)?, 4.0, 1e-6, "WF-30 Fold D1 cascade")?;

    // WF-31: Extract & Rearrange cross-schema chaining (MV -> XY -> 4PL, MV -> Column -> Desc, MV -> Contingency)
    let mv31 = gui.create_table(json!({
        "sheet_id": "wf31-mv",
        "kind": "multiple_variables",
        "rows": 12,
        "variables": [
            { "title": "SampleID", "role": "label" },
            { "title": "Genotype", "role": "categorical" },
            { "title": "Phenotype", "role": "categorical" },
            { "title": "LogConc", "role": "continuous" },
            { "title": "Viability", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv31,
        0,
        0,
        0,
        "W1\tWT\tResponder\t-9\t98\nW2\tWT\tResponder\t-8\t92\nW3\tWT\tResponder\t-7\t75\nW4\tWT\tNonResponder\t-6\t48\nW5\tWT\tNonResponder\t-5\t20\nW6\tWT\tNonResponder\t-4\t6\nM1\tMut\tResponder\t-9\t99\nM2\tMut\tResponder\t-8\t96\nM3\tMut\tResponder\t-7\t94\nM4\tMut\tResponder\t-6\t88\nM5\tMut\tNonResponder\t-5\t52\nM6\tMut\tNonResponder\t-4\t15",
    )?;
    let xy31 = gui.run_analyze(
        &mv31,
        json!({ "entry_id": "transform.select", "selected_data_sets": [3, 4], "target_archetype": "Xy", "filter_formula": "[Genotype] == \"WT\"" }),
    )?;
    let fit31 =
        gui.run_analyze(&xy31, json!({ "entry_id": "fit.nonlinear", "equation_id": "log_inhibitor_vs_response_4pl", "selected_data_sets": [0] }))?;

    // WF-32: Direct MV Welch t-test + effect sizes + Categorical Summary plot + Cmd+Shift+E exclusion
    let mv32 = gui.create_table(json!({
        "sheet_id": "wf32-mv",
        "kind": "multiple_variables",
        "rows": 12,
        "auto_graph": true,
        "variables": [
            { "title": "SampleID", "role": "label" },
            { "title": "Treatment", "role": "categorical" },
            { "title": "Biomarker", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv32,
        0,
        0,
        0,
        "V1\tVehicle\t18.5\nV2\tVehicle\t20.2\nV3\tVehicle\t21.0\nV4\tVehicle\t22.4\nV5\tVehicle\t19.8\nV6\tVehicle\t23.1\nT1\tTreated\t31.0\nT2\tTreated\t34.5\nT3\tTreated\t42.0\nT4\tTreated\t38.2\nT5\tTreated\t46.8\nT6\tTreated\t52.0",
    )?;
    let welch32 = gui.run_analyze(
        &mv32,
        json!({ "entry_id": "stats.two_group", "two_group_method": "welch", "mv_group_column": 1, "mv_response_column": 2, "compute_effect_sizes": true }),
    )?;
    let mv32_graph = gui.graph_for_source(&mv32)?;
    gui.ui_switch_sheet(&mv32_graph)?;
    gui.ui_invoke("ui.graph.bubblePlot.open", json!({}))?;
    gui.ui_answer_ok(json!({ "plot_mode": "categorical_summary", "x_column": 1, "y_column": 2, "color_mode": "categorical", "color_column": 1 }))?;
    let _ = gui.ui_screenshot("canvas", 640, 400)?;
    gui.exclude_cell_cmd_shift_e(&mv32, 11, 2, 0, "InstrumentError", "WF32 outlier")?;

    save_and_verify_headless(&mut gui, &wd.dir, "wf29_32.curvebook", |headless| {
        let _ = read_result_json(headless, &math29)?;
        let rs_stats = read_result_json(headless, &stats29)?;
        let rstats_tab = result_tab(&rs_stats, "row-descriptive-stats")?;
        let r7_mean = tab_cell_f64(rstats_tab, "Row 8", "Mean")?;
        assert_approx(r7_mean, (100.0 + 180.0 + 300.0) / 3.0, 1e-4, "WF-29 reactive row 8 mean")?;
        let _ = read_result_json(headless, &base29)?;
        let rs_fit31 = read_result_json(headless, &fit31)?;
        let _ = tab_cell_f64(result_tab(&rs_fit31, "nonlinear_parameters")?, "LogIC50", "Best-fit value")?;
        let rs_w32 = read_result_json(headless, &welch32)?;
        if result_scalar_f64(&rs_w32, "PValue")? >= 0.01 {
            return Err("WF-32 expected significant Welch PValue".to_owned());
        }
        Ok(())
    })?;
    Ok(())
}

#[test]
fn r73_gui_only_wf33_to_wf36_multifactor_qpcr_survival_and_4pl_mv() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf33_36")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-33: Direct MV Multifactor ANOVA (2-way -> Cmd+T 3-way Type III + QOL-7 + cell edit)
    let mv33 = gui.create_table(json!({
        "sheet_id": "wf33-mv",
        "kind": "multiple_variables",
        "rows": 24,
        "auto_graph": true,
        "variables": [
            { "title": "SampleID", "role": "label" },
            { "title": "Genotype", "role": "categorical" },
            { "title": "Drug", "role": "categorical" },
            { "title": "Timepoint", "role": "categorical" },
            { "title": "Response", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv33,
        0,
        0,
        0,
        "S01\tWT\tVehicle\tEarly\t10.0\nS02\tWT\tVehicle\tEarly\t11.2\nS03\tWT\tVehicle\tEarly\t10.5\nS04\tWT\tVehicle\tLate\t12.0\nS05\tWT\tVehicle\tLate\t12.8\nS06\tWT\tVehicle\tLate\t12.2\nS07\tWT\tDrugX\tEarly\t18.0\nS08\tWT\tDrugX\tEarly\t19.4\nS09\tWT\tDrugX\tEarly\t18.6\nS10\tWT\tDrugX\tLate\t26.0\nS11\tWT\tDrugX\tLate\t27.5\nS12\tWT\tDrugX\tLate\t26.8\nS13\tKO\tVehicle\tEarly\t11.0\nS14\tKO\tVehicle\tEarly\t10.8\nS15\tKO\tVehicle\tEarly\t11.4\nS16\tKO\tVehicle\tLate\t11.6\nS17\tKO\tVehicle\tLate\t12.1\nS18\tKO\tVehicle\tLate\t11.8\nS19\tKO\tDrugX\tEarly\t13.0\nS20\tKO\tDrugX\tEarly\t13.6\nS21\tKO\tDrugX\tEarly\t13.2\nS22\tKO\tDrugX\tLate\t14.2\nS23\tKO\tDrugX\tLate\t14.9\nS24\tKO\tDrugX\tLate\t14.5",
    )?;
    let anova33 = gui.run_analyze(
        &mv33,
        json!({ "entry_id": "stats.factorial_anova", "selected_data_sets": [1, 2], "response_column": 4, "three_way": false, "ss_type": "TypeIII" }),
    )?;
    gui.ui_switch_sheet(&anova33)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    gui.ui_answer_dialog(json!({ "fields": { "multifactor_columns": [1, 2, 3] } }))?;
    gui.ui_answer_ok(json!({ "response_column": 4, "three_way": true, "max_interaction_order": 3, "compute_effect_sizes": true }))?;
    let rem33 = gui.ui_invoke_err("ui.result.applyRemediation", json!({ "sheet_id": anova33 }))?;
    if !rem33["message"].as_str().unwrap_or("").contains("diagnostics_only_policy") {
        return Err(format!("WF-33 expected diagnostics_only_policy rejection: {rem33}"));
    }

    // WF-34: qPCR ddCt Calculated Variables pipeline + MV Welch t-test
    let mv34 = gui.create_table(json!({
        "sheet_id": "wf34-mv",
        "kind": "multiple_variables",
        "rows": 8,
        "auto_graph": true,
        "variables": [
            { "title": "SampleID", "role": "label" },
            { "title": "Condition", "role": "categorical" },
            { "title": "TargetCt", "role": "continuous" },
            { "title": "RefCt", "role": "continuous" },
            { "title": "DeltaCt", "role": "continuous" },
            { "title": "CalibratorMean", "role": "continuous" },
            { "title": "DeltaDeltaCt", "role": "continuous" },
            { "title": "FoldChange", "role": "continuous" },
            { "title": "Log2FC", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv34,
        0,
        0,
        0,
        "C1\tControl\t22.0\t18.0\nC2\tControl\t22.2\t18.1\nC3\tControl\t21.9\t17.9\nC4\tControl\t22.1\t18.2\nP1\tPerturbed\t19.0\t18.0\nP2\tPerturbed\t19.1\t18.2\nP3\tPerturbed\t18.9\t17.9\nP4\tPerturbed\t19.2\t18.1",
    )?;
    set_formula_via_bar(&mut gui, 4, "= [TargetCt] - [RefCt]")?;
    set_formula_via_bar(&mut gui, 5, "= MEANIF([Condition] == \"Control\", [DeltaCt])")?;
    set_formula_via_bar(&mut gui, 6, "= [DeltaCt] - [CalibratorMean]")?;
    set_formula_via_bar(&mut gui, 7, "= 2 ^ (-[DeltaDeltaCt])")?;
    set_formula_via_bar(&mut gui, 8, "= -[DeltaDeltaCt]")?;
    let welch34 = gui.run_analyze(
        &mv34,
        json!({ "entry_id": "stats.two_group", "two_group_method": "welch", "mv_group_column": 1, "mv_response_column": 8, "compute_effect_sizes": true }),
    )?;
    gui.set_cell(&mv34, 4, 2, 0, "21.0")?;
    let grid34 = gui.ui_inspect("grid", false)?;
    assert_approx(mv_cell_f64(&grid34, 4, 7)?, 2.0, 1e-4, "WF-34 P1 FoldChange cascade")?;

    // WF-35: Direct MV Kaplan-Meier survival
    let mv35 = gui.create_table(json!({
        "sheet_id": "wf35-mv",
        "kind": "multiple_variables",
        "rows": 12,
        "auto_graph": true,
        "variables": [
            { "title": "PatientID", "role": "label" },
            { "title": "Cohort", "role": "categorical" },
            { "title": "Days", "role": "continuous" },
            { "title": "Event", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv35,
        0,
        0,
        0,
        "P1\tPlacebo\t10\t1\nP2\tPlacebo\t14\t1\nP3\tPlacebo\t18\t1\nP4\tPlacebo\t22\t1\nL1\tLowDose\t20\t1\nL2\tLowDose\t28\t1\nL3\tLowDose\t35\t0\nL4\tLowDose\t42\t1\nH1\tHighDose\t40\t1\nH2\tHighDose\t56\t0\nH3\tHighDose\t64\t0\nH4\tHighDose\t80\t0",
    )?;
    let km35 = gui.run_analyze(
        &mv35,
        json!({
            "entry_id": "stats.survival_km",
            "analyze.km.mv_time_column": 2,
            "analyze.km.mv_status_column": 3,
            "analyze.km.mv_group_column": 1,
            "analyze.km.log_rank_mantel_cox": true,
            "analyze.km.multiplicity_method": "HolmSidak"
        }),
    )?;

    // WF-36: Direct MV 4PL + Constants + QOL-16 InsertModelEquation Symbolic/Substituted + Residual graph
    let mv36 = gui.create_table(json!({
        "sheet_id": "wf36-mv",
        "kind": "multiple_variables",
        "rows": 16,
        "auto_graph": true,
        "variables": [
            { "title": "Well", "role": "label" },
            { "title": "Compound", "role": "categorical" },
            { "title": "LogConcM", "role": "continuous" },
            { "title": "Response", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv36,
        0,
        0,
        0,
        "A1\tInhibitorA\t-10\t99\nA2\tInhibitorA\t-9\t98.5\nA3\tInhibitorA\t-8\t91\nA4\tInhibitorA\t-7\t50\nA5\tInhibitorA\t-6\t9.5\nA6\tInhibitorA\t-5\t1.5\nA7\tInhibitorA\t-4\t0.5\nA8\tInhibitorA\t-3\t0.2\nB1\tInhibitorB\t-10\t99.5\nB2\tInhibitorB\t-9\t99\nB3\tInhibitorB\t-8\t98\nB4\tInhibitorB\t-7\t96.5\nB5\tInhibitorB\t-6\t76\nB6\tInhibitorB\t-5\t24\nB7\tInhibitorB\t-4\t3\nB8\tInhibitorB\t-3\t0.8",
    )?;
    let fit36 = gui.run_analyze(
        &mv36,
        json!({ "entry_id": "fit.nonlinear", "equation_id": "log_inhibitor_vs_response_4pl", "mv_x_columns": [2], "mv_y_column": 3, "mv_group_columns": [1] }),
    )?;
    gui.edit_analysis_cmd_t(
        &fit36,
        json!({
            "profile_likelihood_ci": true,
            "emit_residuals_tab": true,
            "parameter_rules": [
                { "name": "Bottom", "scope": { "Constant": 0.0 } },
                { "name": "Top", "scope": { "Constant": 100.0 } }
            ]
        }),
    )?;
    let mv36_graph = gui.graph_for_source(&mv36)?;
    gui.ui_switch_sheet(&mv36_graph)?;
    gui.ui_invoke("ui.graph.insertModelEquation", json!({}))?;
    gui.ui_answer_ok(json!({ "result_sheet_id": fit36, "mode": "symbolic" }))?;
    gui.ui_invoke("ui.graph.insertModelEquation", json!({}))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "equation.toggle_mode" }))?;
    gui.ui_answer_ok(json!({}))?;
    gui.ui_invoke("ui.graph.format.open", json!({}))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "format_graph.toggle_model_equation_mode" }))?;
    gui.ui_answer_ok(json!({}))?;

    save_and_verify_headless(&mut gui, &wd.dir, "wf33_36.curvebook", |headless| {
        let rs33 = read_result_json(headless, &anova33)?;
        let _ = result_scalar_f64(&rs33, "Genotype × Drug × Timepoint: F").or_else(|_| result_scalar_f64(&rs33, "Genotype:Drug:Timepoint: F"))?;
        let rs34 = read_result_json(headless, &welch34)?;
        if !result_scalar_f64(&rs34, "PValue")?.is_finite() {
            return Err("WF-34 Welch PValue not finite".to_owned());
        }
        let rs35 = read_result_json(headless, &km35)?;
        let _ = result_tab(&rs35, "survival-pairwise")?;
        let rs36 = read_result_json(headless, &fit36)?;
        let p36 = result_tab(&rs36, "nonlinear_parameters")?;
        assert_approx(tab_cell_f64(p36, "LogIC50 [DS1]", "Best-fit value")?, -7.0, 0.25, "WF-36 InhibitorA LogIC50")?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn r73_gui_only_wf37_to_wf42_clustering_cox_and_glms() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf37_42")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-37: Hierarchical Clustering Euclidean/Ward -> Pearson/Complete heatmap switch
    let mv37 = gui.create_table(json!({
        "sheet_id": "wf37-mv",
        "kind": "multiple_variables",
        "rows": 5,
        "auto_graph": true,
        "graph_sheet_id": "wf37-graph",
        "variables": [
            { "title": "Sample", "role": "label" },
            { "title": "GeneA", "role": "continuous" },
            { "title": "GeneB", "role": "continuous" },
            { "title": "GeneC", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv37,
        0,
        0,
        0,
        "RowSmallPos\t1\t2\t3\nRowMid\t10\t15\t12\nRowSmallNeg\t3\t2\t1\nRowScaledPos\t100\t200\t300\nRowMidNeg\t12\t10\t8",
    )?;
    let clust37 = gui.run_analyze(
        &mv37,
        json!({ "entry_id": "stats.clustering", "selected_data_sets": [1, 2, 3], "distance": "euclidean", "linkage": "ward", "k_means_k": Value::Null }),
    )?;
    gui.ui_switch_sheet("wf37-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({}))?;
    gui.ui_answer_ok(json!({ "option_id": "diag.heatmap" }))?;
    let hm_euc = gui.ui_screenshot("canvas", 720, 480)?;
    gui.edit_analysis_cmd_t(&clust37, json!({ "edit.cluster.distance": "pearson", "edit.cluster.linkage": "complete" }))?;
    gui.ui_switch_sheet("wf37-graph")?;
    let hm_pearson = gui.ui_screenshot("canvas", 720, 480)?;
    if hm_euc == hm_pearson {
        return Err("WF-37 heatmap screenshot should differ after Euclidean/Ward -> Pearson/Complete switch".to_owned());
    }

    // WF-38: K-Means + Append Cluster Column + Ellipses/Hulls
    let mv38 = gui.create_table(json!({
        "sheet_id": "wf38-mv",
        "kind": "multiple_variables",
        "rows": 12,
        "auto_graph": true,
        "graph_sheet_id": "wf38-graph",
        "variables": [
            { "title": "CellId", "role": "label" },
            { "title": "MarkerA", "role": "continuous" },
            { "title": "MarkerB", "role": "continuous" },
            { "title": "MarkerC", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv38,
        0,
        0,
        0,
        "C01\t1.0\t1.1\t0.9\nC02\t1.2\t0.9\t1.1\nC03\t0.8\t1.2\t1.0\nC04\t1.1\t1.0\t1.2\nC05\t10.0\t10.2\t1.1\nC06\t10.3\t9.9\t0.9\nC07\t9.8\t10.1\t1.2\nC08\t10.1\t9.8\t1.0\nC09\t2.0\t11.0\t12.1\nC10\t2.2\t10.8\t11.9\nC11\t1.9\t11.2\t12.0\nC12\t2.1\t10.9\t12.2",
    )?;
    let km38 = gui
        .run_analyze(&mv38, json!({ "entry_id": "stats.clustering", "selected_data_sets": [1, 2, 3], "k_means_k": Value::Null, "k_sweep_max": 6 }))?;
    gui.edit_analysis_cmd_t(&km38, json!({ "edit.cluster.k_means_k": 4 }))?;
    gui.edit_analysis_cmd_t(&km38, json!({ "edit.cluster.k_means_k": 3 }))?;
    gui.ui_switch_sheet(&km38)?;
    let app38 = gui.ui_invoke("ui.result.appendClusterColumn", json!({}))?;
    if app38["result"]["updated_existing"] != json!(false) {
        return Err(format!("WF-38 expected new Cluster column: {app38}"));
    }
    gui.ui_switch_sheet("wf38-graph")?;
    gui.ui_invoke("ui.graph.bubblePlot.open", json!({}))?;
    gui.ui_answer_ok(json!({
        "plot_mode": "bubble_scatter",
        "x_column": 1,
        "y_column": 2,
        "color_mode": "categorical",
        "color_column": 4,
        "show_convex_hulls": true,
        "show_confidence_ellipses": true
    }))?;

    // WF-39: Cox Proportional Hazards (Efron vs Breslow ties + reference level flip)
    let mv39 = gui.create_table(json!({
        "sheet_id": "wf39-mv",
        "kind": "multiple_variables",
        "rows": 14,
        "auto_graph": true,
        "graph_sheet_id": "wf39-graph",
        "variables": [
            { "title": "Time", "role": "continuous" },
            { "title": "Status", "role": "continuous" },
            { "title": "Arm", "role": "categorical" },
            { "title": "Age", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv39,
        0,
        0,
        0,
        "2\t1\tPlacebo\t68\n4\t1\tPlacebo\t66\n4\t1\tPlacebo\t64\n6\t1\tPlacebo\t62\n8\t0\tPlacebo\t60\n4\t1\tLowDose\t65\n7\t1\tLowDose\t63\n7\t1\tLowDose\t59\n11\t0\tLowDose\t56\n12\t1\tLowDose\t61\n9\t0\tHighDose\t58\n12\t1\tHighDose\t64\n12\t0\tHighDose\t55\n16\t1\tHighDose\t62",
    )?;
    let cox39 = gui.run_analyze(
        &mv39,
        json!({ "entry_id": "stats.survival_cox", "cox_time_column": 0, "cox_status_column": 1, "cox_predictor_columns": [2, 3], "reference_levels": [[2, "Placebo"]], "tie_method": "efron" }),
    )?;
    gui.edit_analysis_cmd_t(&cox39, json!({ "edit.cox.tie_method": "breslow" }))?;
    gui.edit_analysis_cmd_t(&cox39, json!({ "edit.cox.tie_method": "efron", "edit.cox.reference_levels": [[2, "LowDose"]] }))?;
    gui.ui_switch_sheet("wf39-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({}))?;
    gui.ui_answer_ok(json!({ "option_id": "survival.km" }))?;

    // WF-40: Multiple Linear Regression (Type III ANOVA + VIF + Residual/QQ)
    let mv40 = gui.create_table(json!({
        "sheet_id": "wf40-mv",
        "kind": "multiple_variables",
        "rows": 14,
        "auto_graph": true,
        "graph_sheet_id": "wf40-graph",
        "variables": [
            { "title": "Biomass", "role": "continuous" },
            { "title": "Temp", "role": "continuous" },
            { "title": "Nutrients", "role": "continuous" },
            { "title": "Media", "role": "categorical" }
        ]
    }))?;
    gui.paste_tsv(
        &mv40,
        0,
        0,
        0,
        "10.1\t1.0\t2.0\tControl\n11.3\t2.0\t1.5\tControl\n16.8\t3.0\t4.0\tControl\n18.4\t4.0\t3.5\tControl\n23.9\t5.0\t6.0\tControl\n24.6\t6.0\t5.0\tControl\n31.7\t7.0\t8.5\tControl\n17.3\t1.5\t2.2\tRich\n21.5\t2.5\t3.6\tRich\n24.9\t3.5\t4.5\tRich\n29.8\t4.5\t6.4\tRich\n32.6\t5.5\t7.1\tRich\n38.4\t6.5\t9.5\tRich\n40.2\t7.5\t9.2\tRich",
    )?;
    let glm40 = gui.run_analyze(
        &mv40,
        json!({ "entry_id": "stats.glm", "glm_family": "linear", "response_column": 0, "selected_data_sets": [1, 2, 3], "reference_levels": [[3, "Control"]], "compute_vif": true }),
    )?;
    gui.ui_switch_sheet("wf40-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({}))?;
    gui.ui_answer_ok(json!({ "option_id": "diag.residual" }))?;

    // WF-41: Multiple Logistic Regression (VIF toggle + 2x2/sweep classification + ROC)
    let mv41 = gui.create_table(json!({
        "sheet_id": "wf41-mv",
        "kind": "multiple_variables",
        "rows": 16,
        "variables": [
            { "title": "Outcome", "role": "continuous" },
            { "title": "Dose", "role": "continuous" },
            { "title": "Biomarker", "role": "continuous" },
            { "title": "Genotype", "role": "categorical" }
        ]
    }))?;
    gui.paste_tsv(
        &mv41,
        0,
        0,
        0,
        "0\t1.0\t2.0\tWT\n0\t1.5\t2.4\tWT\n0\t2.0\t2.8\tMut\n0\t2.5\t3.1\tWT\n1\t2.1\t2.7\tWT\n0\t3.2\t3.6\tWT\n0\t3.6\t4.0\tWT\n1\t4.0\t4.5\tMut\n0\t4.9\t5.0\tWT\n1\t4.8\t5.2\tMut\n1\t5.2\t5.6\tWT\n1\t5.6\t6.0\tMut\n1\t6.0\t6.5\tMut\n1\t6.5\t6.9\tWT\n1\t7.0\t7.3\tMut\n1\t7.5\t7.8\tMut",
    )?;
    let log41 = gui.run_analyze(
        &mv41,
        json!({ "entry_id": "stats.glm", "glm_family": "logistic", "response_column": 0, "selected_data_sets": [1, 2, 3], "reference_levels": [[3, "WT"]], "compute_vif": true }),
    )?;
    gui.edit_analysis_cmd_t(&log41, json!({ "edit.glm.compute_vif": false }))?;
    gui.edit_analysis_cmd_t(&log41, json!({ "edit.glm.compute_vif": true }))?;

    // WF-42: Poisson Count GLM (collinear VIF >= 10 -> Cmd+T prune ExposureX2 -> VIF < 5.0)
    let mv42 = gui.create_table(json!({
        "sheet_id": "wf42-mv",
        "kind": "multiple_variables",
        "rows": 14,
        "variables": [
            { "title": "Counts", "role": "continuous" },
            { "title": "ExposureX1", "role": "continuous" },
            { "title": "ExposureX2", "role": "continuous" },
            { "title": "Temp", "role": "continuous" },
            { "title": "Batch", "role": "categorical" }
        ]
    }))?;
    gui.paste_tsv(
        &mv42,
        0,
        0,
        0,
        "2\t0.5\t1.01\t20.0\tBatchA\n3\t1.0\t2.03\t24.0\tBatchA\n4\t1.5\t2.99\t21.0\tBatchB\n5\t2.0\t4.02\t25.0\tBatchA\n7\t2.5\t4.98\t22.0\tBatchB\n9\t3.0\t6.01\t26.0\tBatchA\n11\t3.5\t7.04\t19.0\tBatchB\n15\t4.0\t7.97\t23.0\tBatchA\n19\t4.5\t9.02\t27.0\tBatchB\n24\t5.0\t9.99\t20.5\tBatchA\n31\t5.5\t11.03\t24.5\tBatchB\n38\t6.0\t12.01\t21.5\tBatchA\n49\t6.5\t12.98\t25.5\tBatchB\n62\t7.0\t14.02\t22.5\tBatchA",
    )?;
    let pois42 = gui.run_analyze(
        &mv42,
        json!({ "entry_id": "stats.glm", "glm_family": "poisson", "response_column": 0, "selected_data_sets": [1, 2, 3, 4], "reference_levels": [[4, "BatchA"]], "compute_vif": true }),
    )?;
    gui.edit_analysis_cmd_t(&pois42, json!({ "edit.glm.predictor_columns": [1, 3, 4] }))?;

    save_and_verify_headless(&mut gui, &wd.dir, "wf37_42.curvebook", |headless| {
        let rs37 = read_result_json(headless, &clust37)?;
        let _ = result_tab(&rs37, "hierarchical-dendrogram")?;
        let rs38 = read_result_json(headless, &km38)?;
        assert_approx(result_scalar_f64(&rs38, "Clusters (k)")?, 3.0, 1e-12, "WF-38 K-means k=3")?;
        let rs39 = read_result_json(headless, &cox39)?;
        let ph39 = result_tab(&rs39, "cox-ph")?;
        if ph39["row_labels"][0] != json!("Arm[Placebo vs LowDose]") {
            return Err(format!("WF-39 expected flipped reference label, got {}", ph39["row_labels"]));
        }
        let rs40 = read_result_json(headless, &glm40)?;
        if result_scalar_f64(&rs40, "R squared")? <= 0.97 {
            return Err("WF-40 expected Linear GLM R2 > 0.97".to_owned());
        }
        let rs41 = read_result_json(headless, &log41)?;
        if result_scalar_f64(&rs41, "ROC AUC")? <= 0.80 {
            return Err("WF-41 expected Logistic ROC AUC > 0.80".to_owned());
        }
        let rs42 = read_result_json(headless, &pois42)?;
        let p42 = result_tab(&rs42, "glm-poisson-params")?;
        let vif1 = tab_cell_f64(p42, "ExposureX1", "VIF")?;
        if vif1 >= 5.0 {
            return Err(format!("WF-42 pruned VIF should be < 5.0, got {vif1}"));
        }
        headless.call_tool_payload("export", json!({ "format": "svg", "sheet_id": "wf37-graph", "path": "wf37_heatmap.svg" }))?;
        let svg_hm = std::fs::read_to_string(wd.dir.join("wf37_heatmap.svg")).map_err(|e| format!("read wf37_heatmap.svg: {e}"))?;
        validate_svg_structure(&svg_hm).map_err(|e| format!("WF-37 heatmap SVG invalid: {e}"))?;
        Ok(())
    })?;
    Ok(())
}
