//! Wave 5 Shard `w5f3` human-path integration suite for `R10.3` Nonlinear Regression
//! Equation Tree, Custom/Implicit/ODE DSL, Constraints, Initial Values, Confidence/Ambiguous
//! Handling, Model Comparison, and Weights & Range controls.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod human_wf_common;

use std::collections::BTreeSet;

use curvebook_doc::{AnalysisKind, FitMethod, ModelComparisonSpec, NonlinearFitSpec, ParameterScope, ResultSheet, ResultStatus, WeightingScheme};
use curvebook_fit::{CompiledDslModel, DslEquationKind, format_model_equation_tex, list_builtin_equations};
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::text_bridge::{DesktopTextMeasurer, TextMeasurer as _, UiTextStyle};
use curvebook_ui_egui::views::{
    ViewShardStates, dispatch_custom_view_action,
    fit_params::{FitParamsViewState, is_fit_analysis_draft, latex_to_display},
};
use curvebook_ui_egui::{HumanPathDriver, KeyModifiers, UiApp, dispatch_gui_action};
use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, new_human_driver,
    press_cmd_shortcut, press_key_combo, scroll_until_widget_visible, type_into_focused,
};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

fn ensure(cond: bool, msg: impl Into<String>) -> TestRes {
    if cond { Ok(()) } else { Err(msg.into()) }
}

fn gui(app: &mut UiApp, method: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, method, params).map_err(|err| format!("{method} failed [{}]: {err}", err.code))
}

fn invoke_menu_path(app: &mut UiApp, path: &str, params: Value) -> Result<Value, String> {
    gui(app, "ui.invoke_menu", json!({ "path": path, "params": params }))
}

fn extract_sheet_id(val: &Value, ctx: &str) -> Result<String, String> {
    val.get("sheet_id")
        .or_else(|| val.get("payload").and_then(|p| p.get("sheet_id")))
        .or_else(|| val.get("result").and_then(|r| r.get("sheet_id")))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{ctx} missing `sheet_id`: {val}"))
}

fn seed_dose_response_app() -> Result<(UiApp, String, String), String> {
    let mut app = UiApp::new();
    app.set_gui_only(true);
    let table_out = invoke_menu_path(&mut app, "Insert/New Data Table", json!({ "kind": "xy", "title": "Agonist Dose-Response" }))?;
    let xy_id = extract_sheet_id(&table_out, "New XY Table")?;
    invoke_menu_path(&mut app, "View/Select Sheet", json!({ "sheet_id": xy_id }))?;

    // Standard log-dose concentration ladder (-9 .. -4) with exact 4PL signal + tiny replicate jitter.
    let log_doses = [-9.0, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.5];
    invoke_menu_path(&mut app, "Data/Set X Values", json!({ "sheet_id": xy_id, "values": log_doses }))?;
    let mut tsv_rows = Vec::with_capacity(log_doses.len());
    for (i, &lx) in log_doses.iter().enumerate() {
        let hill = 10.0 + 90.0 / (1.0 + 10_f64.powf(-6.8 - lx));
        let jitter = if i % 2 == 0 { 1.4 } else { -1.4 };
        tsv_rows.push(format!("{:.4}\t{:.4}", hill + jitter, hill - jitter));
    }
    gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.0.0.0" }))?;
    gui(
        &mut app,
        "ui.type_text",
        json!({ "widget_id": "grid.cell.0.0.0", "text": tsv_rows.join("\n"), "clear_existing": true, "commit_enter": true }),
    )?;

    let before = app.document().result_sheets.len();
    invoke_menu_path(&mut app, "View/Select Sheet", json!({ "sheet_id": xy_id }))?;
    gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": {} }))?;
    gui(
        &mut app,
        "ui.answer_dialog",
        json!({
            "fields": { "entry_id": "fit.nonlinear", "equation_id": "log_agonist_vs_response_4pl" },
            "action": "ok"
        }),
    )?;
    let rs = app.document().result_sheets.get(before).ok_or_else(|| String::from("missing created nonlinear ResultSheet"))?;
    let rs_id = rs.id.as_str().to_owned();
    Ok((app, xy_id, rs_id))
}

