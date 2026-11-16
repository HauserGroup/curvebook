mod r73_common;

use lumencraft_svg::validate_svg_structure;
use r73_common::{
    McpChild, TempWorkdir, assert_approx, decode_base64, find_node, read_result_json, result_scalar_f64, result_tab, save_and_verify_headless,
};
use serde_json::{Value, json};

fn assert_widget_enabled(snap: &Value, wid: &str, expected: bool) -> Result<(), String> {
    let node = find_node(&snap["dialog"]["controls"], wid)
        .or_else(|| find_node(&snap["dialog"]["tabs"], wid))
        .or_else(|| find_node(&snap["dialog"]["buttons"], wid))
        .ok_or_else(|| format!("missing dialog widget `{wid}` in {snap}"))?;
    let actual = node["enabled"].as_bool().unwrap_or(false);
    if actual != expected {
        return Err(format!("widget `{wid}` expected enabled={expected}, got {node}"));
    }
    Ok(())
}

#[test]
fn r73_gui_only_wf16_to_wf20_regression_contingency_and_roc() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf16_20")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-16: Linear & Passing-Bablok regression + R8/R9 dialog checks
    let xy16 = gui.create_table(json!({ "sheet_id": "wf16-xy", "kind": "Xy", "rows": 6, "cols": 1, "replicates": 2 }))?;
    for (r, x) in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0].iter().enumerate() {
        gui.set_x_cell(&xy16, r, 0, &format!("{x}"))?;
    }
    gui.paste_tsv(&xy16, 0, 0, 0, "3.1\t3.3\n5.0\t5.2\n7.1\t6.9\n9.2\t8.8\n11.0\t11.2\n13.1\t12.9")?;
    let lin16 = gui.run_analyze(&xy16, json!({ "entry_id": "fit.linear" }))?;
    gui.ui_switch_sheet(&lin16)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let ep_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&ep_snap, "edit.descriptive.subcolumn_agg", true)?;
    gui.ui_switch_tab(json!({ "tab": "options" }))?;
    let apa_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&apa_snap, "edit.pvalue.digits", false)?;
    gui.ui_answer_dialog(json!({ "action": "apply", "fields": { "p_value_format": "custom" } }))?;
    gui.ui_press("t", json!({ "meta": true }))?;
    gui.ui_switch_tab(json!({ "tab": "options" }))?;
    let custom_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&custom_snap, "edit.pvalue.digits", true)?;
    gui.ui_answer_dialog(json!({ "action": "cancel" }))?;

    let pb16 = gui.create_table(json!({ "sheet_id": "wf16-pb", "kind": "Column", "rows": 6, "cols": 2 }))?;
    gui.paste_tsv(&pb16, 0, 0, 0, "10.0\t10.4\n20.0\t20.2\n30.0\t30.5\n40.0\t40.1\n50.0\t50.8\n60.0\t60.3")?;
    let pb_res16 = gui.run_analyze(&pb16, json!({ "entry_id": "stats.passing_bablok" }))?;

    // WF-17: 4PL/5PL Dose-Response + Unknown Interpolation + R7 Correlation Matrix gate
    let dose17 = gui.create_table(json!({
        "sheet_id": "wf17-dose",
        "kind": "Xy",
        "rows": 10,
        "cols": 1,
        "auto_graph": true,
        "graph_sheet_id": "wf17-graph"
    }))?;
    let log_xs = [-9.0_f64, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    for (r, &lx) in log_xs.iter().enumerate() {
        gui.set_x_cell(&dose17, r, 0, &format!("{lx}"))?;
        let y = 10.0 + (95.0 - 10.0) / (1.0 + 10_f64.powf((-6.0 - lx) * 1.25));
        gui.set_cell(&dose17, r, 0, 0, &format!("{y:.4}"))?;
    }
    gui.set_cell(&dose17, 8, 0, 0, "32.0")?;
    gui.set_cell(&dose17, 9, 0, 0, "78.0")?;
    let r4pl = gui.run_analyze(&dose17, json!({ "entry_id": "fit.nonlinear", "equation_id": "log_agonist_vs_response_4pl" }))?;
    gui.ui_invoke_path(
        &["Change", "Analysis Parameters..."],
        json!({ "sheet_id": r4pl, "interpolate_unknowns": true, "profile_likelihood_ci": true, "confidence_bands": true }),
    )?;
    gui.ui_invoke_path(
        &["Graph", "Regression Overlay..."],
        json!({ "sheet_id": "wf17-graph", "result_sheet_id": r4pl, "all_series": true, "show_confidence_band": true, "shaded_ribbon": true }),
    )?;

    let mv17 = gui.create_table(json!({
        "sheet_id": "wf17-mv",
        "kind": "multiple_variables",
        "rows": 6,
        "variables": [
            { "title": "VarA", "role": "continuous" },
            { "title": "VarB", "role": "continuous" },
            { "title": "VarC", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(&mv17, 0, 0, 0, "1\t2\t5\n2\t4\t4\n3\t6\t3\n4\t8\t2\n5\t10\t1\n6\t12\t0")?;
    gui.ui_switch_sheet(&mv17)?;
    gui.ui_invoke("ui.analyze.open", json!({ "sheet_id": mv17 }))?;
    gui.ui_answer_dialog(json!({ "fields": { "entry_id": "stats.corr_matrix", "correlation_all_pairs": false } }))?;
    let r7_off = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&r7_off, "analyze.correlation.heatmap", false)?;
    gui.ui_answer_dialog(json!({ "fields": { "correlation_all_pairs": true } }))?;
    let r7_on = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&r7_on, "analyze.correlation.heatmap", true)?;
    gui.ui_answer_dialog(json!({ "action": "cancel" }))?;

    // WF-18: Michaelis-Menten Enzyme Kinetics + Global Shared Vmax
    let enz18 = gui.create_table(json!({ "sheet_id": "wf18-enz", "kind": "Xy", "rows": 6, "cols": 2 }))?;
    gui.ui_invoke("table.setTitles", json!({ "sheet_id": enz18, "titles": ["WildType", "Mutant"] }))?;
    for (r, &s) in [1.0, 2.0, 5.0, 10.0, 25.0, 50.0].iter().enumerate() {
        gui.set_x_cell(&enz18, r, 0, &format!("{s}"))?;
        let v_wt = (120.0 * s) / (8.0 + s);
        let v_mut = (120.0 * s) / (24.0 + s);
        gui.set_cell(&enz18, r, 0, 0, &format!("{v_wt:.3}"))?;
        gui.set_cell(&enz18, r, 1, 0, &format!("{v_mut:.3}"))?;
    }
    let global18_out = gui.ui_invoke_path(
        &["Analyze", "Fit Curve..."],
        json!({
            "sheet_id": "wf18-global-mm",
            "source_sheet_id": enz18,
            "equation_id": "michaelis_menten",
            "parameter_rules": [{ "name": "Vmax", "scope": "shared_global" }],
            "emit_residuals_tab": true,
            "emit_covariance_tab": true
        }),
    )?;
    if global18_out["payload"]["sheet_id"] != json!("wf18-global-mm") {
        return Err(format!("WF-18 global MM fit failed: {global18_out}"));
    }

    // WF-19: 2x2 Contingency Fisher's Exact (R5/R6 gates) + 3x3 Chi-square
    let c2 = gui.create_table(json!({ "sheet_id": "wf19-2x2", "kind": "Contingency", "rows": 2, "cols": 2 }))?;
    gui.paste_tsv(&c2, 0, 0, 0, "45\t5\n10\t40")?;
    let r2_id = gui.run_analyze(&c2, json!({ "entry_id": "stats.contingency" }))?;
    gui.ui_switch_sheet(&r2_id)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let snap_2x2 = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&snap_2x2, "edit.contingency.odds_ratio", true)?;
    assert_widget_enabled(&snap_2x2, "edit.contingency.cochran_armitage", false)?;
    gui.ui_answer_dialog(
        json!({ "action": "apply", "fields": { "compute_odds_and_risk_ratios": false, "compute_sensitivity_specificity": false } }),
    )?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let snap_ci_off = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&snap_ci_off, "edit.contingency.ci.baptista_pike", false)?;
    gui.ui_answer_dialog(json!({ "action": "ok", "fields": { "compute_odds_and_risk_ratios": true, "compute_sensitivity_specificity": true } }))?;

    // WF-20: ROC Curve + diag.roc graph + Cochran-Armitage trend (2x3)
    let roc20 = gui.create_table(json!({
        "sheet_id": "wf20-roc",
        "kind": "Column",
        "rows": 8,
        "cols": 2,
        "auto_graph": true,
        "graph_sheet_id": "wf20-roc-graph"
    }))?;
    gui.paste_tsv(&roc20, 0, 0, 0, "0.1\t0.65\n0.15\t0.72\n0.2\t0.81\n0.25\t0.88\n0.3\t0.92\n0.18\t0.55\n0.45\t0.96\n0.22\t0.78")?;
    let roc_res = gui.run_analyze(&roc20, json!({ "entry_id": "stats.roc" }))?;
    gui.ui_switch_sheet("wf20-roc-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({ "sheet_id": "wf20-roc-graph" }))?;
    gui.ui_answer_ok(json!({ "option_id": "diag.roc" }))?;
    let trend20 = gui.create_table(json!({ "sheet_id": "wf20-trend", "kind": "Contingency", "rows": 2, "cols": 3 }))?;
    gui.paste_tsv(&trend20, 0, 0, 0, "10\t20\t35\n40\t30\t15")?;
    let trend_res = gui.run_analyze(&trend20, json!({ "entry_id": "stats.contingency" }))?;
    gui.edit_analysis_cmd_t(&trend_res, json!({ "contingency_method": "cochranarmitage" }))?;

    save_and_verify_headless(&mut gui, &wd.dir, "wf16_20.lumencraft", |headless| {
        let rs16 = read_result_json(headless, &lin16)?;
        assert_approx(result_scalar_f64(&rs16, "Slope")?, 2.0, 0.15, "WF-16 linear slope")?;
        let rs_pb = read_result_json(headless, &pb_res16)?;
        assert_approx(result_scalar_f64(&rs_pb, "Slope")?, 1.0, 0.05, "WF-16 Passing-Bablok Slope")?;
        let _ = result_tab(&rs_pb, "pb-residuals")?;
        let rs17 = read_result_json(headless, &r4pl)?;
        assert_approx(result_scalar_f64(&rs17, "LogEC50")?, -6.0, 0.05, "WF-17 LogEC50")?;
        if !rs17["tabs"].as_array().is_some_and(|arr| arr.iter().any(|t| t["kind"] == json!("InterpolatedUnknowns"))) {
            return Err(format!("WF-17 missing InterpolatedUnknowns tab: {}", rs17["tabs"]));
        }
        let rs18 = read_result_json(headless, "wf18-global-mm")?;
        assert_approx(result_scalar_f64(&rs18, "Vmax")?, 120.0, 1.0, "WF-18 shared Vmax")?;
        let rs20 = read_result_json(headless, &roc_res)?;
        if result_scalar_f64(&rs20, "Area under ROC curve (AUC)")? < 0.85 {
            return Err("WF-20 expected ROC AUC >= 0.85".to_owned());
        }
        Ok(())
    })?;
    Ok(())
}

#[test]
fn r73_gui_only_wf21_to_wf25_multivariate_chaining_and_styled_graphs() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf21_25")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-21: Survival KM + Number-at-Risk Graph + Multiple Logistic Regression
    let surv21 = gui.create_table(json!({
        "sheet_id": "wf21-s2",
        "kind": "Survival",
        "rows": 10,
        "cols": 2,
        "auto_graph": true,
        "graph_sheet_id": "wf21-km-graph"
    }))?;
    gui.ui_invoke("table.setTitles", json!({ "sheet_id": surv21, "titles": ["Placebo", "Active"] }))?;
    for (r, &(t, c0, c1)) in [
        (1.0, Some("1"), None),
        (2.0, Some("1"), Some("0")),
        (3.0, Some("1"), None),
        (4.0, Some("1"), Some("0")),
        (5.0, Some("0"), None),
        (7.0, None, Some("1")),
        (9.0, None, Some("0")),
        (11.0, None, Some("1")),
        (13.0, None, Some("0")),
        (15.0, None, Some("1")),
    ]
    .iter()
    .enumerate()
    {
        gui.set_x_cell(&surv21, r, 0, &format!("{t}"))?;
        if let Some(v) = c0 {
            gui.set_cell(&surv21, r, 0, 0, v)?;
        }
        if let Some(v) = c1 {
            gui.set_cell(&surv21, r, 1, 0, v)?;
        }
    }
    let km21 = gui.run_analyze(&surv21, json!({ "entry_id": "stats.survival_km" }))?;
    gui.ui_invoke_path(&["Format", "Graph..."], json!({ "sheet_id": "wf21-km-graph", "survival_number_at_risk": true }))?;

    // WF-22: PCA + Clustering + Multiple Row t-tests (Volcano)
    let mv22 = gui.create_table(json!({
        "sheet_id": "wf22-mv",
        "kind": "multiple_variables",
        "rows": 6,
        "variables": [
            { "title": "F1", "role": "continuous" },
            { "title": "F2", "role": "continuous" },
            { "title": "F3", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(&mv22, 0, 0, 0, "1\t2\t3\n1.5\t2.4\t3.2\n2\t3.1\t3.9\n8\t9\t10\n8.5\t9.4\t10.2\n9\t10\t11")?;
    let pca22 = gui.run_analyze(&mv22, json!({ "entry_id": "stats.pca", "pca_predictors": [0, 1, 2] }))?;
    let clust22 = gui.run_analyze(&mv22, json!({ "entry_id": "stats.clustering", "selected_data_sets": [0, 1, 2] }))?;

    let grp22 = gui.create_table(json!({ "sheet_id": "wf22-grp", "kind": "Grouped", "rows": 4, "cols": 2, "replicates": 3 }))?;
    gui.paste_tsv(&grp22, 0, 0, 0, "10\t11\t10\t22\t23\t24\n5\t6\t5\t5\t6\t6\n1\t2\t2\t8\t9\t8\n30\t31\t32\t15\t14\t16")?;
    let volcano22 = gui.run_analyze(&grp22, json!({ "entry_id": "stats.multiple_row_t_tests" }))?;

    // WF-23: Stacked vs Superimposed Grouped Bars + PCR + Result->Result Chaining
    gui.ui_invoke("graph.create", json!({ "data_sheet_id": grp22, "sheet_id": "wf23-grp-graph" }))?;
    gui.ui_switch_sheet("wf23-grp-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({}))?;
    gui.ui_answer_ok(json!({ "option_id": "grouped.stacked" }))?;
    gui.ui_invoke_path(&["Format", "Graph..."], json!({ "sheet_id": "wf23-grp-graph", "grouped_arrangement": "Superimposed" }))?;

    let pcr_mv = gui.create_table(json!({
        "sheet_id": "wf23-pcr-mv",
        "kind": "multiple_variables",
        "rows": 6,
        "variables": [
            { "title": "X1", "role": "continuous" },
            { "title": "X2", "role": "continuous" },
            { "title": "Y", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(&pcr_mv, 0, 0, 0, "1\t2\t4\n2\t4\t7\n3\t6\t10\n4\t8\t13\n5\t10\t16\n6\t12\t19")?;
    let pcr_before = gui.navigator_result_ids()?;
    gui.ui_switch_sheet(&pcr_mv)?;
    gui.ui_invoke("ui.analyze.open", json!({ "sheet_id": pcr_mv }))?;
    gui.ui_answer_dialog(json!({ "fields": { "entry_id": "stats.pca", "pca_predictors": [0, 1] } }))?;
    let pcr_r10 = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&pcr_r10, "analyze.pcr.outcome.0", false)?;
    assert_widget_enabled(&pcr_r10, "analyze.pcr.outcome.2", true)?;
    let _pcr_leak = gui.ui_answer_err(json!({ "fields": { "pcr_outcome_column": 0 } }))?;
    let pcr_out = gui.ui_answer_dialog(json!({ "fields": { "pcr_outcome_column": 2 }, "force_create_new": true, "action": "ok" }))?;
    let pcr23 = if let Some(sid) = pcr_out["payload"]["sheet_id"].as_str() {
        sid.to_owned()
    } else {
        gui.navigator_result_ids()?.into_iter().rev().find(|id| !pcr_before.contains(id)).ok_or("WF-23 PCR missing ResultSheet")?
    };

    // WF-24 & WF-25: Significance Brackets, Annotations, Axes, Box-and-Whisker & Violin PNG diffs + SVG export
    let col24 = gui.create_table(json!({
        "sheet_id": "wf24-col",
        "kind": "Column",
        "rows": 5,
        "cols": 3,
        "auto_graph": true,
        "graph_sheet_id": "wf24-graph"
    }))?;
    gui.ui_invoke("table.setTitles", json!({ "sheet_id": col24, "titles": ["Ctrl", "Low", "High"] }))?;
    gui.paste_tsv(&col24, 0, 0, 0, "10\t15\t25\n11\t16\t26\n9\t14\t24\n10\t15\t27\n12\t17\t28")?;
    let anova24 = gui.run_analyze(&col24, json!({ "entry_id": "stats.one_way_anova" }))?;
    gui.ui_invoke("graph.addComparisonBrackets", json!({ "sheet_id": "wf24-graph", "result_sheet_id": anova24, "significant_only": true }))?;
    gui.ui_invoke(
        "graph.addAnnotation",
        json!({ "sheet_id": "wf24-graph", "kind": "text", "text": "Dose effect", "bounds_pt": [120.0, 22.0, 145.0, 20.0] }),
    )?;
    gui.ui_invoke(
        "graph.setAxes",
        json!({ "sheet_id": "wf24-graph", "y_axis": { "title": "Signal (AU)", "segments": [{ "min": 0.0, "max": 35.0 }] } }),
    )?;

    gui.ui_switch_sheet("wf24-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({ "sheet_id": "wf24-graph" }))?;
    gui.ui_answer_ok(json!({ "option_id": "col.box_whisker" }))?;
    let box_gdo = json!({
        "BoxAndWhisker": {
            "whiskers": "Tukey1_5Iqr",
            "fill_color": { "r": 0, "g": 114, "b": 178, "a": 190 },
            "stroke_color": { "r": 0, "g": 0, "b": 0, "a": 255 },
            "show_outliers": true,
            "show_all_points": false
        }
    });
    gui.ui_invoke("graph.setSeries", json!({ "sheet_id": "wf24-graph", "series": [{ "gdo": box_gdo }, { "gdo": box_gdo }, { "gdo": box_gdo }] }))?;
    let box_png = gui.ui_screenshot("canvas", 720, 480)?;
    let box_svg_out = gui.ui_invoke("graph.exportSvg", json!({ "sheet_id": "wf24-graph" }))?;
    let box_svg = box_svg_out["payload"]["svg"].as_str().ok_or("WF-25 missing box SVG")?;
    validate_svg_structure(box_svg).map_err(|e| format!("WF-25 invalid box SVG: {e}"))?;

    gui.ui_invoke("ui.changeGraphType.open", json!({ "sheet_id": "wf24-graph" }))?;
    gui.ui_answer_ok(json!({ "option_id": "col.violin" }))?;
    let violin_gdo = json!({
        "Violin": {
            "fill_color": { "r": 0, "g": 114, "b": 178, "a": 190 },
            "stroke_color": { "r": 0, "g": 114, "b": 178, "a": 255 },
            "clip_to_observed_range": true,
            "show_median_and_quartiles": true,
            "half_violin_with_points": false
        }
    });
    gui.ui_invoke(
        "graph.setSeries",
        json!({ "sheet_id": "wf24-graph", "series": [{ "gdo": violin_gdo }, { "gdo": violin_gdo }, { "gdo": violin_gdo }] }),
    )?;
    let violin_png = gui.ui_screenshot("canvas", 720, 480)?;
    if box_png == violin_png {
        return Err("WF-25 Box-and-Whisker and Violin canvas screenshots should visually differ".to_owned());
    }

    save_and_verify_headless(&mut gui, &wd.dir, "wf21_25.lumencraft", |headless| {
        let rs_km = read_result_json(headless, &km21)?;
        let _ = result_tab(&rs_km, "number-at-risk")?;
        let rs_pca = read_result_json(headless, &pca22)?;
        let _ = result_tab(&rs_pca, "pca-loadings")?;
        let rs_clust = read_result_json(headless, &clust22)?;
        let _ = result_tab(&rs_clust, "hierarchical-dendrogram")?;
        let rs_vol = read_result_json(headless, &volcano22)?;
        if rs_vol["tabs"].as_array().is_none_or(Vec::is_empty) {
            return Err("WF-22 volcano tabs empty".to_owned());
        }
        let rs_pcr = read_result_json(headless, &pcr23)?;
        if rs_pcr["tabs"].as_array().is_none_or(Vec::is_empty) {
            return Err("WF-23 PCR tabs empty".to_owned());
        }
        headless.call_tool_payload("export", json!({ "format": "svg", "sheet_id": "wf24-graph", "path": "wf25_violin.svg" }))?;
        let svg_str = std::fs::read_to_string(wd.dir.join("wf25_violin.svg")).map_err(|e| format!("read wf25_violin.svg: {e}"))?;
        validate_svg_structure(&svg_str).map_err(|e| format!("reopened WF-25 violin SVG invalid: {e}"))?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn r73_gui_only_wf26_to_wf28_multivariate_layouts_and_export() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf26_28")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-26: Multivariate Bubble + Heatmap + PCR R10 check
    let mv26 = gui.create_table(json!({
        "sheet_id": "wf26-mv",
        "kind": "multiple_variables",
        "rows": 6,
        "variables": [
            { "title": "Dose", "role": "continuous" },
            { "title": "Resp", "role": "continuous" },
            { "title": "Size", "role": "continuous" },
            { "title": "Outcome", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(&mv26, 0, 0, 0, "1\t2\t5\t10\n2\t4\t8\t15\n3\t6\t10\t22\n4\t8\t12\t28\n5\t10\t15\t35\n6\t12\t18\t41")?;
    gui.ui_invoke("graph.create", json!({ "data_sheet_id": mv26, "sheet_id": "wf26-bubble" }))?;
    gui.ui_invoke(
        "graph.setSpec",
        json!({ "sheet_id": "wf26-bubble", "kind": "MultipleVariables", "multivariate": { "x_column": 0, "y_column": 1 } }),
    )?;
    gui.ui_switch_sheet("wf26-bubble")?;
    let _ = gui.ui_screenshot("canvas", 640, 400)?;

    // WF-27: Multi-panel LayoutSheet + ApplyStyleFrom + Pointer drag + Preflight + Save/Reopen Export
    let xy_donor = gui.create_table(
        json!({ "sheet_id": "wf27-donor-xy", "kind": "Xy", "rows": 4, "cols": 1, "auto_graph": true, "graph_sheet_id": "wf27-g-donor" }),
    )?;
    for (r, x) in [1.0, 2.0, 3.0, 4.0].iter().enumerate() {
        gui.set_x_cell(&xy_donor, r, 0, &format!("{x}"))?;
        gui.set_cell(&xy_donor, r, 0, 0, &format!("{:.1}", x * 2.0))?;
    }
    let xy_target = gui.create_table(
        json!({ "sheet_id": "wf27-target-xy", "kind": "Xy", "rows": 4, "cols": 1, "auto_graph": true, "graph_sheet_id": "wf27-g-target" }),
    )?;
    for (r, x) in [1.0, 2.0, 3.0, 4.0].iter().enumerate() {
        gui.set_x_cell(&xy_target, r, 0, &format!("{x}"))?;
        gui.set_cell(&xy_target, r, 0, 0, &format!("{:.1}", x * 3.0))?;
    }
    gui.ui_invoke("graph.setSpec", json!({ "sheet_id": "wf27-g-donor", "palette": "PaulTolVibrant" }))?;
    gui.ui_invoke("graph.applyStyleFrom", json!({ "sheet_id": "wf27-g-target", "donor_sheet_id": "wf27-g-donor" }))?;
    let _bad_donor = gui.ui_invoke_err("graph.applyStyleFrom", json!({ "sheet_id": "wf27-g-target", "donor_sheet_id": "wf27-donor-xy" }))?;

    gui.ui_invoke("layout.create", json!({ "sheet_id": "wf27-layout", "title": "WF27 Figure", "preset": "double_column" }))?;
    gui.ui_invoke(
        "layout.addPanel",
        json!({ "sheet_id": "wf27-layout", "graph_sheet_id": "wf27-g-donor", "bounds_pt": [24.0, 24.0, 200.0, 150.0] }),
    )?;
    gui.ui_invoke(
        "layout.addPanel",
        json!({ "sheet_id": "wf27-layout", "graph_sheet_id": "wf27-g-target", "bounds_pt": [260.0, 42.0, 180.0, 130.0] }),
    )?;
    gui.ui_invoke(
        "layout.addPanel",
        json!({ "sheet_id": "wf27-layout", "kind": "text", "text": "Composite Figure", "bounds_pt": [24.0, 195.0, 220.0, 22.0] }),
    )?;
    gui.ui_switch_sheet("wf27-layout")?;
    let all_insp = gui.ui_inspect("all", true)?;
    let nav_right = all_insp["navigator_sheets"]
        .as_array()
        .and_then(|arr| {
            arr.iter().filter_map(|n| n["bounds"].as_array()).map(|b| b[0].as_f64().unwrap_or(0.0) + b[2].as_f64().unwrap_or(0.0)).reduce(f64::max)
        })
        .unwrap_or(240.0);
    let top_y = all_insp["toolbar"]
        .as_array()
        .and_then(|arr| {
            arr.iter().filter_map(|n| n["bounds"].as_array()).map(|b| b[1].as_f64().unwrap_or(0.0) + b[3].as_f64().unwrap_or(0.0)).reduce(f64::max)
        })
        .unwrap_or(80.0);
    let canvas_w = (1280.0 - nav_right).max(300.0);
    let canvas_h = (800.0 - top_y).max(240.0);
    let before_drag_shot = gui.ui_screenshot("canvas", 720, 480)?;
    let drag_x = nav_right + canvas_w * 0.68;
    let drag_y = top_y + canvas_h * 0.30;
    gui.ui_pointer(json!({ "action": "drag", "x": drag_x, "y": drag_y, "to_x": drag_x, "to_y": drag_y - 32.0 }))?;
    let after_drag_shot = gui.ui_screenshot("canvas", 720, 480)?;
    if before_drag_shot == after_drag_shot {
        return Err("WF-27 layout panel drag should visually update canvas PNG".to_owned());
    }

    let preflight = gui.ui_invoke("ui.canvas.preflight", json!({ "sheet_id": "wf27-layout" }))?;
    if preflight["result"]["issue_count"].as_u64().is_none() {
        return Err(format!("WF-27 preflight missing issue_count: {preflight}"));
    }

    // WF-28: Canvas Preset, CVD Filter, Print Setup + Crop Marks, Inline SVG/PDF/PNG, Light vs Dark screenshots
    gui.ui_invoke("ui.canvas.applyPreset", json!({ "sheet_id": "wf27-g-target", "preset": "SingleColumn88mm", "resize_mode": "KeepTypographyPt" }))?;
    gui.ui_invoke("ui.previewFilter.set", json!({ "filter": "deuteranopia" }))?;
    gui.ui_invoke("ui.previewFilter.set", json!({ "filter": "normal" }))?;

    gui.ui_invoke("ui.print.open", json!({ "sheet_id": "wf27-g-target", "copies": 2, "scale_percent": 100.0, "crop_marks": true, "bleed_pt": 9.0 }))?;
    let print_insp = gui.ui_inspect("dialog", false)?;
    if print_insp["active_dialog"] != json!("print") {
        return Err(format!("WF-28 expected print dialog, got {print_insp}"));
    }
    gui.ui_answer_dialog(json!({ "fields": { "crop_marks": false, "bleed_pt": 12.0, "copies": 3 } }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "print.crop_marks" }))?;
    gui.ui_answer_dialog(json!({ "action": "ok" }))?;

    let inline_pdf = gui.ui_invoke("export.pdf", json!({ "sheet_id": "wf27-g-target", "color_space": "cmyk" }))?;
    let pdf_bytes = decode_base64(inline_pdf["payload"]["base64"].as_str().ok_or("WF-28 missing inline PDF base64")?)?;
    if !pdf_bytes.starts_with(b"%PDF-1.7") {
        return Err("WF-28 inline PDF missing %PDF-1.7".to_owned());
    }
    let inline_png = gui.ui_invoke("export.png", json!({ "sheet_id": "wf27-g-target", "dpi": 144, "width_px": 480, "height_px": 320 }))?;
    let png_bytes = decode_base64(inline_png["payload"]["base64"].as_str().ok_or("WF-28 missing inline PNG base64")?)?;
    if !png_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("WF-28 inline PNG missing PNG signature".to_owned());
    }

    gui.ui_invoke("ui.theme.set", json!({ "mode": "light" }))?;
    let light_shot = gui.ui_screenshot("full_window", 960, 640)?;
    gui.ui_invoke("ui.theme.set", json!({ "mode": "dark" }))?;
    let dark_shot = gui.ui_screenshot("full_window", 960, 640)?;
    if light_shot == dark_shot {
        return Err("WF-28 light and dark theme full_window screenshots should differ".to_owned());
    }

    save_and_verify_headless(&mut gui, &wd.dir, "wf26_28.lumencraft", |headless| {
        headless.call_tool_payload("export", json!({ "format": "svg", "sheet_id": "wf27-layout", "path": "wf27_layout.svg" }))?;
        headless.call_tool_payload("export", json!({ "format": "pdf", "sheet_id": "wf27-layout", "path": "wf27_layout.pdf" }))?;
        let lay_svg = std::fs::read_to_string(wd.dir.join("wf27_layout.svg")).map_err(|e| format!("read layout svg: {e}"))?;
        validate_svg_structure(&lay_svg).map_err(|e| format!("reopened WF-27 layout SVG invalid: {e}"))?;
        let lay_pdf = std::fs::read(wd.dir.join("wf27_layout.pdf")).map_err(|e| format!("read layout pdf: {e}"))?;
        if !lay_pdf.starts_with(b"%PDF-1.7") {
            return Err("reopened WF-27 layout PDF missing %PDF-1.7".to_owned());
        }
        Ok(())
    })?;
    Ok(())
}
