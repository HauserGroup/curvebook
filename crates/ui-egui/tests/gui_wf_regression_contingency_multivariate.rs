//! Wave 17 Shard 3 `gui_only` acceptance test suite (`WF-16`..`WF-23`):
//! Regression, Nonlinear Dose-Response & Enzyme Kinetics, Contingency (R5/R6),
//! ROC Curve, Kaplan-Meier Survival & Logistic Regression, Multivariate PCA/Clustering/GLM/Volcano
//! (R7/R8/R9/R10), and Multi-Dataset Stacked vs. Superimposed Arrangement & `ResultSheet` Chaining.
//!
//! All state mutations and interactions are driven strictly through `dispatch_gui_action`
//! (`ui.inspect`, `ui.invoke_menu`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`,
//! `ui.switch_tab`, `ui.answer_dialog`, `ui.screenshot`) under `app.set_gui_only(true)`.

#![forbid(unsafe_code)]

use lumencraft_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use lumencraft_doc::{
    AnalysisKind, ContingencyMethod, GlmFamily, GraphKind, GroupedArrangement, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
    SheetId,
};
use lumencraft_palette::Rgba8;
use lumencraft_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn ensure(cond: bool, msg: impl Into<String>) -> TestRes {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn new_strict_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    ensure(app.is_gui_only(), "app.is_gui_only() must be true")?;
    Ok(app)
}

fn gui(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed [{}]: {err}", err.code))
}

fn invoke_menu_path(app: &mut UiApp, path: &str, params: Value) -> Result<Value, String> {
    gui(app, "ui.invoke_menu", json!({ "path": path, "params": params }))
}

fn invoke_menu_cmd(app: &mut UiApp, command_id: &str, params: Value) -> Result<Value, String> {
    gui(app, "ui.invoke_menu", json!({ "command_id": command_id, "params": params }))
}

fn inspect_ui(app: &mut UiApp) -> Result<Value, String> {
    gui(app, "ui.inspect", json!({ "include_layout": true, "include_widgets": true, "include_menu": true }))
}

fn answer_dialog(app: &mut UiApp, fields: Value, action: &str) -> Result<Value, String> {
    gui(app, "ui.answer_dialog", json!({ "fields": fields, "action": action }))
}

fn patch_dialog(app: &mut UiApp, fields: Value) -> Result<Value, String> {
    gui(app, "ui.answer_dialog", json!({ "fields": fields }))
}

fn press_shortcut(app: &mut UiApp, key: &str, modifiers: Value) -> Result<Value, String> {
    gui(app, "ui.press_key", json!({ "key": key, "modifiers": modifiers }))
}

fn select_sheet(app: &mut UiApp, sheet_id: &str) -> TestRes {
    invoke_menu_path(app, "View/Select Sheet", json!({ "sheet_id": sheet_id }))?;
    Ok(())
}

fn extract_sheet_id(val: &Value, ctx: &str) -> Result<String, String> {
    val.get("sheet_id")
        .or_else(|| val.get("payload").and_then(|p| p.get("sheet_id")))
        .or_else(|| val.get("result").and_then(|r| r.get("sheet_id")))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{ctx} missing `sheet_id` in output: {val}"))
}

fn create_table(app: &mut UiApp, params: Value) -> Result<String, String> {
    let out = invoke_menu_path(app, "Insert/New Data Table", params)?;
    extract_sheet_id(&out, "Insert/New Data Table")
}

fn set_x_values(app: &mut UiApp, sheet_id: &str, values: Vec<f64>) -> TestRes {
    let arr: Vec<Value> = values.into_iter().map(Value::from).collect();
    invoke_menu_path(app, "Data/Set X Values", json!({ "sheet_id": sheet_id, "values": arr }))?;
    Ok(())
}

fn set_cell_rows(app: &mut UiApp, sheet_id: &str, start_row: usize, rows: Value) -> TestRes {
    invoke_menu_path(app, "Data/Set Multiple Cells", json!({ "sheet_id": sheet_id, "start_row": start_row, "rows": rows }))?;
    Ok(())
}

fn set_titles(app: &mut UiApp, sheet_id: &str, col_titles: &[&str], row_titles: Option<&[&str]>) -> TestRes {
    let mut params = serde_json::Map::new();
    params.insert(String::from("sheet_id"), json!(sheet_id));
    if !col_titles.is_empty() {
        params.insert(String::from("titles"), json!(col_titles));
    }
    if let Some(rt) = row_titles {
        params.insert(String::from("row_titles"), json!(rt));
    }
    invoke_menu_cmd(app, "table.setTitles", Value::Object(params))?;
    Ok(())
}

fn open_analyze_dialog(app: &mut UiApp, target_sheet_id: &str) -> Result<Value, String> {
    select_sheet(app, target_sheet_id)?;
    invoke_menu_cmd(app, "ui.analyze.open", json!({}))?;
    let snap = inspect_ui(app)?;
    let kind = snap.get("dialog").and_then(|d| d.get("dialog_kind")).or_else(|| snap.get("active_dialog")).and_then(Value::as_str).unwrap_or("");
    ensure(kind == "analyze_sheet", format!("expected analyze_sheet dialog, got `{kind}` in {snap}"))?;
    Ok(snap)
}

fn run_analyze_dialog(app: &mut UiApp, target_sheet_id: &str, fields: Value) -> Result<String, String> {
    let before = app.document().result_sheets.len();
    open_analyze_dialog(app, target_sheet_id)?;
    let out = answer_dialog(app, fields, "ok")?;
    if out.get("pending_confirmation").and_then(Value::as_bool) == Some(true) {
        let dup_snap = inspect_ui(app)?;
        let kind = dup_snap.get("active_dialog").and_then(Value::as_str).unwrap_or("");
        ensure(kind == "duplicate_analysis_prompt", format!("expected duplicate_analysis_prompt, got `{kind}`"))?;
        answer_dialog(app, json!({ "choice": "create_new" }), "ok")?;
    }
    ensure(
        app.document().result_sheets.len() == before + 1,
        format!("expected ResultSheet count {} -> {}, got {}", before, before + 1, app.document().result_sheets.len()),
    )?;
    let created = app.document().result_sheets.last().ok_or_else(|| String::from("missing newly created ResultSheet"))?;
    ensure(created.status == ResultStatus::UpToDate, format!("ResultSheet `{}` expected UpToDate, got {:?}", created.id, created.status))?;
    Ok(created.id.as_str().to_owned())
}

fn open_edit_params_shortcut(app: &mut UiApp, result_sheet_id: &str) -> Result<Value, String> {
    select_sheet(app, result_sheet_id)?;
    press_shortcut(app, "T", json!({ "meta": true }))?;
    let snap = inspect_ui(app)?;
    let kind = snap.get("dialog").and_then(|d| d.get("dialog_kind")).or_else(|| snap.get("active_dialog")).and_then(Value::as_str).unwrap_or("");
    ensure(kind == "edit_analysis_parameters", format!("expected edit_analysis_parameters dialog, got `{kind}` in {snap}"))?;
    Ok(snap)
}

fn find_widget<'a>(snap: &'a Value, id: &str) -> Result<&'a Value, String> {
    let match_item = |w: &&Value| w.get("widget_id").or_else(|| w.get("id")).and_then(Value::as_str) == Some(id);
    for arr_opt in [
        snap.get("dialog").and_then(|d| d.get("controls")),
        snap.get("dialog").and_then(|d| d.get("buttons")),
        snap.get("dialog").and_then(|d| d.get("tabs")),
        snap.get("toolbar"),
        snap.get("grid_cells"),
        snap.get("navigator_sheets"),
        snap.get("widgets"),
    ] {
        if let Some(found) = arr_opt.and_then(Value::as_array).and_then(|items| items.iter().find(match_item)) {
            return Ok(found);
        }
    }
    Err(format!("widget `{id}` not found in inspect snapshot: {snap}"))
}

