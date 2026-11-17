//! Canonical R10.13 human-path tests for `WF-19`..`WF-24` (`R10.2`, `gui_only = true`, `human_path_only = true`).
//!
//! Every workflow drives `UiApp` exclusively through [`HumanPathDriver`] pointer and keyboard events,
//! verifies zero-shortcut compliance (`assert_zero_shortcut_compliance`), enforces layout-to-pixel
//! parity (`verify_parity().is_clean()`), exercises large-N viewport scrolling (`scroll_until_widget_visible`),
//! validates multi-column `ResultSheet` rendering, and confirms live upstream edit recomputation.

mod human_wf_common;

use curvebook_doc::{
    AnalysisKind, AxisScale, BracketLabelMode, ContingencyMethod, DiagnosticPlotKind, GlmFamily, GraphKind, GroupedArrangement, MultiplicityMethod,
    MultivariateSpec, OneWayMethod, ResultCell, ResultSheet, ResultTab, ResultTabKind, RocCiMethod, SheetId, TwoGroupMethod,
};
use curvebook_ui_egui::{HumanPathDriver, KeyModifiers, UiApp, WidgetId};
use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, new_human_driver,
    press_cmd_shortcut, scroll_until_widget_visible, type_into_focused, widget_hit_center,
};

type TestRes = Result<(), String>;

fn wid(id: &str) -> WidgetId {
    WidgetId::from(id)
}

