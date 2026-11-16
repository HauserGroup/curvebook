mod r73_common;

use lumencraft_svg::validate_svg_structure;
use r73_common::{
    McpChild, TempWorkdir, assert_approx, decode_base64, find_node, read_result_json, result_scalar_f64, result_tab, save_and_verify_headless,
    tab_cell_f64,
};
use serde_json::{Value, json};

#[test]
fn r73_gui_only_wf43_to_wf48_pharma_and_advanced_graphs() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf43_48")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-43: XY Serial Dilution -> Log10(X) -> Constrained Bottom=0/Top=1 4PL IC50 + AUC
    let xy43 = gui.create_table(json!({
        "sheet_id": "wf43-xy",
        "kind": "xy",
        "rows": 8,
        "cols": 1,
        "replicates": 2,
        "auto_graph": true,
        "graph_sheet_id": "wf43-graph"
    }))?;
    let concs = [1e-9_f64, 3.16227766e-9, 1e-8, 3.16227766e-8, 1e-7, 3.16227766e-7, 1e-6, 2e-6];
    let mut tsv_rows = Vec::with_capacity(8);
    for (r, &c) in concs.iter().enumerate() {
        gui.set_x_cell(&xy43, r, 0, &format!("{c}"))?;
        let base = 1.0 / (1.0 + 10_f64.powf(c.log10() + 6.0));
        let j = if r % 2 == 0 { 0.012 } else { -0.012 };
        tsv_rows.push(format!("{:.6}\t{:.6}", base + j, base - j));
    }
    gui.paste_tsv(&xy43, 0, 0, 0, &tsv_rows.join("\n"))?;
    let log43 = gui.run_analyze(&xy43, json!({ "entry_id": "transform.math", "x_transform": "log10", "y_transform": Value::Null }))?;
    let desc43 = gui.run_analyze(&log43, json!({ "entry_id": "stats.descriptive" }))?;
    let fit43 = gui.run_analyze(
        &log43,
        json!({
            "entry_id": "fit.nonlinear",
            "equation_id": "log_inhibitor_vs_response_4pl",
            "parameter_rules": [
                { "name": "Bottom", "scope": { "Constant": 0.0 } },
                { "name": "Top", "scope": { "Constant": 1.0 } }
            ]
        }),
    )?;
    gui.edit_analysis_cmd_t(&fit43, json!({ "edit.fit.profile_likelihood_ci": true, "edit.fit.confidence_bands": true }))?;

    // WF-44: Confidence Ellipses & Convex Hulls + N>=3 guard + undo
    let mv44 = gui.create_table(json!({
        "sheet_id": "wf44-mv",
        "kind": "multiple_variables",
        "rows": 8,
        "auto_graph": true,
        "graph_sheet_id": "wf44-graph",
        "variables": [
            { "title": "BiomarkerX", "role": "continuous" },
            { "title": "ResponseY", "role": "continuous" },
            { "title": "Amplitude", "role": "continuous" },
            { "title": "Cohort", "role": "categorical" },
            { "title": "SampleId", "role": "label" }
        ]
    }))?;
    gui.paste_tsv(
        &mv44,
        0,
        0,
        0,
        "1.0\t2.0\t10\tControl\tS1\n2.2\t3.8\t14\tControl\tS2\n3.1\t2.4\t18\tControl\tS3\n6.0\t7.5\t24\tTreated\tS4\n7.4\t9.1\t30\tTreated\tS5\n8.2\t7.2\t28\tTreated\tS6\n4.5\t11.0\t16\tRare\tS7\n5.1\t11.6\t20\tRare\tS8",
    )?;
    gui.ui_switch_sheet("wf44-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({ "sheet_id": "wf44-graph" }))?;
    gui.ui_answer_ok(json!({ "option_id": "mv.bubble" }))?;
    gui.ui_invoke("ui.graph.bubblePlot.open", json!({ "sheet_id": "wf44-graph" }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "bubble.show_convex_hulls" }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "bubble.show_confidence_ellipses" }))?;
    gui.ui_answer_ok(json!({ "x_column": 0, "y_column": 1, "size_column": 2, "label_column": 4, "color_mode": "categorical", "color_column": 3, "show_convex_hulls": true, "show_confidence_ellipses": true }))?;
    let png_both = gui.ui_screenshot("canvas", 640, 400)?;
    gui.ui_invoke("ui.graph.bubblePlot.open", json!({ "sheet_id": "wf44-graph" }))?;
    gui.ui_answer_ok(json!({ "show_convex_hulls": false, "show_confidence_ellipses": false }))?;
    let png_none = gui.ui_screenshot("canvas", 640, 400)?;
    if png_both == png_none {
        return Err("WF-44 toggling ellipses/hulls off should change canvas PNG".to_owned());
    }

    // WF-45: CorrelationMatrix R7 gate + Cividis Heatmap + SVG export
    let mv45 = gui.create_table(json!({
        "sheet_id": "wf45-mv",
        "kind": "multiple_variables",
        "rows": 6,
        "auto_graph": true,
        "graph_sheet_id": "wf45-graph",
        "variables": [
            { "title": "GeneA", "role": "continuous" },
            { "title": "GeneB", "role": "continuous" },
            { "title": "GeneC", "role": "continuous" },
            { "title": "Donor", "role": "label" }
        ]
    }))?;
    gui.paste_tsv(&mv45, 0, 0, 0, "1\t2.1\t10\tD1\n2\t4.0\t8.2\tD2\n3\t6.2\t6.1\tD3\n4\t8.1\t4.3\tD4\n5\t9.9\t2.0\tD5\n6\t12.2\t0.2\tD6")?;
    gui.ui_switch_sheet(&mv45)?;
    gui.ui_invoke("ui.analyze.open", json!({ "sheet_id": mv45 }))?;
    gui.ui_answer_dialog(json!({ "fields": { "entry_id": "stats.corr_matrix", "correlation_all_pairs": false } }))?;
    let snap45 = gui.ui_inspect("dialog", false)?;
    if find_node(&snap45["dialog"]["controls"], "analyze.correlation.heatmap").and_then(|n| n["enabled"].as_bool()) != Some(false) {
        return Err("WF-45 R7 must disable correlation heatmap when all_pairs=false".to_owned());
    }
    gui.ui_answer_dialog(
        json!({ "fields": { "correlation_all_pairs": true, "correlation_heatmap": true }, "force_create_new": true, "action": "ok" }),
    )?;
    gui.ui_switch_sheet("wf45-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({ "sheet_id": "wf45-graph" }))?;
    gui.ui_answer_ok(json!({ "option_id": "diag.heatmap" }))?;
    gui.ui_invoke("ui.export.figure.open", json!({ "sheet_id": "wf45-graph" }))?;
    let svg45_out = gui.ui_answer_ok(json!({ "format": "svg" }))?;
    let svg45 = svg45_out["payload"]["svg"].as_str().ok_or("WF-45 missing SVG")?;
    validate_svg_structure(svg45).map_err(|e| format!("WF-45 SVG invalid: {e}"))?;

    // WF-46: Interactive 5-channel Bubble Inspector swap + CategoricalSummary duplicate
    gui.ui_switch_sheet("wf44-graph")?;
    gui.ui_invoke("sheet.duplicate", json!({ "sheet_id": "wf44-graph", "new_sheet_id": "wf46-cat" }))?;
    gui.ui_switch_sheet("wf46-cat")?;
    gui.ui_invoke("ui.graph.bubblePlot.open", json!({ "sheet_id": "wf46-cat" }))?;
    gui.ui_answer_ok(json!({ "plot_mode": "categorical_summary", "x_column": 3, "y_column": 1 }))?;
    let _ = gui.ui_screenshot("canvas", 640, 400)?;

    // WF-47: KM Number-at-Risk + X-axis re-bin zoom PNG diff + pairwise survival brackets
    let surv47 = gui.create_table(json!({
        "sheet_id": "wf47-surv",
        "kind": "survival",
        "rows": 12,
        "cols": 2,
        "auto_graph": true,
        "graph_sheet_id": "wf47-graph"
    }))?;
    gui.ui_invoke("table.setGroupTitles", json!({ "sheet_id": surv47, "titles": ["Control", "HighDose"] }))?;
    for (r, &(t, c0, c1)) in [
        (2.0, Some("1"), None),
        (4.0, Some("1"), None),
        (6.0, Some("1"), None),
        (8.0, Some("0"), None),
        (10.0, Some("1"), None),
        (12.0, Some("1"), None),
        (14.0, None, Some("0")),
        (18.0, None, Some("1")),
        (22.0, None, Some("0")),
        (26.0, None, Some("0")),
        (30.0, None, Some("1")),
        (34.0, None, Some("0")),
    ]
    .iter()
    .enumerate()
    {
        gui.set_x_cell(&surv47, r, 0, &format!("{t}"))?;
        if let Some(v) = c0 {
            gui.set_cell(&surv47, r, 0, 0, v)?;
        }
        if let Some(v) = c1 {
            gui.set_cell(&surv47, r, 1, 0, v)?;
        }
    }
    let km47 = gui.run_analyze(&surv47, json!({ "entry_id": "stats.survival_km" }))?;
    gui.ui_switch_sheet("wf47-graph")?;
    gui.ui_invoke("ui.changeGraphType.open", json!({ "sheet_id": "wf47-graph" }))?;
    gui.ui_answer_ok(json!({ "option_id": "survival.km" }))?;
    gui.ui_invoke("graph.setSpec", json!({ "sheet_id": "wf47-graph", "survival_number_at_risk": true }))?;
    gui.ui_invoke("graph.setAxes", json!({ "sheet_id": "wf47-graph", "x_axis": { "title": "Months", "segments": [{ "min": 0.0, "max": 36.0 }] } }))?;
    let km_png_36 = gui.ui_screenshot("canvas", 640, 400)?;
    gui.ui_invoke("graph.setAxes", json!({ "sheet_id": "wf47-graph", "x_axis": { "title": "Months", "segments": [{ "min": 0.0, "max": 16.0 }] } }))?;
    let km_png_16 = gui.ui_screenshot("canvas", 640, 400)?;
    if km_png_36 == km_png_16 {
        return Err("WF-47 KM X-axis zoom should re-bin Number-at-Risk PNG".to_owned());
    }

    // WF-48: Bubble Sizing & Outlier Clamping + Layout Composer + Export Figure PNG
    gui.ui_invoke("layout.create", json!({ "sheet_id": "wf48-layout", "title": "WF48 Figure" }))?;
    gui.ui_invoke("layout.addPanel", json!({ "sheet_id": "wf48-layout", "kind": "graph", "graph_sheet_id": "wf47-graph", "row": 0, "col": 0 }))?;
    gui.ui_invoke("ui.layout.composer.open", json!({ "sheet_id": "wf48-layout" }))?;
    gui.ui_answer_ok(json!({ "cols": 2, "rows": 1, "align_plot_origins": true }))?;
    gui.ui_invoke("ui.export.figure.open", json!({ "sheet_id": "wf47-graph" }))?;
    let png48_out = gui.ui_answer_ok(json!({ "format": "png", "width_px": 720, "height_px": 480, "dpi": 144 }))?;
    let png48 = decode_base64(png48_out["payload"]["base64"].as_str().ok_or("WF-48 missing PNG base64")?)?;
    if !png48.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("WF-48 Export Figure PNG invalid signature".to_owned());
    }

    save_and_verify_headless(&mut gui, &wd.dir, "wf43_48.lumencraft", |headless| {
        let rs43_desc = read_result_json(headless, &desc43)?;
        let trap = result_scalar_f64(&rs43_desc, "Trapezoidal AUC")?;
        let rs43_fit = read_result_json(headless, &fit43)?;
        assert_approx(result_scalar_f64(&rs43_fit, "AUC (trapezoidal)")?, trap, 1e-6, "WF-43 AUC trapezoidal match")?;
        let ic50 = result_scalar_f64(&rs43_fit, "IC50")?;
        if (ic50 - 1e-6).abs() >= 2.5e-7 {
            return Err(format!("WF-43 constrained IC50 out of range: {ic50}"));
        }
        let rs47 = read_result_json(headless, &km47)?;
        let _ = result_tab(&rs47, "number-at-risk")?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn r73_gui_only_wf49_and_wf50_equations_and_cld_end_to_end() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf49_50")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-49: Live TeX Best-Fit Equation Annotations (Equations-Only, Substituted <-> Symbolic toggles, SVG & %PDF-1.7)
    let xy49 = gui.create_table(json!({
        "sheet_id": "wf49-xy-3pl",
        "kind": "xy",
        "rows": 8,
        "cols": 1,
        "x_title": "Log10 [Agonist] (M)",
        "auto_graph": true,
        "graph_sheet_id": "wf49-graph"
    }))?;
    for (r, (x, y)) in [(-9.0, 12.086), (-8.0, 12.851), (-7.0, 19.818), (-6.5, 32.658), (-6.0, 55.0), (-5.5, 77.342), (-5.0, 90.182), (-4.0, 97.149)]
        .iter()
        .enumerate()
    {
        gui.set_x_cell(&xy49, r, 0, &format!("{x}"))?;
        gui.set_cell(&xy49, r, 0, 0, &format!("{y:.3}"))?;
    }
    let fit49 = gui.run_analyze(&xy49, json!({ "entry_id": "fit.nonlinear", "equation_id": "log_agonist_vs_response_3pl" }))?;
    gui.edit_analysis_cmd_t(&fit49, json!({ "confidence_bands": true }))?;
    gui.ui_switch_sheet("wf49-graph")?;
    gui.ui_invoke(
        "graph.setRegressionOverlay",
        json!({ "sheet_id": "wf49-graph", "result_sheet_id": fit49, "series_index": 0, "show_confidence_band": true, "shaded_ribbon": true }),
    )?;
    gui.ui_invoke("ui.graph.insertModelEquation", json!({ "sheet_id": "wf49-graph" }))?;
    gui.ui_answer_ok(json!({ "result_sheet_id": fit49, "mode": "substituted", "bordered": true }))?;
    gui.ui_invoke("ui.export.figure.open", json!({ "sheet_id": "wf49-graph" }))?;
    let svg49_sub = gui.ui_answer_ok(json!({ "format": "svg" }))?["payload"]["svg"].as_str().ok_or("WF-49 missing SVG")?.to_owned();
    validate_svg_structure(&svg49_sub).map_err(|e| format!("WF-49 SVG invalid: {e}"))?;
    for forbidden in ["\\frac", "\\text", "CLD", "AICc"] {
        if svg49_sub.contains(forbidden) {
            return Err(format!("WF-49 SVG leaked `{forbidden}`"));
        }
    }
    gui.ui_invoke("ui.graph.format.open", json!({ "sheet_id": "wf49-graph" }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "format_graph.toggle_model_equation_mode" }))?;
    gui.ui_answer_ok(json!({}))?;
    gui.ui_invoke("ui.export.figure.open", json!({ "sheet_id": "wf49-graph" }))?;
    let pdf49_b64 = gui.ui_answer_ok(json!({ "format": "pdf" }))?["payload"]["base64"].as_str().ok_or("WF-49 missing PDF base64")?.to_owned();
    let pdf49 = decode_base64(&pdf49_b64)?;
    if !pdf49.starts_with(b"%PDF-1.7") || !pdf49.windows(5).any(|w| w == b"%%EOF") {
        return Err("WF-49 exported PDF invalid %PDF-1.7 / %%EOF".to_owned());
    }

    // WF-50: Compact Letter Display (Piepho CLD), Brackets toggle, Live TSV Edit [b, ab, a] -> [c, a, b], Cmd+Z / Cmd+Shift+Z, SVG & PDF
    let col50 = gui.create_table(json!({
        "sheet_id": "wf50-col",
        "kind": "Column",
        "rows": 5,
        "cols": 3,
        "auto_graph": true,
        "graph_sheet_id": "wf50-graph"
    }))?;
    gui.ui_invoke("table.setTitles", json!({ "sheet_id": col50, "titles": ["Ctrl", "Mid", "High"] }))?;
    gui.paste_tsv(&col50, 0, 0, 0, "7.5\t10.0\t12.5\n9.0\t11.5\t14.0\n10.0\t12.5\t15.0\n11.0\t13.5\t16.0\n12.5\t15.0\t17.5")?;
    let anova50 = gui.run_analyze(&col50, json!({ "entry_id": "stats.one_way_anova" }))?;

    gui.ui_switch_sheet("wf50-graph")?;
    gui.ui_answer_ok(json!({ "selected_option_id": "col.bar_replicates" }))?;
    gui.ui_invoke("ui.graph.pairwise.open", json!({ "sheet_id": "wf50-graph" }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "pairwise.cld_enabled" }))?;
    gui.ui_answer_ok(json!({
        "result_sheet_id": anova50,
        "cld_enabled": true,
        "cld_ordering": "descending_mean",
        "cld_uppercase": false
    }))?;
    let svg50_before = gui.ui_invoke("graph.exportSvg", json!({ "sheet_id": "wf50-graph" }))?["payload"]["svg"]
        .as_str()
        .ok_or("WF-50 missing initial SVG")?
        .to_owned();
    validate_svg_structure(&svg50_before).map_err(|e| format!("WF-50 SVG before edit invalid: {e}"))?;
    if !(svg50_before.contains(">ab<") && svg50_before.contains(">a<") && svg50_before.contains(">b<")) {
        return Err("WF-50 initial CLD SVG missing [b, ab, a] labels".to_owned());
    }
    let png50_before = gui.ui_screenshot("canvas", 720, 480)?;

    // Live edit Mid column -> [c, a, b]
    gui.paste_tsv(&col50, 0, 1, 0, "19.5\n21.0\n22.0\n23.0\n24.5")?;
    gui.ui_switch_sheet("wf50-graph")?;
    let png50_after = gui.ui_screenshot("canvas", 720, 480)?;
    if png50_before == png50_after {
        return Err("WF-50 live CLD recomputation must alter canvas PNG".to_owned());
    }
    let svg50_after = gui.ui_invoke("graph.exportSvg", json!({ "sheet_id": "wf50-graph" }))?["payload"]["svg"]
        .as_str()
        .ok_or("WF-50 missing edited SVG")?
        .to_owned();
    if !(svg50_after.contains(">c<") && svg50_after.contains(">a<") && svg50_after.contains(">b<")) {
        return Err("WF-50 edited CLD SVG missing [c, a, b] labels".to_owned());
    }

    // Undo Cmd+Z restores [b, ab, a], Redo Cmd+Shift+Z restores [c, a, b]
    gui.ui_press("z", json!({ "meta": true }))?;
    let svg50_undone = gui.ui_invoke("graph.exportSvg", json!({ "sheet_id": "wf50-graph" }))?["payload"]["svg"]
        .as_str()
        .ok_or("WF-50 missing undone SVG")?
        .to_owned();
    if !svg50_undone.contains(">ab<") {
        return Err("WF-50 Cmd+Z did not restore `ab` CLD label".to_owned());
    }
    gui.ui_press("z", json!({ "meta": true, "shift": true }))?;
    let svg50_redone = gui.ui_invoke("graph.exportSvg", json!({ "sheet_id": "wf50-graph" }))?["payload"]["svg"]
        .as_str()
        .ok_or("WF-50 missing redone SVG")?
        .to_owned();
    if !svg50_redone.contains(">c<") {
        return Err("WF-50 Cmd+Shift+Z did not restore `c` CLD label".to_owned());
    }

    save_and_verify_headless(&mut gui, &wd.dir, "wf49_50.lumencraft", |headless| {
        let rs49 = read_result_json(headless, &fit49)?;
        let logec50 = tab_cell_f64(result_tab(&rs49, "nonlinear_parameters")?, "LogEC50", "Best-fit value")?;
        assert_approx(logec50, -6.0, 0.1, "WF-49 reopened LogEC50")?;
        headless.call_tool_payload("export", json!({ "format": "svg", "sheet_id": "wf50-graph", "path": "wf50_cld.svg" }))?;
        headless.call_tool_payload("export", json!({ "format": "pdf", "sheet_id": "wf50-graph", "path": "wf50_cld.pdf" }))?;
        headless
            .call_tool_payload("export", json!({ "format": "csv", "sheet_id": &anova50, "tab_id": "multiple-comparisons", "path": "wf50_mc.csv" }))?;
        let cld_svg = std::fs::read_to_string(wd.dir.join("wf50_cld.svg")).map_err(|e| format!("read wf50_cld.svg: {e}"))?;
        validate_svg_structure(&cld_svg).map_err(|e| format!("reopened WF-50 CLD SVG invalid: {e}"))?;
        if !cld_svg.contains(">c<") {
            return Err("reopened WF-50 CLD SVG missing persisted `c` label".to_owned());
        }
        Ok(())
    })?;
    Ok(())
}