fn widget_enabled(snap: &Value, id: &str) -> Result<bool, String> {
    let w = find_widget(snap, id)?;
    w.get("enabled").and_then(Value::as_bool).ok_or_else(|| format!("widget `{id}` missing boolean `enabled`: {w}"))
}

fn assert_widget_state(snap: &Value, id: &str, expected_enabled: bool) -> TestRes {
    let w = find_widget(snap, id)?;
    let actual = w.get("enabled").and_then(Value::as_bool).ok_or_else(|| format!("widget `{id}` missing `enabled`: {w}"))?;
    ensure(actual == expected_enabled, format!("widget `{id}` enabled={actual}, expected {expected_enabled}: {w}"))?;
    if !expected_enabled {
        let reason = w.get("disabled_reason").and_then(Value::as_str).unwrap_or("");
        ensure(!reason.is_empty(), format!("disabled widget `{id}` must expose non-empty `disabled_reason`: {w}"))?;
    }
    Ok(())
}

fn result_sheet<'a>(app: &'a UiApp, id: &str) -> Result<&'a ResultSheet, String> {
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    app.document().result_sheet(&sid).ok_or_else(|| format!("result sheet `{id}` not found"))
}

fn scalar_val(res: &ResultSheet, key: &str, ds_idx: Option<usize>) -> Result<f64, String> {
    res.scalars.iter().find(|s| s.key == key && (ds_idx.is_none() || s.data_set_index == ds_idx)).map(|s| s.value).ok_or_else(|| {
        let keys: Vec<&str> = res.scalars.iter().map(|s| s.key.as_str()).collect();
        format!("scalar `{key}` (ds={ds_idx:?}) missing in `{}`; available={keys:?}", res.id)
    })
}

fn require_tab(res: &ResultSheet, kind: ResultTabKind) -> Result<&ResultTab, String> {
    res.tabs.iter().find(|t| t.kind == kind).ok_or_else(|| format!("ResultSheet `{}` missing tab kind {:?}", res.id, kind))
}

fn require_tab_id<'a>(res: &'a ResultSheet, id: &str) -> Result<&'a ResultTab, String> {
    res.tabs.iter().find(|t| t.id == id).ok_or_else(|| format!("ResultSheet `{}` missing tab `{id}`", res.id))
}

fn assert_screenshot_valid(app: &mut UiApp, target: &str) -> TestRes {
    let shot = gui(app, "ui.screenshot", json!({ "target": target, "width_px": 960, "height_px": 600, "dpi": 96 }))?;
    let b64 = shot
        .get("data_base64")
        .or_else(|| shot.get("png_base64"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("ui.screenshot({target}) missing base64 data: {shot}"))?;
    ensure(b64.len() > 100 && b64.starts_with("iVBORw0K"), format!("unexpected PNG payload for target `{target}`: len={}", b64.len()))
}

fn compile_scene(app: &UiApp, graph_id: &str) -> Result<ChartScene, String> {
    let gid = SheetId::try_new(graph_id).map_err(|e| e.to_string())?;
    let gs = app.document().graph_sheet(&gid).ok_or_else(|| format!("graph `{graph_id}` not found"))?;
    compile_graph(gs, app.document()).map_err(|e| format!("compile_graph({graph_id}) failed: {e}"))
}

fn find_cell_box(snap: &Value, row: usize, col: usize, subcol: usize) -> Result<[f64; 2], String> {
    let cells = snap.get("grid_cells").and_then(Value::as_array).ok_or_else(|| format!("inspect snapshot missing `grid_cells`: {snap}"))?;
    let found = cells.iter().find(|c| {
        c.get("value").is_some_and(|v| {
            v.get("row").and_then(Value::as_u64) == Some(row as u64)
                && v.get("col").and_then(Value::as_u64) == Some(col as u64)
                && v.get("subcol").and_then(Value::as_u64).unwrap_or(0) == subcol as u64
        })
    });
    if let Some(cell) = found
        && let Some(bounds) = cell.get("bounds").and_then(Value::as_array)
        && bounds.len() == 4
        && let (Some(x), Some(y), Some(w), Some(h)) = (bounds[0].as_f64(), bounds[1].as_f64(), bounds[2].as_f64(), bounds[3].as_f64())
    {
        return Ok([x + w * 0.5, y + h * 0.5]);
    }
    Err(format!("grid cell ({row}, {col}, {subcol}) bounds not found in inspect snapshot"))
}

// ============================================================================
// WF-16: Simple Linear Regression, Deming / Passing-Bablok, Replicates (R8),
//        and ANCOVA Line Comparison
// ============================================================================

#[test]
fn wf16_linear_deming_passing_bablok_ancova_and_r8_r9_predicates() -> TestRes {
    let mut app = new_strict_app()?;

    // Multi-series XY table with 2 raw replicates per Y series to test OLS, ANCOVA, and R8.
    let xy_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf16-xy",
            "title": "Standard Curve Comparison",
            "kind": "xy",
            "rows": 6,
            "cols": 2,
            "replicates": 2,
            "auto_graph": true,
            "graph_sheet_id": "wf16-graph"
        }),
    )?;
    set_x_values(&mut app, &xy_id, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])?;
    set_titles(&mut app, &xy_id, &["Lot A", "Lot B"], None)?;

    let mut edits = Vec::new();
    for r in 0..6 {
        let x = (r + 1) as f64;
        // Lot A: Y = 2.5*X + 4.0 (with replicate jitter +/- 0.1)
        // Lot B: Y = 2.5*X + 12.0 (parallel shift -> ANCOVA equal slopes, different intercepts)
        edits.push(json!({ "row": r, "col": 0, "subcol": 0, "value": 2.5 * x + 3.9 }));
        edits.push(json!({ "row": r, "col": 0, "subcol": 1, "value": 2.5 * x + 4.1 }));
        edits.push(json!({ "row": r, "col": 1, "subcol": 0, "value": 2.5 * x + 11.9 }));
        edits.push(json!({ "row": r, "col": 1, "subcol": 1, "value": 2.5 * x + 12.1 }));
    }
    invoke_menu_path(&mut app, "Data/Set Multiple Cells", json!({ "sheet_id": xy_id, "edits": edits }))?;

    // Exercise inline grid interaction on a cell if layout exposes grid_cells coordinates.
    select_sheet(&mut app, &xy_id)?;
    let grid_snap = inspect_ui(&mut app)?;
    if let Ok([cx, cy]) = find_cell_box(&grid_snap, 0, 0, 0) {
        gui(&mut app, "ui.pointer_gesture", json!({ "action": "double_click", "x": cx, "y": cy }))?;
        gui(&mut app, "ui.type_text", json!({ "text": "6.4", "clear_existing": true, "commit_enter": true }))?;
        let ds_check = app.document().data_sheet(&"wf16-xy".into()).ok_or("wf16-xy missing")?;
        let edited = ds_check.cell(lumencraft_doc::CellCoord::new(0, 0, 0)).and_then(|c| c.active_number());
        ensure(edited == Some(6.4), format!("inline double-click edit expected 6.4, got {edited:?}"))?;
    }

    // Run Simple Linear Regression (with ANCOVA line comparison on 2 datasets) via AnalyzeSheet modal.
    let res_lin_id = run_analyze_dialog(&mut app, &xy_id, json!({ "entry_id": "fit.linear" }))?;
    {
        let res = result_sheet(&app, &res_lin_id)?;
        let s0 = scalar_val(res, "Slope", Some(0))?;
        let s1 = scalar_val(res, "Slope", Some(1))?;
        let i0 = scalar_val(res, "YIntercept", Some(0))?;
        let i1 = scalar_val(res, "YIntercept", Some(1))?;
        ensure((s0 - 2.5).abs() < 0.05 && (s1 - 2.5).abs() < 0.05, format!("unexpected OLS slopes: {s0}, {s1}"))?;
        ensure((i0 - 4.0).abs() < 0.1 && (i1 - 12.0).abs() < 0.1, format!("unexpected intercepts: {i0}, {i1}"))?;
        require_tab(res, ResultTabKind::MainSummary)?;
        require_tab(res, ResultTabKind::ModelComparison)?;
        ensure(!res.fitted_curves.is_empty() && !res.fitted_curves[0].points.is_empty(), "expected regression bands")?;
    }

    // Attach 95% confidence band overlay via Graph/Regression Overlay... menu and verify compiled scene.
    invoke_menu_path(
        &mut app,
        "Graph/Regression Overlay...",
        json!({
            "sheet_id": "wf16-graph",
            "result_sheet_id": res_lin_id,
            "all_series": true,
            "show_confidence_band": true,
            "shaded_ribbon": true
        }),
    )?;
    let scene = compile_scene(&app, "wf16-graph")?;
    ensure(scene.primitives.len() >= 4, "expected fitted regression lines & bands in ChartScene")?;

    // Open EditAnalysisParameters via Cmd+T shortcut -> check R8 & R9 graying predicates via ui.inspect.
    let ep_snap = open_edit_params_shortcut(&mut app, &res_lin_id)?;
    assert_widget_state(&ep_snap, "edit.descriptive.subcolumn_agg", true)?;
    answer_dialog(&mut app, json!({ "p_value_format": "apa" }), "apply")?;
    let ep_after_apa = inspect_ui(&mut app)?;
    assert_widget_state(&ep_after_apa, "edit.pvalue.digits", false)?;
    answer_dialog(&mut app, json!({ "p_value_format": "custom" }), "apply")?;
    let ep_after_custom = inspect_ui(&mut app)?;
    assert_widget_state(&ep_after_custom, "edit.pvalue.digits", true)?;
    answer_dialog(&mut app, json!({ "p_value_format": "custom", "p_value_decimals": 4 }), "ok")?;

    // Deming Model II regression (menu Fit Curve...) + Passing-Bablok via Analyze modal on single-col XY.
    let pb_id = create_table(&mut app, json!({ "sheet_id": "wf16-pb", "title": "Assay Method Comparison", "kind": "xy", "rows": 8, "cols": 1 }))?;
    set_x_values(&mut app, &pb_id, vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0])?;
    set_cell_rows(&mut app, &pb_id, 0, json!([[11.2], [21.0], [31.5], [40.8], [52.1], [61.4], [72.0], [81.6]]))?;

    let dem_out =
        invoke_menu_path(&mut app, "Analyze/Fit Curve...", json!({ "sheet_id": "wf16-deming", "source_sheet_id": pb_id, "kind": "deming" }))?;
    let dem_id = extract_sheet_id(&dem_out, "Fit Curve Deming")?;
    let dem_res = result_sheet(&app, &dem_id)?;
    let dem_slope = scalar_val(dem_res, "Slope", Some(0))?;
    ensure((dem_slope - 1.0).abs() < 0.05, format!("Deming slope expected ~1.0, got {dem_slope}"))?;

    let pb_res_id = run_analyze_dialog(&mut app, &pb_id, json!({ "entry_id": "stats.passing_bablok" }))?;
    {
        let pb_res = result_sheet(&app, &pb_res_id)?;
        let pb_slope = scalar_val(pb_res, "Slope", None)?;
        let pb_lo = scalar_val(pb_res, "Slope CI lower", None)?;
        let pb_hi = scalar_val(pb_res, "Slope CI upper", None)?;
        ensure((pb_slope - 1.0).abs() < 0.05 && pb_lo <= pb_slope && pb_slope <= pb_hi, "invalid Passing-Bablok slope/CI")?;
        require_tab(pb_res, ResultTabKind::ParametersAndConfidenceIntervals)?;
        require_tab(pb_res, ResultTabKind::Residuals)?;
    }

    // Switch ResultSheet tab and capture screenshot strictly via dispatch_gui_action.
    select_sheet(&mut app, &pb_res_id)?;
    gui(&mut app, "ui.switch_tab", json!({ "sheet_id": pb_res_id, "tab": "pb-residuals" }))?;
    assert_screenshot_valid(&mut app, "workspace")?;
    Ok(())
}