fn open_edit_params_via_human_driver(driver: &mut HumanPathDriver, result_sheet_id: &str) -> TestRes {
    click_widget_center(driver, &WidgetId::from(format!("nav.sheet.{result_sheet_id}")))?;
    press_cmd_shortcut(driver, "t")?;
    ensure(
        driver.app().canvas_dialogs.edit_params.draft.is_some(),
        format!("Cmd+T on ResultSheet `{result_sheet_id}` did not open EditAnalysisParameters draft"),
    )?;
    ensure(is_fit_analysis_draft(driver.app()), "active draft must satisfy is_fit_analysis_draft")?;
    Ok(())
}

fn active_nl(driver: &HumanPathDriver) -> Result<&NonlinearFitSpec, String> {
    match driver.app().canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::NonlinearRegression(nl)) => Ok(nl),
        other => Err(format!("expected NonlinearRegression draft, got {other:?}")),
    }
}

fn active_result_sheet<'a>(driver: &'a HumanPathDriver, rs_id: &str) -> Result<&'a ResultSheet, String> {
    driver.app().document().result_sheets.iter().find(|rs| rs.id.as_str() == rs_id).ok_or_else(|| format!("missing ResultSheet `{rs_id}`"))
}

fn assert_unique_fit_ids(driver: &mut HumanPathDriver, ctx: &str) -> TestRes {
    let tree = driver.layout_tree();
    let mut seen = BTreeSet::new();
    for node in &tree.nodes {
        let id = node.id.as_str();
        if id.starts_with("fit.") && !seen.insert(id.to_owned()) {
            return Err(format!("duplicate widget ID `{id}` emitted in {ctx}"));
        }
    }
    Ok(())
}

