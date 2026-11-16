mod r73_common;

use r73_common::{McpChild, TempWorkdir, assert_approx, find_node, read_result_json, result_scalar_f64, result_tab, save_and_verify_headless};
use serde_json::json;

fn assert_widget_enabled(snap: &serde_json::Value, wid: &str, expected: bool) -> Result<(), String> {
    let node = find_node(&snap["dialog"]["controls"], wid)
        .or_else(|| find_node(&snap["dialog"]["tabs"], wid))
        .or_else(|| find_node(&snap["dialog"]["buttons"], wid))
        .ok_or_else(|| format!("missing dialog widget `{wid}` in {snap}"))?;
    let actual = node["enabled"].as_bool().unwrap_or(false);
    if actual != expected {
        return Err(format!("widget `{wid}` expected enabled={expected}, got {node}"));
    }
    if !expected && node["disabled_reason"].as_str().is_none_or(str::is_empty) {
        return Err(format!("disabled widget `{wid}` missing disabled_reason: {node}"));
    }
    Ok(())
}

#[test]
fn r73_gui_only_wf01_to_wf06_foundations_and_data() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf01_06")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-01: XY Dose-Response creation, TSV + cell edit, Cmd+Shift+E Exclude with Reason, Canvas screenshot, Save + Reopen
    let xy1 = gui.create_table(json!({
        "sheet_id": "wf01-xy",
        "title": "Dose Response",
        "kind": "Xy",
        "rows": 7,
        "cols": 1,
        "auto_graph": true,
        "graph_sheet_id": "wf01-graph"
    }))?;
    for (r, x) in [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0].iter().enumerate() {
        gui.set_x_cell(&xy1, r, 0, &format!("{x}"))?;
    }
    gui.paste_tsv(&xy1, 0, 0, 0, "99\n95\n81\n50\n19\n6\n2")?;
    gui.exclude_cell_cmd_shift_e(&xy1, 6, 0, 0, "Outlier", "saturated bottom well")?;
    let grid1 = gui.ui_inspect("grid", false)?;
    let excl_cell = find_node(&grid1["grid_cells"], "grid.cell.6.0.0").ok_or("missing grid.cell.6.0.0")?;
    if excl_cell["value"]["excluded"] != json!(true) {
        return Err(format!("WF-01 cell 6 expected excluded=true: {excl_cell}"));
    }
    let shot_wf01 = gui.ui_screenshot("workspace", 960, 640)?;
    if shot_wf01.len() < 256 {
        return Err("WF-01 screenshot unexpectedly small".to_owned());
    }

    // WF-02: Normalize -> ResultSheet chaining -> Log10(X) transform
    let norm_id = gui.run_analyze(&xy1, json!({ "entry_id": "transform.normalize" }))?;
    let log_norm_id = gui.run_analyze(&norm_id, json!({ "entry_id": "transform.math", "x_transform": "log10", "y_transform": null }))?;
    let rs_log_tabs = gui.ui_switch_sheet(&log_norm_id)?;
    if rs_log_tabs["sheet_id"] != json!(log_norm_id) {
        return Err("WF-02 failed to switch to chained ResultSheet".to_owned());
    }

    // WF-03: Column brushing (canvas point click -> grid highlight), Format Selected Points, Cmd+Z / Cmd+Shift+Z
    let col3 = gui.create_table(
        json!({ "sheet_id": "wf03-col", "kind": "Column", "rows": 5, "cols": 2, "auto_graph": true, "graph_sheet_id": "wf03-graph" }),
    )?;
    gui.paste_tsv(&col3, 0, 0, 0, "10\t20\n12\t22\n14\t24\n16\t26\n18\t28")?;
    gui.ui_switch_sheet("wf03-graph")?;
    let canvas_snap = gui.ui_inspect("canvas", true)?;
    let pt_id = canvas_snap["canvas_elements"]
        .as_array()
        .and_then(|arr| arr.iter().find(|n| n["role"] == json!("datapoint")))
        .and_then(|n| n["widget_id"].as_str())
        .ok_or("WF-03 expected plotted canvas datapoint widget")?
        .to_owned();
    gui.ui_pointer(json!({ "action": "click", "widget_id": pt_id }))?;
    let brushed = gui.ui_inspect("all", false)?;
    if brushed["brushed_points"].as_array().is_none_or(Vec::is_empty) {
        return Err(format!("WF-03 point click did not brush grid point: {brushed}"));
    }
    gui.set_cell(&col3, 0, 0, 0, "99")?;
    gui.ui_press("z", json!({ "meta": true }))?;
    let after_undo = gui.ui_inspect("grid", false)?;
    if find_node(&after_undo["grid_cells"], "grid.cell.0.0.0").and_then(|c| c["value"]["text"].as_str()) == Some("99") {
        return Err("WF-03 Cmd+Z did not undo cell edit".to_owned());
    }
    gui.ui_press("z", json!({ "meta": true, "shift": true }))?;

    // WF-04: Import Reshape (TidyLongPivot -> Column) & Convert Archetype (Column -> XY)
    gui.ui_invoke_path(&["File", "Import Reshape..."], json!({}))?;
    let reshape_out = gui.ui_answer_ok(json!({
        "mode": "TidyLongPivot",
        "title": "Dialog Tidy Pivot",
        "target_kind": "Column",
        "tidy_csv": "Group,Value\nCtrl,10\nCtrl,12\nDrug,20\nDrug,22"
    }))?;
    let imp4 = reshape_out["payload"]["sheet_id"].as_str().ok_or_else(|| format!("WF-04 ImportReshape missing sheet_id: {reshape_out}"))?.to_owned();
    gui.ui_invoke("ui.table.convertArchetype.open", json!({ "sheet_id": imp4 }))?;
    gui.ui_answer_ok(json!({ "target_kind": "Xy" }))?;
    gui.ui_switch_sheet(&imp4)?;
    let grid4 = gui.ui_inspect("grid", false)?;
    if find_node(&grid4["grid_cells"], "grid.xcell.0.0").is_none() {
        return Err(format!("WF-04 converted Xy table missing grid.xcell.0.0: {grid4}"));
    }

    // WF-05: Prune -> PCA & Clustering pipeline with ui_switch_tab across tabs
    let mv5 = gui.create_table(json!({
        "sheet_id": "wf05-mv",
        "kind": "multiple_variables",
        "rows": 6,
        "variables": [
            { "title": "Label", "role": "label" },
            { "title": "X1", "role": "continuous" },
            { "title": "X2", "role": "continuous" },
            { "title": "X3", "role": "continuous" }
        ]
    }))?;
    gui.paste_tsv(
        &mv5,
        0,
        0,
        0,
        "S1\t1.0\t2.1\t3.0\nS2\t1.2\t2.3\t3.1\nS3\t5.0\t6.1\t7.2\nS4\t5.3\t6.0\t7.0\nS5\t9.1\t10.2\t11.0\nS6\t9.3\t10.0\t11.2",
    )?;
    let pca5 = gui.run_analyze(&mv5, json!({ "entry_id": "stats.pca", "pca_predictors": [1, 2, 3] }))?;
    gui.ui_switch_tab(json!({ "sheet_id": pca5, "tab": "pca-loadings" }))?;
    let clust5 = gui.run_analyze(&mv5, json!({ "entry_id": "stats.clustering", "selected_data_sets": [1, 2, 3] }))?;
    gui.ui_switch_tab(json!({ "sheet_id": clust5, "tab": "kmeans-clusters" }))?;
    gui.ui_switch_tab(json!({ "sheet_id": clust5, "tab": "hierarchical-dendrogram" }))?;

    // WF-06: Prune -> Descriptive -> Survival KM chaining & reactive recompute
    let surv6 = gui.create_table(json!({ "sheet_id": "wf06-surv", "kind": "Survival", "rows": 6, "cols": 2 }))?;
    for (r, (t, s0, s1)) in [(5.0, "1", ""), (10.0, "1", ""), (15.0, "0", ""), (8.0, "", "1"), (18.0, "", "1"), (24.0, "", "0")].iter().enumerate() {
        gui.set_x_cell(&surv6, r, 0, &format!("{t}"))?;
        if !s0.is_empty() {
            gui.set_cell(&surv6, r, 0, 0, s0)?;
        } else {
            gui.set_cell(&surv6, r, 1, 0, s1)?;
        }
    }
    let km6 = gui.run_analyze(&surv6, json!({ "entry_id": "stats.survival_km" }))?;

    save_and_verify_headless(&mut gui, &wd.dir, "wf01_06.curvebook", |headless| {
        let rs_pca = read_result_json(headless, &pca5)?;
        let _ = result_tab(&rs_pca, "pca-loadings")?;
        let rs_clust = read_result_json(headless, &clust5)?;
        let _ = result_tab(&rs_clust, "kmeans-clusters")?;
        let _ = result_tab(&rs_clust, "hierarchical-dendrogram")?;
        let rs_km = read_result_json(headless, &km6)?;
        let _ = result_tab(&rs_km, "km-summary")?;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn r73_gui_only_wf07_to_wf11_two_group_and_one_way_anova() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf07_11")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-07: Two-group Student -> Welch & option predicates
    let col7 = gui.create_table(
        json!({ "sheet_id": "wf07-col", "kind": "Column", "rows": 5, "cols": 2, "auto_graph": true, "graph_sheet_id": "wf07-graph" }),
    )?;
    gui.ui_invoke("table.setTitles", json!({ "sheet_id": col7, "titles": ["Control", "Treated"] }))?;
    gui.paste_tsv(&col7, 0, 0, 0, "10.0\t18.0\n11.0\t19.0\n12.0\t20.0\n10.5\t18.5\n11.5\t21.0")?;
    let tg7 = gui.run_analyze(&col7, json!({ "entry_id": "stats.two_group" }))?;
    gui.ui_switch_sheet(&tg7)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let snap7 = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&snap7, "edit.two_group.welch", true)?;
    assert_widget_enabled(&snap7, "edit.two_group.graph_differences", false)?;
    let _disabled_click = gui.ui_pointer_err(json!({ "action": "click", "widget_id": "edit.two_group.graph_differences" }))?;
    gui.ui_switch_tab(json!({ "tab": "options" }))?;
    gui.ui_answer_dialog(json!({ "action": "apply", "fields": { "p_value_format": "custom", "confidence_percent": 99.0 } }))?;
    gui.ui_press("t", json!({ "meta": true }))?;
    gui.ui_switch_tab(json!({ "tab": "options" }))?;
    let snap_opt = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&snap_opt, "edit.pvalue.digits", true)?;
    gui.ui_type(Some("edit.pvalue.digits"), "4", true, false)?;
    gui.ui_switch_tab(json!({ "tab": "experimental_design" }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "edit.two_group.welch" }))?;
    gui.ui_answer_ok(json!({}))?;

    // WF-08: Paired t-test equal-N check (R1 disabled on unequal N, enabled on equal N)
    let uneq = gui.create_table(json!({ "sheet_id": "wf08-uneq", "kind": "Column", "rows": 5, "cols": 2 }))?;
    gui.paste_tsv(&uneq, 0, 0, 0, "10\t14.2\n11\t16.1\n12\t15.4\n13\t\n14\t")?;
    let tg_uneq = gui.run_analyze(&uneq, json!({ "entry_id": "stats.two_group" }))?;
    gui.ui_switch_sheet(&tg_uneq)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let uneq_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&uneq_snap, "edit.paired_or_matched", false)?;
    let _ = gui.ui_pointer_err(json!({ "action": "click", "widget_id": "edit.paired_or_matched" }))?;
    gui.ui_answer_dialog(json!({ "action": "cancel" }))?;

    gui.set_cell(&uneq, 3, 1, 0, "17.8")?;
    gui.set_cell(&uneq, 4, 1, 0, "19.5")?;
    gui.ui_switch_sheet(&tg_uneq)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let eq_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&eq_snap, "edit.paired_or_matched", true)?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "edit.paired_or_matched" }))?;
    let paired_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&paired_snap, "edit.two_group.welch", false)?;
    gui.ui_answer_ok(json!({}))?;

    // WF-09: Nonparametric Mann-Whitney & Wilcoxon
    gui.ui_switch_sheet(&tg7)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "edit.assume_gaussian" }))?;
    let np_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&np_snap, "edit.two_group.graph_ranks", true)?;
    assert_widget_enabled(&np_snap, "edit.two_group.welch", false)?;
    gui.ui_answer_ok(json!({}))?;

    // WF-10: One-way ANOVA -> Dunnett vs Tukey & Brackets
    let col10 = gui.create_table(
        json!({ "sheet_id": "wf10-col", "kind": "Column", "rows": 4, "cols": 3, "auto_graph": true, "graph_sheet_id": "wf10-graph" }),
    )?;
    gui.ui_invoke("table.setTitles", json!({ "sheet_id": col10, "titles": ["Veh", "DoseA", "DoseB"] }))?;
    gui.paste_tsv(&col10, 0, 0, 0, "10\t15\t25\n11\t16\t26\n12\t17\t27\n10\t15\t28")?;
    let anova10 = gui.run_analyze(&col10, json!({ "entry_id": "stats.one_way_anova" }))?;
    gui.ui_switch_sheet(&anova10)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let _ = gui.ui_switch_tab_err(json!({ "tab": "repeated_measures" }))?;
    gui.ui_switch_tab(json!({ "tab": "multiple_comparisons" }))?;
    let mc_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&mc_snap, "edit.post_hoc.tukey", true)?;
    assert_widget_enabled(&mc_snap, "edit.post_hoc.dunn", false)?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "edit.post_hoc.tukey" }))?;
    gui.ui_answer_ok(json!({}))?;
    gui.ui_invoke("graph.addComparisonBrackets", json!({ "sheet_id": "wf10-graph", "result_sheet_id": anova10, "significant_only": true }))?;

    // WF-11: Kruskal-Wallis & Dunn's multiple comparisons
    gui.ui_switch_sheet(&anova10)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "edit.assume_gaussian" }))?;
    gui.ui_switch_tab(json!({ "tab": "multiple_comparisons" }))?;
    let kw_snap = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&kw_snap, "edit.post_hoc.dunn", true)?;
    assert_widget_enabled(&kw_snap, "edit.post_hoc.tukey", false)?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "edit.post_hoc.dunn" }))?;
    gui.ui_answer_ok(json!({}))?;

    save_and_verify_headless(&mut gui, &wd.dir, "wf07_11.curvebook", |headless| {
        let rs10 = read_result_json(headless, &anova10)?;
        let _ = result_tab(&rs10, "multiple-comparisons")?;
        headless.call_tool_payload("export", json!({ "format": "svg", "sheet_id": "wf10-graph", "path": "wf10.svg" }))?;
        if !wd.dir.join("wf10.svg").is_file() {
            return Err("WF-10 svg export missing".to_owned());
        }
        Ok(())
    })?;
    Ok(())
}