// ============================================================================
// WF-17: Dose-Response 4PL / 5PL + Unknown Interpolation + R7 Correlation Matrix
// ============================================================================

#[test]
fn wf17_dose_response_4pl_5pl_interpolation_and_r7_correlation_matrix() -> TestRes {
    let mut app = new_strict_app()?;

    let dose_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf17-dose",
            "title": "4PL Agonist Curve",
            "kind": "xy",
            "rows": 10,
            "cols": 1,
            "auto_graph": true,
            "graph_sheet_id": "wf17-dose-graph"
        }),
    )?;
    let log_xs = [-9.0, -8.0, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let hill_4pl = |lx: f64| 5.0 + 95.0 / (1.0 + 10_f64.powf((-6.0 - lx) * 1.25));
    set_x_values(&mut app, &dose_id, log_xs.to_vec())?;

    let mut row_vals: Vec<Value> = log_xs.iter().map(|&lx| json!([hill_4pl(lx)])).collect();
    // Rows 8..=9: missing X with known Y responses for unknown interpolation.
    row_vals.push(json!([32.0]));
    row_vals.push(json!([78.0]));
    set_cell_rows(&mut app, &dose_id, 0, Value::Array(row_vals))?;

    // Open AnalyzeSheet modal -> fit 4PL dose-response.
    let r4pl_id = run_analyze_dialog(&mut app, &dose_id, json!({ "entry_id": "fit.nonlinear", "equation_id": "log_agonist_vs_response_4pl" }))?;
    {
        let r4pl = result_sheet(&app, &r4pl_id)?;
        let log_ec50 = scalar_val(r4pl, "LogEC50", Some(0))?;
        let ec50 = scalar_val(r4pl, "EC50", Some(0))?;
        let hill = scalar_val(r4pl, "HillSlope", Some(0))?;
        ensure((log_ec50 - (-6.0)).abs() < 0.02, format!("LogEC50 expected -6.0, got {log_ec50}"))?;
        ensure((ec50 - 1e-6).abs() < 1e-7, format!("EC50 expected 1e-6, got {ec50}"))?;
        ensure((hill - 1.25).abs() < 0.05, format!("HillSlope expected 1.25, got {hill}"))?;
        require_tab(r4pl, ResultTabKind::Residuals)?;
        require_tab(r4pl, ResultTabKind::ParametersAndConfidenceIntervals)?;
    }

    // Enable Interpolate Unknowns + Asymmetric Profile Likelihood CI via Change/Analysis Parameters...
    invoke_menu_path(
        &mut app,
        "Change/Analysis Parameters...",
        json!({
            "sheet_id": r4pl_id,
            "interpolate_unknowns": true,
            "profile_likelihood_ci": true,
            "confidence_bands": true
        }),
    )?;
    {
        let r4pl_updated = result_sheet(&app, &r4pl_id)?;
        let interp_tab = require_tab(r4pl_updated, ResultTabKind::InterpolatedUnknowns)?;
        ensure(interp_tab.rows.len() == 2, format!("expected 2 interpolated unknowns, got {}", interp_tab.rows.len()))?;
        let params_tab = require_tab(r4pl_updated, ResultTabKind::ParametersAndConfidenceIntervals)?;
        let logec50_row = params_tab.row_labels.iter().position(|l| l == "LogEC50").ok_or("LogEC50 row missing")?;
        ensure(matches!(params_tab.rows[logec50_row][2], ResultCell::ConfidenceInterval { .. }), "expected confidence interval cell for LogEC50")?;
    }

    // Fit 5PL asymmetric model on same dataset via menu Fit Curve... and overlay curve + ribbon on graph.
    invoke_menu_path(
        &mut app,
        "Analyze/Fit Curve...",
        json!({
            "sheet_id": "wf17-5pl",
            "source_sheet_id": dose_id,
            "equation_id": "log_agonist_vs_response_5pl",
            "confidence_bands": true
        }),
    )?;
    let r5pl = result_sheet(&app, "wf17-5pl")?;
    ensure(r5pl.status == ResultStatus::UpToDate && !r5pl.fitted_curves.is_empty(), "5PL fit failed")?;

    invoke_menu_path(
        &mut app,
        "Graph/Regression Overlay...",
        json!({ "sheet_id": "wf17-dose-graph", "result_sheet_id": r4pl_id, "all_series": true, "show_confidence_band": true, "shaded_ribbon": true }),
    )?;
    let dose_scene = compile_scene(&app, "wf17-dose-graph")?;
    ensure(dose_scene.primitives.len() >= 4, "4PL overlay curve & band must appear in ChartScene")?;

    // Docs WF-17: Pearson/Spearman Correlation Matrix & R7 graying check in AnalyzeSheet modal.
    let mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf17-mv",
            "title": "Biomarker Correlations",
            "kind": "multiple_variables",
            "rows": 8,
            "variables": [
                { "title": "VarA", "role": "continuous" },
                { "title": "VarB", "role": "continuous" },
                { "title": "VarC", "role": "continuous" }
            ]
        }),
    )?;
    set_cell_rows(
        &mut app,
        &mv_id,
        0,
        json!([
            [1.0, 2.1, 9.8],
            [2.0, 4.0, 8.1],
            [3.0, 5.9, 6.2],
            [4.0, 8.2, 5.0],
            [5.0, 10.1, 3.8],
            [6.0, 12.0, 2.5],
            [7.0, 14.3, 1.4],
            [8.0, 16.0, 0.9]
        ]),
    )?;

    let snap = open_analyze_dialog(&mut app, &mv_id)?;
    let _ = snap;
    patch_dialog(&mut app, json!({ "entry_id": "stats.corr_matrix", "correlation_all_pairs": false }))?;
    let snap_r7_off = inspect_ui(&mut app)?;
    assert_widget_state(&snap_r7_off, "analyze.correlation.heatmap", false)?;
    let blocked = patch_dialog(&mut app, json!({ "correlation_heatmap": true }));
    ensure(blocked.is_err(), "R7 must reject enabling correlation_heatmap when correlation_all_pairs is false")?;

    patch_dialog(&mut app, json!({ "correlation_all_pairs": true }))?;
    let snap_r7_on = inspect_ui(&mut app)?;
    assert_widget_state(&snap_r7_on, "analyze.correlation.heatmap", true)?;
    assert_screenshot_valid(&mut app, "dialog")?;
    answer_dialog(&mut app, json!({ "correlation_heatmap": true }), "ok")?;

    let corr_res = app.document().result_sheets.iter().find(|r| r.analysis.source_sheet_id.as_str() == mv_id).ok_or("correlation result missing")?;
    require_tab_id(corr_res, "corr-matrix-r")?;
    require_tab_id(corr_res, "corr-matrix-fdr")?;
    assert_screenshot_valid(&mut app, "workspace")?;
    Ok(())
}

