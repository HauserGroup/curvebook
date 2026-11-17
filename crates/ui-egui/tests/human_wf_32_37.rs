//! `R10.13` / `R10.2` Wave 6 canonical human-path acceptance suite for `WF-32`..`WF-37`
//! (direct MultipleVariables statistics, qPCR `ddCt` formula pipelines, MV Kaplan-Meier,
//! MV 4PL dose-response, and hierarchical clustering + dendrograms).
//!
//! All tests drive raw pointer, wheel, keyboard, and IME events strictly through
//! `HumanPathDriver` with `human_path_only = true` (`human_wf_common::assert_zero_shortcut_compliance`),
//! never calling `execute_command`, `dispatch_gui_action`, or `app_mut` bypasses.

#![forbid(unsafe_code)]
#![allow(clippy::float_cmp, clippy::too_many_lines)]

mod human_wf_common;

use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, ClusterDistance, ClusterLinkage, GraphAnnotation, GraphKind, GraphSheet, MultivariatePlotMode, MultivariateSpec, ResultCell,
    ResultSheet, ResultTab, ResultTabKind, SheetId, TableArchetype,
};
use curvebook_ui_egui::{ActiveDialog, HumanPathDriver, KeyModifiers, UiApp, WidgetId};

use human_wf_common::{
    assert_input_to_paint_budget, assert_zero_shortcut_compliance, click_and_type, click_widget_center, default_viewport, hover_widget_center,
    new_human_driver, press_cmd_shortcut, press_key_combo, scroll_until_widget_visible, type_into_focused,
};

type TestRes = Result<(), String>;

const LATENCY_BUDGET_MS: f64 = 50.0;

// ============================================================================
// Shared Human-Path Drivers & Read-Only Verification Helpers
// ============================================================================

fn new_strict_human_driver() -> Result<HumanPathDriver, String> {
    let app = UiApp::new();
    let mut driver = new_human_driver(app, default_viewport());
    let _ = assert_zero_shortcut_compliance(&mut driver)?;
    Ok(driver)
}

fn verify_clean_render_and_parity(driver: &mut HumanPathDriver) -> TestRes {
    let _ = assert_zero_shortcut_compliance(driver)?;
    assert_input_to_paint_budget(driver, LATENCY_BUDGET_MS)?;
    let report = driver.verify_parity();
    if !report.is_clean() {
        return Err(format!("UI pixel/hit-box/action parity violations: {:?}", report.violations));
    }
    let _ = driver.render_frame().map_err(|e| format!("render_frame failed: {e}"))?;
    Ok(())
}

fn replace_focused_textbox(driver: &mut HumanPathDriver, wid: &WidgetId, text: &str) -> TestRes {
    let _ = click_widget_center(driver, wid)?;
    let _ = press_cmd_shortcut(driver, "a")?;
    let _ = press_key_combo(driver, "Backspace", KeyModifiers::default())?;
    let _ = type_into_focused(driver, text)?;
    Ok(())
}

fn ensure_widget_drawn(driver: &mut HumanPathDriver, id: &str) -> bool {
    driver.find_widget(id).is_some_and(|n| n.drawn_in_frame && n.bounds.visible_rect_px.width > 0 && n.bounds.visible_rect_px.height > 0)
}

fn select_sheet_in_navigator(driver: &mut HumanPathDriver, sheet_id: &str) -> TestRes {
    let nav_id = WidgetId::from(format!("nav.sheet.{sheet_id}"));
    if !ensure_widget_drawn(driver, nav_id.as_str()) {
        scroll_until_widget_visible(driver, &WidgetId::from("navigator.tree"), &nav_id, 140.0, 12)?;
    }
    let _ = click_widget_center(driver, &nav_id)?;
    let active = driver.app().document().active_sheet_id.as_ref().map(|s| s.as_str().to_owned());
    if active.as_deref() != Some(sheet_id) {
        return Err(format!("expected navigator selection of `{sheet_id}`, got {active:?}"));
    }
    Ok(())
}

fn wheel_container(driver: &mut HumanPathDriver, container_id: &WidgetId, delta_x: f64, delta_y: f64) -> TestRes {
    let _ = driver.wheel_widget(container_id.as_str(), delta_x, delta_y, KeyModifiers::default())?;
    Ok(())
}

fn scroll_container_until_drawn(
    driver: &mut HumanPathDriver,
    container_id: &WidgetId,
    target_id: &str,
    step_x: f64,
    step_y: f64,
    max_steps: usize,
) -> TestRes {
    for _ in 0..max_steps {
        if ensure_widget_drawn(driver, target_id) {
            return Ok(());
        }
        wheel_container(driver, container_id, step_x, step_y)?;
    }
    if ensure_widget_drawn(driver, target_id) {
        Ok(())
    } else {
        Err(format!("widget `{target_id}` did not become visible inside `{}` after {max_steps} scroll steps", container_id.as_str()))
    }
}

fn scroll_grid_widget_into_view(driver: &mut HumanPathDriver, sheet_id: &str, target_id: &str) -> TestRes {
    if ensure_widget_drawn(driver, target_id) {
        return Ok(());
    }
    let grid_id = WidgetId::from(format!("grid.sheet.{sheet_id}"));
    for _ in 0..8 {
        wheel_container(driver, &grid_id, 220.0, 0.0)?;
        if ensure_widget_drawn(driver, target_id) {
            return Ok(());
        }
    }
    for _ in 0..10 {
        wheel_container(driver, &grid_id, -220.0, 0.0)?;
        if ensure_widget_drawn(driver, target_id) {
            return Ok(());
        }
    }
    if scroll_container_until_drawn(driver, &grid_id, target_id, 0.0, 220.0, 40).is_ok() {
        return Ok(());
    }
    scroll_container_until_drawn(driver, &grid_id, target_id, 0.0, -220.0, 40)
}

fn scroll_grid_to_top_left(driver: &mut HumanPathDriver, sheet_id: &str) -> TestRes {
    let grid_id = WidgetId::from(format!("grid.sheet.{sheet_id}"));
    for _ in 0..16 {
        let off = driver.app().interaction.scroll_offsets.offset(grid_id.as_str());
        if off[0] <= 0.5 && off[1] <= 0.5 {
            break;
        }
        wheel_container(driver, &grid_id, -600.0, -600.0)?;
    }
    Ok(())
}

fn create_mv_table_via_welcome(
    driver: &mut HumanPathDriver,
    title: &str,
    cols_len: usize,
    rows_len: usize,
    auto_graph: bool,
) -> Result<String, String> {
    let _ = click_widget_center(driver, &WidgetId::from("welcome.arch.multiple_variables"))?;

    let mut guard = 0usize;
    while driver.app().nav.welcome.table_draft.data_set_count != cols_len && guard < 32 {
        let cur = driver.app().nav.welcome.table_draft.data_set_count;
        let btn = if cur < cols_len { "welcome.datasets.inc" } else { "welcome.datasets.dec" };
        let _ = click_widget_center(driver, &WidgetId::from(btn))?;
        guard += 1;
    }
    if driver.app().nav.welcome.table_draft.data_set_count != cols_len {
        return Err(format!("failed to set Welcome MV column count to {cols_len}; got {}", driver.app().nav.welcome.table_draft.data_set_count));
    }

    let mut row_guard = 0usize;
    while driver.app().nav.welcome.table_draft.row_count != rows_len && row_guard < 520 {
        let cur = driver.app().nav.welcome.table_draft.row_count;
        let btn = if cur < rows_len { "welcome.rows.inc" } else { "welcome.rows.dec" };
        let _ = click_widget_center(driver, &WidgetId::from(btn))?;
        row_guard += 1;
    }
    if driver.app().nav.welcome.table_draft.row_count != rows_len {
        return Err(format!("failed to set Welcome MV row count to {rows_len}; got {}", driver.app().nav.welcome.table_draft.row_count));
    }

    replace_focused_textbox(driver, &WidgetId::from("welcome.title.input"), title)?;
    if driver.app().nav.welcome.table_draft.create_default_graph != auto_graph {
        let _ = click_widget_center(driver, &WidgetId::from("welcome.toggle.auto_graph"))?;
    }

    let before_ids: Vec<String> = driver.app().document().data_sheets.iter().map(|s| s.id.as_str().to_owned()).collect();
    let _ = click_widget_center(driver, &WidgetId::from("welcome.create_cta"))?;
    let created = driver
        .app()
        .document()
        .data_sheets
        .iter()
        .find(|s| !before_ids.iter().any(|b| b == s.id.as_str()))
        .map(|s| s.id.as_str().to_owned())
        .ok_or_else(|| String::from("Welcome Create did not produce a new MultipleVariables DataSheet"))?;
    Ok(created)
}