fn init_driver() -> Result<HumanPathDriver, String> {
    let app = UiApp::new();
    let mut driver = new_human_driver(app, default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    assert_parity(&mut driver, "initial welcome")?;
    Ok(driver)
}

fn assert_parity(driver: &mut HumanPathDriver, stage: &str) -> TestRes {
    let report = driver.verify_parity();
    if !report.is_clean() {
        return Err(format!("parity violations at `{stage}`: {:?}", report.violations));
    }
    Ok(())
}

fn click_checked(driver: &mut HumanPathDriver, id: &str) -> TestRes {
    let out = click_widget_center(driver, &wid(id))?;
    if !out.handled {
        return Err(format!("click on `{id}` was not handled (action: `{}`)", out.action));
    }
    Ok(())
}

fn ensure_node_visible_in(driver: &mut HumanPathDriver, container: &str, target: &str) -> Result<curvebook_ui_egui::WidgetNode, String> {
    if let Some(node) = driver.find_widget(target)
        && node.bounds.visible_rect_px.width > 0
        && node.bounds.visible_rect_px.height > 0
    {
        return Ok(node);
    }
    let container_center = widget_hit_center(driver, &wid(container)).or_else(|_| {
        driver
            .find_widget(container)
            .map(|n| {
                let r = n.bounds.rect_logical;
                [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5]
            })
            .ok_or_else(|| format!("missing scroll container `{container}`"))
    })?;
    for step_y in [120.0_f64, -180.0_f64] {
        for _ in 0..50 {
            let _ = driver.wheel_logical(container_center[0], container_center[1], 0.0, step_y, KeyModifiers::default())?;
            if let Some(node) = driver.find_widget(target)
                && node.bounds.visible_rect_px.width > 0
                && node.bounds.visible_rect_px.height > 0
            {
                return Ok(node);
            }
        }
    }
    Err(format!("widget `{target}` did not become visible in `{container}`"))
}

fn ensure_widget_visible_in(driver: &mut HumanPathDriver, container: &str, target: &str) -> Result<[f64; 2], String> {
    let c_wid = wid(container);
    let t_wid = wid(target);
    if let Ok(center) = widget_hit_center(driver, &t_wid) {
        return Ok(center);
    }
    if scroll_until_widget_visible(driver, &c_wid, &t_wid, 120.0, 50).is_ok() {
        return widget_hit_center(driver, &t_wid);
    }
    scroll_until_widget_visible(driver, &c_wid, &t_wid, -180.0, 50)?;
    widget_hit_center(driver, &t_wid)
}

fn scroll_click(driver: &mut HumanPathDriver, container: &str, target: &str) -> TestRes {
    let center = ensure_widget_visible_in(driver, container, target)?;
    let out = driver.click_logical(center[0], center[1])?;
    if !out.handled {
        return Err(format!("scroll_click on `{target}` inside `{container}` was not handled"));
    }
    Ok(())
}

/// Create a data table exclusively via the Welcome screen controls.
fn create_table_via_welcome(
    driver: &mut HumanPathDriver,
    arch_suffix: &str,
    title: &str,
    dataset_delta: isize,
    row_delta: isize,
    replicate_delta: isize,
) -> Result<SheetId, String> {
    click_checked(driver, &format!("welcome.arch.{arch_suffix}"))?;

    for _ in 0..dataset_delta.unsigned_abs() {
        let btn = if dataset_delta > 0 { "welcome.datasets.inc" } else { "welcome.datasets.dec" };
        click_checked(driver, btn)?;
    }
    for _ in 0..row_delta.unsigned_abs() {
        let btn = if row_delta > 0 { "welcome.rows.inc" } else { "welcome.rows.dec" };
        click_checked(driver, btn)?;
    }
    for _ in 0..replicate_delta.unsigned_abs() {
        let btn = if replicate_delta > 0 { "welcome.replicates.inc" } else { "welcome.replicates.dec" };
        click_checked(driver, btn)?;
    }

    click_checked(driver, "welcome.title.input")?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = type_into_focused(driver, title)?;
    click_checked(driver, "welcome.create_cta")?;
    assert_parity(driver, &format!("after welcome create `{title}`"))?;

    driver.app().document().active_sheet_id.clone().ok_or_else(|| format!("no active sheet after creating `{title}`"))
}

fn enter_inline_widget(driver: &mut HumanPathDriver, container: &str, cell_widget: &str, text: &str) -> TestRes {
    let center = ensure_widget_visible_in(driver, container, cell_widget)?;
    let dbl = driver.double_click_logical(center[0], center[1])?;
    if !dbl.handled || !driver.app().inline_editor.active {
        return Err(format!("double-click on `{cell_widget}` did not open inline editor"));
    }
    let existing_chars = driver.app().inline_editor.buffer.chars().count();
    for _ in 0..existing_chars {
        let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    }
    let typed = driver.type_text(text)?;
    if !typed.handled {
        return Err(format!("typing `{text}` into active inline editor failed for `{cell_widget}`"));
    }
    let commit = driver.press_key("Enter", KeyModifiers::default())?;
    if !commit.handled {
        return Err(format!("Enter did not commit inline edit for `{cell_widget}`"));
    }
    Ok(())
}

fn enter_cell(driver: &mut HumanPathDriver, sheet_id: &SheetId, row: usize, col: usize, subcol: usize, text: &str) -> TestRes {
    enter_inline_widget(driver, &format!("grid.sheet.{}", sheet_id.as_str()), &format!("grid.cell.{row}.{col}.{subcol}"), text)
}

fn enter_x_cell(driver: &mut HumanPathDriver, sheet_id: &SheetId, row: usize, subcol: usize, text: &str) -> TestRes {
    enter_inline_widget(driver, &format!("grid.sheet.{}", sheet_id.as_str()), &format!("grid.xcell.{row}.{subcol}"), text)
}

fn edit_header_title(driver: &mut HumanPathDriver, sheet_id: &SheetId, header_widget: &str, new_title: &str) -> TestRes {
    scroll_click(driver, &format!("grid.sheet.{}", sheet_id.as_str()), header_widget)?;
    click_checked(driver, "grid.title_editor.input")?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = type_into_focused(driver, new_title)?;
    click_checked(driver, "grid.title_editor.apply")?;
    Ok(())
}

fn set_mv_col_role(driver: &mut HumanPathDriver, col: usize, role: &str) -> TestRes {
    if driver.find_widget("mv_formula.vars_drawer").is_none() {
        click_checked(driver, "mv_formula.variables_btn")?;
    }
    click_checked(driver, &format!("mv_formula.vars.col.{col}"))?;
    click_checked(driver, &format!("mv_formula.vars.role.{role}"))?;
    click_checked(driver, "mv_formula.vars.close")?;
    Ok(())
}

fn navigate_to_sheet(driver: &mut HumanPathDriver, sheet_id: &SheetId) -> TestRes {
    let target = format!("nav.sheet.{}", sheet_id.as_str());
    scroll_click(driver, "nav.tree", &target)?;
    if driver.app().document().active_sheet_id.as_ref() != Some(sheet_id) {
        return Err(format!("clicking `{target}` did not activate sheet `{}`", sheet_id.as_str()));
    }
    Ok(())
}

fn navigate_to_graph_sheet(driver: &mut HumanPathDriver, gid: &SheetId, keep_type_picker: bool) -> TestRes {
    navigate_to_sheet(driver, gid)?;
    let picker_open = driver.find_widget("graph_type.panel").is_some();
    if keep_type_picker && !picker_open {
        click_checked(driver, "toolbar.change_graph")?;
    } else if !keep_type_picker && picker_open {
        click_checked(driver, "modal.ok")?;
    }
    Ok(())
}

fn select_result_tab(driver: &mut HumanPathDriver, rs: &ResultSheet, kind: ResultTabKind) -> TestRes {
    let (_, tab) = find_tab(rs, kind)?;
    let tab_wid = format!("result.tab.{}", tab.id);
    if widget_hit_center(driver, &wid(&tab_wid)).is_err() {
        for _ in 0..10 {
            if driver.find_widget("result.tabs.next").is_none() {
                break;
            }
            click_checked(driver, "result.tabs.next")?;
            if widget_hit_center(driver, &wid(&tab_wid)).is_ok() {
                break;
            }
        }
    }
    if widget_hit_center(driver, &wid(&tab_wid)).is_err() {
        for _ in 0..10 {
            if driver.find_widget("result.tabs.prev").is_none() {
                break;
            }
            click_checked(driver, "result.tabs.prev")?;
            if widget_hit_center(driver, &wid(&tab_wid)).is_ok() {
                break;
            }
        }
    }
    click_checked(driver, &tab_wid)?;
    Ok(())
}

fn run_analysis_via_picker(driver: &mut HumanPathDriver, entry_id: &str, search_hint: &str) -> Result<SheetId, String> {
    let before_count = driver.app().document().result_sheets.len();
    click_checked(driver, "toolbar.analyze")?;
    if driver.find_widget("analyze.search").is_some() {
        click_checked(driver, "analyze.search")?;
        let _ = press_cmd_shortcut(driver, "a")?;
        let _ = type_into_focused(driver, search_hint)?;
    }
    let entry_widget = format!("analyze.entry.{entry_id}");
    scroll_click(driver, "analyze.entries_list", &entry_widget)?;
    assert_parity(driver, &format!("analyze picker `{entry_id}`"))?;
    click_checked(driver, "modal.ok")?;
    if driver.find_widget("analyze_picker.dup.create_new").is_some() {
        click_checked(driver, "analyze_picker.dup.create_new")?;
        click_checked(driver, "modal.ok")?;
    }

    let doc = driver.app().document();
    if doc.result_sheets.len() <= before_count {
        return Err(format!("expected `{entry_id}` to create a new ResultSheet"));
    }
    Ok(doc.result_sheets.last().ok_or_else(|| String::from("missing created ResultSheet"))?.id.clone())
}

fn open_edit_params_dialog(driver: &mut HumanPathDriver) -> TestRes {
    if driver.find_widget("result.edit_params_btn").is_some() {
        click_checked(driver, "result.edit_params_btn")?;
    } else {
        let _ = press_cmd_shortcut(driver, "t")?;
    }
    if driver.find_widget("modal.ok").is_none() {
        return Err(String::from("EditAnalysisParameters dialog did not open"));
    }
    Ok(())
}

fn edit_widget_state(driver: &mut HumanPathDriver, id: &str) -> Result<(bool, Option<String>, bool), String> {
    let node = ensure_node_visible_in(driver, "edit.scroll", id)?;
    let active = node.a11y.checked.unwrap_or(false) || node.a11y.selected.unwrap_or(false) || node.state.active;
    Ok((node.state.enablement.enabled, node.state.enablement.disabled_reason, active))
}

fn assert_edit_enabled(driver: &mut HumanPathDriver, id: &str, expected: bool, ctx: &str) -> TestRes {
    let (enabled, reason, _) = edit_widget_state(driver, id)?;
    if enabled != expected {
        return Err(format!("{ctx}: expected `{id}` enabled={expected}, got enabled={enabled} (reason={reason:?})"));
    }
    if !expected && reason.as_deref().is_none_or(|r| r.trim().is_empty()) {
        return Err(format!("{ctx}: disabled control `{id}` must expose a non-empty disabled_reason"));
    }
    Ok(())
}

fn latest_result_sheet(driver: &HumanPathDriver, rid: &SheetId) -> Result<ResultSheet, String> {
    driver.app().document().result_sheet(rid).cloned().ok_or_else(|| format!("missing ResultSheet `{}`", rid.as_str()))
}

fn find_tab(rs: &ResultSheet, kind: ResultTabKind) -> Result<(usize, &ResultTab), String> {
    rs.tabs.iter().enumerate().find(|(_, tab)| tab.kind == kind).ok_or_else(|| format!("ResultSheet `{}` missing tab `{kind:?}`", rs.id.as_str()))
}

fn format_cell(cell: &ResultCell) -> String {
    match cell {
        ResultCell::Text(s) => s.clone(),
        ResultCell::Number { value, .. } => format!("{value:.6}"),
        ResultCell::PValue { raw, formatted, stars } => format!("{formatted} ({stars}, p={raw:.6})"),
        ResultCell::ConfidenceInterval { lower, upper } => format!("[{lower:?}, {upper:?}]"),
        ResultCell::Boolean(b) => b.to_string(),
        ResultCell::Empty => String::from("--"),
    }
}

fn find_scalar_in_tab(tab: &ResultTab, row_needle: &str) -> Option<f64> {
    let row = tab
        .rows
        .iter()
        .find(|r| r.first().is_some_and(|c| matches!(c, ResultCell::Text(t) if t.to_ascii_lowercase().contains(&row_needle.to_ascii_lowercase()))))?;
    row.iter().skip(1).find_map(|cell| match cell {
        ResultCell::Number { value, .. } | ResultCell::PValue { raw: value, .. } if value.is_finite() => Some(*value),
        _ => None,
    })
}

fn norm_scalar_key(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

fn find_scalar_in_sheet(rs: &ResultSheet, needle: &str) -> Option<f64> {
    let n_norm = norm_scalar_key(needle);
    for scalar in &rs.scalars {
        if (scalar.key.eq_ignore_ascii_case(needle) || norm_scalar_key(&scalar.key) == n_norm) && scalar.value.is_finite() {
            return Some(scalar.value);
        }
    }
    if n_norm.len() >= 2 {
        for scalar in &rs.scalars {
            if norm_scalar_key(&scalar.key).contains(&n_norm) && scalar.value.is_finite() {
                return Some(scalar.value);
            }
        }
    }
    for tab in &rs.tabs {
        if let Some(v) = find_scalar_in_tab(tab, needle) {
            return Some(v);
        }
    }
    None
}

fn assert_result_cell_visible_after_scroll(driver: &mut HumanPathDriver, row: usize, col: usize) -> TestRes {
    let target = format!("result.cell.{row}.{col}");
    let _ = ensure_widget_visible_in(driver, "result.table", &target)?;
    let hdr = format!("result.col_hdr.{col}");
    if driver.find_widget(&hdr).is_none() {
        return Err(format!("expected multi-column header `{hdr}` in ResultSheet LayoutTree"));
    }
    Ok(())
}

// ============================================================================
// WF-19: Contingency 2x2 Fisher + R5/R6 + RxC Chi-Square
// ============================================================================

#[test]
fn wf19_human_contingency_2x2_fisher_r5_r6_and_rxc_chi_square() -> TestRes {
    // Part A: 2x2 Contingency (Fisher exact, Baptista-Pike OR, Koopman RR, Newcombe-Wilson, R5/R6)
    let mut driver = init_driver()?;
    let ds_2x2 = create_table_via_welcome(&mut driver, "contingency", "WF19 Clinical 2x2", 0, 0, 0)?;

    edit_header_title(&mut driver, &ds_2x2, "grid.col_title.0", "Responders")?;
    edit_header_title(&mut driver, &ds_2x2, "grid.col_title.1", "NonResponders")?;
    edit_header_title(&mut driver, &ds_2x2, "grid.row_title.0", "Treated")?;
    edit_header_title(&mut driver, &ds_2x2, "grid.row_title.1", "Control")?;

    let counts_2x2 = [["18", "2"], ["5", "20"]];
    for (r, row_vals) in counts_2x2.iter().enumerate() {
        for (c, val) in row_vals.iter().enumerate() {
            enter_cell(&mut driver, &ds_2x2, r, c, 0, val)?;
        }
    }

    let rid_2x2 = run_analysis_via_picker(&mut driver, "stats.contingency", "Contingency")?;
    open_edit_params_dialog(&mut driver)?;

    // R5 on 2x2: Fisher/McNemar enabled, Cochran-Armitage trend disabled.
    assert_edit_enabled(&mut driver, "edit.contingency.fisher", true, "WF19 2x2 R5")?;
    assert_edit_enabled(&mut driver, "edit.contingency.mcnemar", true, "WF19 2x2 R5")?;
    assert_edit_enabled(&mut driver, "edit.contingency.cochran_armitage", false, "WF19 2x2 R5")?;
    scroll_click(&mut driver, "edit.scroll", "edit.contingency.fisher")?;

    // R6 on 2x2 Options tab: CI selectors disabled until effect-size checkboxes are toggled ON.
    click_checked(&mut driver, "edit.tab.options")?;
    assert_edit_enabled(&mut driver, "edit.contingency.odds_ratio", true, "WF19 2x2 Options")?;
    assert_edit_enabled(&mut driver, "edit.contingency.attributable_risk", true, "WF19 2x2 Options")?;
    assert_edit_enabled(&mut driver, "edit.contingency.sensitivity_specificity", true, "WF19 2x2 Options")?;

    // Check R6 gating: turn OFF odds_ratio if checked so baptista_pike/koopman disable, then enable & pick methods.
    let (_, _, or_initially_checked) = edit_widget_state(&mut driver, "edit.contingency.odds_ratio")?;
    if or_initially_checked {
        scroll_click(&mut driver, "edit.scroll", "edit.contingency.odds_ratio")?;
    }
    assert_edit_enabled(&mut driver, "edit.contingency.ci.baptista_pike", false, "WF19 R6 OFF")?;
    assert_edit_enabled(&mut driver, "edit.contingency.ci.koopman", false, "WF19 R6 OFF")?;
    scroll_click(&mut driver, "edit.scroll", "edit.contingency.odds_ratio")?;
    assert_edit_enabled(&mut driver, "edit.contingency.ci.baptista_pike", true, "WF19 R6 ON")?;
    assert_edit_enabled(&mut driver, "edit.contingency.ci.koopman", true, "WF19 R6 ON")?;
    scroll_click(&mut driver, "edit.scroll", "edit.contingency.ci.baptista_pike")?;
    scroll_click(&mut driver, "edit.scroll", "edit.contingency.ci.koopman")?;

    let (_, _, ar_checked) = edit_widget_state(&mut driver, "edit.contingency.attributable_risk")?;
    if ar_checked {
        scroll_click(&mut driver, "edit.scroll", "edit.contingency.attributable_risk")?;
    }
    assert_edit_enabled(&mut driver, "edit.contingency.ci.newcombe_wilson", false, "WF19 R6 AR OFF")?;
    scroll_click(&mut driver, "edit.scroll", "edit.contingency.attributable_risk")?;
    assert_edit_enabled(&mut driver, "edit.contingency.ci.newcombe_wilson", true, "WF19 R6 AR ON")?;
    scroll_click(&mut driver, "edit.scroll", "edit.contingency.ci.newcombe_wilson")?;

    let (_, _, sens_checked) = edit_widget_state(&mut driver, "edit.contingency.sensitivity_specificity")?;
    if sens_checked {
        scroll_click(&mut driver, "edit.scroll", "edit.contingency.sensitivity_specificity")?;
    }
    assert_edit_enabled(&mut driver, "edit.contingency.ci.wilson_brown", false, "WF19 R6 Sens OFF")?;
    scroll_click(&mut driver, "edit.scroll", "edit.contingency.sensitivity_specificity")?;
    assert_edit_enabled(&mut driver, "edit.contingency.ci.wilson_brown", true, "WF19 R6 Sens ON")?;
    scroll_click(&mut driver, "edit.scroll", "edit.contingency.ci.wilson_brown")?;

    click_checked(driver_mut(&mut driver), "modal.ok")?;
    assert_parity(&mut driver, "WF19 2x2 post-modal")?;

    let rs_before = latest_result_sheet(&driver, &rid_2x2)?;
    if !matches!(rs_before.analysis.kind, AnalysisKind::Contingency(ref c) if c.method == ContingencyMethod::FisherExact) {
        return Err(format!("expected FisherExact analysis kind, got {:?}", rs_before.analysis.kind));
    }
    assert_result_cell_visible_after_scroll(&mut driver, 0, 1)?;
    let or_before = find_scalar_in_sheet(&rs_before, "odds ratio").ok_or_else(|| String::from("missing Odds ratio in 2x2 result"))?;
    let p_before = find_scalar_in_sheet(&rs_before, "p value").ok_or_else(|| String::from("missing P value in 2x2 result"))?;
    if p_before >= 0.001 || or_before <= 10.0 {
        return Err(format!("unexpected initial 2x2 Fisher stats: p={p_before}, OR={or_before}"));
    }

    // Live upstream edit: increase cell(0,1) NonResponders from 2 -> 18, verify recomputation.
    navigate_to_sheet(&mut driver, &ds_2x2)?;
    enter_cell(&mut driver, &ds_2x2, 0, 1, 0, "18")?;
    navigate_to_sheet(&mut driver, &rid_2x2)?;
    let rs_after = latest_result_sheet(&driver, &rid_2x2)?;
    let or_after = find_scalar_in_sheet(&rs_after, "odds ratio").ok_or_else(|| String::from("missing recomputed Odds ratio"))?;
    let p_after = find_scalar_in_sheet(&rs_after, "p value").ok_or_else(|| String::from("missing recomputed P value"))?;
    if (or_after - or_before).abs() < 1.0 || p_after <= p_before {
        return Err(format!(
            "expected live edit recomputation in 2x2 contingency: before=(OR {or_before}, p {p_before}), after=(OR {or_after}, p {p_after})"
        ));
    }

    // Part B: 3x3 R x C Chi-square + R5 graying
    let mut rxc_driver = init_driver()?;
    let ds_rxc = create_table_via_welcome(&mut rxc_driver, "contingency", "WF19 RxC 3x3", 1, 1, 0)?;
    let counts_3x3 = [["20", "15", "5"], ["10", "20", "15"], ["5", "10", "25"]];
    for (r, row_vals) in counts_3x3.iter().enumerate() {
        for (c, val) in row_vals.iter().enumerate() {
            enter_cell(&mut rxc_driver, &ds_rxc, r, c, 0, val)?;
        }
    }
    let rid_rxc = run_analysis_via_picker(&mut rxc_driver, "stats.contingency", "Contingency")?;
    open_edit_params_dialog(&mut rxc_driver)?;
    assert_edit_enabled(&mut rxc_driver, "edit.contingency.fisher", false, "WF19 3x3 R5")?;
    assert_edit_enabled(&mut rxc_driver, "edit.contingency.cochran_armitage", false, "WF19 3x3 R5")?;
    click_checked(&mut rxc_driver, "edit.tab.options")?;
    assert_edit_enabled(&mut rxc_driver, "edit.contingency.odds_ratio", false, "WF19 3x3 Options R5")?;
    click_checked(&mut rxc_driver, "modal.ok")?;

    let rxc_before = latest_result_sheet(&rxc_driver, &rid_rxc)?;
    assert_result_cell_visible_after_scroll(&mut rxc_driver, 0, 1)?;
    let chi2_before = find_scalar_in_sheet(&rxc_before, "chi-square").ok_or_else(|| String::from("missing 3x3 Chi-square statistic"))?;
    navigate_to_sheet(&mut rxc_driver, &ds_rxc)?;
    enter_cell(&mut rxc_driver, &ds_rxc, 2, 2, 0, "45")?;
    navigate_to_sheet(&mut rxc_driver, &rid_rxc)?;
    let rxc_after = latest_result_sheet(&rxc_driver, &rid_rxc)?;
    let chi2_after = find_scalar_in_sheet(&rxc_after, "chi-square").ok_or_else(|| String::from("missing recomputed Chi-square"))?;
    if (chi2_after - chi2_before).abs() < 1e-3 {
        return Err(format!("expected live edit recomputation on 3x3 Chi-square: before={chi2_before}, after={chi2_after}"));
    }

    assert_input_to_paint_budget(&driver, 50.0)?;
    assert_input_to_paint_budget(&rxc_driver, 50.0)?;
    Ok(())
}

// Needed by clippy-clean call sites when borrowing &mut driver cleanly.
fn driver_mut(driver: &mut HumanPathDriver) -> &mut HumanPathDriver {
    driver
}

// ============================================================================
// WF-20: Large-N ROC Curve + CurveCoordinates Scroll + Cochran-Armitage Trend
// ============================================================================

#[test]
fn wf20_human_roc_curve_and_cochran_armitage_trend() -> TestRes {
    // Part B first: 2x3 Ordered Contingency Cochran-Armitage Trend + R5 verification (< 26 rows)
    let mut trend_driver = init_driver()?;
    let ds_trend = create_table_via_welcome(&mut trend_driver, "contingency", "WF20 Ordered 2x3 Trend", 1, 0, 0)?;
    let counts_2x3 = [["30", "20", "10"], ["10", "25", "40"]];
    for (r, row_vals) in counts_2x3.iter().enumerate() {
        for (c, val) in row_vals.iter().enumerate() {
            enter_cell(&mut trend_driver, &ds_trend, r, c, 0, val)?;
        }
    }
    let rid_trend = run_analysis_via_picker(&mut trend_driver, "stats.contingency", "Contingency")?;
    open_edit_params_dialog(&mut trend_driver)?;
    assert_edit_enabled(&mut trend_driver, "edit.contingency.cochran_armitage", true, "WF20 2x3 R5")?;
    assert_edit_enabled(&mut trend_driver, "edit.contingency.fisher", false, "WF20 2x3 R5")?;
    scroll_click(&mut trend_driver, "edit.scroll", "edit.contingency.cochran_armitage")?;
    click_checked(&mut trend_driver, "modal.ok")?;

    let trend_before = latest_result_sheet(&trend_driver, &rid_trend)?;
    let trend_p_before = find_scalar_in_sheet(&trend_before, "p value").ok_or_else(|| String::from("missing Cochran-Armitage P value"))?;
    if trend_p_before >= 0.001 {
        return Err(format!("expected significant Cochran-Armitage trend P < 0.001, got {trend_p_before}"));
    }
    navigate_to_sheet(&mut trend_driver, &ds_trend)?;
    enter_cell(&mut trend_driver, &ds_trend, 1, 2, 0, "10")?;
    navigate_to_sheet(&mut trend_driver, &rid_trend)?;
    let trend_after = latest_result_sheet(&trend_driver, &rid_trend)?;
    let trend_p_after = find_scalar_in_sheet(&trend_after, "p value").ok_or_else(|| String::from("missing recomputed trend P value"))?;
    if (trend_p_after - trend_p_before).abs() < 1e-4 {
        return Err(format!("expected live edit recomputation on Cochran-Armitage: before={trend_p_before}, after={trend_p_after}"));
    }
    assert_input_to_paint_budget(&trend_driver, 50.0)?;

    // Part A: Large-N ROC Curve (60 Controls + 60 Patients = 120 samples, 120+ CurveCoordinates rows)
    let mut driver = init_driver()?;
    // Column defaults to 3 datasets, 12 rows -> dec 1 dataset (2 cols), inc 48 rows (60 rows).
    let ds_roc = create_table_via_welcome(&mut driver, "column", "WF20 ROC Large-N", -1, 48, 0)?;
    edit_header_title(&mut driver, &ds_roc, "grid.col_title.0", "Controls")?;
    edit_header_title(&mut driver, &ds_roc, "grid.col_title.1", "Patients")?;

    for r in 0..60usize {
        let ctrl_val = format!("{:.2}", 10.0 + (r as f64) * 0.25);
        let pat_val = format!("{:.2}", 22.0 + (r as f64) * 0.28);
        enter_cell(&mut driver, &ds_roc, r, 0, 0, &ctrl_val)?;
        enter_cell(&mut driver, &ds_roc, r, 1, 0, &pat_val)?;
    }

    let rid_roc = run_analysis_via_picker(&mut driver, "stats.roc", "ROC")?;
    let rs_roc = latest_result_sheet(&driver, &rid_roc)?;
    let auc_delong = find_scalar_in_sheet(&rs_roc, "auc").ok_or_else(|| String::from("missing ROC AUC scalar"))?;
    if auc_delong <= 0.85 {
        return Err(format!("expected ROC AUC > 0.85 for well-separated cohorts, got {auc_delong}"));
    }

    // Switch to CurveCoordinates tab and exercise large-N result.table scrolling via scroll_until_widget_visible.
    let (_, coords_tab) = find_tab(&rs_roc, ResultTabKind::CurveCoordinates)?;
    if coords_tab.rows.len() < 100 {
        return Err(format!("expected >= 100 CurveCoordinates rows for large-N ROC, got {}", coords_tab.rows.len()));
    }
    select_result_tab(&mut driver, &rs_roc, ResultTabKind::CurveCoordinates)?;
    assert_result_cell_visible_after_scroll(&mut driver, 95, 1)?;
    assert_parity(&mut driver, "WF20 ROC CurveCoordinates scrolled")?;

    // Switch ROC CI method via Cmd+T -> Hanley-McNeil.
    open_edit_params_dialog(&mut driver)?;
    click_checked(&mut driver, "edit.roc.ci.hanley_mcneil")?;
    click_checked(&mut driver, "modal.ok")?;
    let rs_hm = latest_result_sheet(&driver, &rid_roc)?;
    if !matches!(rs_hm.analysis.kind, AnalysisKind::RocCurve(ref r) if r.ci_method == RocCiMethod::HanleyMcNeil) {
        return Err(format!("expected HanleyMcNeil ROC CI method, got {:?}", rs_hm.analysis.kind));
    }

    // Switch linked GraphSheet to ROC Curve via Change Graph Type dialog.
    let gid = driver.app().document().graph_sheets.first().ok_or_else(|| String::from("missing default GraphSheet for ROC table"))?.id.clone();
    navigate_to_graph_sheet(&mut driver, &gid, true)?;
    scroll_click(&mut driver, "graph_type.panel", "graph_type.card.diag.roc")?;
    click_checked(&mut driver, "modal.ok")?;
    let gs = driver.app().document().graph_sheet(&gid).ok_or_else(|| String::from("missing ROC graph sheet"))?;
    if gs.spec.kind != GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve) {
        return Err(format!("expected DiagnosticPlotKind::RocCurve, got {:?}", gs.spec.kind));
    }

    // Live upstream edit on scrolled bottom row (row 59) reduces separation -> AUC changes.
    navigate_to_sheet(&mut driver, &ds_roc)?;
    enter_cell(&mut driver, &ds_roc, 59, 0, 0, "60.0")?;
    navigate_to_sheet(&mut driver, &rid_roc)?;
    let rs_edited = latest_result_sheet(&driver, &rid_roc)?;
    let auc_after = find_scalar_in_sheet(&rs_edited, "auc").ok_or_else(|| String::from("missing recomputed ROC AUC"))?;
    if (auc_after - auc_delong).abs() < 1e-4 {
        return Err(format!("expected ROC live upstream edit recomputation: before={auc_delong}, after={auc_after}"));
    }

    assert_input_to_paint_budget(&driver, 50.0)?;
    Ok(())
}

// ============================================================================
// WF-21: Kaplan-Meier Large-N Survival + Logistic GLM + R4 Two-Group Checks
// ============================================================================

#[test]
fn wf21_human_kaplan_meier_survival_logistic_and_r4() -> TestRes {
    // Part B first: Multiple Logistic Regression on MultipleVariables (16 rows)
    let mut glm_driver = init_driver()?;
    let ds_mv = create_table_via_welcome(&mut glm_driver, "multiple_variables", "WF21 Logistic MV", 0, -4, 0)?;
    edit_header_title(&mut glm_driver, &ds_mv, "grid.col_title.0", "Event")?;
    edit_header_title(&mut glm_driver, &ds_mv, "grid.col_title.1", "Dose")?;
    edit_header_title(&mut glm_driver, &ds_mv, "grid.col_title.2", "Biomarker")?;
    edit_header_title(&mut glm_driver, &ds_mv, "grid.col_title.3", "Arm")?;
    set_mv_col_role(&mut glm_driver, 3, "categorical")?;

    let mv_rows = [
        ("0", "1.0", "10.0", "Ctrl"),
        ("0", "1.4", "11.5", "Ctrl"),
        ("0", "1.8", "12.0", "Ctrl"),
        ("0", "2.1", "13.0", "Ctrl"),
        ("0", "2.6", "14.2", "Ctrl"),
        ("0", "2.9", "14.8", "Ctrl"),
        ("0", "3.2", "15.1", "Ctrl"),
        ("1", "3.5", "16.0", "Ctrl"),
        ("0", "3.8", "15.5", "High"),
        ("1", "4.2", "18.0", "High"),
        ("1", "4.8", "19.5", "High"),
        ("1", "5.3", "21.0", "High"),
        ("1", "5.8", "22.2", "High"),
        ("1", "6.2", "24.0", "High"),
        ("1", "6.8", "25.4", "High"),
        ("1", "7.4", "27.0", "High"),
    ];
    for (r, (ev, dose, bio, arm)) in mv_rows.iter().enumerate() {
        enter_cell(&mut glm_driver, &ds_mv, r, 0, 0, ev)?;
        enter_cell(&mut glm_driver, &ds_mv, r, 1, 0, dose)?;
        enter_cell(&mut glm_driver, &ds_mv, r, 2, 0, bio)?;
        enter_cell(&mut glm_driver, &ds_mv, r, 3, 0, arm)?;
    }

    let rid_glm = run_analysis_via_picker(&mut glm_driver, "stats.glm", "Logistic")?;
    open_edit_params_dialog(&mut glm_driver)?;
    click_checked(&mut glm_driver, "edit.glm.family.logistic")?;
    click_checked(&mut glm_driver, "edit.tab.factor_settings")?;
    click_checked(&mut glm_driver, "edit.glm.resp.0")?;
    let (_, _, pred2_sel) = edit_widget_state(&mut glm_driver, "edit.glm.factor_pred.2")?;
    if !pred2_sel {
        scroll_click(&mut glm_driver, "edit.scroll", "edit.glm.factor_pred.2")?;
    }
    click_checked(&mut glm_driver, "modal.ok")?;

    let glm_before = latest_result_sheet(&glm_driver, &rid_glm)?;
    if !matches!(
        glm_before.analysis.kind,
        AnalysisKind::GeneralizedLinearModel(ref g)
            if g.family == GlmFamily::Logistic && g.response_column == 0 && g.predictor_columns.contains(&2)
    ) {
        return Err(format!("expected Logistic GLM with resp=0 and pred 2, got {:?}", glm_before.analysis.kind));
    }
    select_result_tab(&mut glm_driver, &glm_before, ResultTabKind::ParametersAndConfidenceIntervals)?;
    assert_result_cell_visible_after_scroll(&mut glm_driver, 1, 2)?;
    let auc_glm_before = find_scalar_in_sheet(&glm_before, "auc").ok_or_else(|| String::from("missing Logistic GLM AUC"))?;

    navigate_to_sheet(&mut glm_driver, &ds_mv)?;
    enter_cell(&mut glm_driver, &ds_mv, 1, 0, 0, "1")?;
    navigate_to_sheet(&mut glm_driver, &rid_glm)?;
    let glm_after = latest_result_sheet(&glm_driver, &rid_glm)?;
    let auc_glm_after = find_scalar_in_sheet(&glm_after, "auc").ok_or_else(|| String::from("missing recomputed Logistic GLM AUC"))?;
    if (auc_glm_after - auc_glm_before).abs() < 1e-4 {
        return Err(format!("expected Logistic GLM live upstream edit recomputation: before={auc_glm_before}, after={auc_glm_after}"));
    }

    // Part C second: R4 Two-Group Predicates (8 rows)
    let mut tg_driver = init_driver()?;
    let ds_tg = create_table_via_welcome(&mut tg_driver, "column", "WF21 TwoGroup R4", -1, -4, 0)?;
    let pairs = [
        ("10.2", "13.1"),
        ("11.0", "14.4"),
        ("9.8", "12.9"),
        ("10.5", "13.8"),
        ("11.2", "14.6"),
        ("10.1", "13.5"),
        ("10.9", "14.2"),
        ("9.6", "13.0"),
    ];
    for (r, (a, b)) in pairs.iter().enumerate() {
        enter_cell(&mut tg_driver, &ds_tg, r, 0, 0, a)?;
        enter_cell(&mut tg_driver, &ds_tg, r, 1, 0, b)?;
    }

    let rid_tg = run_analysis_via_picker(&mut tg_driver, "stats.two_group", "Unpaired")?;
    open_edit_params_dialog(&mut tg_driver)?;
    click_checked(&mut tg_driver, "edit.tab.options")?;
    assert_edit_enabled(&mut tg_driver, "edit.two_group.welch", true, "WF21 R4 Unpaired")?;
    assert_edit_enabled(&mut tg_driver, "edit.two_group.graph_differences", false, "WF21 R4 Unpaired")?;
    assert_edit_enabled(&mut tg_driver, "edit.two_group.graph_ranks", false, "WF21 R4 Unpaired")?;
    assert_edit_enabled(&mut tg_driver, "edit.two_group.hodges_lehmann", false, "WF21 R4 Unpaired")?;

    click_checked(&mut tg_driver, "edit.tab.experimental_design")?;
    scroll_click(&mut tg_driver, "edit.scroll", "edit.paired_or_matched")?;
    click_checked(&mut tg_driver, "edit.tg.paired")?;
    click_checked(&mut tg_driver, "edit.tab.options")?;
    assert_edit_enabled(&mut tg_driver, "edit.two_group.graph_differences", true, "WF21 R4 Paired")?;
    assert_edit_enabled(&mut tg_driver, "edit.two_group.welch", false, "WF21 R4 Paired")?;
    scroll_click(&mut tg_driver, "edit.scroll", "edit.two_group.graph_differences")?;

    click_checked(&mut tg_driver, "edit.tab.experimental_design")?;
    scroll_click(&mut tg_driver, "edit.scroll", "edit.paired_or_matched")?;
    click_checked(&mut tg_driver, "edit.tg.mw")?;
    click_checked(&mut tg_driver, "edit.tab.options")?;
    assert_edit_enabled(&mut tg_driver, "edit.two_group.graph_ranks", true, "WF21 R4 MW")?;
    assert_edit_enabled(&mut tg_driver, "edit.two_group.hodges_lehmann", true, "WF21 R4 MW")?;
    click_checked(&mut tg_driver, "modal.ok")?;

    let tg_before = latest_result_sheet(&tg_driver, &rid_tg)?;
    if !matches!(tg_before.analysis.kind, AnalysisKind::TwoGroupComparison(ref t) if t.method == TwoGroupMethod::MannWhitneyU) {
        return Err(format!("expected MannWhitneyU method, got {:?}", tg_before.analysis.kind));
    }
    let mw_p_before = find_scalar_in_sheet(&tg_before, "p value").ok_or_else(|| String::from("missing MW P value"))?;
    navigate_to_sheet(&mut tg_driver, &ds_tg)?;
    enter_cell(&mut tg_driver, &ds_tg, 0, 1, 0, "8.0")?;
    navigate_to_sheet(&mut tg_driver, &rid_tg)?;
    let tg_after = latest_result_sheet(&tg_driver, &rid_tg)?;
    let mw_p_after = find_scalar_in_sheet(&tg_after, "p value").ok_or_else(|| String::from("missing recomputed MW P value"))?;
    if (mw_p_after - mw_p_before).abs() < 1e-6 {
        return Err(format!("expected TwoGroup live upstream edit recomputation: before={mw_p_before}, after={mw_p_after}"));
    }

    assert_input_to_paint_budget(&glm_driver, 50.0)?;
    assert_input_to_paint_budget(&tg_driver, 50.0)?;

    // Part A: Large-N Survival Kaplan-Meier (120 time rows across 2 cohorts)
    let mut km_driver = init_driver()?;
    // Survival defaults to 2 cohorts, 16 rows -> +104 rows = 120 rows total.
    let ds_km = create_table_via_welcome(&mut km_driver, "survival", "WF21 KM Large-N", 0, 104, 0)?;

    for r in 0..120usize {
        let t = format!("{:.1}", (r + 1) as f64);
        let ev_ctrl = if r % 2 == 0 { "1" } else { "0" };
        let ev_trt = if r >= 60 && r % 5 == 0 { "1" } else { "0" };
        enter_x_cell(&mut km_driver, &ds_km, r, 0, &t)?;
        enter_cell(&mut km_driver, &ds_km, r, 0, 0, ev_ctrl)?;
        enter_cell(&mut km_driver, &ds_km, r, 1, 0, ev_trt)?;
    }

    let rid_km = run_analysis_via_picker(&mut km_driver, "stats.survival_km", "Kaplan")?;
    let km_before = latest_result_sheet(&km_driver, &rid_km)?;
    let lr_p_before = find_scalar_in_sheet(&km_before, "p value").ok_or_else(|| String::from("missing KM Log-Rank P value"))?;
    if lr_p_before >= 0.05 {
        return Err(format!("expected significant Log-Rank P < 0.05 on separated 120-row survival cohorts, got {lr_p_before}"));
    }

    open_edit_params_dialog(&mut km_driver)?;
    assert_edit_enabled(&mut km_driver, "edit.km.log_rank_mantel_cox", true, "WF21 KM")?;
    assert_edit_enabled(&mut km_driver, "edit.km.gehan_breslow_wilcoxon", true, "WF21 KM")?;
    assert_edit_enabled(&mut km_driver, "edit.km.trend_test", false, "WF21 KM 2-group trend disabled")?;
    click_checked(&mut km_driver, "edit.tab.options")?;
    assert_edit_enabled(&mut km_driver, "edit.km.asymmetrical_greenwood_ci", true, "WF21 KM CI")?;
    click_checked(&mut km_driver, "modal.ok")?;

    let km_latest = latest_result_sheet(&km_driver, &rid_km)?;
    let (_, nar_tab) = find_tab(&km_latest, ResultTabKind::NumberAtRisk)?;
    select_result_tab(&mut km_driver, &km_latest, ResultTabKind::NumberAtRisk)?;
    assert_result_cell_visible_after_scroll(&mut km_driver, 0, 1)?;
    let probe_row = nar_tab.rows.len().saturating_sub(1).min(90);
    assert_result_cell_visible_after_scroll(&mut km_driver, probe_row, 1)?;

    navigate_to_sheet(&mut km_driver, &ds_km)?;
    enter_cell(&mut km_driver, &ds_km, 119, 1, 0, "1")?;
    navigate_to_sheet(&mut km_driver, &rid_km)?;
    let km_after = latest_result_sheet(&km_driver, &rid_km)?;
    let lr_p_after = find_scalar_in_sheet(&km_after, "p value").ok_or_else(|| String::from("missing recomputed KM P value"))?;
    if (lr_p_after - lr_p_before).abs() < 1e-12 {
        return Err(format!("expected KM live upstream edit recomputation: before={lr_p_before}, after={lr_p_after}"));
    }

    assert_input_to_paint_budget(&km_driver, 50.0)?;
    Ok(())
}

// ============================================================================
// WF-22: Multivariate PCA, Clustering, GLM (MLR), Multiple Row t-Tests,
//        Volcano Plot + Format Selected Points...
// ============================================================================

#[test]
fn wf22_human_pca_clustering_glm_volcano_format_points() -> TestRes {
    // Part A: MultipleVariables -> PCA + Clustering + Multiple Linear Regression
    let mut mv_driver = init_driver()?;
    let ds_mv = create_table_via_welcome(&mut mv_driver, "multiple_variables", "WF22 Multivariate", 0, -10, 0)?;
    edit_header_title(&mut mv_driver, &ds_mv, "grid.col_title.0", "X1")?;
    edit_header_title(&mut mv_driver, &ds_mv, "grid.col_title.1", "X2")?;
    edit_header_title(&mut mv_driver, &ds_mv, "grid.col_title.2", "X3")?;
    edit_header_title(&mut mv_driver, &ds_mv, "grid.col_title.3", "Outcome")?;

    let mv_data = [
        ("1.0", "2.1", "5.2", "4.8"),
        ("2.0", "4.0", "5.0", "7.1"),
        ("3.0", "5.9", "4.7", "9.6"),
        ("4.0", "8.1", "4.5", "11.9"),
        ("5.0", "9.8", "4.2", "14.4"),
        ("6.0", "12.1", "3.9", "16.8"),
        ("7.0", "14.2", "3.5", "19.3"),
        ("8.0", "16.0", "3.1", "21.9"),
        ("9.0", "18.1", "2.8", "24.2"),
        ("10.0", "20.2", "2.5", "26.7"),
    ];
    for (r, (x1, x2, x3, y)) in mv_data.iter().enumerate() {
        enter_cell(&mut mv_driver, &ds_mv, r, 0, 0, x1)?;
        enter_cell(&mut mv_driver, &ds_mv, r, 1, 0, x2)?;
        enter_cell(&mut mv_driver, &ds_mv, r, 2, 0, x3)?;
        enter_cell(&mut mv_driver, &ds_mv, r, 3, 0, y)?;
    }

    // A1. PCA
    let rid_pca = run_analysis_via_picker(&mut mv_driver, "stats.pca", "Principal")?;
    open_edit_params_dialog(&mut mv_driver)?;
    scroll_click(&mut mv_driver, "edit.scroll", "edit.pca.standardize")?;
    click_checked(&mut mv_driver, "modal.ok")?;
    let pca_unstd = latest_result_sheet(&mv_driver, &rid_pca)?;
    if !matches!(pca_unstd.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: false, .. })) {
        return Err(format!("expected PCA standardize=false, got {:?}", pca_unstd.analysis.kind));
    }
    assert_result_cell_visible_after_scroll(&mut mv_driver, 0, 1)?;
    open_edit_params_dialog(&mut mv_driver)?;
    scroll_click(&mut mv_driver, "edit.scroll", "edit.pca.standardize")?;
    click_checked(&mut mv_driver, "modal.ok")?;
    let pca_std = latest_result_sheet(&mv_driver, &rid_pca)?;
    let pc1_var_before = find_scalar_in_sheet(&pca_std, "pc1").or_else(|| find_scalar_in_sheet(&pca_std, "variance")).unwrap_or(0.0);

    // A2. Hierarchical + K-means Clustering
    navigate_to_sheet(&mut mv_driver, &ds_mv)?;
    let rid_cluster = run_analysis_via_picker(&mut mv_driver, "stats.clustering", "Cluster")?;
    open_edit_params_dialog(&mut mv_driver)?;
    scroll_click(&mut mv_driver, "edit.scroll", "edit.cluster.dist.manhattan")?;
    scroll_click(&mut mv_driver, "edit.scroll", "edit.cluster.link.complete")?;
    scroll_click(&mut mv_driver, "edit.scroll", "edit.cluster.k_means_k.dec")?;
    click_checked(&mut mv_driver, "modal.ok")?;
    let rs_cluster = latest_result_sheet(&mv_driver, &rid_cluster)?;
    if !matches!(rs_cluster.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: Some(2), .. })) {
        return Err(format!("expected Clustering k=2, got {:?}", rs_cluster.analysis.kind));
    }
    assert_result_cell_visible_after_scroll(&mut mv_driver, 0, 1)?;

    // A3. Multiple Linear Regression (GLM Linear with predictors 0, 1, 2 and response 3)
    navigate_to_sheet(&mut mv_driver, &ds_mv)?;
    let rid_mlr = run_analysis_via_picker(&mut mv_driver, "stats.glm", "GLM")?;
    open_edit_params_dialog(&mut mv_driver)?;
    click_checked(&mut mv_driver, "edit.tab.factor_settings")?;
    click_checked(&mut mv_driver, "edit.glm.resp.3")?;
    for c in [0usize, 1, 2] {
        let wid_pred = format!("edit.glm.factor_pred.{c}");
        let (_, _, checked) = edit_widget_state(&mut mv_driver, &wid_pred)?;
        if !checked {
            scroll_click(&mut mv_driver, "edit.scroll", &wid_pred)?;
        }
    }
    click_checked(&mut mv_driver, "modal.ok")?;
    let mlr_before = latest_result_sheet(&mv_driver, &rid_mlr)?;
    select_result_tab(&mut mv_driver, &mlr_before, ResultTabKind::ParametersAndConfidenceIntervals)?;
    assert_result_cell_visible_after_scroll(&mut mv_driver, 1, 2)?;
    let mlr_r2_before = find_scalar_in_sheet(&mlr_before, "r squared").ok_or_else(|| String::from("missing MLR R squared"))?;

    // Live upstream edit on Multivariate table recomputes downstream PCA & MLR.
    navigate_to_sheet(&mut mv_driver, &ds_mv)?;
    enter_cell(&mut mv_driver, &ds_mv, 9, 0, 0, "18.0")?;
    navigate_to_sheet(&mut mv_driver, &rid_pca)?;
    let pca_after = latest_result_sheet(&mv_driver, &rid_pca)?;
    let pc1_var_after = find_scalar_in_sheet(&pca_after, "pc1").or_else(|| find_scalar_in_sheet(&pca_after, "variance")).unwrap_or(0.0);
    navigate_to_sheet(&mut mv_driver, &rid_mlr)?;
    let mlr_after = latest_result_sheet(&mv_driver, &rid_mlr)?;
    let mlr_r2_after = find_scalar_in_sheet(&mlr_after, "r squared").ok_or_else(|| String::from("missing recomputed MLR R squared"))?;
    if (pc1_var_after - pc1_var_before).abs() < 1e-6 && (mlr_r2_after - mlr_r2_before).abs() < 1e-6 {
        return Err(String::from("expected live MV edit to recompute downstream PCA / MLR"));
    }

    // Part B: Multiple Row t-Tests (Grouped 2x3x8) + Volcano Graph + Format Selected Points...
    let mut rowt_driver = init_driver()?;
    let ds_grp = create_table_via_welcome(&mut rowt_driver, "grouped", "WF22 Gene Expression", 0, 2, 0)?;
    edit_header_title(&mut rowt_driver, &ds_grp, "grid.col_title.0", "Ctrl")?;
    edit_header_title(&mut rowt_driver, &ds_grp, "grid.col_title.1", "Treated")?;
    let genes = [
        ("GENE_A", ["10.0", "10.3", "9.8"], ["22.0", "22.4", "21.8"]),
        ("GENE_B", ["5.1", "5.3", "4.9"], ["12.2", "12.5", "11.9"]),
        ("GENE_C", ["15.0", "15.2", "14.8"], ["6.1", "6.0", "6.3"]),
        ("GENE_D", ["8.0", "8.4", "7.6"], ["8.1", "7.9", "8.3"]),
        ("GENE_E", ["12.0", "11.6", "12.4"], ["12.1", "11.9", "12.3"]),
        ("GENE_F", ["7.2", "7.4", "7.0"], ["16.0", "16.4", "15.8"]),
        ("GENE_G", ["9.1", "8.9", "9.3"], ["9.2", "9.0", "9.4"]),
        ("GENE_H", ["20.0", "20.4", "19.8"], ["20.2", "19.9", "20.3"]),
    ];
    for (r, (name, c_vals, t_vals)) in genes.iter().enumerate() {
        edit_header_title(&mut rowt_driver, &ds_grp, &format!("grid.row_title.{r}"), name)?;
        for (s, val) in c_vals.iter().enumerate() {
            enter_cell(&mut rowt_driver, &ds_grp, r, 0, s, val)?;
        }
        for (s, val) in t_vals.iter().enumerate() {
            enter_cell(&mut rowt_driver, &ds_grp, r, 1, s, val)?;
        }
    }

    let rid_rowt = run_analysis_via_picker(&mut rowt_driver, "stats.multiple_row_t_tests", "Multiple Row")?;
    let rowt_before = latest_result_sheet(&rowt_driver, &rid_rowt)?;
    let mc_kind = if find_tab(&rowt_before, ResultTabKind::MultipleComparisons).is_ok() {
        ResultTabKind::MultipleComparisons
    } else {
        ResultTabKind::MainSummary
    };
    let (_, mc_tab) = find_tab(&rowt_before, mc_kind)?;
    if mc_tab.rows.len() < 8 {
        return Err(format!("expected >= 8 row-t-test results, got {}", mc_tab.rows.len()));
    }
    select_result_tab(&mut rowt_driver, &rowt_before, mc_kind)?;
    assert_result_cell_visible_after_scroll(&mut rowt_driver, 5, 2)?;
    let hits_before = rowt_before.pairwise_comparisons.iter().filter(|c| c.significant).count();
    if hits_before < 3 {
        return Err(format!("expected >= 3 significant Volcano hits in pairwise_comparisons, got {hits_before}"));
    }

    // Select top hit rows in grid so Format > Format Selected Points... targets them cleanly.
    navigate_to_sheet(&mut rowt_driver, &ds_grp)?;
    scroll_click(&mut rowt_driver, &format!("grid.sheet.{}", ds_grp.as_str()), "grid.cell.0.1.0")?;
    let _ = rowt_driver.press_key("ArrowDown", KeyModifiers { shift: true, ..Default::default() })?;
    let _ = rowt_driver.press_key("ArrowDown", KeyModifiers { shift: true, ..Default::default() })?;

    let gid =
        rowt_driver.app().document().graph_sheets.first().ok_or_else(|| String::from("missing default GraphSheet for Grouped table"))?.id.clone();
    navigate_to_graph_sheet(&mut rowt_driver, &gid, false)?;
    click_checked(&mut rowt_driver, "menu.graph")?;
    scroll_click(&mut rowt_driver, "popup.menu.graph", "menu.ui.formatPoints.open")?;
    click_checked(&mut rowt_driver, "fmt_pts.col.red")?;
    click_checked(&mut rowt_driver, "fmt_pts.sym.diamond")?;
    click_checked(&mut rowt_driver, "fmt_pts.labels")?;
    click_checked(&mut rowt_driver, "modal.ok")?;

    let gs_pts = rowt_driver.app().document().graph_sheet(&gid).ok_or_else(|| String::from("missing GraphSheet after format points"))?;
    if gs_pts.spec.point_overrides.is_empty() {
        return Err(String::from("expected FormatSelectedPoints human flow to persist GraphSpec::point_overrides"));
    }

    // Live upstream edit eliminates GENE_A hit significance -> pairwise hits recompute.
    navigate_to_sheet(&mut rowt_driver, &ds_grp)?;
    for s in 0..3usize {
        enter_cell(&mut rowt_driver, &ds_grp, 0, 1, s, "10.1")?;
    }
    navigate_to_sheet(&mut rowt_driver, &rid_rowt)?;
    let rowt_after = latest_result_sheet(&rowt_driver, &rid_rowt)?;
    let hits_after = rowt_after.pairwise_comparisons.iter().filter(|c| c.significant).count();
    if hits_after >= hits_before {
        return Err(format!("expected live edit to reduce significant Volcano hits: before={hits_before}, after={hits_after}"));
    }

    assert_input_to_paint_budget(&mv_driver, 50.0)?;
    assert_input_to_paint_budget(&rowt_driver, 50.0)?;
    Ok(())
}