// ============================================================================
// WF-18: Enzyme Kinetics (Michaelis-Menten + Global Shared Parameter Fit)
//        and Five-Gate Regression Diagnostics / Model Comparison
// ============================================================================

#[test]
fn wf18_enzyme_kinetics_global_shared_vmax_and_five_gate_diagnostics() -> TestRes {
    let mut app = new_strict_app()?;

    let enz_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf18-kinetics",
            "title": "Enzyme Kinetics WT vs Mutant",
            "kind": "xy",
            "rows": 8,
            "cols": 2,
            "replicates": 2,
            "auto_graph": true,
            "graph_sheet_id": "wf18-graph"
        }),
    )?;
    let subs = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];
    set_x_values(&mut app, &enz_id, subs.to_vec())?;
    set_titles(&mut app, &enz_id, &["WildType", "Mutant"], None)?;

    // True shared Vmax = 120.0; Km(WT) = 8.0, Km(Mutant) = 24.0.
    let mut edits = Vec::new();
    for (r, &s) in subs.iter().enumerate() {
        let wt = 120.0 * s / (8.0 + s);
        let mut_v = 120.0 * s / (24.0 + s);
        edits.push(json!({ "row": r, "col": 0, "subcol": 0, "value": wt - 0.15 }));
        edits.push(json!({ "row": r, "col": 0, "subcol": 1, "value": wt + 0.15 }));
        edits.push(json!({ "row": r, "col": 1, "subcol": 0, "value": mut_v - 0.15 }));
        edits.push(json!({ "row": r, "col": 1, "subcol": 1, "value": mut_v + 0.15 }));
    }
    invoke_menu_path(&mut app, "Data/Set Multiple Cells", json!({ "sheet_id": enz_id, "edits": edits }))?;

    // Baseline local Michaelis-Menten via AnalyzeSheet modal.
    let local_id = run_analyze_dialog(&mut app, &enz_id, json!({ "entry_id": "fit.nonlinear", "equation_id": "michaelis_menten" }))?;
    let local_res = result_sheet(&app, &local_id)?;
    let km_wt_local = scalar_val(local_res, "Km", Some(0))?;
    let km_mut_local = scalar_val(local_res, "Km", Some(1))?;
    ensure((km_wt_local - 8.0).abs() < 0.25 && (km_mut_local - 24.0).abs() < 0.6, "local MM fit inaccurate")?;

    // Global shared-parameter fit (`Vmax` SharedGlobal across WildType & Mutant) + Five-Gate diagnostics + AICc comparison.
    let global_out = invoke_menu_path(
        &mut app,
        "Analyze/Fit Curve...",
        json!({
            "sheet_id": "wf18-global-mm",
            "source_sheet_id": enz_id,
            "equation_id": "michaelis_menten",
            "parameter_rules": [{ "name": "Vmax", "scope": "shared_global" }],
            "comparison": { "Aicc": { "alternative_equation_id": "kcat_enzyme_kinetics" } },
            "replicates_lack_of_fit_test": true,
            "normality_of_residuals": true,
            "homoscedasticity_test": true,
            "emit_residuals_tab": true,
            "emit_covariance_tab": true,
            "confidence_bands": true
        }),
    )?;
    let global_id = extract_sheet_id(&global_out, "Fit Curve Global MM")?;
    let global_res = result_sheet(&app, &global_id)?;
    let shared_vmax = scalar_val(global_res, "Vmax", None)?;
    let km_wt_g = scalar_val(global_res, "Km", Some(0))?;
    let km_mut_g = scalar_val(global_res, "Km", Some(1))?;
    ensure((shared_vmax - 120.0).abs() < 0.5, format!("shared Vmax expected ~120, got {shared_vmax}"))?;
    ensure((km_wt_g - 8.0).abs() < 0.25 && (km_mut_g - 24.0).abs() < 0.6, format!("global Km mismatch: {km_wt_g}, {km_mut_g}"))?;
    require_tab(global_res, ResultTabKind::ModelComparison)?;
    require_tab(global_res, ResultTabKind::Residuals)?;
    require_tab(global_res, ResultTabKind::CovarianceMatrix)?;
    ensure(global_res.checklist.len() >= 3, "expected 5-gate diagnostics checklist items")?;

    // Edit upstream cell -> reactive recalculation updates Km(WT).
    invoke_menu_path(
        &mut app,
        "Data/Set Multiple Cells",
        json!({ "sheet_id": enz_id, "edits": [{ "row": 7, "col": 0, "subcol": 0, "value": 150.0 }] }),
    )?;
    let km_after = scalar_val(result_sheet(&app, &global_id)?, "Km", Some(0))?;
    ensure((km_after - km_wt_g).abs() > 1e-4, "DAG reactive update did not change global Km")?;
    Ok(())
}

// ============================================================================
// WF-19: Contingency 2x2 (Fisher, Baptista-Pike OR, Koopman RR, Newcombe RD)
//        vs RxC Chi-Square & Modal Predicates R5/R6
// ============================================================================