fn configure_mv_headers_and_roles(driver: &mut HumanPathDriver, sheet_id: &str, cols: &[(&str, &str)]) -> TestRes {
    select_sheet_in_navigator(driver, sheet_id)?;
    for (col_idx, (title, role)) in cols.iter().enumerate() {
        let hdr_id = format!("grid.col_title.{col_idx}");
        scroll_grid_widget_into_view(driver, sheet_id, &hdr_id)?;
        let _ = click_widget_center(driver, &WidgetId::from(hdr_id))?;
        replace_focused_textbox(driver, &WidgetId::from("grid.title_editor.input"), title)?;
        let _ = click_widget_center(driver, &WidgetId::from("grid.title_editor.apply"))?;

        let badge_id = format!("mv_formula.role_badge.{col_idx}");
        if !ensure_widget_drawn(driver, &badge_id) {
            for _ in 0..16 {
                if ensure_widget_drawn(driver, "mv_formula.role_strip.next") {
                    let _ = click_widget_center(driver, &WidgetId::from("mv_formula.role_strip.next"))?;
                    if ensure_widget_drawn(driver, &badge_id) {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        let _ = click_widget_center(driver, &WidgetId::from(badge_id))?;
        let opt_id = format!("mv_formula.role_opt.{col_idx}.{role}");
        let _ = click_widget_center(driver, &WidgetId::from(opt_id))?;
    }
    scroll_grid_to_top_left(driver, sheet_id)?;
    Ok(())
}

fn edit_grid_cell_human(driver: &mut HumanPathDriver, sheet_id: &str, row: usize, col: usize, value: &str) -> TestRes {
    let cell_id = format!("grid.cell.{row}.{col}.0");
    scroll_grid_widget_into_view(driver, sheet_id, &cell_id)?;
    let wid = WidgetId::from(cell_id);
    let _ = click_widget_center(driver, &wid)?;
    let _ = press_key_combo(driver, "Delete", KeyModifiers::default())?;
    let [cx, cy] = human_wf_common::widget_hit_center(driver, &wid)?;
    let _ = driver.double_click_logical(cx, cy)?;
    let _ = driver.type_text(value)?;
    let _ = press_key_combo(driver, "Enter", KeyModifiers::default())?;
    Ok(())
}

fn enter_mv_rows(driver: &mut HumanPathDriver, sheet_id: &str, rows: &[&[&str]]) -> TestRes {
    select_sheet_in_navigator(driver, sheet_id)?;
    for (r_idx, row_vals) in rows.iter().enumerate() {
        for (c_idx, val) in row_vals.iter().enumerate() {
            edit_grid_cell_human(driver, sheet_id, r_idx, c_idx, val)?;
        }
    }
    scroll_grid_to_top_left(driver, sheet_id)?;
    Ok(())
}

fn set_mv_formula_column(driver: &mut HumanPathDriver, sheet_id: &str, col_idx: usize, formula: &str) -> TestRes {
    select_sheet_in_navigator(driver, sheet_id)?;
    let fx_id = format!("formula_bar.open_col.{col_idx}");
    scroll_grid_widget_into_view(driver, sheet_id, &fx_id)?;
    let _ = click_widget_center(driver, &WidgetId::from(fx_id))?;
    replace_focused_textbox(driver, &WidgetId::from("formula_bar.input"), formula)?;
    let _ = click_widget_center(driver, &WidgetId::from("formula_bar.commit"))?;
    Ok(())
}

fn open_command_palette_and_run(driver: &mut HumanPathDriver, query: &str) -> TestRes {
    let _ = press_cmd_shortcut(driver, "k")?;
    let _ = click_and_type(driver, &WidgetId::from("palette.query"), query)?;
    let _ = press_key_combo(driver, "Enter", KeyModifiers::default())?;
    Ok(())
}

fn open_and_run_analyze(driver: &mut HumanPathDriver, entry_id: &str) -> Result<String, String> {
    let before = result_sheet_ids(driver);
    let _ = click_widget_center(driver, &WidgetId::from("toolbar.analyze"))?;
    replace_focused_textbox(driver, &WidgetId::from("analyze.search"), entry_id)?;
    let target = WidgetId::from(format!("analyze.entry.{entry_id}"));
    if !ensure_widget_drawn(driver, target.as_str()) {
        scroll_until_widget_visible(driver, &WidgetId::from("analyze.entries_list"), &target, 160.0, 12)?;
    }
    let _ = click_widget_center(driver, &target)?;
    let _ = click_widget_center(driver, &WidgetId::from("modal.ok"))?;
    new_result_id_after(driver, &before)
}

fn verify_result_tabs_and_scroll(driver: &mut HumanPathDriver, result_id: &str, expected_tabs: &[&str]) -> TestRes {
    select_sheet_in_navigator(driver, result_id)?;
    for tid in expected_tabs {
        let tab_wid = WidgetId::from(format!("result.tab.{tid}"));
        if !ensure_widget_drawn(driver, tab_wid.as_str()) && ensure_widget_drawn(driver, "result.tabs.next") {
            for _ in 0..8 {
                let _ = click_widget_center(driver, &WidgetId::from("result.tabs.next"))?;
                if ensure_widget_drawn(driver, tab_wid.as_str()) {
                    break;
                }
            }
        }
        let _ = click_widget_center(driver, &tab_wid)?;
        if ensure_widget_drawn(driver, "result.table") {
            let tbl = WidgetId::from("result.table");
            wheel_container(driver, &tbl, 180.0, 180.0)?;
            wheel_container(driver, &tbl, -180.0, -180.0)?;
        }
    }
    if ensure_widget_drawn(driver, "result.interpret_toggle_btn") {
        let _ = click_widget_center(driver, &WidgetId::from("result.interpret_toggle_btn"))?;
        let _ = click_widget_center(driver, &WidgetId::from("result.interpret_toggle_btn"))?;
    }
    Ok(())
}

fn assert_qol7_diagnostics_only(driver: &mut HumanPathDriver, result_id: &str) -> TestRes {
    select_sheet_in_navigator(driver, result_id)?;
    let tree = driver.layout_tree();
    if let Some(banner) = tree.node("result.diagnostic_banner") {
        let label_lc = banner.a11y.label.to_ascii_lowercase();
        for forbidden in ["switch to", "recommend", "run kruskal", "run welch", "run mann"] {
            if label_lc.contains(forbidden) {
                return Err(format!("QOL-7 violation in diagnostic banner: `{}`", banner.a11y.label));
            }
        }
    }
    for n in &tree.nodes {
        let id = n.id.as_str();
        if id.starts_with("result.recommend") || id.starts_with("result.remediate") || id.starts_with("result.switch_test") {
            return Err(format!("QOL-7 violation: unexpected sibling-analysis remediation widget `{id}`"));
        }
    }
    Ok(())
}

fn exclude_cell_with_reason_human(driver: &mut HumanPathDriver, sheet_id: &str, row: usize, col: usize, note: &str) -> TestRes {
    select_sheet_in_navigator(driver, sheet_id)?;
    let cell_id = format!("grid.cell.{row}.{col}.0");
    scroll_grid_widget_into_view(driver, sheet_id, &cell_id)?;
    let _ = click_widget_center(driver, &WidgetId::from(cell_id))?;
    open_command_palette_and_run(driver, "Exclude Selection with Auditable Reason")?;
    if ensure_widget_drawn(driver, "exclude.tag.1") {
        let _ = click_widget_center(driver, &WidgetId::from("exclude.tag.1"))?;
    }
    replace_focused_textbox(driver, &WidgetId::from("exclude.note_input"), note)?;
    let _ = click_widget_center(driver, &WidgetId::from("exclude.apply_note"))?;
    let _ = click_widget_center(driver, &WidgetId::from("modal.ok"))?;
    Ok(())
}

fn open_change_graph_type_for_sheet(driver: &mut HumanPathDriver, graph_id: &str) -> TestRes {
    select_sheet_in_navigator(driver, graph_id)?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = click_widget_center(driver, &WidgetId::from("toolbar.change_graph"))?;
    }
    Ok(())
}

fn configure_mv_categorical_summary_inspector(driver: &mut HumanPathDriver, graph_id: &str, x_col: usize, y_col: usize, color_col: usize) -> TestRes {
    select_sheet_in_navigator(driver, graph_id)?;
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = press_key_combo(driver, "Escape", KeyModifiers::default())?;
    }
    let _ = click_widget_center(driver, &WidgetId::from("inspector.tab.multi"))?;
    let _ = click_widget_center(driver, &WidgetId::from("inspector.mv.mode_cat_summary"))?;
    let _ = click_widget_center(driver, &WidgetId::from("inspector.mv.x_btn"))?;
    let _ = click_widget_center(driver, &WidgetId::from(format!("inspector.popup.mv_x.item_{x_col}")))?;
    let _ = click_widget_center(driver, &WidgetId::from("inspector.mv.y_btn"))?;
    let _ = click_widget_center(driver, &WidgetId::from(format!("inspector.popup.mv_y.item_{y_col}")))?;
    let _ = click_widget_center(driver, &WidgetId::from("inspector.mv.color_btn"))?;
    let _ = click_widget_center(driver, &WidgetId::from(format!("inspector.popup.mv_color.item_{}", color_col + 1)))?;
    let _ = click_widget_center(driver, &WidgetId::from("inspector.mv.color_cat"))?;
    Ok(())
}

fn result_sheet_ids(driver: &HumanPathDriver) -> Vec<String> {
    driver.app().document().result_sheets.iter().map(|r| r.id.as_str().to_owned()).collect()
}

fn new_result_id_after(driver: &HumanPathDriver, before: &[String]) -> Result<String, String> {
    driver
        .app()
        .document()
        .result_sheets
        .iter()
        .rev()
        .find(|r| !before.iter().any(|b| b == r.id.as_str()))
        .map(|r| r.id.as_str().to_owned())
        .ok_or_else(|| String::from("no new ResultSheet produced"))
}

fn result_sheet<'a>(driver: &'a HumanPathDriver, id: &str) -> Result<&'a ResultSheet, String> {
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    driver.app().document().result_sheet(&sid).ok_or_else(|| format!("missing ResultSheet `{id}`"))
}