// ============================================================================
// WF-23: Grouped Stacked vs Superimposed, 2-Hop Chaining, R10 Leakage-Safe PCR
// NOTE: Superimposed selector & R10 PCR UI controls track r10-w6-integ patch #547.
// ============================================================================

#[test]
fn wf23_human_stacked_superimposed_pcr_r10_and_chaining() -> TestRes {
    // Part B first: 2-hop ResultSheet Analyze... chaining: Data -> Transform -> Normalize -> One-Way ANOVA
    let mut chain_driver = init_driver()?;
    let ds_col = create_table_via_welcome(&mut chain_driver, "column", "WF23 Chain Root", 0, -7, 0)?;
    let col_vals = [["10", "30", "100"], ["12", "35", "110"], ["11", "32", "95"], ["14", "38", "120"], ["13", "34", "105"]];
    for (r, row_vals) in col_vals.iter().enumerate() {
        for (c, val) in row_vals.iter().enumerate() {
            enter_cell(&mut chain_driver, &ds_col, r, c, 0, val)?;
        }
    }
    let rid_hop1 = run_analysis_via_picker(&mut chain_driver, "transform.math", "Transform")?;
    let rs1 = latest_result_sheet(&chain_driver, &rid_hop1)?;
    if rs1.analyzable_data.is_none() {
        return Err(String::from("expected Transform ResultSheet 1 to expose analyzable_data"));
    }
    // Hop 2 triggered directly with ResultSheet 1 active.
    let rid_hop2 = run_analysis_via_picker(&mut chain_driver, "transform.normalize", "Normalize")?;
    let rs2 = latest_result_sheet(&chain_driver, &rid_hop2)?;
    if rs2.analyzable_data.is_none() || rs2.analysis.source_sheet_id != rid_hop1 {
        return Err(format!("expected Normalize ResultSheet 2 chained from `{}`", rid_hop1.as_str()));
    }
    // Hop 3 triggered directly with ResultSheet 2 active.
    let rid_hop3 = run_analysis_via_picker(&mut chain_driver, "stats.one_way_anova", "One-way ANOVA")?;
    let rs3_before = latest_result_sheet(&chain_driver, &rid_hop3)?;
    if rs3_before.analysis.source_sheet_id != rid_hop2 {
        return Err(format!("expected ANOVA ResultSheet 3 chained from `{}`", rid_hop2.as_str()));
    }
    let f_before = find_scalar_in_sheet(&rs3_before, "F").ok_or_else(|| String::from("missing chained ANOVA F"))?;
    navigate_to_sheet(&mut chain_driver, &ds_col)?;
    enter_cell(&mut chain_driver, &ds_col, 0, 2, 0, "400")?;
    navigate_to_sheet(&mut chain_driver, &rid_hop3)?;
    let rs3_after = latest_result_sheet(&chain_driver, &rid_hop3)?;
    let f_after = find_scalar_in_sheet(&rs3_after, "F").ok_or_else(|| String::from("missing recomputed chained ANOVA F"))?;
    if (f_after - f_before).abs() < 1e-3 {
        return Err(format!("expected 3-hop live edit propagation from root table: before={f_before}, after={f_after}"));
    }
    assert_input_to_paint_budget(&chain_driver, 50.0)?;

    // Part A: Grouped Stacked (works now) + Superimposed (blocked until #547 wired)
    let mut grp_driver = init_driver()?;
    let ds_grp = create_table_via_welcome(&mut grp_driver, "grouped", "WF23 Grouped Arrangement", 0, -3, 0)?;
    let grp_vals = [[["10", "11", "12"], ["20", "22", "21"]], [["15", "14", "16"], ["25", "27", "26"]], [["8", "9", "7"], ["18", "17", "19"]]];
    for (r, cols) in grp_vals.iter().enumerate() {
        for (c, subs) in cols.iter().enumerate() {
            for (s, val) in subs.iter().enumerate() {
                enter_cell(&mut grp_driver, &ds_grp, r, c, s, val)?;
            }
        }
    }
    let gid = grp_driver.app().document().graph_sheets.first().ok_or_else(|| String::from("missing default Grouped GraphSheet"))?.id.clone();
    navigate_to_graph_sheet(&mut grp_driver, &gid, true)?;
    scroll_click(&mut grp_driver, "graph_type.panel", "graph_type.card.grouped.stacked")?;
    click_checked(&mut grp_driver, "modal.ok")?;
    let gs_stacked = grp_driver.app().document().graph_sheet(&gid).ok_or_else(|| String::from("missing stacked graph"))?;
    if gs_stacked.spec.grouped_arrangement != GroupedArrangement::Stacked {
        return Err(format!("expected GroupedArrangement::Stacked, got {:?}", gs_stacked.spec.grouped_arrangement));
    }

    // Superimposed arrangement switch (expects r10-w6-integ inspector/dialog control from #547).
    click_checked(&mut grp_driver, "inspector.tab.graph")?;
    scroll_click(&mut grp_driver, "scroll.inspector", "inspector.fg.arr_superimposed")?;
    let gs_sup = grp_driver.app().document().graph_sheet(&gid).ok_or_else(|| String::from("missing superimposed graph"))?;
    if gs_sup.spec.grouped_arrangement != GroupedArrangement::Superimposed {
        return Err(format!("expected GroupedArrangement::Superimposed, got {:?}", gs_sup.spec.grouped_arrangement));
    }

    // Part C: R10 Leakage-Safe PCR (expects r10-w6-integ AnalyzeSheet/EditParams PCR wiring from #547).
    let mut pcr_driver = init_driver()?;
    let ds_pcr = create_table_via_welcome(&mut pcr_driver, "multiple_variables", "WF23 PCR R10", 0, -12, 0)?;
    for r in 0..8usize {
        enter_cell(&mut pcr_driver, &ds_pcr, r, 0, 0, &format!("{:.1}", (r + 1) as f64))?;
        enter_cell(&mut pcr_driver, &ds_pcr, r, 1, 0, &format!("{:.1}", (r + 1) as f64 * 2.1))?;
        enter_cell(&mut pcr_driver, &ds_pcr, r, 2, 0, &format!("{:.1}", 10.0 - r as f64 * 0.4))?;
        enter_cell(&mut pcr_driver, &ds_pcr, r, 3, 0, &format!("{:.1}", 5.0 + (r + 1) as f64 * 3.0))?;
    }
    let rid_pcr = run_analysis_via_picker(&mut pcr_driver, "stats.pcr", "Principal Component Regression")?;
    open_edit_params_dialog(&mut pcr_driver)?;
    scroll_click(&mut pcr_driver, "edit.scroll", "edit.pca.pred.0")?;
    scroll_click(&mut pcr_driver, "edit.scroll", "edit.pca.pred.1")?;
    scroll_click(&mut pcr_driver, "edit.scroll", "edit.pca.pred.2")?;
    assert_edit_enabled(&mut pcr_driver, "edit.pcr.outcome.0", false, "WF23 R10 leakage guard")?;
    assert_edit_enabled(&mut pcr_driver, "edit.pcr.outcome.1", false, "WF23 R10 leakage guard")?;
    assert_edit_enabled(&mut pcr_driver, "edit.pcr.outcome.2", false, "WF23 R10 leakage guard")?;
    assert_edit_enabled(&mut pcr_driver, "edit.pcr.outcome.3", true, "WF23 R10 disjoint outcome")?;
    scroll_click(&mut pcr_driver, "edit.scroll", "edit.pcr.outcome.3")?;
    click_checked(&mut pcr_driver, "modal.ok")?;
    let rs_pcr = latest_result_sheet(&pcr_driver, &rid_pcr)?;
    assert_result_cell_visible_after_scroll(&mut pcr_driver, 0, 1)?;
    if rs_pcr.tabs.is_empty() {
        return Err(String::from("expected PCR ResultSheet tabs"));
    }

    Ok(())
}