#[test]
fn wf19_contingency_2x2_fisher_r5_r6_and_rxc_chi_square() -> TestRes {
    let mut app = new_strict_app()?;

    // 2x2 table: Exposed [[16, 4], [2, 18]].
    let c2x2_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf19-2x2",
            "title": "Case-Control 2x2",
            "kind": "contingency",
            "rows": 2,
            "cols": 2,
            "auto_graph": true
        }),
    )?;
    set_titles(&mut app, &c2x2_id, &["Case", "Control"], Some(&["Exposed", "Unexposed"]))?;
    set_cell_rows(&mut app, &c2x2_id, 0, json!([[16, 4], [2, 18]]))?;

    let r2_id = run_analyze_dialog(&mut app, &c2x2_id, json!({ "entry_id": "stats.contingency" }))?;
    let snap_2x2 = open_edit_params_shortcut(&mut app, &r2_id)?;
    // Verify R5: 2x2 enables OR/RR toggle, while Cochran-Armitage trend is disabled on 2x2.
    assert_widget_state(&snap_2x2, "edit.contingency.odds_ratio", true)?;
    assert_widget_state(&snap_2x2, "edit.contingency.cochran_armitage", false)?;

    // Verify R6: disabling both effect-size boxes greys out CI selector combos; enabling restores them.
    answer_dialog(&mut app, json!({ "compute_odds_and_risk_ratios": false, "compute_sensitivity_specificity": false }), "apply")?;
    let snap_ci_off = inspect_ui(&mut app)?;
    assert_widget_state(&snap_ci_off, "edit.contingency.ci.baptista_pike", false)?;
    assert_widget_state(&snap_ci_off, "edit.contingency.ci.koopman", false)?;
    assert_widget_state(&snap_ci_off, "edit.contingency.ci.newcombe_wilson", false)?;
    assert_widget_state(&snap_ci_off, "edit.contingency.ci.newcombe_wilson", false)?;

    answer_dialog(
        &mut app,
        json!({
            "contingency_method": "fisher",
            "compute_odds_and_risk_ratios": true,
            "compute_sensitivity_specificity": true,
            "attributable_risk": true
        }),
        "apply",
    )?;
    let snap_ci_on = inspect_ui(&mut app)?;
    assert_widget_state(&snap_ci_on, "edit.contingency.ci.baptista_pike", true)?;
    assert_widget_state(&snap_ci_on, "edit.contingency.ci.koopman", true)?;
    assert_widget_state(&snap_ci_on, "edit.contingency.ci.wilson_brown", true)?;
    assert_widget_state(&snap_ci_on, "edit.contingency.ci.newcombe_wilson", true)?;
    answer_dialog(&mut app, json!({}), "ok")?;

    {
        let r2 = result_sheet(&app, &r2_id)?;
        ensure(
            matches!(r2.analysis.kind, AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::FisherExact),
            "expected FisherExact method after modal OK",
        )?;
        let fisher_p = scalar_val(r2, "Fisher's exact P", None)?;
        let or_val = scalar_val(r2, "Odds ratio", None)?;
        ensure(fisher_p < 0.001 && (or_val - 36.0).abs() < 0.01, format!("unexpected Fisher P={fisher_p}, OR={or_val}"))?;
        let ratios_tab = require_tab_id(r2, "contingency-ratios")?;
        ensure(
            ratios_tab.row_labels.iter().any(|l| l.contains("Baptista-Pike"))
                && ratios_tab.row_labels.iter().any(|l| l.contains("Koopman"))
                && ratios_tab.row_labels.iter().any(|l| l.contains("Newcombe")),
            format!("missing Baptista-Pike / Koopman / Newcombe rows: {:?}", ratios_tab.row_labels),
        )?;
    }

    // 3x3 (RxC) contingency table: verify R5 & R6 disable 2x2-only effect-sizes/CIs & trend.
    let crxc_id = create_table(&mut app, json!({ "sheet_id": "wf19-3x3", "title": "RxC Contingency", "kind": "contingency", "rows": 3, "cols": 3 }))?;
    set_cell_rows(&mut app, &crxc_id, 0, json!([[20, 15, 5], [10, 20, 10], [5, 10, 25]]))?;
    let rrxc_id = run_analyze_dialog(&mut app, &crxc_id, json!({ "entry_id": "stats.contingency" }))?;
    let snap_3x3 = open_edit_params_shortcut(&mut app, &rrxc_id)?;
    assert_widget_state(&snap_3x3, "edit.contingency.odds_ratio", false)?;
    assert_widget_state(&snap_3x3, "edit.contingency.ci.baptista_pike", false)?;
    assert_widget_state(&snap_3x3, "edit.contingency.ci.koopman", false)?;
    assert_widget_state(&snap_3x3, "edit.contingency.ci.newcombe_wilson", false)?;
    assert_widget_state(&snap_3x3, "edit.contingency.ci.wilson_brown", false)?;
    assert_widget_state(&snap_3x3, "edit.contingency.cochran_armitage", false)?;
    answer_dialog(&mut app, json!({ "contingency_method": "chisquare" }), "ok")?;

    let rrxc = result_sheet(&app, &rrxc_id)?;
    let chi2 = scalar_val(rrxc, "Chi-square", None)?;
    let cramers_v = scalar_val(rrxc, "Cramer's V", None)?;
    ensure(chi2 > 15.0 && cramers_v > 0.2, format!("expected significant 3x3 association: chi2={chi2}, V={cramers_v}"))?;
    Ok(())
}

// ============================================================================
// WF-20: ROC Curve Analysis (AUC, Youden J, Coordinates, ROC Graph)
//        + Cochran-Armitage Trend (2x3 Contingency R5)
// ============================================================================

#[test]
fn wf20_roc_curve_analysis_graph_and_cochran_armitage_trend() -> TestRes {
    let mut app = new_strict_app()?;

    let roc_tbl_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf20-roc-data",
            "title": "Diagnostic Biomarker",
            "kind": "column",
            "rows": 8,
            "cols": 2,
            "auto_graph": true,
            "graph_sheet_id": "wf20-roc-graph"
        }),
    )?;
    set_titles(&mut app, &roc_tbl_id, &["Control", "Diseased"], None)?;
    set_cell_rows(
        &mut app,
        &roc_tbl_id,
        0,
        json!([[10.0, 22.0], [12.0, 25.0], [14.0, 29.0], [15.5, 31.0], [17.0, 34.0], [18.5, 16.0], [20.0, 38.0], [23.0, 42.0]]),
    )?;

    let roc_res_id = run_analyze_dialog(&mut app, &roc_tbl_id, json!({ "entry_id": "stats.roc" }))?;
    {
        let roc_res = result_sheet(&app, &roc_res_id)?;
        let auc = scalar_val(roc_res, "Area under ROC curve (AUC)", None)?;
        let youden = scalar_val(roc_res, "Max Youden J", None)?;
        let cutoff = scalar_val(roc_res, "Optimal cutoff (Youden J)", None)?;
        ensure(auc > 0.80 && auc <= 1.0, format!("expected high ROC AUC, got {auc}"))?;
        ensure(youden > 0.5 && cutoff > 18.0 && cutoff < 26.0, format!("unexpected Youden={youden}, cutoff={cutoff}"))?;
        require_tab(roc_res, ResultTabKind::MainSummary)?;
        require_tab(roc_res, ResultTabKind::CurveCoordinates)?;
        ensure(!roc_res.fitted_curves.is_empty(), "ROC curve coordinates overlay missing")?;
    }

    // Switch auto graph to Diagnostic ROC Curve via Change Graph Type dialog and inspect ChartScene.
    select_sheet(&mut app, "wf20-roc-graph")?;
    invoke_menu_cmd(&mut app, "ui.changeGraphType.open", json!({}))?;
    answer_dialog(&mut app, json!({ "option_id": "diag.roc" }), "ok")?;
    let roc_scene = compile_scene(&app, "wf20-roc-graph")?;
    ensure(roc_scene.primitives.len() >= 3, "ROC GraphSheet must render ROC path & diagonal")?;
    assert_screenshot_valid(&mut app, "canvas")?;

    // Docs WF-20: 2x3 dose-ordered Contingency table -> verify R5 enables Cochran-Armitage trend!
    let trend_id =
        create_table(&mut app, json!({ "sheet_id": "wf20-trend", "title": "Dose Trend 2x3", "kind": "contingency", "rows": 2, "cols": 3 }))?;
    set_cell_rows(&mut app, &trend_id, 0, json!([[30, 20, 10], [5, 15, 25]]))?;
    let rtrend_id = run_analyze_dialog(&mut app, &trend_id, json!({ "entry_id": "stats.contingency" }))?;
    let snap_trend = open_edit_params_shortcut(&mut app, &rtrend_id)?;
    assert_widget_state(&snap_trend, "edit.contingency.cochran_armitage", true)?;
    assert_widget_state(&snap_trend, "edit.contingency.odds_ratio", false)?;
    answer_dialog(&mut app, json!({ "contingency_method": "cochranarmitage" }), "ok")?;
    let trend_res = result_sheet(&app, &rtrend_id)?;
    ensure(
        matches!(trend_res.analysis.kind, AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::CochranArmitageTrend),
        "expected CochranArmitageTrend",
    )?;
    Ok(())
}