fn graph_sheet<'a>(driver: &'a HumanPathDriver, id: &str) -> Result<&'a GraphSheet, String> {
    let sid = SheetId::try_new(id).map_err(|e| e.to_string())?;
    driver.app().document().graph_sheet(&sid).ok_or_else(|| format!("missing GraphSheet `{id}`"))
}

fn linked_graph_id(driver: &HumanPathDriver, source_id: &str) -> Result<String, String> {
    driver
        .app()
        .document()
        .graph_sheets
        .iter()
        .find(|g| g.spec.primary_source_sheet_id.as_str() == source_id)
        .map(|g| g.id.as_str().to_owned())
        .ok_or_else(|| format!("no GraphSheet linked to `{source_id}`"))
}

fn scalar_f64(rs: &ResultSheet, key: &str) -> Result<f64, String> {
    rs.scalars.iter().find(|s| s.key == key).map(|s| s.value).ok_or_else(|| format!("missing scalar `{key}` in ResultSheet `{}`", rs.id))
}

fn tab_by_id<'a>(rs: &'a ResultSheet, id: &str) -> Result<&'a ResultTab, String> {
    rs.tabs.iter().find(|t| t.id == id).ok_or_else(|| format!("missing ResultTab `{id}`"))
}

fn tab_by_kind(rs: &ResultSheet, kind: ResultTabKind) -> Result<&ResultTab, String> {
    rs.tabs.iter().find(|t| t.kind == kind).ok_or_else(|| format!("missing ResultTabKind {kind:?}"))
}

fn cell_num(cell: &ResultCell, ctx: &str) -> Result<f64, String> {
    match cell {
        ResultCell::Number { value, .. } => Ok(*value),
        ResultCell::PValue { raw, .. } => Ok(*raw),
        other => Err(format!("expected numeric ResultCell for {ctx}, got {other:?}")),
    }
}

fn tab_cell_f64(tab: &ResultTab, row_label: &str, col_header: &str) -> Result<f64, String> {
    let r_idx = tab
        .row_labels
        .iter()
        .position(|l| l == row_label)
        .ok_or_else(|| format!("missing row `{row_label}` in tab `{}`: {:?}", tab.id, tab.row_labels))?;
    let c_idx = tab
        .column_headers
        .iter()
        .position(|c| c == col_header)
        .ok_or_else(|| format!("missing column `{col_header}` in tab `{}`: {:?}", tab.id, tab.column_headers))?;
    let cell = tab.rows.get(r_idx).and_then(|r| r.get(c_idx)).ok_or_else(|| format!("missing cell ({r_idx}, {c_idx})"))?;
    cell_num(cell, &format!("{}[{row_label}, {col_header}]", tab.id))
}

fn mv_num_cell(driver: &HumanPathDriver, sheet_id: &str, col: usize, row: usize) -> Result<f64, String> {
    let sid = SheetId::try_new(sheet_id).map_err(|e| e.to_string())?;
    let ds = driver.app().document().data_sheet(&sid).ok_or_else(|| format!("missing `{sheet_id}`"))?;
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return Err(format!("`{sheet_id}` is not MultipleVariables"));
    };
    let var = mv.variables.get(col).ok_or_else(|| format!("missing MV column {col}"))?;
    let cell = var.cells.get(row).ok_or_else(|| format!("missing MV row {row}"))?;
    cell.active_number().ok_or_else(|| format!("expected finite numeric cell at ({row}, {col}), got {:?}", cell.value))
}

fn assert_approx(actual: f64, expected: f64, tol: f64, label: &str) -> TestRes {
    if !actual.is_finite() || (actual - expected).abs() > tol {
        return Err(format!("{label}: expected {expected} +/- {tol}, got {actual}"));
    }
    Ok(())
}

// ============================================================================
// WF-32: Direct MultipleVariables Welch's t-Test & Categorical Summary
// ============================================================================