#[test]
fn r10_w5_fit_params_117_equation_catalog_tree_search_dsl_and_preview() -> TestRes {
    // 1. Verify full 117 built-in equation library & zero missing glyphs across all 117 TeX->display previews.
    let catalog = list_builtin_equations();
    ensure(catalog.len() == 117, format!("expected 117 built-in equations, got {}", catalog.len()))?;
    let measurer = DesktopTextMeasurer::new();
    let style = UiTextStyle::ui_sans(12.0);
    for entry in &catalog {
        let params: Vec<(String, f64)> = entry.param_names.iter().map(|p| (p.clone(), 1.0)).collect();
        let tex = format_model_equation_tex(&entry.id, None, &params, false, 4);
        let disp = latex_to_display(&tex);
        ensure(!disp.is_empty(), format!("empty formula preview for `{}`", entry.id))?;
        let shaped = measurer.shape_line(&disp, &style, "en");
        ensure(shaped.missing_glyph_count == 0, format!("missing glyph in formula `{}` -> `{disp}`", entry.id))?;
    }

    // 2. Also validate Explicit, Implicit, and ODE starter DSL templates compile cleanly.
    let exp_m = CompiledDslModel::compile("e", "Y = Bottom + (Top - Bottom) / (1 + (X / EC50)^HillSlope)").map_err(|e| e.to_string())?;
    ensure(exp_m.kind() == DslEquationKind::Explicit, "expected Explicit DSL kind")?;
    let imp_m = CompiledDslModel::compile("i", "Y + K * Y^2 = A * X + B").map_err(|e| e.to_string())?;
    ensure(imp_m.kind() == DslEquationKind::Implicit, "expected Implicit DSL kind")?;
    let ode_m = CompiledDslModel::compile("o", "dY/dX = -k * Y\nY(0) = Y0").map_err(|e| e.to_string())?;
    ensure(ode_m.kind() == DslEquationKind::OdeFirstOrder, "expected ODE DSL kind")?;

    // 3. Human-path workflow: open Nonlinear Regression EditAnalysisParameters via nav click + Cmd+T.
    let (app, _xy_id, rs_id) = seed_dose_response_app()?;
    let mut driver = new_human_driver(app, default_viewport());
    open_edit_params_via_human_driver(&mut driver, &rs_id)?;
    assert_zero_shortcut_compliance(&mut driver)?;

    // Expand all categories -> scroll to Exponential category header -> collapse Dose-Response Stimulation.
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.expand_all"))?;
    scroll_until_widget_visible(&mut driver, &WidgetId::from("fit.eq.tree.scroll"), &WidgetId::from("fit.eq.cat.exponential"), 60.0, 24)?;
    let tree_expanded = driver.layout_tree();
    ensure(tree_expanded.node("fit.eq.cat.exponential").is_some(), "expected Exponential category header after Expand All + scroll")?;
    assert_zero_shortcut_compliance(&mut driver)?;
    scroll_until_widget_visible(&mut driver, &WidgetId::from("fit.eq.tree.scroll"), &WidgetId::from("fit.eq.cat.dose_stim"), -60.0, 24)?;
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.cat.dose_stim"))?;
    let after_collapse = driver.layout_tree();
    ensure(after_collapse.node("fit.eq.item.log_agonist_vs_response_4pl").is_none(), "expected dose_stim child hidden when collapsed")?;

    // Type live search for `one_phase_decay` in `fit.eq.search`, select `fit.eq.item.one_phase_decay`.
    click_and_type(&mut driver, &WidgetId::from("fit.eq.search"), "one_phase_decay")?;
    scroll_until_widget_visible(&mut driver, &WidgetId::from("fit.eq.tree.scroll"), &WidgetId::from("fit.eq.item.one_phase_decay"), 60.0, 12)?;
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.item.one_phase_decay"))?;
    ensure(active_nl(&driver)?.equation_id == "one_phase_decay", "equation_id should switch to one_phase_decay")?;
    assert_zero_shortcut_compliance(&mut driver)?;

    // Verify formula preview updated and has valid text runs.
    let preview_tree = driver.layout_tree();
    let has_decay_preview = preview_tree
        .text_runs
        .iter()
        .any(|run| run.owner_widget_id.as_str() == "fit.eq.preview_card" && (run.text.contains("One phase decay") || run.text.contains("Plateau")));
    ensure(has_decay_preview, "preview card must reflect selected One phase decay model & parameters")?;

    // Test Custom DSL -> switch ODE starter -> switch Explicit -> edit via Cmd+A + typing -> Apply -> Clear back to built-in.
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.dsl_mode.ode"))?;
    ensure(active_nl(&driver)?.custom_dsl.as_deref().is_some_and(|s| s.contains("dY/dX")), "ODE preset must apply compiling ODE DSL")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.dsl_mode.implicit"))?;
    ensure(active_nl(&driver)?.custom_dsl.as_deref().is_some_and(|s| s.contains("Y^2")), "Implicit preset must apply compiling Implicit DSL")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.dsl.input"))?;
    press_cmd_shortcut(&mut driver, "a")?;
    press_key_combo(&mut driver, "Backspace", KeyModifiers::default())?;
    type_into_focused(&mut driver, "Y = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.dsl_apply"))?;
    ensure(
        active_nl(&driver)?.custom_dsl.as_deref() == Some("Y = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))"),
        "custom explicit DSL must compile and persist in draft",
    )?;
    assert_zero_shortcut_compliance(&mut driver)?;

    // Revert back to built-in 4PL agonist model, confirm via `modal.ok`, and inspect resulting ResultSheet.
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.dsl_clear"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.search_clear"))?;
    click_and_type(&mut driver, &WidgetId::from("fit.eq.search"), "log_agonist_vs_response_4pl")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.eq.item.log_agonist_vs_response_4pl"))?;
    click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let rs = active_result_sheet(&driver, &rs_id)?;
    ensure(rs.status == ResultStatus::UpToDate, format!("expected ResultSheet UpToDate after modal.ok, got {:?}", rs.status))?;
    ensure(!rs.fitted_curves.is_empty(), "expected fitted curve overlay on updated ResultSheet")?;
    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_w5_fit_params_constraints_initials_confidence_compare_weights_end_to_end() -> TestRes {
    let (app, _xy_id, rs_id) = seed_dose_response_app()?;
    let mut driver = new_human_driver(app, default_viewport());
    open_edit_params_via_human_driver(&mut driver, &rs_id)?;

    // 1. Constraints tab: Share `Bottom`, set `HillSlope` Lower > 0, test Constant scope on `Top`, revert `Top` to Local.
    click_widget_center(&mut driver, &WidgetId::from("fit.tab.constraints"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    assert_unique_fit_ids(&mut driver, "constraints tab")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.con.shared.0"))?;
    ensure(
        active_nl(&driver)?.parameter_rules.iter().any(|r| r.name == "Bottom" && r.scope == ParameterScope::SharedGlobal),
        "Bottom must become SharedGlobal after clicking shared toggle",
    )?;
    click_widget_center(&mut driver, &WidgetId::from("fit.con.row.3"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.con.preset.gt0"))?;
    ensure(
        active_nl(&driver)?.parameter_rules.iter().any(|r| r.name == "HillSlope" && r.lower_bound.is_some_and(|v| v > 0.0)),
        "HillSlope lower bound must be positive",
    )?;
    click_widget_center(&mut driver, &WidgetId::from("fit.con.row.1"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.con.set_scope.constant"))?;
    let tree_const = driver.layout_tree();
    ensure(
        tree_const.node("fit.con.preset.ge0").is_some_and(|n| !n.state.enablement.enabled),
        "bounds presets must be disabled while parameter is Constant",
    )?;
    click_widget_center(&mut driver, &WidgetId::from("fit.con.set_scope.local"))?;

    // 2. Initial Values tab: Compute data-driven initials, step `LogEC50` initial.
    click_widget_center(&mut driver, &WidgetId::from("fit.tab.initial_values"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    assert_unique_fit_ids(&mut driver, "initial values tab")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.init.auto_guess"))?;
    ensure(
        active_nl(&driver)?.parameter_rules.iter().any(|r| r.name == "LogEC50" && r.initial_override.is_some()),
        "Compute Data-Driven Initials must populate LogEC50 initial_override",
    )?;
    click_widget_center(&mut driver, &WidgetId::from("fit.init.inc.2"))?;

    // 3. Confidence tab: Switch to Profile Likelihood CI, 99% level, Confidence + Prediction bands, unstable flag & 500 max_iters.
    click_widget_center(&mut driver, &WidgetId::from("fit.tab.confidence"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    assert_unique_fit_ids(&mut driver, "confidence tab")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.ci.profile"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.ci.lvl.99"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.ci.conf_bands"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.ci.pred_bands"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.ci.flag_unstable"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.ci.flag_unstable"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.ci.iters.500"))?;

    // 4. Compare tab: Select AICc, search `3pl`, choose `log_agonist_vs_response_3pl`, then switch to Extra-SS F.
    click_widget_center(&mut driver, &WidgetId::from("fit.tab.compare"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    assert_unique_fit_ids(&mut driver, "compare tab")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.cmp.aicc"))?;
    click_and_type(&mut driver, &WidgetId::from("fit.cmp.alt.search"), "3pl")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.cmp.alt.log_agonist_vs_response_3pl"))?;
    ensure(
        matches!(
            active_nl(&driver)?.comparison,
            Some(ModelComparisonSpec::Aicc { ref alternative_equation_id }) if alternative_equation_id == "log_agonist_vs_response_3pl"
        ),
        "AICc alternative model must update to log_agonist_vs_response_3pl",
    )?;
    click_widget_center(&mut driver, &WidgetId::from("fit.cmp.f"))?;

    // 5. Weights & Range tab: 1/Y^2 weighting, toggle Normality/Homoscedasticity & Residuals, X range filter [-9.5, -4.0].
    click_widget_center(&mut driver, &WidgetId::from("fit.tab.weights_range"))?;
    assert_zero_shortcut_compliance(&mut driver)?;
    assert_unique_fit_ids(&mut driver, "weights_range tab")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.wr.weight.rel_sq"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.wr.opt.homo"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.wr.opt.res_tab"))?;
    ensure(!active_nl(&driver)?.emit_residuals_tab, "clicking default-true emit_residuals_tab once must toggle it off")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.wr.opt.res_tab"))?;
    ensure(active_nl(&driver)?.emit_residuals_tab, "clicking emit_residuals_tab again must re-enable it")?;
    click_and_type(&mut driver, &WidgetId::from("fit.wr.xmin.input"), "-9.5")?;
    click_and_type(&mut driver, &WidgetId::from("fit.wr.xmax.input"), "-4.0")?;
    click_widget_center(&mut driver, &WidgetId::from("fit.wr.range_apply"))?;

    // Confirm dialog and verify final ResultSheet updates cleanly.
    click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    let rs = active_result_sheet(&driver, &rs_id)?;
    ensure(rs.status == ResultStatus::UpToDate, format!("expected UpToDate ResultSheet, got {:?}", rs.status))?;
    ensure((rs.analysis.confidence_level.percent() - 99.0).abs() < 1e-6, "expected 99% confidence level on ResultSheet")?;
    let AnalysisKind::NonlinearRegression(final_nl) = &rs.analysis.kind else {
        return Err(String::from("expected NonlinearRegression ResultSheet"));
    };
    ensure(final_nl.profile_likelihood_ci, "expected profile_likelihood_ci == true")?;
    ensure(final_nl.weighting == WeightingScheme::RelativeSquare, "expected RelativeSquare weighting")?;
    ensure(final_nl.homoscedasticity_test, "expected homoscedasticity_test == true")?;
    ensure(final_nl.emit_residuals_tab, "expected emit_residuals_tab == true")?;
    ensure(final_nl.x_range_filter == Some((Some(-9.5), Some(-4.0))), format!("unexpected range filter: {:?}", final_nl.x_range_filter))?;
    ensure(
        final_nl.comparison == Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("log_agonist_vs_response_3pl") }),
        format!("unexpected comparison spec: {:?}", final_nl.comparison),
    )?;
    ensure(rs.tabs.len() >= 2, "expected multiple result tabs including comparison/residuals")?;
    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

#[test]
fn r10_w5_fit_params_robust_disabled_tooltips_smoke_and_linear_pb() -> TestRes {
    // 1. Preserve scaffold smoke-test contract: unknown `"smoke"` action must return handled == false.
    let mut smoke_app = UiApp::new();
    let mut shards = ViewShardStates::default();
    let out = dispatch_custom_view_action(&mut smoke_app, &mut shards, "fit_params", "smoke")?;
    ensure(!out.handled, "`fit_params` custom action `smoke` must return handled == false")?;

    // 2. Robust mode disables Profile CI, Covariance, and Model Comparisons with explicit hover tooltips.
    let (app, _xy_id, rs_id) = seed_dose_response_app()?;
    let mut driver = new_human_driver(app, default_viewport());
    open_edit_params_via_human_driver(&mut driver, &rs_id)?;

    click_widget_center(&mut driver, &WidgetId::from("fit.tab.weights_range"))?;
    click_widget_center(&mut driver, &WidgetId::from("fit.wr.method.robust"))?;
    ensure(active_nl(&driver)?.method == FitMethod::RobustCauchy, "expected RobustCauchy fit method")?;

    click_widget_center(&mut driver, &WidgetId::from("fit.tab.confidence"))?;
    driver.hover_widget("fit.ci.profile")?;
    let conf_tree = driver.layout_tree();
    let prof_node = conf_tree.node("fit.ci.profile").ok_or_else(|| String::from("missing fit.ci.profile node"))?;
    ensure(!prof_node.state.enablement.enabled, "fit.ci.profile must be disabled in Robust mode")?;
    ensure(
        prof_node.state.enablement.disabled_reason.as_deref().is_some_and(|r| r.contains("Robust")),
        "disabled tooltip reason must mention Robust mode",
    )?;
    let has_tooltip = conf_tree.text_runs.iter().any(|t| t.text.contains("Profile-likelihood CIs are disabled"));
    ensure(has_tooltip, "hovering disabled fit.ci.profile must paint tooltip run")?;
    assert_zero_shortcut_compliance(&mut driver)?;

    click_widget_center(&mut driver, &WidgetId::from("fit.tab.compare"))?;
    driver.hover_widget("fit.cmp.aicc")?;
    let cmp_tree = driver.layout_tree();
    let aicc_node = cmp_tree.node("fit.cmp.aicc").ok_or_else(|| String::from("missing fit.cmp.aicc node"))?;
    ensure(!aicc_node.state.enablement.enabled, "fit.cmp.aicc must be disabled in Robust mode")?;
    assert_zero_shortcut_compliance(&mut driver)?;
    click_widget_center(&mut driver, &WidgetId::from("modal.cancel"))?;

    // 3. Also verify LinearRegression fit modal renders cleanly with zero parity errors.
    let (mut lr_app, xy_lr, _) = seed_dose_response_app()?;
    let before = lr_app.document().result_sheets.len();
    invoke_menu_path(&mut lr_app, "View/Select Sheet", json!({ "sheet_id": xy_lr }))?;
    gui(&mut lr_app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": {} }))?;
    gui(&mut lr_app, "ui.answer_dialog", json!({ "fields": { "entry_id": "fit.linear" }, "action": "ok" }))?;
    let lr_rs_id =
        lr_app.document().result_sheets.get(before).ok_or_else(|| String::from("missing linear regression result"))?.id.as_str().to_owned();
    let mut lr_driver = new_human_driver(lr_app, default_viewport());
    open_edit_params_via_human_driver(&mut lr_driver, &lr_rs_id)?;
    click_widget_center(&mut lr_driver, &WidgetId::from("fit.lr.deming"))?;
    assert_zero_shortcut_compliance(&mut lr_driver)?;
    click_widget_center(&mut lr_driver, &WidgetId::from("modal.ok"))?;

    let lr_rs = active_result_sheet(&lr_driver, &lr_rs_id)?;
    ensure(
        matches!(lr_rs.analysis.kind, AnalysisKind::LinearRegression(ref lr) if lr.deming_model_ii),
        "Deming Model II toggle must persist cleanly on LinearRegression ResultSheet",
    )?;
    assert_input_to_paint_budget(&lr_driver, 50.0)?;

    // 4. Also verify Passing-Bablok modal renders cleanly (`is_fit_analysis_draft` + `fit.pb.*`).
    let mut pb_app = UiApp::new();
    pb_app.set_gui_only(true);
    let tbl_pb = invoke_menu_path(&mut pb_app, "Insert/New Data Table", json!({ "kind": "column", "title": "Method Comparison" }))?;
    let col_id = extract_sheet_id(&tbl_pb, "New Column Table")?;
    invoke_menu_path(&mut pb_app, "View/Select Sheet", json!({ "sheet_id": col_id }))?;
    gui(&mut pb_app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "grid.cell.0.0.0" }))?;
    gui(
        &mut pb_app,
        "ui.type_text",
        json!({ "widget_id": "grid.cell.0.0.0", "text": "10.0\t10.2\n20.0\t20.5\n30.0\t30.1\n40.0\t40.8\n50.0\t50.4", "clear_existing": true, "commit_enter": true }),
    )?;
    let before_pb = pb_app.document().result_sheets.len();
    invoke_menu_path(&mut pb_app, "View/Select Sheet", json!({ "sheet_id": col_id }))?;
    gui(&mut pb_app, "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": {} }))?;
    gui(&mut pb_app, "ui.answer_dialog", json!({ "fields": { "entry_id": "stats.passing_bablok" }, "action": "ok" }))?;
    let pb_rs_id =
        pb_app.document().result_sheets.get(before_pb).ok_or_else(|| String::from("missing passing_bablok result"))?.id.as_str().to_owned();
    let mut pb_driver = new_human_driver(pb_app, default_viewport());
    open_edit_params_via_human_driver(&mut pb_driver, &pb_rs_id)?;
    click_widget_center(&mut pb_driver, &WidgetId::from("fit.pb.ci.99"))?;
    assert_zero_shortcut_compliance(&mut pb_driver)?;
    assert_unique_fit_ids(&mut pb_driver, "passing_bablok")?;
    click_widget_center(&mut pb_driver, &WidgetId::from("modal.ok"))?;
    ensure(
        (active_result_sheet(&pb_driver, &pb_rs_id)?.analysis.confidence_level.percent() - 99.0).abs() < 1e-6,
        "Passing-Bablok CI 99% preset must persist cleanly",
    )?;
    assert_input_to_paint_budget(&pb_driver, 50.0)?;

    // Keep state type exported cleanly for downstream consumers.
    let _ = FitParamsViewState::default();
    Ok(())
}