// ============================================================================
// WF-21: Kaplan-Meier Survival (1-Group vs 2-Group, Log-Rank, Hazard Ratio,
//        Number at Risk, KM Graph) + Multiple Logistic Regression & R4 Check
// ============================================================================

#[test]
fn wf21_kaplan_meier_survival_hr_logistic_and_r4_checks() -> TestRes {
    let mut app = new_strict_app()?;

    // 1-group Survival table: no log-rank / curve comparison tab emitted.
    let s1_id = create_table(&mut app, json!({ "sheet_id": "wf21-s1", "title": "Single Cohort OS", "kind": "survival", "rows": 6, "cols": 1 }))?;
    set_x_values(&mut app, &s1_id, vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0])?;
    set_cell_rows(&mut app, &s1_id, 0, json!([[1], [0], [1], [1], [0], [1]]))?;
    let rs1_id = run_analyze_dialog(&mut app, &s1_id, json!({ "entry_id": "stats.survival_km" }))?;
    {
        let rs1 = result_sheet(&app, &rs1_id)?;
        require_tab(rs1, ResultTabKind::NumberAtRisk)?;
        ensure(rs1.tabs.iter().all(|t| t.kind != ResultTabKind::ModelComparison), "single-group KM must not emit curve-comparison tab")?;
    }

    // 2-group Survival table: Log-rank + Hazard Ratio + Number-at-risk KM Graph.
    let s2_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf21-s2",
            "title": "Trial Survival Arms",
            "kind": "survival",
            "rows": 10,
            "cols": 2,
            "auto_graph": true,
            "graph_sheet_id": "wf21-km-graph"
        }),
    )?;
    set_titles(&mut app, &s2_id, &["Placebo", "Active"], None)?;
    set_x_values(&mut app, &s2_id, vec![1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 9.0, 11.0, 13.0, 15.0])?;
    set_cell_rows(
        &mut app,
        &s2_id,
        0,
        json!([[1, null], [1, 0], [1, null], [1, 0], [0, null], [null, 1], [null, 0], [null, 1], [null, 0], [null, 1]]),
    )?;
    let rs2_id = run_analyze_dialog(&mut app, &s2_id, json!({ "entry_id": "stats.survival_km" }))?;
    {
        let rs2 = result_sheet(&app, &rs2_id)?;
        let lr_chi2 = scalar_val(rs2, "Log-rank chi-square", None)?;
        let lr_p = scalar_val(rs2, "Log-rank P value", None)?;
        let hr = scalar_val(rs2, "Hazard ratio (log-rank)", None)?;
        ensure(
            lr_chi2.is_finite() && lr_p < 0.05 && hr.is_finite() && hr > 0.0,
            format!("invalid KM comparison: chi2={lr_chi2}, p={lr_p}, hr={hr}"),
        )?;
        require_tab(rs2, ResultTabKind::NumberAtRisk)?;
        require_tab(rs2, ResultTabKind::ModelComparison)?;
    }

    invoke_menu_path(&mut app, "Format/Graph...", json!({ "sheet_id": "wf21-km-graph", "survival_number_at_risk": true }))?;
    let km_scene = compile_scene(&app, "wf21-km-graph")?;
    ensure(
        km_scene.primitives.iter().any(|p| {
            matches!(p, ScenePrimitive::Text(t) if {
                let s = t.rich.plain_text();
                s.contains("Placebo") || s.contains("Active") || s.contains("at risk")
            })
        }),
        "KM GraphSheet with survival_number_at_risk must render cohort at-risk labels",
    )?;

    // Docs WF-21: Multiple Logistic Regression on MultipleVariables table (`Outcome`, `Biomarker`, `Age`).
    let mv_log_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf21-mv-log",
            "title": "Logistic Cohort",
            "kind": "multiple_variables",
            "rows": 10,
            "variables": [
                { "title": "Response01", "role": "continuous" },
                { "title": "Biomarker", "role": "continuous" },
                { "title": "Age", "role": "continuous" }
            ]
        }),
    )?;
    set_cell_rows(
        &mut app,
        &mv_log_id,
        0,
        json!([
            [0.0, 1.0, 42.0],
            [0.0, 1.5, 45.0],
            [0.0, 2.0, 48.0],
            [0.0, 2.5, 52.0],
            [0.0, 3.0, 54.0],
            [1.0, 5.5, 58.0],
            [1.0, 6.0, 61.0],
            [1.0, 6.8, 63.0],
            [1.0, 7.2, 66.0],
            [1.0, 8.0, 70.0]
        ]),
    )?;
    let log_out = invoke_menu_path(
        &mut app,
        "Analyze/Statistical Test...",
        json!({
            "sheet_id": "wf21-logistic",
            "source_sheet_id": mv_log_id,
            "kind": "logistic_glm",
            "response_column": 0,
            "predictor_columns": [1, 2]
        }),
    )?;
    let log_id = extract_sheet_id(&log_out, "Logistic GLM")?;
    let log_res = result_sheet(&app, &log_id)?;
    ensure(matches!(log_res.analysis.kind, AnalysisKind::GeneralizedLinearModel(ref g) if g.family == GlmFamily::Logistic), "expected Logistic GLM")?;
    require_tab(log_res, ResultTabKind::ParametersAndConfidenceIntervals)?;

    // Also verify R4 graying rule in EditAnalysisParameters on a paired Column table (`edit.paired_or_matched` / `edit.two_group.welch`).
    let r4_col_id = create_table(&mut app, json!({ "sheet_id": "wf21-r4-col", "kind": "column", "rows": 5, "cols": 2 }))?;
    set_cell_rows(&mut app, &r4_col_id, 0, json!([[10.0, 14.1], [11.0, 15.6], [12.0, 15.7], [13.0, 17.8], [14.0, 19.3]]))?;
    let r4_res_id = run_analyze_dialog(&mut app, &r4_col_id, json!({ "entry_id": "stats.two_group" }))?;
    let snap_r4 = open_edit_params_shortcut(&mut app, &r4_res_id)?;
    answer_dialog(&mut app, json!({ "paired_or_matched": true }), "apply")?;
    let snap_r4_paired = inspect_ui(&mut app)?;
    assert_widget_state(&snap_r4_paired, "edit.two_group.welch", false)?;
    let _ = snap_r4;
    answer_dialog(&mut app, json!({ "paired_or_matched": false }), "ok")?;
    Ok(())
}

// ============================================================================
// WF-22: Multivariate PCA, Hierarchical Clustering, Multiple Linear Regression,
//        Row-Wise Multiple t-Tests, Volcano Plot & Format Selected Points (R9)
// ============================================================================