#[test]
fn wf32_human_direct_mv_welch_t_and_categorical_summary() -> TestRes {
    let mut driver = new_strict_human_driver()?;
    let mv_id = create_mv_table_via_welcome(&mut driver, "Biomarker Cohort MV", 3, 16, true)?;
    configure_mv_headers_and_roles(&mut driver, &mv_id, &[("SampleID", "label"), ("Treatment", "categorical"), ("Biomarker", "continuous")])?;
    let rows: [&[&str]; 16] = [
        &["V1", "Vehicle", "18.5"],
        &["V2", "Vehicle", "20.2"],
        &["V3", "Vehicle", "21.0"],
        &["V4", "Vehicle", "22.4"],
        &["V5", "Vehicle", "19.8"],
        &["V6", "Vehicle", "23.1"],
        &["V7", "Vehicle", "20.9"],
        &["V8", "Vehicle", "21.5"],
        &["T1", "Treated", "31.0"],
        &["T2", "Treated", "34.5"],
        &["T3", "Treated", "42.0"],
        &["T4", "Treated", "38.2"],
        &["T5", "Treated", "46.8"],
        &["T6", "Treated", "35.4"],
        &["T7", "Treated", "40.1"],
        &["T8", "Treated", "52.0"],
    ];
    enter_mv_rows(&mut driver, &mv_id, &rows)?;
    let mv_graph = linked_graph_id(&driver, &mv_id)?;

    // Run direct MV two-group analysis, then configure Welch + column bindings via Cmd+T.
    let welch_id = open_and_run_analyze(&mut driver, "stats.two_group")?;
    let _ = press_cmd_shortcut(&mut driver, "t")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.two_group.welch"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.two_group.group.1"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.two_group.resp.2"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    verify_result_tabs_and_scroll(&mut driver, &welch_id, &["two-group"])?;
    let rs_welch = result_sheet(&driver, &welch_id)?;
    let p_before = scalar_f64(rs_welch, "PValue")?;
    let cohen_before = scalar_f64(rs_welch, "CohensD")?;
    let hedges_before = scalar_f64(rs_welch, "HedgesG")?;
    if !(p_before < 0.005 && cohen_before.abs() > 1.5 && hedges_before.abs() > 1.4) {
        return Err(format!("unexpected Welch stats: p={p_before}, d={cohen_before}, g={hedges_before}"));
    }
    let tg_tab = tab_by_id(rs_welch, "two-group")?;
    let tg_col = tg_tab.column_headers.first().ok_or("two-group tab missing column headers")?.clone();
    let _f_val = tab_cell_f64(tg_tab, "F-test to compare variances (F)", &tg_col)?;
    let _f_p = tab_cell_f64(tg_tab, "F-test P value", &tg_col)?;
    let _glass = tab_cell_f64(tg_tab, "Glass's Delta", &tg_col)?;

    // Configure MV graph to CategoricalSummary mode in live Inspector.
    configure_mv_categorical_summary_inspector(&mut driver, &mv_graph, 1, 2, 1)?;
    let gs = graph_sheet(&driver, &mv_graph)?;
    if !matches!(gs.spec.multivariate.as_ref().map(|m| &m.plot_mode), Some(MultivariatePlotMode::CategoricalSummary { .. })) {
        return Err(format!("expected CategoricalSummary plot_mode on `{mv_graph}`"));
    }
    verify_clean_render_and_parity(&mut driver)?;

    // Live reactivity: auditable exclusion of outlier T8 + upstream cell edit on V1.
    exclude_cell_with_reason_human(&mut driver, &mv_id, 15, 2, "WF32 extreme readout")?;
    let rs_excl = result_sheet(&driver, &welch_id)?;
    let cohen_excl = scalar_f64(rs_excl, "CohensD")?;
    let t_excl = scalar_f64(rs_excl, "t")?;
    if (cohen_excl - cohen_before).abs() < 0.05 || !t_excl.is_finite() {
        return Err(format!("Welch CohensD did not recompute after exclusion: {cohen_before} -> {cohen_excl}"));
    }

    edit_grid_cell_human(&mut driver, &mv_id, 0, 2, "25.0")?;
    let p_edited = scalar_f64(result_sheet(&driver, &welch_id)?, "PValue")?;
    if (p_edited - p_before).abs() < 1e-6 {
        return Err(format!("Welch PValue did not update after V1 edit: {p_before} -> {p_edited}"));
    }
    verify_clean_render_and_parity(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-33: Direct MultipleVariables 2-Way -> 3-Way Factorial ANOVA
// ============================================================================

#[test]
fn wf33_human_direct_mv_multifactor_anova() -> TestRes {
    let mut driver = new_strict_human_driver()?;
    let mv_id = create_mv_table_via_welcome(&mut driver, "2x2x2 Factorial Drug Study", 5, 24, true)?;
    configure_mv_headers_and_roles(
        &mut driver,
        &mv_id,
        &[("SampleID", "label"), ("Genotype", "categorical"), ("Drug", "categorical"), ("Timepoint", "categorical"), ("Response", "continuous")],
    )?;
    let rows: [&[&str]; 24] = [
        &["S01", "WT", "Vehicle", "Early", "10.0"],
        &["S02", "WT", "Vehicle", "Early", "11.2"],
        &["S03", "WT", "Vehicle", "Early", "10.5"],
        &["S04", "WT", "Vehicle", "Late", "12.0"],
        &["S05", "WT", "Vehicle", "Late", "12.8"],
        &["S06", "WT", "Vehicle", "Late", "12.2"],
        &["S07", "WT", "DrugX", "Early", "18.0"],
        &["S08", "WT", "DrugX", "Early", "19.4"],
        &["S09", "WT", "DrugX", "Early", "18.6"],
        &["S10", "WT", "DrugX", "Late", "26.0"],
        &["S11", "WT", "DrugX", "Late", "27.5"],
        &["S12", "WT", "DrugX", "Late", "26.8"],
        &["S13", "KO", "Vehicle", "Early", "11.0"],
        &["S14", "KO", "Vehicle", "Early", "10.8"],
        &["S15", "KO", "Vehicle", "Early", "11.4"],
        &["S16", "KO", "Vehicle", "Late", "11.6"],
        &["S17", "KO", "Vehicle", "Late", "12.1"],
        &["S18", "KO", "Vehicle", "Late", "11.8"],
        &["S19", "KO", "DrugX", "Early", "13.0"],
        &["S20", "KO", "DrugX", "Early", "13.6"],
        &["S21", "KO", "DrugX", "Early", "13.2"],
        &["S22", "KO", "DrugX", "Late", "14.2"],
        &["S23", "KO", "DrugX", "Late", "14.9"],
        &["S24", "KO", "DrugX", "Late", "14.5"],
    ];
    enter_mv_rows(&mut driver, &mv_id, &rows)?;
    let mv_graph = linked_graph_id(&driver, &mv_id)?;

    // Start with 2-way Factorial ANOVA (Genotype x Drug -> Response) via Cmd+T.
    let anova_id = open_and_run_analyze(&mut driver, "stats.factorial_anova")?;
    let _ = press_cmd_shortcut(&mut driver, "t")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.factorial.ss.type3"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.tab.factor_settings"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.fac.resp.4"))?;
    // Ensure Genotype(1) and Drug(2) are checked, Timepoint(3) unchecked for 2-way baseline.
    let tree_f = driver.layout_tree();
    for (idx, want_on) in [(1usize, true), (2usize, true), (3usize, false)] {
        let fid = format!("edit.fac.col.{idx}");
        let is_on = tree_f.node(&fid).and_then(|n| n.a11y.selected).unwrap_or(false);
        if is_on != want_on {
            let _ = click_widget_center(&mut driver, &WidgetId::from(fid))?;
        }
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.tab.multiple_comparisons"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.post_hoc.tukey"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let rs2 = result_sheet(&driver, &anova_id)?;
    let tab2 = tab_by_id(rs2, "factorial-anova")?;
    let _p_eta = tab_cell_f64(tab2, "Genotype × Drug", "Partial eta squared")?;
    let _eta = tab_cell_f64(tab2, "Genotype × Drug", "Eta squared")?;
    let _mc = tab_by_kind(rs2, ResultTabKind::MultipleComparisons)?;

    // Expand to 3-way (Genotype x Drug x Timepoint) via Cmd+T.
    let _ = press_cmd_shortcut(&mut driver, "t")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.factorial.three_way"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.tab.factor_settings"))?;
    let t3_tree = driver.layout_tree();
    if !t3_tree.node("edit.fac.col.3").and_then(|n| n.a11y.selected).unwrap_or(false) {
        let _ = click_widget_center(&mut driver, &WidgetId::from("edit.fac.col.3"))?;
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    verify_result_tabs_and_scroll(&mut driver, &anova_id, &["factorial-anova"])?;
    assert_qol7_diagnostics_only(&mut driver, &anova_id)?;
    let rs3 = result_sheet(&driver, &anova_id)?;
    let tab3 = tab_by_id(rs3, "factorial-anova")?;
    let _three_way_f = scalar_f64(rs3, "Genotype × Drug × Timepoint: F").or_else(|_| scalar_f64(rs3, "Genotype:Drug:Timepoint: F"))?;
    let _three_way_p = scalar_f64(rs3, "Genotype × Drug × Timepoint: P").or_else(|_| scalar_f64(rs3, "Genotype:Drug:Timepoint: P"))?;
    let ms_res_before = scalar_f64(rs3, "MS residual")?;
    let _three_way_eta = tab_cell_f64(tab3, "Genotype × Drug × Timepoint", "Partial eta squared")
        .or_else(|_| tab_cell_f64(tab3, "Genotype:Drug:Timepoint", "Partial eta squared"))?;

    configure_mv_categorical_summary_inspector(&mut driver, &mv_graph, 2, 4, 1)?;
    verify_clean_render_and_parity(&mut driver)?;

    // Live cell edit on Response recomputes MS residual.
    select_sheet_in_navigator(&mut driver, &mv_id)?;
    edit_grid_cell_human(&mut driver, &mv_id, 9, 4, "35.0")?;
    let ms_res_after = scalar_f64(result_sheet(&driver, &anova_id)?, "MS residual")?;
    if (ms_res_after - ms_res_before).abs() < 1e-4 {
        return Err(format!("3-way ANOVA MS residual did not update after Response edit: {ms_res_before} -> {ms_res_after}"));
    }
    verify_clean_render_and_parity(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-34: qPCR ddCt Formula Chain (`MEANIF`, `2^-ddCt`) & Direct MV Welch t-Test
// ============================================================================

#[test]
fn wf34_human_qpcr_ddct_calculated_variables_and_welch() -> TestRes {
    let mut driver = new_strict_human_driver()?;
    let mv_id = create_mv_table_via_welcome(&mut driver, "qPCR ddCt Pipeline", 9, 8, true)?;
    configure_mv_headers_and_roles(
        &mut driver,
        &mv_id,
        &[
            ("SampleID", "label"),
            ("Condition", "categorical"),
            ("TargetCt", "continuous"),
            ("RefCt", "continuous"),
            ("DeltaCt", "continuous"),
            ("CalibratorMean", "continuous"),
            ("DeltaDeltaCt", "continuous"),
            ("FoldChange", "continuous"),
            ("Log2FC", "continuous"),
        ],
    )?;
    let rows: [&[&str]; 8] = [
        &["C1", "Control", "22.0", "18.0"],
        &["C2", "Control", "22.2", "18.1"],
        &["C3", "Control", "21.9", "17.9"],
        &["C4", "Control", "22.1", "18.2"],
        &["P1", "Perturbed", "19.0", "18.0"],
        &["P2", "Perturbed", "19.1", "18.2"],
        &["P3", "Perturbed", "18.9", "17.9"],
        &["P4", "Perturbed", "19.2", "18.1"],
    ];
    enter_mv_rows(&mut driver, &mv_id, &rows)?;
    let mv_graph = linked_graph_id(&driver, &mv_id)?;

    // Define the 5 chained qPCR calculated variables via human formula bar.
    set_mv_formula_column(&mut driver, &mv_id, 4, "= [TargetCt] - [RefCt]")?;
    set_mv_formula_column(&mut driver, &mv_id, 5, "= MEANIF([Condition] == \"Control\", [DeltaCt])")?;
    set_mv_formula_column(&mut driver, &mv_id, 6, "= [DeltaCt] - [CalibratorMean]")?;
    set_mv_formula_column(&mut driver, &mv_id, 7, "= 2 ^ (-[DeltaDeltaCt])")?;
    set_mv_formula_column(&mut driver, &mv_id, 8, "= -[DeltaDeltaCt]")?;

    // Toggle Formula Definitions drawer (`R10.8`) cleanly via right-hand variable panel.
    if ensure_widget_drawn(&mut driver, "mv_formula.panel_defs_toggle") {
        let _ = click_widget_center(&mut driver, &WidgetId::from("mv_formula.panel_defs_toggle"))?;
        let _ = click_widget_center(&mut driver, &WidgetId::from("mv_formula.panel_defs_toggle"))?;
    }

    assert_approx(mv_num_cell(&driver, &mv_id, 5, 0)?, 4.0, 1e-6, "CalibratorMean")?;
    assert_approx(mv_num_cell(&driver, &mv_id, 6, 4)?, -3.0, 1e-6, "P1 DeltaDeltaCt")?;
    assert_approx(mv_num_cell(&driver, &mv_id, 7, 4)?, 8.0, 1e-5, "P1 FoldChange")?;
    assert_approx(mv_num_cell(&driver, &mv_id, 8, 4)?, 3.0, 1e-6, "P1 Log2FC")?;

    // Live formula-chain cascade BEFORE analysis edit check: alter P1 TargetCt 19.0 -> 21.0 and restore.
    edit_grid_cell_human(&mut driver, &mv_id, 4, 2, "21.0")?;
    assert_approx(mv_num_cell(&driver, &mv_id, 4, 4)?, 3.0, 1e-6, "P1 DeltaCt after edit")?;
    assert_approx(mv_num_cell(&driver, &mv_id, 7, 4)?, 2.0, 1e-5, "P1 FoldChange after edit")?;
    edit_grid_cell_human(&mut driver, &mv_id, 4, 2, "19.0")?;
    assert_approx(mv_num_cell(&driver, &mv_id, 7, 4)?, 8.0, 1e-5, "P1 FoldChange restored")?;

    // Direct MV Welch t-test on Condition (col 1) vs Log2FC (col 8).
    let welch_id = open_and_run_analyze(&mut driver, "stats.two_group")?;
    let _ = press_cmd_shortcut(&mut driver, "t")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.two_group.welch"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.two_group.group.1"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.two_group.resp.8"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    verify_result_tabs_and_scroll(&mut driver, &welch_id, &["two-group"])?;
    let rs_welch = result_sheet(&driver, &welch_id)?;
    let p_before = scalar_f64(rs_welch, "PValue")?;
    let cohen_before = scalar_f64(rs_welch, "CohensD")?;
    if !(p_before < 0.001 && cohen_before.abs() > 3.0) {
        return Err(format!("unexpected qPCR Welch stats: p={p_before}, d={cohen_before}"));
    }

    configure_mv_categorical_summary_inspector(&mut driver, &mv_graph, 1, 8, 1)?;
    verify_clean_render_and_parity(&mut driver)?;

    // Upstream TargetCt edit cascades through all 5 calculated columns into Welch PValue.
    select_sheet_in_navigator(&mut driver, &mv_id)?;
    edit_grid_cell_human(&mut driver, &mv_id, 4, 2, "21.0")?;
    let p_after = scalar_f64(result_sheet(&driver, &welch_id)?, "PValue")?;
    if (p_after - p_before).abs() < 1e-8 {
        return Err(format!("qPCR Welch PValue did not cascade from TargetCt edit: {p_before} -> {p_after}"));
    }
    verify_clean_render_and_parity(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-35: Direct MultipleVariables Kaplan-Meier Survival & Large-N Grid Scroll
// ============================================================================

#[test]
fn wf35_human_direct_mv_kaplan_meier_survival() -> TestRes {
    let mut driver = new_strict_human_driver()?;
    // Create 120-row MV table via Welcome to verify large-N virtualized table wheel scrolling.
    let mv_id = create_mv_table_via_welcome(&mut driver, "Survival MV Trial", 4, 120, true)?;
    configure_mv_headers_and_roles(
        &mut driver,
        &mv_id,
        &[("PatientID", "label"), ("Cohort", "categorical"), ("Days", "continuous"), ("Event", "continuous")],
    )?;

    // Large-N vertical table scroll down to deep virtualized row 115 and back to row 0.
    let grid_id = WidgetId::from(format!("grid.sheet.{mv_id}"));
    scroll_container_until_drawn(&mut driver, &grid_id, "grid.cell.115.0.0", 0.0, 240.0, 30)?;
    let _ = hover_widget_center(&mut driver, &WidgetId::from("grid.cell.115.0.0"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("grid.cell.115.0.0"))?;
    scroll_grid_to_top_left(&mut driver, &mv_id)?;
    verify_clean_render_and_parity(&mut driver)?;

    let rows: [&[&str]; 18] = [
        &["P01", "Placebo", "10", "1"],
        &["P02", "Placebo", "14", "1"],
        &["P03", "Placebo", "18", "1"],
        &["P04", "Placebo", "22", "1"],
        &["P05", "Placebo", "26", "0"],
        &["P06", "Placebo", "30", "1"],
        &["L01", "LowDose", "20", "1"],
        &["L02", "LowDose", "28", "1"],
        &["L03", "LowDose", "35", "0"],
        &["L04", "LowDose", "42", "1"],
        &["L05", "LowDose", "48", "0"],
        &["L06", "LowDose", "55", "1"],
        &["H01", "HighDose", "40", "1"],
        &["H02", "HighDose", "56", "0"],
        &["H03", "HighDose", "64", "0"],
        &["H04", "HighDose", "72", "1"],
        &["H05", "HighDose", "80", "0"],
        &["H06", "HighDose", "90", "0"],
    ];
    enter_mv_rows(&mut driver, &mv_id, &rows)?;

    // Run MV Kaplan-Meier and configure log-rank, Gehan-Breslow, trend, and Holm-Sidak pairwise.
    let km_id = open_and_run_analyze(&mut driver, "stats.survival_km")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("result.edit_params_btn"))?;
    let p_tree = driver.layout_tree();
    for tid in ["edit.km.log_rank_mantel_cox", "edit.km.gehan_breslow_wilcoxon", "edit.km.trend_test"] {
        if !p_tree.node(tid).and_then(|n| n.a11y.checked).unwrap_or(false) {
            let _ = click_widget_center(&mut driver, &WidgetId::from(tid))?;
        }
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.tab.multiple_comparisons"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.post_hoc.holm_sidak"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    verify_result_tabs_and_scroll(&mut driver, &km_id, &["km-summary", "number-at-risk", "survival-pairwise"])?;
    let rs_km = result_sheet(&driver, &km_id)?;
    let summary_tab = tab_by_id(rs_km, "km-summary")?;
    let _placebo_med = tab_cell_f64(summary_tab, "Placebo", "Median survival")?;
    let _nar_tab = tab_by_id(rs_km, "number-at-risk")?;
    let pw_tab = tab_by_id(rs_km, "survival-pairwise")?;
    if pw_tab.rows.is_empty() || rs_km.fitted_curves.len() < 3 {
        return Err(format!("unexpected KM survival output: pairwise rows={}, curves={}", pw_tab.rows.len(), rs_km.fitted_curves.len()));
    }

    // Create a Survival graph via Navigator and switch its plot family to Kaplan-Meier.
    let before_graphs: Vec<String> = driver.app().document().graph_sheets.iter().map(|g| g.id.as_str().to_owned()).collect();
    let _ = click_widget_center(&mut driver, &WidgetId::from("nav.create.graphs"))?;
    let km_graph = driver
        .app()
        .document()
        .graph_sheets
        .iter()
        .rev()
        .find(|g| !before_graphs.iter().any(|b| b == g.id.as_str()))
        .map(|g| g.id.as_str().to_owned())
        .ok_or_else(|| String::from("nav.create.graphs did not create a GraphSheet"))?;
    open_change_graph_type_for_sheet(&mut driver, &km_graph)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("graph_type.show_all"))?;
    scroll_until_widget_visible(&mut driver, &WidgetId::from("graph_type.panel"), &WidgetId::from("graph_type.card.survival.km"), 140.0, 16)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("graph_type.card.survival.km"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    if graph_sheet(&driver, &km_graph)?.spec.kind != GraphKind::Survival {
        return Err(String::from("expected KM GraphSheet to become GraphKind::Survival"));
    }
    verify_clean_render_and_parity(&mut driver)?;

    // Stage 5: live Cmd+T toggle + censoring cell edit (Event 1 -> 0) before Median survival threshold UI check.
    select_sheet_in_navigator(&mut driver, &km_id)?;
    let _ = press_cmd_shortcut(&mut driver, "t")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.km.trend_test"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    let events_before = tab_cell_f64(tab_by_id(result_sheet(&driver, &km_id)?, "km-summary")?, "Placebo", "Events")?;
    select_sheet_in_navigator(&mut driver, &mv_id)?;
    edit_grid_cell_human(&mut driver, &mv_id, 1, 3, "0")?;
    let events_after = tab_cell_f64(tab_by_id(result_sheet(&driver, &km_id)?, "km-summary")?, "Placebo", "Events")?;
    assert_approx(events_after, events_before - 1.0, 1e-6, "Placebo Events after censoring")?;

    // Configure Median survival dashed reference threshold via FormatAxes dialog.
    select_sheet_in_navigator(&mut driver, &km_graph)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("toolbar.axes"))?;
    let median_y = if graph_sheet(&driver, &km_graph)?.spec.y_axis.segments.first().and_then(|s| s.max).unwrap_or(100.0) <= 1.5 { 0.5 } else { 50.0 };
    let thr_json = format!("[{{\"value\":{median_y},\"label\":\"Median survival\",\"dash_style\":\"dashed\"}}]");
    replace_focused_textbox(&mut driver, &WidgetId::from("axes.y_thresholds"), &thr_json)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    if graph_sheet(&driver, &km_graph)?.spec.y_axis.thresholds.is_empty() {
        return Err(String::from("expected Median survival Y threshold on KM graph"));
    }
    verify_clean_render_and_parity(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-36: Direct MultipleVariables 4PL Dose-Response + Live TeX Model Equation
// ============================================================================

#[test]
fn wf36_human_direct_mv_4pl_dose_response() -> TestRes {
    let mut driver = new_strict_human_driver()?;
    let mv_id = create_mv_table_via_welcome(&mut driver, "Dose-Response 4PL MV", 4, 16, true)?;
    configure_mv_headers_and_roles(
        &mut driver,
        &mv_id,
        &[("Well", "label"), ("Compound", "categorical"), ("LogConcM", "continuous"), ("Response", "continuous")],
    )?;
    let rows: [&[&str]; 16] = [
        &["A1", "InhibitorA", "-10.0", "99.0"],
        &["A2", "InhibitorA", "-9.0", "98.5"],
        &["A3", "InhibitorA", "-8.0", "91.0"],
        &["A4", "InhibitorA", "-7.0", "50.0"],
        &["A5", "InhibitorA", "-6.0", "9.5"],
        &["A6", "InhibitorA", "-5.0", "1.5"],
        &["A7", "InhibitorA", "-4.0", "0.5"],
        &["A8", "InhibitorA", "-3.0", "0.2"],
        &["B1", "InhibitorB", "-10.0", "99.5"],
        &["B2", "InhibitorB", "-9.0", "99.0"],
        &["B3", "InhibitorB", "-8.0", "98.0"],
        &["B4", "InhibitorB", "-7.0", "96.5"],
        &["B5", "InhibitorB", "-6.0", "76.0"],
        &["B6", "InhibitorB", "-5.0", "24.0"],
        &["B7", "InhibitorB", "-4.0", "3.0"],
        &["B8", "InhibitorB", "-3.0", "0.8"],
    ];
    enter_mv_rows(&mut driver, &mv_id, &rows)?;
    let fit_graph = linked_graph_id(&driver, &mv_id)?;

    // Run MV 4PL nonlinear fit and bind X=LogConcM(2), Y=Response(3), Group=Compound(1).
    let fit_id = open_and_run_analyze(&mut driver, "fit.nonlinear")?;
    let _ = press_cmd_shortcut(&mut driver, "t")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.tab.model"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.mv.x.2"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.mv.y.3"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.mv.group.1"))?;
    replace_focused_textbox(&mut driver, &WidgetId::from("fit.eq.search"), "log_inhibitor_vs_response_4pl")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.eq.item.log_inhibitor_vs_response_4pl"))?;

    // Pin Bottom = 0 and Top = 100 constants, enable profile-likelihood CI and residuals tab.
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.tab.constraints"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.con.row.0"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.con.set_scope.constant"))?;
    replace_focused_textbox(&mut driver, &WidgetId::from("fit.con.const.input"), "0.0")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.con.apply"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.con.row.1"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.con.set_scope.constant"))?;
    replace_focused_textbox(&mut driver, &WidgetId::from("fit.con.const.input"), "100.0")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.con.apply"))?;

    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.tab.confidence"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.ci.profile"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("fit.tab.weights_range"))?;
    if !driver.find_widget("fit.wr.opt.res_tab").and_then(|n| n.a11y.checked).unwrap_or(false) {
        let _ = click_widget_center(&mut driver, &WidgetId::from("fit.wr.opt.res_tab"))?;
    }
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;

    verify_result_tabs_and_scroll(&mut driver, &fit_id, &["nonlinear_parameters", "nonlinear_residuals"])?;
    assert_qol7_diagnostics_only(&mut driver, &fit_id)?;
    let rs_fit = result_sheet(&driver, &fit_id)?;
    let params_tab = tab_by_id(rs_fit, "nonlinear_parameters")?;
    let ic50_a_before = tab_cell_f64(params_tab, "LogIC50 [DS1]", "Best-fit value")?;
    let ic50_b_before = tab_cell_f64(params_tab, "LogIC50 [DS2]", "Best-fit value")?;
    let _bot_const = tab_cell_f64(params_tab, "Bottom (Constant)", "Best-fit value")?;
    let _top_const = tab_cell_f64(params_tab, "Top (Constant)", "Best-fit value")?;
    assert_approx(ic50_a_before, -7.0, 0.25, "InhibitorA LogIC50")?;
    assert_approx(ic50_b_before, -5.5, 0.25, "InhibitorB LogIC50")?;

    // QOL-16: Insert fitted model equation (equations-only) + verify GraphAnnotation::ModelEquation.
    select_sheet_in_navigator(&mut driver, &fit_graph)?;
    if matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { .. }) {
        let _ = press_key_combo(&mut driver, "Escape", KeyModifiers::default())?;
    }
    open_command_palette_and_run(&mut driver, "Insert Fitted Model Equation")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("eq.result.0"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    let gs_eq = graph_sheet(&driver, &fit_graph)?;
    if !matches!(gs_eq.spec.annotations.as_slice(), [GraphAnnotation::ModelEquation { .. }]) {
        return Err(format!("expected single QOL-16 ModelEquation annotation, got {:?}", gs_eq.spec.annotations));
    }

    // Create residual diagnostic plot from graph family chooser and render cleanly.
    open_change_graph_type_for_sheet(&mut driver, &fit_graph)?;
    scroll_until_widget_visible(&mut driver, &WidgetId::from("graph_type.panel"), &WidgetId::from("graph_type.card.diag.residual"), 140.0, 12)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("graph_type.card.diag.residual"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    verify_clean_render_and_parity(&mut driver)?;

    // Upstream Response edit updates InhibitorA LogIC50 reactively.
    select_sheet_in_navigator(&mut driver, &mv_id)?;
    edit_grid_cell_human(&mut driver, &mv_id, 3, 3, "80.0")?;
    let ic50_a_after = tab_cell_f64(tab_by_id(result_sheet(&driver, &fit_id)?, "nonlinear_parameters")?, "LogIC50 [DS1]", "Best-fit value")?;
    if (ic50_a_after - ic50_a_before).abs() < 0.05 {
        return Err(format!("4PL LogIC50 did not update after Response edit: {ic50_a_before} -> {ic50_a_after}"));
    }
    verify_clean_render_and_parity(&mut driver)?;
    Ok(())
}

// ============================================================================
// WF-37: Hierarchical Clustering Distances x Linkages, Matrices & Dendrogram Heatmap
// ============================================================================

fn verify_clustering_matrices_and_coph(
    driver: &HumanPathDriver,
    clust_id: &str,
    dist_exp: ClusterDistance,
    link_exp: ClusterLinkage,
) -> Result<f64, String> {
    let rs = result_sheet(driver, clust_id)?;
    match rs.analysis.kind {
        AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance, linkage, .. }) if distance == dist_exp && linkage == link_exp => {}
        ref other => return Err(format!("unexpected Clustering spec ({dist_exp:?}, {link_exp:?}): {other:?}")),
    }
    let coph_r = scalar_f64(rs, "cophenetic_r")?;
    if !coph_r.is_finite() || !(-1.0 - 1e-9..=1.0 + 1e-9).contains(&coph_r) {
        return Err(format!("cophenetic_r out of [-1, 1]: {coph_r}"));
    }
    let dendro = tab_by_id(rs, "hierarchical-dendrogram")?;
    if dendro.rows.len() != 4 {
        return Err(format!("5 objects must produce 4 merges, got {}", dendro.rows.len()));
    }
    let sils = tab_by_id(rs, "hierarchical-cut-silhouettes")?;
    if sils.rows.is_empty() {
        return Err(String::from("missing hierarchical-cut-silhouettes rows"));
    }
    let dmat = tab_by_id(rs, "clustering-distance-matrix")?;
    let cmat = tab_by_id(rs, "cophenetic-distance-matrix")?;
    if dmat.rows.len() != 5 || cmat.rows.len() != 5 {
        return Err(String::from("distance and cophenetic matrices must be 5x5"));
    }
    for i in 0..5 {
        assert_approx(cell_num(&dmat.rows[i][i], "dmat diag")?, 0.0, 1e-12, "dmat diagonal")?;
        assert_approx(cell_num(&cmat.rows[i][i], "cmat diag")?, 0.0, 1e-12, "cmat diagonal")?;
        for j in 0..5 {
            assert_approx(cell_num(&dmat.rows[i][j], "dmat ij")?, cell_num(&dmat.rows[j][i], "dmat ji")?, 1e-10, "dmat sym")?;
            assert_approx(cell_num(&cmat.rows[i][j], "cmat ij")?, cell_num(&cmat.rows[j][i], "cmat ji")?, 1e-10, "cmat sym")?;
        }
    }
    Ok(coph_r)
}

fn compile_heatmap_scene_and_check_pair(driver: &HumanPathDriver, graph_id: &str, expected_cue: &str, pair_a: &str, pair_b: &str) -> TestRes {
    let gs = graph_sheet(driver, graph_id)?;
    let scene: ChartScene = compile_graph(gs, driver.app().document()).map_err(|e| format!("compile_graph failed: {e}"))?;
    let mut texts = Vec::new();
    let mut row_labels: Vec<(f64, String)> = Vec::new();
    let mut dendro_segments = 0usize;
    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Text(t) => {
                let s = t.rich.plain_text();
                if s.starts_with("Row") {
                    row_labels.push((t.anchor_pt.y, s.clone()));
                }
                texts.push(s);
            }
            ScenePrimitive::Path { path, fill: None, stroke: Some(_), clip: None }
                if path.commands.len() >= 4 && matches!(path.commands.first(), Some(PathCmd::MoveTo(_))) =>
            {
                dendro_segments += 1;
            }
            _ => {}
        }
    }
    if !texts.iter().any(|t| t == expected_cue) {
        return Err(format!("missing `{expected_cue}` in heatmap scene texts: {texts:?}"));
    }
    if dendro_segments < 4 {
        return Err(format!("expected >= 4 dendrogram branch paths on heatmap, got {dendro_segments}"));
    }
    row_labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    let order: Vec<String> = row_labels.into_iter().map(|(_, s)| s).collect();
    let pos_a = order.iter().position(|s| s == pair_a).ok_or_else(|| format!("missing `{pair_a}` in {order:?}"))? as isize;
    let pos_b = order.iter().position(|s| s == pair_b).ok_or_else(|| format!("missing `{pair_b}` in {order:?}"))? as isize;
    if (pos_a - pos_b).abs() != 1 {
        return Err(format!("expected `{pair_a}` & `{pair_b}` adjacent in `{expected_cue}`, got {order:?}"));
    }
    Ok(())
}

#[test]
fn wf37_human_hierarchical_clustering_and_dendrogram() -> TestRes {
    let mut driver = new_strict_human_driver()?;
    let mv_id = create_mv_table_via_welcome(&mut driver, "Gene Expression Profiles", 4, 5, true)?;
    configure_mv_headers_and_roles(
        &mut driver,
        &mv_id,
        &[("Sample", "label"), ("GeneA", "continuous"), ("GeneB", "continuous"), ("GeneC", "continuous")],
    )?;
    // Craft 5 labeled samples where Euclidean/Ward pairs RowSmallPos & RowSmallNeg,
    // while Pearson/Complete pairs RowSmallPos & RowScaledPos (r = +1.0).
    let rows: [&[&str]; 5] = [
        &["RowSmallPos", "1.0", "2.0", "3.0"],
        &["RowMid", "10.0", "15.0", "12.0"],
        &["RowSmallNeg", "3.0", "2.0", "1.0"],
        &["RowScaledPos", "100.0", "200.0", "300.0"],
        &["RowMidNeg", "12.0", "10.0", "8.0"],
    ];
    enter_mv_rows(&mut driver, &mv_id, &rows)?;
    let graph_id = linked_graph_id(&driver, &mv_id)?;

    let clust_id = open_and_run_analyze(&mut driver, "stats.clustering")?;
    verify_result_tabs_and_scroll(
        &mut driver,
        &clust_id,
        &["hierarchical-dendrogram", "hierarchical-cut-silhouettes", "clustering-distance-matrix", "cophenetic-distance-matrix"],
    )?;

    // Sweep the 3x3 supported distances x linkages via Cmd+T (`Euclidean`/`Manhattan`/`Pearson` x `Complete`/`UPGMA`/`Ward`).
    let mut upgma_coph_r = Vec::new();
    for (d_slug, d_enum) in
        [("manhattan", ClusterDistance::Manhattan), ("pearson", ClusterDistance::Pearson), ("euclidean", ClusterDistance::Euclidean)]
    {
        for (l_slug, l_enum) in [("complete", ClusterLinkage::Complete), ("upgma", ClusterLinkage::Upgma), ("ward", ClusterLinkage::Ward)] {
            let _ = press_cmd_shortcut(&mut driver, "t")?;
            let _ = click_widget_center(&mut driver, &WidgetId::from(format!("edit.cluster.dist.{d_slug}")))?;
            let _ = click_widget_center(&mut driver, &WidgetId::from(format!("edit.cluster.link.{l_slug}")))?;
            let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
            let coph = verify_clustering_matrices_and_coph(&driver, &clust_id, d_enum, l_enum)?;
            if l_enum == ClusterLinkage::Upgma {
                upgma_coph_r.push(coph);
            }
        }
    }
    if !upgma_coph_r.iter().all(|&r| r > 0.45) {
        return Err(format!("UPGMA cophenetic_r expected > 0.45, got {upgma_coph_r:?}"));
    }

    // Switch MV graph to Clustered Heatmap (`diag.heatmap`) on Euclidean/Ward -> RowSmallPos pairs RowSmallNeg.
    open_change_graph_type_for_sheet(&mut driver, &graph_id)?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("graph_type.card.diag.heatmap"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    compile_heatmap_scene_and_check_pair(&driver, &graph_id, "Clustered: Euclidean / Ward", "RowSmallPos", "RowSmallNeg")?;
    verify_clean_render_and_parity(&mut driver)?;

    // Switch Clustering via Cmd+T to Pearson/Complete -> Heatmap dynamically pairs RowSmallPos & RowScaledPos.
    select_sheet_in_navigator(&mut driver, &clust_id)?;
    let _ = press_cmd_shortcut(&mut driver, "t")?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.cluster.dist.pearson"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("edit.cluster.link.complete"))?;
    let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
    select_sheet_in_navigator(&mut driver, &graph_id)?;
    compile_heatmap_scene_and_check_pair(&driver, &graph_id, "Clustered: Pearson / Complete", "RowSmallPos", "RowScaledPos")?;
    verify_clean_render_and_parity(&mut driver)?;

    // Live upstream cell edit updates Clustering cophenetic_r reactively.
    let coph_before = scalar_f64(result_sheet(&driver, &clust_id)?, "cophenetic_r")?;
    select_sheet_in_navigator(&mut driver, &mv_id)?;
    edit_grid_cell_human(&mut driver, &mv_id, 0, 3, "-9.0")?;
    let coph_after = scalar_f64(result_sheet(&driver, &clust_id)?, "cophenetic_r")?;
    if (coph_after - coph_before).abs() < 1e-4 {
        return Err(format!("upstream cell edit did not recompute cophenetic_r: {coph_before} -> {coph_after}"));
    }

    // Complete the full 4 distances x 7 linkages spec (Cosine + Single/WPGMA/Centroid/Median).
    select_sheet_in_navigator(&mut driver, &clust_id)?;
    for (d_slug, d_enum) in [("cosine", ClusterDistance::Cosine), ("euclidean", ClusterDistance::Euclidean)] {
        for (l_slug, l_enum) in [
            ("single", ClusterLinkage::Single),
            ("wpgma", ClusterLinkage::Wpgma),
            ("centroid", ClusterLinkage::Centroid),
            ("median", ClusterLinkage::Median),
        ] {
            let _ = press_cmd_shortcut(&mut driver, "t")?;
            let _ = click_widget_center(&mut driver, &WidgetId::from(format!("edit.cluster.dist.{d_slug}")))?;
            let _ = click_widget_center(&mut driver, &WidgetId::from(format!("edit.cluster.link.{l_slug}")))?;
            let _ = click_widget_center(&mut driver, &WidgetId::from("modal.ok"))?;
            let _ = verify_clustering_matrices_and_coph(&driver, &clust_id, d_enum, l_enum)?;
        }
    }
    verify_clean_render_and_parity(&mut driver)?;
    Ok(())
}