#[test]
fn r73_gui_only_wf12_to_wf15_two_way_rm_friedman_and_power_wizard() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wf12_15")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    // WF-12: Two-way ANOVA interaction & Grouped graph
    let grp12 = gui.create_table(json!({
        "sheet_id": "wf12-grp",
        "kind": "Grouped",
        "rows": 2,
        "cols": 2,
        "replicates": 3,
        "auto_graph": true,
        "graph_sheet_id": "wf12-graph"
    }))?;
    gui.ui_invoke("table.setTitles", json!({ "sheet_id": grp12, "titles": ["Veh", "Drug"], "row_titles": ["WT", "KO"] }))?;
    gui.paste_tsv(&grp12, 0, 0, 0, "10\t11\t12\t10\t10\t11\n25\t26\t27\t12\t13\t14")?;
    let anova12 = gui.run_analyze(&grp12, json!({ "entry_id": "stats.factorial_anova" }))?;

    // WF-13: Repeated Measures ANOVA + Sphericity Geisser-Greenhouse
    let col13 = gui.create_table(json!({ "sheet_id": "wf13-rm", "kind": "Column", "rows": 5, "cols": 3 }))?;
    gui.paste_tsv(&col13, 0, 0, 0, "10\t14\t20\n11\t15\t21\n12\t16\t22\n13\t17\t23\n14\t18\t24")?;
    let rm13 = gui.run_analyze(&col13, json!({ "entry_id": "stats.one_way_anova" }))?;
    gui.ui_switch_sheet(&rm13)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    let before_rm = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&before_rm, "edit.sphericity.geisser_greenhouse", false)?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "edit.paired_or_matched" }))?;
    let after_rm = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&after_rm, "edit.sphericity.geisser_greenhouse", true)?;
    gui.ui_answer_ok(json!({}))?;

    // WF-14: Friedman + QOL-7 Diagnostics-Only enforcement (reject applyRemediation!)
    let skew14 = gui.create_table(json!({ "sheet_id": "wf14-skew", "kind": "Column", "rows": 6, "cols": 3 }))?;
    gui.paste_tsv(&skew14, 0, 0, 0, "1\t2\t5\n1.1\t2.1\t5.1\n1.2\t2.2\t5.2\n1.3\t2.3\t5.3\n1.4\t2.4\t5.4\n500\t600\t900")?;
    let rm14 = gui.run_analyze(&skew14, json!({ "entry_id": "stats.one_way_anova" }))?;
    gui.edit_analysis_cmd_t(&rm14, json!({ "paired_or_matched": true }))?;
    let rem_err = gui.ui_invoke_err("ui.result.applyRemediation", json!({ "result_sheet_id": rm14 }))?;
    if !rem_err["message"].as_str().unwrap_or("").contains("diagnostics_only_policy") {
        return Err(format!("WF-14 expected QOL-7 diagnostics_only_policy error, got {rem_err}"));
    }
    gui.ui_switch_sheet(&rm14)?;
    gui.ui_press("t", json!({ "meta": true }))?;
    gui.ui_pointer(json!({ "action": "click", "widget_id": "edit.assume_gaussian" }))?;
    gui.ui_answer_ok(json!({}))?;

    // WF-15: Full 4-Stage Power & Sample Size Wizard via Analyze/Power and Sample Size... + curve hover + create
    gui.ui_invoke_path(&["Analyze", "Power and Sample Size..."], json!({}))?;
    let wiz_s1 = gui.ui_inspect("dialog", false)?;
    assert_widget_enabled(&wiz_s1, "dialog.button.back", false)?;
    assert_widget_enabled(&wiz_s1, "dialog.button.next", true)?;
    assert_widget_enabled(&wiz_s1, "dialog.button.create", false)?;
    let _ = gui.ui_answer_err(json!({ "action": "back" }))?;
    gui.ui_answer_dialog(json!({ "action": "next", "fields": { "family": "unpaired_t", "solve_target": "sample_size", "two_tailed": true } }))?;
    gui.ui_answer_dialog(json!({ "action": "next", "fields": { "effect_mode": "standardized", "d": 0.5 } }))?;
    gui.ui_answer_dialog(json!({ "action": "next", "fields": { "alpha": 0.05, "target_power": 0.80 } }))?;
    let hover_frac = gui.ui_pointer(json!({
        "action": "hover",
        "widget_id": "power.curve_hover",
        "hover_fraction_x": 0.5
    }))?;
    if hover_frac["hover_n"].as_f64().is_none() || hover_frac["hover_power"].as_f64().is_none() {
        return Err(format!("WF-15 hover_fraction_x missing hover_n/hover_power: {hover_frac}"));
    }
    let hover_n = gui.ui_pointer(json!({
        "action": "hover",
        "widget_id": "power.curve_hover",
        "hover_n": 64.0
    }))?;
    if hover_n["hover_power"].as_f64().is_none() {
        return Err(format!("WF-15 hover_n missing hover_power: {hover_n}"));
    }
    gui.ui_type(Some("power.target_power"), "0.90", true, false)?;
    let shot15 = gui.ui_screenshot("dialog", 760, 540)?;
    if shot15.len() < 256 {
        return Err("WF-15 wizard screenshot unexpectedly small".to_owned());
    }
    let pwr_out = gui.ui_answer_dialog(json!({ "action": "create" }))?;
    let pwr_id = pwr_out["result_sheet_id"]
        .as_str()
        .or_else(|| pwr_out["payload"]["sheet_id"].as_str())
        .ok_or_else(|| format!("WF-15 Power wizard missing result_sheet_id: {pwr_out}"))?
        .to_owned();

    let info_id = pwr_out["info_sheet_id"].as_str().ok_or_else(|| format!("WF-15 Power wizard missing info_sheet_id: {pwr_out}"))?.to_owned();

    save_and_verify_headless(&mut gui, &wd.dir, "wf12_15.curvebook", |headless| {
        let rs12 = read_result_json(headless, &anova12)?;
        let _ = result_tab(&rs12, "factorial-anova")?;
        let rs14 = read_result_json(headless, &rm14)?;
        assert_approx(result_scalar_f64(&rs14, "Friedman Q")?, 12.0, 1e-6, "WF-14 Friedman Q")?;
        let rs_pwr = read_result_json(headless, &pwr_id)?;
        if !rs_pwr["title"].as_str().unwrap_or("").contains("Power & Sample Size") {
            return Err(format!("WF-15 unexpected power ResultSheet title: {rs_pwr}"));
        }
        let info_resp = headless.request("resources/read", json!({ "uri": format!("sheet://{info_id}") }))?;
        let info_text =
            info_resp["result"]["contents"][0]["text"].as_str().ok_or_else(|| format!("WF-15 missing InfoSheet resource text: {info_resp}"))?;
        if !info_text.contains("64") || !info_text.contains("128") {
            return Err(format!("WF-15 InfoSheet missing expected sample sizes 64/128: {info_text}"));
        }
        Ok(())
    })?;
    Ok(())
}