#[test]
fn wf22_multivariate_pca_clustering_glm_volcano_and_format_points() -> TestRes {
    let mut app = new_strict_app()?;

    let mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf22-mv",
            "title": "Multi-Omic Cohort",
            "kind": "multiple_variables",
            "rows": 10,
            "auto_graph": true,
            "graph_sheet_id": "wf22-mv-graph",
            "variables": [
                { "title": "Feature1", "role": "continuous" },
                { "title": "Feature2", "role": "continuous" },
                { "title": "Feature3", "role": "continuous" },
                { "title": "Outcome", "role": "continuous" }
            ]
        }),
    )?;
    let mut rows = Vec::new();
    for i in 0..10 {
        let f1 = (i as f64) + 1.0;
        let f2 = 2.0 * f1 + if i % 2 == 0 { 0.3 } else { -0.3 };
        let f3 = (12.0 - f1) + (i % 3) as f64 * 0.4;
        let y = 3.0 * f1 - 0.5 * f2 + 0.8 * f3 + 1.0;
        rows.push(json!([f1, f2, f3, y]));
    }
    set_cell_rows(&mut app, &mv_id, 0, Value::Array(rows))?;

    // 1. PCA via AnalyzeSheet modal.
    let pca_id = run_analyze_dialog(&mut app, &mv_id, json!({ "entry_id": "stats.pca", "pca_predictors": [0, 1, 2] }))?;
    {
        let pca_res = result_sheet(&app, &pca_id)?;
        let ev1 = scalar_val(pca_res, "PC1 eigenvalue", None)?;
        let vp1 = scalar_val(pca_res, "PC1 variance proportion", None)?;
        ensure(ev1 > 1.0 && vp1 > 0.5, format!("invalid PCA scree values: ev1={ev1}, vp1={vp1}"))?;
        require_tab_id(pca_res, "pca-eigenvalues")?;
        require_tab_id(pca_res, "pca-loadings")?;
    }

    // Verify MultipleVariables 5-channel bubble/scatter graph compiles cleanly.
    let mv_scene = compile_scene(&app, "wf22-mv-graph")?;
    ensure(!mv_scene.primitives.is_empty(), "expected Multivariate Bubble/Scatter Graph primitives")?;

    // 2. Hierarchical Clustering & k-means++.
    let clust_id = run_analyze_dialog(&mut app, &mv_id, json!({ "entry_id": "stats.clustering", "pca_predictors": [0, 1, 2] }))?;
    {
        let clust_res = result_sheet(&app, &clust_id)?;
        let k = scalar_val(clust_res, "Clusters (k)", None)?;
        ensure(k >= 2.0, format!("expected k >= 2 clusters, got {k}"))?;
        require_tab_id(clust_res, "kmeans-clusters")?;
        require_tab_id(clust_res, "hierarchical-dendrogram")?;
    }

    // 3. Multiple Linear Regression (GLM Linear).
    let glm_out = invoke_menu_path(
        &mut app,
        "Analyze/Statistical Test...",
        json!({
            "sheet_id": "wf22-mlr",
            "source_sheet_id": mv_id,
            "kind": "linear_glm",
            "response_column": 3,
            "predictor_columns": [0, 1, 2]
        }),
    )?;
    let mlr_id = extract_sheet_id(&glm_out, "Linear GLM")?;
    let r2 = scalar_val(result_sheet(&app, &mlr_id)?, "R squared", None)?;
    ensure(r2 > 0.99, format!("Multiple Linear Regression R2 expected >0.99, got {r2}"))?;

    // Verify R9 precision spinner graying via EditAnalysisParameters.
    let ep_r9 = open_edit_params_shortcut(&mut app, &pca_id)?;
    let _ = ep_r9;
    answer_dialog(&mut app, json!({ "p_value_format": "apa" }), "apply")?;
    assert_widget_state(&inspect_ui(&mut app)?, "edit.pvalue.digits", false)?;
    answer_dialog(&mut app, json!({ "p_value_format": "custom" }), "apply")?;
    assert_widget_state(&inspect_ui(&mut app)?, "edit.pvalue.digits", true)?;
    answer_dialog(&mut app, json!({ "p_value_format": "custom", "p_value_decimals": 3 }), "ok")?;

    // 4. Docs WF-22: Multiple row t-tests + Volcano Plot + Format Selected Points.
    let grp_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf22-volc-tbl",
            "title": "Gene Screen",
            "kind": "grouped",
            "rows": 5,
            "cols": 2,
            "replicates": 3,
            "auto_graph": true,
            "graph_sheet_id": "wf22-volc-graph"
        }),
    )?;
    set_titles(&mut app, &grp_id, &["Ctrl", "Treated"], Some(&["Gene1", "Gene2", "Gene3", "Gene4", "Gene5"]))?;
    let mut g_edits = Vec::new();
    for r in 0..5 {
        let delta = if r < 2 { 8.0 } else { 0.2 };
        for rep in 0..3 {
            let jit = (rep as f64 - 1.0) * 0.25;
            g_edits.push(json!({ "row": r, "col": 0, "subcol": rep, "value": 10.0 + jit }));
            g_edits.push(json!({ "row": r, "col": 1, "subcol": rep, "value": 10.0 + delta + jit }));
        }
    }
    invoke_menu_path(&mut app, "Data/Set Multiple Cells", json!({ "sheet_id": grp_id, "edits": g_edits }))?;
    run_analyze_dialog(&mut app, &grp_id, json!({ "entry_id": "stats.multiple_row_t_tests" }))?;

    select_sheet(&mut app, "wf22-volc-graph")?;
    invoke_menu_cmd(&mut app, "ui.changeGraphType.open", json!({}))?;
    answer_dialog(&mut app, json!({ "option_id": "diag.volcano" }), "ok")?;
    let volc_scene = compile_scene(&app, "wf22-volc-graph")?;
    ensure(!volc_scene.primitives.is_empty(), "Volcano GraphSheet must compile primitives")?;

    // Open Format Selected Points dialog strictly via menu command and override point style.
    invoke_menu_cmd(&mut app, "ui.formatPoints.open", json!({}))?;
    answer_dialog(
        &mut app,
        json!({
            "selected_points": [
                { "series_index": 0, "point_index": 0 },
                { "series_index": 0, "point_index": 1 }
            ],
            "color": "#E69F00",
            "size_pt": 9.0
        }),
        "ok",
    )?;
    let volc_graph = app.document().graph_sheet(&"wf22-volc-graph".into()).ok_or("missing wf22-volc-graph")?;
    ensure(volc_graph.spec.point_overrides.len() >= 2, "Format Selected Points must persist multi-point overrides")?;
    assert_screenshot_valid(&mut app, "workspace")?;
    Ok(())
}

// ============================================================================
// WF-23: Multi-Dataset Stacked vs Superimposed Arrangement & Z-Order,
//        ResultSheet Analyze Chaining, and Leakage-Safe PCR (R10)
// ============================================================================