// ============================================================================
// WF-24: Significance Brackets, Auto-Stars & ns, R1/R2 Predicates, Axes & Render
// ============================================================================

#[test]
fn wf24_human_significance_brackets_stars_and_axes() -> TestRes {
    let mut driver = init_driver()?;
    // Column defaults to 3 datasets, 12 rows -> +1 dataset (4 groups), -6 rows (6 replicates).
    let ds_id = create_table_via_welcome(&mut driver, "column", "WF24 Dose Response ANOVA", 1, -6, 0)?;

    let headers = ["Vehicle", "Low", "Medium", "High"];
    for (c, hdr) in headers.iter().enumerate() {
        edit_header_title(&mut driver, &ds_id, &format!("grid.col_title.{c}"), hdr)?;
    }
    let rows = [
        ["5.0", "5.3", "18.0", "28.0"],
        ["5.4", "5.6", "19.2", "30.1"],
        ["4.9", "5.2", "17.5", "27.8"],
        ["5.2", "5.5", "18.8", "29.4"],
        ["5.1", "5.4", "18.2", "28.6"],
        ["5.3", "5.7", "19.0", "30.5"],
    ];
    for (r, row_vals) in rows.iter().enumerate() {
        for (c, val) in row_vals.iter().enumerate() {
            enter_cell(&mut driver, &ds_id, r, c, 0, val)?;
        }
    }

    let rid = run_analysis_via_picker(&mut driver, "stats.one_way_anova", "One-way ANOVA")?;

    // Check R1 & R2 modal gating via Cmd+T.
    open_edit_params_dialog(&mut driver)?;
    // R1: Unpaired -> Repeated Measures tab disabled.
    assert_edit_enabled(&mut driver, "edit.tab.repeated_measures", false, "WF24 R1 Unpaired")?;
    scroll_click(&mut driver, "edit.scroll", "edit.paired_or_matched")?;
    assert_edit_enabled(&mut driver, "edit.tab.repeated_measures", true, "WF24 R1 Paired")?;
    click_checked(&mut driver, "edit.tab.repeated_measures")?;
    assert_edit_enabled(&mut driver, "edit.sphericity.geisser_greenhouse", true, "WF24 R1 RM sphericity")?;
    click_checked(&mut driver, "edit.tab.experimental_design")?;
    scroll_click(&mut driver, "edit.scroll", "edit.paired_or_matched")?;
    click_checked(&mut driver, "edit.anova.ordinary")?;

    // R2: Assume Gaussian OFF -> parametric post-hoc methods disabled, Dunn's selected.
    scroll_click(&mut driver, "edit.scroll", "edit.assume_gaussian")?;
    click_checked(&mut driver, "edit.tab.multiple_comparisons")?;
    assert_edit_enabled(&mut driver, "edit.post_hoc.tukey", false, "WF24 R2 Non-Gaussian")?;
    assert_edit_enabled(&mut driver, "edit.post_hoc.dunn", true, "WF24 R2 Non-Gaussian")?;
    click_checked(&mut driver, "edit.tab.experimental_design")?;
    scroll_click(&mut driver, "edit.scroll", "edit.assume_gaussian")?;
    click_checked(&mut driver, "edit.tab.multiple_comparisons")?;
    scroll_click(&mut driver, "edit.scroll", "edit.post_hoc.tukey")?;
    click_checked(&mut driver, "modal.ok")?;

    let rs_before = latest_result_sheet(&driver, &rid)?;
    if !matches!(
        rs_before.analysis.kind,
        AnalysisKind::OneWayAnova(ref a) if a.method == OneWayMethod::OrdinaryAnova && a.multiplicity.method == MultiplicityMethod::TukeyHsd
    ) {
        return Err(format!("expected OrdinaryAnova + TukeyHsd, got {:?}", rs_before.analysis.kind));
    }
    select_result_tab(&mut driver, &rs_before, ResultTabKind::MultipleComparisons)?;
    assert_result_cell_visible_after_scroll(&mut driver, 0, 2)?;
    let f_before = find_scalar_in_sheet(&rs_before, "F").ok_or_else(|| String::from("missing ANOVA F"))?;

    // Switch to GraphSheet and add significant + ns brackets with SummaryStars labels via Inspector.
    let gid = driver.app().document().graph_sheets.first().ok_or_else(|| String::from("missing GraphSheet for WF24"))?.id.clone();
    navigate_to_graph_sheet(&mut driver, &gid, false)?;
    click_checked(&mut driver, "inspector.tab.pairwise")?;
    // Default hide_non_significant is true; toggle it OFF so Vehicle vs Low (`ns`) is also bracketed.
    scroll_click(&mut driver, "scroll.inspector", "inspector.pw.sig_only")?;
    scroll_click(&mut driver, "scroll.inspector", "inspector.pw.auto_brackets")?;
    scroll_click(&mut driver, "scroll.inspector", "inspector.pw.lbl_stars")?;
    assert_parity(&mut driver, "WF24 pairwise brackets")?;

    let gs_pw = driver.app().document().graph_sheet(&gid).ok_or_else(|| String::from("missing GraphSheet"))?;
    let has_sig_rec = rs_before.pairwise_comparisons.iter().any(|r| r.significant);
    let has_ns_rec = rs_before.pairwise_comparisons.iter().any(|r| !r.significant);
    let all_brackets_include_ns = !gs_pw.spec.comparisons.is_empty()
        && gs_pw.spec.comparisons.iter().all(|b| b.label_mode == BracketLabelMode::SummaryStars && !b.hide_if_non_significant);
    if !has_sig_rec || !has_ns_rec || !all_brackets_include_ns {
        return Err(format!(
            "expected both sig ({has_sig_rec}) & ns ({has_ns_rec}) comparisons rendered with SummaryStars brackets (count={})",
            gs_pw.spec.comparisons.len()
        ));
    }

    // Configure Y-axis range/ticks and Log10 -> Linear in Inspector Axes tab.
    click_checked(&mut driver, "inspector.tab.axes")?;
    scroll_click(&mut driver, "scroll.inspector", "inspector.axes.target_y")?;
    let _ = click_and_type(driver_mut(&mut driver), &wid("inspector.axes.min.input"), "0")?;
    let _ = click_and_type(driver_mut(&mut driver), &wid("inspector.axes.max.input"), "40")?;
    let _ = click_and_type(driver_mut(&mut driver), &wid("inspector.axes.interval.input"), "10")?;
    scroll_click(&mut driver, "scroll.inspector", "inspector.axes.apply_range")?;

    let gs_axes = driver.app().document().graph_sheet(&gid).ok_or_else(|| String::from("missing GraphSheet after axis edit"))?;
    let y_seg = gs_axes.spec.y_axis.segments.first().ok_or_else(|| String::from("missing Y axis segment"))?;
    if y_seg.min != Some(0.0) || y_seg.max != Some(40.0) || y_seg.major_tick_interval != Some(10.0) {
        return Err(format!("unexpected Y axis bounds: {y_seg:?}"));
    }

    scroll_click(&mut driver, "scroll.inspector", "inspector.axes.scale_log10")?;
    let gs_log = driver.app().document().graph_sheet(&gid).ok_or_else(|| String::from("missing GraphSheet after log10"))?;
    if gs_log.spec.y_axis.segments.first().map(|s| s.scale) != Some(AxisScale::Log10) {
        return Err(String::from("expected Y axis scale switched to Log10"));
    }
    scroll_click(&mut driver, "scroll.inspector", "inspector.axes.scale_linear")?;

    assert_input_to_paint_budget(&driver, 500.0)?;
    // Verify rendered raster frame is non-empty and clean.
    let raster = driver.render_frame().map_err(|e| format!("render_frame failed: {e}"))?;
    if raster.width_px == 0 || raster.height_px == 0 || raster.rgba.iter().all(|&b| b == 0) {
        return Err(String::from("expected non-empty rendered raster frame for WF24"));
    }

    // Live upstream edit on High group changes ANOVA F.
    navigate_to_sheet(&mut driver, &ds_id)?;
    enter_cell(&mut driver, &ds_id, 0, 3, 0, "6.0")?;
    navigate_to_sheet(&mut driver, &rid)?;
    let rs_after = latest_result_sheet(&driver, &rid)?;
    let f_after = find_scalar_in_sheet(&rs_after, "F").ok_or_else(|| String::from("missing recomputed ANOVA F"))?;
    if (f_after - f_before).abs() < 1e-3 {
        return Err(format!("expected live edit recomputation on WF24 ANOVA F: before={f_before}, after={f_after}"));
    }

    // Read a rendered cell string cleanly to ensure helper utilization and non-empty text.
    let first_mc_row = find_tab(&rs_after, ResultTabKind::MultipleComparisons)?.1.rows.first().ok_or_else(|| String::from("empty MC rows"))?;
    if format_cell(first_mc_row.first().ok_or_else(|| String::from("empty MC row 0"))?).trim().is_empty() {
        return Err(String::from("expected non-empty MultipleComparisons row label"));
    }

    assert_input_to_paint_budget(&driver, 500.0)?;
    Ok(())
}