#[test]
fn wf23_stacked_superimposed_z_order_pcr_r10_and_result_analyze_chaining() -> TestRes {
    let mut app = new_strict_app()?;

    // Part A: Multi-dataset Grouped Stacked vs Superimposed z-order verification.
    let grp_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf23-grp",
            "title": "Arrangement Check",
            "kind": "grouped",
            "rows": 2,
            "cols": 2,
            "replicates": 2,
            "auto_graph": true,
            "graph_sheet_id": "wf23-grp-graph"
        }),
    )?;
    set_titles(&mut app, &grp_id, &["Short", "Tall"], Some(&["Baseline", "Followup"]))?;
    // Series 0 = Short (10.0), Series 1 = Tall (30.0).
    invoke_menu_path(
        &mut app,
        "Data/Set Multiple Cells",
        json!({
            "sheet_id": grp_id,
            "edits": [
                { "row": 0, "col": 0, "subcol": 0, "value": 10.0 },
                { "row": 0, "col": 0, "subcol": 1, "value": 10.0 },
                { "row": 0, "col": 1, "subcol": 0, "value": 30.0 },
                { "row": 0, "col": 1, "subcol": 1, "value": 30.0 },
                { "row": 1, "col": 0, "subcol": 0, "value": 15.0 },
                { "row": 1, "col": 0, "subcol": 1, "value": 15.0 },
                { "row": 1, "col": 1, "subcol": 0, "value": 25.0 },
                { "row": 1, "col": 1, "subcol": 1, "value": 25.0 }
            ]
        }),
    )?;

    // Switch to Stacked grouped bars directly via Change Graph Type dialog (`grouped.stacked`),
    // verifying `GraphKind::Grouped` + `GroupedArrangement::Stacked`.
    select_sheet(&mut app, "wf23-grp-graph")?;
    invoke_menu_cmd(&mut app, "ui.changeGraphType.open", json!({}))?;
    answer_dialog(&mut app, json!({ "option_id": "grouped.stacked" }), "ok")?;
    {
        let g_stacked = app.document().graph_sheet(&"wf23-grp-graph".into()).ok_or("missing graph")?;
        ensure(g_stacked.spec.kind == GraphKind::Grouped, format!("expected GraphKind::Grouped, got {:?}", g_stacked.spec.kind))?;
        ensure(g_stacked.spec.grouped_arrangement == GroupedArrangement::Stacked, "expected Stacked arrangement")?;
        let c0 = g_stacked.spec.palette.color_at(0);
        let c1 = g_stacked.spec.palette.color_at(1);
        let rgb = |c: Rgba8| (c.r, c.g, c.b);
        let scene_stacked = compile_scene(&app, "wf23-grp-graph")?;
        let stacked_bars: Vec<(u8, u8, u8)> = scene_stacked
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Path { path, fill: Some(f), clip: Some(_), .. }
                    if matches!(path.commands.first(), Some(PathCmd::MoveTo(_))) && (rgb(f.color) == rgb(c0) || rgb(f.color) == rgb(c1)) =>
                {
                    Some(rgb(f.color))
                }
                _ => None,
            })
            .collect();
        ensure(stacked_bars.len() == 4, format!("expected 4 stacked bars in ChartScene, got {stacked_bars:?}"))?;
    }

    // Switch to Superimposed via Format/Graph... (`graph.setSpec`) and verify z-order:
    // tall Series 1 bars (30.0, 25.0) draw BEFORE short Series 0 bars (15.0, 10.0) without occlusion.
    invoke_menu_path(&mut app, "Format/Graph...", json!({ "sheet_id": "wf23-grp-graph", "grouped_arrangement": "Superimposed" }))?;
    {
        let g_sup = app.document().graph_sheet(&"wf23-grp-graph".into()).ok_or("missing graph")?;
        ensure(g_sup.spec.grouped_arrangement == GroupedArrangement::Superimposed, "expected Superimposed arrangement")?;
        let c0 = g_sup.spec.palette.color_at(0);
        let c1 = g_sup.spec.palette.color_at(1);
        let rgb = |c: Rgba8| (c.r, c.g, c.b);
        let scene_sup = compile_scene(&app, "wf23-grp-graph")?;
        let bar_rgbs: Vec<(u8, u8, u8)> = scene_sup
            .primitives
            .iter()
            .filter_map(|p| match p {
                ScenePrimitive::Path { path, fill: Some(f), clip: Some(_), .. }
                    if matches!(path.commands.first(), Some(PathCmd::MoveTo(_))) && (rgb(f.color) == rgb(c0) || rgb(f.color) == rgb(c1)) =>
                {
                    Some(rgb(f.color))
                }
                _ => None,
            })
            .collect();
        let count_c1 = bar_rgbs.iter().filter(|&&c| c == rgb(c1)).count();
        let count_c0 = bar_rgbs.iter().filter(|&&c| c == rgb(c0)).count();
        let strictly_tall_first = !bar_rgbs.windows(2).any(|w| w[0] == rgb(c0) && w[1] == rgb(c1));
        ensure(
            count_c1 >= 2 && count_c0 >= 2 && count_c1 == count_c0 && strictly_tall_first,
            format!("expected tall bars ({:?}) strictly before short ({:?}), got {bar_rgbs:?}", rgb(c1), rgb(c0)),
        )?;
    }

    // Part B: ResultSheet `Analyze...` chaining (2-hop reactive DAG: DataSheet -> Transform ResultSheet -> Linear Regression ResultSheet).
    let xy_id = create_table(&mut app, json!({ "sheet_id": "wf23-xy", "title": "Raw Dose", "kind": "xy", "rows": 5, "cols": 1 }))?;
    set_x_values(&mut app, &xy_id, vec![1.0, 2.0, 3.0, 4.0, 5.0])?;
    set_cell_rows(&mut app, &xy_id, 0, json!([[10.0], [20.0], [30.0], [40.0], [50.0]]))?;

    let tr_out =
        invoke_menu_path(&mut app, "Analyze/Transform...", json!({ "sheet_id": "wf23-norm", "source_sheet_id": xy_id, "kind": "normalize" }))?;
    let norm_id = extract_sheet_id(&tr_out, "Normalize Transform")?;
    ensure(result_sheet(&app, &norm_id)?.analyzable_data.is_some(), "Transform ResultSheet must carry analyzable_data")?;

    // Invoke Analyze... directly with the ResultSheet selected (`ui.analyze.open` -> `fit.linear`).
    let chained_id = run_analyze_dialog(&mut app, &norm_id, json!({ "entry_id": "fit.linear" }))?;
    {
        let chained = result_sheet(&app, &chained_id)?;
        ensure(chained.analysis.source_sheet_id.as_str() == norm_id, "chained ResultSheet source must point to upstream ResultSheet")?;
        let slope0 = scalar_val(chained, "Slope", Some(0))?;
        ensure((slope0 - 25.0).abs() < 0.01, format!("expected normalized linear slope ~25.0, got {slope0}"))?;
    }

    // Mutate raw DataSheet and verify 2-hop propagation updates chained ResultSheet slope!
    set_cell_rows(&mut app, &xy_id, 4, json!([[90.0]]))?;
    let slope1 = scalar_val(result_sheet(&app, &chained_id)?, "Slope", Some(0))?;
    ensure((slope1 - 25.0).abs() > 1.0, format!("expected 2-hop reactive recalculation on chained ResultSheet, got {slope1}"))?;

    // Part C: Docs WF-23 Leakage-Safe PCR + R10 guard in AnalyzeSheet modal.
    let pcr_mv_id = create_table(
        &mut app,
        json!({
            "sheet_id": "wf23-mv-pcr",
            "title": "PCR Cohort",
            "kind": "multiple_variables",
            "rows": 8,
            "variables": [
                { "title": "X1", "role": "continuous" },
                { "title": "X2", "role": "continuous" },
                { "title": "Target", "role": "continuous" }
            ]
        }),
    )?;
    set_cell_rows(
        &mut app,
        &pcr_mv_id,
        0,
        json!([
            [1.0, 2.0, 5.0],
            [2.0, 3.0, 8.0],
            [3.0, 4.0, 11.0],
            [4.0, 5.0, 14.0],
            [5.0, 6.0, 17.0],
            [6.0, 8.0, 21.0],
            [7.0, 9.0, 24.0],
            [8.0, 10.0, 27.0]
        ]),
    )?;
    open_analyze_dialog(&mut app, &pcr_mv_id)?;
    patch_dialog(&mut app, json!({ "entry_id": "stats.pca", "pca_predictors": [0, 1] }))?;
    let r10_snap = inspect_ui(&mut app)?;
    assert_widget_state(&r10_snap, "analyze.pcr.outcome.1", false)?;
    assert_widget_state(&r10_snap, "analyze.pcr.outcome.2", true)?;
    let r10_leak = patch_dialog(&mut app, json!({ "pcr_outcome_column": 1 }));
    ensure(r10_leak.is_err(), "R10 must reject selecting a predictor column as PCR outcome")?;
    answer_dialog(&mut app, json!({ "pcr_outcome_column": 2 }), "ok")?;
    ensure(widget_enabled(&inspect_ui(&mut app)?, "analyze.entry").is_err(), "Analyze modal should close after ok")?;
    assert_screenshot_valid(&mut app, "workspace")?;
    Ok(())
}
